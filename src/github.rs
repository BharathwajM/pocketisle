//! Your open pull requests on GitHub: a peek when a PR's checks pass or fail, or when
//! someone reviews it. Pocket Isle asks the GitHub CLI (`gh`), which you sign in to yourself
//! (`gh auth login`); Pocket Isle never sees or stores a token. Without `gh` this stays quiet.

use std::collections::BTreeMap;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::events::{self, Event};
use crate::util;

/// Something worth a peek.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub kind: NoteKind,
    /// "owner/repo #12"
    pub place: String,
    pub title: String,
    pub url: String,
    /// Who reviewed (for reviews).
    pub who: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoteKind {
    Passed,
    Failed,
    Approved,
    ChangesRequested,
    Commented,
}

/// What was last seen, so a restart doesn't repeat peeks.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Seen {
    /// PR url → "commit:STATE" of the last check result told about.
    pub checks: BTreeMap<String, String>,
    /// Review ids already told about.
    pub reviews: Vec<String>,
}

fn store() -> PathBuf {
    util::config_dir().join("github.json")
}

fn load() -> Option<Seen> {
    std::fs::read_to_string(store()).ok().and_then(|s| serde_json::from_str(&s).ok())
}

fn save(s: &Seen) {
    if let Ok(t) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(store(), t);
    }
}

/// Where `gh` is: on the PATH, or where its installer puts it (the island may have been
/// started before it was installed, with the old PATH).
fn gh() -> Option<PathBuf> {
    let mut places: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).map(|d| d.join("gh.exe")).collect()).unwrap_or_default();
    for (var, sub) in [("ProgramFiles", "GitHub CLI"), ("LOCALAPPDATA", "Programs\\GitHub CLI")] {
        if let Some(v) = std::env::var_os(var) {
            places.push(PathBuf::from(v).join(sub).join("gh.exe"));
        }
    }
    if let Some(v) = std::env::var_os("LOCALAPPDATA") {
        places.push(PathBuf::from(v).join("Microsoft\\WinGet\\Links\\gh.exe"));
    }
    places.into_iter().find(|p| p.is_file())
}

const QUERY: &str = "query{viewer{login pullRequests(first:30,states:OPEN,orderBy:{field:UPDATED_AT,direction:DESC}){nodes{number title url repository{nameWithOwner} commits(last:1){nodes{commit{oid statusCheckRollup{state}}}} reviews(last:10){nodes{id state author{login}}}}}}}";

