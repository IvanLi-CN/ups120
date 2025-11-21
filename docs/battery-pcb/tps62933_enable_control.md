## TPS62933DRLR Enable Path – Design Note

### 1. Purpose

Whenever the battery monitor **BQ76920** enables its `REGOUT` regulator, the downstream buck converter **TPS62933DRLR** must also be allowed to start.
Conversely, when the board enters the BQ76920 “shipping mode” (REGOUT floats), TPS62933 must remain disabled regardless of VIN.
This document summarises the enable thresholds of TPS62933 and specifies the external gating network that fulfils those requirements.

---

### 2. Background

#### 2.1 TPS62933 EN pin (datasheet SLUSEA4D)

| Parameter | Min | Typ | Max |
| --- | --- | --- | --- |
| EN rising threshold | 1.21 V | 1.24 V | 1.28 V |
| EN falling threshold | 1.10 V | 1.14 V | 1.17 V |
| EN pin leakage | — | a few µA (internally pulled high) | — |

> EN ≥ 1.24 V typically enables the converter. EN ≤ 1.14 V disables it.
The converter also has an internal VIN UVLO (~3.3–3.6 V), but an external divider on EN is the preferred way to impose a higher start‑up threshold.

#### 2.2 BQ76920 REGOUT rail

| BQ76920 state | REGOUT behaviour | Comment |
| --- | --- | --- |
| Normal operation | 3.3 V regulator active | powers the MCU and the logic network |
| Shipping / deep sleep | High impedance, collapses towards 0 V | TPS62933 must stay off even if VIN is present |

---

### 3. Enable network topology

```
            VIN_TPS
               │
               │  (P-channel, TR2)
               ├───NX3008CBKS,115──┬───────────┐
               │                   │           │
               │                   ▼           │
               │                 R47 100 kΩ    │
               │                   │           │
               │                 EN_TPS        │ → TPS62933 EN pin
               │                   │           │
               │                 R48 12 kΩ     │  (pull-down)
               │                   │           │
               └─R39 100 kΩ────────┴───●───────┘
                                 Gate node (Q11 pin5)
                                       │
                             R59 470 kΩ│
                                       ▼
                          Q11 pin6 (N MOS drain)
                                       │
                        NX3008 (N-channel, TR1) controlled by REGOUT
                                       │
                                      GND

        Gate node ↔ VIN_TPS : CESD5V0D5 (SOD-523 TVS/ESD)
```

**Key points（rev4.1 调整后）**

1. The P-MOSFET (TR2) forms a high-side switch between `VIN_TPS` and the EN divider R47/R48.
2. The N-MOSFET (TR1) accepts `REGOUT` and, via **R59=470 kΩ**, pulls the P-gate low only when REGOUT = 3.3 V; the large series resistor limits gate/TVS current to <0.1 mA even at 5‑cell full charge.
3. R39=100 kΩ biases the P-gate back to `VIN_TPS` whenever REGOUT is low or floating.
4. **TVS 改为 CESD5V0D5 (SOD‑523)**，仍跨 `VIN_TPS`（阴极）↔ Gate 节点（阳极），只负责尖峰钳位；静态 |VGS| 由 100 k/470 k 分压决定，5S 满充时 |VGS| ≈ 0.176·VIN ≈ 3.7 V，远低于 NX3008 ±8 V 额定。
5. EN 分压不变（100 kΩ / 12 kΩ），VIN≈11–12 V 时 EN≈1.2 V 触发 TPS62933；R48 继续保持 EN 下拉确保关断。

---

### 4. Behaviour by operating mode

| Condition | TR1 (N-MOS) | TR2 (P-MOS) | EN_TPS | Result |
| --- | --- | --- | --- | --- |
| `REGOUT = 0 V` or floating, any VIN | OFF | Gate pulled to VIN by R49 → OFF | ≈0 V (R48 pull-down) | TPS62933 disabled |
| `REGOUT = 3.3 V`, VIN present | ON, pulls gate low (limited by SMFJ5.0A) | ON | EN ≈ VIN × 12 kΩ/(100 kΩ+12 kΩ) ≈ 0.107·VIN | TPS62933 enabled if EN ≥ 1.24 V |
| `REGOUT` high, VIN falling | Gate follows VIN via R49, TR2 turns off before VIN < threshold | EN follows divider → internal EN/UVLO handles shutdown | Clean disable, no reverse feed |

Shipping-mode entry is therefore safe: REGOUT float → TR1 off → TR2 gate rises to VIN → divider cut off → EN forced low.

---

### 5. Component roles (NX3008CBKS,115)

| Device | Pins | Function in this design |
| --- | --- | --- |
| TR2 (P-channel) | pin4=Source to VIN, pin3=Drain to R47/EN, pin5=Gate | Connect or disconnect VIN from the EN divider. Provides isolation in shipping mode. |
| TR1 (N-channel) | pin1=Source to GND, pin2=Gate from REGOUT, pin6=Drain at gate node | Acts as the low-side driver: REGOUT=3.3 V → TR1 pulls TR2 gate low; REGOUT low/floating → gate released. |
| R49 (100 kΩ) | VIN → gate node | Default bias: keeps TR2 off when REGOUT is inactive; defines clamp current through TVS. |
| SMFJ5.0A | gate node → VIN | Clamps VGS to ~6–7 V, protecting NX3008 from >±8 V stress when VIN is high. |
| R47/R48 divider | 100 kΩ / 12 kΩ | Produces ~1.2 V at EN when VIN≈11–12 V, matching the TPS62933 enable window. |

---

### 6. Practical notes

1. **TVS orientation** – connect SMFJ5.0A single-ended: cathode to VIN_TPS, anode to the gate node.
2. **Component placement** – keep the gate node compact; place R47/R48 near the TPS62933 EN pin for noise immunity.
3. **REGOUT control** – REGOUT should never be driven while floating: an I/O configured as pull-up will mimic the 3.3 V condition and allow the buck to start, as intended.
4. **Optional diagnosis** – if desired, monitor EN_TPS with an ADC; when REGOUT asserts and VIN is above threshold, EN ought to be ≳1.2 V.
5. **VIN range** – the divider is purely resistive; no dynamic interaction with VIN ripple is expected. If VIN greatly exceeds 12 V, the TVS must have sufficient power margin (SMFJ series is 200 W peak, ample for this use).

---

### 7. Summary

By gating the EN divider with NX3008CBKS and protecting its gate using SMFJ5.0A, TPS62933 only sees a valid enable voltage when BQ76920’s REGOUT is asserted and VIN is high enough.
This ensures:

- shipping-mode isolation (REGOUT floating → EN forced low),
- clean enable sequencing driven by REGOUT,
- compliance with TPS62933’s EN thresholds without exposing devices to over-voltage.
