//! Move animations on the battle field, like the games: Dragon Claw rakes three claw marks
//! across the target, Thunderbolt drops a bolt on it, Flamethrower streams fire at it,
//! Crunch snaps jaws shut on it… Picked by the move's name, else by its type and whether
//! it's physical or special, in the move's type colour. The hit lands at `HIT`.

use super::*;
use crate::movedata::MOVES;
use std::f32::consts::{PI, TAU};

/// Seconds into a move's line when it hits (the HP drops and the target flinches then).
pub(super) const HIT: f32 = 0.6;
/// How long a move's line lasts.
pub(super) const MOVE_SECS: f32 = 1.8;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Fx {
    Slash,
    Punch,
    Barrage,
    Kick,
    Bite,
    Beam,
    Stream,
    Bubbles,
    Bolt,
    Ice,
    Leaf,
    Rock,
    Quake,
    Rings,
    Sound,
    Orb,
    Stars,
    Wind,
    Drain,
    Tackle,
}

/// Which animation a move gets.
pub(super) fn fx_of(m: u16) -> Fx {
    let mv = &MOVES[m as usize];
    let n = mv.name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    let ty = pokemon::TYPES[mv.t as usize % 18].0;
    if has(&["claw", "slash", "scratch", "cut", "scissor", "swipe", "blade", "razor shell", "aerial ace", "false swipe", "psycho cut", "sacred sword", "secret sword", "x-scissor", "ceaseless edge", "kowtow"]) {
        Fx::Slash
    } else if has(&["close combat", "arm thrust", "barrage", "fury attack", "double hit", "triple", "comet punch", "dual chop", "bullet punch", "rapid spin"]) {
        Fx::Barrage
    } else if has(&["punch", "hammer", "fist", "chop", "palm", "brick break", "superpower", "dynamic", "power-up", "submission", "body press", "drain punch", "sky uppercut", "wood hammer"]) {
        Fx::Punch
    } else if has(&["kick", "stomp", "trop", "high jump", "axe kick", "smart strike"]) {
        Fx::Kick
    } else if has(&["bite", "crunch", "fang", "jaw", "fishious", "lick", "super fang"]) {
        Fx::Bite
    } else if has(&["drain", "absorb", "leech", "bitter blade", "horn leech", "parabolic"]) {
        Fx::Drain
    } else if has(&["beam", "ray", "laser", "pulse", "cannon", "flash", "dazzling", "spectral", "moongeist", "photon", "luster", "mist ball", "psyshock"]) {
        Fx::Beam
    } else if has(&["ball", "sphere", "orb", "bomb", "blast", "shadow", "gem", "focus", "mud shot", "seed bomb", "aura"]) && mv.special {
        Fx::Orb
    } else if has(&["swift", "star", "dazzle", "draining kiss", "moonblast", "fairy wind", "disarming"]) {
        Fx::Stars
    } else if has(&["voice", "boom", "echo", "uproar", "round", "snarl", "clanging", "overdrive", "disarming voice", "bug buzz", "chatter", "torch song", "sparkling aria", "alluring"]) {
        Fx::Sound
    } else if has(&["thunder", "shock", "bolt", "spark", "discharge", "zap", "electro", "volt", "charge beam", "wild charge", "nuzzle", "plasma"]) {
        Fx::Bolt
    } else if has(&["earthquake", "bulldoze", "magnitude", "earth", "fissure", "stomping", "high horsepower", "precipice", "headlong", "land", "scorching sands"]) {
        Fx::Quake
    } else if has(&["rock", "stone", "boulder", "ancient", "power gem", "diamond", "meteor", "rollout", "head smash", "mountain"]) {
        Fx::Rock
    } else if has(&["leaf", "seed", "petal", "needle", "pin missile", "magical", "grass knot", "solar", "trailblaze", "apple", "flower", "bullet seed", "spore"]) {
        Fx::Leaf
    } else if has(&["ice", "icy", "freeze", "blizzard", "snow", "frost", "glaciate", "icicle", "avalanche", "aurora", "triple axel", "mountain gale"]) {
        Fx::Ice
    } else if has(&["sludge", "acid", "poison", "smog", "gunk", "bubble", "venoshock", "cross poison", "barb", "mortal spin", "noxious"]) {
        Fx::Bubbles
    } else if (has(&["flame", "fire", "ember", "flamethrower", "heat", "overheat", "inferno", "lava", "magma", "blaze", "burn", "mystical fire", "breath", "eruption", "pyro", "bitter malice", "infernal"]) && mv.special)
        || (has(&["water", "hydro", "surf", "scald", "muddy", "aqua", "brine", "whirlpool", "octazooka", "snipe", "steam", "wave crash", "chilling water", "jet punch"]) && mv.special)
    {
        Fx::Stream
    } else if has(&["wing", "air", "gust", "wind", "aerial", "hurricane", "peck", "drill", "twister", "razor wind", "sky", "acrobatics", "dual wingbeat", "bleakwind", "beak"]) {
        Fx::Wind
    } else if has(&["psychic", "confusion", "extrasensory", "psywave", "hex", "night shade", "astonish", "ominous", "dream", "stored power", "expanding", "lumina", "esper", "mystical", "infestation", "rage fist"]) {
        Fx::Rings
    } else if mv.special {
        match ty {
            "Fire" | "Water" | "Dragon" => Fx::Stream,
            "Electric" => Fx::Bolt,
            "Ice" => Fx::Ice,
            "Grass" | "Bug" => Fx::Leaf,
            "Rock" => Fx::Rock,
            "Ground" => Fx::Quake,
            "Psychic" | "Ghost" | "Dark" => Fx::Rings,
            "Poison" => Fx::Bubbles,
            "Flying" => Fx::Wind,
            "Fairy" => Fx::Stars,
            _ => Fx::Orb,
        }
    } else {
        match ty {
            "Ground" => Fx::Quake,
            "Rock" => Fx::Rock,
            _ => Fx::Tackle,
        }
    }
}

/// Contact moves: the attacker lunges at the target.
pub(super) fn lunges(f: Fx) -> bool {
    matches!(f, Fx::Slash | Fx::Punch | Fx::Barrage | Fx::Kick | Fx::Bite | Fx::Tackle)
}

/// Moves the attacker gathers power for first (sparks drawn into a growing glow).
fn charges(f: Fx) -> bool {
    matches!(f, Fx::Beam | Fx::Orb | Fx::Stream | Fx::Bolt | Fx::Rings | Fx::Ice | Fx::Bubbles | Fx::Sound)
}

/// When a charged move is let go.
const RELEASE: f32 = 0.32;

/// How far the attacker has lunged (0..1) at `t`: a little wind-up back, then in.
pub(super) fn lunge(t: f32) -> f32 {
    if t < 0.2 {
        -0.12 * (PI * t / 0.2).sin()
    } else if t < HIT {
        ((t - 0.2) / (HIT - 0.2)).powf(1.8)
    } else {
        (1.0 - (t - HIT) / 0.35).max(0.0).powf(0.7)
    }
}

/// The field shakes (Earthquake…): an offset for everyone, a rumble then a big shake.
pub(super) fn quake(f: Fx, t: f32) -> f32 {
    if f != Fx::Quake || !(0.3..1.3).contains(&t) {
        return 0.0;
    }
    let amp = if t < HIT { 2.0 } else { 7.0 * (1.0 - (t - HIT) / 0.7) };
    (t * 75.0).sin() * amp
}

