//! Little hand-drawn pixel Pokémon (fan art) that take Pip's place in the pill. Rayquaza is
//! the island's own colour (idle, needs-you); while music plays, the album art's colour picks
//! the Pokémon.
//!
//! Sprite cells: '.' is empty; other letters are palette colours. Special cells animate:
//! 'E' eyes (blink to `blink`), 'F'/'f' flames (swap each frame), '*' sparks (flash),
//! 'Y' on Umbreon glows.

pub struct Mon {
    pub name: &'static str,
    pub rows: &'static [&'static str],
    pub palette: &'static [(char, u32)],
    /// The colour eyes take while blinking or asleep.
    pub blink: u32,
    /// Slithers (each row sways in a travelling wave) instead of breathing.
    pub serpent: bool,
}

impl Mon {
    pub fn colour(&self, c: char) -> Option<u32> {
        self.palette.iter().find(|(k, _)| *k == c).map(|(_, v)| *v)
    }
    pub fn width(&self) -> usize {
        self.rows.iter().map(|r| r.len()).max().unwrap_or(0)
    }
}

pub const RAYQUAZA: Mon = Mon {
    name: "Rayquaza",
    rows: &[
        "..........gG....",
        ".........GGGG.g.",
        "........GGEGGGG.",
        ".......GGGGGRR..",
        "......GGgGG.....",
        ".....GGYGg......",
        "....GGGg........",
        "...GGYG.....gG..",
        "..GGGg.....GGYG.",
        "..gGGGGGGGGGGg..",
        "...gGYGGGYGGg...",
        ".....gggggg.....",
    ],
    palette: &[('G', 0x3fa055), ('g', 0x2b7040), ('Y', 0xf2cf4a), ('E', 0xf2cf4a), ('R', 0xd83a3a)],
    blink: 0x3fa055,
    serpent: true,
};

pub const PIKACHU: Mon = Mon {
    name: "Pikachu",
    rows: &[
        "K..........K....",
        "KY........YK....",
        ".YY......YY.....",
        ".YYYYYYYYYY...yy",
        "YYYYYYYYYYYY.yYy",
        "YYEYYYYYYEYY*Yy.",
        "RRYYYKKYYYRR.y..",
        "RRYYYYYYYYRR*Y..",
        ".YYYYYYYYYY.YY..",
        "..yYYYYYYYyYy...",
        "..yy....yy......",
    ],
    palette: &[('Y', 0xf7d23e), ('y', 0xd9a62a), ('K', 0x2b2b2b), ('E', 0x2b2b2b), ('R', 0xe5533d), ('*', 0xfff38a)],
    blink: 0xf7d23e,
    serpent: false,
};

pub const SCEPTILE: Mon = Mon {
    name: "Sceptile",
    rows: &[
        ".....GG.........",
        "....GGGGg.......",
        "...GEGGGg...DD..",
        "..GGGGGGg..DDD..",
        ".......GgG.DD...",
        "......GRRGgD....",
        ".....GGRRGGg....",
        "....G.GRRGg..DD.",
        "......GRRG..DDD.",
        "......GGGGgDDD..",
        ".....GG..GGDD...",
        "....ggg..ggg....",
    ],
    palette: &[('G', 0x57b35a), ('g', 0x2f7d36), ('D', 0x1f6a2c), ('R', 0xd9473b), ('E', 0xf2d94a)],
    blink: 0x57b35a,
    serpent: false,
};

pub const SWAMPERT: Mon = Mon {
    name: "Swampert",
    rows: &[
        ".......bb.......",
        "......bBBb......",
        "....BBBBBBBB....",
        "...BBEBBBBEBB...",
        "..OBBBBBBBBBBO..",
        ".OO.BBLLLLBB.OO.",
        "....BLLLLLLB....",
        "..BBBLLLLLLBBB..",
        ".BB.BBLLLLBB.BB.",
        "....BBBBBBBB....",
        "...bbb....bbb...",
    ],
    palette: &[('B', 0x4f95dc), ('b', 0x2f619f), ('L', 0xa9d4f2), ('O', 0xf39a3a), ('E', 0x1c1c24)],
    blink: 0x4f95dc,
    serpent: false,
};

pub const INFERNAPE: Mon = Mon {
    name: "Infernape",
    rows: &[
        "......F...F.....",
        ".....FF..FfF....",
        "....FfFFFffF....",
        "....WWWWWWW.....",
        "...WEWWWWEWW....",
        "...WWWWKWWWW....",
        "....OWWWWWO.....",
        "...OOOYYYOOO....",
        "..OO.OOOOO.OO...",
        ".OO..OOOOO..OO..",
        ".....OO..OO.....",
        "....bb....bb....",
    ],
    palette: &[('F', 0xe8432e), ('f', 0xffcc33), ('W', 0xf1ece0), ('O', 0xd26a2c), ('Y', 0xe8b84a), ('K', 0x3a2a2a), ('E', 0x1c1c24), ('b', 0x6b3f1e)],
    blink: 0xf1ece0,
    serpent: false,
};

pub const GARCHOMP: Mon = Mon {
    name: "Garchomp",
    rows: &[
        "..pP............",
        ".PPPP.....p.....",
        "PPEPPP...pP.....",
        ".PPPPPPPPPP.....",
        "..RRRPPPPPP.....",
        "....PPPPPPPP....",
        "..PPPPPRRPPPPP..",
        ".PP..PPRRPP..PP.",
        ".p...PPRRPP...p.",
        ".....PPPPPPP....",
        "....PP...PPPp...",
        "...pp.....pppp..",
    ],
    palette: &[('P', 0x6a58b8), ('p', 0x463a86), ('R', 0xd6503f), ('E', 0xf2cf4a)],
    blink: 0x6a58b8,
    serpent: false,
};

pub const ARCEUS: Mon = Mon {
    name: "Arceus",
    rows: &[
        "........WW......",
        "......WWWW......",
        ".....gWWWW......",
        "....ggEWWW......",
        "......WWWWW.....",
        "....YYYWWWWYY...",
        "...Y..WWWWWW.Y..",
        "....YYWWWWWWYY..",
        "......WWWWWWW...",
        ".....WW.W..W.W..",
        ".....gg.g..g.g..",
    ],
    palette: &[('W', 0xf2eee2), ('g', 0xa3a3ab), ('Y', 0xe9c54a), ('E', 0x3fae6a)],
    blink: 0xf2eee2,
    serpent: false,
};

pub const CHARIZARD: Mon = Mon {
    name: "Charizard",
    rows: &[
        "...TT......TT...",
        "..TTT.OO..TTT...",
        ".TTT.OOOO.TTT...",
        ".TT.OEOOOO.TT...",
        ".T..OOOOO...T...",
        ".....OCCO.......",
        "....OOCCOO....F.",
        "...O.OCCOO...Ff.",
        ".....OCCOoooO.F.",
        "....OO..OO......",
        "...oo....oo.....",
    ],
    palette: &[('O', 0xf08a33), ('o', 0xc4651f), ('C', 0xf6d38c), ('T', 0x3a8f8f), ('F', 0xff5a2a), ('f', 0xffd23a), ('E', 0x1c1c24)],
    blink: 0xf08a33,
    serpent: false,
};

pub const LAPRAS: Mon = Mon {
    name: "Lapras",
    rows: &[
        "..LL............",
        ".LELL...........",
        ".LLLLl..........",
        "..LLL...........",
        "...LL...........",
        "...LL.SsSsS.....",
        "...LLSSSSSSS....",
        "..LLLSsSSsSSS...",
        ".CCCCCCCCCCCLL..",
        "..LLLLLLLLLLl...",
        "...ll.....ll....",
    ],
    palette: &[('L', 0x6cc0e8), ('l', 0x3f8fbf), ('S', 0x9a8c78), ('s', 0x6f6556), ('C', 0xf2e6c4), ('E', 0x1c1c24)],
    blink: 0x6cc0e8,
    serpent: false,
};

pub const MEW: Mon = Mon {
    name: "Mew",
    rows: &[
        "...P..P.........",
        "..PPPPPP........",
        "..PEPPEP........",
        "..PPPPPP........",
        "...PPPP.........",
        "..PPPPPP.....p..",
        ".PP.PPPP....pP..",
        "....PPPP...pP...",
        "....PPPPPppP....",
        "...PP..PP.......",
    ],
    palette: &[('P', 0xf6a9c9), ('p', 0xd77fa5), ('E', 0x3c7fd8)],
    blink: 0xf6a9c9,
    serpent: false,
};

pub const UMBREON: Mon = Mon {
    name: "Umbreon",
    rows: &[
        "..K......K......",
        "..KK....KK......",
        "..KYK..KYK......",
        "...KKKKKK.......",
        "..KKEKKEKK......",
        "...KKKKKK.......",
        "....KKKK.....KK.",
        "...KKKKKKKKKKYK.",
        "..KYKKKKKKKKK...",
        "..KK.KK..KK.KK..",
        "..k..k....k..k..",
    ],
    palette: &[('K', 0x454559), ('k', 0x2f2f3d), ('Y', 0xf2cf4a), ('E', 0xe8473b)],
    blink: 0x454559,
    serpent: false,
};

pub const EEVEE: Mon = Mon {
    name: "Eevee",
    rows: &[
        ".B.......B......",
        ".BB.....BB......",
        ".BbB...BbB......",
        "..BBBBBBB.......",
        ".BBEBBBEBB......",
        "..BBBBBBB....BB.",
        "..CCCCCCC...BBC.",
        "...CBBBBBBBBBC..",
        "....BBBBBBBB....",
        "....BB.BB.BB.BB.",
        "....bb.bb.bb.bb.",
    ],
    palette: &[('B', 0xb37a42), ('b', 0x8a5a2c), ('C', 0xf2dcae), ('E', 0x1c1c24)],
    blink: 0xb37a42,
    serpent: false,
};

