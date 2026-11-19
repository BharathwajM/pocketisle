//! The Pokémon League of every region, with trainers of the island's own: eight gyms, each
//! with a type and a badge, then the Elite Four and the champion, back to back. Teams come
//! from the region's own Pokémon (fully evolved once the levels are high enough). Plus three
//! travelling trainers a day.

use crate::battle::{base_total, Mon};
use crate::national::{NATIONAL, REGIONS};
use crate::pokemon;

pub struct Region {
    /// Gym leaders: (name, type).
    pub gyms: [(&'static str, &'static str); 8],
    pub elite: [(&'static str, &'static str); 4],
    pub champion: &'static str,
}

pub static LEAGUE: [Region; 9] = [
    Region {
        gyms: [("Oren", "Normal"), ("Tamsin", "Water"), ("Rafe", "Electric"), ("Junie", "Bug"), ("Corin", "Poison"), ("Hollis", "Psychic"), ("Mara", "Fire"), ("Dax", "Ground")],
        elite: [("Selwyn", "Ice"), ("Briar", "Fighting"), ("Kestrel", "Ghost"), ("Ondine", "Dragon")],
        champion: "Arlo",
    },
    Region {
        gyms: [("Ren", "Flying"), ("Sachi", "Bug"), ("Toma", "Normal"), ("Aiko", "Ghost"), ("Haru", "Fighting"), ("Yuna", "Steel"), ("Kenji", "Ice"), ("Mio", "Dragon")],
        elite: [("Daisuke", "Psychic"), ("Emi", "Poison"), ("Rokuro", "Dark"), ("Saya", "Fire")],
        champion: "Asahi",
    },
    Region {
        gyms: [("Marisol", "Rock"), ("Teo", "Fighting"), ("Coralie", "Electric"), ("Ibis", "Fire"), ("Nalu", "Normal"), ("Makoa", "Flying"), ("Lani", "Psychic"), ("Reva", "Water")],
        elite: [("Delphine", "Dark"), ("Sorrel", "Ghost"), ("Talia", "Ice"), ("Rocco", "Dragon")],
        champion: "Pelagia",
    },
    Region {
        gyms: [("Sven", "Rock"), ("Liisa", "Grass"), ("Bjorn", "Fighting"), ("Elin", "Water"), ("Tove", "Ghost"), ("Halvar", "Steel"), ("Signe", "Ice"), ("Aurel", "Electric")],
        elite: [("Runa", "Bug"), ("Eskil", "Ground"), ("Ylva", "Fire"), ("Torvald", "Psychic")],
        champion: "Astrid",
    },
    Region {
        gyms: [("Jada", "Grass"), ("Marcus", "Normal"), ("Quinn", "Bug"), ("Rosalind", "Electric"), ("Theo", "Ground"), ("Vera", "Flying"), ("Dominic", "Ice"), ("Nadia", "Dragon")],
        elite: [("Sterling", "Ghost"), ("Ivy", "Dark"), ("Rhett", "Psychic"), ("Celeste", "Fighting")],
        champion: "Augustine",
    },
    Region {
        gyms: [("Amélie", "Bug"), ("Bastien", "Rock"), ("Céline", "Fighting"), ("Margaux", "Grass"), ("Remy", "Electric"), ("Solène", "Fairy"), ("Thibault", "Psychic"), ("Odette", "Ice")],
        elite: [("Gaspard", "Fire"), ("Isaure", "Water"), ("Florian", "Steel"), ("Élodie", "Dragon")],
        champion: "Séraphine",
    },
    Region {
        gyms: [("Keoni", "Normal"), ("Malia", "Water"), ("Ikaika", "Fire"), ("Noelani", "Grass"), ("Kekoa", "Electric"), ("Leilani", "Ghost"), ("Hoku", "Fairy"), ("Kawika", "Ground")],
        elite: [("Kalani", "Rock"), ("Pono", "Steel"), ("Iolana", "Dark"), ("Makana", "Flying")],
        champion: "Kainoa",
    },
    Region {
        gyms: [("Rupert", "Grass"), ("Philippa", "Water"), ("Callum", "Fire"), ("Imogen", "Fighting"), ("Alistair", "Ghost"), ("Bryony", "Fairy"), ("Fergus", "Rock"), ("Elspeth", "Dragon")],
        elite: [("Barnaby", "Ice"), ("Clemency", "Dark"), ("Hamish", "Steel"), ("Ottilie", "Psychic")],
        champion: "Percival",
    },
    Region {
        gyms: [("Inés", "Bug"), ("Mateo", "Grass"), ("Lucía", "Electric"), ("Rafael", "Water"), ("Pilar", "Normal"), ("Tomás", "Ghost"), ("Alba", "Psychic"), ("Joaquín", "Ice")],
        elite: [("Leocadia", "Ground"), ("Ramiro", "Steel"), ("Consuelo", "Flying"), ("Ignacio", "Dragon")],
        champion: "Valentina",
    },
];

