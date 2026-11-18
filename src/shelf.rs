//! The shelf: files dropped on the island wait there (across restarts) until they're dragged
//! out again or removed, and things copied lately can be copied again.
//!
//! Dropping uses OLE: each island window registers a drop target, which only posts messages
//! (`WM_APP_DRAG`) so nothing re-enters the app while a drag is going on. Dragging out uses
//! the shell's own data object and drag loop, so Explorer, mail and chat apps all take it.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use windows::core::{implement, Ref, HSTRING};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, POINTL, WPARAM};
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// Posted to an island window: wParam 1 = a drag with files came over it, 0 = it left,
/// 2 = files were dropped (take them with `take_dropped`).
pub const WM_APP_DRAG: u32 = WM_APP + 20;

static DROPPED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

pub fn take_dropped() -> Vec<PathBuf> {
    DROPPED.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
}

fn hdrop_format() -> FORMATETC {
    FORMATETC { cfFormat: 15, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
}

fn has_files(d: &IDataObject) -> bool {
    unsafe { d.QueryGetData(&hdrop_format()).is_ok() }
}

fn files(d: &IDataObject) -> Vec<PathBuf> {
    let mut out = Vec::new();
    unsafe {
        let Ok(mut m) = d.GetData(&hdrop_format()) else { return out };
        let drop = HDROP(m.u.hGlobal.0);
        let n = DragQueryFileW(drop, u32::MAX, None);
        for i in 0..n {
            let len = DragQueryFileW(drop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(drop, i, Some(&mut buf));
            out.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
        }
        ReleaseStgMedium(&mut m);
    }
    out
}

#[implement(IDropTarget)]
struct Target {
    hwnd: HWND,
    files: std::cell::Cell<bool>,
}

impl Target {
    fn post(&self, what: usize) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_APP_DRAG, WPARAM(what), LPARAM(0));
        }
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        let ok = data.as_ref().is_some_and(has_files);
        self.files.set(ok);
        unsafe {
            *effect = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
        }
        if ok {
            self.post(1);
        }
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        unsafe {
            *effect = if self.files.get() { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        if self.files.get() {
            self.post(0);
        }
        Ok(())
    }

    fn Drop(&self, data: Ref<IDataObject>, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        let got = data.as_ref().map(files).unwrap_or_default();
        unsafe {
            *effect = if got.is_empty() { DROPEFFECT_NONE } else { DROPEFFECT_COPY };
        }
        if !got.is_empty() {
            if let Ok(mut v) = DROPPED.lock() {
                v.extend(got);
            }
            self.post(2);
        } else {
            self.post(0);
        }
        Ok(())
    }
}

/// Let files be dropped on this island window. The UI thread must be single-threaded COM.
pub fn accept_drops(hwnd: HWND) {
    unsafe {
        let _ = OleInitialize(None);
        let t: IDropTarget = Target { hwnd, files: std::cell::Cell::new(false) }.into();
        if let Err(e) = RegisterDragDrop(hwnd, &t) {
            crate::util::log(&format!("drag and drop unavailable: {e}"));
        }
    }
}

pub fn stop_drops(hwnd: HWND) {
    unsafe {
        let _ = RevokeDragDrop(hwnd);
    }
}

/// Drag a file out of the island (blocks until it's dropped somewhere). Never call this
/// while the app is borrowed: the drag loop dispatches the island's own messages.
pub fn drag_out(hwnd: HWND, path: &Path) {
    unsafe {
        let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(path.as_os_str()), None) else { return };
        let Ok(data) = item.BindToHandler::<_, IDataObject>(None, &BHID_DataObject) else { return };
        let _ = SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK);
    }
}

/// Put text or a picture (CF_DIB bytes) back on the clipboard.
pub fn put_text(own: HWND, text: &str) -> bool {
    put(own, &[(13, utf16z(text))])
}

/// Put a secret (a one-time code) on the clipboard, marked the way password managers mark
/// theirs: Windows clipboard history and the cloud clipboard don't keep it, and clipboard
/// viewers skip it.
pub fn put_secret(own: HWND, text: &str) -> bool {
    let reg = |name: &str| unsafe { RegisterClipboardFormatW(&HSTRING::from(name)) };
    let no = 0u32.to_le_bytes().to_vec();
    put(
        own,
        &[
            (13, utf16z(text)),
            (reg("ExcludeClipboardContentFromMonitorProcessing"), no.clone()),
            (reg("CanIncludeInClipboardHistory"), no.clone()),
            (reg("CanUploadToCloudClipboard"), no),
        ],
    )
}

fn utf16z(text: &str) -> Vec<u8> {
    text.encode_utf16().chain(std::iter::once(0)).flat_map(|u| u.to_le_bytes()).collect()
}

pub fn put_dib(own: HWND, dib: &[u8]) -> bool {
    put(own, &[(8, dib.to_vec())])
}

fn put(own: HWND, items: &[(u32, Vec<u8>)]) -> bool {
    unsafe {
        if OpenClipboard(Some(own)).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let ok = items.iter().all(|(format, bytes)| {
            (|| {
                if *format == 0 {
                    return None;
                }
                let g: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).ok()?;
                let p = GlobalLock(g) as *mut u8;
                if p.is_null() {
                    return None;
                }
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
                let _ = GlobalUnlock(g);
                SetClipboardData(*format, Some(HANDLE(g.0))).ok()?;
                Some(())
            })()
            .is_some()
        });
        let _ = CloseClipboard();
        ok
    }
}

fn store() -> Option<PathBuf> {
    Some(crate::util::config_dir().join("shelf.json"))
}

pub fn load() -> Vec<PathBuf> {
    store()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Vec<PathBuf>>(&s).ok())
        .unwrap_or_default()
}

pub fn save(items: &[PathBuf]) {
    if let (Some(p), Ok(s)) = (store(), serde_json::to_string_pretty(items)) {
        let _ = std::fs::write(p, s);
    }
}

/// An icon for a file, from its type.
pub fn glyph(p: &Path) -> &'static str {
    if p.is_dir() {
        return "\u{f024b}";
    }
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "heic" | "svg" => "\u{f021f}",
        "pdf" => "\u{f0226}",
        "mp3" | "wav" | "flac" | "m4a" | "ogg" | "aac" => "\u{f0223}",
        "mp4" | "mkv" | "mov" | "avi" | "webm" => "\u{f022b}",
        "zip" | "7z" | "rar" | "tar" | "gz" => "\u{f05c4}",
        "doc" | "docx" | "txt" | "md" | "rtf" | "odt" => "\u{f0219}",
        "xls" | "xlsx" | "csv" => "\u{f021b}",
        "ppt" | "pptx" => "\u{f0227}",
        "exe" | "msi" | "lnk" => "\u{f08c6}",
        _ => "\u{f0214}",
    }
}
