#![no_std]
#![no_main]
// #![feature(type_alias_impl_trait)] // Required for embassy tasks

extern crate alloc; // Required for global allocator

use defmt::*;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_stm32::{
    bind_interrupts,
    i2c::{self, I2c},
    peripherals, // Keep peripherals here
    time::Hertz,
    usb::Driver, // Remove InterruptHandler as it's not directly used here
};

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
mod data_types;
mod shared;
mod usb; // Keep this for our local usb module
mod bq25730_task;
mod ina226_task;
mod bq76920_task;

// For sharing I2C bus
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;

// Global allocator
use embedded_alloc::LlffHeap as Heap; // Import Heap from embedded_alloc

#[global_allocator]
static HEAP: Heap = Heap::empty();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Starting UPS120 data sharing demo...");

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
        measurements_publisher,
        _measurements_subscriber1, // Mark as unused
        _measurements_subscriber2, // Used by usb_task
        bq25730_alerts_publisher,
        _bq25730_alerts_subscriber, // Mark as unused
        bq76920_alerts_publisher,
        _bq76920_alerts_subscriber, // Mark as unused
        bq25730_measurements_publisher, // Used by bq25730_task
        bq25730_measurements_subscriber, // Used by usb_task
        bq76920_measurements_publisher, // Used by bq76920_task
        bq76920_measurements_subscriber, // Used by usb_task
        ina226_measurements_publisher, // Used by ina226_task
        ina226_measurements_subscriber, // Used by usb_task
    ) = shared::init_pubsubs(); // 初始化消息队列并获取生产者和消费者

    info!("消息队列初始化完成，已获取生产者和消费者。");

    let config = embassy_stm32::Config::default();
    let p = embassy_stm32::init(config);

    info!("STM32 initialized.");

    let usb_driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);
    spawner
        .spawn(usb::usb_task(
            usb_driver,
            measurements_publisher, // usb_task now publishes AllMeasurements
            bq25730_measurements_subscriber, // Pass subscriber for BQ25730 data
            ina226_measurements_subscriber, // Pass subscriber for INA226 data
            bq76920_measurements_subscriber, // Pass subscriber for BQ76920 data
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
        p.PA15,
        p.PB7,
        Irqs,
        p.DMA1_CH3,
        p.DMA1_CH4,
        Hertz(100_000),
        i2c_config,
    );

    info!("I2C1 initialized on PA15/PB7 with DMA.");

    // Initialize the static Mutex with the I2C instance
    let i2c_bus_mutex =
        I2C_BUS_MUTEX_CELL.init(Mutex::new(unsafe { core::mem::transmute::<
            embassy_stm32::i2c::I2c<'_, embassy_stm32::mode::Async>,
            embassy_stm32::i2c::I2c<'static, embassy_stm32::mode::Async>,
        >(i2c_instance) }));

    // BQ76920 I2C address (7-bit)
    let bq76920_address = 0x08;
    // BQ25730 I2C address (7-bit)
    let bq25730_address = 0x6B; // Confirmed from bq25730.pdf
    // INA226 I2C address (7-bit)
    let ina226_address = 0x40;

    // Spawn device tasks
    let _bq25730_i2c_bus = I2cDevice::new(i2c_bus_mutex);
    spawner.spawn(bq25730_task::bq25730_task(
        I2cDevice::new(i2c_bus_mutex), // Create a new I2cDevice for the task using the static mutex
        bq25730_address,
        bq25730_alerts_publisher,
        bq25730_measurements_publisher, // Pass the BQ25730 measurements publisher
    )).unwrap();

    spawner.spawn(ina226_task::ina226_task(
        I2cDevice::new(i2c_bus_mutex), // Create a new I2cDevice for the task using the static mutex
        ina226_address,
        ina226_measurements_publisher,
    )).unwrap();

    let bq76920_i2c_bus = I2cDevice::new(i2c_bus_mutex); // Create a new I2cDevice for the task using the static mutex

    spawner.spawn(bq76920_task::bq76920_task(
        bq76920_i2c_bus,
        bq76920_address,
        bq76920_alerts_publisher,
        bq76920_measurements_publisher, // Pass the BQ76920 measurements publisher
    )).unwrap();

    // The main loop is no longer needed here as device logic is in separate tasks
    // This task can now just idle or perform other high-level coordination if needed.

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}
