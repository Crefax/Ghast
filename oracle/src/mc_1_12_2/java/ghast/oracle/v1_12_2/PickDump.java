package ghast.oracle.v1_12_2;

import ghast.oracle.Json;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import net.minecraft.block.Block;
import net.minecraft.block.BlockState;
import net.minecraft.client.MinecraftClient;
import net.minecraft.client.render.GameRenderer;
import net.minecraft.entity.Entity;
import net.minecraft.util.hit.BlockHitResult;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.Box;
import net.minecraft.util.math.Vec3d;
import net.minecraft.util.profiler.Profiler;
import net.minecraft.world.World;
import net.minecraft.world.GameMode;

import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;

/**
 * 1.12.2 crosshair picks in random worlds (golden {@code picks.jsonl.gz}): random states of every
 * block (partial shapes, neighbour-dependent bounds, fluids, stairs), client-side entities, random
 * viewpoints (inside blocks and entities too), random reach for {@code Entity.rayTrace}, and the real
 * {@code GameRenderer.updateTargetedEntity(1.0F)} target per game mode (block, miss, or entity).
 * Schema: oracle/README.md, "Crosshair pick (1.8.9, 1.12.2): picks.jsonl.gz".
 */
final class PickDump {
    static final int WORLDS = 16, RAYS = 200;
    static final int X0 = -6, X1 = 6, Y0 = 59, Y1 = 71, Z0 = -6, Z1 = 6;

    private PickDump() {}

    private static final String[] ENTITIES = {"passive.PigEntity", "passive.ChickenEntity", "mob.ZombieEntity",
            "passive.VillagerEntity", "decoration.ArmorStandEntity", "vehicle.BoatEntity", "vehicle.MinecartEntity",
            "EndCrystalEntity", "mob.SlimeEntity", "ItemEntity", "ExperienceOrbEntity", "passive.SquidEntity",
            "mob.CreeperEntity", "passive.SheepEntity"};

