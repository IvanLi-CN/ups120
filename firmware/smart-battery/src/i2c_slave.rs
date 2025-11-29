#![allow(unsafe_op_in_unsafe_fn)]

use core::ptr::read_volatile;

use crate::data_types::{Bq76920Measurements, Sc8815Measurements};
use crate::sleep_manager;
use crate::{activity::poke_i2c1_activity, charger_control, state_bits};
use defmt::{debug, info, warn};
use embassy_executor::task;
use embassy_stm32::i2c::{self, I2c};
use embassy_stm32::mode::Blocking;
use embassy_stm32::pac;
use embassy_stm32::pac::gpio::vals as gpio_vals;
use embassy_time::{Duration, Instant};
use heapless::Vec;

pub const SLAVE_ADDRESS: u8 = 0x35;

const REG_SPACE: usize = 256;
const MAX_XFER: usize = 64;
const SIG_BYTES: &[u8; 2] = b"SB";
const PROTOCOL_MAJOR: u8 = 0x01;
const WINDOW_START: u8 = 0x08;
const WINDOW_END: u8 = 0x0F;
const STATE_FLAGS_ADDR: u8 = 0x20;
const STATE_BLUE_CODE_ADDR: u8 = 0x22;
const I2C1_BASE: usize = 0x4000_5400;
const I2C_ISR_OFFSET: usize = 0x18;
const I2C_RXDR_OFFSET: usize = 0x24;
pub const CHG_PAUSE_CAUSE_REG: u8 = 0x32;
const PB_SCL_PIN: usize = 6;
const PB_SDA_PIN: usize = 7;
const BUS_RECOVERY_PULSES: usize = 9;
const BUS_RECOVERY_DELAY_LOOPS: u32 = 256;
const BUS_IDLE_SAMPLES: usize = 4;
const RS_PTR_EMPTY_TIMEOUT: Duration = Duration::from_micros(200);
const RS_PTR_PAYLOAD_GAP: Duration = Duration::from_micros(40);
const RS_PTR_TOTAL_TIMEOUT: Duration = Duration::from_micros(1500);

static mut REGISTERS: [u8; REG_SPACE] = [0; REG_SPACE];
static mut REG_PTR: u8 = WINDOW_START;

// Enable verbose I2C diagnostics while we investigate host-side read failures.
const ENABLE_I2C_DIAG: bool = true;

macro_rules! i2c_diag {
    ($($arg:tt)*) => {
        if ENABLE_I2C_DIAG {
            defmt::info!($($arg)*);
        }
    };
}

#[task]
pub async fn task(mut dev: I2c<'static, Blocking, i2c::mode::MultiMaster>) {
    let mut rx = [0u8; MAX_XFER];
    let mut tx = [0u8; MAX_XFER];

    cortex_m::interrupt::free(|_| unsafe {
        initialise_registers();
    });

    loop {
        match dev.listen().await {
            Ok(cmd) => match cmd.kind {
                i2c::SlaveCommandKind::Write => {
                    let _g = sleep_manager::hold("i2c1-write");
                    sleep_manager::bump("i2c1-listen");
                    poke_i2c1_activity();
                    handle_write(&mut dev, &mut rx);
                }
                i2c::SlaveCommandKind::Read => {
                    let _g = sleep_manager::hold("i2c1-read");
                    sleep_manager::bump("i2c1-listen");
                    poke_i2c1_activity();
                    handle_read(&mut dev, &mut tx);
                }
            },
            Err(e) => {
                warn!("i2c1:listen err={:?}", e);
                handle_i2c_fault("listen err");
            }
        }
    }
}

