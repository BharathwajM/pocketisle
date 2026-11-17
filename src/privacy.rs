//! Which apps are using the microphone or the camera right now. Windows keeps this for its own
//! privacy indicator under CapabilityAccessManager\ConsentStore: an app is using the device
//! while its `LastUsedTimeStart` is set and its `LastUsedTimeStop` is 0. A thread waits for
//! that key to change and reports the lists through `events`.

use std::time::Duration;

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS};
use windows::Win32::System::Diagnostics::ToolHelp::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use crate::events::{self, Event};

const ROOT: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore");

pub fn spawn() {
    let _ = std::thread::Builder::new().name("privacy".into()).spawn(run);
}

fn run() {
    let mut root = HKEY::default();
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, ROOT, Some(0), KEY_READ | KEY_NOTIFY, &mut root) } != ERROR_SUCCESS {
        return;
    }
    let ev = unsafe { CreateEventW(None, false, false, None) }.ok();
    let mut last: Option<(Vec<String>, Vec<String>)> = None;
    loop {
        if let Some(e) = ev {
            unsafe {
                let _ = RegNotifyChangeKeyValue(root, true, REG_NOTIFY_CHANGE_LAST_SET | REG_NOTIFY_CHANGE_NAME, Some(e), true);
            }
        }
        let now = (in_use(root, w!("microphone")), in_use(root, w!("webcam")));
        if last.as_ref() != Some(&now) {
            events::send(Event::Privacy { mic: now.0.clone(), cam: now.1.clone() });
            last = Some(now);
        }
        match ev {
            // Also look again now and then: an app that quit without saying so drops off.
            Some(e) => unsafe {
                WaitForSingleObject(e, 10_000);
            },
            None => std::thread::sleep(Duration::from_secs(2)),
        }
        // Several values change at once; let them all land.
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn open(k: HKEY, sub: &str) -> Option<HKEY> {
    let mut out = HKEY::default();
    let ok = unsafe { RegOpenKeyExW(k, &HSTRING::from(sub), Some(0), KEY_READ, &mut out) } == ERROR_SUCCESS;
    ok.then_some(out)
}

fn close(k: HKEY) {
    unsafe {
        let _ = RegCloseKey(k);
    }
}

fn children(k: HKEY) -> Vec<String> {
    let mut v = Vec::new();
    let mut buf = [0u16; 512];
    for i in 0.. {
        let mut n = buf.len() as u32;
        let r = unsafe { RegEnumKeyExW(k, i, Some(PWSTR(buf.as_mut_ptr())), &mut n, None, None, None, None) };
        if r != ERROR_SUCCESS {
            break;
        }
        v.push(String::from_utf16_lossy(&buf[..n as usize]));
    }
    v
}

fn qword(k: HKEY, name: PCWSTR) -> u64 {
    let mut v = 0u64;
    let mut n = 8u32;
    let r = unsafe { RegQueryValueExW(k, name, None, None, Some(&mut v as *mut u64 as *mut u8), Some(&mut n)) };
    if r == ERROR_SUCCESS {
        v
    } else {
        0
    }
}

fn active(k: HKEY) -> bool {
    qword(k, w!("LastUsedTimeStart")) != 0 && qword(k, w!("LastUsedTimeStop")) == 0
}

/// The names of the apps using `cap` ("microphone" / "webcam").
fn in_use(root: HKEY, cap: PCWSTR) -> Vec<String> {
    let mut out = Vec::new();
    let Some(ck) = open(root, &unsafe { cap.to_string() }.unwrap_or_default()) else { return out };
    let mut desktop = Vec::new();
    for child in children(ck) {
        if child == "NonPackaged" {
            if let Some(np) = open(ck, &child) {
                for exe in children(np) {
                    if let Some(k) = open(np, &exe) {
                        if active(k) {
                            desktop.push(exe);
                        }
                        close(k);
                    }
                }
                close(np);
            }
        } else if let Some(k) = open(ck, &child) {
            if active(k) {
                out.push(package_name(&child));
            }
            close(k);
        }
    }
    close(ck);
    if !desktop.is_empty() {
        // A desktop app that crashed leaves its entry "in use"; only count running ones.
        let running = running_exes();
        for path in desktop {
            let file = path.rsplit('#').next().unwrap_or(&path).to_lowercase();
            if running.is_empty() || running.contains(&file) {
                out.push(exe_name(&file));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// "Microsoft.WindowsCamera_8wekyb3d8bbwe" -> "Windows Camera".
fn package_name(id: &str) -> String {
    let base = id.split('_').next().unwrap_or(id);
    let base = base.rsplit('.').next().unwrap_or(base);
    let mut s = String::new();
    let mut prev_lower = false;
    for ch in base.chars() {
        if ch.is_uppercase() && prev_lower {
            s.push(' ');
        }
        prev_lower = ch.is_lowercase();
        s.push(ch);
    }
    s
}

/// "firefox.exe" -> "Firefox".
fn exe_name(file: &str) -> String {
    let stem = file.strip_suffix(".exe").unwrap_or(file);
    let mut c = stem.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn running_exes() -> Vec<String> {
    let mut v = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return v };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            v.push(String::from_utf16_lossy(&e.szExeFile[..n]).to_lowercase());
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    v
}
