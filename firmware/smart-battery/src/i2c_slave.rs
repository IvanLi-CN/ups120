#![allow(unsafe_op_in_unsafe_fn)]

use core::ptr::read_volatile;

use crate::activity::poke_i2c1_activity;
use crate::data_types::{Bq76920Measurements, Sc8815Measurements};
use crate::sleep_manager;
use defmt::{debug, info, warn};
use embassy_executor::task;
use embassy_stm32::i2c::{self, I2c};
use embassy_stm32::mode::Blocking;
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

static mut REGISTERS: [u8; REG_SPACE] = [0; REG_SPACE];
static mut REG_PTR: u8 = WINDOW_START;

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
            Err(e) => warn!("i2c1:listen err={:?}", e),
        }
    }
}

fn handle_write(dev: &mut I2c<'static, Blocking, i2c::mode::MultiMaster>, buffer: &mut [u8]) {
    let count = dev.blocking_respond_to_write(buffer).unwrap_or(0);
    if count == 0 {
        let mut drained: Vec<u8, MAX_XFER> = Vec::new();
        cortex_m::interrupt::free(|_| unsafe {
            const RXNE_BIT: u32 = 1 << 2;
            let mut wait_cycles = 0;

            loop {
                let isr = read_volatile((I2C1_BASE + I2C_ISR_OFFSET) as *const u32);
                if isr & RXNE_BIT != 0 {
                    let byte = read_volatile((I2C1_BASE + I2C_RXDR_OFFSET) as *const u32) as u8;
                    let _ = drained.push(byte);
                    wait_cycles = 0;
                    continue;
                }

                // Allow time for the pointer byte to appear when the master issues a
                // repeated START immediately after writing it.
                if drained.is_empty() && wait_cycles < 32 {
                    wait_cycles += 1;
                    cortex_m::asm::nop();
                    continue;
                }

                // After we have at least one byte, give room for any trailing payload.
                if !drained.is_empty() && wait_cycles < 8 {
                    wait_cycles += 1;
                    cortex_m::asm::nop();
                    continue;
                }

                break;
            }

            if let Some(&ptr_byte) = drained.first() {
                REGISTERS[4] = drained.len().saturating_sub(1) as u8;
                REGISTERS[5] = ptr_byte;
                if drained.len() <= 1 {
                    REG_PTR = ptr_byte;
                } else {
                    let mut ptr = ptr_byte;
                    for &byte in drained.iter().skip(1) {
                        REGISTERS[ptr as usize] = byte;
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
                info!("i2c1:set-ptr (rs) {:02x}", ptr);
            } else {
                info!(
                    "i2c1:write (rs) ptr={:02x} len={} data={=[u8]:02x}",
                    ptr,
                    drained.len() - 1,
                    &drained.as_slice()[1..]
                );
            }
        } else {
            warn!("i2c1:set-ptr (rs) [missing byte]");
        }
        return;
    }

    if count == 1 && buffer[0] == 0xFF {
        cortex_m::interrupt::free(|_| unsafe {
            let window = &REGISTERS[WINDOW_START as usize..=WINDOW_END as usize];
            info!("i2c1:window = {:02x}", window);
        });
        return;
    }

    cortex_m::interrupt::free(|_| unsafe {
        let mut ptr = buffer[0];
        REGISTERS[4] = count.saturating_sub(1) as u8;
        REGISTERS[5] = ptr;
        if count == 1 {
            info!("i2c1:set-ptr {:02x}", ptr);
            REG_PTR = ptr;
            REGISTERS[6] = REG_PTR;
            enforce_signature();
            return;
        }
        let mut preview: Vec<u8, 32> = Vec::new();
        for &byte in buffer[1..count].iter() {
            let idx = ptr as usize;
            REGISTERS[idx] = byte;
            let _ = preview.push(byte);
            ptr = ptr.wrapping_add(1);
        }
        REG_PTR = ptr;
        REGISTERS[6] = REG_PTR;
        enforce_signature();
        info!(
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
        Ok(_) => info!(
            "i2c1:read ptr={:02x} -> {=[u8]:02x}",
            start,
            &buffer[..16.min(buffer.len())]
        ),
        Err(e) => debug!("i2c1:read err={:?}", e),
    }
}

#[inline]
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
}

#[inline]
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
