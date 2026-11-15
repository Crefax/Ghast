package ghast.oracle.v1_16_5;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.mojang.authlib.GameProfile;
import ghast.oracle.Json;
import ghast.oracle.Scenario;
import net.minecraft.client.network.ClientPlayNetworkHandler;
import net.minecraft.client.network.OtherClientPlayerEntity;
import net.minecraft.entity.Entity;
import net.minecraft.entity.EntityType;
import net.minecraft.entity.ItemEntity;
import net.minecraft.item.ItemStack;
import net.minecraft.item.Items;
import net.minecraft.network.Packet;
import net.minecraft.network.PacketByteBuf;
import net.minecraft.network.listener.ClientPlayPacketListener;
import net.minecraft.network.packet.s2c.play.EntitiesDestroyS2CPacket;
import net.minecraft.network.packet.s2c.play.EntityPositionS2CPacket;
import net.minecraft.network.packet.s2c.play.EntityS2CPacket;
import net.minecraft.network.packet.s2c.play.EntityTrackerUpdateS2CPacket;
import net.minecraft.network.packet.s2c.play.PlayerListS2CPacket;
import net.minecraft.network.packet.s2c.play.PlayerSpawnS2CPacket;
import net.minecraft.network.packet.s2c.play.TeamS2CPacket;
import net.minecraft.scoreboard.AbstractTeam;
import net.minecraft.scoreboard.Scoreboard;
import net.minecraft.scoreboard.Team;
import net.minecraft.util.Identifier;
import net.minecraft.util.math.Box;
import net.minecraft.util.registry.Registry;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;

/**
 * Other entities around the 1.16.5 player (added for ghast 0.4.5): entities the player collides
 * with (boats, shulkers) and entities whose client tick pushes it (mobs, other players, boats).
 *
 * <p>Everything goes through the real client: each entity is built the way a server builds it
 * ({@code EntityType.create}, position, id, UUID) and the client gets that entity's own spawn packet
 * ({@code createSpawnPacket}, serialised and decoded) followed by its full entity data
 * ({@code EntityTrackerUpdateS2CPacket} with every entry, as a server sends at spawn), then the
 * real handlers. Other players get a tab-list entry ({@code PlayerListS2CPacket}) before their
 * spawn. Movement is teleport ({@code EntityPositionS2CPacket}), relative move and look-only
 * ({@code EntityS2CPacket}) packets; teams are {@code TeamS2CPacket}s built from a server-side
 * scoreboard team. The client world then ticks every entity in its own order
 * ({@code ClientWorld.tickEntities}: the iteration order of its entity map).
 *
 * <p>A case uses this only when {@link Scenario#versionSetup} is a {@link Setup}: the bytes of every
 * older golden file stay the same.
 */
final class Ents {
    private Ents() {}

    /** Version setup of a case (header key {@code ext} inside {@code setup}). */
    static class Setup {
        /** Entities spawned at setup, in order (after the player joined). Never null. */
        final List<Spawn> entities = new ArrayList<>();
        /** Teams created at setup (before the entities), in order. */
        final List<TeamOp> teams = new ArrayList<>();

        /** Tick records list the other entities ({@link Ext.Setup} cases only when they spawn some). */
        boolean records() {
            return true;
        }
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
     * A spawn: {@code type} is the registry name without namespace, or {@code player} for another
     * player (a tab-list entry with {@code gameMode} first; its name is {@code p<id>}).
     */
    record Spawn(int id, String type, double x, double y, double z, float yaw, int gameMode) implements Scenario.VersionEvent {}

    /** A teleport packet (absolute position, yaw, on ground). */
    record Move(int id, double x, double y, double z, float yaw, boolean onGround) implements Scenario.VersionEvent {}

    /** A relative move packet: deltas in 1/4096 blocks. */
    record Rel(int id, short dx, short dy, short dz, boolean onGround) implements Scenario.VersionEvent {}

    /** A look-only packet (yaw). */
    record Look(int id, float yaw) implements Scenario.VersionEvent {}

    /** Destroy-entities packet. */
    record Remove(int id) implements Scenario.VersionEvent {}

    /**
     * A team packet: mode 0 create (rule, members), 1 remove, 2 update (rule), 3 join (members),
     * 4 leave (members). Members are scoreboard names (player names, other entities' UUID strings).
     */
    record TeamOp(String name, int mode, String rule, List<String> members) implements Scenario.VersionEvent {}

