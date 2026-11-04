package ghast.oracle.v1_8_9;

import com.mojang.authlib.GameProfile;
import ghast.oracle.Json;
import ghast.oracle.Scenario;
import net.minecraft.block.Block;
import net.minecraft.entity.EndCrystalEntity;
import net.minecraft.entity.Entity;
import net.minecraft.entity.EntityType;
import net.minecraft.entity.ExperienceOrbEntity;
import net.minecraft.entity.FallingBlockEntity;
import net.minecraft.entity.FireworkRocketEntity;
import net.minecraft.entity.ItemEntity;
import net.minecraft.entity.LightningBoltEntity;
import net.minecraft.entity.LivingEntity;
import net.minecraft.entity.TntEntity;
import net.minecraft.entity.data.DataTracker;
import net.minecraft.entity.decoration.ArmorStandEntity;
import net.minecraft.entity.decoration.ItemFrameEntity;
import net.minecraft.entity.decoration.LeashKnotEntity;
import net.minecraft.entity.decoration.painting.PaintingEntity;
import net.minecraft.entity.mob.GuardianEntity;
import net.minecraft.entity.mob.SkeletonEntity;
import net.minecraft.entity.mob.SlimeEntity;
import net.minecraft.entity.mob.ZombieEntity;
import net.minecraft.entity.passive.PassiveEntity;
import net.minecraft.entity.player.OtherClientPlayerEntity;
import net.minecraft.entity.projectile.AbstractArrowEntity;
import net.minecraft.entity.projectile.FireballEntity;
import net.minecraft.entity.projectile.FishingBobberEntity;
import net.minecraft.entity.projectile.SmallFireballEntity;
import net.minecraft.entity.projectile.WitherSkullEntity;
import net.minecraft.entity.thrown.EggEntity;
import net.minecraft.entity.thrown.EnderPearlEntity;
import net.minecraft.entity.thrown.ExperienceBottleEntity;
import net.minecraft.entity.thrown.EyeOfEnderEntity;
import net.minecraft.entity.thrown.PotionEntity;
import net.minecraft.entity.thrown.SnowballEntity;
import net.minecraft.entity.vehicle.AbstractMinecartEntity;
import net.minecraft.entity.vehicle.BoatEntity;
import net.minecraft.item.ItemStack;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.Box;
import net.minecraft.util.math.Direction;
import net.minecraft.world.World;

import java.lang.reflect.Modifier;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.TreeMap;
import java.util.TreeSet;
import java.util.UUID;
import java.util.function.Consumer;
import java.util.function.Supplier;

/**
 * Width, height, eye height and box of every 1.8.9 entity as the CLIENT builds it from the packet that spawns
 * it, plus the metadata-dependent variants applied the way the client applies them.
 *
 * <ul>
 *   <li>Mobs (spawn_mob): {@code EntityType.createInstanceFromRawId(type, world)} then
 *       {@code updatePositionAndAngles}, exactly as {@code ClientPlayNetworkHandler.onMobSpawn}.</li>
 *   <li>Objects (spawn_entity): the per-object-type constructors of {@code onEntitySpawn} (they differ from
 *       the registry's {@code (World)} constructors, e.g. the XP orb and the item frame).</li>
 *   <li>XP orbs, paintings (per facing and motive), lightning (spawn_global_entity), other players
 *       (spawn_player, {@code OtherClientPlayerEntity}) and the client player.</li>
 *   <li>Variants: metadata goes through {@code DataTracker.writeUpdatedEntries} (the entity metadata packet
 *       path, which runs the entity's tracked-data hook). Where vanilla applies the size in a client tick
 *       method instead, that hook is run afterwards and the line says {@code "applied":"next_tick"}.</li>
 * </ul>
 * Every variant starts from a freshly spawned entity. Schema: oracle/README.md, "entity_sizes.jsonl.gz".
 */
final class EntitySizeDump {
    private EntitySizeDump() {}

    private static final double X = 0.5, Y = 64, Z = 0.5;
    private static final BlockPos BLOCK = new BlockPos(0, 64, 0);

    // DataTracker value types of the 1.8 entity metadata format.
    private static final int BYTE = 0, INT = 2;

    private record Line(int id, String name, String spawn) {}

