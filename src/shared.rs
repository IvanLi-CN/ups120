//! 共享数据模块，包含消息队列和数据结构定义。

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::{PubSubChannel, Publisher, Subscriber};
use static_cell::StaticCell;
use crate::data_types::{AllMeasurements, Bq25730Alerts, Bq76920Alerts, Bq25730Measurements, Ina226Measurements, Bq76920Measurements};

// 定义消息队列 (PubSub)
// 测量数据 PubSub
pub const MEASUREMENTS_PUBSUB_DEPTH: usize = 4; // 消息队列深度
pub const MEASUREMENTS_PUBSUB_READERS: usize = 2; // 消费者数量
pub static MEASUREMENTS_PUBSUB: StaticCell<
    PubSubChannel<
        CriticalSectionRawMutex,
        AllMeasurements<5>,
        MEASUREMENTS_PUBSUB_DEPTH,
        MEASUREMENTS_PUBSUB_READERS,
        1,
    >,
> = StaticCell::new();

// BQ25730 告警 PubSub
const BQ25730_ALERTS_PUBSUB_DEPTH: usize = 4; // 消息队列深度
const BQ25730_ALERTS_PUBSUB_READERS: usize = 2; // 消费者数量
static BQ25730_ALERTS_PUBSUB: StaticCell<
    PubSubChannel<
        CriticalSectionRawMutex,
        Bq25730Alerts,
        BQ25730_ALERTS_PUBSUB_DEPTH,
        BQ25730_ALERTS_PUBSUB_READERS,
        1,
    >,
> = StaticCell::new();

// BQ76920 告警 PubSub
const BQ76920_ALERTS_PUBSUB_DEPTH: usize = 4; // 消息队列深度
const BQ76920_ALERTS_PUBSUB_READERS: usize = 2; // 消费者数量
static BQ76920_ALERTS_PUBSUB: StaticCell<
    PubSubChannel<
        CriticalSectionRawMutex,
        Bq76920Alerts,
        BQ76920_ALERTS_PUBSUB_DEPTH,
        BQ76920_ALERTS_PUBSUB_READERS,
        1,
    >,
> = StaticCell::new();

// BQ76920 测量数据 PubSub
const BQ76920_MEASUREMENTS_PUBSUB_DEPTH: usize = 4; // 消息队列深度
const BQ76920_MEASUREMENTS_PUBSUB_READERS: usize = 1; // 消费者数量 (目前只有 usb_task)
static BQ76920_MEASUREMENTS_PUBSUB: StaticCell<
    PubSubChannel<
        CriticalSectionRawMutex,
        Bq76920Measurements<5>, // Added generic parameter
        BQ76920_MEASUREMENTS_PUBSUB_DEPTH,
        BQ76920_MEASUREMENTS_PUBSUB_READERS,
        1,
    >,
> = StaticCell::new();

// BQ25730 测量数据 PubSub
const BQ25730_MEASUREMENTS_PUBSUB_DEPTH: usize = 4; // 消息队列深度
const BQ25730_MEASUREMENTS_PUBSUB_READERS: usize = 1; // 消费者数量 (目前只有 bq76920_task)
static BQ25730_MEASUREMENTS_PUBSUB: StaticCell<
    PubSubChannel<
        CriticalSectionRawMutex,
        Bq25730Measurements,
        BQ25730_MEASUREMENTS_PUBSUB_DEPTH,
        BQ25730_MEASUREMENTS_PUBSUB_READERS,
        1,
    >,
> = StaticCell::new();

// INA226 测量数据 PubSub
const INA226_MEASUREMENTS_PUBSUB_DEPTH: usize = 4; // 消息队列深度
const INA226_MEASUREMENTS_PUBSUB_READERS: usize = 2; // 消费者数量
static INA226_MEASUREMENTS_PUBSUB: StaticCell<
    PubSubChannel<
        CriticalSectionRawMutex,
        Ina226Measurements,
        INA226_MEASUREMENTS_PUBSUB_DEPTH,
        INA226_MEASUREMENTS_PUBSUB_READERS,
        1,
    >,
