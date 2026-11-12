package ghast.oracle.v26_1;

import com.google.gson.JsonObject;
import com.mojang.datafixers.util.Pair;
import ghast.oracle.Json;
import ghast.oracle.Scenario;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.protocol.game.ClientboundAddEntityPacket;
import net.minecraft.network.protocol.game.ClientboundRemoveEntitiesPacket;
import net.minecraft.network.protocol.game.ClientboundSetEntityDataPacket;
import net.minecraft.network.protocol.game.ClientboundSetEquipmentPacket;
import net.minecraft.network.syncher.EntityDataAccessor;
import net.minecraft.network.syncher.SynchedEntityData;
import net.minecraft.resources.Identifier;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.projectile.FireworkRocketEntity;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.phys.Vec3;

import java.util.ArrayList;
import java.util.List;
import java.util.UUID;

/**
 * Movement mechanics beyond the plain player (added for ghast 0.4.1): equipment that enables them, the
 * server packets that drive them, and their scenarios. Everything here goes through the real client:
 * equipment through {@code handleSetEquipment}, shared entity flags through {@code handleSetEntityData},
 * other entities through {@code handleAddEntity} / {@code handleRemoveEntities}; the client then ticks
 * them in the level's own order. A case uses this only when {@link Scenario#versionSetup} is a
 * {@link Setup}, so the bytes of every older golden file stay the same.
 */
final class Ext {
    private Ext() {}

    /** Version setup of a case (header key {@code ext} inside {@code setup}). */
    static final class Setup {
        /** Item in the chest slot ({@code elytra}), null for none. */
        String chest;
        /** Damage value of the chest item (an elytra at max damage - 1 cannot glide). */
        int chestDamage;
        /** Item in the selected hotbar slot for the use key instead of the shield ({@code firework_rocket}). */
        String hand;
        /** The server has the player gliding at the start (shared flag 7 in the setup entity data). */
        boolean fallFlying;
        /** Main arm left (the player's main hand entity data, from the client information). */
        boolean leftHanded;
        /** Riptide level of the hand item (a trident). */
        int riptide;
        /** It rains (the server's start-raining and rain-level game events at setup). */
        boolean raining;
        /** The case changes the world border (the level's border then ticks after the entities). */
        boolean border;
        /** Dimension type of the level ({@code the_nether}); null: the overworld. */
        String dimension;
        /**
         * Other entities spawned at setup (after the player, so they tick after it); non-null also when
         * empty: tick records then list every other entity ({@code entities}).
         */
        List<Spawn> entities;
    }

    static Setup setup(Scenario sc) {
        if (sc.versionSetup == null) {
            sc.versionSetup = new Setup();
        }
        return (Setup) sc.versionSetup;
    }

    static Setup of(Scenario sc) {
        return sc.versionSetup instanceof Setup s ? s : null;
    }

    // ---- events ------------------------------------------------------------------------------------

    /**
     * The server's entity data for the player's shared flags byte: the client's current byte with the
     * {@code clear} bits cleared and the {@code set} bits set (bit 3 sprinting, 4 swimming, 7 fall flying).
     */
    record SharedFlags(int set, int clear) implements Scenario.VersionEvent {}

    /**
     * The server's entity data for the player's living-entity flags byte (index 8): the client's current
     * byte with its item use (bits 1, 2) as the server sees it, then {@code clear} cleared and {@code set}
     * set (value 4: riptide spin attack).
     */
    record LivingFlags(int set, int clear) implements Scenario.VersionEvent {}

    /** Equipment packet for the player: {@code item} (null = empty) in {@code slot} with a damage value. */
    record Equip(String slot, String item, int damage) implements Scenario.VersionEvent {}

    /** A firework rocket the server spawned attached to the player (add entity + entity data). */
    record Firework(int id) implements Scenario.VersionEvent {}

    /** Remove entities packet. */
    record Remove(int id) implements Scenario.VersionEvent {}

    /** Set-border-center packet. */
    record BorderCenter(double x, double z) implements Scenario.VersionEvent {}

    /** Set-border-size packet (a static border). */
    record BorderSize(double size) implements Scenario.VersionEvent {}

    /** Set-border-lerp-size packet: from, to, over {@code ticks} game ticks. */
    record BorderLerp(double from, double to, long ticks) implements Scenario.VersionEvent {}