    /** A tab-list game mode update for the other player {@code p<id>}. */
    record GameMode(int id, int gameMode) implements Scenario.VersionEvent {}

    /** The UUID every non-player entity {@code id} gets (its scoreboard name is its string). */
    static UUID uuidOf(int id) {
        return new UUID(0x656e74L, id);
    }

    /** The profile UUID of the other player {@code id}. */
    static UUID playerUuidOf(int id) {
        return new UUID(0x706c6179L, id);
    }

    static String playerNameOf(int id) {
        return "p" + id;
    }

    // ---- harness hooks -----------------------------------------------------------------------------

    private static final Map<Integer, Box> PRE = new HashMap<>();
    private static final List<Entity> ORDER = new ArrayList<>();
    private static final Map<Integer, Integer> QUERY_ORDER = new HashMap<>();
    /** Entities {@code ClientWorld.tickEntities} ticks this tick (decided before the tick). */
    private static final java.util.Set<Integer> TICKS = new java.util.HashSet<>();
    /** Entities in a chunk's entity section before the tick ({@code updateNeeded}). */
    private static final java.util.Set<Integer> IN_SECTION = new java.util.HashSet<>();

    /** Applied at the end of setup (after the player's state overrides). */
    static void finishSetup(Harness.Run run) {
        PRE.clear();
        ORDER.clear();
        QUERY_ORDER.clear();
        TICKS.clear();
        IN_SECTION.clear();
        Setup s = of(run.sc);
        if (s == null) {
            return;
        }
        for (TeamOp t : s.teams) {
            team(t);
        }
        for (Spawn sp : s.entities) {
            spawn(run, sp);
        }
    }

    static void apply(Harness.Run run, Scenario.VersionEvent e) {
        ClientPlayNetworkHandler net = Harness.net;
        if (e instanceof Spawn sp) {
            spawn(run, sp);
        } else if (e instanceof Move m) {
            PacketByteBuf b = Harness.buf();
            b.writeVarInt(m.id());
            b.writeDouble(m.x());
            b.writeDouble(m.y());
            b.writeDouble(m.z());
            b.writeByte(angle(m.yaw()));
            b.writeByte(0);
            b.writeBoolean(m.onGround());
            net.onEntityPosition(Harness.read(new EntityPositionS2CPacket(), b));
        } else if (e instanceof Rel r) {
            PacketByteBuf b = Harness.buf();
            b.writeVarInt(r.id());
            b.writeShort(r.dx());
            b.writeShort(r.dy());
            b.writeShort(r.dz());
            b.writeBoolean(r.onGround());
            net.onEntityUpdate(Harness.read(new EntityS2CPacket.MoveRelative(), b));
        } else if (e instanceof Look l) {
            PacketByteBuf b = Harness.buf();
            b.writeVarInt(l.id());
            b.writeByte(angle(l.yaw()));
            b.writeByte(0);
            b.writeBoolean(true);
            net.onEntityUpdate(Harness.read(new EntityS2CPacket.Rotate(), b));
        } else if (e instanceof Remove r) {
            PacketByteBuf b = Harness.buf();
            b.writeVarInt(1);
            b.writeVarInt(r.id());
            net.onEntitiesDestroy(Harness.read(new EntitiesDestroyS2CPacket(), b));
        } else if (e instanceof TeamOp t) {
            team(t);
        } else if (e instanceof GameMode g) {
            PacketByteBuf b = Harness.buf();
            b.writeEnumConstant(PlayerListS2CPacket.Action.UPDATE_GAME_MODE);
            b.writeVarInt(1);
            b.writeUuid(playerUuidOf(g.id()));
            b.writeVarInt(g.gameMode());
            net.onPlayerList(Harness.read(new PlayerListS2CPacket(), b));
        } else {
            throw new IllegalArgumentException("event " + e);
        }
    }

    private static int angle(float yaw) {
        return (int) (yaw * 256.0F / 360.0F);
    }

