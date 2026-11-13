# ghast lab

Local test servers, the mineflayer baseline, measurement and aarch64 (Raspberry Pi 5) cross-build
scripts. Server jars,
worlds, logs, the built plugin and npm deps live in git-ignored dirs (`lab/servers/`,
`lab/baseline/node_modules/`, `lab/results/`) and are downloaded on first use, never committed.

## Servers (localhost only)

| | 1.8 | 1.12 | 1.16 |
|---|---|---|---|
| Address | `127.0.0.1:25618` (protocol 47) | `127.0.0.1:25612` (protocol 340) | `127.0.0.1:25616` (protocol 754) |
| Server | Paper 1.8.8 build 445 | Paper 1.12.2 build 1620 | Paper 1.16.5 build 794 |
| Java | Temurin 21 (`-DIReallyKnowWhatIAmDoingISwear`) | Temurin 17 (`-DPaper.IgnoreJavaVersion=true`) | Temurin 17 (`-DPaper.IgnoreJavaVersion=true`) |
| Anticheat | GrimAC 2.3.73 (sha256-pinned) | GrimAC 2.3.73 | GrimAC 2.3.73 |

All: offline mode, `-Xmx896M` (`LAB_HEAP`), compression threshold 256, view distance 6, superflat
(bedrock, 2 dirt, grass: grass is y=3, players stand at y=4), spawn `0 4 0`, peaceful, no mobs.
Opped accounts (`LAB_OPS`): `GhastBot`..`GhastBot11`, `GhastBot13`, `Baseline`, `Baseline2`. Op does **not**
exempt from Grim. `GhastBot` and `GhastBot5` (`LAB_FLY`, read at server start) get allowFlight on join via the lab plugin (`/fly [player] [on|off]`,
source in `plugins/labfly`, built at start against the 1.8.8 API, loads on both servers).
Grim alerts and verbose flags are printed to the console (`config.yml` patched on first start).

Why Java 17 for 1.12.2 and 1.16.5: Grim 2.3.73 is Java 17 bytecode and those Paper builds refuse Java 17+
unless told to ignore it; they run fine. Only one lab server at a time in WSL (memory). Why `use-native-transport=false` on 1.8: its netty 4.0 epoll transport can't
read Java 21 direct buffers on x86_64 (every connection is reset with "Unable to access address of buffer").
1.8 survival join spawns are fuzzed by up to 5 blocks (hard-coded in 1.8); use `reset-field.sh 1.8 --tp`.

```
./start.sh 1.8|1.12|1.16|26.1|26.2   # idempotent; first run downloads + creates the world (restarts once to move the spawn chunks)
./stop.sh 1.8|1.12|1.16|26.1|26.2|all  # console "stop" (saves), SIGTERM/SIGKILL fallback
./console.sh 1.8 "list" "tp GhastBot 0.5 4 0.5"   # prints the log lines produced; no args = one command per stdin line
./alerts.sh 1.8              # follow Grim flags/setbacks/kicks; --all = all such lines in latest.log; --count = flags per player+check
./reset-field.sh 1.8 [--tp] [--fly]   # rules (time 6000, clear weather, no ticks) + rebuild the test pad
./pathfinder-course.sh 1.8|1.12 build|tp NAME|follow|clear   # parkour corridor + maze for the pathfinder (see the script)
./airborne-join.sh 26.1 [5]  # regression: log in 1 block above the ground N times, PASS = 0 Grim flags (any lab)
LAB_GRIM_JAR=<jar in servers/dl> ./start.sh 26.1   # run another Grim build instead of the pinned one (cross-checks)
./transfer.sh                # 26.1: /transfer the bot back to the lab twice (--follow-transfers), PASS = both followed, 0 flags
./entities.sh [crowd|horse|villager|border|teams|boat|cart]  # 26.1 with summoned NoAI mobs (the lab spawns none): crowd + boat with entity tracking on/off, horse inventory, villager trade, shrinking world border, two bots with and without a collisionRule-never team; PASS = screens confirmed, 0 flags
./proxy-switch.sh [10]       # 26.1 reconfiguration: Velocity + two backends, N /server switches (own server set, see below)
./online-26.1.sh             # 26.1 online mode + signed chat against a local account stand-in (own server, see below)
```

**26.x labs**: `26.1` is Paper 26.1.2 build 74 on `127.0.0.1:25626`, `26.2` is Paper 26.2 build 135
(sha256-pinned) on `127.0.0.1:25634`, both with Grim 2.3.74 and Java 25, the same pad. The
26.x-only scripts (`elytra.sh`, `riptide.sh`, `entities.sh`, `transfer.sh`, `proxy-switch.sh`,
`online-26.1.sh`) run on 26.1 by default and on 26.2 with `LAB_VER=26.2`; the others take the
version as their first argument.

Server sets with their own directories (each counts as the one lab set that may run; the scripts
refuse to start while another lab JVM runs and stop their servers by PID at the end):

