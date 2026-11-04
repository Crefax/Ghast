# Differential tests against node-minecraft-protocol

`tests/differential.rs` checks ghast-protocol against
[node-minecraft-protocol](https://github.com/PrismarineJS/node-minecraft-protocol) — the
protocol implementation mineflayer uses — for every packet of every supported version, in
both directions:

- **Rust → node**: ghast generates random packets (`testgen`), encodes them; `diff.js`
  parses each with node, requires that every byte is consumed, serializes the parsed value
  again and returns it. The bytes must be identical.
- **node → Rust**: `diff.js` generates random packets with its own schema-driven generator
  (independent of the Rust one) and serializes them with node; ghast decodes them strictly
  (right packet type, no trailing bytes) and re-encodes. The bytes must be identical.

Pinned versions (see `package.json`, `package-lock.json`): minecraft-protocol 1.68.0,
protodef 1.19.0, prismarine-nbt 2.8.0, and minecraft-data **3.119.0** (forced with
`overrides`, the same data the Rust code is generated from).

## Running

```sh
cd crates/ghast-protocol/tests/node
npm ci                # node_modules/ is git-ignored
cd ../../../..
cargo test -p ghast-protocol --test differential -- --nocapture
GHAST_DIFF_N=1000 cargo test -p ghast-protocol --test differential   # more packets per id
```

Without `node` or `node_modules` the tests print `SKIPPED` and pass, so plain `cargo test`
works everywhere.

## Deliberate differences

ghast follows vanilla where minecraft-data does not; such packets are skipped in the
Rust → node direction and adjusted in node's generator (`diverges_from_node` in
`differential.rs`), and covered by `tests/overrides.rs` instead:

- 1.8 `spawn_entity`: velocity iff `objectData > 0` (minecraft-data: `!= 0`), so packets with
  a negative `objectData` differ.

## Known node-minecraft-protocol behaviours the generators avoid

These are limitations of the reference, not of ghast; the generators stay inside the range
where both implementations are well defined (`Gen::node_compat` on the Rust side):

- `varlong` is implemented as a 32-bit unsigned VarInt in node (`readVarLong` calls
  `readVarInt`), so VarLong fields are limited to `0..=i32::MAX`.
- prismarine-nbt reads a name after a TAG_End root (vanilla does not), so the `nbt` type is
  never generated as a bare TAG_End.
- NBT compound keys that look like array indices (`"0"`, `"12"`) are reordered by JavaScript
  objects, and duplicate keys collapse; generated keys are unique and start with a letter.
- The node reader for `topBitSetTerminatedArray` clears flag bits in the buffer it parses, so
  `diff.js` takes the hex of a packet before parsing it.
- Entity-metadata entries whose first byte equals the terminator, and top-bit-terminated
  entries whose first byte has bit 7 set, cannot be encoded by either side; generators retry.