/// An animated sprite: every frame side by side in one picture, and how long each shows.
pub struct Strip {
    pub art: crate::events::Art,
    pub w: u32,
    pub h: u32,
    pub delays: Vec<u32>,
}

/// The frame to show `ms` milliseconds into a looping animation with these frame times.
pub fn frame_at(delays: &[u32], ms: u64) -> usize {
    let total: u64 = delays.iter().map(|d| *d as u64).sum::<u64>().max(1);
    let mut t = ms % total;
    for (i, d) in delays.iter().enumerate() {
        if t < *d as u64 {
            return i;
        }
        t -= *d as u64;
    }
    0
}

/// Shrink every frame of a strip (`frames` frames of `w`×`h`, premultiplied) to `fw`×`fh`,
/// averaging the area each new pixel covers. Done once, so drawing a frame is a plain copy.
pub fn shrink(art: &crate::events::Art, w: u32, h: u32, frames: usize, fw: u32, fh: u32) -> crate::events::Art {
    // For each new pixel along one axis: the old pixels it covers and how much of each.
    let weights = |n_src: u32, n_dst: u32| -> Vec<Vec<(usize, f32)>> {
        let s = n_src as f32 / n_dst as f32;
        (0..n_dst)
            .map(|d| {
                let (a, b) = (d as f32 * s, (d + 1) as f32 * s);
                (a.floor() as usize..(b.ceil() as usize).min(n_src as usize))
                    .filter_map(|i| {
                        let cover = (i as f32 + 1.0).min(b) - (i as f32).max(a);
                        (cover > 0.0).then_some((i, cover / s))
                    })
                    .collect()
            })
            .collect()
    };
    let (wx, wy) = (weights(w, fw), weights(h, fh));
    let (fw, fh, stride) = (fw as usize, fh as usize, art.w as usize * 4);
    let out_w = fw * frames;
    let mut out = vec![0u8; out_w * fh * 4];
    let mut tmp = vec![0f32; h as usize * fw * 4];
    for f in 0..frames {
        let x0 = f * w as usize;
        tmp.iter_mut().for_each(|v| *v = 0.0);
        for y in 0..h as usize {
            for (dx, ws) in wx.iter().enumerate() {
                for &(sx, k) in ws {
                    let si = y * stride + (x0 + sx) * 4;
                    for c in 0..4 {
                        tmp[(y * fw + dx) * 4 + c] += art.px[si + c] as f32 * k;
                    }
                }
            }
        }
        for (dy, ws) in wy.iter().enumerate() {
            for dx in 0..fw {
                let mut acc = [0f32; 4];
                for &(sy, k) in ws {
                    for c in 0..4 {
                        acc[c] += tmp[(sy * fw + dx) * 4 + c] * k;
                    }
                }
                let oi = (dy * out_w + f * fw + dx) * 4;
                for c in 0..4 {
                    out[oi + c] = acc[c].round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    crate::events::Art { w: out_w as u32, h: fh as u32, px: out, tint: art.tint }
}

/// How many milliseconds from `ms` until the animation moves on to its next frame.
pub fn ms_to_next_frame(delays: &[u32], ms: u64) -> u64 {
    let total: u64 = delays.iter().map(|d| *d as u64).sum::<u64>().max(1);
    let mut t = ms % total;
    for d in delays {
        if t < *d as u64 {
            return *d as u64 - t;
        }
        t -= *d as u64;
    }
    1
}

/// The real animated sprite for a Pokémon, if it's been downloaded to
/// `~/.config/pocketisle/sprites/<name>.png` (+ `.json` with the frame size and timings).
pub fn strip_named(name: &str) -> Option<Strip> {
    let dir = crate::util::sprite_dir();
    let name = name.to_lowercase();
    let meta: serde_json::Value = serde_json::from_str(std::fs::read_to_string(dir.join(format!("{name}.json"))).ok()?.trim_start_matches('\u{feff}')).ok()?;
    let (w, h) = (meta["w"].as_u64()? as u32, meta["h"].as_u64()? as u32);
    let frames = meta["frames"].as_u64()? as usize;
    let mut delays: Vec<u32> = match &meta["delays"] {
        serde_json::Value::Array(a) => a.iter().filter_map(|d| d.as_u64()).map(|d| d as u32).collect(),
        v => v.as_u64().map(|d| vec![d as u32]).unwrap_or_default(),
    };
    delays.resize(frames.max(1), 100);
    let art = crate::imaging::decode(&std::fs::read(dir.join(format!("{name}.png"))).ok()?, u32::MAX).ok()?;
    (art.w >= w && w > 0 && h > 0).then_some(Strip { art, w, h, delays })
}

pub const ALL: [&Mon; 12] = [&RAYQUAZA, &PIKACHU, &SCEPTILE, &SWAMPERT, &INFERNAPE, &GARCHOMP, &ARCEUS, &CHARIZARD, &LAPRAS, &MEW, &UMBREON, &EEVEE];

/// The first frame of a downloaded sprite, as a picture of its own (for cards and the Pokédex).
pub fn first_frame(name: &str) -> Option<crate::events::Art> {
    let s = strip_named(name)?;
    let (w, h) = (s.w as usize, s.h as usize);
    let stride = s.art.w as usize * 4;
    let mut px = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        px.extend_from_slice(s.art.px.get(y * stride..y * stride + w * 4)?);
    }
    Some(crate::events::Art { w: s.w, h: s.h, px, tint: s.art.tint })
}

/// Each team member's evolution line: the sprite for each stage and the team level it's
/// reached at. Everyone starts at the first stage.
pub const LINES: [(&str, &[(&str, u32)]); 7] = [
    ("Pikachu", &[("pichu", 1), ("pikachu", 12)]),
    ("Sceptile", &[("treecko", 1), ("grovyle", 16), ("sceptile", 36)]),
    ("Swampert", &[("mudkip", 1), ("marshtomp", 16), ("swampert", 36)]),
    ("Infernape", &[("chimchar", 1), ("monferno", 14), ("infernape", 36)]),
    ("Garchomp", &[("gible", 1), ("gabite", 24), ("garchomp", 48)]),
    ("Charizard", &[("charmander", 1), ("charmeleon", 16), ("charizard", 36)]),
    ("Umbreon", &[("eevee", 1), ("umbreon", 25)]),
];

/// The ones that never evolve turn shiny at this level instead.
pub const SHINY_LEVEL: u32 = 50;
const SHINIES: [(&str, &str); 5] = [("Rayquaza", "rayquaza-shiny"), ("Lapras", "lapras-shiny"), ("Mew", "mew-shiny"), ("Arceus", "arceus-shiny"), ("Eevee", "eevee-shiny")];

/// The sprite a team member shows at a team level.
pub fn team_sprite(mon: &Mon, level: u32) -> String {
    for (m, stages) in LINES {
        if m == mon.name {
            return stages.iter().rev().find(|(_, l)| level >= *l).map_or(stages[0].0, |s| s.0).to_string();
        }
    }
    for (m, s) in SHINIES {
        if m == mon.name && level >= SHINY_LEVEL {
            return s.to_string();
        }
    }
    mon.name.to_lowercase()
}

/// Who changed between two team levels, for the team members `keep` picks (by name):
/// (from, to) sprite names.
pub fn changes_of(old: u32, new: u32, keep: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    for mon in ALL.iter().filter(|m| keep(m.name)) {
        let (a, b) = (team_sprite(mon, old), team_sprite(mon, new));
        if a != b && !v.contains(&(a.clone(), b.clone())) {
            v.push((a, b));
        }
    }
    v
}

/// The next level one of the team members `keep` picks evolves (or turns shiny) at, and who.
pub fn next_change_of(level: u32, keep: impl Fn(&str) -> bool) -> Option<(u32, Vec<(String, String)>)> {
    let mut levels: Vec<u32> = LINES.iter().flat_map(|(_, s)| s.iter().map(|x| x.1)).chain([SHINY_LEVEL]).filter(|l| *l > level).collect();
    levels.sort_unstable();
    levels.dedup();
    levels.into_iter().map(|l| (l, changes_of(level, l, &keep))).find(|(_, ch)| !ch.is_empty())
}

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::national::{Species, EVOLUTIONS, FORMS, FORM_KINDS, NATIONAL, REGIONS};

/// "chimchar" -> "Chimchar", "mew-shiny" -> "Shiny Mew", "mr-mime" -> "Mr. Mime".
pub fn pretty(sprite: &str) -> String {
    let (base, shiny) = match sprite.strip_suffix("-shiny") {
        Some(b) => (b, true),
        None => (sprite, false),
    };
    let n = match species(base) {
        Some(s) => s.display.to_string(),
        None => {
            let mut c = base.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    };
    if shiny {
        format!("Shiny {n}")
    } else {
        n
    }
}

/// Only ever hatched from an Egg: the babies (Pichu, Togepi, Riolu…) and a few more.
const EGG_ONLY: [&str; 3] = ["larvitar", "deino", "axew"];
/// Only out on special days and nights.
pub const SPECIALS: [&str; 8] = ["clefairy", "deoxys", "misdreavus", "shuppet", "duskull", "drifloon", "delibird", "luvdisc"];
/// The legendaries that come with their own weather or time of day instead of from the
/// everyday legendary pool, and when: the birds, the beasts and the lake trio.
pub const ROAMERS: [(&str, &str); 9] = [
    ("zapdos", "thunderstorm or wind"),
    ("articuno", "cold or snow"),
    ("moltres", "hot sunny day"),
    ("raikou", "thunderstorm or downpour"),
    ("entei", "hot sunny day"),
    ("suicune", "rain"),
    ("uxie", "dawn"),
    ("mesprit", "dusk"),
    ("azelf", "night"),
];
/// The National Pokédex: all of them, every region.
pub const DEX_SIZE: usize = NATIONAL.len();

pub fn dex_list() -> impl Iterator<Item = &'static str> {
    NATIONAL.iter().map(|s| s.name)
}

/// The Pokédex's Forms page: regional forms, Mega Evolutions and Gigantamax forms.
pub const FORMS_SIZE: usize = FORMS.len();

pub fn form_list() -> impl Iterator<Item = &'static str> {
    FORMS.iter().map(|s| s.name)
}

/// A Pokémon or form by its sprite name ("mr-mime", "vulpix-alola", or with "-shiny").
pub fn species(name: &str) -> Option<&'static Species> {
    static INDEX: OnceLock<HashMap<&'static str, &'static Species>> = OnceLock::new();
    let index = INDEX.get_or_init(|| NATIONAL.iter().chain(FORMS.iter()).map(|s| (s.name, s)).collect());
    index.get(name.strip_suffix("-shiny").unwrap_or(name)).copied()
}

/// Where it's from: Kanto, Johto, … Paldea; for a form, its kind (Alola, Mega, Gigantamax…).
pub fn region(s: &Species) -> &'static str {
    if s.form > 0 {
        return FORM_KINDS[s.form as usize];
    }
    REGIONS[(s.generation as usize).clamp(1, REGIONS.len()) - 1]
}

