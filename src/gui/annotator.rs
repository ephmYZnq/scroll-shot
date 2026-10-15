use crate::gui::font::draw_text;
use image::RgbaImage;
use std::error::Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConnectionExt, CreateGCAux, CreateWindowAux, EventMask, ImageFormat, Window, WindowClass,
};
use x11rb::protocol::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Rect,
    Arrow,
    Mosaic,
    Highlight,
    Step,
    Text,
}

#[derive(Clone, Debug)]
pub enum Annotation {
    Rect {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: [u8; 4],
        thickness: i32,
    },
    Arrow {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: [u8; 4],
    },
    Mosaic {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        block_size: usize,
    },
    Highlight {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        color: [u8; 4],
    },
    Step {
        x: i32,
        y: i32,
        num: u32,
        color: [u8; 4],
    },
    Text {
        x: i32,
        y: i32,
        text: String,
        color: [u8; 4],
    },
}

const PALETTE: [[u8; 4]; 4] = [
    [230, 45, 45, 255],  // Red
    [40, 180, 70, 255],  // Green
    [30, 135, 240, 255], // Blue
    [250, 205, 30, 255], // Yellow
];

const TOOLBAR_H: usize = 42;

/// Interactive full-featured annotation and mosaic editor
pub fn run_annotator<C: Connection>(
    conn: &C,
    screen_num: usize,
    base_image: RgbaImage,
) -> Result<Option<(RgbaImage, bool)>, Box<dyn Error>> {
    let setup = conn.setup();
    let screen = &setup.roots[screen_num];
    let root = screen.root;
    let screen_w = screen.width_in_pixels as i32;
    let screen_h = screen.height_in_pixels as i32;

    let img_w = base_image.width() as i32;
    let img_h = base_image.height() as i32;

    // Window dimensions: fit on screen, min width 650 to fit all buttons
    let win_w = (img_w.max(650)).min(screen_w - 20);
    let win_h = (img_h + TOOLBAR_H as i32).min(screen_h - 40);

    let win_x = ((screen_w - win_w) / 2).max(10);
    let win_y = ((screen_h - win_h) / 2).max(10);

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
        win_x as i16,
        win_y as i16,
        win_w as u16,
        win_h as u16,
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
    let _ = conn.set_input_focus(
        x11rb::protocol::xproto::InputFocus::PARENT,
        win,
        x11rb::CURRENT_TIME,
    );
    let _ = conn.grab_keyboard(
        false,
        win,
        x11rb::CURRENT_TIME,
        x11rb::protocol::xproto::GrabMode::ASYNC,
        x11rb::protocol::xproto::GrabMode::ASYNC,
    );
    conn.flush()?;

    let mut current_tool = Tool::Rect;
    let mut current_color_idx = 0;
    let mut annotations: Vec<Annotation> = Vec::new();
    let mut drag_start: Option<(i32, i32)> = None;
    let mut current_pos: Option<(i32, i32)> = None;
    let mut scroll_y: i32 = 0;
    let mut next_step_num: u32 = 1;
    let mut active_text: Option<(i32, i32, String)> = None;

    let ww = win_w as usize;
    let wh = win_h as usize;
    let mut win_buf = vec![0u8; ww * wh * 4];

    let mut user_completed: Option<bool> = None; // None: Cancel, Some(false): Done, Some(true): Pin

    // Helper: render full scene to win_buf and paint to X11
    let redraw = |conn: &C,
                  buf: &mut [u8],
                  base: &RgbaImage,
                  annots: &[Annotation],
                  tool: Tool,
                  c_idx: usize,
                  d_start: Option<(i32, i32)>,
                  c_pos: Option<(i32, i32)>,
                  sc_y: i32,
                  act_txt: Option<&(i32, i32, String)>|
     -> Result<(), Box<dyn Error>> {
        render_annotator_frame(
            buf, ww, wh, base, annots, tool, c_idx, d_start, c_pos, sc_y, act_txt,
        );
        paint_frame(conn, win, gc, win_w as u16, win_h as u16, screen.root_depth, buf)?;
        Ok(())
    };

    redraw(
        conn,
        &mut win_buf,
        &base_image,
        &annotations,
        current_tool,
        current_color_idx,
        drag_start,
        current_pos,
        scroll_y,
        active_text.as_ref(),
    )?;

    'event_loop: loop {
        let event = conn.wait_for_event()?;
        match event {
            Event::Expose(_) => {
                let _ = redraw(
                    conn,
                    &mut win_buf,
                    &base_image,
                    &annotations,
                    current_tool,
                    current_color_idx,
                    drag_start,
                    current_pos,
                    scroll_y,
                    active_text.as_ref(),
                );
            }
            Event::ButtonPress(ev) => {
                let mx = ev.event_x as i32;
                let my = ev.event_y as i32;

                // Mouse wheel for scrolling tall images
                if ev.detail == 4 {
                    // Scroll up
                    scroll_y = (scroll_y - 80).max(0);
                    let _ = redraw(
                        conn, &mut win_buf, &base_image, &annotations, current_tool,
                        current_color_idx, drag_start, current_pos, scroll_y, active_text.as_ref(),
                    );
                    continue;
                } else if ev.detail == 5 {
                    // Scroll down
                    let max_scroll = (img_h - (win_h - TOOLBAR_H as i32)).max(0);
                    scroll_y = (scroll_y + 80).min(max_scroll);
                    let _ = redraw(
                        conn, &mut win_buf, &base_image, &annotations, current_tool,
                        current_color_idx, drag_start, current_pos, scroll_y, active_text.as_ref(),
                    );
                    continue;
                }

                if ev.detail == 1 {
                    // Click in toolbar
                    if my < TOOLBAR_H as i32 {
                        // Commit active text if switching tools
                        if let Some((tx, ty, text)) = active_text.take() {
                            if !text.trim().is_empty() {
                                annotations.push(Annotation::Text {
                                    x: tx,
                                    y: ty,
                                    text,
                                    color: PALETTE[current_color_idx],
                                });
                            }
                        }

                        if let Some(action) = handle_toolbar_click(mx, my) {
                            match action {
                                ToolbarAction::SetTool(t) => current_tool = t,
                                ToolbarAction::SetColor(idx) => current_color_idx = idx,
                                ToolbarAction::Undo => {
                                    if let Some(last) = annotations.pop() {
                                        if let Annotation::Step { num, .. } = last {
                                            next_step_num = num;
                                        }
                                    }
                                }
                                ToolbarAction::Pin => {
                                    user_completed = Some(true);
                                    break 'event_loop;
                                }
                                ToolbarAction::Done => {
                                    user_completed = Some(false);
                                    break 'event_loop;
                                }
                                ToolbarAction::Cancel => {
                                    break 'event_loop;
                                }
                            }
                            let _ = redraw(
                                conn, &mut win_buf, &base_image, &annotations, current_tool,
                                current_color_idx, drag_start, current_pos, scroll_y, active_text.as_ref(),
                            );
                        }
                    } else {
                        // Click in image area
                        let img_x = mx;
                        let img_y = my - TOOLBAR_H as i32 + scroll_y;

                        if current_tool == Tool::Step {
                            // Step counter bubble placed immediately on click!
                            annotations.push(Annotation::Step {
                                x: img_x,
                                y: img_y,
                                num: next_step_num,
                                color: PALETTE[current_color_idx],
                            });
                            next_step_num += 1;
                            let _ = redraw(
                                conn, &mut win_buf, &base_image, &annotations, current_tool,
                                current_color_idx, None, None, scroll_y, active_text.as_ref(),
                            );
                        } else if current_tool == Tool::Text {
                            // Commit previous text if present
                            if let Some((tx, ty, text)) = active_text.take() {
                                if !text.trim().is_empty() {
                                    annotations.push(Annotation::Text {
                                        x: tx,
                                        y: ty,
                                        text,
                                        color: PALETTE[current_color_idx],
                                    });
                                }
                            }
                            // Start new text typing position
                            active_text = Some((img_x, img_y, String::new()));
                            let _ = redraw(
                                conn, &mut win_buf, &base_image, &annotations, current_tool,
                                current_color_idx, None, None, scroll_y, active_text.as_ref(),
                            );
                        } else {
                            // Drag-to-draw tools (Rect, Arrow, Mosaic, Highlight)
                            drag_start = Some((img_x, img_y));
                            current_pos = Some((img_x, img_y));
                        }
                    }
                }
            }
            Event::MotionNotify(ev) => {
                if drag_start.is_some() {
                    let mut latest_mx = ev.event_x as i32;
                    let mut latest_my = ev.event_y as i32;
                    let mut released = false;

                    while let Ok(Some(next_ev)) = conn.poll_for_event() {
                        match next_ev {
                            Event::MotionNotify(m) => {
                                latest_mx = m.event_x as i32;
                                latest_my = m.event_y as i32;
                            }
                            Event::ButtonRelease(r) if r.detail == 1 => {
                                released = true;
                                break;
                            }
                            _ => {}
                        }
                    }

                    let mx = latest_mx;
                    let my = latest_my;
                    let img_x = mx;
                    let img_y = my - TOOLBAR_H as i32 + scroll_y;
                    current_pos = Some((img_x, img_y));

                    if released {
                        if let (Some((x1, y1)), Some((x2, y2))) = (drag_start.take(), current_pos.take()) {
                            let color = PALETTE[current_color_idx];
                            if (x1 - x2).abs() > 3 || (y1 - y2).abs() > 3 {
                                match current_tool {
                                    Tool::Rect => {
                                        annotations.push(Annotation::Rect {
                                            x1, y1, x2, y2, color, thickness: 3,
                                        });
                                    }
                                    Tool::Arrow => {
                                        annotations.push(Annotation::Arrow { x1, y1, x2, y2, color });
                                    }
                                    Tool::Mosaic => {
                                        annotations.push(Annotation::Mosaic {
                                            x1, y1, x2, y2, block_size: 10,
                                        });
                                    }
                                    Tool::Highlight => {
                                        annotations.push(Annotation::Highlight { x1, y1, x2, y2, color });
                                    }
                                    Tool::Step | Tool::Text => {}
                                }
                            }
                        }
                        let _ = redraw(
                            conn, &mut win_buf, &base_image, &annotations, current_tool,
                            current_color_idx, None, None, scroll_y, active_text.as_ref(),
                        );
                    } else {
                        let _ = redraw(
                            conn, &mut win_buf, &base_image, &annotations, current_tool,
                            current_color_idx, drag_start, current_pos, scroll_y, active_text.as_ref(),
                        );
                    }
                }
            }
            Event::ButtonRelease(ev) => {
                if ev.detail == 1 {
                    if let (Some((x1, y1)), Some((x2, y2))) = (drag_start.take(), current_pos.take()) {
                        let color = PALETTE[current_color_idx];
                        if (x1 - x2).abs() > 3 || (y1 - y2).abs() > 3 {
                            match current_tool {
                                Tool::Rect => {
                                    annotations.push(Annotation::Rect {
                                        x1, y1, x2, y2, color, thickness: 3,
                                    });
                                }
                                Tool::Arrow => {
                                    annotations.push(Annotation::Arrow { x1, y1, x2, y2, color });
                                }
                                Tool::Mosaic => {
                                    annotations.push(Annotation::Mosaic {
                                        x1, y1, x2, y2, block_size: 10,
                                    });
                                }
                                Tool::Highlight => {
                                    annotations.push(Annotation::Highlight { x1, y1, x2, y2, color });
                                }
                                Tool::Step | Tool::Text => {}
                            }
                        }
                    }
                    let _ = redraw(
                        conn, &mut win_buf, &base_image, &annotations, current_tool,
                        current_color_idx, None, None, scroll_y, active_text.as_ref(),
                    );
                }
            }
            Event::KeyPress(ev) => {
                if active_text.is_some() {
                    // Currently in text input mode
                    if ev.detail == 9 {
                        // ESC -> cancel current text input
                        active_text = None;
                    } else if ev.detail == 36 {
                        // Enter -> commit current text
                        if let Some((tx, ty, text)) = active_text.take() {
                            if !text.trim().is_empty() {
                                annotations.push(Annotation::Text {
                                    x: tx,
                                    y: ty,
                                    text,
                                    color: PALETTE[current_color_idx],
                                });
                            }
                        }
                    } else if ev.detail == 22 {
                        // Backspace -> remove last char
                        if let Some((_, _, ref mut text)) = active_text {
                            text.pop();
                        }
                    } else {
                        // Append printable char
                        let shift = u16::from(ev.state) & 0x01 != 0;
                        if let Some(ch) = keycode_to_char(ev.detail, shift) {
                            if let Some((_, _, ref mut text)) = active_text {
                                text.push(ch);
                            }
                        }
                    }
                    let _ = redraw(
                        conn, &mut win_buf, &base_image, &annotations, current_tool,
                        current_color_idx, drag_start, current_pos, scroll_y, active_text.as_ref(),
                    );
                    continue;
                }

                // Normal shortcut mode
                // ESC = 9
                if ev.detail == 9 {
                    break 'event_loop;
                }
                // Enter = 36, Space = 65
                if ev.detail == 36 || ev.detail == 65 {
                    user_completed = Some(false);
                    break 'event_loop;
                }
                // 'p' = 33 -> Pin
                if ev.detail == 33 {
                    user_completed = Some(true);
                    break 'event_loop;
                }
                // 'r' = 27 -> Rect
                if ev.detail == 27 {
                    current_tool = Tool::Rect;
                }
                // 'a' = 38 -> Arrow
                if ev.detail == 38 {
                    current_tool = Tool::Arrow;
                }
                // 'm' = 58 -> Mosaic
                if ev.detail == 58 {
                    current_tool = Tool::Mosaic;
                }
                // 'h' = 43 -> Highlight
                if ev.detail == 43 {
                    current_tool = Tool::Highlight;
                }
                // 'n' = 57 -> Step (Number)
                if ev.detail == 57 {
                    current_tool = Tool::Step;
                }
                // 't' = 28 -> Text
                if ev.detail == 28 {
                    current_tool = Tool::Text;
                }
                // 'z' = 52 -> Undo
                if ev.detail == 52 {
                    if let Some(last) = annotations.pop() {
                        if let Annotation::Step { num, .. } = last {
                            next_step_num = num;
                        }
                    }
                }

                let _ = redraw(
                    conn, &mut win_buf, &base_image, &annotations, current_tool,
                    current_color_idx, drag_start, current_pos, scroll_y, active_text.as_ref(),
                );
            }
            _ => {}
        }
    }

    // Cleanup window
    let _ = conn.destroy_window(win);
    conn.flush()?;

    if let Some(is_pin) = user_completed {
        // Render annotations onto pristine base image at 1:1 original scale
        let final_image = flatten_annotations_to_image(&base_image, &annotations);
        Ok(Some((final_image, is_pin)))
    } else {
        Ok(None)
    }
}

