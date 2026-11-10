# ghast-protocol

Minecraft Java Edition network protocol for the ghast client library: generated packet
types per protocol version, framing, compression, encryption, NBT. No game logic.

| Module / feature | What it is |
|---|---|
| `v1_8` (feature `v1_8`) | Protocol 47 (1.8 – 1.8.9) |
| `v1_12_2` (feature `v1_12_2`) | Protocol 340 (1.12.2) |
| `v1_16_5` (feature `v1_16_5`) | Protocol 754 (1.16.4 – 1.16.5) |
| `v26_1` (feature `v26_1`) | Protocol 775 (26.1 – 26.1.2), with the configuration state |
| `defs` | every generated definition once, shared by the versions whose code for it is identical (see below) |
| `versions` | the protocol table: number → module, feature, game versions, layout, configuration state |
| `codec` | `Reader` (bounds-checked, zero-copy over `Bytes`), `McWrite`, VarInt/VarLong, `Decode`/`Encode` |
| `types` | `BlockPos` (both bit layouts), slots (`slot::legacy` 1.8–1.12, `slot::modern` 1.13.2–1.20.4), `Holder`/`HolderSet` (registry id or inline value), `LpVec3` (1.21.9+ velocity), `chunk` (26.1 chunk sections and paletted containers, zero-copy), angle/fixed-point helpers |
| `nbt` | `RawNbt` (named root, up to 1.20.1) and `AnonNbt` (unnamed root, 1.20.2+): validated raw bytes, lazily parsed; `NamedTag`/`Tag` tree, Modified UTF-8 |
| `chat` | JSON chat stays a raw string (`to_plain_text`/`parse` with feature `chat-json`); NBT chat (1.20.3+) is `AnonNbt` (`nbt_to_plain_text`) |
| `frame` | `FrameDecoder`/`FrameEncoder`: length prefix, zlib, CFB8 hook, `PacketFilter` |
| `crypto` | AES-128-CFB8 (`Cfb8Encryptor`, batched `Cfb8Decryptor`) |
| `io` (feature `tokio`) | `FramedReader`/`FramedWriter` over `AsyncRead`/`AsyncWrite` halves |
| feature `libdeflate` (default) | whole-packet inflate/deflate with libdeflate; off = pure Rust (flate2 + zlib-rs) |
| feature `testgen` | random value generators for every type (test support) |

## Using it (client side)

```rust,ignore
use ghast_protocol::{FramedReader, FramedWriter, PacketFilter, PacketSet};
use ghast_protocol::v1_8::{self, play::clientbound::{self as cb, ClientboundPlayPacket}};

let (rd, wr) = tcp.into_split();
let mut reader = FramedReader::new(rd);
let mut writer = FramedWriter::new(wr);

// Login: after Set Compression (login::clientbound::Compress) call on both halves
reader.decoder().set_compression(threshold);
writer.encoder().set_compression(threshold);
// after sending the encryption response (secret from ghast-auth):
writer.encoder().enable_encryption(&secret);
reader.decoder().enable_encryption(&secret);

// Play: drop what the bot does not need before it is decoded (or even inflated).
reader.decoder().set_filter(
    PacketFilter::all().without(cb::id::PLAYER_INFO).without(cb::id::WORLD_PARTICLES),
);
while let Some(raw) = reader.read_packet().await? {
    match raw.id {
        cb::id::KEEP_ALIVE => { let p: cb::KeepAlive = raw.decode_as()?; /* ... */ }
        _ => match ClientboundPlayPacket::decode_raw(&raw)? { /* ... */ _ => {} },
    }
}
// One tick: queue the actions and the movement packet, then write them in one syscall.
writer.queue(&v1_8::play::serverbound::KeepAlive { keep_alive_id: 1 })?;
writer.flush().await?;
```

Each generated module has, per state and direction: one struct per packet (implementing
`Packet`, `Decode`, `Encode`), an enum of all packets (`ClientboundPlayPacket`, …,
implementing `PacketSet`), and `id::*` constants. `V1_8` / `V1_16_5` implement `Version`
(protocol number, position layout, the seven packet-set enums), which is the hook for the
version-agnostic layer: write it generically over `V: Version`, or as one `From`/match per
version mapping that version's enums to common events/actions.

## Robustness

Decoding never panics on untrusted input (enforced by property tests on random and mutated
inputs for every packet, NBT and the frame decoder):

- every read is bounds-checked; lengths must be non-negative and fit the remaining input;
- array counts are bounded by `remaining / min_element_size`, preallocation is capped at
  4096 elements;
- NBT nesting is limited to 512, `TAG_End` lists with elements are rejected;
- frames: 3-byte length prefix (vanilla), max frame 2 MiB − 1, max declared uncompressed
  size 8 MiB (configurable), inflated size must match the declared size exactly;
- VarInts longer than 5 bytes (VarLongs: 10) are rejected; like vanilla, padded encodings
  are accepted.

Decoding of a packet body is strict (`decode_raw`/`decode_as` reject trailing bytes, as
vanilla does); `Decode::decode` on a `Reader` is available for lenient use.

## Code generation

