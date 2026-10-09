use crate::cli::CaptureRect;
use std::error::Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt, CreateWindowAux, EventMask, GrabMode, GrabStatus,
    WindowClass,
};
use x11rb::protocol::Event;

/// Interactively select a rectangular region on screen by mouse click & drag
#[allow(dead_code)]
pub fn select_region_interactively<C: Connection>(
    conn: &C,
    screen_num: usize,
) -> Result<CaptureRect, Box<dyn Error>> {
    let setup = conn.setup();
    let screen = &setup.roots[screen_num];
    let root = screen.root;

    // Grab pointer on root window to capture drag gestures
    let event_mask =
        EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION;
    let grab_reply = conn
        .grab_pointer(
            false,
            root,
            event_mask,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            root,
            x11rb::NONE,
            x11rb::CURRENT_TIME,
        )?
        .reply()?;

    if grab_reply.status != GrabStatus::SUCCESS {
        return Err("Failed to grab pointer for region selection".into());
    }

    // Grab keyboard to allow ESC to cancel
    let _ = conn.grab_keyboard(
        false,
        root,
        x11rb::CURRENT_TIME,
        GrabMode::ASYNC,
        GrabMode::ASYNC,
    )?;

    // Create an overlay window with a visible border for selection feedback
    let border_win = conn.generate_id()?;
    let border_color = screen.white_pixel; // Or fallback white/highlight

    conn.create_window(
        x11rb::COPY_FROM_PARENT as u8,
        border_win,
        root,
        0,
        0,
        1,
        1,
        2, // 2px border width
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new()
            .override_redirect(1)
            .border_pixel(border_color)
            .event_mask(EventMask::EXPOSURE),
    )?;

    println!("👉 请用鼠标在屏幕上【按住左键并拖拽】框选需要长截屏的区域（按 ESC 键取消）...");

    let mut start_pos: Option<(i16, i16)> = None;
    let mut selected_rect: Option<CaptureRect> = None;

    loop {
        conn.flush()?;
        let event = conn.wait_for_event()?;

        match event {
            Event::ButtonPress(ev) => {
                if ev.detail == 1 {
                    // Left mouse button down: start anchor
                    start_pos = Some((ev.root_x, ev.root_y));
                    conn.configure_window(
                        border_win,
                        &ConfigureWindowAux::new()
                            .x(ev.root_x as i32)
                            .y(ev.root_y as i32)
                            .width(1)
                            .height(1),
                    )?;
                    conn.map_window(border_win)?;
                } else if ev.detail == 3 {
                    // Right click: cancel
                    break;
                }
            }
            Event::MotionNotify(ev) => {
                if let Some((sx, sy)) = start_pos {
                    let min_x = sx.min(ev.root_x);
                    let min_y = sy.min(ev.root_y);
                    let width = (sx - ev.root_x).unsigned_abs() as u32;
                    let height = (sy - ev.root_y).unsigned_abs() as u32;

                    if width > 0 && height > 0 {
                        conn.configure_window(
                            border_win,
                            &ConfigureWindowAux::new()
                                .x(min_x as i32)
                                .y(min_y as i32)
                                .width(width)
                                .height(height),
                        )?;
                    }
                }
            }
            Event::ButtonRelease(ev) => {
                if ev.detail == 1 {
                    if let Some((sx, sy)) = start_pos {
                        let min_x = sx.min(ev.root_x);
                        let min_y = sy.min(ev.root_y);
                        let width = (sx - ev.root_x).unsigned_abs() as u16;
                        let height = (sy - ev.root_y).unsigned_abs() as u16;

                        if width >= 10 && height >= 10 {
                            selected_rect = Some(CaptureRect {
                                x: min_x,
                                y: min_y,
                                width,
                                height,
                            });
                        }
                        break;
                    }
                }
            }
            Event::KeyPress(ev) => {
                // ESC keycode is 9 on standard Linux X11 systems
                if ev.detail == 9 {
                    println!("❌ 用户取消了框选");
                    break;
                }
            }
            _ => {}
        }
    }

    // Cleanup resources
    let _ = conn.destroy_window(border_win);
    let _ = conn.ungrab_keyboard(x11rb::CURRENT_TIME);
    let _ = conn.ungrab_pointer(x11rb::CURRENT_TIME);
    conn.flush()?;

    match selected_rect {
        Some(rect) => {
            println!(
                "✅ 选区已确认: 坐标 ({}, {}), 尺寸 {}x{}",
                rect.x, rect.y, rect.width, rect.height
            );
            Ok(rect)
        }
        None => Err("未选中有效区域或已被用户取消".into()),
    }
}
