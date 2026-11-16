//! The client's block predictions and the server's answers to them:
//! 26.x `BlockStatePredictionHandler` (`block_changed_ack`) and 1.16.5
//! `ClientPlayerInteractionManager.processPlayerActionResponse` (`acknowledge_player_digging`).

use super::tests::{Script, engine, join};
use super::*;
use crate::common::{DigStatus, Teleport};
use crate::version::ClientVersion;
use ghast_protocol::{BlockPos, RawPacket};

/// Where the player of `join` stands.
const FEET: Vec3 = Vec3::new(8.5, 64.0, 8.5);

fn modern_versions() -> Vec<GameVersion> {
    let mut v = vec![GameVersion::V26_1];
    #[cfg(feature = "v26_2")]
    v.push(GameVersion::V26_2);
    v
}

/// A block state id by name.
fn state(v: GameVersion, name: &str) -> u16 {
    match v {
        #[cfg(feature = "v26_2")]
        GameVersion::V26_2 => ghast_data::v26_2::BlockState::parse(name).unwrap().id(),
        _ => ghast_data::v26_1::BlockState::parse(name).unwrap().id(),
    }
}

/// `block_changed_ack`.
fn ack(v: GameVersion, sequence: i32) -> RawPacket {
    match v {
        #[cfg(feature = "v26_2")]
        GameVersion::V26_2 => RawPacket::from_packet(
            &ghast_protocol::v26_2::play::clientbound::AcknowledgePlayerDigging {
                sequence_id: sequence,
            },
        ),
        _ => RawPacket::from_packet(
            &ghast_protocol::v26_1::play::clientbound::AcknowledgePlayerDigging {
                sequence_id: sequence,
            },
        ),
    }
    .unwrap()
}

/// `block_update`.
fn block_update(v: GameVersion, pos: BlockPos, s: u16) -> RawPacket {
    match v {
        #[cfg(feature = "v26_2")]
        GameVersion::V26_2 => {
            RawPacket::from_packet(&ghast_protocol::v26_2::play::clientbound::BlockChange {
                location: pos,
                r#type: i32::from(s),
            })
        }
        _ => RawPacket::from_packet(&ghast_protocol::v26_1::play::clientbound::BlockChange {
            location: pos,
            r#type: i32::from(s),
        }),
    }
    .unwrap()
}

/// `section_blocks_update` of the section holding the first position.
fn section_update(v: GameVersion, changes: &[(BlockPos, u16)]) -> RawPacket {
    let p0 = changes[0].0;
    let records: Vec<i64> = changes
        .iter()
        .map(|&(p, s)| {
            assert_eq!(
                (p.x >> 4, p.y >> 4, p.z >> 4),
                (p0.x >> 4, p0.y >> 4, p0.z >> 4)
            );
            i64::from(s) << 12 | i64::from((p.x & 15) << 8 | (p.z & 15) << 4 | (p.y & 15))
        })
        .collect();
    match v {
        #[cfg(feature = "v26_2")]
        GameVersion::V26_2 => {
            use ghast_protocol::v26_2::play::clientbound as cb;
            RawPacket::from_packet(&cb::MultiBlockChange {
                chunk_coordinates: cb::MultiBlockChangeChunkCoordinates {
                    x: p0.x >> 4,
                    z: p0.z >> 4,
                    y: p0.y >> 4,
                },
                records,
            })
        }
        _ => {
            use ghast_protocol::v26_1::play::clientbound as cb;
            RawPacket::from_packet(&cb::MultiBlockChange {
                chunk_coordinates: cb::MultiBlockChangeChunkCoordinates {
                    x: p0.x >> 4,
                    z: p0.z >> 4,
                    y: p0.y >> 4,
                },
                records,
            })
        }
    }
    .unwrap()
}

fn block(e: &Engine<Box<dyn Sim>>, p: BlockPos) -> Option<u32> {
    e.sim.raw_block(p.x, p.y, p.z)
}