/// Each gym's badge, by type.
pub fn badge_name(t: &str) -> String {
    let w = match t {
        "Normal" => "Plain",
        "Fire" => "Cinder",
        "Water" => "Tide",
        "Grass" => "Leaf",
        "Electric" => "Spark",
        "Ice" => "Frost",
        "Fighting" => "Fist",
        "Poison" => "Venom",
        "Ground" => "Dune",
        "Flying" => "Gale",
        "Psychic" => "Mind",
        "Bug" => "Hive",
        "Rock" => "Crag",
        "Ghost" => "Shade",
        "Dragon" => "Wyrm",
        "Dark" => "Dusk",
        "Steel" => "Anvil",
        _ => "Charm",
    };
    format!("{w} Badge")
}

/// What a trainer says when the battle starts and when they lose, by their type (the
/// champion has lines of their own).
pub fn lines(region: usize, stage: usize) -> (&'static str, &'static str) {
    if stage >= CHAMPION {
        return ("So you made it this far. Show me everything!", "You're the champion now. I'm proud of you.");
    }
    match stage_type(region, stage) {
        "Normal" => ("Nothing fancy here. Just pure strength!", "Plain and simple, and still beaten. Well done!"),
        "Fire" => ("I hope you can stand the heat!", "Burned out… You earned this one."),
        "Water" => ("Try to keep your head above water!", "Washed away! You've got real flow."),
        "Grass" => ("My team has deep roots. Try to pull them up!", "You weeded us out! Well grown."),
        "Electric" => ("Ready for a shock? We're fully charged!", "Short-circuited! What a jolt."),
        "Ice" => ("Chill out. This will be over quickly.", "My ice has melted… impressive."),
        "Fighting" => ("Fists up! Let's see your fighting spirit!", "A knockout! You fought with heart."),
        "Poison" => ("One sting is all it takes…", "You shrugged off my venom? Remarkable."),
        "Ground" => ("Feel the earth move under your feet!", "Buried! You really stood your ground."),
        "Flying" => ("Up here, the wind is on our side!", "Grounded! You flew higher than us."),
        "Psychic" => ("I already know how this ends.", "I didn't see that coming… truly."),
        "Bug" => ("We're small, but there are lots of us!", "Squashed! You've got a sharp eye."),
        "Rock" => ("You'll need more than that to crack us.", "Crumbled to gravel! Rock-solid work."),
        "Ghost" => ("Do you feel a chill behind you…?", "You weren't scared at all? Spooky."),
        "Dragon" => ("Dragons bow to no one. Prove yourself!", "You tamed my dragons. You're the real deal."),
        "Dark" => ("Things look different in the dark.", "Out of the shadows… and you shine."),
        "Steel" => ("Built tough. Forged to win.", "Dented! My armour wasn't enough."),
        _ => ("Don't let the sparkle fool you!", "What a magical battle! Take this with pride."),
    }
}

