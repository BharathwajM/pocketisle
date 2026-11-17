//! The Game tab's library: the games you've opened (kept in `games.json` next to the
//! config), the "Open game…" file picker, and starting / finding the player (`emu.rs`, run as
//! `pocketisle.exe play <game> <title>` in a process of its own).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE};

use crate::emu;
use crate::util;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Game {
    pub path: String,
    pub title: String,
    /// "DS", "GBA", "GB" or "GBC".
    pub system: String,
    /// When you last played it (unix seconds; 0 = never).
    #[serde(default)]
    pub last: i64,
}

fn file() -> PathBuf {
    util::config_dir().join("games.json")
}

pub fn load() -> Vec<Game> {
    std::fs::read_to_string(file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(games: &[Game]) {
    if let Ok(s) = serde_json::to_string_pretty(games) {
        let _ = std::fs::write(file(), s);
    }
}

/// "Pokemon_SoulSilver_Decrypted[romsretro.com].zip" → "Pokemon SoulSilver".
pub fn title_from(name: &str) -> String {
    let stem = Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let mut out = String::new();
    let mut depth = 0;
    for c in stem.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(if c == '_' { ' ' } else { c }),
            _ => {}
        }
    }
    let words: Vec<&str> = out.split_whitespace().filter(|w| !matches!(w.to_ascii_lowercase().as_str(), "decrypted" | "trimmed" | "rom")).collect();
    let t = words.join(" ");
    let t = t.replace("Pokemon", "Pokémon");
    if t.is_empty() {
        stem.to_string()
    } else {
        t
    }
}

/// A game file you picked, ready for the library (None if it isn't one we can play).
pub fn entry(path: &Path) -> Option<Game> {
    let name = path.file_name()?.to_str()?;
    let system = match emu::system_of(name) {
        Some(s) => s,
        None if name.to_ascii_lowercase().ends_with(".zip") => emu::system_of(&emu::zip_game(path)?)?,
        None => return None,
    };
    Some(Game { path: path.to_string_lossy().into_owned(), title: title_from(name), system: system.into(), last: 0 })
}

/// Whether the emulator for this system is installed.
pub fn has_core(system: &str) -> bool {
    emu::core_for(system).exists()
}

/// The "Open game…" dialog, on a thread of its own; the choice comes back as an event.
pub fn pick() {
    let _ = std::thread::Builder::new().name("pick-game".into()).spawn(|| {
        let mut buf = vec![0u16; 1024];
        let start = util::home().join("Downloads").join("Games");
        let start: Vec<u16> = start.to_string_lossy().encode_utf16().chain([0]).collect();
        let filter: Vec<u16> = "DS, GBA and Game Boy games\0*.nds;*.gba;*.gb;*.gbc;*.zip\0All files\0*.*\0\0".encode_utf16().collect();
        let mut ofn = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            lpstrFilter: PCWSTR(filter.as_ptr()),
            lpstrFile: PWSTR(buf.as_mut_ptr()),
            nMaxFile: buf.len() as u32,
            lpstrInitialDir: PCWSTR(start.as_ptr()),
            lpstrTitle: w!("Open a game"),
            Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
            ..Default::default()
        };
        if unsafe { GetOpenFileNameW(&mut ofn) }.as_bool() {
            let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let path = PathBuf::from(String::from_utf16_lossy(&buf[..n]));
            crate::events::send(crate::events::Event::GamePicked(path));
        }
    });
}

/// Start the player for a game.
pub fn launch(g: &Game) -> Option<std::process::Child> {
    let exe = std::env::current_exe().ok()?;
    // Let the player's window take the focus (the click was on the island).
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(windows::Win32::UI::WindowsAndMessaging::ASFW_ANY);
    }
    match std::process::Command::new(exe).arg("play").arg(&g.path).arg(&g.title).spawn() {
        Ok(c) => Some(c),
        Err(e) => {
            util::log(&format!("game: couldn't start the player: {e}"));
            None
        }
    }
}

/// Bring the player's window to the front.
pub fn show() {
    unsafe {
        if let Ok(h) = FindWindowW(emu::CLASS, PCWSTR::null()) {
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            let _ = SetForegroundWindow(h);
        }
    }
}

/// Ask the player to close (it saves first).
pub fn stop() {
    unsafe {
        if let Ok(h) = FindWindowW(emu::CLASS, PCWSTR::null()) {
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                Some(h),
                windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                windows::Win32::Foundation::WPARAM(0),
                windows::Win32::Foundation::LPARAM(0),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_titles() {
        assert_eq!(title_from("Pokemon_SoulSilver_Decrypted[romsretro.com].zip"), "Pokémon SoulSilver");
        assert_eq!(title_from("Some Game (USA) (Rev 1).gba"), "Some Game");
        assert_eq!(emu::system_of("x.NDS"), Some("DS"));
        assert_eq!(emu::system_of("x.gbc"), Some("GBC"));
        assert_eq!(emu::system_of("x.zip"), None);
    }
}
