#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::{bind_interrupts, i2c, peripherals, time::Hertz};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

// Import the BQ769x0 driver crate
use bq769x0_async_rs::registers::*;
use bq769x0_async_rs::{
    Bq769x0, BatteryConfig, TempSensor, ScdDelay, OcdDelay, UvOvDelay, ProtectionConfig,
};

// Define the I2C interrupt handler
bind_interrupts!(struct Irqs {
    I2C1 => i2c::EventInterruptHandler<peripherals::I2C1>, i2c::ErrorInterruptHandler<peripherals::I2C1>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Starting BQ76920 demo...");

    let config = embassy_stm32::Config::default();
    // Clock configuration is handled by default config or external means as per user instruction.
    // If specific clock speeds are needed, adjust the default config or provide a custom one.

    let p = embassy_stm32::init(config);

    info!("STM32 initialized.");

    // Configure I2C1 (PB6 SCL, PB7 SDA) with DMA
    // Ensure these pins are configured as Alternate Function Open Drain with Pull-ups in your STM32CubeIDE or equivalent configuration tool
    // Assuming DMA1_CH1 for TX and DMA1_CH2 for RX for I2C1 on STM32G031G8U6
    let mut config = i2c::Config::default();
    config.scl_pullup = true;
    config.sda_pullup = true;

    let i2c = i2c::I2c::new(
        p.I2C1,         // 1. peri
        p.PB6,          // 2. scl
        p.PB7,          // 3. sda
        Irqs,           // 4. _irq
        p.DMA1_CH1,     // 5. tx_dma (Assuming DMA1_CH1 for I2C1 TX)
        p.DMA1_CH2,     // 6. rx_dma (Assuming DMA1_CH2 for I2C1 RX)
        Hertz(100_000), // 7. freq
        config, // 8. config
    );

    info!("I2C1 initialized on PB6/PB7 with DMA.");

    // BQ76920 I2C address (7-bit)
    let bq76920_address = 0x08;
    // Pass the I2C peripheral instance by value
    let mut bq: Bq769x0<_, bq769x0_async_rs::Enabled> = Bq769x0::new(i2c, bq76920_address);

    info!("BQ76920 driver instance created.");

    // --- BQ76920 Initialization Sequence ---

    // Note: Waking from SHIP mode is typically handled by external hardware (TS1 pin).
    // Assuming the chip is already in NORMAL mode or has been woken up.

    // Define battery configuration
    let battery_config = BatteryConfig {
        load_present: false,
        adc_enable: true,
        temp_sensor_selection: TempSensor::Internal,
        shutdown_a: false,
        shutdown_b: false,
        delay_disable: false,
        cc_enable: true,
        cc_oneshot: false,
        discharge_on: true, // Enable discharging
        charge_on: true,    // Enable charging
        overvoltage_trip_mv: 4200,
        undervoltage_trip_mv: 2800,
        protection_config: ProtectionConfig {
            rsns_enable: true,
            scd_delay: ScdDelay::Delay70us,
            scd_limit_ma: 60000,
            ocd_delay: OcdDelay::Delay10ms,
            ocd_limit_ma: 20000,
            uv_delay: UvOvDelay::Delay1s,
            ov_delay: UvOvDelay::Delay1s,
        },
        rsense_m_ohm: 3.0, // Use 3mOhm as specified in the original config
    };

    info!("Applying battery configuration...");
    if let Err(e) = bq.set_config(&battery_config).await {
        error!("Failed to apply battery configuration: {:?}", e);
        loop {}
    }
    info!("Battery configuration applied successfully.");

    // 4. Clear initial fault flags
    // Write 0xFF to SYS_STAT to clear all flags
    info!("Clearing initial status flags (writing 0xFF to SYS_STAT)...");
    if let Err(e) = bq.clear_status_flags(0xFF).await {
        error!("Failed to clear status flags: {:?}", e);
        loop {}
    }
    info!("Initial status flags cleared successfully.");

    info!("BQ76920 initialization complete.");

    // --- Main Loop for Data Acquisition ---
    let sense_resistor_m_ohm = 5.0; // Your sense resistor value in milliOhms

    loop {
        info!("--- Reading BQ76920 Data ---");

        // Read Cell Voltages
        match bq.read_cell_voltages().await {
            Ok(voltages) => {
                info!("Cell Voltages (mV):");
                // BQ76920 supports up to 5 cells
                for _i in 0..5 {
                    info!("  Cell {}: {} mV", _i + 1, voltages.voltages_mv[_i]);
                }
            }
            Err(e) => {
                error!("Failed to read cell voltages: {:?}", e);
            }
        }

        // Read Pack Voltage
        match bq.read_pack_voltage().await {
            Ok(voltage) => {
                info!("Pack Voltage: {} mV", voltage);
            }
            Err(e) => {
                error!("Failed to read pack voltage: {:?}", e);
            }
        }

        // Read Temperatures
        match bq.read_temperatures().await {
            Ok(temps) => {
                if temps.is_thermistor {
                    info!("Temperatures (0.1 Ohms):");
                    info!("  TS1: {} ({} Ohms)", temps.ts1, temps.ts1 as f32 / 10.0);
                    // BQ76920 only has TS1
                } else {
                    info!("Temperatures (deci-Celsius):");
                    info!(
                        "  TS1 (Die Temp): {} ({} °C)",
                        temps.ts1,
                        temps.ts1 as f32 / 10.0
                    );
                }
            }
            Err(e) => {
                error!("Failed to read temperatures: {:?}", e);
            }
        }

        // Read Current
        match bq.read_current().await {
            Ok(current) => {
                let current_ma =
                    bq.convert_raw_cc_to_current_ma(current.raw_cc, sense_resistor_m_ohm);
                info!("Raw CC: {}, Current: {} mA", current.raw_cc, current_ma);
            }
            Err(e) => {
                error!("Failed to read current: {:?}", e);
            }
        }

        // Read System Status
        match bq.read_status().await {
            Ok(status) => {
                info!("System Status:");
                info!("  CC Ready: {}", status.cc_ready);
                info!("  Overtemperature: {}", status.ovr_temp);
                info!("  Undervoltage (UV): {}", status.uv);
                info!("  Overvoltage (OV): {}", status.ov);
                info!("  Short Circuit Discharge (SCD): {}", status.scd);
                info!("  Overcurrent Discharge (OCD): {}", status.ocd);
                info!("  Cell Undervoltage (CUV): {}", status.cuv);
                info!("  Cell Overvoltage (COV): {}", status.cov);

                // Clear status flags after reading
                // Only clear flags that are set
                let flags_to_clear = (status.cc_ready as u8 * SYS_STAT_CC_READY)
                    | (status.ovr_temp as u8 * SYS_STAT_OVR_TEMP)
                    | (status.uv as u8 * SYS_STAT_UV)
                    | (status.ov as u8 * SYS_STAT_OV)
                    | (status.scd as u8 * SYS_STAT_SCD)
                    | (status.ocd as u8 * SYS_STAT_OCD)
                    | (status.cuv as u8 * SYS_STAT_CUV)
                    | (status.cov as u8 * SYS_STAT_COV);

                if flags_to_clear != 0 {
                    if let Err(e) = bq.clear_status_flags(flags_to_clear).await {
                        error!("Failed to clear status flags: {:?}", e);
                    } else {
                        info!("Cleared status flags: {:#010b}", flags_to_clear);
                    }
                }
            }
            Err(e) => {
                error!("Failed to read system status: {:?}", e);
            }
        }

        info!("----------------------------");

        // Wait for 1 second
        Timer::after(Duration::from_secs(1)).await;
    }
}
