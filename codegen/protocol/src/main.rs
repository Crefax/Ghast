//! Generates `crates/ghast-protocol/src/generated/` from PrismarineJS minecraft-data.
//!
//! Usage: `cargo run -p ghast-codegen-protocol` (from anywhere in the workspace).
//!
//! minecraft-data 3.119.0 is downloaded once from the npm registry into the git-ignored
//! `.cache/minecraft-data/3.119.0/` (sha512-verified), then `dataPaths.json` is used to find
//! each version's `protocol.json`. The output is formatted with `rustfmt` and committed.

mod emit;
mod ir;
mod names;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use sha2::{Digest, Sha512};

use emit::Emitter;
use ir::Resolver;
use names::{pascal, screaming};

const MCDATA_VERSION: &str = "3.119.0";
const MCDATA_URL: &str = "https://registry.npmjs.org/minecraft-data/-/minecraft-data-3.119.0.tgz";
/// sha512 of the npm tarball (matches the registry's `dist.integrity`).
const MCDATA_SHA512: &str = "444968d28d91a28133a5e40ef90107cd398755380e75b6e8986418066588694d69ba9126008b506fe2e92722f61035e6a4a89c394e2cc2d055cf021669ee65e6";

struct VersionSpec {
    module: &'static str,
    feature: &'static str,
    marker: &'static str,
    name: &'static str,
    covers: &'static str,
    data_key: &'static str,
    protocol: i32,
    layout: &'static str,
    slot: &'static str,
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
        slot: "crate::types::slot::legacy::Slot",
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
        module: "v1_16_5",
        feature: "v1_16_5",
        marker: "V1_16_5",
        name: "1.16.5",
        covers: "1.16.4 – 1.16.5",
        data_key: "1.16.5",
        protocol: 754,
        layout: "Xzy",
        slot: "crate::types::slot::modern::Slot",
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
];

const STATES: &[(&str, &str)] = &[
    ("handshaking", "Handshaking"),
    ("status", "Status"),
    ("login", "Login"),
    ("play", "Play"),
];
const DIRECTIONS: &[(&str, &str, &str)] = &[
    ("toClient", "clientbound", "Clientbound"),
    ("toServer", "serverbound", "Serverbound"),
];

fn main() {
    let root = workspace_root();
    let data_dir = ensure_data(&root);
    let paths: Value = read_json(&data_dir.join("dataPaths.json"));
    let out_root = root.join("crates/ghast-protocol/src/generated");
    for spec in VERSIONS {
        let proto_dir = paths["pc"][spec.data_key]["protocol"]
            .as_str()
            .unwrap_or_else(|| panic!("no protocol path for {}", spec.data_key));
        let proto_file = data_dir.join(proto_dir).join("protocol.json");
        let proto: Value = read_json(&proto_file);
        let rel = format!("data/{proto_dir}/protocol.json");
        let files = generate_version(spec, &proto, &rel);
        let dir = out_root.join(spec.module);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).expect("clean output dir");
        }
        for (path, code) in &files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, code).unwrap();
        }
        let status = Command::new("rustfmt")
            .args(["--edition", "2024"])
            .arg(dir.join("mod.rs"))
            .status()
            .expect("run rustfmt");
        assert!(status.success(), "rustfmt failed for {}", spec.module);
        println!(
            "generated {} ({} files) from {rel}",
            spec.module,
            files.len()
        );
    }
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

fn header(spec: &VersionSpec, rel: &str) -> String {
    format!(
        "// @generated by ghast-codegen-protocol from PrismarineJS minecraft-data {MCDATA_VERSION}\n\
         // ({rel}, Minecraft {name}). Do not edit; run `cargo run -p ghast-codegen-protocol`.\n\
         // minecraft-data is MIT licensed, Copyright (c) 2015 PrismarineJS; see ../NOTICE.\n\n",
        name = spec.name
    )
}

const ALLOWS: &str = "#![allow(\n    unused_imports,\n    unused_variables,\n    unused_mut,\n    unused_parens,\n    unused_braces,\n    clippy::all,\n    clippy::pedantic,\n    non_camel_case_types,\n    non_snake_case\n)]\n";

const IMPORTS: &str = "use bytes::{BufMut, Bytes, BytesMut};\n\
use crate::codec::{Decode, DecodeResult, Encode, EncodeResult, McWrite, Reader, decode_exact};\n\
use crate::error::{DecodeError, EncodeError};\n\
use crate::nbt::RawNbt;\n\
use crate::packet::{Direction, PacketSet, RawPacket, State};\n\
use crate::types::{BlockPos, ByteString, PositionLayout, Uuid};\n\
#[cfg(feature = \"testgen\")]\n\
use crate::testgen::{Gen, Generate};\n";

struct PacketInfo {
    id: i64,
    name: String,
    rust: String,
}

