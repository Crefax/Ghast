//! Live check of ghast-inventory against a real server (the WSL labs): a minimal standing-still
//! client that opens a chest and the player inventory, performs every click type, and verifies
//! that (1) the server accepts every click (its own `onSlotClick` returned the same stack we
//! sent) and (2) the server's `window_items` after reopening equals our prediction.
//!
//! ```sh
//! lab/console.sh 1.8 "setblock 0 4 2 chest 2 replace {Items:[...]}" ...   # see lab notes
//! cargo run -p ghast-inventory --example live_clicks -- --version 1.8 --port 25618 --user GhastBot
//! ```
//! Test tool only (no proxy, offline login, no physics: it stands still on a floor).

use std::error::Error;
use std::time::Duration;

use ghast_inventory::{Click, ClickOutcome, Inventory, OUTSIDE, Outgoing, Stack, Version};
use ghast_protocol::{BlockPos, FramedReader, FramedWriter, PacketSet, RawPacket};
use tokio::net::TcpStream;

type Res<T> = Result<T, Box<dyn Error>>;

#[derive(Debug, Clone)]
enum Step {
    Wait(u32),
    Look(f32, f32),
    UseChest,
    WaitWindow,
    OpenInv,
    Close,
    Click(Click),
    /// Remember the current screen's slots (our prediction).
    Snapshot,
    /// Compare the current screen's slots (just resent by the server) with the snapshot;
    /// `player_only`: only the 36 player slots (the snapshot was taken in window 0).
    Verify {
        player_only: bool,
    },
    Done,
}

/// Standing-still movement: vanilla's packet choice (position refresh every 20 ticks; 1.8
/// checks before incrementing, 1.9+ after) and on-ground false on the first tick after a
/// teleport.
#[derive(Debug, Default)]
struct Mover {
    pos: (f64, f64, f64),
    yaw: f32,
    pitch: f32,
    last_pos: (f64, f64, f64),
    last_yaw: f32,
    last_pitch: f32,
    since_pos: u32,
    since_tp: u32,
    increment_first: bool,
}

enum Move {
    Flying(bool),
    Pos(bool),
    Look(bool),
    PosLook(bool),
}

impl Mover {
    fn step(&mut self) -> Move {
        let on_ground = self.since_tp > 0;
        self.since_tp += 1;
        let (dx, dy, dz) = (
            self.pos.0 - self.last_pos.0,
            self.pos.1 - self.last_pos.1,
            self.pos.2 - self.last_pos.2,
        );
        let turned = self.yaw != self.last_yaw || self.pitch != self.last_pitch;
        if self.increment_first {
            self.since_pos += 1;
        }
        let moved = dx * dx + dy * dy + dz * dz > 9.0e-4 || self.since_pos >= 20;
        if !self.increment_first {
            self.since_pos += 1;
        }
        if moved {
            self.last_pos = self.pos;
            self.since_pos = 0;
        }
        if turned {
            self.last_yaw = self.yaw;
            self.last_pitch = self.pitch;
        }
        match (moved, turned) {
            (true, true) => Move::PosLook(on_ground),
            (true, false) => Move::Pos(on_ground),
            (false, true) => Move::Look(on_ground),
            (false, false) => Move::Flying(on_ground),
        }
    }
}

struct Driver {
    inv: Inventory,
    mover: Mover,
    spawned: bool,
    script: Vec<Step>,
    pc: usize,
    wait: u32,
    snapshot: Vec<Option<Stack>>,
    clicks: u32,
    accepted: u32,
    failures: Vec<String>,
    window_seen: Option<i32>,
}

/// What the tick wants written (version-independent).
enum Action {
    Out(Outgoing),
    UseChest,
}

impl Driver {
    fn new(v: Version, script: Vec<Step>) -> Driver {
        Driver {
            inv: Inventory::new(v),
            mover: Mover {
                increment_first: v != Version::V1_8,
                ..Mover::default()
            },
            spawned: false,
            script,
            pc: 0,
            wait: 0,
            snapshot: Vec::new(),
            clicks: 0,
            accepted: 0,
            failures: Vec::new(),
            window_seen: None,
        }
    }

