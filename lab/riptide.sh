#!/usr/bin/env bash
# 26.1 riptide under Grim: the use_item example holds a riptide III trident (look at the sky, hold
# 40 ticks, release, repeat) standing in the test pad's water strip, then on grass in the rain.
# Passes when it launched in both (a fast climb in the bot's log) and Grim flagged nothing.
#   ./riptide.sh [minutes per phase, default 0.6]
# Starts the 26.1 lab if it is not running and stops it again in that case.
# Needs: cargo build -p ghast --release --example use_item
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select 26.1
mins=${1:-0.6}
BOT=${BOT:-GhastBot11}
bin="$here/../target/release/examples/use_item"
[ -x "$bin" ] || { echo "build it first: cargo build -p ghast --release --example use_item" >&2; exit 2; }
started=0
if ! lab_pid >/dev/null; then "$here/start.sh" "$VER"; started=1; fi
"$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true
# A launch from the water flies high: no fall damage, so the bot lives to try again.
"$here/console.sh" "$VER" "gamerule fall_damage false" >/dev/null
since=$(date +%H:%M:%S)
fail=0
for phase in water rain; do
  log="$DIR/logs/riptide-$phase.log"
  ( sleep 4
    if [ "$phase" = rain ]; then
      "$here/console.sh" "$VER" "weather rain" "tp $BOT 20.5 4 2.5 0 -60" >/dev/null
    else
      "$here/console.sh" "$VER" "weather clear" "tp $BOT 35.5 2 15.5 0 -60" >/dev/null
    fi
    "$here/console.sh" "$VER" "clear $BOT" "give $BOT trident[enchantments={riptide:3}]" >/dev/null ) &
  rc=0
  timeout -s TERM 120 "$bin" --host 127.0.0.1 --port "$PORT" --username "$BOT" --version 26.1 \
    --minutes "$mins" -v 2> "$log" || rc=$?
  wait
  uses=$(grep -ac 'using_item true' "$log" || true)
  launches=$(grep -ac 'launched:' "$log" || true)
  echo "$phase: $uses uses, $launches launches, exit $rc"
  [ "$launches" -gt 0 ] || { echo "FAIL: no launch in $phase"; fail=1; }
  [ "$rc" = 0 ] || fail=1
done
"$here/console.sh" "$VER" "weather clear" "clear $BOT" "gamerule fall_damage true" >/dev/null
sleep 1
flags=$("$here/alerts.sh" "$VER" --count --since "$since")
if [ "$started" = 1 ]; then "$here/stop.sh" "$VER"; fi
[ -z "$flags" ] || { echo "FAIL: Grim flags:"; echo "$flags"; fail=1; }
[ "$fail" = 0 ] && echo "PASS: riptide in water and in rain on 26.1, 0 Grim flags"
exit "$fail"
