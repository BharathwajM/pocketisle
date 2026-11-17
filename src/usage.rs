//! AI coding plan limits (what Codenotch tracked): Claude's 5-hour / weekly usage and
//! Codex's 5-hour / weekly rate limits, read on a background thread.
//!
//! - Claude: `GET https://api.anthropic.com/api/oauth/usage` with Claude Code's own sign-in
//!   (`~/.claude/.credentials.json`). The token is only read, never refreshed, written or logged;
//!   when it has expired, Claude Code's own `claude doctor` is run, which renews it.
//! - Codex: asks the Codex CLI itself (`codex app-server --stdio`, JSON-RPC
//!   `account/rateLimits/read`), so Pocket Isle never touches Codex's credential files.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Networking::WinHttp::*;

use crate::events::{self, Event};
use crate::util;

#[derive(Clone, PartialEq)]
pub struct Limit {
    /// Short column label: "5h", "Week", "Opus".
    pub short: String,
    /// For cards: "5-hour limit", "weekly limit", "Opus weekly limit".
    pub long: String,
    /// Fraction used, 0..1 (can exceed 1 when over).
    pub used: f32,
    /// Unix seconds.
    pub resets_at: Option<i64>,
}

#[derive(Clone)]
pub struct Usage {
    pub provider: &'static str,
    pub limits: Vec<Limit>,
    /// Plan etc., e.g. "Pro" or "Plus · 3 resets".
    pub note: String,
    /// Set when the last fetch failed; `limits` then holds the last good values (if any).
    pub error: Option<String>,
}

#[derive(Clone, PartialEq)]
pub struct UsageCfg {
    pub claude: bool,
    pub codex: bool,
    pub codex_path: Option<String>,
    pub refresh_minutes: u32,
}

pub enum UsageCmd {
    /// A Claude turn just finished: refresh Claude soon (at most every few minutes).
    ClaudeActivity,
    Config(UsageCfg),
}

pub fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// "12m", "2h 14m", "3d 4h".
pub fn countdown(secs: i64) -> String {
    let s = secs.max(0);
    if s < 3600 {
        format!("{}m", (s + 59) / 60)
    } else if s < 86400 {
        format!("{}h {}m", s / 3600, (s % 3600) / 60)
    } else {
        format!("{}d {}h", s / 86400, (s % 86400) / 3600)
    }
}

pub fn spawn(cfg: UsageCfg) -> Sender<UsageCmd> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("usage".into())
        .spawn(move || run(cfg, rx))
        .ok();
    tx
}

struct Source {
    provider: &'static str,
    next: Instant,
    last_fetch: Option<Instant>,
    last_good: Option<Usage>,
    /// Rate limited: nothing may ask again before this, and how long the last wait was
    /// (it doubles while the limit lasts).
    blocked_until: Option<Instant>,
    backoff: Duration,
}

/// Activity refreshes Claude's usage at most this often.
const ACTIVITY_GAP: Duration = Duration::from_secs(3 * 60);
/// A restart reuses a reading this fresh instead of asking again.
const FRESH: Duration = Duration::from_secs(2 * 60);
const BACKOFF_FIRST: Duration = Duration::from_secs(15 * 60);
const BACKOFF_MAX: Duration = Duration::from_secs(60 * 60);

impl Source {
    /// Starts from what was saved last time: the island shows the last reading at once, and a
    /// restart neither re-asks for a fresh one nor walks through a rate limit's wait.
    fn new(provider: &'static str) -> Self {
        let mut src = Self { provider, next: Instant::now(), last_fetch: None, last_good: None, blocked_until: None, backoff: BACKOFF_FIRST };
        if let Some((u, fetched, blocked)) = cache_load(provider) {
            let (now, now_i) = (now_unix(), Instant::now());
            let ago = Duration::from_secs((now - fetched).max(0) as u64);
            if ago < FRESH {
                src.next = now_i + (FRESH - ago);
            }
            if blocked > now {
                let b = now_i + Duration::from_secs((blocked - now) as u64);
                src.blocked_until = Some(b);
                src.next = src.next.max(b);
            }
            let mut shown = u.clone();
            shown.error = None;
            events::send(Event::Usage(shown));
            src.last_good = Some(u);
        }
        src
    }

