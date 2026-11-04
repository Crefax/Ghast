//! AES-128-CFB8 stream cipher used after the login encryption handshake.
//!
//! Key and IV are both the 16-byte shared secret (negotiated by `ghast-auth`). Both
//! directions are independent streams; [`Cfb8Encryptor`] and [`Cfb8Decryptor`] keep the
//! running state between calls, so data can be processed in arbitrary pieces.
//!
//! Encryption is inherently serial in CFB8 (each output byte feeds the next block). The
//! decryptor exploits that every keystream block depends only on *ciphertext*, which is
//! all known up front: it batches many AES block encryptions per call so the
//! hardware AES pipeline (AES-NI, ARMv8 crypto extensions — runtime-detected by the
//! `aes` crate) stays full. See the `decrypt` benchmark for the speedup over a serial
//! implementation.

use aes::Aes128;
use aes::cipher::{BlockCipherEncrypt, KeyInit, KeyIvInit};

/// Bytes decrypted per AES batch.
const BATCH: usize = 64;

/// Streaming AES-128-CFB8 encryptor.
pub struct Cfb8Encryptor {
    inner: cfb8::Encryptor<Aes128>,
}

impl std::fmt::Debug for Cfb8Encryptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cfb8Encryptor { .. }")
    }
}

impl Cfb8Encryptor {
    /// Creates an encryptor with key = IV = `secret`.
    pub fn new(secret: &[u8; 16]) -> Self {
        Cfb8Encryptor {
            inner: cfb8::Encryptor::<Aes128>::new(&(*secret).into(), &(*secret).into()),
        }
    }

    /// Encrypts `buf` in place, continuing the stream.
    #[inline]
    pub fn encrypt(&mut self, buf: &mut [u8]) {
        self.inner.encrypt(buf);
    }
}

/// Streaming AES-128-CFB8 decryptor with batched (pipelined) AES.
pub struct Cfb8Decryptor {
    cipher: Aes128,
    /// The last 16 ciphertext bytes (initially the IV).
    shift: [u8; 16],
    /// Scratch blocks for one batch.
    blocks: Box<[aes::Block; BATCH]>,
}

impl std::fmt::Debug for Cfb8Decryptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cfb8Decryptor { .. }")
    }
}

impl Cfb8Decryptor {
    /// Creates a decryptor with key = IV = `secret`.
    pub fn new(secret: &[u8; 16]) -> Self {
        Cfb8Decryptor {
            cipher: Aes128::new(&(*secret).into()),
            shift: *secret,
            blocks: Box::new([aes::Block::default(); BATCH]),
        }
    }

    /// Decrypts `buf` in place, continuing the stream.
    pub fn decrypt(&mut self, buf: &mut [u8]) {
        // `window` = last 16 ciphertext bytes followed by this batch's ciphertext; the
        // keystream block for byte i is AES(window[i..i + 16]).
        let mut window = [0u8; 16 + BATCH];
        for chunk in buf.chunks_mut(BATCH) {
            let n = chunk.len();
            window[..16].copy_from_slice(&self.shift);
            window[16..16 + n].copy_from_slice(chunk);
            for (i, block) in self.blocks[..n].iter_mut().enumerate() {
                block.copy_from_slice(&window[i..i + 16]);
            }
            self.cipher.encrypt_blocks(&mut self.blocks[..n]);
            for (b, ks) in chunk.iter_mut().zip(self.blocks[..n].iter()) {
                *b ^= ks[0];
            }
            self.shift.copy_from_slice(&window[n..n + 16]);
        }
    }
}

/// A matched pair of stream ciphers for one connection.
#[derive(Debug)]
pub struct CipherPair {
    /// Encrypts outbound bytes.
    pub encryptor: Cfb8Encryptor,
    /// Decrypts inbound bytes.
    pub decryptor: Cfb8Decryptor,
}

impl CipherPair {
    /// Creates both directions from the shared secret.
    pub fn new(secret: &[u8; 16]) -> Self {
        CipherPair {
            encryptor: Cfb8Encryptor::new(secret),
            decryptor: Cfb8Decryptor::new(secret),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// NIST SP 800-38A, F.3.7 / F.3.8 (CFB8-AES128). Uses a distinct IV, so drive the
    /// cipher types directly instead of through the key = IV constructor.
    #[test]
    fn nist_sp800_38a_cfb8() {
        let key: [u8; 16] = hex("2b7e151628aed2a6abf7158809cf4f3c").try_into().unwrap();
        let iv: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let pt = hex("6bc1bee22e409f96e93d7e117393172aae2d");
        let ct = hex("3b79424c9c0dd436bace9e0ed4586a4f32b9");

        let mut enc = cfb8::Encryptor::<Aes128>::new(&key.into(), &iv.into());
        let mut buf = pt.clone();
        enc.encrypt(&mut buf);
        assert_eq!(buf, ct);

        let mut dec = Cfb8Decryptor {
            cipher: Aes128::new(&key.into()),
            shift: iv,
            blocks: Box::new([aes::Block::default(); BATCH]),
        };
        let mut buf = ct.clone();
        dec.decrypt(&mut buf);
        assert_eq!(buf, pt);
    }

    #[test]
    fn stream_in_pieces_matches_reference() {
        let secret = [7u8; 16];
        let data: Vec<u8> = (0..5000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect();
        let mut ct = data.clone();
        let mut enc = Cfb8Encryptor::new(&secret);
        // Encrypt in uneven pieces.
        let mut off = 0;
        for step in [1, 15, 16, 17, 63, 64, 65, 200, 1000].iter().cycle() {
            if off >= ct.len() {
                break;
            }
            let end = (off + step).min(ct.len());
            enc.encrypt(&mut ct[off..end]);
            off = end;
        }
        // Reference: the serial cfb8 decryptor in one go.
        let mut reference = ct.clone();
        cfb8::Decryptor::<Aes128>::new(&secret.into(), &secret.into()).decrypt(&mut reference);
        assert_eq!(reference, data);
        // Ours, in different uneven pieces.
        let mut pt = ct.clone();
        let mut dec = Cfb8Decryptor::new(&secret);
        let mut off = 0;
        for step in [3, 64, 1, 129, 7, 4096].iter().cycle() {
            if off >= pt.len() {
                break;
            }
            let end = (off + step).min(pt.len());
            dec.decrypt(&mut pt[off..end]);
            off = end;
        }
        assert_eq!(pt, data);
    }
}
