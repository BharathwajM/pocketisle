//! `pocketisle.exe play <game>`: the Game tab's player, a small libretro frontend in its own
//! process (so a core that crashes can't take the island with it). melonDS DS plays DS games,
//! mGBA plays Game Boy Advance, Game Boy and Game Boy Color ones; the cores live in
//! `%LOCALAPPDATA%\PocketIsle\cores`. You bring your own games.
//!
//! The game shows in a window of its own (any size, keeps its shape). Keyboard: arrows for the
//! D-pad, X = A, Z = B, S = X, A = Y, Q / W = L / R, Enter = Start, Right Shift or Backspace =
//! Select; the mouse is the DS stylus on the bottom screen. F5 saves a state, F8 loads it, Tab
//! held fast-forwards. In-game saves are kept in `%LOCALAPPDATA%\PocketIsle\games\saves`.

use std::collections::VecDeque;
use std::ffi::{c_char, c_uint, c_void, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::{w, PCSTR, PCWSTR};
use windows::Win32::Foundation::{FreeLibrary, HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, FillRect, GetDC, GetMonitorInfoW, GetStockObject, InvalidateRect, MonitorFromWindow, ReleaseDC, SetStretchBltMode, StretchDIBits,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLACK_BRUSH, COLORONCOLOR, DIB_RGB_COLORS, HBRUSH, HDC, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT,
    SRCCOPY,
};
use windows::Win32::Media::Audio::{
    waveOutClose, waveOutOpen, waveOutPrepareHeader, waveOutReset, waveOutUnprepareHeader, waveOutWrite, CALLBACK_NULL, HWAVEOUT, WAVEFORMATEX, WAVEHDR,
    WAVE_FORMAT_PCM, WAVE_MAPPER,
};
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, ReleaseCapture, SetCapture, VIRTUAL_KEY, VK_BACK, VK_DOWN, VK_F5, VK_F8, VK_LEFT, VK_RETURN, VK_RIGHT, VK_RSHIFT, VK_TAB, VK_UP};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetForegroundWindow, LoadCursorW, PeekMessageW,
    RegisterClassExW, SetWindowTextW, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, IDC_ARROW, MSG, PM_REMOVE, SW_SHOW, WM_CLOSE,
    WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SIZE, WNDCLASSEXW, WS_EX_APPWINDOW, WS_OVERLAPPEDWINDOW,
};

use crate::util;

/// The class of the player's window (the island finds it by this to bring it to the front).
pub const CLASS: PCWSTR = w!("PocketIsleGame");

// ---------- Which core plays what ----------

/// "DS", "GBA", "GB" or "GBC" from a file name (None for anything else).
pub fn system_of(name: &str) -> Option<&'static str> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "nds" | "dsi" => Some("DS"),
        "gba" => Some("GBA"),
        "gb" => Some("GB"),
        "gbc" => Some("GBC"),
        _ => None,
    }
}

pub fn cores_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("PocketIsle").join("cores")
}

pub fn games_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("PocketIsle").join("games")
}

pub fn core_for(system: &str) -> PathBuf {
    cores_dir().join(if system == "DS" { "melondsds_libretro.dll" } else { "mgba_libretro.dll" })
}

fn tar() -> PathBuf {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("System32").join("tar.exe")
}

fn quiet(cmd: &mut std::process::Command) -> &mut std::process::Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000)
}

/// The game inside a .zip (its name, for the library), without unpacking it.
pub fn zip_game(zip: &Path) -> Option<String> {
    let out = quiet(std::process::Command::new(tar()).arg("-tf").arg(zip)).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().map(|l| l.trim().to_string()).find(|l| system_of(l).is_some())
}

