# TPS3823 + TMP75 硬件故障保护链

## 1. 目标与约束复核

- MCU 卡死（看门狗无心跳）或温度越限时，硬件链路独立触发停机，无需 MCU 参与。  
- 将异常状态第一时间同步至 `BQ76920 ALERT` 总线；根据数据手册 8.3.1.3.4，该引脚是**有源高电平**数字中断，默认靠外部下拉保持低电平[3]。本文逻辑会在硬件故障时将 ALERT 拉高，触发 AFE 的 OVRD_ALERT 保护。  
- 将 功率控制器 的功率级控制脚 (`PSTOP`) 拉高，强制停止充电。  
- 复位完成或温度回到窗口后，仅当 MCU 主动释放停机请求且硬件故障解除时才允许恢复。  
- 逻辑器件、电平与大小电流路径符合各器件数据手册规定：  
  - TPS3823-33DBVR：`RESET` 输出推挽、典型保持低 200 ms（数据手册 6.3、7.5）[1]。  
  - TMP75AIDGKR：`ALERT` 开漏输出，越限时拉低，需要外部上拉（表 7-1）[2]。  
  - BQ76920：`ALERT`/`FAULT` 引脚为有源高电平中断，推荐外接 500 kΩ~1 MΩ 下拉至 VSS 确保静态为低（8.3.1.3.4 节）[3]。  
- PWR_CTRL：`PSTOP` 为高=停机，`PSTOP_CTL` MCU 端经 N-MOS 反相驱动（JP datasheet 8.9 + 现有板级实现）[4]。  

## 2. 器件接口可行性分析

### 2.1 TPS3823-33DBVR

- `RESET` 为推挽低有效，上电延时结束或看门狗正常喂狗时输出高。  
- 输出高电平最小值 `VOH ≥ 0.8·VDD`，在 3.3 V 轨上即 ≥2.64 V，可直接驱动 LVC 系列逻辑门输入。  
- 灌电流能力 ±5 mA（表 6-1），与下游 CMOS 输入匹配，无需额外缓冲。

### 2.2 TMP75AIDGKR

- `ALERT` 为开漏低有效，可与 I²C 总线共用 3.3 V 上拉（推荐 4.7–10 kΩ）。  
- Fault Queue、上下阈值可通过寄存器配置，越限后输出拉低，MCU 清除或温度回窗后释放。  
- 为实现“异常=高”的组合逻辑，只需通过 NAND 或 NOR 级把 `ALERT` 取反即可。

### 2.3 BQ76920 ALERT

- **输出行为（表 7.5）**：`VALERT_OH ≥ 0.75 × REGOUT` @ `I = –1 mA`，`VALERT_OL ≤ 0.25 × REGOUT`；说明 ALERT 内部具备推挽驱动能力，可以直接拉高至 3.3 V 轨，也能可靠下拉至地。  
- **弱下拉（表 7.5 中 `RALERT_PD`）**：当 ALERT 被驱动高时，芯片内部仍保留 0.8–8 MΩ 的泄放电阻到地。因此 TI 推荐额外加 500 kΩ–1 MΩ 外部下拉，以便在无事件时保持低电平并抑制噪声。  
- **外部覆盖（8.3.1.3.4）**：当 ALERT 处于低电平且被外部强制拉高到 ≥1 V（`VALERT_IH`），芯片会设置 `OVRD_ALERT`，并自动关闭 CHG/DSG。该机制专为外部硬件保护链路准备。  
- **设计结论**：我们在非异常状态下保持高阻，仅依赖外部分压把 ALERT 固定低；出现硬件故障时，通过三态缓冲器拉高到 REGOUT ≈3.3 V，既满足 OVRD_ALERT 覆盖条件，又不会与芯片自身输出形成直连冲突。

### 2.4 PSTOP

- PWR_CTRL 数据手册定义 `PSTOP=High` 停止功率级，`PSTOP=Low` 允许动作。  
- 现有板级通过 N 沟道 MOSFET 反相驱动：`PSTOP_CTL=High → PSTOP=Low → 允许；PSTOP_CTL=Low → PSTOP=High → 停机`（参考 `firmware/smart-battery/SOFTWARE_DESIGN.md:23` 等文件）。  
- 因此下游逻辑必须把“异常=高”转换成 `PSTOP_CTL=Low`，本文所列 NAND 组合方案可满足。

> 以上分析确保每个器件的输入输出电平、极性与驱动能力互相兼容，为后续逻辑网络选型提供依据。

## 3. 信号属性

