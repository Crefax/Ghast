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

import static ghast.oracle.Scenario.BACK;
import static ghast.oracle.Scenario.FORWARD;
import static ghast.oracle.Scenario.JUMP;
import static ghast.oracle.Scenario.LEFT;
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
        /** An elytra case: tick records carry the gliding state (and list the entities: the rockets). */
        boolean glide;
        /** Item in the chest slot ({@code elytra}), null for none. */
        String chest;
        /** Damage value of the chest item (an elytra at max damage - 1 cannot glide). */
        int chestDamage;
        /** The server has the player gliding at the start (shared flag 7 in the setup entity data). */
        boolean fallFlying;

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

    /**
     * The server's entity data for the player's shared flags byte (index 0): the client's current byte
     * with the {@code clear} bits cleared and the {@code set} bits set (bit values 8 sprinting, 16
     * swimming, 128 fall flying).
     */
    record SharedFlags(int set, int clear) implements Scenario.VersionEvent {}

    /** Equipment packet for the player: {@code item} (null = empty) in {@code slot} with a damage value. */
    record Equip(String slot, String item, int damage) implements Scenario.VersionEvent {}

    /**
     * A firework rocket the server spawned for the gliding player (its entity {@code id}): the spawn
     * packet at the player's position plus the entity data naming the player as the shooter. The server
     * removes it with a destroy packet ({@link Ents.Remove}) when its life runs out.
     */
    record Firework(int id) implements Scenario.VersionEvent {}

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
        if (s.chest != null) {
            equip(run, "chest", s.chest, s.chestDamage);
        }
        if (s.fallFlying) {
            sharedFlags(run, 0x80, 0);
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
        if (e instanceof SharedFlags f) {
            sharedFlags(run, f.set(), f.clear());
            return true;
        }
        if (e instanceof Equip q) {
            equip(run, q.slot(), q.item(), q.damage());
            return true;
        }
        if (e instanceof Firework f) {
            firework(run, f.id());
            return true;
        }
        return false;
    }

    @SuppressWarnings("unchecked")
    private static void sharedFlags(Harness.Run run, int set, int clear) {
        net.minecraft.entity.data.TrackedData<Byte> flags =
                (net.minecraft.entity.data.TrackedData<Byte>) Reflect.get(net.minecraft.entity.Entity.class, null, "FLAGS");
        int cur = run.player.getDataTracker().get(flags);
        int next = ((cur & ~clear) | set) & 0xFF;
        net.minecraft.network.PacketByteBuf b = Harness.buf();
        b.writeVarInt(Harness.PLAYER_ENTITY_ID);
        b.writeByte(0);
        b.writeVarInt(0);
        b.writeByte(next);
        b.writeByte(255);
        Harness.net.onEntityTrackerUpdate(Harness.read(new net.minecraft.network.packet.s2c.play.EntityTrackerUpdateS2CPacket(), b));
    }

    private static void equip(Harness.Run run, String slot, String item, int damage) {
        net.minecraft.item.ItemStack stack = item == null ? net.minecraft.item.ItemStack.EMPTY
                : new net.minecraft.item.ItemStack(Registry.ITEM.getOrEmpty(new net.minecraft.util.Identifier(item))
                        .orElseThrow(() -> new IllegalArgumentException("item " + item)));
        if (damage != 0) {
            stack.setDamage(damage);
        }
        net.minecraft.entity.EquipmentSlot s = net.minecraft.entity.EquipmentSlot.byName(slot);
        net.minecraft.network.packet.s2c.play.EntityEquipmentUpdateS2CPacket p =
                new net.minecraft.network.packet.s2c.play.EntityEquipmentUpdateS2CPacket(Harness.PLAYER_ENTITY_ID,
                        List.of(com.mojang.datafixers.util.Pair.of(s, stack)));
        net.minecraft.network.PacketByteBuf b = Harness.buf();
        writePacket(p, b);
        Harness.net.onEquipmentUpdate(Harness.read(new net.minecraft.network.packet.s2c.play.EntityEquipmentUpdateS2CPacket(), b));
    }

    /**
     * What the server sends when a gliding player uses a rocket ({@code FireworkItem.use}): the entity
     * of the server's own constructor ({@code new FireworkRocketEntity(world, stack, player)}: at the
     * player, shooter = the player) as a spawn packet (no motion: a rocket that has a shooter takes the
     * shooter's) plus its changed entity data (the shooter id).
     */
    private static void firework(Harness.Run run, int id) {
        net.minecraft.entity.projectile.FireworkRocketEntity server = new net.minecraft.entity.projectile.FireworkRocketEntity(
                run.world, new net.minecraft.item.ItemStack(net.minecraft.item.Items.FIREWORK_ROCKET), run.player);
        net.minecraft.network.packet.s2c.play.EntitySpawnS2CPacket spawn = new net.minecraft.network.packet.s2c.play.EntitySpawnS2CPacket(
                id, new java.util.UUID(0x66697265L, id), server.getX(), server.getY(), server.getZ(), server.pitch, server.yaw,
                net.minecraft.entity.EntityType.FIREWORK_ROCKET, 0, net.minecraft.util.math.Vec3d.ZERO);
        net.minecraft.network.PacketByteBuf b = Harness.buf();
        writePacket(spawn, b);
        Harness.net.onEntitySpawn(Harness.read(new net.minecraft.network.packet.s2c.play.EntitySpawnS2CPacket(), b));
        net.minecraft.network.packet.s2c.play.EntityTrackerUpdateS2CPacket data =
                new net.minecraft.network.packet.s2c.play.EntityTrackerUpdateS2CPacket(id, server.getDataTracker(), false);
        net.minecraft.network.PacketByteBuf d = Harness.buf();
        writePacket(data, d);
        Harness.net.onEntityTrackerUpdate(Harness.read(new net.minecraft.network.packet.s2c.play.EntityTrackerUpdateS2CPacket(), d));
    }

    @SuppressWarnings("unchecked")
    private static net.minecraft.entity.data.TrackedData<Byte> sharedFlagsData() {
        return (net.minecraft.entity.data.TrackedData<Byte>) Reflect.get(net.minecraft.entity.Entity.class, null, "FLAGS");
    }

    @SuppressWarnings("unchecked")
    private static net.minecraft.entity.data.TrackedData<Byte> livingFlags() {
        return (net.minecraft.entity.data.TrackedData<Byte>) Reflect.get(net.minecraft.entity.LivingEntity.class, null, "LIVING_FLAGS");
    }

    /** The {@code ext} object of a tick record (cases with a hand item): the riptide state. */
    static void tickRecord(Json j, Harness.Run run) {
        Setup s = of(run.sc);
        if (s != null && s.glide) {
            j.key("ext").beginObject();
            j.field("fall_flying", run.player.isFallFlying());
            j.field("shared_flags", run.player.getDataTracker().get(sharedFlagsData()) & 0xFF);
            j.endObject();
            return;
        }
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
        if (s.glide) {
            j.field("glide", true);
        }
        if (s.chest != null) {
            j.field("chest", s.chest);
            if (s.chestDamage != 0) {
                j.field("chest_damage", s.chestDamage);
            }
        }
        if (s.fallFlying) {
            j.field("fall_flying", true);
        }
    }

    /** Writes an event of this class (the {@code type} and its fields); false for the others. */
    static boolean event(Json j, Scenario.VersionEvent e) {
        if (e instanceof LivingFlags f) {
            j.field("type", "living_flags").field("set", f.set()).field("clear", f.clear());
            return true;
        }
        if (e instanceof SharedFlags f) {
            j.field("type", "shared_flags").field("set", f.set()).field("clear", f.clear());
            return true;
        }
        if (e instanceof Equip q) {
            j.field("type", "equip").field("slot", q.slot());
            if (q.item() != null) {
                j.field("item", q.item());
            }
            j.field("damage", q.damage());
            return true;
        }
        if (e instanceof Firework f) {
            j.field("type", "firework").field("id", f.id());
            return true;
        }
        return false;
    }

    /** An event this class knows, or null. */
    static Scenario.VersionEvent readEvent(JsonObject ev) {
        return switch (ev.get("type").getAsString()) {
            case "living_flags" -> new LivingFlags(ev.get("set").getAsInt(), ev.get("clear").getAsInt());
            case "shared_flags" -> new SharedFlags(ev.get("set").getAsInt(), ev.get("clear").getAsInt());
            case "equip" -> new Equip(ev.get("slot").getAsString(), ev.has("item") ? ev.get("item").getAsString() : null,
                    ev.get("damage").getAsInt());
            case "firework" -> new Firework(ev.get("id").getAsInt());
            default -> null;
        };
    }

    /** True when the {@code ext} object holds fields of this class (the case needs a {@link Setup}). */
    static boolean readSetup(JsonObject x, Scenario sc) {
        if (!x.has("dimension") && !x.has("with_entities") && !x.has("hand") && !x.has("raining") && !x.has("glide")) {
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
        s.glide = x.has("glide") && x.get("glide").getAsBoolean();
        s.chest = x.has("chest") ? x.get("chest").getAsString() : null;
        s.chestDamage = x.has("chest_damage") ? x.get("chest_damage").getAsInt() : 0;
        s.fallFlying = x.has("fall_flying") && x.get("fall_flying").getAsBoolean();
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

    // ---- elytra: gliding, rockets, the server's flag 7 ---------------------------------------------

    /** Shared flag 7 (fall flying) as a bit value. */
    static final int FF = 0x80;

    private static Scenario elytra(Scenario sc) {
        Setup st = setup(sc);
        st.glide = true;
        st.withEntities = true;
        st.chest = "elytra";
        return sc;
    }

    static List<Scenario> elytraGolden() {
        List<Scenario> l = new ArrayList<>(elytra());
        for (int i = 0; i < GOLDEN_FUZZ; i++) {
            l.add(elytraFuzz(1, i, 60));
        }
        return l;
    }

    static List<Scenario> elytra() {
        Harness.init();
        List<Scenario> l = new ArrayList<>();
        l.add(elytra(flat("elytra_start_glide")).desc("fall, press jump in the air: START_FALL_FLYING, glide level, dive, climb")
                .spawn(0.5, 110, 0.5, 0, 0).run(4, 0).run(1, JUMP).run(30, 0).turn(10, 0, 0, 4).run(15, 0, 0, 40)
                .turn(20, 0, 0, -5).run(30, 0, 3, -50));
        l.add(elytra(flat("elytra_jump_from_ground")).desc("jump on the ground (no glide), keep holding (no retrigger), press again in the air")
                .spawn(0.5, 64, 0.5, 90, 10).run(2, 0).run(8, JUMP | FORWARD).run(1, FORWARD).run(1, JUMP | FORWARD).run(40, FORWARD));
        Scenario rising = elytra(flat("elytra_press_while_rising")).desc("press jump while still rising after a jump (1.16 glides rising too)")
                .spawn(0.5, 64, 0.5, 0, 0);
        l.add(rising.run(2, 0).run(1, JUMP).run(1, 0).run(1, JUMP).run(30, 0));
        Scenario broken = elytra(flat("elytra_broken")).desc("elytra at max damage - 1: jump in the air does not glide")
                .spawn(0.5, 90, 0.5, 0, 0);
        setup(broken).chestDamage = 431;
        l.add(broken.run(3, 0).run(1, JUMP).run(20, 0).run(1, JUMP).run(10, 0));
        Scenario worn = elytra(flat("elytra_worn")).desc("elytra at max damage - 2: still glides").spawn(0.5, 90, 0.5, 0, 0);
        setup(worn).chestDamage = 430;
        l.add(worn.run(3, 0).run(1, JUMP).run(20, 0));
        Scenario none = elytra(flat("elytra_no_chest")).desc("no elytra: jump in the air does nothing").spawn(0.5, 80, 0.5, 0, 0);
        setup(none).chest = null;
        l.add(none.run(3, 0).run(1, JUMP).run(20, 0));
        Scenario chestplate = elytra(flat("elytra_chestplate")).desc("a chestplate is not an elytra").spawn(0.5, 80, 0.5, 0, 0);
        setup(chestplate).chest = "diamond_chestplate";
        l.add(chestplate.run(3, 0).run(1, JUMP).run(20, 0));
        l.add(elytra(flat("elytra_water").fill(-6, 64, -6, 6, 70, 6, s("water"), 0))
                .desc("in water: no glide").spawn(0.5, 68, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(10, 0).run(1, JUMP).run(5, 0));
        l.add(elytra(flat("elytra_into_water").fill(-6, 60, 10, 6, 66, 30, s("water"), 0))
                .desc("glide down into a pool: water travel while the flag stays set; sprint-swim attempts")
                .spawn(0.5, 80, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(40, 0, 0, 30).run(20, FORWARD | SPRINT, 0, 30));
        l.add(elytra(flat("elytra_into_lava").fill(-6, 60, 10, 6, 66, 30, s("lava"), 0))
                .desc("glide down into lava: lava travel while the flag stays set")
                .spawn(0.5, 80, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(40, 0, 0, 30));
        Scenario land = elytra(flat("elytra_landing")).desc("glide to the ground: the client keeps gliding until the server clears flag 7")
                .spawn(0.5, 72, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(40, 0, 0, 15).run(20, FORWARD);
        addEvent(land, 52, new SharedFlags(0, FF));
        l.add(land);
        Scenario landSneak = elytra(flat("elytra_land_sneak_soul_sand").fill(-8, 63, 4, 8, 63, 40, s("soul_sand"), 0))
                .desc("land on soul sand gliding (no speed factor while gliding), flag cleared: the glide pose slows the input one tick")
                .spawn(0.5, 70, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(30, FORWARD, 0, 20).run(20, FORWARD | SNEAK);
        addEvent(landSneak, 30, new SharedFlags(0, FF));
        l.add(landSneak);
        l.add(elytra(flat("elytra_ladder").fill(-1, 64, 12, 1, 90, 12, s("stone"), 0).fill(-1, 64, 11, 1, 90, 11, s("ladder[facing=north]"), 0))
                .desc("glide into a ladder; jump on the ladder (climbing: no glide start)")
                .spawn(0.5, 85, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(40, FORWARD, 0, 5).run(1, 0).run(1, JUMP).run(10, 0));
        l.add(elytra(flat("elytra_vines").fill(-3, 64, 10, 3, 90, 13, s("vine[south=true]"), 0))
                .desc("glide through vines").spawn(0.5, 85, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(30, 0, 0, 5).run(1, JUMP).run(10, 0));
        l.add(elytra(flat("elytra_wall").fill(-5, 64, 8, 5, 100, 8, s("stone"), 0))
                .desc("glide into a wall (horizontal collision)").spawn(0.5, 90, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(25, 0, 0, 10));
        Scenario creative = elytra(flat("elytra_creative")).desc("creative: a double tap toggles flight (no glide); flying clears flag 7")
                .spawn(0.5, 80, 0.5, 0, 0).mode(Scenario.GameMode.CREATIVE);
        l.add(creative.run(3, 0).run(1, JUMP).run(2, 0).run(1, JUMP).run(10, 0).run(1, JUMP).run(5, 0).run(1, JUMP).run(2, 0)
                .run(1, JUMP).run(30, 0));
        Scenario flyGlide = elytra(flat("elytra_flying_with_flag")).mode(Scenario.GameMode.CREATIVE).flying(true)
                .desc("flying when the server sets flag 7: one glide travel, then the flight clears the flag").spawn(0.5, 80, 0.5, 0, 0);
        flyGlide.run(3, FORWARD).run(10, FORWARD);
        addEvent(flyGlide, 4, new SharedFlags(FF, 0));
        l.add(flyGlide);
        Scenario lev = elytra(flat("elytra_levitation")).desc("levitation: cannot glide").spawn(0.5, 80, 0.5, 0, 0).effect(25, 0);
        l.add(lev.run(3, 0).run(1, JUMP).run(20, 0));
        Scenario slow = elytra(flat("elytra_slow_falling")).desc("slow falling while gliding (gravity 0.01 when falling)")
                .spawn(0.5, 100, 0.5, 0, 0).effect(28, 0);
        l.add(slow.run(3, 0).run(1, JUMP).run(40, 0, 0, 20).run(20, 0, 0, -30));
        Scenario ceil = elytra(flat("elytra_ceiling").fill(-4, 66, 2, 4, 66, 30, s("stone"), 0))
                .desc("glide under a low ceiling (pose 0.6 high), then the flag is cleared under it")
                .spawn(0.5, 65.2, 0.5, 0, 0).velocity(0, 0, 1.2).run(1, JUMP).run(12, 0).run(10, FORWARD);
        addEvent(ceil, 8, new SharedFlags(0, FF));
        l.add(ceil);
        Scenario fw = elytra(flat("elytra_fireworks")).desc("two rockets attached: boost along the look vector, removed by the server")
                .spawn(0.5, 120, 0.5, 30, 0).run(3, 0).run(1, JUMP).run(10, 0).run(40, 0, 30, -20).run(30, 0, 60, 10);
        addEvent(fw, 8, new Firework(1001));
        addEvent(fw, 20, new Firework(1002));
        addEvent(fw, 26, new Ents.Remove(1001));
        addEvent(fw, 40, new Ents.Remove(1002));
        addEvent(fw, 50, new Firework(1003));
        addEvent(fw, 75, new Ents.Remove(1003));
        l.add(fw);
        // Entity ids that come before and after the player (id 1) in the client world's entity map.
        Scenario order = elytra(flat("elytra_rocket_ids")).desc("rockets with many ids: some tick before the player, some after")
                .spawn(0.5, 150, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(60, 0, 0, 5);
        int[] ids = {2, 3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377};
        for (int k = 0; k < ids.length; k++) {
            addEvent(order, 6 + k * 4, new Firework(ids[k]));
            addEvent(order, 9 + k * 4, new Ents.Remove(ids[k]));
        }
        l.add(order);
        Scenario fwGround = elytra(flat("elytra_firework_not_gliding")).desc("a rocket attached to a player who is not gliding: no boost")
                .spawn(0.5, 64, 0.5, 0, -30).run(5, FORWARD);
        addEvent(fwGround, 1, new Firework(1001));
        l.add(fwGround.run(20, FORWARD).run(1, JUMP).run(10, 0));
        Scenario use = elytra(flat("elytra_firework_use")).desc("use a rocket on the ground (nothing), gliding: use_item + swing; the server spawns it later")
                .spawn(0.5, 110, 0.5, 0, 0);
        setup(use).hand = "firework_rocket";
        use.run(3, 0).run(1, USE).run(2, 0).run(1, JUMP).run(5, 0).run(1, USE).run(30, 0);
        addEvent(use, 14, new Firework(1001));
        addEvent(use, 33, new Ents.Remove(1001));
        l.add(use);
        Scenario flags = elytra(flat("elytra_server_flags")).desc("server entity data: sprint / swim bits, flag 7 set and cleared from the server")
                .spawn(0.5, 100, 0.5, 0, 0).run(3, FORWARD);
        flags.run(10, FORWARD).run(30, FORWARD | SPRINT).run(20, 0);
        addEvent(flags, 2, new SharedFlags(FF, 0));
        addEvent(flags, 12, new SharedFlags(0x08, 0));
        addEvent(flags, 20, new SharedFlags(0x10, 0x08));
        addEvent(flags, 30, new SharedFlags(0, 0x90));
        addEvent(flags, 40, new SharedFlags(0x88, 0));
        l.add(flags);
        Scenario started = elytra(flat("elytra_setup_gliding")).desc("join gliding (flag 7 in the setup entity data), sneak and strafe")
                .spawn(0.5, 100, 0.5, 45, 10).velocity(1.0, -0.2, 1.0);
        setup(started).fallFlying = true;
        l.add(started.run(10, 0).run(10, SNEAK).run(10, LEFT | FORWARD).run(10, BACK).run(10, JUMP));
        Scenario unequip = elytra(flat("elytra_unequip")).desc("elytra taken off while gliding: the client keeps gliding; no restart")
                .spawn(0.5, 110, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(15, 0).run(1, JUMP).run(10, 0);
        addEvent(unequip, 10, new Equip("chest", null, 0));
        addEvent(unequip, 15, new SharedFlags(0, FF));
        addEvent(unequip, 22, new Equip("chest", "elytra", 0));
        l.add(unequip.run(1, JUMP).run(15, 0));
        Scenario edge = elytra(new Scenario("elytra_rocket_chunk_edge").outside(Scenario.Outside.UNLOADED)
                .fill(-16, 63, -96, -1, 63, 15, s("stone"), 0))
                .desc("glide along a loaded / unloaded chunk border: a rocket in an unloaded chunk does not tick")
                .spawn(-0.3, 110, 8.5, 180, 0).run(3, 0).run(1, JUMP).run(30, 0, 200, 0);
        addEvent(edge, 6, new Firework(1001));
        addEvent(edge, 16, new Ents.Remove(1001));
        l.add(edge);
        l.add(elytra(flat("elytra_steep")).desc("look straight down and straight up while gliding (lift, conversion terms)")
                .spawn(0.5, 200, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(20, 0, 0, 90).run(20, 0, 0, -90).run(20, 0, 0, -45));
        return l;
    }

    /**
     * Randomized elytra cases: the {@code withExtras} case (terrain, attributes, effects, inbound packets)
     * with an elytra (sometimes broken or missing), often raised or started gliding, frequent jump presses,
     * wide pitch changes, and server packets: flag 7 cleared or set, sprint / swim bits, attached rockets
     * (added and removed, ids before and after the player's in the entity map), the chest item swapped.
     * Cases get an {@code _el} suffix.
     */
    static Scenario elytraFuzz(long seed, int index, int ticks) {
        Scenario base = Fuzz.withExtras(seed, index, ticks, false);
        Scenario sc = new Scenario(base.name + "_el");
        Fuzz.copyInto(base, sc);
        Rng x = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x656c79747261L);
        Setup st = setup(sc);
        st.glide = true;
        st.withEntities = true;
        if (x.chance(0.92)) {
            st.chest = "elytra";
            if (x.chance(0.05)) {
                st.chestDamage = 431;
            } else if (x.chance(0.2)) {
                st.chestDamage = x.range(1, 430);
            }
        }
        if (x.chance(0.6)) {
            sc.y += x.range(2, 60) + x.nextDouble();
            if (sc.y > 250) {
                sc.y = 250;
            }
        }
        if (x.chance(0.3)) {
            st.fallFlying = true;
        }
        if (x.chance(0.3)) {
            sc.velocity((x.nextDouble() - 0.5) * 3, (x.nextDouble() - 0.7) * 2, (x.nextDouble() - 0.5) * 3);
        }
        float pitch = sc.pitch;
        for (int i = 0; i < sc.ticks.size(); i++) {
            Scenario.Tick tk = sc.ticks.get(i);
            int keys = tk.keys();
            if (x.chance(0.12)) {
                keys ^= JUMP;
            }
            float p = tk.pitch();
            if (x.chance(0.7)) {
                if (x.chance(0.1)) {
                    pitch = (float) (x.nextDouble() * 180 - 90);
                } else {
                    pitch = Math.max(-90f, Math.min(90f, pitch + (float) ((x.nextDouble() - 0.5) * 12)));
                }
                p = pitch;
            }
            sc.ticks.set(i, new Scenario.Tick(keys, tk.yaw(), p, tk.events()));
        }
        List<int[]> live = new ArrayList<>();
        java.util.Set<Integer> used = new java.util.HashSet<>();
        for (int i = 0; i < sc.ticks.size(); i++) {
            if (x.chance(0.035)) {
                addEvent(sc, i, new SharedFlags(0, FF));
            }
            if (x.chance(0.015)) {
                addEvent(sc, i, new SharedFlags(FF, 0));
            }
            if (x.chance(0.01)) {
                int bits = x.pick(new int[] {0x08, 0x10, 0x18});
                addEvent(sc, i, x.chance(0.5) ? new SharedFlags(bits, 0) : new SharedFlags(0, bits));
            }
            if (x.chance(0.04) && live.size() < 3) {
                // Small and large ids: the client world's entity map ticks some before the player.
                int id = x.chance(0.5) ? x.range(2, 60) : x.range(1000, 5000);
                if (used.add(id)) {
                    addEvent(sc, i, new Firework(id));
                    live.add(new int[] {id, i + x.range(1, 25)});
                }
            }
            for (int k = live.size() - 1; k >= 0; k--) {
                if (live.get(k)[1] == i) {
                    addEvent(sc, i, new Ents.Remove(live.get(k)[0]));
                    live.remove(k);
                }
            }
            if (x.chance(0.01)) {
                boolean on = x.chance(0.6);
                addEvent(sc, i, new Equip("chest", on ? "elytra" : null, on && x.chance(0.2) ? 431 : 0));
            }
        }
        sc.description += ", elytra";
        return sc;
    }
}
