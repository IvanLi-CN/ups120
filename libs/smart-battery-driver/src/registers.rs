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
// 0x14..0x17 reserved in current protocol; all temperatures are exposed via
// the compact int8 °C window starting at 0x40.
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
pub const CHG_CONTROL: u8 = 0x31; // bit0: auto, bit1: manual enable, bits[3:2]: speed tier

// Temperature/status window (read-only, int8 in °C):
//   0x40 T_PACK_C, 0x41 T_CHG_C, 0x42..0x45 T_NTC0..3_C, 0x46 T_BQ_INT_C, 0x47 T_MCU_C.
pub const TEMP_WINDOW_BASE: u8 = 0x40;
pub const TEMP_WINDOW_LEN: usize = 8;
// Smart-battery TEMP_STATUS bitfield (0x23) – see firmware docs for bit layout.
pub const TEMP_STATUS: u8 = 0x23;

// Cells (0x50..)
pub const CELL1_L: u8 = 0x50;

// Diagnostics
pub const UPTIME_S_L: u8 = 0x7C; // u16 LE seconds
pub const FRAME_FLAGS: u8 = 0x7E; // bit0 = snapshot fresh
