//! Named Binary Tag (NBT) as used on the network by 1.8 – 1.20.1.
//!
//! The protocol carries NBT as a *named root tag*: a tag-type byte, a `u16`-prefixed
//! name and the payload. A single `0` byte (TAG_End) means "no compound" — vanilla's
//! packet buffer reads both its plain and its optional NBT that way.
//!
//! Packets keep NBT as [`RawNbt`]: the exact bytes of the root tag, found by *skipping*
//! over the structure without allocating. Turning it into a tree ([`RawNbt::parse`]) is
//! opt-in, so a client that only needs item ids and counts from `window_items` never pays
//! for building NBT trees (on busy servers that NBT is most of the inbound bytes).
//!
//! Strings are Java "Modified UTF-8" (NUL as `C0 80`, supplementary characters as
//! surrogate pairs). Raw bytes are always preserved exactly; the parsed tree decodes
//! strings lossily (unpaired surrogates become U+FFFD).

use bytes::{BufMut, Bytes, BytesMut};

use crate::codec::{Decode, DecodeResult, Encode, EncodeResult, Reader};
use crate::error::{DecodeError, EncodeError};

/// Maximum nesting depth of compounds and lists (same as vanilla).
pub const MAX_DEPTH: usize = 512;

/// NBT tag type ids.
#[allow(missing_docs)]
pub mod tag {
    pub const END: u8 = 0;
    pub const BYTE: u8 = 1;
    pub const SHORT: u8 = 2;
    pub const INT: u8 = 3;
    pub const LONG: u8 = 4;
    pub const FLOAT: u8 = 5;
    pub const DOUBLE: u8 = 6;
    pub const BYTE_ARRAY: u8 = 7;
    pub const STRING: u8 = 8;
    pub const LIST: u8 = 9;
    pub const COMPOUND: u8 = 10;
    pub const INT_ARRAY: u8 = 11;
    pub const LONG_ARRAY: u8 = 12;
}

/// The undecoded bytes of one network NBT root tag (type byte, name and payload), or a
/// single `0` byte for "no compound".
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RawNbt(Bytes);

impl RawNbt {
    /// The "no compound" value (a single TAG_End byte).
    pub const END: RawNbt = RawNbt(Bytes::from_static(&[0]));

    /// The raw bytes, exactly as they appear on the wire.
    #[inline]
    pub fn as_bytes(&self) -> &Bytes {
        &self.0
    }

    /// Whether this is the single-byte "no compound" value.
    #[inline]
    pub fn is_end(&self) -> bool {
        self.0.first() == Some(&tag::END)
    }

    /// Validates `bytes` as exactly one root tag and wraps them.
    pub fn from_bytes(bytes: Bytes) -> DecodeResult<Self> {
        let mut r = Reader::new(&bytes);
        let v = Self::read(&mut r)?;
        r.finish()?;
        Ok(v)
    }

    /// Reads a root tag (or a single TAG_End byte) by skipping over its structure.
    pub fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let start = r.position();
        let ty = r.read_u8()?;
        if ty != tag::END {
            let name_len = r.read_u16()? as usize;
            r.skip(name_len)?;
            skip_payload(r, ty, 0)?;
        }
        Ok(RawNbt(r.bytes_since(start)))
    }

    /// Reads protodef `optionalNbt`: a `0` byte is `None`, anything else a root tag.
    pub fn read_optional(r: &mut Reader<'_>) -> DecodeResult<Option<Self>> {
        if r.peek_u8()? == tag::END {
            r.skip(1)?;
            Ok(None)
        } else {
            Self::read(r).map(Some)
        }
    }

    /// Writes the raw bytes.
    #[inline]
    pub fn write(&self, w: &mut BytesMut) {
        w.put_slice(&self.0);
    }

    /// Writes protodef `optionalNbt`.
    #[inline]
    pub fn write_optional(v: &Option<Self>, w: &mut BytesMut) {
        match v {
            Some(n) => n.write(w),
            None => w.put_u8(tag::END),
        }
    }

    /// Parses the tree. Returns `None` for the "no compound" value.
    pub fn parse(&self) -> DecodeResult<Option<NamedTag>> {
        let mut r = Reader::new(&self.0);
        let ty = r.read_u8()?;
        if ty == tag::END {
            return Ok(None);
        }
        let name = read_mutf8(&mut r)?;
        let tag = read_payload(&mut r, ty, 0)?;
        r.finish()?;
        Ok(Some(NamedTag { name, tag }))
    }

    /// Serialises a tree into raw network bytes.
    pub fn from_named(t: &NamedTag) -> Result<Self, EncodeError> {
        let mut w = BytesMut::new();
        t.encode(&mut w)?;
        Ok(RawNbt(w.freeze()))
    }
}

