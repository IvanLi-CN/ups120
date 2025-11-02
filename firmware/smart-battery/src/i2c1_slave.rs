use core::cell::UnsafeCell;

use defmt::{debug, warn};
use embassy_executor::task;
use embassy_stm32::i2c::{self, I2c};
use embassy_stm32::mode::Blocking;

pub const SLAVE_ADDRESS: u8 = 0x35;
const REG_SPACE: usize = 256;
const MAX_XFER: usize = 64;

static REGISTERS: UnsafeCell<[u8; REG_SPACE]> = UnsafeCell::new([0; REG_SPACE]);
static REG_PTR: UnsafeCell<u8> = UnsafeCell::new(0x08);

#[task]
pub async fn task(mut dev: I2c<'static, Blocking, i2c::mode::MultiMaster>) {
    let mut rx = [0u8; MAX_XFER];
    let mut tx = [0u8; MAX_XFER];

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
    let received = dev.blocking_respond_to_write(buffer).unwrap_or(0);
    if received == 0 {
        return;
    }

    unsafe {
        let regs = &mut *REGISTERS.get();
        let ptr_ref = &mut *REG_PTR.get();
        let mut ptr = buffer[0];
        regs[4] = received.saturating_sub(1) as u8;
        regs[5] = ptr;
        for &byte in buffer[1..received].iter() {
            regs[ptr as usize] = byte;
            ptr = ptr.wrapping_add(1);
        }
        *ptr_ref = ptr;
    }
}

fn handle_read(dev: &mut I2c<'static, Blocking, i2c::mode::MultiMaster>, buffer: &mut [u8]) {
    unsafe {
        let regs = &mut *REGISTERS.get();
        let ptr_ref = &mut *REG_PTR.get();
        let mut ptr = *ptr_ref;
        for slot in buffer.iter_mut() {
            *slot = regs[ptr as usize];
            ptr = ptr.wrapping_add(1);
        }
        *ptr_ref = ptr;
        if let Err(e) = dev.blocking_respond_to_read(&buffer[..]) {
            debug!("i2c1:read err={:?}", e);
        }
    }
}
