#!/usr/bin/env bash
# 1.12.2 / 1.16.5 elytra under Grim, three flights of the elytra_flight example from a 37-block tower
# (take-off, glide with a slow turn, firework rockets, dive, landing):
#   plain  - 3 rockets, lands on the grass
#   wall   - a glass wall across the first glide metres (the bot glides into it)
#   water  - 0 rockets, the field under the flight is a pool 2 deep (1.16 only by default)
# Per flight: Grim flags, server teleports after the start, the bot's landing position against the
# server's (`data get entity` on 1.16, `tp ~ ~ ~` on 1.12, asked while the bot stands there; the
# bot's own is the summary's position, 2 s after the first ground contact).
#   ./elytra-legacy-check.sh <1.12|1.16> [plain wall water]
# Needs: cargo build -p ghast --release --example elytra_flight
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-1.16}"; shift || true
runs=${*:-plain wall water}
BOT=${BOT:-GhastBot10}
bin="$here/../target/release/examples/elytra_flight"
[ -x "$bin" ] || { echo "build it first: cargo build -p ghast --release --example elytra_flight" >&2; exit 2; }
case "$VER" in
  1.12) rockets_item=fireworks; chest="replaceitem entity $BOT slot.armor.chest elytra"; glass=glass; grassb=grass
        water="water"; air=air; where() { "$here/console.sh" "$VER" "tp $BOT ~ ~ ~" | grep -aoE 'Teleported .*' | tail -1; } ;;
  1.16) rockets_item=firework_rocket; chest="replaceitem entity $BOT armor.chest elytra"; glass=minecraft:glass; grassb=minecraft:grass_block
        water=minecraft:water; air=minecraft:air; where() { "$here/console.sh" "$VER" "data get entity $BOT Pos" | grep -aoE '\[.*\]' | tail -1; } ;;
  *) echo "legacy labs only (1.12, 1.16)" >&2; exit 2 ;;
esac
q() { "$here/console.sh" "$VER" "$@" >/dev/null; }
"$here/stop.sh" "$VER" >/dev/null 2>&1 || true
# The bot's saved state from earlier runs (a death, a position in the air) would stand in the way.
rm -f "$DIR"/world/playerdata/"$(offline_uuid "$BOT")".dat*
"$here/start.sh" "$VER" || exit 1
for run in $runs; do
  "$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true
  q "fill 0 4 30 2 40 32 stone"
  rockets=3
  case $run in
    wall) q "fill -30 4 37 12 44 37 $glass" ;;
    water) rockets=0; q "fill -60 3 -30 40 4 80 $water" ;;
  esac
  since=$(date +%H:%M:%S)
  log="$DIR/logs/elytra-check-$run.log"
  : > "$log"
  ( sleep 4
    q "gamemode survival $BOT" "clear $BOT" "give $BOT $rockets_item 16" "$chest" "tp $BOT 1.5 41 31.5 0 0"
    # The server's position once the bot reports its landing (it stands still for 2 s then).
    for _ in $(seq 1 120); do grep -aq 'landed at' "$log" && break; sleep 0.5; done
    sleep 1.2; echo "server $(where)" > "$log.server" ) &
  timeout -s TERM 150 "$bin" --host 127.0.0.1 --port "$PORT" --username "$BOT" --version "$GAME" -v \
    --seed "$rockets" 2> "$log"
  rc=$?
  wait
  # Teleports after the start /tp burst (the /tp races the bot's movement packets: Paper answers
  # the in-flight ones with "moved too quickly" teleports to the same spot).
  tps=$(grep -a 'teleport .* we had sent' "$log" | grep -avc 'teleport 1.5 41.0 31.5')
  echo "$run: exit $rc; $(grep -a 'summary:' "$log" | sed 's/.*summary: //'); first ground $(grep -a 'landed at' "$log" | sed 's/.*landed at //' | head -1); $(cat "$log.server" 2>/dev/null)"
  echo "  server teleports after the start: $tps"
  f=$("$here/alerts.sh" "$VER" --count --since "$since"); echo "  ${f:-0 Grim flags}"
  case $run in
    wall) q "fill -30 4 37 12 44 37 $air" ;;
    water) q "fill -60 3 -30 40 3 80 $grassb" "fill -60 4 -30 40 4 80 $air" ;;
  esac
done
q "clear $BOT" "fill 0 4 30 2 40 32 $air"
"$here/stop.sh" "$VER" >/dev/null 2>&1 || true
