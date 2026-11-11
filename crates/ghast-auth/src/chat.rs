//! Signed chat (Minecraft 1.19.3+ chat sessions): the player's profile key pair and the message
//! signature chain of a chat session.
//!
//! An online-mode client fetches a **profile key pair** from the Minecraft services
//! (`MicrosoftAuth::profile_keys` with the `microsoft` feature, or bring your own with
//! [`ProfileKeys::from_der`]), starts a **chat session** (a random session id, announced to the
//! server with the public key, its expiry and the services' signature over them), and signs
//! every chat message and every signable command argument with the private key
//! (SHA256withRSA, PKCS#1 v1.5 — deterministic, so the same input gives the same bytes as the
//! vanilla client).
//!
//! The signed data of one message (`PlayerChatMessage.updateSignature` in 26.1): the version
//! `1` (i32), the link (sender profile id, session id, index), the body (salt, timestamp in
//! **seconds**, content length and UTF-8 bytes, the count and the bytes of the last-seen
//! signatures), all big-endian. [`SignedMessageChain::sign`] builds it straight into the hash.
//!
//! This module does no I/O and knows no packets: the client crate puts the results into
//! `chat_session_update`, `chat_message` and `chat_command_signed`.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::traits::PublicKeyParts;
use rsa::{Pkcs1v15Sign, RsaPrivateKey, RsaPublicKey};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

/// DER `DigestInfo` prefix of SHA-256 (RFC 8017 §9.2, note 1).
const SHA256_PREFIX: [u8; 19] = [
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05,
    0x00, 0x04, 0x20,
];

/// Errors of [`ProfileKeys`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ChatKeyError {
    /// The private key is not a PKCS#8 RSA key.
    #[error("invalid private key: {0}")]
    InvalidPrivateKey(String),
    /// The public key is not a DER `SubjectPublicKeyInfo` RSA key.
    #[error("invalid public key: {0}")]
    InvalidPublicKey(String),
    /// The public key does not belong to the private key.
    #[error("the public key does not match the private key")]
    KeyMismatch,
    /// Signing failed.
    #[error("signing failed: {0}")]
    Sign(#[source] rsa::Error),
}

/// A profile key pair: what the Minecraft services issue for signed chat (valid for about two
/// days; vanilla fetches a new one once [`ProfileKeys::due_refresh`]).
///
/// `Debug` does not print the private key.
#[derive(Clone)]
pub struct ProfileKeys {
    private: RsaPrivateKey,
    public_der: Vec<u8>,
    key_signature: Vec<u8>,
    expires_at: SystemTime,
    refreshed_after: SystemTime,
}

impl ProfileKeys {
    /// A key pair from its parts: the PKCS#8 DER private key, the DER `SubjectPublicKeyInfo`
    /// public key, the services' signature over the public key (`publicKeySignatureV2`), when
    /// it expires and when it should be refreshed.
    pub fn from_der(
        private_pkcs8: &[u8],
        public_spki: &[u8],
        key_signature: Vec<u8>,
        expires_at: SystemTime,
        refreshed_after: SystemTime,
    ) -> Result<Self, ChatKeyError> {
        let private = RsaPrivateKey::from_pkcs8_der(private_pkcs8)
            .map_err(|e| ChatKeyError::InvalidPrivateKey(e.to_string()))?;
        let public = RsaPublicKey::from_public_key_der(public_spki)
            .map_err(|e| ChatKeyError::InvalidPublicKey(e.to_string()))?;
        if public.n() != private.n() || public.e() != private.e() {
            return Err(ChatKeyError::KeyMismatch);
        }
        Ok(Self {
            private,
            public_der: public_spki.to_vec(),
            key_signature,
            expires_at,
            refreshed_after,
        })
    }

    /// The public key as DER `SubjectPublicKeyInfo` (sent in `chat_session_update`).
    #[must_use]
    pub fn public_key_der(&self) -> &[u8] {
        &self.public_der
    }

    /// The services' signature over the profile id, the expiry and the public key.
    #[must_use]
    pub fn key_signature(&self) -> &[u8] {
        &self.key_signature
    }

    /// When the key expires (servers refuse it afterwards).
    #[must_use]
    pub fn expires_at(&self) -> SystemTime {
        self.expires_at
    }

    /// The expiry in milliseconds since the Unix epoch (its wire form).
    #[must_use]
    pub fn expires_at_millis(&self) -> i64 {
        millis(self.expires_at)
    }

    /// When the services want the key replaced.
    #[must_use]
    pub fn refreshed_after(&self) -> SystemTime {
        self.refreshed_after
    }

    /// `ProfileKeyPair.dueRefresh`: the refresh time has passed.
    #[must_use]
    pub fn due_refresh(&self) -> bool {
        SystemTime::now() > self.refreshed_after
    }

    /// The key has expired (or does so within `margin`).
    #[must_use]
    pub fn expires_within(&self, margin: Duration) -> bool {
        SystemTime::now()
            .checked_add(margin)
            .is_none_or(|t| t >= self.expires_at)
    }

    /// The signature length in bytes (the modulus size; 256 for the services' 2048-bit keys).
    #[must_use]
    pub fn signature_len(&self) -> usize {
        self.private.size()
    }