    /** An add-entity packet: {@code type} (registry name without namespace) at a position, no motion. */
    record Spawn(int id, String type, double x, double y, double z, float yaw) implements Scenario.VersionEvent {}

    /** A teleport-entity packet (absolute position, yaw, on ground): the client interpolates there. */
    record Move(int id, double x, double y, double z, float yaw, boolean onGround) implements Scenario.VersionEvent {}

    // ---- harness hooks -----------------------------------------------------------------------------

    static net.minecraft.resources.ResourceKey<net.minecraft.world.level.Level> dimensionKey(Scenario sc) {
        Setup s = of(sc);
        return s != null && "the_nether".equals(s.dimension) ? net.minecraft.world.level.Level.NETHER
                : net.minecraft.world.level.Level.OVERWORLD;
    }

    static net.minecraft.core.Holder<net.minecraft.world.level.dimension.DimensionType> dimensionType(Scenario sc) {
        Setup s = of(sc);
        if (s == null || s.dimension == null) {
            return Harness.overworld;
        }
        return Harness.registries.lookupOrThrow(net.minecraft.core.registries.Registries.DIMENSION_TYPE)
                .getOrThrow(net.minecraft.resources.ResourceKey.create(net.minecraft.core.registries.Registries.DIMENSION_TYPE,
                        Identifier.withDefaultNamespace(s.dimension)));
    }

    /** The hand item for the use key ({@code null}: the harness's shield). */
    static ItemStack handItem(Scenario sc) {
        Setup s = of(sc);
        if (s == null || s.hand == null) {
            return null;
        }
        ItemStack stack = new ItemStack(item(s.hand));
        if (s.riptide > 0) {
            stack.enchant(Harness.registries.lookupOrThrow(net.minecraft.core.registries.Registries.ENCHANTMENT)
                    .getOrThrow(net.minecraft.world.item.enchantment.Enchantments.RIPTIDE), s.riptide);
        }
        return stack;
    }

    /** Applied in setup after equipment, before the position packet. */
    static void applySetup(Harness.Run run) {
        Setup s = of(run.sc);
        if (s == null) {
            return;
        }
        if (s.chest != null) {
            equip(run, "chest", s.chest, s.chestDamage);
        }
        if (s.raining) {
            Harness.listener.handleGameEvent(Harness.roundTrip(net.minecraft.network.protocol.game.ClientboundGameEventPacket.STREAM_CODEC,
                    new net.minecraft.network.protocol.game.ClientboundGameEventPacket(
                            net.minecraft.network.protocol.game.ClientboundGameEventPacket.START_RAINING, 0.0F)));
            Harness.listener.handleGameEvent(Harness.roundTrip(net.minecraft.network.protocol.game.ClientboundGameEventPacket.STREAM_CODEC,
                    new net.minecraft.network.protocol.game.ClientboundGameEventPacket(
                            net.minecraft.network.protocol.game.ClientboundGameEventPacket.RAIN_LEVEL_CHANGE, 1.0F)));
        }
        if (s.leftHanded) {
            @SuppressWarnings("unchecked")
            EntityDataAccessor<net.minecraft.world.entity.HumanoidArm> arm = (EntityDataAccessor<net.minecraft.world.entity.HumanoidArm>)
                    Reflect.get(net.minecraft.world.entity.Avatar.class, null, "DATA_PLAYER_MAIN_HAND");
            Harness.listener.handleSetEntityData(Harness.roundTrip(ClientboundSetEntityDataPacket.STREAM_CODEC,
                    new ClientboundSetEntityDataPacket(Harness.PLAYER_ENTITY_ID,
                            List.of(SynchedEntityData.DataValue.create(arm, net.minecraft.world.entity.HumanoidArm.LEFT)))));
        }
    }

    /** Applied at the end of setup (after the scenario's state overrides). */
    static void finishSetup(Harness.Run run) {
        Setup s = of(run.sc);
        if (s != null && s.fallFlying) {
            sharedFlags(run, 0x80, 0);
        }
        if (s != null && s.entities != null) {
            for (Spawn sp : s.entities) {
                spawn(sp);
            }
        }
    }

