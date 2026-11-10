//! Generates `crates/ghast-protocol/src/generated/` from PrismarineJS minecraft-data.
//!
//! Usage: `cargo run -p ghast-codegen-protocol` (from anywhere in the workspace).
//!
//! minecraft-data 3.119.0 is downloaded once from the npm registry into the git-ignored
//! `.cache/minecraft-data/3.119.0/` (sha512-verified), then `dataPaths.json` is used to find
//! each version's `protocol.json`. The output is formatted with `rustfmt` and committed.
//!
//! Output layout:
//!
//! - `defs/p<N>/…`: every definition (packet struct, nested type, named type) exactly once,
//!   in the module of the FIRST protocol `N` that has it. A definition is shared by every
//!   version whose emitted code for it is identical and whose referenced definitions are
//!   shared too (see [`share`]). Packet structs carry the protocol number as a const
//!   parameter (`KeepAlive<const V: i32>`), so one definition serves versions with
//!   different packet ids. Each item is gated on the features of the versions using it.
//! - `<version>/…`: the thin per-version layer: `id` constants, type aliases
//!   (`pub type KeepAlive = defs::…::KeepAlive<340>;`), re-exports of nested types, the
//!   `Packet` impls, the packet-set enums and the `Version` marker.
//! - `versions_table.rs`: the protocol table behind `ghast_protocol::versions`.

mod emit;
mod ir;
mod names;
mod report;
mod share;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha512};

use emit::{CFG, Emitter, Item};
use ir::Resolver;
use names::{pascal, screaming};

const MCDATA_VERSION: &str = "3.119.0";
const MCDATA_URL: &str = "https://registry.npmjs.org/minecraft-data/-/minecraft-data-3.119.0.tgz";
/// sha512 of the npm tarball (matches the registry's `dist.integrity`).
const MCDATA_SHA512: &str = "444968d28d91a28133a5e40ef90107cd398755380e75b6e8986418066588694d69ba9126008b506fe2e92722f61035e6a4a89c394e2cc2d055cf021669ee65e6";

pub struct VersionSpec {
    module: &'static str,
    feature: &'static str,
    marker: &'static str,
    name: &'static str,
    covers: &'static str,
    data_key: &'static str,
    protocol: i32,
    layout: &'static str,
    /// Concrete type of the native `slot` (versions whose `slot` is a protodef container
    /// have none).
    slot: Option<&'static str>,
    /// Has the configuration state (1.20.2+).
    configuration: bool,
    variant_names: &'static [(&'static str, &'static str)],
    /// `(switch path, condition with {d}, generator hints, doc)`: presence rules that
    /// replace minecraft-data's case keys where they disagree with vanilla.
    presence: &'static [(
        &'static str,
        &'static str,
        &'static [&'static str],
        &'static str,
    )],
}

/// Supported versions, ascending by protocol number (the first protocol that has a
/// definition owns it, so the order matters).
const VERSIONS: &[VersionSpec] = &[
    VersionSpec {
        module: "v1_8",
        feature: "v1_8",
        marker: "V1_8",
        name: "1.8.9",
        covers: "1.8 – 1.8.9",
        data_key: "1.8",
        protocol: 47,
        layout: "Xyz",
        slot: Some("crate::types::slot::legacy::Slot"),
        configuration: false,
        variant_names: &[
            ("EntityMetadataValue.0", "Byte"),
            ("EntityMetadataValue.1", "Short"),
            ("EntityMetadataValue.2", "Int"),
            ("EntityMetadataValue.3", "Float"),
            ("EntityMetadataValue.4", "String"),
            ("EntityMetadataValue.5", "Slot"),
            ("EntityMetadataValue.6", "BlockPos"),
            ("EntityMetadataValue.7", "Rotation"),
        ],
        // Vanilla 1.8.9's spawn-object packet reads and writes the three velocity shorts
        // only when the (signed) object data is > 0; minecraft-data 3.119.0 models it as
        // `{"0": void}` + default, i.e. `!= 0`, so a negative value mis-decodes there
        // (node-minecraft-protocol shares the bug; see tests/node/README.md).
        presence: &[(
            "play.clientbound.spawn_entity.velocity",
            "{d} > 0",
            &["0", "1", "-1", "5", "-2147483648"],
            "present iff `objectData > 0`, as vanilla 1.8.9 reads it (minecraft-data says `!= 0`)",
        )],
    },
    VersionSpec {
        module: "v1_12_2",
        feature: "v1_12_2",
        marker: "V1_12_2",
        name: "1.12.2",
        covers: "1.12.2",
        data_key: "1.12.2",
        protocol: 340,
        layout: "Xyz",
        slot: Some("crate::types::slot::legacy::Slot"),
        configuration: false,
        variant_names: &[
            ("EntityMetadataValue.0", "Byte"),
            ("EntityMetadataValue.1", "VarInt"),
            ("EntityMetadataValue.2", "Float"),
            ("EntityMetadataValue.3", "String"),
            ("EntityMetadataValue.4", "Chat"),
            ("EntityMetadataValue.5", "Slot"),
            ("EntityMetadataValue.6", "Boolean"),
            ("EntityMetadataValue.7", "Rotation"),
            ("EntityMetadataValue.8", "Position"),
            ("EntityMetadataValue.9", "OptPosition"),
            ("EntityMetadataValue.10", "Direction"),
            ("EntityMetadataValue.11", "OptUuid"),
            ("EntityMetadataValue.12", "BlockState"),
            ("EntityMetadataValue.13", "Nbt"),
        ],
        presence: &[],
    },
    VersionSpec {
        module: "v1_16_5",
        feature: "v1_16_5",
        marker: "V1_16_5",
        name: "1.16.5",
        covers: "1.16.4 – 1.16.5",
        data_key: "1.16.5",
        protocol: 754,
        layout: "Xzy",
        slot: Some("crate::types::slot::modern::Slot"),
        configuration: false,
        variant_names: &[
            ("EntityMetadataValue.0", "Byte"),
            ("EntityMetadataValue.1", "VarInt"),
            ("EntityMetadataValue.2", "Float"),
            ("EntityMetadataValue.3", "String"),
            ("EntityMetadataValue.4", "Chat"),
            ("EntityMetadataValue.5", "OptChat"),
            ("EntityMetadataValue.6", "Slot"),
            ("EntityMetadataValue.7", "Boolean"),
            ("EntityMetadataValue.8", "Rotation"),
            ("EntityMetadataValue.9", "Position"),
            ("EntityMetadataValue.10", "OptPosition"),
            ("EntityMetadataValue.11", "Direction"),
            ("EntityMetadataValue.12", "OptUuid"),
            ("EntityMetadataValue.13", "BlockState"),
            ("EntityMetadataValue.14", "Nbt"),
            ("EntityMetadataValue.15", "Particle"),
            ("EntityMetadataValue.16", "VillagerData"),
            ("EntityMetadataValue.17", "OptVarInt"),
            ("EntityMetadataValue.18", "Pose"),
        ],
        presence: &[],
    },
    VersionSpec {
        module: "v26_1",
        feature: "v26_1",
        marker: "V26_1",
        name: "26.1.2",
        covers: "26.1 – 26.1.2",
        data_key: "26.1",
        protocol: 775,
        layout: "Xzy",
        // `Slot` is a protodef container (data components) in this version.
        slot: None,
        configuration: true,
        variant_names: &[],
        // Vanilla 26.1 (`ItemStack.createOptionalStreamCodec`) reads an item stack as
        // empty when the count is <= 0; minecraft-data 3.119.0 only treats 0 as empty.
        presence: &[
            (
                "types.Slot.body",
                "{d} > 0",
                &["0", "1", "-1", "64"],
                "present iff `itemCount > 0`, as vanilla reads it (minecraft-data says `!= 0`)",
            ),
            (
                "types.UntrustedSlot.body",
                "{d} > 0",
                &["0", "1", "-1", "64"],
                "present iff `itemCount > 0`, as vanilla reads it (minecraft-data says `!= 0`)",
            ),
        ],
    },
];

