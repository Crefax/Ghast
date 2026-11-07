//! Client side of the login encryption handshake.
//!
//! When a server sends an Encryption Request (server id, DER public key, verify token) the client:
//!
//! 1. generates a random 16-byte [`SharedSecret`];
//! 2. encrypts the secret and the verify token with the server's RSA key (PKCS#1 v1.5) and sends
//!    both in the Encryption Response;
//! 3. in online mode, computes the [`server_hash`] and posts it to the session server's `join`
//!    endpoint *before* sending the Encryption Response;
//! 4. enables AES-128-CFB8 in both directions, with the shared secret as both key and IV.
//!
//! [`respond_to_encryption_request`] does steps 1 and 2 and computes the hash for step 3. Step 4
//! belongs to the protocol layer: key the cipher with [`SharedSecret::as_bytes`].
//!
//! ```
//! # fn demo(server_id: &str, public_key_der: &[u8], verify_token: &[u8])
//! #     -> Result<(), ghast_auth::crypto::CryptoError> {
//! use ghast_auth::crypto::respond_to_encryption_request;
//!
//! let response = respond_to_encryption_request(server_id, public_key_der, verify_token)?;
//! // send response.encrypted_shared_secret and response.encrypted_verify_token,
//! // then key AES-CFB8 with response.shared_secret.as_bytes()
//! # Ok(()) }
//! ```

use std::fmt;

use rand_core::{CryptoRngCore, OsRng, RngCore};
use rsa::pkcs8::DecodePublicKey;
use rsa::traits::PublicKeyParts;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
use sha1::{Digest, Sha1};
use thiserror::Error;

/// Length of the shared secret in bytes (AES-128).
pub const SHARED_SECRET_LEN: usize = 16;

/// Largest server key accepted, in bits. Vanilla uses 1024-bit keys; the limit stops a hostile
/// server from making the client do huge modular exponentiations.
pub const MAX_KEY_BITS: usize = 4096;

/// The 16-byte AES key/IV shared with the server.
///
/// `Debug` does not print the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct SharedSecret([u8; SHARED_SECRET_LEN]);

impl SharedSecret {
    /// Generates a secret from the operating system's CSPRNG.
    pub fn generate() -> Self {
        Self::generate_with(&mut OsRng)
    }

    /// Generates a secret from the given cryptographically secure RNG.
    pub fn generate_with(rng: &mut impl CryptoRngCore) -> Self {
        let mut bytes = [0u8; SHARED_SECRET_LEN];
        rng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// Wraps existing bytes (for tests and replays).
    pub fn from_bytes(bytes: [u8; SHARED_SECRET_LEN]) -> Self {
        Self(bytes)
    }

    /// The raw secret: the AES-128-CFB8 key and IV for both directions.
    pub fn as_bytes(&self) -> &[u8; SHARED_SECRET_LEN] {
        &self.0
    }
}

impl fmt::Debug for SharedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedSecret([redacted])")
    }
}

/// Errors from the login crypto.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CryptoError {
    /// The server's public key is not a valid DER `SubjectPublicKeyInfo` RSA key.
    #[error("invalid server public key: {0}")]
    InvalidPublicKey(String),
    /// The key is larger than [`MAX_KEY_BITS`].
    #[error("server public key has {0} bits, the maximum is {MAX_KEY_BITS}")]
    KeyTooLarge(usize),
    /// RSA encryption failed (for example a verify token too long for the key).
    #[error("RSA encryption failed: {0}")]
    Encrypt(#[source] rsa::Error),
}

/// A server's RSA public key, parsed from the DER `SubjectPublicKeyInfo` bytes of the Encryption
/// Request.
#[derive(Clone, PartialEq, Eq)]
pub struct ServerPublicKey {
    key: RsaPublicKey,
    der: Vec<u8>,
}

impl ServerPublicKey {
    /// Parses a DER `SubjectPublicKeyInfo` RSA public key.
    pub fn from_der(der: &[u8]) -> Result<Self, CryptoError> {
        let key = RsaPublicKey::from_public_key_der(der)
            .map_err(|e| CryptoError::InvalidPublicKey(e.to_string()))?;
        let bits = key.size() * 8;
        if bits > MAX_KEY_BITS {
            return Err(CryptoError::KeyTooLarge(bits));
        }
        Ok(Self {
            key,
            der: der.to_vec(),
        })
    }

