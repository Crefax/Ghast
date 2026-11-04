//! 1.16.5 table spot checks and state encoding round trips (no external files needed).
#![cfg(feature = "v1_16_5")]

use ghast_data::v1_16_5::{self, Block, BlockState, Item, Property};
use ghast_data::{DigMaterial, FluidKind, PhysicsFlags, ShapeDependency};

fn boxes(s: BlockState) -> Vec<[f64; 6]> {
    s.shape()
        .iter()
        .map(|a| [a.min_x, a.min_y, a.min_z, a.max_x, a.max_y, a.max_z])
        .collect()
}

#[test]
fn counts() {
    assert_eq!(v1_16_5::BLOCK_COUNT, 763);
    assert_eq!(v1_16_5::STATE_COUNT, 17112);
    assert_eq!(Item::all().count(), 975);
    assert_eq!(v1_16_5::biomes().len(), 79);
    assert_eq!(v1_16_5::enchantments().len(), 38);
    assert_eq!(v1_16_5::entities().len(), 108);
}

#[test]
fn every_state_round_trips_through_its_properties() {
    let mut seen = 0usize;
    for b in Block::all() {
        let l = b.layout();
        assert_eq!(b.states().len(), usize::from(l.state_count));
        let product: usize = b.properties().map(|d| d.values.len()).product();
        assert_eq!(product, usize::from(l.state_count), "{}", b.name());
        assert_eq!(b.default_state().block(), b);
        for s in b.states() {
            seen += 1;
            assert_eq!(s.block(), b);
            assert_eq!(BlockState::from_id(s.id()), Some(s));
            let props: Vec<(Property, &str)> = s.properties().collect();
            assert_eq!(props.len(), b.properties().len());
            assert_eq!(b.state_with(&props), Some(s), "{s}");
            // Setting each property to its current value is the identity; any other value
            // changes exactly that property.
            for d in b.properties() {
                assert_eq!(s.with(d.property, s.get(d.property).unwrap()), Some(s));
                for v in d.values {
                    let t = s.with(d.property, v).unwrap();
                    assert_eq!(t.get(d.property), Some(*v));
                    for e in b.properties().filter(|e| e.property != d.property) {
                        assert_eq!(t.get(e.property), s.get(e.property));
                    }
                }
            }
        }
    }
    assert_eq!(seen, v1_16_5::STATE_COUNT);
    assert_eq!(BlockState::from_id(17112), None);
}

#[test]
fn names_and_display() {
    for b in Block::all() {
        assert_eq!(Block::from_name(b.name()), Some(b));
    }
    for i in Item::all() {
        assert_eq!(Item::from_name(i.info().name), Some(i));
    }
    assert_eq!(Property::from_name("facing"), Some(Property::Facing));
    assert_eq!(Property::Waterlogged.name(), "waterlogged");
    assert_eq!(Block::STONE.default_state().to_string(), "minecraft:stone");
    assert_eq!(
        Block::OAK_STAIRS.default_state().to_string(),
        "minecraft:oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]"
    );
    assert_eq!(Block::OAK_STAIRS.default_state().id(), 1965);
    assert_eq!(Block::STONE.default_state().id(), 1);
    assert_eq!(
        Block::from_name("oak_stairs").unwrap().layout().min_state,
        1954
    );
}