impl Decode for RawNbt {
    #[inline]
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Self::read(r)
    }
}

impl Encode for RawNbt {
    #[inline]
    fn encode(&self, w: &mut BytesMut) -> EncodeResult {
        self.write(w);
        Ok(())
    }
}

/// Skips the payload of a tag of type `ty`.
fn skip_payload(r: &mut Reader<'_>, ty: u8, depth: usize) -> DecodeResult<()> {
    match ty {
        tag::BYTE => r.skip(1),
        tag::SHORT => r.skip(2),
        tag::INT | tag::FLOAT => r.skip(4),
        tag::LONG | tag::DOUBLE => r.skip(8),
        tag::BYTE_ARRAY => {
            let n = read_array_len(r, 1)?;
            r.skip(n)
        }
        tag::INT_ARRAY => {
            let n = read_array_len(r, 4)?;
            r.skip(n * 4)
        }
        tag::LONG_ARRAY => {
            let n = read_array_len(r, 8)?;
            r.skip(n * 8)
        }
        tag::STRING => {
            let n = r.read_u16()? as usize;
            r.skip(n)
        }
        tag::LIST => {
            if depth >= MAX_DEPTH {
                return Err(DecodeError::NbtTooDeep(MAX_DEPTH));
            }
            let elem = r.read_u8()?;
            let n = read_list_len(r, elem)?;
            match elem {
                // Fixed-size elements: skip in one step.
                tag::BYTE => r.skip(n),
                tag::SHORT => r.skip(n * 2),
                tag::INT | tag::FLOAT => r.skip(n * 4),
                tag::LONG | tag::DOUBLE => r.skip(n * 8),
                _ => {
                    for _ in 0..n {
                        skip_payload(r, elem, depth + 1)?;
                    }
                    Ok(())
                }
            }
        }
        tag::COMPOUND => {
            if depth >= MAX_DEPTH {
                return Err(DecodeError::NbtTooDeep(MAX_DEPTH));
            }
            loop {
                let t = r.read_u8()?;
                if t == tag::END {
                    return Ok(());
                }
                let name_len = r.read_u16()? as usize;
                r.skip(name_len)?;
                skip_payload(r, t, depth + 1)?;
            }
        }
        other => Err(DecodeError::InvalidNbtTag(other)),
    }
}

/// Reads an `i32` array length and checks it against the remaining input.
fn read_array_len(r: &mut Reader<'_>, elem_size: usize) -> DecodeResult<usize> {
    let n = r.read_i32()?;
    r.check_count(i64::from(n), elem_size)
}

/// Smallest encoded size of one list element of type `elem`.
fn min_size(elem: u8) -> usize {
    match elem {
        tag::BYTE | tag::COMPOUND => 1,
        tag::SHORT | tag::STRING => 2,
        tag::INT | tag::FLOAT | tag::BYTE_ARRAY | tag::INT_ARRAY | tag::LONG_ARRAY => 4,
        tag::LONG | tag::DOUBLE => 8,
        tag::LIST => 5,
        _ => 1,
    }
}

fn read_list_len(r: &mut Reader<'_>, elem: u8) -> DecodeResult<usize> {
    let n = r.read_i32()?;
    if elem == tag::END {
        return if n <= 0 {
            Ok(0)
        } else {
            Err(DecodeError::NbtEndList)
        };
    }
    if elem > tag::LONG_ARRAY {
        return Err(DecodeError::InvalidNbtTag(elem));
    }
    r.check_count(i64::from(n), min_size(elem))
}

/// A named root tag.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedTag {
    /// Root name (usually empty on the network).
    pub name: String,
    /// The root tag (usually a compound).
    pub tag: Tag,
}

/// An NBT tag.
#[derive(Debug, Clone, PartialEq)]
#[allow(missing_docs)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Bytes),
    String(String),
    List(List),
    Compound(Compound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

/// An NBT list: element type id and elements (all of that type).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct List {
    /// Element tag type ([`tag::END`] for an empty untyped list).
    pub elem_type: u8,
    /// The elements; each must be of `elem_type`.
    pub items: Vec<Tag>,
}

