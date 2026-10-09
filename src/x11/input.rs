use crate::cli::{CaptureRect, CursorPos};
use std::error::Error;
use std::thread::sleep;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::Window;
use x11rb::protocol::xtest;

/// Calculate the appropriate cursor coordinate based on CursorPos policy
pub fn calculate_cursor_coords(rect: &CaptureRect, pos: CursorPos) -> (i16, i16) {
    match pos {
        CursorPos::Center => (
            rect.x + (rect.width / 2) as i16,
            rect.y + (rect.height / 2) as i16,
        ),
        CursorPos::Right => {
            // Keep cursor 24 pixels from the right edge to avoid obstructing text content,
            // while staying reliably within the scrollable viewport.
            let margin = if rect.width > 60 { 24 } else { 4 };
            (
                rect.x + (rect.width as i16) - margin,
                rect.y + (rect.height / 2) as i16,
            )
        }
    }
}

/// Move mouse pointer and inject scroll down event (Button 5) using XTest extension
pub fn inject_scroll_down<C: Connection>(
    conn: &C,
    root: Window,
    target_x: i16,
    target_y: i16,
    steps: u32,
) -> Result<(), Box<dyn Error>> {
    // 6 = MotionNotify in X11
    xtest::fake_input(conn, 6, 0, 0, root, target_x, target_y, 0)?;
    conn.flush()?;

    // Send Button 5 press & release events for each step
    for _ in 0..steps {
        // 4 = ButtonPress, 5 = Button 5 (Wheel Down)
        xtest::fake_input(conn, 4, 5, 0, root, target_x, target_y, 0)?;
        // 5 = ButtonRelease
        xtest::fake_input(conn, 5, 5, 0, root, target_x, target_y, 0)?;
        conn.flush()?;
        // Small interval between wheel ticks to allow target window to process
        sleep(Duration::from_millis(12));
    }

    Ok(())
}

/// Send mouse click to activate and focus the target window before scrolling
pub fn activate_window_at<C: Connection>(
    conn: &C,
    root: Window,
    target_x: i16,
    target_y: i16,
) -> Result<(), Box<dyn Error>> {
    // Move to target
    xtest::fake_input(conn, 6, 0, 0, root, target_x, target_y, 0)?;
    conn.flush()?;
    sleep(Duration::from_millis(30));

    // Button 1 Click
    xtest::fake_input(conn, 4, 1, 0, root, target_x, target_y, 0)?;
    xtest::fake_input(conn, 5, 1, 0, root, target_x, target_y, 0)?;
    conn.flush()?;
    sleep(Duration::from_millis(60));

    Ok(())
}