    static void lines(Consumer<String> sink) {
        Scenario sc = new Scenario("entity_sizes").spawn(0.5, 100, 0.5, 0, 0);
        sc.fill(-2, 60, -2, 2, 60, 2, 0, 0);
        Harness.Run run = new Harness.Run(sc);
        World w = run.world;
        TreeSet<Integer> covered = new TreeSet<>();

        // 1. Mobs, by registry id.
        @SuppressWarnings("unchecked")
        Map<Integer, Class<? extends Entity>> idClass =
                new TreeMap<>((Map<Integer, Class<? extends Entity>>) Reflect.get(EntityType.class, null, "ID_CLASS_MAP"));
        for (var en : idClass.entrySet()) {
            int id = en.getKey();
            Class<? extends Entity> c = en.getValue();
            if (Modifier.isAbstract(c.getModifiers())) {
                covered.add(id); // "Mob" / "Monster": abstract, never spawned
                continue;
            }
            if (!LivingEntity.class.isAssignableFrom(c) || c == ArmorStandEntity.class) {
                continue; // objects: spawned through spawn_entity below
            }
            covered.add(id);
            mob(sink, w, id);
        }

        // 2. Objects (spawn_entity), in object type order, with the constructors of onEntitySpawn.
        object(sink, covered, 1, 0, null, () -> new BoatEntity(w, X, Y, Z));
        object(sink, covered, 2, 0, null, () -> new ItemEntity(w, X, Y, Z));
        for (AbstractMinecartEntity.Type t : AbstractMinecartEntity.Type.values()) {
            object(sink, covered, 10, t.getId(), null, () -> AbstractMinecartEntity.createMinecart(w, X, Y, Z, t));
        }
        object(sink, covered, 50, 0, null, () -> new TntEntity(w, X, Y, Z, (LivingEntity) null));
        object(sink, covered, 51, 0, null, () -> new EndCrystalEntity(w, X, Y, Z));
        object(sink, covered, 60, 0, null, () -> new AbstractArrowEntity(w, X, Y, Z));
        object(sink, covered, 61, 0, null, () -> new SnowballEntity(w, X, Y, Z));
        object(sink, covered, 62, 0, null, () -> new EggEntity(w, X, Y, Z));
        // Fireballs carry their acceleration in the packet velocity; any non-zero value, it does not size them.
        object(sink, covered, 63, 1, null, () -> new FireballEntity(w, X, Y, Z, 0, 0, 0.1));
        object(sink, covered, 64, 1, null, () -> new SmallFireballEntity(w, X, Y, Z, 0, 0, 0.1));
        object(sink, covered, 65, 0, null, () -> new EnderPearlEntity(w, X, Y, Z));
        object(sink, covered, 66, 1, null, () -> new WitherSkullEntity(w, X, Y, Z, 0, 0, 0.1));
        object(sink, covered, 70, 12, null, () -> new FallingBlockEntity(w, X, Y, Z, Block.getStateFromRawId(12)));
        for (int f = 0; f < 4; f++) {
            Direction d = Direction.fromHorizontal(f);
            object(sink, covered, 71, f, d, () -> new ItemFrameEntity(w, BLOCK, d));
        }
        object(sink, covered, 72, 0, null, () -> new EyeOfEnderEntity(w, X, Y, Z));
        object(sink, covered, 73, 16421, null, () -> new PotionEntity(w, X, Y, Z, 16421));
        object(sink, covered, 75, 0, null, () -> new ExperienceBottleEntity(w, X, Y, Z));
        object(sink, covered, 76, 0, null, () -> new FireworkRocketEntity(w, X, Y, Z, (ItemStack) null));
        object(sink, covered, 77, 0, null, () -> new LeashKnotEntity(w, BLOCK));
        object(sink, covered, 78, 0, null, () -> new ArmorStandEntity(w, X, Y, Z));
        // Packet data = the owner's entity id (the client spawns the bobber only when that entity is a player);
        // dumped as -1 because the id is run-dependent.
        object(sink, covered, 90, -1, null, () -> new FishingBobberEntity(w, X, Y, Z, run.player));

        // 3. XP orb (spawn_entity_experience_orb).
        Entity orb = new ExperienceOrbEntity(w, X, Y, Z, 1);
        covered.add(EntityType.getIdByEntity(orb));
        write(sink, base(orb, "xp_orb"), "default", null, orb);

        // 4. Paintings (spawn_entity_painting): box depends on facing and motive.
        for (PaintingEntity.PaintingMotive m : PaintingEntity.PaintingMotive.values()) {
            for (int f = 0; f < 4; f++) {
                Direction d = Direction.fromHorizontal(f);
                PaintingEntity p = new PaintingEntity(w, BLOCK, d, m.name);
                covered.add(EntityType.getIdByEntity(p));
                Json j = head(base(p, "painting"), "default", null);
                j.field("facing", d.name().toLowerCase(Locale.ROOT)).field("facing_id", f).field("motive", m.name)
                        .field("block_pos", "0,64,0");
                body(sink, j, p);
            }
        }

        // 5. Lightning (spawn_entity_weather): not in the registry.
        write(sink, new Line(-1, "LightningBolt", "global"), "default", null, new LightningBoltEntity(w, X, Y, Z));

        // 6. Players. Another player (spawn_player), then the client's own player.
        for (boolean sneak : new boolean[] {false, true}) {
            OtherClientPlayerEntity o = new OtherClientPlayerEntity(w, new GameProfile(new UUID(0, 2), "Other"));
            o.updatePositionAndAngles(X, Y, Z, 0, 0);
            if (sneak) {
                meta(o, BYTE, 0, (byte) 2); // entity flags bit 1: sneaking
            }
            write(sink, new Line(-1, "Player", "player"), sneak ? "sneaking" : "default", sneak ? "metadata" : null, o);
        }
        var p = run.player;
        p.updatePosition(X, Y, Z);
        write(sink, new Line(-1, "Player", "client_player"), "default", null, p);
        p.input.sneaking = true;
        write(sink, new Line(-1, "Player", "client_player"), "sneaking", "input", p);
        p.input.sneaking = false;

        // Every registry id must have been covered by one of the spawn paths above.
        for (int id : idClass.keySet()) {
            if (!covered.contains(id)) {
                sink.accept(new Json().beginObject().field("type", "entity_size").field("id", id)
                        .field("name", EntityType.getEntityName(id)).field("variant", "default")
                        .field("error", "registry id not covered by any client spawn path").endObject().toString());
            }
        }
    }