    /// The soonest it may ask (respecting a rate limit).
    fn not_before(&self, t: Instant) -> Instant {
        self.blocked_until.map_or(t, |b| t.max(b))
    }
}

// ---------- the saved last reading ----------

fn cache_path() -> PathBuf {
    util::config_dir().join("usage-cache.json")
}

fn cache_load(provider: &'static str) -> Option<(Usage, i64, i64)> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(cache_path()).ok()?).ok()?;
    let e = &v[provider];
    let limits: Vec<Limit> = e["limits"]
        .as_array()?
        .iter()
        .filter_map(|l| {
            // A limit that has reset since it was saved is back to nothing used.
            let resets_at = l["resets_at"].as_i64();
            let reset = resets_at.is_some_and(|r| r <= now_unix());
            Some(Limit {
                short: l["short"].as_str()?.to_string(),
                long: l["long"].as_str()?.to_string(),
                used: if reset { 0.0 } else { l["used"].as_f64()? as f32 },
                resets_at: if reset { None } else { resets_at },
            })
        })
        .collect();
    if limits.is_empty() {
        return None;
    }
    let note = e["note"].as_str().unwrap_or("").to_string();
    Some((Usage { provider, limits, note, error: None }, e["fetched_at"].as_i64().unwrap_or(0), e["blocked_until"].as_i64().unwrap_or(0)))
}

fn cache_save(src: &Source, fetched_at: Option<i64>) {
    let Some(u) = &src.last_good else { return };
    let path = cache_path();
    let mut all: Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}));
    let prev_fetched = all[src.provider]["fetched_at"].as_i64().unwrap_or(0);
    let now = now_unix();
    let blocked = src.blocked_until.map(|b| now + b.saturating_duration_since(Instant::now()).as_secs() as i64).unwrap_or(0);
    let limits: Vec<Value> = u
        .limits
        .iter()
        .map(|l| serde_json::json!({ "short": l.short, "long": l.long, "used": l.used, "resets_at": l.resets_at }))
        .collect();
    all[src.provider] = serde_json::json!({
        "limits": limits,
        "note": u.note,
        "fetched_at": fetched_at.unwrap_or(prev_fetched),
        "blocked_until": blocked,
    });
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&all).unwrap_or_default());
}

