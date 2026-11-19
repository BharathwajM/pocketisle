//! The Battle tab: turn-based battles (wild visitors, raid bosses, travelling trainers, gyms,
//! the Elite Four and champions), the league of every region, and picking your team.

use super::*;
use crate::battle::{Action, Battle, Kind, Mon, Outcome, Step};
use crate::league;
use crate::movedata::MOVES;

/// How long each line of a battle stays before the next.
const STEP_SECS: f32 = 1.25;
/// The tab's height (below the tab bar): the league, and a battle.
pub(super) const BATTLE_H: f32 = 346.0;
pub(super) const FIGHT_H: f32 = 300.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum BtBtn {
    Move(u8),
    Switch(u8),
    SwitchMenu,
    Back,
    Ball,
    Run,
    Next,
    Continue,
    Region(i32),
    Stage(u8),
    Traveller(u8),
    Slot(u8),
    Pick(&'static str),
    PickPage(i32),
    Remove,
    /// Show (and edit) the League party instead of the Battle party.
    Party(bool),
    /// A new random party.
    Shuffle,
    /// Normal, Hard or Max for league and travelling trainers.
    Difficulty(u8),
    /// Pointing at a stage you can't battle yet: just shows its team.
    Peek(u8),
}

/// How a battle with a wild Pokémon or raid boss went, for its card.
pub(super) struct WildReport {
    /// Your Pokémon that was out at the end (sprite name).
    pub star: String,
    pub turns: u32,
    pub fainted: usize,
    pub team: usize,
}

impl WildReport {
    /// "Scizor won it in 4 turns, no one fainted".
    pub fn won_line(&self) -> String {
        let turns = if self.turns == 1 { "1 turn".to_string() } else { format!("{} turns", self.turns) };
        let hurt = match self.fainted {
            0 => ", no one fainted".to_string(),
            1 => ", 1 of yours fainted".to_string(),
            n => format!(", {n} of yours fainted"),
        };
        format!("{} won it in {turns}{hurt}", pokemon::pretty(&self.star))
    }

    /// "It beat all 6 of yours in 9 turns".
    pub fn lost_line(&self) -> String {
        let who = if self.team == 1 { "your Pokémon".to_string() } else { format!("all {} of yours", self.team) };
        format!("It beat {who} in {} turns", self.turns.max(1))
    }
}

/// Who you're battling.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Foe {
    Wild,
    Raid,
    Traveller(usize),
    League(usize, usize),
}

#[derive(Clone)]
pub(super) struct BattleView {
    b: Battle,
    foe: Foe,
    queue: Vec<Step>,
    cur: Option<Step>,
    at: Instant,
    last: Instant,
    me_idx: usize,
    foe_idx: usize,
    me_hp: f32,
    foe_hp: f32,
    me_target: f32,
    foe_target: f32,
    hit: Option<(bool, Instant)>,
    switching: bool,
    /// The line's move hasn't landed yet (its HP drop waits for the animation).
    pending: bool,
}

impl BattleView {
    fn new(b: Battle, foe: Foe) -> BattleView {
        let queue = b.intro();
        let (me_hp, foe_hp) = (b.me.cur().hp as f32, b.foe.cur().hp as f32);
        BattleView {
            me_idx: b.me.active,
            foe_idx: b.foe.active,
            b,
            foe,
            queue,
            cur: None,
            at: Instant::now(),
            last: Instant::now(),
            me_hp,
            foe_hp,
            me_target: me_hp,
            foe_target: foe_hp,
            hit: None,
            switching: false,
            pending: false,
        }
    }

    /// The move on this line lands: HP drops and the target flinches.
    fn land(&mut self) {
        if !self.pending {
            return;
        }
        self.pending = false;
        if let Some(s) = &self.cur {
            self.me_target = s.me.1 as f32;
            self.foe_target = s.foe.1 as f32;
            let damaging = s.mv.is_none_or(|m| MOVES[m as usize].power > 0);
            if let Some(h) = s.hit.filter(|_| damaging) {
                self.hit = Some((h, Instant::now()));
            }
        }
    }

    /// How long this line stays up.
    fn line_secs(&self) -> f32 {
        if self.cur.as_ref().is_some_and(|c| c.mv.is_some()) { super::battle_fx::MOVE_SECS } else { STEP_SECS }
    }

    /// Show the next line.
    fn advance(&mut self) {
        self.land();
        if self.queue.is_empty() {
            return;
        }
        let s = self.queue.remove(0);
        if s.me.0 != self.me_idx {
            self.me_idx = s.me.0;
            self.me_hp = s.me.1 as f32;
        }
        if s.foe.0 != self.foe_idx {
            self.foe_idx = s.foe.0;
            self.foe_hp = s.foe.1 as f32;
        }
        let moving = s.mv.is_some();
        self.cur = Some(s);
        self.at = Instant::now();
        // A move: the animation plays first, then it lands.
        self.pending = true;
        if !moving {
            self.land();
        }
    }

    /// Lines still to show, or the last one only just up.
    fn busy(&self) -> bool {
        let wait = if self.cur.as_ref().is_some_and(|c| c.mv.is_some()) { 1.2 } else { 0.45 };
        !self.queue.is_empty() || self.pending || self.cur.is_none() || self.at.elapsed().as_secs_f32() < wait
    }

    /// Something on the field is moving: a line playing, an HP bar sliding, a flinch.
    fn moving(&self) -> bool {
        self.busy()
            || (self.me_hp - self.me_target).abs() > 0.01
            || (self.foe_hp - self.foe_target).abs() > 0.01
            || self.hit.is_some_and(|(_, at)| at.elapsed().as_secs_f32() < 0.5)
    }

    fn ready(&self) -> bool {
        !self.busy() && self.b.outcome.is_none()
    }
}

fn type_col(t: usize) -> Rgba {
    let (r, g, b) = pokemon::TYPES[t % 18].1;
    Rgba::rgb(r, g, b)
}

fn type_index(name: &str) -> usize {
    pokemon::TYPES.iter().position(|t| t.0 == name).unwrap_or(0)
}

fn hp_col(f: f32) -> Rgba {
    if f > 0.5 {
        Rgba::rgb(88, 208, 112)
    } else if f > 0.2 {
        Rgba::rgb(248, 200, 56)
    } else {
        Rgba::rgb(240, 80, 64)
    }
}

fn region_key(region: usize, gym: usize) -> String {
    format!("{} {}", crate::national::REGIONS[region % 9], gym + 1)
}

/// A badge's outline, a different shape for each gym: a hexagon, a diamond, a star, a
/// shield…
fn badge_shape(c: (f32, f32), r: f32, gym: usize) -> Vec<(f32, f32)> {
    use std::f32::consts::{PI, TAU};
    let ring = |n: usize, inner: f32, rot: f32| -> Vec<(f32, f32)> {
        let k = if inner > 0.0 { 2 * n } else { n };
        (0..k)
            .map(|i| {
                let rr = if inner > 0.0 && i % 2 == 1 { r * inner } else { r };
                let ang = rot + i as f32 * TAU / k as f32 - PI / 2.0;
                (c.0 + ang.cos() * rr, c.1 + ang.sin() * rr)
            })
            .collect()
    };
    match gym % 8 {
        0 => ring(6, 0.0, PI / 6.0),
        1 => ring(4, 0.0, 0.0),
        2 => ring(5, 0.5, 0.0),
        3 => vec![(c.0 - r * 0.85, c.1 - r * 0.8), (c.0 + r * 0.85, c.1 - r * 0.8), (c.0 + r * 0.85, c.1 + r * 0.1), (c.0, c.1 + r), (c.0 - r * 0.85, c.1 + r * 0.1)],
        4 => ring(8, 0.0, PI / 8.0),
        5 => ring(4, 0.45, 0.0),
        6 => ring(6, 0.6, 0.0),
        _ => ring(3, 0.0, 0.0),
    }
}

/// What a stat move does, for its button: "Atk ↑↑", "Foe Def ↓".
fn stat_summary(mv: &crate::movedata::Move) -> String {
    let parts: Vec<String> = mv
        .stats
        .iter()
        .take(2)
        .map(|(st, ch)| {
            let arrows = if *ch > 0 { "↑" } else { "↓" }.repeat(ch.unsigned_abs().min(3) as usize);
            format!("{} {arrows}", crate::battle::STAT_SHORT[*st as usize % 7])
        })
        .collect();
    let text = parts.join(" ");
    if mv.on_user || text.is_empty() {
        text
    } else {
        format!("Foe {text}")
    }
}

