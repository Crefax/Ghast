#!/usr/bin/env bash
# The client's block predictions under Grim, one probe run (crates/ghast/examples/block_probe.rs) per
# case: a break and a placement the server refuses (spawn protection, a non-op account) and accepts (an
# op), a broken waterlogged block (1.16 / 26.x), a block that a repeating command block puts back every
# tick (1.12+), drop and swap-hands (swap 1.12+). Each case prints the bot's view of the block (or its
# hands) after the action, the server's block and the Grim flags in that time.
#
#   ./predict.sh <1.8|1.12|1.16|26.1|26.2>      starts the lab with spawn-protection=16 and command blocks
#                                              (restarts it if it runs), stops it at the end
# Needs: cargo build --release -p ghast --example block_probe
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-}"
ex="$LAB_ROOT/../target/release/examples/block_probe"
[ -x "$ex" ] || { echo "build the probe first: cargo build --release -p ghast --example block_probe" >&2; exit 2; }
OP=GhastBot13      # opped: spawn protection does not apply
NOOP=GhastProbe    # not opped: refused inside the protected spawn area
con() { "$here/console.sh" "$VER" "$@"; }
q() { con "$@" >/dev/null; }

"$here/stop.sh" "$VER" >/dev/null 2>&1 || true
LAB_PROPS="spawn-protection=16 enable-command-block=true ${LAB_PROPS:-}" "$here/start.sh" "$VER" || exit 1
"$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true

# The placed block: cobblestone (1.8 / 1.12 predict only blocks without block-state properties).
if is_modern || [ "$VER" = 1.16 ]; then
  ns=minecraft:; stone=minecraft:stone; cobble=minecraft:cobblestone; air=minecraft:air
  test_block() { con "execute if block $1 $2" | grep -aoE 'Test (passed|failed)' | head -1; }
else
  stone="stone"; cobble="cobblestone"; air="air"
  test_block() { con "testforblock $1 $2" | grep -aoE 'Successfully found the block|The block at .*' | head -1; }
fi

# probe <label> <user> <mode> <at x,y,z> <face> <checked pos> <expected server block> [item to hold]
probe() {
  local label=$1 user=$2 mode=$3 at=$4 face=$5 pos=$6 want=$7 item=${8:-} since log out
  since=$(date +%H:%M:%S)
  log="$DIR/logs/predict-$label.log"
  ( sleep 2.5; q "tp $user 0.5 4 0.5 -90 30" "clear $user"; [ -n "$item" ] && q "give $user $item" ) &
  out=$(PROBE_MODE=$mode PROBE_AT=$at PROBE_FACE=$face timeout -s TERM 60 "$ex" --host 127.0.0.1 --port "$PORT" \
        --username "$user" --version "$GAME" --minutes 0.9 -v 2> "$log" | grep -a PROBE)
  wait
  sleep 1
  echo "$label: ${out:-no probe output (see $log)}"
  [ -n "$pos" ] && echo "  server $pos $want: $(test_block "$pos" "$want")"
  local f; f=$("$here/alerts.sh" "$VER" --count --since "$since")
  echo "  ${f:-0 Grim flags}"
}

for u in $OP $NOOP; do q "gamemode survival $u"; done
# Break refused / accepted: a stone at feet height in front of the bot (+X).
q "setblock 2 4 0 $stone"
probe dig-refused $NOOP dig 2,4,0 west "2 4 0" "$stone"
q "setblock 2 4 0 $stone"
probe dig-accepted $OP dig 2,4,0 west "2 4 0" "$air"

# Placement refused / accepted: on top of the grass at (2,3,0).
q "setblock 2 4 0 $air"
probe place-refused $NOOP place 2,3,0 up "2 4 0" "$air" "${ns:-}cobblestone 16"
q "setblock 2 4 0 $air"
probe place-accepted $OP place 2,3,0 up "2 4 0" "$cobble" "${ns:-}cobblestone 16"
q "setblock 2 4 0 $air"

if is_modern || [ "$VER" = 1.16 ]; then
  # A waterlogged block in a hole of the grass layer (the water cannot run off).
  q "setblock 2 3 0 minecraft:oak_stairs[waterlogged=true]"
  probe dig-waterlogged $OP dig 2,3,0 up "2 3 0" "minecraft:water"
  q "setblock 2 3 0 minecraft:grass_block"
fi

if [ "$VER" != 1.8 ]; then
  # A block put back every tick (a cobblestone generator's effect): the break is approved, the server's
  # update brings the block back.
  q "setblock 2 4 0 $cobble"
  if [ "$VER" = 1.12 ]; then
    q "setblock 5 1 5 repeating_command_block 0 replace {Command:\"setblock 2 4 0 cobblestone 0 keep\",auto:1b}"
  else
    q "setblock 5 1 5 minecraft:repeating_command_block{Command:\"setblock 2 4 0 minecraft:cobblestone keep\",auto:1b}"
  fi
  probe dig-regenerating $OP dig 2,4,0 west "2 4 0" "$cobble"
  q "setblock 5 1 5 ${ns:-}dirt" "setblock 2 4 0 $air"
fi

# Drop one of the held stack; swap hands (1.9+).
probe drop $OP drop 0,0,0 up "" "" "${ns:-}stone 16"
if [ "$VER" != 1.8 ]; then
  probe swap $OP swap 0,0,0 up "" "" "${ns:-}stone 16"
fi
q "clear $OP" "clear $NOOP" "kill @e[type=${ns:-}item]"
"$here/stop.sh" "$VER" >/dev/null 2>&1 || true
echo "lab $VER stopped"
