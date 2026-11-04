//! Error types.
//!
//! Decoding works on untrusted bytes from the network, so every failure is a value of
//! [`DecodeError`]; nothing in the decode path panics.

use thiserror::Error;

/// Failure while decoding a value from bytes.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum DecodeError {
    /// The input ended before the value was complete.
    #[error("unexpected end of input: needed {needed} more byte(s)")]
    UnexpectedEof {
        /// How many more bytes the reader wanted (at least).
        needed: usize,
    },
    /// A VarInt was longer than 5 bytes (or a VarLong longer than 10).
    #[error("VarInt/VarLong is too long")]
    VarIntTooLong,
    /// A length prefix was negative.
    #[error("negative length {0}")]
    NegativeLength(i64),
    /// A length prefix was larger than the remaining input or a configured limit.
    #[error("length {len} exceeds limit {limit}")]
    LengthTooLarge {
        /// The declared length.
        len: usize,
        /// The limit it exceeded.
        limit: usize,
    },
    /// A string was not valid UTF-8 (or NBT Modified UTF-8).
    #[error("invalid UTF-8 in string")]
    InvalidUtf8,
    /// An NBT tag id was not a known tag type.
    #[error("invalid NBT tag type {0}")]
    InvalidNbtTag(u8),
    /// NBT nested deeper than the allowed limit.
    #[error("NBT nesting deeper than {0}")]
    NbtTooDeep(usize),
    /// An NBT list declared elements of type End with a non-zero length.
    #[error("NBT list of TAG_End with non-zero length")]
    NbtEndList,
    /// A value that is not allowed by the format (e.g. an empty top-bit-terminated array).
    #[error("invalid value: {0}")]
    Invalid(&'static str),
    /// Bytes were left over after a packet body was decoded.
    #[error("{0} trailing byte(s) after packet")]
    TrailingBytes(usize),
    /// The packet id is not known for this state/direction/version.
    #[error("unknown packet id 0x{0:02x}")]
    UnknownPacketId(i32),
    /// A typed decode was requested for a packet with a different id.
    #[error("packet id mismatch: expected 0x{expected:02x}, got 0x{actual:02x}")]
    IdMismatch {
        /// Id of the requested packet type.
        expected: i32,
        /// Id of the raw packet.
        actual: i32,
    },
}

/// Failure while encoding a value.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum EncodeError {
    /// A length does not fit the length prefix type (e.g. more than `i16::MAX` items).
    #[error("length {len} does not fit the length prefix (max {max})")]
    LengthOverflow {
        /// The actual length.
        len: usize,
        /// The largest length the prefix can carry.
        max: usize,
    },
    /// A fixed-length array or a field-counted array has the wrong number of elements.
    #[error("field `{field}`: expected {expected} element(s), got {actual}")]
    CountMismatch {
        /// Field path.
        field: &'static str,
        /// Count required by the format or by the counting field.
        expected: usize,
        /// Actual element count.
        actual: usize,
    },
    /// A conditional field does not match its discriminant (e.g. `Some` where the
    /// discriminant says the field is absent, or the wrong enum variant).
    #[error("field `{0}` does not match its discriminant")]
    SwitchMismatch(&'static str),
    /// A value does not fit its bit field or packed representation.
    #[error("field `{0}` is out of range for its encoding")]
    OutOfRange(&'static str),
    /// A value that the format cannot represent.
    #[error("invalid value: {0}")]
    Invalid(&'static str),
}

/// Failure in the framing layer (length prefix, compression, encryption).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FrameError {
    /// The frame length prefix is malformed (more than 3 bytes, negative, or zero).
    #[error("bad frame length prefix")]
    BadLength,
    /// The frame is larger than the configured maximum.
    #[error("frame of {len} bytes exceeds maximum {max}")]
    FrameTooLarge {
        /// Declared frame length.
        len: usize,
        /// Configured limit.
        max: usize,
    },
    /// The declared uncompressed size is larger than the configured maximum.
    #[error("decompressed size {len} exceeds maximum {max}")]
    DecompressedTooLarge {
        /// Declared uncompressed length.
        len: usize,
        /// Configured limit.
        max: usize,
    },
    /// The zlib stream was corrupt or did not inflate to exactly the declared size.
    #[error("bad compressed packet: {0}")]
    BadCompression(&'static str),
    /// The frame contents could not be decoded (packet id or data-length VarInt).
    #[error(transparent)]
    Decode(#[from] DecodeError),
    /// I/O error from the underlying transport.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
