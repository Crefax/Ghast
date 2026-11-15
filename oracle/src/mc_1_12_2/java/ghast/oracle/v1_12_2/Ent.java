package ghast.oracle.v1_12_2;

import ghast.oracle.Json;
import ghast.oracle.Scenario;
import io.netty.buffer.Unpooled;
import net.minecraft.client.MinecraftClient;
import net.minecraft.client.network.ClientPlayNetworkHandler;
import net.minecraft.entity.Entity;
import net.minecraft.entity.EntityType;
import net.minecraft.entity.player.PlayerEntity;
import net.minecraft.network.Packet;
import net.minecraft.network.listener.ClientPlayPacketListener;
import net.minecraft.network.packet.s2c.play.EntitiesDestroyS2CPacket;
import net.minecraft.network.packet.s2c.play.EntityPositionS2CPacket;
import net.minecraft.network.packet.s2c.play.EntityS2CPacket;
import net.minecraft.network.packet.s2c.play.EntitySpawnS2CPacket;
import net.minecraft.network.packet.s2c.play.MobSpawnS2CPacket;
import net.minecraft.network.packet.s2c.play.PlayerListS2CPacket;
import net.minecraft.network.packet.s2c.play.PlayerSpawnS2CPacket;
import net.minecraft.network.packet.s2c.play.TeamS2CPacket;
import net.minecraft.util.Identifier;
import net.minecraft.util.PacketByteBuf;
import net.minecraft.util.math.Box;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.UUID;

/**
 * Other entities around the 1.12.2 player: spawned, moved and removed through the real client packet
 * handlers ({@code onMobSpawn}, {@code onEntitySpawn}, {@code onPlayerSpawn}, {@code onEntityPosition},
 * {@code onEntityUpdate}, {@code onEntitiesDestroy}), with the tab list ({@code onPlayerList}) and
 * scoreboard teams ({@code onTeam}) the push rules read. Every packet is built as wire bytes and
 * decoded by the packet's own {@code read}, as if it came from a server.
 *
 * <p>The client then ticks those entities in {@code World.tickEntities} after the player (they are
 * added after it): living entities run {@code tickCramming}, boats their push, and both push the
 * player through {@code Entity.pushAwayFrom}; boats and live shulkers are solid for the player's own
 * move ({@code World.doesBoxCollide}).
 *
 * <p>What is faked: nothing beyond the harness's client singleton. The handlers check
 * {@code NetworkThreadUtils.forceMainThread}, so {@code MinecraftClient.currentThread} is set to the
 * harness thread while a packet is handled. No entity metadata is sent (entities keep their client
 * defaults: adult, default slime size, closed shulker without an attached block), so shulkers are
 * spawned at block centres and never moved (a server moves a shulker by its attached block).
 */
final class Ent {
    private Ent() {}

    /** The local player's entity id in cases with entities (the join packet's id). */
    static final int PLAYER_ENTITY_ID = 1;

    /** Setup of a case with entities: events applied after the join, in order (spawns, tab entries, teams). */
    static final class Setup {
        final List<Scenario.Event> events = new ArrayList<>();
    }

    /** Mob, object or player spawn: {@code type} is the registry path ({@code "zombie"}, {@code "boat"}) or {@code "player"}. */
    record Spawn(int id, String type, double x, double y, double z, float yaw) implements Scenario.VersionEvent {}

    /** Entity teleport packet (absolute position, yaw, on ground). */
    record Move(int id, double x, double y, double z, float yaw, boolean onGround) implements Scenario.VersionEvent {}

    /** Relative move packet: deltas in 1/4096 blocks. */
    record Rel(int id, int dx, int dy, int dz, boolean onGround) implements Scenario.VersionEvent {}

    /** Destroy entities packet. */
    record Remove(int id) implements Scenario.VersionEvent {}

    /** Player list packet for player {@code id}: add (name {@code p<id>}) or game mode update. */
    record Tab(int id, boolean add, int gameMode) implements Scenario.VersionEvent {}

    /**
     * Teams packet: mode 0 create (with rule and members), 1 remove, 2 update the rule, 3 add members,
     * 4 remove members. Members are scoreboard names: {@code "oracle"} (the local player), {@code p<id>}
     * (another player) or a mob's UUID string ({@link #uuid}).
     */
    record Team(String name, int mode, String rule, List<String> members) implements Scenario.VersionEvent {}

    static Setup of(Scenario sc) {
        return sc.versionSetup instanceof Setup s ? s : null;
    }