/// An NBT compound: named tags in wire order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Compound(pub Vec<(String, Tag)>);

impl Compound {
    /// Looks up an entry by name (linear scan; compounds are small).
    pub fn get(&self, name: &str) -> Option<&Tag> {
        self.0.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }
}

impl Tag {
    /// The tag type id.
    pub fn type_id(&self) -> u8 {
        match self {
            Tag::Byte(_) => tag::BYTE,
            Tag::Short(_) => tag::SHORT,
            Tag::Int(_) => tag::INT,
            Tag::Long(_) => tag::LONG,
            Tag::Float(_) => tag::FLOAT,
            Tag::Double(_) => tag::DOUBLE,
            Tag::ByteArray(_) => tag::BYTE_ARRAY,
            Tag::String(_) => tag::STRING,
            Tag::List(_) => tag::LIST,
            Tag::Compound(_) => tag::COMPOUND,
            Tag::IntArray(_) => tag::INT_ARRAY,
            Tag::LongArray(_) => tag::LONG_ARRAY,
        }
    }

    /// The compound, if this is one.
    pub fn as_compound(&self) -> Option<&Compound> {
        match self {
            Tag::Compound(c) => Some(c),
            _ => None,
        }
    }
}

fn read_payload(r: &mut Reader<'_>, ty: u8, depth: usize) -> DecodeResult<Tag> {
    Ok(match ty {
        tag::BYTE => Tag::Byte(r.read_i8()?),
        tag::SHORT => Tag::Short(r.read_i16()?),
        tag::INT => Tag::Int(r.read_i32()?),
        tag::LONG => Tag::Long(r.read_i64()?),
        tag::FLOAT => Tag::Float(r.read_f32()?),
        tag::DOUBLE => Tag::Double(r.read_f64()?),
        tag::BYTE_ARRAY => {
            let n = read_array_len(r, 1)?;
            Tag::ByteArray(r.read_bytes(n)?)
        }
        tag::INT_ARRAY => {
            let n = read_array_len(r, 4)?;
            let s = r.take(n * 4)?;
            Tag::IntArray(
                s.chunks_exact(4)
                    .map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]]))
                    .collect(),
            )
        }
        tag::LONG_ARRAY => {
            let n = read_array_len(r, 8)?;
            let s = r.take(n * 8)?;
            Tag::LongArray(
                s.chunks_exact(8)
                    .map(|c| i64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
                    .collect(),
            )
        }
        tag::STRING => Tag::String(read_mutf8(r)?),
        tag::LIST => {
            if depth >= MAX_DEPTH {
                return Err(DecodeError::NbtTooDeep(MAX_DEPTH));
            }
            let elem = r.read_u8()?;
            let n = read_list_len(r, elem)?;
            let mut items = Vec::with_capacity(n);
            for _ in 0..n {
                items.push(read_payload(r, elem, depth + 1)?);
            }
            Tag::List(List {
                elem_type: elem,
                items,
            })
        }
        tag::COMPOUND => {
            if depth >= MAX_DEPTH {
                return Err(DecodeError::NbtTooDeep(MAX_DEPTH));
            }
            let mut entries = Vec::new();
            loop {
                let t = r.read_u8()?;
                if t == tag::END {
                    break;
                }
                let name = read_mutf8(r)?;
                entries.push((name, read_payload(r, t, depth + 1)?));
            }
            Tag::Compound(Compound(entries))
        }
        other => return Err(DecodeError::InvalidNbtTag(other)),
    })
}

impl Encode for NamedTag {
    fn encode(&self, w: &mut BytesMut) -> EncodeResult {
        w.put_u8(self.tag.type_id());
        write_mutf8(w, &self.name)?;
        write_payload(w, &self.tag, 0)
    }
}

fn i32_len(n: usize) -> Result<i32, EncodeError> {
    i32::try_from(n).map_err(|_| EncodeError::LengthOverflow {
        len: n,
        max: i32::MAX as usize,
    })
}

