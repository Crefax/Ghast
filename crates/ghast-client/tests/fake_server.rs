//! The client against an in-process fake server, for both protocol versions: login
//! (offline, compression, refusals), the play state (keep-alive, teleport, kick, close,
//! silence, malformed packets) and the tick order on the wire.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::{BufMut, Bytes, BytesMut};
use ghast_client::connect::TargetAddr;
use ghast_client::version::{ClientVersion, V1_8, V1_16_5};
use ghast_client::{
    Account, Behavior, Client, ClientError, Controls, GameVersion, Idle, Interests, SessionEnd,
    TickContext,
};
use ghast_protocol::nbt::RawNbt;
use ghast_protocol::types::{ByteString, Uuid};
use ghast_protocol::{FrameDecoder, FrameEncoder, PacketSet, RawPacket};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// ---- version-specific packet builders for the fake server ---------------------------

trait Fake: ClientVersion {
    fn login_compress(threshold: i32) -> RawPacket;
    fn login_success(name: &str) -> RawPacket;
    fn login_disconnect(json: &str) -> RawPacket;
    fn encryption_request() -> RawPacket;
    fn join(entity_id: i32) -> RawPacket;
    fn position(x: f64, y: f64, z: f64, teleport_id: i32) -> RawPacket;
    fn keep_alive(id: i64) -> RawPacket;
    fn kick(json: &str) -> RawPacket;
    fn chat(json: &str) -> RawPacket;
    fn chunk(x: i32, z: i32) -> RawPacket;
    /// Play-state set compression (1.8 only).
    fn play_compress(threshold: i32) -> Option<RawPacket>;
    fn transaction(action: i16) -> RawPacket;
    fn sb_play_name(raw: &RawPacket) -> &'static str;
    fn handshake_protocol(raw: &RawPacket) -> i32;
    fn login_start_name(raw: &RawPacket) -> String;
}

fn raw<P: ghast_protocol::Packet>(p: &P) -> RawPacket {
    RawPacket::from_packet(p).unwrap()
}

impl Fake for V1_8 {
    fn login_compress(threshold: i32) -> RawPacket {
        raw(&ghast_protocol::v1_8::login::clientbound::Compress { threshold })
    }
    fn login_success(name: &str) -> RawPacket {
        raw(&ghast_protocol::v1_8::login::clientbound::Success {
            uuid: ByteString::from(ghast_auth::offline::offline_uuid(name).to_string()),
            username: ByteString::from(name),
        })
    }
    fn login_disconnect(json: &str) -> RawPacket {
        raw(&ghast_protocol::v1_8::login::clientbound::Disconnect {
            reason: ByteString::from(json),
        })
    }
    fn encryption_request() -> RawPacket {
        raw(&ghast_protocol::v1_8::login::clientbound::EncryptionBegin {
            server_id: ByteString::from(""),
            public_key: Bytes::from_static(&[1, 2, 3]),
            verify_token: Bytes::from_static(&[4, 5, 6, 7]),
        })
    }
    fn join(entity_id: i32) -> RawPacket {
        raw(&ghast_protocol::v1_8::play::clientbound::Login {
            entity_id,
            game_mode: 0,
            dimension: 0,
            difficulty: 1,
            max_players: 20,
            level_type: ByteString::from("default"),
            reduced_debug_info: false,
        })
    }
    fn position(x: f64, y: f64, z: f64, _teleport_id: i32) -> RawPacket {
        raw(&ghast_protocol::v1_8::play::clientbound::Position {
            x,
            y,
            z,
            yaw: 45.0,
            pitch: 10.0,
            flags: 0,
        })
    }
    fn keep_alive(id: i64) -> RawPacket {
        raw(&ghast_protocol::v1_8::play::clientbound::KeepAlive {
            keep_alive_id: id as i32,
        })
    }
    fn kick(json: &str) -> RawPacket {
        raw(&ghast_protocol::v1_8::play::clientbound::KickDisconnect {
            reason: ByteString::from(json),
        })
    }
    fn chat(json: &str) -> RawPacket {
        raw(&ghast_protocol::v1_8::play::clientbound::Chat {
            message: ByteString::from(json),
            position: 1,
        })
    }
    /// One section (y 48–63) with a stone top layer: the spawn at y = 64 stands on it.
    fn chunk(x: i32, z: i32) -> RawPacket {
        let mut d = BytesMut::with_capacity(12_544);
        for i in 0..4096 {
            d.put_u16_le(if i >> 8 == 15 { 1 << 4 } else { 0 });
        }
        d.put_bytes(0, 2048);
        d.put_bytes(0xff, 2048);
        d.put_bytes(1, 256);
        raw(&ghast_protocol::v1_8::play::clientbound::MapChunk {
            x,
            z,
            ground_up: true,
            bit_map: 1 << 3,
            chunk_data: d.freeze(),
        })
    }
    fn play_compress(threshold: i32) -> Option<RawPacket> {
        Some(raw(
            &ghast_protocol::v1_8::play::clientbound::SetCompression { threshold },
        ))
    }
    fn transaction(action: i16) -> RawPacket {
        raw(&ghast_protocol::v1_8::play::clientbound::Transaction {
            window_id: 0,
            action,
            accepted: false,
        })
    }
    fn sb_play_name(raw: &RawPacket) -> &'static str {
        ghast_protocol::v1_8::play::serverbound::ServerboundPlayPacket::decode_raw(raw)
            .expect("valid serverbound packet")
            .name()
    }
    fn handshake_protocol(raw: &RawPacket) -> i32 {
        raw.decode_as::<ghast_protocol::v1_8::handshaking::serverbound::SetProtocol>()
            .unwrap()
            .protocol_version
    }
    fn login_start_name(raw: &RawPacket) -> String {
        raw.decode_as::<ghast_protocol::v1_8::login::serverbound::LoginStart>()
            .unwrap()
            .username
            .to_string()
    }
}

