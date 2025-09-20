# Smart Battery Firmware – Protection & Charger Bring-up Design

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
