"""Build src/movedata.rs, the battle data: base stats, damaging moves and moves that raise or
lower stats (with their PP and stat changes), and who learns which moves by level, for every
Pokémon and form in src/national.rs.

Uses five tables from PokeAPI's data (https://github.com/PokeAPI/pokeapi, data/v2/csv):
pokemon_stats.csv, moves.csv, move_meta.csv, move_meta_stat_changes.csv and
pokemon_moves.csv. Download them into a folder of their own, then:

    python tools/battle_data.py <csv folder> src/national.rs src/movedata.rs
"""

import csv
import os
import re
import sys

# PokeAPI's type ids, in the order of `pokemon::TYPES`.
TYPE_ORDER = {1: 0, 10: 1, 11: 2, 12: 3, 13: 4, 15: 5, 2: 6, 4: 7, 5: 8, 3: 9, 14: 10, 7: 11, 6: 12, 8: 13, 16: 14, 17: 15, 9: 16, 18: 17}

# Which game's level-up moves to use, best first (Scarlet/Violet, Sword/Shield, Legends Z-A,
# Brilliant Diamond, then older games). Legends: Arceus, Let's Go and the Japanese Red/Green
# lists are left out: their move lists are unusual.
VERSION_PREF = [25, 20, 30, 23, 18, 17, 16, 15, 14, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1]

# Damaging moves that need rules battles here don't have (recharging, charging up, fainting
# the user, only working first turn or on a sleeping target…): left out.
SKIP = {
    "explosion", "self-destruct", "misty-explosion", "hyper-beam", "giga-impact", "blast-burn",
    "hydro-cannon", "frenzy-plant", "rock-wrecker", "roar-of-time", "prismatic-laser",
    "eternabeam", "meteor-assault", "solar-beam", "solar-blade", "sky-attack", "razor-wind",
    "skull-bash", "focus-punch", "dream-eater", "belch", "last-resort", "synchronoise",
    "fake-out", "first-impression", "future-sight", "doom-desire", "bide", "sucker-punch",
    "steel-beam", "mind-blown", "dig", "fly", "dive", "bounce", "phantom-force",
    "shadow-force", "sky-drop", "geomancy", "freeze-shock", "ice-burn", "meteor-beam",
    "electro-shot", "outrage", "thrash", "petal-dance", "uproar", "rollout", "ice-ball",
    "snore", "spit-up", "natural-gift", "fling", "trump-card", "judgment",
    "multi-attack", "techno-blast", "hidden-power", "weather-ball", "terrain-pulse",
    "revelation-dance", "raging-fury", "hyperspace-fury", "light-of-ruin", "head-smash",
    "shell-trap", "burn-up", "double-shock", "steel-roller", "poltergeist", "fusion-flare",
    "fusion-bolt", "relic-song", "dragon-energy", "water-spout", "eruption",
}

# Status moves that change stats in ways battles here can't follow (they need allies, swap
# or copy stats, or do something else as well): left out.
SKIP_STATUS = {
    "acupressure", "aromatic-mist", "flower-shield", "rototiller", "gear-up", "magnetic-flux",
    "coaching", "decorate", "howl", "growth", "spicy-extract", "jungle-healing", "flatter",
    "swagger", "captivate", "tickle", "venom-drench", "memento", "parting-shot", "tearful-look",
    "noble-roar", "baby-doll-eyes", "max-guard", "clangorous-soul", "no-retreat",
    "fillet-away", "tidy-up", "victory-dance", "stuff-cheeks", "belly-drum", "curse",
    "geomancy", "shelter",
}

# PokeAPI's stat ids for Attack…Evasion, in the order battles keep their stages.
STAT_ORDER = {2: 0, 3: 1, 4: 2, 5: 3, 6: 4, 7: 5, 8: 6}
# Moves that work on the user (or the user's side) rather than the target.
USER_TARGETS = {4, 5, 7, 13}

