use crate::cli::CaptureRect;
use crate::gui::font::draw_text;
use image::RgbaImage;
use std::error::Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt, CreateGCAux, CreateWindowAux, EventMask, Gcontext, ImageFormat, Window,
    WindowClass,
};
use x11rb::protocol::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserAction {
    SingleShot(CaptureRect),
    Pin(CaptureRect),
    Edit(CaptureRect),
    ScrollShot(CaptureRect),
    Ocr(CaptureRect),
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragMode {
    None,
    Creating(i16, i16),
    Moving { offset_x: i16, offset_y: i16 },
    ResizingTopLeft,
    ResizingTopRight,
    ResizingBottomLeft,
    ResizingBottomRight,
}

struct ToolbarButton {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    label: &'static str,
    action: UserAction,
}

/// Show interactive full-screen freeze overlay with resizable selection box and toolbar
pub fn run_interactive_overlay<C: Connection>(
    conn: &C,
    screen_num: usize,
) -> Result<(UserAction, Option<RgbaImage>), Box<dyn Error>> {
    let setup = conn.setup();
    let screen = &setup.roots[screen_num];
    let root = screen.root;
    let screen_w = screen.width_in_pixels;
    let screen_h = screen.height_in_pixels;

    // 1. Freeze screen: Capture the entire root screen upfront
    let orig_screen_reply = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            root,
            0,
            0,
            screen_w,
            screen_h,
            !0,
        )?
        .reply()?;

    let mut orig_screen = vec![0u8; (screen_w as usize) * (screen_h as usize) * 4];
    let src_bytes = &orig_screen_reply.data;
    let len = orig_screen.len().min(src_bytes.len());
    orig_screen[..len].copy_from_slice(&src_bytes[..len]);

    // Create darkened background buffer (35% brightness veil)
    let mut dark_screen = orig_screen.clone();
    for pixel in dark_screen.chunks_exact_mut(4) {
        pixel[0] = ((pixel[0] as u32 * 35) / 100) as u8; // Blue
        pixel[1] = ((pixel[1] as u32 * 35) / 100) as u8; // Green
        pixel[2] = ((pixel[2] as u32 * 35) / 100) as u8; // Red
    }

    // 2. Create fullscreen overlay window
    let win = conn.generate_id()?;
    let event_mask = EventMask::EXPOSURE
        | EventMask::BUTTON_PRESS
        | EventMask::BUTTON_RELEASE
        | EventMask::POINTER_MOTION
        | EventMask::KEY_PRESS;

    conn.create_window(
        screen.root_depth,
        win,
        root,
        0,
        0,
        screen_w,
        screen_h,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .override_redirect(1)
            .event_mask(event_mask),
    )?;

    let gc = conn.generate_id()?;
    conn.create_gc(gc, win, &CreateGCAux::new())?;

    conn.map_window(win)?;
    conn.flush()?;

    // Initial default selection box (centered 600x400)
    let init_w = 640.min(screen_w.saturating_sub(100));
    let init_h = 420.min(screen_h.saturating_sub(100));
    let init_x = ((screen_w.saturating_sub(init_w)) / 2) as i16;
    let init_y = ((screen_h.saturating_sub(init_h)) / 2) as i16;

    let mut current_rect = CaptureRect {
        x: init_x,
        y: init_y,
        width: init_w,
        height: init_h,
    };

    let mut drag_mode = DragMode::None;
    let mut frame_buf = dark_screen.clone();

    // Render loop
    let result;

    // Initial draw
    render_composite(
        &orig_screen,
        &dark_screen,
        &mut frame_buf,
        screen_w as usize,
        screen_h as usize,
        Some(&current_rect),
    );
    paint_to_x11(conn, win, gc, screen_w, screen_h, screen.root_depth, &frame_buf)?;

    'event_loop: loop {
        conn.flush()?;
        let event = conn.wait_for_event()?;

        match event {
            Event::Expose(_) => {
                paint_to_x11(conn, win, gc, screen_w, screen_h, screen.root_depth, &frame_buf)?;
            }
            Event::ButtonPress(ev) => {
                if ev.detail == 1 {
                    let mx = ev.event_x as i32;
                    let my = ev.event_y as i32;

                    // 1. Check if clicking floating toolbar buttons
                    let buttons = get_toolbar_buttons(&current_rect, screen_w as i32, screen_h as i32);
                    for btn in buttons {
                        if mx >= btn.x && mx < btn.x + btn.width && my >= btn.y && my < btn.y + btn.height {
                            result = btn.action;
                            break 'event_loop;
                        }
                    }

                    // 2. Check handles (16x16 click area)
                    let rx = current_rect.x as i32;
                    let ry = current_rect.y as i32;
                    let rw = current_rect.width as i32;
                    let rh = current_rect.height as i32;

                    if is_near(mx, my, rx, ry) {
                        drag_mode = DragMode::ResizingTopLeft;
                    } else if is_near(mx, my, rx + rw, ry) {
                        drag_mode = DragMode::ResizingTopRight;
                    } else if is_near(mx, my, rx, ry + rh) {
                        drag_mode = DragMode::ResizingBottomLeft;
                    } else if is_near(mx, my, rx + rw, ry + rh) {
                        drag_mode = DragMode::ResizingBottomRight;
                    } else if mx >= rx && mx <= rx + rw && my >= ry && my <= ry + rh {
                        drag_mode = DragMode::Moving {
                            offset_x: (mx - rx) as i16,
                            offset_y: (my - ry) as i16,
                        };
                    } else {
                        // Click outside: start new drag selection
                        drag_mode = DragMode::Creating(ev.event_x, ev.event_y);
                        current_rect = CaptureRect {
                            x: ev.event_x,
                            y: ev.event_y,
                            width: 1,
                            height: 1,
                        };
                    }
                } else if ev.detail == 3 {
                    // Right click: cancel
                    result = UserAction::Cancel;
                    break 'event_loop;
                }
            }
            Event::MotionNotify(ev) => {
                let mx = ev.event_x;
                let my = ev.event_y;

                let mut changed = false;
                match drag_mode {
                    DragMode::None => {}
                    DragMode::Creating(sx, sy) => {
                        let x = sx.min(mx);
                        let y = sy.min(my);
                        let w = (sx - mx).unsigned_abs() as u16;
                        let h = (sy - my).unsigned_abs() as u16;
                        current_rect = CaptureRect {
                            x,
                            y,
                            width: w.max(1),
                            height: h.max(1),
                        };
                        changed = true;
                    }
                    DragMode::Moving { offset_x, offset_y } => {
                        let nx = (mx as i32 - offset_x as i32)
                            .clamp(0, (screen_w as i32 - current_rect.width as i32).max(0))
                            as i16;
                        let ny = (my as i32 - offset_y as i32)
                            .clamp(0, (screen_h as i32 - current_rect.height as i32).max(0))
                            as i16;
                        current_rect.x = nx;
                        current_rect.y = ny;
                        changed = true;
                    }
                    DragMode::ResizingTopLeft => {
                        let right = (current_rect.x as i32) + (current_rect.width as i32);
                        let bottom = (current_rect.y as i32) + (current_rect.height as i32);
                        let nx = (mx as i32).clamp(0, right - 20);
                        let ny = (my as i32).clamp(0, bottom - 20);
                        current_rect.x = nx as i16;
                        current_rect.y = ny as i16;
                        current_rect.width = (right - nx) as u16;
                        current_rect.height = (bottom - ny) as u16;
                        changed = true;
                    }
                    DragMode::ResizingTopRight => {
                        let left = current_rect.x as i32;
                        let bottom = (current_rect.y as i32) + (current_rect.height as i32);
                        let nr = (mx as i32).clamp(left + 20, screen_w as i32);
                        let ny = (my as i32).clamp(0, bottom - 20);
                        current_rect.y = ny as i16;
                        current_rect.width = (nr - left) as u16;
                        current_rect.height = (bottom - ny) as u16;
                        changed = true;
                    }
                    DragMode::ResizingBottomLeft => {
                        let right = (current_rect.x as i32) + (current_rect.width as i32);
                        let top = current_rect.y as i32;
                        let nx = (mx as i32).clamp(0, right - 20);
                        let nb = (my as i32).clamp(top + 20, screen_h as i32);
                        current_rect.x = nx as i16;
                        current_rect.width = (right - nx) as u16;
                        current_rect.height = (nb - top) as u16;
                        changed = true;
                    }
                    DragMode::ResizingBottomRight => {
                        let left = current_rect.x as i32;
                        let top = current_rect.y as i32;
                        let nr = (mx as i32).clamp(left + 20, screen_w as i32);
                        let nb = (my as i32).clamp(top + 20, screen_h as i32);
                        current_rect.width = (nr - left) as u16;
                        current_rect.height = (nb - top) as u16;
                        changed = true;
                    }
                }

                if changed {
                    render_composite(
                        &orig_screen,
                        &dark_screen,
                        &mut frame_buf,
                        screen_w as usize,
                        screen_h as usize,
                        Some(&current_rect),
                    );
                    paint_to_x11(conn, win, gc, screen_w, screen_h, screen.root_depth, &frame_buf)?;
                }
            }
            Event::ButtonRelease(ev) => {
                if ev.detail == 1 {
                    drag_mode = DragMode::None;
                }
            }
            Event::KeyPress(ev) => {
                // Keycode 9 = ESC
                // Keycode 36 = Return / Enter
                // Keycode 39 = 's' / 'S' (Scroll shot)
                if ev.detail == 9 {
                    result = UserAction::Cancel;
                    break 'event_loop;
                } else if ev.detail == 36 {
                    // Enter -> Single Shot
                    result = UserAction::SingleShot(current_rect);
                    break 'event_loop;
                } else if ev.detail == 39 {
                    // 's' -> Scroll Shot
                    result = UserAction::ScrollShot(current_rect);
                    break 'event_loop;
                } else if ev.detail == 26 {
                    // 'e' -> Edit
                    result = UserAction::Edit(current_rect);
                    break 'event_loop;
                } else if ev.detail == 33 {
                    // 'p' -> Pin to Desktop
                    result = UserAction::Pin(current_rect);
                    break 'event_loop;
                } else if ev.detail == 32 {
                    // 'o' -> OCR
                    result = UserAction::Ocr(current_rect);
                    break 'event_loop;
                }
            }
            _ => {}
        }
    }

    // Cleanup overlay window before returning
    let _ = conn.destroy_window(win);
    conn.flush()?;

    // Small delay to ensure desktop compositor repaints target window
    std::thread::sleep(std::time::Duration::from_millis(50));

    match result {
        UserAction::SingleShot(rect) => {
            let crop = crop_rect_from_bgrx(&orig_screen, screen_w as usize, screen_h as usize, &rect);
            Ok((result, Some(crop)))
        }
        UserAction::Pin(rect) => {
            let crop = crop_rect_from_bgrx(&orig_screen, screen_w as usize, screen_h as usize, &rect);
            Ok((result, Some(crop)))
        }
        UserAction::Edit(rect) => {
            let crop = crop_rect_from_bgrx(&orig_screen, screen_w as usize, screen_h as usize, &rect);
            Ok((result, Some(crop)))
        }
        UserAction::ScrollShot(rect) => {
            let crop = crop_rect_from_bgrx(&orig_screen, screen_w as usize, screen_h as usize, &rect);
            Ok((result, Some(crop)))
        }
        UserAction::Ocr(rect) => {
            let crop = crop_rect_from_bgrx(&orig_screen, screen_w as usize, screen_h as usize, &rect);
            Ok((result, Some(crop)))
        }
        UserAction::Cancel => Ok((UserAction::Cancel, None)),
    }
}

