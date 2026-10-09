use crate::cli::CaptureRect;
use crate::gui::font::draw_text;
use crate::stitcher::{find_vertical_shift, LongCanvas};
use crate::x11::capture::capture_screen_rect;
use image::RgbaImage;
use std::error::Error;
use std::thread::sleep;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt, CreateGCAux, CreateWindowAux, EventMask, ImageFormat, Window, WindowClass,
};
use x11rb::protocol::Event;

/// Interactive manual scroll recording session
pub fn record_manual_scroll<C: Connection>(
    conn: &C,
    screen_num: usize,
    rect: &CaptureRect,
) -> Result<Option<RgbaImage>, Box<dyn Error>> {
    let setup = conn.setup();
    let screen = &setup.roots[screen_num];
    let root = screen.root;
    let screen_w = screen.width_in_pixels;
    let screen_h = screen.height_in_pixels;

    // Capture initial frame
    let frame_0 = capture_screen_rect(conn, root, rect)?;
    let mut canvas = LongCanvas::new(&frame_0);
    let mut prev_frame = frame_0;

    // Create a floating status bar window outside the capture rect
    let bar_w = 330;
    let bar_h = 44;

    let bar_x = (rect.x as i32 + ((rect.width as i32 - bar_w) / 2))
        .clamp(10, (screen_w as i32 - bar_w - 10).max(10));

    let bar_y = if rect.y > 60 {
        rect.y as i32 - bar_h - 12
    } else if (rect.y as i32 + rect.height as i32 + bar_h + 20) < screen_h as i32 {
        rect.y as i32 + rect.height as i32 + 12
    } else {
        12
    };

    let bar_win = conn.generate_id()?;
    let event_mask = EventMask::EXPOSURE | EventMask::BUTTON_PRESS | EventMask::KEY_PRESS;

    conn.create_window(
        screen.root_depth,
        bar_win,
        root,
        bar_x as i16,
        bar_y as i16,
        bar_w as u16,
        bar_h as u16,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .override_redirect(1)
            .event_mask(event_mask),
    )?;

    let gc = conn.generate_id()?;
    conn.create_gc(gc, bar_win, &CreateGCAux::new())?;

    conn.map_window(bar_win)?;
    conn.flush()?;

    // Status bar pixel buffer
    let mut bar_buf = vec![0u8; (bar_w as usize) * (bar_h as usize) * 4];

    render_status_bar(&mut bar_buf, bar_w as usize, bar_h as usize, canvas.height);
    paint_bar(conn, bar_win, gc, bar_w as u16, bar_h as u16, screen.root_depth, &bar_buf)?;

    let finish_btn_x = bar_w - 120;
    let cancel_btn_x = bar_w - 40;

    let user_completed;

    'record_loop: loop {
        // Poll for user interaction (non-blocking)
        while let Some(event) = conn.poll_for_event()? {
            match event {
                Event::Expose(_) => {
                    paint_bar(conn, bar_win, gc, bar_w as u16, bar_h as u16, screen.root_depth, &bar_buf)?;
                }
                Event::ButtonPress(ev) => {
                    let mx = ev.event_x as i32;
                    let my = ev.event_y as i32;
                    if my >= 6 && my <= 38 {
                        if mx >= finish_btn_x && mx < finish_btn_x + 75 {
                            // Clicked Finish
                            user_completed = true;
                            break 'record_loop;
                        } else if mx >= cancel_btn_x && mx < cancel_btn_x + 32 {
                            // Clicked Cancel
                            user_completed = false;
                            break 'record_loop;
                        }
                    }
                }
                Event::KeyPress(ev) => {
                    // ESC = 9
                    if ev.detail == 9 {
                        user_completed = false;
                        break 'record_loop;
                    }
                    // Enter = 36, Space = 65
                    if ev.detail == 36 || ev.detail == 65 {
                        user_completed = true;
                        break 'record_loop;
                    }
                }
                _ => {}
            }
        }

        // Capture current state of the selection area
        sleep(Duration::from_millis(35));
        let curr_frame = capture_screen_rect(conn, root, rect)?;

        // Detect displacement from manual scrolling
        let match_res = find_vertical_shift(&prev_frame, &curr_frame, Some(220));

        if match_res.delta_y > 0 {
            // Append newly revealed rows
            let start_y = (rect.height as u32).saturating_sub(match_res.delta_y);
            canvas.append_rows(&curr_frame, start_y, match_res.delta_y);
            prev_frame = curr_frame;

            // Update floating status bar with new total height
            render_status_bar(&mut bar_buf, bar_w as usize, bar_h as usize, canvas.height);
            paint_bar(conn, bar_win, gc, bar_w as u16, bar_h as u16, screen.root_depth, &bar_buf)?;
        }
    }

    // Cleanup floating window
    let _ = conn.destroy_window(bar_win);
    conn.flush()?;

    if user_completed {
        Ok(canvas.into_image())
    } else {
        Ok(None)
    }
}

