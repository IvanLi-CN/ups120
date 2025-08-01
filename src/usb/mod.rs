//! USB通信模块
//!
//! 提供WebUSB功能，用于与外部设备进行数据通信和状态监控

pub mod endpoints;

use embassy_futures::select::{Either, select};
use embassy_rp::{peripherals, usb};
use embassy_usb::{
    Builder,
    class::web_usb::{self, Url, WebUsb},
};
use static_cell::StaticCell;

use crate::data_types::{
    AllMeasurements, Bq76920Alerts, Bq76920Measurements, Ina226Measurements, OtgStatus,
    Sc8815Alerts, Sc8815Measurements,
};
use crate::shared::{
    Bq76920AlertsSubscriber, Bq76920MeasurementsSubscriber, Ina226MeasurementsSubscriber,
    MeasurementsPublisher, OtgStatusSubscriber, Sc8815AlertsSubscriber,
    Sc8815MeasurementsSubscriber,
};

use self::endpoints::UsbEndpoints;

// Static cells for USB descriptors and state
static CONFIG_DESCRIPTOR_CELL: StaticCell<[u8; 256]> = StaticCell::new();
static BOS_DESCRIPTOR_CELL: StaticCell<[u8; 256]> = StaticCell::new();
static MSOS_DESCRIPTOR_CELL: StaticCell<[u8; 256]> = StaticCell::new();
static CONTROL_BUF_CELL: StaticCell<[u8; 64]> = StaticCell::new();
static WEB_USB_STATE_CELL: StaticCell<web_usb::State> = StaticCell::new();
static WEBUSB_CONFIG_CELL: StaticCell<web_usb::Config> = StaticCell::new();

