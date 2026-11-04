//! Packet trace replay (test only).
//!
//! Replays a packet trace recorded by a reference mineflayer-based bot (jsonl, one event per
//! line: outbound packets, inbound block changes, looks) against the ghast engine and
//! compares every tick's outbound
//! packets: order and content. Two passes:
//!
//! 1. **Input inference.** The trace has packets, not keys. A beam search over the 1.8 physics
//!    (`ghast-physics`) finds, tick by tick, the key sets (and `set_flying` calls) whose
//!    sneak / sprint / abilities / movement packets equal the recorded ones bit for bit.
//! 2. **Engine replay.** A full [`Engine`] (world, inventory, hands, rotation, physics) runs a
//!    behaviour that repeats the trace's actions (`select_slot`, `place`, `dig`, clicks, chat,
//!    `set_flying`), aims at the recorded look and holds the inferred keys. Its outbound
//!    packets are compared with the trace per tick.
//!
//! The world is the lab wheat field (flat world, fields A and B, shifted in z) plus the
//! trace's block changes. Inputs come from the committed lab fixtures or environment
//! variables (see [`lab_trace_replay`]). Only clean traces recorded on a local lab server
//! with throwaway accounts are committed.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read};
use std::time::{Duration, Instant};

use bytes::{BufMut, BytesMut};
use ghast_physics::v1_8::PlayerState;
use ghast_physics::{Keys, Packet, Packets, TickInput};
use ghast_protocol::{BlockPos, RawPacket};

use crate::common::{
    Abilities, Controls, DigStatus, Face, GameMode, Inbound, JoinGame, MovePacket, Outbound,
    Teleport, Vec3,
};
use crate::engine::{Behavior, CoreOptions, Engine, TickContext};
use crate::sim::{Sim, Sim18};
use crate::version::GameVersion;

// ---- trace -------------------------------------------------------------------------------

/// One trace line: its tick, direction, packet id, the parsed JSON and the raw text (numbers
/// are parsed from the text so doubles stay exact).
#[derive(Debug, Clone)]
struct Line {
    t: u64,
    dir: Option<char>,
    p: String,
    e: String,
    v: serde_json::Value,
    raw: String,
}

impl Line {
    /// The raw text of a numeric field (`"key":<number>`).
    fn raw_num(&self, key: &str) -> Option<&str> {
        let pat = format!("\"{key}\":");
        let i = self.raw.find(&pat)? + pat.len();
        let rest = &self.raw[i..];
        let end = rest.find([',', '}', ']']).unwrap_or(rest.len());
        Some(&rest[..end])
    }

    fn f64(&self, key: &str) -> f64 {
        self.raw_num(key)
            .and_then(|s| s.parse().ok())
            .unwrap_or(f64::NAN)
    }

    fn f32(&self, key: &str) -> f32 {
        self.raw_num(key)
            .and_then(|s| s.parse().ok())
            .unwrap_or(f32::NAN)
    }

    fn i64(&self, key: &str) -> i64 {
        self.v
            .get(key)
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0)
    }

    fn str(&self, key: &str) -> &str {
        self.v.get(key).and_then(|v| v.as_str()).unwrap_or("")
    }

    fn pos(&self, key: &str) -> BlockPos {
        let a = self.v.get(key).and_then(|v| v.as_array());
        let c = |i: usize| {
            a.and_then(|a| a.get(i))
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0) as i32
        };
        BlockPos::new(c(0), c(1), c(2))
    }

    /// The `h` hit offsets of a C08 as `f32` (parsed from the text).
    fn hit(&self) -> [f32; 3] {
        let i = self.raw.find("\"h\":[").map(|i| i + 5);
        let mut out = [f32::NAN; 3];
        if let Some(i) = i {
            let rest = &self.raw[i..];
            let end = rest.find(']').unwrap_or(rest.len());
            for (k, s) in rest[..end].split(',').enumerate().take(3) {
                out[k] = s.trim().parse().unwrap_or(f32::NAN);
            }
        }
        out
    }

    fn is_move(&self) -> bool {
        matches!(self.p.as_str(), "C03" | "C04" | "C05" | "C06")
    }

    /// Short text for reports.
    fn short(&self) -> String {
        let mut s = self.raw.clone();
        for k in ["\"t\":", "\"ms\":"] {
            if let Some(i) = s.find(k) {
                let end = s[i..].find(',').map_or(s.len(), |e| i + e + 1);
                s.replace_range(i..end, "");
            }
        }
        s
    }
}

/// Parses a trace (plain or gzip; a truncated gzip stream ends the trace where it breaks).
fn read_trace(path: &str) -> Vec<Line> {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let reader: Box<dyn Read> = if path.ends_with(".gz") {
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut out = Vec::new();
    for l in BufReader::new(reader).lines() {
        let Ok(raw) = l else { break };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
            break;
        };
        out.push(Line {
            t: v.get("t").and_then(serde_json::Value::as_u64).unwrap_or(0),
            dir: v
                .get("d")
                .and_then(|d| d.as_str())
                .and_then(|d| d.chars().next()),
            p: v.get("p").and_then(|d| d.as_str()).unwrap_or("").to_owned(),
            e: v.get("e").and_then(|d| d.as_str()).unwrap_or("").to_owned(),
            v,
            raw,
        });
    }
    out
}

/// One tick of the trace: inbound applied before it, its own outbound packets (actions and
/// movement) and the immediate replies to inbound packets (C0F, C00, the S08 C06).
#[derive(Debug, Default, Clone)]
struct TraceTick {
    inbound: Vec<Line>,
    out: Vec<Line>,
    replies: Vec<Line>,
}

/// Splits the trace into ticks. Inbound lines carry the tick they are applied before.
fn ticks(lines: &[Line]) -> BTreeMap<u64, TraceTick> {
    let mut m: BTreeMap<u64, TraceTick> = BTreeMap::new();
    let mut after_teleport = false;
    for l in lines {
        match l.dir {
            Some('i') => {
                after_teleport = l.p == "S08";
                m.entry(l.t).or_default().inbound.push(l.clone());
            }
            Some('o') => {
                let tick = m.entry(l.t).or_default();
                if matches!(l.p.as_str(), "C0F" | "C00") || (after_teleport && l.p == "C06") {
                    tick.replies.push(l.clone());
                } else {
                    tick.out.push(l.clone());
                }
                after_teleport = false;
            }
            _ => {}
        }
    }
    m
}