impl App {
    /// How long to wait for the next frame in a battle: none while it moves (animations
    /// need every frame), ~30 fps for the sprites while it waits for you.
    pub(super) fn battle_wait_ms(&self) -> u32 {
        if self.battle.as_ref().is_some_and(|v| v.moving()) { 0 } else { 33 }
    }

    // ---------- starting ----------

    /// The party shown on the tab (Battle or League).
    pub(super) fn battle_team(&self) -> Vec<String> {
        self.party(self.league_party)
    }

    /// A party: the six you picked, or (until you pick) your partner, your buddy and your
    /// strongest catches. The League party starts as a copy of the Battle party.
    pub(super) fn party(&self, league: bool) -> Vec<String> {
        let ok = |n: &str| pokemon::species(n).is_some();
        if league {
            let own: Vec<String> = self.dex.league_team.iter().filter(|n| ok(n)).cloned().collect();
            return if own.is_empty() { self.party(false) } else { own.into_iter().take(6).collect() };
        }
        let mut team: Vec<String> = self.dex.team.iter().filter(|n| ok(n)).cloned().collect();
        if team.is_empty() {
            team.push(self.mascot_sprite(PipMood::Calm));
            if !self.dex.buddy.is_empty() {
                team.push(self.dex.buddy.clone());
            }
            // Strongest catches next (not legendaries, Megas or Gigantamax: pick those yourself).
            let plain = |n: &str| pokemon::species(n).is_some_and(|s| s.form < 5) && !pokemon::is_legend(n);
            let mut caught: Vec<&String> = self.dex.caught.keys().filter(|n| ok(n) && plain(n)).collect();
            caught.sort_by_key(|n| std::cmp::Reverse(crate::battle::base_total(n)));
            let base = |s: &str| s.trim_end_matches("-shiny").to_string();
            let mut picked: Vec<String> = Vec::new();
            for n in team.iter().chain(caught) {
                if picked.len() < 6 && !picked.iter().any(|t| base(t) == base(n)) {
                    picked.push(n.clone());
                }
            }
            team = picked;
        }
        team.truncate(6);
        team
    }

    /// Six of your catches at random, one per family (not legendaries, Megas or Gigantamax:
    /// pick those yourself).
    fn random_party(&self) -> Vec<String> {
        let mut pool: Vec<&String> = self
            .dex
            .caught
            .keys()
            .filter(|n| pokemon::species(n).is_some_and(|s| s.form < 5 && !s.legend))
            .collect();
        pool.sort();
        let mut out: Vec<String> = Vec::new();
        while out.len() < 6 && !pool.is_empty() {
            let n = pool.swap_remove((rand01() * pool.len() as f32) as usize % pool.len());
            if !out.iter().any(|o| pokemon::family(o) == pokemon::family(n)) {
                out.push(n.clone());
            }
        }
        out
    }

    /// Give each party six random catches until it has some.
    fn fill_parties(&mut self) {
        let mut changed = false;
        if self.dex.team.iter().all(|n| pokemon::species(n).is_none()) {
            self.dex.team = self.random_party();
            changed = !self.dex.team.is_empty();
        }
        if self.dex.league_team.iter().all(|n| pokemon::species(n).is_none()) {
            self.dex.league_team = self.random_party();
            changed |= !self.dex.league_team.is_empty();
        }
        if changed {
            self.dex.save();
        }
    }

    /// The level a party is shown at: your team's level for the Battle party, the next
    /// league battle's for the League party.
    fn shown_level(&self, league: bool) -> u32 {
        if !league {
            return self.dex.level();
        }
        let region = self.league_region % 9;
        let next = (0..8).find(|g| !self.dex.badges.contains(&region_key(region, *g))).unwrap_or(league::ELITE);
        league::party_level(region, next)
    }

    /// A party as it battles at `lv`: each Pokémon at the stage that fits the level.
    fn party_mons(&self, league: bool, lv: u32) -> Vec<Mon> {
        self.party(league).iter().map(|n| Mon::new(&pokemon::stage_at(n, lv), lv)).collect()
    }

    /// The Battle party at your team's level.
    fn team_mons(&self) -> Vec<Mon> {
        self.party_mons(false, self.dex.level())
    }

    fn set_party(&mut self, team: Vec<String>) {
        if self.league_party {
            self.dex.league_team = team;
        } else {
            self.dex.team = team;
        }
        self.dex.save();
    }

    fn open_battle(&mut self, b: Battle, foe: Foe) {
        self.battle = Some(BattleView::new(b, foe));
        self.team_pick = None;
        self.tab = Tab::Battle;
        self.expanded = true;
        self.dirty = true;
    }

    /// A wild visitor (or the raid boss) that wants to battle: a real battle in the tab.
    pub(super) fn start_wild_battle(&mut self) {
        if self.battle.is_some() {
            // Already battling: back to it.
            self.tab = Tab::Battle;
            self.expanded = true;
            self.dirty = true;
            return;
        }
        self.fill_parties();
        let lv = self.dex.level();
        let Some(w) = self.wild.as_ref() else { return };
        let (raid, legend) = (w.raid, w.legend);
        let level = if raid {
            lv + 8
        } else if legend {
            lv + 3
        } else {
            lv.saturating_sub(6) + (rand01() * 6.0) as u32
        };
        let mut foe = Mon::new(&w.sprite(), level.clamp(2, 100));
        if raid {
            // A raid boss is sturdier than usual.
            foe.stats[0] *= 2;
            foe.hp = foe.stats[0];
        }
        let mut b = Battle::new(Kind::Wild, self.team_mons(), vec![foe], "");
        b.catch_rate = if raid { 0.08 } else if legend { 0.12 } else { 0.55 };
        self.open_battle(b, if raid { Foe::Raid } else { Foe::Wild });
    }

    /// A gym you've beaten in a region you're the champion of: its leader's full rematch team.
    fn is_rematch(&self, region: usize, stage: usize) -> bool {
        stage < league::ELITE
            && self.dex.badges.contains(&region_key(region, stage))
            && self.dex.champions.iter().any(|c| c == crate::national::REGIONS[region % 9])
    }

    fn start_league(&mut self, region: usize, stage: usize) {
        self.fill_parties();
        let rematch = self.is_rematch(region, stage);
        let lv = if rematch { league::rematch_party_level(region, stage) } else { league::party_level(region, stage) };
        let mut me: Vec<Mon> = self.party_mons(true, lv);
        if stage >= league::ELITE {
            match self.dex.league_run.as_ref() {
                Some(r) if r.region == region && r.stage == stage => {
                    if r.hp.len() == me.len() {
                        for (m, h) in me.iter_mut().zip(&r.hp) {
                            m.hp = (*h).min(m.max_hp());
                        }
                    }
                    if r.pp.len() == me.len() {
                        for (m, pp) in me.iter_mut().zip(&r.pp) {
                            if pp.len() == m.pp.len() {
                                for (a, b) in m.pp.iter_mut().zip(pp) {
                                    *a = (*b).min(*a);
                                }
                            }
                        }
                    }
                }
                _ => {
                    self.dex.league_run = Some(crate::dex::LeagueRun { region, stage: league::ELITE, hp: me.iter().map(|m| m.hp).collect(), pp: me.iter().map(|m| m.pp.clone()).collect() });
                    self.dex.save();
                }
            }
        }
        if me.iter().all(|m| m.fainted()) {
            return;
        }
        let diff = self.difficulty();
        let mut b = Battle::new(Kind::Trainer, me, league::stage_mons(region, stage, rematch, league::bonus(diff)), &league::trainer_name(region, stage));
        b.ai = diff;
        let (hi, bye) = league::lines(region, stage);
        b.lines = (hi.to_string(), bye.to_string());
        self.open_battle(b, Foe::League(region, stage));
    }

    fn today(&self) -> String {
        let now = calendar::local_now();
        format!("{:04}-{:02}-{:02}", now.date.y, now.date.m, now.date.d)
    }

    /// The difficulty for league and travelling trainers (0 Normal, 1 Hard, 2 Max).
    fn difficulty(&self) -> u8 {
        self.dex.league_difficulty.min(2)
    }

    /// Put balls from a win in the bag; returns them in words.
    fn give_prize(&mut self, win: league::Win) -> String {
        let balls = league::prize(win, self.difficulty());
        for (b, n) in &balls {
            self.dex.bag.add(*b, *n);
        }
        league::prize_words(&balls)
    }

