# UPS 主控放电 / 输出稳压策略（ESP32-S3 侧）

> 版本：2025-11-30  
> 角色：UPS 主控（ESP32-S3）  
> 范围：描述 UPS 电源板上 SC8815 的放电 / 输出稳压策略，仅约束 ESP32 侧行为，不改动智能电池板（STM32 + BQ76920 + 另一颗 SC8815）的既有策略。

---

## 1. 设计目标

- 在线式 UPS 语义：**在 UPS 功能启用且未出现致命问题时，应始终维持 OUT 总线稳压输出**，无论 AC 是否存在；只有在 UPS 显式禁用或进入故障态时才允许关断 OUT。
- 在 AC 存在或掉电时，尽量保持 OUT 总线电压稳定在硬件设定的目标附近（由 SC8815 外部分压与芯片内部调节共同决定，详见 `docs/SC8815_External_Resistor_Configuration.md`），避免对后级负载产生不必要的跌落和毛刺；**目标电压应略低于正常输入电压**，以保证在线模式下不会反向推高输入侧。
- 在电池电压过低、温度越限或出现故障时，快速且可预测地关闭 SC8815 功率级，保证电池与功率器件安全优先。
- 将放电 / 输出控制集中在 ESP32 的 `power_task` 内实现，与 `charging_policy.md` 中的充电策略风格一致，避免 STM32 侧逻辑“反客为主”。
- 为 UI 与日志提供稳定的数据来源：放电模式（`Discharge`）时的 `OUT` 三元组（`out_v_mv/out_a_ma/out_w_mw`）与 `UPS` 温度（来自 SC8815 ADIN）。

---

## 2. 信号与依赖（输入来源）

UPS 放电 / 输出稳压策略仅依赖下列信号与状态：

1. **适配器与输入电源**
   - `IN_PG`：由 TPS2490 `PG` 汇入 TCA6408A `P0`，经 `firmware/ups-main/src/io_expander.rs` 读取为布尔量。  
   - 在 `power_task` 中映射为 `vin_present` 和时间戳 `vin_state_last_change_ms`，与 `charging_policy.md` 共用。

2. **电池状态（经 STM32 智能电池板）**
   - 包电压、电流：`PowerState.vbat_mv`、`PowerState.ibat_ma`（由 `read_smart_battery_vbat_mv` / `read_smart_battery_ibat_ma` 得到）。  
   - 保护 / 告警位：`PowerState.state_flags`（来自 `STATE_FLAGS`）。  
   - 分节电压 / AC 状态等只允许作为日志字段，不参与 UPS 放电决策优先级判定。

3. **UPS 输出与温度（SC8815 电源板）**
   - 输出测量：通过 `sc8815::SC8815` 驱动的 ADC 读数，得到 `VBUS/VBAT/IBUS/IBAT/ADIN`，在 UPS 主控侧组合为输出三元组：  
     - `out_v_mv`：OUT 总线电压估算；  
     - `out_a_ma`：放电电流（朝负载方向）；  
     - `out_w_mw`：`out_v_mv * out_a_ma`。  
   - UPS 温度：`PowerState.adin_temp_c`，由 `log_sc8815_temperature` 使用 ADIN 电压转换得到（`src/adin_temp.rs`）。

4. **SC8815 功率级控制**
   - 经 TCA6408A 控制的逻辑 API（隐藏具体极性与板级反相）：
     - `io_expander::Tca6408a::set_sc_ce(enable: bool)`：控制 SC8815 芯片上电 / 关断；
     - `io_expander::Tca6408a::set_sc_pstop(stop: bool)`：控制功率级“停机 / 允许运行”。
   - I²C 驱动实例：`sc8815::SC8815<SharedI2cDevice<'static>>`，提供：
     - `init()`：一次性初始化（FACTORY/MASK 等）；  
     - `configure_device(DeviceConfiguration)`：设置 OTG/充电模式、限压限流、PFM/频率等；  
     - `read_all_adc_registers()` + 计算函数：将 ADC 原始值转成 mV/mA；  
     - 状态查询：`is_otg_mode()`、`is_otp_fault()`、`is_vbus_short_fault()` 等。