/// Losslessly crop rectangle directly from raw frozen screen buffer (BGRX -> RGBA)
pub fn crop_rect_from_bgrx(
    screen_buf: &[u8],
    screen_w: usize,
    screen_h: usize,
    rect: &CaptureRect,
) -> RgbaImage {
    let rx = (rect.x.max(0) as usize).min(screen_w);
    let ry = (rect.y.max(0) as usize).min(screen_h);
    let rw = (rect.width as usize).min(screen_w.saturating_sub(rx));
    let rh = (rect.height as usize).min(screen_h.saturating_sub(ry));

    let mut img = RgbaImage::new(rw as u32, rh as u32);
    let row_stride = screen_w * 4;

    for y in 0..rh {
        let src_start = (ry + y) * row_stride + rx * 4;
        for x in 0..rw {
            let s = src_start + x * 4;
            let b = screen_buf[s];
            let g = screen_buf[s + 1];
            let r = screen_buf[s + 2];
            img.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, 255]));
        }
    }
    img
}

fn is_near(mx: i32, my: i32, target_x: i32, target_y: i32) -> bool {
    (mx - target_x).abs() <= 12 && (my - target_y).abs() <= 12
}

fn get_toolbar_buttons(
    rect: &CaptureRect,
    screen_w: i32,
    screen_h: i32,
) -> Vec<ToolbarButton> {
    let tb_w = 540;
    let tb_h = 44;

    // Position toolbar centered horizontally under rect, or at screen bottom
    let mut tb_x = (rect.x as i32) + ((rect.width as i32 - tb_w) / 2);
    tb_x = tb_x.clamp(20, (screen_w - tb_w - 20).max(20));

    let mut tb_y = (rect.y as i32) + (rect.height as i32) + 16;
    if tb_y + tb_h > screen_h - 20 {
        // If below screen, place inside or above rect
        tb_y = (rect.y as i32) - tb_h - 16;
        if tb_y < 20 {
            tb_y = screen_h - tb_h - 30;
        }
    }

    vec![
        ToolbarButton {
            x: tb_x + 8,
            y: tb_y + 6,
            width: 78,
            height: 32,
            label: "Shot",
            action: UserAction::SingleShot(*rect),
        },
        ToolbarButton {
            x: tb_x + 92,
            y: tb_y + 6,
            width: 74,
            height: 32,
            label: "Pin",
            action: UserAction::Pin(*rect),
        },
        ToolbarButton {
            x: tb_x + 172,
            y: tb_y + 6,
            width: 74,
            height: 32,
            label: "Edit",
            action: UserAction::Edit(*rect),
        },
        ToolbarButton {
            x: tb_x + 252,
            y: tb_y + 6,
            width: 74,
            height: 32,
            label: "OCR",
            action: UserAction::Ocr(*rect),
        },
        ToolbarButton {
            x: tb_x + 332,
            y: tb_y + 6,
            width: 96,
            height: 32,
            label: "Scroll",
            action: UserAction::ScrollShot(*rect),
        },
        ToolbarButton {
            x: tb_x + 434,
            y: tb_y + 6,
            width: 98,
            height: 32,
            label: "Cancel",
            action: UserAction::Cancel,
        },
    ]
}

