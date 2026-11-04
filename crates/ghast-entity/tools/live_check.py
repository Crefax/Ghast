#!/usr/bin/env python3
"""Live entity check against the local labs (lab/README.md).

Summons named entities near the probe bot (examples/live_probe.rs), moves some with many small
teleports, lets a bat fly, freezes it, then compares the probe's tracked server positions with
the server's own entity positions. Expected: exact up to the protocol's fixed point (1/32 in
1.8, 1/4096 in 1.16) or the exact double after a spawn / teleport.

    cargo build -p ghast-entity --example live_probe
    python3 crates/ghast-entity/tools/live_check.py 1.8|1.16

Uses the lab account GhastBot3 (must be offline) and the area around x=300, z=300.
"""
import math, os, re, subprocess, sys, time

VER = sys.argv[1]  # 1.8 | 1.16
ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "../../.."))
LAB = f"{ROOT}/lab"
PROBE = os.environ.get("PROBE", f"{os.environ.get('CARGO_TARGET_DIR', ROOT + '/target')}/debug/examples/live_probe")
OUT = os.environ.get("OUT", f"/tmp/ghast-entity-snap-{VER}.txt")
USER = "GhastBot3"
PORT = "25618" if VER == "1.8" else "25616"
BX, BZ = 300, 300

def console(*cmds, wait=0.7):
    r = subprocess.run([f"{LAB}/console.sh", VER, *cmds], capture_output=True, text=True,
                       env={**os.environ, "CMD_WAIT": str(wait)})
    return r.stdout

if VER == "1.8":
    ents = [
        ("p1", f"summon ArmorStand {BX+0.3} 4 {BZ+2.7} {{NoGravity:1,CustomName:\"p1\"}}"),
        ("p2", f"summon ArmorStand {BX-2.123} 4 {BZ-1.877} {{NoGravity:1,CustomName:\"p2\"}}"),
        ("p3", f"summon ArmorStand {BX+3.33} 4 {BZ-3.66} {{NoGravity:1,CustomName:\"p3\"}}"),
        ("c2", f"summon Bat {BX-3} 5 {BZ+3} {{CustomName:\"c2\"}}"),
        ("i1", f"summon Item {BX+1.5} 6 {BZ+1.5} {{Item:{{id:stone,Count:5}},Motion:[0.2,0.3,0.1],CustomName:\"i1\"}}"),
        ("x1", f"summon XPOrb {BX-11.5} 4 {BZ+1.5} {{Value:7,CustomName:\"x1\"}}"),
        ("m1", f"summon MinecartRideable {BX+6.5} 4 {BZ-0.5} {{CustomName:\"m1\"}}"),
        ("b1", f"summon Boat {BX-4.5} 4 {BZ-0.5} {{CustomName:\"b1\"}}"),
        ("r1", f"summon ArmorStand {BX+5.5} 4 {BZ+5.5} {{CustomName:\"r1\",Riding:{{id:Boat,CustomName:\"r2\"}}}}"),
        ("r2", "list"),
    ]
    tpmove = lambda n, x, y, z: f"tp @e[name={n}] {x} {y} {z}"
    moving = ["c2"]
    freeze = lambda n: f"entitydata @e[name={n}] {{NoAI:1}}"
    truth_cmd = lambda n: f"entitydata @e[name={n}] {{}}"
    def parse_truth(out):
        m = re.search(r"Pos:\[0:([-0-9.E]+)d,1:([-0-9.E]+)d,2:([-0-9.E]+)d\]", out)
        return tuple(float(v) for v in m.groups()) if m else None
    Q = 32.0
    tpcmd = f"tp {USER} {BX+0.5} 4 {BZ+0.5}"
    cleanup = "kill @e[type=!Player,r=40,x=300,y=4,z=300]"
