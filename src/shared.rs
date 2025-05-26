//! 共享数据模块，包含消息队列和数据结构定义。

use embassy_sync::pubsub::{PubSubChannel, Publisher, Subscriber};
use static_cell::StaticCell;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use binrw::{BinRead, BinWrite, io::{Read, Write, Seek}, BinResult};
use defmt::Format;
// 在这里定义设备相关的数据结构和消息队列

use bq25730_async_rs::data_types::{AdcMeasurements, ChargerStatus, ProchotStatus};
use bq769x0_async_rs::data_types::{CellVoltages, Temperatures, CoulombCounter, SystemStatus, MosStatus, Bq76920Measurements as Bq76920CoreMeasurements};
use uom::si::{
    electric_potential::millivolt, thermodynamic_temperature::kelvin, electric_current::milliampere,
}; // Import uom units
use uom::si::electric_potential::ElectricPotential; // Import specific uom types
use uom::si::thermodynamic_temperature::ThermodynamicTemperature; // Import specific uom types
use uom::si::electric_current::ElectricCurrent; // Import specific uom types

/// BQ25730 测量数据
#[derive(Debug, Copy, Clone, PartialEq, defmt::Format)] // Removed BinRead, BinWrite
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
#[derive(Debug, Copy, Clone, PartialEq)] // Removed BinRead, BinWrite
pub struct Bq76920Measurements<const N: usize> {
    pub core_measurements: Bq76920CoreMeasurements<N>,
}

/// BQ76920 安全告警信息
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Bq76920Alerts {
    pub system_status: SystemStatus,
}

/// 聚合所有设备的测量数据
#[derive(Debug, Copy, Clone, PartialEq)] // Removed BinRead, BinWrite
pub struct AllMeasurements<const N: usize> {
    pub bq25730: Bq25730Measurements,
    pub bq76920: Bq76920Measurements<N>,
}

impl<const N: usize> Format for AllMeasurements<N> {
    fn format(&self, fmt: defmt::Formatter) {
        defmt::write!(fmt, "AllMeasurements {{ bq25730: {}, bq76920: {{ cell_voltages: [", self.bq25730);
        for i in 0..N {
            defmt::write!(fmt, "{:?}, ", self.bq76920.core_measurements.cell_voltages.voltages[i].get::<millivolt>());
        }
        defmt::write!(fmt, "], temperatures: {{ ts1: {:?}, is_thermistor: {} }}, current: {}, system_status: {{ cc_ready: {}, ovr_temp: {}, uv: {}, ov: {}, scd: {}, ocd: {}, cuv: {}, cov: {} }}, mos_status: {{ charge_on: {}, discharge_on: {} }} }} }}",
            self.bq76920.core_measurements.temperatures.ts1.get::<kelvin>(),
            self.bq76920.core_measurements.temperatures.is_thermistor,
            self.bq76920.core_measurements.current.get::<milliampere>(),
            self.bq76920.core_measurements.system_status.cc_ready,
            self.bq76920.core_measurements.system_status.ovr_temp,
            self.bq76920.core_measurements.system_status.uv,
            self.bq76920.core_measurements.system_status.ov,
            self.bq76920.core_measurements.system_status.scd,
            self.bq76920.core_measurements.system_status.ocd,
            self.bq76920.core_measurements.system_status.cuv,
            self.bq76920.core_measurements.system_status.cov,
            self.bq76920.core_measurements.mos_status.charge_on,
            self.bq76920.core_measurements.mos_status.discharge_on
        );
    }
}

