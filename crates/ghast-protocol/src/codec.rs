//! Low-level reading and writing of protocol primitives.
//!
//! [`Reader`] is a bounds-checked cursor over a [`Bytes`] buffer: every read returns
//! `Result`, and variable-length data ([`Reader::read_bytes`], strings) is returned as
//! zero-copy slices of the underlying buffer. [`McWrite`] adds the protocol's primitive
//! encodings to [`BytesMut`].
//!
//! The [`Decode`] / [`Encode`] traits are implemented by every packet and shared type.

use bytes::{BufMut, Bytes, BytesMut};
use bytestring::ByteString;
use uuid::Uuid;

use crate::error::{DecodeError, EncodeError};

/// Result alias for decoding.
pub type DecodeResult<T> = Result<T, DecodeError>;
/// Result alias for encoding.
pub type EncodeResult = Result<(), EncodeError>;

/// A value that can be decoded from protocol bytes.
pub trait Decode: Sized {
    /// Decodes one value, advancing the reader past it.
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self>;
}

/// A value that can be encoded to protocol bytes.
pub trait Encode {
    /// Appends the encoding of `self` to `w`.
    fn encode(&self, w: &mut BytesMut) -> EncodeResult;
}

/// Decodes a complete value from `buf`, failing if any bytes are left over.
pub fn decode_exact<T: Decode>(buf: &Bytes) -> DecodeResult<T> {
    let mut r = Reader::new(buf);
    let v = T::decode(&mut r)?;
    r.finish()?;
    Ok(v)
}

/// Encodes a value into a fresh buffer.
pub fn encode_to_bytes<T: Encode + ?Sized>(v: &T) -> Result<BytesMut, EncodeError> {
    let mut w = BytesMut::new();
    v.encode(&mut w)?;
    Ok(w)
}

