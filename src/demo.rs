//! `pocketisle.exe demo`: the island with made-up data, playing a tour of its features by itself
//! (for screenshots and videos). Nothing of yours is shown: no notifications, calendar,
//! music, usage, clipboard, files or agent prompts are read, and the mouse and keyboard are
//! ignored. Settings and saves go to `%LOCALAPPDATA%\PocketIsle\demo`, a fresh copy each run, so
//! the real island's files are never touched. A plain backdrop sits behind the island so
//! recordings show only the island.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::events::Art;
use crate::util;

/// Where the demo keeps its settings, saves and notes about the tour.
pub fn dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(util::home)
        .join("PocketIsle")
        .join("demo")
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// Start from a clean folder: your settings (with a few changes for the tour) and your
/// Pokédex are copied in; everything else is made up.
pub fn prepare() {
    let d = dir();
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::create_dir_all(d.join("Downloads"));
    let _ = std::fs::create_dir_all(d.join("Shelf"));
    let real = util::real_config_dir();

    let mut cfg: serde_json::Value = std::fs::read_to_string(real.join("config.json"))
        .ok()
        .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(o) = cfg.as_object_mut() {
        for (k, v) in [
            ("daily_wallpaper", serde_json::json!(false)),
            ("hide_in_fullscreen", serde_json::json!(false)),
            ("timed_catch", serde_json::json!(false)),
            ("decisions_on_island", serde_json::json!(true)),
            ("decisions_when_claude_focused", serde_json::json!(true)),
            ("github_peeks", serde_json::json!(true)),
            ("download_peek", serde_json::json!(true)),
            ("song_peek", serde_json::json!(true)),
            ("mascot", serde_json::json!(true)),
            ("mascot_pokemon", serde_json::json!(true)),
            ("wild_pokemon", serde_json::json!(true)),
            ("clipboard_history", serde_json::json!(true)),
            ("meeting_sign_minutes", serde_json::json!(0)),
            ("agent_status_in_pill", serde_json::json!(true)),
            ("clock_in_pill", serde_json::json!(true)),
            ("side_panels", serde_json::json!("both")),
        ] {
            o.insert(k.to_string(), v);
        }
        // Other side panels ("wings", "corners", "off"), for pictures of each.
        if let Ok(v) = std::env::var("POCKETISLE_DEMO_SIDES") {
            o.insert("side_panels".to_string(), serde_json::json!(v));
        }
    }
    let _ = std::fs::write(d.join("config.json"), serde_json::to_string_pretty(&cfg).unwrap_or_default());
    let _ = std::fs::copy(real.join("pokedex.json"), d.join("pokedex.json"));

    let notes = serde_json::json!([
        { "text": "Water the plants 🌱", "done": true },
        { "text": "Book flights for the trip ✈️", "done": false },
        { "text": "Finish the island README", "done": false },
        { "text": "Call Grandma on Sunday", "done": false }
    ]);
    let _ = std::fs::write(d.join("notes.json"), notes.to_string());
    let clips = serde_json::json!([
        "https://example.com/recipes/coconut-curry",
        "Meet at the pier at 6:30",
        "#3a86ff",
        "cargo build --release",
        "Thanks so much, see you then!"
    ]);
    let _ = std::fs::write(d.join("clips.json"), clips.to_string());
    let games = serde_json::json!([
        { "path": "demo:1", "title": "Tiny Island Quest", "system": "GBA", "last": now_ms() as i64 / 1000 - 3600 },
        { "path": "demo:2", "title": "Coral Kart", "system": "DS", "last": now_ms() as i64 / 1000 - 86400 },
        { "path": "demo:3", "title": "Lighthouse Puzzle", "system": "GBC", "last": 0 },
        { "path": "demo:4", "title": "Pixel Surfer", "system": "GB", "last": 0 }
    ]);
    let _ = std::fs::write(d.join("games.json"), games.to_string());

    // Today's and the coming days' plans.
    let t = crate::calendar::local_now().date;
    let day = |n: i64| {
        let x = t.add(n);
        format!("{:04}-{:02}-{:02}", x.y, x.m, x.d)
    };
    let cal = serde_json::json!([
        { "id": "1", "title": "Stand-up", "date": day(0), "time": "09:30" },
        { "id": "2", "title": "Lunch with Maya", "date": day(0), "time": "13:00" },
        { "id": "3", "title": "Surf lesson 🏄", "date": day(0), "time": "17:30" },
        { "id": "4", "title": "Dentist", "date": day(2), "time": "10:00" },
        { "id": "5", "title": "Movie night", "date": day(4), "time": "19:30" }
    ]);
    let _ = std::fs::write(d.join("calendar-events.json"), cal.to_string());

    let mut shelf = Vec::new();
    for name in ["Trip itinerary.pdf", "Beach photos.zip", "Island sketch.png", "Budget.xlsx"] {
        let p = d.join("Shelf").join(name);
        let _ = std::fs::write(&p, b"");
        shelf.push(p);
    }
    let _ = std::fs::write(d.join("shelf.json"), serde_json::to_string_pretty(&shelf).unwrap_or_default());
    let _ = std::fs::write(d.join("Downloads").join("vacation-playlist.zip"), b"");
    let _ = std::fs::write(d.join("scenes.txt"), "");
    let _ = std::fs::write(d.join("rects.txt"), "");
}