// ---- world -------------------------------------------------------------------------------

/// The lab wheat field (flat world: bedrock, 2 dirt, grass at y = 3), shifted by `dz` in z
/// (0 = the original field layout, 64 = the layout the committed traces were recorded on).
fn field_block(x: i32, y: i32, z: i32, dz: i32) -> u16 {
    const AIR: u16 = 0;
    const BEDROCK: u16 = 7 << 4;
    const DIRT: u16 = 3 << 4;
    const GRASS: u16 = 2 << 4;
    const FARMLAND7: u16 = 60 << 4 | 7;
    const WATER: u16 = 9 << 4;
    const WHEAT7: u16 = 59 << 4 | 7;
    let zr = z - dz;
    let a = (0..=31).contains(&x);
    let b = (40..=71).contains(&x);
    let rows = (8..=25).contains(&zr);
    let water_row = zr == 12 || zr == 21;
    match y {
        0 => BEDROCK,
        1 | 2 => DIRT,
        3 if (a || b) && rows && water_row => WATER,
        3 if (a || b) && rows => FARMLAND7,
        3 => GRASS,
        4 if b && rows && !water_row => WHEAT7,
        _ => AIR,
    }
}

/// A 1.8 ground-up chunk (one section, y 0–15) of [`field_block`].
fn field_chunk(cx: i32, cz: i32, dz: i32) -> RawPacket {
    let mut d = BytesMut::with_capacity(12_544);
    for i in 0..4096 {
        let (y, z, x) = (i >> 8, (i >> 4) & 15, i & 15);
        d.put_u16_le(field_block(cx * 16 + x, y, cz * 16 + z, dz));
    }
    d.put_bytes(0, 2048);
    d.put_bytes(0xff, 2048);
    d.put_bytes(1, 256);
    RawPacket::from_packet(&ghast_protocol::v1_8::play::clientbound::MapChunk {
        x: cx,
        z: cz,
        ground_up: true,
        bit_map: 1,
        chunk_data: d.freeze(),
    })
    .unwrap()
}

/// `"id:meta"` → 1.8 block state.
fn block_state(s: &str) -> i32 {
    let mut it = s.split(':');
    let id: i32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let meta: i32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    id << 4 | meta
}

/// `"id:metaxcount"` → stack.
fn stack(s: Option<&str>) -> Option<ghast_inventory::Stack> {
    let s = s?;
    let (id, rest) = s.split_once(':')?;
    let (meta, count) = rest.split_once('x')?;
    Some(ghast_inventory::Stack::new(
        id.parse().ok()?,
        count.parse().ok()?,
        meta.parse().ok()?,
    ))
}

fn stack_text(s: Option<&ghast_inventory::Stack>) -> String {
    s.map_or_else(
        || "null".to_owned(),
        |s| format!("{}:{}x{}", s.id, s.damage, s.count),
    )
}

