//! LED状态指示任务（单灯规范）
//!
//! 优先级：Fault > Charging(+Bal overlay) > Full(hysteresis) > Idle。
//!
//! - Fault：4 Hz 闪烁（125ms 亮 / 125ms 灭）。
//! - Charging（基线）：1 Hz 闪烁（500ms 亮 / 500ms 灭）。
//! - Balancing 叠加：在 Charging 的亮窗内插入两个“40ms 灭”缺口，间隔 160ms；不改变基线节拍。
//! - Full：依据 VBAT 与 IBAT 的迟滞规则判定；显示为常亮。
//! - Idle：熄灭。

use defmt::*;
use embassy_stm32::gpio::OutputOpenDrain;
use embassy_time::{Duration, Timer};

use crate::global_state::BatteryGlobalState;
use crate::shared::GlobalStateSubscriber;

#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
enum LedMode {
    Fault,
    Preparing,
    Charging,
    Full,
    Idle,
}

// 充电基线节拍与均衡叠加缺口定义
const CHG_PERIOD_MS: u32 = 1000;
const CHG_ON_MS: u32 = 500;
const NOTCH_WIDTH_MS: u32 = 40;
const NOTCH1_START_MS: u32 = 100;
const NOTCH_GAP_MS: u32 = 160; // 缺口间距

/// LED状态指示任务（消费全局状态）
#[embassy_executor::task]
pub async fn led_status_task(
    led_pin: OutputOpenDrain<'static>,
    mut global_state_sub: GlobalStateSubscriber<'static>,
) {
    info!("LED status task started");

    // 配置LED为开漏输出，低使能
    let mut led = led_pin;
    led.set_high(); // 初始状态LED关闭（高电平）

    let mut mode = LedMode::Idle;
    let mut last_toggle = embassy_time::Instant::now(); // 给 Fault 用
    let mut chg_cycle_start = embassy_time::Instant::now();
    let mut overlay_prev: Option<bool> = None;

    // 保存最新的SC8815数据
    let mut latest_state: BatteryGlobalState = BatteryGlobalState::default();

    loop {
        // 非阻塞抓取全局状态
        if let Some(s) = global_state_sub.try_next_message_pure() {
            latest_state = s;
        }

        // 先检查 BQ/SC 故障（最高优先级，立即生效）
        let mut any_fault = false;
        any_fault = latest_state.fault_battery || latest_state.fault_charger;

        let now = embassy_time::Instant::now();

        if any_fault {
            // 故障：4Hz 闪烁，抢占显示
            if now.duration_since(last_toggle) >= Duration::from_millis(125) {
                led.toggle();
                last_toggle = now;
            }
            // 处于故障时不更新其它状态的迟滞累计
            Timer::after(Duration::from_millis(10)).await;
            continue;
        }

        // 评估“充电/暂停/满电/空闲”与迟滞
        // 模式选择（不含故障）：Charging 优先于 Full，再到 Idle
        let desired_mode = if latest_state.charging {
            LedMode::Charging
        } else if latest_state.preparing {
            LedMode::Preparing
        } else if latest_state.full {
            LedMode::Full
        } else {
            LedMode::Idle
        };
        if desired_mode != mode {
            mode = desired_mode;
            if matches!(mode, LedMode::Charging) {
                chg_cycle_start = now;
            }
        }

        // 输出控制
        match mode {
            LedMode::Preparing => {
                // 准备充电：基线熄灭，仅在1s周期中给出两个短亮（80ms）提示（100ms与300ms）
                let elapsed_ms = now.duration_since(chg_cycle_start).as_millis() as u32;
                let phase = elapsed_ms % CHG_PERIOD_MS;
                let in_pulse1 = phase >= 100 && phase < 180; // 80ms
                let in_pulse2 = phase >= 300 && phase < 380; // 80ms
                if in_pulse1 || in_pulse2 {
                    led.set_low();
                } else {
                    led.set_high();
                }
            }
            LedMode::Charging => {
                let elapsed_ms = now.duration_since(chg_cycle_start).as_millis() as u32;
                let phase = elapsed_ms % CHG_PERIOD_MS;
                let in_on = phase < CHG_ON_MS;
                let mut on = in_on;
                // 缺口仅代表“硬件均衡实际进行中”：两缺口（100/140、300/340）
                if in_on && latest_state.balancing_active {
                    let notch2_start = NOTCH1_START_MS + NOTCH_GAP_MS + NOTCH_WIDTH_MS; // 300
                    let in_notch1 =
                        phase >= NOTCH1_START_MS && phase < (NOTCH1_START_MS + NOTCH_WIDTH_MS);
                    let in_notch2 =
                        phase >= notch2_start && phase < (notch2_start + NOTCH_WIDTH_MS);
                    if in_notch1 || in_notch2 {
                        on = false;
                    }
                }
                // 仅在叠加状态改变时记录一条日志，避免刷屏
                if overlay_prev
                    .map(|v| v != latest_state.balancing_active)
                    .unwrap_or(true)
                {
                    info!("led_chg overlay={}", latest_state.balancing_active);
                    overlay_prev = Some(latest_state.balancing_active);
                }
                if on {
                    led.set_low();
                } else {
                    led.set_high();
                }
            }
            LedMode::Full => {
                // 常亮（开漏低）
                led.set_low();
            }
            LedMode::Idle => {
                // 熄灭
                led.set_high();
            }
            LedMode::Fault => ::core::unreachable!(),
        }

        // 短暂延时避免CPU占用过高
        Timer::after(Duration::from_millis(10)).await;
    }
}

// kept empty on purpose: LED task uses derived global state and
// does not re-evaluate fault conditions locally anymore.
