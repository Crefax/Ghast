#!/usr/bin/env bash
# One entry point for the oracle. Needs a JDK 17+ (JAVA_HOME or java on PATH).
#
#   oracle/run.sh                 fetch + remap (first run) + build + regenerate tests/golden/<mc>/
#   oracle/run.sh verify          regenerate into a temp dir and compare content with tests/golden/<mc>/
#   oracle/run.sh selftest        generate twice in one JVM and compare (determinism)
#   oracle/run.sh coverage        rewrite YARN_COVERAGE.md
#   oracle/run.sh decompile       decompile the Yarn-named jar into cache/<mc>/decompiled (local reading only)
#   oracle/run.sh fuzz --seed 7 --count 10000 --ticks 100 [--out FILE.jsonl.gz]
#   oracle/run.sh <any CLI args>  e.g. "scenarios --only ladder_climb --out /tmp/x.jsonl", "list", "help"
#
# Environment: GRADLE (default ./gradlew), MC (Minecraft version, default 1.8.9).
set -euo pipefail
cd "$(dirname "$0")"
GRADLE=${GRADLE:-./gradlew}
MC=${MC:-1.8.9}
cmd=${1:-golden}

case "$cmd" in
  coverage|decompile|prepareVanilla)
    exec "$GRADLE" -q -Pmc="$MC" "$cmd" ;;
  verify)
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT
    "$GRADLE" -q -Pmc="$MC" oracle --args="golden --out $tmp"
    "$GRADLE" -q -Pmc="$MC" oracle --args="compare $tmp $(cd .. && pwd)/tests/golden/$MC" ;;
  golden)
    exec "$GRADLE" -q -Pmc="$MC" oracle --args="golden" ;;
  *)
    exec "$GRADLE" -q -Pmc="$MC" oracle --args="$*" ;;
esac
