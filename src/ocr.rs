use crate::x11::clipboard::encode_image_to_png;
use image::RgbaImage;
use std::error::Error;
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Default)]
pub struct OcrEngineInfo {
    #[allow(dead_code)]
    pub installed_languages: Vec<String>,
    pub lang_argument: Option<String>,
    pub supports_chinese: bool,
}

/// Query available languages and capabilities from tesseract
pub fn get_engine_info() -> Result<OcrEngineInfo, Box<dyn Error>> {
    let check = Command::new("tesseract")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    if check.is_err() || !check.unwrap().success() {
        return Err("Tesseract OCR engine is not installed on this system".into());
    }

    let output = Command::new("tesseract")
        .arg("--list-langs")
        .output()
        .map_err(|e| format!("Failed to list tesseract languages: {e}"))?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut langs: Vec<String> = text
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with("List of"))
        .collect();

    // Filter out "osd" (Orientation and script detection)
    langs.retain(|l| l != "osd");

    let supports_chinese = langs.iter().any(|l| l == "chi_sim" || l == "chi_tra");

    // Order languages: prioritize Chinese, then English, then others
    langs.sort_by_key(|l| match l.as_str() {
        "chi_sim" => 0,
        "chi_tra" => 1,
        "eng" => 2,
        _ => 3,
    });

    let lang_argument = if langs.is_empty() {
        None
    } else {
        Some(langs.join("+"))
    };

    Ok(OcrEngineInfo {
        installed_languages: langs,
        lang_argument,
        supports_chinese,
    })
}

/// Extract text from RgbaImage using local offline OCR engine (Tesseract)
pub fn extract_text_from_image(img: &RgbaImage) -> Result<(String, OcrEngineInfo), Box<dyn Error>> {
    let info = get_engine_info()?;

    // 1. Encode image to PNG in memory
    let png_bytes = encode_image_to_png(img)?;

    // 2. Spawn tesseract with stdin input and stdout output
    let mut cmd = Command::new("tesseract");
    cmd.args(["stdin", "stdout"]);
    if let Some(langs) = &info.lang_argument {
        cmd.args(["-l", langs]);
    }

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&png_bytes)?;
        stdin.flush()?;
    }

    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err("Tesseract failed to recognize image".into());
    }

    let recognized = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((recognized, info))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn test_ocr_availability_check() {
        let img = RgbaImage::from_pixel(10, 10, Rgba([255, 255, 255, 255]));
        let res = extract_text_from_image(&img);
        let _ = res.is_ok();
    }

    #[test]
    fn test_query_best_languages() {
        // Must run without panic whether tesseract is installed or not
        let _ = get_engine_info();
    }

    #[test]
    fn test_engine_info_details() {
        if let Ok(info) = get_engine_info() {
            assert!(info.installed_languages.contains(&"chi_sim".to_string()));
            assert!(info.installed_languages.contains(&"eng".to_string()));
            assert!(info.supports_chinese);
            assert_eq!(info.lang_argument, Some("chi_sim+eng".to_string()));
        }
    }
}
