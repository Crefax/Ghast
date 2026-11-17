package ghast.oracle.v1_8_9;

import ghast.oracle.Json;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import io.netty.buffer.Unpooled;
import net.minecraft.client.MinecraftClient;
import net.minecraft.entity.Entity;
import net.minecraft.entity.vehicle.BoatEntity;
import net.minecraft.network.Packet;
import net.minecraft.network.listener.ClientPlayPacketListener;
import net.minecraft.network.packet.s2c.play.EntityAttachS2CPacket;
import net.minecraft.network.packet.s2c.play.EntityPositionS2CPacket;
import net.minecraft.network.packet.s2c.play.EntityS2CPacket;
import net.minecraft.network.packet.s2c.play.EntitySpawnS2CPacket;
import net.minecraft.network.packet.s2c.play.EntityVelocityUpdateS2CPacket;
import net.minecraft.util.PacketByteBuf;
import net.minecraft.util.math.Box;

import java.util.ArrayList;
import java.util.List;

import static ghast.oracle.Scenario.BACK;
import static ghast.oracle.Scenario.FORWARD;
import static ghast.oracle.Scenario.JUMP;
import static ghast.oracle.Scenario.LEFT;
import static ghast.oracle.Scenario.RIGHT;
import static ghast.oracle.Scenario.SNEAK;
import static ghast.oracle.Scenario.SPRINT;

/**
 * 1.8.9 boat cases (added for ghast 0.4.6): the local player rides a boat. The boat is spawned by the
 * real spawn-object handler at setup, the player boards it by the server's attach-entity packet and the
 * server moves the boat with teleport, relative-move and velocity packets; every packet is written as
 * wire bytes, decoded by the packet's own {@code read} and handed to the real client handler.
 *
 * <p>The 1.8 client simulates the boat it rides: {@code onEntityAttach} clears the boat's "empty" flag
 * for the local player, and {@code BoatEntity.tick} then runs the physics branch (buoyancy, the rider's
 * forward input, the 0.35 speed cap, snow layers and lily pads removed from the client world, its move,
 * the yaw following the motion) instead of the interpolation. The player ticks as its rider right after
 * it ({@code tickRiding}: motion zeroed, its own tick, {@code look} + {@code steer_vehicle} instead of the
 * movement packets, then the seat). The server's teleport snaps the boat and zeroes its motion; a relative
 * move only restores the last server motion when the boat is more than a block off; both set the boat's
 * ground flag.
 *
 * <p>What is faked: nothing; the attach handler shows an overlay text through a HUD the harness does not
 * have, so the handler's steps are applied directly (the packet is still decoded by its own {@code
 * read}). The boat's {@code Math.sin} / {@code Math.cos} (rider input, seat) depend on the JVM's
 * intrinsics; tick records carry the values this JVM computed ({@code jvm}) so a replay can use them.
 * Tick records add {@code vehicle}: the boat after the tick.
 */
final class Boats {
    private Boats() {}

    static final int GOLDEN_FUZZ = 100;
    static final int BOAT = 100;

    /** Version setup of a riding case: the boat the server spawns. */
    static final class Setup {
        double x, y, z;
        float yaw;
    }

    static Setup of(Scenario sc) {
        return sc.versionSetup instanceof Setup s ? s : null;
    }

    /** Attach-entity packet: the player boards the boat ({@code on}) or gets off (holding entity -1). */
    record Ride(boolean on) implements Scenario.VersionEvent {}

    /** Entity-teleport packet for the boat (absolute fixed-point position, angle bytes). */
    record BoatTeleport(int x, int y, int z, int yaw, int pitch, boolean onGround) implements Scenario.VersionEvent {}

    /** Relative-move packet for the boat (fixed-point bytes). */
    record BoatMove(int dx, int dy, int dz, boolean onGround) implements Scenario.VersionEvent {}

    /** Entity-velocity packet for the boat (1/8000 blocks per tick). */
    record BoatVelocity(int x, int y, int z) implements Scenario.VersionEvent {}

    // ---- packets ----------------------------------------------------------------------------------

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

    private static PacketByteBuf buf() {
        return new PacketByteBuf(Unpooled.buffer());
    }

