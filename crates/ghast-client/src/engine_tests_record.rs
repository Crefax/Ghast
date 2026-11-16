//! `Client::record` on every version: a short session through the engine is written in the
//! oracle's replay format (`oracle/run.sh replay --in <file>` runs it in vanilla).
//!
//! `GHAST_KEEP_RECORDINGS=<dir>` keeps the files (`<dir>/<version>.jsonl`) for an oracle replay.

use super::tests::{Script, join, opts};
use super::*;
use crate::sim::SimOptions;

fn sim_with(v: GameVersion, o: &SimOptions) -> Box<dyn Sim> {
    match v.instance() {
        GameVersion::V1_8_9 => Box::new(crate::sim::Sim18::with(o)),
        GameVersion::V1_12_2 => Box::new(crate::sim::Sim12::with(o)),
        GameVersion::V26_1 => Box::new(crate::sim::Sim26::test_overworld_with(o)),
        #[cfg(feature = "v26_2")]
        GameVersion::V26_2 => Box::new(crate::sim::Sim262::test_overworld_with(o)),
        _ => Box::new(crate::sim::Sim16::with(o)),
    }
}

/// The row name the recording carries (the oracle harness it replays on).
fn row(v: GameVersion) -> &'static str {
    match v.instance() {
        GameVersion::V1_8_9 => "1.8.9",
        GameVersion::V1_12_2 => "1.12.2",
        GameVersion::V1_16_5 => "1.16.5",
        GameVersion::V26_2 => "26.2",
        _ => "26.1",
    }
}

#[test]
fn every_version_records_a_session() {
    let keep = std::env::var_os("GHAST_KEEP_RECORDINGS").map(std::path::PathBuf::from);
    let tmp = std::env::temp_dir().join(format!("ghast-record-test-{}", std::process::id()));
    for &v in GameVersion::ALL {
        let path = keep
            .clone()
            .unwrap_or_else(|| tmp.clone())
            .join(format!("{v}.jsonl"));
        let sim = sim_with(
            v,
            &SimOptions {
                record: Some(path.clone()),
                ..Default::default()
            },
        );
        let mut e = Engine::new(opts(v), sim);
        join(&mut e);
        if v.instance() == GameVersion::V1_16_5 {
            // The 1.16 test chunk is all air: a stone floor at y 63.
            use ghast_protocol::v1_16_5::play::clientbound as cb;
            for x in -16..32 {
                for z in -16..32 {
                    let raw = ghast_protocol::RawPacket::from_packet(&cb::BlockChange {
                        location: ghast_protocol::BlockPos::new(x, 63, z),
                        r#type: 1,
                    })
                    .unwrap();
                    e.sim.world_packet(&raw).unwrap();
                }
            }
        }
        let t0 = Instant::now();
        let mut n = 0u32;
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            n += 1;
            ctx.set_aim(90.0 + n as f32, 10.0);
            ctx.set_controls(Controls {
                forward: n > 3,
                jump: n % 9 == 5,
                sprint: n > 20,
                ..Controls::default()
            });
        });
        for i in 0..40 {
            // Inbound keeps flowing (the silent-server guard would freeze the behaviour).
            let now = t0 + Duration::from_millis(50 * i);
            e.note_inbound(now);
            // Server events the recording carries: an effect, a teleport, a velocity, the speed
            // attribute.
            match i {
                8 => e.handle(Inbound::Effect {
                    entity_id: 7,
                    id: 8,
                    amplifier: 1,
                }),
                15 => e.handle(Inbound::Teleport(crate::common::Teleport {
                    x: 9.5,
                    y: 64.0,
                    z: 8.5,
                    yaw: 120.0,
                    pitch: 5.0,
                    flags: 0,
                    teleport_id: Some(5),
                    ..Default::default()
                })),
                22 => {
                    let mut m = [0.1, 0.3, -0.05];
                    if v.caps().configuration_state {
                        // 26.x sends the motion as an LpVec3: only its values reach the client.
                        m = ghast_protocol::types::LpVec3::from_vec3(m).to_vec3();
                    }
                    e.handle(Inbound::Velocity {
                        entity_id: 7,
                        motion: Vec3::new(m[0], m[1], m[2]),
                    });
                }
                26 if v.caps().item_model == crate::version::ItemModel::Components => {
                    e.handle(Inbound::Attributes {
                        entity_id: 7,
                        attributes: vec![crate::common::AttributeUpdate {
                            name: "movement_speed",
                            base: 0.1,
                            modifiers: vec![("minecraft:test_boost".into(), 0.5, 2)],
                        }],
                    });
                }
                26 => e.handle(Inbound::MovementSpeed {
                    entity_id: 7,
                    base: 0.1,
                    modifiers: vec![(
                        uuid::Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0),
                        0.5,
                        2,
                    )],
                }),
                _ => {}
            }
            e.tick(&mut b, now);
        }
        let _ = b;
        let pose_x = e.core.pose.pos.x;
        // The recorder writes the file when the session (here: the engine) ends.
        drop(e);

        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{v}: {e}"));
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{v}: {e}: {l}")))
            .collect();
        let head = &lines[0];
        assert_eq!(head["type"], "scenario", "{v}");
        assert_eq!(head["version"], row(v), "{v}");
        assert!(
            !head["fills"].as_array().unwrap().is_empty(),
            "{v}: no blocks"
        );
        let setup = &head["setup"];
        assert_eq!(setup["pos"][0][0].as_f64(), Some(8.5), "{v}: {setup}");
        assert_eq!(setup["pos"][1][0].as_f64(), Some(64.0), "{v}: {setup}");
        let ticks: Vec<&serde_json::Value> = lines.iter().filter(|l| l["type"] == "tick").collect();
        assert!(ticks.len() >= 30, "{v}: {} ticks", ticks.len());
        assert_eq!(head["ticks"].as_u64(), Some(ticks.len() as u64), "{v}");
        for (i, t) in ticks.iter().enumerate() {
            assert_eq!(t["t"].as_u64(), Some(i as u64 + 1), "{v}");
        }
        let last = ticks.last().unwrap();
        assert!(
            last["input"]["keys"].as_str().unwrap().contains("forward"),
            "{v}"
        );
        // The player walked: the recorded end is not the start (the replay compares the rest).
        let end_x = last["state"]["pos"][0][0].as_f64().unwrap();
        assert!((end_x - 8.5).abs() > 1.0, "{v}: x {end_x}");
        let _ = pose_x;
        if matches!(v.instance(), GameVersion::V26_1 | GameVersion::V26_2) {
            // State ids are named for the replay's remapping.
            let states = head["states"].as_object().unwrap();
            assert!(
                states.values().any(|s| s == "minecraft:stone"),
                "{v}: {states:?}"
            );
        }
        let events: Vec<&str> = ticks
            .iter()
            .filter_map(|t| t["input"]["events"].as_array())
            .flatten()
            .filter_map(|e| e["type"].as_str())
            .collect();
        for want in ["effects", "teleport"] {
            assert!(events.contains(&want), "{v}: {events:?}");
        }
        assert!(
            events.contains(&"velocity") || events.contains(&"motion"),
            "{v}: {events:?}"
        );
        assert!(
            events.contains(&"speed_attribute") || events.contains(&"attribute"),
            "{v}: {events:?}"
        );
        assert!(!path.with_extension("part").exists(), "{v}");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
