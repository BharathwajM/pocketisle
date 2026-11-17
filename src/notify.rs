//! Windows notifications, shown as island cards.
//!
//! The shell's notification counter (a WNF state, the same signal YASB's notification widget
//! listens to) wakes this thread when a toast arrives; it then reads only the new toasts from
//! UserNotificationListener. No polling.

use std::collections::HashSet;
use std::ffi::c_void;
use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;
use std::time::Duration;

use windows::core::{Interface, Result};
use windows::Foundation::Size;
use windows::Storage::Streams::IRandomAccessStreamReference;
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::events::{self, Event};

// Win11 "total notifications" state, falling back to the Win10 unread badge.
const WNF_SHEL_NOTIFICATION_TOTAL: u64 = 0x0D83_063E_A3B8_D035;
const WNF_SHEL_NOTIFICATIONS: u64 = 0x0D83_063E_A3BC_1035;

type WnfCallback = unsafe extern "system" fn(u64, u32, *const c_void, *const c_void, *const c_void, u32) -> i32;

#[link(name = "ntdll")]
extern "system" {
    fn RtlSubscribeWnfStateChangeNotification(
        subscription: *mut *mut c_void,
        state_name: u64,
        change_stamp: u32,
        callback: WnfCallback,
        context: *const c_void,
        type_id: *const c_void,
        serialization_group: u32,
        flags: u32,
    ) -> i32;
}

static WAKE: OnceLock<Sender<()>> = OnceLock::new();

unsafe extern "system" fn on_wnf(_: u64, _: u32, _: *const c_void, _: *const c_void, _: *const c_void, _: u32) -> i32 {
    if let Some(tx) = WAKE.get() {
        let _ = tx.send(());
    }
    0
}

pub fn spawn(ignore: Vec<String>) {
    std::thread::spawn(move || {
        if let Err(e) = run(ignore) {
            crate::util::log(&format!("notification watcher stopped: {e}"));
        }
    });
}

fn run(ignore: Vec<String>) -> Result<()> {
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let listener = UserNotificationListener::Current()?;
    let access = listener.RequestAccessAsync()?.join()?;
    if access != UserNotificationListenerAccessStatus::Allowed {
        crate::util::log("notification access not allowed (Settings > Privacy > Notifications)");
        return Ok(());
    }
    let ignore: Vec<String> = ignore.into_iter().map(|s| s.to_lowercase()).collect();

    let mut seen: HashSet<u32> = HashSet::new();
    for n in listener.GetNotificationsAsync(NotificationKinds::Toast)?.join()? {
        if let Ok(id) = n.Id() {
            seen.insert(id);
        }
    }

    let (tx, rx) = channel::<()>();
    let _ = WAKE.set(tx);
    let mut sub: *mut c_void = std::ptr::null_mut();
    let subscribed = unsafe {
        RtlSubscribeWnfStateChangeNotification(&mut sub, WNF_SHEL_NOTIFICATION_TOTAL, 0, on_wnf, std::ptr::null(), std::ptr::null(), 0, 0) == 0
            || RtlSubscribeWnfStateChangeNotification(&mut sub, WNF_SHEL_NOTIFICATIONS, 0, on_wnf, std::ptr::null(), std::ptr::null(), 0, 0) == 0
    };
    if !subscribed {
        crate::util::log("WNF subscribe failed; checking notifications every 3 s instead");
    }

    loop {
        if subscribed {
            if rx.recv().is_err() {
                break;
            }
        } else {
            let _ = rx.recv_timeout(Duration::from_secs(3));
        }
        // Several state changes arrive per toast; let them settle, then read once.
        std::thread::sleep(Duration::from_millis(120));
        while rx.try_recv().is_ok() {}

        let Ok(list) = listener.GetNotificationsAsync(NotificationKinds::Toast).and_then(|op| op.join()) else {
            continue;
        };
        let mut current = HashSet::new();
        let mut fresh: Vec<UserNotification> = Vec::new();
        for n in list {
            let Ok(id) = n.Id() else { continue };
            current.insert(id);
            if !seen.contains(&id) {
                fresh.push(n);
            }
        }
        seen = current;
        // Show the newest last, so it ends up on screen first after the queue.
        for n in fresh.into_iter().take(3) {
            if let Some(ev) = describe(&n, &ignore) {
                events::send(ev);
            }
        }
    }
    Ok(())
}

fn describe(n: &UserNotification, ignore: &[String]) -> Option<Event> {
    let info = n.AppInfo().and_then(|a| a.DisplayInfo()).ok();
    let app = info.as_ref().and_then(|d| d.DisplayName().ok()).map(|h| h.to_string()).unwrap_or_default();
    if ignore.iter().any(|i| *i == app.to_lowercase()) {
        return None;
    }
    let binding = n
        .Notification()
        .and_then(|t| t.Visual())
        .and_then(|v| v.GetBinding(&KnownNotificationBindings::ToastGeneric()?))
        .ok()?;
    let mut parts: Vec<String> = Vec::new();
    for t in binding.GetTextElements().ok()? {
        if let Ok(s) = t.Text() {
            let s = s.to_string();
            if !s.trim().is_empty() {
                parts.push(s);
            }
        }
    }
    if parts.is_empty() {
        return None;
    }
    let title = parts.remove(0);
    let body = parts.join(" · ");
    let icon = info
        .and_then(|d| d.GetLogo(Size { Width: 64.0, Height: 64.0 }).ok())
        .and_then(|r| r.cast::<IRandomAccessStreamReference>().ok())
        .and_then(|r| crate::imaging::read_stream(&r).ok())
        .and_then(|b| crate::imaging::decode(&b, 64).ok());
    Some(Event::Notification { app, title, body, icon })
}
