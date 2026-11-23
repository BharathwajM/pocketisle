//! Brings the island back if it crashes. The copy that starts with Windows only keeps watch:
//! it runs a second copy of itself that is the island, and starts it again if it stops
//! unexpectedly (not when it's quit, or ended on purpose).

use std::os::windows::io::AsRawHandle;
use std::process::Command;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
};

/// Set for the copy that is the island.
pub const CHILD: &str = "POCKETISLE_ISLAND";

/// Crashes this close together mean something is wrong with every start: stop trying.
const GIVE_UP: usize = 4;
const WINDOW: Duration = Duration::from_secs(10 * 60);

pub fn keep() {
    let Ok(exe) = std::env::current_exe() else { return };
    let job = job();
    let mut crashes: Vec<Instant> = Vec::new();
    loop {
        let mut child = match Command::new(&exe).env(CHILD, "1").spawn() {
            Ok(c) => c,
            Err(e) => {
                crate::util::log(&format!("couldn't start the island: {e}"));
                return;
            }
        };
        if let Some(j) = job {
            unsafe {
                let _ = AssignProcessToJobObject(j, HANDLE(child.as_raw_handle()));
            }
        }
        let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(0) as u32;
        if !crashed(code) {
            return;
        }
        crashes.retain(|t| t.elapsed() < WINDOW);
        crashes.push(Instant::now());
        if crashes.len() >= GIVE_UP {
            crate::util::log(&format!("the island stopped unexpectedly (0x{code:08X}) {GIVE_UP} times in 10 minutes: not starting it again"));
            return;
        }
        crate::util::log(&format!("the island stopped unexpectedly (0x{code:08X}): starting it again"));
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// A crash (a Windows exception, or a Rust panic), rather than quitting (0) or being ended
/// from Task Manager (1) or by a kill (-1).
fn crashed(code: u32) -> bool {
    code == 101 || (code >= 0xC000_0000 && code != 0xFFFF_FFFF)
}

/// A job that ends the island when this watcher ends (so stopping the start-up task stops
/// the island too). Whatever the island opens is left out of it.
fn job() -> Option<HANDLE> {
    unsafe {
        let j = CreateJobObjectW(None, None).ok()?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK;
        SetInformationJobObject(j, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of_val(&info) as u32).ok()?;
        Some(j)
    }
}

#[cfg(test)]
mod tests {
    use super::crashed;

    #[test]
    fn only_crashes_restart() {
        assert!(crashed(0xC000_0374)); // heap corruption
        assert!(crashed(0xC000_0005)); // access violation
        assert!(crashed(101)); // panic
        assert!(!crashed(0)); // quit
        assert!(!crashed(1)); // Task Manager
        assert!(!crashed(0xFFFF_FFFF)); // killed
    }
}
