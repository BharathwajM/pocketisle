//! The game around the wild Pokémon: Poké Balls in the bag, daily challenges, mass
//! outbreaks and battles. Pure rules here; the island draws and runs them.

use serde::{Deserialize, Serialize};

use crate::pokemon;

/// The balls: 0 Poké Ball (never runs out), 1 Great, 2 Ultra, 3 Master.
pub const BALL_NAMES: [&str; 4] = ["Poké Ball", "Great Ball", "Ultra Ball", "Master Ball"];

/// The bag (the plain Poké Ball is unlimited).
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Bag {
    pub great: u32,
    pub ultra: u32,
    pub master: u32,
    /// The ball you'll throw next.
    pub pick: u8,
    /// The starter gift has been given.
    pub gift: bool,
}

impl Bag {
    pub fn count(&self, ball: u8) -> Option<u32> {
        match ball {
            1 => Some(self.great),
            2 => Some(self.ultra),
            3 => Some(self.master),
            _ => None,
        }
    }

    pub fn add(&mut self, ball: u8, n: u32) {
        match ball {
            1 => self.great += n,
            2 => self.ultra += n,
            3 => self.master += n,
            _ => {}
        }
    }

    /// Take the picked ball for a throw (falls back to a Poké Ball when it's run out).
    pub fn take(&mut self) -> u8 {
        let b = self.pick;
        let slot = match b {
            1 => &mut self.great,
            2 => &mut self.ultra,
            3 => &mut self.master,
            _ => return 0,
        };
        if *slot == 0 {
            self.pick = 0;
            return 0;
        }
        *slot -= 1;
        if *slot == 0 {
            self.pick = 0;
        }
        b
    }
}

/// How likely a throw is to hold, with this ball (a Master Ball always does). A visitor you
/// beat in a battle is tired and easier to catch.
pub fn catch_chance(base: f32, ball: u8, tired: bool) -> f32 {
    if ball == 3 {
        return 1.0;
    }
    let k = match ball {
        1 => 1.5,
        2 => 2.0,
        _ => 1.0,
    };
    let c = base * k + if tired { 0.25 } else { 0.0 };
    c.min(if base >= 1.0 { 1.0 } else { 0.97 })
}

/// A mass outbreak: one species everywhere until `until` (unix seconds).
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Outbreak {
    pub species: String,
    pub until: i64,
    pub caught: u32,
}

/// Shiny odds are this many times better for the outbreak's species.
pub const OUTBREAK_SHINY_X: f32 = 3.0;
/// How long an outbreak lasts (seconds), and the share of visitors that are its species.
pub const OUTBREAK_SECS: i64 = 60 * 60;
pub const OUTBREAK_SHARE: f32 = 0.7;
/// The chance a visit starts an outbreak (when none is on).
pub const OUTBREAK_CHANCE: f32 = 0.1;

/// Today's challenges.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Daily {
    pub day: String,
    pub list: Vec<Challenge>,
    /// The all-three bonus was given.
    pub bonus: bool,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Challenge {
    /// "catch", "type", "first" (on the first ball), "throw" (a Great throw or better),
    /// "battle" (win), "hatch".
    pub kind: String,
    /// The type for "type".
    pub t: String,
    pub need: u32,
    pub got: u32,
    /// The reward: a ball (1 Great, 2 Ultra) and how many.
    pub ball: u8,
    pub balls: u32,
}

impl Challenge {
    pub fn done(&self) -> bool {
        self.got >= self.need
    }

    pub fn text(&self) -> String {
        let n = self.need;
        let s = |one: &str, many: &str| if n == 1 { one.to_string() } else { many.to_string() };
        match self.kind.as_str() {
            "type" => format!("Catch {n} {}-type Pokémon", self.t),
            "first" => format!("Catch {n} on the first ball"),
            "throw" => format!("Make {n} Great throws or better"),
            "battle" => format!("Win {n} {}", s("battle", "battles")),
            "hatch" => "Hatch an Egg".to_string(),
            _ => format!("Catch {n} Pokémon"),
        }
    }

