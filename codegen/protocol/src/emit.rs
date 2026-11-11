//! Rust code emission from the IR.
//!
//! For every root (packet or standalone named container) three bodies are emitted:
//! `Decode::decode`, `Encode::encode` and (behind the `testgen` feature)
//! `Generate::generate`. Nested parts are emitted inline, so a switch can refer to any
//! enclosing field (`../action`) through the scope chain without passing context around.
//! Nominal types (structs, enums, mapper newtypes) are collected as definitions.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

use crate::ir::{
    BitField, Bitfield, Bitflags, Container, Count, Mapper, Prim, Switch, SwitchKind, Ty, Variant,
    parse_int,
};
use crate::names::screaming;

/// Generator hint for a field.
#[derive(Debug, Clone, Default)]
pub struct Hint {
    /// Case keys the field is compared against.
    pub values: Vec<String>,
    /// The field is used as an array count.
    pub count: bool,
}

#[derive(Clone)]
enum Kind<'t> {
    Int(Prim),
    Bits {
        size: u32,
        signed: bool,
    },
    Bool,
    Str,
    Mapper(&'t Mapper),
    BitStruct(&'t Bitfield),
    Flags(&'t Bitflags),
    /// An integer under `n` levels of `Option`.
    OptInt(usize, Prim),
    Other,
}

#[derive(Clone)]
struct Entry<'t> {
    name: String,
    expr: String,
    kind: Kind<'t>,
}

#[derive(Clone)]
struct Level<'t> {
    id: usize,
    entries: Vec<Entry<'t>>,
}

type Scope<'t> = Vec<Level<'t>>;

/// Emission mode of the shared walker.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Decode,
    Generate,
    /// Structural skip: reads exactly what `Decode` reads, keeps only the values later
    /// fields depend on (switch discriminants, counts) and builds nothing.
    Skip,
}

/// Named types that get an inherent `skip` (see [`Mode::Skip`]); every other named type a skip
/// reaches is inlined.
pub const SKIP_ROOTS: &[&str] = &["SlotComponent", "Slot"];

/// Marker written before every top-level item; replaced by the item's
/// `#[cfg(any(feature = ...))]` once it is known which versions share the item.
pub const CFG: &str = "#[cfg(⟦cfg⟧)]\n";

/// One emitted definition (nested struct/enum/bitfield/mapper or a root), before
/// placeholder references (`⟪scope|Name⟫`) are resolved to paths.
#[derive(Debug, Clone)]
pub struct Item {
    /// `"types"` (version-wide) or `"<state>/<direction>"`.
    pub scope: String,
    /// Rust name.
    pub name: String,
    /// Code with placeholder references and [`CFG`] markers.
    pub text: String,
}

pub struct Emitter<'t> {
    /// Bit layout of positions (`Xyz` / `Xzy`).
    pub layout: &'static str,
    /// Concrete path of the version's slot type, if it has a native `slot`.
    pub slot: Option<&'static str>,
    /// Scope of the packet module being emitted (`"play/clientbound"`).
    pub scope: String,
    /// Variant name overrides (`Enum.key` -> name).
    pub variant_names: BTreeMap<String, String>,
    /// Named container roots by Rust name (for minimum sizes).
    pub named: HashMap<String, &'t Container>,
    /// Nested definitions in emission order.
    pub items: Vec<Item>,
    shared_names: HashMap<String, &'t Ty>,
    local_names: HashMap<String, &'t Ty>,
    hints: HashMap<(usize, String), Hint>,
    tmp: usize,
    /// Root being emitted (for error messages).
    cur_root: String,
    /// Named types being inlined by a skip (cycle guard).
    skip_stack: Vec<String>,
}

fn int_type(size: u32, signed: bool) -> &'static str {
    match (size, signed) {
        (0..=8, false) => "u8",
        (0..=8, true) => "i8",
        (9..=16, false) => "u16",
        (9..=16, true) => "i16",
        (17..=32, false) => "u32",
        (17..=32, true) => "i32",
        (_, false) => "u64",
        (_, true) => "i64",
    }
}

fn bits_range(size: u32, signed: bool) -> (i128, i128) {
    if signed {
        (-(1i128 << (size - 1)), (1i128 << (size - 1)) - 1)
    } else {
        (0, (1i128 << size) - 1)
    }
}

impl<'t> Emitter<'t> {
    pub fn new(
        layout: &'static str,
        slot: Option<&'static str>,
        variant_names: BTreeMap<String, String>,
    ) -> Self {
        Emitter {
            layout,
            slot,
            scope: String::new(),
            variant_names,
            named: HashMap::new(),
            items: Vec::new(),
            shared_names: HashMap::new(),
            local_names: HashMap::new(),
            hints: HashMap::new(),
            tmp: 0,
            cur_root: String::new(),
            skip_stack: Vec::new(),
        }
    }

    fn var(&mut self) -> String {
        self.tmp += 1;
        format!("v{}", self.tmp)
    }

    /// Starts a new packet module scope (`"play/clientbound"`).
    pub fn begin_scope(&mut self, scope: &str) {
        self.scope = scope.to_string();
        self.local_names.clear();
    }

    /// Placeholder reference to a nominal type, resolved to a path later.
    fn tref(&self, name: &str, shared: bool) -> String {
        let scope = if shared { "types" } else { self.scope.as_str() };
        format!("⟪{scope}|{name}⟫")
    }

