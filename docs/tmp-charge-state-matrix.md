# UPS120 Charge / Request State Matrix (TEMPORARY DRAFT)

> Temporary design note for aligning ESP32 charge *requests* with STM32
> smart‑battery *actual* charge state and defining UI behaviour.  
> This file is for discussion only and can be deleted/replaced after
> the scheme is agreed.

## Signals Used

- **Host command (ESP32 → STM32)**
  - `CHG_CONFIG` (0x31, on STM32, written by ESP32):
    - `AUTO` (bit0): always `0` in UPS120; all decisions are host‑driven.
    - `MANUAL_ENABLE` (bit1): `1` = host *requests* charging, `0` = host *requests stop*.
  - ESP32 snapshot field: `PowerState.chg_config` (cached last written value).
  - Derived host command bit in UPS main:
    - `host_manual = (chg_config & SB_CFG_BIT_MANUAL) != 0`
    - `host_req = Charge` if `host_manual == true`; `Stop` otherwise.

- **Actual smart‑battery state (STM32 → ESP32)**
  - `STATE_FLAGS` (0x20, 16‑bit, periodic snapshot from STM32):
    - `AC_PRESENT`  (bit0)
    - `CHARGING`    (bit1)
    - `CHG_PAUSED`  (bit2)
    - `FULL`        (bit4)
    - `BALANCING`   (bit5)
    - `FAULT_BQ`    (bit6)
    - `FAULT_SC`    (bit7)
    - `ACTIVE_SC`   (bit8)
    - `ACTIVE_BQ`   (bit9)
  - UPS main snapshot field: `PowerState.state_flags : Option<u16>`.

- **Pause / cause / temperature**
  - `CHG_PAUSE_CAUSE` (0x32, RO, STM32):
    - `IMBALANCE`    (bit0)
    - `PACK_TEMP`    (bit1)
    - `CHG_TEMP`     (bit2)
    - `OVUV_OC`      (bit3)
    - `HOLD_OFF`     (bit4)
    - `ADAPTER_MISS` (bit5)
    - `EOC_FULL`     (bit6)
  - `TEMP_STATUS` (0x23, STM32) decoded on ESP32 as `TempFaultFlags`:
    - `temp_low`, `temp_high_chg`, `temp_high_dsg`.

- **Power and current**
  - `VBAT` / `IBAT` window (STM32 → ESP32).
    - `IBAT > 0` ≈ pack being charged.
    - `IBAT < 0` ≈ pack discharging.
  - In new scheme, *IBAT is used only for “direction / magnitude” display*,
    **not** to infer “charging vs not charging”.

- **UPS main snapshot**
  - `PowerState` already contains:
    - `ac_present`, `ac_stable`
    - `chg_config` (last written)
    - `state_flags` (cached STM32 flags)
    - `sb_temp_status`, `temp_pause_active`
    - `vbat_mv`, `ibat_ma`, `out_enabled`, …

---

## State Matrix (Host Command × STM32 Actual × Cause) and UI Behaviour

Legend:

- `host_manual` = `(chg_config & MANUAL_ENABLE) != 0`
- `stm_flags`  = `STATE_FLAGS` (when available)
- `CHG`        = `STATE_FLAGS.CHARGING`
- `PAUSE`      = `STATE_FLAGS.CHG_PAUSED`
- `FULL`       = `STATE_FLAGS.FULL`
- `AC`         = `STATE_FLAGS.AC_PRESENT`
- `CM`         = `CHG_PAUSE_CAUSE` bits (high‑level reason)

Planned UI fields (for later implementation):

- **Top‑line mode text**: `MODE: …`
- **Battery short tag** in dashboard third line: `CHG / IDLE / DSG / REQ-CHG / REQ-STOP / ERR`
- Optional **detail text** (e.g. in batt detail screen or second line) can show reason.

### Group A — Host requests *charging* (`host_manual = 1`)

