//! Turn-based battles: your team against a wild Pokémon or a trainer's team. Real base stats
//! and moves (`movedata`), the usual damage rules (level, Attack against Defense, same-type
//! bonus, type matchups, critical hits, a little luck), faster Pokémon first, switching, and
//! Poké Balls for wild ones. Stats use perfect IVs, no EVs and a neutral nature.

use crate::movedata::{Move, LEARNSET, MONS, MOVES};
use crate::pokemon;
use crate::quest;

/// One Pokémon in a battle.
#[derive(Clone, Debug)]
pub struct Mon {
    /// Sprite name ("infernape", "vulpix-alola").
    pub name: String,
    pub level: u32,
    pub types: Vec<usize>,
    /// HP, Attack, Defense, Sp. Atk, Sp. Def, Speed.
    pub stats: [u32; 6],
    pub hp: u32,
    /// Indexes into `MOVES` (one to four).
    pub moves: Vec<u16>,
}

fn base_row(name: &str) -> Option<&'static (u16, [u8; 6], u16, u8)> {
    let s = pokemon::species(name)?;
    let find = |id: u16| MONS.binary_search_by_key(&id, |m| m.0).ok().map(|i| &MONS[i]);
    find(s.id).or_else(|| find(s.base))
}

/// A stat at a level: perfect IVs, no EVs, a neutral nature.
pub fn stat(base: u32, level: u32, hp: bool) -> u32 {
    let core = (2 * base + 31) * level / 100;
    if hp { core + level + 10 } else { core + 5 }
}

fn tackle() -> u16 {
    MOVES.iter().position(|m| m.name == "Tackle").unwrap_or(0) as u16
}

/// The four moves it knows at this level: from what it has learned by levelling up, the
/// strongest (its own types count half again), preferring different types.
pub fn moves_at(name: &str, level: u32) -> Vec<u16> {
    let Some(row) = base_row(name) else { return vec![tackle()] };
    let types = quest::types_of(name);
    let all = &LEARNSET[row.2 as usize..row.2 as usize + row.3 as usize];
    let mut known: Vec<u16> = all.iter().filter(|(lv, _)| *lv as u32 <= level.max(1)).map(|(_, m)| *m).collect();
    if known.is_empty() {
        known = all.iter().take(1).map(|(_, m)| *m).collect();
    }
    if known.is_empty() {
        return vec![tackle()];
    }
    let score = |m: u16| {
        let mv = &MOVES[m as usize];
        let stab = if types.contains(&(mv.t as usize)) { 1.5 } else { 1.0 };
        let acc = if mv.acc == 0 { 1.0 } else { mv.acc as f32 / 100.0 };
        mv.power as f32 * stab * acc
    };
    let mut picked: Vec<u16> = Vec::new();
    while picked.len() < 4 {
        let best = known
            .iter()
            .copied()
            .filter(|m| !picked.contains(m))
            .map(|m| {
                let same = picked.iter().any(|p| MOVES[*p as usize].t == MOVES[m as usize].t);
                (m, score(m) * if same { 0.55 } else { 1.0 })
            })
            .max_by(|a, b| a.1.total_cmp(&b.1));
        match best {
            Some((m, _)) => picked.push(m),
            None => break,
        }
    }
    picked
}

impl Mon {
    pub fn new(name: &str, level: u32) -> Mon {
        let level = level.clamp(1, 100);
        let base = base_row(name).map(|r| r.1).unwrap_or([60; 6]);
        let mut stats = [0u32; 6];
        for (i, b) in base.iter().enumerate() {
            stats[i] = stat(*b as u32, level, i == 0);
        }
        Mon { name: name.to_string(), level, types: quest::types_of(name), stats, hp: stats[0], moves: moves_at(name, level) }
    }

    pub fn max_hp(&self) -> u32 {
        self.stats[0]
    }

    pub fn fainted(&self) -> bool {
        self.hp == 0
    }

    pub fn display(&self) -> String {
        pokemon::pretty(&self.name)
    }
}

/// Base stat total, for picking strong Pokémon.
pub fn base_total(name: &str) -> u32 {
    base_row(name).map_or(0, |r| r.1.iter().map(|x| *x as u32).sum())
}

/// How hard a move hits (before luck and critical hits) and how effective it is.
pub fn damage(att: &Mon, def: &Mon, mv: &Move, crit: bool, roll: f32) -> (u32, f32) {
    let eff = quest::effect(mv.t as usize, &def.types);
    if eff <= 0.0 {
        return (0, 0.0);
    }
    let (a, d) = if mv.special { (att.stats[3], def.stats[4]) } else { (att.stats[1], def.stats[2]) };
    let base = ((2.0 * att.level as f32 / 5.0 + 2.0) * mv.power as f32 * a as f32 / d.max(1) as f32) / 50.0 + 2.0;
    let stab = if att.types.contains(&(mv.t as usize)) { 1.5 } else { 1.0 };
    let crit = if crit { 1.5 } else { 1.0 };
    let dmg = (base * stab * eff * crit * roll.clamp(0.85, 1.0)).floor().max(1.0);
    (dmg as u32, eff)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Wild,
    Trainer,
}

