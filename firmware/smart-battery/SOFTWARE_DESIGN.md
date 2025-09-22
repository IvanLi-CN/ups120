# Smart Battery Firmware – Protection & Charger Bring-up Design

> 中文要点（外设通信/I2C 对外协议）
>
> - 对外通信总线：I2C1 从机，地址 0x35（7‑bit），引脚 PB6=SCL、PB7=SDA（见 smart-battery.ioc），SMBA 暂不实现。
> - 速率：兼容 100 kHz 与 400 kHz；允许短暂 SCL 拉伸（≤150 µs）。
> - CRC/PEC：遵循 TI/SMBus 习惯。
>   - 写操作：主机每写 1 个数据字节，紧跟 1 个 CRC8（poly 0x07，初值 0x00），校验 [ADDR_W, REG, DATA]，CRC 错误在 CRC 字节处 NACK，整帧丢弃。
>   - 读操作：设备按 [DATA, CRC] 交错返回；首字节 CRC 计算 [ADDR_R, DATA0]，后续仅对各自 DATAi 计算。
> - 寄存器：1 字节地址，自增；多字节 LE；提供电压/电流/温度/故障与充电软控制（详见下文 Register Map）。
> - 选择 I2C1 的原因：支持 STOP 唤醒，满足低功耗场景对外主机唤醒的需求。

## Scope

This document captures the software architecture that now boots the STM32L051C8
smart-battery firmware into a safe operating state. The firmware sequence
initializes the TI **BQ76920** protection IC (CRC variant @ 0x08) ahead of the
**SC8815** charger so that the pack’s safety envelope is validated before any
power stage is enabled. It also describes the shared I2C bus topology, gating
controls, and telemetry loops that underpin bring-up.

## Hardware Interfaces

- **I2C2 – INNER bus (PB10/PB11)**: Operated at 100 kHz with DMA1_CH4/CH5 and
  serviced through the shared I2C2 interrupts. A global `StaticCell` stores the
  bus so multiple async drivers can borrow it via `embassy-embedded-hal`
  shared-bus mutexes. Both the BQ76920 (fixed `0x08`) and SC8815 (`0x11`) ride
  this bus.
- **I2C1 – OUTER bus (external host interface)**: Configured as 7‑bit I²C slave
  to allow a system host (MCU/SoC) to query pack telemetry and command limited
  charging control. I²C1 is used because it supports wake‑from‑STOP on address
  match (WUPEN), enabling ultra‑low‑power idle while remaining responsive.
  Pins per `.ioc`: PB6 = I2C1_SCL, PB7 = I2C1_SDA, PB5 = I2C1_SMBA (alert).
  The slave address is `0x35` (7‑bit). Supported bus rates: 100 kHz and 400 kHz.
  Clock stretching is permitted (≤ 150 µs) while copying a fresh telemetry
  snapshot into the I²C TX buffer. General‑call is disabled.
- **SMBus Alert (PB5) & Alert GPIOs**: Reserved for future SMBus/alert handling;
  interrupt lines `PB1` (BQ alert) and `PB2` (inner bus INT) are wired for EXTI
  wakeups.
- **CE (PA10)**: Active-low charger enable. Held high during safety bring-up
  and whenever no SC8815 activity is required. The firmware asserts it low only
  when the charger must be configured, telemetry must be sampled from the
  SC8815, or charging is actively commanded.
- **PSTOP (PA9)**: Active-high gate for the SC8815 power stage. Remains high
  until charger programming succeeds and stays an emergency kill path for any
  detected charger fault.
- **EXIT_SHIPMODE (PA1)**: Push-pull GPIO used to wake the BQ76920 from ship
  mode with a high pulse before configuration retries.

## Initialization Sequence

1. Configure MCU clocks (LSE on) and instantiate CE/PSTOP outputs high to keep
   the charger path disabled. Prepare the `PA1` wake GPIO so the BQ76920 can be
   nudged out of ship mode if it fails to respond on the first attempt.
