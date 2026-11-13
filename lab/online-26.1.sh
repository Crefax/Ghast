#!/usr/bin/env bash
# Online mode on 26.1 without real accounts: a Paper 26.1.2 + Grim server with online-mode=true and
# enforce-secure-profile=true whose account services are lab/fake-session-server.py (authlib's
# -Dminecraft.api.*.host properties), and the online_chat example logging in through the same
# stand-in: encryption + session join (server hash checked by the server's hasJoined call), a
# profile key pair from player/certificates, chat_session_update, signed chat and signed commands.
#   ./online-26.1.sh
# Runs the bot twice: signed (must pass: no kick, chat logged as secure, 0 Grim flags) and with
# --unsigned (control: the server must refuse its chat). Everything lives in the git-ignored
# lab/servers/online-<26.1|26.2>/ (LAB_VER=26.2); the server and the stand-in are stopped by PID at the end.
# Needs: cargo build -p ghast --release --features microsoft --example online_chat
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${LAB_VER:-26.1}"   # LAB_VER=26.2: Paper 26.2
LAB26="$DIR"
D="$SERVERS/online-$VER"
PORT=25627; SVC=25700
BOT=${BOT:-GhastBot}
HEAP=${ONLINE_HEAP:-640M}
bin="$here/../target/release/examples/online_chat"
[ -x "$bin" ] || { echo "build it first: cargo build -p ghast --release --features microsoft --example online_chat" >&2; exit 2; }
if ps -eo args | grep -E '[j]ava .*(paper-|velocity-)' >/dev/null; then
  echo "another lab server is running; stop it first" >&2; exit 3
fi
python3 -c 'import cryptography' 2>/dev/null || { echo "python3 cryptography module missing" >&2; exit 2; }

JAVA="$(java_home "$JAVA_MAJOR")/bin/java"
fetch "$PAPER_URL" "$DL/$PAPER_JAR" "$PAPER_SHA"
fetch "$GRIM_URL" "$DL/$GRIM_JAR" "$GRIM_SHA"
mkdir -p "$D/plugins" "$D/config"
ln -sf "$DL/$PAPER_JAR" "$D/$PAPER_JAR"
for x in versions libraries cache; do
  if [ -d "$LAB26/$x" ] && [ ! -e "$D/$x" ]; then ln -s "$LAB26/$x" "$D/$x"; fi
done
cp -f "$DL/$GRIM_JAR" "$D/plugins/$GRIM_JAR"
if [ -f "$LAB26/plugins/GrimAC/config.yml" ] && [ ! -f "$D/plugins/GrimAC/config.yml" ]; then
  mkdir -p "$D/plugins/GrimAC"; cp "$LAB26/plugins/GrimAC/config.yml" "$D/plugins/GrimAC/"
fi
echo "eula=true" > "$D/eula.txt"
sed -e "s/^server-port=.*/server-port=$PORT/" -e "s/^online-mode=.*/online-mode=true/" \
    -e "s/^enforce-secure-profile=.*/enforce-secure-profile=true/" \
    -e "s/^motd=.*/motd=ghast lab $GAME online mode (local account stand-in)/" \
    "$here/templates/server-$VER.properties" > "$D/server.properties"
# The bot's profile id (online_chat: offline-style UUID of "lab:<name>"), opped for /say.
printf '[{"uuid":"%s","name":"%s","level":4,"bypassesPlayerLimit":false}]\n' "$(offline_uuid "lab:$BOT")" "$BOT" > "$D/ops.json"

PIDS=()
cleanup() {
  printf 'stop\n' | timeout 3 tee "$D/console.in" >/dev/null 2>&1 || true
  local p
  for p in "${PIDS[@]}"; do for _ in $(seq 1 40); do kill -0 "$p" 2>/dev/null || break; sleep 1; done; done
  for p in "${PIDS[@]}"; do if kill -0 "$p" 2>/dev/null; then kill -TERM "$p"; fi; done
  echo "server and stand-in stopped"
}
trap cleanup EXIT

