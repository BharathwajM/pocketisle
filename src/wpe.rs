//! Wallpaper of the day, from Wallpaper Engine (Steam): every morning each screen gets a
//! different wallpaper from your own Wallpaper Engine library (the ones rated Everyone, never
//! one shown lately), through Wallpaper Engine's command line. Nothing is downloaded.
//!
//! It also tells the island which picture is on the first screen, so the accent colour can
//! follow Wallpaper Engine instead of the (unchanging) Windows wallpaper.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::events::{self, Event};
use crate::util;

/// One wallpaper in the library.
#[derive(Clone, Debug, PartialEq)]
pub struct Paper {
    /// Its folder's name (the Workshop id, or the project's folder).
    pub id: String,
    pub title: String,
    pub project: PathBuf,
    pub preview: Option<PathBuf>,
}

/// Where Wallpaper Engine is installed, and the folders its wallpapers are in.
#[derive(Clone, Debug)]
pub struct Install {
    pub dir: PathBuf,
    pub content: Vec<PathBuf>,
}

/// Steam's folder (from the registry, or the usual place).
fn steam_dir() -> Option<PathBuf> {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let mut buf = [0u16; 512];
    let mut n = (buf.len() * 2) as u32;
    let ok = unsafe { RegGetValueW(HKEY_CURRENT_USER, w!("Software\\Valve\\Steam"), w!("SteamPath"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut n)) }.is_ok();
    if ok {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let p = PathBuf::from(String::from_utf16_lossy(&buf[..len]).replace('/', "\\"));
        if p.exists() {
            return Some(p);
        }
    }
    let p = PathBuf::from(r"C:\Program Files (x86)\Steam");
    p.exists().then_some(p)
}

/// The Steam library folders listed in libraryfolders.vdf (and Steam's own).
pub fn library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix("\"path\"")?.trim();
            let v = rest.trim_matches('"');
            (!v.is_empty()).then(|| PathBuf::from(v.replace("\\\\", "\\")))
        })
        .collect()
}

pub fn install() -> Option<Install> {
    let steam = steam_dir()?;
    let mut libs = vec![steam.clone()];
    if let Ok(vdf) = std::fs::read_to_string(steam.join(r"steamapps\libraryfolders.vdf")) {
        for p in library_paths(&vdf) {
            if !libs.iter().any(|l| l.to_string_lossy().eq_ignore_ascii_case(&p.to_string_lossy())) {
                libs.push(p);
            }
        }
    }
    let lib = libs.iter().find(|l| l.join(r"steamapps\common\wallpaper_engine").exists())?;
    let dir = lib.join(r"steamapps\common\wallpaper_engine");
    let mut content: Vec<PathBuf> = libs.iter().map(|l| l.join(r"steamapps\workshop\content\431960")).filter(|p| p.exists()).collect();
    content.push(dir.join(r"projects\myprojects"));
    content.push(dir.join(r"projects\defaultprojects"));
    Some(Install { dir, content })
}

/// A wallpaper's details from its project.json. Only scenes, videos and web pages rated
/// Everyone (apps and anything rated Questionable or Mature are left out).
pub fn read_project(folder: &Path) -> Option<Paper> {
    let project = folder.join("project.json");
    let v: Value = serde_json::from_slice(&std::fs::read(&project).ok()?).ok()?;
    paper_from(&v, folder)
}

pub fn paper_from(v: &Value, folder: &Path) -> Option<Paper> {
    let kind = v["type"].as_str().unwrap_or("").to_lowercase();
    if !matches!(kind.as_str(), "scene" | "video" | "web") {
        return None;
    }
    let rating = v["contentrating"].as_str().unwrap_or("Everyone");
    if !rating.eq_ignore_ascii_case("everyone") {
        return None;
    }
    let title = v["title"].as_str().unwrap_or("").trim().to_string();
    let id = folder.file_name()?.to_string_lossy().into_owned();
    let preview = v["preview"].as_str().map(|p| folder.join(p)).filter(|p| p.exists());
    Some(Paper { title: if title.is_empty() { id.clone() } else { title }, id, project: folder.join("project.json"), preview })
}

/// Every wallpaper you have.
pub fn library(i: &Install) -> Vec<Paper> {
    let mut out = Vec::new();
    for c in &i.content {
        let Ok(rd) = std::fs::read_dir(c) else { continue };
        for e in rd.flatten() {
            if let Some(p) = read_project(&e.path()) {
                out.push(p);
            }
        }
    }
    out
}

/// The wallpaper on each screen now (screen number, its folder), from Wallpaper Engine's
/// settings.
pub fn selected(i: &Install) -> Vec<(u32, PathBuf)> {
    let Ok(bytes) = std::fs::read(i.dir.join("config.json")) else { return Vec::new() };
    let Ok(v) = serde_json::from_slice::<Value>(&bytes) else { return Vec::new() };
    selected_from(&v)
}