    /// "Max · " before a win's details on Hard and Max.
    fn diff_tag(&self) -> String {
        match self.difficulty() {
            0 => String::new(),
            d => format!("{} · ", league::DIFFICULTY[d as usize]),
        }
    }

    pub(super) fn start_traveller(&mut self, i: usize) {
        self.fill_parties();
        let diff = self.difficulty();
        let list = league::travellers(&self.today(), self.dex.level(), league::bonus(diff));
        let Some(t) = list.get(i) else { return };
        let foe = t.team.iter().map(|(n, l)| Mon::new(n, *l)).collect();
        let mut b = Battle::new(Kind::Trainer, self.team_mons(), foe, &format!("{} {}", t.title, t.name));
        b.ai = diff;
        self.open_battle(b, Foe::Traveller(i));
    }

    // ---------- playing ----------

    fn battle_act(&mut self, a: Action) {
        let Some(mut v) = self.battle.take() else { return };
        if v.ready() || (v.b.must_switch && !v.busy()) {
            let mut rng = || rand01();
            let steps = v.b.turn(a, &mut rng);
            v.queue.extend(steps);
            v.switching = false;
            v.advance();
        }
        self.battle = Some(v);
        self.dirty = true;
    }

    fn throw_in_battle(&mut self) {
        if !self.battle.as_ref().is_some_and(|v| v.ready() && v.b.kind == Kind::Wild) {
            return;
        }
        let kind = self.dex.bag.take();
        self.dex.save();
        let help = match kind {
            1 => 1.5,
            2 => 2.0,
            3 => 255.0,
            _ => 1.0,
        };
        self.battle_ball = kind;
        self.battle_act(Action::Ball(crate::quest::BALL_NAMES[kind as usize], help));
    }

    /// The battle's over and you clicked Continue: rewards, cards, and back to the league.
    fn finish_real_battle(&mut self) {
        let Some(v) = self.battle.take() else { return };
        let Some(out) = v.b.outcome else {
            self.battle = Some(v);
            return;
        };
        let won = out == Outcome::Won;
        match v.foe {
            Foe::Wild | Foe::Raid => {
                if self.wild.is_none() {
                    if won {
                        self.gain_xp(40);
                    }
                    return;
                }
                match out {
                    Outcome::Caught => {
                        let ball = self.battle_ball;
                        if let Some(w) = self.wild.as_mut() {
                            w.ball = ball;
                            w.tries = 1;
                            w.quality = 0.0;
                            w.catch = Some((Instant::now(), true));
                        }
                        self.finish_catch();
                    }
                    Outcome::Won | Outcome::Lost => {
                        if let Some(w) = self.wild.as_mut() {
                            w.fight = Some((Instant::now(), won));
                        }
                        self.battle_report = Some(WildReport {
                            star: v.b.me.cur().name.clone(),
                            turns: v.b.turns,
                            fainted: v.b.me.mons.iter().filter(|m| m.fainted()).count(),
                            team: v.b.me.mons.len(),
                        });
                        self.finish_battle();
                    }
                    Outcome::Ran => {}
                }
            }
            Foe::Traveller(i) => {
                let who = v.b.trainer.clone();
                if won {
                    let today = self.today();
                    if self.dex.travel_day != today {
                        self.dex.travel_day = today;
                        self.dex.travel_beaten.clear();
                    }
                    self.dex.travel_beaten.push(i as u8);
                    self.dex.battles_won += 1;
                    let lv = v.b.foe.mons.iter().map(|m| m.level).max().unwrap_or(1);
                    let xp = league::scaled_xp(40 + lv, self.difficulty());
                    let prize = self.give_prize(league::Win::Traveller);
                    self.dex.save();
                    let tint = self.theme.good;
                    let body = format!("{}+{xp} XP · they gave you {prize}", self.diff_tag());
                    self.quick_card("battle", G_SWORDS, tint, "Battle", &format!("You beat {who}!"), &body, 5.0);
                    self.gain_xp(xp);
                    self.progress(|c| c.kind == "battle");
                } else {
                    self.dex.battles_lost += 1;
                    self.dex.save();
                    let tint = self.theme.bad;
                    self.quick_card("battle", G_SWORDS, tint, "Battle", &format!("{who} won"), "+10 XP for trying · heal up and try again", 4.0);
                    self.gain_xp(10);
                }
            }
            Foe::League(region, stage) => self.league_result(&v, region, stage, won),
        }
        self.dirty = true;
    }

    fn league_result(&mut self, v: &BattleView, region: usize, stage: usize, won: bool) {
        let rname = crate::national::REGIONS[region];
        let who = v.b.trainer.clone();
        if !won {
            self.dex.battles_lost += 1;
            let body = if stage >= league::ELITE {
                self.dex.league_run = None;
                "Your team blacked out · the Elite Four starts over · +10 XP".to_string()
            } else {
                "Train up or change your team, then try again · +10 XP".to_string()
            };
            self.dex.save();
            let tint = self.theme.bad;
            self.quick_card("league", G_SWORDS, tint, "Pokémon League", &format!("{who} won"), &body, 5.0);
            self.gain_xp(10);
            return;
        }
        self.dex.battles_won += 1;
        let diff = self.difficulty();
        let tag = self.diff_tag();
        let (title, body, xp) = if stage < league::ELITE {
            let key = region_key(region, stage);
            if self.is_rematch(region, stage) {
                let xp = league::scaled_xp(200, diff);
                let prize = self.give_prize(league::Win::FullTeam);
                (format!("You beat {who}'s full team!"), format!("{tag}A champion's rematch · {prize} · +{xp} XP"), xp)
            } else if self.dex.badges.contains(&key) {
                let xp = league::scaled_xp(80, diff);
                let prize = self.give_prize(league::Win::Again);
                (format!("You beat {who} again"), format!("{tag}A rematch · {prize} · +{xp} XP"), xp)
            } else {
                self.dex.badges.push(key);
                let have = (0..8).filter(|g| self.dex.badges.contains(&region_key(region, *g))).count();
                let prize = self.give_prize(league::Win::Badge { late: stage >= 4 });
                let xp = league::scaled_xp(150 + 25 * stage as u32, diff);
                let next = if have == 8 { "all 8 badges: the Elite Four awaits!".to_string() } else { format!("{have}/8 {rname} badges") };
                (format!("You earned the {}!", league::badge_name(league::stage_type(region, stage))), format!("{tag}{next} · {prize} · +{xp} XP"), xp)
            }
        } else if stage < league::CHAMPION {
            let hp: Vec<u32> = v.b.me.mons.iter().map(|m| m.hp).collect();
            let pp: Vec<Vec<u8>> = v.b.me.mons.iter().map(|m| m.pp.clone()).collect();
            self.dex.league_run = Some(crate::dex::LeagueRun { region, stage: stage + 1, hp, pp });
            let next = league::trainer_name(region, stage + 1);
            let xp = league::scaled_xp(250, diff);
            let prize = self.give_prize(league::Win::Elite);
            (format!("You beat {who}!"), format!("{tag}Next: {next}, HP and PP carry over · {prize} · +{xp} XP"), xp)
        } else {
            self.dex.league_run = None;
            if !self.dex.champions.iter().any(|c| c == rname) {
                self.dex.champions.push(rname.to_string());
            }
            if diff == 2 && !self.dex.max_champions.iter().any(|c| c == rname) {
                self.dex.max_champions.push(rname.to_string());
            }
            let xp = league::scaled_xp(1000, diff);
            let prize = self.give_prize(league::Win::Champion);
            let title = if diff == 2 { format!("{rname} Champion on Max!") } else { format!("You're the {rname} Champion!") };
            (title, format!("{tag}{prize} · +{xp} XP · {} of 9 regions", self.dex.champions.len()), xp)
        };
        self.dex.save();
        let tint = self.theme.good;
        self.quick_card("league", G_CHECK, tint, "Pokémon League", &title, &body, 7.0);
        self.gain_xp(xp);
        self.progress(|c| c.kind == "battle");
    }

