//! Framing: length prefixes, zlib compression and the encryption hook.
//!
//! [`FrameDecoder`] turns received bytes into [`RawPacket`]s, [`FrameEncoder`] turns
//! packets into bytes ready to send. Both are synchronous and transport-agnostic; the
//! `tokio` feature wraps them in [`crate::io`].
//!
//! Wire format (vanilla): `VarInt length | data`, where without compression
//! `data = VarInt id | body` and with compression
//! `data = VarInt uncompressed_len | (id | body), zlib-compressed unless uncompressed_len == 0`.
//! With encryption on, the whole stream is AES-128-CFB8 encrypted.
//!
//! Lazy decoding: the decoder applies a [`PacketFilter`]. Unwanted packets are dropped
//! before anything is decoded; for compressed frames only the first few bytes (enough
//! for the packet id) are inflated, so ignored packets cost almost nothing.

use std::cell::RefCell;

use bytes::{Buf, BufMut, Bytes, BytesMut};
#[cfg(not(feature = "libdeflate"))]
use flate2::{Compress, Compression, FlushCompress, Status};
use flate2::{Decompress, FlushDecompress};

use crate::codec::{McWrite, Reader, read_varint_prefix, varint_len};
use crate::crypto::{Cfb8Decryptor, Cfb8Encryptor};
use crate::error::{DecodeError, EncodeError, FrameError};
use crate::packet::{Packet, PacketFilter, PacketSet, RawPacket};

/// Largest frame length expressible in the 3-byte length prefix vanilla accepts.
pub const MAX_FRAME_LEN: usize = (1 << 21) - 1;

/// Size of the receive buffer between bursts. A frame that does not fit grows it; once
/// [`SHRINK_AFTER`] frames in a row fit again, the buffered bytes move to a buffer of this size,
/// so a 2 MB chunk packet does not leave a 2 MB buffer behind (and a stream of large frames
/// does not reallocate for each).
const READ_BUF: usize = 8 * 1024;

/// Frames of at most [`READ_BUF`] in a row after which a grown receive buffer shrinks.
const SHRINK_AFTER: u16 = 64;

/// Least free space [`FrameDecoder::reserve_read`] leaves for one read.
const MIN_READ: usize = 2 * 1024;

// The (de)compressor states are scratch space used within one call and reset before each use,
// so one per thread serves every connection on it: a streaming inflater alone is ~47 KB, a
// level-6 compressor several hundred KB, which per connection would dominate a light client's
// memory when many share a process.
thread_local! {
    /// Streaming inflater: packet-id peeks (and whole packets without `libdeflate`).
    static INFLATER: RefCell<Decompress> = RefCell::new(Decompress::new(true));
}
#[cfg(feature = "libdeflate")]
thread_local! {
    static LIBDEFLATE_INFLATER: RefCell<libdeflater::Decompressor> =
        RefCell::new(libdeflater::Decompressor::new());
    // Level 6 like Java's `Deflater` default.
    static DEFLATER: RefCell<libdeflater::Compressor> =
        RefCell::new(libdeflater::Compressor::new(libdeflater::CompressionLvl::default()));
}
#[cfg(not(feature = "libdeflate"))]
thread_local! {
    // Level 6 like Java's `Deflater` default (also gives the same `78 9c` header).
    static DEFLATER: RefCell<Compress> = RefCell::new(Compress::new(Compression::default(), true));
}

/// Limits applied by the decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameLimits {
    /// Maximum frame length (at most [`MAX_FRAME_LEN`]).
    pub max_frame_len: usize,
    /// Maximum declared uncompressed packet size. Vanilla 1.8 uses 2 MiB, later
    /// versions 8 MiB; the default is 8 MiB.
    pub max_uncompressed_len: usize,
}

impl Default for FrameLimits {
    fn default() -> Self {
        FrameLimits {
            max_frame_len: MAX_FRAME_LEN,
            max_uncompressed_len: 8 << 20,
        }
    }
}

/// Decoder counters (useful for profiling what a server sends).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecoderStats {
    /// Frames split off the stream.
    pub frames: u64,
    /// Frames dropped by the filter.
    pub skipped: u64,
    /// Compressed frames dropped after inflating only the packet id.
    pub skipped_compressed: u64,
    /// Bytes produced by inflation.
    pub inflated_bytes: u64,
}

