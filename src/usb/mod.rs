use embassy_futures::select::{Either, select};
use embassy_stm32::uid;
use embassy_stm32::{peripherals, usb};
use embassy_usb::{
    Builder,
    class::web_usb::{self, Url, WebUsb},
    driver::EndpointError,
};
use static_cell::StaticCell;

use crate::data_types::{
    AllMeasurements, Bq25730Alerts, Bq25730Measurements, Bq76920Alerts, Bq76920Measurements,
    Ina226Measurements,
};
use crate::shared::{
    Bq25730AlertsSubscriber, Bq25730MeasurementsSubscriber, Bq76920AlertsSubscriber,
    Bq76920MeasurementsSubscriber, Ina226MeasurementsSubscriber, MeasurementsPublisher,
};

pub mod endpoints;

use crate::usb::endpoints::UsbEndpoints;

// Define statics for USB builder buffers
static CONFIG_DESCRIPTOR_CELL: StaticCell<[u8; 256]> = StaticCell::new();
static BOS_DESCRIPTOR_CELL: StaticCell<[u8; 256]> = StaticCell::new();
static MSOS_DESCRIPTOR_CELL: StaticCell<[u8; 256]> = StaticCell::new();
static CONTROL_BUF_CELL: StaticCell<[u8; 64]> = StaticCell::new();

// Define StaticCells for WebUSB state and config
static WEB_USB_STATE_CELL: StaticCell<web_usb::State> = StaticCell::new();
static WEBUSB_CONFIG_CELL: StaticCell<web_usb::Config> = StaticCell::new();

