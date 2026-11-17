//! The System tab's Storage page: what's taking up space in a folder, biggest first.
//!
//! Opening a folder lists its files straight away, then adds up each subfolder on a
//! background thread (largest first as they finish). Every folder's total found on the way is
//! remembered, so going into a subfolder afterwards is instant. Links and junctions aren't
//! followed (no counting twice), and cloud-only files (OneDrive placeholders) don't count, as
//! they take no space on the disk.

use std::collections::HashMap;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    /// Bytes (for a folder, everything inside it; None while it's still being added up).
    pub size: Option<u64>,
    pub dir: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub path: PathBuf,
    /// Biggest first; folders still being added up at the end.
    pub entries: Vec<Entry>,
    pub done: bool,
    /// Files counted so far, and folders that couldn't be read (no permission).
    pub files: u64,
    pub denied: u64,
    pub secs: f32,
}

impl Listing {
    pub fn total(&self) -> u64 {
        self.entries.iter().filter_map(|e| e.size).sum()
    }
}

struct State {
    current: Listing,
    /// Folder totals found so far (folders of 1 MB and more, and the ones listed).
    sizes: HashMap<PathBuf, u64>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State { current: Listing::default(), sizes: HashMap::new() }))
}

/// Bumped by every `open`, so an older scan stops.
static GEN: AtomicU64 = AtomicU64::new(0);

const REPARSE: u32 = 0x400;
const OFFLINE: u32 = 0x1000;
const RECALL_ON_OPEN: u32 = 0x40000;
const RECALL_ON_DATA: u32 = 0x400000;

fn cloud_only(attrs: u32) -> bool {
    attrs & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA) != 0
}

/// What's in the folder being shown right now.
pub fn current() -> Listing {
    state().lock().map(|s| s.current.clone()).unwrap_or_default()
}

/// Forget what's on screen (back to the list of drives).
pub fn close() {
    GEN.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut s) = state().lock() {
        s.current = Listing::default();
    }
}

/// Forget remembered sizes (to scan again after deleting things).
pub fn forget(path: &Path) {
    if let Ok(mut s) = state().lock() {
        s.sizes.retain(|p, _| !p.starts_with(path));
    }
}

/// Show a folder: its files at once, its folders as they're added up. `wake` is called
/// (from the scanning thread) whenever there's something new to draw.
pub fn open(path: PathBuf, wake: fn()) {
    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let started = Instant::now();
    let mut entries = Vec::new();
    let mut dirs = Vec::new();
    let mut denied = 0;
    match std::fs::read_dir(&path) {
        Ok(rd) => {
            for e in rd.flatten() {
                let Ok(md) = e.metadata() else { continue };
                let attrs = md.file_attributes();
                let name = e.file_name().to_string_lossy().into_owned();
                if md.is_dir() {
                    if attrs & REPARSE != 0 {
                        continue;
                    }
                    dirs.push(name.clone());
                    entries.push(Entry { name, size: None, dir: true });
                } else if !cloud_only(attrs) {
                    entries.push(Entry { name, size: Some(md.file_size()), dir: false });
                }
            }
        }
        Err(_) => denied += 1,
    }
    // Folders already added up don't need it again.
    if let Ok(s) = state().lock() {
        for e in entries.iter_mut().filter(|e| e.dir) {
            if let Some(&n) = s.sizes.get(&path.join(&e.name)) {
                e.size = Some(n);
            }
        }
    }
    sort(&mut entries);
    let pending: Vec<String> = entries.iter().filter(|e| e.dir && e.size.is_none()).map(|e| e.name.clone()).collect();
    let files = entries.iter().filter(|e| !e.dir).count() as u64;
    if let Ok(mut s) = state().lock() {
        s.current = Listing { path: path.clone(), entries, done: pending.is_empty(), files, denied, secs: 0.0 };
    }
    wake();
    if pending.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new().name("scan".into()).spawn(move || {
        let mut last_wake = Instant::now();
        let mut counts = (files, denied);
        for name in pending {
            let dir = path.join(&name);
            let mut found = Vec::new();
            let Some(total) = walk(&dir, gen, &mut counts, &mut found) else { return };
            let Ok(mut s) = state().lock() else { return };
            if GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            s.sizes.extend(found);
            s.sizes.insert(dir, total);
            if let Some(e) = s.current.entries.iter_mut().find(|e| e.dir && e.name == name) {
                e.size = Some(total);
            }
            sort(&mut s.current.entries);
            s.current.files = counts.0;
            s.current.denied = counts.1;
            s.current.secs = started.elapsed().as_secs_f32();
            drop(s);
            if last_wake.elapsed().as_millis() > 200 {
                last_wake = Instant::now();
                wake();
            }
        }
        if let Ok(mut s) = state().lock() {
            if GEN.load(Ordering::SeqCst) == gen {
                s.current.done = true;
                s.current.secs = started.elapsed().as_secs_f32();
            }
        }
        wake();
    });
    let _ = (dirs, gen);
}