fn write_payload(w: &mut BytesMut, t: &Tag, depth: usize) -> EncodeResult {
    if depth > MAX_DEPTH {
        return Err(EncodeError::Invalid("NBT nesting too deep"));
    }
    match t {
        Tag::Byte(v) => w.put_i8(*v),
        Tag::Short(v) => w.put_i16(*v),
        Tag::Int(v) => w.put_i32(*v),
        Tag::Long(v) => w.put_i64(*v),
        Tag::Float(v) => w.put_f32(*v),
        Tag::Double(v) => w.put_f64(*v),
        Tag::ByteArray(b) => {
            w.put_i32(i32_len(b.len())?);
            w.put_slice(b);
        }
        Tag::String(s) => write_mutf8(w, s)?,
        Tag::List(l) => {
            w.put_u8(l.elem_type);
            w.put_i32(i32_len(l.items.len())?);
            if l.elem_type == tag::END && !l.items.is_empty() {
                return Err(EncodeError::Invalid("NBT list of TAG_End with elements"));
            }
            for item in &l.items {
                if item.type_id() != l.elem_type {
                    return Err(EncodeError::Invalid("NBT list element of the wrong type"));
                }
                write_payload(w, item, depth + 1)?;
            }
        }
        Tag::Compound(c) => {
            for (k, v) in &c.0 {
                w.put_u8(v.type_id());
                write_mutf8(w, k)?;
                write_payload(w, v, depth + 1)?;
            }
            w.put_u8(tag::END);
        }
        Tag::IntArray(a) => {
            w.put_i32(i32_len(a.len())?);
            for v in a {
                w.put_i32(*v);
            }
        }
        Tag::LongArray(a) => {
            w.put_i32(i32_len(a.len())?);
            for v in a {
                w.put_i64(*v);
            }
        }
    }
    Ok(())
}

/// Reads a `u16`-prefixed Modified UTF-8 string.
fn read_mutf8(r: &mut Reader<'_>) -> DecodeResult<String> {
    let n = r.read_u16()? as usize;
    let b = r.take(n)?;
    decode_mutf8(b)
}

/// Decodes Java Modified UTF-8.
pub fn decode_mutf8(b: &[u8]) -> DecodeResult<String> {
    // Fast path: plain ASCII without NUL (by far the common case) or UTF-8 that cannot
    // contain MUTF-8 specials (`C0 80` for NUL, `ED` lead bytes for surrogates).
    if b.iter().all(|&c| c != 0 && c < 0x80) {
        return std::str::from_utf8(b)
            .map(str::to_owned)
            .map_err(|_| DecodeError::InvalidUtf8);
    }
    if !b.iter().any(|&c| c == 0 || c == 0xc0 || c == 0xed)
        && let Ok(s) = std::str::from_utf8(b)
    {
        return Ok(s.to_owned());
    }
    let mut units: Vec<u16> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c < 0x80 {
            units.push(u16::from(c));
            i += 1;
        } else if c & 0xe0 == 0xc0 {
            let c2 = *b.get(i + 1).ok_or(DecodeError::InvalidUtf8)?;
            if c2 & 0xc0 != 0x80 {
                return Err(DecodeError::InvalidUtf8);
            }
            units.push((u16::from(c & 0x1f) << 6) | u16::from(c2 & 0x3f));
            i += 2;
        } else if c & 0xf0 == 0xe0 {
            let c2 = *b.get(i + 1).ok_or(DecodeError::InvalidUtf8)?;
            let c3 = *b.get(i + 2).ok_or(DecodeError::InvalidUtf8)?;
            if c2 & 0xc0 != 0x80 || c3 & 0xc0 != 0x80 {
                return Err(DecodeError::InvalidUtf8);
            }
            units.push(
                (u16::from(c & 0x0f) << 12) | (u16::from(c2 & 0x3f) << 6) | u16::from(c3 & 0x3f),
            );
            i += 3;
        } else {
            return Err(DecodeError::InvalidUtf8);
        }
    }
    Ok(String::from_utf16_lossy(&units))
}

