"""Download the regional forms, Mega Evolutions and Gigantamax forms for Pocket Isle's Pokédex
(personal use): sprite, shiny sprite and cry for each one in `FORMS` (src/national.rs), into
~/.config/pocketisle/sprites, the same way national_sprites.py does the 1025 species. A form
with no cry of its own gets its species' cry. Files already there are skipped.

Run:  python tools\\forms_sprites.py     Log: forms-out.txt next to this.
"""

import os
import re
import shutil
import sys
import time
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import national_sprites as ns  # noqa: E402

ns.LOG = os.path.join(ns.HERE, "forms-out.txt")


def cry(job):
    sid, name, base = job
    path = os.path.join(ns.OUT, name + ".ogg")
    if os.path.exists(path):
        return "have"
    data = ns.get(f"{ns.CRIES}/{sid}.ogg")
    if data:
        with open(path, "wb") as f:
            f.write(data)
        return "ok"
    own = os.path.join(ns.OUT, base + ".ogg")
    if os.path.exists(own):
        shutil.copyfile(own, path)
        return "species"
    ns.log(f"{name}: no cry")
    return "failed"


def home(job):
    """The HOME picture, shrunk to sprite size, for a form with no sprite anywhere else."""
    sid, name, shiny = job
    file = f"{name}-shiny" if shiny else name
    js, png = os.path.join(ns.OUT, file + ".json"), os.path.join(ns.OUT, file + ".png")
    if os.path.exists(js) and os.path.exists(png):
        return "have"
    data = ns.get(f"{ns.BASE}/other/home/{'shiny/' if shiny else ''}{sid}.png")
    if not data:
        return "failed"
    im = ns.Image.open(ns.io.BytesIO(data)).convert("RGBA")
    im.thumbnail((110, 110))
    buf = ns.io.BytesIO()
    im.save(buf, "PNG")
    pic, meta = ns.strip(buf.getvalue())
    with open(png, "wb") as f:
        f.write(pic)
    with open(js, "w", encoding="ascii") as f:
        ns.json.dump(meta, f, separators=(",", ":"))
    return "ok"


def main():
    table = open(ns.TABLE, encoding="utf-8").read()
    national = {int(i): n for i, n in re.findall(r'Species \{ id: (\d+), name: "([^"]+)".*form: 0,', table)}
    forms = [(int(i), n, national[int(b)]) for i, n, b in re.findall(r'Species \{ id: (\d+), name: "([^"]+)".*form: [1-9], base: (\d+)', table)]
    assert len(forms) > 150, len(forms)
    open(ns.LOG, "w", encoding="utf-8").write(f"started {time.strftime('%Y-%m-%d %H:%M:%S')}: {len(forms)} forms\n")
    jobs = [(i, n, False) for i, n, _ in forms] + [(i, n, True) for i, n, _ in forms]
    counts = {"ok": 0, "have": 0, "failed": 0}
    cc = {"ok": 0, "have": 0, "species": 0, "failed": 0}
    with ThreadPoolExecutor(max_workers=8) as pool:
        for job, r in zip(jobs, pool.map(ns.sprite, jobs)):
            if r == "failed":
                r = home(job)
            counts[r] += 1
        for r in pool.map(cry, forms):
            cc[r] += 1
    ns.log(f"done {time.strftime('%Y-%m-%d %H:%M:%S')}: sprites {counts}, cries {cc}")


if __name__ == "__main__":
    sys.exit(main())
