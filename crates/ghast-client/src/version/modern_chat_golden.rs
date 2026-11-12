//! Signed chat against the real client: `tests/golden/<version>/chat_signing.jsonl.gz` (oracle
//! `chat-signing`, made with `LocalChatSession`, `SignedMessageChain`, `LastSeenMessagesTracker`,
//! `SignableCommand` over the client's command dispatcher and the serverbound packet codecs).
//! Every packet body this wire layer writes — `chat_session_update`, `chat_message`,
//! `chat_command`, `chat_command_signed`, `message_acknowledgement` — must be byte-identical,
//! signatures included (SHA256withRSA PKCS#1 v1.5 is deterministic).

#![allow(clippy::field_reassign_with_default)] // the wire state is set up field by field

use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use bytes::Bytes;
use ghast_auth::chat::{ProfileKeys, SignedMessageChain, base64_decode};
use ghast_protocol::{FrameEncoder, RawPacket};
use serde_json::Value;

use super::*;

fn vectors() -> Vec<Value> {
    let path = format!(
        "{}/../../tests/golden/{}/chat_signing.jsonl.gz",
        env!("CARGO_MANIFEST_DIR"),
        super::super::GOLDEN_DIR
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&path).expect("golden file"))
        .read_to_string(&mut text)
        .expect("gzip");
    text.lines()
        .map(|l| serde_json::from_str(l).expect("json"))
        .collect()
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn b64(v: &Value) -> Vec<u8> {
    base64_decode(v.as_str().expect("string")).expect("base64")
}

/// Splits the frames the encoder wrote (no compression) into (id, body).
fn frames(enc: &mut FrameEncoder) -> Vec<(i32, Vec<u8>)> {
    let out = enc.out_mut().split().freeze();
    let mut r = &out[..];
    let mut v = Vec::new();
    let varint = |r: &mut &[u8]| {
        let mut n = 0i32;
        for i in 0..5 {
            let b = r[0];
            *r = &r[1..];
            n |= i32::from(b & 0x7f) << (7 * i);
            if b & 0x80 == 0 {
                break;
            }
        }
        n
    };
    while !r.is_empty() {
        let len = varint(&mut r) as usize;
        let (frame, rest) = r.split_at(len);
        let mut f = frame;
        let id = varint(&mut f);
        v.push((id, f.to_vec()));
        r = rest;
    }
    v
}

