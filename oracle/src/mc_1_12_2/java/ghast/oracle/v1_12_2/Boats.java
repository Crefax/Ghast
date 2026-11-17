package ghast.oracle.v1_12_2;

import ghast.oracle.Json;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import io.netty.buffer.Unpooled;
import net.minecraft.client.MinecraftClient;
import net.minecraft.entity.Entity;
import net.minecraft.entity.vehicle.BoatEntity;
import net.minecraft.network.Packet;
import net.minecraft.network.listener.ClientPlayPacketListener;
import net.minecraft.network.packet.s2c.play.SetPassengersS2CPacket;
import net.minecraft.network.packet.s2c.play.VehicleMoveS2CPacket;
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
 * 1.12.2 boat cases (added for ghast 0.4.6): the local player rides a boat it controls. The boat is
 * spawned by the real spawn-object handler, the player boards it by the server's set-passengers packet
 * and the server corrects it with move-vehicle packets; every packet is written as wire bytes and
 * decoded by the packet's own {@code read}, then handed to the real client handler.
 *
 * <p>The client then ticks the boat in {@code World.tickEntities} ({@code BoatEntity.tick}: location,
 * velocity, paddles, {@code steer_boat}, its move) and the player as its passenger right after it
 * ({@code tickRiding}: the player's own tick with zeroed motion, {@code look} + {@code steer_vehicle} +
 * {@code vehicle_move} instead of the movement packets, then the seat). Unlike 1.16 the 1.12 client
 * never dismounts by itself: sneaking only goes to the server in {@code steer_vehicle}.
 *
 * <p>What is faked: nothing beyond {@link Ent}'s. Mouse look is applied as the renderer does
 * ({@code Entity.increaseTransforms} ends with the vehicle's {@code onPassengerLookAround}, the
 * 105-degree clamp). Tick records carry {@code vehicle}: the boat's state after the tick.
 */
final class Boats {
    private Boats() {}

    static final int GOLDEN_FUZZ = 100;
    static final int BOAT = 100;

    /** Version setup of a riding case. */
    static final class Setup extends Ent.Setup {}

    static Setup of(Scenario sc) {
        return sc.versionSetup instanceof Setup s ? s : null;
    }

    static Setup setup(Scenario sc) {
        if (!(sc.versionSetup instanceof Setup)) {
            Ent.Setup old = Ent.of(sc);
            Setup s = new Setup();
            if (old != null) {
                s.events.addAll(old.events);
            }
            sc.versionSetup = s;
        }
        return (Setup) sc.versionSetup;
    }

    /** Set-passengers packet: {@code passengers} (entity ids; the player is 1) ride {@code vehicle}. */
    record Ride(int vehicle, int[] passengers) implements Scenario.VersionEvent {}

    /** Move-vehicle packet: the server's correction of the player's vehicle. */
    record VehicleMove(double x, double y, double z, float yaw, float pitch) implements Scenario.VersionEvent {}

    // ---- packets --------------------------------------------------------------------------------

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

    static boolean apply(Harness.Run run, Scenario.Event e) {
        if (e instanceof Ride r) {
            PacketByteBuf b = new PacketByteBuf(Unpooled.buffer());
            b.writeVarInt(r.vehicle());
            b.writeVarInt(r.passengers().length);
            for (int id : r.passengers()) {
                b.writeVarInt(id);
            }
            // ClientPlayNetworkHandler.onSetPassengers without its overlay message (the "press sneak to
            // dismount" text needs the language storage): the passengers leave, then each listed entity
            // starts riding (forced). The packet is still decoded by its own read.
            SetPassengersS2CPacket p = new SetPassengersS2CPacket();
            try {
                p.read(b);
            } catch (java.io.IOException ex) {
                throw new IllegalStateException(ex);
            }
            Entity v = run.world.getEntityById(p.getEntityId());
            if (v != null) {
                v.removeAllPassengers();
                for (int id : p.getPassengerIds()) {
                    Entity e2 = run.world.getEntityById(id);
                    if (e2 != null) {
                        e2.startRiding(v, true);
                    }
                }
            }
            return true;
        }
        if (e instanceof VehicleMove m) {
            PacketByteBuf b = new PacketByteBuf(Unpooled.buffer());
            b.writeDouble(m.x()).writeDouble(m.y()).writeDouble(m.z());
            b.writeFloat(m.yaw()).writeFloat(m.pitch());
            // ClientPlayNetworkHandler.onVehicleMove, whose answer goes to the connection directly
            // (the harness has none): the same steps, the answer captured like every sent packet.
            VehicleMoveS2CPacket p = new VehicleMoveS2CPacket();
            try {
                p.read(b);
            } catch (java.io.IOException ex) {
                throw new IllegalStateException(ex);
            }
            Entity v = run.player.getRootVehicle();
            if (v != run.player && v.method_13003()) {
                v.updatePositionAndAngles(p.getX(), p.getY(), p.getZ(), p.getYaw(), p.getPitch());
                run.net.sendPacket(new net.minecraft.network.packet.c2s.play.VehicleMoveC2SPacket(v));
            }
            return true;
        }
        return false;
    }

