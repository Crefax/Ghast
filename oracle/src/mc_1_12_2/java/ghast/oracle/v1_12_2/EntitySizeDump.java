package ghast.oracle.v1_12_2;

import com.mojang.authlib.GameProfile;
import ghast.oracle.Json;
import ghast.oracle.Scenario;
import net.minecraft.block.Block;
import net.minecraft.entity.player.OtherClientPlayerEntity;
import net.minecraft.entity.AreaEffectCloudEntity;
import net.minecraft.entity.EndCrystalEntity;
import net.minecraft.entity.Entity;
import net.minecraft.entity.EntityType;
import net.minecraft.entity.ExperienceOrbEntity;
import net.minecraft.entity.FallingBlockEntity;
import net.minecraft.entity.FireworkRocketEntity;
import net.minecraft.entity.ItemEntity;
import net.minecraft.entity.LightningBoltEntity;
import net.minecraft.entity.LivingEntity;
import net.minecraft.entity.ShulkerBulletEntity;
import net.minecraft.entity.TntEntity;
import net.minecraft.entity.data.TrackedData;
import net.minecraft.entity.decoration.ArmorStandEntity;
import net.minecraft.entity.decoration.ItemFrameEntity;
import net.minecraft.entity.decoration.LeashKnotEntity;
import net.minecraft.entity.decoration.painting.PaintingEntity;
import net.minecraft.entity.mob.EvokerFangsEntity;
import net.minecraft.entity.mob.SlimeEntity;
import net.minecraft.entity.mob.ZombieEntity;
import net.minecraft.entity.passive.PassiveEntity;
import net.minecraft.entity.projectile.ArrowEntity;
import net.minecraft.entity.projectile.DragonFireballEntity;
import net.minecraft.entity.projectile.FireballEntity;
import net.minecraft.entity.projectile.FishingBobberEntity;
import net.minecraft.entity.projectile.LlamaSpitEntity;
import net.minecraft.entity.projectile.SmallFireballEntity;
import net.minecraft.entity.projectile.SpectralArrowEntity;
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
import net.minecraft.util.Identifier;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.Box;
import net.minecraft.util.math.Direction;
import net.minecraft.world.World;

import java.lang.reflect.Modifier;
import java.util.Locale;
import java.util.TreeSet;
import java.util.UUID;
import java.util.function.Consumer;
import java.util.function.Supplier;

/**
 * Width, height, eye height and box of every 1.12.2 entity as the CLIENT builds it from the packet that
 * spawns it, plus the metadata-dependent variants applied the way the client applies them.
 *
 * <ul>
 *   <li>Mobs (spawn_mob): {@code EntityType.createInstanceFromRawId(type, world)} then
 *       {@code updatePositionAndAngles}, as {@code ClientPlayNetworkHandler.onMobSpawn}.</li>
 *   <li>Objects (spawn_entity): the per-object-type constructors of {@code onEntitySpawn}.</li>
 *   <li>XP orbs, paintings (per facing and motive), lightning (spawn_global_entity), other players and the
 *       client player (standing and sneaking: 1.9+ players are 1.65 high while sneaking).</li>
 *   <li>Variants: metadata goes through {@code DataTracker.writeUpdatedEntries} (the entity metadata packet
 *       path, which runs the entity's tracked-data hook). Where vanilla applies the size in a client tick, the tick is
 *       run afterwards and the line says {@code "applied":"next_tick"}.</li>
 * </ul>
 * {@code name} is the registry identifier's path (1.11+ names). Schema: oracle/README.md.
 */
final class EntitySizeDump {
    private EntitySizeDump() {}

    private static final double X = 0.5, Y = 64, Z = 0.5;
    private static final BlockPos BLOCK = new BlockPos(0, 64, 0);

    private record Line(int id, String name, String spawn) {}