pub fn selected_from(v: &Value) -> Vec<(u32, PathBuf)> {
    let mut out = Vec::new();
    let Some(users) = v.as_object() else { return out };
    for (_, u) in users {
        let Some(sel) = u["general"]["wallpaperconfig"]["selectedwallpapers"].as_object() else { continue };
        for (k, m) in sel {
            let Some(n) = k.strip_prefix("Monitor").and_then(|n| n.parse::<u32>().ok()) else { continue };
            let Some(file) = m["file"].as_str() else { continue };
            let p = PathBuf::from(file.replace('/', "\\"));
            if let Some(dir) = p.parent() {
                if !out.iter().any(|(s, _)| *s == n) {
                    out.push((n, dir.to_path_buf()));
                }
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out.sort_by_key(|(n, _)| *n);
    out
}

/// Wallpaper Engine's program.
fn exe(i: &Install) -> PathBuf {
    let x64 = i.dir.join("wallpaper64.exe");
    if x64.exists() {
        x64
    } else {
        i.dir.join("wallpaper32.exe")
    }
}

/// Is Wallpaper Engine running? (Looked at every half minute at most.)
fn running() -> bool {
    static LAST: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let mut g = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, on)) = *g {
        if at.elapsed() < Duration::from_secs(30) {
            return on;
        }
    }
    let on = crate::procs::running(&["wallpaper64.exe", "wallpaper32.exe"]);
    *g = Some((Instant::now(), on));
    on
}

/// The preview picture of the wallpaper on the first screen, while Wallpaper Engine is
/// running (for the island's accent colour). Only reads its settings when they change.
pub fn current_preview() -> Option<PathBuf> {
    static INSTALL: std::sync::OnceLock<Option<Install>> = std::sync::OnceLock::new();
    static CACHE: Mutex<Option<(Option<SystemTime>, Option<PathBuf>)>> = Mutex::new(None);
    let i = INSTALL.get_or_init(install).as_ref()?;
    if !running() {
        return None;
    }
    let modified = std::fs::metadata(i.dir.join("config.json")).ok().and_then(|m| m.modified().ok());
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((m, p)) = g.as_ref() {
        if *m == modified {
            return p.clone();
        }
    }
    let p = selected(i).first().and_then(|(_, dir)| read_project(dir)).and_then(|p| p.preview);
    *g = Some((modified, p.clone()));
    p
}

/// Pick `n` different wallpapers, none of them in `avoid` (unless that leaves too few).
/// `seed` makes the choice.
pub fn choose(lib: &[Paper], avoid: &[String], n: usize, seed: u64) -> Vec<usize> {
    let mut pool: Vec<usize> = (0..lib.len()).filter(|&i| !avoid.contains(&lib[i].id)).collect();
    if pool.len() < n {
        pool = (0..lib.len()).collect();
    }
    let mut out = Vec::new();
    let mut s = seed | 1;
    while out.len() < n && !pool.is_empty() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let k = (s % pool.len() as u64) as usize;
        out.push(pool.swap_remove(k));
    }
    out
}

/// What's been shown: the day it last changed, and the wallpapers shown lately.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Day {
    day: String,
    history: Vec<String>,
}

fn day_path() -> PathBuf {
    util::config_dir().join("wallpaper-of-the-day.json")
}

fn load_day() -> Day {
    std::fs::read(day_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_day(d: &Day) {
    if let Ok(s) = serde_json::to_string_pretty(d) {
        let _ = std::fs::write(day_path(), s);
    }
}

fn today() -> String {
    let n = crate::calendar::local_now();
    format!("{:04}-{:02}-{:02}", n.date.y, n.date.m, n.date.d)
}

/// The titles of what's on each screen now.
fn titles_now(i: &Install) -> Vec<String> {
    selected(i).iter().filter_map(|(_, dir)| read_project(dir)).map(|p| p.title).collect()
}

/// One at a time.
static BUSY: Mutex<()> = Mutex::new(());

/// Put a fresh wallpaper on each screen. `announce`: say so with a card.
fn change(i: &Install, announce: bool) {
    let _g = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    let lib = library(i);
    if lib.is_empty() {
        util::log("wallpaper of the day: no wallpapers rated Everyone in Wallpaper Engine");
        return;
    }
    let screens: Vec<u32> = {
        let s: Vec<u32> = selected(i).into_iter().map(|(n, _)| n).collect();
        if s.is_empty() {
            vec![0]
        } else {
            s
        }
    };
    let mut day = load_day();
    let mut avoid = day.history.clone();
    avoid.extend(selected(i).iter().filter_map(|(_, d)| d.file_name().map(|f| f.to_string_lossy().into_owned())));
    let seed = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7);
    let picks = choose(&lib, &avoid, screens.len(), seed);
    let exe = exe(i);
    let mut titles = Vec::new();
    for (screen, k) in screens.iter().zip(&picks) {
        let p = &lib[*k];
        let ok = std::process::Command::new(&exe)
            .args(["-control", "openWallpaper", "-file"])
            .arg(&p.project)
            .args(["-monitor", &screen.to_string()])
            .creation_flags(0x0800_0000)
            .spawn()
            .is_ok();
        util::log(&format!("wallpaper of the day: screen {} → {}{}", screen + 1, p.title, if ok { "" } else { " (couldn't start Wallpaper Engine)" }));
        if ok {
            titles.push(p.title.clone());
            day.history.push(p.id.clone());
        }
        // Give it a moment between screens.
        std::thread::sleep(Duration::from_millis(800));
    }
    // Remember the last 60 (no repeats for a couple of months, when there are enough).
    let keep = 60.min(lib.len().saturating_sub(screens.len()));
    if day.history.len() > keep {
        let cut = day.history.len() - keep;
        day.history.drain(..cut);
    }
    day.day = today();
    save_day(&day);
    if !titles.is_empty() {
        events::send(Event::Wallpaper { titles, announce });
    }
}

use std::os::windows::process::CommandExt;

static INSTALLED: std::sync::OnceLock<Option<Install>> = std::sync::OnceLock::new();

fn installed() -> Option<&'static Install> {
    INSTALLED.get_or_init(install).as_ref()
}

