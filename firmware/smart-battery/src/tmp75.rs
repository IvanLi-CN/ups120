//! Minimal async driver for TMP75AIDGKR temperature sensor, specialized to the
//! INNER I2C2 bus type used in this firmware.
//!
//! - I²C interface, 7‑bit address (default 0x48 on this board).
//! - 12‑bit resolution (0.0625°C/LSB).
//! - Comparator/window mode (TM=0), active‑low open‑drain ALERT (POL=0).
//! - THIGH/TLOW form a hardware temperature window; this driver provides helpers
//!   to program them in integer °C and to read the current temperature.

use embassy_embedded_hal::shared_bus::{I2cDeviceError, asynch::i2c::I2cDevice};
use embassy_stm32::i2c::I2c;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
// Bring async I2C trait into scope for method resolution (`write`/`write_read`).
use embedded_hal_async::i2c::I2c as _;

/// Default 7‑bit I²C address on this board (A2/A1/A0 tied low).
pub const TMP75_DEFAULT_ADDR: u8 = 0x48;

// Register map (TMP75 family, see TI datasheet).
const REG_TEMP: u8 = 0x00;
const REG_CONFIG: u8 = 0x01;
const REG_TLOW: u8 = 0x02;
const REG_THIGH: u8 = 0x03;

/// Temperature format helper: values are stored as Q4 (°C * 16).
const Q4_PER_C: i16 = 16;

#[inline(always)]
fn c_to_q4(temp_c: i16) -> i16 {
    temp_c.saturating_mul(Q4_PER_C)
}

#[inline(always)]
fn q4_to_c(temp_q4: i16) -> i16 {
    // Truncate towards zero; caller applies any extra rounding if needed.
    temp_q4 / Q4_PER_C
}

#[inline(always)]
fn encode_q4_to_reg_bytes(temp_q4: i16) -> [u8; 2] {
    // TMP75 uses 12‑bit signed; the top 12 bits (15..4) carry the temperature.
    // Represent Q4 directly: bits 15..4 = temp_q4, bits 3..0 = 0.
    let raw: i16 = temp_q4 << 4;
    [(raw >> 8) as u8, (raw & 0xFF) as u8]
}

#[inline(always)]
fn decode_reg_bytes_to_q4(msb: u8, lsb: u8) -> i16 {
    let raw = ((msb as i16) << 8) | (lsb as i16);
    // Sign‑extend and drop the lower 4 bits to get Q4 (°C * 16).
    raw >> 4
}

// Concrete INNER bus device type used throughout this firmware.
pub type Tmp75I2cDev = I2cDevice<
    'static,
    CriticalSectionRawMutex,
    I2c<'static, embassy_stm32::mode::Async, embassy_stm32::i2c::mode::Master>,
>;

/// Convenience alias for the concrete error type yielded by the shared‑bus device.
pub type Tmp75Error = I2cDeviceError<embassy_stm32::i2c::Error>;

/// Async TMP75 driver over the INNER I2C bus.
pub struct Tmp75 {
    i2c: Tmp75I2cDev,
    addr: u8,
}

impl Tmp75 {
    /// Construct a new TMP75 driver around an existing I²C device handle.
    pub fn new(i2c: Tmp75I2cDev, addr: u8) -> Self {
        Self { i2c, addr }
    }

    /// Release the underlying I²C device.
    pub fn release(self) -> Tmp75I2cDev {
        self.i2c
    }

    async fn write_reg(&mut self, reg: u8, data: &[u8]) -> Result<(), Tmp75Error> {
        // Small stack buffer: reg + up to 2 data bytes.
        let mut buf = [0u8; 3];
        let len = 1 + data.len();
        buf[0] = reg;
        buf[1..len].copy_from_slice(data);
        self.i2c.write(self.addr, &buf[..len]).await
    }

    async fn read_reg_2(&mut self, reg: u8) -> Result<[u8; 2], Tmp75Error> {
        let mut buf = [0u8; 2];
        self.i2c.write_read(self.addr, &[reg], &mut buf).await?;
        Ok(buf)
    }

