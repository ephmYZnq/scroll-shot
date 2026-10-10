use crate::gui::font::draw_text;
use image::RgbaImage;
use std::error::Error;
use std::time::Instant;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt, CreateGCAux, CreateWindowAux, EventMask, ImageFormat,
    PropMode, StackMode, Window, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;

/// Interactive floating on-top pinned desktop window (Pin to Desktop)
pub fn run_pin_window<C: Connection>(
    conn: &C,
    screen_num: usize,
    image: RgbaImage,
    initial_x: i32,
    initial_y: i32,
) -> Result<(), Box<dyn Error>> {
    let setup = conn.setup();
    let screen = &setup.roots[screen_num];
    let root = screen.root;
    let screen_w = screen.width_in_pixels as i32;
    let screen_h = screen.height_in_pixels as i32;

    let base_w = image.width();
    let base_h = image.height();
    if base_w == 0 || base_h == 0 {
        return Ok(());
    }

    let mut scale: f32 = 1.0;
    let mut opacity: f32 = 1.0;

    let mut cur_w = base_w;
    let mut cur_h = base_h;

    let mut win_x = initial_x.clamp(0, (screen_w - cur_w as i32).max(0));
    let mut win_y = initial_y.clamp(0, (screen_h - cur_h as i32).max(0));

    let win = conn.generate_id()?;
    let event_mask = EventMask::EXPOSURE
        | EventMask::BUTTON_PRESS
        | EventMask::BUTTON_RELEASE
        | EventMask::POINTER_MOTION
        | EventMask::KEY_PRESS
        | EventMask::ENTER_WINDOW
        | EventMask::LEAVE_WINDOW;

    conn.create_window(
        screen.root_depth,
        win,
        root,
        win_x as i16,
        win_y as i16,
        cur_w as u16,
        cur_h as u16,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .override_redirect(1)
            .event_mask(event_mask),
    )?;

    let gc = conn.generate_id()?;
    conn.create_gc(gc, win, &CreateGCAux::new())?;

    // Set _NET_WM_STATE_ABOVE to inform compositors
    if let (Ok(wm_state), Ok(state_above), Ok(atom_type)) = (
        conn.intern_atom(false, b"_NET_WM_STATE"),
        conn.intern_atom(false, b"_NET_WM_STATE_ABOVE"),
        conn.intern_atom(false, b"ATOM"),
    ) {
        if let (Ok(wm_state_r), Ok(state_above_r), Ok(atom_type_r)) =
            (wm_state.reply(), state_above.reply(), atom_type.reply())
        {
            let _ = conn.change_property32(
                PropMode::REPLACE,
                win,
                wm_state_r.atom,
                atom_type_r.atom,
                &[state_above_r.atom],
            );
        }
    }

    conn.map_window(win)?;
    conn.configure_window(
        win,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    )?;
    conn.flush()?;

    let mut is_dragging = false;
    let mut drag_ptr_start = (0, 0);
    let mut drag_win_start = (win_x, win_y);
    let mut last_click: Option<(Instant, i32, i32)> = None;
    let mut is_hovered = false;

    // Allocate render buffer
    let mut render_buf = render_pinned_surface(&image, cur_w, cur_h, is_hovered);
    paint_to_x11(conn, win, gc, cur_w as u16, cur_h as u16, screen.root_depth, &render_buf)?;

    'event_loop: loop {
        let event = conn.wait_for_event()?;
        match event {
            Event::Expose(_) => {
                if !is_dragging {
                    conn.configure_window(
                        win,
                        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                    )?;
                    paint_to_x11(conn, win, gc, cur_w as u16, cur_h as u16, screen.root_depth, &render_buf)?;
                }
            }
            Event::EnterNotify(_) => {
                if !is_hovered && !is_dragging {
                    is_hovered = true;
                    render_buf = render_pinned_surface(&image, cur_w, cur_h, is_hovered);
                    paint_to_x11(conn, win, gc, cur_w as u16, cur_h as u16, screen.root_depth, &render_buf)?;
                }
            }
            Event::LeaveNotify(_) => {
                if is_hovered && !is_dragging {
                    is_hovered = false;
                    render_buf = render_pinned_surface(&image, cur_w, cur_h, is_hovered);
                    paint_to_x11(conn, win, gc, cur_w as u16, cur_h as u16, screen.root_depth, &render_buf)?;
                }
            }
            Event::ButtonPress(ev) => {
                let mx = ev.event_x as i32;
                let my = ev.event_y as i32;

                // Left button
                if ev.detail == 1 {
                    // Check close button in top right corner (24x24 px area)
                    if mx >= (cur_w as i32 - 26) && my <= 26 {
                        break 'event_loop;
                    }

                    // Check double click -> close (Snipaste style)
                    let now = Instant::now();
                    if let Some((last_t, lx, ly)) = last_click {
                        if now.duration_since(last_t).as_millis() < 300
                            && (mx - lx).abs() < 5
                            && (my - ly).abs() < 5
                        {
                            break 'event_loop;
                        }
                    }
                    last_click = Some((now, mx, my));

                    // Start window dragging
                    is_dragging = true;
                    drag_ptr_start = (ev.root_x as i32, ev.root_y as i32);
                    drag_win_start = (win_x, win_y);
                    conn.configure_window(
                        win,
                        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                    )?;
                    conn.flush()?;
                } else if ev.detail == 3 {
                    // Right click -> Close
                    break 'event_loop;
                } else if ev.detail == 2 {
                    // Middle click -> Reset scale to 1.0
                    scale = 1.0;
                    cur_w = base_w;
                    cur_h = base_h;
                    conn.configure_window(
                        win,
                        &ConfigureWindowAux::new()
                            .width(cur_w)
                            .height(cur_h),
                    )?;
                    render_buf = render_pinned_surface(&image, cur_w, cur_h, is_hovered);
                    paint_to_x11(conn, win, gc, cur_w as u16, cur_h as u16, screen.root_depth, &render_buf)?;
                } else if ev.detail == 4 {
                    // Wheel Up
                    let mask = u16::from(ev.state);
                    if mask & 0x04 != 0 || mask & 0x01 != 0 {
                        // Ctrl or Shift: Increase opacity
                        opacity = (opacity + 0.05).min(1.0);
                        let _ = set_window_opacity(conn, win, opacity);
                    } else {
                        // Zoom in
                        let old_w = cur_w;
                        let old_h = cur_h;
                        scale = (scale * 1.10).min(4.0);
                        cur_w = ((base_w as f32 * scale).round() as u32).max(24);
                        cur_h = ((base_h as f32 * scale).round() as u32).max(24);

                        // Center zoom around cursor
                        win_x -= (cur_w as i32 - old_w as i32) / 2;
                        win_y -= (cur_h as i32 - old_h as i32) / 2;

                        conn.configure_window(
                            win,
                            &ConfigureWindowAux::new()
                                .x(win_x)
                                .y(win_y)
                                .width(cur_w)
                                .height(cur_h),
                        )?;
                        render_buf = render_pinned_surface(&image, cur_w, cur_h, is_hovered);
                        paint_to_x11(conn, win, gc, cur_w as u16, cur_h as u16, screen.root_depth, &render_buf)?;
                    }
                } else if ev.detail == 5 {
                    // Wheel Down
                    let mask = u16::from(ev.state);
                    if mask & 0x04 != 0 || mask & 0x01 != 0 {
                        // Ctrl or Shift: Decrease opacity
                        opacity = (opacity - 0.05).max(0.20);
                        let _ = set_window_opacity(conn, win, opacity);
                    } else {
                        // Zoom out
                        let old_w = cur_w;
                        let old_h = cur_h;
                        scale = (scale / 1.10).max(0.15);
                        cur_w = ((base_w as f32 * scale).round() as u32).max(24);
                        cur_h = ((base_h as f32 * scale).round() as u32).max(24);

                        // Center zoom around cursor
                        win_x -= (cur_w as i32 - old_w as i32) / 2;
                        win_y -= (cur_h as i32 - old_h as i32) / 2;

                        conn.configure_window(
                            win,
                            &ConfigureWindowAux::new()
                                .x(win_x)
                                .y(win_y)
                                .width(cur_w)
                                .height(cur_h),
                        )?;
                        render_buf = render_pinned_surface(&image, cur_w, cur_h, is_hovered);
                        paint_to_x11(conn, win, gc, cur_w as u16, cur_h as u16, screen.root_depth, &render_buf)?;
                    }
                }
            }
            Event::ButtonRelease(ev) => {
                if ev.detail == 1 {
                    is_dragging = false;
                    let _ = conn.configure_window(
                        win,
                        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                    );
                    let _ = conn.flush();
                }
            }
            Event::MotionNotify(ev) => {
                if is_dragging {
                    let mut latest_rx = ev.root_x as i32;
                    let mut latest_ry = ev.root_y as i32;

                    // Drain all queued motion events to track the cursor position smoothly
                    while let Ok(Some(next_ev)) = conn.poll_for_event() {
                        match next_ev {
                            Event::MotionNotify(m) => {
                                latest_rx = m.root_x as i32;
                                latest_ry = m.root_y as i32;
                            }
                            Event::ButtonRelease(r) if r.detail == 1 => {
                                is_dragging = false;
                                break;
                            }
                            _ => {}
                        }
                    }

                    win_x = drag_win_start.0 + (latest_rx - drag_ptr_start.0);
                    win_y = drag_win_start.1 + (latest_ry - drag_ptr_start.1);
                    conn.configure_window(
                        win,
                        &ConfigureWindowAux::new().x(win_x).y(win_y),
                    )?;
                    conn.flush()?;
                }
            }
            Event::KeyPress(ev) => {
                // ESC = 9, 'q' = 24 -> Close
                if ev.detail == 9 || ev.detail == 24 {
                    break 'event_loop;
                }
                // 'c' = 54 -> Re-copy to clipboard
                if ev.detail == 54 {
                    let _ = crate::x11::clipboard::copy_to_clipboard(&image);
                }
                // '0' = 19 -> Reset zoom
                if ev.detail == 19 {
                    scale = 1.0;
                    cur_w = base_w;
                    cur_h = base_h;
                    conn.configure_window(
                        win,
                        &ConfigureWindowAux::new()
                            .width(cur_w)
                            .height(cur_h),
                    )?;
                    render_buf = render_pinned_surface(&image, cur_w, cur_h, is_hovered);
                    paint_to_x11(conn, win, gc, cur_w as u16, cur_h as u16, screen.root_depth, &render_buf)?;
                }
            }
            _ => {}
        }
    }

    let _ = conn.destroy_window(win);
    conn.flush()?;
    Ok(())
}

