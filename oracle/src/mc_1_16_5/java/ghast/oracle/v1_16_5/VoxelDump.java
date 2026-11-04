package ghast.oracle.v1_16_5;

import ghast.oracle.Json;
import ghast.oracle.Scenario;
import it.unimi.dsi.fastutil.doubles.DoubleList;
import net.minecraft.block.AbstractBlock;
import net.minecraft.block.Block;
import net.minecraft.block.BlockState;
import net.minecraft.block.Material;
import net.minecraft.block.ShapeContext;
import net.minecraft.fluid.FlowableFluid;
import net.minecraft.fluid.FluidState;
import net.minecraft.state.property.Properties;
import net.minecraft.tag.BlockTags;
import net.minecraft.tag.FluidTags;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.Box;
import net.minecraft.util.math.Direction;
import net.minecraft.util.registry.Registry;
import net.minecraft.util.shape.SimpleVoxelShape;
import net.minecraft.util.shape.VoxelSet;
import net.minecraft.util.shape.VoxelShape;
import net.minecraft.util.shape.VoxelShapes;
import net.minecraft.world.chunk.ChunkSection;
import net.minecraft.world.chunk.WorldChunk;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.HashMap;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.function.Consumer;

/**
 * 1.16.5 collision shapes as the voxel grids the collision code works on (point positions per axis
 * plus the filled cells), deduplicated, and the per-state / per-block physics facts. Schema:
 * oracle/README.md, "voxel_shapes.jsonl.gz".
 */
final class VoxelDump {
    static final int RX = 0, RY = 64, RZ = 0;

    private record Ctx(String name, double dy, boolean descending) {}

    /**
     * Player contexts. The first five are coarse positions; the feet_* ones sit on both sides of the
     * candidate thresholds of context-dependent shapes (scaffolding): the player's feet are at
     * y = 64 + dy (double), and _descending means sneaking.
     */
    private static final Ctx[] CONTEXTS;

    static {
        java.util.List<Ctx> c = new java.util.ArrayList<>(java.util.List.of(new Ctx("below", -1.8, false),
                new Ctx("inside", 0.5, false), new Ctx("above", 1.0, false), new Ctx("above_descending", 1.0, true),
                new Ctx("inside_descending", 0.5, true)));
        double[] feet = {-0.1, -2.0E-5, -1.0E-6, 0.0, 1.0E-6, 0.01, 0.05, 0.1, 0.124, 0.125, 0.13, 0.999, 1.0 - 2.0E-5,
                1.0 - 5.0E-6, 1.0};
        for (double f : feet) {
            c.add(new Ctx("feet_" + f, f, false));
            c.add(new Ctx("feet_" + f + "_descending", f, true));
        }
        CONTEXTS = c.toArray(new Ctx[0]);
    }

    private static final Direction[] SIDES = {Direction.NORTH, Direction.EAST, Direction.SOUTH, Direction.WEST};

    private VoxelDump() {}

