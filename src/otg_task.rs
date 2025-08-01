//! OTG任务模块
//!
//! 负责管理SC8815的OTG功能，实现智能电压控制：
//! - 基于INA226电压检测实现智能电压控制
//! - 高电压时(>90% Vs)输出Vs-0.5V，低电压时(<70% Vs)输出Vs
//! - 滞回控制机制避免频繁切换
//! - 1A电流限制和故障检测

use defmt::*;
use embassy_time::{Duration, Timer};

use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_rp::i2c::I2c;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

use sc8815::{DeviceConfiguration, OperatingMode, SC8815};

use crate::data_types::{OtgConfiguration, OtgControlState, OtgStatus};
use crate::shared::{Ina226MeasurementsSubscriber, OtgStatusPublisher};

/// OTG任务 - 使用SC8815实现OTG功能
#[embassy_executor::task]
pub async fn otg_task(
    i2c_bus: I2cDevice<
        'static,
        CriticalSectionRawMutex,
        I2c<'static, embassy_rp::peripherals::I2C1, embassy_rp::i2c::Async>,
    >,
    address: u8,
    config: OtgConfiguration,
    mut ina226_measurements_subscriber: Ina226MeasurementsSubscriber<'static>,
    otg_status_publisher: OtgStatusPublisher<'static>,
    mut otg_pstop_pin: embassy_rp::gpio::Output<'static>,
) {
    info!("OTG task started - SC8815 OTG mode with voltage control");
    info!(
        "OTG Config: Vs={}mV, High={}%, Low={}%",
        config.target_voltage_mv, config.high_threshold_percent, config.low_threshold_percent
    );

    // 创建SC8815驱动实例
    let mut sc8815 = SC8815::new(i2c_bus, address);

    // 初始化SC8815为OTG模式
    if let Err(e) = sc8815.init().await {
        error!(
            "Failed to initialize SC8815 for OTG: {}",
            defmt::Debug2Format(&e)
        );
        publish_fault_status(&otg_status_publisher, &config).await;
        return;
    }

    // 保持PSTOP为HIGH - SC8815在安全待机模式下进行配置
    otg_pstop_pin.set_high();
    info!("OTG PSTOP set to HIGH - SC8815 in safe standby mode for configuration");

    // 首先禁用短路折返功能以允许带负载启动
    // 当VBUS < 1V时，SC8815通常会将电流限制降低到22%(IBUS)和10%(IBAT)
    // 禁用此功能以允许带负载启动
    if let Err(_e) = sc8815.set_short_foldback_disable(true).await {
        error!("Failed to disable short circuit foldback");
        publish_fault_status(&otg_status_publisher, &config).await;
        return;
    }
    info!("Short circuit foldback disabled for startup with load");

    // 配置OTG模式 - 完整配置（在待机模式下安全配置）
    let mut device_config = DeviceConfiguration::default();

    // 配置电池设置（关键修复 - 参考示例）
    device_config.battery.cell_count = sc8815::CellCount::Cells4S;
    device_config.battery.voltage_per_cell = sc8815::VoltagePerCell::Mv4200;
    device_config.battery.use_internal_setting = true;

    device_config.power.dead_time = sc8815::DeadTime::Ns80; // 关键修复 - 参考示例
    device_config.power.vinreg_voltage_mv = config.target_voltage_mv; // 目标输出电压
    // 电源管理配置
    device_config.power.operating_mode = OperatingMode::OTG; // OTG模式
    device_config.power.switching_frequency = sc8815::SwitchingFrequency::Freq450kHz;

    device_config.current_limits.ibus_limit_ma = 1500; // 1.5A限制（参考示例）
    device_config.current_limits.ibat_limit_ma = 2000; // 2A电池电流限制
    // 电流限制配置 - 参考示例使用较保守的限制
    device_config.current_limits.rs1_mohm = 5; // 5mΩ感应电阻
    device_config.current_limits.rs2_mohm = 5; // 5mΩ感应电阻

    // OTG模式特定配置
    device_config.trickle_charging = false; // 禁用涓流充电
    info!("Configuring SC8815 for OTG mode in standby...");
    device_config.charging_termination = false; // 禁用充电终止
    device_config.use_ibus_for_charging = false; // 不使用IBUS作为充电参考

    if let Err(_e) = sc8815.configure_device(&device_config).await {
        error!("Failed to configure SC8815 for OTG");
        publish_fault_status(&otg_status_publisher, &config).await;
        return;
    }

    info!("SC8815 OTG mode configured successfully in standby");

    // 配置OTG模式（在待机模式下）
    info!("Configuring OTG mode in standby...");
    if let Err(_e) = sc8815.set_otg_mode(true).await {
        error!("Failed to configure OTG mode");
        publish_fault_status(&otg_status_publisher, &config).await;
        return;
    }
    info!("OTG mode configured successfully in standby");

    // 设置VBUS输出电压（在待机模式下）
    let vbus_ratio = if config.target_voltage_mv > 10240 {
        0
    } else {
        1
    };
    info!(
        "Setting VBUS output voltage to {}mV with ratio {}...",
        config.target_voltage_mv, vbus_ratio
    );
    if let Err(_e) = sc8815
        .set_vbus_internal_voltage(config.target_voltage_mv, vbus_ratio)
        .await
    {
        error!("Failed to set VBUS voltage");
        publish_fault_status(&otg_status_publisher, &config).await;
        return;
    }
    info!(
        "VBUS voltage set to {}mV successfully",
        config.target_voltage_mv
    );

    // 启用ADC转换（在待机模式下）
    if let Err(_e) = sc8815.set_adc_conversion(true).await {
        error!("Failed to configure ADC conversion");
        publish_fault_status(&otg_status_publisher, &config).await;
        return;
    }
    info!("ADC conversion configured in standby mode");

    // 清除VBUS短路故障（如果存在）- 在待机模式下安全执行
    // 使用官方推荐的方法：清除DIS_ShortFoldBack（保持10ms），然后重新设置为1
    if let Err(_e) = sc8815
        .clear_vbus_short_fault_with_delay(|| async {
            embassy_time::Timer::after(embassy_time::Duration::from_millis(10)).await;
        })
        .await
    {
        error!("Failed to clear VBUS short fault");
    } else {
        info!("VBUS short fault cleared using official method");
    }

    // 所有配置完成 - 现在可以安全启用电源块
    info!("🔧 Configuration complete - NOW enabling power blocks (PSTOP LOW)");
    otg_pstop_pin.set_low(); // 启用电源块 - 关键时序！
    Timer::after(Duration::from_millis(10)).await; // 等待电源块稳定

    info!(
        "✅ SC8815 configured as OTG: {}mV output, 1.5A current limit",
        config.target_voltage_mv
    );
    info!("Connect load to start power delivery");

    // 控制状态变量 - 初始化为目标电压
    let mut current_output_voltage_mv = config.target_voltage_mv;
    let mut last_voltage_v = 0.0;
    let mut voltage_stable_count: u32 = 0;

    // 计算阈值
    let vs_v = config.target_voltage_mv as f32 / 1000.0;
    let high_threshold_v = vs_v * (config.high_threshold_percent as f32 / 100.0);
    let low_threshold_v = vs_v * (config.low_threshold_percent as f32 / 100.0);
    let reduced_voltage_v = vs_v - (config.voltage_reduction_mv as f32 / 1000.0);

    info!(
        "OTG thresholds: High={}V, Low={}V, Reduced={}V",
        high_threshold_v, low_threshold_v, reduced_voltage_v
    );

    loop {
        // 获取INA226测量数据
        let ina226_measurements = ina226_measurements_subscriber.next_message_pure().await;
        let input_voltage_v = ina226_measurements.voltage;

        // 检查电压变化稳定性
        let voltage_changed = (input_voltage_v - last_voltage_v).abs() > 0.1;
        if voltage_changed {
            voltage_stable_count = 0;
        } else {
            voltage_stable_count = voltage_stable_count.saturating_add(1);
        }
        last_voltage_v = input_voltage_v;

        // 确定目标输出电压和控制状态
        let (target_output_voltage_mv, control_state) = if input_voltage_v > high_threshold_v {
            // 高电压：输出 Vs - 0.5V
            (
                (reduced_voltage_v * 1000.0) as u16,
                OtgControlState::HighVoltage,
            )
        } else if input_voltage_v < low_threshold_v {
            // 低电压：输出 Vs
            (config.target_voltage_mv, OtgControlState::LowVoltage)
        } else {
            // 滞回控制：保持当前输出
            (current_output_voltage_mv, OtgControlState::Normal)
        };

        // 更新输出电压（如果需要且电压稳定）
        if target_output_voltage_mv != current_output_voltage_mv && voltage_stable_count >= 3 {
            // 根据电压选择合适的比率：>10.24V使用12.5x，<=10.24V使用5x
            let vbus_ratio = if target_output_voltage_mv > 10240 {
                0
            } else {
                1
            };

            if let Err(_e) = sc8815
                .set_vbus_internal_voltage(target_output_voltage_mv, vbus_ratio)
                .await
            {
                error!("Failed to set OTG output voltage");
                // 发布故障状态
                publish_fault_status(&otg_status_publisher, &config).await;
                continue;
            }

            current_output_voltage_mv = target_output_voltage_mv;
            info!(
                "OTG voltage updated: {}mV (state: {:?})",
                current_output_voltage_mv, control_state
            );
        }

        // 调试：每10秒强制设置一次电压并检查状态
        static mut LAST_FORCE_UPDATE: u32 = 0;
        let current_time = embassy_time::Instant::now().as_millis() as u32;
        unsafe {
            if current_time - LAST_FORCE_UPDATE > 10000 {
                let vbus_ratio = if target_output_voltage_mv > 10240 {
                    0
                } else {
                    1
                };
                if sc8815
                    .set_vbus_internal_voltage(target_output_voltage_mv, vbus_ratio)
                    .await
                    .is_ok()
                {
                    info!("Force updated OTG voltage: {}mV", target_output_voltage_mv);
                }

                // 读取SC8815状态寄存器进行调试
                if let Ok(status) = sc8815
                    .read_register(sc8815::registers::Register::Status)
                    .await
                {
                    info!("SC8815 OTG Status: 0x{:02X}", status);
                }

                LAST_FORCE_UPDATE = current_time;
            }
        }

        // 读取当前输出状态
        let (output_current_ma, actual_voltage_mv) = match read_otg_status(&mut sc8815).await {
            Ok(status) => status,
            Err(_e) => {
                error!("Failed to read OTG status");
                publish_fault_status(&otg_status_publisher, &config).await;
                continue;
            }
        };

        // 检查过载保护
        if output_current_ma > 1200 {
            // 1A + 20%容差
            warn!("OTG current overload: {}mA > 1200mA", output_current_ma);
            publish_fault_status(&otg_status_publisher, &config).await;
            continue;
        }

        // 发布OTG状态
        // 调试：读取原始ADC寄存器值
        if let Ok((vbus_high, vbus_low)) = sc8815
            .read_consecutive_registers(sc8815::registers::Register::VbusFbValue)
            .await
        {
            let vbus_value2 = (vbus_low >> 6) & 0x03;
            info!(
                "Raw VBUS ADC: HIGH={}, LOW={}, VALUE2={}",
                vbus_high, vbus_low, vbus_value2
            );
        }

        // 读取RATIO寄存器来确认比率设置
        if let Ok(ratio_reg) = sc8815
            .read_register(sc8815::registers::Register::Ratio)
            .await
        {
            let vbus_ratio_bit = ratio_reg & 0x01;
            let vbat_ratio_bit = (ratio_reg >> 1) & 0x01;
            info!(
                "RATIO register: 0x{:02X}, VBUS_RATIO={}, VBAT_RATIO={}",
                ratio_reg, vbus_ratio_bit, vbat_ratio_bit
            );
        }

        let otg_status = OtgStatus {
            enabled: config.enabled,
            output_voltage_mv: actual_voltage_mv,
            output_current_ma,
            input_voltage_mv: (input_voltage_v * 1000.0) as u16,
            control_state,
            last_update_ms: embassy_time::Instant::now().as_millis(),
        };

        otg_status_publisher.publish_immediate(otg_status);

        // 读取完整的ADC测量值
        let measurements = match sc8815.get_adc_measurements().await {
            Ok(m) => m,
            Err(_) => {
                error!("Failed to read OTG ADC measurements");
                continue;
            }
        };

        // 读取PSTOP引脚电平
        let pstop_level = if otg_pstop_pin.is_set_low() {
            "LOW"
        } else {
            "HIGH"
        };

        // OTG输出信息（每秒输出一次）
        info!(
            "[OTG] VBUS: {}mV, IBUS: {}mA, VBAT: {}mV, IBAT: {}mA, PSTOP: {}, State: {:?}",
            measurements.vbus_mv,
            measurements.ibus_ma,
            measurements.vbat_mv,
            measurements.ibat_ma,
            pstop_level,
            control_state
        );

        // 等待下一个周期
        Timer::after(Duration::from_secs(1)).await;
    }
}

/// 读取OTG状态
async fn read_otg_status(
    sc8815: &mut SC8815<
        I2cDevice<
            'static,
            CriticalSectionRawMutex,
            I2c<'static, embassy_rp::peripherals::I2C1, embassy_rp::i2c::Async>,
        >,
    >,
) -> Result<(u16, u16), ()> {
    match sc8815.get_adc_measurements().await {
        Ok(measurements) => Ok((measurements.ibus_ma, measurements.vbus_mv)),
        Err(_) => Err(()),
    }
}

/// 发布故障状态
async fn publish_fault_status(publisher: &OtgStatusPublisher<'static>, _config: &OtgConfiguration) {
    let fault_status = OtgStatus {
        enabled: false,
        output_voltage_mv: 0,
        output_current_ma: 0,
        input_voltage_mv: 0,
        control_state: OtgControlState::Disabled,
        last_update_ms: embassy_time::Instant::now().as_millis(),
    };
    publisher.publish_immediate(fault_status);
    warn!("OTG fault status published");
}