fn render_status_bar(buf: &mut [u8], w: usize, h: usize, current_height: u32) {
    // Background (Dark charcoal: RGB 26, 28, 32)
    for pixel in buf.chunks_exact_mut(4) {
        pixel[0] = 32; // B
        pixel[1] = 28; // G
        pixel[2] = 26; // R
        pixel[3] = 255;
    }

    // Border (Accent gray: RGB 75, 80, 90)
    for c in 0..w {
        set_pixel(buf, w, h, c, 0, [75, 80, 90, 255]);
        set_pixel(buf, w, h, c, h - 1, [75, 80, 90, 255]);
    }
    for r in 0..h {
        set_pixel(buf, w, h, 0, r, [75, 80, 90, 255]);
        set_pixel(buf, w, h, w - 1, r, [75, 80, 90, 255]);
    }

    // Recording indicator (Red dot)
    let dot_cx = 16;
    let dot_cy = h / 2;
    for dy in -4..=4 {
        for dx in -4..=4 {
            if dx * dx + dy * dy <= 16 {
                set_pixel(buf, w, h, (dot_cx as isize + dx) as usize, (dot_cy as isize + dy) as usize, [235, 40, 40, 255]);
            }
        }
    }

    // Label: "Rec | {height}px"
    let status_text = format!("Rec: {} px", current_height);
    draw_text(buf, w, h, 28, (h.saturating_sub(8)) / 2, &status_text, [240, 240, 240, 255]);

    // Finish button [ Done ] (Green background: RGB 35, 125, 60)
    let fb_x = w - 120;
    let fb_y = 6;
    let fb_w = 75;
    let fb_h = 32;

    for r in fb_y..(fb_y + fb_h) {
        for c in fb_x..(fb_x + fb_w) {
            set_pixel(buf, w, h, c, r, [35, 125, 60, 255]);
        }
    }
    draw_text(buf, w, h, fb_x + 18, fb_y + 12, "Done", [255, 255, 255, 255]);

    // Cancel button [ X ] (Gray background: RGB 65, 70, 78)
    let cb_x = w - 38;
    let cb_y = 6;
    let cb_w = 28;
    let cb_h = 32;

    for r in cb_y..(cb_y + cb_h) {
        for c in cb_x..(cb_x + cb_w) {
            set_pixel(buf, w, h, c, r, [65, 70, 78, 255]);
        }
    }
    draw_text(buf, w, h, cb_x + 10, cb_y + 12, "X", [220, 220, 220, 255]);
}

#[inline(always)]
fn set_pixel(buf: &mut [u8], w: usize, h: usize, x: usize, y: usize, color: [u8; 4]) {
    if x < w && y < h {
        let idx = (y * w + x) * 4;
        if idx + 3 < buf.len() {
            buf[idx] = color[2];     // B
            buf[idx + 1] = color[1]; // G
            buf[idx + 2] = color[0]; // R
            buf[idx + 3] = color[3]; // A
        }
    }
}

fn paint_bar<C: Connection>(
    conn: &C,
    win: Window,
    gc: x11rb::protocol::xproto::Gcontext,
    width: u16,
    height: u16,
    depth: u8,
    data: &[u8],
) -> Result<(), Box<dyn Error>> {
    conn.put_image(
        ImageFormat::Z_PIXMAP,
        win,
        gc,
        width,
        height,
        0,
        0,
        0,
        depth,
        data,
    )?;
    conn.flush()?;
    Ok(())
}
