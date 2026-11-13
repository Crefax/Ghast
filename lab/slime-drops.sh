#!/usr/bin/env bash
# Idle drops onto the pad's slime strip (x 24..31, z 8..23) and onto a bed, then the bounce tail:
#   ./slime-drops.sh [26.1|26.2, default 26.2] [runs per height, default 2]
# Heights 8, 12, 20, 30 onto slime and 6 onto a bed, with the afk example (no input). Expected on
# 26.2 with Grim 2.3.74 / 2.3.75: 0 flags, except the 20-block drop: Simulation ~0.0417 on every
# drop (Grim corner case 3 in crates/ghast-client/README.md; the real client sends the same
# packets, oracle scenario slime_drop_20_long). A change of that line means Grim changed.
# PASS = exactly the expected flags. Starts the lab if it is not running and stops it again then.
# Needs: cargo build -p ghast --release --example afk
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-26.2}"
runs=${2:-2}
BOT=${BOT:-GhastBot6}
bin="$here/../target/release/examples/afk"
[ -x "$bin" ] || { echo "build it first: cargo build -p ghast --release --example afk" >&2; exit 2; }
started=0
if ! lab_pid >/dev/null; then "$here/start.sh" "$VER"; started=1; fi
"$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true
con() { "$here/console.sh" "$VER" "$@" >/dev/null; }
con "setblock 4 4 30 red_bed[facing=north,part=foot]" "setblock 4 4 29 red_bed[facing=north,part=head]"
fail=0
drop() { # drop <label> <x> <y> <z> <expected flag lines per run>
  local label=$1 x=$2 y=$3 z=$4 want=$5 since n
  since=$(date +%H:%M:%S)
  for k in $(seq 1 "$runs"); do
    ( sleep 5; con "tp $BOT $x $y $z 0 0" ) &
    timeout -s TERM 70 "$bin" --host 127.0.0.1 --port "$PORT" --version "$GAME" --username "$BOT" \
      --minutes 0.5 > /dev/null 2> "$DIR/logs/slime-drops-$label-$k.log" || true
    wait
  done
  n=$(awk -v s="[$since" 'substr($0, 1, length(s)) >= s' "$DIR/logs/latest.log" | grep -ac "$BOT failed" || true)
  echo "$label: $runs drops, $n Grim flag lines (expected $((want * runs)))"
  [ "$n" = "$((want * runs))" ] || fail=1
}
expect20=0; [ "$VER" = 26.2 ] && expect20=1
drop slime8 27.5 12 15.5 0
drop slime12 27.5 16 15.5 0
drop slime20 28.5 24 18.5 "$expect20"
drop slime30 28.5 34 18.5 0
drop bed6 4.5 10 30.5 0
con "setblock 4 4 30 air" "setblock 4 4 29 air"
[ "$started" = 1 ] && "$here/stop.sh" "$VER" >/dev/null
[ "$fail" = 0 ] && echo "PASS: slime / bed drops on $GAME, flags as expected"
exit "$fail"