#[test]
fn collision_spot_checks() {
    assert!(BlockState::AIR.collision().is_empty());
    assert!(BlockState::AIR.is_air());
    assert!(Block::CAVE_AIR.default_state().is_air());
    assert!(Block::STONE.default_state().collision().is_full_cube());
    assert!(
        Block::GRASS_BLOCK
            .default_state()
            .collision()
            .is_full_cube()
    );
    let slab = |t: &str| Block::OAK_SLAB.state_with(&[(Property::Type, t)]).unwrap();
    assert_eq!(boxes(slab("bottom")), vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]);
    assert_eq!(boxes(slab("top")), vec![[0.0, 0.5, 0.0, 1.0, 1.0, 1.0]]);
    assert!(slab("double").collision().is_full_cube());
    assert_eq!(
        boxes(Block::SOUL_SAND.default_state()),
        vec![[0.0, 0.0, 0.0, 1.0, 0.875, 1.0]]
    );
    assert_eq!(
        boxes(Block::FARMLAND.default_state()),
        vec![[0.0, 0.0, 0.0, 1.0, 0.9375, 1.0]]
    );
    let ladder = Block::LADDER
        .state_with(&[(Property::Facing, "north")])
        .unwrap();
    assert_eq!(boxes(ladder), vec![[0.0, 0.0, 0.8125, 1.0, 1.0, 1.0]]);
    let snow = |n: &str| boxes(Block::SNOW.state_with(&[(Property::Layers, n)]).unwrap());
    assert_eq!(snow("1"), Vec::<[f64; 6]>::new());
    assert_eq!(snow("2"), vec![[0.0, 0.0, 0.0, 1.0, 0.125, 1.0]]);
    assert_eq!(snow("8"), vec![[0.0, 0.0, 0.0, 1.0, 0.875, 1.0]]);
    // Fences carry their connections in the state, so shapes are static.
    let post = Block::OAK_FENCE.state_with(&[]).unwrap();
    assert_eq!(post.collision().dependency, ShapeDependency::None);
    let isolated = Block::OAK_FENCE
        .state_with(&[
            (Property::North, "false"),
            (Property::South, "false"),
            (Property::East, "false"),
            (Property::West, "false"),
        ])
        .unwrap();
    assert_eq!(
        boxes(isolated),
        vec![[0.375, 0.0, 0.375, 0.625, 1.5, 0.625]]
    );
    let north = isolated.with(Property::North, "true").unwrap();
    assert!(
        north
            .shape()
            .iter()
            .any(|a| a.min_z == 0.0 && a.max_y == 1.5)
    );
    for b in [
        Block::COBWEB,
        Block::WATER,
        Block::LAVA,
        Block::GRASS,
        Block::TORCH,
        Block::VINE,
    ] {
        for s in b.states() {
            assert!(s.collision().is_empty(), "{s}");
        }
    }
    assert_eq!(
        Block::SCAFFOLDING.shape_dependency(),
        ShapeDependency::EntityContext
    );
    assert_eq!(
        Block::SHULKER_BOX.shape_dependency(),
        ShapeDependency::BlockEntity
    );
    assert_eq!(
        Block::RED_SHULKER_BOX.shape_dependency(),
        ShapeDependency::BlockEntity
    );
    assert_eq!(
        Block::MOVING_PISTON.shape_dependency(),
        ShapeDependency::BlockEntity
    );
    assert_eq!(
        Block::BAMBOO.shape_dependency(),
        ShapeDependency::PositionOffset
    );
    // scaffolding + 17 shulker boxes + moving piston + bamboo.
    assert_eq!(
        Block::all()
            .filter(|b| b.shape_dependency() != ShapeDependency::None)
            .count(),
        20
    );
}

#[test]
fn physics_and_fluids() {
    for b in Block::all() {
        let expect = match b.name() {
            "ice" | "packed_ice" | "frosted_ice" => 0.98,
            "blue_ice" => 0.989,
            "slime_block" => 0.8,
            _ => 0.6,
        };
        assert_eq!(b.physics().slipperiness, expect, "{}", b.name());
    }
    assert_eq!(Block::SOUL_SAND.physics().speed_factor, f64::from(0.4_f32));
    assert_eq!(
        Block::HONEY_BLOCK.physics().speed_factor,
        f64::from(0.4_f32)
    );
    assert_eq!(Block::HONEY_BLOCK.physics().jump_factor, 0.5);
    assert_eq!(
        Block::COBWEB.physics().stuck_multiplier,
        [0.25, f64::from(0.05_f32), 0.25]
    );
    assert_eq!(
        Block::SWEET_BERRY_BUSH.physics().stuck_multiplier,
        [f64::from(0.8_f32), 0.75, f64::from(0.8_f32)]
    );
    assert_eq!(
        Block::all()
            .filter(|b| b.physics().has(PhysicsFlags::CLIMBABLE))
            .count(),
        7
    );
    assert!(Block::RED_BED.physics().has(PhysicsFlags::BED_BOUNCE));
    let water = |lvl: &str| {
        Block::WATER
            .state_with(&[(Property::Level, lvl)])
            .unwrap()
            .fluid()
            .unwrap()
    };
    assert_eq!((water("0").kind, water("0").level), (FluidKind::Water, 0));
    assert_eq!(water("9").level, 9);
    assert_eq!(
        Block::LAVA.default_state().fluid().unwrap().kind,
        FluidKind::Lava
    );
    let stairs = Block::OAK_STAIRS.default_state();
    assert_eq!(stairs.fluid(), None);
    let wet = stairs.with(Property::Waterlogged, "true").unwrap();
    assert_eq!(
        wet.fluid().map(|f| (f.kind, f.level)),
        Some((FluidKind::Water, 0))
    );
    assert_eq!(
        Block::KELP.default_state().fluid().unwrap().kind,
        FluidKind::Water
    );
    assert_eq!(Block::STONE.default_state().fluid(), None);
}