    pub fn reward(&self) -> String {
        let b = BALL_NAMES[self.ball.min(3) as usize];
        if self.balls == 1 { b.to_string() } else { format!("{} {b}s", self.balls) }
    }
}

/// XP for finishing all three, and how many all-three days earn a Master Ball.
pub const DAILY_BONUS_XP: u32 = 100;
pub const MASTER_EVERY: u32 = 5;

/// A small, fixed number from a string (so a day's challenges are the same all day).
fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// The types common among wild visitors (so a type challenge can be done on any day).
fn common_types() -> Vec<&'static str> {
    let mut n = [0u32; 18];
    for name in pokemon::wild() {
        if let Some(s) = pokemon::species(name) {
            n[s.t1 as usize % 18] += 1;
            if (s.t2 as usize) < 18 {
                n[s.t2 as usize] += 1;
            }
        }
    }
    let mut idx: Vec<usize> = (0..18).collect();
    idx.sort_by(|a, b| n[*b].cmp(&n[*a]));
    idx.into_iter().take(8).map(|i| pokemon::TYPES[i].0).collect()
}

/// Three challenges for `day` ("2026-11-16"). `egg` = an Egg is being carried (so hatching
/// one can be a challenge); `timed` = timed catching is on (so throws can be judged).
pub fn challenges_for(day: &str, egg: bool, timed: bool) -> Vec<Challenge> {
    let mut h = hash(day);
    let mut roll = |n: u64| {
        h = h.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (h >> 33) % n.max(1)
    };
    let types = common_types();
    let mut kinds: Vec<&str> = vec!["catch", "type", "first", "battle"];
    if timed {
        kinds.push("throw");
    }
    if egg {
        kinds.push("hatch");
    }
    let mut out: Vec<Challenge> = Vec::new();
    // Always one plain "catch" first, then two different others.
    out.push(Challenge { kind: "catch".into(), need: 3 + roll(3) as u32, ball: 1, balls: 2, ..Default::default() });
    let mut rest: Vec<&str> = kinds.into_iter().filter(|k| *k != "catch").collect();
    for slot in 0..2 {
        if rest.is_empty() {
            break;
        }
        let k = rest.remove(roll(rest.len() as u64) as usize);
        let mut c = Challenge { kind: k.into(), ..Default::default() };
        match k {
            "type" => {
                c.t = types.get(roll(types.len() as u64) as usize).copied().unwrap_or("Normal").to_string();
                c.need = 2;
            }
            "first" => c.need = 2,
            "throw" => c.need = 2 + roll(2) as u32,
            "battle" => c.need = 1 + roll(2) as u32,
            _ => c.need = 1,
        }
        // The second one gives Great Balls, the last an Ultra Ball.
        (c.ball, c.balls) = if slot == 0 { (1, 3) } else { (2, 1) };
        out.push(c);
    }
    out
}

/// The type chart: how strong each attacking type (row) is against each defending type
/// (column), in `pokemon::TYPES` order. '+' double, '-' half, '0' none, '.' normal.
const CHART: [&str; 18] = [
    "............-0..-.",
    ".--+.+.....+-.-.+.",
    ".+--....+...+.-...",
    ".-+-...-+-.-+.-.-.",
    "..+--...0+....-...",
    ".--+.-..++....+.-.",
    "+....+.-.---+0.++-",
    "...+...--...--..0+",
    ".+.-+..+.0.-+...+.",
    "...+-.+....+-...-.",
    "......++..-....0-.",
    ".-.+..--.-+..-.+--",
    ".+...+-.-+.+....-.",
    "0.........+..+.-..",
    "..............+.-0",
    "......-...+..+.-.-",
    ".--.-+......+...-+",
    ".-....+-......++-.",
];

