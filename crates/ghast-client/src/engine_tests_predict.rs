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

    /// The left button on a waterlogged block: the engine's dig path leaves water too.
    #[test]
    fn left_button_on_a_waterlogged_block_leaves_water() {
        use ghast_data::v1_16_5::{Block, Property};
        let mut e = joined();
        let wet = Block::from_name("oak_stairs")
            .unwrap()
            .state_with(&[(Property::from_name("waterlogged").unwrap(), "true")])
            .unwrap()
            .id();
        let water = Block::from_name("water").unwrap().default_state().id();
        let under = BlockPos::new(8, 63, 8);
        set(&mut e, under, wet);
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
        while block(&e, under) == Some(u32::from(wet)) {
            e.tick(&mut b, t0);
            e.outbound().clear();
            ticks += 1;
            assert!(ticks < 400, "no finish");
        }
        assert_eq!(block(&e, under), Some(u32::from(water)));
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

    /// The move-back uses the player's collision context: scaffolding the player stands inside
    /// has no shape for it (no move), stone does.
    #[test]
    fn move_back_uses_the_players_collision_context() {
        use ghast_data::v1_16_5::{Block, Property};
        let mut e = joined();
        let at_feet = BlockPos::new(8, 64, 8);
        let back = Vec3::new(8.5, 66.0, 8.5);
        let scaffolding = Block::from_name("scaffolding")
            .unwrap()
            .state_with(&[
                (Property::from_name("distance").unwrap(), "0"),
                (Property::from_name("bottom").unwrap(), "false"),
            ])
            .unwrap()
            .id();
        e.sim.dig_sent(at_feet, DigStatus::Finish, true, 0, back);
        e.sim
            .world_packet(&answer(at_feet, scaffolding, FINISH, false))
            .unwrap();
        e.sync_snap();
        assert_eq!(block(&e, at_feet), Some(u32::from(scaffolding)));
        assert_eq!(e.core.pose.pos, FEET, "inside scaffolding");
        set(&mut e, at_feet, 0);
        e.sim.dig_sent(at_feet, DigStatus::Finish, true, 0, back);
        e.sim
            .world_packet(&answer(at_feet, STONE, FINISH, false))
            .unwrap();
        e.sync_snap();
        assert_eq!(e.core.pose.pos, back, "inside stone");
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

/// Holds 16 of `item` in hotbar slot 0, right-clicks the top of the floor block two blocks in
/// front with [`TickContext::place`] and returns the engine with the placement written.
fn place_on_floor(v: GameVersion, item_id: i32) -> (Engine<Box<dyn Sim>>, BlockPos) {
    let mut e = engine(v);
    join(&mut e);
    let support = BlockPos::new(8, 63, 10);
    if v == GameVersion::V1_16_5 {
        // The 1.16.5 test chunks are empty: a floor under the player and the support.
        for z in 7..=10 {
            legacy_set(&mut e, v, BlockPos::new(8, 63, z), 1);
        }
    }
    e.inventory_mut()
        .on_set_slot(0, 36, Some(ghast_inventory::Stack::new(item_id, 16, 0)));
    let t0 = Instant::now();
    e.note_inbound(t0);
    let hit = Vec3::new(8.5, 64.0, 10.5);
    let mut placed = false;
    for _ in 0..40 {
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            ctx.set_aim(0.0, 39.0);
            if ctx.place(support, crate::common::Face::Up, hit) {
                ctx.set_aim(0.0, 39.0);
            }
        });
        e.tick(&mut b, t0);
        placed = e
            .outbound()
            .iter()
            .any(|o| matches!(o, Outbound::Place { pos, .. } if *pos == support));
        e.outbound().clear();
        if placed {
            break;
        }
    }
    assert!(placed, "no placement: {v:?}");
    (e, BlockPos::new(8, 64, 10))
}

