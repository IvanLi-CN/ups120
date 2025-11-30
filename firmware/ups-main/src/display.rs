use core::cell::RefCell;

use critical_section::Mutex;
use embassy_time::{Duration, Timer};
use embedded_hal::{delay::DelayNs, digital::OutputPin, spi::SpiBus};
use embedded_hal_async::spi::SpiBus as AsyncSpiBus;
use esp_hal::delay::Delay;

pub const LOGICAL_WIDTH: u16 = 160;
pub const LOGICAL_HEIGHT: u16 = 50;

const X_OFFSET: u16 = 15;
const Y_OFFSET: u16 = 0;

#[derive(Clone, Copy)]
pub struct Rgb565(pub u16);

impl Rgb565 {
    pub const BLACK: Self = Rgb565(0x0000);
}

pub const FRAME_PIXELS: usize = (LOGICAL_WIDTH as usize) * (LOGICAL_HEIGHT as usize);
pub type FrameBuffer = [Rgb565; FRAME_PIXELS];

static FRAMEBUFFER: Mutex<RefCell<FrameBuffer>> =
    Mutex::new(RefCell::new([Rgb565::BLACK; FRAME_PIXELS]));

#[inline]
fn fb_index(x: u16, y: u16) -> Option<usize> {
    if x < LOGICAL_WIDTH && y < LOGICAL_HEIGHT {
        Some((y as usize * LOGICAL_WIDTH as usize) + x as usize)
    } else {
        None
    }
}

pub fn with_framebuffer<R>(f: impl FnOnce(&mut FrameBuffer) -> R) -> R {
    critical_section::with(|cs| {
        let mut fb = FRAMEBUFFER.borrow_ref_mut(cs);
        f(&mut *fb)
    })
}

pub fn clear_framebuffer(color: Rgb565) {
    with_framebuffer(|fb| fb.fill(color));
}

pub fn fill_rect_buffer(fb: &mut FrameBuffer, x0: u16, y0: u16, x1: u16, y1: u16, color: Rgb565) {
    let (min_x, max_x) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
    let (min_y, max_y) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
    for y in min_y..=max_y {
        if y >= LOGICAL_HEIGHT {
            break;
        }
        let row_base = y as usize * LOGICAL_WIDTH as usize;
        for x in min_x..=max_x {
            if x >= LOGICAL_WIDTH {
                break;
            }
            fb[row_base + x as usize] = color;
        }
    }
}

pub fn put_pixel_buffer(fb: &mut FrameBuffer, x: u16, y: u16, color: Rgb565) {
    if let Some(idx) = fb_index(x, y) {
        fb[idx] = color;
    }
}

pub fn flush_framebuffer<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
) -> Result<(), SPI::Error>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    critical_section::with(|cs_token| {
        let fb = FRAMEBUFFER.borrow_ref(cs_token);
        fill_rect_from_buffer(spi, cs, dc, 0, 0, LOGICAL_WIDTH, LOGICAL_HEIGHT, &fb[..])
    })
}

pub async fn flush_framebuffer_async<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
) -> Result<(), SPI::Error>
where
    SPI: AsyncSpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    let (tx0, ty0, tx1, ty1) = transform_bounds(
        0,
        0,
        LOGICAL_WIDTH - 1,
        LOGICAL_HEIGHT - 1,
        DRAW_ORIENTATION,
    );
    set_address_window_async(spi, cs, dc, tx0, ty0, tx1, ty1).await?;
    write_command_async(spi, cs, dc, 0x2C).await?;

    dc.set_high().ok();
    cs.set_low().ok();

    // Stream full framebuffer in the same orientation/order as the blocking path,
    // but copy each pixel out of the global framebuffer inside a short critical
    // section so we never hold the lock across an await point.
    const BYTES_CAP: usize = 512;
    let mut bytes = [0u8; BYTES_CAP];
    let mut idx = 0usize;

    match DRAW_ORIENTATION {
        ScreenOrientation::Portrait => {
            for y in 0..LOGICAL_HEIGHT {
                for x in 0..LOGICAL_WIDTH {
                    let pix = critical_section::with(|cs_token| {
                        let fb = FRAMEBUFFER.borrow_ref(cs_token);
                        fb[(y as usize * LOGICAL_WIDTH as usize) + x as usize]
                    });
                    bytes[idx] = (pix.0 >> 8) as u8;
                    bytes[idx + 1] = (pix.0 & 0xFF) as u8;
                    idx += 2;
                    if idx == BYTES_CAP {
                        spi.write(&bytes).await?;
                        idx = 0;
                    }
                }
            }
        }
        ScreenOrientation::Landscape => {
            for lx in (0..LOGICAL_WIDTH).rev() {
                for ly in 0..LOGICAL_HEIGHT {
                    let pix = critical_section::with(|cs_token| {
                        let fb = FRAMEBUFFER.borrow_ref(cs_token);
                        fb[(ly as usize * LOGICAL_WIDTH as usize) + lx as usize]
                    });
                    bytes[idx] = (pix.0 >> 8) as u8;
                    bytes[idx + 1] = (pix.0 & 0xFF) as u8;
                    idx += 2;
                    if idx == BYTES_CAP {
                        spi.write(&bytes).await?;
                        idx = 0;
                    }
                }
            }
        }
        ScreenOrientation::PortraitSwapped => {
            for ly in (0..LOGICAL_HEIGHT).rev() {
                for lx in (0..LOGICAL_WIDTH).rev() {
                    let pix = critical_section::with(|cs_token| {
                        let fb = FRAMEBUFFER.borrow_ref(cs_token);
                        fb[(ly as usize * LOGICAL_WIDTH as usize) + lx as usize]
                    });
                    bytes[idx] = (pix.0 >> 8) as u8;
                    bytes[idx + 1] = (pix.0 & 0xFF) as u8;
                    idx += 2;
                    if idx == BYTES_CAP {
                        spi.write(&bytes).await?;
                        idx = 0;
                    }
                }
            }
        }
        ScreenOrientation::LandscapeSwapped => {
            for lx in 0..LOGICAL_WIDTH {
                for ly in (0..LOGICAL_HEIGHT).rev() {
                    let pix = critical_section::with(|cs_token| {
                        let fb = FRAMEBUFFER.borrow_ref(cs_token);
                        fb[(ly as usize * LOGICAL_WIDTH as usize) + lx as usize]
                    });
                    bytes[idx] = (pix.0 >> 8) as u8;
                    bytes[idx + 1] = (pix.0 & 0xFF) as u8;
                    idx += 2;
                    if idx == BYTES_CAP {
                        spi.write(&bytes).await?;
                        idx = 0;
                    }
                }
            }
        }
    }

    if idx > 0 {
        spi.write(&bytes[..idx]).await?;
    }
    spi.flush().await?;
    cs.set_high().ok();

    Ok(())
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
enum ScreenOrientation {
    Portrait,
    Landscape,
    PortraitSwapped,
    LandscapeSwapped,
}

