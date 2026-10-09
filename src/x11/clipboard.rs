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

/// Encode RgbaImage into in-memory PNG bytes
pub fn encode_image_to_png(img: &RgbaImage) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut png_bytes = Vec::new();
    let encoder = PngEncoder::new(&mut png_bytes);
    encoder.write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        ExtendedColorType::Rgba8,
    )?;
    Ok(png_bytes)
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
