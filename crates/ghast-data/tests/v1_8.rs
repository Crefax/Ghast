//! 1.8 table spot checks and encoding round trips (no external files needed).
#![cfg(feature = "v1_8")]

use ghast_data::v1_8::{self, Block, BlockState, Item, MaterialKind};
use ghast_data::{
    Aabb, DigMaterial, EntityKind, FluidKind, NeighbourShape, PhysicsFlags, ShapeDependency,
};

fn boxes(b: Block, meta: u8) -> Vec<[f64; 6]> {
    b.with_meta(meta)
        .shape()
        .iter()
        .map(|a| [a.min_x, a.min_y, a.min_z, a.max_x, a.max_y, a.max_z])
        .collect()
}

#[test]
fn counts() {
    assert_eq!(v1_8::BLOCK_COUNT, 198);
    assert_eq!(Block::all().count(), 198);
    assert_eq!(Item::all().count(), 337);
    assert_eq!(v1_8::biomes().len(), 61);
    assert_eq!(v1_8::enchantments().len(), 25);
    assert_eq!(v1_8::effects().len(), 23);
    assert_eq!(v1_8::entities().len(), 58);
}

#[test]
fn raw_state_round_trip() {
    for raw in 0..=u16::MAX {
        match BlockState::from_raw(raw) {
            Some(s) => {
                assert!(usize::from(raw >> 4) < v1_8::BLOCK_COUNT);
                assert_eq!(s.raw(), raw);
                assert_eq!(s.block().id() as u16, raw >> 4);
                assert_eq!(s.meta() as u16, raw & 15);
                assert_eq!(BlockState::new(s.block(), s.meta()), s);
            }
            None => assert!(usize::from(raw >> 4) >= v1_8::BLOCK_COUNT),
        }
    }
    assert_eq!(Block::from_id(197).map(Block::name), Some("dark_oak_door"));
    assert_eq!(Block::from_id(198), None);
}

#[test]
fn name_round_trip() {
    for b in Block::all() {
        assert_eq!(Block::from_name(b.name()), Some(b));
        assert_eq!(b.info().id, u16::from(b.id()));
    }
    for i in Item::all() {
        assert_eq!(Item::from_name(i.info().name), Some(i));
        assert_eq!(Item::from_id(i.id()), Some(i));
    }
    assert_eq!(Block::from_name("minecraft:stone"), None);
    assert_eq!(Block::from_name("nope"), None);
}

#[test]
fn constants() {
    assert_eq!(Block::AIR.id(), 0);
    assert_eq!(Block::STONE.id(), 1);
    assert_eq!(Block::SOUL_SAND.id(), 88);
    assert_eq!(Item::DIAMOND_PICKAXE.id(), 278);
    assert_eq!(Item::DIAMOND_PICKAXE.info().max_durability, Some(1561));
    #[cfg(feature = "display-names")]
    assert_eq!(Item::STONE.info().variations.len(), 7);
}

