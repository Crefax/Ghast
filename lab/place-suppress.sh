#!/usr/bin/env bash
# 1.8 / 1.12 placements the vanilla client does not even send (BlockItem.canPlaceItemBlock fails) and
# ones it sends although the server refuses, under Grim: into the bot's own box (ghast writes nothing:
# "could not act"), next to a cow standing in the spot (sent; the server refuses, the predicted block
# goes again), on top of a block at the build limit (sent; nothing is placed, nothing predicted).
#
#   ./place-suppress.sh <1.8|1.12>
# Needs: cargo build --release -p ghast --example block_probe
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-}"
ex="$LAB_ROOT/../target/release/examples/block_probe"
[ -x "$ex" ] || { echo "build the probe first" >&2; exit 2; }
BOT=GhastBot13
q() { "$here/console.sh" "$VER" "$@" >/dev/null; }
"$here/stop.sh" "$VER" >/dev/null 2>&1 || true
LAB_PROPS="spawn-animals=true ${LAB_PROPS:-}" "$here/start.sh" "$VER" || exit 1
"$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true
if [ "$VER" = 1.8 ]; then cow=Cow; else cow=cow; fi
# probe <label> <at> <face> <tp "x y z"> [setup commands after the join...]
probe() {
  local label=$1 at=$2 face=$3 tp=$4 since log out; shift 4
  since=$(date +%H:%M:%S); log="$DIR/logs/suppress-$label.log"
  ( sleep 2.5; q "tp $BOT $tp -90 30" "clear $BOT" "give $BOT cobblestone 16"; [ $# -gt 0 ] && q "$@" ) &
  out=$(PROBE_MODE=place PROBE_AT=$at PROBE_FACE=$face PROBE_TRIES=60 timeout -s TERM 60 "$ex" --host 127.0.0.1 \
        --port "$PORT" --username "$BOT" --version "$GAME" --minutes 0.9 -v 2> "$log" | grep -a PROBE)
  wait; sleep 1
  echo "$label: ${out:-no output (see $log)}; block_place written: $(grep -ac 'predicted' "$log") predicted"
  local f; f=$("$here/alerts.sh" "$VER" --count --since "$since"); echo "  ${f:-0 Grim flags}"
}
probe own-box 0,3,0 up "0.5 4 0.5"
probe cow 2,3,0 up "0.5 4 0.5" "summon $cow 2.5 4 0.5 {NoAI:1b}"
q "kill @e[type=$cow]"
q "setblock 0 254 0 stone" "setblock 2 255 0 stone"
probe build-limit 2,255,0 up "0.5 255 0.5"
q "setblock 0 254 0 air" "setblock 2 255 0 air"
"$here/stop.sh" "$VER" >/dev/null 2>&1 || true
