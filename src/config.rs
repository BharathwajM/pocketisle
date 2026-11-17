//! `%USERPROFILE%\.config\pocketisle\config.json`. Written with defaults on first run.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    /// Take over the volume keys so the Windows volume flyout never shows.
    pub hijack_volume_keys: bool,
    /// Volume change per key press, in percent.
    pub volume_step: u32,
    /// Hide while a full-screen app (game, video, presentation) is in front.
    pub hide_in_fullscreen: bool,
    /// Show the island on every screen (a copy at the top of each), not just the main one.
    pub all_monitors: bool,
    /// Show Windows notifications as island cards.
    pub show_notifications: bool,
    /// App display names whose notifications are never shown, e.g. ["Microsoft Teams"].
    pub notification_ignore: Vec<String>,
    /// How long a notification card stays on the island (seconds). Other cards scale with it.
    pub card_seconds: f32,
    /// After this many minutes of steady use, a 20-second eye break (0 = never).
    pub break_minutes: u32,
    /// Keep the last few things copied in the Shelf tab (in memory only).
    pub clipboard_history: bool,
    /// Pokémon cry when poked, caught or evolving (never while the microphone is on).
    pub pokemon_cries: bool,
    /// Now and then a wild Pokémon walks across the pill; click it to catch it.
    pub wild_pokemon: bool,
    /// A wild Pokémon comes at a random time this many minutes apart: [shortest, longest].
    pub wild_minutes: [f32; 2],
    /// Catching is timed: a shrinking ring around the visitor, click when it's small.
    pub timed_catch: bool,
    /// The Pokémon's HP follows what's left of Claude's 5-hour limit; it faints at 100%.
    pub claude_hp: bool,
    /// Pokémon Eggs turn up now and then and hatch after `egg_minutes` of using the PC.
    pub eggs: bool,
    pub egg_minutes: u32,
    /// Minutes before a timed calendar event that the Pokémon holds up a "Meeting in 5"
    /// sign (0 = never).
    pub meeting_sign_minutes: u32,
    /// The first time you're at the PC each day: a card with the weather, today's events
    /// and any special day.
    pub morning_greeting: bool,
    /// Remind this many minutes before a timed YASB calendar event.
    pub calendar_reminder_minutes: u32,
    /// Animate the little equaliser while music plays.
    pub animate_equalizer: bool,
    /// Draw with the graphics card. Off = software rendering, which is lighter for a
    /// window this small (no GPU driver loaded). Restart Pocket Isle after changing.
    pub gpu_rendering: bool,
    /// Override the distance from the top of the screen (logical px). Default: YASB's bar padding.
    pub top_offset: Option<f32>,
    /// Override the collapsed pill height (logical px). Default: YASB's bar height.
    pub pill_height: Option<f32>,
    /// Track Claude's 5-hour and weekly plan usage (uses Claude Code's sign-in, read-only).
    pub usage_claude: bool,
    /// Track Codex's 5-hour and weekly rate limits (asks the Codex CLI).
    pub usage_codex: bool,
    /// Path to codex.exe / codex.cmd if it isn't found automatically.
    pub codex_path: Option<String>,
    /// How often to re-check usage, in minutes.
    pub usage_refresh_minutes: u32,
    /// Fraction at which a limit turns gold (watch) and red (critical).
    pub usage_watch: f32,
    pub usage_critical: f32,
    /// Cards when a limit becomes critical / runs out, and when a high one resets.
    pub usage_alerts: bool,
    /// Show usage rings in the pill when nothing else is going on.
    pub usage_in_pill: bool,
    /// Where to get weather for. Default: the place set in YASB's weather widget.
    pub weather_location: Option<WeatherLocation>,
    pub weather_refresh_minutes: u32,
    /// Show the temperature in the pill when nothing else is going on.
    pub weather_in_pill: bool,
    /// The island's own weather, from the real weather outside: rain falling across it,
    /// sunshine, drifting clouds, fog, snow, storms with lightning and starry nights.
    /// "auto", "off", or one to try out: "sun", "night", "partly", "cloud", "fog",
    /// "drizzle", "rain", "storm", "snow".
    pub weather_effect: String,
    /// Show the day, date and time in the pill when nothing else is going on.
    pub clock_in_pill: bool,
    /// Buttons on the Home tab.
    pub shortcuts: Vec<Shortcut>,
    /// Answer Claude Code's permission prompts (Allow / Deny) on the island.
    pub decisions_on_island: bool,
    /// How long the island waits for an answer before Claude asks in its own window.
    pub decision_timeout_seconds: u32,
    /// Also show them when the Claude app or Windows Terminal is in front (normally you
    /// answer there, and the island only asks while you're in another app).
    pub decisions_when_claude_focused: bool,
    /// Take the island's accent colour from the desktop wallpaper (off = YASB's accent).
    pub accent_from_wallpaper: bool,
    /// Hang the island from the top edge of the screen (off = a floating pill).
    pub attach_to_top: bool,
    /// Show Pip, the little pixel island that reacts to what's going on.
    pub mascot: bool,
    /// The mascot is a Pokémon (Rayquaza, or one matching the music's colour) instead of Pip.
    pub mascot_pokemon: bool,
    /// Show what Claude / Codex are doing ("Editing app.rs") in the pill, and a card when they
    /// finish. Off = they work quietly; the island only speaks up when they need you.
    pub agent_status_in_pill: bool,
    /// Peek out with the album art when a new song starts.
    pub song_peek: bool,
    /// Peek out with a preview when you copy text or an image.
    pub copy_peek: bool,
    /// Peek out with a thumbnail when a screenshot is saved (click it to open).
    pub screenshot_peek: bool,
    /// Peek out when the charger is plugged in or out, and when the battery runs low.
    pub power_peek: bool,
    /// Peek out when headphones or a headset connect, with their battery if Windows knows it.
    pub headphones_peek: bool,
    /// Peek out when a file finishes downloading into Downloads.
    pub download_peek: bool,
    /// Peek out when your pull requests' checks pass or fail, or someone reviews them
    /// (needs the GitHub CLI, signed in with `gh auth login`), looking every this many minutes.
    pub github_peeks: bool,
    pub github_minutes: u32,
    /// Now and then a visitor wants to battle before it can be caught.
    pub battles: bool,
    /// Now and then one species turns up again and again for an hour.
    pub outbreaks: bool,
    /// On weekends a strong legendary raids the island: win battles to break its shields,
    /// then catch it.
    pub raids: bool,
    /// Peek out when the CPU or GPU has been running hot for a while (the GPU at
    /// `heat_gpu_c` °C or more for a minute), naming the app doing most of the work.
    pub heat_peek: bool,
    pub heat_gpu_c: u32,
    /// Wallpaper Engine (Steam) users: each morning every screen gets a different wallpaper
    /// from your own library (rated Everyone, none shown lately).
    pub daily_wallpaper: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct WeatherLocation {
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default)]
    pub name: String,
}

