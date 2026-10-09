mod cli;
mod gui;
mod stitcher;
mod x11;

use clap::Parser;
use cli::{CaptureRect, Cli};
use gui::overlay::{run_interactive_overlay, UserAction};
use gui::recorder::record_manual_scroll;
use image::RgbaImage;
use std::error::Error;
use std::process::Command;
use x11::capture::capture_screen_rect;
use x11::clipboard::{copy_to_clipboard, encode_image_to_png, run_clipboard_daemon};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask};
use x11rb::protocol::Event;

fn main() -> Result<(), Box<dyn Error>> {
    let args = Cli::parse();

    // 1. Check if invoked as internal clipboard daemon
    if args.serve_clipboard {
        return run_clipboard_daemon();
    }

    // Connect to X11 server
    let (conn, screen_num) = x11rb::connect(None)?;

    // 2. Check if invoked as background hotkey daemon
    if args.daemon {
        return run_hotkey_daemon(&conn, screen_num, &args);
    }

    // 3. Normal single run
    let (action, pre_captured) = if let Some(ref rect_str) = args.rect {
        let rect = CaptureRect::parse(rect_str)?;
        let action = if args.single {
            UserAction::SingleShot(rect)
        } else {
            UserAction::ScrollShot(rect)
        };
        (action, None)
    } else {
        // Run full-screen interactive freeze-frame overlay with handles and toolbar
        run_interactive_overlay(&conn, screen_num)?
    };

    handle_action(&conn, screen_num, action, pre_captured, &args)?;
    Ok(())
}

/// Global hotkey daemon: grabs Ctrl+1 and listens continuously
fn run_hotkey_daemon<C: Connection>(
    conn: &C,
    screen_num: usize,
    args: &Cli,
) -> Result<(), Box<dyn Error>> {
    let setup = conn.setup();
    let screen = &setup.roots[screen_num];
    let root = screen.root;

    // Grab Ctrl+1 across common lock states (NumLock = M2, CapsLock = LOCK)
    let modifiers = [
        ModMask::CONTROL,
        ModMask::CONTROL | ModMask::LOCK,
        ModMask::CONTROL | ModMask::M2,
        ModMask::CONTROL | ModMask::LOCK | ModMask::M2,
    ];

    for m in modifiers {
        conn.grab_key(
            true,
            root,
            m,
            10, // Keycode 10 is '1' on standard X11 keyboard layouts
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )?;
    }
    conn.flush()?;

    println!("🔥 scroll-shot 常驻快捷键服务已启动！");
    println!("👉 无论在任何窗口，按下 【Ctrl + 1】 即可直接呼出截图工具！(按 Ctrl+C 退出)");

    loop {
        conn.flush()?;
        let event = conn.wait_for_event()?;
        if let Event::KeyPress(ev) = event {
            if ev.detail == 10 {
                // Key '1' pressed with Ctrl modifier!
                if let Ok((action, pre_captured)) = run_interactive_overlay(conn, screen_num) {
                    let _ = handle_action(conn, screen_num, action, pre_captured, args);
                }
            }
        }
    }
}

fn handle_action<C: Connection>(
    conn: &C,
    screen_num: usize,
    action: UserAction,
    pre_captured: Option<RgbaImage>,
    args: &Cli,
) -> Result<(), Box<dyn Error>> {
    let setup = conn.setup();
    let screen = &setup.roots[screen_num];
    let root = screen.root;

    match action {
        UserAction::Cancel => {
            println!("❌ 用户取消了截图。");
        }
        UserAction::SingleShot(rect) => {
            println!(
                "📸 执行普通单张截图：区域 ({}, {}) {}x{}",
                rect.x, rect.y, rect.width, rect.height
            );

            // Use pristine frozen image if captured from overlay, otherwise capture from screen
            let img = match pre_captured {
                Some(p) => p,
                None => capture_screen_rect(conn, root, &rect)?,
            };

            // Copy directly to clipboard (no saving needed)
            let png_bytes = encode_image_to_png(&img)?;
            copy_to_clipboard(&img)?;

            // Optional: only save if user explicitly passed --output
            if let Some(ref path) = args.output {
                std::fs::write(path, &png_bytes)?;
                println!("💾 已另存为文件: {}", path);
            }

            send_desktop_notification(
                "截图已复制到剪切板",
                &format!("尺寸: {}x{} 像素，直接 Ctrl+V 即可粘贴！", img.width(), img.height()),
            );

            println!(
                "🎉 截图已直接放入系统剪贴板！(尺寸: {}x{}) 无需保存，可直接 Ctrl+V 粘贴使用。",
                img.width(),
                img.height()
            );
        }
        UserAction::ScrollShot(rect) => {
            println!(
                "📜 进入用户手动滚动模式：区域 ({}, {}) {}x{}",
                rect.x, rect.y, rect.width, rect.height
            );
            println!("👉 请随意向下滚动页面，滚动完毕后点击悬浮条上的【Done】按钮或按回车键！");

            let result = record_manual_scroll(conn, screen_num, &rect, pre_captured)?;

            if let Some(final_img) = result {
                // Copy directly to clipboard (no saving needed)
                let png_bytes = encode_image_to_png(&final_img)?;
                copy_to_clipboard(&final_img)?;

                // Optional: only save if user explicitly passed --output
                if let Some(ref path) = args.output {
                    std::fs::write(path, &png_bytes)?;
                    println!("💾 已另存为文件: {}", path);
                }

                send_desktop_notification(
                    "长截图已复制到剪切板",
                    &format!(
                        "总尺寸: {}x{} 像素，直接 Ctrl+V 即可粘贴！",
                        final_img.width(),
                        final_img.height()
                    ),
                );

                println!(
                    "🎉 长截图已直接放入系统剪贴板！\n   尺寸: {}x{} 像素\n   无需保存，可直接 Ctrl+V 粘贴使用！",
                    final_img.width(),
                    final_img.height()
                );
            } else {
                println!("❌ 用户取消了长截图。");
            }
        }
    }

    Ok(())
}

fn send_desktop_notification(title: &str, body: &str) {
    let _ = Command::new("notify-send")
        .args(["-a", "scroll-shot", title, body])
        .spawn();
}