/// Writes a `u16`-prefixed Modified UTF-8 string.
fn write_mutf8(w: &mut BytesMut, s: &str) -> EncodeResult {
    let too_long = |len| EncodeError::LengthOverflow {
        len,
        max: u16::MAX as usize,
    };
    // Fast path: no NUL and no supplementary characters means MUTF-8 == UTF-8.
    if s.bytes().all(|b| b != 0 && b < 0xf0) {
        let n = u16::try_from(s.len()).map_err(|_| too_long(s.len()))?;
        w.put_u16(n);
        w.put_slice(s.as_bytes());
        return Ok(());
    }
    let mut out = Vec::with_capacity(s.len() + 8);
    for u in s.encode_utf16() {
        match u {
            0x0001..=0x007f => out.push(u as u8),
            0x0000 | 0x0080..=0x07ff => {
                out.push(0xc0 | (u >> 6) as u8);
                out.push(0x80 | (u & 0x3f) as u8);
            }
            _ => {
                out.push(0xe0 | (u >> 12) as u8);
                out.push(0x80 | ((u >> 6) & 0x3f) as u8);
                out.push(0x80 | (u & 0x3f) as u8);
            }
        }
    }
    let n = u16::try_from(out.len()).map_err(|_| too_long(out.len()))?;
    w.put_u16(n);
    w.put_slice(&out);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The classic `hello_world.nbt` test file (uncompressed).
    const HELLO_WORLD: &[u8] = &[
        0x0a, 0x00, 0x0b, b'h', b'e', b'l', b'l', b'o', b' ', b'w', b'o', b'r', b'l', b'd', 0x08,
        0x00, 0x04, b'n', b'a', b'm', b'e', 0x00, 0x09, b'B', b'a', b'n', b'a', b'n', b'r', b'a',
        b'm', b'a', 0x00,
    ];

    #[test]
    fn hello_world() {
        let b = Bytes::from_static(HELLO_WORLD);
        let raw = RawNbt::from_bytes(b.clone()).unwrap();
        let t = raw.parse().unwrap().unwrap();
        assert_eq!(t.name, "hello world");
        let c = t.tag.as_compound().unwrap();
        assert_eq!(c.get("name"), Some(&Tag::String("Bananrama".into())));
        assert_eq!(RawNbt::from_named(&t).unwrap().as_bytes(), &b);
    }

    #[test]
    fn optional_and_end() {
        let b = Bytes::from_static(&[0x00, 0x01]);
        let mut r = Reader::new(&b);
        assert_eq!(RawNbt::read_optional(&mut r), Ok(None));
        assert_eq!(r.remaining(), 1);
        let b = Bytes::from_static(&[0x00]);
        let raw = RawNbt::from_bytes(b).unwrap();
        assert!(raw.is_end());
        assert_eq!(raw.parse(), Ok(None));
    }

    #[test]
    fn rejects_bad_input() {
        // Unknown tag type inside a compound.
        let b = Bytes::from_static(&[0x0a, 0x00, 0x00, 0x0d, 0x00, 0x00]);
        assert_eq!(RawNbt::from_bytes(b), Err(DecodeError::InvalidNbtTag(13)));
        // List of TAG_End with a positive length.
        let b = Bytes::from_static(&[0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05]);
        assert_eq!(RawNbt::from_bytes(b), Err(DecodeError::NbtEndList));
        // Huge int array length is bounded by the input instead of allocating.
        let b = Bytes::from_static(&[0x0b, 0x00, 0x00, 0x7f, 0xff, 0xff, 0xff]);
        assert!(matches!(
            RawNbt::from_bytes(b),
            Err(DecodeError::LengthTooLarge { .. })
        ));
        // Deep nesting is limited.
        let mut v = vec![0x09, 0x00, 0x00];
        for _ in 0..600 {
            v.extend_from_slice(&[0x09, 0x00, 0x00, 0x00, 0x01]);
        }
        let b = Bytes::from(v);
        assert_eq!(
            RawNbt::from_bytes(b),
            Err(DecodeError::NbtTooDeep(MAX_DEPTH))
        );
    }

    #[test]
    fn modified_utf8() {
        let s = "a\0b\u{e9}\u{1f600}";
        let mut w = BytesMut::new();
        write_mutf8(&mut w, s).unwrap();
        // NUL -> C0 80, U+1F600 -> surrogate pair, each 3 bytes.
        assert_eq!(
            &w[..],
            &[
                0x00, 0x0c, b'a', 0xc0, 0x80, b'b', 0xc3, 0xa9, 0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80
            ]
        );
        assert_eq!(decode_mutf8(&w[2..]).unwrap(), s);
        assert_eq!(decode_mutf8(&[0xc3]), Err(DecodeError::InvalidUtf8));
        // Standard 4-byte UTF-8 (as written by some non-Java encoders) is accepted.
        assert_eq!(
            decode_mutf8(&[0xf0, 0x9f, 0x98, 0x80]).unwrap(),
            "\u{1f600}"
        );
        assert_eq!(decode_mutf8(&[0xf8, 0x80]), Err(DecodeError::InvalidUtf8));
    }
}