    static Setup setup(Scenario sc) {
        if (!(sc.versionSetup instanceof Setup)) {
            sc.versionSetup = new Setup();
        }
        return (Setup) sc.versionSetup;
    }

    /** UUID of entity {@code id} (players: {@code 706c6179-…}, others {@code 656e74-…}). */
    static UUID uuid(int id, boolean player) {
        return new UUID(player ? 0x706c6179L : 0x656e74L, id);
    }

    /** The scoreboard name of entity {@code id}. */
    static String member(int id, boolean player) {
        return player ? "p" + id : uuid(id, false).toString();
    }

    // ---- packets --------------------------------------------------------------------------------

    private static PacketByteBuf buf() {
        return new PacketByteBuf(Unpooled.buffer());
    }

    @SuppressWarnings("unchecked")
    private static void handle(Harness.Run run, Packet<?> p, PacketByteBuf b) {
        Object thread = Reflect.get(MinecraftClient.class, MinecraftClient.getInstance(), "currentThread");
        Reflect.set(MinecraftClient.class, MinecraftClient.getInstance(), "currentThread", Thread.currentThread());
        try {
            p.read(b);
            if (b.readableBytes() != 0) {
                throw new IllegalStateException(p.getClass().getSimpleName() + ": " + b.readableBytes() + " bytes left");
            }
            ((Packet<ClientPlayPacketListener>) p).apply(run.net);
        } catch (java.io.IOException e) {
            throw new IllegalStateException(e);
        } finally {
            Reflect.set(MinecraftClient.class, MinecraftClient.getInstance(), "currentThread", thread);
        }
    }

    /** Object type ids of the spawn-object packet for the object kinds the cases use. */
    private static final Map<String, Integer> OBJECTS = Map.of("boat", 1, "item", 2, "minecart", 10, "armor_stand", 78);

    private static void spawn(Harness.Run run, Spawn s) {
        byte yaw = (byte) Math.floor(s.yaw() * 256.0F / 360.0F);
        PacketByteBuf b = buf();
        if (s.type().equals("player")) {
            b.writeVarInt(s.id());
            b.writeUuid(uuid(s.id(), true));
            b.writeDouble(s.x()).writeDouble(s.y()).writeDouble(s.z());
            b.writeByte(yaw).writeByte(0);
            b.writeByte(0xff); // no metadata
            handle(run, new PlayerSpawnS2CPacket(), b);
        } else if (OBJECTS.containsKey(s.type())) {
            b.writeVarInt(s.id());
            b.writeUuid(uuid(s.id(), false));
            b.writeByte(OBJECTS.get(s.type()));
            b.writeDouble(s.x()).writeDouble(s.y()).writeDouble(s.z());
            b.writeByte(0).writeByte(yaw);
            b.writeInt(0);
            b.writeShort(0).writeShort(0).writeShort(0);
            handle(run, new EntitySpawnS2CPacket(), b);
        } else {
            Class<? extends Entity> cls = EntityType.getEntityType(s.type());
            if (cls == null) {
                throw new IllegalArgumentException("entity " + s.type());
            }
            b.writeVarInt(s.id());
            b.writeUuid(uuid(s.id(), false));
            b.writeVarInt(EntityType.REGISTRY.getRawId(cls));
            b.writeDouble(s.x()).writeDouble(s.y()).writeDouble(s.z());
            b.writeByte(yaw).writeByte(0).writeByte(yaw);
            b.writeShort(0).writeShort(0).writeShort(0);
            b.writeByte(0xff); // no metadata
            handle(run, new MobSpawnS2CPacket(), b);
        }
    }

    private static void move(Harness.Run run, Move m) {
        PacketByteBuf b = buf();
        b.writeVarInt(m.id());
        b.writeDouble(m.x()).writeDouble(m.y()).writeDouble(m.z());
        b.writeByte((byte) Math.floor(m.yaw() * 256.0F / 360.0F)).writeByte(0);
        b.writeBoolean(m.onGround());
        handle(run, new EntityPositionS2CPacket(), b);
    }

    private static void rel(Harness.Run run, Rel r) {
        PacketByteBuf b = buf();
        b.writeVarInt(r.id());
        b.writeShort(r.dx()).writeShort(r.dy()).writeShort(r.dz());
        b.writeBoolean(r.onGround());
        handle(run, new EntityS2CPacket.MoveRelative(), b);
    }

    private static void remove(Harness.Run run, Remove r) {
        PacketByteBuf b = buf();
        b.writeVarInt(1).writeVarInt(r.id());
        handle(run, new EntitiesDestroyS2CPacket(), b);
    }