    fn screen(&self) -> Vec<Option<Stack>> {
        (0..self.inv.slot_count())
            .map(|i| self.inv.slot(i).cloned())
            .collect()
    }

    /// One tick of the script; returns the actions to write before the movement packet.
    fn tick(&mut self) -> (Vec<Action>, bool) {
        self.inv.tick();
        let ev = self.inv.drain_events();
        for c in ev.clicks {
            match c.outcome {
                ClickOutcome::Accepted => self.accepted += 1,
                o => self.failures.push(format!(
                    "click action {} in window {}: {o:?}",
                    c.action, c.window_id
                )),
            }
            println!("  confirm action {} -> {:?}", c.action, c.outcome);
        }
        if let Some(id) = ev.opened {
            self.window_seen = Some(id);
        }
        let mut out = Vec::new();
        if self.wait > 0 {
            self.wait -= 1;
            return (out, false);
        }
        let Some(step) = self.script.get(self.pc).cloned() else {
            return (out, true);
        };
        let mut advance = true;
        match step {
            Step::Wait(n) => self.wait = n,
            Step::Look(yaw, pitch) => {
                self.mover.yaw = yaw;
                self.mover.pitch = pitch;
            }
            Step::UseChest => {
                self.window_seen = None;
                out.push(Action::UseChest);
            }
            Step::WaitWindow => advance = self.window_seen.is_some() && !self.inv.resyncing(),
            Step::OpenInv => match self.inv.open_inventory() {
                Ok(o) => out.extend(o.map(Action::Out)),
                Err(e) => {
                    println!("  open_inventory: {e}");
                    advance = false;
                }
            },
            Step::Close => {
                if self.inv.pending() {
                    advance = false;
                } else {
                    match self.inv.close() {
                        Ok(o) => out.push(Action::Out(o)),
                        Err(e) => self.failures.push(format!("close: {e}")),
                    }
                }
            }
            Step::Click(c) => match self.inv.click(c) {
                Ok(Outgoing::Click(p)) => {
                    self.clicks += 1;
                    println!(
                        "  click {c:?} -> window {} slot {} button {} mode {} action {} item {:?}",
                        p.window_id,
                        p.slot,
                        p.button,
                        p.mode,
                        p.action,
                        p.item.as_ref().map(|s| (s.id, s.count, s.damage))
                    );
                    out.push(Action::Out(Outgoing::Click(p)));
                }
                Ok(_) => {}
                Err(
                    e @ (ghast_inventory::ClickError::Pending
                    | ghast_inventory::ClickError::SameTick
                    | ghast_inventory::ClickError::Resyncing),
                ) => {
                    let _ = e;
                    advance = false;
                }
                Err(e) => self.failures.push(format!("{c:?} refused: {e}")),
            },
            Step::Snapshot => {
                if self.inv.pending() {
                    advance = false;
                } else {
                    self.snapshot = self.screen();
                    if self.inv.cursor().is_some() {
                        self.failures.push("cursor not empty at snapshot".into());
                    }
                }
            }
            Step::Verify { player_only } => {
                let now = self.screen();
                let n = self.inv.container_len();
                let pairs: Vec<(usize, usize)> = if player_only {
                    // snapshot taken in window 0: slots 9..45 = player part n..n+36
                    (0..36).map(|i| (9 + i, n + i)).collect()
                } else {
                    (0..now.len()).map(|i| (i, i)).collect()
                };
                let mut bad = 0;
                for (a, b) in pairs {
                    let (x, y) = (
                        self.snapshot.get(a).cloned().flatten(),
                        now.get(b).cloned().flatten(),
                    );
                    let key = |s: &Option<Stack>| {
                        s.as_ref()
                            .map(|s| (s.id, s.count, s.damage, s.nbt.is_some()))
                    };
                    if key(&x) != key(&y) {
                        bad += 1;
                        self.failures.push(format!(
                            "slot {b}: predicted {:?}, server {:?}",
                            key(&x),
                            key(&y)
                        ));
                    }
                }
                println!("  verify: {bad} differing slots");
            }
            Step::Done => return (out, true),
        }
        if advance {
            self.pc += 1;
        }
        (out, false)
    }
}

