//! Panels beside the island: two small windows at the top of the main screen that show a
//! glance of what's going on. "wings" sit right beside the pill (your Claude and Codex
//! limits and what's next; the song in place of the limits while music plays), "corners"
//! float at the screen's far edges (your team on the left, the PC on the right). They hide
//! while the island is open or busy. A click opens a panel in place, grown into its own
//! small island with what the matching tab shows; it closes when you click it again or move
//! away. "both" shows the wings and the corners together, and between them how hot the PC is
//! running (left: opens a temperature dashboard) and your Bluetooth devices (right: opens
//! each one with its battery). The island leaves out the tabs the panels show.

use super::*;
use crate::devices::{BtDevice, DevKind};
use std::sync::Mutex;
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::AtomicU32;

pub(super) const SIDE_CLASS: PCWSTR = w!("PocketIsleSide");

/// The gap between the island and a wing, and between a corner and the screen's edge.
const GAP: f32 = 10.0;
const EDGE: f32 = 12.0;
/// Room around the surface inside the window (the shoulders curve out past the body).
const MARGIN: f32 = SHOULDER + 2.0;

const CLAUDE: Rgba = Rgba { r: 0.851, g: 0.467, b: 0.341, a: 1.0 };
const CODEX: Rgba = Rgba { r: 0.471, g: 0.667, b: 1.0, a: 1.0 };
const SKY: Rgba = Rgba { r: 0.51, g: 0.745, b: 1.0, a: 1.0 };
const LEAF: Rgba = Rgba { r: 0.478, g: 0.839, b: 0.549, a: 1.0 };
const LILAC: Rgba = Rgba { r: 0.745, g: 0.588, b: 1.0, a: 1.0 };
const RED: Rgba = Rgba { r: 0.922, g: 0.373, b: 0.333, a: 1.0 };
const GOLD: Rgba = Rgba { r: 0.98, g: 0.784, b: 0.353, a: 1.0 };
/// Nerd Font: restart, update.
const G_THERMO: &str = "\u{f050f}";
/// Nerd Font: flash (power), speedometer (clock).
const G_BOLT: &str = "\u{f0241}";
const G_SPEED: &str = "\u{f04c5}";
/// The temperature dashboard's height (its content).
const TEMP_H: f32 = 64.0 + 8.0 + 78.0 + 8.0 + 56.0 + 8.0 + 26.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum SideKind {
    Limits,
    Next,
    Song,
    Team,
    Machine,
    Temp,
    Devices,
}

impl SideKind {
    fn width(self) -> f32 {
        match self {
            SideKind::Limits => 214.0,
            SideKind::Next => 222.0,
            SideKind::Song => 236.0,
            SideKind::Team => 330.0,
            SideKind::Machine => 318.0,
            SideKind::Temp => 118.0,
            SideKind::Devices => 130.0,
        }
    }

    /// The compact size, for when there's little room (the gap panels only).
    fn small_width(self) -> f32 {
        match self {
            SideKind::Temp => 76.0,
            SideKind::Devices => 76.0,
            k => k.width(),
        }
    }

    /// The smallest size, a symbol alone (while a notification widens the island).
    fn tiny_width(self) -> f32 {
        match self {
            SideKind::Temp => 44.0,
            SideKind::Devices => 44.0,
            k => k.small_width(),
        }
    }

    /// How wide it opens: as wide as its tab needs.
    fn open_width(self) -> f32 {
        match self {
            SideKind::Limits => 470.0,
            SideKind::Next => 450.0,
            SideKind::Song => 420.0,
            SideKind::Team | SideKind::Machine => EXP_W,
            SideKind::Temp => 500.0,
            SideKind::Devices => 420.0,
        }
    }

    /// Drawn at the tiny size: `c` is the content box inside the panel's padding.
    fn is_tiny(self, c: R) -> bool {
        c.w() < self.small_width() - 34.0
    }

    /// The panel that shows a tab, if any does.
    pub(super) fn for_tab(tab: Tab) -> Option<SideKind> {
        match tab {
            Tab::Media => Some(SideKind::Song),
            Tab::Usage => Some(SideKind::Limits),
            Tab::Calendar => Some(SideKind::Next),
            Tab::Dex => Some(SideKind::Team),
            Tab::System => Some(SideKind::Machine),
            _ => None,
        }
    }

    fn tab(self) -> Tab {
        match self {
            SideKind::Limits => Tab::Usage,
            SideKind::Next => Tab::Calendar,
            SideKind::Song => Tab::Media,
            SideKind::Team => Tab::Dex,
            SideKind::Machine => Tab::System,
            SideKind::Temp => Tab::System,
            SideKind::Devices => Tab::Power,
        }
    }
}

/// Where a panel goes: its window on screen (physical px), and its surface inside the window
/// (logical px).
#[derive(Clone, Copy, PartialEq)]
struct Spec {
    kind: SideKind,
    win: (i32, i32, i32, i32),
    body: R,
    rad: f32,
    hang: bool,
    /// The height of the glance (the closed panel), which keeps its place while it opens.
    glance_h: f32,
    /// Where the closed panel sits and where the opened one will end up (in the window):
    /// the glance stays put as it fades, and the details are laid out at their final size
    /// (the growing surface uncovers them) so nothing reflows as it opens.
    closed: R,
    full: R,
}

pub(super) struct Side {
    pub(super) hwnd: HWND,
    r: Renderer,
    spec: Option<Spec>,
    size: (i32, i32),
    shown: bool,
    alpha: Spring,
    key: String,
    last: Instant,
    /// Opened in place: how far (0..1), its buttons and the one under the pointer.
    open: Spring,
    buttons: Vec<(R, Btn)>,
    hot: Option<Btn>,
    tracking: bool,
    /// When the pointer came to rest over it: it opens after HOVER_OPEN.
    hover_since: Option<Instant>,
    /// Its details were drawn once, unseen, while the pointer rested on it, so the first
    /// frame that shows them doesn't stall (loading sprites, laying out text).
    warm: bool,
    /// Where its window was last put, to move it only when that changes.
    pos: (i32, i32, i32, i32),
    /// The team's sprite frames last drawn.
    frames: Vec<usize>,
    /// The areas the last two pictures changed (None: all of it). A picture redraws what it
    /// changes plus those, so both swap-chain buffers end up current.
    drew: [Option<R>; 2],
}

/// How long the pointer stays over a panel before it opens by itself.
const HOVER_OPEN: Duration = Duration::from_millis(250);

/// Bluetooth batteries, checked once a minute on a thread of their own (the System tab's
/// sampling only runs while that tab is open).
static BATTERIES: Mutex<Vec<BtDevice>> = Mutex::new(Vec::new());
/// Look again now (the devices panel was opened).
static RESCAN: AtomicBool = AtomicBool::new(false);
static WATCHING: AtomicBool = AtomicBool::new(false);