/// The client broke `p` in prediction `sequence` (as the engine reports a finish).
fn predict(e: &mut Engine<Box<dyn Sim>>, p: BlockPos, sequence: i32) {
    e.sim.dig_sent(p, DigStatus::Finish, true, sequence, FEET);
}

/// A joined modern engine with the stone layer at y = 63.
fn modern(v: GameVersion) -> Engine<Box<dyn Sim>> {
    let mut e = engine(v);
    join(&mut e);
    e
}

const A: BlockPos = BlockPos::new(3, 63, 3);
const B: BlockPos = BlockPos::new(5, 63, 6);
const C: BlockPos = BlockPos::new(12, 63, 1);

/// The server refuses the break and sends no block update: the ack alone restores it.
#[test]
fn ack_without_update_restores_the_block() {
    for v in modern_versions() {
        let mut e = modern(v);
        let stone = u32::from(state(v, "stone"));
        assert_eq!(block(&e, A), Some(stone));
        predict(&mut e, A, 1);
        assert_eq!(block(&e, A), Some(0), "{v:?}");
        e.sim.world_packet(&ack(v, 1)).unwrap();
        assert_eq!(block(&e, A), Some(stone), "{v:?}");
        // Ended: a later update applies at once.
        e.sim.world_packet(&block_update(v, A, 0)).unwrap();
        assert_eq!(block(&e, A), Some(0), "{v:?}");
    }
}

/// A block update for a predicted block only replaces the remembered server state; at the
/// ack that state wins (whatever the client predicted).
#[test]
fn update_during_prediction_waits_for_the_ack() {
    for v in modern_versions() {
        let mut e = modern(v);
        let dirt = state(v, "dirt");
        predict(&mut e, A, 1);
        e.sim.world_packet(&block_update(v, A, dirt)).unwrap();
        assert_eq!(block(&e, A), Some(0), "deferred: {v:?}");
        e.sim.world_packet(&ack(v, 1)).unwrap();
        assert_eq!(block(&e, A), Some(u32::from(dirt)), "{v:?}");

        // The approved break: the server's air arrives before the ack, nothing changes.
        predict(&mut e, B, 2);
        e.sim.world_packet(&block_update(v, B, 0)).unwrap();
        e.sim.world_packet(&ack(v, 2)).unwrap();
        assert_eq!(block(&e, B), Some(0), "{v:?}");
    }
}

/// `endPredictionsUpTo`: an ack ends the predictions up to its sequence only.
#[test]
fn lower_ack_keeps_later_predictions() {
    for v in modern_versions() {
        let mut e = modern(v);
        let stone = u32::from(state(v, "stone"));
        predict(&mut e, A, 1);
        predict(&mut e, B, 2);
        e.sim.world_packet(&ack(v, 1)).unwrap();
        assert_eq!(block(&e, A), Some(stone), "{v:?}");
        assert_eq!(block(&e, B), Some(0), "{v:?}");
        // B still waits: its update is deferred.
        let dirt = state(v, "dirt");
        e.sim.world_packet(&block_update(v, B, dirt)).unwrap();
        assert_eq!(block(&e, B), Some(0), "{v:?}");
        e.sim.world_packet(&ack(v, 2)).unwrap();
        assert_eq!(block(&e, B), Some(u32::from(dirt)), "{v:?}");
        // Breaking air again changes nothing (destroyBlock: nothing for air), so the record
        // keeps its sequence.
        predict(&mut e, C, 3);
        e.sim.dig_sent(C, DigStatus::Finish, true, 4, FEET);
        e.sim.world_packet(&ack(v, 3)).unwrap();
        assert_eq!(block(&e, C), Some(stone), "{v:?}");
    }
}