    /// SHA256withRSA over everything `feed` passes to the hasher.
    fn sign_with(&self, feed: impl FnOnce(&mut Sha256)) -> Result<Vec<u8>, ChatKeyError> {
        let mut h = Sha256::new();
        feed(&mut h);
        let digest = h.finalize();
        let scheme = Pkcs1v15Sign {
            hash_len: Some(32),
            prefix: Box::new(SHA256_PREFIX),
        };
        self.private
            .sign(scheme, &digest[..])
            .map_err(ChatKeyError::Sign)
    }

    /// SHA256withRSA over `data` (`Signer.from(privateKey, "SHA256withRSA")`).
    pub fn sign(&self, data: &[u8]) -> Result<Vec<u8>, ChatKeyError> {
        self.sign_with(|h| h.update(data))
    }
}

impl fmt::Debug for ProfileKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProfileKeys")
            .field("bits", &(self.private.size() * 8))
            .field("expires_at_ms", &self.expires_at_millis())
            .finish_non_exhaustive()
    }
}

fn millis(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

/// One chat session's signature chain (`SignedMessageChain` with its encoder): every signed
/// message takes the next index of the link (sender, session id, index).
#[derive(Debug, Clone)]
pub struct SignedMessageChain {
    profile_id: Uuid,
    session_id: Uuid,
    /// The next link's index; `None` once the chain is exhausted (`Integer.MAX_VALUE`).
    next: Option<i32>,
}

impl SignedMessageChain {
    /// A new chain at index 0 (`SignedMessageLink.root`).
    #[must_use]
    pub fn new(profile_id: Uuid, session_id: Uuid) -> Self {
        Self {
            profile_id,
            session_id,
            next: Some(0),
        }
    }

    /// The session id.
    #[must_use]
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// The index the next signed message gets.
    #[must_use]
    pub fn next_index(&self) -> Option<i32> {
        self.next
    }

    /// Signs one message body and advances the chain (`SignedMessageChain.Encoder.pack`).
    /// `timestamp_millis` is the packet's timestamp (the signature covers its whole seconds);
    /// `last_seen` are the acknowledged signatures in the order of the update. `None` once the
    /// chain is exhausted (vanilla then sends the message unsigned).
    pub fn sign<'a>(
        &mut self,
        keys: &ProfileKeys,
        content: &str,
        timestamp_millis: i64,
        salt: i64,
        last_seen: impl ExactSizeIterator<Item = &'a [u8]>,
    ) -> Result<Option<Vec<u8>>, ChatKeyError> {
        let Some(index) = self.next else {
            return Ok(None);
        };
        self.next = index.checked_add(1);
        let (profile, session) = (self.profile_id, self.session_id);
        keys.sign_with(|h| {
            h.update(1i32.to_be_bytes());
            h.update(profile.as_bytes());
            h.update(session.as_bytes());
            h.update(index.to_be_bytes());
            h.update(salt.to_be_bytes());
            // Instant.getEpochSecond: floor division.
            h.update(timestamp_millis.div_euclid(1000).to_be_bytes());
            let bytes = content.as_bytes();
            h.update((bytes.len() as i32).to_be_bytes());
            h.update(bytes);
            h.update((last_seen.len() as i32).to_be_bytes());
            for sig in last_seen {
                h.update(sig);
            }
        })
        .map(Some)
    }
}

/// The payload the services sign for a profile key (`ProfilePublicKey.Data.signedPayload`): the
/// profile id, the expiry in milliseconds and the DER public key. For checking a key or for a
/// local test key service.
#[must_use]
pub fn profile_key_payload(
    profile_id: Uuid,
    expires_at_millis: i64,
    public_spki: &[u8],
) -> Vec<u8> {
    let mut v = Vec::with_capacity(24 + public_spki.len());
    v.extend_from_slice(profile_id.as_bytes());
    v.extend_from_slice(&expires_at_millis.to_be_bytes());
    v.extend_from_slice(public_spki);
    v
}

/// Decodes the base64 body of a PEM block (`-----BEGIN …-----`, MIME line breaks), as the
/// services send the key pair (`Crypt.rsaStringToKey`).
pub fn pem_body(pem: &str) -> Option<Vec<u8>> {
    let mut body = String::with_capacity(pem.len());
    for line in pem.lines() {
        let line = line.trim();
        if line.starts_with("-----") || line.is_empty() {
            continue;
        }
        body.push_str(line);
    }
    base64_decode(&body)
}

/// Standard base64 (padding optional, no whitespace).
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    let s = s.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for chunk in s.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            acc |= val(c)? << (18 - 6 * i);
        }
        match chunk.len() {
            4 => out.extend_from_slice(&[(acc >> 16) as u8, (acc >> 8) as u8, acc as u8]),
            3 => out.extend_from_slice(&[(acc >> 16) as u8, (acc >> 8) as u8]),
            2 => out.push((acc >> 16) as u8),
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_known_values() {
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8").unwrap(), b"fo");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
        assert!(base64_decode("Zm9v!").is_none());
        let pem = "-----BEGIN RSA PUBLIC KEY-----\nZm9v\nYmFy\n-----END RSA PUBLIC KEY-----\n";
        assert_eq!(pem_body(pem).unwrap(), b"foobar");
    }

    #[test]
    fn chain_stops_at_the_last_index() {
        let mut c = SignedMessageChain::new(Uuid::nil(), Uuid::nil());
        c.next = Some(i32::MAX);
        assert_eq!(c.next_index(), Some(i32::MAX));
        // The index after MAX does not exist: the chain ends after this one.
        assert_eq!(i32::MAX.checked_add(1), None);
    }
}