// Manual implementation of BinRead and BinWrite for AllMeasurements
impl<const N: usize> BinRead for AllMeasurements<N> {
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(reader: &mut R, endian: binrw::Endian, args: Self::Args<'_>) -> BinResult<Self> {
        let bq25730_psys_raw = u8::read_options(reader, endian, args)?;
        let bq25730_vbus_raw = u8::read_options(reader, endian, args)?;
        let bq25730_idchg_raw = u8::read_options(reader, endian, args)?;
        let bq25730_ichg_raw = u8::read_options(reader, endian, args)?;
        let bq25730_cmpin_raw = u8::read_options(reader, endian, args)?;
        let bq25730_iin_raw = u8::read_options(reader, endian, args)?;
        let bq25730_vbat_raw = u8::read_options(reader, endian, args)?;
        let bq25730_vsys_raw = u8::read_options(reader, endian, args)?;

        let mut cell_voltages_raw = [0.0f32; N]; // Changed to f32
        for i in 0..N {
            cell_voltages_raw[i] = f32::read_options(reader, endian, args)?; // Changed to f32
        }

        let temperatures_ts1_raw = f32::read_options(reader, endian, args)?; // Changed to f32
        let temperatures_is_thermistor_raw = u8::read_options(reader, endian, args)?; // Read as u8
        let temperatures_is_thermistor = temperatures_is_thermistor_raw != 0; // Convert to bool

        let current_raw = f32::read_options(reader, endian, args)?; // Changed to f32 for ElectricCurrent
        let system_status_raw = u8::read_options(reader, endian, args)?;
        let mos_status_raw = u8::read_options(reader, endian, args)?;


        Ok(Self {
            bq25730: Bq25730Measurements {
                adc_measurements: AdcMeasurements::from_register_values(
                    &[
                        bq25730_psys_raw,
                        bq25730_vbus_raw,
                        bq25730_idchg_raw,
                        bq25730_ichg_raw,
                        bq25730_cmpin_raw,
                        bq25730_iin_raw,
                        bq25730_vbat_raw,
                        bq25730_vsys_raw,
                    ]
                ),
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
                    temperatures: Temperatures {
                        ts1: ThermodynamicTemperature::new::<kelvin>(temperatures_ts1_raw),
                        ts2: None, // Not serializing Option types for simplicity
                        ts3: None, // Not serializing Option types for simplicity
                        is_thermistor: temperatures_is_thermistor,
                    },
                    current: ElectricCurrent::new::<milliampere>(current_raw), // Use ElectricCurrent
                    system_status: SystemStatus::new(system_status_raw),
                    mos_status: MosStatus::new(mos_status_raw),
                }
            },
        })
    }
}

impl<const N: usize> BinWrite for AllMeasurements<N> {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(&self, writer: &mut W, endian: binrw::Endian, args: Self::Args<'_>) -> BinResult<()> {
        self.bq25730.adc_measurements.psys.0.write_options(writer, endian, args)?;
        self.bq25730.adc_measurements.vbus.0.write_options(writer, endian, args)?;
        self.bq25730.adc_measurements.idchg.0.write_options(writer, endian, args)?;
        self.bq25730.adc_measurements.ichg.0.write_options(writer, endian, args)?;
        self.bq25730.adc_measurements.cmpin.0.write_options(writer, endian, args)?;
        self.bq25730.adc_measurements.iin.0.write_options(writer, endian, args)?;
        self.bq25730.adc_measurements.vbat.0.write_options(writer, endian, args)?;
        self.bq25730.adc_measurements.vsys.0.write_options(writer, endian, args)?;

        for i in 0..N {
            self.bq76920.core_measurements.cell_voltages.voltages[i].get::<millivolt>().write_options(writer, endian, args)?;
        }

        self.bq76920.core_measurements.temperatures.ts1.get::<kelvin>().write_options(writer, endian, args)?;
        (self.bq76920.core_measurements.temperatures.is_thermistor as u8).write_options(writer, endian, args)?; // Write as u8

        self.bq76920.core_measurements.current.get::<milliampere>().write_options(writer, endian, args)?; // Write ElectricCurrent
        (self.bq76920.core_measurements.system_status.cc_ready as u8).write_options(writer, endian, args)?;
        (self.bq76920.core_measurements.system_status.ovr_temp as u8).write_options(writer, endian, args)?;
        (self.bq76920.core_measurements.system_status.uv as u8).write_options(writer, endian, args)?;
        (self.bq76920.core_measurements.system_status.ov as u8).write_options(writer, endian, args)?;
        (self.bq76920.core_measurements.system_status.scd as u8).write_options(writer, endian, args)?;
        (self.bq76920.core_measurements.system_status.ocd as u8).write_options(writer, endian, args)?;
        (self.bq76920.core_measurements.system_status.cuv as u8).write_options(writer, endian, args)?;
        (self.bq76920.core_measurements.system_status.cov as u8).write_options(writer, endian, args)?;
        (self.bq76920.core_measurements.mos_status.charge_on as u8).write_options(writer, endian, args)?;
        (self.bq76920.core_measurements.mos_status.discharge_on as u8).write_options(writer, endian, args)?;

        Ok(())
    }
}


// 定义消息队列 (PubSub)
// 测量数据 PubSub
pub const MEASUREMENTS_PUBSUB_DEPTH: usize = 4; // 消息队列深度
pub const MEASUREMENTS_PUBSUB_READERS: usize = 2; // 消费者数量
pub static MEASUREMENTS_PUBSUB: StaticCell<PubSubChannel<CriticalSectionRawMutex, AllMeasurements<5>, MEASUREMENTS_PUBSUB_DEPTH, MEASUREMENTS_PUBSUB_READERS, 1>> = StaticCell::new();

