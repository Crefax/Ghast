package ghast.oracle.v1_12_2;

import ghast.oracle.Json;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import net.minecraft.block.Block;
import net.minecraft.block.BlockState;
import net.minecraft.util.hit.BlockHitResult;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.Vec3d;

import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;

import static ghast.oracle.v1_12_2.Blocks.*;

/**
 * 1.12.2 crosshair data: the bounds a block's ray test uses (the state's outline box, Legacy Yarn
 * {@code BlockState.getCollisionBox(view, pos)}, which neighbour-dependent blocks compute from their
 * actual state), whether rays consider the block at all ({@code canCollide}), and raytrace vectors
 * through the real {@code Entity.rayTrace} / {@code World.rayTrace}.
 * Schema: oracle/README.md, "selection_boxes.jsonl.gz" and "raytrace.jsonl.gz".
 */
final class SelectionDump {
    static final int RX = 0, RY = 64, RZ = 0;

    private SelectionDump() {}

    private static Harness.Run run;

    static List<String> lines() {
        Scenario sc = new Scenario("selection_dump").spawn(40.5, 100, 40.5, 0, 0);
        sc.fill(-2, 60, -2, 2, 60, 2, AIR);
        run = new Harness.Run(sc);
        List<String> out = new ArrayList<>();
        for (int id = 0; id < 256; id++) {
            Block b = Block.getById(id);
            if (id != 0 && Block.getIdByBlock(b) != id) {
                continue;
            }
            for (int meta = 0; meta < 16; meta++) {
                out.add(selection(id, meta, "alone", List.of()));
            }
        }
        int[] connecting = {FENCE, NETHER_BRICK_FENCE, SPRUCE_FENCE, ACACIA_FENCE, GLASS_PANE, STAINED_GLASS_PANE,
                IRON_BARS, COBBLESTONE_WALL, CHORUS_PLANT};
        int[][] sides = {{0, 0, -1}, {1, 0, 0}, {0, 0, 1}, {-1, 0, 0}};
        for (int id : connecting) {
            for (int mask = 1; mask < 16; mask++) {
                List<int[]> n = new ArrayList<>();
                for (int s = 0; s < 4; s++) {
                    if ((mask & (1 << s)) != 0) {
                        n.add(new int[] {sides[s][0], 0, sides[s][2], STONE, 0});
                    }
                }
                out.add(selection(id, 0, "stone_sides", n));
            }
            out.add(selection(id, 0, "self_all", List.of(new int[] {0, 0, -1, id, 0}, new int[] {1, 0, 0, id, 0},
                    new int[] {0, 0, 1, id, 0}, new int[] {-1, 0, 0, id, 0})));
            out.add(selection(id, 0, "self_north_south_stone_above", List.of(new int[] {0, 0, -1, id, 0},
                    new int[] {0, 0, 1, id, 0}, new int[] {0, 1, 0, STONE, 0})));
        }
        for (int id : new int[] {OAK_STAIRS, STONE_STAIRS, PURPUR_STAIRS}) {
            for (int meta = 0; meta < 8; meta++) {
                for (int s = 0; s < 4; s++) {
                    for (int nm = 0; nm < 8; nm++) {
                        out.add(selection(id, meta, "stair_neighbour",
                                List.of(new int[] {sides[s][0], 0, sides[s][2], OAK_STAIRS, nm})));
                    }
                }
            }
        }
        for (int id : new int[] {OAK_DOOR, IRON_DOOR}) {
            for (int lower = 0; lower < 8; lower++) {
                for (int upper = 8; upper < 12; upper++) {
                    out.add(selection(id, lower, "door_lower_with_upper", List.of(new int[] {0, 1, 0, id, upper})));
                    out.add(selection(id, upper, "door_upper_with_lower", List.of(new int[] {0, -1, 0, id, lower})));
                }
            }
        }
        for (int id : new int[] {CHEST, TRAPPED_CHEST}) {
            for (int meta = 2; meta < 6; meta++) {
                for (int s = 0; s < 4; s++) {
                    out.add(selection(id, meta, "chest_neighbour", List.of(new int[] {sides[s][0], 0, sides[s][2], id, meta})));
                }
            }
        }
        // Tripwire reads the block below (suspended); its bounds depend on it.
        for (int meta = 0; meta < 16; meta++) {
            out.add(selection(TRIPWIRE, meta, "stone_below", List.of(new int[] {0, -1, 0, STONE, 0})));
        }
        return out;
    }

