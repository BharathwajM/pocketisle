//! `pocketisle.exe hook <agent>`: a tiny client that Claude Code hooks / Codex `notify` run.
//! Codex hooks send the same kind of payload on stdin. It turns the hook payload into a short JSON message and hands it to the running island
//! with WM_COPYDATA. It never fails the hook (always exits 0).
//!
//! Most events return at once. A Claude Code `PermissionRequest` waits for an answer from
//! the island (Allow / Deny / "ask in Claude") on three named events, then prints the
//! hook's decision. If the island isn't running, is hidden, or nobody answers in time, it
//! prints nothing, and Claude Code asks in its own window as usual.

use std::io::{Read, Write};
use std::time::Duration;

use serde_json::{json, Value};
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, WAIT_OBJECT_0, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32NextW, Process32FirstW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_COPYDATA};

pub const AGENT_MAGIC: usize = 0xA7011;

/// The island's WM_COPYDATA reply when it shows a decision: this plus the seconds to wait.
pub const DECISION_ACCEPTED: isize = 1000;

/// The three answers, in the order of the named events.
pub const ANSWERS: [&str; 3] = ["allow", "deny", "pass"];

pub fn event_name(id: &str, answer: &str) -> String {
    format!("Local\\PocketIsleDecision-{id}-{answer}")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Working,
    Waiting,
    Done,
}

/// Read stdin, but never hang if the caller left it open.
fn read_stdin() -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::stdin().take(4 * 1024 * 1024).read_to_string(&mut s);
        let _ = tx.send(s);
    });
    rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default()
}

/// The text of the last assistant message in a Claude Code transcript (JSONL).
fn last_assistant_text(path: &str) -> String {
    let Ok(bytes) = std::fs::read(path) else { return String::new() };
    let tail = &bytes[bytes.len().saturating_sub(768 * 1024)..];
    let text = String::from_utf8_lossy(tail);
    for line in text.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v["type"] != "assistant" {
            continue;
        }
        if let Some(items) = v["message"]["content"].as_array() {
            let joined: Vec<&str> = items
                .iter()
                .filter(|i| i["type"] == "text")
                .filter_map(|i| i["text"].as_str())
                .collect();
            if !joined.is_empty() {
                return joined.join(" ");
            }
        }
    }
    String::new()
}

fn file_name(p: &str) -> String {
    p.rsplit(['\\', '/']).find(|s| !s.is_empty()).unwrap_or(p).to_string()
}

/// Files named in a Codex `apply_patch` body ("*** Update File: src/app.rs").
fn patch_files(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|l| {
            ["*** Update File: ", "*** Add File: ", "*** Delete File: "]
                .iter()
                .find_map(|p| l.strip_prefix(p))
                .map(|f| f.trim().to_string())
        })
        .collect()
}

fn patch_summary(patch: &str) -> String {
    let files = patch_files(patch);
    match files.len() {
        0 => "a file".to_string(),
        1 => file_name(&files[0]),
        n => format!("{n} files"),
    }
}

/// A shell command as text (Codex may send it as an argv array).
fn command_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        Value::Array(a) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    }
}

fn host(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or(rest).to_string()
}

/// "mcp__claude-in-chrome__navigate" -> ("claude in chrome", "navigate").
fn mcp_parts(tool: &str) -> Option<(String, String)> {
    let rest = tool.strip_prefix("mcp__")?;
    let (server, name) = rest.split_once("__")?;
    let server = if server.len() > 24 && server.contains('-') && server.chars().filter(|c| *c == '-').count() >= 4 {
        // A connector id (a UUID); the tool name says enough.
        String::new()
    } else {
        server.replace(['-', '_'], " ")
    };
    Some((server, name.replace('_', " ")))
}

