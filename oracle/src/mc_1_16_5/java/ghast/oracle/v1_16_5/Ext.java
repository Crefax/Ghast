package ghast.oracle.v1_16_5;

import com.google.gson.JsonObject;
import ghast.oracle.Json;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import net.minecraft.util.registry.Registry;
import net.minecraft.util.registry.RegistryKey;
import net.minecraft.world.World;
import net.minecraft.world.biome.Biome;
import net.minecraft.world.biome.BiomeKeys;
import net.minecraft.world.dimension.DimensionType;

import java.util.ArrayList;
import java.util.List;

import static ghast.oracle.Scenario.FORWARD;
import static ghast.oracle.Scenario.JUMP;
import static ghast.oracle.Scenario.SNEAK;
import static ghast.oracle.Scenario.SPRINT;
import static ghast.oracle.Scenario.USE;

/**
 * 1.16.5 movement beyond the plain player (added for ghast 0.4.5): the level's dimension type (the
 * nether: lava pushes with 0.007 instead of 0.0023333) and riptide (a trident in the hand, rain, the
 * server's spin flag). Everything goes through the real client: the level is built with that
 * dimension type the way {@code ClientPlayNetworkHandler.onGameJoin} builds it from the join packet;
 * rain arrives as game state packets, the spin flag as the player's entity data.
 *
 * <p>A case uses this only when {@link Scenario#versionSetup} is a {@link Setup} (a subclass of
 * {@link Ents.Setup}, so a case can also have other entities): the bytes of every older golden file
 * stay the same. Golden files {@code nether.jsonl.gz} and {@code riptide.jsonl.gz}: the scenarios
 * plus {@link #GOLDEN_FUZZ} fuzz cases of seed 1.
 */
final class Ext {
    private Ext() {}

    static final int GOLDEN_FUZZ = 100;

    /** Version setup of a case (header key {@code ext} inside {@code setup}). */
    static final class Setup extends Ents.Setup {
        /** Dimension type of the level ({@code the_nether}); null: the overworld. */
        String dimension;
        /** Item in the selected hotbar slot for the use key instead of the shield ({@code trident}). */
        String hand;
        /** Riptide level of the hand item (a trident). */
        int riptide;
        /** It rains (the server's rain-started and rain-gradient game state packets at setup). */
        boolean raining;
        /** Tick records list the other entities (cases that spawn some). */
        boolean withEntities;

