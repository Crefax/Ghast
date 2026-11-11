#!/usr/bin/env bash
# Server transfers (1.20.5+ `transfer`, play state) on the 26.1 lab: the afk example with
# --follow-transfers is sent back to the same server twice with the vanilla /transfer command
# (the template sets accepts-transfers=true). Passes when the bot followed both transfers (it
# logs in again with the transfer intent), stayed until its time limit and Grim flagged nothing.
#   ./transfer.sh
# Starts the 26.1 lab if it is not running and stops it again in that case.
# Needs the afk example: cargo build -p ghast --release --example afk
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select 26.1
BOT=${BOT:-GhastBot7}
afk="$here/../target/release/examples/afk"
[ -x "$afk" ] || { echo "build the afk example first: cargo build -p ghast --release --example afk" >&2; exit 2; }
started=0
if ! lab_pid >/dev/null; then "$here/start.sh" "$VER"; started=1; fi
grep -q '^accepts-transfers=true' "$DIR/server.properties" || { echo "restart the lab: server.properties predates accepts-transfers" >&2; exit 3; }
since=$(date +%H:%M:%S)
log="$DIR/logs/transfer-bot.log"
timeout -s TERM 150 "$afk" --host 127.0.0.1 --port "$PORT" --username "$BOT" --version 26.1 \
  --minutes 1.0 --follow-transfers 2> "$log" &
bot=$!
sleep 15; "$here/console.sh" "$VER" "transfer 127.0.0.1 $PORT $BOT" >/dev/null
sleep 20; "$here/console.sh" "$VER" "transfer 127.0.0.1 $PORT $BOT" >/dev/null
rc=0; wait "$bot" || rc=$?
grep -aE 'transfer|session|logged in|ERROR|WARN' "$log" | sed 's/^/  [bot] /' || true
followed=$(grep -ac 'following the transfer' "$log" || true)
joins=$(awk -v t="[$since" 'substr($0, 1, length(t)) >= t' "$DIR/logs/latest.log" | grep -ac "$BOT joined the game" || true)
flags=$("$here/alerts.sh" "$VER" --count --since "$since")
if [ "$started" = 1 ]; then "$here/stop.sh" "$VER"; fi
fail=0
[ "$rc" = 0 ] || { echo "FAIL: bot exited with $rc"; fail=1; }
[ "$followed" = 2 ] || { echo "FAIL: followed $followed transfers (want 2)"; fail=1; }
[ "$joins" = 3 ] || { echo "FAIL: $joins joins on the server (want 3)"; fail=1; }
[ -z "$flags" ] || { echo "FAIL: Grim flags:"; echo "$flags"; fail=1; }
[ "$fail" = 0 ] && echo "PASS: 2 transfers followed (3 logins), 0 Grim flags"
exit "$fail"