const FORM_MEGA: u8 = 5;
const FORM_GMAX: u8 = 6;
/// The team level at which a caught Pokémon can Mega Evolve (or undergo Primal Reversion).
pub const MEGA_LEVEL: u32 = 40;

fn regional(s: &Species) -> bool {
    (1..FORM_MEGA).contains(&s.form)
}

/// Pokémon that only evolve from a regional form (Galarian Meowth → Perrserker, Hisuian
/// Qwilfish → Overqwil, Paldean Wooper → Clodsire…), by National number.
const REGIONAL_EVOS: [u16; 9] = [862, 863, 864, 865, 866, 867, 903, 904, 980];

/// Gigantamax Pokémon: rare visitors, more of them in storms and strong wind.
pub fn gigantamax() -> &'static [&'static str] {
    static G: OnceLock<Vec<&'static str>> = OnceLock::new();
    G.get_or_init(|| FORMS.iter().filter(|s| s.form == FORM_GMAX).map(|s| s.name).collect())
}

/// How likely a wild visitor is to be Gigantamax in this weather.
pub fn gigantamax_odds(c: &Conditions) -> f32 {
    if c.sky == Some(Kind::Storm) || c.windy {
        1.0 / 10.0
    } else {
        1.0 / 50.0
    }
}

/// The Mega Evolutions (and Primal Reversions) of a Pokémon: Charizard has two.
pub fn megas_of(name: &str) -> Vec<&'static str> {
    let Some(s) = species(name) else { return Vec::new() };
    if s.form != 0 {
        return Vec::new();
    }
    FORMS.iter().filter(|f| f.form == FORM_MEGA && f.base == s.id).map(|f| f.name).collect()
}

/// What a caught Pokémon grows into as the team levels up, with the level and whether it
/// carries over a shiny: its evolutions (`count` caught take one way each), then its Megas.
pub fn grows_into(name: &str, count: u32) -> Vec<(&'static str, u32, bool)> {
    let mut v: Vec<(&'static str, u32, bool)> =
        evolutions_of(name).into_iter().enumerate().filter(|(k, _)| (*k as u32) < count).map(|(k, (to, at))| (to, at, k == 0)).collect();
    v.extend(megas_of(name).into_iter().map(|m| (m, MEGA_LEVEL, true)));
    v
}

fn egg_only(s: &Species) -> bool {
    s.baby || EGG_ONLY.contains(&s.name)
}

/// Everyday visitors: everyone but the legendaries, the Egg-only and the special-day ones.
pub fn wild() -> &'static [&'static str] {
    static W: OnceLock<Vec<&'static str>> = OnceLock::new();
    W.get_or_init(|| {
        NATIONAL
            .iter()
            .filter(|s| !s.legend && !egg_only(s) && !SPECIALS.contains(&s.name))
            .chain(FORMS.iter().filter(|s| regional(s) && !s.legend))
            .map(|s| s.name)
            .collect()
    })
}

/// Rare visitors (1 in 30), harder to catch: legendaries, mythicals, Ultra Beasts and Paradox
/// Pokémon (not the roamers, who come with their weather or time of day, or the special-night
/// ones).
pub fn legends() -> &'static [&'static str] {
    static L: OnceLock<Vec<&'static str>> = OnceLock::new();
    L.get_or_init(|| {
        NATIONAL
            .iter()
            .filter(|s| s.legend && !ROAMERS.iter().any(|r| r.0 == s.name) && !SPECIALS.contains(&s.name))
            .chain(FORMS.iter().filter(|s| regional(s) && s.legend))
            .map(|s| s.name)
            .collect()
    })
}

/// Every legendary, mythical, Ultra Beast and Paradox Pokémon, by region (one list for each
/// of `REGIONS`), in National order; legendary regional forms (the Galarian birds) go with
/// the region of their form.
pub fn legends_by_region() -> &'static [Vec<&'static str>] {
    static L: OnceLock<Vec<Vec<&'static str>>> = OnceLock::new();
    L.get_or_init(|| {
        let mut out: Vec<Vec<&'static str>> = vec![Vec::new(); REGIONS.len()];
        for s in NATIONAL.iter().filter(|s| s.legend) {
            out[(s.generation as usize).clamp(1, REGIONS.len()) - 1].push(s.name);
        }
        for s in FORMS.iter().filter(|s| regional(s) && s.legend) {
            let i = match FORM_KINDS[s.form as usize] {
                "Alola" => 6,
                "Paldea" => 8,
                _ => 7,
            };
            out[i].push(s.name);
        }
        out
    })
}

/// What an Egg can hatch into.
pub fn eggs() -> &'static [&'static str] {
    static E: OnceLock<Vec<&'static str>> = OnceLock::new();
    E.get_or_init(|| NATIONAL.iter().filter(|s| egg_only(s)).map(|s| s.name).collect())
}

/// Legendary (hard to catch; counted on the trainer card).
/// Friendship (0–255, like the games): a new buddy starts at FRIEND_START; at FRIEND_EVOLVE
/// it's happy enough for a friendship evolution.
pub const FRIEND_START: u32 = 70;
pub const FRIEND_EVOLVE: u32 = 160;
pub const FRIEND_MAX: u32 = 255;

/// When a friendship evolution can happen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum When {
    Any,
    Day,
    Night,
}

/// The Pokémon that evolve when happy (and never just by team level).
pub const FRIEND_EVOS: [(&str, &str, When); 19] = [
    ("eevee", "espeon", When::Day),
    ("eevee", "umbreon", When::Night),
    ("golbat", "crobat", When::Any),
    ("chansey", "blissey", When::Any),
    ("pichu", "pikachu", When::Any),
    ("cleffa", "clefairy", When::Any),
    ("igglybuff", "jigglypuff", When::Any),
    ("togepi", "togetic", When::Any),
    ("azurill", "marill", When::Any),
    ("budew", "roselia", When::Day),
    ("chingling", "chimecho", When::Night),
    ("buneary", "lopunny", When::Any),
    ("happiny", "chansey", When::Day),
    ("munchlax", "snorlax", When::Any),
    ("riolu", "lucario", When::Day),
    ("woobat", "swoobat", When::Any),
    ("swadloon", "leavanny", When::Any),
    ("type-null", "silvally", When::Any),
    ("snom", "frosmoth", When::Night),
];

/// Does `from` → `to` need friendship (rather than team level)?
pub fn is_friend_evo(from: &str, to: &str) -> bool {
    let from = from.strip_suffix("-shiny").unwrap_or(from);
    FRIEND_EVOS.iter().any(|(a, b, _)| *a == from && *b == to)
}

/// What a happy `from` evolves into now (day or night), if it's a friendship evolver.
pub fn friend_evolution(from: &str, day: bool) -> Option<&'static str> {
    let from = from.strip_suffix("-shiny").unwrap_or(from);
    FRIEND_EVOS
        .iter()
        .find(|(a, _, w)| *a == from && match w {
            When::Any => true,
            When::Day => day,
            When::Night => !day,
        })
        .map(|(_, b, _)| *b)
}

