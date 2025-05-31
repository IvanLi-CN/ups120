use binrw::io::Cursor;
use binrw::{BinRead, BinWrite}; // Remove BinReaderExt and BinWriterExt as they are not directly used here
use embassy_usb::Builder;
use embassy_usb::driver::EndpointError;
use embassy_usb::driver::{Driver, Endpoint, EndpointIn, EndpointOut};

use crate::data_types::AllMeasurements;

#[repr(u8)]
#[derive(BinRead, BinWrite, Debug, Clone, Copy, defmt::Format)]
pub enum UsbData {
    // Commands
    #[brw(magic = 0x00u8)]
    SubscribeStatus,
    #[brw(magic = 0x01u8)]
    UnsubscribeStatus,

    // Responses
    #[brw(magic = 0x80u8)]
    StatusResponse(AllMeasurements<5>),

    // Push Data
    #[brw(magic = 0xC0u8)]
    StatusPush(AllMeasurements<5>),
}

pub struct UsbEndpoints<'d, D: Driver<'d>> {
    pub command_read_ep: D::EndpointOut,
    pub response_write_ep: D::EndpointIn,
    pub push_write_ep: D::EndpointIn,
    read_buffer: [u8; 128],
    write_buffer: [u8; 128],
    pub status_subscription_active: bool,
}

impl<'d, D: Driver<'d>> UsbEndpoints<'d, D> {
    pub fn new(builder: &mut Builder<'d, D>) -> Self {
        let mut func = builder.function(0xff, 0x00, 0x00);
        let mut iface = func.interface();
        let mut alt = iface.alt_setting(0xff, 0x00, 0x00, None);

        let command_read_ep = alt.endpoint_interrupt_out(64, 16);
        let response_write_ep = alt.endpoint_interrupt_in(64, 16);
        let push_write_ep = alt.endpoint_interrupt_in(64, 16);

        Self {
            command_read_ep,
            response_write_ep,
            push_write_ep,
            read_buffer: [0; 128],
            write_buffer: [0; 128],
            status_subscription_active: false,
        }
    }

    pub async fn wait_connected(&mut self) {
        self.command_read_ep.wait_enabled().await;
        self.response_write_ep.wait_enabled().await;
        self.push_write_ep.wait_enabled().await;
    }

    pub async fn parse_command(&mut self) -> Result<UsbData, EndpointError> {
        defmt::trace!("parse_command: Waiting for data on command_read_ep");
        let n = self.command_read_ep.read(&mut self.read_buffer).await?;
        defmt::info!(
            "parse_command: Received {} bytes: {:x}",
            n,
            &self.read_buffer[..n]
        );
        let mut reader = Cursor::new(&self.read_buffer[..n]);
        match UsbData::read_be(&mut reader) {
            Ok(cmd) => {
                defmt::info!("parse_command: Parsed command: {:?}", cmd);
                Ok(cmd)
            }
            Err(_e) => {
                // Changed `e` to `_e` as it's not directly formatted
                defmt::error!(
                    "parse_command: Failed to parse command (binrw::Error). Raw data: {:x}",
                    &self.read_buffer[..n]
                );
                Err(EndpointError::BufferOverflow) // Or a more specific error if available
            }
        }
    }

    #[allow(dead_code)]
    pub async fn send_response(&mut self, data: UsbData) -> Result<(), EndpointError> {
        defmt::trace!("send_response: Preparing to send response: {:?}", data);
        let mut writer = Cursor::new(&mut self.write_buffer[..]);
        data.write_be(&mut writer).map_err(|_e| {
            // Changed `e` to `_e` as it's not directly formatted
            defmt::error!("send_response: Error writing data to buffer (binrw::Error).");
            EndpointError::BufferOverflow
        })?;
        let len = writer.position() as usize;
        defmt::info!(
            "send_response: Sending response, len={}, raw_bytes={:x}",
            len,
            &self.write_buffer[..len]
        );

        let mut cur = 0;
        let max_packet = 64; // Assuming max packet size for interrupt endpoint
        while cur < len {
            let size = core::cmp::min(len - cur, max_packet);
            self.response_write_ep
                .write(&self.write_buffer[cur..(cur + size)])
                .await?;
            cur += size;
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn process_command(
        &mut self,
        command: UsbData,
        current_measurements: &AllMeasurements<5>,
    ) -> Result<(), EndpointError> {
        defmt::info!(
            "process_command: Received command: {:?}, current_subscription_status: {}",
            command,
            self.status_subscription_active
        );
        match command {
            UsbData::SubscribeStatus => {
                defmt::debug!(
                    "process_command: Processing SubscribeStatus. Old status_subscription_active: {}",
                    self.status_subscription_active
                );
                self.status_subscription_active = true;
                defmt::info!(
                    "process_command: Status subscription ACTIVATED. New status_subscription_active: {}",
                    self.status_subscription_active
                );
                // Send a response to confirm subscription with current data
                let response_data = current_measurements.clone();
                defmt::debug!(
                    "process_command: Preparing StatusResponse with data: {:?}",
                    response_data
                );
                let response = UsbData::StatusResponse(response_data);
                match self.send_response(response).await {
                    Ok(_) => defmt::info!(
                        "process_command: Successfully sent subscription confirmation response."
                    ),
                    Err(e) => defmt::error!(
                        "process_command: Failed to send subscription confirmation response: {:?}",
                        e
                    ),
                }
            }
            UsbData::UnsubscribeStatus => {
                defmt::debug!(
                    "process_command: Processing UnsubscribeStatus. Old status_subscription_active: {}",
                    self.status_subscription_active
                );
                self.status_subscription_active = false;
                defmt::info!(
                    "process_command: Status subscription DEACTIVATED. New status_subscription_active: {}",
                    self.status_subscription_active
                );
                // Optionally send a response to confirm unsubscription
                // We could send a simple ACK here if needed, but for now, just logging is sufficient.
                defmt::debug!("process_command: UnsubscribeStatus processed.");
            }
            _ => {
                defmt::warn!(
                    "process_command: Received unhandled command type: {:?}",
                    command
                );
            }
        }
        Ok(())
    }

    pub async fn send_status_update(
        &mut self,
        data: AllMeasurements<5>,
    ) -> Result<(), EndpointError> {
        defmt::trace!(
            "send_status_update: Entered. Current status_subscription_active: {}",
            self.status_subscription_active
        );
        if !self.status_subscription_active {
            defmt::debug!("send_status_update: Subscription not active, skipping send.");
            return Ok(());
        }
        defmt::debug!(
            "send_status_update: Subscription active, preparing to send data: {:?}",
            data
        );

        let mut writer = Cursor::new(&mut self.write_buffer[..]);
        UsbData::StatusPush(data)
            .write_be(&mut writer)
            .map_err(|_| EndpointError::BufferOverflow)?; // Simplified error handling
        let len = writer.position() as usize; // Remove map_err and '?' as position() returns u64
        defmt::info!("固件发送原始字节: {:x}", &self.write_buffer[..len]); // 添加日志

        let mut cur = 0;
        let max_packet = 64; // Assuming max packet size for interrupt endpoint
        while cur < len {
            let size = core::cmp::min(len - cur, max_packet);
            self.push_write_ep
                .write(&self.write_buffer[cur..(cur + size)])
                .await?;
            cur += size;
        }
        Ok(())
    }
}
