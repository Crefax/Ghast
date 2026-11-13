#!/usr/bin/env bash
# Reconfiguration test (play -> configuration -> play, 1.20.2+): a Velocity proxy with two Paper
# 26.1.2 + GrimAC backends, and the server_switch example moving between them with `/server`.
#   ./proxy-switch.sh [switches, default 10]
# Backend "a" has the vanilla overworld (y -64..319); backend "b" a datapack overworld of y -128..383
# (different dimension_type registry data, so the bot must re-sync the registries on every switch).
# Passes when every switch completed, Grim flagged nothing on either backend and the bot's resident
# memory did not grow from the second join to the last one (more than 15 %).
# Everything lives in lab/servers/proxy-<26.1|26.2>/ (git-ignored; LAB_VER=26.2 for the 26.2 set); the three servers are started for the
# run and stopped again by PID. Needs ~1.6 GB free memory and no other lab running.
# Env: SWITCH_HEAP (backend heap, default 512M), BOT (default GhastBot6), KEEP=1 (leave them running).
# Needs the example: cargo build -p ghast --release --example server_switch
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
switches=${1:-10}
lab_select "${LAB_VER:-26.1}"   # Paper 26.1.2 (LAB_VER=26.2: Paper 26.2) + Grim 2.3.74, Java 25
LAB26="$DIR"
P="$SERVERS/proxy-$VER"
VEL_JAR=velocity-3.5.1-615.jar
VEL_SHA=b4e3164df5377346854dc6cb9e6a78022b1946ff69e89676313f5f6f1c6f0fb3
VEL_URL="https://fill-data.papermc.io/v1/objects/$VEL_SHA/$VEL_JAR"
VEL_PORT=25629; A_PORT=25631; B_PORT=25632
HEAP=${SWITCH_HEAP:-512M}
BOT=${BOT:-GhastBot6}
bin="$here/../target/release/examples/server_switch"
[ -x "$bin" ] || { echo "build it first: cargo build -p ghast --release --example server_switch" >&2; exit 2; }

# One lab set at a time (other agents start servers too): refuse when any lab JVM runs.
if ps -eo args | grep -E '[j]ava .*(paper-|velocity-)' >/dev/null; then
  echo "another lab server is running; stop it first (ps -eo pid,args | grep java)" >&2; exit 3
fi
avail=$(free -m | awk '/^Mem:/ {print $7}')
[ "$avail" -ge 1600 ] || { echo "only $avail MB available; need ~1600" >&2; exit 3; }

JAVA="$(java_home "$JAVA_MAJOR")/bin/java"
mkdir -p "$P" "$DL"
fetch "$PAPER_URL" "$DL/$PAPER_JAR" "$PAPER_SHA"
fetch "$GRIM_URL" "$DL/$GRIM_JAR" "$GRIM_SHA"
fetch "$VEL_URL" "$DL/$VEL_JAR" "$VEL_SHA"
secret_file="$P/forwarding.secret"
[ -s "$secret_file" ] || python3 -c 'import secrets; print(secrets.token_hex(16), end="")' > "$secret_file"
secret=$(cat "$secret_file")