    /** After the player's setup: the server spawns the boat (spawn-object packet, type 1, no motion). */
    static void setup(Harness.Run run) {
        inSet = false;
        Setup s = of(run.sc);
        if (s == null) {
            return;
        }
        // The join packet's entity id (the JVM-wide entity counter could reach the boat's id).
        run.player.setEntityId(1);
        PacketByteBuf b = buf();
        b.writeVarInt(BOAT);
        b.writeByte(1);
        b.writeInt((int) Math.floor(s.x * 32.0)).writeInt((int) Math.floor(s.y * 32.0)).writeInt((int) Math.floor(s.z * 32.0));
        b.writeByte(0).writeByte((byte) Math.floor(s.yaw * 256.0F / 360.0F));
        b.writeInt(0);
        handle(run, new EntitySpawnS2CPacket(), b);
    }

    static boolean apply(Harness.Run run, Scenario.Event e) {
        if (e instanceof Ride r) {
            PacketByteBuf b = buf();
            b.writeInt(run.player.getEntityId()).writeInt(r.on() ? BOAT : -1).writeByte(0);
            EntityAttachS2CPacket p = new EntityAttachS2CPacket();
            try {
                p.read(b);
            } catch (java.io.IOException ex) {
                throw new IllegalStateException(ex);
            }
            // ClientPlayNetworkHandler.onEntityAttach for the local player, without its overlay text.
            Entity v = run.world.getEntityById(p.getHoldingEntityId());
            if (v instanceof BoatEntity boat) {
                boat.method_3052(false);
            }
            run.player.startRiding(v);
            return true;
        }
        if (e instanceof BoatTeleport t) {
            PacketByteBuf b = buf();
            b.writeVarInt(BOAT);
            b.writeInt(t.x()).writeInt(t.y()).writeInt(t.z());
            b.writeByte(t.yaw()).writeByte(t.pitch()).writeBoolean(t.onGround());
            handle(run, new EntityPositionS2CPacket(), b);
            return true;
        }
        if (e instanceof BoatMove m) {
            PacketByteBuf b = buf();
            b.writeVarInt(BOAT);
            b.writeByte(m.dx()).writeByte(m.dy()).writeByte(m.dz()).writeBoolean(m.onGround());
            handle(run, new EntityS2CPacket.MoveRelative(), b);
            return true;
        }
        if (e instanceof BoatVelocity v) {
            PacketByteBuf b = buf();
            b.writeVarInt(BOAT);
            b.writeShort(v.x()).writeShort(v.y()).writeShort(v.z());
            handle(run, new EntityVelocityUpdateS2CPacket(), b);
            return true;
        }
        return false;
    }

    // ---- the JVM's trigonometry -------------------------------------------------------------------

    /** The rider-input angle of this tick's boat tick and the JVM's sine and cosine of it. */
    private static float inArg;
    private static double inSin, inCos;
    private static boolean inSet;

    /** Before {@code World.tickEntities}: the rider's input angle as the boat will compute it. */
    static void beforeTick(Harness.Run run) {
        inSet = false;
        if (of(run.sc) == null || !(run.player.vehicle instanceof BoatEntity)) {
            return;
        }
        float aa = run.player.yaw + -run.player.sidewaysSpeed * 90.0F;
        inArg = aa * (float) Math.PI / 180.0F;
        inSin = Math.sin(inArg);
        inCos = Math.cos(inArg);
        inSet = true;
    }

    // ---- recording --------------------------------------------------------------------------------

