# Smart Battery Firmware – Protection & Charger Bring-up Design

## Scope

This document captures the software architecture that now boots the STM32L051C8
smart-battery firmware into a safe operating state. The firmware sequence
initializes the TI **BQ7692003** protection IC (CRC variant @ 0x08) ahead of the
**SC8815** charger so that the pack’s safety envelope is validated before any
power stage is enabled. It also describes the shared I2C bus topology, gating
controls, and telemetry loops that underpin bring-up.

## Hardware Interfaces

- **I2C2 – INNER bus (PB10/PB11)**: Operated at 100 kHz with DMA1_CH4/CH5 and
  serviced through the shared I2C2 interrupts. A global `StaticCell` stores the
  bus so multiple async drivers can borrow it via `embassy-embedded-hal`
  shared-bus mutexes. Both the BQ7692003 (fixed `0x08`) and SC8815 (`0x11`) ride
  this bus.
- **SMBus Alert (PB5) & Alert GPIOs**: Reserved for future SMBus/alert handling;
  interrupt lines `PB1` (BQ alert) and `PB2` (inner bus INT) are wired for EXTI
  wakeups.
- **CE (PA10)**: Active-low charger enable. Held high during safety bring-up,
  driven low only after the protection IC reports healthy configuration.
- **PSTOP (PA9)**: Active-high gate for the SC8815 power stage. Remains high
  until charger programming succeeds and stays an emergency kill path for any
  detected charger fault.
- **Discharge Enable (PB9)** / **Charge Allow (PA1)**: Active-low digital inputs
  sampled by the BQ task so external logic can veto FET operation by default.

## Initialization Sequence

1. Configure MCU clocks (LSE on) and instantiate CE/PSTOP outputs high to keep
   the charger path disabled. Set up the `PB9`/`PA1` control inputs with pull-ups
   so the protection FETs default to OFF.
2. Bring up I2C2 with DMA and register it inside a `StaticCell<Mutex<…>>`. This
   shared handle feeds lightweight `I2cDevice` wrappers for each peripheral at
   the moment they need bus access.
3. Initialize the global pub/sub channels; capture the BQ-specific publishers so
   alerts and measurement frames can be streamed once the protection loop runs.
4. Enter a blocking loop that repeatedly attempts to configure the BQ7692003 via
   `Bq769x0` with CRC enabled:
   - Apply per-cell thresholds (OV 3.65 V, UV 2.80 V), 15 A short-circuit, 10 A
     discharge overcurrent, and `rsense = 3 mΩ` to match the hardware shunt.
   - On success, spawn the asynchronous `bq76920_task`, which keeps verifying the
     pack, manages FET states, and publishes telemetry. Until this point the
     charger remains disabled; failures back off for 5 s before retrying.
5. Once the protection stage is alive, wait 10 ms, drive CE low, and delay
   another 100 ms to satisfy the SC8815 wake timing.
6. Create an `I2cDevice` for SC8815 over the same mutex-protected bus, call
   `init()`, push the charger configuration (5 mΩ sensing, 800 mA limits,
   Charging mode, 450 kHz switching, 60 ns dead time, VINREG 11.5 V, VBAT ratio
   forced, with trickle/termination enabled), disable OTG, and start ADC
   conversions. Any error forces CE/PSTOP high and exits early.
7. Hold PSTOP high for an additional 100 ms, then drive it low to energize the
   power stage. Runtime monitoring continues to guard against faults.

## Runtime Behaviour

- **Protection loop**: The spawned `bq76920_task` reuses the shared bus to fetch
  measurements, confirm register integrity, and assert FET control. Configuration
  verification failures keep both FETs disabled and log detailed diagnostics.
- **Charger loop**: The SC8815 owner task executes once per second, dumping
  status and ADC readings through `defmt`. OTP or VBUS/VBAT short flags trigger
  an immediate safety response—raising PSTOP/CE and leaving the loop active for
  inspection.
- **Telemetry plumbing**: Measurement publishers returned by `shared::init_pubsubs`
  are wired for future aggregation; SC8815 values are still logged locally while
  the BQ producers feed alerts/measurements into the shared channels.

## Future Enhancements

- Integrate SC8815 telemetry into the pub/sub fabric so higher-level logic can
  arbitrate charging without relying on logs.
- Implement SMBus alert servicing on PB1/PB5 to react more quickly to protection
  trips rather than polling.
- Add explicit recovery routines (e.g., staggered retries or manual clear hooks)
  once the protection IC reports a cleared fault, keeping the safety-first
  posture that now gates charger bring-up.