/// A .zip unpacked once into the games folder; the game file inside it.
fn unpack(zip: &Path) -> Result<PathBuf, String> {
    let stem = zip.file_stem().and_then(|s| s.to_str()).unwrap_or("game");
    let dir = games_dir().join("unpacked").join(stem);
    let find = |dir: &Path| -> Option<PathBuf> {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).ok()?.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.file_name().and_then(|n| n.to_str()).and_then(system_of).is_some() {
                    return Some(p);
                }
            }
        }
        None
    };
    if let Some(p) = find(&dir) {
        return Ok(p);
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let st = quiet(std::process::Command::new(tar()).arg("-xf").arg(zip).arg("-C").arg(&dir)).status().map_err(|e| e.to_string())?;
    if !st.success() {
        return Err("couldn't unpack the .zip".into());
    }
    find(&dir).ok_or_else(|| "no DS, GBA or Game Boy game in the .zip".into())
}

// ---------- libretro ----------

type EnvFn = unsafe extern "C" fn(c_uint, *mut c_void) -> bool;
type VideoFn = unsafe extern "C" fn(*const c_void, c_uint, c_uint, usize);
type AudioFn = unsafe extern "C" fn(i16, i16);
type AudioBatchFn = unsafe extern "C" fn(*const i16, usize) -> usize;
type PollFn = unsafe extern "C" fn();
type InputFn = unsafe extern "C" fn(c_uint, c_uint, c_uint, c_uint) -> i16;