const DRAW_ORIENTATION: ScreenOrientation = ScreenOrientation::Landscape;

/// Initialize GC9D01 using the vendor 160x50 profile sequence.
pub fn init<SPI, CS, DC, RST>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    rst: &mut RST,
    delay: &mut Delay,
) -> Result<(), SPI::Error>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
    RST: OutputPin,
{
    // Hardware reset: high -> low -> high with vendor timing (50/50/120 ms)
    rst.set_high().ok();
    delay.delay_ms(50u32);
    rst.set_low().ok();
    delay.delay_ms(50u32);
    rst.set_high().ok();
    delay.delay_ms(120u32);

    // Vendor initialization sequence (mirrored from iso-usb-hub_v2 direct-spi demo)
    write_command(spi, cs, dc, 0xFE)?;
    write_command(spi, cs, dc, 0xEF)?;

    write_command_with_data(spi, cs, dc, 0x86, &[0xFF])?;
    write_command_with_data(spi, cs, dc, 0x87, &[0xFF])?;
    write_command_with_data(spi, cs, dc, 0x8E, &[0xFF])?;
    write_command_with_data(spi, cs, dc, 0x8F, &[0xFF])?;
    write_command_with_data(spi, cs, dc, 0x80, &[0x13])?;
    write_command_with_data(spi, cs, dc, 0x81, &[0x40])?;
    write_command_with_data(spi, cs, dc, 0x82, &[0x0A])?;
    write_command_with_data(spi, cs, dc, 0x83, &[0x0B])?;
    write_command_with_data(spi, cs, dc, 0x84, &[0x60])?;
    write_command_with_data(spi, cs, dc, 0x85, &[0x80])?;
    write_command_with_data(spi, cs, dc, 0x89, &[0x10])?;
    write_command_with_data(spi, cs, dc, 0x8A, &[0x0F])?;
    write_command_with_data(spi, cs, dc, 0x8B, &[0x02])?;
    write_command_with_data(spi, cs, dc, 0x8C, &[0x59])?;
    write_command_with_data(spi, cs, dc, 0x8D, &[0x55])?;

    write_command_with_data(spi, cs, dc, 0x3A, &[0x05])?;
    write_command_with_data(spi, cs, dc, 0xEC, &[0x00])?;
    write_command_with_data(spi, cs, dc, 0x7E, &[0x30])?;
    write_command_with_data(
        spi,
        cs,
        dc,
        0x74,
        &[0x05, 0x4D, 0x00, 0x00, 0x01, 0x00, 0x00],
    )?;
    write_command_with_data(spi, cs, dc, 0xB5, &[0x0D, 0x0D])?;
    write_command_with_data(spi, cs, dc, 0xB6, &[0x00, 0x00])?;
    write_command_with_data(spi, cs, dc, 0x60, &[0x38, 0x09, 0x1E, 0x7A])?;
    write_command_with_data(spi, cs, dc, 0x63, &[0x38, 0xAE, 0x1E, 0x7A])?;
    write_command_with_data(spi, cs, dc, 0x64, &[0x38, 0x0B, 0x70, 0xAB, 0x1E, 0x7A])?;
    write_command_with_data(spi, cs, dc, 0x66, &[0x38, 0x0F, 0x70, 0xAF, 0x1E, 0x7A])?;
    write_command_with_data(
        spi,
        cs,
        dc,
        0x68,
        &[0x00, 0x08, 0x07, 0x00, 0x07, 0x55, 0x6A],
    )?;
    write_command_with_data(spi, cs, dc, 0x6A, &[0x00, 0x00])?;
    write_command_with_data(
        spi,
        cs,
        dc,
        0x6C,
        &[0x22, 0x02, 0x22, 0x02, 0x22, 0x22, 0x50],
    )?;
    write_command_with_data(
        spi,
        cs,
        dc,
        0x6E,
        &[
            0x00, 0x00, 0x00, 0x02, 0x14, 0x12, 0x0C, 0x0A, 0x1E, 0x1D, 0x08, 0x00, 0x16, 0x15,
            0x00, 0x00, 0x00, 0x00, 0x15, 0x16, 0x00, 0x07, 0x1D, 0x1E, 0x09, 0x0B, 0x11, 0x13,
            0x01, 0x00, 0x00, 0x00,
        ],
    )?;

    write_command_with_data(spi, cs, dc, 0x98, &[0x3E])?;
    write_command_with_data(spi, cs, dc, 0x99, &[0x3E])?;
    write_command_with_data(spi, cs, dc, 0x9B, &[0x3B])?;
    write_command_with_data(spi, cs, dc, 0x93, &[0x33, 0x7F, 0x00])?;
    write_command_with_data(spi, cs, dc, 0x91, &[0x0E, 0x09])?;
    write_command_with_data(spi, cs, dc, 0x70, &[0x04, 0x02, 0x0D, 0x04, 0x02, 0x0D])?;
    write_command_with_data(spi, cs, dc, 0x71, &[0x04, 0x02, 0x0D])?;
    write_command_with_data(spi, cs, dc, 0xC3, &[0x26])?;
    write_command_with_data(spi, cs, dc, 0xC4, &[0x26])?;
    write_command_with_data(spi, cs, dc, 0xC9, &[0x1C])?;
    write_command_with_data(spi, cs, dc, 0xF0, &[0x02, 0x03, 0x0A, 0x06, 0x00, 0x1A])?;
    write_command_with_data(spi, cs, dc, 0xF2, &[0x02, 0x03, 0x0A, 0x06, 0x00, 0x1A])?;
    write_command_with_data(spi, cs, dc, 0xF1, &[0x38, 0x78, 0x1B, 0x2E, 0x2F, 0xC8])?;
    write_command_with_data(spi, cs, dc, 0xF3, &[0x38, 0x74, 0x12, 0x2E, 0x2F, 0xDF])?;
    write_command_with_data(spi, cs, dc, 0xBF, &[0x00])?;
    write_command_with_data(spi, cs, dc, 0xF9, &[0x40])?;
    // MADCTL per gc9d01-rs embedded-graphics example (no BGR, std landscape)
    write_command_with_data(spi, cs, dc, 0x36, &[0x00])?;

    write_command(spi, cs, dc, 0x2A)?;
    write_data(spi, cs, dc, &[0x00, 0x0F, 0x00, 0x40])?;
    write_command(spi, cs, dc, 0x2B)?;
    write_data(spi, cs, dc, &[0x00, 0x00, 0x00, 0x9F])?;

    write_command(spi, cs, dc, 0x11)?;
    delay.delay_ms(200u32);
    write_command(spi, cs, dc, 0x29)?;
    write_command(spi, cs, dc, 0x2C)?;

    // Ensure logical area starts as black to avoid ghosting
    clear_screen(spi, cs, dc, Rgb565::BLACK)?;

    Ok(())
}

