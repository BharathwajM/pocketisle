//! Quick actions for the Controls, Power and Home tabs: Do Not Disturb, dark mode, touch
//! keyboard, screenshot, lock / sleep / restart..., and launching shortcuts.
//! Do Not Disturb and the touch keyboard use the same shell COM objects as YASB's control centre.

use std::ffi::c_void;
use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::core::{w, GUID, HRESULT, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Security::Authentication::Identity::{GetUserNameExW, NameDisplay};
use windows::Win32::System::Power::SetSuspendState;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Shutdown::{ExitWindowsEx, LockWorkStation, EWX_LOGOFF, SHTDN_REASON_NONE};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{GetDesktopWindow, SendNotifyMessageW, HWND_BROADCAST, SW_SHOWNORMAL, WM_SETTINGCHANGE};

use crate::config::Shortcut;
use crate::util;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[link(name = "ole32")]
extern "system" {
    fn CoCreateInstance(clsid: *const GUID, outer: *mut c_void, ctx: u32, iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    fn CoTaskMemFree(p: *const c_void);
}

/// Call method `index` of a raw COM object's vtable.
unsafe fn vfn<T: Copy>(obj: *mut c_void, index: usize) -> T {
    let vtbl = *(obj as *const *const usize);
    std::mem::transmute_copy(&*vtbl.add(index))
}

unsafe fn release(obj: *mut c_void) {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = vfn(obj, 2);
    f(obj);
}

unsafe fn create(clsid: GUID, iid: GUID, ctx: u32) -> Option<*mut c_void> {
    let mut p = std::ptr::null_mut();
    (CoCreateInstance(&clsid, std::ptr::null_mut(), ctx, &iid, &mut p).is_ok() && !p.is_null()).then_some(p)
}

// ---------- Do Not Disturb (Focus "quiet hours" profile) ----------

const QUIET_CLSID: GUID = GUID::from_u128(0xF53321FA_34F8_4B7F_B9A3_361877CB94CF);
const QUIET_IID: GUID = GUID::from_u128(0x6BFF4732_81EC_4FFB_AE67_B6C1BC29631F);
const PROFILE_OFF: &str = "Microsoft.QuietHoursProfile.Unrestricted";
const PROFILE_PRIORITY: &str = "Microsoft.QuietHoursProfile.PriorityOnly";

pub fn dnd() -> Option<bool> {
    unsafe {
        let obj = create(QUIET_CLSID, QUIET_IID, 4)?;
        let get: unsafe extern "system" fn(*mut c_void, *mut PWSTR) -> HRESULT = vfn(obj, 3);
        let mut s = PWSTR::null();
        let on = if get(obj, &mut s).is_ok() && !s.is_null() {
            let v = s.to_string().unwrap_or_default();
            CoTaskMemFree(s.0 as *const c_void);
            Some(v != PROFILE_OFF)
        } else {
            None
        };
        release(obj);
        on
    }
}

pub fn set_dnd(on: bool) {
    unsafe {
        let Some(obj) = create(QUIET_CLSID, QUIET_IID, 4) else { return };
        let put: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT = vfn(obj, 4);
        let p = HSTRING::from(if on { PROFILE_PRIORITY } else { PROFILE_OFF });
        let _ = put(obj, PCWSTR(p.as_ptr()));
        release(obj);
    }
}

// ---------- Dark mode ----------

const PERSONALIZE: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");

pub fn dark_mode() -> bool {
    let mut v = 1u32;
    let mut n = 4u32;
    unsafe {
        let _ = RegGetValueW(HKEY_CURRENT_USER, PERSONALIZE, w!("AppsUseLightTheme"), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as *mut c_void), Some(&mut n));
    }
    v == 0
}

pub fn set_dark_mode(dark: bool) {
    let v: u32 = if dark { 0 } else { 1 };
    unsafe {
        for name in [w!("AppsUseLightTheme"), w!("SystemUsesLightTheme")] {
            let _ = RegSetKeyValueW(HKEY_CURRENT_USER, PERSONALIZE, name, REG_DWORD.0, Some(&v as *const u32 as *const c_void), 4);
        }
        let _ = SendNotifyMessageW(HWND_BROADCAST, WM_SETTINGCHANGE, WPARAM(0), LPARAM(w!("ImmersiveColorSet").as_ptr() as isize));
    }
}

// ---------- Touch keyboard ----------

const TIP_CLSID: GUID = GUID::from_u128(0x4CE576FA_83DC_4F88_951C_9D0782B4E376);
const TIP_IID: GUID = GUID::from_u128(0x37C994E7_432B_4834_A2F7_DCE1F13B834B);

fn tip_toggle() -> bool {
    unsafe {
        let Some(obj) = create(TIP_CLSID, TIP_IID, 2 | 4) else { return false };
        let toggle: unsafe extern "system" fn(*mut c_void, HWND) -> HRESULT = vfn(obj, 3);
        let ok = toggle(obj, GetDesktopWindow()).is_ok();
        release(obj);
        ok
    }
}

pub fn toggle_keyboard() {
    if tip_toggle() {
        return;
    }
    // TabTip isn't running yet: start it, then toggle once it's up.
    std::thread::spawn(|| {
        let common = std::env::var("CommonProgramFiles").unwrap_or_else(|_| r"C:\Program Files\Common Files".into());
        open(&format!(r"{common}\microsoft shared\ink\TabTip.exe"), None);
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(150));
            if tip_toggle() {
                break;
            }
        }
    });
}