fn empty_compound() -> RawNbt {
    RawNbt::from_bytes(Bytes::from_static(&[10, 0, 0, 0])).unwrap()
}

impl Fake for V1_16_5 {
    fn login_compress(threshold: i32) -> RawPacket {
        raw(&ghast_protocol::v1_16_5::login::clientbound::Compress { threshold })
    }
    fn login_success(name: &str) -> RawPacket {
        raw(&ghast_protocol::v1_16_5::login::clientbound::Success {
            uuid: ghast_auth::offline::offline_uuid(name),
            username: ByteString::from(name),
        })
    }
    fn login_disconnect(json: &str) -> RawPacket {
        raw(&ghast_protocol::v1_16_5::login::clientbound::Disconnect {
            reason: ByteString::from(json),
        })
    }
    fn encryption_request() -> RawPacket {
        raw(
            &ghast_protocol::v1_16_5::login::clientbound::EncryptionBegin {
                server_id: ByteString::from(""),
                public_key: Bytes::from_static(&[1, 2, 3]),
                verify_token: Bytes::from_static(&[4, 5, 6, 7]),
            },
        )
    }
    fn join(entity_id: i32) -> RawPacket {
        raw(&ghast_protocol::v1_16_5::play::clientbound::Login {
            entity_id,
            is_hardcore: false,
            game_mode: 0,
            previous_game_mode: 255,
            world_names: vec![ByteString::from("minecraft:overworld")],
            dimension_codec: empty_compound(),
            dimension: empty_compound(),
            world_name: ByteString::from("minecraft:overworld"),
            hashed_seed: 0,
            max_players: 20,
            view_distance: 8,
            reduced_debug_info: false,
            enable_respawn_screen: true,
            is_debug: false,
            is_flat: false,
        })
    }
    fn position(x: f64, y: f64, z: f64, teleport_id: i32) -> RawPacket {
        raw(&ghast_protocol::v1_16_5::play::clientbound::Position {
            x,
            y,
            z,
            yaw: 45.0,
            pitch: 10.0,
            flags: 0,
            teleport_id,
        })
    }
    fn keep_alive(id: i64) -> RawPacket {
        raw(&ghast_protocol::v1_16_5::play::clientbound::KeepAlive { keep_alive_id: id })
    }
    fn kick(json: &str) -> RawPacket {
        raw(
            &ghast_protocol::v1_16_5::play::clientbound::KickDisconnect {
                reason: ByteString::from(json),
            },
        )
    }
    fn chat(json: &str) -> RawPacket {
        raw(&ghast_protocol::v1_16_5::play::clientbound::Chat {
            message: ByteString::from(json),
            position: 1,
            sender: Uuid::nil(),
        })
    }
    /// A full chunk without sections (1.16.5 uses the stand-still mover).
    fn chunk(x: i32, z: i32) -> RawPacket {
        raw(&ghast_protocol::v1_16_5::play::clientbound::MapChunk {
            x,
            z,
            ground_up: true,
            bit_map: 0,
            heightmaps: empty_compound(),
            biomes: Some(vec![1; 1024]),
            chunk_data: Bytes::new(),
            block_entities: Vec::new(),
        })
    }
    fn play_compress(_threshold: i32) -> Option<RawPacket> {
        None
    }
    fn transaction(action: i16) -> RawPacket {
        raw(&ghast_protocol::v1_16_5::play::clientbound::Transaction {
            window_id: 0,
            action,
            accepted: false,
        })
    }
    fn sb_play_name(raw: &RawPacket) -> &'static str {
        ghast_protocol::v1_16_5::play::serverbound::ServerboundPlayPacket::decode_raw(raw)
            .expect("valid serverbound packet")
            .name()
    }
    fn handshake_protocol(raw: &RawPacket) -> i32 {
        raw.decode_as::<ghast_protocol::v1_16_5::handshaking::serverbound::SetProtocol>()
            .unwrap()
            .protocol_version
    }
    fn login_start_name(raw: &RawPacket) -> String {
        raw.decode_as::<ghast_protocol::v1_16_5::login::serverbound::LoginStart>()
            .unwrap()
            .username
            .to_string()
    }
}

