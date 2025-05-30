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
const BQ76920_MEASUREMENTS_PUBSUB_READERS: usize = 2; // 消费者数量 (usb_task, bq25730_task)
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

// Channel Type Aliases
pub type MeasurementsChannelType<const N: usize> = PubSubChannel<
    CriticalSectionRawMutex,
    AllMeasurements<N>,
    MEASUREMENTS_PUBSUB_DEPTH,
    MEASUREMENTS_PUBSUB_READERS,
    1,
>;
pub type Bq25730AlertsChannelType = PubSubChannel<
    CriticalSectionRawMutex,
    Bq25730Alerts,
    BQ25730_ALERTS_PUBSUB_DEPTH,
    BQ25730_ALERTS_PUBSUB_READERS,
    1,
>;
pub type Bq76920AlertsChannelType = PubSubChannel<
    CriticalSectionRawMutex,
    Bq76920Alerts,
    BQ76920_ALERTS_PUBSUB_DEPTH,
    BQ76920_ALERTS_PUBSUB_READERS,
    1,
>;
pub type Bq25730MeasurementsChannelType = PubSubChannel<
    CriticalSectionRawMutex,
    Bq25730Measurements,
    BQ25730_MEASUREMENTS_PUBSUB_DEPTH,
    BQ25730_MEASUREMENTS_PUBSUB_READERS,
    1,
>;
pub type Bq76920MeasurementsChannelType<const N: usize> = PubSubChannel<
    CriticalSectionRawMutex,
    Bq76920Measurements<N>,
    BQ76920_MEASUREMENTS_PUBSUB_DEPTH,
    BQ76920_MEASUREMENTS_PUBSUB_READERS,
    1,
>;
pub type Ina226MeasurementsChannelType = PubSubChannel<
    CriticalSectionRawMutex,
    Ina226Measurements,
    INA226_MEASUREMENTS_PUBSUB_DEPTH,
    INA226_MEASUREMENTS_PUBSUB_READERS,
    1,
>;

// Define a type alias for the complex return type, now named PubSubSetup
// This tuple returns Publishers and references to their corresponding Channels
// for on-demand Subscriber creation.
#[allow(clippy::type_complexity)] // Allow complex type for the tuple
pub type PubSubSetup<'a, const N: usize> = (
    MeasurementsPublisher<'a, N>,
    &'a MeasurementsChannelType<N>,
    Bq25730AlertsPublisher<'a>,
    &'a Bq25730AlertsChannelType,
    Bq76920AlertsPublisher<'a>,
    &'a Bq76920AlertsChannelType,
    Bq25730MeasurementsPublisher<'a>,
    &'a Bq25730MeasurementsChannelType,
    Bq76920MeasurementsPublisher<'a, N>,
    &'a Bq76920MeasurementsChannelType<N>,
    Ina226MeasurementsPublisher<'a>,
    &'a Ina226MeasurementsChannelType,
);

// 初始化 PubSubChannel 实例的函数
pub fn init_pubsubs() -> PubSubSetup<'static, 5> {
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
        measurements_pubsub, // Return the channel itself
        // measurements_pubsub.subscriber().unwrap(), // Subscriber1 - remove, create on demand
        // measurements_pubsub.subscriber().unwrap(), // Subscriber2 - remove, create on demand
        bq25730_alerts_pubsub.publisher().unwrap(),
        bq25730_alerts_pubsub, // Return the channel itself
        // bq25730_alerts_pubsub.subscriber().unwrap(), // Subscriber - remove, create on demand
        bq76920_alerts_pubsub.publisher().unwrap(),
        bq76920_alerts_pubsub, // Return the channel itself
        // bq76920_alerts_pubsub.subscriber().unwrap(), // Subscriber - remove, create on demand
        bq25730_measurements_pubsub.publisher().unwrap(),
        bq25730_measurements_pubsub, // Return the channel itself
        // bq25730_measurements_pubsub.subscriber().unwrap(), // Subscriber - remove, create on demand
        bq76920_measurements_pubsub.publisher().unwrap(),
        bq76920_measurements_pubsub, // Return the channel itself
        // bq76920_measurements_pubsub.subscriber().unwrap(),// Subscriber - remove, create on demand
        ina226_measurements_pubsub.publisher().unwrap(),
        ina226_measurements_pubsub, // Return the channel itself
        // ina226_measurements_pubsub.subscriber().unwrap(), // Subscriber - remove, create on demand
    )
}
