#!/usr/bin/env bash
# A sub-version lab (1.12.1, 1.16.2, 1.16.3, 1.16.4; any lab works) under Grim, four phases:
#   afk    - stands for 100 s (the server's keep-alives answered, no time-out)
#   course - movement_course for 2 minutes (walk / sprint / jump / sneak / step-up / ice / slime /
#            soul sand / water laps)
#   dig    - dig_place for 60 s (breaks the grass next to the spawn, places dirt back)
#   chest  - inventory_probe (opens a chest, clicks of every mode, compares the prediction)
# Per phase: exit code, the server's teleports other than the phase's own /tp (setbacks), Grim
# flags, and the server's disconnect lines for the bot (kicks show their reason there).
#   ./subversion-check.sh <version> [afk course dig chest]
# Needs: cargo build -p ghast --release --example afk --example movement_course \
#          --example dig_place --example inventory_probe
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-}"; shift || true
phases=${*:-afk course dig chest}
ex="$here/../target/release/examples"
for b in afk movement_course dig_place inventory_probe; do
  [ -x "$ex/$b" ] || { echo "build it first: cargo build -p ghast --release --example $b" >&2; exit 2; }
done
q() { "$here/console.sh" "$VER" "$@" >/dev/null; }
started=0
if ! lab_pid >/dev/null; then "$here/start.sh" "$VER" || exit 1; started=1; fi
if [ "$FAMILY" = 1.12 ]; then
  chest='setblock 0 4 2 chest 2 replace {Items:[{Slot:0b,id:"minecraft:stone",Count:64b},{Slot:1b,id:"minecraft:dirt",Count:5b},{Slot:2b,id:"minecraft:wool",Damage:1s,Count:10b},{Slot:3b,id:"minecraft:planks",Count:12b}]}'
else
  chest='setblock 0 4 2 minecraft:chest[facing=north]{Items:[{Slot:0b,id:"minecraft:stone",Count:64b},{Slot:1b,id:"minecraft:dirt",Count:5b},{Slot:2b,id:"minecraft:orange_wool",Count:10b},{Slot:3b,id:"minecraft:oak_planks",Count:12b}]}'
fi
fail=0
# Grim on this build: its enable line in the current server log (old builds may fail to load it).
if grep -aq 'GrimAC\] Enabling GrimAC' "$DIR/logs/latest.log"; then echo "Grim: loaded"; else
  echo "Grim: NOT loaded (flags below mean nothing): $(grep -a 'GrimAC' "$DIR/logs/latest.log" | grep -a 'ERROR' | head -1)"; fail=1; fi
for phase in $phases; do
  "$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true
  case $phase in
    afk) bot=GhastBot4; tp_to=none ;;
    course) bot=GhastBot2; tp_to="0.5 4.0 0.5" ;;
    dig) bot=GhastBot6; tp_to="0.5 4.0 0.5" ;;
    chest) bot=GhastBot3; tp_to="0.5 4.0 0.5" ;;
    *) echo "unknown phase $phase" >&2; continue ;;
  esac
  # Every phase joins at the spawn pad, not where an earlier run left the account.
  rm -f "$DIR"/world/playerdata/"$(offline_uuid "$bot")".dat*
  log="$DIR/logs/subversion-$phase.log"
  since=$(date +%H:%M:%S)
  common=(--host 127.0.0.1 --port "$PORT" --username "$bot" --version "$GAME" -v)
  rc=0
  case $phase in
    afk)
      GHAST_TRACE=1 timeout -s TERM 160 "$ex/afk" "${common[@]}" --minutes 1.67 > "$log.out" 2> "$log" || rc=$? ;;
    course)
      # --grim-compat by default: the seed-7 laps hop from soul sand onto the grass edge, the
      # known vanilla step-up Grim 2.3.73 sets back (COURSE_ARGS= for the vanilla stream).
      timeout -s TERM 200 "$ex/movement_course" "${common[@]}" --seed 7 --minutes 2 ${COURSE_ARGS---grim-compat} \
        > "$log.out" 2> "$log" || rc=$? ;;
    dig)
      # The dirt must be in hand before the first place (dig_place starts digging at tick 30).
      ( sleep 1; q "gamemode survival $bot" "clear $bot" "give $bot dirt 64" ) &
      timeout -s TERM 120 "$ex/dig_place" "${common[@]}" --minutes 1 > "$log.out" 2> "$log" || rc=$?
      wait ;;
    chest)
      q "$chest"
      # Before the probe aims (it waits 40 ticks): yaw exactly 0, which its whole mouse steps keep.
      ( sleep 1; q "gamemode survival $bot" "clear $bot" "tp $bot 0.5 4 0.5 0 0" "give $bot stone 30" \
          "give $bot dirt 60" "give $bot diamond_helmet 1" "give $bot ender_pearl 10" ) &
      GHAST_TRACE=1 timeout -s TERM 120 "$ex/inventory_probe" "${common[@]}" > "$log.out" 2> "$log" || rc=$?
      wait
      q "setblock 0 4 2 air" ;;
  esac
  login=$(grep -a 'logged in to' "$log" | head -1 | sed 's/.*logged in to //')
  tps=$(grep -a 'teleport .* we had sent' "$log" | grep -avc "teleport $tp_to ")
  f=$("$here/alerts.sh" "$VER" --count --since "$since")
  echo "$phase ($bot): exit $rc; login ${login:-NONE}; setbacks $tps; ${f:-0 Grim flags}"
  case $phase in
    # keep_alive is 0x1f in 1.12.1 - 1.16.5; a server kicks after 30 s without the answers.
    afk) echo "  server keep-alives received: $(grep -ac -- '<- 0x1f ' "$log"); $(grep -a 'session ended' "$log" | tail -1 | sed 's/.*INFO  //')" ;;
    course) echo "  legs arrived: $(grep -ac 'INFO  leg ' "$log"), alerts: $(grep -ac ALERT "$log")" ;;
    dig) grep -a 'time limit' "$log" | sed 's/^/  /' ;;
    chest) grep -a 'PASS\|FAIL\|accepted' "$log.out" | sed 's/^/  /' ;;
  esac
  awk -v s="[$since" 'substr($0, 1, length(s)) >= s' "$DIR/logs/latest.log" | grep -a "$bot lost connection\|$bot.*kick" | sed 's/^/  server: /'
  [ $rc = 0 ] && [ "$tps" = 0 ] && [ -z "$f" ] || fail=1
done
[ $started = 1 ] && { "$here/stop.sh" "$VER" >/dev/null 2>&1 || true; }
[ $fail = 0 ] && echo "PASS: $GAME sub-version lab ($phases)" || echo "FAIL: $GAME"
exit $fail