// ---- the fake server ------------------------------------------------------------------

struct Conn {
    stream: TcpStream,
    dec: FrameDecoder,
    enc: FrameEncoder,
}

impl Conn {
    async fn recv(&mut self) -> Option<RawPacket> {
        loop {
            if let Some(p) = self.dec.next_packet().unwrap() {
                return Some(p);
            }
            let mut buf = [0u8; 4096];
            let n = tokio::time::timeout(Duration::from_secs(5), self.stream.read(&mut buf))
                .await
                .expect("client went quiet")
                .ok()?;
            if n == 0 {
                return None;
            }
            self.dec.push(&buf[..n]);
        }
    }

    async fn send(&mut self, p: &RawPacket) {
        self.enc.write_raw(p).unwrap();
        self.flush().await;
    }

    async fn flush(&mut self) {
        let out = self.enc.take();
        let _ = self.stream.write_all(&out).await;
    }

    async fn send_bytes(&mut self, b: &[u8]) {
        let _ = self.stream.write_all(b).await;
    }

    /// Handshake + login start; checks the protocol and name.
    async fn accept_login<V: Fake>(&mut self, name: &str) {
        let hs = self.recv().await.unwrap();
        assert_eq!(hs.id, 0);
        assert_eq!(V::handshake_protocol(&hs), V::PROTOCOL);
        let ls = self.recv().await.unwrap();
        assert_eq!(V::login_start_name(&ls), name);
    }

    /// Full offline login with compression.
    async fn login<V: Fake>(&mut self, name: &str, threshold: i32) {
        self.accept_login::<V>(name).await;
        self.send(&V::login_compress(threshold)).await;
        self.enc.set_compression(threshold);
        self.dec.set_compression(threshold);
        self.send(&V::login_success(name)).await;
    }

    /// Reads play packets until one named `name`; returns the names seen before it.
    async fn until<V: Fake>(&mut self, name: &str) -> Vec<&'static str> {
        let mut seen = Vec::new();
        loop {
            let p = self
                .recv()
                .await
                .unwrap_or_else(|| panic!("closed before {name}; saw {seen:?}"));
            let n = V::sb_play_name(&p);
            if n == name {
                return seen;
            }
            seen.push(n);
        }
    }
}

