# 智能电池详情页功能设计（方向键向下）

## 目标与范围

- 目标：在仪表盘下按一次“方向键 ↓”即可查看电池充/放、分节电压与均衡状态，便于快速确认电池健康。
- 范围：仅 UI 与输入流转，不改动硬件驱动、充放策略。

## 数据与来源

- 模式/包电压/包电流：复用主控现有字段（BQ76920→STM32→主控），IBAT 为 BQ76920 分流电流。
- 分节电压：BQ76920 逐节 mV。
- 均衡节号：来自 BQ76920 均衡状态或上层策略。
- 温度：暂占位 `--`，预留 4 槽。
- 刷新：与仪表盘同频 5–10 Hz；分节电压可选 2–4 样本滑动平均。

## UI 布局（160×50，7×10 点阵，基线 y=1+n×12）

1. 行1：`BAT <CHG|DSG|IDLE> <VV.VV>V <II.I>A`  
   - 标签/状态 CYAN，电压 ORANGE，电流 RED。
2. 行2：电芯 1–3，格式 `<n>:<mV>`，编号 CYAN，电压 ORANGE，半格（4 px）间隔。
3. 行3：电芯 4–5 + `BAL<n>`（YELLOW），同样半格间隔。
4. 行4：`TEMP--°C--°C--°C--°C`，GRAY，占位。

色表与排版遵循 `docs/software/ui-spec.md`；参考像素稿：  
`docs/software/assets/dashboard_batt_detail_on.png` / `_off.png`。

## 闪烁规则

- 被均衡单节：亮帧 ORANGE，暗帧 GRAY（数字与冒号保留）。推荐 2 Hz、50% 占空。

## 交互与状态机

- 屏幕状态：`Dashboard` ↔ `BattDetail`。  
  - `Down`：Dashboard → BattDetail（blink_on 初始 true）。  
  - `Up`：BattDetail → Dashboard。  
- 闪烁翻转：200–500 ms 定时切换 `blink_on`。
- 其他按键行为与仪表盘一致（上层输入处理决定）。

## 渲染接口（建议）

- 新增 `BattDetailData`：
  - `mode: Mode`
  - `pack_v_mv: u32`
  - `pack_i_ma: i32`
  - `cells_mv: [Option<u16>; 5]`
  - `balancing_index: Option<u8>`
  - `temps_c: [Option<i16>; 4]`
  - `blink_on: bool`
- 新增 `render_batt_detail_once(...)`：按上方布局绘制一帧。

## 容错与占位

- 缺失/失败：显示 `--`（GRAY）；`balancing_index` 缺失则不绘制 BAL。
- 超量程：电压 `>99V`，电流 `>99A`，与仪表盘一致。

## 性能

- 全屏重绘 ≤10 fps；优先按行脏矩形刷新。
- 不新增字形；复用现有 7×10 位图。

## 验收要点

- 像素边界内（≤160×50），on/off 两帧仅颜色变化无抖动。
- 方向键切换 1 帧内响应；闪烁稳定 2 Hz。
- 数据缺失时安全回退占位，不影响其他行显示。

## 待确认

- 温度数据最终来源（BAT/UPS/充电器）的接入路径与采样周期。
- 是否需要在详情页追加 SoC/总电量或告警标记。
