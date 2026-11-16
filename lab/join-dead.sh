#!/usr/bin/env bash
# Joining while already dead (1.16 / 26.x): the bot dies with the death screen on, logs out dead, and
# logs in again with the immediate-respawn rule on: it must ask to respawn by itself (vanilla's screen
# check) and be alive afterwards. PASS = "Health" > 0 on the server, 0 Grim flags.
#   ./join-dead.sh <1.16|26.1|26.2>
# Needs: cargo build --release -p ghast --example block_probe (it never respawns by itself)
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-26.1}"
ex="$LAB_ROOT/../target/release/examples/block_probe"
run() { PROBE_MODE=swap PROBE_WAIT=200 PROBE_TRIES=1 timeout -s TERM 60 "$ex" --host 127.0.0.1 --port "$PORT" \
  --username "$BOT" --version "$GAME" --minutes 0.3 -v 2> "$1"; }
BOT=GhastBot13
if is_modern; then rule=immediate_respawn; else rule=doImmediateRespawn; fi
q() { "$here/console.sh" "$VER" "$@" >/dev/null; }
"$here/stop.sh" "$VER" >/dev/null 2>&1 || true
"$here/start.sh" "$VER" || exit 1
"$here/reset-field.sh" "$VER" >/dev/null 2>&1 || true
since=$(date +%H:%M:%S)
q "gamerule $rule false"
( sleep 3; q "kill $BOT" ) &
run "$DIR/logs/join-dead-1.log" >/dev/null
wait
echo "first session: $(grep -ac '\] t[0-9]* died' "$DIR/logs/join-dead-1.log") death(s), $(grep -ac 'respawn:' "$DIR/logs/join-dead-1.log") respawn(s): left dead"
q "gamerule $rule true"
( sleep 6; "$here/console.sh" "$VER" "data get entity $BOT Health" | grep -a "entity data" | sed 's/^/  [server] /' ) &
run "$DIR/logs/join-dead-2.log" >/dev/null
wait
echo "second session: $(grep -ac '\] t[0-9]* died' "$DIR/logs/join-dead-2.log") death(s) at join, $(grep -ac 'respawn:' "$DIR/logs/join-dead-2.log") respawn(s)"
f=$("$here/alerts.sh" "$VER" --count --since "$since"); echo "${f:-0 Grim flags}"
q "gamerule $rule false"
"$here/stop.sh" "$VER" >/dev/null 2>&1 || true