    static void picks(long seed, int worlds, int rays, Consumer<String> sink) throws Exception {
        Harness.init();
        Scenario sc = new Scenario("pick_dump").spawn(0.5, 65, 0.5, 0, 0);
        Harness.Run run = new Harness.Run(sc);
        MinecraftClient client = MinecraftClient.getInstance();
        GameRenderer renderer = Reflect.allocate(GameRenderer.class);
        Reflect.set(GameRenderer.class, renderer, "client", client);
        if (client.profiler == null) {
            Reflect.set(MinecraftClient.class, client, "profiler", new Profiler());
        }
        List<Integer> raws = new ArrayList<>();
        for (int raw = 16; raw < 4096; raw++) {
            BlockState s = Block.BLOCK_STATES.fromId(raw);
            if (s != null && Block.getIdByBlock(s.getBlock()) == raw >> 4 && s.getBlock().getData(s) == (raw & 15)) {
                raws.add(raw);
            }
        }
        int[] partial = {44, 126, 53, 67, 108, 109, 114, 128, 134, 135, 136, 156, 163, 164, 180, 85, 113, 188, 189, 190, 191, 192,
                101, 102, 160, 139, 64, 71, 193, 194, 195, 196, 197, 54, 146, 96, 167, 107, 183, 184, 185, 186, 187, 78, 171, 65,
                106, 50, 75, 76, 69, 77, 143, 70, 72, 147, 148, 131, 132, 55, 93, 94, 149, 150, 66, 27, 28, 157, 26, 92, 140, 144,
                118, 117, 116, 120, 145, 154, 127, 81, 83, 88, 60, 30, 31, 32, 37, 38, 39, 40, 6, 59, 141, 142, 104, 105, 115, 8, 9,
                10, 11, 151, 178, 63, 68, 176, 177, 34, 29, 33, 36, 90, 119, 122, 198, 199, 200, 203, 205, 207, 208, 212, 213, 216,
                217, 218, 219, 225, 234, 255};
        List<Integer> partialRaws = new ArrayList<>();
        for (int r : raws) {
            for (int p : partial) {
                if (r >> 4 == p) {
                    partialRaws.add(r);
                }
            }
        }
        for (int w = 0; w < worlds; w++) {
            Rng rng = new Rng(seed * 1_000_003L + w ^ 0x7069636b73L);
            double density = 0.05 + rng.nextDouble() * 0.45;
            double partialShare = rng.nextDouble();
            Json world = new Json().beginObject().field("type", "pick_world").field("world", w);
            world.key("region").beginArray().value(X0).value(Y0).value(Z0).value(X1).value(Y1).value(Z1).endArray();
            world.key("blocks").beginArray();
            for (int x = X0; x <= X1; x++) {
                for (int y = Y0; y <= Y1; y++) {
                    for (int z = Z0; z <= Z1; z++) {
                        int raw = 0;
                        if (rng.chance(density)) {
                            raw = rng.chance(partialShare) ? partialRaws.get(rng.nextInt(partialRaws.size())) : raws.get(rng.nextInt(raws.size()));
                        }
                        run.setRaw(x, y, z, raw);
                        if (raw != 0) {
                            world.beginArray().value(x).value(y).value(z).value(raw).endArray();
                        }
                    }
                }
            }
            world.endArray();
            List<Entity> spawned = spawn(run.world, rng, 1000 + 100 * w);
            world.key("entities").beginArray();
            // The world's iteration order. Entities of earlier worlds stay in the chunk lists after
            // ClientWorld.removeEntity (removed, so living ones no longer collide; end crystals do).
            for (Entity e : run.world.getEntitiesIn(run.player, new Box(-1e4, -1e4, -1e4, 1e4, 1e4, 1e4), x -> true)) {
                Box b = e.getBoundingBox();
                world.beginObject().field("id", e.getEntityId()).field("class", e.getClass().getSimpleName());
                world.key("bb").beginArray().f64(b.minX).f64(b.minY).f64(b.minZ).f64(b.maxX).f64(b.maxY).f64(b.maxZ).endArray();
                world.fieldF32("margin", e.getTargetingMargin()).field("pickable", e.collides() && net.minecraft.entity.predicate.EntityPredicate.EXCEPT_SPECTATOR.apply(e));
                world.endObject();
            }
            world.endArray();
            sink.accept(world.endObject().toString());

            for (int k = 0; k < rays; k++) {
                double x = X0 - 1 + rng.nextDouble() * (X1 - X0 + 3);
                double y = Y0 - 1 + rng.nextDouble() * (Y1 - Y0 + 1);
                double z = Z0 - 1 + rng.nextDouble() * (Z1 - Z0 + 3);
                float yaw = (float) (rng.nextDouble() * 720 - 360);
                float pitch = (float) (rng.nextDouble() * 180 - 90);
                if (rng.chance(0.1)) {
                    yaw = 45.0F * rng.range(-8, 8);
                }
                if (rng.chance(0.08)) {
                    pitch = rng.chance(0.5) ? 90.0F : -90.0F;
                }
                if (!spawned.isEmpty() && rng.chance(0.45)) {
                    Entity t = spawned.get(rng.nextInt(spawned.size()));
                    if (rng.chance(0.15)) {
                        x = t.x;
                        y = t.y + (rng.nextDouble() - 0.5) * 0.6;
                        z = t.z;
                    }
                    Box tb = t.getBoundingBox();
                    double tx = tb.minX - 0.2 + rng.nextDouble() * (tb.maxX - tb.minX + 0.4);
                    double ty = tb.minY - 0.2 + rng.nextDouble() * (tb.maxY - tb.minY + 0.4);
                    double tz = tb.minZ - 0.2 + rng.nextDouble() * (tb.maxZ - tb.minZ + 0.4);
                    double dx = tx - x, dy = ty - (y + 1.62), dz = tz - z;
                    yaw = (float) Math.toDegrees(Math.atan2(-dx, dz));
                    pitch = (float) Math.toDegrees(-Math.atan2(dy, Math.sqrt(dx * dx + dz * dz)));
                }
                float headYaw = rng.chance(0.7) ? yaw : yaw + (float) ((rng.nextDouble() - 0.5) * 40);
                boolean sneaking = rng.chance(0.2);
                boolean creative = rng.chance(0.3);
                double rr = rng.nextDouble();
                double reach = rr < 0.4 ? 4.5 : rr < 0.6 ? 5.0 : rr < 0.65 ? (rng.chance(0.5) ? 0.0 : 64.0) : rng.nextDouble() * 12;

                run.player.updatePosition(x, y, z);
                run.player.yaw = yaw;
                run.player.pitch = pitch;
                run.player.headYaw = headYaw;
                Harness.setKeys(sneaking ? Scenario.SNEAK : 0);
                run.player.input.tick();
                client.interactionManager.setGameMode(creative ? GameMode.CREATIVE : GameMode.SURVIVAL);

                BlockHitResult raw = run.player.rayTrace(reach, 1.0F);
                client.result = null;
                client.targetedEntity = null;
                renderer.updateTargetedEntity(1.0F);
                Vec3d eyes = run.player.getCameraPosVec(1.0F);
                Vec3d look = run.player.getRotationVector(1.0F);
                Box pb = run.player.getBoundingBox();

                Json j = new Json().beginObject().field("type", "pick").field("k", k);
                j.key("feet").beginArray().f64(run.player.x).f64(run.player.y).f64(run.player.z).endArray();
                j.fieldF32("yaw", yaw).fieldF32("pitch", pitch).fieldF32("head_yaw", headYaw);
                j.field("sneaking", run.player.isSneaking()).fieldF32("eye_height", run.player.getEyeHeight());
                j.field("creative", creative).fieldF64("reach", reach);
                j.key("bb").beginArray().f64(pb.minX).f64(pb.minY).f64(pb.minZ).f64(pb.maxX).f64(pb.maxY).f64(pb.maxZ).endArray();
                j.key("eyes").beginArray().f64(eyes.x).f64(eyes.y).f64(eyes.z).endArray();
                j.key("look").beginArray().f64(look.x).f64(look.y).f64(look.z).endArray();
                j.key("raw");
                hit(j, raw);
                j.key("target");
                hit(j, client.result);
                j.field("targeted_entity", client.targetedEntity == null ? -1 : client.targetedEntity.getEntityId());
                sink.accept(j.endObject().toString());
            }
            Harness.releaseKeys();
            for (Entity e : spawned) {
                run.world.removeEntity(e.getEntityId());
            }
            for (int x = X0; x <= X1; x++) {
                for (int y = Y0; y <= Y1; y++) {
                    for (int z = Z0; z <= Z1; z++) {
                        run.setRaw(x, y, z, 0);
                    }
                }
            }
        }
        client.interactionManager.setGameMode(GameMode.SURVIVAL);
    }

