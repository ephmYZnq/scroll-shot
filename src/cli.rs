use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CursorPos {
    /// Place cursor near the right margin to avoid obstructing content
    Right,
    /// Place cursor at the center of the region
    Center,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Preset {
    /// Default native desktop apps (optimized for low latency)
    Native,
    /// Optimized for virtual machines (settling detection, right-edge cursor)
    Vm,
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "scroll-shot",
    author = "scroll-shot contributors",
    version,
    about = "Pixel-perfect, high-resolution scrolling screenshot tool for Linux X11"
)]
pub struct Cli {
    /// Output file path (defaults to scroll_shot_<timestamp>.png)
    #[arg(short, long)]
    pub output: Option<String>,

    /// Rectangle area to capture: x,y,width,height (skips interactive selection)
    #[arg(short, long)]
    pub rect: Option<String>,

    /// Base delay in milliseconds after scrolling before capturing next frame
    #[arg(short, long, default_value_t = 80)]
    pub delay: u64,

    /// Maximum timeout in ms to wait for frame to settle (smooth scrolling stabilization)
    #[arg(long, default_value_t = 350)]
    pub settle_timeout: u64,

    /// Cursor placement position when injecting mouse scroll
    #[arg(long, value_enum, default_value_t = CursorPos::Right)]
    pub cursor_pos: CursorPos,

    /// Number of scroll wheel steps per iteration
    #[arg(short, long, default_value_t = 2)]
    pub step: u32,

    /// Maximum number of screens/height multiples to capture (safety limit)
    #[arg(short, long, default_value_t = 25)]
    pub max_screens: u32,

    /// Optimization preset (native or vm)
    #[arg(long, value_enum, default_value_t = Preset::Native)]
    pub preset: Preset,

    /// Take a single snapshot of the selected area without scrolling
    #[arg(long)]
    pub single: bool,

    /// Internal clipboard daemon mode
    #[arg(long, hide = true)]
    pub serve_clipboard: bool,

    /// Print detailed matching and displacement logs
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureRect {
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

impl CaptureRect {
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
        if parts.len() != 4 {
            return Err("Expected format: x,y,width,height (e.g. 100,200,800,600)".to_string());
        }
        let x = parts[0]
            .parse::<i16>()
            .map_err(|e| format!("Invalid x: {e}"))?;
        let y = parts[1]
            .parse::<i16>()
            .map_err(|e| format!("Invalid y: {e}"))?;
        let width = parts[2]
            .parse::<u16>()
            .map_err(|e| format!("Invalid width: {e}"))?;
        let height = parts[3]
            .parse::<u16>()
            .map_err(|e| format!("Invalid height: {e}"))?;

        if width == 0 || height == 0 {
            return Err("Width and height must be greater than 0".to_string());
        }

        Ok(CaptureRect {
            x,
            y,
            width,
            height,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_rect() {
        let r = CaptureRect::parse("10, 20, 800, 600").unwrap();
        assert_eq!(r.x, 10);
        assert_eq!(r.y, 20);
        assert_eq!(r.width, 800);
        assert_eq!(r.height, 600);
    }

    #[test]
    fn test_parse_invalid_rect() {
        assert!(CaptureRect::parse("10, 20, 800").is_err());
        assert!(CaptureRect::parse("10, 20, 0, 600").is_err());
        assert!(CaptureRect::parse("foo, 20, 800, 600").is_err());
    }
}