/// A placement the server refuses with its ack alone: the block shows at once and goes when
/// the ack comes; an accepted one (the server's update, then the ack) never flickers.
#[test]
fn placed_block_is_predicted_and_settled_by_the_ack() {
    for v in modern_versions() {
        let stone_item = match v {
            #[cfg(feature = "v26_2")]
            GameVersion::V26_2 => {
                i32::from(ghast_data::v26_2::Item::from_name("stone").unwrap().id())
            }
            _ => i32::from(ghast_data::v26_1::Item::from_name("stone").unwrap().id()),
        };
        let stone = state(v, "stone");
        let (mut e, at) = place_on_floor(v, stone_item);
        assert_eq!(block(&e, at), Some(u32::from(stone)), "predicted: {v:?}");
        // Refused: the ack alone restores the air.
        e.sim.world_packet(&ack(v, 1)).unwrap();
        assert_eq!(block(&e, at), Some(0), "{v:?}");

        let (mut e, at) = place_on_floor(v, stone_item);
        e.sim.world_packet(&block_update(v, at, stone)).unwrap();
        assert_eq!(block(&e, at), Some(u32::from(stone)), "{v:?}");
        e.sim.world_packet(&ack(v, 1)).unwrap();
        assert_eq!(block(&e, at), Some(u32::from(stone)), "{v:?}");
        // Not a block item: nothing predicted.
        let stick = match v {
            #[cfg(feature = "v26_2")]
            GameVersion::V26_2 => {
                i32::from(ghast_data::v26_2::Item::from_name("stick").unwrap().id())
            }
            _ => i32::from(ghast_data::v26_1::Item::from_name("stick").unwrap().id()),
        };
        let (e, at) = place_on_floor(v, stick);
        assert_eq!(block(&e, at), Some(0), "{v:?}");
        // Oak logs place with their axis: left to the server.
        let log = match v {
            #[cfg(feature = "v26_2")]
            GameVersion::V26_2 => {
                i32::from(ghast_data::v26_2::Item::from_name("oak_log").unwrap().id())
            }
            _ => i32::from(ghast_data::v26_1::Item::from_name("oak_log").unwrap().id()),
        };
        let (e, at) = place_on_floor(v, log);
        assert_eq!(block(&e, at), Some(0), "{v:?}");
    }
}

/// A server block change on 1.8.9 – 1.16.5 (raw `id << 4 | meta` there, a state id on 1.16.5).
fn legacy_set(e: &mut Engine<Box<dyn Sim>>, v: GameVersion, pos: BlockPos, s: u32) {
    let raw = match v {
        GameVersion::V1_8_9 => {
            RawPacket::from_packet(&ghast_protocol::v1_8::play::clientbound::BlockChange {
                location: pos,
                r#type: s as i32,
            })
        }
        GameVersion::V1_12_2 => {
            RawPacket::from_packet(&ghast_protocol::v1_12_2::play::clientbound::BlockChange {
                location: pos,
                r#type: s as i32,
            })
        }
        _ => RawPacket::from_packet(&ghast_protocol::v1_16_5::play::clientbound::BlockChange {
            location: pos,
            r#type: s as i32,
        }),
    };
    e.sim.world_packet(&raw.unwrap()).unwrap();
}

/// 1.8.9 – 1.16.5: the placed block shows at once; the server's block change for it settles
/// it (refused: back to air; accepted: the same block, no flicker). Logs (axis) and wool
/// (variants) are not predicted.
#[test]
fn placed_block_is_predicted_on_legacy_versions() {
    let cases: [(GameVersion, i32, u32, &[i32]); 3] = [
        // cobblestone item 4 → raw 4 << 4; oak log 17, wool 35: not predicted.
        (GameVersion::V1_8_9, 4, 4 << 4, &[17, 35, 280]),
        (GameVersion::V1_12_2, 4, 4 << 4, &[17, 35, 280]),
        (
            GameVersion::V1_16_5,
            i32::from(
                ghast_data::v1_16_5::Item::from_name("cobblestone")
                    .unwrap()
                    .id(),
            ),
            u32::from(
                ghast_data::v1_16_5::Block::from_name("cobblestone")
                    .unwrap()
                    .default_state()
                    .id(),
            ),
            &[
                i32::from(
                    ghast_data::v1_16_5::Item::from_name("oak_log")
                        .unwrap()
                        .id(),
                ),
                i32::from(ghast_data::v1_16_5::Item::from_name("stick").unwrap().id()),
            ],
        ),
    ];
    for (v, item, placed, others) in cases {
        let (mut e, at) = place_on_floor(v, item);
        assert_eq!(block(&e, at), Some(placed), "predicted: {v:?}");
        legacy_set(&mut e, v, at, 0);
        assert_eq!(block(&e, at), Some(0), "refused: {v:?}");
        let (mut e, at) = place_on_floor(v, item);
        legacy_set(&mut e, v, at, placed);
        assert_eq!(block(&e, at), Some(placed), "accepted: {v:?}");
        for &o in others {
            let (e, at) = place_on_floor(v, o);
            assert_eq!(block(&e, at), Some(0), "{v:?} item {o}");
        }
    }
}

