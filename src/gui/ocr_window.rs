use crate::gui::font::draw_text;
use crate::ocr::{extract_text_from_image, OcrEngineInfo};
use crate::x11::clipboard::{copy_text_to_clipboard, copy_to_clipboard};
use image::RgbaImage;
use std::error::Error;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt, CreateGCAux, CreateWindowAux, EventMask, ImageFormat,
    PropMode, StackMode, Window, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;

const WIN_W: u16 = 560;
const WIN_H: u16 = 360;
const INSTALL_CMD: &str = "sudo apt install -y tesseract-ocr tesseract-ocr-chi-sim";
const CHI_INSTALL_CMD: &str = "sudo apt install -y tesseract-ocr-chi-sim";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextPos {
    pub line: usize,
    pub col: usize,
}

impl PartialOrd for TextPos {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TextPos {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.line.cmp(&other.line) {
            std::cmp::Ordering::Equal => self.col.cmp(&other.col),
            other => other,
        }
    }
}

#[derive(Debug, Clone)]
pub enum OcrStatus {
    Success {
        text: String,
        lines: Vec<String>,
        engine_info: OcrEngineInfo,
        copied_feedback: bool,
        copied_selection: bool,
    },
    MissingEngine {
        cmd_copied: bool,
        img_copied: bool,
    },
    Empty {
        img_copied: bool,
    },
}

/// Extract text enclosed in [start, end) text positions across lines
pub fn extract_selected_text(lines: &[String], start: TextPos, end: TextPos) -> String {
    if lines.is_empty() || start >= end {
        return String::new();
    }
    let start_line = start.line.min(lines.len() - 1);
    let end_line = end.line.min(lines.len() - 1);

    if start_line == end_line {
        let line = &lines[start_line];
        let chars: Vec<char> = line.chars().collect();
        let s_col = start.col.min(chars.len());
        let e_col = end.col.min(chars.len());
        if s_col < e_col {
            chars[s_col..e_col].iter().collect()
        } else {
            String::new()
        }
    } else {
        let mut result = Vec::new();
        // First line: from start.col to end
        {
            let line = &lines[start_line];
            let chars: Vec<char> = line.chars().collect();
            let s_col = start.col.min(chars.len());
            let part: String = chars[s_col..].iter().collect();
            result.push(part);
        }
        // Middle full lines
        for l in (start_line + 1)..end_line {
            result.push(lines[l].clone());
        }
        // Last line: from 0 to end.col
        {
            let line = &lines[end_line];
            let chars: Vec<char> = line.chars().collect();
            let e_col = end.col.min(chars.len());
            let part: String = chars[..e_col].iter().collect();
            result.push(part);
        }
        result.join("\n")
    }
}

/// Map dialog mouse coordinates (mx, my) to line & column in text
pub fn mouse_to_text_pos(
    mx: i32,
    my: i32,
    scroll_line_offset: usize,
    lines: &[String],
) -> TextPos {
    if lines.is_empty() {
        return TextPos { line: 0, col: 0 };
    }
    let box_y = 84;
    let line_h = 18;
    let rel_y = (my - (box_y + 10)).max(0);
    let screen_line = (rel_y / line_h) as usize;
    let line_idx = (scroll_line_offset + screen_line).min(lines.len().saturating_sub(1));

    let line_str = &lines[line_idx];
    let rel_x = (mx - 28).max(0) as usize;
    let mut cur_x = 0;
    let mut col_idx = 0;

    for (idx, ch) in line_str.chars().enumerate() {
        let cw = if ch.is_ascii() { 8 } else { 16 };
        if rel_x < cur_x + cw / 2 {
            return TextPos {
                line: line_idx,
                col: idx,
            };
        }
        cur_x += cw;
        col_idx = idx + 1;
    }

    TextPos {
        line: line_idx,
        col: col_idx,
    }
}