/// A correction of minecraft-data's `protocol.json` where it disagrees with the vanilla
/// jar. Applied before code generation; the generator panics when the target no longer
/// holds `expect` (upstream fixed or changed it), so a stale patch cannot survive.
pub struct Patch {
    /// minecraft-data version key (`VersionSpec::data_key`).
    pub data_key: &'static str,
    /// JSON pointer into `protocol.json`.
    pub pointer: &'static str,
    /// The current (wrong) value, as JSON.
    pub expect: &'static str,
    /// The vanilla value, as JSON.
    pub replace: &'static str,
    /// What vanilla does, and where.
    pub why: &'static str,
}

pub const PATCHES: &[Patch] = &[Patch {
    data_key: "26.1",
    pointer: "/play/toClient/types/packet_multi_block_change/1/1/type/1/type",
    expect: "\"varint\"",
    replace: "\"varlong\"",
    why: "ClientboundSectionBlocksUpdatePacket reads/writes each record with \
          readVarLong/writeVarLong (state id << 12 | position); minecraft-data has varint \
          since 1.20.2 (1.16.2 - 1.20.1 had varlong)",
}];

/// Applies the [`PATCHES`] of one minecraft-data version.
pub fn apply_patches(data_key: &str, proto: &mut Value) {
    for p in PATCHES.iter().filter(|p| p.data_key == data_key) {
        let target = proto
            .pointer_mut(p.pointer)
            .unwrap_or_else(|| panic!("patch {}: {} is gone from minecraft-data", data_key, p.pointer));
        let expect: Value = serde_json::from_str(p.expect).unwrap();
        assert!(
            *target == expect,
            "stale patch {data_key} {}: minecraft-data now has {target}, expected {expect} \
             (fixed upstream? then remove the patch)",
            p.pointer
        );
        *target = serde_json::from_str(p.replace).unwrap();
    }
}

/// `(protocol.json key, Rust module, State variant)`.
const STATES: &[(&str, &str, &str)] = &[
    ("handshaking", "handshaking", "Handshaking"),
    ("status", "status", "Status"),
    ("login", "login", "Login"),
    ("configuration", "configuration", "Configuration"),
    ("play", "play", "Play"),
];
const DIRECTIONS: &[(&str, &str, &str)] = &[
    ("toClient", "clientbound", "Clientbound"),
    ("toServer", "serverbound", "Serverbound"),
];

fn states_of(spec: &VersionSpec) -> impl Iterator<Item = &'static (&'static str, &'static str, &'static str)> {
    let conf = spec.configuration;
    STATES.iter().filter(move |(k, _, _)| conf || *k != "configuration")
}