    /** The entity a server would hold, its spawn packet and its full entity data, into the client. */
    @SuppressWarnings("unchecked")
    private static void spawn(Harness.Run run, Spawn s) {
        ClientPlayNetworkHandler net = Harness.net;
        Entity server;
        if (s.type().equals("player")) {
            GameProfile profile = new GameProfile(playerUuidOf(s.id()), playerNameOf(s.id()));
            PacketByteBuf b = Harness.buf();
            b.writeEnumConstant(PlayerListS2CPacket.Action.ADD_PLAYER);
            b.writeVarInt(1);
            b.writeUuid(profile.getId());
            b.writeString(profile.getName());
            b.writeVarInt(0);
            b.writeVarInt(s.gameMode());
            b.writeVarInt(0);
            b.writeBoolean(false);
            net.onPlayerList(Harness.read(new PlayerListS2CPacket(), b));
            server = new OtherClientPlayerEntity(run.world, profile);
        } else if (s.type().equals("item")) {
            server = new ItemEntity(run.world, s.x(), s.y(), s.z(), new ItemStack(Items.STONE));
            server.setVelocity(0, 0, 0);
            server.setUuid(uuidOf(s.id()));
        } else {
            EntityType<?> type = Registry.ENTITY_TYPE.getOrEmpty(new Identifier(s.type()))
                    .orElseThrow(() -> new IllegalArgumentException("entity " + s.type()));
            server = type.create(run.world);
            if (server == null) {
                throw new IllegalArgumentException("entity " + s.type() + " cannot be created");
            }
            server.setUuid(uuidOf(s.id()));
        }
        server.setEntityId(s.id());
        server.refreshPositionAndAngles(s.x(), s.y(), s.z(), s.yaw(), 0.0F);
        server.setHeadYaw(s.yaw());
        if (server instanceof net.minecraft.entity.mob.ShulkerEntity) {
            // A server shulker that has ticked once holds its attached block (its block position).
            @SuppressWarnings("unchecked")
            net.minecraft.entity.data.TrackedData<java.util.Optional<net.minecraft.util.math.BlockPos>> attached =
                    (net.minecraft.entity.data.TrackedData<java.util.Optional<net.minecraft.util.math.BlockPos>>) Reflect.get(
                            net.minecraft.entity.mob.ShulkerEntity.class, null, "ATTACHED_BLOCK");
            server.getDataTracker().set(attached, java.util.Optional.of(server.getBlockPos()));
        }
        Packet<ClientPlayPacketListener> spawn = (Packet<ClientPlayPacketListener>) (server instanceof OtherClientPlayerEntity p
                ? new PlayerSpawnS2CPacket(p) : server.createSpawnPacket());
        roundTrip(spawn).apply(net);
        roundTrip(new EntityTrackerUpdateS2CPacket(s.id(), server.getDataTracker(), true)).apply(net);
        // EntityTrackerEntry.sendPackets: types that always update their velocity get it right
        // after the spawn (not with a mob spawn packet, which carries it). Without it the client's
        // item entity keeps the random velocity of its constructor (unseeded entity random).
        if (server.getType().alwaysUpdateVelocity() && !(spawn instanceof net.minecraft.network.packet.s2c.play.MobSpawnS2CPacket)) {
            roundTrip(new net.minecraft.network.packet.s2c.play.EntityVelocityUpdateS2CPacket(s.id(), server.getVelocity())).apply(net);
        }
    }