    pub(super) fn bt_click(&mut self, b: BtBtn) {
        match b {
            BtBtn::Move(i) => self.battle_act(Action::Move(i as usize)),
            BtBtn::Switch(i) => self.battle_act(Action::Switch(i as usize)),
            BtBtn::SwitchMenu => {
                if let Some(v) = self.battle.as_mut().filter(|v| v.ready()) {
                    v.switching = true;
                }
            }
            BtBtn::Back => {
                if let Some(v) = self.battle.as_mut() {
                    v.switching = false;
                }
                self.team_pick = None;
            }
            BtBtn::Ball => self.throw_in_battle(),
            BtBtn::Run => self.battle_act(Action::Run),
            BtBtn::Next => {
                if let Some(v) = self.battle.as_mut() {
                    v.advance();
                }
            }
            BtBtn::Continue => self.finish_real_battle(),
            BtBtn::Region(d) => self.league_region = (self.league_region as i32 + d).rem_euclid(9) as usize,
            BtBtn::Stage(s) => self.start_league(self.league_region, s as usize),
            BtBtn::Traveller(i) => self.start_traveller(i as usize),
            BtBtn::Slot(i) => {
                self.team_pick = Some(i as usize);
                self.pick_page = 0;
            }
            BtBtn::Pick(n) => {
                if let Some(i) = self.team_pick.take() {
                    // Swap the slot's Pokémon for it (and drop any of the same family).
                    let mut team = self.battle_team();
                    if i < team.len() {
                        team.remove(i);
                    }
                    let fam = pokemon::family(n);
                    team.retain(|t| pokemon::family(t) != fam);
                    let i = i.min(team.len());
                    team.insert(i, n.to_string());
                    team.truncate(6);
                    self.set_party(team);
                }
            }
            BtBtn::Party(l) => self.league_party = l,
            // Fixed while an Elite Four run is going, so its prizes match how it was played.
            BtBtn::Difficulty(d) if self.dex.league_run.is_none() => {
                self.dex.league_difficulty = d.min(2);
                self.dex.save();
            }
            BtBtn::Difficulty(_) | BtBtn::Peek(_) => {}
            BtBtn::Shuffle => {
                let team = self.random_party();
                if !team.is_empty() {
                    self.set_party(team);
                }
            }
            BtBtn::PickPage(d) => self.pick_page = (self.pick_page as i32 + d).max(0) as usize,
            BtBtn::Remove => {
                if let Some(i) = self.team_pick.take() {
                    let mut team = self.battle_team();
                    if i < team.len() && team.len() > 1 {
                        team.remove(i);
                        self.set_party(team);
                    }
                }
            }
        }
        self.dirty = true;
    }

    /// The demo: play the battle out at once (always picking the first move), leaving the
    /// last lines to show.
    pub(super) fn demo_finish_battle(&mut self) {
        let Some(mut v) = self.battle.take() else { return };
        let mut rng = || rand01();
        let mut last = Vec::new();
        for _ in 0..200 {
            if v.b.outcome.is_some() {
                break;
            }
            let a = if v.b.must_switch { Action::Switch(v.b.me.next_alive().unwrap_or(0)) } else { Action::Move(0) };
            last = v.b.turn(a, &mut rng);
        }
        v.queue = last;
        v.switching = false;
        v.advance();
        self.battle = Some(v);
        self.dirty = true;
    }

    /// The demo: your Pokémon shows off these moves on the foe, one after another (no
    /// damage).
    pub(super) fn demo_moves(&mut self, names: &[&str]) {
        let Some(v) = self.battle.as_mut() else { return };
        for n in names {
            let Some(m) = MOVES.iter().position(|m| m.name == *n) else { continue };
            v.queue.push(Step {
                text: format!("{} used {n}!", v.b.me.cur().display()),
                me: (v.b.me.active, v.b.me.cur().hp),
                foe: (v.b.foe.active, v.b.foe.cur().hp),
                hit: Some(true),
                eff: 1.0,
                mv: Some(m as u16),
                stat: None,
                boosts: ([0; 7], [0; 7]),
            });
        }
        self.dirty = true;
    }

    // ---------- drawing ----------

    /// An animated sprite for the battle field (its own cache: bigger than the pill's).
    fn battle_frame(&mut self, name: &str, box_h: f32, max_w: f32) -> Option<(ID2D1Bitmap1, R, f32, f32)> {
        let stale = match self.battle_strips.get(name) {
            None => true,
            Some(Some(sp)) => (sp.box_h - box_h).abs() > 0.5 || sp.scale != self.scale,
            Some(None) => false,
        };
        if stale {
            let s = self.load_sprite_sized(name, box_h, max_w);
            self.battle_strips.insert(name.to_string(), s);
        }
        let sp = self.battle_strips.get(name)?.as_ref()?;
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let i = pokemon::frame_at(&sp.delays, ms);
        Some((sp.bmp.clone(), R::xywh(i as f32 * sp.fw, 0.0, sp.fw, sp.fh), sp.dw, sp.dh))
    }

    fn small_thumb(&mut self, name: &str, cx: f32, cy: f32, size: f32, alpha: f32) {
        if let Some((bmp, w, h)) = self.dex_thumb(name) {
            let k = (size / h).min(size / w);
            let d = R::xywh(cx - w * k / 2.0, cy - h * k / 2.0, w * k, h * k);
            self.r.sprite_flip(&bmp, d, R::xywh(0.0, 0.0, w, h), true, false, alpha);
        }
    }

    fn button(&mut self, r: R, label: &str, b: BtBtn, enabled: bool, strong: bool) {
        let th = self.theme.clone();
        let hot = enabled && self.hot == Some(Btn::Bt(b));
        let fill = if strong && enabled { th.accent.a(if hot { 1.0 } else { 0.85 }) } else if hot { th.hover } else { th.bg2.a(if enabled { 0.7 } else { 0.3 }) };
        self.r.fill_round(r, 10.0, fill);
        let col = if strong && enabled { th.bg } else if enabled { th.text } else { th.subtext.a(0.5) };
        self.r.text(label, r, 11.5, 600, col, Align::Center);
        if enabled {
            self.buttons.push((r, Btn::Bt(b)));
        }
    }

    pub(super) fn draw_battle_tab(&mut self, x0: f32, x1: f32, y: f32) {
        self.fill_parties();
        if self.battle.is_some() {
            self.draw_battle(x0, x1, y);
        } else if self.team_pick.is_some() {
            self.draw_team_picker(x0, x1, y);
        } else {
            self.draw_league(x0, x1, y);
        }
    }

    fn info_box(&mut self, r: R, m: &Mon, hp: f32, show_num: bool, dots: Option<&[Mon]>, boosts: &[i8; 7]) {
        let th = self.theme.clone();
        self.r.fill_round(r, 10.0, th.bg.a(0.72));
        self.r.text(&m.display(), R::new(r.x0 + 10.0, r.y0 + 4.0, r.x1 - 50.0, r.y0 + 22.0), 12.0, 700, th.text, Align::Left);
        self.r.text(&format!("Lv {}", m.level), R::new(r.x1 - 56.0, r.y0 + 4.0, r.x1 - 10.0, r.y0 + 22.0), 11.0, 600, th.subtext, Align::Right);
        let f = (hp / m.max_hp().max(1) as f32).clamp(0.0, 1.0);
        let bar = R::new(r.x0 + 30.0, r.y0 + 26.0, r.x1 - 10.0, r.y0 + 31.0);
        self.r.text("HP", R::new(r.x0 + 10.0, r.y0 + 21.0, r.x0 + 30.0, r.y0 + 36.0), 9.0, 800, th.subtext, Align::Left);
        self.r.fill_round(bar, 2.5, th.bg2);
        if f > 0.0 {
            self.r.fill_round(R::new(bar.x0, bar.y0, bar.x0 + (bar.w() * f).max(3.0), bar.y1), 2.5, hp_col(f));
        }
        if show_num {
            self.r.text(&format!("{} / {}", hp.round() as u32, m.max_hp()), R::new(r.x0 + 10.0, r.y0 + 33.0, r.x1 - 10.0, r.y1 - 2.0), 10.0, 600, th.subtext, Align::Right);
        }
        if let Some(team) = dots {
            for (i, t) in team.iter().enumerate() {
                let c = if t.fainted() { th.subtext.a(0.3) } else { Rgba::rgb(240, 80, 64) };
                self.r.circle(r.x0 + 14.0 + i as f32 * 11.0, r.y1 - 8.0, 3.5, c);
            }
        }
        // Raised and lowered stats: "Atk +2" in green, "Def −1" in red (yours on the left,
        // the foe's on the right).
        let chips: Vec<(String, bool)> = boosts
            .iter()
            .enumerate()
            .filter(|(_, b)| **b != 0)
            .map(|(i, b)| (format!("{} {}{}", crate::battle::STAT_SHORT[i], if *b > 0 { "+" } else { "−" }, b.abs()), *b > 0))
            .collect();
        let row = R::new(r.x0 + 10.0, r.y1 - 15.0, r.x1 - 10.0, r.y1 - 1.0);
        let mut x = if show_num { row.x0 } else { row.x1 };
        for (text, up) in chips {
            let w = self.r.measure(&text, 9.0, 700) + 8.0;
            let (cx0, cx1) = if show_num { (x, x + w) } else { (x - w, x) };
            let chip = R::new(cx0, row.y0, cx1, row.y1 - 1.0);
            let c = if up { th.good } else { th.bad };
            self.r.fill_round(chip, 4.0, c.a(0.2));
            self.r.text(&text, chip, 9.0, 700, c, Align::Center);
            x = if show_num { cx1 + 3.0 } else { cx0 - 3.0 };
        }
    }