/// Updates of positions without a prediction apply at once while predictions wait; a
/// section update splits the same way.
#[test]
fn other_positions_and_section_updates() {
    for v in modern_versions() {
        let mut e = modern(v);
        let dirt = state(v, "dirt");
        let glass = state(v, "glass");
        predict(&mut e, A, 1);
        e.sim.world_packet(&block_update(v, C, dirt)).unwrap();
        assert_eq!(block(&e, C), Some(u32::from(dirt)), "{v:?}");
        // A and B share section (0, 3, 0).
        e.sim.collect_block_updates(true);
        e.sim
            .world_packet(&section_update(v, &[(A, glass), (B, glass)]))
            .unwrap();
        assert_eq!(block(&e, A), Some(0), "deferred: {v:?}");
        assert_eq!(block(&e, B), Some(u32::from(glass)), "{v:?}");
        // The behaviour hears every change the server sent.
        let mut seen = Vec::new();
        e.sim.drain_block_updates(&mut seen);
        assert_eq!(
            seen,
            [(A, u32::from(glass)), (B, u32::from(glass))],
            "{v:?}"
        );
        e.sim.world_packet(&ack(v, 1)).unwrap();
        assert_eq!(block(&e, A), Some(u32::from(glass)), "{v:?}");
    }
}

/// `destroyBlock` leaves the block's fluid: a broken waterlogged block (and seagrass) becomes a
/// water source, a dry one air; a refused break restores the waterlogged state.
#[test]
fn broken_waterlogged_block_leaves_water() {
    for v in modern_versions() {
        let mut e = modern(v);
        let water = u32::from(state(v, "water"));
        for name in [
            "oak_stairs[waterlogged=true]",
            "oak_slab[type=bottom,waterlogged=true]",
            "seagrass",
        ] {
            let s = state(v, name);
            e.sim.world_packet(&block_update(v, A, s)).unwrap();
            predict(&mut e, A, 1);
            assert_eq!(block(&e, A), Some(water), "{v:?} {name}");
            e.sim.world_packet(&ack(v, 1)).unwrap();
            assert_eq!(block(&e, A), Some(u32::from(s)), "{v:?} {name}");
        }
        let dry = state(v, "oak_stairs[waterlogged=false]");
        e.sim.world_packet(&block_update(v, A, dry)).unwrap();
        predict(&mut e, A, 2);
        assert_eq!(block(&e, A), Some(0), "{v:?}");
    }
}

/// `isColliding` uses the player's collision context: scaffolding the player stands inside
/// (not above its top) has no collision shape, so restoring it does not move the player; a
/// full block there does.
#[test]
fn move_back_uses_the_players_collision_context() {
    for v in modern_versions() {
        let mut e = modern(v);
        let at_feet = BlockPos::new(8, 64, 8);
        let back = Vec3::new(8.5, 66.0, 8.5);
        let scaffolding = state(v, "scaffolding[distance=0,bottom=false]");
        e.sim
            .world_packet(&block_update(v, at_feet, scaffolding))
            .unwrap();
        e.sim.dig_sent(at_feet, DigStatus::Finish, true, 1, back);
        e.sim.world_packet(&ack(v, 1)).unwrap();
        e.sync_snap();
        assert_eq!(block(&e, at_feet), Some(u32::from(scaffolding)), "{v:?}");
        assert_eq!(e.core.pose.pos, FEET, "inside scaffolding: {v:?}");
        let stone = state(v, "stone");
        e.sim
            .world_packet(&block_update(v, at_feet, stone))
            .unwrap();
        e.sim.dig_sent(at_feet, DigStatus::Finish, true, 2, back);
        e.sim.world_packet(&ack(v, 2)).unwrap();
        e.sync_snap();
        assert_eq!(e.core.pose.pos, back, "inside stone: {v:?}");
    }
}

