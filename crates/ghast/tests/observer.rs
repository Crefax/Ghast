//! An observer written against the public facade only, as a downstream crate would: it
//! implements `Behavior::after_tick` and reads every `TickView` accessor, and runs against a
//! minimal in-process 1.8 server. Guards against public API types that cannot be named
//! outside the workspace.

use std::time::Duration;

use ghast::connect::TargetAddr;
use ghast::protocol::types::ByteString;
use ghast::protocol::v1_8::{handshaking, login, play};
use ghast::protocol::{FrameDecoder, FrameEncoder, Packet, RawPacket};
use ghast::{
    Account, Behavior, Client, Controls, GameMode, GameVersion, Pose, SessionEnd, TeleportEvent,
    TickContext, TickView,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// One `after_tick` reading.
#[derive(Debug, Clone)]
struct Seen {
    tick: u64,
    version: GameVersion,
    pose: Pose,
    sent: Pose,
    column_loaded: bool,
    has_entities: bool,
    held_item: bool,
    teleports: usize,
    entity_id: i32,
    health: f32,
    game_mode: GameMode,
    held_slot: u8,
    mining: bool,
    swings: u64,
    broken: u64,
    dimension: String,
    using_item: bool,
}

/// Walks forward for a few ticks and records what the server has after each one.
#[derive(Default)]
struct Observer {
    ticks: u32,
    teleports: Vec<TeleportEvent>,
    seen: Vec<Seen>,
}

impl Behavior for Observer {
    fn name(&self) -> &str {
        "observer"
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        self.ticks += 1;
        self.teleports
            .extend(ctx.events().teleports.iter().copied());
        ctx.set_controls(Controls {
            forward: true,
            ..Controls::default()
        });
        if self.ticks >= 10 {
            ctx.disconnect("done");
        }
    }

    fn after_tick(&mut self, view: &TickView<'_>) {
        let p = view.sent();
        self.seen.push(Seen {
            tick: view.tick(),
            version: view.version(),
            pose: *view.pose(),
            sent: *p,
            column_loaded: view
                .world()
                .is_column_loaded((p.pos.x.floor() as i32) >> 4, (p.pos.z.floor() as i32) >> 4),
            has_entities: view.entities().is_some(),
            held_item: view.inventory().held().is_some(),
            teleports: view.events().teleports.len(),
            entity_id: view.entity_id(),
            health: view.health().health,
            game_mode: view.game_mode(),
            held_slot: view.held_slot(),
            mining: view.mining().is_some(),
            swings: view.swings(),
            broken: view.broken(),
            dimension: view.dimension().to_owned(),
            using_item: view.using_item(),
        });
    }
}

/// The world types are nameable too (opaque section storage).
#[allow(dead_code)]
fn world_types_are_nameable(_: Option<&ghast::world::Indexed>, _: Option<&ghast::world::Direct>) {}

// ---- a minimal 1.8 server ---------------------------------------------------------------

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

    async fn send<P: Packet>(&mut self, p: &P) {
        self.enc
            .write_raw(&RawPacket::from_packet(p).unwrap())
            .unwrap();
        let out = self.enc.take();
        let _ = self.stream.write_all(&out).await;
    }
}

/// One section (y 48–63) with a stone top layer: the spawn at y = 64 stands on it.
fn chunk(x: i32, z: i32) -> play::clientbound::MapChunk {
    let mut d = Vec::with_capacity(12_544);
    for i in 0..4096 {
        let state: u16 = if i >> 8 == 15 { 1 << 4 } else { 0 };
        d.extend_from_slice(&state.to_le_bytes());
    }
    d.extend(std::iter::repeat_n(0u8, 2048));
    d.extend(std::iter::repeat_n(0xffu8, 2048));
    d.extend(std::iter::repeat_n(1u8, 256));
    play::clientbound::MapChunk {
        x,
        z,
        ground_up: true,
        bit_map: 1 << 3,
        chunk_data: d.into(),
    }
}

async fn serve(l: TcpListener) {
    let (stream, _) = l.accept().await.unwrap();
    let mut c = Conn {
        stream,
        dec: FrameDecoder::new(),
        enc: FrameEncoder::new(),
    };
    let hs = c.recv().await.unwrap();
    let hs = hs
        .decode_as::<handshaking::serverbound::SetProtocol>()
        .unwrap();
    assert_eq!(hs.protocol_version, 47);
    let ls = c.recv().await.unwrap();
    let name = ls
        .decode_as::<login::serverbound::LoginStart>()
        .unwrap()
        .username;
    assert_eq!(&*name, "Watcher");
    c.send(&login::clientbound::Success {
        uuid: ByteString::from(ghast::auth::offline::offline_uuid("Watcher").to_string()),
        username: name,
    })
    .await;
    c.send(&play::clientbound::Login {
        entity_id: 42,
        game_mode: 0,
        dimension: 0,
        difficulty: 1,
        max_players: 20,
        level_type: ByteString::from("default"),
        reduced_debug_info: false,
    })
    .await;
    c.send(&chunk(0, 0)).await;
    c.send(&play::clientbound::Position {
        x: 8.5,
        y: 64.0,
        z: 8.5,
        yaw: 0.0,
        pitch: 0.0,
        flags: 0,
    })
    .await;
    // Reads until the client disconnects.
    while c.recv().await.is_some() {}
}

#[tokio::test]
async fn after_tick_from_outside_the_crate() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = TargetAddr::from(l.local_addr().unwrap());
    let client = Client::new(
        addr,
        GameVersion::V1_8_9,
        Account::offline("Watcher").unwrap(),
    )
    .read_timeout(Duration::from_secs(5))
    .login_timeout(Duration::from_secs(5));

    // Boxed, as a runner picking a behaviour at run time would (the box forwards
    // `after_tick`).
    let mut b: Box<Observer> = Box::default();
    let (end, ()) = tokio::join!(client.run(&mut b), serve(l));
    assert!(matches!(end, Ok(SessionEnd::Quit(_))), "{end:?}");

    assert!(b.seen.len() >= 10, "{} readings", b.seen.len());
    assert!(
        b.seen.windows(2).all(|w| w[0].tick < w[1].tick),
        "ticks increase"
    );
    let teleports: usize = b.seen.iter().map(|s| s.teleports).sum();
    assert_eq!(teleports, b.teleports.len(), "views see the tick's events");
    assert!(!b.teleports.is_empty(), "the spawn position is a teleport");
    for s in &b.seen {
        assert_eq!(s.version, GameVersion::V1_8_9);
        assert_eq!(s.entity_id, 42);
        assert_eq!(s.dimension, "0");
        assert_eq!(s.game_mode, GameMode::Survival);
        assert_eq!(s.held_slot, 0);
        assert!(s.health > 0.0);
        assert!(s.column_loaded);
        assert!(!s.has_entities, "entities are opt-in");
        assert!(!s.held_item && !s.mining && !s.using_item);
        assert_eq!((s.swings, s.broken), (0, 0));
        assert!(
            (s.sent.pos.y - 64.0).abs() < 1e-9,
            "stands on the stone: {:?}",
            s.sent
        );
    }
    let last = b.seen.last().unwrap();
    assert!(last.sent.on_ground, "{:?}", last.sent);
    assert!(last.pose.pos.z > 8.5, "walked forward: {:?}", last.pose);
}