/// Splits a byte stream into packets.
pub struct FrameDecoder {
    buf: BytesMut,
    /// `buf` grew past [`READ_BUF`] for a frame: shrink it once frames fit again.
    grown: bool,
    /// Frames that fit [`READ_BUF`] split off since `buf` grew.
    fitting: u16,
    /// Copy uncompressed frames out of `buf` instead of sharing it.
    detach: bool,
    decryptor: Option<Cfb8Decryptor>,
    compression: bool,
    filter: PacketFilter,
    limits: FrameLimits,
    stats: DecoderStats,
}

impl std::fmt::Debug for FrameDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameDecoder")
            .field("buffered", &self.buf.len())
            .field("encrypted", &self.decryptor.is_some())
            .field("compression", &self.compression)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameDecoder {
    /// A decoder with no compression, no encryption, default limits and no filter.
    pub fn new() -> Self {
        FrameDecoder {
            buf: BytesMut::with_capacity(READ_BUF),
            grown: false,
            fitting: 0,
            detach: false,
            decryptor: None,
            compression: false,
            filter: PacketFilter::all(),
            limits: FrameLimits::default(),
            stats: DecoderStats::default(),
        }
    }

    /// Sets the limits.
    pub fn set_limits(&mut self, limits: FrameLimits) {
        self.limits = FrameLimits {
            max_frame_len: limits.max_frame_len.min(MAX_FRAME_LEN),
            ..limits
        };
    }

    /// Turns compression on or off (from the server's Set Compression packet; a
    /// negative threshold means off). Call it right after decoding that packet.
    pub fn set_compression(&mut self, threshold: i32) {
        self.compression = threshold >= 0;
    }

    /// Whether compression is on.
    pub fn compression(&self) -> bool {
        self.compression
    }

    /// Turns on decryption. Bytes already buffered but not yet split into frames are
    /// decrypted too, since the server encrypts everything after its encryption request
    /// was answered.
    pub fn enable_encryption(&mut self, secret: &[u8; 16]) {
        let mut d = Cfb8Decryptor::new(secret);
        d.decrypt(&mut self.buf);
        self.decryptor = Some(d);
    }

    /// Whether decryption is on.
    pub fn is_encrypted(&self) -> bool {
        self.decryptor.is_some()
    }

    /// Sets the per-id interest filter.
    pub fn set_filter(&mut self, filter: PacketFilter) {
        self.filter = filter;
    }

    /// The current filter.
    pub fn filter(&self) -> &PacketFilter {
        &self.filter
    }

    /// Counters.
    pub fn stats(&self) -> DecoderStats {
        self.stats
    }

    /// Number of buffered, not yet consumed bytes.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    /// Appends received bytes (decrypting them if encryption is on).
    pub fn push(&mut self, data: &[u8]) {
        let start = self.buf.len();
        self.buf.extend_from_slice(data);
        self.decrypt_from(start);
    }

    /// The receive buffer, for reading directly into it. After appending `n` bytes call
    /// [`FrameDecoder::commit`] with the length before the read.
    ///
    /// [`FrameDecoder::reserve_read`] also makes room for the read first.
    pub fn buffer_mut(&mut self) -> &mut BytesMut {
        &mut self.buf
    }

    /// The receive buffer with room for the next read: at least the rest of the frame being
    /// received, at least a couple of KB otherwise. The consumed front is reused in place when
    /// no packet still shares the buffer. After appending call [`FrameDecoder::commit`] with
    /// the length before the read, as with [`FrameDecoder::buffer_mut`].
    pub fn reserve_read(&mut self) -> &mut BytesMut {
        let need = self.needed().unwrap_or(0);
        let len = self.buf.len();
        if self.buf.capacity() - len < need.max(MIN_READ) {
            self.buf
                .reserve(need.max(READ_BUF.saturating_sub(len)).max(MIN_READ));
            self.note_growth();
        }
        &mut self.buf
    }

    /// Decrypts bytes appended to [`FrameDecoder::buffer_mut`] since `old_len`.
    pub fn commit(&mut self, old_len: usize) {
        self.decrypt_from(old_len);
    }

    /// Copies uncompressed frames out of the receive buffer instead of sharing it with the
    /// packets (off by default). A packet body that something keeps (a registry entry, a
    /// profile property) then does not keep a whole receive buffer alive. Meant for the short
    /// login and configuration states: it costs an allocation per frame.
    pub fn set_detach_frames(&mut self, on: bool) {
        self.detach = on;
    }

    /// Moves the buffered bytes to a fresh receive buffer of the standard size. Packets split
    /// off before keep the old buffer (only) to themselves; call it at a state change (the end
    /// of the configuration) so that the packets of the new state do not share a buffer with
    /// data kept from the old one.
    pub fn rehome(&mut self) {
        let mut fresh = BytesMut::with_capacity(READ_BUF.max(self.buf.len()));
        fresh.extend_from_slice(&self.buf);
        self.buf = fresh;
        self.grown = false;
    }

    /// After a reserve: the buffer is larger than the standard size (for a large frame).
    fn note_growth(&mut self) {
        if !self.grown && self.buf.capacity() > READ_BUF {
            self.grown = true;
            self.fitting = 0;
        }
    }

    fn decrypt_from(&mut self, start: usize) {
        if let Some(d) = &mut self.decryptor
            && start < self.buf.len()
        {
            d.decrypt(&mut self.buf[start..]);
        }
    }

    /// How many more bytes are needed to complete the next frame, if known.
    pub fn needed(&self) -> Option<usize> {
        match read_varint_prefix(&self.buf[..self.buf.len().min(3)]) {
            Ok(Some((len, n))) if len >= 0 => (n + len as usize).checked_sub(self.buf.len()),
            _ => None,
        }
    }

    /// Splits off the next packet that passes the filter.
    ///
    /// Returns `Ok(None)` when more bytes are needed. Errors are fatal for the
    /// connection (the stream position is lost).
    pub fn next_packet(&mut self) -> Result<Option<RawPacket>, FrameError> {
        loop {
            let Some(frame) = self.next_frame()? else {
                return Ok(None);
            };
            self.stats.frames += 1;
            if let Some(p) = self.unpack(frame)? {
                return Ok(Some(p));
            }
            self.stats.skipped += 1;
        }
    }

    /// Splits off the next raw frame (length prefix removed).
    fn next_frame(&mut self) -> Result<Option<Bytes>, FrameError> {
        let head = &self.buf[..self.buf.len().min(3)];
        let (len, n) = match read_varint_prefix(head) {
            Ok(Some(v)) => v,
            Ok(None) if head.len() < 3 => return Ok(None),
            _ => return Err(FrameError::BadLength),
        };
        if len <= 0 {
            return Err(FrameError::BadLength);
        }
        let len = len as usize;
        if len > self.limits.max_frame_len {
            return Err(FrameError::FrameTooLarge {
                len,
                max: self.limits.max_frame_len,
            });
        }
        if self.buf.len() < n + len {
            // Make room for the whole frame up front (one allocation).
            self.buf.reserve(n + len - self.buf.len());
            self.note_growth();
            return Ok(None);
        }
        self.buf.advance(n);
        let frame = if self.detach {
            let f = Bytes::copy_from_slice(&self.buf[..len]);
            self.buf.advance(len);
            f
        } else {
            self.buf.split_to(len).freeze()
        };
        if self.grown {
            if n + len > READ_BUF {
                self.fitting = 0;
            } else {
                self.fitting = self.fitting.saturating_add(1);
                if self.fitting >= SHRINK_AFTER && self.buf.len() <= READ_BUF / 2 {
                    // Large frames have stopped: the grown allocation goes with the last
                    // packets that still share it; what follows uses a standard buffer.
                    self.rehome();
                }
            }
        }
        Ok(Some(frame))
    }

    /// Turns a frame into a packet, or `None` if the filter drops it.
    fn unpack(&mut self, frame: Bytes) -> Result<Option<RawPacket>, FrameError> {
        if !self.compression {
            return self.split_id(frame);
        }
        let mut r = Reader::new(&frame);
        let data_len = r.read_varint()?;
        if data_len < 0 {
            return Err(FrameError::Decode(DecodeError::NegativeLength(i64::from(
                data_len,
            ))));
        }
        if data_len == 0 {
            let rest = frame.slice(r.position()..);
            return self.split_id(rest);
        }
        let data_len = data_len as usize;
        if data_len > self.limits.max_uncompressed_len {
            return Err(FrameError::DecompressedTooLarge {
                len: data_len,
                max: self.limits.max_uncompressed_len,
            });
        }
        let input = &frame[r.position()..];
        if !self.filter.is_all() {
            // Inflate just the packet id (at most 5 bytes) and decide.
            let mut head = [0u8; 5];
            let want = data_len.min(5);
            let got = INFLATER.with_borrow_mut(|inf| {
                inf.reset(true);
                inf.decompress(input, &mut head[..want], FlushDecompress::None)
                    .map_err(|_| FrameError::BadCompression("corrupt zlib stream"))?;
                Ok::<_, FrameError>(inf.total_out() as usize)
            })?;
            let id = match read_varint_prefix(&head[..got])? {
                Some((id, _)) => id,
                None if got < want => return Err(FrameError::BadCompression("stream ended early")),
                None => return Err(FrameError::Decode(DecodeError::VarIntTooLong)),
            };
            self.stats.inflated_bytes += got as u64;
            if !self.filter.contains(id) {
                self.stats.skipped_compressed += 1;
                return Ok(None);
            }
        }
        let out = self.inflate(input, data_len)?;
        self.stats.inflated_bytes += data_len as u64;
        let data = Bytes::from(out);
        let mut r = Reader::new(&data);
        let id = r.read_varint()?;
        let body = data.slice(r.position()..);
        Ok(Some(RawPacket::new(id, body)))
    }

    /// Inflates a whole zlib stream that must produce exactly `data_len` bytes.
    #[cfg(feature = "libdeflate")]
    fn inflate(&mut self, input: &[u8], data_len: usize) -> Result<Vec<u8>, FrameError> {
        let mut out = vec![0u8; data_len];
        match LIBDEFLATE_INFLATER.with_borrow_mut(|d| d.zlib_decompress(input, &mut out)) {
            Ok(n) if n == data_len => Ok(out),
            Ok(_) | Err(libdeflater::DecompressionError::InsufficientSpace) => Err(
                FrameError::BadCompression("size differs from declared length"),
            ),
            Err(_) => Err(FrameError::BadCompression("corrupt zlib stream")),
        }
    }

    /// Inflates a whole zlib stream that must produce exactly `data_len` bytes.
    #[cfg(not(feature = "libdeflate"))]
    fn inflate(&mut self, input: &[u8], data_len: usize) -> Result<Vec<u8>, FrameError> {
        INFLATER.with_borrow_mut(|inf| inflate_with(inf, input, data_len))
    }

    fn split_id(&self, data: Bytes) -> Result<Option<RawPacket>, FrameError> {
        let mut r = Reader::new(&data);
        let id = r.read_varint()?;
        if !self.filter.contains(id) {
            return Ok(None);
        }
        let body = data.slice(r.position()..);
        Ok(Some(RawPacket::new(id, body)))
    }
}