/// "Espeon by day, Umbreon at night" (empty when it doesn't evolve by friendship).
pub fn friend_evo_text(from: &str) -> String {
    FRIEND_EVOS
        .iter()
        .filter(|(a, _, _)| *a == from)
        .map(|(_, b, w)| {
            let when = match w {
                When::Any => "",
                When::Day => " by day",
                When::Night => " at night",
            };
            format!("{}{when}", pretty(b))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Friendship as five hearts: "♥♥♡♡♡".
pub fn hearts(f: u32) -> String {
    let n = (f.min(FRIEND_MAX) * 5 / FRIEND_MAX) as usize;
    format!("{}{}", "♥".repeat(n), "♡".repeat(5 - n))
}

/// What bites when you fish. The FISH_ONLY ones never walk by: you have to fish for them.
pub const FISH: &[&str] = &[
    "magikarp", "goldeen", "seaking", "horsea", "seadra", "kingdra", "tentacool", "tentacruel", "shellder", "staryu", "krabby", "poliwag",
    "psyduck", "slowpoke", "remoraid", "octillery", "qwilfish", "chinchou", "lanturn", "corsola", "mantine", "wailmer", "carvanha",
    "barboach", "whiscash", "corphish", "clamperl", "feebas", "luvdisc", "relicanth", "finneon", "lumineon", "basculin", "alomomola",
    "wishiwashi", "bruxish", "arrokuda", "barraskewda", "chewtle", "finizen", "wooper", "gyarados", "lapras", "milotic", "dratini",
];
pub const FISH_ONLY: &[&str] = &["feebas", "luvdisc", "relicanth", "finneon", "lumineon", "alomomola", "wishiwashi", "bruxish", "arrokuda"];

/// The fishing list's real species.
pub fn fish_pool() -> Vec<&'static str> {
    FISH.iter().filter_map(|n| species(n)).map(|s| s.name).collect()
}

pub fn is_legend(name: &str) -> bool {
    species(name).is_some_and(|s| s.legend)
}

/// What a Pokémon evolves into and at what team level (more than one for Eevee, Oddish's
/// Gloom, Kirlia and the like).
pub fn evolutions_of(name: &str) -> Vec<(&'static str, u32)> {
    let Some(s) = species(name) else { return Vec::new() };
    if s.form != 0 && !regional(s) {
        return Vec::new();
    }
    EVOLUTIONS
        .iter()
        .filter(|e| e.0 == s.base)
        .filter_map(|e| {
            let to = NATIONAL.get(e.1 as usize - 1)?;
            let only_regional = REGIONAL_EVOS.contains(&to.id);
            let name = if s.form == 0 {
                // A Kanto Meowth becomes Persian, never Perrserker.
                (!only_regional).then_some(to.name)?
            } else if let Some(f) = FORMS.iter().find(|f| f.base == to.id && f.form == s.form) {
                // Alolan Vulpix → Alolan Ninetales.
                f.name
            } else {
                // Galarian Meowth → Perrserker; Alolan Pikachu doesn't exist, so no Raichu.
                only_regional.then_some(to.name)?
            };
            Some((name, e.2 as u32))
        })
        .collect()
}

/// The one this evolves from (Charmeleon → Charmander, Alolan Ninetales → Alolan Vulpix,
/// Perrserker → Galarian Meowth), if any, and the team level the evolution happens at.
fn evolves_from(s: &'static Species) -> Option<(&'static Species, u32)> {
    let e = EVOLUTIONS.iter().find(|e| e.1 == s.base)?;
    let from = if s.form != 0 {
        FORMS.iter().find(|f| f.base == e.0 && f.form == s.form)?
    } else if REGIONAL_EVOS.contains(&s.id) {
        FORMS.iter().find(|f| f.base == e.0 && evolutions_of(f.name).iter().any(|(n, _)| *n == s.name))?
    } else {
        NATIONAL.get(e.0 as usize - 1)?
    };
    Some((from, e.2 as u32))
}

/// The member of a Pokémon's family that fits `level`: a Charizard at Lv 20 is a Charmeleon
/// and at Lv 10 a Charmander; a Charmander at Lv 40 is a Charizard (when it has only one way
/// to evolve, so an Eevee stays an Eevee). Never back into a baby. A Mega or Gigantamax
/// keeps its form from the Mega level on, if its plain form fits.
pub fn stage_at(name: &str, level: u32) -> String {
    let Some(orig) = species(name) else { return name.to_string() };
    let special = orig.form >= FORM_MEGA;
    let mut cur = if special { NATIONAL.get(orig.base as usize - 1).unwrap_or(orig) } else { orig };
    while let Some((from, at)) = evolves_from(cur) {
        if level >= at || from.baby {
            break;
        }
        cur = from;
    }
    loop {
        let evos = evolutions_of(cur.name);
        let [(next, at)] = evos.as_slice() else { break };
        match species(next) {
            Some(n) if level >= *at => cur = n,
            _ => break,
        }
    }
    if special && cur.id == orig.base && level >= MEGA_LEVEL {
        cur = orig;
    }
    if name.ends_with("-shiny") {
        format!("{}-shiny", cur.name)
    } else {
        cur.name.to_string()
    }
}

/// The first of a Pokémon's family (Charizard → Charmander), to tell families apart.
pub fn family(name: &str) -> &'static str {
    let Some(mut cur) = species(name) else { return "" };
    if cur.form >= FORM_MEGA {
        cur = NATIONAL.get(cur.base as usize - 1).unwrap_or(cur);
    }
    while let Some((from, _)) = evolves_from(cur) {
        cur = from;
    }
    cur.name
}

/// Evolved Pokémon turn up less often than first stages.
pub fn rarity(name: &str) -> f32 {
    match species(name).map(|s| s.stage) {
        Some(0) | None => 1.0,
        Some(1) => 0.35,
        _ => 0.12,
    }
}

/// The legendaries this weather might bring, with the weather's name. Each has a rare big
/// weather and an everyday one, so they all come even where it never snows: Zapdos in a
/// thunderstorm or the wind, Raikou in a thunderstorm or a downpour, Suicune in any rain,
/// Articuno in snow, hail or the cold, and Moltres and Entei in the heat (dry) or on a warm
/// sunny day (25°+).
pub fn weather_legends(c: &Conditions) -> Vec<(&'static str, &'static str)> {
    let mut v = Vec::new();
    match c.sky {
        Some(Kind::Storm) => v.extend([("zapdos", "thunderstorm"), ("raikou", "thunderstorm")]),
        Some(Kind::Snow) => v.push(("articuno", "snow")),
        Some(Kind::Hail) => v.push(("articuno", "hail")),
        Some(Kind::Downpour) => v.extend([("raikou", "downpour"), ("suicune", "rain")]),
        Some(Kind::Drizzle | Kind::Rain) => v.push(("suicune", "rain")),
        _ => {}
    }
    let has = |v: &Vec<(&str, &str)>, n: &str| v.iter().any(|w| w.0 == n);
    if c.windy && !has(&v, "zapdos") {
        v.push(("zapdos", "wind"));
    }
    if c.cold && !has(&v, "articuno") {
        v.push(("articuno", "cold"));
    }
    if c.hot && !c.wet() {
        v.extend([("moltres", "heat"), ("entei", "heat")]);
    } else if c.warm && c.day && c.sky == Some(Kind::Clear) {
        v.extend([("moltres", "sunshine"), ("entei", "sunshine")]);
    }
    v
}

/// One visitor in this many is one of the weather's legendaries, while that weather lasts.
pub const WEATHER_LEGEND_ODDS: f32 = 12.0;

/// The lake trio comes out at its time of day: Uxie at dawn (5–8 am), Mesprit at dusk
/// (5–8 pm) and Azelf the rest of the night. With the time's name.
pub fn time_legend(hour: u32, day: bool) -> Option<(&'static str, &'static str)> {
    match hour {
        5..=7 => Some(("uxie", "dawn")),
        17..=19 => Some(("mesprit", "dusk")),
        _ if !day => Some(("azelf", "night")),
        _ => None,
    }
}

/// One visitor in this many is the lake trio member out at that time (rarer than the weather
/// ones, since dawn, dusk and night come every day).
pub const TIME_LEGEND_ODDS: f32 = 30.0;

/// A special day or night: who comes out, who looks after the island, and how it shows.
pub struct Special {
    pub key: &'static str,
    /// "Halloween", "Full moon".
    pub name: &'static str,
    /// For the Pokédex tab.
    pub line: &'static str,
    /// For the island keeper's card: "Gengar came out for Halloween!"
    pub phrase: &'static str,
    /// This share of visitors comes from `visitors` (when downloaded).
    pub visitors: &'static [&'static str],
    pub share: f32,
    /// Its visitors are legendary (rarer to catch).
    pub legend: bool,
    pub keepers: &'static [&'static str],
    /// Shiny odds multiplied by this.
    pub shiny_x: f32,
}

pub static HALLOWEEN: Special = Special {
    key: "halloween",
    name: "Halloween",
    line: "Halloween week: ghosts everywhere",
    phrase: "for Halloween",
    visitors: &["gastly", "litwick", "misdreavus", "shuppet", "duskull", "drifloon", "zorua"],
    share: 0.5,
    legend: false,
    keepers: &["gengar", "mismagius", "chandelure", "banette", "dusknoir", "giratina"],
    shiny_x: 1.0,
};
pub static CHRISTMAS: Special = Special {
    key: "christmas",
    name: "Christmas",
    line: "Christmas: Delibird is out delivering",
    phrase: "for Christmas",
    visitors: &["delibird", "swablu", "snorlax", "piplup"],
    share: 0.4,
    legend: false,
    keepers: &["delibird", "glaceon", "articuno", "abomasnow"],
    shiny_x: 1.0,
};
pub static NEW_YEAR: Special = Special {
    key: "newyear",
    name: "New Year",
    line: "New Year: shinies twice as likely",
    phrase: "to see in the New Year",
    visitors: &[],
    share: 0.0,
    legend: false,
    keepers: &["victini", "jirachi", "togekiss"],
    shiny_x: 2.0,
};
pub static VALENTINE: Special = Special {
    key: "valentine",
    name: "Valentine's Day",
    line: "Valentine's Day: Luvdisc are out",
    phrase: "for Valentine's Day",
    visitors: &["luvdisc", "clefairy", "jigglypuff", "marill"],
    share: 0.4,
    legend: false,
    keepers: &["gardevoir", "milotic", "togekiss", "clefable"],
    shiny_x: 1.0,
};
pub static POKEMON_DAY: Special = Special {
    key: "pokemonday",
    name: "Pokémon Day",
    line: "Pokémon Day: starters out, shinies twice as likely",
    phrase: "for Pokémon Day",
    visitors: &["bulbasaur", "squirtle", "chikorita", "cyndaquil", "totodile", "piplup", "turtwig", "snivy", "tepig", "oshawott"],
    share: 0.5,
    legend: false,
    keepers: &["pikachu", "charizard", "blastoise", "venusaur"],
    shiny_x: 2.0,
};
pub static METEORS: Special = Special {
    key: "meteors",
    name: "meteor shower",
    line: "Meteor shower tonight: make a wish (Jirachi, Deoxys)",
    phrase: "with the meteor shower",
    visitors: &["jirachi", "deoxys"],
    share: 0.2,
    legend: true,
    keepers: &["jirachi", "deoxys", "rayquaza"],
    shiny_x: 1.0,
};
pub static FULL_MOON: Special = Special {
    key: "fullmoon",
    name: "full moon",
    line: "Full moon: Clefairy are dancing",
    phrase: "for the full moon",
    visitors: &["clefairy"],
    share: 0.35,
    legend: false,
    keepers: &["clefable", "cresselia"],
    shiny_x: 1.0,
};