/// Note when a scene of the tour starts (unix ms), so stills can be cut from a recording.
pub fn scene(name: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir().join("scenes.txt")) {
        let _ = writeln!(f, "{} {name}", now_ms());
    }
}

/// Note where the island is on screen (pixels) whenever that changes, so stills cut from a
/// recording can be cropped to it.
pub fn rect(x0: i32, y0: i32, x1: i32, y1: i32) {
    use std::io::Write;
    use std::sync::Mutex;
    static LAST: Mutex<[i32; 4]> = Mutex::new([0; 4]);
    let r = [x0, y0, x1, y1];
    if let Ok(mut last) = LAST.lock() {
        if *last == r {
            return;
        }
        *last = r;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir().join("rects.txt")) {
        let _ = writeln!(f, "{} {x0} {y0} {x1} {y1}", now_ms());
    }
}

/// A made-up album cover: a sunset over the sea.
pub fn album_art() -> Art {
    let (w, h) = (160u32, 160u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let fy = y as f32 / h as f32;
            let fx = x as f32 / w as f32;
            let sea = fy > 0.62;
            let (mut r, mut g, mut b) = if sea {
                let k = (fy - 0.62) / 0.38;
                (40.0 + 30.0 * (1.0 - k), 70.0 + 40.0 * (1.0 - k), 140.0 + 40.0 * (1.0 - k))
            } else {
                let k = fy / 0.62;
                (255.0 - 40.0 * (1.0 - k), 110.0 + 70.0 * k, 120.0 - 40.0 * k)
            };
            // The sun, half set, and its glitter on the water.
            let (dx, dy) = (fx - 0.5, fy - 0.6);
            let d = (dx * dx + dy * dy).sqrt();
            if !sea && d < 0.18 {
                (r, g, b) = (255.0, 214.0, 120.0);
            }
            if sea && dx.abs() < 0.12 * (1.0 - (fy - 0.62) * 1.5) && ((y / 4) % 2 == 0) {
                r = r * 0.4 + 255.0 * 0.6;
                g = g * 0.4 + 200.0 * 0.6;
                b = b * 0.4 + 140.0 * 0.6;
            }
            px.extend_from_slice(&[b as u8, g as u8, r as u8, 255]);
        }
    }
    Art { w, h, px, tint: [0.98, 0.52, 0.42] }
}

/// The music's loudness while the demo "plays": a beat twice a second.
pub fn level(t: f32) -> f32 {
    let beat = (t * 2.0).fract();
    let kick = (-beat * 5.0).exp();
    (0.10 + 0.55 * kick + 0.06 * (t * 7.3).sin().abs()).clamp(0.0, 1.0)
}

// ---------- the backdrop ----------

unsafe extern "system" fn backdrop_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            // A soft dusk sky, darker at the top where the island hangs.
            let v = [
                TRIVERTEX { x: 0, y: 0, Red: 0x1400, Green: 0x1c00, Blue: 0x3000, Alpha: 0 },
                TRIVERTEX { x: rc.right, y: rc.bottom, Red: 0x4a00, Green: 0x3a00, Blue: 0x6e00, Alpha: 0 },
            ];
            let m = [GRADIENT_RECT { UpperLeft: 0, LowerRight: 1 }];
            let _ = GradientFill(dc, &v, m.as_ptr() as *const _, 1, GRADIENT_FILL_RECT_V);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// A plain window behind the island (x, y, w, h in screen pixels), and a note of where it is
/// so a recording can crop to it.
pub fn backdrop(x: i32, y: i32, w: i32, h: i32) {
    // The whole width of the (main) screen, so the side panels at its edges have it too.
    let (bx, bw) = unsafe { (0, windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN)) };
    let (bx, bw) = if bw > 0 { (bx, bw) } else { (x, w) };
    unsafe {
        let Ok(hm) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = hm.into();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(backdrop_proc),
            hInstance: hinst,
            lpszClassName: w!("PocketIsleDemoBackdrop"),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        if let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("PocketIsleDemoBackdrop"),
            w!("Pocket Isle demo"),
            WS_POPUP | WS_VISIBLE,
            bx,
            y,
            bw,
            h,
            None,
            None,
            Some(hinst),
            None,
        ) {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
    let _ = std::fs::write(dir().join("rect.txt"), format!("{x} {y} {w} {h}"));
}

