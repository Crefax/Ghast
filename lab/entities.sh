#!/usr/bin/env bash
# 26.1 entity checks under Grim, with summoned NoAI mobs (the lab spawns none by itself):
#   crowd   walk through a corridor of NoAI cows into an oak boat, entity tracking ON (must be 0
#           flags), then the same with tracking OFF (reported: the client then neither collides
#           with nor is pushed by them)
#   horse   sneak-right-click a tamed horse: its inventory, saddle out and back (clicks confirmed)
#   villager right-click a villager with one emerald -> bread offer: trade screen, select_trade,
#           take the result (clicks confirmed)
#   border  walk into the world border's edge and keep walking while it shrinks past the bot
#           (0 flags), border reset
#   teams   two bots walk through each other, without a team and in a collisionRule-never team
#           (0 flags both ways; with the team they pass straight)
#   boat    board an oak boat on the water strip (tracking on: right click; off: `ride`), paddle
#           straight / left / right onto land, sneak off (0 flags; Grim checks vehicle moves)
#   cart    ride a minecart along powered rails (seated by `ride`), sneak off (0 flags)
#   ./entities.sh [phase ...]   (default: crowd horse villager border)
# Starts the 26.1 lab if it is not running and stops it again in that case.
# Needs: cargo build -p ghast --release --example entity_probe
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select 26.1
BOT=${BOT:-GhastBot11}
bin="$here/../target/release/examples/entity_probe"
[ -x "$bin" ] || { echo "build it first: cargo build -p ghast --release --example entity_probe" >&2; exit 2; }
phases=("$@"); [ ${#phases[@]} -gt 0 ] || phases=(crowd horse villager border)
started=0
if ! lab_pid >/dev/null; then "$here/start.sh" "$VER"; started=1; fi
"$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true
con() { "$here/console.sh" "$VER" "$@" >/dev/null; }
clean() { con "kill @e[type=!player]" "worldborder center 0 0" "worldborder set 59999968"; }
clean
fail=0
# probe <mode> <seconds> [args]: runs the example under a hard timeout, output in the log.
probe() {
  local mode=$1 secs=$2; shift 2
  local log="$DIR/logs/entities-$mode$*.log"; log=${log// /}
  local rc=0
  GHAST_PROBE=$mode timeout -s TERM $((secs + 60)) "$bin" --host 127.0.0.1 --port "$PORT" \
    --username "$BOT" --version 26.1 --minutes "$(awk "BEGIN{print $secs/60}")" "$@" -v > "$log.out" 2> "$log" || rc=$?
  cat "$log.out"
  return $rc
}
flags_since() { "$here/alerts.sh" "$VER" --count --since "$1"; }
crowd_scene() {
  clean
  for z in -10 -9 -8 -7 -6; do
    con "summon cow -7.6 4 $z.3 {NoAI:1b,PersistenceRequired:1b,Silent:1b}"
  done
  con "summon cow -8.3 4 -8.6 {NoAI:1b,PersistenceRequired:1b,Silent:1b}" \
      "summon oak_boat -8 4 -2 {Rotation:[90f,0f]}"
}
for phase in "${phases[@]}"; do
  case "$phase" in
    crowd)
      for tracking in on off; do
        crowd_scene
        since=$(date +%H:%M:%S)
        ( sleep 4; con "tp $BOT -7.7 4 -14 0 0" ) &
        args=(); [ "$tracking" = on ] && args=(--entities)
        probe walk 12 "${args[@]}" || fail=1
        wait
        f=$(flags_since "$since")
        echo "crowd, entity tracking $tracking: ${f:-0 Grim flags}"
        if [ "$tracking" = on ] && [ -n "$f" ]; then echo "FAIL: flags with tracking on"; fail=1; fi
      done ;;
    horse)
      clean
      since=$(date +%H:%M:%S)
      con "summon horse 4.5 4 -8.5 {NoAI:1b,PersistenceRequired:1b,Tame:1b,Silent:1b}" \
          "item replace entity @e[type=horse,limit=1] saddle with saddle"
      ( sleep 4; con "clear $BOT" "tp $BOT 4.5 4 -11.5 0 20" ) &
      probe horse 40 || { echo "FAIL: horse"; fail=1; }
      wait
      f=$(flags_since "$since"); [ -z "$f" ] || { echo "FAIL: Grim flags: $f"; fail=1; } ;;
    villager)
      clean
      since=$(date +%H:%M:%S)
      con "summon villager 10.5 4 -8.5 {NoAI:1b,PersistenceRequired:1b,Silent:1b,VillagerData:{profession:\"minecraft:farmer\",level:2,type:\"minecraft:plains\"},Offers:{Recipes:[{buy:{id:\"minecraft:emerald\",count:1},sell:{id:\"minecraft:bread\",count:6},maxUses:99,rewardExp:0b}]}}"
      ( sleep 4; con "clear $BOT" "give $BOT emerald 4" "tp $BOT 10.5 4 -11.5 0 20" ) &
      probe villager 40 || { echo "FAIL: villager"; fail=1; }
      wait
      f=$(flags_since "$since"); [ -z "$f" ] || { echo "FAIL: Grim flags: $f"; fail=1; } ;;
    border)
      clean
      since=$(date +%H:%M:%S)
      # Walk into the edge (z 6), keep pressing while it shrinks past (26.x: lerp time in ticks).
      ( sleep 4; con "tp $BOT -7.5 4 -14 0 0"; sleep 1
        con "worldborder center -8 -6" "worldborder set 24"; sleep 7
        con "worldborder set 10 200" ) &
      probe walk 24 || fail=1
      wait
      f=$(flags_since "$since")
      echo "border: ${f:-0 Grim flags}"
      [ -z "$f" ] || { echo "FAIL: flags at the border"; fail=1; } ;;
    teams)
      # Two bots walk through each other along z (entity tracking on): first without a team
      # (they push each other aside), then both in a team with collisionRule never (no push).
      BOT2=${BOT2:-GhastBot13}
      for rule in none never; do
        clean
        con "team remove ghastlab" || true
        if [ "$rule" = never ]; then
          con "team add ghastlab" "team modify ghastlab collisionRule never"
        fi
        since=$(date +%H:%M:%S)
        # Members join once online (the bots then get the team update packet).
        ( sleep 4; con "tp $BOT -7.5 4 -14 0 0" "tp $BOT2 -7.4 4 2 180 0"
          [ "$rule" = never ] && con "team join ghastlab @a" ) &
        GHAST_YAW=180 probe walk 10 --entities --username "$BOT2" > "$DIR/logs/entities-teams-$rule-2.txt" &
        p2=$!
        probe walk 10 --entities || fail=1
        wait $p2 || fail=1
        wait
        echo "teams $rule: $BOT $(grep 'end position' "$DIR/logs/entities-walk--entities.log.out" | tail -1), $BOT2 $(grep 'end position' "$DIR/logs/entities-teams-$rule-2.txt" | tail -1)"
        f=$(flags_since "$since")
        echo "teams $rule: ${f:-0 Grim flags}"
        [ -z "$f" ] || { echo "FAIL: flags with teams $rule"; fail=1; }
      done
      con "team remove ghastlab" || true ;;
    boat)
      # Board an oak boat (tracking on: a right click; off: the server's `ride` command),
      # paddle straight / left / right against the pool's edge, then on the ice strip, sneak
      # to dismount.
      for scene in water:on water:off ice:off; do
        IFS=: read -r where tracking <<< "$scene"
        clean
        if [ "$where" = water ]; then at="35.5 4 10.5"; stand="35.5 4 7.6"; else at="3.5 4 10.5"; stand="3.5 4 7.6"; fi
        con "summon oak_boat $at {Rotation:[0f,0f]}"
        since=$(date +%H:%M:%S)
        if [ "$tracking" = on ]; then
          ( sleep 4; con "tp $BOT $stand 0 30" ) &
          probe boat 40 --entities || { echo "FAIL: boat on $where (tracking on)"; fail=1; }
        else
          ( sleep 4; con "tp $BOT $stand 0 30"; sleep 3; con "ride $BOT mount @e[type=oak_boat,limit=1]" ) &
          probe boat 40 || { echo "FAIL: boat on $where (tracking off)"; fail=1; }
        fi
        wait
        f=$(flags_since "$since")
        echo "boat on $where, tracking $tracking: ${f:-0 Grim flags}"
        [ -z "$f" ] || { echo "FAIL: flags in the boat"; fail=1; }
      done ;;
    cart)
      # Ride a minecart the server moves along powered rails (seated by `ride`; tracking on and
      # off), sneak to get off.
      for tracking in on off; do
        clean
        con "fill -12 3 -15 -12 3 20 minecraft:grass_block" "fill -12 4 -15 -12 4 20 minecraft:air" "setblock -12 4 -15 minecraft:stone" \
            "fill -12 4 -14 -12 4 20 minecraft:rail[shape=north_south]"
        for z in -14 -8 -2 4 10; do
          con "setblock -12 4 $z minecraft:powered_rail[shape=north_south]"
        done
        con "summon minecart -11.5 4.1 -13.5"
        since=$(date +%H:%M:%S)
        # Seated first, then the rails are powered (the cart starts off the stone behind it).
        ( sleep 4; con "tp $BOT -10.5 4 -13.5 0 30"; sleep 3; con "ride $BOT mount @e[type=minecart,limit=1]"
          sleep 2; for z in -14 -8 -2 4 10; do con "setblock -12 3 $z minecraft:redstone_block"; done ) &
        args=(); [ "$tracking" = on ] && args=(--entities)
        probe boat 40 "${args[@]}" || { echo "FAIL: cart (tracking $tracking)"; fail=1; }
        wait
        f=$(flags_since "$since")
        echo "cart, tracking $tracking: ${f:-0 Grim flags}"
        [ -z "$f" ] || { echo "FAIL: flags in the cart"; fail=1; }
      done
      con "fill -12 3 -15 -12 3 20 minecraft:grass_block" "fill -12 4 -15 -12 4 20 minecraft:air" ;;
    *) echo "unknown phase $phase" >&2; exit 2 ;;
  esac
done
clean
con "clear $BOT"
if [ "$started" = 1 ]; then "$here/stop.sh" "$VER"; fi
[ "$fail" = 0 ] && echo "PASS: ${phases[*]}"
exit "$fail"
