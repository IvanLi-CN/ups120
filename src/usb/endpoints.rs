use binrw::io::Cursor;
use binrw::{BinRead, BinWrite}; // Remove BinReaderExt and BinWriterExt as they are not directly used here
use embassy_usb::Builder;
use embassy_usb::driver::EndpointError;
use embassy_usb::driver::{Driver, Endpoint, EndpointIn, EndpointOut};

use crate::shared::AllMeasurements;

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
        let n = self.command_read_ep.read(&mut self.read_buffer).await?;
        let mut reader = Cursor::new(&self.read_buffer[..n]);
        let cmd = UsbData::read_be(&mut reader).map_err(|_| EndpointError::BufferOverflow)?; // Use direct BinRead trait method
        Ok(cmd)
    }

    pub async fn process_command(&mut self, command: UsbData) -> Result<(), EndpointError> {
        match command {
            UsbData::SubscribeStatus => {
                self.status_subscription_active = true;
                defmt::info!("Status subscription active");
                // Optionally send a response to confirm subscription
                // let response = UsbData::StatusResponse(...);
                // self.send_response(response).await?;
            }
            UsbData::UnsubscribeStatus => {
                self.status_subscription_active = false;
                defmt::info!("Status subscription inactive");
                // Optionally send a response to confirm unsubscription
                // let response = UsbData::StatusResponse(...);
                // self.send_response(response).await?;
            }
            _ => {
                // Handle unknown commands or other data types if necessary
            }
        }
        Ok(())
    }

    pub async fn send_status_update(
        &mut self,
        data: AllMeasurements<5>,
    ) -> Result<(), EndpointError> {
        if !self.status_subscription_active {
            return Ok(());
        }

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
