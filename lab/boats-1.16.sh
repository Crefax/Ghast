#!/usr/bin/env bash
# 1.16.5 boats under Grim: the entity_probe example (GHAST_PROBE=boat, entity tracking on)
# right-clicks an oak boat, paddles straight / left / right / straight on, stops, sneaks off.
#   water - the boat starts on the field's pool (x 32..39) and is driven over its edge onto land
#   ice   - the boat starts on the ice strip (x 0..7)
# Per scene: Grim flags, server teleports / vehicle corrections after the start, and the boat's
# position as the bot last drove it against the server's (`data get entity`, after the dismount).
#   ./boats-1.16.sh [water ice]
# Needs: cargo build -p ghast --release --example entity_probe
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select 1.16
scenes=${*:-water ice}
BOT=${BOT:-GhastBot11}
bin="$here/../target/release/examples/entity_probe"
[ -x "$bin" ] || { echo "build it first: cargo build -p ghast --release --example entity_probe" >&2; exit 2; }
con() { "$here/console.sh" "$VER" "$@"; }
q() { con "$@" >/dev/null; }
started=0
if ! lab_pid >/dev/null; then
  "$here/start.sh" "$VER" || exit 1; started=1
fi
for scene in $scenes; do
  "$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true
  q "kill @e[type=!player]"
  # The bot joins at the spawn, not where the last scene left it (e.g. sunk in the pool).
  rm -f "$DIR"/world/playerdata/"$(offline_uuid "$BOT")".dat*
  case $scene in
    water) at="35.5 4 10.5"; stand="35.5 4 7.6" ;;
    ice) at="3.5 4 10.5"; stand="3.5 4 7.6" ;;
    *) echo "unknown scene $scene" >&2; continue ;;
  esac
  q "summon minecraft:boat $at {Type:\"oak\",Rotation:[0f,0f]}"
  since=$(date +%H:%M:%S)
  log="$DIR/logs/boats-$scene.log"
  : > "$log"
  ( sleep 4; q "gamemode survival $BOT" "clear $BOT" "tp $BOT $stand 0 30"
    for _ in $(seq 1 160); do grep -aq 'dismounted after' "$log.out" 2>/dev/null && break; sleep 0.5; done
    echo "server boat $(con "data get entity @e[type=minecraft:boat,limit=1] Pos" | grep -aoE '\[.*\]' | tail -1)" > "$log.server" ) &
  GHAST_TRACE=1 GHAST_PROBE=boat timeout -s TERM 100 "$bin" --host 127.0.0.1 --port "$PORT" --username "$BOT" \
    --version "$GAME" --minutes 0.67 --entities -v > "$log.out" 2> "$log"
  rc=$?
  wait
  echo "$scene: exit $rc; $(grep -a 'boarded' "$log.out" | head -1 | cut -c1-80)..."
  grep -a 'dismounted\|PASS\|FAIL' "$log.out" | sed 's/^/  /'
  echo "  last bot vehicle: $(grep -a 'vehicle at' "$log" | tail -1 | sed 's/.*vehicle at //')"
  echo "  $(cat "$log.server" 2>/dev/null)"
  echo "  server teleports after the start: $(grep -a 'teleport .* we had sent' "$log" | grep -avc "teleport ${stand% *}")"
  # Clientbound vehicle_move (0x2b): the server corrects (or echoes back) the boat's position.
  echo "  server vehicle_move packets: $(grep -ac -- '<- 0x2b ' "$log")"
  f=$("$here/alerts.sh" "$VER" --count --since "$since"); echo "  ${f:-0 Grim flags}"
done
q "kill @e[type=!player]"
[ $started = 1 ] && { "$here/stop.sh" "$VER" >/dev/null 2>&1 || true; }
