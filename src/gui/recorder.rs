use crate::cli::CaptureRect;
use crate::gui::font::draw_text;
use crate::stitcher::{find_vertical_shift, frames_settled, LongCanvas};
use crate::x11::capture::capture_screen_rect;
use image::RgbaImage;
use std::error::Error;
use std::thread::sleep;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt, CreateGCAux, CreateWindowAux, EventMask, Gcontext,
    ImageFormat, PropMode, StackMode, Window, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollOutcome {
    Done,
    Ocr,
    Pin,
    Edit,
}

/// Preview panel maximum width (fits right of capture area, or screen edge)
const PREVIEW_W: u16 = 220;

/// Interactive manual scroll recording session with live thumbnail preview
pub fn record_manual_scroll<C: Connection>(
    conn: &C,
    screen_num: usize,
    rect: &CaptureRect,
    initial_frame: Option<RgbaImage>,
) -> Result<Option<(RgbaImage, ScrollOutcome)>, Box<dyn Error>> {
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
    let mut keyframe = frame_0.clone();
    let mut prev_sample = frame_0;
    let mut still_ticks = 0;

    // === Create floating status bar ===
    let bar_w: i32 = 470;
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

    if let (Ok(wm_state), Ok(state_above), Ok(atom_type)) = (
        conn.intern_atom(false, b"_NET_WM_STATE"),
        conn.intern_atom(false, b"_NET_WM_STATE_ABOVE"),
        conn.intern_atom(false, b"ATOM"),
    ) {
        if let (Ok(wm_state_r), Ok(state_above_r), Ok(atom_type_r)) =
            (wm_state.reply(), state_above.reply(), atom_type.reply())
        {
            for &w in &[bar_win, preview_win] {
                let _ = conn.change_property32(
                    PropMode::REPLACE,
                    w,
                    wm_state_r.atom,
                    atom_type_r.atom,
                    &[state_above_r.atom],
                );
            }
        }
    }

    // Map both windows
    conn.map_window(bar_win)?;
    conn.map_window(preview_win)?;
    let _ = conn.configure_window(
        bar_win,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    );
    let _ = conn.configure_window(
        preview_win,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    );
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

    let finish_btn_x = bar_w - 276; // [Done]
    let ocr_btn_x = bar_w - 212;    // [OCR]
    let pin_btn_x = bar_w - 154;    // [Pin]
    let edit_btn_x = bar_w - 96;    // [Edit]
    let cancel_btn_x = bar_w - 34;  // [X]

    let mut user_completed: Option<ScrollOutcome> = None;

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
                            if mx >= finish_btn_x && mx < finish_btn_x + 58 {
                                user_completed = Some(ScrollOutcome::Done);
                                break 'record_loop;
                            } else if mx >= ocr_btn_x && mx < ocr_btn_x + 52 {
                                user_completed = Some(ScrollOutcome::Ocr);
                                break 'record_loop;
                            } else if mx >= pin_btn_x && mx < pin_btn_x + 52 {
                                user_completed = Some(ScrollOutcome::Pin);
                                break 'record_loop;
                            } else if mx >= edit_btn_x && mx < edit_btn_x + 56 {
                                user_completed = Some(ScrollOutcome::Edit);
                                break 'record_loop;
                            } else if mx >= cancel_btn_x && mx < cancel_btn_x + 28 {
                                break 'record_loop;
                            }
                        }
                    }
                }
                Event::KeyPress(ev) => {
                    if ev.detail == 9 {
                        break 'record_loop;
                    }
                    if ev.detail == 36 || ev.detail == 65 {
                        // Enter or Space -> Done
                        user_completed = Some(ScrollOutcome::Done);
                        break 'record_loop;
                    }
                    if ev.detail == 32 {
                        // 'o' -> OCR
                        user_completed = Some(ScrollOutcome::Ocr);
                        break 'record_loop;
                    }
                    if ev.detail == 33 {
                        // 'p' -> Pin
                        user_completed = Some(ScrollOutcome::Pin);
                        break 'record_loop;
                    }
                    if ev.detail == 26 {
                        // 'e' -> Edit
                        user_completed = Some(ScrollOutcome::Edit);
                        break 'record_loop;
                    }
                }
                _ => {}
            }
        }

        // Capture current state of the selection area
        sleep(Duration::from_millis(25));
        let curr_frame = capture_screen_rect(conn, root, rect)?;

        // Track displacement from keyframe
        let max_search = (rect.height as u32 * 75 / 100).min(rect.height as u32 - 40);
        let match_res = find_vertical_shift(&keyframe, &curr_frame, Some(max_search));

        // Check if screen is currently stationary (settled)
        let is_still = frames_settled(&prev_sample, &curr_frame);
        if is_still {
            still_ticks += 1;
        } else {
            still_ticks = 0;
        }
        prev_sample = curr_frame.clone();

        // Stitch a keyframe when:
        // 1. Content has moved a substantial step (>= min_settled_shift) AND the screen has paused/settled for >= 2 ticks (~50ms)
        // OR 2. Content has moved more than 55% of viewport height (safety trigger to avoid losing overlap during rapid continuous scrolling)
        let min_settled_shift = (rect.height as u32 * 8 / 100).clamp(25, 45);
        let max_overlap_shift = (rect.height as u32 * 55 / 100).max(60);
        let should_stitch = (match_res.delta_y >= min_settled_shift && still_ticks >= 2)
            || (match_res.delta_y >= max_overlap_shift);

        if should_stitch {
            canvas.splice_keyframe(&curr_frame, match_res.delta_y);
            keyframe = curr_frame;
            still_ticks = 0;

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

    // Cleanup floating windows
    let _ = conn.destroy_window(bar_win);
    let _ = conn.destroy_window(preview_win);
    conn.flush()?;

    if let Some(outcome) = user_completed {
        // Allow desktop window 80ms to settle any ongoing smooth scrolling animation
        sleep(Duration::from_millis(80));

        // Capture final static pristine frame
        if let Ok(final_crisp_frame) = capture_screen_rect(conn, root, rect) {
            let max_search = (rect.height as u32 * 75 / 100).min(rect.height as u32 - 40);
            let match_res = find_vertical_shift(&keyframe, &final_crisp_frame, Some(max_search));
            if match_res.delta_y > 5 {
                canvas.splice_keyframe(&final_crisp_frame, match_res.delta_y);
            }
        }

        if let Some(img) = canvas.into_image() {
            Ok(Some((img, outcome)))
        } else {
            Ok(None)
        }
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

    // Finish button [Done] (Green)
    let fb_x = w - 276;
    let fb_y = 6;
    for r in fb_y..(fb_y + 32) {
        for c in fb_x..(fb_x + 58) {
            set_pixel(buf, w, h, c, r, [35, 125, 60, 255]);
        }
    }
    draw_text(buf, w, h, fb_x + 13, fb_y + 12, "Done", [255, 255, 255, 255]);

    // OCR button [OCR] (Amber/Orange)
    let ob_x = w - 212;
    let ob_y = 6;
    for r in ob_y..(ob_y + 32) {
        for c in ob_x..(ob_x + 52) {
            set_pixel(buf, w, h, c, r, [185, 90, 20, 255]);
        }
    }
    draw_text(buf, w, h, ob_x + 14, ob_y + 12, "OCR", [255, 255, 255, 255]);

    // Pin button [Pin] (Purple)
    let pb_x = w - 154;
    let pb_y = 6;
    for r in pb_y..(pb_y + 32) {
        for c in pb_x..(pb_x + 52) {
            set_pixel(buf, w, h, c, r, [100, 60, 165, 255]);
        }
    }
    draw_text(buf, w, h, pb_x + 13, pb_y + 12, "Pin", [255, 255, 255, 255]);

    // Edit button [Edit] (Blue)
    let eb_x = w - 96;
    let eb_y = 6;
    for r in eb_y..(eb_y + 32) {
        for c in eb_x..(eb_x + 56) {
            set_pixel(buf, w, h, c, r, [45, 100, 215, 255]);
        }
    }
    draw_text(buf, w, h, eb_x + 12, eb_y + 12, "Edit", [255, 255, 255, 255]);

    // Cancel button [X] (Gray)
    let cb_x = w - 34;
    let cb_y = 6;
    for r in cb_y..(cb_y + 32) {
        for c in cb_x..(cb_x + 26) {
            set_pixel(buf, w, h, c, r, [65, 70, 78, 255]);
        }
    }
    draw_text(buf, w, h, cb_x + 9, cb_y + 12, "X", [220, 220, 220, 255]);
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