/// How effective an attack of type `atk` is on a Pokémon with these types.
pub fn effect(atk: usize, def: &[usize]) -> f32 {
    def.iter()
        .filter(|d| **d < 18)
        .map(|d| match CHART[atk % 18].as_bytes()[*d] {
            b'+' => 2.0,
            b'-' => 0.5,
            b'0' => 0.0,
            _ => 1.0,
        })
        .product()
}

/// A Pokémon's types (one or two), by sprite or species name.
pub fn types_of(sprite: &str) -> Vec<usize> {
    let base = sprite.trim_end_matches("-shiny");
    let Some(s) = pokemon::species(base) else { return vec![0] };
    let mut t = vec![s.t1 as usize];
    if (s.t2 as usize) < 18 {
        t.push(s.t2 as usize);
    }
    t
}

/// The best attack each side has against the other: (type, how effective).
pub fn best_attack(atk: &[usize], def: &[usize]) -> (usize, f32) {
    atk.iter().map(|a| (*a, effect(*a, def))).fold((atk[0], -1.0), |best, x| if x.1 > best.1 { x } else { best })
}

/// The chance your Pokémon (`me`, team level `level`) beats a wild one (`them`).
pub fn win_chance(me: &[usize], them: &[usize], level: u32) -> f32 {
    let (_, mine) = best_attack(me, them);
    let (_, theirs) = best_attack(them, me);
    let l = |x: f32| if x <= 0.0 { -2.0 } else { x.log2() };
    let p = 0.55 + 0.15 * (l(mine) - l(theirs)) + (level as f32 - 30.0) / 200.0;
    p.clamp(0.15, 0.92)
}

/// "super effective" and friends, for a battle's card.
pub fn effect_words(x: f32) -> &'static str {
    if x >= 2.0 {
        "super effective"
    } else if x == 0.0 {
        "no use at all"
    } else if x < 1.0 {
        "not very effective"
    } else {
        "a solid hit"
    }
}

/// A weekend raid: one strong legendary, the same all weekend. Win battles to break its
/// shields, then catch it (five balls before it gets away).
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Raid {
    /// The weekend's Saturday ("2026-11-21").
    pub weekend: String,
    pub species: String,
    /// Battle wins still needed.
    pub shields: u32,
    /// After a lost battle it can't be battled again until then (unix seconds).
    pub rest_until: i64,
    /// It has turned up; it was caught; it was caught or got away.
    pub met: bool,
    pub caught: bool,
    pub done: bool,
}

pub const RAID_SHIELDS: u32 = 3;
pub const RAID_THROWS: u8 = 5;
pub const RAID_REST_SECS: i64 = 300;
pub const RAID_XP: u32 = 200;
/// Once it has turned up, the share of weekend visitors that are the raid boss coming back.
pub const RAID_SHARE: f32 = 0.35;

/// The Saturday of this weekend, on a Saturday or Sunday.
pub fn raid_weekend(d: crate::calendar::Date) -> Option<crate::calendar::Date> {
    match d.weekday() {
        6 => Some(d),
        0 => Some(d.add(-1)),
        _ => None,
    }
}

/// Days until the next raid starts (0 on a weekend).
pub fn days_to_raid(d: crate::calendar::Date) -> i64 {
    match d.weekday() {
        6 | 0 => 0,
        w => 6 - w as i64,
    }
}

/// The weekend's boss, the same all weekend: one of `pool`.
pub fn raid_species<'a>(weekend: &str, pool: &[&'a str]) -> Option<&'a str> {
    (!pool.is_empty()).then(|| pool[(hash(&format!("raid {weekend}")) % pool.len() as u64) as usize])
}

/// A raid boss is tougher than a visitor.
pub fn raid_win_chance(me: &[usize], them: &[usize], level: u32) -> f32 {
    (win_chance(me, them, level) - 0.15).clamp(0.1, 0.8)
}