/// Switching to spectator stops an item use only on 26.x (the server's `stopUsingItem`);
/// 1.8.9 – 1.16.5 keep it until release or finish.
#[test]
fn spectator_switch_stops_a_use_only_on_26_x() {
    let mut versions = vec![
        (GameVersion::V1_8_9, false),
        (GameVersion::V1_12_2, false),
        (GameVersion::V1_16_5, false),
        (GameVersion::V26_1, true),
    ];
    #[cfg(feature = "v26_2")]
    versions.push((GameVersion::V26_2, true));
    for (v, stops) in versions {
        let mut e = engine(v);
        join(&mut e);
        e.core.hands.test_start_using(1);
        assert!(e.core.hands.using_item(), "{v:?}");
        e.handle(Inbound::GameState {
            reason: 3,
            value: 3.0,
        });
        assert_eq!(e.core.game_mode, GameMode::Spectator, "{v:?}");
        assert_eq!(e.core.hands.using_item(), !stops, "{v:?}");
    }
}

/// 1.8.9 / 1.12.2 `canPlaceItemBlock` before the block packet: decided for a spot that is not
/// replaceable and for a full-cube block in the player's box; anvils, slabs and non-block items
/// are left alone. `ctx.place` then sends nothing.
#[test]
fn legacy_client_suppresses_unplaceable_blocks() {
    use crate::common::Face;
    for v in [GameVersion::V1_8_9, GameVersion::V1_12_2] {
        let mut e = engine(v);
        join(&mut e);
        let under = BlockPos::new(8, 63, 8);
        let ahead = BlockPos::new(8, 63, 10);
        // Into the player's own box (cobblestone 4, stone 1 with its variants).
        assert!(e.sim.place_suppressed(under, Face::Up, 4), "{v:?}");
        assert!(e.sim.place_suppressed(under, Face::Up, 1), "{v:?}");
        // Free air.
        assert!(!e.sim.place_suppressed(ahead, Face::Up, 4), "{v:?}");
        // The spot next to the face is stone: no block item fits (torch 50 too).
        assert!(e.sim.place_suppressed(ahead, Face::North, 4), "{v:?}");
        assert!(e.sim.place_suppressed(ahead, Face::North, 50), "{v:?}");
        // Not decided: anvil, stone slab; not a block item: stick.
        assert!(!e.sim.place_suppressed(ahead, Face::North, 145), "{v:?}");
        assert!(!e.sim.place_suppressed(ahead, Face::North, 44), "{v:?}");
        assert!(!e.sim.place_suppressed(ahead, Face::North, 280), "{v:?}");
        // A torch has no collision box: it may go into the player's box.
        assert!(!e.sim.place_suppressed(under, Face::Up, 50), "{v:?}");
        // Tall grass is replaced itself: the spot is the grass, not the block above.
        legacy_set(&mut e, v, BlockPos::new(8, 64, 10), 31 << 4 | 1);
        assert!(
            !e.sim
                .place_suppressed(BlockPos::new(8, 64, 10), Face::Up, 4),
            "{v:?}"
        );
    }
    // 1.16.5 has no such check.
    let mut e = engine(GameVersion::V1_16_5);
    join(&mut e);
    assert!(!e.sim.place_suppressed(BlockPos::new(8, 63, 8), Face::Up, 1));
}