> = StaticCell::new();

pub type MeasurementsPublisher<'a, const N: usize> = Publisher<
    'a,
    CriticalSectionRawMutex,
    AllMeasurements<N>,
    MEASUREMENTS_PUBSUB_DEPTH,
    MEASUREMENTS_PUBSUB_READERS,
    1,
>;
pub type MeasurementsSubscriber<'a, const N: usize> = Subscriber<
    'a,
    CriticalSectionRawMutex,
    AllMeasurements<N>,
    MEASUREMENTS_PUBSUB_DEPTH,
    MEASUREMENTS_PUBSUB_READERS,
    1,
>;

pub type Bq25730AlertsPublisher<'a> = Publisher<
    'a,
    CriticalSectionRawMutex,
    Bq25730Alerts,
    BQ25730_ALERTS_PUBSUB_DEPTH,
    BQ25730_ALERTS_PUBSUB_READERS,
    1,
>;
pub type Bq25730AlertsSubscriber<'a> = Subscriber<
    'a,
    CriticalSectionRawMutex,
    Bq25730Alerts,
    BQ25730_ALERTS_PUBSUB_DEPTH,
    BQ25730_ALERTS_PUBSUB_READERS,
    1,
>;

pub type Bq76920AlertsPublisher<'a> = Publisher<
    'a,
    CriticalSectionRawMutex,
    Bq76920Alerts,
    BQ76920_ALERTS_PUBSUB_DEPTH,
    BQ76920_ALERTS_PUBSUB_READERS,
    1,
>;
pub type Bq76920AlertsSubscriber<'a> = Subscriber<
    'a,
    CriticalSectionRawMutex,
    Bq76920Alerts,
    BQ76920_ALERTS_PUBSUB_DEPTH,
    BQ76920_ALERTS_PUBSUB_READERS,
    1,
>;

pub type Bq25730MeasurementsPublisher<'a> = Publisher<
    'a,
    CriticalSectionRawMutex,
    Bq25730Measurements,
    BQ25730_MEASUREMENTS_PUBSUB_DEPTH,
    BQ25730_MEASUREMENTS_PUBSUB_READERS,
    1,
>;
pub type Bq25730MeasurementsSubscriber<'a> = Subscriber<
    'a,
    CriticalSectionRawMutex,
    Bq25730Measurements,
    BQ25730_MEASUREMENTS_PUBSUB_DEPTH,
    BQ25730_MEASUREMENTS_PUBSUB_READERS,
    1,
>;

pub type Bq76920MeasurementsPublisher<'a, const N: usize> = Publisher< // Added generic parameter
    'a,
    CriticalSectionRawMutex,
    Bq76920Measurements<N>, // Added generic parameter
    BQ76920_MEASUREMENTS_PUBSUB_DEPTH,
    BQ76920_MEASUREMENTS_PUBSUB_READERS,
    1,
>;
pub type Bq76920MeasurementsSubscriber<'a, const N: usize> = Subscriber< // Added generic parameter
    'a,
    CriticalSectionRawMutex,
    Bq76920Measurements<N>, // Added generic parameter
    BQ76920_MEASUREMENTS_PUBSUB_DEPTH,
    BQ76920_MEASUREMENTS_PUBSUB_READERS,
    1,
>;

pub type Ina226MeasurementsPublisher<'a> = Publisher<
    'a,
    CriticalSectionRawMutex,
    Ina226Measurements,
    INA226_MEASUREMENTS_PUBSUB_DEPTH,
    INA226_MEASUREMENTS_PUBSUB_READERS,
    1,
>;
pub type Ina226MeasurementsSubscriber<'a> = Subscriber<
    'a,
    CriticalSectionRawMutex,
    Ina226Measurements,
    INA226_MEASUREMENTS_PUBSUB_DEPTH,
    INA226_MEASUREMENTS_PUBSUB_READERS,
    1,
>;