| 信号来源 | 逻辑网名（单一命名） | 物理特性 | 正常态 | 故障态 | 说明 |
| --- | --- | --- | --- | --- | --- |
| 看门狗 `RESET_N` | `WD_RST_N` | 推挽输出，3.3 V 轨 | `1` | `0` | MCU 心跳正常时保持高；喂狗超时、VDD 掉电或上电延时内拉低 |
| 温度告警开漏 | `TEMP_ALERT_N` | 开漏输出，上拉至 3.3 V | `1` | `0` | 超过窗口立即拉低，直至 MCU 清除或温度回窗 |
| MCU GPIO | `PSTOP_MCU` | 推挽输出，3.3 V | `1` | `0` | 1=允许功率级，0=MCU 主动停机 |
| 逻辑合成 | `FAULT_LATCH` | 推挽输出，3.3 V | `0` | `1` | NAND 输出，高电平等价于硬件故障存在 |
| 逻辑合成 | `PSTOP_CTL` | 推挽输出，3.3 V | `1` | `0` | 高电平=功率级允许、低电平=停机；具体逻辑实现见第 3 章 |
| 输出 | `PSTOP` | 由板载 MOS 反相获得 | `0` | `1` | 高=停机，低=允许；`PSTOP = ~PSTOP_CTL` |
| 输出 | `ALERT_BUS` | 三态驱动，默认下拉保持低 | `0` | `1` | 故障时由三态缓冲器拉高告警总线 |

`PSTOP_MCU` 默认配置为 3.3 V 推挽输出，高电平表示“允许功率级、无急停请求”，低电平表示“MCU 主动要求停机”；该信号现阶段通过板上 N 沟道 MOS 管间接控制 PWR_CTRL 的 `PSTOP` 电平。

## 3. 逻辑实现

### 3.1 公共前级

- **故障锁存**：`FAULT_LATCH = ~(WD_RST_N · TEMP_ALERT_N)`。只要看门狗复位或温度告警拉低，其输出立即变为高电平，用于触发告警和停机链路。  
- **告警输出**：`FAULT_LATCH` 驱动三态缓冲器的使能端。故障时缓冲器把 `ALERT_BUS` 拉高；无故障时保持高阻，外部下拉 `R_ALERT` 把总线维持在低电平。  
- **功率级接口**：`PSTOP_CTL` 送入既有的板载 MOS 反相网络（文中以 `BOARD_INV` 代指），该网络输出 `PSTOP = \lnot PSTOP_CTL` 并驱动功率控制器的停机脚。设计目标是：`PSTOP_CTL=1` 表示“允许”，`PSTOP_CTL=0` 表示“停机”。

下文给出三种等效实现方案，均满足“任一故障或 MCU 请求即停机”以及“全部正常时允许功率级”的目标，而且不需要新增除所述门电路之外的逻辑芯片。

### 3.2 方案 A：双 NAND + 反相器

思路是使用第一颗 NAND 生成 `FAULT_LATCH`，随后用单级反相器得到 `SAFE_OK = \lnot FAULT_LATCH = WD_RST_N · TEMP_ALERT_N`，最后再用第二颗 NAND 组合 `SAFE_OK` 与 `PSTOP_MCU`，输出 `PSTOP_CTL = SAFE_OK · PSTOP_MCU`。

推荐选型：
- 施密特 NAND 使用 [`component-candidates/nand_gate_options_youxin.md`](component-candidates/nand_gate_options_youxin.md) 中的 SN74LVC2G132DCUR（双路施密特输入，推挽输出）。
- 反相器使用 [`component-candidates/single_inverter_options.md`](component-candidates/single_inverter_options.md) 中的 SN74LVC1G14DBVR（施密特输入，SOT-23-5）。

```
WD_RST_N ─┐
           ├─ NAND ──> FAULT_LATCH ──┬─> 三态缓冲 → ALERT_BUS
TEMP_ALERT_N ─┘                        │
                                       └─> 反相器 → SAFE_OK ─┐
PSTOP_MCU ──────────────────────────────┴─ NAND ──> PSTOP_CTL ─> BOARD_INV → PSTOP
```