async fn listen() -> (TcpListener, TargetAddr) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    (l, TargetAddr::from(addr))
}

async fn accept(l: &TcpListener) -> Conn {
    let (stream, _) = l.accept().await.unwrap();
    Conn {
        stream,
        dec: FrameDecoder::new(),
        enc: FrameEncoder::new(),
    }
}

fn client(addr: TargetAddr, v: GameVersion, name: &str) -> Client {
    Client::new(addr, v, Account::offline(name).unwrap())
        .read_timeout(Duration::from_secs(5))
        .login_timeout(Duration::from_secs(5))
}

/// Join, chunk and spawn position: the client is in the world after this.
async fn spawn<V: Fake>(c: &mut Conn) {
    c.send(&V::join(42)).await;
    c.send(&V::chunk(0, 0)).await;
    c.send(&V::position(8.5, 64.0, 8.5, 1)).await;
}

// ---- tests ------------------------------------------------------------------------------

/// Runs a test body for both versions.
macro_rules! both {
    ($name:ident, $body:ident) => {
        mod $name {
            use super::*;
            #[tokio::test]
            async fn v1_8() {
                $body::<V1_8>(GameVersion::V1_8_9).await;
            }
            #[tokio::test]
            async fn v1_16_5() {
                $body::<V1_16_5>(GameVersion::V1_16_5).await;
            }
        }
    };
}