/// Bounds-checked read cursor over a [`Bytes`] buffer.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    buf: &'a Bytes,
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Creates a reader at the start of `buf`.
    #[inline]
    pub fn new(buf: &'a Bytes) -> Self {
        Reader { buf, pos: 0 }
    }

    /// Number of unread bytes.
    #[inline]
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// Current offset from the start of the buffer.
    #[inline]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// The unread bytes, without consuming them.
    #[inline]
    pub fn rest_slice(&self) -> &'a [u8] {
        &self.buf[self.pos..]
    }

    /// Fails with [`DecodeError::TrailingBytes`] unless the input is fully consumed.
    #[inline]
    pub fn finish(&self) -> DecodeResult<()> {
        match self.remaining() {
            0 => Ok(()),
            n => Err(DecodeError::TrailingBytes(n)),
        }
    }

    #[inline]
    fn eof(&self, n: usize) -> DecodeError {
        DecodeError::UnexpectedEof {
            needed: n - self.remaining(),
        }
    }

    /// Consumes `n` bytes and returns them as a borrowed slice.
    #[inline]
    pub fn take(&mut self, n: usize) -> DecodeResult<&'a [u8]> {
        if n > self.remaining() {
            return Err(self.eof(n));
        }
        let buf: &'a Bytes = self.buf;
        let s = &buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    /// Advances past `n` bytes.
    #[inline]
    pub fn skip(&mut self, n: usize) -> DecodeResult<()> {
        if n > self.remaining() {
            return Err(self.eof(n));
        }
        self.pos += n;
        Ok(())
    }

    #[inline]
    fn take_array<const N: usize>(&mut self) -> DecodeResult<[u8; N]> {
        let s = self.take(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        Ok(a)
    }

    /// Consumes `n` bytes and returns them as a zero-copy [`Bytes`] slice.
    #[inline]
    pub fn read_bytes(&mut self, n: usize) -> DecodeResult<Bytes> {
        if n > self.remaining() {
            return Err(self.eof(n));
        }
        let b = self.buf.slice(self.pos..self.pos + n);
        self.pos += n;
        Ok(b)
    }

    /// Zero-copy slice of everything read since offset `start` (from [`Reader::position`]).
    #[inline]
    pub fn bytes_since(&self, start: usize) -> Bytes {
        self.buf.slice(start.min(self.pos)..self.pos)
    }

    /// Consumes and returns everything that is left (protodef `restBuffer`).
    #[inline]
    pub fn read_rest(&mut self) -> Bytes {
        let b = self.buf.slice(self.pos..);
        self.pos = self.buf.len();
        b
    }

    /// Returns the next byte without consuming it.
    #[inline]
    pub fn peek_u8(&self) -> DecodeResult<u8> {
        self.buf
            .get(self.pos)
            .copied()
            .ok_or(DecodeError::UnexpectedEof { needed: 1 })
    }

    /// Reads an unsigned byte.
    #[inline]
    pub fn read_u8(&mut self) -> DecodeResult<u8> {
        let b = self.peek_u8()?;
        self.pos += 1;
        Ok(b)
    }

    /// Reads a signed byte.
    #[inline]
    pub fn read_i8(&mut self) -> DecodeResult<i8> {
        Ok(self.read_u8()? as i8)
    }

    /// Reads a boolean. Like vanilla, any non-zero byte is `true`.
    #[inline]
    pub fn read_bool(&mut self) -> DecodeResult<bool> {
        Ok(self.read_u8()? != 0)
    }

    /// Reads a big-endian `u16`.
    #[inline]
    pub fn read_u16(&mut self) -> DecodeResult<u16> {
        Ok(u16::from_be_bytes(self.take_array()?))
    }

    /// Reads a big-endian `i16`.
    #[inline]
    pub fn read_i16(&mut self) -> DecodeResult<i16> {
        Ok(i16::from_be_bytes(self.take_array()?))
    }

    /// Reads a big-endian `i32`.
    #[inline]
    pub fn read_i32(&mut self) -> DecodeResult<i32> {
        Ok(i32::from_be_bytes(self.take_array()?))
    }

    /// Reads a big-endian `i64`.
    #[inline]
    pub fn read_i64(&mut self) -> DecodeResult<i64> {
        Ok(i64::from_be_bytes(self.take_array()?))
    }

    /// Reads a big-endian `u32`.
    #[inline]
    pub fn read_u32(&mut self) -> DecodeResult<u32> {
        Ok(u32::from_be_bytes(self.take_array()?))
    }

    /// Reads a big-endian `u64`.
    #[inline]
    pub fn read_u64(&mut self) -> DecodeResult<u64> {
        Ok(u64::from_be_bytes(self.take_array()?))
    }

    /// Reads a big-endian IEEE-754 `f32`.
    #[inline]
    pub fn read_f32(&mut self) -> DecodeResult<f32> {
        Ok(f32::from_be_bytes(self.take_array()?))
    }

    /// Reads a big-endian IEEE-754 `f64`.
    #[inline]
    pub fn read_f64(&mut self) -> DecodeResult<f64> {
        Ok(f64::from_be_bytes(self.take_array()?))
    }

    /// Reads a UUID (two big-endian 64-bit halves).
    #[inline]
    pub fn read_uuid(&mut self) -> DecodeResult<Uuid> {
        Ok(Uuid::from_u128(u128::from_be_bytes(self.take_array()?)))
    }

    /// Reads a VarInt (at most 5 bytes).
    ///
    /// Like vanilla, the unused high bits of a 5th byte are ignored; a 6th byte is an
    /// error ([`DecodeError::VarIntTooLong`]).
    #[inline]
    pub fn read_varint(&mut self) -> DecodeResult<i32> {
        let rest = &self.buf[self.pos..];
        if let Some(&b) = rest.first()
            && b < 0x80
        {
            self.pos += 1;
            return Ok(i32::from(b));
        }
        let (v, n) = varint_slow(rest)?;
        self.pos += n;
        Ok(v)
    }

    /// Reads a VarLong (at most 10 bytes).
    #[inline]
    pub fn read_varlong(&mut self) -> DecodeResult<i64> {
        let rest = &self.buf[self.pos..];
        let mut result: u64 = 0;
        for i in 0..10 {
            let Some(&b) = rest.get(i) else {
                return Err(DecodeError::UnexpectedEof { needed: 1 });
            };
            result |= u64::from(b & 0x7f) << (7 * i);
            if b & 0x80 == 0 {
                self.pos += i + 1;
                return Ok(result as i64);
            }
        }
        Err(DecodeError::VarIntTooLong)
    }

    /// Reads a non-negative length of type `i32` and checks it against `limit`.
    #[inline]
    fn check_len(len: i64, limit: usize) -> DecodeResult<usize> {
        if len < 0 {
            return Err(DecodeError::NegativeLength(len));
        }
        let len = len as usize;
        if len > limit {
            return Err(DecodeError::LengthTooLarge { len, limit });
        }
        Ok(len)
    }

    /// Reads a VarInt length prefix and checks it is non-negative and at most `limit`.
    #[inline]
    pub fn read_len_varint(&mut self, limit: usize) -> DecodeResult<usize> {
        let n = self.read_varint()?;
        Self::check_len(i64::from(n), limit)
    }

    /// Reads an array count of the given prefix width and validates it.
    ///
    /// `min_elem_size` is the smallest number of bytes one element can occupy; when it is
    /// non-zero the count is bounded by the remaining input, so a hostile count can never
    /// cause a large allocation or a long loop.
    #[inline]
    pub fn check_count(&self, count: i64, min_elem_size: usize) -> DecodeResult<usize> {
        let limit = self
            .remaining()
            .checked_div(min_elem_size)
            .unwrap_or(ZERO_SIZE_ELEMENT_LIMIT);
        Self::check_len(count, limit)
    }

    /// Reads a VarInt-prefixed byte array as a zero-copy slice.
    #[inline]
    pub fn read_byte_array(&mut self) -> DecodeResult<Bytes> {
        let n = self.read_len_varint(self.remaining())?;
        self.read_bytes(n)
    }

    /// Reads a VarInt-prefixed UTF-8 string as a zero-copy [`ByteString`].
    #[inline]
    pub fn read_string(&mut self) -> DecodeResult<ByteString> {
        let n = self.read_len_varint(self.remaining())?;
        let b = self.read_bytes(n)?;
        ByteString::try_from(b).map_err(|_| DecodeError::InvalidUtf8)
    }
}

