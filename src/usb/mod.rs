use embassy_stm32::uid;
use embassy_stm32::{peripherals, usb};
use embassy_futures::select::{select, Either};
use embassy_usb::{
    Builder,
    class::web_usb::{self, Url, WebUsb},
    driver::EndpointError,
};
use static_cell::StaticCell;

use crate::data_types::{AllMeasurements, Bq25730Measurements, Ina226Measurements, Bq76920Measurements, Bq25730Alerts, Bq76920Alerts}; // Import necessary data types
use crate::shared::{
    MeasurementsPublisher, // Import MeasurementsPublisher
    Bq25730MeasurementsSubscriber, // Import BQ25730 subscriber
    Ina226MeasurementsSubscriber, // Import INA226 subscriber
    Bq76920MeasurementsSubscriber, // Import BQ76920 subscriber
    Bq25730AlertsSubscriber,     // Import BQ25730 alerts subscriber
    Bq76920AlertsSubscriber,     // Import BQ76920 alerts subscriber
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
    mut bq25730_alerts_subscriber: Bq25730AlertsSubscriber<'static>,         // BQ25730 alerts subscriber
    mut bq76920_alerts_subscriber: Bq76920AlertsSubscriber<'static>,         // BQ76920 alerts subscriber
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

        loop {
            usb_endpoints.wait_connected().await;

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
                                usb_endpoints.parse_command()
                            )
                        )
                    )
                )
            ).await {
                Either::First(bq25730_meas_res) => { // BQ25730 Measurements
                    match bq25730_meas_res {
                        embassy_sync::pubsub::WaitResult::Message(msg) => latest_bq25730_measurements = Some(msg),
                        embassy_sync::pubsub::WaitResult::Lagged(c) => defmt::warn!("USB BQ25730 Meas sub: lagged {} messages", c),
                    }
                },
                Either::Second(either_b_c_d_e_f) => match either_b_c_d_e_f {
                    Either::First(ina226_meas_res) => { // INA226 Measurements
                        match ina226_meas_res {
                            embassy_sync::pubsub::WaitResult::Message(msg) => latest_ina226_measurements = Some(msg),
                            embassy_sync::pubsub::WaitResult::Lagged(c) => defmt::warn!("USB INA226 Meas sub: lagged {} messages", c),
                        }
                    },
                    Either::Second(either_c_d_e_f) => match either_c_d_e_f {
                        Either::First(bq76920_meas_res) => { // BQ76920 Measurements
                            match bq76920_meas_res {
                                embassy_sync::pubsub::WaitResult::Message(msg) => latest_bq76920_measurements = Some(msg),
                                embassy_sync::pubsub::WaitResult::Lagged(c) => defmt::warn!("USB BQ76920 Meas sub: lagged {} messages", c),
                            }
                        },
                        Either::Second(either_d_e_f) => match either_d_e_f {
                            Either::First(bq25730_alert_res) => { // BQ25730 Alerts
                                match bq25730_alert_res {
                                    embassy_sync::pubsub::WaitResult::Message(msg) => latest_bq25730_alerts = Some(msg),
                                    embassy_sync::pubsub::WaitResult::Lagged(c) => defmt::warn!("USB BQ25730 Alerts sub: lagged {} messages", c),
                                }
                            },
                            Either::Second(either_e_f) => match either_e_f {
                                Either::First(bq76920_alert_res) => { // BQ76920 Alerts
                                    match bq76920_alert_res {
                                        embassy_sync::pubsub::WaitResult::Message(msg) => latest_bq76920_alerts = Some(msg),
                                        embassy_sync::pubsub::WaitResult::Lagged(c) => defmt::warn!("USB BQ76920 Alerts sub: lagged {} messages", c),
                                    }
                                },
                                Either::Second(cmd_result) => { // USB Command
                                    match cmd_result {
                                        Ok(cmd) => {
                                            defmt::info!("USB command received: {:?}", cmd);
                                            // Aggregation will happen outside this specific arm, before publishing/sending
                                        }
                                        Err(e) => {
                                            defmt::error!("USB command endpoint error: {:?}", e);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Aggregate all latest data (measurements and alerts)
            let all_measurements_and_alerts = AllMeasurements {
                bq25730: latest_bq25730_measurements.unwrap_or_else(|| Bq25730Measurements {
                    adc_measurements: bq25730_async_rs::data_types::AdcMeasurements {
                        psys: bq25730_async_rs::data_types::AdcPsys::from_u8(0),
                        vbus: bq25730_async_rs::data_types::AdcVbus::from_u8(0),
                        idchg: bq25730_async_rs::data_types::AdcIdchg::from_u8(0),
                        ichg: bq25730_async_rs::data_types::AdcIchg::from_u8(0),
                        cmpin: bq25730_async_rs::data_types::AdcCmpin::from_u8(0),
                        iin: bq25730_async_rs::data_types::AdcIin::from_u8(0, true),
                        vbat: bq25730_async_rs::data_types::AdcVbat::from_register_value(0, 0, 0),
                        vsys: bq25730_async_rs::data_types::AdcVsys::from_register_value(0, 0, 0),
                    },
                }),
                ina226: latest_ina226_measurements.unwrap_or_else(|| Ina226Measurements {
                    voltage: 0.0,
                    current: 0.0,
                    power: 0.0,
                }),
                bq76920: latest_bq76920_measurements.unwrap_or_else(|| Bq76920Measurements {
                    core_measurements: bq769x0_async_rs::data_types::Bq76920Measurements {
                        cell_voltages: bq769x0_async_rs::data_types::CellVoltages::new(),
                        temperatures: bq769x0_async_rs::data_types::TemperatureSensorReadings::new(),
                        current: 0i32,
                        system_status: bq769x0_async_rs::data_types::SystemStatus::new(0),
                        mos_status: bq769x0_async_rs::data_types::MosStatus::new(0),
                    },
                }),
                bq25730_alerts: latest_bq25730_alerts.unwrap_or_else(|| Bq25730Alerts {
                    charger_status: bq25730_async_rs::data_types::ChargerStatus {
                        status_flags: bq25730_async_rs::registers::ChargerStatusFlags::empty(),
                        fault_flags: bq25730_async_rs::registers::ChargerStatusFaultFlags::empty(),
                    },
                    prochot_status: bq25730_async_rs::data_types::ProchotStatus {
                        msb_flags: bq25730_async_rs::registers::ProchotStatusMsbFlags::empty(),
                        lsb_flags: bq25730_async_rs::registers::ProchotStatusFlags::empty(),
                        prochot_width: 0,
                    },
                }),
                bq76920_alerts: latest_bq76920_alerts.unwrap_or_else(|| Bq76920Alerts {
                    system_status: bq769x0_async_rs::data_types::SystemStatus::new(0),
                }),
            };

            // Publish the aggregated data
            measurements_publisher.publish_immediate(all_measurements_and_alerts.clone());

            // Send the aggregated data over USB if subscription is active
            if usb_endpoints.status_subscription_active {
                 if let Err(e) = usb_endpoints.send_status_update(all_measurements_and_alerts).await {
                      defmt::error!("Failed to send status update over USB: {:?}", e);
                 }
            }

            // Note: The result of parse_command() is now handled within the select_biased! arm.
            // The previous comment about the result not being directly accessible here is no longer fully accurate.

        }
    };

    let mut usb = builder.build();
    let usb_fut = usb.run();

    // Join the USB driver future with the main processing future
    embassy_futures::join::join(usb_fut, main_usb_processing_fut).await;
}

struct Disconnected {}

impl From<EndpointError> for Disconnected {
    fn from(val: EndpointError) -> Self {
        match val {
            EndpointError::BufferOverflow => panic!("Buffer overflow"),
            EndpointError::Disabled => Disconnected {},
        }
    }
}