/// Composite darkened screen with bright selection, border, handles, and toolbar
fn render_composite(
    orig: &[u8],
    dark: &[u8],
    buf: &mut [u8],
    screen_w: usize,
    screen_h: usize,
    rect: Option<&CaptureRect>,
) {
    // 1. Reset to darkened screen
    buf.copy_from_slice(dark);

    let Some(r) = rect else { return };
    if r.width < 5 || r.height < 5 {
        return;
    }

    let rx = (r.x.max(0) as usize).min(screen_w);
    let ry = (r.y.max(0) as usize).min(screen_h);
    let rw = (r.width as usize).min(screen_w.saturating_sub(rx));
    let rh = (r.height as usize).min(screen_h.saturating_sub(ry));

    // 2. Copy bright original pixels inside the selection box
    let row_stride = screen_w * 4;
    for y in ry..(ry + rh) {
        let start = y * row_stride + rx * 4;
        let end = start + rw * 4;
        buf[start..end].copy_from_slice(&orig[start..end]);
    }

    // 3. Draw 2px bright white border around rect
    draw_rect_border(buf, screen_w, screen_h, rx, ry, rw, rh, [255, 255, 255, 255], 2);

    // 4. Draw 4 circular handles at the corners (just like GNOME screenshot!)
    draw_handle(buf, screen_w, screen_h, rx, ry);
    draw_handle(buf, screen_w, screen_h, rx + rw, ry);
    draw_handle(buf, screen_w, screen_h, rx, ry + rh);
    draw_handle(buf, screen_w, screen_h, rx + rw, ry + rh);

    // 5. Draw dimensions text badge (e.g. "800 x 600")
    let badge_text = format!("{} x {} px", r.width, r.height);
    let badge_y = if ry > 24 { ry - 22 } else { ry + 8 };
    draw_badge(buf, screen_w, screen_h, rx + 8, badge_y, &badge_text);

    // 6. Draw floating pill toolbar
    let buttons = get_toolbar_buttons(r, screen_w as i32, screen_h as i32);
    if let Some(first_btn) = buttons.first() {
        let tb_bg_x = first_btn.x - 8;
        let tb_bg_y = first_btn.y - 6;
        let tb_bg_w = 540;
        let tb_bg_h = 44;

        // Dark pill background (RGB: 32, 34, 38)
        draw_fill_rect(
            buf,
            screen_w,
            screen_h,
            tb_bg_x as usize,
            tb_bg_y as usize,
            tb_bg_w as usize,
            tb_bg_h as usize,
            [32, 34, 38, 255],
        );
        // Border outline
        draw_rect_border(
            buf,
            screen_w,
            screen_h,
            tb_bg_x as usize,
            tb_bg_y as usize,
            tb_bg_w as usize,
            tb_bg_h as usize,
            [70, 74, 82, 255],
            1,
        );

        // Draw buttons inside toolbar
        for btn in buttons {
            let bx = btn.x as usize;
            let by = btn.y as usize;
            let bw = btn.width as usize;
            let bh = btn.height as usize;

            let (btn_bg, text_color, icon) = match btn.action {
                UserAction::SingleShot(_) => ([50, 80, 150, 255], [255, 255, 255, 255], &ICON_SHOT),
                UserAction::Pin(_) => ([100, 60, 165, 255], [255, 255, 255, 255], &ICON_PIN),
                UserAction::Edit(_) => ([50, 95, 215, 255], [255, 255, 255, 255], &ICON_EDIT),
                UserAction::Ocr(_) => ([185, 90, 20, 255], [255, 255, 255, 255], &ICON_OCR),
                UserAction::ScrollShot(_) => ([40, 110, 60, 255], [255, 255, 255, 255], &ICON_SCROLL),
                UserAction::Cancel => ([60, 64, 72, 255], [220, 220, 220, 255], &ICON_CANCEL),
            };

            draw_fill_rect(buf, screen_w, screen_h, bx, by, bw, bh, btn_bg);
            draw_rect_border(buf, screen_w, screen_h, bx, by, bw, bh, [100, 110, 125, 255], 1);

            let content_w = 8 + 6 + btn.label.len() * 8;
            let start_x = bx + (bw.saturating_sub(content_w)) / 2;
            let start_y = by + (bh.saturating_sub(8)) / 2;
            draw_icon(buf, screen_w, screen_h, start_x, start_y, icon, text_color);
            draw_text(buf, screen_w, screen_h, start_x + 14, start_y, btn.label, text_color);
        }
    }
}

