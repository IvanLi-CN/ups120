use binrw::{
    binrw, io::{Read, Seek, Write}, BinRead, BinResult, BinWrite
};
use defmt::Format;

use bq769x0_async_rs::data_types::{
    Bq76920Measurements as Bq76920CoreMeasurements, CellVoltages, MosStatus, SystemStatus,
    TemperatureSensorReadings,
};
use bq25730_async_rs::data_types::{AdcMeasurements, ChargerStatus, ProchotStatus};
use uom::si::electric_current::ElectricCurrent;
use uom::si::electric_potential::ElectricPotential;
use uom::si::{electric_current::milliampere, electric_potential::millivolt};

/// BQ25730 测量数据
#[derive(Debug, Copy, Clone, PartialEq, defmt::Format)]
pub struct Bq25730Measurements {
    pub adc_measurements: AdcMeasurements,
    // 添加其他非告警相关的测量数据字段（如果需要）
}

/// BQ25730 安全告警信息
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Bq25730Alerts {
    pub charger_status: ChargerStatus,
    pub prochot_status: ProchotStatus,
}

/// BQ76920 测量数据
#[derive(Debug, Copy, Clone, PartialEq)]
#[binrw]
pub struct Bq76920Measurements<const N: usize> {
    pub core_measurements: Bq76920CoreMeasurements<N>,
}

/// BQ76920 安全告警信息
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Bq76920Alerts {
    pub system_status: SystemStatus,
}

/// 聚合所有设备的测量数据
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct AllMeasurements<const N: usize> {
    pub bq25730: Bq25730Measurements,
    pub bq76920: Bq76920Measurements<N>,
}

impl<const N: usize> Format for AllMeasurements<N> {
    fn format(&self, fmt: defmt::Formatter) {
        defmt::write!(
            fmt,
            "AllMeasurements {{ bq25730: {}, bq76920: {{ cell_voltages: [",
            self.bq25730
        );
        for i in 0..N {
            defmt::write!(
                fmt,
                "{:?}, ",
                self.bq76920.core_measurements.cell_voltages.voltages[i].get::<millivolt>()
            );
        }
        defmt::write!(
            fmt,
            "], temperatures: {{ ts1: {:?}, is_thermistor: {} }}, current: {}, system_status: {{ cc_ready: {}, device_xready: {}, ovrd_alert: {}, uv: {}, ov: {}, scd: {}, ocd: {} }}, mos_status: {{ charge_on: {}, discharge_on: {} }} }} }}",
            self.bq76920
                .core_measurements
                .temperatures
                .ts1
                .get::<uom::si::electric_potential::volt>(),
            self.bq76920.core_measurements.temperatures.is_thermistor,
            self.bq76920.core_measurements.current.get::<milliampere>(),
            self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::CC_READY),
            self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::DEVICE_XREADY),
            self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::OVRD_ALERT),
            self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::UV),
            self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::OV),
            self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::SCD),
            self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::OCD),
            // self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::OVR_TEMP), // Removed OVR_TEMP check
            self.bq76920.core_measurements.mos_status.0.contains(bq769x0_async_rs::registers::SysCtrl2Flags::CHG_ON),
            self.bq76920.core_measurements.mos_status.0.contains(bq769x0_async_rs::registers::SysCtrl2Flags::DSG_ON)
        );
    }
}