enum ToolbarAction {
    SetTool(Tool),
    SetColor(usize),
    Undo,
    Pin,
    Done,
    Cancel,
}

fn handle_toolbar_click(x: i32, y: i32) -> Option<ToolbarAction> {
    if y > TOOLBAR_H as i32 {
        return None;
    }

    // Tools
    if x >= 8 && x < 66 {
        return Some(ToolbarAction::SetTool(Tool::Rect));
    }
    if x >= 68 && x < 126 {
        return Some(ToolbarAction::SetTool(Tool::Arrow));
    }
    if x >= 128 && x < 190 {
        return Some(ToolbarAction::SetTool(Tool::Mosaic));
    }
    if x >= 192 && x < 250 {
        return Some(ToolbarAction::SetTool(Tool::Highlight));
    }
    if x >= 252 && x < 310 {
        return Some(ToolbarAction::SetTool(Tool::Step));
    }
    if x >= 312 && x < 370 {
        return Some(ToolbarAction::SetTool(Tool::Text));
    }

    // Color palette dots
    let colors_x = 376;
    for i in 0..4 {
        let cx = colors_x + i * 22;
        if x >= cx && x < cx + 18 && y >= 10 && y < 32 {
            return Some(ToolbarAction::SetColor(i as usize));
        }
    }

    // Undo button
    if x >= 468 && x < 512 {
        return Some(ToolbarAction::Undo);
    }

    // Pin button
    if x >= 516 && x < 560 {
        return Some(ToolbarAction::Pin);
    }

    // Done button
    if x >= 564 && x < 612 {
        return Some(ToolbarAction::Done);
    }

    // Cancel button
    if x >= 616 && x < 644 {
        return Some(ToolbarAction::Cancel);
    }

    None
}

