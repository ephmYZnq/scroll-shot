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
    ConfigureWindowAux, ConnectionExt, CreateGCAux, CreateWindowAux, EventMask, Gcontext,
    ImageFormat, Window, WindowClass,
};
use x11rb::protocol::Event;

/// Preview panel maximum width (fits right of capture area, or screen edge)
const PREVIEW_W: u16 = 220;

/// Interactive manual scroll recording session with live thumbnail preview
pub fn record_manual_scroll<C: Connection>(
    conn: &C,
    screen_num: usize,
    rect: &CaptureRect,
    initial_frame: Option<RgbaImage>,
) -> Result<Option<RgbaImage>, Box<dyn Error>> {
    let setup = conn.setup();
    let screen = &setup.roots[screen_num];
    let root = screen.root;
    let screen_w = screen.width_in_pixels;
    let screen_h = screen.height_in_pixels;

    // Use initial pristine frame from freeze overlay if available, else capture fresh
    let frame_0 = match initial_frame {
        Some(f) => f,
        None => capture_screen_rect(conn, root, rect)?,
    };
    let mut canvas = LongCanvas::new(&frame_0);
    let mut prev_frame = frame_0;

    // === Create floating status bar ===
    let bar_w: i32 = 330;
    let bar_h: i32 = 44;

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
            .event_mask(EventMask::EXPOSURE | EventMask::BUTTON_PRESS | EventMask::KEY_PRESS),
    )?;

    let bar_gc = conn.generate_id()?;
    conn.create_gc(bar_gc, bar_win, &CreateGCAux::new())?;

    // === Create floating live preview window ===
    // Place preview to the right of capture rect, or to the left if no space
    let preview_w = PREVIEW_W as i32;
    // Preview height starts equal to capture rect height, will grow dynamically
    let initial_preview_h = (rect.height as i32).min(screen_h as i32 - 40);

    let preview_x = if rect.x as i32 + rect.width as i32 + preview_w + 20 < screen_w as i32 {
        rect.x as i32 + rect.width as i32 + 12
    } else if rect.x as i32 - preview_w - 12 > 0 {
        rect.x as i32 - preview_w - 12
    } else {
        (screen_w as i32 - preview_w - 10).max(10)
    };
    let preview_y = (rect.y as i32).clamp(10, (screen_h as i32 - initial_preview_h - 10).max(10));

    let preview_win = conn.generate_id()?;
    conn.create_window(
        screen.root_depth,
        preview_win,
        root,
        preview_x as i16,
        preview_y as i16,
        preview_w as u16,
        initial_preview_h as u16,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .override_redirect(1)
            .event_mask(EventMask::EXPOSURE),
    )?;

    let preview_gc = conn.generate_id()?;
    conn.create_gc(preview_gc, preview_win, &CreateGCAux::new())?;

    // Map both windows
    conn.map_window(bar_win)?;
    conn.map_window(preview_win)?;
    conn.flush()?;

    // Status bar pixel buffer
    let mut bar_buf = vec![0u8; (bar_w as usize) * (bar_h as usize) * 4];
    render_status_bar(&mut bar_buf, bar_w as usize, bar_h as usize, canvas.height);
    paint_buf(conn, bar_win, bar_gc, bar_w as u16, bar_h as u16, screen.root_depth, &bar_buf)?;

    // Initial preview render
    let mut cur_preview_h = initial_preview_h as u16;
    render_and_paint_preview(
        conn, preview_win, preview_gc, screen.root_depth,
        preview_w as u16, &mut cur_preview_h, screen_h,
        &canvas,
    )?;

    let finish_btn_x = bar_w - 120;
    let cancel_btn_x = bar_w - 40;

    let user_completed;

    'record_loop: loop {
        // Poll for user interaction (non-blocking)
        while let Some(event) = conn.poll_for_event()? {
            match event {
                Event::Expose(ev) => {
                    if ev.window == bar_win {
                        paint_buf(conn, bar_win, bar_gc, bar_w as u16, bar_h as u16, screen.root_depth, &bar_buf)?;
                    }
                    // Preview expose is handled below after re-render
                }
                Event::ButtonPress(ev) => {
                    if ev.event == bar_win {
                        let mx = ev.event_x as i32;
                        let my = ev.event_y as i32;
                        if my >= 6 && my <= 38 {
                            if mx >= finish_btn_x && mx < finish_btn_x + 75 {
                                user_completed = true;
                                break 'record_loop;
                            } else if mx >= cancel_btn_x && mx < cancel_btn_x + 32 {
                                user_completed = false;
                                break 'record_loop;
                            }
                        }
                    }
                }
                Event::KeyPress(ev) => {
                    if ev.detail == 9 {
                        user_completed = false;
                        break 'record_loop;
                    }
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
            let start_y = (rect.height as u32).saturating_sub(match_res.delta_y);
            canvas.append_rows(&curr_frame, start_y, match_res.delta_y);
            prev_frame = curr_frame;

            // Update status bar
            render_status_bar(&mut bar_buf, bar_w as usize, bar_h as usize, canvas.height);
            paint_buf(conn, bar_win, bar_gc, bar_w as u16, bar_h as u16, screen.root_depth, &bar_buf)?;

            // Update live preview thumbnail
            render_and_paint_preview(
                conn, preview_win, preview_gc, screen.root_depth,
                preview_w as u16, &mut cur_preview_h, screen_h,
                &canvas,
            )?;
        }
    }

    // Cleanup
    let _ = conn.destroy_window(bar_win);
    let _ = conn.destroy_window(preview_win);
    conn.flush()?;

    if user_completed {
        // Allow desktop window 80ms to settle any ongoing smooth scrolling animation
        sleep(Duration::from_millis(80));

        // Capture final static pristine frame
        if let Ok(final_crisp_frame) = capture_screen_rect(conn, root, rect) {
            let match_res = find_vertical_shift(&prev_frame, &final_crisp_frame, Some(240));
            if match_res.delta_y > 0 {
                let start_y = (rect.height as u32).saturating_sub(match_res.delta_y);
                canvas.append_rows(&final_crisp_frame, start_y, match_res.delta_y);
            }
            // Overwrite the bottom viewport of the canvas with this 100% sharp, static frame
            canvas.overwrite_bottom_rows(&final_crisp_frame, 0, rect.height as u32);
        }

        Ok(canvas.into_image())
    } else {
        Ok(None)
    }
}

