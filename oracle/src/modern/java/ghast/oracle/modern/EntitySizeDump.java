package ghast.oracle.modern;

import ghast.oracle.Json;
import ghast.oracle.Scenario;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityDimensions;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.InterpolationHandler;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.Mob;
import net.minecraft.world.entity.Pose;
import net.minecraft.world.entity.decoration.ArmorStand;
import net.minecraft.world.phys.AABB;

import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.List;
import java.util.Objects;
import java.util.function.Consumer;
import net.minecraft.network.syncher.EntityDataSerializers;
import net.minecraft.network.syncher.SynchedEntityData;

/**
 * Dimensions and eye heights of every 26.1 entity type: the type's registered dimensions, and for an
 * instance created on the client (as an add-entity packet does: {@code EntityType.create(level, LOAD)})
 * the dimensions and eye height in every pose,
 * plus the metadata-dependent variants: babies, slime / magma cube sizes, armor stand small / marker,
 * and the client player. The default line also says how the client instance moves on position packets
 * (living, interpolation steps) and whether it can be picked; each variant line lists the synched data
 * entries that differ from the default instance (the metadata that selects the variant). Schema:
 * oracle/README.md, "entity_sizes.jsonl.gz".
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
            if (type == Reg.entity("player")) {
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
            List<SynchedEntityData.DataValue<?>> base = all(e);
            write(sink, id, name, "default", e, null);
            try {
                if (e instanceof Mob m && hasSetBaby(m)) {
                    m.setBaby(true);
                    write(sink, id, name, "baby", e, base);
                    if (Compat.isCubeMob(e)) {
                        // 26.2: cube mobs have a baby form and still a size.
                        m.setBaby(false);
                        for (int size : new int[] {1, 2, 3, 4}) {
                            Compat.setCubeSize(e, size);
                            write(sink, id, name, "size_" + size, e, base);
                        }
                    }
                } else if (Compat.isCubeMob(e)) {
                    for (int size : new int[] {1, 2, 3, 4}) {
                        Compat.setCubeSize(e, size);
                        write(sink, id, name, "size_" + size, e, base);
                    }
                } else if (e instanceof ArmorStand a) {
                    Method small = ArmorStand.class.getDeclaredMethod("setSmall", boolean.class);
                    Method marker = ArmorStand.class.getDeclaredMethod("setMarker", boolean.class);
                    small.setAccessible(true);
                    marker.setAccessible(true);
                    small.invoke(a, true);
                    write(sink, id, name, "small", e, base);
                    marker.invoke(a, true);
                    write(sink, id, name, "small_marker", e, base);
                    small.invoke(a, false);
                    write(sink, id, name, "marker", e, base);
                }
            } catch (ReflectiveOperationException | RuntimeException ex) {
                sink.accept(new Json().beginObject().field("type", "entity_size").field("id", id).field("name", name)
                        .field("variant", "variants").field("error", ex.toString()).endObject().toString());
            }
        }
        write(sink, BuiltInRegistries.ENTITY_TYPE.getId(Reg.entity("player")), "minecraft:player (client player)", "default", run.player, null);
    }

    /** Every synched data entry of an entity (index order). */
    static List<SynchedEntityData.DataValue<?>> all(Entity e) {
        List<SynchedEntityData.DataValue<?>> out = new ArrayList<>();
        for (SynchedEntityData.DataItem<?> item : (SynchedEntityData.DataItem<?>[]) Reflect.get(SynchedEntityData.class, e.getEntityData(), "itemsById")) {
            out.add(item.value());
        }
        return out;
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

    /**
     * One size line. {@code base} = the default instance's packed data (variant lines list the entries
     * that differ from it as {@code [index, serializer id, value]}); {@code null} for the default line,
     * which instead says how the instance follows position packets and whether it is pickable.
     */
    private static void write(Consumer<String> sink, int id, String name, String variant, Entity e,
            List<SynchedEntityData.DataValue<?>> base) {
        Json j = new Json().beginObject().field("type", "entity_size").field("id", id).field("name", name)
                .field("variant", variant);
        if (base == null) {
            InterpolationHandler h = e.getInterpolation();
            j.field("living", e instanceof LivingEntity)
                    .field("interpolation", h == null ? -1 : Reflect.getInt(InterpolationHandler.class, h, "interpolationSteps"))
                    .field("pickable", e.isPickable());
            if (e instanceof Mob m) {
                Object control = Reflect.get(Mob.class, m, "bodyRotationControl");
                j.field("mob", true).field("max_head_y_rot", m.getMaxHeadYRot())
                        .field("body_control", control.getClass().getSimpleName());
            }
        } else {
            j.key("meta").beginArray();
            for (SynchedEntityData.DataValue<?> v : all(e)) {
                SynchedEntityData.DataValue<?> old = null;
                for (SynchedEntityData.DataValue<?> b : base) {
                    if (b.id() == v.id()) {
                        old = b;
                    }
                }
                if (old == null || !Objects.equals(old.value(), v.value())) {
                    j.beginArray().value(v.id()).value(EntityDataSerializers.getSerializedId(v.serializer()))
                            .value(String.valueOf(v.value())).endArray();
                }
            }
            j.endArray();
        }
        EntityDimensions cur = e.getDimensions(e.getPose());
        j.fieldF32("width", e.getBbWidth()).fieldF32("height", e.getBbHeight()).fieldF32("eye_height", e.getEyeHeight());
        j.field("pose", e.getPose().name().toLowerCase()).field("fixed", cur.fixed());
        AABB b = e.getBoundingBox();
        j.key("box").beginArray().f64(b.minX).f64(b.minY).f64(b.minZ).f64(b.maxX).f64(b.maxY).f64(b.maxZ).endArray();
        j.fieldF32("targeting_margin", e.getPickRadius());
        j.key("poses").beginObject();
        // Each pose as the client holds it after a pose update (setPose refreshes the cached
        // dimensions; some types look at their own pose, e.g. the digging warden), with `fixed`.
        Pose was = e.getPose();
        for (Pose p : Pose.values()) {
            e.setPose(p);
            EntityDimensions d = e.getDimensions(e.getPose());
            j.key(p.name().toLowerCase()).beginArray().f32(e.getBbWidth()).f32(e.getBbHeight()).f32(e.getEyeHeight())
                    .value(d.fixed()).endArray();
        }
        e.setPose(was);
        j.endObject();
        sink.accept(j.endObject().toString());
    }
}
