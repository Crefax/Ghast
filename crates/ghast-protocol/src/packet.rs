//! Packet traits, raw packets and the per-id interest filter.

use bytes::{Bytes, BytesMut};

use crate::codec::{Decode, DecodeResult, Encode, EncodeResult, McWrite, decode_exact};
use crate::error::DecodeError;

/// Connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum State {
    /// The first packet of every connection.
    Handshaking,
    /// Server list ping.
    Status,
    /// Login (encryption, compression, login success).
    Login,
    /// Configuration (1.20.2+): registries, tags, resource packs, between login and play
    /// and again whenever the server sends `start_configuration`.
    Configuration,
    /// In game.
    Play,
}

/// Packet direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Server to client.
    Clientbound,
    /// Client to server.
    Serverbound,
}

/// A typed packet of one version, state and direction.
pub trait Packet: Encode + Decode {
    /// Packet id in its state and direction.
    const ID: i32;
    /// minecraft-data name (e.g. `"map_chunk"`).
    const NAME: &'static str;
    /// Connection state the packet belongs to.
    const STATE: State;
    /// Direction of the packet.
    const DIRECTION: Direction;
}

/// All packets of one version, state and direction as an enum (generated).
pub trait PacketSet: Sized {
    /// Connection state of this set.
    const STATE: State;
    /// Direction of this set.
    const DIRECTION: Direction;

    /// Decodes a raw packet into the matching variant (strict: no trailing bytes).
    fn decode_raw(raw: &RawPacket) -> DecodeResult<Self>;
    /// Packet id of this value.
    fn id(&self) -> i32;
    /// minecraft-data name of this value.
    fn name(&self) -> &'static str;
    /// Encodes the packet body (without the id).
    fn encode_body(&self, w: &mut BytesMut) -> EncodeResult;
    /// minecraft-data name for a packet id, if the id is known.
    fn name_of(id: i32) -> Option<&'static str>;
}

/// A packet whose body has not been decoded yet.
///
/// The body is a zero-copy slice of the received (and, if needed, inflated) frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPacket {
    /// Packet id.
    pub id: i32,
    /// Packet body after the id.
    pub body: Bytes,
}

impl RawPacket {
    /// Wraps an id and body.
    #[inline]
    pub fn new(id: i32, body: Bytes) -> Self {
        RawPacket { id, body }
    }

    /// Decodes the body as packet type `P`; checks the id and that the whole body is used.
    #[inline]
    pub fn decode_as<P: Packet>(&self) -> DecodeResult<P> {
        if self.id != P::ID {
            return Err(DecodeError::IdMismatch {
                expected: P::ID,
                actual: self.id,
            });
        }
        decode_exact(&self.body)
    }

    /// Encodes a typed packet into a raw packet.
    pub fn from_packet<P: Packet>(p: &P) -> Result<Self, crate::error::EncodeError> {
        let mut w = BytesMut::new();
        p.encode(&mut w)?;
        Ok(RawPacket::new(P::ID, w.freeze()))
    }
}

/// Writes `id` + body of a typed packet (the uncompressed, unframed packet data).
#[inline]
pub fn write_packet<P: Packet>(p: &P, w: &mut BytesMut) -> EncodeResult {
    w.put_varint(P::ID);
    p.encode(w)
}

/// Set of packet ids a client wants to receive.
///
/// The frame decoder drops packets whose id is not in the filter *before* decoding them
/// and — when compression is on — after inflating only the few bytes that hold the id.
/// Ids above 255 are never used by the supported versions and are always accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PacketFilter {
    bits: [u64; 4],
}

impl Default for PacketFilter {
    fn default() -> Self {
        Self::all()
    }
}

impl PacketFilter {
    /// Accepts every packet.
    pub const fn all() -> Self {
        PacketFilter {
            bits: [u64::MAX; 4],
        }
    }

    /// Accepts no packet (ids above 255 excepted).
    pub const fn none() -> Self {
        PacketFilter { bits: [0; 4] }
    }

    /// Returns the filter with `id` accepted.
    #[must_use]
    pub const fn with(mut self, id: i32) -> Self {
        if id >= 0 && id < 256 {
            self.bits[(id as usize) >> 6] |= 1 << (id & 63);
        }
        self
    }

