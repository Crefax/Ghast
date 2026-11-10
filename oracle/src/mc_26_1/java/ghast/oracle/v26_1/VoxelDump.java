package ghast.oracle.v26_1;

import ghast.oracle.Json;
import ghast.oracle.Scenario;
import it.unimi.dsi.fastutil.doubles.DoubleList;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.tags.BlockTags;
import net.minecraft.tags.FluidTags;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FlowingFluid;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.DiscreteVoxelShape;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.HashMap;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.function.Consumer;

/**
 * 26.1 collision shapes as the voxel grids the collision code works on (point positions per axis plus
 * the filled cells), deduplicated, and the per-state / per-block physics facts. Same schema as the 1.16.5
 * file (oracle/README.md, "voxel_shapes.jsonl.gz") except that 26.1 has no block materials: the
 * material fields are replaced by the per-state flags that replaced them (see "26.1").
 */
final class VoxelDump {
    static final int RX = 0, RY = 64, RZ = 0;

    /**
     * Player contexts. The first five are coarse positions; the feet_* ones sit on both sides of the
     * candidate thresholds of context-dependent shapes (scaffolding): the player's feet are at
     * y = 64 + dy (double), and _descending means sneaking.
     */
    private static final CollisionDump.Ctx[] CONTEXTS;

    static {
        java.util.List<CollisionDump.Ctx> c = new java.util.ArrayList<>(java.util.List.of(CollisionDump.CONTEXTS));
        double[] feet = {-0.1, -2.0E-5, -1.0E-6, 0.0, 1.0E-6, 0.01, 0.05, 0.1, 0.124, 0.125, 0.13, 0.999, 1.0 - 2.0E-5,
                1.0 - 5.0E-6, 1.0};
        for (double f : feet) {
            c.add(new CollisionDump.Ctx("feet_" + f, f, false));
            c.add(new CollisionDump.Ctx("feet_" + f + "_descending", f, true));
        }
        CONTEXTS = c.toArray(new CollisionDump.Ctx[0]);
    }

    private static final Direction[] SIDES = {Direction.NORTH, Direction.EAST, Direction.SOUTH, Direction.WEST};

    private VoxelDump() {}

    static void lines(Consumer<String> sink) throws Exception {
        Harness.Run run = runAround();
        Method points = VoxelShape.class.getDeclaredMethod("getCoords", Direction.Axis.class);
        points.setAccessible(true);
        Field voxelsField = VoxelShape.class.getDeclaredField("shape");
        voxelsField.setAccessible(true);
        Field shapeCache = BlockBehaviour.BlockStateBase.class.getDeclaredField("cache");
        shapeCache.setAccessible(true);

        Map<String, Integer> shapeIds = new HashMap<>();
        Map<VoxelShape, Integer> byInstance = new IdentityHashMap<>();

        for (Block b : BuiltInRegistries.BLOCK) {
            Json j = new Json().beginObject().field("type", "block");
            j.field("name", BuiltInRegistries.BLOCK.getKey(b).toString());
            j.field("first_state", States.id(b.getStateDefinition().getPossibleStates().get(0)));
            j.field("states", b.getStateDefinition().getPossibleStates().size());
            j.field("default_state", States.id(b.defaultBlockState()));
            j.fieldF32("slipperiness", b.getFriction());
            j.fieldF32("velocity_multiplier", b.getSpeedFactor());
            j.fieldF32("jump_velocity_multiplier", b.getJumpFactor());
            BlockState d = b.defaultBlockState();
            j.key("tags").beginObject();
            j.field("climbable", d.is(BlockTags.CLIMBABLE)).field("fences", d.is(BlockTags.FENCES))
                    .field("walls", d.is(BlockTags.WALLS)).field("fence_gates", d.is(BlockTags.FENCE_GATES))
                    .field("trapdoors", d.is(BlockTags.TRAPDOORS)).field("soul_speed_blocks", d.is(BlockTags.SOUL_SPEED_BLOCKS))
                    .field("fire", d.is(BlockTags.FIRE));
            j.endObject();
            sink.accept(j.endObject().toString());
        }

        // The contexts used below (exact feet heights).
        for (CollisionDump.Ctx c : CONTEXTS) {
            Json j = new Json().beginObject().field("type", "context").field("name", c.name());
            j.fieldF64("feet_dy", c.dy()).fieldF64("feet_y", RY + c.dy()).field("block_y", RY).field("descending", c.descending());
            j.fieldF64("fall_distance", c.fall()).field("leather_boots", c.leatherBoots());
            sink.accept(j.endObject().toString());
        }
        BlockPos pos = new BlockPos(RX, RY, RZ);
        for (int id = 0; id < States.count(); id++) {
            BlockState state = States.byId(id);
            place(run, RX, RY, RZ, state);
            VoxelShape shape = state.getCollisionShape(run.level, pos, CollisionContext.empty());
            Map<String, Integer> ctxIds = new LinkedHashMap<>();
            boolean ctxDiff = false;
            int baseId = shapeId(shape, sink, shapeIds, byInstance, points, voxelsField);
            for (CollisionDump.Ctx c : CONTEXTS) {
                VoxelShape s = state.getCollisionShape(run.level, pos, CollisionDump.context(run, RY + c.dy(), c));
                int sid = shapeId(s, sink, shapeIds, byInstance, points, voxelsField);
                ctxIds.put(c.name(), sid);
                ctxDiff |= sid != baseId;
            }
            CollisionDump.reset(run);

            Json j = new Json().beginObject().field("type", "state").field("id", id).field("state", States.name(state));
            j.field("collision_shape", baseId);
            if (ctxDiff) {
                j.key("context_shapes").beginObject();
                ctxIds.forEach((k, v) -> j.field(k, v));
                j.endObject();
            }
            j.field("full_cube_identity", shape == Shapes.block());
            j.field("empty_identity", shape == Shapes.empty());
            j.field("exceeds_cube", state.hasLargeCollisionShape());
            j.field("should_suffocate", state.isSuffocating(run.level, pos));
            j.field("is_air", state.isAir());
            j.field("has_dynamic_bounds", shapeCache.get(state) == null);
            j.field("blocks_motion", state.blocksMotion());
            j.field("liquid", state.liquid()).field("solid", state.isSolid()).field("replaceable", state.canBeReplaced());
            j.field("has_offset", state.hasOffsetFunction());
            j.key("side_solid_full_square").beginArray();
            for (Direction d : SIDES) {
                j.value(state.isFaceSturdy(run.level, pos, d));
            }
            j.endArray();
            j.field("side_solid_up", state.isFaceSturdy(run.level, pos, Direction.UP));
            j.field("side_solid_down", state.isFaceSturdy(run.level, pos, Direction.DOWN));
            FluidState fluid = state.getFluidState();
            if (fluid.isEmpty()) {
                j.key("fluid").raw("null");
            } else {
                j.key("fluid").beginObject();
                j.field("kind", fluid.is(FluidTags.WATER) ? "water" : fluid.is(FluidTags.LAVA) ? "lava" : "other");
                j.field("level", fluid.getAmount());
                j.field("falling", fluid.hasProperty(FlowingFluid.FALLING) && fluid.getValue(FlowingFluid.FALLING));
                j.field("still", fluid.isSource());
                j.fieldF32("height", fluid.getHeight(run.level, pos));
                // The same fluid above makes the height 1.0.
                BlockState above = fluid.createLegacyBlock();
                place(run, RX, RY + 1, RZ, above);
                j.fieldF32("height_fluid_above", fluid.getHeight(run.level, pos));
                place(run, RX, RY + 1, RZ, States.byId(0));
                j.endObject();
            }
            place(run, RX, RY, RZ, States.byId(0));
            sink.accept(j.endObject().toString());
        }
    }