/// What a tool call is doing, for the pill and the Agents tab ("Running cargo build").
pub fn activity(tool: &str, input: &Value) -> String {
    let s = |k: &str| input[k].as_str().unwrap_or("").trim().to_string();
    let first_line = |t: String| t.lines().next().unwrap_or("").trim().to_string();
    let text = match tool {
        "Bash" | "PowerShell" | "shell" | "exec_command" => {
            let d = s("description");
            if d.is_empty() { format!("Running {}", first_line(command_text(&input["command"]))) } else { d }
        }
        "apply_patch" => format!("Editing {}", patch_summary(&s("command"))),
        "update_plan" => "Updating its plan".into(),
        "spawn_agent" => "Starting a helper agent".into(),
        "Read" | "NotebookRead" => format!("Reading {}", file_name(&s("file_path"))),
        "Edit" | "MultiEdit" | "NotebookEdit" => format!("Editing {}", file_name(&[s("file_path"), s("notebook_path")].concat())),
        "Write" => format!("Writing {}", file_name(&s("file_path"))),
        "Grep" => format!("Searching for “{}”", s("pattern")),
        "Glob" => format!("Finding {}", s("pattern")),
        "WebFetch" => format!("Reading {}", host(&s("url"))),
        "WebSearch" => format!("Searching the web for “{}”", s("query")),
        "Agent" | "Task" => {
            let d = s("description");
            if d.is_empty() { "Running a helper agent".into() } else { format!("Agent: {d}") }
        }
        "Skill" => format!("Using the {} skill", s("skill")),
        "AskUserQuestion" => "Asking you a question".into(),
        "ExitPlanMode" => "Showing you a plan".into(),
        "ToolSearch" => "Loading tools".into(),
        "TodoWrite" => "Updating its to-do list".into(),
        t if t.starts_with("mcp__") => match mcp_parts(t) {
            Some((server, name)) if server.is_empty() => format!("Using {name}"),
            Some((server, name)) => format!("{server}: {name}"),
            None => t.to_string(),
        },
        t => format!("Using {t}"),
    };
    crate::util::clip(&text, 120)
}

/// What a permission prompt asks, as (question, detail): ("Run a command?", "cargo build").
pub fn question(tool: &str, input: &Value) -> (String, String) {
    let s = |k: &str| input[k].as_str().unwrap_or("").trim().to_string();
    let (q, d) = match tool {
        "Bash" | "PowerShell" | "shell" | "exec_command" => ("Run this command?".to_string(), command_text(&input["command"])),
        "apply_patch" => {
            let files = patch_files(&s("command"));
            let q = if files.len() > 1 { "Edit these files?" } else { "Edit this file?" };
            (q.to_string(), if files.is_empty() { s("command") } else { files.join(", ") })
        }
        "Edit" | "MultiEdit" | "NotebookEdit" => ("Edit this file?".to_string(), [s("file_path"), s("notebook_path")].concat()),
        "Write" => ("Create or overwrite this file?".to_string(), s("file_path")),
        "Read" => ("Read this file?".to_string(), s("file_path")),
        "WebFetch" => ("Open this page?".to_string(), s("url")),
        "WebSearch" => ("Search the web?".to_string(), s("query")),
        "Agent" | "Task" => ("Start a helper agent?".to_string(), s("description")),
        t if t.starts_with("mcp__") => {
            let name = match mcp_parts(t) {
                Some((server, name)) if server.is_empty() => name,
                Some((server, name)) => format!("{server}: {name}"),
                None => t.to_string(),
            };
            (format!("Use {name}?"), compact_json(input))
        }
        t => (format!("Use {t}?"), compact_json(input)),
    };
    (q, crate::util::clip(&d, 400))
}

fn compact_json(v: &Value) -> String {
    match v.as_object() {
        Some(o) => o
            .iter()
            .filter_map(|(k, v)| {
                let val = match v {
                    Value::String(s) => s.clone(),
                    Value::Null => return None,
                    other => other.to_string(),
                };
                Some(format!("{k}: {val}"))
            })
            .collect::<Vec<_>>()
            .join(" · "),
        None => v.to_string(),
    }
}

