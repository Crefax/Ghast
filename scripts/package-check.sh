#!/usr/bin/env bash
# Publish rehearsal without publishing: packages every publishable crate in dependency order
# (`cargo package --workspace` resolves the workspace crates through a local registry overlay, so
# a version not on crates.io yet works) and builds each one from its packaged sources. Fails on
# packaged files that must never be published and on a crate over crates.io's 10 MB limit.
#   scripts/package-check.sh              # list + verify
#   NO_VERIFY=1 scripts/package-check.sh  # list + sizes only
set -euo pipefail
cd "$(dirname "$0")/.."
status=0
for m in crates/*/Cargo.toml; do
  c=$(basename "$(dirname "$m")")
  list=$(cargo package --no-verify --list -p "$c")
  bad=$(grep -E -i '(^|/)(private|\.cache|target)/|CLAUDE\.md|\.jar$|\.env$' <<<"$list" || true)
  if [ -n "$bad" ]; then
    echo "$c: files that must not be published:"; echo "$bad"; status=1
  fi
done
verify=()
[ -n "${NO_VERIFY:-}" ] && verify=(--no-verify)
log=$(mktemp)
if ! cargo package --workspace --exclude ghast-codegen-data --exclude ghast-codegen-protocol \
  "${verify[@]}" > "$log" 2>&1; then
  cat "$log"; status=1
fi
grep -E 'Packaged|Verifying' "$log" || true
rm -f "$log"
for f in target/package/*.crate; do
  s=$(stat -c %s "$f")
  awk -v s="$s" -v n="$(basename "$f")" 'BEGIN { printf "%8.1f KiB  %s\n", s / 1024, n }'
  if [ "$s" -gt 10485760 ]; then echo "  over the 10 MB crates.io limit"; status=1; fi
done
exit $status
