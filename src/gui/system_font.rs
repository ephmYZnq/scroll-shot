use std::collections::HashMap;
use std::ffi::CString;
use std::sync::{Mutex, OnceLock};

#[repr(C)]
struct FT_Bitmap {
    rows: libc::c_uint,
    width: libc::c_uint,
    pitch: libc::c_int,
    buffer: *mut u8,
    num_grays: libc::c_ushort,
    pixel_mode: libc::c_uchar,
    palette_mode: libc::c_uchar,
    palette: *mut libc::c_void,
}

#[repr(C)]
struct FT_GlyphSlotRec {
    library: *mut libc::c_void,
    face: *mut libc::c_void,
    next: *mut libc::c_void,
    glyph_index: libc::c_uint,
    generic_data: *mut libc::c_void,
    generic_finalizer: *mut libc::c_void,
    metrics: [libc::c_long; 8],
    linear_hori_advance: libc::c_long,
    linear_vert_advance: libc::c_long,
    advance_x: libc::c_long,
    advance_y: libc::c_long,
    format: libc::c_uint,
    bitmap: FT_Bitmap,
    bitmap_left: libc::c_int,
    bitmap_top: libc::c_int,
}

#[repr(C)]
struct FT_FaceRec {
    num_faces: libc::c_long,
    face_index: libc::c_long,
    face_flags: libc::c_long,
    style_flags: libc::c_long,
    num_glyphs: libc::c_long,
    family_name: *mut libc::c_char,
    style_name: *mut libc::c_char,
    num_fixed_sizes: libc::c_int,
    available_sizes: *mut libc::c_void,
    num_charmaps: libc::c_int,
    charmaps: *mut libc::c_void,
    generic_data: *mut libc::c_void,
    generic_finalizer: *mut libc::c_void,
    bbox_x_min: libc::c_long,
    bbox_y_min: libc::c_long,
    bbox_x_max: libc::c_long,
    bbox_y_max: libc::c_long,
    units_per_em: libc::c_ushort,
    ascender: libc::c_short,
    descender: libc::c_short,
    height: libc::c_short,
    max_advance_width: libc::c_short,
    max_advance_height: libc::c_short,
    underline_position: libc::c_short,
    underline_thickness: libc::c_short,
    glyph: *mut FT_GlyphSlotRec,
}

#[allow(non_camel_case_types)]
type FT_Init_FreeType_Fn = unsafe extern "C" fn(*mut *mut libc::c_void) -> libc::c_int;
#[allow(non_camel_case_types)]
type FT_Done_FreeType_Fn = unsafe extern "C" fn(*mut libc::c_void) -> libc::c_int;
#[allow(non_camel_case_types)]
type FT_New_Face_Fn = unsafe extern "C" fn(
    *mut libc::c_void,
    *const libc::c_char,
    libc::c_long,
    *mut *mut FT_FaceRec,
) -> libc::c_int;
#[allow(non_camel_case_types)]
type FT_Done_Face_Fn = unsafe extern "C" fn(*mut FT_FaceRec) -> libc::c_int;
#[allow(non_camel_case_types)]
type FT_Set_Pixel_Sizes_Fn =
    unsafe extern "C" fn(*mut FT_FaceRec, libc::c_uint, libc::c_uint) -> libc::c_int;
#[allow(non_camel_case_types)]
type FT_Load_Char_Fn =
    unsafe extern "C" fn(*mut FT_FaceRec, libc::c_ulong, libc::c_int) -> libc::c_int;
#[allow(non_camel_case_types)]
type FT_Get_Char_Index_Fn =
    unsafe extern "C" fn(*mut FT_FaceRec, libc::c_ulong) -> libc::c_uint;

#[derive(Clone)]
pub struct CachedGlyph {
    pub width: u32,
    pub rows: u32,
    #[allow(dead_code)]
    pub left: i32,
    pub top: i32,
    pub advance: usize,
    pub buffer: Vec<u8>,
}