2. Bring up I2C2 with DMA and register it inside a `StaticCell<Mutex<…>>`. This
   shared handle feeds lightweight `I2cDevice` wrappers for each peripheral at
   the moment they need bus access.
3. Initialize the global pub/sub channels; capture the BQ-specific publishers so
   alerts and measurement frames can be streamed once the protection loop runs.
4. Enter a blocking loop that repeatedly attempts to configure the BQ76920 via
   `Bq769x0` with CRC enabled:
   - First boot attempt performs an immediate configuration using per-cell
     thresholds (OV 3.65 V, UV 2.50 V), 15 A short-circuit, 10 A discharge
     overcurrent, and `rsense = 3 mΩ` to match the hardware shunt.
   - If this initial communication fails, pulse `EXIT_SHIPMODE` high, hold the
     line asserted for a full 500 ms to exit ship mode, then drop it low before
     retrying configuration.
   - Subsequent failures fall back to a 1 s retry cadence without repeating the
     ship-mode pulse. MOSFETs and the charger stay disabled until configuration
     succeeds.
   - On success, spawn the asynchronous `bq76920_task`, which keeps verifying the
     pack, manages FET states, and publishes telemetry.
5. Once the protection stage is alive, wait 10 ms, drive CE low, and delay
   another 100 ms to satisfy the SC8815 wake timing.
6. Reassert `PSTOP` high immediately before configuring the SC8815. Create an
   `I2cDevice` for SC8815 over the same mutex-protected bus, call `init()`, push
   the charger configuration (10 mΩ sensing, 800 mA limits, Charging mode,
   450 kHz switching, 60 ns dead time, VINREG 11.5 V, VBAT ratio forced, with
   trickle/termination enabled), disable OTG, and start ADC conversions. Any
   error forces CE/PSTOP high and exits early.
7. Hold PSTOP high for an additional 100 ms, then drive it low to energize the
   power stage. Runtime monitoring continues to guard against faults.

## Runtime Behaviour

- **Protection loop**: The spawned `bq76920_task` reuses the shared bus to fetch
  measurements, confirm register integrity, and assert FET control. Configuration
  verification failures keep both FETs disabled and log detailed diagnostics. The
  task acquires a fresh measurement frame once per second and republishes pack
  voltage, pack current, per-cell voltages, temperatures, MOS status, and alert
  bits via the `BQ76920_MEASUREMENTS` pub/sub queue so downstream tasks (such as
  the charger controller and USB bridge) always have up-to-date data.
- **Pack-voltage supervision**: When the BQ76920 reports healthy status, the
  firmware holds the discharge FET enabled whenever protection flags are clear so
  the SC8815 VBAT sense always tracks the pack. Only undervoltage, short, or
  overcurrent faults—and the 12.5 V cutoff—force the discharge FET open. If
  charging is permitted by the BQ76920, the firmware consults the reported pack
  voltage to decide SC8815 gating: below 17.0 V, drive the CE/PSTOP sequence to
  begin charging; at 18.5 V, halt charging even if other conditions remain true;
  at 12.5 V or lower, immediately disable output FETs and charger gates to
  prevent deep discharge. A
  hardware erratum leaves
  the ALERT pin permanently asserted, so software now forces the charge FET on
  whenever voltage and protection limits are satisfied, even if the
  `OVRD_ALERT` flag latches back in. The alert bit is still logged, but it no
  longer vetoes the charge path.
