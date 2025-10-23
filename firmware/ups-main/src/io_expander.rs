use defmt::debug;
use embedded_hal::i2c::I2c;
use tca6408a_async::{Register, Tca6408a, DEFAULT_ADDRESS};

const PORT_IN_PG: u8 = 0;
const PORT_CE: u8 = 1;
const PORT_PSTOP: u8 = 2;
const PORT_ALERT: u8 = 3;

fn new_dev<'a, I2C>(i2c: &'a mut I2C) -> Tca6408a<&'a mut I2C>
where
    I2C: I2c,
{
    Tca6408a::new(i2c, DEFAULT_ADDRESS)
}

pub fn init<I2C, E>(i2c: &mut I2C) -> Result<(), E>
where
    I2C: I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    let mut dev = new_dev(i2c);
    dev.write_polarity(0x00)?;

    let cfg_inputs = (1 << PORT_IN_PG) | (1 << PORT_ALERT) | 0b1111_0000;
    let cfg_outputs_clear = (1 << PORT_CE) | (1 << PORT_PSTOP);
    let _ = dev.update_register(Register::Configuration, |mut cfg| {
        cfg |= cfg_inputs;
        cfg &= !cfg_outputs_clear;
        cfg
    })?;

    let safe_mask_set = (1 << PORT_CE) | (1 << PORT_PSTOP);
    let _ = dev.set_outputs(safe_mask_set, 0x00)?;

    let cfg = dev.read_configuration()?;
    let out = dev.read_outputs()?;
    debug!("tca6408.init cfg=0x{:02X} out=0x{:02X}", cfg, out);
    Ok(())
}

pub fn set_sc_ce<I2C, E>(i2c: &mut I2C, enable: bool) -> Result<(), E>
where
    I2C: I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    let mut dev = new_dev(i2c);
    if enable {
        let _ = dev.set_outputs(0x00, 1 << PORT_CE)?;
    } else {
        let _ = dev.set_outputs(1 << PORT_CE, 0x00)?;
    }
    Ok(())
}

pub fn set_sc_pstop<I2C, E>(i2c: &mut I2C, stop: bool) -> Result<(), E>
where
    I2C: I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    let mut dev = new_dev(i2c);
    if stop {
        let _ = dev.set_outputs(1 << PORT_PSTOP, 0x00)?;
    } else {
        let _ = dev.set_outputs(0x00, 1 << PORT_PSTOP)?;
    }
    Ok(())
}

pub fn read_in_pg<I2C, E>(i2c: &mut I2C) -> Result<bool, E>
where
    I2C: I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    let mut dev = new_dev(i2c);
    Ok((dev.read_inputs()? & (1 << PORT_IN_PG)) != 0)
}

pub fn read_alert<I2C, E>(i2c: &mut I2C) -> Result<bool, E>
where
    I2C: I2c<Error = E>,
    E: embedded_hal::i2c::Error,
{
    let mut dev = new_dev(i2c);
    Ok((dev.read_inputs()? & (1 << PORT_ALERT)) == 0)
}