// BQ25730 告警 PubSub
const BQ25730_ALERTS_PUBSUB_DEPTH: usize = 4; // 消息队列深度
const BQ25730_ALERTS_PUBSUB_READERS: usize = 2; // 消费者数量
static BQ25730_ALERTS_PUBSUB: StaticCell<PubSubChannel<CriticalSectionRawMutex, Bq25730Alerts, BQ25730_ALERTS_PUBSUB_DEPTH, BQ25730_ALERTS_PUBSUB_READERS, 1>> = StaticCell::new();

// BQ76920 告警 PubSub
const BQ76920_ALERTS_PUBSUB_DEPTH: usize = 4; // 消息队列深度
const BQ76920_ALERTS_PUBSUB_READERS: usize = 2; // 消费者数量
static BQ76920_ALERTS_PUBSUB: StaticCell<PubSubChannel<CriticalSectionRawMutex, Bq76920Alerts, BQ76920_ALERTS_PUBSUB_DEPTH, BQ76920_ALERTS_PUBSUB_READERS, 1>> = StaticCell::new();


pub type MeasurementsPublisher<'a, const N: usize> = Publisher<'a, CriticalSectionRawMutex, AllMeasurements<N>, MEASUREMENTS_PUBSUB_DEPTH, MEASUREMENTS_PUBSUB_READERS, 1>;
pub type MeasurementsSubscriber<'a, const N: usize> = Subscriber<'a, CriticalSectionRawMutex, AllMeasurements<N>, MEASUREMENTS_PUBSUB_DEPTH, MEASUREMENTS_PUBSUB_READERS, 1>;

pub type Bq25730AlertsPublisher<'a> = Publisher<'a, CriticalSectionRawMutex, Bq25730Alerts, BQ25730_ALERTS_PUBSUB_DEPTH, BQ25730_ALERTS_PUBSUB_READERS, 1>;
pub type Bq25730AlertsSubscriber<'a> = Subscriber<'a, CriticalSectionRawMutex, Bq25730Alerts, BQ25730_ALERTS_PUBSUB_DEPTH, BQ25730_ALERTS_PUBSUB_READERS, 1>;

pub type Bq76920AlertsPublisher<'a> = Publisher<'a, CriticalSectionRawMutex, Bq76920Alerts, BQ76920_ALERTS_PUBSUB_DEPTH, BQ76920_ALERTS_PUBSUB_READERS, 1>;
pub type Bq76920AlertsSubscriber<'a> = Subscriber<'a, CriticalSectionRawMutex, Bq76920Alerts, BQ76920_ALERTS_PUBSUB_DEPTH, BQ76920_ALERTS_PUBSUB_READERS, 1>;

// 初始化 PubSubChannel 实例的函数
pub fn init_pubsubs() -> (
    MeasurementsPublisher<'static, 5>,
    MeasurementsSubscriber<'static, 5>,
    MeasurementsSubscriber<'static, 5>,
    Bq25730AlertsPublisher<'static>,
    Bq25730AlertsSubscriber<'static>,
    Bq76920AlertsPublisher<'static>,
    Bq76920AlertsSubscriber<'static>,
) {
    let measurements_pubsub: &'static PubSubChannel<CriticalSectionRawMutex, AllMeasurements<5>, MEASUREMENTS_PUBSUB_DEPTH, MEASUREMENTS_PUBSUB_READERS, 1> = MEASUREMENTS_PUBSUB.init(PubSubChannel::new());
    let bq25730_alerts_pubsub: &'static PubSubChannel<CriticalSectionRawMutex, Bq25730Alerts, BQ25730_ALERTS_PUBSUB_DEPTH, BQ25730_ALERTS_PUBSUB_READERS, 1> = BQ25730_ALERTS_PUBSUB.init(PubSubChannel::new());
    let bq76920_alerts_pubsub: &'static PubSubChannel<CriticalSectionRawMutex, Bq76920Alerts, BQ76920_ALERTS_PUBSUB_DEPTH, BQ76920_ALERTS_PUBSUB_READERS, 1> = BQ76920_ALERTS_PUBSUB.init(PubSubChannel::new());

    (
        measurements_pubsub.publisher().unwrap(),
        measurements_pubsub.subscriber().unwrap(),
        measurements_pubsub.subscriber().unwrap(),
        bq25730_alerts_pubsub.publisher().unwrap(),
        bq25730_alerts_pubsub.subscriber().unwrap(),
        bq76920_alerts_pubsub.publisher().unwrap(),
        bq76920_alerts_pubsub.subscriber().unwrap(),
    )
}