- **Charger loop**: The SC8815 owner task wakes every second, consumes the most
  recent BQ76920 measurement frame,并按以下规范管理“会话创建/功率级放行/暂停”：
  - 会话创建（仅两条，必须同时满足）：
    1) `Vpack < 17.0 V`；2) BQ76920 无故障（OV/UV/OCD/SCD 均为假）。”spread“不是故障，不参与会话创建判定。
  - 功率级放行（PSTOP 低）与暂停（PSTOP 高）：
    - 正常充电：无暂停条件时，放行功率级（PSTOP 低）。
    - 过压（OV，电池故障）：立即暂停，仅拉高 PSTOP，保持会话不结束；进入 180 s 冷却计时，计时结束且仍满足“会话创建两条”时再放行。
    - 严重不均衡（Δcell ≥ 100 mV，非故障）：立即暂停，仅拉高 PSTOP，保持会话不结束；当 `Δcell < 50 mV` 立刻解除暂停并放行（无时间迟滞）。
    - 其它电池或充电故障（如 UV/OCD/SCD、OTP、VBUS/VBAT 短路等）：立即暂停并结束会话（CE/PSTOP 置高），待故障消除后按会话创建规则重来。
  - 进入充电时序：会话创建成功后，先断开功率级（PSTOP 高）、拉低 CE、延时 100 ms、再释放 PSTOP 以放行功率级。

  Charging is considered active when the SC8815 is enabled and battery charge
  current `IBAT` exceeds 100 mA for three consecutive samples; it is considered
  inactive after three consecutive samples ≤ 80 mA. These thresholds are used
  solely for control and indication logic and do not introduce additional UI
  states.
- **Telemetry plumbing**: Measurement publishers returned by `shared::init_pubsubs`
  keep per-device channels decoupled. The BQ76920 producer updates its queue at
  1 Hz, the SC8815 task pushes charger telemetry on the same cadence, and any
  consumer (USB bridge, logging task, etc.) can subscribe to combine them into an
  `AllMeasurements` snapshot without every task polling individual peripherals.
**Balancing and Charging Coupling (Normative)**

- Evaluation cadence: Balancing SHALL be evaluated once per second (1 Hz).
- Charging/Paused gating: Balancing MAY run only while the system is either
  charging (`expected_charging || charging_confirmed`) or in a charging pause
  (OV cooldown or severe-imbalance pause).
- Start condition: Pack cell spread (max − min) ≥ 10 mV.
- Single-cell only: At any time, at most one cell may be actively balanced. When changing the target cell, firmware shall first disable all balancing FETs, wait a deadtime ≥ 40 ms, then enable the new cell.
- Selection rule: Choose one globally highest-voltage cell that exceeds at least one immediate neighbor by > 1 mV (for end cells, compare the single neighbor; for middle cells, either left or right neighbor suffices).
- Stop condition: Stop balancing when all adjacent cell-to-cell differences are ≤ 1 mV across the pack.
- Charger coupling: While balancing is required or active, the charger SHOULD maintain CV (as
  implemented by the charger task’s policy).
- Adapter loss: If the adapter is lost and the system is not in a charging-pause context, balancing
  shall stop but continue to be evaluated at 1 Hz; if the system is in a charging-pause context
  (OV cooldown or severe-imbalance pause), balancing MAY continue.

**LED Status – Single LED (Normative)**

Only one monochrome LED is available. Patterns below are mandatory and mutually prioritized.

- Priority order (high → low): Fault > Charging (with balancing overlay) > Full (with hysteresis) > Idle.
- Fault (only when a real chip fault bit is set):
  - Battery faults: BQ76920 OV/UV/OCD/SCD = true.
  - Charger faults: SC8815 OTP or VBUS/VBAT short = true.
  - Pattern: 4 Hz flashing (125 ms on / 125 ms off).
- Charging baseline: 1 Hz flashing (500/500).
- Overlay (drawn only during the Charging on-window, and only to indicate “hardware balancing is running”):
  - If and only if bal_cell≠0, insert two 40 ms off notches at 100–140 ms and 300–340 ms.
  - Severe imbalance (Δ≥100 mV, non-fault; including paused state) does not add any overlay; it remains at the Charging baseline unless a chip fault makes it Fault.
- Full (with hysteresis): solid on. If the adapter is present during maintenance/float, briefly turn off for 40 ms every 8 s.
- Idle/Standby: off.

Full detection (parameters):