/// Days since the last new moon (0 to 29.53), from a known new moon (6 Jan 2000, 18:14 UTC).
pub fn moon_age(unix_secs: i64) -> f64 {
    const SYNODIC: f64 = 29.530588853;
    (unix_secs as f64 / 86_400.0 - 10_962.76).rem_euclid(SYNODIC)
}

/// How much of the moon's face is lit (0 at new moon, 1 at full).
pub fn moon_lit_share(age: f64) -> f64 {
    let a = age / 29.530588853 * std::f64::consts::TAU;
    (1.0 - a.cos()) / 2.0
}

/// Is this point of the moon's face lit tonight? `nx`, `ny` go -1..1 across the disc (right
/// and down). From the north a waxing moon is lit on the right; from the south, the left.
pub fn moon_lit(age: f64, nx: f64, ny: f64, south: bool) -> bool {
    let nx = if south { -nx } else { nx };
    let a = age / 29.530588853 * std::f64::consts::TAU;
    let edge = (1.0 - ny * ny).max(0.0).sqrt() * a.cos();
    if a < std::f64::consts::PI {
        nx > edge
    } else {
        nx < -edge
    }
}

/// "Waxing gibbous" and so on.
pub fn moon_phase_name(age: f64) -> &'static str {
    const NAMES: [&str; 8] = ["New moon", "Waxing crescent", "First quarter", "Waxing gibbous", "Full moon", "Waning gibbous", "Last quarter", "Waning crescent"];
    NAMES[((age / 29.530588853 * 8.0 + 0.5).floor() as usize) % 8]
}

/// The moon is full (within a day of it).
pub fn full_moon(unix_secs: i64) -> bool {
    (moon_age(unix_secs) - 14.765).abs() <= 1.0
}

/// Peaks of the yearly meteor showers (month, day): Quadrantids, Lyrids, Eta Aquariids,
/// Perseids, Orionids, Leonids, Geminids.
const SHOWERS: [(u32, u32); 7] = [(1, 3), (4, 22), (5, 6), (8, 12), (10, 21), (11, 17), (12, 14)];

/// A night within two days of a meteor shower's peak, or one clear night in six for luck.
fn meteor_night(night_of: crate::calendar::Date) -> bool {
    let z = night_of.days();
    let near = SHOWERS.iter().any(|&(m, d)| (night_of.y - 1..=night_of.y + 1).any(|y| (crate::calendar::Date { y, m, d }.days() - z).abs() <= 2));
    near || (z.wrapping_mul(2_654_435_761) >> 7).rem_euclid(6) == 0
}

/// A special day by the calendar alone.
pub fn special_day(date: crate::calendar::Date) -> Option<&'static Special> {
    let (m, d) = (date.m, date.d);
    match (m, d) {
        (10, 25..) => Some(&HALLOWEEN),
        (12, 20..=26) => Some(&CHRISTMAS),
        (12, 31) | (1, 1) => Some(&NEW_YEAR),
        (2, 14) => Some(&VALENTINE),
        (2, 27) => Some(&POKEMON_DAY),
        _ => None,
    }
}

/// What tonight might bring (for the morning greeting): a meteor shower (if it's clear) or
/// a full moon. `unix_tonight` is a time late this evening.
pub fn tonight(date: crate::calendar::Date, unix_tonight: i64) -> Option<&'static Special> {
    if meteor_night(date) {
        Some(&METEORS)
    } else if full_moon(unix_tonight) {
        Some(&FULL_MOON)
    } else {
        None
    }
}

/// The Pokémon you only meet on special days and nights that you haven't caught yet, with
/// when to look for them.
pub fn special_missing(caught: impl Fn(&str) -> bool) -> Vec<(&'static str, &'static str)> {
    let mut v: Vec<(&'static str, &'static str)> = Vec::new();
    for s in [&FULL_MOON, &METEORS, &HALLOWEEN, &CHRISTMAS, &VALENTINE] {
        for &n in s.visitors {
            if !wild().contains(&n) && !caught(n) && !v.iter().any(|(m, _)| *m == n) {
                v.push((n, s.name));
            }
        }
    }
    for (n, when) in ROAMERS {
        if !caught(n) {
            v.push((n, when));
        }
    }
    v
}

/// The special days, in the order the trainer card shows them.
pub const SPECIAL_DAYS: [&Special; 7] = [&FULL_MOON, &METEORS, &HALLOWEEN, &CHRISTMAS, &NEW_YEAR, &VALENTINE, &POKEMON_DAY];

/// The types, in the games' order, with their usual colours.
pub const TYPES: [(&str, (u8, u8, u8)); 18] = [
    ("Normal", (168, 168, 120)),
    ("Fire", (240, 128, 48)),
    ("Water", (104, 144, 240)),
    ("Grass", (120, 200, 80)),
    ("Electric", (248, 208, 48)),
    ("Ice", (152, 216, 216)),
    ("Fighting", (192, 48, 40)),
    ("Poison", (160, 64, 160)),
    ("Ground", (224, 192, 104)),
    ("Flying", (168, 144, 240)),
    ("Psychic", (248, 88, 136)),
    ("Bug", (168, 184, 32)),
    ("Rock", (184, 160, 56)),
    ("Ghost", (112, 88, 152)),
    ("Dragon", (112, 56, 248)),
    ("Dark", (112, 88, 72)),
    ("Steel", (184, 184, 208)),
    ("Fairy", (238, 153, 172)),
];

/// A Pokédex entry's main type: an index into `TYPES`.
pub fn type_of(name: &str) -> Option<usize> {
    species(name).map(|s| s.t1 as usize)
}

/// Whether a Pokémon has this type (as either of its two).
pub fn has_type(name: &str, t: &str) -> bool {
    let Some(s) = species(name) else { return false };
    let Some(i) = TYPES.iter().position(|(n, _)| *n == t) else { return false };
    s.t1 as usize == i || s.t2 as usize == i
}

/// Today's special day or tonight's special night, if any. `hour` is the local hour (a
/// night belongs to the evening it started on).
pub fn special_for(date: crate::calendar::Date, hour: u32, c: &Conditions, unix_secs: i64) -> Option<&'static Special> {
    if let Some(s) = special_day(date) {
        return Some(s);
    }
    if !c.day {
        let night_of = if hour < 12 { date.add(-1) } else { date };
        if c.sky == Some(Kind::Clear) && meteor_night(night_of) {
            return Some(&METEORS);
        }
        if !c.wet() && full_moon(unix_secs) {
            return Some(&FULL_MOON);
        }
    }
    None
}

/// What the sky is doing, from the WMO weather code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Clear,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Downpour,
    Storm,
    Hail,
    Snow,
}

/// Hot (°C and up), cold (°C and below) and windy (km/h and up). Set for a mild coastal
/// climate like Sydney's, where it never snows and 30° days and 30 km/h winds are rare:
/// the heat, the cold and the wind should still come round every year.
pub const HOT: f32 = 28.0;
pub const COLD: f32 = 10.0;
pub const WINDY: f32 = 25.0;

pub fn sky_for(code: i64) -> Kind {
    match code {
        // A partly cloudy sky still counts as sunny.
        0..=2 => Kind::Clear,
        45 | 48 => Kind::Fog,
        51..=57 => Kind::Drizzle,
        65 | 67 | 82 => Kind::Downpour,
        61..=67 | 80 | 81 => Kind::Rain,
        71..=77 | 85 | 86 => Kind::Snow,
        96 | 99 => Kind::Hail,
        95..=99 => Kind::Storm,
        _ => Kind::Cloudy,
    }
}

/// The weather outside: the sky, daylight, and whether it's hot, cold or windy (see HOT,
/// COLD and WINDY).
#[derive(Clone, Copy, Debug)]
pub struct Conditions {
    pub sky: Option<Kind>,
    pub day: bool,
    pub hot: bool,
    pub cold: bool,
    pub windy: bool,
    /// Warm (25° and up): a warm sunny day brings Moltres and Entei.
    pub warm: bool,
    /// The season where you are (none in tests that don't care).
    pub season: Option<Season>,
}

/// The real season where you are (the other way round in the southern hemisphere).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

/// The season for a month (1–12), north or south of the equator.
pub fn season_of(month: u32, south: bool) -> Season {
    let m = if south { (month + 5) % 12 + 1 } else { month };
    match m {
        3..=5 => Season::Spring,
        6..=8 => Season::Summer,
        9..=11 => Season::Autumn,
        _ => Season::Winter,
    }
}

/// Out in autumn above all.
const AUTUMNAL: [&str; 6] = ["pumpkaboo", "gourgeist", "phantump", "trevenant", "deerling", "sawsbuck"];

/// How much the season changes a visitor's chances: spring brings grass and bug types,
/// summer fire, water and bugs, autumn ghosts and dark types (and pumpkins), winter icy types
/// (and fewer bugs and plants).
pub fn season_weight(s: Season, name: &str) -> f32 {
    let ty = |t: &str| has_type(name, t);
    match s {
        Season::Spring if ty("Grass") || ty("Bug") => 2.0,
        Season::Spring if ty("Fairy") => 1.5,
        Season::Summer if ty("Fire") || ty("Water") || ty("Bug") => 1.5,
        Season::Summer if ty("Ice") => 0.5,
        Season::Autumn if AUTUMNAL.contains(&name) => 3.0,
        Season::Autumn if ty("Ghost") || ty("Dark") => 1.5,
        Season::Winter if ty("Ice") || SNOWY.contains(&name) => 2.0,
        Season::Winter if ty("Bug") => 0.4,
        Season::Winter if ty("Grass") => 0.6,
        _ => 1.0,
    }
}

