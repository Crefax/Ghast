#!/usr/bin/env bash
# Boats under Grim (1.8.9, 1.12.2, 1.16.5): the entity_probe example (GHAST_PROBE=boat, entity
# tracking on) right-clicks a boat, paddles straight / left / right / straight on, stops, sneaks
# off (it counts as off once the server's set_passengers / attach_entity says so).
#   water - the boat starts on the field's pool (x 32..39)
#   ice   - the boat starts on the ice strip (x 0..7) and is driven onto the grass
#   lily  - 1.8: the pool with a row of lily pads across it
#   snow  - 1.8: grass under two-layer snow west of the strips (snow does not stay on ice)
#   wall  - 1.8: the pool with a stone wall across it at z 21 (the server breaks a fast boat)
# Per scene: Grim flags, server teleports of the player after the start, the server's boat
# corrections (vehicle_move on 1.12 / 1.16, entity_teleport on 1.8; the probe's log has the
# tracked server position next to the client's every 20 ticks), and the boat's final position
# as the bot drove it against the server's.
#   ./boats.sh <1.8|1.12|1.16> [scenes]   (default: water ice; 1.8 also lily snow wall)
# Needs: cargo build -p ghast --release --example entity_probe
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-1.16}"; shift || true
BOT=${BOT:-GhastBot11}
bin="$here/../target/release/examples/entity_probe"
[ -x "$bin" ] || { echo "build it first: cargo build -p ghast --release --example entity_probe" >&2; exit 2; }
case "$FAMILY" in
  1.8)  scenes=${*:-water ice lily snow wall}; boat=Boat; others='@e[type=!Player]'
        summon() { echo "summon Boat $1 {Rotation:[0f,0f]}"; }
        fix_id=0x18; lily=minecraft:waterlily; snow="minecraft:snow_layer 1" ;;
  1.12) scenes=${*:-water ice}; boat=boat; others='@e[type=!player]'
        summon() { echo "summon minecraft:boat $1 {Type:\"oak\",Rotation:[0f,0f]}"; }
        fix_id=0x29; lily=minecraft:waterlily; snow="minecraft:snow_layer 1" ;;
  1.16) scenes=${*:-water ice}; boat=minecraft:boat; others='@e[type=!player]'
        summon() { echo "summon minecraft:boat $1 {Type:\"oak\",Rotation:[0f,0f]}"; }
        fix_id=0x2b; lily=minecraft:lily_pad; snow="minecraft:snow[layers=2]" ;;
  *) echo "boats.sh: 1.8, 1.12 or 1.16" >&2; exit 2 ;;
esac
con() { "$here/console.sh" "$VER" "$@"; }
q() { con "$@" >/dev/null; }
where_boat() { # the server's boat position (1.8 / 1.12: entitydata echoes the entity's NBT)
  if [ "$FAMILY" = 1.16 ]; then con "data get entity @e[type=$boat,limit=1] Pos" | grep -aoE '\[.*\]' | tail -1
  else con "entitydata @e[type=$boat,c=1] {}" | grep -aoE 'Pos:\[[^]]*\]' | tail -1; fi
}
started=0
if ! lab_pid >/dev/null; then "$here/start.sh" "$VER" || exit 1; started=1; fi
fail=0
for scene in $scenes; do
  "$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true
  q "kill $others"
  # The bot joins at the spawn, not where the last scene left it (e.g. sunk in the pool).
  rm -f "$DIR"/world/playerdata/"$(offline_uuid "$BOT")".dat*
  case $scene in
    water|lily|wall) at="35.5 4 10.5"; stand="35.5 4 7.6" ;;
    ice) at="3.5 4 10.5"; stand="3.5 4 7.6" ;;
    snow) at="-4.5 4.2 10.5"; stand="-4.5 4.2 7.6" ;;
    *) echo "unknown scene $scene" >&2; continue ;;
  esac
  case $scene in
    lily) q "fill 32 4 15 39 4 15 $lily" ;;
    snow) q "fill -8 4 6 -1 4 30 $snow" ;;
    wall) q "fill 32 3 21 39 5 21 minecraft:stone" ;;
  esac
  q "$(summon "$at")"
  since=$(date +%H:%M:%S)
  log="$DIR/logs/boats-$scene.log"
  : > "$log"
  ( sleep 4; q "gamemode survival $BOT" "clear $BOT" "tp $BOT $stand 0 30"
    for _ in $(seq 1 160); do grep -aq 'dismounted after\|FAIL' "$log.out" 2>/dev/null && break; sleep 0.5; done
    echo "server boat $(where_boat)" > "$log.server" ) &
  rc=0
  GHAST_TRACE=1 GHAST_PROBE=boat timeout -s TERM 100 "$bin" --host 127.0.0.1 --port "$PORT" --username "$BOT" \
    --version "$GAME" --minutes 0.67 --entities -v > "$log.out" 2> "$log" || rc=$?
  wait
  echo "$scene: exit $rc; $(grep -a 'boarded' "$log.out" | head -1 | cut -c1-60)..."
  grep -a 'dismounted\|PASS\|FAIL' "$log.out" | sed 's/^/  /'
  echo "  last bot vehicle: $(grep -a 'vehicle at' "$log" | tail -1 | sed 's/.*vehicle at //')"
  echo "  $(cat "$log.server" 2>/dev/null)"
  echo "  server teleports after the start: $(grep -a 'teleport .* we had sent' "$log" | grep -avc "teleport ${stand% *}")"
  echo "  server boat corrections ($fix_id): $(grep -ac -- "<- $fix_id " "$log")"
  f=$("$here/alerts.sh" "$VER" --count --since "$since"); echo "  ${f:-0 Grim flags}"
  [ $rc = 0 ] && [ -z "$f" ] || fail=1
done
q "kill $others"
[ $started = 1 ] && { "$here/stop.sh" "$VER" >/dev/null 2>&1 || true; }
exit $fail
