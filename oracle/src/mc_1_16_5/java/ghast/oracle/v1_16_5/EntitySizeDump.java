package ghast.oracle.v1_16_5;

import ghast.oracle.Json;
import ghast.oracle.Scenario;
import net.minecraft.entity.Entity;
import net.minecraft.entity.EntityDimensions;
import net.minecraft.entity.EntityPose;
import net.minecraft.entity.EntityType;
import net.minecraft.entity.decoration.ArmorStandEntity;
import net.minecraft.entity.mob.MobEntity;
import net.minecraft.entity.mob.SlimeEntity;
import net.minecraft.util.math.Box;
import net.minecraft.util.registry.Registry;

import java.lang.reflect.Method;
import java.util.function.Consumer;

/**
 * Dimensions and eye heights of every 1.16.5 entity type: the type's registered dimensions, and for an
 * instance created on the client (as a spawn packet does) the dimensions and eye height in every pose,
 * plus the metadata-dependent variants: babies, slime / magma cube sizes, armor stand small / marker,
 * and the client player. Schema: oracle/README.md, "entity_sizes.jsonl.gz".
 */
final class EntitySizeDump {
    private EntitySizeDump() {}

    static void lines(Consumer<String> sink) {
        Scenario sc = new Scenario("entity_sizes").spawn(0.5, 100, 0.5, 0, 0);
        sc.fill(-2, 60, -2, 2, 60, 2, 0, 0);
        Harness.Run run = new Harness.Run(sc);
        for (EntityType<?> type : Registry.ENTITY_TYPE) {
            int id = Registry.ENTITY_TYPE.getRawId(type);
            String name = Registry.ENTITY_TYPE.getId(type).toString();
            EntityDimensions d = type.getDimensions();
            Json t = new Json().beginObject().field("type", "entity_type").field("id", id).field("name", name);
            t.fieldF32("width", d.width).fieldF32("height", d.height).field("fixed", d.fixed);
            sink.accept(t.endObject().toString());
            Entity e;
            try {
                e = type.create(run.world);
            } catch (RuntimeException ex) {
                e = null;
            }
            if (e == null) {
                continue;
            }
            e.updatePosition(0.5, 64, 0.5);
            write(sink, id, name, "default", e);
            try {
                if (e instanceof MobEntity m && hasSetBaby(m)) {
                    m.setBaby(true);
                    write(sink, id, name, "baby", e);
                } else if (e instanceof SlimeEntity s) {
                    Method set = SlimeEntity.class.getDeclaredMethod("setSize", int.class, boolean.class);
                    set.setAccessible(true);
                    for (int size : new int[] {1, 2, 3, 4}) {
                        set.invoke(s, size, true);
                        write(sink, id, name, "size_" + size, e);
                    }
                } else if (e instanceof ArmorStandEntity a) {
                    Method small = ArmorStandEntity.class.getDeclaredMethod("setSmall", boolean.class);
                    Method marker = ArmorStandEntity.class.getDeclaredMethod("setMarker", boolean.class);
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
        write(sink, Registry.ENTITY_TYPE.getRawId(EntityType.PLAYER), "minecraft:player (client player)", "default", run.player);
    }

    /** Only the mobs that implement a baby form (MobEntity.setBaby is a no-op by default). */
    private static boolean hasSetBaby(MobEntity m) {
        for (Class<?> c = m.getClass(); c != MobEntity.class && c != null; c = c.getSuperclass()) {
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
        j.fieldF32("width", e.getWidth()).fieldF32("height", e.getHeight()).fieldF32("eye_height", e.getStandingEyeHeight());
        j.field("pose", e.getPose().name().toLowerCase()).field("fixed", cur.fixed);
        Box b = e.getBoundingBox();
        j.key("box").beginArray().f64(b.minX).f64(b.minY).f64(b.minZ).f64(b.maxX).f64(b.maxY).f64(b.maxZ).endArray();
        j.fieldF32("targeting_margin", e.getTargetingMargin());
        j.key("poses").beginObject();
        for (EntityPose p : EntityPose.values()) {
            EntityDimensions d = e.getDimensions(p);
            j.key(p.name().toLowerCase()).beginArray().f32(d.width).f32(d.height).f32(e.getEyeHeight(p)).endArray();
        }
        j.endObject();
        sink.accept(j.endObject().toString());
    }
}