/// Set opacity via _NET_WM_WINDOW_OPACITY atom
pub fn set_window_opacity<C: Connection>(
    conn: &C,
    win: Window,
    opacity: f32,
) -> Result<(), Box<dyn Error>> {
    let opacity_atom = conn.intern_atom(false, b"_NET_WM_WINDOW_OPACITY")?.reply()?.atom;
    let cardinal_atom = conn.intern_atom(false, b"CARDINAL")?.reply()?.atom;
    let val = (opacity.clamp(0.1, 1.0) * 4294967295.0f32) as u32;
    conn.change_property32(
        PropMode::REPLACE,
        win,
        opacity_atom,
        cardinal_atom,
        &[val],
    )?;
    conn.flush()?;
    Ok(())
}

/// Scale and render pinned surface to X11 BGRA buffer with subtle border and optional close button
pub fn render_pinned_surface(
    image: &RgbaImage,
    target_w: u32,
    target_h: u32,
    hovered: bool,
) -> Vec<u8> {
    let mut buf = scale_image_bilinear(image, target_w, target_h);
    let w = target_w as usize;
    let h = target_h as usize;

    // Draw crisp 1px accent border around the pinned window so it stands out against identical backgrounds
    let border_col = [70, 130, 240, 255]; // Vivid soft blue border
    for x in 0..w {
        set_pixel_bgra(&mut buf, w, h, x, 0, border_col);
        set_pixel_bgra(&mut buf, w, h, x, h - 1, border_col);
    }
    for y in 0..h {
        set_pixel_bgra(&mut buf, w, h, 0, y, border_col);
        set_pixel_bgra(&mut buf, w, h, w - 1, y, border_col);
    }

    // Top-right corner close badge
    if hovered && w > 36 && h > 36 {
        let btn_x = w - 24;
        let btn_y = 4;
        for r in btn_y..(btn_y + 18) {
            for c in btn_x..(btn_x + 18) {
                set_pixel_bgra(&mut buf, w, h, c, r, [220, 50, 50, 230]);
            }
        }
        draw_text(&mut buf, w, h, btn_x + 5, btn_y + 5, "X", [255, 255, 255, 255]);
    }

    buf
}