/// Async variant of GC9D01 init sequence using embedded-hal-async SPI and Embassy timers.
pub async fn init_async<SPI, CS, DC, RST>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    rst: &mut RST,
) -> Result<(), SPI::Error>
where
    SPI: AsyncSpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
    RST: OutputPin,
{
    // Hardware reset: high -> low -> high with vendor timing (50/50/120 ms)
    rst.set_high().ok();
    Timer::after(Duration::from_millis(50)).await;
    rst.set_low().ok();
    Timer::after(Duration::from_millis(50)).await;
    rst.set_high().ok();
    Timer::after(Duration::from_millis(120)).await;

    // Vendor initialization sequence (mirrored from iso-usb-hub_v2 direct-spi demo)
    write_command_async(spi, cs, dc, 0xFE).await?;
    write_command_async(spi, cs, dc, 0xEF).await?;

    write_command_with_data_async(spi, cs, dc, 0x86, &[0xFF]).await?;
    write_command_with_data_async(spi, cs, dc, 0x87, &[0xFF]).await?;
    write_command_with_data_async(spi, cs, dc, 0x8E, &[0xFF]).await?;
    write_command_with_data_async(spi, cs, dc, 0x8F, &[0xFF]).await?;
    write_command_with_data_async(spi, cs, dc, 0x80, &[0x13]).await?;
    write_command_with_data_async(spi, cs, dc, 0x81, &[0x40]).await?;
    write_command_with_data_async(spi, cs, dc, 0x82, &[0x0A]).await?;
    write_command_with_data_async(spi, cs, dc, 0x83, &[0x0B]).await?;
    write_command_with_data_async(spi, cs, dc, 0x84, &[0x60]).await?;
    write_command_with_data_async(spi, cs, dc, 0x85, &[0x80]).await?;
    write_command_with_data_async(spi, cs, dc, 0x89, &[0x10]).await?;
    write_command_with_data_async(spi, cs, dc, 0x8A, &[0x0F]).await?;
    write_command_with_data_async(spi, cs, dc, 0x8B, &[0x02]).await?;
    write_command_with_data_async(spi, cs, dc, 0x8C, &[0x59]).await?;
    write_command_with_data_async(spi, cs, dc, 0x8D, &[0x55]).await?;

    write_command_with_data_async(spi, cs, dc, 0x3A, &[0x05]).await?;
    write_command_with_data_async(spi, cs, dc, 0xEC, &[0x00]).await?;
    write_command_with_data_async(spi, cs, dc, 0x7E, &[0x30]).await?;
    write_command_with_data_async(
        spi,
        cs,
        dc,
        0x74,
        &[0x05, 0x4D, 0x00, 0x00, 0x01, 0x00, 0x00],
    )
    .await?;
    write_command_with_data_async(spi, cs, dc, 0xB5, &[0x0D, 0x0D]).await?;
    write_command_with_data_async(spi, cs, dc, 0xB6, &[0x00, 0x00]).await?;
    write_command_with_data_async(spi, cs, dc, 0x60, &[0x38, 0x09, 0x1E, 0x7A]).await?;
    write_command_with_data_async(spi, cs, dc, 0x63, &[0x38, 0xAE, 0x1E, 0x7A]).await?;
    write_command_with_data_async(spi, cs, dc, 0x64, &[0x38, 0x0B, 0x70, 0xAB, 0x1E, 0x7A]).await?;
    write_command_with_data_async(spi, cs, dc, 0x66, &[0x38, 0x0F, 0x70, 0xAF, 0x1E, 0x7A]).await?;
    write_command_with_data_async(
        spi,
        cs,
        dc,
        0x68,
        &[0x00, 0x08, 0x07, 0x00, 0x07, 0x55, 0x6A],
    )
    .await?;
    write_command_with_data_async(spi, cs, dc, 0x6A, &[0x00, 0x00]).await?;
    write_command_with_data_async(
        spi,
        cs,
        dc,
        0x6C,
        &[0x22, 0x02, 0x22, 0x02, 0x22, 0x22, 0x50],
    )
    .await?;
    write_command_with_data_async(
        spi,
        cs,
        dc,
        0x6E,
        &[
            0x00, 0x00, 0x00, 0x02, 0x14, 0x12, 0x0C, 0x0A, 0x1E, 0x1D, 0x08, 0x00, 0x16, 0x15,
            0x00, 0x00, 0x00, 0x00, 0x15, 0x16, 0x00, 0x07, 0x1D, 0x1E, 0x09, 0x0B, 0x11, 0x13,
            0x01, 0x00, 0x00, 0x00,
        ],
    )
    .await?;

    write_command_with_data_async(spi, cs, dc, 0x98, &[0x3E]).await?;
    write_command_with_data_async(spi, cs, dc, 0x99, &[0x3E]).await?;
    write_command_with_data_async(spi, cs, dc, 0x9B, &[0x3B]).await?;
    write_command_with_data_async(spi, cs, dc, 0x93, &[0x33, 0x7F, 0x00]).await?;
    write_command_with_data_async(spi, cs, dc, 0x91, &[0x0E, 0x09]).await?;
    write_command_with_data_async(spi, cs, dc, 0x70, &[0x04, 0x02, 0x0D, 0x04, 0x02, 0x0D]).await?;
    write_command_with_data_async(spi, cs, dc, 0x71, &[0x04, 0x02, 0x0D]).await?;
    write_command_with_data_async(spi, cs, dc, 0xC3, &[0x26]).await?;
    write_command_with_data_async(spi, cs, dc, 0xC4, &[0x26]).await?;
    write_command_with_data_async(spi, cs, dc, 0xC9, &[0x1C]).await?;
    write_command_with_data_async(spi, cs, dc, 0xF0, &[0x02, 0x03, 0x0A, 0x06, 0x00, 0x1A]).await?;
    write_command_with_data_async(spi, cs, dc, 0xF2, &[0x02, 0x03, 0x0A, 0x06, 0x00, 0x1A]).await?;
    write_command_with_data_async(spi, cs, dc, 0xF1, &[0x38, 0x78, 0x1B, 0x2E, 0x2F, 0xC8]).await?;
    write_command_with_data_async(spi, cs, dc, 0xF3, &[0x38, 0x74, 0x12, 0x2E, 0x2F, 0xDF]).await?;
    write_command_with_data_async(spi, cs, dc, 0xBF, &[0x00]).await?;
    write_command_with_data_async(spi, cs, dc, 0xF9, &[0x40]).await?;
    // MADCTL per gc9d01-rs embedded-graphics example (no BGR, std landscape)
    write_command_with_data_async(spi, cs, dc, 0x36, &[0x00]).await?;

    write_command_async(spi, cs, dc, 0x2A).await?;
    write_data_async(spi, cs, dc, &[0x00, 0x0F, 0x00, 0x40]).await?;
    write_command_async(spi, cs, dc, 0x2B).await?;
    write_data_async(spi, cs, dc, &[0x00, 0x00, 0x00, 0x9F]).await?;

    write_command_async(spi, cs, dc, 0x11).await?;
    Timer::after(Duration::from_millis(200)).await;
    write_command_async(spi, cs, dc, 0x29).await?;
    write_command_async(spi, cs, dc, 0x2C).await?;

    // Ensure logical area starts as black to avoid ghosting
    clear_screen_async(spi, cs, dc, Rgb565::BLACK).await?;

    Ok(())
}

