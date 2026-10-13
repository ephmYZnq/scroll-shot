use crate::cli::CaptureRect;
use image::RgbaImage;
use std::error::Error;
use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::sync::Mutex;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat, Window};

/// Capture raw BGRA bytes (B, G, R, A/X) for a rectangle from X11 root window,
/// automatically falling back to GNOME Wayland compositor D-Bus capture under Wayland.
pub fn capture_raw_bgra_rect<C: Connection>(
    conn: &C,
    root: Window,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
) -> Result<Vec<u8>, Box<dyn Error>> {
    if let Ok( cookie) = conn.get_image(ImageFormat::Z_PIXMAP, root, x, y, width, height, !0) {
        if let Ok(reply) = cookie.reply() {
            let expected_len = (width as usize) * (height as usize) * 4;
            if reply.data.len() >= expected_len {
                let mut out = vec![0u8; expected_len];
                out.copy_from_slice(&reply.data[..expected_len]);
                return Ok(out);
            }
        }
    }

    // Fallback for Wayland (Xwayland root window returns BadMatch on GetImage)
    let rgba = capture_wayland_rect(x as i32, y as i32, width as u32, height as u32)?;
    let raw_rgba = rgba.into_raw();
    let mut bgra = vec![0u8; raw_rgba.len()];
    for (src, dst) in raw_rgba.chunks_exact(4).zip(bgra.chunks_exact_mut(4)) {
        dst[0] = src[2]; // B
        dst[1] = src[1]; // G
        dst[2] = src[0]; // R
        dst[3] = 255;    // A
    }
    Ok(bgra)
}

/// Capture a specified rectangle area from the screen and convert it to an RgbaImage.
/// Supports both X11 (zero-copy ZPixmap) and GNOME Wayland (compositor D-Bus capture).
pub fn capture_screen_rect<C: Connection>(
    conn: &C,
    root: Window,
    rect: &CaptureRect,
) -> Result<RgbaImage, Box<dyn Error>> {
    // 1. Try fast native X11 root window GetImage first
    if let Ok(cookie) = conn.get_image(
        ImageFormat::Z_PIXMAP,
        root,
        rect.x,
        rect.y,
        rect.width,
        rect.height,
        !0,
    ) {
        if let Ok(reply) = cookie.reply() {
            let width = rect.width as u32;
            let height = rect.height as u32;
            let expected_len = (width * height * 4) as usize;

            if reply.data.len() >= expected_len {
                let mut img = RgbaImage::new(width, height);
                let raw_pixels = &reply.data[..expected_len];
                let img_buf = img.as_mut();

                for (src, dst) in raw_pixels.chunks_exact(4).zip(img_buf.chunks_exact_mut(4)) {
                    dst[0] = src[2]; // Red
                    dst[1] = src[1]; // Green
                    dst[2] = src[0]; // Blue
                    dst[3] = 255;    // Alpha
                }
                return Ok(img);
            }
        }
    }

    // 2. Fallback to GNOME Wayland compositor capture
    capture_wayland_rect(
        rect.x as i32,
        rect.y as i32,
        rect.width as u32,
        rect.height as u32,
    )
}

// ============================================================================
// GNOME Wayland Compositor Screen Capture via GIO D-Bus (libgio-2.0.so.0)
// ============================================================================

#[repr(C)]
struct GError {
    domain: u32,
    code: c_int,
    message: *const c_char,
}

type GBusGetSyncFn = unsafe extern "C" fn(
    bus_type: c_int,
    cancellable: *mut c_void,
    error: *mut *mut GError,
) -> *mut c_void;

type GDBusConnectionCallSyncFn = unsafe extern "C" fn(
    connection: *mut c_void,
    bus_name: *const c_char,
    object_path: *const c_char,
    interface_name: *const c_char,
    method_name: *const c_char,
    parameters: *mut c_void,
    reply_type: *const c_void,
    flags: c_int,
    timeout_msec: c_int,
    cancellable: *mut c_void,
    error: *mut *mut GError,
) -> *mut c_void;