    fn slot_path(&self) -> &'static str {
        self.slot
            .unwrap_or_else(|| panic!("native `slot` used by a version without a slot type"))
    }

    // ------------------------------------------------------------------ hints

    /// Collects generator hints (switch case keys, count fields) for a root.
    pub fn collect_hints(&mut self, root: &'t Container) {
        let mut stack = vec![root.id];
        self.hint_fields(&root.fields, &mut stack);
    }

    fn hint_fields(&mut self, fields: &'t [crate::ir::Field], stack: &mut Vec<usize>) {
        for f in fields {
            match &f.ty {
                Ty::Container(c) if f.anon => self.hint_fields(&c.fields, stack),
                t => self.hint_ty(t, stack),
            }
        }
    }

    fn hint_key(path: &str, stack: &[usize]) -> (usize, String) {
        let mut lvl = stack.len() - 1;
        let mut rest = Vec::new();
        for seg in path.split('/') {
            if seg == ".." {
                lvl -= 1;
            } else {
                rest.push(seg);
            }
        }
        (stack[lvl], rest.join("/"))
    }

    fn hint_ty(&mut self, t: &'t Ty, stack: &mut Vec<usize>) {
        match t {
            Ty::Container(c) => {
                stack.push(c.id);
                self.hint_fields(&c.fields, stack);
                stack.pop();
            }
            Ty::Switch(s) => {
                let key = Self::hint_key(&s.compare_to, stack);
                let h = self.hints.entry(key).or_default();
                for (k, _) in &s.cases {
                    h.values.push(k.clone());
                }
                if let Some(o) = &s.presence {
                    for v in &o.hints {
                        if !h.values.contains(v) {
                            h.values.push(v.clone());
                        }
                    }
                }
                for (_, c) in &s.cases {
                    self.hint_ty(c, stack);
                }
                self.hint_ty(&s.default, stack);
            }
            Ty::Array { count, elem, .. } => {
                if let Count::Field(f) = count {
                    let key = Self::hint_key(f, stack);
                    self.hints.entry(key).or_default().count = true;
                }
                self.hint_ty(elem, stack);
            }
            Ty::Option(e) | Ty::Holder(e) | Ty::Boxed(e) => self.hint_ty(e, stack),
            Ty::MetaLoop { elem, .. } | Ty::TopBitArray { elem, .. } => self.hint_ty(elem, stack),
            _ => {}
        }
    }

    // ------------------------------------------------------------------ types

    /// Minimum encoded size in bytes.
    fn min_size(&self, t: &Ty) -> usize {
        match t {
            Ty::Void => 0,
            Ty::Prim(p) => match p {
                Prim::VarInt | Prim::VarLong | Prim::U8 | Prim::I8 | Prim::Bool => 1,
                Prim::U16 | Prim::I16 => 2,
                Prim::I32 | Prim::U32 | Prim::F32 => 4,
                Prim::I64 | Prim::U64 | Prim::F64 | Prim::Position => 8,
                Prim::Uuid => 16,
                Prim::String
                | Prim::ByteArray
                | Prim::Nbt
                | Prim::OptionalNbt
                | Prim::AnonNbt
                | Prim::AnonOptionalNbt
                | Prim::LpVec3
                | Prim::Slot => 1,
                Prim::RestBuffer => 0,
            },
            Ty::Named(n, _) => self.named.get(n).map_or(0, |c| self.fields_min(&c.fields)),
            Ty::Bitflags(b) => self.min_size(&Ty::Prim(b.underlying)),
            Ty::Holder(_) | Ty::HolderSet => 1,
            Ty::FixedBytes(n) => *n,
            // Only recursive references are boxed; they sit under an option or a switch.
            Ty::Boxed(_) => 0,
            Ty::Container(c) => self.fields_min(&c.fields),
            Ty::Switch(s) => s
                .cases
                .iter()
                .map(|(_, t)| self.min_size(t))
                .chain([self.min_size(&s.default)])
                .min()
                .unwrap_or(0),
            Ty::Array { count, elem, .. } => match count {
                Count::Prefix(p) => self.min_size(&Ty::Prim(*p)),
                Count::Fixed(n) => n * self.min_size(elem),
                Count::Field(_) => 0,
            },
            Ty::Option(_) => 1,
            Ty::Bitfield(b) => (b.fields.iter().map(|f| f.size).sum::<u32>() / 8) as usize,
            Ty::Mapper(m) => self.min_size(&Ty::Prim(m.underlying)),
            Ty::MetaLoop { .. } | Ty::TopBitArray { .. } => 1,
        }
    }

    fn fields_min(&self, fields: &[crate::ir::Field]) -> usize {
        fields.iter().map(|f| self.min_size(&f.ty)).sum()
    }

    fn prim_type(&self, p: Prim) -> String {
        match p {
            Prim::VarInt | Prim::I32 => "i32",
            Prim::VarLong | Prim::I64 => "i64",
            Prim::U8 => "u8",
            Prim::I8 => "i8",
            Prim::U16 => "u16",
            Prim::I16 => "i16",
            Prim::U32 => "u32",
            Prim::U64 => "u64",
            Prim::F32 => "f32",
            Prim::F64 => "f64",
            Prim::Bool => "bool",
            Prim::Uuid => "Uuid",
            Prim::String => "ByteString",
            Prim::ByteArray | Prim::RestBuffer => "Bytes",
            Prim::Nbt => "RawNbt",
            Prim::OptionalNbt => "Option<RawNbt>",
            Prim::AnonNbt => "AnonNbt",
            Prim::AnonOptionalNbt => "Option<AnonNbt>",
            Prim::LpVec3 => "LpVec3",
            Prim::Position => "BlockPos",
            Prim::Slot => self.slot_path(),
        }
        .to_string()
    }

    /// Rust type of a (non-void) type; registers nominal definitions on the way.
    pub fn rtype(&mut self, t: &'t Ty) -> String {
        match t {
            Ty::Void => "()".into(),
            Ty::Prim(p) => self.prim_type(*p),
            Ty::Named(n, shared) => self.tref(n, *shared),
            Ty::Bitflags(b) => {
                self.define_bitflags(t, b);
                self.tref(&b.rust_name, b.shared)
            }
            Ty::Holder(e) => format!("Holder<{}>", self.rtype(e)),
            Ty::HolderSet => "HolderSet".into(),
            Ty::FixedBytes(_) => "Bytes".into(),
            Ty::Boxed(e) => format!("Box<{}>", self.rtype(e)),
            Ty::Container(c) => {
                self.define_container(t, c);
                self.tref(&c.rust_name, c.shared)
            }
            Ty::Switch(s) => match s.classify(&self.variant_names) {
                SwitchKind::AlwaysVoid => "()".into(),
                SwitchKind::Always(inner) => self.rtype(inner),
                SwitchKind::Optional(inner) => format!("Option<{}>", self.rtype(inner)),
                SwitchKind::Enum(vs) => {
                    self.define_enum(t, s, &vs);
                    self.tref(&s.rust_name, s.shared)
                }
            },
            Ty::Array { elem, .. } | Ty::MetaLoop { elem, .. } | Ty::TopBitArray { elem, .. } => {
                format!("Vec<{}>", self.rtype(elem))
            }
            Ty::Option(e) => format!("Option<{}>", self.rtype(e)),
            Ty::Bitfield(b) => {
                self.define_bitfield(t, b);
                self.tref(&b.rust_name, b.shared)
            }
            Ty::Mapper(m) => {
                self.define_mapper(t, m);
                self.tref(&m.rust_name, m.shared)
            }
        }
    }

    /// Returns false if already defined (with the same shape).
    fn claim(&mut self, name: &str, shared: bool, t: &'t Ty) -> bool {
        let map = if shared {
            &mut self.shared_names
        } else {
            &mut self.local_names
        };
        if let Some(prev) = map.get(name) {
            assert!(prev.same_shape(t), "two different types named {name}");
            return false;
        }
        map.insert(name.to_string(), t);
        true
    }

    fn push_def(&mut self, shared: bool, name: &str, code: String) {
        let scope = if shared {
            "types".to_string()
        } else {
            self.scope.clone()
        };
        self.items.push(Item {
            scope,
            name: name.to_string(),
            text: code,
        });
    }

    /// Struct fields of a container: `(rust name, type, doc)`, with anonymous containers
    /// and bit fields flattened.
    fn struct_fields(&mut self, c: &'t Container, out: &mut Vec<(String, String, String)>) {
        for f in &c.fields {
            match &f.ty {
                Ty::Container(inner) if f.anon => self.struct_fields(inner, out),
                Ty::Bitfield(b) if f.anon => {
                    for bf in &b.fields {
                        out.push((
                            bf.rust.clone(),
                            int_type(bf.size, bf.signed).into(),
                            format!("`{}` ({} bits)", bf.name, bf.size),
                        ));
                    }
                }
                t if is_void(t, &self.variant_names) => {}
                t => {
                    let ty = self.rtype(t);
                    let mut doc = if f.anon {
                        "Anonymous switch".to_string()
                    } else {
                        format!("`{}`", f.name)
                    };
                    if let Ty::Switch(s) = t
                        && let Some(o) = &s.presence
                    {
                        doc.push_str(&format!(" — {}", o.doc));
                    }
                    out.push((f.rust.clone(), ty, doc));
                }
            }
        }
    }

    fn define_container(&mut self, t: &'t Ty, c: &'t Container) {
        if !self.claim(&c.rust_name, c.shared, t) {
            return;
        }
        let mut fields = Vec::new();
        self.struct_fields(c, &mut fields);
        let code = struct_def(&c.rust_name, "Nested container.", &fields);
        self.push_def(c.shared, &c.rust_name, code);
    }

    /// Registers a root container struct (packet or named type) and returns its definition.
    /// `generics` is appended to the struct name (`"<const V: i32>"` for packets).
    pub fn root_struct(&mut self, c: &'t Container, doc: &str, generics: &str) -> String {
        let mut fields = Vec::new();
        self.struct_fields(c, &mut fields);
        let mut seen = HashSet::new();
        for (n, _, _) in &fields {
            assert!(
                seen.insert(n.clone()),
                "{}: duplicate field {n}",
                c.rust_name
            );
        }
        struct_def(&format!("{}{generics}", c.rust_name), doc, &fields)
    }

    fn define_enum(&mut self, t: &'t Ty, s: &'t Switch, vs: &[Variant<'t>]) {
        if !self.claim(&s.rust_name, s.shared, t) {
            return;
        }
        let mut code = String::from(CFG);
        let _ = writeln!(code, "/// Switch on `{}`.", s.compare_to);
        code.push_str("#[derive(Debug, Clone, PartialEq)]\npub enum ");
        code.push_str(&s.rust_name);
        code.push_str(" {\n");
        for v in vs {
            let keys = if v.is_default {
                if v.keys.is_empty() {
                    "Any other value.".to_string()
                } else {
                    format!("Case(s) {} and any other value.", quote_keys(&v.keys))
                }
            } else {
                format!("Case(s) {}.", quote_keys(&v.keys))
            };
            let _ = writeln!(code, "    /// {keys}");
            if is_void(v.ty, &self.variant_names) {
                let _ = writeln!(code, "    {},", v.name);
            } else {
                let ty = self.rtype(v.ty);
                let _ = writeln!(code, "    {}({ty}),", v.name);
            }
        }
        code.push_str("}\n");
        self.push_def(s.shared, &s.rust_name, code);
    }

    fn define_bitfield(&mut self, t: &'t Ty, b: &'t Bitfield) {
        if !self.claim(&b.rust_name, b.shared, t) {
            return;
        }
        let fields: Vec<(String, String, String)> = b
            .fields
            .iter()
            .map(|f| {
                (
                    f.rust.clone(),
                    int_type(f.size, f.signed).to_string(),
                    format!("`{}` ({} bits)", f.name, f.size),
                )
            })
            .collect();
        let mut code = struct_def(&b.rust_name, "Packed bit field.", &fields);
        code = code.replace(
            "#[derive(Debug, Clone, PartialEq)]",
            "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]",
        );
        self.push_def(b.shared, &b.rust_name, code);
    }

    fn define_mapper(&mut self, t: &'t Ty, m: &'t Mapper) {
        if !self.claim(&m.rust_name, m.shared, t) {
            return;
        }
        let ut = self.prim_type(m.underlying);
        let name = &m.rust_name;
        let mut code = String::from(CFG);
        let _ = writeln!(
            code,
            "/// Mapped `{ut}` value; unknown values are kept as-is."
        );
        code.push_str("#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]\n");
        let _ = writeln!(code, "pub struct {name}(pub {ut});");
        code.push_str(CFG);
        let _ = writeln!(code, "impl {name} {{");
        for (v, n) in &m.mappings {
            let _ = writeln!(
                code,
                "    /// `{n}`\n    pub const {}: Self = Self({v});",
                screaming(n)
            );
        }
        code.push_str("    /// minecraft-data name of the value, if known.\n");
        let _ = writeln!(
            code,
            "    pub const fn name(self) -> Option<&'static str> {{\n        match self.0 {{"
        );
        for (v, n) in &m.mappings {
            let _ = writeln!(code, "            {v} => Some(\"{n}\"),");
        }
        code.push_str("            _ => None,\n        }\n    }\n}\n");
        code.push_str(CFG);
        let _ = writeln!(
            code,
            "impl std::fmt::Debug for {name} {{\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        match self.name() {{\n            Some(n) => write!(f, \"{name}::{{n}}\"),\n            None => write!(f, \"{name}({{}})\", self.0),\n        }}\n    }}\n}}"
        );
        self.push_def(m.shared, &m.rust_name, code);
    }

    fn define_bitflags(&mut self, t: &'t Ty, b: &'t Bitflags) {
        if !self.claim(&b.rust_name, b.shared, t) {
            return;
        }
        let ut = self.prim_type(b.underlying);
        let name = &b.rust_name;
        let mut code = String::from(CFG);
        let _ = writeln!(
            code,
            "/// Bit flags in a `{ut}`; unknown bits are kept as-is."
        );
        code.push_str(
            "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]\n",
        );
        let _ = writeln!(code, "pub struct {name}(pub {ut});");
        code.push_str(CFG);
        let _ = writeln!(code, "impl {name} {{");
        for (i, f) in b.flags.iter().enumerate() {
            let _ = writeln!(
                code,
                "    /// `{f}`\n    pub const {}: Self = Self(1 << {i});",
                screaming(f)
            );
        }
        let _ = writeln!(
            code,
            "    /// Whether every bit of `f` is set.\n    #[inline]\n    pub const fn contains(self, f: Self) -> bool {{ self.0 & f.0 == f.0 }}\n\
             \x20   /// The union of both sets of bits.\n    #[inline]\n    #[must_use]\n    pub const fn with(self, f: Self) -> Self {{ Self(self.0 | f.0) }}\n}}"
        );
        self.push_def(b.shared, name, code);
    }

    // ------------------------------------------------------------------ scope

    fn lookup(&self, path: &str, sc: &Scope<'t>) -> Entry<'t> {
        let mut lvl = sc.len() - 1;
        let mut segs = Vec::new();
        for seg in path.split('/') {
            if seg == ".." {
                lvl = lvl
                    .checked_sub(1)
                    .unwrap_or_else(|| panic!("path {path} escapes the root"));
            } else {
                segs.push(seg);
            }
        }
        let first = segs[0];
        // A name not found at its level is looked up in the enclosing containers, like
        // node-protodef's generated code does (lexical scoping of container fields).
        let e = (0..=lvl)
            .rev()
            .find_map(|l| sc[l].entries.iter().rev().find(|e| e.name == first))
            .unwrap_or_else(|| {
                let names: Vec<Vec<&str>> = sc
                    .iter()
                    .map(|l| l.entries.iter().map(|e| e.name.as_str()).collect())
                    .collect();
                panic!(
                    "cannot resolve `{path}` in {}; scope {names:?}",
                    self.cur_root
                )
            })
            .clone();
        match segs.get(1) {
            None => e,
            Some(sub) => {
                if let Kind::Flags(b) = e.kind {
                    let i = b
                        .flags
                        .iter()
                        .position(|f| f == sub)
                        .unwrap_or_else(|| panic!("`{path}`: no such flag"));
                    return Entry {
                        name: path.to_string(),
                        expr: format!("(({}.0 >> {i}) & 1 != 0)", e.expr),
                        kind: Kind::Bool,
                    };
                }
                let Kind::BitStruct(b) = e.kind else {
                    panic!("`{path}`: not a bit field")
                };
                let bf = b
                    .fields
                    .iter()
                    .find(|f| f.name == *sub)
                    .unwrap_or_else(|| panic!("`{path}`: no sub field"));
                Entry {
                    name: path.to_string(),
                    expr: format!("{}.{}", e.expr, bf.rust),
                    kind: Kind::Bits {
                        size: bf.size,
                        signed: bf.signed,
                    },
                }
            }
        }
    }

    fn kind_of(&self, t: &'t Ty) -> Kind<'t> {
        let opt = |k: Kind<'t>| match k {
            Kind::Int(p) => Kind::OptInt(1, p),
            Kind::OptInt(n, p) => Kind::OptInt(n + 1, p),
            _ => Kind::Other,
        };
        match t {
            Ty::Option(e) => opt(self.kind_of(e)),
            Ty::Switch(s) => match s.classify(&self.variant_names) {
                SwitchKind::Optional(inner) => opt(self.kind_of(inner)),
                SwitchKind::Always(inner) => self.kind_of(inner),
                _ => Kind::Other,
            },
            Ty::Prim(Prim::Bool) => Kind::Bool,
            Ty::Prim(Prim::String) => Kind::Str,
            Ty::Prim(p) if p.is_int() => Kind::Int(*p),
            Ty::Mapper(m) => Kind::Mapper(m),
            Ty::Bitfield(b) => Kind::BitStruct(b),
            Ty::Bitflags(b) => Kind::Flags(b),
            _ => Kind::Other,
        }
    }

    /// Boolean expression: does the discriminant at `path` equal case `key`?
    fn cond(&self, path: &str, key: &str, sc: &Scope<'t>) -> String {
        let e = self.lookup(path, sc);
        let int_cmp = |range: (i128, i128)| match parse_int(key) {
            Some(v) if (v as i128) >= range.0 && (v as i128) <= range.1 => {
                format!("({} == {v})", e.expr)
            }
            _ => "false".to_string(),
        };
        match e.kind {
            Kind::Int(p) => int_cmp(p.int_range()),
            Kind::Bits { size, signed } => int_cmp(bits_range(size, signed)),
            Kind::Bool => match key {
                "true" | "1" => e.expr.clone(),
                "false" | "0" => format!("(!{})", e.expr),
                _ => "false".into(),
            },
            Kind::Str => format!("(&*{} == {key:?})", e.expr),
            Kind::Mapper(m) => {
                let v = m
                    .mappings
                    .iter()
                    .find(|(_, n)| n == key)
                    .map(|(v, _)| *v)
                    .or_else(|| parse_int(key));
                match v {
                    Some(v) => format!("({}.0 == {v})", e.expr),
                    None => "false".into(),
                }
            }
            Kind::OptInt(n, p) => {
                let (lo, hi) = p.int_range();
                match parse_int(key) {
                    Some(v) if (v as i128) >= lo && (v as i128) <= hi => {
                        format!("({} == {}{v}{})", e.expr, "Some(".repeat(n), ")".repeat(n))
                    }
                    _ => "false".to_string(),
                }
            }
            Kind::BitStruct(_) | Kind::Flags(_) | Kind::Other => {
                panic!("cannot compare `{path}` with {key:?}")
            }
        }
    }

    fn any_cond(&self, path: &str, keys: &[&str], sc: &Scope<'t>) -> String {
        let parts: Vec<String> = keys
            .iter()
            .map(|k| self.cond(path, k, sc))
            .filter(|c| c != "false")
            .collect();
        if parts.is_empty() {
            "false".into()
        } else {
            parts.join(" || ")
        }
    }

    /// Whether a switch's selected branch is non-void (for option-like switches).
    fn present_cond(&self, s: &'t Switch, sc: &Scope<'t>) -> String {
        if let Some(o) = &s.presence {
            let e = self.lookup(&s.compare_to, sc);
            return format!("({})", o.cond.replace("{d}", &e.expr));
        }
        if matches!(s.default, Ty::Void) {
            let keys: Vec<&str> = s
                .cases
                .iter()
                .filter(|(_, t)| !matches!(t, Ty::Void))
                .map(|(k, _)| k.as_str())
                .collect();
            self.any_cond(&s.compare_to, &keys, sc)
        } else {
            let keys: Vec<&str> = s
                .cases
                .iter()
                .filter(|(_, t)| matches!(t, Ty::Void))
                .map(|(k, _)| k.as_str())
                .collect();
            format!("!({})", self.any_cond(&s.compare_to, &keys, sc))
        }
    }

    // ------------------------------------------------------------------ decode / generate

    /// Expression producing a value of `t` (decoding from `r` or generating with `g`).
    fn value(
        &mut self,
        t: &'t Ty,
        sc: &mut Scope<'t>,
        mode: Mode,
        hint: Option<&Hint>,
        name: &str,
    ) -> String {
        let gen_ = mode == Mode::Generate;
        if mode == Mode::Skip
            && let Some(v) = self.skip_value(t, sc, name)
        {
            return v;
        }
        match t {
            Ty::Void => "()".into(),
            Ty::Prim(p) => {
                if gen_ {
                    self.gen_prim(*p, hint)
                } else {
                    self.dec_prim(*p)
                }
            }
            Ty::Named(n, shared) => {
                let tr = self.tref(n, *shared);
                if gen_ {
                    format!("<{tr} as Generate>::generate(g)")
                } else {
                    format!("<{tr} as Decode>::decode(r)?")
                }
            }
            Ty::Container(c) => {
                let tr = self.rtype(t);
                self.container_value(tr, c, sc, mode, None)
            }
            Ty::Switch(s) => self.switch_value(t, s, sc, mode),
            Ty::Bitflags(b) => {
                let tr = self.rtype(t);
                if gen_ {
                    match b.underlying {
                        Prim::U64 => format!("{tr}(g.u64())"),
                        p => {
                            let (lo, hi) = p.int_range();
                            format!("{tr}(g.int_in({lo}, {hi}, &[]) as {})", self.prim_type(p))
                        }
                    }
                } else {
                    format!("{tr}({})", self.dec_prim(b.underlying))
                }
            }
            Ty::Holder(e) => {
                let inner = self.value(e, sc, mode, None, name);
                if gen_ {
                    format!(
                        "if g.bool() {{ Holder::Id(match g.int_in(-2147483648, 2147483647, &[0, 1, 2]) as i32 {{ -1 => 0, v => v }}) }} else {{ Holder::Inline({inner}) }}"
                    )
                } else {
                    format!(
                        "{{ let id = r.read_varint()?; if id == 0 {{ Holder::Inline({inner}) }} else {{ Holder::Id(id.wrapping_sub(1)) }} }}"
                    )
                }
            }
            Ty::HolderSet => {
                if gen_ {
                    "<HolderSet as Generate>::generate(g)".into()
                } else {
                    "<HolderSet as Decode>::decode(r)?".into()
                }
            }
            Ty::FixedBytes(n) => {
                if gen_ {
                    format!("g.fixed_bytes({n})")
                } else {
                    format!("r.read_bytes({n})?")
                }
            }
            Ty::Boxed(e) => {
                let inner = self.value(e, sc, mode, hint, name);
                format!("Box::new({inner})")
            }
            Ty::Array { count, elem, .. } => {
                let n = self.var();
                let v = self.var();
                let count_expr = match count {
                    Count::Prefix(p) => {
                        if gen_ {
                            "g.count()".to_string()
                        } else {
                            let min = self.min_size(elem);
                            format!(
                                "{{ let c = {}; r.check_count(i64::from(c), {min})? }}",
                                self.dec_prim(*p)
                            )
                        }
                    }
                    Count::Fixed(k) => k.to_string(),
                    Count::Field(f) => {
                        let e = self.lookup(f, sc);
                        if gen_ {
                            format!("usize::try_from({}).unwrap_or(0)", e.expr)
                        } else {
                            let min = self.min_size(elem);
                            format!("r.check_count(i64::from({}), {min})?", e.expr)
                        }
                    }
                };
                let ev = self.value(elem, sc, mode, None, name);
                format!(
                    "{{ let {n} = {count_expr}; let mut {v} = Vec::with_capacity({n}.min(crate::codec::PREALLOC_LIMIT)); for _ in 0..{n} {{ {v}.push({ev}); }} {v} }}"
                )
            }
            Ty::Option(e) => {
                let inner = self.value(e, sc, mode, None, name);
                let c = if gen_ { "g.bool()" } else { "r.read_bool()?" };
                format!("if {c} {{ Some({inner}) }} else {{ None }}")
            }
            Ty::Bitfield(b) => {
                let bits = self.var();
                let read = if gen_ {
                    String::new()
                } else {
                    format!("let {bits} = {};", read_bits(b))
                };
                let mut fields = Vec::new();
                for (i, f) in b.fields.iter().enumerate() {
                    let h = hint_for(&self.hints, sc, &format!("{name}/{}", f.name));
                    let expr = if gen_ {
                        gen_bits(f, h.as_ref())
                    } else {
                        extract_bits(&bits, b, i)
                    };
                    fields.push(format!("{}: {expr}", f.rust));
                }
                let tr = self.tref(&b.rust_name, b.shared);
                self.rtype(t);
                format!("{{ {read} {tr} {{ {} }} }}", fields.join(", "))
            }
            Ty::Mapper(m) => {
                let tr = self.rtype(t);
                if gen_ {
                    let mut vals: Vec<i64> = m.mappings.iter().map(|(v, _)| *v).collect();
                    if let Some(h) = hint {
                        vals.extend(h.values.iter().filter_map(|k| parse_int(k)));
                    }
                    let (lo, hi) = m.underlying.int_range();
                    format!(
                        "{tr}(g.int_in({lo}, {hi}, &{vals:?}) as {})",
                        self.prim_type(m.underlying)
                    )
                } else {
                    format!("{tr}({})", self.dec_prim(m.underlying))
                }
            }
            Ty::MetaLoop { end, elem, .. } => {
                let v = self.var();
                if gen_ {
                    let e = self.var();
                    let ev = self.value(elem, sc, mode, None, name);
                    let enc = self.encode(elem, &e, sc);
                    format!(
                        "{{ let n = g.count(); let mut {v} = Vec::with_capacity(n); for _ in 0..n {{ {v}.push(loop {{ let {e} = {ev}; if crate::testgen::first_byte_ok(|w: &mut BytesMut| -> EncodeResult {{ {enc} Ok(()) }}, |b| b != {end}) {{ break {e}; }} }}); }} {v} }}"
                    )
                } else {
                    let ev = self.value(elem, sc, mode, None, name);
                    format!(
                        "{{ let mut {v} = Vec::new(); loop {{ if r.peek_u8()? == {end} {{ r.skip(1)?; break; }} {v}.push({ev}); }} {v} }}"
                    )
                }
            }
            Ty::TopBitArray { elem, .. } => {
                let Ty::Container(c) = &**elem else {
                    panic!("topBitSetTerminatedArray of non-container")
                };
                let v = self.var();
                if gen_ {
                    let e = self.var();
                    let ev = self.value(elem, sc, mode, None, name);
                    let enc = self.encode(elem, &e, sc);
                    format!(
                        "{{ let n = 1 + g.count(); let mut {v} = Vec::with_capacity(n); for _ in 0..n {{ {v}.push(loop {{ let {e} = {ev}; if crate::testgen::first_byte_ok(|w: &mut BytesMut| -> EncodeResult {{ {enc} Ok(()) }}, |b| b & 0x80 == 0) {{ break {e}; }} }}); }} {v} }}"
                    )
                } else {
                    let first_ty = match &c.fields.first().map(|f| &f.ty) {
                        Some(Ty::Prim(Prim::I8)) => "i8",
                        Some(Ty::Prim(Prim::U8)) => "u8",
                        _ => panic!("topBitSetTerminatedArray element must start with a byte"),
                    };
                    let b0 = self.var();
                    let first = format!("(({b0} & 0x7f) as {first_ty})");
                    let tr = self.rtype(elem);
                    let ev = self.container_value(tr, c, sc, mode, Some(first));
                    format!(
                        "{{ let mut {v} = Vec::new(); loop {{ let {b0} = r.read_u8()?; {v}.push({ev}); if {b0} & 0x80 == 0 {{ break; }} }} {v} }}"
                    )
                }
            }
        }
    }

    /// [`Mode::Skip`] forms that differ from decoding (`None`: decode the value, it is
    /// cheap and may be needed by a later field).
    fn skip_value(&mut self, t: &'t Ty, sc: &mut Scope<'t>, name: &str) -> Option<String> {
        Some(match t {
            Ty::Named(n, shared) => {
                let tr = self.tref(n, *shared);
                if SKIP_ROOTS.contains(&n.as_str()) {
                    format!("{{ {tr}::skip(r)?; }}")
                } else if self.skip_stack.contains(n) || !self.named.contains_key(n) {
                    format!("{{ let _ = <{tr} as Decode>::decode(r)?; }}")
                } else {
                    let c = self.named[n];
                    self.skip_stack.push(n.clone());
                    let mut inner = Vec::new();
                    let v = self.container_value(String::new(), c, &mut inner, Mode::Skip, None);
                    self.skip_stack.pop();
                    v
                }
            }
            Ty::Prim(Prim::Slot) => format!("{{ let _ = {}; }}", self.dec_prim(Prim::Slot)),
            Ty::Holder(e) => {
                let inner = self.value(e, sc, Mode::Skip, None, name);
                format!("{{ let id = r.read_varint()?; if id == 0 {{ let _ = {inner}; }} }}")
            }
            Ty::HolderSet => "{ HolderSet::skip(r)?; }".into(),
            Ty::FixedBytes(n) => format!("{{ r.skip({n})?; }}"),
            Ty::Array { count, elem, .. } => {
                let n = self.var();
                let count_expr = match count {
                    Count::Prefix(p) => {
                        let min = self.min_size(elem);
                        format!(
                            "{{ let c = {}; r.check_count(i64::from(c), {min})? }}",
                            self.dec_prim(*p)
                        )
                    }
                    Count::Fixed(k) => k.to_string(),
                    Count::Field(f) => {
                        let e = self.lookup(f, sc);
                        let min = self.min_size(elem);
                        format!("r.check_count(i64::from({}), {min})?", e.expr)
                    }
                };
                let ev = self.value(elem, sc, Mode::Skip, None, name);
                format!("{{ let {n} = {count_expr}; for _ in 0..{n} {{ let _ = {ev}; }} }}")
            }
            Ty::MetaLoop { end, elem, .. } => {
                let ev = self.value(elem, sc, Mode::Skip, None, name);
                format!(
                    "{{ loop {{ if r.peek_u8()? == {end} {{ r.skip(1)?; break; }} let _ = {ev}; }} }}"
                )
            }
            Ty::TopBitArray { elem, .. } => {
                let Ty::Container(c) = &**elem else {
                    panic!("topBitSetTerminatedArray of non-container")
                };
                let first_ty = match &c.fields.first().map(|f| &f.ty) {
                    Some(Ty::Prim(Prim::I8)) => "i8",
                    Some(Ty::Prim(Prim::U8)) => "u8",
                    _ => panic!("topBitSetTerminatedArray element must start with a byte"),
                };
                let b0 = self.var();
                let first = format!("(({b0} & 0x7f) as {first_ty})");
                let ev = self.container_value(String::new(), c, sc, Mode::Skip, Some(first));
                format!(
                    "{{ loop {{ let {b0} = r.read_u8()?; let _ = {ev}; if {b0} & 0x80 == 0 {{ break; }} }} }}"
                )
            }
            _ => return None,
        })
    }

    /// Statements skipping one value of a root container (see [`Mode::Skip`]).
    pub fn root_skip(&mut self, c: &'t Container) -> String {
        self.tmp = 0;
        self.cur_root = c.rust_name.clone();
        let mut sc = Vec::new();
        self.container_value(String::new(), c, &mut sc, Mode::Skip, None)
    }

    fn gen_prim(&self, p: Prim, hint: Option<&Hint>) -> String {
        if p.is_int() {
            let (lo, hi) = p.int_range();
            let ty = self.prim_type(p);
            if let Some(h) = hint {
                if h.count {
                    return format!("(g.small_count() as {ty})");
                }
                let vals: Vec<i64> = h
                    .values
                    .iter()
                    .filter_map(|k| parse_int(k))
                    .filter(|v| (*v as i128) >= lo && (*v as i128) <= hi)
                    .collect();
                if !vals.is_empty() {
                    return format!("(g.int_in({lo}, {hi}, &{vals:?}) as {ty})");
                }
            }
            return match p {
                Prim::VarLong => "g.varlong()".into(),
                Prim::I64 => "g.i64()".into(),
                Prim::U64 => "g.u64()".into(),
                _ => format!("(g.int_in({lo}, {hi}, &[]) as {ty})"),
            };
        }
        match p {
            Prim::F32 => "g.f32()".into(),
            Prim::F64 => "g.f64()".into(),
            Prim::Bool => "g.bool()".into(),
            Prim::Uuid => "g.uuid()".into(),
            Prim::String => match hint {
                Some(h) if !h.values.is_empty() => format!("g.string_in(&{:?})", h.values),
                _ => "g.string()".into(),
            },
            Prim::ByteArray | Prim::RestBuffer => "g.bytes()".into(),
            Prim::Nbt => "g.nbt()".into(),
            Prim::OptionalNbt => "g.opt_nbt()".into(),
            Prim::AnonNbt => "g.anon_nbt()".into(),
            Prim::AnonOptionalNbt => "g.anon_opt_nbt()".into(),
            Prim::LpVec3 => "<LpVec3 as Generate>::generate(g)".into(),
            Prim::Position => "g.position()".into(),
            Prim::Slot => format!("<{} as Generate>::generate(g)", self.slot_path()),
            _ => unreachable!(),
        }
    }

    /// Decodes/generates a container. `first` overrides the first field's value
    /// (top-bit-terminated arrays).
    fn container_value(
        &mut self,
        tr: String,
        c: &'t Container,
        sc: &mut Scope<'t>,
        mode: Mode,
        first: Option<String>,
    ) -> String {
        sc.push(Level {
            id: c.id,
            entries: Vec::new(),
        });
        let mut body = String::new();
        let mut inits = Vec::new();
        self.fields_value(c, sc, mode, &mut body, &mut inits, first);
        sc.pop();
        if mode == Mode::Skip {
            return format!("{{ {body} }}");
        }
        format!("{{ {body} {tr} {{ {} }} }}", inits.join(", "))
    }

    fn fields_value(
        &mut self,
        c: &'t Container,
        sc: &mut Scope<'t>,
        mode: Mode,
        body: &mut String,
        inits: &mut Vec<String>,
        mut first: Option<String>,
    ) {
        let gen_ = mode == Mode::Generate;
        for f in &c.fields {
            let lvl_id = sc.last().map(|l| l.id).unwrap_or(0);
            match &f.ty {
                Ty::Container(inner) if f.anon => {
                    self.fields_value(inner, sc, mode, body, inits, first.take());
                }
                Ty::Bitfield(b) if f.anon => {
                    let bits = self.var();
                    if !gen_ {
                        let _ = write!(body, "let {bits} = {};", read_bits(b));
                    }
                    for (i, bf) in b.fields.iter().enumerate() {
                        let v = self.var();
                        let expr = if gen_ {
                            let h = self.hints.get(&(lvl_id, bf.name.clone())).cloned();
                            gen_bits(bf, h.as_ref())
                        } else {
                            extract_bits(&bits, b, i)
                        };
                        let _ = write!(body, "let {v} = {expr};");
                        inits.push(format!("{}: {v}", bf.rust));
                        sc.last_mut().unwrap().entries.push(Entry {
                            name: bf.name.clone(),
                            expr: v,
                            kind: Kind::Bits {
                                size: bf.size,
                                signed: bf.signed,
                            },
                        });
                    }
                }
                t => {
                    let v = self.var();
                    let hint = self.hints.get(&(lvl_id, f.name.clone())).cloned();
                    let expr = match first.take() {
                        Some(e) => e,
                        None => self.value(t, sc, mode, hint.as_ref(), &f.name),
                    };
                    if is_void(t, &self.variant_names) {
                        // Void fields still consume nothing; keep evaluation order.
                        continue;
                    }
                    let _ = write!(body, "let {v} = {expr};");
                    inits.push(format!("{}: {v}", f.rust));
                    if !f.anon {
                        sc.last_mut().unwrap().entries.push(Entry {
                            name: f.name.clone(),
                            expr: v,
                            kind: self.kind_of(t),
                        });
                    }
                }
            }
        }
    }

    fn switch_value(&mut self, t: &'t Ty, s: &'t Switch, sc: &mut Scope<'t>, mode: Mode) -> String {
        match s.classify(&self.variant_names) {
            SwitchKind::AlwaysVoid => "()".into(),
            SwitchKind::Always(inner) => self.value(inner, sc, mode, None, ""),
            SwitchKind::Optional(inner) => {
                let c = self.present_cond(s, sc);
                let v = self.value(inner, sc, mode, None, "");
                format!("if {c} {{ Some({v}) }} else {{ None }}")
            }
            SwitchKind::Enum(vs) => {
                let tr = self.rtype(t);
                let mut out = String::new();
                let mut default = None;
                for v in &vs {
                    if v.is_default {
                        default = Some(v);
                        continue;
                    }
                    let c = self.any_cond(&s.compare_to, &v.keys, sc);
                    let val = self.variant_value(&tr, v, sc, mode);
                    if mode == Mode::Skip {
                        let _ = write!(out, "if {c} {{ let _ = {val}; }} else ");
                    } else {
                        let _ = write!(out, "if {c} {{ {val} }} else ");
                    }
                }
                let d = default.expect("default variant");
                let val = self.variant_value(&tr, d, sc, mode);
                if mode == Mode::Skip {
                    let _ = write!(out, "{{ let _ = {val}; }}");
                } else {
                    let _ = write!(out, "{{ {val} }}");
                }
                format!("({out})")
            }
        }
    }

    fn variant_value(
        &mut self,
        tr: &str,
        v: &Variant<'t>,
        sc: &mut Scope<'t>,
        mode: Mode,
    ) -> String {
        if mode == Mode::Skip {
            return if is_void(v.ty, &self.variant_names) {
                "()".into()
            } else {
                self.value(v.ty, sc, mode, None, "")
            };
        }
        if is_void(v.ty, &self.variant_names) {
            format!("{tr}::{}", v.name)
        } else {
            let inner = self.value(v.ty, sc, mode, None, "");
            format!("{tr}::{}({inner})", v.name)
        }
    }

    // ------------------------------------------------------------------ encode

    /// Statements encoding the value at place `p` into `w`.
    fn encode(&mut self, t: &'t Ty, p: &str, sc: &mut Scope<'t>) -> String {
        match t {
            Ty::Void => String::new(),
            Ty::Prim(pr) => enc_prim(*pr, p, self.layout),
            Ty::Named(..) | Ty::HolderSet => format!("{p}.encode(w)?;"),
            Ty::Bitflags(b) => enc_prim(b.underlying, &format!("{p}.0"), self.layout),
            Ty::FixedBytes(n) => format!(
                "if {p}.len() != {n} {{ return Err(EncodeError::CountMismatch {{ field: \"fixed buffer\", expected: {n}, actual: {p}.len() }}); }} w.put_slice(&{p});"
            ),
            Ty::Boxed(e) => self.encode(e, &format!("(*{p})"), sc),
            Ty::Holder(e) => {
                let x = self.var();
                let id = self.var();
                let inner = self.encode(e, &format!("(*{x})"), sc);
                format!(
                    "match &{p} {{ Holder::Id({id}) => {{ if *{id} == -1 {{ return Err(EncodeError::OutOfRange(\"holder id\")); }} w.put_varint({id}.wrapping_add(1)); }} Holder::Inline({x}) => {{ w.put_varint(0); {inner} }} }}"
                )
            }
            Ty::Container(c) => {
                sc.push(Level {
                    id: c.id,
                    entries: Vec::new(),
                });
                let mut out = String::new();
                self.fields_encode(c, p, sc, &mut out);
                sc.pop();
                out
            }
            Ty::Switch(s) => self.switch_encode(t, s, p, sc),
            Ty::Array { count, elem, path } => {
                let it = self.var();
                let mut out = String::new();
                match count {
                    Count::Prefix(pr) => {
                        let _ = write!(out, "{}", put_count(*pr, &format!("{p}.len()")));
                    }
                    Count::Fixed(k) => {
                        let _ = write!(
                            out,
                            "if {p}.len() != {k} {{ return Err(EncodeError::CountMismatch {{ field: {path:?}, expected: {k}, actual: {p}.len() }}); }}"
                        );
                    }
                    Count::Field(f) => {
                        let e = self.lookup(f, sc);
                        let _ = write!(
                            out,
                            "if usize::try_from({e}).ok() != Some({p}.len()) {{ return Err(EncodeError::CountMismatch {{ field: {path:?}, expected: usize::try_from({e}).unwrap_or(0), actual: {p}.len() }}); }}",
                            e = e.expr
                        );
                    }
                }
                let inner = self.encode(elem, &format!("(*{it})"), sc);
                let _ = write!(out, "for {it} in {p}.iter() {{ {inner} }}");
                out
            }
            Ty::Option(e) => {
                let x = self.var();
                let inner = self.encode(e, &format!("(*{x})"), sc);
                format!(
                    "match &{p} {{ Some({x}) => {{ w.put_u8(1); {inner} }} None => w.put_u8(0), }}"
                )
            }
            Ty::Bitfield(b) => {
                let parts: Vec<(String, &BitField)> = b
                    .fields
                    .iter()
                    .map(|f| (format!("{p}.{}", f.rust), f))
                    .collect();
                write_bits(b, &parts)
            }
            Ty::Mapper(m) => enc_prim(m.underlying, &format!("{p}.0"), self.layout),
            Ty::MetaLoop { end, elem, path } => {
                let it = self.var();
                let start = self.var();
                let inner = self.encode(elem, &format!("(*{it})"), sc);
                format!(
                    "for {it} in {p}.iter() {{ let {start} = w.len(); {inner} if w.get({start}) == Some(&{end}) {{ return Err(EncodeError::OutOfRange({path:?})); }} }} w.put_u8({end});"
                )
            }
            Ty::TopBitArray { elem, path } => {
                let it = self.var();
                let i = self.var();
                let start = self.var();
                let inner = self.encode(elem, &format!("(*{it})"), sc);
                format!(
                    "if {p}.is_empty() {{ return Err(EncodeError::CountMismatch {{ field: {path:?}, expected: 1, actual: 0 }}); }} for ({i}, {it}) in {p}.iter().enumerate() {{ let {start} = w.len(); {inner} match w.get_mut({start}) {{ Some(b) if *b & 0x80 == 0 => {{ if {i} + 1 < {p}.len() {{ *b |= 0x80; }} }} _ => return Err(EncodeError::OutOfRange({path:?})), }} }}"
                )
            }
        }
    }

    fn fields_encode(&mut self, c: &'t Container, p: &str, sc: &mut Scope<'t>, out: &mut String) {
        for f in &c.fields {
            match &f.ty {
                Ty::Container(inner) if f.anon => self.fields_encode(inner, p, sc, out),
                Ty::Bitfield(b) if f.anon => {
                    let parts: Vec<(String, &BitField)> = b
                        .fields
                        .iter()
                        .map(|bf| (format!("{p}.{}", bf.rust), bf))
                        .collect();
                    out.push_str(&write_bits(b, &parts));
                    for (expr, bf) in parts {
                        sc.last_mut().unwrap().entries.push(Entry {
                            name: bf.name.clone(),
                            expr,
                            kind: Kind::Bits {
                                size: bf.size,
                                signed: bf.signed,
                            },
                        });
                    }
                }
                t if is_void(t, &self.variant_names) => {}
                t => {
                    let place = format!("{p}.{}", f.rust);
                    let code = self.encode(t, &place, sc);
                    out.push_str(&code);
                    if !f.anon {
                        sc.last_mut().unwrap().entries.push(Entry {
                            name: f.name.clone(),
                            expr: place,
                            kind: self.kind_of(t),
                        });
                    }
                }
            }
        }
    }

    fn switch_encode(&mut self, t: &'t Ty, s: &'t Switch, p: &str, sc: &mut Scope<'t>) -> String {
        let path = &s.path;
        match s.classify(&self.variant_names) {
            SwitchKind::AlwaysVoid => String::new(),
            SwitchKind::Always(inner) => self.encode(inner, p, sc),
            SwitchKind::Optional(inner) => {
                let c = self.present_cond(s, sc);
                let x = self.var();
                let enc = self.encode(inner, &format!("(*{x})"), sc);
                format!(
                    "match (&{p}, {c}) {{ (Some({x}), true) => {{ {enc} }} (None, false) => {{}} _ => return Err(EncodeError::SwitchMismatch({path:?})), }}"
                )
            }
            SwitchKind::Enum(vs) => {
                let tr = self.rtype(t);
                let all_explicit: Vec<&str> = vs
                    .iter()
                    .filter(|v| !v.is_default)
                    .flat_map(|v| v.keys.iter().copied())
                    .collect();
                let mut arms = String::new();
                for v in &vs {
                    let ok = if v.is_default {
                        format!("!({})", self.any_cond(&s.compare_to, &all_explicit, sc))
                    } else {
                        self.any_cond(&s.compare_to, &v.keys, sc)
                    };
                    let check = format!(
                        "if !({ok}) {{ return Err(EncodeError::SwitchMismatch({path:?})); }}"
                    );
                    if is_void(v.ty, &self.variant_names) {
                        let _ = write!(arms, "{tr}::{} => {{ {check} }}", v.name);
                    } else {
                        let x = self.var();
                        let enc = self.encode(v.ty, &format!("(*{x})"), sc);
                        let _ = write!(arms, "{tr}::{}({x}) => {{ {check} {enc} }}", v.name);
                    }
                }
                format!("match &{p} {{ {arms} }}")
            }
        }
    }

    // ------------------------------------------------------------------ roots

    /// `Decode` body expression for a root container (constructs `Self`).
    pub fn root_decode(&mut self, c: &'t Container) -> String {
        self.tmp = 0;
        self.cur_root = c.rust_name.clone();
        let mut sc = Vec::new();
        self.container_value("Self".into(), c, &mut sc, Mode::Decode, None)
    }

    /// `Generate` body expression for a root container (constructs `Self`).
    pub fn root_generate(&mut self, c: &'t Container) -> String {
        self.tmp = 0;
        self.cur_root = c.rust_name.clone();
        let mut sc = Vec::new();
        self.container_value("Self".into(), c, &mut sc, Mode::Generate, None)
    }

    /// `Encode` body statements for a root container (value is `self`).
    pub fn root_encode(&mut self, c: &'t Container) -> String {
        self.tmp = 0;
        self.cur_root = c.rust_name.clone();
        let mut sc = vec![Level {
            id: c.id,
            entries: Vec::new(),
        }];
        let mut out = String::new();
        self.fields_encode(c, "(*self)", &mut sc, &mut out);
        out
    }
}

fn is_void(t: &Ty, variant_names: &BTreeMap<String, String>) -> bool {
    match t {
        Ty::Void => true,
        Ty::Switch(s) => matches!(s.classify(variant_names), SwitchKind::AlwaysVoid),
        _ => false,
    }
}

fn hint_for(hints: &HashMap<(usize, String), Hint>, sc: &Scope<'_>, key: &str) -> Option<Hint> {
    let id = sc.last().map(|l| l.id)?;
    hints.get(&(id, key.to_string())).cloned()
}

fn quote_keys(keys: &[&str]) -> String {
    keys.iter()
        .map(|k| format!("`{k}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn struct_def(name: &str, doc: &str, fields: &[(String, String, String)]) -> String {
    let mut code = String::from(CFG);
    for line in doc.lines() {
        let _ = writeln!(code, "/// {line}");
    }
    code.push_str("#[derive(Debug, Clone, PartialEq)]\n");
    let _ = writeln!(code, "pub struct {name} {{");
    for (n, t, d) in fields {
        let _ = writeln!(code, "    /// {d}\n    pub {n}: {t},");
    }
    code.push_str("}\n");
    code
}

impl Emitter<'_> {
    fn dec_prim(&self, p: Prim) -> String {
        dec_prim(p, self.layout, self.slot)
    }
}

fn dec_prim(p: Prim, layout: &str, slot: Option<&str>) -> String {
    match p {
        Prim::VarInt => "r.read_varint()?".into(),
        Prim::VarLong => "r.read_varlong()?".into(),
        Prim::U8 => "r.read_u8()?".into(),
        Prim::I8 => "r.read_i8()?".into(),
        Prim::U16 => "r.read_u16()?".into(),
        Prim::I16 => "r.read_i16()?".into(),
        Prim::U32 => "r.read_u32()?".into(),
        Prim::U64 => "r.read_u64()?".into(),
        Prim::I32 => "r.read_i32()?".into(),
        Prim::I64 => "r.read_i64()?".into(),
        Prim::F32 => "r.read_f32()?".into(),
        Prim::F64 => "r.read_f64()?".into(),
        Prim::Bool => "r.read_bool()?".into(),
        Prim::Uuid => "r.read_uuid()?".into(),
        Prim::String => "r.read_string()?".into(),
        Prim::ByteArray => "r.read_byte_array()?".into(),
        Prim::RestBuffer => "r.read_rest()".into(),
        Prim::Nbt => "RawNbt::read(r)?".into(),
        Prim::OptionalNbt => "RawNbt::read_optional(r)?".into(),
        Prim::AnonNbt => "AnonNbt::read(r)?".into(),
        Prim::AnonOptionalNbt => "AnonNbt::read_optional(r)?".into(),
        Prim::LpVec3 => "LpVec3::read(r)?".into(),
        Prim::Position => format!("BlockPos::read(r, PositionLayout::{layout})?"),
        Prim::Slot => format!("<{} as Decode>::decode(r)?", slot.expect("slot type")),
    }
}

fn enc_prim(p: Prim, v: &str, layout: &str) -> String {
    match p {
        Prim::VarInt => format!("w.put_varint({v});"),
        Prim::VarLong => format!("w.put_varlong({v});"),
        Prim::U8 => format!("w.put_u8({v});"),
        Prim::I8 => format!("w.put_i8({v});"),
        Prim::U16 => format!("w.put_u16({v});"),
        Prim::I16 => format!("w.put_i16({v});"),
        Prim::U32 => format!("w.put_u32({v});"),
        Prim::U64 => format!("w.put_u64({v});"),
        Prim::I32 => format!("w.put_i32({v});"),
        Prim::I64 => format!("w.put_i64({v});"),
        Prim::F32 => format!("w.put_f32({v});"),
        Prim::F64 => format!("w.put_f64({v});"),
        Prim::Bool => format!("w.put_bool({v});"),
        Prim::Uuid => format!("w.put_uuid(&{v});"),
        Prim::String => format!("w.put_string(&{v})?;"),
        Prim::ByteArray => format!("w.put_byte_array(&{v})?;"),
        Prim::RestBuffer => format!("w.put_slice(&{v});"),
        Prim::Nbt => format!("{v}.write(w);"),
        Prim::OptionalNbt => format!("RawNbt::write_optional(&{v}, w);"),
        Prim::AnonNbt => format!("{v}.write(w);"),
        Prim::AnonOptionalNbt => format!("AnonNbt::write_optional(&{v}, w);"),
        Prim::LpVec3 => format!("{v}.write(w);"),
        Prim::Position => format!("{v}.write(w, PositionLayout::{layout})?;"),
        Prim::Slot => format!("{v}.encode(w)?;"),
    }
}

fn put_count(p: Prim, len: &str) -> String {
    match p {
        Prim::VarInt => format!("w.put_len_varint({len})?;"),
        Prim::I16 => format!("w.put_i16(crate::codec::count_i16({len})?);"),
        Prim::I32 => format!("w.put_i32(crate::codec::count_i32({len})?);"),
        Prim::U8 => format!("w.put_u8(crate::codec::count_u8({len})?);"),
        other => panic!("unsupported count type {other:?}"),
    }
}

fn total_bits(b: &Bitfield) -> u32 {
    b.fields.iter().map(|f| f.size).sum()
}

fn read_bits(b: &Bitfield) -> String {
    match total_bits(b) {
        8 => "u64::from(r.read_u8()?)".into(),
        16 => "u64::from(r.read_u16()?)".into(),
        32 => "u64::from(r.read_i32()? as u32)".into(),
        64 => "r.read_u64()?".into(),
        n => format!(
            "r.take({})?.iter().fold(0u64, |a, &b| (a << 8) | u64::from(b))",
            n / 8
        ),
    }
}

fn extract_bits(bits: &str, b: &Bitfield, i: usize) -> String {
    let total = total_bits(b);
    let before: u32 = b.fields[..i].iter().map(|f| f.size).sum();
    let f = &b.fields[i];
    let shift = total - before - f.size;
    let mask = if f.size == 64 {
        u64::MAX
    } else {
        (1u64 << f.size) - 1
    };
    let ty = int_type(f.size, f.signed);
    if f.signed {
        let s = 64 - f.size;
        format!("(((((({bits} >> {shift}) & {mask:#x}) << {s}) as i64) >> {s}) as {ty})")
    } else {
        format!("((({bits} >> {shift}) & {mask:#x}) as {ty})")
    }
}

fn write_bits(b: &Bitfield, parts: &[(String, &BitField)]) -> String {
    let total = total_bits(b);
    let mut out = String::from("{ let mut bits: u64 = 0;");
    for (expr, f) in parts {
        let (lo, hi) = bits_range(f.size, f.signed);
        let mask = if f.size == 64 {
            u64::MAX
        } else {
            (1u64 << f.size) - 1
        };
        let _ = write!(
            out,
            "{{ let x = i128::from({expr}); if x < {lo} || x > {hi} {{ return Err(EncodeError::OutOfRange({:?})); }} bits = (bits << {}) | ((x as u64) & {mask:#x}); }}",
            f.name, f.size
        );
    }
    let _ = write!(
        out,
        "{} }}",
        match total {
            8 => "w.put_u8(bits as u8);".to_string(),
            16 => "w.put_u16(bits as u16);".to_string(),
            32 => "w.put_u32(bits as u32);".to_string(),
            64 => "w.put_u64(bits);".to_string(),
            n => format!("w.put_slice(&bits.to_be_bytes()[{}..]);", 8 - n / 8),
        }
    );
    out
}

fn gen_bits(f: &BitField, hint: Option<&Hint>) -> String {
    let (lo, hi) = bits_range(f.size, f.signed);
    let vals: Vec<i64> = hint
        .map(|h| {
            h.values
                .iter()
                .filter_map(|k| parse_int(k))
                .filter(|v| (*v as i128) >= lo && (*v as i128) <= hi)
                .collect()
        })
        .unwrap_or_default();
    format!(
        "(g.int_in({lo}, {hi}, &{vals:?}) as {})",
        int_type(f.size, f.signed)
    )
}
