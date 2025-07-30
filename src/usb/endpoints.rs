//! USB端点处理模块
//! 
//! 定义USB数据协议和端点通信逻辑

use binrw::io::Cursor;
use binrw::io::{Read, Seek};
use binrw::{BinRead, BinResult, BinWrite, Endian};
use embassy_usb::Builder;
use embassy_usb::driver::EndpointError;
use embassy_usb::driver::{Driver, Endpoint, EndpointIn, EndpointOut};

use crate::data_types::AllMeasurementsUsbPayload;

/// USB数据协议定义
#[repr(u8)]
#[derive(BinWrite, Debug, Clone, Copy, defmt::Format)]
pub enum UsbData {
    // 命令
    #[brw(magic = 0x00u8)]
    SubscribeStatus,
    #[brw(magic = 0x01u8)]
    UnsubscribeStatus,

    // 响应
    #[brw(magic = 0x80u8)]
    StatusResponse(AllMeasurementsUsbPayload),

    // 推送数据
    #[brw(magic = 0xC0u8)]
    StatusPush(AllMeasurementsUsbPayload),
}

impl BinRead for UsbData {
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(
        reader: &mut R,
        endian: Endian,
        _args: Self::Args<'_>,
    ) -> BinResult<Self> {
        let magic = u8::read_options(reader, endian, ())?;
        match magic {
            0x00 => Ok(UsbData::SubscribeStatus),
            0x01 => Ok(UsbData::UnsubscribeStatus),
            0x80 => {
                let payload = AllMeasurementsUsbPayload::read_options(reader, endian, ())?;
                Ok(UsbData::StatusResponse(payload))
            }
            0xC0 => {
                let payload = AllMeasurementsUsbPayload::read_options(reader, endian, ())?;
                Ok(UsbData::StatusPush(payload))
            }
            _ => {
                defmt::error!("[UsbData] Unknown magic byte: {:#02x}", magic);
                Err(binrw::Error::NoVariantMatch {
                    pos: reader.stream_position().unwrap_or(0).saturating_sub(1),
                })
            }
        }
    }
}

/// USB端点管理结构
pub struct UsbEndpoints<'d, D: Driver<'d>> {
    pub command_read_ep: D::EndpointOut,
    pub response_write_ep: D::EndpointIn,
    pub push_write_ep: D::EndpointIn,
    read_buffer: [u8; 128],
    write_buffer: [u8; 128],
    pub status_subscription_active: bool,
}

impl<'d, D: Driver<'d>> UsbEndpoints<'d, D> {
    /// 创建新的USB端点实例
    pub fn new(builder: &mut Builder<'d, D>) -> Self {
        let mut func = builder.function(0xff, 0x00, 0x00);
        let mut iface = func.interface();
        let mut alt = iface.alt_setting(0xff, 0x00, 0x00, None);

        // 创建端点
        let command_read_ep = alt.endpoint_bulk_out(64);
        let response_write_ep = alt.endpoint_bulk_in(64);
        let push_write_ep = alt.endpoint_bulk_in(64);

        Self {
            command_read_ep,
            response_write_ep,
            push_write_ep,
            read_buffer: [0; 128],
            write_buffer: [0; 128],
            status_subscription_active: false,
        }
    }

    /// 等待USB连接
    pub async fn wait_connected(&mut self) {
        self.command_read_ep.wait_enabled().await;
    }

    /// 解析USB命令
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
                defmt::error!(
                    "parse_command: Failed to parse command. Raw data: {:x}",
                    &self.read_buffer[..n]
                );
                Err(EndpointError::BufferOverflow)
            }
        }
    }

    /// 发送响应数据
    async fn send_response(&mut self, data: UsbData) -> Result<(), EndpointError> {
        let mut writer = Cursor::new(&mut self.write_buffer[..]);
        data.write_be(&mut writer)
            .map_err(|_| EndpointError::BufferOverflow)?;
        let len = writer.position() as usize;

        let mut cur = 0;
        let max_packet = 64;
        while cur < len {
            let size = core::cmp::min(len - cur, max_packet);
            self.response_write_ep
                .write(&self.write_buffer[cur..(cur + size)])
                .await?;
            cur += size;
        }
        Ok(())
    }

    /// 处理USB命令
    pub async fn process_command(
        &mut self,
        command: UsbData,
        current_payload: &AllMeasurementsUsbPayload,
    ) -> Result<(), EndpointError> {
        defmt::info!(
            "process_command: Received command: {:?}, current_subscription_status: {}",
            command,
            self.status_subscription_active
        );

        match command {
            UsbData::SubscribeStatus => {
                defmt::info!("process_command: Processing SubscribeStatus command");
                self.status_subscription_active = true;
                let response = UsbData::StatusResponse(*current_payload);
                self.send_response(response).await?;
                defmt::info!("process_command: SubscribeStatus processed, subscription activated");
            }
            UsbData::UnsubscribeStatus => {
                defmt::info!("process_command: Processing UnsubscribeStatus command");
                self.status_subscription_active = false;
                let response = UsbData::StatusResponse(*current_payload);
                self.send_response(response).await?;
                defmt::info!("process_command: UnsubscribeStatus processed, subscription deactivated");
            }
            UsbData::StatusResponse(_) | UsbData::StatusPush(_) => {
                defmt::warn!("process_command: Received unexpected response/push command, ignoring");
            }
        }

        Ok(())
    }

    /// 发送状态更新
    pub async fn send_status_update(
        &mut self,
        data: AllMeasurementsUsbPayload,
    ) -> Result<(), EndpointError> {
        defmt::trace!("send_status_update: Preparing to send status update");

        let mut writer = Cursor::new(&mut self.write_buffer[..]);
        UsbData::StatusPush(data)
            .write_be(&mut writer)
            .map_err(|_| EndpointError::BufferOverflow)?;
        let len = writer.position() as usize;
        defmt::info!("固件发送原始字节: {:x}", &self.write_buffer[..len]);

        let mut cur = 0;
        let max_packet = 64;
        while cur < len {
            let size = core::cmp::min(len - cur, max_packet);
            self.push_write_ep
                .write(&self.write_buffer[cur..(cur + size)])
                .await?;
            cur += size;
        }

        defmt::trace!("send_status_update: Status update sent successfully");
        Ok(())
    }
}
