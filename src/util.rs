use std::io::Write;
use std::path::PathBuf;

pub fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// YASB's config folder (honours YASB_CONFIG_HOME like YASB itself does).
pub fn yasb_dir() -> PathBuf {
    std::env::var_os("YASB_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config").join("yasb"))
}

static DEMO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `pocketisle.exe demo`: made-up data and a tour (see demo.rs).
pub fn set_demo() {
    DEMO.store(true, std::sync::atomic::Ordering::SeqCst);
}

pub fn demo() -> bool {
    DEMO.load(std::sync::atomic::Ordering::SeqCst)
}

/// Your settings folder, even in the demo.
pub fn real_config_dir() -> PathBuf {
    let p = home().join(".config").join("pocketisle");
    let _ = std::fs::create_dir_all(&p);
    p
}

/// Where settings and saves go: the demo has a folder of its own.
pub fn config_dir() -> PathBuf {
    if demo() {
        let p = crate::demo::dir();
        let _ = std::fs::create_dir_all(&p);
        return p;
    }
    real_config_dir()
}

/// Downloaded sprites and cries (shared with the demo, so it needn't fetch them again).
pub fn sprite_dir() -> PathBuf {
    real_config_dir().join("sprites")
}

/// YASB's calendar events (the demo's are made up).
pub fn calendar_file() -> PathBuf {
    if demo() {
        return config_dir().join("calendar-events.json");
    }
    yasb_dir().join("calendar-events.json")
}

pub fn log_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(home)
        .join("PocketIsle");
    let _ = std::fs::create_dir_all(&base);
    base.join("pocketisle.log")
}

pub fn log(msg: &str) {
    let path = log_path();
    if std::fs::metadata(&path).map(|m| m.len() > 512 * 1024).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let n = crate::calendar::local_now();
        let _ = writeln!(
            f,
            "[{:04}-{:02}-{:02} {:02}:{:02}:{:02}] {}",
            n.date.y, n.date.m, n.date.d, n.h, n.min, n.s, msg
        );
    }
}

/// Shorten to `max` chars with an ellipsis, on a char boundary.
pub fn clip(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max {
        s
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
