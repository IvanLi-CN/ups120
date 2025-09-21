/// CRC mode marker trait
pub trait CrcMode {}

/// CRC disabled (not used by the real device; provided for API parity)
pub struct Disabled;
impl CrcMode for Disabled {}

/// CRC enabled (TI interleaved per-byte)
pub struct Enabled;
impl CrcMode for Enabled {}

/// Calculate CRC-8 over given bytes, polynomial 0x07, init 0x00.
pub fn calculate_crc(data: &[u8]) -> u8 {
    let mut crc: u8 = 0;
    for &byte in data {
        let mut b = byte ^ crc;
        for _ in 0..8 {
            let msb = b & 0x80;
            b <<= 1;
            if msb != 0 { b ^= 0x07; }
        }
        crc = b;
    }
    crc
}

