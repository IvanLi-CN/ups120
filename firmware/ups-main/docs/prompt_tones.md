# UPS 主控提示音方案（蜂鸣器）

本文件定义 UPS 主控（`firmware/ups-main`）的提示音需求、事件分组、提示音目录与调度策略。它是后续实现提示音管理器与按键反馈音的实现基线。

## 背景

- 设备具备无源蜂鸣器（见 `docs/mcu_hardware.md`：`BUZZER=GPIO38`，LEDC PWM）。
- 当前固件已初始化 BUZZER PWM 但保持静音；缺少统一的“事件 → 提示音”能力。
- 需要参考 `isolapurr-usb-hub` 的分层方案：硬件驱动层（buzzer）+ 非阻塞提示音状态机（prompt_tone），并扩展到本项目的事件与交互需求。

## 目标

- 非阻塞：提示音播放不得 busy-wait；由硬件 PWM + 状态机推进。
- 统一入口：所有任务以事件的形式上报（`SoundEvent`），由提示音层统一映射与调度。
- 三类声音形态（外加按键动作音）：
  - **模式切换**：一次性、**≥5s** 的可辨识柔和旋律（含市电掉电/恢复）。
  - **一般事件**：一次性、**≥2s** 的提示音。
  - **严重事件**：持续循环播放，直到事件退出；区分“可恢复”与“不可自动恢复”。
  - **按键动作反馈**：每次按键动作完成都要发声（确认/失败/故障三种）。
- 报警优先：任一严重事件报警循环激活时，必须保证报警持续可感知；同时仍允许按键动作反馈音。

## 非目标

- 不做音乐播放器/复杂编曲；旋律仅用于“可辨识”与“柔和”。
- 不引入运行期音量/静音 UI（后续若需要可再扩展）。
- 不要求蜂鸣器替代屏幕；声音仅作为冗余/即时反馈。

## 术语与约束

- `Action`：按键动作结果提示音（短促，≤350ms），分为 `OK/FAIL/FAULT`。
- `ModeMelody`：模式切换旋律（一次性，5–7s）。
- `NoticeOnce`：一般事件提示音（一次性，≥2.1s）。
- `AlarmLoop`：严重事件报警循环（持续，直到退出）。

硬件/实现约束：

- BUZZER 使用独立 LEDC timer（目前为 Timer1），允许在不影响风扇/背光的情况下动态改频以形成旋律。
- 默认占空比保持低（例如 6–12%）以降低“刺耳感”与器件风险；靠节奏/音高区分为主。

## 声音目录（SoundId）

> 说明：SoundId 是“可播放的声音集合”，事件如何映射到它由 `SoundEvent → SoundId` 规则决定。

### Action（短促）

- `ACTION_OK`：确认音（动作成功）
- `ACTION_FAIL`：失败音（动作失败/无效）
- `ACTION_FAULT`：故障提示音（动作因系统故障/安全门控被拒绝，或设备处于严重报警态导致动作不被接受）

### ModeMelody（一次性 ≥5s）

- `MELODY_MODE_READY`
- `MELODY_MODE_CHARGE`
- `MELODY_MODE_DISCHARGE`
- `MELODY_MODE_LOWBATT`
- `MELODY_AC_LOST`
- `MELODY_AC_RESTORED`

补充：开机旋律

- 每次 ESP32 启动后，延迟约 0.9s 播放一次开机旋律（当前复用 `MELODY_AC_RESTORED` 的 pattern）。
- 目的：给用户一个“系统已启动且可交互”的听觉确认；不依赖 AC 边沿事件。

### NoticeOnce（一次性 ≥2s）

- `NOTICE_INFO_ONCE`
- `NOTICE_WARN_ONCE`
- `NOTICE_ERROR_ONCE`

### AlarmLoop（持续循环）

- `ALARM_LATCHED_LOOP`：不可自动恢复（硬故障/寿命/需人工介入）
- `ALARM_THERMAL_LOOP`：温度/保护类（可恢复但紧急）
- `ALARM_COMM_LOOP`：通信/状态不可用（影响判断）
- `ALARM_LOWBATT_LOOP`：低电量（固定慢节奏，≈8s/次）

## 事件清单与映射（SoundEvent → SoundId）

> 原则：只对“用户需要关注”的状态变化发声；对持续状态做边沿触发与冷却，避免刷屏。

### A. 按键动作（所有按键动作必须有反馈）

触发来源：`button_task`（五向键：center/up/right/down/left）。

动作契约：

- 每个按键触发的动作必须产出结果：`Ok / Fail / Fault`。
- **仅在动作完成后**上报提示音事件（不是按下瞬间）。

映射：

- `Ok` → `ACTION_OK`
- `Fail` → `ACTION_FAIL`
- `Fault` → `ACTION_FAULT`

报警期间要求：

- `AlarmLoop` 激活时仍要播放 Action 声音；
- Action 不改变报警“状态”，仅作为短促插播；插播策略见“调度策略”。

### B. 模式切换（ModeMelody）

#### DashboardMode 旋律（5–7s）

