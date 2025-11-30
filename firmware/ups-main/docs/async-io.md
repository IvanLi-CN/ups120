# UPS Main 异步 I/O 接口总览

本文件面向新加入的开发者，帮你快速了解 `firmware/ups-main` 中哪些路径是真异步硬件 I/O，以及它们由哪些任务在什么节奏下驱动、通过什么数据结构通信。

## 任务与调度节奏

- `button_task`（`src/main.rs`）  
  - 职责：按键去抖 / 状态机 + 导航事件（Dashboard / BattDetail）产生。  
  - 周期：每 `fan_control::SAMPLE_PERIOD_MS = 20 ms` 一次 `Timer::after(...)`。  
  - 对外通信：通过 `UiEventSender`（`UI_EVENT_CHANNEL`）向 UI 发送 `UiEvent`。

- `power::power_task`（`src/power.rs`）  
  - 职责：所有 I2C 电源相关事务（STM32 smart-battery、TCA6408A、SC8815）+ 充电决策。  
  - 周期：主循环 `Timer::after(Duration::from_millis(500))`，500 ms 一次。  
  - 对外通信：持有 `&'static PowerStateMutex`，每轮更新一次 `PowerState` 快照。

- `thermal::thermal_task`（`src/thermal.rs`）  
  - 职责：TSENS 采样、风扇闭环控制、聚合温度 / FAN 状态。  
  - 周期：每 `SAMPLE_PERIOD_MS = 20 ms` 一次 `Timer::after(...)`。  
  - 对外通信：  
    - 只读 `&'static PowerStateMutex`（读取 smart-battery 温度和 VIN 状态）。  
    - 写入 `&'static ThermalStateMutex`（发布 `ThermalState`）。

- `ui_task`（`src/main.rs`）  
  - 职责：从 `PowerState` / `ThermalState` / `UiEvent` 得到 UI 模型，调用异步 SPI 完整刷新 LCD。  
  - 周期：  
    - 内部 tick：每 `20 ms` 一次 `Timer::after(...)`。  
    - UI 刷新：通过 500 ms 计数器节流，约 2 Hz 重绘一次当前页面。  
  - 对外通信：  
    - 从 `UiEventReceiver` 消费导航事件。  
    - 只读 `&'static PowerStateMutex`、`&'static ThermalStateMutex`。

数据结构：

- `PowerState`（`src/power.rs`）：AC 状态、VBAT / IBAT、`cells_mv`、`state_flags`、`smart_batt_temps`、`adin_temp_c` 等。  
- `ThermalState`（`src/thermal.rs`）：UPS / pack / charger 三路温度 + `fan_control::FanStatus`。  
- `UiEvent`（`src/main.rs`）：`SwitchToDashboard` / `SwitchToBattDetail`。

## 异步 SPI 接口（GC9D01 LCD）

所有 SPI 物理总线通过 `Spi<'static, Async>` 暴露为 `embedded-hal-async` 接口，CS / DC 为 `Output<'static>`。

### 显示驱动 `display.rs`

- `display::init_async<SPI, CS, DC, RST>`  
  - 用途：GC9D01 复位 + 厂商初始化序列（完全异步）。  
  - 调用者：`main` 启动阶段。  
  - 典型频率：只在上电 / 重启时调用一次。

- `display::flush_framebuffer_async<SPI, CS, DC>`  
  - 用途：将全局 framebuffer 以当前 `DRAW_ORIENTATION` 全屏写入 LCD；每次调用都是完整帧刷新。  
  - 调用者：  
    - `ui::boot_init_begin_async` / `ui::boot_update_async`（目前预留，未在主路径使用）。  
    - `ui::render_dashboard_once_async` / `ui::render_batt_detail_once_async`。  
  - 典型频率：由 `ui_task` 驱动，约 2 Hz（500 ms 一次）在当前页面上调用一次。

- 低层异步 SPI 辅助函数（主要被 `init_async` 使用）：  
  - `display::write_command_async` / `write_data_async` / `write_command_with_data_async`  
  - `display::set_address_window_async`  
  - `display::clear_screen_async` / `fill_rect_from_buffer_async` / `fill_area_with_color_async`  
  这些函数只在面板初始化或全屏填充时触发，频率取决于上层调用（目前主要在启动阶段）。

### UI 渲染 `ui.rs`

