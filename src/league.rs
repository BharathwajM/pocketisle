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
fn fits_level(name: &str, stage: u8, level: u32) -> bool {
    let last = pokemon::evolutions_of(name).is_empty();
    if level < 18 {
        stage == 0
    } else if level < 32 {
        stage <= 1
    } else {
        last
    }
}

/// Pokémon of a region (0..9) and type ("" for any), not legendary, that fit the level.
fn pool(region: Option<usize>, t: &str, level: u32) -> Vec<&'static str> {
    NATIONAL
        .iter()
        .filter(|s| s.form == 0 && !s.legend && !s.baby)
        .filter(|s| region.is_none_or(|r| s.generation as usize == r + 1))
        .filter(|s| t.is_empty() || pokemon::TYPES.get(s.t1 as usize).is_some_and(|x| x.0 == t) || pokemon::TYPES.get(s.t2 as usize).is_some_and(|x| x.0 == t))
        .filter(|s| fits_level(s.name, s.stage, level))
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
pub fn stage_team(region: usize, stage: usize) -> Vec<(&'static str, u32)> {
    let region = region % 9;
    let level = stage_level(region, stage);
    let t = stage_type(region, stage);
    let n = team_size(stage);
    let seed = format!("{}-{stage}", REGIONS[region]);
    let names: Vec<&'static str> = if stage == CHAMPION {
        // The region's strongest, of different types.
        let mut all = pool(Some(region), "", level);
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
        let mut from = pool(Some(region), t, level);
        if from.len() < n {
            // Too few of this type from the region: others of the type join in.
            let extra: Vec<&'static str> = pool(None, t, level).into_iter().filter(|x| !from.contains(x)).collect();
            from.extend(pick(&extra, n - from.len(), &seed));
        }
        pick(&from, n, &seed)
    };
    let k = names.len();
    names.into_iter().enumerate().map(|(i, nm)| (nm, level.saturating_sub(((k - 1 - i) as u32).min(4)).max(2))).collect()
}

pub fn stage_mons(region: usize, stage: usize) -> Vec<Mon> {
    stage_team(region, stage).into_iter().map(|(n, l)| Mon::new(n, l)).collect()
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
pub fn travellers(day: &str, level: u32) -> Vec<Traveller> {
    (0..3)
        .map(|i| {
            let h = hash(&format!("{day}-{i}"));
            let title = TITLES[(h % TITLES.len() as u64) as usize];
            let name = NAMES[((h >> 8) % NAMES.len() as u64) as usize];
            let region = ((h >> 16) % 9) as usize;
            let lv = (level as i64 - 3 + ((h >> 24) % 4) as i64).clamp(3, 100) as u32;
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
                let mut names: Vec<_> = t.iter().map(|x| x.0).collect();
                names.dedup();
                assert_eq!(names.len(), t.len());
                assert!(t.iter().all(|(n, l)| !pokemon::is_legend(n) && *l <= 100));
                if s < CHAMPION {
                    let ty = stage_type(r, s);
                    assert!(t.iter().all(|(n, _)| pokemon::has_type(n, ty)), "{} stage {s} {ty}: {t:?}", REGIONS[r]);
                }
            }
        }
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
        let a = travellers("2026-11-19", 50);
        assert_eq!(a.len(), 3);
        assert!(a.iter().all(|t| (2..=3).contains(&t.team.len())));
        let b = travellers("2026-11-20", 50);
        assert!(a.iter().zip(&b).any(|(x, y)| x.team[0].0 != y.team[0].0 || x.name != y.name));
    }
}

