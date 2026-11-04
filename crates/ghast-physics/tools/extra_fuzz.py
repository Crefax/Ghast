#!/usr/bin/env python3
"""Writes random movement cases in the oracle's replay input format (header + tick inputs).

The oracle's own fuzz (`oracle/run.sh fuzz`) covers walking terrain; this one adds what it leaves
out or rarely hits: doors (with and without their other half), chests and anvils, dense clusters
of stairs / fences / gates / walls / panes with their connection exceptions (barrier, pumpkin,
nether brick vs wooden fence), water and lava of every level incl. falling liquid next to ice
and walls (flow vectors), soul sand and cobweb clusters, jump taps (creative flight toggles),
teleport / abilities / velocity events, item use (the "use" key), effects, and border-adjacent
coordinates (including just past 2^24, where float-built boxes snap).

Usage (from the repository root):

    python3 crates/ghast-physics/tools/extra_fuzz.py --seed 1 --count 2000 --ticks 80 \
        --out oracle/cache/1.8.9/fuzz-in/extra_s1.jsonl.gz
    oracle/run.sh replay --in "$PWD/oracle/cache/1.8.9/fuzz-in/extra_s1.jsonl.gz" \
        --out "$PWD/oracle/cache/1.8.9/fuzz/extra_s1.jsonl.gz"
    cargo test --release -p ghast-physics --features oracle-cache --test fuzz_cache -- --nocapture
"""
import argparse
import gzip
import json
import random

STONE, AIR = 1, 0

# Block palettes: (id, metas, weight).
CLUSTER = [
    (53, range(8), 6), (67, range(8), 3), (108, range(8), 1), (164, range(8), 1),  # stairs
    (85, [0], 5), (113, [0], 3), (188, [0], 1), (192, [0], 1),  # fences
    (107, range(8), 3), (183, range(8), 1), (187, range(8), 1),  # gates
    (139, [0, 1], 4),  # walls
    (101, [0], 3), (102, [0], 3), (160, range(16), 2), (20, [0], 2), (95, range(16), 1),  # panes, glass
    (86, range(4), 1), (91, range(4), 1), (103, [0], 1), (166, [0], 1),  # pumpkin, melon, barrier
    (54, range(2, 6), 2), (146, range(2, 6), 1), (145, range(12), 2),  # chests, anvils
    (44, range(16), 3), (126, range(14), 1),  # slabs
    (1, [0], 6), (18, range(4), 1), (89, [0], 1), (152, [0], 1), (46, [0], 1),  # misc full blocks
    (88, [0], 2), (30, [0], 2), (165, [0], 1), (81, range(16), 1), (92, range(7), 1),
    (78, range(8), 2), (171, range(16), 1), (65, range(2, 6), 2), (106, range(16), 1),
    (96, range(16), 2), (167, range(16), 1), (154, range(6), 1), (118, range(4), 1),
    (0, [0], 6),
]
LIQUID = [(8, range(16), 3), (9, range(16), 3), (10, range(16), 1), (11, range(16), 1),
          (79, [0], 2), (174, [0], 1), (1, [0], 3), (0, [0], 3), (20, [0], 1), (30, [0], 1)]
DOORS = [64, 71, 193, 194, 195, 196, 197]

ORIGINS = [(0, 0), (0, 0), (0, 0), (300, -77), (-5000, 1234), (987_654, -321_000),
           (29_999_960, 100), (-29_999_960, -29_999_960), (100, 29_999_975),
           (-16_777_217, -16_777_219), (16_777_216, 3)]

KEYS = ["forward", "back", "left", "right", "jump", "sneak", "sprint", "use"]


def pick(r, table):
    total = sum(w for _, _, w in table)
    x = r.randrange(total)
    for bid, metas, w in table:
        x -= w
        if x < 0:
            return bid, r.choice(list(metas))
    raise AssertionError


