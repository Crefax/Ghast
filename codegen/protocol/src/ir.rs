//! Intermediate representation of protodef types and the resolver that builds it from
//! minecraft-data's `protocol.json`.
//!
//! Resolution inlines type aliases and parametrised types (`$compareTo`), assigns Rust
//! names to every nominal type (structs, enums, mapper newtypes) and leaves references to
//! standalone named container types as [`Ty::Named`].

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::names::{pascal, snake};

/// Built-in leaf types with hand-written Rust counterparts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prim {
    VarInt,
    VarLong,
    U8,
    I8,
    U16,
    I16,
    I32,
    I64,
    F32,
    F64,
    Bool,
    Uuid,
    String,
    ByteArray,
    RestBuffer,
    Nbt,
    OptionalNbt,
    Position,
    Slot,
}

impl Prim {
    /// Whether values of this type are plain integers (usable as switch discriminants).
    pub fn is_int(self) -> bool {
        matches!(
            self,
            Prim::VarInt
                | Prim::VarLong
                | Prim::U8
                | Prim::I8
                | Prim::U16
                | Prim::I16
                | Prim::I32
                | Prim::I64
        )
    }

    /// Value range of integer types.
    pub fn int_range(self) -> (i128, i128) {
        match self {
            Prim::U8 => (0, u8::MAX as i128),
            Prim::I8 => (i8::MIN as i128, i8::MAX as i128),
            Prim::U16 => (0, u16::MAX as i128),
            Prim::I16 => (i16::MIN as i128, i16::MAX as i128),
            Prim::VarInt | Prim::I32 => (i32::MIN as i128, i32::MAX as i128),
            Prim::VarLong | Prim::I64 => (i64::MIN as i128, i64::MAX as i128),
            _ => panic!("not an integer type: {self:?}"),
        }
    }
}

/// How an array's element count is determined.
#[derive(Debug, Clone, PartialEq)]
pub enum Count {
    /// A length prefix of this integer type.
    Prefix(Prim),
    /// A fixed number of elements.
    Fixed(usize),
    /// The value of a sibling field (protodef path).
    Field(String),
}

#[derive(Debug, Clone)]
pub struct Field {
    /// Original protodef name (empty for anonymous fields).
    pub name: String,
    /// Rust field identifier.
    pub rust: String,
    pub anon: bool,
    pub ty: Ty,
}

#[derive(Debug, Clone)]
pub struct Container {
    /// Rust struct name.
    pub rust_name: String,
    /// Defined in the version's `types` module (rooted in a named type).
    pub shared: bool,
    pub fields: Vec<Field>,
    /// Scope id used for generator hints.
    pub id: usize,
}

#[derive(Debug, Clone)]
pub struct Switch {
    pub compare_to: String,
    /// `(case key, type)` in protocol.json order.
    pub cases: Vec<(String, Ty)>,
    pub default: Ty,
    /// Rust enum name (used if the switch is not option-like).
    pub rust_name: String,
    pub shared: bool,
    /// `packet.field` path for error messages.
    pub path: String,
    /// Hand-written presence rule replacing the case keys (see [`PresenceOverride`]).
    pub presence: Option<PresenceOverride>,
}

/// Overrides when an option-like switch carries its value, for places where
/// minecraft-data's case keys do not match vanilla (e.g. 1.8 `spawn_entity.velocity` is
/// read when `objectData > 0`, not when `!= 0`).
#[derive(Debug, Clone, PartialEq)]
pub struct PresenceOverride {
    /// Rust boolean expression; `{d}` is replaced by the discriminant.
    pub cond: String,
    /// Discriminant values the test generator should favour.
    pub hints: Vec<String>,
    /// Human-readable rule for the field docs.
    pub doc: String,
}

#[derive(Debug, Clone)]
pub struct BitField {
    pub name: String,
    pub rust: String,
    pub size: u32,
    pub signed: bool,
}

