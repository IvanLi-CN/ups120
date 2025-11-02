use defmt::debug;
use embedded_hal::i2c::I2c;

// Minimal TCA6408A helper (avoids external crate dependency)
// Registers: 0x00=Input, 0x01=Output, 0x02=Polarity, 0x03=Configuration
const DEFAULT_ADDRESS: u8 = 0x20; // A2..A0=0
#[repr(u8)]
#[derive(Copy, Clone)]
enum Register {
    Input = 0x00,
    Output = 0x01,
    Polarity = 0x02,
    Configuration = 0x03,
}

const PORT_IN_PG: u8 = 0;
const PORT_CE: u8 = 1;
const PORT_PSTOP: u8 = 2;
const PORT_ALERT: u8 = 3;

fn write_reg<I2C, E>(i2c: &mut I2C, reg: Register, value: u8) -> Result<(), E>
where
    I2C: I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    let buf = [reg as u8, value];
    i2c.write(DEFAULT_ADDRESS, &buf)
}

fn read_reg<I2C, E>(i2c: &mut I2C, reg: Register) -> Result<u8, E>
where
    I2C: I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    let mut byte = [0u8; 1];
    i2c.write_read(DEFAULT_ADDRESS, &[reg as u8], &mut byte)?;
    Ok(byte[0])
}

fn update_register<I2C, E>(i2c: &mut I2C, reg: Register, f: impl FnOnce(u8) -> u8) -> Result<u8, E>
where
    I2C: I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    let cur = read_reg(i2c, reg)?;
    let newv = f(cur);
    write_reg(i2c, reg, newv)?;
    Ok(newv)
}

pub fn init<I2C, E>(i2c: &mut I2C) -> Result<(), E>
where
    I2C: I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    write_reg(i2c, Register::Polarity, 0x00)?;

    let cfg_inputs = (1 << PORT_IN_PG) | (1 << PORT_ALERT) | 0b1111_0000;
    let cfg_outputs_clear = (1 << PORT_CE) | (1 << PORT_PSTOP);
    let _ = update_register(i2c, Register::Configuration, |mut cfg| {
        cfg |= cfg_inputs;
        cfg &= !cfg_outputs_clear;
        cfg
    })?;

    let safe_mask_set = (1 << PORT_CE) | (1 << PORT_PSTOP);
    // Clear outputs to a safe state: CE=0, PSTOP=0 (masking ones clear bits)
    set_outputs(i2c, safe_mask_set, 0x00)?;

    let cfg = read_reg(i2c, Register::Configuration)?;
    let out = read_reg(i2c, Register::Output)?;
    debug!("tca6408.init cfg=0x{:02X} out=0x{:02X}", cfg, out);
    Ok(())
}

pub fn set_sc_ce<I2C, E>(i2c: &mut I2C, enable: bool) -> Result<(), E>
where
    I2C: I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    if enable {
        set_outputs(i2c, 0x00, 1 << PORT_CE)?;
    } else {
        set_outputs(i2c, 1 << PORT_CE, 0x00)?;
    }
    Ok(())
}

pub fn set_sc_pstop<I2C, E>(i2c: &mut I2C, stop: bool) -> Result<(), E>
where
    I2C: I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    if stop {
        set_outputs(i2c, 1 << PORT_PSTOP, 0x00)?;
    } else {
        set_outputs(i2c, 0x00, 1 << PORT_PSTOP)?;
    }
    Ok(())
}

pub fn read_in_pg<I2C, E>(i2c: &mut I2C) -> Result<bool, E>
where
    I2C: I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    Ok((read_reg(i2c, Register::Input)? & (1 << PORT_IN_PG)) != 0)
}

pub fn read_alert<I2C, E>(i2c: &mut I2C) -> Result<bool, E>
where
    I2C: I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    Ok((read_reg(i2c, Register::Input)? & (1 << PORT_ALERT)) == 0)
}

fn set_outputs<I2C, E>(i2c: &mut I2C, mask_set: u8, mask_clear: u8) -> Result<u8, E>
where
    I2C: I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    update_register(i2c, Register::Output, |mut out| {
        out |= mask_set;
        out &= !mask_clear;
        out
    })
}