#[test]
fn dig_inputs_and_misc() {
    let stone = Block::STONE.info();
    assert_eq!(stone.hardness, Some(1.5));
    assert_eq!(stone.material, Some(DigMaterial::Rock));
    assert!(stone.can_harvest_with(Some(Item::NETHERITE_PICKAXE.id())));
    assert!(!stone.can_harvest_with(None));
    assert_eq!(
        v1_16_5::tool_multiplier(DigMaterial::Rock, Item::NETHERITE_PICKAXE.id()),
        9.0
    );
    assert_eq!(
        v1_16_5::tool_multiplier(DigMaterial::Rock, Item::GOLDEN_PICKAXE.id()),
        12.0
    );
    assert!(!Block::BEDROCK.info().diggable);
    assert_eq!(Item::DIAMOND_PICKAXE.info().max_durability, Some(1561));
    assert_eq!(v1_16_5::food(Item::APPLE.id()).unwrap().food_points, 4);
    assert_eq!(v1_16_5::food(Item::BREAD.id()).unwrap().name, "bread");
    let zombie = v1_16_5::entity_by_name("zombie").unwrap();
    assert_eq!((zombie.width, zombie.height), (Some(0.6), Some(1.95)));
    assert_eq!(v1_16_5::biome_by_name("plains").unwrap().id, 1);
    assert_eq!(
        v1_16_5::enchantment_by_name("efficiency")
            .unwrap()
            .max_level,
        5
    );
}

#[test]
fn tags_and_fluid_amounts() {
    let tagged = |f: PhysicsFlags| Block::all().filter(|b| b.physics().has(f)).count();
    // 6 wooden + crimson + warped + nether brick.
    assert_eq!(tagged(PhysicsFlags::FENCE), 9);
    assert_eq!(tagged(PhysicsFlags::WALL), 17);
    assert_eq!(tagged(PhysicsFlags::FENCE_GATE), 8);
    assert_eq!(tagged(PhysicsFlags::TRAPDOOR), 9);
    assert_eq!(tagged(PhysicsFlags::SOUL_SPEED), 2);
    assert!(!Block::OAK_FENCE_GATE.physics().has(PhysicsFlags::FENCE));
    assert!(!Block::OAK_WALL_SIGN.physics().has(PhysicsFlags::WALL));
    assert_eq!(Block::SOUL_SAND.physics().velocity_multiplier(), 0.4_f32);
    assert_eq!(Block::STONE.physics().velocity_multiplier(), 1.0);
    let water = |lvl: &str| {
        Block::WATER
            .state_with(&[(Property::Level, lvl)])
            .unwrap()
            .fluid()
            .unwrap()
    };
    assert_eq!(
        (
            water("0").amount(),
            water("0").still(),
            water("0").falling()
        ),
        (8, true, false)
    );
    assert_eq!((water("1").amount(), water("1").falling()), (7, false));
    assert_eq!((water("7").amount(), water("7").falling()), (1, false));
    assert_eq!((water("8").amount(), water("8").falling()), (8, true));
    assert_eq!((water("15").amount(), water("15").falling()), (8, true));
    let wet = Block::OAK_STAIRS
        .default_state()
        .with(Property::Waterlogged, "true")
        .unwrap();
    assert!(wet.fluid().unwrap().still());
}

#[test]
fn identity_predicates() {
    for b in Block::all() {
        assert_eq!(b.is_bed(), b.name().ends_with("_bed"), "{}", b.name());
        assert_eq!(
            b.is_bed(),
            b.physics().has(PhysicsFlags::BED_BOUNCE),
            "{}",
            b.name()
        );
        assert_eq!(b.is_air(), b.name().ends_with("air"), "{}", b.name());
    }
    const _: () = assert!(Block::WATER.id() == 26 && !Block::BUBBLE_COLUMN.is_air());
}