/// Ask GitHub, through `gh`, about your open pull requests.
fn fetch(gh: &PathBuf) -> Result<Value, String> {
    let mut child = Command::new(gh)
        .args(["api", "graphql", "-f", &format!("query={QUERY}")])
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x0800_0000)
        .spawn()
        .map_err(|e| e.to_string())?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > Duration::from_secs(40) => {
                let _ = child.kill();
                return Err("gh took too long".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("gh failed").trim();
        return Err(util::clip(line, 140));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}

/// Compare GitHub's answer with what was seen before: the peeks to show, and what's seen now.
/// With nothing seen before (the first look), it only takes note.
pub fn changes(v: &Value, before: Option<&Seen>) -> (Vec<Note>, Seen) {
    let me = v["data"]["viewer"]["login"].as_str().unwrap_or("");
    let mut seen = Seen::default();
    let mut notes = Vec::new();
    let empty = Vec::new();
    for pr in v["data"]["viewer"]["pullRequests"]["nodes"].as_array().unwrap_or(&empty) {
        let url = pr["url"].as_str().unwrap_or("").to_string();
        if url.is_empty() {
            continue;
        }
        let place = format!("{} #{}", pr["repository"]["nameWithOwner"].as_str().unwrap_or(""), pr["number"].as_u64().unwrap_or(0));
        let title = pr["title"].as_str().unwrap_or("").to_string();
        let note = |kind, who: &str| Note { kind, place: place.clone(), title: title.clone(), url: url.clone(), who: who.to_string() };
        let commit = &pr["commits"]["nodes"][0]["commit"];
        let oid = commit["oid"].as_str().unwrap_or("");
        let state = commit["statusCheckRollup"]["state"].as_str().unwrap_or("");
        let key = format!("{oid}:{state}");
        if matches!(state, "SUCCESS" | "FAILURE" | "ERROR") {
            let old = before.and_then(|b| b.checks.get(&url));
            if before.is_some() && old != Some(&key) {
                notes.push(note(if state == "SUCCESS" { NoteKind::Passed } else { NoteKind::Failed }, ""));
            }
        }
        seen.checks.insert(url.clone(), key);
        for r in pr["reviews"]["nodes"].as_array().unwrap_or(&empty) {
            let id = r["id"].as_str().unwrap_or("").to_string();
            let who = r["author"]["login"].as_str().unwrap_or("someone");
            if id.is_empty() {
                continue;
            }
            let new = before.is_some_and(|b| !b.reviews.contains(&id));
            if new && who != me {
                let kind = match r["state"].as_str().unwrap_or("") {
                    "APPROVED" => Some(NoteKind::Approved),
                    "CHANGES_REQUESTED" => Some(NoteKind::ChangesRequested),
                    "COMMENTED" => Some(NoteKind::Commented),
                    _ => None,
                };
                if let Some(k) = kind {
                    notes.push(note(k, who));
                }
            }
            seen.reviews.push(id);
        }
    }
    // Keep the reviews of PRs that closed out of the list too (bounded).
    if let Some(b) = before {
        for id in &b.reviews {
            if !seen.reviews.contains(id) && seen.reviews.len() < 400 {
                seen.reviews.push(id.clone());
            }
        }
    }
    (notes, seen)
}

/// Look every `minutes` minutes, on a thread of its own.
pub fn watch(minutes: u32) {
    let every = Duration::from_secs(minutes.clamp(1, 60) as u64 * 60);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(20));
        let mut seen = load();
        let mut last_err = String::new();
        loop {
            match gh() {
                None => {
                    if last_err != "no gh" {
                        util::log("GitHub peeks: the GitHub CLI (gh) isn't installed");
                        last_err = "no gh".into();
                    }
                    std::thread::sleep(Duration::from_secs(30 * 60));
                    continue;
                }
                Some(path) => match fetch(&path) {
                    Ok(v) => {
                        let (notes, now) = changes(&v, seen.as_ref());
                        if !last_err.is_empty() {
                            util::log("GitHub peeks: working");
                            last_err.clear();
                        }
                        if seen.as_ref() != Some(&now) {
                            save(&now);
                        }
                        seen = Some(now);
                        if !notes.is_empty() {
                            events::send(Event::GitHub(notes));
                        }
                    }
                    Err(e) => {
                        if e != last_err {
                            util::log(&format!("GitHub peeks: {e}"));
                            last_err = e;
                        }
                    }
                },
            }
            std::thread::sleep(every);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(state: &str, oid: &str, reviews: Value) -> Value {
        serde_json::json!({"data": {"viewer": {"login": "me", "pullRequests": {"nodes": [{
            "number": 12, "title": "Fix it", "url": "https://github.com/o/r/pull/12",
            "repository": {"nameWithOwner": "o/r"},
            "commits": {"nodes": [{"commit": {"oid": oid, "statusCheckRollup": {"state": state}}}]},
            "reviews": {"nodes": reviews}
        }]}}}})
    }

    #[test]
    fn first_look_is_quiet_then_changes_peek() {
        let (n, s) = changes(&answer("PENDING", "a", serde_json::json!([])), None);
        assert!(n.is_empty());
        let (n, s) = changes(&answer("SUCCESS", "a", serde_json::json!([])), Some(&s));
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].kind, NoteKind::Passed);
        assert_eq!(n[0].place, "o/r #12");
        // Same result again: quiet. A new commit that fails: a peek.
        let (n, s) = changes(&answer("SUCCESS", "a", serde_json::json!([])), Some(&s));
        assert!(n.is_empty());
        let (n, s) = changes(&answer("FAILURE", "b", serde_json::json!([])), Some(&s));
        assert_eq!(n[0].kind, NoteKind::Failed);
        // A review by someone else peeks once; my own doesn't.
        let r = serde_json::json!([{"id": "R1", "state": "APPROVED", "author": {"login": "sam"}}, {"id": "R2", "state": "COMMENTED", "author": {"login": "me"}}]);
        let (n, s) = changes(&answer("FAILURE", "b", r.clone()), Some(&s));
        assert_eq!(n.len(), 1);
        assert_eq!((n[0].kind, n[0].who.as_str()), (NoteKind::Approved, "sam"));
        let (n, _) = changes(&answer("FAILURE", "b", r), Some(&s));
        assert!(n.is_empty());
    }
}
