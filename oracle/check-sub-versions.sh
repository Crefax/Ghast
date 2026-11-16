#!/usr/bin/env bash
# Re-checks every sub-version row against the row it is served by: for each
# versions/<mc>.properties with a `harness.label` (e.g. 1.16.3 -> 1.16.5), regenerates the
# golden set on that version's jar and compares it with tests/golden/<label>/ (content, file by
# file). Run it after changing a row's harness or regenerating its goldens.
#
#   oracle/check-sub-versions.sh            every sub-version
#   oracle/check-sub-versions.sh 1.16.3     only these
#
# Needs a Linux JDK 17+ (JAVA_HOME). Each version compiles its row's harness into the shared
# build directory, so the versions run one after another. Exit status 1 if any file differs.
set -uo pipefail
cd "$(dirname "$0")"
if [ $# -gt 0 ]; then
  versions=("$@")
else
  mapfile -t versions < <(grep -l '^harness.label=' versions/*.properties | sed 's#versions/##; s#\.properties$##' | sort -V)
fi
fail=0
for v in "${versions[@]}"; do
  label=$(sed -n 's/^harness.label=//p' "versions/$v.properties")
  log=$(mktemp)
  if MC=$v ./run.sh verify > "$log" 2>&1; then
    echo "ok   $v = $label ($(grep -c '^same ' "$log") files identical)"
  else
    echo "FAIL $v vs $label:"
    grep -E '^DIFF |error:|Exception' "$log" | head -20
    fail=1
  fi
  rm -f "$log"
done
exit $fail
