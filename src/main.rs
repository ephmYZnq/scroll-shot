mod cli;
mod gui;
mod stitcher;
mod x11;

use clap::Parser;
use cli::{CaptureRect, Cli};
use gui::overlay::{run_interactive_overlay, UserAction};
use gui::recorder::record_manual_scroll;
use std::error::Error;
use std::process::Command;
use x11::capture::capture_screen_rect;
use x11::clipboard::{copy_to_clipboard, run_clipboard_daemon};
use x11rb::connection::Connection;

fn main() -> Result<(), Box<dyn Error>> {
    let args = Cli::parse();

    // Check if invoked as internal clipboard daemon
    if args.serve_clipboard {
        return run_clipboard_daemon();
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
            println!(
                "📸 执行普通单张截图：区域 ({}, {}) {}x{}",
                rect.x, rect.y, rect.width, rect.height
            );

            let img = capture_screen_rect(&conn, root, &rect)?;

            // Copy directly to clipboard (no saving needed)
            copy_to_clipboard(&img)?;

            // Optional: only save if user explicitly passed --output
            if let Some(ref path) = args.output {
                img.save(path)?;
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

            let result = record_manual_scroll(&conn, screen_num, &rect)?;

            if let Some(final_img) = result {
                // Copy directly to clipboard (no saving needed)
                copy_to_clipboard(&final_img)?;

                // Optional: only save if user explicitly passed --output
                if let Some(ref path) = args.output {
                    final_img.save(path)?;
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
