# Design Memorandum - UPS Project

This document records key design decisions, observations, and debugging notes for the UPS project firmware.

## BQ25730 Configuration and ADC Readings

### Initial `cell_count` Configuration for BQ25730

* **Decision (User Input)**: The `cell_count` parameter passed to `Bq25730::new()` in the firmware is intentionally set to `4`.
* **Rationale**:
  * The BQ25730 is primarily designed for 3.7V/4.2V Li-ion cells. Configuring the `CELL_BATPRESZ` pin for a 5-cell LiFePO4 pack might lead to a default overvoltage protection (OVP) setting around 21V if interpreted as 5S Li-ion.
  * To ensure a safer default OVP level (e.g., around 16.8V for 4S Li-ion) upon initial power-up or in case of I2C communication failure before full configuration, a 4-cell equivalent hardware/default configuration is preferred.
  * The intention is to then use I2C commands to fine-tune all necessary parameters (charge voltage, current, protection thresholds) to precisely match the 5S LiFePO4 battery pack (e.g., ~18V charge voltage).
* **Implication**: This means the firmware must correctly adjust all relevant settings via I2C to override any 4-cell defaults and properly manage the 5S LiFePO4 pack. The `cell_count` parameter passed to `Bq25730::new()` directly influences the `offset_mv` used in `VBAT` and `VSYS` ADC conversions within the `bq25730_async_rs` driver. If the hardware `CELL_BATPRESZ` configuration implies a different cell count than what's passed to `new()`, this can lead to ADC reading inaccuracies.
