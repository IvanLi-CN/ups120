use embassy_stm32::uid;
use embassy_stm32::{peripherals, usb};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub;
use embassy_usb::{
    Builder,
    class::web_usb::{self, Url, WebUsb},
    driver::EndpointError,
};
use static_cell::StaticCell;

use crate::shared::{AllMeasurements, MEASUREMENTS_PUBSUB_DEPTH, MEASUREMENTS_PUBSUB_READERS}; // Remove MEASUREMENTS_PUBSUB as it's not directly used here

pub mod endpoints;

use crate::usb::endpoints::UsbEndpoints;

use embassy_futures::select;

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
    mut measurements_sub: pubsub::Subscriber<
        'static,
        CriticalSectionRawMutex,
        AllMeasurements<5>,
        MEASUREMENTS_PUBSUB_DEPTH, // Use constant
        MEASUREMENTS_PUBSUB_READERS, // Use constant
        1, // Publishers count is 1
    >,
) {
    let vid: u16 = u16::from_str_radix(env!("USB_VID").trim_start_matches("0x"), 16).expect("Invalid USB_VID");
    let pid: u16 = u16::from_str_radix(env!("USB_PID").trim_start_matches("0x"), 16).expect("Invalid USB_PID");

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
        loop {
            usb_endpoints.wait_connected().await;

            let result = select::select(
                usb_endpoints.parse_command(),
                measurements_sub.next_message(),
            )
            .await;

            match result {
                select::Either::First(parse_result) => {
                    match parse_result {
                        Ok(cmd) => {
                            if let Err(e) = usb_endpoints.process_command(cmd).await {
                                defmt::error!("Error processing command: {:?}", e);
                            }
                        }
                        Err(e) => {
                            defmt::error!("USB command endpoint error: {:?}", e);
                        }
                    }
                }
                select::Either::Second(pubsub::WaitResult::Message(msg)) => {
                    if let Err(e) = usb_endpoints.send_status_update(msg).await {
                        defmt::error!("Send status update failed: {:?}", e);
                    }
                }
                select::Either::Second(pubsub::WaitResult::Lagged(c)) => {
                    defmt::warn!("USB measurements sub: lagged {} messages", c);
                }
            }
        }
    };

    let mut usb = builder.build();
    let usb_fut = usb.run();

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