#[repr(C)]
struct SystemInfo {
    library_name: *const c_char,
    library_version: *const c_char,
    valid_extensions: *const c_char,
    need_fullpath: bool,
    block_extract: bool,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Geometry {
    base_width: c_uint,
    base_height: c_uint,
    max_width: c_uint,
    max_height: c_uint,
    aspect_ratio: f32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Timing {
    fps: f64,
    sample_rate: f64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct AvInfo {
    geometry: Geometry,
    timing: Timing,
}

#[repr(C)]
struct GameInfo {
    path: *const c_char,
    data: *const c_void,
    size: usize,
    meta: *const c_char,
}

#[repr(C)]
struct Variable {
    key: *const c_char,
    value: *const c_char,
}

struct Core {
    lib: HMODULE,
    init: unsafe extern "C" fn(),
    deinit: unsafe extern "C" fn(),
    run: unsafe extern "C" fn(),
    get_system_info: unsafe extern "C" fn(*mut SystemInfo),
    get_av_info: unsafe extern "C" fn(*mut AvInfo),
    load_game: unsafe extern "C" fn(*const GameInfo) -> bool,
    unload_game: unsafe extern "C" fn(),
    serialize_size: unsafe extern "C" fn() -> usize,
    serialize: unsafe extern "C" fn(*mut c_void, usize) -> bool,
    unserialize: unsafe extern "C" fn(*const c_void, usize) -> bool,
    get_memory_data: unsafe extern "C" fn(c_uint) -> *mut c_void,
    get_memory_size: unsafe extern "C" fn(c_uint) -> usize,
    set_controller_port_device: unsafe extern "C" fn(c_uint, c_uint),
    set_environment: unsafe extern "C" fn(EnvFn),
    set_video_refresh: unsafe extern "C" fn(VideoFn),
    set_audio_sample: unsafe extern "C" fn(AudioFn),
    set_audio_sample_batch: unsafe extern "C" fn(AudioBatchFn),
    set_input_poll: unsafe extern "C" fn(PollFn),
    set_input_state: unsafe extern "C" fn(InputFn),
}

impl Core {
    unsafe fn load(path: &Path) -> Result<Core, String> {
        let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
        let lib = LoadLibraryW(PCWSTR(wide.as_ptr())).map_err(|e| format!("couldn't load {}: {e}", path.display()))?;
        macro_rules! sym {
            ($n:literal) => {
                std::mem::transmute(GetProcAddress(lib, PCSTR(concat!($n, "\0").as_ptr())).ok_or(concat!("the core has no ", $n))?)
            };
        }
        Ok(Core {
            lib,
            init: sym!("retro_init"),
            deinit: sym!("retro_deinit"),
            run: sym!("retro_run"),
            get_system_info: sym!("retro_get_system_info"),
            get_av_info: sym!("retro_get_system_av_info"),
            load_game: sym!("retro_load_game"),
            unload_game: sym!("retro_unload_game"),
            serialize_size: sym!("retro_serialize_size"),
            serialize: sym!("retro_serialize"),
            unserialize: sym!("retro_unserialize"),
            get_memory_data: sym!("retro_get_memory_data"),
            get_memory_size: sym!("retro_get_memory_size"),
            set_controller_port_device: sym!("retro_set_controller_port_device"),
            set_environment: sym!("retro_set_environment"),
            set_video_refresh: sym!("retro_set_video_refresh"),
            set_audio_sample: sym!("retro_set_audio_sample"),
            set_audio_sample_batch: sym!("retro_set_audio_sample_batch"),
            set_input_poll: sym!("retro_set_input_poll"),
            set_input_state: sym!("retro_set_input_state"),
        })
    }
}

/// What the core's callbacks read and write. Never held while calling into the core.
struct Shared {
    pixfmt: u32,
    frame: Vec<u32>,
    fw: usize,
    fh: usize,
    audio: Vec<i16>,
    /// Buttons held, as libretro joypad bits.
    pad: u16,
    /// The mouse over the window (client px) and whether the button is down.
    mouse: (i32, i32, bool),
    client: (i32, i32),
    sys_dir: CString,
    save_dir: CString,
    user: CString,
    av: AvInfo,
    ff: bool,
}

static SH: Mutex<Option<Shared>> = Mutex::new(None);
static QUIT: AtomicBool = AtomicBool::new(false);
/// F5 / F8 pressed (1 = save state, 2 = load state).
static ACTION: AtomicU32 = AtomicU32::new(0);

fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> Option<R> {
    SH.lock().ok()?.as_mut().map(f)
}

const EXP: c_uint = 0x10000;

unsafe extern "C" fn env(cmd: c_uint, data: *mut c_void) -> bool {
    let set_str = |d: *mut c_void, s: &CString| *(d as *mut *const c_char) = s.as_ptr();
    match cmd {
        // GET_CAN_DUPE
        3 => {
            *(data as *mut bool) = true;
            true
        }
        // GET_SYSTEM_DIRECTORY, GET_SAVE_DIRECTORY, GET_CORE_ASSETS_DIRECTORY, GET_USERNAME
        9 | 30 => with(|s| set_str(data, &s.sys_dir)).is_some(),
        31 => with(|s| set_str(data, &s.save_dir)).is_some(),
        38 => with(|s| set_str(data, &s.user)).is_some(),
        // SET_PIXEL_FORMAT: 0RGB1555, XRGB8888 or RGB565.
        10 => {
            let f = *(data as *const u32);
            f <= 2 && with(|s| s.pixfmt = f).is_some()
        }
        // SET_INPUT_DESCRIPTORS, SET_VARIABLES, SET_SUBSYSTEM_INFO, SET_CONTROLLER_INFO,
        // SET_SERIALIZATION_QUIRKS, SET_MEMORY_MAPS, SET_SUPPORT_ACHIEVEMENTS: noted, unused.
        11 | 16 | 34 | 35 | 44 => true,
        x if x == 36 | EXP || x == 42 | EXP => true,
        // GET_VARIABLE: the core's defaults.
        15 => {
            let v = data as *mut Variable;
            if !v.is_null() {
                (*v).value = std::ptr::null();
            }
            false
        }
        // GET_VARIABLE_UPDATE
        17 => {
            *(data as *mut bool) = false;
            true
        }
        // GET_INPUT_DEVICE_CAPABILITIES: joypad and pointer.
        24 => {
            *(data as *mut u64) = (1 << 1) | (1 << 6);
            true
        }
        // SET_SYSTEM_AV_INFO, SET_GEOMETRY
        32 => with(|s| s.av = *(data as *const AvInfo)).is_some(),
        37 => with(|s| s.av.geometry = *(data as *const Geometry)).is_some(),
        // GET_LANGUAGE: English.
        39 => {
            *(data as *mut c_uint) = 0;
            true
        }
        // GET_CORE_OPTIONS_VERSION: 0, so options come as plain variables.
        52 => {
            *(data as *mut c_uint) = 0;
            true
        }
        // GET_INPUT_MAX_USERS
        61 => {
            *(data as *mut c_uint) = 1;
            true
        }
        // GET_JIT_CAPABLE
        74 => {
            *(data as *mut bool) = true;
            true
        }
        // GET_AUDIO_VIDEO_ENABLE, GET_FASTFORWARDING, GET_TARGET_REFRESH_RATE, GET_INPUT_BITMASKS
        x if x == 47 | EXP => {
            *(data as *mut i32) = 3;
            true
        }
        x if x == 49 | EXP => {
            *(data as *mut bool) = with(|s| s.ff).unwrap_or(false);
            true
        }
        x if x == 50 | EXP => {
            *(data as *mut f32) = 60.0;
            true
        }
        x if x == 51 | EXP => true,
        _ => false,
    }
}

unsafe extern "C" fn video(data: *const c_void, w: c_uint, h: c_uint, pitch: usize) {
    // NULL: the same frame again; -1: a GPU frame (never asked for).
    if data.is_null() || data as isize == -1 || w == 0 || h == 0 {
        return;
    }
    let (w, h) = (w as usize, h as usize);
    with(|s| {
        s.fw = w;
        s.fh = h;
        s.frame.resize(w * h, 0);
        let base = data as *const u8;
        for y in 0..h {
            let row = base.add(y * pitch);
            let out = &mut s.frame[y * w..(y + 1) * w];
            match s.pixfmt {
                1 => {
                    let src = std::slice::from_raw_parts(row as *const u32, w);
                    out.copy_from_slice(src);
                }
                2 => {
                    let src = std::slice::from_raw_parts(row as *const u16, w);
                    for (o, &p) in out.iter_mut().zip(src) {
                        let (r, g, b) = ((p >> 11) & 31, (p >> 5) & 63, p & 31);
                        *o = (((r << 3 | r >> 2) as u32) << 16) | (((g << 2 | g >> 4) as u32) << 8) | (b << 3 | b >> 2) as u32;
                    }
                }
                _ => {
                    let src = std::slice::from_raw_parts(row as *const u16, w);
                    for (o, &p) in out.iter_mut().zip(src) {
                        let (r, g, b) = ((p >> 10) & 31, (p >> 5) & 31, p & 31);
                        *o = (((r << 3 | r >> 2) as u32) << 16) | (((g << 3 | g >> 2) as u32) << 8) | (b << 3 | b >> 2) as u32;
                    }
                }
            }
        }
    });
}

unsafe extern "C" fn audio_sample(l: i16, r: i16) {
    with(|s| s.audio.extend([l, r]));
}

unsafe extern "C" fn audio_batch(data: *const i16, frames: usize) -> usize {
    if !data.is_null() {
        let src = std::slice::from_raw_parts(data, frames * 2);
        with(|s| s.audio.extend_from_slice(src));
    }
    frames
}

unsafe extern "C" fn input_poll() {}

/// Where the picture sits in the window: (x, y, w, h).
fn viewport(s: &Shared) -> (i32, i32, i32, i32) {
    let (cw, ch) = s.client;
    let aspect = if s.av.geometry.aspect_ratio > 0.0 {
        s.av.geometry.aspect_ratio as f64
    } else if s.fh > 0 {
        s.fw as f64 / s.fh as f64
    } else {
        1.0
    };
    let (mut w, mut h) = (cw as f64, cw as f64 / aspect);
    if h > ch as f64 {
        h = ch as f64;
        w = h * aspect;
    }
    let (w, h) = (w.round() as i32, h.round() as i32);
    ((cw - w) / 2, (ch - h) / 2, w.max(1), h.max(1))
}

unsafe extern "C" fn input_state(port: c_uint, device: c_uint, _index: c_uint, id: c_uint) -> i16 {
    if port != 0 {
        return 0;
    }
    with(|s| match device & 0xff {
        // Joypad: one button, or all of them as bits.
        1 if id == 256 => s.pad as i16,
        1 => ((s.pad >> id) & 1) as i16,
        // Pointer: the mouse over the picture is the stylus (-0x7fff..0x7fff across it).
        6 => {
            let (vx, vy, vw, vh) = viewport(s);
            let (mx, my, down) = s.mouse;
            let inside = mx >= vx && mx < vx + vw && my >= vy && my < vy + vh;
            let scale = |p: i32, o: i32, n: i32| (((p - o) as f64 / n as f64) * 65534.0 - 32767.0).round().clamp(-32767.0, 32767.0) as i16;
            match id {
                0 if inside => scale(mx, vx, vw),
                1 if inside => scale(my, vy, vh),
                0 | 1 => -32768,
                2 | 3 => (inside && down) as i16,
                _ => 0,
            }
        }
        _ => 0,
    })
    .unwrap_or(0)
}

// ---------- Sound ----------

struct Buf {
    hdr: WAVEHDR,
    data: Vec<i16>,
}

struct Speaker {
    h: HWAVEOUT,
    queue: VecDeque<Box<Buf>>,
}

impl Speaker {
    fn open(rate: u32) -> Option<Speaker> {
        let fmt = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM as u16,
            nChannels: 2,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * 4,
            nBlockAlign: 4,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        let mut h = HWAVEOUT::default();
        let r = unsafe { waveOutOpen(Some(&mut h), WAVE_MAPPER, &fmt, None, None, CALLBACK_NULL) };
        (r == 0).then_some(Speaker { h, queue: VecDeque::new() })
    }

    /// Buffers still waiting to be heard.
    fn queued(&mut self) -> usize {
        while self.queue.front().is_some_and(|b| b.hdr.dwFlags & 1 != 0) {
            if let Some(mut b) = self.queue.pop_front() {
                unsafe {
                    waveOutUnprepareHeader(self.h, &mut b.hdr, std::mem::size_of::<WAVEHDR>() as u32);
                }
            }
        }
        self.queue.len()
    }

    fn play(&mut self, samples: &[i16]) {
        // Never more than about 8 frames behind.
        if samples.is_empty() || self.queued() > 8 {
            return;
        }
        let mut b = Box::new(Buf { hdr: WAVEHDR::default(), data: samples.to_vec() });
        b.hdr.lpData = windows::core::PSTR(b.data.as_mut_ptr() as *mut u8);
        b.hdr.dwBufferLength = (b.data.len() * 2) as u32;
        unsafe {
            let size = std::mem::size_of::<WAVEHDR>() as u32;
            if waveOutPrepareHeader(self.h, &mut b.hdr, size) == 0 && waveOutWrite(self.h, &mut b.hdr, size) == 0 {
                self.queue.push_back(b);
            }
        }
    }
}

impl Drop for Speaker {
    fn drop(&mut self) {
        unsafe {
            waveOutReset(self.h);
            for b in self.queue.iter_mut() {
                waveOutUnprepareHeader(self.h, &mut b.hdr, std::mem::size_of::<WAVEHDR>() as u32);
            }
            waveOutClose(self.h);
        }
    }
}

// ---------- The window ----------

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let xy = || ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32);
    match msg {
        WM_CLOSE => {
            QUIT.store(true, Ordering::SeqCst);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_SIZE => {
            let (w, h) = ((lp.0 & 0xffff) as i32, ((lp.0 >> 16) & 0xffff) as i32);
            with(|s| s.client = (w, h));
            let _ = InvalidateRect(Some(hwnd), None, true);
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            present(hwnd, hdc, true);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            SetCapture(hwnd);
            let (x, y) = xy();
            with(|s| s.mouse = (x, y, true));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let _ = ReleaseCapture();
            let (x, y) = xy();
            with(|s| s.mouse = (x, y, false));
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = xy();
            with(|s| s.mouse.0 = x);
            with(|s| s.mouse.1 = y);
            LRESULT(0)
        }
        WM_KEYDOWN if (lp.0 >> 30) & 1 == 0 => {
            let vk = VIRTUAL_KEY(wp.0 as u16);
            if vk == VK_F5 {
                ACTION.store(1, Ordering::SeqCst);
            } else if vk == VK_F8 {
                ACTION.store(2, Ordering::SeqCst);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Draw the latest frame, letterboxed (black around it).
unsafe fn present(hwnd: HWND, hdc: HDC, clear: bool) {
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let guard = SH.lock();
    let Ok(guard) = guard else { return };
    let Some(s) = guard.as_ref() else { return };
    if clear || s.fw == 0 {
        FillRect(hdc, &rc, HBRUSH(GetStockObject(BLACK_BRUSH).0));
    }
    if s.fw == 0 || s.frame.len() < s.fw * s.fh {
        return;
    }
    let (vx, vy, vw, vh) = viewport(s);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: s.fw as i32,
            biHeight: -(s.fh as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    SetStretchBltMode(hdc, COLORONCOLOR);
    StretchDIBits(hdc, vx, vy, vw, vh, 0, 0, s.fw as i32, s.fh as i32, Some(s.frame.as_ptr() as *const c_void), &bmi, DIB_RGB_COLORS, SRCCOPY);
}

fn set_title(hwnd: HWND, s: &str) {
    let w: Vec<u16> = s.encode_utf16().chain([0]).collect();
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(w.as_ptr()));
    }
}

/// libretro joypad bits from the keyboard (only while the game's window is in front).
fn read_pad(hwnd: HWND) -> (u16, bool) {
    unsafe {
        if GetForegroundWindow() != hwnd {
            return (0, false);
        }
        let down = |k: u16| GetAsyncKeyState(k as i32) < 0;
        let keys: [(u16, u32); 12] = [
            (b'Z' as u16, 0),
            (b'A' as u16, 1),
            (VK_RSHIFT.0, 2),
            (VK_RETURN.0, 3),
            (VK_UP.0, 4),
            (VK_DOWN.0, 5),
            (VK_LEFT.0, 6),
            (VK_RIGHT.0, 7),
            (b'X' as u16, 8),
            (b'S' as u16, 9),
            (b'Q' as u16, 10),
            (b'W' as u16, 11),
        ];
        let mut pad = 0u16;
        for (k, bit) in keys {
            if down(k) {
                pad |= 1 << bit;
            }
        }
        if down(VK_BACK.0) {
            pad |= 1 << 2;
        }
        (pad, down(VK_TAB.0))
    }
}

// ---------- Playing ----------

/// `pocketisle.exe play <game> [title]`.
pub fn run(args: &[String]) {
    let Some(path) = args.first().map(PathBuf::from) else { return };
    // Sharp at any display scaling (the island does the same).
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let title = args.get(1).cloned().unwrap_or_else(|| path.file_stem().and_then(|s| s.to_str()).unwrap_or("Game").to_string());
    if let Err(e) = play(&path, &title) {
        util::log(&format!("game: {e}"));
        let msg: Vec<u16> = format!("Couldn't play {title}:\n{e}").encode_utf16().chain([0]).collect();
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::MessageBoxW(None, PCWSTR(msg.as_ptr()), w!("Pocket Isle"), windows::Win32::UI::WindowsAndMessaging::MB_ICONWARNING);
        }
    }
}

fn play(path: &Path, title: &str) -> Result<(), String> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    let rom = if name.to_ascii_lowercase().ends_with(".zip") { unpack(path)? } else { path.to_path_buf() };
    let rom_name = rom.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    let system = system_of(&rom_name).ok_or("not a DS, GBA or Game Boy game")?;
    let core_path = core_for(system);
    if !core_path.exists() {
        return Err(format!("the {system} emulator isn't installed ({})", core_path.display()));
    }
    let stem = rom.file_stem().and_then(|s| s.to_str()).unwrap_or("game").to_string();
    let gd = games_dir();
    let (sys_dir, save_dir, state_dir) = (gd.join("system"), gd.join("saves"), gd.join("states"));
    for d in [&sys_dir, &save_dir, &state_dir] {
        let _ = std::fs::create_dir_all(d);
    }
    let cs = |p: &Path| CString::new(p.to_string_lossy().as_bytes()).unwrap_or_default();
    *SH.lock().map_err(|_| "lock")? = Some(Shared {
        pixfmt: 0,
        frame: Vec::new(),
        fw: 0,
        fh: 0,
        audio: Vec::with_capacity(4096),
        pad: 0,
        mouse: (-1, -1, false),
        client: (1, 1),
        sys_dir: cs(&sys_dir),
        save_dir: cs(&save_dir),
        user: CString::new(std::env::var("USERNAME").unwrap_or_else(|_| "Player".into())).unwrap_or_default(),
        av: AvInfo::default(),
        ff: false,
    });

    unsafe {
        let core = Core::load(&core_path)?;
        (core.set_environment)(env);
        (core.set_video_refresh)(video);
        (core.set_audio_sample)(audio_sample);
        (core.set_audio_sample_batch)(audio_batch);
        (core.set_input_poll)(input_poll);
        (core.set_input_state)(input_state);
        (core.init)();

        let mut info = std::mem::zeroed::<SystemInfo>();
        (core.get_system_info)(&mut info);
        let cpath = cs(&rom);
        let data = if info.need_fullpath { Vec::new() } else { std::fs::read(&rom).map_err(|e| format!("couldn't read the game: {e}"))? };
        let game = GameInfo {
            path: cpath.as_ptr(),
            data: if data.is_empty() { std::ptr::null() } else { data.as_ptr() as *const c_void },
            size: data.len(),
            meta: std::ptr::null(),
        };
        if !(core.load_game)(&game) {
            (core.deinit)();
            return Err("the emulator couldn't start this game".into());
        }
        (core.set_controller_port_device)(0, 1);
        let mut av = AvInfo::default();
        (core.get_av_info)(&mut av);
        with(|s| s.av = av);
        let fps = if av.timing.fps > 1.0 { av.timing.fps } else { 60.0 };
        let rate = if av.timing.sample_rate > 1000.0 { av.timing.sample_rate.round() as u32 } else { 32768 };
        util::log(&format!("game: {title} ({system}) {}x{} at {fps:.2} fps, {rate} Hz", av.geometry.base_width, av.geometry.base_height));

        // In-game saves (the battery-backed memory), if the core shows it to us.
        let srm = save_dir.join(format!("{stem}.srm"));
        let sram = |core: &Core| -> Option<&'static mut [u8]> {
            let n = (core.get_memory_size)(0);
            let p = (core.get_memory_data)(0) as *mut u8;
            (n > 0 && !p.is_null()).then(|| std::slice::from_raw_parts_mut(p, n))
        };
        if let (Some(mem), Ok(saved)) = (sram(&core), std::fs::read(&srm)) {
            let n = mem.len().min(saved.len());
            mem[..n].copy_from_slice(&saved[..n]);
        }
        let mut last_saved: Vec<u8> = sram(&core).map(|m| m.to_vec()).unwrap_or_default();

        let hwnd = open_window(title, &av)?;
        let mut speaker = Speaker::open(rate);
        timeBeginPeriod(1);
        let frame = Duration::from_secs_f64(1.0 / fps);
        let mut next = Instant::now();
        let mut note: Option<(String, Instant)> = None;
        let mut save_check = Instant::now();
        let state = state_dir.join(format!("{stem}.state"));

        while !QUIT.load(Ordering::SeqCst) {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if QUIT.load(Ordering::SeqCst) {
                break;
            }
            let (pad, ff) = read_pad(hwnd);
            with(|s| {
                s.pad = pad;
                s.ff = ff;
            });
            match ACTION.swap(0, Ordering::SeqCst) {
                1 => {
                    let n = (core.serialize_size)();
                    let mut buf = vec![0u8; n];
                    let ok = n > 0 && (core.serialize)(buf.as_mut_ptr() as *mut c_void, n) && std::fs::write(&state, &buf).is_ok();
                    note = Some((if ok { "state saved (F8 to load it)" } else { "couldn't save a state" }.to_string(), Instant::now()));
                }
                2 => {
                    let ok = std::fs::read(&state).is_ok_and(|b| (core.unserialize)(b.as_ptr() as *const c_void, b.len()));
                    note = Some((if ok { "state loaded" } else { "no saved state yet (F5 saves one)" }.to_string(), Instant::now()));
                }
                _ => {}
            }
            // Fast-forward runs a few frames at a time, without sound.
            let runs = if ff { 4 } else { 1 };
            for _ in 0..runs {
                (core.run)();
            }
            let samples = with(|s| std::mem::take(&mut s.audio)).unwrap_or_default();
            if let Some(sp) = speaker.as_mut() {
                if !ff {
                    sp.play(&samples);
                }
            }
            with(|s| s.audio = Vec::with_capacity(samples.capacity()));
            let hdc = GetDC(Some(hwnd));
            present(hwnd, hdc, false);
            ReleaseDC(Some(hwnd), hdc);

            let shown = match &note {
                Some((t, at)) if at.elapsed() < Duration::from_secs(2) => format!("{title} — {t}"),
                Some(_) => {
                    note = None;
                    format!("{title} — Pocket Isle")
                }
                None => String::new(),
            };
            if !shown.is_empty() {
                set_title(hwnd, &shown);
            }
            // Keep in-game saves on disk (checked every 5 s).
            if save_check.elapsed() > Duration::from_secs(5) {
                save_check = Instant::now();
                if let Some(mem) = sram(&core) {
                    if mem != last_saved.as_slice() {
                        last_saved = mem.to_vec();
                        let _ = std::fs::write(&srm, &last_saved);
                    }
                }
            }
            // Keep time: the game's own frame rate, nudged to keep the sound flowing.
            next += frame;
            let now = Instant::now();
            let queued = speaker.as_mut().map(|s| s.queued()).unwrap_or(3);
            if ff || queued < 2 {
                next = next.max(now);
                continue;
            }
            if queued > 5 {
                next += Duration::from_millis(1);
            }
            if next > now {
                std::thread::sleep(next - now);
            } else if now - next > Duration::from_millis(200) {
                next = now;
            }
        }

        if let Some(mem) = sram(&core) {
            if mem != last_saved.as_slice() {
                let _ = std::fs::write(&srm, &*mem);
            }
        }
        drop(speaker);
        timeEndPeriod(1);
        let _ = DestroyWindow(hwnd);
        (core.unload_game)();
        (core.deinit)();
        let _ = FreeLibrary(core.lib);
        drop(data);
        drop(cpath);
    }
    util::log(&format!("game: {title} closed"));
    Ok(())
}

unsafe fn open_window(title: &str, av: &AvInfo) -> Result<HWND, String> {
    let hinst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: hinst.into(),
        lpszClassName: CLASS,
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        style: CS_HREDRAW | CS_VREDRAW,
        ..Default::default()
    };
    RegisterClassExW(&wc);
    let t: Vec<u16> = format!("{title} — Pocket Isle").encode_utf16().chain([0]).collect();
    let hwnd = CreateWindowExW(
        WS_EX_APPWINDOW,
        CLASS,
        PCWSTR(t.as_ptr()),
        WS_OVERLAPPEDWINDOW,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        640,
        480,
        None,
        None,
        Some(hinst.into()),
        None,
    )
    .map_err(|e| e.to_string())?;
    let dark: i32 = 1;
    let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const i32 as *const c_void, 4);
    // As big as fits nicely: about 70% of the screen's height, in the game's shape.
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut mi);
    let work = mi.rcWork;
    let (gw, gh) = (av.geometry.base_width.max(1) as f64, av.geometry.base_height.max(1) as f64);
    let aspect = if av.geometry.aspect_ratio > 0.0 { av.geometry.aspect_ratio as f64 } else { gw / gh };
    let wh = (work.bottom - work.top) as f64;
    let ch = (wh * 0.72).round();
    let cw = (ch * aspect).round();
    let mut r = RECT { left: 0, top: 0, right: cw as i32, bottom: ch as i32 };
    let _ = AdjustWindowRectEx(&mut r, WS_OVERLAPPEDWINDOW, false, WS_EX_APPWINDOW);
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let x = work.left + ((work.right - work.left) - w) / 2;
    let y = work.top + ((work.bottom - work.top) - h) / 2;
    let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(
        hwnd,
        None,
        x,
        y,
        w,
        h,
        windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER,
    );
    let _ = ShowWindow(hwnd, SW_SHOW);
    // In front of everything once (then an ordinary window), and focused for the keyboard.
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos as swp, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE};
    let _ = swp(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
    let _ = swp(hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
    let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd);
    Ok(hwnd)
}
