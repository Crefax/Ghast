package ghast.oracle.v1_12_2;

import ghast.oracle.Json;
import ghast.oracle.Scenario;
import net.minecraft.block.Block;
import net.minecraft.block.BlockState;
import net.minecraft.block.material.Material;
import net.minecraft.util.Identifier;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.Box;

import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;

import static ghast.oracle.v1_12_2.Blocks.*;

/**
 * Dumps what the real collision code ({@code BlockState.appendCollisionBoxes}, as called by the world's
 * collision query) returns for every raw block id/meta, plus neighbour configurations for blocks whose
 * shape depends on their neighbours. Schema: oracle/README.md (1.12.2 section).
 *
 * <p>1.12 blocks have no mutable bounds (shapes come from the state and, for some blocks, the
 * neighbours), so there is no "settled" / "constructor" distinction as in 1.8.9.
 */
final class CollisionDump {
    /** Reference position. x = z = 0 keeps the horizontal coordinates exact. */
    static final int RX = 0, RY = 64, RZ = 0;

    private CollisionDump() {}

    private static Harness.Run run;

    /** Yarn field names of the vanilla materials, for the identity checks the movement code makes. */
    private static final Map<Material, String> MATERIAL_NAMES = new IdentityHashMap<>();

    static String materialName(Material m) {
        if (MATERIAL_NAMES.isEmpty()) {
            for (Field f : Material.class.getDeclaredFields()) {
                if (Modifier.isStatic(f.getModifiers()) && f.getType() == Material.class) {
                    try {
                        MATERIAL_NAMES.putIfAbsent((Material) f.get(null), f.getName());
                    } catch (IllegalAccessException e) {
                        throw new IllegalStateException(e);
                    }
                }
            }
        }
        String n = MATERIAL_NAMES.get(m);
        return n == null ? "?" : n.toLowerCase();
    }