/// Render a scaled-down thumbnail of the entire canvas into the preview window
fn render_and_paint_preview<C: Connection>(
    conn: &C,
    win: Window,
    gc: Gcontext,
    depth: u8,
    preview_w: u16,
    cur_h: &mut u16,
    screen_h: u16,
    canvas: &LongCanvas,
) -> Result<(), Box<dyn Error>> {
    if canvas.width == 0 || canvas.height == 0 {
        return Ok(());
    }

    // Calculate thumbnail dimensions maintaining aspect ratio
    let scale = (preview_w as f64 - 8.0) / canvas.width as f64; // 4px border each side
    let thumb_w = ((canvas.width as f64) * scale) as usize;
    let thumb_h = ((canvas.height as f64) * scale) as usize;

    // Total preview panel height = thumb + header + padding
    let header_h = 20;
    let padding = 8;
    let panel_h = (thumb_h + header_h + padding * 2).min((screen_h as usize).saturating_sub(40));
    let visible_thumb_h = panel_h.saturating_sub(header_h + padding * 2);

    // Resize window if needed
    let new_h = panel_h as u16;
    if new_h != *cur_h {
        conn.configure_window(win, &ConfigureWindowAux::new().height(new_h as u32))?;
        *cur_h = new_h;
    }

    let pw = preview_w as usize;
    let ph = panel_h;
    let mut buf = vec![0u8; pw * ph * 4];

    // Dark background
    for pixel in buf.chunks_exact_mut(4) {
        pixel[0] = 28; pixel[1] = 26; pixel[2] = 24; pixel[3] = 255;
    }

    // Border
    for c in 0..pw {
        set_pixel(&mut buf, pw, ph, c, 0, [75, 80, 90, 255]);
        set_pixel(&mut buf, pw, ph, c, ph.saturating_sub(1), [75, 80, 90, 255]);
    }
    for r in 0..ph {
        set_pixel(&mut buf, pw, ph, 0, r, [75, 80, 90, 255]);
        set_pixel(&mut buf, pw, ph, pw.saturating_sub(1), r, [75, 80, 90, 255]);
    }

    // Header label
    let header_text = format!("Preview {}x{}", canvas.width, canvas.height);
    draw_text(&mut buf, pw, ph, 6, 6, &header_text, [200, 200, 200, 255]);

    // Render scaled thumbnail using nearest-neighbor sampling
    // Show the bottom portion of the canvas if it overflows preview area
    let canvas_row_bytes = canvas.width as usize * 4;
    let thumb_offset_x = (pw.saturating_sub(thumb_w)) / 2;
    let thumb_offset_y = header_h + padding;

    // If thumbnail is taller than visible area, show bottom portion
    let src_start_y = if thumb_h > visible_thumb_h {
        // Map visible area to the bottom of the canvas
        let skip_thumb_rows = thumb_h - visible_thumb_h;
        ((skip_thumb_rows as f64) / scale) as u32
    } else {
        0
    };

    let rows_to_draw = visible_thumb_h.min(thumb_h);
    for ty in 0..rows_to_draw {
        let canvas_y = if thumb_h > visible_thumb_h {
            // Mapping from visible thumbnail row to canvas row
            src_start_y + ((ty as f64) / scale) as u32
        } else {
            ((ty as f64) / scale) as u32
        };

        if canvas_y >= canvas.height {
            break;
        }

        let src_row_start = (canvas_y as usize) * canvas_row_bytes;
        for tx in 0..thumb_w {
            let canvas_x = ((tx as f64) / scale) as u32;
            if canvas_x >= canvas.width {
                break;
            }
            let src_idx = src_row_start + (canvas_x as usize) * 4;
            if src_idx + 3 < canvas.buffer.len() {
                let px = thumb_offset_x + tx;
                let py = thumb_offset_y + ty;
                if px < pw && py < ph {
                    let dst_idx = (py * pw + px) * 4;
                    if dst_idx + 3 < buf.len() {
                        // Canvas buffer is RGBA, X11 wants BGRX
                        buf[dst_idx]     = canvas.buffer[src_idx + 2]; // B
                        buf[dst_idx + 1] = canvas.buffer[src_idx + 1]; // G
                        buf[dst_idx + 2] = canvas.buffer[src_idx];     // R
                        buf[dst_idx + 3] = 255;
                    }
                }
            }
        }
    }

    // Draw a thin white border around the thumbnail area
    let tb_x = thumb_offset_x.saturating_sub(1);
    let tb_y = thumb_offset_y.saturating_sub(1);
    let tb_w = thumb_w + 2;
    let tb_h = rows_to_draw + 2;
    for c in tb_x..(tb_x + tb_w).min(pw) {
        set_pixel(&mut buf, pw, ph, c, tb_y, [110, 115, 125, 255]);
        set_pixel(&mut buf, pw, ph, c, (tb_y + tb_h).min(ph - 1), [110, 115, 125, 255]);
    }
    for r in tb_y..(tb_y + tb_h).min(ph) {
        set_pixel(&mut buf, pw, ph, tb_x, r, [110, 115, 125, 255]);
        set_pixel(&mut buf, pw, ph, (tb_x + tb_w).min(pw - 1), r, [110, 115, 125, 255]);
    }

    // Paint to window in bands
    let band_h = 60;
    let mut y = 0;
    while y < ph {
        let chunk_h = band_h.min(ph - y);
        let start = y * pw * 4;
        let end = start + chunk_h * pw * 4;
        conn.put_image(
            ImageFormat::Z_PIXMAP, win, gc,
            pw as u16, chunk_h as u16,
            0, y as i16, 0, depth,
            &buf[start..end],
        )?;
        y += chunk_h;
    }
    conn.flush()?;

    Ok(())
}

