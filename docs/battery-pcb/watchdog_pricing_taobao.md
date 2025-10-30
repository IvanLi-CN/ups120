# Watchdog Component Pricing (Taobao)

数据更新：2025-10-30。内容基于淘宝首屏检索结果，聚焦智能电池 MCU 使用的连续信号看门狗芯片。

数据表引用：TPS3823/TPS3824 系列提供低有效持续 RESET 输出，[1]；STWD100 提供低有效持续 WDO 输出，可选推挽或开漏。[2]

## 型号总表

| 型号 | 单价 (¥) | 运费 (¥) | 输出电平/状态 | 驱动方式 | 备注 |
| --- | --- | --- | --- | --- | --- |
| TPS3823-33DBVR | 0.69 | 1.50 | 超时或欠压保持 RESET=L | 推挽 | 3.3 V 轨复位监控，含 MR |
| TPS3824-33DBVR | 3.63 | 0.00 | RESET(L) 持续，另带 RESET(H) | 双向推挽 | 同封装正反向复位输出 |
| STWD100NYWY3F | 1.32 | 1.50 | WDO 低电平保持直至喂狗 | 推挽/开漏可选 | 独立看门狗，EN 控制待机 |

## TPS3823-33DBVR（TI）

### 器件简介

- 适配 3.3 V 轨的电源监控，VDD 低于阈值或上电复位计时结束前保持 RESET 为低。[1]
- 默认 1.6 s 看门狗周期，可通过 WDI 翻转或 MR 管脚重新触发，防止误复位。[1]
- RESET 为推挽低有效输出，如需高电平报警需外接反相逻辑或使用后级晶体管。[1]

| 店铺 | 购买链接 | 单价 (¥) | 运费 (¥) |
| --- | --- | --- | --- |
| 深圳优信电子 | <https://item.taobao.com/item.htm?id=675187366447> | 0.69 | 1.50 |
| 深圳市远大芯程科技 | <https://item.taobao.com/item.htm?id=733699852343> | 0.40 | 1.00 |
| 深圳市恒芯科创电子 | <https://item.taobao.com/item.htm?id=707168250712> | 0.47 | 1.00 |
| 深圳市垚鑫电子科技 | <https://item.taobao.com/item.htm?id=608674230903> | 0.38 | 0.50 |
| 深圳市铭科芯电子 | <https://item.taobao.com/item.htm?id=733906869349> | 0.94 | 0.00 |
| 港柏芯城 | <https://item.taobao.com/item.htm?id=735663142560> | 0.38 | 1.00 |

## TPS3824-33DBVR（TI）

### 器件简介

- 提供同步的低有效 RESET 与高有效 RESET 输出，便于同时驱动不同极性的复位端。[1]
- 默认 1.6 s 看门狗定时，WDI 不翻转将保持低有效 RESET，直到重新喂狗或完成延迟。[1]
- 与 TPS3823 共用 5-pin SOT-23 封装，可直接替换共享 PCB 封装焊盘。[1]

| 店铺 | 购买链接 | 单价 (¥) | 运费 (¥) |
| --- | --- | --- | --- |
| 深圳优信电子 | <https://item.taobao.com/item.htm?id=748953673192> | 3.63 | 0.00 |
| 成港电子 | <https://item.taobao.com/item.htm?id=609308284027> | 0.95 | 2.00 |
| 深圳市诺维利电子企业店 | <https://item.taobao.com/item.htm?id=967450507950> | 1.90 | 0.00 |
| 全新集成电子商城 | <https://item.taobao.com/item.htm?id=710776572841> | 1.00 | 0.00 |
| 科利讯电子 | <https://item.taobao.com/item.htm?id=799626907282> | 0.45 | 3.00 |
| 汕头和祥电子 | <https://item.taobao.com/item.htm?id=666267401429> | 1.50 | 6.00 |

## STWD100NYWY3F（ST）

### 器件简介

- 独立看门狗电路，WDI 超时后 WDO 持续拉低直至重新喂狗或通过 EN 重新启动。[2]
- WDO 可选推挽或开漏，支持 3.4 ms 至 1.6 s 多档超时窗口，适合不同主控心跳频率。[2]
- EN 引脚默认下拉启用，方便系统待机时通过 MCU 高电平关断以降低静态功耗。[2]

| 店铺 | 购买链接 | 单价 (¥) | 运费 (¥) |
| --- | --- | --- | --- |
| 深圳市捷洲电子科技 | <https://item.taobao.com/item.htm?id=798146856741> | 0.94 | 0.00 |
| ICGO | <https://item.taobao.com/item.htm?id=753329757565> | 1.50 | 3.00 |
| 优质电子 | <https://item.taobao.com/item.htm?id=972126445746> | 1.30 | 0.00 |
| 深圳市东盛芯电子 | <https://item.taobao.com/item.htm?id=986028979106> | 0.90 | 8.00 |
| 凌捷达电子 | <https://item.taobao.com/item.htm?id=908838461707> | 1.08 | 3.00 |

---

[1]: https://www.ti.com/lit/ds/symlink/tps3823.pdf
[2]: https://www.st.com/resource/en/datasheet/stwd100.pdf
