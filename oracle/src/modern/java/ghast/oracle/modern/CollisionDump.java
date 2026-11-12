package ghast.oracle.modern;

import ghast.oracle.Json;
import ghast.oracle.Scenario;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.tags.BlockTags;
import net.minecraft.tags.FluidTags;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.LevelChunk;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.VoxelShape;

import java.util.List;
import java.util.function.Consumer;

/**
 * Collision shapes of every 26.1 block state, as the real collision query sees them
 * ({@code BlockState.getCollisionShape(level, pos, context)}). Shapes are state-driven; the context (the
 * colliding entity: its feet height, whether it is descending, its boots) matters for a few blocks
 * (scaffolding, powder snow, fluids), so those are dumped per context. Schema: oracle/README.md ("26.1").
 */
final class CollisionDump {
    static final int RX = 0, RY = 64, RZ = 0;

    /** Far reference positions (see the 1.8.9 dump): far x/z, and just past 2^24 on both axes. */
    static final int[][] FAR_POSITIONS = {{29_999_000, 200, -29_999_000}, {-1_234_567, 5, 7_654_321},
            {-16_777_217, 100, -16_777_219}};

    /**
     * Entity contexts: name, feet height relative to the block's y, descending (sneaking), fall distance,
     * leather boots worn (powder snow reads the last two from the entity).
     */
    record Ctx(String name, double dy, boolean descending, double fall, boolean leatherBoots) {
        Ctx(String name, double dy, boolean descending) {
            this(name, dy, descending, 0.0, false);
        }
    }

    static final Ctx[] CONTEXTS = {new Ctx("below", -1.8, false), new Ctx("inside", 0.5, false),
            new Ctx("above", 1.0, false), new Ctx("above_descending", 1.0, true), new Ctx("inside_descending", 0.5, true),
            new Ctx("above_falling", 1.0, false, 3.0, false), new Ctx("inside_falling", 0.5, false, 3.0, false),
            new Ctx("above_leather_boots", 1.0, false, 0.0, true), new Ctx("above_leather_boots_descending", 1.0, true, 0.0, true)};

    private CollisionDump() {}

