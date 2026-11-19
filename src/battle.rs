//! Turn-based battles: your team against a wild Pokémon or a trainer's team. Real base stats
//! and moves (`movedata`), the usual damage rules (level, Attack against Defense, same-type
//! bonus, type matchups, critical hits, a little luck), faster Pokémon first, switching, and
//! Poké Balls for wild ones. Stats use perfect IVs, no EVs and a neutral nature. Each move
//! has PP (out of PP everywhere, a Pokémon Struggles), and moves can raise or lower stats by
//! stages, −6 to +6, like the games: Swords Dance, Growl, Close Combat's drop, Psychic's
//! 10% chance to lower Sp. Def… Stages reset when a Pokémon switches out.

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
    /// PP left for each move.
    pub pp: Vec<u8>,
    /// Stat stages, −6..6: Attack, Defense, Sp. Atk, Sp. Def, Speed, accuracy, evasion.
    pub boosts: [i8; 7],
}

/// The stages' names, in `boosts` order.
pub const STAT_NAMES: [&str; 7] = ["Attack", "Defense", "Sp. Atk", "Sp. Def", "Speed", "accuracy", "evasiveness"];
/// Short names, for the info boxes.
pub const STAT_SHORT: [&str; 7] = ["Atk", "Def", "SpA", "SpD", "Spe", "Acc", "Eva"];

/// The move slot that means Struggle (every move out of PP).
pub const STRUGGLE: usize = usize::MAX;

/// What a stat stage does to a stat: +1 is ×1.5, +2 ×2… −1 is ×⅔, −2 ×½…
pub fn stage_mult(s: i8) -> f32 {
    if s >= 0 {
        (2 + s) as f32 / 2.0
    } else {
        2.0 / (2 - s) as f32
    }
}

/// The same for accuracy against evasion (+1 is ×4⁄3…).
fn acc_mult(s: i8) -> f32 {
    let s = s.clamp(-6, 6);
    if s >= 0 {
        (3 + s) as f32 / 3.0
    } else {
        3.0 / (3 - s) as f32
    }
}

pub fn struggle() -> u16 {
    MOVES.iter().position(|m| m.name == "Struggle").unwrap_or(0) as u16
}