| `PSTOP_MCU` | `WD_RST_N` | `TEMP_ALERT_N` | `FAULT_LATCH` | `SAFE_OK` | `PSTOP_CTL` | `PSTOP` | `ALERT_BUS` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | 0 | 0 | 1 | 0 | 0 | 1 (停机) | 1 |
| 0 | 0 | 1 | 1 | 0 | 0 | 1 (停机) | 1 |
| 0 | 1 | 0 | 1 | 0 | 0 | 1 (停机) | 1 |
| 0 | 1 | 1 | 0 | 1 | 0 | 1 (停机) | 0 |
| 1 | 0 | 0 | 1 | 0 | 0 | 1 (停机) | 1 |
| 1 | 0 | 1 | 1 | 0 | 0 | 1 (停机) | 1 |
| 1 | 1 | 0 | 1 | 0 | 0 | 1 (停机) | 1 |
| 1 | 1 | 1 | 0 | 1 | 1 | 0 (放行) | 0 |

`SAFE_OK` 仅在看门狗与温度输入均为高电平时才为 1，因此 `PSTOP_CTL` 只有在安全条件满足且 MCU 允许的情况下才会输出高电平。其余任一组合均会令 `PSTOP_CTL` 断言停机。

### 3.3 方案 B：NAND + 三输入与门

在维持第一颗 NAND 生成 `FAULT_LATCH` 的同时，直接使用三输入与门计算 `PSTOP_CTL = WD_RST_N · TEMP_ALERT_N · PSTOP_MCU`。这样省去了单独的反相器，逻辑结构更加直接。

推荐选型：
- 施密特 NAND 选用 [`component-candidates/schmitt_trigger_nand_options.md`](component-candidates/schmitt_trigger_nand_options.md) 中的 74AUP1G00（全输入施密特迟滞，低功耗）。
- 三输入与门选用 [`component-candidates/three_input_and_gate_options_youxin.md`](component-candidates/three_input_and_gate_options_youxin.md) 中的 SN74LVC1G11DCKR（单门 3 输入，SC-70-6）。

```
WD_RST_N ─┐                 ┌─────────> 三输入与门 ──> PSTOP_CTL ─> BOARD_INV → PSTOP
           ├─ NAND ──> FAULT_LATCH ─┤
TEMP_ALERT_N ─┘                     │
                                    └─> 三态缓冲 → ALERT_BUS
PSTOP_MCU ─────────────────────────────┘
```

| `PSTOP_MCU` | `WD_RST_N` | `TEMP_ALERT_N` | `FAULT_LATCH` | `PSTOP_CTL` | `PSTOP` |
| --- | --- | --- | --- | --- | --- |
| 0 | 0 | 0 | 1 | 0 | 1 (停机) |
| 0 | 0 | 1 | 1 | 0 | 1 (停机) |
| 0 | 1 | 0 | 1 | 0 | 1 (停机) |
| 0 | 1 | 1 | 0 | 0 | 1 (停机) |
| 1 | 0 | 0 | 1 | 0 | 1 (停机) |
| 1 | 0 | 1 | 1 | 0 | 1 (停机) |
| 1 | 1 | 0 | 1 | 0 | 1 (停机) |
| 1 | 1 | 1 | 0 | 1 | 0 (放行) |

由于 `PSTOP_CTL` 直接等于三路输入的与运算，所以只有全部安全且 MCU 允许时才为高电平，其余情况均会立即触发停机。
`ALERT_BUS` 依旧仅由 `FAULT_LATCH` 控制，高电平表示硬件强制告警，行为与方案 A 相同。

### 3.4 方案 C：NAND + 两输入或门 + 反相器

此方案把故障检测与 MCU 请求统一成“停机请求”再反相：先用 NAND 得到 `FAULT_LATCH`，再将 `FAULT_LATCH` 与 `MCU_STOP = \lnot PSTOP_MCU` 经由两输入或门级联求和（两级或门即可覆盖三路条件），得到 `STOP_REQUEST = FAULT_LATCH + MCU_STOP`，最后使用单路施密特反相器生成 `PSTOP_CTL = \lnot STOP_REQUEST`。

推荐选型：
- 施密特 NAND：SN74LVC2G132DCUR（[`component-candidates/nand_gate_options_youxin.md`](component-candidates/nand_gate_options_youxin.md)）。
- 两输入或门：74LVC1G32GV,125（[`component-candidates/or_gate_options_youxin.md`](component-candidates/or_gate_options_youxin.md)）。
- 反相器：SN74LVC1G14DBVR（[`component-candidates/single_inverter_options.md`](component-candidates/single_inverter_options.md)）。

```
WD_RST_N ─┐
           ├─ NAND ──> FAULT_LATCH ──┬─> 三态缓冲 → ALERT_BUS
TEMP_ALERT_N ─┘                        │
                                       ├─> 或门 → STOP_REQUEST ─┬─> 反相器 → PSTOP_CTL → BOARD_INV → PSTOP
PSTOP_MCU ──┐                          │                        │
             └─ 取反得到 MCU_STOP ────┘                        └─> （或门级联用于容纳第三个条件）
```