const CHEST: BlockPos = BlockPos::new(0, 4, 2);

fn script() -> Vec<Step> {
    use Step::*;
    use ghast_inventory::Click::*;
    // chest (27) + player: chest 0..27, main 27..54, hotbar 54..63
    vec![
        Wait(60),
        Look(0.0, 245.0 * 0.15), // GCD multiple at sensitivity 0.5, at the chest's north face
        Wait(3),
        UseChest,
        WaitWindow,
        Wait(3),
        Step::Click(Pickup {
            slot: 0,
            right: false,
        }), // take 64 stone
        Step::Click(Pickup {
            slot: 30,
            right: false,
        }), // put it into main
        Step::Click(Pickup {
            slot: 1,
            right: true,
        }), // half of the dirt
        Step::Click(Pickup {
            slot: OUTSIDE,
            right: true,
        }), // drop one
        Step::Click(Pickup {
            slot: 1,
            right: false,
        }), // merge back
        Step::Click(QuickMove {
            slot: 2,
            right: false,
        }), // wool -> player (reverse)
        Step::Click(QuickMove {
            slot: 30,
            right: false,
        }), // stone -> chest
        Step::Click(Swap { slot: 3, hotbar: 0 }), // chest slot 3 with hotbar 0
        Step::Click(Swap { slot: 0, hotbar: 1 }), // chest stone with hotbar 1
        Step::Click(Drop {
            slot: 1,
            all: false,
        }), // Q
        Step::Click(Pickup {
            slot: 54,
            right: false,
        }), // pick up hotbar 0
        Step::Click(PickupAll { slot: 54 }),      // double click
        Step::Click(Pickup {
            slot: 54,
            right: false,
        }), // put down
        Step::Click(QuickMove {
            slot: 54,
            right: true,
        }),
        Step::Click(Drop {
            slot: OUTSIDE,
            all: false,
        }), // GUI click outside with empty cursor
        Snapshot,
        Close,
        Wait(5),
        UseChest,
        WaitWindow,
        Wait(3),
        Verify { player_only: false },
        Close,
        Wait(5),
        // the player inventory (window 0)
        OpenInv,
        Wait(2),
        Step::Click(QuickMove {
            slot: 38,
            right: false,
        }), // helmet -> armor slot 5
        Step::Click(QuickMove {
            slot: 39,
            right: false,
        }), // pearls -> main 9
        Step::Click(Pickup {
            slot: 9,
            right: false,
        }), // 10 pearls on the cursor
        Step::Click(Pickup {
            slot: 1,
            right: true,
        }), // one into the crafting grid
        Step::Click(Pickup {
            slot: 2,
            right: true,
        }), // one into the next grid slot
        Step::Click(Pickup {
            slot: 1,
            right: false,
        }), // merge the rest (max 16)
        Step::Click(Pickup {
            slot: 1,
            right: false,
        }), // take them back
        Step::Click(Pickup {
            slot: 2,
            right: false,
        }), // merge into grid 2
        Step::Click(QuickMove {
            slot: 2,
            right: false,
        }), // grid -> inventory
        Step::Click(Swap {
            slot: 44,
            hotbar: 6,
        }), // wool to hotbar 6
        Step::Click(QuickMove {
            slot: 5,
            right: false,
        }), // helmet back
        Step::Click(Drop {
            slot: 42,
            all: false,
        }), // Q on the wool
        Step::Click(Pickup {
            slot: 42,
            right: false,
        }),
        Step::Click(PickupAll { slot: 42 }),
        Step::Click(Pickup {
            slot: 15,
            right: false,
        }),
        Snapshot,
        Close,
        Wait(5),
        UseChest,
        WaitWindow,
        Wait(3),
        Verify { player_only: true },
        Close,
        Wait(10),
        Done,
    ]
}

