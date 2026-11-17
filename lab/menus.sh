#!/usr/bin/env bash
# Menus under Grim (1.8.9, 1.12.2, 1.16.5) with the menu_probe example: each scene puts a block
# (at 0 4 2, in front of the spawn pad) or a NoAI mob (at 0.5 4 2.5) there, gives the bot its
# items (hotbar slots 0, 1, ... in order), opens the screen and runs a click script; every click
# must be confirmed as predicted, unchanged 4 ticks later, and equal to the server's contents
# after a reopen (`v`).
#   villager  payments in, take the output, shift-take it, select the second trade, payments out
#   brewing   blaze powder, glass bottles, nether wart in by shift-click, then out
#   enchant   a sword and lapis in, then out
#   beacon    an iron ingot in, then out
#   furnace   coal, a bucket and iron ore by shift-click (the furnace starts burning: its fuel
#             slot then changes by the server's own tick), then out
#   smoker    1.16: coal, a bucket and a porkchop
#   horse     a tamed horse's inventory (sneak + right click, no set_mount_rules): saddle and
#             iron horse armor in, then out
#   llama     1.12 / 1.16: a tamed llama, a carpet in, then out
#   stonecutter, smithing, grindstone, loom   1.16: inputs, the recipe / pattern button, take
#   ./menus.sh <1.8|1.12|1.16> [scenes]
# Needs: cargo build -p ghast --release --example menu_probe
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-}"; shift || true
BOT=${BOT:-GhastBot3}
bin="$here/../target/release/examples/menu_probe"
[ -x "$bin" ] || { echo "build it first: cargo build -p ghast --release --example menu_probe" >&2; exit 2; }
case "$FAMILY" in
  1.8)  all="villager brewing enchant beacon furnace horse"; others='@e[type=!Player]' ;;
  1.12) all="villager brewing enchant beacon furnace horse llama"; others='@e[type=!player]' ;;
  1.16) all="villager brewing enchant beacon furnace smoker horse llama stonecutter smithing grindstone loom"; others='@e[type=!player]' ;;
  *) echo "menus.sh: 1.8, 1.12 or 1.16" >&2; exit 2 ;;
esac
scenes=${*:-$all}
con() { "$here/console.sh" "$VER" "$@"; }
q() { con "$@" >/dev/null; }
# Block names: 1.8 / 1.12 ids with data values, 1.16 block states.
blk() { # blk <1.8/1.12 form> <1.16 form>
  if [ "$FAMILY" = 1.16 ]; then echo "$2"; else echo "$1"; fi
}
trades8='{Recipes:[{buy:{id:"minecraft:emerald",Count:1b},sell:{id:"minecraft:bread",Count:6b},maxUses:99,rewardExp:0b},{buy:{id:"minecraft:emerald",Count:2b},sell:{id:"minecraft:apple",Count:1b},maxUses:99,rewardExp:0b}]}'
trades16='{Recipes:[{buy:{id:"minecraft:emerald",Count:1b},sell:{id:"minecraft:bread",Count:6b},maxUses:99,rewardExp:0b,xp:0},{buy:{id:"minecraft:emerald",Count:2b},sell:{id:"minecraft:apple",Count:1b},maxUses:99,rewardExp:0b,xp:0}]}'
started=0
# Horses and llamas are animals, villagers NPCs: the lab defaults spawn-animals=false (they are
# removed at once) and spawn-npcs=false (1.12+ refuses the summon).
if ! lab_pid >/dev/null; then LAB_PROPS="spawn-animals=true spawn-npcs=true ${LAB_PROPS:-}" "$here/start.sh" "$VER" || exit 1; started=1
elif ! grep -q '^spawn-npcs=true' "$DIR/server.properties"; then
  echo "note: the running lab has spawn-animals / spawn-npcs off (mob scenes need a restart through this script)" >&2; fi