/// Composite base image, completed annotations, and in-progress preview into window buffer
fn render_annotator_frame(
    buf: &mut [u8],
    w: usize,
    h: usize,
    base: &RgbaImage,
    annotations: &[Annotation],
    current_tool: Tool,
    color_idx: usize,
    drag_start: Option<(i32, i32)>,
    current_pos: Option<(i32, i32)>,
    scroll_y: i32,
    active_text: Option<&(i32, i32, String)>,
) {
    let img_w = base.width() as usize;
    let img_h = base.height() as usize;
    let base_raw = base.as_raw();

    // 1. Fill background with dark neutral tone
    for pixel in buf.chunks_exact_mut(4) {
        pixel[0] = 30;
        pixel[1] = 28;
        pixel[2] = 26;
        pixel[3] = 255;
    }

    // 2. Draw base image starting below toolbar with scroll offset
    let dst_y_start = TOOLBAR_H;
    let visible_rows = (h.saturating_sub(dst_y_start)).min(img_h.saturating_sub(scroll_y as usize));

    for r in 0..visible_rows {
        let src_y = (scroll_y as usize) + r;
        let dst_y = dst_y_start + r;

        let src_row = src_y * img_w * 4;
        let copy_cols = img_w.min(w);

        for c in 0..copy_cols {
            let s = src_row + c * 4;
            let d = (dst_y * w + c) * 4;
            // base is RGBA, buf is BGRX
            buf[d] = base_raw[s + 2];     // B
            buf[d + 1] = base_raw[s + 1]; // G
            buf[d + 2] = base_raw[s];     // R
            buf[d + 3] = 255;
        }
    }

    // 3. Draw completed annotations (adjusted for scroll offset and toolbar offset)
    for annot in annotations {
        draw_annotation_to_buf(buf, base_raw, w, h, img_w, annot, scroll_y, dst_y_start as i32);
    }

    // 4. Draw active text typing with blinking cursor bar
    if let Some((tx, ty, text)) = active_text {
        let sy = ty - scroll_y + dst_y_start as i32;
        let preview_text = format!("{}|", text);
        draw_text_annotation(buf, w, h, *tx, sy, &preview_text, PALETTE[color_idx]);
    }

    // 5. Draw in-progress annotation preview if dragging
    if let (Some((x1, y1)), Some((x2, y2))) = (drag_start, current_pos) {
        let color = PALETTE[color_idx];
        let preview_annot = match current_tool {
            Tool::Rect => Some(Annotation::Rect { x1, y1, x2, y2, color, thickness: 3 }),
            Tool::Arrow => Some(Annotation::Arrow { x1, y1, x2, y2, color }),
            Tool::Mosaic => Some(Annotation::Mosaic { x1, y1, x2, y2, block_size: 10 }),
            Tool::Highlight => Some(Annotation::Highlight { x1, y1, x2, y2, color }),
            Tool::Step | Tool::Text => None,
        };
        if let Some(pa) = preview_annot {
            draw_annotation_to_buf(buf, base_raw, w, h, img_w, &pa, scroll_y, dst_y_start as i32);
        }
    }

    // 6. Draw top toolbar
    render_annotator_toolbar(buf, w, current_tool, color_idx);
}