#[test]
fn collision_spot_checks() {
    assert!(BlockState::AIR.collision().is_empty());
    assert!(Block::STONE.default_state().collision().is_full_cube());
    assert!(Block::GRASS.default_state().collision().is_full_cube());
    // Slabs: meta 0..7 bottom, 8..15 top.
    assert_eq!(
        boxes(Block::STONE_SLAB, 0),
        vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]
    );
    assert_eq!(
        boxes(Block::STONE_SLAB, 8),
        vec![[0.0, 0.5, 0.0, 1.0, 1.0, 1.0]]
    );
    assert_eq!(
        boxes(Block::WOODEN_SLAB, 3),
        vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]
    );
    assert!(
        Block::DOUBLE_STONE_SLAB
            .default_state()
            .collision()
            .is_full_cube()
    );
    // Soul sand is 14/16 high.
    assert_eq!(
        boxes(Block::SOUL_SAND, 0),
        vec![[0.0, 0.0, 0.0, 1.0, 0.875, 1.0]]
    );
    // Cactus is inset by 1/16 (and 1/16 lower).
    assert_eq!(
        boxes(Block::CACTUS, 0),
        vec![[0.0625, 0.0, 0.0625, 0.9375, 0.9375, 0.9375]]
    );
    // Snow layers: meta m -> m/8 high; one layer is a zero-height box (oracle override).
    assert_eq!(
        boxes(Block::SNOW_LAYER, 0),
        vec![[0.0, 0.0, 0.0, 1.0, 0.0, 1.0]]
    );
    assert!(!Block::SNOW_LAYER.with_meta(0).collision().is_empty());
    assert_eq!(
        boxes(Block::SNOW_LAYER, 1),
        vec![[0.0, 0.0, 0.0, 1.0, 0.125, 1.0]]
    );
    assert_eq!(
        boxes(Block::SNOW_LAYER, 7),
        vec![[0.0, 0.0, 0.0, 1.0, 0.875, 1.0]]
    );
    // 1.8 ladders are 2/16 thick and farmland is a full cube (oracle overrides).
    assert_eq!(
        boxes(Block::LADDER, 2),
        vec![[0.0, 0.0, 0.875, 1.0, 1.0, 1.0]]
    );
    assert!(Block::FARMLAND.with_meta(7).collision().is_full_cube());
    // Unregistered metas read as air in vanilla: no box, not registered.
    let stone7 = Block::STONE.with_meta(7);
    assert!(!stone7.is_registered() && stone7.collision().is_empty());
    assert_eq!(BlockState::from_raw_vanilla(stone7.raw()), BlockState::AIR);
    assert_eq!(BlockState::from_raw_vanilla(198 << 4), BlockState::AIR);
    let granite = Block::STONE.with_meta(1);
    assert!(granite.is_registered());
    assert_eq!(BlockState::from_raw_vanilla(granite.raw()), granite);
    assert!(!Block::FENCE.with_meta(1).is_registered());
    // Passable blocks.
    for b in [
        Block::WEB,
        Block::TALLGRASS,
        Block::TORCH,
        Block::WATER,
        Block::LAVA,
        Block::VINE,
        Block::REDSTONE_WIRE,
    ] {
        for m in 0..16 {
            assert!(b.with_meta(m).collision().is_empty(), "{}:{m}", b.name());
        }
    }
    // Closed fence gate is 1.5 high; open has no collision.
    assert_eq!(
        boxes(Block::FENCE_GATE, 0),
        vec![[0.0, 0.0, 0.375, 1.0, 1.5, 0.625]]
    );
    assert!(Block::FENCE_GATE.with_meta(4).collision().is_empty());
}

#[test]
fn neighbour_dependent_blocks_are_flagged() {
    let dep = |b: Block| b.shape_dependency();
    assert_eq!(
        dep(Block::FENCE),
        ShapeDependency::Neighbours(NeighbourShape::Fence)
    );
    assert_eq!(
        dep(Block::NETHER_BRICK_FENCE),
        ShapeDependency::Neighbours(NeighbourShape::Fence)
    );
    assert_eq!(
        dep(Block::ACACIA_FENCE),
        ShapeDependency::Neighbours(NeighbourShape::Fence)
    );
    assert_eq!(dep(Block::ACACIA_FENCE_GATE), ShapeDependency::None);
    assert_eq!(
        dep(Block::GLASS_PANE),
        ShapeDependency::Neighbours(NeighbourShape::Pane)
    );
    assert_eq!(
        dep(Block::IRON_BARS),
        ShapeDependency::Neighbours(NeighbourShape::Pane)
    );
    assert_eq!(
        dep(Block::COBBLESTONE_WALL),
        ShapeDependency::Neighbours(NeighbourShape::Wall)
    );
    assert_eq!(
        dep(Block::OAK_STAIRS),
        ShapeDependency::Neighbours(NeighbourShape::Stairs)
    );
    assert_eq!(
        dep(Block::QUARTZ_STAIRS),
        ShapeDependency::Neighbours(NeighbourShape::Stairs)
    );
    assert_eq!(
        dep(Block::WOODEN_DOOR),
        ShapeDependency::Neighbours(NeighbourShape::Door)
    );
    assert_eq!(
        dep(Block::IRON_DOOR),
        ShapeDependency::Neighbours(NeighbourShape::Door)
    );
    assert_eq!(dep(Block::TRAPDOOR), ShapeDependency::None);
    assert_eq!(
        dep(Block::CHEST),
        ShapeDependency::Neighbours(NeighbourShape::Chest)
    );
    assert_eq!(dep(Block::ANVIL), ShapeDependency::None);
    assert_eq!(dep(Block::ENDER_CHEST), ShapeDependency::None);
    let flagged = Block::all()
        .filter(|b| b.shape_dependency() != ShapeDependency::None)
        .count();
    // 7 fences + 3 panes + 1 wall + 13 stairs + 7 doors + 2 chests + moving piston.
    assert_eq!(flagged, 34);
    assert_eq!(dep(Block::PISTON_EXTENSION), ShapeDependency::BlockEntity);
    let c = Block::FENCE.default_state().collision();
    assert!(!c.is_empty() && !c.is_full_cube());
    assert_eq!(c.boxes, &[Aabb::new(0.375, 0.0, 0.375, 0.625, 1.5, 0.625)]);
}