fn generate_version(spec: &VersionSpec, proto: &Value, rel: &str) -> Vec<(String, String)> {
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
        named_roots.push((
            name.clone(),
            resolver.resolve_root(rust, &format!("types.{name}"), &def, true),
        ));
    }

    // Packets per state and direction.
    let mut sets = Vec::new();
    for (state, state_pascal) in STATES {
        for (dir_key, dir_mod, dir_pascal) in DIRECTIONS {
            let ns = &proto[*state][*dir_key]["types"];
            let packet = &ns["packet"];
            let mapper = &packet[1][0]["type"][1]["mappings"];
            let mut infos = Vec::new();
            let mut roots = Vec::new();
            for (id, name) in mapper.as_object().expect("packet mappings") {
                let name = name.as_str().unwrap().to_string();
                let id = ir::parse_int(id).expect("packet id");
                let def = &ns[format!("packet_{name}")];
                assert!(!def.is_null(), "{state}.{dir_key}: missing packet_{name}");
                let rust = pascal(&name);
                let root =
                    resolver.resolve_root(&rust, &format!("{state}.{dir_mod}.{name}"), def, false);
                infos.push(PacketInfo { id, name, rust });
                roots.push(root);
            }
            let mut order: Vec<usize> = (0..infos.len()).collect();
            order.sort_by_key(|&i| infos[i].id);
            let infos: Vec<PacketInfo> = order
                .iter()
                .map(|&i| PacketInfo {
                    id: infos[i].id,
                    name: infos[i].name.clone(),
                    rust: infos[i].rust.clone(),
                })
                .collect();
            let roots: Vec<_> = order.iter().map(|&i| roots[i].clone()).collect();
            sets.push((*state, *state_pascal, *dir_mod, *dir_pascal, infos, roots));
        }
    }

    resolver.assert_presence_used();

    let mut em = Emitter::new(spec.layout, variant_names);
    for (_, c) in &named_roots {
        em.named.insert(c.rust_name.clone(), c);
    }
    for (_, c) in &named_roots {
        em.collect_hints(c);
    }
    for set in &sets {
        for c in &set.5 {
            em.collect_hints(c);
        }
    }

    let hdr = header(spec, rel);
    let mut files = Vec::new();

    // Packet modules.
    for (state, state_pascal, dir_mod, dir_pascal, infos, roots) in &sets {
        em.in_types = false;
        em.local_defs.clear();
        let mut body = String::new();
        let enum_name = format!("{dir_pascal}{state_pascal}Packet");
        // Ids.
        body.push_str("/// Packet ids.\npub mod id {\n");
        for p in infos {
            let _ = writeln!(
                body,
                "    /// `{}`\n    pub const {}: i32 = {:#04x};",
                p.name,
                screaming(&p.name),
                p.id
            );
        }
        body.push_str("}\n\n");
        for (p, c) in infos.iter().zip(roots) {
            let doc = format!("`{}` ({dir_mod} {state}, id {:#04x}).", p.name, p.id);
            body.push_str(&em.root_struct(c, &doc));
            let dec = em.root_decode(c);
            let enc = em.root_encode(c);
            let gen_ = em.root_generate(c);
            let _ = write!(
                body,
                "impl crate::packet::Packet for {r} {{\n    const ID: i32 = id::{ids};\n    const NAME: &'static str = {n:?};\n    const STATE: State = State::{state_pascal};\n    const DIRECTION: Direction = Direction::{dir_pascal};\n}}\n\
                 impl Decode for {r} {{\n    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {{ Ok({dec}) }}\n}}\n\
                 impl Encode for {r} {{\n    fn encode(&self, w: &mut BytesMut) -> EncodeResult {{ {enc} Ok(()) }}\n}}\n\
                 #[cfg(feature = \"testgen\")]\nimpl Generate for {r} {{\n    fn generate(g: &mut Gen) -> Self {{ {gen_} }}\n}}\n\
                 impl From<{r}> for {enum_name} {{\n    fn from(p: {r}) -> Self {{ Self::{r}(p) }}\n}}\n\n",
                r = p.rust,
                ids = screaming(&p.name),
                n = p.name,
            );
        }
        body.push_str(&packet_enum(&enum_name, state_pascal, dir_pascal, infos));
        let mut file = hdr.clone();
        let _ = writeln!(file, "//! {dir_pascal} packets of the {state} state.\n");
        file.push_str(ALLOWS);
        file.push_str(IMPORTS);
        let _ = writeln!(file, "use crate::{}::types;\n", spec.module);
        for d in &em.local_defs {
            file.push_str(d);
            file.push('\n');
        }
        file.push_str(&body);
        files.push((format!("{state}/{dir_mod}.rs"), file));
    }
    for (state, _) in STATES {
        let file =
            format!("{hdr}//! The {state} state.\n\npub mod clientbound;\npub mod serverbound;\n");
        files.push((format!("{state}/mod.rs"), file));
    }

    // Types module (after packets: packets may register shared nominal types).
    em.in_types = true;
    let mut tbody = String::new();
    let _ = writeln!(
        tbody,
        "/// Item slot in this version's format.\npub type Slot = {};\n",
        spec.slot
    );
    for (name, c) in &named_roots {
        let doc = format!("Named type `{name}`.");
        tbody.push_str(&em.root_struct(c, &doc));
        let dec = em.root_decode(c);
        let enc = em.root_encode(c);
        let gen_ = em.root_generate(c);
        let _ = write!(
            tbody,
            "impl Decode for {r} {{\n    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {{ Ok({dec}) }}\n}}\n\
             impl Encode for {r} {{\n    fn encode(&self, w: &mut BytesMut) -> EncodeResult {{ {enc} Ok(()) }}\n}}\n\
             #[cfg(feature = \"testgen\")]\nimpl Generate for {r} {{\n    fn generate(g: &mut Gen) -> Self {{ {gen_} }}\n}}\n\n",
            r = c.rust_name
        );
    }
    let mut tfile = hdr.clone();
    tfile.push_str("//! Types shared by several packets of this version.\n\n");
    tfile.push_str(ALLOWS);
    tfile.push_str(IMPORTS);
    let _ = writeln!(tfile, "use crate::{}::types;\n", spec.module);
    for d in &em.shared_defs {
        tfile.push_str(d);
        tfile.push('\n');
    }
    tfile.push_str(&tbody);
    files.push(("types.rs".into(), tfile));

    // Version root.
    let mut m = hdr.clone();
    let _ = write!(
        m,
        "//! Protocol {proto} (Minecraft {covers}), generated from minecraft-data.\n\
         //!\n\
         //! One module per connection state, each with `clientbound` and `serverbound`\n\
         //! packet structs, an enum of all packets and the packet `id` constants.\n\n\
         pub mod handshaking;\npub mod login;\npub mod play;\npub mod status;\npub mod types;\n\n\
         /// Marker type for protocol {proto} (Minecraft {covers}).\n\
         #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]\npub struct {marker};\n\n\
         impl crate::packet::Version for {marker} {{\n\
         \x20   const PROTOCOL: i32 = {proto};\n\
         \x20   const NAME: &'static str = {name:?};\n\
         \x20   const POSITION_LAYOUT: crate::types::PositionLayout = crate::types::PositionLayout::{layout};\n\
         \x20   type HandshakingServerbound = handshaking::serverbound::ServerboundHandshakingPacket;\n\
         \x20   type StatusClientbound = status::clientbound::ClientboundStatusPacket;\n\
         \x20   type StatusServerbound = status::serverbound::ServerboundStatusPacket;\n\
         \x20   type LoginClientbound = login::clientbound::ClientboundLoginPacket;\n\
         \x20   type LoginServerbound = login::serverbound::ServerboundLoginPacket;\n\
         \x20   type PlayClientbound = play::clientbound::ClientboundPlayPacket;\n\
         \x20   type PlayServerbound = play::serverbound::ServerboundPlayPacket;\n\
         }}\n",
        proto = spec.protocol,
        covers = spec.covers,
        marker = spec.marker,
        name = spec.name,
        layout = spec.layout,
    );
    let _ = spec.feature;
    files.push(("mod.rs".into(), m));
    files
}