fn args() -> (Version, u16, String) {
    let a: Vec<String> = std::env::args().collect();
    let get = |k: &str| {
        a.iter()
            .position(|x| x == k)
            .and_then(|i| a.get(i + 1))
            .cloned()
    };
    let v = match get("--version").as_deref() {
        Some("1.16" | "1.16.5") => Version::V1_16_5,
        Some("1.12" | "1.12.2") => Version::V1_12,
        _ => Version::V1_8,
    };
    let port = get("--port").and_then(|p| p.parse().ok()).unwrap_or(25618);
    (v, port, get("--user").unwrap_or_else(|| "GhastBot".into()))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Res<()> {
    let (v, port, user) = args();
    let tcp = TcpStream::connect(("127.0.0.1", port)).await?;
    tcp.set_nodelay(true)?;
    let (rd, wr) = tcp.into_split();
    let mut r = FramedReader::new(rd);
    let mut w = FramedWriter::new(wr);
    let mut d = Driver::new(v, script());
    match v {
        Version::V1_8 => v18::run(&mut r, &mut w, &mut d, port, &user).await?,
        Version::V1_12 => v12::run(&mut r, &mut w, &mut d, port, &user).await?,
        Version::V1_16_5 => v16::run(&mut r, &mut w, &mut d, port, &user).await?,
    }
    println!(
        "clicks sent {}, accepted {}, failures {}",
        d.clicks,
        d.accepted,
        d.failures.len()
    );
    for f in &d.failures {
        println!("  FAIL {f}");
    }
    if d.failures.is_empty() && d.clicks == d.accepted {
        println!("PASS");
        Ok(())
    } else {
        Err("live click check failed".into())
    }
}

type Rd = FramedReader<tokio::net::tcp::OwnedReadHalf>;
type Wr = FramedWriter<tokio::net::tcp::OwnedWriteHalf>;

macro_rules! session {
    ($m:ident, $ver:ident, $proto:expr, { $($extra:item)* }) => {
        mod $m {
            use super::*;
            use ghast_protocol::$ver::{handshaking, login, play};
            use play::clientbound::{self as cb, ClientboundPlayPacket as P};
            use play::serverbound::{self as sb, ServerboundPlayPacket as S};

            pub(super) async fn run(r: &mut Rd, w: &mut Wr, d: &mut Driver, port: u16, user: &str) -> Res<()> {
                w.queue(&handshaking::serverbound::SetProtocol {
                    protocol_version: $proto,
                    server_host: "127.0.0.1".into(),
                    server_port: port,
                    next_state: 2,
                })?;
                w.queue(&login::serverbound::LoginStart { username: user.into() })?;
                w.flush().await?;
                loop {
                    let raw = r.read_packet().await?.ok_or("closed during login")?;
                    match login::clientbound::ClientboundLoginPacket::decode_raw(&raw)? {
                        login::clientbound::ClientboundLoginPacket::Compress(c) => {
                            r.decoder().set_compression(c.threshold);
                            w.encoder().set_compression(c.threshold);
                        }
                        login::clientbound::ClientboundLoginPacket::Success(_) => break,
                        login::clientbound::ClientboundLoginPacket::Disconnect(dc) => {
                            return Err(format!("login refused: {}", dc.reason).into());
                        }
                        other => return Err(format!("unexpected login packet {}", other.name()).into()),
                    }
                }
                println!("joined as {user}");
                let mut tick = tokio::time::interval(Duration::from_millis(50));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        p = r.read_packet() => {
                            let raw = p?.ok_or("server closed")?;
                            inbound(&raw, r, w, d)?;
                            w.flush().await?;
                        }
                        _ = tick.tick() => {
                            if !d.spawned { continue; }
                            let (actions, done) = d.tick();
                            for a in actions {
                                match a {
                                    Action::Out(o) => queue(w, &o)?,
                                    Action::UseChest => use_chest(w, d)?,
                                }
                            }
                            let on = |m: &Mover| (m.pos, m.yaw, m.pitch);
                            let (pos, yaw, pitch) = on(&d.mover);
                            let p = match d.mover.step() {
                                Move::Flying(g) => S::Flying(sb::Flying { on_ground: g }),
                                Move::Pos(g) => S::Position(sb::Position { x: pos.0, y: pos.1, z: pos.2, on_ground: g }),
                                Move::Look(g) => S::Look(sb::Look { yaw, pitch, on_ground: g }),
                                Move::PosLook(g) => S::PositionLook(sb::PositionLook { x: pos.0, y: pos.1, z: pos.2, yaw, pitch, on_ground: g }),
                            };
                            w.queue_set(&p)?;
                            w.flush().await?;
                            if done { return Ok(()); }
                        }
                    }
                }
            }

            fn queue(w: &mut Wr, o: &Outgoing) -> Res<()> {
                if let Some(p) = to_packet(o) {
                    w.queue_set(&p)?;
                }
                Ok(())
            }

            fn inbound(raw: &RawPacket, r: &mut Rd, w: &mut Wr, d: &mut Driver) -> Res<()> {
                let interesting = [
                    cb::id::KEEP_ALIVE, cb::id::POSITION, cb::id::LOGIN, cb::id::RESPAWN,
                    cb::id::KICK_DISCONNECT, cb::id::OPEN_WINDOW, cb::id::CLOSE_WINDOW,
                    cb::id::WINDOW_ITEMS, cb::id::SET_SLOT, cb::id::TRANSACTION,
                    cb::id::HELD_ITEM_SLOT, cb::id::CRAFT_PROGRESS_BAR, extra_id(),
                ];
                if !interesting.contains(&raw.id) {
                    return Ok(());
                }
                let p = P::decode_raw(raw)?;
                if let Some(o) = apply(&mut d.inv, &p) {
                    queue(w, &o)?;
                }
                match &p {
                    P::KeepAlive(k) => w.queue(&sb::KeepAlive { keep_alive_id: k.keep_alive_id })?,
                    P::KickDisconnect(k) => return Err(format!("kicked: {}", k.reason).into()),
                    P::Position(t) => {
                        teleport(t, w, d)?;
                    }
                    P::OpenWindow(_) => println!("  window opened"),
                    _ => {}
                }
                let _ = r;
                Ok(())
            }

            $($extra)*
        }
    };
}