pub struct SystemFont {
    _lib_handle: *mut libc::c_void,
    ft_lib: *mut libc::c_void,
    face: *mut FT_FaceRec,
    ft_done_free_type: FT_Done_FreeType_Fn,
    ft_done_face: FT_Done_Face_Fn,
    ft_load_char: FT_Load_Char_Fn,
    ft_get_char_index: FT_Get_Char_Index_Fn,
    glyph_cache: HashMap<char, Option<CachedGlyph>>,
}

// Safety: SystemFont is wrapped in a Mutex so accessing its raw pointers is synchronized.
unsafe impl Send for SystemFont {}

impl Drop for SystemFont {
    fn drop(&mut self) {
        unsafe {
            if !self.face.is_null() {
                (self.ft_done_face)(self.face);
            }
            if !self.ft_lib.is_null() {
                (self.ft_done_free_type)(self.ft_lib);
            }
            if !self._lib_handle.is_null() {
                libc::dlclose(self._lib_handle);
            }
        }
    }
}

impl SystemFont {
    pub fn try_init() -> Option<Self> {
        let candidate_libs: &[&[u8]] = &[
            b"libfreetype.so.6\0",
            b"libfreetype.so\0",
            b"/usr/lib/x86_64-linux-gnu/libfreetype.so.6\0",
            b"/usr/lib64/libfreetype.so.6\0",
            b"/usr/lib/libfreetype.so.6\0",
        ];

        let mut lib_handle = std::ptr::null_mut();
        for lib in candidate_libs {
            lib_handle = unsafe { libc::dlopen(lib.as_ptr() as *const libc::c_char, libc::RTLD_LAZY) };
            if !lib_handle.is_null() {
                break;
            }
        }

        if lib_handle.is_null() {
            return None;
        }

        let ft_init: FT_Init_FreeType_Fn = unsafe {
            std::mem::transmute(libc::dlsym(lib_handle, b"FT_Init_FreeType\0".as_ptr() as *const _))
        };
        let ft_done: FT_Done_FreeType_Fn = unsafe {
            std::mem::transmute(libc::dlsym(lib_handle, b"FT_Done_FreeType\0".as_ptr() as *const _))
        };
        let ft_new_face: FT_New_Face_Fn = unsafe {
            std::mem::transmute(libc::dlsym(lib_handle, b"FT_New_Face\0".as_ptr() as *const _))
        };
        let ft_done_face: FT_Done_Face_Fn = unsafe {
            std::mem::transmute(libc::dlsym(lib_handle, b"FT_Done_Face\0".as_ptr() as *const _))
        };
        let ft_set_pixel_sizes: FT_Set_Pixel_Sizes_Fn = unsafe {
            std::mem::transmute(libc::dlsym(lib_handle, b"FT_Set_Pixel_Sizes\0".as_ptr() as *const _))
        };
        let ft_load_char: FT_Load_Char_Fn = unsafe {
            std::mem::transmute(libc::dlsym(lib_handle, b"FT_Load_Char\0".as_ptr() as *const _))
        };
        let ft_get_char_index: FT_Get_Char_Index_Fn = unsafe {
            std::mem::transmute(libc::dlsym(lib_handle, b"FT_Get_Char_Index\0".as_ptr() as *const _))
        };

        let mut ft_lib = std::ptr::null_mut();
        if unsafe { ft_init(&mut ft_lib) } != 0 || ft_lib.is_null() {
            unsafe { libc::dlclose(lib_handle) };
            return None;
        }

        let candidate_fonts = [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
            "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/opentype/source-han-sans/SourceHanSansCN-Regular.otf",
            "/usr/share/fonts/truetype/arphic/uming.ttc",
            "/usr/share/fonts/truetype/arphic/ukai.ttc",
        ];

        let mut face: *mut FT_FaceRec = std::ptr::null_mut();
        for font_path in &candidate_fonts {
            if std::path::Path::new(font_path).exists() {
                if let Ok(c_path) = CString::new(*font_path) {
                    let ret = unsafe { ft_new_face(ft_lib, c_path.as_ptr(), 0, &mut face) };
                    if ret == 0 && !face.is_null() {
                        break;
                    }
                }
            }
        }

        if face.is_null() {
            unsafe {
                ft_done(ft_lib);
                libc::dlclose(lib_handle);
            }
            return None;
        }

        // Set default pixel height (14px font height matches 18px line height)
        unsafe {
            ft_set_pixel_sizes(face, 0, 14);
        }

        Some(Self {
            _lib_handle: lib_handle,
            ft_lib,
            face,
            ft_done_free_type: ft_done,
            ft_done_face,
            ft_load_char,
            ft_get_char_index,
            glyph_cache: HashMap::new(),
        })
    }

