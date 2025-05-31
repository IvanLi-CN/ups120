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
}; // Import necessary data types
use crate::shared::{
    Bq25730AlertsSubscriber,       // Import BQ25730 alerts subscriber
    Bq25730MeasurementsSubscriber, // Import BQ25730 subscriber
    Bq76920AlertsSubscriber,       // Import BQ76920 alerts subscriber
    Bq76920MeasurementsSubscriber, // Import BQ76920 subscriber
    Ina226MeasurementsSubscriber,  // Import INA226 subscriber
    MeasurementsPublisher,         // Import MeasurementsPublisher
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
                ina226: latest_ina226_measurements.unwrap_or(Ina226Measurements {
                    voltage: 0.0,
                    current: 0.0,
                    power: 0.0,
                }),
                bq76920: latest_bq76920_measurements.unwrap_or_else(|| Bq76920Measurements {
                    core_measurements: bq769x0_async_rs::data_types::Bq76920Measurements {
                        cell_voltages: bq769x0_async_rs::data_types::CellVoltages::new(),
                        temperatures: bq769x0_async_rs::data_types::TemperatureSensorReadings::new(
                        ),
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
// Process USB command if one was stored from select!
            if let Some(cmd) = usb_command_to_process.take() {
                defmt::info!("usb_task: Processing stored USB command: {:?}", cmd);
                let command_payload = convert_to_payload(&aggregated_data);
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
                let status_update_payload = convert_to_payload(&aggregated_data);
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

// Helper function to convert AllMeasurements<5> to AllMeasurementsUsbPayload
fn convert_to_payload(data: &AllMeasurements<5>) -> crate::data_types::AllMeasurementsUsbPayload {
    crate::data_types::AllMeasurementsUsbPayload {
        // BQ25730 Measurements
        bq25730_adc_vbat_raw: data.bq25730.adc_measurements.vbat.to_u16(),
        bq25730_adc_vsys_raw: data.bq25730.adc_measurements.vsys.to_u16(),
        bq25730_adc_ichg_raw: data.bq25730.adc_measurements.ichg.to_u16(),
        bq25730_adc_idchg_raw: data.bq25730.adc_measurements.idchg.to_u16(),
        bq25730_adc_iin_raw: data.bq25730.adc_measurements.iin.to_u16(),
        bq25730_adc_psys_raw: data.bq25730.adc_measurements.psys.to_u16(),
        bq25730_adc_vbus_raw: data.bq25730.adc_measurements.vbus.to_u16(),
        bq25730_adc_cmpin_raw: data.bq25730.adc_measurements.cmpin.to_u16(),

        // BQ76920 Measurements
        bq76920_cell1_mv: data.bq76920.core_measurements.cell_voltages.voltages[0],
        bq76920_cell2_mv: data.bq76920.core_measurements.cell_voltages.voltages[1],
        bq76920_cell3_mv: data.bq76920.core_measurements.cell_voltages.voltages[2],
        bq76920_cell4_mv: data.bq76920.core_measurements.cell_voltages.voltages[3],
        bq76920_cell5_mv: data.bq76920.core_measurements.cell_voltages.voltages[4],
        bq76920_ts1_raw_adc: data.bq76920.core_measurements.temperatures.ts1,
        bq76920_ts2_present: data.bq76920.core_measurements.temperatures.ts2.is_some() as u8,
        bq76920_ts2_raw_adc: data.bq76920.core_measurements.temperatures.ts2.unwrap_or(0),
        bq76920_ts3_present: data.bq76920.core_measurements.temperatures.ts3.is_some() as u8,
        bq76920_ts3_raw_adc: data.bq76920.core_measurements.temperatures.ts3.unwrap_or(0),
        bq76920_is_thermistor: data.bq76920.core_measurements.temperatures.is_thermistor as u8,
        bq76920_current_ma: data.bq76920.core_measurements.current,
        bq76920_system_status_bits: data.bq76920.core_measurements.system_status.0.bits(),
        bq76920_mos_status_bits: data.bq76920.core_measurements.mos_status.0.bits(),

        // Ina226 Measurements
        ina226_voltage_f32: data.ina226.voltage,
        ina226_current_f32: data.ina226.current,
        ina226_power_f32: data.ina226.power,

        // Bq25730 Alerts
        bq25730_charger_status_raw_u16: data.bq25730_alerts.charger_status.to_u16(),
        bq25730_prochot_status_raw_u16: data.bq25730_alerts.prochot_status.to_u16(),

        // Bq76920 Alerts
        bq76920_alerts_system_status_bits: data.bq76920_alerts.system_status.0.bits(),
    }
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
