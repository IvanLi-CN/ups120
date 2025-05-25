#![no_std]
#![no_main]

use bq769x0_async_rs::units::ElectricalResistance;
use defmt::*;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts,
    i2c::{self, I2c},
//use embassy_stm32::i2c::{self, I2c};
    peripherals,
    time::Hertz,
};

bind_interrupts!(
    struct Irqs {
        I2C1_EV => i2c::EventInterruptHandler<peripherals::I2C1>;
        I2C1_ER => i2c::ErrorInterruptHandler<peripherals::I2C1>;
    }
);
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

// Import the BQ769x0 driver crate
use bq769x0_async_rs::registers::*;
use bq769x0_async_rs::{BatteryConfig, Bq769x0, RegisterAccess};

// Import the BQ25730 driver crate
use bq25730_async_rs::Bq25730;

// Import the INA226 driver crate
use ina226::INA226;

// For sharing I2C bus
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;

// Define the I2C interrupt handler


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
    let mut i2c_config = i2c::Config::default();
    i2c_config.scl_pullup = true;
    i2c_config.sda_pullup = true;

    // Create a static Mutex to share the I2C bus between multiple drivers
    static I2C_BUS_MUTEX_CELL: static_cell::StaticCell<
        Mutex<CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async>>,
    > = static_cell::StaticCell::new();
let i2c_instance = embassy_stm32::i2c::I2c::new(
    p.I2C1,         // 1. peri
    p.PA15,          // 2. scl
    p.PB7,          // 3. sda
    Irqs,
    p.DMA1_CH3,
    p.DMA1_CH4,
    Hertz(100_000), // 7. freq
    i2c_config,     // 8. config
);

    info!("I2C1 initialized on PA15/PB7 with DMA.");

    // Initialize the static Mutex with the I2C instance
    let i2c_bus_mutex = I2C_BUS_MUTEX_CELL.init(Mutex::new(unsafe { core::mem::transmute(i2c_instance) }));

    // BQ76920 I2C address (7-bit)
    let bq76920_address = 0x08;
    // BQ25730 I2C address (7-bit)
    let bq25730_address = 0x6B; // Confirmed from bq25730.pdf
// Pass the I2C peripheral instance by value, wrapped in I2cAsynch
let mut bq: Bq769x0<_, bq769x0_async_rs::Enabled> = {
    let i2c_bus = I2cDevice::new(i2c_bus_mutex);
    Bq769x0::new(i2c_bus, bq76920_address)
};
let mut bq25730 = {
    let i2c_bus = I2cDevice::new(i2c_bus_mutex);
    Bq25730::new(i2c_bus, bq25730_address)
};