#[test]
fn item_fixes() {
    let w = Item::WARPED_FUNGUS_ON_A_STICK.info();
    assert_eq!((w.stack_size, w.max_durability), (1, Some(100)));
    assert_eq!(Item::CARROT_ON_A_STICK.info().stack_size, 1);
}

#[test]
fn vanilla_state_facts() {
    use ghast_data::v1_16_5::OffsetType;
    assert!(Block::STONE.default_state().full_cube());
    assert!(Block::STONE.default_state().solid_block());
    assert!(Block::STONE.default_state().should_suffocate());
    assert!(!Block::GLASS.default_state().should_suffocate());
    assert!(!Block::OAK_STAIRS.default_state().full_cube());
    assert!(!Block::COBWEB.vanilla().collidable);
    assert_eq!(Block::BAMBOO.vanilla().offset, OffsetType::Xz);
    let water = Block::WATER.default_state().fluid().unwrap();
    assert_eq!(water.height(), 8.0 / 9.0);
    let top = Block::SCAFFOLDING
        .state_with(&[(Property::Bottom, "false"), (Property::Distance, "3")])
        .unwrap()
        .scaffolding_shapes()
        .unwrap();
    assert!(!top.stable.is_empty() && top.bottom.is_empty());
    let bottom = Block::SCAFFOLDING
        .state_with(&[(Property::Bottom, "true"), (Property::Distance, "3")])
        .unwrap()
        .scaffolding_shapes()
        .unwrap();
    assert_eq!(bottom.bottom.len(), 1);
    assert_eq!(bottom.bottom[0].max_y, 0.125);
    assert!(Block::STONE.default_state().scaffolding_shapes().is_none());
}

#[test]
fn voxel_api() {
    use ghast_data::v1_16_5::{Material, Side, VoxelClass};
    let stone = Block::STONE.default_state();
    assert!(stone.full_cube_identity());
    let v = stone.voxel_shape();
    assert_eq!(
        (v.class, v.sizes, v.empty),
        (VoxelClass::Simple, [1, 1, 1], false)
    );
    assert!(v.filled(0, 0, 0) && !v.filled(1, 0, 0));
    assert!(BlockState::AIR.voxel_shape().empty);
    let slab = Block::OAK_SLAB
        .state_with(&[(Property::Type, "bottom")])
        .unwrap();
    assert_eq!(slab.voxel_shape().ys, &[0.0, 0.5, 1.0]);
    assert!(!slab.side_solid_full_square(Side::North));
    assert!(stone.side_solid_full_square(Side::West));
    assert_eq!(Block::ICE.material(), Material::Ice);
    assert_eq!(Block::FROSTED_ICE.material(), Material::Ice);
    assert_eq!(Block::PACKED_ICE.material(), Material::DenseIce);
    assert!(stone.material_blocks_movement() && !BlockState::AIR.material_blocks_movement());
    assert!(Block::SCAFFOLDING.default_state().has_dynamic_bounds());
    assert!(Block::FIRE.physics().has(PhysicsFlags::FIRE));
}

#[test]
fn scaffolding_select() {
    let bottom = Block::SCAFFOLDING
        .state_with(&[(Property::Bottom, "true"), (Property::Distance, "2")])
        .unwrap()
        .scaffolding_shapes()
        .unwrap();
    assert_eq!(bottom.select(65.0, 64, false).0.len(), bottom.stable.len());
    assert_eq!(bottom.select(65.0, 64, true).0, bottom.bottom);
    assert_eq!(bottom.select(64.0, 64, false).0, bottom.bottom);
    assert_eq!(bottom.select(63.999_999, 64, false).0, bottom.bottom);
    assert!(bottom.select(63.999_98, 64, false).0.is_empty());
    assert!(bottom.select(63.999_98, 64, false).1.empty);
    let top = Block::SCAFFOLDING
        .default_state()
        .scaffolding_shapes()
        .unwrap();
    assert!(top.select(64.5, 64, false).0.is_empty());
    assert!(!top.select(64.999_995, 64, false).0.is_empty());
    assert!(top.select(64.999_98, 64, false).0.is_empty());
}
