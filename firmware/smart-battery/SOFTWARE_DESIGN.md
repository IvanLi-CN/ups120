# Smart Battery Firmware – SC8815 Bring-up Design

## Scope

This document captures the minimal software architecture required to establish
reliable communications with the SC8815 charger controller on the STM32L051C8
based smart-battery board. The implementation focuses on the mandatory power
sequencing, charger configuration, and telemetry collection needed to verify
the charger path.

## Hardware Interfaces

- **I2C2 – INNER bus (PB10/PB11)**: Operated at 100 kHz using DMA1_CH4 (TX)
  and DMA1_CH5 (RX) with the shared I2C2 interrupt. Pins correspond to the
  schematic labels `INNER_SCL`/`INNER_SDA`, and the SC8815 default address
  `0x11` is used.
- **CE (PA10)**: Active low enable. The MCU must pull CE low and wait 100 ms
  before issuing any I2C traffic to guarantee the charger is awake.
- **PSTOP (PA9)**: High keeps the power stage disabled; low enables the
  high-side drivers. PSTOP stays high through the entire configuration window
  and is released 100 ms after configuration completes. Any detected fault
  forces PSTOP and CE high again for safety.

## Initialization Sequence

1. Configure the MCU clocks (LSE enabled for stable timing) and instantiate the
   GPIO outputs for CE and PSTOP. Both start high so the SC8815 remains inactive.
2. Bring up I2C1 with DMA and interrupts, matching the hardware muxing shown in
   `smart-battery.ioc`.
3. Pull CE low and delay 100 ms to respect the SC8815 wake-up time.
4. Execute the driver `init()` call while PSTOP is still high.
5. Apply the charger configuration:
   - External voltage divider enabled (`Ru = 140 kΩ`, `Rd = 10 kΩ`, target
     ≈ 18 V).
   - Current-sense resistors fixed to 5 mΩ on both IBUS and IBAT.
   - Input/output limits set to 800 mA.
   - Operating mode `Charging`, switching frequency 450 kHz, dead time 60 ns,
     VINREG 11.5 V, trickle charging and termination enabled, IBAT feedback.
   - VBAT monitor ratio forced to the 12.5× path to match the external divider.
6. Disable OTG mode and enable continuous ADC conversions.
7. Hold PSTOP high for an additional 100 ms, then drive it low to activate the
   power stage.

## Runtime Behaviour

- The firmware keeps ownership of the SC8815 driver and polls every second.
  Each cycle retrieves the device status and ADC measurements. Results are
  logged over defmt for early bring-up visibility.
- Any OTP or VBAT/VBUS short indication triggers an immediate shutdown by
  forcing both PSTOP and CE high. The loop continues running so a debugger can
  inspect state, but the power stage remains disabled until manual re-enable.
- ADC telemetry provides VBUS, VBAT, IBUS, and IBAT readings in physical units
  using the 5 mΩ calibration captured in the driver configuration.

## Future Enhancements

- Integrate this bring-up flow into a dedicated `charger_task` that publishes
  measurements through the existing pub/sub channels and reacts to system level
  charge requests.
- Add explicit recovery logic (e.g., re-enabling PSTOP after manual clearance)
  and expose fault status to higher-level supervision tasks.
- Extend the design to coordinate with the BQ76920 protection IC once both
  tasks are active.