fn main() {
    let root = workspace_root();
    let data_dir = ensure_data(&root);
    let paths: Value = read_json(&data_dir.join("dataPaths.json"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("packets-report") {
        let [_, data_key, report, out] = &args[..] else {
            panic!("usage: packets-report <minecraft-data version> <packets.json> <out.tsv>");
        };
        let proto_dir = paths["pc"][data_key.as_str()]["protocol"]
            .as_str()
            .unwrap_or_else(|| panic!("no protocol path for {data_key}"));
        let proto: Value = read_json(&data_dir.join(proto_dir).join("protocol.json"));
        report::write_table(&proto, &read_json(Path::new(report)), Path::new(out));
        return;
    }
    let proto_versions: Value = read_json(&data_dir.join("pc/common/protocolVersions.json"));
    let out_root = root.join("crates/ghast-protocol/src/generated");

    let mut outs = Vec::new();
    for spec in VERSIONS {
        let proto_dir = paths["pc"][spec.data_key]["protocol"]
            .as_str()
            .unwrap_or_else(|| panic!("no protocol path for {}", spec.data_key));
        let mut proto: Value = read_json(&data_dir.join(proto_dir).join("protocol.json"));
        apply_patches(spec.data_key, &mut proto);
        let rel = format!("data/{proto_dir}/protocol.json");
        outs.push(emit_version(spec, &proto, rel));
    }
    let shared = share::analyse(&outs);

    // Clean output (generated directories only; NOTICE stays).
    for dir in std::iter::once("defs").chain(VERSIONS.iter().map(|s| s.module)) {
        let d = out_root.join(dir);
        if d.exists() {
            std::fs::remove_dir_all(&d).expect("clean output dir");
        }
    }
    let mut files = write_defs(&outs, &shared);
    for (vi, out) in outs.iter().enumerate() {
        files.extend(write_version(vi, out, &shared));
    }
    files.push((
        "versions_table.rs".into(),
        versions_table(&proto_versions),
    ));
    for (path, code) in &files {
        let p = out_root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, code).unwrap();
    }
    let mut roots: Vec<PathBuf> = vec![out_root.join("defs/mod.rs"), out_root.join("versions_table.rs")];
    roots.extend(VERSIONS.iter().map(|s| out_root.join(s.module).join("mod.rs")));
    let status = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .args(&roots)
        .status()
        .expect("run rustfmt");
    assert!(status.success(), "rustfmt failed");
    share::report(&outs, &shared);
}

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read_json(p: &Path) -> Value {
    let s = std::fs::read_to_string(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("parse {}: {e}", p.display()))
}

/// Returns `<cache>/package/minecraft-data/data`, downloading and verifying if needed.
/// The cache directory can be overridden with `GHAST_MCDATA_CACHE`.
fn ensure_data(root: &Path) -> PathBuf {
    let cache = std::env::var_os("GHAST_MCDATA_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(".cache/minecraft-data").join(MCDATA_VERSION));
    let data = cache.join("package/minecraft-data/data");
    if data.join("dataPaths.json").exists() {
        return data;
    }
    std::fs::create_dir_all(&cache).unwrap();
    let tgz = cache.join(format!("minecraft-data-{MCDATA_VERSION}.tgz"));
    if !tgz.exists() {
        println!("downloading {MCDATA_URL}");
        let status = Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&tgz)
            .arg(MCDATA_URL)
            .status()
            .expect("run curl");
        assert!(status.success(), "download failed");
    }
    let bytes = std::fs::read(&tgz).unwrap();
    let digest = Sha512::digest(&bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    if hex != MCDATA_SHA512 {
        let _ = std::fs::remove_file(&tgz);
        panic!("sha512 mismatch for {}: {hex}", tgz.display());
    }
    let status = Command::new("tar")
        .arg("xzf")
        .arg(&tgz)
        .arg("-C")
        .arg(&cache)
        .status()
        .expect("run tar");
    assert!(status.success(), "extract failed");
    data
}

fn header(source: &str) -> String {
    format!(
        "// @generated by ghast-codegen-protocol from PrismarineJS minecraft-data {MCDATA_VERSION}\n\
         // ({source}). Do not edit; run `cargo run -p ghast-codegen-protocol`.\n\
         // minecraft-data is MIT licensed, Copyright (c) 2015 PrismarineJS; see the NOTICE file\n\
         // in crates/ghast-protocol/src/generated/.\n\n"
    )
}

const ALLOWS: &str = "#![allow(\n    unused_imports,\n    unused_variables,\n    unused_mut,\n    unused_parens,\n    unused_braces,\n    clippy::all,\n    clippy::pedantic,\n    non_camel_case_types,\n    non_snake_case\n)]\n";

const IMPORTS: &str = "use bytes::{BufMut, Bytes, BytesMut};\n\
use crate::codec::{Decode, DecodeResult, Encode, EncodeResult, McWrite, Reader, decode_exact};\n\
use crate::error::{DecodeError, EncodeError};\n\
use crate::nbt::{AnonNbt, RawNbt};\n\
use crate::packet::{Direction, PacketSet, RawPacket, State};\n\
use crate::types::{BlockPos, ByteString, Holder, HolderSet, LpVec3, PositionLayout, Uuid};\n\
#[cfg(feature = \"testgen\")]\n\
use crate::testgen::{Gen, Generate};\n";

/// What an item is, for the per-version layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A packet: `(minecraft-data name, id)`.
    Packet { name: String, id: i64 },
    /// A standalone named type (`types` scope).
    Named { name: String },
    /// A nested struct/enum/bitfield/mapper.
    Nested,
}

/// One emitted item of one version.
#[derive(Debug, Clone)]
pub struct VItem {
    pub item: Item,
    pub kind: Kind,
}

/// Packet set of one state and direction.
pub struct SetInfo {
    /// protocol.json state key.
    pub state: &'static str,
    pub state_mod: &'static str,
    pub state_pascal: &'static str,
    pub dir_mod: &'static str,
    pub dir_pascal: &'static str,
    /// `(minecraft-data name, Rust name, id)` ascending by id.
    pub packets: Vec<(String, String, i64)>,
}

/// Everything emitted for one version.
pub struct VersionOut {
    pub spec: &'static VersionSpec,
    pub source: String,
    /// Items in emission order; keys `(scope, name)` are unique.
    pub items: Vec<VItem>,
    pub sets: Vec<SetInfo>,
}

impl VersionOut {
    pub fn key_index(&self) -> BTreeMap<(String, String), usize> {
        self.items
            .iter()
            .enumerate()
            .map(|(i, v)| ((v.item.scope.clone(), v.item.name.clone()), i))
            .collect()
    }
}

fn emit_version(spec: &'static VersionSpec, proto: &Value, source: String) -> VersionOut {
    let types = proto["types"].as_object().expect("types");
    let mut resolver = Resolver::new(types);
    resolver
        .renames
        .insert("entityMetadataItem".into(), "EntityMetadataValue".into());
    let variant_names: BTreeMap<String, String> = spec
        .variant_names
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    resolver.variant_names = variant_names.clone();
    for (path, cond, hints, doc) in spec.presence {
        resolver.presence.insert(
            path.to_string(),
            ir::PresenceOverride {
                cond: cond.to_string(),
                hints: hints.iter().map(|h| h.to_string()).collect(),
                doc: doc.to_string(),
            },
        );
    }

    // Standalone named container types.
    let named_list: Vec<(String, String)> = resolver
        .named
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut named_roots = Vec::new();
    for (name, rust) in &named_list {
        let def = types[name].clone();
        named_roots.push(resolver.resolve_root(rust, &format!("types.{name}"), &def, true));
    }
    ir::box_cycles(&mut named_roots, true);
    let named_roots: Vec<(String, ir::Container)> = named_list
        .iter()
        .map(|(n, _)| n.clone())
        .zip(named_roots)
        .collect();

    // Packets per state and direction.
    let mut sets = Vec::new();
    for &(state, state_mod, state_pascal) in states_of(spec) {
        for &(dir_key, dir_mod, dir_pascal) in DIRECTIONS {
            let ns = &proto[state][dir_key]["types"];
            assert!(!ns.is_null(), "{}: no {state}.{dir_key}", spec.name);
            // State-local types; the container ones become named types of this module.
            let local_list = resolver.set_local(ns.as_object());
            let mut local_roots: Vec<ir::Container> = local_list
                .iter()
                .map(|(name, rust)| {
                    resolver.resolve_root(rust, &format!("{state}.{dir_mod}.{name}"), &ns[name], false)
                })
                .collect();
            ir::box_cycles(&mut local_roots, false);
            let local_roots: Vec<(String, ir::Container)> = local_list
                .iter()
                .map(|(n, _)| n.clone())
                .zip(local_roots)
                .collect();
            let packet = &ns["packet"];
            let mapper = &packet[1][0]["type"][1]["mappings"];
            let mut rows = Vec::new();
            for (id, name) in mapper.as_object().expect("packet mappings") {
                let name = name.as_str().unwrap().to_string();
                let id = ir::parse_int(id).expect("packet id");
                // The packet switch names each packet's type: usually `packet_<name>` in
                // this namespace, sometimes a shared global `packet_common_*` or `void`.
                let tname = packet[1][1]["type"][1]["fields"][name.as_str()]
                    .as_str()
                    .unwrap_or_else(|| panic!("{state}.{dir_key}: no type for {name}"));
                let empty = serde_json::json!(["container", []]);
                let def = if tname == "void" {
                    &empty
                } else if !ns[tname].is_null() {
                    &ns[tname]
                } else {
                    &proto["types"][tname]
                };
                assert!(!def.is_null(), "{state}.{dir_key}: missing {tname}");
                let rust = pascal(&name);
                let root =
                    resolver.resolve_root(&rust, &format!("{state}.{dir_mod}.{name}"), def, false);
                rows.push((id, name, rust, root));
            }
            rows.sort_by_key(|r| r.0);
            sets.push((state, state_mod, state_pascal, dir_mod, dir_pascal, rows, local_roots));
        }
    }
    resolver.set_local(None);
    resolver.assert_presence_used();

    let mut em = Emitter::new(spec.layout, spec.slot, variant_names);
    for (_, c) in named_roots.iter().chain(sets.iter().flat_map(|s| s.6.iter())) {
        let prev = em.named.insert(c.rust_name.clone(), c);
        assert!(
            prev.is_none_or(|p| p.same_shape_root(c)),
            "{}: two named types {}",
            spec.name,
            c.rust_name
        );
    }
    for (_, c) in &named_roots {
        em.collect_hints(c);
    }
    for set in &sets {
        for (_, c) in &set.6 {
            em.collect_hints(c);
        }
        for row in &set.5 {
            em.collect_hints(&row.3);
        }
    }

    let mut items: Vec<VItem> = Vec::new();
    let mut set_infos = Vec::new();
    for (state, state_mod, state_pascal, dir_mod, dir_pascal, rows, locals) in &sets {
        let scope = format!("{state_mod}/{dir_mod}");
        em.begin_scope(&scope);
        for (name, c) in locals {
            let text = named_text(&mut em, name, c);
            for it in em.items.drain(..) {
                items.push(VItem {
                    item: it,
                    kind: Kind::Nested,
                });
            }
            items.push(VItem {
                item: Item {
                    scope: scope.clone(),
                    name: c.rust_name.clone(),
                    text,
                },
                kind: Kind::Nested,
            });
        }
        for (id, name, rust, c) in rows {
            let doc = format!("`{name}` ({dir_mod} {state}).");
            let mut text = em.root_struct(c, &doc, "<const V: i32>");
            let dec = em.root_decode(c);
            let enc = em.root_encode(c);
            let gen_ = em.root_generate(c);
            let _ = write!(
                text,
                "{CFG}impl<const V: i32> Decode for {rust}<V> {{\n    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {{ Ok({dec}) }}\n}}\n\
                 {CFG}impl<const V: i32> Encode for {rust}<V> {{\n    fn encode(&self, w: &mut BytesMut) -> EncodeResult {{ {enc} Ok(()) }}\n}}\n\
                 {CFG}#[cfg(feature = \"testgen\")]\nimpl<const V: i32> Generate for {rust}<V> {{\n    fn generate(g: &mut Gen) -> Self {{ {gen_} }}\n}}\n"
            );
            // Nested definitions registered while emitting this packet come first.
            for it in em.items.drain(..) {
                items.push(VItem {
                    item: it,
                    kind: Kind::Nested,
                });
            }
            items.push(VItem {
                item: Item {
                    scope: scope.clone(),
                    name: rust.clone(),
                    text,
                },
                kind: Kind::Packet {
                    name: name.clone(),
                    id: *id,
                },
            });
        }
        set_infos.push(SetInfo {
            state,
            state_mod,
            state_pascal,
            dir_mod,
            dir_pascal,
            packets: rows
                .iter()
                .map(|(id, name, rust, _)| (name.clone(), rust.clone(), *id))
                .collect(),
        });
    }

    // Named types (after packets: packets may register shared nominal types).
    em.begin_scope("types");
    for (name, c) in &named_roots {
        let text = named_text(&mut em, name, c);
        let r = &c.rust_name;
        for it in em.items.drain(..) {
            items.push(VItem {
                item: it,
                kind: Kind::Nested,
            });
        }
        items.push(VItem {
            item: Item {
                scope: "types".into(),
                name: r.clone(),
                text,
            },
            kind: Kind::Named { name: name.clone() },
        });
    }
    assert!(em.items.is_empty());

    // Keys must be unique per version.
    let mut seen = BTreeSet::new();
    for v in &items {
        assert!(
            seen.insert((v.item.scope.clone(), v.item.name.clone())),
            "{}: duplicate definition {}::{}",
            spec.name,
            v.item.scope,
            v.item.name
        );
    }
    VersionOut {
        spec,
        source,
        items,
        sets: set_infos,
    }
}

/// Struct and impls of a standalone named type.
fn named_text<'t>(em: &mut Emitter<'t>, name: &str, c: &'t ir::Container) -> String {
    let doc = format!("Named type `{name}`.");
    let mut text = em.root_struct(c, &doc, "");
    let dec = em.root_decode(c);
    let enc = em.root_encode(c);
    let gen_ = em.root_generate(c);
    let r = &c.rust_name;
    let _ = write!(
        text,
        "{CFG}impl Decode for {r} {{\n    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {{ Ok({dec}) }}\n}}\n\
         {CFG}impl Encode for {r} {{\n    fn encode(&self, w: &mut BytesMut) -> EncodeResult {{ {enc} Ok(()) }}\n}}\n\
         {CFG}#[cfg(feature = \"testgen\")]\nimpl Generate for {r} {{\n    fn generate(g: &mut Gen) -> Self {{ {gen_} }}\n}}\n"
    );
    text
}