    private static void tab(Harness.Run run, Tab t) {
        PacketByteBuf b = buf();
        b.writeVarInt(t.add() ? 0 : 1); // ADD_PLAYER, UPDATE_GAME_MODE
        b.writeVarInt(1);
        b.writeUuid(uuid(t.id(), true));
        if (t.add()) {
            b.writeString("p" + t.id());
            b.writeVarInt(0); // no properties
            b.writeVarInt(t.gameMode());
            b.writeVarInt(0); // latency
            b.writeBoolean(false); // no display name
        } else {
            b.writeVarInt(t.gameMode());
        }
        handle(run, new PlayerListS2CPacket(), b);
    }

    private static void team(Harness.Run run, Team t) {
        PacketByteBuf b = buf();
        b.writeString(t.name());
        b.writeByte(t.mode());
        if (t.mode() == 0 || t.mode() == 2) {
            b.writeString(t.name()); // display name
            b.writeString(""); // prefix
            b.writeString(""); // suffix
            b.writeByte(0); // friendly flags
            b.writeString("always"); // name tag visibility
            b.writeString(t.rule());
            b.writeByte(-1); // no colour
        }
        if (t.mode() == 0 || t.mode() == 3 || t.mode() == 4) {
            b.writeVarInt(t.members().size());
            for (String m : t.members()) {
                b.writeString(m);
            }
        }
        handle(run, new TeamS2CPacket(), b);
    }

    /** Applies one of this file's events (an inbound packet before the tick). */
    static boolean apply(Harness.Run run, Scenario.Event e) {
        if (e instanceof Spawn s) {
            spawn(run, s);
        } else if (e instanceof Move m) {
            move(run, m);
        } else if (e instanceof Rel r) {
            rel(run, r);
        } else if (e instanceof Remove r) {
            remove(run, r);
        } else if (e instanceof Tab t) {
            tab(run, t);
        } else if (e instanceof Team t) {
            team(run, t);
        } else {
            return false;
        }
        return true;
    }

    /** End of the case setup: the player gets the join id, then the setup packets arrive. */
    static void setupAfterJoin(Harness.Run run) {
        PRE.clear();
        Setup s = of(run.sc);
        if (s == null) {
            return;
        }
        run.player.setEntityId(PLAYER_ENTITY_ID);
        for (Scenario.Event e : s.events) {
            apply(run, e);
        }
    }

    // ---- recording --------------------------------------------------------------------------------

    /** The world's entity list before the tick (its tick order) with each entity's box and removed flag then. */
    private record Before(Entity e, Box box, boolean removed) {}

    private static final List<Before> PRE = new ArrayList<>();

    /** Before {@code World.tickEntities}: every entity's box as the player's tick sees it. */
    static void beforeTick(Harness.Run run) {
        PRE.clear();
        if (of(run.sc) == null) {
            return;
        }
        for (Entity e : run.world.loadedEntities) {
            PRE.add(new Before(e, e.getBoundingBox(), e.removed));
        }
    }

    static String typeName(Entity e) {
        if (e instanceof PlayerEntity) {
            return "player";
        }
        Identifier id = EntityType.getId(e);
        return id == null ? e.getClass().getSimpleName() : id.getPath();
    }

    /**
     * The {@code entities} of a tick record: every entity of the world's entity list in tick order (the
     * local player as {@code {"id":1,"player":true}}), with its box before the tick ({@code pre}), its
     * box and position after it ({@code post}, {@code pos}) and {@code alive}. An entity removed by a
     * packet before the tick is still listed with {@code removed} (it is still in its chunk's entity
     * list while the player moves, and leaves the world list in this tick), without {@code post}.
     * The tick 0 record lists the entities after the setup ({@code post} only).
     */
    static void tickRecord(Json j, Harness.Run run) {
        if (of(run.sc) == null) {
            return;
        }
        j.key("entities").beginArray();
        if (PRE.isEmpty()) {
            for (Entity e : run.world.loadedEntities) {
                entity(j, run, e, null, false);
            }
        } else {
            for (Before b : PRE) {
                entity(j, run, b.e(), b.box(), b.removed());
            }
        }
        j.endArray();
    }

