//! Parsing helpers shared by the version modules.

use bytes::Bytes;
use ghast_protocol::nbt::RawNbt;
use ghast_protocol::{DecodeError, Reader};

use crate::entity::Item;
use crate::store::{Entities, Event, PlayerInfo};

pub(crate) type R<T> = Result<T, DecodeError>;

/// Vanilla's angle byte to degrees: `(float)(b * 360) / 256.0F`.
#[inline]
pub(crate) fn angle(b: i8) -> f32 {
    (i32::from(b) * 360) as f32 / 256.0
}

/// Copies NBT out of the packet buffer (so a stored item never pins a packet's memory).
pub(crate) fn own_nbt(n: Option<RawNbt>) -> Option<Bytes> {
    n.map(|n| Bytes::copy_from_slice(n.as_bytes()))
}

pub(crate) fn legacy_item(s: ghast_protocol::types::slot::legacy::Slot) -> Option<Item> {
    s.0.map(|i| Item {
        id: i32::from(i.id),
        count: i.count,
        damage: i.damage,
        nbt: own_nbt(i.nbt),
    })
}

pub(crate) fn modern_item(s: ghast_protocol::types::slot::modern::Slot) -> Option<Item> {
    s.0.map(|i| Item {
        id: i.id,
        count: i.count,
        damage: 0,
        nbt: own_nbt(i.nbt),
    })
}

/// A string copied out of the packet.
pub(crate) fn own_str(r: &mut Reader<'_>) -> R<Box<str>> {
    Ok(Box::from(&*r.read_string()?))
}

/// `player_info` (same layout in 1.8 and 1.16.5). Profile properties (skins) are skipped
/// without allocating.
pub(crate) fn player_info(store: &mut Entities, r: &mut Reader<'_>) -> R<()> {
    let action = r.read_varint()?;
    let n = r.read_varint()?;
    let n = r.check_count(i64::from(n), 16)?;
    for _ in 0..n {
        let uuid = r.read_uuid()?;
        match action {
            0 => {
                let name = own_str(r)?;
                let props = r.read_varint()?;
                let props = r.check_count(i64::from(props), 3)?;
                let mut textures = None;
                for _ in 0..props {
                    if store.config.skins {
                        let key = r.read_string()?;
                        let value = r.read_string()?;
                        if &*key == "textures" {
                            textures = Some(Box::<str>::from(&*value));
                        }
                    } else {
                        skip_string(r)?;
                        skip_string(r)?;
                    }
                    if r.read_bool()? {
                        skip_string(r)?;
                    }
                }
                let game_mode = r.read_varint()?;
                let ping = r.read_varint()?;
                let display_name = if r.read_bool()? {
                    Some(own_str(r)?)
                } else {
                    None
                };
                store.players.insert(
                    uuid,
                    PlayerInfo {
                        name,
                        game_mode,
                        ping,
                        display_name,
                        textures,
                    },
                );
            }
            1 => {
                let g = r.read_varint()?;
                if let Some(p) = store.players.get_mut(&uuid) {
                    p.game_mode = g;
                }
            }
            2 => {
                let ping = r.read_varint()?;
                if let Some(p) = store.players.get_mut(&uuid) {
                    p.ping = ping;
                }
            }
            3 => {
                let d = if r.read_bool()? {
                    Some(own_str(r)?)
                } else {
                    None
                };
                if let Some(p) = store.players.get_mut(&uuid) {
                    p.display_name = d;
                }
            }
            4 => {
                store.players.remove(&uuid);
            }
            _ => return Err(DecodeError::Invalid("unknown player_info action")),
        }
    }
    store.event(Event::PlayerList);
    Ok(())
}

pub(crate) fn skip_string(r: &mut Reader<'_>) -> R<()> {
    let n = r.read_len_varint(r.remaining())?;
    r.skip(n)
}

/// Reads a VarInt-counted list of VarInt ids, calling `f` for each (no allocation).
pub(crate) fn for_each_varint(r: &mut Reader<'_>, mut f: impl FnMut(i32)) -> R<()> {
    let n = r.read_varint()?;
    let n = r.check_count(i64::from(n), 1)?;
    for _ in 0..n {
        f(r.read_varint()?);
    }
    Ok(())
}

/// A set of packet ids `0..256`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct IdMask([u64; 4]);

impl IdMask {
    pub(crate) fn add(&mut self, ids: &[i32]) {
        for &id in ids {
            if (0..256).contains(&id) {
                self.0[(id >> 6) as usize] |= 1 << (id & 63);
            }
        }
    }

    #[inline]
    pub(crate) fn contains(&self, id: i32) -> bool {
        (0..256).contains(&id) && self.0[(id >> 6) as usize] & (1 << (id & 63)) != 0
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = i32> + '_ {
        (0..256).filter(|&id| self.contains(id))
    }
}