#[derive(Clone, Debug)]
pub struct Side {
    pub mons: Vec<Mon>,
    pub active: usize,
}

impl Side {
    pub fn new(mons: Vec<Mon>) -> Side {
        let active = mons.iter().position(|m| !m.fainted()).unwrap_or(0);
        Side { mons, active }
    }
    pub fn cur(&self) -> &Mon {
        &self.mons[self.active]
    }
    fn cur_mut(&mut self) -> &mut Mon {
        &mut self.mons[self.active]
    }
    pub fn next_alive(&self) -> Option<usize> {
        self.mons.iter().position(|m| !m.fainted())
    }
}

/// One line of the battle, with both sides' active Pokémon and HP right after it (the panel
/// plays these back one by one).
#[derive(Clone, Debug)]
pub struct Step {
    pub text: String,
    pub me: (usize, u32),
    pub foe: (usize, u32),
    /// Someone was hit: true for the foe, false for yours.
    pub hit: Option<bool>,
    pub eff: f32,
    /// The move that hit (index into `MOVES`), for its animation.
    pub mv: Option<u16>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    Move(usize),
    Switch(usize),
    /// A ball: its name and how much it helps (≥ 100 always catches).
    Ball(&'static str, f32),
    Run,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Outcome {
    Won,
    Lost,
    Caught,
    Ran,
}

#[derive(Clone, Debug)]
pub struct Battle {
    pub kind: Kind,
    pub me: Side,
    pub foe: Side,
    /// The trainer's name ("Gym Leader Oren"), empty for a wild one.
    pub trainer: String,
    /// Wild: how easy it is to catch at full HP (0..1).
    pub catch_rate: f32,
    pub outcome: Option<Outcome>,
    /// Your Pokémon fainted: pick the next one (a free switch).
    pub must_switch: bool,
    pub turns: u32,
}

impl Battle {
    pub fn new(kind: Kind, me: Vec<Mon>, foe: Vec<Mon>, trainer: &str) -> Battle {
        Battle { kind, me: Side::new(me), foe: Side::new(foe), trainer: trainer.to_string(), catch_rate: 0.5, outcome: None, must_switch: false, turns: 0 }
    }

    fn step(&self, text: String, hit: Option<bool>, eff: f32) -> Step {
        Step { text, me: (self.me.active, self.me.cur().hp), foe: (self.foe.active, self.foe.cur().hp), hit, eff, mv: None }
    }

    /// The opening lines.
    pub fn intro(&self) -> Vec<Step> {
        let first = match self.kind {
            Kind::Wild => format!("A wild {} appeared!", self.foe.cur().display()),
            Kind::Trainer => format!("{} wants to battle! They sent out {}!", self.trainer, self.foe.cur().display()),
        };
        vec![self.step(first, None, 1.0), self.step(format!("Go, {}!", self.me.cur().display()), None, 1.0)]
    }

    /// The foe picks: wild Pokémon at random, trainers the move that should hurt most.
    fn foe_move(&self, rng: &mut dyn FnMut() -> f32) -> usize {
        let f = self.foe.cur();
        let n = f.moves.len().max(1);
        if self.kind == Kind::Wild {
            return ((rng() * n as f32) as usize).min(n - 1);
        }
        let me = self.me.cur();
        (0..n)
            .map(|i| {
                let mv = &MOVES[f.moves[i] as usize];
                let (d, _) = damage(f, me, mv, false, 0.92);
                let acc = if mv.acc == 0 { 1.0 } else { mv.acc as f32 / 100.0 };
                (i, d as f32 * acc * (0.8 + 0.4 * rng()))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0, |b| b.0)
    }

    /// `foe_side` attacks with move slot `slot`; returns the lines. Handles fainting.
    fn attack(&mut self, foe_side: bool, slot: usize, rng: &mut dyn FnMut() -> f32, out: &mut Vec<Step>) {
        let (att, def) = if foe_side { (self.foe.cur().clone(), self.me.cur().clone()) } else { (self.me.cur().clone(), self.foe.cur().clone()) };
        if att.fainted() || def.fainted() {
            return;
        }
        let Some(&m) = att.moves.get(slot) else { return };
        let mv = &MOVES[m as usize];
        let who = if foe_side {
            match self.kind {
                Kind::Wild => format!("The wild {}", att.display()),
                Kind::Trainer => format!("The foe's {}", att.display()),
            }
        } else {
            att.display()
        };
        if mv.acc > 0 && rng() * 100.0 >= mv.acc as f32 {
            out.push(self.step(format!("{who} used {}, but it missed!", mv.name), None, 1.0));
            return;
        }
        let crit = rng() < 1.0 / 24.0;
        let roll = 0.85 + 0.15 * rng();
        let (dmg, eff) = damage(&att, &def, mv, crit, roll);
        if eff <= 0.0 {
            out.push(self.step(format!("{who} used {}! It doesn't affect {}…", mv.name, def.display()), None, 0.0));
            return;
        }
        {
            let target = if foe_side { self.me.cur_mut() } else { self.foe.cur_mut() };
            target.hp = target.hp.saturating_sub(dmg);
        }
        let mut text = format!("{who} used {}!", mv.name);
        if crit {
            text.push_str(" A critical hit!");
        }
        if eff >= 2.0 {
            text.push_str(" It's super effective!");
        } else if eff < 1.0 {
            text.push_str(" It's not very effective…");
        }
        let mut s = self.step(text, Some(!foe_side), eff);
        s.mv = Some(m);
        out.push(s);
        let fainted = if foe_side { self.me.cur().fainted() } else { self.foe.cur().fainted() };
        if !fainted {
            return;
        }
        if foe_side {
            out.push(self.step(format!("{} fainted!", def.display()), None, 1.0));
            if self.me.next_alive().is_some() {
                self.must_switch = true;
                out.push(self.step("Choose your next Pokémon".into(), None, 1.0));
            } else {
                self.outcome = Some(Outcome::Lost);
                out.push(self.step("All your Pokémon have fainted…".into(), None, 1.0));
            }
        } else {
            let who = match self.kind {
                Kind::Wild => format!("The wild {}", def.display()),
                Kind::Trainer => format!("The foe's {}", def.display()),
            };
            out.push(self.step(format!("{who} fainted!"), None, 1.0));
            match self.foe.next_alive() {
                Some(i) if self.kind == Kind::Trainer => {
                    self.foe.active = i;
                    let name = self.foe.cur().display();
                    out.push(self.step(format!("{} sent out {name}!", self.trainer), None, 1.0));
                }
                _ => {
                    self.outcome = Some(Outcome::Won);
                    let text = match self.kind {
                        Kind::Wild => "You won!".to_string(),
                        Kind::Trainer => format!("You defeated {}!", self.trainer),
                    };
                    out.push(self.step(text, None, 1.0));
                }
            }
        }
    }

    /// Play one turn. `rng` gives numbers in 0..1.
    pub fn turn(&mut self, action: Action, rng: &mut dyn FnMut() -> f32) -> Vec<Step> {
        let mut out = Vec::new();
        if self.outcome.is_some() {
            return out;
        }
        if self.must_switch {
            if let Action::Switch(i) = action {
                if i < self.me.mons.len() && !self.me.mons[i].fainted() {
                    self.me.active = i;
                    self.must_switch = false;
                    out.push(self.step(format!("Go, {}!", self.me.cur().display()), None, 1.0));
                }
            }
            return out;
        }
        self.turns += 1;
        match action {
            Action::Run => {
                if self.kind == Kind::Wild {
                    self.outcome = Some(Outcome::Ran);
                    out.push(self.step("Got away safely!".into(), None, 1.0));
                } else {
                    // From a trainer battle you can only give up (a loss).
                    self.outcome = Some(Outcome::Lost);
                    out.push(self.step("You gave up the battle".into(), None, 1.0));
                }
                return out;
            }
            Action::Switch(i) => {
                if i == self.me.active || i >= self.me.mons.len() || self.me.mons[i].fainted() {
                    return out;
                }
                let old = self.me.cur().display();
                self.me.active = i;
                out.push(self.step(format!("Come back, {old}! Go, {}!", self.me.cur().display()), None, 1.0));
                let s = self.foe_move(rng);
                self.attack(true, s, rng, &mut out);
            }
            Action::Ball(name, help) => {
                if self.kind != Kind::Wild {
                    return out;
                }
                let f = self.foe.cur();
                let (m, h) = (f.max_hp() as f32, f.hp as f32);
                let chance = if help >= 100.0 { 1.0 } else { ((3.0 * m - 2.0 * h) / (3.0 * m) * self.catch_rate * help).clamp(0.02, 1.0) };
                out.push(self.step(format!("You threw a {name}…"), None, 1.0));
                if rng() < chance {
                    self.outcome = Some(Outcome::Caught);
                    out.push(self.step(format!("Gotcha! {} was caught!", f.display()), None, 1.0));
                    return out;
                }
                out.push(self.step("Oh no! It broke free!".into(), None, 1.0));
                let s = self.foe_move(rng);
                self.attack(true, s, rng, &mut out);
            }
            Action::Move(i) => {
                let i = i.min(self.me.cur().moves.len().saturating_sub(1));
                let f = self.foe_move(rng);
                let prio = |side: &Side, slot: usize| side.cur().moves.get(slot).map_or(0, |m| MOVES[*m as usize].prio);
                let (pm, pf) = (prio(&self.me, i), prio(&self.foe, f));
                let (sm, sf) = (self.me.cur().stats[5], self.foe.cur().stats[5]);
                let me_first = pm > pf || (pm == pf && (sm > sf || (sm == sf && rng() < 0.5)));
                let foe_before = self.foe.active;
                if me_first {
                    self.attack(false, i, rng, &mut out);
                    // A trainer's fresh Pokémon doesn't get a move the turn it comes out.
                    if self.outcome.is_none() && self.foe.active == foe_before {
                        self.attack(true, f, rng, &mut out);
                    }
                } else {
                    self.attack(true, f, rng, &mut out);
                    if self.outcome.is_none() && !self.must_switch {
                        self.attack(false, i, rng, &mut out);
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(v: Vec<f32>) -> impl FnMut() -> f32 {
        let mut i = 0;
        move || {
            i += 1;
            v[(i - 1) % v.len()]
        }
    }

    #[test]
    fn stats_like_the_games() {
        // Pikachu: base HP 35, Speed 90, at Lv 50 with perfect IVs.
        let p = Mon::new("pikachu", 50);
        assert_eq!(p.max_hp(), 110);
        assert_eq!(p.stats[5], 110);
        assert!(!p.moves.is_empty() && p.moves.len() <= 4);
    }

    #[test]
    fn everyone_has_moves_at_every_level() {
        for s in crate::national::NATIONAL.iter().chain(crate::national::FORMS.iter()) {
            for lv in [1, 5, 30, 100] {
                assert!(!moves_at(s.name, lv).is_empty(), "{} at {lv}", s.name);
            }
        }
    }

    #[test]
    fn type_matchups_matter() {
        let fire = Mon::new("charizard", 50);
        let grass = Mon::new("venusaur", 50);
        let water = Mon::new("blastoise", 50);
        let flame = MOVES.iter().find(|m| m.name == "Flamethrower").unwrap();
        let (a, ea) = damage(&fire, &grass, flame, false, 1.0);
        let (b, eb) = damage(&fire, &water, flame, false, 1.0);
        assert_eq!(ea, 2.0);
        assert_eq!(eb, 0.5);
        assert!(a > b * 3);
        let quake = MOVES.iter().find(|m| m.name == "Earthquake").unwrap();
        assert_eq!(damage(&water, &fire, quake, false, 1.0).1, 0.0);
    }

    #[test]
    fn a_battle_ends() {
        let me = vec![Mon::new("infernape", 55), Mon::new("garchomp", 55)];
        let foe = vec![Mon::new("snorlax", 50), Mon::new("gengar", 50)];
        let mut b = Battle::new(Kind::Trainer, me, foe, "Hiker Dale");
        let mut rng = seq(vec![0.3, 0.7, 0.5, 0.1, 0.9]);
        assert_eq!(b.intro().len(), 2);
        for _ in 0..200 {
            if b.outcome.is_some() {
                break;
            }
            let a = if b.must_switch { Action::Switch(b.me.next_alive().unwrap()) } else { Action::Move(0) };
            b.turn(a, &mut rng);
        }
        assert!(matches!(b.outcome, Some(Outcome::Won) | Some(Outcome::Lost)));
        assert_eq!(b.turn(Action::Move(0), &mut rng).len(), 0);
    }

    #[test]
    fn balls_and_running() {
        let mut b = Battle::new(Kind::Wild, vec![Mon::new("pikachu", 30)], vec![Mon::new("rattata", 10)], "");
        let mut rng = seq(vec![0.5]);
        b.turn(Action::Ball("Master Ball", 255.0), &mut rng);
        assert_eq!(b.outcome, Some(Outcome::Caught));
        let mut t = Battle::new(Kind::Trainer, vec![Mon::new("pikachu", 30)], vec![Mon::new("rattata", 10)], "Youngster");
        t.turn(Action::Ball("Poké Ball", 1.0), &mut rng);
        assert_eq!(t.outcome, None);
        t.turn(Action::Run, &mut rng);
        assert_eq!(t.outcome, Some(Outcome::Lost));
        let mut w = Battle::new(Kind::Wild, vec![Mon::new("pikachu", 30)], vec![Mon::new("rattata", 10)], "");
        w.turn(Action::Run, &mut rng);
        assert_eq!(w.outcome, Some(Outcome::Ran));
    }
}
