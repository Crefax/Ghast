//! Registry references that may carry their value inline (1.20.5+).

use bytes::BytesMut;
use bytestring::ByteString;

use crate::codec::{Decode, DecodeResult, Encode, EncodeResult, McWrite, Reader};
use crate::error::EncodeError;

/// A registry entry: either a reference by registry id or the value itself
/// (protodef `registryEntryHolder`).
///
/// On the wire a VarInt: `0` means the value follows inline, any other value is the
/// registry id plus one. `Id(-1)` therefore cannot be encoded.
#[derive(Debug, Clone, PartialEq)]
pub enum Holder<T> {
    /// Registry id (already shifted back, i.e. the wire value minus one).
    Id(i32),
    /// The value, sent inline.
    Inline(T),
}

impl<T> Holder<T> {
    /// The registry id, if this is a reference.
    #[inline]
    pub fn id(&self) -> Option<i32> {
        match *self {
            Holder::Id(id) => Some(id),
            Holder::Inline(_) => None,
        }
    }
}

/// A set of registry entries: a tag or an explicit list of registry ids
/// (protodef `registryEntryHolderSet`).
///
/// On the wire a VarInt `n`: `0` means a tag name (identifier string) follows, any other
/// value is followed by `n - 1` VarInt registry ids.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HolderSet {
    /// A tag, e.g. `minecraft:logs`.
    Tag(ByteString),
    /// Explicit registry ids.
    Ids(Vec<i32>),
}

impl Decode for HolderSet {
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let n = r.read_varint()?;
        if n == 0 {
            return Ok(HolderSet::Tag(r.read_string()?));
        }
        let count = r.check_count(i64::from(n) - 1, 1)?;
        let mut ids = Vec::with_capacity(count.min(crate::codec::PREALLOC_LIMIT));
        for _ in 0..count {
            ids.push(r.read_varint()?);
        }
        Ok(HolderSet::Ids(ids))
    }
}

impl HolderSet {
    /// Skips one encoded holder set without building it.
    pub fn skip(r: &mut Reader<'_>) -> DecodeResult<()> {
        let n = r.read_varint()?;
        if n == 0 {
            r.read_string()?;
            return Ok(());
        }
        let count = r.check_count(i64::from(n) - 1, 1)?;
        for _ in 0..count {
            r.read_varint()?;
        }
        Ok(())
    }
}

impl Encode for HolderSet {
    fn encode(&self, w: &mut BytesMut) -> EncodeResult {
        match self {
            HolderSet::Tag(t) => {
                w.put_varint(0);
                w.put_string(t)
            }
            HolderSet::Ids(ids) => {
                let n = i32::try_from(ids.len())
                    .ok()
                    .and_then(|n| n.checked_add(1))
                    .ok_or(EncodeError::OutOfRange("holder set length"))?;
                w.put_varint(n);
                for &id in ids {
                    w.put_varint(id);
                }
                Ok(())
            }
        }
    }
}

#[cfg(feature = "testgen")]
impl crate::testgen::Generate for HolderSet {
    fn generate(g: &mut crate::testgen::Gen) -> Self {
        if g.bool() {
            HolderSet::Tag(g.string())
        } else {
            HolderSet::Ids(
                (0..g.count())
                    .map(|_| g.int_in(i64::from(i32::MIN), i64::from(i32::MAX), &[0, 1]) as i32)
                    .collect(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::decode_exact;

    #[test]
    fn holder_set_wire() {
        let tag = HolderSet::Tag("minecraft:logs".into());
        let mut w = BytesMut::new();
        tag.encode(&mut w).unwrap();
        assert_eq!(w[0], 0);
        assert_eq!(decode_exact::<HolderSet>(&w.freeze()).unwrap(), tag);
        let ids = HolderSet::Ids(vec![3, 300]);
        let mut w = BytesMut::new();
        ids.encode(&mut w).unwrap();
        assert_eq!(&w[..], &[3, 3, 0xac, 0x02]);
        assert_eq!(decode_exact::<HolderSet>(&w.freeze()).unwrap(), ids);
    }
}
