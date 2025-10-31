# NAND 与非门器件评估

本文对符合《hardware_protection_tps3823_tmp75.md》硬件保护链需求的单路与双路 NAND（与非）器件进行整理与评估，仅保留关键电气参数与封装信息。

## 1. 器件摘要

| 型号 / 链接 | 通道 | 输入属性 | V<sub>CC</sub> 范围 | 输出能力 | 封装 | 优信电子单价 | 适配要点 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| [SN74LVC1G00DBVR](https://www.ti.com/product/SN74LVC1G00)[^ti-lvc1g00-ds] | 单路 2 输入 | LVC CMOS，**非施密特** | 1.65–5.5 V[^ti-lvc1g00-ds] | 推挽，±24 mA @ 3.3 V[^ti-lvc1g00-ds] | SOT-23-5 | [¥ 1.76 / 件][^youxin-lvc1g00] | 放置在已整形节点执行末级反相或逻辑合成。 |
| [SN74LVC2G132DCUR](https://www.ti.com/product/SN74LVC2G132)[^ti-lvc2g132-ds] | 双路 2 输入 | LVC CMOS，全输入施密特 | 1.65–5.5 V[^ti-lvc2g132-ds] | 推挽，±24 mA @ 3.3 V[^ti-lvc2g132-ds] | VSSOP-8 | [¥ 0.91 / 件][^youxin-lvc2g132] | 提供两个施密特 NAND，可同时生成 `FAULT_LATCH`、`STOP_REQUEST`。 |

## 2. 与硬件保护链的适配建议
- **前级故障合成**：`SN74LVC2G132` 提供两个施密特 NAND，可按《hardware_protection_tps3823_tmp75.md》第 3 章的方案，将 `WD_RST_N`、`TEMP_ALERT_N` 和 `PSTOP_MCU` 分别整合成 `FAULT_LATCH` 与 `STOP_REQUEST`。施密特迟滞有效吸收 TMP75 开漏信号的慢上升沿。[^ti-lvc2g132-ds]
- **末级翻转/缓冲**：若仍需单路 NAND（例如把 `STOP_REQUEST` 反相得到 `PSTOP_CTL`），`SN74LVC1G00` 在 3.3 V 下的驱动能力足以直接控制报警线与 MOS 门极。不过其输入为标准 CMOS 阈值，建议仅放置在已被前级施密特整形后的节点；若需直接面对噪声或慢边沿信号，可考虑同系列的 `SN74AHC1G00` / `SN74AUP1G00` 以获得内建施密特迟滞。[^hardware-doc]
- **版图兼容性**：两款器件的 SOT-23-5 / VSSOP-8 封装均与目前 PCB 工艺兼容，可在 `FAULT_LATCH` 与 `PSTOP_CTL` 区域直接替换现有 LVC/LVC2G 逻辑。不需要引入根目录 Cargo 工具链变更。

## 3. 后续动作
- 订购 `SN74LVC2G132DCUR` 样品，用于验证施密特输入在硬件链路中的噪声裕量。
- 若 `SN74LVC1G00` 在慢边沿测试中出现抖动，准备切换至具施密特输入的同系器件，并同步更新选型文档。

---

[^ti-lvc1g00-ds]: Texas Instruments, *SN74LVC1G00 Single 2-Input Positive-NAND Gate*, SCES212AB, Apr. 2014, pp. 1–2.
[^ti-lvc2g132-ds]: Texas Instruments, *SN74LVC2G132 Dual 2-Input NAND Gate With Schmitt-Trigger Inputs*, SCES547D, Dec. 2013, pp. 1–2.
[^hardware-doc]: 《hardware_protection_tps3823_tmp75.md》，第 3 章“逻辑实现”，强调使用施密特 NAND 抑制告警线慢边沿。
[^youxin-lvc1g00]: 深圳优信电子淘宝店，《原装正品 SN74LVC1G00DBVR SOT-23-5 单路2输入正与非门 逻辑芯片》，¥1.76/件，访问于 2025-05-12，https://item.taobao.com/item.htm?id=672658816379。
[^youxin-lvc2g132]: 深圳优信电子淘宝店，《原装正品 SN74LVC2G132DCUR VSSOP-8 双路2输入与非门芯片》，¥0.91/件，访问于 2025-05-12，https://item.taobao.com/item.htm?id=714861431492。
