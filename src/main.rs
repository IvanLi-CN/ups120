#![no_std]
#![no_main]

extern crate alloc; // Required for global allocator

mod bq76920_task;
mod charger_task;
mod data_types;
mod ina226_task;
mod led_status_task;
mod otg_task;
mod shared;
mod usb;

use defmt::*;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts,
    gpio::{Input, Level, Output, Pull},
    i2c::{self, I2c},
    peripherals,
    usb::{Driver, InterruptHandler},
};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

// Import BQ76920 related types
use bq769x0_async_rs::data_types::NtcParameters;

// For sharing I2C bus
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;

// Global allocator
use embedded_alloc::LlffHeap as Heap;

#[global_allocator]
static HEAP: Heap = Heap::empty();

// Bind interrupts for I2C and USB
bind_interrupts!(struct Irqs {
    I2C0_IRQ => i2c::InterruptHandler<peripherals::I2C0>;
    I2C1_IRQ => i2c::InterruptHandler<peripherals::I2C1>;
    USBCTRL_IRQ => InterruptHandler<peripherals::USB>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Initialize global allocator
    {
        const HEAP_SIZE: usize = 16_384;
        static mut HEAP_MEM: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

        unsafe {
            let heap_start = core::ptr::addr_of_mut!(HEAP_MEM).cast::<u8>();
            HEAP.init(heap_start as usize, HEAP_SIZE)
        }
    }

    let p = embassy_rp::init(Default::default());
    info!("UPS120 RP2040 Firmware Starting...");

    // Configure I2C0 (GP0 SDA, GP1 SCL) for device communication
    // External 4.7kΩ pull-up resistors added for reliable operation
    let mut i2c_config = i2c::Config::default();
    i2c_config.frequency = 100_000; // 100kHz with external pull-ups for optimal performance

    // Create a static Mutex to share the I2C bus between multiple drivers
    static I2C_BUS_MUTEX_CELL: static_cell::StaticCell<
        Mutex<CriticalSectionRawMutex, I2c<'static, peripherals::I2C0, i2c::Async>>,
    > = static_cell::StaticCell::new();

    // Create I2C instance
    // Note: If communication is unstable, add external 4.7kΩ pull-up resistors
    let i2c_instance = I2c::new_async(
        p.I2C0, p.PIN_1, // SCL (GP1)
        p.PIN_0, // SDA (GP0)
        Irqs, i2c_config,
    );

    // Initialize the static Mutex with the I2C instance
    let i2c_bus_mutex = I2C_BUS_MUTEX_CELL.init(Mutex::new(unsafe {
        core::mem::transmute::<
            I2c<'_, peripherals::I2C0, i2c::Async>,
            I2c<'static, peripherals::I2C0, i2c::Async>,
        >(i2c_instance)
    }));

    // Device I2C addresses (7-bit)
    let _bq76920_address = 0x08;
    let _sc8815_address = 0x74;

    // Configure I2C1 (GP2 SDA, GP3 SCL) for OTG device communication
    let mut i2c1_config = i2c::Config::default();
    i2c1_config.frequency = 100_000; // 100kHz

    // Create a static Mutex to share the I2C1 bus for OTG
    static I2C1_BUS_MUTEX_CELL: static_cell::StaticCell<
        Mutex<CriticalSectionRawMutex, I2c<'static, peripherals::I2C1, i2c::Async>>,
    > = static_cell::StaticCell::new();

    // Create I2C1 instance for OTG
    let i2c1_instance = I2c::new_async(
        p.I2C1,
        p.PIN_3, // SCL (GP3)
        p.PIN_2, // SDA (GP2)
        Irqs,
        i2c1_config,
    );

    // Initialize the static Mutex with the I2C1 instance
    let i2c1_bus_mutex = I2C1_BUS_MUTEX_CELL.init(Mutex::new(unsafe {
        core::mem::transmute::<
            I2c<'_, peripherals::I2C1, i2c::Async>,
            I2c<'static, peripherals::I2C1, i2c::Async>,
        >(i2c1_instance)
    }));

    // Configure GPIO pins for RP2040 (updated pin assignments)
    // PSTOP control pin for charging SC8815 (GP4) - High = charging disabled, Low = charging enabled
    let pstop_pin = Output::new(p.PIN_4, Level::High);

    // PSTOP control pin for OTG SC8815 (GP7) - High = OTG disabled, Low = OTG enabled
    let otg_pstop_pin = Output::new(p.PIN_7, Level::High);

    // LED status pin (GP25 - onboard LED)
    let led_pin = Output::new(p.PIN_25, Level::Low);

    // Discharge control input (GP5) - Low = discharge enabled
    let _discharge_control = Input::new(p.PIN_5, Pull::Up);

    // Charge control input (GP6) - Low = charge allowed
    let _charge_control = Input::new(p.PIN_6, Pull::Up);

    info!("Hardware initialization complete");
    info!("I2C0 bus configured on GP0(SDA)/GP1(SCL) for main devices");
    info!("I2C1 bus configured on GP2(SDA)/GP3(SCL) for OTG device");
    info!("GPIO pins configured:");
    info!("  - GP4: PSTOP control (charging)");
    info!("  - GP5: Discharge control input");
    info!("  - GP6: Charge control input");
    info!("  - GP7: PSTOP control (OTG)");
    info!("  - GP25: Status LED");

    // Test I2C bus availability
    let _i2c_device = I2cDevice::new(i2c_bus_mutex);
    info!("I2C device created successfully");

    // Initialize PubSub system
    let (
        measurements_publisher,
        _measurements_channel,
        sc8815_alerts_publisher,
        sc8815_alerts_channel,
        bq76920_alerts_publisher,
        bq76920_alerts_channel,
        sc8815_measurements_publisher,
        sc8815_measurements_channel,
        bq76920_measurements_publisher,
        bq76920_measurements_channel,
        ina226_measurements_publisher,
        ina226_measurements_channel,
        otg_status_publisher,
        otg_status_channel,
    ) = shared::init_pubsubs();

    info!("PubSub system initialized");

    // Create subscribers for LED task
    let sc8815_alerts_subscriber = sc8815_alerts_channel.subscriber().unwrap();
    let sc8815_measurements_subscriber = sc8815_measurements_channel.subscriber().unwrap();
    let bq76920_alerts_subscriber = bq76920_alerts_channel.subscriber().unwrap();
    let otg_status_subscriber_for_led = otg_status_channel.subscriber().unwrap();

    // Spawn LED status task
    spawner
        .spawn(led_status_task::led_status_task(
            led_pin,
            sc8815_alerts_subscriber,
            sc8815_measurements_subscriber,
            bq76920_alerts_subscriber,
            otg_status_subscriber_for_led,
        ))
        .unwrap();

    info!("LED status task spawned");

    // Create BQ76920 I2C device
    let bq76920_i2c_device = I2cDevice::new(i2c_bus_mutex);

    // BQ76920 configuration parameters
    let bq76920_address = 0x08; // 7-bit I2C address
    let sense_resistor_m_ohm = 3; // 3mΩ sense resistor (corrected to match actual hardware)
    let ntc_params: Option<NtcParameters> = None; // No NTC parameters for now

    // Spawn BQ76920 task
    spawner
        .spawn(bq76920_task::bq76920_task(
            bq76920_i2c_device,
            bq76920_address,
            sense_resistor_m_ohm,
            ntc_params,
            _discharge_control,
            _charge_control,
            bq76920_alerts_publisher,
            bq76920_measurements_publisher,
        ))
        .unwrap();

    info!("BQ76920 task spawned");

    // Create SC8815 I2C device
    let sc8815_i2c_device = I2cDevice::new(i2c_bus_mutex);

    // SC8815 configuration parameters
    let sc8815_address = 0x74; // 7-bit I2C address

    // Create BQ76920 measurements subscriber for charger task
    let bq76920_measurements_subscriber_for_charger =
        bq76920_measurements_channel.subscriber().unwrap();

    // Spawn charger task
    spawner
        .spawn(charger_task::charger_task(
            sc8815_i2c_device,
            sc8815_address,
            pstop_pin,
            sc8815_alerts_publisher,
            sc8815_measurements_publisher,
            bq76920_measurements_subscriber_for_charger,
        ))
        .unwrap();

    info!("Charger task spawned");

    // Create INA226 I2C device
    let ina226_i2c_device = I2cDevice::new(i2c_bus_mutex);

    // INA226 configuration parameters
    let ina226_address = 0x40; // 7-bit I2C address (default for INA226)

    // Spawn INA226 task
    spawner
        .spawn(ina226_task::ina226_task(
            ina226_i2c_device,
            ina226_address,
            ina226_measurements_publisher,
        ))
        .unwrap();

    info!("INA226 task spawned");

    // Create OTG I2C device
    let otg_i2c_device = I2cDevice::new(i2c1_bus_mutex);

    // OTG configuration
    let otg_config = crate::data_types::OtgConfiguration::default();
    let otg_address = 0x74; // SC8815 default address

    // Create INA226 measurements subscriber for OTG task
    let ina226_measurements_subscriber_for_otg = ina226_measurements_channel.subscriber().unwrap();

    // Spawn OTG task
    spawner
        .spawn(otg_task::otg_task(
            otg_i2c_device,
            otg_address,
            otg_config,
            ina226_measurements_subscriber_for_otg,
            otg_status_publisher,
            otg_pstop_pin,
        ))
        .unwrap();

    info!("OTG task spawned");

    // Create USB driver
    let usb_driver = Driver::new(p.USB, Irqs);

    // Create subscribers for USB task
    let ina226_measurements_subscriber_for_usb = ina226_measurements_channel.subscriber().unwrap();
    let sc8815_measurements_subscriber_for_usb = sc8815_measurements_channel.subscriber().unwrap();
    let bq76920_measurements_subscriber_for_usb =
        bq76920_measurements_channel.subscriber().unwrap();
    let sc8815_alerts_subscriber_for_usb = sc8815_alerts_channel.subscriber().unwrap();
    let bq76920_alerts_subscriber_for_usb = bq76920_alerts_channel.subscriber().unwrap();
    let otg_status_subscriber_for_usb = otg_status_channel.subscriber().unwrap();

    // Spawn USB task
    spawner
        .spawn(usb::usb_task(
            usb_driver,
            measurements_publisher,
            ina226_measurements_subscriber_for_usb,
            sc8815_measurements_subscriber_for_usb,
            bq76920_measurements_subscriber_for_usb,
            sc8815_alerts_subscriber_for_usb,
            bq76920_alerts_subscriber_for_usb,
            otg_status_subscriber_for_usb,
        ))
        .unwrap();

    info!("USB task spawned");

    // Main loop - just keep the system running
    loop {
        info!("System heartbeat");
        Timer::after(Duration::from_millis(5000)).await;
    }
}