/// The world packets of an inbound line (block changes), as raw packets.
fn world_packets(l: &Line) -> Vec<RawPacket> {
    use ghast_protocol::v1_8::play::clientbound as cb;
    match l.p.as_str() {
        "S23" => vec![
            RawPacket::from_packet(&cb::BlockChange {
                location: l.pos("pos"),
                r#type: block_state(l.str("b")),
            })
            .unwrap(),
        ],
        "S22" => l
            .v
            .get("c")
            .and_then(|c| c.as_array())
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let a = e.as_array()?;
                let n = |i: usize| a.get(i).and_then(serde_json::Value::as_i64).unwrap_or(0) as i32;
                Some(
                    RawPacket::from_packet(&cb::BlockChange {
                        location: BlockPos::new(n(0), n(1), n(2)),
                        r#type: block_state(a.get(3)?.as_str()?),
                    })
                    .unwrap(),
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn teleport_of(l: &Line) -> Teleport {
    Teleport {
        x: l.f64("x"),
        y: l.f64("y"),
        z: l.f64("z"),
        yaw: l.f32("yaw"),
        pitch: l.f32("pitch"),
        flags: l.i64("rel") as i8,
        teleport_id: None,
    }
}

fn abilities_of(l: &Line, allow: bool) -> Abilities {
    let flying = l.v.get("flying").and_then(serde_json::Value::as_bool) == Some(true);
    let allow =
        l.v.get("allow")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(allow);
    Abilities {
        flags: if flying { 2 } else { 0 } | if allow { 4 } else { 0 },
        flying_speed: l.v.get("fly").map_or(0.05, |_| l.f32("fly")),
        walking_speed: l.v.get("walk").map_or(0.1, |_| l.f32("walk")),
    }
}

// ---- packet comparison -------------------------------------------------------------------

/// Position tolerance per axis and tick. The reference bot's flight physics uses `Math.sin`
/// instead of vanilla's sine table: its positions differ from vanilla
/// (and ghast) by about 1e-6 a tick. Everything else is compared exactly.
const POS_TOL: f64 = 1e-4;

/// A movement trace line as the physics packet it should be (parsed once per tick).
fn expected(l: &Line) -> Packet {
    let g = l.i64("g") == 1;
    match l.p.as_str() {
        "C0B" => match l.str("a") {
            "START_SNEAKING" => Packet::StartSneaking,
            "STOP_SNEAKING" => Packet::StopSneaking,
            "START_SPRINTING" => Packet::StartSprinting,
            _ => Packet::StopSprinting,
        },
        "C13" => Packet::Abilities(ghast_physics::Abilities {
            flying: l.v.get("flying").and_then(serde_json::Value::as_bool) == Some(true),
            ..ghast_physics::Abilities::default()
        }),
        "C03" => Packet::Flying { on_ground: g },
        "C04" => Packet::Position {
            x: l.f64("x"),
            y: l.f64("y"),
            z: l.f64("z"),
            on_ground: g,
        },
        "C05" => Packet::Look {
            yaw: l.f32("yaw"),
            pitch: l.f32("pitch"),
            on_ground: g,
        },
        _ => Packet::PositionLook {
            x: l.f64("x"),
            y: l.f64("y"),
            z: l.f64("z"),
            yaw: l.f32("yaw"),
            pitch: l.f32("pitch"),
            on_ground: g,
        },
    }
}

/// Whether a physics packet matches the expected one (positions within `tol`, abilities by
/// the flying flag, everything else exactly).
fn physics_matches(p: &Packet, w: &Packet, tol: f64) -> bool {
    let close = |a: f64, b: f64| (a - b).abs() <= tol;
    match (*p, *w) {
        (Packet::Abilities(a), Packet::Abilities(b)) => a.flying == b.flying,
        (
            Packet::Position { x, y, z, on_ground },
            Packet::Position {
                x: wx,
                y: wy,
                z: wz,
                on_ground: wg,
            },
        ) => close(x, wx) && close(y, wy) && close(z, wz) && on_ground == wg,
        (
            Packet::PositionLook {
                x,
                y,
                z,
                yaw,
                pitch,
                on_ground,
            },
            Packet::PositionLook {
                x: wx,
                y: wy,
                z: wz,
                yaw: wyaw,
                pitch: wpitch,
                on_ground: wg,
            },
        ) => {
            close(x, wx)
                && close(y, wy)
                && close(z, wz)
                && yaw == wyaw
                && pitch == wpitch
                && on_ground == wg
        }
        (a, b) => a == b,
    }
}

/// The movement part of a tick (what the physics writes): C0B sneak / sprint, C13, C03–C06.
fn movement_lines(tick: &TraceTick) -> Vec<&Line> {
    tick.out
        .iter()
        .filter(|l| {
            l.is_move()
                || l.p == "C13"
                || (l.p == "C0B"
                    && matches!(
                        l.str("a"),
                        "START_SNEAKING" | "STOP_SNEAKING" | "START_SPRINTING" | "STOP_SPRINTING"
                    ))
        })
        .collect()
}

/// An engine packet in trace notation (`C08 pos=… f=… h16=… i=…`) for comparison.
fn engine_text(o: &Outbound) -> Option<String> {
    Some(match o {
        Outbound::Move(MovePacket::Flying { on_ground }) => {
            format!("C03 g={}", u8::from(*on_ground))
        }
        Outbound::Move(MovePacket::Position { pos, on_ground }) => format!(
            "C04 {:?} {:?} {:?} g={}",
            pos.x,
            pos.y,
            pos.z,
            u8::from(*on_ground)
        ),
        Outbound::Move(MovePacket::Look {
            yaw,
            pitch,
            on_ground,
        }) => format!("C05 {yaw:?} {pitch:?} g={}", u8::from(*on_ground)),
        Outbound::Move(MovePacket::PositionLook {
            pos,
            yaw,
            pitch,
            on_ground,
        }) => format!(
            "C06 {:?} {:?} {:?} {yaw:?} {pitch:?} g={}",
            pos.x,
            pos.y,
            pos.z,
            u8::from(*on_ground)
        ),
        Outbound::EntityAction { action, .. } => {
            use crate::common::EntityAction as A;
            let a = match action {
                A::StartSneaking => "START_SNEAKING",
                A::StopSneaking => "STOP_SNEAKING",
                A::StartSprinting => "START_SPRINTING",
                A::StopSprinting => "STOP_SPRINTING",
            };
            format!("C0B {a}")
        }
        Outbound::Abilities { flags, .. } => format!("C13 flying={}", flags & 2 != 0),
        Outbound::HeldSlot(s) => format!("C09 {s}"),
        Outbound::Swing => "C0A".to_owned(),
        Outbound::Dig { status, pos, face } => {
            let a = match status {
                DigStatus::Start => "START_DESTROY_BLOCK",
                DigStatus::Abort => "ABORT_DESTROY_BLOCK",
                DigStatus::Finish => "STOP_DESTROY_BLOCK",
                DigStatus::ReleaseUseItem => "RELEASE_USE_ITEM",
            };
            format!("C07 {a} {} {} {} f={}", pos.x, pos.y, pos.z, face.id())
        }
        Outbound::Place {
            pos,
            face,
            cursor,
            held,
            ..
        } => format!(
            "C08 {} {} {} f={} h16={},{},{} i={}",
            pos.x,
            pos.y,
            pos.z,
            face.id(),
            (cursor.x as f32 * 16.0) as i32,
            (cursor.y as f32 * 16.0) as i32,
            (cursor.z as f32 * 16.0) as i32,
            stack_text(held.as_ref())
        ),
        Outbound::UseItem { held, .. } => format!("C08 use i={}", stack_text(held.as_ref())),
        Outbound::Chat(s) => format!("C01 {s}"),
        Outbound::Inventory(ghast_inventory::Outgoing::Click(c)) => format!(
            "C0E w={} slot={} btn={} mode={} n={} i={}",
            c.window_id,
            c.slot,
            c.button,
            c.mode,
            c.action,
            stack_text(c.item.as_ref())
        ),
        Outbound::Inventory(ghast_inventory::Outgoing::CloseWindow(_)) => "C0D".to_owned(),
        Outbound::Inventory(ghast_inventory::Outgoing::OpenInventory) => {
            "C16 OPEN_INVENTORY_ACHIEVEMENT".to_owned()
        }
        // Replies to inbound packets are compared on their own.
        Outbound::Inventory(ghast_inventory::Outgoing::Confirm { .. })
        | Outbound::KeepAlive(_)
        | Outbound::TeleportConfirm(_) => return None,
        other => format!("{other:?}"),
    })
}

/// Replaces the position of a C04 / C06 text by `pos` (compared numerically).
fn mask(t: &str) -> String {
    let mut f: Vec<&str> = t.split(' ').collect();
    match f.first() {
        Some(&"C04" | &"C06") if f.len() >= 4 => {
            f.splice(1..4, ["pos"]);
            f.join(" ")
        }
        _ => t.to_owned(),
    }
}

/// A trace line in the notation of [`engine_text`].
fn trace_text(l: &Line) -> String {
    match l.p.as_str() {
        "C03" => format!("C03 g={}", l.i64("g")),
        "C04" => format!(
            "C04 {:?} {:?} {:?} g={}",
            l.f64("x"),
            l.f64("y"),
            l.f64("z"),
            l.i64("g")
        ),
        "C05" => format!(
            "C05 {:?} {:?} g={}",
            l.f32("yaw"),
            l.f32("pitch"),
            l.i64("g")
        ),
        "C06" => format!(
            "C06 {:?} {:?} {:?} {:?} {:?} g={}",
            l.f64("x"),
            l.f64("y"),
            l.f64("z"),
            l.f32("yaw"),
            l.f32("pitch"),
            l.i64("g")
        ),
        "C0B" => format!("C0B {}", l.str("a")),
        "C13" => format!(
            "C13 flying={}",
            l.v.get("flying").and_then(serde_json::Value::as_bool) == Some(true)
        ),
        "C09" => format!("C09 {}", l.i64("slot")),
        "C0A" => "C0A".to_owned(),
        "C07" => {
            let p = l.pos("pos");
            format!(
                "C07 {} {} {} {} f={}",
                l.str("a"),
                p.x,
                p.y,
                p.z,
                l.i64("f")
            )
        }
        "C08" if l.i64("f") == 255 => format!("C08 use i={}", l.str("i")),
        "C08" => {
            let p = l.pos("pos");
            let h = l.hit();
            let i = l.v.get("i").and_then(|v| v.as_str()).unwrap_or("null");
            format!(
                "C08 {} {} {} f={} h16={},{},{} i={i}",
                p.x,
                p.y,
                p.z,
                l.i64("f"),
                (h[0] * 16.0) as i32,
                (h[1] * 16.0) as i32,
                (h[2] * 16.0) as i32
            )
        }
        "C01" => format!("C01 {}", l.str("s")),
        "C0E" => format!(
            "C0E w={} slot={} btn={} mode={} n={} i={}",
            l.i64("w"),
            l.i64("slot"),
            l.i64("btn"),
            l.i64("mode"),
            l.i64("n"),
            l.v.get("i").and_then(|v| v.as_str()).unwrap_or("null")
        ),
        "C0D" => "C0D".to_owned(),
        "C16" => format!("C16 {}", l.str("a")),
        _ => l.short(),
    }
}

/// The position a tick's movement packet carries, if any.
fn sent_position(lines: &[Line]) -> Option<[f64; 3]> {
    lines
        .iter()
        .rev()
        .find(|l| l.p == "C04" || l.p == "C06")
        .map(|l| [l.f64("x"), l.f64("y"), l.f64("z")])
}

/// Moves the physics player (`after` one tick from `before`) onto the recorded position
/// (feet y = box minimum) so the reference bot's float drift does not accumulate. The velocity is
/// scaled per axis by recorded / own displacement: a velocity a hair over a vanilla threshold
/// (`|motion| < 0.005` → 0) would otherwise split the two runs.
fn snap(before: &PlayerState, after: &mut PlayerState, pos: [f64; 3]) {
    let from = [before.pos[0], before.bb.min_y, before.pos[2]];
    let own = [after.pos[0], after.bb.min_y, after.pos[2]];
    for i in 0..3 {
        let d_own = own[i] - from[i];
        let d_rec = pos[i] - from[i];
        if d_own.abs() > 1e-9 && (d_rec / d_own).is_finite() {
            let r = d_rec / d_own;
            if (0.5..=2.0).contains(&r) {
                after.vel[i] *= r;
            }
        }
    }
    after.set_position(pos[0], pos[1], pos[2]);
    after.sent.pos = pos;
}

// ---- pass 1: input inference -------------------------------------------------------------

/// What the behaviour did to the movement on one tick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Input {
    keys: Keys,
    /// `set_flying` called before the movement step.
    fly: Option<bool>,
}

/// Every key set: none first, then by key count (the search keeps the first of equal states).
fn key_sets() -> Vec<Keys> {
    let mut v = Vec::new();
    for fb in 0..3 {
        for lr in 0..3 {
            for jump in [false, true] {
                for sneak in [false, true] {
                    for sprint in [false, true] {
                        v.push(Keys {
                            forward: fb == 1,
                            back: fb == 2,
                            left: lr == 1,
                            right: lr == 2,
                            jump,
                            sneak,
                            sprint,
                        });
                    }
                }
            }
        }
    }
    let n = |k: &Keys| {
        [
            k.forward, k.back, k.left, k.right, k.jump, k.sneak, k.sprint,
        ]
        .iter()
        .filter(|b| **b)
        .count()
    };
    v.sort_by_key(n);
    v
}

/// A search hypothesis: physics state and the arena index of its input history.
#[derive(Debug, Clone, Copy)]
struct Hyp {
    p: PlayerState,
    node: usize,
}

/// Result of the inference.
struct Inferred {
    /// First traced tick, initial physics state (its position-refresh counter is unknown:
    /// the engine pass tries each), inputs per tick from there.
    start: u64,
    initial: PlayerState,
    inputs: Vec<Input>,
    /// The tick where no input reproduced the trace (the replay stops there).
    failed_at: Option<(u64, String)>,
    /// Ticks reproduced only with another position (the reference bot left vanilla physics): tick
    /// and the largest axis deviation.
    deviations: Vec<(u64, f64)>,
    max_beam: usize,
}

const BEAM: usize = 512;
/// Initial hypotheses: the position-refresh counter 0..20.
const ROOTS: usize = 20;

/// Pass 1: beam search over physics states.
fn infer(
    header: &Line,
    ticks: &BTreeMap<u64, TraceTick>,
    world: &mut Sim18,
    dz: i32,
    limit: u64,
) -> Inferred {
    let pos = header.v.get("pos").and_then(|v| v.as_array()).unwrap();
    let pf = |i: usize| pos[i].as_f64().unwrap();
    let (x, y, z) = (pf(0), pf(1), pf(2));
    let yaw = header.f32("yaw");
    let pitch = header.f32("pitch");
    let flying = header.v.get("flying").and_then(serde_json::Value::as_bool) == Some(true);
    let _ = dz;

    // Settle a fresh player standing at the header position (velocity, ground flags).
    let mut base = PlayerState::new([x, y, z], yaw, pitch);
    base.abilities.allow_flying = true;
    base.abilities.flying = flying;
    base.abilities.walk_speed = header.f32("walkSpeed");
    base.abilities.fly_speed = header.f32("flySpeed");
    let mut pk = Packets::new();
    for _ in 0..40 {
        pk.clear();
        let input = TickInput {
            keys: Keys::NONE,
            yaw,
            pitch,
            using_item: false,
        };
        ghast_physics::v1_8::tick(&mut base, &input, world.test_world(), &mut pk);
    }
    base.sent.pos = [base.pos[0], base.bb.min_y, base.pos[2]];
    base.sent.yaw = yaw;
    base.sent.pitch = pitch;

    // Arena of inputs: (parent, input).
    // Arena of inputs: (parent, input); nodes 0..20 are the roots, one per initial
    // position-refresh counter (unknown: the trace starts mid-session).
    let mut arena: Vec<(usize, Input)> = vec![(usize::MAX, Input::default()); ROOTS];
    let mut beam: Vec<Hyp> = (0..ROOTS)
        .map(|n| {
            let mut p = base;
            p.sent.ticks_since_pos = n as i32;
            Hyp { p, node: n }
        })
        .collect();
    let keys = key_sets();
    let start = header.t + 1;
    let last = ticks
        .keys()
        .next_back()
        .copied()
        .unwrap_or(start)
        .min(start + limit);
    let mut failed_at = None;
    let mut deviations: Vec<(u64, f64)> = Vec::new();
    let mut max_beam = beam.len();
    let empty = TraceTick::default();
    for t in start..=last {
        let tick = ticks.get(&t).unwrap_or(&empty);
        for l in &tick.inbound {
            for raw in world_packets(l) {
                world.world_packet(&raw).unwrap();
            }
            match l.p.as_str() {
                "S08" => {
                    let tp = teleport_of(l);
                    let rel = [
                        tp.flags & 1 != 0,
                        tp.flags & 2 != 0,
                        tp.flags & 4 != 0,
                        tp.flags & 8 != 0,
                        tp.flags & 16 != 0,
                    ];
                    for h in &mut beam {
                        let _ =
                            h.p.apply_teleport([tp.x, tp.y, tp.z], tp.yaw, tp.pitch, rel);
                    }
                }
                "S39" => {
                    for h in &mut beam {
                        let a = abilities_of(l, h.p.abilities.allow_flying);
                        h.p.apply_abilities(ghast_physics::Abilities {
                            invulnerable: a.flags & 1 != 0,
                            flying: a.flags & 2 != 0,
                            allow_flying: a.flags & 4 != 0,
                            creative: a.flags & 8 != 0,
                            fly_speed: a.flying_speed,
                            walk_speed: a.walking_speed,
                        });
                    }
                }
                _ => {}
            }
        }
        let want = movement_lines(tick);
        let want_pk: Vec<Packet> = want.iter().map(|l| expected(l)).collect();
        let look = want
            .iter()
            .rev()
            .find(|l| l.p == "C05" || l.p == "C06")
            .map(|l| (l.f32("yaw"), l.f32("pitch")));
        let c13 = want
            .iter()
            .find(|l| l.p == "C13")
            .map(|l| l.v.get("flying").and_then(serde_json::Value::as_bool) == Some(true));
        let flys: Vec<Option<bool>> = match c13 {
            Some(f) => vec![None, Some(f)],
            None => vec![None],
        };
        let mut next: Vec<Hyp> = Vec::new();
        let target_pos = sent_position(&tick.out);
        let mut loose: Option<(f64, Hyp, Input, PlayerState)> = None;
        let mut loose_state = beam[0].p;
        for h in &beam {
            let (yaw, pitch) = look.unwrap_or((h.p.sent.yaw, h.p.sent.pitch));
            for &fly in &flys {
                let mut p0 = h.p;
                let mut pre = 0;
                if let Some(f) = fly {
                    if p0.abilities.flying == f || (f && !p0.abilities.allow_flying) {
                        continue;
                    }
                    p0.abilities.flying = f;
                    pre = 1;
                    if want.first().is_none_or(|l| l.p != "C13") {
                        continue;
                    }
                }
                // The previous tick's keys first; when they reproduce a sent position of the only
                // hypothesis, nothing else is tried (the common case: held keys).
                let prev = arena[h.node].1.keys;
                let order: Vec<Keys> = std::iter::once(prev)
                    .chain(keys.iter().copied().filter(|k| *k != prev))
                    .collect();
                for k in &order {
                    let mut p = p0;
                    pk.clear();
                    let input = TickInput {
                        keys: *k,
                        yaw,
                        pitch,
                        using_item: false,
                    };
                    ghast_physics::v1_8::tick(&mut p, &input, world.test_world(), &mut pk);
                    let got = pk.as_slice();
                    let fits = |tol: f64| {
                        got.len() + pre == want.len()
                            && got
                                .iter()
                                .zip(&want_pk[pre..])
                                .all(|(g, w)| physics_matches(g, w, tol))
                    };
                    if !fits(POS_TOL) {
                        // Same packets, other position: the reference bot's physics left vanilla here.
                        if next.is_empty() && fits(f64::INFINITY) {
                            let d = target_pos.map_or(0.0, |w| {
                                (0..3).map(|i| (p.pos[i] - w[i]).abs()).fold(0.0, f64::max)
                            });
                            if loose
                                .as_ref()
                                .is_none_or(|b: &(f64, Hyp, Input, PlayerState)| d < b.0)
                            {
                                loose = Some((d, *h, Input { keys: *k, fly }, h.p));
                                loose_state = p;
                            }
                        }
                        continue;
                    }
                    if let Some(pos) = target_pos {
                        snap(&h.p, &mut p, pos);
                    }
                    if next.iter().any(|n| n.p == p) {
                        continue;
                    }
                    arena.push((h.node, Input { keys: *k, fly }));
                    next.push(Hyp {
                        p,
                        node: arena.len() - 1,
                    });
                    if *k == prev && beam.len() == 1 && fly.is_none() && target_pos.is_some() {
                        break;
                    }
                    if next.len() >= BEAM {
                        break;
                    }
                }
            }
        }
        if next.is_empty()
            && let Some((d, h, input, before)) = loose
            && d < 0.5
        {
            let mut p = loose_state;
            if let Some(pos) = target_pos {
                snap(&before, &mut p, pos);
            }
            deviations.push((t, d));
            arena.push((h.node, input));
            next.push(Hyp {
                p,
                node: arena.len() - 1,
            });
        }
        if next.is_empty() {
            let mut why = String::new();
            let _ = write!(why, "want [");
            for l in &want {
                let _ = write!(why, "{} | ", trace_text(l));
            }
            // The nearest candidates of the first hypothesis.
            let h = beam[0];
            let (yaw, pitch) = look.unwrap_or((h.p.sent.yaw, h.p.sent.pitch));
            let target = want
                .iter()
                .find(|l| l.p == "C04" || l.p == "C06")
                .map(|l| [l.f64("x"), l.f64("y"), l.f64("z")]);
            let mut cands: Vec<(f64, String)> = Vec::new();
            for &fly in &flys {
                for k in &keys {
                    let mut p = h.p;
                    if let Some(f) = fly {
                        p.abilities.flying = f;
                    }
                    pk.clear();
                    let input = TickInput {
                        keys: *k,
                        yaw,
                        pitch,
                        using_item: false,
                    };
                    ghast_physics::v1_8::tick(&mut p, &input, world.test_world(), &mut pk);
                    let pos = [p.pos[0], p.bb.min_y, p.pos[2]];
                    let d = target.map_or(0.0, |t| {
                        (0..3).map(|i| (pos[i] - t[i]).powi(2)).sum::<f64>().sqrt()
                    });
                    cands.push((d, format!("fly {fly:?} keys {k:?} -> {:?}", pk.as_slice())));
                }
            }
            cands.sort_by(|a, b| a.0.total_cmp(&b.0));
            let _ = write!(
                why,
                "]; beam {}; state before pos {:?} vel {:?} ground {} flying {} sneaking {} sprinting {}; nearest:",
                beam.len(),
                h.p.pos,
                h.p.vel,
                h.p.on_ground,
                h.p.abilities.flying,
                h.p.sent.sneaking,
                h.p.sprinting
            );
            for (d, c) in cands.iter().take(3) {
                let _ = write!(why, "\n    d={d:.3e} {c}");
            }
            failed_at = Some((t, why));
            break;
        }
        max_beam = max_beam.max(next.len());
        beam = next;
    }
    // Follow the first surviving hypothesis back to its root.
    let mut inputs = Vec::new();
    let mut node = beam[0].node;
    while node >= ROOTS {
        inputs.push(arena[node].1);
        node = arena[node].0;
    }
    inputs.reverse();
    let mut initial = base;
    initial.sent.ticks_since_pos = node as i32;
    Inferred {
        deviations,
        start,
        initial,
        inputs,
        failed_at,
        max_beam,
    }
}

// ---- pass 2: engine replay ---------------------------------------------------------------

/// The replay behaviour: per tick, the trace's actions, the recorded look and the keys.
struct Script<'a> {
    ticks: &'a BTreeMap<u64, TraceTick>,
    inputs: &'a [Input],
    start: u64,
    t: u64,
    forced_looks: u64,
    forced_examples: Vec<String>,
    refused: Vec<String>,
}

impl Behavior for Script<'_> {
    fn name(&self) -> &str {
        "trace-replay"
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        let t = self.t;
        let Some(input) = self.inputs.get((t - self.start) as usize).copied() else {
            return;
        };
        let k = input.keys;
        ctx.set_controls(Controls {
            forward: k.forward,
            back: k.back,
            left: k.left,
            right: k.right,
            jump: k.jump,
            sneak: k.sneak,
            sprint: k.sprint,
        });
        if let Some(f) = input.fly {
            ctx.set_flying(f);
        }
        let empty = TraceTick::default();
        let tick = self.ticks.get(&t).unwrap_or(&empty);
        for l in &tick.out {
            let ok = match l.p.as_str() {
                "C09" => {
                    ctx.select_slot(l.i64("slot") as u8);
                    true
                }
                "C08" if l.i64("f") != 255 => {
                    let p = l.pos("pos");
                    let h = l.hit();
                    let face = Face::from_id(l.i64("f") as i8).unwrap_or(Face::Up);
                    let hit = Vec3::new(
                        f64::from(p.x) + f64::from(h[0]),
                        f64::from(p.y) + f64::from(h[1]),
                        f64::from(p.z) + f64::from(h[2]),
                    );
                    ctx.place(p, face, hit)
                }
                "C07" if l.str("a") == "START_DESTROY_BLOCK" => {
                    let face = Face::from_id(l.i64("f") as i8).unwrap_or(Face::Up);
                    ctx.dig(l.pos("pos"), face)
                }
                "C01" => {
                    ctx.chat(l.str("s").to_owned());
                    true
                }
                _ => true,
            };
            if !ok {
                self.refused
                    .push(format!("t{t} refused: {}", trace_text(l)));
            }
        }
        // The look of this tick's movement packet (unchanged without one).
        let look = tick
            .out
            .iter()
            .rev()
            .find(|l| l.p == "C05" || l.p == "C06")
            .map(|l| (l.f32("yaw"), l.f32("pitch")));
        if let Some((yaw, pitch)) = look {
            let pose = ctx.pose();
            // Aim at the recorded look; when the rotation step would not land on it exactly
            // (a speed-limited turn whose target the trace does not know), set it directly.
            // A speed-limited turn is tried with targets beyond the recorded look (the real
            // target was further: the step stops at the per-tick limit).
            let (dy, dp) = (yaw - pose.yaw, pitch - pose.pitch);
            // Beyond-targets first: a step that stops exactly at the limit keeps its carried
            // budget only when the target was further.
            let targets = [
                (yaw + 2.0 * dy, pitch + 2.0 * dp),
                (yaw + 2.0 * dy, pitch),
                (yaw, pitch + 2.0 * dp),
                (yaw, pitch),
            ];
            let hit = targets.into_iter().find(|&(ty, tp)| {
                let mut r = ctx.test_rotation();
                r.set_target(ty, tp);
                r.step(pose.yaw, pose.pitch) == (yaw, pitch)
            });
            if let Some((ty, tp)) = hit {
                ctx.set_aim(ty, tp);
            } else {
                self.forced_looks += 1;
                if self.forced_examples.len() < 8 {
                    let step = f64::from(crate::rotation::mouse_gcd(
                        ctx.test_rotation().sensitivity(),
                    ));
                    self.forced_examples.push(format!(
                        "t{t} yaw {} -> {yaw} ({:.4} counts) pitch {} -> {pitch} ({:.4} counts)",
                        pose.yaw,
                        f64::from(yaw - pose.yaw) / step,
                        pose.pitch,
                        f64::from(pitch - pose.pitch) / step
                    ));
                }
                ctx.test_force_look(yaw, pitch);
            }
        }
    }
}

