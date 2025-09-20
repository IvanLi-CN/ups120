//! 共享数据模块，包含消息队列和数据结构定义。

use crate::data_types::{
    AllMeasurements, BalancingCvRequest, Bq76920Alerts, Bq76920Measurements, Sc8815Alerts,
    Sc8815Measurements,
};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::{PubSubChannel, Publisher, Subscriber};
use static_cell::StaticCell;

// Removed BQ25730 imports as we're using SC8815 now

// LocalNtcParametersWrapper and its impls are removed as Bq76920RuntimeConfig is removed.

// Removed BQ25730RuntimeConfig as we're using SC8815 now

// Bq76920RuntimeConfig and its impl Default are removed.

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

// SC8815 告警 PubSub
const SC8815_ALERTS_PUBSUB_DEPTH: usize = 4; // 消息队列深度
const SC8815_ALERTS_PUBSUB_READERS: usize = 2; // 消费者数量
static SC8815_ALERTS_PUBSUB: StaticCell<
    PubSubChannel<
        CriticalSectionRawMutex,
        Sc8815Alerts,
        SC8815_ALERTS_PUBSUB_DEPTH,
        SC8815_ALERTS_PUBSUB_READERS,
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

// SC8815 测量数据 PubSub
const SC8815_MEASUREMENTS_PUBSUB_DEPTH: usize = 4; // 消息队列深度
const SC8815_MEASUREMENTS_PUBSUB_READERS: usize = 2; // 消费者数量 (usb_task + led_status_task)
static SC8815_MEASUREMENTS_PUBSUB: StaticCell<
    PubSubChannel<
        CriticalSectionRawMutex,
        Sc8815Measurements,
        SC8815_MEASUREMENTS_PUBSUB_DEPTH,
        SC8815_MEASUREMENTS_PUBSUB_READERS,
        1,
    >,
> = StaticCell::new();

// Balancing→Charger coupling PubSub
const BALANCING_CV_PUBSUB_DEPTH: usize = 4; // 消息队列深度
const BALANCING_CV_PUBSUB_READERS: usize = 2; // 消费者数量 (sc8815_task + maybe led)
static BALANCING_CV_PUBSUB: StaticCell<
    PubSubChannel<
        CriticalSectionRawMutex,
        BalancingCvRequest,
        BALANCING_CV_PUBSUB_DEPTH,
        BALANCING_CV_PUBSUB_READERS,
        1,
    >,
> = StaticCell::new();

// BQ25730_RUNTIME_CONFIG_PUBSUB related consts and StaticCell were removed.
// BQ76920_RUNTIME_CONFIG_PUBSUB related consts and StaticCell were removed.

pub type MeasurementsPublisher<'a, const N: usize> = Publisher<
    'a,
    CriticalSectionRawMutex,
    AllMeasurements<N>,
    MEASUREMENTS_PUBSUB_DEPTH,
    MEASUREMENTS_PUBSUB_READERS,
    1,
>;

pub type Sc8815AlertsPublisher<'a> = Publisher<
    'a,
    CriticalSectionRawMutex,
    Sc8815Alerts,
    SC8815_ALERTS_PUBSUB_DEPTH,
    SC8815_ALERTS_PUBSUB_READERS,
    1,
>;
pub type Sc8815AlertsSubscriber<'a> = Subscriber<
    'a,
    CriticalSectionRawMutex,
    Sc8815Alerts,
    SC8815_ALERTS_PUBSUB_DEPTH,
    SC8815_ALERTS_PUBSUB_READERS,
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

pub type Sc8815MeasurementsPublisher<'a> = Publisher<
    'a,
    CriticalSectionRawMutex,
    Sc8815Measurements,
    SC8815_MEASUREMENTS_PUBSUB_DEPTH,
    SC8815_MEASUREMENTS_PUBSUB_READERS,
    1,
>;
pub type Sc8815MeasurementsSubscriber<'a> = Subscriber<
    'a,
    CriticalSectionRawMutex,
    Sc8815Measurements,
    SC8815_MEASUREMENTS_PUBSUB_DEPTH,
    SC8815_MEASUREMENTS_PUBSUB_READERS,
    1,
>;

pub type Bq76920MeasurementsPublisher<'a, const N: usize> = Publisher<
    // Added generic parameter
    'a,
    CriticalSectionRawMutex,
    Bq76920Measurements<N>, // Added generic parameter
    BQ76920_MEASUREMENTS_PUBSUB_DEPTH,
    BQ76920_MEASUREMENTS_PUBSUB_READERS,
    1,
>;
pub type Bq76920MeasurementsSubscriber<'a, const N: usize> = Subscriber<
    // Added generic parameter
    'a,
    CriticalSectionRawMutex,
    Bq76920Measurements<N>, // Added generic parameter
    BQ76920_MEASUREMENTS_PUBSUB_DEPTH,
    BQ76920_MEASUREMENTS_PUBSUB_READERS,
    1,
>;

pub type BalancingCvRequestPublisher<'a> = Publisher<
    'a,
    CriticalSectionRawMutex,
    BalancingCvRequest,
    BALANCING_CV_PUBSUB_DEPTH,
    BALANCING_CV_PUBSUB_READERS,
    1,
