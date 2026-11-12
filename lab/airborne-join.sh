#!/usr/bin/env bash
# Regression check: joining with the saved position floating above the ground.
#   ./airborne-join.sh <1.8|1.12|1.16|26.1|26.2> [runs, default 5]
# Each run uses a fresh offline name: it logs in, stands on a block placed at (21, 4, 21), logs
# out, the block is removed while it is offline, and it logs in again 1 block above the grass
# (the bot then falls on its first ticks). Passes when Grim flagged nothing during all runs.
# On 26.x this covers the level-load gate (player_loaded only once the player's column and its
# eight neighbours are loaded, a tick later; earlier gates drew Simulation + GroundSpoof here).
# Starts the lab if it is not running and stops it again in that case.
# Needs the afk example: cargo build -p ghast --release --example afk
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-}"
runs=${2:-5}
afk="$here/../target/release/examples/afk"
[ -x "$afk" ] || { echo "build the afk example first: cargo build -p ghast --release --example afk" >&2; exit 2; }
started=0
if ! lab_pid >/dev/null; then "$here/start.sh" "$VER"; started=1; fi
"$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true
since=$(date +%H:%M:%S)
bot() { # bot <name> <minutes>
  timeout -s TERM 60 "$afk" --host 127.0.0.1 --port "$PORT" --username "$1" --version "$GAME" \
    --minutes "$2" >/dev/null 2>&1 || true
}
for i in $(seq 1 "$runs"); do
  name="GhastJoin$i"
  ( sleep 3; "$here/console.sh" "$VER" "setblock 21 4 21 dirt" "tp $name 21.5 5 21.5" >/dev/null ) &
  bot "$name" 0.15
  wait
  "$here/console.sh" "$VER" "setblock 21 4 21 air" >/dev/null
  bot "$name" 0.25
  echo "run $i ($name): joined 1 block above the ground"
done
sleep 1
flags=$("$here/alerts.sh" "$VER" --count --since "$since")
if [ "$started" = 1 ]; then "$here/stop.sh" "$VER"; fi
if [ -n "$flags" ]; then
  echo "FAIL: Grim flags during the airborne joins:"; echo "$flags"; exit 1
fi
echo "PASS: $runs airborne joins on $VER ($GAME), 0 Grim flags"