/// Engine options for the replay.
fn opts(sensitivity: f32) -> CoreOptions {
    CoreOptions {
        name: "replay".into(),
        version: GameVersion::V1_8_9,
        settings: crate::common::ClientSettings::default(),
        brand: Some("vanilla".into()),
        sensitivity,
        silent_hold: Duration::from_secs(3600),
        inventory: ghast_inventory::Version::V1_8,
        hold_turn_on_action: true,
    }
}

/// Report of one replay.
struct Report {
    text: String,
    /// Pass 1 found inputs for every tick.
    complete: bool,
    ticks: u64,
    matched: u64,
    mismatched: u64,
}

#[allow(clippy::too_many_lines)]
fn replay(path: &str, dz: i32, sensitivity: f32, limit: u64) -> Report {
    let lines = read_trace(path);
    let header = lines
        .iter()
        .find(|l| l.e == "start")
        .expect("start line")
        .clone();
    let ticks = ticks(&lines);
    let (pcx, pcz) = {
        let pos = header.v.get("pos").and_then(|v| v.as_array()).unwrap();
        (
            (pos[0].as_f64().unwrap().floor() as i32) >> 4,
            (pos[2].as_f64().unwrap().floor() as i32) >> 4,
        )
    };
    let load = |s: &mut Sim18| {
        for cx in pcx - 6..=pcx + 6 {
            for cz in pcz - 6..=pcz + 6 {
                s.world_packet(&field_chunk(cx, cz, dz)).unwrap();
            }
        }
    };
    let mut text = String::new();

    // Pass 1.
    let mut world = Sim18::new();
    load(&mut world);
    let inf = infer(&header, &ticks, &mut world, dz, limit);
    let _ = writeln!(
        text,
        "pass 1: inputs for {} ticks from t{} (max beam {}); {} ticks only with another \
         position (reference physics not vanilla): {:?}{}",
        inf.inputs.len(),
        inf.start,
        inf.max_beam,
        inf.deviations.len(),
        inf.deviations
            .iter()
            .take(20)
            .map(|(t, d)| format!("t{t} {d:.3e}"))
            .collect::<Vec<_>>(),
        inf.failed_at
            .as_ref()
            .map_or(String::new(), |(t, w)| format!("; NO INPUT at t{t}: {w}"))
    );

    // Pass 2 from the initial state the inferred inputs came from.
    let mut r = engine_pass(&header, &ticks, &inf, inf.initial, dz, sensitivity, &load);
    r.text = text + &r.text;
    r.complete = inf.failed_at.is_none();
    r
}