| Case | Host cmd (`MANUAL`) | STM32 `STATE_FLAGS` (AC / CHG / PAUSE / FULL / FAULT*) | Typical CM / temp cause                           | IBAT sign (typical) | State semantics                                         | UI top mode (proposal)       | Battery tag (proposal) | Notes |
|------|---------------------|---------------------------------------------------------|---------------------------------------------------|---------------------|---------------------------------------------------------|--------------------------------|------------------------|-------|
| A1   | 1 (request charge)  | `AC=1, CHG=1, PAUSE=0, FULL=0, FAULT=0`                | `CM=0`, TEMP_STATUS clear                         | `IBAT > 0`          | 正常充电会话进行中；充电请求已被 STM32 接受并执行。     | `MODE: CHARGE`                | `CHG`                 | “健康充电”基线状态。 |
| A2   | 1                   | `AC=1, CHG=1, PAUSE=1, FULL=0, FAULT=0`                | `CM.PACK_TEMP / CHG_TEMP / IMBALANCE / OVUV_OC`   | ≈0 / 小电流         | 适配器在，主机请求充电，**会话存在但处于暂停**。        | `MODE: CHARGE (PAUSE)` or `MODE: CHARGE` + pause icon | `CHG` or `CHG*`       | 不算“请求/实际不匹配”，STM32 报告仍处于充电状态机中。 |
| A3   | 1                   | `AC=1, CHG=1, PAUSE=0 or 1, FULL=1, FAULT=0`           | 通常 `CM.EOC_FULL=1`                              | `IBAT` 小正值或≈0    | 已达到 FULL，仍处于 CV/均衡阶段，充电通道逻辑上打开。   | `MODE: CHARGE (FULL)`         | `CHG`                 | FULL 但 CHG=1：优先当作“充电中但已满/维护阶段”。 |
| A4   | 1                   | `AC=1, CHG=0, PAUSE=0, FULL=1, FAULT=0`                | 常见：`CM.EOC_FULL=1`                              | `IBAT` ≈0           | 主机仍保持 MANUAL=1，但 STM32 报告“已满且不再充电”。   | `MODE: REQUEST CHARGE (FULL)` | `REQ-CHG`             | **请求≠实际**（请求充电但不再流动）；UI 强调“已满”。 |
| A5   | 1                   | `AC=0, CHG=0, PAUSE=0, FULL=0, FAULT=0`                | `CM.ADAPTER_MISS=1`                               | `IBAT` ≤0           | 主机请求充电，但 STM32 报告“适配器缺失/离线”。         | `MODE: REQUEST CHARGE (NO AC)`| `REQ-CHG`             | **请求≠实际**，典型：AC 拔掉但 ESP32 尚未撤销请求。 |
| A6   | 1                   | `AC=1, CHG=0, PAUSE=0, FULL=0, FAULT≠0`                | `CM.OVUV_OC` 或 FAULT_BQ/FAULT_SC 置位            | `IBAT` ≤0 或≈0      | 主机请求充电，但 STM32 因故障/保护而完全停止充电。     | `MODE: REQUEST CHARGE (FAULT)`| `REQ-CHG` or `ERR`    | **请求≠实际**，UI 应突出安全故障，而非简单“未充电”。 |
| A7   | 1                   | `AC=1, CHG=0, PAUSE=0, FULL=0, FAULT=0`                | `CM=0`，无 FULL，可能为短暂过渡/实现缺陷           | `IBAT` 任意         | 主机请求充电，但 STM32 既不充电也不报告 FULL/暂停。    | `MODE: REQUEST CHARGE (?)`    | `REQ-CHG` + `!`       | 视为异常/调试态，应在日志中重点关注。 |

> **“请求充电状态”定义**：所有 `host_manual = 1` 且 `CHG = 0` 的组合（A4/A5/A6/A7）。  
> UI 顶部可统一采用 `MODE: REQUEST CHARGE`，第三行用 `REQ-CHG`，并在细节中根据 FULL / AC / FAULT / CM 原因区分。

### Group B — Host requests *stop / not charging* (`host_manual = 0`)

