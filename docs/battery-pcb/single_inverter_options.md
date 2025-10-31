# 单路施密特触发反相器选型

## 需求概述

- 硬件保护链要求将 `TEMP_ALERT_N`、`WD_RST_N` 等 3.3 V 信号组合得到“异常=高”的推挽输出，并在故障时驱动 `BQ76920 ALERT` 与停机链路，逻辑级需具备施密特迟滞以可靠处理缓慢边沿与开漏释放[^hp-chain]。
- 本记录补充单路反相器的选型，覆盖与 `TEMP_ALERT_N` 等低有效信号相关的极性转换，并保持 3.3 V 供电兼容、推挽输出与低静态功耗。

## 候选器件概览

| 型号 | 封装 | 输入类型 | 价格 (RMB/片) | 购买链接 |
| --- | --- | --- | --- | --- |
| UMW SN74LVC1G14DBVR | SOT-23-5 | 施密特触发 | [¥0.14](https://item.taobao.com/item.htm?id=672674224651) | [链接](https://item.taobao.com/item.htm?id=672674224651) |
| Nexperia 74LVC1G14GW,125 | SOT-353 | 施密特触发 | [¥0.26](https://item.taobao.com/item.htm?id=695968450355) | [链接](https://item.taobao.com/item.htm?id=695968450355) |
| TI SN74LVC1G04DCKR | SC-70-5 | 标准 CMOS | [¥0.13](https://item.taobao.com/item.htm?id=731779366159) | [链接](https://item.taobao.com/item.htm?id=731779366159) |
| Nexperia 74LVC1G04GW,125 | SOT-353 | 标准 CMOS | [¥0.17](https://item.taobao.com/item.htm?id=695601151155) | [链接](https://item.taobao.com/item.htm?id=695601151155) |

## 器件关键参数

| 指标 | SN74LVC1G14（施密特） | SN74LVC1G04（标准） |
| --- | --- | --- |
| 工作电压 | 1.65 V – 5.5 V[^ti-lvc1g14] | 1.65 V – 5.5 V |
| 输入类型 | 施密特触发，允许慢边沿 | CMOS，无迟滞 |
| 输出类型 | 推挽，I<sub>OH</sub>/I<sub>OL</sub> ≈ ±32 mA | 推挽，I<sub>OH</sub>/I<sub>OL</sub> ≈ ±32 mA |
| 静态电流 | ≤10 µA | ≤10 µA |
| 封装选项 | DBV (SOT-23-5)、DCK (SC-70-5)、GW (SOT-353)、DRL (SOT-553) | 相同封装系列 |

## 适配性评估

- **信号完整性**：施密特输入可直接读取 `TEMP_ALERT_N` 的上拉/释放过程，避免传统 CMOS 反相器对缓慢边沿产生多次翻转问题，满足硬件保护链对噪声裕度的要求。
- **驱动能力**：±32 mA 输出足以拉动 `ALERT` 总线和 `PSTOP_CTL` 分支，且对 500 kΩ~1 MΩ 下拉具备充裕余量，与文档中对 NAND 的驱动要求一致。
- **供电与极性**：与现有 3.3 V 轨兼容，可直接替换硬件链路中需要“低有效 → 高有效”转换的级联节点，无需额外电平转换。
- **封装兼容**：DBV 封装与当前板子上使用的 LVC/LVC2G 系列引脚排列相近，改板成本低；若后续要在密度更高区域放置，可考虑 GW/DRL 微型封装，但需重新规划焊盘和工艺能力。
- **供应风控**：部分链接标注品牌为 UMW，采购时应确认是否为 TI/Nexperia 原厂封装或同规格国产替代；如需确保原厂，可转向明确标注 TI/Nexperia 的同系列链接。

## 建议与后续动作

- 优先下单 **SN74LVC1G14DBVR**（SOT-23-5），与现有焊盘兼容，单价与 NAND 备件相当，可快速验证硬件链路。
- 若需要缩小封装或分摊功耗，可同步采购 74LVC1G14GW（SOT-353）样品，在后续 PCB 修订中评估焊接与测试可行性。
- 收货后建议抽检丝印与功能，确认施密特迟滞与输出驱动符合数据手册，再纳入正式 BOM。

---

[^hp-chain]: `docs/battery-pcb/hardware_protection_tps3823_tmp75.md` 中对硬件故障链路的电平、极性以及施密特迟滞需求描述。
[^ti-lvc1g14]: Texas Instruments, *SN74LVC1G14 Single Schmitt-Trigger Inverter* 产品页面。