    fn draw_battle(&mut self, x0: f32, x1: f32, y: f32) {
        let Some(mut v) = self.battle.take() else { return };
        let th = self.theme.clone();
        // Play the lines back.
        if v.cur.is_none() || (!v.queue.is_empty() && v.at.elapsed().as_secs_f32() > v.line_secs()) {
            v.advance();
        }
        if v.pending && v.at.elapsed().as_secs_f32() >= super::battle_fx::HIT {
            v.land();
        }
        let dt = v.last.elapsed().as_secs_f32().min(0.1);
        v.last = Instant::now();
        let k = (dt * 5.0).min(1.0);
        v.me_hp += (v.me_target - v.me_hp) * k;
        v.foe_hp += (v.foe_target - v.foe_hp) * k;
        if (v.me_hp - v.me_target).abs() < 0.5 {
            v.me_hp = v.me_target;
        }
        if (v.foe_hp - v.foe_target).abs() < 0.5 {
            v.foe_hp = v.foe_target;
        }
        let me = v.b.me.mons[v.me_idx].clone();
        let foe = v.b.foe.mons[v.foe_idx].clone();
        // The field.
        let field = R::new(x0, y, x1, y + 168.0);
        self.r.fill_round(field, 14.0, th.bg2.a(0.32));
        // The move being used: (move, the foe is the target, seconds in).
        let anim = v.cur.as_ref().and_then(|c| c.mv.zip(c.hit)).map(|(m, h)| (m, h, v.at.elapsed().as_secs_f32())).filter(|a| a.2 < super::battle_fx::MOVE_SECS);
        if let Some((mv, _, t)) = anim {
            self.draw_fx_bg(mv, t, field);
        }
        let (fx, fy) = (x1 - 118.0, y + 94.0);
        let (mx, my) = (x0 + 120.0, y + 160.0);
        self.r.fill_round(R::new(fx - 70.0, fy - 7.0, fx + 70.0, fy + 7.0), 7.0, th.bg2.a(0.7));
        self.r.fill_round(R::new(mx - 80.0, my - 8.0, mx + 80.0, my + 6.0), 7.0, th.bg2.a(0.7));
        let hit_k = |who: bool| v.hit.filter(|(h, _)| *h == who).map(|(_, at)| at.elapsed().as_secs_f32()).filter(|s| *s < 0.45);
        let (foe_c, me_c) = ((fx, fy - 36.0), (mx, my - 44.0));
        let shake_all = anim.map_or(0.0, |(m, _, t)| super::battle_fx::quake(super::battle_fx::fx_of(m), t));
        for (who, m, hp, cx, base, bh, mw, flip) in [(true, &foe, v.foe_hp, fx, fy + 3.0, 80.0, 112.0, false), (false, &me, v.me_hp, mx, my + 2.0, 92.0, 128.0, true)] {
            if hp <= 0.0 && m.hp == 0 {
                continue;
            }
            // A super effective hit shakes harder.
            let hard = if v.cur.as_ref().is_some_and(|c| c.eff >= 2.0) { 2.0 } else { 1.0 };
            let shake = hit_k(who).map_or(0.0, |s| (s * 60.0).sin() * 4.0 * hard * (1.0 - s / 0.45));
            let alpha = hit_k(who).map_or(1.0, |s| if ((s * 14.0) as i32) % 2 == 0 { 0.35 } else { 1.0 });
            // The attacker lunges (or jolts as it lets go); the target is knocked back.
            let (mut dx, mut dy) = (shake + shake_all, 0.0);
            if let Some((mv, foe_hit, t)) = anim {
                let (a, b) = if foe_hit { (me_c, foe_c) } else { (foe_c, me_c) };
                let (at, to) = super::battle_fx::offsets(mv, t, a, b);
                let o = if who == foe_hit { to } else { at };
                dx += o.0;
                dy += o.1;
            }
            if let Some((bmp, src, dw, dh)) = self.battle_frame(&m.name, bh, mw) {
                let d = R::xywh(cx - dw / 2.0 + dx, base - dh + dy, dw, dh);
                self.r.sprite_flip(&bmp, d, src, true, flip, alpha);
            } else {
                self.small_thumb(&m.name, cx + dx, base - bh / 2.0 + dy, bh, alpha);
            }
        }
        if let Some((mv, foe_hit, t)) = anim {
            let (a, b, size) = if foe_hit { (me_c, foe_c, 70.0) } else { (foe_c, me_c, 84.0) };
            self.draw_move_fx(mv, t, a, b, size);
        }
        if let Some((on_foe, up)) = v.cur.as_ref().and_then(|c| c.stat) {
            let t = v.at.elapsed().as_secs_f32();
            let (c, size) = if on_foe { (foe_c, 70.0) } else { (me_c, 84.0) };
            self.stat_arrows(c, up, t, (size / 70.0f32).clamp(0.7, 1.5));
        }
        let dots = (v.b.kind == Kind::Trainer).then(|| v.b.foe.mons.clone());
        let (my_b, foe_b) = v.cur.as_ref().map_or(([0; 7], [0; 7]), |c| c.boosts);
        let tall = dots.is_some() || foe_b.iter().any(|b| *b != 0);
        self.info_box(R::new(x0 + 10.0, y + 10.0, x0 + 232.0, y + 10.0 + if tall { 52.0 } else { 40.0 }), &foe, v.foe_hp, false, dots.as_deref(), &foe_b);
        self.info_box(R::new(x1 - 232.0, y + 104.0, x1 - 10.0, y + 154.0), &me, v.me_hp, true, None, &my_b);
        // The line.
        let msg = R::new(x0, y + 174.0, x1, y + 206.0);
        self.r.fill_round(msg, 10.0, th.bg2.a(0.55));
        let text = v.cur.as_ref().map(|s| s.text.clone()).unwrap_or_default();
        self.r.text(&text, R::new(msg.x0 + 12.0, msg.y0, msg.x1 - 28.0, msg.y1), 12.0, 600, th.text, Align::Left);
        if !v.queue.is_empty() {
            self.r.text("▾", R::new(msg.x1 - 26.0, msg.y0, msg.x1 - 8.0, msg.y1), 12.0, 700, th.accent, Align::Center);
            self.buttons.push((msg, Btn::Bt(BtBtn::Next)));
        }
        let cy = y + 212.0;
        let ready = v.ready();
        let busy = v.busy();
        let outcome = v.b.outcome;
        let must = v.b.must_switch;
        let switching = v.switching;
        let wild = v.b.kind == Kind::Wild;
        let b = v.b.clone();
        let foe_types = foe.types.clone();
        self.battle = Some(v);
        if let Some(out) = outcome {
            if busy {
                return;
            }
            let (line, col) = match out {
                Outcome::Won => ("You won the battle!", th.good),
                Outcome::Lost => ("You lost this one…", th.bad),
                Outcome::Caught => ("Caught!", th.good),
                Outcome::Ran => ("You got away", th.subtext),
            };
            self.r.text(line, R::new(x0 + 4.0, cy + 8.0, x1 - 150.0, cy + 40.0), 14.0, 700, col, Align::Left);
            self.button(R::new(x1 - 140.0, cy + 10.0, x1, cy + 44.0), "Continue", BtBtn::Continue, true, true);
            return;
        }
        if (must && !busy) || switching {
            // Your team: pick who goes in.
            let back = if must { 0.0 } else { 70.0 };
            let n = 6usize;
            let gap = 6.0;
            let tw = (x1 - x0 - back - gap * n as f32) / n as f32;
            for i in 0..n {
                let r = R::xywh(x0 + i as f32 * (tw + gap), cy, tw, 84.0);
                let Some(m) = b.me.mons.get(i) else {
                    self.r.fill_round(r, 10.0, th.bg2.a(0.2));
                    continue;
                };
                let can = !m.fainted() && i != b.me.active && (must || ready);
                let hot = can && self.hot == Some(Btn::Bt(BtBtn::Switch(i as u8)));
                self.r.fill_round(r, 10.0, if hot { th.hover } else { th.bg2.a(if can { 0.6 } else { 0.25 }) });
                self.small_thumb(&m.name, r.cx(), r.y0 + 24.0, 36.0, if m.fainted() { 0.3 } else { 1.0 });
                self.r.text(&m.display(), R::new(r.x0 + 2.0, r.y0 + 44.0, r.x1 - 2.0, r.y0 + 60.0), 10.0, 600, th.text, Align::Center);
                let f = m.hp as f32 / m.max_hp().max(1) as f32;
                let bar = R::new(r.x0 + 8.0, r.y0 + 66.0, r.x1 - 8.0, r.y0 + 70.0);
                self.r.fill_round(bar, 2.0, th.bg);
                if f > 0.0 {
                    self.r.fill_round(R::new(bar.x0, bar.y0, bar.x0 + bar.w() * f, bar.y1), 2.0, hp_col(f));
                }
                let note = if i == b.me.active { "in battle" } else if m.fainted() { "fainted" } else { "" };
                self.r.text(note, R::new(r.x0, r.y0 + 70.0, r.x1, r.y1), 9.0, 600, th.subtext, Align::Center);
                if can {
                    self.buttons.push((r, Btn::Bt(BtBtn::Switch(i as u8))));
                }
            }
            if !must {
                self.button(R::new(x1 - 64.0, cy, x1, cy + 84.0), "Back", BtBtn::Back, true, false);
            }
            return;
        }
        // Moves (2 × 2) and Switch / Ball / Run.
        let side = 114.0;
        let gap = 6.0;
        let mw = (x1 - x0 - side - gap * 2.0) / 2.0;
        let mh = 41.0;
        let mine = b.me.cur();
        for i in 0..4 {
            let r = R::xywh(x0 + (i % 2) as f32 * (mw + gap), cy + (i / 2) as f32 * (mh + gap), mw, mh);
            let Some(&m) = mine.moves.get(i) else {
                self.r.fill_round(r, 10.0, th.bg2.a(0.2));
                self.r.text("—", r, 11.0, 600, th.subtext.a(0.4), Align::Center);
                continue;
            };
            let mv = &MOVES[m as usize];
            let col = type_col(mv.t as usize);
            let (pp, max) = (mine.pp.get(i).copied().unwrap_or(0), mv.pp);
            // Out of PP everywhere: any button Struggles.
            let usable = ready && (pp > 0 || mine.out_of_pp());
            let hot = usable && self.hot == Some(Btn::Bt(BtBtn::Move(i as u8)));
            self.r.fill_round(r, 10.0, if hot { col.a(0.42) } else { col.a(if usable { 0.22 } else { 0.1 }) });
            self.r.fill_round(R::new(r.x0, r.y0, r.x0 + 5.0, r.y1), 2.5, col);
            self.r.text(mv.name, R::new(r.x0 + 12.0, r.y0 + 3.0, r.x1 - 56.0, r.y0 + 22.0), 12.0, 700, th.text.a(if usable { 1.0 } else { 0.5 }), Align::Left);
            let ppc = if pp == 0 { th.bad } else if pp * 4 <= max { Rgba::rgb(240, 170, 60) } else { th.subtext };
            self.r.text(&format!("PP {pp}/{max}"), R::new(r.x1 - 70.0, r.y0 + 4.0, r.x1 - 8.0, r.y0 + 21.0), 9.5, 700, ppc, Align::Right);
            let eff = if mv.power == 0 { 1.0 } else { crate::quest::effect(mv.t as usize, &foe_types) };
            let what = stat_summary(mv);
            let (word, wc) = if !what.is_empty() && mv.power == 0 {
                (what.as_str(), th.subtext)
            } else if eff >= 2.0 {
                ("Super effective", th.good)
            } else if eff <= 0.0 {
                ("No effect", th.bad)
            } else if eff < 1.0 {
                ("Not very effective", th.subtext)
            } else {
                ("", th.subtext)
            };
            let sub = if mv.power == 0 {
                format!("{} · Status", pokemon::TYPES[mv.t as usize % 18].0)
            } else {
                format!("{} · {}{}", pokemon::TYPES[mv.t as usize % 18].0, mv.power, if mv.special { " sp." } else { "" })
            };
            self.r.text(&sub, R::new(r.x0 + 12.0, r.y0 + 21.0, r.x1 - 6.0, r.y1 - 3.0), 10.0, 500, th.subtext, Align::Left);
            self.r.text(word, R::new(r.x0 + 12.0, r.y0 + 21.0, r.x1 - 8.0, r.y1 - 3.0), 10.0, 700, wc, Align::Right);
            if usable {
                self.buttons.push((r, Btn::Bt(BtBtn::Move(i as u8))));
            }
        }
        let bx = x1 - side;
        let bh = (mh * 2.0 + gap - 2.0 * 4.0) / 3.0;
        let others = b.me.mons.iter().enumerate().any(|(i, m)| i != b.me.active && !m.fainted());
        self.button(R::xywh(bx, cy, side, bh), "Switch", BtBtn::SwitchMenu, ready && others, false);
        let pick = self.dex.bag.pick;
        let ball = match self.dex.bag.count(pick) {
            None => "Poké Ball".to_string(),
            Some(n) => format!("{} ×{n}", crate::quest::BALL_NAMES[pick as usize].trim_end_matches(" Ball")),
        };
        self.button(R::xywh(bx, cy + bh + 4.0, side, bh), &ball, BtBtn::Ball, ready && wild, false);
        self.button(R::xywh(bx, cy + 2.0 * (bh + 4.0), side, bh), if wild { "Run" } else { "Give up" }, BtBtn::Run, ready, false);
    }