/// 1.12.2 with entity tracking: a cow standing in the spot (client position) makes vanilla's
/// `hasEntityIn` fail, so no block packet; without tracking the cow is unknown and the packet
/// is sent as before. 1.8.9 never decides other entities.
#[test]
fn tracked_entity_in_the_spot_suppresses_on_1_12_2() {
    use crate::common::Face;
    use ghast_protocol::v1_12_2::play::clientbound as cb;
    let cow = |id: i32| {
        RawPacket::from_packet(&cb::SpawnEntityLiving {
            entity_id: id,
            entity_uuid: uuid::Uuid::from_u128(0x636f_7700 + id as u128),
            r#type: 92,
            x: 8.5,
            y: 64.0,
            z: 10.5,
            yaw: 0,
            pitch: 0,
            head_pitch: 0,
            velocity: ghast_protocol::defs::p47::types::Vec3i16 { x: 0, y: 0, z: 0 },
            metadata: Vec::new(),
        })
        .unwrap()
    };
    let ahead = BlockPos::new(8, 63, 10);
    let mut e = engine(GameVersion::V1_12_2);
    join(&mut e);
    e.sim.world_packet(&cow(50)).unwrap();
    assert!(!e.sim.place_suppressed(ahead, Face::Up, 4), "tracking off");
    e.sim.set_entities(Some(ghast_entity::Config::default()));
    e.sim.world_packet(&cow(51)).unwrap();
    assert!(e.sim.place_suppressed(ahead, Face::Up, 4));
    // A torch has no collision box: the cow does not matter.
    assert!(!e.sim.place_suppressed(ahead, Face::Up, 50));
    // The cow's box ends at x 8.95: the spot west of it is free.
    assert!(
        !e.sim
            .place_suppressed(BlockPos::new(9, 63, 10), Face::Up, 4)
    );

    let mut e = engine(GameVersion::V1_8_9);
    join(&mut e);
    e.sim.set_entities(Some(ghast_entity::Config::default()));
    let raw = RawPacket::from_packet(
        &ghast_protocol::v1_8::play::clientbound::SpawnEntityLiving {
            entity_id: 51,
            r#type: 92,
            x: 8 * 32 + 16,
            y: 64 * 32,
            z: 10 * 32 + 16,
            yaw: 0,
            pitch: 0,
            head_pitch: 0,
            velocity: ghast_protocol::defs::p47::types::Vec3i16 { x: 0, y: 0, z: 0 },
            metadata: Vec::new(),
        },
    )
    .unwrap();
    e.sim.world_packet(&raw).unwrap();
    assert!(
        !e.sim.place_suppressed(ahead, Face::Up, 4),
        "1.8.9: not decided"
    );
}

/// `ctx.place` into the player's own box on 1.8.9 / 1.12.2 writes nothing and reports it.
#[test]
fn legacy_place_into_the_player_is_refused() {
    for v in [GameVersion::V1_8_9, GameVersion::V1_12_2] {
        let mut e = engine(v);
        join(&mut e);
        e.inventory_mut()
            .on_set_slot(0, 36, Some(ghast_inventory::Stack::new(4, 16, 0)));
        let t0 = Instant::now();
        e.note_inbound(t0);
        let under = BlockPos::new(8, 63, 8);
        let mut tried = 0;
        let mut accepted = false;
        for _ in 0..20 {
            let mut b = Script(|ctx: &mut TickContext<'_>| {
                ctx.set_aim(0.0, 90.0);
                if ctx.target().is_some_and(|t| t.pos == under) {
                    tried += 1;
                    accepted |=
                        ctx.place(under, crate::common::Face::Up, Vec3::new(8.5, 64.0, 8.5));
                }
            });
            e.tick(&mut b, t0);
            assert!(
                !e.outbound()
                    .iter()
                    .any(|o| matches!(o, Outbound::Place { .. })),
                "{v:?}"
            );
            e.outbound().clear();
        }
        assert!(tried > 0 && !accepted, "{v:?}");
    }
}

/// Replays of the oracle's right clicks (`tests/golden/<version>/place.jsonl.gz`, the real
/// client's `MultiPlayerGameMode.useItemOn` and its prediction records): every placement this
/// client predicts is the vanilla one, and after the server's answer (an ack alone, a block
/// update then the ack, or the client's own state then the ack, with the player moved in
/// between) the clicked block, the target block and the player's position are vanilla's.
mod place_golden {
    use super::*;
    use serde_json::Value;
    use std::io::BufRead;