- Enter full when `VBAT ≥ PACK_CHARGE_STOP_THRESHOLD_MV` and `IBAT ≤ I_term` continuously for 45–90 s.
- Exit full when `VBAT` leaves the float band, or `IBAT ≥ 1.2 × I_term` continuously for ≥ 10 s, or any fault occurs.
- Debounce: 800 ms on all state edges.

## Future Enhancements

- Integrate SC8815 telemetry into the pub/sub fabric so higher-level logic can
  arbitrate charging without relying on logs.
- Implement SMBus alert servicing on PB1/PB5 to react more quickly to protection
  trips rather than polling.
- Add explicit recovery routines (e.g., staggered retries or manual clear hooks)
  once the protection IC reports a cleared fault, keeping the safety-first
  posture that now gates charger bring-up.

---

## Control-Flow Diagrams

Policy note: Balancing now strictly requires AC presence. When the adapter is absent, any active balancing is immediately cleared, and CV-hold requests are withdrawn.

### Tasks, PubSub, and Data Flow

```mermaid
flowchart LR
  MAIN[main]
  BQ[bq76920_task]
  SC[sc8815_task]
  GS[global_state_task]
  LED[led_status_task]

  SCA[Sc8815Alerts]
  SCM[Sc8815Measurements]
  BQA[Bq76920Alerts]
  BQM[Bq76920Measurements]
  BAL[BalancingCvRequest]
  GSTATE[BatteryGlobalState]

  MAIN --> BQ
  MAIN --> SC
  MAIN --> GS
  MAIN --> LED

  SC --> SCA
  SC --> SCM
  BQ --> BQA
  BQ --> BQM
  BQ --> BAL
  GS --> GSTATE

  GS --- SCA
  GS --- SCM
  GS --- BQA
  GS --- BAL
  LED --- GSTATE
  BQ --- SCA
  SC --- BQM
```

---

## External Communications (I2C1 Slave)

This section specifies the on‑board I²C1 slave protocol used by the pack to
communicate with an external host. Summary (Chinese): 智能电池通过 I2C 从机模式与外部通信；
我们使用 I2C1 对外通信，因为其支持从 STOP 状态唤醒；本文定义查询电压/电流/温度/故障及读写充电状态的一组命令与寄存器。

### Link & Electrical

- Address: 7‑bit `0x35` (write: 0x6A, read: 0x6B).
- Modes: Standard‑mode (100 kHz), Fast‑mode (400 kHz).
- Wake: STOP‑mode wake on address match enabled (I2C1.CR1.WUPEN = 1).
- Stretching: The device may stretch SCL up to 150 µs while staging a
  consistent snapshot for multi‑byte reads.
- Filtering: Analog filter enabled, digital filter disabled; no general‑call;
  no 10‑bit addressing.

### Access Model

- Memory‑mapped register bank with auto‑increment.
- Transactions use a 1‑byte register pointer written by the master, followed by
  an optional repeated‑start read of N bytes.
- Byte order: Little‑endian for all multi‑byte quantities.
- Units: Voltage in millivolts (mV), current in milliamps (mA, signed;
  discharge is negative), temperature in centi‑degrees Celsius (c°C, signed).
- Coherency: Telemetry is snapshotted into a TX buffer at 1 Hz; multi‑byte
  reads are internally consistent. 读取侧返回 CRC（与 TI 一致，主机可选择校验，但推荐校验）。

### CRC Policy – TI‑Style (Read & Write)

- CRC: CRC‑8 polynomial 0x07 (x^8+x^2+x+1), initial value 0x00.
- Writes (mandatory, interleaved per byte): For each data byte written, the
  master must append one CRC byte computed over `[SLAVE_ADDR(W), REG_ADDR, DATA]`.
  For block writes with auto‑increment, `REG_ADDR` is the specific address of
  that data byte (i.e., it increments per byte). On CRC mismatch, the device
  NACKs the CRC byte and discards the transaction.
- Reads (device returns CRC interleaved per byte): After the master issues a
  repeated‑start and the read address, the device returns each data byte
  followed by one CRC byte. The first CRC is computed over `[SLAVE_ADDR(R), DATA0]`;
  subsequent CRCs are computed over only the corresponding data byte
  (`[DATAi]`). The master ACKs every data and CRC byte pair, and NACKs the last
  CRC byte to end the transfer.