    // ---- mobs ----

    private static void mob(Consumer<String> sink, World w, int id) {
        Supplier<Entity> spawn = () -> {
            Entity e = EntityType.createInstanceFromRawId(id, w);
            if (e != null) {
                e.updatePositionAndAngles(X, Y, Z, 0, 0);
            }
            return e;
        };
        Entity e = spawn.get();
        String name = EntityType.getEntityName(id);
        if (e == null) {
            sink.accept(new Json().beginObject().field("type", "entity_size").field("id", id).field("name", name)
                    .field("spawn", "mob").field("variant", "default").field("error", "not constructible on the client")
                    .endObject().toString());
            return;
        }
        Line l = new Line(id, name, "mob");
        write(sink, l, "default", null, e);
        try {
            if (e instanceof PassiveEntity) {
                // Metadata 12 = -1 (baby); the client resizes in tickMovement (method_5377(isBaby())).
                PassiveEntity b = (PassiveEntity) spawn.get();
                meta(b, BYTE, 12, (byte) -1);
                b.method_5377(b.isBaby());
                write(sink, l, "baby", "next_tick", b);
            } else if (e instanceof ZombieEntity) {
                // The 1.8.9 client never resizes a zombie for metadata 12 (no tracked-data or tick hook): the
                // baby keeps the adult box. "baby_server" is ZombieEntity.setBaby, the size the server uses.
                Entity b = spawn.get();
                meta(b, BYTE, 12, (byte) 1);
                write(sink, l, "baby", "metadata", b);
                ZombieEntity s = (ZombieEntity) spawn.get();
                s.setBaby(true);
                write(sink, l, "baby_server", "server", s);
                Entity v = spawn.get();
                meta(v, BYTE, 13, (byte) 1);
                write(sink, l, "villager", "metadata", v);
            } else if (e instanceof SkeletonEntity) {
                // Metadata 13 = 1 (wither skeleton). The client sets the bounds in tickMovement; setType(1)
                // runs the same setBounds(0.72, 2.535) without moving the entity.
                SkeletonEntity s = (SkeletonEntity) spawn.get();
                meta(s, BYTE, 13, (byte) 1);
                s.setType(1);
                write(sink, l, "wither", "next_tick", s);
            } else if (e instanceof GuardianEntity) {
                Entity g = spawn.get();
                meta(g, INT, 16, 4); // flags bit 2: elder; the tracked-data hook resizes
                write(sink, l, "elder", "metadata", g);
            } else if (e instanceof SlimeEntity) {
                for (int size : new int[] {1, 2, 3, 4}) {
                    Entity s = spawn.get();
                    meta(s, BYTE, 16, (byte) size); // the tracked-data hook resizes
                    write(sink, l, "size_" + size, "metadata", s);
                }
            }
        } catch (RuntimeException ex) {
            sink.accept(new Json().beginObject().field("type", "entity_size").field("id", id).field("name", name)
                    .field("spawn", "mob").field("variant", "variants").field("error", ex.toString()).endObject().toString());
        }
    }

