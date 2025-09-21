#![no_std]

#[cfg(feature = "defmt")]
extern crate defmt;

#[cfg(not(feature = "async"))]
use embedded_hal::i2c::I2c;
#[cfg(feature = "async")]
use embedded_hal_async::i2c::I2c;

use core::ops::Deref;
use heapless::Vec;

pub mod registers;
pub mod crc;
pub mod errors;

pub use crc::{CrcMode, Disabled, Enabled, calculate_crc};
pub use errors::Error;
pub use registers as regs;

const I2C_ADDR_7BIT: u8 = 0x35;

/// Smart Battery driver (host-side) for the ups120 I2C1-slave protocol.
/// CRC mode is TI-style interleaved per data byte.
pub struct SmartBattery<I2C, M: CrcMode> {
    address: u8,
    i2c: I2C,
    _crc: core::marker::PhantomData<M>,
}

impl<I2C, M: CrcMode> SmartBattery<I2C, M> {
    pub fn with_addr(mut self, addr7: u8) -> Self { self.address = addr7; self }
    pub fn release(self) -> I2C { self.i2c }
}

impl<I2C> SmartBattery<I2C, Enabled> {
    pub fn new(i2c: I2C) -> Self { Self { i2c, address: I2C_ADDR_7BIT, _crc: core::marker::PhantomData } }
}

#[maybe_async_cfg::maybe(
    sync(cfg(not(feature = "async")), self = "RegisterAccess",),
    async(feature = "async", keep_self)
)]
#[allow(async_fn_in_trait)]
pub trait RegisterAccess<E>
where
    Self: Sized,
    E: PartialEq,
{
    type ReadBuffer: Deref<Target = [u8]>;

    async fn read_register(&mut self, reg: u8) -> Result<u8, Error<E>>;
    async fn read_registers(&mut self, reg: u8, len: usize) -> Result<Self::ReadBuffer, Error<E>>;
    async fn write_register(&mut self, reg: u8, value: u8) -> Result<(), Error<E>>;
    async fn write_registers(&mut self, reg: u8, values: &[u8]) -> Result<(), Error<E>>;
}

#[maybe_async_cfg::maybe(
    sync(cfg(not(feature = "async")), self = "SmartBattery",),
    async(feature = "async", keep_self)
)]
impl<I2C, E> RegisterAccess<E> for SmartBattery<I2C, Enabled>
where
    I2C: I2c<Error = E>,
    E: PartialEq,
{
    type ReadBuffer = Vec<u8, 64>;

    async fn read_register(&mut self, reg: u8) -> Result<u8, Error<E>> {
        let buf = self.read_registers(reg, 1).await?;
        Ok(buf[0])
    }

    async fn read_registers(&mut self, reg: u8, len: usize) -> Result<Self::ReadBuffer, Error<E>> {
        // device returns interleaved [D0, CRC0, D1, CRC1, ...]
        let mut rx: Vec<u8, 64> = Vec::new();
        rx.resize(len * 2, 0).map_err(|_| Error::LengthMismatch)?;
        self.i2c
            .write_read(self.address, core::slice::from_ref(&reg), &mut rx)
            .await
            .map_err(Error::I2c)?;

        let mut out: Vec<u8, 64> = Vec::new();
        out.resize(len, 0).map_err(|_| Error::LengthMismatch)?;
        let addr_r = (self.address << 1) | 1;
        for i in 0..len {
            let data = rx[2 * i];
            let crc = rx[2 * i + 1];
            let expected = if i == 0 { calculate_crc(&[addr_r, data]) } else { calculate_crc(&[data]) };
            if crc != expected { return Err(Error::CrcRead { index: i, expected, got: crc }); }
            out[i] = data;
        }
        Ok(out)
    }

    async fn write_register(&mut self, reg: u8, value: u8) -> Result<(), Error<E>> {
        self.write_registers(reg, core::slice::from_ref(&value)).await
    }

    async fn write_registers(&mut self, reg: u8, values: &[u8]) -> Result<(), Error<E>> {
        let mut out: Vec<u8, 64> = Vec::new();
        out.push(reg).map_err(|_| Error::LengthMismatch)?;
        let addr_w = self.address << 1;
        for (i, &d) in values.iter().enumerate() {
            let r = reg.wrapping_add(i as u8);
            out.push(d).map_err(|_| Error::LengthMismatch)?;
            out.push(calculate_crc(&[addr_w, r, d])).map_err(|_| Error::LengthMismatch)?;
        }
        self.i2c.write(self.address, &out).await.map_err(Error::I2c)
    }
}

#[maybe_async_cfg::maybe(
    sync(cfg(not(feature = "async")), self = "SmartBattery",),
    async(feature = "async", keep_self)
)]
impl<I2C, E> SmartBattery<I2C, Enabled>
where
    I2C: I2c<Error = E>,
    E: PartialEq,
{
    /// basic identity check
    pub async fn ping(&mut self) -> Result<bool, Error<E>> {
        let mut two: Vec<u8, 2> = Vec::new();
        // read two bytes starting at SIG0
        let buf = <Self as RegisterAccess<E>>::read_registers(self, regs::SIG0, 2).await?;
        two.extend_from_slice(&buf).ok();
        Ok(two.as_slice() == [b'S', b'B'])
    }

    pub async fn read_vbat_mv(&mut self) -> Result<u16, Error<E>> {
        let v = <Self as RegisterAccess<E>>::read_registers(self, regs::VBAT_L, 2).await?;
        Ok(u16::from_le_bytes([v[0], v[1]]))
    }

    pub async fn read_ibat_ma(&mut self) -> Result<i16, Error<E>> {
        let b = <Self as RegisterAccess<E>>::read_registers(self, regs::IBAT_L, 2).await?;
        Ok(i16::from_le_bytes([b[0], b[1]]))
    }

    pub async fn read_tpack_cc(&mut self) -> Result<i16, Error<E>> {
        let b = <Self as RegisterAccess<E>>::read_registers(self, regs::TPACK_L, 2).await?;
        Ok(i16::from_le_bytes([b[0], b[1]]))
    }

    pub async fn read_status(&mut self) -> Result<(u8, u8, u8, u8), Error<E>> {
        let sys = <Self as RegisterAccess<E>>::read_register(self, regs::SYS_STATUS).await?;
        let bq = <Self as RegisterAccess<E>>::read_register(self, regs::BQ_FAULTS).await?;
        let c = <Self as RegisterAccess<E>>::read_register(self, regs::CHARGER_FAULTS).await?;
        let st = <Self as RegisterAccess<E>>::read_register(self, regs::CHG_STATUS).await?;
        Ok((sys, bq, c, st))
    }

    pub async fn set_charging_enable(&mut self, en: bool) -> Result<(), Error<E>> {
        <Self as RegisterAccess<E>>::write_register(self, regs::CHG_ENABLE_REQ, if en { 1 } else { 0 }).await
    }

    pub async fn set_current_limit_ma(&mut self, ma: u16) -> Result<(), Error<E>> {
        <Self as RegisterAccess<E>>::write_registers(self, regs::CHG_CURRENT_LIMIT_L, &ma.to_le_bytes()).await
    }
}