/// Which challenge: gyms 0–7, the Elite Four 8–11, the champion 12.
pub const ELITE: usize = 8;
pub const CHAMPION: usize = 12;
#[cfg(test)]
pub const STAGES: usize = 13;

/// The trainer's title and name ("Gym Leader Oren").
pub fn trainer_name(region: usize, stage: usize) -> String {
    let r = &LEAGUE[region % 9];
    match stage {
        0..=7 => format!("Gym Leader {}", r.gyms[stage].0),
        8..=11 => format!("Elite Four {}", r.elite[stage - 8].0),
        _ => format!("Champion {}", r.champion),
    }
}

/// Its type ("" for the champion, who uses several).
pub fn stage_type(region: usize, stage: usize) -> &'static str {
    let r = &LEAGUE[region % 9];
    match stage {
        0..=7 => r.gyms[stage].1,
        8..=11 => r.elite[stage - 8].1,
        _ => "",
    }
}

/// The level of its strongest Pokémon: later gyms and later regions are tougher
/// (Kanto's first gym Lv 12, its champion Lv 68; Paldea's champion Lv 100).
pub fn stage_level(region: usize, stage: usize) -> u32 {
    let r = region as u32 * 4;
    let lv = match stage {
        0..=7 => 12 + 6 * stage as u32 + r,
        8..=11 => 58 + 2 * (stage as u32 - 8) + r,
        _ => 68 + r,
    };
    lv.min(100)
}

/// The level your League party battles at: a little above each gym leader's strongest,
/// and for the Elite Four and champion (back to back) just under the champion's.
pub fn party_level(region: usize, stage: usize) -> u32 {
    if stage < ELITE {
        (stage_level(region, stage) + 2).min(100)
    } else {
        stage_level(region, CHAMPION).saturating_sub(2).min(100)
    }
}

/// A gym's rematch level once you're the region's champion: its leader brings out their
/// full team of six, not far below the champion's own level.
pub fn rematch_level(region: usize, stage: usize) -> u32 {
    (stage_level(region, CHAMPION) - 8 + stage.min(7) as u32).min(100)
}

/// The level your League party battles a gym's rematch at.
pub fn rematch_party_level(region: usize, stage: usize) -> u32 {
    (rematch_level(region, stage) + 2).min(100)
}

/// The difficulty settings, by number.
pub const DIFFICULTY: [&str; 3] = ["Normal", "Hard", "Max"];

/// How many levels above the usual the trainers are on a difficulty.
pub fn bonus(diff: u8) -> u32 {
    [0, 5, 10][diff.min(2) as usize]
}

/// A win's XP on a difficulty: ×1.5 on Hard, ×2 on Max.
pub fn scaled_xp(xp: u32, diff: u8) -> u32 {
    xp * [2, 3, 4][diff.min(2) as usize] / 2
}

/// What kind of win it was, for its prize.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Win {
    /// A gym's first win, earning its badge (`late`: gyms 5–8).
    Badge { late: bool },
    /// Beating a gym again before you're champion.
    Again,
    /// A gym leader's full rematch team.
    FullTeam,
    Elite,
    Champion,
    Traveller,
}

/// The balls a win gives, as (ball kind, count): more on Hard, the best on Max.
pub fn prize(win: Win, diff: u8) -> Vec<(u8, u32)> {
    let d = diff.min(2) as usize;
    let three = |a: Vec<(u8, u32)>, b: Vec<(u8, u32)>, c: Vec<(u8, u32)>| [a, b, c].into_iter().nth(d).unwrap_or_default();
    match win {
        Win::Badge { late: false } => three(vec![(1, 3)], vec![(1, 5)], vec![(2, 3)]),
        Win::Badge { late: true } => three(vec![(2, 1)], vec![(2, 2)], vec![(2, 4)]),
        Win::Again | Win::Traveller => three(vec![(1, 1)], vec![(1, 2)], vec![(2, 1), (1, 1)]),
        Win::FullTeam | Win::Elite => three(vec![(2, 1)], vec![(2, 2)], vec![(2, 3)]),
        Win::Champion => three(vec![(3, 1)], vec![(3, 1), (2, 3)], vec![(3, 2), (2, 5)]),
    }
}

