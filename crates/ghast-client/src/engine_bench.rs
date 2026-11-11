//! Engine cost probe (ignored test): a fixed per-tick workload through `Engine::handle` and
//! `Engine::tick` for each version, so a refactor of the version-agnostic core can be compared
//! by instruction count (`perfcount` on the test binary with `GHAST_BENCH_TICKS=N` vs `=0`).
//!
//! ```text
//! cargo test --release -p ghast-client --lib engine_bench -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use crate::Controls;
use crate::common::{
    ChatMessage, ChatPosition, ClientSettings, GameMode, Health, Inbound, JoinGame, Teleport, Vec3,
};
use crate::engine::{Behavior, CoreOptions, Engine, TickContext};
use crate::hands::Buttons;
use crate::sim::Sim;
use crate::version::GameVersion;

/// Walks a square, turns, sneaks now and then, holds the left button some of the time.
struct Walker;

impl Behavior for Walker {
    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        let t = ctx.tick();
        let leg = (t / 40) % 4;
        ctx.set_aim(leg as f32 * 90.0 + (t % 7) as f32, (t % 11) as f32);
        ctx.set_controls(Controls {
            forward: true,
            sprint: t % 80 < 40,
            sneak: t % 100 > 90,
            jump: t.is_multiple_of(33),
            ..Controls::NONE
        });
        ctx.set_buttons(Buttons {
            left: t % 50 < 10,
            right: false,
        });
        for m in &ctx.events().chat {
            std::hint::black_box(m.position);
        }
    }
}

fn opts(caps_version: GameVersion) -> CoreOptions {
    CoreOptions {
        name: "bench".into(),
        caps: caps_version.caps(),
        settings: ClientSettings::default(),
        brand: Some("vanilla".into()),
        sensitivity: 0.5,
        silent_hold: Duration::from_secs(3600),
        hold_turn_on_action: true,
        accept_required_packs: true,
        inventory: match caps_version {
            GameVersion::V1_8_9 => ghast_inventory::Version::V1_8,
            GameVersion::V1_12_2 => ghast_inventory::Version::V1_12,
            _ => ghast_inventory::Version::V1_16_5,
        },
    }
}

