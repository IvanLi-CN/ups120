# UPS 主控充电策略（ESP32-S3 侧）

> 版本：2025-11-29  
> 角色：UPS 主控（ESP32-S3）  
> 范围：仅描述 **UPS 充电策略**，不约束智能电池在独立使用场景下的自充电逻辑。

---

## 1. 设计目标

- 在 AC 适配器掉电/抖动时，**立即停止充电**，避免靠电池倒灌或在输入不稳定时硬顶充电。
- 在 AC 恢复时，要求电源 **连续稳定一段时间（10 s）** 后再恢复充电，滤除插头晃动、墙插抖动等短暂故障。
- **充电策略完全由 UPS 主控（ESP32）决定**：只使用 UPS 本侧的输入电源状态与电池电压/温度，不依赖智能电池上报的 AC 状态位。

---

## 2. AC 状态检测来源（唯一可信输入）

- AC 是否存在，只通过 **TCA6408A 的 `IN_PG` 引脚** 判断：
  - `IN_PG = High` → `vin_present = true`（适配器存在 / USB-C 5V 正常）  
  - `IN_PG = Low`  → `vin_present = false`（适配器缺失或严重掉压）
- ESP32 周期性读取 `IN_PG`（当前控制循环约 500 ms 一次），映射为 `vin_present`，并传给：
  - 风扇控制器（降噪/保护逻辑）；
  - 充电策略（是否允许开启充电）。

> 约束：**UPS 充电策略不得以任何形式使用 STM32 智能电池的 `AC_PRESENT` 位或其它 AC 相关状态位作为判据。**  
> 这些位只允许作为诊断日志字段（例如打印 `stm_ac` 供对比），不参与任何决策。

---

## 3. AC 掉电行为（必须立即停充）

当检测到 `IN_PG` 出现 **高→低** 边沿（`vin_present` 由 `true` 变为 `false`）时，ESP32 必须：

1. 立即清除智能电池 `CHG_CONFIG` 中的 `MANUAL_ENABLE` 位：  
   - 通过 I²C 写 `CHG_CONFIG` 寄存器（地址 `SB_REG_CHG_CONFIG`），将 `MANUAL_ENABLE=false`；  
   - 保持 `AUTO_ENABLED`（自动算法）按既有设计开启/关闭，由智能电池自行管理安全保护。
2. 在充电决策日志中记录一次事件，例如：
   - `charge: disabled because adapter is missing`
3. 后续控制循环中，只要 `vin_present=false`，即使 `VBAT` 低于起充电压阈值，也 **不得尝试重新打开 `MANUAL_ENABLE`**。

> 总结：AC 掉电 = **立刻停充**，以 UPS 主控为最终裁决者。

---

## 4. AC 恢复行为（10 秒稳定窗口）

当检测到 `IN_PG` 出现 **低→高** 边沿（`vin_present` 由 `false` 变为 `true`）时：

1. 记录一个时间戳 `vin_state_last_change_ms = now_ms`；
2. 定义逻辑量 `vin_ok_for_charge`：
   - `vin_ok_for_charge = vin_present && (now_ms - vin_state_last_change_ms >= 10_000 ms)`；
   - 即：`IN_PG` 必须 **连续为 High ≥ 10 s** 才认为输入电源“稳定可充电”。
3. 在 10 s 稳定窗口内：
   - 允许继续保持“已关闭”状态（`MANUAL_ENABLE=false`）；
   - **禁止因为 `VBAT` 低而重新开启 `MANUAL_ENABLE`**；
   - 可以打印提示日志，例如：
     - `charge: skip enable (adapter unstable, vbat=XXXXmV)`
4. 只有当 `vin_ok_for_charge=true` 时，才进入正常的电压阈值判定：
   - `VBAT <= CHARGE_START_VBAT_MV` → 允许将 `MANUAL_ENABLE` 从 `false` 置为 `true`；
   - `VBAT >= CHARGE_STOP_VBAT_MV`  → 允许将 `MANUAL_ENABLE` 从 `true` 清为 `false`。

若在 10 s 窗口内 `IN_PG` 再次抖动（高↔低），必须：

- 立即执行“AC 掉电行为”（见上节，清除 `MANUAL_ENABLE`）；  
- 重置 `vin_state_last_change_ms`，重新计时新的 10 s 稳定窗口。

---

## 5. 与温度/电池状态的优先级关系

在做出是否开启充电的决策时，优先级从高到低为：

1. **温度暂停**：若电池/充电器温度超过阈值（由 STM32 上报），进入温度暂停状态：  
   - 即使 `vin_ok_for_charge=true` 且 `VBAT` 很低，也 **不得** 开启 `MANUAL_ENABLE`；  
   - 只有温度恢复到恢复阈值以下，才允许退出暂停。
2. **AC 缺失或不稳定**：
   - `vin_present=false` → 立刻停充；  
   - `vin_present=true` 但 `vin_ok_for_charge=false` → 不开启充电，仅打印“适配器不稳定”日志。
3. **电池电压阈值**：
   - `VBAT <= CHARGE_START_VBAT_MV` 且上述条件全部满足 → 允许开启充电；  
   - `VBAT >= CHARGE_STOP_VBAT_MV` → 关闭充电。

---

## 6. 与 STM32 智能电池逻辑的关系

智能电池 MCU（STM32L0 + SC8815）可以在**独立使用**场景下，基于自身感知的 `AC_PRESENT` 位等信息，实现自主充电策略；但在 UPS 项目中，约束如下：

1. **UPS 充电策略由 ESP32 主控统一裁决**：
   - ESP32 通过 `CHG_CONFIG.MANUAL_ENABLE` 控制是否允许充电；
   - STM32 仅负责执行充电芯片的底层驱动与保护逻辑。
2. **禁止反向依赖 STM32 的 AC 状态**：
   - UPS 主控不得用 `STATE_FLAGS.AC_PRESENT`、`STATE_FLAGS.*` 或任何 STM32 上报的 AC 状态位来决定是否充电；
   - 这些位只允许用于日志（如 `power: adapter present (stm_ac=true flags=0xXXXX)`），方便排查现场差异。
3. **两侧策略互不串味**：
   - UPS 项目的“AC 掉电/恢复策略”仅在 ESP32 固件中实现和约束；
   - 智能电池在其它产品/板卡上的自充电策略，可以独立演进，不影响 UPS。

---

## 7. 实现与验证要点

- 实现位置：`firmware/ups-main/src/main.rs` 中电源状态刷新与充电决策逻辑处：
  - 在读取 `IN_PG` 的逻辑中维护 `vin_present` 与 `vin_state_last_change_ms`；
  - 在充电决策分支中判断 `vin_ok_for_charge`，并输出清晰的 `INFO` 日志。
- 验证步骤建议：
  1. 上电、接入 AC，等待 10 s 以上，确认日志中出现 `charge: enabled ...` 且开始充电；
  2. 在充电过程中拔掉 AC，确认立即看到 `charge: disabled because adapter is missing`，电流降为放电或待机；
  3. 在 10 s 内反复“插拔” AC，确认始终 **不会** 重新开启充电，只看到 “adapter missing/unstable” 类日志；
  4. AC 持续稳定 ≥10 s 后，再次观察到按照电压阈值策略重新开启充电。

上述行为为 UPS 项目中 **必须遵守的充电策略约束**，后续任何改动应同步更新本文档并在硬件上完成回归验证。

