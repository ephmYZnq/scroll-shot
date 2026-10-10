use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder, RgbaImage};
use std::error::Error;
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt, CreateWindowAux, EventMask, PropMode, SelectionNotifyEvent,
    SelectionRequestEvent, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;

/// Encode RgbaImage into in-memory PNG bytes with 1:1 96 DPI metadata (pHYs chunk)
pub fn encode_image_to_png(img: &RgbaImage) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut png_bytes = Vec::new();
    let encoder = PngEncoder::new(&mut png_bytes);
    encoder.write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        ExtendedColorType::Rgba8,
    )?;

    // Insert 96 DPI (3780 pixels per meter) pHYs chunk to ensure 100% 1:1 native rendering
    let png_with_phys = insert_phys_chunk(png_bytes, 3780);
    Ok(png_with_phys)
}

/// Copy RgbaImage directly to X11 CLIPBOARD by spawning a background clipboard server
pub fn copy_to_clipboard(img: &RgbaImage) -> Result<(), Box<dyn Error>> {
    let png_bytes = encode_image_to_png(img)?;

    // Spawn a detached background process of ourselves with --serve-clipboard
    let exe = std::env::current_exe()?;
    let mut child = Command::new(exe)
        .arg("--serve-clipboard")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&png_bytes)?;
        stdin.flush()?;
    }

    Ok(())
}

/// Copy UTF-8 text directly to X11 CLIPBOARD by spawning a background text clipboard server
pub fn copy_text_to_clipboard(text: &str) -> Result<(), Box<dyn Error>> {
    let text_bytes = text.as_bytes();

    let exe = std::env::current_exe()?;
    let mut child = Command::new(exe)
        .arg("--serve-text-clipboard")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text_bytes)?;
        stdin.flush()?;
    }

    Ok(())
}

/// Run clipboard daemon for text (reads UTF-8 from stdin, owns CLIPBOARD and PRIMARY)
pub fn run_text_clipboard_daemon() -> Result<(), Box<dyn Error>> {
    let mut text_bytes = Vec::new();
    io::stdin().read_to_end(&mut text_bytes)?;

    if text_bytes.is_empty() {
        return Ok(());
    }

    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;

    // Intern required atoms for text clipboard
    let clipboard_atom = conn.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
    let primary_atom = AtomEnum::PRIMARY.into();
    let targets_atom = conn.intern_atom(false, b"TARGETS")?.reply()?.atom;
    let utf8_string_atom = conn.intern_atom(false, b"UTF8_STRING")?.reply()?.atom;
    let string_atom = AtomEnum::STRING.into();
    let text_atom = conn.intern_atom(false, b"TEXT")?.reply()?.atom;

    // Create an unmapped hidden 1x1 window to own the selection
    let win = conn.generate_id()?;
    conn.create_window(
        0,
        win,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )?;

    // Acquire both CLIPBOARD and PRIMARY selections (supports Ctrl+V and middle-click)
    conn.set_selection_owner(win, clipboard_atom, x11rb::CURRENT_TIME)?;
    let _ = conn.set_selection_owner(win, primary_atom, x11rb::CURRENT_TIME);

    let owner_reply = conn.get_selection_owner(clipboard_atom)?.reply()?;
    if owner_reply.owner != win {
        return Err("Failed to acquire X11 text clipboard selection".into());
    }

    conn.flush()?;

    // Event loop to serve SelectionRequest
    loop {
        let event = match conn.wait_for_event() {
            Ok(ev) => ev,
            Err(_) => break,
        };

        match event {
            Event::SelectionRequest(req) => {
                if req.selection == clipboard_atom || req.selection == primary_atom {
                    handle_text_selection_request(
                        &conn,
                        &req,
                        targets_atom,
                        utf8_string_atom,
                        string_atom,
                        text_atom,
                        &text_bytes,
                    )?;
                }
            }
            Event::SelectionClear(ev) => {
                if ev.selection == clipboard_atom {
                    break;
                }
            }
            _ => {}
        }
    }

    let _ = conn.destroy_window(win);
    let _ = conn.flush();
    Ok(())
}

fn handle_text_selection_request<C: Connection>(
    conn: &C,
    req: &SelectionRequestEvent,
    targets_atom: Atom,
    utf8_string_atom: Atom,
    string_atom: Atom,
    text_atom: Atom,
    text_bytes: &[u8],
) -> Result<(), Box<dyn Error>> {
    let mut success = false;
    let property = if req.property == x11rb::NONE {
        req.target
    } else {
        req.property
    };

    if req.target == targets_atom {
        let supported_targets = [targets_atom, utf8_string_atom, string_atom, text_atom];
        conn.change_property32(
            PropMode::REPLACE,
            req.requestor,
            property,
            AtomEnum::ATOM,
            &supported_targets,
        )?;
        success = true;
    } else if req.target == utf8_string_atom || req.target == string_atom || req.target == text_atom {
        conn.change_property8(
            PropMode::REPLACE,
            req.requestor,
            property,
            req.target,
            text_bytes,
        )?;
        success = true;
    }

    let notify = SelectionNotifyEvent {
        response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: req.time,
        requestor: req.requestor,
        selection: req.selection,
        target: req.target,
        property: if success { property } else { x11rb::NONE },
    };

    conn.send_event(false, req.requestor, EventMask::NO_EVENT, notify)?;
    conn.flush()?;
    Ok(())
}

