package ghast.oracle.v26_1;

import ghast.oracle.Json;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import it.unimi.dsi.fastutil.doubles.DoubleList;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.entity.Pose;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.DiscreteVoxelShape;
import net.minecraft.world.phys.shapes.VoxelShape;

import java.lang.reflect.Constructor;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Consumer;

/**
 * 26.1 crosshair data.
 *
 * <p>{@code selection_shapes.jsonl.gz}: per state the shapes the block pick uses: the outline shape
 * ({@code BlockState.getShape(level, pos, CollisionContext.of(player))}, empty hand), the variants with
 * an empty context and while holding the block's own item where they differ (scaffolding, light),
 * and the interaction shape ({@code getInteractionShape}, which can only change the hit face). Shapes
 * of offset blocks (plants) are {@code base.move(offset(pos))}; the base shape is recorded.
 *
 * <p>{@code raytrace.jsonl.gz}: random worlds of random states, random viewpoints, and the real
 * {@code LocalPlayer.raycastHitResult(1.0F, player)} (what {@code Minecraft.pick} stores as the
 * crosshair target), with randomised block / entity interaction ranges, poses and old positions.
 * Schema: oracle/README.md.
 */
final class SelectionDump {
    static final int RX = 0, RY = 64, RZ = 0;
    /** Second position for the position-independence check. */
    static final int PX = 3, PY = 66, PZ = 5;

    private SelectionDump() {}

    private static Method points;
    private static Field voxelsField;
    private static Field offsetDelegate;
    private static Constructor<?> arrayShape;

    private static void reflect() throws Exception {
        points = VoxelShape.class.getDeclaredMethod("getCoords", Direction.Axis.class);
        points.setAccessible(true);
        voxelsField = VoxelShape.class.getDeclaredField("shape");
        voxelsField.setAccessible(true);
        offsetDelegate = net.minecraft.world.phys.shapes.OffsetDoubleList.class.getDeclaredField("delegate");
        offsetDelegate.setAccessible(true);
        Class<?> array = Class.forName("net.minecraft.world.phys.shapes.ArrayVoxelShape");
        arrayShape = array.getDeclaredConstructor(DiscreteVoxelShape.class, DoubleList.class, DoubleList.class, DoubleList.class);
        arrayShape.setAccessible(true);
    }

    /** Shape table shared by one dump (ids by content). */
    private static final class Shapes {
        final Map<String, Integer> ids = new HashMap<>();
        final Map<VoxelShape, Integer> byInstance = new IdentityHashMap<>();
        final Consumer<String> sink;

        Shapes(Consumer<String> sink) {
            this.sink = sink;
        }

        int id(VoxelShape shape) throws Exception {
            Integer known = byInstance.get(shape);
            if (known != null) {
                return known;
            }
            DiscreteVoxelShape voxels = (DiscreteVoxelShape) voxelsField.get(shape);
            int nx = voxels.getXSize(), ny = voxels.getYSize(), nz = voxels.getZSize();
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
            Json bj = new Json().beginArray();
            for (AABB b : shape.toAabbs()) {
                bj.beginArray().f64(b.minX).f64(b.minY).f64(b.minZ).f64(b.maxX).f64(b.maxY).f64(b.maxZ).endArray();
            }
            String boxesJson = bj.endArray().toString();
            String key = cls + "|" + nx + "," + ny + "," + nz + "|" + filled + "|" + pointsJson + "|" + boxesJson + "|" + shape.isEmpty();
            Integer sid = ids.get(key);
            if (sid == null) {
                sid = ids.size();
                ids.put(key, sid);
                Json j = new Json().beginObject().field("type", "voxel_shape").field("sid", sid).field("class", cls);
                j.key("sizes").beginArray().value(nx).value(ny).value(nz).endArray();
                j.raw(pointsJson.substring(1, pointsJson.length() - 1));
                j.field("filled", filled.toString());
                j.field("empty", shape.isEmpty());
                j.key("boxes").raw(boxesJson);
                sink.accept(j.endObject().toString());
            }
            byInstance.put(shape, sid);
            return sid;
        }
    }

    /**
     * The unshifted shape of a moved one ({@code base.move(d)} keeps {@code base}'s point lists inside
     * OffsetDoubleLists), as an array shape over those points; {@code null} when the shape is not moved.
     */
    private static VoxelShape base(VoxelShape shape) throws Exception {
        if (shape.isEmpty()) {
            return null;
        }
        DoubleList[] base = new DoubleList[3];
        for (Direction.Axis axis : Direction.Axis.values()) {
            DoubleList list = (DoubleList) points.invoke(shape, axis);
            if (!(list instanceof net.minecraft.world.phys.shapes.OffsetDoubleList)) {
                return null;
            }
            base[axis.ordinal()] = (DoubleList) offsetDelegate.get(list);
        }
        return (VoxelShape) arrayShape.newInstance(voxelsField.get(shape), base[0], base[1], base[2]);
    }