    private static void spawn(Spawn s) {
        EntityType<?> type = BuiltInRegistries.ENTITY_TYPE.getOptional(Identifier.withDefaultNamespace(s.type()))
                .orElseThrow(() -> new IllegalArgumentException("entity " + s.type()));
        byte yRot = (byte) Math.floor(s.yaw() * 256.0F / 360.0F);
        ClientboundAddEntityPacket add = new ClientboundAddEntityPacket(s.id(), new UUID(0x656e74L, s.id()), s.x(), s.y(), s.z(),
                0.0F, s.yaw(), type, 0, Vec3.ZERO, s.yaw());
        Harness.listener.handleAddEntity(Harness.roundTrip(ClientboundAddEntityPacket.STREAM_CODEC, add));
    }

    private static void move(Move m) {
        Harness.listener.handleTeleportEntity(Harness.roundTrip(net.minecraft.network.protocol.game.ClientboundTeleportEntityPacket.STREAM_CODEC,
                new net.minecraft.network.protocol.game.ClientboundTeleportEntityPacket(m.id(),
                        new net.minecraft.world.entity.PositionMoveRotation(new Vec3(m.x(), m.y(), m.z()), Vec3.ZERO, m.yaw(), 0.0F),
                        java.util.Set.of(), m.onGround())));
    }

    // ---- other entities around the player: boxes before the player's tick and after each tick ----

    private static final java.util.Map<Integer, net.minecraft.world.phys.AABB> PRE = new java.util.HashMap<>();
    private static final List<Entity> ORDER = new ArrayList<>();
    /** Position of each entity in the level's own {@code getEntities} order (sections, then insertion). */
    private static final java.util.Map<Integer, Integer> QUERY_ORDER = new java.util.HashMap<>();

    static void beforeEntities(Harness.Run run) {
        PRE.clear();
        ORDER.clear();
        QUERY_ORDER.clear();
        Setup s = of(run.sc);
        if (s == null || s.entities == null) {
            return;
        }
        for (Entity e : run.level.entitiesForRendering()) {
            if (e.getId() != Harness.PLAYER_ENTITY_ID) {
                PRE.put(e.getId(), e.getBoundingBox());
            }
        }
        // Tick order: the level's ticking list (insertion order).
        Object list = Reflect.get(net.minecraft.client.multiplayer.ClientLevel.class, run.level, "tickingEntities");
        @SuppressWarnings("unchecked")
        it.unimi.dsi.fastutil.ints.Int2ObjectMap<Entity> active = (it.unimi.dsi.fastutil.ints.Int2ObjectMap<Entity>)
                Reflect.get(net.minecraft.world.level.entity.EntityTickList.class, list, "active");
        ORDER.addAll(active.values());
        int k = 0;
        for (Entity e : run.level.getEntities((Entity) run.player, run.player.getBoundingBox().inflate(64.0))) {
            QUERY_ORDER.put(e.getId(), k++);
        }
    }

    /** {@code ClientLevel.tick}'s border step (after the entities in {@code Minecraft.tick}). */
    static void afterEntities(Harness.Run run) {
        Setup s = of(run.sc);
        if (s != null && s.border) {
            run.level.getWorldBorder().tick();
        }
    }