    pub fn get_glyph(&mut self, ch: char) -> Option<CachedGlyph> {
        if let Some(cached) = self.glyph_cache.get(&ch) {
            return cached.clone();
        }

        // Glyph index 0 is .notdef (missing glyph box ☒); return None so it is never rendered
        let glyph_idx = unsafe { (self.ft_get_char_index)(self.face, ch as libc::c_ulong) };
        if glyph_idx == 0 {
            self.glyph_cache.insert(ch, None);
            return None;
        }

        // 4 = FT_LOAD_RENDER (renders grayscale bitmap directly)
        let ret = unsafe { (self.ft_load_char)(self.face, ch as libc::c_ulong, 4) };
        if ret != 0 {
            self.glyph_cache.insert(ch, None);
            return None;
        }

        let slot = unsafe { &*(*self.face).glyph };
        let bm = &slot.bitmap;

        let advance = if ch.is_ascii() {
            8
        } else {
            16
        };

        if bm.width == 0 || bm.rows == 0 {
            let empty_glyph = CachedGlyph {
                width: 0,
                rows: 0,
                left: 0,
                top: 0,
                advance,
                buffer: Vec::new(),
            };
            self.glyph_cache.insert(ch, Some(empty_glyph.clone()));
            return Some(empty_glyph);
        }

        let pitch = bm.pitch.abs() as usize;
        let mut buffer = Vec::with_capacity((bm.width * bm.rows) as usize);
        for r in 0..(bm.rows as usize) {
            let row_slice = unsafe {
                std::slice::from_raw_parts(bm.buffer.add(r * pitch), bm.width as usize)
            };
            buffer.extend_from_slice(row_slice);
        }

        let glyph = CachedGlyph {
            width: bm.width,
            rows: bm.rows,
            left: slot.bitmap_left,
            top: slot.bitmap_top,
            advance,
            buffer,
        };

        self.glyph_cache.insert(ch, Some(glyph.clone()));
        Some(glyph)
    }
}

static GLOBAL_FONT: OnceLock<Mutex<Option<SystemFont>>> = OnceLock::new();

fn get_global_font() -> &'static Mutex<Option<SystemFont>> {
    GLOBAL_FONT.get_or_init(|| Mutex::new(SystemFont::try_init()))
}

