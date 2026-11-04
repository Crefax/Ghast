package ghast.oracle.v1_8_9;

import ghast.oracle.Json;
import ghast.oracle.Scenario;
import net.minecraft.block.Block;
import net.minecraft.block.BlockState;
import net.minecraft.block.material.Material;
import net.minecraft.util.Identifier;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.Box;

import java.util.ArrayList;
import java.util.List;

import static ghast.oracle.v1_8_9.Blocks.*;

/**
 * Dumps what the real collision code ({@code Block.appendCollisionBoxes}, as called by the world's
 * collision query) returns for every raw block id/meta, plus neighbour configurations for blocks
 * whose shape depends on their neighbours. Schema: oracle/README.md.
 */
final class CollisionDump {
    /** Reference position. x = z = 0 keeps the horizontal coordinates exact; see README for y. */
    static final int RX = 0, RY = 64, RZ = 0;

    private CollisionDump() {}

    private static Harness.Run run;

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
            Material m = b.getMaterial();
            Identifier name = Block.REGISTRY.getIdentifier(b);
            Json j = new Json().beginObject().field("type", "block").field("id", id).field("name", name.toString());
            j.fieldF32("slipperiness", b.slipperiness);
            j.field("full_cube", b.isFullCube()).field("normal_block", b.isNormalBlock()).field("full_block", b.isFullBlock());
            j.field("render_as_normal_block", b.renderAsNormalBlock()).field("has_collision", b.hasCollision());
            j.field("material_blocks_movement", m.blocksMovement()).field("material_fluid", m.isFluid())
                    .field("material_solid", m.isSolid()).field("material_opaque", m.isOpaque())
                    .field("material_replaceable", m.isReplaceable());
            j.field("climbable", b == net.minecraft.block.Blocks.LADDER || b == net.minecraft.block.Blocks.VINE);
            out.add(j.endObject().toString());
        }

        // Every id/meta alone (air neighbours).
        for (int id = 0; id < 256; id++) {
            if (registered(id) == null) {
                continue;
            }
            for (int meta = 0; meta < 16; meta++) {
                out.add(boxes(id, meta, "alone", List.of(), true));
            }
        }

        // Neighbour-dependent shapes.
        int[] connecting = {FENCE, NETHER_BRICK_FENCE, SPRUCE_FENCE, ACACIA_FENCE, GLASS_PANE, STAINED_GLASS_PANE,
                IRON_BARS, COBBLESTONE_WALL};
        int[][] sideKinds = {{STONE, 0}, {GLASS, 0}, {STAINED_GLASS, 3}, {FENCE_GATE, 0}, {FENCE_GATE, 1},
                {SPRUCE_FENCE_GATE, 0}, {FENCE, 0}, {NETHER_BRICK_FENCE, 0}, {SPRUCE_FENCE, 0}, {COBBLESTONE_WALL, 0},
                {GLASS_PANE, 0}, {STAINED_GLASS_PANE, 0}, {IRON_BARS, 0}, {LEAVES, 0}, {STONE_SLAB, 0}, {GLOWSTONE, 0},
                {CHEST, 2}, {OAK_STAIRS, 0}, {ICE, 0}, {SLIME, 0}, {SOUL_SAND, 0}, {PISTON, 0}};
        int[][] sides = {{0, 0, -1}, {1, 0, 0}, {0, 0, 1}, {-1, 0, 0}}; // north, east, south, west
        for (int id : connecting) {
            int metaCount = id == COBBLESTONE_WALL || id == STAINED_GLASS_PANE ? 2 : 1;
            for (int meta = 0; meta < metaCount; meta++) {
                // All 16 combinations of stone / air on the four sides.
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
            }
        }
        // Stairs: every target meta with a stair of every meta on each side (inner / outer corners).
        for (int id : new int[] {OAK_STAIRS, STONE_STAIRS}) {
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
        for (int id : new int[] {OAK_DOOR, IRON_DOOR}) {
            for (int lower = 0; lower < 8; lower++) {
                for (int upper = 8; upper < 12; upper++) {
                    out.add(boxes(id, lower, "door_lower_with_upper", List.of(new int[] {0, 1, 0, id, upper}), false));
                    out.add(boxes(id, upper, "door_upper_with_lower", List.of(new int[] {0, -1, 0, id, lower}), false));
                }
            }
        }
        // History-dependent blocks, settled: the block's own shape update runs right before collecting.
        for (int id : HISTORY_DEPENDENT) {
            for (int meta = 0; meta < 16; meta++) {
                out.add(boxes(id, meta, "settled", List.of(), false, true));
            }
        }
        for (int id : new int[] {CHEST, TRAPPED_CHEST}) {
            for (int meta = 2; meta < 6; meta++) {
                for (int s = 0; s < 4; s++) {
                    for (int nm = 2; nm < 6; nm++) {
                        out.add(boxes(id, meta, "settled", List.of(new int[] {sides[s][0], 0, sides[s][2], id, nm}), false, true));
                    }
                }
                // A chest of the other kind does not connect.
                int otherKind = id == CHEST ? TRAPPED_CHEST : CHEST;
                out.add(boxes(id, meta, "settled", List.of(new int[] {0, 0, 1, otherKind, meta}), false, true));
            }
        }
        // Chests: a second chest on each side (constructor bounds, no shape update).
        for (int id : new int[] {CHEST, TRAPPED_CHEST}) {
            for (int meta = 2; meta < 6; meta++) {
                for (int s = 0; s < 4; s++) {
                    out.add(boxes(id, meta, "chest_neighbour",
                            List.of(new int[] {sides[s][0], 0, sides[s][2], id, meta}), false));
                }
            }
        }
        // Fence gates and walls between walls (gate "in wall" state, wall post rules).
        for (int meta = 0; meta < 8; meta++) {
            out.add(boxes(FENCE_GATE, meta, "gate_between_walls", List.of(new int[] {1, 0, 0, COBBLESTONE_WALL, 0},
                    new int[] {-1, 0, 0, COBBLESTONE_WALL, 0}), false));
        }
        return out;
    }

    private static Block registered(int id) {
        Block b = Block.getById(id);
        if (id != 0 && Block.getIdByBlock(b) != id) {
            return null; // unknown ids fall back to air
        }
        return b;
    }

    /**
     * Places the target (and neighbours), runs the collision query of the target block, then clears
     * everything again. {@code checkHistory} also tests whether the result depends on bounds left behind
     * by earlier shape updates of the same block (the mutable bounds fields of 1.8 blocks).
     */
    /** Blocks whose collision box is whatever bounds were last computed (see README, history-dependent shapes). */
    static final int[] HISTORY_DEPENDENT = {CHEST, ANVIL, TRAPPED_CHEST};

    private static boolean historyDependent(int id) {
        for (int h : HISTORY_DEPENDENT) {
            if (h == id) {
                return true;
            }
        }
        return false;
    }

    private static String boxes(int id, int meta, String config, List<int[]> neighbours, boolean checkHistory) {
        return boxes(id, meta, config, neighbours, checkHistory, false);
    }

    /**
     * @param settle call the block's own bounds-from-world update ({@code setBoundingBox(world, pos)})
     *               right before collecting, i.e. the shape as it is after a shape update of this block
     */
    private static String boxes(int id, int meta, String config, List<int[]> neighbours, boolean checkHistory,
                                boolean settle) {
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

        Harness.restoreBlockBounds();
        if (settle) {
            state.getBlock().setBoundingBox(run.world, pos);
        }
        List<Box> list = collide(pos, state);
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
        if (historyDependent(id)) {
            // "constructor": bounds as left by bootstrap (no shape update); "settled": after this block's
            // own shape update at its position. ghast models "settled" (README, history-dependent shapes).
            j.field("shape_source", settle ? "settled" : "constructor");
        }

        if (checkHistory && decoded != null) {
            // Shape updates of the same block at another position (every meta, alone and with a copy of
            // itself on one side, as a raytrace at a double chest would do), then re-query the target.
            String base = render(list);
            List<Integer> dependsOn = new ArrayList<>();
            BlockPos other = new BlockPos(RX + 1, RY + 2, RZ + 1);
            int[][] side = {null, {0, -1}, {1, 0}, {0, 1}, {-1, 0}};
            for (int m2 = 0; m2 < 16; m2++) {
                if (Block.BLOCK_STATES.fromId(id << 4 | m2) == null) {
                    continue;
                }
                boolean differs = false;
                for (int[] sd : side) {
                    run.setRaw(other.getX(), other.getY(), other.getZ(), id << 4 | m2);
                    if (sd != null) {
                        run.setRaw(other.getX() + sd[0], other.getY(), other.getZ() + sd[1], id << 4 | m2);
                    }
                    state.getBlock().setBoundingBox(run.world, other);
                    differs |= !render(collide(pos, state)).equals(base);
                    Harness.restoreBlockBounds();
                    run.setRaw(other.getX(), other.getY(), other.getZ(), 0);
                    if (sd != null) {
                        run.setRaw(other.getX() + sd[0], other.getY(), other.getZ() + sd[1], 0);
                    }
                }
                if (differs) {
                    dependsOn.add(m2);
                }
            }
            j.field("history_dependent", !dependsOn.isEmpty());
            if (!dependsOn.isEmpty()) {
                j.key("history_metas").beginArray();
                for (int m2 : dependsOn) {
                    j.value(m2);
                }
                j.endArray();
            }
        }
        Harness.restoreBlockBounds();

        run.setRaw(RX, RY, RZ, 0);
        for (int[] n : neighbours) {
            run.setRaw(RX + n[0], RY + n[1], RZ + n[2], 0);
        }
        return j.endObject().toString();
    }

    private static List<Box> collide(BlockPos pos, BlockState state) {
        List<Box> list = new ArrayList<>();
        Box mask = new Box(pos.getX() - 2, pos.getY() - 2, pos.getZ() - 2, pos.getX() + 3, pos.getY() + 3, pos.getZ() + 3);
        state.getBlock().appendCollisionBoxes(run.world, pos, state, mask, list, run.player);
        return list;
    }

    private static String render(List<Box> list) {
        StringBuilder sb = new StringBuilder();
        for (Box b : list) {
            sb.append(Double.doubleToRawLongBits(b.minX)).append(',').append(Double.doubleToRawLongBits(b.minY)).append(',')
                    .append(Double.doubleToRawLongBits(b.minZ)).append(',').append(Double.doubleToRawLongBits(b.maxX)).append(',')
                    .append(Double.doubleToRawLongBits(b.maxY)).append(',').append(Double.doubleToRawLongBits(b.maxZ)).append(';');
        }
        return sb.toString();
    }

    // ---- far reference positions -------------------------------------------------------------------

    /**
     * Far reference positions. Some blocks build their collision box in float (or otherwise
     * position-dependent) arithmetic, so the absolute box is not always (double) pos + local shape.
     * The last one sits just past 2^24 on both axes, where floats can no longer hold every integer.
     */
    static final int[][] FAR_POSITIONS = {{29_999_000, 200, -29_999_000}, {-1_234_567, 5, 7_654_321},
            {-16_777_217, 100, -16_777_219}};

    /**
     * Every registered id x meta alone (plus "settled" for the history-dependent blocks) at each far
     * position, in absolute coordinates, with a flag telling whether the result equals the shape from the
     * origin dump added to the position in double precision.
     */
    static List<String> farLines() {
        // Local shapes at the origin reference position (same as collision_boxes.jsonl.gz "alone"/"settled").
        Harness.Run origin = runAround(RX, RY, RZ);
        List<double[][]> local = new ArrayList<>();
        List<int[]> keys = new ArrayList<>();
        for (int id = 0; id < 256; id++) {
            if (registered(id) == null) {
                continue;
            }
            for (int pass = 0; pass < (historyDependent(id) ? 2 : 1); pass++) {
                for (int meta = 0; meta < 16; meta++) {
                    List<Box> boxes = query(origin, RX, RY, RZ, id << 4 | meta, pass == 1);
                    double[][] rel = new double[boxes.size()][];
                    for (int k = 0; k < boxes.size(); k++) {
                        Box b = boxes.get(k);
                        rel[k] = new double[] {b.minX - RX, b.minY - RY, b.minZ - RZ, b.maxX - RX, b.maxY - RY, b.maxZ - RZ};
                    }
                    local.add(rel);
                    keys.add(new int[] {id, meta, pass});
                }
            }
        }
        List<String> out = new ArrayList<>();
        for (int[] p : FAR_POSITIONS) {
            Harness.Run far = runAround(p[0], p[1], p[2]);
            for (int k = 0; k < keys.size(); k++) {
                int id = keys.get(k)[0], meta = keys.get(k)[1];
                boolean settle = keys.get(k)[2] == 1;
                List<Box> boxes = query(far, p[0], p[1], p[2], id << 4 | meta, settle);
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
                j.field("config", settle ? "settled" : "alone");
                j.key("pos").beginArray().value(p[0]).value(p[1]).value(p[2]).endArray();
                j.field("state", decoded == null ? null : decoded.toString());
                j.key("boxes").beginArray();
                for (Box x : boxes) {
                    j.beginArray().f64(x.minX).f64(x.minY).f64(x.minZ).f64(x.maxX).f64(x.maxY).f64(x.maxZ).endArray();
                }
                j.endArray();
                j.field("same_as_local", same);
                if (historyDependent(id)) {
                    j.field("shape_source", settle ? "settled" : "constructor");
                }
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

    /** One isolated block at (x, y, z): place, reset bounds, optionally settle, collect, clear. */
    private static List<Box> query(Harness.Run r, int x, int y, int z, int raw, boolean settle) {
        r.setRaw(x, y, z, raw);
        BlockPos pos = new BlockPos(x, y, z);
        BlockState state = r.world.getBlockState(pos);
        Harness.restoreBlockBounds();
        if (settle) {
            state.getBlock().setBoundingBox(r.world, pos);
        }
        List<Box> list = new ArrayList<>();
        Box mask = new Box(x - 2, y - 2, z - 2, x + 3, y + 3, z + 3);
        state.getBlock().appendCollisionBoxes(r.world, pos, state, mask, list, r.player);
        Harness.restoreBlockBounds();
        r.setRaw(x, y, z, 0);
        return list;
    }
}