| `PSTOP_MCU` | `WD_RST_N` | `TEMP_ALERT_N` | `FAULT_LATCH` | `MCU_STOP` | `STOP_REQUEST` | `PSTOP_CTL` | `PSTOP` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 0 | 0 | 0 | 1 | 1 | 1 | 0 | 1 (停机) |
| 0 | 0 | 1 | 1 | 1 | 1 | 0 | 1 (停机) |
| 0 | 1 | 0 | 1 | 1 | 1 | 0 | 1 (停机) |
| 0 | 1 | 1 | 0 | 1 | 1 | 0 | 1 (停机) |
| 1 | 0 | 0 | 1 | 0 | 1 | 0 | 1 (停机) |
| 1 | 0 | 1 | 1 | 0 | 1 | 0 | 1 (停机) |
| 1 | 1 | 0 | 1 | 0 | 1 | 0 | 1 (停机) |
| 1 | 1 | 1 | 0 | 0 | 0 | 1 | 0 (放行) |

两级或门可以由同一器件的多个通道级联实现，最终的反相由单路施密特反相器完成，因此不会引入额外的复杂逻辑。`PSTOP_CTL` 只有在“无故障且 MCU 许可”这一组合下才为 1。
`ALERT_BUS` 的驱动方式与前两种方案一致，始终由 `FAULT_LATCH` 控制。

### 3.5 方案 D：TMP75 直驱与门（无看门狗）

当前智能电池网表（`docs/battery-pcb/netlist_battery.enet`）中已经采用了“只保留温度告警 + MCU 许可”的最简结构：

- `TMP75` 的开漏输出在网表中命名为 `TEMP_FAULT_N`（`U12` 引脚 3），通过 `R58`（10 kΩ）上拉到 3.3 V。常态=高，过温=低。  
- `U22`（SN74AUP1G08DCKR）直接计算 `PSTOP_CTL = TEMP_FAULT_N · PSTOP_MCU`；因此只要温度或 MCU 任一拉低，功率级立即停机。  
- `U19`（74LVC1G14GW,125）把 `TEMP_FAULT_N` 反相成 `TEMP_FAULT`，并交给 `U13`（74AUP1T34GW,125）缓冲驱动 `ALERT` 总线，实现“过温→ALERT 拉高”的硬件链路。  
- 该方案完全移除了 `TPS3823` 看门狗相关网络，不再生成 `FAULT_LATCH`/`SAFE_OK`；硬件依赖 MCU 自主喂狗与否的路径也被删除。  

```
TEMP_FAULT_N ─┬───────────────┐
              │               │
              │           ┌───▼───┐
              │           │ U19   │  施密特反相器
              │           └───┬───┘
              │               │TEMP_FAULT
              │               │
              │           ┌───▼───┐
              │           │ U13   │  三态缓冲
              │           └───┬───┘
              │               │
              │               ▼
              │             ALERT
              │
              │           ┌───────┐
              └─> U22 ────►       │
PSTOP_MCU ────────────────► AND   │──► PSTOP_CTL ──► BOARD_INV ──► PSTOP
                          └───────┘
```

| `TEMP_FAULT_N` | `PSTOP_MCU` | `PSTOP_CTL` | `PSTOP` | `TEMP_FAULT` | `ALERT` |
| --- | --- | --- | --- | --- | --- |
| 0 (过温) | 0 (MCU 停机) | 0 | 1 (停机) | 1 | 1 |
| 0 (过温) | 1 (MCU 允许) | 0 | 1 (停机) | 1 | 1 |
| 1 (正常) | 0 (MCU 停机) | 0 | 1 (停机) | 0 | 0 |
| 1 (正常) | 1 (MCU 允许) | 1 | 0 (放行) | 0 | 0 |

> 注意：由于去掉了看门狗，硬件再也无法覆盖 “MCU 卡死但温度正常” 的场景。若后续需要恢复该能力，可参考方案 A~C 加回 `TPS3823` 及其派生逻辑。

### 3.6 时序要点

> 若采用方案 D，则条目 1 与 4 中涉及 `WD_RST_N` 的约束不适用，硬件只剩温度链路与 MCU 许可。

