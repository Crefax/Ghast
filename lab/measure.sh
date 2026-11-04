#!/usr/bin/env bash
# CPU and memory of one process (all its threads) over a window:
#   ./measure.sh <pid> <seconds> [interval=10]
# CPU = delta(utime+stime) from /proc/<pid>/stat over wall time, in % of ONE core (100 = one core busy).
# Memory: RSS (VmRSS), peak RSS (VmHWM), PSS (smaps_rollup) and thread count, sampled each interval.
# Prints one line per interval and a summary; the last line is machine-readable (key=value).
# Plain bash + awk + /proc: runs unchanged on an aarch64 target (copy it there, or `ssh "$TARGET_HOST" bash -s < measure.sh`).
set -euo pipefail
pid=${1:?usage: measure.sh <pid> <seconds> [interval]}; secs=${2:?seconds}; step=${3:-10}
[ -r "/proc/$pid/stat" ] || { echo "no such process $pid" >&2; exit 1; }
hz=$(getconf CLK_TCK)
cpu_ticks() { # utime+stime of the whole process (fields 14 and 15, after the ")" of the comm field)
  local s; s=$(cat "/proc/$pid/stat") || return 1; s=${s##*) }; set -- $s; echo $(( ${12} + ${13} ))
}
kb() { awk -v k="$1:" '$1 == k { print $2 }' "$2" 2>/dev/null || echo 0; }
now() { date +%s.%N; }
name=$(tr '\0' ' ' < "/proc/$pid/cmdline" | cut -c1-100)
echo "measuring pid $pid ($name) for ${secs}s, every ${step}s, $(nproc) cpus, CLK_TCK $hz"
t0=$(now); c0=$(cpu_ticks); tp=$t0; cp=$c0
max_cpu=0; sum_pss=0; n=0; max_rss=0
end=$(awk -v a="$t0" -v b="$secs" 'BEGIN { printf "%.3f", a + b }')
while :; do
  left=$(awk -v e="$end" -v n="$(now)" 'BEGIN { l = e - n; printf "%.3f", (l > 0 ? l : 0) }')
  [ "$left" = 0.000 ] && break
  sleep "$(awk -v l="$left" -v s="$step" 'BEGIN { print (l < s ? l : s) }')"
  c=$(cpu_ticks) || { echo "process exited" >&2; break; }; t=$(now)
  rss=$(kb VmRSS "/proc/$pid/status"); thr=$(kb Threads "/proc/$pid/status")
  pss=$(kb Pss "/proc/$pid/smaps_rollup")
  pct=$(awk -v dc=$((c - cp)) -v dt="$(awk -v a="$t" -v b="$tp" 'BEGIN { print a - b }')" -v hz="$hz" 'BEGIN { printf "%.2f", (dt > 0 ? 100 * dc / hz / dt : 0) }')
  printf '%7.1fs  cpu %6s %%  rss %7.1f MB  pss %7.1f MB  threads %s\n' "$(awk -v a="$t" -v b="$t0" 'BEGIN { print a - b }')" "$pct" "$(awk -v k="$rss" 'BEGIN { print k / 1024 }')" "$(awk -v k="$pss" 'BEGIN { print k / 1024 }')" "$thr"
  max_cpu=$(awk -v a="$max_cpu" -v b="$pct" 'BEGIN { print (b > a ? b : a) }')
  [ "${rss:-0}" -gt "$max_rss" ] && max_rss=$rss
  sum_pss=$((sum_pss + ${pss:-0})); n=$((n + 1)); tp=$t; cp=$c
done
t1=$(now); c1=$cp
hwm=$(kb VmHWM "/proc/$pid/status")
awk -v dc=$((c1 - c0)) -v dt="$(awk -v a="$tp" -v b="$t0" 'BEGIN { print a - b }')" -v hz="$hz" -v mx="$max_cpu" \
    -v rss="$max_rss" -v hwm="${hwm:-0}" -v pss=$((n > 0 ? sum_pss / n : 0)) -v pid="$pid" 'BEGIN {
  cpu = (dt > 0 ? 100 * dc / hz / dt : 0)
  printf "summary: cpu avg %.2f %% of one core (max interval %.2f %%), cpu time %.2f s in %.1f s; rss max %.1f MB, peak (VmHWM) %.1f MB, pss avg %.1f MB\n", cpu, mx, dc / hz, dt, rss / 1024, hwm / 1024, pss / 1024
  printf "pid=%s secs=%.1f cpu_pct=%.3f cpu_max_pct=%.2f cpu_s=%.2f rss_mb=%.1f hwm_mb=%.1f pss_mb=%.1f\n", pid, dt, cpu, mx, dc / hz, rss / 1024, hwm / 1024, pss / 1024 }'