    /** The boat after the tick (riding cases, while it exists), and the JVM's trigonometry of the tick. */
    static void tickRecord(Json j, Harness.Run run) {
        if (of(run.sc) == null) {
            return;
        }
        Entity e = run.world.getEntityById(BOAT);
        if (e instanceof BoatEntity b) {
            Class<?> c = BoatEntity.class;
            j.key("vehicle").beginObject().field("id", BOAT).field("riding", run.player.vehicle == b);
            j.key("pos").beginArray().f64(b.x).f64(b.y).f64(b.z).endArray();
            j.key("vel").beginArray().f64(b.velocityX).f64(b.velocityY).f64(b.velocityZ).endArray();
            j.fieldF32("yaw", b.yaw).fieldF32("pitch", b.pitch);
            Box bb = b.getBoundingBox();
            j.key("bb").beginArray().f64(bb.minX).f64(bb.minY).f64(bb.minZ).f64(bb.maxX).f64(bb.maxY).f64(bb.maxZ).endArray();
            j.field("on_ground", b.onGround).field("horizontal_collision", b.horizontalCollision)
                    .field("vertical_collision", b.verticalCollision);
            j.fieldF32("fall_distance", b.fallDistance);
            j.field("in_web", Reflect.getBoolean(Entity.class, b, "inWeb"));
            j.field("removed", b.removed);
            j.field("empty", Reflect.getBoolean(c, b, "field_3877"));
            j.fieldF64("speed", Reflect.getDouble(c, b, "field_3879"));
            j.field("lerp_steps", Reflect.getInt(c, b, "field_3880"));
            j.key("server_vel").beginArray().f64(Reflect.getDouble(c, b, "field_3886")).f64(Reflect.getDouble(c, b, "field_3887"))
                    .f64(Reflect.getDouble(c, b, "field_3878")).endArray();
            j.endObject();
            if (run.player.vehicle == b || inSet) {
                j.key("jvm").beginObject();
                if (inSet) {
                    j.fieldF32("in_arg", inArg).fieldF64("in_sin", inSin).fieldF64("in_cos", inCos);
                }
                double a = b.yaw * Math.PI / 180.0;
                j.fieldF64("seat_arg", a).fieldF64("seat_cos", Math.cos(a)).fieldF64("seat_sin", Math.sin(a));
                j.endObject();
            }
        }
    }

    static boolean event(Json j, Scenario.Event e) {
        if (e instanceof Ride r) {
            j.field("type", "ride").field("on", r.on());
        } else if (e instanceof BoatTeleport t) {
            j.field("type", "boat_teleport").field("x", t.x()).field("y", t.y()).field("z", t.z()).field("yaw", t.yaw())
                    .field("pitch", t.pitch()).field("on_ground", t.onGround());
        } else if (e instanceof BoatMove m) {
            j.field("type", "boat_move").field("dx", m.dx()).field("dy", m.dy()).field("dz", m.dz()).field("on_ground", m.onGround());
        } else if (e instanceof BoatVelocity v) {
            j.field("type", "boat_velocity").field("x", v.x()).field("y", v.y()).field("z", v.z());
        } else {
            return false;
        }
        return true;
    }

    static Scenario.Event read(com.google.gson.JsonObject ev) {
        return switch (ev.get("type").getAsString()) {
            case "ride" -> new Ride(ev.get("on").getAsBoolean());
            case "boat_teleport" -> new BoatTeleport(ev.get("x").getAsInt(), ev.get("y").getAsInt(), ev.get("z").getAsInt(),
                    ev.get("yaw").getAsInt(), ev.get("pitch").getAsInt(), ev.get("on_ground").getAsBoolean());
            case "boat_move" -> new BoatMove(ev.get("dx").getAsInt(), ev.get("dy").getAsInt(), ev.get("dz").getAsInt(),
                    ev.get("on_ground").getAsBoolean());
            case "boat_velocity" -> new BoatVelocity(ev.get("x").getAsInt(), ev.get("y").getAsInt(), ev.get("z").getAsInt());
            default -> null;
        };
    }

    /** The header's {@code setup.ext} of a riding case: the boat's spawn. */
    static void header(Json j, Scenario sc) {
        Setup s = of(sc);
        if (s == null) {
            return;
        }
        j.key("ext").beginObject().key("boat").beginObject();
        j.key("pos").beginArray().f64(s.x).f64(s.y).f64(s.z).endArray();
        j.fieldF32("yaw", s.yaw).field("id", BOAT);
        j.endObject().endObject();
    }

