//! 共享数据模块，包含消息队列和数据结构定义。

use embassy_sync::pubsub::{PubSubChannel, Publisher, Subscriber};
use static_cell::StaticCell;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
// 在这里定义设备相关的数据结构和消息队列

use bq25730_async_rs::data_types::{AdcMeasurements, ChargerStatus, ProchotStatus};
use bq769x0_async_rs::data_types::{CellVoltages, Temperatures, CoulombCounter, SystemStatus};

/// BQ25730 测量数据
#[derive(Debug, Copy, Clone, PartialEq)]
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
pub struct Bq76920Measurements<const N: usize> {
    pub cell_voltages: CellVoltages<N>,
    pub temperatures: Temperatures,
    pub coulomb_counter: CoulombCounter,
    // 添加其他非告警相关的测量数据字段（如果需要）
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

// 定义消息队列 (PubSub)
// 测量数据 PubSub
const MEASUREMENTS_PUBSUB_DEPTH: usize = 4; // 消息队列深度
const MEASUREMENTS_PUBSUB_READERS: usize = 2; // 消费者数量
static MEASUREMENTS_PUBSUB: StaticCell<PubSubChannel<CriticalSectionRawMutex, AllMeasurements<5>, MEASUREMENTS_PUBSUB_DEPTH, MEASUREMENTS_PUBSUB_READERS, 1>> = StaticCell::new();

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