/// [`FrameDecoder::inflate`] with the thread's streaming inflater.
#[cfg(not(feature = "libdeflate"))]
fn inflate_with(
    inflater: &mut Decompress,
    input: &[u8],
    data_len: usize,
) -> Result<Vec<u8>, FrameError> {
    inflater.reset(true);
    let mut out: Vec<u8> = Vec::with_capacity(data_len);
    let status = inflater
        .decompress_vec(input, &mut out, FlushDecompress::Finish)
        .map_err(|_| FrameError::BadCompression("corrupt zlib stream"))?;
    if out.len() != data_len {
        return Err(FrameError::BadCompression(
            "size differs from declared length",
        ));
    }
    if status != Status::StreamEnd {
        // The output is full; the stream must end without producing more.
        let consumed = inflater.total_in() as usize;
        let mut extra = [0u8; 1];
        let st = inflater
            .decompress(
                &input[consumed.min(input.len())..],
                &mut extra,
                FlushDecompress::Finish,
            )
            .map_err(|_| FrameError::BadCompression("corrupt zlib stream"))?;
        if inflater.total_out() as usize != data_len || st != Status::StreamEnd {
            return Err(FrameError::BadCompression(
                "size differs from declared length",
            ));
        }
    }
    Ok(out)
}

/// Turns packets into wire bytes. Packets are appended to an internal output buffer;
/// take it with [`FrameEncoder::take`] (or let [`crate::io::FramedWriter`] flush it), so
/// several packets of one tick go out in a single write.
pub struct FrameEncoder {
    out: BytesMut,
    scratch: BytesMut,
    compressed: Vec<u8>,
    threshold: Option<usize>,
    encryptor: Option<Cfb8Encryptor>,
}