backend() { # backend <name> <port> <tall 0|1>
  local d="$P/$1" port=$2 tall=$3
  mkdir -p "$d/plugins" "$d/config"
  ln -sf "$DL/$PAPER_JAR" "$d/$PAPER_JAR"
  # Share the paperclip output of the 26.1 lab when it exists (saves ~160 MB and the patch step).
  for x in versions libraries cache; do
    if [ -d "$LAB26/$x" ] && [ ! -e "$d/$x" ]; then ln -s "$LAB26/$x" "$d/$x"; fi
  done
  cp -f "$DL/$GRIM_JAR" "$d/plugins/$GRIM_JAR"
  if [ -f "$LAB26/plugins/GrimAC/config.yml" ] && [ ! -f "$d/plugins/GrimAC/config.yml" ]; then
    mkdir -p "$d/plugins/GrimAC"; cp "$LAB26/plugins/GrimAC/config.yml" "$d/plugins/GrimAC/"
  fi
  echo "eula=true" > "$d/eula.txt"
  local layers='{"block":"minecraft:bedrock","height":1},{"block":"minecraft:stone","height":64}'
  [ "$tall" = 1 ] && layers='{"block":"minecraft:bedrock","height":1},{"block":"minecraft:stone","height":128}'
  sed -e "s/^server-port=.*/server-port=$port/" \
      -e "s/^motd=.*/motd=ghast lab proxy backend $1/" \
      -e "s/^view-distance=.*/view-distance=6/" -e "s/^simulation-distance=.*/simulation-distance=6/" \
      -e "s|^generator-settings=.*|generator-settings={\"layers\":[$layers,{\"block\":\"minecraft:dirt\",\"height\":2},{\"block\":\"minecraft:grass_block\",\"height\":1}],\"biome\":\"minecraft:plains\",\"features\":false,\"lakes\":false}|" \
      "$here/templates/server-$VER.properties" > "$d/server.properties"
  { printf '['; local sep= n
    for n in $LAB_OPS; do printf '%s{"uuid":"%s","name":"%s","level":4,"bypassesPlayerLimit":false}' "$sep" "$(offline_uuid "$n")" "$n"; sep=,; done
    printf ']\n'; } > "$d/ops.json"
  # Modern forwarding from the proxy (the backend itself stays offline mode).
  if [ ! -f "$d/config/paper-global.yml" ] && [ -f "$LAB26/config/paper-global.yml" ]; then
    cp "$LAB26/config/paper-global.yml" "$d/config/"
  fi
  if [ -f "$d/config/paper-global.yml" ]; then
    python3 - "$d/config/paper-global.yml" "$secret" <<'PY'
import re, sys
p, secret = sys.argv[1], sys.argv[2]
s = open(p).read()
s = re.sub(r"(  velocity:\n    enabled:) \w+\n    online-mode: \w+\n    secret: .*",
           r"\1 true\n    online-mode: false\n    secret: '" + secret + "'", s)
open(p, "w").write(s)
PY
  fi
  if [ "$tall" = 1 ]; then
    # A taller overworld: the vanilla dimension type with min_y -128 and height 512.
    local pack="$d/world/datapacks/ghast-tall"
    if [ ! -f "$pack/pack.mcmeta" ]; then
      mkdir -p "$pack/data/minecraft/dimension_type"
      local tmp; tmp=$(mktemp -d)
      local vjar; vjar=$(ls "$LAB26"/versions/*/paper-*.jar | head -1)
      (cd "$tmp" && "$(dirname "$JAVA")/jar" xf "$vjar" data/minecraft/dimension_type/overworld.json version.json)
      fmt=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["pack_version"]["data_major"])' "$tmp/version.json")
      python3 - "$tmp/data/minecraft/dimension_type/overworld.json" "$pack/data/minecraft/dimension_type/overworld.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
d["min_y"], d["height"], d["logical_height"] = -128, 512, 512
json.dump(d, open(sys.argv[2], "w"), indent=2)
PY
      rm -rf "$tmp"
      printf '{"pack":{"description":"ghast lab: taller overworld","min_format":%s,"max_format":[%s,1]}}\n' "$fmt" "$fmt" > "$pack/pack.mcmeta"
    fi
  fi
}

PIDS=()
start_jvm() { # start_jvm <dir> <heap> <jar> <ready regex> <args...>: console FIFO, waits for the ready line
  local d=$1 heap=$2 jar=$3 ready=$4; shift 4
  cd "$d"
  [ -p console.in ] || { rm -f console.in; mkfifo console.in; }
  mkdir -p logs; : > logs/stdout.txt
  setsid nohup bash -c 'exec 3<>console.in; exec "$0" -Xms128M -Xmx'"$heap"' -XX:+UseG1GC -Dfile.encoding=UTF-8 -jar '"$jar"' "$@" <&3' \
    "$JAVA" "$@" >> logs/stdout.txt 2>&1 < /dev/null &
  local pid=$!; echo "$pid" > server.pid; PIDS+=("$pid")
  cd - >/dev/null
  for _ in $(seq 1 240); do
    if grep -qaE "$ready" "$d/logs/stdout.txt"; then return 0; fi
    kill -0 "$pid" 2>/dev/null || { echo "$d: exited; tail:" >&2; tail -n 30 "$d/logs/stdout.txt" >&2; return 1; }
    sleep 1
  done
  echo "$d: timed out" >&2; return 1
}
console() { printf '%s\n' "$2" | timeout 3 tee "$1/console.in" >/dev/null || true; }
stop_all() {
  [ "${KEEP:-0}" = 1 ] && { echo "KEEP=1: servers left running: ${PIDS[*]}"; return; }
  console "$P/velocity" end; console "$P/a" stop; console "$P/b" stop
  local p
  for p in "${PIDS[@]}"; do for _ in $(seq 1 40); do kill -0 "$p" 2>/dev/null || break; sleep 1; done; done
  for p in "${PIDS[@]}"; do if kill -0 "$p" 2>/dev/null; then kill -TERM "$p"; fi; done
  sleep 2
  for p in "${PIDS[@]}"; do if kill -0 "$p" 2>/dev/null; then kill -KILL "$p"; fi; done
  echo "servers stopped"
}
trap stop_all EXIT

backend a "$A_PORT" 0
backend b "$B_PORT" 1
mkdir -p "$P/velocity"
ln -sf "$DL/$VEL_JAR" "$P/velocity/$VEL_JAR"
cp -f "$secret_file" "$P/velocity/forwarding.secret"
cat > "$P/velocity/velocity.toml" <<EOF
# ghast lab proxy (written by lab/proxy-switch.sh on every run)
config-version = "2.8"
bind = "127.0.0.1:$VEL_PORT"
motd = "ghast lab proxy"
show-max-players = 20
online-mode = false
force-key-authentication = false
prevent-client-proxy-connections = false
player-info-forwarding-mode = "MODERN"
forwarding-secret-file = "forwarding.secret"
announce-forge = false
kick-existing-players = false
ping-passthrough = "DISABLED"
enable-player-address-logging = false

[servers]
a = "127.0.0.1:$A_PORT"
b = "127.0.0.1:$B_PORT"
try = ["a"]

[forced-hosts]

[advanced]
compression-threshold = 256
login-ratelimit = 0
connection-timeout = 5000
read-timeout = 30000
failover-on-unexpected-server-disconnect = false
log-player-connections = true
log-command-executions = true
EOF

since=$(date +%H:%M:%S)
start_jvm "$P/a" "$HEAP" "$PAPER_JAR" 'Done \(' nogui
console "$P/a" "setworldspawn 0 4 0"
start_jvm "$P/b" "$HEAP" "$PAPER_JAR" 'Done \(' nogui
console "$P/b" "setworldspawn 0 4 0"
start_jvm "$P/velocity" 192M "$VEL_JAR" 'Done \('
echo "proxy 127.0.0.1:$VEL_PORT -> a :$A_PORT (y -64..319), b :$B_PORT (y -128..383); $(free -m | awk '/^Mem:/ {print $7}') MB available"

out="$P/server_switch.log"
set +e
timeout -s TERM $((switches * 30 + 120)) "$bin" --host 127.0.0.1 --port "$VEL_PORT" --username "$BOT" \
  --version "$GAME" --switches "$switches" --servers a,b 2> "$out"
rc=$?
set -e
grep -aE 'switch [0-9]+: on|summary|session|kicked|ERROR|WARN' "$out" || true
sleep 1
flags=$(for s in a b; do awk -v t="[$since" 'substr($0, 1, length(t)) >= t' "$P/$s/logs/latest.log" |
  grep -aoE '[A-Za-z0-9_]+ failed [A-Za-z]+' | sed "s/^/$s: /" || true; done | sort | uniq -c)
summary=$(grep -a 'summary:' "$out" | tail -1 || true)
growth=$(python3 - "$summary" <<'PY'
import re, sys
m = re.search(r"\[([0-9, ]+)\]", sys.argv[1])
r = [int(x) for x in m.group(1).split(",")] if m else []
print("n/a" if len(r) < 3 else f"{(r[-1] - r[1]) * 100 / r[1]:+.1f}")
PY
)
echo "rss growth from the 2nd join to the last: $growth %"
fail=0
[ "$rc" = 0 ] || { echo "FAIL: server_switch exited with $rc (see $out)"; fail=1; }
[ -z "$flags" ] || { echo "FAIL: Grim flags:"; echo "$flags"; fail=1; }
case "$growth" in n/a) fail=1; echo "FAIL: not enough joins for the memory check" ;;
  *) python3 -c "import sys; sys.exit(0 if float(sys.argv[1]) <= 15 else 1)" "$growth" || { echo "FAIL: memory grew"; fail=1; } ;;
esac
[ "$fail" = 0 ] && echo "PASS: $switches proxy switches (a <-> b), 0 Grim flags, rss growth $growth %"
exit "$fail"