/// Everything under `dir`, in bytes (None if a newer scan took over). Totals of 1 MB and
/// more go into `found` along the way.
fn walk(dir: &Path, gen: u64, counts: &mut (u64, u64), found: &mut Vec<(PathBuf, u64)>) -> Option<u64> {
    if GEN.load(Ordering::Relaxed) != gen {
        return None;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        counts.1 += 1;
        return Some(0);
    };
    let mut total = 0u64;
    for e in rd.flatten() {
        let Ok(md) = e.metadata() else { continue };
        let attrs = md.file_attributes();
        if md.is_dir() {
            if attrs & REPARSE == 0 {
                total += walk(&e.path(), gen, counts, found)?;
            }
        } else if !cloud_only(attrs) {
            total += md.file_size();
            counts.0 += 1;
        }
    }
    if total >= 1 << 20 {
        found.push((dir.to_path_buf(), total));
    }
    Some(total)
}

fn sort(entries: &mut [Entry]) {
    entries.sort_by(|a, b| match (a.size, b.size) {
        (Some(x), Some(y)) => y.cmp(&x).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
}

/// "1.4 GB", "350 MB", "12 KB".
pub fn human(b: u64) -> String {
    let f = b as f64;
    if f >= 1_099_511_627_776.0 {
        format!("{:.2} TB", f / 1_099_511_627_776.0)
    } else if f >= 1_073_741_824.0 {
        format!("{:.1} GB", f / 1_073_741_824.0)
    } else if f >= 1_048_576.0 {
        format!("{:.0} MB", f / 1_048_576.0)
    } else if f >= 1024.0 {
        format!("{:.0} KB", f / 1024.0)
    } else {
        format!("{b} B")
    }
}

/// Show something in File Explorer (selected), or open a folder there.
pub fn reveal(path: &Path, select: bool) {
    let mut c = std::process::Command::new("explorer.exe");
    if select {
        c.arg(format!("/select,{}", path.display()));
    } else {
        c.arg(path);
    }
    let _ = c.spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_order() {
        assert_eq!(human(1536), "2 KB");
        assert_eq!(human(350 * 1_048_576), "350 MB");
        assert_eq!(human(3 * 1_073_741_824 / 2), "1.5 GB");
        let mut v = vec![
            Entry { name: "b".into(), size: Some(5), dir: false },
            Entry { name: "pending".into(), size: None, dir: true },
            Entry { name: "a".into(), size: Some(50), dir: true },
        ];
        sort(&mut v);
        assert_eq!(v.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["a", "b", "pending"]);
    }

    #[test]
    fn scans_a_folder() {
        let dir = std::env::temp_dir().join(format!("pocketisle-scan-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("big").join("inner")).unwrap();
        std::fs::write(dir.join("big").join("inner").join("x.bin"), vec![0u8; 3000]).unwrap();
        std::fs::write(dir.join("big").join("y.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(dir.join("small.txt"), vec![0u8; 10]).unwrap();
        fn nothing() {}
        open(dir.clone(), nothing);
        let t = Instant::now();
        while !current().done && t.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let l = current();
        assert!(l.done);
        assert_eq!(l.entries[0].name, "big");
        assert_eq!(l.entries[0].size, Some(4000));
        assert_eq!(l.entries[1].name, "small.txt");
        assert_eq!(l.total(), 4010);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
