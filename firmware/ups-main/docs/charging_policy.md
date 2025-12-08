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

AC GOOD 由 **TPS2490 PG + INA226 外部输入电压** 组合判定：

- 硬件连接：
  - TPS2490 `PG` → TCA6408A `P0` → `IN_PG` 网络（开漏，**高电平 = Power Good**）；
  - INA226 挂在与 TCA6408A 相同的 I²C 总线上，测量 UPS 外部输入总线电压。
- 固件中的中间量定义：
  - `in_pg_raw: bool`：由 `read_in_pg()` 读取的原始 PG 电平；
    - `in_pg_raw = true`  → 线为高电平（PG 开路被上拉） = **PG Good**（输入路径导通、MOSFET VDS 足够小且无故障，未处于限流/保护）；  
    - `in_pg_raw = false` → 线被拉低 = **Not Good**（启动中 / 关断 / UVLO / 过流/过功率限流等）。
  - `pg_good = in_pg_raw && !in_pg_read_failed`：TPS2490 视角下输入路径良好；
  - `vin_meas_mv: Option<u32>`：INA226 测得的 UPS 外部输入电压（以 mV 表示）；
  - `ac_present` / `AC GOOD` 定义：

    ```rust
    let pg_good = in_pg_raw && !in_pg_read_failed;
    let vin_ok = vin_meas_mv.map(|v| v > 11_500).unwrap_or(false);
    let ac_present = pg_good && vin_ok;
    ```

    其中 11.5 V 阈值由 `UPS_VBUS_AC_ONLINE_MV` 常量给出。

- 刷新策略：
  - `power::power_task` 每 500 ms：
    - 读取 `IN_PG`，更新 `in_pg_raw`；  
    - 通过 INA226 读取总线电压，更新 `vin_meas_mv`；  
    - 组合出新的 `ac_present`，并在 AC GOOD 边沿上更新 `ac_stable` 窗口（见下一节）。
  - 风扇控制器与 UI 只消费 `PowerState.ac_present / ac_stable`，不直接使用原始 PG。

> 约束：**UPS 充电策略不得以任何形式使用 STM32 智能电池的 `AC_PRESENT` 位或其它 AC 相关状态位作为判据。**  
> 这些位只允许作为诊断日志字段（例如打印 `stm_ac` 供对比），不参与任何决策。

---

## 3. AC 掉电行为（必须立即停充）

当检测到 `ac_present` 出现 **true→false** 边沿（AC GOOD 从存在变为缺失）时，ESP32 必须：

1. 立即清除智能电池 `CHG_CONFIG` 中的 `MANUAL_ENABLE` 位：  
   - 通过 I²C 写 `CHG_CONFIG` 寄存器（地址 `SB_REG_CHG_CONFIG`），将 `MANUAL_ENABLE=false`；  
   - 保持 `AUTO_ENABLED`（自动算法）按既有设计开启/关闭，由智能电池自行管理安全保护。
2. 在充电决策日志中记录一次事件，例如：
   - `charge: disabled because adapter is missing`
3. 后续控制循环中，只要 `ac_present=false`，即使 `VBAT` 低于起充电压阈值，也 **不得尝试重新打开 `MANUAL_ENABLE`**。

> 总结：AC 掉电 = **立刻停充**，以 UPS 主控为最终裁决者。

---

## 4. AC 恢复行为（10 秒稳定窗口）

当检测到 `ac_present` 出现 **false→true** 边沿（AC GOOD 从缺失变为存在）时：

1. 记录一个时间戳 `ac_state_last_change_ms = now_ms`；
2. 定义逻辑量 `vin_ok_for_charge`（实现上等价于 `ac_stable`）：

   ```rust
   let vin_ok_for_charge =
       ac_present && (now_ms - ac_state_last_change_ms >= AC_STABLE_MS);
   // AC_STABLE_MS = 10_000 ms
   ```

   即：`ac_present` 必须 **连续为 true ≥ 10 s** 才认为输入电源“稳定可充电”，从而允许重新开始充电。
3. 在 10 s 稳定窗口内：
   - 允许继续保持“已关闭”状态（`MANUAL_ENABLE=false`）；
   - **禁止因为 `VBAT` 低而重新开启 `MANUAL_ENABLE`**；
   - 可以打印提示日志，例如：
     - `charge: skip enable (adapter unstable, vbat=XXXXmV)`
4. 只有当 `vin_ok_for_charge=true` 时，才进入正常的电压阈值判定：
   - `VBAT <= CHARGE_START_VBAT_MV` → 允许将 `MANUAL_ENABLE` 从 `false` 置为 `true`；
   - `VBAT >= CHARGE_STOP_VBAT_MV`  → 允许将 `MANUAL_ENABLE` 从 `true` 清为 `false`。

若在 10 s 窗口内 AC GOOD 再次抖动（`ac_present` 在 true/false 间切换），必须：

- 立即执行“AC 掉电行为”（见上节，清除 `MANUAL_ENABLE`）；  
- 重置 `vin_state_last_change_ms`，重新计时新的 10 s 稳定窗口。

---

## 5. 与温度/电池状态的优先级关系

在做出是否开启充电的决策时，优先级从高到低为：

1. **温度暂停**：若电池/充电器温度超过阈值（由 STM32 上报），进入温度暂停状态：  
   - 即使 `vin_ok_for_charge=true` 且 `VBAT` 很低，也 **不得** 开启 `MANUAL_ENABLE`；  
   - 只有温度恢复到恢复阈值以下，才允许退出暂停。
2. **AC 缺失或不稳定**：
   - `ac_present=false` → 立刻停充；  
   - `ac_present=true` 但 `vin_ok_for_charge=false`（即 `ac_stable=false`）→ 不开启充电，仅打印“适配器不稳定”日志。
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
  - 在 `power::power_task` 中组合 `in_pg_raw` 与 INA226 电压，维护 `ac_present` 与 `ac_state_last_change_ms`；
  - 在充电决策分支中判断 `vin_ok_for_charge`（`ac_stable`），并输出清晰的 `INFO` 日志。
- 验证步骤建议：
  1. 上电、接入 AC，等待 10 s 以上，确认日志中出现 `charge: enabled ...` 且开始充电；
  2. 在充电过程中拔掉 AC，确认立即看到 `charge: disabled because adapter is missing`，电流降为放电或待机；
  3. 在 10 s 内反复“插拔” AC，确认始终 **不会** 重新开启充电，只看到 “adapter missing/unstable” 类日志；
  4. AC 持续稳定 ≥10 s 后，再次观察到按照电压阈值策略重新开启充电。

上述行为为 UPS 项目中 **必须遵守的充电策略约束**，后续任何改动应同步更新本文档并在硬件上完成回归验证。