pub fn draw_chessboard<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
) -> Result<(), SPI::Error>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    const BACKGROUND: Rgb565 = Rgb565(0x0843);
    const LIGHT_SQUARE: Rgb565 = Rgb565(0xF7BE);
    const DARK_SQUARE: Rgb565 = Rgb565(0x41A7);

    // Paint entire background first
    clear_screen(spi, cs, dc, BACKGROUND)?;

    const SQUARE_SIZE: u16 = 10;
    const GRID_WIDTH: u16 = 16;
    const GRID_HEIGHT: u16 = 5;

    for row in 0..GRID_HEIGHT {
        let y0 = row * SQUARE_SIZE;
        if y0 >= LOGICAL_HEIGHT {
            break;
        }
        let mut y1 = y0 + SQUARE_SIZE - 1;
        if y1 >= LOGICAL_HEIGHT {
            y1 = LOGICAL_HEIGHT - 1;
        }

        for col in 0..GRID_WIDTH {
            let x0 = col * SQUARE_SIZE;
            if x0 >= LOGICAL_WIDTH {
                break;
            }
            let mut x1 = x0 + SQUARE_SIZE - 1;
            if x1 >= LOGICAL_WIDTH {
                x1 = LOGICAL_WIDTH - 1;
            }

            let is_light = ((row + col) & 1) == 0;
            let color = if is_light { LIGHT_SQUARE } else { DARK_SQUARE };
            fill_area_with_color(spi, cs, dc, x0, y0, x1, y1, color)?;
        }
    }

    Ok(())
}

