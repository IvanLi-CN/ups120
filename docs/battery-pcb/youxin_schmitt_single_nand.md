# 单路施密特触发与非门检索记录

本轮检索聚焦于淘宝渠道公开在售的**单路 (single-gate) 与非门**且**输入带施密特触发**的逻辑芯片，作为硬件保护链的候选器件。

## 检索步骤

- 以逻辑 NAND 相关关键词检索，整理 1G/单路器件清单；
- 查阅官方数据手册，确认输入端具备施密特触发迟滞，并记录主要电气指标；
- 排除多路器件（如 74HC132、SN74LVC2G132）或未提供施密特触发特性的型号。

## 符合条件的在售型号

| 芯片型号 | 封装 | 价格 (¥) | 购买链接 | 施密特触发依据 |
| --- | --- | --- | --- | --- |
| SN74AHC1G00DBVR | SOT-23-5 | 1.95 | https://item.taobao.com/item.htm?id=673364782576 | 数据手册列出“Schmitt trigger action at all inputs”[[1]](#ref-ti-ahc1g00) |
| SN74AHC1G00DCKR | SC-70-5 | 0.25 | https://item.taobao.com/item.htm?id=704713346062 | 同上[[1]](#ref-ti-ahc1g00) |
| 74AUP1G00GW,125 | SOT-353 (TSSOP5) | 0.49 | https://item.taobao.com/item.htm?id=696330677234 | 型号说明书明确“Schmitt-trigger action at all inputs”[[2]](#ref-nxp-74aup1g00) |
| 74AUP1G00GX,125 | X2SON-5 | 0.40 | https://item.taobao.com/item.htm?id=696835563731 | 同一数据手册覆盖所有封装[[2]](#ref-nxp-74aup1g00) |
| SN74AUP1G00DCKR | SC-70-5 | 0.96 | https://item.taobao.com/item.htm?id=705201135652 | TI 数据手册指出“Input hysteresis allows slow input transition”[[3]](#ref-ti-aup1g00) |

## 已排除的单路与非门

- SN74LVC1G00（含 DBVR/DCKR 等封装）：数据手册特性未包含施密特触发，仅为标准 CMOS 输入[[4]](#ref-ti-lvc1g00)。
- SN74AHCT1G00（DBVR 等封装）：特性仅强调 TTL 兼容输入，无施密特描述[[5]](#ref-ti-ahct1g00)。
- SN74HCT1G00GV、SN74LVC1G38 等：分别为非施密特 TTL NAND、开漏 NAND，官方资料均未给出施密特输入特性[[5]](#ref-ti-ahct1g00)[[6]](#ref-ti-lvc1g38)。

---

<a id="ref-ti-ahc1g00"></a>[1] Texas Instruments, *SN74AHC1G00 Single 2-Input Positive-NAND Gate*, 特性条目列出 “Schmitt trigger action at all inputs”. https://www.ti.com/document-viewer/SN74AHC1G00/datasheet  
<a id="ref-nxp-74aup1g00"></a>[2] Nexperia, *74AUP1G00 Low-power 2-input NAND gate*, “Schmitt-trigger action at all inputs makes the circuit tolerant of slower input rise and fall times.” https://assets.nexperia.com/documents/data-sheet/74AUP1G00.pdf  
<a id="ref-ti-aup1g00"></a>[3] Texas Instruments, *SN74AUP1G00 Low-Power Single 2-Input Positive-NAND Gate*, “Input hysteresis allows slow input transition and better switching noise immunity.” https://www.ti.com/lit/ds/symlink/sn74aup1g00.pdf  
<a id="ref-ti-lvc1g00"></a>[4] Texas Instruments, *SN74LVC1G00 Single 2-Input Positive-NAND Gate*, 特性仅列出 CMOS 常规参数，无施密特触发。https://www.ti.com/document-viewer/SN74LVC1G00/datasheet  
<a id="ref-ti-ahct1g00"></a>[5] Texas Instruments, *SN74AHCT1G00 Single 2-Input Positive-NAND Gate*, 特性仅说明 TTL 兼容输入。https://www.ti.com/document-viewer/SN74AHCT1G00/datasheet  
<a id="ref-ti-lvc1g38"></a>[6] Texas Instruments, *SN74LVC1G38 Single 2-Input NAND Gate With Open-Drain Output*, 特性列表中无施密特触发项。https://www.ti.com/lit/ds/symlink/sn74lvc1g38.pdf