        @Override
        boolean records() {
            return withEntities;
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
     * The server's entity data for the player's living-entity flags byte (index 7): the client's
     * current byte with its item use (bits 1, 2) as the server sees it, then {@code clear} cleared
     * and {@code set} set (value 4: riptide spin attack).
     */
    record LivingFlags(int set, int clear) implements Scenario.VersionEvent {}

    // ---- harness hooks -----------------------------------------------------------------------------

    /** The hand item for the use key ({@code null}: the harness's shield). */
    static net.minecraft.item.ItemStack handStack(Scenario sc) {
        Setup s = of(sc);
        if (s == null || s.hand == null) {
            return null;
        }
        net.minecraft.item.ItemStack stack = new net.minecraft.item.ItemStack(
                Registry.ITEM.getOrEmpty(new net.minecraft.util.Identifier(s.hand))
                        .orElseThrow(() -> new IllegalArgumentException("item " + s.hand)));
        if (s.riptide > 0) {
            stack.addEnchantment(net.minecraft.enchantment.Enchantments.RIPTIDE, s.riptide);
        }
        return stack;
    }

    /** End of setup (after the entities): weather. */
    static void finishSetup(Harness.Run run) {
        Setup s = of(run.sc);
        if (s == null) {
            return;
        }
        if (s.raining) {
            gameState(net.minecraft.network.packet.s2c.play.GameStateChangeS2CPacket.RAIN_STARTED, 0.0F);
            gameState(net.minecraft.network.packet.s2c.play.GameStateChangeS2CPacket.RAIN_GRADIENT_CHANGED, 1.0F);
        }
    }

    private static void gameState(net.minecraft.network.packet.s2c.play.GameStateChangeS2CPacket.Reason reason, float value) {
        net.minecraft.network.PacketByteBuf b = Harness.buf();
        writePacket(new net.minecraft.network.packet.s2c.play.GameStateChangeS2CPacket(reason, value), b);
        Harness.net.onGameStateChange(Harness.read(new net.minecraft.network.packet.s2c.play.GameStateChangeS2CPacket(), b));
    }

    private static void writePacket(net.minecraft.network.Packet<?> p, net.minecraft.network.PacketByteBuf b) {
        try {
            p.write(b);
        } catch (java.io.IOException e) {
            throw new IllegalStateException(e);
        }
    }

    /** Applies an event of this class; false for the others. */
    static boolean apply(Harness.Run run, Scenario.VersionEvent e) {
        if (e instanceof LivingFlags f) {
            int cur = run.player.getDataTracker().get(livingFlags());
            // The server's byte carries its own view of the item use (bits 1 and 2): a server that
            // agrees with the client (a cleared bit 1 would stop the client's use).
            int use = run.player.isUsingItem() ? 1 | (run.player.getActiveHand() == net.minecraft.util.Hand.OFF_HAND ? 2 : 0) : 0;
            int next = ((((cur & ~3) | use) & ~f.clear()) | f.set()) & 0xFF;
            net.minecraft.network.PacketByteBuf b = Harness.buf();
            b.writeVarInt(Harness.PLAYER_ENTITY_ID);
            b.writeByte(7);
            b.writeVarInt(0);
            b.writeByte(next);
            b.writeByte(255);
            Harness.net.onEntityTrackerUpdate(Harness.read(new net.minecraft.network.packet.s2c.play.EntityTrackerUpdateS2CPacket(), b));
            return true;
        }
        return false;
    }

    @SuppressWarnings("unchecked")
    private static net.minecraft.entity.data.TrackedData<Byte> livingFlags() {
        return (net.minecraft.entity.data.TrackedData<Byte>) Reflect.get(net.minecraft.entity.LivingEntity.class, null, "LIVING_FLAGS");
    }

    /** The {@code ext} object of a tick record (cases with a hand item): the riptide state. */
    static void tickRecord(Json j, Harness.Run run) {
        Setup s = of(run.sc);
        if (s == null || s.hand == null) {
            return;
        }
        j.key("ext").beginObject();
        j.field("riptide_ticks", Reflect.getInt(net.minecraft.entity.LivingEntity.class, run.player, "riptideTicks"));
        j.field("spin_attack", run.player.isUsingRiptide());
        j.field("living_flags", run.player.getDataTracker().get(livingFlags()) & 0xFF);
        j.field("item_use_time", run.player.getItemUseTime());
        j.endObject();
    }

    static RegistryKey<World> dimensionKey(Scenario sc) {
        Setup s = of(sc);
        return s != null && "the_nether".equals(s.dimension) ? World.NETHER : World.OVERWORLD;
    }

    static DimensionType dimensionType(Scenario sc) {
        Setup s = of(sc);
        if (s == null || s.dimension == null) {
            return Harness.overworld;
        }
        RegistryKey<DimensionType> key = switch (s.dimension) {
            case "the_nether" -> DimensionType.THE_NETHER_REGISTRY_KEY;
            default -> throw new IllegalArgumentException("dimension " + s.dimension);
        };
        return Harness.registries.getDimensionTypes().get(key);
    }

    /** The biome of every column (the nether's own in the nether; it does not change movement). */
    static Biome biome(Scenario sc) {
        Setup s = of(sc);
        if (s != null && "the_nether".equals(s.dimension)) {
            return Harness.registries.get(Registry.BIOME_KEY).get(BiomeKeys.NETHER_WASTES);
        }
        return Harness.plains;
    }

    // ---- recording / reading -----------------------------------------------------------------------

    /** Extra fields of the header's {@code ext} object. */
    static void header(Json j, Ents.Setup st) {
        if (!(st instanceof Setup s)) {
            return;
        }
        if (s.dimension != null) {
            j.field("dimension", s.dimension);
        }
        if (s.hand != null) {
            j.field("hand", s.hand);
        }
        if (s.riptide != 0) {
            j.field("riptide", s.riptide);
        }
        if (s.raining) {
            j.field("raining", true);
        }
        if (s.withEntities) {
            j.field("with_entities", true);
        }
    }

    /** Writes an event of this class (the {@code type} and its fields); false for the others. */
    static boolean event(Json j, Scenario.VersionEvent e) {
        if (e instanceof LivingFlags f) {
            j.field("type", "living_flags").field("set", f.set()).field("clear", f.clear());
            return true;
        }
        return false;
    }

    /** An event this class knows, or null. */
    static Scenario.VersionEvent readEvent(JsonObject ev) {
        return switch (ev.get("type").getAsString()) {
            case "living_flags" -> new LivingFlags(ev.get("set").getAsInt(), ev.get("clear").getAsInt());
            default -> null;
        };
    }

    /** True when the {@code ext} object holds fields of this class (the case needs a {@link Setup}). */
    static boolean readSetup(JsonObject x, Scenario sc) {
        if (!x.has("dimension") && !x.has("with_entities") && !x.has("hand") && !x.has("raining")) {
            return false;
        }
        Setup s = setup(sc);
        if (x.has("dimension")) {
            s.dimension = x.get("dimension").getAsString();
        }
        if (x.has("hand")) {
            s.hand = x.get("hand").getAsString();
        }
        s.riptide = x.has("riptide") ? x.get("riptide").getAsInt() : 0;
        s.raining = x.has("raining") && x.get("raining").getAsBoolean();
        s.withEntities = x.has("with_entities") && x.get("with_entities").getAsBoolean();
        return true;
    }

    // ---- cases -------------------------------------------------------------------------------------

    private static int s(String spec) {
        return States.id(spec);
    }

    private static Scenario nether(Scenario sc) {
        setup(sc).dimension = "the_nether";
        return sc;
    }

    static List<Scenario> netherGolden() {
        List<Scenario> l = new ArrayList<>(nether());
        for (int i = 0; i < GOLDEN_FUZZ; i++) {
            l.add(netherFuzz(1, i, 60));
        }
        return l;
    }

    static List<Scenario> nether() {
        Harness.init();
        List<Scenario> l = new ArrayList<>();
        int lava = s("lava"), flowing = s("lava[level=3]"), netherrack = s("netherrack");
        Scenario base = new Scenario("nether_lava_flow").fill(-8, 30, -8, 8, 30, 8, netherrack, 0);
        l.add(nether(base.fill(-3, 31, -3, 3, 31, 3, lava, 0).fill(4, 31, -3, 6, 31, 3, flowing, 0))
                .desc("walk through still and flowing lava in the nether: the stronger flow push").spawn(0.5, 31, 0.5, 90, 0)
                .run(3, 0).run(30, FORWARD).run(20, FORWARD | JUMP));
        Scenario fall = new Scenario("nether_lava_fall").fill(-8, 30, -8, 8, 30, 8, netherrack, 0);
        l.add(nether(fall.fill(-3, 31, -3, 3, 32, 3, flowing, 0)).desc("fall into flowing lava from 6 blocks, swim up")
                .spawn(0.5, 38, 0.5, 0, 0).run(30, 0).run(20, JUMP));
        // A lava slope (levels 1..7 falling away from a source) the player stands in: the flow
        // vector of several levels at once.
        Scenario slope = new Scenario("nether_lava_slope").fill(-8, 30, -8, 8, 30, 8, netherrack, 0).block(0, 31, 0, lava);
        for (int k = 1; k <= 7; k++) {
            slope.fill(k, 31, -1, k, 31, 1, s("lava[level=" + k + "]"), 0);
        }
        l.add(nether(slope).desc("stand, then sprint against the flow down a lava slope").spawn(3.5, 31, 0.5, 90, 0)
                .run(20, 0).run(20, FORWARD | SPRINT).run(10, FORWARD | JUMP));
        Scenario deep = new Scenario("nether_lava_deep").fill(-8, 30, -8, 8, 30, 8, netherrack, 0)
                .fill(-4, 31, -4, 4, 34, 4, lava, 0).fill(-4, 35, -4, 4, 35, 4, s("lava[level=8]"), 0);
        l.add(nether(deep).desc("deep lava with falling lava on top: sink, swim up, climb out").spawn(0.5, 33, 0.5, 0, 0)
                .run(20, 0).run(30, JUMP).run(20, FORWARD | JUMP));
        Scenario flyLava = new Scenario("nether_lava_flying").fill(-8, 30, -8, 8, 30, 8, netherrack, 0)
                .fill(-3, 31, -3, 3, 31, 3, s("lava[level=2]"), 0);
        l.add(nether(flyLava).mode(Scenario.GameMode.CREATIVE).flying(true).desc("creative flight through flowing lava (no flow push while flying)")
                .spawn(0.5, 31.2, 0.5, 0, 0).run(10, 0).run(20, FORWARD));
        Scenario roof = new Scenario("nether_low_world").fill(-8, 0, -8, 8, 0, 8, s("bedrock"), 0);
        l.add(nether(roof.fill(-8, 4, -8, 8, 4, 8, s("bedrock"), 0)).desc("walk and jump at the bottom of the nether")
                .spawn(0.5, 1, 0.5, 0, 0).run(3, 0).run(20, FORWARD | JUMP));
        Scenario top = new Scenario("nether_top").fill(-8, 254, -8, 8, 254, 8, s("bedrock"), 0);
        l.add(nether(top).desc("jump on the highest layer (y 254-255)").spawn(0.5, 255, 0.5, 0, 0).run(3, 0).run(20, FORWARD | JUMP));
        Scenario water = new Scenario("nether_water").fill(-8, 30, -8, 8, 30, 8, netherrack, 0)
                .fill(-3, 31, -3, 3, 31, 3, s("water[level=3]"), 0);
        l.add(nether(water).desc("flowing water in the nether (a server can place it): water pushes as in the overworld")
                .spawn(0.5, 31, 0.5, 0, 0).run(30, FORWARD));
        return l;
    }

    /**
     * Randomized nether cases: the {@code withExtras} case in the nether dimension type, with lava
     * pools (still and flowing) at and around the spawn. Cases get an {@code _ne} suffix.
     */
    static Scenario netherFuzz(long seed, int index, int ticks) {
        Scenario base = Fuzz.withExtras(seed, index, ticks, false);
        Scenario sc = new Scenario(base.name + "_ne");
        Fuzz.copyInto(base, sc);
        nether(sc);
        Rng x = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x6e6574686572L);
        int bx = (int) Math.floor(sc.x), by = (int) Math.floor(sc.y), bz = (int) Math.floor(sc.z);
        int pools = x.range(1, 4);
        for (int i = 0; i < pools; i++) {
            int r = x.range(1, 4);
            int cx = bx + x.range(-4, 4), cz = bz + x.range(-4, 4), y = by + x.range(-2, 0);
            int level = x.chance(0.4) ? 0 : x.range(1, 8);
            int y1 = Math.min(255, y + x.range(0, 2));
            if (y < 0) {
                continue;
            }
            sc.fill(cx - r, y, cz - r, cx + r, y1, cz + r, s(level == 0 ? "lava" : "lava[level=" + level + "]"), 0);
        }
        sc.description += ", nether";
        return sc;
    }