pub fn run_hook_cli(args: &[String]) {
    let agent = args.first().map(String::as_str).unwrap_or("claude").to_string();
    // Codex's `notify` passes JSON as the last argument; hooks (Claude Code's and Codex's)
    // send it on stdin.
    let payload: Value = match args.last().filter(|a| a.starts_with('{')) {
        Some(a) => serde_json::from_str(a).unwrap_or(Value::Null),
        None => serde_json::from_str(&read_stdin()).unwrap_or(Value::Null),
    };
    let s = |k: &str| payload[k].as_str().unwrap_or("").to_string();

    let event = [s("hook_event_name"), s("type"), args.get(1).cloned().unwrap_or_default()]
        .into_iter()
        .find(|e| !e.is_empty() && !e.starts_with('{'))
        .unwrap_or_default();
    let session = [s("session_id"), s("thread-id"), s("turn-id")]
        .into_iter()
        .find(|e| !e.is_empty())
        .unwrap_or_else(|| "default".into());
    let mut message = [s("message"), s("last-assistant-message"), s("last_assistant_message")]
        .into_iter()
        .find(|e| !e.is_empty())
        .unwrap_or_default();
    if message.is_empty() && event == "Stop" {
        message = last_assistant_text(&s("transcript_path"));
    }
    let tool = s("tool_name");
    let input = &payload["tool_input"];
    // Sessions in the Claude / Codex desktop app vs. a terminal (for where "open" goes).
    let desktop = match agent.as_str() {
        "claude" => std::env::var("CLAUDE_CODE_ENTRYPOINT").map(|e| e.contains("desktop")).unwrap_or(false),
        _ => started_by("chatgpt.exe"),
    };

    let mut out = json!({
        "agent": agent,
        "event": event,
        "session": session,
        "cwd": s("cwd"),
        "message": crate::util::clip(&message, 300),
        "desktop": desktop,
    });
    match event.as_str() {
        "PreToolUse" => out["activity"] = json!(activity(&tool, input)),
        "PostToolUse" | "UserPromptSubmit" => out["activity"] = json!("Thinking"),
        _ => {}
    }

    // Claude's questions and plans become choice cards on the island. Claude Code shows them
    // without a PermissionRequest, so they're answered from PreToolUse.
    if event == "PreToolUse" && agent == "claude" && matches!(tool.as_str(), "AskUserQuestion" | "ExitPlanMode") {
        dump(&format!("{tool} {input}"));
        send(&out.to_string());
        ask_claude(&out, &tool, input);
        return;
    }
    // Questions and plans have their own answers (Codex's go through `pocketisle ask`, see AGENTS.md).
    if event == "PermissionRequest" && !matches!(tool.as_str(), "AskUserQuestion" | "ExitPlanMode" | "request_user_input") {
        decide(out, &tool, input);
        return;
    }
    send(&out.to_string());
}

/// Show Claude's `AskUserQuestion` (one card per question) or `ExitPlanMode` (approve / keep
/// planning) on the island and print the answer as the hook's decision. Prints nothing when
/// the island can't take it or you choose "Open chat instead", so Claude asks as usual.
fn ask_claude(out: &Value, tool: &str, input: &Value) {
    // Questions wait as long as permission prompts do.
    let secs = crate::config::Config::load().decision_timeout_seconds.clamp(10, 3600);
    // The picked option indexes (one, or several for a pick-several question); None = Claude asks.
    let ask = |question: &str, detail: &str, labels: &[String], multi: bool| -> Option<Vec<usize>> {
        let options: Vec<Value> = labels
            .iter()
            .enumerate()
            .map(|(i, l)| json!({ "id": format!("option-{}", i + 1), "label": crate::util::clip(l, 80) }))
            .collect();
        let request = json!({
            "agent": out["agent"], "session": out["session"], "cwd": out["cwd"], "desktop": out["desktop"],
            "question": crate::util::clip(question, 240), "detail": detail, "options": options,
            "multi": multi, "timeout_seconds": secs,
        });
        let reply = crate::ask::request_value(&request);
        dump(&format!("ask {request}\n  -> {reply}"));
        if reply["status"] != "answered" {
            return None;
        }
        let picked: Vec<Value> = if multi { reply["choices"].as_array().cloned()? } else { vec![reply["choice"].clone()] };
        let idx: Vec<usize> = picked
            .iter()
            .filter_map(|c| c["id"].as_str()?.strip_prefix("option-")?.parse::<usize>().ok()?.checked_sub(1))
            .filter(|i| *i < labels.len())
            .collect();
        (!idx.is_empty()).then_some(idx)
    };
    let card = |question: &str, detail: &str, labels: &[String]| ask(question, detail, labels, false).map(|v| v[0]);

    let decision = match tool {
        "AskUserQuestion" => {
            let Some(questions) = input["questions"].as_array().filter(|q| !q.is_empty()) else { return };
            let mut answers = serde_json::Map::new();
            for q in questions {
                let text = q["question"].as_str().unwrap_or("").trim();
                let options = q["options"].as_array().cloned().unwrap_or_default();
                let labels: Vec<String> = options.iter().filter_map(|o| o["label"].as_str().map(str::to_string)).collect();
                if text.is_empty() || labels.is_empty() {
                    return;
                }
                // Pick-several questions get toggles and a Done button; the answer lists the
                // picked labels the way Claude's own picker does.
                let multi = q["multiSelect"].as_bool().unwrap_or(false);
                let Some(picked) = ask(text, &option_notes(&options), &labels, multi) else { return };
                let answer: Vec<&str> = picked.iter().map(|i| labels[*i].as_str()).collect();
                answers.insert(text.to_string(), json!(answer.join(", ")));
            }
            let mut updated = input.clone();
            updated["answers"] = Value::Object(answers);
            json!({ "permissionDecision": "allow", "permissionDecisionReason": "Answered on Pocket Isle", "updatedInput": updated })
        }
        "ExitPlanMode" => {
            let plan = crate::util::clip(&plain(input["plan"].as_str().unwrap_or("")), 500);
            let labels = ["Approve plan".to_string(), "Keep planning".to_string()];
            match card("Claude's plan is ready. Start building?", &plan, &labels) {
                Some(0) => json!({ "permissionDecision": "allow", "permissionDecisionReason": "Plan approved on Pocket Isle" }),
                Some(_) => json!({ "permissionDecision": "deny", "permissionDecisionReason": "The user chose \"Keep planning\" on Pocket Isle. Ask what they'd like changed in the plan." }),
                None => return,
            }
        }
        _ => return,
    };
    let mut specific = decision;
    specific["hookEventName"] = json!("PreToolUse");
    println!("{}", json!({ "hookSpecificOutput": specific }));
}

