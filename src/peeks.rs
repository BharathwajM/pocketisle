//! Little moments the island peeks out for: something copied, a screenshot taken, the charger
//! plugged in, headphones connected. Reading the clipboard happens on the UI thread (it's
//! told by WM_CLIPBOARDUPDATE); the screenshots folder and the headphone battery are looked
//! up on worker threads, which report back through `events`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use windows::core::{w, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::*;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Devices::Properties::{DEVPKEY_Device_ContainerId, DEVPROPTYPE};
use windows::Win32::Foundation::{CloseHandle, DEVPROPKEY, HGLOBAL, HWND};
use windows::Win32::Media::Audio::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::UI::Shell::{SHGetKnownFolderPath, FOLDERID_Downloads, FOLDERID_Screenshots, KF_FLAG_DEFAULT};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

use crate::events::{self, Art, Event};

const CF_UNICODETEXT: u32 = 13;
const CF_DIB: u32 = 8;

/// What was just copied.
pub enum Clip {
    Text(String),
    /// A thumbnail, the picture's real size, and the clipboard's CF_DIB bytes.
    Image(Option<Art>, u32, u32, Vec<u8>),
}

/// Read the clipboard after it changed. None when it's empty, ours, or marked private (password
/// managers mark what they copy so clipboard history and viewers like this one skip it).
pub fn clipboard(own: HWND) -> Option<Clip> {
    unsafe {
        if let Ok(owner) = GetClipboardOwner() {
            let (mut a, mut b) = (0u32, 0u32);
            GetWindowThreadProcessId(owner, Some(&mut a));
            GetWindowThreadProcessId(own, Some(&mut b));
            if a != 0 && a == b {
                return None;
            }
        }
        for private in [w!("ExcludeClipboardContentFromMonitorProcessing"), w!("Clipboard Viewer Ignore")] {
            if IsClipboardFormatAvailable(RegisterClipboardFormatW(private)).is_ok() {
                return None;
            }
        }
        let history = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        let image = IsClipboardFormatAvailable(CF_DIB).is_ok();
        let text = IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok();
        if !image && !text {
            return None;
        }
        // Another app may hold the clipboard for a moment.
        let mut opened = false;
        for _ in 0..4 {
            if OpenClipboard(Some(own)).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        if !opened {
            return None;
        }
        let read = |format: u32| -> Option<Vec<u8>> {
            let h = GetClipboardData(format).ok()?;
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u8;
            if p.is_null() {
                return None;
            }
            let v = std::slice::from_raw_parts(p, GlobalSize(g)).to_vec();
            let _ = GlobalUnlock(g);
            Some(v)
        };
        let excluded = IsClipboardFormatAvailable(history).is_ok()
            && read(history).is_some_and(|v| v.len() >= 4 && u32::from_le_bytes([v[0], v[1], v[2], v[3]]) == 0);
        let out = if excluded {
            None
        } else if image {
            read(CF_DIB).map(|dib| {
                let (w, h) = dib_size(&dib);
                let art = dib_to_art(&dib);
                Clip::Image(art, w, h, dib)
            })
        } else {
            read(CF_UNICODETEXT).map(|v| {
                let units: Vec<u16> = v.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|c| *c != 0).collect();
                Clip::Text(String::from_utf16_lossy(&units))
            })
        };
        let _ = CloseClipboard();
        out
    }
}

/// The DIB's width and height (height is negative for top-down bitmaps).
fn dib_size(dib: &[u8]) -> (u32, u32) {
    if dib.len() < 12 {
        return (0, 0);
    }
    let i = |o: usize| i32::from_le_bytes([dib[o], dib[o + 1], dib[o + 2], dib[o + 3]]);
    (i(4).unsigned_abs(), i(8).unsigned_abs())
}

/// A clipboard DIB as a small thumbnail: give it a BMP file header and let WIC decode it.
fn dib_to_art(dib: &[u8]) -> Option<Art> {
    if dib.len() < 40 {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes([dib[o], dib[o + 1], dib[o + 2], dib[o + 3]]);
    let header = u32_at(0) as usize;
    let bits = u16::from_le_bytes([dib[14], dib[15]]) as u32;
    let compression = u32_at(16);
    let used = u32_at(32);
    let masks = if header == 40 && compression == 3 { 12 } else { 0 };
    let palette = if bits <= 8 { if used > 0 { used } else { 1 << bits } } else { used } as usize * 4;
    let offset = 14 + header + masks + palette;
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((14 + dib.len()) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0, 0, 0, 0]);
    bmp.extend_from_slice(&(offset as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    crate::imaging::decode(&bmp, 320).ok()
}

/// One line of copied text for the peek.
pub fn preview(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let more = text.trim().lines().count() > 1;
    let clipped = crate::util::clip(line, 70);
    if more && clipped == line { format!("{clipped} …") } else { clipped }
}

/// (on charger, battery percent). None on a desktop without a battery.
pub fn power() -> Option<(bool, u8)> {
    let mut s = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut s).ok()? };
    // 128 = no battery; 255 = unknown.
    if s.BatteryFlag & 128 != 0 || s.BatteryLifePercent > 100 {
        return None;
    }
    Some((s.ACLineStatus == 1, s.BatteryLifePercent))
}

