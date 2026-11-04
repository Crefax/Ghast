#!/usr/bin/env bash
# Grim output from a lab server's log.
#   ./alerts.sh <1.8|1.16>          follow new Grim lines (alerts, verbose flags, setbacks, kicks)
#   ./alerts.sh <1.8|1.16> --all    every such line of the last 24 h, then exit
#   ./alerts.sh <1.8|1.16> --count  Grim flag lines per player and check, last 24 h (no output = clean)
# --all/--count read latest.log plus the rotated logs/*.log.gz of the last 24 h (Paper rotates at midnight),
# so a run crossing 00:00 is counted whole; lines from rotated files are prefixed with their date.
# Narrow with --since HH:MM[:SS] (today's latest.log only), e.g. ./alerts.sh 1.8 --count --since 23:50
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; . "$here/common.sh"
lab_select "${1:-}"
LOG="$DIR/logs/latest.log"
pat='Grim|failed|kicked|lost connection|setback'
since=""; [ "${3:-}" = --since ] && since=${4:?--since HH:MM[:SS]}
recent() { # last 24 h of log lines, oldest first
  if [ -n "$since" ]; then awk -v s="[$since" 'substr($0, 1, length(s)) >= s' "$LOG"; return; fi
  find "$DIR/logs" -maxdepth 1 -name '*.log.gz' -mmin -1440 -printf '%T@ %p\n' | sort -n | cut -d' ' -f2- |
    while read -r f; do d=$(basename "$f"); zcat "$f" | sed "s/^/${d:0:10} /"; done
  cat "$LOG"
}
case "${2:-}" in
  --all)   recent | grep -aE "$pat" || true ;;
  --count) recent | grep -aoE '[A-Za-z0-9_]+ failed [A-Za-z]+' | sort | uniq -c | sort -rn || true ;;
  *)       tail -n0 -F "$LOG" | grep -a --line-buffered -E "$pat" ;;
esac