/// A Home tab button. `target` is a folder, file, program or URI ("ms-settings:"); `~` and
/// %VARS% are expanded. `hidden` runs a program without a console window.
#[derive(Serialize, Deserialize, Clone)]
pub struct Shortcut {
    pub name: String,
    /// A Nerd Font glyph.
    pub icon: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

fn sc(name: &str, icon: &str, target: &str) -> Shortcut {
    Shortcut { name: name.into(), icon: icon.into(), target: target.into(), args: None, hidden: false }
}

pub fn default_shortcuts() -> Vec<Shortcut> {
    vec![
        sc("Home", "\u{f02dc}", "~"),
        sc("Downloads", "\u{f024d}", "~\\Downloads"),
        sc("Documents", "\u{f0219}", "~\\Documents"),
        sc("Pictures", "\u{f024f}", "~\\Pictures"),
        sc("Settings", "\u{f0493}", "ms-settings:"),
        sc("Task Manager", "\u{f04c5}", "taskmgr.exe"),
        sc("Terminal", "\u{f018d}", "wt.exe"),
        sc("This PC", "\u{f0379}", "ms-settings:about"),
    ]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hijack_volume_keys: true,
            volume_step: 2,
            hide_in_fullscreen: true,
            all_monitors: true,
            show_notifications: true,
            notification_ignore: Vec::new(),
            card_seconds: 3.5,
            break_minutes: 60,
            clipboard_history: true,
            pokemon_cries: true,
            wild_pokemon: true,
            wild_minutes: [5.0, 15.0],
            timed_catch: true,
            claude_hp: true,
            eggs: true,
            egg_minutes: 120,
            meeting_sign_minutes: 5,
            morning_greeting: true,
            calendar_reminder_minutes: 10,
            animate_equalizer: true,
            gpu_rendering: false,
            top_offset: None,
            pill_height: None,
            usage_claude: true,
            usage_codex: true,
            codex_path: None,
            usage_refresh_minutes: 5,
            usage_watch: 0.5,
            usage_critical: 0.7,
            usage_alerts: true,
            usage_in_pill: false,
            weather_location: None,
            weather_refresh_minutes: 10,
            weather_in_pill: true,
            weather_effect: "auto".into(),
            clock_in_pill: true,
            shortcuts: default_shortcuts(),
            decisions_on_island: true,
            decision_timeout_seconds: 90,
            decisions_when_claude_focused: false,
            accent_from_wallpaper: true,
            attach_to_top: true,
            mascot: true,
            mascot_pokemon: true,
            agent_status_in_pill: false,
            song_peek: true,
            copy_peek: true,
            screenshot_peek: true,
            power_peek: true,
            headphones_peek: true,
            download_peek: true,
            github_peeks: true,
            github_minutes: 3,
            battles: true,
            outbreaks: true,
            raids: true,
            heat_peek: true,
            heat_gpu_c: 85,
            daily_wallpaper: true,
        }
    }
}

impl Config {
    pub fn usage(&self) -> crate::usage::UsageCfg {
        crate::usage::UsageCfg {
            claude: self.usage_claude,
            codex: self.usage_codex,
            codex_path: self.codex_path.clone(),
            refresh_minutes: self.usage_refresh_minutes,
        }
    }

    pub fn weather_place(&self) -> Option<crate::weather::Place> {
        match &self.weather_location {
            Some(l) => Some(crate::weather::Place { lat: l.latitude, lon: l.longitude, name: l.name.clone() }),
            None => crate::weather::yasb_place(),
        }
    }

    pub fn path() -> std::path::PathBuf {
        crate::util::config_dir().join("config.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<Self>(&s) {
                Ok(c) => {
                    // Add settings introduced since the file was written, so they're discoverable.
                    let had = serde_json::from_str::<serde_json::Value>(&s).ok();
                    let full = serde_json::to_value(&c).ok();
                    if let (Some(serde_json::Value::Object(had)), Some(serde_json::Value::Object(full))) = (had, full) {
                        if full.keys().any(|k| !had.contains_key(k)) {
                            if let Ok(out) = serde_json::to_string_pretty(&c) {
                                let _ = std::fs::write(&path, out);
                            }
                        }
                    }
                    c
                }
                Err(e) => {
                    crate::util::log(&format!("config.json ignored: {e}"));
                    Self::default()
                }
            },
            Err(_) => {
                let c = Self::default();
                if let Ok(s) = serde_json::to_string_pretty(&c) {
                    let _ = std::fs::write(&path, s);
                }
                c
            }
        }
    }
}
