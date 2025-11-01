# 单路施密特触发与非门器件总结

旨在为 `hardware_protection_tps3823_tmp75.md` 中的硬件保护链挑选合适的单路施密特触发 NAND，使 `WD_RST_N`、`TEMP_ALERT_N` 等信号能够直接整合为“异常=高”的组合逻辑，同时驱动 `BQ76920 ALERT` 与 `PSTOP_CTL`。

## 1. 关键参数对比

| 型号 / 链接 | 单价（CNY） | 逻辑族 | 输入属性 | V<sub>CC</sub> 范围 | 输出级 | I<sub>CC</sub> (max) | t<sub>pd</sub> @3.3 V (典型/最大) | 封装 | 备注 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| [SN74AHC1G00DBVR](https://item.taobao.com/item.htm?id=673364782576) / [DCKR](https://item.taobao.com/item.htm?id=704713346062) | ¥1.95（DBVR）、¥0.25（DCKR） | AHC, CMOS | 全输入施密特触发；阈值≈0.3/0.7·V<sub>CC</sub> | 2.0–5.5 V，输入不得超过 V<sub>CC</sub> | 推挽，±8 mA @5 V，≈±4 mA @3.3 V | 10 µA | 4.0 ns / 7.1 ns[^ti-ahc1g00] | SOT‑23‑5、SC‑70‑5 | 满足 3.3 V 逻辑、驱动充足；功耗较高 |
| [74AUP1G00GW,125](https://item.taobao.com/item.htm?id=696330677234) | ¥0.49 | AUP, CMOS | 全输入施密特触发；支持缓慢边沿 | 0.8–3.6 V；输入容许 3.6 V | 推挽，±4 mA @3.3 V | 0.9 µA | 3.2 ns / 4.8 ns[^nxp-74aup1g00] | SOT‑353 (TSSOP5) | 低功耗、阈值与电源兼容；输出电流满足 ALERT/PSTOP 控制 |
| [74AUP1G00GX,125](https://item.taobao.com/item.htm?id=696835563731) | ¥0.49 | AUP, CMOS | 同上 | 0.8–3.6 V；输入容许 3.6 V | 推挽，±4 mA @3.3 V | 0.9 µA | 3.2 ns / 4.8 ns[^nxp-74aup1g00] | X2SON‑5 (SOT886) | 与 GW 版电气一致，体积更小；焊接要求更高 |
| [SN74AUP1G00DBVR](https://item.taobao.com/item.htm?id=696330677234) | ¥0.49（同上链接） | AUP, CMOS | 输入带 250 mV 典型迟滞 | 0.8–3.6 V；输入容许 3.6 V | 推挽，±4 mA @3.3 V | 1 µA | 3.3 ns / 4.9 ns[^ti-aup1g00] | SOT‑23‑5 | 与现有 SOT‑23 足迹兼容，低功耗 |
| [SN74AUP1G00DCKR](https://item.taobao.com/item.htm?id=705201135652) | ¥0.96 | AUP, CMOS | 输入带 250 mV 典型迟滞 | 0.8–3.6 V；输入容许 3.6 V | 推挽，±4 mA @3.3 V | 1 µA | 3.3 ns / 4.9 ns[^ti-aup1g00] | SC‑70‑5 | 与 AUP 系列兼容；SC‑70 封装节省面积 |
| SN74LVC1G38DBVR | —（店内未上架） | LVC, CMOS，开漏输出 | **开漏 NAND**，非施密特输入 | 1.65–5.5 V；输入容许 5.5 V | 开漏，需外部上拉 | 10 µA | 4.5 ns / 7.5 ns[^ti-lvc1g38] | SOT‑23‑5、SC‑70‑5 | **不推荐**：输出为开漏且输入无施密特迟滞 |

> 价格信息更新于 2025-10-31，来源：[深圳优信电子](https://youxindianzi.taobao.com/)（chrome-devtools 抓取）。

> 说明：SN74AHCT1G00、SN74LVC1G00 等型号虽为单路 NAND，但仅提供 TTL/标准 CMOS 阈值，不含施密特触发，故未列入。

## 2. 项目适用性分析

1. **供电与阈值**  
   智能电池板上相关信号（`WD_RST_N`、`TEMP_ALERT_N`、`PSTOP_MCU` 等）均在 3.3 V 轨工作，并可能出现缓慢边沿。AUP、AHC 两个族在 3.3 V 下提供 ≥250 mV 的输入迟滞，能有效抑制慢速或带噪声的告警线抖动。

2. **驱动能力**  
   - `FAULT_LATCH`、`PSTOP_CTL` 需要推挽输出以直接驱动下游 MOS 管与 BQ76920 的 ALERT 强制输入。AUP/ AHC 器件提供 ±4 mA（AUP）至 ±8 mA（AHC）驱动，在 3.3 V 下足够克服 BQ76920 推荐的 500 kΩ~1 MΩ 下拉。  
   - 开漏版本（SN74LVC1G38）虽可外接上拉，但在 ALERT BUS 需要快速释放时会引入额外延迟，因此淘汰。

3. **功耗权衡**  
   看门狗与温度告警常驻高电平，器件静态功耗会长期体现。AUP1G00（TI 或 Nexperia）I<sub>CC</sub> ≤1 µA，适合电池应用；AHC1G00 静态 10 µA，可接受但不如 AUP。

4. **封装与焊接**  
   - 若 PCB 采用 SOT‑23 足迹，可考虑 AHC1G00DBVR、AUP1G00DBVR/DCKR 等封装。  
   - 若需节省空间，可选 SOT‑353 或 X2SON，但需重新评估装配能力。

5. **逻辑族差异（CMOS vs. TTL）**  
   所列器件全部属于 CMOS 工艺（AHC、AUP 系列），内部使用施密特触发，阈值自适应 V<sub>CC</sub>；AHCT 系列虽为 TTL 兼容输入，但数据手册未赋予施密特性质，因此不满足需求。

## 3. 推荐方案

- **首选：SN74AUP1G00DCKR（TI）或 74AUP1G00GW,125（Nexperia）**  
  低功耗、全输入施密特触发、推挽输出且与 3.3 V 轨兼容；封装与现有 LVC/LVC2G 封装接近，替换改板成本最低。

- **备选：SN74AHC1G00DBVR**  
  当更高输出驱动或 5 V 逻辑兼容性成为必须时可选，但需注意静态功耗增加且输入端不得超过供电电压。

上述结论与 `hardware_protection_tps3823_tmp75.md` 中的信号拓扑相一致，选型时按表中参数选择合适器件即可保持 ALERT/PSTOP 的极性与时序设计目标。

---

[^ti-ahc1g00]: Texas Instruments, *SN74AHC1G00 Single 2-Input Positive-NAND Gate*, SCLS313Q, 2024-01. Features 列表注明 “Schmitt trigger action at all inputs”；表 5-6 提供 t<sub>pd</sub> 及输出电流规格。  
[^nxp-74aup1g00]: Nexperia, *74AUP1G00 Low-power 2-input NAND gate*, Rev.10, 2024-09-19. Section 1 描述施密特触发输入，表 7/8 给出 V<sub>CC</sub> 范围与时序。  
[^ti-aup1g00]: Texas Instruments, *SN74AUP1G00 Low-Power Single 2-Input Positive-NAND Gate*, SCES604J, 2016-12. Features 列出 “Input hysteresis allows slow input transition”，表 7-6 给出时序/驱动能力。  
[^ti-lvc1g38]: Texas Instruments, *SN74LVC1G38 Single 2-Input NAND Gate with Open-Drain Output*, SCES538G, 2020-02. 数据手册说明输出为开漏，输入不具备施密特迟滞。