// Manual implementation of BinRead and BinWrite for AllMeasurements
impl<const N: usize> BinRead for AllMeasurements<N> {
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> BinResult<Self> {
        // BQ25730 Measurements (u8)
        let bq25730_psys_raw = u8::read_options(reader, endian, args)?;
        let bq25730_vbus_raw = u8::read_options(reader, endian, args)?;
        let bq25730_idchg_raw = u8::read_options(reader, endian, args)?;
        let bq25730_ichg_raw = u8::read_options(reader, endian, args)?;
        let bq25730_cmpin_raw = u8::read_options(reader, endian, args)?;
        let bq25730_iin_raw = u8::read_options(reader, endian, args)?;
        let bq25730_vbat_raw = u8::read_options(reader, endian, args)?;
        let bq25730_vsys_raw = u8::read_options(reader, endian, args)?;

        // Cell Voltages (f32)
        let mut cell_voltages_raw = [0.0f32; N];
        for i in 0..N {
            cell_voltages_raw[i] = f32::read_options(reader, endian, args)?;
        }

        // Temperatures (f32 for ts1, u8 for is_thermistor)
        let temperatures_ts1_raw = f32::read_options(reader, endian, args)?;
        let temperatures_is_thermistor_raw = u8::read_options(reader, endian, args)?;
        let temperatures_is_thermistor = temperatures_is_thermistor_raw != 0;

        // Current (f32)
        let current_raw = f32::read_options(reader, endian, args)?;

        // System Status (u8 for each boolean flag)
        let system_status_raw = u8::read_options(reader, endian, args)?;

        // Mos Status (u8 for each boolean flag)
        let mos_status_charge_on_raw = u8::read_options(reader, endian, args)?;
        let mos_status_discharge_on_raw = u8::read_options(reader, endian, args)?;

        Ok(Self {
            bq25730: Bq25730Measurements {
                adc_measurements: AdcMeasurements {
                    psys: bq25730_async_rs::data_types::AdcPsys::from_u8(
                        bq25730_psys_raw,
                    ),
                    vbus: bq25730_async_rs::data_types::AdcVbus::from_u8(
                        bq25730_vbus_raw,
                    ),
                    idchg: bq25730_async_rs::data_types::AdcIdchg::from_u8(
                        bq25730_idchg_raw,
                    ),
                    ichg: bq25730_async_rs::data_types::AdcIchg::from_u8(
                        bq25730_ichg_raw,
                    ),
                    cmpin: bq25730_async_rs::data_types::AdcCmpin::from_u8(
                        bq25730_cmpin_raw,
                    ),
                    iin: bq25730_async_rs::data_types::AdcIin::from_u8(bq25730_iin_raw, true), // Assuming 5mOhm sense resistor
                    vbat: bq25730_async_rs::data_types::AdcVbat::from_register_value(
                        0, // LSB is not used in from_register_value
                        bq25730_vbat_raw,
                        0, // OFFSET_MV is 0
                    ),
                    vsys: bq25730_async_rs::data_types::AdcVsys::from_register_value(
                        0, // LSB is not used in from_register_value
                        bq25730_vsys_raw,
                        0, // OFFSET_MV is 0
                    ),
                },
            },
            bq76920: Bq76920Measurements {
                core_measurements: Bq76920CoreMeasurements {
                    cell_voltages: {
                        let mut voltages = [ElectricPotential::new::<millivolt>(0.0); N];
                        for i in 0..N {
                            voltages[i] = ElectricPotential::new::<millivolt>(cell_voltages_raw[i]);
                        }
                        CellVoltages { voltages }
                    },
                    temperatures: TemperatureSensorReadings {
                        ts1: ElectricPotential::new::<uom::si::electric_potential::volt>(
                            temperatures_ts1_raw,
                        ),
                        ts2: None, // Assuming only TS1 is serialized for now
                        ts3: None, // Assuming only TS1 is serialized for now
                        is_thermistor: temperatures_is_thermistor,
                    },
                    current: ElectricCurrent::new::<milliampere>(current_raw),
                    system_status: SystemStatus::new(system_status_raw),
                    mos_status: MosStatus::new(mos_status_charge_on_raw | (mos_status_discharge_on_raw << 1)), // Reconstruct SysCtrl2Flags byte
                },
            },
        })
    }
}

impl<const N: usize> BinWrite for AllMeasurements<N> {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> BinResult<()> {
        // BQ25730 Measurements (u16)
        ((self.bq25730.adc_measurements.psys.0 / 12) as u8).write_options(writer, endian, args)?;
        ((self.bq25730.adc_measurements.vbus.0 / 96) as u8).write_options(writer, endian, args)?;
        ((self.bq25730.adc_measurements.idchg.0 / 512) as u8)
            .write_options(writer, endian, args)?;
        ((self.bq25730.adc_measurements.ichg.0 / 128) as u8).write_options(writer, endian, args)?;
        ((self.bq25730.adc_measurements.cmpin.0 / 12) as u8).write_options(writer, endian, args)?;
        ((self.bq25730.adc_measurements.iin.milliamps / 100) as u8).write_options(writer, endian, args)?;
        ((self.bq25730.adc_measurements.vbat.0 / 64) as u8).write_options(writer, endian, args)?;
        ((self.bq25730.adc_measurements.vsys.0 / 64) as u8).write_options(writer, endian, args)?;