    private static Harness.Run newRun() {
        Scenario sc = new Scenario("selection_dump").spawn(RX + 0.5, RY + 40, RZ + 0.5, 0, 0);
        sc.fill(RX - 2, RY, RZ - 2, RX + 2, RY, RZ + 2, 0, 0);
        return new Harness.Run(sc);
    }

    private static void hold(Harness.Run run, Item item) {
        run.player.getInventory().setItem(run.player.getInventory().getSelectedSlot(), item == null ? ItemStack.EMPTY : new ItemStack(item));
    }

    static void selection(Consumer<String> sink) throws Exception {
        reflect();
        Harness.Run run = newRun();
        Shapes shapes = new Shapes(sink);
        BlockPos pos = new BlockPos(RX, RY, RZ), pos2 = new BlockPos(PX, PY, PZ);
        for (int id = 0; id < States.count(); id++) {
            BlockState state = States.byId(id);
            Block block = state.getBlock();
            Item own = block.asItem();
            CollisionDump.place(run, RX, RY, RZ, state);
            hold(run, null);
            VoxelShape outline = state.getShape(run.level, pos, CollisionContext.of(run.player));
            VoxelShape outlineEmpty = state.getShape(run.level, pos, CollisionContext.empty());
            VoxelShape outlineHolding = null;
            if (own != Items.AIR) {
                hold(run, own);
                outlineHolding = state.getShape(run.level, pos, CollisionContext.of(run.player));
                hold(run, null);
            }
            VoxelShape interaction = state.getInteractionShape(run.level, pos);

            Json j = new Json().beginObject().field("type", "state").field("id", id).field("state", States.name(state));
            VoxelShape outlineBase = base(outline);
            j.field("outline", shapes.id(outline));
            int emptySid = shapes.id(outlineEmpty);
            if (emptySid != shapes.id(outline)) {
                j.field("outline_empty_context", emptySid);
            }
            if (outlineHolding != null && shapes.id(outlineHolding) != shapes.id(outline)) {
                j.field("outline_holding_own_item", shapes.id(outlineHolding));
            }
            j.field("interaction", shapes.id(interaction));
            if (base(interaction) != null) {
                throw new IllegalStateException("moved interaction shape: " + States.name(state));
            }
            if (outlineBase != null) {
                j.field("outline_base", shapes.id(outlineBase));
                Vec3 off = state.getOffset(pos);
                j.key("offset_at_dump").beginArray().f64(off.x).f64(off.y).f64(off.z).endArray();
            }
            CollisionDump.place(run, RX, RY, RZ, States.byId(0));

            // Position independence: the same shape (or the same base) at another position.
            CollisionDump.place(run, PX, PY, PZ, state);
            VoxelShape other = state.getShape(run.level, pos2, CollisionContext.of(run.player));
            VoxelShape otherBase = base(other);
            boolean same = outlineBase != null
                    ? otherBase != null && shapes.id(otherBase) == shapes.id(outlineBase)
                    : otherBase == null && shapes.id(other) == shapes.id(outline);
            same &= shapes.id(state.getInteractionShape(run.level, pos2)) == shapes.id(interaction);
            CollisionDump.place(run, PX, PY, PZ, States.byId(0));
            j.field("position_independent", same);
            sink.accept(j.endObject().toString());
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Raytrace vectors

    static final int RAY_WORLDS = 16, RAY_RAYS = 200;
    /** The random block region (inclusive). */
    static final int X0 = -6, X1 = 6, Y0 = 59, Y1 = 71, Z0 = -6, Z1 = 6;

    /** Blocks whose outline depends on a block entity the dump does not create. */
    private static boolean excluded(Block b) {
        return b instanceof net.minecraft.world.level.block.ShulkerBoxBlock
                || b instanceof net.minecraft.world.level.block.piston.MovingPistonBlock;
    }

    /** Blocks with partial / special outlines, drawn more often than the uniform pick. */
    private static final String[] PARTIAL = {"oak_stairs", "stone_slab", "oak_fence", "cobblestone_wall", "glass_pane", "iron_bars",
            "oak_door", "oak_trapdoor", "oak_fence_gate", "scaffolding", "hopper", "cauldron", "water_cauldron", "composter",
            "lectern", "bell", "chain", "lantern", "candle", "pointed_dripstone", "bamboo", "short_grass", "tall_grass", "poppy",
            "torch", "wall_torch", "oak_sign", "oak_wall_sign", "ladder", "vine", "snow", "carpet", "white_carpet", "end_rod",
            "lightning_rod", "chest", "ender_chest", "anvil", "brewing_stand", "enchanting_table", "cake", "flower_pot",
            "sea_pickle", "turtle_egg", "big_dripleaf", "small_dripleaf", "azalea", "amethyst_cluster", "grindstone", "stonecutter",
            "campfire", "soul_sand", "mud", "farmland", "dirt_path", "honey_block", "powder_snow", "cobweb", "light", "barrier",
            "structure_void", "sweet_berry_bush", "kelp", "seagrass", "water", "lava", "bubble_column", "decorated_pot",
            "player_head", "dragon_head", "piston_head", "piston", "moss_carpet", "pale_moss_carpet", "leaf_litter",
            "pink_petals", "hanging_roots", "glow_lichen", "sculk_vein", "redstone_wire", "repeater", "comparator", "rail",
            "lever", "stone_button", "stone_pressure_plate", "tripwire", "tripwire_hook", "daylight_detector", "conduit",
            "copper_grate", "chiseled_bookshelf", "crafter", "vault", "trial_spawner", "heavy_core", "creaking_heart",
            "copper_bulb", "copper_chain", "copper_lantern", "shelf", "oak_shelf", "dried_ghast", "copper_golem_statue"};

    private static int[] partialStates;
    private static int[] allStates;

    private static void palettes() {
        List<Integer> all = new ArrayList<>();
        for (int id = 0; id < States.count(); id++) {
            if (!excluded(States.byId(id).getBlock())) {
                all.add(id);
            }
        }
        allStates = all.stream().mapToInt(Integer::intValue).toArray();
        List<Integer> partial = new ArrayList<>();
        for (String name : PARTIAL) {
            var key = net.minecraft.resources.Identifier.tryParse("minecraft:" + name);
            if (key == null || !BuiltInRegistries.BLOCK.containsKey(key)) {
                continue;
            }
            Block b = BuiltInRegistries.BLOCK.getValue(key);
            for (BlockState s : b.getStateDefinition().getPossibleStates()) {
                partial.add(States.id(s));
            }
        }
        partialStates = partial.stream().mapToInt(Integer::intValue).toArray();
    }

    static void rays(long seed, int worlds, int raysPerWorld, Consumer<String> sink) {
        Scenario sc = new Scenario("ray_dump").spawn(0.5, RY + 1, 0.5, 0, 0);
        Harness.Run run = new Harness.Run(sc);
        palettes();
        Pose[] poses = {Pose.STANDING, Pose.CROUCHING, Pose.SWIMMING, Pose.FALL_FLYING, Pose.SPIN_ATTACK};
        for (int w = 0; w < worlds; w++) {
            Rng rng = new Rng(seed * 1_000_003L + w ^ 0x7261797472616365L);
            double density = 0.05 + rng.nextDouble() * 0.45;
            double partialShare = rng.nextDouble();
            Json world = new Json().beginObject().field("type", "ray_world").field("world", w);
            world.key("region").beginArray().value(X0).value(Y0).value(Z0).value(X1).value(Y1).value(Z1).endArray();
            world.key("blocks").beginArray();
            Map<Integer, String> used = new java.util.TreeMap<>();
            for (int x = X0; x <= X1; x++) {
                for (int y = Y0; y <= Y1; y++) {
                    for (int z = Z0; z <= Z1; z++) {
                        int id = 0;
                        if (rng.chance(density)) {
                            id = rng.chance(partialShare) ? partialStates[rng.nextInt(partialStates.length)]
                                    : allStates[rng.nextInt(allStates.length)];
                        }
                        CollisionDump.place(run, x, y, z, States.byId(id));
                        if (id != 0) {
                            world.beginArray().value(x).value(y).value(z).value(id).endArray();
                            used.put(id, States.name(States.byId(id)));
                        }
                    }
                }
            }
            world.endArray();
            world.key("states").beginObject();
            used.forEach((k, v) -> world.field(Integer.toString(k), v));
            world.endObject();
            sink.accept(world.endObject().toString());

            for (int k = 0; k < raysPerWorld; k++) {
                double x = X0 - 1 + rng.nextDouble() * (X1 - X0 + 3);
                double y = Y0 - 1 + rng.nextDouble() * (Y1 - Y0 + 1);
                double z = Z0 - 1 + rng.nextDouble() * (Z1 - Z0 + 3);
                if (rng.chance(0.1)) {
                    // On a block boundary (eye exactly on a face plane after the eye offset is not
                    // guaranteed, but feet on a grid line exercise the integer paths).
                    x = Math.floor(x);
                    z = rng.chance(0.5) ? Math.floor(z) : z + 0.5;
                }
                double xo = x, yo = y, zo = z;
                if (rng.chance(0.4)) {
                    xo = x + (rng.nextDouble() - 0.5) * 0.6;
                    yo = y + (rng.nextDouble() - 0.5) * 0.6;
                    zo = z + (rng.nextDouble() - 0.5) * 0.6;
                }
                float yaw, pitch;
                double ry = rng.nextDouble();
                if (ry < 0.12) {
                    yaw = 45.0F * rng.range(-8, 8);
                } else if (ry < 0.17) {
                    yaw = (float) ((rng.nextDouble() - 0.5) * 2.0E5);
                } else {
                    yaw = (float) (rng.nextDouble() * 720 - 360);
                }
                double rp = rng.nextDouble();
                if (rp < 0.06) {
                    pitch = rng.chance(0.5) ? 90.0F : -90.0F;
                } else if (rp < 0.12) {
                    pitch = rng.chance(0.5) ? 0.0F : 45.0F * rng.range(-2, 2);
                } else {
                    pitch = (float) (rng.nextDouble() * 180 - 90);
                }
                Pose pose = rng.chance(0.6) ? Pose.STANDING : rng.pick(poses);
                double blockRange = pickRange(rng, 4.5);
                double entityRange = pickRange(rng, 3.0);
                Item held = null;
                double rh = rng.nextDouble();
                if (rh < 0.08) {
                    held = Items.SCAFFOLDING;
                } else if (rh < 0.12) {
                    held = Items.LIGHT;
                } else if (rh < 0.16) {
                    held = Items.STONE;
                }

                run.player.setPose(pose);
                run.player.setPos(x, y, z);
                run.player.xo = xo;
                run.player.yo = yo;
                run.player.zo = zo;
                run.player.setYRot(yaw);
                run.player.setXRot(pitch);
                run.player.getAttribute(Attributes.BLOCK_INTERACTION_RANGE).setBaseValue(blockRange);
                run.player.getAttribute(Attributes.ENTITY_INTERACTION_RANGE).setBaseValue(entityRange);
                hold(run, held);

                HitResult hit = run.player.raycastHitResult(1.0F, run.player);
                double br = run.player.blockInteractionRange(), er = run.player.entityInteractionRange();
                HitResult clip = run.player.pick(Math.max(br, er), 1.0F, false);
                Vec3 eyes = run.player.getEyePosition(1.0F);
                Vec3 view = run.player.getViewVector(1.0F);

                Json j = new Json().beginObject().field("type", "ray").field("k", k);
                j.key("feet").beginArray().f64(run.player.getX()).f64(run.player.getY()).f64(run.player.getZ()).endArray();
                j.key("old").beginArray().f64(xo).f64(yo).f64(zo).endArray();
                j.fieldF32("yaw", run.player.getYRot()).fieldF32("pitch", run.player.getXRot());
                j.field("pose", pose.name().toLowerCase());
                j.fieldF32("eye_height", run.player.getEyeHeight());
                j.fieldF64("block_range", br).fieldF64("entity_range", er);
                j.field("held", held == null ? null : BuiltInRegistries.ITEM.getKey(held).toString());
                j.key("eyes").beginArray().f64(eyes.x).f64(eyes.y).f64(eyes.z).endArray();
                j.key("view").beginArray().f64(view.x).f64(view.y).f64(view.z).endArray();
                j.key("clip");
                hit(j, clip);
                j.key("pick");
                hit(j, hit);
                sink.accept(j.endObject().toString());
            }
            for (int x = X0; x <= X1; x++) {
                for (int y = Y0; y <= Y1; y++) {
                    for (int z = Z0; z <= Z1; z++) {
                        CollisionDump.place(run, x, y, z, States.byId(0));
                    }
                }
            }
        }
        hold(run, null);
    }

    private static double pickRange(Rng rng, double dflt) {
        double r = rng.nextDouble();
        if (r < 0.35) {
            return dflt;
        } else if (r < 0.45) {
            return dflt + (rng.chance(0.5) ? 0.5 : 2.0);
        } else if (r < 0.52) {
            return rng.pick(new Double[] {0.0, 1.0E-4, 0.5, 64.0, 100.0});
        } else {
            return rng.nextDouble() * 12.0;
        }
    }

    private static void hit(Json j, HitResult h) {
        j.beginObject();
        j.field("type", h.getType().name().toLowerCase());
        Vec3 l = h.getLocation();
        j.key("location").beginArray().f64(l.x).f64(l.y).f64(l.z).endArray();
        if (h instanceof BlockHitResult b) {
            BlockPos p = b.getBlockPos();
            j.key("block").beginArray().value(p.getX()).value(p.getY()).value(p.getZ()).endArray();
            j.field("face", b.getDirection().getName());
            j.field("inside", b.isInside());
        }
        j.endObject();
    }
}
