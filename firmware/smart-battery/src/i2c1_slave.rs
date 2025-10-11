use core::sync::atomic::{AtomicBool, AtomicI16, AtomicU16, Ordering};

use defmt::*;
use embassy_futures::select::{Either3, select3};
use embassy_stm32::i2c::{self, I2c};
use embassy_stm32::mode::Blocking;

use crate::activity::poke_i2c1_activity;
use crate::shared::{Bq76920MeasurementsChannelType, Sc8815MeasurementsChannelType};
use crate::state_bits::{self, bits as sbits};

// Mirror storage for future I2C1-slave responses (ISR 无关，传输层在 embassy 从机 API 中实现)
static VBAT_MV: AtomicU16 = AtomicU16::new(0);
static IBAT_MA: AtomicI16 = AtomicI16::new(0);
static CELLS_PRESENT: AtomicU16 = AtomicU16::new(5);
static CHG_ENABLE_REQ: AtomicBool = AtomicBool::new(false);
static CHG_CURRENT_LIMIT_MA: AtomicU16 = AtomicU16::new(0);

// 寄存器指针（主机写入后在读侧自增）。
static REG_PTR: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

// Intentionally no low-level pinmux/enable fallbacks here.
// We rely solely on embassy-stm32 I2C slave API per project policy.

fn crc8(mut c: u8, byte: u8) -> u8 {
    c ^= byte;
    for _ in 0..8 {
        let msb = c & 0x80;
        c <<= 1;
        if msb != 0 {
            c ^= 0x07;
        }
    }
    c
}

fn read_reg(addr: u8) -> u8 {
    let snap = state_bits::snapshot();
    match addr {
        0x00 => b'S',
        0x01 => b'B',
        0x02 => 0x01,
        0x03 => 0, // FW_VER_MAJOR placeholder
        0x04 => 1, // FW_VER_MINOR placeholder
        0x05 => 0, // FW_VER_PATCH placeholder
        0x06 => 0,
        0x07 => {
            let mut bits: u8 = 0;
            if (snap.flags & sbits::AC_PRESENT) != 0 {
                bits |= 1 << 0;
            }
            if (snap.flags & sbits::CHARGING) != 0 {
                bits |= 1 << 1;
            }
            if (snap.flags & sbits::CHG_PAUSED) != 0 {
                bits |= 1 << 2;
            }
            if (snap.flags & sbits::FULL) != 0 {
                bits |= 1 << 3;
            }
            if (snap.flags & sbits::BALANCING) != 0 {
                bits |= 1 << 4;
            }
            if (snap.flags & (sbits::FAULT_BQ | sbits::FAULT_SC)) != 0 {
                bits |= 1 << 5;
            }
            if (snap.flags & sbits::PREPARING) != 0 {
                bits |= 1 << 6;
            }
            bits
        }
        0x10 => (VBAT_MV.load(Ordering::Relaxed) & 0xFF) as u8,
        0x11 => (VBAT_MV.load(Ordering::Relaxed) >> 8) as u8,
        0x12 => (IBAT_MA.load(Ordering::Relaxed) as u16 & 0xFF) as u8,
        0x13 => ((IBAT_MA.load(Ordering::Relaxed) as u16) >> 8) as u8,
        0x1E => ((snap.flags & sbits::AC_PRESENT) != 0) as u8,
        0x1F => CELLS_PRESENT.load(Ordering::Relaxed) as u8,
        0x20 => snap.blue_code,
        0x21 => {
            let mut bits = 0u8;
            if (snap.flags & sbits::ACTIVE_SC) != 0 {
                bits |= 1 << 0;
            }
            if (snap.flags & sbits::ACTIVE_BQ) != 0 {
                bits |= 1 << 1;
            }
            bits
        }
        0x30 => CHG_ENABLE_REQ.load(Ordering::Relaxed) as u8,
        0x31 => CHG_ENABLE_REQ.load(Ordering::Relaxed) as u8,
        0x32 => (CHG_CURRENT_LIMIT_MA.load(Ordering::Relaxed) & 0xFF) as u8,
        0x33 => (CHG_CURRENT_LIMIT_MA.load(Ordering::Relaxed) >> 8) as u8,
        0x7C => (embassy_time::Instant::now().as_secs() as u16 & 0xFF) as u8,
        0x7D => ((embassy_time::Instant::now().as_secs() as u16) >> 8) as u8,
        _ => 0,
    }
}
// 构造由 main 创建；此处不再提供 builder，避免 Peri<'d> 生存期问题。

