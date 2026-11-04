#!/usr/bin/env bash
# Run a bot command, let it settle, measure it with measure.sh, then stop it:
#   ./bench-afk.sh [--warmup 30] [--secs 300] [--label name] -- <command...>
# e.g. ./bench-afk.sh -- node baseline/afk.mjs --port 25618
#      ./bench-afk.sh --label ghast-afk -- ../target/release/examples/afk --addr 127.0.0.1:25618
# The bot's output and the measurement go to lab/results/<label>-<time>.{log,txt} (git-ignored).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
warmup=30; secs=300; label=bot
while [ $# -gt 0 ]; do case "$1" in
  --warmup) warmup=$2; shift 2 ;; --secs) secs=$2; shift 2 ;; --label) label=$2; shift 2 ;;
  --) shift; break ;; *) break ;; esac; done
[ $# -gt 0 ] || { echo "usage: bench-afk.sh [--warmup S] [--secs S] [--label L] -- <command...>" >&2; exit 2; }
mkdir -p "$here/results"; out="$here/results/$label-$(date +%Y%m%d-%H%M%S)"
"$@" > "$out.log" 2>&1 &
bot=$!
trap 'kill $bot 2>/dev/null || true' EXIT
echo "bot pid $bot: $*  (warmup ${warmup}s, log $out.log)"
sleep "$warmup"
kill -0 $bot 2>/dev/null || { echo "bot exited during warmup:" >&2; tail -n 20 "$out.log" >&2; exit 1; }
"$here/measure.sh" "$bot" "$secs" | tee "$out.txt"
kill -0 $bot 2>/dev/null || { echo "WARNING: bot exited during the measurement" | tee -a "$out.txt"; tail -n 5 "$out.log"; }
kill $bot 2>/dev/null || true; wait $bot 2>/dev/null || true
echo "bot log tail:"; tail -n 4 "$out.log"