/// `"play/clientbound"` -> `"play::clientbound"`.
fn scope_path(scope: &str) -> String {
    scope.replace('/', "::")
}

/// `#[cfg(...)]` condition for a set of version indices.
fn cfg_of(versions: &BTreeSet<usize>) -> String {
    let feats: Vec<String> = versions
        .iter()
        .map(|&v| format!("feature = \"{}\"", VERSIONS[v].feature))
        .collect();
    if feats.len() == 1 {
        feats[0].clone()
    } else {
        format!("any({})", feats.join(", "))
    }
}

/// Replaces placeholder references of an item of version `vi` by full paths.
fn resolve_refs(text: &str, vi: usize, shared: &share::Shared) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('\u{27ea}') {
        out.push_str(&rest[..start]);
        let after = &rest[start + '\u{27ea}'.len_utf8()..];
        let end = after.find('\u{27eb}').expect("unterminated reference");
        let inner = &after[..end];
        let (scope, name) = inner.split_once('|').expect("reference");
        let owner = shared.owner(vi, scope, name);
        let _ = write!(
            out,
            "crate::defs::p{}::{}::{name}",
            VERSIONS[owner].protocol,
            scope_path(scope)
        );
        rest = &after[end + '\u{27eb}'.len_utf8()..];
    }
    out.push_str(rest);
    out
}

