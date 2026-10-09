mod cli;
mod gui;
mod stitcher;
mod x11;

use chrono::Local;
use clap::Parser;
use cli::{CaptureRect, Cli, Preset};
use gui::overlay::{run_interactive_overlay, UserAction};
use image::RgbaImage;
use std::error::Error;
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};
use stitcher::{find_vertical_shift, frames_settled, LongCanvas};
use x11::capture::capture_screen_rect;
use x11::input::{activate_window_at, calculate_cursor_coords, inject_scroll_down};
use x11rb::connection::Connection;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = Cli::parse();

    // Apply preset overrides if applicable
    if args.preset == Preset::Vm {
        if args.delay == 80 {
            args.delay = 140; // Slightly higher base debounce for VM rendering
        }
        if args.settle_timeout == 350 {
            args.settle_timeout = 500;
        }
    }

    // Connect to X11 server
    let (conn, screen_num) = x11rb::connect(None)?;
    let setup = conn.setup();
    let screen = &setup.roots[screen_num];
    let root = screen.root;

    // Determine action: either from CLI flags or from GUI interactive overlay
    let action = if let Some(ref rect_str) = args.rect {
        let rect = CaptureRect::parse(rect_str)?;
        if args.single {
            UserAction::SingleShot(rect)
        } else {
            UserAction::ScrollShot(rect)
        }
    } else {
        // Run full-screen interactive freeze-frame overlay with handles and toolbar
        run_interactive_overlay(&conn, screen_num)?
    };

    match action {
        UserAction::Cancel => {
            println!("❌ 用户取消了截图。");
            return Ok(());
        }
        UserAction::SingleShot(rect) => {
            let output_path = resolve_output_path(args.output.as_deref(), "single");
            println!(
                "📸 执行普通单张截图：区域 ({}, {}) {}x{}，输出: {}",
                rect.x, rect.y, rect.width, rect.height, output_path
            );

            let img = capture_screen_rect(&conn, root, &rect)?;
            img.save(&output_path)?;

            send_desktop_notification("普通截图已完成", &format!("已保存至: {}", output_path));

            println!(
                "🎉 普通截图保存成功！\n   尺寸: {}x{} 像素\n   路径: {}",
                img.width(),
                img.height(),
                output_path
            );
        }
        UserAction::ScrollShot(rect) => {
            let output_path = resolve_output_path(args.output.as_deref(), "long");
            println!(
                "🚀 启动滚动长截图：目标区域 ({}, {}) {}x{}，输出: {}",
                rect.x, rect.y, rect.width, rect.height, output_path
            );

            // Calculate cursor anchor coordinate
            let (cursor_x, cursor_y) = calculate_cursor_coords(&rect, args.cursor_pos);
            if args.verbose {
                println!("📍 滚轮注入光标位置: ({}, {})", cursor_x, cursor_y);
            }

            // CRITICAL: Activate and focus the target window before scrolling!
            activate_window_at(&conn, root, cursor_x, cursor_y)?;

            // Capture initial frame (Frame 0)
            let frame_0 = capture_screen_rect(&conn, root, &rect)?;
            let mut canvas = LongCanvas::new(&frame_0);
            let mut prev_frame = frame_0;

            let mut consecutive_zero_shifts = 0;
            let max_height = (rect.height as u32) * args.max_screens;
            let mut step_count = 0;

            println!("📸 正在执行自动滚动与像素级缝合（按 Ctrl+C 可随时停止）...");

            loop {
                step_count += 1;

                // 1. Inject scroll wheel event
                inject_scroll_down(&conn, root, cursor_x, cursor_y, args.step)?;

                // 2. Wait base debounce delay
                sleep(Duration::from_millis(args.delay));

                // 3. Settling detection (wait for smooth scroll animation to stabilize)
                let curr_frame = capture_stable_frame(
                    &conn,
                    root,
                    &rect,
                    args.settle_timeout,
                    args.preset == Preset::Vm,
                )?;

                // 4. Calculate 1D vertical displacement with Full Overlap Verification
                let match_res = find_vertical_shift(&prev_frame, &curr_frame, None);

                if args.verbose {
                    println!(
                        "   [步数 #{:02}] 垂直位移: {:3}px, 比对误差: {:.2}, 当前总高: {}px",
                        step_count,
                        match_res.delta_y,
                        match_res.avg_diff,
                        canvas.height
                    );
                }

                // 5. Evaluate bottom detection
                if match_res.delta_y == 0 {
                    consecutive_zero_shifts += 1;
                    if consecutive_zero_shifts >= 2 {
                        println!("🏁 检测到页面已到底部（无有效新位移），自动完成截图。");
                        break;
                    }
                } else {
                    consecutive_zero_shifts = 0;

                    // 6. Lossless row slice append
                    let start_y = (rect.height as u32).saturating_sub(match_res.delta_y);
                    canvas.append_rows(&curr_frame, start_y, match_res.delta_y);
                    prev_frame = curr_frame;
                }

                // 7. Safety upper limit check
                if canvas.height >= max_height {
                    println!("⚠️ 已达到设定的最大截屏高度限制（{} 屏），停止录制。", args.max_screens);
                    break;
                }
            }

            // Finalize and save
            let final_img = canvas
                .into_image()
                .ok_or("Failed to convert canvas buffer into image")?;

            final_img.save(&output_path)?;

            send_desktop_notification("长截图已完成", &format!("已保存至: {}", output_path));

            println!(
                "🎉 长截图保存成功！\n   尺寸: {}x{} 像素\n   路径: {}",
                final_img.width(),
                final_img.height(),
                output_path
            );
        }
    }

    Ok(())
}

fn resolve_output_path(user_output: Option<&str>, prefix: &str) -> String {
    if let Some(p) = user_output {
        p.to_string()
    } else {
        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        format!("scroll_shot_{}_{}.png", prefix, timestamp)
    }
}

fn send_desktop_notification(title: &str, body: &str) {
    let _ = Command::new("notify-send")
        .args(["-a", "scroll-shot", title, body])
        .spawn();
}

/// Capture frame and optionally wait for motion settling
fn capture_stable_frame<C: x11rb::connection::Connection>(
    conn: &C,
    root: x11rb::protocol::xproto::Window,
    rect: &CaptureRect,
    settle_timeout_ms: u64,
    enable_settling_check: bool,
) -> Result<RgbaImage, Box<dyn Error>> {
    let mut frame = capture_screen_rect(conn, root, rect)?;

    if !enable_settling_check || settle_timeout_ms == 0 {
        return Ok(frame);
    }

    let start = Instant::now();
    let max_duration = Duration::from_millis(settle_timeout_ms);

    while start.elapsed() < max_duration {
        sleep(Duration::from_millis(25));
        let next_frame = capture_screen_rect(conn, root, rect)?;
        if frames_settled(&frame, &next_frame) {
            return Ok(next_frame);
        }
        frame = next_frame;
    }

    Ok(frame)
}