    static void readHeader(Scenario sc, com.google.gson.JsonObject ext) {
        if (!ext.has("boat")) {
            return;
        }
        com.google.gson.JsonObject o = ext.getAsJsonObject("boat");
        com.google.gson.JsonArray p = o.getAsJsonArray("pos");
        Setup s = new Setup();
        s.x = ScenarioReader.f64(p.get(0));
        s.y = ScenarioReader.f64(p.get(1));
        s.z = ScenarioReader.f64(p.get(2));
        s.yaw = ScenarioReader.f32(o.get("yaw"));
        sc.versionSetup = s;
    }

    // ---- cases ------------------------------------------------------------------------------------

    private static void addEvent(Scenario sc, int tick, Scenario.Event e) {
        Scenario.Tick tk = sc.ticks.get(tick);
        List<Scenario.Event> events = new ArrayList<>(tk.events());
        events.add(e);
        sc.ticks.set(tick, new Scenario.Tick(tk.keys(), tk.yaw(), tk.pitch(), List.copyOf(events)));
    }

    /** A case with a boat at a position; the player boards it in the first tick. */
    private static Scenario boat(Scenario sc, double x, double y, double z, float yaw) {
        Setup s = new Setup();
        s.x = x;
        s.y = y;
        s.z = z;
        s.yaw = yaw;
        sc.versionSetup = s;
        addEvent(sc, 0, new Ride(true));
        return sc;
    }

    private static Scenario pond(String name) {
        return new Scenario(name).fill(-24, 58, -24, 24, 58, 24, Blocks.STONE, 0).fill(-24, 59, -24, 24, 62, 24, Blocks.WATER, 0);
    }

    private static Scenario flat(String name) {
        return new Scenario(name).fill(-24, 63, -24, 24, 63, 24, Blocks.STONE, 0);
    }

    static List<Scenario> golden() {
        List<Scenario> l = new ArrayList<>(all());
        for (int i = 0; i < GOLDEN_FUZZ; i++) {
            l.add(fuzz(1, i, 60));
        }
        return l;
    }

