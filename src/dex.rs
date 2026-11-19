//! The team's experience and the Pokédex, kept in `~/.config/pocketisle/pokedex.json`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Dex {
    /// The whole team's experience: the level decides who has evolved.
    pub xp: u64,
    /// How many of each Pokémon were caught (and how many of those were shiny).
    pub caught: BTreeMap<String, u32>,
    pub shiny: BTreeMap<String, u32>,
    /// Seen walking past (caught or not).
    pub seen: BTreeMap<String, u32>,
    /// When the next wild Pokémon is due (unix seconds), so restarts don't reset the wait.
    pub next_wild: i64,
    /// The egg being carried, if any.
    pub egg: Option<Egg>,
    pub hatched: u32,
    /// The Pokémon looking after the island for the weather (sprite name; empty = Rayquaza),
    /// the weather it came for, and when (unix seconds).
    pub mascot: String,
    pub mascot_for: String,
    pub mascot_at: i64,
    /// Catch combo: the species caught last and how many in a row.
    pub chain_species: String,
    pub chain: u32,
    /// The Pokédex tab sorts by type instead of number.
    pub by_type: bool,
    /// The last few caught or hatched (sprite names), newest first.
    pub recent: Vec<String>,
    /// The day the morning greeting was last shown ("2026-11-16").
    pub greeted: String,
    /// Team members you've seen on the island (by name: "Infernape" for the Chimchar line):
    /// only their evolutions are announced.
    pub met: Vec<String>,
    /// For the trainer card: days you've used the PC with the island, the first and the
    /// last of them, the longest catch combo and the special days you caught something on.
    pub days_played: u32,
    pub first_day: String,
    pub last_day: String,
    pub best_chain: u32,
    pub best_chain_species: String,
    pub special_catches: BTreeMap<String, u32>,
    /// The wild Pokémon waiting on the island (it stays until caught or it runs off).
    pub visitor: Option<Visitor>,
    /// Great, Ultra and Master Balls, and which one you'll throw.
    pub bag: crate::quest::Bag,
    /// The mass outbreak going on, if any.
    pub outbreak: Option<crate::quest::Outbreak>,
    /// Today's challenges, and how many days you've finished all three.
    pub daily: crate::quest::Daily,
    pub full_days: u32,
    pub battles_won: u32,
    pub battles_lost: u32,
    /// This weekend's raid, and how many raid bosses you've caught.
    pub raid: Option<crate::quest::Raid>,
    pub raids_won: u32,
    /// Running totals for the legend quests and medals ("fished", "night", …).
    pub counts: BTreeMap<String, u32>,
    /// Legend quests finished whose legendary is still to be caught, and those all done.
    pub quests_open: Vec<String>,
    pub quests_done: Vec<String>,
    /// Medals earned (their keys).
    pub medals: Vec<String>,
    /// Quests and medals have been checked once (the first time, quietly).
    pub goals_seen: bool,
    /// Your buddy (on the pill in the keeper's place; empty = none), each Pokémon's
    /// friendship, the minutes you've spent together (a point every five), and today's
    /// friendship from petting (capped each day).
    pub buddy: String,
    pub friendship: BTreeMap<String, u32>,
    pub buddy_minutes: u32,
    pub pet_day: String,
    pub pet_today: u32,
    /// Your battle team (sprite names, up to six; empty = picked for you).
    pub team: Vec<String>,
    /// Your League party, for gyms, the Elite Four and champions (empty = the same as your
    /// team). It battles at the league's levels, not your team's.
    pub league_team: Vec<String>,
    /// Gym badges ("Kanto 1" … "Kanto 8"), the regions you're champion of, and an Elite Four
    /// run in progress.
    pub badges: Vec<String>,
    pub champions: Vec<String>,
    pub league_run: Option<LeagueRun>,
    /// How tough league and travelling trainers are: 0 Normal, 1 Hard, 2 Max.
    pub league_difficulty: u8,
    /// The regions whose champion you've beaten on Max.
    pub max_champions: Vec<String>,
    /// The day of the travelling trainers you've beaten (by number).
    pub travel_day: String,
    pub travel_beaten: Vec<u8>,
}

/// The Elite Four and champion, back to back: the region, who's next (8–12) and your team's
/// HP carried from battle to battle.
#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct LeagueRun {
    pub region: usize,
    pub stage: usize,
    pub hp: Vec<u32>,
    /// PP left, per Pokémon per move.
    pub pp: Vec<Vec<u8>>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Visitor {
    pub name: String,
    pub shiny: bool,
    pub legend: bool,
    /// It wants to battle first; or it lost one and is tired (easier to catch).
    pub battle: bool,
    pub tired: bool,
    /// A weekend raid boss; out in weather that suits it (more XP, easier to catch).
    pub raid: bool,
    pub boosted: bool,
    /// Fished up rather than walked by.
    pub fished: bool,
}

impl Dex {
    /// Remember a catch for the "Recent" row.
    pub fn remember(&mut self, sprite: String) {
        self.recent.insert(0, sprite);
        self.recent.truncate(10);
    }
}

/// A Pokémon Egg: it hatches after `need` minutes of you using the computer.
#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Egg {
    pub species: String,
    pub shiny: bool,
    pub minutes: u32,
    pub need: u32,
}

impl Egg {
    pub fn sprite(&self) -> String {
        if self.shiny {
            format!("{}-shiny", self.species)
        } else {
            self.species.clone()
        }
    }
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn path() -> std::path::PathBuf {
    crate::util::config_dir().join("pokedex.json")
}

impl Dex {
    pub fn load() -> Dex {
        std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path(), s);
        }
    }

    pub fn level(&self) -> u32 {
        level_for(self.xp)
    }
}

/// Level 1 at 0 XP; level L needs 4·(L−1)² XP (Lv 16 at 900, Lv 36 at 4900, Lv 50 at 9604).
pub fn level_for(xp: u64) -> u32 {
    ((xp as f64 / 4.0).sqrt().floor() as u32 + 1).min(100)
}

pub fn xp_at(level: u32) -> u64 {
    4 * (level.max(1) as u64 - 1).pow(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_line_up() {
        for l in 1..100 {
            assert_eq!(level_for(xp_at(l)), l);
            assert_eq!(level_for(xp_at(l + 1) - 1), l);
        }
    }
}