/// How far the attacker and the target are moved at `t`: the attacker lunges in (or rises
/// and jolts back as it lets a move go), the target is knocked back when it's hit, reels
/// after a psychic or sound move and is bounced by an earthquake.
pub(super) fn offsets(m: u16, t: f32, a: (f32, f32), b: (f32, f32)) -> ((f32, f32), (f32, f32)) {
    let f = fx_of(m);
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let len = (vx * vx + vy * vy).sqrt().max(1.0);
    let (ux, uy) = (vx / len, vy / len);
    let mut at = (0.0, 0.0);
    if lunges(f) {
        let l = lunge(t) * if f == Fx::Tackle { 0.42 } else { 0.32 };
        at = (vx * l, vy * l);
    } else if charges(f) {
        let up = if t < RELEASE { -3.0 * t / RELEASE } else { -3.0 * (1.0 - (t - RELEASE) / 0.2).max(0.0) };
        let back = if (RELEASE..RELEASE + 0.25).contains(&t) { -7.0 * (PI * (t - RELEASE) / 0.25).sin() } else { 0.0 };
        at = (ux * back, up + uy * back);
    }
    let mut to = (0.0, 0.0);
    let hits: &[(f32, f32)] = match f {
        Fx::Barrage => &[(HIT - 0.15, 6.0), (HIT, 6.0), (HIT + 0.15, 9.0)],
        Fx::Stars => &[(HIT - 0.08, 4.0), (HIT, 4.0), (HIT + 0.08, 6.0)],
        Fx::Drain | Fx::Rings => &[(HIT, 5.0)],
        _ => &[(HIT, 11.0)],
    };
    for &(th, d) in hits {
        let k = (t - th) / 0.4;
        if (0.0..1.0).contains(&k) {
            let push = d * (1.0 - k).powi(2);
            to.0 += ux * push;
            to.1 += uy * push * 0.5;
        }
    }
    if matches!(f, Fx::Rings | Fx::Sound) && (HIT..HIT + 0.8).contains(&t) {
        to.0 += (t * 30.0).sin() * 4.0 * (1.0 - (t - HIT) / 0.8);
    }
    if f == Fx::Quake && (HIT..HIT + 0.6).contains(&t) {
        to.1 -= ((t - HIT) * 22.0).sin().abs() * 7.0 * (1.0 - (t - HIT) / 0.6);
    }
    (at, to)
}

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// Fade in quickly from `t0`, out by `t1`.
fn life(t: f32, t0: f32, t1: f32) -> f32 {
    if t < t0 || t > t1 {
        0.0
    } else {
        clamp01((t - t0) / 0.06).min(clamp01((t1 - t) / 0.25))
    }
}

fn lerp(a: (f32, f32), b: (f32, f32), k: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k)
}

fn ease_out(k: f32) -> f32 {
    1.0 - (1.0 - clamp01(k)).powi(2)
}

fn ease_in(k: f32) -> f32 {
    clamp01(k).powi(2)
}

/// A repeatable 0..1 "random" number for particle `i`.
fn rnd(i: u32, salt: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B).wrapping_add(0x27D4_EB2F);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x & 0xFFFF) as f32 / 65535.0
}

/// A point `k` of the way along a curve from `a` to `b`, bowed `bow` pixels to one side.
fn curve(a: (f32, f32), b: (f32, f32), bow: f32, k: f32) -> (f32, f32) {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let len = (vx * vx + vy * vy).sqrt().max(1.0);
    let c = ((a.0 + b.0) / 2.0 - vy / len * bow, (a.1 + b.1) / 2.0 + vx / len * bow);
    let u = 1.0 - k;
    (u * u * a.0 + 2.0 * u * k * c.0 + k * k * b.0, u * u * a.1 + 2.0 * u * k * c.1 + k * k * b.1)
}

/// A star with `n` points.
fn star_pts(c: (f32, f32), ro: f32, ri: f32, n: u32, rot: f32) -> Vec<(f32, f32)> {
    (0..n * 2)
        .map(|i| {
            let r = if i % 2 == 0 { ro } else { ri };
            let ang = rot + i as f32 * PI / n as f32 - PI / 2.0;
            (c.0 + ang.cos() * r, c.1 + ang.sin() * r)
        })
        .collect()
}

/// A lumpy rock.
fn rock_pts(c: (f32, f32), r: f32, seed: u32, rot: f32) -> Vec<(f32, f32)> {
    (0..7)
        .map(|i| {
            let ang = rot + i as f32 / 7.0 * TAU;
            let rr = r * (0.72 + 0.34 * rnd(i, seed));
            (c.0 + ang.cos() * rr, c.1 + ang.sin() * rr)
        })
        .collect()
}

/// A leaf (pointed at both ends), `len` long, pointing at `ang`.
fn leaf_pts(c: (f32, f32), len: f32, w: f32, ang: f32) -> Vec<(f32, f32)> {
    let n = 6;
    let mut v = Vec::with_capacity(2 * n);
    for i in 0..=n {
        let u = i as f32 / n as f32;
        v.push(((u - 0.5) * len, -(PI * u).sin() * w * 0.5));
    }
    for i in (1..n).rev() {
        let u = i as f32 / n as f32;
        v.push(((u - 0.5) * len, (PI * u).sin() * w * 0.5 * 0.7));
    }
    let (sn, cs) = ang.sin_cos();
    v.into_iter().map(|(x, y)| (c.0 + x * cs - y * sn, c.1 + x * sn + y * cs)).collect()
}

/// A flame: round at the bottom, a flickering point at the top.
fn flame_pts(base: (f32, f32), h: f32, w: f32, sway: f32) -> Vec<(f32, f32)> {
    let n = 7;
    let side = |u: f32, sign: f32| {
        let half = w * 0.5 * (PI * u.powf(0.6)).sin();
        (base.0 + sign * half + sway * u * u, base.1 - h * u)
    };
    let mut v: Vec<(f32, f32)> = (0..=n).map(|i| side(i as f32 / n as f32, -1.0)).collect();
    v.extend((1..n).rev().map(|i| side(i as f32 / n as f32, 1.0)));
    v
}

/// Points round an ellipse from angle `a0` to `a1`.
fn arc_path(c: (f32, f32), rx: f32, ry: f32, a0: f32, a1: f32) -> Vec<(f32, f32)> {
    (0..=12)
        .map(|i| {
            let ang = a0 + (a1 - a0) * i as f32 / 12.0;
            (c.0 + ang.cos() * rx, c.1 + ang.sin() * ry)
        })
        .collect()
}

/// A thick stroke along `path` that tapers to points at both ends (claw marks, air blades).
fn taper(path: &[(f32, f32)], w: f32) -> Vec<(f32, f32)> {
    let n = path.len();
    if n < 2 {
        return Vec::new();
    }
    let mut left = Vec::with_capacity(n * 2);
    let mut right = Vec::with_capacity(n);
    for (i, p) in path.iter().enumerate() {
        let (p0, p1) = (path[i.saturating_sub(1)], path[(i + 1).min(n - 1)]);
        let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
        let len = (dx * dx + dy * dy).sqrt().max(0.001);
        let hw = w * 0.5 * (PI * i as f32 / (n - 1) as f32).sin();
        let (nx, ny) = (-dy / len * hw, dx / len * hw);
        left.push((p.0 + nx, p.1 + ny));
        right.push((p.0 - nx, p.1 - ny));
    }
    left.extend(right.into_iter().rev());
    left
}

/// A jagged line from `a` to `b` (lightning, cracks).
fn zigzag(a: (f32, f32), b: (f32, f32), n: u32, jig: f32, seed: u32) -> Vec<(f32, f32)> {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let len = (vx * vx + vy * vy).sqrt().max(1.0);
    let (nx, ny) = (-vy / len, vx / len);
    (0..=n)
        .map(|i| {
            let k = i as f32 / n as f32;
            let j = if i == 0 || i == n { 0.0 } else { (rnd(i, seed) - 0.5) * 2.0 * jig };
            (a.0 + vx * k + nx * j, a.1 + vy * k + ny * j)
        })
        .collect()
}

/// A burst of particles flying out from a point (sparks, debris, droplets).
#[derive(Clone, Copy)]
struct Spray {
    seed: u32,
    n: u32,
    /// How long each lasts.
    dur: f32,
    speed: f32,
    /// Pulls them down (pixels/s²).
    grav: f32,
    rad: f32,
    col: Rgba,
    glow: bool,
    /// Which way they go (radians) and how widely they spread round it.
    dir: f32,
    spread: f32,
}