#[derive(Debug, Clone)]
pub struct Bitfield {
    pub rust_name: String,
    pub shared: bool,
    pub fields: Vec<BitField>,
}

#[derive(Debug, Clone)]
pub struct Mapper {
    pub rust_name: String,
    pub shared: bool,
    pub underlying: Prim,
    /// `(value, name)` sorted by value.
    pub mappings: Vec<(i64, String)>,
}

#[derive(Debug, Clone)]
pub enum Ty {
    Void,
    Prim(Prim),
    /// A standalone named container type (`types::Name`) with its own impls.
    Named(String),
    Container(Box<Container>),
    Switch(Box<Switch>),
    Array {
        count: Count,
        elem: Box<Ty>,
        path: String,
    },
    Option(Box<Ty>),
    Bitfield(Box<Bitfield>),
    Mapper(Box<Mapper>),
    /// Entries until a byte equal to `end` (entity metadata).
    MetaLoop {
        end: u8,
        elem: Box<Ty>,
        path: String,
    },
    /// Entries whose first byte has the top bit set while more entries follow.
    TopBitArray {
        elem: Box<Ty>,
        path: String,
    },
}

impl Ty {
    /// Structural equality, ignoring generated names.
    pub fn same_shape(&self, other: &Ty) -> bool {
        match (self, other) {
            (Ty::Void, Ty::Void) => true,
            (Ty::Prim(a), Ty::Prim(b)) => a == b,
            (Ty::Named(a), Ty::Named(b)) => a == b,
            (Ty::Container(a), Ty::Container(b)) => {
                a.fields.len() == b.fields.len()
                    && a.fields.iter().zip(&b.fields).all(|(x, y)| {
                        x.name == y.name && x.anon == y.anon && x.ty.same_shape(&y.ty)
                    })
            }
            (
                Ty::Array {
                    count: c1,
                    elem: e1,
                    ..
                },
                Ty::Array {
                    count: c2,
                    elem: e2,
                    ..
                },
            ) => c1 == c2 && e1.same_shape(e2),
            (Ty::Option(a), Ty::Option(b)) => a.same_shape(b),
            (Ty::Bitfield(a), Ty::Bitfield(b)) => {
                a.fields.len() == b.fields.len()
                    && a.fields
                        .iter()
                        .zip(&b.fields)
                        .all(|(x, y)| x.name == y.name && x.size == y.size && x.signed == y.signed)
            }
            (Ty::Mapper(a), Ty::Mapper(b)) => {
                a.underlying == b.underlying && a.mappings == b.mappings
            }
            (Ty::Switch(a), Ty::Switch(b)) => {
                a.compare_to == b.compare_to
                    && a.presence == b.presence
                    && a.default.same_shape(&b.default)
                    && a.cases.len() == b.cases.len()
                    && a.cases
                        .iter()
                        .zip(&b.cases)
                        .all(|((k1, t1), (k2, t2))| k1 == k2 && t1.same_shape(t2))
            }
            (
                Ty::MetaLoop {
                    end: a, elem: e1, ..
                },
                Ty::MetaLoop {
                    end: b, elem: e2, ..
                },
            ) => a == b && e1.same_shape(e2),
            (Ty::TopBitArray { elem: e1, .. }, Ty::TopBitArray { elem: e2, .. }) => {
                e1.same_shape(e2)
            }
            _ => false,
        }
    }
}