/// The wire's numbers for the packets of one tick, as the connection writes them.
fn write_tick<V: ClientVersion>(
    out: &[Outbound],
    wire: &mut V::Wire,
    finishes: &mut Vec<i32>,
) -> Option<i32> {
    let mut enc = ghast_protocol::FrameEncoder::new();
    V::prepare_play(out, wire);
    for o in out {
        V::write_play(&mut enc, o, wire).unwrap();
        if matches!(
            o,
            Outbound::Dig {
                status: DigStatus::Finish,
                ..
            }
        ) {
            finishes.push(V::prediction_sequence(wire).unwrap());
        }
    }
    V::prediction_sequence(wire)
}

/// The left button breaks the block under the feet; the engine numbers the prediction like
/// the wire numbers the finish. The server refuses: its ack restores the block while the
/// player falls into the hole, and the player returns to where it stood (`syncBlockState`).
fn dig_refused<V: ClientVersion>(v: GameVersion)
where
    V::Wire: Default,
{
    let mut e = modern(v);
    let stone = u32::from(state(v, "stone"));
    let under = BlockPos::new(8, 63, 8);
    let t0 = Instant::now();
    e.note_inbound(t0);
    let mut wire = V::Wire::default();
    // Some predictions before (the numbers do not start at 0).
    for _ in 0..3 {
        let _ = V::write_play(
            &mut ghast_protocol::FrameEncoder::new(),
            &Outbound::UseItem {
                off_hand: false,
                held: None,
            },
            &mut wire,
        );
    }
    e.note_sequence(V::prediction_sequence(&wire));
    let mut b = Script(|ctx: &mut TickContext<'_>| {
        ctx.set_aim(90.0, 89.5);
        let down = ctx.target().is_some_and(|t| t.pos == under);
        ctx.set_buttons(Buttons {
            left: down,
            right: false,
        });
    });
    let mut finishes = Vec::new();
    let mut ticks = 0;
    while finishes.is_empty() {
        e.tick(&mut b, t0);
        let seq = write_tick::<V>(e.outbound(), &mut wire, &mut finishes);
        e.outbound().clear();
        e.note_sequence(seq);
        ticks += 1;
        assert!(ticks < 400, "no finish: {v:?}");
    }
    assert_eq!(block(&e, under), Some(0), "{v:?}");
    // Falling into the hole.
    for _ in 0..3 {
        e.tick(&mut b, t0);
        let seq = write_tick::<V>(e.outbound(), &mut wire, &mut finishes);
        e.outbound().clear();
        e.note_sequence(seq);
    }
    assert!(e.core.pose.pos.y < 64.0, "{v:?} {:?}", e.core.pose.pos);
    let finish = finishes[0];
    // An ack below the finish's number leaves it (the start's prediction broke nothing).
    e.sim.world_packet(&ack(v, finish - 1)).unwrap();
    e.sync_snap();
    assert_eq!(block(&e, under), Some(0), "{v:?}");
    e.sim.world_packet(&ack(v, finish)).unwrap();
    e.sync_snap();
    assert_eq!(block(&e, under), Some(stone), "{v:?}");
    assert_eq!(e.core.pose.pos, FEET, "moved back: {v:?}");
    // The physics continues from there: standing on the stone again.
    e.tick(&mut b, t0);
    assert!(e.core.pose.pos.y >= 64.0, "{v:?} {:?}", e.core.pose.pos);
}

#[test]
fn refused_dig_restores_and_moves_back_26_1() {
    dig_refused::<crate::version::V26_1>(GameVersion::V26_1);
}

#[cfg(feature = "v26_2")]
#[test]
fn refused_dig_restores_and_moves_back_26_2() {
    dig_refused::<crate::version::V26_2>(GameVersion::V26_2);
}