/// A prize in words: "3 Great Balls", "a Master Ball and 3 Ultra Balls".
pub fn prize_words(balls: &[(u8, u32)]) -> String {
    let words: Vec<String> = balls
        .iter()
        .map(|(b, n)| {
            let name = crate::quest::BALL_NAMES[(*b as usize).min(3)];
            match n {
                1 if name.starts_with('U') => format!("an {name}"),
                1 => format!("a {name}"),
                _ => format!("{n} {name}s"),
            }
        })
        .collect();
    words.join(" and ")
}

fn team_size(stage: usize) -> usize {
    match stage {
        0..=7 => [2, 3, 3, 4, 4, 5, 5, 6][stage],
        8..=11 => 5,
        _ => 6,
    }
}

fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Who can be on a team at this level: young Pokémon early on, fully evolved ones later.
fn fits_level(name: &str, level: u32) -> bool {
    // The stage it'd really be at this level: evolved by now, but not yet into the next one
    // (a Lv 20 trainer has a Charmeleon, never a Charizard or a Charmander).
    pokemon::stage_at(name, level) == name && pokemon::evolutions_of(name).iter().all(|(_, at)| level < *at)
}

/// Pokémon of a region (0..9) and type ("" for any), not legendary, that fit the level.
fn pool(region: Option<usize>, t: &str, level: u32) -> Vec<&'static str> {
    pool_between(region, t, level, level)
}

/// The same, fitting every level from `lo` to `hi` (a team's weakest to its ace).
fn pool_between(region: Option<usize>, t: &str, lo: u32, hi: u32) -> Vec<&'static str> {
    NATIONAL
        .iter()
        .filter(|s| s.form == 0 && !s.legend && !s.baby)
        .filter(|s| region.is_none_or(|r| s.generation as usize == r + 1))
        .filter(|s| t.is_empty() || pokemon::TYPES.get(s.t1 as usize).is_some_and(|x| x.0 == t) || pokemon::TYPES.get(s.t2 as usize).is_some_and(|x| x.0 == t))
        .filter(|s| fits_level(s.name, lo) && fits_level(s.name, hi))
        .map(|s| s.name)
        .collect()
}

/// Pick `n` different ones from `from`, the same every time for `seed`; the strongest last
/// (the ace).
fn pick(from: &[&'static str], n: usize, seed: &str) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = from.to_vec();
    v.sort_by_key(|n| hash(&format!("{seed}{n}")));
    v.truncate(n);
    v.sort_by_key(|n| base_total(n));
    v
}

/// A league trainer's team, as (sprite name, level).
#[cfg(test)]
pub fn stage_team(region: usize, stage: usize) -> Vec<(&'static str, u32)> {
    team(region, stage, false, 0)
}

/// A gym leader's rematch team (six, at the rematch level).
#[cfg(test)]
pub fn rematch_team(region: usize, stage: usize) -> Vec<(&'static str, u32)> {
    team(region, stage.min(7), true, 0)
}

/// A trainer's team; `bonus` levels higher on Hard and Max (still of their type, each at
/// the stage that fits the higher level).
pub fn team(region: usize, stage: usize, rematch: bool, bonus: u32) -> Vec<(&'static str, u32)> {
    let region = region % 9;
    let level = (if rematch { rematch_level(region, stage) } else { stage_level(region, stage) } + bonus).min(100);
    let t = stage_type(region, stage);
    let n = if rematch { 6 } else { team_size(stage) };
    let seed = format!("{}-{stage}{}", REGIONS[region], if rematch { "-rematch" } else { "" });
    // The ace is at the stage's level, the others up to 4 below.
    let lo = level.saturating_sub((n as u32 - 1).min(4)).max(2);
    let names: Vec<&'static str> = if stage == CHAMPION {
        // The region's strongest, of different types.
        let mut all = pool_between(Some(region), "", lo, level);
        all.sort_by_key(|n| std::cmp::Reverse(base_total(n)));
        let mut out: Vec<&'static str> = Vec::new();
        for n in all {
            let t = pokemon::type_of(n);
            if !out.iter().any(|o| pokemon::type_of(o) == t) {
                out.push(n);
            }
            if out.len() == 6 {
                break;
            }
        }
        out.sort_by_key(|n| base_total(n));
        out
    } else {
        let mut from = pool_between(Some(region), t, lo, level);
        if from.len() < n {
            // Too few of this type from the region: others of the type join in.
            let extra: Vec<&'static str> = pool_between(None, t, lo, level).into_iter().filter(|x| !from.contains(x)).collect();
            from.extend(pick(&extra, n - from.len(), &seed));
        }
        pick(&from, n, &seed)
    };
    let k = names.len();
    names.into_iter().enumerate().map(|(i, nm)| (nm, level.saturating_sub(((k - 1 - i) as u32).min(4)).max(2))).collect()
}