- `proxy-switch.sh` — `servers/proxy-26.1/` (`LAB_VER=26.2`: `servers/proxy-26.2/`, Paper 26.2 backends): Velocity 3.5.1 build 615 (sha256-pinned) on
  `127.0.0.1:25629`, modern forwarding, backends `a` (`:25631`, vanilla overworld y -64..319) and
  `b` (`:25632`, a data pack overworld y -128..383, so the registries differ between them), both
  Paper 26.1.2 + Grim 2.3.74, `SWITCH_HEAP` (default 512M) each. The `server_switch` example walks,
  sends `/server a|b`, logs each backend's floor and its resident memory. PASS = every switch done,
  0 Grim flags on both backends, memory not growing by more than 15 % from the 2nd join to the last.
- `online-26.1.sh` — `servers/online-26.1/` (`LAB_VER=26.2`: `servers/online-26.2/`) on `127.0.0.1:25627`: Paper 26.1.2 + Grim with
  `online-mode=true` and `enforce-secure-profile=true`; its account services are
  `fake-session-server.py` on `127.0.0.1:25700` (authlib's `-Dminecraft.api.session.host`,
  `services.host`, `profiles.host`; needs the Python `cryptography` module). The stand-in accepts
  the access token `fake:<profile id>`, records `join`, answers `hasJoined`, serves its own signing
  key at `/publickeys` and issues profile key pairs at `/player/certificates`. The `online_chat`
  example (feature `microsoft`) logs in through it, chats and runs `/say`, `/me`, `/msg`; a second
  run with `--unsigned` is the control (the server must refuse its chat). No real account is used.

Test pad (`reset-field.sh`): `x -16..47, z -16..47` cleared to grass; floor strips at y=3, `z 8..23`:
x 0..7 ice, 8..15 packed ice, 16..23 soul sand, 24..31 slime, 32..39 water (2 deep); x 40..47 step-up course
(z 8 bottom slab, z 10 full block, z 12 two-high wall, z 14 fence). Server files: `servers/<ver>/`, log
`servers/<ver>/logs/latest.log`. Each server uses up to `LAB_HEAP` of RAM; stop them when not needed.

## Measuring

```
./measure.sh <pid> <seconds> [interval=10]   # CPU % of ONE core (utime+stime of all threads), RSS, peak RSS, PSS
./bench-afk.sh [--warmup 30] [--secs 300] [--label L] -- <command...>   # start, settle, measure, stop
./bench-afk.sh --label mineflayer-afk-1.8 -- node baseline/afk.mjs --port 25618
cd baseline && npm install   # mineflayer 4.39.0
node baseline/afk.mjs [--host 127.0.0.1] [--port 25618] [--version 1.8.9] [--user Baseline] [--secs N]
```

`measure.sh` is plain bash/awk over `/proc` and runs unchanged on an aarch64 target.

## aarch64 (Raspberry Pi 5) builds and measurements

```
./build-pi.sh -p ghast --example afk           # cargo-zigbuild, aarch64-unknown-linux-gnu.2.39, -C target-cpu=cortex-a76
./build-pi.sh --benches -p ghast-protocol      # criterion bench binaries; run them on the target with --bench
```

Cross-compiling needs no sudo: `rustup target add aarch64-unknown-linux-gnu`, zig 0.14.1 in
`$HOME/.local/zig-0.14.1` (or on PATH) and `cargo install cargo-zigbuild --locked`. zig is the C compiler and
linker, so C dependencies (libdeflate) cross-build too. Output goes to `target/pi/` (separate target dir, so
it doesn't wait on the host build lock). The default glibc is 2.39 (Ubuntu 24.04; `PI_GLIBC` to change).
Building on the host keeps the target's CPU free for the bots.

To measure on the target, copy the binary, `measure.sh` and `baseline/` there, start both AFK bots side by
side against a lab server reachable from the target (with different accounts: lab accounts kick each other),
and run `measure.sh <pid> 300` for each after a warm-up.

## Results

ghast-protocol criterion benches, Pi 5 (Cortex-A76 @ 2.4 GHz, cross-built as above, `--warm-up-time 1
--measurement-time 3`) vs x86_64 WSL (same benches, earlier run on the dev machine):

| bench | Pi 5 | x86_64 |
|---|---|---|
| cfb8_64KiB/decrypt_batched | 366 µs (171 MiB/s) | 123 µs |
| cfb8_64KiB/decrypt_serial_cfb8_crate | 899 µs | 1017 µs |
| cfb8_64KiB/encrypt | 873 µs | 986 µs |
| inflate_chunk/libdeflater | 94 µs (1.94 GiB/s) | 55 µs |
| inflate_chunk/flate2_zlib_rs | 162 µs | 119 µs |
| frame_inflate/window_items_17k | 7.6 µs | 4.4 µs |
| decode/window_items_90_slots_lazy_nbt | 12.9 µs | 5.7 µs |
| decode/window_items_90_slots_parse_all_nbt | 118 µs | 58 µs |
| filter_busy_stream/decode_everything | 815 µs | 444 µs |
| filter_busy_stream/filtered_keepalive_only | 137 µs | 61 µs |
| frame_chunk/filter_drops_it | 4.0 µs | 2.6 µs |
| deflate_small/libdeflater_300B | 13.4 µs | 6.6 µs |
