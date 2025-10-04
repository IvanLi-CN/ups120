# 智能电池外部 I2C 从机唤醒功能规划（STM32L051 + BQ76920 + SC8815）

> 设计合并提示：LED/状态机/低功耗的最新统一设计请参考 `firmware/smart-battery/SOFTWARE_DESIGN.md`；本文件仅保留与 I2C 从机唤醒相关的规划说明。

> 版本增补（2025-09-29）：
> 1) 硬件更新：MCU 侧通过 N 沟道 MOSFET 驱动 SC8815 的 `CE`/`PSTOP`，导致 MCU 引脚语义与芯片有效电平反相。固件需以语义 API 控制（`CE_OUT 低=使能`，`PSTOP_OUT 高=停机`）。
> 2) LED 规范更新：四色 LED 与状态机显示规则见 `docs/sb-led-and-state-machine-v2.md`，绿灯在 Sleep 期间常灭，仅通信时异步脉冲一次；其余灯按 3 秒周期与优先级仲裁执行。

本文档规划固件在“出厂/低功耗待机”场景下的初始化、休眠与基于 I²C（从机模式）唤醒的整体方案，并给出可交付的实现分解与验收标准。为加速落地，当前阶段采用 SLEEP 模式（非 STOP）；后续再演进到 STOP。目标平台：STM32L051C8T6（Embassy），外设：BQ76920（BMS AFE）、SC8815（充电/功率器件），对外通信：I2C1 从机（PB6/PB7，7‑bit 地址 0x35）。

---

## 1. 目标与约束（结合反馈）

- 上电后：
  - 完成 BQ76920 基础初始化，确保保护参数/ADC 工作正常，但保持 CHG/DSG FET 全部关闭（输入/输出均关闭）。
  - 对 SC8815 执行“出厂化重置”（software reset to defaults），随后禁用功率级（CE=HIGH、PSTOP=HIGH），禁止 OTG/充电；ADC 可按需关闭以省电。
- LED 指示：上电即常亮（运行态）；当进入 SLEEP 时可选择关闭 LED（Hi‑Z 非必需，后续 STOP 版再做）。
- 进入低功耗 SLEEP 模式；当外部主机通过 I2C 访问从设备地址 0x35 时，由 I2C 中断唤醒 CPU，完成一次事务处理后在空闲回落 SLEEP。
- 遵循现有仓库约束（独立工程、Makefile 驱动构建、Embassy 运行时、defmt 日志）。

---

## 2. 技术要点（I²C + SLEEP 快速实现）

- SLEEP 模式对 I2C“无影响”，I2C 中断可使设备退出 SLEEP（参考手册表格说明）。因此本阶段无需 WUPEN、无需强制 DNF=0。
- I2C1 作为从机配置 7‑bit 地址（OAR1），开启必要中断（ADDRIE/ERRIE，按实现选择 RXIE/TXIE/STOPIE/NACKIE）。
- 时钟：无需常开 HSI16KERON；保持常规系统时钟配置即可。为确保 I2C 在 SLEEP 期间有时钟，需使能 `RCC_APB1SMENR.I2C1SMEN=1`（I2C1 Sleep 模式时钟使能）。
- 运行时：Embassy idle 路径采用 `WFI` 进入 SLEEP（`SLEEPDEEP=0`）。我们将：
  - 启用 LSE 作为低速时基（已在 `main.rs` 配置 `default_lse()`）。
  - 确保没有持续活跃的定时器/任务阻止 SLEEP（通过“睡眠协调器”集中裁决）。
 - NVIC：必须在 NVIC 侧开启 I2C1 事件/错误中断通道，否则无法依靠 I2C 中断从 SLEEP 唤醒；建议在上电日志打印 NVIC 相关通道的使能状态。

---

## 3. 上电初始化序列（Power‑On Sequence）

1) 基础硬件态：
   - LED：运行期常亮；进入 SLEEP 前关闭 LED（Hi‑Z 可留待后续 STOP 版）。
   - 配置 I2C1 从机（地址 0x35）；保守默认 `DNF=0` 与 `ANFOFF=0`；绑定 I2C1 事件/错误中断。
   - 置位 `RCC_APB1SMENR.I2C1SMEN=1`，保证 SLEEP 期间 I2C 有时钟；无需设置 `RCC.CR.HSI16KERON`。

2) BQ76920 初始化（仅准备测量与保护，不开 FET）：
   - 通过驱动写入/校验保护参数（OV/UV/OCD/SCD 等）与 ADC/CC 相关配置。
   - 明确将 `CHG_ON=0`、`DSG_ON=0`（保持输入/输出关闭）。
   - 清除状态位，启动周期性测量（用于对外只读遥测）。