    /** The {@code entities} of a tick record: every other ticking entity in tick order (cases with entities). */
    static void tickRecord(Json j, Harness.Run run) {
        Setup s = of(run.sc);
        if (s == null || s.entities == null) {
            return;
        }
        j.key("entities").beginArray();
        for (Entity e : ORDER) {
            if (e.getId() == Harness.PLAYER_ENTITY_ID) {
                j.beginObject().field("id", e.getId()).field("player", true).endObject();
                continue;
            }
            j.beginObject().field("id", e.getId()).field("type", BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).getPath());
            net.minecraft.world.phys.AABB pre = PRE.get(e.getId());
            if (pre != null) {
                box(j.key("pre"), pre);
            }
            if (!e.isRemoved()) {
                box(j.key("post"), e.getBoundingBox());
                j.key("pos").beginArray().f64(e.getX()).f64(e.getY()).f64(e.getZ()).endArray();
            }
            j.field("alive", e.isAlive());
            Integer q = QUERY_ORDER.get(e.getId());
            if (q != null) {
                j.field("query_order", q);
            }
            j.endObject();
        }
        j.endArray();
    }

    private static void box(Json j, net.minecraft.world.phys.AABB b) {
        j.beginArray().f64(b.minX).f64(b.minY).f64(b.minZ).f64(b.maxX).f64(b.maxY).f64(b.maxZ).endArray();
    }

    static void apply(Harness.Run run, Scenario.VersionEvent e) {
        if (e instanceof SharedFlags f) {
            sharedFlags(run, f.set(), f.clear());
        } else if (e instanceof LivingFlags f) {
            livingFlags(run, f.set(), f.clear());
        } else if (e instanceof Equip q) {
            equip(run, q.slot(), q.item(), q.damage());
        } else if (e instanceof Firework f) {
            firework(run, f.id());
        } else if (e instanceof BorderCenter c) {
            net.minecraft.world.level.border.WorldBorder wb = new net.minecraft.world.level.border.WorldBorder();
            wb.setCenter(c.x(), c.z());
            Harness.listener.handleSetBorderCenter(Harness.roundTrip(net.minecraft.network.protocol.game.ClientboundSetBorderCenterPacket.STREAM_CODEC,
                    new net.minecraft.network.protocol.game.ClientboundSetBorderCenterPacket(wb)));
        } else if (e instanceof BorderSize z) {
            net.minecraft.world.level.border.WorldBorder wb = new net.minecraft.world.level.border.WorldBorder();
            wb.setSize(z.size());
            Harness.listener.handleSetBorderSize(Harness.roundTrip(net.minecraft.network.protocol.game.ClientboundSetBorderSizePacket.STREAM_CODEC,
                    new net.minecraft.network.protocol.game.ClientboundSetBorderSizePacket(wb)));
        } else if (e instanceof BorderLerp b) {
            net.minecraft.world.level.border.WorldBorder wb = new net.minecraft.world.level.border.WorldBorder();
            wb.lerpSizeBetween(b.from(), b.to(), b.ticks(), 0L);
            Harness.listener.handleSetBorderLerpSize(Harness.roundTrip(net.minecraft.network.protocol.game.ClientboundSetBorderLerpSizePacket.STREAM_CODEC,
                    new net.minecraft.network.protocol.game.ClientboundSetBorderLerpSizePacket(wb)));
        } else if (e instanceof Spawn sp) {
            spawn(sp);
        } else if (e instanceof Move m) {
            move(m);
        } else if (e instanceof Remove r) {
            Harness.listener.handleRemoveEntities(Harness.roundTrip(ClientboundRemoveEntitiesPacket.STREAM_CODEC,
                    new ClientboundRemoveEntitiesPacket(r.id())));
        } else {
            throw new IllegalArgumentException("event " + e);
        }
    }

    @SuppressWarnings("unchecked")
    private static void sharedFlags(Harness.Run run, int set, int clear) {
        EntityDataAccessor<Byte> flags = (EntityDataAccessor<Byte>) Reflect.get(Entity.class, null, "DATA_SHARED_FLAGS_ID");
        int cur = run.player.getEntityData().get(flags);
        byte next = (byte) ((cur & ~clear) | set);
        List<SynchedEntityData.DataValue<?>> values = List.of(SynchedEntityData.DataValue.create(flags, next));
        Harness.listener.handleSetEntityData(Harness.roundTrip(ClientboundSetEntityDataPacket.STREAM_CODEC,
                new ClientboundSetEntityDataPacket(Harness.PLAYER_ENTITY_ID, values)));
    }

    @SuppressWarnings("unchecked")
    private static void livingFlags(Harness.Run run, int set, int clear) {
        EntityDataAccessor<Byte> flags = (EntityDataAccessor<Byte>) Reflect.get(net.minecraft.world.entity.LivingEntity.class, null,
                "DATA_LIVING_ENTITY_FLAGS");
        int cur = run.player.getEntityData().get(flags);
        // The server's byte carries its own view of the item use (bits 1 and 2): a server that agrees
        // with the client (a cleared bit 1 would stop the client's use).
        int use = run.player.isUsingItem() ? 1 | (run.player.getUsedItemHand() == net.minecraft.world.InteractionHand.OFF_HAND ? 2 : 0) : 0;
        byte next = (byte) ((((cur & ~3) | use) & ~clear) | set);
        Harness.listener.handleSetEntityData(Harness.roundTrip(ClientboundSetEntityDataPacket.STREAM_CODEC,
                new ClientboundSetEntityDataPacket(Harness.PLAYER_ENTITY_ID, List.of(SynchedEntityData.DataValue.create(flags, next)))));
    }

    private static void equip(Harness.Run run, String slot, String item, int damage) {
        ItemStack stack = item == null ? ItemStack.EMPTY : new ItemStack(item(item));
        if (damage != 0) {
            stack.setDamageValue(damage);
        }
        EquipmentSlot s = EquipmentSlot.byName(slot);
        Harness.listener.handleSetEquipment(Harness.roundTrip(ClientboundSetEquipmentPacket.STREAM_CODEC,
                new ClientboundSetEquipmentPacket(Harness.PLAYER_ENTITY_ID, List.of(Pair.of(s, stack)))));
    }

    /**
     * What the server sends when a gliding player uses a rocket: the entity built by the server's own
     * constructor ({@code new FireworkRocketEntity(level, stack, player)}: at the player, attached to it,
     * owned by it) as an add-entity packet (owner id as its data) plus its non-default entity data.
     */
    private static void firework(Harness.Run run, int id) {
        ItemStack stack = new ItemStack(Items.FIREWORK_ROCKET);
        FireworkRocketEntity server = new FireworkRocketEntity(run.level, stack, run.player);
        Vec3 pos = server.position();
        ClientboundAddEntityPacket add = new ClientboundAddEntityPacket(id, new UUID(0x66697265L, id), pos.x, pos.y, pos.z,
                server.getXRot(), server.getYRot(), EntityType.FIREWORK_ROCKET, Harness.PLAYER_ENTITY_ID, server.getDeltaMovement(), 0.0);
        Harness.listener.handleAddEntity(Harness.roundTrip(ClientboundAddEntityPacket.STREAM_CODEC, add));
        List<SynchedEntityData.DataValue<?>> values = server.getEntityData().getNonDefaultValues();
        if (values != null) {
            Harness.listener.handleSetEntityData(Harness.roundTrip(ClientboundSetEntityDataPacket.STREAM_CODEC,
                    new ClientboundSetEntityDataPacket(id, values)));
        }
    }

    static Item item(String name) {
        return BuiltInRegistries.ITEM.getOptional(Identifier.withDefaultNamespace(name))
                .orElseThrow(() -> new IllegalArgumentException("item " + name));
    }

    // ---- recording ---------------------------------------------------------------------------------

    /** The {@code ext} object of the header's setup (only for cases that use it). */
    static void header(Json j, Scenario sc) {
        Setup s = of(sc);
        if (s == null) {
            return;
        }
        j.key("ext").beginObject();
        if (s.chest != null) {
            j.field("chest", s.chest);
            if (s.chestDamage != 0) {
                j.field("chest_damage", s.chestDamage);
            }
        }
        if (s.hand != null) {
            j.field("hand", s.hand);
        }
        if (s.fallFlying) {
            j.field("fall_flying", true);
        }
        if (s.leftHanded) {
            j.field("left_handed", true);
        }
        if (s.riptide != 0) {
            j.field("riptide", s.riptide);
        }
        if (s.raining) {
            j.field("raining", true);
        }
        if (s.dimension != null) {
            j.field("dimension", s.dimension);
        }
        if (s.border) {
            j.field("border", true);
        }
        if (s.entities != null) {
            j.key("entities").beginArray();
            for (Spawn sp : s.entities) {
                spawnJson(j.beginObject(), sp).endObject();
            }
            j.endArray();
        }
        j.endObject();
    }

    static void event(Json j, Scenario.VersionEvent e) {
        if (e instanceof SharedFlags f) {
            j.field("type", "shared_flags").field("set", f.set()).field("clear", f.clear());
        } else if (e instanceof LivingFlags f) {
            j.field("type", "living_flags").field("set", f.set()).field("clear", f.clear());
        } else if (e instanceof Equip q) {
            j.field("type", "equipment").field("slot", q.slot()).field("item", q.item());
            if (q.damage() != 0) {
                j.field("damage", q.damage());
            }
        } else if (e instanceof Firework f) {
            j.field("type", "firework").field("id", f.id());
        } else if (e instanceof BorderCenter c) {
            j.field("type", "border_center").fieldF64("x", c.x()).fieldF64("z", c.z());
        } else if (e instanceof BorderSize z) {
            j.field("type", "border_size").fieldF64("size", z.size());
        } else if (e instanceof BorderLerp b) {
            j.field("type", "border_lerp").fieldF64("from", b.from()).fieldF64("to", b.to()).field("ticks", b.ticks());
        } else if (e instanceof Spawn sp) {
            spawnJson(j.field("type", "spawn"), sp);
        } else if (e instanceof Move m) {
            j.field("type", "entity_move").field("id", m.id());
            j.key("pos").beginArray().f64(m.x()).f64(m.y()).f64(m.z()).endArray();
            j.fieldF32("yaw", m.yaw()).field("on_ground", m.onGround());
        } else if (e instanceof Remove r) {
            j.field("type", "remove_entity").field("id", r.id());
        } else {
            throw new IllegalArgumentException("event " + e);
        }
    }

    private static Json spawnJson(Json j, Spawn sp) {
        j.field("id", sp.id()).field("entity", sp.type());
        j.key("pos").beginArray().f64(sp.x()).f64(sp.y()).f64(sp.z()).endArray();
        return j.fieldF32("yaw", sp.yaw());
    }

    private static Spawn readSpawn(JsonObject o) {
        com.google.gson.JsonArray p = o.getAsJsonArray("pos");
        return new Spawn(o.get("id").getAsInt(), o.get("entity").getAsString(), ScenarioReader.f64(p.get(0)), ScenarioReader.f64(p.get(1)),
                ScenarioReader.f64(p.get(2)), ScenarioReader.f32(o.get("yaw")));
    }

    static void readSetup(JsonObject s, Scenario sc) {
        if (!s.has("ext")) {
            return;
        }
        JsonObject x = s.getAsJsonObject("ext");
        Setup st = setup(sc);
        if (x.has("chest")) {
            st.chest = x.get("chest").getAsString();
            st.chestDamage = x.has("chest_damage") ? x.get("chest_damage").getAsInt() : 0;
        }
        if (x.has("hand")) {
            st.hand = x.get("hand").getAsString();
        }
        st.fallFlying = x.has("fall_flying") && x.get("fall_flying").getAsBoolean();
        st.leftHanded = x.has("left_handed") && x.get("left_handed").getAsBoolean();
        st.riptide = x.has("riptide") ? x.get("riptide").getAsInt() : 0;
        st.raining = x.has("raining") && x.get("raining").getAsBoolean();
        st.border = x.has("border") && x.get("border").getAsBoolean();
        if (x.has("dimension")) {
            st.dimension = x.get("dimension").getAsString();
        }
        if (x.has("entities")) {
            st.entities = new ArrayList<>();
            for (com.google.gson.JsonElement e : x.getAsJsonArray("entities")) {
                st.entities.add(readSpawn(e.getAsJsonObject()));
            }
        }
    }

    /** An event this class knows, or null. */
    static Scenario.VersionEvent readEvent(JsonObject ev) {
        return switch (ev.get("type").getAsString()) {
            case "shared_flags" -> new SharedFlags(ev.get("set").getAsInt(), ev.get("clear").getAsInt());
            case "living_flags" -> new LivingFlags(ev.get("set").getAsInt(), ev.get("clear").getAsInt());
            case "equipment" -> new Equip(ev.get("slot").getAsString(), ev.get("item").isJsonNull() ? null : ev.get("item").getAsString(),
                    ev.has("damage") ? ev.get("damage").getAsInt() : 0);
            case "firework" -> new Firework(ev.get("id").getAsInt());
            case "remove_entity" -> new Remove(ev.get("id").getAsInt());
            case "spawn" -> readSpawn(ev);
            case "border_center" -> new BorderCenter(ScenarioReader.f64(ev.get("x")), ScenarioReader.f64(ev.get("z")));
            case "border_size" -> new BorderSize(ScenarioReader.f64(ev.get("size")));
            case "border_lerp" -> new BorderLerp(ScenarioReader.f64(ev.get("from")), ScenarioReader.f64(ev.get("to")), ev.get("ticks").getAsLong());
            case "entity_move" -> {
                com.google.gson.JsonArray p = ev.getAsJsonArray("pos");
                yield new Move(ev.get("id").getAsInt(), ScenarioReader.f64(p.get(0)), ScenarioReader.f64(p.get(1)), ScenarioReader.f64(p.get(2)),
                        ScenarioReader.f32(ev.get("yaw")), ev.get("on_ground").getAsBoolean());
            }
            default -> null;
        };
    }

    static List<Scenario> copyList(List<Scenario> l) {
        return new ArrayList<>(l);
    }
}
