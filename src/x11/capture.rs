use crate::cli::CaptureRect;
use image::RgbaImage;
use std::error::Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat, Window};

/// Capture a specified rectangle area from the X11 screen and convert it to an RgbaImage
pub fn capture_screen_rect<C: Connection>(
    conn: &C,
    root: Window,
    rect: &CaptureRect,
) -> Result<RgbaImage, Box<dyn Error>> {
    // Request image from X11 root window using ZPixmap format
    let reply = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            root,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            !0, // All planes
        )?
        .reply()?;

    let width = rect.width as u32;
    let height = rect.height as u32;
    let mut img = RgbaImage::new(width, height);

    let raw_bytes = reply.data;
    let expected_len = (width * height * 4) as usize;

    if raw_bytes.len() < expected_len {
        return Err(format!(
            "Unexpected X11 image buffer length: expected {} bytes, got {}",
            expected_len,
            raw_bytes.len()
        )
        .into());
    }

    // Convert BGRX to RGBA (lossless)
    let raw_pixels = &raw_bytes[..expected_len];
    let img_buf = img.as_mut();

    for (src, dst) in raw_pixels.chunks_exact(4).zip(img_buf.chunks_exact_mut(4)) {
        dst[0] = src[2]; // Red
        dst[1] = src[1]; // Green
        dst[2] = src[0]; // Blue
        dst[3] = 255; // Alpha full opacity
    }

    Ok(img)
}