/// The `defs` tree: every distinct definition once, in its owner's module.
fn write_defs(outs: &[VersionOut], shared: &share::Shared) -> Vec<(String, String)> {
    // owner version -> scope -> items (in the owner's emission order).
    let mut by_owner: BTreeMap<usize, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    let mut owner_cfg: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (vi, out) in outs.iter().enumerate() {
        for v in &out.items {
            let users = shared.users(vi, &v.item.scope, &v.item.name);
            if shared.owner(vi, &v.item.scope, &v.item.name) != vi {
                continue;
            }
            let text = resolve_refs(&v.item.text, vi, shared);
            let text = text.replace("\u{27e6}cfg\u{27e7}", &cfg_of(users));
            by_owner
                .entry(vi)
                .or_default()
                .entry(v.item.scope.clone())
                .or_default()
                .push(text);
            owner_cfg.entry(vi).or_default().extend(users.iter().copied());
        }
    }
    let mut files = Vec::new();
    let mut root = header("protocol.json of every supported version");
    root.push_str(
        "//! Packet and type definitions, each exactly once.\n\
         //!\n\
         //! A definition lives in the module of the first protocol that has it (`p47`, `p340`,\n\
         //! …) and is shared by every later version whose wire layout and Rust code for it\n\
         //! are identical. Packet structs take the protocol number as a const parameter, so\n\
         //! one definition serves versions with different packet ids. Use the version\n\
         //! modules (`crate::v1_8`, …): they alias and re-export what they use from here.\n\n",
    );
    for (&vi, scopes) in &by_owner {
        let spec = VERSIONS[vi].protocol;
        let cfg = cfg_of(&owner_cfg[&vi]);
        let _ = writeln!(
            root,
            "/// Definitions first used by protocol {spec} (Minecraft {}).\n#[cfg({cfg})]\npub mod p{spec};",
            VERSIONS[vi].covers
        );
        let dir = format!("defs/p{spec}");
        let hdr = header(&outs[vi].source);
        // Module tree: types + states with directions.
        let mut states: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for scope in scopes.keys() {
            if let Some((st, d)) = scope.split_once('/') {
                states.entry(st.to_string()).or_default().insert(d.to_string());
            }
        }
        let mut m = hdr.clone();
        let _ = writeln!(
            m,
            "//! Definitions first used by protocol {spec} (Minecraft {}).\n",
            VERSIONS[vi].covers
        );
        for (scope, texts) in scopes {
            let (path, doc) = match scope.split_once('/') {
                None => (format!("{dir}/types.rs"), "Types shared by several packets.".to_string()),
                Some((st, d)) => (
                    format!("{dir}/{st}/{d}.rs"),
                    format!("{} packets of the {st} state.", capitalize(d)),
                ),
            };
            let mut f = hdr.clone();
            let _ = writeln!(f, "//! {doc}\n");
            f.push_str(ALLOWS);
            f.push_str(IMPORTS);
            f.push('\n');
            for t in texts {
                f.push_str(t);
                f.push('\n');
            }
            files.push((path, f));
        }
        if scopes.contains_key("types") {
            let users = scope_users(outs, shared, vi, |s| s == "types");
            let _ = writeln!(m, "#[cfg({})]\npub mod types;", cfg_of(&users));
        }
        for (st, dirs) in &states {
            let users = scope_users(outs, shared, vi, |s| s.split('/').next() == Some(st));
            let _ = writeln!(m, "#[cfg({})]\npub mod {st};", cfg_of(&users));
            let mut sm = hdr.clone();
            let _ = writeln!(sm, "//! Definitions of the {st} state.\n");
            for d in dirs {
                let scope = format!("{st}/{d}");
                let users = scope_users(outs, shared, vi, |s| s == scope);
                let _ = writeln!(sm, "#[cfg({})]\npub mod {d};", cfg_of(&users));
            }
            files.push((format!("{dir}/{st}/mod.rs"), sm));
        }
        files.push((format!("{dir}/mod.rs"), m));
    }
    files.push(("defs/mod.rs".into(), root));
    files
}