3) SC8815 出厂化重置并禁用：
   - 调用驱动 `reset()`（清空 CTRL 寄存器→`init()` 复位到默认安全配置）。
   - 确保 `CE=HIGH`、`PSTOP=HIGH`，禁止功率级；禁止 OTG；必要时关闭 ADC 转换以节能。

4) 对外协议（I2C1 从机）：
   - 复用现有寄存器映射与 TI‑式逐字节 PEC 校验；
   - 将 `SYS_STATUS.awake` 位用于“当前是否处于活动态（非 SLEEP）”的只读提示；
   - `CHG_ENABLE_REQ`（0x31）等写寄存器预留，但默认不触发任何上电使能，除非后续需求另行放开。

5) 进入 SLEEP：
   - 启动“睡眠协调器”，当无活跃会话/事务且安全条件满足（见 §5）时执行 SLEEP；
   - I2C 事件/错误中断唤醒后，处理一帧事务，空闲超时（如 200–500 ms）再次入 SLEEP。

---

## 4. I²C 唤醒工作流（Host ↔ Pack）

- Host 发起：`START → SLA+W(0x6A) → RegPtr … [可选重复启动读]`；
- 从设备：I2C1 在 SLEEP 中由 I2C 中断唤醒 CPU；ISR 触发、Embassy 任务 `i2c1_slave::slave_task()` 的 `listen().await` 继续；
- 处理：
  - 写操作：校验 interleaved PEC，更新镜像寄存器或命令；
  - 读操作：根据当前 `REG_PTR` 组帧，交错返回 `[DATA, CRC]`；
- 空闲回落：事务完成、空闲计时达到阈值后，睡眠协调器再次入 SLEEP。

CHG_ENABLE_REQ（0x31, RW）的说明：

- 定义：对外协议中的“充电允许请求”位，仅表示“主机请求允许充电”的意图（请求态）。
- 本阶段策略（按你确认）：I²C 写入不改变当前硬编码的硬件流程，仅记录镜像状态（不落地执行）。
- 未来落地条件（规划占位，不在本次实现）：需满足 BQ76920 无故障、SC8815 初始化完毕且处于充电模式允许、适配器存在、Pack 电压/温度在安全范围内，且通过策略层两阶段确认后才可能放行。

---

## 5. 睡眠协调器（Sleep Manager）

- 设计：新增 `sleep_manager.rs` 提供原子计数型“活动票据”（busy token）与空闲定时窗；以下事件会“持有活跃票据”防止 SLEEP：
  - 内总线 I2C2 活跃传输；
  - BQ76920/SC8815 关键配置阶段；
  - 任何需要严格实时性的 ISR 后处理窗口；
- 条件满足即执行：
  - 清除 `SLEEPDEEP` 并执行 `WFI` 进入 SLEEP（Embassy idle path）；
  - I2C1 事件/错误、RTC/LPTIM 均可唤醒。

超时与 LED 策略：

- `SLEEP_REENTER_IDLE_MS = 300 ms`：最后一次事务/活动后 300ms 回落 SLEEP（可通过 feature/env 调整）。
- LED 与睡眠状态同步：在“运行/唤醒”态时启用 LED 任务并维持常亮（或后续扩展为原设计灯效）；进入 SLEEP 前关闭 LED（Hi‑Z 可选）。

---

## 6. 任务划分与改动点

- 新增/修改文件：
  - `firmware/smart-battery/src/sleep_manager.rs`：忙闲票据、空闲回落策略、统计/调试计数。
  - `firmware/smart-battery/src/main.rs`：
    - I2C1 从机：保守 `DNF=0`、`ANFOFF=0`；开启 ADDRIE/ERRIE 等；
    - SLEEP 支持：`RCC_APB1SMENR.I2C1SMEN=1`；无需 HSI16KERON；
    - 初始化序列重排：LED→I2C1→BQ→SC；
    - 启动睡眠协调器并在关键阶段借/还票据；
  - `firmware/smart-battery/src/bq76920_task.rs`：默认保持 FET 关闭；仅做测量与告警；（后续如需放开，可经策略或主机命令触发）。
  - `firmware/smart-battery/src/sc8815_task.rs`：启动阶段调用 `reset().await`，随后确保 `CE=HIGH`、`PSTOP=HIGH`、禁用 OTG/ADC（按需）。
  - `firmware/smart-battery/src/led_status_task.rs`：
    - 新增 `set_led_run_mode()`：运行/唤醒态下常亮（或走原灯效 API）。
    - 新增 `set_led_sleep_mode()`：SLEEP 前关闭 LED（Hi‑Z 可选）。
    - 与睡眠协调器对接：状态切换时调用以上两个接口，无独立活动窗口概念。