async fn login_join_keepalive_kick<V: Fake>(v: GameVersion) {
    let (l, addr) = listen().await;
    let server = async {
        let mut c = accept(&l).await;
        c.login::<V>("Tester", 64).await;
        spawn::<V>(&mut c).await;
        // Settings and brand right after join, then the teleport answer.
        let first: Vec<_> = {
            let mut v = Vec::new();
            for _ in 0..3 {
                v.push(V::sb_play_name(&c.recv().await.unwrap()));
            }
            v
        };
        assert_eq!(
            first,
            [
                "settings",
                "custom_payload",
                if V::PROTOCOL == 47 {
                    "position_look"
                } else {
                    "teleport_confirm"
                }
            ]
        );
        if V::PROTOCOL != 47 {
            assert_eq!(V::sb_play_name(&c.recv().await.unwrap()), "position_look");
        }
        // Mid-stream compression change (1.8 play-state packet).
        if let Some(p) = V::play_compress(256) {
            c.send(&p).await;
            c.enc.set_compression(256);
            c.dec.set_compression(256);
        }
        c.send(&V::keep_alive(1234)).await;
        let before = c.until::<V>("keep_alive").await;
        assert!(
            before
                .iter()
                .all(|n| *n == "flying" || n.starts_with("position")),
            "{before:?}"
        );
        c.send(&V::transaction(-7)).await;
        c.until::<V>("transaction").await;
        c.send(&V::kick(r#"{"text":"bye ","extra":[{"text":"now"}]}"#))
            .await;
    };
    let cl = client(addr, v, "Tester");
    let mut idle = Idle;
    let (end, ()) = tokio::join!(cl.run(&mut idle), server);
    match end.unwrap() {
        SessionEnd::Kicked { reason, .. } => assert_eq!(reason, "bye now"),
        e => panic!("unexpected end {e:?}"),
    }
}
both!(login_join_keepalive_kick, login_join_keepalive_kick);

async fn login_refused<V: Fake>(v: GameVersion) {
    let (l, addr) = listen().await;
    let server = async {
        let mut c = accept(&l).await;
        c.accept_login::<V>("Tester").await;
        c.send(&V::login_disconnect(r#"{"text":"You are banned"}"#))
            .await;
    };
    let cl = client(addr, v, "Tester");
    let mut idle = Idle;
    let (end, ()) = tokio::join!(cl.run(&mut idle), server);
    match end {
        Err(ClientError::LoginRefused(r)) => assert_eq!(r, "You are banned"),
        e => panic!("unexpected {e:?}"),
    }
}
both!(login_refused, login_refused);

async fn online_mode_needs_an_account<V: Fake>(v: GameVersion) {
    let (l, addr) = listen().await;
    let server = async {
        let mut c = accept(&l).await;
        c.accept_login::<V>("Tester").await;
        c.send(&V::encryption_request()).await;
        // The client gives up without answering.
        assert!(c.recv().await.is_none());
    };
    let cl = client(addr, v, "Tester");
    let mut idle = Idle;
    let (end, ()) = tokio::join!(cl.run(&mut idle), server);
    assert!(
        matches!(end, Err(ClientError::OnlineModeRequired)),
        "{end:?}"
    );
}
both!(online_mode_needs_an_account, online_mode_needs_an_account);

async fn server_closes<V: Fake>(v: GameVersion) {
    let (l, addr) = listen().await;
    let server = async {
        let mut c = accept(&l).await;
        c.login::<V>("Tester", 256).await;
        spawn::<V>(&mut c).await;
        c.until::<V>("position_look").await;
        drop(c);
    };
    let cl = client(addr, v, "Tester");
    let mut idle = Idle;
    let (end, ()) = tokio::join!(cl.run(&mut idle), server);
    assert!(matches!(end, Ok(SessionEnd::ServerClosed)), "{end:?}");
}
both!(server_closes, server_closes);

async fn closed_during_login<V: Fake>(v: GameVersion) {
    let (l, addr) = listen().await;
    let server = async {
        let mut c = accept(&l).await;
        c.accept_login::<V>("Tester").await;
        drop(c);
    };
    let cl = client(addr, v, "Tester");
    let mut idle = Idle;
    let (end, ()) = tokio::join!(cl.run(&mut idle), server);
    assert!(matches!(end, Err(ClientError::LoginClosed)), "{end:?}");
}
both!(closed_during_login, closed_during_login);

async fn malformed_packet<V: Fake>(v: GameVersion) {
    let (l, addr) = listen().await;
    let server = async {
        let mut c = accept(&l).await;
        c.login::<V>("Tester", -1).await;
        spawn::<V>(&mut c).await;
        // A keep-alive with a trailing byte: strict decoding rejects it.
        let mut p = V::keep_alive(1);
        let mut body = BytesMut::from(&p.body[..]);
        body.put_u8(0xaa);
        p.body = body.freeze();
        c.send(&p).await;
        while c.recv().await.is_some() {}
    };
    let cl = client(addr, v, "Tester");
    let mut idle = Idle;
    let (end, ()) = tokio::join!(cl.run(&mut idle), server);
    assert!(matches!(end, Err(ClientError::Decode { .. })), "{end:?}");
}
both!(malformed_packet, malformed_packet);

async fn garbage_frame<V: Fake>(v: GameVersion) {
    let (l, addr) = listen().await;
    let server = async {
        let mut c = accept(&l).await;
        c.login::<V>("Tester", 16).await;
        spawn::<V>(&mut c).await;
        // A compressed frame whose zlib stream is garbage.
        c.send_bytes(&[0x06, 0x40, 0xde, 0xad, 0xbe, 0xef, 0x00])
            .await;
        while c.recv().await.is_some() {}
    };
    let cl = client(addr, v, "Tester");
    let mut idle = Idle;
    let (end, ()) = tokio::join!(cl.run(&mut idle), server);
    assert!(matches!(end, Err(ClientError::Frame(_))), "{end:?}");
}
both!(garbage_frame, garbage_frame);

async fn read_timeout<V: Fake>(v: GameVersion) {
    let (l, addr) = listen().await;
    let server = async {
        let mut c = accept(&l).await;
        c.login::<V>("Tester", 256).await;
        spawn::<V>(&mut c).await;
        // Silent from now on; keep the socket open and drain what the client writes.
        while c.recv().await.is_some() {}
    };
    let cl = client(addr, v, "Tester").read_timeout(Duration::from_millis(600));
    let started = std::time::Instant::now();
    let mut idle = Idle;
    let (end, ()) = tokio::join!(cl.run(&mut idle), server);
    assert!(matches!(end, Err(ClientError::Timeout(_))), "{end:?}");
    assert!(started.elapsed() < Duration::from_secs(3));
}
both!(read_timeout, read_timeout);

/// Counts behaviour ticks; sends a chat line on tick 5 and quits on tick 60.
struct Counter {
    ticks: Arc<AtomicU64>,
    quit_at: u64,
}

impl Behavior for Counter {
    fn interests(&self) -> Interests {
        Interests::CHAT
    }
    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        let n = self.ticks.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 5 {
            ctx.chat("hello");
            ctx.set_controls(Controls {
                sneak: true,
                ..Controls::default()
            });
        }
        if n >= self.quit_at {
            ctx.disconnect("done");
        }
    }
}

async fn tick_order_on_the_wire<V: Fake>(v: GameVersion) {
    let (l, addr) = listen().await;
    let ticks = Arc::new(AtomicU64::new(0));
    let server = async {
        let mut c = accept(&l).await;
        c.login::<V>("Tester", 128).await;
        spawn::<V>(&mut c).await;
        c.until::<V>("position_look").await;
        // Every tick is exactly one movement packet; the chat of the behaviour's 5th tick
        // comes after the 4th movement packet and before that tick's own, followed by the
        // sneak entity_action.
        let mut ticks_seen = 0;
        let mut order = Vec::new();
        loop {
            let Some(p) = c.recv().await else { break };
            let n = V::sb_play_name(&p);
            match n {
                "flying" | "position" | "look" | "position_look" => ticks_seen += 1,
                _ => order.push((ticks_seen, n)),
            }
        }
        (ticks_seen, order)
    };
    let mut b = Counter {
        ticks: ticks.clone(),
        quit_at: 40,
    };
    // The fake server sends nothing after the spawn: keep the silent-server guard out of it.
    let cl = client(addr, v, "Tester").silent_hold(Duration::from_secs(10));
    let started = std::time::Instant::now();
    let (end, (movement, order)) = tokio::join!(cl.run(&mut b), server);
    let elapsed = started.elapsed();
    assert!(
        matches!(end, Ok(SessionEnd::Quit(ref r)) if r == "done"),
        "{end:?}"
    );
    assert_eq!(ticks.load(Ordering::Relaxed), 40);
    // 40 behaviour ticks = 40 movement packets.
    assert_eq!(movement, 40, "{order:?}");
    assert_eq!(order, [(4, "chat"), (4, "entity_action")]);
    // ~50 ms per tick.
    assert!(
        elapsed >= Duration::from_millis(40 * 50 - 100),
        "{elapsed:?}"
    );
}
both!(tick_order_on_the_wire, tick_order_on_the_wire);

async fn silent_server_freezes_behaviour<V: Fake>(v: GameVersion) {
    let (l, addr) = listen().await;
    let ticks = Arc::new(AtomicU64::new(0));
    let server = async {
        let mut c = accept(&l).await;
        c.login::<V>("Tester", 128).await;
        spawn::<V>(&mut c).await;
        c.until::<V>("position_look").await;
        // Silent for 1 s: movement packets keep coming.
        let mut moves = 0;
        let t = std::time::Instant::now();
        while t.elapsed() < Duration::from_millis(1000) {
            let n = V::sb_play_name(&c.recv().await.unwrap());
            if n == "flying" || n.starts_with("position") || n == "look" {
                moves += 1;
            }
        }
        assert!(moves >= 15, "movement continues while silent: {moves}");
        c.send(&V::kick(r#"{"text":"x"}"#)).await;
    };
    let mut b = Counter {
        ticks: ticks.clone(),
        quit_at: u64::MAX,
    };
    let cl = client(addr, v, "Tester").silent_hold(Duration::from_millis(200));
    let (end, ()) = tokio::join!(cl.run(&mut b), server);
    assert!(matches!(end, Ok(SessionEnd::Kicked { .. })), "{end:?}");
    // About 200 ms of behaviour ticks, not 1 s.
    let n = ticks.load(Ordering::Relaxed);
    assert!((2..=8).contains(&n), "behaviour ticks while silent: {n}");
}
both!(
    silent_server_freezes_behaviour,
    silent_server_freezes_behaviour
);

async fn shutdown_is_clean<V: Fake>(v: GameVersion) {
    let (l, addr) = listen().await;
    let server = async {
        let mut c = accept(&l).await;
        c.login::<V>("Tester", 128).await;
        spawn::<V>(&mut c).await;
        // Reads until the client's FIN.
        while c.recv().await.is_some() {}
    };
    let cl = client(addr, v, "Tester");
    let shutdown = tokio::time::sleep(Duration::from_millis(300));
    let mut idle = Idle;
    let (end, ()) = tokio::join!(cl.run_until(&mut idle, shutdown), server);
    assert!(matches!(end, Ok(SessionEnd::Shutdown)), "{end:?}");
}
both!(shutdown_is_clean, shutdown_is_clean);

async fn chat_reaches_the_behaviour<V: Fake>(v: GameVersion) {
    struct Listen(Vec<String>);
    impl Behavior for Listen {
        fn interests(&self) -> Interests {
            Interests::CHAT
        }
        fn tick(&mut self, ctx: &mut TickContext<'_>) {
            self.0.extend(ctx.events().chat.iter().map(|m| m.plain()));
            if self.0.len() >= 2 {
                ctx.disconnect("got it");
            }
        }
    }
    let (l, addr) = listen().await;
    let server = async {
        let mut c = accept(&l).await;
        c.login::<V>("Tester", 128).await;
        // Chat before the spawn position is kept for the first tick.
        c.send(&V::join(42)).await;
        c.send(&V::chat(r#"{"text":"§awelcome"}"#)).await;
        c.send(&V::chunk(0, 0)).await;
        c.send(&V::position(8.5, 64.0, 8.5, 1)).await;
        c.until::<V>("position_look").await;
        c.send(&V::chat(
            r#"{"translate":"chat.type.text","with":["Bob","hi"]}"#,
        ))
        .await;
        while c.recv().await.is_some() {}
    };
    let mut b = Listen(Vec::new());
    let cl = client(addr, v, "Tester");
    let (end, ()) = tokio::join!(cl.run(&mut b), server);
    assert!(matches!(end, Ok(SessionEnd::Quit(_))), "{end:?}");
    assert_eq!(b.0[0], "welcome");
    assert!(b.0[1].contains("Bob") && b.0[1].contains("hi"), "{:?}", b.0);
}
both!(chat_reaches_the_behaviour, chat_reaches_the_behaviour);

/// The session future is `Send` with a `Send` behaviour, so many bots can share a
/// multi-threaded runtime; one bot's failure stays in its task.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn many_bots_one_runtime() {
    fn assert_send<T: Send>(t: T) -> T {
        t
    }
    let (l, addr) = listen().await;
    let server = tokio::spawn(async move {
        // First bot: refused; second bot: kicked after joining.
        let mut c = accept(&l).await;
        c.accept_login::<V1_8>("BotA").await;
        c.send(&V1_8::login_disconnect(r#"{"text":"no"}"#)).await;
        let mut c = accept(&l).await;
        c.login::<V1_8>("BotB", 64).await;
        spawn::<V1_8>(&mut c).await;
        c.until::<V1_8>("position_look").await;
        c.send(&V1_8::kick(r#"{"text":"bye"}"#)).await;
    });
    let a = client(addr.clone(), GameVersion::V1_8_9, "BotA");
    let ra = tokio::spawn(assert_send(async move { a.run(&mut Idle).await }));
    let ra = ra.await.unwrap();
    let b = client(addr, GameVersion::V1_8_9, "BotB");
    let rb = tokio::spawn(assert_send(async move { b.run(&mut Idle).await }));
    assert!(matches!(ra, Err(ClientError::LoginRefused(_))));
    assert!(matches!(rb.await.unwrap(), Ok(SessionEnd::Kicked { .. })));
    server.await.unwrap();
}