fn engine_pass(
    header: &Line,
    ticks: &BTreeMap<u64, TraceTick>,
    inf: &Inferred,
    initial: PlayerState,
    _dz: i32,
    sensitivity: f32,
    load: &dyn Fn(&mut Sim18),
) -> Report {
    let mut sim = Sim18::new();
    load(&mut sim);
    let mut e = Engine::new(opts(sensitivity), sim);
    e.handle(Inbound::JoinGame(JoinGame {
        entity_id: 1,
        game_mode: GameMode::Survival,
        dimension: "0".into(),
        hardcore: false,
        dimension_nbt: None,
        respawn_screen: true,
    }));
    e.handle(Inbound::Teleport(Teleport {
        x: initial.pos[0],
        y: initial.bb.min_y,
        z: initial.pos[2],
        yaw: initial.yaw,
        pitch: initial.pitch,
        flags: 0,
        teleport_id: None,
    }));
    let a = initial.abilities;
    e.handle(Inbound::Abilities(Abilities {
        flags: if a.flying { 2 } else { 0 } | if a.allow_flying { 4 } else { 0 },
        flying_speed: a.fly_speed,
        walking_speed: a.walk_speed,
    }));
    {
        let (pose, sent) = e.core.test_poses();
        e.sim.test_set_player(pose, initial);
        *sent = *pose;
    }
    // Inventory from the header: main inventory 0..35 (0..8 = hotbar = window slots 36..44).
    if let Some(inv) = header.v.get("inv").and_then(|v| v.as_array()) {
        for (i, s) in inv.iter().enumerate() {
            let slot = if i < 9 { 36 + i } else { i } as i16;
            e.inventory_mut().on_set_slot(0, slot, stack(s.as_str()));
        }
    }
    e.core.test_set_slot(header.i64("slot") as u8);
    e.outbound().clear();

    let t0 = Instant::now();
    let mut script = Script {
        ticks,
        inputs: &inf.inputs,
        start: inf.start,
        t: 0,
        forced_looks: 0,
        forced_examples: Vec::new(),
        refused: Vec::new(),
    };
    let mut text = String::new();
    let (mut n, mut matched, mut mismatched) = (0u64, 0u64, 0u64);
    let mut reply_diffs = 0u64;
    let mut max_dev = 0.0f64;
    let mut pos_dev_ticks: Vec<(u64, f64)> = Vec::new();
    let mut kinds: BTreeMap<String, u64> = BTreeMap::new();
    let empty = TraceTick::default();
    let end = inf.start + inf.inputs.len() as u64;
    for t in inf.start..end {
        let now = t0 + Duration::from_millis(50 * (t - inf.start));
        let tick = ticks.get(&t).unwrap_or(&empty);
        for l in &tick.inbound {
            for raw in world_packets(l) {
                e.sim.world_packet(&raw).unwrap();
            }
            match l.p.as_str() {
                "S08" => e.handle(Inbound::Teleport(teleport_of(l))),
                "S39" => {
                    let allow = e.sim.test_player().abilities.allow_flying;
                    e.handle(Inbound::Abilities(abilities_of(l, allow)));
                }
                "S2F" => {
                    let w = l.i64("w") as i32;
                    let slot = l.i64("slot") as i16;
                    e.inventory_mut().on_set_slot(
                        w,
                        slot,
                        stack(l.v.get("i").and_then(|v| v.as_str())),
                    );
                }
                "S09" => e.handle(Inbound::HeldSlot(l.i64("slot") as i8)),
                _ => {}
            }
        }
        // Replies to this tick's inbound (C06 after S08): compared on their own.
        let replies: Vec<String> = e
            .outbound()
            .drain(..)
            .filter_map(|o| engine_text(&o))
            .collect();
        let want_replies: Vec<String> = tick
            .replies
            .iter()
            .filter(|l| l.p == "C06")
            .map(trace_text)
            .collect();
        if replies != want_replies {
            reply_diffs += 1;
            let _ = writeln!(text, "t{t} reply: ghast {replies:?} trace {want_replies:?}");
        }
        e.note_inbound(now);
        script.t = t;
        let before = *e.sim.test_player();
        e.tick(&mut script, now);
        let out: Vec<Outbound> = std::mem::take(e.outbound());
        let got: Vec<String> = out
            .iter()
            .filter_map(engine_text)
            .map(|t| mask(&t))
            .collect();
        let want: Vec<String> = tick.out.iter().map(|l| mask(&trace_text(l))).collect();
        // Positions: within the tolerance, then the engine continues from the recorded one.
        let got_pos = out.iter().rev().find_map(|o| match o {
            Outbound::Move(
                MovePacket::Position { pos, .. } | MovePacket::PositionLook { pos, .. },
            ) => Some([pos.x, pos.y, pos.z]),
            _ => None,
        });
        let want_pos = sent_position(&tick.out);
        let mut pos_ok = true;
        if let (Some(g), Some(w)) = (got_pos, want_pos) {
            let d = (0..3).map(|i| (g[i] - w[i]).abs()).fold(0.0, f64::max);
            max_dev = max_dev.max(d);
            pos_ok = d <= POS_TOL || inf.deviations.iter().any(|(dt, _)| *dt == t);
            let mut p = *e.sim.test_player();
            snap(&before, &mut p, w);
            if d > POS_TOL {
                pos_dev_ticks.push((t, d));
            }
            let (pose, sent) = e.core.test_poses();
            e.sim.test_set_player(pose, p);
            sent.pos = pose.pos;
        }
        n += 1;
        for l in &tick.out {
            *kinds.entry(l.p.clone()).or_insert(0u64) += 1;
        }
        if got == want && pos_ok {
            matched += 1;
        } else {
            mismatched += 1;
            if mismatched <= 12 {
                let _ = writeln!(text, "t{t} DIFF\n  ghast {got:?}\n  trace {want:?}");
            }
        }
    }
    let _ = writeln!(
        text,
        "pass 2: {n} ticks, {matched} identical, {mismatched} different (max position deviation \
         {max_dev:.2e}); {} looks set directly (speed-limited turns), {} turns held on action \
         ticks, {reply_diffs} reply diffs",
        script.forced_looks,
        e.core.test_held_turns()
    );
    let _ = writeln!(text, "  compared packets: {kinds:?}");
    if !pos_dev_ticks.is_empty() {
        let _ = writeln!(
            text,
            "  position deviations > {POS_TOL:e} (reference physics not vanilla there): {:?}",
            pos_dev_ticks
                .iter()
                .take(20)
                .map(|(t, d)| format!("t{t} {d:.3e}"))
                .collect::<Vec<_>>()
        );
    }
    for f in &script.forced_examples {
        let _ = writeln!(text, "  look set directly: {f}");
    }
    for r in script.refused.iter().take(12) {
        let _ = writeln!(text, "  {r}");
    }
    if script.refused.len() > 12 {
        let _ = writeln!(text, "  … {} refused actions", script.refused.len());
    }
    Report {
        text,
        complete: true,
        ticks: n,
        matched,
        mismatched,
    }
}

