//! Quick notes and to-dos for the Home tab, kept in `~/.config/pocketisle/notes.json`, and the
//! clipboard history's text (so recent copies are still there after a restart).

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Note {
    pub text: String,
    pub done: bool,
}

/// The most notes kept.
pub const MAX_NOTES: usize = 8;

fn path(name: &str) -> std::path::PathBuf {
    crate::util::config_dir().join(name)
}

pub fn load() -> Vec<Note> {
    std::fs::read_to_string(path("notes.json")).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
}

pub fn save(notes: &[Note]) {
    if let Ok(s) = serde_json::to_string_pretty(notes) {
        let _ = std::fs::write(path("notes.json"), s);
    }
}

pub fn load_clips() -> Vec<String> {
    std::fs::read_to_string(path("clips.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save_clips(texts: &[String]) {
    let p = path("clips.json");
    if texts.is_empty() {
        let _ = std::fs::remove_file(p);
    } else if let Ok(s) = serde_json::to_string(texts) {
        let _ = std::fs::write(p, s);
    }
}

/// Typing into a note: a key (WM_CHAR unit) changes the draft. Returns true when Enter
/// was pressed (add it), false otherwise. `hi` holds the first half of an emoji.
pub fn type_key(draft: &mut String, hi: &mut Option<u16>, unit: u16, paste: impl FnOnce() -> Option<String>) -> bool {
    match unit {
        13 => return true,
        8 => {
            draft.pop();
        }
        // Ctrl+Backspace: the last word.
        127 => {
            let t = draft.trim_end().len();
            let cut = draft[..t].rfind(' ').map(|i| i + 1).unwrap_or(0);
            draft.truncate(cut);
        }
        // Ctrl+V.
        22 => {
            if let Some(t) = paste() {
                let line = t.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
                draft.push_str(&line);
            }
        }
        0xD800..=0xDBFF => *hi = Some(unit),
        0xDC00..=0xDFFF => {
            if let Some(h) = hi.take() {
                draft.push_str(&String::from_utf16_lossy(&[h, unit]));
            }
        }
        u if u >= 32 => draft.push_str(&String::from_utf16_lossy(&[u])),
        _ => {}
    }
    if draft.chars().count() > 200 {
        *draft = draft.chars().take(200).collect();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_works() {
        let (mut d, mut hi) = (String::new(), None);
        for u in "Buy milk".encode_utf16() {
            assert!(!type_key(&mut d, &mut hi, u, || None));
        }
        assert_eq!(d, "Buy milk");
        type_key(&mut d, &mut hi, 8, || None);
        assert_eq!(d, "Buy mil");
        type_key(&mut d, &mut hi, 127, || None);
        assert_eq!(d, "Buy ");
        type_key(&mut d, &mut hi, 22, || Some("bread\nand eggs".into()));
        assert_eq!(d, "Buy bread and eggs");
        for u in "🥚".encode_utf16() {
            type_key(&mut d, &mut hi, u, || None);
        }
        assert!(d.ends_with('🥚'));
        assert!(type_key(&mut d, &mut hi, 13, || None));
    }
}