/// I2C1 从机任务：按 TI/SMBus 风格实现逐字节 PEC（写侧校验，读侧交错返回）。
#[embassy_executor::task]
pub async fn slave_task(mut dev: I2c<'static, Blocking, i2c::mode::MultiMaster>) {
    info!("i2c1:slave");
    let mut rx = [0u8; 64];
    let mut tx = [0u8; 64];

    loop {
        // Mark activity around each I2C1 transaction to help sleep manager
        match dev.listen().await {
            Ok(cmd) => match cmd.kind {
                i2c::SlaveCommandKind::Write => {
                    let _g = crate::sleep_manager::hold("i2c1-write");
                    crate::sleep_manager::bump("i2c1-listen");
                    poke_i2c1_activity();
                    // 接收一帧写入
                    let n = dev.blocking_respond_to_write(&mut rx).unwrap_or(0);
                    if n == 0 {
                        continue;
                    }
                    // 解析：首字节为寄存器指针，后面交替 [DATA, CRC]
                    let mut idx = 0usize;
                    let mut ptr = rx[0];
                    REG_PTR.store(ptr, Ordering::Relaxed);
                    let addr_w = 0x35u8 << 1;
                    while idx + 2 < n {
                        let reg = ptr;
                        let data = rx[idx + 1];
                        let pec = rx[idx + 2];
                        let mut c = 0u8;
                        c = crc8(c, addr_w);
                        c = crc8(c, reg);
                        c = crc8(c, data);
                        if c == pec {
                            match reg {
                                0x31 => CHG_ENABLE_REQ.store(data != 0, Ordering::Relaxed),
                                0x32 => {
                                    let lo = data as u16;
                                    let hi = CHG_CURRENT_LIMIT_MA.load(Ordering::Relaxed) & 0xFF00;
                                    CHG_CURRENT_LIMIT_MA.store(hi | lo, Ordering::Relaxed);
                                }
                                0x33 => {
                                    let hi = (data as u16) << 8;
                                    let lo = CHG_CURRENT_LIMIT_MA.load(Ordering::Relaxed) & 0x00FF;
                                    CHG_CURRENT_LIMIT_MA.store(hi | lo, Ordering::Relaxed);
                                }
                                _ => {}
                            }
                            ptr = ptr.wrapping_add(1);
                            REG_PTR.store(ptr, Ordering::Relaxed);
                        } else {
                            defmt::debug!("pec {} {} {} {}", reg, data, pec, c);
                        }
                        idx += 2;
                    }
                }
                i2c::SlaveCommandKind::Read => {
                    let _g = crate::sleep_manager::hold("i2c1-read");
                    crate::sleep_manager::bump("i2c1-listen");
                    poke_i2c1_activity();
                    // 读取：根据当前 REG_PTR 构造交错 [DATA, CRC]
                    let mut p = REG_PTR.load(Ordering::Relaxed);
                    let mut k = 0usize;
                    let mut first = true;
                    let addr_r = (0x35u8 << 1) | 1;
                    while k + 2 <= tx.len() {
                        let d = read_reg(p);
                        let mut c = 0u8;
                        if first {
                            c = crc8(c, addr_r);
                        }
                        c = crc8(c, d);
                        tx[k] = d;
                        tx[k + 1] = c;
                        p = p.wrapping_add(1);
                        if first {
                            first = false;
                        }
                        k += 2;
                        // 为了避免过长帧，最多准备 32 数据（64字节交错）。
                        if (k / 2) >= 32 {
                            break;
                        }
                    }
                    let _ = dev.blocking_respond_to_read(&tx[..k]);
                    REG_PTR.store(p, Ordering::Relaxed);
                }
            },
            Err(e) => defmt::info!("i2c1:listen {:?}", e),
        }
    }
}