    /** Mouse turning of a passenger ({@code Entity.increaseTransforms}): the vehicle then clamps it. */
    static void afterTurn(Harness.Run run) {
        Entity v = run.player.getVehicle();
        if (v != null) {
            v.onPassengerLookAround(run.player);
        }
    }

    // ---- recording --------------------------------------------------------------------------------

    /** The player's vehicle after the tick (riding cases): everything the boat's client tick reads and writes. */
    static void tickRecord(Json j, Harness.Run run) {
        if (of(run.sc) != null && run.player.getVehicle() instanceof net.minecraft.entity.vehicle.AbstractMinecartEntity m) {
            Class<?> c = net.minecraft.entity.vehicle.AbstractMinecartEntity.class;
            j.key("vehicle").beginObject().field("id", m.getEntityId()).field("type", "minecart");
            j.key("pos").beginArray().f64(m.x).f64(m.y).f64(m.z).endArray();
            j.fieldF32("yaw", m.yaw).fieldF32("pitch", m.pitch);
            Box bb = m.getBoundingBox();
            j.key("bb").beginArray().f64(bb.minX).f64(bb.minY).f64(bb.minZ).f64(bb.maxX).f64(bb.maxY).f64(bb.maxZ).endArray();
            j.field("lerp_steps", Reflect.getInt(c, m, "clientInterpolationSteps"));
            j.endObject();
            return;
        }
        if (of(run.sc) == null || !(run.player.getVehicle() instanceof BoatEntity b)) {
            return;
        }
        Class<?> c = BoatEntity.class;
        j.key("vehicle").beginObject().field("id", b.getEntityId()).field("type", "boat");
        j.key("pos").beginArray().f64(b.x).f64(b.y).f64(b.z).endArray();
        j.key("vel").beginArray().f64(b.velocityX).f64(b.velocityY).f64(b.velocityZ).endArray();
        j.fieldF32("yaw", b.yaw).fieldF32("pitch", b.pitch);
        Box bb = b.getBoundingBox();
        j.key("bb").beginArray().f64(bb.minX).f64(bb.minY).f64(bb.minZ).f64(bb.maxX).f64(bb.maxY).f64(bb.maxZ).endArray();
        j.field("on_ground", b.onGround).field("horizontal_collision", b.horizontalCollision).field("vertical_collision", b.verticalCollision);
        j.fieldF32("fall_distance", b.fallDistance);
        Object st = Reflect.get(c, b, "location"), old = Reflect.get(c, b, "lastLocation");
        j.field("status", st == null ? null : st.toString().toLowerCase());
        j.field("old_status", old == null ? null : old.toString().toLowerCase());
        j.fieldF32("delta_rotation", (Float) Reflect.get(c, b, "yawVelocity"));
        j.fieldF32("out_of_control", (Float) Reflect.get(c, b, "ticksUnderwater"));
        j.fieldF64("water_level", (Double) Reflect.get(c, b, "waterLevel"));
        j.fieldF32("land_friction", (Float) Reflect.get(c, b, "field_12217"));
        j.fieldF64("last_yd", (Double) Reflect.get(c, b, "fallVelocity"));
        j.key("input").beginArray().value((Boolean) Reflect.get(c, b, "pressingLeft")).value((Boolean) Reflect.get(c, b, "pressingRight"))
                .value((Boolean) Reflect.get(c, b, "pressingForward")).value((Boolean) Reflect.get(c, b, "pressingBack")).endArray();
        j.key("paddle").beginArray().value(b.isPaddleMoving(0)).value(b.isPaddleMoving(1)).endArray();
        j.field("lerp_steps", Reflect.getInt(c, b, "field_12223"));
        j.endObject();
    }