    private static List<Entity> spawn(World world, Rng rng, int firstId) throws Exception {
        List<Entity> out = new ArrayList<>();
        int n = rng.range(4, 22);
        for (int i = 0; i < n; i++) {
            String cls = ENTITIES[rng.nextInt(ENTITIES.length)];
            Entity e = (Entity) Class.forName("net.minecraft.entity." + cls).getConstructor(World.class).newInstance(world);
            double x = X0 - 1 + rng.nextDouble() * (X1 - X0 + 3);
            double y = Y0 - 1 + rng.nextDouble() * (Y1 - Y0 + 1);
            double z = Z0 - 1 + rng.nextDouble() * (Z1 - Z0 + 3);
            if (!out.isEmpty() && rng.chance(0.25)) {
                Entity prev = out.get(out.size() - 1);
                x = prev.x + (rng.chance(0.5) ? 0.0 : (rng.nextDouble() - 0.5) * 0.4);
                y = prev.y;
                z = prev.z;
            }
            e.yaw = (float) (rng.nextDouble() * 360);
            e.updatePosition(x, y, z);
            ((net.minecraft.client.world.ClientWorld) world).addEntity(firstId + i, e);
            out.add(e);
        }
        return out;
    }

    private static void hit(Json j, BlockHitResult h) {
        j.beginObject();
        if (h == null) {
            j.field("type", "null");
        } else {
            j.field("type", h.type.name().toLowerCase());
            j.key("hit").beginArray().f64(h.pos.x).f64(h.pos.y).f64(h.pos.z).endArray();
            if (h.type == BlockHitResult.Type.ENTITY) {
                j.field("entity", h.entity.getEntityId());
            } else {
                BlockPos p = h.getBlockPos();
                j.key("block").beginArray().value(p.getX()).value(p.getY()).value(p.getZ()).endArray();
                j.field("side", h.direction == null ? null : h.direction.name().toLowerCase());
            }
        }
        j.endObject();
    }
}