    // ---- objects ----

    private static void object(Consumer<String> sink, TreeSet<Integer> covered, int objectType, int data, Direction facing,
                               Supplier<Entity> make) {
        Entity e;
        try {
            e = make.get();
        } catch (RuntimeException ex) {
            sink.accept(new Json().beginObject().field("type", "entity_size").field("spawn", "object")
                    .field("object_type", objectType).field("data", data).field("variant", "default")
                    .field("error", ex.toString()).endObject().toString());
            return;
        }
        Line l = base(e, "object");
        if (l.id > 0) {
            covered.add(l.id);
        }
        write(sink, l, "default", null, e, objectType, data, facing);
        if (e instanceof ArmorStandEntity) {
            // Metadata 10 flags (1 small, 16 marker). The client applies the marker bounds in tick(). In 1.8.9
            // "small" halves only the eye height; the box stays 0.5 x 1.975.
            for (int flags : new int[] {1, 16, 17}) {
                ArmorStandEntity a = (ArmorStandEntity) make.get();
                meta(a, BYTE, 10, (byte) flags);
                a.tick();
                String v = flags == 1 ? "small" : flags == 16 ? "marker" : "small_marker";
                write(sink, l, v, "next_tick", a, objectType, data, facing);
            }
        }
    }

    // ---- output ----

    private static Line base(Entity e, String spawn) {
        int id = EntityType.getIdByEntity(e);
        String name = EntityType.getEntityName(e);
        if (e instanceof FishingBobberEntity) {
            name = "FishingBobber"; // not registered in 1.8.9
        }
        return new Line(id > 0 ? id : -1, name, spawn);
    }

    private static void meta(Entity e, int valueType, int index, Object value) {
        e.getDataTracker().writeUpdatedEntries(List.of(new DataTracker.DataEntry(valueType, index, value)));
    }

    private static Json head(Line l, String variant, String applied) {
        Json j = new Json().beginObject().field("type", "entity_size").field("id", l.id).field("name", l.name)
                .field("spawn", l.spawn).field("variant", variant);
        if (applied != null) {
            j.field("applied", applied);
        }
        return j;
    }

    private static void write(Consumer<String> sink, Line l, String variant, String applied, Entity e) {
        body(sink, head(l, variant, applied), e);
    }

    private static void write(Consumer<String> sink, Line l, String variant, String applied, Entity e,
                              int objectType, int data, Direction facing) {
        Json j = head(l, variant, applied).field("object_type", objectType).field("data", data);
        if (facing != null) {
            j.field("facing", facing.name().toLowerCase(Locale.ROOT)).field("block_pos", "0,64,0");
        }
        body(sink, j, e);
    }

    private static void body(Consumer<String> sink, Json j, Entity e) {
        j.key("pos").beginArray().f64(e.x).f64(e.y).f64(e.z).endArray();
        j.fieldF32("width", e.width).fieldF32("height", e.height).fieldF32("eye_height", e.getEyeHeight());
        Box b = e.getBoundingBox();
        j.key("box").beginArray().f64(b.minX).f64(b.minY).f64(b.minZ).f64(b.maxX).f64(b.maxY).f64(b.maxZ).endArray();
        j.fieldF32("collision_border", e.getTargetingMargin());
        sink.accept(j.endObject().toString());
    }
}