    static void lines(Consumer<String> sink) {
        Scenario sc = new Scenario("entity_sizes").spawn(0.5, 100, 0.5, 0, 0);
        sc.fill(-2, 60, -2, 2, 60, 2, 0, 0);
        Harness.Run run = new Harness.Run(sc);
        World w = run.world;
        TreeSet<Integer> covered = new TreeSet<>();
        TreeSet<Integer> ids = new TreeSet<>();
        for (Class<? extends Entity> c : EntityType.REGISTRY) {
            ids.add(EntityType.REGISTRY.getRawId(c));
        }

        // 1. Mobs, by registry id.
        for (int id : ids) {
            Class<? extends Entity> c = EntityType.getEntityById(id);
            if (Modifier.isAbstract(c.getModifiers())) {
                covered.add(id);
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
        object(sink, covered, 3, 0, null, () -> new AreaEffectCloudEntity(w, X, Y, Z));
        for (AbstractMinecartEntity.Type t : AbstractMinecartEntity.Type.values()) {
            object(sink, covered, 10, t.getId(), null, () -> AbstractMinecartEntity.createMinecart(w, X, Y, Z, t));
        }
        object(sink, covered, 50, 0, null, () -> new TntEntity(w, X, Y, Z, null));
        object(sink, covered, 51, 0, null, () -> new EndCrystalEntity(w, X, Y, Z));
        object(sink, covered, 60, 0, null, () -> new ArrowEntity(w, X, Y, Z));
        object(sink, covered, 61, 0, null, () -> new SnowballEntity(w, X, Y, Z));
        object(sink, covered, 62, 0, null, () -> new EggEntity(w, X, Y, Z));
        object(sink, covered, 63, 1, null, () -> new FireballEntity(w, X, Y, Z, 0, 0, 0.1));
        object(sink, covered, 64, 1, null, () -> new SmallFireballEntity(w, X, Y, Z, 0, 0, 0.1));
        object(sink, covered, 65, 0, null, () -> new EnderPearlEntity(w, X, Y, Z));
        object(sink, covered, 66, 1, null, () -> new WitherSkullEntity(w, X, Y, Z, 0, 0, 0.1));
        object(sink, covered, 67, 0, null, () -> new ShulkerBulletEntity(w, X, Y, Z, 0, 0, 0.1));
        object(sink, covered, 68, 0, null, () -> new LlamaSpitEntity(w, X, Y, Z, 0, 0, 0.1));
        object(sink, covered, 70, 12, null, () -> new FallingBlockEntity(w, X, Y, Z, Block.getStateFromRawId(12)));
        for (int f = 0; f < 4; f++) {
            Direction d = Direction.fromHorizontal(f);
            object(sink, covered, 71, f, d, () -> new ItemFrameEntity(w, BLOCK, d));
        }
        object(sink, covered, 72, 0, null, () -> new EyeOfEnderEntity(w, X, Y, Z));
        object(sink, covered, 73, 0, null, () -> new PotionEntity(w, X, Y, Z, ItemStack.EMPTY));
        object(sink, covered, 75, 0, null, () -> new ExperienceBottleEntity(w, X, Y, Z));
        object(sink, covered, 76, 0, null, () -> new FireworkRocketEntity(w, X, Y, Z, ItemStack.EMPTY));
        object(sink, covered, 77, 0, null, () -> new LeashKnotEntity(w, BLOCK));
        object(sink, covered, 78, 0, null, () -> new ArmorStandEntity(w, X, Y, Z));
        object(sink, covered, 79, 0, null, () -> new EvokerFangsEntity(w, X, Y, Z, 0.0F, 0, null));
        // Packet data = the owner's entity id (the client spawns the bobber only when that entity is a player);
        // dumped as -1 because the id is run-dependent.
        object(sink, covered, 90, -1, null, () -> new FishingBobberEntity(w, run.player, X, Y, Z));
        object(sink, covered, 91, 0, null, () -> new SpectralArrowEntity(w, X, Y, Z));
        object(sink, covered, 93, 0, null, () -> new DragonFireballEntity(w, X, Y, Z, 0, 0, 0.1));

        // 3. XP orb (spawn_entity_experience_orb).
        Entity orb = new ExperienceOrbEntity(w, X, Y, Z, 1);
        covered.add(rawId(orb));
        write(sink, base(orb, "xp_orb"), "default", null, orb);

        // 4. Paintings (spawn_entity_painting): box depends on facing and motive.
        for (PaintingEntity.PaintingMotive m : PaintingEntity.PaintingMotive.values()) {
            for (int f = 0; f < 4; f++) {
                Direction d = Direction.fromHorizontal(f);
                PaintingEntity p = new PaintingEntity(w, BLOCK, d, m.name);
                covered.add(rawId(p));
                Json j = head(base(p, "painting"), "default", null);
                j.field("facing", d.name().toLowerCase(Locale.ROOT)).field("facing_id", f).field("motive", m.name)
                        .field("block_pos", "0,64,0");
                body(sink, j, p);
            }
        }

        // 5. Lightning (spawn_entity_weather): not in the registry.
        write(sink, new Line(-1, "lightning_bolt", "global"), "default", null, new LightningBoltEntity(w, X, Y, Z, false));

        // 6. Players. Another player (spawn_player), then the client's own player.
        for (boolean sneak : new boolean[] {false, true}) {
            OtherClientPlayerEntity o = new OtherClientPlayerEntity(w, new GameProfile(new UUID(0, 2), "Other"));
            o.updatePositionAndAngles(X, Y, Z, 0, 0);
            String applied = null;
            if (sneak) {
                // Entity flags bit 1 (metadata); the size follows in the next tick's size update.
                o.setSneaking(true);
                o.tick();
                applied = "next_tick";
            }
            write(sink, new Line(-1, "player", "player"), sneak ? "sneaking" : "default", applied, o);
        }
        var p = run.player;
        p.updatePosition(X, Y, Z);
        write(sink, new Line(-1, "player", "client_player"), "default", null, p);
        p.input.sneaking = true;
        // The client player resizes at the end of its tick (PlayerEntity.method_13266).
        Reflect.invoke(net.minecraft.entity.player.PlayerEntity.class, p, "method_13266");
        write(sink, new Line(-1, "player", "client_player"), "sneaking", "next_tick", p);
        p.input.sneaking = false;
        Reflect.invoke(net.minecraft.entity.player.PlayerEntity.class, p, "method_13266");

        // Every registry id must have been covered by one of the spawn paths above.
        for (int id : ids) {
            if (!covered.contains(id)) {
                sink.accept(new Json().beginObject().field("type", "entity_size").field("id", id)
                        .field("name", name(EntityType.REGISTRY.getIdentifier(EntityType.getEntityById(id)))).field("variant", "default")
                        .field("error", "registry id not covered by any client spawn path").endObject().toString());
            }
        }
    }

    private static int rawId(Entity e) {
        return EntityType.REGISTRY.getRawId(e.getClass());
    }

    private static String name(Identifier id) {
        return id == null ? "?" : id.getPath();
    }

    // ---- mobs ----

    /** The entity metadata packet path: {@code DataTracker.writeUpdatedEntries} runs the hook unconditionally. */
    @SuppressWarnings("unchecked")
    private static <T> void track(Entity e, Class<?> owner, String field, T value) {
        meta(e, (TrackedData<T>) Reflect.get(owner, null, field), value);
    }

    private static <T> void meta(Entity e, TrackedData<T> key, T value) {
        e.getDataTracker().writeUpdatedEntries(java.util.List.of(new net.minecraft.entity.data.DataTracker.DataEntry<>(key, value)));
    }

    private static void mob(Consumer<String> sink, World w, int id) {
        Supplier<Entity> spawn = () -> {
            Entity e = EntityType.createInstanceFromRawId(id, w);
            if (e != null) {
                e.updatePositionAndAngles(X, Y, Z, 0, 0);
            }
            return e;
        };
        Entity e = spawn.get();
        String name = name(EntityType.REGISTRY.getIdentifier(EntityType.getEntityById(id)));
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
                // The child flag's tracked-data hook resizes (PassiveEntity.method_5377).
                Entity b = spawn.get();
                track(b, PassiveEntity.class, "field_14476", true);
                write(sink, l, "baby", "metadata", b);
            } else if (e instanceof ZombieEntity) {
                // 1.9+: the baby flag's tracked-data hook resizes the zombie on the client too.
                Entity b = spawn.get();
                track(b, ZombieEntity.class, "field_14785", true);
                write(sink, l, "baby", "metadata", b);
            } else if (e instanceof SlimeEntity) {
                for (int size : new int[] {1, 2, 3, 4}) {
                    Entity s = spawn.get();
                    track(s, SlimeEntity.class, "field_14781", size);
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
        if (l.id >= 0) {
            covered.add(l.id);
        }
        write(sink, l, "default", null, e, objectType, data, facing);
        if (e instanceof ArmorStandEntity) {
            // Metadata flags (1 small, 16 marker); the client applies the bounds in tick().
            for (int flags : new int[] {1, 16, 17}) {
                ArmorStandEntity a = (ArmorStandEntity) make.get();
                meta(a, ArmorStandEntity.field_14724, (byte) flags);
                a.tick();
                String v = flags == 1 ? "small" : flags == 16 ? "marker" : "small_marker";
                write(sink, l, v, "next_tick", a, objectType, data, facing);
            }
        }
    }

    // ---- output ----

    private static Line base(Entity e, String spawn) {
        Identifier id = EntityType.getId(e);
        int raw = id == null ? -1 : rawId(e);
        return new Line(raw, name(id), spawn);
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