fn handle_write(dev: &mut I2c<'static, Blocking, i2c::mode::MultiMaster>, buffer: &mut [u8]) {
    let count = match dev.blocking_respond_to_write(buffer) {
        Ok(c) => c,
        Err(e) => {
            warn!("i2c1:write resp err={:?}", e);
            handle_i2c_error("write resp err", e);
            return;
        }
    };
    if count == 0 {
        let drained = match drain_repeated_start_bytes() {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!("i2c1:repeated-start drain err={:?}", err);
                soft_reset_i2c1("ptr-drain");
                return;
            }
        };

        cortex_m::interrupt::free(|_| unsafe {
            if let Some(&ptr_byte) = drained.first() {
                REGISTERS[4] = drained.len().saturating_sub(1) as u8;
                REGISTERS[5] = ptr_byte;
                if drained.len() <= 1 {
                    REG_PTR = ptr_byte;
                } else {
                    let mut ptr = ptr_byte;
                    for &byte in drained.iter().skip(1) {
                        let idx = ptr as usize;
                        let stored = match ptr {
                            x if x == charger_control::CHG_CONFIG_REG => {
                                charger_control::write_config(byte)
                            }
                            _ => byte,
                        };
                        REGISTERS[idx] = stored;
                        ptr = ptr.wrapping_add(1);
                    }
                    REG_PTR = ptr;
                }
                REGISTERS[6] = REG_PTR;
                enforce_signature();
            } else {
                REGISTERS[4] = 0;
            }
        });

        if let Some(ptr) = drained.first().copied() {
            if drained.len() == 1 {
                i2c_diag!("i2c1:set-ptr (rs) {:02x}", ptr);
            } else {
                i2c_diag!(
                    "i2c1:write (rs) ptr={:02x} len={} data={=[u8]:02x}",
                    ptr,
                    drained.len() - 1,
                    &drained.as_slice()[1..]
                );
            }
        } else {
            i2c_diag!("i2c1:set-ptr (rs) [missing byte]");
        }
        return;
    }

    if count == 1 && buffer[0] == 0xFF {
        cortex_m::interrupt::free(|_| unsafe {
            let window = &REGISTERS[WINDOW_START as usize..=WINDOW_END as usize];
            i2c_diag!("i2c1:window = {:02x}", window);
        });
        return;
    }

    cortex_m::interrupt::free(|_| unsafe {
        let mut ptr = buffer[0];
        REGISTERS[4] = count.saturating_sub(1) as u8;
        REGISTERS[5] = ptr;
        if count == 1 {
            i2c_diag!("i2c1:set-ptr {:02x}", ptr);
            REG_PTR = ptr;
            REGISTERS[6] = REG_PTR;
            enforce_signature();
            return;
        }
        let mut preview: Vec<u8, 32> = Vec::new();
        for &byte in buffer[1..count].iter() {
            let idx = ptr as usize;
            let stored = match ptr {
                x if x == charger_control::CHG_CONFIG_REG => charger_control::write_config(byte),
                // CHG_PAUSE_CAUSE is read-only; ignore writes but allow pointer advance
                x if x == CHG_PAUSE_CAUSE_REG => state_bits::pause_cause(),
                _ => byte,
            };
            REGISTERS[idx] = stored;
            let _ = preview.push(stored);
            ptr = ptr.wrapping_add(1);
        }
        REG_PTR = ptr;
        REGISTERS[6] = REG_PTR;
        enforce_signature();
        // additional register handling already applied above
        i2c_diag!(
            "i2c1:write ptr={:02x} len={} data={=[u8]:02x}",
            buffer[0],
            count - 1,
            preview.as_slice()
        );
    });
}