/// For the Pokédex tab.
pub fn season_line(s: Season) -> &'static str {
    match s {
        Season::Spring => "Spring: more grass, bug & fairy types",
        Season::Summer => "Summer: more fire, water & bug types",
        Season::Autumn => "Autumn: more ghost & dark types (and pumpkins)",
        Season::Winter => "Winter: more icy types, fewer bugs",
    }
}

impl Conditions {
    pub fn new(code: Option<i64>, day: bool, temp: Option<f32>, wind: Option<f32>) -> Conditions {
        Conditions {
            sky: code.map(sky_for),
            day,
            hot: temp.is_some_and(|t| t >= HOT),
            cold: temp.is_some_and(|t| t <= COLD),
            windy: wind.is_some_and(|w| w >= WINDY),
            warm: temp.is_some_and(|t| t >= 25.0),
            season: None,
        }
    }

    /// Rain of any kind (fire types stay in).
    pub fn wet(&self) -> bool {
        matches!(self.sky, Some(Kind::Drizzle | Kind::Rain | Kind::Downpour | Kind::Storm | Kind::Hail))
    }
}

/// Out in the snow as well as the icy types.
const SNOWY: [&str; 4] = ["piplup", "swablu", "snorlax", "delibird"];

/// How much more (or less) likely a visitor is in this weather: 1 is normal, 0 never.
pub fn spawn_weight(c: &Conditions, name: &str) -> f32 {
    let ty = |t: &str| has_type(name, t);
    let (water, fire, grass, electric) = (ty("Water"), ty("Fire"), ty("Grass"), ty("Electric"));
    let icy = ty("Ice") || SNOWY.contains(&name);
    // Ghost, dark and psychic types are out at night; ghosts and Ditto drift in the fog.
    let night = ty("Ghost") || ty("Dark") || ty("Psychic");
    let foggy = ty("Ghost") || name == "ditto";
    let flying = ty("Flying");
    let mut w = 1.0;
    match c.sky {
        Some(Kind::Storm) => {
            if electric {
                w *= 5.0;
            }
            if water || name == "kyogre" {
                w *= 3.0;
            }
        }
        Some(Kind::Hail) => {
            if electric || icy {
                w *= 4.0;
            }
            if water {
                w *= 2.0;
            }
        }
        Some(Kind::Downpour) if water || name == "kyogre" => w *= 6.0,
        Some(Kind::Rain) if water || name == "kyogre" => w *= 4.0,
        Some(Kind::Drizzle) if water => w *= 2.5,
        Some(Kind::Snow) if icy => w *= 5.0,
        Some(Kind::Fog) if foggy || name == "mewtwo" => w *= 4.0,
        Some(Kind::Cloudy) if ty("Fairy") || ty("Fighting") || ty("Poison") => w *= 3.0,
        Some(Kind::Clear) if c.day => {
            if fire || grass {
                w *= 4.0;
            }
            if water && name != "magikarp" {
                w *= 0.5;
            }
        }
        _ => {}
    }
    if fire {
        if c.wet() {
            // Fire types stay in out of the rain.
            return 0.0;
        }
        if c.sky == Some(Kind::Snow) || c.cold {
            w *= 0.3;
        }
        if c.hot {
            w *= 3.0;
        }
    }
    if c.hot && icy {
        w *= 0.3;
    }
    if c.cold && icy {
        w *= 3.0;
    }
    if c.windy && flying {
        w *= 4.0;
    }
    if !c.day {
        if night || name == "darkrai" {
            w *= 4.0;
        }
        if grass {
            w *= 0.5;
        }
    }
    if let Some(s) = c.season {
        w *= season_weight(s, name);
    }
    w
}

/// The weather suits it (at least twice as likely as usual): a weather-boosted visitor gives
/// more XP and is easier to catch.
pub fn weather_boosted(c: &Conditions, name: &str) -> bool {
    // The weather only: the season doesn't boost.
    let c = Conditions { season: None, ..*c };
    c.sky.is_some() && spawn_weight(&c, name) >= 2.0
}

/// A short line for the Pokédex tab about who the weather brings out ("" when nothing does).
pub fn spawn_line(c: &Conditions) -> String {
    let head = match c.sky {
        Some(Kind::Storm) => "Thunderstorm: electric & water types, no fire types",
        Some(Kind::Hail) => "Hail: electric, icy & water types, no fire types",
        Some(Kind::Downpour) => "Heavy rain: water types everywhere, no fire types",
        Some(Kind::Rain) => "Rain: water types out, no fire types",
        Some(Kind::Drizzle) => "Drizzle: a few water types, no fire types",
        Some(Kind::Snow) => "Snow: snowy-day Pokémon out",
        Some(Kind::Fog) => "Fog: ghosts & drifters out",
        Some(Kind::Cloudy) => "Cloudy: fairy, fighting & poison types out",
        Some(Kind::Clear) if c.day => "Sunny: fire & grass types out",
        _ => "",
    };
    let mut parts = vec![head];
    if c.hot {
        parts.push("hot: more fire types");
    }
    if c.cold {
        parts.push("cold: icy types");
    }
    if c.windy {
        parts.push("windy: flying types");
    }
    if !c.day {
        parts.push("night: ghost & dark types");
    }
    let line = parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(", ");
    let mut ch = line.chars();
    match ch.next() {
        Some(f) => f.to_uppercase().chain(ch).collect(),
        None => line,
    }
}

/// Who looks after the island in this weather (in Rayquaza's place): (name of the weather,
/// how the card puts it, fully evolved and legendary Pokémon that suit it). Only the ones
/// whose sprites are downloaded get picked.
pub fn keeper_pool(c: &Conditions) -> (&'static str, &'static str, &'static [&'static str]) {
    match c.sky {
        Some(Kind::Storm) => ("storm", "with the thunderstorm", &["zapdos", "raikou", "zekrom", "thundurus", "ampharos", "luxray", "electivire", "raichu", "jolteon", "pikachu"]),
        Some(Kind::Hail) => ("hail", "in the hail", &["articuno", "kyurem", "regice", "mamoswine", "glaceon", "walrein", "lapras"]),
        Some(Kind::Snow) => ("snow", "in the snow", &["articuno", "glaceon", "regice", "abomasnow", "froslass", "weavile", "lapras"]),
        Some(Kind::Downpour) => ("downpour", "in the downpour", &["kyogre", "palkia", "gyarados", "blastoise", "kingdra", "feraligatr"]),
        Some(Kind::Rain) => ("rain", "in the rain", &["kyogre", "blastoise", "gyarados", "feraligatr", "empoleon", "samurott", "swampert", "suicune"]),
        Some(Kind::Drizzle) => ("drizzle", "in the drizzle", &["suicune", "vaporeon", "milotic", "lapras", "azumarill", "ludicolo", "swampert"]),
        Some(Kind::Fog) => ("fog", "out of the fog", &["mewtwo", "giratina", "gengar", "gardevoir", "mismagius", "darkrai"]),
        _ if c.windy => ("wind", "on the wind", &["rayquaza", "lugia", "tornadus", "pidgeot", "staraptor", "salamence", "dragonite", "braviary"]),
        _ if c.hot && c.day => ("hot", "in the heat", &["groudon", "moltres", "entei", "heatran", "reshiram", "charizard", "arcanine", "magmortar", "typhlosion"]),
        _ if c.cold => ("cold", "in the cold", &["articuno", "glaceon", "lapras", "walrein", "abomasnow", "weavile"]),
        _ if !c.day => ("night", "for the night", &["darkrai", "umbreon", "cresselia", "gengar", "absol", "houndoom", "mismagius"]),
        Some(Kind::Clear) => ("sunny", "in the sunshine", &["groudon", "charizard", "victini", "reshiram", "entei", "arcanine", "ninetales", "typhlosion", "infernape"]),
        _ => ("cloudy", "under the clouds", &["rayquaza", "lugia", "latias", "latios", "altaria", "dragonite", "togekiss"]),
    }
}

/// Whether a sprite has been downloaded (a quick look for its files).
pub fn has_sprite(name: &str) -> bool {
    let dir = crate::util::sprite_dir();
    dir.join(format!("{name}.json")).exists() && dir.join(format!("{name}.png")).exists()
}