/// 合并后的 I2C1 从机 + 两路镜像状态机任务。
#[embassy_executor::task]
pub async fn slave_mux_task(
    mut dev: I2c<'static, Blocking, i2c::mode::MultiMaster>,
    sc_ch: &'static Sc8815MeasurementsChannelType,
    bq_ch: &'static Bq76920MeasurementsChannelType<5>,
) {
    info!("i2c1:slave");
    let mut rx = [0u8; 64];
    let mut tx = [0u8; 64];
    let mut sc_sub = sc_ch.subscriber().expect("i2c1 sc sub");
    let mut bq_sub = bq_ch.subscriber().expect("i2c1 bq sub");

    loop {
        // 并发等待：I2C 监听 / SC 测量 / BQ 测量
        match select3(
            dev.listen(),
            sc_sub.next_message_pure(),
            bq_sub.next_message_pure(),
        )
        .await
        {
            Either3::First(res) => match res {
                Ok(cmd) => match cmd.kind {
                    i2c::SlaveCommandKind::Write => {
                        let _g = crate::sleep_manager::hold("i2c1-write");
                        crate::sleep_manager::bump("i2c1-listen");
                        poke_i2c1_activity();
                        let n = dev.blocking_respond_to_write(&mut rx).unwrap_or(0);
                        if n == 0 {
                            continue;
                        }
                        let mut idx = 0usize;
                        let mut ptr = rx[0];
                        REG_PTR.store(ptr, Ordering::Relaxed);
                        let addr_w = 0x35u8 << 1;
                        while idx + 2 < n {
                            let reg = ptr;
                            let data = rx[idx + 1];
                            let pec = rx[idx + 2];
                            let mut c = 0u8;
                            c = crc8(c, addr_w);
                            c = crc8(c, reg);
                            c = crc8(c, data);
                            if c == pec {
                                match reg {
                                    0x31 => CHG_ENABLE_REQ.store(data != 0, Ordering::Relaxed),
                                    0x32 => {
                                        let lo = data as u16;
                                        let hi =
                                            CHG_CURRENT_LIMIT_MA.load(Ordering::Relaxed) & 0xFF00;
                                        CHG_CURRENT_LIMIT_MA.store(hi | lo, Ordering::Relaxed);
                                    }
                                    0x33 => {
                                        let hi = (data as u16) << 8;
                                        let lo =
                                            CHG_CURRENT_LIMIT_MA.load(Ordering::Relaxed) & 0x00FF;
                                        CHG_CURRENT_LIMIT_MA.store(hi | lo, Ordering::Relaxed);
                                    }
                                    _ => {}
                                }
                                ptr = ptr.wrapping_add(1);
                                REG_PTR.store(ptr, Ordering::Relaxed);
                            } else {
                                defmt::debug!("pec {} {} {} {}", reg, data, pec, c);
                            }
                            idx += 2;
                        }
                    }
                    i2c::SlaveCommandKind::Read => {
                        let _g = crate::sleep_manager::hold("i2c1-read");
                        crate::sleep_manager::bump("i2c1-listen");
                        poke_i2c1_activity();
                        let mut p = REG_PTR.load(Ordering::Relaxed);
                        let mut k = 0usize;
                        let mut first = true;
                        let addr_r = (0x35u8 << 1) | 1;
                        while k + 2 <= tx.len() {
                            let d = read_reg(p);
                            let mut c = 0u8;
                            if first {
                                c = crc8(c, addr_r);
                            }
                            c = crc8(c, d);
                            tx[k] = d;
                            tx[k + 1] = c;
                            p = p.wrapping_add(1);
                            if first {
                                first = false;
                            }
                            k += 2;
                            if (k / 2) >= 32 {
                                break;
                            }
                        }
                        let _ = dev.blocking_respond_to_read(&tx[..k]);
                        REG_PTR.store(p, Ordering::Relaxed);
                    }
                },
                Err(e) => defmt::info!("i2c1:listen {:?}", e),
            },
            Either3::Second(m) => {
                // SC8815 measurements mirror
                VBAT_MV.store(m.adc_measurements.vbat_mv, Ordering::Relaxed);
                IBAT_MA.store(m.adc_measurements.ibat_ma as i16, Ordering::Relaxed);
            }
            Either3::Third(m) => {
                // BQ76920 measurements mirror
                CELLS_PRESENT.store(5, Ordering::Relaxed);
                VBAT_MV.store(
                    m.core_measurements.total_voltage_mv as u16,
                    Ordering::Relaxed,
                );
            }
        }
    }
}