fn render_annotator_toolbar(buf: &mut [u8], w: usize, current_tool: Tool, color_idx: usize) {
    let th = TOOLBAR_H;

    // Dark pill header background
    for r in 0..th {
        for c in 0..w {
            set_pixel(buf, w, th, c, r, [34, 36, 42, 255]);
        }
    }
    // Bottom border
    for c in 0..w {
        set_pixel(buf, w, th, c, th - 1, [70, 75, 85, 255]);
    }

    // Button 1: Rect
    draw_button(buf, w, th, 8, 6, 56, 28, "▢ Rect", current_tool == Tool::Rect);

    // Button 2: Arrow
    draw_button(buf, w, th, 68, 6, 56, 28, "-> Arr", current_tool == Tool::Arrow);

    // Button 3: Mosaic
    draw_button(buf, w, th, 128, 6, 60, 28, "░ Mos", current_tool == Tool::Mosaic);

    // Button 4: Mark
    draw_button(buf, w, th, 192, 6, 56, 28, "~ Mark", current_tool == Tool::Highlight);

    // Button 5: Step
    draw_button(buf, w, th, 252, 6, 56, 28, "① Step", current_tool == Tool::Step);

    // Button 6: Text
    draw_button(buf, w, th, 312, 6, 56, 28, "T Text", current_tool == Tool::Text);

    // Palette dots
    let colors_x = 376isize;
    for i in 0..4isize {
        let cx = colors_x + i * 22;
        let cy = 20isize;
        let col = PALETTE[i as usize];
        let is_sel = color_idx == i as usize;

        // Draw dot
        for dy in -6isize..=6 {
            for dx in -6isize..=6 {
                if dx * dx + dy * dy <= 36 {
                    let px = cx + 9 + dx;
                    let py = cy + dy;
                    if px >= 0 && py >= 0 {
                        set_pixel(buf, w, th, px as usize, py as usize, col);
                    }
                }
            }
        }
        // Selection outline
        if is_sel {
            for dy in -8isize..=8 {
                for dx in -8isize..=8 {
                    let d2 = dx * dx + dy * dy;
                    if d2 >= 49 && d2 <= 64 {
                        let px = cx + 9 + dx;
                        let py = cy + dy;
                        if px >= 0 && py >= 0 {
                            set_pixel(buf, w, th, px as usize, py as usize, [255, 255, 255, 255]);
                        }
                    }
                }
            }
        }
    }

    // Undo button
    draw_button(buf, w, th, 468, 6, 44, 28, "Undo", false);

    // Pin button (Purple)
    for r in 6..34 {
        for c in 516..560 {
            set_pixel(buf, w, th, c, r, [100, 60, 165, 255]);
        }
    }
    draw_text(buf, w, th, 526, 15, "Pin", [255, 255, 255, 255]);

    // Done button (Green)
    for r in 6..34 {
        for c in 564..612 {
            set_pixel(buf, w, th, c, r, [45, 140, 65, 255]);
        }
    }
    draw_text(buf, w, th, 574, 15, "Done", [255, 255, 255, 255]);

    // Cancel button
    draw_button(buf, w, th, 616, 6, 24, 28, "X", false);
}