    @SuppressWarnings("unchecked")
    private static <P extends Packet<ClientPlayPacketListener>> P roundTrip(P p) {
        try {
            PacketByteBuf b = Harness.buf();
            p.write(b);
            P fresh = (P) p.getClass().getDeclaredConstructor().newInstance();
            return Harness.read(fresh, b);
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    /** A team packet as a server sends it, built from a server-side scoreboard team. */
    private static void team(TeamOp t) {
        Scoreboard sb = new Scoreboard();
        Team team = sb.addTeam(t.name());
        if (t.rule() != null) {
            AbstractTeam.CollisionRule rule = AbstractTeam.CollisionRule.getRule(t.rule());
            if (rule == null) {
                throw new IllegalArgumentException("collision rule " + t.rule());
            }
            team.setCollisionRule(rule);
        }
        TeamS2CPacket p;
        switch (t.mode()) {
            case 0 -> {
                for (String m : t.members()) {
                    sb.addPlayerToTeam(m, team);
                }
                p = new TeamS2CPacket(team, 0);
            }
            case 1, 2 -> p = new TeamS2CPacket(team, t.mode());
            case 3, 4 -> p = new TeamS2CPacket(team, t.members(), t.mode());
            default -> throw new IllegalArgumentException("team mode " + t.mode());
        }
        roundTrip(p).apply(Harness.net);
    }

    /** Before {@code ClientWorld.tickEntities}: every other entity's box, the tick order and the query order. */
    static void beforeTick(Harness.Run run) {
        PRE.clear();
        ORDER.clear();
        QUERY_ORDER.clear();
        TICKS.clear();
        IN_SECTION.clear();
        if (of(run.sc) == null || !of(run.sc).records()) {
            return;
        }
        for (Entity e : run.world.getEntities()) {
            ORDER.add(e);
            if (e.getEntityId() != Harness.PLAYER_ENTITY_ID) {
                PRE.put(e.getEntityId(), e.getBoundingBox());
                // ClientWorld.tickEntities / tickEntity: not a passenger, not removed, a player or
                // in a loaded chunk, then updateNeeded (or a spectator).
                boolean ticks = !e.hasVehicle() && !e.removed
                        && (e instanceof net.minecraft.entity.player.PlayerEntity || run.world.getChunkManager().shouldTickEntity(e))
                        && (e.updateNeeded || e.isSpectator());
                if (ticks) {
                    TICKS.add(e.getEntityId());
                }
                if (e.updateNeeded) {
                    IN_SECTION.add(e.getEntityId());
                }
            }
        }
        // World.getOtherEntities order (chunks by x then z, sections, then each section's list).
        int k = 0;
        for (Entity e : run.world.getOtherEntities(run.player, run.player.getBoundingBox().expand(48.0), null)) {
            QUERY_ORDER.put(e.getEntityId(), k++);
        }
    }

    // ---- recording ---------------------------------------------------------------------------------

    private static void box(Json j, Box b) {
        j.beginArray().f64(b.minX).f64(b.minY).f64(b.minZ).f64(b.maxX).f64(b.maxY).f64(b.maxZ).endArray();
    }

    /** The {@code entities} of a tick record: every entity of the world in tick order. */
    static void tickRecord(Json j, Harness.Run run) {
        if (of(run.sc) == null || !of(run.sc).records()) {
            return;
        }
        j.key("entities").beginArray();
        for (Entity e : ORDER) {
            if (e.getEntityId() == Harness.PLAYER_ENTITY_ID) {
                j.beginObject().field("id", e.getEntityId()).field("player", true).endObject();
                continue;
            }
            j.beginObject().field("id", e.getEntityId()).field("type", Registry.ENTITY_TYPE.getId(e.getType()).getPath());
            Box pre = PRE.get(e.getEntityId());
            if (pre != null) {
                box(j.key("pre"), pre);
            }
            box(j.key("post"), e.getBoundingBox());
            j.key("pos").beginArray().f64(e.getX()).f64(e.getY()).f64(e.getZ()).endArray();
            j.field("alive", e.isAlive());
            j.field("ticks", TICKS.contains(e.getEntityId()));
            j.field("in_section_pre", IN_SECTION.contains(e.getEntityId()));
            j.field("in_section_post", e.updateNeeded);
            if (e instanceof net.minecraft.entity.player.PlayerEntity) {
                j.field("spectator", e.isSpectator());
            }
            Integer q = QUERY_ORDER.get(e.getEntityId());
            if (q != null) {
                j.field("query_order", q);
            }
            j.endObject();
        }
        j.endArray();
    }

    /** The {@code ext} object of the header's setup. */
    static void header(Json j, Scenario sc) {
        Setup s = of(sc);
        if (s == null) {
            return;
        }
        j.key("ext").beginObject();
        j.key("teams").beginArray();
        for (TeamOp t : s.teams) {
            teamJson(j.beginObject(), t).endObject();
        }
        j.endArray();
        j.key("entities").beginArray();
        for (Spawn sp : s.entities) {
            spawnJson(j.beginObject(), sp).endObject();
        }
        j.endArray();
        Ext.header(j, s);
        j.endObject();
    }

    static void event(Json j, Scenario.VersionEvent e) {
        if (e instanceof Spawn sp) {
            spawnJson(j.field("type", "spawn"), sp);
        } else if (e instanceof Move m) {
            j.field("type", "entity_move").field("id", m.id());
            j.key("pos").beginArray().f64(m.x()).f64(m.y()).f64(m.z()).endArray();
            j.fieldF32("yaw", m.yaw()).field("on_ground", m.onGround());
        } else if (e instanceof Rel r) {
            j.field("type", "entity_rel").field("id", r.id());
            j.key("delta").beginArray().value(r.dx()).value(r.dy()).value(r.dz()).endArray();
            j.field("on_ground", r.onGround());
        } else if (e instanceof Look l) {
            j.field("type", "entity_look").field("id", l.id()).fieldF32("yaw", l.yaw());
        } else if (e instanceof Remove r) {
            j.field("type", "remove_entity").field("id", r.id());
        } else if (e instanceof TeamOp t) {
            teamJson(j.field("type", "team"), t);
        } else if (e instanceof GameMode g) {
            j.field("type", "game_mode").field("id", g.id()).field("game_mode", g.gameMode());
        } else {
            throw new IllegalArgumentException("event " + e);
        }
    }

    private static Json spawnJson(Json j, Spawn sp) {
        j.field("id", sp.id()).field("entity", sp.type());
        j.key("pos").beginArray().f64(sp.x()).f64(sp.y()).f64(sp.z()).endArray();
        j.fieldF32("yaw", sp.yaw());
        if (sp.type().equals("player")) {
            j.field("name", playerNameOf(sp.id())).field("uuid", playerUuidOf(sp.id()).toString()).field("game_mode", sp.gameMode());
        } else {
            j.field("uuid", uuidOf(sp.id()).toString());
        }
        return j;
    }

    private static Json teamJson(Json j, TeamOp t) {
        j.field("name", t.name()).field("mode", t.mode());
        if (t.rule() != null) {
            j.field("rule", t.rule());
        }
        j.key("members").beginArray();
        for (String m : t.members()) {
            j.value(m);
        }
        return j.endArray();
    }

    // ---- reading (replay) --------------------------------------------------------------------------

    private static Spawn readSpawn(JsonObject o) {
        JsonArray p = o.getAsJsonArray("pos");
        return new Spawn(o.get("id").getAsInt(), o.get("entity").getAsString(), ScenarioReader.f64(p.get(0)),
                ScenarioReader.f64(p.get(1)), ScenarioReader.f64(p.get(2)), ScenarioReader.f32(o.get("yaw")),
                o.has("game_mode") ? o.get("game_mode").getAsInt() : 0);
    }

    private static TeamOp readTeam(JsonObject o) {
        List<String> members = new ArrayList<>();
        for (JsonElement m : o.getAsJsonArray("members")) {
            members.add(m.getAsString());
        }
        return new TeamOp(o.get("name").getAsString(), o.get("mode").getAsInt(), o.has("rule") ? o.get("rule").getAsString() : null,
                List.copyOf(members));
    }

    static void readSetup(JsonObject s, Scenario sc) {
        if (!s.has("ext")) {
            return;
        }
        JsonObject x = s.getAsJsonObject("ext");
        Ext.readSetup(x, sc);
        Setup st = setup(sc);
        for (JsonElement e : x.getAsJsonArray("teams")) {
            st.teams.add(readTeam(e.getAsJsonObject()));
        }
        for (JsonElement e : x.getAsJsonArray("entities")) {
            st.entities.add(readSpawn(e.getAsJsonObject()));
        }
    }

    /** An event this class knows, or null. */
    static Scenario.VersionEvent readEvent(JsonObject ev) {
        return switch (ev.get("type").getAsString()) {
            case "spawn" -> readSpawn(ev);
            case "entity_move" -> {
                JsonArray p = ev.getAsJsonArray("pos");
                yield new Move(ev.get("id").getAsInt(), ScenarioReader.f64(p.get(0)), ScenarioReader.f64(p.get(1)),
                        ScenarioReader.f64(p.get(2)), ScenarioReader.f32(ev.get("yaw")), ev.get("on_ground").getAsBoolean());
            }
            case "entity_rel" -> {
                JsonArray d = ev.getAsJsonArray("delta");
                yield new Rel(ev.get("id").getAsInt(), d.get(0).getAsShort(), d.get(1).getAsShort(), d.get(2).getAsShort(),
                        ev.get("on_ground").getAsBoolean());
            }
            case "entity_look" -> new Look(ev.get("id").getAsInt(), ScenarioReader.f32(ev.get("yaw")));
            case "remove_entity" -> new Remove(ev.get("id").getAsInt());
            case "team" -> readTeam(ev);
            case "game_mode" -> new GameMode(ev.get("id").getAsInt(), ev.get("game_mode").getAsInt());
            default -> null;
        };
    }
}
