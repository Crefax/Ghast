//! Async framed reader and writer over tokio's `AsyncRead` / `AsyncWrite`
//! (feature `tokio`).
//!
//! Use them on the two halves of a connection (e.g. `TcpStream::into_split`). Each half
//! has its own compression/encryption switches; the connection logic flips both at the
//! protocol-defined points.

use bytes::Buf;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{EncodeError, FrameError};
use crate::frame::{FrameDecoder, FrameEncoder};
use crate::packet::{Packet, PacketSet, RawPacket};

/// Minimum free space requested from the socket per read.
const READ_CHUNK: usize = 16 * 1024;

/// Reads packets from an `AsyncRead`.
#[derive(Debug)]
pub struct FramedReader<R> {
    inner: R,
    decoder: FrameDecoder,
}

impl<R: AsyncRead + Unpin> FramedReader<R> {
    /// Wraps a reader.
    pub fn new(inner: R) -> Self {
        FramedReader {
            inner,
            decoder: FrameDecoder::new(),
        }
    }

    /// The frame decoder (compression, encryption, filter, limits, stats).
    pub fn decoder(&mut self) -> &mut FrameDecoder {
        &mut self.decoder
    }

    /// The underlying reader.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// Reads the next packet that passes the filter. `Ok(None)` on a clean end of
    /// stream (no partial frame buffered).
    ///
    /// Cancel safe: if the future is dropped, no data is lost.
    pub async fn read_packet(&mut self) -> Result<Option<RawPacket>, FrameError> {
        loop {
            if let Some(p) = self.decoder.next_packet()? {
                return Ok(Some(p));
            }
            let want = self.decoder.needed().unwrap_or(0).max(READ_CHUNK);
            let buf = self.decoder.buffer_mut();
            buf.reserve(want);
            let old = buf.len();
            let n = self.inner.read_buf(buf).await?;
            if n == 0 {
                if self.decoder.buffered() == 0 {
                    return Ok(None);
                }
                return Err(FrameError::Io(std::io::ErrorKind::UnexpectedEof.into()));
            }
            self.decoder.commit(old);
        }
    }
}

/// Writes packets to an `AsyncWrite`, batching until [`FramedWriter::flush`].
#[derive(Debug)]
pub struct FramedWriter<W> {
    inner: W,
    encoder: FrameEncoder,
}

impl<W: AsyncWrite + Unpin> FramedWriter<W> {
    /// Wraps a writer.
    pub fn new(inner: W) -> Self {
        FramedWriter {
            inner,
            encoder: FrameEncoder::new(),
        }
    }

    /// The frame encoder (compression, encryption).
    pub fn encoder(&mut self) -> &mut FrameEncoder {
        &mut self.encoder
    }

    /// The underlying writer.
    pub fn get_mut(&mut self) -> &mut W {
        &mut self.inner
    }

    /// Queues a typed packet (nothing is written until [`FramedWriter::flush`]).
    pub fn queue<P: Packet>(&mut self, p: &P) -> Result<(), EncodeError> {
        self.encoder.write_packet(p)
    }

    /// Queues a packet-set value.
    pub fn queue_set<S: PacketSet>(&mut self, p: &S) -> Result<(), EncodeError> {
        self.encoder.write_set(p)
    }

    /// Queues a raw packet.
    pub fn queue_raw(&mut self, p: &RawPacket) -> Result<(), EncodeError> {
        self.encoder.write_raw(p)
    }

    /// Writes everything queued and flushes the writer.
    ///
    /// Cancel safe in the sense that bytes already written are removed from the queue;
    /// a dropped flush can be resumed by calling it again.
    pub async fn flush(&mut self) -> std::io::Result<()> {
        let out = self.encoder.out_mut();
        while out.has_remaining() {
            let n = self.inner.write_buf(out).await?;
            if n == 0 {
                return Err(std::io::ErrorKind::WriteZero.into());
            }
        }
        self.inner.flush().await
    }

    /// Queues a typed packet and flushes.
    pub async fn send<P: Packet>(&mut self, p: &P) -> Result<(), FrameError> {
        self.queue(p)
            .map_err(|e| FrameError::Io(std::io::Error::other(e)))?;
        self.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[tokio::test]
    async fn duplex_roundtrip() {
        let (a, b) = tokio::io::duplex(64);
        let mut w = FramedWriter::new(a);
        let mut r = FramedReader::new(b);
        let key = [5u8; 16];
        w.encoder().set_compression(32);
        w.encoder().enable_encryption(&key);
        r.decoder().set_compression(32);
        r.decoder().enable_encryption(&key);
        let packets: Vec<RawPacket> = (0..20)
            .map(|i| RawPacket::new(i, Bytes::from(vec![i as u8; (i * 13) as usize])))
            .collect();
        let send = packets.clone();
        let writer = tokio::spawn(async move {
            for p in &send {
                w.queue_raw(p).unwrap();
            }
            w.flush().await.unwrap();
            drop(w);
        });
        let mut got = Vec::new();
        while let Some(p) = r.read_packet().await.unwrap() {
            got.push(p);
        }
        writer.await.unwrap();
        assert_eq!(got, packets);
    }
}