| Case | Host cmd (`MANUAL`) | STM32 `STATE_FLAGS` (AC / CHG / PAUSE / FULL / FAULT*) | Typical CM / temp cause                       | IBAT sign (typical) | State semantics                                           | UI top mode (proposal)         | Battery tag (proposal) | Notes |
|------|---------------------|---------------------------------------------------------|-----------------------------------------------|---------------------|-----------------------------------------------------------|----------------------------------|------------------------|-------|
| B1   | 0 (request stop)    | `CHG=0, FULL=0, FAULT=0` (AC 任意)                     | `CM=0`                                        | `IBAT` ≈0 或 <0     | 正常“未充电”状态；请求停止充电，STM32 也报告未在充电。 | `MODE: STANDBY` or `MODE: DISCHARGE` (按 IBAT 正负) | `IDLE` / `DSG`        | 主 UI 基线：不在充电、请求与实际一致。 |
| B2   | 0                   | `CHG=0, FULL=1`                                        | `CM.EOC_FULL=1` 或已维持 FULL 状态           | `IBAT` ≈0           | 已满电且不再充电。                                       | `MODE: STANDBY (FULL)`          | `IDLE`                | 典型“已满待机”状态。 |
| B3   | 0                   | `CHG=0, FAULT≠0`                                      | `CM.OVUV_OC` 或其它故障                       | `IBAT` 任意         | 未充电且存在电池侧故障；主机也不再请求充电。           | `MODE: STANDBY (FAULT)`         | `ERR` or `IDLE`       | 非 mismatch，但 UI 应突出故障。 |
| B4   | 0                   | `AC=1, CHG=1, PAUSE=0, FULL 任意`                      | `CM` 任意                                     | `IBAT ≥ 0`          | **主机请求停止充电，但 STM32 报告仍在充电会话中。**     | `MODE: REQUEST STOP (STILL CHG)`| `REQ-STOP`            | 典型“请求停止充电状态”，需调查命令/通信路径。 |
| B5   | 0                   | `AC=1, CHG=1, PAUSE=1, FULL 任意`                      | `CM` 含温度/失衡等                            | `IBAT` ≈0           | 主机请求停止，但 STA 仍保持“充电会话+暂停”标志。       | `MODE: REQUEST STOP (PAUSED)`   | `REQ-STOP`            | 同样视为“请求停止充电状态”，但当前无电流。 |
| B6   | 0                   | `AC=0, CHG=1`（理论上不应出现）                       | 若出现多半为实现 bug                          | `IBAT` 任意         | 适配器不存在却仍标记 CHARGING，视为严重异常。           | `MODE: REQUEST STOP (ERROR)`    | `ERR`                 | 仅用于调试/保护；UI 应突出异常。 |

> **“请求停止充电状态”定义**：所有 `host_manual = 0` 且 `CHG = 1` 的组合（B4/B5/B6）。  
> UI 顶部可统一采用 `MODE: REQUEST STOP`，第三行用 `REQ-STOP`，并在细节中强调“仍在充电 / 暂停 / 异常”。

### Group C — Flags unavailable / I²C errors

| Case | Host cmd (`MANUAL`) | STM32 flags                                           | Semantics / UI behaviour                                                      |
|------|---------------------|-------------------------------------------------------|-------------------------------------------------------------------------------|
| C1   | 0 或 1              | `state_flags = None`（读取失败或 STM32 离线）        | 无法信任“实际充电状态”；UI 顶部可以显示 `MODE: UNKNOWN`，第三行显示 `ERR`，并在细节中展示通信错误（`stm32: … read failed` 日志），禁止再基于 IBAT 做“是否在充电”的判断。 |

在 C 组情况下，**禁止**从 IBAT 推断“是否充电”，只能展示“请求状态 + 通信错误”，提醒用户检查 STM32 侧。 |

---

## Summary / Intended UI Rules (for future implementation)

1. “是否在充电” 的判定：
   - 首选 STM32 `STATE_FLAGS.CHARGING` / `CHG_PAUSED` / `FULL`，**不再使用 IBAT 阈值**。
2. “请求状态” 的判定：
   - 使用 ESP32 已写入的 `CHG_CONFIG.MANUAL_ENABLE` 位（`host_manual`）：
     - `1` → “请求充电”
     - `0` → “请求停止充电”
3. **正常状态**：
   - `host_manual == 1 && CHG == 1` → “充电中”（A1/A2/A3）。
   - `host_manual == 0 && CHG == 0` → “未充电”（B1/B2/B3）。
4. **请求 / 实际不匹配状态**：
   - `host_manual == 1 && CHG == 0` → “请求充电状态”（A4/A5/A6/A7；再根据 FULL / AC / FAULT 区分文案）。  
   - `host_manual == 0 && CHG == 1` → “请求停止充电状态”（B4/B5/B6）。
5. **通信异常**：
   - `state_flags == None` → “未知实际状态”，UI 只显示“请求 + 通信错误”，禁止任何基于电流的推断。

这一矩阵可以作为后续修改 `PowerState`、UI 层模式枚举与文案的基础，实现完全“由 STM32 报告真实状态 + 由 ESP32 报告请求状态”的 UI。