/// Render a character using SystemFont if available with antialiasing.
/// Returns the advance width of the character in pixels.
pub fn draw_system_char(
    buf: &mut [u8],
    buf_w: usize,
    buf_h: usize,
    x: usize,
    baseline_y: usize,
    ch: char,
    color: [u8; 4],
) -> Option<usize> {
    let font_mutex = get_global_font();
    let mut guard = font_mutex.lock().ok()?;
    let font = guard.as_mut()?;
    let glyph = font.get_glyph(ch)?;

    if glyph.width > 0 && glyph.rows > 0 {
        let left_offset = if ch.is_ascii() {
            (8i32 - glyph.width as i32) / 2
        } else {
            (16i32 - glyph.width as i32) / 2
        };

        for r in 0..(glyph.rows as usize) {
            let target_y = baseline_y as i32 - glyph.top + r as i32;
            if target_y < 0 {
                continue;
            }
            let py = target_y as usize;
            if py >= buf_h {
                continue;
            }

            for c in 0..(glyph.width as usize) {
                let alpha = glyph.buffer[r * glyph.width as usize + c] as u32;
                if alpha == 0 {
                    continue;
                }

                let target_x = x as i32 + left_offset.max(0) + c as i32;
                if target_x < 0 {
                    continue;
                }
                let px = target_x as usize;
                if px >= buf_w {
                    continue;
                }

                let idx = (py * buf_w + px) * 4;
                if idx + 3 < buf.len() {
                    let inv = 255 - alpha;
                    let b = ((buf[idx] as u32 * inv + color[2] as u32 * alpha) / 255) as u8;
                    let g = ((buf[idx + 1] as u32 * inv + color[1] as u32 * alpha) / 255) as u8;
                    let r_col = ((buf[idx + 2] as u32 * inv + color[0] as u32 * alpha) / 255) as u8;
                    buf[idx] = b;
                    buf[idx + 1] = g;
                    buf[idx + 2] = r_col;
                    buf[idx + 3] = 255;
                }
            }
        }
    }

    Some(glyph.advance)
}

/// Calculate advance width for a character
#[allow(dead_code)]
pub fn char_advance(ch: char) -> usize {
    if ch.is_ascii() {
        8
    } else {
        16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_freetype_system_font_loads_and_renders_cjk() {
        let font_opt = SystemFont::try_init();
        if let Some(mut font) = font_opt {
            // Test ASCII
            let g_ascii = font.get_glyph('A');
            assert!(g_ascii.is_some());
            assert_eq!(g_ascii.unwrap().advance, 8);

            // Test Chinese character '字'
            let g_cjk = font.get_glyph('字');
            assert!(g_cjk.is_some(), "Chinese glyph '字' should be successfully loaded");
            let cjk = g_cjk.unwrap();
            assert_eq!(cjk.advance, 16);
            assert!(cjk.width > 0);
            assert!(cjk.rows > 0);
            assert!(!cjk.buffer.is_empty());
        }
    }

    #[test]
    fn test_draw_system_char_buffer() {
        let mut buf = vec![0u8; 100 * 100 * 4];
        let adv = draw_system_char(&mut buf, 100, 100, 10, 30, '中', [255, 255, 255, 255]);
        if adv.is_some() {
            assert_eq!(adv.unwrap(), 16);
            // Verify at least one pixel is non-zero
            assert!(buf.iter().any(|&b| b > 0));
        }
    }

    #[test]
    fn test_render_chinese_text_phrase() {
        let phrase = "2. 字母大小写保持原样";
        let mut buf = vec![0u8; 300 * 50 * 4];
        let mut cur_x = 10;
        for ch in phrase.chars() {
            if let Some(adv) = draw_system_char(&mut buf, 300, 50, cur_x, 30, ch, [255, 255, 255, 255]) {
                cur_x += adv;
            } else {
                cur_x += if ch.is_ascii() { 8 } else { 16 };
            }
        }
        assert_eq!(cur_x, 178);
        let rendered_pixels = buf.chunks_exact(4).filter(|p| p[0] > 0 || p[1] > 0 || p[2] > 0).count();
        assert!(rendered_pixels > 200, "Should have rendered at least 200 visible pixels for the Chinese phrase");
    }

    #[test]
    fn test_missing_glyph_returns_none_instead_of_notdef() {
        let font_opt = SystemFont::try_init();
        if let Some(mut font) = font_opt {
            // Emojis not in CJK font must return None instead of .notdef (☒)
            assert!(font.get_glyph('📸').is_none());
            assert!(font.get_glyph('📌').is_none());
        }
    }
}