/// A server teleport after the prediction: the ack restores the block but does not move the
/// player (`lastTeleportSequence < sequence` fails); a block that does not hold the player never
/// moves it.
#[test]
fn teleport_after_prediction_keeps_the_position() {
    for v in modern_versions() {
        let mut e = modern(v);
        let under = BlockPos::new(8, 63, 8);
        e.note_sequence(Some(5));
        predict(&mut e, under, 5);
        let inside = Teleport {
            x: 8.5,
            y: 63.5,
            z: 8.5,
            yaw: 90.0,
            pitch: 0.0,
            flags: 0,
            teleport_id: Some(3),
            ..Default::default()
        };
        e.handle(Inbound::Teleport(inside));
        e.sim.world_packet(&ack(v, 5)).unwrap();
        e.sync_snap();
        assert_ne!(block(&e, under), Some(0), "{v:?}");
        assert_eq!(e.core.pose.pos.y, 63.5, "{v:?}");

        // A later prediction (sequence 6 > the teleport's 5) moves the player back.
        predict(&mut e, C, 6);
        e.sim
            .dig_sent(under, DigStatus::Finish, true, 6, Vec3::new(8.5, 70.0, 8.5));
        e.sim.world_packet(&ack(v, 6)).unwrap();
        e.sync_snap();
        assert_eq!(e.core.pose.pos.y, 70.0, "{v:?}");
        // C did not hold the player: no move for it (none pending any more).
        assert!(e.sim.take_snap().is_none());
    }
}

/// 1.16.5 `acknowledge_player_digging`.
mod v1_16_5 {
    use super::*;
    use ghast_protocol::v1_16_5::play::clientbound as cb;

    const STONE: u16 = 1;
    const START: i32 = 0;
    const FINISH: i32 = 2;

    fn answer(pos: BlockPos, block: u16, status: i32, successful: bool) -> RawPacket {
        RawPacket::from_packet(&cb::AcknowledgePlayerDigging {
            location: pos,
            block: i32::from(block),
            status,
            successful,
        })
        .unwrap()
    }

    fn set(e: &mut Engine<Box<dyn Sim>>, pos: BlockPos, s: u16) {
        e.sim
            .world_packet(
                &RawPacket::from_packet(&cb::BlockChange {
                    location: pos,
                    r#type: i32::from(s),
                })
                .unwrap(),
            )
            .unwrap();
    }

    fn joined() -> Engine<Box<dyn Sim>> {
        let mut e = engine(GameVersion::V1_16_5);
        join(&mut e);
        for p in [A, B, C] {
            set(&mut e, p, STONE);
        }
        e
    }

    /// Refused: the server's state comes back; approved with the same state: nothing to do.
    #[test]
    fn refused_finish_restores() {
        let mut e = joined();
        e.sim.dig_sent(A, DigStatus::Finish, true, 0, FEET);
        assert_eq!(block(&e, A), Some(0));
        e.sim
            .world_packet(&answer(A, STONE, FINISH, false))
            .unwrap();
        assert_eq!(block(&e, A), Some(u32::from(STONE)));

        e.sim.dig_sent(B, DigStatus::Finish, true, 0, FEET);
        e.sim.world_packet(&answer(B, 0, FINISH, true)).unwrap();
        assert_eq!(block(&e, B), Some(0));
        e.sync_snap();
        assert_eq!(e.core.pose.pos, FEET);
    }

    /// `breakBlock` leaves the block's fluid (a waterlogged block: a water source).
    #[test]
    fn broken_waterlogged_block_leaves_water() {
        use ghast_data::v1_16_5::{Block, Property};
        let mut e = joined();
        let stairs = |w: &str| {
            Block::from_name("oak_stairs")
                .unwrap()
                .state_with(&[(Property::from_name("waterlogged").unwrap(), w)])
                .unwrap()
                .id()
        };
        let id = |n: &str| Block::from_name(n).unwrap().default_state().id();
        let wet = stairs("true");
        set(&mut e, A, wet);
        e.sim.dig_sent(A, DigStatus::Finish, true, 0, FEET);
        assert_eq!(block(&e, A), Some(u32::from(id("water"))));
        e.sim.world_packet(&answer(A, wet, FINISH, false)).unwrap();
        assert_eq!(block(&e, A), Some(u32::from(wet)));
        set(&mut e, B, stairs("false"));
        e.sim.dig_sent(B, DigStatus::Finish, true, 0, FEET);
        assert_eq!(block(&e, B), Some(0));
    }

