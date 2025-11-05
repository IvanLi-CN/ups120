# SC8815 VBATS Charge-Termination Calibration

## 1. Objective

Guarantee an **18.0 V** charge termination for the 5S LiFePO₄ stack when SC8815 operates with an external VBATS divider, and provide a consistent calibration flow for production and service.

## 2. Fixed Parameters

| Item | Typical | Tolerance |
| ---- | ---- | ---- |
| Upper resistor `Rtop` | 140 kΩ | ±1 % |
| Lower resistor `Rbottom` | 10 kΩ | ±1 % |
| SC8815 reference `VBATS_REF` | 1.203 V | ±0.5 % |

SC8815 regulates according to:

```
VBAT_limit = VBATS_REF × (1 + RUP / RDOWN)
RUP   = Rtop + (1 − α) · Rp
RDOWN = Rbottom + α · Rp
0 ≤ α ≤ 1
```

`Rp` is the total potentiometer resistance and α is the normalized wiper position measured from the ground side.

## 3. Potentiometer Range Requirement

Solve `VBAT_limit = 18.0 V` while sweeping the component corners (Rtop/Rbottom ±1 %, VBATS_REF ±0.5 %). Enforcing `0 ≤ α ≤ 1` gives:

```
α ≥ 0  ⇒ Rp ≥ m·Rbottom − Rtop
α ≤ 1  ⇒ Rp ≥ (Rtop − m·Rbottom) / m
m = 18 / VBATS_REF − 1
```

The tightest case yields:

```
Rp_actual ≥ 3.18 kΩ
```

If the potentiometer tolerance is ±25 %, the nominal value must satisfy `Rp_nom × (1 − 0.25) ≥ 3.18 kΩ`, thus:

```
Rp_nom ≥ 4.3 kΩ
```

## 4. Selected Component

Use a standard 5 kΩ trimmer; even at the −25 % corner it still provides 3.75 kΩ, safely above the requirement. Recommended part:

- **VGF39NCHXT-B502** (HDK/Hokuriku) — 270° ± 20° single turn, linear (B) taper, 0.15 W @ 70 °C, 50 V max, residual resistance ≤200 Ω, no post-solder cleaning.

## 5. Wiring

Connect the potentiometer end terminals to the existing 140 kΩ / 10 kΩ divider nodes and route the wiper directly to VBATS. After soldering, rotate fully toward ground and back off ~5° to reduce end-stop residual resistance.

## 6. Calibration Procedure

1. Apply 18.5 V / 0.5 A from a lab supply; monitor PACK voltage with a calibrated ≥4½-digit DMM while SC8815 enters constant-voltage mode.
2. If PACK voltage exceeds 18.0 V, rotate counter-clockwise (away from ground); if it is below 18.0 V, rotate clockwise.
3. Wait until the voltage stabilizes at **18.00 V ±0.03 V** for at least 5 s, then record the knob position and reading.
4. Power down, lock the wiper with a small drop of UV or conformal adhesive, cure, and confirm the voltage remains in range.
5. Log timestamp, fixture ID, and final voltage in the production traceability system.

## 7. Residual Error Summary

| Source | Typical impact |
| ---- | ---- |
| Wiper repeatability ±1 % travel | ≈ ±0.13 V (≤±0.05 V with fixtures) |
| VBATS_REF ±0.5 % | ±0.09 V |
| Divider tempco 25 ppm/°C (0–60 °C) | ±0.04 V |
| Potentiometer aging (70 °C/1000 h ±5 %) | ≈ ±0.9 V, plan annual recheck |

Recalibrate if drift exceeds ±0.2 V; beyond ±0.5 V, adjust SC8815 software limits as an additional guard.
