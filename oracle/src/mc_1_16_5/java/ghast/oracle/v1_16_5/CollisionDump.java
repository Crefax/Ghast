package ghast.oracle.v1_16_5;

import ghast.oracle.Json;
import ghast.oracle.Scenario;
import net.minecraft.block.Block;
import net.minecraft.block.BlockState;
import net.minecraft.block.ShapeContext;
import net.minecraft.fluid.FluidState;
import net.minecraft.tag.BlockTags;
import net.minecraft.tag.FluidTags;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.Box;
import net.minecraft.util.registry.Registry;
import net.minecraft.util.shape.VoxelShape;
import net.minecraft.world.chunk.ChunkSection;
import net.minecraft.world.chunk.WorldChunk;

import java.util.List;
import java.util.function.Consumer;

/**
 * Collision shapes of every 1.16.5 block state, as the real collision query sees them
 * ({@code BlockState.getCollisionShape(world, pos, context)}). Shapes are state-driven in 1.16; the
 * context (the colliding entity: its feet height and whether it is descending) matters for a few
 * blocks (scaffolding, fluids), so those are dumped per context. Schema: oracle/README.md.
 */
final class CollisionDump {
    static final int RX = 0, RY = 64, RZ = 0;

    /** Far reference positions (see the 1.8.9 dump): far x/z, and just past 2^24 on both axes. */
    static final int[][] FAR_POSITIONS = {{29_999_000, 200, -29_999_000}, {-1_234_567, 5, 7_654_321},
            {-16_777_217, 100, -16_777_219}};

    /** Entity contexts: name, feet height relative to the block's y, descending (sneaking). */
    private record Ctx(String name, double dy, boolean descending) {}

    private static final Ctx[] CONTEXTS = {new Ctx("below", -1.8, false), new Ctx("inside", 0.5, false),
            new Ctx("above", 1.0, false), new Ctx("above_descending", 1.0, true), new Ctx("inside_descending", 0.5, true)};

    private CollisionDump() {}

    static void lines(Consumer<String> sink) {
        Harness.Run run = runAround(RX, RY, RZ);
        // Block-level properties.
        for (Block b : Registry.BLOCK) {
            Json j = new Json().beginObject().field("type", "block");
            j.field("name", Registry.BLOCK.getId(b).toString());
            j.field("first_state", States.id(b.getStateManager().getStates().get(0)));
            j.field("states", b.getStateManager().getStates().size());
            j.field("default_state", States.id(b.getDefaultState()));
            j.fieldF32("slipperiness", b.getSlipperiness());
            j.fieldF32("velocity_multiplier", b.getVelocityMultiplier());
            j.fieldF32("jump_velocity_multiplier", b.getJumpVelocityMultiplier());
            j.field("collidable", Reflect.getBoolean(net.minecraft.block.AbstractBlock.class, b, "collidable"));
            j.field("climbable", b.isIn(BlockTags.CLIMBABLE));
            j.field("soul_speed_block", b.isIn(BlockTags.SOUL_SPEED_BLOCKS));
            j.field("offset_type", b.getOffsetType().name().toLowerCase());
            sink.accept(j.endObject().toString());
        }
        // Every state.
        BlockPos pos = new BlockPos(RX, RY, RZ);
        for (int id = 0; id < States.count(); id++) {
            BlockState state = States.byId(id);
            place(run, RX, RY, RZ, state);
            Json j = new Json().beginObject().field("type", "shape").field("id", id).field("state", States.name(state));
            VoxelShape base = state.getCollisionShape(run.world, pos, ShapeContext.absent());
            j.key("boxes");
            boxes(j, base, 0, 0, 0);
            j.field("full_cube", state.isFullCube(run.world, pos));
            j.field("solid_block", state.isSolidBlock(run.world, pos));
            j.field("suffocates", state.shouldSuffocate(run.world, pos));
            FluidState fluid = state.getFluidState();
            j.field("fluid", fluid.isEmpty() ? null : fluid.isIn(FluidTags.WATER) ? "water" : fluid.isIn(FluidTags.LAVA) ? "lava" : "other");
            if (!fluid.isEmpty()) {
                j.field("fluid_level", fluid.getLevel());
                j.field("fluid_still", fluid.isStill());
                j.fieldF32("fluid_height", fluid.getHeight(run.world, pos));
            }
            // Entity contexts, only where the result differs from the context-free shape.
            String baseKey = render(base, 0, 0, 0);
            boolean anyDiff = false;
            Json ctx = new Json().beginObject();
            for (Ctx c : CONTEXTS) {
                VoxelShape s = state.getCollisionShape(run.world, pos, context(run, RY + c.dy(), c.descending()));
                String key = render(s, 0, 0, 0);
                if (!key.equals(baseKey)) {
                    anyDiff = true;
                }
                ctx.key(c.name());
                boxes(ctx, s, 0, 0, 0);
            }
            ctx.endObject();
            if (anyDiff) {
                j.key("contexts").raw(ctx.toString());
            }
            place(run, RX, RY, RZ, States.byId(0));
            sink.accept(j.endObject().toString());
        }
        reset(run);
    }