type GVariantNewFn = unsafe extern "C" fn(format_string: *const c_char, ...) -> *mut c_void;
type GVariantUnrefFn = unsafe extern "C" fn(value: *mut c_void);
type GErrorFreeFn = unsafe extern "C" fn(error: *mut GError);

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

const RTLD_LAZY: c_int = 1;
const G_BUS_TYPE_SESSION: c_int = 2;

struct WaylandDbusClient {
    conn: *mut c_void,
    call_sync: GDBusConnectionCallSyncFn,
    variant_new: GVariantNewFn,
    variant_unref: GVariantUnrefFn,
    error_free: GErrorFreeFn,
}

unsafe impl Send for WaylandDbusClient {}

static WAYLAND_CLIENT: Mutex<Option<WaylandDbusClient>> = Mutex::new(None);

impl WaylandDbusClient {
    fn init() -> Result<Self, String> {
        unsafe {
            let gio_name = CString::new("libgio-2.0.so.0").unwrap();
            let glib_name = CString::new("libglib-2.0.so.0").unwrap();

            let gio_handle = dlopen(gio_name.as_ptr(), RTLD_LAZY);
            let glib_handle = dlopen(glib_name.as_ptr(), RTLD_LAZY);
            if gio_handle.is_null() || glib_handle.is_null() {
                return Err("Failed to load libgio-2.0.so.0 / libglib-2.0.so.0".to_string());
            }

            let bus_get_sync_ptr = dlsym(gio_handle, b"g_bus_get_sync\0".as_ptr() as *const c_char);
            let call_sync_ptr = dlsym(
                gio_handle,
                b"g_dbus_connection_call_sync\0".as_ptr() as *const c_char,
            );
            let variant_new_ptr = dlsym(glib_handle, b"g_variant_new\0".as_ptr() as *const c_char);
            let variant_unref_ptr =
                dlsym(glib_handle, b"g_variant_unref\0".as_ptr() as *const c_char);
            let error_free_ptr = dlsym(glib_handle, b"g_error_free\0".as_ptr() as *const c_char);

            if bus_get_sync_ptr.is_null()
                || call_sync_ptr.is_null()
                || variant_new_ptr.is_null()
                || variant_unref_ptr.is_null()
                || error_free_ptr.is_null()
            {
                return Err("Missing required GDBus/GVariant symbols".to_string());
            }

            let g_bus_get_sync: GBusGetSyncFn = std::mem::transmute(bus_get_sync_ptr);
            let call_sync: GDBusConnectionCallSyncFn = std::mem::transmute(call_sync_ptr);
            let variant_new: GVariantNewFn = std::mem::transmute(variant_new_ptr);
            let variant_unref: GVariantUnrefFn = std::mem::transmute(variant_unref_ptr);
            let error_free: GErrorFreeFn = std::mem::transmute(error_free_ptr);

            let mut err: *mut GError = std::ptr::null_mut();
            let conn = g_bus_get_sync(G_BUS_TYPE_SESSION, std::ptr::null_mut(), &mut err);
            if conn.is_null() {
                let msg = if !err.is_null() && !(*err).message.is_null() {
                    let s = CStr::from_ptr((*err).message).to_string_lossy().into_owned();
                    error_free(err);
                    s
                } else {
                    "Unknown D-Bus session connection error".to_string()
                };
                return Err(format!("g_bus_get_sync failed: {}", msg));
            }

            // Claim 'org.gnome.Screenshot' on the session bus so GNOME Shell's
            // DBusSenderChecker authorizes direct ScreenshotArea calls without prompts.
            // Flags 7 = ALLOW_REPLACEMENT (1) | REPLACE_EXISTING (2) | DO_NOT_QUEUE (4)
            let params = variant_new(
                b"(su)\0".as_ptr() as *const c_char,
                b"org.gnome.Screenshot\0".as_ptr() as *const c_char,
                7 as c_uint,
            );
            let mut req_err: *mut GError = std::ptr::null_mut();
            let reply = call_sync(
                conn,
                b"org.freedesktop.DBus\0".as_ptr() as *const c_char,
                b"/org/freedesktop/DBus\0".as_ptr() as *const c_char,
                b"org.freedesktop.DBus\0".as_ptr() as *const c_char,
                b"RequestName\0".as_ptr() as *const c_char,
                params,
                std::ptr::null(),
                0,
                2000,
                std::ptr::null_mut(),
                &mut req_err,
            );
            if !reply.is_null() {
                variant_unref(reply);
            }
            if !req_err.is_null() {
                error_free(req_err);
            }

            // Brief pause on first init so gnome-shell's NameOwnerChanged watcher updates allowlist
            std::thread::sleep(Duration::from_millis(20));

            Ok(Self {
                conn,
                call_sync,
                variant_new,
                variant_unref,
                error_free,
            })
        }
    }