    static boolean event(Json j, Scenario.Event e) {
        if (e instanceof Ride r) {
            j.field("type", "ride").field("vehicle", r.vehicle()).key("passengers").beginArray();
            for (int id : r.passengers()) {
                j.value(id);
            }
            j.endArray();
            return true;
        }
        if (e instanceof VehicleMove m) {
            j.field("type", "vehicle_move").key("pos").beginArray().f64(m.x()).f64(m.y()).f64(m.z()).endArray();
            j.fieldF32("yaw", m.yaw()).fieldF32("pitch", m.pitch());
            return true;
        }
        return false;
    }

    static Scenario.Event read(com.google.gson.JsonObject ev) {
        return switch (ev.get("type").getAsString()) {
            case "ride" -> {
                com.google.gson.JsonArray a = ev.getAsJsonArray("passengers");
                int[] ids = new int[a.size()];
                for (int i = 0; i < ids.length; i++) {
                    ids[i] = a.get(i).getAsInt();
                }
                yield new Ride(ev.get("vehicle").getAsInt(), ids);
            }
            case "vehicle_move" -> {
                com.google.gson.JsonArray p = ev.getAsJsonArray("pos");
                yield new VehicleMove(ScenarioReader.f64(p.get(0)), ScenarioReader.f64(p.get(1)), ScenarioReader.f64(p.get(2)),
                        ScenarioReader.f32(ev.get("yaw")), ScenarioReader.f32(ev.get("pitch")));
            }
            default -> null;
        };
    }

    /** The header's {@code setup.ext} object of a riding case. */
    static void header(Json j, Scenario sc) {
        if (of(sc) != null) {
            j.key("ext").beginObject().field("ride", true).endObject();
        }
    }

    /** {@code setup.ext} with {@code ride}: keeps the setup events {@link Ent} read already. */
    static void readHeader(Scenario sc) {
        setup(sc);
    }

    // ---- cases ------------------------------------------------------------------------------------

    private static void addEvent(Scenario sc, int tick, Scenario.Event e) {
        Scenario.Tick tk = sc.ticks.get(tick);
        List<Scenario.Event> events = new ArrayList<>(tk.events());
        events.add(e);
        sc.ticks.set(tick, new Scenario.Tick(tk.keys(), tk.yaw(), tk.pitch(), List.copyOf(events)));
    }