/// The Pokémon for a colour (r, g, b in 0..1).
///
/// white / silver → Arceus, black / dark grey → Umbreon, brown → Eevee, red → Infernape,
/// orange → Charizard, yellow → Pikachu, green → Sceptile, cyan → Lapras, blue → Swampert,
/// purple → Garchomp, pink → Mew.
pub fn for_colour(rgb: [f32; 3]) -> &'static Mon {
    let (h, s, l) = crate::imaging::rgb_to_hsl(rgb);
    let deg = h * 360.0;
    if s < 0.15 {
        return if l >= 0.45 { &ARCEUS } else { &UMBREON };
    }
    if l < 0.18 {
        return &UMBREON;
    }
    if l > 0.9 {
        return &ARCEUS;
    }
    match deg {
        d if (15.0..45.0).contains(&d) && l < 0.42 => &EEVEE,
        d if !(15.0..345.0).contains(&d) => &INFERNAPE,
        d if d < 45.0 => &CHARIZARD,
        d if d < 70.0 => &PIKACHU,
        d if d < 165.0 => &SCEPTILE,
        d if d < 200.0 => &LAPRAS,
        d if d < 255.0 => &SWAMPERT,
        d if d < 290.0 => &GARCHOMP,
        _ => &MEW,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_fit_the_level() {
        assert_eq!(stage_at("charizard", 20), "charmeleon");
        assert_eq!(stage_at("charizard", 10), "charmander");
        assert_eq!(stage_at("charizard", 40), "charizard");
        assert_eq!(stage_at("charmander", 40), "charizard");
        assert_eq!(stage_at("charmander-shiny", 20), "charmeleon-shiny");
        assert_eq!(stage_at("tyranitar", 14), "larvitar");
        assert_eq!(stage_at("eevee", 57), "eevee");
        assert_eq!(stage_at("vaporeon", 10), "eevee");
        assert_eq!(stage_at("ninetales-alola", 10), "vulpix-alola");
        assert_eq!(stage_at("pikachu", 5), "pikachu");
        assert_eq!(stage_at("charizard-mega-x", 50), "charizard-mega-x");
        assert_eq!(stage_at("charizard-mega-x", 20), "charmeleon");
        assert_eq!(stage_at("lapras", 5), "lapras");
        assert_eq!(family("charizard-mega-x"), "charmander");
        assert_eq!(family("raichu"), "pichu");
    }

    #[test]
    fn friendship() {
        for (a, b, _) in FRIEND_EVOS {
            assert!(species(a).is_some() && species(b).is_some(), "{a} → {b}");
            // They really are evolutions, so the level rule has something to skip.
            assert!(evolutions_of(a).iter().any(|(t, _)| *t == b), "{a} → {b}");
        }
        assert_eq!(friend_evolution("eevee", true), Some("espeon"));
        assert_eq!(friend_evolution("eevee", false), Some("umbreon"));
        assert_eq!(friend_evolution("eevee-shiny", false), Some("umbreon"));
        assert_eq!(friend_evolution("riolu", false), None);
        assert_eq!(friend_evolution("golbat", false), Some("crobat"));
        assert_eq!(friend_evolution("bulbasaur", true), None);
        assert!(is_friend_evo("eevee", "umbreon") && !is_friend_evo("eevee", "vaporeon"));
        assert_eq!(friend_evo_text("eevee"), "Espeon by day, Umbreon at night");
        assert_eq!(hearts(FRIEND_START), "♥♡♡♡♡");
        assert_eq!(hearts(FRIEND_EVOLVE), "\u{2665}\u{2665}\u{2665}\u{2661}\u{2661}");
        assert_eq!(hearts(FRIEND_MAX), "♥♥♥♥♥");
    }

    #[test]
    fn seasons() {
        assert_eq!(season_of(11, false), Season::Autumn);
        assert_eq!(season_of(11, true), Season::Spring);
        assert_eq!(season_of(1, false), Season::Winter);
        assert_eq!(season_of(1, true), Season::Summer);
        assert_eq!(season_of(7, true), Season::Winter);
        assert_eq!(season_of(4, false), Season::Spring);
        assert_eq!(season_weight(Season::Spring, "bulbasaur"), 2.0);
        assert_eq!(season_weight(Season::Winter, "caterpie"), 0.4);
        assert_eq!(season_weight(Season::Autumn, "pumpkaboo"), 3.0);
        let mut c = Conditions::new(Some(0), true, Some(20.0), Some(5.0));
        let base = spawn_weight(&c, "bulbasaur");
        c.season = Some(Season::Spring);
        assert_eq!(spawn_weight(&c, "bulbasaur"), base * 2.0);
        // The season never makes a visitor weather-boosted.
        let calm = Conditions { season: Some(Season::Spring), ..Conditions::new(Some(3), true, Some(20.0), Some(5.0)) };
        assert!(!weather_boosted(&calm, "caterpie"));
    }

    #[test]
    fn fishing_list_is_real() {
        assert_eq!(fish_pool().len(), FISH.len());
        assert!(FISH_ONLY.iter().all(|n| FISH.contains(n)));
        assert!(!FISH.iter().any(|n| is_legend(n)));
    }

    #[test]
    fn weather_brings_out_real_visitors() {
        let rain = Conditions::new(Some(61), true, Some(18.0), Some(10.0));
        assert_eq!(spawn_weight(&rain, "growlithe"), 0.0);
        assert_eq!(spawn_weight(&Conditions::new(Some(95), false, None, None), "tepig"), 0.0);
        assert!(spawn_weight(&rain, "psyduck") > 1.0);
        let pour = Conditions::new(Some(65), true, None, None);
        let drizzle = Conditions::new(Some(53), true, None, None);
        assert!(spawn_weight(&pour, "psyduck") > spawn_weight(&drizzle, "psyduck"));
        let hot = Conditions::new(Some(0), true, Some(36.0), None);
        let mild = Conditions::new(Some(0), true, Some(22.0), None);
        assert!(spawn_weight(&hot, "tepig") > spawn_weight(&mild, "tepig"));
        assert!(spawn_weight(&Conditions::new(Some(3), true, None, Some(45.0)), "pidgey") > 1.0);
        assert_eq!(sky_for(96), Kind::Hail);
        assert_eq!(sky_for(82), Kind::Downpour);
        assert!(spawn_line(&rain).starts_with("Rain"));
        assert_eq!(spawn_line(&Conditions::new(Some(3), false, None, None)), "Cloudy: fairy, fighting & poison types out, night: ghost & dark types");
        assert_eq!(spawn_line(&Conditions::new(Some(45), false, None, None)), "Fog: ghosts & drifters out, night: ghost & dark types");
        // Partly cloudy counts as sunny.
        assert!(spawn_line(&Conditions::new(Some(2), true, Some(20.0), Some(5.0))).starts_with("Sunny"));
        // Sydney weather: 28° is hot, a 10° night is cold, a 25 km/h wind is windy.
        let sydney = |t: f32, w: f32| Conditions::new(Some(1), true, Some(t), Some(w));
        assert!(sydney(28.0, 5.0).hot && !sydney(27.0, 5.0).hot);
        assert!(sydney(10.0, 5.0).cold && !sydney(11.0, 5.0).cold);
        assert!(sydney(20.0, 25.0).windy && !sydney(20.0, 24.0).windy);
        assert!(spawn_line(&Conditions::new(None, true, Some(20.0), Some(5.0))).is_empty());
        let cloudy = Conditions::new(Some(3), true, Some(20.0), Some(5.0));
        assert!(spawn_weight(&cloudy, "machop") > 2.0);
        assert!(weather_boosted(&cloudy, "clefairy"));
        assert!(!weather_boosted(&cloudy, "pidgey"));
        assert!(weather_boosted(&Conditions::new(Some(61), true, None, None), "squirtle"));
        assert!(!weather_boosted(&Conditions::new(None, false, None, None), "gastly"));
        for e in eggs() {
            assert!(!wild().contains(e) && !legends().contains(e), "{e}");
        }
    }

    #[test]
    fn caught_pokemon_evolve() {
        assert_eq!(evolutions_of("marill"), vec![("azumarill", 18)]);
        assert_eq!(evolutions_of("pidgeotto"), vec![("pidgeot", 36)]);
        assert!(evolutions_of("eevee").len() >= 8);
        assert!(evolutions_of("mewtwo").is_empty());
        // Each evolution comes after the one before it.
        for (from, to, at) in EVOLUTIONS.iter() {
            let prev = EVOLUTIONS.iter().find(|e| e.1 == *from).map(|e| e.2).unwrap_or(0);
            let baby = NATIONAL[*from as usize - 1].baby || EVOLUTIONS.iter().any(|e| e.1 == *from && NATIONAL[e.0 as usize - 1].baby);
            assert!(*at > prev || baby, "{from} -> {to}");
        }
    }

    #[test]
    fn forms() {
        assert!(FORMS_SIZE >= 180, "{FORMS_SIZE}");
        assert!(form_list().all(|n| type_of(n).is_some() && species(n).is_some_and(|s| s.form > 0)));
        assert_eq!(pretty("vulpix-alola-shiny"), "Shiny Alolan Vulpix");
        assert_eq!(region(species("charizard-mega-x").unwrap()), "Mega");
        assert_eq!(evolutions_of("vulpix-alola"), vec![("ninetales-alola", 30)]);
        assert_eq!(evolutions_of("meowth-galar"), vec![("perrserker", 28)]);
        assert!(!evolutions_of("meowth").iter().any(|(n, _)| *n == "perrserker"));
        assert!(evolutions_of("charizard-mega-x").is_empty());
        assert_eq!(megas_of("charizard"), vec!["charizard-mega-x", "charizard-mega-y"]);
        assert_eq!(megas_of("kyogre"), vec!["kyogre-primal"]);
        let g = grows_into("charmeleon", 1);
        assert_eq!(g, vec![("charizard", 36, true)]);
        assert!(grows_into("charizard", 1).iter().all(|(_, at, _)| *at == MEGA_LEVEL));
        assert!(wild().contains(&"growlithe-hisui") && !wild().contains(&"charizard-gmax"));
        assert!(legends().contains(&"articuno-galar"));
        assert!(gigantamax().contains(&"pikachu-gmax"));
    }

    #[test]
    fn national_pokedex() {
        assert_eq!(DEX_SIZE, 1025);
        assert_eq!(species("pecharunt").map(|s| s.id), Some(1025));
        assert_eq!(pretty("mr-mime-shiny"), "Shiny Mr. Mime");
        assert_eq!(pretty("farfetchd"), "Farfetch’d");
        assert_eq!(species("sprigatito").map(region), Some("Paldea"));
        assert!(wild().len() > 800 && wild().contains(&"pikachu") && !wild().contains(&"mewtwo"));
        assert!(legends().contains(&"mewtwo") && legends().contains(&"nihilego") && !legends().contains(&"zapdos"));
        assert!(eggs().contains(&"pichu") && eggs().contains(&"riolu") && !wild().contains(&"togepi"));
        assert!(rarity("charizard") < rarity("charmeleon") && rarity("charmeleon") < rarity("charmander"));
        // Types: a water type in the rain, a flying one on the wind, any fire type stays in.
        let rain = Conditions::new(Some(61), true, Some(18.0), Some(10.0));
        assert!(spawn_weight(&rain, "froakie") > 1.0);
        assert_eq!(spawn_weight(&rain, "fuecoco"), 0.0);
        assert!(has_type("pidgey", "Flying") && type_of("gyarados") == Some(2));
    }

    #[test]
    fn weather_legends_and_evolutions() {
        let storm = Conditions::new(Some(95), true, Some(22.0), Some(10.0));
        let snow = Conditions::new(Some(73), true, Some(-1.0), Some(5.0));
        let heat = Conditions::new(Some(0), true, Some(34.0), Some(5.0));
        let rain = Conditions::new(Some(61), true, Some(31.0), Some(5.0));
        let names = |c: &Conditions| weather_legends(c).into_iter().map(|w| w.0).collect::<Vec<_>>();
        assert_eq!(names(&storm), ["zapdos", "raikou"]);
        assert_eq!(names(&snow), ["articuno"]);
        assert_eq!(names(&heat), ["moltres", "entei"]);
        assert_eq!(names(&rain), ["suicune"]);
        // Everyday weather brings them too, where it never snows or hits 30°: a cool clear
        // night, a warm sunny day, a windy day and a downpour.
        assert_eq!(names(&Conditions::new(Some(0), false, Some(9.0), Some(5.0))), ["articuno"]);
        assert_eq!(names(&Conditions::new(Some(0), true, Some(26.0), Some(5.0))), ["moltres", "entei"]);
        assert_eq!(names(&Conditions::new(Some(0), false, Some(26.0), Some(5.0))), Vec::<&str>::new());
        assert_eq!(names(&Conditions::new(Some(3), true, Some(18.0), Some(35.0))), ["zapdos"]);
        assert_eq!(names(&Conditions::new(Some(65), true, Some(18.0), Some(5.0))), ["raikou", "suicune"]);
        assert!(names(&Conditions::new(Some(3), true, Some(20.0), Some(5.0))).is_empty());
        assert_eq!(time_legend(6, true).map(|w| w.0), Some("uxie"));
        assert_eq!(time_legend(18, true).map(|w| w.0), Some("mesprit"));
        assert_eq!(time_legend(23, false).map(|w| w.0), Some("azelf"));
        assert_eq!(time_legend(13, true), None);
        // The roamers only come their own way, not from the everyday legendary pool.
        assert!(ROAMERS.iter().all(|r| !legends().contains(&r.0)));
        // Only the team members you've met are announced.
        assert!(changes_of(13, 14, |n| n == "Swampert").is_empty());
        assert_eq!(changes_of(13, 14, |n| n == "Infernape"), vec![("chimchar".to_string(), "monferno".to_string())]);
        assert_eq!(next_change_of(13, |n| n == "Swampert").map(|c| c.0), Some(16));
        assert!(next_change_of(1, |_| false).is_none());
    }

    #[test]
    fn every_dex_entry_has_a_type() {
        for n in dex_list() {
            assert!(type_of(n).is_some(), "{n}");
        }
        let missing = special_missing(|n| n == "clefairy");
        assert!(missing.iter().any(|(n, when)| *n == "deoxys" && *when == "meteor shower"));
        assert!(!missing.iter().any(|(n, _)| *n == "clefairy" || *n == "gastly"));
    }

    #[test]
    fn special_days_and_nights() {
        use crate::calendar::Date;
        let night = Conditions::new(Some(0), false, Some(15.0), Some(5.0));
        let day = Conditions::new(Some(0), true, Some(20.0), Some(5.0));
        let rainy_night = Conditions::new(Some(61), false, None, None);
        let t = 1_794_700_000; // mid-Nov 2026
        assert_eq!(special_for(Date { y: 2026, m: 10, d: 31 }, 15, &day, t).map(|s| s.key), Some("halloween"));
        assert_eq!(special_for(Date { y: 2026, m: 12, d: 25 }, 9, &day, t).map(|s| s.key), Some("christmas"));
        assert_eq!(special_for(Date { y: 2027, m: 2, d: 14 }, 9, &day, t).map(|s| s.key), Some("valentine"));
        // The Leonids (17 Nov): a clear night is a meteor shower, a rainy one isn't.
        assert_eq!(special_for(Date { y: 2026, m: 11, d: 17 }, 22, &night, t).map(|s| s.key), Some("meteors"));
        assert_ne!(special_for(Date { y: 2026, m: 11, d: 17 }, 22, &rainy_night, t).map(|s| s.key), Some("meteors"));
        assert!(special_for(Date { y: 2026, m: 11, d: 17 }, 14, &day, t).is_none());
        // Its shape: nothing lit at new moon, all at full, the right half at first quarter
        // (the left half from the south), the left half at last quarter.
        let p = 29.530588853;
        assert!(!moon_lit(0.0, 0.5, 0.0, false) && !moon_lit(0.0, -0.5, 0.0, false));
        assert!(moon_lit(p / 2.0, 0.9, 0.0, false) && moon_lit(p / 2.0, -0.9, 0.0, false) && moon_lit(p / 2.0, 0.0, 0.9, false));
        assert!(moon_lit(p / 4.0, 0.5, 0.0, false) && !moon_lit(p / 4.0, -0.5, 0.0, false));
        assert!(!moon_lit(p / 4.0, 0.5, 0.0, true) && moon_lit(p / 4.0, -0.5, 0.0, true));
        assert!(moon_lit(p * 0.75, -0.5, 0.0, false) && !moon_lit(p * 0.75, 0.5, 0.0, false));
        // A thin waxing crescent: only the right edge.
        assert!(moon_lit(2.5, 0.95, 0.0, false) && !moon_lit(2.5, 0.3, 0.0, false));
        assert!(moon_lit_share(0.0) < 0.01 && moon_lit_share(p / 2.0) > 0.99);
        assert!((moon_lit_share(p / 4.0) - 0.5).abs() < 0.01);
        assert_eq!(moon_phase_name(0.2), "New moon");
        assert_eq!(moon_phase_name(p / 4.0), "First quarter");
        assert_eq!(moon_phase_name(p / 2.0), "Full moon");
        assert_eq!(moon_phase_name(p * 0.6), "Waning gibbous");
        assert_eq!(moon_phase_name(p - 0.2), "New moon");
        // A known full moon (21 Jan 2000, 04:40 UTC) and new moon (6 Jan 2000, 18:14 UTC).
        assert!(full_moon(948_429_600));
        assert!(moon_age(947_182_440) < 0.1 || moon_age(947_182_440) > 29.4);
        for s in [&HALLOWEEN, &CHRISTMAS, &NEW_YEAR, &VALENTINE, &POKEMON_DAY, &METEORS, &FULL_MOON] {
            assert!(!s.keepers.is_empty(), "{}", s.key);
            for v in s.visitors {
                assert!(species(v).is_some(), "{v}");
            }
        }
    }

    #[test]
    fn every_weather_has_keepers() {
        for code in [None, Some(0), Some(3), Some(45), Some(53), Some(61), Some(65), Some(71), Some(95), Some(96)] {
            for (day, temp, wind) in [(true, 20.0, 5.0), (false, 20.0, 5.0), (true, 35.0, 5.0), (true, 4.0, 5.0), (true, 20.0, 50.0)] {
                let (key, phrase, pool) = keeper_pool(&Conditions::new(code, day, Some(temp), Some(wind)));
                assert!(!key.is_empty() && !phrase.is_empty() && pool.len() >= 5, "{key}");
                for (i, n) in pool.iter().enumerate() {
                    assert!(n.chars().all(|c| c.is_ascii_lowercase()), "{n}");
                    assert!(!pool[..i].contains(n), "{key}: {n} twice");
                }
            }
        }
    }

    #[test]
    fn sprites_are_well_formed() {
        for m in ALL {
            assert!(m.rows.len() <= 12, "{} too tall", m.name);
            for r in m.rows {
                assert_eq!(r.len(), 16, "{}: row {r:?}", m.name);
                for c in r.chars().filter(|c| *c != '.') {
                    assert!(m.colour(c).is_some(), "{}: no colour for {c:?}", m.name);
                }
            }
        }
    }

    #[test]
    fn colours_pick_the_right_pokemon() {
        let pick = |r: f32, g: f32, b: f32| for_colour([r, g, b]).name;
        assert_eq!(pick(0.95, 0.85, 0.2), "Pikachu");
        assert_eq!(pick(0.3, 0.75, 0.35), "Sceptile");
        assert_eq!(pick(0.25, 0.45, 0.9), "Swampert");
        assert_eq!(pick(0.9, 0.2, 0.2), "Infernape");
        assert_eq!(pick(0.6, 0.3, 0.85), "Garchomp");
        assert_eq!(pick(0.95, 0.95, 0.95), "Arceus");
        assert_eq!(pick(0.95, 0.55, 0.2), "Charizard");
        assert_eq!(pick(0.3, 0.85, 0.9), "Lapras");
        assert_eq!(pick(0.95, 0.45, 0.75), "Mew");
        assert_eq!(pick(0.12, 0.12, 0.14), "Umbreon");
        assert_eq!(pick(0.5, 0.3, 0.15), "Eevee");
    }

    #[test]
    fn legends_by_region_cover_every_legend() {
        let all = legends_by_region();
        assert_eq!(all.len(), REGIONS.len());
        // One page each: the Legends view fits 32.
        assert!(all.iter().all(|l| !l.is_empty() && l.len() <= 32));
        assert_eq!(&all[0][..], ["articuno", "zapdos", "moltres", "mewtwo", "mew"]);
        assert!(all[7].contains(&"zapdos-galar") && all[2].contains(&"rayquaza") && all[8].contains(&"pecharunt"));
        let n = NATIONAL.iter().filter(|s| s.legend).count() + 3;
        assert_eq!(all.iter().map(|l| l.len()).sum::<usize>(), n);
    }
}