/// Watch the clock: each morning (from 6 am) a new wallpaper. The very first time, it only
/// remembers today, so nothing changes until tomorrow morning.
pub fn spawn(on: bool) {
    let _ = std::thread::Builder::new().name("wallpaper of the day".into()).spawn(move || {
        let Some(i) = installed() else { return };
        events::send(Event::Wallpaper { titles: titles_now(i), announce: false });
        if !on {
            return;
        }
        loop {
            let now = crate::calendar::local_now();
            let mut day = load_day();
            if now.h >= 6 && day.day != today() {
                if day.day.is_empty() {
                    day.day = today();
                    save_day(&day);
                    util::log("wallpaper of the day: starts tomorrow morning");
                } else if running() {
                    change(i, true);
                }
            }
            std::thread::sleep(Duration::from_secs(5 * 60));
        }
    });
}

/// "Another": a fresh wallpaper now.
pub fn change_now() {
    let _ = std::thread::Builder::new().name("wallpaper now".into()).spawn(|| {
        if let Some(i) = installed() {
            change(i, false);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_steam_and_wallpaper_engine_files() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}";
        assert_eq!(library_paths(vdf), vec![PathBuf::from(r"C:\Program Files (x86)\Steam"), PathBuf::from(r"D:\SteamLibrary")]);
        let cfg: Value = serde_json::from_str(r#"{"someone":{"general":{"wallpaperconfig":{"selectedwallpapers":{
            "Monitor1":{"file":"C:/Steam/steamapps/workshop/content/431960/222/scene.pkg"},
            "Monitor0":{"file":"C:/Steam/steamapps/workshop/content/431960/111/scene.pkg"}}}}}}"#).unwrap();
        let sel = selected_from(&cfg);
        assert_eq!(sel.len(), 2);
        assert_eq!(sel[0].0, 0);
        assert!(sel[0].1.ends_with("111"));
        let dir = PathBuf::from(r"C:\nowhere\123");
        let ok: Value = serde_json::from_str(r#"{"title":"Rainy city","type":"video","contentrating":"Everyone"}"#).unwrap();
        assert_eq!(paper_from(&ok, &dir).map(|p| p.title), Some("Rainy city".into()));
        let mature: Value = serde_json::from_str(r#"{"title":"X","type":"scene","contentrating":"Mature"}"#).unwrap();
        assert!(paper_from(&mature, &dir).is_none());
        let app: Value = serde_json::from_str(r#"{"title":"X","type":"application"}"#).unwrap();
        assert!(paper_from(&app, &dir).is_none());
        let old: Value = serde_json::from_str(r#"{"title":"No rating","type":"scene"}"#).unwrap();
        assert!(paper_from(&old, &dir).is_some());
    }

    #[test]
    fn picks_fresh_ones() {
        let lib: Vec<Paper> = (0..5).map(|i| Paper { id: i.to_string(), title: format!("W{i}"), project: PathBuf::new(), preview: None }).collect();
        let avoid = vec!["0".to_string(), "1".to_string(), "2".to_string()];
        let picks = choose(&lib, &avoid, 2, 42);
        assert_eq!(picks.len(), 2);
        assert_ne!(picks[0], picks[1]);
        assert!(picks.iter().all(|&k| k == 3 || k == 4));
        // Too few fresh ones: anything goes.
        assert_eq!(choose(&lib, &avoid, 4, 42).len(), 4);
        assert!(choose(&[], &[], 2, 1).is_empty());
    }
}