// INA226 I2C address (7-bit)
let ina226_address = 0x40;
let mut ina226 = {
    let i2c_bus = I2cDevice::new(i2c_bus_mutex);
    INA226::new(i2c_bus, ina226_address)
};

    info!("BQ76920 driver instance created.");

    // --- BQ76920 Initialization Sequence ---

    // Note: Waking from SHIP mode is typically handled by external hardware (TS1 pin).
    // Assuming the chip is already in NORMAL mode or has been woken up.

    // Define battery configuration
    let battery_config = BatteryConfig::default();

    info!("Applying battery configuration...");
    if let Err(e) = bq.set_config(&battery_config).await {
        error!("Failed to apply battery configuration: {:?}", e);
        core::panic!("Failed to apply battery configuration: {:?}", e);
    }
    info!("Battery configuration applied successfully.");

    // Set CC_CFG register to 0x19 for optimal performance
    info!("Setting CC_CFG register to 0x19...");
    if let Err(e) = bq.write_register(Register::CcCfg, 0x19).await {
        error!("Failed to set CC_CFG: {:?}", e);
        core::panic!("Failed to set CC_CFG: {:?}", e);
    }
    info!("CC_CFG set successfully.");

    // 4. Clear initial fault flags
    // Write 0xFF to SYS_STAT to clear all flags
    info!("Clearing initial status flags (writing 0xFF to SYS_STAT)...");
    if let Err(e) = bq.clear_status_flags(0xFF).await {
        error!("Failed to clear status flags: {:?}", e);
        core::panic!("Failed to clear status flags: {:?}", e);
    }
    info!("Initial status flags cleared successfully.");

    info!("BQ76920 initialization complete.");

    // --- Main Loop for Data Acquisition ---
    let sense_resistor = ElectricalResistance::new::<uom::si::electrical_resistance::milliohm>(3.0); // Your sense resistor value in milliOhms

    loop {
        info!("--- Reading BQ76920 Data ---");

        // Ensure CC_EN is enabled in SYS_CTRL2
        info!("Ensuring CC_EN is enabled in SYS_CTRL2...");
        let sys_ctrl2_val = bq.read_register(Register::SysCtrl2).await.unwrap_or(0);
        if let Err(e) = bq.write_register(Register::SysCtrl2, sys_ctrl2_val | SYS_CTRL2_CC_EN).await {
            error!("Failed to enable CC_EN: {:?}", e);
        }
        info!("CC_EN enable attempt complete.");

        // --- Reading INA226 Data ---
        info!("--- Reading INA226 Data ---");

        match ina226.bus_voltage_millivolts().await {
            Ok(voltage) => {
                info!("INA226 Voltage: {} mV", voltage);
            }
            Err(e) => {
                error!("Failed to read INA226 voltage: {:?}", e);
            }
        }


        match ina226.current_amps().await {
            Ok(current) => {
                if let Some(current_amps) = current {
                    let current_ma = current_amps * 1000.0; // Convert to mA
                    info!("INA226 Current: {} mA", current_ma);
                } else {
                    info!("INA226 Current: None");
                }
            }
            Err(e) => {
                error!("Failed to read INA226 current: {:?}", e);
            }
        }

        // --- Reading BQ25730 Data ---
        info!("--- Reading BQ25730 Data ---");

        // Read Charger Status
        match bq25730.read_charger_status().await {
            Ok(status) => {
                info!("BQ25730 Charger Status:");
                info!("  Input Present: {}", status.stat_ac);
                info!("  ICO Complete: {}", status.ico_done);
                info!("  In VAP Mode: {}", status.in_vap);
                info!("  In VINDPM: {}", status.in_vindpm);
                info!("  In IIN_DPM: {}", status.in_iin_dpm);
                info!("  In Fast Charge: {}", status.in_fchrg);
                info!("  In Pre-Charge: {}", status.in_pchrg);
                info!("  In OTG Mode: {}", status.in_otg);
                info!("  Fault ACOV: {}", status.fault_acov);
                info!("  Fault BATOC: {}", status.fault_batoc);
                info!("  Fault ACOC: {}", status.fault_acoc);
                info!("  Fault SYSOVP: {}", status.fault_sysovp);
                info!("  Fault VSYS_UVP: {}", status.fault_vsys_uvp);
                info!(
                    "  Fault Force Converter Off: {}",
                    status.fault_force_converter_off
                );
                info!("  Fault OTG OVP: {}", status.fault_otg_ovp);
                info!("  Fault OTG UVP: {}", status.fault_otg_uvp);
            }
            Err(e) => {
                error!("Failed to read BQ25730 Charger Status: {:?}", e);
            }
        }

        // Read Prochot Status
        match bq25730.read_prochot_status().await {
            Ok(status) => {
                info!("BQ25730 Prochot Status:");
                info!("  VINDPM Triggered: {}", status.stat_vindpm);
                info!("  Comparator Triggered: {}", status.stat_comp);
                info!("  ICRIT Triggered: {}", status.stat_icrit);
                info!("  INOM Triggered: {}", status.stat_inom);
                info!("  IDCHG1 Triggered: {}", status.stat_idchg1);
                info!("  VSYS Triggered: {}", status.stat_vsys);
                info!("  Battery Removal: {}", status.stat_bat_removal);
                info!("  Adapter Removal: {}", status.stat_adpt_removal);
                info!("  VAP Fail: {}", status.stat_vap_fail);
                info!("  Exit VAP: {}", status.stat_exit_vap);
                info!("  IDCHG2 Triggered: {}", status.stat_idchg2);
                info!("  PTM Operation: {}", status.stat_ptm);
            }
            Err(e) => {
                error!("Failed to read BQ25730 Prochot Status: {:?}", e);
            }
        }

        // Read Cell Voltages
        match bq.read_cell_voltages::<5>().await {
            Ok(voltages) => {
                info!("Cell Voltages (mV):");
                // BQ76920 supports up to 5 cells
                for _i in 0..5 {
                    // Get voltage in millivolts as i32 for printing
                    info!(
                        "  Cell {}: {} mV",
                        _i + 1,
                        voltages.voltages[_i].get::<uom::si::electric_potential::millivolt>()
                    );
                }
            }
            Err(e) => {
                error!("Failed to read cell voltages: {:?}", e);
            }
        }

        // Read Pack Voltage
        match bq.read_pack_voltage().await {
            Ok(voltage) => {
                info!(
                    "Pack Voltage: {} mV",
                    voltage.get::<uom::si::electric_potential::millivolt>()
                );
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
                    info!(
                        "  TS1: {} ({} Ohms)",
                        temps
                            .ts1
                            .get::<uom::si::thermodynamic_temperature::kelvin>(),
                        temps
                            .ts1
                            .get::<uom::si::thermodynamic_temperature::kelvin>()
                            as f32
                            / 10.0
                    );
                    // BQ76920 only has TS1
                } else {
                    info!("Temperatures (deci-Celsius):");
                    let ts1_kelvin_integer = temps
                        .ts1
                        .get::<uom::si::thermodynamic_temperature::kelvin>();
                    let ts1_celsius_f32 = ts1_kelvin_integer as f32 - 273.15; // Manually convert kelvin to celsius float

                    info!(
                        "  TS1 (Die Temp): kelvin_value={}, celsius_manual_f32={}",
                        ts1_kelvin_integer, ts1_celsius_f32
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
                let current_ma = bq.convert_raw_cc_to_current_ma(current.raw_cc, sense_resistor);
                info!(
                    "Raw CC: {}, Current: {} mA",
                    current.raw_cc,
                    current_ma.get::<uom::si::electric_current::milliampere>()
                );
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
                    | (status.ovr_temp as u8 * SYS_STAT_OVRD_ALERT)
                    | (status.uv as u8 * SYS_STAT_UV)
                    | (status.ov as u8 * SYS_STAT_OV)
                    | (status.scd as u8 * SYS_STAT_SCD)
                    | (status.ocd as u8 * SYS_STAT_OCD)
                    | (status.cuv as u8 * SYS_STAT_UV)
                    | (status.cov as u8 * SYS_STAT_OV);

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