- `ui::boot_init_begin_async` / `ui::boot_update_async`  
  - 用途：异步版本的启动进度条渲染，最终通过 `flush_framebuffer_async` 输出。  
  - 调用者：目前未使用；同步版本 `boot_init_begin` / `boot_update` 由 `main` 启动阶段调用。  
  - 建议频率：启动阶段按步骤进度调用，典型为数次 / 启动。

- `ui::render_dashboard_once_async`  
  - 用途：根据 `DashboardData` 将仪表盘绘制到 framebuffer，然后完整刷新屏幕。  
  - 调用者：`ui_task` 在 `UiScreen::Dashboard` 下。  
  - 典型频率：约 2 Hz（由 `ui_task` 500 ms UI 周期决定）。

- `ui::render_batt_detail_once_async`  
  - 用途：根据 `BattDetailData` 绘制电池详情页面（包含按平衡状态闪烁的 cell 高亮），然后完整刷新屏幕。  
  - 调用者：`ui_task` 在 `UiScreen::BattDetail` 下。  
  - 典型频率：约 2 Hz。

所有上述渲染路径都走全屏 framebuffer + `flush_framebuffer_async`，不会做局部增量更新，因此不会产生叠加残影。

## 异步 I2C 接口

I2C0 通过 `I2cBusMutex = Mutex<NoopRawMutex, I2c<'static, Async>>` 暴露为共享总线，单一 `power::power_task` 负责所有 I2C 事务。上层访问方式：

- 在 `power::power_task` 中用 `I2cDevice::new(&I2C0_BUS)` 创建临时设备实例。  
- 每个物理芯片（STM32 smart-battery、TCA6408A、SC8815）在同一时刻只维护一个驱动实例，遵循“单实例 per 芯片”的策略。

### STM32 smart‑battery 从设备（`main.rs`，地址 `STM32_ADDR = 0x35`）

高层 API（全部基于 `embedded_hal_async::i2c::I2c`）：

- `read_smart_battery_temperatures`  
  - 用途：一次性读取 pack / charger 两路温度，返回 `SmartBatteryTemps`。  
  - 调用者：`power::power_task` 每 500 ms 一次。  
  - 重试策略：单次读失败直接 `warn!`，本轮返回 `None`，下一轮重试。

- `read_smart_battery_vbat_mv` / `read_smart_battery_ibat_ma`  
  - 用途：读取整包电压（mV）与电流（mA，放电为负）。  
  - 调用者：`power::power_task` 每 500 ms 一次。  
  - 重试策略：单次失败返回 `None`，利用上一次成功值做回退（`vbat_mv.or(sb_last_vbat_mv)`）。

- `read_smart_battery_state_flags`  
  - 用途：读取 `STATE_FLAGS` 寄存器（16 bit）。  
  - 调用者：  
    - `power::power_task` 在检测 IN_PG 边沿时用于记录 AC_PRESENT。  
    - `ui_task` 通过 `PowerState.state_flags` 间接得到平衡标志。  
  - 频率：与 IN_PG 变化相关（通常较低）。

- `read_smart_battery_reg` / `read_smart_battery_reg_retry`  
  - 用途：通用寄存器读（带或不带重试），用于 CHG_CONFIG / CHG_STATUS / CELLS_PRESENT 等。  
  - 调用者：`power::power_task`：  
    - 每 `SB_CFG_VERIFY_INTERVAL_MS = 1000 ms` 验证一次 `CHG_CONFIG` 是否漂移。  
    - 每 `SB_STATE_POLL_INTERVAL_MS = 10_000 ms` 做一次状态快照（status / pause / flags / cell_mv）。  
  - 重试策略：调用处传入 `attempts`（通常为 2）和 `delay_ms`（通常为 2 ms），在失败时做有限次带延时重试。

- `write_smart_battery_reg` / `write_smart_battery_reg_retry`  
  - 用途：写入如 `SB_REG_CHG_CONFIG` 等控制寄存器。  
  - 调用者：`power::power_task` 在以下场景更新充电策略：  
    - 启动时写入初始配置。  
    - 检测到配置漂移时重新应用。  
    - 温度 / 适配器状态变化导致充电 enable / disable。  
  - 重试策略：`SB_WRITE_RETRY_ATTEMPTS = 3`，失败时每次间隔 `SB_WRITE_RETRY_DELAY_MS = 5 ms`。

- `stm_one_shot_validate`  
  - 用途：在 `power::power_task` 启动前对 STM32 I2C 接口做一次窗口读写自检。  
  - 调用者：`power::power_task` 开头，单次调用。  
  - 重试策略：无重试，失败仅记录日志。

### TCA6408A I/O 扩展（`io_expander.rs`）

