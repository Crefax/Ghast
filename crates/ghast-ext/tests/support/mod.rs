//! An in-process 1.8 fake server for the helper tests: offline login, a small world, and
//! vanilla-like answers to digs, places, window clicks and chat. Every serverbound packet is
//! logged so the tests can check what reached the wire, and in which order.

#![allow(dead_code)]

use std::collections::HashMap;
use std::time::Duration;

use bytes::{BufMut, BytesMut};
use ghast_client::connect::TargetAddr;
use ghast_client::{Account, BlockPos, Client, ClientError, GameVersion, SessionEnd};
use ghast_ext::Ext;
use ghast_protocol::types::ByteString;
use ghast_protocol::types::slot::legacy::{ItemStack, Slot};
use ghast_protocol::v1_8::play::clientbound as cb;
use ghast_protocol::v1_8::play::serverbound::ServerboundPlayPacket as Sb;
use ghast_protocol::{FrameDecoder, FrameEncoder, PacketSet, RawPacket};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub const STONE: u16 = 1 << 4;
pub const DIRT: u16 = 3 << 4;
pub const CHEST: u16 = 54 << 4 | 2;
pub const COBBLESTONE: i16 = 4;
pub const DIAMOND_PICKAXE: i16 = 278;
pub const BREAD: i16 = 297;

/// A chest's position and its items (slot, id, count).
pub type Chest = ([i32; 3], Vec<(i16, i16, i8)>);

/// The test world and the player's start.
pub struct Setup {
    /// Raw 1.8 states (`id << 4 | meta`) in chunks -1..=1 (x, z in -16..=31).
    pub blocks: HashMap<[i32; 3], u16>,
    /// Window-0 slot -> (item id, count): 9..=35 main, 36..=44 hotbar.
    pub inventory: Vec<(i16, i16, i8)>,
    /// A chest at this position opens a 27-slot window with these items (slot, id, count).
    pub chest: Option<Chest>,
    /// Feet position after the join.
    pub spawn: [f64; 3],
    /// Experience orbs (entity id, position) spawned after the join.
    pub orbs: Vec<(i32, [f64; 3])>,
}

impl Setup {
    /// A stone floor at y = 63 over all three chunks, the player at (8.5, 64, 8.5).
    pub fn flat() -> Setup {
        let mut blocks = HashMap::new();
        for x in -16..32 {
            for z in -16..32 {
                blocks.insert([x, 63, z], STONE);
            }
        }
        Setup {
            blocks,
            inventory: Vec::new(),
            chest: None,
            spawn: [8.5, 64.0, 8.5],
            orbs: Vec::new(),
        }
    }

    pub fn block(mut self, p: [i32; 3], raw: u16) -> Setup {
        self.blocks.insert(p, raw);
        self
    }

    pub fn item(mut self, slot: i16, id: i16, count: i8) -> Setup {
        self.inventory.push((slot, id, count));
        self
    }
}

/// What the server saw.
#[derive(Debug, Default, Clone)]
pub struct Log {
    /// Every serverbound play packet name, in order.
    pub packets: Vec<&'static str>,
    /// block_dig (status, position).
    pub digs: Vec<(i32, BlockPos)>,
    /// block_place (clicked block, face).
    pub places: Vec<(BlockPos, i8)>,
    /// window_click (window, slot, button, mode).
    pub clicks: Vec<(u8, i16, i8, i8)>,
    /// Chat lines.
    pub chats: Vec<String>,
    /// Selected hotbar slots.
    pub held: Vec<i16>,
    /// The last movement packet's position and look.
    pub pos: [f64; 3],
    pub look: (f32, f32),
    /// The final world.
    pub blocks: HashMap<[i32; 3], u16>,
}

fn raw<P: ghast_protocol::Packet>(p: &P) -> RawPacket {
    RawPacket::from_packet(p).unwrap()
}

fn slot(id: i16, count: i8) -> Slot {
    Slot(Some(ItemStack {
        id,
        count,
        damage: 0,
        nbt: None,
    }))
}

