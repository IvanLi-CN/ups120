#![allow(dead_code)]

// Base info
pub const SIG0: u8 = 0x00; // 'S'
pub const SIG1: u8 = 0x01; // 'B'
pub const PROTO_VER: u8 = 0x02; // 0x01
pub const FW_VER_MAJOR: u8 = 0x03;
pub const FW_VER_MINOR: u8 = 0x04;
pub const FW_VER_PATCH: u8 = 0x05;
pub const DEVICE_CAPS: u8 = 0x06;
pub const SYS_STATUS: u8 = 0x07;
// 0x0E/0x0F reserved

// Measurements
pub const VBAT_L: u8 = 0x10; // u16 LE
pub const IBAT_L: u8 = 0x12; // i16 LE
pub const TPACK_L: u8 = 0x14; // i16 LE (c°C)
pub const TMOS_L: u8 = 0x16; // i16 LE (c°C)
pub const VCELL_MAX_L: u8 = 0x18; // u16 LE
pub const VCELL_MIN_L: u8 = 0x1A; // u16 LE
pub const DELTA_CELL_L: u8 = 0x1C; // u16 LE
pub const ADAPTER_PRESENT: u8 = 0x1E; // u8 0/1
pub const CELLS_PRESENT: u8 = 0x1F; // u8 4/5

// Faults
pub const BQ_FAULTS: u8 = 0x20;
pub const CHARGER_FAULTS: u8 = 0x21;
pub const SYSTEM_FAULTS: u8 = 0x22;

// Charging control
pub const CHG_STATUS: u8 = 0x30;
pub const CHG_ENABLE_REQ: u8 = 0x31; // u8 0/1
pub const CHG_CURRENT_LIMIT_L: u8 = 0x32; // u16 LE mA

// Cells (0x50..)
pub const CELL1_L: u8 = 0x50;

// Diagnostics
pub const UPTIME_S_L: u8 = 0x7C; // u16 LE seconds
pub const FRAME_FLAGS: u8 = 0x7E; // bit0 = snapshot fresh

