# NAND 与非门器件评估

使用关键词“与非门”对指定渠道的 48 个结果进行筛选，并结合数据手册核对关键电气参数，整理出可满足《hardware_protection_tps3823_tmp75.md》硬件保护链需求的单路与双路 NAND（与非）器件。

## 1. 单路 2 输入 NAND

| 型号 | 逻辑族 | 输入特性 | V<sub>CC</sub> (V) | 输出能力 | 封装示例 | 渠道单价 (CNY) | 适配要点 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| SN74AHC1G00DBV[^price-ahc1g00] | AHC | **施密特触发**，容忍慢沿 | 2.0–5.5[^ti-ahc1g00] | 推挽，±8 mA @ 5 V[^ti-ahc1g00] | SOT-23-5、SC-70-5 | 0.25 | 放置在 `FAULT_LATCH` 前级抑制温度告警慢上升沿。 |
| SN74AUP1G00DPW[^price-aup1g00] | AUP | 典型 250 mV 迟滞，3.6 V 容限 | 0.8–3.6[^ti-aup1g00] | 推挽，±4 mA @ 3.3 V[^ti-aup1g00] | X2SON-5、SOT-353 | 0.49 | 提供低功耗与迟滞，可直接驱动 `PSTOP_CTL`。 |
| SN74LVC1G00DBVR[^price-lvc1g00] | LVC | 标准 CMOS 阈值（**无施密特**） | 1.65–5.5[^ti-lvc1g00] | 推挽，±24 mA @ 3.3 V[^ti-lvc1g00] | SOT-23-5、SC-70-5 | 1.76 | 适合作为整形后的末级反相或 `ALERT` 推挽输出。 |

## 2. 双路 2 输入 NAND

| 型号 | 逻辑族 | 输入特性 | V<sub>CC</sub> (V) | 输出能力 | 封装示例 | 渠道单价 (CNY) | 适配要点 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| SN74LVC2G132DCUR[^price-lvc2g132] | LVC | **双通道施密特触发** | 1.65–5.5[^ti-lvc2g132] | 推挽，±24 mA @ 3.3 V[^ti-lvc2g132] | VSSOP-8、X2SON-8 | 0.91 | 单颗器件覆盖 `FAULT_LATCH` 与 `STOP_REQUEST`，提供噪声裕量。 |
| SN74LVC2G00DCUR[^price-lvc2g00] | LVC | 标准 CMOS 阈值 | 1.65–5.5[^ti-lvc2g00] | 推挽，±24 mA @ 3.3 V[^ti-lvc2g00] | VSSOP-8、US8 | 1.42 | 适用于前级已整形的场景，提供更高驱动或占位替换。 |

## 3. 与硬件保护链的适配建议
- **前级故障合成**：`SN74LVC2G132` 与 `SN74AHC1G00` 均具施密特输入，可按《hardware_protection_tps3823_tmp75.md》第 3 章拓扑整合 `WD_RST_N`、`TEMP_ALERT_N`、`PSTOP_MCU`，抑制慢沿与抖动。[^ti-lvc2g132][^ti-ahc1g00][^hardware-doc]
- **末级翻转/缓冲**：`SN74AUP1G00` 在 3.3 V 下兼具迟滞与低静态功耗，适合直接驱动 `PSTOP_CTL` 或 `ALERT`。若需要更大灌/拉电流，可在整形节点使用 `SN74LVC1G00` 或 `SN74LVC2G00` 替换。[^ti-aup1g00][^ti-lvc1g00][^ti-lvc2g00]
- **版图兼容性**：上述器件覆盖 SOT-23-5、SC-70-5、SOT-353、VSSOP-8 等封装，与现有 PCB 工艺兼容，可无缝替换现有单/双路 LVC 器件。

## 4. 后续动作
- 采购 `SN74LVC2G132DCUR` 与 `SN74AUP1G00DPW` 样品，验证施密特整形及低功耗末级在硬件链路中的噪声余量。
- 若验证显示仍需更高驱动，在末级切换至 `SN74LVC1G00` / `SN74LVC2G00` 并更新《hardware_protection_tps3823_tmp75.md》的逻辑实现。

---

[^price-ahc1g00]: 渠道页面商品 ID 704713346062，访问日期 2025-01-30，促销单价 0.25 CNY。
[^price-aup1g00]: 渠道页面商品 ID 696330677234，访问日期 2025-01-30，促销单价 0.49 CNY。
[^price-lvc1g00]: 渠道页面商品 ID 672658816379，访问日期 2025-01-30，促销单价 1.76 CNY。
[^price-lvc2g132]: 渠道页面商品 ID 714861431492，访问日期 2025-01-30，促销单价 0.91 CNY。
[^price-lvc2g00]: 渠道页面商品 ID 714651342193，访问日期 2025-01-30，促销单价 1.42 CNY。
[^ti-ahc1g00]: Texas Instruments, *SN74AHC1G00 Single 2-Input Positive-NAND Gate*, SCLS313Q, Jan. 2024.
[^ti-aup1g00]: Texas Instruments, *SN74AUP1G00 Low-Power Single 2-Input Positive-NAND Gate*, SCES604J, Dec. 2016.
[^ti-lvc1g00]: Texas Instruments, *SN74LVC1G00 Single 2-Input Positive-NAND Gate*, SCES212AB, Apr. 2014.
[^ti-lvc2g132]: Texas Instruments, *SN74LVC2G132 Dual 2-Input NAND Gate With Schmitt-Trigger Inputs*, SCES547D, Dec. 2013.
[^ti-lvc2g00]: Texas Instruments, *SN74LVC2G00 Dual 2-Input Positive-NAND Gate*, SCES193N, Jan. 2015.
[^hardware-doc]: 《hardware_protection_tps3823_tmp75.md》，第 3 章“逻辑实现”。