#[test]
fn physics_properties() {
    for b in Block::all() {
        let p = b.physics();
        let expect = match b.name() {
            "ice" | "packed_ice" => 0.98,
            "slime" => 0.8,
            _ => 0.6,
        };
        assert_eq!(p.slipperiness, expect, "{}", b.name());
    }
    assert_eq!(Block::SOUL_SAND.physics().speed_factor, 0.4);
    assert!(
        Block::SOUL_SAND
            .physics()
            .has(PhysicsFlags::COLLIDE_SLOWDOWN)
    );
    assert!(Block::LADDER.physics().has(PhysicsFlags::CLIMBABLE));
    assert!(Block::VINE.physics().has(PhysicsFlags::CLIMBABLE));
    assert_eq!(
        Block::all()
            .filter(|b| b.physics().has(PhysicsFlags::CLIMBABLE))
            .count(),
        2
    );
    let web = Block::WEB.physics();
    assert!(web.has(PhysicsFlags::STUCK));
    assert_eq!(web.stuck_multiplier, [0.25, 0.05000000074505806, 0.25]);
    assert!(Block::SLIME.physics().has(PhysicsFlags::SLIME));
    let water = Block::FLOWING_WATER.with_meta(3).fluid().unwrap();
    assert_eq!((water.kind, water.level), (FluidKind::Water, 3));
    assert_eq!(
        Block::LAVA.with_meta(0).fluid().unwrap().kind,
        FluidKind::Lava
    );
    assert_eq!(Block::STONE.default_state().fluid(), None);
}

#[test]
fn dig_inputs() {
    let stone = Block::STONE.info();
    assert_eq!(stone.hardness, Some(1.5));
    assert_eq!(stone.material, Some(DigMaterial::Rock));
    assert!(stone.requires_tool());
    assert!(stone.can_harvest_with(Some(Item::WOODEN_PICKAXE.id())));
    assert!(!stone.can_harvest_with(None));
    assert!(Block::DIRT.info().can_harvest_with(None));
    assert_eq!(Block::BEDROCK.info().hardness, None);
    assert_eq!(Block::OBSIDIAN.info().hardness, Some(50.0));
    assert_eq!(
        v1_8::tool_multiplier(DigMaterial::Rock, Item::DIAMOND_PICKAXE.id()),
        8.0
    );
    assert_eq!(
        v1_8::tool_multiplier(DigMaterial::Rock, Item::GOLDEN_PICKAXE.id()),
        12.0
    );
    assert_eq!(
        v1_8::tool_multiplier(DigMaterial::Dirt, Item::DIAMOND_PICKAXE.id()),
        1.0
    );
    assert_eq!(
        v1_8::tool_multiplier(DigMaterial::Web, Item::SHEARS.id()),
        15.0
    );
    assert_eq!(stone.drops[0].item, Block::COBBLESTONE.id().into());
}

#[test]
fn misc_tables() {
    let creeper = v1_8::entity(EntityKind::Mob, 50).unwrap();
    assert_eq!(creeper.name, "Creeper");
    assert_eq!((creeper.width, creeper.height), (Some(0.6), Some(1.8)));
    assert_eq!(
        v1_8::entity(EntityKind::Object, 50).unwrap().name,
        "PrimedTnt"
    );
    assert_eq!(v1_8::entity_by_name("Zombie").unwrap().id, 54);
    assert_eq!(v1_8::biome(1).unwrap().name, "plains");
    assert_eq!(v1_8::biome_by_name("desert").unwrap().id, 2);
    assert_eq!(v1_8::enchantment_by_name("efficiency").unwrap().id, 32);
    assert_eq!(v1_8::enchantment(32).unwrap().max_level, 5);
    assert_eq!(v1_8::food(Item::BREAD.id()).unwrap().food_points, 5);
    assert_eq!(v1_8::effect(1).unwrap().name, "Speed");
    assert!(v1_8::effect(1).unwrap().good);
}

