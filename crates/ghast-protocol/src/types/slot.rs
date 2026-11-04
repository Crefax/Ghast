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
