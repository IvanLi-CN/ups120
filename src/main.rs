#![no_std]
#![no_main]
// #![feature(type_alias_impl_trait)] // Required for embassy tasks

extern crate alloc; // Required for global allocator

// use defmt::*; // Removed unused import
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts,
    gpio::{Input, Level, Output, OutputOpenDrain, Pull, Speed},
    i2c::{self, I2c},
    peripherals, // Keep peripherals here
    time::Hertz,
    usb::Driver, // Remove InterruptHandler as it's not directly used here
};
// Import NtcParameters if it's to be configured here
use bq769x0_async_rs::data_types::NtcParameters;

bind_interrupts!(
    struct Irqs {
        USB_LP => embassy_stm32::usb::InterruptHandler<peripherals::USB>;
        I2C1_EV => i2c::EventInterruptHandler<peripherals::I2C1>;
        I2C1_ER => i2c::ErrorInterruptHandler<peripherals::I2C1>;
    }
);
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

// 声明共享模块
// mod bq25730_task; // Commented out - replaced with SC8815
mod bq76920_task;
mod data_types;
// mod ina226_task; // Commented out - not using INA226 for now
mod led_status_task; // Added LED status indication task
mod sc8815_task; // Added SC8815 task
mod shared;
mod usb; // Keep this for our local usb module

// For sharing I2C bus
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;

// Global allocator
use embedded_alloc::LlffHeap as Heap; // Import Heap from embedded_alloc