    static List<String> lines() {
        Scenario sc = new Scenario("collision_dump").spawn(40.5, 100, 40.5, 0, 0);
        sc.fill(-2, 60, -2, 2, 60, 2, AIR); // loads the chunks around the reference position
        run = new Harness.Run(sc);
        List<String> out = new ArrayList<>();

        // Block-level properties.
        for (int id = 0; id < 256; id++) {
            Block b = registered(id);
            if (b == null) {
                continue;
            }
            BlockState def = b.getDefaultState();
            Material m = def.getMaterial();
            Identifier name = Block.REGISTRY.getIdentifier(b);
            Json j = new Json().beginObject().field("type", "block").field("id", id).field("name", name.toString());
            j.fieldF32("slipperiness", b.slipperiness);
            j.field("has_collision", b.hasCollision());
            j.field("material", materialName(m));
            j.field("material_blocks_movement", m.blocksMovement()).field("material_fluid", m.isFluid())
                    .field("material_solid", m.isSolid()).field("material_opaque", m.isOpaque())
                    .field("material_replaceable", m.isReplaceable());
            j.field("climbable", b == net.minecraft.block.Blocks.LADDER || b == net.minecraft.block.Blocks.VINE);
            out.add(j.endObject().toString());
        }

        // Every id/meta alone (air neighbours), with the per-state flags.
        for (int id = 0; id < 256; id++) {
            if (registered(id) == null) {
                continue;
            }
            for (int meta = 0; meta < 16; meta++) {
                out.add(boxes(id, meta, "alone", List.of(), true));
            }
        }

        int[][] sides = {{0, 0, -1}, {1, 0, 0}, {0, 0, 1}, {-1, 0, 0}}; // north, east, south, west
        // Neighbour-dependent shapes: fences, panes, bars, walls (1.12 connects by the neighbour's face shape).
        int[] connecting = {FENCE, NETHER_BRICK_FENCE, SPRUCE_FENCE, BIRCH_FENCE, ACACIA_FENCE, GLASS_PANE,
                STAINED_GLASS_PANE, IRON_BARS, COBBLESTONE_WALL};
        int[][] sideKinds = {{STONE, 0}, {GLASS, 0}, {STAINED_GLASS, 3}, {FENCE_GATE, 0}, {FENCE_GATE, 1},
                {FENCE_GATE, 4}, {SPRUCE_FENCE_GATE, 0}, {ACACIA_FENCE_GATE, 1}, {FENCE, 0}, {NETHER_BRICK_FENCE, 0},
                {SPRUCE_FENCE, 0}, {COBBLESTONE_WALL, 0}, {GLASS_PANE, 0}, {STAINED_GLASS_PANE, 0}, {IRON_BARS, 0},
                {LEAVES, 0}, {STONE_SLAB, 0}, {STONE_SLAB, 8}, {DOUBLE_STONE_SLAB, 0}, {PURPUR_SLAB, 0},
                {GLOWSTONE, 0}, {CHEST, 2}, {OAK_STAIRS, 0}, {OAK_STAIRS, 2}, {OAK_STAIRS, 4}, {ICE, 0}, {SLIME, 0},
                {SOUL_SAND, 0}, {PISTON, 0}, {PISTON, 3}, {BARRIER, 0}, {PACKED_ICE, 0}, {WHITE_SHULKER_BOX, 1},
                {CONCRETE, 0}, {OBSERVER, 0}, {TRAPDOOR, 0}, {TRAPDOOR, 4}, {MAGMA, 0}, {GRASS_PATH, 0},
                {FARMLAND, 0}, {SNOW, 0}, {HOPPER, 0}, {END_ROD, 2}, {CHORUS_PLANT, 0}, {PURPUR_BLOCK, 0},
                {WHITE_GLAZED_TERRACOTTA, 0}, {CACTUS, 0}, {ANVIL, 0}, {ENCHANTING_TABLE, 0}, {LADDER, 2},
                {BED, 0}, {OAK_DOOR, 0}, {CARPET, 0}, {SNOW_LAYER, 0}, {HAY_BLOCK, 0}};
        for (int id : connecting) {
            int metaCount = id == COBBLESTONE_WALL || id == STAINED_GLASS_PANE ? 2 : 1;
            for (int meta = 0; meta < metaCount; meta++) {
                // All combinations of stone / air on the four sides.
                for (int mask = 1; mask < 16; mask++) {
                    List<int[]> n = new ArrayList<>();
                    for (int s = 0; s < 4; s++) {
                        if ((mask & (1 << s)) != 0) {
                            n.add(new int[] {sides[s][0], sides[s][1], sides[s][2], STONE, 0});
                        }
                    }
                    out.add(boxes(id, meta, "stone_sides", n, false));
                }
                // Each neighbour kind on the south side, and the same block on two opposite sides.
                for (int[] k : sideKinds) {
                    out.add(boxes(id, meta, "one_side", List.of(new int[] {0, 0, 1, k[0], k[1]}), false));
                }
                out.add(boxes(id, meta, "self_south", List.of(new int[] {0, 0, 1, id, meta}), false));
                out.add(boxes(id, meta, "self_north_south",
                        List.of(new int[] {0, 0, -1, id, meta}, new int[] {0, 0, 1, id, meta}), false));
                out.add(boxes(id, meta, "self_all", List.of(new int[] {0, 0, -1, id, meta}, new int[] {1, 0, 0, id, meta},
                        new int[] {0, 0, 1, id, meta}, new int[] {-1, 0, 0, id, meta}), false));
                out.add(boxes(id, meta, "self_north_south_stone_above", List.of(new int[] {0, 0, -1, id, meta},
                        new int[] {0, 0, 1, id, meta}, new int[] {0, 1, 0, STONE, 0}), false));
                // Walls: the post depends on what is above.
                out.add(boxes(id, meta, "self_east_west_wall_above", List.of(new int[] {1, 0, 0, id, meta},
                        new int[] {-1, 0, 0, id, meta}, new int[] {0, 1, 0, COBBLESTONE_WALL, 0}), false));
            }
        }
        // Stairs: every target meta with a stair of every meta on each side (inner / outer corners).
        for (int id : new int[] {OAK_STAIRS, STONE_STAIRS, PURPUR_STAIRS}) {
            for (int meta = 0; meta < 8; meta++) {
                for (int s = 0; s < 4; s++) {
                    for (int nm = 0; nm < 8; nm++) {
                        out.add(boxes(id, meta, "stair_neighbour",
                                List.of(new int[] {sides[s][0], 0, sides[s][2], OAK_STAIRS, nm}), false));
                    }
                }
            }
        }
        // Doors: the two halves together.
        for (int id : new int[] {OAK_DOOR, IRON_DOOR, SPRUCE_DOOR}) {
            for (int lower = 0; lower < 8; lower++) {
                for (int upper = 8; upper < 12; upper++) {
                    out.add(boxes(id, lower, "door_lower_with_upper", List.of(new int[] {0, 1, 0, id, upper}), false));
                    out.add(boxes(id, upper, "door_upper_with_lower", List.of(new int[] {0, -1, 0, id, lower}), false));
                }
            }
        }
        // Chests: a second chest on each side (1.9+ single chests shrink towards a neighbour chest).
        for (int id : new int[] {CHEST, TRAPPED_CHEST}) {
            for (int meta = 2; meta < 6; meta++) {
                for (int s = 0; s < 4; s++) {
                    for (int nm = 2; nm < 6; nm++) {
                        out.add(boxes(id, meta, "chest_neighbour",
                                List.of(new int[] {sides[s][0], 0, sides[s][2], id, nm}), false));
                    }
                }
                int otherKind = id == CHEST ? TRAPPED_CHEST : CHEST;
                out.add(boxes(id, meta, "chest_other_kind", List.of(new int[] {0, 0, 1, otherKind, meta}), false));
            }
        }
        // Fence gates between walls (gate "in wall" state).
        for (int meta = 0; meta < 8; meta++) {
            out.add(boxes(FENCE_GATE, meta, "gate_between_walls", List.of(new int[] {1, 0, 0, COBBLESTONE_WALL, 0},
                    new int[] {-1, 0, 0, COBBLESTONE_WALL, 0}), false));
            out.add(boxes(FENCE_GATE, meta, "gate_between_walls_ns", List.of(new int[] {0, 0, 1, COBBLESTONE_WALL, 0},
                    new int[] {0, 0, -1, COBBLESTONE_WALL, 0}), false));
        }
        // Chorus plants connect to chorus plants / flowers on all six sides and to end stone below.
        int[][] six = {{0, 0, -1}, {1, 0, 0}, {0, 0, 1}, {-1, 0, 0}, {0, 1, 0}, {0, -1, 0}};
        for (int mask = 0; mask < 64; mask++) {
            List<int[]> n = new ArrayList<>();
            for (int s = 0; s < 6; s++) {
                if ((mask & (1 << s)) != 0) {
                    n.add(new int[] {six[s][0], six[s][1], six[s][2], CHORUS_PLANT, 0});
                }
            }
            out.add(boxes(CHORUS_PLANT, 0, "chorus_sides", n, false));
        }
        for (int[] k : new int[][] {{CHORUS_FLOWER, 0}, {END_STONE, 0}, {STONE, 0}}) {
            for (int s = 0; s < 6; s++) {
                out.add(boxes(CHORUS_PLANT, 0, "chorus_one_side",
                        List.of(new int[] {six[s][0], six[s][1], six[s][2], k[0], k[1]}), false));
            }
        }
        return out;
    }

