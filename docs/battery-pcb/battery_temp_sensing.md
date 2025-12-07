# Battery Temperature Sensing Design

## Objectives

- Provide reliable cell-surface temperature monitoring for the LiFePO₄ 26650 pack.
- Keep sensing current near zero during idle to minimize pack self-discharge.
- Maintain firmware compatibility with the STM32L051C8T6 ADC input (VREF+ tied to VDDA).

## Operating Window

- Control window: 0 °C to 50 °C (charge 0–45 °C per cell datasheet, discharge limited to 0–50 °C per project requirement in `docs/battery-pcb/README.md`).
- Sensor capability: 10 kΩ/B3380 NTC supports −55 °C to 125 °C per Cantherm MF52C1103F3380 datasheet, so excursions beyond 0–50 °C can still be detected and logged as faults.

## Sensor & Bias Selection

> Note: The smart-battery PCB also carries an older, optional NTC footprint on the BQ7692003 TS1 pin. On current project hardware this TS1-side NTC is **not stuffed** and TS1 is not used as an external pack sensor; BQ7692003 runs in internal-temperature mode only. All pack-surface temperature sensing described in this document comes from the 4× NTC network into the STM32 ADC.

- NTC: Cantherm MF52C1103F3380, 10 kΩ ±1 % @ 25 °C, B25/50 = 3380 K ±1 %, epoxy bead ≤2 mm with 0.2 mm enamelled copper leads (length can be specified before crimping), 50 mW rating, −55 °C to +125 °C operating window.
- Pull-up: 43 kΩ ±1 %, 1/16 W or higher, tied to MCU GPIO when enabled.
- Filter capacitor: 100 nF X7R to ground at ADC pin for noise suppression.
- Rationale:
  - 10 kΩ curve matches common BMS libraries and existing calibration tables.
  - 43 kΩ provides >0.6 V midpoint at 25 °C and keeps divider current <80 µA across full range.
  - B3380 beta fits expected thermal profile while retaining compatibility with prior firmware.
  - MF52C package matches the enamelled-lead bead format used in the Taobao reference, avoiding PCB rework.

## GPIO-Driven Excitation

- `VCC_SC` is sourced from a push-pull MCU GPIO instead of a permanent 3.3 V rail.
- Sampling sequence:
  1. Set GPIO high.
  2. Wait ≥18 ms (≈5× time constant at worst-case −40 °C with 100 nF).
  3. Start ADC conversion with sampling time ≥160.5 cycles to satisfy the 50 kΩ input impedance limit (STM32L051 datasheet Table 59).
  4. After EOC, switch GPIO to high-Z input to collapse the divider and eliminate static drain.
- Average divider current with 20 % on-duty at 5 Hz polling is ≈12 µA, reducing daily drain below 0.3 mAh.

## ADC Interface Notes

- STM32L051C8T6 LQFP48 package bonds VREF+ internally to VDDA; external reference equals the 3.3 V analog rail (datasheet pin table + Table 58 constraints).
- Configure ADC input as single-ended channel with 3.3 V reference and enable internal averaging/oversampling if noise requires.
- Store Steinhart–Hart coefficients or look-up curves for the 10 k/B3380 thermistor and interpolate to compute °C.

## Mechanical Integration

- Affix the bead to the 26650 cell mid-body or negative tab using thin thermal adhesive tape plus Kapton overwrap; trim the enamelled leads to length after routing.
- Route the enamelled leads alongside the harness and terminate into the JST/board connector with strain relief.
- Avoid compressing the bead; maintain direct thermal contact with minimal insulating material.

## Recommended Bill of Materials

| Item | Description | Notes |
| ---- | ----------- | ----- |
| NTC | Cantherm MF52C1103F3380, 10 kΩ ±1 %, B25/50 = 3380 K ±1 %, epoxy bead ≤2 mm with Ø0.2 mm enamelled leads | Specify ≥65 mm lead length before crimp or solder operations |
| R_pullup | 43 kΩ ±1 % resistor, 0402, 3.3 V rated | Choose low-tempco thick-film where possible |
| C_filter | 100 nF, 50 V, X7R capacitor, 0402 | Place close to ADC pin |

## Firmware Checklist

- Implement GPIO power gating with configurable warm-up delay.
- Use 50 kΩ-compatible sampling time and optional oversampling for noise reduction.
- Compare readings against project safety envelopes (charge allowed 0–45 °C, discharge allowed 0–50 °C); log violations and hand off to the existing protection logic.
- Periodically self-test: detect open/short via out-of-range ADC codes and flag sensor fault.

## Temperature-to-Voltage Table

Divider calculations assume a 43 kΩ pull-up to 3.3 V (`VCC_SC`) with the MF52C1103F3380 thermistor referenced to ground. Values are typical zero-power resistances from the manufacturer’s 3380 K beta specification; apply the ±1 % tolerance envelope when defining firmware thresholds.

| Temp (°C) | R_NTC (Ω) | V_ADIN (V) |
|-----------|-----------:|-----------:|
| −40 | 235 830.76 | 2.7911 |
| −35 | 173 946.05 | 2.6459 |
| −30 | 129 916.74 | 2.4794 |
| −25 | 98 180.09 | 2.2949 |
| −20 | 75 021.69 | 2.0977 |
| −15 | 57 926.35 | 1.8940 |
| −10 | 45 168.27 | 1.6906 |
| −5 | 35 548.39 | 1.4935 |
| 0 | 28 223.73 | 1.3077 |
| +5 | 22 594.95 | 1.1367 |
| +10 | 18 231.40 | 0.9826 |
| +15 | 14 820.50 | 0.8459 |
| +20 | 12 133.17 | 0.7262 |
| +25 | 10 000.00 | 0.6226 |
| +30 | 8 294.61 | 0.5336 |
| +35 | 6 921.92 | 0.4576 |
| +40 | 5 809.87 | 0.3928 |
| +45 | 4 903.40 | 0.3378 |
| +50 | 4 160.14 | 0.2911 |
| +55 | 3 547.27 | 0.2515 |
| +60 | 3 039.19 | 0.2178 |
| +65 | 2 615.81 | 0.1892 |
| +70 | 2 261.28 | 0.1649 |
| +75 | 1 962.99 | 0.1441 |
| +80 | 1 710.89 | 0.1263 |
| +85 | 1 496.90 | 0.1110 |
| +90 | 1 314.50 | 0.0979 |
| +95 | 1 158.41 | 0.0866 |
| +100 | 1 024.32 | 0.0768 |
| +105 | 908.70 | 0.0683 |
| +110 | 808.66 | 0.0609 |
| +115 | 721.79 | 0.0545 |
| +120 | 646.12 | 0.0489 |
| +125 | 580.00 | 0.0439 |

## References

- Cantherm MF52 series datasheet (https://www.cantherm.com/wp-content/uploads/2017/05/cantherm_mf52_1.pdf).
- STM32L051C8 datasheet Tables 15, 58, 59 (`docs/stm32l051c8/STM32L051C8_Part_001.md`, `docs/stm32l051c8/STM32L051C8_Part_002.md`).
- Wiltson Energy 26650 LiFePO₄ operating temperature guidance (web reference captured via Tavily search).