fn watch_batteries() {
    if WATCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new().name("side-batteries".into()).spawn(|| {
        for n in 0u64.. {
            if n % 20 == 0 || RESCAN.swap(false, Ordering::SeqCst) {
                let list = crate::devices::bluetooth();
                let changed = BATTERIES.lock().map(|mut b| std::mem::replace(&mut *b, list.clone()) != list).unwrap_or(false);
                if changed {
                    events::send(Event::Glance);
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

/// The connected Bluetooth devices (headphones first, then the phone, then the rest): each
/// one's symbol, name and battery, if Windows knows it.
fn devices_list() -> Vec<(&'static str, String, Option<u8>)> {
    if util::demo() {
        return vec![(G_HEADPHONES, "Headphones".into(), Some(80)), (G_PHONE, "Phone".into(), Some(64))];
    }
    let Ok(list) = BATTERIES.lock() else { return Vec::new() };
    let rank = |k: DevKind| match k {
        DevKind::Audio => 0,
        DevKind::Phone => 1,
        _ => 2,
    };
    let mut out: Vec<(u8, &'static str, String, Option<u8>)> = list
        .iter()
        .map(|d| {
            let glyph = match d.kind {
                DevKind::Audio => G_HEADPHONES,
                DevKind::Phone => G_PHONE,
                DevKind::Mouse => G_MOUSE,
                DevKind::Keyboard => G_KEYBOARD,
                DevKind::Controller => G_GAME,
                DevKind::Pen => G_PEN,
                DevKind::Other => G_BLUETOOTH,
            };
            (rank(d.kind), glyph, d.name.clone(), d.pct)
        })
        .collect();
    out.sort_by_key(|g| g.0);
    out.into_iter().map(|(_, g, n, p)| (g, n, p)).collect()
}

/// The devices at a glance: each one's symbol and battery (at most four).
fn gadgets() -> Vec<(&'static str, Option<u8>)> {
    devices_list().into_iter().take(4).map(|(g, _, p)| (g, p)).collect()
}

/// How full a battery is, in a word and a colour.
fn charge_word(pct: u8) -> (&'static str, Rgba) {
    match pct {
        80..=255 => ("Full", LEAF),
        50..=79 => ("Good", LEAF),
        21..=49 => ("Getting low", GOLD),
        _ => ("Charge soon", RED),
    }
}

/// One device's row in the opened devices panel.
const DEV_ROW: f32 = 52.0;

/// The middle square of a picture, shrunk to at most 64 px (averaging the pixels).
pub(super) fn thumb(a: &crate::events::Art) -> crate::events::Art {
    let m = a.w.min(a.h).max(1);
    let n = m.min(64);
    let (ox, oy) = ((a.w - m) / 2, (a.h - m) / 2);
    let mut px = vec![0u8; (n * n * 4) as usize];
    for ty in 0..n {
        let (y0, y1) = (oy + ty * m / n, oy + (ty + 1) * m / n);
        for tx in 0..n {
            let (x0, x1) = (ox + tx * m / n, ox + (tx + 1) * m / n);
            let mut acc = [0u32; 4];
            let mut count = 0u32;
            for y in y0..y1.max(y0 + 1) {
                for x in x0..x1.max(x0 + 1) {
                    let i = ((y * a.w + x) * 4) as usize;
                    if let Some(p) = a.px.get(i..i + 4) {
                        for c in 0..4 {
                            acc[c] += p[c] as u32;
                        }
                        count += 1;
                    }
                }
            }
            let o = ((ty * n + tx) * 4) as usize;
            for c in 0..4 {
                px[o + c] = (acc[c] / count.max(1)) as u8;
            }
        }
    }
    crate::events::Art { w: n, h: n, px, tint: a.tint }
}

/// How wide a device is beside what's next: a symbol and its battery, or the symbol alone.
fn gadget_w(g: &(&'static str, Option<u8>)) -> f32 {
    if g.1.is_some() {
        58.0
    } else {
        26.0
    }
}

/// How quick the internet is, checked every 15 s on a thread of its own while the gaps are
/// shown; the island hears when it changes.
static PING: AtomicU32 = AtomicU32::new(0);
static GLANCING: AtomicBool = AtomicBool::new(false);
static GLANCE_ON: AtomicBool = AtomicBool::new(false);
/// PING when nothing answered.
const OFFLINE: u32 = u32::MAX;

fn watch_glances() {
    if GLANCING.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new().name("side-glances".into()).spawn(|| {
        for n in 0u64.. {
            if GLANCE_ON.load(Ordering::SeqCst) {
                let mut changed = false;
                if n % 15 == 0 {
                    let ms = measure_ping();
                    changed |= PING.swap(ms, Ordering::SeqCst) != ms;
                }
                if changed {
                    events::send(Event::Glance);
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

/// The round trip to a public DNS server, in ms: the quicker of two TCP handshakes (they
/// need no admin rights, unlike ICMP from some setups). OFFLINE when neither answers.
fn measure_ping() -> u32 {
    let targets: [SocketAddr; 2] = [([1, 1, 1, 1], 443).into(), ([8, 8, 8, 8], 443).into()];
    for t in targets {
        let best = (0..2)
            .filter_map(|_| {
                let start = Instant::now();
                TcpStream::connect_timeout(&t, Duration::from_secs(2)).ok().map(|_| start.elapsed().as_millis() as u32)
            })
            .min();
        if let Some(ms) = best {
            return ms.max(1);
        }
    }
    OFFLINE
}

/// The last ping: None before the first one, Some(OFFLINE) when nothing answered.
fn ping_ms() -> Option<u32> {
    if util::demo() {
        return Some(18);
    }
    Some(PING.load(Ordering::SeqCst)).filter(|ms| *ms != 0)
}

pub(super) unsafe extern "system" fn side_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONUP if !util::demo() => {
            with_app(|a| a.with_side(hwnd, |a| a.side_click(hwnd, lp)));
            return LRESULT(0);
        }
        WM_MOUSEMOVE if !util::demo() => {
            with_app(|a| a.with_side(hwnd, |a| a.side_move(hwnd, lp)));
            return LRESULT(0);
        }
        WM_MOUSELEAVE if !util::demo() => {
            with_app(|a| a.with_side(hwnd, |a| a.side_left(hwnd)));
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// A panel grown open to `w` x `h` (its surface, logical px), `k` of the way (0..1). It
/// grows away from its anchored edge: -1 keeps its left edge, 1 its right, 0 its middle.
/// The window takes the full open size at once; the surface grows inside it.
fn grow(p: Spec, anchor: i8, w: f32, h: f32, k: f32, s: f32) -> Spec {
    let m = p.body.x0;
    let (bw, bh) = (p.body.w(), p.body.h());
    let (w, h) = (w.max(bw), h.max(bh));
    let k = k.clamp(0.0, 1.0);
    let (cw, ch) = (bw + (w - bw) * k, bh + (h - bh) * k);
    let ww = ((w + 2.0 * m) * s).round() as i32;
    let wh = ((p.body.y0 + h + 4.0) * s).round() as i32;
    let sx0 = p.win.0 as f32 + m * s;
    let sx1 = p.win.0 as f32 + (m + bw) * s;
    let (x, bx0) = match anchor {
        -1 => (p.win.0, m),
        1 => ((sx1 + m * s).round() as i32 - ww, m + w - cw),
        _ => (((sx0 + sx1) / 2.0 - ww as f32 / 2.0).round() as i32, m + (w - cw) / 2.0),
    };
    let body = R::new(bx0, p.body.y0, bx0 + cw, p.body.y0 + ch);
    let rad = (p.rad + (22.0 - p.rad) * k).min(body.h() / 2.0);
    let closed = R::new(m + (w - bw) * (anchor as f32 + 1.0) / 2.0, p.body.y0, m + (w - bw) * (anchor as f32 + 1.0) / 2.0 + bw, p.body.y0 + bh);
    let full = R::new(m, p.body.y0, m + w, p.body.y0 + h);
    Spec { win: (x, p.win.1, ww, wh), body, rad, closed, full, ..p }
}

/// The panel the demo's tour has opened (it shows them as you'd open them).
static DEMO_SIDE: Mutex<Option<SideKind>> = Mutex::new(None);

/// For checking the opened panels in the demo: POCKETISLE_DEMO_OPEN=<slot> opens one.
fn demo_open() -> Option<usize> {
    util::demo().then(|| std::env::var("POCKETISLE_DEMO_OPEN").ok()?.parse().ok()).flatten()
}

/// "in 12 min", "in 1 h 5 min".
fn minutes_words(m: u32) -> String {
    match m {
        0 => "now".into(),
        m if m < 60 => format!("in {m} min"),
        m if m % 60 == 0 => format!("in {} h", m / 60),
        m => format!("in {} h {} min", m / 60, m % 60),
    }
}

impl App {
    fn side_mode(&self) -> &str {
        match self.cfg.side_panels.as_str() {
            "wings" | "corners" | "both" => self.cfg.side_panels.as_str(),
            _ => "off",
        }
    }

    /// Make (or remove) every screen's panel windows to match the setting. Called at start
    /// and whenever the settings, the screens or the graphics device change.
    pub(super) fn sync_sides(&mut self) {
        self.each_view(|a| a.sync_own_sides());
    }

    /// Run `f` as the island (on whichever screen) that owns the panel window `hwnd`.
    fn with_side<T>(&mut self, hwnd: HWND, f: impl FnOnce(&mut App) -> T) -> T {
        match self.views.iter().find(|v| v.sides.iter().any(|s| s.hwnd == hwnd)).map(|v| v.hwnd) {
            Some(view) => self.with_view(view, f),
            None => f(self),
        }
    }

    /// The panel windows of the island in place (this screen's).
    pub(super) fn sync_own_sides(&mut self) {
        for s in self.sides.drain(..) {
            unsafe {
                let _ = DestroyWindow(s.hwnd);
            }
        }
        if self.side_mode() == "off" {
            return;
        }
        if !util::demo() {
            watch_batteries();
            GLANCE_ON.store(self.side_mode() == "both", Ordering::SeqCst);
            if self.side_mode() == "both" {
                watch_glances();
            }
        }
        self.side_next = self.cal.upcoming(&calendar::local_now(), 24 * 60);
        let hinst: HINSTANCE = unsafe { GetModuleHandleW(None) }.map(|h| h.into()).unwrap_or_default();
        let count = if self.side_mode() == "both" { 6 } else { 2 };
        for _ in 0..count {
            let made = unsafe {
                CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP,
                    SIDE_CLASS,
                    w!("Pocket Isle"),
                    WS_POPUP,
                    0,
                    0,
                    8,
                    8,
                    None,
                    None,
                    Some(hinst),
                    None,
                )
            };
            let Ok(hwnd) = made else {
                util::log("side panel window failed");
                continue;
            };
            match self.r.sibling(hwnd, 8, 8, self.scale) {
                Ok(r) => self.sides.push(Side {
                    hwnd,
                    r,
                    spec: None,
                    size: (8, 8),
                    shown: false,
                    alpha: Spring::new(0.0, 260.0, 30.0, 0.01),
                    key: String::new(),
                    last: Instant::now(),
                    open: Spring::new(0.0, 520.0, 47.0, 0.002),
                    buttons: Vec::new(),
                    hot: None,
                    tracking: false,
                    hover_since: None,
                    warm: false,
                    pos: (0, 0, 0, 0),
                    frames: Vec::new(),
                    drew: [None, None],
                }),
                Err(e) => {
                    util::log(&format!("side panel renderer failed: {e}"));
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            }
        }
        self.dirty = true;
    }

    /// The song shows in the wing beside this island (on its screen), so the pill doesn't
    /// repeat it.
    pub(super) fn song_beside(&self) -> bool {
        !self.sides.is_empty() && matches!(self.side_mode(), "wings" | "both") && self.sides_wanted() && self.song_on()
    }

    /// Music is playing, or was paused a moment ago (a skip or a seek pauses briefly).
    fn song_on(&self) -> bool {
        self.media.is_some() && (self.playing || self.paused_at.is_some_and(|t| t.elapsed() < PAUSE_GRACE))
    }

    /// What the panels show right now: the wings (left, right), then the corners (left,
    /// right); None hides one.
    fn has_limits(&self) -> bool {
        self.usage.iter().any(|u| !u.limits.is_empty() && u.error.as_deref() != Some(usage::SIGNED_OUT))
    }

    /// A side panel shows this tab (opened in place), so the island beside it
    /// leaves it out. Usage stays while there are no limits to show beside it (signed out).
    pub(super) fn tab_on_side(&self, tab: Tab) -> bool {
        if self.sides.is_empty() {
            return false;
        }
        let wings = matches!(self.side_mode(), "wings" | "both");
        let corners = matches!(self.side_mode(), "corners" | "both");
        match tab {
            Tab::Usage => wings && self.has_limits(),
            Tab::Calendar | Tab::Media => wings,
            Tab::Dex | Tab::System => corners,
            _ => false,
        }
    }

    fn side_kinds(&self) -> Vec<Option<SideKind>> {
        let has_limits = self.has_limits();
        let wings = if self.song_on() {
            vec![Some(SideKind::Song), Some(SideKind::Next)]
        } else {
            vec![has_limits.then_some(SideKind::Limits), Some(SideKind::Next)]
        };
        let corners = vec![Some(SideKind::Team), Some(SideKind::Machine)];
        // Between them: how hot it's running, and your Bluetooth devices.
        let gaps = vec![Some(SideKind::Temp), Some(SideKind::Devices)];
        match self.side_mode() {
            "wings" => wings,
            "corners" => corners,
            "both" => [wings, corners, gaps].concat(),
            _ => Vec::new(),
        }
    }

    /// The panels show beside the pill, and stay (moving aside) while a notification card
    /// shows; not while it's open or asking something, or hidden for a full-screen app.
    fn sides_wanted(&self) -> bool {
        self.visible && matches!(self.mode, Mode::Compact | Mode::Card) && !self.expanded && self.osd.is_none() && self.scrub.is_none() && self.decisions.is_empty()
    }

    /// The corners want the PC's numbers sampled.
    pub(super) fn sides_want_sys(&self) -> bool {
        matches!(self.side_mode(), "corners" | "both") && self.sides_wanted()
    }

    fn side_spec(&self, i: usize, kind: SideKind) -> Option<Spec> {
        let s = self.scale;
        let mut wr = RECT::default();
        unsafe { GetWindowRect(self.hwnd, &mut wr) }.ok()?;
        let (island, rad, _) = self.body();
        let attach = self.cfg.attach_to_top;
        let w = kind.width();
        let bottom = self.neck + self.pill_h;
        let px = |v: f32| (v * s).round() as i32;
        match kind {
            SideKind::Team | SideKind::Machine => {
                let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                unsafe {
                    let _ = GetMonitorInfoW(self.monitor, &mut mi);
                }
                let m = 3.0;
                let (y0, y1) = if attach { ((self.neck * 0.6).max(3.0), bottom - 2.0) } else { (0.0, self.pill_h) };
                let body = R::new(m, y0, m + w, y1);
                let (ww, wh) = (px(w + 2.0 * m), px(y1 + m));
                let x = if i == 0 { mi.rcMonitor.left + px(EDGE - m) } else { mi.rcMonitor.right - px(EDGE - m) - ww };
                Some(Spec { kind, win: (x, wr.top, ww, wh), body, rad: body.h() / 2.0, hang: false, glance_h: body.h(), closed: body, full: body })
            }
            _ => {
                let body = if attach { R::new(MARGIN, 0.0, MARGIN + w, bottom) } else { R::new(MARGIN, 0.0, MARGIN + w, self.pill_h) };
                let (ww, wh) = (px(w + 2.0 * MARGIN), px(body.y1 + 3.0));
                let x = if i == 0 { wr.left + px(island.x0 - GAP - w - MARGIN) } else { wr.left + px(island.x1 + GAP - MARGIN) };
                Some(Spec { kind, win: (x, wr.top, ww, wh), body, rad: rad.min(body.h() / 2.0), hang: attach, glance_h: body.h(), closed: body, full: body })
            }
        }
    }

    /// A small capsule in the middle of the space between a wing (or the pill) and a corner;
    /// None when it wouldn't fit.
    fn gap_spec(&self, left: usize, kind: SideKind, wing: Option<Spec>, corner: Option<Spec>) -> Option<Spec> {
        let s = self.scale;
        let corner = corner?;
        let inner = |p: &Spec| p.win.0 as f32 + p.body.x0 * s;
        let outer = |p: &Spec| p.win.0 as f32 + p.body.x1 * s;
        let mut wr = RECT::default();
        unsafe { GetWindowRect(self.hwnd, &mut wr) }.ok()?;
        let island = self.body().0;
        let (a, b) = if left == 0 {
            (outer(&corner), wing.map(|w| inner(&w)).unwrap_or(wr.left as f32 + island.x0 * s))
        } else {
            (wing.map(|w| outer(&w)).unwrap_or(wr.left as f32 + island.x1 * s), inner(&corner))
        };
        // Full size if it fits, else a compact one (the pill is wider while Claude works).
        let room = (b - a) / s - GAP;
        let w = self.gap_widths(kind).into_iter().find(|w| *w <= room)?;
        let m = 3.0;
        let body = R::new(m, corner.body.y0, m + w, corner.body.y1);
        let ww = ((w + 2.0 * m) * s).round() as i32;
        let x = ((a + b) / 2.0 - ww as f32 / 2.0).round() as i32;
        Some(Spec { kind, win: (x, corner.win.1, ww, corner.win.3), body, rad: body.h() / 2.0, hang: false, glance_h: body.h(), closed: body, full: body })
    }

    /// A panel is fading, or shows something that moves (the music's bars).
    pub(super) fn sides_busy(&self) -> bool {
        self.side_open.is_some() || self.sides.iter().any(|s| s.hover_since.is_some() || !s.alpha.settled() || !s.open.settled() || (s.shown && s.spec.is_some_and(|p| p.kind == SideKind::Song) && self.playing && self.cfg.animate_equalizer) || (s.shown && s.spec.is_some_and(|p| p.kind == SideKind::Team) && self.team_moves()))
    }

    /// Draw the panels that changed; how long until they want another frame (None = not
    /// until something changes).
    pub(super) fn frame_sides(&mut self, dirty: bool) -> Option<u32> {
        if self.sides.is_empty() {
            return None;
        }
        let wanted = self.sides_wanted();
        let kinds = self.side_kinds();
        if let Some(i) = demo_open() {
            self.side_open = kinds.get(i).copied().flatten().map(|k| (i, k));
        } else if util::demo() {
            // The tour's panel, once it's beside the island (the limits wait for the song to
            // go).
            let want = DEMO_SIDE.lock().ok().and_then(|k| *k);
            let at = want.and_then(|k| kinds.iter().position(|x| *x == Some(k)).map(|i| (i, k)));
            if at != self.side_open {
                match at {
                    Some((i, k)) if wanted => self.open_side(i, k),
                    Some(_) => {}
                    None => self.side_open = None,
                }
            }
        }
        // An opened panel closes when the island gets busy, when what it shows changes, or a
        // moment after the pointer leaves it. Where the pointer is gets checked here: Windows
        // says it left when the panel grows under a pointer that hasn't moved.
        if let Some((j, _)) = self.side_open.filter(|_| !util::demo()) {
            let mut pt = POINT::default();
            let mut wr = RECT::default();
            let over = unsafe { GetCursorPos(&mut pt).is_ok() && GetWindowRect(self.sides[j].hwnd, &mut wr).is_ok() }
                && (wr.left..wr.right).contains(&pt.x)
                && (wr.top..wr.bottom).contains(&pt.y);
            if over {
                self.side_close_at = None;
            } else if self.side_close_at.is_none() {
                self.side_close_at = Some(Instant::now() + Duration::from_millis(700));
            }
        }
        if !wanted || self.side_close_at.is_some_and(|t| Instant::now() >= t) {
            self.side_open = None;
            self.side_close_at = None;
        }
        if let Some((j, k)) = self.side_open {
            if kinds.get(j).copied().flatten() != Some(k) {
                self.side_open = None;
            }
        }
        let t = self.t0.elapsed().as_secs_f32();
        let mut wait: Option<u32> = None;
        // Resting the pointer on a panel opens it.
        for i in 0..self.sides.len() {
            let Some(since) = self.sides[i].hover_since else { continue };
            let left = HOVER_OPEN.saturating_sub(since.elapsed());
            if !left.is_zero() {
                wait = Some(wait.unwrap_or(u32::MAX).min(left.as_millis() as u32));
                continue;
            }
            self.sides[i].hover_since = None;
            let kind = self.sides[i].spec.map(|p| p.kind);
            if let Some(kind) = kind.filter(|_| wanted && self.sides[i].shown && self.side_open.map(|(j, _)| j) != Some(i)) {
                self.open_side(i, kind);
            }
        }
        let mut specs: Vec<Option<Spec>> = Vec::new();
        for i in 0..self.sides.len() {
            let spec = if !wanted {
                None
            } else if i >= 4 {
                kinds.get(i).copied().flatten().and_then(|k| self.gap_spec(i % 2, k, specs[i - 4], specs[i - 2]))
            } else {
                kinds.get(i).copied().flatten().and_then(|k| self.side_spec(i % 2, k))
            };
            specs.push(spec);
        }
        // Step the opening, grow the opened panel (or the one still closing), and fade out
        // the neighbours it would cover.
        let now = Instant::now();
        let mut dts = Vec::new();
        for i in 0..self.sides.len() {
            // A slow frame moves it on a little rather than jumping (which reads as a stutter).
            let dt = (now - self.sides[i].last).as_secs_f32().clamp(0.0, 1.0 / 50.0);
            self.sides[i].last = now;
            dts.push(dt);
            let open = self.side_open.is_some_and(|(j, _)| j == i) && specs[i].is_some();
            self.sides[i].open.set(if open { 1.0 } else { 0.0 });
            self.sides[i].open.step(dt);
            if specs[i].is_none() {
                self.sides[i].open = Spring::new(0.0, 520.0, 47.0, 0.002);
            }
        }
        let s = self.scale;
        let mut grown: Vec<Option<Spec>> = vec![None; self.sides.len()];
        let mut cut = vec![false; self.sides.len()];
        for i in 0..self.sides.len() {
            let Some(p) = specs[i] else { continue };
            let k = self.sides[i].open.v;
            if self.sides[i].open.t == 0.0 && k < 0.002 {
                continue;
            }
            let (w, h) = self.open_size(&p);
            let anchor = match i {
                0 | 3 => 1,
                1 | 2 => -1,
                _ => 0,
            };
            let now_ = grow(p, anchor, w, h, k, s);
            grown[i] = Some(now_);
            // A neighbour goes the moment the growing panel reaches it (it would only be
            // covered), and fades back once it has shrunk away again.
            let (fx0, fx1) = (now_.win.0 as f32 + now_.body.x0 * s, now_.win.0 as f32 + now_.body.x1 * s);
            for (j, q) in specs.clone().into_iter().enumerate() {
                if let Some(q) = q.filter(|_| j != i) {
                    let (a, b) = (q.win.0 as f32 + q.body.x0 * s, q.win.0 as f32 + q.body.x1 * s);
                    if a < fx1 + GAP * s && b > fx0 - GAP * s {
                        specs[j] = None;
                        cut[j] = true;
                    }
                }
            }
        }
        for i in 0..self.sides.len() {
            if grown[i].is_some() && specs[i].is_some() {
                specs[i] = grown[i];
            }
        }
        for i in 0..self.sides.len() {
            let spec = specs[i];
            let mut side = self.sides.remove(i);
            let dt = dts[i];
            if cut[i] {
                side.alpha = Spring::new(0.0, 260.0, 30.0, 0.01);
            }
            // A different panel fades the old one out first.
            let swap = matches!((spec, side.spec), (Some(a), Some(b)) if a.kind != b.kind);
            side.alpha.set(if spec.is_some() && !swap { 1.0 } else { 0.0 });
            side.alpha.step(dt);
            if spec.is_some() && (side.spec.is_none() || side.alpha.v < 0.02) {
                side.spec = spec;
            } else if let (Some(a), Some(b)) = (spec, side.spec) {
                if a.kind == b.kind {
                    side.spec = spec;
                }
            }
            if side.alpha.v < 0.01 && side.alpha.t == 0.0 {
                if side.shown {
                    unsafe {
                        let _ = ShowWindow(side.hwnd, SW_HIDE);
                    }
                    side.shown = false;
                    side.key.clear();
                }
                if !side.alpha.settled() {
                    wait = Some(0);
                }
                self.sides.insert(i, side);
                continue;
            }
            let Some(p) = side.spec else {
                self.sides.insert(i, side);
                continue;
            };
            let (x, y, ww, wh) = p.win;
            if (ww, wh) != side.size {
                if let Err(e) = side.r.resize(ww as u32, wh as u32, self.scale) {
                    util::log(&format!("side panel resize failed: {e}"));
                }
                side.size = (ww, wh);
                side.key.clear();
            }
            if !side.shown || side.pos != p.win {
                unsafe {
                    let _ = SetWindowPos(side.hwnd, Some(HWND_TOPMOST), x, y, ww, wh, SWP_NOACTIVATE | if side.shown { SWP_NOZORDER } else { SWP_SHOWWINDOW });
                }
                side.pos = p.win;
            }
            side.shown = true;
            // Again once it has opened (its page is set up then), while it has barely moved.
            let warm = (side.hover_since.is_some() || side.open.t > 0.0) && !side.warm && side.open.v < 0.3;
            let opened = side.open.v > 0.002;
            let team = if p.kind == SideKind::Team && side.open.v < 0.5 { self.team_next_frame() } else { None };
            let moving = (p.kind == SideKind::Song && self.playing && self.cfg.animate_equalizer) || opened;
            let key = format!("{:?}{:.3}{}{:?}", p.kind, side.alpha.v, self.side_key(p.kind), side.hot);
            let frames = if team.is_some() { self.team_frames() } else { Vec::new() };
            let changed = dirty || moving || warm || key != side.key;
            if changed || frames != side.frames {
                // Only the sprites moved: redraw and present just their strip (much less work
                // for the software rasteriser, and a smaller present that holds up the island
                // less), once both buffers hold the rest of the panel.
                // What this picture changes: everything when what it shows changed, otherwise
                // just the panel's surface while it grows (the window is already its opened
                // size) or the team's sprites. Redrawing and presenting only that is much less
                // work for the software rasteriser, and holds up the island less.
                let whole_change = changed && !(moving && !dirty && !warm && key == side.key) || side.alpha.v < 0.999;
                let need = if whole_change {
                    None
                } else if moving {
                    let b = p.body;
                    Some(R::new(b.x0 - SHOULDER - 4.0, b.y0 - 4.0, b.x1 + SHOULDER + 4.0, b.y1 + 4.0))
                } else {
                    Some(team_strip(&p, self.side_party().len()))
                };
                let area = match (need, side.drew[0], side.drew[1]) {
                    (Some(a), Some(b), Some(c)) => Some(a.union(&b).union(&c)),
                    _ => None,
                };
                side.drew = [need, side.drew[0]];
                side.frames = frames;
                side.warm |= warm;
                side.key = key;
                std::mem::swap(&mut self.r, &mut side.r);
                std::mem::swap(&mut self.buttons, &mut side.buttons);
                std::mem::swap(&mut self.hot, &mut side.hot);
                self.buttons.clear();
                let whole = R::new(0.0, 0.0, ww as f32 / self.scale, wh as f32 / self.scale);
                let full = area.unwrap_or(whole);
                self.r.begin(full);
                self.draw_side(&p, side.alpha.v, t, side.open.v, warm);
                let res = self.r.end(full);
                std::mem::swap(&mut self.r, &mut side.r);
                std::mem::swap(&mut self.buttons, &mut side.buttons);
                std::mem::swap(&mut self.hot, &mut side.hot);
                if let Err(e) = res {
                    util::log(&format!("side panel present failed: {e}"));
                    self.device_lost = true;
                }
            }
            let next = if !side.alpha.settled() || !side.open.settled() {
                Some(0)
            } else if opened {
                Some(50)
            } else if let Some(ms) = team {
                Some(ms)
            } else if moving {
                Some(42)
            } else {
                None
            };
            wait = match (wait, next) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            self.sides.insert(i, side);
        }
        wait
    }

    /// What a panel's picture depends on (beyond its kind and fade), to redraw only on change.
    fn side_key(&self, kind: SideKind) -> String {
        match kind {
            SideKind::Limits => self.usage.iter().map(|u| format!("{}{:?}", u.provider, u.limits.first().map(|l| (l.used * 100.0).round()))).collect(),
            SideKind::Next => format!("{:?}{:?}", self.side_next, self.cal_next.as_ref().map(|c| &c.0)),
            SideKind::Song => format!("{:?}{}", self.media.as_ref().map(|m| (&m.title, &m.artist)), self.art_bmp.is_some()),
            SideKind::Team => format!("{}{}{:?}", self.dex.xp, self.dex.daily.list.iter().filter(|c| c.done()).count(), self.side_party()),
            SideKind::Temp => format!("{:?}", self.sys.as_ref().and_then(|s| s.gpu.as_ref()).and_then(|g| g.temp)),
            SideKind::Devices => format!("{:?}", devices_list()),
            SideKind::Machine => match &self.sys {
                Some(s) => format!("{:.0}{}{:?}{:?}", s.cpu * 100.0, s.mem_used >> 24, s.gpu.as_ref().map(|g| (g.util * 100.0).round()), self.ping_now()),
                None => "-".into(),
            },
        }
    }

    pub(super) fn side_click(&mut self, hwnd: HWND, lp: LPARAM) {
        let Some(i) = self.sides.iter().position(|s| s.hwnd == hwnd) else { return };
        let Some(kind) = self.sides[i].spec.map(|p| p.kind) else { return };
        let (x, y) = self.logical(lp);
        if self.side_open.is_some_and(|(j, _)| j == i) {
            // Open: a button does what it does on the island; anywhere else closes it.
            match self.sides[i].buttons.iter().find(|(r, _)| r.contains(x, y)).map(|(_, b)| *b) {
                Some(b) => self.press(b),
                None => self.side_open = None,
            }
            self.sides[i].key.clear();
            self.dirty = true;
            return;
        }
        self.sides[i].hover_since = None;
        self.open_side(i, kind);
    }

    /// The demo's tour: open a panel (the island closes; cards make way).
    pub(super) fn demo_panel(&mut self, kind: SideKind) {
        self.expanded = false;
        self.card = None;
        self.queue.clear();
        if let Ok(mut k) = DEMO_SIDE.lock() {
            *k = Some(kind);
        }
        self.dirty = true;
    }

    /// The demo's tour: close its panel.
    pub(super) fn demo_panel_off(&mut self) {
        if let Ok(mut k) = DEMO_SIDE.lock() {
            *k = None;
        }
        self.side_open = None;
    }

    /// Open a panel in place, fetching what its tab shows (the island itself stays as it is).
    fn open_side(&mut self, i: usize, kind: SideKind) {
        self.side_open = Some((i, kind));
        self.sides[i].warm = false;
        self.side_close_at = None;
        if kind == SideKind::Devices {
            RESCAN.store(true, Ordering::SeqCst);
            crate::devices::ask_soon();
        } else {
            let old = self.tab;
            self.tab = kind.tab();
            self.enter_tab();
            self.tab = old;
        }
        self.dirty = true;
    }

    /// The pointer over a panel: light up the button under it, and keep an opened one open.
    pub(super) fn side_move(&mut self, hwnd: HWND, lp: LPARAM) {
        let (x, y) = self.logical(lp);
        let Some(i) = self.sides.iter().position(|s| s.hwnd == hwnd) else { return };
        if !self.sides[i].tracking {
            let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            unsafe {
                let _ = TrackMouseEvent(&mut tme);
            }
            self.sides[i].tracking = true;
            self.sides[i].hover_since = Some(Instant::now());
            self.sides[i].warm = false;
        }
        if self.side_open.is_some_and(|(j, _)| j == i) {
            self.side_close_at = None;
        }
        let hot = self.sides[i].buttons.iter().find(|(r, _)| r.contains(x, y)).map(|(_, b)| *b);
        if hot != self.sides[i].hot {
            self.sides[i].hot = hot;
            self.dirty = true;
        }
    }

    /// The pointer left a panel: an opened one closes a moment later (unless it comes back).
    pub(super) fn side_left(&mut self, hwnd: HWND) {
        let Some(i) = self.sides.iter().position(|s| s.hwnd == hwnd) else { return };
        self.sides[i].tracking = false;
        self.sides[i].hot = None;
        self.sides[i].hover_since = None;
        if self.side_open.is_some_and(|(j, _)| j == i) {
            self.side_close_at = Some(Instant::now() + Duration::from_millis(700));
        }
        self.dirty = true;
    }

    /// How big a panel grows when opened: its tab's width, and the height of what the tab
    /// shows (below the neck when it hangs from the top).
    fn open_size(&mut self, p: &Spec) -> (f32, f32) {
        let content = if p.kind == SideKind::Temp {
            TEMP_H
        } else if p.kind == SideKind::Devices {
            let n = devices_list().len() as f32;
            if n == 0.0 { 40.0 } else { n * (DEV_ROW + 8.0) + 22.0 }
        } else {
            let old = self.tab;
            self.tab = p.kind.tab();
            let h = self.expanded_height() - (12.0 + TAB_H + 12.0 + 12.0);
            self.tab = old;
            h
        };
        let top = if p.hang { self.neck } else { 0.0 };
        (p.kind.open_width(), top + 14.0 + content + 16.0)
    }

    /// What an opened panel shows: its tab's page.
    fn side_details(&mut self, kind: SideKind, x0: f32, x1: f32, y: f32) {
        if kind == SideKind::Temp {
            return self.draw_temp_dash(x0, x1, y);
        }
        if kind == SideKind::Devices {
            return self.draw_devices(x0, x1, y);
        }
        match kind.tab() {
            Tab::Usage => self.draw_usage_tab(x0, x1, y),
            Tab::Calendar => self.draw_calendar_tab(x0, x1, y),
            Tab::Media => self.draw_media_tab(x0, x1, y),
            Tab::Dex => self.draw_dex_tab(x0, x1, y),
            Tab::System => self.draw_system_tab(x0, x1, y),
            _ => {}
        }
    }

    /// One panel's surface (like the island's) and what's on it.
    fn draw_side(&mut self, p: &Spec, alpha: f32, t: f32, k: f32, warm: bool) {
        let th = self.theme.clone();
        // Big enough for an opened panel (the Pokédex is taller than 400).
        let whole = R::new(0.0, 0.0, 4000.0, 2000.0);
        let Some(fade) = self.r.island(whole, 0.0, 0.0, false) else { return };
        let Some(shape) = self.r.island(p.body, p.rad, if p.hang { SHOULDER } else { 0.0 }, p.hang) else { return };
        // Layers only where something fades or must be clipped: each one is a whole extra
        // pass in the software rasteriser, and these redraw every frame while they move.
        let faded = alpha < 0.999;
        if faded {
            self.r.push_shape(&fade, alpha);
        }
        let mood = self.mood();
        self.r.fill_vertical(&shape, p.body.y0, p.body.y1, th.bg.mix(mood, 0.05), th.bg.mix(mood, 0.2));
        if k < 0.45 {
            // The glance sits in the middle of the whole closed panel you see (hanging from
            // the top edge too, rather than below the neck, where it looked low). It stays
            // inside the closed panel, so it only needs a layer to fade.
            let o = 1.0 - k / 0.45;
            if o < 0.999 {
                self.r.push_shape(&shape, o);
            }
            let c = R::new(p.closed.x0 + 14.0, p.closed.y0, p.closed.x1 - 14.0, p.closed.y0 + p.glance_h);
            match p.kind {
                SideKind::Limits => self.side_limits(c),
                SideKind::Next => self.side_next_up(c),
                SideKind::Song => self.side_song(c, t),
                SideKind::Team => self.side_team(c),
                SideKind::Machine => self.side_machine(c),
                SideKind::Temp => self.side_temp(c),
                SideKind::Devices => self.side_devices(c),
            }
            if o < 0.999 {
                self.r.pop();
            }
        }
        let y = p.full.y0 + if p.hang { self.neck } else { 0.0 } + 14.0;
        if warm {
            // Unseen: readies what the details draw before the panel opens.
            let n = self.buttons.len();
            // (Faint rather than hidden: a hidden layer isn't drawn at all.)
            self.r.push_shape(&shape, 0.01);
            self.side_details(p.kind, p.full.x0 + 16.0, p.full.x1 - 16.0, y);
            self.r.pop();
            self.buttons.truncate(n);
        }
        if k > 0.55 {
            // Clipped to the surface while it grows over them; plain once it's open.
            let o = ((k - 0.55) / 0.4).min(1.0);
            let layer = o < 0.999 || k < 0.999;
            if layer {
                self.r.push_shape(&shape, o);
            }
            self.side_details(p.kind, p.full.x0 + 16.0, p.full.x1 - 16.0, y);
            if layer {
                self.r.pop();
            }
        }
        let rim = th.border.mix(mood, 0.5);
        let rim_top = if p.hang { rim.a(0.0) } else { rim.a(0.55) };
        self.r.stroke_vertical(&shape, p.body.y0, p.body.y1, rim_top, rim.a(0.9), 1.0);
        if faded {
            self.r.pop();
        }
    }

    /// A line of text centred on `cy`.
    fn side_text(&mut self, s: &str, x0: f32, x1: f32, cy: f32, size: f32, weight: u32, c: Rgba, align: Align) {
        self.r.text(s, R::new(x0, cy - 10.0, x1, cy + 10.0), size, weight, c, align);
    }

    fn side_bar(&mut self, x0: f32, x1: f32, cy: f32, frac: f32, c: Rgba) {
        let th = self.theme.clone();
        self.r.fill_round(R::new(x0, cy - 2.0, x1, cy + 2.0), 2.0, th.bg2);
        if frac > 0.0 {
            self.r.fill_round(R::new(x0, cy - 2.0, x0 + ((x1 - x0) * frac.min(1.0)).max(4.0), cy + 2.0), 2.0, c);
        }
    }

    /// Claude's mark: eight short rays.
    fn claude_mark(&mut self, cx: f32, cy: f32, r: f32, c: Rgba) {
        let segs: Vec<(f32, f32, f32, f32)> = (0..4)
            .map(|i| {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                (cx - a.cos() * r, cy - a.sin() * r, cx + a.cos() * r, cy + a.sin() * r)
            })
            .collect();
        self.r.lines(&segs, c, 1.9);
    }

    fn side_limits(&mut self, c: R) {
        let th = self.theme.clone();
        let (l1, l2) = (c.cy() - 7.0, c.cy() + 8.0);
        let list: Vec<(String, f32)> = self
            .usage
            .iter()
            .filter(|u| !u.limits.is_empty() && u.error.as_deref() != Some(usage::SIGNED_OUT))
            .take(2)
            .map(|u| (u.provider.to_string(), u.limits[0].used))
            .collect();
        let first_claude = list.first().is_some_and(|(n, _)| n == "Claude");
        let x0 = if first_claude {
            self.claude_mark(c.x0 + 6.0, c.cy(), 6.0, CLAUDE);
            c.x0 + 20.0
        } else {
            c.x0
        };
        let n = list.len().max(1) as f32;
        let gap = 14.0;
        let cw = (c.x1 - x0 - gap * (n - 1.0)) / n;
        for (i, (name, used)) in list.iter().enumerate() {
            let lx = x0 + i as f32 * (cw + gap);
            let col = if name == "Claude" { CLAUDE } else if name == "Codex" { CODEX } else { th.accent };
            self.side_text(name, lx, lx + cw, l1, 11.5, 600, th.text, Align::Left);
            self.side_bar(lx, lx + cw - 34.0, l2, *used, col);
            let pcol = if *used >= self.cfg.usage_watch { self.level_color(*used) } else { th.subtext };
            self.side_text(&format!("{}%", (used * 100.0).round()), lx + cw - 34.0, lx + cw, l2, 10.5, 500, pcol, Align::Right);
        }
    }

    fn side_next_up(&mut self, c: R) {
        let th = self.theme.clone();
        let (l1, l2) = (c.cy() - 7.0, c.cy() + 8.0);
        self.r.icon(G_CAL, R::new(c.x0 - 2.0, c.y0, c.x0 + 18.0, c.y1), 15.0, SKY);
        let tx1 = c.x1;
        let (title, when) = match (&self.side_next, &self.cal_next) {
            (Some((title, mins)), _) => (title.clone(), minutes_words(*mins)),
            (None, Some((title, d))) => {
                let days = d.days() - self.today.days();
                (title.clone(), if days == 1 { "Tomorrow".to_string() } else { d.short() })
            }
            (None, None) => ("Nothing on".to_string(), "the rest of today".to_string()),
        };
        self.side_text(&title, c.x0 + 26.0, tx1 - 6.0, l1, 11.5, 600, th.text, Align::Left);
        self.side_text(&when, c.x0 + 26.0, tx1 - 6.0, l2, 10.5, 400, th.subtext, Align::Left);
    }

    fn side_song(&mut self, c: R, t: f32) {
        let th = self.theme.clone();
        let (l1, l2) = (c.cy() - 7.0, c.cy() + 8.0);
        let art = R::new(c.x0 - 4.0, c.cy() - 14.0, c.x0 + 24.0, c.cy() + 14.0);
        match self.art_thumb.clone().or_else(|| self.art_bmp.clone()) {
            Some(b) => self.r.image(&b, art, 7.0, 1.0),
            None => {
                self.r.fill_round(art, 7.0, th.bg2);
                self.r.icon(G_MUSIC, art, 13.0, self.tint.unwrap_or(th.accent));
            }
        }
        let (title, artist) = self.media.as_ref().map(|m| (m.title.clone(), m.artist.clone())).unwrap_or_default();
        let tx1 = c.x1 - 34.0;
        self.side_text(&title, c.x0 + 32.0, tx1, l1, 11.5, 600, th.text, Align::Left);
        self.side_text(&artist, c.x0 + 32.0, tx1, l2, 10.5, 400, th.subtext, Align::Left);
        // Five bars in the album's colours, moving with the music.
        let (c1, c2) = self.music_colours();
        let lvl = self.peak.v.clamp(0.0, 1.0);
        let go = self.playing && self.cfg.animate_equalizer;
        for i in 0..5 {
            let amp = if go {
                let wobble = 0.5 + 0.5 * (t * (5.5 + i as f32 * 1.1) + i as f32 * 1.9).sin();
                (0.25 + 0.75 * lvl) * (0.4 + 0.6 * wobble)
            } else {
                0.2
            };
            let h = 4.0 + 14.0 * amp;
            let x = c.x1 - 26.0 + i as f32 * 5.5;
            self.r.fill_round(R::new(x, c.cy() - h / 2.0, x + 3.0, c.cy() + h / 2.0), 1.5, c1.mix(c2, i as f32 / 4.0));
        }
    }

    /// A team member's animated sprite for the panel: (bitmap, this frame's source, width,
    /// height). Loaded and sized once per Pokémon.
    fn team_frame(&mut self, name: &str) -> Option<(ID2D1Bitmap1, R, f32, f32)> {
        const BOX: f32 = 28.0;
        let stale = match self.side_strips.get(name) {
            None => true,
            Some(Some(sp)) => sp.scale != self.scale,
            Some(None) => false,
        };
        if stale {
            let s = self.load_sprite_sized(name, BOX, 30.0);
            self.side_strips.insert(name.to_string(), s);
        }
        let sp = self.side_strips.get(name)?.as_ref()?;
        let i = if self.cfg.animate_equalizer { pokemon::frame_at(&sp.delays, team_clock()) } else { 0 };
        Some((sp.bmp.clone(), R::xywh(i as f32 * sp.fw, 0.0, sp.fw, sp.fh), sp.dw, sp.dh))
    }

    /// The frame each team member shows now (redrawn only when one changes).
    fn team_frames(&self) -> Vec<usize> {
        if !self.cfg.animate_equalizer {
            return Vec::new();
        }
        let ms = team_clock();
        self.side_party().iter().filter_map(|n| self.side_strips.get(n)?.as_ref()).map(|s| pokemon::frame_at(&s.delays, ms)).collect()
    }

    /// The team's sprites move (only those that have more than one frame).
    fn team_moves(&self) -> bool {
        self.cfg.animate_equalizer && self.side_party().iter().any(|n| self.side_strips.get(n).is_some_and(|s| s.as_ref().is_some_and(|s| s.delays.len() > 1)))
    }

    /// Milliseconds until one of the team's sprites shows its next frame (at most 20 a
    /// second: six sprites with their own timings would otherwise redraw it every refresh).
    fn team_next_frame(&self) -> Option<u32> {
        if !self.cfg.animate_equalizer {
            return None;
        }
        let ms = now_ms();
        self.side_party()
            .iter()
            .filter_map(|n| self.side_strips.get(n)?.as_ref())
            .filter(|s| s.delays.len() > 1)
            .map(|s| pokemon::ms_to_next_frame(&s.delays, ms).clamp(TEAM_FRAME_MS, 1000) as u32)
            .min()
    }

    /// The team as the Battle tab shows it: each Pokémon at the stage that fits your level.
    fn side_party(&self) -> Vec<String> {
        let lv = self.dex.level();
        self.party(false).iter().map(|n| pokemon::stage_at(n, lv)).collect()
    }

    fn side_team(&mut self, c: R) {
        let th = self.theme.clone();
        let (l1, l2) = (c.cy() - 7.0, c.cy() + 8.0);
        let mut x = c.x0 - 2.0;
        let s = self.scale;
        let snap = |v: f32| (v * s).round() / s;
        for name in self.side_party() {
            if let Some((bmp, src, dw, dh)) = self.team_frame(&name) {
                let d = R::xywh(snap(x + (30.0 - dw) / 2.0), snap(c.cy() - dh / 2.0), snap(dw), snap(dh));
                self.r.sprite_flip(&bmp, d, src, true, false, 1.0);
            } else if let Some((bmp, w, h)) = self.dex_thumb(&name) {
                let k = (28.0 / h).min(30.0 / w);
                let (dw, dh) = (w * k, h * k);
                let d = R::xywh(x + (30.0 - dw) / 2.0, c.cy() - dh / 2.0, dw, dh);
                self.r.sprite_flip(&bmp, d, R::xywh(0.0, 0.0, w, h), true, false, 1.0);
            }
            x += 32.0;
        }
        x += 6.0;
        let lvl = self.dex.level();
        let (a, b) = (crate::dex::xp_at(lvl), crate::dex::xp_at(lvl + 1));
        let frac = if lvl >= 100 { 1.0 } else { self.dex.xp.saturating_sub(a) as f32 / (b - a).max(1) as f32 };
        self.side_text(&format!("Lv {lvl}"), x, x + 64.0, l1, 11.5, 600, th.text, Align::Left);
        self.side_bar(x, x + 60.0, l2, frac, LEAF);
        x += 72.0;
        let list = &self.dex.daily.list;
        let (done, all) = (list.iter().filter(|c| c.done()).count(), list.len());
        if all > 0 {
            let col = if done == all { LEAF } else { th.text };
            let s = format!("{done}/{all}");
            let sw = self.text_width_at(&s, 11.5, 600);
            self.side_text(&s, x, c.x1, l1, 11.5, 600, col, Align::Left);
            if done == all {
                self.r.icon(G_CHECK, R::new(x + sw + 2.0, l1 - 9.0, x + sw + 16.0, l1 + 9.0), 11.0, LEAF);
            }
            self.side_text("today", x, c.x1, l2, 10.5, 400, th.subtext, Align::Left);
        }
    }

    fn ping_now(&self) -> Option<u32> {
        ping_ms().or_else(|| self.sys.as_ref().and_then(|s| s.ping))
    }

    /// Signal bars for the ping: green to red, more bars the quicker. Returns its word and
    /// colour.
    fn ping_bars(&mut self, x0: f32, base: f32, ms: Option<u32>) -> (&'static str, Rgba) {
        let th = self.theme.clone();
        let (word, col, lit) = match ms {
            None => ("Checking", th.subtext, 0),
            Some(OFFLINE) => ("Offline", RED, 1),
            Some(m) if m < 60 => ("Good", LEAF, 4),
            Some(m) if m < 150 => ("OK", GOLD, 3),
            Some(_) => ("Slow", RED, 2),
        };
        for (k, h) in [4.0, 7.0, 10.0, 13.0].into_iter().enumerate() {
            let x = x0 + k as f32 * 4.0;
            let bar = if k < lit { col } else { th.subtext.a(0.3) };
            self.r.line(x, base - h, x, base, bar, 2.4);
        }
        (word, col)
    }

    /// How hot it's running (the GPU's sensor: Windows can't read the CPU's without a kernel
    /// driver), and a word for it.
    fn heat_word(&self, temp: Option<u32>) -> (&'static str, Rgba) {
        match temp {
            None => ("–", self.theme.subtext),
            Some(t) if t < 60 => ("Cool", LEAF),
            Some(t) if t < 80 => ("Warm", GOLD),
            Some(_) => ("Hot", RED),
        }
    }

    /// Between the left corner and wing: the temperature.
    fn side_temp(&mut self, c: R) {
        let th = self.theme.clone();
        let temp = self.sys.as_ref().and_then(|s| s.gpu.as_ref()).and_then(|g| g.temp);
        let (word, col) = self.heat_word(temp);
        let val = temp.map(|t| format!("{t}°C")).unwrap_or_else(|| "–".into());
        if SideKind::Temp.is_tiny(c) {
            self.r.icon(G_THERMO, R::new(c.cx() - 10.0, c.y0, c.cx() + 10.0, c.y1), 15.0, col);
            return;
        }
        self.r.icon(G_THERMO, R::new(c.x0 - 2.0, c.y0, c.x0 + 16.0, c.y1), 15.0, col);
        if c.w() < SideKind::Temp.width() - 30.0 {
            self.side_text(&val, c.x0 + 20.0, c.x1 + 8.0, c.cy(), 11.5, 600, th.text, Align::Left);
            return;
        }
        let (l1, l2) = (c.cy() - 7.0, c.cy() + 8.0);
        self.side_text("Temperature", c.x0 + 22.0, c.x1 + 6.0, l1, 10.0, 400, th.subtext, Align::Left);
        self.side_text(&format!("{val} · {word}"), c.x0 + 22.0, c.x1 + 6.0, l2, 11.5, 600, col, Align::Left);
    }

    /// The temperature dashboard (the temperature panel opened): now, the last five minutes,
    /// this session's low and high, what the GPU is doing, and the CPU's speed.
    fn draw_temp_dash(&mut self, x0: f32, x1: f32, mut y: f32) {
        let th = self.theme.clone();
        let Some(s) = self.sys.clone() else {
            return self.empty_row(G_THERMO, "Reading temperatures…", x0, x1, y);
        };
        let Some(g) = s.gpu.clone() else {
            return self.empty_row(G_THERMO, "No temperature sensor Windows can read here", x0, x1, y);
        };
        let (word, col) = self.heat_word(g.temp);
        // Now.
        let hero = R::new(x0, y, x1, y + 64.0);
        self.r.fill_round(hero, 12.0, th.bg2.a(0.6));
        self.r.icon(G_THERMO, R::xywh(x0 + 6.0, y + 8.0, 48.0, 48.0), 28.0, col);
        let val = g.temp.map(|t| format!("{t}°C")).unwrap_or_else(|| "–".into());
        self.r.text(&val, R::new(x0 + 58.0, y + 6.0, x0 + 200.0, y + 40.0), 26.0, 600, th.text, Align::Left);
        self.r.text(&format!("{word} · {}", g.name), R::new(x0 + 58.0, y + 38.0, x1 - 150.0, y + 58.0), 11.5, 500, col, Align::Left);
        if let Some((lo, hi)) = self.temp_range {
            self.r.text("This session", R::new(x1 - 160.0, y + 10.0, x1 - 14.0, y + 28.0), 10.5, 400, th.subtext, Align::Right);
            self.r.text(&format!("{lo}° low · {hi}° high"), R::new(x1 - 200.0, y + 28.0, x1 - 14.0, y + 52.0), 13.5, 600, th.text, Align::Right);
        }
        y += 72.0;
        // The last five minutes, on a 30–95 °C scale with the warm and hot lines.
        let box_ = R::new(x0, y, x1, y + 78.0);
        self.r.fill_round(box_, 12.0, th.bg2.a(0.6));
        self.r.text("Last 5 minutes", R::new(x0 + 12.0, y + 4.0, x1 - 12.0, y + 22.0), 10.5, 400, th.subtext, Align::Left);
        let plot = R::new(x0 + 12.0, y + 24.0, x1 - 40.0, y + 70.0);
        let at = |t: f32| plot.y1 - ((t - 30.0) / 65.0).clamp(0.0, 1.0) * plot.h();
        for (t, c) in [(60.0, GOLD), (80.0, RED)] {
            let ly = at(t);
            self.r.line(plot.x0, ly, plot.x1, ly, c.a(0.25), 1.0);
            self.r.text(&format!("{t:.0}°"), R::new(plot.x1 + 4.0, ly - 8.0, x1 - 8.0, ly + 8.0), 9.5, 400, c.a(0.8), Align::Left);
        }
        let hist: Vec<f32> = self.temp_hist.iter().map(|t| ((t - 30.0) / 65.0).clamp(0.0, 1.0)).collect();
        self.r.sparkline(plot, &hist, 300, col);
        y += 86.0;
        // What the GPU is doing.
        let gap = 8.0;
        let cw = (x1 - x0 - 2.0 * gap) / 3.0;
        let load = format!("{}%", (g.util * 100.0).round());
        let load_col = self.load_color(g.util);
        self.stat_tile(R::xywh(x0, y, cw, 56.0), G_GPU, "Load", &load, "", None, Some(g.util), load_col);
        let power = g.power.map(|w| format!("{w:.0} W")).unwrap_or_else(|| "–".into());
        self.stat_tile(R::xywh(x0 + cw + gap, y, cw, 56.0), G_BOLT, "Power", &power, "", None, None, GOLD);
        let clock = g.clock.map(|m| format!("{m} MHz")).unwrap_or_else(|| "–".into());
        self.stat_tile(R::xywh(x0 + 2.0 * (cw + gap), y, cw, 56.0), G_SPEED, "Clock", &clock, "", None, None, SKY);
        y += 64.0;
        // The CPU: its speed (Windows can't read its temperature).
        self.r.icon(G_CPU, R::xywh(x0, y, 20.0, 26.0), 13.0, th.accent);
        let speed = s.cpu_mhz.map(|m| format!("{:.1} GHz", m as f32 / 1000.0)).unwrap_or_else(|| "–".into());
        self.r.text(&format!("CPU  {speed} · {}% busy", (s.cpu * 100.0).round()), R::new(x0 + 26.0, y, x1 - 200.0, y + 26.0), 11.5, 600, th.text, Align::Left);
        self.r.text("its temperature isn't readable on Windows", R::new(x1 - 260.0, y, x1, y + 26.0), 10.5, 400, th.subtext, Align::Right);
    }

    /// The connected Bluetooth devices: each one's symbol and battery; their symbols alone
    /// when there's less room; the Bluetooth symbol when there's least.
    fn side_devices(&mut self, c: R) {
        let th = self.theme.clone();
        let list = gadgets();
        let col_of = |p: Option<u8>| match p {
            Some(p) if p <= 20 => RED,
            Some(p) if p < 50 => GOLD,
            _ => th.subtext,
        };
        if list.is_empty() {
            if c.w() < 70.0 {
                self.r.icon(G_BLUETOOTH, R::new(c.cx() - 10.0, c.y0, c.cx() + 10.0, c.y1), 15.0, th.subtext);
                return;
            }
            let (l1, l2) = (c.cy() - 7.0, c.cy() + 8.0);
            self.r.icon(G_BLUETOOTH, R::new(c.x0 - 2.0, c.y0, c.x0 + 16.0, c.y1), 15.0, th.subtext);
            self.side_text("Bluetooth", c.x0 + 22.0, c.x1 + 6.0, l1, 10.0, 400, th.subtext, Align::Left);
            self.side_text("Nothing on", c.x0 + 22.0, c.x1 + 6.0, l2, 11.5, 600, th.text, Align::Left);
            return;
        }
        let full: f32 = list.iter().map(gadget_w).sum::<f32>() - 6.0;
        let icons = list.len() as f32 * 26.0 - 6.0;
        if c.w() + 4.0 >= full {
            let mut x = c.cx() - full / 2.0;
            for g in &list {
                let (glyph, pct) = *g;
                self.r.icon(glyph, R::new(x, c.y0, x + 20.0, c.y1), 14.0, col_of(pct));
                if let Some(pct) = pct {
                    self.side_text(&format!("{pct}%"), x + 22.0, x + gadget_w(g), c.cy(), 11.5, 600, th.text, Align::Left);
                }
                x += gadget_w(g);
            }
        } else if c.w() + 4.0 >= icons {
            let mut x = c.cx() - icons / 2.0;
            for (glyph, pct) in &list {
                self.r.icon(glyph, R::new(x, c.y0, x + 20.0, c.y1), 14.0, if pct.is_some() { col_of(*pct) } else { th.subtext });
                x += 26.0;
            }
        } else {
            self.r.icon(G_BLUETOOTH, R::new(c.cx() - 10.0, c.y0, c.cx() + 10.0, c.y1), 15.0, th.accent);
        }
    }

    /// The devices panel opened: every connected Bluetooth device with its name and how full
    /// its battery is.
    fn draw_devices(&mut self, x0: f32, x1: f32, mut y: f32) {
        let th = self.theme.clone();
        let list = devices_list();
        if list.is_empty() {
            return self.empty_row(G_BLUETOOTH, "No Bluetooth devices connected", x0, x1, y);
        }
        for (glyph, name, pct) in &list {
            let row = R::new(x0, y, x1, y + DEV_ROW);
            self.r.fill_round(row, 12.0, th.bg2.a(0.6));
            let (word, col) = match pct {
                Some(p) => charge_word(*p),
                None => ("Battery not reported", th.subtext),
            };
            self.r.icon(glyph, R::xywh(x0 + 6.0, y + 6.0, 40.0, 40.0), 22.0, col);
            self.r.text(name, R::new(x0 + 52.0, y + 7.0, x1 - 70.0, y + 27.0), 12.5, 600, th.text, Align::Left);
            match pct {
                Some(p) => {
                    self.side_bar(x0 + 52.0, x1 - 150.0, y + 37.0, *p as f32 / 100.0, col);
                    self.r.text(word, R::new(x1 - 144.0, y + 28.0, x1 - 70.0, y + 46.0), 10.5, 500, col, Align::Left);
                    self.r.text(&format!("{p}%"), R::new(x1 - 70.0, y + 8.0, x1 - 14.0, y + 44.0), 17.0, 600, th.text, Align::Right);
                }
                None => {
                    self.r.text(word, R::new(x0 + 52.0, y + 28.0, x1 - 14.0, y + 46.0), 10.5, 400, th.subtext, Align::Left);
                }
            }
            y += DEV_ROW + 8.0;
        }
        self.r.text("Charge as each device reports it (Windows can't read battery wear)", R::new(x0 + 4.0, y, x1, y + 18.0), 10.0, 400, th.subtext, Align::Left);
    }

    /// The sizes a gap panel can take, widest first.
    fn gap_widths(&self, kind: SideKind) -> [f32; 3] {
        if kind != SideKind::Devices {
            return [kind.width(), kind.small_width(), kind.tiny_width()];
        }
        let list = gadgets();
        if list.is_empty() {
            return [kind.width(), 44.0, 44.0];
        }
        let full = list.iter().map(gadget_w).sum::<f32>() - 6.0 + 30.0;
        let icons = list.len() as f32 * 26.0 - 6.0 + 30.0;
        [full, icons, 44.0]
    }

    fn side_machine(&mut self, c: R) {
        let th = self.theme.clone();
        let (l1, l2) = (c.cy() - 7.0, c.cy() + 8.0);
        let sys = self.sys.clone();
        let mem = sys.as_ref().map(|s| if s.mem_total > 0 { s.mem_used as f32 / s.mem_total as f32 } else { 0.0 });
        let gpu = sys.as_ref().and_then(|s| s.gpu.as_ref().map(|g| g.util));
        let mut x = c.x0;
        for (label, v, col) in [("CPU", sys.as_ref().map(|s| s.cpu), SKY), ("RAM", mem, LILAC), ("GPU", gpu, LEAF)] {
            self.side_text(label, x, x + 30.0, l1, 10.0, 400, th.subtext, Align::Left);
            let val = v.map(|v| format!("{}%", (v * 100.0).round())).unwrap_or_else(|| "–".into());
            self.side_text(&val, x + 28.0, x + 62.0, l1, 11.0, 600, th.text, Align::Left);
            self.side_bar(x, x + 56.0, l2, v.unwrap_or(0.0), col);
            x += 72.0;
        }
        // How quick the internet is.
        let ms = self.ping_now();
        let (word, col) = self.ping_bars(x, c.cy() + 7.0, ms);
        let val = match ms {
            Some(OFFLINE) => "Offline".to_string(),
            Some(m) => format!("{m} ms"),
            None => "–".into(),
        };
        self.side_text(&val, x + 22.0, c.x1 + 6.0, l1, 11.0, 600, th.text, Align::Left);
        self.side_text(word, x + 22.0, c.x1 + 6.0, l2, 10.5, 600, col, Align::Left);
    }
}

/// Milliseconds on the wall clock, for sprite frames.
fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Where the team's sprites sit in its panel (with a pixel to spare).
fn team_strip(p: &Spec, n: usize) -> R {
    let (x0, cy) = (p.closed.x0 + 14.0 - 2.0, p.closed.y0 + p.glance_h / 2.0);
    R::new(x0 - 1.0, cy - 16.0, x0 + 32.0 * n as f32 + 1.0, cy + 16.0)
}

/// The team's clock, in steps of TEAM_FRAME_MS so it's redrawn at most that often.
fn team_clock() -> u64 {
    now_ms() / TEAM_FRAME_MS * TEAM_FRAME_MS
}

/// The fewest milliseconds between redraws of the moving team.
const TEAM_FRAME_MS: u64 = 50;