    static List<Scenario> all() {
        Harness.init();
        List<Scenario> l = new ArrayList<>();
        l.add(boat(pond("boat_water_paddle").desc("paddle a boat on still water: forward, turns, back, idle drift")
                .spawn(0.5, 63, 0.5, 0, 0).run(5, 0).run(60, FORWARD).run(15, FORWARD | LEFT).run(15, RIGHT).run(10, LEFT)
                .run(15, BACK).run(10, LEFT | RIGHT).run(30, 0), 0.5, 62.6, 0.5, 0.0F));
        l.add(boat(pond("boat_look_steer").desc("steer by looking around while paddling (the boat follows the rider's yaw)")
                .spawn(0.5, 63, 0.5, 0, 0).run(5, 0).run(20, FORWARD, 60, 10).run(20, FORWARD, -90, -20).run(20, FORWARD | RIGHT, 200, 30)
                .run(20, BACK | LEFT, 20, 0), 0.5, 62.6, 0.5, 30.0F));
        l.add(boat(flat("boat_land").desc("a boat on stone: paddling on land (halved on the ground)")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(40, FORWARD).run(20, FORWARD | RIGHT).run(10, 0), 0.5, 64, 0.5, 0.0F));
        Scenario ice = new Scenario("boat_ice").fill(-24, 63, -24, 24, 63, 24, Blocks.PACKED_ICE, 0).fill(-24, 63, 6, 24, 63, 24, Blocks.ICE, 0);
        l.add(boat(ice.desc("a boat on packed ice and ice (no slipperiness for a boat in 1.8)").spawn(0.5, 64, 0.5, 0, 0).run(3, 0)
                .run(60, FORWARD).run(20, 0), 0.5, 64, 0.5, 0.0F));
        l.add(boat(pond("boat_drop_into_water").desc("a boat falls from the air into water").spawn(0.5, 70, 0.5, 0, 0).run(80, FORWARD),
                0.5, 69, 0.5, 0.0F));
        l.add(boat(flat("boat_drop_on_land").desc("a boat falls 6 blocks onto stone").spawn(0.5, 71, 0.5, 0, 0).run(40, FORWARD),
                0.5, 70, 0.5, 0.0F));
        Scenario wall = pond("boat_wall").fill(-3, 59, 6, 3, 65, 6, Blocks.STONE, 0).fill(5, 59, -3, 5, 65, 3, Blocks.PLANKS, 0);
        l.add(boat(wall.desc("paddle into walls head-on and at an angle (fast: the server would break it)").spawn(0.5, 63, 0.5, 0, 0)
                .run(3, 0).run(60, FORWARD).run(30, FORWARD, 90, 0).run(20, FORWARD), 0.5, 62.6, 0.5, 0.0F));
        Scenario flow = pond("boat_flowing").fill(-24, 62, -24, 24, 62, 24, Blocks.AIR, 0).fill(-24, 61, -2, 24, 61, 2, Blocks.FLOWING_WATER, 1)
                .fill(-24, 61, 3, 24, 61, 8, Blocks.FLOWING_WATER, 3);
        l.add(boat(flow.desc("flowing water pushes the boat; paddle against and along it").spawn(0.5, 62, 0.5, 90, 0).run(3, 0).run(30, 0)
                .run(30, FORWARD).run(30, BACK), 0.5, 61.6, 0.5, 90.0F));
        Scenario deep = pond("boat_under_water").fill(-24, 63, -24, 24, 64, 24, Blocks.WATER, 0).fill(-24, 65, -24, 24, 65, 24, Blocks.GLASS, 0);
        l.add(boat(deep.desc("a boat held under a water roof").spawn(0.5, 62, 0.5, 0, 0).run(60, FORWARD), 0.5, 61.5, 0.5, 0.0F));
        Scenario step = flat("boat_slab_soul_sand").fill(-3, 64, 4, 3, 64, 4, Blocks.STONE_SLAB, 0).fill(-3, 64, 7, 3, 64, 9, Blocks.SOUL_SAND, 0);
        l.add(boat(step.desc("a boat on land meets a slab (no step-up) and soul sand").spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(60, FORWARD),
                0.5, 64, 0.5, 0.0F));
        Scenario slime = flat("boat_slime").fill(-3, 63, -3, 3, 63, 3, Blocks.SLIME, 0);
        l.add(boat(slime.desc("a boat dropped onto slime bounces").spawn(0.5, 69, 0.5, 0, 0).run(50, 0), 0.5, 68, 0.5, 0.0F));
        Scenario lily = pond("boat_lily_pads_snow").fill(-3, 63, 3, 3, 63, 9, Blocks.LILY_PAD, 0).fill(-3, 63, 12, 3, 63, 20, Blocks.STONE, 0)
                .fill(-3, 64, 12, 3, 64, 20, Blocks.SNOW_LAYER, 1);
        l.add(boat(lily.desc("paddle over lily pads and onto snow layers (the client removes both)").spawn(0.5, 63, 0.5, 0, 0).run(3, 0)
                .run(80, FORWARD), 0.5, 62.6, 0.5, 0.0F));
        Scenario web = flat("boat_cobweb").fill(-1, 64, 4, 1, 64, 5, Blocks.WEB, 0);
        l.add(boat(web.desc("paddle into cobwebs on land").spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(50, FORWARD), 0.5, 64, 0.5, 0.0F));
        Scenario lava = new Scenario("boat_lava").fill(-24, 63, -24, 24, 63, 24, Blocks.STONE, 0).fill(-2, 63, 3, 2, 63, 9, Blocks.LAVA, 0)
                .fill(-3, 62, 2, 3, 62, 10, Blocks.STONE, 0);
        l.add(boat(lava.desc("a boat paddled into a lava pool").spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(60, FORWARD), 0.5, 64, 0.5, 0.0F));
        Scenario keys = boat(pond("boat_rider_keys").desc("jump, sprint and sneak keys while riding (the rider's own tick, steer_vehicle)")
                .spawn(0.5, 63, 0.5, 0, 0).run(5, JUMP).run(10, FORWARD | SPRINT).run(10, SNEAK).run(10, JUMP | FORWARD).run(10, 0),
                0.5, 62.6, 0.5, 0.0F);
        l.add(keys);
        Scenario landKeys = boat(flat("boat_land_rider_keys").desc("on land the rider's own tick starts inside the stone: jump, sprint")
                .spawn(0.5, 64, 0.5, 0, 0).run(5, JUMP).run(10, FORWARD | SPRINT | JUMP).run(10, SNEAK | FORWARD).run(10, 0), 0.5, 64, 0.5, 0.0F);
        l.add(landKeys);
        Scenario fix = boat(pond("boat_server_moves").desc("the server teleports the boat, sends its motion and relative moves (small: kept, large: motion restored)")
                .spawn(0.5, 63, 0.5, 0, 0).run(5, 0).run(60, FORWARD), 0.5, 62.6, 0.5, 0.0F);
        addEvent(fix, 10, new BoatVelocity(400, 0, 1200));
        addEvent(fix, 15, new BoatTeleport(16, 62 * 32 + 19, 64, 10, 0, false));
        addEvent(fix, 20, new BoatMove(3, 0, 5, false));
        addEvent(fix, 23, new BoatMove(40, 0, 20, false));
        addEvent(fix, 30, new BoatVelocity(-800, 100, 0));
        addEvent(fix, 31, new BoatMove(-60, 0, -60, true));
        addEvent(fix, 40, new BoatTeleport(32, 62 * 32 + 19, 96, -20, 0, true));
        l.add(fix);
        Scenario off = boat(pond("boat_dismount").desc("sneak while paddling (sent in steer_vehicle only), then the server takes the player off")
                .spawn(0.5, 63, 0.5, 0, 0).run(5, 0).run(20, FORWARD).run(5, FORWARD | SNEAK).run(3, 0), 0.5, 62.6, 0.5, 0.0F);
        addEvent(off, 30, new Ride(false));
        l.add(off);
        return l;
    }

