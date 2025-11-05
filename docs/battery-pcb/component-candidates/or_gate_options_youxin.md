# 单路或门器件总结

> 目的：在 `hardware_protection_tps3823_tmp75.md` 所述保护链中，为 `FAULT_LATCH` / `ALERT_BUS` 等“异常=高”路径提供单路 2 输入或门，当仅需 OR 组合（例如把两个开漏告警转正）时避免双门器件浪费；整理基于 `schmitt_trigger_nand_options.md` 的选型思路，对常见现货型号进行记录与适配性评估。

## 1. 可选清单

| 型号 / 参考编号 | 逻辑族 | 供电范围 | 静态功耗 | 输出能力 | 封装 | 参考单价 (人民币) | 适配性摘要 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 74LVC1G32GV,125（ID 696756982789） | TI/Nexperia · LVC CMOS | 1.65–5.5 V[^ti-lvc] | 常规 CMOS | 推挽，32 mA 驱动[^ti-lvc] | SOT-753 (TSSOP-5) | [¥0.27](https://item.taobao.com/item.htm?id=696756982789) | 输出电流裕量大，适合 ALERT 总线外接大阻下拉场景 |
| 74AUP1G32GW,125（ID 695609498883） | Nexperia · AUP CMOS | 0.8–3.6 V[^ti-aup] | <1 µA 静态[^ti-aup] | 推挽，≤±4 mA | SOT-353 (SC-70-5) | [¥0.52](https://item.taobao.com/item.htm?id=695609498883) | 低功耗，封装与 `SN74LVC2G132` 单门焊盘尺寸接近 |
| 74AUP1G32GW-Q100H（ID 695037904952） | Nexperia · AUP CMOS（AEC-Q100） | 0.8–3.6 V[^ti-aup] | <1 µA | 推挽，≤±4 mA | SOT-353 | [¥0.57](https://item.taobao.com/item.htm?id=695037904952) | 车规版本，参数与标准版一致，适合需要认证的场合 |

## 2. 电气适配要点

- `WD_RST_N` / `TEMP_ALERT_N` 等信号位于 3.3 V 轨，74LVC1G32 允许 1.65–5.5 V 供电并提供 32 mA 推挽驱动，足以克服 `BQ76920 ALERT` 推荐的 500 kΩ~1 MΩ 下拉，并可直接驱动板载 MOS 反向链路[^ti-lvc]。
- 74AUP1G32 系列支持 0.8–3.6 V 供电，静态电流 <1 µA，输入具施密特迟滞，适合电池待机功耗敏感场景；其推挽输出版可稳定驱动 ALERT 总线和 `PSTOP_CTL` 的 CMOS 负载[^ti-aup]。
- 三个型号均为 SOT‑23/SOT‑353 小封装，焊盘与现有 `SN74LVC2G132` 单门位兼容；AUP 系列体积更小，贴装前需确认产线治具裕量。

## 3. 选型建议

1. 需兼顾功耗与现有板级设计时，优先选择 **74AUP1G32GW,125**：静态电流最低，且和 `schmitt_trigger_nand_options.md` 中推荐的 AUP 族保持一致。
2. 若 ALERT 总线或下游断路器需要更高驱动余量，可选 **74LVC1G32GV,125**，在 3.3 V 供电下提供 ≥32 mA 输出。
3. 对应 AEC-Q100 认证需求时，可采用 **74AUP1G32GW-Q100H**，其电气参数与标准版一致，便于无差异 BOM 切换。

---

[^ti-lvc]: Texas Instruments, *SN74LVC1G32 data sheet*, 特性概述 “1.65-V to 5.5-V 32-mA drive strength OR gate”。
[^ti-aup]: Texas Instruments, *SN74AUP1G32 data sheet*, 特性概述 “Single, 2-input 0.8-V to 3.6-V low-power (< 1uA) OR gate”。