fn packet_enum(name: &str, state: &str, dir: &str, infos: &[PacketInfo]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "/// Every {dir} packet of the {state} state.");
    s.push_str("#[derive(Debug, Clone, PartialEq)]\n");
    let _ = writeln!(s, "pub enum {name} {{");
    for p in infos {
        let _ = writeln!(s, "    /// `{}`\n    {}({}),", p.name, p.rust, p.rust);
    }
    s.push_str("}\n\n");
    let arms = |f: &dyn Fn(&PacketInfo) -> String| infos.iter().map(f).collect::<String>();
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
        dec = arms(&|p| format!(
            "            id::{} => decode_exact(&raw.body).map(Self::{}),\n",
            screaming(&p.name),
            p.rust
        )),
        ids = arms(&|p| format!(
            "            Self::{}(_) => id::{},\n",
            p.rust,
            screaming(&p.name)
        )),
        names = arms(&|p| format!("            Self::{}(_) => {:?},\n", p.rust, p.name)),
        enc = arms(&|p| format!("            Self::{}(ref p) => p.encode(w),\n", p.rust)),
        name_of = arms(&|p| format!(
            "            id::{} => Some({:?}),\n",
            screaming(&p.name),
            p.name
        )),
    );
    // All ids, and random generation by id for the test suites.
    let _ = write!(
        s,
        "impl {name} {{\n\
         \x20   /// All packet ids of this set, ascending.\n\
         \x20   pub const IDS: &'static [i32] = &[{ids}];\n\
         \x20   /// Generates a random packet with the given id (test support).\n\
         \x20   #[cfg(feature = \"testgen\")]\n\
         \x20   pub fn generate_by_id(id: i32, g: &mut Gen) -> Option<Self> {{\n        match id {{\n{gen_}            _ => None,\n        }}\n    }}\n\
         }}\n",
        ids = infos
            .iter()
            .map(|p| format!("id::{}", screaming(&p.name)))
            .collect::<Vec<_>>()
            .join(", "),
        gen_ = arms(&|p| format!(
            "            id::{} => Some(Self::{}(<{} as Generate>::generate(g))),\n",
            screaming(&p.name),
            p.rust,
            p.rust
        )),
    );
    s
}