来源：UI 顶部模式（`ui::DashboardMode`），目前为 `Ready / Charge / Discharge / LowBatt`。

- `Ready` → `MELODY_MODE_READY`
- `Charge` → `MELODY_MODE_CHARGE`
- `Discharge` → `MELODY_MODE_DISCHARGE`
- `LowBatt` → `MELODY_MODE_LOWBATT`

触发规则：

- 仅在模式切换“稳定确认”后播放（建议稳定 1.5–2s）。
- 同一旋律建议 30s 冷却，避免模式抖动反复播放。

#### AC 掉电/恢复旋律（5–7s）

来源：`PowerState.ac_present / ac_stable`（定义见 `docs/charging_policy.md`）。

- `ac_present: true → false` → `MELODY_AC_LOST`（风格更抓耳但仍是旋律）
- `ac_stable: false → true` → `MELODY_AC_RESTORED`（柔和恢复）

### C. 一般事件（NoticeOnce，≥2s）

下列事件一律只在“进入/发生”时播放一次，并做冷却（建议 10–30s），避免抖动刷屏。

- 温度暂停/恢复（充电策略）：`temp_pause_effective` 边沿
  - 进入暂停 → `NOTICE_WARN_ONCE`
  - 退出暂停 → `NOTICE_INFO_ONCE`
- INA226 测量故障/恢复：`ina226_fault` 边沿
  - 故障 → `NOTICE_WARN_ONCE`
  - 恢复 → `NOTICE_INFO_ONCE`
- `IN_PG` 读取失败/恢复：`in_pg_read_failed` 边沿
  - 失败 → `NOTICE_WARN_ONCE`
  - 恢复 → `NOTICE_INFO_ONCE`
- smart-battery 配置漂移（`CHG_CONFIG` drift）：
  - 检测到 drift 并尝试修复 → `NOTICE_WARN_ONCE`
  - 修复失败或持续写失败（短时间内重复）→ `NOTICE_ERROR_ONCE`
- Boot/Bring-up 降级项（开机阶段只需一次，归入 WARN/ERROR）：
  - LCD init failed → `NOTICE_WARN_ONCE`
  - STM32 I2C selfcheck failed → `NOTICE_WARN_ONCE`
  - TCA6408A init failed → `NOTICE_WARN_ONCE`

### D. 严重事件（AlarmLoop，持续循环直到退出）

严重事件的来源以 `SbFaultCode`（UI 已使用）和电源板/风扇状态为主。

#### 不可自动恢复（LATCHED）→ `ALARM_LATCHED_LOOP`

进入条件（任一满足）：

- `SbFaultCode::FaultBq`
- `SbFaultCode::FaultSc`
- `sc_fault_latched == true`（例如 SC8815 OTP fault 被 latch）
- `pack_critical_fault == true`（STATE_FLAGS 中关键 fault 位）

退出条件：

- 对于“确实可在运行期自动恢复”的条件，退出时停止报警；
- 对于真正的 latch 故障（例如 OTP latched），默认不会退出，直到人工处理/重启（但提示音层仍应支持 Exit 事件以便未来扩展）。

#### 温度/保护类（可恢复但紧急）→ `ALARM_THERMAL_LOOP`

进入条件（任一满足）：

- `SbFaultCode::TempLow / TempHotChg / TempHotDsg`
- `SbFaultCode::OvUvOc`
- 风扇控制进入 `Overtemp`（≥80°C）
- `ups_temp_pause_active == true`（UPS 板温度越限导致放电停机）

退出条件：

- 对应条件解除（TEMP_STATUS 位恢复、Overtemp 退出、UPS 温度恢复等）。

#### 低电量（慢节奏）→ `ALARM_LOWBATT_LOOP`

进入条件：

- `DashboardMode == LowBatt`（目前由最小 cell < 3.2V 且 AC 不存在派生）

退出条件：

- 退出 LowBatt（例如 AC 恢复或电压回升）。

节奏要求：

- 固定慢节奏：≈8s/次（避免长时间吵闹但仍可持续提醒）。

#### 通信/状态不可用 → `ALARM_COMM_LOOP`

进入条件（任一满足，且持续超过阈值才升级为 Alarm）：

- `SbFaultCode::CommErr`
- `SbFaultCode::StateNA`
- 其它“影响判断”的通信故障持续存在（例如 INA226 故障长期不恢复）

退出条件：

- 通信恢复或状态可用。

## 默认节拍参数（实现基线）

> 说明：以下给出“形状 + 时间尺度”，后续可根据实机试听做小范围调整，但必须保持可辨识度与约束不变。

### Action

- `ACTION_OK`：`40ms tone`
- `ACTION_FAIL`：`40ms tone + 50ms silence + 40ms tone`
- `ACTION_FAULT`：`50ms tone + 50ms silence` × 3

### AlarmLoop（必须包含可插播 Action 的静默窗口）