// Define a type alias for the complex return type
pub type PubSubChannels<'a, const N: usize> = (
    MeasurementsPublisher<'a, N>,
    MeasurementsSubscriber<'a, N>,
    MeasurementsSubscriber<'a, N>,
    Bq25730AlertsPublisher<'a>,
    Bq25730AlertsSubscriber<'a>,
    Bq76920AlertsPublisher<'a>,
    Bq76920AlertsSubscriber<'a>,
    Bq25730MeasurementsPublisher<'a>,
    Bq25730MeasurementsSubscriber<'a>,
    Bq76920MeasurementsPublisher<'a, N>,
    Bq76920MeasurementsSubscriber<'a, N>,
    Ina226MeasurementsPublisher<'a>,
    Ina226MeasurementsSubscriber<'a>,
);

// 初始化 PubSubChannel 实例的函数
pub fn init_pubsubs() -> PubSubChannels<'static, 5> {
    let measurements_pubsub: &'static PubSubChannel<
        CriticalSectionRawMutex,
        AllMeasurements<5>,
        MEASUREMENTS_PUBSUB_DEPTH,
        MEASUREMENTS_PUBSUB_READERS,
        1,
    > = MEASUREMENTS_PUBSUB.init(PubSubChannel::new());
    let bq25730_alerts_pubsub: &'static PubSubChannel<
        CriticalSectionRawMutex,
        Bq25730Alerts,
        BQ25730_ALERTS_PUBSUB_DEPTH,
        BQ25730_ALERTS_PUBSUB_READERS,
        1,
    > = BQ25730_ALERTS_PUBSUB.init(PubSubChannel::new());
    let bq76920_alerts_pubsub: &'static PubSubChannel<
        CriticalSectionRawMutex,
        Bq76920Alerts,
        BQ76920_ALERTS_PUBSUB_DEPTH,
        BQ76920_ALERTS_PUBSUB_READERS,
        1,
    > = BQ76920_ALERTS_PUBSUB.init(PubSubChannel::new());
    let bq76920_measurements_pubsub: &'static PubSubChannel<
        CriticalSectionRawMutex,
        Bq76920Measurements<5>, // Added generic parameter
        BQ76920_MEASUREMENTS_PUBSUB_DEPTH,
        BQ76920_MEASUREMENTS_PUBSUB_READERS,
        1,
    > = BQ76920_MEASUREMENTS_PUBSUB.init(PubSubChannel::new());

    let bq25730_measurements_pubsub: &'static PubSubChannel<
        CriticalSectionRawMutex,
        Bq25730Measurements,
        BQ25730_MEASUREMENTS_PUBSUB_DEPTH,
        BQ25730_MEASUREMENTS_PUBSUB_READERS,
        1,
    > = BQ25730_MEASUREMENTS_PUBSUB.init(PubSubChannel::new());
    let ina226_measurements_pubsub: &'static PubSubChannel<
        CriticalSectionRawMutex,
        Ina226Measurements,
        INA226_MEASUREMENTS_PUBSUB_DEPTH,
        INA226_MEASUREMENTS_PUBSUB_READERS,
        1,
    > = INA226_MEASUREMENTS_PUBSUB.init(PubSubChannel::new());

    (
        measurements_pubsub.publisher().unwrap(),
        measurements_pubsub.subscriber().unwrap(),
        measurements_pubsub.subscriber().unwrap(),
        bq25730_alerts_pubsub.publisher().unwrap(),
        bq25730_alerts_pubsub.subscriber().unwrap(),
        bq76920_alerts_pubsub.publisher().unwrap(),
        bq76920_alerts_pubsub.subscriber().unwrap(),
        bq25730_measurements_pubsub.publisher().unwrap(),
        bq25730_measurements_pubsub.subscriber().unwrap(),
        bq76920_measurements_pubsub.publisher().unwrap(),
        bq76920_measurements_pubsub.subscriber().unwrap(),
        ina226_measurements_pubsub.publisher().unwrap(),
        ina226_measurements_pubsub.subscriber().unwrap(),
    )
}