    /**
     * Randomized boat cases: a terrain of water pools, land, ice, snow and walls around a boat the player
     * boards in the first tick; random paddling keys and looks, the server's teleports, relative moves and
     * motion, and sometimes a dismount near the end. Cases get a {@code _bo} suffix.
     */
    static Scenario fuzz(long seed, int index, int ticks) {
        Harness.init();
        Rng x = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x626f617473L);
        Scenario sc = new Scenario("fuzz_s" + seed + "_" + index + "_bo");
        sc.fill(-32, 58, -32, 32, 58, 32, Blocks.STONE, 0);
        int[][] grounds = {{Blocks.WATER, 0}, {Blocks.WATER, 0}, {Blocks.WATER, 0}, {Blocks.STONE, 0}, {Blocks.GRASS, 0}, {Blocks.ICE, 0},
                {Blocks.PACKED_ICE, 0}, {Blocks.SOUL_SAND, 0}, {Blocks.SLIME, 0}, {Blocks.SAND, 0}, {Blocks.FLOWING_WATER, 2},
                {Blocks.FLOWING_WATER, 5}, {Blocks.LILY_PAD, 0}, {Blocks.WEB, 0}, {Blocks.LAVA, 0}, {Blocks.SNOW_LAYER, 0}};
        int patches = x.range(3, 12);
        for (int i = 0; i < patches; i++) {
            int cx = x.range(-12, 12), cz = x.range(-12, 12), r = x.range(1, 6);
            int[] g = grounds[x.nextInt(grounds.length)];
            int top = x.chance(0.7) ? 62 : x.range(60, 63);
            if (g[0] == Blocks.LILY_PAD) {
                sc.fill(cx - r, 59, cz - r, cx + r, 62, cz + r, Blocks.WATER, 0).fill(cx - r, 63, cz - r, cx + r, 63, cz + r, Blocks.LILY_PAD, 0);
            } else if (g[0] == Blocks.WEB) {
                sc.fill(cx - r, 63, cz - r, cx + r, 63, cz + r, Blocks.WEB, 0);
            } else if (g[0] == Blocks.SNOW_LAYER) {
                sc.fill(cx - r, 59, cz - r, cx + r, top, cz + r, Blocks.STONE, 0)
                        .fill(cx - r, top + 1, cz - r, cx + r, top + 1, cz + r, Blocks.SNOW_LAYER, x.range(0, 7));
            } else if (g[0] == Blocks.WATER || g[0] == Blocks.FLOWING_WATER) {
                sc.fill(cx - r, 59, cz - r, cx + r, top, cz + r, Blocks.WATER, 0);
                if (g[0] == Blocks.FLOWING_WATER) {
                    sc.fill(cx - r, top, cz - r, cx + r, top, cz + r, g[0], g[1]);
                }
            } else {
                sc.fill(cx - r, 59, cz - r, cx + r, top, cz + r, g[0], g[1]);
            }
        }
        int walls = x.range(0, 5);
        int[][] wallBlocks = {{Blocks.STONE, 0}, {Blocks.PLANKS, 0}, {Blocks.FENCE, 0}, {Blocks.COBBLESTONE_WALL, 0}, {Blocks.STONE_SLAB, 0},
                {Blocks.GLASS_PANE, 0}};
        for (int i = 0; i < walls; i++) {
            int cx = x.range(-10, 10), cz = x.range(-10, 10), len = x.range(1, 6);
            int[] w = wallBlocks[x.nextInt(wallBlocks.length)];
            if (x.chance(0.5)) {
                sc.fill(cx, 63, cz, cx + len, 63 + x.range(0, 2), cz, w[0], w[1]);
            } else {
                sc.fill(cx, 63, cz, cx, 63 + x.range(0, 2), cz + len, w[0], w[1]);
            }
        }
        if (x.chance(0.2)) {
            int cx = x.range(-8, 8), cz = x.range(-8, 8);
            sc.fill(cx - 3, 59, cz - 3, cx + 3, 64, cz + 3, Blocks.WATER, 0).fill(cx - 3, 65, cz - 3, cx + 3, 65, cz + 3, Blocks.GLASS, 0);
        }
        double bx = x.range(-3, 3) + 0.5, bz = x.range(-3, 3) + 0.5;
        double by = x.chance(0.8) ? 63.2 : 63 + x.range(1, 6);
        float yaw = (float) (x.nextDouble() * 360 - 180);
        sc.spawn(bx, by + 0.5, bz, yaw, 0);
        float look = yaw, pitch = 0;
        int[] keySets = {0, FORWARD, FORWARD, FORWARD, FORWARD | LEFT, FORWARD | RIGHT, LEFT, RIGHT, BACK, BACK | LEFT, LEFT | RIGHT,
                FORWARD | BACK, FORWARD | JUMP, FORWARD | SPRINT, SNEAK, JUMP};
        int keys = 0;
        for (int t = 0; t < ticks; t++) {
            if (x.chance(0.12)) {
                keys = keySets[x.nextInt(keySets.length)];
            }
            if (x.chance(0.25)) {
                look += (float) ((x.nextDouble() - 0.5) * 90);
                pitch = (float) Math.max(-90, Math.min(90, pitch + (x.nextDouble() - 0.5) * 40));
            }
            sc.run(1, keys, look, pitch);
        }
        boat(sc, bx, by, bz, yaw);
        for (int t = 3; t < ticks; t++) {
            if (x.chance(0.02)) {
                addEvent(sc, t, new BoatTeleport((int) Math.floor((bx + (x.nextDouble() - 0.5) * 6) * 32), (int) Math.floor((63 + x.nextDouble()) * 32),
                        (int) Math.floor((bz + (x.nextDouble() - 0.5) * 6) * 32), x.range(-128, 127), 0, x.chance(0.5)));
            }
            if (x.chance(0.03)) {
                int s = x.chance(0.3) ? 60 : 8;
                addEvent(sc, t, new BoatMove(x.range(-s, s), x.range(-4, 4), x.range(-s, s), x.chance(0.3)));
            }
            if (x.chance(0.02)) {
                addEvent(sc, t, new BoatVelocity(x.range(-3000, 3000), x.range(-800, 800), x.range(-3000, 3000)));
            }
        }
        if (x.chance(0.15)) {
            // Near the end: the 1.8 player is solid to boats only on the server side here.
            addEvent(sc, ticks - x.range(1, 3), new Ride(false));
        }
        sc.description = "randomized boat case: pools, land, ice, snow, walls; paddling keys, looks, server moves";
        return sc;
    }
}
