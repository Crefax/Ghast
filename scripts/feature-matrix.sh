#!/usr/bin/env bash
# Checks that every game version builds alone, warning-free, through the facade crate and the
# client: `cargo check --no-default-features --features <version>` for each version.
#
#   scripts/feature-matrix.sh            # ghast and ghast-client, every version
#   VERSIONS="v1_8" scripts/feature-matrix.sh
set -euo pipefail
cd "$(dirname "$0")/.."
versions=${VERSIONS:-"v1_8 v1_12_2 v1_16_5 v26_1"}
packages=${PACKAGES:-"ghast-client ghast"}
status=0
for pkg in $packages; do
  for v in $versions; do
    echo "== $pkg --features $v"
    if ! RUSTFLAGS="${RUSTFLAGS:-} -D warnings" cargo check -q -p "$pkg" --no-default-features --features "$v"; then
      status=1
    fi
  done
done
exit $status