    // ---- riptide: a trident in the hand, water or rain, the server's spin flag ----------------------

    private static Scenario trident(Scenario sc, int level) {
        Setup st = setup(sc);
        st.hand = "trident";
        st.riptide = level;
        return sc;
    }

    private static Scenario flat(String name) {
        return new Scenario(name).fill(-24, 63, -24, 24, 63, 24, s("stone"), 0);
    }

    private static Scenario pool(String name) {
        return flat(name).fill(-8, 60, -8, 8, 62, 8, s("stone"), 0).fill(-7, 61, -7, 7, 63, 7, s("water"), 0);
    }

    private static void addEvent(Scenario sc, int tick, Scenario.Event e) {
        Scenario.Tick tk = sc.ticks.get(tick);
        List<Scenario.Event> events = new ArrayList<>(tk.events());
        events.add(e);
        sc.ticks.set(tick, new Scenario.Tick(tk.keys(), tk.yaw(), tk.pitch(), List.copyOf(events)));
    }

    private static Scenario withEntities(Scenario sc, Ents.Spawn... spawns) {
        Setup st = setup(sc);
        st.withEntities = true;
        st.entities.addAll(List.of(spawns));
        return sc;
    }

    static List<Scenario> riptideGolden() {
        List<Scenario> l = new ArrayList<>(riptide());
        for (int i = 0; i < GOLDEN_FUZZ; i++) {
            l.add(riptideFuzz(1, i, 60));
        }
        return l;
    }