/// High-performance bilinear image scaling directly producing X11 BGRA buffer
pub fn scale_image_bilinear(src: &RgbaImage, target_w: u32, target_h: u32) -> Vec<u8> {
    let src_w = src.width();
    let src_h = src.height();
    let mut out = vec![0u8; (target_w * target_h * 4) as usize];
    let src_raw = src.as_raw();

    if target_w == 0 || target_h == 0 || src_w == 0 || src_h == 0 {
        return out;
    }

    // Fast-path: 1:1 original scale
    if target_w == src_w && target_h == src_h {
        for (s, d) in src_raw.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
            d[0] = s[2]; // B
            d[1] = s[1]; // G
            d[2] = s[0]; // R
            d[3] = 255;
        }
        return out;
    }

    let x_ratio = if target_w > 1 {
        (src_w - 1) as f32 / (target_w - 1) as f32
    } else {
        0.0
    };
    let y_ratio = if target_h > 1 {
        (src_h - 1) as f32 / (target_h - 1) as f32
    } else {
        0.0
    };

    for y in 0..target_h {
        let src_y = y as f32 * y_ratio;
        let y0 = src_y.floor() as u32;
        let y1 = (y0 + 1).min(src_h - 1);
        let y_diff = src_y - y0 as f32;
        let inv_y_diff = 1.0 - y_diff;

        let row_offset = (y * target_w * 4) as usize;

        for x in 0..target_w {
            let src_x = x as f32 * x_ratio;
            let x0 = src_x.floor() as u32;
            let x1 = (x0 + 1).min(src_w - 1);
            let x_diff = src_x - x0 as f32;
            let inv_x_diff = 1.0 - x_diff;

            let idx00 = ((y0 * src_w + x0) * 4) as usize;
            let idx01 = ((y0 * src_w + x1) * 4) as usize;
            let idx10 = ((y1 * src_w + x0) * 4) as usize;
            let idx11 = ((y1 * src_w + x1) * 4) as usize;

            let dst_idx = row_offset + (x * 4) as usize;

            // Bilinear blend for R, G, B
            for c in 0..3 {
                let v00 = src_raw[idx00 + c] as f32;
                let v01 = src_raw[idx01 + c] as f32;
                let v10 = src_raw[idx10 + c] as f32;
                let v11 = src_raw[idx11 + c] as f32;

                let top = v00 * inv_x_diff + v01 * x_diff;
                let bot = v10 * inv_x_diff + v11 * x_diff;
                let val = (top * inv_y_diff + bot * y_diff) as u8;

                // c=0 is R (to d[2]), c=1 is G (to d[1]), c=2 is B (to d[0])
                out[dst_idx + (2 - c)] = val;
            }
            out[dst_idx + 3] = 255;
        }
    }

    out
}

