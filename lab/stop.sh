#!/usr/bin/env bash
# Graceful stop (console "stop" -> worlds saved), SIGTERM/SIGKILL fallback: ./stop.sh <1.8|1.12|1.16|26.1|all>
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
stop_one() {
  lab_select "$1"
  local p; if ! p=$(lab_pid); then echo "lab $VER not running"; rm -f "$DIR/server.pid"; return 0; fi
  printf 'stop\n' | timeout 3 tee "$DIR/console.in" >/dev/null || true
  for _ in $(seq 1 40); do kill -0 "$p" 2>/dev/null || break; sleep 1; done
  if kill -0 "$p" 2>/dev/null; then kill -TERM "$p"; sleep 10; fi
  if kill -0 "$p" 2>/dev/null; then kill -KILL "$p"; echo "lab $VER killed"; else echo "lab $VER stopped"; fi
  rm -f "$DIR/server.pid"
}
case "${1:-all}" in
  all) stop_one 1.8; stop_one 1.12; stop_one 1.16; stop_one 26.1 ;;
  *) stop_one "$1" ;;
esac