fn run<S: Sim>(version: GameVersion, sim: S, ticks: u64) -> u64 {
    let mut e = Engine::new(opts(version), sim);
    let load = |e: &mut Engine<S>, x: i32, z: i32| {
        let raw = match version {
            GameVersion::V1_8_9 => crate::sim::test_chunks::v1_8(x, z),
            GameVersion::V1_12_2 => crate::sim::test_chunks::v1_12_2(x, z),
            _ => crate::sim::test_chunks::v1_16_5(x, z),
        };
        e.sim.world_packet(&raw).unwrap();
    };
    e.handle(Inbound::JoinGame(JoinGame {
        entity_id: 7,
        game_mode: GameMode::Survival,
        dimension: "0".into(),
        hardcore: false,
        dimension_nbt: None,
        respawn_screen: true,
    }));
    for x in -3..=3 {
        for z in -3..=3 {
            load(&mut e, x, z);
        }
    }
    let home = Teleport {
        x: 8.5,
        y: 64.0,
        z: 8.5,
        yaw: 0.0,
        pitch: 0.0,
        flags: 0,
        teleport_id: Some(1),
        ..Default::default()
    };
    e.handle(Inbound::Teleport(home));
    let chat = ChatMessage {
        rendered: None,
        json: ghast_protocol::types::ByteString::from_static(r#"{"text":"hello"}"#),
        position: ChatPosition::Chat,
        sender: None,
    };
    let t0 = Instant::now();
    let mut b = Walker;
    let mut sent = 0u64;
    for i in 0..ticks {
        let now = t0 + Duration::from_millis(50 * i);
        e.note_inbound(now);
        e.handle(Inbound::KeepAlive(i as i64));
        e.handle(Inbound::Velocity {
            entity_id: 99,
            motion: Vec3::new(0.1, 0.0, 0.1),
        });
        if i.is_multiple_of(5) {
            e.handle(Inbound::Chat(chat.clone()));
        }
        if i.is_multiple_of(20) {
            e.handle(Inbound::Health(Health {
                health: 20.0,
                food: 20,
                saturation: 5.0,
            }));
        }
        if i % 200 == 199 {
            e.handle(Inbound::Teleport(home));
        }
        e.tick(&mut b, now);
        sent += e.outbound().len() as u64;
        e.outbound().clear();
    }
    sent
}

#[test]
#[ignore = "cost probe; run with GHAST_BENCH_TICKS"]
fn engine_bench() {
    let ticks: u64 = std::env::var("GHAST_BENCH_TICKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    let only = std::env::var("GHAST_BENCH_VERSION").ok();
    let want = |v: GameVersion| only.as_deref().is_none_or(|o| o == v.name());
    let mut total = 0;
    if want(GameVersion::V1_8_9) {
        total += run(GameVersion::V1_8_9, crate::sim::Sim18::new(), ticks);
    }
    if want(GameVersion::V1_12_2) {
        total += run(GameVersion::V1_12_2, crate::sim::Sim12::new(), ticks);
    }
    if want(GameVersion::V1_16_5) {
        total += run(GameVersion::V1_16_5, crate::sim::Sim16::new(), ticks);
    }
    println!("engine_bench: {ticks} ticks, {total} packets");
}

#[test]
#[ignore = "prints type sizes"]
fn type_sizes() {
    println!(
        "Outbound {} Inbound {} MovePacket {} Teleport {} ChatMessage {} Pose {}",
        std::mem::size_of::<crate::common::Outbound>(),
        std::mem::size_of::<Inbound>(),
        std::mem::size_of::<crate::common::MovePacket>(),
        std::mem::size_of::<Teleport>(),
        std::mem::size_of::<ChatMessage>(),
        std::mem::size_of::<crate::common::Pose>(),
    );
}

/// Break ticks of every 1.8.9 / 1.16.5 block with the exact (jar) destroy progress vs the
/// minecraft-data formula used before (0.3.0), for a few held items: prints the differences.
#[test]
#[ignore = "prints dig timing changes"]
fn dig_timing_changes() {
    use crate::hands::{DigFactors, relative_hardness};
    use crate::sim::WorldView;
    let ticks = |p: f32| {
        if p <= 0.0 {
            u32::MAX
        } else {
            (1.0 / p).ceil() as u32
        }
    };
    let f = DigFactors::default().with_on_ground(true);
    let mut s18 = crate::sim::Sim18::new();
    s18.world_packet(&crate::sim::test_chunks::v1_8(0, 0))
        .unwrap();
    // empty hand, diamond pickaxe, axe, shovel, sword, shears
    for item in [
        None,
        Some(278u16),
        Some(279),
        Some(277),
        Some(276),
        Some(359),
    ] {
        let held =
            item.map(|id| crate::sim::held_item_facts(crate::version::DataSet::V1_8, id, 1, 0));
        for id in 1..=197u16 {
            let st = ghast_data::v1_8::BlockState::from_raw(id << 4).unwrap();
            s18.test_set_block(1, 62, 1, st);
            let Some(b) = s18.block_query(1, 62, 1) else {
                continue;
            };
            let new = relative_hardness(&b, held.as_ref(), f);
            let mut old_b = b;
            old_b.exact = None;
            let old = relative_hardness(&old_b, held.as_ref(), f);
            if ticks(new) != ticks(old) {
                println!(
                    "1.8.9 item {item:?} block {id} {}: {} -> {} ticks",
                    st.block().info().name,
                    ticks(old),
                    ticks(new)
                );
            }
        }
    }
    let mut s12 = crate::sim::Sim12::new();
    s12.world_packet(&crate::sim::test_chunks::v1_12_2(0, 0))
        .unwrap();
    for item in [
        None,
        Some(278u16),
        Some(279),
        Some(277),
        Some(276),
        Some(359),
    ] {
        let held =
            item.map(|id| crate::sim::held_item_facts(crate::version::DataSet::V1_12, id, 1, 0));
        for id in 1..=255u16 {
            let Some(st) = ghast_data::v1_12::BlockState::from_raw(id << 4) else {
                continue;
            };
            s12.test_set_block(1, 62, 1, st);
            let Some(b) = s12.block_query(1, 62, 1) else {
                continue;
            };
            let new = relative_hardness(&b, held.as_ref(), f);
            let mut old_b = b;
            old_b.exact = None;
            let old = relative_hardness(&old_b, held.as_ref(), f);
            if ticks(new) != ticks(old) {
                println!(
                    "1.12.2 item {item:?} block {id} {}: {} -> {} ticks",
                    st.block().info().name,
                    ticks(old),
                    ticks(new)
                );
            }
        }
    }
    let mut s16 = crate::sim::Sim16::new();
    s16.world_packet(&crate::sim::test_chunks::v1_16_5(0, 0))
        .unwrap();
    let items = [
        "diamond_pickaxe",
        "diamond_axe",
        "diamond_shovel",
        "diamond_sword",
        "shears",
    ];
    let mut held_ids: Vec<Option<u16>> = vec![None];
    held_ids.extend(
        items
            .iter()
            .map(|n| ghast_data::v1_16_5::Item::from_name(n).map(|i| i.id())),
    );
    for item in held_ids {
        let held =
            item.map(|id| crate::sim::held_item_facts(crate::version::DataSet::V1_16_5, id, 1, 0));
        for b in ghast_data::v1_16_5::Block::all() {
            s16.test_set_block(1, 62, 1, b.default_state());
            let Some(q) = s16.block_query(1, 62, 1) else {
                continue;
            };
            let new = relative_hardness(&q, held.as_ref(), f);
            let mut old_q = q;
            old_q.exact = None;
            let old = relative_hardness(&old_q, held.as_ref(), f);
            if ticks(new) != ticks(old) {
                println!(
                    "1.16.5 item {item:?} block {}: {} -> {} ticks",
                    b.info().name,
                    ticks(old),
                    ticks(new)
                );
            }
        }
    }
}

/// Crosshair changes vs 0.3.0 on 1.8.9 / 1.12.2: the oracle pick (now) against the old ray
/// path (`hands::pick` over the block queries) for every block state, 96 rays each from
/// around the block, once alone and once with the same state on its four sides and on top
/// (so fences, panes, walls, stairs and wires connect). Prints, per block, how many rays now
/// hit another block (or none), another face, or the same face with another cursor value as
/// sent on the wire (1.8: whole sixteenths in a byte; 1.12: f32).
#[test]
#[ignore = "prints crosshair changes"]
fn pick_changes() {
    use crate::sim::WorldView;
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut rays = Vec::new();
    for _ in 0..96 {
        let eye = Vec3::new(
            8.5 + rnd() * 6.0 - 3.0,
            59.0 + rnd() * 3.0,
            8.5 + rnd() * 6.0 - 3.0,
        );
        let target = Vec3::new(8.0 + rnd(), 60.0 + rnd(), 8.0 + rnd());
        let (dx, dy, dz) = (target.x - eye.x, target.y - eye.y, target.z - eye.z);
        let yaw = (-dx).atan2(dz).to_degrees() as f32;
        let pitch = -(dy.atan2((dx * dx + dz * dz).sqrt())).to_degrees() as f32;
        rays.push((eye, yaw, pitch));
    }
    // [other block / none, other face, other cursor]
    type Kinds = [usize; 3];
    let compare = |w: &dyn WorldView, legacy_cursor: bool| -> Kinds {
        let blocks = |x: i32, y: i32, z: i32| w.block_query(x, y, z);
        let cursor = |h: &crate::hands::BlockHit| -> [u32; 3] {
            let r = [
                h.hit.x - h.pos.x as f64,
                h.hit.y - h.pos.y as f64,
                h.hit.z - h.pos.z as f64,
            ];
            if legacy_cursor {
                r.map(|v| (v * 16.0) as u8 as u32)
            } else {
                r.map(|v| (v as f32).to_bits())
            }
        };
        let mut k = [0; 3];
        for &(eye, yaw, pitch) in &rays {
            let look = crate::hands::look_vector(yaw, pitch);
            let old = crate::hands::pick(&blocks, eye, look, crate::hands::REACH);
            let new = w.crosshair(eye, (yaw, pitch), look, crate::hands::REACH);
            match (old, new) {
                (None, None) => {}
                (Some(a), Some(b)) if a.pos == b.pos => {
                    if a.face != b.face {
                        k[1] += 1;
                    } else if cursor(&a) != cursor(&b) {
                        k[2] += 1;
                    }
                }
                _ => k[0] += 1,
            }
        }
        k
    };
    let mut changed: std::collections::BTreeMap<String, Kinds> = Default::default();
    let mut add = |key: String, k: Kinds| {
        if k != [0; 3] {
            let e = changed.entry(key).or_default();
            for i in 0..3 {
                e[i] += k[i];
            }
        }
    };
    const AROUND: [(i32, i32, i32); 5] = [(1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, 1, 0)];
    let mut s18 = crate::sim::Sim18::new();
    s18.world_packet(&crate::sim::test_chunks::v1_8(0, 0))
        .unwrap();
    for raw in 1..(198u16 << 4) {
        let Some(st) = ghast_data::v1_8::BlockState::from_raw(raw) else {
            continue;
        };
        let name = st.block().info().name;
        s18.test_set_block(8, 60, 8, st);
        add(format!("1.8.9  {name} (alone)"), compare(&s18, true));
        for (dx, dy, dz) in AROUND {
            s18.test_set_block(8 + dx, 60 + dy, 8 + dz, st);
        }
        add(
            format!("1.8.9  {name} (with neighbours)"),
            compare(&s18, true),
        );
        for (dx, dy, dz) in AROUND {
            s18.test_set_block(8 + dx, 60 + dy, 8 + dz, ghast_data::v1_8::BlockState::AIR);
        }
    }
    let mut s12 = crate::sim::Sim12::new();
    s12.world_packet(&crate::sim::test_chunks::v1_12_2(0, 0))
        .unwrap();
    for raw in 1..(256u16 << 4) {
        let Some(st) = ghast_data::v1_12::BlockState::from_raw(raw) else {
            continue;
        };
        let name = st.block().info().name;
        s12.test_set_block(8, 60, 8, st);
        add(format!("1.12.2 {name} (alone)"), compare(&s12, false));
        for (dx, dy, dz) in AROUND {
            s12.test_set_block(8 + dx, 60 + dy, 8 + dz, st);
        }
        add(
            format!("1.12.2 {name} (with neighbours)"),
            compare(&s12, false),
        );
        for (dx, dy, dz) in AROUND {
            s12.test_set_block(8 + dx, 60 + dy, 8 + dz, ghast_data::v1_12::BlockState::AIR);
        }
    }
    println!("block: rays [other block or none, other face, other cursor] over all its states");
    for (k, v) in &changed {
        println!("{k}: {v:?}");
    }
}