`codegen/protocol` (`cargo run -p ghast-codegen-protocol`) downloads minecraft-data 3.119.0
from npm into the git-ignored `/.cache/minecraft-data/3.119.0/` (sha512-checked), follows
`dataPaths.json` (pc 1.8 → `pc/1.8`, pc 1.16.5 → `pc/1.16.2`), and writes
`src/generated/` (committed; plain `cargo build` needs no network). Output is
deterministic and rustfmt-formatted. minecraft-data is MIT licensed; see
`src/generated/NOTICE`.

Layout of the generated code:

- `defs/p<N>/`: every packet struct, nested type and named type exactly once, in the module
  of the first protocol `N` that has it. A later version reuses a definition when the
  emitted Rust code is identical (the concrete slot type and position layout are part of
  the code) and every definition it refers to is shared as well (a greatest-fixpoint
  partition refinement, so recursive types work). Each item is gated with
  `#[cfg(any(feature = ...))]` on the versions that use it, so a version feature that is
  off removes all code only it uses. Adding a version never moves an existing definition.
- Packet structs take the protocol number as a const parameter
  (`struct KeepAlive<const V: i32>`), so one definition serves versions with different
  packet ids. Decoding and encoding are monomorphised per version: the machine code is the
  same as with one copy per version, without run-time version branches.
- `<version>/`: the per-version layer — `id` constants, `pub type KeepAlive =
  defs::p340::play::clientbound::KeepAlive<340>;`, re-exports of the nested types, the
  `Packet` impls (id, name, state, direction), the packet-set enums and the `Version`
  marker. Struct literals, patterns, `decode_as::<cb::KeepAlive>()` and
  `write_packet(&sb::KeepAlive { .. })` work through the aliases.
- `versions_table.rs`: rows of `versions::TABLE` (game versions per protocol number come
  from minecraft-data's `protocolVersions.json`).

`cargo run -p ghast-codegen-protocol` prints how many definitions each version reuses.

How protodef types map to Rust:

| protodef | Rust |
|---|---|
| `varint`/`varlong`/ints/floats/`bool` | `i32`/`i64`/… |
| `string` | `ByteString` (zero-copy, UTF-8 validated) |
| `buffer` (varint count), `restBuffer` | `bytes::Bytes` (zero-copy) |
| `UUID` | `uuid::Uuid` |
| `nbt` / `optionalNbt` | `RawNbt` / `Option<RawNbt>` (hand-written, lazy) |
| `anonymousNbt` / `anonOptionalNbt` | `AnonNbt` / `Option<AnonNbt>` (hand-written, lazy) |
| `registryEntryHolder` / `registryEntryHolderSet` | `Holder<T>` / `HolderSet` |
| `bitflags` | newtype over the integer with one constant per flag; switches can test a flag (`../action/add_player`) |
| `lpVec3` | `LpVec3` (exact wire form, `to_vec3`/`from_vec3`) |
| `buffer` with a fixed count | `bytes::Bytes`, length checked on encode |
| recursive named types | `Box<T>` on the references that close a cycle |
| state-local types (`play.toClient.types`) | named types of that packet module |
| `position` | `BlockPos` with the version's layout (hand-written) |
| `slot` | `types::Slot` = `slot::legacy::Slot` / `slot::modern::Slot` (hand-written) |
| `container` | struct (anonymous containers and bit fields are flattened into the parent) |
| `switch` | `Option<T>` when every branch is void or one type `T`; otherwise an enum with one variant per case plus `Default(T)`/`Other`; the discriminant stays a separate field and encoding checks that both agree |
| `mapper` | newtype over the integer with named constants (`PlayerInfoAction::ADD_PLAYER`); unknown values are kept |
| `bitfield` | struct of integers; encoding range-checks every field |
| `array` (count type / fixed / field) | `Vec<T>`; fixed and field counts are checked on encode |
| `option` | `Option<T>` |
| `entityMetadataLoop`, `topBitSetTerminatedArray` | `Vec<T>` with the terminator/flag handled by the codec |
| `$compareTo`-parametrised types (`particleData`, `entityMetadataItem`) | one enum in `types`, instantiated inline per use |

Vanilla overrides (`VersionSpec::presence` in the generator, tested in
`tests/overrides.rs`): 1.8 `spawn_entity.velocity` is present iff `objectData > 0`, as vanilla
reads it (minecraft-data says `!= 0`). The generator panics if an override no longer matches
an option-like switch, so a changed minecraft-data cannot silently drop it.

Renames: `entityMetadataItem` → `EntityMetadataValue`, with readable variant names for the
metadata type ids (`Byte`, `VarInt`, `Slot`, `Position`, `Pose`, …).

## Tests

```sh
cargo test -p ghast-protocol                       # everything (node tests skip if node is absent)
cargo test -p ghast-protocol --test differential   # see tests/node/README.md
cargo bench -p ghast-protocol
```

- `src/**` unit tests: known-answer vectors (VarInt/VarLong edges, both position layouts,
  NIST SP 800-38A CFB8-AES128, `hello_world.nbt`, Modified UTF-8, slots, frames).
- `tests/roundtrip.rs`: every packet of both versions, 400 seeds each plus proptest; value
  and bytes must survive encode → decode → encode; random/mutated bodies never panic.
- `tests/robustness.rs`: NBT and framing (all compression/encryption/filter modes, random
  chunking) round trips, and garbage/mutation inputs.
- `tests/differential.rs`: byte-exact agreement with node-minecraft-protocol in both
  directions, plus value-level checks for positions, bit fields, UUIDs, i64, mappers, slots
  and entity metadata.