pub fn clear_screen<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    color: Rgb565,
) -> Result<(), SPI::Error>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    fill_area_with_color(
        spi,
        cs,
        dc,
        0,
        0,
        LOGICAL_WIDTH - 1,
        LOGICAL_HEIGHT - 1,
        color,
    )
}

pub async fn clear_screen_async<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    color: Rgb565,
) -> Result<(), SPI::Error>
where
    SPI: AsyncSpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    fill_area_with_color_async(
        spi,
        cs,
        dc,
        0,
        0,
        LOGICAL_WIDTH - 1,
        LOGICAL_HEIGHT - 1,
        color,
    )
    .await
}

pub fn fill_rect_from_buffer<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    x0: u16,
    y0: u16,
    width: u16,
    height: u16,
    buffer: &[Rgb565],
) -> Result<(), SPI::Error>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    if width == 0 || height == 0 {
        return Ok(());
    }

    let x1 = x0 + width - 1;
    let y1 = y0 + height - 1;
    let total_pixels = (width as usize) * (height as usize);
    let safe_len = core::cmp::min(buffer.len(), total_pixels);
    debug_assert!(buffer.len() >= total_pixels);

    let (tx0, ty0, tx1, ty1) = transform_bounds(x0, y0, x1, y1, DRAW_ORIENTATION);
    set_address_window(spi, cs, dc, tx0, ty0, tx1, ty1)?;
    write_command(spi, cs, dc, 0x2C)?;

    dc.set_high().ok();
    cs.set_low().ok();

    let mut bytes = [0u8; 512];
    let mut byte_idx = 0usize;

    match DRAW_ORIENTATION {
        ScreenOrientation::Portrait => {
            for pix in buffer.iter().take(safe_len) {
                bytes[byte_idx] = (pix.0 >> 8) as u8;
                bytes[byte_idx + 1] = (pix.0 & 0xFF) as u8;
                byte_idx += 2;
                if byte_idx == bytes.len() {
                    spi.write(&bytes)?;
                    byte_idx = 0;
                }
            }
        }
        ScreenOrientation::Landscape => {
            for lx in (x0..=x1).rev() {
                for ly in y0..=y1 {
                    debug_assert!(lx >= x0 && lx <= x1);
                    debug_assert!(ly >= y0 && ly <= y1);
                    let rel_x = usize::from(lx - x0);
                    let rel_y = usize::from(ly - y0);
                    let idx = rel_y * (width as usize) + rel_x;
                    if idx >= safe_len {
                        continue;
                    }
                    let pix = buffer[idx];
                    bytes[byte_idx] = (pix.0 >> 8) as u8;
                    bytes[byte_idx + 1] = (pix.0 & 0xFF) as u8;
                    byte_idx += 2;
                    if byte_idx == bytes.len() {
                        spi.write(&bytes)?;
                        byte_idx = 0;
                    }
                }
            }
        }
        ScreenOrientation::PortraitSwapped => {
            for ly in (y0..=y1).rev() {
                for lx in (x0..=x1).rev() {
                    debug_assert!(lx >= x0 && lx <= x1);
                    debug_assert!(ly >= y0 && ly <= y1);
                    let rel_x = usize::from(lx - x0);
                    let rel_y = usize::from(ly - y0);
                    let idx = rel_y * (width as usize) + rel_x;
                    if idx >= safe_len {
                        continue;
                    }
                    let pix = buffer[idx];
                    bytes[byte_idx] = (pix.0 >> 8) as u8;
                    bytes[byte_idx + 1] = (pix.0 & 0xFF) as u8;
                    byte_idx += 2;
                    if byte_idx == bytes.len() {
                        spi.write(&bytes)?;
                        byte_idx = 0;
                    }
                }
            }
        }
        ScreenOrientation::LandscapeSwapped => {
            for lx in x0..=x1 {
                for ly in (y0..=y1).rev() {
                    debug_assert!(lx >= x0 && lx <= x1);
                    debug_assert!(ly >= y0 && ly <= y1);
                    let rel_x = usize::from(lx - x0);
                    let rel_y = usize::from(ly - y0);
                    let idx = rel_y * (width as usize) + rel_x;
                    if idx >= safe_len {
                        continue;
                    }
                    let pix = buffer[idx];
                    bytes[byte_idx] = (pix.0 >> 8) as u8;
                    bytes[byte_idx + 1] = (pix.0 & 0xFF) as u8;
                    byte_idx += 2;
                    if byte_idx == bytes.len() {
                        spi.write(&bytes)?;
                        byte_idx = 0;
                    }
                }
            }
        }
    }

    if byte_idx > 0 {
        spi.write(&bytes[..byte_idx])?;
    }
    spi.flush()?;
    cs.set_high().ok();
    Ok(())
}

