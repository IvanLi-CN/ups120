use core::ptr::read_volatile;

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
const I2C1_BASE: usize = 0x4000_5400;
const I2C_ISR_OFFSET: usize = 0x18;
const I2C_RXDR_OFFSET: usize = 0x24;

static mut REGISTERS: [u8; REG_SPACE] = [0; REG_SPACE];
static mut REG_PTR: u8 = 0x08;

#[task]
pub async fn task(mut dev: I2c<'static, Blocking, i2c::mode::MultiMaster>) {
    let mut rx = [0u8; MAX_XFER];
    let mut tx = [0u8; MAX_XFER];

    cortex_m::interrupt::free(|_| unsafe {
        let regs = core::ptr::addr_of_mut!(REGISTERS);
        (*regs).fill(0);
        (*regs)[0] = SIG_BYTES[0];
        (*regs)[1] = SIG_BYTES[1];
        (*regs)[2] = PROTOCOL_MAJOR;
        (*regs)[3] = 0; // version minor placeholder
        (*regs)[4] = 0; // last write length
        (*regs)[5] = 0; // last write pointer
        (*regs)[6] = WINDOW_START; // expose current pointer for debugging
        REG_PTR = WINDOW_START;
        info!(
            "i2c1:init sig=[{:02x} {:02x} {:02x}] ptr={:02x}",
            (*regs)[0],
            (*regs)[1],
            (*regs)[2],
            REG_PTR
        );
    });

    loop {
        match dev.listen().await {
            Ok(cmd) => match cmd.kind {
                i2c::SlaveCommandKind::Write => handle_write(&mut dev, &mut rx),
                i2c::SlaveCommandKind::Read => handle_read(&mut dev, &mut tx),
            },
            Err(e) => warn!("i2c1:listen err={:?}", e),
        }
    }
}

fn handle_write(dev: &mut I2c<'static, Blocking, i2c::mode::MultiMaster>, buffer: &mut [u8]) {
    let count = dev.blocking_respond_to_write(buffer).unwrap_or(0);
    if count == 0 {
        info!("i2c1:write-zero raw={=[u8]:02x}", &buffer[..4]);
        let mut drained: Vec<u8, MAX_XFER> = Vec::new();
        cortex_m::interrupt::free(|_| unsafe {
            while read_volatile((I2C1_BASE + I2C_ISR_OFFSET) as *const u32) & (1 << 2) != 0 {
                let byte = read_volatile((I2C1_BASE + I2C_RXDR_OFFSET) as *const u32) as u8;
                let _ = drained.push(byte);
            }

            if let Some(&ptr_byte) = drained.first() {
                REGISTERS[4] = drained.len().saturating_sub(1) as u8;
                REGISTERS[5] = ptr_byte;
                if drained.len() == 1 {
                    REG_PTR = ptr_byte;
                } else {
                    let mut ptr = ptr_byte;
                    for &byte in drained.iter().skip(1) {
                        if (WINDOW_START..=WINDOW_END).contains(&ptr) {
                            REGISTERS[ptr as usize] = byte;
                        }
                        ptr = ptr.wrapping_add(1);
                    }
                    REG_PTR = ptr;
                }
                REGISTERS[6] = REG_PTR;
                enforce_signature();
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
            info!("i2c1:set-ptr (rs) [none]");
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
            info!("i2c1:sig peek -> {=[u8]:02x}", &REGISTERS[0..3]);
            return;
        }
        let mut preview: Vec<u8, 32> = Vec::new();
        for &byte in buffer[1..count].iter() {
            let idx = ptr as usize;
            if (WINDOW_START..=WINDOW_END).contains(&ptr) {
                REGISTERS[idx] = byte;
            }
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
fn enforce_signature() {
    unsafe {
        REGISTERS[0] = SIG_BYTES[0];
        REGISTERS[1] = SIG_BYTES[1];
        REGISTERS[2] = PROTOCOL_MAJOR;
    }
}