    /** A case with a boat at a position; the player boards it in the first tick (set-passengers). */
    private static Scenario boat(Scenario sc, double x, double y, double z, float yaw) {
        Setup st = setup(sc);
        st.events.add(new Ent.Spawn(BOAT, "boat", x, y, z, yaw));
        addEvent(sc, 0, new Ride(BOAT, new int[] {Ent.PLAYER_ENTITY_ID}));
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
                .spawn(0.5, 63, 0.5, 0, 0).run(5, 0).run(40, FORWARD).run(15, FORWARD | LEFT).run(15, RIGHT).run(10, LEFT)
                .run(15, BACK).run(10, LEFT | RIGHT).run(10, FORWARD | BACK).run(30, 0), 0.5, 62.6, 0.5, 0.0F));
        l.add(boat(pond("boat_turn_look").desc("turn in place while looking around (rotation clamp to 105 degrees)")
                .spawn(0.5, 63, 0.5, 0, 0).run(5, 0).run(20, LEFT, 150, 10).run(20, RIGHT, -40, -20).run(20, FORWARD, 200, 30), 0.5, 62.6, 0.5, 30.0F));
        l.add(boat(flat("boat_land").desc("a boat on stone: paddling on land (land friction, halved each tick)")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(30, FORWARD).run(20, FORWARD | RIGHT).run(10, 0), 0.5, 64, 0.5, 0.0F));
        Scenario ice = new Scenario("boat_ice").fill(-24, 63, -24, 24, 63, 24, Blocks.PACKED_ICE, 0).fill(-24, 63, 6, 24, 63, 24, Blocks.ICE, 0)
                .fill(-24, 64, 30, 24, 66, 30, Blocks.STONE, 0);
        l.add(boat(ice.desc("a boat on packed ice and ice: speed up, turn").spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(60, FORWARD)
                .run(20, FORWARD | LEFT).run(20, 0), 0.5, 64, 0.5, 0.0F));
        l.add(boat(pond("boat_drop_into_water").desc("a boat falls from the air into water (IN_AIR to IN_WATER snaps to the surface)")
                .spawn(0.5, 70, 0.5, 0, 0).run(60, FORWARD), 0.5, 69, 0.5, 0.0F));
        l.add(boat(flat("boat_drop_on_land").desc("a boat falls 6 blocks onto stone (fall damage path, no removal on the client)")
                .spawn(0.5, 71, 0.5, 0, 0).run(40, FORWARD), 0.5, 70, 0.5, 0.0F));
        Scenario wall = pond("boat_wall").fill(-3, 59, 5, 3, 65, 5, Blocks.STONE, 0).fill(4, 59, -3, 4, 65, 3, Blocks.PLANKS, 0);
        l.add(boat(wall.desc("paddle into walls head-on and at an angle").spawn(0.5, 63, 0.5, 0, 0).run(3, 0).run(40, FORWARD)
                .run(20, FORWARD | RIGHT).run(20, FORWARD), 0.5, 62.6, 0.5, 0.0F));
        Scenario flow = pond("boat_flowing").fill(-24, 62, -24, 24, 62, 24, Blocks.AIR, 0).fill(-24, 61, -2, 24, 61, 2, Blocks.FLOWING_WATER, 1)
                .fill(-24, 61, 3, 24, 61, 8, Blocks.FLOWING_WATER, 3);
        l.add(boat(flow.desc("flowing water pushes the boat; paddle against and along it").spawn(0.5, 62, 0.5, 90, 0).run(3, 0).run(30, 0)
                .run(30, FORWARD).run(30, BACK), 0.5, 61.6, 0.5, 90.0F));
        Scenario under = pond("boat_under_water").fill(-24, 63, -24, 24, 64, 24, Blocks.WATER, 0).fill(-24, 65, -24, 24, 65, 24, Blocks.GLASS, 0);
        l.add(boat(under.desc("a boat held under a water roof (UNDER_WATER)").spawn(0.5, 62, 0.5, 0, 0).run(80, FORWARD), 0.5, 61.5, 0.5, 0.0F));
        Scenario step = flat("boat_slab_soul_sand").fill(-3, 64, 4, 3, 64, 4, Blocks.STONE_SLAB, 0).fill(-3, 64, 7, 3, 64, 9, Blocks.SOUL_SAND, 0);
        l.add(boat(step.desc("a boat on land meets a slab (no step-up) and soul sand").spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(60, FORWARD),
                0.5, 64, 0.5, 0.0F));
        Scenario slime = flat("boat_slime").fill(-3, 63, -3, 3, 63, 3, Blocks.SLIME, 0);
        l.add(boat(slime.desc("a boat dropped onto slime bounces (x0.8, not living)").spawn(0.5, 69, 0.5, 0, 0).run(50, 0), 0.5, 68, 0.5, 0.0F));
        Scenario two = pond("boat_hits_boat").desc("paddle into another boat (entity collision)").spawn(0.5, 63, 0.5, 0, 0).run(3, 0).run(50, FORWARD);
        setup(two).events.add(new Ent.Spawn(BOAT + 1, "boat", 0.6, 62.6, 4.5, 90.0F));
        l.add(boat(two, 0.5, 62.6, 0.5, 0.0F));
        Scenario mob = pond("boat_hits_mob").desc("paddle into a zombie standing on the bottom (living entities block the boat)")
                .fill(-24, 59, 3, 24, 62, 6, Blocks.AIR, 0).fill(-24, 62, 3, 24, 62, 6, Blocks.STONE, 0)
                .spawn(0.5, 63, 0.5, 0, 0).run(3, 0).run(40, FORWARD);
        setup(mob).events.add(new Ent.Spawn(BOAT + 1, "zombie", 0.5, 63.0, 4.5, 0.0F));
        l.add(boat(mob, 0.5, 62.6, 0.5, 0.0F));
        Scenario carts = flat("boat_hits_minecart_armor_stand").desc("paddle on land into a minecart (pushable: blocks), past an armor stand and a bat (not pushable)")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(25, FORWARD).run(10, BACK).run(10, FORWARD | LEFT).run(30, FORWARD);
        setup(carts).events.add(new Ent.Spawn(BOAT + 1, "armor_stand", 0.5, 64.0, 2.5, 0.0F));
        setup(carts).events.add(new Ent.Spawn(BOAT + 2, "bat", -1.0, 64.0, 3.0, 0.0F));
        setup(carts).events.add(new Ent.Spawn(BOAT + 3, "minecart", 0.5, 64.0, 5.5, 90.0F));
        l.add(boat(carts, 0.5, 64, 0.5, 0.0F));
        Scenario ladder = flat("boat_hits_climbing_mob").desc("a zombie in a ladder block climbs (not pushable): the boat drives into it and stops at the ladder")
                .fill(-1, 64, 5, 1, 65, 5, Blocks.STONE, 0).fill(0, 64, 4, 0, 65, 4, Blocks.LADDER, 2)
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(40, FORWARD);
        setup(ladder).events.add(new Ent.Spawn(BOAT + 1, "zombie", 0.5, 64.0, 4.5, 0.0F));
        l.add(boat(ladder, 0.5, 64, 0.5, 0.0F));
        Scenario lava = new Scenario("boat_lava").fill(-24, 63, -24, 24, 63, 24, Blocks.STONE, 0).fill(-2, 63, 3, 2, 63, 9, Blocks.LAVA, 0)
                .fill(-3, 62, 2, 3, 62, 10, Blocks.STONE, 0);
        l.add(boat(lava.desc("a boat on land paddled into a lava pool and out (falls in, lava halves the fall distance)").spawn(0.5, 64, 0.5, 0, 0)
                .run(3, 0).run(60, FORWARD).run(20, BACK), 0.5, 64, 0.5, 0.0F));
        Scenario fix = boat(pond("boat_server_correction").desc("the server corrects the boat (move-vehicle packets)")
                .spawn(0.5, 63, 0.5, 0, 0).run(5, 0).run(40, FORWARD), 0.5, 62.6, 0.5, 0.0F);
        addEvent(fix, 15, new VehicleMove(0.5, 62.6, 2.0, 10.0F, 0.0F));
        addEvent(fix, 25, new VehicleMove(0.5, 62.6, 3.0, 0.0F, 0.0F));
        addEvent(fix, 26, new VehicleMove(0.5, 62.6, 3.0, 0.0F, 0.0F));
        l.add(fix);
        Scenario off = boat(pond("boat_dismount").desc("sneak while paddling (sent in steer_vehicle only), then the server takes the player off")
                .spawn(0.5, 63, 0.5, 0, 0).run(5, 0).run(20, FORWARD).run(5, FORWARD | SNEAK).run(30, 0), 0.5, 62.6, 0.5, 0.0F);
        addEvent(off, 27, new Ride(BOAT, new int[0]));
        l.add(off);
        Scenario keys = boat(pond("boat_rider_keys").desc("jump, sprint and sneak keys while riding (the passenger's own tick)")
                .spawn(0.5, 63, 0.5, 0, 0).run(5, JUMP).run(10, FORWARD | SPRINT).run(10, SNEAK).run(10, JUMP | FORWARD).run(10, 0), 0.5, 62.6, 0.5, 0.0F);
        l.add(keys);
        Scenario landKeys = boat(flat("boat_land_rider_keys").desc("on land the passenger's own tick lands inside the stone: jump and sprint")
                .spawn(0.5, 64, 0.5, 0, 0).run(5, JUMP).run(10, FORWARD | SPRINT | JUMP).run(10, SNEAK | FORWARD).run(10, 0), 0.5, 64, 0.5, 0.0F);
        l.add(landKeys);
        Scenario lily = pond("boat_lily_pads").fill(-3, 63, 3, 3, 63, 12, Blocks.LILY_PAD, 0);
        l.add(boat(lily.desc("paddle over lily pads (no box for a boat; it breaks them)").spawn(0.5, 63, 0.5, 0, 0).run(3, 0).run(50, FORWARD), 0.5, 62.6, 0.5, 0.0F));
        Scenario web = flat("boat_cobweb").fill(-1, 64, 4, 1, 64, 5, Blocks.WEB, 0);
        l.add(boat(web.desc("paddle into cobwebs on land").spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(50, FORWARD), 0.5, 64, 0.5, 0.0F));
        return l;
    }

    /**
     * Randomized boat cases: a terrain of water pools, land, ice and walls around a boat the player boards
     * in the first tick; random paddling keys, looks, server corrections, other boats and mobs, dismounts.
     * Cases get a {@code _bo} suffix.
     */
    static Scenario fuzz(long seed, int index, int ticks) {
        Harness.init();
        Rng x = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x626f617473L);
        Scenario sc = new Scenario("fuzz_s" + seed + "_" + index + "_bo");
        sc.fill(-32, 58, -32, 32, 58, 32, Blocks.STONE, 0);
        int[][] grounds = {{Blocks.WATER, 0}, {Blocks.WATER, 0}, {Blocks.WATER, 0}, {Blocks.STONE, 0}, {Blocks.GRASS, 0}, {Blocks.ICE, 0},
                {Blocks.PACKED_ICE, 0}, {Blocks.SOUL_SAND, 0}, {Blocks.SLIME, 0}, {Blocks.SAND, 0}, {Blocks.FLOWING_WATER, 2},
                {Blocks.FLOWING_WATER, 5}, {Blocks.MAGMA, 0}, {Blocks.LILY_PAD, 0}, {Blocks.WEB, 0}, {Blocks.LAVA, 0}};
        int patches = x.range(3, 12);
        for (int i = 0; i < patches; i++) {
            int cx = x.range(-12, 12), cz = x.range(-12, 12), r = x.range(1, 6);
            int[] g = grounds[x.nextInt(grounds.length)];
            int top = x.chance(0.7) ? 62 : x.range(60, 63);
            if (g[0] == Blocks.LILY_PAD) {
                sc.fill(cx - r, 59, cz - r, cx + r, 62, cz + r, Blocks.WATER, 0).fill(cx - r, 63, cz - r, cx + r, 63, cz + r, Blocks.LILY_PAD, 0);
            } else if (g[0] == Blocks.WEB) {
                sc.fill(cx - r, 63, cz - r, cx + r, 63, cz + r, Blocks.WEB, 0);
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
        int[] keySets = {0, FORWARD, FORWARD, FORWARD | LEFT, FORWARD | RIGHT, LEFT, RIGHT, BACK, BACK | LEFT, LEFT | RIGHT, FORWARD | BACK,
                FORWARD | JUMP, FORWARD | SPRINT, SNEAK};
        int keys = 0;
        for (int t = 0; t < ticks; t++) {
            if (x.chance(0.12)) {
                keys = keySets[x.nextInt(keySets.length)];
            }
            if (x.chance(0.2)) {
                look += (float) ((x.nextDouble() - 0.5) * 90);
                pitch = (float) Math.max(-90, Math.min(90, pitch + (x.nextDouble() - 0.5) * 40));
            }
            sc.run(1, keys, look, pitch);
        }
        boat(sc, bx, by, bz, yaw);
        if (x.chance(0.4)) {
            int n = x.range(1, 3);
            // What blocks a boat: boxes of boats (getBox) and of pushable entities (getHardCollisionBox:
            // boats, minecarts, living entities but bats and armor stands); items block nothing.
            String[] others = {"boat", "boat", "boat", "zombie", "minecart", "armor_stand", "bat", "item", "pig"};
            for (int i = 0; i < n; i++) {
                setup(sc).events.add(new Ent.Spawn(BOAT + 1 + i, others[x.nextInt(others.length)], bx + (x.nextDouble() - 0.5) * 8, 63.2,
                        bz + (x.nextDouble() - 0.5) * 8, (float) (x.nextDouble() * 360)));
            }
        }
        for (int t = 5; t < ticks; t++) {
            if (x.chance(0.02)) {
                addEvent(sc, t, new VehicleMove(bx + (x.nextDouble() - 0.5) * 6, 63 + x.nextDouble(), bz + (x.nextDouble() - 0.5) * 6,
                        (float) (x.nextDouble() * 360 - 180), 0.0F));
            }
        }
        if (x.chance(0.15)) {
            addEvent(sc, x.range(ticks / 2, ticks - 1), new Ride(BOAT, new int[0]));
        }
        sc.description = "randomized boat case: pools, land, ice, walls, mobs; paddling keys, looks, corrections";
        return sc;
    }

    // ---- minecarts: the local player sits in a minecart the server moves --------------------------

    /** A case with a minecart at a position; the player boards it in the first tick (set-passengers). */
    private static Scenario cart(Scenario sc, double x, double y, double z, float yaw) {
        Setup st = setup(sc);
        st.events.add(new Ent.Spawn(BOAT, "minecart", x, y, z, yaw));
        addEvent(sc, 0, new Ride(BOAT, new int[] {Ent.PLAYER_ENTITY_ID}));
        return sc;
    }

    /** The server rolling the cart: a relative move of (dx, dy, dz) blocks every tick in [from, to). */
    private static void roll(Scenario sc, int from, int to, double dx, double dy, double dz) {
        for (int t = from; t < to && t < sc.ticks.size(); t++) {
            addEvent(sc, t, new Ent.Rel(BOAT, (int) Math.round(dx * 4096), (int) Math.round(dy * 4096), (int) Math.round(dz * 4096), true));
        }
    }

    static List<Scenario> cartsGolden() {
        List<Scenario> l = new ArrayList<>(carts());
        for (int i = 0; i < GOLDEN_FUZZ; i++) {
            l.add(cartFuzz(1, i, 60));
        }
        return l;
    }

    static List<Scenario> carts() {
        Harness.init();
        List<Scenario> l = new ArrayList<>();
        l.add(cart(flat("minecart_still").desc("sit in a minecart on stone: idle, look around, keys").spawn(0.5, 64, 0.5, 0, 0)
                .run(5, 0).run(10, FORWARD, 120, 20).run(10, LEFT | JUMP, -60, -40).run(10, SPRINT | FORWARD).run(10, SNEAK), 0.5, 64, 0.5, 0.0F));
        Scenario rolling = cart(flat("minecart_rolling").desc("the server rolls the cart (relative moves every tick, then a stop)")
                .spawn(0.5, 64, 0.5, 0, 0).run(5, 0).run(60, FORWARD), 0.5, 64, 0.5, 0.0F);
        roll(rolling, 3, 40, 0.0, 0.0, 0.3);
        l.add(rolling);
        Scenario sparse = cart(flat("minecart_sparse_moves").desc("relative moves every third tick, a far and a near teleport, a zero move")
                .spawn(0.5, 64, 0.5, 0, 0).run(70, 0), 0.5, 64, 0.5, 0.0F);
        for (int t = 3; t < 45; t += 3) {
            addEvent(sparse, t, new Ent.Rel(BOAT, 1229, 0, 600, true));
        }
        addEvent(sparse, 50, new Ent.Move(BOAT, 8.5, 64, 3.25, 90.0F, true));
        addEvent(sparse, 58, new Ent.Move(BOAT, 8.51, 64, 3.25, 180.0F, true));
        addEvent(sparse, 62, new Ent.Rel(BOAT, 0, 0, 0, true));
        l.add(sparse);
        Scenario wall = cart(flat("minecart_into_wall").desc("rolled under a low ceiling and against a wall: the passenger's own tick collides")
                .fill(-3, 66, 4, 3, 66, 12, Blocks.STONE, 0).fill(-3, 64, 14, 3, 66, 14, Blocks.STONE, 0)
                .spawn(0.5, 64, 0.5, 0, 0).run(70, JUMP), 0.5, 64, 0.5, 0.0F);
        roll(wall, 3, 60, 0.0, 0.0, 0.25);
        l.add(wall);
        Scenario water = cart(flat("minecart_water").desc("rolled into and through water: the passenger touches water (not a boat)")
                .fill(-3, 63, 4, 3, 63, 9, Blocks.WATER, 0).fill(-3, 64, 6, 3, 65, 7, Blocks.WATER, 0)
                .spawn(0.5, 64, 0.5, 0, 0).run(60, JUMP | FORWARD), 0.5, 64, 0.5, 0.0F);
        roll(water, 3, 50, 0.0, 0.0, 0.2);
        l.add(water);
        Scenario drop = cart(flat("minecart_falls").desc("the server drops the cart down a step (moves with dy)")
                .fill(-3, 63, 5, 3, 63, 20, Blocks.AIR, 0).fill(-3, 60, 5, 3, 60, 20, Blocks.STONE, 0)
                .spawn(0.5, 64, 0.5, 0, 0).run(60, 0), 0.5, 64, 0.5, 0.0F);
        roll(drop, 3, 18, 0.0, 0.0, 0.4);
        roll(drop, 18, 24, 0.0, -0.5, 0.3);
        l.add(drop);
        Scenario off = cart(flat("minecart_server_dismount").desc("sneak (sent in steer_vehicle only), then the server takes the player off")
                .spawn(0.5, 64, 0.5, 0, 0).run(8, 0).run(5, SNEAK).run(20, FORWARD), 0.5, 64, 0.5, 0.0F);
        roll(off, 3, 30, 0.2, 0.0, 0.0);
        addEvent(off, 14, new Ride(BOAT, new int[0]));
        l.add(off);
        return l;
    }

    /**
     * Randomized minecart cases: stone with patches of water, slabs, glass, soul sand, cobwebs, lava and
     * ladders, ceilings, around a cart the player boards in the first tick; the server rolls it with relative
     * moves (every 1-4 ticks), teleports it; random keys and looks; sometimes a server dismount. Cases get a
     * {@code _mc} suffix.
     */
    static Scenario cartFuzz(long seed, int index, int ticks) {
        Harness.init();
        Rng x = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x6d696e6563L);
        Scenario sc = new Scenario("fuzz_s" + seed + "_" + index + "_mc");
        sc.fill(-32, 63, -32, 32, 63, 32, Blocks.STONE, 0);
        int[][] blocks = {{Blocks.WATER, 0}, {Blocks.STONE, 0}, {Blocks.STONE_SLAB, 0}, {Blocks.GLASS, 0}, {Blocks.SOUL_SAND, 0},
                {Blocks.WEB, 0}, {Blocks.LAVA, 0}, {Blocks.LADDER, 2}, {Blocks.AIR, 0}};
        int patches = x.range(2, 10);
        for (int i = 0; i < patches; i++) {
            int cx = x.range(-12, 12), cz = x.range(-12, 12), r = x.range(1, 5);
            int[] b = blocks[x.nextInt(blocks.length)];
            int y0 = x.chance(0.5) ? 63 : 64, y1 = y0 + x.range(0, 2);
            sc.fill(cx - r, y0, cz - r, cx + r, y1, cz + r, b[0], b[1]);
        }
        if (x.chance(0.3)) {
            int cx = x.range(-8, 8), cz = x.range(-8, 8);
            sc.fill(cx - 4, 65 + x.range(0, 1), cz - 4, cx + 4, 66, cz + 4, Blocks.STONE, 0);
        }
        double cx = x.range(-3, 3) + 0.5, cz = x.range(-3, 3) + 0.5;
        float yaw = (float) (x.nextDouble() * 360 - 180);
        sc.spawn(cx, 64.5, cz, yaw, 0);
        float look = yaw, pitch = 0;
        int[] keySets = {0, 0, FORWARD, FORWARD | LEFT, JUMP, JUMP | FORWARD, SPRINT | FORWARD, BACK, LEFT | RIGHT, SNEAK};
        int keys = 0;
        for (int t = 0; t < ticks; t++) {
            if (x.chance(0.1)) {
                keys = keySets[x.nextInt(keySets.length)];
            }
            if (x.chance(0.25)) {
                look += (float) ((x.nextDouble() - 0.5) * 120);
                pitch = (float) Math.max(-90, Math.min(90, pitch + (x.nextDouble() - 0.5) * 50));
            }
            sc.run(1, keys, look, pitch);
        }
        cart(sc, cx, 64, cz, yaw);
        double vx = (x.nextDouble() - 0.5) * 0.8, vy = 0, vz = (x.nextDouble() - 0.5) * 0.8;
        int every = x.chance(0.5) ? 1 : x.range(2, 4);
        for (int t = 2; t < ticks; t++) {
            if (x.chance(0.05)) {
                vx = (x.nextDouble() - 0.5) * 0.8;
                vz = (x.nextDouble() - 0.5) * 0.8;
                vy = x.chance(0.2) ? (x.nextDouble() - 0.5) * 0.6 : 0;
            }
            if (t % every == 0) {
                addEvent(sc, t, new Ent.Rel(BOAT, (int) Math.round(vx * every * 4096), (int) Math.round(vy * every * 4096),
                        (int) Math.round(vz * every * 4096), x.chance(0.8)));
            }
            if (x.chance(0.02)) {
                double near = x.chance(0.3) ? 0.01 : 8;
                addEvent(sc, t, new Ent.Move(BOAT, cx + (x.nextDouble() - 0.5) * near, 64 + x.nextDouble() * Math.min(near, 1), cz + (x.nextDouble() - 0.5) * near,
                        (float) (x.nextDouble() * 360 - 180), true));
            }
        }
        if (x.chance(0.1)) {
            addEvent(sc, x.range(ticks / 2, ticks - 1), new Ride(BOAT, new int[0]));
        }
        sc.description = "randomized minecart case: stone, water, slabs, ceilings; the server's moves and teleports; keys, looks";
        return sc;
    }
}