/// What a legend quest asks for.
pub enum Goal {
    /// Catch all of these.
    Catch(&'static [&'static str]),
    /// One of the Pokédex's running totals reaches this.
    Count(&'static str, u32),
    /// This many species in the Pokédex.
    Species(u32),
}

/// Finish one and its legendary starts visiting (one visitor in two) until it's caught.
pub struct LegendQuest {
    pub key: &'static str,
    pub legend: &'static str,
    pub title: &'static str,
    pub goal: Goal,
}

pub const QUESTS: [LegendQuest; 10] = [
    LegendQuest { key: "birds", legend: "lugia", title: "Catch Articuno, Zapdos & Moltres", goal: Goal::Catch(&["articuno", "zapdos", "moltres"]) },
    LegendQuest { key: "beasts", legend: "ho-oh", title: "Catch Raikou, Entei & Suicune", goal: Goal::Catch(&["raikou", "entei", "suicune"]) },
    LegendQuest { key: "lakes", legend: "dialga", title: "Catch Uxie, Mesprit & Azelf", goal: Goal::Catch(&["uxie", "mesprit", "azelf"]) },
    LegendQuest { key: "night_water", legend: "kyogre", title: "Catch 10 water types at night", goal: Goal::Count("night_water", 10) },
    LegendQuest { key: "sun_fire", legend: "groudon", title: "Catch 10 fire types on sunny days", goal: Goal::Count("sun_fire", 10) },
    LegendQuest { key: "psychic", legend: "mewtwo", title: "Catch 10 psychic types", goal: Goal::Count("psychic", 10) },
    LegendQuest { key: "ghost", legend: "giratina", title: "Catch 10 ghost types", goal: Goal::Count("ghost", 10) },
    LegendQuest { key: "dragon", legend: "rayquaza", title: "Catch 8 dragon types", goal: Goal::Count("dragon", 8) },
    LegendQuest { key: "fished", legend: "palkia", title: "Fish up 15 Pokémon", goal: Goal::Count("fished", 15) },
    LegendQuest { key: "dex", legend: "mew", title: "Fill 100 Pokédex entries", goal: Goal::Species(100) },
];

/// One visitor in this many is an unlocked quest's legendary.
pub const QUEST_SHARE: f32 = 0.5;

/// Everything the quests and medals count.
#[derive(Default, Clone, Debug)]
pub struct Stats {
    pub caught: u32,
    pub species: u32,
    pub shinies: u32,
    pub legends: u32,
    pub hatched: u32,
    pub battles: u32,
    pub raids: u32,
    pub combo: u32,
    pub days: u32,
    pub full_days: u32,
    pub level: u32,
    pub quests: u32,
    /// The running totals ("fished", "night", "boosted", …).
    pub counts: std::collections::BTreeMap<String, u32>,
    /// Who's been caught.
    pub have: std::collections::BTreeSet<String>,
}

impl Stats {
    pub fn count(&self, k: &str) -> u32 {
        self.counts.get(k).copied().unwrap_or(0)
    }
}

/// How far along a quest is: (done, needed).
pub fn quest_progress(q: &LegendQuest, s: &Stats) -> (u32, u32) {
    match q.goal {
        Goal::Catch(list) => (list.iter().filter(|n| s.have.contains(**n)).count() as u32, list.len() as u32),
        Goal::Count(k, n) => (s.count(k).min(n), n),
        Goal::Species(n) => (s.species.min(n), n),
    }
}

/// A medal on the trainer card: earned once `stat` reaches `need`.
pub struct Medal {
    pub key: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    pub stat: &'static str,
    pub need: u32,
}