/// Versions using any item owned by `owner` in a scope matching `pred`.
fn scope_users(
    outs: &[VersionOut],
    shared: &share::Shared,
    owner: usize,
    pred: impl Fn(&str) -> bool,
) -> BTreeSet<usize> {
    let mut set = BTreeSet::new();
    for v in &outs[owner].items {
        if pred(&v.item.scope) && shared.owner(owner, &v.item.scope, &v.item.name) == owner {
            set.extend(shared.users(owner, &v.item.scope, &v.item.name).iter().copied());
        }
    }
    set
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Path of the definition an item of version `vi` uses.
fn def_path(vi: usize, scope: &str, name: &str, shared: &share::Shared) -> String {
    let owner = shared.owner(vi, scope, name);
    format!(
        "crate::defs::p{}::{}::{name}",
        VERSIONS[owner].protocol,
        scope_path(scope)
    )
}

/// The thin per-version layer.
fn write_version(vi: usize, out: &VersionOut, shared: &share::Shared) -> Vec<(String, String)> {
    let spec = out.spec;
    let hdr = header(&format!("{}, Minecraft {}", out.source, spec.name));
    let mut files = Vec::new();
    let proto = spec.protocol;

    for set in &out.sets {
        let scope = format!("{}/{}", set.state_mod, set.dir_mod);
        let enum_name = format!("{}{}Packet", set.dir_pascal, set.state_pascal);
        let mut body = String::new();
        body.push_str("/// Packet ids.\npub mod id {\n");
        for (name, _, id) in &set.packets {
            let _ = writeln!(
                body,
                "    /// `{name}`\n    pub const {}: i32 = {id:#04x};",
                screaming(name)
            );
        }
        body.push_str("}\n\n");
        // Re-exports of nested types of this scope.
        let nested: Vec<&VItem> = out
            .items
            .iter()
            .filter(|v| v.item.scope == scope && v.kind == Kind::Nested)
            .collect();
        for v in &nested {
            let _ = writeln!(
                body,
                "pub use {};",
                def_path(vi, &scope, &v.item.name, shared)
            );
        }
        body.push('\n');
        for (name, rust, id) in &set.packets {
            let path = def_path(vi, &scope, rust, shared);
            let _ = write!(
                body,
                "/// `{name}` ({dir} {state}, id {id:#04x}).\npub type {rust} = {path}<{proto}>;\n\
                 impl crate::packet::Packet for {rust} {{\n    const ID: i32 = id::{ids};\n    const NAME: &'static str = {name:?};\n    const STATE: State = State::{sp};\n    const DIRECTION: Direction = Direction::{dp};\n}}\n\
                 impl From<{rust}> for {enum_name} {{\n    fn from(p: {rust}) -> Self {{ Self::{rust}(p) }}\n}}\n\n",
                dir = set.dir_mod,
                state = set.state,
                ids = screaming(name),
                sp = set.state_pascal,
                dp = set.dir_pascal,
            );
        }
        body.push_str(&packet_enum(
            &enum_name,
            set.state_pascal,
            set.dir_pascal,
            &set.packets,
        ));
        let mut file = hdr.clone();
        let _ = writeln!(
            file,
            "//! {} packets of the {} state.\n",
            set.dir_pascal, set.state
        );
        file.push_str(ALLOWS);
        file.push_str(IMPORTS);
        file.push('\n');
        file.push_str(&body);
        files.push((format!("{}/{}/{}.rs", spec.module, set.state_mod, set.dir_mod), file));
    }
    for &(state, state_mod, _) in states_of(spec) {
        let file = format!(
            "{hdr}//! The {state} state.\n\npub mod clientbound;\npub mod serverbound;\n"
        );
        files.push((format!("{}/{state_mod}/mod.rs", spec.module), file));
    }

    // Types module: the slot alias and re-exports of version-wide types.
    let mut t = hdr.clone();
    t.push_str("//! Types shared by several packets of this version.\n\n");
    if let Some(slot) = spec.slot {
        let _ = writeln!(t, "/// Item slot in this version's format.\npub type Slot = {slot};\n");
    }
    for v in out.items.iter().filter(|v| v.item.scope == "types") {
        let _ = writeln!(t, "pub use {};", def_path(vi, "types", &v.item.name, shared));
    }
    files.push((format!("{}/types.rs", spec.module), t));

    // Version root.
    let mut m = hdr.clone();
    let state_mods: Vec<&str> = states_of(spec).map(|s| s.1).collect();
    let mut mods: Vec<&str> = state_mods.clone();
    mods.push("types");
    mods.sort();
    let _ = write!(
        m,
        "//! Protocol {proto} (Minecraft {covers}), generated from minecraft-data.\n\
         //!\n\
         //! One module per connection state, each with `clientbound` and `serverbound`\n\
         //! packet types, an enum of all packets and the packet `id` constants. The packet\n\
         //! types are aliases of definitions in [`crate::defs`] that may be shared with\n\
         //! other versions.\n\n",
        covers = spec.covers,
    );
    for md in &mods {
        let _ = writeln!(m, "pub mod {md};");
    }
    let conf = |dir: &str| {
        if spec.configuration {
            format!("configuration::{dir}::{}ConfigurationPacket", capitalize(dir))
        } else {
            "crate::packet::NoPackets".to_string()
        }
    };
    let _ = write!(
        m,
        "\n/// Marker type for protocol {proto} (Minecraft {covers}).\n\
         #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]\npub struct {marker};\n\n\
         impl crate::packet::Version for {marker} {{\n\
         \x20   const PROTOCOL: i32 = {proto};\n\
         \x20   const NAME: &'static str = {name:?};\n\
         \x20   const POSITION_LAYOUT: crate::types::PositionLayout = crate::types::PositionLayout::{layout};\n\
         \x20   const INFO: &'static crate::versions::ProtocolVersion = &crate::versions::TABLE[{vi}];\n\
         \x20   type HandshakingServerbound = handshaking::serverbound::ServerboundHandshakingPacket;\n\
         \x20   type StatusClientbound = status::clientbound::ClientboundStatusPacket;\n\
         \x20   type StatusServerbound = status::serverbound::ServerboundStatusPacket;\n\
         \x20   type LoginClientbound = login::clientbound::ClientboundLoginPacket;\n\
         \x20   type LoginServerbound = login::serverbound::ServerboundLoginPacket;\n\
         \x20   type ConfigurationClientbound = {cc};\n\
         \x20   type ConfigurationServerbound = {cs};\n\
         \x20   type PlayClientbound = play::clientbound::ClientboundPlayPacket;\n\
         \x20   type PlayServerbound = play::serverbound::ServerboundPlayPacket;\n\
         }}\n",
        covers = spec.covers,
        marker = spec.marker,
        name = spec.name,
        layout = spec.layout,
        cc = conf("clientbound"),
        cs = conf("serverbound"),
    );
    files.push((format!("{}/mod.rs", spec.module), m));
    files
}

/// Release game versions per protocol number, from minecraft-data's
/// `protocolVersions.json` (newest first there; returned oldest first).
fn game_versions(pv: &Value, protocol: i32) -> Vec<String> {
    let mut out: Vec<String> = pv
        .as_array()
        .expect("protocolVersions")
        .iter()
        .filter(|e| e["version"].as_i64() == Some(i64::from(protocol)))
        .filter(|e| match e["releaseType"].as_str() {
            Some(t) => t == "release",
            None => {
                let n = e["minecraftVersion"].as_str().unwrap_or("");
                !n.is_empty() && n.chars().all(|c| c.is_ascii_digit() || c == '.')
            }
        })
        .map(|e| e["minecraftVersion"].as_str().unwrap().to_string())
        .collect();
    out.dedup();
    out.reverse();
    assert!(!out.is_empty(), "no release names for protocol {protocol}");
    out
}

fn versions_table(pv: &Value) -> String {
    let mut s = header("pc/common/protocolVersions.json and the codegen's version list");
    s.push_str(
        "// Included by `crate::versions`.\n\n\
         /// Every protocol this build knows, ascending by protocol number; rows of versions\n\
         /// whose feature is off have `enabled == false`.\n\
         pub const TABLE: &[ProtocolVersion] = &[\n",
    );
    for spec in VERSIONS {
        let gv = game_versions(pv, spec.protocol);
        let _ = writeln!(
            s,
            "    ProtocolVersion {{ protocol: {}, module: {:?}, feature: {:?}, name: {:?}, game_versions: &{:?}, position_layout: PositionLayout::{}, configuration: {}, enabled: cfg!(feature = {:?}) }},",
            spec.protocol,
            spec.module,
            spec.feature,
            spec.name,
            gv,
            spec.layout,
            spec.configuration,
            spec.feature
        );
    }
    s.push_str("];\n");
    s
}

fn packet_enum(name: &str, state: &str, dir: &str, packets: &[(String, String, i64)]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "/// Every {dir} packet of the {state} state.");
    s.push_str("#[derive(Debug, Clone, PartialEq)]\n");
    let _ = writeln!(s, "pub enum {name} {{");
    for (n, rust, _) in packets {
        let _ = writeln!(s, "    /// `{n}`\n    {rust}({rust}),");
    }
    s.push_str("}\n\n");
    let arms = |f: &dyn Fn(&str, &str) -> String| {
        packets
            .iter()
            .map(|(n, r, _)| f(n, r))
            .collect::<String>()
    };
    let _ = write!(
        s,
        "impl PacketSet for {name} {{\n\
         \x20   const STATE: State = State::{state};\n\
         \x20   const DIRECTION: Direction = Direction::{dir};\n\
         \x20   fn decode_raw(raw: &RawPacket) -> DecodeResult<Self> {{\n        match raw.id {{\n{dec}            other => Err(DecodeError::UnknownPacketId(other)),\n        }}\n    }}\n\
         \x20   fn id(&self) -> i32 {{\n        match *self {{\n{ids}        }}\n    }}\n\
         \x20   fn name(&self) -> &'static str {{\n        match *self {{\n{names}        }}\n    }}\n\
         \x20   fn encode_body(&self, w: &mut BytesMut) -> EncodeResult {{\n        match *self {{\n{enc}        }}\n    }}\n\
         \x20   fn name_of(id: i32) -> Option<&'static str> {{\n        match id {{\n{name_of}            _ => None,\n        }}\n    }}\n\
         }}\n\n",
        dec = arms(&|n, r| format!(
            "            id::{} => decode_exact(&raw.body).map(Self::{r}),\n",
            screaming(n)
        )),
        ids = arms(&|n, r| format!("            Self::{r}(_) => id::{},\n", screaming(n))),
        names = arms(&|n, r| format!("            Self::{r}(_) => {n:?},\n")),
        enc = arms(&|_, r| format!("            Self::{r}(ref p) => p.encode(w),\n")),
        name_of = arms(&|n, _| format!("            id::{} => Some({n:?}),\n", screaming(n))),
    );
    let _ = write!(
        s,
        "impl {name} {{\n\
         \x20   /// All packet ids of this set, ascending.\n\
         \x20   pub const IDS: &'static [i32] = &[{ids}];\n\
         \x20   /// Generates a random packet with the given id (test support).\n\
         \x20   #[cfg(feature = \"testgen\")]\n\
         \x20   pub fn generate_by_id(id: i32, g: &mut Gen) -> Option<Self> {{\n        match id {{\n{gen_}            _ => None,\n        }}\n    }}\n\
         }}\n",
        ids = packets
            .iter()
            .map(|(n, _, _)| format!("id::{}", screaming(n)))
            .collect::<Vec<_>>()
            .join(", "),
        gen_ = arms(&|n, r| format!(
            "            id::{} => Some(Self::{r}(<{r} as Generate>::generate(g))),\n",
            screaming(n)
        )),
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every patch still matches the pinned minecraft-data (needs the cache; run
    /// `cargo run -p ghast-codegen-protocol` once). Fails once upstream fixes a patched
    /// value, so the patch gets removed.
    #[test]
    fn patches_still_apply() {
        let root = workspace_root();
        let data = root
            .join(".cache/minecraft-data")
            .join(MCDATA_VERSION)
            .join("package/minecraft-data/data");
        let Ok(paths) = std::fs::read_to_string(data.join("dataPaths.json")) else {
            eprintln!("skipped: no minecraft-data cache");
            return;
        };
        let paths: Value = serde_json::from_str(&paths).unwrap();
        for p in PATCHES {
            let dir = paths["pc"][p.data_key]["protocol"].as_str().unwrap();
            let mut proto = read_json(&data.join(dir).join("protocol.json"));
            apply_patches(p.data_key, &mut proto);
            let now = proto.pointer(p.pointer).unwrap();
            assert_eq!(*now, serde_json::from_str::<Value>(p.replace).unwrap());
        }
    }
}