#[embassy_executor::task]
pub async fn usb_task(
    driver: usb::Driver<'static, peripherals::USB>,
    measurements_publisher: MeasurementsPublisher<'static, 5>, // usb_task now publishes AllMeasurements
    mut bq25730_measurements_subscriber: Bq25730MeasurementsSubscriber<'static>, // BQ25730 subscriber
    mut ina226_measurements_subscriber: Ina226MeasurementsSubscriber<'static>, // INA226 subscriber
    mut bq76920_measurements_subscriber: Bq76920MeasurementsSubscriber<'static, 5>, // BQ76920 subscriber - Added generic parameter
    mut bq25730_alerts_subscriber: Bq25730AlertsSubscriber<'static>, // BQ25730 alerts subscriber
    mut bq76920_alerts_subscriber: Bq76920AlertsSubscriber<'static>, // BQ76920 alerts subscriber
) {
    let vid: u16 =
        u16::from_str_radix(env!("USB_VID").trim_start_matches("0x"), 16).expect("Invalid USB_VID");
    let pid: u16 =
        u16::from_str_radix(env!("USB_PID").trim_start_matches("0x"), 16).expect("Invalid USB_PID");

    let mut usb_config = embassy_usb::Config::new(vid, pid);
    usb_config.manufacturer = Some("Ivan");
    usb_config.product = Some("UPS120");
    usb_config.serial_number = Some(uid::uid_hex());
    usb_config.max_power = 100;
    usb_config.max_packet_size_0 = 64;

    // Initialize descriptor and control buffers using StaticCell
    let config_descriptor: &'static mut [u8; 256] = CONFIG_DESCRIPTOR_CELL.init([0; 256]);
    let bos_descriptor: &'static mut [u8; 256] = BOS_DESCRIPTOR_CELL.init([0; 256]);
    let msos_descriptor: &'static mut [u8; 256] = MSOS_DESCRIPTOR_CELL.init([0; 256]);
    let control_buf: &'static mut [u8; 64] = CONTROL_BUF_CELL.init([0; 64]);

    let web_usb_state = WEB_USB_STATE_CELL.init(web_usb::State::new());
    let webusb_config = WEBUSB_CONFIG_CELL.init(web_usb::Config {
        max_packet_size: 64,
        vendor_code: 1,
        landing_url: Some(Url::new(env!("WEBUSB_LANDING_URL"))),
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

    let main_usb_processing_fut = async {
        // Variables to hold the latest measurements from each task
        let mut latest_bq25730_measurements: Option<Bq25730Measurements> = None;
        let mut latest_ina226_measurements: Option<Ina226Measurements> = None;
        let mut latest_bq76920_measurements: Option<Bq76920Measurements<5>> = None;
        let mut latest_bq25730_alerts: Option<Bq25730Alerts> = None;
        let mut latest_bq76920_alerts: Option<Bq76920Alerts> = None;
        let mut usb_command_to_process: Option<endpoints::UsbData> = None; // Variable to store command from select

        loop {
            usb_endpoints.wait_connected().await;
            usb_command_to_process = None; // Clear previous command at the start of each loop iteration

            // Use select to prioritize handling USB commands and new data
            match select(
                bq25730_measurements_subscriber.next_message(),
                select(
                    ina226_measurements_subscriber.next_message(),
                    select(
                        bq76920_measurements_subscriber.next_message(),
                        select(
                            bq25730_alerts_subscriber.next_message(),
                            select(
                                bq76920_alerts_subscriber.next_message(),
                                usb_endpoints.parse_command(),
                            ),
                        ),
                    ),
                ),
            )
            .await
            {
                Either::First(bq25730_meas_res) => {
                    // BQ25730 Measurements
                    match bq25730_meas_res {
                        embassy_sync::pubsub::WaitResult::Message(msg) => {
                            latest_bq25730_measurements = Some(msg)
                        }
                        embassy_sync::pubsub::WaitResult::Lagged(c) => {
                            defmt::warn!("USB BQ25730 Meas sub: lagged {} messages", c)
                        }
                    }
                }
                Either::Second(either_b_c_d_e_f) => {
                    match either_b_c_d_e_f {
                        Either::First(ina226_meas_res) => {
                            // INA226 Measurements
                            match ina226_meas_res {
                                embassy_sync::pubsub::WaitResult::Message(msg) => {
                                    latest_ina226_measurements = Some(msg)
                                }
                                embassy_sync::pubsub::WaitResult::Lagged(c) => {
                                    defmt::warn!("USB INA226 Meas sub: lagged {} messages", c)
                                }
                            }
                        }
                        Either::Second(either_c_d_e_f) => {
                            match either_c_d_e_f {
                                Either::First(bq76920_meas_res) => {
                                    // BQ76920 Measurements
                                    match bq76920_meas_res {
                                        embassy_sync::pubsub::WaitResult::Message(msg) => {
                                            latest_bq76920_measurements = Some(msg)
                                        }
                                        embassy_sync::pubsub::WaitResult::Lagged(c) => {
                                            defmt::warn!(
                                                "USB BQ76920 Meas sub: lagged {} messages",
                                                c
                                            )
                                        }
                                    }
                                }
                                Either::Second(either_d_e_f) => {
                                    match either_d_e_f {
                                        Either::First(bq25730_alert_res) => {
                                            // BQ25730 Alerts
                                            match bq25730_alert_res {
                                                embassy_sync::pubsub::WaitResult::Message(msg) => {
                                                    latest_bq25730_alerts = Some(msg)
                                                }
                                                embassy_sync::pubsub::WaitResult::Lagged(c) => {
                                                    defmt::warn!(
                                                        "USB BQ25730 Alerts sub: lagged {} messages",
                                                        c
                                                    )
                                                }
                                            }
                                        }
                                        Either::Second(either_e_f) => {
                                            match either_e_f {
                                                Either::First(bq76920_alert_res) => {
                                                    // BQ76920 Alerts
                                                    match bq76920_alert_res {
                                        embassy_sync::pubsub::WaitResult::Message(msg) => latest_bq76920_alerts = Some(msg),
                                        embassy_sync::pubsub::WaitResult::Lagged(c) => defmt::warn!("USB BQ76920 Alerts sub: lagged {} messages", c),
                                    }
                                                }
                                                Either::Second(cmd_result) => {
                                                    // USB Command
                                                    match cmd_result {
                                                        Ok(cmd) => {
                                                            defmt::info!(
                                                                "usb_task: USB command received by select, will process after aggregation: {:?}",
                                                                cmd
                                                            );
                                                            usb_command_to_process = Some(cmd); // Store command for later processing
                                                        }
                                                        Err(e) => {
                                                            defmt::error!(
                                                                "usb_task: USB command endpoint error: {:?}",
                                                                e
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Unified aggregation of all latest data (measurements and alerts)
            let aggregated_data = AllMeasurements {
                bq25730: latest_bq25730_measurements.unwrap_or_default(),
                ina226: latest_ina226_measurements.unwrap_or_default(),
                bq76920: latest_bq76920_measurements.unwrap_or_default(),
                bq25730_alerts: latest_bq25730_alerts.unwrap_or_default(),
                bq76920_alerts: latest_bq76920_alerts.unwrap_or_default(),
            };
            // Process USB command if one was stored from select!
            if let Some(cmd) = usb_command_to_process.take() {
                defmt::info!("usb_task: Processing stored USB command: {:?}", cmd);
                let command_payload = aggregated_data.to_usb_payload();
                if let Err(e) = usb_endpoints.process_command(cmd, &command_payload).await {
                    defmt::error!("usb_task: Error processing USB command: {:?}", e);
                }
                defmt::debug!(
                    "usb_task: process_command finished. Current status_subscription_active: {}",
                    usb_endpoints.status_subscription_active
                );
            }

            defmt::trace!(
                "usb_task: Aggregated data for publishing/sending: {:?}",
                aggregated_data
            );

            // Publish the aggregated data
            measurements_publisher.publish_immediate(aggregated_data);
            defmt::debug!("usb_task: Published aggregated data.");

            // Send the aggregated data over USB if subscription is active
            defmt::debug!(
                "usb_task: Checking if status subscription is active for sending update. status_subscription_active: {}",
                usb_endpoints.status_subscription_active
            );
            if usb_endpoints.status_subscription_active {
                defmt::info!(
                    "usb_task: Subscription active, attempting to send status update via USB."
                );
                // Convert to AllMeasurementsUsbPayload before sending
                let status_update_payload = aggregated_data.to_usb_payload();
                if let Err(e) = usb_endpoints
                    .send_status_update(status_update_payload) // Pass the converted payload
                    .await
                {
                    defmt::error!("usb_task: Failed to send status update over USB: {:?}", e);
                } else {
                    defmt::debug!("usb_task: Successfully sent status update via USB.");
                }
            } else {
                defmt::debug!(
                    "usb_task: Subscription not active, not sending status update via USB."
                );
            }

            // Note: The result of parse_command() is now handled within the select_biased! arm.
            // The previous comment about the result not being directly accessible here is no longer fully accurate.
            defmt::trace!("usb_task: End of loop iteration.");
        }
    };

    let mut usb = builder.build();
    let usb_fut = usb.run();

    // Join the USB driver future with the main processing future
    embassy_futures::join::join(usb_fut, main_usb_processing_fut).await;
}

// The convert_to_payload function has been moved to an impl block for AllMeasurements in data_types.rs
struct Disconnected {}

impl From<EndpointError> for Disconnected {
    fn from(val: EndpointError) -> Self {
        match val {
            EndpointError::BufferOverflow => panic!("Buffer overflow"),
            EndpointError::Disabled => Disconnected {},
        }
    }
}
