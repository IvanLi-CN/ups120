# Smart-Battery NTC / TEMP_STATUS Integration Notes

This file records bring-up issues and fixes for the STM32L051-based
smart-battery firmware related to the `feat/smart-battery-ntc-temp` branch.

## 2025-12-05 – NTC / TEMP_STATUS Integration Causing STM32 Exceptions

**Context**

- Branch: `feat/smart-battery-ntc-temp`
- MCUs:
  - STM32L051C8 – `firmware/smart-battery`
  - ESP32S3 – `firmware/ups-main`
- New features on this branch:
  - Unified thermal policy and TEMP_STATUS (0x23) on STM32.
  - Extended compact temperature window 0x40..0x47 (int8 °C).
  - NTC + MCU temperature aggregation feeding both TEMP_STATUS and the
    smart-battery I2C window.

**Symptom**

- After ESP32 has been running for a short time:
  - STM32 RTT occasionally reports:
    - `Firmware exited unexpectedly: Exception`
    - Frame at `__INTERRUPTS @ 0x080000b8` (HardFault-style abort).
  - ESP32 logs show repeated I2C NACKs when talking to the smart-battery:
    - `stm32: temp read failed: kind=nack`
    - `stm32: vbat read failed: kind=nack`
    - `stm32: ibat read failed: kind=nack`
    - `smart-battery: cfg read failed`
- Once the STM32 core has faulted, the I2C slave at 0x35 stops responding
  until the STM32 is reset or reflashed.

**Root Cause (high level)**

The instability only appeared on `feat/smart-battery-ntc-temp`, i.e. after
introducing the new NTC/MCU temperature paths and TEMP_STATUS wiring. The
legacy `main` branch (no NTC task, no extended window) did not exhibit this.

While no single line was isolated as “the” bug, the pattern was:

- Enabling the full NTC ADC task (ADC1 + multiple channels + PB12 power
  gating), combined with the new thermal aggregation and I2C mirror logic,
  made the STM32 firmware sporadically hit an exception on real hardware.
- Once the core aborted, the I2C slave disappeared (NACK on all reads),
  leading to the ESP32-side “read failed: kind=nack” storms.

The extended TEMP_STATUS / 0x40..0x47 plumbing itself is not inherently
problematic; the issues only surfaced when the NTC/MCU ADC task was fully
enabled on top of it.

**Fix Strategy**

The fix was implemented in small, hardware-validated stages:

1. **Disable NTC ADC task, keep unified thermal policy**
   - Commented out the `ntc_temp::ntc_temp_task(...)` spawn in
     `firmware/smart-battery/src/main.rs`, leaving:
     - TEMP_STATUS (0x23) updates driven from the BQ task and TMP75/BQ temps.
     - 0x40..0x47 window still populated (with fallbacks), but no NTC/MCU
       contribution.
   - Result: STM32 ran stably; no more exceptions, and ESP32 stopped seeing
     persistent I2C NACKs.

2. **Re-enable NTC task as a stub (no ADC access)**
   - Restored `ntc_temp_task` spawning, but with a trivial implementation:
     - Did not touch ADC1 at all.
     - Periodically published:
       - `t_ntc = [TEMP_INVALID_0_01C; 4]`
       - `t_mcu = TEMP_INVALID_0_01C`
   - Purpose: validate that task wiring + `thermal::update_*` calls do not
     destabilize the system on their own.
   - Result: stable; no STM32 faults and no I2C errors.

3. **Enable MCU internal temperature only**

   - Implemented in `firmware/smart-battery/src/ntc_temp.rs`:
     - Used ADC1 internal temperature channel plus factory calibration
       (`TS_CAL1/TS_CAL2`) to compute MCU temperature in 0.01 °C.
     - Continued to keep all NTC channels invalid:
       - `t_ntc = [TEMP_INVALID_0_01C; 4]`
       - `thermal::update_mcu_temp(t_mcu)`
   - Result:
     - `therm:` logs showed `mcu=<non-sentinel>` values.
     - TEMP_STATUS remained 0x00 in normal ambient.
     - No exceptions / I2C faults observed over extended runs.

4. **Bring up a single NTC channel (TS45 → NTC0)**

   - `ntc_temp_task` was updated so that each period:
     - Reads MCU temperature via internal channel.
     - Drives PB12 (NTC_3V3) high, waits `NTC_WARMUP_MS`, then samples PA0
       (TS45) as NTC0, and turns PB12 back low.
     - Uses the existing LUT to convert the ADC code into
       `t_ntc0_0_01c`.
     - Publishes:
       - `t_ntc = [t_ntc0, INVALID, INVALID, INVALID]`
       - `t_mcu = <MCU temp>`
   - Result:
     - `therm:` logs: `ntc=[<valid>, -32768, -32768, -32768]`.
     - `temp-policy: inputs pack_ntc_max=<matches NTC0> ... mcu=<valid>`.
     - System remained stable; no HardFaults or I2C issues.

5. **Reconnect NTC temperatures to the ESP32 UI (read-only)**

   - On the ESP32 side (`firmware/ups-main`):
     - Kept pack/charger temperatures sourced from the legacy 0x14..0x17
       window (no behavioural change there).
     - Introduced `read_smart_battery_temp_window()` to read the compact
       window at 0x40..0x47 as `[i8; 8]`.
     - In `read_smart_battery_temperatures()`:
       - Filled `SmartBatteryTemps::ntc_c` from the NTC0–3 bytes in the
         compact window (int8 °C → `Option<f32>`).
       - Left pack/chg temperatures unchanged (from 0x14..0x17).
     - The existing `thermal_task` and UI already propagate these NTC values:
       - `ThermalState.sb_ntc_temps_c` ← `SmartBatteryTemps::ntc_c`.
       - `BattDetailData.temps_c` is derived from `sb_ntc_temps_c`.
   - Result:
     - ESP32 log shows:
       - `stm32: temp-window pack=26C chg=25C ntc=[26, 26, 26, 26] ...`
       - `ui: ntc temps slots=Some(26),Some(26),Some(26),Some(26)`
     - On the battery detail page, the four NTC slots now render valid
       temperatures (e.g. `26°C`) instead of `--`.
     - No regression in stability; STM32 remains responsive and ESP32 sees
       no new I2C read failures.

**Takeaways / Guidelines**

- When introducing new ADC/temperature paths on the STM32:
  - Stage changes: first exercise task wiring and state aggregation without
    touching the ADC hardware, then add MCU temperature, and only then bring
    up NTC channels one by one.
  - Keep PB12 (NTC_3V3) gated and avoid leaving the RC network biased when
    not sampling.
  - Use `TEMP_INVALID_0_01C` as the only sentinel for “no reading”; do not
    leak raw out-of-range or rail ADC codes into the thermal snapshot.
- On the ESP32:
  - Continue to treat the legacy 0x14..0x17 window as the primary pack/charger
    temperature source until the compact window has been validated across
    temperature and load conditions.
  - Use the compact 0x40..0x47 window for per-NTC temperatures and MCU/BQ
    observability, but make these consumers tolerant of missing data
    (int8::MIN → `None`).

