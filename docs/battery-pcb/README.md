# Smart Battery Module

## Overview

The smart battery board manages a 5-series stack of 26650 cells for the UPS120 platform. A TI **BQ7692003** analog front end measures every cell tap, while the PCB routes each tap through 100 ohm / 1 uF RC filters to keep the monitor within its input slew limits. The stack exports `BAT` as the pack positive node and `BGND` as the pack negative reference.

## Cell and Pack Specifications

- **Cell**: Planned 26650-format LiFePO₄ cell rated 3.2 V nominal, ~4.0 Ah capacity, ≤25 mΩ internal resistance, and up to 5C discharge capability (per supplied datasheet).
- **Environmental limits**: Charge temperature 0 °C to 45 °C per the cell spec; project discharge temperature currently constrained to 0 °C to 50 °C to preserve lifespan.
- **Physical**: Cylindrical can ~26 mm × 65 mm, mass approximately 85 g ± 2 g.
- **Pack topology**: 5-series, 1-parallel (5S1P) stack managed by the board’s cell monitor.
- **Pack ratings**: Nominal voltage about 16 V (5 × 3.2 V) with planned operating window 12.5 V (discharge cutoff) to 18.0 V (charge cutoff). Total energy around 64 Wh at nominal capacity.
- **Charge cutoff**: System charger terminates near 18.0 V pack, equivalent to ~3.6 V per cell, consistent with the cell datasheet limit.
- **Discharge cutoff**: Smart-battery firmware configures the BQ7692003 undervoltage trip at 2.50 V per cell (`firmware/smart-battery/src/main.rs:201`), producing the 12.5 V pack cutoff target above.
- **Charging architecture**: On-board charger regulates pack current at approximately 1 A with constant-current/constant-voltage behavior coordinated by the MCU.
- **Continuous current**: Power-stage sizing currently aligns with the cell’s 5C guidance (~20 A) while considering MOSFET thermal headroom and shunt limits; adjust fuse/shunt if future validation dictates tighter limits.

## Functional Blocks

- **Protection and switching** - A TI **BQ76200** high-side driver coordinates the charge, discharge, and pre-charge MOSFET banks. The low-value current-sense shunt between `BGND` and `GND` captures pack current, while a resettable fuse protects the housekeeping supply input.
- **Temperature sensing** - A 10 kOhm MF52 NTC thermistor on TS1 feeds the BQ7692003, backed by a bias network and surge clamp so the monitor reads accurate pack temperatures.
- **Housekeeping power** - A **TPS62933** buck converter drops the pack voltage to the `3V3_OUT` rail through a shielded inductor and bulk storage capacitor. An **LM66100** ideal diode isolates the regulated `3V3` rail, allowing external 3.3 V injection via the VBUS header without back-feeding the buck. Distributed 1 uF and 100 nF capacitors decouple the MCU and analog front ends.
- **System controller** - An **STM32L051C8** MCU supervises the AFE through an internal I2C bus and bridges to the host-facing bus. A resistor network provides 3.3 V pull-ups, and dedicated alert and interrupt lines land on the MCU for fault handling.
- **External interfaces** -
  - 1.0 mm 5-pin harness reserved for internal signal acquisition, carrying the monitored I2C pair plus the alert line, 3.3 V reference, and return.
  - 1.0 mm 4-pin SWD header dedicated to MCU debug access (`SWDIO`, `SWCLK`, supply, return).
  - 2.54 mm 4-pin power inlet routed to the on-board charger, intended for the external charger supply input (VBUS and ground pairs).
  - Dual 6-pin high-current terminals exporting the battery stack positive and negative nodes for pack integration.
- **User interaction** - A top-side reset switch reaches the MCU reset pin, while dedicated LED driver outputs fan out for front-panel indicators.

## Reference Netlist

The schematic netlist exported from EasyEDA lives in `netlist_battery.enet`. Each component entry includes manufacturing metadata (LCSC code, footprint ID, datasheet URL) to keep fabrication and BOM exports aligned with the PCB project.

## Directory Layout

- [`README.md`](README.md) - High-level description of the smart battery PCB (this file).
- [`netlist_battery.enet`](netlist_battery.enet) - EasyEDA JSON netlist for the smart battery design.
- [`tps62933_enable_control.md`](tps62933_enable_control.md) - Notes about the TPS62933 enable and power-sequencing logic.
- [`vbats_cutoff_calibration.md`](vbats_cutoff_calibration.md) - SC8815 VBATS divider hardware plus 18 V charge-cutoff calibration procedure.
- [`hardware_protection_tps3823_tmp75.md`](hardware_protection_tps3823_tmp75.md) - Consolidated hardware fault-chain design notes (final build uses scheme D: TMP75 + MCU AND gating).
- [`battery_temp_sensing.md`](battery_temp_sensing.md) - Analog temperature sensing topology for the pack.
- [`component-candidates/`](component-candidates) - Sourcing digests and option tables ([`schmitt_trigger_nand_options.md`](component-candidates/schmitt_trigger_nand_options.md), [`nand_gate_options_youxin.md`](component-candidates/nand_gate_options_youxin.md), [`or_gate_options_youxin.md`](component-candidates/or_gate_options_youxin.md), [`single_inverter_options.md`](component-candidates/single_inverter_options.md), [`three_input_and_gate_options_youxin.md`](component-candidates/three_input_and_gate_options_youxin.md), [`temperature_protection_tmp.md`](component-candidates/temperature_protection_tmp.md), [`watchdog_pricing_taobao.md`](component-candidates/watchdog_pricing_taobao.md), [`youxin_schmitt_single_nand.md`](component-candidates/youxin_schmitt_single_nand.md)).
