#!/usr/bin/env bash
# Item use while walking (the movement slowdown from the item's use effects): the use_item
# example holds the right button with a golden apple (eaten), a shield (blocking) and a bow
# with arrows (drawn, released after 40 ticks) in turn, walking forward meanwhile.
#   ./use-items.sh [1.8|1.12|1.16|26.1, default 26.1] [minutes per item, default 0.6]
# Passes when Grim flagged nothing (NoSlow catches a wrong speed multiplier).
# Starts the lab if it is not running and stops it again in that case.
# Needs: cargo build -p ghast --release --example use_item
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-26.1}"
mins=${2:-0.6}
case "$VER" in 1.8) GAME=1.8.9 ;; 1.12) GAME=1.12.2 ;; 1.16) GAME=1.16.5 ;; 26.1) GAME=26.1 ;; esac
BOT=${BOT:-GhastBot2}
bin="$here/../target/release/examples/use_item"
[ -x "$bin" ] || { echo "build it first: cargo build -p ghast --release --example use_item" >&2; exit 2; }
started=0
if ! lab_pid >/dev/null; then "$here/start.sh" "$VER"; started=1; fi
"$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true
since=$(date +%H:%M:%S)
fail=0
for item in golden_apple shield bow; do
  ( sleep 4
    "$here/console.sh" "$VER" "clear $BOT" "tp $BOT 0.5 4 30.5 0 0" >/dev/null
    if [ "$item" = bow ]; then "$here/console.sh" "$VER" "give $BOT bow 1" "give $BOT arrow 64" >/dev/null
    else "$here/console.sh" "$VER" "give $BOT $item 16" >/dev/null; fi ) &
  rc=0
  timeout -s TERM 120 "$bin" --host 127.0.0.1 --port "$PORT" --username "$BOT" --version "$GAME" \
    --minutes "$mins" 2> "$DIR/logs/use-items-$item.log" || rc=$?
  wait
  echo "$item: $(grep -ac 'using_item true' "$DIR/logs/use-items-$item.log" || true) use lines, exit $rc"
  [ "$rc" = 0 ] || fail=1
done
sleep 1
flags=$("$here/alerts.sh" "$VER" --count --since "$since")
if [ "$started" = 1 ]; then "$here/stop.sh" "$VER"; fi
[ -z "$flags" ] || { echo "FAIL: Grim flags:"; echo "$flags"; fail=1; }
[ "$fail" = 0 ] && echo "PASS: golden apple, shield, bow used while walking on $VER, 0 Grim flags"
exit "$fail"
