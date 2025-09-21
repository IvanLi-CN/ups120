use core::cell::Cell;
use core::sync::atomic::{AtomicBool, AtomicI16, AtomicU16, Ordering};

use embassy_stm32::{interrupt, pac};
use pac::i2c::vals as ivals;

use crate::shared::{Bq76920MeasurementsChannelType, Sc8815MeasurementsChannelType};

// Mirror storage for future I2C1-slave responses (kept simple & ISR-friendly)
static VBAT_MV: AtomicU16 = AtomicU16::new(0);
static IBAT_MA: AtomicI16 = AtomicI16::new(0);
static ADAPTER_PRESENT: AtomicBool = AtomicBool::new(false);
static CELLS_PRESENT: AtomicU16 = AtomicU16::new(5);
static CHG_ENABLE_REQ: AtomicBool = AtomicBool::new(false);
static CHG_CURRENT_LIMIT_MA: AtomicU16 = AtomicU16::new(0);

#[derive(Copy, Clone)]
struct I2cState {
    reg_ptr: u8,
    send_crc_next: bool,
    last_tx_data: u8,
    awaiting_ptr: bool,
    awaiting_crc: bool,
    last_rx_data: u8,
    first_read_byte: bool,
}

impl Default for I2cState {
    fn default() -> Self {
        Self {
            reg_ptr: 0,
            send_crc_next: false,
            last_tx_data: 0,
            awaiting_ptr: true,
            awaiting_crc: false,
            last_rx_data: 0,
            first_read_byte: true,
        }
    }
}

const INIT_STATE: I2cState = I2cState {
    reg_ptr: 0,
    send_crc_next: false,
    last_tx_data: 0,
    awaiting_ptr: true,
    awaiting_crc: false,
    last_rx_data: 0,
    first_read_byte: true,
};

static STATE: critical_section::Mutex<Cell<I2cState>> =
    critical_section::Mutex::new(Cell::new(INIT_STATE));

pub fn init_i2c1_slave() {
    // Clocks
    let rcc = pac::RCC;
    // Enable I2C1 clock
    rcc.apb1enr().modify(|w| w.set_i2c1en(true));
    // Reset I2C1
    rcc.apb1rstr().modify(|w| w.set_i2c1rst(true));
    rcc.apb1rstr().modify(|w| w.set_i2c1rst(false));

    // Pins PB6/PB7 are configured via board init; if required we will set AF in a subsequent patch.

    // Configure I2C1 in slave mode
    let i2c = pac::I2C1;
    // Disable peripheral
    i2c.cr1().modify(|w| w.set_pe(false));
    // TIMINGR from .ioc (works for 100/400 kHz as slave)
    i2c.timingr().write(|w| { w.0 = 0x0000_0608 });
    // Own address 1: 7-bit 0x35
    i2c.oar1().write(|w| {
        // OA1[9:0] at bits 0..9, OA1EN at bit 15, 7-bit mode (OA1MODE=0)
        w.0 = (((0x35u16 as u32) << 1) & 0x03FF) | (1u32 << 15);
    });
    // CR1: enable analog filter, clock stretching, wakeup, and slave interrupts
    i2c.cr1().modify(|w| {
        w.set_anfoff(false);
        w.set_nostretch(false);
        w.set_rxie(true);
        w.set_txie(true);
        w.set_addrie(true);
        w.set_stopie(true);
        w.set_nackie(true);
        w.set_errie(true);
    });
    // Enable peripheral
    i2c.cr1().modify(|w| w.set_pe(true));

    // Enable NVIC for I2C1
    unsafe { cortex_m::peripheral::NVIC::unmask(pac::Interrupt::I2C1) };
}

fn crc8(mut c: u8, byte: u8) -> u8 {
    c ^= byte;
    for _ in 0..8 {
        let msb = c & 0x80;
        c <<= 1;
        if msb != 0 {
            c ^= 0x07;
        }
    }
    c
}

fn read_reg(addr: u8) -> u8 {
    match addr {
        0x00 => b'S',
        0x01 => b'B',
        0x02 => 0x01,
        0x03 => 0, // FW_VER_MAJOR placeholder
        0x04 => 1, // FW_VER_MINOR placeholder
        0x05 => 0, // FW_VER_PATCH placeholder
        0x06 => 0,
        0x07 => 0b0001_0111, // basic sys status
        0x10 => (VBAT_MV.load(Ordering::Relaxed) & 0xFF) as u8,
        0x11 => (VBAT_MV.load(Ordering::Relaxed) >> 8) as u8,
        0x12 => (IBAT_MA.load(Ordering::Relaxed) as u16 & 0xFF) as u8,
        0x13 => ((IBAT_MA.load(Ordering::Relaxed) as u16) >> 8) as u8,
        0x1E => ADAPTER_PRESENT.load(Ordering::Relaxed) as u8,
        0x1F => CELLS_PRESENT.load(Ordering::Relaxed) as u8,
        0x30 => CHG_ENABLE_REQ.load(Ordering::Relaxed) as u8,
        0x31 => CHG_ENABLE_REQ.load(Ordering::Relaxed) as u8,
        0x32 => (CHG_CURRENT_LIMIT_MA.load(Ordering::Relaxed) & 0xFF) as u8,
        0x33 => (CHG_CURRENT_LIMIT_MA.load(Ordering::Relaxed) >> 8) as u8,
        0x7C => (embassy_time::Instant::now().as_secs() as u16 & 0xFF) as u8,
        0x7D => ((embassy_time::Instant::now().as_secs() as u16) >> 8) as u8,
        _ => 0,
    }
}