fn render_status_bar(buf: &mut [u8], w: usize, h: usize, current_height: u32) {
    // Background
    for pixel in buf.chunks_exact_mut(4) {
        pixel[0] = 32; pixel[1] = 28; pixel[2] = 26; pixel[3] = 255;
    }

    // Border
    for c in 0..w {
        set_pixel(buf, w, h, c, 0, [75, 80, 90, 255]);
        set_pixel(buf, w, h, c, h - 1, [75, 80, 90, 255]);
    }
    for r in 0..h {
        set_pixel(buf, w, h, 0, r, [75, 80, 90, 255]);
        set_pixel(buf, w, h, w - 1, r, [75, 80, 90, 255]);
    }

    // Recording indicator (Red dot)
    let dot_cx: isize = 16;
    let dot_cy = (h / 2) as isize;
    for dy in -4..=4_isize {
        for dx in -4..=4_isize {
            if dx * dx + dy * dy <= 16 {
                set_pixel(buf, w, h, (dot_cx + dx) as usize, (dot_cy + dy) as usize, [235, 40, 40, 255]);
            }
        }
    }

    // Label
    let status_text = format!("Rec: {} px", current_height);
    draw_text(buf, w, h, 28, (h.saturating_sub(8)) / 2, &status_text, [240, 240, 240, 255]);

    // Finish button [Done]
    let fb_x = w - 120;
    let fb_y = 6;
    for r in fb_y..(fb_y + 32) {
        for c in fb_x..(fb_x + 75) {
            set_pixel(buf, w, h, c, r, [35, 125, 60, 255]);
        }
    }
    draw_text(buf, w, h, fb_x + 18, fb_y + 12, "Done", [255, 255, 255, 255]);

    // Cancel button [X]
    let cb_x = w - 38;
    let cb_y = 6;
    for r in cb_y..(cb_y + 32) {
        for c in cb_x..(cb_x + 28) {
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
            buf[idx] = color[2];
            buf[idx + 1] = color[1];
            buf[idx + 2] = color[0];
            buf[idx + 3] = color[3];
        }
    }
}

fn paint_buf<C: Connection>(
    conn: &C,
    win: Window,
    gc: Gcontext,
    width: u16,
    height: u16,
    depth: u8,
    data: &[u8],
) -> Result<(), Box<dyn Error>> {
    conn.put_image(
        ImageFormat::Z_PIXMAP, win, gc,
        width, height, 0, 0, 0, depth, data,
    )?;
    conn.flush()?;
    Ok(())
}
