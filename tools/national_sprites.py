"""Download every Pokémon for Pocket Isle's National Pokédex (personal use): the animated sprite,
its shiny version and its cry, for all 1025, into ~/.config/pocketisle/sprites.

Sprites: animated Black/White GIFs (#1-649) from the PokeAPI sprites repository, Pokémon
Showdown's animated GIFs after that, and the still picture where there's no animation.
Each becomes a PNG strip of its frames plus a JSON of frame sizes and timings, the same
format the island already uses. Cries: OGG from the PokeAPI cries repository. Files that
are already there are skipped, so it can be run again to fill gaps.

Run:  python tools\\national_sprites.py     Log: national-out.txt next to this.
"""

import io
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor

from PIL import Image, ImageSequence

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(os.path.expanduser("~"), ".config", "pocketisle", "sprites")
TABLE = os.path.join(HERE, "..", "src", "national.rs")
LOG = os.path.join(HERE, "national-out.txt")
BASE = "https://raw.githubusercontent.com/PokeAPI/sprites/master/sprites/pokemon"
CRIES = "https://raw.githubusercontent.com/PokeAPI/cries/main/cries/pokemon/latest"
MAX_FRAMES = 90

os.makedirs(OUT, exist_ok=True)
log_lock = __import__("threading").Lock()


def log(msg):
    with log_lock:
        with open(LOG, "a", encoding="utf-8") as f:
            f.write(msg + "\n")


def get(url):
    """The file's bytes, or None if it isn't there. Retries on rate limits and blips."""
    for attempt in range(4):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "pocketisle-sprites"})
            with urllib.request.urlopen(req, timeout=30) as r:
                return r.read()
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
            time.sleep(2 + attempt * 5)
        except Exception:
            time.sleep(2 + attempt * 5)
    return None


def strip(data):
    """A GIF or PNG -> (PNG strip bytes, json dict), cropped to where the Pokémon is."""
    im = Image.open(io.BytesIO(data))
    frames, delays = [], []
    for f in ImageSequence.Iterator(im):
        frames.append(f.convert("RGBA").copy())
        d = f.info.get("duration", 100) or 100
        delays.append(int(d) if d >= 20 else 100)
    if len(frames) > MAX_FRAMES:
        k = -(-len(frames) // MAX_FRAMES)
        frames2, delays2 = [], []
        for i in range(0, len(frames), k):
            frames2.append(frames[i])
            delays2.append(sum(delays[i:i + k]))
        frames, delays = frames2, delays2
    box = None
    for fr in frames:
        b = fr.getchannel("A").getbbox()
        if b:
            box = b if box is None else (min(box[0], b[0]), min(box[1], b[1]), max(box[2], b[2]), max(box[3], b[3]))
    if box is None:
        raise ValueError("empty")
    w, h = box[2] - box[0], box[3] - box[1]
    out = Image.new("RGBA", (w * len(frames), h))
    for i, fr in enumerate(frames):
        out.paste(fr.crop(box), (i * w, 0))
    buf = io.BytesIO()
    out.save(buf, "PNG")
    return buf.getvalue(), {"frames": len(frames), "w": w, "h": h, "delays": delays}


def sprite(job):
    sid, name, shiny = job
    file = f"{name}-shiny" if shiny else name
    js, png = os.path.join(OUT, file + ".json"), os.path.join(OUT, file + ".png")
    if os.path.exists(js) and os.path.exists(png):
        return "have"
    s = "shiny/" if shiny else ""
    urls = []
    if sid <= 649:
        urls.append(f"{BASE}/versions/generation-v/black-white/animated/{s}{sid}.gif")
    urls += [f"{BASE}/other/showdown/{s}{sid}.gif", f"{BASE}/{s}{sid}.png"]
    for url in urls:
        data = get(url)
        if not data:
            continue
        try:
            pic, meta = strip(data)
        except Exception as e:
            log(f"{file}: couldn't read {url}: {e}")
            continue
        with open(png, "wb") as f:
            f.write(pic)
        with open(js, "w", encoding="ascii") as f:
            json.dump(meta, f, separators=(",", ":"))
        return "ok"
    log(f"{file}: no sprite found")
    return "failed"


def cry(job):
    sid, name = job
    path = os.path.join(OUT, name + ".ogg")
    if os.path.exists(path):
        return "have"
    data = get(f"{CRIES}/{sid}.ogg")
    if not data:
        log(f"{name}: no cry")
        return "failed"
    with open(path, "wb") as f:
        f.write(data)
    return "ok"


def main():
    table = open(TABLE, encoding="utf-8").read()
    mons = [(int(i), n) for i, n in re.findall(r'Species \{ id: (\d+), name: "([^"]+)"', table)]
    assert len(mons) == 1025, len(mons)
    open(LOG, "w", encoding="utf-8").write(f"started {time.strftime('%Y-%m-%d %H:%M:%S')}: {len(mons)} Pokémon\n")
    jobs = [(i, n, False) for i, n in mons] + [(i, n, True) for i, n in mons]
    counts = {"ok": 0, "have": 0, "failed": 0}
    with ThreadPoolExecutor(max_workers=8) as pool:
        for k, r in enumerate(pool.map(sprite, jobs), 1):
            counts[r] += 1
            if k % 100 == 0:
                log(f"sprites {k}/{len(jobs)}: {counts}")
        cc = {"ok": 0, "have": 0, "failed": 0}
        for r in pool.map(cry, mons):
            cc[r] += 1
    size = sum(os.path.getsize(os.path.join(OUT, f)) for f in os.listdir(OUT))
    log(f"done {time.strftime('%Y-%m-%d %H:%M:%S')}: sprites {counts}, cries {cc}, folder {size / 1048576:.1f} MB")


if __name__ == "__main__":
    sys.exit(main())