fn on_i2c1_irq() {
    let i2c = pac::I2C1;
    let isr = i2c.isr().read();

    // Address matched
    if isr.addr() {
        i2c.icr().write(|w| w.set_addrcf(true));
        critical_section::with(|cs| {
            let mut st = STATE.borrow(cs).get();
            st.send_crc_next = false;
            st.awaiting_crc = false;
            st.first_read_byte = true;
            let dir_read = isr.dir() == ivals::Dir::READ;
            st.awaiting_ptr = !dir_read; // write -> expect ptr
            STATE.borrow(cs).set(st);
        });
    }

    // RXNE: data from master
    if isr.rxne() {
        let b = i2c.rxdr().read().rxdata();
        critical_section::with(|cs| {
            let mut st = STATE.borrow(cs).get();
            if st.awaiting_ptr {
                st.reg_ptr = b;
                st.awaiting_ptr = false;
                st.awaiting_crc = false;
            } else if !st.awaiting_crc {
                st.last_rx_data = b;
                st.awaiting_crc = true;
            } else {
                // CRC byte received
                let mut c = 0u8;
                c = crc8(c, (0x35u8) << 1); // ADDR_W
                c = crc8(c, st.reg_ptr);
                c = crc8(c, st.last_rx_data);
                if b == c {
                    match st.reg_ptr {
                        0x31 => CHG_ENABLE_REQ.store(st.last_rx_data != 0, Ordering::Relaxed),
                        0x32 => {
                            let lo = st.last_rx_data as u16;
                            let hi = CHG_CURRENT_LIMIT_MA.load(Ordering::Relaxed) & 0xFF00;
                            CHG_CURRENT_LIMIT_MA.store(hi | lo, Ordering::Relaxed)
                        }
                        0x33 => {
                            let hi = (st.last_rx_data as u16) << 8;
                            let lo = CHG_CURRENT_LIMIT_MA.load(Ordering::Relaxed) & 0x00FF;
                            CHG_CURRENT_LIMIT_MA.store(hi | lo, Ordering::Relaxed)
                        }
                        _ => {}
                    }
                    st.reg_ptr = st.reg_ptr.wrapping_add(1);
                }
                st.awaiting_crc = false;
            }
            STATE.borrow(cs).set(st);
        });
    }

    // TXIS: master wants a byte
    if isr.txis() {
        let (send_crc, d, first_read) = critical_section::with(|cs| {
            let st = STATE.borrow(cs).get();
            (st.send_crc_next, read_reg(st.reg_ptr), st.first_read_byte)
        });
        if send_crc {
            let mut c = 0u8;
            if first_read { c = crc8(c, (0x35u8 << 1) | 1); }
            let last = critical_section::with(|cs| STATE.borrow(cs).get().last_tx_data);
            c = crc8(c, last);
            i2c.txdr().write(|w| w.set_txdata(c));
            critical_section::with(|cs| {
                let mut st = STATE.borrow(cs).get();
                st.send_crc_next = false;
                st.first_read_byte = false;
                STATE.borrow(cs).set(st);
            });
        } else {
            i2c.txdr().write(|w| w.set_txdata(d));
            critical_section::with(|cs| {
                let mut st = STATE.borrow(cs).get();
                st.last_tx_data = d;
                st.send_crc_next = true;
                st.reg_ptr = st.reg_ptr.wrapping_add(1);
                STATE.borrow(cs).set(st);
            });
        }
    }

    // STOP
    if isr.stopf() {
        i2c.icr().write(|w| w.set_stopcf(true));
        critical_section::with(|cs| {
            let mut st = STATE.borrow(cs).get();
            st.awaiting_ptr = true;
            st.awaiting_crc = false;
            st.send_crc_next = false;
            st.first_read_byte = true;
            STATE.borrow(cs).set(st);
        });
    }

    // Clear NACK flag if any
    if isr.nackf() {
        i2c.icr().write(|w| w.set_nackcf(true));
    }
}

struct I2c1Handler;
impl interrupt::typelevel::Handler<interrupt::typelevel::I2C1> for I2c1Handler {
    unsafe fn on_interrupt() {
        on_i2c1_irq();
    }
}

embassy_stm32::bind_interrupts!(struct I2c1Irqs { I2C1 => I2c1Handler; });

#[embassy_executor::task]
pub async fn sc_meas_mirror_task(ch: &'static Sc8815MeasurementsChannelType) {
    let mut sub = ch.subscriber().expect("i2c1 sc sub");
    loop {
        let m = sub.next_message_pure().await;
        VBAT_MV.store(m.adc_measurements.vbat_mv as u16, Ordering::Relaxed);
        IBAT_MA.store(m.adc_measurements.ibat_ma as i16, Ordering::Relaxed);
        ADAPTER_PRESENT.store(m.adc_measurements.vbus_mv > 0, Ordering::Relaxed);
    }
}

#[embassy_executor::task]
pub async fn bq_meas_mirror_task(ch: &'static Bq76920MeasurementsChannelType<5>) {
    let mut sub = ch.subscriber().expect("i2c1 bq sub");
    loop {
        let m = sub.next_message_pure().await;
        CELLS_PRESENT.store(5, Ordering::Relaxed);
        VBAT_MV.store(m.core_measurements.total_voltage_mv as u16, Ordering::Relaxed);
    }
}