    fn draw_league(&mut self, x0: f32, x1: f32, mut y: f32) {
        let th = self.theme.clone();
        let region = self.league_region % 9;
        let rname = crate::national::REGIONS[region];
        let lv = self.dex.level();
        let have: Vec<bool> = (0..8).map(|g| self.dex.badges.contains(&region_key(region, g))).collect();
        let badges = have.iter().filter(|b| **b).count();
        let champ = self.dex.champions.iter().any(|c| c == rname);
        // Region and progress.
        for (r, d, glyph) in [(R::new(x0, y, x0 + 26.0, y + 24.0), -1, "‹"), (R::new(x0 + 150.0, y, x0 + 176.0, y + 24.0), 1, "›")] {
            let hot = self.hot == Some(Btn::Bt(BtBtn::Region(d)));
            self.r.fill_round(r, 10.0, if hot { th.hover } else { th.bg2.a(0.6) });
            self.r.text(glyph, r, 15.0, 700, th.text, Align::Center);
            self.buttons.push((r, Btn::Bt(BtBtn::Region(d))));
        }
        self.r.text(&format!("{rname} League"), R::new(x0 + 28.0, y, x0 + 148.0, y + 24.0), 12.5, 700, th.text, Align::Center);
        let next_stage = if badges < 8 { have.iter().position(|b| !b).unwrap_or(0) } else { league::ELITE };
        // Normal / Hard / Max.
        let diff = self.difficulty();
        let bonus = league::bonus(diff);
        let locked = self.dex.league_run.is_some();
        let dx = x0 + 184.0;
        let seg = 46.0;
        self.r.fill_round(R::new(dx, y, dx + 3.0 * seg, y + 24.0), 10.0, th.bg2.a(0.6));
        for (d, name) in league::DIFFICULTY.iter().enumerate() {
            let r = R::new(dx + d as f32 * seg, y, dx + (d + 1) as f32 * seg, y + 24.0);
            let on = d as u8 == diff;
            let hot = !locked && self.hot == Some(Btn::Bt(BtBtn::Difficulty(d as u8)));
            let col = [th.good, th.accent, th.bad][d];
            if on {
                self.r.fill_round(r.inset(2.0), 8.0, col.a(if locked { 0.45 } else { 0.9 }));
            } else if hot {
                self.r.fill_round(r.inset(2.0), 8.0, th.hover);
            }
            self.r.text(name, r, 10.5, 700, if on { th.bg } else { th.text.a(if locked { 0.45 } else { 1.0 }) }, Align::Center);
            if !locked {
                self.buttons.push((r, Btn::Bt(BtBtn::Difficulty(d as u8))));
            }
        }
        let maxed = self.dex.max_champions.iter().any(|c| c == rname);
        let crown = if maxed { " · Max Champion ✓" } else if champ { " · Champion ✓" } else { "" };
        let right = format!("{badges}/8 badges{crown} · you Lv {}", league::party_level(region, next_stage));
        self.r.text(&right, R::new(dx + 3.0 * seg + 6.0, y, x1 - 2.0, y + 24.0), 10.5, 600, if champ { th.good } else { th.subtext }, Align::Right);
        y += 30.0;
        // The eight gyms, in order.
        let mut tiles: Vec<(usize, R)> = Vec::new();
        let gap = 6.0;
        let gw = (x1 - x0 - 3.0 * gap) / 4.0;
        let next = have.iter().position(|b| !b).unwrap_or(8);
        for (g, &done) in have.iter().enumerate() {
            let r = R::xywh(x0 + (g % 4) as f32 * (gw + gap), y + (g / 4) as f32 * (46.0 + gap), gw, 46.0);
            let open = g <= next;
            let t = league::stage_type(region, g);
            let sub = if champ && done { format!("{t} · Rematch Lv {}", (league::rematch_level(region, g) + bonus).min(100)) } else { format!("{t} · Lv {}", (league::stage_level(region, g) + bonus).min(100)) };
            self.stage_tile(r, g, &format!("{}. {}", g + 1, league::LEAGUE[region].gyms[g].0), &sub, type_col(type_index(t)), done, open, g == next);
            tiles.push((g, r));
        }
        y += 2.0 * 46.0 + gap + 8.0;
        let run = self.dex.league_run.as_ref().filter(|r| r.region == region).map(|r| r.stage);
        let lvls = format!("Lv {}–{}", (league::stage_level(region, league::ELITE) + bonus).min(100), (league::stage_level(region, league::CHAMPION) + bonus).min(100));
        let heading = "Elite Four & Champion";
        let e4 = if badges == 8 { format!("{heading} · {lvls} · back to back, no healing") } else { format!("{heading} · {lvls} · after all 8 badges") };
        self.r.text(&e4, R::new(x0 + 2.0, y, x1, y + 16.0), 10.5, 600, th.subtext, Align::Left);
        let note = if locked {
            "Difficulty is set until this Elite Four run ends".to_string()
        } else {
            ["Hard and Max: tougher trainers, bigger prizes", "Hard: trainers +5 Lv, sharper, more prizes", "Max: trainers +10 Lv, smartest, the best prizes"][diff as usize].to_string()
        };
        self.r.text(&note, R::new(x0 + 2.0, y, x1 - 2.0, y + 16.0), 10.0, 600, [th.subtext, th.accent, th.bad][diff as usize], Align::Right);
        y += 18.0;
        let ew = (x1 - x0 - 4.0 * gap) / 5.0;
        for i in 0..5 {
            let s = league::ELITE + i;
            let r = R::xywh(x0 + i as f32 * (ew + gap), y, ew, 46.0);
            let (name, t) = if s == league::CHAMPION { (league::LEAGUE[region].champion, "Champion") } else { (league::LEAGUE[region].elite[i].0, league::stage_type(region, s)) };
            let col = if s == league::CHAMPION { Rgba::rgb(248, 200, 56) } else { type_col(type_index(t)) };
            let cur = run.unwrap_or(league::ELITE);
            let beaten = run.is_some_and(|r| s < r) || (champ && run.is_none());
            let open = badges == 8 && s == cur;
            self.stage_tile(r, s, name, t, col, beaten, open, open);
            tiles.push((s, r));
        }
        y += 46.0 + 8.0;
        // Travelling trainers.
        let today = self.today();
        let beaten: Vec<u8> = if self.dex.travel_day == today { self.dex.travel_beaten.clone() } else { Vec::new() };
        self.r.text("Travelling trainers today", R::new(x0 + 2.0, y, x1, y + 16.0), 10.5, 600, th.subtext, Align::Left);
        y += 18.0;
        let list = league::travellers(&today, lv, bonus);
        let tw = (x1 - x0 - 2.0 * gap) / 3.0;
        for (i, t) in list.iter().enumerate() {
            let r = R::xywh(x0 + i as f32 * (tw + gap), y, tw, 44.0);
            let done = beaten.contains(&(i as u8));
            let hot = !done && self.hot == Some(Btn::Bt(BtBtn::Traveller(i as u8)));
            self.r.fill_round(r, 10.0, if done { th.good.a(0.16) } else if hot { th.hover } else { th.bg2.a(0.55) });
            for (j, (n, _)) in t.team.iter().enumerate() {
                self.small_thumb(n, r.x0 + 16.0 + j as f32 * 22.0, r.cy(), 22.0, if done { 0.5 } else { 1.0 });
            }
            let tx = r.x0 + 12.0 + t.team.len() as f32 * 22.0;
            self.r.text(&format!("{} {}", t.title, t.name), R::new(tx, r.y0 + 4.0, r.x1 - 6.0, r.y0 + 22.0), 11.0, 700, th.text, Align::Left);
            let lvl = t.team.first().map_or(0, |x| x.1);
            let sub = if done { "Beaten today ✓".to_string() } else { format!("Lv {lvl} · {} Pokémon", t.team.len()) };
            self.r.text(&sub, R::new(tx, r.y0 + 21.0, r.x1 - 6.0, r.y1 - 4.0), 10.0, 500, if done { th.good } else { th.subtext }, Align::Left);
            if !done {
                self.buttons.push((r, Btn::Bt(BtBtn::Traveller(i as u8))));
            }
        }
        y += 44.0 + 8.0;
        // Your two parties.
        for (i, (label, l)) in [("Battle party", false), ("League party", true)].into_iter().enumerate() {
            let r = R::xywh(x0 + i as f32 * 92.0, y - 2.0, 88.0, 19.0);
            let on = self.league_party == l;
            let hot = self.hot == Some(Btn::Bt(BtBtn::Party(l)));
            self.r.fill_round(r, 9.5, if on { th.accent } else if hot { th.hover } else { th.bg2.a(0.6) });
            self.r.text(label, r, 10.0, 600, if on { th.bg } else { th.text }, Align::Center);
            self.buttons.push((r, Btn::Bt(BtBtn::Party(l))));
        }
        let shuffle = R::xywh(x0 + 186.0, y - 2.0, 64.0, 19.0);
        let hot = self.hot == Some(Btn::Bt(BtBtn::Shuffle));
        self.r.fill_round(shuffle, 9.5, if hot { th.hover } else { th.bg2.a(0.6) });
        self.r.text("Shuffle", shuffle, 10.0, 600, th.text, Align::Center);
        self.buttons.push((shuffle, Btn::Bt(BtBtn::Shuffle)));
        let plv = self.shown_level(self.league_party);
        let note = if self.league_party {
            format!("Regional League · Lv {plv} next")
        } else {
            format!("Wild visitors & travelling trainers · Lv {lv}")
        };
        self.r.text(&note, R::new(x0 + 256.0, y, x1 - 2.0, y + 16.0), 10.0, 500, th.subtext, Align::Right);
        y += 20.0;
        let team: Vec<String> = self.battle_team().iter().map(|n| pokemon::stage_at(n, plv)).collect();
        let sw = (x1 - x0 - 5.0 * gap) / 6.0;
        for i in 0..6 {
            let r = R::xywh(x0 + i as f32 * (sw + gap), y, sw, 50.0);
            let hot = self.hot == Some(Btn::Bt(BtBtn::Slot(i as u8)));
            self.r.fill_round(r, 10.0, if hot { th.hover } else { th.bg2.a(0.55) });
            match team.get(i) {
                Some(n) => {
                    self.small_thumb(n, r.cx(), r.y0 + 18.0, 30.0, 1.0);
                    self.r.text(&pokemon::pretty(n), R::new(r.x0 + 2.0, r.y1 - 17.0, r.x1 - 2.0, r.y1 - 2.0), 9.5, 600, th.text, Align::Center);
                }
                None => {
                    self.r.icon(G_PLUS, r, 14.0, th.subtext);
                }
            }
            self.buttons.push((r, Btn::Bt(BtBtn::Slot(i.min(team.len()) as u8))));
        }
        // Pointing at a gym, Elite Four member or champion: who they'll send out.
        let peek = match self.hot {
            Some(Btn::Bt(BtBtn::Stage(s) | BtBtn::Peek(s))) => Some(s as usize),
            _ => None,
        };
        if let Some((s, r)) = peek.and_then(|p| tiles.iter().find(|t| t.0 == p).copied()) {
            self.draw_peek(r, region, s, x0, x1);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stage_tile(&mut self, r: R, stage: usize, name: &str, sub: &str, col: Rgba, beaten: bool, open: bool, next: bool) {
        let th = self.theme.clone();
        let btn = Btn::Bt(BtBtn::Stage(stage as u8));
        let can = open || beaten && stage < league::ELITE;
        let hot = can && self.hot == Some(btn);
        let fill = if hot { th.hover } else if next { th.accent.a(0.2) } else if beaten { th.good.a(0.14) } else { th.bg2.a(if can { 0.55 } else { 0.25 }) };
        self.r.fill_round(r, 10.0, fill);
        let a = if can || beaten { 1.0 } else { 0.45 };
        if stage < league::ELITE {
            // The gym's badge: just an outline until you've earned it.
            let c = (r.x0 + 17.0, r.cy());
            let shape = badge_shape(c, 11.0, stage);
            if beaten {
                self.r.poly(&badge_shape(c, 12.5, stage), col.mix(Rgba::rgb(0, 0, 0), 0.35));
                self.r.poly(&shape, col);
                self.r.poly(&badge_shape((c.0 - 2.0, c.1 - 2.0), 4.5, stage), Rgba::rgb(255, 255, 255).a(0.7));
            } else {
                self.r.poly(&shape, col.a(0.15 * a));
                let mut ring = shape.clone();
                ring.push(shape[0]);
                let segs: Vec<(f32, f32, f32, f32)> = ring.windows(2).map(|w| (w[0].0, w[0].1, w[1].0, w[1].1)).collect();
                self.r.lines(&segs, col.a(0.8 * a), 1.6);
            }
        } else {
            self.r.circle(r.x0 + 17.0, r.cy(), 10.0, col.a(a));
            if beaten {
                self.r.icon(G_CHECK, R::xywh(r.x0 + 7.0, r.cy() - 10.0, 20.0, 20.0), 12.0, th.bg);
            }
        }
        self.r.text(name, R::new(r.x0 + 32.0, r.y0 + 5.0, r.x1 - 4.0, r.y0 + 23.0), 11.0, 700, th.text.a(a), Align::Left);
        self.r.text(sub, R::new(r.x0 + 32.0, r.y0 + 22.0, r.x1 - 4.0, r.y1 - 5.0), 9.5, 500, if next { th.accent } else { th.subtext.a(a) }, Align::Left);
        if can {
            self.buttons.push((r, btn));
        } else {
            self.buttons.push((r, Btn::Bt(BtBtn::Peek(stage as u8))));
        }
    }

    /// The team a stage's trainer will use, over its tile: sprites, names and levels.
    fn draw_peek(&mut self, tile: R, region: usize, stage: usize, x0: f32, x1: f32) {
        let th = self.theme.clone();
        let rematch = self.is_rematch(region, stage);
        let team = league::team(region, stage, rematch, league::bonus(self.difficulty()));
        if team.is_empty() {
            return;
        }
        let cell = 58.0;
        let w = team.len() as f32 * cell + 20.0;
        let h = 94.0;
        let px = (tile.cx() - w / 2.0).clamp(x0, (x1 - w).max(x0));
        let below = stage < 4;
        let py = if below { tile.y1 + 4.0 } else { tile.y0 - h - 4.0 };
        let panel = R::xywh(px, py, w, h);
        self.r.fill_round(panel.inset(-1.0), 12.0, th.subtext.a(0.35));
        self.r.fill_round(panel, 11.0, th.bg);
        let who = league::trainer_name(region, stage);
        let kind = if rematch { "rematch team" } else { "team" };
        let diff = self.difficulty();
        let tag = if diff > 0 { format!(" · {}", league::DIFFICULTY[diff as usize]) } else { String::new() };
        self.r.text(&format!("{who}'s {kind}{tag}"), R::new(px + 10.0, py + 5.0, px + w - 10.0, py + 21.0), 10.5, 700, th.text, Align::Left);
        for (i, (n, lv)) in team.iter().enumerate() {
            let cx = px + 10.0 + cell * (i as f32 + 0.5);
            self.small_thumb(n, cx, py + 42.0, 34.0, 1.0);
            let c = R::new(cx - cell / 2.0 + 1.0, py + 60.0, cx + cell / 2.0 - 1.0, py + 75.0);
            self.r.text(&pokemon::pretty(n), c, 9.0, 600, th.text, Align::Center);
            self.r.text(&format!("Lv {lv}"), R::new(c.x0, c.y1 - 1.0, c.x1, c.y1 + 14.0), 9.0, 500, th.subtext, Align::Center);
        }
    }

    fn draw_team_picker(&mut self, x0: f32, x1: f32, mut y: f32) {
        let th = self.theme.clone();
        let slot = self.team_pick.unwrap_or(0);
        let team = self.battle_team();
        let what = match team.get(slot) {
            Some(n) => format!("Swap {} for…", pokemon::pretty(n)),
            None => format!("Add a Pokémon to your {} party", if self.league_party { "League" } else { "Battle" }),
        };
        self.r.text(&what, R::new(x0 + 2.0, y, x1 - 170.0, y + 24.0), 12.0, 700, th.text, Align::Left);
        if slot < team.len() && team.len() > 1 {
            self.button(R::new(x1 - 164.0, y, x1 - 84.0, y + 24.0), "Remove", BtBtn::Remove, true, false);
        }
        self.button(R::new(x1 - 78.0, y, x1, y + 24.0), "Back", BtBtn::Back, true, false);
        y += 30.0;
        // Everyone you've caught, strongest first.
        let mut all: Vec<&'static str> = self.dex.caught.keys().filter_map(|n| pokemon::species(n).map(|s| s.name)).collect();
        all.sort_by_key(|n| std::cmp::Reverse(crate::battle::base_total(n)));
        all.dedup();
        let (cols, rows) = (10usize, 7usize);
        let per = cols * rows;
        let pages = all.len().div_ceil(per).max(1);
        self.pick_page = self.pick_page.min(pages - 1);
        let cw = (x1 - x0) / cols as f32;
        let ch = 36.0;
        let lv = self.shown_level(self.league_party);
        for (i, &n) in all.iter().skip(self.pick_page * per).take(per).enumerate() {
            let cell = R::xywh(x0 + (i % cols) as f32 * cw, y + (i / cols) as f32 * ch, cw, ch).inset(2.0);
            let on = team.iter().any(|t| pokemon::family(t) == pokemon::family(n));
            let hot = self.hot == Some(Btn::Bt(BtBtn::Pick(n)));
            self.r.fill_round(cell, 8.0, if hot { th.hover } else if on { th.accent.a(0.2) } else { th.bg2.a(0.5) });
            self.small_thumb(n, cell.cx(), cell.cy(), 28.0, 1.0);
            self.buttons.push((cell, Btn::Bt(BtBtn::Pick(n))));
        }
        if all.is_empty() {
            self.r.text("Catch some Pokémon first", R::new(x0, y, x1, y + 40.0), 11.0, 500, th.subtext, Align::Center);
        }
        y += rows as f32 * ch + 6.0;
        let hover = match self.hot {
            Some(Btn::Bt(BtBtn::Pick(n))) => {
                let m = Mon::new(&pokemon::stage_at(n, lv), lv);
                let mv: Vec<&str> = m.moves.iter().map(|i| MOVES[*i as usize].name).collect();
                format!("{} Lv {lv} · {} · HP {} Atk {} Def {} SpA {} SpD {} Spe {} · {}", pokemon::pretty(&m.name), type_names(&m.types), m.stats[0], m.stats[1], m.stats[2], m.stats[3], m.stats[4], m.stats[5], mv.join(", "))
            }
            _ => format!("Strongest first · page {} of {pages}", self.pick_page + 1),
        };
        self.r.text_wrapped(&hover, R::new(x0 + 34.0, y, x1 - 34.0, y + 30.0), 10.0, 500, th.subtext);
        for (r, d, glyph) in [(R::new(x0, y, x0 + 28.0, y + 24.0), -1, "‹"), (R::new(x1 - 28.0, y, x1, y + 24.0), 1, "›")] {
            let hot = self.hot == Some(Btn::Bt(BtBtn::PickPage(d)));
            self.r.fill_round(r, 10.0, if hot { th.hover } else { th.bg2.a(0.6) });
            self.r.text(glyph, r, 15.0, 700, th.text, Align::Center);
            self.buttons.push((r, Btn::Bt(BtBtn::PickPage(d))));
        }
    }
}