/// How useful a stat move is to a Pokémon with these base stats (0 or less: not at all):
/// raising its better attacking stat or its Speed counts most, lowering the target's less,
/// lowering its own against it.
fn status_value(mv: &Move, base: &[u8; 6]) -> f32 {
    let physical = base[1] >= base[3];
    mv.stats
        .iter()
        .map(|&(st, ch)| {
            if !mv.on_user {
                return -(ch as f32) * if st == 5 { 0.5 } else { 0.8 };
            }
            let w = match st {
                0 => if physical { 2.0 } else { 0.2 },
                2 => if physical { 0.2 } else { 2.0 },
                4 => 1.4,
                1 | 3 => 0.9,
                5 => 0.6,
                _ => 1.0,
            };
            w * ch as f32
        })
        .sum()
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
/// strongest (its own types count half again), preferring different types, plus its most
/// useful move for raising or lowering stats if it has learned one.
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
    let status = known
        .iter()
        .copied()
        .filter(|m| MOVES[*m as usize].power == 0)
        .map(|m| (m, status_value(&MOVES[m as usize], &row.1)))
        .filter(|(_, v)| *v > 0.0)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(m, _)| m);
    known.retain(|m| MOVES[*m as usize].power > 0);
    if known.is_empty() {
        known.push(tackle());
    }
    let room = if status.is_some() { 3 } else { 4 };
    let score = |m: u16| {
        let mv = &MOVES[m as usize];
        let stab = if types.contains(&(mv.t as usize)) { 1.5 } else { 1.0 };
        let acc = if mv.acc == 0 { 1.0 } else { mv.acc as f32 / 100.0 };
        mv.power as f32 * stab * acc
    };
    let mut picked: Vec<u16> = Vec::new();
    while picked.len() < room {
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
    picked.extend(status);
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
        let moves = moves_at(name, level);
        let pp = moves.iter().map(|m| MOVES[*m as usize].pp).collect();
        Mon { name: name.to_string(), level, types: quest::types_of(name), stats, hp: stats[0], moves, pp, boosts: [0; 7] }
    }

    /// Attack (1) … Speed (5) with its stage.
    pub fn eff(&self, i: usize) -> f32 {
        self.stats[i] as f32 * stage_mult(self.boosts[i - 1])
    }

    /// Every move is out of PP.
    pub fn out_of_pp(&self) -> bool {
        self.pp.iter().all(|p| *p == 0)
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
/// Stat stages count; a critical hit ignores the attacker's drops and the target's rises.
/// Struggle has no type.
pub fn damage(att: &Mon, def: &Mon, mv: &Move, crit: bool, roll: f32) -> (u32, f32) {
    let typeless = mv.name == "Struggle";
    let eff = if typeless { 1.0 } else { quest::effect(mv.t as usize, &def.types) };
    if eff <= 0.0 || mv.power == 0 {
        return (0, eff);
    }
    let (ai, di) = if mv.special { (3, 4) } else { (1, 2) };
    let (ab, db) = (att.boosts[ai - 1], def.boosts[di - 1]);
    let (ab, db) = if crit { (ab.max(0), db.min(0)) } else { (ab, db) };
    let a = att.stats[ai] as f32 * stage_mult(ab);
    let d = (def.stats[di] as f32 * stage_mult(db)).max(1.0);
    let base = ((2.0 * att.level as f32 / 5.0 + 2.0) * mv.power as f32 * a / d) / 50.0 + 2.0;
    let stab = if !typeless && att.types.contains(&(mv.t as usize)) { 1.5 } else { 1.0 };
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
    /// A stat changed: (on the foe's side, it rose), for arrows over that Pokémon.
    pub stat: Option<(bool, bool)>,
    /// Both active Pokémon's stat stages right after it (yours, the foe's).
    pub boosts: ([i8; 7], [i8; 7]),
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
    /// What the trainer says when the battle starts and when they lose ("" for nothing).
    pub lines: (String, String),
    /// How the trainer picks moves: 0 a little at random, 1 always the hardest hit, 2 the
    /// smartest (a sure knockout first, powering up only when it's safe).
    pub ai: u8,
}

impl Battle {
    pub fn new(kind: Kind, me: Vec<Mon>, foe: Vec<Mon>, trainer: &str) -> Battle {
        Battle { kind, me: Side::new(me), foe: Side::new(foe), trainer: trainer.to_string(), catch_rate: 0.5, outcome: None, must_switch: false, turns: 0, lines: (String::new(), String::new()), ai: 0 }
    }

    fn step(&self, text: String, hit: Option<bool>, eff: f32) -> Step {
        Step { text, me: (self.me.active, self.me.cur().hp), foe: (self.foe.active, self.foe.cur().hp), hit, eff, mv: None, stat: None, boosts: (self.me.cur().boosts, self.foe.cur().boosts) }
    }

    fn side_mut(&mut self, foe: bool) -> &mut Side {
        if foe {
            &mut self.foe
        } else {
            &mut self.me
        }
    }

    /// How a side's active Pokémon is named in a line ("The foe's Gengar").
    fn who(&self, foe: bool) -> String {
        if !foe {
            return self.me.cur().display();
        }
        match self.kind {
            Kind::Wild => format!("The wild {}", self.foe.cur().display()),
            Kind::Trainer => format!("The foe's {}", self.foe.cur().display()),
        }
    }

    /// Raise or lower a side's active Pokémon's stats, with a line for each change ("…'s
    /// Attack rose sharply!"); `loud` also says when a stat can't go any further.
    fn change_stats(&mut self, foe: bool, changes: &[(u8, i8)], loud: bool, out: &mut Vec<Step>) {
        let who = self.who(foe);
        // Stats that change by the same amount share a line.
        let mut lines: Vec<(i8, Vec<&str>)> = Vec::new();
        for &(st, ch) in changes {
            let st = st as usize;
            if st >= 7 {
                continue;
            }
            let m = self.side_mut(foe).cur_mut();
            let old = m.boosts[st];
            let new = (old + ch).clamp(-6, 6);
            m.boosts[st] = new;
            let said = if new == old { 0 } else { new - old };
            if said == 0 && !loud {
                continue;
            }
            let key = if said == 0 { if ch > 0 { 100 } else { -100 } } else { said.clamp(-3, 3) };
            match lines.iter_mut().find(|l| l.0 == key) {
                Some(l) => l.1.push(STAT_NAMES[st]),
                None => lines.push((key, vec![STAT_NAMES[st]])),
            }
        }
        for (key, names) in lines {
            let list = match names.len() {
                1 => names[0].to_string(),
                n => format!("{} and {}", names[..n - 1].join(", "), names[n - 1]),
            };
            let how = match key {
                100 => "won't go any higher!",
                -100 => "won't go any lower!",
                1 => "rose!",
                2 => "rose sharply!",
                3 => "rose drastically!",
                -1 => "fell!",
                -2 => "harshly fell!",
                _ => "severely fell!",
            };
            let mut s = self.step(format!("{who}'s {list} {how}"), None, 1.0);
            if key.abs() < 100 {
                s.stat = Some((foe, key > 0));
            }
            out.push(s);
        }
    }

    /// The opening lines.
    pub fn intro(&self) -> Vec<Step> {
        let mut out = Vec::new();
        match self.kind {
            Kind::Wild => out.push(self.step(format!("A wild {} appeared!", self.foe.cur().display()), None, 1.0)),
            Kind::Trainer if !self.lines.0.is_empty() => {
                out.push(self.step(format!("{}: “{}”", self.trainer, self.lines.0), None, 1.0));
                out.push(self.step(format!("{} sent out {}!", self.trainer, self.foe.cur().display()), None, 1.0));
            }
            Kind::Trainer => out.push(self.step(format!("{} wants to battle! They sent out {}!", self.trainer, self.foe.cur().display()), None, 1.0)),
        }
        out.push(self.step(format!("Go, {}!", self.me.cur().display()), None, 1.0));
        out
    }

    /// The foe picks: wild Pokémon at random, trainers the move that should hurt most, now
    /// and then powering up first (or lowering your stats). Only moves with PP left.
    fn foe_move(&self, rng: &mut dyn FnMut() -> f32) -> usize {
        let f = self.foe.cur();
        let usable: Vec<usize> = (0..f.moves.len()).filter(|i| f.pp.get(*i).is_some_and(|p| *p > 0)).collect();
        if usable.is_empty() {
            return STRUGGLE;
        }
        if self.kind == Kind::Wild {
            return usable[((rng() * usable.len() as f32) as usize).min(usable.len() - 1)];
        }
        let me = self.me.cur();
        if self.ai > 0 {
            return self.smart_move(&usable, rng);
        }
        let healthy = f.hp * 10 >= f.max_hp() * 6;
        for &i in &usable {
            let mv = &MOVES[f.moves[i] as usize];
            if mv.power > 0 {
                continue;
            }
            let worth = if mv.on_user {
                healthy && mv.stats.iter().all(|(st, ch)| *ch < 0 || f.boosts[*st as usize] < 2) && rng() < 0.35
            } else {
                mv.stats.iter().all(|(st, _)| me.boosts[*st as usize] > -2) && rng() < 0.15
            };
            if worth {
                return i;
            }
        }
        usable
            .iter()
            .map(|&i| {
                let mv = &MOVES[f.moves[i] as usize];
                let (d, _) = damage(f, me, mv, false, 0.92);
                let acc = if mv.acc == 0 { 1.0 } else { mv.acc as f32 / 100.0 };
                (i, d as f32 * acc * (0.8 + 0.4 * rng()))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(usable[0], |b| b.0)
    }

    /// Hard and Max: no guessing. Hard always picks the hardest hit it can land; Max takes a
    /// sure knockout when there is one (the most accurate, a priority move if it's slower),
    /// and only powers up when it's healthy and can't hit hard yet.
    fn smart_move(&self, usable: &[usize], rng: &mut dyn FnMut() -> f32) -> usize {
        let (f, me) = (self.foe.cur(), self.me.cur());
        let hits: Vec<(usize, f32, f32, i8)> = usable
            .iter()
            .map(|&i| {
                let mv = &MOVES[f.moves[i] as usize];
                let (d, _) = damage(f, me, mv, false, 0.92);
                let acc = if mv.acc == 0 { 1.0 } else { (mv.acc as f32 * acc_mult(f.boosts[5] - me.boosts[6]) / 100.0).min(1.0) };
                (i, d as f32, acc, mv.prio)
            })
            .collect();
        let expected = |h: &(usize, f32, f32, i8)| h.1.min(me.hp as f32) * h.2;
        let best = hits.iter().filter(|h| h.1 > 0.0).max_by(|a, b| expected(a).total_cmp(&expected(b)));
        if self.ai >= 2 {
            let slower = f.eff(5) < me.eff(5);
            let ko = hits
                .iter()
                .filter(|h| h.1 >= me.hp as f32)
                .max_by(|a, b| {
                    let s = |h: &(usize, f32, f32, i8)| h.2 + if slower && h.3 > 0 { 1.0 } else { 0.0 };
                    s(a).total_cmp(&s(b))
                });
            if let Some(k) = ko {
                return k.0;
            }
            // Can't knock it out: power up if it's safe and the hits are weak.
            let healthy = f.hp * 10 >= f.max_hp() * 7;
            let weak = best.is_none_or(|b| b.1 * 2.0 < me.hp as f32);
            if healthy && weak {
                for &i in usable {
                    let mv = &MOVES[f.moves[i] as usize];
                    if mv.power == 0 && mv.on_user && mv.stats.iter().any(|(_, ch)| *ch > 0) && mv.stats.iter().all(|(st, ch)| *ch < 0 || f.boosts[*st as usize] < 4) && rng() < 0.7 {
                        return i;
                    }
                }
            }
        }
        match best {
            Some(b) => b.0,
            // Nothing that hurts: any stat move, else the first.
            None => usable.iter().copied().find(|&i| MOVES[f.moves[i] as usize].power == 0).unwrap_or(usable[0]),
        }
    }

    /// `foe_side` attacks with move slot `slot`; returns the lines. Handles fainting.
    fn attack(&mut self, foe_side: bool, slot: usize, rng: &mut dyn FnMut() -> f32, out: &mut Vec<Step>) {
        let (att, def) = if foe_side { (self.foe.cur().clone(), self.me.cur().clone()) } else { (self.me.cur().clone(), self.foe.cur().clone()) };
        if att.fainted() || def.fainted() {
            return;
        }
        let m = if slot == STRUGGLE {
            struggle()
        } else {
            let Some(&m) = att.moves.get(slot) else { return };
            let side = self.side_mut(foe_side).cur_mut();
            if let Some(p) = side.pp.get_mut(slot) {
                *p = p.saturating_sub(1);
            }
            m
        };
        let mv = &MOVES[m as usize];
        let who = self.who(foe_side);
        let acc = mv.acc as f32 * acc_mult(att.boosts[5] - def.boosts[6]);
        if mv.acc > 0 && rng() * 100.0 >= acc {
            out.push(self.step(format!("{who} used {}, but it missed!", mv.name), None, 1.0));
            return;
        }
        if mv.power == 0 {
            // A move that only raises or lowers stats.
            let mut s = self.step(format!("{who} used {}!", mv.name), Some(!foe_side), 1.0);
            s.mv = Some(m);
            out.push(s);
            let on = if mv.on_user { foe_side } else { !foe_side };
            self.change_stats(on, mv.stats, true, out);
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
        // Its side effect on stats: the target's (if it's still standing) or the user's own.
        if !mv.stats.is_empty() && (mv.on_user || !fainted) && rng() * 100.0 < mv.chance as f32 {
            let on = if mv.on_user { foe_side } else { !foe_side };
            self.change_stats(on, mv.stats, false, out);
        }
        if slot == STRUGGLE {
            // Struggling hurts the user too (never down to nothing here).
            let a = self.side_mut(foe_side).cur_mut();
            a.hp = a.hp.saturating_sub((a.max_hp() / 4).max(1)).max(1);
            out.push(self.step(format!("{who} is hit with recoil!"), None, 1.0));
        }
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
                    if self.kind == Kind::Trainer && !self.lines.1.is_empty() {
                        out.push(self.step(format!("{}: “{}”", self.trainer, self.lines.1), None, 1.0));
                    }
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
                    self.me.cur_mut().boosts = [0; 7];
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
                self.me.cur_mut().boosts = [0; 7];
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
                let me = self.me.cur();
                let i = if me.out_of_pp() {
                    STRUGGLE
                } else {
                    let i = i.min(me.moves.len().saturating_sub(1));
                    if me.pp.get(i).is_some_and(|p| *p == 0) {
                        // No PP left for that one: pick another.
                        self.turns -= 1;
                        return out;
                    }
                    i
                };
                if i == STRUGGLE {
                    out.push(self.step(format!("{} has no moves left!", self.me.cur().display()), None, 1.0));
                }
                let f = self.foe_move(rng);
                let prio = |side: &Side, slot: usize| side.cur().moves.get(slot).map_or(0, |m| MOVES[*m as usize].prio);
                let (pm, pf) = (prio(&self.me, i), prio(&self.foe, f));
                let (sm, sf) = (self.me.cur().eff(5), self.foe.cur().eff(5));
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

    fn by_name(n: &str) -> u16 {
        MOVES.iter().position(|m| m.name == n).unwrap() as u16
    }

    #[test]
    fn pp_runs_out_then_struggle() {
        let mut me = Mon::new("snorlax", 60);
        me.moves = vec![by_name("Tackle")];
        me.pp = vec![2];
        let mut b = Battle::new(Kind::Wild, vec![me], vec![Mon::new("blissey", 70)], "");
        let mut rng = seq(vec![0.5]);
        b.turn(Action::Move(0), &mut rng);
        b.turn(Action::Move(0), &mut rng);
        assert_eq!(b.me.cur().pp[0], 0);
        let lines = b.turn(Action::Move(0), &mut rng);
        assert!(lines.iter().any(|s| s.text.contains("Struggle")), "{lines:?}");
        assert!(lines.iter().any(|s| s.text.contains("recoil")));
    }

    #[test]
    fn stats_rise_and_fall() {
        let mut me = Mon::new("garchomp", 60);
        me.moves = vec![by_name("Swords Dance"), by_name("Close Combat")];
        me.pp = vec![20, 5];
        let mut foe = Mon::new("snorlax", 60);
        foe.moves = vec![by_name("Growl")];
        foe.pp = vec![40];
        let mut b = Battle::new(Kind::Wild, vec![me], vec![foe], "");
        let mut rng = seq(vec![0.5]);
        let lines = b.turn(Action::Move(0), &mut rng);
        assert!(lines.iter().any(|s| s.text == "Garchomp's Attack rose sharply!"), "{lines:?}");
        assert!(lines.iter().any(|s| s.text == "Garchomp's Attack fell!"), "{lines:?}");
        assert_eq!(b.me.cur().boosts[0], 1);
        b.turn(Action::Move(1), &mut rng);
        assert_eq!(b.me.cur().boosts[1], -1);
        assert_eq!(b.me.cur().boosts[3], -1);
        // Boosted attacks hit harder; stages stop at +6.
        let mut a = Mon::new("garchomp", 60);
        let d = Mon::new("snorlax", 60);
        let quake = &MOVES[by_name("Earthquake") as usize];
        let plain = damage(&a, &d, quake, false, 1.0).0;
        a.boosts[0] = 2;
        assert!(damage(&a, &d, quake, false, 1.0).0 > plain * 19 / 10);
        assert_eq!(stage_mult(6), 4.0);
        assert_eq!(stage_mult(-2), 0.5);
    }

    #[test]
    fn some_learn_a_stat_move() {
        let with = crate::national::NATIONAL.iter().filter(|s| moves_at(s.name, 60).iter().any(|m| MOVES[*m as usize].power == 0)).count();
        assert!(with > 200, "{with}");
        for s in crate::national::NATIONAL.iter() {
            let m = moves_at(s.name, 60);
            assert!(m.iter().any(|x| MOVES[*x as usize].power > 0), "{} has no attack", s.name);
        }
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