#[global_allocator]
static HEAP: Heap = Heap::empty();

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

    // 初始化消息队列并获取生产者和消费者
    let (
        measurements_publisher,        // Publisher for AllMeasurements
        _measurements_channel,         // Channel for AllMeasurements, if needed to create more subs
        sc8815_alerts_publisher,       // Publisher for SC8815 Alerts
        sc8815_alerts_channel,         // Channel for SC8815 Alerts
        bq76920_alerts_publisher,      // Publisher for BQ76920 Alerts
        bq76920_alerts_channel,        // Channel for BQ76920 Alerts, used to create subscriber
        sc8815_measurements_publisher, // Publisher for SC8815 Measurements
        sc8815_measurements_channel,   // Channel for SC8815 Measurements, used to create subscriber
        bq76920_measurements_publisher,
        bq76920_measurements_channel, // Channel for BQ76920 Measurements, used to create subscriber
    ) = shared::init_pubsubs();

    let config = embassy_stm32::Config::default();
    let p = embassy_stm32::init(config);

    let usb_driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);
    spawner
        .spawn(usb::usb_task(
            usb_driver,
            measurements_publisher, // This is MeasurementsPublisher<'static, 5>
            sc8815_measurements_channel.subscriber().unwrap(), // Create SC8815 measurements subscriber
            bq76920_measurements_channel.subscriber().unwrap(), // Create BQ76920 measurements subscriber
            sc8815_alerts_channel.subscriber().unwrap(),        // Create SC8815 alerts subscriber
            bq76920_alerts_channel.subscriber().unwrap(),       // Create BQ76920 alerts subscriber
        ))
        .unwrap();

    // Configure I2C1 (PB6 SCL, PB7 SDA) with DMA
    let mut i2c_config = i2c::Config::default();
    i2c_config.scl_pullup = true;
    i2c_config.sda_pullup = true;

    // Create a static Mutex to share the I2C bus between multiple drivers
    static I2C_BUS_MUTEX_CELL: static_cell::StaticCell<
        Mutex<CriticalSectionRawMutex, I2c<'static, embassy_stm32::mode::Async>>,
    > = static_cell::StaticCell::new();
    let i2c_instance = embassy_stm32::i2c::I2c::new(
        p.I2C1,
        p.PA15, // Assuming PA15 is SCL, PB7 is SDA. Please verify.
        p.PB7,
        Irqs,
        p.DMA1_CH3, // DMA for TX
        p.DMA1_CH4, // DMA for RX
        Hertz(100_000),
        i2c_config,
    );

    // Initialize the static Mutex with the I2C instance
    let i2c_bus_mutex = I2C_BUS_MUTEX_CELL.init(Mutex::new(unsafe {
        core::mem::transmute::<
            embassy_stm32::i2c::I2c<'_, embassy_stm32::mode::Async>,
            embassy_stm32::i2c::I2c<'static, embassy_stm32::mode::Async>,
        >(i2c_instance)
    }));

    // BQ76920 I2C address (7-bit)
    let bq76920_address = 0x08;
    // SC8815 I2C address (7-bit)
    let sc8815_address = 0x74; // Default SC8815 address

    // Configure PSTOP GPIO pin for SC8815 (PA0)
    // PSTOP high = charging disabled, PSTOP low = charging enabled
    let pstop_pin = Output::new(p.PA0, Level::High, Speed::Low); // Start with charging disabled

    // Configure LED status pin (PA5) - Open drain, active low
    let led_pin = OutputOpenDrain::new(p.PA5, Level::High, Speed::Low); // Start with LED off (high level)

    // Configure PB9 as input with pull-up for BQ76920 discharge control
    // When PB9 is connected to GND, discharge is enabled; otherwise disabled
    let pb9_discharge_control = Input::new(p.PB9, Pull::Up);

    // Configure PC13 as input with pull-up for SC8815 PSTOP control
    // When PC13 is connected to GND, charging is enabled; otherwise disabled
    let pc13_pstop_control = Input::new(p.PC13, Pull::Up);

    // Configure PA1 as input with pull-up for charging control
    // When PA1 is connected to GND (low level), charging is allowed; otherwise disabled
    let pa1_charge_control = Input::new(p.PA1, Pull::Up);

    // Spawn device tasks
    spawner
        .spawn(sc8815_task::sc8815_task(
            I2cDevice::new(i2c_bus_mutex), // Create a new I2cDevice for the task using the static mutex
            sc8815_address,
            pstop_pin,          // PSTOP control pin
            pc13_pstop_control, // PC13 PSTOP control input pin
            sc8815_alerts_publisher,
            sc8815_measurements_publisher, // This is Sc8815MeasurementsPublisher
            bq76920_measurements_channel.subscriber().unwrap(), // Create BQ76920 measurements subscriber for sc8815_task
        ))
        .unwrap();

    // Commented out BQ25730 and INA226 tasks - replaced with SC8815
    // spawner
    //     .spawn(bq25730_task::bq25730_task(
    //         I2cDevice::new(i2c_bus_mutex),
    //         bq25730_address,
    //         bq25730_alerts_publisher,
    //         bq25730_measurements_publisher,
    //         bq76920_measurements_channel.subscriber().unwrap(),
    //     ))
    //     .unwrap();

    // spawner
    //     .spawn(ina226_task::ina226_task(
    //         I2cDevice::new(i2c_bus_mutex),
    //         ina226_address,
    //         ina226_measurements_publisher,
    //     ))
    //     .unwrap();

    let bq76920_i2c_bus = I2cDevice::new(i2c_bus_mutex); // Create a new I2cDevice for the task using the static mutex

    // Define BQ76920 specific configurations needed for its driver initialization
    let bq76920_sense_resistor_m_ohm: u32 = 3; // Example: 3 mΩ
    // TODO: Determine the actual source of NtcParameters if external thermistors are used.
    let bq76920_ntc_params: Option<NtcParameters> = None;
    // Example for fixed NTC:
    // let bq76920_ntc_params = Some(NtcParameters {
    // b_value: 3950.0,
    // ref_temp_k: 298,
    // ref_resistance_ohm: 10000,
    // });

    spawner
        .spawn(bq76920_task::bq76920_task(
            bq76920_i2c_bus,
            bq76920_address,
            bq76920_sense_resistor_m_ohm, // Pass sense resistor value
            bq76920_ntc_params,           // Pass NTC parameters
            pb9_discharge_control,        // Pass PB9 discharge control pin
            pa1_charge_control,           // Pass PA1 charge control pin
            bq76920_alerts_publisher,
            bq76920_measurements_publisher, // Pass the BQ76920 measurements publisher
        ))
        .unwrap();

    // Spawn LED status indication task
    spawner
        .spawn(led_status_task::led_status_task(
            led_pin,
            sc8815_alerts_channel.subscriber().unwrap(), // Create SC8815 alerts subscriber for LED task
            sc8815_measurements_channel.subscriber().unwrap(), // Create SC8815 measurements subscriber for LED task
            bq76920_alerts_channel.subscriber().unwrap(), // Create BQ76920 alerts subscriber for LED task
        ))
        .unwrap();

    // The main loop is no longer needed here as device logic is in separate tasks
    // This task can now just idle or perform other high-level coordination if needed.

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}