    static Block registered(int id) {
        Block b = Block.getById(id);
        if (id != 0 && Block.getIdByBlock(b) != id) {
            return null; // unknown ids fall back to air
        }
        return b;
    }

    private static String boxes(int id, int meta, String config, List<int[]> neighbours, boolean flags) {
        int raw = id << 4 | meta;
        for (int[] n : neighbours) {
            run.setRaw(RX + n[0], RY + n[1], RZ + n[2], n[3] << 4 | n[4]);
        }
        run.setRaw(RX, RY, RZ, raw);
        BlockPos pos = new BlockPos(RX, RY, RZ);
        BlockState state = run.world.getBlockState(pos);
        BlockState decoded = Block.BLOCK_STATES.fromId(raw);

        Json j = new Json().beginObject().field("type", "boxes").field("id", id).field("meta", meta);
        j.field("config", config);
        j.key("neighbours").beginArray();
        for (int[] n : neighbours) {
            j.beginArray().value(n[0]).value(n[1]).value(n[2]).value(n[3]).value(n[4]).endArray();
        }
        j.endArray();
        j.field("state", decoded == null ? null : decoded.toString());
        if (decoded != null) {
            j.field("actual_state", state.getBlockState(run.world, pos).toString());
        }

        List<Box> list = collide(run, pos, state);
        j.key("boxes").beginArray();
        boolean exact = true;
        for (Box b : list) {
            double[] rel = {b.minX - RX, b.minY - RY, b.minZ - RZ, b.maxX - RX, b.maxY - RY, b.maxZ - RZ};
            exact &= rel[0] + RX == b.minX && rel[1] + RY == b.minY && rel[2] + RZ == b.minZ
                    && rel[3] + RX == b.maxX && rel[4] + RY == b.maxY && rel[5] + RZ == b.maxZ;
            j.beginArray();
            for (double d : rel) {
                j.f64(d);
            }
            j.endArray();
        }
        j.endArray();
        j.field("exact", exact);
        if (flags && decoded != null) {
            // Per-state flags (1.9+ moved these from the block to the state). Yarn 1.12.2 names:
            // method_11730 = full cube, method_11734 = normal cube (pushOutOfBlocks), method_11733 =
            // block normal cube, method_11739 = top solid, isFullBoundsCubeForCulling = opaque cube,
            // method_13763 = causes suffocation.
            j.field("full_cube", state.method_11730()).field("normal_cube", state.method_11734())
                    .field("block_normal_cube", state.method_11733()).field("top_solid", state.method_11739())
                    .field("opaque_cube", state.isFullBoundsCubeForCulling()).field("full_block", state.isFullBlock())
                    .field("causes_suffocation", state.method_13763());
            Material m = state.getMaterial();
            j.field("material", materialName(m));
        }

        run.setRaw(RX, RY, RZ, 0);
        for (int[] n : neighbours) {
            run.setRaw(RX + n[0], RY + n[1], RZ + n[2], 0);
        }
        return j.endObject().toString();
    }

