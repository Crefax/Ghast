package ghast.oracle.v1_16_5;

import ghast.oracle.Json;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import it.unimi.dsi.fastutil.doubles.DoubleList;
import net.minecraft.block.Block;
import net.minecraft.block.BlockState;
import net.minecraft.block.ShapeContext;
import net.minecraft.entity.Entity;
import net.minecraft.entity.EntityPose;
import net.minecraft.entity.EntityType;
import net.minecraft.world.GameMode;
import net.minecraft.item.Item;
import net.minecraft.item.ItemStack;
import net.minecraft.item.Items;
import net.minecraft.util.Hand;
import net.minecraft.util.Identifier;
import net.minecraft.util.hit.BlockHitResult;
import net.minecraft.util.hit.HitResult;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.Box;
import net.minecraft.util.math.Direction;
import net.minecraft.util.math.Vec3d;
import net.minecraft.util.registry.Registry;
import net.minecraft.util.shape.SimpleVoxelShape;
import net.minecraft.util.shape.VoxelSet;
import net.minecraft.util.shape.VoxelShape;
import net.minecraft.world.chunk.ChunkSection;
import net.minecraft.world.chunk.WorldChunk;

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
 * 1.16.5 crosshair data.
 *
 * <p>{@code selection_shapes.jsonl.gz}: per state the shapes the block pick uses: the outline shape
 * ({@code BlockState.getOutlineShape(world, pos, ShapeContext.of(player))}, empty hand), the variants
 * with an absent context and while holding the block's own item where they differ (scaffolding), and
 * the raycast shape ({@code getRaycastShape}, which can only change the hit side). Shapes of offset
 * blocks (plants) are {@code base.offset(modelOffset(pos))}; the base shape is recorded.
 *
 * <p>{@code raytrace.jsonl.gz}: random worlds of random states, random viewpoints, and the real
 * {@code Entity.raycast(reach, 1.0F, false)} (what {@code GameRenderer.updateTargetedEntity} stores as
 * the crosshair target when no entity is nearer), with reach 4.5 / 5.0 and random ones. Schema:
 * oracle/README.md.
 */
final class SelectionDump {
    static final int RX = 0, RY = 64, RZ = 0;
    static final int PX = 3, PY = 66, PZ = 5;

    private SelectionDump() {}

    private static Method points;
    private static Field voxelsField;
    private static Field offsetDelegate;
    private static Constructor<?> arrayShape;

