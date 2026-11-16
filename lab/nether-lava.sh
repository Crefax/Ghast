#!/usr/bin/env bash
# 1.16.5 lava push in the nether (faster lava flow: 0.007 instead of 0.0023) against the overworld, under
# Grim: the bot walks along +z over a stone strip through a lava stream (fire resistance on), once in each
# dimension. PASS = 0 Grim flags, the bot crosses the stream in both.
#
#   ./nether-lava.sh [1.16]      starts the lab with allow-nether=true, stops it at the end
# Needs: cargo build --release -p ghast --example entity_probe
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-1.16}"
ex="$LAB_ROOT/../target/release/examples/entity_probe"
[ -x "$ex" ] || { echo "build the example first: cargo build --release -p ghast --example entity_probe" >&2; exit 2; }
BOT=GhastBot13
q() { "$here/console.sh" "$VER" "$@" >/dev/null; }
"$here/stop.sh" "$VER" >/dev/null 2>&1 || true
LAB_PROPS="allow-nether=true ${LAB_PROPS:-}" "$here/start.sh" "$VER" || exit 1
"$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true

# strip <dimension> <y>: a 5-wide stone strip z 0..60 with walls, a lava source over z 20.
strip() {
  local d=$1 y=$2 x="execute in $1 run"
  q "$x forceload add -16 -16 16 80"
  sleep 2
  q "$x fill -3 $y -1 5 $((y + 4)) 62 minecraft:air" \
    "$x fill -3 $y -1 5 $y 62 minecraft:stone" \
    "$x fill -3 $((y + 1)) -1 -3 $((y + 2)) 62 minecraft:glass" \
    "$x fill 5 $((y + 1)) -1 5 $((y + 2)) 62 minecraft:glass" \
    "$x fill -3 $((y + 1)) 62 5 $((y + 2)) 62 minecraft:glass" \
    "$x setblock 1 $((y + 1)) 20 minecraft:lava"
  sleep 4   # let the stream spread
}

for run in overworld nether; do
  if [ $run = nether ]; then dim=minecraft:the_nether; y=100; else dim=minecraft:overworld; y=30; fi
  strip $dim $y
  since=$(date +%H:%M:%S)
  log="$DIR/logs/nether-lava-$run.log"
  ( sleep 3
    q "effect give $BOT minecraft:fire_resistance 9999 0 true"
    q "execute in $dim run tp $BOT 1.5 $((y + 1)) 2.5 0 0" ) &
  end=$(GHAST_PROBE=walk timeout -s TERM 90 "$ex" --host 127.0.0.1 --port "$PORT" --username "$BOT" \
        --version "$GAME" --minutes 0.4 -v 2> "$log" | grep -a "end position")
  wait
  sleep 1
  # Lava contact: the fluid flags the movement packets do not carry; count the slow ticks from the log
  # is not possible, so report the end position (past z 27 = crossed the stream).
  echo "$run: ${end:-no end position (see $log)}"
  f=$("$here/alerts.sh" "$VER" --count --since "$since"); echo "  ${f:-0 Grim flags}"
done
q "effect clear $BOT"
"$here/stop.sh" "$VER" >/dev/null 2>&1 || true