    static void lines(Consumer<String> sink) throws Exception {
        Harness.Run run = runAround();
        Map<Material, String> materials = materialNames();
        Method points = VoxelShape.class.getDeclaredMethod("getPointPositions", Direction.Axis.class);
        points.setAccessible(true);
        Field voxelsField = VoxelShape.class.getDeclaredField("voxels");
        voxelsField.setAccessible(true);
        Field shapeCache = AbstractBlock.AbstractBlockState.class.getDeclaredField("shapeCache");
        shapeCache.setAccessible(true);

        Map<String, Integer> shapeIds = new HashMap<>();
        Map<VoxelShape, Integer> byInstance = new IdentityHashMap<>();

        for (Block b : Registry.BLOCK) {
            Json j = new Json().beginObject().field("type", "block");
            j.field("name", Registry.BLOCK.getId(b).toString());
            j.field("first_state", States.id(b.getStateManager().getStates().get(0)));
            j.field("states", b.getStateManager().getStates().size());
            j.field("default_state", States.id(b.getDefaultState()));
            j.fieldF32("slipperiness", b.getSlipperiness());
            j.fieldF32("velocity_multiplier", b.getVelocityMultiplier());
            j.fieldF32("jump_velocity_multiplier", b.getJumpVelocityMultiplier());
            j.key("tags").beginObject();
            j.field("climbable", b.isIn(BlockTags.CLIMBABLE)).field("fences", b.isIn(BlockTags.FENCES))
                    .field("walls", b.isIn(BlockTags.WALLS)).field("fence_gates", b.isIn(BlockTags.FENCE_GATES))
                    .field("trapdoors", b.isIn(BlockTags.TRAPDOORS)).field("soul_speed_blocks", b.isIn(BlockTags.SOUL_SPEED_BLOCKS))
                    .field("fire", b.isIn(BlockTags.FIRE));
            j.endObject();
            sink.accept(j.endObject().toString());
        }

        // The contexts used below (exact feet heights).
        for (Ctx c : CONTEXTS) {
            Json j = new Json().beginObject().field("type", "context").field("name", c.name());
            j.fieldF64("feet_dy", c.dy()).fieldF64("feet_y", RY + c.dy()).field("block_y", RY).field("descending", c.descending());
            sink.accept(j.endObject().toString());
        }
        BlockPos pos = new BlockPos(RX, RY, RZ);
        for (int id = 0; id < States.count(); id++) {
            BlockState state = States.byId(id);
            place(run, RX, RY, RZ, state);
            VoxelShape shape = state.getCollisionShape(run.world, pos, ShapeContext.absent());
            Map<String, Integer> ctxIds = new LinkedHashMap<>();
            boolean ctxDiff = false;
            int baseId = shapeId(shape, sink, shapeIds, byInstance, points, voxelsField);
            for (Ctx c : CONTEXTS) {
                run.player.setPos(run.player.getX(), RY + c.dy(), run.player.getZ());
                run.player.input.sneaking = c.descending();
                VoxelShape s = state.getCollisionShape(run.world, pos, ShapeContext.of(run.player));
                int sid = shapeId(s, sink, shapeIds, byInstance, points, voxelsField);
                ctxIds.put(c.name(), sid);
                ctxDiff |= sid != baseId;
            }
            run.player.input.sneaking = false;

            Json j = new Json().beginObject().field("type", "state").field("id", id).field("state", States.name(state));
            j.field("collision_shape", baseId);
            if (ctxDiff) {
                j.key("context_shapes").beginObject();
                ctxIds.forEach((k, v) -> j.field(k, v));
                j.endObject();
            }
            j.field("full_cube_identity", shape == VoxelShapes.fullCube());
            j.field("empty_identity", shape == VoxelShapes.empty());
            j.field("exceeds_cube", state.exceedsCube());
            j.field("should_suffocate", state.shouldSuffocate(run.world, pos));
            j.field("is_air", state.isAir());
            j.field("has_dynamic_bounds", shapeCache.get(state) == null);
            Material m = state.getMaterial();
            j.field("material", materials.getOrDefault(m, "unknown"));
            j.field("material_blocks_movement", m.blocksMovement());
            j.field("material_liquid", m.isLiquid()).field("material_solid", m.isSolid())
                    .field("material_replaceable", m.isReplaceable());
            j.key("side_solid_full_square").beginArray();
            for (Direction d : SIDES) {
                j.value(state.isSideSolidFullSquare(run.world, pos, d));
            }
            j.endArray();
            j.field("side_solid_up", state.isSideSolidFullSquare(run.world, pos, Direction.UP));
            j.field("side_solid_down", state.isSideSolidFullSquare(run.world, pos, Direction.DOWN));
            FluidState fluid = state.getFluidState();
            if (fluid.isEmpty()) {
                j.key("fluid").raw("null");
            } else {
                j.key("fluid").beginObject();
                j.field("kind", fluid.isIn(FluidTags.WATER) ? "water" : fluid.isIn(FluidTags.LAVA) ? "lava" : "other");
                j.field("level", fluid.getLevel());
                j.field("falling", fluid.contains(FlowableFluid.FALLING) && fluid.get(FlowableFluid.FALLING));
                j.field("still", fluid.isStill());
                j.fieldF32("height", fluid.getHeight(run.world, pos));
                // The same fluid above makes the height 1.0.
                BlockState above = fluid.getBlockState();
                place(run, RX, RY + 1, RZ, above);
                j.fieldF32("height_fluid_above", fluid.getHeight(run.world, pos));
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
        VoxelSet voxels = (VoxelSet) voxelsField.get(shape);
        int nx = voxels.getXSize(), ny = voxels.getYSize(), nz = voxels.getZSize();
        String cls = shape instanceof SimpleVoxelShape ? "simple"
                : shape.getClass().getSimpleName().equals("ArrayVoxelShape") ? "array"
                : shape.getClass().getSimpleName().equals("SlicedVoxelShape") ? "sliced" : shape.getClass().getSimpleName();
        StringBuilder filled = new StringBuilder(nx * ny * nz);
        for (int x = 0; x < nx; x++) {
            for (int y = 0; y < ny; y++) {
                for (int z = 0; z < nz; z++) {
                    filled.append(voxels.contains(x, y, z) ? '1' : '0');
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
            for (Box b : shape.getBoundingBoxes()) {
                j.beginArray().f64(b.minX).f64(b.minY).f64(b.minZ).f64(b.maxX).f64(b.maxY).f64(b.maxZ).endArray();
            }
            j.endArray();
            sink.accept(j.endObject().toString());
        }
        byInstance.put(shape, sid);
        return sid;
    }

    private static Map<Material, String> materialNames() throws IllegalAccessException {
        Map<Material, String> m = new IdentityHashMap<>();
        for (Field f : Material.class.getDeclaredFields()) {
            if (Modifier.isStatic(f.getModifiers()) && f.getType() == Material.class) {
                m.put((Material) f.get(null), f.getName().toLowerCase());
            }
        }
        return m;
    }

    private static Harness.Run runAround() {
        Scenario sc = new Scenario("voxel_dump").spawn(RX + 0.5, RY + 40, RZ + 0.5, 0, 0);
        sc.fill(RX - 2, RY, RZ - 2, RX + 2, RY, RZ + 2, 0, 0);
        return new Harness.Run(sc);
    }

    private static void place(Harness.Run run, int x, int y, int z, BlockState state) {
        WorldChunk chunk = run.world.getChunk(x >> 4, z >> 4);
        ChunkSection[] sections = chunk.getSectionArray();
        if (sections[y >> 4] == null) {
            sections[y >> 4] = new ChunkSection((y >> 4) << 4);
        }
        sections[y >> 4].setBlockState(x & 15, y & 15, z & 15, state, false);
    }
}