fn handle_read(dev: &mut I2c<'static, Blocking, i2c::mode::MultiMaster>, buffer: &mut [u8]) {
    let mut start = 0u8;
    cortex_m::interrupt::free(|_| unsafe {
        REGISTERS[charger_control::CHG_CONFIG_REG as usize] =
            charger_control::config_register_value();
        REGISTERS[CHG_PAUSE_CAUSE_REG as usize] = state_bits::pause_cause();
        enforce_signature();
        start = REG_PTR;
        let mut ptr = start;
        for slot in buffer.iter_mut() {
            *slot = REGISTERS[ptr as usize];
            ptr = ptr.wrapping_add(1);
        }
        REG_PTR = ptr;
        REGISTERS[6] = REG_PTR;
        enforce_signature();
    });

    match dev.blocking_respond_to_read(buffer) {
        Ok(_) => i2c_diag!(
            "i2c1:read ptr={:02x} -> {=[u8]:02x}",
            start,
            &buffer[..16.min(buffer.len())]
        ),
        Err(e) => {
            debug!("i2c1:read err={:?}", e);
            handle_i2c_error("read resp err", e);
        }
    }
}

unsafe fn initialise_registers() {
    REGISTERS = [0; REG_SPACE];
    REGISTERS[0] = SIG_BYTES[0];
    REGISTERS[1] = SIG_BYTES[1];
    REGISTERS[2] = PROTOCOL_MAJOR;
    REGISTERS[3] = 0;
    REGISTERS[4] = 0;
    REGISTERS[5] = WINDOW_START;
    REGISTERS[6] = WINDOW_START;
    REG_PTR = WINDOW_START;
    charger_control::reset_state();
    REGISTERS[charger_control::CHG_CONFIG_REG as usize] = charger_control::config_register_value();
}

unsafe fn enforce_signature() {
    REGISTERS[0] = SIG_BYTES[0];
    REGISTERS[1] = SIG_BYTES[1];
    REGISTERS[2] = PROTOCOL_MAJOR;
}

pub fn write_registers(addr: u8, values: &[u8]) {
    cortex_m::interrupt::free(|_| unsafe {
        for (offset, &byte) in values.iter().enumerate() {
            let index = addr.wrapping_add(offset as u8) as usize;
            if index < REG_SPACE {
                REGISTERS[index] = byte;
            }
        }
    });
}

pub fn update_sc_measurements(meas: &Sc8815Measurements) {
    let adc = &meas.adc_measurements;
    write_u16_le(0x40, adc.vbus_mv);
    write_u16_le(0x42, adc.vbat_mv);
    write_u16_le(0x44, adc.ibus_ma);
    write_u16_le(0x46, adc.ibat_ma);
    write_u16_le(0x48, adc.adin_mv);
}

pub fn update_bq_measurements<const N: usize>(meas: &Bq76920Measurements<N>) {
    let core = &meas.core_measurements;
    let pack_mv = core.total_voltage_mv.clamp(0, u16::MAX as i32) as u16;
    let pack_current = core.current_ma.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    let temps = &core.temperatures;
    let mut hottest = temps.ts1;
    if let Some(ts2) = temps.ts2 {
        hottest = hottest.max(ts2);
    }
    if let Some(ts3) = temps.ts3 {
        hottest = hottest.max(ts3);
    }
    write_u16_le(0x10, pack_mv);
    write_i16_le(0x12, pack_current);
    write_i16_le(0x14, hottest);
    write_i16_le(0x16, temps.ts1);

    // CELLS_PRESENT (0x1F) and per-cell voltages (0x50..)
    let cells_present = core.cell_voltages.voltages.len().min(5) as u8;
    write_registers(0x1F, &[cells_present]);
    for (i, &mv) in core.cell_voltages.voltages.iter().enumerate().take(5) {
        let base = 0x50u8.wrapping_add((i as u8) * 2);
        let mv_clamped = mv.clamp(0, u16::MAX as i32) as u16;
        write_u16_le(base, mv_clamped);
    }
}

pub fn update_state_snapshot(flags: u16, blue_code: u8) {
    cortex_m::interrupt::free(|_| unsafe {
        REGISTERS[STATE_FLAGS_ADDR as usize] = (flags & 0xFF) as u8;
        REGISTERS[(STATE_FLAGS_ADDR + 1) as usize] = (flags >> 8) as u8;
        REGISTERS[STATE_BLUE_CODE_ADDR as usize] = blue_code;
    });
}