/// Largest count accepted for arrays whose elements can be zero bytes long.
pub const ZERO_SIZE_ELEMENT_LIMIT: usize = 1 << 16;

#[inline(never)]
fn varint_slow(rest: &[u8]) -> DecodeResult<(i32, usize)> {
    let mut result: u32 = 0;
    for i in 0..5 {
        let Some(&b) = rest.get(i) else {
            return Err(DecodeError::UnexpectedEof { needed: 1 });
        };
        result |= u32::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            return Ok((result as i32, i + 1));
        }
    }
    Err(DecodeError::VarIntTooLong)
}

/// Decodes a VarInt from the start of a plain slice. Returns the value and its length.
///
/// Returns `Ok(None)` when the slice ends before the VarInt does.
#[inline]
pub fn read_varint_prefix(buf: &[u8]) -> DecodeResult<Option<(i32, usize)>> {
    match varint_slow(buf) {
        Ok(v) => Ok(Some(v)),
        Err(DecodeError::UnexpectedEof { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Number of bytes the VarInt encoding of `v` takes (1..=5).
#[inline]
pub const fn varint_len(v: i32) -> usize {
    let v = v as u32;
    match v {
        0..=0x7f => 1,
        0x80..=0x3fff => 2,
        0x4000..=0x1f_ffff => 3,
        0x20_0000..=0x0fff_ffff => 4,
        _ => 5,
    }
}

/// Encodes `v` as a VarInt into a fixed buffer; returns the buffer and the used length.
#[inline]
pub const fn varint_bytes(v: i32) -> ([u8; 5], usize) {
    let mut v = v as u32;
    let mut out = [0u8; 5];
    let mut i = 0;
    loop {
        if v < 0x80 {
            out[i] = v as u8;
            return (out, i + 1);
        }
        out[i] = (v as u8 & 0x7f) | 0x80;
        v >>= 7;
        i += 1;
    }
}

/// Protocol primitive encodings on top of [`BytesMut`].
pub trait McWrite {
    /// Writes a VarInt.
    fn put_varint(&mut self, v: i32);
    /// Writes a VarLong.
    fn put_varlong(&mut self, v: i64);
    /// Writes a boolean as one byte (0 or 1).
    fn put_bool(&mut self, v: bool);
    /// Writes a UUID as 16 big-endian bytes.
    fn put_uuid(&mut self, v: &Uuid);
    /// Writes a VarInt length prefix.
    fn put_len_varint(&mut self, len: usize) -> EncodeResult;
    /// Writes a VarInt-prefixed byte array.
    fn put_byte_array(&mut self, b: &[u8]) -> EncodeResult;
    /// Writes a VarInt-prefixed UTF-8 string.
    fn put_string(&mut self, s: &str) -> EncodeResult;
}

impl McWrite for BytesMut {
    #[inline]
    fn put_varint(&mut self, v: i32) {
        if (0..0x80).contains(&v) {
            self.put_u8(v as u8);
        } else {
            let (b, n) = varint_bytes(v);
            self.put_slice(&b[..n]);
        }
    }

    #[inline]
    fn put_varlong(&mut self, v: i64) {
        let mut v = v as u64;
        loop {
            if v < 0x80 {
                self.put_u8(v as u8);
                return;
            }
            self.put_u8((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
    }

    #[inline]
    fn put_bool(&mut self, v: bool) {
        self.put_u8(u8::from(v));
    }

    #[inline]
    fn put_uuid(&mut self, v: &Uuid) {
        self.put_u128(v.as_u128());
    }

    #[inline]
    fn put_len_varint(&mut self, len: usize) -> EncodeResult {
        let v = i32::try_from(len).map_err(|_| EncodeError::LengthOverflow {
            len,
            max: i32::MAX as usize,
        })?;
        self.put_varint(v);
        Ok(())
    }

    #[inline]
    fn put_byte_array(&mut self, b: &[u8]) -> EncodeResult {
        self.put_len_varint(b.len())?;
        self.put_slice(b);
        Ok(())
    }

    #[inline]
    fn put_string(&mut self, s: &str) -> EncodeResult {
        self.put_byte_array(s.as_bytes())
    }
}

/// Upper bound on the capacity reserved up front for a decoded array; larger arrays
/// grow as elements are actually decoded, so a hostile count cannot force a big
/// allocation.
pub const PREALLOC_LIMIT: usize = 4096;

fn overflow(len: usize, max: usize) -> EncodeError {
    EncodeError::LengthOverflow { len, max }
}

/// Converts a collection length into an `i16` count prefix.
#[inline]
pub fn count_i16(len: usize) -> Result<i16, EncodeError> {
    i16::try_from(len).map_err(|_| overflow(len, i16::MAX as usize))
}

/// Converts a collection length into an `i32` count prefix.
#[inline]
pub fn count_i32(len: usize) -> Result<i32, EncodeError> {
    i32::try_from(len).map_err(|_| overflow(len, i32::MAX as usize))
}

/// Converts a collection length into a `u8` count prefix.
#[inline]
pub fn count_u8(len: usize) -> Result<u8, EncodeError> {
    u8::try_from(len).map_err(|_| overflow(len, u8::MAX as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rd(bytes: &[u8]) -> DecodeResult<i32> {
        let b = Bytes::copy_from_slice(bytes);
        let mut r = Reader::new(&b);
        let v = r.read_varint()?;
        r.finish()?;
        Ok(v)
    }

    #[test]
    fn varint_known_answers() {
        let cases: &[(i32, &[u8])] = &[
            (0, &[0x00]),
            (1, &[0x01]),
            (2, &[0x02]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (255, &[0xff, 0x01]),
            (25565, &[0xdd, 0xc7, 0x01]),
            (2097151, &[0xff, 0xff, 0x7f]),
            (2147483647, &[0xff, 0xff, 0xff, 0xff, 0x07]),
            (-1, &[0xff, 0xff, 0xff, 0xff, 0x0f]),
            (-2147483648, &[0x80, 0x80, 0x80, 0x80, 0x08]),
        ];
        for &(v, bytes) in cases {
            let mut w = BytesMut::new();
            w.put_varint(v);
            assert_eq!(&w[..], bytes, "encode {v}");
            assert_eq!(varint_len(v), bytes.len());
            assert_eq!(rd(bytes), Ok(v), "decode {bytes:02x?}");
        }
    }

    #[test]
    fn varint_rejects_too_long_and_eof() {
        assert_eq!(
            rd(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01]),
            Err(DecodeError::VarIntTooLong)
        );
        assert!(matches!(
            rd(&[0x80]),
            Err(DecodeError::UnexpectedEof { .. })
        ));
        assert!(matches!(rd(&[]), Err(DecodeError::UnexpectedEof { .. })));
        // Non-canonical (padded) encodings are accepted, like vanilla.
        assert_eq!(rd(&[0x80, 0x00]), Ok(0));
        assert_eq!(read_varint_prefix(&[0xff, 0xff]), Ok(None));
    }

    #[test]
    fn varlong_known_answers() {
        let cases: &[(i64, &[u8])] = &[
            (0, &[0x00]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (2147483647, &[0xff, 0xff, 0xff, 0xff, 0x07]),
            (
                9223372036854775807,
                &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f],
            ),
            (
                -1,
                &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01],
            ),
            (
                -9223372036854775808,
                &[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01],
            ),
        ];
        for &(v, bytes) in cases {
            let mut w = BytesMut::new();
            w.put_varlong(v);
            assert_eq!(&w[..], bytes, "encode {v}");
            let b = Bytes::copy_from_slice(bytes);
            let mut r = Reader::new(&b);
            assert_eq!(r.read_varlong(), Ok(v));
            assert_eq!(r.remaining(), 0);
        }
        let b = Bytes::from_static(&[0x80; 11]);
        assert_eq!(
            Reader::new(&b).read_varlong(),
            Err(DecodeError::VarIntTooLong)
        );
    }

    #[test]
    fn strings_are_bounded_and_validated() {
        let b = Bytes::from_static(&[0x05, b'h', b'i']);
        assert!(matches!(
            Reader::new(&b).read_string(),
            Err(DecodeError::LengthTooLarge { .. })
        ));
        let b = Bytes::from_static(&[0x02, 0xc3, 0x28]);
        assert_eq!(Reader::new(&b).read_string(), Err(DecodeError::InvalidUtf8));
        let b = Bytes::from_static(&[0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert_eq!(
            Reader::new(&b).read_string(),
            Err(DecodeError::NegativeLength(-1))
        );
        let b = Bytes::from_static(&[0x03, b'a', b'b', b'c', 0x00]);
        let mut r = Reader::new(&b);
        assert_eq!(&*r.read_string().unwrap(), "abc");
        assert_eq!(r.finish(), Err(DecodeError::TrailingBytes(1)));
    }

    #[test]
    fn fixed_width_known_answers() {
        let b = Bytes::from_static(&[
            0x12, 0x34, 0xff, 0xfe, 0x3f, 0x80, 0x00, 0x00, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55,
            0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
        ]);
        let mut r = Reader::new(&b);
        assert_eq!(r.read_u16(), Ok(0x1234));
        assert_eq!(r.read_i16(), Ok(-2));
        assert_eq!(r.read_f32(), Ok(1.0));
        assert_eq!(
            r.read_uuid().unwrap().to_string(),
            "00112233-4455-6677-8899-aabbccddeeff"
        );
        assert_eq!(r.remaining(), 0);
    }
}
