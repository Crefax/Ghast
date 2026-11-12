//! Inventory slots (item stacks) in their per-version wire formats.
//!
//! NBT is kept raw ([`RawNbt`](crate::nbt::RawNbt)): decoding a `window_items` packet only walks the NBT to
//! find its end; building a tree is up to the caller ([`RawNbt::parse`](crate::nbt::RawNbt::parse)).

/// Slot format of 1.8 – 1.12.2: `i16` item id (`-1` = empty), `i8` count, `i16` damage,
/// optional NBT (`0` byte = none).
pub mod legacy {
    use bytes::{BufMut, BytesMut};

    use crate::codec::{Decode, DecodeResult, Encode, EncodeResult, Reader};
    use crate::error::EncodeError;
    use crate::nbt::RawNbt;

    /// A non-empty item stack.
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub struct ItemStack {
        /// Numeric item id. Never `-1` (that encodes an empty slot).
        pub id: i16,
        /// Stack size.
        pub count: i8,
        /// Damage / metadata value.
        pub damage: i16,
        /// Item NBT, if any.
        pub nbt: Option<RawNbt>,
    }

    /// One slot: empty or an item stack.
    #[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
    pub struct Slot(pub Option<ItemStack>);

    impl Slot {
        /// The empty slot.
        pub const EMPTY: Slot = Slot(None);

        /// Whether the slot is empty.
        #[inline]
        pub fn is_empty(&self) -> bool {
            self.0.is_none()
        }

        /// The item stack, if any.
        #[inline]
        pub fn item(&self) -> Option<&ItemStack> {
            self.0.as_ref()
        }
    }

    impl Decode for Slot {
        #[inline]
        fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
            let id = r.read_i16()?;
            if id == -1 {
                return Ok(Slot(None));
            }
            Ok(Slot(Some(ItemStack {
                id,
                count: r.read_i8()?,
                damage: r.read_i16()?,
                nbt: RawNbt::read_optional(r)?,
            })))
        }
    }

    impl Encode for Slot {
        #[inline]
        fn encode(&self, w: &mut BytesMut) -> EncodeResult {
            match &self.0 {
                None => w.put_i16(-1),
                Some(it) => {
                    if it.id == -1 {
                        return Err(EncodeError::Invalid(
                            "item id -1 is reserved for empty slots",
                        ));
                    }
                    w.put_i16(it.id);
                    w.put_i8(it.count);
                    w.put_i16(it.damage);
                    RawNbt::write_optional(&it.nbt, w);
                }
            }
            Ok(())
        }
    }
}

/// Slot format of 1.13.2 – 1.20.4: `bool` present, then VarInt item id, `i8` count,
/// optional NBT.
pub mod modern {
    use bytes::{BufMut, BytesMut};

    use crate::codec::{Decode, DecodeResult, Encode, EncodeResult, McWrite, Reader};
    use crate::nbt::RawNbt;

    /// A non-empty item stack.
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub struct ItemStack {
        /// Registry item id.
        pub id: i32,
        /// Stack size.
        pub count: i8,
        /// Item NBT, if any.
        pub nbt: Option<RawNbt>,
    }

    /// One slot: empty or an item stack.
    #[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
    pub struct Slot(pub Option<ItemStack>);

    impl Slot {
        /// The empty slot.
        pub const EMPTY: Slot = Slot(None);

        /// Whether the slot is empty.
        #[inline]
        pub fn is_empty(&self) -> bool {
            self.0.is_none()
        }

        /// The item stack, if any.
        #[inline]
        pub fn item(&self) -> Option<&ItemStack> {
            self.0.as_ref()
        }
    }

    impl Decode for Slot {
        #[inline]
        fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
            if !r.read_bool()? {
                return Ok(Slot(None));
            }
            Ok(Slot(Some(ItemStack {
                id: r.read_varint()?,
                count: r.read_i8()?,
                nbt: RawNbt::read_optional(r)?,
            })))
        }
    }

    impl Encode for Slot {
        #[inline]
        fn encode(&self, w: &mut BytesMut) -> EncodeResult {
            match &self.0 {
                None => w.put_bool(false),
                Some(it) => {
                    w.put_bool(true);
                    w.put_varint(it.id);
                    w.put_i8(it.count);
                    RawNbt::write_optional(&it.nbt, w);
                }
            }
            Ok(())
        }
    }
}

