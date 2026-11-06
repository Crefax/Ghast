# Contributing

Thanks for helping. Please read [ARCHITECTURE.md](ARCHITECTURE.md) first.

## Build and test

Rust stable, MSRV 1.89 (edition 2024). No network, Java or Node is needed for a normal build:
generated code and golden vectors are committed.

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Optional:
- Protocol differential tests against node-minecraft-protocol need Node.js
  (`cd crates/ghast-protocol/tests/node && npm ci`; see its README). They skip without Node.
- Benches: `cargo bench -p <crate>`.
- Regenerating code from minecraft-data (pinned 3.119.0, downloaded into the git-ignored
  `.cache/`): `cargo run -p ghast-codegen-protocol` and `cargo run -p ghast-codegen-data`.
  Commit the regenerated output.

## The oracle

The oracle runs the real vanilla client code and produces the golden vectors in
`tests/golden/<version>/`. It needs a JDK 21 (17 works too) and, on the first run, network
access to Mojang's and Fabric's servers (~25 MB, cached in the git-ignored `oracle/cache/`).

```sh
export JAVA_HOME=/path/to/jdk-21
oracle/run.sh verify                  # 1.8.9: regenerate and compare with tests/golden/1.8.9/
MC=1.16.5 oracle/run.sh verify        # the same for 1.16.5
oracle/run.sh help                    # fuzz, replay, clicks, boxes, ...
```

A change to physics, collision, raytrace or click behaviour should come with oracle evidence:
new or regenerated vectors, or a fuzz run (seeds and tick counts in the PR description).
See [oracle/README.md](oracle/README.md).

## Rules (these keep the project publishable)

1. **No Mojang code or files.** No client or server jars, no decompiled source, no assets. The
   oracle downloads what it needs at run time into git-ignored caches.
2. **Our own implementation.** Read vanilla to understand the behaviour, then write it in our
   own structure. Constants and algorithms are facts; a line-by-line transliteration of
   decompiled Java is not welcome.
3. **Yarn names.** Use Yarn / Legacy Yarn (CC0) names for vanilla classes and members in code,
   comments and docs (unnamed members keep their intermediary name, e.g. `method_1234`). Do
   not use MCP names: their licence forbids redistribution. 1.14.4+ may also be read with
   Mojang's official mappings locally, but they are not redistributed.
4. **No private data.** Traces and fixtures must come from your own local lab server with
   throwaway accounts: no real-server traces, account names, coordinates, server addresses,
   proxies, tokens or passwords.
5. Keep golden vectors small enough for the repository; large fuzz runs stay in
   `oracle/cache/`.

## Versioning

All crates share one version number and are released together. Before 1.0: 0.y.Z = additive
(new crates, new API, fixes; existing code keeps compiling), 0.Y.0 = breaking changes, listed in
CHANGELOG.md with a migration note. 1.0 is not a date but a checklist:
- every supported version (at least 1.8.9, 1.12.2, 1.16.5 and a current release) verified against
  the vanilla oracle with zero physics diffs, plus a live lab run with 0 anticheat flags;
- no breaking API change for at least two months of releases, checked with cargo-semver-checks;
- real bots running on it in production for at least a month;
- a guide (mdBook) and examples for every public crate.

## Licence

By contributing you agree that your contribution is dual licensed under MIT OR Apache-2.0,
as described in the [README](README.md#license).