/// Replays the committed lab traces (`tests/fixtures/traces/`: wheat plant and harvest on a
/// local 1.8 lab server, 2026-11-04), or `GHAST_TRACE` (paths separated by `:`) when set.
/// The world is the lab wheat field shifted by `GHAST_TRACE_FIELD_DZ` (default 64: the
/// layout of the committed traces), the mouse sensitivity `GHAST_TRACE_SENSITIVITY` (default
/// 0.3380282, the recording bot's setting); `GHAST_TRACE_TICKS` limits the ticks. Pass =
/// every tick identical.
///
/// `GHAST_TRACE=a.jsonl.gz:b.jsonl.gz cargo test -p ghast-client --release --lib lab_trace_replay -- --nocapture`
#[test]
fn lab_trace_replay() {
    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/traces");
    let paths = std::env::var("GHAST_TRACE")
        .unwrap_or_else(|_| format!("{fixtures}/plant.jsonl.gz:{fixtures}/harvest.jsonl.gz"));
    let dz = std::env::var("GHAST_TRACE_FIELD_DZ")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64);
    let sens = std::env::var("GHAST_TRACE_SENSITIVITY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.338_028_2);
    let limit = std::env::var("GHAST_TRACE_TICKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(u64::MAX / 2);
    let mut all_ok = true;
    for path in paths.split(':') {
        let r = replay(path, dz, sens, limit);
        println!("== {path}\n{}", r.text);
        all_ok &= r.mismatched == 0 && r.ticks > 0 && r.complete;
        if path.starts_with(fixtures) {
            // The whole trace (an early stop would also pass the checks above).
            assert!(r.ticks >= 4480, "{path}: only {} ticks replayed", r.ticks);
            assert_eq!(r.matched, r.ticks, "{path}");
        }
    }
    assert!(all_ok, "replay differs");
}