> 约束：UPS 放电策略不得从 STM32 的 `AC_PRESENT` 或其它 AC 状态位中导出决策条件，与充电策略一致；这些位仅允许作为日志字段出现。

---

## 3. 逻辑输出状态

从策略视角，UPS 输出只区分两种逻辑状态，并在“UPS 功能启用且无故障”时**偏好保持在 OUT_ENABLED**：

1. **OUT_DISABLED（输出关闭）**
   - SC8815 功率级被显式停机：  
     - 调用 `tca.set_sc_pstop(true)` 请求停机；  
     - 视功耗需要，可在停机后调用 `tca.set_sc_ce(false)` 将芯片进入低功耗。  
   - OTG / 放电模式应被关闭或保持在不会向 OUT 提供持续功率的状态（如不置 EN_OTG 或将功率级限流到 0）。  
   - UI 处于“就绪”（`Ready`）、“充电”（`Charge`）或叠加 `LowBatt` 提示但未放电时，屏幕第三行**不得伪装成仍在放电**：
     - 第三行应分别按照 `ui-spec.md` 使用 `IDLE <时长>` 或 `CHG <功率>` 布局，`OUT` 三元组可以显示为 `--` 或完全不显示；
     - 历史 OUT 数值只用于日志或离线诊断，不应在这些模式下误导为“当前仍在放电”。

2. **OUT_ENABLED（输出开启）**
   - 在 UPS 功能启用且无第 4 节所列关闭条件时，这是**默认期望状态**：  
     - `tca.set_sc_ce(true)` 使 SC8815 上电；  
     - 成功完成一次 `sc.init().await` 与 `configure_device(..)`；  
     - `tca.set_sc_pstop(false)` 允许功率级工作；  
     - 使能 OTG / 放电模式：`sc.set_otg_mode(true)`。  
   - 周期性读取 SC8815 ADC，将 OUT 三元组与 UPS 温度写入 `PowerState`，供 UI 与日志使用。  
   - UI 处于“放电”（`Discharge`）模式时，第三行 `OUT <V/A/W>` 的数值来自该状态；在 `LowBatt` 但仍在放电的场景下，底层仍视为 OUT_ENABLED，UI 第三行布局与 `Discharge` 相同，仅第一行 MODE 显示为 `LOWBATT`。

> 说明：CE/PSTOP 的具体电平极性与板级反相关系以 `io_expander.rs` 与实际硬件为准；本策略只约束 `set_sc_ce(enable)` / `set_sc_pstop(stop)` 的 **语义**。

---

## 4. 关闭条件（必须立即停机）

一旦满足下列任一条件，UPS 主控必须尽快将状态切换到 **OUT_DISABLED**，并记录清晰的日志原因；优先级从高到低：

1. **电池 / 保护板硬故障**
   - STM32 上报的 `STATE_FLAGS` 中出现致命保护（如 OV/UV/短路等）或 BQ FET 已关断：  
     - 视实际定义，将其映射为布尔量 `pack_critical_fault`；  
     - 一旦检测到 `pack_critical_fault=true`，必须：
       - 调用 `tca.set_sc_pstop(true)` 立刻停机；  
       - 追加 `sc.set_otg_mode(false)`、必要时关断 CE；  
       - 日志示例：`discharge: disabled due to pack fault flags=0xXXXX`。

2. **UPS 温度越限（SC8815 / 电源板）**
   - 使用 `PowerState.adin_temp_c`（SC8815 ADIN 转换后的 UPS 温度）进行越限判定：  
     - 定义两级阈值（与风扇 / 充电温度策略保持一致）：  
       - `UPS_DISCH_STOP_C`（停机阈值）；  
       - `UPS_DISCH_RESUME_C`（恢复阈值，低于停机阈值若干度，用于滞回）。  
     - 当 `UPS 温度 ≥ UPS_DISCH_STOP_C` 时：
       - 立即执行与上节相同的停机动作；  
       - 日志：`discharge: disabled due to high UPS temperature temp=XXC`。  
     - 只有当温度降至 `UPS_DISCH_RESUME_C` 以下，才允许在其它条件满足时重新进入 OUT_ENABLED。