Examples:

- Write `CHG_ENABLE_REQ` (0x31) = 0x01: `START → 0x6A(W) → 0x31 → 0x01 → CRC(0x6A,0x31,0x01) → STOP`.
- Write `CHG_CURRENT_LIMIT_MA` (0x32..0x33) = 900 (0x0384 LE):
  `START → 0x6A → 0x32 → 0x84 → CRC(0x6A,0x32,0x84) → 0x03 → CRC(0x6A,0x33,0x03) → STOP`.
- Read `VBAT_MV` (0x10..0x11, 2 bytes):
  - Master: `START → 0x6A(W) → 0x10 → REPEATED START → 0x6B(R)`
  - Device returns: `D0_L, CRC0(0x6B,D0_L), D0_H, CRC1(D0_H)`; master NACKs the
    last CRC and STOPs.
- Read a burst (e.g., 8 data bytes starting at 0x10): device returns 16 bytes
  interleaved as `[D0, CRC(D0 with 0x6B), D1, CRC(D1), …, D7, CRC(D7)]`.

### Register Map

Base system info and sequencing (read‑only unless noted):

- 0x00 `SIG0` = 0x53 ('S')
- 0x01 `SIG1` = 0x42 ('B')
- 0x02 `PROTO_VER` = 0x01
- 0x03 `FW_VER_MAJOR`
- 0x04 `FW_VER_MINOR`
- 0x05 `FW_VER_PATCH`
- 0x06 `DEVICE_CAPS` bitfield (RW reserved for future, default 0)
- 0x07 `SYS_STATUS` bitfield (RO): bit0=awake, bit1=stop_capable, bit2=i2c1_ok,
  bit3=bq_ok, bit4=charger_ok, others reserved
- 0x0E RESERVED
- 0x0F RESERVED

Pack measurements (read‑only):

- 0x10 `VBAT_MV_LO`; 0x11 `VBAT_MV_HI` (u16)
- 0x12 `IBAT_MA_LO`; 0x13 `IBAT_MA_HI` (i16; discharge negative)
- 0x14 `T_PACK_Cc_LO`; 0x15 `T_PACK_Cc_HI` (i16)
- 0x16 `T_MOS_Cc_LO`; 0x17 `T_MOS_Cc_HI` (i16)
- 0x18 `V_CELL_MAX_MV_LO`; 0x19 `V_CELL_MAX_MV_HI` (u16)
- 0x1A `V_CELL_MIN_MV_LO`; 0x1B `V_CELL_MIN_MV_HI` (u16)
- 0x1C `DELTA_CELL_MV_LO`; 0x1D `DELTA_CELL_MV_HI` (u16)
- 0x1E `ADAPTER_PRESENT` (RO, 0/1)
- 0x1F `CELLS_PRESENT` (RO, e.g., 4 or 5)

Faults & status (read‑only unless noted):

- 0x20 `BQ_FAULTS` bitfield: bit0=UV, bit1=OV, bit2=OCD, bit3=SCD, bit4=ALERT,
  bit5=OT/UT, bit6=COMM_ERR, bit7=RESERVED
- 0x21 `CHARGER_FAULTS` bitfield: bit0=OTP, bit1=VIN_UV, bit2=VIN_OV,
  bit3=VBAT_OV, bit4=SHORT, bit5=THERM, bit6=COMM_ERR, bit7=RESERVED
- 0x22 `SYSTEM_FAULTS` bitfield: internal safety interlocks; 0 means OK

Charging control and status:

- 0x30 `CHG_STATUS` (RO) bitfield: bit0=charging_active, bit1=precharge,
  bit2=CC, bit3=CV, bit4=full, bit5=balancing, bit6=adapter_present,
  bit7=blocked_by_fault