/// Run clipboard daemon (reads PNG from stdin, owns XA_CLIPBOARD, serves requests until replaced)
pub fn run_clipboard_daemon() -> Result<(), Box<dyn Error>> {
    let mut png_bytes = Vec::new();
    io::stdin().read_to_end(&mut png_bytes)?;

    if png_bytes.is_empty() {
        return Ok(());
    }

    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;

    // Intern required atoms for image clipboard
    let clipboard_atom = conn.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
    let targets_atom = conn.intern_atom(false, b"TARGETS")?.reply()?.atom;
    let png_atom = conn.intern_atom(false, b"image/png")?.reply()?.atom;
    let x_png_atom = conn.intern_atom(false, b"image/x-png")?.reply()?.atom;

    // Create an unmapped hidden 1x1 window to own the clipboard
    let win = conn.generate_id()?;
    conn.create_window(
        0,
        win,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )?;

    // Acquire clipboard ownership
    conn.set_selection_owner(win, clipboard_atom, x11rb::CURRENT_TIME)?;
    let owner_reply = conn.get_selection_owner(clipboard_atom)?.reply()?;
    if owner_reply.owner != win {
        return Err("Failed to acquire X11 clipboard selection".into());
    }

    conn.flush()?;

    // Event loop to serve SelectionRequest
    loop {
        let event = match conn.wait_for_event() {
            Ok(ev) => ev,
            Err(_) => break,
        };

        match event {
            Event::SelectionRequest(req) => {
                if req.selection == clipboard_atom {
                    handle_selection_request(
                        &conn,
                        &req,
                        targets_atom,
                        png_atom,
                        x_png_atom,
                        &png_bytes,
                    )?;
                }
            }
            Event::SelectionClear(ev) => {
                // Another application has copied something else to clipboard -> exit daemon cleanly
                if ev.selection == clipboard_atom {
                    break;
                }
            }
            _ => {}
        }
    }

    let _ = conn.destroy_window(win);
    let _ = conn.flush();
    Ok(())
}

fn handle_selection_request<C: Connection>(
    conn: &C,
    req: &SelectionRequestEvent,
    targets_atom: Atom,
    png_atom: Atom,
    x_png_atom: Atom,
    png_bytes: &[u8],
) -> Result<(), Box<dyn Error>> {
    let mut success = false;
    let property = if req.property == x11rb::NONE {
        req.target
    } else {
        req.property
    };

    if req.target == targets_atom {
        // Reply with list of supported targets
        let supported_targets = [targets_atom, png_atom, x_png_atom];
        conn.change_property32(
            PropMode::REPLACE,
            req.requestor,
            property,
            AtomEnum::ATOM,
            &supported_targets,
        )?;
        success = true;
    } else if req.target == png_atom || req.target == x_png_atom {
        // Send PNG image bytes
        conn.change_property8(
            PropMode::REPLACE,
            req.requestor,
            property,
            req.target,
            png_bytes,
        )?;
        success = true;
    }

    let notify = SelectionNotifyEvent {
        response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: req.time,
        requestor: req.requestor,
        selection: req.selection,
        target: req.target,
        property: if success { property } else { x11rb::NONE },
    };

    conn.send_event(false, req.requestor, EventMask::NO_EVENT, notify)?;
    conn.flush()?;
    Ok(())
}

/// Insert a standard PNG pHYs (Physical pixel dimensions) chunk right after IHDR
/// 3780 pixels per meter corresponds to exactly 96.0 DPI
pub fn insert_phys_chunk(png_data: Vec<u8>, pixels_per_meter: u32) -> Vec<u8> {
    if png_data.len() < 33 || &png_data[0..8] != b"\x89PNG\r\n\x1a\n" {
        return png_data;
    }

    // Check if pHYs is already present
    if png_data.windows(4).any(|w| w == b"pHYs") {
        return png_data;
    }

    let mut phys = Vec::with_capacity(21);
    // Chunk data length: 9 bytes
    phys.extend_from_slice(&9u32.to_be_bytes());
    // Chunk type: pHYs
    phys.extend_from_slice(b"pHYs");
    // Pixels per unit X
    phys.extend_from_slice(&pixels_per_meter.to_be_bytes());
    // Pixels per unit Y
    phys.extend_from_slice(&pixels_per_meter.to_be_bytes());
    // Unit specifier: 1 = meter
    phys.push(1);

    // CRC32 of chunk type + data
    let crc = compute_crc32(&phys[4..]);
    phys.extend_from_slice(&crc.to_be_bytes());

    // Insert at byte 33 (directly after 8-byte signature + 25-byte IHDR)
    let mut result = Vec::with_capacity(png_data.len() + 21);
    result.extend_from_slice(&png_data[..33]);
    result.extend_from_slice(&phys);
    result.extend_from_slice(&png_data[33..]);
    result
}

fn compute_crc32(buf: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in buf {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xedb8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc ^ 0xffff_ffff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_phys_chunk() {
        let img = RgbaImage::new(10, 10);
        let mut png_bytes = Vec::new();
        let encoder = PngEncoder::new(&mut png_bytes);
        encoder.write_image(img.as_raw(), 10, 10, ExtendedColorType::Rgba8).unwrap();

        assert!(!png_bytes.windows(4).any(|w| w == b"pHYs"));

        let with_phys = insert_phys_chunk(png_bytes, 3780);
        assert!(with_phys.windows(4).any(|w| w == b"pHYs"));

        // Must still decode cleanly with image crate
        let decoded = image::load_from_memory(&with_phys).unwrap();
        assert_eq!(decoded.width(), 10);
        assert_eq!(decoded.height(), 10);
    }
}
