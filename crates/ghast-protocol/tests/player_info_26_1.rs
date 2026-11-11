//! 26.1 `player_info` (`ClientboundPlayerInfoUpdatePacket`): the action bits follow vanilla's
//! `Action` ordinals (... 5 `UPDATE_DISPLAY_NAME`, 6 `UPDATE_LIST_ORDER` (VarInt),
//! 7 `UPDATE_HAT` (bool)), entries are read in ordinal order, and `UPDATE_LISTED` is a bool.
//! minecraft-data 3.119.0 swaps bits 6 and 7 and reads `listed` as a VarInt (patched in
//! codegen). Bytes built by hand from the vanilla codec.
#![cfg(feature = "v26_1")]

use bytes::{BufMut, Bytes, BytesMut};
use ghast_protocol::v26_1::play::clientbound::{PlayerInfo, PlayerInfoAction, id};
use ghast_protocol::{Encode, RawPacket};

const UUID: [u8; 16] = [7; 16];

fn packet(action: u8, entry: &[u8]) -> RawPacket {
    let mut b = BytesMut::new();
    b.put_u8(action); // EnumSet as a fixed bit set of 8 actions: one byte
    b.put_u8(1); // one entry
    b.put_slice(&UUID);
    b.put_slice(entry);
    RawPacket::new(id::PLAYER_INFO, b.freeze())
}

fn decode(action: u8, entry: &[u8]) -> PlayerInfo {
    let raw = packet(action, entry);
    let p: PlayerInfo = raw.decode_as().expect("decodes");
    // and writes the same bytes back
    let mut w = BytesMut::new();
    p.encode(&mut w).unwrap();
    assert_eq!(Bytes::from(w), raw.body);
    p
}

#[test]
fn list_order_is_bit_6_and_a_varint() {
    // UPDATE_LIST_ORDER only: VarInt 300 (two bytes); with the bits swapped this would read a
    // bool and leave a byte over
    let p = decode(1 << 6, &[0xAC, 0x02]);
    assert_eq!(p.action, PlayerInfoAction::UPDATE_LIST_ORDER);
    assert_eq!(p.data[0].list_priority, Some(300));
    assert_eq!(p.data[0].show_hat, None);
}

#[test]
fn hat_is_bit_7_and_a_bool() {
    let p = decode(1 << 7, &[1]);
    assert_eq!(p.action, PlayerInfoAction::UPDATE_HAT);
    assert_eq!(p.data[0].show_hat, Some(true));
    assert_eq!(p.data[0].list_priority, None);
}

#[test]
fn list_order_is_read_before_hat() {
    // both: entries in ordinal order, VarInt 200 (two bytes) then the hat bool
    let p = decode((1 << 6) | (1 << 7), &[0xC8, 0x01, 0]);
    assert_eq!(p.data[0].list_priority, Some(200));
    assert_eq!(p.data[0].show_hat, Some(false));
}

#[test]
fn listed_is_a_bool() {
    let p = decode(1 << 3, &[1]);
    assert_eq!(p.data[0].listed, Some(true));
}