    /**
     * The context-free shape of every state at far positions, in absolute coordinates, with a flag
     * telling whether it equals the origin shape offset by the position in double precision.
     */
    static void farLines(Consumer<String> sink) {
        Harness.Run origin = runAround(RX, RY, RZ);
        VoxelShape[] localShapes = new VoxelShape[States.count()];
        BlockPos o = new BlockPos(RX, RY, RZ);
        for (int id = 0; id < localShapes.length; id++) {
            BlockState state = States.byId(id);
            place(origin, RX, RY, RZ, state);
            localShapes[id] = state.getCollisionShape(origin.world, o, ShapeContext.absent());
            place(origin, RX, RY, RZ, States.byId(0));
        }
        for (int[] p : FAR_POSITIONS) {
            Harness.Run far = runAround(p[0], p[1], p[2]);
            BlockPos pos = new BlockPos(p[0], p[1], p[2]);
            for (int id = 0; id < localShapes.length; id++) {
                BlockState state = States.byId(id);
                place(far, p[0], p[1], p[2], state);
                // What BlockCollisionSpliterator does: the state's shape offset by the block position.
                VoxelShape abs = state.getCollisionShape(far.world, pos, ShapeContext.absent()).offset(p[0], p[1], p[2]);
                VoxelShape expect = localShapes[id].offset(p[0], p[1], p[2]);
                Json j = new Json().beginObject().field("type", "shape_abs").field("id", id);
                j.key("pos").beginArray().value(p[0]).value(p[1]).value(p[2]).endArray();
                j.key("boxes");
                boxes(j, abs, 0, 0, 0);
                j.field("same_as_local", render(abs, 0, 0, 0).equals(render(expect, 0, 0, 0)));
                place(far, p[0], p[1], p[2], States.byId(0));
                sink.accept(j.endObject().toString());
            }
            reset(far);
        }
        reset(origin);
    }

    private static Harness.Run runAround(int x, int y, int z) {
        Scenario sc = new Scenario("collision_dump").spawn(x + 0.5, Math.min(250, y + 40), z + 0.5, 0, 0);
        sc.fill(x - 2, y, z - 2, x + 2, y, z + 2, 0, 0);
        return new Harness.Run(sc);
    }

    private static void reset(Harness.Run run) {
        run.player.input.sneaking = false;
    }

    /** Writes a state straight into the loaded chunk section (no neighbour updates, no lighting). */
    private static void place(Harness.Run run, int x, int y, int z, BlockState state) {
        WorldChunk chunk = run.world.getChunk(x >> 4, z >> 4);
        ChunkSection[] sections = chunk.getSectionArray();
        if (sections[y >> 4] == null) {
            sections[y >> 4] = new ChunkSection((y >> 4) << 4);
        }
        sections[y >> 4].setBlockState(x & 15, y & 15, z & 15, state, false);
    }

    /** The context the collision query builds for the player (ShapeContext.of) at the given feet height. */
    private static ShapeContext context(Harness.Run run, double feetY, boolean descending) {
        run.player.setPos(run.player.getX(), feetY, run.player.getZ());
        run.player.input.sneaking = descending;
        return ShapeContext.of(run.player);
    }

    private static void boxes(Json j, VoxelShape shape, double ox, double oy, double oz) {
        j.beginArray();
        List<Box> list = shape.getBoundingBoxes();
        for (Box b : list) {
            j.beginArray().f64(b.minX - ox).f64(b.minY - oy).f64(b.minZ - oz).f64(b.maxX - ox).f64(b.maxY - oy).f64(b.maxZ - oz).endArray();
        }
        j.endArray();
    }

    private static String render(VoxelShape shape, double ox, double oy, double oz) {
        StringBuilder sb = new StringBuilder();
        for (Box b : shape.getBoundingBoxes()) {
            sb.append(Double.doubleToRawLongBits(b.minX)).append(',').append(Double.doubleToRawLongBits(b.minY)).append(',')
                    .append(Double.doubleToRawLongBits(b.minZ)).append(',').append(Double.doubleToRawLongBits(b.maxX)).append(',')
                    .append(Double.doubleToRawLongBits(b.maxY)).append(',').append(Double.doubleToRawLongBits(b.maxZ)).append(';');
        }
        return sb.toString();
    }
}