        // Cell Voltages (f32)
        for i in 0..N {
            self.bq76920.core_measurements.cell_voltages.voltages[i]
                .get::<millivolt>()
                .write_options(writer, endian, args)?;
        }

        // Temperatures (f32 for ts1, u8 for is_thermistor)
        self.bq76920
            .core_measurements
            .temperatures
            .ts1
            .get::<uom::si::electric_potential::volt>()
            .write_options(writer, endian, args)?;
        (self.bq76920.core_measurements.temperatures.is_thermistor as u8)
            .write_options(writer, endian, args)?;

        // Current (f32)
        self.bq76920
            .core_measurements
            .current
            .get::<milliampere>()
            .write_options(writer, endian, args)?;

        // System Status (u8 for each boolean flag)
        let mut system_status_byte: u8 = 0;
        if self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::CC_READY) {
            system_status_byte |= bq769x0_async_rs::registers::SysStatFlags::CC_READY.bits();
        }
        // if self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::OVR_TEMP) { // Removed OVR_TEMP check
        //     system_status_byte |= bq769x0_async_rs::registers::SysStatFlags::OVR_TEMP.bits();
        // }
        if self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::DEVICE_XREADY) {
            system_status_byte |= bq769x0_async_rs::registers::SysStatFlags::DEVICE_XREADY.bits();
        }
        if self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::OVRD_ALERT) {
            system_status_byte |= bq769x0_async_rs::registers::SysStatFlags::OVRD_ALERT.bits();
        }
        if self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::UV) {
            system_status_byte |= bq769x0_async_rs::registers::SysStatFlags::UV.bits();
        }
        if self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::OV) {
            system_status_byte |= bq769x0_async_rs::registers::SysStatFlags::OV.bits();
        }
        if self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::SCD) {
            system_status_byte |= bq769x0_async_rs::registers::SysStatFlags::SCD.bits();
        }
        if self.bq76920.core_measurements.system_status.0.contains(bq769x0_async_rs::registers::SysStatFlags::OCD) {
            system_status_byte |= bq769x0_async_rs::registers::SysStatFlags::OCD.bits();
        }
        system_status_byte.write_options(writer, endian, args)?;

        // Mos Status (u8 for each boolean flag)
        (self.bq76920.core_measurements.mos_status.0.contains(bq769x0_async_rs::registers::SysCtrl2Flags::CHG_ON) as u8)
            .write_options(writer, endian, args)?;
        (self.bq76920.core_measurements.mos_status.0.contains(bq769x0_async_rs::registers::SysCtrl2Flags::DSG_ON) as u8)
            .write_options(writer, endian, args)?;

        Ok(())
    }
}

impl BinRead for Bq25730Alerts {
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> BinResult<Self> {
        let charger_status = ChargerStatus::read_options(reader, endian, args)?;
        let prochot_status = ProchotStatus::read_options(reader, endian, args)?;
        Ok(Self {
            charger_status,
            prochot_status,
        })
    }
}

impl BinWrite for Bq25730Alerts {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> BinResult<()> {
        self.charger_status.write_options(writer, endian, args)?;
        self.prochot_status.write_options(writer, endian, args)?;
        Ok(())
    }
}

impl BinRead for Bq76920Alerts {
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> BinResult<Self> {
        let system_status_raw = u8::read_options(reader, endian, args)?;
        let system_status = SystemStatus::new(system_status_raw);
        Ok(Self { system_status })
    }
}

impl BinWrite for Bq76920Alerts {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(
        &self,
        writer: &mut W,
        endian: binrw::Endian,
        args: Self::Args<'_>,
    ) -> BinResult<()> {
        self.system_status.0.bits().write_options(writer, endian, args)?;
        Ok(())
    }
}