const ICON_SHOT: [u8; 8] = [
    0b00111100,
    0b01111110,
    0b11011011,
    0b10000001,
    0b10011001,
    0b10011001,
    0b11111111,
    0b00000000,
];

const ICON_PIN: [u8; 8] = [
    0b00111100,
    0b01111110,
    0b00111100,
    0b00011000,
    0b00011000,
    0b00011000,
    0b00011000,
    0b00010000,
];

const ICON_EDIT: [u8; 8] = [
    0b00000011,
    0b00000111,
    0b00001110,
    0b00011100,
    0b00111000,
    0b01110000,
    0b11100000,
    0b11000000,
];

const ICON_OCR: [u8; 8] = [
    0b11111111,
    0b10011001,
    0b00011000,
    0b00011000,
    0b00011000,
    0b00011000,
    0b10011001,
    0b11111111,
];

const ICON_SCROLL: [u8; 8] = [
    0b00011000,
    0b00111100,
    0b01111110,
    0b00011000,
    0b00011000,
    0b01111110,
    0b00111100,
    0b00011000,
];

const ICON_CANCEL: [u8; 8] = [
    0b11000011,
    0b01100110,
    0b00111100,
    0b00011000,
    0b00111100,
    0b01100110,
    0b11000011,
    0b00000000,
];

