# Battery Temperature Sensing Design

## Objectives

- Provide reliable cell-surface temperature monitoring for the LiFePO₄ 26650 pack.
- Keep sensing current near zero during idle to minimize pack self-discharge.
- Maintain firmware compatibility with the STM32L051C8T6 ADC input (VREF+ tied to VDDA).

## Operating Window

- Control window: 0 °C to 50 °C (charge 0–45 °C per cell datasheet, discharge limited to 0–50 °C per project requirement in `docs/battery-pcb/README.md`).
- Sensor capability: 10 kΩ/B3380 NTC supports −55 °C to 125 °C per datasheet NTCM-10K-B3380, so excursions beyond 0–50 °C can still be detected and logged as faults.

## Sensor & Bias Selection

- NTC: 10 kΩ @ 25 °C, B = 3380 K, 1 % tolerance, glass-bead MF51 style, 1.2–1.3 mm bead, 66 mm enamelled leads (spec available from multiple vendors).
- Pull-up: 43 kΩ ±1 %, 1/16 W or higher, tied to MCU GPIO when enabled.
- Filter capacitor: 100 nF X7R to ground at ADC pin for noise suppression.
- Rationale:
  - 10 kΩ curve matches common BMS libraries and existing calibration tables.
  - 43 kΩ provides >0.6 V midpoint at 25 °C and keeps divider current <80 µA across full range.
  - B3380 beta fits expected thermal profile while retaining compatibility with prior firmware.

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

- Affix the bead to the 26650 cell mid-body or negative tab using thin thermal adhesive tape plus Kapton overwrap.
- Route the enamelled leads alongside the harness and terminate into the JST/board connector with strain relief.
- Avoid compressing the bead; maintain direct thermal contact with minimal insulating material.

## Recommended Bill of Materials

| Item | Description | Notes |
| ---- | ----------- | ----- |
| NTC | 10 kΩ, B3380 K, 1 %, glass-bead MF51, 1.2–1.3 mm head, 66 mm leads | Ensure procurement matches 10 kΩ resistance, B3380 K beta, 1 % tolerance, ≈66 mm leads |
| R_pullup | 43 kΩ ±1 % resistor, 0402, 3.3 V rated | Choose low-tempco thick-film where possible |
| C_filter | 100 nF, 50 V, X7R capacitor, 0402 | Place close to ADC pin |

## Firmware Checklist

- Implement GPIO power gating with configurable warm-up delay.
- Use 50 kΩ-compatible sampling time and optional oversampling for noise reduction.
- Compare readings against project safety envelopes (charge allowed 0–45 °C, discharge allowed 0–50 °C); log violations and hand off to the existing protection logic.
- Periodically self-test: detect open/short via out-of-range ADC codes and flag sensor fault.

## References

- NTCM-10K-B3380 datasheet (docs/tme/NTCM-10K-B3380.pdf).
- STM32L051C8 datasheet Tables 15, 58, 59 (`docs/stm32l051c8/STM32L051C8_Part_001.md`, `docs/stm32l051c8/STM32L051C8_Part_002.md`).
- Wiltson Energy 26650 LiFePO₄ operating temperature guidance (web reference captured via Tavily search).