    /// Returns the filter with `id` dropped.
    #[must_use]
    pub const fn without(mut self, id: i32) -> Self {
        if id >= 0 && id < 256 {
            self.bits[(id as usize) >> 6] &= !(1 << (id & 63));
        }
        self
    }

    /// Accepts `id`.
    pub fn insert(&mut self, id: i32) {
        *self = self.with(id);
    }

    /// Drops `id`.
    pub fn remove(&mut self, id: i32) {
        *self = self.without(id);
    }

    /// Whether packets with this id are wanted.
    #[inline]
    pub const fn contains(&self, id: i32) -> bool {
        if id < 0 || id >= 256 {
            return true;
        }
        self.bits[(id as usize) >> 6] & (1 << (id & 63)) != 0
    }

    /// Whether every id is accepted (lets the decoder skip the peek).
    #[inline]
    pub const fn is_all(&self) -> bool {
        self.bits[0] == u64::MAX
            && self.bits[1] == u64::MAX
            && self.bits[2] == u64::MAX
            && self.bits[3] == u64::MAX
    }
}

/// The packet set of a state a version does not have (the configuration state before
/// 1.20.2). Uninhabited: decoding always fails with [`DecodeError::UnknownPacketId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoPackets {}

impl PacketSet for NoPackets {
    const STATE: State = State::Configuration;
    const DIRECTION: Direction = Direction::Clientbound;

    fn decode_raw(raw: &RawPacket) -> DecodeResult<Self> {
        Err(DecodeError::UnknownPacketId(raw.id))
    }
    fn id(&self) -> i32 {
        match *self {}
    }
    fn name(&self) -> &'static str {
        match *self {}
    }
    fn encode_body(&self, _: &mut BytesMut) -> EncodeResult {
        match *self {}
    }
    fn name_of(_: i32) -> Option<&'static str> {
        None
    }
}

pub(crate) mod sealed {
    /// Seals [`super::Version`]: only this crate's generated markers implement it.
    #[allow(unnameable_types)] // unnameable on purpose: that is what seals the trait
    pub trait Sealed {}
}

/// A protocol version: its number and the packet sets of every state and direction.
///
/// Implemented by the generated marker types (e.g. [`crate::v1_8::V1_8`]). A
/// version-agnostic layer can be written generically over `V: Version` and map each
/// version's packet enums into common events. Sealed: new items can be added without
/// breaking anyone, so it cannot be implemented outside this crate.
pub trait Version: sealed::Sealed + Copy + Send + Sync + 'static {
    /// Protocol number sent in the handshake (47 for 1.8.x, 754 for 1.16.5).
    const PROTOCOL: i32;
    /// Human-readable version name (e.g. `"1.8.9"`).
    const NAME: &'static str;
    /// Bit layout of packed block positions.
    const POSITION_LAYOUT: crate::types::PositionLayout;
    /// This version's row in [`crate::versions::TABLE`].
    const INFO: &'static crate::versions::ProtocolVersion;

    /// Serverbound handshaking packets.
    type HandshakingServerbound: PacketSet;
    /// Clientbound status packets.
    type StatusClientbound: PacketSet;
    /// Serverbound status packets.
    type StatusServerbound: PacketSet;
    /// Clientbound login packets.
    type LoginClientbound: PacketSet;
    /// Serverbound login packets.
    type LoginServerbound: PacketSet;
    /// Clientbound configuration packets ([`NoPackets`] before 1.20.2).
    type ConfigurationClientbound: PacketSet;
    /// Serverbound configuration packets ([`NoPackets`] before 1.20.2).
    type ConfigurationServerbound: PacketSet;
    /// Clientbound play packets.
    type PlayClientbound: PacketSet;
    /// Serverbound play packets.
    type PlayServerbound: PacketSet;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter() {
        let f = PacketFilter::none().with(0x21).with(200);
        assert!(f.contains(0x21) && f.contains(200) && !f.contains(0x22));
        assert!(f.contains(300) && f.contains(-1));
        assert!(PacketFilter::all().is_all() && !f.is_all());
        assert!(!PacketFilter::all().without(5).contains(5));
    }
}