SPECIAL_NAMES = {
    "u-turn": "U-turn", "x-scissor": "X-Scissor", "double-edge": "Double-Edge",
    "v-create": "V-create", "lock-on": "Lock-On", "will-o-wisp": "Will-O-Wisp",
    "soft-boiled": "Soft-Boiled", "self-destruct": "Self-Destruct", "mud-slap": "Mud-Slap",
    "wake-up-slap": "Wake-Up Slap", "baby-doll-eyes": "Baby-Doll Eyes", "power-up-punch": "Power-Up Punch",
    "multi-attack": "Multi-Attack", "trop-kick": "Trop Kick", "freeze-dry": "Freeze-Dry",
    "topsy-turvy": "Topsy-Turvy", "double-hit": "Double Hit", "all-out-pummeling": "All-Out Pummeling",
}


def pretty(ident):
    return SPECIAL_NAMES.get(ident) or " ".join(w.capitalize() for w in ident.split("-"))


def main(src, national, out):
    text = open(national, encoding="utf-8").read()
    # Every Pokémon and form: (PokeAPI pokemon id, the National number it's a form of).
    mons = [(int(i), int(b)) for i, b in re.findall(r"Species \{ id: (\d+), .*?base: (\d+) \}", text)]

    stats = {}
    for r in csv.DictReader(open(os.path.join(src, "pokemon_stats.csv"), encoding="utf-8")):
        stats.setdefault(int(r["pokemon_id"]), {})[int(r["stat_id"])] = int(r["base_stat"])

    meta = {}
    for r in csv.DictReader(open(os.path.join(src, "move_meta.csv"), encoding="utf-8")):
        meta[int(r["move_id"])] = (int(r["meta_category_id"]), int(r["stat_chance"] or 0))
    changes = {}
    for r in csv.DictReader(open(os.path.join(src, "move_meta_stat_changes.csv"), encoding="utf-8")):
        st = STAT_ORDER.get(int(r["stat_id"]))
        if st is not None and int(r["change"]) != 0:
            changes.setdefault(int(r["move_id"]), []).append((st, int(r["change"])))

    moves = {}
    for r in csv.DictReader(open(os.path.join(src, "moves.csv"), encoding="utf-8")):
        mid = int(r["id"])
        t = TYPE_ORDER.get(int(r["type_id"]))
        if t is None or mid >= 10000:
            continue
        cat, chance = meta.get(mid, (0, 0))
        sc = changes.get(mid, [])
        acc = int(r["accuracy"]) if r["accuracy"] else 0
        pp = int(r["pp"] or 0) or 10
        prio = int(r["priority"])
        if r["damage_class_id"] == "1":
            # A status move that only raises or lowers stats (Swords Dance, Growl…).
            if cat != 2 or not sc or r["identifier"] in SKIP_STATUS:
                continue
            on_user = int(r["target_id"]) in USER_TARGETS
            moves[mid] = (pretty(r["identifier"]), t, 0, acc, False, prio, pp, sc, 100, on_user)
            continue
        if not r["power"] or r["identifier"] in SKIP:
            continue
        power = int(r["power"])
        if power < 10:
            continue
        # A side effect on stats: lowering the target's (Psychic, Crunch…) or changing the
        # user's own (Close Combat, Overheat, Power-Up Punch…).
        if cat not in (6, 7):
            sc = []
        moves[mid] = (pretty(r["identifier"]), t, min(power, 250), acc, r["damage_class_id"] == "3", prio, pp, sc, chance or 100, cat == 7)

    # Level-up moves per Pokémon per game.
    learn = {}
    for r in csv.DictReader(open(os.path.join(src, "pokemon_moves.csv"), encoding="utf-8")):
        if r["pokemon_move_method_id"] != "1":
            continue
        m = int(r["move_id"])
        if m not in moves:
            continue
        lv = max(1, int(r["level"] or 1))
        learn.setdefault(int(r["pokemon_id"]), {}).setdefault(int(r["version_group_id"]), []).append((lv, m))

    def best(pid):
        games = learn.get(pid, {})
        for v in VERSION_PREF:
            if games.get(v):
                seen = {}
                for lv, m in sorted(games[v]):
                    seen.setdefault(m, lv)
                return sorted((lv, m) for m, lv in seen.items())
        return []

    used, rows, sets = {}, [], []
    for pid, base in mons:
        st = stats.get(pid) or stats.get(base)
        if not st:
            continue
        ls = best(pid) or best(base)
        start = len(sets)
        for lv, m in ls:
            if m not in used:
                used[m] = len(used)
            sets.append((lv, used[m]))
        rows.append((pid, [st.get(i, 50) for i in range(1, 7)], start, len(sets) - start))

    # Struggle: what a Pokémon does once all its moves are out of PP.
    if 165 in moves:
        used.setdefault(165, len(used))
    order = sorted(used, key=used.get)
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write("//! Battle data, generated by `tools/battle_data.py` from PokeAPI's data: base stats,\n")
        f.write("//! the damaging moves and the moves that raise or lower stats, and the moves each Pokémon\n")
        f.write("//! learns by level.\n\n")
        f.write("pub struct Move {\n    pub name: &'static str,\n    /// Index into `pokemon::TYPES`.\n    pub t: u8,\n")
        f.write("    /// 0 for a status move (it only changes stats).\n    pub power: u8,\n    /// 0 = never misses.\n    pub acc: u8,\n    pub special: bool,\n    pub prio: i8,\n")
        f.write("    /// How many times it can be used in a battle.\n    pub pp: u8,\n")
        f.write("    /// Stat stage changes: (Attack, Defense, Sp. Atk, Sp. Def, Speed, accuracy, evasion as\n")
        f.write("    /// 0..7, how many stages).\n    pub stats: &'static [(u8, i8)],\n")
        f.write("    /// The chance (%) of the stat changes.\n    pub chance: u8,\n")
        f.write("    /// The stat changes are the user's own (Swords Dance, Close Combat), not the target's.\n    pub on_user: bool,\n}\n\n")
        f.write(f"pub static MOVES: [Move; {len(order)}] = [\n")
        for m in order:
            n, t, p, a, s, pr, pp, st, ch, me = moves[m]
            sts = "[" + ", ".join(f"({i}, {c})" for i, c in st) + "]"
            if not st:
                ch, me = 0, False
            f.write(f'    Move {{ name: "{n}", t: {t}, power: {p}, acc: {a}, special: {"true" if s else "false"}, prio: {pr}, pp: {pp}, stats: &{sts}, chance: {ch}, on_user: {"true" if me else "false"} }},\n')
        f.write("];\n\n")
        f.write("/// (PokeAPI pokemon id, base HP, Attack, Defense, Sp. Atk, Sp. Def, Speed, first learnset\n")
        f.write("/// entry, how many), sorted by id.\n")
        rows.sort()
        f.write(f"pub static MONS: [(u16, [u8; 6], u16, u8); {len(rows)}] = [\n")
        for pid, st, start, n in rows:
            f.write(f"    ({pid}, {[min(255, x) for x in st]}, {start}, {min(n, 255)}),\n")
        f.write("];\n\n")
        f.write("/// (level, index into `MOVES`), each Pokémon's in level order.\n")
        f.write(f"pub static LEARNSET: [(u8, u16); {len(sets)}] = [\n")
        for i in range(0, len(sets), 12):
            f.write("    " + " ".join(f"({lv}, {m})," for lv, m in sets[i:i + 12]) + "\n")
        f.write("];\n")
    print(f"{len(rows)} Pokémon, {len(order)} moves, {len(sets)} learnset entries")


if __name__ == "__main__":
    main(*sys.argv[1:4])