    static List<Scenario> riptide() {
        Harness.init();
        List<Scenario> l = new ArrayList<>();
        Scenario w = trident(pool("riptide_water"), 3).desc("riptide III in water: hold 12 ticks, release looking up; the server's spin flag a tick later")
                .spawn(0.5, 62, 0.5, 0, -40).run(3, 0).run(12, USE).run(30, 0);
        addEvent(w, 16, new LivingFlags(4, 0));
        addEvent(w, 36, new LivingFlags(0, 4));
        l.add(w);
        l.add(trident(pool("riptide_short_hold"), 3).desc("released after 6 and 9 ticks: nothing; after 10: the launch").spawn(0.5, 62, 0.5, 0, -40)
                .run(3, 0).run(6, USE).run(10, 0).run(9, USE).run(10, 0).run(10, USE).run(15, 0));
        l.add(trident(flat("riptide_dry"), 3).desc("on land without rain: the trident cannot be used (the client retries every 4 ticks)")
                .spawn(0.5, 64, 0.5, 0, -30).run(3, 0).run(15, USE).run(10, 0));
        Scenario walkIn = trident(pool("riptide_walk_into_water"), 2).fill(-7, 63, 8, 7, 63, 12, s("stone"), 0)
                .desc("use held on land (refused, retried), walking into the pool: the retry starts the use there").spawn(0.5, 64, 10.5, 180, -20);
        l.add(walkIn.run(3, 0).run(25, USE | FORWARD).run(6, USE).run(20, 0));
        Scenario rain = trident(flat("riptide_rain"), 2).desc("in the rain on open ground: move up 1.2 first, then the push").spawn(0.5, 64, 0.5, 45, -20);
        setup(rain).raining = true;
        l.add(rain.run(3, 0).run(12, USE).run(40, 0));
        Scenario roof = trident(flat("riptide_rain_roof").fill(-3, 67, -3, 3, 67, 3, s("stone"), 0), 2)
                .desc("rain under a roof: no riptide").spawn(0.5, 64, 0.5, 45, -20);
        setup(roof).raining = true;
        l.add(roof.run(3, 0).run(12, USE).run(10, 0));
        // Glass does not block sky light much, but it blocks motion: the MOTION_BLOCKING height map
        // (not the light) keeps the rain off. The harness sends no light data (the client's sky
        // light reads 15 everywhere in the overworld), so blocks that only dim the sky light
        // (cobwebs) are not cased.
        Scenario glass = trident(flat("riptide_rain_under_glass").fill(-3, 67, -3, 0, 67, 3, s("glass"), 0), 2)
                .desc("rain: under glass (motion-blocking) no riptide; step out from under it and launch")
                .spawn(0.5, 64, 0.5, -90, -20);
        setup(glass).raining = true;
        l.add(glass.run(3, 0).run(12, USE).run(5, 0).run(6, FORWARD).run(12, USE).run(30, 0));
        Scenario wall = trident(pool("riptide_wall").fill(-7, 61, 5, 7, 75, 5, s("stone"), 0), 3)
                .desc("launch into a wall: a horizontal collision ends the spin counter").spawn(0.5, 62, 0.5, 0, -10);
        wall.run(3, 0).run(11, USE).run(25, 0);
        addEvent(wall, 15, new LivingFlags(4, 0));
        addEvent(wall, 30, new LivingFlags(0, 4));
        l.add(wall);
        for (int lvl = 1; lvl <= 3; lvl++) {
            Scenario c = trident(pool("riptide_level" + lvl), lvl).desc("riptide " + lvl + " straight up, then down into the pool")
                    .spawn(0.5, 62, 0.5, 0, -90).run(2, 0).run(10, USE).run(40, 0, 0, 30);
            addEvent(c, 13, new LivingFlags(4, 0));
            addEvent(c, 33, new LivingFlags(0, 4));
            l.add(c);
        }
        Scenario low = trident(pool("riptide_spin_pose").fill(-7, 65, -7, 7, 65, 7, s("stone"), 0), 3)
                .desc("spin pose (0.6 high) under a ceiling until the flag clears").spawn(0.5, 62, 0.5, 90, 0);
        low.run(2, 0).run(10, USE).run(30, FORWARD);
        addEvent(low, 13, new LivingFlags(4, 0));
        addEvent(low, 25, new LivingFlags(0, 4));
        l.add(low);
        Scenario spinLand = trident(flat("riptide_spin_flag_on_land"), 1).desc("the server's spin flag without a launch: the pose, sneaking and walking under it")
                .fill(-7, 65, 3, 7, 65, 9, s("stone"), 0).spawn(0.5, 64, 0.5, 0, 0);
        spinLand.run(3, 0).run(10, FORWARD).run(10, FORWARD | SNEAK).run(10, FORWARD | SPRINT).run(10, 0);
        addEvent(spinLand, 2, new LivingFlags(4, 0));
        addEvent(spinLand, 35, new LivingFlags(0, 4));
        l.add(spinLand);
        Scenario mob = withEntities(trident(pool("riptide_into_mob"), 3), new Ents.Spawn(100, "zombie", 0.5, 62, 4.5, 0.0F, 0))
                .desc("launch into a zombie: the hit ends the spin and reverses the motion (x -0.2)").spawn(0.5, 62, 0.5, 0, -5);
        mob.run(3, 0).run(11, USE).run(25, 0);
        addEvent(mob, 15, new LivingFlags(4, 0));
        addEvent(mob, 30, new LivingFlags(0, 4));
        l.add(mob);
        Scenario stand = withEntities(trident(pool("riptide_into_armor_stand"), 2), new Ents.Spawn(100, "armor_stand", 0.5, 62, 3.5, 0.0F, 0))
                .desc("an armor stand is a living entity: hit as well").spawn(0.5, 62, 0.5, 0, -5);
        stand.run(3, 0).run(11, USE).run(25, 0);
        l.add(stand);
        Scenario item = withEntities(trident(pool("riptide_item_at_wall").fill(-7, 61, 5, 7, 75, 5, s("stone"), 0), 3),
                new Ents.Spawn(100, "item", 0.5, 62.2, 4.62, 0.0F, 0))
                .desc("an item at the wall (not living: no hit, but while it touches the box the wall does not end the spin)")
                .spawn(0.5, 62, 0.5, 0, -5);
        item.run(3, 0).run(11, USE).run(25, 0);
        l.add(item);
        Scenario boat = withEntities(trident(pool("riptide_into_boat"), 3), new Ents.Spawn(101, "boat", 0.5, 63.0, 3.5, 0.0F, 0))
                .desc("launch into a boat: solid, a horizontal collision that touches nothing: the spin ends")
                .spawn(0.5, 62, 0.5, 0, -5);
        boat.run(3, 0).run(11, USE).run(25, 0);
        l.add(boat);
        Scenario nether = nether(trident(new Scenario("riptide_rain_nether").fill(-8, 30, -8, 8, 30, 8, s("netherrack"), 0), 3))
                .desc("rain in the nether: no sky light, no rain, no riptide").spawn(0.5, 31, 0.5, 0, -30);
        setup(nether).raining = true;
        l.add(nether.run(3, 0).run(12, USE).run(10, 0));
        return l;
    }

