use embedded_hal::delay::DelayNs;
use esp_hal::{delay::Delay, efuse::Efuse, peripherals};

#[allow(improper_ctypes)]
extern "C" {
    fn esp_rom_regi2c_read(block: u8, block_hostid: u8, reg_add: u8) -> u8;
    fn rom_i2c_writeReg(block: u8, block_hostid: u8, reg_add: u8, indata: u8);
}

// Conversion constants derived from ESP-IDF calibration tables for ESP32-S3
// (see: https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-reference/peripherals/temp_sensor.html)
const TSENS_ADC_FACTOR: f32 = 0.4386;
const TSENS_DAC_FACTOR: f32 = 27.88;
const TSENS_SYS_OFFSET: f32 = 20.52;

const TSENS_DAC_TABLE: &[(u8, i8)] = &[(0x05, -2), (0x07, -1), (0x0F, 0), (0x0B, 1), (0x0A, 2)];

#[derive(Clone, Copy, Debug)]
pub struct Reading {
    pub raw: u8,
    pub dac: u8,
    pub base_celsius: f32,
}

pub fn init(delay: &mut Delay) {
    // Enable APB_SARADC peripheral clock so TSENS digital domain is alive.
    let sys = esp_hal::peripherals::SYSTEM::regs();
    sys.perip_clk_en0()
        .modify(|_, w| w.apb_saradc_clk_en().set_bit());

    // Select digital controller for SAR1 and prevent RTC domain from overriding it.
    let sens = esp_hal::peripherals::SENS::regs();
    sens.sar_meas1_mux()
        .modify(|_, w| w.sar1_dig_force().set_bit());
    sens.sar_meas2_mux()
        .modify(|_, w| w.sar2_rtc_force().clear_bit());

    sens.sar_power_xpd_sar()
        .modify(|_, w| unsafe { w.force_xpd_sar().bits(0x3) });
    sens.sar_power_xpd_sar()
        .modify(|_, w| w.sarclk_en().set_bit());

    sens.sar_peri_clk_gate_conf()
        .modify(|_, w| w.tsens_clk_en().set_bit().saradc_clk_en().set_bit());
    sens.sar_tsens_ctrl2().modify(|_, w| unsafe {
        w.sar_tsens_xpd_force()
            .bits(1)
            .sar_tsens_xpd_wait()
            .bits(0x03ff)
    });
    sens.sar_tsens_ctrl()
        .modify(|_, w| w.sar_tsens_power_up().set_bit());
    sens.sar_tsens_ctrl()
        .modify(|_, w| w.sar_tsens_power_up_force().set_bit());
    sens.sar_tsens_ctrl()
        .modify(|_, w| unsafe { w.sar_tsens_clk_div().bits(6) });
    sens.sar_tsens_ctrl()
        .modify(|_, w| w.sar_tsens_dump_out().clear_bit());

    // Enable TSENS analog path (ENT_TSENS) and lock DAC to the standard offset (1 -> -1 LSB).
    unsafe {
        let reg7 = esp_rom_regi2c_read(0x69, 1, 0x07);
        rom_i2c_writeReg(0x69, 1, 0x07, reg7 | (1 << 2));

        let reg6_full = esp_rom_regi2c_read(0x69, 1, 0x06);
        let desired = (reg6_full & !0x0F) | 0x05;
        if reg6_full != desired {
            rom_i2c_writeReg(0x69, 1, 0x06, desired);
        }
    }

    // Allow the sensor bias circuitry to settle before first read.
    delay.delay_us(200u32);

    let ctrl = sens.sar_tsens_ctrl().read();
    let ctrl2 = sens.sar_tsens_ctrl2().read();
    let gate = sens.sar_peri_clk_gate_conf().read();
    let regi7 = unsafe { esp_rom_regi2c_read(0x69, 1, 0x07) };
    let regi6 = unsafe { esp_rom_regi2c_read(0x69, 1, 0x06) };
    defmt::info!(
        "tsens.diag: clk_en={} saradc_clk={} power={} force={} dump={} clk_div={} xpd_force={} xpd_wait={} regi7=0x{=u8:X} regi6=0x{=u8:X}",
        gate.tsens_clk_en().bit() as u8,
        gate.saradc_clk_en().bit() as u8,
        ctrl.sar_tsens_power_up().bit() as u8,
        ctrl.sar_tsens_power_up_force().bit() as u8,
        ctrl.sar_tsens_dump_out().bit() as u8,
        ctrl.sar_tsens_clk_div().bits(),
        ctrl2.sar_tsens_xpd_force().bits(),
        ctrl2.sar_tsens_xpd_wait().bits(),
        regi7,
        regi6,
    );
}

pub fn read_celsius(delay: &mut Delay) -> Reading {
    let regs = esp_hal::peripherals::SENS::regs();

    regs.sar_tsens_ctrl()
        .modify(|_, w| w.sar_tsens_dump_out().set_bit());

    let mut raw: u8 = 0;
    for _ in 0..200 {
        let reg = regs.sar_tsens_ctrl().read();
        if reg.sar_tsens_ready().bit() {
            raw = reg.sar_tsens_out().bits();
            break;
        }
        delay.delay_us(50u32);
    }

    regs.sar_tsens_ctrl()
        .modify(|_, w| w.sar_tsens_dump_out().clear_bit());

    let dac = unsafe { esp_rom_regi2c_read(0x69, 1, 0x06) } & 0x0F;
    let dac_offset = dac_offset_from_reg(dac);
    let raw_f = raw as f32;
    let base = TSENS_ADC_FACTOR * raw_f - TSENS_DAC_FACTOR * (dac_offset as f32) - TSENS_SYS_OFFSET;
    Reading {
        raw,
        dac,
        base_celsius: base,
    }
}

pub fn read_delta_calibration() -> Option<f32> {
    if Efuse::rtc_calib_version() != 1 {
        return None;
    }
    let regs = peripherals::EFUSE::regs();
    let raw_bits = (regs.rd_sys_part1_data4().read().bits() >> 4) & 0x1FF;
    let magnitude = (raw_bits & 0xFF) as i16;
    let sign = (raw_bits & 0x100) != 0;
    let signed = if sign { magnitude } else { -magnitude };
    Some(signed as f32 / 10.0)
}

fn dac_offset_from_reg(reg: u8) -> i8 {
    for &(value, offset) in TSENS_DAC_TABLE {
        if value == (reg & 0x0F) {
            return offset;
        }
    }
    -(reg as i8)
}