pub fn screenshot() {
    open("ms-screenclip:", None);
}

// ---------- Power ----------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PowerAct {
    Lock,
    SignOut,
    Sleep,
    Hibernate,
    Restart,
    ShutDown,
}

impl PowerAct {
    pub const ALL: [PowerAct; 6] = [PowerAct::Lock, PowerAct::SignOut, PowerAct::Sleep, PowerAct::Hibernate, PowerAct::Restart, PowerAct::ShutDown];

    pub fn label(self) -> &'static str {
        match self {
            PowerAct::Lock => "Lock",
            PowerAct::SignOut => "Sign out",
            PowerAct::Sleep => "Sleep",
            PowerAct::Hibernate => "Hibernate",
            PowerAct::Restart => "Restart",
            PowerAct::ShutDown => "Shut down",
        }
    }

    /// Same glyphs as YASB's power menu.
    pub fn glyph(self) -> &'static str {
        match self {
            PowerAct::Lock => "\u{f0341}",
            PowerAct::SignOut => "\u{f0343}",
            PowerAct::Sleep => "\u{f0904}",
            PowerAct::Hibernate => "\u{f0901}",
            PowerAct::Restart => "\u{f0453}",
            PowerAct::ShutDown => "\u{f0906}",
        }
    }

    /// Ends the session or closes apps, so it needs a second click.
    pub fn needs_confirm(self) -> bool {
        !matches!(self, PowerAct::Lock | PowerAct::Sleep)
    }

    pub fn run(self) {
        util::log(&format!("power: {}", self.label()));
        unsafe {
            match self {
                PowerAct::Lock => {
                    let _ = LockWorkStation();
                }
                PowerAct::SignOut => {
                    let _ = ExitWindowsEx(EWX_LOGOFF, SHTDN_REASON_NONE);
                }
                PowerAct::Sleep => {
                    let _ = SetSuspendState(false, false, false);
                }
                PowerAct::Hibernate => {
                    let _ = SetSuspendState(true, false, false);
                }
                PowerAct::Restart => {
                    let _ = Command::new("shutdown.exe").args(["/r", "/t", "0"]).creation_flags(CREATE_NO_WINDOW).spawn();
                }
                PowerAct::ShutDown => {
                    let _ = Command::new("shutdown.exe").args(["/s", "/hybrid", "/t", "0"]).creation_flags(CREATE_NO_WINDOW).spawn();
                }
            }
        }
    }
}

pub fn user_name() -> String {
    if crate::util::demo() {
        return "Alex Rivera".into();
    }
    unsafe {
        let mut buf = [0u16; 256];
        let mut n = buf.len() as u32;
        if GetUserNameExW(NameDisplay, Some(PWSTR(buf.as_mut_ptr())), &mut n) && n > 0 {
            return String::from_utf16_lossy(&buf[..n as usize]);
        }
    }
    std::env::var("USERNAME").unwrap_or_default()
}

pub fn uptime() -> String {
    let s = unsafe { GetTickCount64() } / 1000;
    let (d, h, m) = (s / 86400, (s % 86400) / 3600, (s % 3600) / 60);
    if d > 0 {
        format!("Up {d}d {h}h")
    } else if h > 0 {
        format!("Up {h}h {m}m")
    } else {
        format!("Up {m}m")
    }
}

// ---------- Shortcuts ----------

fn expand(s: &str) -> String {
    let mut out = if let Some(rest) = s.strip_prefix('~') {
        format!("{}{rest}", util::home().display())
    } else {
        s.to_string()
    };
    // %VAR% expansion.
    while let Some(a) = out.find('%') {
        let Some(b) = out[a + 1..].find('%') else { break };
        let name = &out[a + 1..a + 1 + b];
        let val = std::env::var(name).unwrap_or_default();
        out = format!("{}{}{}", &out[..a], val, &out[a + 2 + b..]);
    }
    out
}

fn open(target: &str, args: Option<&str>) {
    unsafe {
        let t = HSTRING::from(target);
        let a = args.map(HSTRING::from);
        ShellExecuteW(None, w!("open"), &t, a.as_ref().map_or(PCWSTR::null(), |a| PCWSTR(a.as_ptr())), None, SW_SHOWNORMAL);
    }
}

pub fn launch(s: &Shortcut) {
    let target = expand(&s.target);
    let args = s.args.as_deref().map(expand);
    if s.hidden {
        let mut c = Command::new(&target);
        if let Some(a) = &args {
            c.raw_arg(a);
        }
        if let Err(e) = c.creation_flags(CREATE_NO_WINDOW).spawn() {
            util::log(&format!("shortcut {}: {e}", s.name));
        }
    } else {
        open(&target, args.as_deref());
    }
}