fn draw_button(buf: &mut [u8], w: usize, h: usize, x: usize, y: usize, bw: usize, bh: usize, label: &str, active: bool) {
    let bg_color = if active { [65, 105, 225, 255] } else { [50, 54, 62, 255] };
    for r in y..(y + bh) {
        for c in x..(x + bw) {
            set_pixel(buf, w, h, c, r, bg_color);
        }
    }
    draw_text(buf, w, h, x + 6, y + 10, label, [235, 235, 235, 255]);
}

fn draw_annotation_to_buf(
    buf: &mut [u8],
    base_raw: &[u8],
    w: usize,
    h: usize,
    img_w: usize,
    annot: &Annotation,
    scroll_y: i32,
    y_offset: i32,
) {
    match *annot {
        Annotation::Rect { x1, y1, x2, y2, color, thickness } => {
            let sy1 = y1 - scroll_y + y_offset;
            let sy2 = y2 - scroll_y + y_offset;
            draw_rect_outline(buf, w, h, x1, sy1, x2, sy2, thickness, color);
        }
        Annotation::Arrow { x1, y1, x2, y2, color } => {
            let sy1 = y1 - scroll_y + y_offset;
            let sy2 = y2 - scroll_y + y_offset;
            draw_arrow(buf, w, h, x1, sy1, x2, sy2, color);
        }
        Annotation::Mosaic { x1, y1, x2, y2, block_size } => {
            let sy1 = y1 - scroll_y + y_offset;
            let sy2 = y2 - scroll_y + y_offset;
            apply_mosaic(buf, base_raw, w, h, img_w, x1, sy1, x2, sy2, y1, y2, block_size, y_offset);
        }
        Annotation::Highlight { x1, y1, x2, y2, color } => {
            let sy1 = y1 - scroll_y + y_offset;
            let sy2 = y2 - scroll_y + y_offset;
            apply_highlight(buf, w, h, x1, sy1, x2, sy2, color, y_offset);
        }
        Annotation::Step { x, y, num, color } => {
            let sy = y - scroll_y + y_offset;
            draw_step_bubble(buf, w, h, x, sy, num, color);
        }
        Annotation::Text { x, y, ref text, color } => {
            let sy = y - scroll_y + y_offset;
            draw_text_annotation(buf, w, h, x, sy, text, color);
        }
    }
}

/// Flatten all annotations onto the 1:1 original image to produce the output PNG
pub fn flatten_annotations_to_image(base: &RgbaImage, annotations: &[Annotation]) -> RgbaImage {
    let mut out = base.clone();
    let (w, h) = out.dimensions();
    let w_usize = w as usize;
    let h_usize = h as usize;

    let base_raw = base.as_raw().clone();
    let mut buf = vec![0u8; w_usize * h_usize * 4];

    // Convert RGBA base to BGRX for drawing routines
    for (src, dst) in base_raw.chunks_exact(4).zip(buf.chunks_exact_mut(4)) {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
        dst[3] = 255;
    }

    for annot in annotations {
        match *annot {
            Annotation::Rect { x1, y1, x2, y2, color, thickness } => {
                draw_rect_outline(&mut buf, w_usize, h_usize, x1, y1, x2, y2, thickness, color);
            }
            Annotation::Arrow { x1, y1, x2, y2, color } => {
                draw_arrow(&mut buf, w_usize, h_usize, x1, y1, x2, y2, color);
            }
            Annotation::Mosaic { x1, y1, x2, y2, block_size } => {
                apply_mosaic(&mut buf, &base_raw, w_usize, h_usize, w_usize, x1, y1, x2, y2, y1, y2, block_size, 0);
            }
            Annotation::Highlight { x1, y1, x2, y2, color } => {
                apply_highlight(&mut buf, w_usize, h_usize, x1, y1, x2, y2, color, 0);
            }
            Annotation::Step { x, y, num, color } => {
                draw_step_bubble(&mut buf, w_usize, h_usize, x, y, num, color);
            }
            Annotation::Text { x, y, ref text, color } => {
                draw_text_annotation(&mut buf, w_usize, h_usize, x, y, text, color);
            }
        }
    }

    // Convert BGRX back to RGBA
    let out_raw = out.as_mut();
    for (src, dst) in buf.chunks_exact(4).zip(out_raw.chunks_exact_mut(4)) {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
        dst[3] = 255;
    }

    out
}

// === Drawing Primitives ===

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

