#!/usr/bin/env bash
# Minecarts under Grim (1.8.9, 1.12.2, 1.16.5): entity_probe GHAST_PROBE=cart (entity tracking
# on) right-clicks a rideable minecart on a rail loop west of the pad (x -14..-4, z -14..-4:
# straights, four curves, powered rails on redstone blocks, a one-block hump on the west side),
# pushes it east, looks around for 60 s with jump / sprint presses, then sneaks off (1.16.5 by
# itself; 1.8 / 1.12: the server takes it off on the sneak flag of steer_vehicle).
# Reports Grim flags, the player's server teleports after the start, the cart's position as the
# client has it against the server's (tracked, every 20 ticks), the seat position at the
# dismount and the server's cart position after it.
#   ./carts.sh <1.8|1.12|1.16>
# Needs: cargo build -p ghast --release --example entity_probe
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-1.16}"
BOT=${BOT:-GhastBot11}
bin="$here/../target/release/examples/entity_probe"
[ -x "$bin" ] || { echo "build it first: cargo build -p ghast --release --example entity_probe" >&2; exit 2; }
con() { "$here/console.sh" "$VER" "$@"; }
q() { con "$@" >/dev/null; }
if [ "$FAMILY" = 1.16 ]; then
  others='@e[type=!player]'; cart="summon minecraft:minecart"
  r() { echo "minecraft:rail[shape=$1]"; }
  p() { echo "minecraft:powered_rail[shape=$1,powered=true]"; }
  where_cart() { con "data get entity @e[type=minecraft:minecart,limit=1] Pos" | grep -aoE '\[.*\]' | tail -1; }
  players=world/playerdata
else
  [ "$FAMILY" = 1.8 ] && { others='@e[type=!Player]'; cart="summon MinecartRideable"; sel=MinecartRideable; } \
    || { others='@e[type=!player]'; cart="summon minecraft:minecart"; sel=minecart; }
  # Legacy rail metadata: 0 N-S, 1 E-W, 4 / 5 ascending north / south, 6 SE, 7 SW, 8 NW, 9 NE.
  r() { case $1 in north_south) echo "minecraft:rail 0" ;; east_west) echo "minecraft:rail 1" ;;
        ascending_north) echo "minecraft:rail 4" ;; ascending_south) echo "minecraft:rail 5" ;;
        south_east) echo "minecraft:rail 6" ;; south_west) echo "minecraft:rail 7" ;;
        north_west) echo "minecraft:rail 8" ;; north_east) echo "minecraft:rail 9" ;; esac; }
  p() { case $1 in north_south) echo "minecraft:golden_rail 8" ;; east_west) echo "minecraft:golden_rail 9" ;; esac; }
  where_cart() { con "entitydata @e[type=$sel,c=1] {}" | grep -aoE 'Pos:\[[^]]*\]' | tail -1; }
  players=world/playerdata
fi
started=0
if ! lab_pid >/dev/null; then "$here/start.sh" "$VER" || exit 1; started=1; fi
"$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true
q "kill $others"
rm -f "$DIR/$players/$(offline_uuid "$BOT")".dat*
# The loop: redstone blocks under the powered stretches first, then the rails (corners last).
q "fill -11 3 -14 -8 3 -14 minecraft:redstone_block" "fill -4 3 -11 -4 3 -8 minecraft:redstone_block" \
  "fill -10 3 -4 -7 3 -4 minecraft:redstone_block" "fill -14 3 -7 -14 3 -6 minecraft:redstone_block" \
  "fill -13 4 -14 -5 4 -14 $(r east_west)" "fill -13 4 -4 -5 4 -4 $(r east_west)" \
  "fill -4 4 -13 -4 4 -5 $(r north_south)" "fill -14 4 -13 -14 4 -5 $(r north_south)" \
  "fill -11 4 -14 -8 4 -14 $(p east_west)" "fill -10 4 -4 -7 4 -4 $(p east_west)" \
  "fill -4 4 -11 -4 4 -8 $(p north_south)" "fill -14 4 -7 -14 4 -6 $(p north_south)" \
  "setblock -14 4 -10 minecraft:stone" "setblock -14 5 -10 $(r north_south)" \
  "setblock -14 4 -11 $(r ascending_south)" "setblock -14 4 -9 $(r ascending_north)" \
  "setblock -14 4 -14 $(r south_east)" "setblock -4 4 -14 $(r south_west)" \
  "setblock -4 4 -4 $(r north_west)" "setblock -14 4 -4 $(r north_east)"
q "$cart -12.5 4.1 -13.5"
since=$(date +%H:%M:%S)
log="$DIR/logs/carts.log"
( sleep 4; q "gamemode survival $BOT" "clear $BOT" "tp $BOT -12.5 4 -11.6 180 30"
  for _ in $(seq 1 240); do grep -aq 'dismounted after\|FAIL' "$log.out" 2>/dev/null && break; sleep 0.5; done
  sleep 0.5; echo "server cart $(where_cart)" > "$log.server" ) &
rc=0
GHAST_TRACE=1 GHAST_PROBE=cart timeout -s TERM 150 "$bin" --host 127.0.0.1 --port "$PORT" --username "$BOT" \
  --version "$GAME" --minutes 1.6 --entities -v > "$log.out" 2> "$log" || rc=$?
wait
echo "$GAME cart: exit $rc; $(grep -a 'boarded' "$log.out" | head -1 | cut -c1-50)..."
grep -a 'dismounted\|PASS\|FAIL' "$log.out" | sed 's/^/  /'
echo "  cart laps (positions every 20 ticks): $(grep -ac 'cart at' "$log")"
echo "  last: $(grep -a 'cart at' "$log" | tail -1 | sed 's/.*cart at //')"
echo "  $(cat "$log.server" 2>/dev/null)"
echo "  player teleports after the start: $(grep -a 'teleport .* we had sent' "$log" | grep -avc 'teleport -12.5 4.0 -11.6 ')"
f=$("$here/alerts.sh" "$VER" --count --since "$since"); echo "  ${f:-0 Grim flags}"
q "kill $others"
[ $started = 1 ] && { "$here/stop.sh" "$VER" >/dev/null 2>&1 || true; }
[ $rc = 0 ] && [ -z "$f" ]
