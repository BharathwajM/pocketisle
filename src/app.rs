//! The island window: placement inside YASB's bar, the state machine (compact / pop-up /
//! card / expanded), spring-animated shape, drawing, and input.
//!
//! The UI thread blocks in GetMessage whenever nothing is moving, so an idle island uses
//! no CPU. Frames are only rendered while the shape is morphing or something is animating.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use windows::core::{w, Result, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::agent::{self, Phase, AGENT_MAGIC};
use crate::anim::Spring;
use crate::audio::Audio;
use crate::calendar::{self, Calendar, Date};
use crate::config::Config;
use crate::controls::{self, PowerAct};
use crate::sysinfo::{self, Sys};
use crate::wallpaper;
use crate::weather::{self, Weather};
use crate::events::{self, Art, Event, MediaInfo, WM_APP_APPBAR, WM_APP_EVENT, WM_APP_VOLKEY};
use crate::media::{self, MediaCmd};
use crate::peeks;
use crate::pokemon;
use crate::render::{Align, Renderer, R};
use crate::theme::{Rgba, Theme};
use crate::usage::{self, Usage, UsageCmd};
use crate::util;

const CLASS: PCWSTR = w!("PocketIsle");
/// The copies of the island on the other screens.
const MIRROR_CLASS: PCWSTR = w!("PocketIsleMirror");
/// The window is a fixed transparent canvas; only the pill inside it is visible / clickable.
const WIN_W: f32 = 560.0;
/// Width of the opened island.
const EXP_W: f32 = 540.0;
const WIN_H: f32 = 640.0;

/// commctrl.h's WM_MOUSELEAVE (not exported by the modules imported here).
const WM_MOUSELEAVE: u32 = 0x02A3;

const T_HOVER: usize = 1;
const T_LEAVE: usize = 2;
const T_EXPIRE: usize = 3;
const T_CAL: usize = 4;
const T_ARM: usize = 5;
const T_DECIDE: usize = 6;
const T_WALL: usize = 7;
const T_CLOCK: usize = 8;

// Nerd Font (Material Design) glyphs; YASB's icon font has them.
const G_VOL_HIGH: &str = "\u{f057e}";
const G_VOL_MED: &str = "\u{f0580}";
const G_VOL_LOW: &str = "\u{f057f}";
const G_VOL_MUTE: &str = "\u{f0581}";
const G_SUN: &str = "\u{f00e0}";
const G_PLAY: &str = "\u{f040a}";
const G_PAUSE: &str = "\u{f03e4}";
const G_NEXT: &str = "\u{f04ad}";
const G_PREV: &str = "\u{f04ae}";
const G_ROBOT: &str = "\u{f06a9}";
const G_BELL: &str = "\u{f009a}";
const G_CAL: &str = "\u{f00ed}";
const G_CHECK: &str = "\u{f012c}";
const G_MUSIC: &str = "\u{f075a}";
const G_GAUGE: &str = "\u{f029a}";
const G_RESTORE: &str = "\u{f099b}";
// The same glyphs YASB's bar used for these widgets.
const G_CPU: &str = "\u{f4bc}";
const G_HOME: &str = "\u{f06a1}";
const G_CONTROLS: &str = "\u{eaf8}";
const G_ACCOUNT: &str = "\u{f0009}";
const G_WEATHER: &str = "\u{f0590}";
const G_MEMORY: &str = "\u{f035b}";
const G_GPU: &str = "\u{f08ae}";
const G_DISK: &str = "\u{f02ca}";
const G_MIC: &str = "\u{f036c}";

static HOOK_ON: AtomicBool = AtomicBool::new(false);
/// The main island window (the copies on other screens forward their input to it).
static MAIN_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<T>(f: impl FnOnce(&mut App) -> T) -> Option<T> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Compact,
    Osd,
    Card,
    /// A Claude Code permission prompt with Allow / Deny buttons.
    Decision,
    Expanded,
}

/// Which window clicking an agent card brings forward.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Focus {
    No,
    Terminal,
    Claude,
    Codex,
}

impl Focus {
    /// Where an agent's session lives: its desktop app, or a terminal.
    fn agent(agent: &str, desktop: bool) -> Self {
        match (agent, desktop) {
            ("claude", true) => Focus::Claude,
            ("codex", true) => Focus::Codex,
            _ => Focus::Terminal,
        }
    }
}

/// A permission prompt waiting on the island; the hook process blocks until it's answered.
struct Decision {
    id: String,
    key: String,
    label: String,
    question: String,
    detail: String,
    choices: Vec<crate::ask::Choice>,
    /// Pick-several: choices toggle, and Done sends them.
    multi: bool,
    picked: Vec<bool>,
    pid: u32,
    focus: Focus,
    since: Instant,
    deadline: Instant,
}

#[derive(Clone, Copy, PartialEq)]
enum OsdKind {
    Volume,
    Brightness,
}

struct Osd {
    kind: OsdKind,
    value: f32,
    muted: bool,
    until: Instant,
}

/// A Pokémon's animated sprite, ready for the pill: every frame side by side, already at
/// its on-screen size (`fw`×`fh` bitmap pixels per frame, drawn `dw`×`dh` logical px).
struct Sprite {
    fw: f32,
    fh: f32,
    dw: f32,
    dh: f32,
    delays: Vec<u32>,
    bmp: ID2D1Bitmap1,
    /// The pill height and screen scale it was sized for.
    box_h: f32,
    scale: f32,
}

/// A wild Pokémon crossing the pill.
struct Wild {
    name: &'static str,
    shiny: bool,
    legend: bool,
    since: Instant,
    from_left: bool,
    /// A Poké Ball was thrown: when, and whether it will hold.
    catch: Option<(Instant, bool)>,
    /// Where along its walk it was caught.
    u_at_catch: f32,
    /// Poké Balls thrown so far, how good the last throw was (0..1) and whether it runs off.
    tries: u8,
    quality: f32,
    flee: bool,
    /// The ball thrown last (0 Poké, 1 Great, 2 Ultra, 3 Master).
    ball: u8,
    /// It wants to battle before it can be caught; it lost one and is tired (easier to catch);
    /// a battle going on (since when, and whether you win).
    battle: bool,
    tired: bool,
    fight: Option<(Instant, bool)>,
    /// This weekend's raid boss; out in weather that suits it (more XP, easier to catch).
    raid: bool,
    boosted: bool,
    /// Fished up rather than walked by.
    fished: bool,
}

/// A line cast from beside the mascot: when the next bite comes, a bite going on (click
/// within FISH_BITE to reel it in), and how many got away.
struct Fishing {
    bite_at: Instant,
    bite: Option<Instant>,
    misses: u8,
}

/// The rod, line and bobber beside the mascot.
const FISH_W: f32 = 24.0;
const FISH_BITE: Duration = Duration::from_millis(3000);

/// Friendship from petting your buddy (at most every 30 s, up to PET_DAILY a day), and a
/// point for every WALK_MINUTES at the PC together.
const PET_GAIN: u32 = 3;
const PET_DAILY: u32 = 45;
const WALK_MINUTES: u32 = 5;
const G_FISH: &str = "\u{f023a}";
const G_MEDAL: &str = "\u{f0987}";
const G_QUEST: &str = "\u{f04ce}";

/// How long until the next bite (quicker in the rain).
fn fish_wait(wet: bool) -> Duration {
    Duration::from_secs_f32((6.0 + rand01() * 19.0) * if wet { 0.6 } else { 1.0 })
}

impl Wild {
    fn sprite(&self) -> String {
        if self.shiny {
            format!("{}-shiny", self.name)
        } else {
            self.name.to_string()
        }
    }
}

struct Card {
    key: Option<String>,
    glyph: &'static str,
    tint: Rgba,
    app: String,
    title: String,
    body: String,
    icon: Option<Art>,
    sticky: bool,
    ttl: Duration,
    focus: Focus,
    kind: CardKind,
}

/// What a card is for, which decides its size, look and what clicking it does.
#[derive(Clone, PartialEq)]
enum CardKind {
    Plain,
    /// A new song: the live album art, with the tide underneath. Click = open Music.
    Song,
    /// A screenshot or copied image: a big thumbnail. Click = open the file (once saved).
    Shot(Option<PathBuf>),
    /// Charger plugged in / out: a fill sweeps to the battery level.
    Power { pct: u8, charging: bool },
    /// The 20-20-20 eye break: counts down from when it shows.
    Break,
    /// A finished download. Click = open it; its buttons show it in the folder or shelve it.
    Download(PathBuf),
    /// Click = open this web page.
    Link(String),
}

struct Agent {
    key: String,
    label: String,
    phase: Phase,
    since: Instant,
    /// What it's doing right now ("Running cargo build"), from the last hook event.
    activity: String,
    focus: Focus,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Btn {
    Prev,
    PlayPause,
    Next,
    Tab(Tab),
    Shortcut(usize),
    Quick(Quick),
    Slider(Slider),
    Power(PowerAct),
    /// Answer the current permission prompt: an index into `agent::ANSWERS`.
    Decide(usize),
    /// A file on the shelf (click = open, drag = take it out) and its remove button.
    ShelfFile(usize),
    ShelfRemove(usize),
    /// Something copied lately: click = copy it again.
    Clip(usize),
    /// Pokédex view: by number (0), by type (1) or the trainer card (2); and the page.
    DexView(u8),
    DexPage(i32),
    /// System tab's top apps: by CPU (false) or memory (true); close one.
    SysSort(bool),
    Kill(usize),
    /// The do-not-disturb button in the tab bar (on every tab).
    DndBar,
    /// Game tab: open a game, play / remove one from the library, show / stop the player.
    GameOpen,
    GamePlay(usize),
    GameRemove(usize),
    GameShow,
    GameStop,
    /// System tab: open a detail page (1 CPU, 2 Memory, 3 GPU, 4 Storage), go back, open a
    /// drive, a row of the folder list, its pages, and the folder in File Explorer.
    SysPage(u8),
    SysBack,
    Drive(usize),
    ScanRow(usize),
    ScanPage(i32),
    ScanExplorer,
    /// Shelf: the recent copies' pages, remove one, clear them all.
    ClipPage(i32),
    ClipRemove(usize),
    ClipClear,
    /// A download's card: show it in its folder, or keep it on the shelf.
    CardFolder,
    CardShelf,
    /// Controls: make this sound output the default.
    Output(usize),
    /// Home: tick a note, remove it, start typing a new one, clear the ticked ones.
    Note(usize),
    NoteRemove(usize),
    NoteAdd,
    NotesClear,
    /// Pokédex bag: the ball to throw next.
    Ball(u8),
    /// Controls: another wallpaper from Wallpaper Engine now.
    WallNext,
    /// Pokédex bag: go fishing (the island closes so you can watch the bobber), or stop.
    Fish,
    /// Trainer card: a medal (point at it to read it).
    Medal(usize),
    /// Pokédex grid: a caught Pokémon (click to make it your buddy, again to stop).
    Buddy(&'static str),
}

/// Something copied lately, kept for the Shelf tab.
#[derive(Clone, PartialEq)]
enum HistClip {
    Text(String),
    Image { w: u32, h: u32, dib: std::rc::Rc<Vec<u8>> },
}

/// Controls tab toggles (the same set as YASB's control centre).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Quick {
    Dnd,
    Mute,
    MicMute,
    Screenshot,
    Keyboard,
    DarkMode,
}

const QUICKS: [Quick; 6] = [Quick::Dnd, Quick::Mute, Quick::MicMute, Quick::Screenshot, Quick::Keyboard, Quick::DarkMode];

impl Quick {
    fn glyph(self) -> &'static str {
        match self {
            Quick::Dnd => "\u{f00a0}",
            Quick::Mute => G_VOL_MUTE,
            Quick::MicMute => "\u{f036d}",
            Quick::Screenshot => "\u{f0190}",
            Quick::Keyboard => "\u{f030c}",
            Quick::DarkMode => "\u{f050e}",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Quick::Dnd => "Do not disturb",
            Quick::Mute => "Mute",
            Quick::MicMute => "Mic mute",
            Quick::Screenshot => "Screenshot",
            Quick::Keyboard => "Keyboard",
            Quick::DarkMode => "Dark mode",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Slider {
    Brightness,
    Volume,
    Mic,
    /// "Remind me to take a break in…" (0 to 120 minutes).
    Break,
}

/// The expanded island's tabs, drawn like YASB's round widget buttons.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Tab {
    Media,
    Usage,
    Calendar,
    System,
    Weather,
    Shelf,
    Dex,
    Game,
    Home,
    Controls,
    Power,
}

const TABS: [Tab; 11] = [Tab::Media, Tab::Usage, Tab::Calendar, Tab::System, Tab::Weather, Tab::Shelf, Tab::Dex, Tab::Game, Tab::Home, Tab::Controls, Tab::Power];
/// Tabs from this index on sit at the right, like the bar's right-hand widgets.
const RIGHT_TABS: usize = 8;
const TAB_H: f32 = 30.0;
/// Width of the permission prompt.
const DEC_W: f32 = 430.0;
/// Games listed in the Game tab.
const GAME_ROWS: usize = 5;
/// Longest agent name / activity shown in the pill.
/// Radius of the curves where the hanging island meets the screen's top edge.
const SHOULDER: f32 = 10.0;
/// How long a click ripple and Pip's hop last.
const RIPPLE: Duration = Duration::from_millis(520);
const HOP: Duration = Duration::from_millis(560);
/// Pip's pixel size and footprint in the pill (logical px).
const PIP_PX: f32 = 1.5;
const PIP_W: f32 = 24.0;
/// The widest a real Pokémon sprite may be in the pill (logical px).
const MON_MAX_W: f32 = 52.0;

const PILL_LABEL_W: f32 = 130.0;
const PILL_ACT_W: f32 = 210.0;

impl Tab {
    fn glyph(self) -> &'static str {
        match self {
            Tab::Media => G_MUSIC,
            Tab::Usage => G_GAUGE,
            Tab::Calendar => G_CAL,
            Tab::System => G_CPU,
            Tab::Weather => G_WEATHER,
            Tab::Shelf => G_SHELF,
            Tab::Dex => G_DEX,
            Tab::Home => G_HOME,
            Tab::Controls => G_CONTROLS,
            Tab::Power => G_ACCOUNT,
            Tab::Game => G_GAME,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Tab::Media => "Music",
            Tab::Usage => "Usage",
            Tab::Calendar => "Calendar",
            Tab::System => "System",
            Tab::Weather => "Weather",
            Tab::Shelf => "Shelf",
            Tab::Dex => "Pokédex",
            Tab::Home => "Home",
            Tab::Controls => "Controls",
            Tab::Power => "Power",
            Tab::Game => "Games",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum PipMood {
    Calm,
    Music,
    Needs,
    Sleep,
}

/// The weather over Pip's island.
#[derive(Clone, Copy, PartialEq)]
enum Sky {
    None,
    Sun,
    Moon,
    Cloud,
    Rain,
    Snow,
    Storm,
}

/// The weather the island itself shows.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Wx {
    Sun,
    Night,
    PartlySun,
    PartlyNight,
    Cloud,
    Fog,
    Drizzle,
    Rain,
    Downpour,
    Storm,
    Hail,
    Snow,
}

impl Wx {
    /// Milliseconds between frames: as few as still look smooth for how fast things move.
    fn frame_ms(self) -> u32 {
        match self {
            Wx::Storm | Wx::Hail => 40,
            Wx::Downpour => 45,
            Wx::Drizzle | Wx::Rain => 50,
            Wx::Snow => 66,
            _ => 100,
        }
    }
}

/// A fixed pseudo-random number in 0..1 for particle `i` (the same every frame).
fn rnd(i: u32, salt: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B1) ^ salt.wrapping_mul(0x85EB_CA77).wrapping_add(0x27D4_EB2F);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// How long a card's content takes to slide in.
const CARD_IN: Duration = Duration::from_millis(380);
/// Horizontal scroll that counts as a swipe (a mouse notch is 120), and the slide it shows.
const SWIPE_AT: f32 = 300.0;
const SWIPE_PX: f32 = 28.0;
const T_SWIPE: usize = 9;
const T_POWER: usize = 10;
/// After pausing, the pill stays on the song this long before going back to the clock.
const T_PAUSED: usize = 11;
const PAUSE_GRACE: Duration = Duration::from_millis(1500);
const G_HEADPHONES: &str = "\u{f02cb}";
const G_BATTERY_CHARGING: &str = "\u{f0084}";
const G_BATTERY_LOW: &str = "\u{f0083}";
const G_BATTERY: &str = "\u{f0079}";
const G_CLIPBOARD: &str = "\u{f0147}";
const G_SCREENSHOT: &str = "\u{f02e9}";
const G_CAMERA: &str = "\u{f0100}";
const G_WIFI: &str = "\u{f05a9}";
const G_WIFI_OFF: &str = "\u{f05aa}";
const G_VPN: &str = "\u{f0582}";
const G_ETHERNET: &str = "\u{f0200}";
const G_EYE: &str = "\u{f0208}";
const G_SHELF: &str = "\u{f003c}";
const G_CLOSE: &str = "\u{f0156}";
const G_DEX: &str = "\u{f0440}";
const G_GAME: &str = "\u{f0297}";
const G_FOLDER: &str = "\u{f024b}";
const G_FILE: &str = "\u{f0214}";
const G_GITHUB: &str = "\u{f02a4}";
const G_SWORDS: &str = "\u{f0787}";
const G_BLUETOOTH: &str = "\u{f00af}";
const G_MOUSE: &str = "\u{f037d}";
const G_PHONE: &str = "\u{f011c}";
const G_HEAT: &str = "\u{f050f}";
const G_SHIELD: &str = "\u{f0498}";
const G_KEYBOARD: &str = "\u{f030c}";
const G_PEN: &str = "\u{f03eb}";
const G_MONITOR: &str = "\u{f0379}";
const G_SPEAKER: &str = "\u{f04c3}";
const G_CIRCLE: &str = "\u{f0130}";
const G_CHECK_CIRCLE: &str = "\u{f05e0}";
const G_PLUS: &str = "\u{f0415}";
/// Recent copies shown a page on the Shelf tab, and the most kept.
const CLIP_ROWS: usize = 6;
const CLIPS_KEPT: usize = 20;
/// Sound outputs shown on the Controls tab.
const OUTPUTS_SHOWN: usize = 6;
/// Rows in the Storage page's folder list.
const SCAN_ROWS: usize = 9;
/// A wild Pokémon wanders by (main island's timer).
const T_WILD: usize = 13;
/// A single click waits this long in case it's the first half of a double-click.
const T_CLICK: usize = 14;
/// Checks every so often that no other app's window has been put in front of the island.
const T_ONTOP: usize = 15;
const ONTOP_MS: u32 = 1500;
/// The demo's next scene.
const T_DEMO: usize = 16;
static DEMO_STEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// How long a wild Pokémon takes to cross the pill (it stops halfway to look around).
const WILD_WALK: f32 = 24.0;
/// One shrink of the catching ring.
const RING_SECS: f32 = 1.3;
/// HP at or below this: fainted.
const FAINT: f32 = 0.005;
const HATCH_SECS: f32 = 3.6;
/// How long a battle on the pill lasts.
const FIGHT_SECS: f32 = 3.0;
/// The break reminder slider goes up to this many minutes.
const BREAK_MAX: f32 = 120.0;
/// Pokédex grid columns.
const DEX_COLS: usize = 12;
/// Height of a Pokédex grid cell.
const DEX_CELL: f32 = 36.0;
/// Pokédex grid rows a page.
const DEX_ROWS: usize = 6;
/// A Pokémon Egg, in pixels: outline, shell and spots.
const EGG: [&str; 9] = ["..XXX..", ".XWWWX.", "XWWSWWX", "XWSSWWX", "XWWWWSX", "XWWWSSX", "XWWWWWX", ".XWWWX.", "..XXX.."];
const HEART: Rgba = Rgba::rgb(255, 105, 140);
/// Once a minute: has the screen been in use long enough for an eye break?
const T_BREAK: usize = 12;
const BREAK_SECS: f32 = 20.0;
/// The privacy dots, like the phone's: green = camera, orange = microphone.
const CAM_GREEN: Rgba = Rgba::rgb(48, 209, 88);
const MIC_ORANGE: Rgba = Rgba::rgb(255, 159, 10);

enum Compact {
    Waiting(usize),
    Working(usize),
    Media,
    Usage,
    Idle,
}

struct App {
    hwnd: HWND,
    r: Renderer,
    theme: Theme,
    cfg: Config,
    scale: f32,
    pill_h: f32,
    monitor: HMONITOR,
    /// Island surface above the content when it hangs from the top edge (0 = floating).
    neck: f32,

    /// The surface's colour of the moment (album art, agent, alert), eased in r, g, b.
    mood: [Spring; 3],
    /// Cursor light: strength (0..1) and where the cursor is (logical px).
    glow: Spring,
    mouse: (f32, f32),
    /// Music loudness, smoothed (drives the tide and the equaliser).
    peak: Spring,
    /// Click ripples: where and when.
    ripples: Vec<(f32, f32, Instant)>,
    /// When Pip was last poked (it hops).
    hop: Option<Instant>,
    /// Kick on each beat of the music (1 at the beat, decaying), and the slow level it's
    /// measured against.
    beat: f32,
    beat_slow: f32,
    last_beat: Instant,
    /// Swipe to skip: horizontal scroll so far, the content's slide (logical px), and a short
    /// lock after a skip so one swipe skips once.
    swipe_acc: f32,
    swipe: Spring,
    swipe_lock: Option<Instant>,
    /// When the island started (no peeks for what was already true at start-up).
    born: Instant,
    /// Downloaded animated Pokémon, loaded on first use: (frame w, frame h, frame times, strip).
    strips: HashMap<String, Option<Sprite>>,
    last_clip: Option<Instant>,
    /// Last (on charger, battery %) seen, and the low-battery level last warned about.
    power: Option<(bool, u8)>,
    low_warned: u8,

    w: Spring,
    h: Spring,
    rad: Spring,
    alpha: Spring,
    mode: Mode,
    last_rgn: (i32, i32, i32, i32),
    /// The islands on the other screens.
    views: Vec<View>,
    /// Right-drag on the pill to scrub the song: where the drag started (logical x, seconds
    /// into the track), and whether it has moved far enough to count as scrubbing.
    scrub: Option<(f32, f64, bool)>,
    /// Apps using the microphone / camera now (None until the first look).
    mic_apps: Vec<String>,
    cam_apps: Vec<String>,
    privacy_seen: bool,
    net: Option<crate::net::Net>,
    /// When the current stretch of using the computer began.
    use_since: Instant,
    /// Files waiting on the shelf, and the last few things copied (newest first).
    shelf: Vec<PathBuf>,
    clips: VecDeque<HistClip>,
    /// Files are being dragged over an island.
    drag_over: bool,
    /// Pressed on a shelf file (index, where): moving far enough drags it out.
    shelf_press: Option<(usize, f32, f32)>,
    shelf_dragged: bool,
    /// The copy just put back on the clipboard shows a tick for a moment.
    clip_flash: Option<(usize, Instant)>,
    /// The page of recent copies shown.
    clip_page: usize,
    /// The sound outputs (Controls tab).
    outputs: Vec<crate::devices::Output>,
    /// Home tab notes, the one being typed, whether the keyboard is typing into it, and the
    /// first half of an emoji typed.
    notes: Vec<crate::notes::Note>,
    note_draft: String,
    note_typing: bool,
    note_hi: Option<u16>,
    /// The Pokédex tab shows the bag (balls, challenges, outbreak, battles).
    dex_bag: bool,
    /// The team's XP and the Pokédex.
    dex: crate::dex::Dex,
    wild: Option<Wild>,
    /// Gone fishing (beside the mascot), and where the rod is drawn (click the bobber).
    fishing: Option<Fishing>,
    fish_rect: Option<R>,
    /// The Pokédex tab's legend quests page.
    dex_quests: bool,
    /// Hearts floating up from the Pokémon (x, y, born), and "+XP" pops (amount, when).
    hearts: Vec<(f32, f32, Instant)>,
    /// An egg is hatching (since when).
    hatch: Option<Instant>,
    /// A calendar event starting soon: its title and minutes left (for the sign).
    meeting: Option<(String, u32)>,
    /// A break reminder you set on the Controls tab.
    break_at: Option<Instant>,
    /// System tab: top apps by memory instead of CPU, the rows drawn, and the one waiting
    /// for a second click to close (by exe).
    sys_by_mem: bool,
    /// The System tab's page (0 the overview, 1 CPU, 2 Memory, 3 GPU, 4 Storage) and the
    /// Storage page's folder list page.
    sys_page: u8,
    scan_page: usize,
    sys_top: Vec<crate::procs::Proc>,
    kill_armed: Option<String>,
    /// The Pokédex tab shows the trainer card instead of the grid.
    dex_card: bool,
    /// The Pokédex's Forms view (regional forms, Megas, Gigantamax), and how many pages the
    /// grid has right now.
    dex_forms: bool,
    /// The Pokédex shows the shiny one of every Pokémon and form.
    dex_shiny: bool,
    dex_pages: usize,
    /// The Game tab's library and the player that's running (with the game's title).
    games: Vec<crate::games::Game>,
    game_child: Option<(std::process::Child, String)>,
    /// The Pokédex grid's page (72 a page).
    dex_page: usize,
    /// A catch may have something to evolve (checked once its card is up).
    pending_evolution: bool,
    xp_pops: Vec<(u32, Instant)>,
    /// The pointer has been resting on the Pokémon since (petting).
    pet_at: Option<Instant>,
    last_heart: Instant,
    pet_xp_at: Option<Instant>,
    poke_xp_at: Option<Instant>,
    /// First frames of sprites for the Pokédex grid (bitmap, width, height).
    dex_thumbs: HashMap<String, Option<(ID2D1Bitmap1, f32, f32)>>,
    /// A single click waiting to see if it's a double-click: Some(true) = open, Some(false) = close.
    pending_click: Option<bool>,
    /// The button-up that ends a double-click isn't a click.
    swallow_up: bool,
    /// Where the Pokémon and a wild one were last drawn (per island).
    mon_rect: Option<R>,
    wild_rect: Option<R>,
    /// Where scrubbing currently points (seconds).
    scrub_to: f64,
    /// Drawing failed (graphics device lost): rebuild every island's renderer.
    device_lost: bool,
    /// Areas drawn in the last two frames (one per swap-chain buffer), for dirty rects.
    drawn: [R; 2],
    dirty_rect: R,
    width_cache: HashMap<(String, u32, u32), f32>,
    last: Instant,
    t0: Instant,
    dirty: bool,
    visible: bool,
    fullscreen: bool,

    hover: bool,
    tracking: bool,
    expanded: bool,
    buttons: Vec<(R, Btn)>,
    hot: Option<Btn>,
    tab: Tab,
    /// Animated width of each tab button (the selected one widens to show its name).
    tab_w: [Spring; TABS.len()],

    sys: Option<Sys>,
    sys_on: bool,
    cpu_hist: VecDeque<f32>,
    mem_hist: VecDeque<f32>,
    gpu_hist: VecDeque<f32>,
    weather: Option<Weather>,
    ctl_dnd: Option<bool>,
    ctl_dark: bool,
    ctl_mic: Option<(f32, bool)>,
    brightness: Option<u8>,
    bright_tx: Sender<u8>,
    dragging: Option<Slider>,
    /// Slider track, x from..to (logical px).
    track: (f32, f32),
    armed: Option<PowerAct>,
    user: String,

    osd: Option<Osd>,
    last_vol: Option<(f32, bool)>,
    card: Option<Card>,
    card_since: Instant,
    card_bmp: Option<ID2D1Bitmap1>,
    queue: VecDeque<Card>,

    media: Option<MediaInfo>,
    playing: bool,
    /// When the music was paused (None while playing).
    paused_at: Option<Instant>,
    art: Option<Art>,
    art_bmp: Option<ID2D1Bitmap1>,
    tint: Option<Rgba>,
    timeline: Option<(f64, f64, Instant)>,

    agents: Vec<Agent>,
    decisions: Vec<Decision>,

    /// The wallpaper the accent was last picked from, and that accent.
    wall_key: Option<wallpaper::Key>,
    /// Wallpaper Engine's wallpapers (one title per screen); empty without it.
    wpe_titles: Vec<String>,
    /// When "Another" was last clicked (its button shows "Changing…" for a moment).
    wpe_asked: Option<Instant>,
    wall_accent: Option<Rgba>,

    usage: Vec<Usage>,
    /// Limit alerts already shown: (level, when that limit resets). Saved, so a restart
    /// doesn't show the same alert again.
    usage_alerted: HashMap<String, (u8, Option<i64>)>,
    usage_tx: Sender<UsageCmd>,

    cal: Calendar,
    cal_today: Vec<String>,
    cal_next: Option<(String, Date)>,
    /// The pill's ("Sun 15 Nov", "1:42 PM"), refreshed on the minute.
    clock: (String, String),
    today: Date,

    audio: Option<Audio>,
    media_tx: Sender<MediaCmd>,
    rx: Receiver<Event>,
}

struct Placement {
    x: i32,
    y: i32,
    pw: i32,
    ph: i32,
    scale: f32,
    pill_h: f32,
    /// Hanging from the top edge: the island surface above the content (logical px).
    neck: f32,
    monitor: HMONITOR,
}

fn placement(theme: &Theme, cfg: &Config) -> Placement {
    placement_on(unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }, theme, cfg)
}

/// Every screen but `main`.
fn other_monitors(main: HMONITOR) -> Vec<HMONITOR> {
    unsafe extern "system" fn add(m: HMONITOR, _: HDC, _: *mut RECT, list: LPARAM) -> windows::core::BOOL {
        (*(list.0 as *mut Vec<HMONITOR>)).push(m);
        windows::core::BOOL(1)
    }
    let mut all: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(add), LPARAM(&mut all as *mut _ as isize));
    }
    all.retain(|m| *m != main);
    all
}

/// The island on another screen: everything that's its own (where it is, its shape and
/// animations, hover, whether it's open and on which tab). While that island is being
/// drawn or handles input, these are swapped into `App`, so all the code works on it as if
/// it were the only island; music, agents, cards and the rest stay shared.
struct View {
    hwnd: HWND,
    r: Renderer,
    scale: f32,
    pill_h: f32,
    monitor: HMONITOR,
    neck: f32,
    mood: [Spring; 3],
    glow: Spring,
    mouse: (f32, f32),
    peak: Spring,
    ripples: Vec<(f32, f32, Instant)>,
    beat: f32,
    beat_slow: f32,
    last_beat: Instant,
    swipe: Spring,
    w: Spring,
    h: Spring,
    rad: Spring,
    alpha: Spring,
    mode: Mode,
    last_rgn: (i32, i32, i32, i32),
    drawn: [R; 2],
    dirty_rect: R,
    last: Instant,
    visible: bool,
    fullscreen: bool,
    hover: bool,
    tracking: bool,
    expanded: bool,
    buttons: Vec<(R, Btn)>,
    hot: Option<Btn>,
    tab: Tab,
    tab_w: [Spring; TABS.len()],
    dragging: Option<Slider>,
    track: (f32, f32),
    sys_on: bool,
    mon_rect: Option<R>,
    wild_rect: Option<R>,
}

fn alerts_path() -> PathBuf {
    util::config_dir().join("usage-alerts.json")
}

/// The limit alerts shown before, minus those whose limit has reset since.
fn load_alerts() -> HashMap<String, (u8, Option<i64>)> {
    let now = usage::now_unix();
    let Ok(text) = std::fs::read_to_string(alerts_path()) else { return HashMap::new() };
    let Ok(serde_json::Value::Object(m)) = serde_json::from_str::<serde_json::Value>(&text) else { return HashMap::new() };
    m.into_iter()
        .filter_map(|(k, v)| Some((k, (v["level"].as_u64()? as u8, v["resets_at"].as_i64()))))
        .filter(|(_, (_, r))| r.is_none_or(|r| r > now))
        .collect()
}

fn save_alerts(a: &HashMap<String, (u8, Option<i64>)>) {
    let m: serde_json::Map<String, serde_json::Value> =
        a.iter().map(|(k, (lv, r))| (k.clone(), serde_json::json!({ "level": lv, "resets_at": r }))).collect();
    let _ = std::fs::write(alerts_path(), serde_json::to_string_pretty(&serde_json::Value::Object(m)).unwrap_or_default());
}

fn main_hwnd() -> HWND {
    HWND(MAIN_HWND.load(Ordering::SeqCst) as *mut _)
}

fn placement_on(monitor: HMONITOR, theme: &Theme, cfg: &Config) -> Placement {
    unsafe {
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut mi);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let scale = dx as f32 / 96.0;
        let rc = mi.rcMonitor;
        let top = cfg.top_offset.unwrap_or(theme.bar_top + 1.0);
        let pill_h = cfg.pill_height.unwrap_or(theme.bar_height - 2.0).max(20.0);
        let pw = (WIN_W * scale).round() as i32;
        let ph = (WIN_H * scale).round() as i32;
        // Attached, the window starts at the screen's edge and the gap becomes island.
        let (y, neck) = if cfg.attach_to_top { (rc.top, top.max(0.0)) } else { (rc.top + (top * scale).round() as i32, 0.0) };
        Placement {
            x: rc.left + ((rc.right - rc.left) - pw) / 2,
            y,
            pw,
            ph,
            scale,
            pill_h,
            neck,
            monitor,
        }
    }
}

/// Lower-case exe name of the process that owns a window.
fn window_exe(hwnd: HWND) -> String {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(p);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().unwrap_or("").to_lowercase()
    }
}

fn foreground_exe() -> (String, HWND) {
    let fg = unsafe { GetForegroundWindow() };
    (window_exe(fg), fg)
}

/// Whether a visible window of another app sits above `h` and overlaps it. Menus and
/// tooltips don't count (they close by themselves), nor do windows Windows has hidden.
fn covered(h: HWND) -> bool {
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    unsafe {
        if !IsWindowVisible(h).as_bool() {
            return false;
        }
        let mut me = RECT::default();
        if GetWindowRect(h, &mut me).is_err() {
            return false;
        }
        let own = GetCurrentProcessId();
        let mut w = GetWindow(h, GW_HWNDPREV).unwrap_or_default();
        let mut steps = 0;
        while !w.is_invalid() && steps < 400 {
            steps += 1;
            let mut r = RECT::default();
            let mut pid = 0u32;
            GetWindowThreadProcessId(w, Some(&mut pid));
            if pid != own
                && IsWindowVisible(w).as_bool()
                && GetWindowRect(w, &mut r).is_ok()
                && r.left < me.right
                && me.left < r.right
                && r.top < me.bottom
                && me.top < r.bottom
            {
                let mut class = [0u16; 64];
                let n = GetClassNameW(w, &mut class).max(0) as usize;
                let class = String::from_utf16_lossy(&class[..n]);
                let mut cloaked = 0u32;
                let _ = DwmGetWindowAttribute(w, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
                if cloaked == 0 && !matches!(class.as_str(), "#32768" | "tooltips_class32" | "SysShadow") {
                    return true;
                }
            }
            w = GetWindow(w, GW_HWNDPREV).unwrap_or_default();
        }
        false
    }
}

/// Another app came to the front: make sure it didn't land on top of the island.
unsafe extern "system" fn on_foreground(_: windows::Win32::UI::Accessibility::HWINEVENTHOOK, _: u32, _: HWND, _: i32, _: i32, _: u32, _: u32) {
    with_app(|a| a.keep_on_top());
}

fn bring_forward(h: HWND) {
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let _ = SetForegroundWindow(h);
    }
}

fn focus_terminal() {
    if let Ok(h) = unsafe { FindWindowW(w!("CASCADIA_HOSTING_WINDOW_CLASS"), None) } {
        bring_forward(h);
    }
}

/// A desktop app's main window: a visible, titled Electron window owned by `exe`.
fn app_window(exe: &str) -> Option<HWND> {
    struct Search<'a> {
        exe: &'a str,
        found: Option<HWND>,
    }
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> windows::core::BOOL {
        let s = &mut *(lp.0 as *mut Search);
        let mut class = [0u16; 64];
        let n = GetClassNameW(h, &mut class);
        if IsWindowVisible(h).as_bool()
            && GetWindowTextLengthW(h) > 0
            && String::from_utf16_lossy(&class[..n.max(0) as usize]) == "Chrome_WidgetWin_1"
            && window_exe(h) == s.exe
        {
            s.found = Some(h);
            return false.into();
        }
        true.into()
    }
    let mut s = Search { exe, found: None };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut s as *mut _ as isize));
    }
    s.found
}

/// Bring an app's window forward, or open it (from the tray) by its app id.
fn focus_app(exe: &str, app_id: &str) {
    match app_window(exe) {
        Some(h) => bring_forward(h),
        None => unsafe {
            let args = HSTRING::from(format!("shell:AppsFolder\\{app_id}"));
            ShellExecuteW(None, w!("open"), w!("explorer.exe"), &args, None, SW_SHOWNORMAL);
        },
    }
}

fn focus(f: Focus) {
    match f {
        Focus::No => {}
        Focus::Terminal => focus_terminal(),
        Focus::Claude => focus_app("claude.exe", "Claude_pzs8sxrjxfjjc!Claude"),
        // The Codex desktop app runs as ChatGPT.exe.
        Focus::Codex => focus_app("chatgpt.exe", "OpenAI.Codex_2p2nqsd0c76g0!App"),
    }
}

/// Is the process with this id still running?
fn alive(pid: u32) -> bool {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else { return false };
        let running = WaitForSingleObject(h, 0) == WAIT_TIMEOUT;
        let _ = CloseHandle(h);
        running
    }
}

/// Time left on a decision card: seconds for the last two minutes, minutes before that.
fn countdown(left: Duration) -> String {
    let s = left.as_secs();
    if s >= 120 { format!("{}m", (s + 59) / 60) } else { format!("{s}s") }
}

impl App {
    // ---------- state ----------

    fn compact(&self) -> Compact {
        if let Some(i) = self.agents.iter().position(|a| a.phase == Phase::Waiting) {
            return Compact::Waiting(i);
        }
        // Music shows only while it plays (and briefly after a pause, so a skip or a seek
        // doesn't flicker); paused, the pill goes back to the date and time.
        if self.media.is_some() && (self.playing || self.paused_at.is_some_and(|t| t.elapsed() < PAUSE_GRACE)) {
            return Compact::Media;
        }
        if self.cfg.agent_status_in_pill {
            if let Some(i) = self.agents.iter().position(|a| a.phase == Phase::Working) {
                return Compact::Working(i);
            }
        }
        if self.cfg.clock_in_pill || self.weather_chip().is_some() || !self.usage_chips().is_empty() {
            return Compact::Usage;
        }
        Compact::Idle
    }

    fn any_working(&self) -> bool {
        self.agents.iter().any(|a| a.phase == Phase::Working)
    }

    /// The little "agent working" spinner beside the music (only when agent status shows).
    fn media_spinner(&self) -> bool {
        self.cfg.agent_status_in_pill && self.any_working()
    }


    /// Text width, measured once per distinct string (called every animation frame).
    fn text_width(&mut self, s: &str) -> f32 {
        self.text_width_at(s, 12.0, 600)
    }

    fn text_width_at(&mut self, s: &str, size: f32, weight: u32) -> f32 {
        let key = (s.to_string(), (size * 10.0) as u32, weight);
        if let Some(w) = self.width_cache.get(&key) {
            return *w;
        }
        if self.width_cache.len() > 64 {
            self.width_cache.clear();
        }
        let w = self.r.measure(s, size, weight);
        self.width_cache.insert(key, w);
        w
    }

    /// The (name, percent text, 5-hour fraction, weekly fraction) chips shown in the idle pill.
    fn usage_chips(&self) -> Vec<(&'static str, String, f32, Option<f32>)> {
        if !self.cfg.usage_in_pill {
            return Vec::new();
        }
        self.usage
            .iter()
            .filter(|u| !u.limits.is_empty())
            .map(|u| {
                let s = u.limits[0].used;
                (u.provider, format!("{}%", (s * 100.0).round()), s, u.limits.get(1).map(|l| l.used))
            })
            .collect()
    }

    fn weather_chip(&self) -> Option<(&'static str, String)> {
        let w = self.weather.as_ref().filter(|_| self.cfg.weather_in_pill)?;
        Some((weather::describe(w.code, w.day).0, format!("{}°", w.temp.round())))
    }

    /// Night, and the sky isn't hidden by rain, snow or fog: the pill's weather chip shows
    /// tonight's moon (dimmer when it's overcast). With no weather yet, it goes by the clock.
    /// Returns how bright to draw it.
    fn moon_out(&self) -> Option<f32> {
        if !self.cfg.weather_in_pill {
            return None;
        }
        match self.weather.as_ref() {
            Some(w) if !w.day && matches!(w.code, 0..=2) => Some(1.0),
            Some(w) if !w.day && w.code == 3 => Some(0.6),
            Some(_) => None,
            None => {
                let h = calendar::local_now().h;
                (h >= 19 || h < 6).then_some(1.0)
            }
        }
    }

    /// CPU / GPU / memory load colour.
    fn load_color(&self, f: f32) -> Rgba {
        if f >= 0.9 {
            self.theme.bad
        } else if f >= 0.7 {
            self.theme.accent.mix(self.theme.bad, 0.5)
        } else {
            self.theme.accent
        }
    }

    fn level_color(&self, used: f32) -> Rgba {
        if used >= self.cfg.usage_critical {
            self.theme.bad
        } else if used >= self.cfg.usage_watch {
            self.theme.warn
        } else {
            self.theme.good
        }
    }

    /// The mascot's mood beside the music.
    fn media_mood(&self) -> PipMood {
        if self.playing { PipMood::Music } else { self.pip_mood() }
    }

    /// Which Pokémon: Rayquaza is the island's own colour; with music, the album art's.
    fn mascot_mon(&self, m: PipMood) -> &'static pokemon::Mon {
        match (m, self.tint) {
            (PipMood::Music, Some(c)) => pokemon::for_colour([c.r, c.g, c.b]),
            _ => &pokemon::RAYQUAZA,
        }
    }

    /// The sprite the team member for this mood shows at the team's level.
    fn mascot_sprite(&self, m: PipMood) -> String {
        let mon = self.mascot_mon(m);
        // In Rayquaza's place: the Pokémon that came with the weather.
        if mon.name == "Rayquaza" {
            if let Some(b) = self.buddy_sprite() {
                return b;
            }
        }
        if mon.name == "Rayquaza" && !self.dex.mascot.is_empty() {
            return self.dex.mascot.clone();
        }
        pokemon::team_sprite(mon, self.dex.level())
    }

    /// It stands on the pill's floor and may reach up into the island above it.
    fn mascot_box_h(&self) -> f32 {
        self.pill_h - 2.0 + self.neck * 0.8
    }

    /// The mascot's mood in the pill right now.
    fn compact_mood(&self) -> PipMood {
        match self.compact() {
            Compact::Media => self.media_mood(),
            Compact::Waiting(_) => PipMood::Needs,
            _ => self.pip_mood(),
        }
    }

    /// The real animated sprite's size in the pill (logical width, height), loading it the
    /// first time. None when it hasn't been downloaded.
    fn sprite_box(&mut self, name: &str, box_h: f32) -> Option<(f32, f32)> {
        let stale = match self.strips.get(name) {
            None => true,
            Some(Some(sp)) => (sp.box_h - box_h).abs() > 0.5 || sp.scale != self.scale,
            Some(None) => false,
        };
        if stale {
            let loaded = self.load_sprite(name, box_h);
            self.strips.insert(name.to_string(), loaded);
        }
        let sp = self.strips.get(name)?.as_ref()?;
        Some((sp.dw, sp.dh))
    }

    /// Read a downloaded sprite and size it for the pill once: whole screen pixels per sprite
    /// pixel when it's big enough (crisp), otherwise shrunk smoothly to its exact on-screen size.
    /// Either way drawing a frame is then a plain pixel copy, which keeps the animation cheap.
    fn load_sprite(&self, name: &str, box_h: f32) -> Option<Sprite> {
        let s = pokemon::strip_named(name)?;
        let (w, h) = (s.w as f32, s.h as f32);
        let dev = (box_h / h).min(MON_MAX_W / w) * self.scale;
        let frames = s.delays.len();
        let (art, fw, fh, k) = if dev >= 1.0 {
            (s.art, s.w, s.h, dev.floor())
        } else {
            let (fw, fh) = ((w * dev).round().max(1.0) as u32, (h * dev).round().max(1.0) as u32);
            (pokemon::shrink(&s.art, s.w, s.h, frames, fw, fh), fw, fh, 1.0)
        };
        let bmp = self.r.bitmap(&art)?;
        Some(Sprite {
            fw: fw as f32,
            fh: fh as f32,
            dw: fw as f32 * k / self.scale,
            dh: fh as f32 * k / self.scale,
            delays: s.delays,
            bmp,
            box_h,
            scale: self.scale,
        })
    }

    /// How much room the mascot takes in the pill.
    fn mascot_w(&mut self, m: PipMood) -> f32 {
        let w = self.mascot_w_bare(m);
        if w > 0.0 && self.fishing.is_some() { w + FISH_W } else { w }
    }

    fn mascot_w_bare(&mut self, m: PipMood) -> f32 {
        if !self.cfg.mascot {
            return 0.0;
        }
        if self.cfg.mascot_pokemon {
            let (name, bh) = (self.mascot_sprite(m), self.mascot_box_h());
            if let Some((dw, _)) = self.sprite_box(&name, bh) {
                let egg = if self.dex.egg.is_some() || self.hatch.is_some() { 11.0 } else { 0.0 };
                let sign = match &self.meeting {
                    Some((_, left)) => self.text_width_at(&sign_text(*left), 9.5, 700) + 14.0,
                    None => 0.0,
                };
                return dw.ceil() + 4.0 + egg + sign;
            }
        }
        PIP_W
    }

    fn compact_width(&mut self) -> f32 {
        let mood = self.pip_mood();
        let pip = self.mascot_w(mood);
        match self.compact() {
            Compact::Waiting(i) => {
                let t = format!("{} needs you", self.agents[i].label);
                let lead = if self.cfg.mascot { self.mascot_w(PipMood::Needs) + 4.0 } else { 10.0 + 8.0 };
                12.0 + lead + self.text_width(&t).min(230.0) + 14.0
            }
            Compact::Working(i) => {
                let (label, act) = (self.agents[i].label.clone(), self.agents[i].activity.clone());
                let mut w = 10.0 + 16.0 + 8.0 + self.text_width(&label).min(PILL_LABEL_W) + 14.0;
                if !act.is_empty() {
                    w += 8.0 + self.text_width_at(&act, 12.0, 400).min(PILL_ACT_W);
                }
                w
            }
            Compact::Media => {
                let title = self.media.as_ref().map(|m| m.title.clone()).unwrap_or_default();
                let spin = if self.media_spinner() { 20.0 } else { 0.0 };
                let mood = self.media_mood();
                6.0 + 22.0 + 8.0 + self.text_width(&title).min(160.0) + 10.0 + 14.0 + 12.0 + spin + self.mascot_w(mood)
            }
            Compact::Usage => {
                let chips = self.usage_chips();
                let mut w = pip;
                if self.cfg.clock_in_pill {
                    let (date, time) = self.clock.clone();
                    w += 12.0 + self.text_width_at(&date, 11.0, 500) + 6.0 + self.text_width_at(&time, 12.0, 600);
                }
                if let Some((_, temp)) = self.weather_chip() {
                    w += 12.0 + 16.0 + 4.0 + self.text_width_at(&temp, 12.0, 600);
                } else if self.moon_out().is_some() {
                    w += 12.0 + 16.0;
                }
                for (name, pct, _, _) in &chips {
                    w += 12.0 + 16.0 + 6.0 + self.text_width_at(name, 11.0, 500) + 4.0 + self.text_width_at(pct, 12.0, 600);
                }
                w + 14.0
            }
            Compact::Idle => 24.0 + pip,
        }
    }

    /// What Pip is up to.
    fn pip_mood(&self) -> PipMood {
        if self.agents.iter().any(|a| a.phase == Phase::Waiting) {
            PipMood::Needs
        } else if self.playing && self.media.is_some() {
            PipMood::Music
        } else if matches!(calendar::local_now().h, 23 | 0..=5) {
            PipMood::Sleep
        } else {
            PipMood::Calm
        }
    }

    /// Pip, the palm-tree islet: 12 x 9 pixels on a live water line, `x` from the left and
    /// centred on `cy`. Fronds sway (dancing to the music), it hops when there's news, raises
    /// a "!" when something needs you and dozes late at night. Drawn on whole device pixels.
    /// The weather over Pip, in the same chunky pixels: `x` is Pip's left edge, `base` the
    /// top of its sprite (before hopping), `px` one pixel.
    fn draw_sky(&mut self, x: f32, base: f32, px: f32, t: f32, sky: Sky) {
        let dot = |r: &Renderer, cx: f32, cy: f32, c: Rgba| r.rect(R::xywh(x + cx * px, base + cy * px, px, px), c);
        let shape = |r: &Renderer, rows: &[&str], ox: f32, oy: f32, c: Rgba| {
            for (j, row) in rows.iter().enumerate() {
                for (i, ch) in row.chars().enumerate() {
                    if ch != '.' {
                        dot(r, ox + i as f32, oy + j as f32, c);
                    }
                }
            }
        };
        let cloud = |r: &Renderer, c: Rgba| {
            // Drifts a pixel back and forth.
            let drift = ((t * 0.35).sin() * 1.0).round();
            shape(r, &[".CCC..", "CCCCC."], -2.0 + drift, -2.5, c);
        };
        let r = &self.r;
        match sky {
            Sky::None => {}
            Sky::Sun => {
                let sun = Rgba::rgb(0xff, 0xd4, 0x5c);
                shape(r, &[".S.", "SSS", ".S."], -2.0, -3.0, sun);
                for (dx, dy) in [(-1.0, -4.5), (-1.0, 0.5), (-3.5, -2.0), (1.5, -2.0)] {
                    dot(r, dx, dy, sun.a(0.45));
                }
            }
            Sky::Moon => {
                // Tonight's moon, five pixels across: lit where the real one is, the rest faint.
                let moon = Rgba::rgb(0xe8, 0xe4, 0xc9);
                let (age, south) = (pokemon::moon_age(crate::dex::now_unix()), southern());
                for j in 0..5 {
                    for i in 0..5 {
                        let (nx, ny) = ((i as f64 + 0.5) / 2.5 - 1.0, (j as f64 + 0.5) / 2.5 - 1.0);
                        if nx * nx + ny * ny > 1.1 {
                            continue;
                        }
                        let lit = pokemon::moon_lit(age, nx, ny, south);
                        dot(r, -3.0 + i as f32, -5.0 + j as f32, if lit { moon } else { moon.a(0.16) });
                    }
                }
                dot(r, 3.0, -5.5, moon.a(0.55));
                dot(r, 5.0, -3.5, moon.a(0.35));
            }
            Sky::Cloud => cloud(r, Rgba::rgb(0xc9, 0xd1, 0xd9).a(0.9)),
            Sky::Rain | Sky::Storm => {
                let dark = sky == Sky::Storm;
                cloud(r, if dark { Rgba::rgb(0x8a, 0x93, 0xa0) } else { Rgba::rgb(0xb4, 0xbe, 0xc8) });
                let drop = Rgba::rgb(0x7a, 0xb8, 0xf5);
                // Three drops falling from the cloud to the sea, each on its own beat.
                for (i, col) in [-1.0f32, 1.0, 3.0].iter().enumerate() {
                    let u = (t * 1.1 + i as f32 * 0.37).fract();
                    let y = -0.5 + u * 9.5;
                    dot(r, *col, y.floor(), drop.a(0.85 * (1.0 - u * 0.5)));
                }
                // Now and then, lightning.
                if dark && (t % 4.0) < 0.14 {
                    shape(r, &[".L", "L.", ".L"], 4.0, -1.0, Rgba::rgb(0xff, 0xe2, 0x6e));
                }
            }
            Sky::Snow => {
                cloud(r, Rgba::rgb(0xd6, 0xdc, 0xe2));
                for (i, col) in [-1.0f32, 1.5, 3.5].iter().enumerate() {
                    let u = (t * 0.45 + i as f32 * 0.33).fract();
                    let wobble = ((t * 2.0 + i as f32).sin() * 0.8).round();
                    dot(r, col + wobble, (-0.5 + u * 9.5).floor(), Rgba::rgb(255, 255, 255).a(0.9 * (1.0 - u * 0.4)));
                }
            }
        }
    }

    fn draw_pip(&mut self, x: f32, cy: f32, t: f32, m: PipMood) {
        self.draw_pip_body(x, cy, t, m);
        if self.fishing.is_some() {
            let bare = self.mascot_w_bare(m);
            self.draw_fishing(x + bare, cy, t);
        }
    }

    /// Gone fishing: a rod, its line and a red-and-white bobber on the water. When something
    /// bites the bobber dips and a "!" shows: click it (or the mascot) to reel it in.
    fn draw_fishing(&mut self, x: f32, cy: f32, t: f32) {
        let now = Instant::now();
        self.tick_fishing(now);
        let Some(bite) = self.fishing.as_ref().map(|f| f.bite.map(|b| now.duration_since(b).as_secs_f32())) else { return };
        let s = self.scale;
        let snap = |v: f32| (v * s).round() / s;
        let px = (1.0 * s).round().max(1.0) / s;
        let top = cy - self.pill_h / 2.0;
        let floor = snap(cy + self.pill_h / 2.0 - 5.0);
        // The rod, from low by the mascot up to its tip.
        let rod = Rgba::rgb(0xb8, 0x8a, 0x56);
        let (ax, ay, bx, by) = (x + 1.0, floor - 2.0, x + 11.0, top + 6.0);
        for i in 0..=12 {
            let u = i as f32 / 12.0;
            self.r.rect(R::xywh(snap(ax + (bx - ax) * u), snap(ay + (by - ay) * u), px * 1.5, px * 1.5), rod);
        }
        // The bobber floats gently; on a bite it jerks under.
        let bob = match bite {
            Some(b) => 1.5 + (b * 28.0).sin().abs() * 1.5,
            None => (t * 2.2).sin(),
        };
        let (fx, fy) = (x + FISH_W - 6.0, floor + bob);
        let line = self.theme.subtext.a(0.55);
        for i in 0..=14 {
            let u = i as f32 / 14.0;
            let sag = (u * std::f32::consts::PI).sin() * 2.0;
            self.r.rect(R::xywh(snap(bx + (fx - bx) * u), snap(by + (fy - 3.0 - by) * u + sag), px, px), line);
        }
        // Ripples either side.
        let water = Rgba::rgb(0x5a, 0xbb, 0xdb);
        let k = 0.5 + 0.5 * (t * if bite.is_some() { 9.0 } else { 3.0 }).sin();
        self.r.rect(R::xywh(snap(fx - 7.0), snap(floor + 2.0), 3.0, px), water.a(0.25 + 0.35 * k));
        self.r.rect(R::xywh(snap(fx + 4.0), snap(floor + 2.0), 3.0, px), water.a(0.25 + 0.35 * (1.0 - k)));
        self.r.rect(R::xywh(snap(fx - 1.5), snap(fy - 3.0), 3.0, 2.0), Rgba::rgb(0xe8, 0x40, 0x40));
        self.r.rect(R::xywh(snap(fx - 1.5), snap(fy - 1.0), 3.0, 2.0), Rgba::rgb(0xf4, 0xf4, 0xf4));
        if bite.is_some() {
            let c = self.theme.warn;
            self.r.text("!", R::new(fx - 6.0, top, fx + 6.0, fy - 3.0), 12.0, 800, c, Align::Center);
        }
        self.fish_rect = Some(R::new(x, top, x + FISH_W, top + self.pill_h));
    }

    /// The bite comes when it's due; three that get away and you stop fishing.
    fn tick_fishing(&mut self, now: Instant) {
        let wet = self.conditions().wet();
        let Some(f) = self.fishing.as_mut() else { return };
        match f.bite {
            None if now >= f.bite_at => {
                f.bite = Some(now);
                self.hop = Some(now);
                self.dirty = true;
            }
            Some(b) if now.duration_since(b) > FISH_BITE => {
                f.bite = None;
                f.misses += 1;
                f.bite_at = now + fish_wait(wet);
                if f.misses >= 3 {
                    self.fishing = None;
                    self.fish_rect = None;
                    util::log("fishing: three got away, stopped");
                    let tint = self.theme.accent;
                    self.quick_card("fishing", G_FISH, tint, "Fishing", "Three got away…", "Click the bobber as soon as the \"!\" shows · right-click the island → Go fishing to try again", 5.0);
                }
            }
            _ => {}
        }
    }

    /// Go fishing (or stop).
    fn toggle_fishing(&mut self) {
        self.dirty = true;
        if self.fishing.take().is_some() {
            self.fish_rect = None;
            util::log("fishing: stopped");
            return;
        }
        let tint = self.theme.accent;
        if !self.cfg.mascot {
            self.quick_card("fishing", G_FISH, tint, "Fishing", "Fishing needs the mascot", "Turn on mascot in the settings file, then cast a line beside it", 4.0);
            return;
        }
        if let Some(w) = self.wild.as_ref() {
            let who = pokemon::pretty(w.name);
            self.quick_card("fishing", G_FISH, tint, "Fishing", &format!("A wild {who} is here already"), "Catch it (or let it go) first, then cast a line", 4.0);
            return;
        }
        let wet = self.conditions().wet();
        self.fishing = Some(Fishing { bite_at: Instant::now() + fish_wait(wet), bite: None, misses: 0 });
        util::log(&format!("fishing: cast a line{}", if wet { " (rain: quicker bites)" } else { "" }));
    }

    /// Reeled in: a water Pokémon, chosen by the weather and time like any visitor.
    fn reel_in(&mut self) {
        self.fishing = None;
        self.fish_rect = None;
        let c = self.conditions();
        let all = pokemon::fish_pool();
        let have: Vec<&'static str> = all.iter().copied().filter(|n| pokemon::has_sprite(n)).collect();
        let list = if have.is_empty() { all } else { have };
        if list.is_empty() {
            return;
        }
        let weight = |n: &str| pokemon::spawn_weight(&c, n) * pokemon::rarity(n);
        let mut pick = rand01() * list.iter().map(|n| weight(n)).sum::<f32>();
        let mut name = list[list.len() - 1];
        for n in &list {
            if pick < weight(n) {
                name = n;
                break;
            }
            pick -= weight(n);
        }
        let shiny = rand01() < 1.0 / self.shiny_odds(name) as f32 && pokemon::has_sprite(&format!("{name}-shiny"));
        let boosted = pokemon::weather_boosted(&c, name);
        *self.dex.seen.entry(name.to_string()).or_default() += 1;
        util::log(&format!("fishing: reeled in {name}{}{}", if shiny { " (shiny)" } else { "" }, if boosted { " (weather boost)" } else { "" }));
        self.wild = Some(Wild { name, shiny, legend: false, since: Instant::now(), from_left: false, catch: None, u_at_catch: 0.0, tries: 0, quality: 0.0, flee: false, ball: 0, battle: false, tired: false, fight: None, raid: false, boosted, fished: true });
        self.dex.visitor = Some(crate::dex::Visitor { name: name.to_string(), shiny, legend: false, battle: false, tired: false, raid: false, boosted, fished: true });
        self.dex.save();
        self.hop = Some(Instant::now());
        self.dirty = true;
    }

    fn draw_pip_body(&mut self, x: f32, cy: f32, t: f32, m: PipMood) {
        if self.cfg.mascot_pokemon {
            return self.draw_mon(x, cy, t, m);
        }
        const PIP: [&str; 9] = [
            "....GGGG....",
            "..GGGGGGGG..",
            ".GGg.cTc.gGG",
            "Gg....T...gG",
            "......T.....",
            ".....T......",
            ".....T......",
            "...sSSSSs...",
            ".sSSSSSSSSs.",
        ];
        let s = self.scale;
        let snap = |v: f32| (v * s).round() / s;
        let px = (PIP_PX * s).round().max(1.0) / s;
        let lvl = self.peak.v;
        let hop = self.hop.map(|h| (h.elapsed().as_secs_f32() / HOP.as_secs_f32()).min(1.0)).map(|u| (u * std::f32::consts::PI).sin() * 5.0).unwrap_or(0.0)
            // Pip bounces on the beat.
            + if m == PipMood::Music { self.beat * 2.5 } else { 0.0 };
        let base = snap(cy - 5.0 * px);
        let y0 = snap(base - hop);
        let x = snap(x);
        let sway = match m {
            PipMood::Music => (t * 2.6).sin() * (0.4 + 1.6 * lvl),
            PipMood::Sleep => 0.0,
            _ => (t * 1.3).sin() * 0.5,
        };
        let colour = |c: char| match c {
            'G' => Some(Rgba::rgb(0x7c, 0xcf, 0x74)),
            'g' => Some(Rgba::rgb(0x4e, 0x9b, 0x55)),
            'T' => Some(Rgba::rgb(0xb3, 0x80, 0x52)),
            'c' => Some(Rgba::rgb(0x84, 0x52, 0x31)),
            'S' => Some(Rgba::rgb(0xf0, 0xd9, 0x9c)),
            's' => Some(Rgba::rgb(0xc7, 0xa8, 0x6a)),
            _ => None,
        };
        for (row, line) in PIP.iter().enumerate() {
            // Fronds move most at the top; the trunk a little; the sand not at all.
            let (dx, dy) = match row {
                0..=3 => {
                    let k = (4 - row) as f32 / 4.0;
                    (sway * k * 1.5, if m == PipMood::Music { -lvl * 1.5 * k } else { 0.0 })
                }
                4..=6 => (sway * 0.3, 0.0),
                _ => (0.0, 0.0),
            };
            let (dx, dy) = ((dx * s).round() / s, (dy * s).round() / s);
            for (col, ch) in line.chars().enumerate() {
                if let Some(c) = colour(ch) {
                    self.r.rect(R::xywh(x + col as f32 * px + dx, y0 + row as f32 * px + dy, px, px), c);
                }
            }
        }
        // The sea stays put while Pip hops out of it, shimmering in the mood's colour.
        let water = Rgba::rgb(0x5a, 0xbb, 0xdb).mix(self.mood(), 0.3);
        for col in 0..12 {
            let shimmer = 0.5 + 0.5 * (col as f32 * 1.3 - t * 3.0).sin();
            self.r.rect(R::xywh(x + col as f32 * px, base + 9.0 * px, px, px), water.a(0.45 + 0.5 * shimmer));
        }
        if matches!(m, PipMood::Calm | PipMood::Sleep) {
            let sky = self.sky();
            self.draw_sky(x, base, px, t, sky);
        }
        self.draw_mood_mark(R::xywh(x + 11.0 * px, y0 - 3.0, 9.0, 10.0), t, m);
    }

    /// A Pokémon in the pill instead of the palm: Rayquaza in the island's own colour; with
    /// music, the one matching the album art. Always moving: Rayquaza slithers, the others
    /// breathe and blink, flames flicker, sparks flash, and everyone bounces to the beat.
    fn draw_mon(&mut self, x: f32, cy: f32, t: f32, m: PipMood) {
        let mon = self.mascot_mon(m);
        let keeper = mon.name == "Rayquaza" && !self.dex.mascot.is_empty();
        if !keeper && !self.dex.met.iter().any(|n| n == mon.name) {
            self.dex.met.push(mon.name.to_string());
            self.dex.save();
        }
        let s = self.scale;
        let snap = |v: f32| (v * s).round() / s;
        let music = m == PipMood::Music;
        // The real animated sprite, when it's been downloaded.
        let (name, bh) = (self.mascot_sprite(m), self.mascot_box_h());
        if let Some((dw, dh)) = self.sprite_box(&name, bh) {
            let hop = self.hop.map(|h| (h.elapsed().as_secs_f32() / HOP.as_secs_f32()).min(1.0)).map(|u| (u * std::f32::consts::PI).sin() * 5.0).unwrap_or(0.0)
                + if music { self.beat * 3.0 } else { 0.0 };
            // Out of HP (Claude's 5-hour limit is used up): it has fainted and lies still.
            let hp = self.hp_now();
            let fainted = hp.is_some_and(|h| h <= FAINT);
            // Asleep at night: it moves at half speed; left alone 5 minutes, it dozes off.
            let dozing = fainted || (m != PipMood::Needs && user_idle_ms() >= 5 * 60_000);
            let speed = if fainted { 0.0 } else if dozing { 0.25 } else if m == PipMood::Sleep { 0.5 } else { 1.0 };
            let floor = snap(cy + self.pill_h / 2.0 - 1.0);
            let dest = R::xywh(snap(x), snap(floor - dh - hop), snap(dw), snap(dh));
            self.mon_rect = Some(dest);
            // It turns to look at the pointer.
            let flip = self.hover && self.mouse.0 > dest.cx() + 8.0;
            if let Some(Some(sp)) = self.strips.get(&name) {
                let i = pokemon::frame_at(&sp.delays, (t * 1000.0 * speed) as u64);
                self.r.sprite_flip(&sp.bmp, dest, R::xywh(i as f32 * sp.fw, 0.0, sp.fw, sp.fh), true, flip, if fainted { 0.5 } else { 1.0 });
            }
            self.draw_mood_mark(R::xywh(dest.x1 - 7.0, (dest.y0 + 2.0).max(1.0), 9.0, 10.0), t, if dozing { PipMood::Sleep } else { m });
            self.draw_hearts(dest);
            self.draw_egg(dest, floor, t);
            self.draw_sign(dest, floor, t);
            return;
        }
        let px = (PIP_PX * s).round().max(1.0) / s;
        let rows = mon.rows.len() as f32;
        let lvl = self.peak.v;
        let music = m == PipMood::Music;
        let asleep = m == PipMood::Sleep;
        let hop = self.hop.map(|h| (h.elapsed().as_secs_f32() / HOP.as_secs_f32()).min(1.0)).map(|u| (u * std::f32::consts::PI).sin() * 5.0).unwrap_or(0.0)
            + if music { self.beat * 3.0 } else { 0.0 };
        // Breathing: a one-pixel rise and fall (quicker with music); Rayquaza floats instead.
        let lift = if asleep {
            0.0
        } else if mon.serpent {
            snap((t * 1.4).sin() * px)
        } else if ((t * if music { 2.6 } else { 1.3 }) as i64) % 2 == 1 {
            px
        } else {
            0.0
        };
        let top = snap(cy - rows * px / 2.0);
        let y0 = snap(top - hop - lift);
        let x = snap(x - 1.0);
        let frame = (t * 5.0) as i64;
        let blink = asleep || (t % 3.4) < 0.16;
        let hex = |v: u32| Rgba::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
        for (row, line) in mon.rows.iter().enumerate() {
            // Rayquaza's body ripples in a wave running from head to tail.
            let dx = if mon.serpent && !asleep {
                snap((t * 3.0 - row as f32 * 0.75).sin() * px * if music { 1.0 + lvl } else { 1.0 })
            } else {
                0.0
            };
            for (col, ch) in line.chars().enumerate() {
                let colour = match ch {
                    '.' => continue,
                    'E' if blink => Some(mon.blink),
                    'F' if frame % 2 == 1 => mon.colour('f'),
                    'f' if frame % 2 == 1 => mon.colour('F'),
                    '*' if frame % 3 != 0 => continue,
                    c => mon.colour(c),
                };
                let Some(v) = colour else { continue };
                let mut c = hex(v);
                if mon.name == "Umbreon" && ch == 'Y' {
                    c = c.a(0.5 + 0.5 * (t * 2.2).sin().abs());
                }
                self.r.rect(R::xywh(x + col as f32 * px + dx, y0 + row as f32 * px, px, px), c);
            }
        }
        if matches!(m, PipMood::Calm | PipMood::Sleep) {
            let sky = self.sky();
            self.draw_sky(x, top, px, t, sky);
        }
        let w = mon.width() as f32;
        self.draw_mood_mark(R::xywh(x + (w - 3.0) * px, y0 - 3.0, 9.0, 10.0), t, m);
    }

    /// The little mark by the mascot's head: "!" when you're needed, "z" asleep, "♪" with music.
    fn draw_mood_mark(&mut self, top_right: R, t: f32, m: PipMood) {
        let th = self.theme.clone();
        let lvl = self.peak.v;
        match m {
            PipMood::Needs => {
                let blink = 0.65 + 0.35 * (t * 5.0).sin().abs();
                self.r.fill_round(top_right, 3.0, th.accent.a(blink));
                self.r.text("!", top_right, 8.5, 800, th.bg, Align::Center);
            }
            PipMood::Sleep => {
                let u = (t * 0.45).fract();
                let r = R::xywh(top_right.x0, top_right.y0 + 3.0 - u * 6.0, 9.0, 10.0);
                self.r.text("z", r, 8.0 + 2.0 * u, 700, th.subtext.a(1.0 - u), Align::Center);
            }
            PipMood::Music => {
                let u = (t * 0.6).fract();
                let r = R::xywh(top_right.x0 + 1.0, top_right.y0 + 5.0 - u * 9.0, 9.0, 10.0);
                let note = self.tint.unwrap_or(th.accent);
                self.r.text("\u{266A}", r, 9.0, 600, note.a((1.0 - u) * (0.4 + 0.6 * lvl)), Align::Center);
            }
            PipMood::Calm => {}
        }
    }

    fn expanded_height(&self) -> f32 {
        let content = match self.tab {
            Tab::Media => match self.media {
                Some(_) => 66.0 + if self.timeline.is_some() { 14.0 } else { 0.0 },
                None => 26.0,
            },
            Tab::Usage => self.usage.len().max(1) as f32 * 34.0,
            Tab::Calendar => 26.0 + if self.cal_next.is_some() { 26.0 } else { 0.0 },
            Tab::System => {
                if self.sys.is_none() {
                    26.0
                } else {
                    self.sys_page_height()
                }
            }
            Tab::Weather => {
                if self.weather.is_some() {
                    56.0 + 10.0 + 58.0 + 8.0 + 58.0
                } else {
                    26.0
                }
            }
            Tab::Home => {
                let rows = self.cfg.shortcuts.len().div_ceil(4).max(1) as f32;
                rows * 58.0 + (rows - 1.0) * 8.0 + self.notes_height()
            }
            Tab::Controls => 40.0 * 2.0 + 8.0 + 12.0 + 30.0 * 4.0 + self.wpe_height() + self.outputs_height(),
            Tab::Shelf => self.shelf_height(),
            Tab::Dex => {
                let more = if self.dex_forms || self.dex_shiny || !self.special_missing().is_empty() { 34.0 } else { 0.0 };
                let more = if self.dex_bag { f32::max(more, 52.0) } else { more };
                let more = if self.dex_card { f32::max(more, 126.0) } else { more };
                60.0 + 10.0 + 34.0 + 32.0 + 26.0 + DEX_ROWS as f32 * DEX_CELL + more
            }
            Tab::Power => 34.0 + 10.0 + 62.0,
            Tab::Game => {
                let rows = self.games.len().clamp(1, GAME_ROWS) as f32;
                let playing = if self.game_child.is_some() { 52.0 } else { 0.0 };
                34.0 + 8.0 + playing + rows * 44.0 + 6.0 + 34.0
            }
        };
        12.0 + TAB_H + 12.0 + content + 12.0
    }

    /// Width of a tab button: a circle, or a pill with the name when selected.
    fn tab_target(&mut self, tab: Tab) -> f32 {
        if tab == self.tab {
            30.0 + self.text_width_at(tab.label(), 12.0, 600) + 12.0
        } else {
            TAB_H
        }
    }

    /// Opening the island: jump to whatever needs attention, else stay on the last tab.
    fn pick_tab(&mut self) {
        self.check_wallpaper();
        // The tab bar's do-not-disturb button shows whether it's on.
        self.ctl_dnd = controls::dnd();
        if self.tab == Tab::Media && self.media.is_none() && !self.usage.is_empty() {
            self.tab = Tab::Usage;
        }
        self.enter_tab();
    }

    /// Fetch whatever the current tab shows that isn't pushed to us.
    fn enter_tab(&mut self) {
        self.armed = None;
        match self.tab {
            Tab::Calendar => self.refresh_calendar(),
            Tab::Controls => {
                self.refresh_outputs();
                self.ctl_dnd = controls::dnd();
                self.ctl_dark = controls::dark_mode();
                if let Some(a) = &self.audio {
                    self.ctl_mic = a.mic_level();
                    if let Some(v) = a.level() {
                        self.last_vol = Some(v);
                    }
                }
            }
            Tab::Power => self.user = controls::user_name(),
            Tab::Game => {
                self.games = crate::games::load();
                self.check_game();
            }
            _ => {}
        }
    }

    fn tab_badge(&self, tab: Tab) -> Option<Rgba> {
        match tab {
            Tab::Usage => {
                let worst = self.usage.iter().flat_map(|u| u.limits.iter()).map(|l| l.used).fold(0.0f32, f32::max);
                (worst >= self.cfg.usage_critical).then_some(self.theme.bad)
            }
            Tab::Media if self.playing => Some(self.tint.unwrap_or(self.theme.accent).a(0.8)),
            _ => None,
        }
    }

    fn target(&mut self) -> (Mode, f32, f32, f32) {
        let bh = self.pill_h;
        if matches!(self.scrub, Some((_, _, true))) {
            return (Mode::Compact, 320.0, bh, bh / 2.0);
        }
        if self.osd.is_some() {
            (Mode::Osd, 270.0, bh, bh / 2.0)
        } else if !self.decisions.is_empty() {
            let h = self.decision_height();
            (Mode::Decision, DEC_W, h, 22.0)
        } else if let Some(c) = &self.card {
            match c.kind {
                CardKind::Shot(_) => (Mode::Card, 380.0, 112.0, 22.0),
                _ => (Mode::Card, 380.0, 76.0, 22.0),
            }
        } else if self.expanded {
            (Mode::Expanded, EXP_W, self.expanded_height(), 22.0)
        } else {
            // Under the cursor it swells a little before opening.
            let swell = if self.hover { 1.0 } else { 0.0 };
            (Mode::Compact, self.compact_width() + 12.0 * swell, bh + 3.0 * swell, bh / 2.0)
        }
    }

    /// The colour the surface leans towards: album art while music plays, the accent when
    /// something needs you, a card's own colour, else a quiet blend.
    fn mood_target(&self) -> Rgba {
        let th = &self.theme;
        match self.mode {
            Mode::Decision | Mode::Osd => th.accent,
            Mode::Card => self.card.as_ref().map(|c| c.tint).unwrap_or(th.accent),
            Mode::Compact | Mode::Expanded => match self.compact() {
                Compact::Waiting(_) => th.accent,
                Compact::Media if self.playing => self.tint.unwrap_or(th.accent),
                Compact::Working(_) => th.accent.mix(th.bg2, 0.35),
                _ => th.bg2.mix(th.accent, 0.3),
            },
        }
    }

    fn mood(&self) -> Rgba {
        Rgba { r: self.mood[0].v, g: self.mood[1].v, b: self.mood[2].v, a: 1.0 }
    }

    /// Music is playing and the tide is showing (the compact music pill or the open island).
    fn tide(&self) -> bool {
        self.playing
            && self.media.is_some()
            && match self.mode {
                Mode::Compact => matches!(self.compact(), Compact::Media),
                Mode::Expanded => true,
                Mode::Card => self.card.as_ref().is_some_and(|c| c.kind == CardKind::Song),
                _ => false,
            }
    }

    /// A card's content is still sliding in.
    fn card_entering(&self) -> bool {
        self.mode == Mode::Card && self.card_since.elapsed() < CARD_IN + Duration::from_millis(40)
    }

    /// The sky Pip shows, from the current weather.
    fn sky(&self) -> Sky {
        let Some(w) = self.weather.as_ref().filter(|_| self.cfg.mascot) else { return Sky::None };
        match w.code {
            0 | 1 => if w.day { Sky::Sun } else { Sky::Moon },
            2 | 3 | 45 | 48 => Sky::Cloud,
            51..=67 | 80..=82 => Sky::Rain,
            71..=77 | 85 | 86 => Sky::Snow,
            95..=99 => Sky::Storm,
            _ => Sky::Cloud,
        }
    }

    /// The weather the island shows: the real weather (or the one set to try out).
    fn island_weather(&self) -> Option<Wx> {
        Some(match self.cfg.weather_effect.as_str() {
            "off" => return None,
            "sun" => Wx::Sun,
            "night" => Wx::Night,
            "partly" => Wx::PartlySun,
            "cloud" => Wx::Cloud,
            "fog" => Wx::Fog,
            "drizzle" => Wx::Drizzle,
            "rain" => Wx::Rain,
            "downpour" => Wx::Downpour,
            "storm" => Wx::Storm,
            "hail" => Wx::Hail,
            "snow" => Wx::Snow,
            "wind" | "cold" => Wx::Cloud,
            "meteors" | "fullmoon" | "halloween" => Wx::Night,
            "christmas" => Wx::Snow,
            "hot" => Wx::Sun,
            _ => {
                let w = self.weather.as_ref()?;
                match w.code {
                    0 | 1 => if w.day { Wx::Sun } else { Wx::Night },
                    2 => if w.day { Wx::PartlySun } else { Wx::PartlyNight },
                    45 | 48 => Wx::Fog,
                    51..=57 => Wx::Drizzle,
                    65 | 67 | 82 => Wx::Downpour,
                    61..=67 | 80 | 81 => Wx::Rain,
                    71..=77 | 85 | 86 => Wx::Snow,
                    96 | 99 => Wx::Hail,
                    95..=99 => Wx::Storm,
                    _ => Wx::Cloud,
                }
            }
        })
    }

    /// The moon as it is tonight, in pixels, centred on (cx, cy): the lit part bright with a
    /// few darker seas, the dark part faintly there, and a glow as bright as it is full.
    fn draw_moon(&mut self, cx: f32, cy: f32, age: f64, k: f32, glow: bool) {
        const N: usize = 11;
        // The darker seas on its face (column, row).
        const SEAS: [(usize, usize); 9] = [(3, 3), (4, 3), (4, 4), (6, 2), (7, 3), (6, 6), (7, 6), (6, 7), (3, 7)];
        let s = self.scale;
        let px = (1.0 * s).round().max(1.0) / s;
        let south = southern();
        let share = pokemon::moon_lit_share(age) as f32;
        let face = Rgba::rgb(0xf2, 0xee, 0xd6);
        if glow {
            self.r.glow(cx, cy, 10.0 + 10.0 * share, face.a((0.04 + 0.12 * share) * k));
        }
        let x0 = ((cx - N as f32 * px / 2.0) * s).round() / s;
        let y0 = ((cy - N as f32 * px / 2.0) * s).round() / s;
        for j in 0..N {
            for i in 0..N {
                let (nx, ny) = ((i as f64 + 0.5) / (N as f64 / 2.0) - 1.0, (j as f64 + 0.5) / (N as f64 / 2.0) - 1.0);
                if nx * nx + ny * ny > 1.04 {
                    continue;
                }
                let col = if pokemon::moon_lit(age, nx, ny, south) {
                    if SEAS.contains(&(i, j)) { Rgba::rgb(0xd2, 0xcc, 0xb0).a(0.85 * k) } else { face.a(0.9 * k) }
                } else {
                    Rgba::rgb(120, 132, 165).a(0.15 * k)
                };
                self.r.rect(R::xywh(x0 + i as f32 * px, y0 + j as f32 * px, px, px), col);
            }
        }
    }

    /// The weather moving across the island's surface, behind its content.
    fn draw_weather(&mut self, body: R, t: f32, wx: Wx, k: f32) {
        use std::f32::consts::{PI, TAU};
        let (x0, x1, y0, y1) = (body.x0, body.x1, body.y0, body.y1);
        let (w, h) = (x1 - x0, y1 - y0);
        if w < 20.0 || k <= 0.01 {
            return;
        }
        let warm = Rgba::rgb(255, 196, 90);
        let cool = Rgba::rgb(214, 226, 255);
        // Wind, heat and cold go on top of whatever the sky is doing.
        let c = self.conditions();
        let fx = self.cfg.weather_effect.clone();
        let windy = c.windy || fx == "wind";
        let special = match fx.as_str() {
            "meteors" | "fullmoon" | "halloween" | "christmas" => Some(fx.as_str()),
            _ => self.special().map(|s| s.key),
        };
        let gust = if windy { 0.45 } else { 0.0 };
        // Soft cloud banks drifting slowly across.
        let clouds = |r: &mut Renderer, tone: Rgba, alpha: f32, extra: u32| {
            let n = 3 + (w / 110.0) as u32 + extra;
            let span = w + 160.0;
            for i in 0..n {
                let speed = 3.0 + 5.0 * rnd(i, 1);
                let x = x0 - 80.0 + (t * speed + rnd(i, 2) * span).rem_euclid(span);
                let y = y0 + h * (0.2 + 0.6 * rnd(i, 3));
                let rad = 22.0 + 20.0 * rnd(i, 4);
                r.glow(x, y, rad, tone.a(alpha * k));
                r.glow(x + rad * 0.65, y + 4.0, rad * 0.75, tone.a(alpha * 0.85 * k));
                r.glow(x - rad * 0.55, y + 5.0, rad * 0.6, tone.a(alpha * 0.7 * k));
            }
        };
        // Rain: streaks slanting in the wind, each splashing on the lip. All the drops are
        // one shape and all the splashes another, so it's two draws however hard it rains.
        let rain = |r: &mut Renderer, gap: f32, speed: f32, alpha: f32, wind: f32| {
            let n = (w / gap) as u32;
            let fall = h + 12.0;
            let hit = (y1 - 3.0 - (y0 - 8.0)) / fall;
            let mut drops = Vec::with_capacity(n as usize);
            let mut splash = Vec::new();
            for i in 0..n {
                let v = (110.0 + 60.0 * rnd(i, 5)) * speed;
                let ph = (t * v / fall + rnd(i, 6)).fract();
                let len = 5.0 + 3.0 * rnd(i, 7);
                let xb = x0 + (w + 20.0) * rnd(i, 8);
                if ph < hit {
                    let y = y0 - 8.0 + ph * fall;
                    let x = xb - wind * (y - y0);
                    drops.push((x + wind * len, y - len, x, y));
                } else {
                    // A little spray thrown up both ways.
                    let u = (ph - hit) / (1.0 - hit);
                    let (x, y) = (xb - wind * (y1 - 3.0 - y0), y1 - 3.0);
                    let (sx, sy) = (1.0 + 2.5 * u, 2.2 * u * (1.0 - u) * 2.0);
                    splash.push((x - sx, y - sy, x - sx - 0.6, y - sy + 0.5));
                    splash.push((x + sx, y - sy, x + sx + 0.6, y - sy + 0.5));
                }
            }
            let c = Rgba::rgb(175, 205, 255);
            r.lines(&drops, c.a(alpha * k), 1.0);
            r.lines(&splash, c.a(alpha * 0.8 * k), 0.9);
        };
        match wx {
            Wx::Sun | Wx::PartlySun => {
                // The sun peeking in at the top right, its rays slowly turning.
                let (sx, sy) = (x1 - 24.0, y0 + h * 0.22);
                let pulse = 0.5 + 0.5 * (t * 0.9).sin();
                self.r.glow(sx, sy, 72.0 + 8.0 * pulse, warm.a((0.15 + 0.05 * pulse) * k));
                let rays: Vec<_> = (0..10)
                    .map(|i| {
                        let ang = t * 0.22 + i as f32 * TAU / 10.0;
                        let reach = 10.0 + 14.0 * (1.0 + 0.3 * (t * 1.6 + i as f32).sin());
                        let (c, s) = (ang.cos(), ang.sin());
                        (sx + c * 8.0, sy + s * 8.0, sx + c * reach, sy + s * reach)
                    })
                    .collect();
                self.r.lines(&rays, warm.a(0.13 * k), 1.2);
                self.r.circle(sx, sy, 5.0, warm.a(0.4 * k));
                // Warm motes rising in the light.
                for i in 0..7 {
                    let u = (t * 0.07 * (0.6 + rnd(i, 9)) + rnd(i, 10)).fract();
                    let x = x0 + w * rnd(i, 11) + (t * 0.7 + i as f32).sin() * 4.0;
                    self.r.rect(R::xywh(x, y1 - u * h, 1.2, 1.2), warm.a(0.4 * (u * PI).sin() * k));
                }
                if wx == Wx::PartlySun {
                    clouds(&mut self.r, Rgba::rgb(235, 238, 245), 0.13, 0);
                }
            }
            Wx::Night | Wx::PartlyNight => {
                self.r.glow(x1 - 26.0, y0 + h * 0.3, 44.0, cool.a(0.08 * k));
                if special == Some("fullmoon") {
                    // A big bright full moon.
                    self.r.glow(x1 - 26.0, y0 + h * 0.3, 70.0, Rgba::rgb(250, 246, 225).a(0.14 * k));
                }
                // Pixel stars, each twinkling at its own pace.
                for i in 0..(w / 13.0) as u32 {
                    let x = x0 + 6.0 + (w - 12.0) * rnd(i, 12);
                    let y = y0 + 3.0 + (h - 8.0) * rnd(i, 13);
                    let tw = (t * (0.8 + 1.6 * rnd(i, 14)) + rnd(i, 15) * TAU).sin().max(0.0);
                    let sz = if rnd(i, 16) > 0.75 { 1.4 } else { 1.0 };
                    self.r.rect(R::xywh(x, y, sz, sz), cool.a((0.15 + 0.6 * tw) * k));
                }
                // Now and then, a shooting star; in a meteor shower, every couple of seconds.
                let every = if special == Some("meteors") { 1.8 } else { 11.0 };
                let cyc = (t / every).floor();
                let u = (t - cyc * every) / 0.8;
                if u < 1.0 {
                    let c = cyc as u32;
                    let (sx, sy) = (x0 + w * (0.15 + 0.6 * rnd(c, 17)), y0 + 2.0 + 4.0 * rnd(c, 18));
                    let (hx, hy) = (sx + 70.0 * u, sy + 14.0 * u);
                    self.r.line(hx - 16.0, hy - 3.2, hx, hy, cool.a(0.7 * (1.0 - u) * k), 1.1);
                }
                if wx == Wx::PartlyNight {
                    clouds(&mut self.r, Rgba::rgb(150, 160, 185), 0.1, 0);
                }
            }
            Wx::Cloud => clouds(&mut self.r, Rgba::rgb(220, 226, 236), 0.17, 1),
            Wx::Fog => {
                clouds(&mut self.r, Rgba::rgb(210, 214, 222), 0.15, 3);
                self.r.rect(R::new(x0, y0 + h * 0.45, x1, y1), Rgba::rgb(210, 214, 222).a(0.05 * k));
            }
            Wx::Drizzle => {
                clouds(&mut self.r, Rgba::rgb(190, 198, 214), 0.08, 0);
                rain(&mut self.r, 15.0, 0.8, 0.26, 0.2 + gust);
            }
            Wx::Rain => {
                clouds(&mut self.r, Rgba::rgb(170, 180, 200), 0.1, 0);
                rain(&mut self.r, 8.0, 1.0, 0.34, 0.25 + gust);
            }
            Wx::Downpour => {
                clouds(&mut self.r, Rgba::rgb(150, 160, 182), 0.13, 1);
                rain(&mut self.r, 4.5, 1.4, 0.42, 0.3 + gust);
            }
            Wx::Hail => {
                clouds(&mut self.r, Rgba::rgb(125, 132, 152), 0.14, 1);
                rain(&mut self.r, 10.0, 1.2, 0.28, 0.4 + gust);
                // Hailstones: quick white pellets that bounce off the lip.
                let fall = h + 6.0;
                for i in 0..(w / 13.0) as u32 {
                    let v = 70.0 + 40.0 * rnd(i, 26);
                    let ph = (t * v / fall + rnd(i, 27)).fract();
                    let xb = x0 + w * rnd(i, 28);
                    let (x, y) = if ph < 0.85 {
                        (xb + gust * 8.0 * ph, y0 - 3.0 + ph / 0.85 * (fall - 6.0))
                    } else {
                        let u = (ph - 0.85) / 0.15;
                        (xb + gust * 6.8 + 3.0 * u, y1 - 3.0 - 12.0 * u * (1.0 - u))
                    };
                    let sz = 1.6 + 0.8 * rnd(i, 29);
                    self.r.rect(R::xywh(x, y, sz, sz), Rgba::rgb(240, 246, 255).a(0.85 * k));
                }
            }
            Wx::Storm => {
                clouds(&mut self.r, Rgba::rgb(120, 128, 150), 0.14, 1);
                rain(&mut self.r, 6.0, 1.3, 0.36, 0.45 + gust);
                // Lightning: a double flash lighting the whole island, and the bolt.
                let cyc = (t / 6.5).floor();
                let c = cyc as u32;
                let local = t - cyc * 6.5 - rnd(c, 19) * 4.5;
                if (0.0..0.7).contains(&local) {
                    let f = (-local * 9.0).exp() + if local > 0.18 { 0.7 * (-(local - 0.18) * 11.0).exp() } else { 0.0 };
                    self.r.rect(body, Rgba::rgb(235, 240, 255).a((0.2 * f).min(0.3) * k));
                    let mut bolt = Vec::new();
                    let (mut bx, mut by) = (x0 + w * (0.2 + 0.6 * rnd(c, 20)), y0);
                    for s in 0..4u32 {
                        let nx = bx + (rnd(c, 21 + s) - 0.5) * 14.0;
                        let ny = y0 + h * (s + 1) as f32 / 4.0;
                        bolt.push((bx, by, nx, ny));
                        (bx, by) = (nx, ny);
                    }
                    self.r.lines(&bolt, Rgba::rgb(240, 244, 255).a((0.9 * f).min(1.0) * k), 1.3);
                }
            }
            Wx::Snow => {
                clouds(&mut self.r, Rgba::rgb(225, 230, 240), 0.08, 0);
                let fall = h + 6.0;
                for i in 0..(w / 10.0) as u32 {
                    let v = 9.0 + 10.0 * rnd(i, 22);
                    let ph = (t * v / fall + rnd(i, 23)).fract();
                    let y = y0 - 3.0 + ph * fall;
                    let x = x0 + w * rnd(i, 24) + (t * 1.1 + i as f32).sin() * 3.0;
                    let fade = (ph * 5.0).min(1.0) * ((1.0 - ph) * 6.0).min(1.0);
                    let sz = 1.3 + 1.0 * rnd(i, 25);
                    self.r.rect(R::xywh(x + gust * 20.0 * ph, y, sz, sz), Rgba::rgb(255, 255, 255).a(0.85 * fade * k));
                }
            }
        }
        // Halloween: wisps drifting up; Christmas: lights twinkling along the edge.
        if special == Some("halloween") {
            for i in 0..2 + (w / 120.0) as u32 {
                let u = (t * 0.08 * (0.6 + rnd(i, 41)) + rnd(i, 42)).fract();
                let x = x0 + w * rnd(i, 43) + (t * 0.9 + i as f32).sin() * 6.0;
                let col = if i % 2 == 0 { Rgba::rgb(170, 110, 255) } else { Rgba::rgb(120, 255, 170) };
                self.r.glow(x, y1 - u * h, 7.0, col.a(0.35 * (u * PI).sin() * k));
            }
        }
        if special == Some("christmas") {
            let cols = [Rgba::rgb(255, 80, 80), Rgba::rgb(90, 220, 120), Rgba::rgb(255, 210, 90), Rgba::rgb(110, 170, 255)];
            let n = (w / 18.0) as u32;
            for i in 0..n {
                let x = x0 + 10.0 + (w - 20.0) * i as f32 / n.max(1) as f32;
                let on = 0.35 + 0.65 * (t * 1.5 + i as f32 * 0.9).sin().max(0.0);
                self.r.circle(x, y1 - 2.5, 1.3, cols[i as usize % 4].a(on * k));
            }
        }
        // Wind: gusts streaking across.
        if windy {
            let span = w + 120.0;
            let mut gusts = Vec::new();
            for i in 0..2 + (w / 70.0) as u32 {
                let v = 140.0 + 120.0 * rnd(i, 30);
                let x = x0 - 60.0 + (t * v + rnd(i, 31) * span).rem_euclid(span);
                let y = y0 + 4.0 + (h - 8.0) * rnd(i, 32);
                let len = 16.0 + 26.0 * rnd(i, 33);
                let wave = (t * 3.0 + i as f32).sin() * 1.5;
                gusts.push((x - len, y + wave, x - len * 0.5, y));
                gusts.push((x - len * 0.5, y, x, y - wave * 0.6));
            }
            self.r.lines(&gusts, Rgba::rgb(230, 236, 245).a(0.18 * k), 1.0);
        }
        // Heat: a shimmer rising off the island on a hot, dry day.
        let wet = matches!(wx, Wx::Drizzle | Wx::Rain | Wx::Downpour | Wx::Storm | Wx::Hail | Wx::Snow);
        if (c.hot && c.day && !wet) || fx == "hot" {
            let mut haze = Vec::new();
            for i in 0..(w / 26.0) as u32 {
                let u = (t * 0.25 * (0.7 + rnd(i, 34)) + rnd(i, 35)).fract();
                let x = x0 + w * rnd(i, 36);
                let y = y1 - 2.0 - u * h * 0.8;
                for s in 0..3 {
                    let wob = |j: f32| (t * 4.0 + i as f32 + j).sin() * 1.2;
                    haze.push((x + wob(s as f32), y - s as f32 * 3.0, x + wob(s as f32 + 1.0), y - (s + 1) as f32 * 3.0));
                }
            }
            self.r.lines(&haze, warm.a(0.14 * k), 1.0);
        }
        // Cold: frost along the lip, glinting.
        if c.cold || fx == "cold" {
            self.r.rect(R::new(x0, y1 - 2.5, x1, y1), cool.a(0.12 * k));
            for i in 0..(w / 22.0) as u32 {
                let tw = (t * (0.6 + rnd(i, 37)) + rnd(i, 38) * TAU).sin().max(0.0);
                let x = x0 + 4.0 + (w - 8.0) * rnd(i, 39);
                self.twinkle(x, y1 - 3.0 - 3.0 * rnd(i, 40), 1.8 * tw, Rgba::rgb(235, 245, 255).a(0.7 * tw * k));
            }
        }
    }

    /// The island's outline: (body, bottom-corner radius, shoulder radius). Hanging from the
    /// top edge, the body starts at the edge, above the content.
    fn body(&self) -> (R, f32, f32) {
        let cx = WIN_W / 2.0;
        let p = R::new(cx - self.w.v / 2.0, self.neck, cx + self.w.v / 2.0, self.neck + self.h.v);
        let rad = self.rad.v.min(p.h() / 2.0).max(0.0);
        if self.cfg.attach_to_top {
            (R::new(p.x0, 0.0, p.x1, p.y1), rad, SHOULDER)
        } else {
            (p, rad, 0.0)
        }
    }

    fn settled(&self) -> bool {
        self.w.settled()
            && self.h.settled()
            && self.rad.settled()
            && self.alpha.settled()
            && self.mood.iter().all(|s| s.settled())
            && self.glow.settled()
            && self.peak.settled()
            && self.ripples.is_empty()
            && self.hop.is_none()
            && self.swipe.settled()
            && !self.card_entering()
            && (self.mode != Mode::Expanded || self.tab_w.iter().all(|s| s.settled()))
    }

    fn ambient(&self) -> bool {
        match self.mode {
            Mode::Compact => match self.compact() {
                Compact::Waiting(_) | Compact::Working(_) => true,
                _ if self.wild.is_some() || self.fishing.is_some() || self.hatch.is_some() || !self.hearts.is_empty() || !self.xp_pops.is_empty() => true,
                _ if self.island_weather().is_some() => true,
                // The Pokémon keeps moving even when nothing else in the pill does.
                Compact::Media => {
                    (self.playing && self.cfg.animate_equalizer) || self.media_spinner() || (self.cfg.mascot && self.cfg.mascot_pokemon)
                }
                // The Pokémon is always moving; rain, snow and storms fall past Pip.
                Compact::Usage | Compact::Idle => {
                    (self.cfg.mascot && self.cfg.mascot_pokemon) || matches!(self.sky(), Sky::Rain | Sky::Snow | Sky::Storm)
                }
            },
            Mode::Expanded => match self.tab {
                _ if self.tide() && self.cfg.animate_equalizer => true,
                Tab::Media => self.playing && self.timeline.is_some(),
                _ => false,
            },
            Mode::Card => self.card.as_ref().is_some_and(|c| c.sticky || matches!(c.kind, CardKind::Song | CardKind::Power { .. } | CardKind::Break)),
            Mode::Decision => true,
            Mode::Osd => false,
        }
    }

    /// Milliseconds until the pill's real Pokémon sprite moves on to its next frame.
    fn sprite_wait_ms(&self) -> Option<u32> {
        if !(self.cfg.mascot && self.cfg.mascot_pokemon) {
            return None;
        }
        let m = match self.compact() {
            Compact::Media => self.media_mood(),
            Compact::Usage | Compact::Idle => self.pip_mood(),
            _ => return None,
        };
        let Some(Some(Sprite { delays, .. })) = self.strips.get(self.mascot_sprite(m).as_str()) else { return None };
        let speed = if m == PipMood::Sleep { 0.5 } else { 1.0 };
        let ms = (self.t0.elapsed().as_secs_f64() * 1000.0 * speed) as u64;
        let left = pokemon::ms_to_next_frame(delays, ms) as f64 / speed;
        // A hair late rather than early, so the frame has really changed when we draw.
        Some(left.ceil() as u32 + 1)
    }

    fn needs_frames(&self) -> bool {
        self.visible && (self.dirty || !self.settled() || self.ambient())
    }

    /// After a frame: how long to sleep (waking early for input) before the next one.
    /// 0 = the shape is still morphing, go again at the next vblank.
    fn frame_wait_ms(&self) -> u32 {
        if self.dirty || !self.settled() {
            return 0;
        }
        match self.mode {
            Mode::Compact => {
                // The real sprite: wake exactly when its next frame is due, so every frame
                // shows for just as long as the game shows it.
                let sprite = self.sprite_wait_ms();
                let gusty = self.conditions().windy || self.cfg.weather_effect == "wind";
                let wx = self.island_weather().map(|w| if gusty { w.frame_ms().min(50) } else { w.frame_ms() });
                let base = match self.compact() {
                    Compact::Media if self.playing && self.cfg.animate_equalizer => sprite.map_or(42, |s| s.min(42)), // tide and equaliser, ~24 fps
                    Compact::Media if self.media_spinner() => sprite.map_or(50, |s| s.min(50)),
                    Compact::Media | Compact::Usage | Compact::Idle => sprite.unwrap_or(90),
                    _ => 50, // spinner / pulse, ~20 fps
                };
                let wild_busy = self.wild.as_ref().is_some_and(|w| !wild_waiting(w) || self.hover) || self.fishing.is_some();
                let base = if wild_busy || self.hatch.is_some() || !self.hearts.is_empty() || !self.xp_pops.is_empty() {
                    base.min(33)
                } else if self.wild.is_some() {
                    base.min(66)
                } else {
                    base
                };
                wx.map_or(base, |w| base.min(w))
            }
            Mode::Card if self.tide() => 42,
            Mode::Expanded if self.tide() && self.cfg.animate_equalizer => 42,
            Mode::Decision => 100, // pulse and countdown
            Mode::Expanded => 250, // progress bar
            _ => 50,
        }
    }

    fn update_visibility(&mut self) {
        let want = !(self.cfg.hide_in_fullscreen && self.fullscreen) || self.osd.is_some();
        if want != self.visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, if want { SW_SHOWNOACTIVATE } else { SW_HIDE });
            }
            self.visible = want;
            self.dirty = true;
        }
    }

    fn set_fullscreen(&mut self, fs: bool) {
        let (exe, fg) = foreground_exe();
        // Screenshot tools cover the screen too; stay visible so the island can be captured.
        let tool = matches!(exe.as_str(), "snippingtool.exe" | "screenclippinghost.exe" | "sharex.exe" | "screensketch.exe");
        let same_monitor = unsafe { MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST) } == self.monitor;
        self.fullscreen = fs && !tool && same_monitor;
        self.update_visibility();
    }

    // ---------- cards & pop-ups ----------

    fn show_osd(&mut self, kind: OsdKind, value: f32, muted: bool) {
        if self.expanded && self.tab == Tab::Controls {
            // The sliders already show it.
            self.dirty = true;
            return;
        }
        self.osd = Some(Osd { kind, value, muted, until: Instant::now() + Duration::from_millis(1600) });
        self.schedule_expiry();
        self.update_visibility();
        self.dirty = true;
    }

    fn show_card(&mut self, c: Card) {
        self.card_bmp = c.icon.as_ref().and_then(|a| self.r.bitmap(a));
        self.card_since = Instant::now();
        self.card = Some(c);
        self.dirty = true;
    }

    fn next_card(&mut self) {
        self.card = None;
        self.card_bmp = None;
        if let Some(c) = self.queue.pop_front() {
            self.show_card(c);
        }
        self.dirty = true;
    }

    fn drop_cards(&mut self, key: &str) {
        self.queue.retain(|c| c.key.as_deref() != Some(key));
        if self.card.as_ref().and_then(|c| c.key.as_deref()) == Some(key) {
            self.next_card();
        }
    }

    fn push_card(&mut self, c: Card) {
        if let Some(k) = c.key.clone() {
            self.drop_cards(&k);
        }
        match &self.card {
            None => self.show_card(c),
            Some(cur) if c.sticky && !cur.sticky => {
                let old = self.card.take().unwrap();
                self.queue.push_front(old);
                self.show_card(c);
            }
            Some(_) => {
                self.queue.push_back(c);
                while self.queue.len() > 6 {
                    self.queue.pop_front();
                }
            }
        }
        self.schedule_expiry();
    }

    /// How long a card stays: `base` seconds at the default notification time, scaled by the
    /// `card_seconds` setting.
    fn card_ttl(&self, base: f32) -> Duration {
        let scale = (self.cfg.card_seconds / 3.5).clamp(0.2, 30.0);
        Duration::from_secs_f32((base * scale).clamp(1.0, 300.0))
    }

    /// The pointer really is over an island (hover pauses cards; if the island ever missed
    /// the pointer leaving, a card must not hang around because of it).
    fn hovering(&self) -> bool {
        if !self.hover {
            return false;
        }
        let mut pt = POINT::default();
        unsafe {
            if GetCursorPos(&mut pt).is_err() {
                return true;
            }
            let h = WindowFromPoint(pt);
            h == self.hwnd || h == main_hwnd() || self.views.iter().any(|v| v.hwnd == h)
        }
    }

    /// When the current card goes: its own time, or sooner when others are waiting.
    fn card_due(&self, c: &Card) -> Instant {
        let ttl = if self.queue.is_empty() { c.ttl } else { c.ttl.min(Duration::from_millis(2500)) };
        self.card_since + ttl
    }

    fn schedule_expiry(&self) {
        let now = Instant::now();
        let mut next: Option<Duration> = None;
        let mut take = |d: Duration| next = Some(next.map_or(d, |n| n.min(d)));
        if let Some(o) = &self.osd {
            take(o.until.saturating_duration_since(now));
        }
        if let Some(c) = &self.card {
            if !c.sticky {
                if self.hovering() {
                    // Look again in a moment, in case the pointer left unnoticed.
                    take(Duration::from_secs(1));
                } else {
                    take(self.card_due(c).saturating_duration_since(now));
                }
            }
        }
        unsafe {
            match next {
                Some(d) => {
                    SetTimer(Some(main_hwnd()), T_EXPIRE, (d.as_millis() as u32).max(10), None);
                }
                None => {
                    let _ = KillTimer(Some(main_hwnd()), T_EXPIRE);
                }
            }
        }
    }

    fn expire(&mut self) {
        let now = Instant::now();
        if self.osd.as_ref().map(|o| o.until <= now).unwrap_or(false) {
            self.osd = None;
        }
        let card_done = self
            .card
            .as_ref()
            .map(|c| !c.sticky && !self.hovering() && self.card_due(c) <= now)
            .unwrap_or(false);
        if card_done {
            self.next_card();
        }
        self.schedule_expiry();
        self.update_visibility();
        self.dirty = true;
    }

    // ---------- events ----------

    fn drain(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::Volume { level, muted } => {
                    let changed = self.last_vol.map(|(l, m)| (l - level).abs() > 0.004 || m != muted).unwrap_or(true);
                    self.last_vol = Some((level, muted));
                    if changed {
                        self.show_osd(OsdKind::Volume, level, muted);
                    }
                }
                Event::AudioDeviceChanged => {
                    if let Some(a) = &mut self.audio {
                        a.bind();
                    }
                    self.refresh_outputs();
                    if self.cfg.headphones_peek && self.born.elapsed() > Duration::from_secs(5) {
                        peeks::check_output();
                    }
                }
                Event::OutputDevice { name, battery } => {
                    let body = match battery {
                        Some(b) => format!("Connected · battery {b}%"),
                        None => "Connected · sound plays here now".into(),
                    };
                    self.push_card(Card {
                        key: Some("output".into()),
                        glyph: G_HEADPHONES,
                        tint: if battery.is_some_and(|b| b <= 20) { self.theme.bad } else { self.theme.accent },
                        app: "Headphones".into(),
                        title: name,
                        body,
                        icon: None,
                        sticky: false,
                        ttl: self.card_ttl(3.0),
                        focus: Focus::No,
                        kind: CardKind::Plain,
                    });
                }
                Event::Screenshot { path, art } => {
                    if self.cfg.screenshot_peek {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        // Replaces the "copied" peek the same snip may have just shown.
                        self.push_card(Card {
                            key: Some("shot".into()),
                            glyph: G_SCREENSHOT,
                            tint: art.as_ref().map(|a| Rgba::from_f(a.tint)).unwrap_or(self.theme.accent),
                            app: "Screenshot saved".into(),
                            title: name,
                            body: "Click to open".into(),
                            icon: art,
                            sticky: false,
                            ttl: self.card_ttl(4.0),
                            focus: Focus::No,
                            kind: CardKind::Shot(Some(path)),
                        });
                    }
                }
                Event::Brightness(b) => {
                    self.brightness = Some(b);
                    self.show_osd(OsdKind::Brightness, b as f32 / 100.0, false);
                }
                Event::BrightnessLevel(b) => self.brightness = Some(b),
                Event::Sys(mut sys) => {
                    if util::demo() {
                        crate::demo::scrub_sys(&mut sys);
                    }
                    let push = |h: &mut VecDeque<f32>, v: f32| {
                        h.push_back(v);
                        while h.len() > 60 {
                            h.pop_front();
                        }
                    };
                    push(&mut self.cpu_hist, sys.cpu);
                    push(&mut self.mem_hist, if sys.mem_total > 0 { sys.mem_used as f32 / sys.mem_total as f32 } else { 0.0 });
                    if let Some(g) = &sys.gpu {
                        push(&mut self.gpu_hist, g.util);
                    }
                    self.sys = Some(sys);
                }
                Event::Weather(w) => {
                    self.weather = Some(w);
                    self.update_keeper(true);
                }
                Event::Wallpaper { titles, announce } => {
                    // Never right at start-up.
                    if announce && !titles.is_empty() && self.born.elapsed() > Duration::from_secs(120) && !self.expanded {
                        let accent = self.theme.accent;
                        self.quick_card("wallpaper", G_SCREENSHOT, accent, "Wallpaper Engine", "Wallpaper of the day", &titles.join("  ·  "), 6.0);
                    }
                    self.wpe_titles = titles;
                    self.wpe_asked = None;
                }
                Event::WallpaperAccent(key, accent) => {
                    if self.wall_key.as_ref() == Some(&key) {
                        self.wall_accent = accent.map(Rgba::from_f);
                        self.apply_accent();
                    }
                }
                Event::Media(m) => {
                    // Pip hops for a new song, and the island peeks out with it.
                    if let Some(n) = m.as_ref().filter(|n| self.media.as_ref().map(|o| &o.title) != Some(&n.title)) {
                        self.hop = Some(Instant::now());
                        let fresh = self.born.elapsed() > Duration::from_secs(5);
                        if self.cfg.song_peek && fresh && !n.title.is_empty() && !self.expanded {
                            self.push_card(Card {
                                key: Some("song".into()),
                                glyph: G_MUSIC,
                                tint: self.tint.unwrap_or(self.theme.accent),
                                app: if n.app.is_empty() { "Now playing".into() } else { n.app.clone() },
                                title: n.title.clone(),
                                body: n.artist.clone(),
                                icon: None,
                                sticky: false,
                                ttl: Duration::from_millis(3500),
                                focus: Focus::No,
                                kind: CardKind::Song,
                            });
                        }
                    }
                    if m.is_none() {
                        self.playing = false;
                        self.timeline = None;
                    }
                    self.media = m;
                }
                Event::MediaArt(a) => {
                    self.art_bmp = a.as_ref().and_then(|a| self.r.bitmap(a));
                    self.tint = a.as_ref().map(|a| Rgba::from_f(a.tint));
                    // The song peek takes the new artwork's colour.
                    if let (Some(c), Some(t)) = (self.card.as_mut().filter(|c| c.kind == CardKind::Song), self.tint) {
                        c.tint = t;
                    }
                    self.art = a;
                }
                Event::Playback(p) => {
                    if let Some((pos, end, at)) = self.timeline {
                        // Re-anchor so the progress bar doesn't jump when pausing.
                        let p_now = if self.playing { pos + at.elapsed().as_secs_f64() } else { pos };
                        self.timeline = Some((p_now.min(end), end, Instant::now()));
                    }
                    if self.playing && !p {
                        self.paused_at = Some(Instant::now());
                        unsafe {
                            SetTimer(Some(main_hwnd()), T_PAUSED, PAUSE_GRACE.as_millis() as u32 + 20, None);
                        }
                    } else if p {
                        self.paused_at = None;
                    }
                    self.playing = p;
                    self.dirty = true;
                }
                Event::Timeline { pos, end } => {
                    self.timeline = (end > 1.0).then_some((pos, end, Instant::now()));
                }
                Event::Notification { app, title, body, icon } => {
                    self.push_card(Card {
                        key: None,
                        glyph: G_BELL,
                        tint: self.theme.accent,
                        app,
                        title,
                        body,
                        icon,
                        sticky: false,
                        ttl: self.card_ttl(3.5),
                        focus: Focus::No,
                        kind: CardKind::Plain,
                    });
                }
                Event::Usage(u) => self.on_usage(u),
                Event::Privacy { mic, cam } => {
                    let new_cam = cam.iter().find(|a| !self.cam_apps.contains(a)).cloned();
                    let new_mic = mic.iter().find(|a| !self.mic_apps.contains(a)).cloned();
                    let first = !self.privacy_seen;
                    self.privacy_seen = true;
                    self.mic_apps = mic;
                    self.cam_apps = cam;
                    if !first {
                        if let Some(a) = new_cam {
                            self.quick_card("privacy", G_CAMERA, CAM_GREEN, "Camera on", &a, "is using your camera", 3.0);
                        } else if let Some(a) = new_mic {
                            self.quick_card("privacy", G_MIC, MIC_ORANGE, "Microphone on", &a, "is listening", 3.0);
                        }
                    }
                }
                Event::GamePicked(path) => self.game_picked(path),
                Event::Download { path, size } => self.download_card(path, size),
                Event::GitHub(notes) => self.github_cards(notes),
                Event::Heat(h) => self.heat_card(h),
                Event::ScanProgress => self.dirty = true,
                Event::Net(n) => {
                    if let Some(old) = self.net.replace(n.clone()) {
                        self.net_changed(&old, &n);
                    }
                }
            }
        }
        self.update_visibility();
        self.dirty = true;
    }

    fn volume_key(&mut self, vk: u16) {
        let Some(a) = &self.audio else { return };
        let step = self.cfg.volume_step.clamp(1, 20) as i32;
        let r = match VIRTUAL_KEY(vk) {
            VK_VOLUME_UP => a.step(step),
            VK_VOLUME_DOWN => a.step(-step),
            VK_VOLUME_MUTE => a.toggle_mute(),
            _ => None,
        };
        if let Some((level, muted)) = r {
            self.last_vol = Some((level, muted));
            self.show_osd(OsdKind::Volume, level, muted);
        }
    }

    fn on_usage(&mut self, u: Usage) {
        let hp_before = self.claude_hp().map(|h| h.0);
        self.on_usage_inner(u);
        self.hp_changed(hp_before);
    }

    fn on_usage_inner(&mut self, u: Usage) {
        let (watch, crit) = (self.cfg.usage_watch, self.cfg.usage_critical);
        let prev = self.usage.iter().position(|p| p.provider == u.provider);
        if self.cfg.usage_alerts && u.error.is_none() {
            let now = usage::now_unix();
            let mut cards = Vec::new();
            let mut alerts_changed = false;
            let starting = prev.is_none() && self.born.elapsed() < Duration::from_secs(30);
            for l in &u.limits {
                let key = format!("{}:{}", u.provider, l.short);
                let pct = (l.used * 100.0).round();
                let resets = l.resets_at.map(|r| format!("Resets in {}", usage::countdown(r - now))).unwrap_or_default();
                let old = prev.and_then(|i| self.usage[i].limits.iter().find(|x| x.short == l.short)).map(|x| x.used);
                if let Some(o) = old {
                    if o >= watch && l.used <= o - 0.25 {
                        cards.push(Card {
                            key: Some(format!("usage:{key}")),
                            glyph: G_RESTORE,
                            tint: self.theme.good,
                            app: u.provider.to_string(),
                            title: format!("{} {} reset", u.provider, l.long),
                            body: format!("Back to {pct}% (was {}%)", (o * 100.0).round()),
                            icon: None,
                            sticky: false,
                            ttl: self.card_ttl(4.5),
                            focus: Focus::No,
                            kind: CardKind::Plain,
                        });
                    }
                }
                let level = if l.used >= 0.995 { 2u8 } else if l.used >= crit { 1 } else { 0 };
                // An alert counts until its limit resets.
                let had = match self.usage_alerted.get(&key) {
                    Some(&(lv, r)) if r.is_none_or(|r| r > now) => lv,
                    _ => 0,
                };
                // Not at start-up: the first numbers only say where things stand (they're
                // already on the Usage tab), so the island doesn't open with an alert.
                if level > had && !starting {
                    cards.push(Card {
                        key: Some(format!("usage:{key}")),
                        glyph: G_GAUGE,
                        tint: self.theme.bad,
                        app: u.provider.to_string(),
                        title: if level == 2 { format!("{} {} reached", u.provider, l.long) } else { format!("{} {} at {pct}%", u.provider, l.long) },
                        body: if resets.is_empty() { "Pace yourself".into() } else { resets },
                        icon: None,
                        sticky: false,
                        ttl: self.card_ttl(6.0),
                        focus: Focus::No,
                        kind: CardKind::Plain,
                    });
                }
                // Hysteresis, so hovering around the threshold doesn't alert repeatedly.
                if level >= had || l.used < crit - 0.05 {
                    let new = (level, l.resets_at);
                    if self.usage_alerted.get(&key) != Some(&new) {
                        self.usage_alerted.insert(key, new);
                        alerts_changed = true;
                    }
                }
            }
            if alerts_changed {
                save_alerts(&self.usage_alerted);
            }
            for c in cards {
                self.push_card(c);
            }
        }
        match prev {
            Some(i) => self.usage[i] = u,
            None => {
                self.usage.push(u);
                self.usage.sort_by_key(|u| u.provider != "Claude");
            }
        }
        self.dirty = true;
    }

    /// A message from `pocketisle.exe hook`. Returns the WM_COPYDATA reply: for a permission
    /// prompt the island shows, `DECISION_ACCEPTED` + seconds to wait; otherwise 1.
    fn agent_message(&mut self, s: &str) -> isize {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(s) else { return 1 };
        let get = |k: &str| v[k].as_str().unwrap_or("").to_string();
        let (agent, mut event, session, cwd, mut msg) = (get("agent"), get("event"), get("session"), get("cwd"), get("message"));
        let activity = v.get("activity").and_then(|a| a.as_str()).map(str::to_string);
        let to = Focus::agent(&agent, v["desktop"].as_bool().unwrap_or(false));
        let key = if matches!(event.as_str(), "IslandQuestion" | "QuestionDone") {
            format!("{agent}:island:{session}:{}", get("id"))
        } else { format!("{agent}:{session}") };
        let name = match agent.as_str() {
            "claude" => "Claude".to_string(),
            "codex" => "Codex".to_string(),
            other => other.to_string(),
        };
        let folder = Path::new(&cwd).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let label = if folder.is_empty() { name.clone() } else { format!("{name} · {folder}") };

        if event == "QuestionDone" {
            let id = get("id");
            self.decisions.retain(|d| d.id != id);
            self.agents.retain(|a| a.key != key);
            self.drop_cards(&key);
            self.schedule_decisions();
            self.dirty = true;
            return 1;
        }
        if event == "DecisionDone" {
            let id = get("id");
            self.decisions.retain(|d| d.id != id);
            self.dirty = true;
            if agent == "claude" {
                // Claude Code shows its prompt and sends a Notification of its own.
                return 1;
            }
            // Codex has no notification hook: say it's waiting in its own window.
            event = "waiting".into();
            msg = format!("Waiting for your approval in {name}");
        }

        self.agents.retain(|a| !(a.phase == Phase::Done && a.since.elapsed() > Duration::from_secs(900)));
        let idx = self.agents.iter().position(|a| a.key == key);

        if event == "SessionEnd" {
            if let Some(i) = idx {
                self.agents.remove(i);
            }
            self.drop_cards(&key);
            self.dirty = true;
            return 1;
        }

        let mut reply = 1;
        let mut on_island = false;
        let phase = match event.as_str() {
            "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "SubagentStart" | "working" => Phase::Working,
            "PermissionRequest" | "IslandQuestion" => {
                let explicit_question = event == "IslandQuestion";
                let choices = if explicit_question {
                    match crate::ask::choices(&v["choices"]) { Ok(c) => c, Err(_) => return 1 }
                } else { Vec::new() };
                // A card taller than the island: shorten the option notes, then leave them out,
                // before handing the question back to the app.
                let mut detail = get("detail");
                if explicit_question {
                    let room = WIN_H - 32.0 - self.neck;
                    let question = get("question");
                    for limit in [360usize, 220, 120, 0] {
                        if self.question_geometry(&question, &detail, &choices).3 <= room {
                            break;
                        }
                        detail = if limit == 0 { String::new() } else { util::clip(&detail, limit) };
                    }
                    if self.question_geometry(&question, &detail, &choices).3 > room {
                        util::log(&format!("question not shown on the island: too tall ({} choices)", choices.len()));
                        return 1;
                    }
                }
                // When the agent's own window (or the terminal) is in front, answer there.
                let (fg, _) = foreground_exe();
                // (Codex's explicit questions have no prompt of their own to fall back to;
                // Claude's questions and plans do, like its permission prompts.)
                let at_agent = !util::demo() && !(explicit_question && agent == "codex") && !self.cfg.decisions_when_claude_focused && matches!(fg.as_str(), "claude.exe" | "chatgpt.exe" | "windowsterminal.exe");
                if !self.cfg.decisions_on_island || !self.visible || at_agent || get("id").is_empty() {
                    if explicit_question {
                        let why = if !self.cfg.decisions_on_island { "decisions_on_island is off" } else if !self.visible { "something is full screen" } else if at_agent { "its window is in front" } else { "no id" };
                        util::log(&format!("question not shown on the island: {why}"));
                    }
                    if agent == "claude" {
                        // Claude Code follows up with a Notification when it asks.
                        return 1;
                    }
                    // Codex asks in its own window and sends nothing more: show "needs you".
                    msg = [get("question"), get("detail")].join(" ").trim().to_string();
                    Phase::Waiting
                } else {
                let secs = if explicit_question {
                    v["timeout_seconds"].as_u64().unwrap_or(3600).clamp(10, 3600) as u32
                } else { self.cfg.decision_timeout_seconds.clamp(10, 3600) };
                let now = Instant::now();
                self.decisions.push(Decision {
                    id: get("id"),
                    key: key.clone(),
                    label: label.clone(),
                    question: get("question"),
                    detail,
                    multi: explicit_question && v["multi"].as_bool().unwrap_or(false),
                    picked: vec![false; choices.len()],
                    choices,
                    pid: v["pid"].as_u64().unwrap_or(0) as u32,
                    focus: to,
                    since: now,
                    deadline: now + Duration::from_secs(secs as u64),
                });
                self.schedule_decisions();
                reply = agent::DECISION_ACCEPTED + secs as isize;
                on_island = true;
                Phase::Waiting
                }
            }
            "Notification" | "waiting" => {
                let prev = idx.map(|i| self.agents[i].phase);
                // Claude's "waiting for your input" nudge after a finished turn is not news.
                if msg.to_lowercase().contains("waiting for your input") && prev != Some(Phase::Working) {
                    return 1;
                }
                Phase::Waiting
            }
            "Stop" | "agent-turn-complete" | "done" => {
                if agent == "claude" {
                    let _ = self.usage_tx.send(UsageCmd::ClaudeActivity);
                }
                Phase::Done
            }
            _ => return 1,
        };

        // Pip hops when an agent starts waiting for you.
        if phase == Phase::Waiting && idx.map(|i| self.agents[i].phase) != Some(Phase::Waiting) {
            self.hop = Some(Instant::now());
        }
        match idx {
            Some(i) => {
                let a = &mut self.agents[i];
                if a.phase != phase {
                    a.since = Instant::now();
                }
                a.phase = phase;
                a.label = label.clone();
                a.focus = to;
                if let Some(act) = &activity {
                    a.activity = act.clone();
                }
                if phase == Phase::Done {
                    a.activity.clear();
                }
            }
            None => {
                self.agents.push(Agent {
                    key: key.clone(),
                    label: label.clone(),
                    phase,
                    since: Instant::now(),
                    activity: activity.clone().unwrap_or_default(),
                    focus: to,
                });
                if self.agents.len() > 4 {
                    if let Some(i) = self.agents.iter().position(|a| a.phase == Phase::Done) {
                        self.agents.remove(i);
                    } else {
                        self.agents.remove(0);
                    }
                }
            }
        }

        match phase {
            Phase::Working => self.drop_cards(&key),
            // The prompt itself is on the island; no "needs you" card on top of it.
            Phase::Waiting if on_island => self.drop_cards(&key),
            Phase::Waiting => self.push_card(Card {
                key: Some(key),
                glyph: G_ROBOT,
                tint: self.theme.accent,
                app: name,
                title: format!("{label} needs you"),
                body: if msg.is_empty() { "Waiting for your answer".into() } else { msg },
                icon: None,
                sticky: true,
                ttl: Duration::from_secs(3600),
                focus: to,
                kind: CardKind::Plain,
            }),
            // Working quietly: no "finished" card either (the Agents tab still has it).
            Phase::Done if !self.cfg.agent_status_in_pill => {
                self.drop_cards(&key);
                self.gain_xp(20);
            }
            Phase::Done => {
                self.push_card(Card {
                key: Some(key),
                glyph: G_CHECK,
                tint: self.theme.good,
                app: name,
                title: format!("{label} finished"),
                body: if msg.is_empty() { "Done".into() } else { msg },
                icon: None,
                sticky: false,
                ttl: self.card_ttl(4.0),
                focus: to,
                kind: CardKind::Plain,
                });
                self.gain_xp(20);
            }
        }
        self.dirty = true;
        reply
    }

    /// Answer permission prompt `i` with `agent::ANSWERS[which]` (allow / deny / ask in the app).
    fn answer(&mut self, i: usize, which: usize) {
        if i >= self.decisions.len() {
            return;
        }
        let generic = !self.decisions[i].choices.is_empty();
        let n = self.decisions[i].choices.len();
        if self.decisions[i].multi {
            // Choices toggle; n = open chat; n + 1 = Done (needs at least one pick).
            if which < n {
                self.decisions[i].picked[which] ^= true;
                self.dirty = true;
                return;
            }
            if which == n + 1 && !self.decisions[i].picked.contains(&true) {
                return;
            }
        }
        let max = if !generic { agent::ANSWERS.len() } else if self.decisions[i].multi { n + 2 } else { n + 1 };
        if which >= max { return; }
        let d = self.decisions.remove(i);
        self.gain_xp(8);
        let set = |answer: &str| unsafe {
            let name = HSTRING::from(agent::event_name(&d.id, answer));
            match OpenEventW(EVENT_MODIFY_STATE, false, &name) {
                Ok(h) => {
                    let _ = SetEvent(h);
                    let _ = CloseHandle(h);
                }
                Err(e) => util::log(&format!("decision {}: hook gone ({e})", d.id)),
            }
        };
        if d.multi && which == n + 1 {
            // The picks first, then Done, so the hook reads a complete set.
            for (k, _) in d.picked.iter().enumerate().filter(|(_, p)| **p) {
                set(&crate::ask::answer_name(k));
            }
        }
        let answer = if generic { crate::ask::answer_name(which) } else { agent::ANSWERS[which].to_string() };
        set(&answer);
        util::log(&format!("decision: {} for {}", answer, d.label));
        let answered = if !generic { which < 2 } else if d.multi { which == n + 1 } else { which < n };
        if answered {
            if let Some(a) = self.agents.iter_mut().find(|a| a.key == d.key) {
                a.phase = Phase::Working;
                a.since = Instant::now();
            }
        } else {
            focus(d.focus);
        }
        self.schedule_decisions();
        self.dirty = true;
    }

    /// Drop prompts whose hook gave up (timed out, or Claude cancelled the turn).
    fn prune_decisions(&mut self) {
        let now = Instant::now();
        let before = self.decisions.len();
        self.decisions.retain(|d| now < d.deadline && alive(d.pid));
        if self.decisions.len() != before {
            self.dirty = true;
        }
        self.schedule_decisions();
    }

    fn schedule_decisions(&self) {
        unsafe {
            if self.decisions.is_empty() {
                let _ = KillTimer(Some(main_hwnd()), T_DECIDE);
            } else {
                SetTimer(Some(main_hwnd()), T_DECIDE, 1000, None);
            }
        }
    }

    fn refresh_calendar(&mut self) {
        let now = calendar::local_now();
        self.today = now.date;
        self.cal_today = self.cal.on(now.date);
        self.cal_next = self.cal.next_after(now.date, 180);
    }

    /// Update the pill's clock and wake again when the next minute starts. Only redraws when
    /// the clock is actually on screen.
    fn tick_clock(&mut self) {
        let soon = match self.cfg.meeting_sign_minutes {
            0 => None,
            n => self.cal.upcoming(&calendar::local_now(), n),
        };
        if soon != self.meeting {
            self.meeting = soon;
            self.dirty = true;
        }
        let clock = calendar::clock_text();
        if clock != self.clock {
            self.clock = clock;
            if self.mode == Mode::Compact && matches!(self.compact(), Compact::Usage) {
                self.dirty = true;
            }
        }
        unsafe {
            SetTimer(Some(main_hwnd()), T_CLOCK, calendar::ms_to_next_minute(), None);
        }
    }

    fn schedule_calendar(&mut self) {
        let ms = self.cal.next_wake_ms(&calendar::local_now(), self.cfg.calendar_reminder_minutes);
        unsafe {
            SetTimer(Some(main_hwnd()), T_CAL, ms, None);
        }
    }

    // ---------- input ----------

    fn logical(&self, lp: LPARAM) -> (f32, f32) {
        let x = (lp.0 & 0xffff) as u16 as i16 as f32;
        let y = ((lp.0 >> 16) & 0xffff) as u16 as i16 as f32;
        (x / self.scale, y / self.scale)
    }

    fn mouse_move(&mut self, lp: LPARAM) {
        unsafe {
            if !self.tracking {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: self.hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
                self.tracking = true;
            }
            if !self.hover {
                self.hover = true;
                let _ = KillTimer(Some(self.hwnd), T_LEAVE);
                SetTimer(Some(self.hwnd), T_HOVER, 260, None);
                self.schedule_expiry();
            }
        }
        let (x, y) = self.logical(lp);
        // The cursor light follows along.
        self.mouse = (x, y);
        if self.mode == Mode::Compact && self.mon_rect.is_some_and(|r| r.inset(-3.0).contains(x, y)) {
            self.pet_at.get_or_insert(Instant::now());
        } else {
            self.pet_at = None;
        }
        self.dirty = true;
        if let Some(s) = self.dragging {
            self.slide(s, x);
        }
        let hot = self.buttons.iter().find(|(r, _)| r.contains(x, y)).map(|(_, b)| *b);
        if hot != self.hot {
            self.hot = hot;
            self.dirty = true;
        }
    }

    fn remember_clip(&mut self, clip: &peeks::Clip) {
        if !self.cfg.clipboard_history {
            return;
        }
        let item = match clip {
            peeks::Clip::Text(t) if !t.trim().is_empty() && t.len() <= 1 << 20 => HistClip::Text(t.clone()),
            peeks::Clip::Image(_, w, h, dib) if !dib.is_empty() && dib.len() <= 64 << 20 => HistClip::Image { w: *w, h: *h, dib: std::rc::Rc::new(dib.clone()) },
            _ => return,
        };
        self.clips.retain(|c| c != &item);
        self.clips.push_front(item);
        // Twenty at most, and only the five newest pictures (they're big).
        let mut pics = 0;
        self.clips.retain(|c| {
            if matches!(c, HistClip::Image { .. }) {
                pics += 1;
                pics <= 5
            } else {
                true
            }
        });
        self.clips.truncate(CLIPS_KEPT);
        self.save_clips();
    }

    /// Keep the copied text for after a restart (pictures stay in memory only).
    fn save_clips(&self) {
        let texts: Vec<String> = self
            .clips
            .iter()
            .filter_map(|c| match c {
                HistClip::Text(t) if t.len() <= 64 << 10 => Some(t.clone()),
                _ => None,
            })
            .collect();
        crate::notes::save_clips(&texts);
    }

    /// Put an earlier copy back on the clipboard (it moves to the top).
    fn recopy(&mut self, i: usize) {
        let Some(item) = self.clips.remove(i) else { return };
        let ok = match &item {
            HistClip::Text(t) => crate::shelf::put_text(self.hwnd, t),
            HistClip::Image { dib, .. } => crate::shelf::put_dib(self.hwnd, dib),
        };
        self.clips.push_front(item);
        self.clip_page = 0;
        self.save_clips();
        if ok {
            self.clip_flash = Some((0, Instant::now()));
            self.last_clip = Some(Instant::now());
        }
    }

    /// Keep a file on the shelf (and show it there).
    fn shelve(&mut self, p: PathBuf) {
        if !self.shelf.contains(&p) {
            self.shelf.push(p);
            while self.shelf.len() > 12 {
                self.shelf.remove(0);
            }
            crate::shelf::save(&self.shelf);
        }
        self.tab = Tab::Shelf;
        self.expanded = true;
        self.hop = Some(Instant::now());
        self.leave_later(2200);
    }

    /// A file finished downloading: peek with its name and size.
    fn download_card(&mut self, path: PathBuf, size: u64) {
        if !self.cfg.download_peek {
            return;
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        util::log(&format!("downloaded: {name} ({})", crate::scan::human(size)));
        let ttl = self.card_ttl(6.0);
        self.push_card(Card {
            key: Some("download".into()),
            glyph: crate::shelf::glyph(&path),
            tint: self.theme.accent,
            app: "Downloaded".into(),
            title: name,
            body: format!("{} · click to open", crate::scan::human(size)),
            icon: None,
            sticky: false,
            ttl,
            focus: Focus::No,
            kind: CardKind::Download(path),
        });
    }

    /// The CPU or GPU has been running hot for a while.
    fn heat_card(&mut self, h: crate::heat::Heat) {
        use crate::heat::Part;
        if !self.cfg.heat_peek {
            return;
        }
        let mins = (h.secs / 60).max(1);
        let long = if h.secs < 120 { "over a minute".to_string() } else { format!("{mins} minutes") };
        let (app, title) = match (h.part, h.temp, h.limit) {
            (Part::Gpu, Some(t), _) => (self.sys.as_ref().and_then(|s| s.gpu.as_ref()).map_or("GPU".to_string(), |g| g.name.clone()), format!("GPU running hot · {t}°C")),
            (Part::Cpu, _, Some(l)) => ("CPU".to_string(), format!("CPU held back to {l}% speed")),
            _ => ("CPU".to_string(), format!("CPU flat out · {:.0}%", h.load * 100.0)),
        };
        let why = match h.part {
            Part::Gpu => format!("For {long}"),
            Part::Cpu if h.limit.is_some() => format!("Windows has been slowing it for {long}, usually from heat"),
            Part::Cpu => format!("Busy for {long}"),
        };
        let what = match h.part {
            Part::Gpu => "GPU",
            Part::Cpu => "CPU",
        };
        let body = match &h.app {
            Some((n, share)) => format!("{why} · mostly {n} ({:.0}% of the {what})", share * 100.0),
            None => why,
        };
        util::log(&format!("heat: {title}"));
        let ttl = self.card_ttl(7.0);
        let tint = self.theme.bad;
        self.push_card(Card {
            key: Some(format!("heat {what}")),
            glyph: G_HEAT,
            tint,
            app,
            title,
            body,
            icon: None,
            sticky: false,
            ttl,
            focus: Focus::No,
            kind: CardKind::Plain,
        });
    }

    /// Your pull requests: checks finished, or someone reviewed.
    fn github_cards(&mut self, notes: Vec<crate::github::Note>) {
        use crate::github::NoteKind;
        if !self.cfg.github_peeks {
            return;
        }
        for n in notes {
            let (app, tint) = match n.kind {
                NoteKind::Passed => ("Checks passed".to_string(), self.theme.good),
                NoteKind::Failed => ("Checks failed".to_string(), self.theme.bad),
                NoteKind::Approved => (format!("{} approved", n.who), self.theme.good),
                NoteKind::ChangesRequested => (format!("{} asked for changes", n.who), self.theme.bad),
                NoteKind::Commented => (format!("{} reviewed", n.who), self.theme.accent),
            };
            util::log(&format!("GitHub: {app} · {}", n.place));
            let ttl = self.card_ttl(6.0);
            self.push_card(Card {
                key: Some(format!("gh {}", n.url)),
                glyph: G_GITHUB,
                tint,
                app: format!("GitHub · {app}"),
                title: n.title.clone(),
                body: format!("{} · click to open", n.place),
                icon: None,
                sticky: false,
                ttl,
                focus: Focus::No,
                kind: CardKind::Link(n.url.clone()),
            });
        }
    }

    /// The sound outputs to offer (Steam's streaming ones are left out).
    fn refresh_outputs(&mut self) {
        self.outputs = crate::devices::outputs().into_iter().filter(|o| !o.hw.contains("Steam Streaming")).collect();
        self.dirty = true;
    }

    fn outputs_height(&self) -> f32 {
        let n = self.outputs.len().min(OUTPUTS_SHOWN);
        if n == 0 {
            return 0.0;
        }
        let rows = n.div_ceil(3) as f32;
        12.0 + 20.0 + rows * 40.0 + (rows - 1.0) * 8.0
    }

    fn notes_height(&self) -> f32 {
        12.0 + 20.0 + self.notes.len().min(crate::notes::MAX_NOTES) as f32 * 26.0 + 32.0
    }

    /// A key typed while adding a note (Esc stops, Enter adds it).
    fn note_key(&mut self, unit: u16) {
        if !self.note_typing {
            return;
        }
        self.dirty = true;
        if unit == 27 {
            self.note_typing = false;
            self.leave_later(400);
            return;
        }
        let own = self.hwnd;
        let paste = || match peeks::clipboard(own) {
            Some(peeks::Clip::Text(t)) => Some(t),
            _ => None,
        };
        if crate::notes::type_key(&mut self.note_draft, &mut self.note_hi, unit, paste) {
            let text = self.note_draft.trim().to_string();
            if !text.is_empty() && self.notes.len() < crate::notes::MAX_NOTES {
                self.notes.push(crate::notes::Note { text, done: false });
                crate::notes::save(&self.notes);
            }
            self.note_draft.clear();
        }
    }

    /// The island lost the keyboard (you clicked somewhere else): stop typing.
    fn note_blur(&mut self) {
        if self.note_typing {
            self.note_typing = false;
            self.dirty = true;
            self.leave_later(300);
        }
    }

    /// Files dragged over (1), away (0) or dropped (2) on this island.
    fn drag_event(&mut self, what: usize) {
        match what {
            1 => {
                self.drag_over = true;
                self.tab = Tab::Shelf;
                self.expanded = true;
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), T_LEAVE);
                }
            }
            2 => {
                self.drag_over = false;
                let mut added = false;
                for p in crate::shelf::take_dropped() {
                    if !self.shelf.contains(&p) {
                        self.shelf.push(p);
                        added = true;
                    }
                }
                while self.shelf.len() > 12 {
                    self.shelf.remove(0);
                }
                if added {
                    crate::shelf::save(&self.shelf);
                }
                self.tab = Tab::Shelf;
                self.expanded = true;
                self.hop = Some(Instant::now());
                self.leave_later(1800);
            }
            _ => {
                self.drag_over = false;
                self.leave_later(450);
            }
        }
        self.dirty = true;
    }

    /// pocketisle.exe was opened again (say by double-clicking it) while this one runs: open
    /// the island for a few seconds, so it's clear where it is.
    fn opened_again(&mut self) {
        if self.mode == Mode::Compact {
            self.refresh_calendar();
            self.pick_tab();
            self.expanded = true;
            self.hop = Some(Instant::now());
            self.dirty = true;
            self.leave_later(4000);
        }
    }

    fn leave_later(&mut self, ms: u32) {
        if !self.hover {
            unsafe {
                SetTimer(Some(self.hwnd), T_LEAVE, ms, None);
            }
        }
    }

    /// The pointer moved with the button down: far enough from a pressed shelf file, and it's
    /// dragged out (the caller runs the drag, outside the app).
    fn shelf_drag(&mut self, lp: LPARAM) -> Option<PathBuf> {
        let (i, x0, y0) = self.shelf_press?;
        let (x, y) = self.logical(lp);
        if (x - x0).powi(2) + (y - y0).powi(2) < 36.0 {
            return None;
        }
        self.shelf_press = None;
        self.shelf_dragged = true;
        self.shelf.get(i).filter(|p| p.exists()).cloned()
    }

    fn shelf_height(&self) -> f32 {
        let files = if self.shelf.is_empty() { 52.0 } else { self.shelf.len().div_ceil(4) as f32 * (58.0 + 8.0) - 8.0 };
        files + 12.0 + 20.0 + self.clips.len().clamp(1, CLIP_ROWS) as f32 * 28.0
    }

    fn draw_shelf_tab(&mut self, x0: f32, x1: f32, mut y: f32) {
        let th = self.theme.clone();
        let t = self.t0.elapsed().as_secs_f32();
        let pulse = 0.6 + 0.4 * (t * 5.0).sin().abs();
        if self.shelf.is_empty() {
            let r = R::new(x0, y, x1, y + 52.0);
            self.r.fill_round(r, 12.0, if self.drag_over { th.accent.a(0.1 + 0.1 * pulse) } else { th.bg2.a(0.45) });
            self.r.stroke_round(r.inset(0.5), 11.5, if self.drag_over { th.accent.a(0.8) } else { th.border }, 1.0);
            self.r.icon(G_SHELF, R::xywh(x0 + 10.0, y, 30.0, 52.0), 17.0, th.accent);
            let msg = if self.drag_over { "Let go to keep it here" } else { "Drag files onto the island to keep them here" };
            self.r.text(msg, R::new(x0 + 44.0, y, x1 - 10.0, y + 52.0), 12.0, 500, th.subtext, Align::Left);
            y += 52.0;
        } else {
            let gap = 8.0;
            let cw = (x1 - x0 - 3.0 * gap) / 4.0;
            let rows = self.shelf.len().div_ceil(4) as f32;
            if self.drag_over {
                let all = R::new(x0 - 4.0, y - 4.0, x1 + 4.0, y + rows * (58.0 + gap) - gap + 4.0);
                self.r.stroke_round(all, 15.0, th.accent.a(0.5 + 0.4 * pulse), 1.5);
            }
            for (i, path) in self.shelf.clone().iter().enumerate() {
                let r = R::xywh(x0 + (i % 4) as f32 * (cw + gap), y + (i / 4) as f32 * (58.0 + gap), cw, 58.0);
                let hot = matches!(self.hot, Some(Btn::ShelfFile(j)) | Some(Btn::ShelfRemove(j)) if j == i);
                let a = if path.exists() { 1.0 } else { 0.4 };
                self.r.fill_round(r, 12.0, if hot { th.hover } else { th.bg2.a(0.6) });
                self.r.icon(crate::shelf::glyph(path), R::new(r.x0, r.y0 + 6.0, r.x1, r.y0 + 32.0), 18.0, th.accent.a(a));
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.r.text(&util::clip(&name, 14), R::new(r.x0 + 4.0, r.y0 + 33.0, r.x1 - 4.0, r.y0 + 52.0), 11.0, 500, th.text.a(a), Align::Center);
                if hot {
                    let xr = R::xywh(r.x1 - 20.0, r.y0 + 3.0, 17.0, 17.0);
                    let xhot = self.hot == Some(Btn::ShelfRemove(i));
                    self.r.fill_round(xr, 8.5, if xhot { th.bad.a(0.85) } else { th.bg });
                    self.r.icon(G_CLOSE, xr, 10.0, if xhot { th.text } else { th.subtext });
                    // Before the tile, so a click on the cross finds it first.
                    self.buttons.push((xr, Btn::ShelfRemove(i)));
                }
                self.buttons.push((r, Btn::ShelfFile(i)));
            }
            y += rows * (58.0 + gap) - gap;
        }
        y += 12.0;
        self.r.text("Recent copies", R::new(x0 + 2.0, y, x1, y + 20.0), 11.5, 600, th.subtext, Align::Left);
        let pages = self.clips.len().div_ceil(CLIP_ROWS).max(1);
        self.clip_page = self.clip_page.min(pages - 1);
        let page = self.clip_page;
        if !self.clips.is_empty() {
            let clear = R::new(x1 - 46.0, y + 1.0, x1, y + 19.0);
            let hot = self.hot == Some(Btn::ClipClear);
            self.r.fill_round(clear, 9.0, if hot { th.bad.a(0.85) } else { th.bg2.a(0.6) });
            self.r.text("Clear", clear, 10.5, 600, th.text, Align::Center);
            self.buttons.push((clear, Btn::ClipClear));
            if pages > 1 {
                for (r, d, g) in [(R::new(x1 - 132.0, y, x1 - 112.0, y + 20.0), -1, "‹"), (R::new(x1 - 74.0, y, x1 - 54.0, y + 20.0), 1, "›")] {
                    if self.hot == Some(Btn::ClipPage(d)) {
                        self.r.fill_round(r, 10.0, th.hover);
                    }
                    self.r.text(g, r, 14.0, 700, th.text, Align::Center);
                    self.buttons.push((r, Btn::ClipPage(d)));
                }
                self.r.text(&format!("{} / {pages}", page + 1), R::new(x1 - 112.0, y, x1 - 74.0, y + 20.0), 10.5, 500, th.subtext, Align::Center);
                self.r.text("Click to copy again", R::new(x0, y, x1 - 140.0, y + 20.0), 10.5, 400, th.subtext.a(0.8), Align::Right);
            } else {
                self.r.text("Click to copy again", R::new(x0, y, x1 - 54.0, y + 20.0), 10.5, 400, th.subtext.a(0.8), Align::Right);
            }
        }
        y += 20.0;
        if self.clips.is_empty() {
            let msg = if self.cfg.clipboard_history { "Copy something and it shows up here" } else { "Clipboard history is off" };
            self.r.text(msg, R::new(x0 + 2.0, y, x1, y + 28.0), 12.0, 400, th.subtext, Align::Left);
            return;
        }
        for (k, c) in self.clips.iter().skip(page * CLIP_ROWS).take(CLIP_ROWS).cloned().enumerate() {
            let i = page * CLIP_ROWS + k;
            let r = R::new(x0, y, x1, y + 26.0);
            let hot = matches!(self.hot, Some(Btn::Clip(j)) | Some(Btn::ClipRemove(j)) if j == i);
            if hot {
                self.r.fill_round(r, 9.0, th.hover);
                let xr = R::xywh(x1 - 24.0, y + 4.0, 18.0, 18.0);
                let xhot = self.hot == Some(Btn::ClipRemove(i));
                self.r.fill_round(xr, 9.0, if xhot { th.bad.a(0.85) } else { th.bg });
                self.r.icon(G_CLOSE, xr, 10.0, if xhot { th.text } else { th.subtext });
                self.buttons.push((xr, Btn::ClipRemove(i)));
            }
            let flash = self.clip_flash.is_some_and(|(j, at)| j == i && at.elapsed() < Duration::from_millis(1500));
            let (g, label) = match &c {
                HistClip::Text(s) => (G_CLIPBOARD, util::clip(&peeks::preview(s), 58)),
                HistClip::Image { w, h, .. } => (G_SCREENSHOT, format!("Picture · {w} × {h}")),
            };
            self.r.icon(if flash { G_CHECK } else { g }, R::xywh(x0 + 4.0, y, 22.0, 26.0), 13.0, if flash { th.good } else { th.accent });
            self.r.text(&label, R::new(x0 + 30.0, y, x1 - 30.0, y + 26.0), 12.0, 400, th.text, Align::Left);
            self.buttons.push((r, Btn::Clip(i)));
            y += 28.0;
        }
    }

    /// Clicked the Pokémon: it hops and cries, and a few hearts pop out.
    fn poke(&mut self) {
        if self.hp_now().is_some_and(|h| h <= FAINT) {
            // Fainted: no hop and no cry, just a little sympathy.
            if let Some(r) = self.mon_rect {
                self.hearts.push((r.cx(), r.y0 + 6.0, Instant::now()));
            }
            return;
        }
        let name = self.mascot_sprite(self.compact_mood());
        if self.cfg.pokemon_cries && self.mic_apps.is_empty() {
            crate::sound::cry(&name, 0.5);
        }
        let now = Instant::now();
        self.hop = Some(now);
        if let Some(r) = self.mon_rect {
            for k in 0..3 {
                let x = r.cx() + (k as f32 - 1.0) * r.w() * 0.25;
                self.hearts.push((x, r.y0 + 6.0, now - Duration::from_millis(k * 140)));
            }
        }
        if self.poke_xp_at.is_none_or(|t| t.elapsed() > Duration::from_secs(30)) {
            self.poke_xp_at = Some(now);
            // Petting your buddy makes it fonder of you (up to a daily limit).
            if self.buddy_sprite().is_some_and(|b| b == name) {
                let today = self.today_key();
                if self.dex.pet_day != today {
                    self.dex.pet_day = today;
                    self.dex.pet_today = 0;
                }
                if self.dex.pet_today < PET_DAILY {
                    self.dex.pet_today += PET_GAIN;
                    self.add_friendship(PET_GAIN);
                }
            }
            self.gain_xp(1);
        }
    }

    /// Your buddy's sprite, when you have one, it's been downloaded and the Pokémon mascot
    /// is on.
    fn buddy_sprite(&self) -> Option<String> {
        let b = &self.dex.buddy;
        if b.is_empty() || !self.cfg.mascot || !self.cfg.mascot_pokemon {
            return None;
        }
        let shiny = format!("{b}-shiny");
        if self.dex.shiny.contains_key(b) && pokemon::has_sprite(&shiny) {
            return Some(shiny);
        }
        pokemon::has_sprite(b).then(|| b.clone())
    }

    fn today_key(&self) -> String {
        let n = calendar::local_now();
        format!("{:04}-{:02}-{:02}", n.date.y, n.date.m, n.date.d)
    }

    /// Make this caught Pokémon your buddy (or, if it already is, stop).
    fn set_buddy(&mut self, name: &'static str) {
        if !self.dex.caught.contains_key(name) {
            return;
        }
        if self.dex.buddy == name {
            self.dex.buddy.clear();
            util::log(&format!("buddy: {name} rests"));
            self.dex.save();
            self.update_keeper(false);
        } else {
            self.dex.buddy = name.to_string();
            self.dex.friendship.entry(name.to_string()).or_insert(pokemon::FRIEND_START);
            util::log(&format!("buddy: {name}"));
            self.dex.save();
            if self.cfg.pokemon_cries && self.mic_apps.is_empty() {
                crate::sound::cry(name, 0.5);
            }
            self.hop = Some(Instant::now());
        }
        self.dirty = true;
    }

    /// More friendship for your buddy; a happy friendship evolver evolves.
    fn add_friendship(&mut self, n: u32) {
        if self.dex.buddy.is_empty() {
            return;
        }
        let f = self.dex.friendship.entry(self.dex.buddy.clone()).or_insert(pokemon::FRIEND_START);
        *f = (*f + n).min(pokemon::FRIEND_MAX);
        self.dex.save();
        self.try_friend_evolve();
    }

    /// A minute at the PC with your buddy out: a friendship point every five.
    fn buddy_minute(&mut self) {
        let here = user_idle_ms() < 60_000 || self.playing;
        if !here || self.buddy_sprite().is_none() {
            return;
        }
        self.dex.buddy_minutes += 1;
        if self.dex.buddy_minutes % WALK_MINUTES == 0 {
            self.add_friendship(1);
        } else {
            self.try_friend_evolve();
        }
    }

    /// Happy enough (and the right time of day): your buddy evolves. Only on the pill, so
    /// you see it.
    fn try_friend_evolve(&mut self) {
        let b = self.dex.buddy.clone();
        let f = self.dex.friendship.get(&b).copied().unwrap_or(0);
        if f < pokemon::FRIEND_EVOLVE || self.mode != Mode::Compact || self.wild.is_some() || self.hatch.is_some() {
            return;
        }
        let day = self.conditions().day;
        let Some(to) = pokemon::friend_evolution(&b, day) else { return };
        *self.dex.caught.entry(to.to_string()).or_default() += 1;
        *self.dex.seen.entry(to.to_string()).or_default() += 1;
        let shiny = self.dex.shiny.contains_key(&b);
        if shiny {
            *self.dex.shiny.entry(to.to_string()).or_default() += 1;
        }
        let shiny_sprite = format!("{to}-shiny");
        self.dex.remember(if shiny && pokemon::has_sprite(&shiny_sprite) { shiny_sprite } else { to.to_string() });
        self.dex.buddy = to.to_string();
        let keep = self.dex.friendship.get(to).copied().unwrap_or(0).max(f);
        self.dex.friendship.insert(to.to_string(), keep);
        self.dex.save();
        self.dex_thumbs.clear();
        self.hop = Some(Instant::now());
        self.dirty = true;
        util::log(&format!("friendship evolution: {b} → {to} ({})", if day { "day" } else { "night" }));
        if self.cfg.pokemon_cries && self.mic_apps.is_empty() {
            crate::sound::cry(to, 0.5);
        }
        let ttl = self.card_ttl(6.0);
        self.push_card(Card {
            key: Some("friend-evo".into()),
            glyph: G_DEX,
            tint: self.theme.good,
            app: "Your buddy".into(),
            title: format!("Your {} evolved into {}!", pokemon::pretty(&b), pokemon::pretty(to)),
            body: "It's that fond of you · +50 XP".into(),
            icon: pokemon::first_frame(to),
            sticky: false,
            ttl,
            focus: Focus::No,
            kind: CardKind::Plain,
        });
        self.gain_xp(50);
    }

    fn gain_xp(&mut self, n: u32) {
        let old = self.dex.level();
        self.dex.xp += n as u64;
        let new = self.dex.level();
        self.dex.save();
        self.xp_pops.push((n, Instant::now()));
        self.dirty = true;
        if new > old {
            self.level_up(old, new);
        }
        self.check_goals();
    }

    /// The team levelled up: say who evolved (or turned shiny).
    fn level_up(&mut self, old: u32, new: u32) {
        let met = self.dex.met.clone();
        let changes = pokemon::changes_of(old, new, |n| met.iter().any(|m| m == n));
        let what = |a: &str, b: &str| {
            if b.ends_with("-shiny") {
                format!("{} turned shiny!", pokemon::pretty(a))
            } else {
                format!("{} evolved into {}!", pokemon::pretty(a), pokemon::pretty(b))
            }
        };
        let (title, body, star) = match changes.as_slice() {
            [] => ("Level up!".to_string(), format!("Your team reached Lv {new}"), None),
            [(a, b)] => (what(a, b), format!("Your team reached Lv {new}"), Some(b.clone())),
            many => {
                let list = many
                    .iter()
                    .map(|(a, b)| if b.ends_with("-shiny") { format!("{} ✨", pokemon::pretty(a)) } else { format!("{} → {}", pokemon::pretty(a), pokemon::pretty(b)) })
                    .collect::<Vec<_>>()
                    .join(", ");
                (format!("{} of your Pokémon evolved!", many.len()), list, Some(many[0].1.clone()))
            }
        };
        if let Some(n) = &star {
            if self.cfg.pokemon_cries && self.mic_apps.is_empty() {
                crate::sound::cry(n, 0.5);
            }
        }
        let evolved = self.evolve_caught();
        let face = star.unwrap_or_else(|| self.mascot_sprite(self.pip_mood()));
        let icon = pokemon::first_frame(&face);
        let ttl = self.card_ttl(6.0);
        self.push_card(Card {
            key: Some("levelup".into()),
            glyph: G_DEX,
            tint: self.theme.accent,
            app: format!("Team Lv {new}"),
            title,
            body,
            icon,
            sticky: false,
            ttl,
            focus: Focus::No,
            kind: CardKind::Plain,
        });
        self.evolution_card(&evolved);
    }

    /// Hearts and "+XP" floating up from the Pokémon.
    fn draw_hearts(&mut self, mon: R) {
        let s = self.scale;
        let px = (1.2 * s).round().max(1.0) / s;
        const SHAPE: [&str; 4] = [".X.X.", "XXXXX", ".XXX.", "..X.."];
        for (hx, hy, born) in self.hearts.clone() {
            let u = (born.elapsed().as_secs_f32() / 1.3).clamp(0.0, 1.0);
            let x = ((hx + (u * 7.0 + hx).sin() * 2.5 - 2.5 * px) * s).round() / s;
            let y = ((hy - 16.0 * u) * s).round() / s;
            for (j, row) in SHAPE.iter().enumerate() {
                for (i, c) in row.chars().enumerate() {
                    if c == 'X' {
                        self.r.rect(R::xywh(x + i as f32 * px, y + j as f32 * px, px, px), HEART.a(1.0 - u * u));
                    }
                }
            }
        }
        let th = self.theme.clone();
        for (n, at) in self.xp_pops.clone() {
            let u = (at.elapsed().as_secs_f32() / 1.4).clamp(0.0, 1.0);
            let y = mon.y0 - 2.0 - 10.0 * u;
            let r = R::new(mon.cx() - 30.0, y - 12.0, mon.cx() + 30.0, y);
            self.r.text(&format!("+{n} XP"), r, 10.0, 700, th.accent.a(1.0 - u * u), Align::Center);
        }
    }

    /// Each frame: hearts while petting (and a little XP for it), visitors leaving, catches landing.
    fn step_pokemon(&mut self) {
        let now = Instant::now();
        self.hearts.retain(|h| h.2.elapsed() < Duration::from_millis(1300));
        self.xp_pops.retain(|p| p.1.elapsed() < Duration::from_millis(1400));
        if let (Some(since), Some(r)) = (self.pet_at, self.mon_rect) {
            if since.elapsed() > Duration::from_millis(350) && self.last_heart.elapsed() > Duration::from_millis(480) {
                self.hearts.push((r.cx() + (rand01() - 0.5) * r.w() * 0.6, r.y0 + 6.0, now));
                self.last_heart = now;
            }
            if since.elapsed() > Duration::from_secs(2) && self.pet_xp_at.is_none_or(|t| t.elapsed() > Duration::from_secs(120)) {
                self.pet_xp_at = Some(now);
                self.gain_xp(3);
            }
        }
        let done = match &self.wild {
            Some(w) => match w.catch {
                // It doesn't leave on its own.
                None => false,
                Some((at, ok)) => at.elapsed().as_secs_f32() > if ok || !w.flee { 2.7 } else { 3.0 },
            },
            None => false,
        };
        if done {
            self.finish_catch();
        }
        let fought = self.wild.as_ref().and_then(|w| w.fight).is_some_and(|(at, _)| at.elapsed().as_secs_f32() > FIGHT_SECS);
        if fought {
            self.finish_battle();
        }
        if self.hatch.is_some_and(|h| h.elapsed().as_secs_f32() > HATCH_SECS) {
            self.finish_hatch();
        }
    }

    /// A random wait before the next wild Pokémon, between the `wild_minutes` bounds.
    fn wild_wait_ms(&self) -> u32 {
        // In a mass outbreak, every 2–4 minutes.
        if self.outbreak_species().is_some() {
            return ((2.0 + rand01() * 2.0) * 60_000.0) as u32;
        }
        let [a, b] = self.cfg.wild_minutes;
        let (lo, hi) = (a.min(b).max(1.0), a.max(b).max(1.0));
        ((lo + rand01() * (hi - lo)) * 60_000.0) as u32
    }

    /// At start-up: carry on with the saved wait; if one was due while Pocket Isle wasn't running,
    /// it comes in 1–3 minutes.
    fn schedule_first_wild(&mut self) {
        let left = self.dex.next_wild - crate::dex::now_unix();
        let ms = if left > 0 { (left as u32).saturating_mul(1000).min(self.wild_wait_ms().max(60_000) * 2) } else { (60.0 + rand01() * 120.0) as u32 * 1000 };
        self.set_wild_timer(ms);
    }

    fn set_wild_timer(&mut self, ms: u32) {
        self.dex.next_wild = crate::dex::now_unix() + (ms / 1000) as i64;
        self.dex.save();
        unsafe {
            SetTimer(Some(main_hwnd()), T_WILD, ms, None);
        }
    }

    /// Main island, every `wild_minutes`: a wild Pokémon wanders by if you're around to see it.
    fn wild_timer(&mut self) {
        let mut next = self.wild_wait_ms();
        if self.cfg.wild_pokemon && self.cfg.mascot {
            let idle = user_idle_ms();
            let why = if self.wild.is_some() {
                "one is already here"
            } else if !self.visible || self.fullscreen {
                "something is full screen"
            } else if self.mode != Mode::Compact {
                "the island is open"
            } else if idle >= 120_000 {
                "you're away"
            } else if self.dnd_on() {
                "do not disturb is on"
            } else {
                ""
            };
            if why.is_empty() {
                let chance = if self.dex.hatched == 0 { 0.3 } else { 0.12 };
                if self.cfg.eggs && self.cfg.mascot_pokemon && self.dex.egg.is_none() && self.hatch.is_none() && rand01() < chance {
                    self.find_egg();
                } else {
                    self.spawn_wild();
                }
            } else {
                // Not a good moment: look again in a minute.
                util::log(&format!("wild Pokémon waiting: {why}"));
                next = 60_000;
            }
        }
        self.set_wild_timer(next);
    }

    fn spawn_wild(&mut self) {
        if self.wild.as_ref().is_some_and(|w| w.catch.is_some()) {
            return;
        }
        self.fishing = None;
        self.fish_rect = None;
        let now = crate::dex::now_unix();
        if self.dex.outbreak.as_ref().is_some_and(|o| o.until <= now) {
            if let Some(o) = self.dex.outbreak.take() {
                util::log(&format!("the {} outbreak is over ({} caught)", o.species, o.caught));
            }
            self.dex.save();
        }
        // On weekends, the raid boss.
        if let Some(boss) = self.raid_due() {
            self.spawn_raid(boss);
            return;
        }
        let outbreak: Option<&'static str> = self.outbreak_species().and_then(pokemon::species).map(|s| s.name).filter(|n| pokemon::has_sprite(n));
        // A special day or night sends its own visitors some of the time.
        let special = self.special();
        let guest = special.and_then(|s| {
            let have: Vec<&'static str> = s.visitors.iter().copied().filter(|n| pokemon::has_sprite(n)).collect();
            (!have.is_empty() && rand01() < s.share).then(|| (have[(rand01() * have.len() as f32) as usize % have.len()], s.legend))
        });
        // A roaming legendary: one the weather brings, or the lake trio member out at this
        // time of day.
        let c = self.conditions();
        let roamers: Vec<(&'static str, &'static str)> = pokemon::weather_legends(&c).into_iter().filter(|(n, _)| pokemon::has_sprite(n)).collect();
        let bird = (!roamers.is_empty() && rand01() < 1.0 / pokemon::WEATHER_LEGEND_ODDS).then(|| roamers[(rand01() * roamers.len() as f32) as usize % roamers.len()]);
        let bird = bird.or_else(|| pokemon::time_legend(calendar::local_now().h, c.day).filter(|(n, _)| pokemon::has_sprite(n) && rand01() < 1.0 / pokemon::TIME_LEGEND_ODDS));
        if let Some((n, w)) = bird {
            util::log(&format!("the {w} brought {n}"));
        }
        let guest = guest.or(bird.map(|(n, _)| (n, true)));
        // An unlocked legend quest's legendary.
        let quest = if guest.is_none() { self.quest_visitor() } else { None };
        if let Some((n, _)) = quest {
            util::log(&format!("legend quest visitor: {n}"));
        }
        let guest = guest.or(quest);
        // Now and then a Gigantamax Pokémon (more often in a storm or strong wind).
        let guest = guest.or_else(|| {
            let have: Vec<&'static str> = pokemon::gigantamax().iter().copied().filter(|n| pokemon::has_sprite(n)).collect();
            (!have.is_empty() && rand01() < pokemon::gigantamax_odds(&self.conditions())).then(|| (have[(rand01() * have.len() as f32) as usize % have.len()], true))
        });
        let from_outbreak = guest.is_none() && outbreak.is_some() && rand01() < crate::quest::OUTBREAK_SHARE;
        let guest = if from_outbreak { outbreak.map(|n| (n, false)) } else { guest };
        let (name, legend) = match guest {
            Some(g) => g,
            None => {
                let legend = rand01() < 1.0 / 30.0;
                // Only the ones whose sprite is downloaded.
                let all: &[&'static str] = if legend { pokemon::legends() } else { pokemon::wild() };
                let have: Vec<&'static str> = all.iter().copied().filter(|n| pokemon::has_sprite(n) && !pokemon::FISH_ONLY.contains(n)).collect();
                let list: &[&'static str] = if have.is_empty() { all } else { &have };
                // The weather, the temperature, the wind and the time of day decide who's
                // about; a catch combo makes that species more likely too.
                let c = self.conditions();
                let (chained, chain) = (self.dex.chain_species.clone(), self.dex.chain);
                let weight = |n: &str| pokemon::spawn_weight(&c, n) * pokemon::rarity(n) * if n == chained { (1.0 + 0.25 * chain as f32).min(3.0) } else { 1.0 };
                let mut pick = rand01() * list.iter().map(|n| weight(n)).sum::<f32>();
                let mut name = list[list.len() - 1];
                for n in list {
                    if pick < weight(n) {
                        name = n;
                        break;
                    }
                    pick -= weight(n);
                }
                (name, legend)
            }
        };
        let why = match special {
            Some(s) => s.line.to_string(),
            None => pokemon::spawn_line(&self.conditions()),
        };
        let shiny = rand01() < 1.0 / self.shiny_odds(name) as f32 && pokemon::has_sprite(&format!("{name}-shiny"));
        *self.dex.seen.entry(name.to_string()).or_default() += 1;
        self.dex.save();
        let why = if from_outbreak { "mass outbreak".to_string() } else { why };
        let why = if why.is_empty() { String::new() } else { format!(" · {why}") };
        // Now and then one wants to battle first (never a legendary or a shiny).
        let battle = self.cfg.battles && !legend && !shiny && rand01() < 0.2;
        // Out in weather that suits it: more XP, easier to catch.
        let boosted = special.is_none() && !from_outbreak && pokemon::weather_boosted(&self.conditions(), name);
        util::log(&format!(
            "wild Pokémon: {name}{}{}{}{}{why}",
            if shiny { " (shiny)" } else { "" },
            if legend { " (legendary)" } else { "" },
            if battle { " (wants to battle)" } else { "" },
            if boosted { " (weather boost)" } else { "" }
        ));
        self.wild = Some(Wild { name, shiny, legend, since: Instant::now(), from_left: rand01() < 0.5, catch: None, u_at_catch: 0.0, tries: 0, quality: 0.0, flee: false, ball: 0, battle, tired: false, fight: None, raid: false, boosted, fished: false });
        self.dex.visitor = Some(crate::dex::Visitor { name: name.to_string(), shiny, legend, battle, tired: false, raid: false, boosted, fished: false });
        self.dex.save();
        self.dirty = true;
        // Sometimes a visit starts a mass outbreak of its kind.
        if self.cfg.outbreaks && outbreak.is_none() && guest.is_none() && !legend && rand01() < crate::quest::OUTBREAK_CHANCE {
            self.dex.outbreak = Some(crate::quest::Outbreak { species: name.to_string(), until: now + crate::quest::OUTBREAK_SECS, caught: 0 });
            self.dex.save();
            util::log(&format!("mass outbreak: {name} for an hour"));
            let tint = self.theme.accent;
            let body = format!("For the next hour · shiny odds 1 in {}", self.shiny_odds(name));
            self.quick_card("outbreak", G_DEX, tint, "Mass outbreak!", &format!("{} are appearing everywhere!", pokemon::pretty(name)), &body, 6.0);
        }
    }

    /// This weekend's raid (made when the weekend starts), if raids are on and it's a weekend.
    fn ensure_raid(&mut self) -> Option<crate::quest::Raid> {
        if !self.cfg.raids {
            return None;
        }
        let sat = crate::quest::raid_weekend(calendar::local_now().date)?;
        let key = format!("{:04}-{:02}-{:02}", sat.y, sat.m, sat.d);
        if self.dex.raid.as_ref().is_none_or(|r| r.weekend != key) {
            let pool: Vec<&'static str> = pokemon::legends().iter().copied().filter(|n| pokemon::has_sprite(n)).collect();
            let species = crate::quest::raid_species(&key, &pool)?;
            self.dex.raid = Some(crate::quest::Raid { weekend: key, species: species.to_string(), shields: crate::quest::RAID_SHIELDS, ..Default::default() });
            self.dex.save();
        }
        self.dex.raid.clone()
    }

    /// Should this visitor be the raid boss? The weekend's first visitor is; after that it
    /// comes back now and then until it's caught or gets away.
    fn raid_due(&mut self) -> Option<&'static str> {
        let r = self.ensure_raid()?;
        if r.done {
            return None;
        }
        let name = pokemon::species(&r.species).map(|s| s.name).filter(|n| pokemon::has_sprite(n))?;
        (!r.met || rand01() < crate::quest::RAID_SHARE).then_some(name)
    }

    fn spawn_raid(&mut self, name: &'static str) {
        let Some(r) = self.dex.raid.clone() else { return };
        if let Some(r) = self.dex.raid.as_mut() {
            r.met = true;
        }
        *self.dex.seen.entry(name.to_string()).or_default() += 1;
        let battle = r.shields > 0;
        util::log(&format!("raid boss: {name} ({} shields left)", r.shields));
        self.wild = Some(Wild {
            name,
            shiny: false,
            legend: true,
            since: Instant::now(),
            from_left: rand01() < 0.5,
            catch: None,
            u_at_catch: 0.0,
            tries: 0,
            quality: 0.0,
            flee: false,
            ball: 0,
            battle,
            tired: !battle,
            fight: None,
            raid: true,
            boosted: false,
            fished: false,
        });
        self.dex.visitor = Some(crate::dex::Visitor { name: name.to_string(), shiny: false, legend: true, battle, tired: !battle, raid: true, boosted: false, fished: false });
        self.dex.save();
        self.dirty = true;
        if !r.met {
            let tint = self.theme.accent;
            let body = format!("Win {} battles to break its shields, then catch it · all weekend", crate::quest::RAID_SHIELDS);
            self.quick_card("raid", G_SHIELD, tint, "Weekend raid!", &format!("{} is raiding the island!", pokemon::pretty(name)), &body, 7.0);
        }
    }

    /// The Bag page's line about raids.
    fn raid_line(&mut self) -> String {
        let today = calendar::local_now().date;
        match self.ensure_raid() {
            Some(r) if r.caught => format!("Weekend raid: you caught {}! A new raid next weekend", pokemon::pretty(&r.species)),
            Some(r) if r.done => format!("Weekend raid: {} got away · a new raid next weekend", pokemon::pretty(&r.species)),
            Some(r) if !r.met => "Weekend raid: a strong legendary turns up today · battle it, then catch it".to_string(),
            Some(r) if r.shields == 0 => format!("Weekend raid: {}'s shields are down · catch it!", pokemon::pretty(&r.species)),
            Some(r) => format!("Weekend raid: {} · {} of {} shields left · win battles to break them", pokemon::pretty(&r.species), r.shields, crate::quest::RAID_SHIELDS),
            None if !self.cfg.raids => "Weekend raids are off (raids in the settings)".to_string(),
            None => {
                let d = crate::quest::days_to_raid(today);
                format!("Next weekend raid in {d} day{} · a strong legendary to battle, then catch", if d == 1 { "" } else { "s" })
            }
        }
    }

    /// The species of the mass outbreak going on now, if any.
    fn outbreak_species(&self) -> Option<&str> {
        self.dex.outbreak.as_ref().filter(|o| o.until > crate::dex::now_unix()).map(|o| o.species.as_str())
    }

    /// Today's challenges (made fresh each day), and the starter gift of balls.
    fn ensure_daily(&mut self) {
        let now = calendar::local_now();
        let today = format!("{:04}-{:02}-{:02}", now.date.y, now.date.m, now.date.d);
        if self.dex.daily.day != today {
            let list = crate::quest::challenges_for(&today, self.dex.egg.is_some(), self.cfg.timed_catch);
            self.dex.daily = crate::quest::Daily { day: today, list, bonus: false };
            self.dex.save();
        }
        if !self.dex.bag.gift {
            self.dex.bag.gift = true;
            self.dex.bag.add(1, 3);
            self.dex.bag.add(2, 1);
            self.dex.save();
        }
    }

    /// Count progress on today's challenges that `hit` matches; finishing one gives its
    /// balls, and finishing all three gives XP (and every fifth time, a Master Ball).
    fn progress(&mut self, hit: impl Fn(&crate::quest::Challenge) -> bool) {
        self.ensure_daily();
        let mut finished = Vec::new();
        let mut moved = false;
        for c in self.dex.daily.list.iter_mut() {
            if !c.done() && hit(c) {
                c.got += 1;
                moved = true;
                if c.done() {
                    finished.push(c.clone());
                }
            }
        }
        if !moved {
            return;
        }
        let tint = self.theme.good;
        for c in &finished {
            self.dex.bag.add(c.ball, c.balls);
            util::log(&format!("challenge done: {} ({})", c.text(), c.reward()));
            self.quick_card("challenge", G_CHECK_CIRCLE, tint, "Challenge done!", &c.text(), &format!("Reward: {} · in the Pokédex tab's Bag", c.reward()), 5.0);
        }
        let all = !self.dex.daily.list.is_empty() && self.dex.daily.list.iter().all(|c| c.done());
        if all && !self.dex.daily.bonus {
            self.dex.daily.bonus = true;
            self.dex.full_days += 1;
            let master = self.dex.full_days % crate::quest::MASTER_EVERY == 0;
            if master {
                self.dex.bag.add(3, 1);
            }
            let body = if master {
                format!("+{} XP and a Master Ball!", crate::quest::DAILY_BONUS_XP)
            } else {
                let left = crate::quest::MASTER_EVERY - self.dex.full_days % crate::quest::MASTER_EVERY;
                format!("+{} XP · {left} more day{} like this for a Master Ball", crate::quest::DAILY_BONUS_XP, if left == 1 { "" } else { "s" })
            };
            self.dex.save();
            self.quick_card("challenges", G_DEX, tint, "Pokédex", "All of today's challenges done!", &body, 6.0);
            self.gain_xp(crate::quest::DAILY_BONUS_XP);
        }
        self.dex.save();
    }

    /// Clicked a visitor that wants to battle: your Pokémon takes it on.
    fn start_battle(&mut self) {
        if self.hp_now().is_some_and(|h| h <= FAINT) {
            let who = pokemon::pretty(&self.mascot_sprite(PipMood::Calm));
            let tint = self.theme.bad;
            self.quick_card("battle", G_SWORDS, tint, "Battle", &format!("{who} has fainted and can't battle"), "Claude's 5-hour limit is used up: wait for it to reset", 4.0);
            return;
        }
        let now = crate::dex::now_unix();
        let rest = self.dex.raid.as_ref().map_or(0, |r| r.rest_until - now);
        if self.wild.as_ref().is_some_and(|w| w.raid) && rest > 0 {
            let foe = self.wild.as_ref().map(|w| pokemon::pretty(w.name)).unwrap_or_default();
            let tint = self.theme.bad;
            let mins = (rest + 59) / 60;
            self.quick_card("battle", G_SHIELD, tint, "Raid", &format!("{foe} is too strong right now"), &format!("Your team needs a rest after that battle: try again in {mins} min"), 4.0);
            return;
        }
        let me = crate::quest::types_of(&self.mascot_sprite(PipMood::Calm));
        let level = self.dex.level();
        let Some(w) = self.wild.as_mut() else { return };
        let them = crate::quest::types_of(w.name);
        let p = if w.raid { crate::quest::raid_win_chance(&me, &them, level) } else { crate::quest::win_chance(&me, &them, level) };
        let win = rand01() < p;
        util::log(&format!("battle with {}: {} ({:.0}% chance)", w.name, if win { "won" } else { "lost" }, p * 100.0));
        w.fight = Some((Instant::now(), win));
        self.dirty = true;
    }

    /// The battle's over: a win leaves it tired and easier to catch, a loss and it runs.
    fn finish_battle(&mut self) {
        let me_sprite = self.mascot_sprite(PipMood::Calm);
        let me = crate::quest::types_of(&me_sprite);
        let Some(w) = self.wild.as_mut() else { return };
        let Some((_, win)) = w.fight.take() else { return };
        let (name, sprite) = (w.name, w.sprite());
        let them = crate::quest::types_of(name);
        let (mine, my_x) = crate::quest::best_attack(&me, &them);
        let (theirs, their_x) = crate::quest::best_attack(&them, &me);
        let (who, foe) = (pokemon::pretty(&me_sprite), pokemon::pretty(name));
        if w.raid {
            self.finish_raid_battle(win, (mine, my_x), (theirs, their_x), &who, &foe, &me_sprite, &sprite);
            return;
        }
        if win {
            w.battle = false;
            w.tired = true;
            if let Some(v) = self.dex.visitor.as_mut() {
                v.battle = false;
                v.tired = true;
            }
            self.dex.battles_won += 1;
            let ball = rand01() < 1.0 / 3.0;
            if ball {
                self.dex.bag.add(1, 1);
            }
            self.dex.save();
            let xp = 40 + if my_x >= 2.0 { 10 } else { 0 };
            let found = if ball { " · found a Great Ball" } else { "" };
            let body = format!("{} on {}: {} · +{xp} XP{found} · {foe} is tired: easier to catch", pokemon::TYPES[mine].0, type_names(&them), crate::quest::effect_words(my_x));
            let ttl = self.card_ttl(6.0);
            self.push_card(Card {
                key: Some("battle".into()),
                glyph: G_SWORDS,
                tint: self.theme.good,
                app: "Battle".into(),
                title: format!("{who} won the battle!"),
                body,
                icon: pokemon::first_frame(&me_sprite),
                sticky: false,
                ttl,
                focus: Focus::No,
                kind: CardKind::Plain,
            });
            self.gain_xp(xp);
            self.progress(|c| c.kind == "battle");
        } else {
            self.wild = None;
            self.wild_rect = None;
            self.dex.visitor = None;
            self.dex.battles_lost += 1;
            self.dex.save();
            let body = format!("Its {} on your {}: {} · +10 XP for trying", pokemon::TYPES[theirs].0, type_names(&me), crate::quest::effect_words(their_x));
            let ttl = self.card_ttl(5.0);
            self.push_card(Card {
                key: Some("battle".into()),
                glyph: G_SWORDS,
                tint: self.theme.bad,
                app: "Battle".into(),
                title: format!("{foe} won and ran off"),
                body,
                icon: pokemon::first_frame(&sprite),
                sticky: false,
                ttl,
                focus: Focus::No,
                kind: CardKind::Plain,
            });
            self.gain_xp(10);
        }
        self.dirty = true;
    }

    /// A battle with the raid boss: a win breaks a shield, a loss means a rest before the next.
    #[allow(clippy::too_many_arguments)]
    fn finish_raid_battle(&mut self, win: bool, (mine, my_x): (usize, f32), (theirs, their_x): (usize, f32), who: &str, foe: &str, me_sprite: &str, boss_sprite: &str) {
        let now = crate::dex::now_unix();
        let (title, body, tint, icon, xp) = if win {
            self.dex.battles_won += 1;
            let left = match self.dex.raid.as_mut() {
                Some(r) => {
                    r.shields = r.shields.saturating_sub(1);
                    r.shields
                }
                None => 0,
            };
            if left == 0 {
                if let Some(w) = self.wild.as_mut() {
                    w.battle = false;
                    w.tired = true;
                }
                if let Some(v) = self.dex.visitor.as_mut() {
                    v.battle = false;
                    v.tired = true;
                }
            }
            let xp = 60 + if my_x >= 2.0 { 10 } else { 0 };
            let (title, body) = if left == 0 {
                (format!("{foe}'s last shield broke!"), format!("Catch it now: {} balls before it gets away · +{xp} XP", crate::quest::RAID_THROWS))
            } else {
                (
                    format!("{who} broke a shield!"),
                    format!("{} attack: {} · {left} shield{} left · +{xp} XP", pokemon::TYPES[mine].0, crate::quest::effect_words(my_x), if left == 1 { "" } else { "s" }),
                )
            };
            (title, body, self.theme.good, me_sprite, xp)
        } else {
            self.dex.battles_lost += 1;
            if let Some(r) = self.dex.raid.as_mut() {
                r.rest_until = now + crate::quest::RAID_REST_SECS;
            }
            let body = format!(
                "Its {} attack was {} · rest {} min, then battle again · +10 XP",
                pokemon::TYPES[theirs].0,
                crate::quest::effect_words(their_x),
                crate::quest::RAID_REST_SECS / 60
            );
            (format!("{foe} held its ground"), body, self.theme.bad, boss_sprite, 10)
        };
        self.dex.save();
        let ttl = self.card_ttl(6.0);
        self.push_card(Card {
            key: Some("battle".into()),
            glyph: G_SHIELD,
            tint,
            app: "Raid".into(),
            title,
            body,
            icon: pokemon::first_frame(icon),
            sticky: false,
            ttl,
            focus: Focus::No,
            kind: CardKind::Plain,
        });
        self.gain_xp(xp);
        if win {
            self.progress(|c| c.kind == "battle");
        }
        self.dirty = true;
    }

    /// At start-up: the visitor that was waiting when Pocket Isle closed walks back in.
    fn restore_visitor(&mut self) {
        let Some(v) = self.dex.visitor.clone() else { return };
        let sprite = if v.shiny { format!("{}-shiny", v.name) } else { v.name.clone() };
        match pokemon::species(&v.name) {
            Some(s) if pokemon::has_sprite(&sprite) => {
                util::log(&format!("wild Pokémon still waiting: {}{}", v.name, if v.shiny { " (shiny)" } else { "" }));
                self.wild = Some(Wild {
                    name: s.name,
                    shiny: v.shiny,
                    legend: v.legend,
                    since: Instant::now(),
                    from_left: rand01() < 0.5,
                    catch: None,
                    u_at_catch: 0.0,
                    tries: 0,
                    quality: 0.0,
                    flee: false,
                    ball: 0,
                    battle: v.battle,
                    tired: v.tired,
                    fight: None,
                    raid: v.raid,
                    boosted: v.boosted,
                    fished: v.fished,
                });
                self.dirty = true;
            }
            _ => {
                self.dex.visitor = None;
                self.dex.save();
            }
        }
    }

    fn throw_ball(&mut self) {
        let timed = self.cfg.timed_catch;
        if self.wild.is_none() {
            return;
        }
        let ball = self.dex.bag.take();
        if ball != 0 {
            self.dex.save();
        }
        let Some(w) = self.wild.as_mut() else { return };
        // Timed: the smaller the ring when you click, the better the throw, and three Poké
        // Balls before it runs. Untimed: common ones always stay in, legendaries one time in three.
        let q = if timed { 1.0 - ring_k(w.since) } else { 0.0 };
        let chance = match (timed, w.legend) {
            (true, true) => 0.05 + 0.30 * q.powf(1.5),
            (true, false) => 0.45 + 0.5 * q,
            (false, true) => 0.34,
            (false, false) => 1.0,
        };
        // A raid boss with its shields down is still hard to hold; the weather helps.
        let chance = if w.raid { if timed { 0.05 + 0.25 * q } else { 0.2 } } else { chance };
        let chance = if w.boosted { (chance + 0.1).min(1.0) } else { chance };
        let chance = crate::quest::catch_chance(chance, ball, w.tired);
        // The demo's throws always land.
        let ok = util::demo() || rand01() < chance;
        util::log(&format!("threw a {} at {}: {} ({:.0}% chance)", crate::quest::BALL_NAMES[ball as usize], w.name, if ok { "caught" } else { "broke free" }, chance * 100.0));
        w.ball = ball;
        w.tries += 1;
        w.quality = q;
        // Three Poké Balls (five at a raid boss), then it runs off.
        w.flee = !ok && w.tries >= if w.raid { crate::quest::RAID_THROWS } else { 3 };
        w.u_at_catch = wild_u(w.since);
        w.catch = Some((Instant::now(), ok));
    }

    /// The visitor has gone: walked off, been caught, or broken free.
    fn finish_catch(&mut self) {
        let Some(mut w) = self.wild.take() else { return };
        self.wild_rect = None;
        self.dirty = true;
        if !matches!(w.catch, Some((_, false))) || w.flee {
            self.dex.visitor = None;
            self.dex.save();
        }
        let Some((_, ok)) = w.catch else { return };
        if !ok && !w.flee {
            // Out of the ball, it carries on along its walk: throw again.
            w.catch = None;
            w.since = Instant::now().checked_sub(Duration::from_secs_f32(w.u_at_catch * WILD_WALK)).unwrap_or_else(Instant::now);
            self.wild = Some(w);
            return;
        }
        let pretty = pokemon::pretty(w.name);
        if ok {
            if self.dex.chain_species == w.name {
                self.dex.chain += 1;
            } else {
                self.dex.chain_species = w.name.to_string();
                self.dex.chain = 1;
            }
            if self.dex.chain > self.dex.best_chain {
                self.dex.best_chain = self.dex.chain;
                self.dex.best_chain_species = w.name.to_string();
            }
            // Every 5 in a row: a Great Ball; every 10: an Ultra Ball.
            let reward = if self.dex.chain % 10 == 0 {
                self.dex.bag.add(2, 1);
                " · Combo reward: an Ultra Ball"
            } else if self.dex.chain % 5 == 0 {
                self.dex.bag.add(1, 1);
                " · Combo reward: a Great Ball"
            } else {
                ""
            };
            if let Some(o) = self.dex.outbreak.as_mut().filter(|o| o.species == w.name) {
                o.caught += 1;
            }
            if let Some(s) = self.special() {
                *self.dex.special_catches.entry(s.name.to_string()).or_default() += 1;
            }
            let combo = if self.dex.chain >= 2 { format!(" · Combo ×{}! Shiny odds 1 in {}{reward}", self.dex.chain, self.shiny_odds(w.name)) } else { String::new() };
            *self.dex.caught.entry(w.name.to_string()).or_default() += 1;
            {
                let c = self.conditions();
                let mut keys: Vec<&str> = Vec::new();
                if w.fished {
                    keys.push("fished");
                }
                if !c.day {
                    keys.push("night");
                }
                if w.boosted {
                    keys.push("boosted");
                }
                if !c.day && pokemon::has_type(w.name, "Water") {
                    keys.push("night_water");
                }
                if c.day && c.sky == Some(pokemon::Kind::Clear) && pokemon::has_type(w.name, "Fire") {
                    keys.push("sun_fire");
                }
                for (t, k) in [("Psychic", "psychic"), ("Ghost", "ghost"), ("Dragon", "dragon")] {
                    if pokemon::has_type(w.name, t) {
                        keys.push(k);
                    }
                }
                for k in keys {
                    *self.dex.counts.entry(k.into()).or_default() += 1;
                }
            }
            // An unlocked quest's legendary, caught: that quest is done.
            let quest_done = crate::quest::QUESTS.iter().find(|q| q.legend == w.name && self.dex.quests_open.iter().any(|k| k == q.key)).map(|q| q.key);
            if let Some(k) = quest_done {
                self.dex.quests_open.retain(|o| o != k);
                self.dex.quests_done.push(k.to_string());
            }
            self.dex.remember(if w.shiny { format!("{}-shiny", w.name) } else { w.name.to_string() });
            if w.shiny {
                *self.dex.shiny.entry(w.name.to_string()).or_default() += 1;
            }
            self.dex.save();
            self.dex_thumbs.clear();
            self.pending_evolution = true;
            if self.cfg.pokemon_cries && self.mic_apps.is_empty() {
                crate::sound::cry(w.name, 0.5);
            }
            let bonus = if w.quality >= 0.8 { 10 } else if w.quality >= 0.55 { 5 } else { 0 };
            let xp = if w.shiny { 150 } else if w.legend { 100 } else { 30 } + bonus;
            let boost = if w.boosted { xp / 2 } else { 0 };
            let xp = xp + boost + if w.raid { crate::quest::RAID_XP } else { 0 };
            if w.raid {
                if let Some(r) = self.dex.raid.as_mut() {
                    r.caught = true;
                    r.done = true;
                }
                self.dex.raids_won += 1;
                self.dex.save();
            }
            let combo = if boost > 0 { format!(" · Weather boost{combo}") } else { combo };
            let combo = if w.raid { format!(" · Raid won!{combo}") } else { combo };
            let combo = if w.fished { format!(" · Fished up{combo}") } else { combo };
            let combo = if quest_done.is_some() { format!(" · Legend quest done!{combo}") } else { combo };
            let how = throw_note(w.quality);
            let how = if how.is_empty() { String::new() } else { format!("{how} ") };
            let species = pokemon::dex_list().filter(|n| self.dex.caught.contains_key(*n)).count();
            let ttl = self.card_ttl(5.0);
            self.push_card(Card {
                key: Some("catch".into()),
                glyph: G_DEX,
                tint: self.theme.good,
                app: "Pokédex".into(),
                title: if w.shiny { format!("Gotcha! Shiny {pretty} was caught!") } else { format!("Gotcha! {pretty} was caught!") },
                body: format!("{how}+{xp} XP · {species} of {} in your Pokédex{combo}", pokemon::DEX_SIZE),
                icon: pokemon::first_frame(&w.sprite()),
                sticky: false,
                ttl,
                focus: Focus::No,
                kind: CardKind::Plain,
            });
            self.gain_xp(xp);
            let (name, first, great) = (w.name, w.tries == 1, w.quality >= 0.55);
            self.progress(|c| match c.kind.as_str() {
                "catch" => true,
                "type" => pokemon::has_type(name, &c.t),
                "first" => first,
                "throw" => great,
                _ => false,
            });
        } else {
            let ttl = self.card_ttl(4.0);
            self.push_card(Card {
                key: Some("catch".into()),
                glyph: G_DEX,
                tint: self.theme.bad,
                app: "Pokédex".into(),
                title: format!("Oh no! {pretty} fled"),
                body: if w.raid {
                    "It broke out of five balls and got away. A new raid comes next weekend".into()
                } else if w.tries >= 3 {
                    "It broke out of three Poké Balls. Maybe next time…".into()
                } else {
                    "It broke free and ran. Maybe next time…".into()
                },
                icon: pokemon::first_frame(&w.sprite()),
                sticky: false,
                ttl,
                focus: Focus::No,
                kind: CardKind::Plain,
            });
        }
    }

    /// A Poké Ball in chunky pixels, centred on (cx, cy).
    fn draw_ball(&mut self, cx: f32, cy: f32, a: f32) {
        self.draw_ball_kind(cx, cy, a, 0);
    }

    /// A Poké (0), Great (1), Ultra (2) or Master Ball (3).
    fn draw_ball_kind(&mut self, cx: f32, cy: f32, a: f32, kind: u8) {
        const BALL: [&str; 8] = ["..XXXX..", ".XRRRRX.", "XRRRRRRX", "XXXwwXXX", "XWWwwWWX", "XWWWWWWX", ".XWWWWX.", "..XXXX.."];
        let (top, mark) = match kind {
            1 => (Rgba::rgb(60, 124, 236), Some(Rgba::rgb(232, 64, 64))),
            2 => (Rgba::rgb(50, 50, 58), Some(Rgba::rgb(248, 206, 60))),
            3 => (Rgba::rgb(146, 70, 200), Some(Rgba::rgb(240, 118, 190))),
            _ => (Rgba::rgb(232, 64, 64), None),
        };
        let s = self.scale;
        let px = (1.25 * s).round().max(1.0) / s;
        let x0 = ((cx - 4.0 * px) * s).round() / s;
        let y0 = ((cy - 4.0 * px) * s).round() / s;
        for (j, row) in BALL.iter().enumerate() {
            for (i, c) in row.chars().enumerate() {
                let col = match c {
                    'X' => Rgba::rgb(30, 30, 36),
                    'R' if (j == 1 || j == 2) && (i == 2 || i == 5) => mark.unwrap_or(top),
                    'R' => top,
                    'W' => Rgba::rgb(240, 240, 244),
                    'w' => Rgba::rgb(255, 255, 255),
                    _ => continue,
                };
                self.r.rect(R::xywh(x0 + i as f32 * px, y0 + j as f32 * px, px, px), col.a(a));
            }
        }
    }

    /// A four-point twinkle.
    fn twinkle(&mut self, x: f32, y: f32, size: f32, c: Rgba) {
        self.r.rect(R::new(x - size, y - 0.6, x + size, y + 0.6), c);
        self.r.rect(R::new(x - 0.6, y - size, x + 0.6, y + size), c);
    }

    /// A wild Pokémon walking across the pill, or being caught.
    fn draw_wild(&mut self, p: R, t: f32) {
        let Some(w) = self.wild.as_ref() else {
            self.wild_rect = None;
            return;
        };
        let (name, shiny, legend, since, from_left, catch, u0) = (w.sprite(), w.shiny, w.legend, w.since, w.from_left, w.catch, w.u_at_catch);
        let (tries, quality, flee, ball, battle, fight) = (w.tries, w.quality, w.flee, w.ball, w.battle, w.fight);
        let (raid, boosted) = (w.raid, w.boosted);
        let Some((dw, dh)) = self.sprite_box(&name, self.pill_h - 5.0) else {
            // Not downloaded: it can't visit.
            self.wild = None;
            self.wild_rect = None;
            self.dex.visitor = None;
            self.dex.save();
            return;
        };
        let s = self.scale;
        let snap = |v: f32| (v * s).round() / s;
        let u = match catch {
            Some(_) => u0,
            None => wild_u(since),
        };
        let (prog, walking) = wild_path(u);
        // Waiting, it hops now and then so you notice it.
        let hop = if !walking && catch.is_none() {
            let ph = since.elapsed().as_secs_f32() % 5.0;
            if ph < 0.45 { (ph / 0.45 * std::f32::consts::PI).sin() * 3.0 } else { 0.0 }
        } else {
            0.0
        };
        let (xa, xb) = (p.x0 - dw - 4.0, p.x1 + 4.0);
        let x = if from_left { xa + (xb - xa) * prog } else { xb - (xb - xa) * prog };
        let floor = snap(p.cy() + self.pill_h / 2.0 - 2.0);
        let bob = if walking && catch.is_none() { (t * 9.0).sin().abs() * 1.5 } else { hop };
        let dest = R::xywh(snap(x), snap(floor - dh - bob), snap(dw), snap(dh));
        // Kept while a ball is out too, so the pointer resting there doesn't open the island.
        self.wild_rect = Some(dest);
        let th = self.theme.clone();
        // Sprites face left; walking right, it turns round. Standing, it looks at the pointer.
        let flip = if walking { from_left } else { self.hover && self.mouse.0 > dest.cx() };
        let frame = |me: &Self| -> Option<(ID2D1Bitmap1, R)> {
            let Some(Some(sp)) = me.strips.get(&name) else { return None };
            let i = pokemon::frame_at(&sp.delays, (t * 1000.0) as u64);
            Some((sp.bmp.clone(), R::xywh(i as f32 * sp.fw, 0.0, sp.fw, sp.fh)))
        };
        if let (None, Some((at, win))) = (catch, fight) {
            // A battle: three exchanges of attacks, then the loser flashes.
            let c = at.elapsed().as_secs_f32();
            let me = crate::quest::types_of(&self.mascot_sprite(PipMood::Calm));
            let them = crate::quest::types_of(&name);
            let (mine, my_x) = crate::quest::best_attack(&me, &them);
            let (theirs, their_x) = crate::quest::best_attack(&them, &me);
            // A super effective hit lands big, a weak one small, one with no effect fizzles.
            let hit = |x: f32| if x <= 0.0 { 0.25 } else { (0.6 + 0.4 * x).min(1.6) };
            let tc = |i: usize| {
                let (r, g, b) = pokemon::TYPES[i % 18].1;
                Rgba::rgb(r, g, b)
            };
            let from = self.mon_rect.map(|r| (r.cx(), r.cy())).unwrap_or((p.cx(), p.cy()));
            let phase = (c / 0.7).floor() as i32;
            let k = (c / 0.7).fract();
            let mut shake = 0.0;
            if phase < 3 {
                let (a, b, col, x) = if k < 0.5 { (from, (dest.cx(), dest.cy()), tc(mine), my_x) } else { ((dest.cx(), dest.cy()), from, tc(theirs), their_x) };
                let u = (k % 0.5) / 0.5;
                if u < 0.7 {
                    let v = u / 0.7;
                    let (bx, by) = (a.0 + (b.0 - a.0) * v, a.1 + (b.1 - a.1) * v - (v * std::f32::consts::PI).sin() * 6.0);
                    self.r.glow(bx, by, 9.0, col.a(0.55));
                    self.r.circle(bx, by, 2.6, col);
                } else {
                    let v = (u - 0.7) / 0.3;
                    self.r.glow(b.0, b.1, 16.0 * hit(x), col.a(0.5 * (1.0 - v)));
                    if k < 0.5 {
                        shake = (v * 30.0).sin() * 2.0 * hit(x);
                    }
                }
            }
            let lost_flash = phase >= 3 && win;
            let alpha = if lost_flash { 0.55 + 0.45 * (c * 18.0).cos().abs() } else { 1.0 };
            if let Some((bmp, src)) = frame(self) {
                self.r.sprite_flip(&bmp, dest.offset(shake, 0.0), src, true, flip, alpha);
            }
            if phase >= 3 {
                let r = R::new(dest.cx() - 60.0, (dest.y0 - 14.0).max(p.y0 - 6.0), dest.cx() + 60.0, dest.y0 + 2.0);
                let (msg, col) = if win { ("You won!", Rgba::rgb(255, 225, 110)) } else { ("It won…", Rgba::rgb(255, 120, 100)) };
                self.r.text(msg, r, 10.5, 800, col, Align::Center);
            } else {
                // Over whoever is being hit, like the games: "Super effective!" and friends
                // (crossed swords over the visitor for an ordinary hit).
                let (x, over) = if k < 0.5 { (my_x, Some(dest)) } else { (their_x, self.mon_rect) };
                let words = if x >= 2.0 {
                    Some(("Super effective!", Rgba::rgb(255, 225, 110)))
                } else if x <= 0.0 {
                    Some(("No effect…", Rgba::rgb(190, 190, 200)))
                } else if x < 1.0 {
                    Some(("Not very effective…", Rgba::rgb(190, 190, 200)))
                } else {
                    None
                };
                match (words, over) {
                    (Some((msg, col)), Some(o)) => {
                        let r = R::new(o.cx() - 70.0, (o.y0 - 14.0).max(p.y0 - 6.0), o.cx() + 70.0, o.y0 + 2.0);
                        self.r.text(msg, r, 9.5, 800, col, Align::Center);
                    }
                    _ if k < 0.5 => {
                        let r = R::new(dest.cx() - 8.0, (dest.y0 - 15.0).max(p.y0 - 6.0), dest.cx() + 8.0, dest.y0 + 1.0);
                        self.r.icon(G_SWORDS, r, 11.0, Rgba::rgb(255, 140, 100));
                    }
                    _ => {}
                }
            }
            return;
        }
        match catch {
            None => {
                if legend {
                    self.r.glow(dest.cx(), dest.cy(), dw, th.accent.a(0.22 + 0.08 * (t * 3.0).sin()));
                }
                if let Some((bmp, src)) = frame(self) {
                    self.r.sprite_flip(&bmp, dest, src, true, flip, 1.0);
                }
                // A raid boss: its shields down the side (lit while still up).
                let up = if raid { self.dex.raid.as_ref().map_or(0, |r| r.shields) } else { 0 };
                if up > 0 {
                    for i in 0..crate::quest::RAID_SHIELDS {
                        let r = R::xywh(dest.x1, dest.y0 + 1.0 + i as f32 * 9.0, 10.0, 9.0);
                        let col = if i < up { Rgba::rgb(120, 190, 255) } else { th.subtext.a(0.35) };
                        self.r.icon(G_SHIELD, r, 8.5, col);
                    }
                }
                // Weather boost: the weather's icon by its head.
                if boosted {
                    if let Some(g) = self.weather.as_ref().map(|w| weather::describe(w.code, w.day).0) {
                        self.r.icon(g, R::xywh(dest.x0 - 11.0, dest.y0 - 2.0, 12.0, 12.0), 10.0, Rgba::rgb(140, 200, 255));
                    }
                }
                // Wants to battle: crossed swords now and then (click it to fight).
                if battle && since.elapsed() >= Duration::from_millis(1500) && since.elapsed().as_secs_f32() % 3.0 < 1.6 {
                    let r = R::new(dest.cx() - 8.0, (dest.y0 - 15.0).max(p.y0 - 6.0), dest.cx() + 8.0, dest.y0 + 1.0);
                    self.r.icon(G_SWORDS, r, 11.0, Rgba::rgb(255, 140, 100));
                }
                if shiny {
                    for k in 0..3 {
                        let ph = (t * 2.2 + k as f32 * 2.1).sin();
                        if ph > 0.2 {
                            let (ox, oy) = [(0.15, 0.2), (0.85, 0.35), (0.5, 0.05)][k];
                            self.twinkle(dest.x0 + dest.w() * ox, dest.y0 + dest.h() * oy, 2.5 * ph, Rgba::rgb(255, 230, 120).a(ph));
                        }
                    }
                }
                // Timed catching: rest the pointer on it and a ring shrinks; click when it's small.
                let aiming = self.hover && dest.inset(-6.0).contains(self.mouse.0, self.mouse.1) && !battle;
                if aiming && self.dex.bag.pick != 0 {
                    // The ball you'll throw.
                    let pick = self.dex.bag.pick;
                    self.draw_ball_kind(dest.x1 + 3.0, dest.y1 - 6.0, 0.95, pick);
                }
                if aiming && self.cfg.timed_catch {
                    let hi = (dh * 0.5 + 3.0).min(self.pill_h * 0.55);
                    let lo = hi * 0.35;
                    let col = if legend { Rgba::rgb(240, 80, 64) } else if shiny { Rgba::rgb(248, 200, 56) } else { Rgba::rgb(88, 208, 112) };
                    self.r.ring(dest.cx(), dest.cy(), hi, th.text.a(0.22), 1.0);
                    self.r.ring(dest.cx(), dest.cy(), lo + (hi - lo) * ring_k(since), col.a(0.95), 1.6);
                }
                // "!" as it arrives.
                if since.elapsed() < Duration::from_millis(1500) {
                    let r = R::new(dest.cx() - 8.0, (dest.y0 - 14.0).max(p.y0 - 6.0), dest.cx() + 8.0, dest.y0 + 2.0);
                    self.r.text("!", r, 13.0, 800, th.text, Align::Center);
                }
            }
            Some((at, ok)) => {
                let c = at.elapsed().as_secs_f32();
                let rest = (dest.cx(), floor - 6.0);
                let from = self.mon_rect.map(|r| (r.cx(), r.y0 + 4.0)).unwrap_or((p.cx(), p.y0 + 4.0));
                if c < 0.35 {
                    // The throw, in an arc.
                    if let Some((bmp, src)) = frame(self) {
                        self.r.sprite_flip(&bmp, dest, src, true, flip, 1.0);
                    }
                    let k = c / 0.35;
                    let bx = from.0 + (dest.cx() - from.0) * k;
                    let by = from.1 + (dest.cy() - from.1) * k - (k * std::f32::consts::PI).sin() * 12.0;
                    self.draw_ball_kind(bx, by, 1.0, ball);
                } else if c < 0.65 {
                    // Drawn in with a red flash.
                    let k = (c - 0.35) / 0.3;
                    if let Some((bmp, src)) = frame(self) {
                        self.r.sprite_flip(&bmp, dest, src, true, flip, 1.0 - k);
                    }
                    self.r.glow(dest.cx(), dest.cy(), dw * 0.8, Rgba::rgb(255, 80, 80).a(0.5 * (1.0 - k)));
                    self.draw_ball_kind(dest.cx() + (rest.0 - dest.cx()) * k, dest.cy() + (rest.1 - dest.cy()) * k, 1.0, ball);
                } else if c < 2.1 {
                    // Three wobbles.
                    let k = (c - 0.65) / 1.45;
                    let wob = (k * 3.0 * std::f32::consts::TAU).sin() * 1.8 * (1.0 - 0.4 * k);
                    self.draw_ball_kind(rest.0 + wob, rest.1, 1.0, ball);
                } else if ok {
                    // Click! Stars.
                    let k = ((c - 2.1) / 0.6).min(1.0);
                    self.draw_ball_kind(rest.0, rest.1, 1.0 - k * 0.3, ball);
                    for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (0.0, -1.4)] {
                        self.twinkle(rest.0 + dx * 10.0 * k, rest.1 + dy * 8.0 * k, 3.0 * (1.0 - k), Rgba::rgb(255, 230, 120).a(1.0 - k));
                    }
                } else if !flee {
                    // It breaks out but stays: throw again.
                    let k = ((c - 2.1) / 0.6).min(1.0);
                    self.r.glow(rest.0, rest.1, 18.0, Rgba::rgb(255, 255, 255).a(0.6 * (1.0 - k)));
                    if let Some((bmp, src)) = frame(self) {
                        self.r.sprite_flip(&bmp, dest, src, true, flip, k);
                    }
                    let left = (if raid { crate::quest::RAID_THROWS } else { 3 }).saturating_sub(tries);
                    let r = R::new(dest.cx() - 70.0, (dest.y0 - 14.0).max(p.y0 - 6.0), dest.cx() + 70.0, dest.y0 + 2.0);
                    let balls = if left == 1 { "1 ball left".to_string() } else { format!("{left} balls left") };
                    self.r.text(&format!("Broke free! {balls}"), r, 10.0, 700, th.text, Align::Center);
                } else {
                    // It breaks out and runs.
                    let k = ((c - 2.1) / 0.9).min(1.0);
                    let away = if from_left { 1.0 } else { -1.0 } * 140.0 * k;
                    self.r.glow(rest.0, rest.1, 18.0, Rgba::rgb(255, 255, 255).a(0.6 * (1.0 - k)));
                    if let Some((bmp, src)) = frame(self) {
                        self.r.sprite_flip(&bmp, dest.offset(away, 0.0), src, true, from_left, 1.0);
                    }
                }
                // How good the throw was.
                let note = throw_note(quality);
                if !note.is_empty() && c < 1.6 {
                    let r = R::new(dest.cx() - 50.0, (dest.y0 - 14.0).max(p.y0 - 6.0), dest.cx() + 50.0, dest.y0 + 2.0);
                    self.r.text(note, r, 10.5, 800, Rgba::rgb(255, 225, 110).a((1.0 - c / 1.6).min(1.0)), Align::Center);
                }
            }
        }
    }

    /// What's left of Claude's 5-hour limit as the team's HP (0..1), and when it resets.
    fn claude_hp(&self) -> Option<(f32, Option<i64>)> {
        let u = self.usage.iter().find(|u| u.provider == "Claude")?;
        let l = u.limits.iter().find(|l| l.short == "5h")?;
        Some(((1.0 - l.used).clamp(0.0, 1.0), l.resets_at))
    }

    /// The HP to show: full again as soon as the limit's reset time has passed.
    fn hp_now(&self) -> Option<f32> {
        if !self.cfg.claude_hp || !self.cfg.mascot {
            return None;
        }
        let (hp, resets) = self.claude_hp()?;
        Some(if resets.is_some_and(|r| r <= usage::now_unix()) { 1.0 } else { hp })
    }

    /// A card when the Pokémon faints (the limit ran out) or wakes up (it reset).
    fn hp_changed(&mut self, before: Option<f32>) {
        if !self.cfg.claude_hp || !self.cfg.mascot {
            return;
        }
        let (Some(b), Some((hp, resets))) = (before, self.claude_hp()) else { return };
        let who = pokemon::pretty(&self.mascot_sprite(PipMood::Calm));
        if b > FAINT && hp <= FAINT {
            let body = match resets {
                Some(r) => format!("Claude's 5-hour limit is used up · back in {}", usage::countdown(r - usage::now_unix())),
                None => "Claude's 5-hour limit is used up".into(),
            };
            let tint = self.theme.bad;
            self.quick_card("hp", G_DEX, tint, "Out of HP", &format!("{who} fainted!"), &body, 6.0);
        } else if b <= FAINT && hp > FAINT {
            let tint = self.theme.good;
            self.quick_card("hp", G_DEX, tint, "HP restored", &format!("{who} is full of energy again!"), "Claude's 5-hour limit has reset", 5.0);
        }
    }

    /// Caught Pokémon evolve as the team levels up: each evolved form joins the Pokédex
    /// (the one you caught stays in it too). With two ways to evolve (Eevee, Gloom, Kirlia…),
    /// each one you caught takes the next way, so catching more fills in the rest. A shiny
    /// evolves into a shiny. Returns (from, to) for each new one.
    fn evolve_caught(&mut self) -> Vec<(String, String)> {
        let lvl = self.dex.level();
        let mut done: Vec<(String, String)> = Vec::new();
        loop {
            let caught: Vec<(String, u32)> = self.dex.caught.iter().map(|(k, v)| (k.clone(), *v)).collect();
            let mut changed = false;
            for (name, n) in caught {
                for (to, at, first) in pokemon::grows_into(&name, n) {
                    if at > lvl || self.dex.caught.contains_key(to) || pokemon::is_friend_evo(&name, to) {
                        continue;
                    }
                    self.dex.caught.insert(to.to_string(), 1);
                    *self.dex.seen.entry(to.to_string()).or_default() += 1;
                    let shiny = first && self.dex.shiny.get(&name).copied().unwrap_or(0) > 0;
                    if shiny {
                        self.dex.shiny.insert(to.to_string(), 1);
                    }
                    let shiny_sprite = format!("{to}-shiny");
                    self.dex.remember(if shiny && pokemon::has_sprite(&shiny_sprite) { shiny_sprite } else { to.to_string() });
                    done.push((name.clone(), to.to_string()));
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        if !done.is_empty() {
            self.dex.save();
            self.dex_thumbs.clear();
            self.dirty = true;
            util::log(&format!("caught Pokémon evolved: {done:?}"));
        }
        done
    }

    /// The soonest a caught Pokémon will evolve: (from, to, team level).
    fn next_caught_evolution(&self) -> Option<(String, &'static str, u32)> {
        let lvl = self.dex.level();
        self.dex
            .caught
            .iter()
            .flat_map(|(name, &n)| {
                pokemon::grows_into(name, n)
                    .into_iter()
                    .filter(move |(to, at, _)| *at > lvl && !self.dex.caught.contains_key(*to) && !pokemon::is_friend_evo(name, to))
                    .map(move |(to, at, _)| (name.clone(), to, at))
            })
            .min_by_key(|(_, _, at)| *at)
    }

    /// "Your Marill evolved into Azumarill!"
    fn evolution_card(&mut self, evolved: &[(String, String)]) {
        let Some((_, last)) = evolved.last() else { return };
        let (title, body) = match evolved {
            [(a, b)] if b.contains("-mega") || b.ends_with("-primal") => {
                (format!("Your {} can Mega Evolve!", pokemon::pretty(a)), format!("{} added to your Pokédex's Forms", pokemon::pretty(b)))
            }
            [(a, b)] => (format!("Your {} evolved into {}!", pokemon::pretty(a), pokemon::pretty(b)), "Added to your Pokédex".to_string()),
            many => (
                format!("{} of your Pokémon evolved!", many.len()),
                many.iter().map(|(a, b)| format!("{} → {}", pokemon::pretty(a), pokemon::pretty(b))).collect::<Vec<_>>().join(", "),
            ),
        };
        if self.cfg.pokemon_cries && self.mic_apps.is_empty() {
            crate::sound::cry(last, 0.5);
        }
        let ttl = self.card_ttl(6.0);
        let icon = pokemon::first_frame(last);
        self.push_card(Card {
            key: Some("evolved".into()),
            glyph: G_DEX,
            tint: self.theme.accent,
            app: "Pokédex".into(),
            title,
            body,
            icon,
            sticky: false,
            ttl,
            focus: Focus::No,
            kind: CardKind::Plain,
        });
    }

    /// Once a minute: count a day you used the PC (for the trainer card).
    fn count_day(&mut self) {
        let now = calendar::local_now();
        let today = format!("{:04}-{:02}-{:02}", now.date.y, now.date.m, now.date.d);
        if self.dex.last_day == today || user_idle_ms() >= 120_000 {
            return;
        }
        self.dex.last_day = today.clone();
        self.dex.days_played += 1;
        if self.dex.first_day.is_empty() {
            self.dex.first_day = today;
        }
        self.dex.save();
    }

    /// The trainer card (Pokédex tab): your stats, and the special days you've caught on.
    /// The Pokédex tab's Bag: pick the ball to throw, today's challenges, any outbreak and
    /// your battles.
    fn draw_bag(&mut self, x0: f32, x1: f32, mut y: f32) {
        self.ensure_daily();
        let th = self.theme.clone();
        let bag = self.dex.bag.clone();
        let gap = 8.0;
        let cw = (x1 - x0 - 3.0 * gap) / 4.0;
        for b in 0..4u8 {
            let r = R::xywh(x0 + b as f32 * (cw + gap), y, cw, 44.0);
            let count = bag.count(b);
            let have = count.is_none_or(|n| n > 0);
            let on = bag.pick == b;
            let hot = self.hot == Some(Btn::Ball(b));
            self.r.fill_round(r, 12.0, if on { th.accent } else if hot && have { th.hover } else { th.bg2.a(0.6) });
            let a = if have { 1.0 } else { 0.4 };
            self.draw_ball_kind(r.x0 + 18.0, r.cy(), a, b);
            let (fg, sub) = if on { (th.bg, th.bg.a(0.75)) } else { (th.text.a(a), th.subtext.a(a)) };
            self.r.text(crate::quest::BALL_NAMES[b as usize], R::new(r.x0 + 32.0, r.y0 + 5.0, r.x1 - 4.0, r.y0 + 23.0), 11.5, 600, fg, Align::Left);
            let left = match count {
                None => "Never runs out".to_string(),
                Some(n) => format!("×{n}"),
            };
            self.r.text(&left, R::new(r.x0 + 32.0, r.y0 + 22.0, r.x1 - 4.0, r.y1 - 4.0), 10.5, 500, sub, Align::Left);
            self.buttons.push((r, Btn::Ball(b)));
        }
        y += 44.0 + 4.0;
        self.r.text("Click a ball to throw it next · Great ×1.5, Ultra ×2, a Master Ball always catches", R::new(x0 + 4.0, y, x1, y + 18.0), 10.5, 400, th.subtext, Align::Left);
        y += 24.0;

        let daily = self.dex.daily.clone();
        self.r.text("Today's challenges", R::new(x0 + 4.0, y, x1, y + 20.0), 11.5, 600, th.subtext, Align::Left);
        let towards = self.dex.full_days % crate::quest::MASTER_EVERY;
        let right = if daily.bonus { "All done today!".to_string() } else { format!("All three: +{} XP · {towards}/{} to a Master Ball", crate::quest::DAILY_BONUS_XP, crate::quest::MASTER_EVERY) };
        self.r.text(&right, R::new(x0, y, x1 - 4.0, y + 20.0), 10.5, 500, if daily.bonus { th.good } else { th.subtext }, Align::Right);
        y += 22.0;
        for c in &daily.list {
            let r = R::new(x0, y, x1, y + 26.0);
            self.r.fill_round(r, 9.0, th.bg2.a(0.45));
            let done = c.done();
            self.r.icon(if done { G_CHECK_CIRCLE } else { G_CIRCLE }, R::xywh(x0 + 4.0, y, 22.0, 26.0), 13.0, if done { th.good } else { th.subtext });
            self.r.text(&c.text(), R::new(x0 + 30.0, y, x1 - 230.0, y + 26.0), 11.5, 600, if done { th.subtext } else { th.text }, Align::Left);
            let bar = R::new(x1 - 222.0, y + 11.0, x1 - 150.0, y + 15.0);
            self.r.fill_round(bar, 2.0, th.bg2);
            let f = (c.got as f32 / c.need.max(1) as f32).clamp(0.0, 1.0);
            self.r.fill_round(R::new(bar.x0, bar.y0, bar.x0 + (bar.w() * f).max(4.0), bar.y1), 2.0, if done { th.good } else { th.accent });
            self.r.text(&format!("{} / {}", c.got.min(c.need), c.need), R::new(x1 - 146.0, y, x1 - 112.0, y + 26.0), 10.5, 600, th.text, Align::Left);
            self.r.text(&c.reward(), R::new(x1 - 112.0, y, x1 - 8.0, y + 26.0), 10.5, 500, th.subtext, Align::Right);
            y += 28.0;
        }
        y += 4.0;
        let now = crate::dex::now_unix();
        let outbreak = match self.dex.outbreak.clone().filter(|o| o.until > now) {
            Some(o) => format!("Mass outbreak: {} for {} more · shiny 1 in {} · {} caught", pokemon::pretty(&o.species), mins_text(((o.until - now) / 60).max(1) as u32), self.shiny_odds(&o.species), o.caught),
            None => "No outbreak now · now and then one Pokémon turns up everywhere for an hour".to_string(),
        };
        self.r.icon(G_DEX, R::xywh(x0 + 4.0, y, 22.0, 24.0), 12.0, th.accent);
        self.r.text(&outbreak, R::new(x0 + 30.0, y, x1 - 4.0, y + 24.0), 11.0, 500, th.text, Align::Left);
        y += 26.0;
        let raid = self.raid_line();
        self.r.icon(G_SHIELD, R::xywh(x0 + 4.0, y, 22.0, 24.0), 12.0, th.accent);
        self.r.text(&raid, R::new(x0 + 30.0, y, x1 - 4.0, y + 24.0), 11.0, 500, th.text, Align::Left);
        y += 26.0;
        let (won, lost) = (self.dex.battles_won, self.dex.battles_lost);
        let battles = if won + lost == 0 {
            "Battles: some visitors want to battle first (crossed swords). Win and they're easier to catch".to_string()
        } else {
            format!("Battles: {won} won · {lost} lost · type matchups and your level decide who wins")
        };
        self.r.icon(G_SWORDS, R::xywh(x0 + 4.0, y, 22.0, 24.0), 12.0, th.accent);
        self.r.text(&battles, R::new(x0 + 30.0, y, x1 - 4.0, y + 24.0), 11.0, 500, th.text, Align::Left);
        y += 26.0;
        let fished = self.dex.counts.get("fished").copied().unwrap_or(0);
        let fishing = self.fishing.is_some();
        let line = if fishing {
            "Fishing… click the bobber when the \"!\" shows".to_string()
        } else if fished == 0 {
            "Fishing: cast a line beside your Pokémon · some only bite, they never walk by".to_string()
        } else {
            format!("Fishing: {fished} fished up · some Pokémon only bite, they never walk by")
        };
        self.r.icon(G_FISH, R::xywh(x0 + 4.0, y, 22.0, 24.0), 12.0, th.accent);
        self.r.text(&line, R::new(x0 + 30.0, y, x1 - 96.0, y + 24.0), 11.0, 500, th.text, Align::Left);
        let b = R::new(x1 - 88.0, y + 1.0, x1 - 4.0, y + 23.0);
        let hot = self.hot == Some(Btn::Fish);
        self.r.fill_round(b, 9.0, if hot { th.accent } else { th.hover });
        self.r.text(if fishing { "Stop" } else { "Go fishing" }, b, 11.0, 600, if hot { th.bg } else { th.text }, Align::Center);
        self.buttons.push((b, Btn::Fish));
    }

    fn draw_trainer(&mut self, x0: f32, x1: f32, y: f32) {
        let th = self.theme.clone();
        if self.user.is_empty() {
            self.user = controls::user_name();
        }
        let first = self.user.split_whitespace().next().unwrap_or("").to_string();
        let who = if first.is_empty() { "Your trainer card".to_string() } else { format!("Trainer {first}") };
        self.r.text(&who, R::new(x0 + 4.0, y, x1 - 200.0, y + 22.0), 13.5, 700, th.text, Align::Left);
        let since = match self.dex.first_day.split('-').map(|p| p.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>().as_slice() {
            [y, m, d] if (1..=12).contains(m) => {
                const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
                format!("Playing since {d} {} {y}", MONTHS[*m as usize - 1])
            }
            _ => String::new(),
        };
        self.r.text(&since, R::new(x1 - 220.0, y, x1 - 4.0, y + 22.0), 10.5, 400, th.subtext, Align::Right);

        let d = self.dex.clone();
        let caught: u32 = d.caught.values().sum();
        let shinies: u32 = d.shiny.values().sum();
        let legends: u32 = d.caught.iter().filter(|(n, _)| pokemon::is_legend(n)).map(|(_, c)| *c).sum();
        let combo_sub = if d.best_chain >= 2 { format!("Longest combo · {}", pokemon::pretty(&d.best_chain_species)) } else { "Longest combo".to_string() };
        let tiles = [
            (d.days_played.to_string(), "Days played".to_string()),
            (caught.to_string(), "Pokémon caught".to_string()),
            (shinies.to_string(), "Shinies".to_string()),
            (d.hatched.to_string(), "Eggs hatched".to_string()),
            (if d.best_chain >= 2 { format!("×{}", d.best_chain) } else { "–".to_string() }, combo_sub),
            (legends.to_string(), "Legendaries".to_string()),
        ];
        let gap = 8.0;
        let tw = (x1 - x0 - 2.0 * gap) / 3.0;
        let top = y + 28.0;
        for (i, (value, label)) in tiles.iter().enumerate() {
            let r = R::xywh(x0 + (i % 3) as f32 * (tw + gap), top + (i / 3) as f32 * (48.0 + gap), tw, 48.0);
            self.r.fill_round(r, 12.0, th.bg2.a(0.6));
            self.r.text(value, R::new(r.x0 + 12.0, r.y0 + 4.0, r.x1 - 8.0, r.y0 + 28.0), 18.0, 700, th.text, Align::Left);
            self.r.text(label, R::new(r.x0 + 12.0, r.y0 + 27.0, r.x1 - 8.0, r.y1 - 4.0), 10.5, 500, th.subtext, Align::Left);
        }
        // The special days you've caught something on.
        let y2 = top + 2.0 * 48.0 + gap + 10.0;
        self.r.text("Caught on special days", R::new(x0 + 4.0, y2, x1, y2 + 18.0), 11.0, 600, th.subtext, Align::Left);
        let pw = (x1 - x0 - 3.0 * 6.0) / 4.0;
        for (i, s) in pokemon::SPECIAL_DAYS.iter().enumerate() {
            let r = R::xywh(x0 + (i % 4) as f32 * (pw + 6.0), y2 + 22.0 + (i / 4) as f32 * 28.0, pw, 22.0);
            let n = d.special_catches.get(s.name).copied().unwrap_or(0);
            let label = if n > 0 { format!("{} ×{n}", capitalise(s.name)) } else { capitalise(s.name) };
            self.r.fill_round(r, 11.0, if n > 0 { th.accent } else { th.bg2.a(0.4) });
            self.r.text(&label, r, 10.5, 600, if n > 0 { th.bg } else { th.subtext.a(0.6) }, Align::Center);
        }
        // Medals: lit once earned; point at one to read it.
        let rows = pokemon::SPECIAL_DAYS.len().div_ceil(4) as f32;
        let y3 = y2 + 22.0 + rows * 28.0 + 2.0;
        let stats = self.stats();
        let earned = crate::quest::MEDALS.iter().filter(|m| d.medals.iter().any(|k| k == m.key)).count();
        self.r.text(&format!("Medals {earned} / {}", crate::quest::MEDALS.len()), R::new(x0 + 4.0, y3, x0 + 140.0, y3 + 18.0), 11.0, 600, th.subtext, Align::Left);
        let hovered = match self.hot {
            Some(Btn::Medal(i)) => crate::quest::MEDALS.get(i),
            _ => None,
        };
        let about = match hovered {
            Some(m) if d.medals.iter().any(|k| k == m.key) => format!("{}: {} · earned", m.name, m.desc),
            Some(m) => format!("{}: {} · {} / {}", m.name, m.desc, crate::quest::medal_value(m, &stats).min(m.need), m.need),
            None => "Point at a medal to read it".to_string(),
        };
        self.r.text(&about, R::new(x0 + 120.0, y3, x1 - 4.0, y3 + 18.0), 10.5, 500, if hovered.is_some() { th.text } else { th.subtext.a(0.7) }, Align::Right);
        let n = crate::quest::MEDALS.len() as f32;
        let mw = (x1 - x0 - (n - 1.0) * 4.0) / n;
        for (i, m) in crate::quest::MEDALS.iter().enumerate() {
            let r = R::xywh(x0 + i as f32 * (mw + 4.0), y3 + 20.0, mw, 22.0);
            let got = d.medals.iter().any(|k| k == m.key);
            let hot = self.hot == Some(Btn::Medal(i));
            self.r.fill_round(r, 11.0, if got { th.accent } else if hot { th.hover } else { th.bg2.a(0.4) });
            self.r.icon(G_MEDAL, r, 13.0, if got { th.bg } else { th.subtext.a(0.45) });
            self.buttons.push((r, Btn::Medal(i)));
        }
        // A divider, then the buddy panel below it with room to breathe.
        let yl = y3 + 20.0 + 22.0 + 9.0;
        self.r.rect(R::new(x0 + 4.0, yl, x1 - 4.0, yl + 1.0), th.subtext.a(0.18));
        self.draw_buddy_panel(x0, x1, yl + 10.0);
    }

    /// Trainer card: your buddy's friendship — hearts, a bar with the "happy" mark, the
    /// number, what it evolves into, and how much petting is left today.
    fn draw_buddy_panel(&mut self, x0: f32, x1: f32, y: f32) {
        let th = self.theme.clone();
        let r = R::new(x0, y, x1, y + 58.0);
        self.r.fill_round(r, 12.0, th.bg2.a(0.55));
        let b = self.dex.buddy.clone();
        if b.is_empty() {
            self.r.text("♡", R::xywh(r.x0 + 8.0, r.y0, 30.0, r.h()), 18.0, 700, th.subtext.a(0.6), Align::Center);
            self.r.text("No buddy yet", R::new(r.x0 + 44.0, r.y0 + 8.0, r.x1 - 8.0, r.y0 + 26.0), 11.5, 600, th.text, Align::Left);
            self.r.text(
                "Click a caught Pokémon in the Pokédex grid to make it your buddy: it sits on the pill and grows fond of you",
                R::new(r.x0 + 44.0, r.y0 + 28.0, r.x1 - 8.0, r.y1 - 10.0),
                10.5,
                400,
                th.subtext,
                Align::Left,
            );
            return;
        }
        let f = self.dex.friendship.get(&b).copied().unwrap_or(pokemon::FRIEND_START);
        let show = if self.dex.shiny.contains_key(&b) && pokemon::has_sprite(&format!("{b}-shiny")) { format!("{b}-shiny") } else { b.clone() };
        if let Some((bmp, w, h)) = self.dex_thumb(&show) {
            let k = (44.0 / h).min(44.0 / w);
            let d = R::xywh(r.x0 + 6.0 + (44.0 - w * k) / 2.0, r.y0 + 7.0 + (44.0 - h * k) / 2.0, w * k, h * k);
            self.r.sprite_flip(&bmp, d, R::xywh(0.0, 0.0, w, h), true, false, 1.0);
        }
        let tx = r.x0 + 56.0;
        let name = format!("Buddy {}", pokemon::pretty(&b));
        let nw = self.text_width_at(&name, 11.5, 600);
        self.r.text(&name, R::new(tx, r.y0 + 7.0, tx + nw + 2.0, r.y0 + 25.0), 11.5, 600, th.text, Align::Left);
        self.r.text(&pokemon::hearts(f), R::new(tx + nw + 8.0, r.y0 + 6.0, tx + nw + 90.0, r.y0 + 25.0), 13.0, 700, th.accent, Align::Left);
        // What happens next.
        let evo = pokemon::friend_evo_text(&b);
        let what = if evo.is_empty() {
            if f >= pokemon::FRIEND_MAX { "Best friends!".to_string() } else { "Doesn't evolve by friendship · it just loves you".to_string() }
        } else if f >= pokemon::FRIEND_EVOLVE {
            format!("Happy! Evolves into {evo} (watch the pill)")
        } else {
            format!("{} more to evolve into {evo}", pokemon::FRIEND_EVOLVE - f)
        };
        self.r.text(&what, R::new(tx + nw + 92.0, r.y0 + 7.0, r.x1 - 10.0, r.y0 + 25.0), 10.5, 500, th.subtext, Align::Right);
        // The bar: 0–255 with a notch where it's happy enough to evolve.
        let bar = R::new(tx, r.y0 + 31.0, r.x1 - 110.0, r.y0 + 36.0);
        self.r.fill_round(bar, 2.5, th.bg2);
        let k = (f as f32 / pokemon::FRIEND_MAX as f32).clamp(0.0, 1.0);
        self.r.fill_round(R::new(bar.x0, bar.y0, bar.x0 + (bar.w() * k).max(5.0), bar.y1), 2.5, if f >= pokemon::FRIEND_EVOLVE { th.good } else { th.accent });
        if !evo.is_empty() {
            let mx = bar.x0 + bar.w() * pokemon::FRIEND_EVOLVE as f32 / pokemon::FRIEND_MAX as f32;
            self.r.rect(R::new(mx - 0.75, bar.y0 - 3.0, mx + 0.75, bar.y1 + 3.0), th.text.a(0.7));
        }
        self.r.text(&format!("{f} / {}", pokemon::FRIEND_MAX), R::new(r.x1 - 104.0, r.y0 + 25.0, r.x1 - 10.0, r.y0 + 42.0), 10.5, 600, th.text, Align::Right);
        let today = self.today_key();
        let petted = if self.dex.pet_day == today { self.dex.pet_today } else { 0 };
        let left = PET_DAILY.saturating_sub(petted);
        let pet = if left == 0 { "No more from petting today".to_string() } else { format!("Petting: +{PET_GAIN} each, {left} more today") };
        self.r.text(&format!("{pet} · +1 every {WALK_MINUTES} min at the PC together"), R::new(tx, r.y0 + 39.0, r.x1 - 10.0, r.y1 - 5.0), 9.5, 400, th.subtext, Align::Left);
    }

    /// Everything the quests and medals count.
    fn stats(&self) -> crate::quest::Stats {
        let d = &self.dex;
        crate::quest::Stats {
            caught: d.caught.values().sum(),
            species: pokemon::dex_list().filter(|n| d.caught.contains_key(*n)).count() as u32,
            shinies: d.shiny.values().sum(),
            legends: d.caught.iter().filter(|(n, _)| pokemon::is_legend(n)).map(|(_, c)| *c).sum(),
            hatched: d.hatched,
            battles: d.battles_won,
            raids: d.raids_won,
            combo: d.best_chain,
            days: d.days_played,
            full_days: d.full_days,
            level: d.level(),
            quests: d.quests_done.len() as u32,
            counts: d.counts.clone(),
            have: d.caught.keys().cloned().collect(),
        }
    }

    /// Finished a legend quest, earned a medal? A card for each (none the very first time,
    /// when everything already done is just noted).
    fn check_goals(&mut self) {
        let quiet = !self.dex.goals_seen;
        let mut changed = quiet;
        let stats = self.stats();
        let tint = self.theme.accent;
        for q in &crate::quest::QUESTS {
            let key = q.key.to_string();
            if self.dex.quests_open.contains(&key) || self.dex.quests_done.contains(&key) {
                continue;
            }
            let (got, need) = crate::quest::quest_progress(q, &stats);
            if got < need {
                continue;
            }
            changed = true;
            let legend = pokemon::pretty(q.legend);
            if stats.have.contains(q.legend) {
                // You have it already: the quest is simply done.
                self.dex.quests_done.push(key);
                util::log(&format!("legend quest done: {} (already have {legend})", q.title));
                if !quiet {
                    self.dex.bag.add(2, 1);
                    self.quick_card("quest", G_QUEST, tint, "Legend quest", "Legend quest complete!", &format!("{} · you have {legend} already, so here's an Ultra Ball", q.title), 6.0);
                }
            } else {
                self.dex.quests_open.push(key);
                util::log(&format!("legend quest done: {} → {legend} will visit", q.title));
                if !quiet {
                    self.quick_card("quest", G_QUEST, tint, "Legend quest", &format!("{legend} is coming to the island!"), &format!("{} · watch for it in the pill until you catch it", q.title), 7.0);
                }
            }
        }
        let stats = self.stats();
        for m in &crate::quest::MEDALS {
            if self.dex.medals.iter().any(|k| k == m.key) || crate::quest::medal_value(m, &stats) < m.need {
                continue;
            }
            changed = true;
            self.dex.medals.push(m.key.to_string());
            util::log(&format!("medal: {}", m.name));
            if !quiet {
                self.quick_card("medal", G_MEDAL, tint, "Medal", &format!("Medal earned: {}", m.name), m.desc, 5.0);
            }
        }
        self.dex.goals_seen = true;
        if changed {
            self.dex.save();
        }
    }

    /// An unlocked legend quest's legendary, half the time.
    fn quest_visitor(&self) -> Option<(&'static str, bool)> {
        let open: Vec<&'static str> = self.dex.quests_open.iter().filter_map(|k| crate::quest::QUESTS.iter().find(|q| q.key == k)).map(|q| q.legend).filter(|n| pokemon::has_sprite(n)).collect();
        (!open.is_empty() && rand01() < crate::quest::QUEST_SHARE).then(|| (open[(rand01() * open.len() as f32) as usize % open.len()], true))
    }

    /// The Pokédex's legend quests: what each asks, how far along you are, and the
    /// legendary it brings (a shadow until you've caught it).
    fn draw_quests(&mut self, x0: f32, x1: f32, y: f32) {
        let th = self.theme.clone();
        let stats = self.stats();
        let gap = 6.0;
        let (h, cols) = (42.0, 2usize);
        let tw = (x1 - x0 - gap) / cols as f32;
        for (i, q) in crate::quest::QUESTS.iter().enumerate() {
            let r = R::xywh(x0 + (i % cols) as f32 * (tw + gap), y + (i / cols) as f32 * (h + 5.0), tw, h);
            let open = self.dex.quests_open.iter().any(|k| k == q.key);
            let done = self.dex.quests_done.iter().any(|k| k == q.key);
            self.r.fill_round(r, 11.0, if done { th.accent.a(0.22) } else { th.bg2.a(0.55) });
            let caught = stats.have.contains(q.legend);
            if let Some((bmp, w, hh)) = self.dex_thumb(q.legend) {
                let k = (34.0 / hh).min(34.0 / w);
                let d = R::xywh(r.x0 + 4.0 + (34.0 - w * k) / 2.0, r.y0 + 4.0 + (34.0 - hh * k) / 2.0, w * k, hh * k);
                self.r.sprite_flip(&bmp, d, R::xywh(0.0, 0.0, w, hh), true, false, if caught || open { 1.0 } else { 0.18 });
            }
            let (got, need) = crate::quest::quest_progress(q, &stats);
            let legend = pokemon::pretty(q.legend);
            self.r.text(q.title, R::new(r.x0 + 44.0, r.y0 + 3.0, r.x1 - 8.0, r.y0 + 21.0), 11.0, 600, th.text, Align::Left);
            let (status, col) = if done {
                (format!("Done · {legend} caught"), th.good)
            } else if open {
                (format!("{legend} is visiting!"), th.accent)
            } else {
                (format!("→ {legend}"), th.subtext)
            };
            let bar = R::new(r.x0 + 44.0, r.y0 + 27.0, r.x0 + 124.0, r.y0 + 31.0);
            self.r.fill_round(bar, 2.0, th.bg2);
            let f = (got as f32 / need.max(1) as f32).clamp(0.0, 1.0);
            self.r.fill_round(R::new(bar.x0, bar.y0, bar.x0 + (bar.w() * f).max(4.0), bar.y1), 2.0, if got >= need { th.good } else { th.accent });
            self.r.text(&format!("{got} / {need}"), R::new(r.x0 + 130.0, r.y0 + 20.0, r.x0 + 180.0, r.y1 - 4.0), 10.5, 600, th.text, Align::Left);
            self.r.text(&status, R::new(r.x0 + 180.0, r.y0 + 20.0, r.x1 - 8.0, r.y1 - 4.0), 10.5, 600, col, Align::Right);
        }
    }

    /// Is Windows' do not disturb on? (Also keeps the Controls tab's toggle current.)
    fn dnd_on(&mut self) -> bool {
        self.ctl_dnd = controls::dnd();
        self.ctl_dnd == Some(true)
    }

    /// Minutes until the break reminder you set (0 = none).
    fn break_left_min(&self) -> u32 {
        self.break_at.map_or(0, |t| t.saturating_duration_since(Instant::now()).as_secs().div_ceil(60) as u32)
    }

    fn break_label(&self) -> String {
        match self.break_left_min() {
            0 => "Off".into(),
            m if m < 60 => format!("{m} min"),
            m if m % 60 == 0 => format!("{} h", m / 60),
            m => format!("{}h {:02}", m / 60, m % 60),
        }
    }

    /// The special-day Pokémon you haven't caught yet.
    fn special_missing(&self) -> Vec<(&'static str, &'static str)> {
        pokemon::special_missing(|n| self.dex.caught.contains_key(n))
    }

    /// The System tab's Close button: the first click asks "Sure?", the second closes it.
    fn close_app(&mut self, i: usize) {
        let Some(p) = self.sys_top.get(i).cloned() else { return };
        if self.kill_armed.as_deref() != Some(p.exe.as_str()) {
            self.kill_armed = Some(p.exe.clone());
            unsafe {
                SetTimer(Some(main_hwnd()), T_ARM, 4000, None);
            }
            return;
        }
        self.kill_armed = None;
        let nice = crate::procs::close(&p.pids);
        util::log(&format!("closed {} ({})", p.exe, if nice { "asked its windows to close" } else { "no windows, ended it" }));
        let body = if nice { "Asked it to close, so it can save first" } else { "It had no windows, so it was ended" };
        let tint = self.theme.accent;
        self.quick_card("close-app", G_CPU, tint, "System", &format!("Closing {}", p.name), body, 3.0);
    }

    /// The first time you're at the PC each day (from 5 am): a card with the weather,
    /// today's events and any special day or night.
    fn morning(&mut self) {
        if !self.cfg.morning_greeting {
            return;
        }
        let now = calendar::local_now();
        let today = format!("{:04}-{:02}-{:02}", now.date.y, now.date.m, now.date.d);
        if self.dex.greeted == today || now.h < 5 {
            return;
        }
        let why = if user_idle_ms() >= 120_000 {
            "you're away"
        } else if self.fullscreen || !self.visible {
            "something is full screen"
        } else if self.weather.is_none() && self.born.elapsed() < Duration::from_secs(180) {
            // Give the weather a moment to arrive after start-up.
            "waiting for the weather"
        } else {
            ""
        };
        if !why.is_empty() {
            util::log(&format!("morning greeting waiting: {why}"));
            return;
        }
        self.dex.greeted = today;
        self.dex.save();
        let hello = match now.h {
            5..=11 => "Good morning",
            12..=17 => "Good afternoon",
            _ => "Good evening",
        };
        let user = controls::user_name();
        let first = user.split_whitespace().next().unwrap_or("").to_string();
        let title = if first.is_empty() { format!("{hello}!") } else { format!("{hello}, {first}!") };
        let mut bits = Vec::new();
        let mut glyph = G_WEATHER;
        if let Some(w) = &self.weather {
            let (g, desc) = weather::describe(w.code, w.day);
            glyph = g;
            let hi = w.days.first().map(|d| format!(", high {}°", d.hi.round())).unwrap_or_default();
            bits.push(format!("{}° {}{hi}", w.temp.round(), desc.to_lowercase()));
        }
        let events = self.cal.on(now.date).len();
        bits.push(match events {
            0 => "no events today".to_string(),
            1 => "1 event today".to_string(),
            n => format!("{n} events today"),
        });
        if let Some((title, left)) = self.cal.upcoming(&now, 24 * 60) {
            bits.push(if left == 0 { format!("{title} now") } else { format!("{title} in {}", mins_text(left)) });
        }
        let mut body = bits.join(" · ");
        if let Some(c) = body.get(..1).map(|c| c.to_uppercase()) {
            body.replace_range(..1, &c);
        }
        // The special day, or what tonight might bring.
        let date = calendar::clock_text().0;
        let unix_tonight = crate::dex::now_unix() + (22_i64 - now.h as i64).max(0) * 3600;
        let app = match (pokemon::special_day(now.date), pokemon::tonight(now.date, unix_tonight)) {
            (Some(s), _) => format!("{date} · {}", s.name),
            (None, Some(s)) if s.key == "meteors" => format!("{date} · Meteor shower tonight if it's clear"),
            (None, Some(s)) => format!("{date} · {} tonight", capitalise(s.name)),
            _ => date,
        };
        util::log(&format!("morning greeting: {title} {app} / {body}"));
        let ttl = self.card_ttl(8.0);
        let icon = if self.dex.mascot.is_empty() { None } else { pokemon::first_frame(&self.dex.mascot) };
        self.push_card(Card {
            key: Some("morning".into()),
            glyph,
            tint: self.theme.accent,
            app,
            title,
            body,
            icon,
            sticky: false,
            ttl,
            focus: Focus::No,
            kind: CardKind::Plain,
        });
    }

    /// Today's special day or tonight's special night, if any.
    fn special(&self) -> Option<&'static pokemon::Special> {
        let now = calendar::local_now();
        pokemon::special_for(now.date, now.h, &self.conditions(), crate::dex::now_unix())
    }

    /// One in this many visitors of `name` is shiny: 1 in 64, better with a catch combo of
    /// that species (1 in 32 after one catch, down to 1 in 8), doubled on some special days.
    fn shiny_odds(&self, name: &str) -> u32 {
        let chain = if self.dex.chain_species == name { self.dex.chain.min(7) } else { 0 };
        let x = self.special().map(|s| s.shiny_x).unwrap_or(1.0);
        let o = if self.outbreak_species() == Some(name) { crate::quest::OUTBREAK_SHINY_X } else { 1.0 };
        ((64.0 / (1.0 + chain as f32) / x / o).round() as u32).max(4)
    }

    /// "Meeting in 5": a little wooden sign the Pokémon holds up before a calendar event.
    fn draw_sign(&mut self, mon: R, floor: f32, t: f32) {
        let Some((_, left)) = self.meeting.clone() else { return };
        let text = sign_text(left);
        let s = self.scale;
        let snap = |v: f32| (v * s).round() / s;
        let px = 1.0 / s;
        let egg = if self.dex.egg.is_some() || self.hatch.is_some() { 11.0 } else { 0.0 };
        let tw = self.text_width_at(&text, 9.5, 700);
        // It's held up: a gentle one-pixel bob.
        let bob = if (t * 1.6).sin() > 0.0 { 0.0 } else { px };
        let x = snap(mon.x1 + egg + 2.0);
        let board = R::new(x, snap(floor - 23.0) + bob, x + snap(tw + 10.0), snap(floor - 10.0) + bob);
        let wood = Rgba::rgb(214, 168, 108);
        let dark = Rgba::rgb(110, 74, 44);
        let post = snap(board.cx() - 1.0);
        self.r.rect(R::new(post, board.y1, post + snap(2.0).max(px), floor - 1.0), dark);
        self.r.fill_round(board, 2.5, if left == 0 { Rgba::rgb(236, 120, 96) } else { wood });
        self.r.stroke_round(board.inset(0.5), 2.0, dark, 1.0);
        self.r.text(&text, board, 9.5, 700, Rgba::rgb(58, 36, 18), Align::Center);
    }

    /// The weather outside (the Weather tab's place) and whether it's day.
    fn conditions(&self) -> pokemon::Conditions {
        let hour = calendar::local_now().h;
        let w = self.weather.as_ref();
        let day = w.map(|w| w.day).unwrap_or((6..19).contains(&hour));
        let mut c = pokemon::Conditions::new(w.map(|w| w.code), day, w.map(|w| w.temp), w.map(|w| w.wind));
        c.season = Some(pokemon::season_of(calendar::local_now().date.m, southern()));
        c
    }

    /// The Pokémon looking after the island (in Rayquaza's place) suits the weather: one of
    /// those that fit it and are downloaded, at random, kept until the weather changes or for
    /// three hours. Shiny from team level 50 when its shiny sprite is there.
    fn update_keeper(&mut self, announce: bool) {
        // Your buddy looks after the island instead.
        if self.weather.is_none() || self.buddy_sprite().is_some() {
            return;
        }
        let c = self.conditions();
        let (key, phrase, pool) = match self.special().filter(|s| s.keepers.iter().any(|n| pokemon::has_sprite(n))) {
            Some(s) => (s.key, s.phrase, s.keepers),
            None => pokemon::keeper_pool(&c),
        };
        let now = crate::dex::now_unix();
        let base = self.dex.mascot.trim_end_matches("-shiny").to_string();
        let fresh = now - self.dex.mascot_at < 3 * 3600;
        if self.dex.mascot_for == key && fresh && pool.contains(&base.as_str()) && pokemon::has_sprite(&self.dex.mascot) {
            return;
        }
        let have: Vec<&str> = pool.iter().copied().filter(|n| pokemon::has_sprite(n)).collect();
        let others: Vec<&str> = if have.len() > 1 { have.iter().copied().filter(|n| *n != base).collect() } else { have };
        let Some(&pick) = others.get((rand01() * others.len() as f32) as usize % others.len().max(1)) else {
            // None of them downloaded: Rayquaza looks after it.
            if !self.dex.mascot.is_empty() {
                self.dex.mascot.clear();
                self.dex.save();
                self.dirty = true;
            }
            return;
        };
        let shiny = format!("{pick}-shiny");
        let sprite = if self.dex.level() >= pokemon::SHINY_LEVEL && pokemon::has_sprite(&shiny) { shiny } else { pick.to_string() };
        let changed = sprite != self.dex.mascot;
        self.dex.mascot = sprite.clone();
        self.dex.mascot_for = key.to_string();
        self.dex.mascot_at = now;
        self.dex.save();
        self.dirty = true;
        util::log(&format!("island keeper: {sprite} ({key})"));
        if announce && changed && self.cfg.mascot && self.cfg.mascot_pokemon && !self.dnd_on() {
            if self.cfg.pokemon_cries && self.mic_apps.is_empty() {
                crate::sound::cry(pick, 0.5);
            }
            let ttl = self.card_ttl(5.0);
            self.push_card(Card {
                key: Some("keeper".into()),
                glyph: G_DEX,
                tint: self.theme.accent,
                app: "Your island".into(),
                title: format!("{} came out {phrase}!", pokemon::pretty(&sprite)),
                body: "It looks after the island while this weather lasts".into(),
                icon: pokemon::first_frame(&sprite),
                sticky: false,
                ttl,
                focus: Focus::No,
                kind: CardKind::Plain,
            });
        }
    }

    /// An egg turns up instead of a visitor now and then.
    fn find_egg(&mut self) {
        let all = pokemon::eggs();
        let have: Vec<&'static str> = all.iter().copied().filter(|n| pokemon::has_sprite(n)).collect();
        let pool: &[&'static str] = if have.is_empty() { all } else { &have };
        let name = pool[(rand01() * pool.len() as f32) as usize % pool.len()];
        let need = self.cfg.egg_minutes.max(1);
        self.dex.egg = Some(crate::dex::Egg { species: name.to_string(), shiny: rand01() < 1.0 / 32.0, minutes: 0, need });
        self.dex.save();
        util::log(&format!("found an egg ({name})"));
        let tint = self.theme.accent;
        let body = format!("It hatches after about {} at your PC (watching or listening counts)", mins_text(need));
        self.quick_card("egg", G_DEX, tint, "Pokédex", "You found a Pokémon Egg!", &body, 5.0);
    }

    /// Once a minute: the egg grows while you're at the computer, and hatches when it's ready.
    fn egg_minute(&mut self) {
        // A minute at the PC: you used the mouse or keyboard, or something's playing (a video,
        // music, a game in full screen) — watching counts too.
        let here = user_idle_ms() < 60_000 || self.playing || self.game_child.is_some();
        let grow = self.dex.egg.as_ref().is_some_and(|e| e.minutes < e.need) && here;
        if grow {
            if let Some(e) = self.dex.egg.as_mut() {
                e.minutes += 1;
            }
            self.dex.save();
        }
        let ready = self.dex.egg.as_ref().is_some_and(|e| e.minutes >= e.need);
        if ready && self.hatch.is_none() && self.wild.is_none() && self.mode == Mode::Compact && self.visible && !self.fullscreen && self.cfg.mascot {
            if let Some(n) = self.dex.egg.as_ref().map(|e| e.sprite()) {
                let bh = self.pill_h - 5.0;
                self.sprite_box(&n, bh);
            }
            util::log("an egg is hatching");
            self.hatch = Some(Instant::now());
            self.dirty = true;
        }
    }

    fn finish_hatch(&mut self) {
        self.hatch = None;
        self.dirty = true;
        let Some(e) = self.dex.egg.take() else { return };
        let name = e.species.clone();
        *self.dex.caught.entry(name.clone()).or_default() += 1;
        if e.shiny {
            *self.dex.shiny.entry(name.clone()).or_default() += 1;
        }
        *self.dex.seen.entry(name.clone()).or_default() += 1;
        self.dex.hatched += 1;
        self.dex.remember(e.sprite());
        self.dex.save();
        self.pending_evolution = true;
        self.dex_thumbs.clear();
        if self.cfg.pokemon_cries && self.mic_apps.is_empty() {
            crate::sound::cry(&name, 0.5);
        }
        let sprite = e.sprite();
        util::log(&format!("egg hatched: {sprite}"));
        let ttl = self.card_ttl(6.0);
        self.push_card(Card {
            key: Some("egg".into()),
            glyph: G_DEX,
            tint: self.theme.good,
            app: "Pokédex".into(),
            title: format!("Oh? The Egg hatched into {}{}!", if e.shiny { "a shiny " } else { "" }, pokemon::pretty(&name)),
            body: "+50 XP · a Pokémon you can only get from Eggs".into(),
            icon: pokemon::first_frame(&sprite),
            sticky: false,
            ttl,
            focus: Focus::No,
            kind: CardKind::Plain,
        });
        self.gain_xp(50);
        self.progress(|c| c.kind == "hatch");
    }

    /// The egg in pixels at (x, y), with `cracks` (0..1) across it.
    fn draw_egg_px(&mut self, x: f32, y: f32, px: f32, a: f32, cracks: f32) {
        for (j, row) in EGG.iter().enumerate() {
            for (i, ch) in row.chars().enumerate() {
                let c = match ch {
                    'X' => Rgba::rgb(70, 58, 48),
                    'W' => Rgba::rgb(250, 244, 228),
                    'S' => Rgba::rgb(124, 200, 120),
                    _ => continue,
                };
                self.r.rect(R::xywh(x + i as f32 * px, y + j as f32 * px, px, px), c.a(a));
            }
        }
        const CRACK: [(f32, f32); 5] = [(1.0, 4.0), (2.0, 3.0), (3.0, 4.0), (4.0, 3.0), (5.0, 4.0)];
        let n = (cracks * CRACK.len() as f32).ceil() as usize;
        for (i, j) in CRACK.iter().take(n) {
            self.r.rect(R::xywh(x + i * px, y + j * px, px, px), Rgba::rgb(70, 58, 48).a(a));
        }
    }

    /// The egg beside the Pokémon: it wobbles now and then (more often as it gets close),
    /// then shakes, cracks, flashes and hatches.
    fn draw_egg(&mut self, mon: R, floor: f32, t: f32) {
        let s = self.scale;
        let snap = |v: f32| (v * s).round() / s;
        let px = (1.25 * s).round().max(1.0) / s;
        let (x, y) = (snap(mon.x1 + 1.5), snap(floor - 9.0 * px - 0.5));
        let (cx, cy) = (x + 3.5 * px, y + 4.5 * px);
        if let Some(at) = self.hatch {
            let c = at.elapsed().as_secs_f32();
            if c < 1.8 {
                let dx = snap((c * 28.0).sin() * px * (0.6 + c / 1.8));
                self.draw_egg_px(x + dx, y, px, 1.0, ((c - 0.5) / 1.2).clamp(0.0, 1.0));
            } else if c < 2.3 {
                let k = (c - 1.8) / 0.5;
                self.draw_egg_px(x, y, px, 1.0 - k, 1.0);
                self.r.glow(cx, cy, 6.0 + 16.0 * k, Rgba::rgb(255, 255, 255).a(0.85));
            } else {
                let k = ((c - 2.3) / 0.6).min(1.0);
                self.r.glow(cx, cy, 22.0, Rgba::rgb(255, 255, 255).a(0.6 * (1.0 - k)));
                let baby = self.dex.egg.as_ref().map(|e| e.sprite()).unwrap_or_default();
                let bh = self.pill_h - 5.0;
                if let Some((dw, dh)) = self.sprite_box(&baby, bh) {
                    let dest = R::xywh(snap(cx - dw / 2.0), snap(floor - dh), snap(dw), snap(dh));
                    if let Some(Some(sp)) = self.strips.get(&baby) {
                        let i = pokemon::frame_at(&sp.delays, (t * 1000.0) as u64);
                        self.r.sprite_flip(&sp.bmp, dest, R::xywh(i as f32 * sp.fw, 0.0, sp.fw, sp.fh), true, false, k);
                    }
                }
                for (k2, (ox, oy)) in [(-9.0, -6.0), (8.0, -9.0), (10.0, 2.0)].iter().enumerate() {
                    let ph = ((c - 2.3) * 3.0 + k2 as f32 * 0.7).sin().max(0.0);
                    self.twinkle(cx + ox, cy + oy, 3.0 * ph, Rgba::rgb(255, 230, 120).a(ph));
                }
            }
            return;
        }
        let Some(e) = self.dex.egg.as_ref() else { return };
        let p = e.minutes as f32 / e.need.max(1) as f32;
        let period = if p > 0.9 { 1.2 } else if p > 0.5 { 3.0 } else { 6.0 };
        let w = t % period;
        let dx = if w < 0.5 { snap((w / 0.5 * std::f32::consts::TAU * 2.0).sin() * px) } else { 0.0 };
        self.draw_egg_px(x + dx, y, px, 1.0, 0.0);
    }

    fn dex_thumb(&mut self, name: &str) -> Option<(ID2D1Bitmap1, f32, f32)> {
        if !self.dex_thumbs.contains_key(name) {
            let t = pokemon::first_frame(name).and_then(|a| {
                let (w, h) = (a.w as f32, a.h as f32);
                self.r.bitmap(&a).map(|b| (b, w, h))
            });
            self.dex_thumbs.insert(name.to_string(), t);
        }
        self.dex_thumbs.get(name).cloned().flatten()
    }

    fn draw_dex_tab(&mut self, x0: f32, x1: f32, mut y: f32) {
        let th = self.theme.clone();
        let lvl = self.dex.level();
        let (a, b) = (crate::dex::xp_at(lvl), crate::dex::xp_at(lvl + 1));
        let frac = if lvl >= 100 { 1.0 } else { (self.dex.xp.saturating_sub(a)) as f32 / (b - a).max(1) as f32 };
        self.r.fill_round(R::new(x0, y, x1, y + 60.0), 12.0, th.bg2.a(0.55));
        // The Pokémon on the island now.
        let name = self.mascot_sprite(self.pip_mood());
        if let Some((bmp, w, h)) = self.dex_thumb(&name) {
            let k = (44.0 / h).min(44.0 / w);
            let d = R::xywh(x0 + 8.0 + (44.0 - w * k) / 2.0, y + 8.0 + (44.0 - h * k) / 2.0, w * k, h * k);
            self.r.sprite_flip(&bmp, d, R::xywh(0.0, 0.0, w, h), true, false, 1.0);
        }
        let lv = format!("Lv {lvl}");
        let lw = self.text_width_at(&lv, 20.0, 700);
        self.r.text(&lv, R::new(x0 + 60.0, y + 6.0, x0 + 62.0 + lw, y + 32.0), 20.0, 700, th.text, Align::Left);
        self.r.text(&pokemon::pretty(&name), R::new(x0 + 70.0 + lw, y + 9.0, x1 - 150.0, y + 31.0), 12.0, 600, th.subtext, Align::Left);
        let bar = R::new(x0 + 60.0, y + 36.0, x1 - 160.0, y + 41.0);
        self.r.fill_round(bar, 2.5, th.bg2);
        self.r.fill_round(R::new(bar.x0, bar.y0, bar.x0 + (bar.w() * frac.clamp(0.0, 1.0)).max(5.0), bar.y1), 2.5, th.accent);
        let met = self.dex.met.clone();
        let next = match pokemon::next_change_of(lvl, |n| met.iter().any(|m| m == n)) {
            Some((l, ch)) => {
                let who = match ch.as_slice() {
                    [(_, to)] => pokemon::pretty(to),
                    many => format!("{} evolutions", many.len()),
                };
                format!("{} / {} XP · {who} at Lv {l}", self.dex.xp - a, b - a)
            }
            None => format!("{} XP", self.dex.xp),
        };
        self.r.text(&next, R::new(x0 + 60.0, y + 42.0, x1 - 150.0, y + 58.0), 10.5, 400, th.subtext, Align::Left);
        let (species, size) = if self.dex_shiny {
            (pokemon::dex_list().chain(pokemon::form_list()).filter(|n| self.dex.shiny.contains_key(*n)).count(), pokemon::DEX_SIZE + pokemon::FORMS_SIZE)
        } else if self.dex_forms {
            (pokemon::form_list().filter(|n| self.dex.caught.contains_key(*n)).count(), pokemon::FORMS_SIZE)
        } else {
            (pokemon::dex_list().filter(|n| self.dex.caught.contains_key(*n)).count(), pokemon::DEX_SIZE)
        };
        let total: u32 = self.dex.caught.values().sum();
        let shinies: u32 = self.dex.shiny.values().sum();
        self.r.text(&format!("{species} / {size}"), R::new(x1 - 150.0, y + 8.0, x1 - 12.0, y + 32.0), 16.0, 700, th.text, Align::Right);
        let sub = if self.dex_shiny {
            "shinies caught".to_string()
        } else if self.dex_forms {
            "forms caught".to_string()
        } else if shinies > 0 { format!("caught · {shinies} shiny") } else if total > 0 { format!("caught · {total} in all") } else { "Click wild Pokémon to catch them".into() };
        self.r.text(&sub, R::new(x1 - 220.0, y + 32.0, x1 - 12.0, y + 50.0), 10.5, 400, th.subtext, Align::Right);
        y += 70.0;
        // HP, what the weather brings out, and the egg.
        let mut bits = Vec::new();
        if !self.dex.buddy.is_empty() {
            let b = self.dex.buddy.clone();
            let f = self.dex.friendship.get(&b).copied().unwrap_or(pokemon::FRIEND_START);
            let evo = pokemon::friend_evo_text(&b);
            let evo = if evo.is_empty() { String::new() } else if f >= pokemon::FRIEND_EVOLVE { format!(" · happy: evolves into {evo}") } else { format!(" · evolves into {evo} when happy") };
            bits.push(format!("Buddy {} {} {f}/{}{evo}", pokemon::pretty(&b), pokemon::hearts(f), pokemon::FRIEND_MAX));
        }
        if let Some(h) = self.hp_now() {
            bits.push(if h <= FAINT { "Fainted: Claude's 5-hour limit is used up".to_string() } else { format!("HP {}% (Claude's 5-hour limit)", (h * 100.0).round()) });
        }
        if let Some(s) = self.special() {
            bits.push(s.line.to_string());
        }
        if let Some(o) = self.dex.outbreak.clone().filter(|o| o.until > crate::dex::now_unix()) {
            bits.push(format!("Mass outbreak: {} ({} left)", pokemon::pretty(&o.species), mins_text(((o.until - crate::dex::now_unix()) / 60).max(1) as u32)));
        }
        let c = self.conditions();
        let why = pokemon::spawn_line(&c);
        if !why.is_empty() {
            bits.push(why);
        }
        if let Some(s) = c.season {
            bits.push(pokemon::season_line(s).to_string());
        }
        if !c.day {
            bits.push(pokemon::moon_phase_name(pokemon::moon_age(crate::dex::now_unix())).to_string());
        }
        if self.dex.chain >= 2 {
            let name = self.dex.chain_species.clone();
            bits.push(format!("Combo: {} ×{} (shiny 1 in {})", pokemon::pretty(&name), self.dex.chain, self.shiny_odds(&name)));
        }
        if let Some((a, b, at)) = self.next_caught_evolution() {
            bits.push(format!("Next: {} → {} at Lv {at}", pokemon::pretty(&a), pokemon::pretty(b)));
        }
        if self.ctl_dnd == Some(true) {
            bits.push("Do not disturb: wild Pokémon paused".into());
        }
        if let Some(e) = &self.dex.egg {
            bits.push(if e.minutes >= e.need { "Egg: about to hatch!".to_string() } else { format!("Egg: {} to go", mins_text(e.need - e.minutes)) });
        }
        if bits.is_empty() {
            bits.push("Rain, sun and night change who visits".into());
        }
        if self.dex.buddy.is_empty() {
            bits.push("Click a caught Pokémon to make it your buddy".into());
        }
        self.r.text_wrapped(&bits.join("  ·  "), R::new(x0 + 4.0, y - 4.0, x1 - 4.0, y + 28.0), 10.5, 400, th.subtext);
        y += 34.0;
        // Recently caught, and how the grid is sorted.
        self.r.text("Recent", R::new(x0 + 4.0, y, x0 + 60.0, y + 28.0), 10.5, 600, th.subtext, Align::Left);
        let mut x = x0 + 50.0;
        let recent = self.dex.recent.clone();
        if recent.is_empty() {
            self.r.text("Nothing caught yet", R::new(x, y, x1 - 330.0, y + 28.0), 10.5, 400, th.subtext.a(0.6), Align::Left);
        }
        for s in &recent {
            if x + 28.0 > x1 - 328.0 {
                break;
            }
            if let Some((bmp, w, h)) = self.dex_thumb(s) {
                let k = (26.0 / h).min(26.0 / w);
                let d = R::xywh(x + (28.0 - w * k) / 2.0, y + 1.0 + (26.0 - h * k) / 2.0, w * k, h * k);
                self.r.sprite_flip(&bmp, d, R::xywh(0.0, 0.0, w, h), true, false, 1.0);
                if s.ends_with("-shiny") {
                    self.twinkle(x + 3.0, y + 4.0, 2.5, Rgba::rgb(255, 215, 90));
                }
            }
            x += 30.0;
        }
        let view = if self.dex_card { 2 } else if self.dex_bag { 5 } else if self.dex_quests { 6 } else if self.dex_shiny { 4 } else if self.dex_forms { 3 } else if self.dex.by_type { 1 } else { 0 };
        for (i, (label, v)) in [("No.", 0u8), ("Type", 1), ("Forms", 3), ("Shiny", 4), ("Bag", 5), ("Quests", 6), ("Trainer", 2)].into_iter().enumerate() {
            let r = R::new(x1 - 322.0 + i as f32 * 46.0, y + 4.0, x1 - 279.0 + i as f32 * 46.0, y + 24.0);
            let on = view == v;
            let hot = self.hot == Some(Btn::DexView(v));
            self.r.fill_round(r, 10.0, if on { th.accent } else if hot { th.hover } else { th.bg2.a(0.6) });
            self.r.text(label, r, 10.5, 600, if on { th.bg } else { th.text }, Align::Center);
            self.buttons.push((r, Btn::DexView(v)));
        }
        y += 32.0;
        if self.dex_card {
            return self.draw_trainer(x0, x1, y);
        }
        if self.dex_bag {
            return self.draw_bag(x0, x1, y);
        }
        if self.dex_quests {
            return self.draw_quests(x0, x1, y);
        }
        let per = DEX_COLS * DEX_ROWS;
        // Regional forms on their own pages, then the Megas, then Gigantamax.
        let form_pages = || {
            let group = |n: &str| match pokemon::species(n).map(|s| pokemon::region(s)) {
                Some("Mega") => 1,
                Some("Gigantamax") => 2,
                _ => 0,
            };
            let mut out: Vec<Vec<&'static str>> = Vec::new();
            for g in 0..3 {
                let list: Vec<&'static str> = pokemon::form_list().filter(|n| group(n) == g).collect();
                out.extend(list.chunks(per).map(|c| c.to_vec()));
            }
            out
        };
        let shiny_view = self.dex_shiny;
        let all_pages: Vec<Vec<&'static str>> = if shiny_view {
            // All 1025 by number, then every form.
            let order: Vec<&'static str> = pokemon::dex_list().collect();
            let mut out: Vec<Vec<&'static str>> = order.chunks(per).map(|c| c.to_vec()).collect();
            out.extend(form_pages());
            out
        } else if self.dex_forms {
            form_pages()
        } else {
            let mut order: Vec<&'static str> = pokemon::dex_list().collect();
            if self.dex.by_type {
                order.sort_by_key(|n| pokemon::type_of(n).unwrap_or(0));
            }
            // One page of the National Pokédex at a time: ‹ Kanto · #001–072 ›.
            order.chunks(per).map(|c| c.to_vec()).collect()
        };
        let pages = all_pages.len().max(1);
        self.dex_pages = pages;
        self.dex_page = self.dex_page.min(pages - 1);
        let page: Vec<&'static str> = all_pages.get(self.dex_page).cloned().unwrap_or_default();
        let (a, b) = (page.first().and_then(|n| pokemon::species(n)), page.last().and_then(|n| pokemon::species(n)));
        let span = |x: &str, y: &str| if x == y { x.to_string() } else { format!("{x}–{y}") };
        let label = match (a, b) {
            (Some(a), Some(b)) if (self.dex_forms || shiny_view) && a.form > 0 => {
                let kind = span(pokemon::region(a), pokemon::region(b));
                if kind.contains("Alola") || kind.contains("Galar") || kind.contains("Hisui") { "Regional forms".to_string() } else { kind }
            }
            (Some(a), Some(b)) if self.dex.by_type && !shiny_view => {
                let t = |s: &crate::national::Species| pokemon::TYPES[s.t1 as usize].0;
                span(t(a), t(b))
            }
            (Some(a), Some(b)) => format!("{} · #{:03}–{:03}", span(pokemon::region(a), pokemon::region(b)), a.id, b.id),
            _ => String::new(),
        };
        let here = page.iter().filter(|n| if shiny_view { self.dex.shiny.contains_key(**n) } else { self.dex.caught.contains_key(**n) }).count();
        for (r, d, glyph) in [(R::new(x0, y, x0 + 30.0, y + 22.0), -1, "‹"), (R::new(x1 - 30.0, y, x1, y + 22.0), 1, "›")] {
            let hot = self.hot == Some(Btn::DexPage(d));
            self.r.fill_round(r, 10.0, if hot { th.hover } else { th.bg2.a(0.6) });
            self.r.text(glyph, r, 16.0, 700, th.text, Align::Center);
            self.buttons.push((r, Btn::DexPage(d)));
        }
        let mid = R::new(x0 + 36.0, y, x1 - 36.0, y + 22.0);
        self.r.text(&format!("{label}   ·   {here} / {} caught   ·   page {} of {pages}", page.len(), self.dex_page + 1), mid, 11.0, 600, th.subtext, Align::Center);
        y += 26.0;
        let cols = DEX_COLS;
        let cw = (x1 - x0) / cols as f32;
        let rows = DEX_ROWS;
        for (i, name) in page.into_iter().enumerate() {
            let cell = R::xywh(x0 + (i % cols) as f32 * cw, y + (i / cols) as f32 * DEX_CELL, cw, DEX_CELL).inset(2.0);
            let n = self.dex.caught.get(name).copied().unwrap_or(0);
            let sh = self.dex.shiny.get(name).copied().unwrap_or(0);
            if shiny_view {
                // Its shiny: caught, or a faint hint if you've only caught the usual colours.
                self.r.fill_round(cell, 9.0, th.bg2.a(if sh > 0 { 0.6 } else { 0.28 }));
                let (show, alpha) = if sh > 0 { (format!("{name}-shiny"), 1.0) } else if n > 0 { (name.to_string(), 0.18) } else { (String::new(), 0.0) };
                if show.is_empty() {
                    self.r.text("?", cell, 14.0, 700, th.subtext.a(0.3), Align::Center);
                    continue;
                }
                if let Some((bmp, w, h)) = self.dex_thumb(&show) {
                    let k = ((cell.h() - 6.0) / h).min((cell.w() - 6.0) / w);
                    let d = R::xywh(cell.cx() - w * k / 2.0, cell.cy() - h * k / 2.0, w * k, h * k);
                    self.r.sprite_flip(&bmp, d, R::xywh(0.0, 0.0, w, h), true, false, alpha);
                }
                if sh > 0 {
                    if sh > 1 {
                        self.r.text(&format!("×{sh}"), R::new(cell.x0, cell.y1 - 13.0, cell.x1 - 3.0, cell.y1 - 1.0), 9.0, 700, th.subtext, Align::Right);
                    }
                    self.twinkle(cell.x0 + 6.0, cell.y0 + 6.0, 3.0, Rgba::rgb(255, 215, 90));
                }
                continue;
            }
            self.r.fill_round(cell, 9.0, th.bg2.a(if n > 0 { 0.6 } else { 0.28 }));
            if self.dex.by_type {
                if let Some(t) = pokemon::type_of(name) {
                    let (r, g, b) = pokemon::TYPES[t].1;
                    self.r.circle(cell.x1 - 5.0, cell.y0 + 5.0, 2.2, Rgba::rgb(r, g, b).a(if n > 0 { 1.0 } else { 0.55 }));
                }
            }
            if n == 0 && pokemon::eggs().contains(&name) {
                // Only from Eggs.
                let px = (1.4 * self.scale).round().max(1.0) / self.scale;
                self.draw_egg_px(cell.cx() - 3.5 * px, cell.cy() - 4.5 * px, px, 0.4, 0.0);
                continue;
            }
            if n == 0 {
                let seen = self.dex.seen.contains_key(name);
                self.r.text("?", cell, 14.0, 700, if seen { th.accent.a(0.6) } else { th.subtext.a(0.3) }, Align::Center);
                continue;
            }
            let show = if sh > 0 && sh == n { format!("{name}-shiny") } else { name.to_string() };
            if let Some((bmp, w, h)) = self.dex_thumb(&show) {
                let k = ((cell.h() - 6.0) / h).min((cell.w() - 6.0) / w);
                let d = R::xywh(cell.cx() - w * k / 2.0, cell.cy() - h * k / 2.0, w * k, h * k);
                self.r.sprite_flip(&bmp, d, R::xywh(0.0, 0.0, w, h), true, false, 1.0);
            }
            if n > 1 {
                self.r.text(&format!("×{n}"), R::new(cell.x0, cell.y1 - 13.0, cell.x1 - 3.0, cell.y1 - 1.0), 9.0, 700, th.subtext, Align::Right);
            }
            if sh > 0 {
                self.twinkle(cell.x0 + 6.0, cell.y0 + 6.0, 3.0, Rgba::rgb(255, 215, 90));
            }
            if self.dex.buddy == name {
                self.r.text("♥", R::new(cell.x1 - 14.0, cell.y0, cell.x1 - 1.0, cell.y0 + 13.0), 10.0, 700, th.accent, Align::Center);
            } else if self.hot == Some(Btn::Buddy(name)) {
                self.r.text("♡", R::new(cell.x1 - 14.0, cell.y0, cell.x1 - 1.0, cell.y0 + 13.0), 10.0, 700, th.subtext, Align::Center);
            }
            self.buttons.push((cell, Btn::Buddy(name)));
        }
        if shiny_view {
            let y = y + rows as f32 * DEX_CELL + 4.0;
            let line = format!(
                "Shiny Pokémon sparkle as they arrive (about 1 in {}) · catch the same Pokémon again and again to make its shiny more likely · Eggs can hatch shiny too",
                self.shiny_odds("-")
            );
            self.r.text_wrapped(&line, R::new(x0 + 4.0, y, x1 - 4.0, y + 30.0), 10.5, 400, th.subtext);
            return;
        }
        if self.dex_forms {
            let y = y + rows as f32 * DEX_CELL + 4.0;
            let line = format!(
                "Regional forms wander by and evolve into their own forms · Megas: Pokémon you've caught, at team Lv {} · Gigantamax: rare visitors, more in storms and wind",
                pokemon::MEGA_LEVEL
            );
            self.r.text_wrapped(&line, R::new(x0 + 4.0, y, x1 - 4.0, y + 30.0), 10.5, 400, th.subtext);
            return;
        }
        // Who you can only meet on special days and nights, and haven't caught yet.
        let missing = self.special_missing();
        if !missing.is_empty() {
            let y = y + rows as f32 * DEX_CELL + 4.0;
            let list: Vec<String> = missing.iter().map(|(n, when)| format!("{} ({when})", pokemon::pretty(n))).collect();
            let line = format!("Still to catch on special days and in rare weather: {}", list.join(" · "));
            self.r.text_wrapped(&line, R::new(x0 + 4.0, y, x1 - 4.0, y + 30.0), 10.5, 400, th.subtext);
        }
    }

    /// A short peek with an icon, a heading, a title and a line.
    #[allow(clippy::too_many_arguments)]
    fn quick_card(&mut self, key: &str, glyph: &'static str, tint: Rgba, app: &str, title: &str, body: &str, secs: f32) {
        let ttl = self.card_ttl(secs);
        self.push_card(Card {
            key: Some(key.into()),
            glyph,
            tint,
            app: app.into(),
            title: title.into(),
            body: body.into(),
            icon: None,
            sticky: false,
            ttl,
            focus: Focus::No,
            kind: CardKind::Plain,
        });
    }

    /// Wi-Fi dropped / came back / changed, a VPN went up or down.
    fn net_changed(&mut self, old: &crate::net::Net, new: &crate::net::Net) {
        let name = |n: &crate::net::Net| match &n.wifi {
            Some(s) if !s.is_empty() => s.clone(),
            Some(_) => "Wi-Fi".to_string(),
            None => "Ethernet".to_string(),
        };
        let (good, bad, accent) = (self.theme.good, self.theme.bad, self.theme.accent);
        if old.online() && !new.online() {
            let what = if old.wifi.is_some() { format!("Lost {}", name(old)) } else { "Ethernet unplugged".into() };
            self.quick_card("net", G_WIFI_OFF, bad, "Network", "Offline", &what, 4.0);
        } else if !old.online() && new.online() {
            let g = if new.wifi.is_some() { G_WIFI } else { G_ETHERNET };
            self.quick_card("net", g, good, "Network", "Back online", &name(new), 3.0);
        } else if let (Some(a), Some(b)) = (&old.wifi, &new.wifi) {
            if a != b && !b.is_empty() {
                self.quick_card("net", G_WIFI, accent, "Wi-Fi", b, "Switched networks", 3.0);
            }
        }
        if old.vpn != new.vpn {
            match (&new.vpn, &old.vpn) {
                (Some(v), _) => self.quick_card("vpn", G_VPN, good, "VPN", "Connected", v, 3.0),
                (None, Some(v)) => self.quick_card("vpn", G_VPN, bad, "VPN", "Disconnected", v, 4.0),
                _ => {}
            }
        }
    }

    /// Once a minute. A stretch of use ends after 3 minutes away; after `break_minutes` of it,
    /// a 20-20-20 break: look 20 ft (6 m) away for 20 seconds. Not during a call (the
    /// microphone is on) or anything full-screen; it asks again a minute later.
    fn check_break(&mut self) {
        self.egg_minute();
        self.buddy_minute();
        self.update_keeper(true);
        self.morning();
        self.count_day();
        if std::mem::take(&mut self.pending_evolution) {
            let evolved = self.evolve_caught();
            self.evolution_card(&evolved);
        }
        if self.break_at.is_some_and(|t| Instant::now() >= t) {
            self.break_at = None;
            self.use_since = Instant::now();
            util::log("break reminder due");
            self.break_card("Your break reminder".into());
            return;
        }
        if user_idle_ms() >= 3 * 60_000 {
            self.use_since = Instant::now();
            return;
        }
        let mins = self.cfg.break_minutes;
        if mins == 0 || self.use_since.elapsed() < Duration::from_secs(mins as u64 * 60) {
            return;
        }
        if !self.mic_apps.is_empty() || self.fullscreen || self.card.as_ref().is_some_and(|c| c.kind == CardKind::Break) {
            return;
        }
        self.use_since = Instant::now();
        self.gain_xp(10);
        let h = mins / 60;
        let spent = if h > 0 && mins % 60 == 0 { format!("{h} h") } else { format!("{mins} min") };
        self.break_card(format!("{spent} on screen"));
    }

    /// The 20-20-20 eye break card.
    fn break_card(&mut self, app: String) {
        self.push_card(Card {
            key: Some("break".into()),
            glyph: G_EYE,
            tint: self.theme.good,
            app,
            title: "Rest your eyes".into(),
            body: String::new(),
            icon: None,
            sticky: false,
            ttl: Duration::from_secs_f32(BREAK_SECS + 2.5),
            focus: Focus::No,
            kind: CardKind::Break,
        });
    }

    /// Seconds into the current track right now.
    fn track_pos(&self) -> Option<(f64, f64)> {
        let (pos, end, at) = self.timeline?;
        let now = if self.playing { pos + at.elapsed().as_secs_f64() } else { pos };
        Some((now.min(end), end))
    }

    /// A click that opens or closes the island waits a moment, in case it's a double-click.
    fn defer_click(&mut self, open: bool) {
        self.pending_click = Some(open);
        let ms = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() }.clamp(150, 320);
        unsafe {
            SetTimer(Some(self.hwnd), T_CLICK, ms, None);
        }
    }

    /// Double-click on the island (not on a button or a Pokémon): play / pause.
    fn double_click(&mut self, lp: LPARAM) {
        let (x, y) = self.logical(lp);
        let on_button = self.mode == Mode::Expanded && self.buttons.iter().any(|(r, _)| r.contains(x, y));
        let on_mon = self.mode == Mode::Compact
            && (self.mon_rect.is_some_and(|r| r.inset(-3.0).contains(x, y))
                || self.wild_rect.is_some_and(|r| r.inset(-12.0).contains(x, y))
                || (self.fishing.is_some() && self.fish_rect.is_some_and(|r| r.inset(-6.0).contains(x, y))));
        if on_button || on_mon || !matches!(self.mode, Mode::Compact | Mode::Expanded) {
            // Two quick clicks on a button are two clicks.
            self.mouse_down(lp);
            return;
        }
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_CLICK);
        }
        self.pending_click = None;
        self.swallow_up = true;
        if self.media.is_some() {
            let _ = self.media_tx.send(MediaCmd::PlayPause);
            self.hop = Some(Instant::now());
            self.dirty = true;
        }
    }

    /// Middle-click: mute / unmute, with the volume pop-up.
    fn middle_click(&mut self) {
        let r = self.audio.as_ref().and_then(|a| a.toggle_mute());
        if let Some((level, muted)) = r {
            self.last_vol = Some((level, muted));
            self.show_osd(OsdKind::Volume, level, muted);
        }
    }

    fn right_down(&mut self, lp: LPARAM) {
        let (x, _) = self.logical(lp);
        if matches!(self.mode, Mode::Compact | Mode::Expanded) && self.media.is_some() {
            if let Some((pos, _)) = self.track_pos() {
                self.scrub = Some((x, pos, false));
                self.scrub_to = pos;
                unsafe {
                    SetCapture(self.hwnd);
                }
            }
        }
    }

    fn right_move(&mut self, lp: LPARAM) {
        let Some((x0, from, moved)) = self.scrub else { return };
        let Some((_, end)) = self.track_pos() else { return };
        let (x, _) = self.logical(lp);
        let dx = x - x0;
        let moving = moved || dx.abs() > 6.0;
        // 300 px of dragging spans the whole song.
        let span = 300.0;
        self.scrub_to = (from + dx as f64 / span * end).clamp(0.0, end);
        self.scrub = Some((x0, from, moving));
        self.dirty = true;
    }

    /// Right button up: finish a scrub, or (if it didn't move) open the menu.
    fn right_up(&mut self) -> bool {
        let Some((_, _, moved)) = self.scrub.take() else { return false };
        unsafe {
            let _ = ReleaseCapture();
        }
        self.dirty = true;
        if moved {
            let _ = self.media_tx.send(MediaCmd::Seek(self.scrub_to));
            if let Some((_, end, _)) = self.timeline {
                self.timeline = Some((self.scrub_to, end, Instant::now()));
            }
            return true;
        }
        false
    }

    fn mouse_leave(&mut self) {
        self.pet_at = None;
        self.tracking = false;
        self.hover = false;
        self.hot = None;
        if let Some(c) = &self.card {
            // Give a card the user just looked at a moment more.
            let floor = Instant::now() + Duration::from_millis(1200);
            if self.card_since + c.ttl < floor {
                self.card_since = floor - c.ttl;
            }
        }
        unsafe {
            let _ = KillTimer(Some(self.hwnd), T_HOVER);
            SetTimer(Some(self.hwnd), T_LEAVE, 280, None);
        }
        self.schedule_expiry();
        self.dirty = true;
    }

    fn click(&mut self, lp: LPARAM) {
        let (x, y) = self.logical(lp);
        self.ripples.push((x, y, Instant::now()));
        if self.mode == Mode::Compact {
            // On (or just beside) a visitor: throw. While a ball is out, clicks there do
            // nothing, so catching never opens the island.
            if self.wild.is_some() && self.wild_rect.is_some_and(|r| r.inset(-12.0).contains(x, y)) {
                if self.wild.as_ref().is_some_and(|w| w.catch.is_none() && w.fight.is_none()) {
                    if self.wild.as_ref().is_some_and(|w| w.battle) {
                        self.start_battle();
                    } else {
                        self.throw_ball();
                    }
                }
                self.dirty = true;
                return;
            }
            // Fishing: a click on the bobber (or the mascot) while it bites reels it in; on the
            // bobber at any other time it does nothing (it never opens the island).
            if self.fishing.is_some() {
                let on_rod = self.fish_rect.is_some_and(|r| r.inset(-6.0).contains(x, y));
                let on_mon = self.mon_rect.is_some_and(|r| r.inset(-3.0).contains(x, y));
                if (on_rod || on_mon) && self.fishing.as_ref().is_some_and(|f| f.bite.is_some()) {
                    self.reel_in();
                    return;
                }
                if on_rod {
                    self.dirty = true;
                    return;
                }
            }
            if self.mon_rect.is_some_and(|r| r.inset(-3.0).contains(x, y)) {
                self.poke();
                self.dirty = true;
                return;
            }
        }
        match self.mode {
            Mode::Card => {
                let hit = self.buttons.iter().find(|(r, _)| r.contains(x, y)).map(|(_, b)| *b);
                if let (Some(b @ (Btn::CardFolder | Btn::CardShelf)), Some(CardKind::Download(p))) = (hit, self.card.as_ref().map(|c| c.kind.clone())) {
                    self.next_card();
                    if b == Btn::CardFolder {
                        crate::scan::reveal(&p, true);
                    } else {
                        self.shelve(p);
                    }
                    self.schedule_expiry();
                    self.dirty = true;
                    return;
                }
                if let Some(c) = &self.card {
                    focus(c.focus);
                    match &c.kind {
                        CardKind::Shot(Some(path)) => open_path(path),
                        CardKind::Download(path) => open_path(path),
                        CardKind::Link(url) => open_path(Path::new(url)),
                        CardKind::Song => {
                            self.tab = Tab::Media;
                            self.expanded = true;
                        }
                        _ => {}
                    }
                }
                self.next_card();
                self.schedule_expiry();
            }
            Mode::Decision => {
                if let Some((_, Btn::Decide(which))) = self.buttons.iter().find(|(r, _)| r.contains(x, y)).copied() {
                    self.answer(0, which);
                }
            }
            Mode::Expanded => {
                if let Some((_, b)) = self.buttons.iter().find(|(r, _)| r.contains(x, y)).copied() {
                    match b {
                        Btn::Prev => drop(self.media_tx.send(MediaCmd::Prev)),
                        Btn::PlayPause => drop(self.media_tx.send(MediaCmd::PlayPause)),
                        Btn::Next => drop(self.media_tx.send(MediaCmd::Next)),
                        Btn::Tab(t) => {
                            self.tab = t;
                            self.enter_tab();
                        }
                        Btn::ShelfFile(i) if !self.shelf_dragged => {
                            if let Some(p) = self.shelf.get(i).cloned() {
                                if p.exists() {
                                    self.expanded = false;
                                    open_path(&p);
                                } else {
                                    self.shelf.remove(i);
                                    crate::shelf::save(&self.shelf);
                                }
                            }
                        }
                        Btn::ShelfFile(_) => {}
                        Btn::ShelfRemove(i) => {
                            if i < self.shelf.len() {
                                self.shelf.remove(i);
                                crate::shelf::save(&self.shelf);
                            }
                        }
                        Btn::Clip(i) => self.recopy(i),
                        Btn::ClipPage(d) => {
                            let pages = self.clips.len().div_ceil(CLIP_ROWS).max(1) as i32;
                            self.clip_page = (self.clip_page as i32 + d).rem_euclid(pages) as usize;
                        }
                        Btn::ClipRemove(i) => {
                            if i < self.clips.len() {
                                self.clips.remove(i);
                                self.save_clips();
                            }
                        }
                        Btn::ClipClear => {
                            self.clips.clear();
                            self.clip_page = 0;
                            self.save_clips();
                        }
                        Btn::CardFolder | Btn::CardShelf => {}
                        Btn::Medal(_) => {}
                        Btn::Buddy(name) => self.set_buddy(name),
                        Btn::Fish => {
                            self.toggle_fishing();
                            if self.fishing.is_some() {
                                self.expanded = false;
                            }
                        }
                        Btn::WallNext => {
                            if self.wpe_asked.map_or(true, |t| t.elapsed() > Duration::from_secs(4)) {
                                self.wpe_asked = Some(Instant::now());
                                crate::wpe::change_now();
                            }
                        }
                        Btn::Output(i) => {
                            if let Some(o) = self.outputs.get(i).cloned() {
                                if !o.default {
                                    let ok = crate::devices::set_default_output(&o.id);
                                    util::log(&format!("sound output: {} ({}) {}", o.name, o.hw, if ok { "is the default now" } else { "couldn't be switched" }));
                                    self.refresh_outputs();
                                }
                            }
                        }
                        Btn::Note(i) => {
                            if let Some(n) = self.notes.get_mut(i) {
                                n.done = !n.done;
                                crate::notes::save(&self.notes);
                            }
                        }
                        Btn::NoteRemove(i) => {
                            if i < self.notes.len() {
                                self.notes.remove(i);
                                crate::notes::save(&self.notes);
                            }
                        }
                        Btn::NotesClear => {
                            self.notes.retain(|n| !n.done);
                            crate::notes::save(&self.notes);
                        }
                        Btn::NoteAdd => {
                            self.note_typing = true;
                            unsafe {
                                let _ = SetForegroundWindow(self.hwnd);
                                let _ = SetFocus(Some(self.hwnd));
                            }
                        }
                        Btn::Ball(b) => {
                            if b == 0 || self.dex.bag.count(b).unwrap_or(0) > 0 {
                                self.dex.bag.pick = b;
                                self.dex.save();
                            }
                        }
                        Btn::DexView(v) => {
                            self.dex_card = v == 2;
                            self.dex_bag = v == 5;
                            self.dex_quests = v == 6;
                            let grid = v != 2 && v != 5 && v != 6;
                            if grid && ((v == 3) != self.dex_forms || (v == 4) != self.dex_shiny) {
                                self.dex_page = 0;
                            }
                            if grid {
                                self.dex_forms = v == 3;
                                self.dex_shiny = v == 4;
                            }
                            if v < 2 {
                                self.dex.by_type = v == 1;
                                self.dex.save();
                            }
                        }
                        Btn::DexPage(d) => {
                            let pages = self.dex_pages.max(1) as i32;
                            self.dex_page = (self.dex_page as i32 + d).rem_euclid(pages) as usize;
                        }
                        Btn::SysSort(m) => self.sys_by_mem = m,
                        Btn::Kill(i) => self.close_app(i),
                        Btn::Shortcut(i) => {
                            if let Some(sc) = self.cfg.shortcuts.get(i).cloned() {
                                self.expanded = false;
                                controls::launch(&sc);
                            }
                        }
                        Btn::Quick(q) => self.quick(q),
                        Btn::SysPage(p) => {
                            self.sys_page = p;
                            if p == 4 {
                                // Fresh sizes each time the Storage page is opened.
                                crate::scan::close();
                                crate::scan::forget(std::path::Path::new(""));
                            }
                        }
                        Btn::SysBack => self.sys_back(),
                        Btn::Drive(i) => {
                            if let Some(d) = self.sys.as_ref().and_then(|s| s.drives.get(i)).cloned() {
                                self.scan_page = 0;
                                crate::scan::open(std::path::PathBuf::from(format!("{}\\", d.root)), scan_wake);
                            }
                        }
                        Btn::ScanRow(i) => {
                            let l = crate::scan::current();
                            if let Some(e) = l.entries.get(i) {
                                let p = l.path.join(&e.name);
                                if e.dir {
                                    self.scan_page = 0;
                                    crate::scan::open(p, scan_wake);
                                } else {
                                    crate::scan::reveal(&p, true);
                                }
                            }
                        }
                        Btn::ScanPage(d) => {
                            let n = crate::scan::current().entries.len().div_ceil(SCAN_ROWS).max(1) as i32;
                            self.scan_page = (self.scan_page as i32 + d).clamp(0, n - 1) as usize;
                        }
                        Btn::ScanExplorer => {
                            let l = crate::scan::current();
                            if !l.path.as_os_str().is_empty() {
                                crate::scan::reveal(&l.path, false);
                            }
                        }
                        Btn::GameOpen => crate::games::pick(),
                        Btn::GamePlay(i) => self.play_game(i),
                        Btn::GameRemove(i) => {
                            if i < self.games.len() {
                                self.games.remove(i);
                                crate::games::save(&self.games);
                            }
                        }
                        Btn::GameShow => crate::games::show(),
                        Btn::GameStop => crate::games::stop(),
                        Btn::DndBar => {
                            self.quick(Quick::Dnd);
                            util::log(&format!("do not disturb {} from the tab bar", if self.ctl_dnd == Some(true) { "on" } else { "off" }));
                        }
                        Btn::Slider(_) | Btn::Decide(_) => {}
                        Btn::Power(act) => {
                            if !act.needs_confirm() || self.armed == Some(act) {
                                self.armed = None;
                                self.expanded = false;
                                act.run();
                            } else {
                                // Ending the session: ask for a second click.
                                self.armed = Some(act);
                                unsafe {
                                    SetTimer(Some(main_hwnd()), T_ARM, 4000, None);
                                }
                            }
                        }
                    }
                } else {
                    self.defer_click(false);
                }
            }
            Mode::Compact => self.defer_click(true),
            Mode::Osd => self.osd = None,
        }
        self.dirty = true;
    }

    fn quick(&mut self, q: Quick) {
        match q {
            Quick::Dnd => {
                controls::set_dnd(!self.ctl_dnd.unwrap_or(false));
                self.ctl_dnd = controls::dnd();
            }
            Quick::Mute => {
                if let Some(v) = self.audio.as_ref().and_then(|a| a.toggle_mute()) {
                    self.last_vol = Some(v);
                }
            }
            Quick::MicMute => self.ctl_mic = self.audio.as_ref().and_then(|a| a.toggle_mic_mute()),
            Quick::Screenshot => {
                self.expanded = false;
                controls::screenshot();
            }
            Quick::Keyboard => controls::toggle_keyboard(),
            Quick::DarkMode => {
                controls::set_dark_mode(!self.ctl_dark);
                self.ctl_dark = controls::dark_mode();
            }
        }
    }

    fn quick_active(&self, q: Quick) -> bool {
        match q {
            Quick::Dnd => self.ctl_dnd.unwrap_or(false),
            Quick::Mute => self.last_vol.map(|v| v.1).unwrap_or(false),
            Quick::MicMute => self.ctl_mic.map(|v| v.1).unwrap_or(false),
            Quick::DarkMode => self.ctl_dark,
            Quick::Screenshot | Quick::Keyboard => false,
        }
    }

    /// Pressing on a slider starts dragging it.
    fn mouse_down(&mut self, lp: LPARAM) {
        if self.mode != Mode::Expanded {
            return;
        }
        let (x, y) = self.logical(lp);
        self.shelf_dragged = false;
        self.shelf_press = match self.buttons.iter().find(|(r, _)| r.contains(x, y)) {
            Some((_, Btn::ShelfFile(i))) => Some((*i, x, y)),
            _ => None,
        };
        let hit = self.buttons.iter().find(|(r, b)| matches!(b, Btn::Slider(_)) && r.contains(x, y)).map(|(_, b)| *b);
        if let Some(Btn::Slider(s)) = hit {
            self.dragging = Some(s);
            unsafe {
                SetCapture(self.hwnd);
            }
            self.slide(s, x);
        }
    }

    /// Returns true if a slider drag ended (so the button-up isn't also a click).
    fn end_drag(&mut self) -> bool {
        if let Some(s) = self.dragging.take() {
            if s == Slider::Break {
                util::log(&format!("break reminder: {}", self.break_label()));
            }
            unsafe {
                let _ = ReleaseCapture();
            }
            self.dirty = true;
            return true;
        }
        false
    }

    fn slide(&mut self, s: Slider, x: f32) {
        let (a, b) = self.track;
        let f = ((x - a) / (b - a).max(1.0)).clamp(0.0, 1.0);
        match s {
            Slider::Volume => {
                if let Some(v) = self.audio.as_ref().and_then(|a| a.set_level(f)) {
                    self.last_vol = Some(v);
                }
            }
            Slider::Mic => self.ctl_mic = self.audio.as_ref().and_then(|a| a.set_mic_level(f)),
            Slider::Break => {
                let m = ((f * BREAK_MAX / 5.0).round() * 5.0) as u64;
                self.break_at = (m > 0).then(|| Instant::now() + Duration::from_secs(m * 60));
            }
            Slider::Brightness => {
                let n = (f * 100.0).round() as u8;
                if self.brightness != Some(n) {
                    self.brightness = Some(n);
                    let _ = self.bright_tx.send(n);
                }
            }
        }
        self.dirty = true;
    }

    /// Two-finger swipe (or tilt wheel) sideways: the music slides with your fingers and skips
    /// once it's gone far enough. Right = next, left = previous.
    fn hwheel(&mut self, delta: i16) {
        if self.media.is_none() || self.swipe_lock.is_some_and(|t| t.elapsed() < Duration::from_millis(650)) {
            return;
        }
        self.swipe_acc += delta as f32;
        self.swipe.set(-(self.swipe_acc / SWIPE_AT).clamp(-1.0, 1.0) * SWIPE_PX * 0.6);
        if self.swipe_acc.abs() >= SWIPE_AT {
            let next = self.swipe_acc > 0.0;
            let _ = self.media_tx.send(if next { MediaCmd::Next } else { MediaCmd::Prev });
            // The old song leaves on the side you swiped towards; the next comes in from the other.
            self.swipe.snap(if next { SWIPE_PX } else { -SWIPE_PX });
            self.swipe.set(0.0);
            self.swipe_acc = 0.0;
            self.swipe_lock = Some(Instant::now());
            self.hop = Some(Instant::now());
        }
        unsafe {
            SetTimer(Some(self.hwnd), T_SWIPE, 220, None);
        }
        self.dirty = true;
    }

    /// The clipboard changed: peek with what was copied.
    fn clipboard(&mut self) {
        if !(self.cfg.copy_peek || self.cfg.screenshot_peek) || self.born.elapsed() < Duration::from_secs(3) {
            return;
        }
        // Some apps set the clipboard several times in a row; peek once.
        if self.last_clip.is_some_and(|t| t.elapsed() < Duration::from_millis(400)) {
            return;
        }
        let Some(clip) = peeks::clipboard(self.hwnd) else { return };
        self.last_clip = Some(Instant::now());
        self.remember_clip(&clip);
        let card = match clip {
            peeks::Clip::Text(t) if self.cfg.copy_peek => {
                let preview = peeks::preview(&t);
                if preview.is_empty() {
                    return;
                }
                Card {
                    key: Some("copy".into()),
                    glyph: G_CLIPBOARD,
                    tint: self.theme.accent,
                    app: "Copied".into(),
                    title: preview,
                    body: format!("{} characters", t.chars().count()),
                    icon: None,
                    sticky: false,
                    ttl: Duration::from_millis(2200),
                    focus: Focus::No,
                    kind: CardKind::Plain,
                }
            }
            peeks::Clip::Image(art, w, h, _) if self.cfg.screenshot_peek || self.cfg.copy_peek => Card {
                key: Some("shot".into()),
                glyph: G_SCREENSHOT,
                tint: art.as_ref().map(|a| Rgba::from_f(a.tint)).unwrap_or(self.theme.accent),
                app: "Image copied".into(),
                title: if w > 0 { format!("{w} × {h}") } else { "Picture".into() },
                body: "Ready to paste".into(),
                icon: art,
                sticky: false,
                ttl: self.card_ttl(3.0),
                focus: Focus::No,
                kind: CardKind::Shot(None),
            },
            _ => return,
        };
        self.push_card(card);
    }

    /// Charger plugged in or out, or the battery level moved: peek at the moments that matter.
    fn power_changed(&mut self) {
        let Some((ac, pct)) = peeks::power() else { return };
        let before = self.power.replace((ac, pct));
        if !self.cfg.power_peek {
            return;
        }
        let th = self.theme.clone();
        let (glyph, tint, title, body, charging) = match before {
            Some((was_ac, _)) if was_ac != ac && ac => {
                self.low_warned = 0;
                (G_BATTERY_CHARGING, th.good, format!("Charging · {pct}%"), "Plugged in", true)
            }
            Some((was_ac, _)) if was_ac != ac => {
                let tint = if pct <= 20 { th.bad } else { th.accent };
                (G_BATTERY, tint, format!("On battery · {pct}%"), "Unplugged", false)
            }
            _ if !ac && ((pct <= 10 && self.low_warned != 10) || (pct <= 20 && pct > 10 && self.low_warned == 0)) => {
                self.low_warned = if pct <= 10 { 10 } else { 20 };
                (G_BATTERY_LOW, th.bad, format!("Battery low · {pct}%"), "Plug in soon", false)
            }
            _ => return,
        };
        self.push_card(Card {
            key: Some("power".into()),
            glyph,
            tint,
            app: "Battery".into(),
            title,
            body: body.into(),
            icon: None,
            sticky: false,
            ttl: Duration::from_millis(3200),
            focus: Focus::No,
            kind: CardKind::Power { pct, charging },
        });
    }

    fn wheel(&mut self, delta: i16) {
        if let Some(a) = &self.audio {
            let step = self.cfg.volume_step.clamp(1, 20) as i32;
            if let Some((level, muted)) = a.step(if delta > 0 { step } else { -step }) {
                self.last_vol = Some((level, muted));
                self.show_osd(OsdKind::Volume, level, muted);
            }
        }
    }

    fn timer(&mut self, id: usize) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), id);
        }
        match id {
            T_HOVER => {
                let (mx, my) = self.mouse;
                // Petting, aiming at a visitor or watching the ball wobble: don't open.
                let catching = self.wild.as_ref().is_some_and(|w| w.catch.is_some());
                let on_mon = catching
                    || self.mon_rect.is_some_and(|r| r.inset(-3.0).contains(mx, my))
                    || self.wild_rect.is_some_and(|r| r.inset(-24.0).contains(mx, my))
                    || (self.fishing.is_some() && self.fish_rect.is_some_and(|r| r.inset(-12.0).contains(mx, my)));
                if on_mon && self.hover && self.mode == Mode::Compact {
                    unsafe {
                        SetTimer(Some(self.hwnd), T_HOVER, 260, None);
                    }
                } else if self.hover && self.mode == Mode::Compact {
                    self.refresh_calendar();
                    self.pick_tab();
                    self.expanded = true;
                }
            }
            T_LEAVE => {
                if !self.hover && !self.note_typing {
                    self.expanded = false;
                }
            }
            T_EXPIRE => self.expire(),
            T_ARM => {
                self.armed = None;
                self.kill_armed = None;
            }
            T_DECIDE => self.prune_decisions(),
            T_WALL => self.check_wallpaper(),
            T_CLOCK => self.tick_clock(),
            T_POWER => self.power_changed(),
            T_PAUSED => self.dirty = true,
            T_BREAK => {
                // Timers stop after firing (above): keep this one going every minute.
                unsafe {
                    SetTimer(Some(main_hwnd()), T_BREAK, 60_000, None);
                }
                self.check_break();
            }
            T_WILD => self.wild_timer(),
            T_DEMO => self.demo_step(),
            T_ONTOP => {
                unsafe {
                    SetTimer(Some(main_hwnd()), T_ONTOP, ONTOP_MS, None);
                }
                self.keep_on_top();
            }
            T_CLICK => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), T_CLICK);
                }
                match self.pending_click.take() {
                    Some(true) if self.mode == Mode::Compact => {
                        self.refresh_calendar();
                        self.pick_tab();
                        self.expanded = true;
                    }
                    Some(false) => self.expanded = false,
                    _ => {}
                }
            }
            T_SWIPE => {
                // Fingers lifted before it counted: spring back.
                self.swipe_acc = 0.0;
                self.swipe.set(0.0);
            }
            T_CAL => {
                let now = calendar::local_now();
                for (title, time) in self.cal.due(&now, self.cfg.calendar_reminder_minutes) {
                    self.push_card(Card {
                        key: Some(format!("cal:{title}")),
                        glyph: G_CAL,
                        tint: self.theme.accent,
                        app: "Calendar".into(),
                        title,
                        body: format!("Starts at {time}"),
                        icon: None,
                        sticky: false,
                        ttl: self.card_ttl(7.0),
                        focus: Focus::No,
                        kind: CardKind::Plain,
                    });
                }
                self.refresh_calendar();
                self.schedule_calendar();
            }
            _ => {}
        }
        self.dirty = true;
    }

    // ---------- the demo's tour ----------

    fn demo_open(&mut self, tab: Tab, view: u8) {
        self.expanded = true;
        self.tab = tab;
        self.dex_card = view == 1;
        self.dex_quests = view == 2;
        self.dex_bag = view == 3;
        self.dex_forms = false;
        self.dex_shiny = false;
        self.enter_tab();
        if tab == Tab::Controls {
            crate::demo::scrub_outputs(&mut self.outputs);
        }
    }

    fn demo_agent(&mut self, event: &str, extra: serde_json::Value) {
        let mut v = serde_json::json!({ "agent": "claude", "event": event, "session": "demo", "cwd": "C:\\code\\pocketisle" });
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            for (k, x) in e {
                o.insert(k.clone(), x.clone());
            }
        }
        self.agent_message(&v.to_string());
    }

    /// One scene of the tour, then the timer for the next.
    fn demo_step(&mut self) {
        use crate::events::send;
        let i = DEMO_STEP.fetch_add(1, Ordering::SeqCst);
        let (name, ms): (&str, u32) = match i {
            0 => {
                send(Event::Net(crate::net::Net { wifi: Some("Lagoon Wi-Fi".into()), wired: false, vpn: None }));
                send(Event::Weather(crate::demo::weather(false)));
                for u in crate::demo::usage() {
                    send(Event::Usage(u));
                }
                send(Event::Privacy { mic: Vec::new(), cam: Vec::new() });
                ("pill", 4500)
            }
            1 => {
                send(Event::Notification { app: "Messages".into(), title: "Maya".into(), body: "Still on for lunch at 1? 🌮".into(), icon: None });
                ("notification", 4500)
            }
            2 => {
                self.demo_agent("UserPromptSubmit", serde_json::json!({ "activity": "Thinking" }));
                ("claude-working", 3500)
            }
            3 => {
                self.demo_agent("PreToolUse", serde_json::json!({ "activity": "Editing README.md" }));
                ("claude-editing", 3500)
            }
            4 => {
                self.demo_agent("PermissionRequest", serde_json::json!({ "id": "demo-1", "pid": std::process::id(), "question": "Run this command?", "detail": "cargo test --release", "activity": "Running cargo test" }));
                ("claude-asks", 6500)
            }
            5 => {
                self.demo_agent("DecisionDone", serde_json::json!({ "id": "demo-1" }));
                self.demo_agent("Stop", serde_json::json!({}));
                ("claude-done", 3500)
            }
            6 => {
                send(Event::Media(Some(MediaInfo { title: "Ocean Avenue Drive".into(), artist: "The Tidewaters".into(), app: "Spotify".into() })));
                send(Event::MediaArt(Some(crate::demo::album_art())));
                send(Event::Playback(true));
                send(Event::Timeline { pos: 48.0, end: 213.0 });
                ("song", 6000)
            }
            7 => {
                self.show_osd(OsdKind::Volume, 0.62, false);
                ("volume", 2600)
            }
            8 => {
                self.show_osd(OsdKind::Brightness, 0.75, false);
                ("brightness", 2600)
            }
            9 => {
                send(Event::Privacy { mic: vec!["Teams".into()], cam: vec!["Teams".into()] });
                ("camera", 4000)
            }
            10 => {
                send(Event::Privacy { mic: Vec::new(), cam: Vec::new() });
                send(Event::GitHub(vec![crate::github::Note {
                    kind: crate::github::NoteKind::Passed,
                    place: "you/pocketisle #42".into(),
                    title: "Add fishing beside the mascot".into(),
                    url: "https://github.com/".into(),
                    who: String::new(),
                }]));
                ("github", 4500)
            }
            11 => {
                send(Event::Download { path: crate::demo::dir().join("Downloads").join("vacation-playlist.zip"), size: 48_213_000 });
                ("download", 4500)
            }
            12 => {
                send(Event::OutputDevice { name: "Earbuds Pro".into(), battery: Some(80) });
                ("headphones", 4000)
            }
            13 => {
                self.card = None;
                self.queue.clear();
                self.demo_open(Tab::Media, 0);
                ("tab-music", 5000)
            }
            14 => {
                self.demo_open(Tab::Usage, 0);
                ("tab-usage", 4500)
            }
            15 => {
                self.demo_open(Tab::Calendar, 0);
                ("tab-calendar", 4500)
            }
            16 => {
                self.demo_open(Tab::System, 0);
                ("tab-system", 5000)
            }
            17 => {
                self.demo_open(Tab::Weather, 0);
                ("tab-weather", 4500)
            }
            18 => {
                self.demo_open(Tab::Shelf, 0);
                ("tab-shelf", 4500)
            }
            19 => {
                self.demo_open(Tab::Dex, 0);
                ("tab-pokedex", 5000)
            }
            20 => {
                self.demo_open(Tab::Dex, 1);
                ("tab-trainer", 5000)
            }
            21 => {
                self.demo_open(Tab::Dex, 2);
                ("tab-quests", 4500)
            }
            22 => {
                self.demo_open(Tab::Dex, 3);
                ("tab-bag", 4500)
            }
            23 => {
                self.demo_open(Tab::Game, 0);
                ("tab-games", 4000)
            }
            24 => {
                self.demo_open(Tab::Home, 0);
                ("tab-home", 4500)
            }
            25 => {
                self.demo_open(Tab::Controls, 0);
                ("tab-controls", 4500)
            }
            26 => {
                self.demo_open(Tab::Power, 0);
                ("tab-power", 4000)
            }
            27 => {
                self.expanded = false;
                send(Event::Playback(false));
                ("collapse", 3000)
            }
            28 => {
                self.card = None;
                self.queue.clear();
                self.spawn_wild();
                ("wild", 9000)
            }
            29 => {
                self.throw_ball();
                ("catch", 7000)
            }
            30 => {
                self.card = None;
                self.queue.clear();
                if self.wild.is_some() {
                    self.wild = None;
                    self.wild_rect = None;
                }
                self.toggle_fishing();
                if let Some(f) = self.fishing.as_mut() {
                    f.bite_at = Instant::now() + Duration::from_millis(3200);
                }
                ("fishing", 4400)
            }
            31 => {
                if self.fishing.as_ref().is_some_and(|f| f.bite.is_some()) {
                    self.reel_in();
                }
                ("reel", 3500)
            }
            32 => {
                self.throw_ball();
                ("catch-fish", 7000)
            }
            33 => {
                self.card = None;
                self.queue.clear();
                self.wild = None;
                self.wild_rect = None;
                self.fishing = None;
                self.dex.egg = Some(crate::dex::Egg { species: "togepi".into(), shiny: false, minutes: 1, need: 1 });
                ("egg", 2500)
            }
            34 => {
                self.egg_minute();
                ("hatch", 7000)
            }
            35 => {
                self.card = None;
                self.queue.clear();
                send(Event::Weather(crate::demo::weather(true)));
                ("rain", 6000)
            }
            _ => {
                crate::demo::scene("end");
                unsafe {
                    PostQuitMessage(0);
                }
                return;
            }
        };
        crate::demo::scene(name);
        self.dirty = true;
        unsafe {
            SetTimer(Some(main_hwnd()), T_DEMO, ms, None);
        }
    }

    /// Always-on-top windows of other apps (and the app you just switched to, if it is one)
    /// can end up in front of the island. Put every island window back on top when that happens.
    fn keep_on_top(&self) {
        let mut hs = vec![self.hwnd];
        hs.extend(self.views.iter().map(|v| v.hwnd));
        for h in hs {
            if covered(h) {
                unsafe {
                    let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
                }
            }
        }
    }

    fn reposition(&mut self) {
        let p = placement(&self.theme, &self.cfg);
        self.scale = p.scale;
        self.pill_h = p.pill_h;
        self.monitor = p.monitor;
        self.neck = p.neck;
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), p.x, p.y, p.pw, p.ph, SWP_NOACTIVATE);
        }
        if let Err(e) = self.r.resize(p.pw as u32, p.ph as u32, p.scale) {
            util::log(&format!("resize failed: {e}"));
        }
        self.last_rgn = (0, 0, 0, 0);
        self.drawn = [R::new(0.0, 0.0, WIN_W, WIN_H); 2];
        self.dirty = true;
        self.sync_views();
    }

    /// An island of its own at the top of every other screen: its own window, renderer
    /// (on the same graphics device) and state, so hovering or opening one leaves the other
    /// alone. Rebuilt whenever screens come, go or change. Called with the main island's
    /// state in place.
    fn sync_views(&mut self) {
        for v in self.views.drain(..) {
            crate::shelf::stop_drops(v.hwnd);
            unsafe {
                let _ = DestroyWindow(v.hwnd);
            }
        }
        if !self.cfg.all_monitors {
            return;
        }
        let hinst: HINSTANCE = unsafe { GetModuleHandleW(None) }.map(|h| h.into()).unwrap_or_default();
        for mon in other_monitors(self.monitor) {
            let p = placement_on(mon, &self.theme, &self.cfg);
            let made = unsafe {
                CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP,
                    MIRROR_CLASS,
                    w!("Pocket Isle"),
                    WS_POPUP,
                    p.x,
                    p.y,
                    p.pw,
                    p.ph,
                    None,
                    None,
                    Some(hinst),
                    None,
                )
            };
            let hwnd = match made {
                Ok(h) => h,
                Err(e) => {
                    util::log(&format!("island on another screen failed: {e}"));
                    continue;
                }
            };
            match self.r.sibling(hwnd, p.pw as u32, p.ph as u32, p.scale) {
                Ok(r) => {
                    unsafe {
                        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    }
                    let now = Instant::now();
                    let pill_h = p.pill_h;
                    crate::shelf::accept_drops(hwnd);
                    self.views.push(View {
                        hwnd,
                        r,
                        scale: p.scale,
                        pill_h,
                        monitor: p.monitor,
                        neck: p.neck,
                        mood: std::array::from_fn(|i| Spring::new(self.mood[i].v, 60.0, 15.0, 0.002)),
                        glow: Spring::new(0.0, 90.0, 18.0, 0.01),
                        mouse: (0.0, 0.0),
                        peak: Spring::new(0.0, 260.0, 26.0, 0.01),
                        ripples: Vec::new(),
                        beat: 0.0,
                        beat_slow: 0.0,
                        last_beat: now,
                        swipe: Spring::new(0.0, 240.0, 22.0, 0.2),
                        w: Spring::new(self.w.v, 380.0, 27.0, 0.3),
                        h: Spring::new(pill_h, 380.0, 27.0, 0.3),
                        rad: Spring::new(pill_h / 2.0, 380.0, 27.0, 0.2),
                        alpha: Spring::new(1.0, 260.0, 30.0, 0.01),
                        mode: Mode::Compact,
                        last_rgn: (0, 0, 0, 0),
                        drawn: [R::new(0.0, 0.0, WIN_W, WIN_H); 2],
                        dirty_rect: R::new(0.0, 0.0, WIN_W, WIN_H),
                        last: now,
                        visible: true,
                        fullscreen: false,
                        hover: false,
                        tracking: false,
                        expanded: false,
                        buttons: Vec::new(),
                        hot: None,
                        tab: Tab::Media,
                        tab_w: std::array::from_fn(|_| Spring::new(TAB_H, 420.0, 30.0, 0.3)),
                        dragging: None,
                        track: (0.0, 1.0),
                        sys_on: false,
                        mon_rect: None,
                        wild_rect: None,
                    });
                }
                Err(e) => {
                    util::log(&format!("island renderer on another screen failed: {e}"));
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            }
        }
        self.dirty = true;
    }

    fn swap_view(&mut self, v: &mut View) {
        std::mem::swap(&mut self.hwnd, &mut v.hwnd);
        std::mem::swap(&mut self.r, &mut v.r);
        std::mem::swap(&mut self.scale, &mut v.scale);
        std::mem::swap(&mut self.pill_h, &mut v.pill_h);
        std::mem::swap(&mut self.monitor, &mut v.monitor);
        std::mem::swap(&mut self.neck, &mut v.neck);
        std::mem::swap(&mut self.mood, &mut v.mood);
        std::mem::swap(&mut self.glow, &mut v.glow);
        std::mem::swap(&mut self.mouse, &mut v.mouse);
        std::mem::swap(&mut self.peak, &mut v.peak);
        std::mem::swap(&mut self.ripples, &mut v.ripples);
        std::mem::swap(&mut self.beat, &mut v.beat);
        std::mem::swap(&mut self.beat_slow, &mut v.beat_slow);
        std::mem::swap(&mut self.last_beat, &mut v.last_beat);
        std::mem::swap(&mut self.swipe, &mut v.swipe);
        std::mem::swap(&mut self.w, &mut v.w);
        std::mem::swap(&mut self.h, &mut v.h);
        std::mem::swap(&mut self.rad, &mut v.rad);
        std::mem::swap(&mut self.alpha, &mut v.alpha);
        std::mem::swap(&mut self.mode, &mut v.mode);
        std::mem::swap(&mut self.last_rgn, &mut v.last_rgn);
        std::mem::swap(&mut self.drawn, &mut v.drawn);
        std::mem::swap(&mut self.dirty_rect, &mut v.dirty_rect);
        std::mem::swap(&mut self.last, &mut v.last);
        std::mem::swap(&mut self.visible, &mut v.visible);
        std::mem::swap(&mut self.fullscreen, &mut v.fullscreen);
        std::mem::swap(&mut self.hover, &mut v.hover);
        std::mem::swap(&mut self.tracking, &mut v.tracking);
        std::mem::swap(&mut self.expanded, &mut v.expanded);
        std::mem::swap(&mut self.buttons, &mut v.buttons);
        std::mem::swap(&mut self.hot, &mut v.hot);
        std::mem::swap(&mut self.tab, &mut v.tab);
        std::mem::swap(&mut self.tab_w, &mut v.tab_w);
        std::mem::swap(&mut self.dragging, &mut v.dragging);
        std::mem::swap(&mut self.track, &mut v.track);
        std::mem::swap(&mut self.sys_on, &mut v.sys_on);
        std::mem::swap(&mut self.mon_rect, &mut v.mon_rect);
        std::mem::swap(&mut self.wild_rect, &mut v.wild_rect);
    }

    /// Run `f` as the island whose window is `hwnd` (the main one, or one on another screen).
    fn with_view<T>(&mut self, hwnd: HWND, f: impl FnOnce(&mut App) -> T) -> T {
        let Some(i) = self.views.iter().position(|v| v.hwnd == hwnd) else { return f(self) };
        let mut v = self.views.remove(i);
        self.swap_view(&mut v);
        let out = f(self);
        self.swap_view(&mut v);
        self.views.insert(i, v);
        out
    }

    fn each_view<T>(&mut self, mut f: impl FnMut(&mut App) -> T) -> Vec<T> {
        let mut out = vec![f(self)];
        for i in 0..self.views.len() {
            let mut v = self.views.remove(i);
            self.swap_view(&mut v);
            out.push(f(self));
            self.swap_view(&mut v);
            self.views.insert(i, v);
        }
        out
    }

    fn any_needs_frames(&mut self) -> bool {
        self.each_view(|a| a.needs_frames()).into_iter().any(|b| b)
    }

    /// Draw every island that needs it; how long to wait before the next frame.
    fn frame_all(&mut self) -> u32 {
        // Something shared changed: every island redraws once.
        let dirty = self.dirty;
        let waits = self.each_view(|a| {
            a.dirty = dirty;
            if a.needs_frames() {
                a.frame();
                Some(a.frame_wait_ms())
            } else {
                None
            }
        });

        self.dirty = false;
        if self.device_lost {
            self.device_lost = false;
            self.recreate_renderer();
            self.sync_views();
        }
        waits.into_iter().flatten().min().unwrap_or(0)
    }

    /// Re-pick the accent if the wallpaper changed (cheap: one file stat).
    fn check_wallpaper(&mut self) {
        if !self.cfg.accent_from_wallpaper {
            self.wall_key = None;
            self.wall_accent = None;
            self.apply_accent();
            return;
        }
        let key = wallpaper::current();
        if key == self.wall_key {
            return;
        }
        self.wall_key = key.clone();
        match key {
            Some(k) => wallpaper::pick_async(k),
            None => {
                self.wall_accent = None;
                self.apply_accent();
            }
        }
    }

    fn apply_accent(&mut self) {
        let accent = self.wall_accent.unwrap_or(self.theme.warn);
        if accent != self.theme.accent {
            util::log(&format!("accent {:.2} {:.2} {:.2}", accent.r, accent.g, accent.b));
            self.theme.accent = accent;
            self.dirty = true;
        }
    }

    fn reload_theme(&mut self) {
        self.theme = Theme::load();
        self.cfg = Config::load();
        self.wall_key = None;
        self.check_wallpaper();
        self.apply_accent();
        let _ = self.usage_tx.send(UsageCmd::Config(self.cfg.usage()));
        HOOK_ON.store(self.cfg.hijack_volume_keys && self.audio.is_some(), Ordering::SeqCst);
        self.r.set_fonts(&self.theme.font, &self.theme.icon_font);
        self.reposition();
    }

    fn recreate_renderer(&mut self) {
        let p = placement(&self.theme, &self.cfg);
        match Renderer::new(self.hwnd, p.pw as u32, p.ph as u32, p.scale, &self.theme.font, &self.theme.icon_font, self.cfg.gpu_rendering) {
            Ok(r) => {
                self.r = r;
                self.art_bmp = self.art.as_ref().and_then(|a| self.r.bitmap(a));
                self.card_bmp = self.card.as_ref().and_then(|c| c.icon.as_ref()).and_then(|a| self.r.bitmap(a));
                self.strips.clear();
                self.drawn = [R::new(0.0, 0.0, WIN_W, WIN_H); 2];
            }
            Err(e) => util::log(&format!("renderer recreate failed: {e}")),
        }
    }

    // ---------- frame ----------

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().clamp(0.0, 1.0 / 20.0);
        self.last = now;
        let (mode, tw, th, tr) = self.target();
        if mode != self.mode {
            if mode == Mode::Expanded {
                // Open with the tabs already at their final size.
                for (i, tab) in TABS.iter().enumerate() {
                    let tw = self.tab_target(*tab);
                    self.tab_w[i].snap(tw);
                }
            }
            self.mode = mode;
            self.alpha.snap(0.0);
            self.buttons.clear();
            self.hot = None;
        }
        self.alpha.set(1.0);
        self.w.set(tw);
        self.h.set(th);
        self.rad.set(tr);
        let was_moving = !self.settled();
        for s in [&mut self.w, &mut self.h, &mut self.rad, &mut self.alpha] {
            s.step(dt);
        }
        // The surface colour, the cursor light and the music level ease to their targets.
        let mood = self.mood_target();
        for (s, v) in self.mood.iter_mut().zip([mood.r, mood.g, mood.b]) {
            s.set(v);
            s.step(dt);
        }
        self.glow.set(if self.hover { 1.0 } else { 0.0 });
        self.glow.step(dt);
        let level = if self.tide() && self.cfg.animate_equalizer {
            if util::demo() { crate::demo::level(self.t0.elapsed().as_secs_f32()) } else { self.audio.as_ref().map(|a| a.peak()).unwrap_or(0.0) }.sqrt()
        } else {
            0.0
        };
        self.peak.set(level);
        self.peak.step(dt);
        // A beat: the level jumps well above where it has been lately.
        if level > 0.0 {
            self.beat_slow += (level - self.beat_slow) * (dt * 3.0).min(1.0);
            if level > self.beat_slow * 1.22 + 0.04 && level > 0.22 && self.last_beat.elapsed() > Duration::from_millis(240) {
                self.beat = 1.0;
                self.last_beat = now;
            }
        } else {
            self.beat_slow = 0.0;
        }
        self.beat *= (-dt * 7.0).exp();
        if self.beat < 0.01 {
            self.beat = 0.0;
        }
        self.swipe.step(dt);
        self.ripples.retain(|r| r.2.elapsed() < RIPPLE);
        if self.hop.is_some_and(|h| h.elapsed() >= HOP) {
            self.hop = None;
        }
        self.step_pokemon();
        if self.mode == Mode::Expanded {
            for (i, tab) in TABS.iter().enumerate() {
                let tw = self.tab_target(*tab);
                self.tab_w[i].set(tw);
                self.tab_w[i].step(dt);
            }
        }
        let want_sys = self.visible && self.mode == Mode::Expanded && self.tab == Tab::System;
        if want_sys != self.sys_on {
            self.sys_on = want_sys;
            sysinfo::set_active(want_sys);
        }
        self.update_region();
        self.draw(now);
        let _ = was_moving;
        if let Err(e) = self.r.end(self.dirty_rect) {
            util::log(&format!("present failed ({e}); recreating renderer"));
            self.device_lost = true;
        }
        self.dirty = false;
    }

    /// Clicks and hover only count on the island itself (a bit outside, for its soft edge).
    fn update_region(&mut self) {
        let s = self.scale;
        let (body, rad, sh) = self.body();
        let key = ((body.x0 * s) as i32, (body.x1 * s) as i32, (body.y1 * s).ceil() as i32, (rad * s) as i32);
        if key != self.last_rgn {
            self.last_rgn = key;
            let attach = self.cfg.attach_to_top;
            let region = |s: f32| unsafe {
                if attach {
                    let grown = R::new(body.x0 - 1.5, body.y0, body.x1 + 1.5, body.y1 + 1.5);
                    let pts: Vec<POINT> = crate::render::island_outline(grown, rad + 1.5, sh)
                        .into_iter()
                        .map(|(x, y)| POINT { x: (x * s).round() as i32, y: (y * s).round().max(0.0) as i32 })
                        .collect();
                    CreatePolygonRgn(&pts, WINDING)
                } else {
                    let rr = (rad * 2.0 * s).round() as i32;
                    CreateRoundRectRgn((body.x0 * s).floor() as i32 - 1, 0, (body.x1 * s).ceil() as i32 + 2, (body.y1 * s).ceil() as i32 + 2, rr, rr)
                }
            };
            unsafe {
                SetWindowRgn(self.hwnd, Some(region(s)), false);
            }
        }
    }

    fn draw(&mut self, now: Instant) {
        let t = (now - self.t0).as_secs_f32();
        let th = self.theme.clone();
        let cx = WIN_W / 2.0;
        // `p` is where the content goes; `body` is the whole surface (from the screen's edge).
        let p = R::new(cx - self.w.v / 2.0, self.neck, cx + self.w.v / 2.0, self.neck + self.h.v);
        let (body, rad, sh) = self.body();
        let (_, tw, thh, _) = self.target_cached();
        let closeness = 1.0 - (((self.w.v - tw).abs() + (self.h.v - thh).abs()) / 70.0).min(1.0);
        let a = self.alpha.v * closeness;

        // Only clear/redraw around the island (plus whatever the last two frames covered).
        // When nothing changed but the mascot moving, only around the mascot: it animates all
        // the time, and redrawing the whole pill for it would cost far more.
        let mascot_only = !self.dirty
            && self.settled()
            && self.mode == Mode::Compact
            && matches!(self.compact(), Compact::Usage)
            && self.island_weather().is_none()
            && self.wild.is_none()
            && self.hatch.is_none()
            && self.hearts.is_empty()
            && self.xp_pops.is_empty();
        let cur = if mascot_only {
            R::new(p.x0 + 1.0, 0.0, (p.x0 + 12.0 + self.mascot_w(self.pip_mood()) + 12.0).min(p.x1), p.y1 + 1.0)
        } else {
            R::new((body.x0 - sh - 3.0).max(0.0), 0.0, (body.x1 + sh + 3.0).min(WIN_W), (body.y1 + 3.0).min(WIN_H))
        };
        self.dirty_rect = cur.union(&self.drawn[0]).union(&self.drawn[1]);
        self.drawn = [cur, self.drawn[0]];
        self.r.begin(self.dirty_rect);
        let Some(shape) = self.r.island(body, rad, sh, self.cfg.attach_to_top) else { return };

        // Deep water: dark at the edge, taking on the mood's colour towards the lip.
        let mood = self.mood();
        let alert = self.mode == Mode::Decision || (self.mode == Mode::Card && self.card.as_ref().map(|c| c.sticky).unwrap_or(false));
        let pulse = if alert { 0.5 + 0.5 * (t * 3.0).sin() } else { 0.0 };
        let light = mood.mix(Rgba::rgb(255, 255, 255), 0.35);
        self.r.fill_vertical(&shape, body.y0, body.y1, th.bg.mix(mood, 0.05), th.bg.mix(mood, 0.2 + 0.12 * pulse));

        self.r.push_shape(&shape, 1.0);
        // A soft light follows the cursor across the surface.
        if self.glow.v > 0.01 {
            let (mx, my) = self.mouse;
            self.r.glow(mx, my, 130.0, light.a(0.13 * self.glow.v));
        }
        // The weather outside, falling and drifting across the pill.
        if self.mode == Mode::Compact {
            if let Some(wx) = self.island_weather() {
                self.draw_weather(body, t, wx, a);
            }
        }
        // The tide: two waves along the lip, moving with the music's loudness.
        if self.tide() {
            let tint = self.tint.unwrap_or(th.accent).mix(Rgba::rgb(255, 255, 255), 0.15);
            let lvl = self.peak.v;
            let still = if self.cfg.animate_equalizer { 1.0 } else { 0.0 };
            let (wx0, wx1, wy) = (body.x0 + rad * 0.8, body.x1 - rad * 0.8, body.y1 - 3.2);
            self.r.wave(wx0, wx1, wy, 0.6 + 3.0 * lvl, 41.0, t * 2.3 * still, tint.a(0.5 + 0.3 * lvl), 1.4);
            self.r.wave(wx0, wx1, wy + 0.6, 0.5 + 2.0 * lvl, 26.0, 1.3 - t * 1.6 * still, tint.a(0.25 + 0.2 * lvl), 1.1);
            // Each beat swells a glow up from the lip.
            if self.beat > 0.0 {
                self.r.glow(cx, body.y1, 70.0 + 50.0 * self.beat, tint.a(0.2 * self.beat));
            }
        }
        if a > 0.01 {
            // Fading content needs its own layer; fully shown, the shape mask already clips it.
            let fading = a < 0.999;
            if fading {
                self.r.push_shape(&shape, a);
            }
            match self.mode {
                Mode::Compact => self.draw_compact(p, t),
                Mode::Osd => self.draw_osd(p),
                Mode::Card => self.draw_card(p),
                Mode::Decision => self.draw_decision(p),
                Mode::Expanded => self.draw_expanded(p, t),
            }
            if fading {
                self.r.pop();
            }
        }
        if self.mode == Mode::Compact && a > 0.01 {
            self.draw_wild(p, t);
        } else {
            self.wild_rect = None;
        }
        // Something is using the camera (green) or the microphone (orange).
        if !self.cam_apps.is_empty() || !self.mic_apps.is_empty() {
            let c = if self.cam_apps.is_empty() { MIC_ORANGE } else { CAM_GREEN };
            let (dx, dy) = if self.mode == Mode::Compact { (p.x1 - 5.0, p.cy()) } else { (p.x1 - 10.0, p.y0 + 9.0) };
            self.r.glow(dx, dy, 10.0, c.a(0.4));
            self.r.circle(dx, dy, 3.0, c);
        }
        // Click ripples spread from the cursor and fade.
        for (x, y, at) in self.ripples.clone() {
            let u = (at.elapsed().as_secs_f32() / RIPPLE.as_secs_f32()).min(1.0);
            let e = 1.0 - (1.0 - u).powi(3);
            self.r.glow(x, y, 12.0 + 44.0 * e, light.a(0.12 * (1.0 - u)));
            self.r.ring(x, y, 4.0 + 40.0 * e, light.a(0.4 * (1.0 - u)), 1.4);
        }
        self.r.pop();

        // The rim: invisible at the screen's edge, catching the light along the lip.
        let rim = th.border.mix(mood, 0.5 + 0.35 * pulse).mix(light, 0.6 * self.beat);
        let rim_top = if self.cfg.attach_to_top { rim.a(0.0) } else { rim.a(0.55) };
        self.r.stroke_vertical(&shape, body.y0, body.y1, rim_top, rim.a(0.9), 1.0);
    }

    /// The target size without re-measuring text (used for the fade-in amount).
    fn target_cached(&self) -> (Mode, f32, f32, f32) {
        (self.mode, self.w.t, self.h.t, self.rad.t)
    }

    fn spinner(&self, cx: f32, cy: f32, t: f32, c: Rgba) {
        for i in 0..8 {
            let ang = i as f32 / 8.0 * std::f32::consts::TAU;
            let phase = ((t * 10.0 - i as f32).rem_euclid(8.0)) / 8.0;
            self.r.circle(cx + ang.cos() * 5.5, cy + ang.sin() * 5.5, 1.35, c.a(0.25 + 0.75 * (1.0 - phase)));
        }
    }

    fn draw_compact(&mut self, p: R, t: f32) {
        let th = self.theme.clone();
        let cy = p.cy();
        self.mon_rect = None;
        if let (Some((_, _, true)), Some((_, end))) = (self.scrub, self.track_pos()) {
            let tint = self.tint.unwrap_or(th.accent);
            let frac = if end > 0.0 { (self.scrub_to / end) as f32 } else { 0.0 };
            let fmt = |s: f64| format!("{}:{:02}", (s / 60.0) as u32, (s % 60.0) as u32);
            let label = format!("{} / {}", fmt(self.scrub_to), fmt(end));
            let bar = R::new(p.x0 + 16.0, cy + 6.0, p.x1 - 16.0, cy + 9.0);
            self.r.fill_round(bar, 1.5, th.border.a(0.6));
            self.r.fill_round(R::new(bar.x0, bar.y0, bar.x0 + bar.w() * frac, bar.y1), 1.5, tint);
            self.r.circle(bar.x0 + bar.w() * frac, bar.cy(), 4.0, tint);
            self.r.text(&label, R::new(p.x0, p.y0 - 2.0, p.x1, cy + 4.0), 12.0, 600, th.text, Align::Center);
            return;
        }
        match self.compact() {
            Compact::Waiting(i) => {
                let tx = if self.cfg.mascot {
                    self.draw_pip(p.x0 + 12.0, cy, t, PipMood::Needs);
                    p.x0 + 12.0 + self.mascot_w(PipMood::Needs) + 4.0
                } else {
                    let pulse = 0.55 + 0.45 * (t * 4.0).sin().abs();
                    self.r.circle(p.x0 + 17.0, cy, 4.5, th.accent.a(pulse));
                    p.x0 + 28.0
                };
                let label = format!("{} needs you", self.agents[i].label);
                self.r.text(&label, R::new(tx, p.y0, p.x1 - 12.0, p.y1), 12.0, 600, th.text, Align::Left);
            }
            Compact::Working(i) => {
                self.spinner(p.x0 + 18.0, cy, t, th.accent);
                let (label, act) = (self.agents[i].label.clone(), self.agents[i].activity.clone());
                let lw = self.text_width(&label).min(PILL_LABEL_W);
                self.r.text(&label, R::new(p.x0 + 34.0, p.y0, (p.x0 + 34.0 + lw + 1.0).min(p.x1 - 12.0), p.y1), 12.0, 600, th.text, Align::Left);
                if !act.is_empty() {
                    let ax = p.x0 + 34.0 + lw + 8.0;
                    self.r.text(&act, R::new(ax, p.y0, p.x1 - 12.0, p.y1), 12.0, 400, th.subtext, Align::Left);
                }
            }
            Compact::Media => {
                // Swiping slides the whole song sideways (the island's edge clips it).
                let p = R::new(p.x0 + self.swipe.v, p.y0, p.x1 + self.swipe.v, p.y1);
                let tint = self.tint.unwrap_or(th.accent);
                let art = R::xywh(p.x0 + 6.0, cy - 11.0, 22.0, 22.0);
                match &self.art_bmp {
                    Some(b) => self.r.image(b, art, 11.0, 1.0),
                    None => {
                        self.r.fill_round(art, 11.0, th.bg2);
                        self.r.icon(G_MUSIC, art, 12.0, tint);
                    }
                }
                let spin = if self.media_spinner() { 20.0 } else { 0.0 };
                let mood = self.media_mood();
                let pip = self.mascot_w(mood);
                let title = self.media.as_ref().map(|m| m.title.clone()).unwrap_or_default();
                let col = if self.playing { th.text } else { th.subtext };
                self.r.text(&title, R::new(p.x0 + 36.0, p.y0, p.x1 - 26.0 - spin - pip, p.y1), 12.0, 600, col, Align::Left);
                if self.cfg.mascot {
                    self.draw_pip(p.x1 - 10.0 - spin - pip + 4.0, cy, t, mood);
                }
                let ex = p.x1 - 12.0 - spin - pip - 12.0;
                // Bars follow the music's real loudness, each at its own pace.
                let lvl = self.peak.v;
                for i in 0..3 {
                    let amp = if self.playing && self.cfg.animate_equalizer {
                        let wobble = 0.5 + 0.5 * (t * (6.0 + i as f32 * 1.3) + i as f32 * 1.7).sin();
                        (0.2 + 0.8 * lvl) * (0.45 + 0.55 * wobble)
                    } else {
                        0.15
                    };
                    let hgt = 3.0 + 9.0 * amp;
                    let x = ex + i as f32 * 4.5;
                    self.r.fill_round(R::new(x, cy - hgt / 2.0, x + 2.6, cy + hgt / 2.0), 1.3, tint);
                }
                if spin > 0.0 {
                    self.spinner(p.x1 - 20.0, cy, t, th.accent);
                }
            }
            Compact::Usage => {
                let mut x = p.x0 + 12.0;
                if self.cfg.mascot {
                    let mood = self.pip_mood();
                    self.draw_pip(x - 2.0, cy, t, mood);
                    x += self.mascot_w(mood);
                }
                if self.cfg.clock_in_pill {
                    let (date, time) = self.clock.clone();
                    let dw = self.text_width_at(&date, 11.0, 500);
                    self.r.text(&date, R::new(x, p.y0, x + dw + 1.0, p.y1), 11.0, 500, th.subtext, Align::Left);
                    x += dw + 6.0;
                    let tw = self.text_width_at(&time, 12.0, 600);
                    self.r.text(&time, R::new(x, p.y0, x + tw + 1.0, p.y1), 12.0, 600, th.text, Align::Left);
                    x += tw + 12.0;
                }
                let moon = self.moon_out();
                let full = self.special().is_some_and(|s| s.key == "fullmoon") || self.cfg.weather_effect == "fullmoon";
                let age = if full { 14.77 } else { pokemon::moon_age(crate::dex::now_unix()) };
                if let Some((glyph, temp)) = self.weather_chip() {
                    match moon {
                        Some(k) => self.draw_moon(x + 8.0, cy, age, k, true),
                        None => self.r.icon(glyph, R::new(x - 2.0, p.y0, x + 18.0, p.y1), 14.0, th.accent),
                    }
                    x += 16.0 + 4.0;
                    let tw = self.text_width_at(&temp, 12.0, 600);
                    self.r.text(&temp, R::new(x, p.y0, x + tw + 1.0, p.y1), 12.0, 600, th.text, Align::Left);
                    x += tw + 12.0;
                } else if let Some(k) = moon {
                    self.draw_moon(x + 8.0, cy, age, k, true);
                    x += 16.0 + 12.0;
                }
                for (name, pct, session, week) in self.usage_chips() {
                    // Outer ring: 5-hour window. Inner ring: weekly.
                    let rc = x + 8.0;
                    self.r.ring(rc, cy, 7.0, th.border.a(0.55), 2.0);
                    self.r.arc(rc, cy, 7.0, session, self.level_color(session), 2.0);
                    if let Some(wk) = week {
                        self.r.ring(rc, cy, 3.6, th.border.a(0.55), 1.8);
                        self.r.arc(rc, cy, 3.6, wk, self.level_color(wk), 1.8);
                    }
                    x += 16.0 + 6.0;
                    let nw = self.text_width_at(name, 11.0, 500);
                    self.r.text(name, R::new(x, p.y0, x + nw + 1.0, p.y1), 11.0, 500, th.subtext, Align::Left);
                    x += nw + 4.0;
                    let pw = self.text_width_at(&pct, 12.0, 600);
                    let worst = session.max(week.unwrap_or(0.0));
                    let col = if worst >= self.cfg.usage_watch { self.level_color(worst) } else { th.text };
                    self.r.text(&pct, R::new(x, p.y0, x + pw + 1.0, p.y1), 12.0, 600, col, Align::Left);
                    x += pw + 12.0;
                }
            }
            Compact::Idle => {
                if self.cfg.mascot {
                    let mood = self.pip_mood();
                    let w = self.mascot_w(mood);
                    self.draw_pip(p.x0 + (p.w() - w) / 2.0 + 2.0, cy, t, mood);
                } else {
                    self.r.circle(p.x0 + p.w() / 2.0, cy, 3.0, th.accent.a(0.9));
                }
            }
        }
    }

    fn draw_osd(&mut self, p: R) {
        let th = self.theme.clone();
        let Some(o) = &self.osd else { return };
        let (kind, value, muted) = (o.kind, o.value.clamp(0.0, 1.0), o.muted);
        let glyph = match kind {
            OsdKind::Brightness => G_SUN,
            OsdKind::Volume if muted || value == 0.0 => G_VOL_MUTE,
            OsdKind::Volume if value < 0.34 => G_VOL_LOW,
            OsdKind::Volume if value < 0.67 => G_VOL_MED,
            OsdKind::Volume => G_VOL_HIGH,
        };
        self.r.icon(glyph, R::new(p.x0 + 8.0, p.y0, p.x0 + 38.0, p.y1), 15.0, th.text);
        let bar = R::new(p.x0 + 42.0, p.cy() - 3.0, p.x1 - 56.0, p.cy() + 3.0);
        self.r.fill_round(bar, 3.0, th.bg2);
        let fill = if muted { th.subtext.a(0.6) } else { th.accent };
        if value > 0.0 {
            let fw = (bar.w() * value).max(6.0);
            self.r.fill_round(R::new(bar.x0, bar.y0, bar.x0 + fw, bar.y1), 3.0, fill);
        }
        let label = if muted { "Muted".to_string() } else { format!("{}%", (value * 100.0).round()) };
        self.r.text(&label, R::new(p.x1 - 54.0, p.y0, p.x1 - 12.0, p.y1), 12.0, 600, th.text, Align::Right);
    }

    fn draw_card(&mut self, p: R) {
        let th = self.theme.clone();
        let Some(c) = &self.card else { return };
        let (glyph, tint, app, title, body, kind) = (c.glyph, c.tint, c.app.clone(), c.title.clone(), c.body.clone(), c.kind.clone());
        let left = BREAK_SECS - self.card_since.elapsed().as_secs_f32();
        let body = if kind == CardKind::Break {
            if left > 0.0 {
                format!("Look at something 6 m away · {} s", left.ceil() as u32)
            } else {
                "Done, nice one".to_string()
            }
        } else {
            body
        };
        let t = self.t0.elapsed().as_secs_f32();
        // Content slides in from the left, the picture a beat ahead of the words.
        let u = (self.card_since.elapsed().as_secs_f32() / CARD_IN.as_secs_f32()).min(1.0);
        let ease = |u: f32| 1.0 - (1.0 - u.clamp(0.0, 1.0)).powi(3);
        let (e_pic, e_txt) = (ease(u * 1.3), ease(u * 1.1 - 0.12));
        let slide = |e: f32| (1.0 - e) * -22.0;

        // Charger: a fill sweeps across to the battery level, its leading edge glowing.
        if let CardKind::Power { pct, charging } = kind {
            let to = (pct as f32 / 100.0).clamp(0.03, 1.0);
            let sweep = ease(self.card_since.elapsed().as_secs_f32() / 1.1) * to;
            let fx = p.x0 + (p.w() + 2.0) * sweep;
            let fill = if charging { th.good } else { tint };
            self.r.rect(R::new(p.x0 - 2.0, p.y0 - self.neck - 2.0, fx, p.y1 + 2.0), fill.a(0.13));
            let breathe = if charging { 0.75 + 0.25 * (t * 3.0).sin() } else { 0.6 };
            self.r.glow(fx, p.cy(), 46.0, fill.a(0.22 * breathe));
            self.r.rect(R::new(fx - 1.0, p.y0 - self.neck, fx + 0.5, p.y1), fill.a(0.5 * breathe));
        }

        let ic = match kind {
            CardKind::Shot(_) => R::xywh(p.x0 + 14.0, p.y0 + 12.0, 136.0, p.h() - 24.0),
            CardKind::Song => R::xywh(p.x0 + 14.0, p.cy() - 24.0, 48.0, 48.0),
            _ => R::xywh(p.x0 + 16.0, p.cy() - 20.0, 40.0, 40.0),
        }
        .offset(slide(e_pic), 0.0);
        let pic_a = e_pic;
        let bmp = if kind == CardKind::Song { self.art_bmp.clone() } else { self.card_bmp.clone() };
        match (&bmp, &kind) {
            (Some(b), CardKind::Shot(_)) => {
                self.r.fill_round(ic, 10.0, th.bg2.a(pic_a));
                self.r.image(b, ic.inset(2.0), 8.0, pic_a);
                self.r.stroke_round(ic.inset(0.5), 9.5, th.border.a(0.8 * pic_a), 1.0);
            }
            (Some(b), CardKind::Song) => {
                // The artwork casts its own colour behind it.
                self.r.glow(ic.cx(), ic.cy(), 44.0, tint.a(0.25 * pic_a));
                self.r.image(b, ic, 10.0, pic_a);
            }
            (Some(b), _) => {
                self.r.fill_round(ic, 11.0, th.bg2.a(pic_a));
                self.r.image(b, ic.inset(4.0), 8.0, pic_a);
            }
            (None, _) => {
                self.r.fill_round(ic, 12.0, th.bg2.a(pic_a));
                self.r.icon(glyph, ic, 19.0, tint.a(pic_a));
            }
        }
        let x = ic.x1 + 12.0 - slide(e_pic) + slide(e_txt);
        let download = matches!(kind, CardKind::Download(_));
        let x1 = p.x1 - if kind == CardKind::Song { 44.0 } else if download { 86.0 } else { 18.0 };
        if download {
            self.buttons.clear();
            let a = e_txt.max(0.0);
            for (k, (g, b)) in [(G_FOLDER, Btn::CardFolder), (G_SHELF, Btn::CardShelf)].into_iter().enumerate() {
                let r = R::xywh(p.x1 - 78.0 + k as f32 * 36.0, p.cy() - 15.0, 30.0, 30.0);
                let hot = self.hot == Some(b);
                self.r.fill_round(r, 15.0, if hot { th.hover.a(a) } else { th.bg2.a(0.7 * a) });
                self.r.icon(g, r, 14.0, th.accent.a(a));
                self.buttons.push((r, b));
            }
        }
        let cy = p.cy();
        let ta = e_txt.max(0.0);
        self.r.text(&app, R::new(x, cy - 26.0, x1, cy - 12.0), 11.0, 500, th.subtext.a(ta), Align::Left);
        self.r.text(&title, R::new(x, cy - 12.0, x1, cy + 7.0), 13.5, 600, th.text.a(ta), Align::Left);
        self.r.text(&body, R::new(x, cy + 7.0, x1, cy + 25.0), 12.0, 400, th.subtext.a(ta), Align::Left);
        if kind == CardKind::Break {
            let f = (left / BREAK_SECS).clamp(0.0, 1.0);
            let bar = R::new(x, cy + 27.0, x1, cy + 30.0);
            self.r.fill_round(bar, 1.5, th.border.a(0.5 * ta));
            self.r.fill_round(R::new(bar.x0, bar.y0, bar.x0 + bar.w() * f, bar.y1), 1.5, tint.a(ta));
        }
        // A new song: the equaliser plays along on the right.
        if kind == CardKind::Song {
            let lvl = self.peak.v;
            let ex = p.x1 - 34.0;
            for i in 0..4 {
                let wobble = 0.5 + 0.5 * (t * (6.0 + i as f32 * 1.4) + i as f32 * 1.9).sin();
                let amp = if self.playing { (0.2 + 0.8 * lvl) * (0.4 + 0.6 * wobble) } else { 0.15 };
                let hgt = 4.0 + 14.0 * amp;
                let bx = ex + i as f32 * 5.0;
                self.r.fill_round(R::new(bx, cy - hgt / 2.0, bx + 3.0, cy + hgt / 2.0), 1.5, tint.a(ta));
            }
        }
    }

    fn question_geometry(&mut self, question: &str, detail: &str, choices: &[crate::ask::Choice]) -> (f32, f32, Vec<f32>, f32) {
        let width = DEC_W - 36.0;
        let question_h = self.r.wrapped_height(question, width, 14.0, 600).max(22.0) + 4.0;
        let detail_h = if detail.is_empty() { 0.0 } else { self.r.wrapped_height(detail, width - 20.0, 12.0, 400) + 24.0 };
        let buttons: Vec<f32> = choices.iter().map(|choice| self.r.wrapped_height(&choice.label, width - 24.0, 12.5, 600).max(22.0) + 16.0).collect();
        let height = 32.0 + question_h + 8.0 + detail_h + buttons.iter().map(|h| h + 6.0).sum::<f32>() + 30.0 + 16.0;
        (question_h, detail_h, buttons, height)
    }

    fn draw_question(&mut self, p: R) {
        let Some(d) = self.decisions.first() else { return };
        let (label, question, detail, choices) = (d.label.clone(), d.question.clone(), d.detail.clone(), d.choices.clone());
        let (multi, picked) = (d.multi, d.picked.clone());
        let left = d.deadline.saturating_duration_since(Instant::now());
        let count = self.decisions.len();
        let (qh, dh, button_heights, _) = self.question_geometry(&question, &detail, &choices);
        let th = self.theme.clone();
        self.buttons.clear();
        let x0 = p.x0 + 18.0;
        let x1 = p.x1 - 18.0;
        let mut y = p.y0 + 16.0;
        self.r.icon(G_ROBOT, R::xywh(x0 - 3.0, y - 1.0, 20.0, 18.0), 13.0, th.accent);
        self.r.text(&label, R::new(x0 + 20.0, y, x1 - 100.0, y + 16.0), 11.5, 500, th.subtext, Align::Left);
        let right = if count > 1 { format!("1 of {count} · {}", countdown(left)) } else { countdown(left) };
        self.r.text(&right, R::new(x1 - 100.0, y, x1, y + 16.0), 11.0, 400, th.subtext, Align::Right);
        y += 16.0;
        self.r.text_wrapped(&question, R::new(x0, y, x1, y + qh), 14.0, 600, th.text);
        y += qh + 8.0;
        if dh > 0.0 {
            let r = R::new(x0, y, x1, y + dh - 10.0);
            self.r.fill_round(r, 10.0, th.bg2.a(0.8));
            self.r.text_wrapped(&detail, R::new(r.x0 + 10.0, r.y0 + 6.0, r.x1 - 10.0, r.y1 - 4.0), 12.0, 400, th.text);
            y += dh;
        }
        for (index, (choice, height)) in choices.iter().zip(button_heights).enumerate() {
            let r = R::new(x0, y, x1, y + height);
            let hot = self.hot == Some(Btn::Decide(index));
            let on = multi && picked.get(index).copied().unwrap_or(false);
            let fill = if on { th.accent.mix(th.bg2, 0.78) } else if hot { th.hover } else { th.bg2 };
            self.r.fill_round(r, 10.0, fill);
            self.r.stroke_round(r.inset(0.5), 9.5, if hot || on { th.accent } else { th.border }, 1.0);
            // Pick-several: a check box on the right of each choice.
            let text_x1 = if multi {
                let b = R::xywh(x1 - 30.0, y + height / 2.0 - 8.0, 16.0, 16.0);
                if on {
                    self.r.fill_round(b, 5.0, th.accent);
                    self.r.icon(G_CHECK, b, 11.0, th.bg);
                } else {
                    self.r.stroke_round(b.inset(0.75), 4.5, th.subtext.a(0.7), 1.5);
                }
                x1 - 40.0
            } else {
                x1 - 12.0
            };
            self.r.text_wrapped(&choice.label, R::new(x0 + 12.0, y + 8.0, text_x1, y + height - 4.0), 12.5, 600, th.text);
            self.buttons.push((r, Btn::Decide(index)));
            y += height + 6.0;
        }
        let n = choices.len();
        if multi {
            // "Open chat instead" on the left, "Done" on the right (lit once something's picked).
            let half = (x1 - x0 - 8.0) / 2.0;
            let chat = R::xywh(x0, y, half, 30.0);
            self.r.fill_round(chat, 10.0, if self.hot == Some(Btn::Decide(n)) { th.hover } else { th.bg2.a(0.0) });
            self.r.text("Open chat instead", chat, 12.0, 500, th.subtext, Align::Center);
            self.buttons.push((chat, Btn::Decide(n)));
            let done = R::xywh(x0 + half + 8.0, y, half, 30.0);
            let count = picked.iter().filter(|p| **p).count();
            if count > 0 {
                let hot = self.hot == Some(Btn::Decide(n + 1));
                self.r.fill_round(done, 10.0, if hot { th.accent.mix(th.text, 0.15) } else { th.accent });
                self.r.text(&format!("Done · {count}"), done, 12.5, 600, th.bg, Align::Center);
            } else {
                self.r.stroke_round(done.inset(0.5), 9.5, th.border, 1.0);
                self.r.text("Pick one or more", done, 12.0, 500, th.subtext.a(0.7), Align::Center);
            }
            self.buttons.push((done, Btn::Decide(n + 1)));
        } else {
            let r = R::xywh(x0, y, x1 - x0, 30.0);
            self.r.fill_round(r, 10.0, if self.hot == Some(Btn::Decide(n)) { th.hover } else { th.bg2.a(0.0) });
            self.r.text("Open chat instead", r, 12.0, 500, th.subtext, Align::Center);
            self.buttons.push((r, Btn::Decide(n)));
        }
    }

    /// Lines the prompt's detail (usually the command) takes, at most 3.
    fn decision_lines(&mut self) -> f32 {
        let Some(d) = self.decisions.first() else { return 0.0 };
        if d.detail.is_empty() {
            return 0.0;
        }
        let detail = d.detail.clone();
        let w = self.text_width_at(&detail, 12.0, 400);
        (w / (DEC_W - 2.0 * 18.0 - 2.0 * 10.0 - 30.0)).ceil().clamp(1.0, 3.0)
    }

    fn decision_height(&mut self) -> f32 {
        if let Some(d) = self.decisions.first().filter(|d| !d.choices.is_empty()) {
            let (question, detail, choices) = (d.question.clone(), d.detail.clone(), d.choices.clone());
            return self.question_geometry(&question, &detail, &choices).3;
        }
        let lines = self.decision_lines();
        let detail = if lines > 0.0 { lines * 16.0 + 14.0 + 10.0 } else { 0.0 };
        16.0 + 16.0 + 22.0 + 8.0 + detail + 30.0 + 16.0
    }

    fn draw_decision(&mut self, p: R) {
        if self.decisions.first().map(|d| !d.choices.is_empty()).unwrap_or(false) {
            self.draw_question(p);
            return;
        }
        let th = self.theme.clone();
        self.buttons.clear();
        let lines = self.decision_lines();
        let count = self.decisions.len();
        let Some(d) = self.decisions.first() else { return };
        let (label, question, detail) = (d.label.clone(), d.question.clone(), d.detail.clone());
        let left = d.deadline.saturating_duration_since(Instant::now());
        let total = d.deadline.saturating_duration_since(d.since).as_secs_f32().max(1.0);

        let x0 = p.x0 + 18.0;
        let x1 = p.x1 - 18.0;
        let mut y = p.y0 + 16.0;

        // Header: who's asking, and how long until Claude asks in its own window instead.
        self.r.icon(G_ROBOT, R::xywh(x0 - 3.0, y - 1.0, 20.0, 18.0), 13.0, th.accent);
        self.r.text(&label, R::new(x0 + 20.0, y, x1 - 90.0, y + 16.0), 11.5, 500, th.subtext, Align::Left);
        let mut right = countdown(left);
        if count > 1 {
            right = format!("1 of {count} · {right}");
        }
        self.r.text(&right, R::new(x1 - 90.0, y, x1, y + 16.0), 11.0, 400, th.subtext, Align::Right);
        y += 16.0;
        self.r.text(&question, R::new(x0, y, x1, y + 22.0), 14.0, 600, th.text, Align::Left);
        y += 22.0 + 8.0;

        if lines > 0.0 {
            let box_h = lines * 16.0 + 14.0;
            let b = R::new(x0, y, x1, y + box_h);
            self.r.fill_round(b, 10.0, th.bg2.a(0.8));
            self.r.text_wrapped(&detail, R::new(b.x0 + 10.0, b.y0 + 6.0, b.x1 - 10.0, b.y1 - 4.0), 12.0, 400, th.text);
            y += box_h + 10.0;
        }

        // Buttons: "Ask in Claude" / "Ask in Codex" on the left, Deny / Allow on the right.
        let bh = 30.0;
        let ask = if self.decisions[0].key.starts_with("codex:") { "Ask in Codex" } else { "Ask in Claude" };
        let aw = self.text_width_at(ask, 12.0, 600) + 26.0;
        let buttons = [
            (R::xywh(x0, y, aw, bh), 2usize, ask),
            (R::xywh(x1 - 90.0 - 8.0 - 90.0, y, 90.0, bh), 1, "Deny"),
            (R::xywh(x1 - 90.0, y, 90.0, bh), 0, "Allow"),
        ];
        for (r, which, text) in buttons {
            let hot = self.hot == Some(Btn::Decide(which));
            let (fill, col) = match which {
                0 => (if hot { th.accent.mix(th.text, 0.18) } else { th.accent }, th.bg),
                1 => (if hot { th.bad.a(0.3) } else { th.bg2 }, if hot { th.bad } else { th.text }),
                _ => (if hot { th.hover } else { th.bg2.a(0.0) }, th.subtext),
            };
            self.r.fill_round(r, bh / 2.0, fill);
            if which == 2 {
                self.r.stroke_round(r.inset(0.5), bh / 2.0 - 0.5, th.border, 1.0);
            }
            self.r.text(text, r, 12.0, 600, col, Align::Center);
            self.buttons.push((r, Btn::Decide(which)));
        }

        // Time left, as a hairline along the bottom.
        let frac = (left.as_secs_f32() / total).clamp(0.0, 1.0);
        let line = R::new(p.x0 + 22.0, p.y1 - 4.0, p.x1 - 22.0, p.y1 - 2.5);
        self.r.fill_round(R::new(line.x0, line.y0, line.x0 + line.w() * frac, line.y1), 0.75, th.accent.a(0.5));
    }

    fn draw_expanded(&mut self, p: R, _t: f32) {
        let th = self.theme.clone();
        let x0 = p.x0 + 14.0;
        let x1 = p.x1 - 14.0;
        let mut y = p.y0 + 12.0;
        self.buttons.clear();

        // Tab bar: round buttons like YASB's widgets; the selected one shows its name.
        let right_w: f32 = self.tab_w[RIGHT_TABS..].iter().map(|s| s.v.max(TAB_H) + 6.0).sum::<f32>() - 6.0;
        let mut x = x0;
        let mut left_end = x0;
        for (i, tab) in TABS.iter().copied().enumerate() {
            if i == RIGHT_TABS {
                left_end = x;
                x = x1 - right_w;
            }
            let w = self.tab_w[i].v.max(TAB_H);
            let r = R::xywh(x, y, w, TAB_H);
            let sel = tab == self.tab;
            let hot = self.hot == Some(Btn::Tab(tab));
            let fill = if sel || hot { th.hover } else { th.bg2 };
            self.r.fill_round(r, TAB_H / 2.0, fill);
            self.r.stroke_round(r.inset(0.5), TAB_H / 2.0 - 0.5, if sel { th.accent.a(0.55) } else { th.border }, 1.0);
            let k = ((w - TAB_H) / 40.0).clamp(0.0, 1.0);
            let ic = x + TAB_H / 2.0 + 3.0 * k;
            if tab == Tab::Dex {
                // No Poké Ball in the icon font: the island's own pixel one.
                self.draw_ball(ic, y + TAB_H / 2.0, 1.0);
            } else {
                self.r.icon(tab.glyph(), R::new(ic - 12.0, y, ic + 12.0, y + TAB_H), 14.0, th.accent);
            }
            if w > TAB_H + 6.0 {
                self.r.text(tab.label(), R::new(x + 30.0, y, x + w - 10.0, y + TAB_H), 12.0, 600, th.text.a(k), Align::Left);
            }
            if let Some(c) = self.tab_badge(tab) {
                self.r.circle(r.x1 - 5.0, y + 5.0, 4.0, th.bg);
                self.r.circle(r.x1 - 5.0, y + 5.0, 2.8, c);
            }
            self.buttons.push((r, Btn::Tab(tab)));
            x += w + 6.0;
        }
        // Do not disturb, on every tab, just left of the right-hand tabs: lit up while it's
        // on (it also pauses wild Pokémon, eggs and the keeper's news).
        {
            let on = self.ctl_dnd == Some(true);
            let bx = x1 - right_w - 12.0 - TAB_H;
            if bx > left_end + 2.0 {
                let r = R::xywh(bx, y, TAB_H, TAB_H);
                let hot = self.hot == Some(Btn::DndBar);
                let fill = if on { th.accent.a(if hot { 0.38 } else { 0.28 }) } else if hot { th.hover } else { th.bg2 };
                self.r.fill_round(r, TAB_H / 2.0, fill);
                self.r.stroke_round(r.inset(0.5), TAB_H / 2.0 - 0.5, if on { th.accent.a(0.7) } else { th.border }, 1.0);
                let ic = if on { th.accent } else { th.subtext };
                self.r.icon(Quick::Dnd.glyph(), r, 14.0, ic);
                self.buttons.push((r, Btn::DndBar));
            }
        }
        y += TAB_H + 12.0;

        match self.tab {
            Tab::Media => self.draw_media_tab(x0, x1, y),
            Tab::Usage => self.draw_usage_tab(x0, x1, y),
            Tab::Calendar => self.draw_calendar_tab(x0, x1, y),
            Tab::System => self.draw_system_tab(x0, x1, y),
            Tab::Weather => self.draw_weather_tab(x0, x1, y),
            Tab::Shelf => self.draw_shelf_tab(x0, x1, y),
            Tab::Dex => self.draw_dex_tab(x0, x1, y),
            Tab::Home => self.draw_home_tab(x0, x1, y),
            Tab::Controls => self.draw_controls_tab(x0, x1, y),
            Tab::Power => self.draw_power_tab(x0, x1, y),
            Tab::Game => self.draw_game_tab(x0, x1, y),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stat_tile(&mut self, r: R, glyph: &str, label: &str, value: &str, sub: &str, hist: Option<Vec<f32>>, bar: Option<f32>, col: Rgba) {
        let th = self.theme.clone();
        self.r.fill_round(r, 12.0, th.bg2.a(0.6));
        self.r.icon(glyph, R::xywh(r.x0 + 6.0, r.y0 + 5.0, 22.0, 22.0), 13.0, th.accent);
        self.r.text(label, R::new(r.x0 + 30.0, r.y0 + 5.0, r.x1 - 88.0, r.y0 + 27.0), 11.5, 600, th.subtext, Align::Left);
        self.r.text(value, R::new(r.x1 - 110.0, r.y0 + 4.0, r.x1 - 10.0, r.y0 + 28.0), 14.0, 600, th.text, Align::Right);
        let bottom = if sub.is_empty() { r.y1 - 8.0 } else { r.y1 - 19.0 };
        if let Some(h) = hist {
            self.r.sparkline(R::new(r.x0 + 10.0, r.y0 + 30.0, r.x1 - 10.0, bottom), &h, 60, col);
        }
        if let Some(f) = bar {
            let b = R::new(r.x0 + 10.0, r.y0 + 34.0, r.x1 - 10.0, r.y0 + 39.0);
            self.r.fill_round(b, 2.5, th.bg2);
            self.r.fill_round(R::new(b.x0, b.y0, b.x0 + (b.w() * f.clamp(0.0, 1.0)).max(5.0), b.y1), 2.5, col);
        }
        if !sub.is_empty() {
            self.r.text(sub, R::new(r.x0 + 10.0, r.y1 - 19.0, r.x1 - 10.0, r.y1 - 4.0), 10.5, 400, th.subtext, Align::Left);
        }
    }

    fn draw_system_tab(&mut self, x0: f32, x1: f32, y: f32) {
        let Some(s) = self.sys.clone() else {
            return self.empty_row(G_CPU, "Reading system stats…", x0, x1, y);
        };
        match self.sys_page {
            1 => return self.draw_cpu_page(&s, x0, x1, y),
            2 => return self.draw_mem_page(&s, x0, x1, y),
            3 => return self.draw_gpu_page(&s, x0, x1, y),
            4 => return self.draw_storage_page(&s, x0, x1, y),
            _ => {}
        }
        let gb = |b: u64| b as f64 / 1_073_741_824.0;
        let gap = 8.0;
        let cw = (x1 - x0 - gap) / 2.0;
        let h = 64.0;
        let pct = |f: f32| format!("{}%", (f * 100.0).round());

        let hist: Vec<f32> = self.cpu_hist.iter().copied().collect();
        let cpu_name = if s.cpu_info.name.is_empty() { "CPU".to_string() } else { s.cpu_info.name.clone() };
        self.stat_tile(R::xywh(x0, y, cw, h), G_CPU, &cpu_name, &pct(s.cpu), "", Some(hist), None, self.load_color(s.cpu));

        let mf = if s.mem_total > 0 { s.mem_used as f32 / s.mem_total as f32 } else { 0.0 };
        let hist: Vec<f32> = self.mem_hist.iter().copied().collect();
        let sub = format!("{:.1} / {:.1} GB", gb(s.mem_used), gb(s.mem_total));
        let ram = s.mem_info.name();
        let ram = if ram.is_empty() { "Memory".to_string() } else { ram };
        self.stat_tile(R::xywh(x0 + cw + gap, y, cw, h), G_MEMORY, &ram, &pct(mf), &sub, Some(hist), None, self.load_color(mf));

        let y2 = y + h + gap;
        match &s.gpu {
            Some(g) => {
                let hist: Vec<f32> = self.gpu_hist.iter().copied().collect();
                let mut sub = format!("VRAM {:.1} / {:.0} GB", gb(g.vram_used), gb(g.vram_total));
                if let Some(t) = g.temp {
                    sub.push_str(&format!(" · {t}°C"));
                }
                self.stat_tile(R::xywh(x0, y2, cw, h), G_GPU, &g.name, &pct(g.util), &sub, Some(hist), None, self.load_color(g.util));
            }
            None => self.stat_tile(R::xywh(x0, y2, cw, h), G_GPU, "GPU", "–", "No NVIDIA GPU stats", None, None, self.theme.accent),
        }

        let df = if s.disk_total > 0 { 1.0 - s.disk_free as f32 / s.disk_total as f32 } else { 0.0 };
        let value = format!("{:.0} GB free", gb(s.disk_free));
        let mut sub = format!("{:.0} GB used of {:.0} GB", gb(s.disk_total - s.disk_free), gb(s.disk_total));
        // "Samsung NVMe SSD (C:)" on top, the exact model under the bar.
        let disk = match s.drives.iter().find(|d| d.root.eq_ignore_ascii_case("C:")) {
            Some(d) if !d.model.is_empty() => {
                let (brand, rest) = d.model.split_once(' ').unwrap_or((d.model.as_str(), ""));
                let kind = if d.kind.is_empty() { "disk" } else { d.kind.as_str() };
                if !rest.is_empty() {
                    sub.push_str(&format!(" · {rest}"));
                }
                format!("{brand} {kind} (C:)")
            }
            _ => "Disk C:".to_string(),
        };
        self.stat_tile(R::xywh(x0 + cw + gap, y2, cw, h), G_DISK, &disk, &value, &sub, None, Some(df), self.load_color(df));
        // Each tile opens its page.
        for (k, r) in [R::xywh(x0, y, cw, h), R::xywh(x0 + cw + gap, y, cw, h), R::xywh(x0, y2, cw, h), R::xywh(x0 + cw + gap, y2, cw, h)].into_iter().enumerate() {
            let page = [1u8, 2, 3, 4][k];
            if self.hot == Some(Btn::SysPage(page)) {
                self.r.stroke_round(r.inset(0.5), 11.5, self.theme.accent.a(0.7), 1.0);
                self.r.text("›", R::new(r.x1 - 18.0, r.y1 - 20.0, r.x1 - 6.0, r.y1 - 4.0), 13.0, 700, self.theme.accent, Align::Right);
            }
            self.buttons.push((r, Btn::SysPage(page)));
        }

        // The network: what it's on, any VPN, speed and ping.
        let th = self.theme.clone();
        let y3 = y2 + h + gap;
        self.r.fill_round(R::new(x0, y3, x1, y3 + 34.0), 12.0, th.bg2.a(0.55));
        let net = self.net.clone().unwrap_or_default();
        let (g, mut label) = match (&net.wifi, net.wired) {
            (Some(n), _) => (G_WIFI, if n.is_empty() { "Wi-Fi".to_string() } else { n.clone() }),
            (None, true) => (G_ETHERNET, "Ethernet".to_string()),
            _ => (G_WIFI_OFF, "Offline".to_string()),
        };
        self.r.icon(g, R::xywh(x0 + 8.0, y3, 24.0, 34.0), 15.0, if net.online() { th.accent } else { th.bad });
        if let Some(v) = &net.vpn {
            label.push_str(&format!("  ·  VPN {v}"));
        }
        self.r.text(&label, R::new(x0 + 36.0, y3, x1 - 200.0, y3 + 34.0), 12.0, 600, th.text, Align::Left);
        let rate = |b: f64| if b >= 1e6 { format!("{:.1} MB/s", b / 1e6) } else { format!("{:.0} KB/s", b / 1e3) };
        let mut right = format!("↓ {}   ↑ {}", rate(s.net_down), rate(s.net_up));
        if let Some(ms) = s.ping {
            right.push_str(&format!("   ·   {ms} ms"));
        }
        self.r.text(&right, R::new(x1 - 210.0, y3, x1 - 12.0, y3 + 34.0), 11.5, 500, th.subtext, Align::Right);

        // Bluetooth devices' batteries.
        let yb = y3 + 34.0 + 8.0;
        self.draw_batteries(&s.batteries, x0, x1, yb);

        // The apps using the most, with a button to close one (click twice).
        let mut y4 = yb + 34.0 + 8.0;
        self.r.text("Top apps", R::new(x0 + 4.0, y4, x0 + 200.0, y4 + 22.0), 11.0, 600, th.subtext, Align::Left);
        for (i, (label, m)) in [("CPU", false), ("Memory", true)].into_iter().enumerate() {
            let r = R::new(x1 - 128.0 + i as f32 * 64.0, y4 + 1.0, x1 - 68.0 + i as f32 * 64.0 + 4.0 * i as f32, y4 + 21.0);
            let on = self.sys_by_mem == m;
            let hot = self.hot == Some(Btn::SysSort(m));
            self.r.fill_round(r, 10.0, if on { th.accent } else if hot { th.hover } else { th.bg2.a(0.6) });
            self.r.text(label, r, 10.5, 600, if on { th.bg } else { th.text }, Align::Center);
            self.buttons.push((r, Btn::SysSort(m)));
        }
        y4 += 24.0;
        let by_mem = self.sys_by_mem;
        self.draw_top_apps(&s, x0, x1, y4, by_mem, 3);
    }

    /// The apps using the most CPU or memory, `n` rows, each with a Close button (click twice).
    fn draw_top_apps(&mut self, s: &crate::sysinfo::Sys, x0: f32, x1: f32, y4: f32, by_mem: bool, n: usize) {
        let th = self.theme.clone();
        let gb = |b: u64| b as f64 / 1_073_741_824.0;
        let mut procs = s.procs.clone();
        if by_mem {
            procs.sort_by(|a, b| b.mem.cmp(&a.mem));
        } else {
            procs.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.mem.cmp(&a.mem)));
        }
        procs.truncate(n);
        self.r.fill_round(R::new(x0, y4, x1, y4 + n as f32 * 26.0), 12.0, th.bg2.a(0.55));
        if procs.is_empty() {
            self.r.text("Looking at what's running…", R::new(x0 + 12.0, y4, x1, y4 + 26.0), 11.5, 500, th.subtext, Align::Left);
        }
        for (i, p) in procs.iter().enumerate() {
            let ry = y4 + i as f32 * 26.0;
            self.r.text(&p.name, R::new(x0 + 12.0, ry, x1 - 200.0, ry + 26.0), 12.0, 600, th.text, Align::Left);
            let cpu = if p.cpu < 0.1 { format!("{:.1}%", p.cpu * 100.0) } else { format!("{:.0}%", p.cpu * 100.0) };
            let mem = if p.mem >= 1 << 30 { format!("{:.1} GB", gb(p.mem)) } else { format!("{} MB", p.mem >> 20) };
            let (wc, wm) = if by_mem { (400, 600) } else { (600, 400) };
            self.r.text(&cpu, R::new(x1 - 200.0, ry, x1 - 142.0, ry + 26.0), 11.5, wc, th.text, Align::Right);
            self.r.text(&mem, R::new(x1 - 140.0, ry, x1 - 66.0, ry + 26.0), 11.5, wm, th.text, Align::Right);
            let armed = self.kill_armed.as_deref() == Some(p.exe.as_str());
            let hot = self.hot == Some(Btn::Kill(i));
            let b = R::new(x1 - 58.0, ry + 4.0, x1 - 6.0, ry + 22.0);
            self.r.fill_round(b, 9.0, if armed { th.bad } else if hot { th.hover } else { th.bg2 });
            self.r.text(if armed { "Sure?" } else { "Close" }, b, 10.5, 600, if armed { th.bg } else { th.text }, Align::Center);
            self.buttons.push((b, Btn::Kill(i)));
        }
        self.sys_top = procs;
    }

    fn sys_page_height(&self) -> f32 {
        let header = 36.0;
        let fact = 46.0 + 8.0;
        let graph = 84.0 + 8.0;
        let apps = 22.0 + 5.0 * 26.0;
        match self.sys_page {
            1 => header + graph + fact + 60.0 + 8.0 + apps,
            2 => {
                let sticks = self.sys.as_ref().map(|s| !s.mem_info.modules.is_empty()).unwrap_or(false);
                header + graph + fact * if sticks { 3.0 } else { 2.0 } + apps
            }
            3 => header + graph + fact * 2.0,
            4 => {
                if crate::scan::current().path.as_os_str().is_empty() {
                    let n = self.sys.as_ref().map(|s| s.drives.len()).unwrap_or(1).max(1) as f32;
                    header + n * 62.0 + 24.0
                } else {
                    header + 40.0 + SCAN_ROWS as f32 * 30.0 + 28.0
                }
            }
            _ => 64.0 * 2.0 + 8.0 + 8.0 + 34.0 + 8.0 + 34.0 + 8.0 + 24.0 + 78.0,
        }
    }

    fn sys_back(&mut self) {
        if self.sys_page == 4 {
            let l = crate::scan::current();
            if !l.path.as_os_str().is_empty() {
                self.scan_page = 0;
                match l.path.parent() {
                    Some(p) if l.path.file_name().is_some() => crate::scan::open(p.to_path_buf(), scan_wake),
                    _ => crate::scan::close(),
                }
                return;
            }
        }
        self.sys_page = 0;
    }

    /// "‹  [icon] Title ......... right"; returns where the page goes on.
    fn sys_header(&mut self, x0: f32, x1: f32, y: f32, glyph: &str, title: &str, right: &str) -> f32 {
        let th = self.theme.clone();
        let back = R::new(x0, y, x0 + 30.0, y + 28.0);
        let hot = self.hot == Some(Btn::SysBack);
        self.r.fill_round(back, 14.0, if hot { th.hover } else { th.bg2.a(0.6) });
        self.r.text("‹", back.offset(0.0, -1.0), 17.0, 700, th.text, Align::Center);
        self.buttons.push((back, Btn::SysBack));
        self.r.icon(glyph, R::xywh(x0 + 36.0, y, 24.0, 28.0), 15.0, th.accent);
        self.r.text(title, R::new(x0 + 62.0, y, x1 - 170.0, y + 28.0), 13.0, 700, th.text, Align::Left);
        self.r.text(right, R::new(x1 - 200.0, y, x1 - 4.0, y + 28.0), 12.0, 600, th.subtext, Align::Right);
        y + 36.0
    }

    /// A big history graph with a heading and a number.
    fn sys_graph(&mut self, x0: f32, x1: f32, y: f32, label: &str, value: &str, hist: Vec<f32>, col: Rgba) -> f32 {
        let th = self.theme.clone();
        let r = R::new(x0, y, x1, y + 84.0);
        self.r.fill_round(r, 12.0, th.bg2.a(0.6));
        self.r.text(label, R::new(x0 + 12.0, y + 4.0, x1 - 120.0, y + 24.0), 11.0, 600, th.subtext, Align::Left);
        self.r.text(value, R::new(x1 - 140.0, y + 3.0, x1 - 12.0, y + 25.0), 14.0, 700, th.text, Align::Right);
        self.r.sparkline(R::new(x0 + 10.0, y + 26.0, x1 - 10.0, y + 78.0), &hist, 60, col);
        y + 84.0 + 8.0
    }

    /// A row of small facts: (label, value, under it).
    fn sys_facts(&mut self, x0: f32, x1: f32, y: f32, facts: &[(&str, String, String)]) -> f32 {
        let th = self.theme.clone();
        let gap = 8.0;
        let n = facts.len().max(1) as f32;
        let w = (x1 - x0 - gap * (n - 1.0)) / n;
        for (i, (label, value, sub)) in facts.iter().enumerate() {
            let r = R::xywh(x0 + i as f32 * (w + gap), y, w, 46.0);
            self.r.fill_round(r, 10.0, th.bg2.a(0.55));
            self.r.text(label, R::new(r.x0 + 10.0, r.y0 + 3.0, r.x1 - 6.0, r.y0 + 18.0), 10.0, 500, th.subtext, Align::Left);
            self.r.text(value, R::new(r.x0 + 10.0, r.y0 + 16.0, r.x1 - 6.0, r.y0 + 34.0), 13.0, 700, th.text, Align::Left);
            if !sub.is_empty() {
                self.r.text(sub, R::new(r.x0 + 10.0, r.y0 + 31.0, r.x1 - 6.0, r.y0 + 45.0), 9.5, 400, th.subtext, Align::Left);
            }
        }
        y + 46.0 + 8.0
    }

    fn draw_cpu_page(&mut self, s: &crate::sysinfo::Sys, x0: f32, x1: f32, y: f32) {
        let th = self.theme.clone();
        let pct = format!("{}%", (s.cpu * 100.0).round());
        let name = if s.cpu_info.name.is_empty() { "CPU".to_string() } else { s.cpu_info.name.clone() };
        let y = self.sys_header(x0, x1, y, G_CPU, &name, &pct);
        let hist: Vec<f32> = self.cpu_hist.iter().copied().collect();
        let col = self.load_color(s.cpu);
        let speed = s.cpu_mhz.map(|m| format!("{:.2} GHz", m as f32 / 1000.0)).unwrap_or_default();
        let y = self.sys_graph(x0, x1, y, "Use over the last minute", &speed, hist, col);
        let up = {
            let t = s.uptime_secs;
            let (d, h, m) = (t / 86_400, t / 3600 % 24, t / 60 % 60);
            if d > 0 { format!("{d}d {h}h {m}m") } else { format!("{h}h {m}m") }
        };
        let base = if s.cpu_info.base_mhz > 0 { format!("base {:.2} GHz", s.cpu_info.base_mhz as f32 / 1000.0) } else { String::new() };
        let y = self.sys_facts(
            x0,
            x1,
            y,
            &[
                ("Speed", if speed.is_empty() { "–".into() } else { speed.clone() }, base),
                ("Cores", format!("{} cores", s.cpu_info.cores), format!("{} threads", s.cpu_info.threads)),
                ("Processes", s.perf.processes.to_string(), format!("{} threads", s.perf.threads)),
                ("Up time", up, "since the last restart".into()),
            ],
        );
        // Each logical processor.
        let r = R::new(x0, y, x1, y + 60.0);
        self.r.fill_round(r, 12.0, th.bg2.a(0.55));
        self.r.text("Each processor", R::new(x0 + 12.0, y + 2.0, x1, y + 18.0), 10.0, 500, th.subtext, Align::Left);
        let n = s.cores.len().max(1);
        let area = R::new(x0 + 12.0, y + 20.0, x1 - 12.0, y + 52.0);
        let bw = area.w() / n as f32;
        for (i, &c) in s.cores.iter().enumerate() {
            let b = R::new(area.x0 + i as f32 * bw + 1.0, area.y0, area.x0 + (i + 1) as f32 * bw - 1.0, area.y1);
            self.r.fill_round(b, 2.0, th.bg2);
            let hgt = (b.h() * c.clamp(0.0, 1.0)).max(2.0);
            self.r.fill_round(R::new(b.x0, b.y1 - hgt, b.x1, b.y1), 2.0, self.load_color(c));
        }
        let y = y + 60.0 + 8.0;
        self.r.text("Top apps by CPU", R::new(x0 + 4.0, y, x1, y + 22.0), 11.0, 600, th.subtext, Align::Left);
        self.draw_top_apps(s, x0, x1, y + 22.0, false, 5);
    }

    fn draw_mem_page(&mut self, s: &crate::sysinfo::Sys, x0: f32, x1: f32, y: f32) {
        let th = self.theme.clone();
        let gb = |b: u64| format!("{:.1} GB", b as f64 / 1_073_741_824.0);
        let mf = if s.mem_total > 0 { s.mem_used as f32 / s.mem_total as f32 } else { 0.0 };
        let right = format!("{} of {} · {}%", gb(s.mem_used), gb(s.mem_total), (mf * 100.0).round());
        let ram = s.mem_info.name();
        let title = if ram.is_empty() { "Memory".to_string() } else { format!("{ram} memory") };
        let y = self.sys_header(x0, x1, y, G_MEMORY, &title, &right);
        let hist: Vec<f32> = self.mem_hist.iter().copied().collect();
        let col = self.load_color(mf);
        let y = self.sys_graph(x0, x1, y, "In use over the last minute", &format!("{}%", (mf * 100.0).round()), hist, col);
        let p = &s.perf;
        let y = self.sys_facts(
            x0,
            x1,
            y,
            &[
                ("In use", gb(s.mem_used), format!("of {}", gb(s.mem_total))),
                ("Available", gb(s.mem_total.saturating_sub(s.mem_used)), "free for apps".into()),
                ("Committed", gb(p.commit), format!("of {} limit", gb(p.commit_limit))),
                ("Cached", gb(p.cached), "files kept in memory".into()),
            ],
        );
        let mb = |b: u64| format!("{} MB", b >> 20);
        let y = self.sys_facts(
            x0,
            x1,
            y,
            &[
                ("Paged pool", mb(p.paged), "Windows, can swap".into()),
                ("Non-paged pool", mb(p.nonpaged), "Windows, stays in RAM".into()),
                ("Processes", p.processes.to_string(), format!("{} threads", p.threads)),
                ("Handles", p.handles.to_string(), "open by all apps".into()),
            ],
        );
        let y = if let Some(m) = s.mem_info.modules.first() {
            let mi = &s.mem_info;
            let parts: Vec<&str> = mi.modules.iter().map(|m| m.part.as_str()).filter(|p| !p.is_empty()).collect();
            let part = if parts.is_empty() { "–".to_string() } else if parts.iter().all(|p| *p == parts[0]) { parts[0].to_string() } else { parts.join(", ") };
            self.sys_facts(
                x0,
                x1,
                y,
                &[
                    ("Type", if m.kind.is_empty() { "–".into() } else { m.kind.clone() }, if m.speed > 0 { format!("{} MT/s", m.speed) } else { String::new() }),
                    ("Sticks", mi.sticks(), format!("{} of {} slots used", mi.modules.len(), mi.slots.max(mi.modules.len()))),
                    ("Maker", if m.maker.is_empty() { "–".into() } else { m.maker.clone() }, if part == "–" { String::new() } else { part }),
                ],
            )
        } else {
            y
        };
        self.r.text("Top apps by memory", R::new(x0 + 4.0, y, x1, y + 22.0), 11.0, 600, th.subtext, Align::Left);
        self.draw_top_apps(s, x0, x1, y + 22.0, true, 5);
    }

    fn draw_gpu_page(&mut self, s: &crate::sysinfo::Sys, x0: f32, x1: f32, y: f32) {
        let Some(g) = s.gpu.clone() else {
            let y = self.sys_header(x0, x1, y, G_GPU, "GPU", "");
            return self.empty_row(G_GPU, "No NVIDIA GPU stats (the driver's nvml.dll isn't there)", x0, x1, y);
        };
        let gb = |b: u64| format!("{:.1} GB", b as f64 / 1_073_741_824.0);
        let y = self.sys_header(x0, x1, y, G_GPU, &g.name, &format!("{}%", (g.util * 100.0).round()));
        let hist: Vec<f32> = self.gpu_hist.iter().copied().collect();
        let col = self.load_color(g.util);
        let y = self.sys_graph(x0, x1, y, "Use over the last minute", &format!("{}%", (g.util * 100.0).round()), hist, col);
        let vf = if g.vram_total > 0 { g.vram_used as f32 / g.vram_total as f32 } else { 0.0 };
        let y = self.sys_facts(
            x0,
            x1,
            y,
            &[
                ("Video memory", gb(g.vram_used), format!("of {} · {}%", gb(g.vram_total), (vf * 100.0).round())),
                ("Memory load", format!("{}%", (g.mem_util * 100.0).round()), "how busy VRAM is".into()),
                ("Temperature", g.temp.map(|t| format!("{t}°C")).unwrap_or("–".into()), String::new()),
                ("Power", g.power.map(|w| format!("{w:.0} W")).unwrap_or("–".into()), "right now".into()),
            ],
        );
        let mhz = |m: Option<u32>| m.map(|m| format!("{m} MHz")).unwrap_or("–".into());
        self.sys_facts(
            x0,
            x1,
            y,
            &[
                ("Core clock", mhz(g.clock), String::new()),
                ("Memory clock", mhz(g.mem_clock), String::new()),
                ("Driver", if g.driver.is_empty() { "–".into() } else { g.driver.clone() }, "NVIDIA".into()),
            ],
        );
    }

    fn draw_storage_page(&mut self, s: &crate::sysinfo::Sys, x0: f32, x1: f32, y: f32) {
        let th = self.theme.clone();
        let l = crate::scan::current();
        if l.path.as_os_str().is_empty() {
            // The drives.
            let right = format!("{} drive{}", s.drives.len(), if s.drives.len() == 1 { "" } else { "s" });
            let mut y = self.sys_header(x0, x1, y, G_DISK, "Storage", &right);
            for (i, d) in s.drives.iter().enumerate() {
                let r = R::new(x0, y, x1, y + 56.0);
                let hot = self.hot == Some(Btn::Drive(i));
                self.r.fill_round(r, 12.0, if hot { th.hover } else { th.bg2.a(0.55) });
                self.r.icon(G_DISK, R::xywh(x0 + 8.0, y, 26.0, 56.0), 17.0, th.accent);
                let name = if d.removable { format!("{} ({}) · removable", d.label, d.root) } else { format!("{} ({})", d.label, d.root) };
                self.r.text(&name, R::new(x0 + 40.0, y + 4.0, x1 - 200.0, y + 22.0), 12.5, 600, th.text, Align::Left);
                let model = [d.model.as_str(), d.kind.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · ");
                self.r.text(&model, R::new(x0 + 40.0, y + 21.0, x1 - 26.0, y + 37.0), 10.5, 400, th.subtext, Align::Left);
                let used = d.total.saturating_sub(d.free);
                let f = used as f32 / d.total.max(1) as f32;
                self.r.text(
                    &format!("{} free of {}", crate::scan::human(d.free), crate::scan::human(d.total)),
                    R::new(x1 - 220.0, y + 4.0, x1 - 26.0, y + 24.0),
                    11.0,
                    500,
                    th.subtext,
                    Align::Right,
                );
                let bar = R::new(x0 + 40.0, y + 41.0, x1 - 26.0, y + 47.0);
                self.r.fill_round(bar, 3.0, th.bg2);
                self.r.fill_round(R::new(bar.x0, bar.y0, bar.x0 + (bar.w() * f).max(6.0), bar.y1), 3.0, self.load_color(f));
                self.r.text("›", R::new(x1 - 22.0, y, x1 - 8.0, y + 56.0), 15.0, 700, th.subtext, Align::Center);
                self.buttons.push((r, Btn::Drive(i)));
                y += 62.0;
            }
            self.r.text("Click a drive to see what's using the space, biggest first", R::new(x0 + 4.0, y, x1, y + 20.0), 10.5, 400, th.subtext, Align::Left);
            return;
        }
        // A folder: biggest first.
        let title = l.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| l.path.display().to_string());
        let total = l.total();
        let y = self.sys_header(x0, x1, y, G_FOLDER, &title, "");
        let open = R::new(x1 - 128.0, y - 36.0 + 2.0, x1, y - 36.0 + 26.0);
        let hot = self.hot == Some(Btn::ScanExplorer);
        self.r.fill_round(open, 12.0, if hot { th.hover } else { th.bg2.a(0.7) });
        self.r.text("Open in Explorer", open, 10.5, 600, th.text, Align::Center);
        self.buttons.push((open, Btn::ScanExplorer));
        self.r.text(&l.path.display().to_string(), R::new(x0 + 4.0, y - 2.0, x1 - 4.0, y + 16.0), 10.0, 400, th.subtext, Align::Left);
        let mut status = if l.done {
            format!("{} in {} items", crate::scan::human(total), l.entries.len())
        } else {
            format!("Adding up… {} so far · {} files", crate::scan::human(total), l.files)
        };
        if l.denied > 0 {
            status.push_str(&format!(" · {} folder{} couldn't be read", l.denied, if l.denied == 1 { "" } else { "s" }));
        }
        self.r.text(&status, R::new(x0 + 4.0, y + 15.0, x1 - 4.0, y + 33.0), 10.5, 600, if l.done { th.text } else { th.accent }, Align::Left);
        let y = y + 40.0;
        let pages = l.entries.len().div_ceil(SCAN_ROWS).max(1);
        self.scan_page = self.scan_page.min(pages - 1);
        let start = self.scan_page * SCAN_ROWS;
        let biggest = l.entries.first().and_then(|e| e.size).unwrap_or(1).max(1);
        if l.entries.is_empty() {
            self.empty_row(G_FOLDER, if l.done { "This folder is empty" } else { "Reading…" }, x0, x1, y);
        }
        for (k, e) in l.entries.iter().enumerate().skip(start).take(SCAN_ROWS) {
            let ry = y + (k - start) as f32 * 30.0;
            let r = R::new(x0, ry, x1, ry + 27.0);
            let hot = self.hot == Some(Btn::ScanRow(k));
            self.r.fill_round(r, 8.0, if hot { th.hover } else { th.bg2.a(0.45) });
            // A bar behind the row: its share of the biggest one here.
            if let Some(sz) = e.size {
                let f = sz as f32 / biggest as f32;
                self.r.fill_round(R::new(r.x0, r.y1 - 3.0, r.x0 + (r.w() * f).max(3.0), r.y1), 1.5, th.accent.a(0.55));
            }
            self.r.icon(if e.dir { G_FOLDER } else { G_FILE }, R::xywh(x0 + 6.0, ry, 22.0, 27.0), 13.0, if e.dir { th.accent } else { th.subtext });
            self.r.text(&e.name, R::new(x0 + 32.0, ry, x1 - 150.0, ry + 27.0), 11.5, if e.dir { 600 } else { 400 }, th.text, Align::Left);
            let (size, share) = match e.size {
                Some(sz) => (crate::scan::human(sz), if total > 0 { format!("{:.0}%", sz as f64 * 100.0 / total as f64) } else { String::new() }),
                None => ("…".into(), String::new()),
            };
            self.r.text(&share, R::new(x1 - 150.0, ry, x1 - 96.0, ry + 27.0), 10.5, 400, th.subtext, Align::Right);
            self.r.text(&size, R::new(x1 - 94.0, ry, x1 - 22.0, ry + 27.0), 11.5, 600, th.text, Align::Right);
            self.r.text(if e.dir { "›" } else { "" }, R::new(x1 - 20.0, ry, x1 - 6.0, ry + 27.0), 13.0, 700, th.subtext, Align::Center);
            self.buttons.push((r, Btn::ScanRow(k)));
        }
        let py = y + SCAN_ROWS as f32 * 30.0 + 2.0;
        for (r, d, glyph) in [(R::new(x0, py, x0 + 30.0, py + 22.0), -1, "‹"), (R::new(x1 - 30.0, py, x1, py + 22.0), 1, "›")] {
            let hot = self.hot == Some(Btn::ScanPage(d));
            self.r.fill_round(r, 10.0, if hot { th.hover } else { th.bg2.a(0.6) });
            self.r.text(glyph, r, 15.0, 700, th.text, Align::Center);
            self.buttons.push((r, Btn::ScanPage(d)));
        }
        let shown = if l.entries.is_empty() { "0".to_string() } else { format!("{}–{}", start + 1, (start + SCAN_ROWS).min(l.entries.len())) };
        let line = format!("{shown} of {}   ·   click a folder to look inside, a file to show it in Explorer", l.entries.len());
        self.r.text(&line, R::new(x0 + 36.0, py, x1 - 36.0, py + 22.0), 10.5, 500, th.subtext, Align::Center);
    }

    fn draw_weather_tab(&mut self, x0: f32, x1: f32, mut y: f32) {
        let th = self.theme.clone();
        let Some(w) = self.weather.clone() else {
            return self.empty_row(G_WEATHER, "Getting the weather…", x0, x1, y);
        };
        let (glyph, desc) = weather::describe(w.code, w.day);
        self.r.icon(glyph, R::xywh(x0 - 4.0, y, 52.0, 56.0), 34.0, th.accent);
        let temp = format!("{}°", w.temp.round());
        let tw = self.text_width_at(&temp, 30.0, 600);
        self.r.text(&temp, R::new(x0 + 52.0, y, x0 + 54.0 + tw, y + 56.0), 30.0, 600, th.text, Align::Left);
        let tx = x0 + 52.0 + tw + 12.0;
        self.r.text(desc, R::new(tx, y + 9.0, x1 - 140.0, y + 28.0), 13.5, 600, th.text, Align::Left);
        let today = w.days.first().cloned();
        let line2 = match &today {
            Some(d) => format!("Feels {}° · H {}° L {}°", w.feels.round(), d.hi.round(), d.lo.round()),
            None => format!("Feels {}°", w.feels.round()),
        };
        self.r.text(&line2, R::new(tx, y + 28.0, x1 - 140.0, y + 46.0), 11.5, 400, th.subtext, Align::Left);
        self.r.text(&w.place, R::new(x1 - 140.0, y + 9.0, x1, y + 28.0), 12.0, 600, th.text, Align::Right);
        let rain = today.map(|d| format!("Rain {}% · ", d.rain)).unwrap_or_default();
        let line = format!("{rain}{} km/h · {}%", w.wind.round(), w.humidity);
        self.r.text(&line, R::new(x1 - 160.0, y + 28.0, x1, y + 46.0), 11.5, 400, th.subtext, Align::Right);
        y += 56.0 + 10.0;

        // Next hours, then the next days.
        let panels: [Vec<(String, &'static str, String)>; 2] = [
            w.hours.iter().map(|h| (h.label.clone(), weather::describe(h.code, h.day).0, format!("{}°", h.temp.round()))).collect(),
            w.days.iter().take(6).map(|d| (d.label.clone(), weather::describe(d.code, true).0, format!("{}° / {}°", d.hi.round(), d.lo.round()))).collect(),
        ];
        for cols in panels {
            let panel = R::new(x0, y, x1, y + 58.0);
            self.r.fill_round(panel, 12.0, th.bg2.a(0.55));
            let n = cols.len().max(1) as f32;
            let cw = panel.w() / n;
            for (i, (label, g, t)) in cols.iter().enumerate() {
                let cx = x0 + i as f32 * cw;
                self.r.text(label, R::new(cx, y + 4.0, cx + cw, y + 19.0), 11.0, 500, th.subtext, Align::Center);
                self.r.icon(g, R::new(cx, y + 18.0, cx + cw, y + 39.0), 15.0, th.accent);
                self.r.text(t, R::new(cx, y + 38.0, cx + cw, y + 55.0), 11.5, 600, th.text, Align::Center);
            }
            y += 58.0 + 8.0;
        }
    }

    fn draw_home_tab(&mut self, x0: f32, x1: f32, y: f32) {
        let th = self.theme.clone();
        let gap = 8.0;
        let cw = (x1 - x0 - 3.0 * gap) / 4.0;
        for (i, sc) in self.cfg.shortcuts.clone().iter().enumerate() {
            let r = R::xywh(x0 + (i % 4) as f32 * (cw + gap), y + (i / 4) as f32 * (58.0 + gap), cw, 58.0);
            let hot = self.hot == Some(Btn::Shortcut(i));
            self.r.fill_round(r, 12.0, if hot { th.hover } else { th.bg2.a(0.6) });
            self.r.icon(&sc.icon, R::new(r.x0, r.y0 + 6.0, r.x1, r.y0 + 32.0), 18.0, th.accent);
            self.r.text(&sc.name, R::new(r.x0 + 4.0, r.y0 + 33.0, r.x1 - 4.0, r.y0 + 52.0), 11.5, 500, th.text, Align::Center);
            self.buttons.push((r, Btn::Shortcut(i)));
        }
        let rows = self.cfg.shortcuts.len().div_ceil(4).max(1) as f32;
        self.draw_notes(x0, x1, y + rows * 58.0 + (rows - 1.0) * 8.0 + 12.0);
    }

    /// Notes and to-dos: click one to tick it, × to remove, and the box at the bottom to type
    /// a new one (Enter adds it, Esc stops).
    fn draw_notes(&mut self, x0: f32, x1: f32, mut y: f32) {
        let th = self.theme.clone();
        self.r.text("Notes", R::new(x0 + 2.0, y, x1, y + 20.0), 11.5, 600, th.subtext, Align::Left);
        if self.notes.iter().any(|n| n.done) {
            let r = R::new(x1 - 78.0, y + 1.0, x1, y + 19.0);
            let hot = self.hot == Some(Btn::NotesClear);
            self.r.fill_round(r, 9.0, if hot { th.hover } else { th.bg2.a(0.6) });
            self.r.text("Clear ticked", r, 10.5, 600, th.text, Align::Center);
            self.buttons.push((r, Btn::NotesClear));
        }
        y += 20.0;
        for (i, n) in self.notes.clone().iter().enumerate().take(crate::notes::MAX_NOTES) {
            let r = R::new(x0, y, x1, y + 24.0);
            let hot = matches!(self.hot, Some(Btn::Note(j)) | Some(Btn::NoteRemove(j)) if j == i);
            if hot {
                self.r.fill_round(r, 9.0, th.hover);
                let xr = R::xywh(x1 - 23.0, y + 3.0, 18.0, 18.0);
                let xhot = self.hot == Some(Btn::NoteRemove(i));
                self.r.fill_round(xr, 9.0, if xhot { th.bad.a(0.85) } else { th.bg });
                self.r.icon(G_CLOSE, xr, 10.0, if xhot { th.text } else { th.subtext });
                self.buttons.push((xr, Btn::NoteRemove(i)));
            }
            self.r.icon(if n.done { G_CHECK_CIRCLE } else { G_CIRCLE }, R::xywh(x0 + 4.0, y, 22.0, 24.0), 14.0, if n.done { th.good } else { th.subtext });
            let col = if n.done { th.subtext } else { th.text };
            let text = util::clip(&n.text, 70);
            self.r.text(&text, R::new(x0 + 30.0, y, x1 - 30.0, y + 24.0), 12.0, 500, col, Align::Left);
            if n.done {
                let w = self.text_width_at(&text, 12.0, 500).min(x1 - 30.0 - (x0 + 30.0));
                self.r.rect(R::new(x0 + 30.0, y + 12.0, x0 + 30.0 + w, y + 13.0), th.subtext.a(0.8));
            }
            self.buttons.push((r, Btn::Note(i)));
            y += 26.0;
        }
        // The box to type a new one in.
        let field = R::new(x0, y + 4.0, x1, y + 30.0);
        let full = self.notes.len() >= crate::notes::MAX_NOTES;
        let hot = self.hot == Some(Btn::NoteAdd);
        self.r.fill_round(field, 10.0, if hot && !self.note_typing { th.hover } else { th.bg2.a(0.6) });
        if self.note_typing {
            self.r.stroke_round(field.inset(0.5), 9.5, th.accent.a(0.9), 1.0);
        }
        self.r.icon(G_PLUS, R::xywh(field.x0 + 6.0, field.y0, 20.0, field.h()), 13.0, th.accent);
        let tx = field.x0 + 30.0;
        let room = field.x1 - 10.0 - tx;
        if full {
            self.r.text("Eight notes at most: tick and clear some to add more", R::new(tx, field.y0, field.x1 - 8.0, field.y1), 11.5, 400, th.subtext, Align::Left);
            return;
        }
        if self.note_draft.is_empty() && !self.note_typing {
            self.r.text("Add a note or to-do", R::new(tx, field.y0, field.x1 - 8.0, field.y1), 11.5, 400, th.subtext, Align::Left);
        } else {
            // Show the end of what's typed when it's longer than the box.
            let mut shown = self.note_draft.clone();
            while !shown.is_empty() && self.text_width_at(&shown, 12.0, 500) > room - 6.0 {
                shown.remove(0);
            }
            let w = if shown.is_empty() { 0.0 } else { self.text_width_at(&shown, 12.0, 500) };
            self.r.text(&shown, R::new(tx, field.y0, field.x1 - 8.0, field.y1), 12.0, 500, if self.note_typing { th.text } else { th.subtext }, Align::Left);
            if self.note_typing {
                self.r.rect(R::new(tx + w + 1.0, field.y0 + 7.0, tx + w + 2.5, field.y1 - 7.0), th.accent);
                if self.note_draft.is_empty() {
                    self.r.text("Type, then Enter · Esc to stop", R::new(tx + 8.0, field.y0, field.x1 - 8.0, field.y1), 11.0, 400, th.subtext.a(0.7), Align::Left);
                }
            }
        }
        self.buttons.push((field, Btn::NoteAdd));
    }

    /// The System tab's row of Bluetooth batteries (earbuds, mouse, controller…).
    fn draw_batteries(&mut self, list: &[crate::devices::Battery], x0: f32, x1: f32, y: f32) {
        use crate::devices::DevKind;
        let th = self.theme.clone();
        self.r.fill_round(R::new(x0, y, x1, y + 34.0), 12.0, th.bg2.a(0.55));
        self.r.icon(G_BLUETOOTH, R::xywh(x0 + 8.0, y, 24.0, 34.0), 15.0, th.accent);
        if list.is_empty() {
            self.r.text("No Bluetooth devices with a battery connected", R::new(x0 + 36.0, y, x1 - 12.0, y + 34.0), 12.0, 500, th.subtext, Align::Left);
            return;
        }
        let n = list.len().min(3);
        let cw = (x1 - x0 - 40.0) / n as f32;
        for (i, b) in list.iter().take(n).enumerate() {
            let cx = x0 + 36.0 + i as f32 * cw;
            let g = match b.kind {
                DevKind::Audio => G_HEADPHONES,
                DevKind::Mouse => G_MOUSE,
                DevKind::Keyboard => G_KEYBOARD,
                DevKind::Controller => G_GAME,
                DevKind::Pen => G_PEN,
                DevKind::Phone => G_PHONE,
                DevKind::Other => G_BLUETOOTH,
            };
            let low = b.pct <= 20;
            self.r.icon(g, R::xywh(cx, y, 20.0, 34.0), 13.0, th.text);
            let pct = format!("{}%", b.pct);
            let pw = self.text_width_at(&pct, 11.5, 600);
            let bar_w = 22.0;
            let right = cx + cw - 10.0;
            self.r.text(&b.name, R::new(cx + 22.0, y, right - pw - bar_w - 12.0, y + 34.0), 11.5, 600, th.text, Align::Left);
            let bar = R::new(right - pw - bar_w - 6.0, y + 14.0, right - pw - 6.0, y + 20.0);
            self.r.fill_round(bar, 3.0, th.bg2);
            let col = if low { th.bad } else { th.good };
            self.r.fill_round(R::new(bar.x0, bar.y0, bar.x0 + (bar.w() * b.pct as f32 / 100.0).max(3.0), bar.y1), 3.0, col);
            self.r.text(&pct, R::new(right - pw - 2.0, y, right, y + 34.0), 11.5, 600, if low { th.bad } else { th.text }, Align::Right);
        }
    }

    /// Controls tab: the sound outputs, the default one lit (click another to switch).
    fn draw_outputs(&mut self, x0: f32, x1: f32, mut y: f32) {
        if self.outputs.is_empty() {
            return;
        }
        let th = self.theme.clone();
        y += 12.0;
        self.r.text("Sound output", R::new(x0 + 2.0, y, x1, y + 20.0), 11.5, 600, th.subtext, Align::Left);
        if self.outputs.len() > 1 {
            self.r.text("Click one to switch", R::new(x0, y, x1 - 2.0, y + 20.0), 10.5, 400, th.subtext.a(0.8), Align::Right);
        }
        y += 20.0;
        let gap = 8.0;
        let cw = (x1 - x0 - 2.0 * gap) / 3.0;
        for (i, o) in self.outputs.clone().iter().take(OUTPUTS_SHOWN).enumerate() {
            let r = R::xywh(x0 + (i % 3) as f32 * (cw + gap), y + (i / 3) as f32 * (40.0 + gap), cw, 40.0);
            let hot = self.hot == Some(Btn::Output(i));
            let (fill, fg, sub) = if o.default { (th.accent, th.bg, th.bg.a(0.75)) } else if hot { (th.hover, th.text, th.subtext) } else { (th.bg2.a(0.6), th.text, th.subtext) };
            self.r.fill_round(r, 12.0, fill);
            let g = match o.form {
                3 | 5 => G_HEADPHONES,
                9 => G_MONITOR,
                _ => G_SPEAKER,
            };
            self.r.icon(g, R::xywh(r.x0 + 6.0, r.y0, 24.0, 40.0), 15.0, if o.default { th.bg } else { th.accent });
            let hw = o.hw.replace("(R)", "").replace("(TM)", "").replace("  ", " ").trim().to_string();
            let (title, small) = if o.form == 9 || hw.is_empty() { (o.name.clone(), hw) } else { (hw, o.name.clone()) };
            self.r.text(&title, R::new(r.x0 + 32.0, r.y0 + 4.0, r.x1 - 6.0, r.y0 + 22.0), 11.5, 600, fg, Align::Left);
            self.r.text(&small, R::new(r.x0 + 32.0, r.y0 + 20.0, r.x1 - 6.0, r.y1 - 4.0), 10.0, 400, sub, Align::Left);
            self.buttons.push((r, Btn::Output(i)));
        }
    }

    fn draw_controls_tab(&mut self, x0: f32, x1: f32, mut y: f32) {
        let th = self.theme.clone();
        let gap = 8.0;
        let cw = (x1 - x0 - 2.0 * gap) / 3.0;
        for (i, q) in QUICKS.iter().copied().enumerate() {
            let r = R::xywh(x0 + (i % 3) as f32 * (cw + gap), y + (i / 3) as f32 * (40.0 + gap), cw, 40.0);
            let on = self.quick_active(q);
            let hot = self.hot == Some(Btn::Quick(q));
            let (fill, fg) = if on { (th.accent, th.bg) } else if hot { (th.hover, th.text) } else { (th.bg2.a(0.6), th.text) };
            self.r.fill_round(r, 12.0, fill);
            self.r.icon(q.glyph(), R::xywh(r.x0 + 8.0, r.y0, 24.0, 40.0), 15.0, if on { th.bg } else { th.accent });
            self.r.text(q.label(), R::new(r.x0 + 36.0, r.y0, r.x1 - 6.0, r.y1), 11.5, 600, fg, Align::Left);
            self.buttons.push((r, Btn::Quick(q)));
        }
        y += 40.0 * 2.0 + gap + 12.0;

        let vol = self.last_vol;
        let vol_glyph = match vol {
            Some((v, m)) if m || v == 0.0 => G_VOL_MUTE,
            Some((v, _)) if v < 0.34 => G_VOL_LOW,
            Some((v, _)) if v < 0.67 => G_VOL_MED,
            _ => G_VOL_HIGH,
        };
        let rows = [
            (Slider::Brightness, G_SUN, self.brightness.map(|b| (b as f32 / 100.0, false))),
            (Slider::Volume, vol_glyph, vol),
            (Slider::Mic, G_MIC, self.ctl_mic),
            (Slider::Break, G_EYE, Some((self.break_left_min() as f32 / BREAK_MAX, false))),
        ];
        let (ta, tb) = (x0 + 34.0, x1 - 48.0);
        self.track = (ta, tb);
        for (s, g, val) in rows {
            let cy = y + 15.0;
            self.r.icon(g, R::xywh(x0, y, 24.0, 30.0), 14.0, th.text);
            let bar = R::new(ta, cy - 3.0, tb, cy + 3.0);
            self.r.fill_round(bar, 3.0, th.bg2);
            match val {
                Some((v, muted)) => {
                    let col = if muted { th.subtext.a(0.6) } else { th.accent };
                    let kx = ta + (tb - ta) * v.clamp(0.0, 1.0);
                    self.r.fill_round(R::new(ta, bar.y0, kx.max(ta + 6.0), bar.y1), 3.0, col);
                    let knob = if self.dragging == Some(s) || self.hot == Some(Btn::Slider(s)) { 8.0 } else { 6.5 };
                    self.r.circle(kx, cy, knob, th.text);
                    let label = if s == Slider::Break {
                        self.break_label()
                    } else if muted {
                        "Muted".to_string()
                    } else {
                        format!("{}%", (v * 100.0).round())
                    };
                    self.r.text(&label, R::new(x1 - 44.0, y, x1, y + 30.0), 12.0, 600, th.text, Align::Right);
                }
                None => {
                    self.r.text("–", R::new(x1 - 44.0, y, x1, y + 30.0), 12.0, 600, th.subtext, Align::Right);
                }
            }
            if val.is_some() {
                self.buttons.push((R::new(ta - 10.0, y, tb + 10.0, y + 30.0), Btn::Slider(s)));
            }
            y += 30.0;
        }
        y = self.draw_wpe(x0, x1, y);
        self.draw_outputs(x0, x1, y);
    }

    fn wpe_height(&self) -> f32 {
        if self.wpe_titles.is_empty() { 0.0 } else { 12.0 + 20.0 + 36.0 }
    }

    /// Controls: what Wallpaper Engine is showing, and "Another" for a fresh one now.
    fn draw_wpe(&mut self, x0: f32, x1: f32, mut y: f32) -> f32 {
        if self.wpe_titles.is_empty() {
            return y;
        }
        let th = self.theme.clone();
        y += 12.0;
        let head = if self.cfg.daily_wallpaper { "Wallpaper of the day" } else { "Wallpaper" };
        self.r.text(head, R::new(x0 + 2.0, y, x1, y + 20.0), 11.5, 600, th.subtext, Align::Left);
        self.r.text("Wallpaper Engine", R::new(x0, y, x1 - 2.0, y + 20.0), 10.5, 400, th.subtext.a(0.8), Align::Right);
        y += 20.0;
        let row = R::new(x0, y, x1, y + 36.0);
        self.r.fill_round(row, 12.0, th.bg2.a(0.6));
        self.r.icon(G_SCREENSHOT, R::xywh(row.x0 + 6.0, row.y0, 24.0, 36.0), 15.0, th.accent);
        let bw = 84.0;
        let title = self.wpe_titles.join("  ·  ");
        self.r.text(&title, R::new(row.x0 + 34.0, row.y0, row.x1 - bw - 10.0, row.y1), 11.5, 600, th.text, Align::Left);
        let b = R::new(row.x1 - bw - 4.0, row.y0 + 4.0, row.x1 - 4.0, row.y1 - 4.0);
        let busy = self.wpe_asked.is_some_and(|t| t.elapsed() < Duration::from_secs(4));
        let hot = self.hot == Some(Btn::WallNext);
        self.r.fill_round(b, 9.0, if hot && !busy { th.accent } else { th.hover });
        self.r.text(if busy { "Changing…" } else { "Another" }, b, 11.0, 600, if hot && !busy { th.bg } else { th.text }, Align::Center);
        self.buttons.push((b, Btn::WallNext));
        y + 36.0
    }

    fn draw_power_tab(&mut self, x0: f32, x1: f32, mut y: f32) {
        let th = self.theme.clone();
        self.r.icon(G_ACCOUNT, R::xywh(x0 - 2.0, y, 30.0, 34.0), 22.0, th.accent);
        let name = if self.user.is_empty() { "You".to_string() } else { self.user.clone() };
        self.r.text(&name, R::new(x0 + 34.0, y, x1 - 120.0, y + 34.0), 13.5, 600, th.text, Align::Left);
        self.r.text(&controls::uptime(), R::new(x1 - 120.0, y, x1, y + 34.0), 11.5, 400, th.subtext, Align::Right);
        y += 34.0 + 10.0;
        let gap = 6.0;
        let n = PowerAct::ALL.len() as f32;
        let cw = (x1 - x0 - (n - 1.0) * gap) / n;
        for (i, act) in PowerAct::ALL.iter().copied().enumerate() {
            let r = R::xywh(x0 + i as f32 * (cw + gap), y, cw, 62.0);
            let armed = self.armed == Some(act);
            let hot = self.hot == Some(Btn::Power(act));
            let fill = if armed { th.bad } else if hot { th.hover } else { th.bg2.a(0.6) };
            self.r.fill_round(r, 12.0, fill);
            self.r.icon(act.glyph(), R::new(r.x0, r.y0 + 8.0, r.x1, r.y0 + 36.0), 18.0, if armed { th.bg } else { th.accent });
            let label = if armed { "Sure?" } else { act.label() };
            self.r.text(label, R::new(r.x0 + 2.0, r.y0 + 36.0, r.x1 - 2.0, r.y0 + 55.0), 11.0, 600, if armed { th.bg } else { th.text }, Align::Center);
            self.buttons.push((r, Btn::Power(act)));
        }
    }

    /// Whether the player is still running (forgets it once it has closed).
    fn check_game(&mut self) {
        if let Some((c, _)) = self.game_child.as_mut() {
            if !matches!(c.try_wait(), Ok(None)) {
                self.game_child = None;
                self.dirty = true;
            }
        }
    }

    fn game_picked(&mut self, path: std::path::PathBuf) {
        let Some(g) = crate::games::entry(&path) else {
            let tint = self.theme.bad;
            self.quick_card("game", G_GAME, tint, "Games", "Can't play that file", "Pick a .nds, .gba, .gb, .gbc or a .zip with one inside", 5.0);
            return;
        };
        self.games = crate::games::load();
        let i = match self.games.iter().position(|x| x.path == g.path) {
            Some(i) => i,
            None => {
                self.games.insert(0, g);
                0
            }
        };
        crate::games::save(&self.games);
        self.play_game(i);
    }

    fn play_game(&mut self, i: usize) {
        self.check_game();
        if self.game_child.is_some() {
            crate::games::show();
            return;
        }
        let Some(g) = self.games.get(i).cloned() else { return };
        if !crate::games::has_core(&g.system) {
            let tint = self.theme.bad;
            self.quick_card("game", G_GAME, tint, "Games", "The emulator isn't installed", &format!("Its core goes in {}", crate::emu::cores_dir().display()), 6.0);
            return;
        }
        if let Some(child) = crate::games::launch(&g) {
            util::log(&format!("game: playing {} ({})", g.title, g.system));
            self.game_child = Some((child, g.title.clone()));
            let mut g = g;
            g.last = crate::dex::now_unix();
            self.games.remove(i);
            self.games.insert(0, g);
            crate::games::save(&self.games);
            self.expanded = false;
            self.dirty = true;
        }
    }

    fn draw_game_tab(&mut self, x0: f32, x1: f32, mut y: f32) {
        let th = self.theme.clone();
        self.check_game();
        self.r.icon(G_GAME, R::xywh(x0 - 2.0, y, 30.0, 34.0), 20.0, th.accent);
        let n = self.games.len();
        let head = if n == 0 { "Games".to_string() } else { format!("Games · {n}") };
        self.r.text(&head, R::new(x0 + 32.0, y, x1 - 140.0, y + 34.0), 13.5, 600, th.text, Align::Left);
        let open = R::new(x1 - 118.0, y + 4.0, x1, y + 30.0);
        let hot = self.hot == Some(Btn::GameOpen);
        self.r.fill_round(open, 13.0, if hot { th.accent.a(0.85) } else { th.accent });
        self.r.text("Open game…", open, 11.5, 700, th.bg, Align::Center);
        self.buttons.push((open, Btn::GameOpen));
        y += 34.0 + 8.0;

        if let Some(title) = self.game_child.as_ref().map(|(_, t)| t.clone()) {
            let card = R::new(x0, y, x1, y + 44.0);
            self.r.fill_round(card, 12.0, th.accent.a(0.16));
            self.r.text(&format!("Playing {title}"), R::new(x0 + 12.0, y + 4.0, x1 - 150.0, y + 24.0), 12.5, 700, th.text, Align::Left);
            self.r.text("F5 save state · F8 load · hold Tab to fast-forward", R::new(x0 + 12.0, y + 23.0, x1 - 150.0, y + 40.0), 10.5, 400, th.subtext, Align::Left);
            for (k, (label, b)) in [("Show", Btn::GameShow), ("Stop", Btn::GameStop)].into_iter().enumerate() {
                let r = R::new(x1 - 136.0 + k as f32 * 66.0, y + 10.0, x1 - 76.0 + k as f32 * 66.0, y + 34.0);
                let hot = self.hot == Some(b);
                let fill = if k == 1 { if hot { th.bad.a(0.85) } else { th.bad.a(0.7) } } else if hot { th.hover } else { th.bg2 };
                self.r.fill_round(r, 12.0, fill);
                self.r.text(label, r, 11.0, 700, th.text, Align::Center);
                self.buttons.push((r, b));
            }
            y += 52.0;
        }

        if self.games.is_empty() {
            self.r.fill_round(R::new(x0, y, x1, y + 40.0), 12.0, th.bg2.a(0.5));
            self.r.text("Open a DS, GBA or Game Boy game you own (.nds, .gba, .gb, .gbc, or a .zip of one)", R::new(x0 + 12.0, y, x1 - 12.0, y + 40.0), 11.0, 500, th.subtext, Align::Left);
            y += 44.0;
        }
        let today = crate::dex::now_unix() / 86_400;
        let games = self.games.clone();
        for (i, g) in games.iter().enumerate().take(GAME_ROWS) {
            let row = R::new(x0, y, x1, y + 40.0);
            let hot_row = self.hot == Some(Btn::GamePlay(i));
            self.r.fill_round(row, 12.0, if hot_row { th.hover } else { th.bg2.a(0.55) });
            let badge = R::new(x0 + 10.0, y + 10.0, x0 + 48.0, y + 30.0);
            self.r.fill_round(badge, 6.0, th.accent.a(0.22));
            self.r.text(&g.system, badge, 10.0, 700, th.accent, Align::Center);
            self.r.text(&g.title, R::new(x0 + 58.0, y + 3.0, x1 - 130.0, y + 23.0), 12.5, 600, th.text, Align::Left);
            let when = match g.last {
                0 => "Not played yet".to_string(),
                t => match today - t / 86_400 {
                    0 => "Played today".into(),
                    1 => "Played yesterday".into(),
                    d => format!("Played {d} days ago"),
                },
            };
            let sub = if crate::games::has_core(&g.system) { when } else { "Emulator not installed".into() };
            self.r.text(&sub, R::new(x0 + 58.0, y + 21.0, x1 - 130.0, y + 37.0), 10.5, 400, th.subtext, Align::Left);
            let play = R::new(x1 - 112.0, y + 8.0, x1 - 44.0, y + 32.0);
            self.r.fill_round(play, 12.0, th.accent);
            self.r.icon(G_PLAY, R::new(play.x0 + 4.0, play.y0, play.x0 + 26.0, play.y1), 13.0, th.bg);
            self.r.text("Play", R::new(play.x0 + 22.0, play.y0, play.x1 - 4.0, play.y1), 11.0, 700, th.bg, Align::Center);
            let rm = R::new(x1 - 38.0, y + 8.0, x1 - 14.0, y + 32.0);
            let hot_rm = self.hot == Some(Btn::GameRemove(i));
            self.r.icon(G_CLOSE, rm, 13.0, if hot_rm { th.bad } else { th.subtext });
            // The whole row plays it; the × only takes it off the list (the file stays).
            self.buttons.push((rm, Btn::GameRemove(i)));
            self.buttons.push((row, Btn::GamePlay(i)));
            y += 44.0;
        }
        y += 6.0;
        let keys = "Arrows: D-pad · X / Z: A / B · S / A: X / Y · Q / W: L / R · Enter: Start · Right Shift: Select · Mouse: DS touch screen · F5 / F8: save / load state · hold Tab: fast-forward";
        self.r.text_wrapped(keys, R::new(x0 + 4.0, y, x1 - 4.0, y + 32.0), 10.5, 400, th.subtext);
    }

    fn empty_row(&mut self, glyph: &str, msg: &str, x0: f32, x1: f32, y: f32) {
        let th = self.theme.clone();
        self.r.icon(glyph, R::xywh(x0, y, 20.0, 26.0), 14.0, th.subtext);
        self.r.text(msg, R::new(x0 + 28.0, y, x1, y + 26.0), 12.0, 500, th.subtext, Align::Left);
    }

    fn draw_media_tab(&mut self, x0: f32, x1: f32, mut y: f32) {
        let th = self.theme.clone();
        let Some(m) = self.media.clone() else {
            return self.empty_row(G_MUSIC, "Nothing playing", x0, x1, y);
        };
        let tint = self.tint.unwrap_or(th.accent);
        let art = R::xywh(x0, y, 56.0, 56.0);
        match &self.art_bmp {
            Some(b) => self.r.image(b, art, 12.0, 1.0),
            None => {
                self.r.fill_round(art, 12.0, th.bg2);
                self.r.icon(G_MUSIC, art, 22.0, tint);
            }
        }
        let btns = [(Btn::Prev, G_PREV, 16.0), (Btn::PlayPause, if self.playing { G_PAUSE } else { G_PLAY }, 19.0), (Btn::Next, G_NEXT, 16.0)];
        let bx0 = x1 - 3.0 * 32.0;
        for (i, (b, g, size)) in btns.iter().enumerate() {
            let r = R::xywh(bx0 + i as f32 * 32.0, y + 13.0, 30.0, 30.0);
            if *b == Btn::PlayPause {
                self.r.fill_round(r, 15.0, if self.hot == Some(*b) { tint } else { tint.a(0.88) });
                self.r.icon(g, r, *size, th.bg);
            } else {
                if self.hot == Some(*b) {
                    self.r.fill_round(r, 15.0, th.hover);
                }
                self.r.icon(g, r, *size, th.text);
            }
            self.buttons.push((r, *b));
        }
        let tx = x0 + 68.0;
        let tr = bx0 - 8.0;
        self.r.text(&m.title, R::new(tx, y + 3.0, tr, y + 23.0), 13.5, 600, th.text, Align::Left);
        self.r.text(&m.artist, R::new(tx, y + 23.0, tr, y + 39.0), 12.0, 400, th.subtext, Align::Left);
        self.r.text(&m.app, R::new(tx, y + 39.0, tr, y + 54.0), 11.0, 400, th.subtext.a(0.7), Align::Left);
        y += 66.0;
        if let Some((pos, end, at)) = self.timeline {
            let cur = if self.playing { pos + at.elapsed().as_secs_f64() } else { pos };
            let frac = (cur / end).clamp(0.0, 1.0) as f32;
            let bar = R::new(x0, y, x1, y + 4.0);
            self.r.fill_round(bar, 2.0, th.bg2);
            self.r.fill_round(R::new(x0, y, x0 + (bar.w() * frac).max(4.0), y + 4.0), 2.0, tint);
        }
    }

    fn draw_usage_tab(&mut self, x0: f32, x1: f32, mut y: f32) {
        let th = self.theme.clone();
        if self.usage.is_empty() {
            return self.empty_row(G_GAUGE, "Checking Claude and Codex limits…", x0, x1, y);
        }
        let now = usage::now_unix();
        for u in self.usage.clone() {
            let worst = u.limits.iter().map(|l| l.used).fold(0.0f32, f32::max);
            let icol = if u.limits.is_empty() { th.subtext } else { self.level_color(worst) };
            self.r.icon(G_GAUGE, R::xywh(x0, y, 20.0, 34.0), 14.0, icol);
            self.r.text(u.provider, R::new(x0 + 28.0, y + 1.0, x0 + 100.0, y + 18.0), 12.0, 600, th.text, Align::Left);
            let (note, ncol) = match (&u.error, u.limits.is_empty()) {
                (Some(_), false) => ("not updated".to_string(), th.subtext.a(0.7)),
                _ => (u.note.clone(), th.subtext),
            };
            self.r.text(&note, R::new(x0 + 28.0, y + 17.0, x0 + 98.0, y + 32.0), 10.5, 400, ncol, Align::Left);
            let cx0 = x0 + 104.0;
            if u.limits.is_empty() {
                let msg = u.error.clone().unwrap_or_default();
                self.r.text(&msg, R::new(cx0, y, x1, y + 34.0), 11.5, 400, th.subtext, Align::Left);
            } else {
                let n = u.limits.len().min(3);
                let gap = 14.0;
                let cw = (x1 - cx0 - gap * (n as f32 - 1.0)) / n as f32;
                for (i, l) in u.limits.iter().take(n).enumerate() {
                    let lx = cx0 + i as f32 * (cw + gap);
                    let col = self.level_color(l.used);
                    let label = match l.resets_at {
                        Some(r) if l.used > 0.0 && r > now => format!("{} · {}", l.short, usage::countdown(r - now)),
                        _ => l.short.clone(),
                    };
                    let pct = format!("{}%", (l.used * 100.0).round());
                    self.r.text(&label, R::new(lx, y + 2.0, lx + cw - 34.0, y + 20.0), 11.0, 400, th.subtext, Align::Left);
                    let pcol = if l.used >= self.cfg.usage_watch { col } else { th.text };
                    self.r.text(&pct, R::new(lx + cw - 40.0, y + 2.0, lx + cw, y + 20.0), 12.0, 600, pcol, Align::Right);
                    let bar = R::new(lx, y + 23.0, lx + cw, y + 27.0);
                    self.r.fill_round(bar, 2.0, th.bg2);
                    if l.used > 0.0 {
                        self.r.fill_round(R::new(lx, bar.y0, lx + (cw * l.used.min(1.0)).max(4.0), bar.y1), 2.0, col);
                    }
                }
            }
            y += 34.0;
        }
    }

    fn draw_calendar_tab(&mut self, x0: f32, x1: f32, mut y: f32) {
        let th = self.theme.clone();
        self.r.icon(G_CAL, R::xywh(x0, y, 20.0, 26.0), 14.0, th.accent);
        let today = if self.cal_today.is_empty() { "Nothing on today".to_string() } else { self.cal_today.join(" · ") };
        self.r.text("Today", R::new(x0 + 28.0, y, x0 + 80.0, y + 26.0), 12.0, 600, th.text, Align::Left);
        let col = if self.cal_today.is_empty() { th.subtext } else { th.text };
        self.r.text(&today, R::new(x0 + 80.0, y, x1, y + 26.0), 12.0, 400, col, Align::Left);
        y += 26.0;
        if let Some((title, d)) = self.cal_next.clone() {
            let days = d.days() - self.today.days();
            self.r.text("Next", R::new(x0 + 28.0, y, x0 + 80.0, y + 26.0), 12.0, 600, th.subtext, Align::Left);
            self.r.text(&title, R::new(x0 + 80.0, y, x1 - 120.0, y + 26.0), 12.0, 400, th.text, Align::Left);
            let when = if days == 1 { format!("{} · tomorrow", d.short()) } else { format!("{} · in {days} d", d.short()) };
            self.r.text(&when, R::new(x1 - 120.0, y, x1, y + 26.0), 11.5, 400, th.subtext, Align::Right);
        }
    }
}

// ---------- window plumbing ----------

unsafe extern "system" fn kb_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && HOOK_ON.load(Ordering::Relaxed) {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        let vk = VIRTUAL_KEY(k.vkCode as u16);
        if vk == VK_VOLUME_UP || vk == VK_VOLUME_DOWN || vk == VK_VOLUME_MUTE {
            let msg = wp.0 as u32;
            if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
                let _ = PostMessageW(Some(events::hwnd()), WM_APP_VOLKEY, WPARAM(k.vkCode as usize), LPARAM(0));
            }
            // Swallow the key so the Windows volume flyout never appears.
            return LRESULT(1);
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

/// How long since the last keyboard or mouse input (ms).
/// The catching ring's size: 1 (widest) shrinking to 0, over and over.
fn ring_k(since: Instant) -> f32 {
    1.0 - (since.elapsed().as_secs_f32() / RING_SECS).fract()
}

/// What a throw of this quality (0..1) is called.
fn throw_note(q: f32) -> &'static str {
    if q >= 0.8 {
        "Excellent!"
    } else if q >= 0.55 {
        "Great!"
    } else if q >= 0.3 {
        "Nice!"
    } else {
        ""
    }
}

/// "full moon" → "Full moon".
fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// What the meeting sign says.
fn sign_text(left: u32) -> String {
    if left == 0 {
        "Meeting now!".into()
    } else {
        format!("Meeting in {left}")
    }
}

/// "45 min", "2 h", "1 h 20 min".
fn mins_text(m: u32) -> String {
    match (m / 60, m % 60) {
        (0, r) => format!("{r} min"),
        (h, 0) => format!("{h} h"),
        (h, r) => format!("{h} h {r} min"),
    }
}

fn user_idle_ms() -> u32 {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    unsafe {
        let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        if GetLastInputInfo(&mut li).as_bool() {
            windows::Win32::System::SystemInformation::GetTickCount().wrapping_sub(li.dwTime)
        } else {
            0
        }
    }
}

/// A random number in 0..1 (xorshift, seeded from the clock).
/// Is the island's weather place in the southern hemisphere (where the moon looks the other
/// way round)? Worked out once.
/// A Pokémon's types for a card: "Grass" or "Water/Ground".
fn type_names(types: &[usize]) -> String {
    types.iter().map(|t| pokemon::TYPES[*t % 18].0).collect::<Vec<_>>().join("/")
}

fn southern() -> bool {
    static SOUTH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SOUTH.get_or_init(|| Config::load().weather_place().is_some_and(|p| p.lat < 0.0))
}

fn rand01() -> f32 {
    use std::sync::atomic::AtomicU64;
    static SEED: AtomicU64 = AtomicU64::new(0);
    let mut x = SEED.load(Ordering::Relaxed);
    if x == 0 {
        x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x9e37_79b9) | 1;
    }
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    SEED.store(x, Ordering::Relaxed);
    (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32 / (1u64 << 24) as f32
}

/// Where a wild Pokémon is along its walk (0..1 across) and whether it's walking: in to
/// just before the middle, a look around, then out the other side.
/// How far along its walk a visitor is: it walks in, then waits there until you've caught it
/// (or it breaks out of three Poké Balls and runs off).
fn wild_u(since: Instant) -> f32 {
    (since.elapsed().as_secs_f32() / WILD_WALK).min(0.5)
}

/// A visitor that has arrived and is just waiting (no walking, no ball in the air).
fn wild_waiting(w: &Wild) -> bool {
    w.catch.is_none() && w.fight.is_none() && w.since.elapsed().as_secs_f32() / WILD_WALK >= 0.36
}

fn wild_path(u: f32) -> (f32, bool) {
    if u < 0.35 {
        (u / 0.35 * 0.45, true)
    } else if u < 0.6 {
        (0.45, false)
    } else {
        ((0.45 + (u - 0.6) / 0.4 * 0.55).min(1.0), true)
    }
}

fn open_path(p: &Path) {
    unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(p.as_os_str()), None, None, SW_SHOWNORMAL);
    }
}

unsafe fn context_menu(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else { return };
    let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, w!("Pocket Isle"));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, 1, w!("Reload YASB theme && settings"));
    let _ = AppendMenuW(menu, MF_STRING, 2, w!("Open settings file"));
    let _ = AppendMenuW(menu, MF_STRING, 3, w!("Open log"));
    let _ = AppendMenuW(menu, MF_STRING, 4, w!("Look for wild Pokémon"));
    let fishing = with_app(|a| a.fishing.is_some()).unwrap_or(false);
    let _ = AppendMenuW(menu, MF_STRING, 5, if fishing { w!("Stop fishing") } else { w!("Go fishing") });
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let _ = AppendMenuW(menu, MF_STRING, 9, w!("Quit Pocket Isle"));
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON, pt.x, pt.y, None, hwnd, None);
    let _ = DestroyMenu(menu);
    match cmd.0 {
        1 => {
            with_app(|a| a.reload_theme());
        }
        2 => open_path(&Config::path()),
        3 => open_path(&util::log_path()),
        4 => {
            with_app(|a| a.spawn_wild());
        }
        5 => {
            with_app(|a| a.toggle_fishing());
        }
        9 => PostQuitMessage(0),
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if util::demo()
        && matches!(
            msg,
            WM_MOUSEMOVE
                | WM_MOUSELEAVE
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_LBUTTONDBLCLK
                | WM_RBUTTONDOWN
                | WM_RBUTTONUP
                | WM_MBUTTONUP
                | WM_MOUSEWHEEL
                | WM_MOUSEHWHEEL
                | WM_CHAR
                | WM_COPYDATA
                | WM_APP_APPBAR
                | WM_APP_VOLKEY
                | WM_CLIPBOARDUPDATE
        )
    {
        return LRESULT(if msg == WM_COPYDATA { 1 } else { 0 });
    }
    match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_APP_EVENT => {
            with_app(|a| a.drain());
            return LRESULT(0);
        }
        events::WM_APP_SHOW => {
            with_app(|a| a.opened_again());
            return LRESULT(0);
        }
        WM_APP_VOLKEY => {
            with_app(|a| a.volume_key(wp.0 as u16));
            return LRESULT(0);
        }
        WM_APP_APPBAR => {
            if wp.0 as u32 == ABN_FULLSCREENAPP {
                with_app(|a| a.set_fullscreen(lp.0 != 0));
            }
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            if wp.0 & 0x0002 != 0 {
                // MK_RBUTTON: scrubbing the song.
                with_app(|a| a.with_view(hwnd, |a| a.right_move(lp)));
            }
            with_app(|a| a.with_view(hwnd, |a| a.mouse_move(lp)));
            if wp.0 & 0x0001 != 0 {
                // MK_LBUTTON: maybe dragging a file off the shelf. The drag runs its own
                // message loop, so it must run with the app not borrowed.
                if let Some(Some(path)) = with_app(|a| a.with_view(hwnd, |a| a.shelf_drag(lp))) {
                    crate::shelf::drag_out(hwnd, &path);
                    with_app(|a| a.dirty = true);
                }
            }
            return LRESULT(0);
        }
        crate::shelf::WM_APP_DRAG => {
            with_app(|a| a.with_view(hwnd, |a| a.drag_event(wp.0)));
            return LRESULT(0);
        }
        WM_MOUSELEAVE => {
            with_app(|a| a.with_view(hwnd, |a| a.mouse_leave()));
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => {
            with_app(|a| a.with_view(hwnd, |a| a.mouse_down(lp)));
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            with_app(|a| {
                a.with_view(hwnd, |a| {
                    if std::mem::take(&mut a.swallow_up) {
                        return;
                    }
                    if !a.end_drag() {
                        a.click(lp);
                    }
                })
            });
            return LRESULT(0);
        }
        WM_MOUSEHWHEEL => {
            let delta = ((wp.0 >> 16) & 0xffff) as u16 as i16;
            with_app(|a| a.with_view(hwnd, |a| a.hwheel(delta)));
            return LRESULT(0);
        }
        WM_CLIPBOARDUPDATE => {
            with_app(|a| a.clipboard());
            return LRESULT(0);
        }
        WM_CHAR => {
            with_app(|a| a.note_key(wp.0 as u16));
            return LRESULT(0);
        }
        WM_KILLFOCUS => {
            with_app(|a| a.note_blur());
        }
        WM_ACTIVATE => {
            if wp.0 & 0xffff == 0 {
                with_app(|a| a.note_blur());
            }
        }
        WM_POWERBROADCAST => {
            // PBT_APMPOWERSTATUSCHANGE: the charger or the battery level changed.
            if wp.0 == 0x000A && hwnd == main_hwnd() {
                SetTimer(Some(hwnd), T_POWER, 300, None);
            }
            return LRESULT(1);
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp.0 >> 16) & 0xffff) as u16 as i16;
            with_app(|a| a.with_view(hwnd, |a| a.wheel(delta)));
            return LRESULT(0);
        }
        WM_RBUTTONDOWN => {
            with_app(|a| a.with_view(hwnd, |a| a.right_down(lp)));
            return LRESULT(0);
        }
        WM_RBUTTONUP => {
            if !with_app(|a| a.with_view(hwnd, |a| a.right_up())).unwrap_or(false) {
                context_menu(hwnd);
            }
            return LRESULT(0);
        }
        WM_LBUTTONDBLCLK => {
            with_app(|a| a.with_view(hwnd, |a| a.double_click(lp)));
            return LRESULT(0);
        }
        WM_MBUTTONUP => {
            with_app(|a| a.with_view(hwnd, |a| a.middle_click()));
            return LRESULT(0);
        }
        WM_TIMER => {
            with_app(|a| a.with_view(hwnd, |a| a.timer(wp.0)));
            return LRESULT(0);
        }
        WM_COPYDATA => {
            let cds = &*(lp.0 as *const COPYDATASTRUCT);
            if cds.dwData == AGENT_MAGIC && !cds.lpData.is_null() {
                let bytes = std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
                let s = String::from_utf8_lossy(bytes).into_owned();
                // A permission prompt the island will show replies with how long to wait.
                return LRESULT(with_app(|a| a.agent_message(&s)).unwrap_or(1));
            }
            return LRESULT(1);
        }
        // Screen changes reach every window; the main one handles them (for the copies too).
        WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE if hwnd.0 as isize != MAIN_HWND.load(Ordering::SeqCst) => {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            with_app(|a| a.reposition());
            return LRESULT(0);
        }
        WM_SETTINGCHANGE => {
            if wp.0 as u32 == SPI_SETWORKAREA.0 {
                with_app(|a| a.reposition());
            } else if wp.0 as u32 == SPI_SETDESKWALLPAPER.0 {
                // Give Windows a moment to finish writing the new picture.
                SetTimer(Some(hwnd), T_WALL, 800, None);
            }
        }
        WM_DESTROY if hwnd.0 as isize == MAIN_HWND.load(Ordering::SeqCst) => {
            PostQuitMessage(0);
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// The Storage page's scanner has news: redraw.
fn scan_wake() {
    events::send(Event::ScanProgress);
}

pub fn run() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let _ = CoInitializeSecurity(
            None,
            -1,
            None,
            None,
            RPC_C_AUTHN_LEVEL_DEFAULT,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
            None,
        );

        let _single = CreateMutexW(None, true, w!("Local\\PocketIsleSingleton"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // Already running: open that island instead of quietly doing nothing.
            if let Ok(h) = FindWindowW(CLASS, None) {
                let _ = PostMessageW(Some(h), events::WM_APP_SHOW, WPARAM(0), LPARAM(0));
            }
            return Ok(());
        }

        let theme = Theme::load();
        let cfg = Config::load();
        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_HAND)?,
            style: CS_DBLCLKS,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        RegisterClassExW(&WNDCLASSEXW { lpszClassName: MIRROR_CLASS, ..wc });

        let p = placement(&theme, &cfg);
        let demo = util::demo();
        if demo {
            crate::demo::backdrop(p.x, p.y, p.pw, p.ph);
        }
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP,
            CLASS,
            w!("Pocket Isle"),
            WS_POPUP,
            p.x,
            p.y,
            p.pw,
            p.ph,
            None,
            None,
            Some(hinst),
            None,
        )?;

        MAIN_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
        if !demo {
            crate::shelf::accept_drops(hwnd);
        }
        let rx = events::init(hwnd);
        let r = Renderer::new(hwnd, p.pw as u32, p.ph as u32, p.scale, &theme.font, &theme.icon_font, cfg.gpu_rendering)?;
        let audio = Audio::new().map_err(|e| util::log(&format!("audio unavailable: {e}"))).ok();
        // The demo reads none of your things: its music, usage, weather and so on are made up.
        let media_tx = if demo { std::sync::mpsc::channel().0 } else { media::spawn() };
        if cfg.show_notifications && !demo {
            crate::notify::spawn(cfg.notification_ignore.clone());
        }
        crate::brightness::spawn();
        let usage_tx = if demo { std::sync::mpsc::channel().0 } else { usage::spawn(cfg.usage()) };
        sysinfo::spawn();
        if !demo {
            crate::privacy::spawn();
            crate::net::spawn();
            weather::spawn(cfg.weather_place(), cfg.weather_refresh_minutes);
        }
        let bright_tx = crate::brightness::spawn_setter();
        if !demo {
            if cfg.screenshot_peek {
                peeks::watch_screenshots();
            }
            if cfg.download_peek {
                peeks::watch_downloads();
            }
            if cfg.github_peeks {
                crate::github::watch(cfg.github_minutes);
            }
            if cfg.heat_peek {
                crate::heat::spawn(cfg.heat_gpu_c);
            }
            crate::wpe::spawn(cfg.daily_wallpaper);
            if cfg.copy_peek || cfg.screenshot_peek {
                let _ = windows::Win32::System::DataExchange::AddClipboardFormatListener(hwnd);
            }
        }

        HOOK_ON.store(cfg.hijack_volume_keys && audio.is_some() && !demo, Ordering::SeqCst);
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_hook), Some(hinst), 0).ok();

        // Registering as an app bar (without reserving any space) makes Windows tell us when
        // a full-screen app opens or closes, so we can step aside without polling.
        let mut abd = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            hWnd: hwnd,
            uCallbackMessage: WM_APP_APPBAR,
            ..Default::default()
        };
        SHAppBarMessage(ABM_NEW, &mut abd);

        let pill_h = p.pill_h;
        let now = Instant::now();
        let app = App {
            hwnd,
            r,
            theme,
            cfg,
            scale: p.scale,
            pill_h,
            monitor: p.monitor,
            neck: p.neck,
            mood: std::array::from_fn(|_| Spring::new(0.5, 60.0, 15.0, 0.002)),
            glow: Spring::new(0.0, 90.0, 18.0, 0.01),
            mouse: (0.0, 0.0),
            peak: Spring::new(0.0, 260.0, 26.0, 0.01),
            ripples: Vec::new(),
            hop: None,
            beat: 0.0,
            beat_slow: 0.0,
            last_beat: now,
            swipe_acc: 0.0,
            swipe: Spring::new(0.0, 240.0, 22.0, 0.2),
            swipe_lock: None,
            born: now,
            strips: HashMap::new(),
            last_clip: None,
            power: peeks::power(),
            low_warned: 0,
            w: Spring::new(44.0, 380.0, 27.0, 0.3),
            h: Spring::new(pill_h, 380.0, 27.0, 0.3),
            rad: Spring::new(pill_h / 2.0, 380.0, 27.0, 0.2),
            alpha: Spring::new(1.0, 260.0, 30.0, 0.01),
            mode: Mode::Compact,
            last_rgn: (0, 0, 0, 0),
            views: Vec::new(),
            device_lost: false,
            scrub: None,
            scrub_to: 0.0,
            mic_apps: Vec::new(),
            cam_apps: Vec::new(),
            privacy_seen: false,
            net: None,
            use_since: Instant::now(),
            shelf: crate::shelf::load(),
            clips: crate::notes::load_clips().into_iter().map(HistClip::Text).collect(),
            clip_page: 0,
            outputs: Vec::new(),
            notes: crate::notes::load(),
            note_draft: String::new(),
            note_typing: false,
            note_hi: None,
            dex_bag: false,
            drag_over: false,
            shelf_press: None,
            shelf_dragged: false,
            clip_flash: None,
            dex: crate::dex::Dex::load(),
            wild: None,
            fishing: None,
            fish_rect: None,
            dex_quests: false,
            hearts: Vec::new(),
            hatch: None,
            meeting: None,
            break_at: None,
            sys_by_mem: false,
            sys_page: 0,
            scan_page: 0,
            sys_top: Vec::new(),
            kill_armed: None,
            dex_card: false,
            dex_forms: false,
            dex_shiny: false,
            dex_pages: 1,
            games: crate::games::load(),
            game_child: None,
            dex_page: 0,
            pending_evolution: false,
            xp_pops: Vec::new(),
            pet_at: None,
            last_heart: Instant::now(),
            pet_xp_at: None,
            poke_xp_at: None,
            dex_thumbs: HashMap::new(),
            pending_click: None,
            swallow_up: false,
            mon_rect: None,
            wild_rect: None,
            drawn: [R::new(0.0, 0.0, WIN_W, WIN_H); 2],
            dirty_rect: R::new(0.0, 0.0, WIN_W, WIN_H),
            width_cache: HashMap::new(),
            last: now,
            t0: now,
            dirty: true,
            visible: false,
            fullscreen: false,
            hover: false,
            tracking: false,
            expanded: false,
            buttons: Vec::new(),
            hot: None,
            tab: Tab::Media,
            tab_w: std::array::from_fn(|_| Spring::new(TAB_H, 420.0, 30.0, 0.3)),
            sys: None,
            sys_on: false,
            cpu_hist: VecDeque::new(),
            mem_hist: VecDeque::new(),
            gpu_hist: VecDeque::new(),
            weather: None,
            ctl_dnd: None,
            ctl_dark: false,
            ctl_mic: None,
            brightness: None,
            bright_tx,
            dragging: None,
            track: (0.0, 1.0),
            armed: None,
            user: String::new(),
            osd: None,
            last_vol: None,
            card: None,
            card_since: now,
            card_bmp: None,
            queue: VecDeque::new(),
            media: None,
            playing: false,
            paused_at: None,
            art: None,
            art_bmp: None,
            tint: None,
            timeline: None,
            agents: Vec::new(),
            decisions: Vec::new(),
            wall_key: None,
            wpe_titles: Vec::new(),
            wpe_asked: None,
            wall_accent: None,
            usage: Vec::new(),
            usage_alerted: load_alerts(),
            usage_tx,
            cal: Calendar::new(),
            cal_today: Vec::new(),
            cal_next: None,
            today: calendar::local_now().date,
            clock: calendar::clock_text(),
            audio,
            media_tx,
            rx,
        };
        APP.with(|a| *a.borrow_mut() = Some(app));
        with_app(|a| {
            a.refresh_calendar();
            a.schedule_calendar();
            a.tick_clock();
            // Catch up on evolutions the team's level already allows (no card at start-up).
            let evolved = a.evolve_caught();
            if !evolved.is_empty() {
                util::log(&format!("caught Pokémon evolved at start-up: {evolved:?}"));
            }
            SetTimer(Some(main_hwnd()), T_BREAK, 60_000, None);
            // Stay in front of other apps: check when you switch apps, and every so often.
            SetTimer(Some(main_hwnd()), T_ONTOP, ONTOP_MS, None);
            let _ = windows::Win32::UI::Accessibility::SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(on_foreground),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if !a.cfg.clipboard_history && !a.clips.is_empty() {
                a.clips.clear();
                crate::notes::save_clips(&[]);
            }
            if util::demo() {
                a.dex.visitor = None;
                a.dex.egg = None;
                // A buddy for the trainer card, a little short of evolving.
                if a.dex.buddy.is_empty() {
                    let pick = if a.dex.caught.contains_key("eevee") { Some("eevee".to_string()) } else { a.dex.caught.keys().next().cloned() };
                    if let Some(p) = pick {
                        a.dex.friendship.insert(p.clone(), 140);
                        a.dex.buddy = p;
                    }
                }
                SetTimer(Some(main_hwnd()), T_DEMO, 1500, None);
            }
            a.restore_visitor();
            a.ensure_daily();
            a.check_goals();
            if !util::demo() {
                a.schedule_first_wild();
            }
            a.check_wallpaper();
            a.update_visibility();
            a.sync_views();
        });
        util::log("started");

        let mut msg = MSG::default();
        // A precise timer for frame waits: the default wait rounds to the 15.6 ms system tick,
        // which makes sprite frames show for uneven lengths and the animation stutter.
        let tick = CreateWaitableTimerExW(None, PCWSTR::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, 0x1F0003).ok();
        'outer: loop {
            if with_app(|a| a.any_needs_frames()).unwrap_or(false) {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        break 'outer;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                let started = Instant::now();
                let mut wait = with_app(|a| a.frame_all()).unwrap_or(0);
                if wait == 0 {
                    // Mid-animation: one frame per screen refresh. Present alone no longer
                    // paces this with two islands drawing, so wait for the compositor (and
                    // at most ~120 fps: these screens refresh at 200+ Hz, and every frame of
                    // an animation is drawn in software).
                    let _ = windows::Win32::Graphics::Dwm::DwmFlush();
                    wait = 8u32.saturating_sub(started.elapsed().as_millis() as u32);
                }
                if wait > 0 {
                    // Sleep until the next ambient frame, but wake at once for input/events.
                    let due = -(wait as i64) * 10_000;
                    match tick {
                        Some(h) if SetWaitableTimer(h, &due, 0, None, None, false).is_ok() => {
                            MsgWaitForMultipleObjectsEx(Some(&[h]), wait + 50, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                        }
                        _ => {
                            MsgWaitForMultipleObjectsEx(None, wait, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                        }
                    }
                }
            } else {
                if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        SHAppBarMessage(ABM_REMOVE, &mut abd);
        if let Some(h) = hook {
            let _ = UnhookWindowsHookEx(h);
        }
        APP.with(|a| a.borrow_mut().take());
        util::log("stopped");
        Ok(())
    }
}