fn spray(seed: u32, n: u32, col: Rgba) -> Spray {
    Spray { seed, n, dur: 0.55, speed: 120.0, grav: 0.0, rad: 3.0, col, glow: false, dir: 0.0, spread: TAU }
}

const WHITE: Rgba = Rgba::rgb(255, 255, 255);

impl App {
    /// Draw `sp` `e` seconds after it burst out of `c`.
    fn draw_spray(&mut self, sp: Spray, c: (f32, f32), e: f32, s: f32) {
        if e < 0.0 {
            return;
        }
        for i in 0..sp.n {
            let span = sp.dur * (0.6 + 0.4 * rnd(i, sp.seed + 1));
            let k = e / span;
            if k >= 1.0 {
                continue;
            }
            let ang = sp.dir + (rnd(i, sp.seed) - 0.5) * sp.spread;
            let v = sp.speed * s * (0.35 + 0.65 * rnd(i, sp.seed + 2));
            let d = v * e * (1.0 - 0.5 * k);
            let (x, y) = (c.0 + ang.cos() * d, c.1 + ang.sin() * d + 0.5 * sp.grav * s * e * e);
            let r = sp.rad * s * (0.55 + 0.45 * rnd(i, sp.seed + 3)) * (1.0 - 0.6 * k);
            let al = 1.0 - k * k;
            if sp.glow {
                self.r.glow(x, y, r * 3.2, sp.col.a(0.45 * al));
            }
            self.r.circle(x, y, r, sp.col.a(al));
        }
    }

    fn polyline(&mut self, pts: &[(f32, f32)], c: Rgba, w: f32) {
        let segs: Vec<(f32, f32, f32, f32)> = pts.windows(2).map(|p| (p[0].0, p[0].1, p[1].0, p[1].1)).collect();
        self.r.lines(&segs, c, w);
    }

    /// The attacker gathers power: sparks spiral into a growing glow, then it's let go.
    fn charge_up(&mut self, a: (f32, f32), t: f32, col: Rgba, s: f32) {
        if t < RELEASE {
            let k = t / RELEASE;
            self.r.glow(a.0, a.1, (14.0 + 26.0 * k) * s, col.a(0.3 + 0.4 * k));
            for i in 0..12 {
                let t0 = rnd(i, 7) * 0.14;
                let p = (t - t0) / 0.18;
                if !(0.0..1.0).contains(&p) {
                    continue;
                }
                let ang = rnd(i, 9) * TAU + p * 1.8;
                let d = (46.0 - 40.0 * p) * s;
                let (x, y) = (a.0 + ang.cos() * d, a.1 + ang.sin() * d);
                self.r.line(x, y, x - ang.cos() * 6.0 * s, y - ang.sin() * 6.0 * s, col.mix(WHITE, 0.5).a(p.min(1.0 - p) * 2.0), 2.0 * s);
            }
            self.r.circle(a.0, a.1, (2.0 + 6.0 * k) * s, WHITE.a(0.85 * k));
        } else if t < RELEASE + 0.14 {
            let k = (t - RELEASE) / 0.14;
            self.r.ring(a.0, a.1, (10.0 + 30.0 * k) * s, WHITE.a(1.0 - k), (3.0 * (1.0 - k) + 1.0) * s);
            self.r.glow(a.0, a.1, 34.0 * s, col.a(0.7 * (1.0 - k)));
        }
    }

    /// The impact of a blow: a star-shaped flash, a shockwave and flying bits.
    fn impact(&mut self, c: (f32, f32), e: f32, big: f32, col: Rgba, seed: u32, s: f32) {
        let k = e / 0.42;
        if !(0.0..1.0).contains(&k) {
            return;
        }
        let s2 = s * big;
        self.r.glow(c.0, c.1, (34.0 + 20.0 * k) * s2, col.a(0.65 * (1.0 - k)));
        if k < 0.45 {
            let q = k / 0.45;
            let ro = (14.0 + 24.0 * ease_out(q)) * s2;
            let rot = rnd(seed, 3) * 0.6;
            self.r.poly(&star_pts(c, ro, ro * 0.42, 8, rot), col.mix(WHITE, 0.35).a(1.0 - q));
            self.r.poly(&star_pts(c, ro * 0.6, ro * 0.25, 8, rot + 0.2), WHITE.a(1.0 - q * 0.8));
        }
        self.r.ring(c.0, c.1, (8.0 + 48.0 * ease_out(k)) * s2, WHITE.a(0.85 * (1.0 - k)), (4.0 * (1.0 - k) + 1.0) * s);
        self.draw_spray(Spray { speed: 170.0, grav: 320.0, rad: 2.6, dur: 0.45, ..spray(seed, 10, col.mix(WHITE, 0.3)) }, c, e, s);
    }

    /// The field behind the Pokémon during move `m`: darkened for a Ghost or Dark move,
    /// washed pink for Psychic, warm for Fire…, and a flash when a bolt or a strong move hits.
    pub(super) fn draw_fx_bg(&mut self, m: u16, t: f32, field: R) {
        let mv = &MOVES[m as usize];
        let f = fx_of(m);
        let env = clamp01(t / 0.15).min(clamp01((MOVE_SECS - 0.1 - t) / 0.35));
        let tint = match pokemon::TYPES[mv.t as usize % 18].0 {
            "Dark" | "Ghost" => Some((Rgba::rgb(10, 5, 25), 0.45)),
            "Psychic" => Some((Rgba::rgb(255, 110, 190), 0.16 + 0.06 * (t * 9.0).sin())),
            "Fire" => Some((Rgba::rgb(255, 120, 40), 0.14)),
            "Ice" => Some((Rgba::rgb(150, 220, 255), 0.16)),
            "Dragon" => Some((Rgba::rgb(110, 60, 230), 0.18)),
            "Water" => Some((Rgba::rgb(60, 140, 255), 0.12)),
            "Poison" => Some((Rgba::rgb(160, 60, 200), 0.14)),
            "Fairy" => Some((Rgba::rgb(255, 150, 210), 0.14)),
            "Grass" => Some((Rgba::rgb(90, 200, 90), 0.1)),
            "Electric" => Some((Rgba::rgb(255, 230, 80), 0.08)),
            "Ground" | "Rock" => Some((Rgba::rgb(170, 130, 70), 0.1)),
            "Fighting" => Some((Rgba::rgb(200, 60, 40), 0.1)),
            _ => None,
        };
        if let Some((c, al)) = tint {
            if env > 0.0 {
                self.r.fill_round(field, 14.0, c.a(al * env));
            }
        }
        let mut flash = 0.0f32;
        if f == Fx::Bolt {
            for at in [HIT - 0.12, HIT, HIT + 0.08] {
                let k = (t - at) / 0.07;
                if (0.0..1.0).contains(&k) {
                    flash = flash.max(0.4 * (1.0 - k));
                }
            }
        }
        if mv.power >= 80 {
            let k = (t - HIT) / 0.16;
            if (0.0..1.0).contains(&k) {
                flash = flash.max(0.28 * (1.0 - k));
            }
        }
        if flash > 0.0 {
            self.r.fill_round(field, 14.0, WHITE.a(flash));
        }
    }

