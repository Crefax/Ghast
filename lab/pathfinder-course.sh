#!/usr/bin/env bash
# Live check of ghast-pathfinder's gap jumps and maze solving on the local 1.8, 1.12, 1.16, 26.1 or 26.2
# lab:
#   ./pathfinder-course.sh 1.8 build    build the course (below) and summon the armor stand
#   ./pathfinder-course.sh 1.8 tp NAME  teleport NAME to the corridor start (62.5 4 0.5)
#   ./pathfinder-course.sh 1.8 follow   move the armor stand through the maze (every 15 s, ~2 min)
#   ./pathfinder-course.sh 1.8 clear    remove the course again
# Superflat (grass y3, players stand at y4). Inside the spawn chunks, clear of the test pad:
# - parkour corridor x 60..123, z -1..1 between 4-high stone walls (closed at the east end):
#   trenches dug to bedrock (3 deep, dead ends) 1, 2 and 3 wide; a gap of 1 up onto a raised
#   floor; a gap of 2 down from it; stepping stones (1x1 pillars) with gaps 2, 1, 3, 2 (up),
#   1 (down), 2. The bot runs it with `goto` from x 62 to x 119 and back.
# - a 31x31 perfect maze (seed 7), 3-high stone walls, at x 20..50, z 60..90; centre cell
#   35 4 75, openings at 20 4 61 (west) and 50 4 89 (east).
# Run the bot: cargo run -p ghast-ext --release --example lab_course (see its docs).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-}"; shift || true
cmd=${1:-build}
GRASS=minecraft:grass
case "$VER" in
  1.8) STAND=ArmorStand ;;
  1.12) STAND=armor_stand ;;
  1.16) STAND=armor_stand; GRASS=minecraft:grass_block ;;
  26.1|26.2) STAND=armor_stand; GRASS=minecraft:grass_block ;;
  *) echo "only the 1.8, 1.12, 1.16, 26.1 and 26.2 labs (their block and entity names)" >&2; exit 2 ;;
esac
console() { CMD_GAP=${CMD_GAP:-0.02} "$here/console.sh" "$VER" | grep -vE 'blocks? (filled|placed)|filled [0-9]+ blocks|No blocks were filled' || true; }

course() { # $1 = block for walls/floors (minecraft:stone, or minecraft:air to clear)
  python3 - "$1" "$STAND" "$GRASS" <<'PY'
import random, sys
b = sys.argv[1]
stand = sys.argv[2]
grass = sys.argv[3]
clear = b == "minecraft:air"
out = []
f = lambda x1, y1, z1, x2, y2, z2, blk: out.append(f"fill {x1} {y1} {z1} {x2} {y2} {z2} {blk}")
# Ground: grass at y3 and dirt below everywhere first.
f(57, 4, -4, 126, 10, 4, "minecraft:air")
f(57, 1, -4, 126, 2, 4, "minecraft:dirt")
f(57, 3, -4, 126, 3, 4, grass)
f(17, 4, 57, 53, 10, 93, "minecraft:air")
f(17, 1, 57, 53, 2, 93, "minecraft:dirt")
f(17, 3, 57, 53, 3, 93, grass)
if not clear:
    # Corridor walls (4 high) and the east end cap.
    for z in (-2, 2):
        f(60, 4, z, 123, 7, z, b)
    f(123, 4, -2, 123, 7, 2, b)
    trench = lambda x1, x2: (f(x1, 3, -1, x2, 3, 1, "minecraft:air"), f(x1, 1, -1, x2, 2, 1, "minecraft:air"))
    trench(66, 66)            # gap 1
    trench(71, 72)            # gap 2
    trench(77, 79)            # gap 3
    trench(84, 84)            # gap 1, then one up
    f(85, 4, -1, 89, 4, 1, b)  # raised floor
    trench(90, 91)            # gap 2, one down
    trench(96, 111)           # stepping stones
    for x, top in ((98, 3), (100, 3), (104, 3), (107, 4), (109, 3)):
        f(x, 1, 0, x, top, 0, b)
    # Walls down into the trenches so the corridor stays closed below the floor.
    for z in (-2, 2):
        f(60, 1, z, 123, 3, z, b)
    # The maze: recursive backtracker, 15x15 cells, grid 31x31 at (20, 60).
    n, ox, oz, h = 15, 20, 60, 3
    size = 2 * n + 1
    f(ox, 4, oz, ox + size - 1, 3 + h, oz + size - 1, b)
    rnd = random.Random(7)
    carve = lambda gx, gz: f(ox + gx, 4, oz + gz, ox + gx, 3 + h, oz + gz, "minecraft:air")
    seen = {(0, 0)}
    stack = [(0, 0)]
    carve(1, 1)
    while stack:
        cx, cz = stack[-1]
        nb = [(cx + dx, cz + dz) for dx, dz in ((1, 0), (-1, 0), (0, 1), (0, -1))
              if 0 <= cx + dx < n and 0 <= cz + dz < n and (cx + dx, cz + dz) not in seen]
        if not nb:
            stack.pop()
            continue
        nx, nz = rnd.choice(nb)
        seen.add((nx, nz))
        carve(2 * nx + 1, 2 * nz + 1)
        carve(cx + nx + 1, cz + nz + 1)
        stack.append((nx, nz))
    carve(0, 1)
    carve(size - 1, size - 2)
    out.append(f"kill @e[type={stand}]")
    out.append(f"summon {stand} 35.5 4 75.5 {{NoGravity:1b,Invulnerable:1b}}")
else:
    out.append(f"kill @e[type={stand}]")
print("\n".join(out))
PY
}

case "$cmd" in
  build)
    if is_modern || [ "$VER" = 1.16 ]; then
      # 26.x keeps no spawn chunks loaded (and 1.16 not all of the course): force-load the
      # course area, give it a moment.
      echo "forceload add 16 -16 127 95" | console
      sleep 3
    fi
    course minecraft:stone | console ;;
  clear)
    course minecraft:air | console
    { is_modern || [ "$VER" = 1.16 ]; } && echo "forceload remove all" | console ;;
  tp) printf 'tp %s 62.5 4 0.5\n' "${2:?name}" | console ;;
  follow)
    # Maze cells (feet blocks): x = 21 + 2i, z = 61 + 2j; one waypoint hangs 5 blocks up in
    # the air (out of reach: the bot waits under it).
    for p in "21 4 61" "49 4 89" "27 4 83" "43 4 67" "35 9 75" "35 4 75" "21 4 89" "49 4 61" "35 4 75"; do
      set -- $p
      printf 'tp @e[type=%s] %s.5 %s %s.5\n' "$STAND" "$1" "$2" "$3" | console
      sleep "${FOLLOW_GAP:-15}"
    done ;;
  *) echo "usage: $0 1.8|1.12|1.16|26.1|26.2 build|tp NAME|follow|clear" >&2; exit 2 ;;
esac