pub async fn fill_rect_from_buffer_async<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    x0: u16,
    y0: u16,
    width: u16,
    height: u16,
    buffer: &[Rgb565],
) -> Result<(), SPI::Error>
where
    SPI: AsyncSpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    if width == 0 || height == 0 {
        return Ok(());
    }

    let x1 = x0 + width - 1;
    let y1 = y0 + height - 1;
    let total_pixels = (width as usize) * (height as usize);
    let safe_len = core::cmp::min(buffer.len(), total_pixels);

    let (tx0, ty0, tx1, ty1) = transform_bounds(x0, y0, x1, y1, DRAW_ORIENTATION);
    set_address_window_async(spi, cs, dc, tx0, ty0, tx1, ty1).await?;
    write_command_async(spi, cs, dc, 0x2C).await?;

    dc.set_high().ok();
    cs.set_low().ok();

    let mut bytes = [0u8; 512];
    let mut byte_idx = 0usize;

    match DRAW_ORIENTATION {
        ScreenOrientation::Portrait => {
            for pix in buffer.iter().take(safe_len) {
                bytes[byte_idx] = (pix.0 >> 8) as u8;
                bytes[byte_idx + 1] = (pix.0 & 0xFF) as u8;
                byte_idx += 2;
                if byte_idx == bytes.len() {
                    spi.write(&bytes).await?;
                    byte_idx = 0;
                }
            }
        }
        ScreenOrientation::Landscape => {
            for lx in (x0..=x1).rev() {
                for ly in y0..=y1 {
                    let rel_x = usize::from(lx - x0);
                    let rel_y = usize::from(ly - y0);
                    let idx = rel_y * (width as usize) + rel_x;
                    if idx >= safe_len {
                        continue;
                    }
                    let pix = buffer[idx];
                    bytes[byte_idx] = (pix.0 >> 8) as u8;
                    bytes[byte_idx + 1] = (pix.0 & 0xFF) as u8;
                    byte_idx += 2;
                    if byte_idx == bytes.len() {
                        spi.write(&bytes).await?;
                        byte_idx = 0;
                    }
                }
            }
        }
        ScreenOrientation::PortraitSwapped => {
            for ly in (y0..=y1).rev() {
                for lx in (x0..=x1).rev() {
                    let rel_x = usize::from(lx - x0);
                    let rel_y = usize::from(ly - y0);
                    let idx = rel_y * (width as usize) + rel_x;
                    if idx >= safe_len {
                        continue;
                    }
                    let pix = buffer[idx];
                    bytes[byte_idx] = (pix.0 >> 8) as u8;
                    bytes[byte_idx + 1] = (pix.0 & 0xFF) as u8;
                    byte_idx += 2;
                    if byte_idx == bytes.len() {
                        spi.write(&bytes).await?;
                        byte_idx = 0;
                    }
                }
            }
        }
        ScreenOrientation::LandscapeSwapped => {
            for lx in x0..=x1 {
                for ly in (y0..=y1).rev() {
                    let rel_x = usize::from(lx - x0);
                    let rel_y = usize::from(ly - y0);
                    let idx = rel_y * (width as usize) + rel_x;
                    if idx >= safe_len {
                        continue;
                    }
                    let pix = buffer[idx];
                    bytes[byte_idx] = (pix.0 >> 8) as u8;
                    bytes[byte_idx + 1] = (pix.0 & 0xFF) as u8;
                    byte_idx += 2;
                    if byte_idx == bytes.len() {
                        spi.write(&bytes).await?;
                        byte_idx = 0;
                    }
                }
            }
        }
    }

    if byte_idx > 0 {
        spi.write(&bytes[..byte_idx]).await?;
    }
    spi.flush().await?;
    cs.set_high().ok();

    Ok(())
}

