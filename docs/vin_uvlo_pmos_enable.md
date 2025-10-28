## TPS62933 VIN 欠压 + REGOUT 理想二极管方案（纯 MOS 实现）

本备忘简述一种仅使用 MOSFET 与分压电阻来替换原 **D4** 肖特基，满足以下约束：

- **以 VIN 为欠压判据。**
- **REGOUT 只允许连接 MOS 栅极**，不得承担负载或被外部拖低。
- 兼顾防反灌与极低压降，确保 VIN 与 REGOUT 任一不足时，`3V3_EN` 都保持低电平。

### 1. 拓扑概述

```
VIN ──┬───────────────────────────────┐
      │                               │
      │                        ┌──────▼─────┐
      │                        │    Q1      │  P-MOSFET，源→VIN，漏→3V3_EN
      │                        │  (20V/低Rds)│  例：Diodes DMP2005UFG-7
      │                        │             │
      └─ RPU 1MΩ ────────────►│ Gate        │
                               └─────▲──────┘
                                     │
                                     │
                              ┌──────┴──────────────┐
                              │        Q2           │  N-MOSFET（例：BSS138）
                              │ Drain ──────────────┘
                              │ Source ──┐
                              └──────────┘
                                         │
VIN ─ RUVH ─┐                            ▼
   200kΩ    ├─────► 节点 X ─── Q3 ───► BGND
VIN ─ RUVL ─┘                      （源→地、漏→节点 X、栅→REGOUT）
   82kΩ

3V3_EN ─ RPD 470kΩ ─ BGND
可选：3V3_EN ─ Rhys 1MΩ ─ Q2 Gate（≈百 mV 迟滞），以及 Q1 Gate ↔ VIN 加 CGS 47–100pF
```

### 2. 元件职责

- **Q1（P-MOS）**：为 TPS62933 提供“理想二极管”通路。RPU 将 Gate 拉到 Source，默认关断；只有在 Q2、Q3 同时导通时 Gate 才被拉低。
- **Q2（N-MOS）**：对 VIN 进行欠压判定。其栅极由 `RUVH/RUVL` 分压获得，满足 `V_G ≥ V_GS(th)` 才具备拉低能力。
- **Q3（N-MOS）**：由 REGOUT 驱动，只负责给节点 X 提供地回路。**REGOUT 唯一连接对象就是 Q3 Gate**，确保任何状态下 REGOUT 不会被外界拖动。
- **RPD**：在高边关断时把 `3V3_EN` 迅速拉向地。

### 3. 欠压门限计算

Q2 的栅极电压：

```
V_G(Q2) = V_IN × RUVL / (RUVH + RUVL)
```

门限条件：`V_G(Q2) ≥ V_GS(th,Q2)`，因此：

```
V_IN,UVLO ≈ V_GS(th,Q2) × (RUVH + RUVL) / RUVL
```

以 BSS138（典型 VGS(th)=1.2V，最大约 2V）为例：

- 目标 6V：取 RUVH=270kΩ、RUVL=68kΩ ⇒ 典型门限 ≈ 6.0V。
- 目标 5V：取 RUVH=180kΩ、RUVL=82kΩ ⇒ 典型门限 ≈ 4.1V。

设计时请同时检查 VGS(th) 的最小/最大值，并根据需要选择阈值范围更紧的 MOSFET。

### 4. 工作时序

1. **VIN 欠压或 REGOUT 低**：  
   - Q2 或 Q3 任一关断，Q1 Gate 被 RPU 拉到 VIN ⇒ P 管关断，3V3_EN 被 RPD 拉低。
   - Q1 Gate 与 Source 等电位，REGOUT 与外界完全隔离。

2. **VIN ≥ 门限 且 REGOUT 高**：  
   - Q3 导通给节点 X 提供地回路；Q2 也满足阈值，拉低 Q1 Gate ⇒ P 管导通，3V3_EN ≈ VIN。

3. **掉电**：  
   - 只要 VIN 或 REGOUT 任一跌破条件，Q1 Gate 立即回到 VIN，P 管彻底关断；Rhys（若存在）提供少量迟滞，可避免门限附近抖动。

### 5. 实施要点

1. **移除原 D4**（SOD‑123FL 肖特基，`REGOUT ↔ 3V3_EN`）。  
2. **新增 Q1/Q2/Q3 及电阻网络**，所有参考地接 BGND。  
3. **PCB 布线**：Q1 源/漏铜皮短而粗；Gate 网络细线并远离高 dV/dt 节点。  
4. **调试**：慢升 VIN，记录 3V3_EN 跃升电压；如偏离预期，调整 RUVH/RUVL 或换阈值合适的 Q2。  
5. **验证**：  
   - VIN 欠压 + REGOUT 正常 → 3V3_EN 应保持低电平。  
   - VIN 正常 + REGOUT 低 → 3V3_EN 同样为低。  c
   - 双高 → 3V3_EN ≈ VIN。  
   - 示波 Q1 Gate，确保尖峰 < ±20V（DMP2005UFG VGS 额定 ±20V）。

这样即可在保持“REGOUT 仅连接栅极”的前提下，实现 VIN 欠压保护 + 低压差理想二极管功能。