    /// The golden's player stands at (0.5, 64, 0.5); the test engine's at (8.5, 64, 8.5).
    const SHIFT: i32 = 8;

    fn f64_of(v: &Value) -> f64 {
        let bits = v[1].as_str().expect("hex bits");
        f64::from_bits(u64::from_str_radix(bits, 16).expect("hex"))
    }

    fn pos_of(v: &Value) -> BlockPos {
        BlockPos::new(
            v[0].as_i64().unwrap() as i32 + SHIFT,
            v[1].as_i64().unwrap() as i32,
            v[2].as_i64().unwrap() as i32 + SHIFT,
        )
    }

    fn vec_of(v: &Value) -> Vec3 {
        Vec3::new(
            f64_of(&v[0]) + f64::from(SHIFT),
            f64_of(&v[1]),
            f64_of(&v[2]) + f64::from(SHIFT),
        )
    }

    fn item_id(v: GameVersion, name: &str) -> i32 {
        match v {
            #[cfg(feature = "v26_2")]
            GameVersion::V26_2 => i32::from(ghast_data::v26_2::Item::from_name(name).unwrap().id()),
            _ => i32::from(ghast_data::v26_1::Item::from_name(name).unwrap().id()),
        }
    }

    fn block_name(v: GameVersion, id: u32) -> String {
        match v {
            #[cfg(feature = "v26_2")]
            GameVersion::V26_2 => ghast_data::v26_2::BlockState::from_id(id as u16)
                .unwrap()
                .block()
                .name()
                .to_string(),
            _ => ghast_data::v26_1::BlockState::from_id(id as u16)
                .unwrap()
                .block()
                .name()
                .to_string(),
        }
    }