/// Classification of a switch for code generation.
pub enum SwitchKind<'a> {
    /// Every branch is void: the field carries nothing.
    AlwaysVoid,
    /// Every branch has the same non-void type: an unconditional `T`.
    Always(&'a Ty),
    /// Void or one type `T`: `Option<T>`.
    Optional(&'a Ty),
    /// Several shapes: an enum.
    Enum(Vec<Variant<'a>>),
}

/// One enum variant: the case keys that select it (or the default branch).
pub struct Variant<'a> {
    pub name: String,
    pub keys: Vec<&'a str>,
    pub is_default: bool,
    pub ty: &'a Ty,
}

impl Switch {
    pub fn classify(&self, variant_names: &BTreeMap<String, String>) -> SwitchKind<'_> {
        if self.presence.is_some() {
            let inner = self
                .cases
                .iter()
                .map(|(_, t)| t)
                .chain([&self.default])
                .find(|t| !matches!(t, Ty::Void))
                .unwrap_or_else(|| {
                    panic!("{}: presence override on an all-void switch", self.path)
                });
            return SwitchKind::Optional(inner);
        }
        let branches: Vec<&Ty> = self
            .cases
            .iter()
            .map(|(_, t)| t)
            .chain([&self.default])
            .collect();
        let non_void: Vec<&Ty> = branches
            .iter()
            .copied()
            .filter(|t| !matches!(t, Ty::Void))
            .collect();
        if non_void.is_empty() {
            return SwitchKind::AlwaysVoid;
        }
        let first = non_void[0];
        if non_void.iter().all(|t| t.same_shape(first)) {
            return if non_void.len() == branches.len() {
                SwitchKind::Always(first)
            } else {
                SwitchKind::Optional(first)
            };
        }
        // One variant per case key (in protocol.json order) plus the default branch.
        let mut variants: Vec<Variant<'_>> = self
            .cases
            .iter()
            .map(|(k, t)| Variant {
                name: String::new(),
                keys: vec![k.as_str()],
                is_default: false,
                ty: t,
            })
            .collect();
        variants.push(Variant {
            name: String::new(),
            keys: vec![],
            is_default: true,
            ty: &self.default,
        });
        let mut used = Vec::<String>::new();
        for v in &mut variants {
            let mut name = if let Some(k) = v.keys.first() {
                let key = format!("{}.{}", self.rust_name, k);
                variant_names
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| variant_name(k))
            } else if matches!(v.ty, Ty::Void) {
                "Other".to_string()
            } else {
                "Default".to_string()
            };
            while used.contains(&name) {
                name.push('_');
            }
            used.push(name.clone());
            v.name = name;
        }
        SwitchKind::Enum(variants)
    }
}

/// Enum variant name for a case key.
pub fn variant_name(key: &str) -> String {
    let k = key.strip_prefix("minecraft:").unwrap_or(key);
    if let Some(rest) = k.strip_prefix('-')
        && rest.chars().all(|c| c.is_ascii_digit())
    {
        return format!("CaseNeg{rest}");
    }
    if k.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return format!(
            "Case{}",
            k.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
        );
    }
    pascal(k)
}

/// Per-version resolver configuration.
pub struct Resolver<'a> {
    /// The global `types` section.
    pub types: &'a serde_json::Map<String, Value>,
    /// Named container types that get standalone Rust types (protodef name -> Rust name).
    pub named: BTreeMap<String, String>,
    /// Overrides of Rust names for named (inlined) types.
    pub renames: BTreeMap<String, String>,
    /// Overrides of enum variant names: `"EnumName.caseKey"` -> variant name.
    pub variant_names: BTreeMap<String, String>,
    /// Presence overrides by switch path (`state.direction.packet.field`).
    pub presence: BTreeMap<String, PresenceOverride>,
    /// Presence overrides that matched a switch (see [`Resolver::assert_presence_used`]).
    presence_used: BTreeSet<String>,
    next_id: usize,
}

/// Where a nominal type is being defined.
#[derive(Clone)]
struct Ctx {
    /// Rust name prefix for nominal types found here.
    prefix: String,
    /// `packet.field` path for errors.
    path: String,
    /// `$compareTo` substitution for parametrised types.
    compare_to: Option<String>,
    /// Nominal types found here belong to the `types` module.
    shared: bool,
}

