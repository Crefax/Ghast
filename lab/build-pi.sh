#!/usr/bin/env bash
# Cross-compile workspace binaries for the Raspberry Pi 5 (aarch64, Cortex-A76, Ubuntu 24.04 glibc 2.39)
# from an x86_64 Linux host with cargo-zigbuild (zig is the C compiler + linker, so C deps such as libdeflate cross-build too).
#   ./build-pi.sh [cargo build args...]        e.g. ./build-pi.sh -p ghast --example afk
#   ./build-pi.sh --benches -p ghast-protocol  bench binaries (run them on the Pi with --bench)
# Release profile; output under target/pi/aarch64-unknown-linux-gnu/release/ (own target dir, so it never
# waits for the lock of the host builds in target/). Prints the built executables' paths, one per line.
# One-time setup (no sudo needed):
#   rustup target add aarch64-unknown-linux-gnu
#   zig 0.14.x unpacked to ~/.local/zig-0.14.1 (or zig on PATH), and: cargo install cargo-zigbuild --locked
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; root="$(cd "$here/.." && pwd)"
TARGET=aarch64-unknown-linux-gnu
GLIBC="${PI_GLIBC:-2.39}"          # `ldd --version` on the target; never above the target's
[ -x "$HOME/.local/zig-0.14.1/zig" ] && export PATH="$HOME/.local/zig-0.14.1:$PATH"
command -v zig >/dev/null || { echo "zig not found (see the header of $0)" >&2; exit 1; }
command -v cargo-zigbuild >/dev/null || { echo "cargo-zigbuild not found: cargo install cargo-zigbuild --locked" >&2; exit 1; }
rustup target list --installed | grep -qx "$TARGET" || rustup target add "$TARGET"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target/pi}"
# target-only flags: build scripts and proc macros still run on (and are tuned for) the x86_64 host
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="${PI_RUSTFLAGS:--C target-cpu=cortex-a76}"
cd "$root"
cargo zigbuild --release --target "$TARGET.$GLIBC" --message-format=json-render-diagnostics "$@" \
  | python3 -c 'import json, sys
for l in sys.stdin:
    try: m = json.loads(l)
    except ValueError: continue
    if m.get("reason") == "compiler-artifact" and m.get("executable"): print(m["executable"])'