1. **上电阶段（方案 A~C）**：看门狗芯片默认 200 ms 复位窗口，此时 `WD_RST_N=0` → `FAULT_LATCH=1` → `PSTOP_CTL` 保持 0，使功率级在上电自检前停机。  
2. **温度告警**：温度传感器越限时 `TEMP_ALERT_N`（或当前网表中的 `TEMP_FAULT_N`）被拉低，引发 `FAULT_LATCH=1`，各方案都会立即拉高停机请求并拉高 `ALERT`。  
3. **MCU 急停**：固件若要主动停机，只需把 `PSTOP_MCU` 拉低。所有方案都会把该输入视为停机条件，使 `PSTOP_CTL` 立刻跌落。  
4. **恢复条件（方案 A~C）**：必须同时满足 `WD_RST_N=1`、`TEMP_ALERT_N=1`、`PSTOP_MCU=1`，所选方案才会输出 `PSTOP_CTL=1`；板载反相器再把 `PSTOP` 拉低重新放行。

## 4. 实施细节

- `ALERT_BUS` 线上按 TI 建议放置 500 kΩ~1 MΩ 下拉至地；若存在长线，可在近端加 100 pF~220 pF 小电容辅助去耦。  
- `TMP_ALERT_N` 与 I²C 上拉值保持一致（目前 3.3 V / 4.7 kΩ），避免过强上拉造成 TEMP_ALERT_N 漏电流过大。  
- `PSTOP_CTL` 驱动的 N-MOS（已在主板上实现）需确认门极阈值 <2 V；若存在电平不够的问题，可在该节点追加缓冲器。  
- 若固件端希望进一步区分语义，可在软件层将 `PSTOP_MCU` 抽象为 “停机请求” 信号，再映射到本文硬件命名。  
- 若后续需要保留硬件锁存，可在 `FAULT_LATCH` 后加 SR 锁存电路（例如用一对互补 NOR 门），要求人工复位后方能恢复。

## 5. 方案成本评估

- **方案 A**（双 NAND + 反相器）：SN74LVC2G132DCUR 约 ¥0.91/片（[`component-candidates/nand_gate_options_youxin.md`](component-candidates/nand_gate_options_youxin.md)），SN74LVC1G14DBVR 约 ¥0.14/片（[`component-candidates/single_inverter_options.md`](component-candidates/single_inverter_options.md)），合计约 ¥1.05。  
- **方案 B**（NAND + 三输入与门）：74AUP1G00 系列约 ¥0.49/片（[`component-candidates/schmitt_trigger_nand_options.md`](component-candidates/schmitt_trigger_nand_options.md)），SN74LVC1G11DCKR 约 ¥0.19/片（[`component-candidates/three_input_and_gate_options_youxin.md`](component-candidates/three_input_and_gate_options_youxin.md)），合计约 ¥0.68。  
- **方案 C**（NAND + 或门 + 反相器）：SN74LVC2G132DCUR 约 ¥0.91/片（[`component-candidates/nand_gate_options_youxin.md`](component-candidates/nand_gate_options_youxin.md)），74LVC1G32GV,125 约 ¥0.27/片（[`component-candidates/or_gate_options_youxin.md`](component-candidates/or_gate_options_youxin.md)），SN74LVC1G14DBVR 约 ¥0.14/片（[`component-candidates/single_inverter_options.md`](component-candidates/single_inverter_options.md)），合计约 ¥1.32。  

## 6. 实施决策

受 PCB 空间与布线约束，最终量产板采用 **方案 D（TMP75 直驱与门）**。固化配置如下：
- `U12`（TMP75AIDGKR）→ `TEMP_FAULT_N` 经 `R58` 上拉。
- `U19`（74LVC1G14GW）反相生成 `TEMP_FAULT`，`U13`（74AUP1T34GW）将其推挽驱动到 `ALERT` 总线。
- `U22`（SN74AUP1G08DCKR）实现 `PSTOP_CTL = TEMP_FAULT_N · PSTOP_MCU`，再由板载反相网络得到 `PSTOP`。

若未来重新引入硬件看门狗锁存，可回滚至方案 B，并根据前文器件建议更新原理图与 BOM。

---

[1]: https://www.ti.com/lit/ds/symlink/tps3823.pdf  
[2]: https://www.ti.com/lit/ds/symlink/tmp75.pdf  
[3]: https://www.ti.com/lit/gpn/bq76920  
[4]: [`firmware/smart-battery/SOFTWARE_DESIGN.md`](../../firmware/smart-battery/SOFTWARE_DESIGN.md) 章节 2 及公司内部原理图记录
[^nand-price]: [`component-candidates/schmitt_trigger_nand_options.md`](component-candidates/schmitt_trigger_nand_options.md) 记录了多款单路施密特 NAND（74AUP1G00、SN74AHC1G00 等）参数，需在采购阶段确认实际单价后填入。