    /// An approved start the client recorded keeps the client's state (an instant break stays
    /// air although the answer carries the state before); without a record, or refused, the
    /// server's state is set.
    #[test]
    fn approved_start_keeps_the_client_state() {
        let mut e = joined();
        e.sim.dig_sent(A, DigStatus::Start, true, 0, FEET);
        e.sim.world_packet(&answer(A, STONE, START, true)).unwrap();
        assert_eq!(block(&e, A), Some(0));
        // The record is gone: the same answer again sets the state.
        e.sim.world_packet(&answer(A, STONE, START, true)).unwrap();
        assert_eq!(block(&e, A), Some(u32::from(STONE)));

        e.sim.dig_sent(B, DigStatus::Start, true, 0, FEET);
        e.sim.world_packet(&answer(B, STONE, START, false)).unwrap();
        assert_eq!(block(&e, B), Some(u32::from(STONE)));

        // Keyed by action: a finish answer does not match a recorded start.
        e.sim.dig_sent(C, DigStatus::Start, true, 0, FEET);
        e.sim.world_packet(&answer(C, STONE, FINISH, true)).unwrap();
        assert_eq!(block(&e, C), Some(u32::from(STONE)));
    }

    /// A restored block that holds the player moves it to where it was when the action was
    /// sent; a block beside it does not.
    #[test]
    fn restored_block_moves_the_player_back() {
        let mut e = joined();
        let at_feet = BlockPos::new(8, 64, 8);
        let back = Vec3::new(8.5, 66.0, 8.5);
        e.sim.dig_sent(A, DigStatus::Finish, true, 0, back);
        e.sim
            .world_packet(&answer(A, STONE, FINISH, false))
            .unwrap();
        e.sync_snap();
        assert_eq!(e.core.pose.pos, FEET);
        e.sim.dig_sent(at_feet, DigStatus::Abort, false, 0, back);
        e.sim
            .world_packet(&answer(at_feet, STONE, 1, true))
            .unwrap();
        e.sync_snap();
        assert_eq!(block(&e, at_feet), Some(u32::from(STONE)));
        assert_eq!(e.core.pose.pos, back);
    }

    /// The left button records its start and finish with the position before the tick's
    /// movement; the refused finish brings the block back under the falling player and
    /// moves it back up.
    #[test]
    fn refused_dig_with_the_left_button() {
        let mut e = joined();
        let under = BlockPos::new(8, 63, 8);
        set(&mut e, under, STONE);
        let t0 = Instant::now();
        e.note_inbound(t0);
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            ctx.set_aim(90.0, 89.5);
            let down = ctx.target().is_some_and(|t| t.pos == under);
            ctx.set_buttons(Buttons {
                left: down,
                right: false,
            });
        });
        let mut ticks = 0;
        while block(&e, under) != Some(0) {
            e.tick(&mut b, t0);
            e.outbound().clear();
            ticks += 1;
            assert!(ticks < 400, "no finish");
        }
        for _ in 0..3 {
            e.tick(&mut b, t0);
            e.outbound().clear();
        }
        assert!(e.core.pose.pos.y < 64.0, "{:?}", e.core.pose.pos);
        // The start's approval changes nothing (the client's air stays).
        e.sim
            .world_packet(&answer(under, STONE, START, true))
            .unwrap();
        e.sync_snap();
        assert_eq!(block(&e, under), Some(0));
        e.sim
            .world_packet(&answer(under, STONE, FINISH, false))
            .unwrap();
        e.sync_snap();
        assert_eq!(block(&e, under), Some(u32::from(STONE)));
        assert_eq!(e.core.pose.pos, FEET);
    }
}