/// Watch the Screenshots folder (Win+PrtScn and the Snipping Tool save there) and report each
/// new picture with a thumbnail.
pub fn watch_screenshots() {
    watch_folder(&FOLDERID_Screenshots, true, |dir, name| {
        if is_picture(name) {
            let path = dir.join(name);
            std::thread::spawn(move || {
                let art = thumbnail(&path);
                events::send(Event::Screenshot { path, art });
            });
        }
    });
}

/// Watch the Downloads folder: each file that lands there (once it's finished downloading)
/// is reported with its size.
pub fn watch_downloads() {
    watch_folder(&FOLDERID_Downloads, false, |dir, name| {
        if !is_download(name) {
            return;
        }
        let path = dir.join(name);
        {
            let Ok(mut busy) = WAITING.lock() else { return };
            if busy.contains(&path) {
                return;
            }
            busy.push(path.clone());
        }
        std::thread::spawn(move || {
            let done = wait_finished(&path);
            if let Ok(mut busy) = WAITING.lock() {
                busy.retain(|p| p != &path);
            }
            if let Some(size) = done {
                events::send(Event::Download { path, size });
            }
        });
    });
}

static WAITING: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Not a browser's half-done file, a lock file or Explorer's own.
pub fn is_download(name: &str) -> bool {
    let n = name.to_lowercase();
    let temp = [".crdownload", ".part", ".partial", ".tmp", ".download", ".opdownload", ".!ut", ".aria2", ".dtapart"];
    !(n.is_empty() || n.starts_with("~$") || n.starts_with('.') || n == "desktop.ini" || n.starts_with("unconfirmed ") || temp.iter().any(|t| n.ends_with(t)))
}

/// Wait until the file is whole: some bytes, the same size twice in a row, and no browser
/// part-file beside it (Firefox makes the real name first and fills `.part`). None if it
/// goes away, is a folder, or never settles (half an hour).
fn wait_finished(path: &Path) -> Option<u64> {
    let mut last = u64::MAX;
    for _ in 0..1800 {
        std::thread::sleep(Duration::from_millis(1000));
        let meta = std::fs::metadata(path).ok()?;
        if meta.is_dir() {
            return None;
        }
        let part = PathBuf::from(format!("{}.part", path.display()));
        let size = meta.len();
        if size > 0 && size == last && !part.exists() && std::fs::OpenOptions::new().read(true).open(path).is_ok() {
            return Some(size);
        }
        last = size;
    }
    None
}

/// Watch a known folder's file names (not its subfolders) on a thread of its own, calling
/// `found` with each file that's added or renamed into it.
fn watch_folder(folder: &'static windows::core::GUID, create: bool, found: impl Fn(&Path, &str) + Send + 'static) {
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(p) = SHGetKnownFolderPath(folder, KF_FLAG_DEFAULT, None) else { return };
        let dir = PathBuf::from(p.to_string().unwrap_or_default());
        CoTaskMemFree(Some(p.0 as _));
        if create {
            let _ = std::fs::create_dir_all(&dir);
        }
        let wide: Vec<u16> = dir.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
        let Ok(h) = CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        ) else {
            return;
        };
        let mut buf = vec![0u32; 4096];
        loop {
            let mut got = 0u32;
            if ReadDirectoryChangesW(h, buf.as_mut_ptr() as _, (buf.len() * 4) as u32, false, FILE_NOTIFY_CHANGE_FILE_NAME, Some(&mut got), None, None).is_err() {
                break;
            }
            let bytes = std::slice::from_raw_parts(buf.as_ptr() as *const u8, got as usize);
            let mut off = 0usize;
            while off + 12 <= bytes.len() {
                let rd = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
                let (next, action, len) = (rd(off) as usize, rd(off + 4), rd(off + 8) as usize);
                let name: Vec<u16> = bytes[off + 12..(off + 12 + len).min(bytes.len())].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                let name = String::from_utf16_lossy(&name);
                if action == FILE_ACTION_ADDED.0 || action == FILE_ACTION_RENAMED_NEW_NAME.0 {
                    found(&dir, &name);
                }
                if next == 0 {
                    break;
                }
                off += next;
            }
        }
        let _ = CloseHandle(h);
    });
}

fn is_picture(name: &str) -> bool {
    let n = name.to_lowercase();
    [".png", ".jpg", ".jpeg"].iter().any(|e| n.ends_with(e))
}