pub const MEDALS: [Medal; 17] = [
    Medal { key: "first", name: "First catch", desc: "Catch your first Pokémon", stat: "caught", need: 1 },
    Medal { key: "collector", name: "Collector", desc: "50 species in your Pokédex", stat: "species", need: 50 },
    Medal { key: "professor", name: "Professor", desc: "250 species in your Pokédex", stat: "species", need: 250 },
    Medal { key: "shiny", name: "Shiny hunter", desc: "Catch a shiny", stat: "shinies", need: 1 },
    Medal { key: "sparkle", name: "Sparkle", desc: "Catch 5 shinies", stat: "shinies", need: 5 },
    Medal { key: "legend", name: "Legend tamer", desc: "Catch a legendary", stat: "legends", need: 1 },
    Medal { key: "nursery", name: "Nursery", desc: "Hatch 3 eggs", stat: "hatched", need: 3 },
    Medal { key: "angler", name: "Angler", desc: "Fish up 10 Pokémon", stat: "fished", need: 10 },
    Medal { key: "nightowl", name: "Night owl", desc: "Catch 25 Pokémon at night", stat: "night", need: 25 },
    Medal { key: "storm", name: "Storm chaser", desc: "Catch 20 weather-boosted Pokémon", stat: "boosted", need: 20 },
    Medal { key: "raid", name: "Raider", desc: "Catch a weekend raid boss", stat: "raids", need: 1 },
    Medal { key: "battler", name: "Battler", desc: "Win 10 battles", stat: "battles", need: 10 },
    Medal { key: "combo", name: "Combo king", desc: "A catch combo of 10", stat: "combo", need: 10 },
    Medal { key: "regular", name: "Regular", desc: "Play on 30 days", stat: "days", need: 30 },
    Medal { key: "dailies", name: "Daily grinder", desc: "Finish all three challenges on 7 days", stat: "full_days", need: 7 },
    Medal { key: "veteran", name: "Veteran", desc: "Team level 25", stat: "level", need: 25 },
    Medal { key: "questor", name: "Questor", desc: "Finish 3 legend quests", stat: "quests", need: 3 },
];

