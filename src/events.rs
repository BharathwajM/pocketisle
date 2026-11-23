//! Background threads hand events to the UI thread through one channel, then wake it
//! with a posted message. The UI thread never polls.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

pub const WM_APP_EVENT: u32 = WM_APP + 1;
pub const WM_APP_VOLKEY: u32 = WM_APP + 2;
pub const WM_APP_APPBAR: u32 = WM_APP + 3;
/// Pocket Isle was started again while it's running: open the island.
pub const WM_APP_SHOW: u32 = WM_APP + 4;

static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
static TX: OnceLock<Sender<Event>> = OnceLock::new();

/// A decoded image, premultiplied BGRA, plus its average (vivid) colour.
#[derive(Clone)]
pub struct Art {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
    pub tint: [f32; 3],
}

#[derive(Clone, PartialEq)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub app: String,
}

pub enum Event {
    /// The accent picked from a wallpaper (None if it couldn't be read).
    WallpaperAccent(crate::wallpaper::Key, Option<[f32; 3]>),
    /// Wallpaper Engine's wallpapers now (a title per screen); `announce` when the wallpaper
    /// of the day just changed them.
    Wallpaper { titles: Vec<String>, announce: bool },
    Volume { level: f32, muted: bool },
    AudioDeviceChanged,
    Brightness(u8),
    Media(Option<MediaInfo>),
    MediaArt(Option<Art>),
    Playback(bool),
    Timeline { pos: f64, end: f64 },
    Notification { app: String, title: String, body: String, icon: Option<Art> },
    Usage(crate::usage::Usage),
    Sys(crate::sysinfo::Sys),
    Weather(crate::weather::Weather),
    /// Current panel brightness (no pop-up), e.g. for the Controls slider.
    BrightnessLevel(u8),
    /// A new picture in the Screenshots folder.
    Screenshot { path: std::path::PathBuf, art: Option<Art> },
    /// Headphones / a headset became the default output.
    OutputDevice { name: String, battery: Option<u8> },
    /// The apps using the microphone / the camera right now.
    Privacy { mic: Vec<String>, cam: Vec<String> },
    Net(crate::net::Net),
    /// The Storage page's folder scan has news.
    ScanProgress,
    /// A game picked in the Game tab's "Open game…" dialog.
    GamePicked(std::path::PathBuf),
    /// A file finished downloading into Downloads (its size in bytes).
    Download { path: std::path::PathBuf, size: u64 },
    /// Your pull requests' checks or reviews changed.
    GitHub(Vec<crate::github::Note>),
    /// The CPU or GPU has been running hot for a while.
    Heat(crate::heat::Heat),
    /// The microphone was muted or unmuted, or a restart for an update started or stopped
    /// waiting (for the panels beside the island).
    Glance,
}

pub fn init(hwnd: HWND) -> Receiver<Event> {
    HWND_RAW.store(hwnd.0 as isize, Ordering::SeqCst);
    let (tx, rx) = channel();
    let _ = TX.set(tx);
    rx
}

pub fn hwnd() -> HWND {
    HWND(HWND_RAW.load(Ordering::SeqCst) as *mut _)
}

pub fn send(ev: Event) {
    // The demo shows its own volume and brightness pop-ups, not the PC's.
    if crate::util::demo() && matches!(ev, Event::Volume { .. } | Event::Brightness(_)) {
        return;
    }
    if let Some(tx) = TX.get() {
        if tx.send(ev).is_ok() {
            unsafe {
                let _ = PostMessageW(Some(hwnd()), WM_APP_EVENT, WPARAM(0), LPARAM(0));
            }
        }
    }
}