/// The saved picture as a thumbnail, waiting a moment if it's still being written.
fn thumbnail(path: &Path) -> Option<Art> {
    for _ in 0..12 {
        std::thread::sleep(Duration::from_millis(150));
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(a) = crate::imaging::decode(&bytes, 320) {
                return Some(a);
            }
        }
    }
    None
}

/// After the default output changes: report it if it's headphones or a headset, with the
/// Bluetooth battery level when Windows knows it (read a couple of seconds later, once the
/// headset has reported it).
pub fn check_output() {
    std::thread::spawn(|| unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        std::thread::sleep(Duration::from_millis(1500));
        let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return };
        let Ok(dev) = en.GetDefaultAudioEndpoint(eRender, eConsole) else { return };
        let Ok(store) = dev.OpenPropertyStore(STGM_READ) else { return };
        let name = store.GetValue(&PKEY_Device_FriendlyName).map(|v| v.to_string()).unwrap_or_default();
        let form = store.GetValue(&PKEY_AudioEndpoint_FormFactor).ok().and_then(|v| u32::try_from(&v).ok()).unwrap_or(0);
        // EndpointFormFactor: Headphones = 3, Headset = 5.
        let headphones = form == 3 || form == 5;
        let container = store
            .GetValue(&windows::Win32::Devices::FunctionDiscovery::PKEY_Device_ContainerId)
            .map(|v| normalise_guid(&v.to_string()))
            .unwrap_or_default();
        let battery = if container.is_empty() { None } else { battery_for(&container) };
        if headphones || battery.is_some() {
            events::send(Event::OutputDevice { name: short_name(&name), battery });
        }
    });
}

/// "Headphones (WH-1000XM4)" → "WH-1000XM4".
fn short_name(name: &str) -> String {
    match (name.find('('), name.rfind(')')) {
        (Some(a), Some(b)) if b > a + 1 => name[a + 1..b].trim().to_string(),
        _ => name.trim().to_string(),
    }
}

pub(crate) fn normalise_guid(s: &str) -> String {
    s.trim().trim_matches(|c| c == '{' || c == '}').to_uppercase()
}

/// The battery level Windows shows for a Bluetooth device, found by its container id.
unsafe fn battery_for(container: &str) -> Option<u8> {
    // DEVPKEY_Bluetooth_Battery (what Settings > Bluetooth & devices shows).
    const BATTERY: DEVPROPKEY = DEVPROPKEY { fmtid: windows::core::GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5), pid: 2 };
    let set = SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_ALLCLASSES | DIGCF_PRESENT).ok()?;
    let mut found = None;
    let mut i = 0;
    loop {
        let mut d = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
        if SetupDiEnumDeviceInfo(set, i, &mut d).is_err() {
            break;
        }
        i += 1;
        let mut ty = DEVPROPTYPE::default();
        let mut g = [0u8; 16];
        if SetupDiGetDevicePropertyW(set, &d, &DEVPKEY_Device_ContainerId, &mut ty, Some(&mut g), None, 0).is_err() {
            continue;
        }
        if normalise_guid(&format!("{:?}", guid_from(&g))) != container {
            continue;
        }
        let mut b = [0u8; 1];
        if SetupDiGetDevicePropertyW(set, &d, &BATTERY, &mut ty, Some(&mut b), None, 0).is_ok() && b[0] <= 100 {
            found = Some(b[0]);
            break;
        }
    }
    let _ = SetupDiDestroyDeviceInfoList(set);
    found
}

/// A GUID from its in-memory bytes (first three fields little-endian).
pub(crate) fn guid_from(g: &[u8; 16]) -> windows::core::GUID {
    let d1 = u32::from_le_bytes([g[0], g[1], g[2], g[3]]);
    let d2 = u16::from_le_bytes([g[4], g[5]]);
    let d3 = u16::from_le_bytes([g[6], g[7]]);
    windows::core::GUID::from_values(d1, d2, d3, [g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_read_cleanly() {
        assert_eq!(preview("  hello world  "), "hello world");
        assert_eq!(preview("\nfirst\nsecond"), "first …");
        assert_eq!(short_name("Headphones (WH-1000XM4)"), "WH-1000XM4");
        assert_eq!(short_name("Speakers"), "Speakers");
    }

    #[test]
    fn downloads_skip_half_done_files() {
        assert!(is_download("report.pdf"));
        assert!(is_download("setup.exe"));
        assert!(!is_download("report.pdf.crdownload"));
        assert!(!is_download("film.mkv.part"));
        assert!(!is_download("Unconfirmed 123456.crdownload"));
        assert!(!is_download("desktop.ini"));
        assert!(!is_download("~$letter.docx"));
    }

    #[test]
    fn guid_round_trip() {
        let g = windows::core::GUID::from_u128(0x8c7ed206_3f8a_4827_b3ab_ae9e1faefc6c);
        let raw: [u8; 16] = unsafe { std::mem::transmute(g) };
        assert_eq!(guid_from(&raw), g);
    }
}