- `ALARM_LOWBATT_LOOP`（≈8s）：`200ms tone + 200ms silence + 200ms tone + 7400ms silence`
- `ALARM_THERMAL_LOOP`（≈2s）：`200ms tone + 200ms silence + 200ms tone + 1400ms silence`
- `ALARM_LATCHED_LOOP`（≈1s）：`250ms tone + 150ms silence + 250ms tone + 350ms silence`
- `ALARM_COMM_LOOP`（≈6s）：`120ms tone + 120ms silence` × 3 + `≈5280ms silence`

### NoticeOnce（≥2.1s）

建议统一基频与占空比，用节奏区分：

- `NOTICE_INFO_ONCE`：短×4，尾静默补足到 2.1s
- `NOTICE_WARN_ONCE`：中×2，尾静默补足到 2.1s
- `NOTICE_ERROR_ONCE`：中×6，尾静默补足到 2.1s

### ModeMelody（5–7s，可辨识）

要求：

- 每个旋律必须能“靠听区分”，不依赖屏幕；
- 旋律整体偏柔和（低占空比、节奏不过密），但 `MELODY_AC_LOST` 可相对更抓耳。

实现建议（形状）：

- `MELODY_MODE_READY`：下行/低频为主、间隔较大（“安静/就绪”）
- `MELODY_MODE_CHARGE`：上行 motif（“能量上升”）
- `MELODY_MODE_DISCHARGE`：下行 motif（“能量输出”）
- `MELODY_MODE_LOWBATT`：更紧凑的 motif（“焦虑但仍是旋律”）
- `MELODY_AC_LOST`：节奏更醒目（但不循环）
- `MELODY_AC_RESTORED`：柔和回归（不循环）

## 调度策略（PromptToneManager 行为约束）

### 优先级与互斥

- 同一时刻只允许播放一个 `AlarmLoop`，优先级从高到低：
  1) `ALARM_LATCHED_LOOP`
  2) `ALARM_THERMAL_LOOP`
  3) `ALARM_LOWBATT_LOOP`（慢节奏）
  4) `ALARM_COMM_LOOP`
- 任一 `AlarmLoop` 激活时：
  - **drop** 所有 `ModeMelody` 与 `NoticeOnce`（不播放、不入队、不补播）。
  - 允许 `Action` 插播（见下条）。

### 报警期间的 Action 插播

- 目标：按键反馈音延迟可控（建议 ≤250ms），且不破坏报警“持续可感知”。
- 策略：当 `AlarmLoop` active 时收到 `Action`：
  - 尽量在下一段 `Silence` 插入 `Action`；
  - 若当前正处于 `Tone`，等待该 Tone step 结束后再插入（因此报警 pattern 必须短 Tone + 足够 Silence）。
- 限流：为防止长按/连击导致报警被 Action 淹没，建议 Action 插播做最小间隔（例如 120–200ms）；超出频率直接丢弃。

### 去重与冷却

- `ModeMelody`：模式稳定确认后才触发；同一旋律 30s 冷却。
- `NoticeOnce`：同类事件 10–30s 冷却；只对边沿触发一次。
- `AlarmLoop`：对边沿触发；退出后立即停止；不补播其它事件。

## 模块边界与接口形状（概要）

### `buzzer`：硬件驱动层

- 职责：封装 LEDC PWM 的频率/占空比设置与停止输出。
- 形状：`start_tone(freq_hz, duty_pct)` / `stop()`

### `prompt_tone`：提示音管理层

- 职责：维护 SoundId 目录与 pattern；处理事件映射与调度；实现非阻塞 tick。
- 运行模型：独立 `tone_task` 持有 manager；其它任务通过 channel 上报事件（必须 `try_send`，不可阻塞关键任务）。

事件来源建议：

- `button_task`：按键动作结果（Ok/Fail/Fault）
- `power_task`：AC 边沿、SbFaultCode 边沿、TEMP_STATUS、SC8815/UPS 温度状态、通信故障等
- `thermal_task`：风扇模式（Safe/Overtemp）边沿
- `ui_task`：DashboardMode 边沿（稳定确认后上报）

## 兼容性与迁移

- 仅新增提示音模块与事件上报，不改变现有充电/放电/温控策略本身。
- 提示音为旁路能力；即便提示音层故障，也不得阻塞其它任务。

## 验收标准（设计基线）

- 非阻塞：播放提示音期间，`button_task`/`power_task`/`thermal_task`/`ui_task` 的节奏不被阻塞。
- 模式旋律：`Ready/Charge/Discharge/LowBatt` 以及 `AC lost/restored` 的旋律可稳定区分（≥5s）。
- 一般事件：温度暂停/恢复、INA226 故障/恢复、PG 读失败/恢复、配置漂移等事件能以 ≥2s 的一次性提示音告知用户，且不刷屏。
- 严重事件：
  - 可恢复严重（温度/保护/OVUVOC）进入后循环报警，退出后立即停止。
  - 不可自动恢复严重（FaultBq/FaultSc/OTP latch）进入后循环报警，音型与可恢复明显不同。
  - 低电量报警为慢节奏（≈8s/次）。
- 报警期间：
  - `ModeMelody` 与 `NoticeOnce` 不播放、不排队、不补播；
  - 任意按键动作完成仍有 Action 声音反馈，且延迟可控（建议 ≤250ms）。