#[test]
fn float_world_coordinates() {
    // Blocks whose 1.8 collision box is built in `float` world coordinates.
    let float_blocks: Vec<&str> = Block::all()
        .filter(|b| (0..16).any(|m| b.with_meta(m).collision().uses_float_world_coords()))
        .map(Block::name)
        .collect();
    assert_eq!(
        float_blocks,
        [
            "snow_layer",
            "cactus",
            "soul_sand",
            "cake",
            "fence_gate",
            "spruce_fence_gate",
            "birch_fence_gate",
            "jungle_fence_gate",
            "dark_oak_fence_gate",
            "acacia_fence_gate"
        ]
    );
    // Near the origin float and double placement agree.
    for b in Block::all() {
        for m in 0..16 {
            let c = b.with_meta(m).collision();
            for (w, a) in c.world_boxes(-7, 64, 300).zip(c.boxes) {
                assert_eq!(w, a.at(-7, 64, 300), "{}:{m}", b.name());
            }
        }
    }
    // Far away the float rounding shows: (float)20000001 = 2.0E7, + 0.0625F = 2.0E7 (ulp 2).
    let cactus = Block::CACTUS.default_state().collision();
    let far = cactus.world_box(0, 20_000_001, 70, -20_000_001);
    assert_eq!(far.min_x, 20_000_000.0);
    assert_eq!(far.max_x, 20_000_002.0);
    assert_eq!(far.min_y, 70.0);
    assert_eq!(far.max_y, 70.9375);
    assert_eq!(far.min_z, -20_000_000.0);
    assert_eq!(
        cactus.boxes[0].at(20_000_001, 70, 0).min_x,
        20_000_001.062_5
    );
    // Plain blocks use double placement.
    let slab = Block::STONE_SLAB.with_meta(8).collision();
    assert!(!slab.uses_float_world_coords());
    assert_eq!(slab.world_box(0, 20_000_001, 0, 0).min_y, 0.5);
    // Open fence gate: no box, no rules needed.
    assert!(
        Block::FENCE_GATE
            .with_meta(4)
            .collision()
            .world_coords
            .is_none()
    );
}

#[test]
fn vanilla_material_flags() {
    let kinds: Vec<(&str, MaterialKind)> = Block::all()
        .filter(|b| b.material().kind != MaterialKind::Other)
        .map(|b| (b.name(), b.material().kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("air", MaterialKind::Air),
            ("flowing_water", MaterialKind::Water),
            ("water", MaterialKind::Water),
            ("flowing_lava", MaterialKind::Lava),
            ("lava", MaterialKind::Lava),
            ("ice", MaterialKind::Ice),
        ]
    );
    for b in Block::all() {
        let liquid = b.physics().has(PhysicsFlags::WATER) || b.physics().has(PhysicsFlags::LAVA);
        assert_eq!(b.material().fluid, liquid, "{}", b.name());
    }
    assert!(Block::STONE.full_cube() && Block::STONE.vanilla().normal_block);
    assert!(Block::STONE.material().blocks_movement && Block::STONE.material().solid);
    assert!(!Block::GLASS.vanilla().full_block);
    assert!(!Block::AIR.material().blocks_movement && Block::AIR.material().replaceable);
    assert!(!Block::STONE_SLAB.full_cube());
    assert!(Block::WATER.material().fluid && !Block::WATER.material().solid);
}

#[test]
fn block_families() {
    let tagged = |f: PhysicsFlags| {
        Block::all()
            .filter(|b| b.physics().has(f))
            .map(Block::name)
            .collect::<Vec<_>>()
    };
    assert_eq!(tagged(PhysicsFlags::FENCE).len(), 7);
    assert_eq!(tagged(PhysicsFlags::WALL), ["cobblestone_wall"]);
    assert_eq!(tagged(PhysicsFlags::FENCE_GATE).len(), 6);
    assert_eq!(
        tagged(PhysicsFlags::TRAPDOOR),
        ["trapdoor", "iron_trapdoor"]
    );
    assert!(tagged(PhysicsFlags::SOUL_SPEED).is_empty());
}

#[test]
fn item_fixes() {
    let f = Item::from_id(62).unwrap();
    assert_eq!(f, Item::LIT_FURNACE);
    assert_eq!((f.info().name, f.info().stack_size), ("lit_furnace", 64));
}

#[test]
fn selection_api() {
    let stone = Block::STONE.default_state().selection();
    assert!(stone.can_collide && stone.has_collision_box && !stone.multi_pass_ray);
    assert_eq!(stone.bounds, Aabb::FULL);
    let water = Block::WATER.default_state().selection();
    assert!(!water.can_collide && water.can_collide_liquid);
    assert!(!BlockState::AIR.selection().can_collide);
    let slab = Block::STONE_SLAB.with_meta(8).selection();
    assert_eq!(slab.bounds.min_y, 0.5);
    assert!(Block::OAK_STAIRS.default_state().selection().multi_pass_ray);
    // Cactus collides inset but its ray-test bounds are the full cube.
    assert_eq!(Block::CACTUS.default_state().selection().bounds, Aabb::FULL);
    assert_eq!(
        Block::FENCE.default_state().selection().dependency,
        ShapeDependency::Neighbours(NeighbourShape::Fence)
    );
}
