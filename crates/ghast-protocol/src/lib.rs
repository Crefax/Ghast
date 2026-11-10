//! Minecraft Java Edition network protocol for the ghast client library.
//!
//! Layers, bottom to top:
//!
//! - [`codec`]: bounds-checked [`Reader`] over [`bytes::Bytes`] (zero-copy byte arrays
//!   and strings), [`McWrite`] for encoding, VarInt/VarLong.
//! - [`types`], [`nbt`], [`chat`]: hand-written shared types — [`BlockPos`] with the
//!   per-version bit layout, slots, lazily parsed NBT ([`nbt::RawNbt`]), chat JSON.
//! - Generated packet modules per protocol version (`v1_8` = protocol 47, `v1_12_2` =
//!   340, `v1_16_5` = 754, `v26_1` = 775), each with typed packets, one enum per state
//!   and direction, and id constants. The definitions themselves live once in [`defs`]
//!   and are shared by every version whose code for them is identical. A [`Version`]
//!   marker type ties a version's packet sets together so a version-agnostic layer can be
//!   written generically; [`versions`] is the table of known protocol numbers.
//! - [`frame`]: length framing, zlib compression with partial inflation, the
//!   AES-128-CFB8 hook ([`crypto`]), and the per-id [`PacketFilter`] that drops unwanted
//!   packets before they are decoded.
//! - [`io`] (feature `tokio`): async framed reader/writer.
//!
//! Decoding never panics on untrusted input: every failure is a [`DecodeError`] or
//! [`FrameError`], and allocations are bounded by the input size.
//!
//! ```
//! use ghast_protocol::{FrameDecoder, FrameEncoder, PacketSet, RawPacket};
//! use ghast_protocol::v1_8::play::serverbound::{KeepAlive, ServerboundPlayPacket};
//!
//! let mut enc = FrameEncoder::new();
//! enc.write_packet(&KeepAlive { keep_alive_id: 42 }).unwrap();
//! let mut dec = FrameDecoder::new();
//! dec.push(&enc.take());
//! let raw: RawPacket = dec.next_packet().unwrap().unwrap();
//! let p = ServerboundPlayPacket::decode_raw(&raw).unwrap();
//! assert_eq!(p, ServerboundPlayPacket::KeepAlive(KeepAlive { keep_alive_id: 42 }));
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, unnameable_types)]

pub mod chat;
pub mod codec;
pub mod crypto;
pub mod error;
pub mod frame;
#[cfg(feature = "tokio")]
pub mod io;
pub mod nbt;
pub mod packet;
#[cfg(feature = "testgen")]
pub mod testgen;
pub mod types;
pub mod versions;

#[path = "generated/defs/mod.rs"]
pub mod defs;

#[cfg(feature = "v1_8")]
#[path = "generated/v1_8/mod.rs"]
pub mod v1_8;

#[cfg(feature = "v1_12_2")]
#[path = "generated/v1_12_2/mod.rs"]
pub mod v1_12_2;

#[cfg(feature = "v1_16_5")]
#[path = "generated/v1_16_5/mod.rs"]
pub mod v1_16_5;

pub use codec::{Decode, Encode, McWrite, Reader};
pub use error::{DecodeError, EncodeError, FrameError};
pub use frame::{FrameDecoder, FrameEncoder, FrameLimits};
#[cfg(feature = "tokio")]
pub use io::{FramedReader, FramedWriter};
pub use packet::{
    Direction, NoPackets, Packet, PacketFilter, PacketSet, RawPacket, State, Version,
};
pub use versions::ProtocolVersion;
pub use types::BlockPos;