- 非功能性：
  - defmt 日志级别保持 `info`；加入关键寄存器只读回显（I2C1.CR1/OAR1/TIMINGR/ISR、RCC.CR）。
  - 严禁在仓库根运行 Cargo；仅通过 Makefile 目标构建/烧录。

---

## 7. 风险与缓解

- SLEEP 前提：确保 `RCC_APB1SMENR.I2C1SMEN=1` 且 I2C1 中断已开启；DNF 无硬性限制（默认 0 以降低不确定性）。
- BQ76920 FET 缓启策略：默认全关，避免上电误导通；后续若开放主机使能，需要与保护逻辑/SC8815 状态联动的安全闸。
- SC8815 “出厂化重置”：驱动 `reset()` 通过清控寄存器并 `init()` 恢复默认，再强制 CE/PSTOP 关闭功率级，避免误开关频。
- 低功耗抖动：通过睡眠协调器聚合空闲判据与超时，避免频繁进出 SLEEP。

---

## 8. 实施步骤（可交付分解）

1) I2C1 唤醒底座（SLEEP）：配置 `RCC_APB1SMENR.I2C1SMEN=1`、开启必要中断；保守 `DNF=0/ANFOFF=0`；寄存器级读回显（上电日志）。
2) LED Hi‑Z：`power_off_pins()` 与默认不启动 LED 任务。
3) BQ76920 初始化最小闭环：保护参数→校验→FET 全关；周期测量发布。
4) SC8815 reset/禁用：`reset().await`、CE/PSTOP 高、OTG 禁止、ADC 按需停。
5) 睡眠协调器：busy token 接入现有任务；空闲入 SLEEP；空闲超时回落阈值可配置（env/feature）。
6) I2C1 从机联调：逻分仪验证“中断→唤醒→事务→回落”；对外寄存器 BANK 与 PEC 校验回归；验证 LED 在运行/唤醒态常亮，进入 SLEEP 后关闭（可选 Hi‑Z）。
7) 文档与验收：更新 `SOFTWARE_DESIGN.md` 与新增低功耗章节、实验记录（电流曲线与时序图）。

---

## 9. 验收标准（Acceptance Criteria）

- 上电日志：
- 打印 I2C1 关键寄存器（CR1/OAR1/TIMINGR/ISR）与 `RCC_APB1SMENR.I2C1SMEN=1` 状态；
  - 打印 NVIC 侧 I2C1 事件/错误中断通道的使能状态，以及 `I2C_CR1` 中断使能位（如 ADDRIE/ERRIE/RXIE/TXIE/STOPIE/NACKIE）读回显；
  - 明确 “BQ: FET=OFF（CHG=0, DSG=0）”、“SC: CE=HIGH, PSTOP=HIGH, OTG=OFF, ADC=OFF/ON(按配置)”；
  - LED 引脚已 Hi‑Z（仅一次性打印）。
- 休眠/唤醒：
  - 空闲进入 SLEEP，静态电流较运行态下降；
  - 外部主机对 0x35 发起寻址，I2C 中断唤醒 MCU 并完成读写；
  - 事务完成后在设定空闲窗内再次进入 SLEEP；运行/唤醒时 LED 常亮（或原灯效），入 SLEEP 时 LED 关闭（可选 Hi‑Z）。
- I2C 兼容性：100 kHz、400 kHz 下读写均成功；PEC 校验正确；
  - 响应时序：在 100 kHz 与 400 kHz 下记录一次事务的唤醒/响应总延迟与（如有）SCL 拉伸时长上限；留存逻分仪波形截图。
- 安全性：在任何异常（I2C 内部总线故障/AFE 报警）下不会自动使能 CHG/DSG 或放开 PSTOP。

---

## 10. 参考资料

- STMicroelectronics – STM32L051x6/x8/xB 数据手册与参考手册（SLEEP 模式下 I2C 中断可唤醒；STOP 模式方案作为后续演进参考）。
- Texas Instruments – BQ76920/30/40 Battery Monitor AFE Datasheet（初始化、保护、`CHG_ON/DSG_ON`、ADC/CC）。
- Southchip – SC8815（本仓库 `sc8815` 驱动与寄存器说明），I²C 控制/CTRLx_SET/Status/ADC。
- Embassy – `embassy-stm32` I2C 从机 / 低功耗 idle 路径与 `time-driver`（LSE/LPTIM/RTC）。

> 备注：具体寄存器位与约束将在实现提交中以注释+数据手册页码标注；本文为高层规划与系统方案。
