# 三输入与门器件总结

> 目的：为 `hardware_protection_tps3823_tmp75.md` 中的三输入组合逻辑（`WD_RST_N`、`TEMP_ALERT_N`、`PSTOP_MCU` → `SAFE_OK`) 挑选可直接在 3.3 V 轨上工作的器件，并记录优信电子淘宝店的现货选项，便于后续 BOM 与样机采购。整理思路参考 `schmitt_trigger_nand_options.md`。

## 1. 可选清单

| 型号 / 淘宝 ID | 逻辑族 | 通道 | V<sub>CC</sub> 范围 | 输入特性 | 输出能力 | 封装 | 参考单价 (RMB) | 适配性摘要 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| [SN74LVC1G11DBVR](https://item.taobao.com/item.htm?id=707087275445) | TI · LVC CMOS | 单门 3 输入 | 1.65–5.5 V[^ti-lvc1g11] | 标准 CMOS 阈值，支持缓慢边沿 | 推挽，±24 mA @3.3 V[^ti-lvc1g11] | SOT‑23‑6 | ¥0.46 | 单门 3 输入，封装占板最少；输出驱动足够拉高 ALERT 总线与 `PSTOP_CTL`。 |
| [SN74LVC1G11DCKR](https://item.taobao.com/item.htm?id=707088575930) | TI · LVC CMOS | 单门 3 输入 | 1.65–5.5 V[^ti-lvc1g11] | 同上 | 推挽，±24 mA @3.3 V[^ti-lvc1g11] | SC‑70‑6 | ¥0.19 | 与 DBVR 电气一致，体积最小；适合紧凑焊盘或共形涂层密集区域。 |
| [74HC11PW-Q100J](https://item.taobao.com/item.htm?id=695479261937) | Nexperia · HC CMOS (Q100) | 三门 3 输入 | 2.0–6.0 V[^nxp-74hc11] | CMOS 阈值 | 推挽，±6 mA @4.5 V[^nxp-74hc11] | TSSOP‑14 | ¥0.84 | 一颗提供 3 路 3 输入，与设计拓扑匹配；车规版本便于未来认证。 |
| [74HC11D,653](https://item.taobao.com/item.htm?id=673038109943) | Nexperia · HC CMOS | 三门 3 输入 | 2.0–6.0 V[^nxp-74hc11] | CMOS 阈值 | 推挽，±6 mA @4.5 V[^nxp-74hc11] | SOIC‑14 | ¥0.52 | 同电气规格，SOIC 易手工返修；适合实验板或低密度布线。 |
| [SN74HC11DR](https://item.taobao.com/item.htm?id=796176058632) | TI · HC CMOS | 三门 3 输入 | 2.0–6.0 V[^ti-hc11] | CMOS 阈值 | 推挽，±6 mA @4.5 V[^ti-hc11] | SOIC‑14 | ¥0.71 | 与 Nexperia 版本互换；当需要 TI 料号或双供策略时可选。 |
| [CD4073BM96](https://item.taobao.com/item.htm?id=673706803453) | TI · CD4000 CMOS | 三门 3 输入 | 3.0–15 V[^ti-cd4073] | 宽裕迟滞，支持慢边沿 | 推挽，±6.8 mA @5 V[^ti-cd4073] | SOIC‑14 | ¥1.03 | 兼容 3.3 V，但传播延迟与功耗均高于 HC/LVC；作为低速备选。 |

> 店内还有 SN74LS11N 等 TTL 器件，仅支持 5 V 供电，阈值不兼容 3.3 V 系统，故不列入推荐。

## 2. 与硬件保护链的适配分析

1. **电源与阈值**  
   - LVC1G11 在 1.65–5.5 V 内工作，输入高阈值约 0.7·V<sub>CC</sub>，完全覆盖 `WD_RST_N` / `TEMP_ALERT_N` / `PSTOP_MCU` 的 3.3 V CMOS 电平，无需额外上拉或电平转换。  
   - HC11 系列要求 ≥2 V 供电，3.3 V 下典型 VIH ≈0.7·V<sub>CC</sub>；与前述信号兼容，同时可在后续若扩展至 5 V 电路时保持可用性。  
   - CD4073 虽支持更宽电压，但在 3.3 V 下的迟滞与传播延迟（典型 200 ns 级[^ti-cd4073]）明显高于 HC/LVC（<10 ns），只建议在极端低频或安规冗余需求时使用。

2. **驱动能力与 ALERT 总线**  
   `hardware_protection_tps3823_tmp75.md` 建议 ALERT 总线上串接 500 kΩ–1 MΩ 下拉；LVC1G11 提供 ±24 mA 驱动余量，HC11 亦可提供 ≥±6 mA，均可在 3.3 V 下快速拉高告警线并驱动 `PSTOP_CTL`。CD4073 的输出电流稍小，但仍足以克服高阻下拉，只是沿速更慢。

3. **封装与布线**  
   - LVC1G11 单门版本适合在现有单门 NAND 焊盘附近直接放置，省去多余逻辑。SC‑70 版本面积最小但焊接难度略高。  
   - 74HC11PW (TSSOP‑14) 可在板上预留三路互锁逻辑时一次完成布线；若需手工调试，可改用 SOIC‑14 版本。  
   - 若板上仅缺少一门 3 输入逻辑（例如替换原先双路 NAND + OR 组合），优先选单门 LVC1G11，可避免多余管脚闲置。

4. **时序**  
   - LVC / HC 门在 3.3 V 下传播延迟 <10 ns，与 TPS3823 的 200 ms 复位窗口及 TMP75 的告警时序相比，可认为近似即时，不会改变保护时序。  
   - CD4073 在 3.3 V 下延迟约 200–400 ns[^ti-cd4073]，虽然仍满足毫秒级保护时间，但若系统后续引入高频抖动滤波，需要重新评估。

## 3. 选型建议

1. **首选：SN74LVC1G11DBVR / DCKR**  
   单门设计即插即用，功耗与驱动能力均满足需求；推荐 DBVR 版本以兼容常规 SOT‑23 焊盘，空间极限时可换 DCKR。
2. **多路合并：74HC11PW-Q100J**  
   当需要在同一芯片内实现多路 3 输入逻辑或考虑车规认证时，选用 TSSOP‑14 车规版本；其电气参数与普通 74HC11 一致。
3. **备选：74HC11D / SN74HC11DR 或 CD4073BM96**  
   SOIC 封装便于手工焊接及实验室验证。若需更大器件间距或对传播延迟不敏感，可选择 CD4073 作为冗余。

后续若确定改版使用单门 LVC1G11，可在 `hardware_protection_tps3823_tmp75.md` 中的逻辑框图直接替换原来“多级 NAND + OR”组合，减少器件数量与功耗。

---

[^ti-lvc1g11]: Texas Instruments, *SN74LVC1G11 Single 3-Input Positive-AND Gate*, SCES580R, 2023. V<sub>CC</sub> 范围、输出驱动和输入特性见表 6-1、6-3。
[^nxp-74hc11]: Nexperia, *74HC11; 74HCT11 Triple 3-input AND gate*, Rev. 9, 2022-05-26. 供电与输出能力见表 7/8。
[^ti-hc11]: Texas Instruments, *SN74HC11 Triple 3-Input Positive-AND Gates*, SCLS240U, 2022. 电气特性参见表 6-1。
[^ti-cd4073]: Texas Instruments, *CD4073B CMOS Triple 3-Input AND Gate*, SCLS097N, 2022. 传播延迟与输出能力见表 7-3、特性描述。