def case(seed, index, ticks):
    r = random.Random(seed * 1_000_003 + index)
    ox, oz = r.choice(ORIGINS)
    ox += r.randint(-12, 12)
    oz += r.randint(-12, 12)
    fy = 63
    size = r.randint(3, 7)
    fills = []

    def fill(x0, y0, z0, x1, y1, z1, bid, meta=0):
        fills.append([x0, y0, z0, x1, y1, z1, bid, meta])

    fill(ox - size - 4, fy - 2, oz - size - 4, ox + size + 4, fy, oz + size + 4, STONE)
    kind = r.random()
    if kind < 0.35:
        # Liquid basin: floor dug out, random liquids and walls inside.
        fill(ox - size, fy - 1, oz - size, ox + size, fy, oz + size, AIR)
        for _ in range(r.randint(10, 60)):
            bid, meta = pick(r, LIQUID)
            x, z = ox + r.randint(-size, size), oz + r.randint(-size, size)
            y = fy - 1 + r.randint(0, 3)
            fill(x, y, z, x + r.randint(0, 2), y + r.randint(0, 1), z + r.randint(0, 2), bid, meta)
    else:
        for _ in range(r.randint(15, 70)):
            bid, meta = pick(r, CLUSTER)
            x, z = ox + r.randint(-size, size), oz + r.randint(-size, size)
            y = fy + 1 + r.randint(0, 2)
            shape = r.randrange(5)
            if shape == 0:
                fill(x, y, z, x + r.randint(0, 3), y, z, bid, meta)
            elif shape == 1:
                fill(x, y, z, x, y, z + r.randint(0, 3), bid, meta)
            else:
                fill(x, y, z, x, y, z, bid, meta)
        for _ in range(r.randint(0, 5)):
            d = r.choice(DOORS)
            x, z = ox + r.randint(-size, size), oz + r.randint(-size, size)
            lower = r.randrange(8)
            fill(x, fy + 1, z, x, fy + 1, z, d, lower)
            if r.random() < 0.85:
                fill(x, fy + 2, z, x, fy + 2, z, d, 8 + r.randrange(4))
    if r.random() < 0.25:
        cy = fy + r.randint(3, 4)
        fill(ox - size, cy, oz - size, ox + size, cy, oz + size, r.choice([1, 44, 20, 102]),
             r.choice([0, 8]))

    sx = ox + r.randint(-2, 2) + 0.05 + 0.9 * r.random()
    sz = oz + r.randint(-2, 2) + 0.05 + 0.9 * r.random()
    bx, bz = int(sx // 1), int(sz // 1)
    sy = fy + 1 + (0 if r.random() < 0.6 else r.randint(0, 4) + r.random())
    if r.random() < 0.8:
        fill(bx, fy + 1, bz, bx, fy + 3 + int(sy - fy), bz, AIR)
    creative = r.random() < 0.25
    flying = creative and r.random() < 0.5
    setup = {
        "game_mode": "creative" if creative else "survival",
        "pos": [sx, sy + (1 if flying else 0), sz],
        "yaw": r.uniform(-360, 360),
        "pitch": r.uniform(-90, 90),
        "vel": [r.uniform(-0.3, 0.3), r.uniform(-0.2, 0.3), r.uniform(-0.3, 0.3)] if r.random() < 0.2 else [0, 0, 0],
        "on_ground": False,
        "fall_distance": r.choice([0.0, 0.0, r.uniform(0, 6)]),
        "sprinting": r.random() < 0.15,
        "flying": flying,
        "fly_speed": r.choice([0.05, 0.05, 0.1, 0.02]),
        "walk_speed": 0.1,
        "movement_speed_base": r.choice([0.1, 0.1, 0.1, 0.12, 0.13, 0.08]),
        "food_level": r.choice([20, 20, 20, 6, 7, 3]),
        "effects": ([[8, r.randint(0, 3)]] if r.random() < 0.15 else [])
        + ([[15, 0]] if r.random() < 0.05 else []),
    }
    out_ticks = []
    yaw, pitch = setup["yaw"], setup["pitch"]
    t = 0
    while t < ticks:
        held = set(r.sample(KEYS, r.randint(0, 3)))
        if r.random() < 0.4:
            held.add("forward")
        span = min(ticks - t, r.randint(1, 10))
        tap_jump = r.random() < 0.2
        dyaw = r.choice([0.0, r.uniform(-25, 25)])
        dpitch = r.choice([0.0, 0.0, r.uniform(-8, 8)])
        for k in range(span):
            keys = set(held)
            if tap_jump:
                keys.discard("jump")
                if k % 2 == 0:
                    keys.add("jump")
            yaw += dyaw
            pitch = max(-90.0, min(90.0, pitch + dpitch))
            inp = {"keys": ",".join(n for n in KEYS if n in keys), "yaw": yaw, "pitch": pitch}
            ev = []
            x = r.random()
            if x < 0.015:
                ev.append({"type": "velocity", "x": r.randint(-6000, 6000), "y": r.randint(-2000, 6000),
                           "z": r.randint(-6000, 6000)})
            elif x < 0.022:
                ev.append({"type": "teleport", "pos": [ox + r.uniform(-3, 3), fy + 1 + r.uniform(0, 3),
                                                        oz + r.uniform(-3, 3)],
                           "yaw": r.uniform(-400, 400), "pitch": r.uniform(-90, 90)})
            elif x < 0.03:
                allow = r.random() < 0.6
                ev.append({"type": "abilities", "invulnerable": allow, "flying": allow and r.random() < 0.5,
                           "allow_flying": allow, "creative": allow and r.random() < 0.5,
                           "fly_speed": r.choice([0.05, 0.1]), "walk_speed": 0.1})
            if ev:
                inp["events"] = ev
            t += 1
            out_ticks.append({"type": "tick", "t": t, "input": inp})
    header = {
        "type": "scenario", "format": 1, "version": "1.8.9",
        "name": f"extra_s{seed}_{index}", "description": "ghast-physics extra fuzz",
        "outside": "unloaded" if r.random() < 0.08 else "air", "loaded_chunks": [],
        "fills": fills, "setup": setup, "ticks": len(out_ticks),
    }
    if header["outside"] == "unloaded":
        header["loaded_chunks"] = [[(ox >> 4) - 1, (oz >> 4) - 1, (ox >> 4), (oz >> 4)]]
    return header, out_ticks


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--count", type=int, default=1000)
    ap.add_argument("--ticks", type=int, default=80)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    with gzip.open(a.out, "wt", encoding="utf-8") as f:
        for i in range(a.count):
            h, ts = case(a.seed, i, a.ticks)
            f.write(json.dumps(h) + "\n")
            for t in ts:
                f.write(json.dumps(t) + "\n")


if __name__ == "__main__":
    main()
