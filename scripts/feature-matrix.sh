#!/usr/bin/env bash
# Checks that every publishable crate builds on its own (no workspace feature unification: what
# `cargo publish` verifies), warning-free, with its default features and with each game version
# alone; the facade and the client also with all their targets, without display names and
# without any version.
#
#   scripts/feature-matrix.sh                 # everything below
#   PACKAGES="ghast-ext" scripts/feature-matrix.sh   # only these crates
#
# Version features per crate (lower crates name 1.12 `v1_12`, as ghast-data does; ghast-auth has
# no version features; ghast-viewer draws 1.8.9 always and 26.1 with `v26_1`).
set -euo pipefail
cd "$(dirname "$0")/.."
status=0
check() { # check <label> <cargo check args...>
  echo "== $1"
  shift
  if ! RUSTFLAGS="${RUSTFLAGS:-} -D warnings" cargo check -q "$@"; then
    status=1
  fi
}

# crate : version features : extra features for a single-version build : --all-targets for those
crates=(
  "ghast-protocol:v1_8 v1_12_2 v1_16_5 v26_1::"
  "ghast-data:v1_8 v1_12 v1_16_5 v26_1::"
  "ghast-physics:v1_8 v1_12 v1_16_5 v26_1::"
  "ghast-world:v1_8 v1_12 v1_16_5 v26_1::"
  "ghast-entity:v1_8 v1_12 v1_16_5 v26_1::all"
  "ghast-inventory:v1_8 v1_12 v1_16_5 v26_1::all"
  "ghast-auth:::"
  "ghast-client:v1_8 v1_12_2 v1_16_5 v26_1::all"
  "ghast-pathfinder:v1_8 v1_12_2 v1_16_5 v26_1:client:all"
  "ghast-ext:v1_8 v1_12_2 v1_16_5 v26_1:pathfinder:all"
  "ghast-viewer:v26_1:download:all"
  "ghast:v1_8 v1_12_2 v1_16_5 v26_1::all"
)
want() { [ -z "${PACKAGES:-}" ] || [[ " $PACKAGES " == *" $1 "* ]]; }

for spec in "${crates[@]}"; do
  IFS=: read -r pkg versions extra targets <<<"$spec"
  want "$pkg" || continue
  check "$pkg (default features, alone)" -p "$pkg"
  all=()
  [ "$targets" = all ] && all=(--all-targets)
  for v in $versions; do
    f=$v
    [ -n "$extra" ] && f="$extra,$v"
    check "$pkg --features $f (alone)" -p "$pkg" "${all[@]}" --no-default-features --features "$f"
  done
done

if [ -z "${PACKAGES:-}" ]; then
  # Lean builds: no display names (all versions); no version at all (connects fail with
  # VersionNotEnabled; lookups answer "unknown").
  check "ghast --features v1_8,v1_12_2,v1_16_5,v26_1 (no display-names)" \
    -p ghast --no-default-features --features v1_8,v1_12_2,v1_16_5,v26_1
  check "ghast-client, ghast --no-default-features (no version)" \
    -p ghast-client -p ghast --no-default-features
  check "ghast-ext --no-default-features (no version)" -p ghast-ext --no-default-features
  check "ghast-pathfinder --features client (no version)" \
    -p ghast-pathfinder --no-default-features --features client
  # The facade's optional crates with one version each.
  for v in v1_8 v26_1; do
    check "ghast --features $v,ext,pathfinder,viewer" \
      -p ghast --no-default-features --features "$v,ext,pathfinder,viewer"
  done
fi
exit $status
