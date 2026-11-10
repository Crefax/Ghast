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
