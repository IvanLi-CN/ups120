#![allow(unsafe_op_in_unsafe_fn)]

use core::ptr::read_volatile;

use crate::data_types::{Bq76920Measurements, Sc8815Measurements};
use crate::sleep_manager;
use crate::thermal;
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
// Legacy temperature base (0x14..0x17) kept as a reserved 4-byte window to
// preserve read-length expectations; no meaningful temperature data is written
// there anymore. All temperatures are exposed exclusively via 0x40..0x47.
const TEMP_BASE_ADDR: u8 = 0x14;
// Temperature/status window (see SOFTWARE_DESIGN.md Register Map).
const TEMP_STATUS_ADDR: u8 = 0x23;
const TEMP_WINDOW_BASE_ADDR: u8 = 0x40;
const TEMP_WINDOW_LEN: usize = 8;
const TEMP_INVALID_I8: i8 = i8::MIN;
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

// Verbose I2C diagnostics are useful while bringing up the slave; enable them
// while we investigate host-side NACKs. Once stable, this can be flipped back
// to `false` to reduce RTT traffic. Keep disabled in production builds.
const ENABLE_I2C_DIAG: bool = false;

macro_rules! i2c_diag {
    ($($arg:tt)*) => {
        if ENABLE_I2C_DIAG {
            defmt::debug!($($arg)*);
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
    let mut len: usize = 0;

    cortex_m::interrupt::free(|_| unsafe {
        // Refresh dynamic registers that can change between reads.
        REGISTERS[charger_control::CHG_CONFIG_REG as usize] =
            charger_control::config_register_value();
        REGISTERS[CHG_PAUSE_CAUSE_REG as usize] = state_bits::pause_cause();
        enforce_signature();

        start = REG_PTR;

        // Host-side protocol only ever issues fixed-size reads from specific
        // starting addresses. If we always respond with MAX_XFER bytes here,
        // the STM32 I2C peripheral sees an early NACK (master finished early)
        // and embassy_stm32 reports Error::Nack. That in turn makes the ESP32
        // see NACK on every smart-battery read. To avoid this, bound the
        // response length to what the ESP32 actually requests for each register.
        len = match start {
            // One-shot validation window: 16 bytes from 0x00.
            0x00 => 16,
            // One-shot validation tail: 4 bytes from WINDOW_END-1 (0x0E).
            x if x == WINDOW_END.wrapping_sub(1) => 4,
            // Pack voltage and current (u16 LE).
            0x10 | 0x12 => 2,
            // Pack / charger temperatures (two i16 LE).
            TEMP_BASE_ADDR => 4,
            // Extended temperature window: 8×int8 °C from 0x40..0x47.
            TEMP_WINDOW_BASE_ADDR => TEMP_WINDOW_LEN,
            // Single-byte status / flags / pause-cause / cells-present.
            0x1F => 1,
            STATE_FLAGS_ADDR => 1,
            x if x == STATE_FLAGS_ADDR.wrapping_add(1) => 1,
            CHG_PAUSE_CAUSE_REG => 1,
            0x30..=0x32 => 1,
            // Per-cell voltages: host reads one byte at a time.
            0x50..=0x5F => 1,
            // Default: conservative single-byte response.
            _ => 1,
        };

        let buf_len = buffer.len();
        if buf_len == 0 {
            len = 0;
            return;
        }
        let used = len.min(buf_len);

        let mut ptr = start;
        for slot in buffer[..used].iter_mut() {
            *slot = REGISTERS[ptr as usize];
            ptr = ptr.wrapping_add(1);
        }
        REG_PTR = ptr;
        REGISTERS[6] = REG_PTR;
        enforce_signature();

        len = used;
    });

    if len == 0 {
        // Nothing to send; just ignore this read gracefully.
        return;
    }

    let slice = &buffer[..len];
    match dev.blocking_respond_to_read(slice) {
        Ok(_) => i2c_diag!(
            "i2c1:read ptr={:02x} -> {=[u8]:02x}",
            start,
            &slice[..16.min(slice.len())]
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
    // For the legacy 0x14..0x17 temperature window that some hosts may still
    // poll, seed the two i16 (0.01 °C) slots with the explicit INVALID
    // sentinel instead of leaving them at 0 °C. This makes any stale use of
    // the old registers fail loudly rather than silently disabling thermal
    // protections.
    let invalid_0_01c = thermal::TEMP_INVALID_0_01C as u16;
    let lo = (invalid_0_01c & 0xFF) as u8;
    let hi = (invalid_0_01c >> 8) as u8;
    let base = TEMP_BASE_ADDR as usize;
    REGISTERS[base] = lo;
    REGISTERS[base + 1] = hi;
    REGISTERS[base + 2] = lo;
    REGISTERS[base + 3] = hi;
    // Explicitly initialise TEMP_STATUS so the host can rely on a defined
    // bitfield value even before any thermal policy is wired in.
    REGISTERS[TEMP_STATUS_ADDR as usize] = 0;
    // Initialise the extended temperature window (0x40..0x47) to the INVALID
    // sentinel so hosts never see misleading 0 °C placeholders before any
    // thermal data has been sampled and mirrored.
    for offset in 0..TEMP_WINDOW_LEN {
        REGISTERS[TEMP_WINDOW_BASE_ADDR as usize + offset] = TEMP_INVALID_I8 as u8;
    }
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
    let _ = meas;
    // SC8815 ADC measurements were previously mirrored into the 0x40..0x48 window.
    // That window is now reserved for compact int8 °C temperature telemetry, so
    // charger-side ADC values are no longer exposed over I2C here. They remain
    // available to internal tasks via the Sc8815Measurements pub/sub path.
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

    // CELLS_PRESENT (0x1F) and per-cell voltages (0x50..)
    let cells_present = core.cell_voltages.voltages.len().min(5) as u8;
    write_registers(0x1F, &[cells_present]);
    for (i, &mv) in core.cell_voltages.voltages.iter().enumerate().take(5) {
        let base = 0x50u8.wrapping_add((i as u8) * 2);
        let mv_clamped = mv.clamp(0, u16::MAX as i32) as u16;
        write_u16_le(base, mv_clamped);
    }

    // Extended temperature window (0x40..0x47, all int8 in °C).
    //
    // Values are sourced from the aggregated thermal snapshot so that NTCs,
    // TMP75, BQ internal temperature and MCU temperature share a single
    // encoding path.
    let snapshot = thermal::snapshot();

    let encode_temp_i8 = |raw_0_01c: i16| -> i8 {
        if raw_0_01c == i16::MIN {
            TEMP_INVALID_I8
        } else {
            // Round 0.01 °C to nearest whole °C without floats.
            let c = if raw_0_01c >= 0 {
                (raw_0_01c + 50) / 100
            } else {
                (raw_0_01c - 50) / 100
            };
            c.clamp(i8::MIN as i16, i8::MAX as i16) as i8
        }
    };

    // Start from the raw snapshot values in 0.01 °C domain.
    let mut t_pack_0_01c = snapshot.t_pack_0_01c;
    let mut t_chg_0_01c = snapshot.t_chg_0_01c;
    let mut t_ntc_0_01c = snapshot.t_ntc_0_01c;
    let mut t_bq_int_0_01c = snapshot.t_bq_int_0_01c;
    let mut t_mcu_0_01c = snapshot.t_mcu_0_01c;

    // If we have at least one valid temperature source (pack), avoid exposing
    // 0x80 sentinels in the steady-state telemetry window by falling back to
    // the pack temperature for any still-invalid fields.
    if t_pack_0_01c != crate::thermal::TEMP_INVALID_0_01C {
        if t_chg_0_01c == crate::thermal::TEMP_INVALID_0_01C {
            t_chg_0_01c = t_pack_0_01c;
        }
        for t in &mut t_ntc_0_01c {
            if *t == crate::thermal::TEMP_INVALID_0_01C {
                *t = t_pack_0_01c;
            }
        }
        if t_bq_int_0_01c == crate::thermal::TEMP_INVALID_0_01C {
            t_bq_int_0_01c = t_pack_0_01c;
        }
        if t_mcu_0_01c == crate::thermal::TEMP_INVALID_0_01C {
            t_mcu_0_01c = t_pack_0_01c;
        }
    }

    let t_pack_i8 = encode_temp_i8(t_pack_0_01c);
    let t_chg_i8 = encode_temp_i8(t_chg_0_01c);
    let t_ntc0_i8 = encode_temp_i8(t_ntc_0_01c[0]);
    let t_ntc1_i8 = encode_temp_i8(t_ntc_0_01c[1]);
    let t_ntc2_i8 = encode_temp_i8(t_ntc_0_01c[2]);
    let t_ntc3_i8 = encode_temp_i8(t_ntc_0_01c[3]);
    let t_bq_int_i8 = encode_temp_i8(t_bq_int_0_01c);
    let t_mcu_i8 = encode_temp_i8(t_mcu_0_01c);

    // Log the aggregated thermal snapshot at info level so we can correlate
    // raw 0.01 °C values with the encoded I2C window on real hardware.
    info!(
        "therm: pack={} chg={} ntc={:?} bq_int={} mcu={}",
        snapshot.t_pack_0_01c,
        snapshot.t_chg_0_01c,
        snapshot.t_ntc_0_01c,
        snapshot.t_bq_int_0_01c,
        snapshot.t_mcu_0_01c
    );

    let temp_window: [u8; TEMP_WINDOW_LEN] = [
        t_pack_i8 as u8,
        t_chg_i8 as u8,
        t_ntc0_i8 as u8,
        t_ntc1_i8 as u8,
        t_ntc2_i8 as u8,
        t_ntc3_i8 as u8,
        t_bq_int_i8 as u8,
        t_mcu_i8 as u8,
    ];
    write_registers(TEMP_WINDOW_BASE_ADDR, &temp_window);
}

pub fn update_state_snapshot(flags: u16, blue_code: u8) {
    cortex_m::interrupt::free(|_| unsafe {
        REGISTERS[STATE_FLAGS_ADDR as usize] = (flags & 0xFF) as u8;
        REGISTERS[(STATE_FLAGS_ADDR + 1) as usize] = (flags >> 8) as u8;
        REGISTERS[STATE_BLUE_CODE_ADDR as usize] = blue_code;
    });
}

/// Update the TEMP_STATUS bitfield (0x23) exposed on the I2C slave.
///
/// This is written from the unified thermal policy and is kept intentionally
/// simple so that it can be called from async tasks without holding any
/// additional locks.
pub fn update_temp_status(bits: u8) {
    cortex_m::interrupt::free(|_| unsafe {
        REGISTERS[TEMP_STATUS_ADDR as usize] = bits;
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
    warn!("i2c1:fault reason={}", reason);
    if bus_lines_stuck_low() {
        recover_i2c1(reason);
    } else {
        soft_reset_i2c1(reason);
    }
}

fn handle_i2c_error(reason: &str, error: i2c::Error) {
    // Rate-limit noisy errors to avoid log storms and potential overflow paths
    log_isr_state(reason);
    warn!("i2c1:error reason={} err={:?}", reason, error);
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