fn draw_thick_line(
    buf: &mut [u8],
    w: usize,
    h: usize,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    thickness: i32,
    color: [u8; 4],
) {
    let dx = (x1 - x0).abs();
    let dy = (y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx - dy;

    let mut curr_x = x0;
    let mut curr_y = y0;
    let half_t = (thickness / 2).max(1);

    loop {
        for dy_t in -half_t..=half_t {
            for dx_t in -half_t..=half_t {
                if dx_t * dx_t + dy_t * dy_t <= half_t * half_t + 1 {
                    let px = curr_x + dx_t;
                    let py = curr_y + dy_t;
                    if px >= 0 && py >= 0 {
                        set_pixel(buf, w, h, px as usize, py as usize, color);
                    }
                }
            }
        }

        if curr_x == x1 && curr_y == y1 {
            break;
        }

        let e2 = 2 * err;
        if e2 > -dy {
            err -= dy;
            curr_x += sx;
        }
        if e2 < dx {
            err += dx;
            curr_y += sy;
        }
    }
}

fn draw_rect_outline(
    buf: &mut [u8],
    w: usize,
    h: usize,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    thickness: i32,
    color: [u8; 4],
) {
    let min_x = x1.min(x2);
    let max_x = x1.max(x2);
    let min_y = y1.min(y2);
    let max_y = y1.max(y2);

    draw_thick_line(buf, w, h, min_x, min_y, max_x, min_y, thickness, color);
    draw_thick_line(buf, w, h, min_x, max_y, max_x, max_y, thickness, color);
    draw_thick_line(buf, w, h, min_x, min_y, min_x, max_y, thickness, color);
    draw_thick_line(buf, w, h, max_x, min_y, max_x, max_y, thickness, color);
}

fn draw_arrow(
    buf: &mut [u8],
    w: usize,
    h: usize,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: [u8; 4],
) {
    // 1. Draw shaft
    draw_thick_line(buf, w, h, x0, y0, x1, y1, 3, color);

    // 2. Compute arrowhead wings
    let angle = ((y1 - y0) as f64).atan2((x1 - x0) as f64);
    let wing_len = 16.0;
    let wing_angle = std::f64::consts::PI / 6.0;

    let w1_x = (x1 as f64 - wing_len * (angle - wing_angle).cos()).round() as i32;
    let w1_y = (y1 as f64 - wing_len * (angle - wing_angle).sin()).round() as i32;
    let w2_x = (x1 as f64 - wing_len * (angle + wing_angle).cos()).round() as i32;
    let w2_y = (y1 as f64 - wing_len * (angle + wing_angle).sin()).round() as i32;

    fill_triangle(buf, w, h, x1, y1, w1_x, w1_y, w2_x, w2_y, color);
}

fn fill_triangle(
    buf: &mut [u8],
    w: usize,
    h: usize,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: [u8; 4],
) {
    let min_x = x0.min(x1).min(x2).max(0);
    let max_x = x0.max(x1).max(x2).min(w as i32 - 1);
    let min_y = y0.min(y1).min(y2).max(0);
    let max_y = y0.max(y1).max(y2).min(h as i32 - 1);

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let d0 = (x1 - x0) * (y - y0) - (y1 - y0) * (x - x0);
            let d1 = (x2 - x1) * (y - y1) - (y2 - y1) * (x - x1);
            let d2 = (x0 - x2) * (y - y2) - (y0 - y2) * (x - x2);
            let has_neg = (d0 < 0) || (d1 < 0) || (d2 < 0);
            let has_pos = (d0 > 0) || (d1 > 0) || (d2 > 0);
            if !(has_neg && has_pos) {
                set_pixel(buf, w, h, x as usize, y as usize, color);
            }
        }
    }
}

fn apply_mosaic(
    buf: &mut [u8],
    base_raw: &[u8],
    w: usize,
    h: usize,
    img_w: usize,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    orig_y1: i32,
    orig_y2: i32,
    block_size: usize,
    y_min_guard: i32,
) {
    let min_x = (x1.min(x2).max(0) as usize).min(w);
    let max_x = (x1.max(x2).max(0) as usize).min(w);
    let min_y = (y1.min(y2).max(y_min_guard) as usize).min(h);
    let max_y = (y1.max(y2).max(y_min_guard) as usize).min(h);

    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let bs = block_size.max(4);
    let img_base_y1 = orig_y1.min(orig_y2).max(0) as usize;

    let mut by = min_y;
    let mut grid_row = 0;
    while by < max_y {
        let block_h = bs.min(max_y - by);
        let mut bx = min_x;
        while bx < max_x {
            let block_w = bs.min(max_x - bx);

            // Compute average color across this block from base_raw
            let mut sum_r = 0u64;
            let mut sum_g = 0u64;
            let mut sum_b = 0u64;
            let mut count = 0u64;

            let sample_y_start = img_base_y1 + grid_row * bs;
            for r in 0..block_h {
                let sy = sample_y_start + r;
                for c in bx..(bx + block_w) {
                    let idx = (sy * img_w + c) * 4;
                    if idx + 3 < base_raw.len() {
                        sum_r += base_raw[idx] as u64;
                        sum_g += base_raw[idx + 1] as u64;
                        sum_b += base_raw[idx + 2] as u64;
                        count += 1;
                    }
                }
            }

            if count > 0 {
                let avg_r = (sum_r / count) as u8;
                let avg_g = (sum_g / count) as u8;
                let avg_b = (sum_b / count) as u8;

                for r in by..(by + block_h) {
                    for c in bx..(bx + block_w) {
                        set_pixel(buf, w, h, c, r, [avg_r, avg_g, avg_b, 255]);
                    }
                }
            }

            bx += block_w;
        }
        by += block_h;
        grid_row += 1;
    }
}

fn apply_highlight(
    buf: &mut [u8],
    w: usize,
    h: usize,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: [u8; 4],
    y_min_guard: i32,
) {
    let min_x = (x1.min(x2).max(0) as usize).min(w);
    let max_x = (x1.max(x2).max(0) as usize).min(w);
    let min_y = (y1.min(y2).max(y_min_guard) as usize).min(h);
    let max_y = (y1.max(y2).max(y_min_guard) as usize).min(h);

    let alpha = 0.35f32;
    let inv_alpha = 1.0f32 - alpha;

    for y in min_y..max_y {
        for x in min_x..max_x {
            let idx = (y * w + x) * 4;
            if idx + 3 < buf.len() {
                let orig_b = buf[idx] as f32;
                let orig_g = buf[idx + 1] as f32;
                let orig_r = buf[idx + 2] as f32;

                buf[idx] = (orig_b * inv_alpha + (color[2] as f32) * alpha) as u8;
                buf[idx + 1] = (orig_g * inv_alpha + (color[1] as f32) * alpha) as u8;
                buf[idx + 2] = (orig_r * inv_alpha + (color[0] as f32) * alpha) as u8;
            }
        }
    }
}