    private static String selection(int id, int meta, String config, List<int[]> neighbours) {
        for (int[] n : neighbours) {
            run.setRaw(RX + n[0], RY + n[1], RZ + n[2], n[3] << 4 | n[4]);
        }
        run.setRaw(RX, RY, RZ, id << 4 | meta);
        BlockPos pos = new BlockPos(RX, RY, RZ);
        BlockState state = run.world.getBlockState(pos);
        BlockState decoded = Block.BLOCK_STATES.fromId(id << 4 | meta);
        Block block = state.getBlock();
        net.minecraft.util.math.Box box = state.getCollisionBox(run.world, pos);
        double[] b = {box.minX, box.minY, box.minZ, box.maxX, box.maxY, box.maxZ};
        Json j = new Json().beginObject().field("type", "selection").field("id", id).field("meta", meta);
        j.field("config", config);
        j.key("neighbours").beginArray();
        for (int[] n : neighbours) {
            j.beginArray().value(n[0]).value(n[1]).value(n[2]).value(n[3]).value(n[4]).endArray();
        }
        j.endArray();
        j.field("state", decoded == null ? null : decoded.toString());
        j.key("bounds").beginArray();
        for (double d : b) {
            j.f64(d);
        }
        j.endArray();
        j.field("can_collide", block.canCollide(state, false));
        j.field("can_collide_liquid", block.canCollide(state, true));
        j.field("has_collision_box", state.method_11726(run.world, pos) != null);
        run.setRaw(RX, RY, RZ, 0);
        for (int[] n : neighbours) {
            run.setRaw(RX + n[0], RY + n[1], RZ + n[2], 0);
        }
        return j.endObject().toString();
    }

    /**
     * Raytrace vectors: worlds from the fuzz terrain generator, then random viewpoints. For each:
     * the real crosshair pick ({@code Entity.rayTrace(reach, 1.0F)}, which calls
     * {@code World.rayTrace(eyes, end, false, false, true)}) and {@code World.rayTrace(eyes, end, false,
     * false, false)}.
     */
    static void rays(long seed, int worlds, int raysPerWorld, Consumer<String> sink) {
        for (int w = 0; w < worlds; w++) {
            Scenario sc = Fuzz.one(seed, w, 1);
            Scenario world = new Scenario("ray_s" + seed + "_" + w);
            world.description = sc.description;
            world.outside = sc.outside;
            world.fills.addAll(sc.fills);
            world.spawn(sc.x, sc.y, sc.z, 0, 0);
            Harness.Run r = new Harness.Run(world);
            sink.accept(Recorder.header(world, r, Main.VERSION));
            Rng rng = new Rng(seed * 31 + w ^ 0x7261797472616365L);
            for (int k = 0; k < raysPerWorld; k++) {
                double x = sc.x + (rng.nextDouble() - 0.5) * 6, y = 64 + rng.nextDouble() * 3, z = sc.z + (rng.nextDouble() - 0.5) * 6;
                float yaw = (float) (rng.nextDouble() * 720 - 360);
                float pitch = (float) (rng.nextDouble() * 180 - 90);
                float headYaw = rng.chance(0.7) ? yaw : yaw + (float) ((rng.nextDouble() - 0.5) * 40);
                boolean sneaking = rng.chance(0.25);
                boolean creative = rng.chance(0.15);
                double reach = creative ? 5.0 : 4.5;
                r.player.updatePosition(x, y, z);
                r.player.yaw = yaw;
                r.player.pitch = pitch;
                r.player.headYaw = headYaw;
                Harness.setKeys(sneaking ? Scenario.SNEAK : 0);
                r.player.input.tick();
                Vec3d eyes = r.player.getCameraPosVec(1.0F);
                Vec3d look = r.player.getRotationVector(1.0F);
                Vec3d end = eyes.add(look.x * reach, look.y * reach, look.z * reach);
                Json j = new Json().beginObject().field("type", "ray").field("k", k);
                j.key("feet").beginArray().f64(x).f64(y).f64(z).endArray();
                j.fieldF32("yaw", yaw).fieldF32("pitch", pitch).fieldF32("head_yaw", headYaw);
                j.field("sneaking", r.player.isSneaking());
                j.fieldF32("eye_height", r.player.getEyeHeight());
                j.fieldF64("reach", reach);
                j.key("eyes").beginArray().f64(eyes.x).f64(eyes.y).f64(eyes.z).endArray();
                j.key("look").beginArray().f64(look.x).f64(look.y).f64(look.z).endArray();
                j.key("pick");
                hit(j, r.player.rayTrace(reach, 1.0F));
                j.key("ray_fff");
                hit(j, r.world.rayTrace(eyes, end, false, false, false));
                sink.accept(j.endObject().toString());
            }
            Harness.releaseKeys();
        }
    }

    private static void hit(Json j, BlockHitResult h) {
        j.beginObject();
        if (h == null) {
            j.field("type", "null");
        } else {
            j.field("type", h.type.name().toLowerCase());
            BlockPos p = h.getBlockPos();
            if (p != null) {
                j.key("block").beginArray().value(p.getX()).value(p.getY()).value(p.getZ()).endArray();
            }
            j.field("side", h.direction == null ? null : h.direction.name().toLowerCase());
            if (h.pos != null) {
                j.key("hit").beginArray().f64(h.pos.x).f64(h.pos.y).f64(h.pos.z).endArray();
            }
        }
        j.endObject();
    }
}