    fn replay(v: GameVersion, label: &str) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/golden")
            .join(label)
            .join("place.jsonl.gz");
        let Ok(file) = std::fs::File::open(&path) else {
            panic!("missing {}", path.display());
        };
        let lines = std::io::BufReader::new(flate2::read::GzDecoder::new(file)).lines();
        let mut e = modern(v);
        let mut n = 0;
        let mut predicted = 0;
        let mut vanilla_placed = 0;
        let mut missed: std::collections::BTreeMap<String, u32> = Default::default();
        let mut sequence = 0;
        let mut bad = Vec::new();
        // Predictions where vanilla also changed neighbours (shape updates).
        let mut extra: std::collections::BTreeMap<String, u32> = Default::default();
        for line in lines {
            let c: Value = serde_json::from_str(&line.unwrap()).unwrap();
            n += 1;
            let k = c["k"].as_i64().unwrap();
            let clicked = pos_of(&c["clicked"]);
            let face = crate::common::Face::from_id(c["face"].as_i64().unwrap() as i8).unwrap();
            let target = face.offset(clicked);
            let (cs, ts) = (
                c["clicked_state"].as_u64().unwrap() as u16,
                c["target_state"].as_u64().unwrap() as u16,
            );
            // Nothing predicted from the case before; the case's blocks.
            sequence += 1;
            e.sim.world_packet(&ack(v, i32::MAX >> 1)).unwrap();
            let _ = e.sim.take_snap();
            for p in [clicked, target] {
                let d = (p.x - SHIFT).abs().max((p.z - SHIFT).abs());
                let floor = if p.y == 63 && d <= 4 {
                    state(v, "stone")
                } else {
                    0
                };
                e.sim.world_packet(&block_update(v, p, floor)).unwrap();
            }
            e.sim.world_packet(&block_update(v, clicked, cs)).unwrap();
            e.sim.world_packet(&block_update(v, target, ts)).unwrap();
            let player = vec_of(&c["player"]);
            e.sim.set_player_pos(player);
            let held = c["held"].as_str().map(|name| {
                ghast_inventory::Stack::new(
                    item_id(v, name),
                    c["count"].as_i64().unwrap() as i32,
                    0,
                )
            });
            let game_mode = match c["mode"].as_str().unwrap() {
                "survival" => GameMode::Survival,
                "creative" => GameMode::Creative,
                "adventure" => GameMode::Adventure,
                _ => GameMode::Spectator,
            };
            e.sim.place_sent(&crate::sim::predict::PlaceSent {
                pos: clicked,
                face,
                held: held.as_ref(),
                sneaking: c["sneaking"].as_bool().unwrap(),
                any_item: held.is_some(),
                game_mode,
                sequence,
                player,
            });
            let ours = block(&e, target).unwrap();
            let changed: Vec<(BlockPos, u32)> = c["changed"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| (pos_of(&x[0]), x[1].as_u64().unwrap() as u32))
                .collect();
            let vanilla_target = changed.iter().find(|x| x.0 == target).map(|x| x.1);
            if vanilla_target.is_some() {
                vanilla_placed += 1;
            }
            let we_predicted = ours != u32::from(ts);
            if we_predicted {
                predicted += 1;
                if vanilla_target != Some(ours) || c["result"] != "Success" {
                    bad.push(format!(
                        "case {k}: predicted {} at {target:?} clicking {}, vanilla {:?} ({}): {c}",
                        block_name(v, ours),
                        block_name(v, u32::from(cs)),
                        vanilla_target.map(|s| block_name(v, s)),
                        c["result"]
                    ));
                    continue;
                }
            } else if let Some(s) = vanilla_target {
                *missed
                    .entry(format!(
                        "{} on {}",
                        block_name(v, s),
                        block_name(v, u32::from(cs))
                    ))
                    .or_default() += 1;
            }
            if we_predicted && changed.len() > 1 {
                *extra
                    .entry(format!(
                        "{} next to {:?}",
                        block_name(v, ours),
                        changed
                            .iter()
                            .filter(|x| x.0 != target)
                            .map(|x| block_name(v, x.1))
                            .collect::<Vec<_>>()
                    ))
                    .or_default() += 1;
            }
            // The answer: comparable when the client changed what vanilla changed.
            let same = changed.is_empty() && !we_predicted || we_predicted && changed.len() == 1;
            if !same {
                continue;
            }
            e.sim.set_player_pos(Vec3::new(
                player.x,
                player.y - f64_of(&c["moved"]),
                player.z,
            ));
            let update = c["update"].as_i64().unwrap();
            if update >= 0 {
                e.sim
                    .world_packet(&block_update(v, target, update as u16))
                    .unwrap();
            }
            if c["sequence"].as_i64().unwrap() >= 0 {
                e.sim.world_packet(&ack(v, sequence)).unwrap();
            }
            assert_eq!(
                block(&e, target),
                Some(c["target_after"].as_u64().unwrap() as u32),
                "case {k} target after the answer: {c}"
            );
            assert_eq!(
                block(&e, clicked),
                Some(c["clicked_after"].as_u64().unwrap() as u32),
                "case {k} clicked after the answer: {c}"
            );
            let after = vec_of(&c["player_after"]);
            let now = e.sim.take_snap().unwrap_or(Vec3::new(
                player.x,
                player.y - f64_of(&c["moved"]),
                player.z,
            ));
            assert_eq!(now, after, "case {k} player after the answer: {c}");
        }
        assert!(n > 0);
        for b in bad.iter().take(12) {
            eprintln!("{b}");
        }
        assert!(
            bad.is_empty(),
            "{label}: {} predictions differ from vanilla",
            bad.len()
        );
        eprintln!("{label}: neighbours vanilla changed too: {extra:?}");
        let mut top: Vec<_> = missed.into_iter().collect();
        top.sort_by_key(|x| std::cmp::Reverse(x.1));
        eprintln!(
            "{label}: {n} clicks, vanilla placed {vanilla_placed}, predicted {predicted} (all \
             vanilla); not predicted (top): {:?}",
            &top[..top.len().min(25)]
        );
    }

    #[test]
    fn place_golden_26_1() {
        replay(GameVersion::V26_1, "26.1");
    }

    #[cfg(feature = "v26_2")]
    #[test]
    fn place_golden_26_2() {
        replay(GameVersion::V26_2, "26.2");
    }
}
