//! Explicit, user-clicked choices for assistants whose chat cannot use local hooks.
use std::{collections::HashSet, io::Read};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects, WaitForSingleObject};

#[derive(Clone, Deserialize, Serialize)]
pub struct Choice {
    pub id: String,
    pub label: String,
}

pub fn choices(value: &Value) -> Result<Vec<Choice>, String> {
    let choices: Vec<Choice> = serde_json::from_value(value.clone()).map_err(|_| "options must contain id and label strings")?;
    if choices.is_empty() || choices.len() > 5 {
        return Err("provide between one and five choices".into());
    }
    let mut ids = HashSet::new();
    for choice in &choices {
        if choice.id.trim().is_empty() || choice.label.trim().is_empty() || choice.label.chars().count() > 80 || !ids.insert(&choice.id) {
            return Err("choice ids must be unique and nonempty; labels must contain 1–80 characters".into());
        }
    }
    Ok(choices)
}

pub fn answer_name(index: usize) -> String { format!("choice-{index}") }

pub fn run() {
    let result = request();
    println!("{result}");
}

fn request() -> Value {
    let mut input = String::new();
    if std::io::stdin().take(64 * 1024).read_to_string(&mut input).is_err() {
        return json!({"status":"error", "error":"cannot read request"});
    }
    let Ok(payload) = serde_json::from_str::<Value>(&input) else {
        return json!({"status":"error", "error":"request must be JSON"});
    };
    request_value(&payload)
}

pub(crate) fn request_value(payload: &Value) -> Value {
    let question = payload["question"].as_str().unwrap_or("").trim();
    let detail = payload["detail"].as_str().unwrap_or("").trim();
    if question.is_empty() || question.chars().count() > 240 || detail.chars().count() > 600 {
        return json!({"status":"error", "error":"question must contain 1–240 characters and detail at most 600"});
    }
    let choices = match choices(&payload["options"]) {
        Ok(c) => c,
        Err(e) => return json!({"status":"error", "error":e}),
    };
    let pid = std::process::id();
    let id = format!("{pid}-{}", unsafe { windows::Win32::System::SystemInformation::GetTickCount64() });
    let timeout = payload["timeout_seconds"].as_u64().unwrap_or(3600).clamp(10, 3600);
    let session = payload["session"].as_str().unwrap_or(&id);
    // Codex asks through `pocketisle ask`; the Claude Code hook reuses this for its questions.
    let agent = payload["agent"].as_str().unwrap_or("codex");
    let desktop = payload["desktop"].as_bool().unwrap_or(true);
    let cwd = payload["cwd"].as_str().map(str::to_owned).unwrap_or_else(|| std::env::current_dir().unwrap_or_default().display().to_string());
    // Pick-several: the island toggles choices, then signals each picked choice's event
    // followed by a "submit" event (index n + 1).
    let multi = payload["multi"].as_bool().unwrap_or(false);
    let n = choices.len();
    let mut events: Vec<HANDLE> = Vec::new();
    for index in 0..=(if multi { n + 1 } else { n }) {
        let name = HSTRING::from(crate::agent::event_name(&id, &answer_name(index)));
        match unsafe { CreateEventW(None, true, false, &name) } {
            Ok(handle) => events.push(handle),
            Err(_) => {
                for handle in events { unsafe { let _ = CloseHandle(handle); } }
                return json!({"status":"error", "error":"cannot create answer channel"});
            }
        }
    }
    let request = json!({"agent":agent, "event":"IslandQuestion", "session":session,
        "cwd":cwd, "desktop":desktop, "id":id, "pid":pid, "question":question,
        "detail":detail, "choices":choices, "multi":multi, "timeout_seconds":timeout});
    let reply = crate::agent::send(&request.to_string());
    let result = if reply <= crate::agent::DECISION_ACCEPTED {
        json!({"status":"unavailable", "request_id":id})
    } else if multi {
        let seconds = (reply - crate::agent::DECISION_ACCEPTED) as u32;
        // Wait for "open chat" (n) or "submit" (n + 1), then read which choices were set.
        let waited = unsafe { WaitForMultipleObjects(&events[n..], false, seconds.saturating_mul(1000)) };
        match waited.0.wrapping_sub(WAIT_OBJECT_0.0) {
            0 => json!({"status":"open_chat", "request_id":id}),
            1 => {
                let picked: Vec<&Choice> = choices
                    .iter()
                    .zip(&events)
                    .filter(|(_, h)| unsafe { WaitForSingleObject(**h, 0) } == WAIT_OBJECT_0)
                    .map(|(c, _)| c)
                    .collect();
                if picked.is_empty() {
                    json!({"status":"open_chat", "request_id":id})
                } else {
                    json!({"status":"answered", "request_id":id, "choices":picked})
                }
            }
            _ => json!({"status":"timeout", "request_id":id}),
        }
    } else {
        let seconds = (reply - crate::agent::DECISION_ACCEPTED) as u32;
        let waited = unsafe { WaitForMultipleObjects(&events, false, seconds.saturating_mul(1000)) };
        let index = waited.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
        if let Some(choice) = choices.get(index) {
            json!({"status":"answered", "request_id":id, "choice":choice})
        } else if index == choices.len() {
            json!({"status":"open_chat", "request_id":id})
        } else {
            json!({"status":"timeout", "request_id":id})
        }
    };
    for handle in events { unsafe { let _ = CloseHandle(handle); } }
    crate::agent::send(&json!({"agent":agent, "event":"QuestionDone", "session":session, "id":id}).to_string());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_or_unrenderable_choices() {
        assert!(choices(&json!([])).is_err());
        assert!(choices(&json!([{"id":"a","label":"Yes"},{"id":"a","label":"No"}])).is_err());
        assert!(choices(&json!([{"id":"a","label":"x".repeat(81)}])).is_err());
        assert!(choices(&json!([{"id":"a","label":"Yes"},{"id":"b","label":"No"}])).is_ok());
    }
}
