use super::*;

#[test]
fn state_strings_round_trip() {
    for id in 0..STATE_COUNT as u16 {
        let s = BlockState::from_id(id).unwrap();
        let text = s.to_string();
        assert_eq!(BlockState::parse(&text), Some(s), "{text}");
    }
}

#[test]
fn packed_records_are_consistent() {
    let air = BlockState::AIR.phys();
    assert!(air.has(state_flags::AIR | state_flags::EMPTY));
    let stone = Block::STONE.default_state().phys();
    assert!(stone.has(state_flags::FULL_CUBE | state_flags::SUFFOCATING));
    assert_eq!(stone.shape(), FULL_CUBE_VOXEL);
    let water = Block::WATER.default_state().phys();
    assert_eq!(water.fluid() >> 6, 1);
    assert_eq!(water.fluid() & 15, 8);
    assert_eq!(water.kind(), BlockKind::Water as u8);
    assert_eq!(Block::ICE.data().friction, 0.98);
    assert_eq!(Block::SOUL_SAND.data().speed_factor, 0.4);
    assert_eq!(Block::HONEY_BLOCK.data().jump_factor, 0.5);
    assert!(Block::LADDER.data().flags & block_flags::CLIMBABLE != 0);
}

/// The client push-out (`moveTowardsClosestSpace`) only reads the cells inside its probe column:
/// the face / edge cells of vanilla's cursor count only for suffocating large-shape (or moving
/// piston) states, and every such state's shape stays inside its own cube, so it can never reach
/// the probe area. If a future data set breaks this, the physics shortcut must go.
#[test]
fn suffocating_large_shapes_stay_in_their_cube() {
    for id in 0..STATE_COUNT as u16 {
        let s = BlockState::from_id(id).unwrap();
        let p = s.phys();
        if !p.has(state_flags::SUFFOCATING) {
            continue;
        }
        if p.has(state_flags::LARGE) || p.has(state_flags::MOVING_PISTON) {
            assert!(
                !p.has(state_flags::CONTEXT) && !p.has(state_flags::OFFSET),
                "{s}"
            );
            for b in s.voxel_shape().boxes {
                assert!(
                    b.min_x >= 0.0
                        && b.min_y >= 0.0
                        && b.min_z >= 0.0
                        && b.max_x <= 1.0
                        && b.max_y <= 1.0
                        && b.max_z <= 1.0,
                    "{s}: {b:?}"
                );
            }
        }
    }
}
