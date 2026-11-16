#!/usr/bin/env bash
# Server resource pack answers on 1.8 / 1.12 / 1.16 (the 26.x configuration phase has its own tests):
# the lab announces a pack (server.properties resource-pack=, an address nothing listens on) and the
# lab plugin logs every status the client sends. PASS = the expected statuses, 0 Grim flags, still
# connected at the end.
#
#   ./resource-pack.sh <1.8|1.12|1.16> [url]     (default url http://127.0.0.1:9/lab-pack.zip)
# Needs: cargo build --release -p ghast --example afk
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-}"
url=${2:-http://127.0.0.1:9/lab-pack.zip}
ex="$LAB_ROOT/../target/release/examples/afk"
[ -x "$ex" ] || { echo "build the example first: cargo build --release -p ghast --example afk" >&2; exit 2; }
"$here/stop.sh" "$VER" >/dev/null 2>&1 || true
LAB_PROPS="resource-pack=$url ${LAB_PROPS:-}" "$here/start.sh" "$VER" || exit 1
since=$(date +%H:%M:%S)
log="$DIR/logs/resource-pack.log"
timeout -s TERM 60 "$ex" --host 127.0.0.1 --port "$PORT" --username GhastBot13 --version "$GAME" --minutes 0.3 -v 2> "$log"
echo "exit $?; session: $(grep -a 'session ended' "$log" | tail -1)"
echo "server saw:"
grep -a '\[lab\] resource pack status' "$DIR/logs/latest.log" | sed 's/^.*\[lab\] /  /'
f=$("$here/alerts.sh" "$VER" --count --since "$since"); echo "${f:-0 Grim flags}"
"$here/stop.sh" "$VER" >/dev/null 2>&1 || true