fn paint_frame<C: Connection>(
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

fn draw_step_bubble(
    buf: &mut [u8],
    w: usize,
    h: usize,
    cx: i32,
    cy: i32,
    num: u32,
    color: [u8; 4],
) {
    // Outer white/contrast ring (radius 13)
    for dy in -13isize..=13 {
        for dx in -13isize..=13 {
            let d2 = dx * dx + dy * dy;
            if d2 <= 169 && d2 > 121 {
                let px = cx as isize + dx;
                let py = cy as isize + dy;
                if px >= 0 && px < w as isize && py >= 0 && py < h as isize {
                    set_pixel(buf, w, h, px as usize, py as usize, [255, 255, 255, 255]);
                }
            }
        }
    }

    // Filled circle (radius 11)
    for dy in -11isize..=11 {
        for dx in -11isize..=11 {
            if dx * dx + dy * dy <= 121 {
                let px = cx as isize + dx;
                let py = cy as isize + dy;
                if px >= 0 && px < w as isize && py >= 0 && py < h as isize {
                    set_pixel(buf, w, h, px as usize, py as usize, color);
                }
            }
        }
    }

    // Centered number text inside bubble
    let num_str = format!("{}", num);
    let text_len = num_str.len();
    let tx = cx as isize - (text_len as isize * 8) / 2;
    let ty = cy as isize - 4;
    if tx >= 0 && ty >= 0 {
        draw_text(buf, w, h, tx as usize, ty as usize, &num_str, [255, 255, 255, 255]);
    }
}

fn draw_text_annotation(
    buf: &mut [u8],
    w: usize,
    h: usize,
    x: i32,
    y: i32,
    text: &str,
    color: [u8; 4],
) {
    if x >= 0 && y >= 0 && x < w as i32 && y < h as i32 {
        crate::gui::font::draw_outlined_text_scaled(buf, w, h, x as usize, y as usize, text, color, 2);
    }
}

fn keycode_to_char(keycode: u8, shift: bool) -> Option<char> {
    match keycode {
        10 => Some(if shift { '!' } else { '1' }),
        11 => Some(if shift { '@' } else { '2' }),
        12 => Some(if shift { '#' } else { '3' }),
        13 => Some(if shift { '$' } else { '4' }),
        14 => Some(if shift { '%' } else { '5' }),
        15 => Some(if shift { '^' } else { '6' }),
        16 => Some(if shift { '&' } else { '7' }),
        17 => Some(if shift { '*' } else { '8' }),
        18 => Some(if shift { '(' } else { '9' }),
        19 => Some(if shift { ')' } else { '0' }),
        20 => Some(if shift { '_' } else { '-' }),
        21 => Some(if shift { '+' } else { '=' }),
        24 => Some(if shift { 'Q' } else { 'q' }),
        25 => Some(if shift { 'W' } else { 'w' }),
        26 => Some(if shift { 'E' } else { 'e' }),
        27 => Some(if shift { 'R' } else { 'r' }),
        28 => Some(if shift { 'T' } else { 't' }),
        29 => Some(if shift { 'Y' } else { 'y' }),
        30 => Some(if shift { 'U' } else { 'u' }),
        31 => Some(if shift { 'I' } else { 'i' }),
        32 => Some(if shift { 'O' } else { 'o' }),
        33 => Some(if shift { 'P' } else { 'p' }),
        34 => Some(if shift { '{' } else { '[' }),
        35 => Some(if shift { '}' } else { ']' }),
        38 => Some(if shift { 'A' } else { 'a' }),
        39 => Some(if shift { 'S' } else { 's' }),
        40 => Some(if shift { 'D' } else { 'd' }),
        41 => Some(if shift { 'F' } else { 'f' }),
        42 => Some(if shift { 'G' } else { 'g' }),
        43 => Some(if shift { 'H' } else { 'h' }),
        44 => Some(if shift { 'J' } else { 'j' }),
        45 => Some(if shift { 'K' } else { 'k' }),
        46 => Some(if shift { 'L' } else { 'l' }),
        47 => Some(if shift { ':' } else { ';' }),
        48 => Some(if shift { '"' } else { '\'' }),
        52 => Some(if shift { 'Z' } else { 'z' }),
        53 => Some(if shift { 'X' } else { 'x' }),
        54 => Some(if shift { 'C' } else { 'c' }),
        55 => Some(if shift { 'V' } else { 'v' }),
        56 => Some(if shift { 'B' } else { 'b' }),
        57 => Some(if shift { 'N' } else { 'n' }),
        58 => Some(if shift { 'M' } else { 'm' }),
        59 => Some(if shift { '<' } else { ',' }),
        60 => Some(if shift { '>' } else { '.' }),
        61 => Some(if shift { '?' } else { '/' }),
        65 => Some(' '),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn test_toolbar_hit_testing() {
        // Tool buttons
        assert_eq!(
            handle_toolbar_click(30, 20).map(|a| match a {
                ToolbarAction::SetTool(t) => Some(t),
                _ => None,
            }),
            Some(Some(Tool::Rect))
        );
        assert_eq!(
            handle_toolbar_click(90, 20).map(|a| match a {
                ToolbarAction::SetTool(t) => Some(t),
                _ => None,
            }),
            Some(Some(Tool::Arrow))
        );
        assert_eq!(
            handle_toolbar_click(150, 20).map(|a| match a {
                ToolbarAction::SetTool(t) => Some(t),
                _ => None,
            }),
            Some(Some(Tool::Mosaic))
        );
        assert_eq!(
            handle_toolbar_click(210, 20).map(|a| match a {
                ToolbarAction::SetTool(t) => Some(t),
                _ => None,
            }),
            Some(Some(Tool::Highlight))
        );
        assert_eq!(
            handle_toolbar_click(280, 20).map(|a| match a {
                ToolbarAction::SetTool(t) => Some(t),
                _ => None,
            }),
            Some(Some(Tool::Step))
        );
        assert_eq!(
            handle_toolbar_click(340, 20).map(|a| match a {
                ToolbarAction::SetTool(t) => Some(t),
                _ => None,
            }),
            Some(Some(Tool::Text))
        );

        // Palette dots
        assert_eq!(
            handle_toolbar_click(380, 20).map(|a| match a {
                ToolbarAction::SetColor(idx) => Some(idx),
                _ => None,
            }),
            Some(Some(0))
        );
        assert_eq!(
            handle_toolbar_click(402, 20).map(|a| match a {
                ToolbarAction::SetColor(idx) => Some(idx),
                _ => None,
            }),
            Some(Some(1))
        );

        // Undo, Pin, Done & Cancel
        assert!(matches!(
            handle_toolbar_click(480, 20),
            Some(ToolbarAction::Undo)
        ));
        assert!(matches!(
            handle_toolbar_click(530, 20),
            Some(ToolbarAction::Pin)
        ));
        assert!(matches!(
            handle_toolbar_click(580, 20),
            Some(ToolbarAction::Done)
        ));
        assert!(matches!(
            handle_toolbar_click(625, 20),
            Some(ToolbarAction::Cancel)
        ));
    }

    #[test]
    fn test_flatten_empty_annotations() {
        let base = RgbaImage::from_pixel(100, 100, Rgba([120, 130, 140, 255]));
        let out = flatten_annotations_to_image(&base, &[]);
        assert_eq!(out.dimensions(), (100, 100));
        assert_eq!(out.get_pixel(50, 50), &Rgba([120, 130, 140, 255]));
    }

    #[test]
    fn test_flatten_rect_annotation() {
        let base = RgbaImage::from_pixel(100, 100, Rgba([255, 255, 255, 255]));
        let red = [255, 0, 0, 255];
        let annots = vec![Annotation::Rect {
            x1: 10,
            y1: 10,
            x2: 40,
            y2: 40,
            color: red,
            thickness: 3,
        }];
        let out = flatten_annotations_to_image(&base, &annots);
        assert_eq!(out.get_pixel(25, 25), &Rgba([255, 255, 255, 255]));
        assert_eq!(out.get_pixel(10, 20), &Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn test_flatten_mosaic_annotation() {
        let mut base = RgbaImage::new(40, 40);
        for y in 0..40 {
            for x in 0..40 {
                base.put_pixel(x, y, Rgba([(x * 5) as u8, (y * 5) as u8, 100, 255]));
            }
        }
        let annots = vec![Annotation::Mosaic {
            x1: 10,
            y1: 10,
            x2: 30,
            y2: 30,
            block_size: 10,
        }];
        let out = flatten_annotations_to_image(&base, &annots);
        let p1 = out.get_pixel(12, 12);
        let p2 = out.get_pixel(16, 16);
        assert_eq!(p1, p2, "Mosaic cell pixels should be averaged to the same color");
        assert_eq!(out.get_pixel(2, 2), base.get_pixel(2, 2));
    }

    #[test]
    fn test_flatten_highlight_annotation() {
        let base = RgbaImage::from_pixel(50, 50, Rgba([100, 100, 100, 255]));
        let yellow = [250, 205, 30, 255];
        let annots = vec![Annotation::Highlight {
            x1: 10,
            y1: 10,
            x2: 30,
            y2: 30,
            color: yellow,
        }];
        let out = flatten_annotations_to_image(&base, &annots);
        let p = out.get_pixel(20, 20);
        assert!(p[0] > 100, "Red channel should increase towards yellow");
        assert!(p[1] > 100, "Green channel should increase towards yellow");
        assert_eq!(out.get_pixel(5, 5), &Rgba([100, 100, 100, 255]));
    }

    #[test]
    fn test_flatten_step_annotation() {
        let base = RgbaImage::from_pixel(60, 60, Rgba([0, 0, 0, 255]));
        let red = [230, 45, 45, 255];
        let annots = vec![Annotation::Step {
            x: 30,
            y: 30,
            num: 1,
            color: red,
        }];
        let out = flatten_annotations_to_image(&base, &annots);
        // The bubble center/interior should be non-black (red circle or white text/border)
        let p_center = out.get_pixel(30, 30);
        assert_ne!(p_center, &Rgba([0, 0, 0, 255]));
        // Outside bubble should remain untouched black
        assert_eq!(out.get_pixel(5, 5), &Rgba([0, 0, 0, 255]));
    }

    #[test]
    fn test_flatten_text_annotation() {
        let base = RgbaImage::from_pixel(80, 40, Rgba([255, 255, 255, 255]));
        let blue = [30, 135, 240, 255];
        let annots = vec![Annotation::Text {
            x: 10,
            y: 10,
            text: "TEST".to_string(),
            color: blue,
        }];
        let out = flatten_annotations_to_image(&base, &annots);
        // Text area should have colored pixels
        let mut has_text_pixel = false;
        for y in 10..26 {
            for x in 10..60 {
                let p = out.get_pixel(x, y);
                if p != &Rgba([255, 255, 255, 255]) {
                    has_text_pixel = true;
                    break;
                }
            }
        }
        assert!(has_text_pixel, "Text should draw pixels onto image");
    }

    #[test]
    fn test_keycode_mapper() {
        assert_eq!(keycode_to_char(10, false), Some('1'));
        assert_eq!(keycode_to_char(10, true), Some('!'));
        assert_eq!(keycode_to_char(24, false), Some('q'));
        assert_eq!(keycode_to_char(24, true), Some('Q'));
        assert_eq!(keycode_to_char(65, false), Some(' '));
    }
}