fail=0
for scene in $scenes; do
  "$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true
  q "kill $others" "setblock 0 4 2 $(blk minecraft:air minecraft:air)"
  rm -f "$DIR"/world/playerdata/"$(offline_uuid "$BOT")".dat*
  block=0,4,2; sneak=0; setup=(); give=(); returns=0; live=""; aim=""; result=""
  case $scene in
    villager)
      block=""
      case $FAMILY in
        1.8)  setup=("summon Villager 0.5 4 2.5 {NoAI:1,Profession:0,Offers:$trades8}") ;;
        1.12) setup=("summon minecraft:villager 0.5 4 2.5 {NoAI:1b,Profession:0,Career:1,CareerLevel:99,Offers:$trades8}") ;;
        1.16) setup=("summon minecraft:villager 0.5 4 2.5 {NoAI:1b,VillagerData:{profession:\"minecraft:farmer\",level:2,type:\"minecraft:plains\"},Offers:$trades16}") ;;
      esac
      give=("emerald 64" "emerald 16")
      steps="ph0 p0 p2 ph2 q2 t1 w5 q0 q1 v" ;;
    brewing)
      setup=("setblock 0 4 2 $(blk 'minecraft:brewing_stand 0' minecraft:brewing_stand)")
      # 1.8 / 1.12: the stand's ray shape is its 1/8-high base (1.16: base and rod).
      aim=0.06
      give=("blaze_powder 4" "glass_bottle 3" "nether_wart 4")
      # 1.9+: the stand takes a blaze powder from the fuel slot on its own tick.
      if [ "$FAMILY" = 1.8 ]; then steps="qh0 qh1 qh2 v q0 q1 q2 q3 v"
      else steps="qh0 qh1 qh2 v q0 q1 q2 q3 q4 v"; live=4; fi ;;
    enchant)
      setup=("setblock 0 4 2 minecraft:enchanting_table")
      if [ "$FAMILY" = 1.16 ]; then give=("diamond_sword 1" "lapis_lazuli 16"); else give=("diamond_sword 1" "dye 16 4"); fi
      steps="qh0 qh1 q0 q1 qh0 qh1 v"; returns=1 ;;
    beacon)
      setup=("setblock 0 4 2 minecraft:beacon")
      give=("iron_ingot 3")
      steps="qh0 v q0 v" ;;
    furnace)
      setup=("setblock 0 4 2 $(blk 'minecraft:furnace 2' 'minecraft:furnace[facing=north]')")
      give=("coal 8" "iron_ore 4" "bucket 1")
      steps="qh0 qh2 qh1 v q0 q1 v"; live=0,1,2 ;;
    smoker)
      setup=("setblock 0 4 2 minecraft:smoker[facing=north]")
      give=("coal 8" "porkchop 4" "bucket 1")
      steps="qh0 qh2 qh1 v q0 q1 v"; live=0,1,2 ;;
    horse)
      block=""; sneak=1
      case $FAMILY in
        1.8)  setup=("summon EntityHorse 0.5 4 2.5 {NoAI:1,Tame:1,Type:0}") ;;
        *)    setup=("summon minecraft:horse 0.5 4 2.5 {NoAI:1b,Tame:1b}") ;;
      esac
      give=("saddle 1" "iron_horse_armor 1")
      steps="qh0 qh1 v q0 q1 v" ;;
    llama)
      block=""; sneak=1
      setup=("summon minecraft:llama 0.5 4 2.5 {NoAI:1b,Tame:1b}")
      if [ "$FAMILY" = 1.16 ]; then give=("white_carpet 2"); else give=("carpet 2 0"); fi
      steps="qh0 v q1 v" ;;
    stonecutter)
      setup=("setblock 0 4 2 minecraft:stonecutter[facing=north]")
      give=("stone 4")
      steps="qh0 b0 p1 ph0 v"; returns=1; result=1 ;;
    smithing)
      setup=("setblock 0 4 2 minecraft:smithing_table")
      give=("diamond_sword 1" "netherite_ingot 1")
      steps="qh0 qh1 p2 ph0 v"; returns=1; result=2 ;;
    grindstone)
      setup=("setblock 0 4 2 minecraft:grindstone[face=floor,facing=north]")
      give=("diamond_sword{Damage:500} 1" "diamond_sword{Damage:900} 1")
      steps="qh0 qh1 p2 ph0 v"; returns=1; result=2 ;;
    loom)
      setup=("setblock 0 4 2 minecraft:loom[facing=north]")
      give=("white_banner 1" "red_dye 4")
      steps="qh0 qh1 b1 p3 ph0 v"; returns=1; result=3 ;;
    *) echo "unknown scene $scene" >&2; continue ;;
  esac
  for c in "${setup[@]}"; do q "$c"; done
  since=$(date +%H:%M:%S)
  log="$DIR/logs/menus-$scene.log"
  ( sleep 1; q "gamemode survival $BOT" "clear $BOT" "tp $BOT 0.5 4 0.5 0 0"
    for g in "${give[@]}"; do q "give $BOT $g"; done ) &
  env=(GHAST_MENU_STEPS="$steps")
  if [ -n "$block" ]; then env+=(GHAST_MENU_BLOCK="$block"); else env+=(GHAST_MENU_ENTITY=1); fi
  [ $sneak = 1 ] && env+=(GHAST_MENU_SNEAK=1)
  [ $returns = 1 ] && env+=(GHAST_MENU_RETURNS=1)
  [ -n "$live" ] && env+=(GHAST_MENU_LIVE="$live")
  [ -n "$aim" ] && env+=(GHAST_MENU_AIM_Y="$aim")
  [ -n "$result" ] && env+=(GHAST_MENU_RESULT="$result")
  rc=0
  env "${env[@]}" GHAST_TRACE=1 timeout -s TERM 90 "$bin" --host 127.0.0.1 --port "$PORT" \
    --username "$BOT" --version "$GAME" -v > "$log.out" 2> "$log" || rc=$?
  wait
  f=$("$here/alerts.sh" "$VER" --count --since "$since")
  tps=$(grep -a 'teleport .* we had sent' "$log" | grep -avc 'teleport 0.5 4.0 0.5 ')
  echo "$scene: exit $rc; setbacks $tps; ${f:-0 Grim flags}; $(grep -a 'clicks sent' "$log.out")"
  grep -a 'opened\|FAIL\|differing\|trade uses' "$log.out" | sed 's/^/  /'
  [ $rc = 0 ] && [ -z "$f" ] && [ "$tps" = 0 ] || fail=1
done
q "kill $others" "setblock 0 4 2 minecraft:air"
[ $started = 1 ] && { "$here/stop.sh" "$VER" >/dev/null 2>&1 || true; }
exit $fail
