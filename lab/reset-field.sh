#!/usr/bin/env bash
# Re-apply world rules and rebuild the test pad: ./reset-field.sh <1.8|1.12|1.16|26.1> [--tp] [--fly]
#   --tp   teleport $LAB_BOT (default GhastBot) to the spawn pad 0.5 4 0.5
#   --fly  /fly $LAB_BOT on
# Superflat: bedrock y0, dirt y1-2, grass y3; players stand at y=4. Pad x -16..47, z -16..47 (spawn chunks).
# Physics strips at y=3 (floor) / y=4 (on top), each 8 wide in x, from z 8 to 23:
#   x 0..7 ice | 8..15 packed ice | 16..23 soul sand | 24..31 slime | 32..39 water pool 2 deep (y 2-3)
#   x 40..47: step-up course along z: z 8 bottom slab, z 10 full block, z 12 two-high wall, z 14 fence
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-}"; shift || true
tp=0; fly=0
for a in "$@"; do case "$a" in --tp) tp=1 ;; --fly) fly=1 ;; *) echo "unknown arg $a" >&2; exit 2 ;; esac; done
base=$'gamerule doDaylightCycle false\ngamerule doMobSpawning false\ngamerule doFireTick false\ngamerule mobGriefing false\ngamerule keepInventory true\ngamerule logAdminCommands false\ngamerule randomTickSpeed 0'
if [ "$VER" = 26.1 ]; then
  # 26.x: registry game rules with snake_case names; no always-loaded spawn chunks (force-load the pad).
  grass=minecraft:grass_block; slime=minecraft:slime_block; item=item; slab="minecraft:smooth_stone_slab[type=bottom]"; fence=minecraft:oak_fence; water=minecraft:water
  base=$'forceload add -16 -16 47 47\ngamerule advance_time false\ngamerule spawn_mobs false\ngamerule fire_spread_radius_around_player 0\ngamerule mob_griefing false\ngamerule keep_inventory true\ngamerule log_admin_commands false\ngamerule random_tick_speed 0'
  rules=$'gamerule respawn_radius 0\ngamerule advance_weather false\ngamerule spawn_phantoms false\ngamerule spawn_patrols false\ngamerule spawn_wandering_traders false\ngamerule show_advancement_messages false\nweather clear'
elif [ "$VER" = 1.8 ]; then
  grass=minecraft:grass; slime=minecraft:slime; item=Item; slab="minecraft:stone_slab 0"; fence=minecraft:fence; water="minecraft:water 0"
  rules="weather clear 1000000"
elif [ "$VER" = 1.12 ]; then
  # 1.12: pre-flattening block names like 1.8, lower-case entity ids, 1.9+ game rules.
  grass=minecraft:grass; slime=minecraft:slime; item=item; slab="minecraft:stone_slab 0"; fence=minecraft:fence; water="minecraft:water 0"
  rules=$'gamerule spawnRadius 0\ngamerule doWeatherCycle false\ngamerule announceAdvancements false\nweather clear 1000000'
else
  grass=minecraft:grass_block; slime=minecraft:slime_block; item=item; slab="minecraft:smooth_stone_slab[type=bottom]"; fence=minecraft:oak_fence; water=minecraft:water
  rules=$'gamerule spawnRadius 0\ngamerule doWeatherCycle false\ngamerule doInsomnia false\ngamerule doPatrolSpawning false\ngamerule doTraderSpawning false\ngamerule announceAdvancements false\nweather clear'
fi
{
  cat <<CMDS
$base
$rules
time set 6000
setworldspawn 0 4 0
fill -16 4 -16 47 10 47 minecraft:air
fill -16 11 -16 47 17 47 minecraft:air
fill -16 18 -16 47 24 47 minecraft:air
fill -16 3 -16 47 3 47 $grass
fill -16 1 -16 47 2 47 minecraft:dirt
fill 0 3 8 7 3 23 minecraft:ice
fill 8 3 8 15 3 23 minecraft:packed_ice
fill 16 3 8 23 3 23 minecraft:soul_sand
fill 24 3 8 31 3 23 $slime
fill 32 2 8 39 3 23 $water
fill 40 4 8 47 4 8 $slab
fill 40 4 10 47 4 10 minecraft:stone
fill 40 4 12 47 5 12 minecraft:stone
fill 40 4 14 47 4 14 $fence
kill @e[type=$item]
CMDS
  [ "$tp" = 1 ] && echo "tp $LAB_BOT 0.5 4 0.5"
  [ "$fly" = 1 ] && echo "fly $LAB_BOT on"
  true
} | sed '/^$/d' | "$here/console.sh" "$VER" | tee "$DIR/reset.out" | grep -vE 'Game ?rule .*(updated|set to)|[Mm]arked .* to be force loaded|already marked|blocks? (filled|placed)|filled [0-9]+ blocks|No blocks were filled|No entity was found' || true
if grep -q 'outside of the world' "$DIR/reset.out"; then
  echo "pad not loaded: the spawn chunks are elsewhere. ./stop.sh $VER && ./start.sh $VER, then retry" >&2; exit 1
fi