    private static List<Box> collide(Harness.Run r, BlockPos pos, BlockState state) {
        List<Box> list = new ArrayList<>();
        Box mask = new Box(pos.getX() - 2, pos.getY() - 2, pos.getZ() - 2, pos.getX() + 3, pos.getY() + 3, pos.getZ() + 3);
        // What World's collision query calls: isActualState = false, so neighbour-dependent blocks
        // compute their actual state themselves.
        state.appendCollisionBoxes(r.world, pos, mask, list, r.player, false);
        return list;
    }

    // ---- far reference positions -------------------------------------------------------------------

    /**
     * Far reference positions. The last one sits just past 2^24 on both axes, where floats can no longer
     * hold every integer.
     */
    static final int[][] FAR_POSITIONS = {{29_999_000, 200, -29_999_000}, {-1_234_567, 5, 7_654_321},
            {-16_777_217, 100, -16_777_219}};

    /**
     * Every registered id x meta alone at each far position, in absolute coordinates, with a flag telling
     * whether the result equals the shape from the origin dump added to the position in double precision.
     */
    static List<String> farLines() {
        Harness.Run origin = runAround(RX, RY, RZ);
        List<double[][]> local = new ArrayList<>();
        List<int[]> keys = new ArrayList<>();
        for (int id = 0; id < 256; id++) {
            if (registered(id) == null) {
                continue;
            }
            for (int meta = 0; meta < 16; meta++) {
                List<Box> boxes = query(origin, RX, RY, RZ, id << 4 | meta);
                double[][] rel = new double[boxes.size()][];
                for (int k = 0; k < boxes.size(); k++) {
                    Box b = boxes.get(k);
                    rel[k] = new double[] {b.minX - RX, b.minY - RY, b.minZ - RZ, b.maxX - RX, b.maxY - RY, b.maxZ - RZ};
                }
                local.add(rel);
                keys.add(new int[] {id, meta});
            }
        }
        List<String> out = new ArrayList<>();
        for (int[] p : FAR_POSITIONS) {
            Harness.Run far = runAround(p[0], p[1], p[2]);
            for (int k = 0; k < keys.size(); k++) {
                int id = keys.get(k)[0], meta = keys.get(k)[1];
                List<Box> boxes = query(far, p[0], p[1], p[2], id << 4 | meta);
                double[][] rel = local.get(k);
                boolean same = rel.length == boxes.size();
                for (int b = 0; same && b < boxes.size(); b++) {
                    Box x = boxes.get(b);
                    double[] abs = {x.minX, x.minY, x.minZ, x.maxX, x.maxY, x.maxZ};
                    for (int c = 0; c < 6; c++) {
                        same &= Double.doubleToRawLongBits(abs[c]) == Double.doubleToRawLongBits(rel[b][c] + p[c % 3]);
                    }
                }
                BlockState decoded = Block.BLOCK_STATES.fromId(id << 4 | meta);
                Json j = new Json().beginObject().field("type", "boxes_abs").field("id", id).field("meta", meta);
                j.field("config", "alone");
                j.key("pos").beginArray().value(p[0]).value(p[1]).value(p[2]).endArray();
                j.field("state", decoded == null ? null : decoded.toString());
                j.key("boxes").beginArray();
                for (Box x : boxes) {
                    j.beginArray().f64(x.minX).f64(x.minY).f64(x.minZ).f64(x.maxX).f64(x.maxY).f64(x.maxZ).endArray();
                }
                j.endArray();
                j.field("same_as_local", same);
                out.add(j.endObject().toString());
            }
        }
        return out;
    }

    private static Harness.Run runAround(int x, int y, int z) {
        Scenario sc = new Scenario("collision_dump_" + x + "_" + y + "_" + z).spawn(x + 40.5, Math.min(250, y + 40), z + 40.5, 0, 0);
        sc.fill(x - 2, y, z - 2, x + 2, y, z + 2, AIR); // loads the chunks around the reference position
        return new Harness.Run(sc);
    }

    /** One isolated block at (x, y, z): place, collect, clear. */
    private static List<Box> query(Harness.Run r, int x, int y, int z, int raw) {
        r.setRaw(x, y, z, raw);
        BlockPos pos = new BlockPos(x, y, z);
        List<Box> list = collide(r, pos, r.world.getBlockState(pos));
        r.setRaw(x, y, z, 0);
        return list;
    }
}