3. **包电压过低（防止过放）**
   - 使用由 STM32 上报的 `vbat_mv`（或 `sb_last_vbat_mv`）判定：  
     - 设定两个门限：  
       - `DISCH_STOP_VBAT_MV`：UPS 停止放电的最低安全电压；  
       - `DISCH_RESUME_VBAT_MV`：允许恢复放电的电压（高于停止阈值，提供滞回）。  
     - 约束关系：  
       - `DISCH_STOP_VBAT_MV` 不得低于智能电池侧 `PACK_OUTPUT_CUTOFF_THRESHOLD_MV`，建议预留一定裕量（例如 ≥该阈值 + 0.5–1.0 V）。  
     - 当 `vbat_mv <= DISCH_STOP_VBAT_MV` 时：
       - 立即停机 (`set_sc_pstop(true)`，必要时清除 OTG / 关断 CE)；  
       - 日志：`discharge: disabled due to low pack voltage vbat=XXXXmV`。  
     - 在 `vbat_mv` 再次升至 `DISCH_RESUME_VBAT_MV` 以上之前，禁止重新开启 OUT。

4. **SC8815 自身故障**
   - 通过驱动查询：  
     - `is_otp_fault()`（芯片 OTP）；  
     - `is_vbus_short_fault()`（输出短路）；  
     - 其它状态字段 `SC8815Status` 中的严重错误。  
   - 出现上述任一情况时：  
     - 先调用 `set_sc_pstop(true)` 停机，再按需要执行 datasheet 推荐复位 / 清故障序列（如 `clear_vbus_short_fault_with_delay`）；  
     - 日志：`discharge: disabled due to SC8815 fault ...`。  
   - 故障清除后需通过人工或策略层确认，方可再次允许 OUT_ENABLED。

---

## 5. 允许条件（何时可以开启放电）

UPS 主控只有在下列条件全部满足时，才可以从 **OUT_DISABLED** 进入 **OUT_ENABLED**；在满足条件之后，应尽量保持 OUT_ENABLED，不因 AC 正常存在而主动关断 OUT：

1. **无致命故障**
   - 电池侧：`pack_critical_fault=false`；  
   - SC8815：未检测到 OTP / VBUS_SHORT 等致命状态，或已按推荐流程清除并确认恢复；  
   - 若存在仅影响精度的软性告警，可记录日志但不阻塞放电。

2. **包电压在安全区间内**
   - `vbat_mv >= DISCH_RESUME_VBAT_MV`；  
   - 若电压接近上限（例如靠近充电终止电压），策略可选择仍允许放电（这属于“放电策略”，与充电不同），但仍需遵守 BQ76920 / 智能电池的保护范围。

3. **温度在安全区间**
   - `UPS 温度 < UPS_DISCH_STOP_C` 且上一次因温度导致的停机已经解除（`UPS 温度 <= UPS_DISCH_RESUME_C`）；  
   - 电池与充电器温度如已触发自身保护，可视为“致命故障”并归入第 4 节。

4. **策略允许 UPS 供电**
   - 更上层的模式逻辑认为当前应该由 UPS 输出供电（在线式 UPS 语义下，**只要 UPS 功能启用且系统健康，就认为应由 UPS 输出稳压**）：  
     - 该逻辑最终反映为 UI 处于“放电”（`Discharge`）模式，或在叠加 `LowBatt` 提示但仍在放电时，第一行显示 `MODE: LOWBATT`、第三行仍为 `OUT <V/A/W>`；  
     - 本文件不约束该模式切换的细节，只要求：当判定需要 UPS 稳压输出时，上述安全条件需已满足，才能真正打开 SC8815。

当所有条件满足时，建议按照如下顺序启用输出：

