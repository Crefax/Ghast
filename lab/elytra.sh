#!/usr/bin/env bash
# 26.1 elytra under Grim: the elytra_flight example walks off a 37-block tower, starts gliding
# with a jump press in the air, glides with a slow turn, boosts with firework rockets, dives and
# lands. Passes when it glided, landed and Grim flagged nothing.
#   ./elytra.sh [rockets, default 2]
# Starts the 26.1 lab if it is not running and stops it again in that case.
# Needs: cargo build -p ghast --release --example elytra_flight
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select 26.1
rockets=${1:-2}
BOT=${BOT:-GhastBot10}
bin="$here/../target/release/examples/elytra_flight"
[ -x "$bin" ] || { echo "build it first: cargo build -p ghast --release --example elytra_flight" >&2; exit 2; }
started=0
if ! lab_pid >/dev/null; then "$here/start.sh" "$VER"; started=1; fi
"$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true
"$here/console.sh" "$VER" "fill 0 4 30 2 40 32 stone" >/dev/null
since=$(date +%H:%M:%S)
log="$DIR/logs/elytra-bot.log"
: > "$log"
setup() {
  sleep 4
  "$here/console.sh" "$VER" "gamemode survival $BOT" "clear $BOT" "give $BOT firework_rocket 16" \
    "item replace entity $BOT armor.chest with elytra" "tp $BOT 1.5 41 31.5 0 0" >/dev/null
}
setup &
rc=0
timeout -s TERM 150 "$bin" --host 127.0.0.1 --port "$PORT" --username "$BOT" --version 26.1 -v \
  --seed "$rockets" 2> "$log" || rc=$?
wait
grep -aE 'gliding at|rocket|landed|summary|no glide|session ended|WARN|ERROR' "$log" | sed 's/^/  [bot] /' || true
summary=$(grep -a 'summary:' "$log" | tail -1 || true)
"$here/console.sh" "$VER" "clear $BOT" "fill 0 4 30 2 40 32 air" >/dev/null
flags=$("$here/alerts.sh" "$VER" --count --since "$since")
if [ "$started" = 1 ]; then "$here/stop.sh" "$VER"; fi
fail=0
[ "$rc" = 0 ] || { echo "FAIL: bot exited with $rc"; fail=1; }
echo "$summary" | grep -q 'landed true' || { echo "FAIL: did not land"; fail=1; }
echo "$summary" | grep -qE 'summary: [1-9][0-9]* ticks gliding' || { echo "FAIL: did not glide"; fail=1; }
[ -z "$flags" ] || { echo "FAIL: Grim flags:"; echo "$flags"; fail=1; }
[ "$fail" = 0 ] && echo "PASS: tower take-off, glide, $rockets rocket(s), landing on 26.1, 0 Grim flags"
exit "$fail"