    private static void reflect() throws Exception {
        points = VoxelShape.class.getDeclaredMethod("getPointPositions", Direction.Axis.class);
        points.setAccessible(true);
        voxelsField = VoxelShape.class.getDeclaredField("voxels");
        voxelsField.setAccessible(true);
        offsetDelegate = net.minecraft.util.shape.OffsetDoubleList.class.getDeclaredField("oldList");
        offsetDelegate.setAccessible(true);
        Class<?> array = Class.forName("net.minecraft.util.shape.ArrayVoxelShape");
        arrayShape = array.getDeclaredConstructor(VoxelSet.class, DoubleList.class, DoubleList.class, DoubleList.class);
        arrayShape.setAccessible(true);
    }

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
            Json bj = new Json().beginArray();
            for (Box b : shape.getBoundingBoxes()) {
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

    /** The unshifted shape of an offset one, or {@code null} when the shape is not offset. */
    private static VoxelShape base(VoxelShape shape) throws Exception {
        if (shape.isEmpty()) {
            return null;
        }
        DoubleList[] base = new DoubleList[3];
        for (Direction.Axis axis : Direction.Axis.values()) {
            DoubleList list = (DoubleList) points.invoke(shape, axis);
            if (!(list instanceof net.minecraft.util.shape.OffsetDoubleList)) {
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

    private static void place(Harness.Run run, int x, int y, int z, BlockState state) {
        WorldChunk chunk = run.world.getChunk(x >> 4, z >> 4);
        ChunkSection[] sections = chunk.getSectionArray();
        if (sections[y >> 4] == null) {
            sections[y >> 4] = new ChunkSection((y >> 4) << 4);
        }
        sections[y >> 4].setBlockState(x & 15, y & 15, z & 15, state, false);
    }

    private static void hold(Harness.Run run, Item item) {
        run.player.setStackInHand(Hand.MAIN_HAND, item == null ? ItemStack.EMPTY : new ItemStack(item));
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
            place(run, RX, RY, RZ, state);
            hold(run, null);
            VoxelShape outline = state.getOutlineShape(run.world, pos, ShapeContext.of(run.player));
            VoxelShape outlineAbsent = state.getOutlineShape(run.world, pos, ShapeContext.absent());
            VoxelShape outlineHolding = null;
            if (own != Items.AIR) {
                hold(run, own);
                outlineHolding = state.getOutlineShape(run.world, pos, ShapeContext.of(run.player));
                hold(run, null);
            }
            VoxelShape raycast = state.getRaycastShape(run.world, pos);

            Json j = new Json().beginObject().field("type", "state").field("id", id).field("state", States.name(state));
            VoxelShape outlineBase = base(outline);
            j.field("outline", shapes.id(outline));
            int absentSid = shapes.id(outlineAbsent);
            if (absentSid != shapes.id(outline)) {
                j.field("outline_empty_context", absentSid);
            }
            if (outlineHolding != null && shapes.id(outlineHolding) != shapes.id(outline)) {
                j.field("outline_holding_own_item", shapes.id(outlineHolding));
            }
            j.field("interaction", shapes.id(raycast));
            if (base(raycast) != null) {
                throw new IllegalStateException("offset raycast shape: " + States.name(state));
            }
            if (outlineBase != null) {
                j.field("outline_base", shapes.id(outlineBase));
                Vec3d off = state.getModelOffset(run.world, pos);
                j.key("offset_at_dump").beginArray().f64(off.x).f64(off.y).f64(off.z).endArray();
            }
            place(run, RX, RY, RZ, States.byId(0));

            place(run, PX, PY, PZ, state);
            VoxelShape other = state.getOutlineShape(run.world, pos2, ShapeContext.of(run.player));
            VoxelShape otherBase = base(other);
            boolean same = outlineBase != null
                    ? otherBase != null && shapes.id(otherBase) == shapes.id(outlineBase)
                    : otherBase == null && shapes.id(other) == shapes.id(outline);
            same &= shapes.id(state.getRaycastShape(run.world, pos2)) == shapes.id(raycast);
            place(run, PX, PY, PZ, States.byId(0));
            j.field("position_independent", same);
            sink.accept(j.endObject().toString());
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Raytrace vectors

    static final int RAY_WORLDS = 16, RAY_RAYS = 200;
    static final int X0 = -6, X1 = 6, Y0 = 59, Y1 = 71, Z0 = -6, Z1 = 6;

    private static boolean excluded(Block b) {
        return b instanceof net.minecraft.block.ShulkerBoxBlock || b instanceof net.minecraft.block.PistonExtensionBlock;
    }

    private static final String[] PARTIAL = {"oak_stairs", "stone_slab", "oak_fence", "cobblestone_wall", "glass_pane", "iron_bars",
            "oak_door", "oak_trapdoor", "oak_fence_gate", "scaffolding", "hopper", "cauldron", "composter", "lectern", "bell",
            "chain", "lantern", "bamboo", "grass", "tall_grass", "poppy", "torch", "wall_torch", "oak_sign", "oak_wall_sign",
            "ladder", "vine", "snow", "white_carpet", "end_rod", "chest", "ender_chest", "anvil", "brewing_stand",
            "enchanting_table", "cake", "flower_pot", "sea_pickle", "turtle_egg", "grindstone", "stonecutter", "campfire",
            "soul_sand", "farmland", "grass_path", "honey_block", "cobweb", "barrier", "structure_void", "sweet_berry_bush",
            "kelp", "seagrass", "water", "lava", "bubble_column", "player_head", "dragon_head", "piston_head", "piston",
            "redstone_wire", "repeater", "comparator", "rail", "lever", "stone_button", "stone_pressure_plate", "tripwire",
            "tripwire_hook", "daylight_detector", "conduit", "crimson_roots", "warped_fungus", "nether_sprouts",
            "twisting_vines", "weeping_vines", "soul_lantern", "respawn_anchor", "lodestone", "target"};

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
            Identifier key = new Identifier("minecraft", name);
            if (!Registry.BLOCK.containsId(key)) {
                continue;
            }
            Block b = Registry.BLOCK.get(key);
            for (BlockState s : b.getStateManager().getStates()) {
                partial.add(States.id(s));
            }
        }
        partialStates = partial.stream().mapToInt(Integer::intValue).toArray();
    }

    static void rays(long seed, int worlds, int raysPerWorld, Consumer<String> sink) {
        Scenario sc = new Scenario("ray_dump").spawn(0.5, RY + 1, 0.5, 0, 0);
        Harness.Run run = new Harness.Run(sc);
        palettes();
        // A GameRenderer of our own for the crosshair update (the harness one is a bare stub).
        net.minecraft.client.render.GameRenderer renderer = Reflect.allocate(net.minecraft.client.render.GameRenderer.class);
        Reflect.set(net.minecraft.client.render.GameRenderer.class, renderer, "client", Harness.client);
        if (Harness.client.getProfiler() == null) {
            Reflect.set(net.minecraft.client.MinecraftClient.class, Harness.client, "profiler",
                    net.minecraft.util.profiler.DummyProfiler.INSTANCE);
        }
        EntityPose[] poses = {EntityPose.STANDING, EntityPose.CROUCHING, EntityPose.SWIMMING, EntityPose.FALL_FLYING,
                EntityPose.SPIN_ATTACK};
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
                        place(run, x, y, z, States.byId(id));
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
            List<Entity> spawned = spawnEntities(run, rng);
            world.key("entities").beginArray();
            for (Entity e : run.world.getOtherEntities(run.player, new Box(-1.0E4, -1.0E4, -1.0E4, 1.0E4, 1.0E4, 1.0E4), e -> true)) {
                Box b = e.getBoundingBox();
                world.beginObject().field("id", e.getEntityId()).field("type", Registry.ENTITY_TYPE.getId(e.getType()).toString());
                world.key("bb").beginArray().f64(b.minX).f64(b.minY).f64(b.minZ).f64(b.maxX).f64(b.maxY).f64(b.maxZ).endArray();
                world.fieldF32("pick_radius", e.getTargetingMargin()).field("pickable", !e.isSpectator() && e.collides());
                world.endObject();
            }
            world.endArray();
            sink.accept(world.endObject().toString());

            for (int k = 0; k < raysPerWorld; k++) {
                double x = X0 - 1 + rng.nextDouble() * (X1 - X0 + 3);
                double y = Y0 - 1 + rng.nextDouble() * (Y1 - Y0 + 1);
                double z = Z0 - 1 + rng.nextDouble() * (Z1 - Z0 + 3);
                if (rng.chance(0.1)) {
                    x = Math.floor(x);
                    z = rng.chance(0.5) ? Math.floor(z) : z + 0.5;
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
                EntityPose pose = rng.chance(0.6) ? EntityPose.STANDING : rng.pick(poses);
                double rr = rng.nextDouble();
                double reach = rr < 0.45 ? 4.5 : rr < 0.7 ? 5.0 : rr < 0.75 ? rng.pick(new Double[] {0.0, 1.0E-4, 64.0}) : rng.nextDouble() * 12.0;
                Item held = null;
                double rh = rng.nextDouble();
                if (rh < 0.08) {
                    held = Items.SCAFFOLDING;
                } else if (rh < 0.12) {
                    held = Items.STONE;
                }

                boolean creative = rng.chance(0.3);

                run.player.setPose(pose);
                run.player.setPosition(x, y, z);
                run.player.prevX = x;
                run.player.prevY = y;
                run.player.prevZ = z;
                run.player.yaw = yaw;
                run.player.pitch = pitch;
                hold(run, held);
                if (!spawned.isEmpty() && rng.chance(0.45)) {
                    // Aim at (or stand in) an entity so entity picks are common.
                    Entity t = spawned.get(rng.nextInt(spawned.size()));
                    if (rng.chance(0.15)) {
                        run.player.setPosition(t.getX(), t.getY() + (rng.nextDouble() - 0.5) * 0.6, t.getZ());
                    }
                    Box tb = t.getBoundingBox().expand(0.2);
                    double tx = tb.minX + rng.nextDouble() * (tb.maxX - tb.minX);
                    double ty = tb.minY + rng.nextDouble() * (tb.maxY - tb.minY);
                    double tz = tb.minZ + rng.nextDouble() * (tb.maxZ - tb.minZ);
                    Vec3d e0 = run.player.getCameraPosVec(1.0F);
                    double dx = tx - e0.x, dy = ty - e0.y, dz = tz - e0.z;
                    run.player.yaw = (float) Math.toDegrees(Math.atan2(-dx, dz));
                    run.player.pitch = (float) Math.toDegrees(-Math.atan2(dy, Math.sqrt(dx * dx + dz * dz)));
                }
                Harness.client.interactionManager.setGameMode(creative ? GameMode.CREATIVE : GameMode.SURVIVAL);
                renderer.updateTargetedEntity(1.0F);
                HitResult target = Harness.client.crosshairTarget;

                HitResult hit = run.player.raycast(reach, 1.0F, false);
                Vec3d eyes = run.player.getCameraPosVec(1.0F);
                Vec3d view = run.player.getRotationVec(1.0F);

                Json j = new Json().beginObject().field("type", "ray").field("k", k);
                j.key("feet").beginArray().f64(run.player.getX()).f64(run.player.getY()).f64(run.player.getZ()).endArray();
                j.fieldF32("yaw", run.player.yaw).fieldF32("pitch", run.player.pitch);
                j.field("pose", pose.name().toLowerCase());
                j.fieldF32("eye_height", run.player.getStandingEyeHeight());
                j.fieldF64("reach", reach);
                j.field("held", held == null ? null : Registry.ITEM.getId(held).toString());
                j.field("creative", creative);
                Box pb = run.player.getBoundingBox();
                j.key("bb").beginArray().f64(pb.minX).f64(pb.minY).f64(pb.minZ).f64(pb.maxX).f64(pb.maxY).f64(pb.maxZ).endArray();
                j.key("eyes").beginArray().f64(eyes.x).f64(eyes.y).f64(eyes.z).endArray();
                j.key("view").beginArray().f64(view.x).f64(view.y).f64(view.z).endArray();
                j.key("pick");
                hit(j, hit);
                j.key("target");
                hit(j, target);
                sink.accept(j.endObject().toString());
            }
            for (Entity e : spawned) {
                run.world.removeEntity(e.getEntityId());
            }
            for (int x = X0; x <= X1; x++) {
                for (int y = Y0; y <= Y1; y++) {
                    for (int z = Z0; z <= Z1; z++) {
                        place(run, x, y, z, States.byId(0));
                    }
                }
            }
        }
        hold(run, null);
        Harness.client.interactionManager.setGameMode(GameMode.SURVIVAL);
    }

    private static EntityType<?>[] entityTypes() {
        return new EntityType<?>[] {EntityType.ARMOR_STAND, EntityType.PIG, EntityType.CHICKEN, EntityType.ZOMBIE,
                EntityType.VILLAGER, EntityType.BOAT, EntityType.MINECART, EntityType.END_CRYSTAL, EntityType.ITEM_FRAME,
                EntityType.SLIME, EntityType.SHULKER, EntityType.ITEM, EntityType.EXPERIENCE_ORB, EntityType.ARROW};
    }

    private static List<Entity> spawnEntities(Harness.Run run, Rng rng) {
        List<Entity> out = new ArrayList<>();
        int n = rng.range(4, 22);
        for (int i = 0; i < n; i++) {
            EntityType<?>[] types = entityTypes();
            EntityType<?> type = types[rng.nextInt(types.length)];
            Entity e = type.create(run.world);
            if (e == null) {
                continue;
            }
            e.setEntityId(1000 + i);
            double x = X0 - 1 + rng.nextDouble() * (X1 - X0 + 3);
            double y = Y0 - 1 + rng.nextDouble() * (Y1 - Y0 + 1);
            double z = Z0 - 1 + rng.nextDouble() * (Z1 - Z0 + 3);
            if (rng.chance(0.2)) {
                x = Math.floor(x) + 0.5;
                y = Math.floor(y);
                z = Math.floor(z) + 0.5;
            }
            if (!out.isEmpty() && rng.chance(0.25)) {
                Entity prev = out.get(out.size() - 1);
                x = prev.getX() + (rng.chance(0.5) ? 0.0 : (rng.nextDouble() - 0.5) * 0.4);
                y = prev.getY();
                z = prev.getZ();
            }
            e.yaw = (float) (rng.nextDouble() * 360);
            e.setPosition(x, y, z);
            run.world.addEntity(1000 + i, e);
            out.add(e);
        }
        return out;
    }

    private static void hit(Json j, HitResult h) {
        j.beginObject();
        j.field("type", h.getType().name().toLowerCase());
        Vec3d l = h.getPos();
        j.key("location").beginArray().f64(l.x).f64(l.y).f64(l.z).endArray();
        if (h instanceof net.minecraft.util.hit.EntityHitResult) {
            j.field("entity", ((net.minecraft.util.hit.EntityHitResult) h).getEntity().getEntityId());
        }
        if (h instanceof BlockHitResult) {
            BlockHitResult b = (BlockHitResult) h;
            BlockPos p = b.getBlockPos();
            j.key("block").beginArray().value(p.getX()).value(p.getY()).value(p.getZ()).endArray();
            j.field("face", b.getSide().getName());
            j.field("inside", b.isInsideBlock());
        }
        j.endObject();
    }
}
