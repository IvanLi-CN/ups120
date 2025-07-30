#![no_std]
#![no_main]

extern crate alloc; // Required for global allocator

mod data_types;
mod shared;

use defmt::*;
use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts,
    gpio::{Input, Level, Output, Pull},
    i2c::{self, I2c},
    peripherals,
};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

// For sharing I2C bus
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;

// Global allocator
use embedded_alloc::LlffHeap as Heap;

#[global_allocator]
static HEAP: Heap = Heap::empty();

// Bind interrupts for I2C
bind_interrupts!(struct Irqs {
    I2C0_IRQ => i2c::InterruptHandler<peripherals::I2C0>;
    I2C1_IRQ => i2c::InterruptHandler<peripherals::I2C1>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
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

    // Configure I2C0 (GP4 SDA, GP5 SCL) for device communication
    let i2c_config = i2c::Config::default();

    // Create a static Mutex to share the I2C bus between multiple drivers
    static I2C_BUS_MUTEX_CELL: static_cell::StaticCell<
        Mutex<CriticalSectionRawMutex, I2c<'static, peripherals::I2C0, i2c::Async>>,
    > = static_cell::StaticCell::new();

    let i2c_instance = I2c::new_async(
        p.I2C0,
        p.PIN_1,  // SCL
        p.PIN_0,  // SDA
        Irqs,
        i2c_config,
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

    // Configure GPIO pins for RP2040
    // PSTOP control pin for SC8815 (GP2) - High = charging disabled, Low = charging enabled
    let _pstop_pin = Output::new(p.PIN_2, Level::High);

    // LED status pin (GP25 - onboard LED)
    let mut led_pin = Output::new(p.PIN_25, Level::Low);

    // Discharge control input (GP3) - Low = discharge enabled
    let _discharge_control = Input::new(p.PIN_3, Pull::Up);

    // Charge control input (GP4) - Low = charge allowed
    let _charge_control = Input::new(p.PIN_4, Pull::Up);

    info!("Hardware initialization complete");
    info!("I2C bus configured on GP0(SDA)/GP1(SCL)");
    info!("GPIO pins configured:");
    info!("  - GP2: PSTOP control");
    info!("  - GP3: Discharge control input");
    info!("  - GP4: Charge control input");
    info!("  - GP25: Status LED");

    // Test I2C bus availability
    let _i2c_device = I2cDevice::new(i2c_bus_mutex);
    info!("I2C device created successfully");

    // Basic LED blink to show system is working
    loop {
        info!("System heartbeat - LED on");
        led_pin.set_high();
        Timer::after(Duration::from_millis(100)).await;

        led_pin.set_low();
        Timer::after(Duration::from_millis(900)).await;
    }
}