/// Fill a rectangle (x,y,w,h) within logical 160x50 with a solid color.
#[allow(dead_code)]
pub fn fill_rect<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    color: Rgb565,
) -> Result<(), SPI::Error>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    if w == 0 || h == 0 {
        return Ok(());
    }
    let x1 = x.saturating_add(w - 1);
    let y1 = y.saturating_add(h - 1);
    fill_area_with_color(spi, cs, dc, x, y, x1, y1, color)
}

/// Draw a single pixel.
#[allow(dead_code)]
pub fn draw_pixel<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    x: u16,
    y: u16,
    color: Rgb565,
) -> Result<(), SPI::Error>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    fill_rect(spi, cs, dc, x, y, 1, 1, color)
}

fn fill_area_with_color<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    x0: u16,
    y0: u16,
    x1: u16,
    y1: u16,
    color: Rgb565,
) -> Result<(), SPI::Error>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    let (x0, x1) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
    let (y0, y1) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };

    let width = x1 - x0 + 1;
    let height = y1 - y0 + 1;
    let pixel_count = u32::from(width) * u32::from(height);

    let (tx0, ty0, tx1, ty1) = transform_bounds(x0, y0, x1, y1, DRAW_ORIENTATION);
    set_address_window(spi, cs, dc, tx0, ty0, tx1, ty1)?;
    // Enter RAMWR (0x2C) and stream all pixel data with CS kept LOW.
    // Some controllers get noisy if CS toggles between bursts while RAMWR is active.
    write_command(spi, cs, dc, 0x2C)?;

    // Prepare a fixed batch filled with the target color.
    const BATCH_SIZE: usize = 512; // multiple of 2
    let mut batch = [0u8; BATCH_SIZE];
    let hi = (color.0 >> 8) as u8;
    let lo = (color.0 & 0xFF) as u8;
    for chunk in batch.chunks_exact_mut(2) {
        chunk[0] = hi;
        chunk[1] = lo;
    }

    let pixels_per_batch = (BATCH_SIZE / 2) as u32;
    let full_batches = pixel_count / pixels_per_batch;
    let remainder_pixels = (pixel_count % pixels_per_batch) as usize;

    // Begin a contiguous data phase
    dc.set_high().ok();
    cs.set_low().ok();
    for _ in 0..full_batches {
        spi.write(&batch)?;
    }
    if remainder_pixels > 0 {
        let byte_len = remainder_pixels * 2;
        spi.write(&batch[..byte_len])?;
    }
    spi.flush()?;
    cs.set_high().ok();

    Ok(())
}

async fn fill_area_with_color_async<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    x0: u16,
    y0: u16,
    x1: u16,
    y1: u16,
    color: Rgb565,
) -> Result<(), SPI::Error>
where
    SPI: AsyncSpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    let (x0, x1) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
    let (y0, y1) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };

    let width = x1 - x0 + 1;
    let height = y1 - y0 + 1;
    let pixel_count = u32::from(width) * u32::from(height);

    let (tx0, ty0, tx1, ty1) = transform_bounds(x0, y0, x1, y1, DRAW_ORIENTATION);
    set_address_window_async(spi, cs, dc, tx0, ty0, tx1, ty1).await?;
    // Enter RAMWR (0x2C) and stream all pixel data with CS kept LOW.
    write_command_async(spi, cs, dc, 0x2C).await?;

    // Prepare a fixed batch filled with the target color.
    const BATCH_SIZE: usize = 512; // multiple of 2
    let mut batch = [0u8; BATCH_SIZE];
    let hi = (color.0 >> 8) as u8;
    let lo = (color.0 & 0xFF) as u8;
    for chunk in batch.chunks_exact_mut(2) {
        chunk[0] = hi;
        chunk[1] = lo;
    }

    let pixels_per_batch = (BATCH_SIZE / 2) as u32;
    let full_batches = pixel_count / pixels_per_batch;
    let remainder_pixels = (pixel_count % pixels_per_batch) as usize;

    // Begin a contiguous data phase
    dc.set_high().ok();
    cs.set_low().ok();
    for _ in 0..full_batches {
        spi.write(&batch).await?;
    }
    if remainder_pixels > 0 {
        let byte_len = remainder_pixels * 2;
        spi.write(&batch[..byte_len]).await?;
    }
    spi.flush().await?;
    cs.set_high().ok();

    Ok(())
}