: > "$D/session-server.log"
setsid nohup python3 "$here/fake-session-server.py" "$SVC" >> "$D/session-server.log" 2>&1 < /dev/null &
PIDS+=("$!")
cd "$D"
[ -p console.in ] || { rm -f console.in; mkfifo console.in; }
mkdir -p logs; : > logs/stdout.txt
hosts="-Dminecraft.api.session.host=http://127.0.0.1:$SVC -Dminecraft.api.services.host=http://127.0.0.1:$SVC -Dminecraft.api.profiles.host=http://127.0.0.1:$SVC"
setsid nohup bash -c 'exec 3<>console.in; exec "$0" -Xms128M -Xmx'"$HEAP"' -XX:+UseG1GC -Dfile.encoding=UTF-8 '"$hosts"' -jar '"$PAPER_JAR"' nogui <&3' \
  "$JAVA" >> logs/stdout.txt 2>&1 < /dev/null &
PIDS+=("$!")
cd - >/dev/null
for _ in $(seq 1 240); do grep -qa 'Done (' "$D/logs/stdout.txt" && break; sleep 1; done
grep -qa 'Done (' "$D/logs/stdout.txt" || { echo "server did not start; see $D/logs/stdout.txt" >&2; exit 1; }
printf 'setworldspawn 0 4 0\n' | timeout 3 tee "$D/console.in" >/dev/null
echo "online-mode lab up: 127.0.0.1:$PORT (services 127.0.0.1:$SVC), $(free -m | awk '/^Mem:/ {print $7}') MB available"

run_bot() { # run_bot <tag> [extra args]
  local tag=$1; shift
  local log="$D/bot-$tag.log" rc=0
  timeout -s TERM 120 "$bin" --host 127.0.0.1 --port "$PORT" --username "$BOT" --version "$GAME" \
    --session-base "http://127.0.0.1:$SVC" --minutes 0.7 "$@" 2> "$log" || rc=$?
  grep -aE 'chat:|sending:|session|kicked|ERROR|WARN' "$log" | sed "s/^/  [$tag] /" || true
  return "$rc"
}

fail=0
since=$(date +%H:%M:%S)
run_bot signed || { echo "FAIL: the signed run did not end cleanly"; fail=1; }
sleep 2
sig_log=$(awk -v t="[$since" 'substr($0, 1, length(t)) >= t' "$D/logs/latest.log")
echo "$sig_log" | grep -aE "$BOT|Not Secure|secure|signature|chat" | sed 's/^/  [server] /' || true
echo "$sig_log" | grep -aq "<$BOT> hello from a signed chat session" || { echo "FAIL: chat line missing"; fail=1; }
if echo "$sig_log" | grep -aE "Not Secure.*$BOT|$BOT.*Not Secure" >/dev/null; then echo "FAIL: chat logged as not secure"; fail=1; fi
echo "$sig_log" | grep -aq "\[$BOT\] signed say command" || { echo "FAIL: /say not run"; fail=1; }
flags=$(echo "$sig_log" | grep -aoE '[A-Za-z0-9_]+ failed [A-Za-z]+' | sort | uniq -c || true)
[ -z "$flags" ] || { echo "FAIL: Grim flags: $flags"; fail=1; }
grep -aE 'join|hasJoined|certificates|publickeys' "$D/session-server.log" | sort | uniq -c | sed 's/^/  [services] /'

since=$(date +%H:%M:%S)
run_bot unsigned --unsigned || true
sleep 2
uns_log=$(awk -v t="[$since" 'substr($0, 1, length(t)) >= t' "$D/logs/latest.log")
echo "$uns_log" | grep -aE "$BOT" | sed 's/^/  [server] /' || true
if echo "$uns_log" | grep -aq "<$BOT> hello from a signed chat session"; then
  echo "FAIL (control): the server accepted unsigned chat"; fail=1
fi
[ "$fail" = 0 ] && echo "PASS: online-mode login, signed chat + commands accepted with enforce-secure-profile, unsigned chat refused, 0 Grim flags"
exit "$fail"