/// With `POCKETISLE_HOOK_DUMP` set to a file path, append a line about the hook's work there.
fn dump(line: &str) {
    if let Some(path) = std::env::var_os("POCKETISLE_HOOK_DUMP") {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Markdown as plain text for the card: no heading marks, emphasis or code ticks.
fn plain(md: &str) -> String {
    md.lines()
        .map(|l| l.trim().trim_start_matches('#').trim().replace("**", "").replace('`', ""))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The options' descriptions, one short line each, for the card's detail box.
fn option_notes(options: &[Value]) -> String {
    let lines: Vec<String> = options
        .iter()
        .filter_map(|o| {
            let label = o["label"].as_str()?;
            // Skip notes that only repeat the label ("Apple", "Select Apple").
            let note = o["description"].as_str().map(str::trim).filter(|d| {
                let d = d.to_lowercase();
                let l = label.trim().to_lowercase();
                !d.is_empty() && d != l && d != format!("select {l}") && d != format!("choose {l}")
            })?;
            Some(format!("{}: {}", crate::util::clip(label, 30), crate::util::clip(note, 90)))
        })
        .collect();
    crate::util::clip(&lines.join(" · "), 420)
}

/// Is a process with this exe name among our ancestors? (The Codex app is ChatGPT.exe.)
fn started_by(exe: &str) -> bool {
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return false };
        let mut parents = std::collections::HashMap::new();
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
                let name = String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase();
                parents.insert(e.th32ProcessID, (e.th32ParentProcessID, name));
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        let mut pid = std::process::id();
        for _ in 0..12 {
            let Some((parent, _)) = parents.get(&pid) else { return false };
            match parents.get(parent) {
                Some((_, name)) if name == exe => return true,
                Some(_) => pid = *parent,
                None => return false,
            }
        }
        false
    }
}

/// Show a permission prompt on the island and print its answer as the hook's decision.
fn decide(mut out: Value, tool: &str, input: &Value) {
    let pid = std::process::id();
    let id = format!("{pid}-{}", unsafe { windows::Win32::System::SystemInformation::GetTickCount64() });
    let (q, detail) = question(tool, input);
    out["id"] = json!(id);
    out["pid"] = json!(pid);
    out["question"] = json!(q);
    out["detail"] = json!(detail);
    out["activity"] = json!(activity(tool, input));

    // Create the answer events before the island can see the request.
    let events: Vec<HANDLE> = ANSWERS
        .iter()
        .filter_map(|a| unsafe { CreateEventW(None, true, false, &HSTRING::from(event_name(&id, a))).ok() })
        .collect();
    if events.len() != ANSWERS.len() {
        for h in events {
            unsafe {
                let _ = CloseHandle(h);
            }
        }
        send(&out.to_string());
        return;
    }

    let reply = send(&out.to_string());
    let mut answer = None;
    if reply > DECISION_ACCEPTED {
        let secs = (reply - DECISION_ACCEPTED) as u32;
        let r = unsafe { WaitForMultipleObjects(&events, false, secs.saturating_mul(1000)) };
        let i = r.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
        if i < ANSWERS.len() {
            answer = Some(ANSWERS[i]);
        } else {
            // Timed out: take the prompt off the island.
            let done = json!({ "agent": out["agent"], "event": "DecisionDone", "session": out["session"], "id": id, "cwd": out["cwd"], "desktop": out["desktop"] });
            send(&done.to_string());
        }
    }
    for h in events {
        unsafe {
            let _ = CloseHandle(h);
        }
    }

    let decision = match answer {
        Some("allow") => json!({ "behavior": "allow" }),
        Some("deny") => json!({ "behavior": "deny", "message": "Denied from Pocket Isle" }),
        _ => return, // No answer: Claude Code asks in its own window.
    };
    let o = json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } });
    println!("{o}");
}