    fn screenshot_area(&self, x: i32, y: i32, w: i32, h: i32, out_path: &str) -> Result<(), String> {
        let path_cstr = CString::new(out_path).map_err(|e| e.to_string())?;

        // Retry a few times if gnome-shell is still registering NameOwnerChanged on initial call
        let mut last_err = String::new();
        for attempt in 0..8 {
            unsafe {
                let params = (self.variant_new)(
                    b"(iiiibs)\0".as_ptr() as *const c_char,
                    x as c_int,
                    y as c_int,
                    w as c_int,
                    h as c_int,
                    0 as c_int, // flash = false
                    path_cstr.as_ptr(),
                );
                let mut err: *mut GError = std::ptr::null_mut();
                let reply = (self.call_sync)(
                    self.conn,
                    b"org.gnome.Shell.Screenshot\0".as_ptr() as *const c_char,
                    b"/org/gnome/Shell/Screenshot\0".as_ptr() as *const c_char,
                    b"org.gnome.Shell.Screenshot\0".as_ptr() as *const c_char,
                    b"ScreenshotArea\0".as_ptr() as *const c_char,
                    params,
                    std::ptr::null(),
                    0,
                    3000,
                    std::ptr::null_mut(),
                    &mut err,
                );

                if !reply.is_null() {
                    (self.variant_unref)(reply);
                    return Ok(());
                }

                if !err.is_null() {
                    let msg = if !(*err).message.is_null() {
                        CStr::from_ptr((*err).message).to_string_lossy().into_owned()
                    } else {
                        "GDBus ScreenshotArea error".to_string()
                    };
                    (self.error_free)(err);
                    last_err = msg;
                }
            }

            if attempt < 7 {
                std::thread::sleep(Duration::from_millis(15));
            }
        }

        Err(format!("Wayland ScreenshotArea failed: {}", last_err))
    }
}

fn capture_wayland_rect(x: i32, y: i32, width: u32, height: u32) -> Result<RgbaImage, Box<dyn Error>> {
    let mut guard = WAYLAND_CLIENT.lock().map_err(|_| "Wayland lock poisoned")?;
    if guard.is_none() {
        *guard = Some(WaylandDbusClient::init()?);
    }
    let client = guard.as_ref().unwrap();

    // Prefer RAM-backed /dev/shm to avoid disk I/O during scrolling capture
    let pid = std::process::id();
    let shm_dir = if std::path::Path::new("/dev/shm").is_dir() {
        "/dev/shm"
    } else {
        "/tmp"
    };
    let tmp_path = format!("{}/scroll_shot_wl_{}.png", shm_dir, pid);

    client.screenshot_area(x.max(0), y.max(0), width.max(1) as i32, height.max(1) as i32, &tmp_path)?;

    let img_bytes = std::fs::read(&tmp_path)?;
    let _ = std::fs::remove_file(&tmp_path);

    let decoded = image::load_from_memory(&img_bytes)?.into_rgba8();
    if decoded.width() == width && decoded.height() == height {
        Ok(decoded)
    } else {
        Ok(image::imageops::resize(
            &decoded,
            width,
            height,
            image::imageops::FilterType::CatmullRom,
        ))
    }
}
