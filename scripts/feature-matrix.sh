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
    if ! RUSTFLAGS="${RUSTFLAGS:-} -D warnings" cargo check -q -p "$pkg" --all-targets --no-default-features --features "$v"; then
      status=1
    fi
  done
done
# A lean build without display names (all versions).
if [ -z "${VERSIONS:-}${PACKAGES:-}" ]; then
  echo "== ghast --features v1_8,v1_12_2,v1_16_5,v26_1 (no display-names)"
  if ! RUSTFLAGS="${RUSTFLAGS:-} -D warnings" cargo check -q -p ghast --no-default-features --features v1_8,v1_12_2,v1_16_5,v26_1; then
    status=1
  fi
fi
# No version at all (connects fail with VersionNotEnabled; must still build warning-free).
if [ -z "${VERSIONS:-}${PACKAGES:-}" ]; then
  echo "== ghast-client, ghast --no-default-features (no version)"
  if ! RUSTFLAGS="${RUSTFLAGS:-} -D warnings" cargo check -q -p ghast-client -p ghast --no-default-features; then
    status=1
  fi
fi
# Lower crates that name 1.12 `v1_12` (ghast-data's names).
if [ -z "${VERSIONS:-}${PACKAGES:-}" ]; then
  for spec in "ghast-entity:v1_8 v1_12 v1_16_5 v26_1" "ghast-inventory:v1_8 v1_12 v1_16_5 v26_1"; do
    pkg=${spec%%:*}
    for v in ${spec#*:}; do
      echo "== $pkg --features $v"
      if ! RUSTFLAGS="${RUSTFLAGS:-} -D warnings" cargo check -q -p "$pkg" --all-targets --no-default-features --features "$v"; then
        status=1
      fi
    done
  done
fi
exit $status