#[test]
fn signed_chat_matches_the_real_client() {
    let lines = vectors();
    let session = &lines[0];
    assert_eq!(session["type"], "session");
    let profile = uuid::Uuid::parse_str(session["profile_id"].as_str().unwrap()).unwrap();
    let session_id = uuid::Uuid::parse_str(session["session_id"].as_str().unwrap()).unwrap();
    let expires = UNIX_EPOCH + Duration::from_millis(session["expires_at_ms"].as_u64().unwrap());
    let keys = Arc::new(
        ProfileKeys::from_der(
            &b64(&session["private_pkcs8"]),
            &b64(&session["public_spki"]),
            b64(&session["key_signature"]),
            expires,
            expires,
        )
        .unwrap(),
    );
    let commands = &lines[1];
    assert_eq!(commands["type"], "commands");
    let tree = RawPacket::new(
        cb::id::DECLARE_COMMANDS,
        Bytes::from(hex(commands["body"].as_str().unwrap())),
    );

    let mut enc = FrameEncoder::new();
    let mut wire = Wire::default();
    // chat_session_update.
    wire.chat = Some(ChatSession {
        keys: keys.clone(),
        chain: SignedMessageChain::new(profile, session_id),
    });
    Marker::write_play(&mut enc, &Outbound::ChatSessionUpdate, &mut wire).unwrap();
    let f = frames(&mut enc);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].0, sb::id::CHAT_SESSION_UPDATE);
    assert_eq!(
        f[0].1,
        hex(session["session_update_body"].as_str().unwrap())
    );

    let (mut chats, mut cmds, mut signed_cmds, mut acks, mut recv) = (0, 0, 0, 0, 0);
    for chain in &lines[2..] {
        assert_eq!(chain["type"], "chain");
        let index = chain["index"].as_u64().unwrap();
        // A fresh tracker and encoder per chain (the session is the same).
        wire.seen = LastSeen::default();
        wire.chat = chain["signed"].as_bool().unwrap().then(|| ChatSession {
            keys: keys.clone(),
            chain: SignedMessageChain::new(profile, session_id),
        });
        wire.op_level = chain["op_level"].as_u64().unwrap() as u8;
        wire.commands = None;
        wire.commands_raw = Some(deflate(&tree));
        for (n, e) in chain["events"].as_array().unwrap().iter().enumerate() {
            let at = format!("chain {index} event {n}: {e}");
            if let Some(sig) = e.get("recv") {
                recv += 1;
                wire.seen
                    .add(Bytes::from(b64(sig)), e["shown"].as_bool().unwrap());
                Marker::write_wire_replies(&mut enc, &mut wire).unwrap();
                let f = frames(&mut enc);
                match e.get("ack_body") {
                    Some(b) => {
                        acks += 1;
                        assert_eq!(f.len(), 1, "{at}");
                        assert_eq!(f[0].0, sb::id::MESSAGE_ACKNOWLEDGEMENT, "{at}");
                        assert_eq!(f[0].1, hex(b.as_str().unwrap()), "{at}");
                    }
                    None => assert!(f.is_empty(), "{at}: unexpected {f:?}"),
                }
            } else if let Some(text) = e.get("chat") {
                chats += 1;
                wire.write_chat_at(
                    &mut enc,
                    text.as_str().unwrap(),
                    e["timestamp_ms"].as_i64().unwrap(),
                    e["salt"].as_i64().unwrap(),
                )
                .unwrap();
                let f = frames(&mut enc);
                assert_eq!(f.len(), 1, "{at}");
                assert_eq!(f[0].0, sb::id::CHAT_MESSAGE, "{at}");
                assert_eq!(f[0].1, hex(e["body"].as_str().unwrap()), "{at}");
            } else if let Some(cmd) = e.get("command") {
                let signed = e["packet"] == "chat_command_signed";
                let (ts, salt) = if signed {
                    signed_cmds += 1;
                    (
                        e["timestamp_ms"].as_i64().unwrap(),
                        e["salt"].as_i64().unwrap(),
                    )
                } else {
                    cmds += 1;
                    (0, 0)
                };
                wire.write_command_at(&mut enc, cmd.as_str().unwrap(), ts, salt)
                    .unwrap();
                let f = frames(&mut enc);
                assert_eq!(f.len(), 1, "{at}");
                let id = if signed {
                    sb::id::CHAT_COMMAND_SIGNED
                } else {
                    sb::id::CHAT_COMMAND
                };
                assert_eq!(f[0].0, id, "{at}");
                assert_eq!(f[0].1, hex(e["body"].as_str().unwrap()), "{at}");
            } else {
                panic!("{at}: unknown event");
            }
        }
    }
    eprintln!(
        "chat signing: {chats} chats, {signed_cmds} signed + {cmds} plain commands, {recv} received, {acks} acks: identical"
    );
    assert!(chats > 50 && signed_cmds > 30 && cmds > 30 && acks >= 2);
}