else:
    def nm(n): return f'CustomName:\'{{"text":"{n}"}}\''
    ents = [
        ("p1", f"summon armor_stand {BX+0.3} 4 {BZ+2.7} {{NoGravity:1b,{nm('p1')}}}"),
        ("p2", f"summon armor_stand {BX-2.123} 4 {BZ-1.877} {{NoGravity:1b,{nm('p2')}}}"),
        ("p3", f"summon armor_stand {BX+3.33} 4 {BZ-3.66} {{NoGravity:1b,{nm('p3')}}}"),
        ("c2", f"summon bat {BX-3} 5 {BZ+3} {{{nm('c2')}}}"),
        ("i1", f"summon item {BX+1.5} 6 {BZ+1.5} {{Item:{{id:\"minecraft:stone\",Count:5b}},Motion:[0.2,0.3,0.1],{nm('i1')}}}"),
        ("x1", f"summon experience_orb {BX-11.5} 4 {BZ+1.5} {{Value:7s,{nm('x1')}}}"),
        ("m1", f"summon minecart {BX+6.5} 4 {BZ-0.5} {{{nm('m1')}}}"),
        ("b1", f"summon boat {BX-4.5} 4 {BZ-0.5} {{{nm('b1')}}}"),
        ("r1", f"summon boat {BX+5.5} 4 {BZ+5.5} {{{nm('r2')},Passengers:[{{id:\"minecraft:armor_stand\",{nm('r1')}}}]}}"),
        ("r2", "list"),
    ]
    tpmove = lambda n, x, y, z: f"tp @e[name={n},limit=1] {x} {y} {z}"
    moving = ["c2"]
    freeze = lambda n: f"data merge entity @e[name={n},limit=1] {{NoAI:1b}}"
    truth_cmd = lambda n: f"data get entity @e[name={n},limit=1] Pos"
    def parse_truth(out):
        m = re.search(r"entity data: \[([-0-9.E]+)d, ([-0-9.E]+)d, ([-0-9.E]+)d\]", out)
        return tuple(float(v) for v in m.groups()) if m else None
    Q = 4096.0
    tpcmd = f"tp {USER} {BX+0.5} 4 {BZ+0.5}"
    cleanup = "kill @e[type=!player,x=300,y=4,z=300,distance=..40]"

print(console("list"))
probe = subprocess.Popen([PROBE, "--port", PORT, "--version", VER, "--user", USER,
                          "--secs", "60", "--out", OUT])
time.sleep(3)
print(console(tpcmd, cleanup, wait=1.0))
time.sleep(4)
print(console(*[c for _, c in ents], wait=1.0))
import random
rnd = random.Random(5)
p2 = [BX-2.123, 4.0, BZ-1.877]
p3 = [BX+3.33, 4.0, BZ-3.66]
for step in range(30):
    for name, p, span in (("p2", p2, 0.4), ("p3", p3, 3.0)):
        for k in range(3):
            p[k] += rnd.uniform(-span, span)
        p[1] = max(p[1], 4.0)
        console(tpmove(name, *(f"{v:.6f}" for v in p)), wait=0.0)
    time.sleep(0.15)
time.sleep(3)  # let the bat fly
console(*[freeze(n) for n in moving], wait=0.5)
time.sleep(5)  # final position updates (vanilla resends small moves every 60 ticks)

truth = {}
for n, _ in ents:
    t = parse_truth(console(truth_cmd(n), wait=0.4))
    truth[n] = t
time.sleep(1.5)
snap = open(OUT).read().splitlines()
probe.terminate()
probe.wait()

print(snap[0])
tracked = {}
for line in snap[1:]:
    f = line.split()
    tracked[f[2]] = (f[1], float(f[3]), float(f[4]), float(f[5]), f[-2], f[-1], line)

ok = 0
bad = 0
for n, _ in ents:
    t = truth.get(n)
    tr = tracked.get(n)
    if tr is None and t is not None:
        # 1.8 minecarts keep their custom name outside the metadata: match unnamed by position.
        for line in snap[1:]:
            f = line.split()
            if f[2] == "-" and all(abs(float(a) - b) < 0.05 for a, b in zip(f[3:6], t)):
                tr = (f[1], float(f[3]), float(f[4]), float(f[5]), f[-2], f[-1], line)
    if t is None and tr is None:
        print(f"{n}: gone on both sides (e.g. picked up)")
        continue
    if t is None or tr is None:
        print(f"{n}: MISSING truth={t} tracked={tr}")
        bad += 1
        continue
    pos = tr[1:4]
    # Expected: the exact double (after spawn/teleport) or floor(x * Q) / Q (after rel moves).
    exp_q = tuple(math.floor(v * Q) / Q for v in t)
    match = all(p == e or p == v for p, e, v in zip(pos, exp_q, t))
    err = max(abs(p - v) for p, v in zip(pos, t))
    print(f"{n:3} {tr[0]:22} tracked={pos} truth={t} maxerr={err:.6f} {'OK' if match else 'DIFF'} vehicle={tr[4]} passengers={tr[5]}")
    ok += match
    bad += not match
print(f"{VER}: {ok} exact, {bad} differ")
console(cleanup, wait=0.3)