pub fn stage_mons(region: usize, stage: usize, rematch: bool, bonus: u32) -> Vec<Mon> {
    team(region, stage, rematch, bonus).into_iter().map(|(n, l)| Mon::new(n, l)).collect()
}

/// A travelling trainer: title, name, team.
#[derive(Clone, Debug)]
pub struct Traveller {
    pub title: &'static str,
    pub name: &'static str,
    pub team: Vec<(&'static str, u32)>,
}

const TITLES: [&str; 12] = ["Hiker", "Swimmer", "Birdwatcher", "Fisher", "Student", "Ranger", "Climber", "Gardener", "Skater", "Chef", "Painter", "Busker"];
const NAMES: [&str; 24] = [
    "Dale", "Fern", "Milo", "Tess", "Gus", "Wren", "Ned", "Pia", "Abe", "Lux", "Rory", "Sana", "Jem", "Kit", "Bo", "Nell", "Otto", "Faye", "Cy", "Dot", "Hal", "Ivo", "Lea", "Zed",
];

/// Today's three travelling trainers, near your team's level.
pub fn travellers(day: &str, level: u32, bonus: u32) -> Vec<Traveller> {
    (0..3)
        .map(|i| {
            let h = hash(&format!("{day}-{i}"));
            let title = TITLES[(h % TITLES.len() as u64) as usize];
            let name = NAMES[((h >> 8) % NAMES.len() as u64) as usize];
            let region = ((h >> 16) % 9) as usize;
            let lv = (level as i64 - 3 + ((h >> 24) % 4) as i64 + bonus as i64).clamp(3, 100) as u32;
            let n = 2 + ((h >> 28) % 2) as usize;
            let from = pool(Some(region), "", lv);
            let team = pick(&from, n, &format!("{day}-{i}-team")).into_iter().map(|nm| (nm, lv)).collect();
            Traveller { title, name, team }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stage_has_a_full_team() {
        for r in 0..9 {
            for s in 0..STAGES {
                let t = stage_team(r, s);
                assert_eq!(t.len(), team_size(s), "{} stage {s}", REGIONS[r]);
                let (hi, bye) = lines(r, s);
                assert!(hi.chars().count() <= 46 && bye.chars().count() <= 46, "{hi} / {bye}");
                if s < ELITE {
                    let re = rematch_team(r, s);
                    assert_eq!(re.len(), 6, "{} rematch {s}", REGIONS[r]);
                    let ty = stage_type(r, s);
                    assert!(re.iter().all(|(n, l)| pokemon::has_type(n, ty) && pokemon::stage_at(n, *l) == *n), "{} rematch {s}: {re:?}", REGIONS[r]);
                    assert!(rematch_level(r, s) > stage_level(r, s));
                }
                let mut names: Vec<_> = t.iter().map(|x| x.0).collect();
                names.dedup();
                assert_eq!(names.len(), t.len());
                assert!(t.iter().all(|(n, l)| !pokemon::is_legend(n) && *l <= 100));
                // Each at the stage that fits its level: no Lv 20 Charizard.
                assert!(t.iter().all(|(n, l)| pokemon::stage_at(n, *l) == *n), "{} stage {s}: {t:?}", REGIONS[r]);
                if s < CHAMPION {
                    let ty = stage_type(r, s);
                    assert!(t.iter().all(|(n, _)| pokemon::has_type(n, ty)), "{} stage {s} {ty}: {t:?}", REGIONS[r]);
                }
                // Hard and Max: higher, and still a full team that fits.
                for d in 1..3u8 {
                    for re in [false, true] {
                        if re && s >= ELITE {
                            continue;
                        }
                        let h = team(r, s, re, bonus(d));
                        let base = team(r, s, re, 0);
                        assert_eq!(h.len(), base.len(), "{} stage {s} d{d}", REGIONS[r]);
                        assert!(h.last().unwrap().1 >= base.last().unwrap().1, "{} stage {s} d{d}", REGIONS[r]);
                        assert!(h.iter().all(|(n, l)| pokemon::stage_at(n, *l) == *n && !pokemon::is_legend(n)), "{} stage {s} d{d}: {h:?}", REGIONS[r]);
                        if s < CHAMPION {
                            let ty = stage_type(r, s);
                            assert!(h.iter().all(|(n, _)| pokemon::has_type(n, ty)), "{} stage {s} d{d}: {h:?}", REGIONS[r]);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn prizes_grow_with_difficulty() {
        let worth = |v: Vec<(u8, u32)>| v.iter().map(|(b, n)| [1, 2, 4, 20][*b as usize] * n).sum::<u32>();
        for w in [Win::Badge { late: false }, Win::Badge { late: true }, Win::Again, Win::FullTeam, Win::Elite, Win::Champion, Win::Traveller] {
            assert!(worth(prize(w, 0)) > 0);
            assert!(worth(prize(w, 1)) > worth(prize(w, 0)), "{w:?}");
            assert!(worth(prize(w, 2)) > worth(prize(w, 1)), "{w:?}");
        }
        assert_eq!(prize_words(&prize(Win::Champion, 2)), "2 Master Balls and 5 Ultra Balls");
        assert_eq!(prize_words(&prize(Win::Elite, 0)), "an Ultra Ball");
        assert_eq!(prize_words(&prize(Win::Traveller, 0)), "a Great Ball");
        assert_eq!(scaled_xp(1000, 2), 2000);
        assert_eq!(scaled_xp(250, 1), 375);
        assert_eq!(travellers("2026-11-19", 50, 10)[0].team[0].1, travellers("2026-11-19", 50, 0)[0].team[0].1 + 10);
    }

    #[test]
    fn levels_rise() {
        assert_eq!(stage_level(0, 0), 12);
        assert_eq!(stage_level(0, CHAMPION), 68);
        assert_eq!(stage_level(8, CHAMPION), 100);
        assert!(stage_level(0, 7) < stage_level(0, ELITE));
        assert_eq!(trainer_name(0, 0), "Gym Leader Oren");
        assert_eq!(badge_name("Fire"), "Cinder Badge");
        assert_eq!(party_level(0, 0), 14);
        assert_eq!(party_level(0, ELITE), 66);
        assert_eq!(party_level(8, CHAMPION), 98);
    }

    #[test]
    fn travellers_change_daily() {
        let a = travellers("2026-11-19", 50, 0);
        assert_eq!(a.len(), 3);
        assert!(a.iter().all(|t| (2..=3).contains(&t.team.len())));
        let b = travellers("2026-11-20", 50, 0);
        assert!(a.iter().zip(&b).any(|(x, y)| x.team[0].0 != y.team[0].0 || x.name != y.name));
    }
}