impl<'a> Resolver<'a> {
    pub fn new(types: &'a serde_json::Map<String, Value>) -> Self {
        let mut named = BTreeMap::new();
        for (name, def) in types {
            if def
                .as_array()
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                == Some("container")
                && !matches!(name.as_str(), "slot" | "position")
                && !contains_compare_to_param(def)
            {
                named.insert(name.clone(), pascal(name));
            }
        }
        Resolver {
            types,
            named,
            renames: BTreeMap::new(),
            variant_names: BTreeMap::new(),
            presence: BTreeMap::new(),
            presence_used: BTreeSet::new(),
            next_id: 1,
        }
    }

    /// Panics if a presence override matched no switch (minecraft-data changed or the
    /// path is misspelt), so a stale override cannot silently stop applying.
    pub fn assert_presence_used(&self) {
        for path in self.presence.keys() {
            assert!(
                self.presence_used.contains(path),
                "presence override `{path}` matched no switch"
            );
        }
    }

    fn id(&mut self) -> usize {
        self.next_id += 1;
        self.next_id
    }

    /// Resolves a packet (or standalone named container) definition.
    pub fn resolve_root(
        &mut self,
        rust_name: &str,
        path: &str,
        def: &Value,
        shared: bool,
    ) -> Container {
        let ctx = Ctx {
            prefix: rust_name.to_string(),
            path: path.to_string(),
            compare_to: None,
            shared,
        };
        match self.resolve(def, &ctx) {
            Ty::Container(c) => {
                let mut c = *c;
                c.rust_name = rust_name.to_string();
                c
            }
            Ty::Void => Container {
                rust_name: rust_name.to_string(),
                shared: ctx.shared,
                fields: vec![],
                id: self.id(),
            },
            other => panic!("{path}: root is not a container: {other:?}"),
        }
    }

    fn resolve(&mut self, t: &Value, ctx: &Ctx) -> Ty {
        match t {
            Value::String(s) => self.resolve_name(s, None, ctx),
            Value::Array(a) => {
                let kind = a[0].as_str().expect("type constructor name");
                let arg = &a[1];
                self.resolve_ctor(kind, arg, ctx)
            }
            other => panic!("{}: unexpected type {other}", ctx.path),
        }
    }