>;
pub type BalancingCvRequestSubscriber<'a> = Subscriber<
    'a,
    CriticalSectionRawMutex,
    BalancingCvRequest,
    BALANCING_CV_PUBSUB_DEPTH,
    BALANCING_CV_PUBSUB_READERS,
    1,
>;

// Removed INA226 types as we're replacing with SC8815

// Removed Bq25730RuntimeConfigPublisher and Bq25730RuntimeConfigSubscriber type aliases
// Removed Bq76920RuntimeConfigPublisher and Bq76920RuntimeConfigSubscriber type aliases

// Channel Type Aliases
pub type MeasurementsChannelType<const N: usize> = PubSubChannel<
    CriticalSectionRawMutex,
    AllMeasurements<N>,
    MEASUREMENTS_PUBSUB_DEPTH,
    MEASUREMENTS_PUBSUB_READERS,
    1,
>;
pub type Sc8815AlertsChannelType = PubSubChannel<
    CriticalSectionRawMutex,
    Sc8815Alerts,
    SC8815_ALERTS_PUBSUB_DEPTH,
    SC8815_ALERTS_PUBSUB_READERS,
    1,
>;
pub type Bq76920AlertsChannelType = PubSubChannel<
    CriticalSectionRawMutex,
    Bq76920Alerts,
    BQ76920_ALERTS_PUBSUB_DEPTH,
    BQ76920_ALERTS_PUBSUB_READERS,
    1,
>;
pub type Sc8815MeasurementsChannelType = PubSubChannel<
    CriticalSectionRawMutex,
    Sc8815Measurements,
    SC8815_MEASUREMENTS_PUBSUB_DEPTH,
    SC8815_MEASUREMENTS_PUBSUB_READERS,
    1,
>;
pub type Bq76920MeasurementsChannelType<const N: usize> = PubSubChannel<
    CriticalSectionRawMutex,
    Bq76920Measurements<N>,
    BQ76920_MEASUREMENTS_PUBSUB_DEPTH,
    BQ76920_MEASUREMENTS_PUBSUB_READERS,
    1,
>;
pub type BalancingCvRequestChannelType = PubSubChannel<
    CriticalSectionRawMutex,
    BalancingCvRequest,
    BALANCING_CV_PUBSUB_DEPTH,
    BALANCING_CV_PUBSUB_READERS,
    1,
>;
// Removed INA226 channel type as we're replacing with SC8815
// Removed Bq25730RuntimeConfigChannelType type alias.
// Bq76920RuntimeConfigChannelType type alias was removed.

// Define a type alias for the complex return type, now named PubSubSetup
// This tuple returns Publishers and references to their corresponding Channels
// for on-demand Subscriber creation.
#[allow(clippy::type_complexity)] // Allow complex type for the tuple
pub type PubSubSetup<'a, const N: usize> = (
    MeasurementsPublisher<'a, N>,
    &'a MeasurementsChannelType<N>,
    Sc8815AlertsPublisher<'a>,
    &'a Sc8815AlertsChannelType,
    Bq76920AlertsPublisher<'a>,
    &'a Bq76920AlertsChannelType,
    Sc8815MeasurementsPublisher<'a>,
    &'a Sc8815MeasurementsChannelType,
    Bq76920MeasurementsPublisher<'a, N>,
    &'a Bq76920MeasurementsChannelType<N>,
    BalancingCvRequestPublisher<'a>,
    &'a BalancingCvRequestChannelType,
);

// 初始化 PubSubChannel 实例的函数
pub fn init_pubsubs() -> PubSubSetup<'static, 5> {
    let measurements_pubsub: &'static MeasurementsChannelType<5> =
        MEASUREMENTS_PUBSUB.init(PubSubChannel::new());
    let sc8815_alerts_pubsub: &'static Sc8815AlertsChannelType =
        SC8815_ALERTS_PUBSUB.init(PubSubChannel::new());
    let bq76920_alerts_pubsub: &'static Bq76920AlertsChannelType =
        BQ76920_ALERTS_PUBSUB.init(PubSubChannel::new());
    let bq76920_measurements_pubsub: &'static Bq76920MeasurementsChannelType<5> =
        BQ76920_MEASUREMENTS_PUBSUB.init(PubSubChannel::new());
    let sc8815_measurements_pubsub: &'static Sc8815MeasurementsChannelType =
        SC8815_MEASUREMENTS_PUBSUB.init(PubSubChannel::new());
    let balancing_cv_pubsub: &'static BalancingCvRequestChannelType =
        BALANCING_CV_PUBSUB.init(PubSubChannel::new());

    (
        measurements_pubsub.publisher().unwrap(),
        measurements_pubsub,
        sc8815_alerts_pubsub.publisher().unwrap(),
        sc8815_alerts_pubsub,
        bq76920_alerts_pubsub.publisher().unwrap(),
        bq76920_alerts_pubsub,
        sc8815_measurements_pubsub.publisher().unwrap(),
        sc8815_measurements_pubsub,
        bq76920_measurements_pubsub.publisher().unwrap(),
        bq76920_measurements_pubsub,
        balancing_cv_pubsub.publisher().unwrap(),
        balancing_cv_pubsub,
    )
}