1. 通过 `io_expander::Tca6408a` 拉到允许状态：`set_sc_ce(true)`、`set_sc_pstop(false)`；  
2. 若首次启用或上次出现故障后：  
   - 调用 `sc.init().await` 验证通信并设置 FACTORY/MASK；  
   - 配置 `DeviceConfiguration`：  
     - `config.power.operating_mode = OperatingMode::OTG`；  
     - 根据硬件评估设置适当的 `ibus_limit_ma/ibat_limit_ma` 与开关频率等；  
     - `config.battery.use_internal_setting = false`（外部分压模式，参见外部电阻配置文档）。  
3. 进入周期性 ADC 采样与 OUT 三元组更新逻辑。

---

## 6. AC 事件下的行为（与充电策略的关系）

放电 / 输出策略与充电策略的关系如下：

1. **AC 掉电**
   - 充电策略：一旦 `vin_present=false`，必须立即清除 `CHG_CONFIG.MANUAL_ENABLE`，停止充电（见 `charging_policy.md`）。  
   - 放电策略：若电池与温度条件允许，OUT **应继续维持由电池供电的稳压输出**，不因 AC 掉电自动关闭；只有当电池/温度/故障命中第 4 节关闭条件时才允许关断 OUT。特定异常场景下是否需要额外切断 OUT，由后续“系统能量流策略”进一步定义。

2. **AC 恢复与抖动**
   - 充电策略对 `vin_ok_for_charge` 施加 10 s 稳定窗口；  
   - 放电策略不直接依赖该窗口，只要放电允许条件仍满足，应持续维持 OUT 稳压，不因为 AC 恢复或轻微抖动反复切断输出；  
   - 若系统需要在 AC 恢复后切换为“旁路模式”（例如直接由 AC 路径供电而非 DC/DC），该切换逻辑应在更上层策略中定义，并在切换前先安全地将 OUT 转入 **OUT_DISABLED**；在在线式配置中，该旁路通常处于关闭或保留状态。

---

## 7. 实现位置与建议

- 实现主体：`firmware/ups-main/src/power.rs::power_task`。  
  - 在现有 `vin_present`、`vbat_mv`、`smart_batt_temps`、`adin_temp_c` 刷新逻辑基础上增加：  
    - 状态机变量（例如 `out_enabled: bool`）；  
    - 对第 4/5 节条件的检查与转移；  
    - 与 `io_expander::Tca6408a` 和 `sc8815::SC8815` 的控制调用。
- UI 对接：  
  - 放电模式（`Discharge`）的判定应基于 `out_enabled` 和功率流向，以确保屏幕第三行 `OUT` 三元组仅在实际放电（包括叠加 `LowBatt` 的放电场景）时显示；  
  - `PowerState` 中的 `adin_temp_c` 与未来的 OUT 测量字段作为 UI 的唯一数据来源。

---

## 8. 验证要点（建议）

1. **基本启停**
   - 在电池充足、温度正常情况下切换到放电模式，确认：  
     - `set_sc_ce(true)` / `set_sc_pstop(false)` 被调用；  
     - OUT 电压稳定在预期附近；  
     - UI 第三行展示 `OUT V/A/W`，数值合理。

2. **低电压停机**
   - 人为将包电压拉低到略低于 `DISCH_STOP_VBAT_MV`：  
     - 观察到日志 `discharge: disabled due to low pack voltage ...`；  
     - OUT 电压断开或显著降为 0；  
     - 待电压恢复至 `DISCH_RESUME_VBAT_MV` 以上，并手动或策略允许后，OUT 才重新开启。

3. **UPS 过温停机**
   - 通过负载与环境或热风机提升 UPS 电源板温度，超过 `UPS_DISCH_STOP_C`：  
     - 观察停机与相应日志；  
     - 温度恢复并满足恢复阈值前，禁止 OUT 重新开启。

4. **SC8815 短路保护**
   - 在受控条件下对 OUT 端施加短路 / 重载，触发 SC8815 VBUS_SHORT 或相关保护：  
     - 验证策略正确停机，并根据驱动 `clear_vbus_short_fault_with_delay` 等流程恢复；  
     - 恢复后仅在条件满足时重新进入 OUT_ENABLED。

上述行为作为 UPS120 放电 / 输出稳压策略的目标行为，后续实现与调整应保持与本文档一致；如需更改，需同步更新本文件并在硬件上进行回归验证。