/// Where a medal's stat stands.
pub fn medal_value(m: &Medal, s: &Stats) -> u32 {
    match m.stat {
        "caught" => s.caught,
        "species" => s.species,
        "shinies" => s.shinies,
        "legends" => s.legends,
        "hatched" => s.hatched,
        "battles" => s.battles,
        "raids" => s.raids,
        "combo" => s.combo,
        "days" => s.days,
        "full_days" => s.full_days,
        "level" => s.level,
        "quests" => s.quests,
        k => s.count(k),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quests_and_medals() {
        for q in &QUESTS {
            assert!(pokemon::species(q.legend).is_some_and(|s| s.legend), "{}", q.legend);
            if let Goal::Catch(list) = q.goal {
                assert!(list.iter().all(|n| pokemon::species(n).is_some()), "{}", q.key);
            }
        }
        let mut s = Stats::default();
        s.have.insert("zapdos".into());
        s.counts.insert("fished".into(), 20);
        s.species = 40;
        assert_eq!(quest_progress(&QUESTS[0], &s), (1, 3));
        assert_eq!(quest_progress(&QUESTS[8], &s), (15, 15));
        assert_eq!(quest_progress(&QUESTS[9], &s), (40, 100));
        let angler = MEDALS.iter().find(|m| m.key == "angler").unwrap();
        assert_eq!(medal_value(angler, &s), 20);
        let collector = MEDALS.iter().find(|m| m.key == "collector").unwrap();
        assert!(medal_value(collector, &s) < collector.need);
        let keys: std::collections::BTreeSet<_> = MEDALS.iter().map(|m| m.key).collect();
        assert_eq!(keys.len(), MEDALS.len());
    }

    fn t(name: &str) -> usize {
        pokemon::TYPES.iter().position(|(n, _)| *n == name).unwrap()
    }

    #[test]
    fn chart_matches_the_games() {
        assert!(CHART.iter().all(|r| r.len() == 18));
        assert_eq!(effect(t("Fire"), &[t("Grass")]), 2.0);
        assert_eq!(effect(t("Water"), &[t("Fire")]), 2.0);
        assert_eq!(effect(t("Electric"), &[t("Ground")]), 0.0);
        assert_eq!(effect(t("Ground"), &[t("Flying")]), 0.0);
        assert_eq!(effect(t("Ice"), &[t("Dragon"), t("Flying")]), 4.0);
        assert_eq!(effect(t("Fighting"), &[t("Ghost")]), 0.0);
        assert_eq!(effect(t("Dragon"), &[t("Fairy")]), 0.0);
        assert_eq!(effect(t("Fairy"), &[t("Dragon")]), 2.0);
        assert_eq!(effect(t("Steel"), &[t("Fairy")]), 2.0);
        assert_eq!(effect(t("Normal"), &[t("Rock")]), 0.5);
        assert_eq!(effect(t("Psychic"), &[t("Dark")]), 0.0);
        assert_eq!(effect(t("Bug"), &[t("Dark")]), 2.0);
        assert_eq!(effect(t("Poison"), &[t("Steel")]), 0.0);
        assert_eq!(effect(t("Ghost"), &[t("Normal")]), 0.0);
    }

    #[test]
    fn type_advantage_wins_more() {
        let fire = [t("Fire")];
        let grass = [t("Grass")];
        let water = [t("Water")];
        assert!(win_chance(&fire, &grass, 30) > win_chance(&fire, &water, 30));
        assert!(win_chance(&fire, &grass, 60) > win_chance(&fire, &grass, 10));
        assert!((0.15..=0.92).contains(&win_chance(&fire, &water, 1)));
    }

    #[test]
    fn balls_help_and_run_out() {
        assert_eq!(catch_chance(0.1, 3, false), 1.0);
        assert!(catch_chance(0.3, 2, false) > catch_chance(0.3, 1, false));
        assert!(catch_chance(0.3, 1, false) > catch_chance(0.3, 0, false));
        assert!(catch_chance(0.9, 2, false) <= 0.97);
        assert_eq!(catch_chance(1.0, 0, false), 1.0);
        let mut bag = Bag { great: 1, pick: 1, ..Default::default() };
        assert_eq!(bag.take(), 1);
        assert_eq!((bag.great, bag.pick), (0, 0));
        assert_eq!(bag.take(), 0);
    }

    #[test]
    fn a_day_has_three_steady_challenges() {
        let a = challenges_for("2026-11-16", false, true);
        assert_eq!(a, challenges_for("2026-11-16", false, true));
        assert_eq!(a.len(), 3);
        assert_eq!(a[0].kind, "catch");
        assert_ne!(a[1].kind, a[2].kind);
        assert!(a.iter().all(|c| c.kind != "hatch"));
        assert!(challenges_for("2026-11-17", false, false).iter().all(|c| c.kind != "throw"));
        assert!(a.iter().all(|c| !c.text().is_empty() && c.need >= 1));
    }
    #[test]
    fn raids_on_weekends() {
        use crate::calendar::Date;
        let sat = Date { y: 2026, m: 11, d: 21 };
        let sun = Date { y: 2026, m: 11, d: 22 };
        let wed = Date { y: 2026, m: 11, d: 18 };
        assert_eq!(raid_weekend(sat), Some(sat));
        assert_eq!(raid_weekend(sun), Some(sat));
        assert_eq!(raid_weekend(wed), None);
        assert_eq!(days_to_raid(wed), 3);
        assert_eq!(days_to_raid(sun), 0);
        let pool = ["mewtwo", "lugia", "kyogre"];
        let a = raid_species("2026-11-21", &pool);
        assert!(a.is_some());
        assert_eq!(a, raid_species("2026-11-21", &pool));
        assert_eq!(raid_species("2026-11-21", &[]), None);
        let me = types_of("pikachu");
        let them = types_of("gyarados");
        assert!(raid_win_chance(&me, &them, 40) < win_chance(&me, &them, 40));
    }
}