/// Hand a message to the island. Returns its reply (0 if it isn't running).
pub(crate) fn send(msg: &str) -> isize {
    unsafe {
        let Ok(hwnd) = FindWindowW(w!("PocketIsle"), None) else { return 0 };
        let cds = COPYDATASTRUCT {
            dwData: AGENT_MAGIC,
            cbData: msg.len() as u32,
            lpData: msg.as_ptr() as *mut _,
        };
        let mut result = 0usize;
        let ok = SendMessageTimeoutW(
            hwnd,
            WM_COPYDATA,
            WPARAM(0),
            LPARAM(&cds as *const _ as isize),
            SMTO_ABORTIFHUNG,
            1500,
            Some(&mut result),
        );
        if ok.0 == 0 {
            0
        } else {
            result as isize
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn question_cards_read_cleanly() {
        let options = vec![
            json!({ "label": "Apple", "description": "Apple" }),
            json!({ "label": "Banana", "description": "Select Banana" }),
            json!({ "label": "Cherry", "description": "Sour and red" }),
        ];
        assert_eq!(option_notes(&options), "Cherry: Sour and red");
        assert_eq!(plain("## Plan\n1. **Build** `pocketisle`\n\n2. Test"), "Plan 1. Build pocketisle 2. Test");
    }

    #[test]
    fn describes_tools() {
        let v = json!({ "command": "cargo build --release\necho done" });
        assert_eq!(activity("Bash", &v), "Running cargo build --release");
        let v = json!({ "command": "ls", "description": "List files" });
        assert_eq!(activity("Bash", &v), "List files");
        let v = json!({ "file_path": "C:\\x\\src\\app.rs" });
        assert_eq!(activity("Edit", &v), "Editing app.rs");
        let v = json!({ "url": "https://docs.rs/windows/latest" });
        assert_eq!(activity("WebFetch", &v), "Reading docs.rs");
        assert_eq!(activity("mcp__claude-in-chrome__navigate", &json!({})), "claude in chrome: navigate");
        assert_eq!(activity("mcp__1a59c906-04da-521d-bda7-7f71b9f9e01c__batch", &json!({})), "Using batch");
        let (q, d) = question("Bash", &json!({ "command": "rm -rf build" }));
        assert_eq!((q.as_str(), d.as_str()), ("Run this command?", "rm -rf build"));
        let patch = "*** Begin Patch
*** Update File: src/app.rs
@@
-a
+b
*** End Patch";
        assert_eq!(activity("apply_patch", &json!({ "command": patch })), "Editing app.rs");
        assert_eq!(question("apply_patch", &json!({ "command": patch })).1, "src/app.rs");
        assert_eq!(activity("Bash", &json!({ "command": ["git", "status"] })), "Running git status");
    }
}