    static void lines(Consumer<String> sink) {
        Harness.Run run = runAround(RX, RY, RZ);
        // Block-level properties.
        for (Block b : BuiltInRegistries.BLOCK) {
            Json j = new Json().beginObject().field("type", "block");
            j.field("name", BuiltInRegistries.BLOCK.getKey(b).toString());
            j.field("first_state", States.id(b.getStateDefinition().getPossibleStates().get(0)));
            j.field("states", b.getStateDefinition().getPossibleStates().size());
            j.field("default_state", States.id(b.defaultBlockState()));
            j.fieldF32("slipperiness", b.getFriction());
            j.fieldF32("velocity_multiplier", b.getSpeedFactor());
            j.fieldF32("jump_velocity_multiplier", b.getJumpFactor());
            j.field("collidable", Reflect.getBoolean(BlockBehaviour.class, b, "hasCollision"));
            j.field("climbable", b.defaultBlockState().is(BlockTags.CLIMBABLE));
            j.field("soul_speed_block", b.defaultBlockState().is(BlockTags.SOUL_SPEED_BLOCKS));
            j.field("offset", b.defaultBlockState().hasOffsetFunction());
            sink.accept(j.endObject().toString());
        }
        // Every state.
        BlockPos pos = new BlockPos(RX, RY, RZ);
        for (int id = 0; id < States.count(); id++) {
            BlockState state = States.byId(id);
            place(run, RX, RY, RZ, state);
            Json j = new Json().beginObject().field("type", "shape").field("id", id).field("state", States.name(state));
            VoxelShape base = state.getCollisionShape(run.level, pos, CollisionContext.empty());
            j.key("boxes");
            boxes(j, base, 0, 0, 0);
            j.field("full_cube", state.isCollisionShapeFullBlock(run.level, pos));
            j.field("solid_block", state.isRedstoneConductor(run.level, pos));
            j.field("suffocates", state.isSuffocating(run.level, pos));
            FluidState fluid = state.getFluidState();
            j.field("fluid", fluid.isEmpty() ? null : fluid.is(FluidTags.WATER) ? "water" : fluid.is(FluidTags.LAVA) ? "lava" : "other");
            if (!fluid.isEmpty()) {
                j.field("fluid_level", fluid.getAmount());
                j.field("fluid_still", fluid.isSource());
                j.fieldF32("fluid_height", fluid.getHeight(run.level, pos));
            }
            // Entity contexts, only where the result differs from the context-free shape.
            String baseKey = render(base, 0, 0, 0);
            boolean anyDiff = false;
            Json ctx = new Json().beginObject();
            for (Ctx c : CONTEXTS) {
                VoxelShape s = state.getCollisionShape(run.level, pos, context(run, RY + c.dy(), c));
                String key = render(s, 0, 0, 0);
                if (!key.equals(baseKey)) {
                    anyDiff = true;
                }
                ctx.key(c.name());
                boxes(ctx, s, 0, 0, 0);
            }
            ctx.endObject();
            reset(run);
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
            localShapes[id] = state.getCollisionShape(origin.level, o, CollisionContext.empty());
            place(origin, RX, RY, RZ, States.byId(0));
        }
        for (int[] p : FAR_POSITIONS) {
            Harness.Run far = runAround(p[0], p[1], p[2]);
            BlockPos pos = new BlockPos(p[0], p[1], p[2]);
            for (int id = 0; id < localShapes.length; id++) {
                BlockState state = States.byId(id);
                place(far, p[0], p[1], p[2], state);
                // What BlockCollisions does: the state's shape moved by the block position.
                VoxelShape abs = state.getCollisionShape(far.level, pos, CollisionContext.empty()).move(p[0], p[1], p[2]);
                VoxelShape expect = localShapes[id].move(p[0], p[1], p[2]);
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

    static void reset(Harness.Run run) {
        sneak(run, false);
        run.player.fallDistance = 0.0;
        run.player.setItemSlot(EquipmentSlot.FEET, ItemStack.EMPTY);
    }

    /** The player's held sneak key, which makes a player "descending" for collision contexts. */
    static void sneak(Harness.Run run, boolean shift) {
        run.player.input.keyPresses = new Input(false, false, false, false, false, shift, false);
    }

    /** Writes a state straight into the loaded chunk section (no neighbour updates, no lighting). */
    static void place(Harness.Run run, int x, int y, int z, BlockState state) {
        LevelChunk chunk = run.level.getChunk(x >> 4, z >> 4);
        chunk.getSection(chunk.getSectionIndex(y)).setBlockState(x & 15, y & 15, z & 15, state, false);
    }

    /** The context the collision query builds for the player (CollisionContext.of) at the given feet height. */
    static CollisionContext context(Harness.Run run, double feetY, Ctx c) {
        run.player.setPos(run.player.getX(), feetY, run.player.getZ());
        sneak(run, c.descending());
        run.player.fallDistance = c.fall();
        run.player.setItemSlot(EquipmentSlot.FEET, c.leatherBoots() ? new ItemStack(Items.LEATHER_BOOTS) : ItemStack.EMPTY);
        return CollisionContext.of(run.player);
    }

    private static void boxes(Json j, VoxelShape shape, double ox, double oy, double oz) {
        j.beginArray();
        List<AABB> list = shape.toAabbs();
        for (AABB b : list) {
            j.beginArray().f64(b.minX - ox).f64(b.minY - oy).f64(b.minZ - oz).f64(b.maxX - ox).f64(b.maxY - oy).f64(b.maxZ - oz).endArray();
        }
        j.endArray();
    }

    private static String render(VoxelShape shape, double ox, double oy, double oz) {
        StringBuilder sb = new StringBuilder();
        for (AABB b : shape.toAabbs()) {
            sb.append(Double.doubleToRawLongBits(b.minX)).append(',').append(Double.doubleToRawLongBits(b.minY)).append(',')
                    .append(Double.doubleToRawLongBits(b.minZ)).append(',').append(Double.doubleToRawLongBits(b.maxX)).append(',')
                    .append(Double.doubleToRawLongBits(b.maxY)).append(',').append(Double.doubleToRawLongBits(b.maxZ)).append(';');
        }
        return sb.toString();
    }
}