session!(v18, v1_8, 47, {
    fn extra_id() -> i32 {
        cb::id::SET_COMPRESSION
    }
    fn apply(inv: &mut Inventory, p: &P) -> Option<Outgoing> {
        if let P::SetCompression(_) = p {
            return None;
        }
        inv.apply_v1_8(p)
    }
    fn to_packet(o: &Outgoing) -> Option<S> {
        Some(o.to_v1_8())
    }
    fn teleport(t: &cb::Position, w: &mut Wr, d: &mut Driver) -> Res<()> {
        // 1.8: relative flags, then C06 echo with on_ground false; lastReported untouched
        let f = t.flags;
        let m = &mut d.mover;
        m.pos.0 = if f & 1 != 0 { m.pos.0 + t.x } else { t.x };
        m.pos.1 = if f & 2 != 0 { m.pos.1 + t.y } else { t.y };
        m.pos.2 = if f & 4 != 0 { m.pos.2 + t.z } else { t.z };
        m.yaw = if f & 8 != 0 { m.yaw + t.yaw } else { t.yaw };
        m.pitch = if f & 16 != 0 {
            m.pitch + t.pitch
        } else {
            t.pitch
        };
        m.since_tp = 0;
        w.queue(&sb::PositionLook {
            x: m.pos.0,
            y: m.pos.1,
            z: m.pos.2,
            yaw: m.yaw,
            pitch: m.pitch,
            on_ground: false,
        })?;
        d.spawned = true;
        Ok(())
    }
    fn use_chest(w: &mut Wr, d: &mut Driver) -> Res<()> {
        w.queue(&sb::BlockPlace {
            location: CHEST,
            direction: 2,
            held_item: ghast_inventory::to_legacy_slot(d.inv.held()),
            cursor_x: 8,
            cursor_y: 8,
            cursor_z: 0,
        })?;
        w.queue(&sb::ArmAnimation {})?;
        Ok(())
    }
});