    private static void entity(Json j, Harness.Run run, Entity e, Box pre, boolean removedBefore) {
        if (e == run.player) {
            j.beginObject().field("id", e.getEntityId()).field("player", true).endObject();
            return;
        }
        j.beginObject().field("id", e.getEntityId()).field("type", typeName(e));
        if (pre != null) {
            box(j.key("pre"), pre);
        }
        if (removedBefore) {
            j.field("removed", true);
        } else {
            box(j.key("post"), e.getBoundingBox());
            j.key("pos").beginArray().f64(e.x).f64(e.y).f64(e.z).endArray();
            j.field("alive", e.isAlive());
        }
        j.endObject();
    }

    private static void box(Json j, Box b) {
        j.beginArray().f64(b.minX).f64(b.minY).f64(b.minZ).f64(b.maxX).f64(b.maxY).f64(b.maxZ).endArray();
    }

    /** Writes one of this file's events. */
    static boolean event(Json j, Scenario.Event e) {
        if (e instanceof Spawn s) {
            j.field("type", "spawn").field("id", s.id()).field("entity", s.type());
            j.key("pos").beginArray().f64(s.x()).f64(s.y()).f64(s.z()).endArray();
            j.fieldF32("yaw", s.yaw());
        } else if (e instanceof Move m) {
            j.field("type", "entity_move").field("id", m.id());
            j.key("pos").beginArray().f64(m.x()).f64(m.y()).f64(m.z()).endArray();
            j.fieldF32("yaw", m.yaw()).field("on_ground", m.onGround());
        } else if (e instanceof Rel r) {
            j.field("type", "entity_rel_move").field("id", r.id()).field("dx", r.dx()).field("dy", r.dy()).field("dz", r.dz())
                    .field("on_ground", r.onGround());
        } else if (e instanceof Remove r) {
            j.field("type", "remove_entity").field("id", r.id());
        } else if (e instanceof Tab t) {
            j.field("type", "tab").field("id", t.id()).field("add", t.add()).field("game_mode", t.gameMode());
        } else if (e instanceof Team t) {
            j.field("type", "team").field("name", t.name()).field("mode", t.mode());
            if (t.mode() == 0 || t.mode() == 2) {
                j.field("rule", t.rule());
            }
            j.key("members").beginArray();
            for (String m : t.members()) {
                j.value(m);
            }
            j.endArray();
        } else {
            return false;
        }
        return true;
    }

    /** The header's {@code setup.entities}: the setup events. */
    static void header(Json j, Scenario sc) {
        Setup s = of(sc);
        if (s == null) {
            return;
        }
        j.key("entities").beginArray();
        for (Scenario.Event e : s.events) {
            j.beginObject();
            event(j, e);
            j.endObject();
        }
        j.endArray();
    }

    // ---- reading back (ScenarioReader) -------------------------------------------------------------

    static Scenario.Event read(com.google.gson.JsonObject ev) {
        java.util.function.Function<String, Double> d = k -> ScenarioReader.f64(ev.getAsJsonArray("pos").get(
                switch (k) {
                    case "x" -> 0;
                    case "y" -> 1;
                    default -> 2;
                }));
        return switch (ev.get("type").getAsString()) {
            case "spawn" -> new Spawn(ev.get("id").getAsInt(), ev.get("entity").getAsString(), d.apply("x"), d.apply("y"),
                    d.apply("z"), ScenarioReader.f32(ev.get("yaw")));
            case "entity_move" -> new Move(ev.get("id").getAsInt(), d.apply("x"), d.apply("y"), d.apply("z"),
                    ScenarioReader.f32(ev.get("yaw")), ev.get("on_ground").getAsBoolean());
            case "entity_rel_move" -> new Rel(ev.get("id").getAsInt(), ev.get("dx").getAsInt(), ev.get("dy").getAsInt(),
                    ev.get("dz").getAsInt(), ev.get("on_ground").getAsBoolean());
            case "remove_entity" -> new Remove(ev.get("id").getAsInt());
            case "tab" -> new Tab(ev.get("id").getAsInt(), ev.get("add").getAsBoolean(), ev.get("game_mode").getAsInt());
            case "team" -> {
                List<String> members = new ArrayList<>();
                ev.getAsJsonArray("members").forEach(m -> members.add(m.getAsString()));
                yield new Team(ev.get("name").getAsString(), ev.get("mode").getAsInt(),
                        ev.has("rule") ? ev.get("rule").getAsString() : "", List.copyOf(members));
            }
            default -> null;
        };
    }

    static void readHeader(com.google.gson.JsonObject setup, Scenario sc) {
        if (!setup.has("entities")) {
            return;
        }
        Setup s = setup(sc);
        setup.getAsJsonArray("entities").forEach(e -> s.events.add(read(e.getAsJsonObject())));
    }
}