fn transform_bounds(
    x0: u16,
    y0: u16,
    x1: u16,
    y1: u16,
    orientation: ScreenOrientation,
) -> (u16, u16, u16, u16) {
    match orientation {
        ScreenOrientation::Portrait => (x0, y0, x1, y1),
        ScreenOrientation::Landscape => (y0, LOGICAL_WIDTH - 1 - x1, y1, LOGICAL_WIDTH - 1 - x0),
        ScreenOrientation::PortraitSwapped => (
            LOGICAL_WIDTH - 1 - x1,
            LOGICAL_HEIGHT - 1 - y1,
            LOGICAL_WIDTH - 1 - x0,
            LOGICAL_HEIGHT - 1 - y0,
        ),
        ScreenOrientation::LandscapeSwapped => {
            (LOGICAL_HEIGHT - 1 - y1, x0, LOGICAL_HEIGHT - 1 - y0, x1)
        }
    }
}

fn set_address_window<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    x0: u16,
    y0: u16,
    x1: u16,
    y1: u16,
) -> Result<(), SPI::Error>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    let x0 = x0 + X_OFFSET;
    let x1 = x1 + X_OFFSET;
    let y0 = y0 + Y_OFFSET;
    let y1 = y1 + Y_OFFSET;

    write_command(spi, cs, dc, 0x2A)?;
    write_data(
        spi,
        cs,
        dc,
        &[
            (x0 >> 8) as u8,
            (x0 & 0xFF) as u8,
            (x1 >> 8) as u8,
            (x1 & 0xFF) as u8,
        ],
    )?;
    write_command(spi, cs, dc, 0x2B)?;
    write_data(
        spi,
        cs,
        dc,
        &[
            (y0 >> 8) as u8,
            (y0 & 0xFF) as u8,
            (y1 >> 8) as u8,
            (y1 & 0xFF) as u8,
        ],
    )?;
    Ok(())
}

async fn set_address_window_async<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    x0: u16,
    y0: u16,
    x1: u16,
    y1: u16,
) -> Result<(), SPI::Error>
where
    SPI: AsyncSpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    let x0 = x0 + X_OFFSET;
    let x1 = x1 + X_OFFSET;
    let y0 = y0 + Y_OFFSET;
    let y1 = y1 + Y_OFFSET;

    write_command_async(spi, cs, dc, 0x2A).await?;
    write_data_async(
        spi,
        cs,
        dc,
        &[
            (x0 >> 8) as u8,
            (x0 & 0xFF) as u8,
            (x1 >> 8) as u8,
            (x1 & 0xFF) as u8,
        ],
    )
    .await?;
    write_command_async(spi, cs, dc, 0x2B).await?;
    write_data_async(
        spi,
        cs,
        dc,
        &[
            (y0 >> 8) as u8,
            (y0 & 0xFF) as u8,
            (y1 >> 8) as u8,
            (y1 & 0xFF) as u8,
        ],
    )
    .await?;
    Ok(())
}

fn write_command<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    cmd: u8,
) -> Result<(), SPI::Error>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    dc.set_low().ok();
    cs.set_low().ok();
    spi.write(&[cmd])?;
    spi.flush()?;
    cs.set_high().ok();
    Ok(())
}

fn write_data<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    data: &[u8],
) -> Result<(), SPI::Error>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    if data.is_empty() {
        return Ok(());
    }

    dc.set_high().ok();
    cs.set_low().ok();
    spi.write(data)?;
    spi.flush()?;
    cs.set_high().ok();
    Ok(())
}

fn write_command_with_data<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    cmd: u8,
    data: &[u8],
) -> Result<(), SPI::Error>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    write_command(spi, cs, dc, cmd)?;
    write_data(spi, cs, dc, data)
}

async fn write_command_async<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    cmd: u8,
) -> Result<(), SPI::Error>
where
    SPI: AsyncSpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    dc.set_low().ok();
    cs.set_low().ok();
    spi.write(&[cmd]).await?;
    spi.flush().await?;
    cs.set_high().ok();
    Ok(())
}

async fn write_data_async<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    data: &[u8],
) -> Result<(), SPI::Error>
where
    SPI: AsyncSpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    if data.is_empty() {
        return Ok(());
    }

    dc.set_high().ok();
    cs.set_low().ok();
    spi.write(data).await?;
    spi.flush().await?;
    cs.set_high().ok();
    Ok(())
}

async fn write_command_with_data_async<SPI, CS, DC>(
    spi: &mut SPI,
    cs: &mut CS,
    dc: &mut DC,
    cmd: u8,
    data: &[u8],
) -> Result<(), SPI::Error>
where
    SPI: AsyncSpiBus<u8>,
    CS: OutputPin,
    DC: OutputPin,
{
    write_command_async(spi, cs, dc, cmd).await?;
    write_data_async(spi, cs, dc, data).await
}