session!(v12, v1_12_2, 340, {
    fn extra_id() -> i32 {
        cb::id::CRAFT_PROGRESS_BAR
    }
    fn apply(inv: &mut Inventory, p: &P) -> Option<Outgoing> {
        inv.apply_v1_12_2(p)
    }
    fn to_packet(o: &Outgoing) -> Option<S> {
        o.to_v1_12_2()
    }
    fn teleport(t: &cb::Position, w: &mut Wr, d: &mut Driver) -> Res<()> {
        let f = t.flags;
        let m = &mut d.mover;
        m.pos.0 = if f & 1 != 0 { m.pos.0 + t.x } else { t.x };
        m.pos.1 = if f & 2 != 0 { m.pos.1 + t.y } else { t.y };
        m.pos.2 = if f & 4 != 0 { m.pos.2 + t.z } else { t.z };
        m.yaw = if f & 8 != 0 { m.yaw + t.yaw } else { t.yaw };
        m.pitch = if f & 16 != 0 {
            m.pitch + t.pitch
        } else {
            t.pitch
        };
        m.since_tp = 0;
        w.queue(&sb::TeleportConfirm {
            teleport_id: t.teleport_id,
        })?;
        w.queue(&sb::PositionLook {
            x: m.pos.0,
            y: m.pos.1,
            z: m.pos.2,
            yaw: m.yaw,
            pitch: m.pitch,
            on_ground: false,
        })?;
        d.spawned = true;
        Ok(())
    }
    fn use_chest(w: &mut Wr, _d: &mut Driver) -> Res<()> {
        w.queue(&sb::BlockPlace {
            location: CHEST,
            direction: 2,
            hand: 0,
            cursor_x: 0.5,
            cursor_y: 0.5,
            cursor_z: 0.0,
        })?;
        w.queue(&sb::ArmAnimation { hand: 0 })?;
        Ok(())
    }
});

session!(v16, v1_16_5, 754, {
    fn extra_id() -> i32 {
        cb::id::OPEN_HORSE_WINDOW
    }
    fn apply(inv: &mut Inventory, p: &P) -> Option<Outgoing> {
        inv.apply_v1_16_5(p)
    }
    fn to_packet(o: &Outgoing) -> Option<S> {
        o.to_v1_16_5()
    }
    fn teleport(t: &cb::Position, w: &mut Wr, d: &mut Driver) -> Res<()> {
        let f = t.flags;
        let m = &mut d.mover;
        m.pos.0 = if f & 1 != 0 { m.pos.0 + t.x } else { t.x };
        m.pos.1 = if f & 2 != 0 { m.pos.1 + t.y } else { t.y };
        m.pos.2 = if f & 4 != 0 { m.pos.2 + t.z } else { t.z };
        m.yaw = if f & 8 != 0 { m.yaw + t.yaw } else { t.yaw };
        m.pitch = if f & 16 != 0 {
            m.pitch + t.pitch
        } else {
            t.pitch
        };
        m.since_tp = 0;
        w.queue(&sb::TeleportConfirm {
            teleport_id: t.teleport_id,
        })?;
        w.queue(&sb::PositionLook {
            x: m.pos.0,
            y: m.pos.1,
            z: m.pos.2,
            yaw: m.yaw,
            pitch: m.pitch,
            on_ground: false,
        })?;
        d.spawned = true;
        Ok(())
    }
    fn use_chest(w: &mut Wr, _d: &mut Driver) -> Res<()> {
        w.queue(&sb::BlockPlace {
            hand: 0,
            location: CHEST,
            direction: 2,
            cursor_x: 0.5,
            cursor_y: 0.5,
            cursor_z: 0.0,
            inside_block: false,
        })?;
        w.queue(&sb::ArmAnimation { hand: 0 })?;
        Ok(())
    }
});