fn write_u16_le(addr: u8, value: u16) {
    let lo = (value & 0xFF) as u8;
    let hi = (value >> 8) as u8;
    write_registers(addr, &[lo, hi]);
}

fn write_i16_le(addr: u8, value: i16) {
    write_u16_le(addr, value as u16);
}

#[inline(always)]
fn bus_recovery_delay() {
    for _ in 0..BUS_RECOVERY_DELAY_LOOPS {
        cortex_m::asm::nop();
    }
}

fn recover_i2c1(reason: &str) {
    warn!("i2c1:bus recovery start ({})", reason);
    cortex_m::interrupt::free(|_| {
        let gpiob = pac::GPIOB;
        let i2c1 = pac::I2C1;

        i2c1.cr1().modify(|w| w.set_pe(false));

        gpiob.moder().modify(|w| {
            w.set_moder(PB_SCL_PIN, gpio_vals::Moder::OUTPUT);
            w.set_moder(PB_SDA_PIN, gpio_vals::Moder::OUTPUT);
        });
        gpiob.otyper().modify(|w| {
            w.set_ot(PB_SCL_PIN, gpio_vals::Ot::OPEN_DRAIN);
            w.set_ot(PB_SDA_PIN, gpio_vals::Ot::OPEN_DRAIN);
        });
        gpiob.ospeedr().modify(|w| {
            w.set_ospeedr(PB_SCL_PIN, gpio_vals::Ospeedr::HIGH_SPEED);
            w.set_ospeedr(PB_SDA_PIN, gpio_vals::Ospeedr::HIGH_SPEED);
        });
        gpiob.pupdr().modify(|w| {
            w.set_pupdr(PB_SCL_PIN, gpio_vals::Pupdr::PULL_UP);
            w.set_pupdr(PB_SDA_PIN, gpio_vals::Pupdr::PULL_UP);
        });

        gpiob.bsrr().write(|w| {
            w.set_bs(PB_SCL_PIN, true);
            w.set_bs(PB_SDA_PIN, true);
        });
        bus_recovery_delay();

        for _ in 0..BUS_RECOVERY_PULSES {
            gpiob.bsrr().write(|w| w.set_br(PB_SCL_PIN, true));
            bus_recovery_delay();
            gpiob.bsrr().write(|w| w.set_bs(PB_SCL_PIN, true));
            bus_recovery_delay();
        }

        // STOP: SDA low then high while SCL high
        gpiob.bsrr().write(|w| w.set_br(PB_SDA_PIN, true));
        bus_recovery_delay();
        gpiob.bsrr().write(|w| w.set_bs(PB_SDA_PIN, true));
        bus_recovery_delay();

        // Restore alternate function
        gpiob.moder().modify(|w| {
            w.set_moder(PB_SCL_PIN, gpio_vals::Moder::ALTERNATE);
            w.set_moder(PB_SDA_PIN, gpio_vals::Moder::ALTERNATE);
        });
        gpiob.otyper().modify(|w| {
            w.set_ot(PB_SCL_PIN, gpio_vals::Ot::OPEN_DRAIN);
            w.set_ot(PB_SDA_PIN, gpio_vals::Ot::OPEN_DRAIN);
        });
        gpiob.ospeedr().modify(|w| {
            w.set_ospeedr(PB_SCL_PIN, gpio_vals::Ospeedr::HIGH_SPEED);
            w.set_ospeedr(PB_SDA_PIN, gpio_vals::Ospeedr::HIGH_SPEED);
        });
        gpiob.pupdr().modify(|w| {
            w.set_pupdr(PB_SCL_PIN, gpio_vals::Pupdr::PULL_UP);
            w.set_pupdr(PB_SDA_PIN, gpio_vals::Pupdr::PULL_UP);
        });
        gpiob.afr(0).modify(|w| {
            w.set_afr(PB_SCL_PIN, 1);
            w.set_afr(PB_SDA_PIN, 1);
        });

        i2c1.cr1().modify(|w| w.set_pe(true));
    });
    info!("i2c1:bus recovery complete");
}