- 0x31 `CHG_ENABLE_REQ` (RW): 0=disable charging; 1=enable allowed.
- 0x32 `CHG_CURRENT_LIMIT_MA_LO`; 0x33 `_HI` (RW u16; 100…1500 mA typical).

Per‑cell voltages (length depends on `CELLS_PRESENT`, RO):

- 0x50/0x51 `CELL1_MV` (u16)
- 0x52/0x53 `CELL2_MV` (u16)
- 0x54/0x55 `CELL3_MV` (u16)
- 0x56/0x57 `CELL4_MV` (u16)
- 0x58/0x59 `CELL5_MV` (u16; present only on 5S)

Diagnostics & reserved:

- 0x7C `UPTIME_S_LO`; 0x7D `_HI` (RO u16 seconds since boot)
- 0x7E `FRAME_FLAGS` (RO; bit0=snapshot_fresh)
- 0x7F RESERVED

### Semantics & Rules

- Writes to `CHG_ENABLE_REQ` immediately gate charger control logic; the
  firmware may force this back to 0 upon any safety fault. Hosts should treat a
  latched 0 as “charging inhibited until fault is cleared”.
- `CHG_CURRENT_LIMIT_MA` is a soft limit; out‑of‑range values are clamped to the
  nearest supported setting. A value of 0 means “use firmware default”.
- Multi‑byte writes must send the low byte first. Multi‑byte reads are
  contiguous and auto‑increment the pointer; coherency is guaranteed by the
  snapshot mechanism。读侧提供 CRC 供主机可选校验。

### Example Transactions

- Read pack voltage/current/temperature in one burst (8 data bytes → 16 bus bytes with CRC):
  - Master: `START → 0x6A(W) → 0x10 → REPEATED START → 0x6B(R)`
  - Device returns: `VBAT_L, CRC(VBAT_L with 0x6B), VBAT_H, CRC(VBAT_H), IBAT_L, CRC(IBAT_L), IBAT_H, CRC(IBAT_H), TPACK_L, CRC(TPACK_L), TPACK_H, CRC(TPACK_H), TMOS_L, CRC(TMOS_L), TMOS_H, CRC(TMOS_H)`; master NACKs the last CRC then `STOP`.
- Enable charging (with CRC per‑byte):
  - Master: `START → 0x6A(W) → 0x31 → 0x01 → CRC(0x6A,0x31,0x01) → STOP`.
- Set current limit to 900 mA (0x0384 LE; with CRC per‑byte):
  - Master: `START → 0x6A(W) → 0x32 → 0x84 → CRC(0x6A,0x32,0x84) → 0x03 → CRC(0x6A,0x33,0x03) → STOP`.

### Implementation Notes

- The I²C1 ISR wakes the MCU from STOP on address match, stages a copy of the
  most recent telemetry snapshot into a TX buffer, and services RX writes to the
  small control register set. The telemetry producer task updates the snapshot
  at 1 Hz to minimize bus jitter and stretching.
- Fault sources are aggregated from the BQ76920 protection loop and the SC8815
  charger loop, preserving the project’s safety‑first posture: any critical
  fault suppresses charging regardless of host requests.

### BQ76920 Task – Balancing (Strict AC Gating)

```mermaid
flowchart TD
  A0([Start]) --> A1[Read SC alerts]
  A1 --> A2{Adapter present}
  A2 -- No --> A2N[Stop balancing if any] --> A2P[Publish require cv false and overlay false] --> A99([End])
  A2 -- Yes --> A3{Evaluation due}
  A3 -- No --> A7[Prepare publish fields]
  A3 -- Yes --> A4{Charging phase}
  A4 -- No --> A7
  A4 -- Yes --> A5[Read BQ data and compute delta]
  A5 --> A6{Delta large and local peak}
  A6 -- Yes --> A6Y[Set cell balancing] --> A7
  A6 -- No --> A6N[Clear cell balancing] --> A7
  A7 --> A8[Publish balancing request] --> A99
```

### SC8815 Task – Charger Session Lifecycle (Session Begin to Session End)