    private static final String[] SPIN_POOL = {"zombie", "cow", "pig", "armor_stand", "item", "boat", "sheep", "chicken"};

    /**
     * Randomized riptide cases: the {@code withExtras} case, often with a pool at the spawn and rain, a
     * trident with riptide I-III in the hand, use-key spans of 3-25 ticks, wide pitch changes, the
     * server's spin flag set and cleared, sometimes other entities around. Cases get an {@code _rt}
     * suffix.
     */
    static Scenario riptideFuzz(long seed, int index, int ticks) {
        Scenario base = Fuzz.withExtras(seed, index, ticks, false);
        Scenario sc = new Scenario(base.name + "_rt");
        Fuzz.copyInto(base, sc);
        Rng x = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x7269707469L);
        trident(sc, x.range(1, 3));
        Setup st = setup(sc);
        int bx = (int) Math.floor(sc.x), by = (int) Math.floor(sc.y), bz = (int) Math.floor(sc.z);
        if (x.chance(0.7)) {
            int r = x.range(2, 6), d = x.range(1, 4);
            int y0 = Math.max(0, by - d), y1 = Math.min(255, by + x.range(0, 2));
            sc.fill(bx - r, y0, bz - r, bx + r, y1, bz + r, s("water"), 0);
        }
        if (x.chance(0.3)) {
            st.raining = true;
        }
        if (x.chance(0.25)) {
            st.withEntities = true;
            int n = x.range(1, 3);
            for (int i = 0; i < n; i++) {
                double ex = sc.x + (x.nextDouble() - 0.5) * 8, ey = sc.y + (x.nextDouble() - 0.3) * 4,
                        ez = sc.z + (x.nextDouble() - 0.5) * 8;
                st.entities.add(new Ents.Spawn(100 + i * 7 + x.range(0, 6), x.pick(SPIN_POOL), ex, ey, ez,
                        (float) (x.nextDouble() * 360), 0));
            }
        }
        boolean held = false;
        int span = 0;
        float pitch = sc.pitch;
        for (int i = 0; i < sc.ticks.size(); i++) {
            Scenario.Tick tk = sc.ticks.get(i);
            if (span-- <= 0) {
                held = !held;
                span = held ? x.range(3, 25) : x.range(1, 12);
            }
            int keys = (tk.keys() & ~USE) | (held ? USE : 0);
            if (x.chance(0.08)) {
                keys ^= JUMP;
            }
            if (x.chance(0.5)) {
                pitch = x.chance(0.15) ? (float) (x.nextDouble() * 180 - 90)
                        : Math.max(-90f, Math.min(90f, pitch + (float) ((x.nextDouble() - 0.5) * 20)));
            }
            sc.ticks.set(i, new Scenario.Tick(keys, tk.yaw(), pitch, tk.events()));
        }
        for (int i = 0; i < sc.ticks.size(); i++) {
            if (x.chance(0.04)) {
                addEvent(sc, i, new LivingFlags(4, 0));
            }
            if (x.chance(0.04)) {
                addEvent(sc, i, new LivingFlags(0, 4));
            }
        }
        sc.description += ", riptide";
        return sc;
    }
}