    /// Draw move `m`'s animation `t` seconds in, from the attacker at `a` to the target at
    /// `b` (their centres); `size` is roughly the target's height.
    pub(super) fn draw_move_fx(&mut self, m: u16, t: f32, a: (f32, f32), b: (f32, f32), size: f32) {
        let f = fx_of(m);
        let mv = &MOVES[m as usize];
        let (tname, (r, g, bl)) = pokemon::TYPES[mv.t as usize % 18];
        let col = Rgba::rgb(r, g, bl);
        let light = col.mix(WHITE, 0.55);
        let black = Rgba::rgb(0, 0, 0);
        let s = (size / 70.0).clamp(0.7, 1.5);
        let dir = if b.0 > a.0 { 1.0 } else { -1.0 };
        let e = t - HIT;
        let seed = m as u32 * 31 + 5;
        if charges(f) {
            self.charge_up(a, t, col, s);
        }
        match f {
            Fx::Slash => {
                // Three claw marks rake across it one after another: they glow, linger, fade.
                for i in 0..3u32 {
                    let t0 = HIT - 0.12 + i as f32 * 0.06;
                    let p = clamp01((t - t0) / 0.09);
                    let al = life(t, t0, 1.55);
                    if p <= 0.0 || al <= 0.0 {
                        continue;
                    }
                    let off = (i as f32 - 1.0) * 13.0 * s;
                    let from = (b.0 - 24.0 * s * dir + off, b.1 - 28.0 * s);
                    let to = (b.0 + 20.0 * s * dir + off, b.1 + 26.0 * s);
                    let path: Vec<(f32, f32)> = (0..=10).map(|j| curve(from, to, 7.0 * s * dir, j as f32 / 10.0 * p)).collect();
                    let fresh = if t < t0 + 0.3 { 1.0 } else { 0.6 };
                    self.r.poly(&taper(&path, 14.0 * s), col.a(0.55 * al * fresh));
                    self.r.poly(&taper(&path, 6.5 * s), light.a(0.95 * al));
                    self.r.poly(&taper(&path, 2.6 * s), WHITE.a(al));
                    if p < 1.0 {
                        let tip = path[path.len() - 1];
                        self.r.glow(tip.0, tip.1, 16.0 * s, WHITE.a(0.9));
                    }
                }
                if e >= 0.0 {
                    self.r.glow(b.0, b.1, 46.0 * s, col.a(0.6 * (1.0 - e / 0.3).max(0.0)));
                    self.draw_spray(Spray { speed: 160.0, rad: 2.2, glow: true, dur: 0.45, ..spray(seed, 14, light) }, b, e, s);
                }
            }
            Fx::Punch | Fx::Kick | Fx::Barrage | Fx::Tackle => {
                // Speed lines close in, then each blow lands with a flash and a shockwave.
                let hits: &[(f32, f32, f32, f32)] = match f {
                    Fx::Barrage => &[(HIT - 0.15, -12.0, -10.0, 0.75), (HIT, 10.0, 4.0, 0.75), (HIT + 0.15, -2.0, 12.0, 1.0)],
                    Fx::Kick => &[(HIT, 0.0, 14.0, 1.0)],
                    Fx::Tackle => &[(HIT, -8.0, 0.0, 1.0)],
                    _ => &[(HIT, 0.0, 0.0, 1.15)],
                };
                let first = hits[0].0;
                let k = (t - (first - 0.24)) / 0.24;
                if (0.0..1.0).contains(&k) {
                    for j in 0..10u32 {
                        let ang = j as f32 / 10.0 * TAU + rnd(j, seed) * 0.5;
                        let r0 = (62.0 - 32.0 * k) * s;
                        let r1 = r0 + (10.0 + 8.0 * rnd(j, seed + 1)) * s;
                        let (cs, sn) = (ang.cos(), ang.sin());
                        self.r.line(b.0 + cs * r0, b.1 + sn * r0, b.0 + cs * r1, b.1 + sn * r1, WHITE.a(0.7 * k), 2.0 * s);
                    }
                }
                let c = if f == Fx::Tackle { Rgba::rgb(235, 235, 235) } else { col };
                for (i, &(th, dx, dy, big)) in hits.iter().enumerate() {
                    self.impact((b.0 + dx * dir * s, b.1 + dy * s), t - th, big, c, seed + i as u32 * 17, s);
                }
            }
            Fx::Bite => {
                // Two rows of fangs open wide, then snap shut on it.
                let close = ease_in((t - 0.25) / (HIT - 0.25));
                let al = life(t, 0.22, 1.1);
                if al > 0.0 {
                    let gap = (36.0 - 28.0 * close) * s;
                    let shake = if (0.0..0.25).contains(&e) { (e * 80.0).sin() * 2.0 * s } else { 0.0 };
                    let w = 56.0 * s;
                    let outline = col.mix(black, 0.35);
                    for (sign, n, off) in [(-1.0f32, 4u32, 0.5f32), (1.0, 3, 1.0)] {
                        let gum = b.1 + sign * gap;
                        let band = R::new(b.0 - w / 2.0 + shake, gum.min(gum + sign * 7.0 * s), b.0 + w / 2.0 + shake, gum.max(gum + sign * 7.0 * s));
                        self.r.fill_round(band.inset(-1.5), 4.0 * s, outline.a(al));
                        self.r.fill_round(band, 4.0 * s, col.mix(WHITE, 0.2).a(al));
                        for i in 0..n {
                            let x = b.0 - w / 2.0 + (i as f32 + off) * w / 4.0 + shake;
                            let outer = i == 0 || i == n - 1;
                            let tip = (if outer && n == 4 { 17.0 } else { 13.0 }) * s;
                            let hw = 5.5 * s;
                            let tri = [(x - hw, gum), (x + hw, gum), (x, gum - sign * tip)];
                            let big = [(x - hw - 1.5, gum), (x + hw + 1.5, gum), (x, gum - sign * (tip + 2.0))];
                            self.r.poly(&big, outline.a(al));
                            self.r.poly(&tri, WHITE.a(al));
                        }
                    }
                }
                if e >= 0.0 {
                    self.impact(b, e, 0.8, col, seed, s);
                }
            }
            Fx::Beam => {
                // A beam shoots out and holds on it, pulsing, rings racing along it.
                let grow = ease_out((t - RELEASE) / 0.18);
                let al = life(t, RELEASE, 1.35);
                if t >= RELEASE && al > 0.0 {
                    let end = lerp(a, b, grow);
                    let thin = if t > 1.05 { (1.0 - (t - 1.05) / 0.3).max(0.0) } else { 1.0 };
                    let wob = 1.0 + 0.18 * (t * 47.0).sin() + 0.1 * (t * 83.0).sin();
                    self.r.line(a.0, a.1, end.0, end.1, col.a(0.28 * al), 26.0 * s * wob * thin);
                    self.r.line(a.0, a.1, end.0, end.1, col.a(0.75 * al), 12.0 * s * wob * thin);
                    self.r.line(a.0, a.1, end.0, end.1, light.a(0.95 * al), 6.0 * s * thin);
                    self.r.line(a.0, a.1, end.0, end.1, WHITE.a(al), 2.4 * s * thin);
                    for j in 0..4 {
                        let k = ((t * 2.4 + j as f32 / 4.0) % 1.0) * grow;
                        let p = lerp(a, b, k);
                        self.r.ring(p.0, p.1, 9.0 * s * wob * thin, WHITE.a(0.5 * al), 1.6);
                    }
                    if tname == "Ice" {
                        for j in 0..5 {
                            let k = ((t * 1.5 + j as f32 / 5.0) % 1.0) * grow;
                            let p = lerp(a, b, k);
                            self.twinkle(p.0, p.1 + (j as f32 - 2.0) * 5.0 * s, 5.0 * s * thin, WHITE.a(al));
                        }
                    }
                    self.r.glow(a.0, a.1, 22.0 * s, light.a(0.8 * al));
                    if grow >= 1.0 {
                        let pulse = 1.0 + 0.15 * (t * 40.0).sin();
                        self.r.glow(b.0, b.1, 48.0 * s * pulse, col.a(0.7 * al));
                        self.r.circle(b.0, b.1, 9.0 * s * pulse * thin, WHITE.a(0.9 * al));
                    }
                }
                for w in 0..4u32 {
                    let start = HIT + w as f32 * 0.12;
                    self.draw_spray(Spray { speed: 150.0, rad: 2.0, glow: true, dur: 0.4, ..spray(seed + w, 6, light) }, b, t - start, s);
                }
            }
            Fx::Stream => {
                // A gush of fire (or water) pours at it, then it's left burning (or splashed).
                let pal: [Rgba; 3] = match tname {
                    "Fire" => [Rgba::rgb(255, 240, 170), Rgba::rgb(255, 150, 40), Rgba::rgb(220, 60, 30)],
                    "Water" => [WHITE, Rgba::rgb(140, 200, 255), Rgba::rgb(50, 120, 230)],
                    _ => [light, col.mix(WHITE, 0.3), col],
                };
                let water = tname == "Water";
                for i in 0..26u32 {
                    let t0 = RELEASE - 0.06 + i as f32 * 0.021;
                    let k = (t - t0) / 0.3;
                    if !(0.0..1.0).contains(&k) {
                        continue;
                    }
                    let (x, y) = lerp(a, b, k);
                    let y = y + (rnd(i, seed) - 0.5) * 24.0 * s * k + (k * 10.0 + i as f32).sin() * 3.0 * s;
                    let rad = (3.0 + 7.0 * k) * s * (0.7 + 0.5 * rnd(i, seed + 1));
                    if water {
                        self.r.circle(x, y, rad, pal[2].a(0.75));
                        self.r.circle(x, y, rad * 0.7, pal[1].a(0.8));
                        self.r.circle(x - rad * 0.3, y - rad * 0.3, rad * 0.3, WHITE.a(0.9));
                    } else {
                        let c = if k < 0.3 { pal[0] } else if k < 0.65 { pal[1] } else { pal[2] };
                        self.r.glow(x, y, rad * 2.8, pal[1].a(0.5));
                        self.r.circle(x, y, rad, c.a(0.95));
                    }
                }
                if water {
                    self.draw_spray(Spray { dir: -PI / 2.0, spread: 2.4, speed: 190.0, grav: 520.0, rad: 3.2, dur: 0.7, ..spray(seed, 18, pal[1]) }, b, e, s);
                    let k = e / 0.5;
                    if (0.0..1.0).contains(&k) {
                        self.r.ring(b.0, b.1 + size * 0.3, (10.0 + 40.0 * k) * s, WHITE.a(0.8 * (1.0 - k)), 3.0 * s * (1.0 - k) + 1.0);
                    }
                } else {
                    let al = life(t, HIT, 1.7);
                    if al > 0.0 {
                        for j in 0..6u32 {
                            let base = (b.0 + (j as f32 - 2.5) * 11.0 * s, b.1 + size * 0.38);
                            let flick = 0.8 + 0.25 * (t * 20.0 + j as f32 * 1.7).sin();
                            let h = (24.0 + 16.0 * rnd(j, seed)) * s * flick * al;
                            let sway = (t * 9.0 + j as f32).sin() * 5.0 * s;
                            self.r.poly(&flame_pts(base, h, 13.0 * s, sway), pal[2].a(0.85 * al));
                            self.r.poly(&flame_pts(base, h * 0.7, 9.0 * s, sway * 0.7), pal[1].a(0.9 * al));
                            self.r.poly(&flame_pts(base, h * 0.38, 5.0 * s, sway * 0.4), pal[0].a(al));
                        }
                        for j in 0..5u32 {
                            let k = (t - (HIT + 0.2 + j as f32 * 0.12)) / 0.6;
                            if (0.0..1.0).contains(&k) {
                                let x = b.0 + (rnd(j, seed + 2) - 0.5) * 34.0 * s;
                                self.r.circle(x, b.1 - 10.0 * s - 40.0 * s * k, (5.0 + 8.0 * k) * s, Rgba::rgb(90, 85, 90).a(0.4 * (1.0 - k)));
                            }
                        }
                    }
                    if (0.0..0.4).contains(&e) {
                        self.r.glow(b.0, b.1, 44.0 * s, pal[1].a(0.6 * (1.0 - e / 0.4)));
                    }
                }
            }
            Fx::Bubbles => {
                // Blobs of sludge lob over and splat on it, then drip and fizz.
                for i in 0..5u32 {
                    let t0 = RELEASE - 0.02 + i as f32 * 0.05;
                    let k = (t - t0) / 0.26;
                    if !(0.0..1.0).contains(&k) {
                        continue;
                    }
                    let (x, y) = lerp(a, b, k);
                    let y = y - (PI * k).sin() * 38.0 * s + (i as f32 - 2.0) * 4.0 * s;
                    let rad = (5.0 + 2.0 * rnd(i, seed)) * s;
                    self.r.circle(x, y, rad + 1.5, col.mix(black, 0.4));
                    self.r.circle(x, y, rad, col);
                    self.r.circle(x - rad * 0.35, y - rad * 0.35, rad * 0.3, WHITE.a(0.85));
                }
                let al = life(t, HIT, 1.6);
                if al > 0.0 {
                    let spread = ease_out(e / 0.2);
                    for j in 0..7u32 {
                        let ang = j as f32 / 7.0 * TAU + 0.4;
                        let d = (8.0 + 18.0 * rnd(j, seed + 3)) * s * spread;
                        let rad = (5.0 + 5.0 * rnd(j, seed + 4)) * s;
                        self.r.circle(b.0 + ang.cos() * d, b.1 + ang.sin() * d * 0.8, rad, col.a(0.85 * al));
                    }
                    for j in 0..5u32 {
                        let k = ((e - j as f32 * 0.12) / 0.6).max(0.0);
                        if k < 1.0 && e > 0.15 {
                            let x = b.0 + (j as f32 - 2.0) * 10.0 * s;
                            self.r.circle(x, b.1 + 14.0 * s + 40.0 * s * k * k, 3.0 * s, col.mix(black, 0.2).a(al * (1.0 - k)));
                        }
                        let kb = ((t * 1.6 + j as f32 * 0.2) % 1.0, j as f32);
                        let bx = b.0 + (kb.1 - 2.0) * 13.0 * s + (t * 5.0 + kb.1).sin() * 4.0 * s;
                        self.r.ring(bx, b.1 - 4.0 * s - 36.0 * s * kb.0, (2.0 + 5.0 * kb.0) * s, light.a(al * (1.0 - kb.0)), 1.6);
                    }
                }
            }
            Fx::Bolt => {
                // A crackling bolt leaps at it and another strikes from above; it's left
                // fizzing with sparks.
                let yellow = Rgba::rgb(255, 245, 170);
                let al = life(t, HIT - 0.15, HIT + 0.4);
                let on = ((t * 30.0) as i32) % 3 != 0;
                if al > 0.0 && on {
                    let flick = (t * 20.0) as u32;
                    let main = zigzag(a, b, 9, 12.0 * s, flick);
                    let sky = zigzag((b.0 + 4.0 * s, b.1 - 64.0 * s), b, 5, 9.0 * s, flick + 5);
                    let mut forks = Vec::new();
                    for (n, i) in [3usize, 6].into_iter().enumerate() {
                        let p = main[i];
                        let ang = rnd(n as u32, flick) * TAU;
                        forks.push(zigzag(p, (p.0 + ang.cos() * 26.0 * s, p.1 + ang.sin() * 26.0 * s), 3, 5.0 * s, flick + 9 + n as u32));
                    }
                    for path in [&main, &sky] {
                        self.polyline(path, col.a(0.45 * al), 12.0 * s);
                        self.polyline(path, yellow.a(al), 5.0 * s);
                        self.polyline(path, WHITE.a(al), 2.0 * s);
                    }
                    for path in &forks {
                        self.polyline(path, yellow.a(0.8 * al), 2.4 * s);
                    }
                    self.r.glow(b.0, b.1, 52.0 * s, col.a(0.7 * al));
                }
                if e > 0.0 && t < 1.55 {
                    let fade = 1.0 - e / 0.95;
                    let flick = (t * 25.0) as u32;
                    for j in 0..4u32 {
                        let ang = rnd(j, flick) * TAU;
                        let (r0, r1) = (10.0 * s, (26.0 + 10.0 * rnd(j, flick + 1)) * s);
                        let p0 = (b.0 + ang.cos() * r0, b.1 + ang.sin() * r0);
                        let p1 = (b.0 + ang.cos() * r1, b.1 + ang.sin() * r1);
                        let arc = zigzag(p0, p1, 4, 4.0 * s, flick + j);
                        self.polyline(&arc, yellow.a(fade), 2.0 * s);
                    }
                    self.draw_spray(Spray { rad: 1.8, speed: 210.0, glow: true, ..spray(seed, 14, yellow) }, b, e, s);
                }
            }
            Fx::Ice => {
                // Snow swirls in, shards fly at it and stick, it's frozen in a block of ice
                // that sparkles and then shatters.
                let icy = Rgba::rgb(200, 240, 255);
                for i in 0..14u32 {
                    let t0 = 0.08 + rnd(i, seed) * 0.25;
                    let k = (t - t0) / 0.35;
                    if !(0.0..1.0).contains(&k) {
                        continue;
                    }
                    let ang = rnd(i, seed + 1) * TAU + k * 2.5;
                    let d = (72.0 - 60.0 * k) * s;
                    self.r.circle(b.0 + ang.cos() * d, b.1 + ang.sin() * d * 0.8, 2.2 * s, WHITE.a(0.9 * (k.min(1.0 - k) * 4.0).min(1.0)));
                }
                if (0.3..1.35).contains(&t) {
                    let k = ease_in((t - 0.3) / (HIT - 0.3));
                    for i in 0..7 {
                        let ang = i as f32 / 7.0 * TAU + 0.4;
                        let d = (72.0 - 50.0 * k) * s;
                        let (ux, uy) = (ang.cos(), ang.sin());
                        let c = (b.0 + ux * d, b.1 + uy * d);
                        let pts = [(c.0 - ux * 11.0 * s, c.1 - uy * 11.0 * s), (c.0 - uy * 3.5 * s, c.1 + ux * 3.5 * s), (c.0 + ux * 9.0 * s, c.1 + uy * 9.0 * s), (c.0 + uy * 3.5 * s, c.1 - ux * 3.5 * s)];
                        self.r.poly(&pts, icy.a(0.95 * life(t, 0.3, 1.35)));
                    }
                }
                let al = life(t, HIT, 1.4);
                if al > 0.0 {
                    let hex: Vec<(f32, f32)> = (0..6).map(|i| {
                        let ang = i as f32 / 6.0 * TAU + PI / 6.0;
                        (b.0 + ang.cos() * 38.0 * s, b.1 + ang.sin() * 40.0 * s)
                    }).collect();
                    self.r.poly(&hex, icy.a(0.3 * al));
                    let mut ring = hex.clone();
                    ring.push(hex[0]);
                    self.polyline(&ring, WHITE.a(0.85 * al), 2.0 * s);
                    self.r.line(hex[3].0 + 8.0 * s, hex[3].1, hex[4].0 + 4.0 * s, hex[4].1 + 10.0 * s, WHITE.a(0.6 * al), 2.0 * s);
                    for (j, p) in hex.iter().enumerate().step_by(2) {
                        let tw = 3.0 + 3.0 * (t * 12.0 + j as f32).sin().abs();
                        self.twinkle(p.0, p.1, tw * s, WHITE.a(al));
                    }
                }
                let k = (t - 1.3) / 0.45;
                if (0.0..1.0).contains(&k) {
                    let e2 = t - 1.3;
                    for i in 0..10u32 {
                        let ang = rnd(i, seed + 5) * TAU;
                        let v = (90.0 + 80.0 * rnd(i, seed + 6)) * s;
                        let c = (b.0 + ang.cos() * v * e2, b.1 + ang.sin() * v * e2 + 300.0 * s * e2 * e2);
                        let rot = e2 * 10.0 + i as f32;
                        let (ux, uy) = (rot.cos() * 6.0 * s, rot.sin() * 6.0 * s);
                        self.r.poly(&[(c.0 - ux, c.1 - uy), (c.0 - uy * 0.5, c.1 + ux * 0.5), (c.0 + ux, c.1 + uy)], icy.a(1.0 - k));
                    }
                }
            }
            Fx::Leaf => {
                // Spinning leaves sweep in on curving paths, then a few flutter down.
                let dark = col.mix(black, 0.45);
                for i in 0..8u32 {
                    let t0 = 0.05 + i as f32 * 0.04;
                    let k = (t - t0) / 0.42;
                    if !(0.0..1.0).contains(&k) {
                        continue;
                    }
                    let bow = (rnd(i, seed) - 0.5) * 90.0 * s;
                    for tr in 1..4 {
                        let kk = k - tr as f32 * 0.045;
                        if kk > 0.0 {
                            let p = curve(a, b, bow, kk);
                            self.r.circle(p.0, p.1, (3.0 - tr as f32 * 0.6) * s, light.a(0.4 - tr as f32 * 0.1));
                        }
                    }
                    let p = curve(a, b, bow, k);
                    let ang = t * 16.0 + i as f32;
                    self.r.poly(&leaf_pts(p, 19.0 * s, 9.0 * s, ang), dark);
                    self.r.poly(&leaf_pts(p, 16.0 * s, 7.0 * s, ang), col);
                    let (cs, sn) = (ang.cos() * 7.0 * s, ang.sin() * 7.0 * s);
                    self.r.line(p.0 - cs, p.1 - sn, p.0 + cs, p.1 + sn, light, 1.2);
                }
                if e >= 0.0 {
                    self.r.glow(b.0, b.1, 40.0 * s, col.a(0.55 * (1.0 - e / 0.3).max(0.0)));
                    self.draw_spray(Spray { rad: 2.4, speed: 140.0, ..spray(seed, 12, light) }, b, e, s);
                    let k = e / 1.0;
                    if k < 1.0 {
                        for j in 0..3 {
                            let x = b.0 + (j as f32 - 1.0) * 20.0 * s + (e * 6.0 + j as f32).sin() * 10.0 * s;
                            let y = b.1 + 40.0 * s * e;
                            self.r.poly(&leaf_pts((x, y), 14.0 * s, 6.0 * s, (e * 5.0 + j as f32).sin() * 0.8), col.a(1.0 - k));
                        }
                    }
                }
            }
            Fx::Rock => {
                // Rocks crash down on it one after another, bounce off and throw up dust.
                let (brown, pale, dark) = (Rgba::rgb(150, 125, 90), Rgba::rgb(205, 185, 145), Rgba::rgb(95, 78, 55));
                let fall = 0.32;
                for i in 0..4u32 {
                    let t0 = 0.1 + i as f32 * 0.09;
                    let k = (t - t0) / fall;
                    if k < 0.0 {
                        continue;
                    }
                    let dx = [-18.0, 14.0, -2.0, 22.0][i as usize] * s;
                    let land = b.1 + [10.0, -4.0, 4.0, 14.0][i as usize] * s;
                    let r = (9.0 + 3.0 * (i % 3) as f32) * s;
                    let (x, y, al) = if k < 1.0 {
                        (b.0 + dx, land - 80.0 * s * (1.0 - k * k), (k * 4.0).min(1.0))
                    } else {
                        let e2 = (k - 1.0) * fall;
                        let side = if dx < 0.0 { -1.0 } else { 1.0 };
                        (b.0 + dx + side * e2 * 40.0 * s, land - 18.0 * s * (PI * (e2 / 0.28).min(1.0)).sin(), 1.0 - clamp01((e2 - 0.28) / 0.3))
                    };
                    if al <= 0.0 {
                        continue;
                    }
                    let rot = t * 4.0 + i as f32;
                    self.r.poly(&rock_pts((x, y), r * 1.12, i, rot), dark.a(al));
                    self.r.poly(&rock_pts((x, y), r, i, rot), brown.a(al));
                    self.r.poly(&rock_pts((x - r * 0.25, y - r * 0.25), r * 0.45, i + 9, rot), pale.a(al));
                    if k >= 1.0 {
                        let e2 = (k - 1.0) * fall;
                        let q = e2 / 0.5;
                        if q < 1.0 {
                            for j in 0..4 {
                                self.r.circle(b.0 + dx + (j as f32 - 1.5) * 8.0 * s, land + r * 0.6 - 10.0 * s * q, (4.0 + 10.0 * q) * s, Rgba::rgb(190, 170, 130).a(0.5 * (1.0 - q)));
                            }
                        }
                        self.draw_spray(Spray { dir: -PI / 2.0, spread: 2.6, speed: 150.0, grav: 450.0, rad: 2.2, ..spray(seed + i, 6, brown) }, (b.0 + dx, land), e2, s);
                    }
                }
            }
            Fx::Quake => {
                // The ground rumbles, then cracks open under it and throws up rocks and dust.
                let ground = b.1 + size * 0.45;
                let brown = Rgba::rgb(150, 125, 90);
                if (0.25..HIT).contains(&t) {
                    for j in 0..5 {
                        let x = b.0 + (j as f32 - 2.0) * 18.0 * s;
                        self.r.circle(x, ground - (t * 30.0 + j as f32).sin().abs() * 6.0 * s, 2.2 * s, brown);
                    }
                }
                if e >= 0.0 && t < 1.65 {
                    let fade = clamp01((1.65 - t) / 0.4);
                    let grow = ease_out(e / 0.3);
                    for (n, (side, br)) in [(-1.0f32, 0.0f32), (-1.0, 1.0), (1.0, 0.0), (1.0, 1.0)].into_iter().enumerate() {
                        let len = (60.0 + 20.0 * br) * s * grow;
                        let crack = zigzag((b.0, ground), (b.0 + side * len, ground + (br * 2.0 - 1.0) * 6.0 * s), 6, 4.0 * s, seed + n as u32);
                        self.polyline(&crack, Rgba::rgb(70, 50, 30).a(0.85 * fade), 3.2 * s);
                        self.polyline(&crack, Rgba::rgb(210, 165, 100).a(0.7 * fade), 1.2);
                    }
                    self.draw_spray(Spray { dir: -PI / 2.0, spread: 2.0, speed: 230.0, grav: 600.0, rad: 3.2, dur: 0.8, ..spray(seed, 16, brown) }, (b.0, ground), e, s);
                    for j in 0..7 {
                        let k = clamp01((e - (j % 3) as f32 * 0.05) / 0.6);
                        if k < 1.0 {
                            let x = b.0 + (j as f32 - 3.0) * 14.0 * s;
                            self.r.circle(x, ground - 14.0 * s * k, (6.0 + 10.0 * k) * s, Rgba::rgb(190, 165, 120).a(0.55 * (1.0 - k)));
                        }
                    }
                }
            }
            Fx::Rings => {
                // Waves of power ripple from the attacker into it; it's wrapped in a pulsing
                // aura (with dark wisps rising off it for a Ghost or Dark move).
                for i in 0..4 {
                    let t0 = RELEASE - 0.06 + i as f32 * 0.06;
                    let k = (t - t0) / 0.3;
                    if (0.0..1.0).contains(&k) {
                        let p = lerp(a, b, k);
                        self.r.ring(p.0, p.1, (6.0 + 14.0 * k) * s, col.a(0.9 * (1.0 - k * 0.5)), 3.0 * s);
                    }
                }
                for i in 0..4 {
                    let k = (t - (HIT + i as f32 * 0.13)) / 0.5;
                    if (0.0..1.0).contains(&k) {
                        self.r.ring(b.0, b.1, (10.0 + 46.0 * k) * s, light.a(0.9 * (1.0 - k)), (4.0 * (1.0 - k) + 1.0) * s);
                    }
                }
                let al = life(t, HIT - 0.05, 1.5);
                if al > 0.0 {
                    self.r.glow(b.0, b.1, 42.0 * s * (1.0 + 0.15 * (t * 18.0).sin()), col.a(0.55 * al));
                    if matches!(tname, "Ghost" | "Dark") {
                        let wisp = col.mix(black, 0.5);
                        for j in 0..6 {
                            let k = (t - (HIT - 0.2 + j as f32 * 0.1)) / 0.7;
                            if (0.0..1.0).contains(&k) {
                                let ang = j as f32 * 1.1 + k * 3.0;
                                let (x, y) = (b.0 + ang.cos() * 24.0 * s, b.1 + 20.0 * s - 50.0 * s * k);
                                self.r.glow(x, y, 12.0 * s, col.a(0.5 * (1.0 - k)));
                                self.r.circle(x, y, 5.0 * s * (1.0 - k * 0.6), wisp.a(0.75 * (1.0 - k)));
                            }
                        }
                    } else {
                        for j in 0..4 {
                            let ang = t * 4.0 + j as f32 * TAU / 4.0;
                            self.twinkle(b.0 + ang.cos() * 30.0 * s, b.1 + ang.sin() * 24.0 * s, 5.0 * s, WHITE.a(al));
                        }
                    }
                }
            }
            Fx::Sound => {
                // Sound waves pour out at it, and it shudders.
                let mid = if dir > 0.0 { 0.0 } else { PI };
                for i in 0..5 {
                    let k = (t - (RELEASE - 0.1 + i as f32 * 0.08)) / 0.32;
                    if (0.0..1.0).contains(&k) {
                        let p = lerp(a, b, k);
                        let r = (12.0 + 26.0 * k) * s;
                        let wave = arc_path(p, r, r, mid - 0.95, mid + 0.95);
                        self.r.poly(&taper(&wave, 10.0 * s), col.a(0.6 * (1.0 - k * 0.5)));
                        self.r.poly(&taper(&wave, 5.0 * s), light.a(1.0 - k * 0.5));
                    }
                }
                let al = life(t, HIT, 1.4);
                if al > 0.0 {
                    for j in 0..8 {
                        let ang = j as f32 / 8.0 * TAU;
                        let r0 = 28.0 * s + (t * 40.0 + j as f32).sin() * 3.0 * s;
                        let (cs, sn) = (ang.cos(), ang.sin());
                        self.r.line(b.0 + cs * r0, b.1 + sn * r0, b.0 + cs * (r0 + 10.0 * s), b.1 + sn * (r0 + 10.0 * s), WHITE.a(0.8 * al), 2.4 * s);
                    }
                    self.r.glow(b.0, b.1, 36.0 * s, col.a(0.4 * al));
                }
            }
            Fx::Orb => {
                // A ball of energy forms, flies at it trailing light and bursts.
                let path = |k: f32| {
                    let p = lerp(a, b, k);
                    (p.0, p.1 - (PI * k).sin() * 22.0 * s)
                };
                if t < HIT {
                    let k = clamp01((t - RELEASE) / (HIT - RELEASE));
                    let r = (4.0 + 8.0 * clamp01(t / RELEASE)) * s;
                    if k > 0.0 {
                        for j in 1..6 {
                            let kk = k - j as f32 * 0.05;
                            if kk < 0.0 {
                                break;
                            }
                            let p = path(kk);
                            self.r.circle(p.0, p.1, r * (1.0 - j as f32 * 0.14), col.a(0.4 * (1.0 - j as f32 / 6.0)));
                        }
                    }
                    let p = path(k);
                    self.r.glow(p.0, p.1, r * 3.2, col.a(0.75));
                    self.r.circle(p.0, p.1, r, col.mix(black, 0.25));
                    for j in 0..2 {
                        let ang = t * 20.0 + j as f32 * PI;
                        self.r.circle(p.0 + ang.cos() * r * 0.55, p.1 + ang.sin() * r * 0.55, r * 0.3, light);
                    }
                    self.r.circle(p.0 - r * 0.3, p.1 - r * 0.3, r * 0.25, WHITE.a(0.85));
                }
                if e >= 0.0 {
                    let k = e / 0.45;
                    if k < 1.0 {
                        self.r.glow(b.0, b.1, (30.0 + 40.0 * k) * s, light.a(1.0 - k));
                        if k < 0.5 {
                            let ro = (16.0 + 30.0 * ease_out(k * 2.0)) * s;
                            self.r.poly(&star_pts(b, ro, ro * 0.45, 10, 0.2), light.a(1.0 - k * 2.0));
                        }
                        self.r.ring(b.0, b.1, (10.0 + 50.0 * k) * s, col.a(1.0 - k), (4.0 * (1.0 - k) + 1.0) * s);
                    }
                    let k2 = (e - 0.1) / 0.45;
                    if (0.0..1.0).contains(&k2) {
                        self.r.ring(b.0, b.1, (10.0 + 40.0 * k2) * s, WHITE.a(0.7 * (1.0 - k2)), 2.0 * s);
                    }
                    self.draw_spray(Spray { speed: 180.0, glow: true, rad: 2.6, ..spray(seed, 18, light) }, b, e, s);
                    for j in 0..5u32 {
                        let k = (e - 0.15 - j as f32 * 0.08) / 0.7;
                        if (0.0..1.0).contains(&k) {
                            let x = b.0 + (rnd(j, seed) - 0.5) * 40.0 * s;
                            self.r.circle(x, b.1 - 40.0 * s * k, (5.0 + 8.0 * k) * s, col.mix(black, 0.3).a(0.4 * (1.0 - k)));
                        }
                    }
                }
            }
            Fx::Stars => {
                // A stream of spinning stars curves in, each bursting on it.
                let gold = if tname == "Fairy" { Rgba::rgb(255, 170, 220) } else { Rgba::rgb(255, 225, 110) };
                for i in 0..6u32 {
                    let t0 = 0.12 + i as f32 * 0.05;
                    let k = (t - t0) / 0.36;
                    if k < 0.0 {
                        continue;
                    }
                    let bow = (i as f32 - 2.5) * 14.0 * s;
                    if k < 1.0 {
                        let p = curve(a, b, bow, k);
                        if k > 0.1 {
                            let q = curve(a, b, bow, k - 0.1);
                            self.twinkle(q.0, q.1, 4.0 * s, WHITE.a(0.7));
                        }
                        let rot = t * 10.0 + i as f32;
                        self.r.glow(p.0, p.1, 16.0 * s, gold.a(0.5));
                        self.r.poly(&star_pts(p, 9.0 * s, 4.0 * s, 5, rot), gold);
                        self.r.poly(&star_pts(p, 4.5 * s, 2.0 * s, 5, rot), WHITE);
                    } else {
                        let c = (b.0 + bow * 0.4, b.1 + (i as f32 - 2.5) * 5.0 * s);
                        self.draw_spray(Spray { rad: 2.0, speed: 120.0, glow: true, dur: 0.4, ..spray(seed + i, 6, gold) }, c, (k - 1.0) * 0.36, s);
                    }
                }
                let al = life(t, HIT - 0.1, 1.0);
                if al > 0.0 {
                    self.r.glow(b.0, b.1, 40.0 * s, gold.a(0.45 * al));
                }
            }
            Fx::Wind => {
                // Blades of air slice across, then a whirlwind spins round it.
                let mid = if dir > 0.0 { 0.0 } else { PI };
                for i in 0..4 {
                    let k = (t - (0.12 + i as f32 * 0.07)) / 0.35;
                    if !(0.0..1.0).contains(&k) {
                        continue;
                    }
                    let p = lerp(a, b, ease_out(k));
                    let p = (p.0, p.1 + (i as f32 - 1.5) * 12.0 * s);
                    let blade = arc_path(p, 18.0 * s, 18.0 * s, mid - 1.1, mid + 1.1);
                    self.r.poly(&taper(&blade, 7.0 * s), WHITE.a(0.85 * (1.0 - k * 0.4)));
                    self.r.poly(&taper(&blade, 2.6 * s), light.a(1.0 - k * 0.4));
                }
                let al = life(t, HIT - 0.05, 1.5);
                if al > 0.0 {
                    for j in 0..3 {
                        let r = (18.0 + j as f32 * 9.0) * s;
                        let rot = t * (11.0 - j as f32 * 2.0) + j as f32 * 2.0;
                        let c = (b.0, b.1 + (1.0 - j as f32) * 16.0 * s);
                        let swirl = arc_path(c, r, r * 0.4, rot, rot + 2.2);
                        self.r.poly(&taper(&swirl, 4.5 * s), WHITE.a(0.65 * al));
                    }
                }
            }
            Fx::Drain => {
                // It's hit, then glowing orbs float back to the attacker, which glows as it heals.
                let green = Rgba::rgb(140, 240, 120);
                let pale = Rgba::rgb(220, 255, 200);
                if e >= 0.0 {
                    let k = e / 0.35;
                    if k < 1.0 {
                        self.r.glow(b.0, b.1, 42.0 * s, col.a(0.6 * (1.0 - k)));
                        self.r.ring(b.0, b.1, (10.0 + 30.0 * k) * s, WHITE.a(0.8 * (1.0 - k)), 2.5 * s);
                    }
                    self.draw_spray(Spray { rad: 2.2, ..spray(seed, 10, col.mix(WHITE, 0.3)) }, b, e, s);
                }
                for i in 0..9u32 {
                    let k = (t - (HIT + 0.05 + i as f32 * 0.05)) / 0.5;
                    if !(0.0..1.0).contains(&k) {
                        continue;
                    }
                    let bow = ((i % 3) as f32 - 1.0) * 30.0 * s;
                    let smooth = k * k * (3.0 - 2.0 * k);
                    for tr in 1..3 {
                        let kk = smooth - tr as f32 * 0.05;
                        if kk > 0.0 {
                            let p = curve(b, a, bow, kk);
                            self.r.circle(p.0, p.1, (2.6 - tr as f32 * 0.7) * s, green.a(0.5 - tr as f32 * 0.15));
                        }
                    }
                    let p = curve(b, a, bow, smooth);
                    self.r.glow(p.0, p.1, 11.0 * s, green.a(0.8));
                    self.r.circle(p.0, p.1, 3.2 * s, pale);
                }
                let al = life(t, 1.0, 1.75);
                if al > 0.0 {
                    self.r.glow(a.0, a.1, 42.0 * s, green.a(0.5 * al));
                    for j in 0..5 {
                        let k = (t * 1.5 + j as f32 / 5.0) % 1.0;
                        self.twinkle(a.0 + (j as f32 - 2.0) * 12.0 * s, a.1 + 20.0 * s - 40.0 * s * k, 4.0 * s, pale.a(al * (1.0 - k)));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by_name(n: &str) -> Fx {
        fx_of(MOVES.iter().position(|m| m.name == n).unwrap() as u16)
    }

    #[test]
    fn moves_look_right() {
        assert_eq!(by_name("Dragon Claw"), Fx::Slash);
        assert_eq!(by_name("Shadow Claw"), Fx::Slash);
        assert_eq!(by_name("Crunch"), Fx::Bite);
        assert_eq!(by_name("Thunderbolt"), Fx::Bolt);
        assert_eq!(by_name("Thunder Punch"), Fx::Punch);
        assert_eq!(by_name("Flamethrower"), Fx::Stream);
        assert_eq!(by_name("Ice Beam"), Fx::Beam);
        assert_eq!(by_name("Earthquake"), Fx::Quake);
        assert_eq!(by_name("Shadow Ball"), Fx::Orb);
        assert_eq!(by_name("Close Combat"), Fx::Barrage);
        assert_eq!(by_name("Razor Leaf"), Fx::Leaf);
        assert_eq!(by_name("Psychic"), Fx::Rings);
        assert_eq!(by_name("Tackle"), Fx::Tackle);
        assert_eq!(by_name("Giga Drain"), Fx::Drain);
        assert_eq!(by_name("Hyper Voice"), Fx::Sound);
        assert_eq!(by_name("Swift"), Fx::Stars);
    }
}