fn soft_reset_i2c1(reason: &str) {
    debug!("i2c1:soft reset ({})", reason);
    cortex_m::interrupt::free(|_| {
        let i2c1 = pac::I2C1;
        i2c1.cr1().modify(|w| w.set_pe(false));
        cortex_m::asm::nop();
        i2c1.icr().write(|w| {
            w.set_stopcf(true);
            w.set_nackcf(true);
            w.set_berrcf(true);
            w.set_arlocf(true);
            w.set_ovrcf(true);
        });
        i2c1.cr1().modify(|w| w.set_pe(true));
    });
}

fn handle_i2c_fault(reason: &str) {
    // If the bus is physically stuck, do a full recovery; otherwise prefer a light reset.
    if bus_lines_stuck_low() {
        recover_i2c1(reason);
    } else {
        soft_reset_i2c1(reason);
    }
}

fn handle_i2c_error(reason: &str, error: i2c::Error) {
    // Rate-limit noisy errors to avoid log storms and potential overflow paths
    log_isr_state(reason);
    match error {
        i2c::Error::Timeout
        | i2c::Error::Nack
        | i2c::Error::Overrun
        | i2c::Error::ZeroLengthTransfer => soft_reset_i2c1(reason),
        _ => handle_i2c_fault(reason),
    }
}

fn bus_lines_stuck_low() -> bool {
    for _ in 0..BUS_IDLE_SAMPLES {
        if bus_lines_idle_once() {
            return false;
        }
        cortex_m::asm::nop();
    }
    true
}

fn bus_lines_idle_once() -> bool {
    cortex_m::interrupt::free(|_| {
        let gpiob = pac::GPIOB;
        let idr = gpiob.idr().read();
        matches!(idr.idr(PB_SCL_PIN), gpio_vals::Idr::HIGH)
            && matches!(idr.idr(PB_SDA_PIN), gpio_vals::Idr::HIGH)
    })
}

fn log_isr_state(context: &str) {
    let isr = unsafe { read_volatile((I2C1_BASE + I2C_ISR_OFFSET) as *const u32) };
    debug!("i2c1:isr after {} = 0x{:04x}", context, isr);
}

#[derive(Debug, Copy, Clone, defmt::Format, PartialEq, Eq)]
enum PtrDrainError {
    MissingPointer,
    PayloadTimeout(usize),
    Overflow,
}

fn drain_repeated_start_bytes() -> Result<Vec<u8, MAX_XFER>, PtrDrainError> {
    const RXNE_BIT: u32 = 1 << 2;
    let mut drained: Vec<u8, MAX_XFER> = Vec::new();
    let start = Instant::now();
    let empty_deadline = start + RS_PTR_EMPTY_TIMEOUT;
    let mut payload_deadline = start + RS_PTR_PAYLOAD_GAP;
    let total_deadline = start + RS_PTR_TOTAL_TIMEOUT;

    loop {
        let isr = unsafe { read_volatile((I2C1_BASE + I2C_ISR_OFFSET) as *const u32) };
        if isr & RXNE_BIT != 0 {
            let byte = unsafe { read_volatile((I2C1_BASE + I2C_RXDR_OFFSET) as *const u32) as u8 };
            drained.push(byte).map_err(|_| PtrDrainError::Overflow)?;
            payload_deadline = Instant::now() + RS_PTR_PAYLOAD_GAP;
            continue;
        }

        let now = Instant::now();
        if drained.is_empty() {
            if now >= empty_deadline {
                return Err(PtrDrainError::MissingPointer);
            }
        } else if now >= payload_deadline {
            break;
        }

        if now >= total_deadline {
            return Err(PtrDrainError::PayloadTimeout(drained.len()));
        }

        cortex_m::asm::nop();
    }

    Ok(drained)
}