fn run(mut cfg: UsageCfg, rx: Receiver<UsageCmd>) {
    let mut claude = Source::new("Claude");
    let mut codex = Source::new("Codex");
    loop {
        let interval = Duration::from_secs(cfg.refresh_minutes.clamp(1, 120) as u64 * 60);
        let now = Instant::now();
        if cfg.claude && now >= claude.next {
            claude.next = poll(&mut claude, fetch_claude(), interval);
        }
        if cfg.codex && now >= codex.next {
            codex.next = poll(&mut codex, fetch_codex(cfg.codex_path.as_deref()), interval);
        }
        let mut wake = now + Duration::from_secs(3600);
        if cfg.claude {
            wake = wake.min(claude.next);
        }
        if cfg.codex {
            wake = wake.min(codex.next);
        }
        match rx.recv_timeout(wake.saturating_duration_since(Instant::now())) {
            Ok(UsageCmd::ClaudeActivity) => {
                let soon = claude.last_fetch.map_or(Instant::now(), |t| t + ACTIVITY_GAP);
                claude.next = claude.next.min(claude.not_before(soon.max(Instant::now() + Duration::from_secs(5))));
            }
            Ok(UsageCmd::Config(c)) => {
                if c != cfg {
                    cfg = c;
                    claude.next = claude.not_before(Instant::now());
                    codex.next = codex.not_before(Instant::now());
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

enum Fetch {
    Ok(Usage),
    /// The provider isn't set up on this PC (no sign-in / CLI): show nothing.
    Absent,
    Err { provider: &'static str, msg: String, retry: Option<Duration> },
    /// HTTP 429: wait (longer each time it happens again) before asking again.
    Limited { provider: &'static str },
}

/// Send the result to the UI and return when to fetch next.
fn poll(src: &mut Source, f: Fetch, interval: Duration) -> Instant {
    let now = Instant::now();
    src.last_fetch = Some(now);
    let mut next = now + interval;
    match f {
        Fetch::Ok(u) => {
            // Refetch just after the soonest reset so the island can say the limit is back.
            let t = now_unix();
            if let Some(r) = u.limits.iter().filter(|l| l.used > 0.0).filter_map(|l| l.resets_at).filter(|&r| r > t).min() {
                next = next.min(now + Duration::from_secs((r - t) as u64 + 20));
            }
            src.last_good = Some(u.clone());
            src.blocked_until = None;
            src.backoff = BACKOFF_FIRST;
            cache_save(src, Some(now_unix()));
            events::send(Event::Usage(u));
        }
        Fetch::Limited { provider } => {
            let wait = src.backoff;
            util::log(&format!("{provider} usage: rate limited; asking again in {} min", wait.as_secs() / 60));
            src.blocked_until = Some(now + wait);
            src.backoff = (src.backoff * 2).min(BACKOFF_MAX);
            next = now + wait;
            cache_save(src, None);
            // Keep showing the last reading meanwhile.
            if let Some(mut u) = src.last_good.clone() {
                u.error = Some("rate limited".into());
                events::send(Event::Usage(u));
            }
        }
        Fetch::Absent => {
            next = now + Duration::from_secs(30 * 60);
        }
        Fetch::Err { provider, msg, retry } => {
            util::log(&format!("{provider} usage: {msg}"));
            if let Some(r) = retry {
                next = now + r;
            }
            let mut u = src.last_good.clone().unwrap_or(Usage { provider, limits: Vec::new(), note: String::new(), error: None });
            u.error = Some(msg);
            events::send(Event::Usage(u));
        }
    }
    next
}

// ---------- Claude ----------

fn claude_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| util::home().join(".claude"))
}

fn plan_name(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Claude Code's saved sign-in: (access token, plan, expiry in ms).
fn read_sign_in() -> Option<(String, String, Option<i64>)> {
    let text = std::fs::read_to_string(claude_dir().join(".credentials.json")).ok()?;
    let cred = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).ok()?;
    let oauth = &cred["claudeAiOauth"];
    let token = oauth["accessToken"].as_str()?.to_string();
    let note = oauth["subscriptionType"].as_str().map(plan_name).unwrap_or_default();
    Some((token, note, oauth["expiresAt"].as_i64()))
}

/// The saved sign-in has expired: let Claude Code's own command-line tool renew it.
/// `claude doctor` does (it signs in to check the account, using its refresh token, and saves
/// the new sign-in back); `claude auth status` only reads it, so it never did. The sign-in
/// lasts about 8 hours and the Claude desktop app keeps its own elsewhere, so without the CLI
/// running the file just goes stale. We never refresh or write tokens ourselves. At most
/// every 10 minutes; true if it ran.
fn renew_sign_in() -> bool {
    use std::os::windows::process::CommandExt;
    static LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    {
        let Ok(mut last) = LAST.lock() else { return false };
        if last.is_some_and(|t| t.elapsed() < Duration::from_secs(10 * 60)) {
            return false;
        }
        *last = Some(Instant::now());
    }
    let local = util::home().join(".local").join("bin").join("claude.exe");
    let exe = if local.exists() { local } else { PathBuf::from("claude") };
    let child = Command::new(exe)
        .arg("doctor")
        // It reads settings in the folder it runs in: use Pocket Isle's own.
        .current_dir(util::config_dir())
        .env_remove("CLAUDECODE")
        .env_remove("ANTHROPIC_BASE_URL")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn();
    let Ok(mut child) = child else {
        util::log("Claude usage: sign-in expired and the claude command wasn't found to renew it");
        return false;
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < Duration::from_secs(60) => std::thread::sleep(Duration::from_millis(200)),
            _ => {
                let _ = child.kill();
                break;
            }
        }
    }
    util::log("Claude usage: sign-in had expired; asked Claude Code to renew it");
    true
}

fn fetch_claude() -> Fetch {
    const P: &str = "Claude";
    let Some(mut cred) = read_sign_in() else { return Fetch::Absent };
    let expired = |c: &(String, String, Option<i64>)| c.2.is_some_and(|e| e / 1000 < now_unix() + 60);
    if expired(&cred) && renew_sign_in() {
        if let Some(c) = read_sign_in() {
            cred = c;
        }
    }
    if expired(&cred) {
        return Fetch::Err { provider: P, msg: "sign-in expired; run `claude` once to renew".into(), retry: Some(Duration::from_secs(10 * 60)) };
    }
    let (token, note, _) = cred;
    let headers = format!("Authorization: Bearer {token}\r\nanthropic-beta: oauth-2025-04-20\r\nAccept: application/json\r\n");
    drop(token);
    let res = https_get("api.anthropic.com", "/api/oauth/usage", headers);
    let (status, body) = match res {
        Ok(r) => r,
        Err(e) => return Fetch::Err { provider: P, msg: e, retry: Some(Duration::from_secs(120)) },
    };
    match status {
        200 => {}
        401 | 403 => {
            // Revoked or rotated early: have it renewed, then try again soon.
            let renewed = renew_sign_in();
            let retry = Duration::from_secs(if renewed { 20 } else { 10 * 60 });
            return Fetch::Err { provider: P, msg: "sign-in rejected; renewing it".into(), retry: Some(retry) };
        }
        429 => return Fetch::Limited { provider: P },
        s => return Fetch::Err { provider: P, msg: format!("HTTP {s}"), retry: Some(Duration::from_secs(5 * 60)) },
    }
    let Ok(v) = serde_json::from_slice::<Value>(&body) else {
        return Fetch::Err { provider: P, msg: "unreadable response".into(), retry: None };
    };
    let mut limits = Vec::new();
    let window = |v: &Value, short: &str, long: &str| -> Option<Limit> {
        let w = v.as_object()?;
        let used = w.get("utilization")?.as_f64()? as f32 / 100.0;
        Some(Limit { short: short.into(), long: long.into(), used, resets_at: w.get("resets_at").and_then(parse_time) })
    };
    limits.extend(window(&v["five_hour"], "5h", "5-hour limit"));
    limits.extend(window(&v["seven_day"], "Week", "weekly limit"));
    // Per-model weekly caps (Opus, Sonnet, ...) only matter once they are in use.
    if let Some(arr) = v["limits"].as_array() {
        for e in arr {
            if e["kind"].as_str() != Some("weekly_scoped") {
                continue;
            }
            let (Some(name), Some(pct)) = (e["scope"]["model"]["display_name"].as_str(), e["percent"].as_f64()) else { continue };
            if pct <= 0.0 {
                continue;
            }
            limits.push(Limit {
                short: util::clip(name, 10),
                long: format!("{name} weekly limit"),
                used: pct as f32 / 100.0,
                resets_at: e.get("resets_at").and_then(parse_time),
            });
        }
    }
    if limits.is_empty() {
        return Fetch::Err { provider: P, msg: "no limits in response".into(), retry: None };
    }
    Fetch::Ok(Usage { provider: P, limits, note, error: None })
}

/// One HTTPS GET with WinHTTP (system proxy settings). Returns (status, body).
pub fn https_get(host: &str, path: &str, headers: String) -> Result<(u32, Vec<u8>), String> {
    struct H(*mut core::ffi::c_void);
    impl Drop for H {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    let _ = WinHttpCloseHandle(self.0);
                }
            }
        }
    }
    let mut hdr: Vec<u16> = headers.encode_utf16().collect();
    drop(headers);
    let result = unsafe {
        (|| {
            let s = H(WinHttpOpen(w!("pocketisle/0.2"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
            if s.0.is_null() {
                return Err("network unavailable".to_string());
            }
            let _ = WinHttpSetTimeouts(s.0, 10_000, 10_000, 10_000, 15_000);
            let c = H(WinHttpConnect(s.0, &HSTRING::from(host), INTERNET_DEFAULT_HTTPS_PORT, 0));
            if c.0.is_null() {
                return Err("can't reach server".to_string());
            }
            let r = H(WinHttpOpenRequest(c.0, w!("GET"), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE));
            if r.0.is_null() {
                return Err("request failed".to_string());
            }
            WinHttpSendRequest(r.0, Some(&hdr), None, 0, 0, 0).map_err(|_| "offline or server unreachable".to_string())?;
            WinHttpReceiveResponse(r.0, std::ptr::null_mut()).map_err(|_| "no response".to_string())?;
            let mut status = 0u32;
            let mut len = 4u32;
            WinHttpQueryHeaders(
                r.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some(&mut status as *mut u32 as *mut _),
                &mut len,
                std::ptr::null_mut(),
            )
            .map_err(|_| "bad response".to_string())?;
            let mut body = Vec::new();
            loop {
                let mut avail = 0u32;
                if WinHttpQueryDataAvailable(r.0, &mut avail).is_err() || avail == 0 {
                    break;
                }
                let start = body.len();
                body.resize(start + avail as usize, 0);
                let mut read = 0u32;
                if WinHttpReadData(r.0, body[start..].as_mut_ptr() as *mut _, avail, &mut read).is_err() {
                    break;
                }
                body.truncate(start + read as usize);
                if read == 0 || body.len() > (1 << 20) {
                    break;
                }
            }
            Ok((status, body))
        })()
    };
    // Don't leave the token lying around in freed memory.
    hdr.iter_mut().for_each(|c| *c = 0);
    result
}

/// ISO 8601 ("2026-11-15T03:00:00.123+00:00" / "...Z") or unix seconds/milliseconds.
fn parse_time(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_f64() {
        return Some(if n > 1e12 { (n / 1000.0) as i64 } else { n as i64 });
    }
    let s = v.as_str()?;
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    // Civil date -> days since 1970-01-01 (Howard Hinnant's algorithm).
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let mut t = days * 86400 + h * 3600 + mi * 60 + se;
    // Time zone suffix after optional fractional seconds.
    let rest = &s[19..];
    let tz = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    if let Some(sign) = tz.chars().next().filter(|c| *c == '+' || *c == '-') {
        let oh = tz.get(1..3).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
        let om = tz.get(4..6).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
        let off = oh * 3600 + om * 60;
        t -= if sign == '+' { off } else { -off };
    }
    Some(t)
}

// ---------- Codex ----------

fn find_codex(configured: Option<&str>) -> Option<(PathBuf, bool)> {
    if let Some(p) = configured.map(str::trim).filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        let via_cmd = p.extension().map(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat")).unwrap_or(false);
        return p.is_file().then_some((p, via_cmd));
    }
    // The npm package ships a native exe; running it directly skips node and a console shim.
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let base = PathBuf::from(appdata).join(r"npm\node_modules\@openai\codex\node_modules\@openai");
        for (pkg, triple) in [("codex-win32-x64", "x86_64-pc-windows-msvc"), ("codex-win32-arm64", "aarch64-pc-windows-msvc")] {
            let exe = base.join(pkg).join("vendor").join(triple).join(r"bin\codex.exe");
            if exe.is_file() {
                return Some((exe, false));
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        if dir.join("codex.exe").is_file() {
            return Some((dir.join("codex.exe"), false));
        }
    }
    for dir in std::env::split_paths(&path) {
        if dir.join("codex.cmd").is_file() {
            return Some((dir.join("codex.cmd"), true));
        }
    }
    None
}

fn fetch_codex(configured: Option<&str>) -> Fetch {
    const P: &str = "Codex";
    let Some((exe, via_cmd)) = find_codex(configured) else { return Fetch::Absent };
    match codex_rate_limits(&exe, via_cmd) {
        Ok(v) => parse_codex(&v),
        Err(msg) => Fetch::Err { provider: P, msg, retry: Some(Duration::from_secs(5 * 60)) },
    }
}

fn codex_rate_limits(exe: &PathBuf, via_cmd: bool) -> Result<Value, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = if via_cmd {
        let mut c = Command::new("cmd");
        c.arg("/d").arg("/c").arg(exe);
        c
    } else {
        Command::new(exe)
    };
    let mut child = cmd
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("can't start Codex CLI: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (tx, rx) = channel::<Value>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if tx.send(v).is_err() {
                    break;
                }
            }
        }
    });
    let wait_for = |id: i64| -> Result<Value, String> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let m = rx.recv_timeout(left).map_err(|_| "Codex CLI didn't answer".to_string())?;
            if m["id"].as_i64() == Some(id) {
                if let Some(e) = m.get("error") {
                    return Err(format!("Codex: {}", e["message"].as_str().unwrap_or("error")));
                }
                return Ok(m["result"].clone());
            }
        }
    };
    let mut send = |s: &str| writeln!(stdin, "{s}").and_then(|_| stdin.flush()).map_err(|_| "Codex CLI closed".to_string());
    let result = (|| {
        send(r#"{"id":1,"method":"initialize","params":{"clientInfo":{"name":"pocketisle","title":"Pocket Isle","version":"0.2.3"}}}"#)?;
        wait_for(1)?;
        send(r#"{"method":"initialized"}"#)?;
        send(r#"{"id":2,"method":"account/rateLimits/read"}"#)?;
        wait_for(2)
    })();
    drop(send);
    drop(stdin);
    // Closing stdin ends the server; make sure it's gone either way.
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(2) {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn window_names(mins: Option<i64>) -> (String, String) {
    match mins {
        Some(300) => ("5h".into(), "5-hour limit".into()),
        Some(10080) => ("Week".into(), "weekly limit".into()),
        Some(1440) => ("Day".into(), "daily limit".into()),
        Some(m) if m % 1440 == 0 => (format!("{}d", m / 1440), format!("{}-day limit", m / 1440)),
        Some(m) if m % 60 == 0 => (format!("{}h", m / 60), format!("{}-hour limit", m / 60)),
        _ => ("Limit".into(), "limit".into()),
    }
}

fn parse_codex(v: &Value) -> Fetch {
    const P: &str = "Codex";
    let by_id = v["rateLimitsByLimitId"].as_object();
    let limits_v = by_id
        .and_then(|m| m.get("codex").filter(|x| x.is_object()).or_else(|| m.values().find(|x| x.is_object())))
        .or_else(|| v.get("rateLimits").filter(|x| x.is_object()));
    let Some(l) = limits_v else {
        return Fetch::Err { provider: P, msg: "no limits; sign in to Codex with ChatGPT".into(), retry: Some(Duration::from_secs(30 * 60)) };
    };
    let mut limits = Vec::new();
    for key in ["primary", "secondary"] {
        let w = &l[key];
        let Some(used) = w["usedPercent"].as_f64() else { continue };
        let (short, long) = window_names(w["windowDurationMins"].as_i64());
        limits.push(Limit { short, long, used: used as f32 / 100.0, resets_at: parse_time(&w["resetsAt"]) });
    }
    if limits.is_empty() {
        return Fetch::Err { provider: P, msg: "no limits in response".into(), retry: None };
    }
    let mut note = l["planType"].as_str().map(plan_name).unwrap_or_default();
    if let Some(n) = v["rateLimitResetCredits"]["availableCount"].as_i64().filter(|n| *n > 0) {
        let s = format!("{n} reset{}", if n == 1 { "" } else { "s" });
        note = if note.is_empty() { s } else { format!("{note} · {s}") };
    }
    Fetch::Ok(Usage { provider: P, limits, note, error: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_times() {
        assert_eq!(parse_time(&Value::from("1970-01-01T00:00:00Z")), Some(0));
        assert_eq!(parse_time(&Value::from("2026-11-15T03:00:00.123456+00:00")), Some(1794711600));
        assert_eq!(parse_time(&Value::from("2026-11-15T13:00:00+10:00")), Some(1794711600));
        assert_eq!(parse_time(&Value::from(1794710765)), Some(1794710765));
    }

    #[test]
    fn codex_sample() {
        let v: Value = serde_json::from_str(r#"{"rateLimits":{"limitId":"codex","primary":{"usedPercent":0,"windowDurationMins":300,"resetsAt":1794710765},"secondary":{"usedPercent":10,"windowDurationMins":10080,"resetsAt":1795004861},"planType":"plus"},"rateLimitsByLimitId":{"codex":{"limitId":"codex","primary":{"usedPercent":0,"windowDurationMins":300,"resetsAt":1794710765},"secondary":{"usedPercent":10,"windowDurationMins":10080,"resetsAt":1795004861},"planType":"plus"}},"rateLimitResetCredits":{"availableCount":3}}"#).unwrap();
        let Fetch::Ok(u) = parse_codex(&v) else { panic!() };
        assert_eq!(u.limits.len(), 2);
        assert_eq!(u.limits[1].short, "Week");
        assert!((u.limits[1].used - 0.10).abs() < 1e-6);
        assert_eq!(u.note, "Plus · 3 resets");
    }

    #[test]
    fn countdowns() {
        assert_eq!(countdown(30), "1m");
        assert_eq!(countdown(2 * 3600 + 14 * 60), "2h 14m");
        assert_eq!(countdown(3 * 86400 + 4 * 3600), "3d 4h");
    }
}