```mermaid
flowchart TD
  S0([Session begin]) --> S1[Init SC8815 and start ADC]
  S1 --> IF{Init ok}
  IF -- No  --> END_INIT([Session end init fail])
  IF -- Yes --> A[Active]

  A --> PF[Fault pause]
  PF --> A
  A --> PI[Imbalance pause]
  PI --> A

  A --> END_CUT([Session end cutoff])
  A --> END_SCF([Session end sc fault])
  A --> END_STOP([Session end stop])

```

### SC8815 Task – Tick Loop

```mermaid
flowchart TD
  TEND([Tick end])
  T0([Tick start]) --> IN[Read inputs]
  IN --> ADP{Adapter present}
  ADP -- No --> END_REQ([Request session end])
  ADP -- Yes --> SCF{SC fault}
  SCF -- Yes --> END_REQ
  SCF -- No  --> CUT{Pack cutoff le 12.5V}
  CUT -- Yes --> END_REQ
  CUT -- No  --> BQF{BQ critical fault}
  BQF -- Yes --> GATE[Gate PSTOP]
  BQF -- No  --> IMB{Spread ge 100 mV}
  IMB -- Yes --> GATE
  IMB -- No  --> RES{Timer zero and faults cleared}
  RES -- Yes --> UNG[Un gate PSTOP] --> TEND
  RES -- No  --> CHK{Stop candidate Vpack ge 18.5V}
  CHK -- Yes --> BAL{Balancing not complete}
  BAL -- Yes --> CVH[Continue charging CV hold] --> TEND
  BAL -- No  --> END_REQ
  CHK -- No --> KEEP[Maintain PSTOP state] --> TEND

  GATE --> TEND
```

## Operating Thresholds & Timers

- Voltage thresholds
  - Charge start candidate: Vpack < 17.0 V
  - Stop candidate: Vpack ≥ 18.5 V
  - Pack cutoff: Vpack ≤ 12.5 V

- Balancing thresholds
  - Balancing start (by spread): Δ ≥ 10 mV
  - Severe imbalance enter: Δ ≥ 100 mV
  - Severe imbalance release: Δ < 50 mV

- Pauses / holds
  - Critical fault pause (OV/UV/OCD/SCD): 180 s
  - Imbalance pause: no timer; released by Δ < 50 mV
  - CV hold at stop candidate: continue charging while “Balancing not complete”

- Full-state hysteresis (for global state/LED)
  - Enter full: VBAT ≥ 18.5 V and IBAT ≤ 100 mA for 60 s
  - Exit full: IBAT ≥ 120 mA or VBAT < 17.0 V or not charging for 10 s

- Retry / cadence
  - SC session init failure backoff: 5 s
  - Control loop cadence: 1 s

### Global State Aggregation

```mermaid
flowchart TD
  G0([Start]) --> G1[Read inputs]
  G1 --> G2[Compute ac present and charger fault]
  G2 --> G3[Compute charging active and charging paused]
  G3 --> G4{AC present}
  G4 -- Yes --> G4Y[Update full latch timers]
  G4 -- No --> G4N[Clear full latch]
  G4Y --> G5[Compute preparing flag]
  G4N --> G5
  G5 --> G6[balancing active from overlay]
  G6 --> G7{State changed}
  G7 -- Yes --> G8[Publish global state] --> GE([End])
  G7 -- No --> G9[No publish] --> GE
```

### LED State Machine

```mermaid
flowchart TD
  L0([Start]) --> L1{Fault}
  L1 -- Yes --> Lf[Blink 4Hz] --> Lend([End])
  L1 -- No --> L2{Charging}
  L2 -- Yes --> Lc[Blink 1Hz add notches when balancing] --> Lend
  L2 -- No --> L3{Preparing}
  L3 -- Yes --> Lp[Two pulses per second] --> Lend
  L3 -- No --> L4{Full}
  L4 -- Yes --> Lfull[Solid on] --> Lend
  L4 -- No --> Lidle[Off] --> Lend
```
