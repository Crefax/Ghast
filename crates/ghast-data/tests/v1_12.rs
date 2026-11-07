//! 1.12 tables: known facts from the vanilla 1.12.2 oracle dumps and the API contract.
#![cfg(feature = "v1_12")]

use ghast_data::v1_12::{self, Block, BlockState, Face, FaceShape, MaterialKind, StateFlags};
use ghast_data::{NeighbourShape, ShapeDependency};

#[test]
fn ids_names_and_gaps() {
    assert_eq!(Block::from_name("stone"), Some(Block::STONE));
    assert_eq!(Block::STONE.id(), 1);
    assert_eq!(
        Block::from_id(255).map(Block::name),
        Some("structure_block")
    );
    // 253 and 254 are not 1.12 blocks: they read as air.
    assert!(Block::from_id(253).is_none() && Block::from_id(254).is_none());
    assert_eq!(BlockState::from_raw_vanilla(253 << 4), BlockState::AIR);
    assert_eq!(Block::all().count(), 254);
    for b in Block::all() {
        assert_eq!(Block::from_name(b.name()), Some(b), "{}", b.name());
    }
}

#[test]
fn registered_metas_decode_like_the_palette() {
    // stone has metas 0-6 (granite .. polished andesite); 7+ read as air.
    assert!(BlockState::new(Block::STONE, 6).is_registered());
    assert_eq!(BlockState::from_raw_vanilla(1 << 4 | 7), BlockState::AIR);
    // tripwire meta 15 is not registered in 1.12.
    let tripwire = Block::from_name("tripwire").unwrap();
    assert!(BlockState::new(tripwire, 14).is_registered());
    assert!(!BlockState::new(tripwire, 15).is_registered());
}

#[test]
fn shapes_flags_and_materials() {
    let ladder = Block::LADDER.with_meta(2);
    assert_eq!(ladder.shape()[0].min_z, 0.8125); // 3/16 thick since 1.9
    let farmland = Block::FARMLAND.default_state();
    assert_eq!(farmland.shape()[0].max_y, 0.9375);
    assert_eq!(Block::GRASS_PATH.default_state().shape()[0].max_y, 0.9375);
    assert!(Block::STONE.default_state().is_normal_cube());
    assert!(!Block::GLASS.default_state().is_normal_cube());
    // Glass is not a "full cube" in 1.12 (render flag), stone is.
    assert!(
        !Block::GLASS
            .default_state()
            .flags()
            .contains(StateFlags::FULL_CUBE)
    );
    assert!(
        Block::STONE
            .default_state()
            .flags()
            .contains(StateFlags::FULL_CUBE)
    );
    assert_eq!(Block::WATER.material().kind, MaterialKind::Water);
    assert_eq!(Block::AIR.material().kind, MaterialKind::Air);
    assert_eq!(Block::ICE.physics().slipperiness, 0.98);
    assert_eq!(Block::FROSTED_ICE.physics().slipperiness, 0.98);
    assert_eq!(
        Block::FENCE.shape_dependency(),
        ShapeDependency::Neighbours(NeighbourShape::Fence)
    );
    assert_eq!(
        Block::CHORUS_PLANT.shape_dependency(),
        ShapeDependency::Neighbours(NeighbourShape::ChorusPlant)
    );
    // Fences show a pole to their sides, stone a solid face; leaves are excluded from fences.
    assert_eq!(
        Block::FENCE.default_state().face_shape(Face::North),
        FaceShape::MiddlePole
    );
    assert_eq!(
        Block::STONE.default_state().face_shape(Face::Up),
        FaceShape::Solid
    );
    assert!(Block::LEAVES.fence_excluded());
    assert!(!Block::STONE.fence_excluded());
}

#[test]
fn items_and_entities() {
    assert_eq!(v1_12::Item::from_name("shield").map(|i| i.id()), Some(442));
    assert!(v1_12::vanilla_entity_size("zombie").is_some());
    let s = v1_12::slime_dimensions(2);
    assert_eq!(s.width, 1.020_000_1);
}