/// USB任务
#[embassy_executor::task]
#[allow(clippy::too_many_arguments)]
pub async fn usb_task(
    driver: usb::Driver<'static, peripherals::USB>,
    measurements_publisher: MeasurementsPublisher<'static, 5>,
    mut ina226_measurements_subscriber: Ina226MeasurementsSubscriber<'static>,
    mut sc8815_measurements_subscriber: Sc8815MeasurementsSubscriber<'static>,
    mut bq76920_measurements_subscriber: Bq76920MeasurementsSubscriber<'static, 5>,
    mut sc8815_alerts_subscriber: Sc8815AlertsSubscriber<'static>,
    mut bq76920_alerts_subscriber: Bq76920AlertsSubscriber<'static>,
    mut otg_status_subscriber: OtgStatusSubscriber<'static>,
) {
    // USB配置
    let vid: u16 = 0x1209; // Generic PID.codes VID
    let pid: u16 = 0x0001; // Test PID

    let mut usb_config = embassy_usb::Config::new(vid, pid);
    usb_config.manufacturer = Some("Ivan");
    usb_config.product = Some("UPS120");
    usb_config.serial_number = Some("123456789");
    usb_config.max_power = 100;
    usb_config.max_packet_size_0 = 64;

    // 初始化描述符缓冲区
    let config_descriptor: &'static mut [u8; 256] = CONFIG_DESCRIPTOR_CELL.init([0; 256]);
    let bos_descriptor: &'static mut [u8; 256] = BOS_DESCRIPTOR_CELL.init([0; 256]);
    let msos_descriptor: &'static mut [u8; 256] = MSOS_DESCRIPTOR_CELL.init([0; 256]);
    let control_buf: &'static mut [u8; 64] = CONTROL_BUF_CELL.init([0; 64]);

    let web_usb_state = WEB_USB_STATE_CELL.init(web_usb::State::new());
    let webusb_config = WEBUSB_CONFIG_CELL.init(web_usb::Config {
        max_packet_size: 64,
        vendor_code: 1,
        landing_url: Some(Url::new("https://ups120.example.com")),
    });

    let mut builder = Builder::new(
        driver,
        usb_config,
        config_descriptor,
        bos_descriptor,
        msos_descriptor,
        control_buf,
    );

    WebUsb::configure(&mut builder, web_usb_state, webusb_config);

    let mut usb_endpoints = UsbEndpoints::new(&mut builder);

    // 数据聚合和处理逻辑
    let main_usb_processing_fut = async {
        let mut latest_ina226_measurements: Option<Ina226Measurements> = None;
        let mut latest_sc8815_measurements: Option<Sc8815Measurements> = None;
        let mut latest_bq76920_measurements: Option<Bq76920Measurements<5>> = None;
        let mut latest_sc8815_alerts: Option<Sc8815Alerts> = None;
        let mut latest_bq76920_alerts: Option<Bq76920Alerts> = None;
        let mut latest_otg_status: Option<OtgStatus> = None;
        #[allow(unused_assignments)]
        let mut usb_command_to_process: Option<endpoints::UsbData> = None;

        loop {
            usb_endpoints.wait_connected().await;
            usb_command_to_process = None;

            // 使用select来处理多个数据源
            match select(
                ina226_measurements_subscriber.next_message(),
                select(
                    sc8815_measurements_subscriber.next_message(),
                    select(
                        bq76920_measurements_subscriber.next_message(),
                        select(
                            sc8815_alerts_subscriber.next_message(),
                            select(
                                bq76920_alerts_subscriber.next_message(),
                                select(
                                    otg_status_subscriber.next_message(),
                                    usb_endpoints.parse_command(),
                                ),
                            ),
                        ),
                    ),
                ),
            )
            .await
            {
                Either::First(ina226_meas_res) => {
                    // INA226测量数据
                    match ina226_meas_res {
                        embassy_sync::pubsub::WaitResult::Message(msg) => {
                            latest_ina226_measurements = Some(msg)
                        }
                        embassy_sync::pubsub::WaitResult::Lagged(c) => {
                            defmt::warn!("USB INA226 Meas sub: lagged {} messages", c)
                        }
                    }
                }
                Either::Second(Either::First(sc8815_meas_res)) => {
                    // SC8815测量数据
                    match sc8815_meas_res {
                        embassy_sync::pubsub::WaitResult::Message(msg) => {
                            latest_sc8815_measurements = Some(msg)
                        }
                        embassy_sync::pubsub::WaitResult::Lagged(c) => {
                            defmt::warn!("USB SC8815 Meas sub: lagged {} messages", c)
                        }
                    }
                }
                Either::Second(Either::Second(Either::First(bq76920_meas_res))) => {
                    // BQ76920测量数据
                    match bq76920_meas_res {
                        embassy_sync::pubsub::WaitResult::Message(msg) => {
                            latest_bq76920_measurements = Some(msg)
                        }
                        embassy_sync::pubsub::WaitResult::Lagged(c) => {
                            defmt::warn!("USB BQ76920 Meas sub: lagged {} messages", c)
                        }
                    }
                }
                Either::Second(Either::Second(Either::Second(Either::First(
                    sc8815_alerts_res,
                )))) => {
                    // SC8815告警数据
                    match sc8815_alerts_res {
                        embassy_sync::pubsub::WaitResult::Message(msg) => {
                            latest_sc8815_alerts = Some(msg)
                        }
                        embassy_sync::pubsub::WaitResult::Lagged(c) => {
                            defmt::warn!("USB SC8815 Alerts sub: lagged {} messages", c)
                        }
                    }
                }
                Either::Second(Either::Second(Either::Second(Either::Second(Either::First(
                    bq76920_alerts_res,
                ))))) => {
                    // BQ76920告警数据
                    match bq76920_alerts_res {
                        embassy_sync::pubsub::WaitResult::Message(msg) => {
                            latest_bq76920_alerts = Some(msg)
                        }
                        embassy_sync::pubsub::WaitResult::Lagged(c) => {
                            defmt::warn!("USB BQ76920 Alerts sub: lagged {} messages", c)
                        }
                    }
                }
                Either::Second(Either::Second(Either::Second(Either::Second(Either::Second(
                    Either::First(otg_status_res),
                ))))) => {
                    // OTG状态数据
                    match otg_status_res {
                        embassy_sync::pubsub::WaitResult::Message(msg) => {
                            latest_otg_status = Some(msg)
                        }
                        embassy_sync::pubsub::WaitResult::Lagged(c) => {
                            defmt::warn!("USB OTG Status sub: lagged {} messages", c)
                        }
                    }
                }
                Either::Second(Either::Second(Either::Second(Either::Second(Either::Second(
                    Either::Second(usb_cmd_res),
                ))))) => {
                    // USB命令
                    match usb_cmd_res {
                        Ok(cmd) => {
                            defmt::info!("USB task: Received USB command: {:?}", cmd);
                            usb_command_to_process = Some(cmd);
                        }
                        Err(e) => {
                            defmt::error!("USB task: Error parsing USB command: {:?}", e);
                        }
                    }
                }
            }

            // 聚合所有数据
            let aggregated_data = AllMeasurements {
                ina226: latest_ina226_measurements.unwrap_or_default(),
                sc8815: latest_sc8815_measurements.unwrap_or_default(),
                bq76920: latest_bq76920_measurements.unwrap_or_default(),
                sc8815_alerts: latest_sc8815_alerts.unwrap_or_default(),
                bq76920_alerts: latest_bq76920_alerts.unwrap_or_default(),
            };

            // 处理USB命令
            if let Some(cmd) = usb_command_to_process.take() {
                defmt::info!("USB task: Processing stored USB command: {:?}", cmd);
                let command_payload = aggregated_data.to_usb_payload(latest_otg_status);
                if let Err(e) = usb_endpoints.process_command(cmd, &command_payload).await {
                    defmt::error!("USB task: Error processing USB command: {:?}", e);
                }
            }

            // 发布聚合数据
            measurements_publisher.publish_immediate(aggregated_data);

            // 如果订阅活跃，发送状态更新
            if usb_endpoints.status_subscription_active {
                defmt::info!("USB task: Subscription active, sending status update via USB.");
                let status_update_payload = aggregated_data.to_usb_payload(latest_otg_status);
                if let Err(e) = usb_endpoints
                    .send_status_update(status_update_payload)
                    .await
                {
                    defmt::error!("USB task: Failed to send status update over USB: {:?}", e);
                } else {
                    defmt::debug!("USB task: Successfully sent status update via USB.");
                }
            }
        }
    };

    let mut usb = builder.build();
    let usb_fut = usb.run();

    // 并发运行USB驱动和主处理逻辑
    embassy_futures::join::join(usb_fut, main_usb_processing_fut).await;
}