    /// The DER bytes as received (they are part of the server hash).
    pub fn der(&self) -> &[u8] {
        &self.der
    }

    /// The modulus size in bits.
    pub fn bits(&self) -> usize {
        self.key.size() * 8
    }

    /// Encrypts `data` with RSA PKCS#1 v1.5.
    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        self.encrypt_with(&mut OsRng, data)
    }

    /// Encrypts `data` with RSA PKCS#1 v1.5, taking the padding randomness from `rng`.
    pub fn encrypt_with(
        &self,
        rng: &mut impl CryptoRngCore,
        data: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        self.key
            .encrypt(rng, Pkcs1v15Encrypt, data)
            .map_err(CryptoError::Encrypt)
    }
}

impl fmt::Debug for ServerPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerPublicKey")
            .field("bits", &self.bits())
            .finish_non_exhaustive()
    }
}

/// Everything the client needs after an Encryption Request.
#[derive(Debug, Clone)]
pub struct EncryptionResponse {
    /// The fresh secret. Key AES-128-CFB8 with it once the response is sent.
    pub shared_secret: SharedSecret,
    /// The secret encrypted with the server key (Encryption Response field 1).
    pub encrypted_shared_secret: Vec<u8>,
    /// The verify token encrypted with the server key (Encryption Response field 2).
    pub encrypted_verify_token: Vec<u8>,
    /// The session-server hash for the `join` call (only needed in online mode).
    pub server_hash: String,
}

/// Handles an Encryption Request: generates the shared secret, encrypts it and the verify token,
/// and computes the server hash.
pub fn respond_to_encryption_request(
    server_id: &str,
    public_key_der: &[u8],
    verify_token: &[u8],
) -> Result<EncryptionResponse, CryptoError> {
    respond_with_rng(&mut OsRng, server_id, public_key_der, verify_token)
}

/// [`respond_to_encryption_request`] with an explicit RNG (deterministic tests).
pub fn respond_with_rng(
    rng: &mut impl CryptoRngCore,
    server_id: &str,
    public_key_der: &[u8],
    verify_token: &[u8],
) -> Result<EncryptionResponse, CryptoError> {
    let key = ServerPublicKey::from_der(public_key_der)?;
    let shared_secret = SharedSecret::generate_with(rng);
    let encrypted_shared_secret = key.encrypt_with(rng, shared_secret.as_bytes())?;
    let encrypted_verify_token = key.encrypt_with(rng, verify_token)?;
    let server_hash = server_hash(server_id, shared_secret.as_bytes(), key.der());
    Ok(EncryptionResponse {
        shared_secret,
        encrypted_shared_secret,
        encrypted_verify_token,
        server_hash,
    })
}

/// The session-server hash: SHA-1 over the server id (ISO-8859-1), the shared secret and the
/// server's DER public key, printed as a signed two's-complement hex number (Java's
/// `new BigInteger(digest).toString(16)`).
pub fn server_hash(server_id: &str, shared_secret: &[u8], public_key_der: &[u8]) -> String {
    let mut sha = Sha1::new();
    // Java encodes the id with ISO-8859-1; unmappable characters become '?'.
    let id: Vec<u8> = server_id
        .chars()
        .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
        .collect();
    sha.update(&id);
    sha.update(shared_secret);
    sha.update(public_key_der);
    signed_hex_digest(sha.finalize().into())
}

/// Formats a SHA-1 digest the way Minecraft does: as a signed big-endian two's-complement
/// number in lowercase hex, without leading zeros, with a `-` for negative values.
///
/// ```
/// use ghast_auth::crypto::signed_hex_digest;
/// use sha1::{Digest, Sha1};
///
/// let digest: [u8; 20] = Sha1::digest(b"jeb_").into();
/// assert_eq!(signed_hex_digest(digest), "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1");
/// ```
pub fn signed_hex_digest(mut digest: [u8; 20]) -> String {
    let negative = digest[0] & 0x80 != 0;
    if negative {
        // Two's-complement negation: invert, then add one.
        let mut carry = true;
        for byte in digest.iter_mut().rev() {
            let (v, c) = (!*byte).overflowing_add(u8::from(carry));
            *byte = v;
            carry = c;
        }
    }
    let mut out = String::with_capacity(41);
    if negative {
        out.push('-');
    }
    let mut started = false;
    for byte in digest {
        for nibble in [byte >> 4, byte & 0x0f] {
            if nibble != 0 || started {
                started = true;
                out.push(char::from_digit(u32::from(nibble), 16).unwrap_or('0'));
            }
        }
    }
    if !started {
        out.push('0');
    }
    out
}