    fn resolve_name(&mut self, name: &str, args: Option<&Value>, ctx: &Ctx) -> Ty {
        let prim = match name {
            "varint" | "optvarint" => Some(Prim::VarInt),
            "varlong" => Some(Prim::VarLong),
            "u8" => Some(Prim::U8),
            "i8" => Some(Prim::I8),
            "u16" => Some(Prim::U16),
            "i16" => Some(Prim::I16),
            "i32" => Some(Prim::I32),
            "i64" => Some(Prim::I64),
            "f32" => Some(Prim::F32),
            "f64" => Some(Prim::F64),
            "bool" => Some(Prim::Bool),
            "UUID" => Some(Prim::Uuid),
            "string" => Some(Prim::String),
            "ByteArray" => Some(Prim::ByteArray),
            "restBuffer" => Some(Prim::RestBuffer),
            "nbt" => Some(Prim::Nbt),
            "optionalNbt" => Some(Prim::OptionalNbt),
            "position" => Some(Prim::Position),
            "slot" => Some(Prim::Slot),
            _ => None,
        };
        if let Some(p) = prim {
            return Ty::Prim(p);
        }
        if name == "void" {
            return Ty::Void;
        }
        if let Some(rust) = self.named.get(name) {
            return Ty::Named(rust.clone());
        }
        let def = self
            .types
            .get(name)
            .unwrap_or_else(|| panic!("{}: unknown type {name}", ctx.path))
            .clone();
        if def.as_str() == Some("native") {
            panic!("{}: unsupported native type {name}", ctx.path);
        }
        // Inline a named (non-container or parametrised) type: nominal types inside get
        // names rooted at the type's own name and live in the `types` module.
        let rust = self
            .renames
            .get(name)
            .cloned()
            .unwrap_or_else(|| pascal(name));
        let compare_to = args
            .and_then(|a| a.get("compareTo"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let inner = Ctx {
            prefix: rust,
            path: ctx.path.clone(),
            compare_to: compare_to.or_else(|| ctx.compare_to.clone()),
            shared: true,
        };
        self.resolve(&def, &inner)
    }

    fn field_ctx(ctx: &Ctx, field: &str) -> Ctx {
        Ctx {
            prefix: format!("{}{}", ctx.prefix, pascal(field)),
            path: format!("{}.{}", ctx.path, field),
            compare_to: ctx.compare_to.clone(),
            shared: ctx.shared,
        }
    }

    fn resolve_ctor(&mut self, kind: &str, arg: &Value, ctx: &Ctx) -> Ty {
        match kind {
            "container" => {
                let mut fields = Vec::new();
                for f in arg.as_array().expect("container fields") {
                    let anon = f.get("anon").and_then(Value::as_bool).unwrap_or(false);
                    let name = f
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let fctx = if anon {
                        Self::field_ctx(ctx, "body")
                    } else {
                        Self::field_ctx(ctx, &name)
                    };
                    let ty = self.resolve(&f["type"], &fctx);
                    let rust = if anon {
                        "body".to_string()
                    } else {
                        snake(&name)
                    };
                    fields.push(Field {
                        name,
                        rust,
                        anon,
                        ty,
                    });
                }
                Ty::Container(Box::new(Container {
                    rust_name: ctx.prefix.clone(),
                    shared: ctx.shared,
                    fields,
                    id: self.id(),
                }))
            }
            "switch" => {
                let mut compare_to = arg["compareTo"].as_str().expect("compareTo").to_string();
                if compare_to == "$compareTo" {
                    compare_to = ctx
                        .compare_to
                        .clone()
                        .unwrap_or_else(|| panic!("{}: unbound $compareTo", ctx.path));
                }
                let mut cases = Vec::new();
                if let Some(fields) = arg.get("fields").and_then(Value::as_object) {
                    for (k, v) in fields {
                        let vctx = Ctx {
                            prefix: format!(
                                "{}{}",
                                ctx.prefix,
                                self.variant_names
                                    .get(&format!("{}.{}", ctx.prefix, k))
                                    .cloned()
                                    .unwrap_or_else(|| variant_name(k))
                            ),
                            path: ctx.path.clone(),
                            compare_to: ctx.compare_to.clone(),
                            shared: ctx.shared,
                        };
                        cases.push((k.clone(), self.resolve(v, &vctx)));
                    }
                }
                let default = match arg.get("default") {
                    Some(d) => {
                        let dctx = Ctx {
                            prefix: format!("{}Default", ctx.prefix),
                            path: ctx.path.clone(),
                            compare_to: ctx.compare_to.clone(),
                            shared: ctx.shared,
                        };
                        self.resolve(d, &dctx)
                    }
                    None => Ty::Void,
                };
                let mut sw = Switch {
                    compare_to,
                    cases,
                    default,
                    rust_name: ctx.prefix.clone(),
                    shared: ctx.shared,
                    path: ctx.path.clone(),
                    presence: None,
                };
                if let Some(o) = self.presence.get(&ctx.path) {
                    // Only option-like switches (void or one type) can take a presence
                    // rule; anything else means minecraft-data changed under the override.
                    assert!(
                        matches!(sw.classify(&self.variant_names), SwitchKind::Optional(_)),
                        "{}: presence override on a switch that is not option-like",
                        ctx.path
                    );
                    sw.presence = Some(o.clone());
                    self.presence_used.insert(ctx.path.clone());
                }
                Ty::Switch(Box::new(sw))
            }
            "array" => {
                let count = if let Some(ct) = arg.get("countType") {
                    match self.resolve(ct, ctx) {
                        Ty::Prim(p) if p.is_int() => Count::Prefix(p),
                        other => panic!("{}: bad countType {other:?}", ctx.path),
                    }
                } else if let Some(c) = arg.get("count") {
                    match c {
                        Value::Number(n) => Count::Fixed(n.as_u64().expect("count") as usize),
                        Value::String(s) => Count::Field(s.clone()),
                        _ => panic!("{}: bad count", ctx.path),
                    }
                } else {
                    panic!("{}: array without count", ctx.path)
                };
                let elem = self.resolve(&arg["type"], ctx);
                Ty::Array {
                    count,
                    elem: Box::new(elem),
                    path: ctx.path.clone(),
                }
            }
            "option" => Ty::Option(Box::new(self.resolve(arg, ctx))),
            "bitfield" => {
                let fields = arg
                    .as_array()
                    .expect("bitfield")
                    .iter()
                    .map(|b| BitField {
                        name: b["name"].as_str().unwrap().to_string(),
                        rust: snake(b["name"].as_str().unwrap()),
                        size: b["size"].as_u64().unwrap() as u32,
                        signed: b["signed"].as_bool().unwrap_or(false),
                    })
                    .collect::<Vec<_>>();
                let total: u32 = fields.iter().map(|f| f.size).sum();
                assert!(
                    total.is_multiple_of(8) && total <= 64,
                    "{}: bitfield of {total} bits",
                    ctx.path
                );
                Ty::Bitfield(Box::new(Bitfield {
                    rust_name: ctx.prefix.clone(),
                    shared: ctx.shared,
                    fields,
                }))
            }
            "mapper" => {
                let underlying = match self.resolve(&arg["type"], ctx) {
                    Ty::Prim(p) if p.is_int() => p,
                    other => panic!("{}: bad mapper type {other:?}", ctx.path),
                };
                let mut mappings: Vec<(i64, String)> = arg["mappings"]
                    .as_object()
                    .expect("mappings")
                    .iter()
                    .map(|(k, v)| {
                        (
                            parse_int(k).expect("mapper key"),
                            v.as_str().unwrap().to_string(),
                        )
                    })
                    .collect();
                mappings.sort();
                Ty::Mapper(Box::new(Mapper {
                    rust_name: ctx.prefix.clone(),
                    shared: ctx.shared,
                    underlying,
                    mappings,
                }))
            }
            "entityMetadataLoop" => {
                let end = arg["endVal"].as_u64().expect("endVal") as u8;
                let elem = self.resolve(
                    &arg["type"],
                    &Ctx {
                        prefix: format!("{}Entry", ctx.prefix),
                        ..ctx.clone()
                    },
                );
                Ty::MetaLoop {
                    end,
                    elem: Box::new(elem),
                    path: ctx.path.clone(),
                }
            }
            "topBitSetTerminatedArray" => {
                let elem = self.resolve(&arg["type"], ctx);
                Ty::TopBitArray {
                    elem: Box::new(elem),
                    path: ctx.path.clone(),
                }
            }
            "buffer" => match arg.get("countType").and_then(Value::as_str) {
                Some("varint") => Ty::Prim(Prim::ByteArray),
                _ => panic!("{}: unsupported buffer {arg}", ctx.path),
            },
            "pstring" => match arg.get("countType").and_then(Value::as_str) {
                Some("varint") => Ty::Prim(Prim::String),
                _ => panic!("{}: unsupported pstring {arg}", ctx.path),
            },
            other => {
                // A parametrised reference to a named type, e.g. ["particleData", {...}].
                if self.types.contains_key(other) {
                    self.resolve_name(other, Some(arg), ctx)
                } else {
                    panic!("{}: unsupported type constructor {other}", ctx.path)
                }
            }
        }
    }
}

fn contains_compare_to_param(v: &Value) -> bool {
    match v {
        Value::String(s) => s == "$compareTo",
        Value::Array(a) => a.iter().any(contains_compare_to_param),
        Value::Object(o) => o.values().any(contains_compare_to_param),
        _ => false,
    }
}

/// Parses a decimal or `0x` hex integer key.
pub fn parse_int(s: &str) -> Option<i64> {
    if let Some(h) = s.strip_prefix("0x") {
        i64::from_str_radix(h, 16).ok()
    } else {
        s.parse().ok()
    }
}