    async fn read_reg_1(&mut self, reg: u8) -> Result<u8, Tmp75Error> {
        let mut buf = [0u8; 1];
        self.i2c.write_read(self.addr, &[reg], &mut buf).await?;
        Ok(buf[0])
    }

    /// Initialize the TMP75 in comparator/window mode with:
    ///
    /// - 12‑bit resolution
    /// - fault queue = 4 consecutive faults
    /// - ALERT active‑low open‑drain
    /// - continuous conversion
    pub async fn init_comparator_mode(&mut self) -> Result<(), Tmp75Error> {
        // OS=0, R1/R0=11 (12‑bit), F1/F0=10 (4 faults), POL=0, TM=0, SD=0.
        const CONFIG_12BIT_COMPARATOR_ACTIVE_LOW: u8 = 0b0111_0000; // 0x70
        self.write_reg(REG_CONFIG, &[CONFIG_12BIT_COMPARATOR_ACTIVE_LOW])
            .await
    }

    /// Program THIGH/TLOW window in integer °C.
    ///
    /// This does not change the config register; call `init_comparator_mode`
    /// once at boot before programming the window.
    pub async fn set_window_celsius(
        &mut self,
        thigh_c: i16,
        tlow_c: i16,
    ) -> Result<(), Tmp75Error> {
        let thigh_bytes = encode_q4_to_reg_bytes(c_to_q4(thigh_c));
        let tlow_bytes = encode_q4_to_reg_bytes(c_to_q4(tlow_c));
        self.write_reg(REG_THIGH, &thigh_bytes).await?;
        self.write_reg(REG_TLOW, &tlow_bytes).await
    }

    /// Read back THIGH/TLOW window as integer °C (for diagnostics).
    pub async fn read_window_celsius(&mut self) -> Result<(i16, i16), Tmp75Error> {
        let thigh_bytes = self.read_reg_2(REG_THIGH).await?;
        let tlow_bytes = self.read_reg_2(REG_TLOW).await?;
        let thigh_q4 = decode_reg_bytes_to_q4(thigh_bytes[0], thigh_bytes[1]);
        let tlow_q4 = decode_reg_bytes_to_q4(tlow_bytes[0], tlow_bytes[1]);
        Ok((q4_to_c(thigh_q4), q4_to_c(tlow_q4)))
    }

    /// Read back the configuration register (for diagnostics).
    pub async fn read_config(&mut self) -> Result<u8, Tmp75Error> {
        self.read_reg_1(REG_CONFIG).await
    }

    /// Read the current temperature in Q4 units (°C * 16).
    pub async fn read_temperature_q4(&mut self) -> Result<i16, Tmp75Error> {
        let bytes = self.read_reg_2(REG_TEMP).await?;
        Ok(decode_reg_bytes_to_q4(bytes[0], bytes[1]))
    }

    /// Read the current temperature in whole °C (truncated towards zero).
    pub async fn read_temperature_c(&mut self) -> Result<i16, Tmp75Error> {
        let t_q4 = self.read_temperature_q4().await?;
        Ok(q4_to_c(t_q4))
    }
}

#[cfg(test)]
mod tests {
    use super::{c_to_q4, decode_reg_bytes_to_q4, encode_q4_to_reg_bytes, q4_to_c};

    #[test]
    fn encode_decode_roundtrip() {
        for t_c in [-40, 0, 25, 45, 55, 100].iter().copied() {
            let q4 = c_to_q4(t_c);
            let bytes = encode_q4_to_reg_bytes(q4);
            let q4_back = decode_reg_bytes_to_q4(bytes[0], bytes[1]);
            // Allow 1 LSB in Q4 as tolerance; integer °C must round‑trip.
            assert!((q4_back - q4).abs() <= 1);
            assert_eq!(q4_to_c(q4_back), t_c);
        }
    }
}
