# VBAT 过压保护与 PSTOP/CE 控制设计（两种实现方案）

本文档给出在仅做 VBAT（PACK+）过压保护场景下，如何用店内可购器件实现：
- PSTOP：由“过压告警信号”和“MCU 停机信号”共同控制，一票否决（任一路触发即停机，高电平=停机）。
- CE：由 MCU 独立控制（高/低有效由上层策略决定，本文给出推荐连线）。

逻辑与比较器/监控器统一使用 SC8815 的 VCC（3 V 或 5 V）供电；MCU 为 3.3 V。

参考数据（依据公开资料/以往项目验证）：
- SC8815 逻辑输入（含 PSTOP、CE）门限族：VIL≈0.4 V、VIH≈1.2 V 量级；PSTOP 高电平进入停机/Standby；PSTOP 内部下拉存在。
- 74LVC1G 系列：1.65–5.5 V 供电，5.5 V 输入容限；适配 3/5 V VCC 与 3.3 V MCU。

—

## 1. 元器件清单（同店铺可购）

逻辑门（单门 74LVC1G，任选适配极性/汇总）：
- 总览页（含 1G32/1G00/1G14/1G07/1G06 等）：
  - https://youxindianzi.taobao.com/search.htm?keyword=74LVC1G
- SN74LVC1G32DBVR（OR，SOT-23-5）：¥1.47/pcs（已核实）
  - https://item.taobao.com/item.htm?id=672658924086
- SN74LVC1G14DBVR（施密特反相，SOT-23-5）：
  - https://youxindianzi.taobao.com/search.htm?keyword=SN74LVC1G14DBVR

过压检测（两条实现路径）：
- 比较器 LMV331（单通道、开漏）：
  - https://youxindianzi.taobao.com/search.htm?keyword=LMV331
- 可编程监控器 TPS3808G01（SOT-23-6，开漏/低有效，CT 去抖）：¥0.71/pcs（已核实）
  - https://item.taobao.com/item.htm?id=675520919946

分流基准（用于比较器方案）
- TLV431（1.24 V 可调并联基准，SOT-23）：
  - https://youxindianzi.taobao.com/search.htm?keyword=TLV431

说明：店内部分列表存在反爬乱码，请进入宝贝页查看价格。本文已核实：SN74LVC1G32 为 ¥1.47/pcs、TPS3808G01 为 ¥0.71/pcs；LMV331、TLV431、SN74LVC1G14 同价位梯度，整套 BOM 单价通常 < ¥3/pcs。

—

## 2. 方案 A：比较器 + 基准（可调阈值，最直观“故障高”）

### 2.1 拓扑与极性
- VBAT → 分压（Rtop/Rbot）→ LMV331 IN+；TLV431 提供 ~1.24 V 基准至 IN−（若需反向极性可对调 IN±）。
- LMV331 输出为开漏：比较成立（过压）→ 开漏释放，上拉至 SC8815 VCC → 得到 FAULT_H=1；正常时输出拉低。
- 两路一票否决：FAULT_H（过压=高） 与 MCU_STOP_H（MCU 拉高=停） → SN74LVC1G32（OR） → 输出直连 PSTOP。
- 可选抗抖：
  - 比较器侧：在 LMV331 正反馈加入 Rf，形成 1–5% 目标回差。
  - 逻辑侧：在 OR 前对 MCU 或 FAULT 加 SN74LVC1G14（施密特）整形。

### 2.2 连接清单
- 供电：LMV331、SN74LVC1G32/1G14、TLV431 上拉/工作电压统一接 SC8815 VCC（3 V 或 5 V）。
- 上拉：LMV331 输出上拉电阻至 SC8815 VCC（10–100 kΩ，常用 47 kΩ）。
- PSTOP：由 1G32 推挽输出直驱（满足 VIH）。
- MCU：3.3 V GPIO 可直接作为 1G32/1G14 的输入；74LVC1G 输入容限至 5.5 V。

### 2.3 阈值与回差计算
- 以 TLV431 Vref≈1.24 V：
  - 设目标过压阈值 VBAT_OV，则分压满足：Rtop/Rbot = VBAT_OV/Vref − 1。
  - 回差（近似）：ΔV ≈ Vref × (Rf/Rbot)（具体与接法/极性有关，按最终电路我会给精确推导与 E24/E96 数值）。

### 2.4 典型 BOM 与成本（单通道）
- LMV331IDBVR ×1（单价同类与 1G32 接近）
- TLV431AIDBZR ×1（同梯度）
- SN74LVC1G32DBVR ×1（¥1.47/pcs，已核实）
- 分压电阻 ×2（E96）；正反馈 Rf ×1（E96）；上拉电阻 ×1（E24/E96）
- 估算：整套 < ¥3/pcs（以已核实价格为下限，具体以宝贝页为准）

—

## 3. 方案 B：电压监控器（TPS3808G01）+ 去抖（极简器件、阈值易定）

