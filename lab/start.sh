#!/usr/bin/env bash
# Start a local lab server (idempotent): ./start.sh <1.8|1.16>
# First run downloads Paper + GrimAC into lab/servers/ (git-ignored), builds the lab /fly plugin,
# writes eula/ops/properties, starts the server in the background and waits for "Done".
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-}"
if p=$(lab_pid); then echo "lab $VER already running (pid $p, 127.0.0.1:$PORT)"; exit 0; fi

mkdir -p "$DIR/plugins" "$DL"
JAVA="$(java_home "$JAVA_MAJOR")/bin/java"; JDK21="$(java_home 21)"
fetch "$PAPER_URL" "$DL/$PAPER_JAR" "$PAPER_SHA"
fetch "$GRIM_URL" "$DL/$GRIM_JAR" "$GRIM_SHA"
ln -sf "$DL/$PAPER_JAR" "$DIR/$PAPER_JAR"
cp -f "$DL/$GRIM_JAR" "$DIR/plugins/$GRIM_JAR"

# Lab /fly plugin, compiled once against the 1.8.8 Bukkit API (Java 8 bytecode, loads on 1.16.5 too).
B="$SERVERS/build"; src="$here/plugins/labfly"
h=$(cat $(find "$src" -type f | sort) | sha256sum | cut -c1-16)
if [ ! -f "$B/GhastLabFly.jar" ] || [ "$(cat "$B/labfly.hash" 2>/dev/null)" != "$h" ]; then
  api="$SERVERS/1.8/cache/patched_1.8.8.jar"
  if [ ! -f "$api" ]; then
    mkdir -p "$SERVERS/1.8"; fetch "https://fill-data.papermc.io/v1/objects/7ff6d2cec671ef0d95b3723b5c92890118fb882d73b7f8fa0a2cd31d97c55f86/paper-1.8.8-445.jar" "$DL/paper-1.8.8-445.jar" 7ff6d2cec671ef0d95b3723b5c92890118fb882d73b7f8fa0a2cd31d97c55f86
    (cd "$SERVERS/1.8" && "$JDK21/bin/java" -Dpaperclip.patchonly=true -jar "$DL/paper-1.8.8-445.jar" >/dev/null)
  fi
  rm -rf "$B/labfly"; mkdir -p "$B/labfly"
  "$JDK21/bin/javac" -nowarn -Xlint:-options --release 8 -cp "$api" -d "$B/labfly" $(find "$src/src" -name '*.java')
  cp "$src/plugin.yml" "$src/config.yml" "$B/labfly/"
  (cd "$B/labfly" && "$JDK21/bin/jar" cf "$B/GhastLabFly.jar" .)
  echo "$h" > "$B/labfly.hash"
fi
cp -f "$B/GhastLabFly.jar" "$DIR/plugins/GhastLabFly.jar"
mkdir -p "$DIR/plugins/GhastLabFly"
{ printf '# Players that get allowFlight automatically on join / respawn / world or gamemode change.\nauto-fly:\n'
  for n in $LAB_FLY; do printf '  - %s\n' "$n"; done; } > "$DIR/plugins/GhastLabFly/config.yml"

echo "eula=true" > "$DIR/eula.txt"
cp "$here/templates/server-$VER.properties" "$DIR/server.properties"
{ printf '['; sep=
  for n in $LAB_OPS; do printf '%s{"uuid":"%s","name":"%s","level":4,"bypassesPlayerLimit":false}' "$sep" "$(offline_uuid "$n")" "$n"; sep=,; done
  printf ']\n'; } > "$DIR/ops.json"

# Grim: print verbose flags to the console as well (alerts already do); no update checks.
grim_cfg() {
  local g="$DIR/plugins/GrimAC/config.yml"; [ -f "$g" ] || return 1
  python3 - "$g" <<'PY'
import re, sys
p = sys.argv[1]; s = open(p).read(); o = s
s = re.sub(r'(verbose:\n\s+print-to-console:) \w+', r'\1 true', s)
s = re.sub(r'(alerts:\n(?:\s+#.*\n)*\s+print-to-console:) \w+', r'\1 true', s)
s = re.sub(r'^check-for-updates: \w+', 'check-for-updates: false', s, flags=re.M)
if s != o: open(p, 'w').write(s); print("patched")
PY
}
fresh=0; [ -d "$DIR/world" ] || fresh=1
had_grim_cfg=0; grim_cfg >/dev/null && had_grim_cfg=1

cd "$DIR"
[ -p console.in ] || { rm -f console.in; mkfifo console.in; }
mkdir -p logs; : > logs/stdout.txt
# stdin = the console FIFO held open read+write, so the console reader blocks instead of seeing EOF.
setsid nohup bash -c 'exec 3<>console.in; exec "$0" -Xms256M -Xmx'"$LAB_HEAP"' -XX:+UseG1GC -XX:MaxGCPauseMillis=50 \
  -Dfile.encoding=UTF-8 '"$JAVA_FLAGS"' \
  -jar '"$PAPER_JAR"' nogui <&3' "$JAVA" >> logs/stdout.txt 2>&1 < /dev/null &
echo $! > server.pid
for _ in $(seq 1 180); do
  if grep -qa 'Done (' logs/stdout.txt; then
    if [ "$had_grim_cfg" = 0 ] && [ "$(grim_cfg)" = patched ]; then "$here/console.sh" "$VER" "grim reload" >/dev/null || true; fi
    if [ "$fresh" = 1 ]; then
      # new world: move the spawn (and so the always-loaded spawn chunks) to the test pad at 0,0, then restart
      "$here/console.sh" "$VER" "setworldspawn 0 4 0" >/dev/null; "$here/stop.sh" "$VER" >/dev/null; exec "$here/start.sh" "$VER"
    fi
    echo "lab $VER up: 127.0.0.1:$PORT (pid $(cat server.pid), offline mode)"; exit 0
  fi
  kill -0 "$(cat server.pid)" 2>/dev/null || { echo "server exited; tail of $DIR/logs/stdout.txt:" >&2; tail -n 30 logs/stdout.txt >&2; exit 1; }
  sleep 1
done
echo "timed out waiting for 'Done'; see $DIR/logs/stdout.txt" >&2; exit 1