/// Slot format of 26.1 (protocol 775) with lazily decoded data components.
///
/// On the wire (vanilla `ItemStack.OPTIONAL_STREAM_CODEC`): VarInt count (`<= 0` = empty),
/// VarInt item id, VarInt added-component count, VarInt removed-component count, the added
/// components (VarInt type + value, **not** length-prefixed), the removed component types.
/// [`ItemStack::read`](v775::ItemStack::read) walks the components with the generated
/// structural skip ([`SlotComponent::skip`](crate::defs::p775::types::SlotComponent::skip))
/// and keeps the patch as one zero-copy [`Bytes`](bytes::Bytes) slice; a
/// component is decoded only when asked for.
///
/// 26.2 (protocol 776) has the same slot format with its own component registry (one more
/// component type, so most type ids differ). Its stacks use the same
/// [`ComponentPatch`](v775::ComponentPatch) and [`ItemStack`](v775::ItemStack) types, read
/// with [`ComponentPatch::read_776`](v775::ComponentPatch::read_776) /
/// [`ItemStack::read_776`](v775::ItemStack::read_776); the patch remembers which protocol's
/// component codec it was read with ([`ComponentPatch::protocol`](v775::ComponentPatch::protocol)),
/// so iteration skips with the right one. Type ids in a
/// patch are always the ids of that protocol.
#[cfg(any(feature = "v26_1", feature = "v26_2"))]
pub mod v775 {
    use bytes::{BufMut, Bytes, BytesMut};

    use crate::codec::{DecodeResult, McWrite, Reader};
    #[cfg(feature = "v26_1")]
    use crate::defs::p775::types::SlotComponent;
    use crate::error::DecodeError;