fn draw_icon(
    buf: &mut [u8],
    w: usize,
    h: usize,
    x: usize,
    y: usize,
    icon: &[u8; 8],
    color: [u8; 4],
) {
    for row in 0..8 {
        let py = y + row;
        if py >= h {
            break;
        }
        let bits = icon[row];
        for col in 0..8 {
            let px = x + col;
            if px >= w {
                break;
            }
            if (bits & (1 << (7 - col))) != 0 {
                let offset = (py * w + px) * 4;
                if offset + 3 < buf.len() {
                    buf[offset..offset + 4].copy_from_slice(&color);
                }
            }
        }
    }
}

fn draw_rect_border(
    buf: &mut [u8],
    w: usize,
    h: usize,
    x: usize,
    y: usize,
    rw: usize,
    rh: usize,
    color: [u8; 4],
    thickness: usize,
) {
    for t in 0..thickness {
        // Top and bottom borders
        for col in x.saturating_sub(t)..(x + rw + t).min(w) {
            set_pixel(buf, w, h, col, y + t, color);
            if y + rh >= t + 1 {
                set_pixel(buf, w, h, col, y + rh - 1 - t, color);
            }
        }
        // Left and right borders
        for row in y.saturating_sub(t)..(y + rh + t).min(h) {
            set_pixel(buf, w, h, x + t, row, color);
            if x + rw >= t + 1 {
                set_pixel(buf, w, h, x + rw - 1 - t, row, color);
            }
        }
    }
}