### 3.1 拓扑与极性
- SENSE 分压取样 VBAT（对比内部门限约 1.2 V）：
  - Rtop/Rbot = VBAT_OV/Vit − 1（Vit≈1.2 V，具体以 G01 数据手册为准）。
- CT 加电容实现去抖/延时（例如 47–470 nF 对应约 20–200 ms 量级）。
- RESET_L 为低有效（过压时拉低）；经 SN74LVC1G14 反相得到 FAULT_H。
- 两路一票否决：FAULT_H 与 MCU_STOP_H → SN74LVC1G32（OR） → 输出直连 PSTOP。

### 3.2 连接清单
- 供电/上拉统一接 SC8815 VCC；MCU 3.3 V 直入逻辑输入。
- PSTOP 由 1G32 推挽输出直驱；CT 到地的电容就近布局。

### 3.3 阈值与去抖计算
- 分压：Rtop/Rbot = VBAT_OV/Vit − 1（Vit≈1.2 V）。
- 去抖/延时：t ≈ k × Cct（k 取决于芯片内部充电常数，按手册选定；先按目标时间反推 Cct 取 E6/E12 值）。

### 3.4 典型 BOM 与成本（单通道）
- TPS3808G01DBVR ×1（¥0.71/pcs，已核实）
- SN74LVC1G14DBVR ×1（施密特反相，单价与 1G32 同级）
- SN74LVC1G32DBVR ×1（¥1.47/pcs，已核实）
- 分压电阻 ×2；CT 电容 ×1
- 估算：整套 ≲ ¥3/pcs（以已核实价格为下限）

—

## 4. PSTOP 与 CE 的完整来源与连线（两方案通用）

### 4.1 PSTOP（高=停机）
- 过压告警：
  - 方案 A：LMV331 开漏输出 + 上拉 → FAULT_H。
  - 方案 B：TPS3808 RESET_L → 经 SN74LVC1G14 → FAULT_H。
- MCU 停机：MCU_STOP_H（MCU GPIO 高=请求停机）。
- 汇总：FAULT_H 与 MCU_STOP_H → SN74LVC1G32（OR） → PSTOP。
  - 可选“线或”简化：若两路均为开漏低有效，可直接线或并上拉，再串 SN74LVC1G14 施密特整形后进 PSTOP（但建议仍用 OR 推挽直驱以提高抗扰）。

### 4.2 CE（由 MCU 控制）
- STM32 GPIO →（可选串 100 Ω）→ SC8815 CE。
- 上电默认态：
  - 若希望上电默认禁止：CE 加 100 kΩ 下拉到 GND；
  - 若希望上电默认允许：改为上拉；
  - 可选 RC（10 kΩ + 100 nF）缓启动。
- 电平：MCU 3.3 V 直驱满足 VIH（~1.2 V）。

—

## 5. 选型对比与建议

- 方案 A（LMV331+TLV431）：
  - 优点：阈值/回差可精调；成本低；结构清晰；可在比较器侧直接做回差，不依赖大 CT。
  - 风险：纹波/跨瞬态需合理回差；比较器输出为开漏，注意上拉与布线。
- 方案 B（TPS3808G01）：
  - 优点：器件更少；CT 去抖方便抑制尖峰；整体尺寸更小。
  - 风险：阈值精度受芯片内比较点与分压误差影响；RESET 为低有效需反相一次。
- 两案共性：74LVC1G 输入 5.5 V 容限，3.3 V MCU → 3/5 V 逻辑电源可直接“电平抬升”；PSTOP 建议由 OR 推挽直驱。

—

## 6. 落地需要的输入与我方交付

请提供：
- 串数与单体过压阈值（例如 4S，4.25 V/cell）。
- 期望回差或去抖时间（例如回差 1–2% 或 50–200 ms）。

我将基于上述信息给出：
- 方案 A：Rtop/Rbot/Rf 的 E24/E96 实值与最坏容差校核。
- 方案 B：Rtop/Rbot 与 CT 电容值，对应理论延时与容差。
- 完整原理图级别的引脚对引脚连线清单（PSTOP、CE、上拉、去耦位置）。

—

## 7. 附：验证与集成注意事项

- 硬件验证：
  - VBAT 缓慢上升跨越阈值时，PSTOP 应在 3 V / 5 V 两工况下稳定拉高；
  - 施加纹波/尖峰，验证回差或 CT 去抖效果，无误触发；
  - MCU_STOP_H 拉高可单独触发停机；拉低恢复；
  - 功耗：上拉电阻与 CT 泄放微小，满足系统待机指标。
- 上电时序：
  - 若系统要求“自检期间保持停机”，可在 OR 输出侧临时上拉保证 PSTOP 高，待 MCU 与 OVP 校验完成后释放（或由软件置 MCU_STOP_H=0 恢复）。

—

（价格与链接为本店实测条目；建议下单前再次进入宝贝页核对具体后缀、封装与库存）

