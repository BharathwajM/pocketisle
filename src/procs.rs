//! The apps using the most CPU and memory (for the System tab), grouped by program so a
//! browser's many processes count as one, and closing one of them.

use std::collections::HashMap;
use std::time::Instant;

use windows::core::{BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, FILETIME, HWND, LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_TERMINATE,
};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindow, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, GW_OWNER, WM_CLOSE};

/// One program: its name, its processes, its share of the whole CPU (0..1) and memory in use.
#[derive(Clone, Default, Debug)]
pub struct Proc {
    pub name: String,
    pub exe: String,
    pub pids: Vec<u32>,
    pub cpu: f32,
    pub mem: u64,
}

/// Windows itself and the island: never listed, so never closed from here.
const HIDDEN: [&str; 26] = [
    "system", "[system process]", "registry", "memory compression", "secure system", "idle", "smss.exe", "csrss.exe", "wininit.exe",
    "services.exe", "lsass.exe", "winlogon.exe", "svchost.exe", "dwm.exe", "fontdrvhost.exe", "msmpeng.exe", "explorer.exe",
    "pocketisle.exe", "vmmem", "vmmemwsl", "sihost.exe", "ctfmon.exe", "taskhostw.exe", "audiodg.exe", "lsaiso.exe", "searchindexer.exe",
];

#[derive(Default)]
pub struct Sampler {
    /// CPU time (100 ns) of each process at the last sample, and when that was.
    last: HashMap<u32, u64>,
    at: Option<Instant>,
    /// Friendly names by exe ("Google Chrome" for chrome.exe).
    names: HashMap<String, String>,
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

impl Sampler {
    /// The top six by CPU and the top six by memory (together, without repeats).
    pub fn sample(&mut self) -> Vec<Proc> {
        let now = Instant::now();
        let dt = self.at.map(|a| now.duration_since(a).as_secs_f64()).unwrap_or(0.0);
        let ncpu = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
        let mut groups: HashMap<String, Proc> = HashMap::new();
        let mut seen = HashMap::new();
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return Vec::new() };
            let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut ok = Process32FirstW(snap, &mut e).is_ok();
            while ok {
                let pid = e.th32ProcessID;
                let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                let exe = String::from_utf16_lossy(&e.szExeFile[..n]).to_lowercase();
                if pid != 0 && pid != std::process::id() && !HIDDEN.contains(&exe.as_str()) {
                    if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                        let (mut c, mut x, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
                        let t = if GetProcessTimes(h, &mut c, &mut x, &mut k, &mut u).is_ok() { ft(k) + ft(u) } else { 0 };
                        let mut m = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
                        let mem = if K32GetProcessMemoryInfo(h, &mut m, m.cb).as_bool() { m.WorkingSetSize as u64 } else { 0 };
                        if !self.names.contains_key(&exe) {
                            let name = friendly_name(h).unwrap_or_else(|| plain_name(&exe));
                            self.names.insert(exe.clone(), name);
                        }
                        let _ = CloseHandle(h);
                        let cpu = match self.last.get(&pid) {
                            Some(&t0) if dt > 0.0 && dt < 10.0 => (t.saturating_sub(t0) as f64 / 1e7 / dt / ncpu).clamp(0.0, 1.0),
                            _ => 0.0,
                        };
                        seen.insert(pid, t);
                        let g = groups.entry(exe.clone()).or_insert_with(|| Proc { name: self.names[&exe].clone(), exe: exe.clone(), ..Default::default() });
                        g.pids.push(pid);
                        g.cpu += cpu as f32;
                        g.mem += mem;
                    }
                }
                ok = Process32NextW(snap, &mut e).is_ok();
            }
            let _ = CloseHandle(snap);
        }
        self.last = seen;
        self.at = Some(now);
        let mut v: Vec<Proc> = groups.into_values().collect();
        v.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.mem.cmp(&a.mem)));
        let mut out: Vec<Proc> = v.iter().take(6).cloned().collect();
        v.sort_by(|a, b| b.mem.cmp(&a.mem));
        for p in v.iter().take(6) {
            if !out.iter().any(|o| o.exe == p.exe) {
                out.push(p.clone());
            }
        }
        out
    }
}

/// A process's program name ("Google Chrome"), or None for Windows itself and the island.
/// Is any of these programs running? (Names like "wallpaper64.exe", any case.)
pub fn running(names: &[&str]) -> bool {
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return false };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        let mut found = false;
        while ok && !found {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            let exe = String::from_utf16_lossy(&e.szExeFile[..len]);
            found = names.iter().any(|n| n.eq_ignore_ascii_case(&exe));
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
        found
    }
}

pub fn name_of(pid: u32) -> Option<String> {
    if pid == 0 || pid == std::process::id() {
        return None;
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let path = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).ok().map(|_| String::from_utf16_lossy(&buf[..len as usize]));
        let exe = path.as_deref().and_then(|p| p.rsplit('\\').next()).unwrap_or("").to_lowercase();
        let name = if exe.is_empty() || HIDDEN.contains(&exe.as_str()) { None } else { Some(friendly_name(h).unwrap_or_else(|| plain_name(&exe))) };
        let _ = CloseHandle(h);
        name
    }
}

/// "chrome.exe" → "Chrome".
fn plain_name(exe: &str) -> String {
    let s = exe.trim_end_matches(".exe");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => exe.to_string(),
    }
}

/// The program's own description from its file ("Google Chrome", "Visual Studio Code").
unsafe fn friendly_name(h: windows::Win32::Foundation::HANDLE) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).ok()?;
    buf[len as usize] = 0;
    let path = PCWSTR(buf.as_ptr());
    let size = GetFileVersionInfoSizeW(path, None);
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    GetFileVersionInfoW(path, None, size, data.as_mut_ptr() as *mut _).ok()?;
    // The first language and code page it has.
    let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
    let mut n = 0u32;
    let tr = windows::core::w!("\\VarFileInfo\\Translation");
    if !VerQueryValueW(data.as_ptr() as *const _, tr, &mut p, &mut n).as_bool() || n < 4 {
        return None;
    }
    let (lang, cp) = (*(p as *const u16), *(p as *const u16).add(1));
    let key: Vec<u16> = format!("\\StringFileInfo\\{lang:04x}{cp:04x}\\FileDescription").encode_utf16().chain([0]).collect();
    if !VerQueryValueW(data.as_ptr() as *const _, PCWSTR(key.as_ptr()), &mut p, &mut n).as_bool() || n == 0 {
        return None;
    }
    let s = std::slice::from_raw_parts(p as *const u16, n as usize);
    let s = String::from_utf16_lossy(s).trim_matches(char::from(0)).trim().to_string();
    (!s.is_empty() && s.len() <= 40).then_some(s)
}

/// Close a program: ask its windows to close (so it can save), or, with no windows, end it.
/// Returns true if it was asked nicely.
pub fn close(pids: &[u32]) -> bool {
    struct Find<'a> {
        pids: &'a [u32],
        asked: bool,
    }
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let f = &mut *(lp.0 as *mut Find);
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if f.pids.contains(&pid) && IsWindowVisible(h).as_bool() && GetWindow(h, GW_OWNER).map_or(true, |o| o.is_invalid()) {
            let _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
            f.asked = true;
        }
        true.into()
    }
    let mut f = Find { pids, asked: false };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut f as *mut _ as isize));
    }
    if f.asked {
        return true;
    }
    for &pid in pids {
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                let _ = TerminateProcess(h, 1);
                let _ = CloseHandle(h);
            }
        }
    }
    false
}
