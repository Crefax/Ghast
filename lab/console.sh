#!/usr/bin/env bash
# Run console command(s) and print the log lines they produced:
#   ./console.sh <1.8|1.16> "list" "fly GhastBot on"     (no commands: one per stdin line)
# Env: CMD_GAP (s between commands, default 0.02), CMD_WAIT (s to collect output, default 0.7).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-}"; shift
lab_pid >/dev/null || { echo "lab $VER not running (./start.sh $VER)" >&2; exit 1; }
LOG="$DIR/logs/latest.log"; FIFO="$DIR/console.in"
if [ $# -eq 0 ]; then mapfile -t cmds < <(grep -vE '^\s*(#|$)'); else cmds=("$@"); fi
start=$(stat -c %s "$LOG")
for c in "${cmds[@]}"; do
  printf '%s\n' "${c#/}" | timeout 3 tee "$FIFO" >/dev/null
  sleep "${CMD_GAP:-0.02}"
done
sleep "${CMD_WAIT:-0.7}"
tail -c +"$((start + 1))" "$LOG" | sed -E 's/^\[[0-9:]+\] \[Server thread\/INFO\]: //; /^\s*$/d'