    /** Emits a voxel_shape line the first time a shape (by content; identity tracked) is seen. */
    private static int shapeId(VoxelShape shape, Consumer<String> sink, Map<String, Integer> ids, Map<VoxelShape, Integer> byInstance,
                               Method points, Field voxelsField) throws Exception {
        Integer known = byInstance.get(shape);
        if (known != null) {
            return known;
        }
        DiscreteVoxelShape voxels = (DiscreteVoxelShape) voxelsField.get(shape);
        int nx = voxels.getXSize(), ny = voxels.getYSize(), nz = voxels.getZSize();
        // Same class names as the 1.16.5 file: simple = CubeVoxelShape, array = ArrayVoxelShape, sliced = SliceShape.
        String sn = shape.getClass().getSimpleName();
        String cls = sn.equals("CubeVoxelShape") ? "simple" : sn.equals("ArrayVoxelShape") ? "array"
                : sn.equals("SliceShape") ? "sliced" : sn;
        StringBuilder filled = new StringBuilder(nx * ny * nz);
        for (int x = 0; x < nx; x++) {
            for (int y = 0; y < ny; y++) {
                for (int z = 0; z < nz; z++) {
                    filled.append(voxels.isFull(x, y, z) ? '1' : '0');
                }
            }
        }
        Json pj = new Json().beginObject();
        for (Direction.Axis axis : Direction.Axis.values()) {
            DoubleList list = (DoubleList) points.invoke(shape, axis);
            pj.key("points_" + axis.getName()).beginArray();
            for (int i = 0; i < list.size(); i++) {
                pj.f64(list.getDouble(i));
            }
            pj.endArray();
        }
        String pointsJson = pj.endObject().toString();
        String key = cls + "|" + nx + "," + ny + "," + nz + "|" + filled + "|" + pointsJson;
        Integer sid = ids.get(key);
        if (sid == null) {
            sid = ids.size();
            ids.put(key, sid);
            Json j = new Json().beginObject().field("type", "voxel_shape").field("sid", sid).field("class", cls);
            j.key("sizes").beginArray().value(nx).value(ny).value(nz).endArray();
            String inner = pointsJson.substring(1, pointsJson.length() - 1);
            j.raw(inner);
            j.field("filled", filled.toString());
            j.field("empty", shape.isEmpty());
            j.key("boxes").beginArray();
            for (AABB b : shape.toAabbs()) {
                j.beginArray().f64(b.minX).f64(b.minY).f64(b.minZ).f64(b.maxX).f64(b.maxY).f64(b.maxZ).endArray();
            }
            j.endArray();
            sink.accept(j.endObject().toString());
        }
        byInstance.put(shape, sid);
        return sid;
    }

    private static Harness.Run runAround() {
        Scenario sc = new Scenario("voxel_dump").spawn(RX + 0.5, RY + 40, RZ + 0.5, 0, 0);
        sc.fill(RX - 2, RY, RZ - 2, RX + 2, RY, RZ + 2, 0, 0);
        return new Harness.Run(sc);
    }

    private static void place(Harness.Run run, int x, int y, int z, BlockState state) {
        CollisionDump.place(run, x, y, z, state);
    }
}