/// Floating interactive OCR dialog window with one-click copy, mouse text selection, and multi-lang status
pub fn show_ocr_window<C: Connection>(
    conn: &C,
    screen_num: usize,
    image: &RgbaImage,
    initial_x: i32,
    initial_y: i32,
) -> Result<(), Box<dyn Error>> {
    let setup = conn.setup();
    let screen = &setup.roots[screen_num];
    let root = screen.root;
    let screen_w = screen.width_in_pixels as i32;
    let screen_h = screen.height_in_pixels as i32;

    let mut win_x = initial_x.clamp(20, (screen_w - WIN_W as i32 - 20).max(20));
    let mut win_y = initial_y.clamp(20, (screen_h - WIN_H as i32 - 20).max(20));

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
        WIN_W,
        WIN_H,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .override_redirect(1)
            .event_mask(event_mask),
    )?;

    let gc = conn.generate_id()?;
    conn.create_gc(gc, win, &CreateGCAux::new())?;

    // Keep on top
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

    // Perform initial OCR attempt
    let mut status = match extract_text_from_image(image) {
        Ok((text, engine_info)) if !text.trim().is_empty() => {
            let trimmed = text.trim().to_string();
            let _ = copy_text_to_clipboard(&trimmed);
            let lines: Vec<String> = trimmed.lines().map(|s| s.to_string()).collect();
            OcrStatus::Success {
                text: trimmed,
                lines,
                engine_info,
                copied_feedback: true,
                copied_selection: false,
            }
        }
        Ok(_) => {
            let _ = copy_to_clipboard(image);
            OcrStatus::Empty {
                img_copied: true,
            }
        }
        Err(_) => {
            let _ = copy_to_clipboard(image);
            OcrStatus::MissingEngine {
                cmd_copied: false,
                img_copied: true,
            }
        }
    };

    let mut is_dragging = false;
    let mut drag_ptr_start = (0, 0);
    let mut drag_win_start = (win_x, win_y);
    let mut scroll_line_offset: usize = 0;

    let mut selection: Option<(TextPos, TextPos)> = None;
    let mut selection_anchor: Option<TextPos> = None;
    let mut is_text_selecting = false;

    let mut buf = vec![0u8; (WIN_W as usize) * (WIN_H as usize) * 4];
    render_dialog(&mut buf, WIN_W as usize, WIN_H as usize, &status, scroll_line_offset, selection);
    paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;

    let mut should_pin = false;

    'event_loop: loop {
        let event = conn.wait_for_event()?;
        match event {
            Event::Expose(_) => {
                if !is_dragging {
                    paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;
                }
            }
            Event::ButtonPress(ev) => {
                let mx = ev.event_x as i32;
                let my = ev.event_y as i32;

                if ev.detail == 1 {
                    // Close button top-right (x: 524..554, y: 6..32)
                    if mx >= (WIN_W as i32 - 36) && my <= 32 {
                        break 'event_loop;
                    }

                    // Click detection based on status
                    match &mut status {
                        OcrStatus::MissingEngine { cmd_copied, img_copied } => {
                            // Button 1: [📋 复制安装命令] (x: 20..190, y: 304..340)
                            if mx >= 20 && mx <= 190 && my >= 304 && my <= 340 {
                                let _ = copy_text_to_clipboard(INSTALL_CMD);
                                *cmd_copied = true;
                                render_dialog(&mut buf, WIN_W as usize, WIN_H as usize, &status, scroll_line_offset, selection);
                                paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;
                                continue;
                            }
                            // Button 2: [🔄 重试识别] (x: 200..310, y: 304..340)
                            if mx >= 200 && mx <= 310 && my >= 304 && my <= 340 {
                                if let Ok((new_text, new_info)) = extract_text_from_image(image) {
                                    if !new_text.trim().is_empty() {
                                        let trimmed = new_text.trim().to_string();
                                        let _ = copy_text_to_clipboard(&trimmed);
                                        let lines: Vec<String> = trimmed.lines().map(|s| s.to_string()).collect();
                                        status = OcrStatus::Success {
                                            text: trimmed,
                                            lines,
                                            engine_info: new_info,
                                            copied_feedback: true,
                                            copied_selection: false,
                                        };
                                        selection = None;
                                        render_dialog(&mut buf, WIN_W as usize, WIN_H as usize, &status, scroll_line_offset, selection);
                                        paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;
                                        continue;
                                    }
                                }
                            }
                            // Button 3: [📷 复制原图] (x: 320..430, y: 304..340)
                            if mx >= 320 && mx <= 430 && my >= 304 && my <= 340 {
                                let _ = copy_to_clipboard(image);
                                *img_copied = true;
                                render_dialog(&mut buf, WIN_W as usize, WIN_H as usize, &status, scroll_line_offset, selection);
                                paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;
                                continue;
                            }
                            // Button 4: [✕ 关闭] (x: 440..540, y: 304..340)
                            if mx >= 440 && mx <= 540 && my >= 304 && my <= 340 {
                                break 'event_loop;
                            }
                        }
                        OcrStatus::Success { text, lines, engine_info, copied_feedback, copied_selection } => {
                            let supports_chi = engine_info.supports_chinese;

                            // Button 1: Copy Selected or Copy All (x: 20..190, y: 304..340)
                            if mx >= 20 && mx <= 190 && my >= 304 && my <= 340 {
                                let (to_copy, is_sel) = if let Some((s, e)) = selection {
                                    if s < e {
                                        let extracted = extract_selected_text(lines, s, e);
                                        if !extracted.is_empty() {
                                            (extracted, true)
                                        } else {
                                            (text.clone(), false)
                                        }
                                    } else {
                                        (text.clone(), false)
                                    }
                                } else {
                                    (text.clone(), false)
                                };
                                let _ = copy_text_to_clipboard(&to_copy);
                                *copied_feedback = true;
                                *copied_selection = is_sel;
                                render_dialog(&mut buf, WIN_W as usize, WIN_H as usize, &status, scroll_line_offset, selection);
                                paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;
                                continue;
                            }

                            if !supports_chi {
                                // Missing Chinese package: Show [Get Chi Pack] button (x: 198..322, y: 304..340)
                                if mx >= 198 && mx <= 322 && my >= 304 && my <= 340 {
                                    let _ = copy_text_to_clipboard(CHI_INSTALL_CMD);
                                    *copied_feedback = true;
                                    *copied_selection = false;
                                    render_dialog(&mut buf, WIN_W as usize, WIN_H as usize, &status, scroll_line_offset, selection);
                                    paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;
                                    continue;
                                }
                                // Pin button (x: 330..420, y: 304..340)
                                if mx >= 330 && mx <= 420 && my >= 304 && my <= 340 {
                                    should_pin = true;
                                    break 'event_loop;
                                }
                                // Close button (x: 430..540, y: 304..340)
                                if mx >= 430 && mx <= 540 && my >= 304 && my <= 340 {
                                    break 'event_loop;
                                }
                            } else {
                                // Full Chinese support:
                                // Button 2: [📌 贴图置顶] (x: 204..330, y: 304..340)
                                if mx >= 204 && mx <= 330 && my >= 304 && my <= 340 {
                                    should_pin = true;
                                    break 'event_loop;
                                }
                                // Button 3: [✕ 完成] (x: 420..540, y: 304..340)
                                if mx >= 420 && mx <= 540 && my >= 304 && my <= 340 {
                                    break 'event_loop;
                                }
                            }
                        }
                        OcrStatus::Empty { img_copied, .. } => {
                            // Button 1: [📷 复制原图] (x: 20..160, y: 304..340)
                            if mx >= 20 && mx <= 160 && my >= 304 && my <= 340 {
                                let _ = copy_to_clipboard(image);
                                *img_copied = true;
                                render_dialog(&mut buf, WIN_W as usize, WIN_H as usize, &status, scroll_line_offset, selection);
                                paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;
                                continue;
                            }
                            // Button 2: [✕ 关闭] (x: 420..540, y: 304..340)
                            if mx >= 420 && mx <= 540 && my >= 304 && my <= 340 {
                                break 'event_loop;
                            }
                        }
                    }

                    // Check if mouse clicked inside the text preview area (x: 16..544, y: 84..268)
                    if mx >= 16 && mx <= (WIN_W as i32 - 16) && my >= 84 && my <= 268 {
                        if let OcrStatus::Success { lines, .. } = &status {
                            let pos = mouse_to_text_pos(mx, my, scroll_line_offset, lines);
                            selection_anchor = Some(pos);
                            selection = None;
                            is_text_selecting = true;
                            render_dialog(&mut buf, WIN_W as usize, WIN_H as usize, &status, scroll_line_offset, selection);
                            paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;
                            continue;
                        }
                    }

                    // Click on header or bottom margin area starts window drag
                    if my < 44 || (my >= 270 && my < 300) {
                        is_dragging = true;
                        drag_ptr_start = (ev.root_x as i32, ev.root_y as i32);
                        drag_win_start = (win_x, win_y);
                    }
                } else if ev.detail == 4 {
                    // Wheel up -> scroll text up
                    if scroll_line_offset > 0 {
                        scroll_line_offset -= 1;
                        render_dialog(&mut buf, WIN_W as usize, WIN_H as usize, &status, scroll_line_offset, selection);
                        paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;
                    }
                } else if ev.detail == 5 {
                    // Wheel down -> scroll text down
                    scroll_line_offset += 1;
                    render_dialog(&mut buf, WIN_W as usize, WIN_H as usize, &status, scroll_line_offset, selection);
                    paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;
                }
            }
            Event::ButtonRelease(ev) => {
                if ev.detail == 1 {
                    if is_text_selecting {
                        is_text_selecting = false;
                        if let Some((s, e)) = selection {
                            if s == e {
                                selection = None;
                            }
                        }
                        render_dialog(&mut buf, WIN_W as usize, WIN_H as usize, &status, scroll_line_offset, selection);
                        paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;
                    }
                    if is_dragging {
                        is_dragging = false;
                        let _ = conn.configure_window(
                            win,
                            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                        );
                        let _ = conn.flush();
                    }
                }
            }
            Event::MotionNotify(ev) => {
                if is_text_selecting {
                    let mut latest_mx = ev.event_x as i32;
                    let mut latest_my = ev.event_y as i32;

                    while let Ok(Some(next_ev)) = conn.poll_for_event() {
                        match next_ev {
                            Event::MotionNotify(m) => {
                                latest_mx = m.event_x as i32;
                                latest_my = m.event_y as i32;
                            }
                            Event::ButtonRelease(r) if r.detail == 1 => {
                                is_text_selecting = false;
                                break;
                            }
                            _ => {}
                        }
                    }

                    if let OcrStatus::Success { lines, .. } = &status {
                        let cur_pos = mouse_to_text_pos(latest_mx, latest_my, scroll_line_offset, lines);
                        if let Some(anchor) = selection_anchor {
                            if anchor != cur_pos {
                                let (s, e) = if anchor <= cur_pos {
                                    (anchor, cur_pos)
                                } else {
                                    (cur_pos, anchor)
                                };
                                selection = Some((s, e));
                            } else {
                                selection = None;
                            }
                        }
                        render_dialog(&mut buf, WIN_W as usize, WIN_H as usize, &status, scroll_line_offset, selection);
                        paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;
                    }
                } else if is_dragging {
                    let mut latest_rx = ev.root_x as i32;
                    let mut latest_ry = ev.root_y as i32;

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
                // Enter = 36
                if ev.detail == 36 {
                    break 'event_loop;
                }
                // 'p' = 33 -> Pin
                if ev.detail == 33 {
                    should_pin = true;
                    break 'event_loop;
                }
                // 'a' = 38 with Control modifier -> Select All
                if ev.detail == 38 && (u16::from(ev.state) & 0x04 != 0) {
                    if let OcrStatus::Success { lines, .. } = &status {
                        if !lines.is_empty() {
                            let last_l = lines.len() - 1;
                            let last_c = lines[last_l].chars().count();
                            selection = Some((TextPos { line: 0, col: 0 }, TextPos { line: last_l, col: last_c }));
                            render_dialog(&mut buf, WIN_W as usize, WIN_H as usize, &status, scroll_line_offset, selection);
                            paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;
                        }
                    }
                }
                // 'c' = 54 -> Copy text or install cmd
                if ev.detail == 54 {
                    match &mut status {
                        OcrStatus::Success { text, lines, copied_feedback, copied_selection, .. } => {
                            let (to_copy, is_sel) = if let Some((s, e)) = selection {
                                if s < e {
                                    let extracted = extract_selected_text(lines, s, e);
                                    if !extracted.is_empty() {
                                        (extracted, true)
                                    } else {
                                        (text.clone(), false)
                                    }
                                } else {
                                    (text.clone(), false)
                                }
                            } else {
                                (text.clone(), false)
                            };
                            let _ = copy_text_to_clipboard(&to_copy);
                            *copied_feedback = true;
                            *copied_selection = is_sel;
                            render_dialog(&mut buf, WIN_W as usize, WIN_H as usize, &status, scroll_line_offset, selection);
                            paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;
                        }
                        OcrStatus::MissingEngine { cmd_copied, .. } => {
                            let _ = copy_text_to_clipboard(INSTALL_CMD);
                            *cmd_copied = true;
                            render_dialog(&mut buf, WIN_W as usize, WIN_H as usize, &status, scroll_line_offset, selection);
                            paint_dialog(conn, win, gc, WIN_W, WIN_H, screen.root_depth, &buf)?;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    // Cleanup dialog window
    let _ = conn.destroy_window(win);
    let _ = conn.flush();

    if should_pin {
        crate::gui::pin::run_pin_window(conn, screen_num, image.clone(), win_x, win_y)?;
    }

    Ok(())
}

fn render_dialog(
    buf: &mut [u8],
    w: usize,
    h: usize,
    status: &OcrStatus,
    scroll_line_offset: usize,
    selection: Option<(TextPos, TextPos)>,
) {
    // 1. Fill base dialog background (Dark Slate RGB: 30, 32, 38)
    for pixel in buf.chunks_exact_mut(4) {
        pixel[0] = 38; // B
        pixel[1] = 32; // G
        pixel[2] = 30; // R
        pixel[3] = 255;
    }

    // 2. Window border (1px accent outline)
    for c in 0..w {
        set_pixel(buf, w, h, c, 0, [60, 68, 85, 255]);
        set_pixel(buf, w, h, c, h - 1, [60, 68, 85, 255]);
    }
    for r in 0..h {
        set_pixel(buf, w, h, 0, r, [60, 68, 85, 255]);
        set_pixel(buf, w, h, w - 1, r, [60, 68, 85, 255]);
    }

    // 3. Header bar (y: 1..38)
    for r in 1..38 {
        for c in 1..(w - 1) {
            set_pixel(buf, w, h, c, r, [45, 48, 58, 255]);
        }
    }
    draw_text(buf, w, h, 16, 14, "OCR Text Recognition - scroll-shot", [230, 235, 245, 255]);

    // Top-right close button [X]
    let cb_x = w - 34;
    for r in 8..30 {
        for c in cb_x..(cb_x + 22) {
            set_pixel(buf, w, h, c, r, [65, 70, 82, 255]);
        }
    }
    draw_text(buf, w, h, cb_x + 7, 14, "X", [220, 220, 220, 255]);

    // 4. Content Area
    match status {
        OcrStatus::Success { text, lines, engine_info, copied_feedback, copied_selection } => {
            // Status banner (Green/Blue)
            let sb_y = 48;
            let banner_color = if *copied_feedback {
                [35, 125, 65, 255]
            } else if !engine_info.supports_chinese {
                [50, 95, 160, 255]
            } else {
                [35, 115, 60, 255]
            };

            for r in sb_y..(sb_y + 28) {
                for c in 16..(w - 16) {
                    set_pixel(buf, w, h, c, r, banner_color);
                }
            }

            let banner_msg = if *copied_feedback {
                if *copied_selection {
                    "COPIED: Highlighted selection copied to clipboard"
                } else {
                    "SUCCESS: All text extracted & copied (Ctrl+V to paste)"
                }
            } else if !engine_info.supports_chinese {
                "SUCCESS: English model active (Chinese pack missing)"
            } else {
                "SUCCESS: Multi-lingual OCR active (chi_sim + eng)"
            };
            draw_text(buf, w, h, 24, sb_y + 10, banner_msg, [255, 255, 255, 255]);

            // Inset text preview box (y: 84..288)
            let box_y = 84;
            let box_h = 204;
            for r in box_y..(box_y + box_h) {
                for c in 16..(w - 16) {
                    set_pixel(buf, w, h, c, r, [22, 24, 28, 255]);
                }
            }
            // Box border
            for c in 16..(w - 16) {
                set_pixel(buf, w, h, c, box_y, [55, 60, 72, 255]);
                set_pixel(buf, w, h, c, box_y + box_h - 1, [55, 60, 72, 255]);
            }
            for r in box_y..(box_y + box_h) {
                set_pixel(buf, w, h, 16, r, [55, 60, 72, 255]);
                set_pixel(buf, w, h, w - 17, r, [55, 60, 72, 255]);
            }

            // Draw lines of text with selection highlighting
            let visible_lines = 9;
            let start_line = scroll_line_offset.min(lines.len().saturating_sub(1));

            for (screen_idx, line) in lines.iter().skip(start_line).take(visible_lines).enumerate() {
                let line_idx = start_line + screen_idx;
                let ty = box_y + 10 + screen_idx * 18;
                let mut cur_cx = 28;

                for (col_idx, ch) in line.chars().enumerate() {
                    let cw = if ch.is_ascii() { 8 } else { 16 };
                    let cx = cur_cx;
                    cur_cx += cw;
                    if cx + cw > w - 20 {
                        break;
                    }

                    let pos = TextPos { line: line_idx, col: col_idx };
                    let is_selected = selection.map_or(false, |(s, e)| pos >= s && pos < e);

                    if is_selected {
                        // Draw selection highlight background rectangle across cw
                        for py in (ty.saturating_sub(2))..(ty + 16) {
                            for px in cx..(cx + cw) {
                                set_pixel(buf, w, h, px, py, [45, 95, 210, 255]);
                            }
                        }
                        let mut char_buf = [0u8; 4];
                        let ch_str = ch.encode_utf8(&mut char_buf);
                        draw_text(buf, w, h, cx, ty, ch_str, [255, 255, 255, 255]);
                    } else {
                        let mut char_buf = [0u8; 4];
                        let ch_str = ch.encode_utf8(&mut char_buf);
                        draw_text(buf, w, h, cx, ty, ch_str, [215, 225, 240, 255]);
                    }
                }
            }

            // Status bar / hint at bottom of box
            let count_info = if !engine_info.supports_chinese {
                "Hint: Drag mouse to select. For Chinese: sudo apt install tesseract-ocr-chi-sim".to_string()
            } else {
                format!("Count: {} chars | Drag to select | Press [C] / Ctrl+C to copy", text.chars().count())
            };
            draw_text(buf, w, h, 28, box_y + box_h - 18, &count_info, [130, 145, 165, 255]);

            // Dynamic Copy button label
            let copy_btn_label = if *copied_feedback {
                "Copied! [C]"
            } else if selection.is_some() {
                "Copy Selection [C]"
            } else {
                "Copy All [C]"
            };

            // Buttons at bottom (y: 304..340)
            if !engine_info.supports_chinese {
                draw_button(buf, w, h, 20, 304, 170, 36, copy_btn_label, [45, 105, 215, 255]);
                draw_button(buf, w, h, 198, 304, 124, 36, "Get Chi Pack", [35, 125, 65, 255]);
                draw_button(buf, w, h, 330, 304, 90, 36, "Pin [P]", [100, 60, 165, 255]);
                draw_button(buf, w, h, 430, 304, 110, 36, "Done [ESC]", [60, 68, 80, 255]);
            } else {
                draw_button(buf, w, h, 20, 304, 170, 36, copy_btn_label, [45, 105, 215, 255]);
                draw_button(buf, w, h, 204, 304, 126, 36, "Pin [P]", [100, 60, 165, 255]);
                draw_button(buf, w, h, 420, 304, 120, 36, "Done [ESC]", [60, 68, 80, 255]);
            }
        }
        OcrStatus::MissingEngine { cmd_copied, img_copied } => {
            // Status banner (Amber/Red)
            let sb_y = 48;
            for r in sb_y..(sb_y + 28) {
                for c in 16..(w - 16) {
                    set_pixel(buf, w, h, c, r, [180, 50, 40, 255]);
                }
            }
            draw_text(buf, w, h, 24, sb_y + 10, "WARNING: Local Tesseract OCR engine is not installed", [255, 255, 255, 255]);

            // Inset message box
            let box_y = 84;
            let box_h = 204;
            for r in box_y..(box_y + box_h) {
                for c in 16..(w - 16) {
                    set_pixel(buf, w, h, c, r, [24, 26, 32, 255]);
                }
            }
            for c in 16..(w - 16) {
                set_pixel(buf, w, h, c, box_y, [65, 70, 85, 255]);
                set_pixel(buf, w, h, c, box_y + box_h - 1, [65, 70, 85, 255]);
            }
            for r in box_y..(box_y + box_h) {
                set_pixel(buf, w, h, 16, r, [65, 70, 85, 255]);
                set_pixel(buf, w, h, w - 17, r, [65, 70, 85, 255]);
            }

            draw_text(buf, w, h, 28, box_y + 16, "scroll-shot requires the offline tesseract engine to extract text.", [215, 220, 230, 255]);
            draw_text(buf, w, h, 28, box_y + 36, "Zero configuration required. Run this command in terminal:", [160, 170, 185, 255]);

            // Command highlight box (y: 150..184)
            let cmd_y = box_y + 64;
            for r in cmd_y..(cmd_y + 36) {
                for c in 28..(w - 28) {
                    set_pixel(buf, w, h, c, r, [15, 16, 20, 255]);
                }
            }
            draw_text(buf, w, h, 38, cmd_y + 14, INSTALL_CMD, [250, 215, 75, 255]);

            let hint_copied = if *cmd_copied {
                "Command copied! Open terminal and paste (Ctrl+Shift+V) to install."
            } else {
                "Click [Copy Command] below to copy command, then [Retry OCR]."
            };
            draw_text(buf, w, h, 28, box_y + 120, hint_copied, [130, 205, 140, 255]);

            if *img_copied {
                draw_text(buf, w, h, 28, box_y + 148, "(Screenshot image has been saved to clipboard as backup)", [140, 150, 165, 255]);
            }

            // Buttons
            let copy_btn_label = if *cmd_copied { "Copied! [C]" } else { "Copy Command" };
            draw_button(buf, w, h, 20, 304, 170, 36, copy_btn_label, [35, 125, 65, 255]);
            draw_button(buf, w, h, 200, 304, 110, 36, "Retry OCR", [45, 105, 215, 255]);
            draw_button(buf, w, h, 320, 304, 110, 36, "Copy Image", [100, 60, 165, 255]);
            draw_button(buf, w, h, 440, 304, 100, 36, "Close", [60, 68, 80, 255]);
        }
        OcrStatus::Empty { img_copied, .. } => {
            // Status banner (Yellow/Orange)
            let sb_y = 48;
            for r in sb_y..(sb_y + 28) {
                for c in 16..(w - 16) {
                    set_pixel(buf, w, h, c, r, [180, 110, 30, 255]);
                }
            }
            draw_text(buf, w, h, 24, sb_y + 10, "NOTICE: No text was recognized in the selected region", [255, 255, 255, 255]);

            // Inset box
            let box_y = 84;
            let box_h = 204;
            for r in box_y..(box_y + box_h) {
                for c in 16..(w - 16) {
                    set_pixel(buf, w, h, c, r, [24, 26, 32, 255]);
                }
            }
            draw_text(buf, w, h, 28, box_y + 24, "Please select an area containing clearer text or higher contrast.", [210, 215, 225, 255]);
            if *img_copied {
                draw_text(buf, w, h, 28, box_y + 60, "The screenshot image has been copied to your clipboard.", [140, 150, 165, 255]);
            }

            draw_button(buf, w, h, 20, 304, 140, 36, "Copy Image", [100, 60, 165, 255]);
            draw_button(buf, w, h, 420, 304, 120, 36, "Close [ESC]", [60, 68, 80, 255]);
        }
    }
}

fn draw_button(buf: &mut [u8], w: usize, h: usize, x: usize, y: usize, bw: usize, bh: usize, label: &str, bg: [u8; 4]) {
    for r in y..(y + bh) {
        for c in x..(x + bw) {
            set_pixel(buf, w, h, c, r, bg);
        }
    }
    // Border
    for c in x..(x + bw) {
        set_pixel(buf, w, h, c, y, [110, 120, 135, 255]);
        set_pixel(buf, w, h, c, y + bh - 1, [110, 120, 135, 255]);
    }
    for r in y..(y + bh) {
        set_pixel(buf, w, h, x, r, [110, 120, 135, 255]);
        set_pixel(buf, w, h, x + bw - 1, r, [110, 120, 135, 255]);
    }

    let text_len = label.chars().count();
    let tx = x + (bw.saturating_sub(text_len * 8)) / 2;
    let ty = y + (bh.saturating_sub(8)) / 2;
    draw_text(buf, w, h, tx, ty, label, [255, 255, 255, 255]);
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

fn paint_dialog<C: Connection>(
    conn: &C,
    win: Window,
    gc: x11rb::protocol::xproto::Gcontext,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_pos_ordering() {
        let p1 = TextPos { line: 0, col: 5 };
        let p2 = TextPos { line: 0, col: 10 };
        let p3 = TextPos { line: 1, col: 0 };
        assert!(p1 < p2);
        assert!(p2 < p3);
        assert!(p1 < p3);
        assert_eq!(p1, TextPos { line: 0, col: 5 });
    }

    #[test]
    fn test_extract_selected_text_single_line() {
        let lines = vec![
            "Hello World, Rust is awesome!".to_string(),
            "Second line here.".to_string(),
        ];
        // Select "World"
        let start = TextPos { line: 0, col: 6 };
        let end = TextPos { line: 0, col: 11 };
        let extracted = extract_selected_text(&lines, start, end);
        assert_eq!(extracted, "World");
    }

    #[test]
    fn test_extract_selected_text_multi_line() {
        let lines = vec![
            "Line One: Apple".to_string(),
            "Line Two: Banana".to_string(),
            "Line Three: Cherry".to_string(),
        ];
        // From "Apple" to "Line Three: Cher"
        let start = TextPos { line: 0, col: 10 };
        let end = TextPos { line: 2, col: 16 };
        let extracted = extract_selected_text(&lines, start, end);
        assert_eq!(
            extracted,
            "Apple\nLine Two: Banana\nLine Three: Cher"
        );
    }

    #[test]
    fn test_extract_selected_text_cjk() {
        let lines = vec![
            "这是中文测试，Hello World!".to_string(),
            "第二行数据".to_string(),
        ];
        // Select "中文测试" (cols 2..6 in chars)
        let start = TextPos { line: 0, col: 2 };
        let end = TextPos { line: 0, col: 6 };
        let extracted = extract_selected_text(&lines, start, end);
        assert_eq!(extracted, "中文测试");
    }

    #[test]
    fn test_mouse_to_text_pos_clamping() {
        let lines = vec!["Short".to_string()];
        // Far to the right
        let pos = mouse_to_text_pos(500, 100, 0, &lines);
        assert_eq!(pos.line, 0);
        assert_eq!(pos.col, 5); // clamped to length of "Short"

        // Negative/far to the left
        let pos = mouse_to_text_pos(10, 100, 0, &lines);
        assert_eq!(pos.line, 0);
        assert_eq!(pos.col, 0);
    }
}