impl std::fmt::Debug for FrameEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameEncoder")
            .field("pending", &self.out.len())
            .field("threshold", &self.threshold)
            .field("encrypted", &self.encryptor.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for FrameEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameEncoder {
    /// An encoder with no compression and no encryption.
    pub fn new() -> Self {
        FrameEncoder {
            // A tick's packets are a few dozen bytes; both grow when a larger one is written.
            out: BytesMut::with_capacity(256),
            scratch: BytesMut::with_capacity(256),
            compressed: Vec::new(),
            threshold: None,
            encryptor: None,
        }
    }

    /// Sets the compression threshold (negative = off), as received from the server.
    pub fn set_compression(&mut self, threshold: i32) {
        self.threshold = usize::try_from(threshold).ok();
    }

    /// Turns on encryption for everything written from now on.
    pub fn enable_encryption(&mut self, secret: &[u8; 16]) {
        self.encryptor = Some(Cfb8Encryptor::new(secret));
    }

    /// Encodes and frames a typed packet.
    pub fn write_packet<P: Packet>(&mut self, p: &P) -> Result<(), EncodeError> {
        self.scratch.clear();
        self.scratch.put_varint(P::ID);
        p.encode(&mut self.scratch)?;
        self.frame_scratch()
    }

    /// Encodes and frames a packet-set enum value.
    pub fn write_set<S: PacketSet>(&mut self, p: &S) -> Result<(), EncodeError> {
        self.scratch.clear();
        self.scratch.put_varint(p.id());
        p.encode_body(&mut self.scratch)?;
        self.frame_scratch()
    }

    /// Frames a raw packet.
    pub fn write_raw(&mut self, p: &RawPacket) -> Result<(), EncodeError> {
        self.scratch.clear();
        self.scratch.put_varint(p.id);
        self.scratch.put_slice(&p.body);
        self.frame_scratch()
    }

    fn frame_scratch(&mut self) -> Result<(), EncodeError> {
        let start = self.out.len();
        let len = self.scratch.len();
        let too_big = |len| EncodeError::LengthOverflow {
            len,
            max: MAX_FRAME_LEN,
        };
        match self.threshold {
            None => {
                if len > MAX_FRAME_LEN {
                    return Err(too_big(len));
                }
                self.out.put_varint(len as i32);
                self.out.put_slice(&self.scratch);
            }
            Some(t) if len < t => {
                if len + 1 > MAX_FRAME_LEN {
                    return Err(too_big(len + 1));
                }
                self.out.put_varint(len as i32 + 1);
                self.out.put_u8(0);
                self.out.put_slice(&self.scratch);
            }
            Some(_) => {
                let len_i = i32::try_from(len).map_err(|_| too_big(len))?;
                self.deflate()?;
                let frame_len = varint_len(len_i) + self.compressed.len();
                if frame_len > MAX_FRAME_LEN {
                    return Err(too_big(frame_len));
                }
                self.out.put_varint(frame_len as i32);
                self.out.put_varint(len_i);
                self.out.put_slice(&self.compressed);
            }
        }
        if let Some(e) = &mut self.encryptor {
            e.encrypt(&mut self.out[start..]);
        }
        Ok(())
    }

    /// Compresses `scratch` into `compressed`.
    #[cfg(feature = "libdeflate")]
    fn deflate(&mut self) -> Result<(), EncodeError> {
        DEFLATER.with_borrow_mut(|deflater| {
            let bound = deflater.zlib_compress_bound(self.scratch.len());
            self.compressed.clear();
            self.compressed.resize(bound, 0);
            let n = deflater
                .zlib_compress(&self.scratch, &mut self.compressed)
                .map_err(|_| EncodeError::Invalid("deflate failed"))?;
            self.compressed.truncate(n);
            Ok(())
        })
    }

    /// Compresses `scratch` into `compressed`.
    #[cfg(not(feature = "libdeflate"))]
    fn deflate(&mut self) -> Result<(), EncodeError> {
        DEFLATER.with_borrow_mut(|deflater| self.deflate_with(deflater))
    }

    #[cfg(not(feature = "libdeflate"))]
    fn deflate_with(&mut self, deflater: &mut Compress) -> Result<(), EncodeError> {
        deflater.reset();
        self.compressed.clear();
        self.compressed
            .reserve(self.scratch.len() + self.scratch.len() / 1000 + 64);
        loop {
            let consumed = deflater.total_in() as usize;
            let st = deflater
                .compress_vec(
                    &self.scratch[consumed..],
                    &mut self.compressed,
                    FlushCompress::Finish,
                )
                .map_err(|_| EncodeError::Invalid("deflate failed"))?;
            if st == Status::StreamEnd {
                return Ok(());
            }
            self.compressed.reserve(self.compressed.capacity().max(64));
        }
    }

    /// Bytes waiting to be sent.
    pub fn pending(&self) -> &[u8] {
        &self.out
    }

    /// Takes the bytes waiting to be sent.
    pub fn take(&mut self) -> BytesMut {
        self.out.split()
    }

    /// The output buffer (for transports that consume it in place).
    pub fn out_mut(&mut self) -> &mut BytesMut {
        &mut self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression, FlushCompress};

    fn roundtrip(threshold: Option<i32>, secret: Option<[u8; 16]>, filter: PacketFilter) {
        let mut enc = FrameEncoder::new();
        let mut dec = FrameDecoder::new();
        if let Some(t) = threshold {
            enc.set_compression(t);
            dec.set_compression(t);
        }
        if let Some(s) = secret {
            enc.enable_encryption(&s);
            dec.enable_encryption(&s);
        }
        dec.set_filter(filter);
        let packets: Vec<RawPacket> = (0..50)
            .map(|i| {
                let body: Vec<u8> = (0..(i * 37) % 700).map(|j| (j * 7 + i) as u8).collect();
                RawPacket::new(i % 9, Bytes::from(body))
            })
            .collect();
        for p in &packets {
            enc.write_raw(p).unwrap();
        }
        let wire = enc.take();
        // Feed in odd-sized pieces.
        let mut got = Vec::new();
        for chunk in wire.chunks(13) {
            dec.push(chunk);
            while let Some(p) = dec.next_packet().unwrap() {
                got.push(p);
            }
        }
        let want: Vec<RawPacket> = packets
            .into_iter()
            .filter(|p| filter.contains(p.id))
            .collect();
        assert_eq!(got, want);
        assert_eq!(dec.buffered(), 0);
    }

    #[test]
    fn roundtrips() {
        let key = [3u8; 16];
        let f = PacketFilter::none().with(1).with(4);
        for (t, s, filt) in [
            (None, None, PacketFilter::all()),
            (Some(256), None, PacketFilter::all()),
            (Some(0), None, PacketFilter::all()),
            (Some(64), Some(key), PacketFilter::all()),
            (None, Some(key), f),
            (Some(256), None, f),
            (Some(1), Some(key), f),
        ] {
            roundtrip(t, s, filt);
        }
    }

    #[test]
    fn known_answer_frames() {
        // Uncompressed: length 3, id 0x00, body [0x01, 0x02].
        let mut enc = FrameEncoder::new();
        enc.write_raw(&RawPacket::new(0, Bytes::from_static(&[1, 2])))
            .unwrap();
        assert_eq!(&enc.take()[..], &[0x03, 0x00, 0x01, 0x02]);
        // Below the threshold: data length 0 marker.
        enc.set_compression(256);
        enc.write_raw(&RawPacket::new(0x21, Bytes::from_static(&[9])))
            .unwrap();
        assert_eq!(&enc.take()[..], &[0x03, 0x00, 0x21, 0x09]);
        // A zlib frame produced by Java's Deflater (level 6) for id 0x26 + 300 zero bytes
        // decodes back to that packet.
        let mut body = vec![0x26];
        body.extend_from_slice(&[0u8; 300]);
        let mut z = Compress::new(Compression::default(), true);
        let mut c = Vec::with_capacity(400);
        z.compress_vec(&body, &mut c, FlushCompress::Finish)
            .unwrap();
        assert_eq!(&c[..2], &[0x78, 0x9c], "zlib header for the default level");
        let mut frame = BytesMut::new();
        frame.put_varint((varint_len(301) + c.len()) as i32);
        frame.put_varint(301);
        frame.put_slice(&c);
        let mut dec = FrameDecoder::new();
        dec.set_compression(256);
        dec.push(&frame);
        let p = dec.next_packet().unwrap().unwrap();
        assert_eq!(p.id, 0x26);
        assert_eq!(&p.body[..], &[0u8; 300][..]);
    }

    #[test]
    fn rejects_malformed() {
        let mut dec = FrameDecoder::new();
        dec.push(&[0xff, 0xff, 0xff, 0x01]);
        assert!(matches!(dec.next_packet(), Err(FrameError::BadLength)));
        let mut dec = FrameDecoder::new();
        dec.push(&[0x00]);
        assert!(matches!(dec.next_packet(), Err(FrameError::BadLength)));
        // Declared uncompressed size too large.
        let mut dec = FrameDecoder::new();
        dec.set_compression(0);
        dec.push(&[0x05, 0x80, 0x80, 0x80, 0x10, 0x00]);
        assert!(matches!(
            dec.next_packet(),
            Err(FrameError::DecompressedTooLarge { .. })
        ));
        // Garbage instead of zlib.
        let mut dec = FrameDecoder::new();
        dec.set_compression(0);
        dec.push(&[0x04, 0x05, 0x01, 0x02, 0x03]);
        assert!(matches!(
            dec.next_packet(),
            Err(FrameError::BadCompression(_))
        ));
        // Declared size larger than the actual inflated size.
        let mut z = Compress::new(Compression::default(), true);
        let mut c = Vec::with_capacity(64);
        z.compress_vec(&[1, 2, 3], &mut c, FlushCompress::Finish)
            .unwrap();
        let mut frame = BytesMut::new();
        frame.put_varint(1 + c.len() as i32);
        frame.put_varint(4);
        frame.put_slice(&c);
        let mut dec = FrameDecoder::new();
        dec.set_compression(0);
        dec.push(&frame);
        assert!(matches!(
            dec.next_packet(),
            Err(FrameError::BadCompression(_))
        ));
        // ... and smaller.
        let mut frame = BytesMut::new();
        frame.put_varint(1 + c.len() as i32);
        frame.put_varint(2);
        frame.put_slice(&c);
        let mut dec = FrameDecoder::new();
        dec.set_compression(0);
        dec.push(&frame);
        assert!(matches!(
            dec.next_packet(),
            Err(FrameError::BadCompression(_))
        ));
    }

    #[test]
    fn partial_inflation_skips_unwanted() {
        let mut enc = FrameEncoder::new();
        enc.set_compression(16);
        let big = RawPacket::new(0x38, Bytes::from(vec![7u8; 100_000]));
        enc.write_raw(&big).unwrap();
        enc.write_raw(&RawPacket::new(0x00, Bytes::from(vec![1u8; 64])))
            .unwrap();
        let mut dec = FrameDecoder::new();
        dec.set_compression(16);
        dec.set_filter(PacketFilter::all().without(0x38));
        dec.push(&enc.take());
        let p = dec.next_packet().unwrap().unwrap();
        assert_eq!(p.id, 0);
        let s = dec.stats();
        assert_eq!((s.frames, s.skipped, s.skipped_compressed), (2, 1, 1));
        assert!(
            s.inflated_bytes < 100,
            "inflated {} bytes",
            s.inflated_bytes
        );
    }

    #[test]
    fn encryption_enabled_mid_buffer() {
        // Bytes that arrive before `enable_encryption` but belong after the switch are
        // decrypted when it is enabled.
        let key = [9u8; 16];
        let mut plain = FrameEncoder::new();
        plain
            .write_raw(&RawPacket::new(1, Bytes::from_static(b"plain")))
            .unwrap();
        let mut crypt = FrameEncoder::new();
        crypt.enable_encryption(&key);
        crypt
            .write_raw(&RawPacket::new(2, Bytes::from_static(b"secret")))
            .unwrap();
        let mut dec = FrameDecoder::new();
        dec.push(&plain.take());
        dec.push(&crypt.take());
        assert_eq!(dec.next_packet().unwrap().unwrap().id, 1);
        dec.enable_encryption(&key);
        let p = dec.next_packet().unwrap().unwrap();
        assert_eq!((p.id, &p.body[..]), (2, &b"secret"[..]));
    }

    /// Feeds `wire` through `reserve_read` / `commit` in reads of at most `chunk` bytes (like
    /// a socket would) and returns the packets and the largest buffer capacity seen after a
    /// packet was split off.
    fn read_all(dec: &mut FrameDecoder, wire: &[u8], chunk: usize) -> (Vec<RawPacket>, usize) {
        let mut got = Vec::new();
        let mut pos = 0;
        let mut max_after = 0;
        loop {
            while let Some(p) = dec.next_packet().unwrap() {
                max_after = max_after.max(dec.buf.capacity());
                got.push(p);
            }
            if pos == wire.len() {
                return (got, max_after);
            }
            let buf = dec.reserve_read();
            let old = buf.len();
            let free = buf.capacity() - old;
            assert!(free > 0);
            let n = free.min(chunk).min(wire.len() - pos);
            buf.extend_from_slice(&wire[pos..pos + n]);
            pos += n;
            dec.commit(old);
        }
    }

    #[test]
    fn a_large_frame_does_not_leave_a_large_buffer() {
        let mut enc = FrameEncoder::new();
        let small = RawPacket::new(1, Bytes::from_static(b"keep alive"));
        let big = RawPacket::new(
            2,
            Bytes::from((0..300_000).map(|i| i as u8).collect::<Vec<_>>()),
        );
        let mut sent = vec![small.clone(), big.clone(), big.clone()];
        sent.extend(std::iter::repeat_n(
            small.clone(),
            SHRINK_AFTER as usize + 2,
        ));
        for p in &sent {
            enc.write_raw(p).unwrap();
        }
        let wire = enc.take();
        for chunk in [7, 1000, 64 * 1024, usize::MAX] {
            let mut dec = FrameDecoder::new();
            let (got, _) = read_all(&mut dec, &wire, chunk);
            assert_eq!(got, sent);
            assert!(!dec.grown, "chunk {chunk}");
            drop(got);
            // The big frames' allocation went with them; the decoder is back to the standard
            // size.
            assert!(
                dec.buf.capacity() <= READ_BUF,
                "chunk {chunk}: {}",
                dec.buf.capacity()
            );
        }
        // Large frames in a row keep the grown buffer (no reallocation per frame).
        let mut enc = FrameEncoder::new();
        for _ in 0..5 {
            enc.write_raw(&big).unwrap();
        }
        let wire = enc.take();
        let mut dec = FrameDecoder::new();
        let (got, _) = read_all(&mut dec, &wire, 64 * 1024);
        assert_eq!(got.len(), 5);
        assert!(dec.grown);
    }

    #[test]
    fn steady_reads_keep_the_standard_buffer() {
        let mut enc = FrameEncoder::new();
        enc.set_compression(256);
        let packets: Vec<RawPacket> = (0..3000)
            .map(|i| {
                RawPacket::new(
                    i % 50,
                    Bytes::from(vec![i as u8; (i as usize * 131) % 3000]),
                )
            })
            .collect();
        for p in &packets {
            enc.write_raw(p).unwrap();
        }
        let wire = enc.take();
        for chunk in [100, 1500, 9000] {
            let mut dec = FrameDecoder::new();
            dec.set_compression(256);
            let (got, max_after) = read_all(&mut dec, &wire, chunk);
            assert_eq!(got, packets);
            // Packets dropped as they are handled: one standard buffer throughout.
            let mut dec = FrameDecoder::new();
            dec.set_compression(256);
            let mut n = 0;
            let mut pos = 0;
            let mut max_cap = 0;
            while n < packets.len() {
                while let Some(p) = dec.next_packet().unwrap() {
                    assert_eq!(p, packets[n]);
                    n += 1;
                }
                let buf = dec.reserve_read();
                let old = buf.len();
                let k = (buf.capacity() - old).min(chunk).min(wire.len() - pos);
                buf.extend_from_slice(&wire[pos..pos + k]);
                pos += k;
                dec.commit(old);
                max_cap = max_cap.max(dec.buf.capacity() + dec.buf.len());
            }
            assert!(max_cap <= 2 * READ_BUF, "chunk {chunk}: {max_cap}");
            assert!(max_after <= 2 * READ_BUF, "chunk {chunk}: {max_after}");
        }
    }

    #[test]
    fn detached_frames_do_not_share_the_buffer() {
        let mut enc = FrameEncoder::new();
        for i in 0..4 {
            enc.write_raw(&RawPacket::new(i, Bytes::from_static(b"registry entry")))
                .unwrap();
        }
        let wire = enc.take();
        let mut dec = FrameDecoder::new();
        dec.set_detach_frames(true);
        dec.push(&wire[..wire.len() / 2]);
        let a = dec.next_packet().unwrap().unwrap();
        assert!(a.body.is_unique());
        dec.set_detach_frames(false);
        let b = dec.next_packet().unwrap().unwrap();
        assert!(!b.body.is_unique());
        dec.rehome();
        assert!(b.body.is_unique());
        dec.push(&wire[wire.len() / 2..]);
        let rest: Vec<_> = std::iter::from_fn(|| dec.next_packet().unwrap()).collect();
        assert_eq!(rest.len(), 2);
        assert_eq!(a.body, b"registry entry"[..]);
    }
}
