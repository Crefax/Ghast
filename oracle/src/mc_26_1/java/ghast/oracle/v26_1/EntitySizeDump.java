package ghast.oracle.v26_1;

import ghast.oracle.Json;
import ghast.oracle.Scenario;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityDimensions;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.Mob;
import net.minecraft.world.entity.Pose;
import net.minecraft.world.entity.decoration.ArmorStand;
import net.minecraft.world.entity.monster.Slime;
import net.minecraft.world.phys.AABB;

import java.lang.reflect.Method;
import java.util.function.Consumer;

/**
 * Dimensions and eye heights of every 26.1 entity type: the type's registered dimensions, and for an
 * instance created on the client (as an add-entity packet does: {@code EntityType.create(level, LOAD)})
 * the dimensions and eye height in every pose,
 * plus the metadata-dependent variants: babies, slime / magma cube sizes, armor stand small / marker,
 * and the client player. Schema: oracle/README.md, "entity_sizes.jsonl.gz".
 */
final class EntitySizeDump {
    private EntitySizeDump() {}

    static void lines(Consumer<String> sink) {
        Scenario sc = new Scenario("entity_sizes").spawn(0.5, 100, 0.5, 0, 0);
        sc.fill(-2, 60, -2, 2, 60, 2, 0, 0);
        Harness.Run run = new Harness.Run(sc);
        for (EntityType<?> type : BuiltInRegistries.ENTITY_TYPE) {
            int id = BuiltInRegistries.ENTITY_TYPE.getId(type);
            String name = BuiltInRegistries.ENTITY_TYPE.getKey(type).toString();
            EntityDimensions d = type.getDimensions();
            Json t = new Json().beginObject().field("type", "entity_type").field("id", id).field("name", name);
            t.fieldF32("width", d.width()).fieldF32("height", d.height()).fieldF32("eye_height", d.eyeHeight()).field("fixed", d.fixed());
            sink.accept(t.endObject().toString());
            if (type == EntityType.PLAYER) {
                continue; // a remote player is a RemotePlayer, measured like the client player below
            }
            Entity e;
            try {
                e = type.create(run.level, EntitySpawnReason.LOAD);
            } catch (RuntimeException ex) {
                e = null;
            }
            if (e == null) {
                continue;
            }
            e.setPos(0.5, 64, 0.5);
            write(sink, id, name, "default", e);
            try {
                if (e instanceof Mob m && hasSetBaby(m)) {
                    m.setBaby(true);
                    write(sink, id, name, "baby", e);
                } else if (e instanceof Slime s) {
                    for (int size : new int[] {1, 2, 3, 4}) {
                        s.setSize(size, true);
                        write(sink, id, name, "size_" + size, e);
                    }
                } else if (e instanceof ArmorStand a) {
                    Method small = ArmorStand.class.getDeclaredMethod("setSmall", boolean.class);
                    Method marker = ArmorStand.class.getDeclaredMethod("setMarker", boolean.class);
                    small.setAccessible(true);
                    marker.setAccessible(true);
                    small.invoke(a, true);
                    write(sink, id, name, "small", e);
                    marker.invoke(a, true);
                    write(sink, id, name, "small_marker", e);
                    small.invoke(a, false);
                    write(sink, id, name, "marker", e);
                }
            } catch (ReflectiveOperationException | RuntimeException ex) {
                sink.accept(new Json().beginObject().field("type", "entity_size").field("id", id).field("name", name)
                        .field("variant", "variants").field("error", ex.toString()).endObject().toString());
            }
        }
        write(sink, BuiltInRegistries.ENTITY_TYPE.getId(EntityType.PLAYER), "minecraft:player (client player)", "default", run.player);
    }

    /** Only the mobs that implement a baby form (Mob.setBaby is a no-op by default). */
    private static boolean hasSetBaby(Mob m) {
        for (Class<?> c = m.getClass(); c != Mob.class && c != null; c = c.getSuperclass()) {
            try {
                c.getDeclaredMethod("setBaby", boolean.class);
                return true;
            } catch (NoSuchMethodException ignored) {
                // keep looking
            }
        }
        return false;
    }

    private static void write(Consumer<String> sink, int id, String name, String variant, Entity e) {
        Json j = new Json().beginObject().field("type", "entity_size").field("id", id).field("name", name)
                .field("variant", variant);
        EntityDimensions cur = e.getDimensions(e.getPose());
        j.fieldF32("width", e.getBbWidth()).fieldF32("height", e.getBbHeight()).fieldF32("eye_height", e.getEyeHeight());
        j.field("pose", e.getPose().name().toLowerCase()).field("fixed", cur.fixed());
        AABB b = e.getBoundingBox();
        j.key("box").beginArray().f64(b.minX).f64(b.minY).f64(b.minZ).f64(b.maxX).f64(b.maxY).f64(b.maxZ).endArray();
        j.fieldF32("targeting_margin", e.getPickRadius());
        j.key("poses").beginObject();
        for (Pose p : Pose.values()) {
            EntityDimensions d = e.getDimensions(p);
            j.key(p.name().toLowerCase()).beginArray().f32(d.width()).f32(d.height()).f32(e.getEyeHeight(p)).endArray();
        }
        j.endObject();
        sink.accept(j.endObject().toString());
    }
}