/// Fills `buf` with random bytes from the OS CSPRNG (re-exported convenience, for example for
/// verify tokens in test servers).
pub fn random_bytes(buf: &mut [u8]) {
    OsRng.fill_bytes(buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::RsaPrivateKey;
    use rsa::pkcs8::EncodePublicKey;

    fn sha1_hex(s: &str) -> String {
        signed_hex_digest(Sha1::digest(s.as_bytes()).into())
    }

    #[test]
    fn known_digest_vectors() {
        // The classic vectors from wiki.vg, checked against Java's BigInteger.toString(16).
        assert_eq!(
            sha1_hex("Notch"),
            "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48"
        );
        assert_eq!(
            sha1_hex("jeb_"),
            "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1"
        );
        assert_eq!(sha1_hex("simon"), "88e16a1019277b15d58faf0541e11910eb756f6");
    }

    #[test]
    fn digest_edge_cases() {
        assert_eq!(signed_hex_digest([0; 20]), "0");
        let mut one = [0u8; 20];
        one[19] = 1;
        assert_eq!(signed_hex_digest(one), "1");
        assert_eq!(signed_hex_digest([0xff; 20]), "-1");
        let mut min = [0u8; 20];
        min[0] = 0x80;
        assert_eq!(signed_hex_digest(min), format!("-8{}", "0".repeat(39)));
    }

    #[test]
    fn full_server_hash_matches_java() {
        // Generated with Java: SHA-1("" + bytes 0..16 + key bytes (i*7+3) for i in 0..162),
        // then new BigInteger(digest).toString(16).
        let secret: Vec<u8> = (0..16).collect();
        let key: Vec<u8> = (0..162u32).map(|i| (i * 7 + 3) as u8).collect();
        assert_eq!(
            server_hash("", &secret, &key),
            "-50a7c2d3cac900829c4a471fc66f28a9477ec7ae"
        );
    }

    #[test]
    fn server_id_is_latin1() {
        let s = [0u8; 16];
        assert_eq!(server_hash("\u{e9}", &s, &[]), {
            let mut sha = Sha1::new();
            sha.update([0xe9]);
            sha.update(s);
            signed_hex_digest(sha.finalize().into())
        });
        assert_eq!(server_hash("\u{20ac}", &s, &[]), server_hash("?", &s, &[]));
    }

    #[test]
    fn encrypt_round_trip() {
        let mut rng = OsRng;
        let private = RsaPrivateKey::new(&mut rng, 1024).unwrap();
        let der = private.to_public_key().to_public_key_der().unwrap();
        let token = [1u8, 2, 3, 4];
        let resp = respond_to_encryption_request("", der.as_bytes(), &token).unwrap();
        assert_eq!(resp.encrypted_shared_secret.len(), 128);
        assert_eq!(resp.encrypted_verify_token.len(), 128);
        let secret = private
            .decrypt(Pkcs1v15Encrypt, &resp.encrypted_shared_secret)
            .unwrap();
        assert_eq!(secret, resp.shared_secret.as_bytes());
        let tok = private
            .decrypt(Pkcs1v15Encrypt, &resp.encrypted_verify_token)
            .unwrap();
        assert_eq!(tok, token);
        assert_eq!(
            resp.server_hash,
            server_hash("", resp.shared_secret.as_bytes(), der.as_bytes())
        );
        assert_eq!(
            format!("{:?}", resp.shared_secret),
            "SharedSecret([redacted])"
        );
    }

    #[test]
    fn rejects_bad_keys() {
        assert!(matches!(
            ServerPublicKey::from_der(&[]),
            Err(CryptoError::InvalidPublicKey(_))
        ));
        assert!(matches!(
            ServerPublicKey::from_der(&[0x30, 0x82, 0xff, 0xff, 0x00]),
            Err(CryptoError::InvalidPublicKey(_))
        ));
    }

    #[test]
    fn secrets_are_random() {
        assert_ne!(SharedSecret::generate(), SharedSecret::generate());
    }
}