fn chat(text: &str) -> RawPacket {
    raw(&cb::Chat {
        message: ByteString::from(format!(r#"{{"text":"{text}"}}"#)),
        position: 0,
    })
}

/// A ground-up 1.8 chunk with sections 3 and 4 (y 48..=79).
fn chunk(cx: i32, cz: i32, blocks: &HashMap<[i32; 3], u16>) -> RawPacket {
    let mut d = BytesMut::with_capacity(2 * 12_288 + 256);
    for sy in 3..5 {
        for i in 0..4096 {
            let (x, z, y) = (i & 15, (i >> 4) & 15, i >> 8);
            let p = [cx * 16 + x, sy * 16 + y, cz * 16 + z];
            d.put_u16_le(blocks.get(&p).copied().unwrap_or(0));
        }
    }
    d.put_bytes(0, 2 * 2048);
    d.put_bytes(0xff, 2 * 2048);
    d.put_bytes(1, 256);
    raw(&cb::MapChunk {
        x: cx,
        z: cz,
        ground_up: true,
        bit_map: 0b11000,
        chunk_data: d.freeze(),
    })
}

struct Conn {
    stream: TcpStream,
    dec: FrameDecoder,
    enc: FrameEncoder,
}

impl Conn {
    async fn recv(&mut self) -> Option<RawPacket> {
        loop {
            if let Some(p) = self.dec.next_packet().ok()? {
                return Some(p);
            }
            let mut buf = [0u8; 8192];
            let n = self.stream.read(&mut buf).await.ok()?;
            if n == 0 {
                return None;
            }
            self.dec.push(&buf[..n]);
        }
    }

    async fn send(&mut self, p: &RawPacket) {
        self.enc.write_raw(p).unwrap();
        let out = self.enc.take();
        let _ = self.stream.write_all(&out).await;
    }
}

fn offset(p: BlockPos, face: i8) -> BlockPos {
    match face {
        0 => BlockPos::new(p.x, p.y - 1, p.z),
        1 => BlockPos::new(p.x, p.y + 1, p.z),
        2 => BlockPos::new(p.x, p.y, p.z - 1),
        3 => BlockPos::new(p.x, p.y, p.z + 1),
        4 => BlockPos::new(p.x - 1, p.y, p.z),
        _ => BlockPos::new(p.x + 1, p.y, p.z),
    }
}

async fn serve(l: TcpListener, setup: Setup) -> Log {
    let (stream, _) = l.accept().await.unwrap();
    let mut c = Conn {
        stream,
        dec: FrameDecoder::new(),
        enc: FrameEncoder::new(),
    };
    let mut log = Log::default();
    let mut blocks = setup.blocks;
    // Handshake, login start; offline success without compression.
    c.recv().await.unwrap();
    let ls = c.recv().await.unwrap();
    let name = ls
        .decode_as::<ghast_protocol::v1_8::login::serverbound::LoginStart>()
        .unwrap()
        .username
        .to_string();
    c.send(&raw(&ghast_protocol::v1_8::login::clientbound::Success {
        uuid: ByteString::from(ghast_auth::offline::offline_uuid(&name).to_string()),
        username: ByteString::from(name.as_str()),
    }))
    .await;
    c.send(&raw(&cb::Login {
        entity_id: 42,
        game_mode: 0,
        dimension: 0,
        difficulty: 1,
        max_players: 20,
        level_type: ByteString::from("flat"),
        reduced_debug_info: false,
    }))
    .await;
    for cx in -1..=1 {
        for cz in -1..=1 {
            c.send(&chunk(cx, cz, &blocks)).await;
        }
    }
    let mut inv: Vec<Slot> = vec![Slot::default(); 45];
    for &(s, id, n) in &setup.inventory {
        inv[s as usize] = slot(id, n);
    }
    c.send(&raw(&cb::WindowItems {
        window_id: 0,
        items: inv.clone(),
    }))
    .await;
    let [sx, sy, sz] = setup.spawn;
    c.send(&raw(&cb::Position {
        x: sx,
        y: sy,
        z: sz,
        yaw: 0.0,
        pitch: 0.0,
        flags: 0,
    }))
    .await;
    for &(id, [x, y, z]) in &setup.orbs {
        c.send(&raw(&cb::SpawnEntityExperienceOrb {
            entity_id: id,
            x: (x * 32.0) as i32,
            y: (y * 32.0) as i32,
            z: (z * 32.0) as i32,
            count: 1,
        }))
        .await;
    }
    let mut held = 0i16;
    let mut keep_alive = 0i32;
    let mut beat = tokio::time::interval(Duration::from_millis(250));
    loop {
        let p = tokio::select! {
            p = c.recv() => p,
            _ = beat.tick() => {
                keep_alive += 1;
                c.send(&raw(&cb::KeepAlive { keep_alive_id: keep_alive })).await;
                continue;
            }
        };
        let Some(p) = p else { break };
        let Ok(pkt) = Sb::decode_raw(&p) else {
            continue;
        };
        log.packets.push(pkt.name());
        match pkt {
            Sb::Chat(m) => {
                let text = m.message.to_string();
                log.chats.push(text.clone());
                if text == "/tp" {
                    c.send(&raw(&cb::Position {
                        x: sx + 3.0,
                        y: sy,
                        z: sz,
                        yaw: 90.0,
                        pitch: 0.0,
                        flags: 0,
                    }))
                    .await;
                } else if let Some(cmd) = text.strip_prefix('/') {
                    c.send(&chat(&format!("pong {cmd}"))).await;
                } else {
                    c.send(&chat(&format!("<{name}> {text}"))).await;
                }
            }
            Sb::BlockDig(d) => {
                log.digs.push((d.status, d.location));
                let l = d.location;
                if d.status == 2 {
                    blocks.insert([l.x, l.y, l.z], 0);
                    c.send(&raw(&cb::BlockChange {
                        location: l,
                        r#type: 0,
                    }))
                    .await;
                }
            }
            Sb::BlockPlace(b) => {
                let l = b.location;
                log.places.push((l, b.direction));
                if b.direction < 0 {
                    continue;
                }
                if let Some((cp, items)) = &setup.chest
                    && *cp == [l.x, l.y, l.z]
                {
                    c.send(&raw(&cb::OpenWindow {
                        window_id: 1,
                        inventory_type: ByteString::from("minecraft:chest"),
                        window_title: ByteString::from(r#"{"text":"Chest"}"#),
                        slot_count: 27,
                        entity_id: None,
                    }))
                    .await;
                    let mut w: Vec<Slot> = vec![Slot::default(); 63];
                    for &(s, id, n) in items {
                        w[s as usize] = slot(id, n);
                    }
                    // Player main (window 0 slots 9..=35) and hotbar (36..=44) follow.
                    for k in 9..45 {
                        w[27 + k - 9] = inv[k].clone();
                    }
                    c.send(&raw(&cb::WindowItems {
                        window_id: 1,
                        items: w,
                    }))
                    .await;
                    continue;
                }
                let stack = inv[36 + held as usize].0.clone();
                if let Some(s) = stack
                    && (1..256).contains(&s.id)
                {
                    let t = offset(l, b.direction);
                    let r = (s.id as u16) << 4;
                    blocks.insert([t.x, t.y, t.z], r);
                    c.send(&raw(&cb::BlockChange {
                        location: t,
                        r#type: i32::from(r),
                    }))
                    .await;
                }
            }
            Sb::HeldItemSlot(h) => {
                held = h.slot_id;
                log.held.push(h.slot_id);
            }
            Sb::WindowClick(w) => {
                log.clicks
                    .push((w.window_id, w.slot, w.mouse_button, w.mode));
                c.send(&raw(&cb::Transaction {
                    window_id: w.window_id as i8,
                    action: w.action,
                    accepted: true,
                }))
                .await;
            }
            Sb::Position(m) => log.pos = [m.x, m.y, m.z],
            Sb::PositionLook(m) => {
                log.pos = [m.x, m.y, m.z];
                log.look = (m.yaw, m.pitch);
            }
            Sb::Look(m) => log.look = (m.yaw, m.pitch),
            _ => {}
        }
    }
    log.blocks = blocks;
    log
}

/// Runs `ext` against a fake server built from `setup` until the session ends (the main
/// task returns, by default). Panics after `secs` seconds.
pub async fn run(setup: Setup, ext: &mut Ext, secs: u64) -> (SessionEnd, Log) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = TargetAddr::from(l.local_addr().unwrap());
    let server = tokio::spawn(serve(l, setup));
    let client = Client::new(
        addr,
        GameVersion::V1_8_9,
        Account::offline("Helper").unwrap(),
    )
    .read_timeout(Duration::from_secs(5))
    .login_timeout(Duration::from_secs(5));
    let end: Result<SessionEnd, ClientError> =
        tokio::time::timeout(Duration::from_secs(secs), client.run(ext))
            .await
            .expect("test session timed out");
    let log = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("server did not end")
        .unwrap();
    (end.expect("session failed"), log)
}