- 低层：`write_reg` / `read_reg` / `update_register` / `set_outputs`（全部 async）。  
- 驱动结构：`Tca6408a<I2C>` 单实例，`i2c` 为 `I2cDevice<'static, ...>`。

公开 async 方法：

- `Tca6408a::init`  
  - 用途：配置端口方向、默认输出，使 CE / PSTOP 处于安全态。  
  - 调用者：`power::power_task` 启动时调用一次。

- `Tca6408a::set_sc_ce` / `set_sc_pstop`  
  - 用途：控制 SC8815 的 CE 与 PSTOP 引脚，实现“逻辑关断 / 启用”与安全停机。  
  - 调用者：  
    - `power::power_task` 在充电 / 保护逻辑中需要时更新 PSTOP。  
    - `log_sc8815_temperature` 在采样 SC8815 ADIN 前后切换 CE。  
  - 频率：500 ms 级别（随 `power_task` 循环和温度采样调用）。

- `Tca6408a::read_in_pg` / `read_alert`  
  - 用途：读取适配器 PG 状态与告警引脚。  
  - 调用者：  
    - `power::power_task` 每轮 500 ms 读取 IN_PG，用于 AC / stability 判定。  
    - `read_alert` 当前仅作辅助 / 预留（在部分路径中调用）。

### SC8815 升压 / 充电器（`log_sc8815_temperature` + `sc8815` crate）

- 在 `power::power_task` 中维护一个 `Option<sc8815::SC8815<SharedI2cDevice<'static>>>` 单实例。  
- 异步方法（通过 `log_sc8815_temperature` 封装）：  
  - `SC8815::init`：上电后一次性初始化；需在 CE 拉低后调用。  
  - `SC8815::set_adc_conversion(true / false)`：开启 / 关闭 ADC 转换。  
  - `SC8815::get_adc_measurements`：读取包括 ADIN 在内的一组测量值。

调用节奏（`log_sc8815_temperature`，被 `power_task` 调用）：

- 每 500 ms：  
  - `Tca6408a::set_sc_ce(true)` → `Timer::after(5 ms)` → `SC8815` init / 测量 / 停止 ADC → `Tca6408a::set_sc_ce(false)`。  
  - 最终将 ADIN 电压转换为 UPS 温度并存入 `PowerState.adin_temp_c`。

## 关键 await 点与周期总结

- UI 路径  
  - `ui_task`：  
    - 每 20 ms：`Timer::after(Duration::from_millis(SAMPLE_PERIOD_MS))`。  
    - 每 500 ms：`power_state.lock().await` + `thermal_state.lock().await` + 一次 `render_*_once_async`（内部调用 `flush_framebuffer_async` 全屏刷新）。

- 电源路径  
  - `power::power_task`：  
    - 主循环：`Timer::after(Duration::from_millis(500))`。  
    - 每轮内：多次 smart-battery I2C 读写（VBAT / IBAT / 温度 / 状态）、TCA6408A 读写、SC8815 温度采样。  
    - 慢速周期：`SB_CFG_VERIFY_INTERVAL_MS = 1000 ms`（配置漂移检查）、`SB_STATE_POLL_INTERVAL_MS = 10_000 ms`（状态快照）。  
    - 写寄存器失败时：在 `write_smart_battery_reg_retry` / `read_smart_battery_reg_retry` 内部做 ms 级短延时重试。

- 温度 / 风扇路径  
  - `thermal::thermal_task`：  
    - 每 20 ms：  
      - `tsens::read_celsius_async().await`（内部为多次 `Timer::after_micros` 轮询 READY）。  
      - 之后根据 `PowerState` 中的 smart-battery / ADIN 温度与 VIN 状态做一次控制步进，更新 `ThermalState`。

- 按键路径  
  - `button_task`：  
    - 每 20 ms：`Timer::after(Duration::from_millis(SAMPLE_PERIOD_MS))`。  
    - 无外设 I/O，纯 GPIO 采样 + 状态机，将边沿转换为 `UiEvent` 发送到 `UI_EVENT_CHANNEL`。

通过以上结构，所有真实硬件 I/O（I2C / SPI / TSENS）都由各自的专用异步任务集中管理，UI 侧只消费经 `PowerState` / `ThermalState` 汇总后的快照，并通过 `UiEvent` 渠道完成页面切换，不直接触碰底层总线。这样既保证了时序可控，也便于后续在不改 UI 的情况下调整 I2C 轮询策略或显示刷新节奏。