#[inline(always)]
fn set_pixel_bgra(buf: &mut [u8], w: usize, h: usize, x: usize, y: usize, color: [u8; 4]) {
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

fn paint_to_x11<C: Connection>(
    conn: &C,
    win: Window,
    gc: x11rb::protocol::xproto::Gcontext,
    width: u16,
    height: u16,
    depth: u8,
    data: &[u8],
) -> Result<(), Box<dyn Error>> {
    let w = width as usize;
    let band_height = 80;

    let mut y = 0;
    while y < height as usize {
        let chunk_h = band_height.min(height as usize - y);
        let start = y * w * 4;
        let end = start + chunk_h * w * 4;
        conn.put_image(
            ImageFormat::Z_PIXMAP,
            win,
            gc,
            width,
            chunk_h as u16,
            0,
            y as i16,
            0,
            depth,
            &data[start..end],
        )?;
        y += chunk_h;
    }
    conn.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn test_scale_image_1to1() {
        let mut img = RgbaImage::new(10, 10);
        img.put_pixel(5, 5, Rgba([255, 128, 64, 255]));
        let buf = scale_image_bilinear(&img, 10, 10);
        let idx = (5 * 10 + 5) * 4;
        assert_eq!(buf[idx], 64);   // B
        assert_eq!(buf[idx + 1], 128); // G
        assert_eq!(buf[idx + 2], 255); // R
    }

    #[test]
    fn test_scale_image_upscale_and_downscale() {
        let img = RgbaImage::from_pixel(20, 20, Rgba([100, 150, 200, 255]));
        // Upscale to 40x40
        let up = scale_image_bilinear(&img, 40, 40);
        assert_eq!(up.len(), 40 * 40 * 4);
        assert_eq!(up[0], 200); // B
        assert_eq!(up[1], 150); // G
        assert_eq!(up[2], 100); // R

        // Downscale to 10x10
        let down = scale_image_bilinear(&img, 10, 10);
        assert_eq!(down.len(), 10 * 10 * 4);
        assert_eq!(down[0], 200);
    }
}