/// Login encryption against the real classes: `tests/golden/<version>/net_crypto.jsonl.gz`
/// (oracle `net-crypto`): the session hash, the `hello` packet (with `shouldAuthenticate`),
/// the key packet layout (both ways: the real client's packet decrypts to its secret and
/// challenge, ours to ours) and the AES/CFB8 streams of both directions.
#[test]
fn login_encryption_matches_the_real_client() {
    use ghast_protocol::crypto::{Cfb8Decryptor, Cfb8Encryptor};
    use rsa::pkcs8::DecodePrivateKey;
    use rsa::{Pkcs1v15Encrypt, RsaPrivateKey};

    let path = format!(
        "{}/../../tests/golden/{}/net_crypto.jsonl.gz",
        env!("CARGO_MANIFEST_DIR"),
        super::super::GOLDEN_DIR
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&path).expect("golden file"))
        .read_to_string(&mut text)
        .expect("gzip");
    let mut n = 0;
    for line in text.lines() {
        let c: Value = serde_json::from_str(line).unwrap();
        let at = format!("case {}", c["index"]);
        let der = b64(&c["server_public_der"]);
        let private = RsaPrivateKey::from_pkcs8_der(&b64(&c["server_private_pkcs8"])).unwrap();
        let secret: [u8; 16] = hex(c["shared_secret"].as_str().unwrap())
            .try_into()
            .unwrap();
        let challenge = hex(c["challenge"].as_str().unwrap());
        let server_id = c["server_id"].as_str().unwrap();
        assert_eq!(
            ghast_auth::crypto::server_hash(server_id, &secret, &der),
            c["server_hash"].as_str().unwrap(),
            "{at}"
        );
        // The hello packet as the client decodes it.
        let raw = RawPacket::new(
            login::clientbound::id::ENCRYPTION_BEGIN,
            Bytes::from(hex(c["hello_body"].as_str().unwrap())),
        );
        let (id, key, token) = match Marker::decode_login(&raw).unwrap().unwrap() {
            LoginInbound::EncryptionRequest {
                server_id,
                public_key,
                verify_token,
            } => {
                assert!(c["should_authenticate"].as_bool().unwrap(), "{at}");
                (server_id, public_key, verify_token)
            }
            LoginInbound::EncryptionRequestUnauthenticated {
                server_id,
                public_key,
                verify_token,
            } => {
                assert!(!c["should_authenticate"].as_bool().unwrap(), "{at}");
                (server_id, public_key, verify_token)
            }
            other => panic!("{at}: {other:?}"),
        };
        assert_eq!(
            (&*id, &key[..], &token[..]),
            (server_id, &der[..], &challenge[..]),
            "{at}"
        );
        // The real client's key packet: RSA PKCS#1 v1.5 of the secret and the challenge.
        let real: login::serverbound::EncryptionBegin = RawPacket::new(
            login::serverbound::id::ENCRYPTION_BEGIN,
            Bytes::from(hex(c["key_body"].as_str().unwrap())),
        )
        .decode_as()
        .unwrap();
        let dec = |b: &[u8]| private.decrypt(Pkcs1v15Encrypt, b).unwrap();
        assert_eq!(dec(&real.shared_secret), secret, "{at}");
        assert_eq!(dec(&real.verify_token), challenge, "{at}");
        // Ours, written by the client's login path, decrypts with the server key the same way.
        let ours = ghast_auth::crypto::respond_with_rng(
            &mut rand_core::OsRng,
            server_id,
            &der,
            &challenge,
        )
        .unwrap();
        let mut enc = FrameEncoder::new();
        Marker::write_encryption_response(
            &mut enc,
            &ours.encrypted_shared_secret,
            &ours.encrypted_verify_token,
        )
        .unwrap();
        let f = frames(&mut enc);
        let back: login::serverbound::EncryptionBegin =
            RawPacket::new(f[0].0, Bytes::from(f[0].1.clone()))
                .decode_as()
                .unwrap();
        assert_eq!(
            dec(&back.shared_secret),
            ours.shared_secret.as_bytes(),
            "{at}"
        );
        assert_eq!(dec(&back.verify_token), challenge, "{at}");
        // AES/CFB8, one stream per direction.
        let mut e = Cfb8Encryptor::new(&secret);
        for pair in c["encrypt"].as_array().unwrap() {
            let mut buf = hex(pair[0].as_str().unwrap());
            e.encrypt(&mut buf);
            assert_eq!(buf, hex(pair[1].as_str().unwrap()), "{at} encrypt");
        }
        let mut d = Cfb8Decryptor::new(&secret);
        for pair in c["decrypt"].as_array().unwrap() {
            let mut buf = hex(pair[0].as_str().unwrap());
            d.decrypt(&mut buf);
            assert_eq!(buf, hex(pair[1].as_str().unwrap()), "{at} decrypt");
        }
        n += 1;
    }
    assert!(n >= 12, "{n} cases");
}
