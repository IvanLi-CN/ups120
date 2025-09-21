#[cfg(feature = "defmt")]
use defmt::Format;

#[derive(Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(Format))]
pub enum Error<E: PartialEq> {
    /// I2C bus error
    I2c(E),
    /// Buffer too large for internal stack scratch area
    LengthMismatch,
    /// CRC mismatch on read
    CrcRead { index: usize, expected: u8, got: u8 },
}

