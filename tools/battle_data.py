"""Build src/movedata.rs, the battle data: base stats, damaging moves and who learns which
moves by level, for every Pokémon and form in src/national.rs.

Uses three tables from PokeAPI's data (https://github.com/PokeAPI/pokeapi, data/v2/csv):
pokemon_stats.csv, moves.csv and pokemon_moves.csv. Download them into a folder of their
own, then:

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
    "struggle", "snore", "spit-up", "natural-gift", "fling", "trump-card", "judgment",
    "multi-attack", "techno-blast", "hidden-power", "weather-ball", "terrain-pulse",
    "revelation-dance", "raging-fury", "hyperspace-fury", "light-of-ruin", "head-smash",
    "shell-trap", "burn-up", "double-shock", "steel-roller", "poltergeist", "fusion-flare",
    "fusion-bolt", "relic-song", "dragon-energy", "water-spout", "eruption",
}

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

    moves = {}
    for r in csv.DictReader(open(os.path.join(src, "moves.csv"), encoding="utf-8")):
        if r["damage_class_id"] not in ("2", "3") or not r["power"] or r["identifier"] in SKIP:
            continue
        t = TYPE_ORDER.get(int(r["type_id"]))
        power = int(r["power"])
        if t is None or power < 10 or int(r["id"]) >= 10000:
            continue
        acc = int(r["accuracy"]) if r["accuracy"] else 0
        moves[int(r["id"])] = (pretty(r["identifier"]), t, min(power, 250), acc, r["damage_class_id"] == "3", int(r["priority"]))

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

    order = sorted(used, key=used.get)
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write("//! Battle data, generated by `tools/battle_data.py` from PokeAPI's data: base stats,\n")
        f.write("//! the damaging moves, and the moves each Pokémon learns by level.\n\n")
        f.write("pub struct Move {\n    pub name: &'static str,\n    /// Index into `pokemon::TYPES`.\n    pub t: u8,\n")
        f.write("    pub power: u8,\n    /// 0 = never misses.\n    pub acc: u8,\n    pub special: bool,\n    pub prio: i8,\n}\n\n")
        f.write(f"pub static MOVES: [Move; {len(order)}] = [\n")
        for m in order:
            n, t, p, a, s, pr = moves[m]
            f.write(f'    Move {{ name: "{n}", t: {t}, power: {p}, acc: {a}, special: {"true" if s else "false"}, prio: {pr} }},\n')
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
