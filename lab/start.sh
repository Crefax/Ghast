#!/usr/bin/env bash
# Start a local lab server (idempotent): ./start.sh <1.8|1.12|1.16|26.1|26.2>
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
# LAB_GRIM_JAR=<file in lab/servers/dl>: run that Grim build instead of the pinned one (cross-checks
# against a newer build); exactly one Grim jar is left in plugins/.
grim=${LAB_GRIM_JAR:-$GRIM_JAR}
[ -f "$DL/$grim" ] || { echo "no $DL/$grim" >&2; exit 1; }
for f in "$DIR"/plugins/grimac-bukkit-*.jar; do
  if [ -e "$f" ] && [ "$(basename "$f")" != "$grim" ]; then rm -f "$f"; fi
done
cp -f "$DL/$grim" "$DIR/plugins/$grim"

# Plugins built from source against the 1.8.8 Bukkit API (Java 8 bytecode; the /fly one loads on 1.16.5 too).
# build_plugin <src dir> <jar name>: rebuilt only when the source changed; src dir has plugin.yml + src/**.java.
B="$SERVERS/build"
build_plugin() {
  local src=$1 name=$2 h
  h=$(cat $(find "$src" -type f -not -path '*/.git/*' | sort) | sha256sum | cut -c1-16)
  if [ ! -f "$B/$name.jar" ] || [ "$(cat "$B/$name.hash" 2>/dev/null)" != "$h" ]; then
    local api="$SERVERS/1.8/cache/patched_1.8.8.jar"
    if [ ! -f "$api" ]; then
      mkdir -p "$SERVERS/1.8"; fetch "https://fill-data.papermc.io/v1/objects/7ff6d2cec671ef0d95b3723b5c92890118fb882d73b7f8fa0a2cd31d97c55f86/paper-1.8.8-445.jar" "$DL/paper-1.8.8-445.jar" 7ff6d2cec671ef0d95b3723b5c92890118fb882d73b7f8fa0a2cd31d97c55f86
      (cd "$SERVERS/1.8" && "$JDK21/bin/java" -Dpaperclip.patchonly=true -jar "$DL/paper-1.8.8-445.jar" >/dev/null)
    fi
    rm -rf "$B/$name"; mkdir -p "$B/$name"
    "$JDK21/bin/javac" -nowarn -Xlint:-options --release 8 -encoding UTF-8 -cp "$api" -d "$B/$name" $(find "$src/src" -name '*.java')
    find "$src" -maxdepth 1 -name '*.yml' -exec cp {} "$B/$name/" \;
    (cd "$B/$name" && "$JDK21/bin/jar" cf "$B/$name.jar" .)
    echo "$h" > "$B/$name.hash"
  fi
  cp -f "$B/$name.jar" "$DIR/plugins/$name.jar"
}
build_plugin "$here/plugins/labfly" GhastLabFly
# Extra, out-of-repo plugins for this version (e.g. stand-ins for a private server's plugins, kept in the private
# bot repo): one source dir per line in the git-ignored lab/servers/extra-plugins-<ver>.txt.
if [ -f "$SERVERS/extra-plugins-$VER.txt" ]; then
  while read -r d; do
    case "$d" in ''|'#'*) continue ;; esac
    d=${d/#\~/$HOME}; build_plugin "$d" "$(basename "$d")"
  done < "$SERVERS/extra-plugins-$VER.txt"
fi
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