    /// The component codec a patch was read with.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    enum Codec {
        /// Protocol 775 (26.1 - 26.1.2).
        #[default]
        P775,
        /// Protocol 776 (26.2).
        P776,
    }

    /// Skips one component (type id + value) with `codec`'s generated structural skip.
    #[inline]
    fn skip_one(codec: Codec, r: &mut Reader<'_>) -> DecodeResult<()> {
        match codec {
            #[cfg(feature = "v26_1")]
            Codec::P775 => SlotComponent::skip(r),
            #[cfg(feature = "v26_2")]
            Codec::P776 => crate::defs::p776::types::SlotComponent::skip(r),
            #[allow(unreachable_patterns)]
            _ => Err(DecodeError::Invalid(
                "component codec of a disabled protocol",
            )),
        }
    }

    /// The data component patch of a stack (added components, then removed types), exactly as
    /// received.
    #[derive(Debug, Clone, Default)]
    pub struct ComponentPatch {
        /// Added components followed by the removed types.
        bytes: Bytes,
        /// Byte offset of the removed list in `bytes`.
        split: u32,
        added: u16,
        removed: u16,
        /// Added / removed types below 127 as bits; bit 127 = a type outside `0..127` occurs
        /// (lookups of such types scan). Lets lookups of absent types skip the scan.
        added_mask: u128,
        removed_mask: u128,
        /// Fits in the padding after `removed`: the struct does not grow.
        codec: Codec,
    }

    /// The mask bit of a component type.
    #[inline]
    fn bit(ty: i32) -> u128 {
        match u32::try_from(ty) {
            Ok(t) if t < 127 => 1u128 << t,
            _ => 1u128 << 127,
        }
    }

    /// Whether a mask may contain `ty`.
    #[inline]
    fn may_have(mask: u128, ty: i32) -> bool {
        mask & (bit(ty) | 1u128 << 127) != 0
    }

    /// Patches are equal when their bytes are (component order included; see
    /// [`ComponentPatch::same_components`] for vanilla's map equality).
    impl PartialEq for ComponentPatch {
        fn eq(&self, other: &Self) -> bool {
            self.added == other.added
                && self.removed == other.removed
                && self.bytes == other.bytes
                && (self.codec == other.codec || self.bytes.is_empty())
        }
    }

    impl Eq for ComponentPatch {}

    impl std::hash::Hash for ComponentPatch {
        fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
            self.bytes.hash(h);
        }
    }

    impl ComponentPatch {
        /// The empty patch (the item's default components).
        pub const EMPTY: ComponentPatch = ComponentPatch {
            bytes: Bytes::new(),
            split: 0,
            added: 0,
            removed: 0,
            added_mask: 0,
            removed_mask: 0,
            codec: Codec::P775,
        };

        /// Reads `added` components and `removed` types (protocol 775) by skipping over them.
        #[cfg(feature = "v26_1")]
        pub fn read(r: &mut Reader<'_>, added: i32, removed: i32) -> DecodeResult<Self> {
            Self::read_with(Codec::P775, r, added, removed)
        }

        /// [`read`](Self::read) for protocol 776 (26.2).
        #[cfg(feature = "v26_2")]
        pub fn read_776(r: &mut Reader<'_>, added: i32, removed: i32) -> DecodeResult<Self> {
            Self::read_with(Codec::P776, r, added, removed)
        }

        /// The protocol number whose component registry the type ids belong to (775 or
        /// 776).
        #[must_use]
        pub fn protocol(&self) -> i32 {
            match self.codec {
                Codec::P775 => 775,
                Codec::P776 => 776,
            }
        }

        #[inline]
        fn read_with(
            codec: Codec,
            r: &mut Reader<'_>,
            added: i32,
            removed: i32,
        ) -> DecodeResult<Self> {
            let added_n = r.check_count(i64::from(added), 1)?;
            let removed_n = r.check_count(i64::from(removed), 1)?;
            let (Ok(added), Ok(removed)) = (u16::try_from(added_n), u16::try_from(removed_n))
            else {
                return Err(DecodeError::Invalid("component count"));
            };
            let start = r.position();
            let mut added_mask = 0u128;
            for _ in 0..added_n {
                added_mask |= bit(r.clone().read_varint()?);
                skip_one(codec, r)?;
            }
            let split = (r.position() - start) as u32;
            let mut removed_mask = 0u128;
            for _ in 0..removed_n {
                removed_mask |= bit(r.read_varint()?);
            }
            Ok(ComponentPatch {
                bytes: r.bytes_since(start),
                split,
                added,
                removed,
                added_mask,
                removed_mask,
                codec,
            })
        }

        /// `true` when nothing is added or removed.
        #[must_use]
        pub fn is_empty(&self) -> bool {
            self.added == 0 && self.removed == 0
        }

        /// Number of added components.
        #[must_use]
        pub fn added_len(&self) -> usize {
            usize::from(self.added)
        }

        /// Number of removed component types.
        #[must_use]
        pub fn removed_len(&self) -> usize {
            usize::from(self.removed)
        }

        /// The patch bytes as received (added components, then removed types).
        #[must_use]
        pub fn as_bytes(&self) -> &Bytes {
            &self.bytes
        }

        /// The added components as `(type id, value bytes)`, in wire order.
        pub fn iter(&self) -> Components<'_> {
            Components {
                bytes: self.bytes.slice(..self.split as usize),
                pos: 0,
                left: self.added,
                codec: self.codec,
                _p: std::marker::PhantomData,
            }
        }

        /// The removed component type ids, in wire order.
        pub fn removed(&self) -> impl Iterator<Item = i32> + '_ {
            let tail = self.bytes.slice(self.split as usize..);
            let mut pos = 0usize;
            (0..self.removed).map_while(move |_| {
                let mut r = Reader::new(&tail);
                r.skip(pos).ok()?;
                let v = r.read_varint().ok()?;
                pos = r.position();
                Some(v)
            })
        }

        /// `true` when the patch removes component `ty`.
        #[must_use]
        pub fn removes(&self, ty: i32) -> bool {
            may_have(self.removed_mask, ty) && self.removed().any(|t| t == ty)
        }

        /// Whether the patch adds component `ty` (no scan when it certainly does not).
        #[must_use]
        pub fn adds(&self, ty: i32) -> bool {
            may_have(self.added_mask, ty) && self.iter().any(|(t, _)| t == ty)
        }

        /// The value bytes of added component `ty` (vanilla keeps the last one of a type).
        #[must_use]
        pub fn get(&self, ty: i32) -> Option<Bytes> {
            if !may_have(self.added_mask, ty) {
                return None;
            }
            self.iter().filter(|(t, _)| *t == ty).last().map(|(_, b)| b)
        }

        /// Byte offset of the last added component of type `ty`.
        fn last_of(&self, ty: i32) -> Option<DecodeResult<usize>> {
            if !may_have(self.added_mask, ty) {
                return None;
            }
            let mut r = Reader::new(&self.bytes);
            let mut last = None;
            for _ in 0..self.added {
                let start = r.position();
                let t = match r.clone().read_varint() {
                    Ok(t) => t,
                    Err(e) => return Some(Err(e)),
                };
                if let Err(e) = skip_one(self.codec, &mut r) {
                    return Some(Err(e));
                }
                if t == ty {
                    last = Some(start);
                }
            }
            last.map(Ok)
        }

        /// Decodes the component at byte offset `start` with `T`.
        fn decode_at<T: crate::codec::Decode>(&self, start: usize) -> DecodeResult<T> {
            let mut r = Reader::new(&self.bytes);
            r.skip(start)?;
            T::decode(&mut r)
        }

        /// Decodes added component `ty` with the generated type (the last one of that type).
        /// A patch of protocol 776 gives `Some(Err(..))`; use
        /// [`decode_776`](Self::decode_776).
        #[cfg(feature = "v26_1")]
        #[must_use]
        pub fn decode(&self, ty: i32) -> Option<DecodeResult<SlotComponent>> {
            if self.codec != Codec::P775 {
                return Some(Err(DecodeError::Invalid("patch of protocol 776")));
            }
            Some(self.last_of(ty)?.and_then(|start| self.decode_at(start)))
        }

        /// Decodes every added component with the generated type.
        ///
        /// # Errors
        /// A malformed component, or a patch of protocol 776.
        #[cfg(feature = "v26_1")]
        pub fn decode_all(&self) -> DecodeResult<Vec<SlotComponent>> {
            if self.codec != Codec::P775 {
                return Err(DecodeError::Invalid("patch of protocol 776"));
            }
            let mut r = Reader::new(&self.bytes);
            (0..self.added)
                .map(|_| <SlotComponent as crate::codec::Decode>::decode(&mut r))
                .collect()
        }

        /// [`decode`](Self::decode) for a patch of protocol 776 (26.2).
        #[cfg(feature = "v26_2")]
        #[must_use]
        pub fn decode_776(
            &self,
            ty: i32,
        ) -> Option<DecodeResult<crate::defs::p776::types::SlotComponent>> {
            if self.codec != Codec::P776 {
                return Some(Err(DecodeError::Invalid("patch of protocol 775")));
            }
            Some(self.last_of(ty)?.and_then(|start| self.decode_at(start)))
        }

        /// [`decode_all`](Self::decode_all) for a patch of protocol 776 (26.2).
        ///
        /// # Errors
        /// A malformed component, or a patch of protocol 775.
        #[cfg(feature = "v26_2")]
        pub fn decode_all_776(&self) -> DecodeResult<Vec<crate::defs::p776::types::SlotComponent>> {
            if self.codec != Codec::P776 {
                return Err(DecodeError::Invalid("patch of protocol 775"));
            }
            let mut r = Reader::new(&self.bytes);
            (0..self.added)
                .map(|_| {
                    <crate::defs::p776::types::SlotComponent as crate::codec::Decode>::decode(
                        &mut r,
                    )
                })
                .collect()
        }

        /// Vanilla's patch equality (`DataComponentPatch.equals`: a map from type to value):
        /// the same removed types and the same added `(type, value bytes)` set, in any order.
        /// Values compare by bytes (values that differ only in NBT compound key order compare
        /// unequal here; vanilla would compare their trees).
        #[must_use]
        pub fn same_components(&self, other: &ComponentPatch) -> bool {
            if self == other {
                return true;
            }
            let mut a: Vec<i32> = self.removed().collect();
            let mut b: Vec<i32> = other.removed().collect();
            a.sort_unstable();
            a.dedup();
            b.sort_unstable();
            b.dedup();
            if a != b {
                return false;
            }
            let last = |p: &ComponentPatch| {
                let mut v: Vec<(i32, Bytes)> = Vec::new();
                for (t, b) in p.iter() {
                    match v.iter_mut().find(|(k, _)| *k == t) {
                        Some(e) => e.1 = b,
                        None => v.push((t, b)),
                    }
                }
                v.sort_by_key(|(t, _)| *t);
                v
            };
            last(self) == last(other)
        }

        /// Builds a patch from `(type, value bytes)` components and removed types (tests,
        /// creative clients). Type ids of protocol 775.
        #[must_use]
        pub fn from_parts(added: &[(i32, &[u8])], removed: &[i32]) -> ComponentPatch {
            Self::from_parts_with(Codec::P775, added, removed)
        }

        /// [`from_parts`](Self::from_parts) with type ids of protocol 776 (26.2).
        #[must_use]
        pub fn from_parts_776(added: &[(i32, &[u8])], removed: &[i32]) -> ComponentPatch {
            Self::from_parts_with(Codec::P776, added, removed)
        }

        fn from_parts_with(
            codec: Codec,
            added: &[(i32, &[u8])],
            removed: &[i32],
        ) -> ComponentPatch {
            let mut w = BytesMut::new();
            for (t, v) in added {
                w.put_varint(*t);
                w.put_slice(v);
            }
            let split = w.len() as u32;
            for t in removed {
                w.put_varint(*t);
            }
            ComponentPatch {
                bytes: w.freeze(),
                split,
                added: added.len() as u16,
                removed: removed.len() as u16,
                added_mask: added.iter().fold(0, |m, (t, _)| m | bit(*t)),
                removed_mask: removed.iter().fold(0, |m, t| m | bit(*t)),
                codec,
            }
        }
    }

    /// Iterator over the added components of a [`ComponentPatch`].
    #[derive(Debug, Clone)]
    pub struct Components<'a> {
        bytes: Bytes,
        pos: usize,
        left: u16,
        codec: Codec,
        _p: std::marker::PhantomData<&'a ComponentPatch>,
    }

    impl Iterator for Components<'_> {
        type Item = (i32, Bytes);

        fn next(&mut self) -> Option<(i32, Bytes)> {
            if self.left == 0 {
                return None;
            }
            self.left -= 1;
            let mut r = Reader::new(&self.bytes);
            r.skip(self.pos).ok()?;
            let mut value = r.clone();
            let ty = value.read_varint().ok()?;
            let value_start = value.position();
            skip_one(self.codec, &mut r).ok()?;
            self.pos = r.position();
            Some((ty, self.bytes.slice(value_start..self.pos)))
        }
    }

    /// A non-empty 26.1 item stack.
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub struct ItemStack {
        /// Item registry id.
        pub id: i32,
        /// Stack size (`> 0`).
        pub count: i32,
        /// Data component patch against the item's defaults.
        pub components: ComponentPatch,
    }

    impl ItemStack {
        /// Reads one slot; `None` for an empty one (count `<= 0`, as vanilla reads it).
        #[cfg(feature = "v26_1")]
        pub fn read(r: &mut Reader<'_>) -> DecodeResult<Option<ItemStack>> {
            Self::read_with(Codec::P775, r)
        }

        /// [`read`](Self::read) for protocol 776 (26.2).
        #[cfg(feature = "v26_2")]
        pub fn read_776(r: &mut Reader<'_>) -> DecodeResult<Option<ItemStack>> {
            Self::read_with(Codec::P776, r)
        }

        #[inline]
        fn read_with(codec: Codec, r: &mut Reader<'_>) -> DecodeResult<Option<ItemStack>> {
            let count = r.read_varint()?;
            if count <= 0 {
                return Ok(None);
            }
            let id = r.read_varint()?;
            let added = r.read_varint()?;
            let removed = r.read_varint()?;
            let components = ComponentPatch::read_with(codec, r, added, removed)?;
            Ok(Some(ItemStack {
                id,
                count,
                components,
            }))
        }

        /// Writes one slot (`None` = empty).
        pub fn write(s: Option<&ItemStack>, w: &mut BytesMut) {
            match s {
                None => w.put_varint(0),
                Some(s) => {
                    w.put_varint(s.count);
                    w.put_varint(s.id);
                    w.put_varint(i32::from(s.components.added));
                    w.put_varint(i32::from(s.components.removed));
                    w.put_slice(&s.components.bytes);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{Encode, decode_exact};
    use bytes::{Bytes, BytesMut};

    #[test]
    fn legacy_known_answers() {
        // Empty slot.
        let b = Bytes::from_static(&[0xff, 0xff]);
        assert_eq!(decode_exact::<legacy::Slot>(&b), Ok(legacy::Slot::EMPTY));
        // 64 x stone (id 1), damage 0, no NBT.
        let b = Bytes::from_static(&[0x00, 0x01, 0x40, 0x00, 0x00, 0x00]);
        let s = decode_exact::<legacy::Slot>(&b).unwrap();
        let it = s.item().unwrap();
        assert_eq!(
            (it.id, it.count, it.damage, it.nbt.is_none()),
            (1, 64, 0, true)
        );
        let mut w = BytesMut::new();
        s.encode(&mut w).unwrap();
        assert_eq!(&w[..], &b[..]);
        // With an empty-named empty compound as NBT.
        let b = Bytes::from_static(&[0x01, 0x14, 0x01, 0x00, 0x05, 0x0a, 0x00, 0x00, 0x00]);
        let s = decode_exact::<legacy::Slot>(&b).unwrap();
        assert_eq!(s.item().unwrap().nbt.as_ref().unwrap().as_bytes().len(), 4);
    }

    #[test]
    fn modern_known_answers() {
        let b = Bytes::from_static(&[0x00]);
        assert_eq!(decode_exact::<modern::Slot>(&b), Ok(modern::Slot::EMPTY));
        let b = Bytes::from_static(&[0x01, 0xac, 0x02, 0x10, 0x00]);
        let s = decode_exact::<modern::Slot>(&b).unwrap();
        let it = s.item().unwrap();
        assert_eq!((it.id, it.count), (300, 16));
    }
}