fn draw_handle(buf: &mut [u8], w: usize, h: usize, cx: usize, cy: usize) {
    let radius = 6;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if dx * dx + dy * dy <= radius * radius {
                let px = (cx as isize + dx) as usize;
                let py = (cy as isize + dy) as usize;
                let is_edge = dx * dx + dy * dy >= (radius - 1) * (radius - 1);
                let color = if is_edge {
                    [40, 40, 40, 255]
                } else {
                    [255, 255, 255, 255]
                };
                set_pixel(buf, w, h, px, py, color);
            }
        }
    }
}

fn draw_badge(buf: &mut [u8], w: usize, h: usize, x: usize, y: usize, text: &str) {
    let tw = text.len() * 8 + 8;
    let th = 16;
    draw_fill_rect(buf, w, h, x, y, tw, th, [20, 22, 25, 255]);
    draw_rect_border(buf, w, h, x, y, tw, th, [80, 85, 95, 255], 1);
    draw_text(buf, w, h, x + 4, y + 4, text, [255, 255, 255, 255]);
}

fn draw_fill_rect(
    buf: &mut [u8],
    w: usize,
    h: usize,
    x: usize,
    y: usize,
    rw: usize,
    rh: usize,
    color: [u8; 4],
) {
    for r in y..(y + rh).min(h) {
        for c in x..(x + rw).min(w) {
            set_pixel(buf, w, h, c, r, color);
        }
    }
}

#[inline(always)]
fn set_pixel(buf: &mut [u8], w: usize, h: usize, x: usize, y: usize, color: [u8; 4]) {
    if x < w && y < h {
        let idx = (y * w + x) * 4;
        if idx + 3 < buf.len() {
            // Note: X11 ZPixmap order is BGRA
            buf[idx] = color[2];     // B
            buf[idx + 1] = color[1]; // G
            buf[idx + 2] = color[0]; // R
            buf[idx + 3] = color[3]; // A
        }
    }
}

/// Send image buffer to X11 window in scanline bands to stay safely within X11 request length limits
fn paint_to_x11<C: Connection>(
    conn: &C,
    win: Window,
    gc: Gcontext,
    width: u16,
    height: u16,
    depth: u8,
    data: &[u8],
) -> Result<(), Box<dyn Error>> {
    let w = width as usize;
    let band_height = 80; // Send 80 rows per request (safely under 256K words limit)

    let mut y = 0;
    while y < height as usize {
        let chunk_h = band_height.min(height as usize - y);
        let start_byte = y * row_stride(w);
        let end_byte = start_byte + chunk_h * row_stride(w);
        let chunk_data = &data[start_byte..end_byte];

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
            chunk_data,
        )?;

        y += chunk_h;
    }
    conn.flush()?;
    Ok(())
}

#[inline(always)]
fn row_stride(w: usize) -> usize {
    w * 4
}