// ---------- made-up data ----------

/// A sunny (or rainy) day by the sea.
pub fn weather(rain: bool) -> crate::weather::Weather {
    use crate::weather::{Day, Hour, Weather};
    let now = crate::calendar::local_now();
    let hours = (0..8)
        .map(|i| {
            let h = (now.h + 1 + i * 2) % 24;
            let day = (6..19).contains(&h);
            Hour { label: format!("{h:02}:00"), temp: 27.0 - (i as f32 - 3.0).abs(), code: if rain { 61 } else if i % 3 == 2 { 2 } else { 1 }, rain: if rain { 70 } else { 5 }, day }
        })
        .collect();
    let names = ["Today", "Wed", "Thu", "Fri", "Sat"];
    let codes = [if rain { 61 } else { 1 }, 2, 0, 61, 1];
    let days = names
        .iter()
        .zip(codes)
        .enumerate()
        .map(|(i, (n, c))| Day { label: n.to_string(), code: c, hi: 29.0 - i as f32 * 0.5, lo: 22.0 + (i % 2) as f32, rain: if c == 61 { 60 } else { 10 } })
        .collect();
    Weather { place: "Honolulu".into(), temp: if rain { 24.0 } else { 27.0 }, feels: if rain { 25.0 } else { 29.0 }, code: if rain { 63 } else { 1 }, day: true, humidity: 68, wind: 14.0, hours, days }
}

/// Claude and Codex usage, comfortably under their limits.
pub fn usage() -> Vec<crate::usage::Usage> {
    use crate::usage::{Limit, Usage};
    let now = crate::usage::now_unix();
    let lim = |short: &str, long: &str, used: f32, secs: i64| Limit { short: short.into(), long: long.into(), used, resets_at: Some(now + secs) };
    vec![
        Usage { provider: "Claude", limits: vec![lim("5h", "5-hour limit", 0.42, 2 * 3600 + 900), lim("Week", "weekly limit", 0.27, 3 * 86400)], note: "Max".into(), error: None },
        Usage { provider: "Codex", limits: vec![lim("5h", "5-hour limit", 0.18, 3 * 3600), lim("Week", "weekly limit", 0.35, 4 * 86400)], note: "Plus".into(), error: None },
    ]
}

/// The System tab with made-up hardware, apps and Bluetooth devices: the demo shows none of
/// this PC's own (its load and temperature are live, so the graphs move).
pub fn scrub_sys(s: &mut crate::sysinfo::Sys) {
    use crate::devices::{Battery, DevKind};
    use crate::procs::Proc;
    s.cpu_info.name = "Octa-core CPU".into();
    if let Some(g) = s.gpu.as_mut() {
        g.name = "Lagoon Graphics".into();
        g.driver = String::new();
    }
    for m in s.mem_info.modules.iter_mut() {
        m.maker = String::new();
        m.part = String::new();
    }
    for d in s.drives.iter_mut() {
        d.model = "Lagoon SSD 1TB".into();
        d.label = String::new();
    }
    let mb = 1024 * 1024;
    s.procs = [("Claude", 2.4, 1900), ("Browser", 1.6, 1400), ("Code editor", 0.9, 820), ("Music", 0.4, 260), ("Terminal", 0.2, 90)]
        .into_iter()
        .map(|(name, cpu, mem): (&str, f32, u64)| Proc { name: name.into(), exe: String::new(), pids: Vec::new(), cpu: cpu / 100.0, mem: mem * mb })
        .collect();
    s.batteries = vec![
        Battery { name: "Earbuds Pro".into(), pct: 80, kind: DevKind::Audio },
        Battery { name: "Mouse".into(), pct: 56, kind: DevKind::Mouse },
        Battery { name: "Controller".into(), pct: 34, kind: DevKind::Controller },
    ];
}

/// Sound outputs named by kind only.
pub fn scrub_outputs(list: &mut [crate::devices::Output]) {
    let mut phones = 0;
    for o in list {
        if o.form == 3 {
            phones += 1;
        }
        let (name, hw) = match o.form {
            3 if phones > 1 => ("Headphones", "Studio headphones"),
            3 => ("Headphones", "Earbuds Pro"),
            5 => ("Headset", "Gaming headset"),
            9 => ("Monitor", "Display audio"),
            _ => ("Speakers", "Built-in audio"),
        };
        o.name = name.into();
        o.hw = hw.into();
    }
}
