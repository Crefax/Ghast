package ghast.oracle.v1_12_2;

import ghast.oracle.Json;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import io.netty.buffer.Unpooled;
import net.minecraft.client.MinecraftClient;
import net.minecraft.entity.Entity;
import net.minecraft.entity.EquipmentSlot;
import net.minecraft.entity.FireworkRocketEntity;
import net.minecraft.entity.data.TrackedData;
import net.minecraft.item.Item;
import net.minecraft.item.ItemStack;
import net.minecraft.item.Items;
import net.minecraft.network.Packet;
import net.minecraft.network.listener.ClientPlayPacketListener;
import net.minecraft.network.packet.s2c.play.EntityEquipmentUpdateS2CPacket;
import net.minecraft.network.packet.s2c.play.EntitySpawnS2CPacket;
import net.minecraft.network.packet.s2c.play.EntityTrackerUpdateS2CPacket;
import net.minecraft.util.Identifier;
import net.minecraft.util.PacketByteBuf;

import java.util.ArrayList;
import java.util.List;

import static ghast.oracle.Scenario.BACK;
import static ghast.oracle.Scenario.FORWARD;
import static ghast.oracle.Scenario.JUMP;
import static ghast.oracle.Scenario.LEFT;
import static ghast.oracle.Scenario.SNEAK;
import static ghast.oracle.Scenario.SPRINT;

/**
 * 1.12.2 elytra cases (added for ghast 0.4.6): an elytra in the chest slot (equipment packet), the
 * server's shared flag 7 (entity data packets: in 1.12 only the server starts a glide, the client
 * asks with {@code START_FALL_FLYING}), and firework rockets the server spawns for a gliding player
 * (spawn-object packet type 76 plus the entity data naming the player as the shooter). Every packet
 * is written as wire bytes and decoded by the packet's own {@code read}, then handed to the real
 * client handler.
 *
 * <p>A case uses this only when {@link Scenario#versionSetup} is a {@link Setup} (a subclass of
 * {@link Ent.Setup}: the player gets entity id 1 and tick records list the world's entities in tick
 * order, the rockets among them). Golden file {@code elytra.jsonl.gz}: the cases plus
 * {@link #GOLDEN_FUZZ} fuzz cases of seed 1.
 */
final class Elytra {
    private Elytra() {}

    static final int GOLDEN_FUZZ = 100;
    /** Shared flag 7 (fall flying) as a bit value. */
    static final int FF = 0x80;

    /** Version setup of an elytra case. */
    static final class Setup extends Ent.Setup {
        /** Item in the chest slot ({@code elytra}), null for none. */
        String chest;
        /** Damage value of the chest item (an elytra at max damage - 1 cannot glide). */
        int chestDamage;
        /** The server has the player gliding at the start (shared flag 7 in the setup entity data). */
        boolean fallFlying;
        /** Item in the selected hotbar slot for the use key ({@code fireworks}), null: the harness's shield. */
        String hand;
    }

    static Setup of(Scenario sc) {
        return sc.versionSetup instanceof Setup s ? s : null;
    }

    static Setup setup(Scenario sc) {
        if (!(sc.versionSetup instanceof Setup)) {
            sc.versionSetup = new Setup();
        }
        return (Setup) sc.versionSetup;
    }

    /** The server's entity data for the player's shared flags byte: the client's byte with {@code clear} cleared and {@code set} set. */
    record SharedFlags(int set, int clear) implements Scenario.VersionEvent {}

    /** Equipment packet for the player: {@code item} (null = empty) in {@code slot} with a damage value. */
    record Equip(String slot, String item, int damage) implements Scenario.VersionEvent {}

    /** A firework rocket the server spawned for the player (entity {@code id}); a destroy packet ({@link Ent.Remove}) ends it. */
    record Firework(int id) implements Scenario.VersionEvent {}

    // ---- packets --------------------------------------------------------------------------------

    @SuppressWarnings("unchecked")
    private static void handle(Packet<?> p, PacketByteBuf b, Harness.Run run) {
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

    private static PacketByteBuf write(Packet<?> p) {
        PacketByteBuf b = new PacketByteBuf(Unpooled.buffer());
        try {
            p.write(b);
        } catch (java.io.IOException e) {
            throw new IllegalStateException(e);
        }
        return b;
    }

    @SuppressWarnings("unchecked")
    private static TrackedData<Byte> flagsData() {
        return (TrackedData<Byte>) Reflect.get(Entity.class, null, "FLAGS");
    }

    private static void sharedFlags(Harness.Run run, int set, int clear) {
        int cur = run.player.getDataTracker().get(flagsData());
        int next = ((cur & ~clear) | set) & 0xFF;
        PacketByteBuf b = new PacketByteBuf(Unpooled.buffer());
        b.writeVarInt(Ent.PLAYER_ENTITY_ID);
        b.writeByte(0); // index 0: the shared flags
        b.writeVarInt(0); // byte serializer
        b.writeByte(next);
        b.writeByte(0xff);
        handle(new EntityTrackerUpdateS2CPacket(), b, run);
    }

    static Item item(String name) {
        Item i = Item.REGISTRY.get(new Identifier(name));
        if (i == null) {
            throw new IllegalArgumentException("item " + name);
        }
        return i;
    }

    private static void equip(Harness.Run run, String slot, String item, int damage) {
        ItemStack stack = item == null ? ItemStack.EMPTY : new ItemStack(item(item));
        if (damage != 0) {
            stack.setDamage(damage);
        }
        EquipmentSlot s = EquipmentSlot.method_13031(slot);
        handle(new EntityEquipmentUpdateS2CPacket(), write(new EntityEquipmentUpdateS2CPacket(Ent.PLAYER_ENTITY_ID, s, stack)), run);
    }

    /**
     * What the server sends when a gliding player uses a rocket ({@code FireworkItem.use}): the entity
     * of the server's constructor ({@code new FireworkRocketEntity(world, stack, player)}: at the player,
     * shooter = the player) as a spawn-object packet (type 76, no motion: a rocket with a shooter takes
     * the shooter's) plus its changed entity data (the shooter id).
     */
    private static void firework(Harness.Run run, int id) {
        FireworkRocketEntity server = new FireworkRocketEntity(run.world, new ItemStack(Items.FIREWORKS), run.player);
        PacketByteBuf b = new PacketByteBuf(Unpooled.buffer());
        b.writeVarInt(id);
        b.writeUuid(new java.util.UUID(0x66697265L, id));
        b.writeByte(76);
        b.writeDouble(server.x).writeDouble(server.y).writeDouble(server.z);
        b.writeByte(0).writeByte(0);
        b.writeInt(0);
        b.writeShort(0).writeShort(0).writeShort(0);
        handle(new EntitySpawnS2CPacket(), b, run);
        PacketByteBuf d = write(new EntityTrackerUpdateS2CPacket(id, server.getDataTracker(), false));
        // The packet names the server entity's id; the client knows it by the spawn packet's.
        d.readerIndex(0);
        PacketByteBuf fixed = new PacketByteBuf(Unpooled.buffer());
        fixed.writeVarInt(id);
        int idLen = varIntLength(id);
        fixed.writeBytes(d, idLen, d.readableBytes() - idLen);
        handle(new EntityTrackerUpdateS2CPacket(), fixed, run);
    }

    private static int varIntLength(int v) {
        int n = 1;
        while ((v & ~0x7F) != 0) {
            v >>>= 7;
            n++;
        }
        return n;
    }

    // ---- harness hooks ---------------------------------------------------------------------------

    /** After the setup's entity packets: the chest item and the setup's flag 7. */
    static void finishSetup(Harness.Run run) {
        Setup s = of(run.sc);
        if (s == null) {
            return;
        }
        if (s.chest != null) {
            equip(run, "chest", s.chest, s.chestDamage);
        }
        if (s.fallFlying) {
            sharedFlags(run, FF, 0);
        }
    }

    /** The hand item for the use key ({@code null}: the harness's shield). */
    static ItemStack handStack(Scenario sc) {
        Setup s = of(sc);
        return s == null || s.hand == null ? null : new ItemStack(item(s.hand));
    }

    static boolean apply(Harness.Run run, Scenario.Event e) {
        if (e instanceof SharedFlags f) {
            sharedFlags(run, f.set(), f.clear());
        } else if (e instanceof Equip q) {
            equip(run, q.slot(), q.item(), q.damage());
        } else if (e instanceof Firework f) {
            firework(run, f.id());
        } else {
            return false;
        }
        return true;
    }

    /** The {@code ext} object of a tick record (elytra cases): the glide flag and the shared flags byte. */
    static void tickRecord(Json j, Harness.Run run) {
        if (of(run.sc) == null) {
            return;
        }
        j.key("ext").beginObject();
        j.field("fall_flying", run.player.method_13055());
        j.field("shared_flags", run.player.getDataTracker().get(flagsData()) & 0xFF);
        j.endObject();
    }

    static boolean event(Json j, Scenario.Event e) {
        if (e instanceof SharedFlags f) {
            j.field("type", "shared_flags").field("set", f.set()).field("clear", f.clear());
        } else if (e instanceof Equip q) {
            j.field("type", "equip").field("slot", q.slot());
            if (q.item() != null) {
                j.field("item", q.item());
            }
            j.field("damage", q.damage());
        } else if (e instanceof Firework f) {
            j.field("type", "firework").field("id", f.id());
        } else {
            return false;
        }
        return true;
    }

    static Scenario.Event read(com.google.gson.JsonObject ev) {
        return switch (ev.get("type").getAsString()) {
            case "shared_flags" -> new SharedFlags(ev.get("set").getAsInt(), ev.get("clear").getAsInt());
            case "equip" -> new Equip(ev.get("slot").getAsString(), ev.has("item") ? ev.get("item").getAsString() : null,
                    ev.get("damage").getAsInt());
            case "firework" -> new Firework(ev.get("id").getAsInt());
            default -> null;
        };
    }

    /** The header's {@code setup.ext} object. */
    static void header(Json j, Scenario sc) {
        Setup s = of(sc);
        if (s == null) {
            return;
        }
        j.key("ext").beginObject();
        j.field("glide", true);
        if (s.chest != null) {
            j.field("chest", s.chest);
            if (s.chestDamage != 0) {
                j.field("chest_damage", s.chestDamage);
            }
        }
        if (s.fallFlying) {
            j.field("fall_flying", true);
        }
        if (s.hand != null) {
            j.field("hand", s.hand);
        }
        j.endObject();
    }

    static void readHeader(com.google.gson.JsonObject setup, Scenario sc) {
        if (!setup.has("ext")) {
            return;
        }
        com.google.gson.JsonObject x = setup.getAsJsonObject("ext");
        // Keep the setup events Ent read already.
        Ent.Setup old = Ent.of(sc);
        Setup s = setup(sc);
        if (old != null && old != s) {
            s.events.addAll(old.events);
        }
        s.chest = x.has("chest") ? x.get("chest").getAsString() : null;
        s.chestDamage = x.has("chest_damage") ? x.get("chest_damage").getAsInt() : 0;
        s.fallFlying = x.has("fall_flying") && x.get("fall_flying").getAsBoolean();
        s.hand = x.has("hand") ? x.get("hand").getAsString() : null;
    }

    // ---- cases ------------------------------------------------------------------------------------

    private static int s(String name) {
        return switch (name) {
            case "stone" -> Blocks.STONE;
            case "water" -> Blocks.WATER;
            case "lava" -> Blocks.LAVA;
            case "soul_sand" -> Blocks.SOUL_SAND;
            case "ladder" -> Blocks.LADDER;
            default -> throw new IllegalArgumentException(name);
        };
    }

    private static Scenario elytra(Scenario sc) {
        setup(sc).chest = "elytra";
        return sc;
    }

    private static Scenario flat(String name) {
        return new Scenario(name).fill(-24, 63, -24, 24, 63, 24, s("stone"), 0);
    }

    private static void addEvent(Scenario sc, int tick, Scenario.Event e) {
        Scenario.Tick tk = sc.ticks.get(tick);
        List<Scenario.Event> events = new ArrayList<>(tk.events());
        events.add(e);
        sc.ticks.set(tick, new Scenario.Tick(tk.keys(), tk.yaw(), tk.pitch(), List.copyOf(events)));
    }

    static List<Scenario> golden() {
        List<Scenario> l = new ArrayList<>(all());
        for (int i = 0; i < GOLDEN_FUZZ; i++) {
            l.add(fuzz(1, i, 60));
        }
        return l;
    }

    /**
     * The hand cases. In 1.12 a glide starts only when the server sets flag 7: the cases send it the
     * tick after the client's {@code START_FALL_FLYING} (what a server does when its own checks pass).
     */
    static List<Scenario> all() {
        Harness.init();
        List<Scenario> l = new ArrayList<>();
        Scenario start = elytra(flat("elytra_start_glide")).desc("fall, press jump: START_FALL_FLYING; the server's flag; glide level, dive, climb")
                .spawn(0.5, 110, 0.5, 0, 0).run(4, 0).run(1, JUMP).run(30, 0).turn(10, 0, 0, 4).run(15, 0, 0, 40)
                .turn(20, 0, 0, -5).run(30, 0, 3, -50);
        addEvent(start, 5, new SharedFlags(FF, 0));
        l.add(start);
        Scenario ground = elytra(flat("elytra_jump_from_ground")).desc("jump on the ground (rising: no request), press again falling")
                .spawn(0.5, 64, 0.5, 90, 10).run(2, 0).run(1, JUMP | FORWARD).run(1, FORWARD).run(1, JUMP | FORWARD).run(6, FORWARD)
                .run(1, JUMP | FORWARD).run(30, FORWARD);
        addEvent(ground, 12, new SharedFlags(FF, 0));
        l.add(ground);
        Scenario held = elytra(flat("elytra_held_jump")).desc("jump held while falling: one request only (a press)")
                .spawn(0.5, 90, 0.5, 0, 0).run(3, 0).run(20, JUMP);
        l.add(held);
        Scenario broken = elytra(flat("elytra_broken")).desc("elytra at max damage - 1: no request").spawn(0.5, 90, 0.5, 0, 0);
        setup(broken).chestDamage = 431;
        l.add(broken.run(3, 0).run(1, JUMP).run(20, 0));
        Scenario worn = elytra(flat("elytra_worn")).desc("elytra at max damage - 2: the request").spawn(0.5, 90, 0.5, 0, 0);
        setup(worn).chestDamage = 430;
        l.add(worn.run(3, 0).run(1, JUMP).run(20, 0));
        Scenario none = flat("elytra_no_chest").desc("no elytra: no request").spawn(0.5, 80, 0.5, 0, 0);
        setup(none);
        l.add(none.run(3, 0).run(1, JUMP).run(20, 0));
        Scenario plate = flat("elytra_chestplate").desc("a chestplate: no request").spawn(0.5, 80, 0.5, 0, 0);
        setup(plate).chest = "diamond_chestplate";
        l.add(plate.run(3, 0).run(1, JUMP).run(20, 0));
        Scenario water = elytra(flat("elytra_water").fill(-6, 64, -6, 6, 70, 6, s("water"), 0))
                .desc("sinking in water: the 1.12 client asks anyway; the flag in water: water travel").spawn(0.5, 69, 0.5, 0, 0);
        water.run(6, 0).run(1, JUMP).run(10, 0);
        addEvent(water, 8, new SharedFlags(FF, 0));
        l.add(water);
        Scenario intoWater = elytra(flat("elytra_into_water").fill(-6, 60, 10, 6, 66, 30, s("water"), 0))
                .desc("glide down into a pool: water travel while the flag stays set").spawn(0.5, 80, 0.5, 0, 0)
                .run(3, 0).run(1, JUMP).run(40, 0, 0, 30).run(20, FORWARD | SPRINT, 0, 30);
        addEvent(intoWater, 4, new SharedFlags(FF, 0));
        l.add(intoWater);
        Scenario lava = elytra(flat("elytra_into_lava").fill(-6, 60, 10, 6, 66, 30, s("lava"), 0))
                .desc("glide down into lava").spawn(0.5, 80, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(40, 0, 0, 30);
        addEvent(lava, 4, new SharedFlags(FF, 0));
        l.add(lava);
        Scenario land = elytra(flat("elytra_landing")).desc("glide to the ground: the client glides on until the server clears flag 7")
                .spawn(0.5, 72, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(40, 0, 0, 15).run(20, FORWARD);
        addEvent(land, 4, new SharedFlags(FF, 0));
        addEvent(land, 52, new SharedFlags(0, FF));
        l.add(land);
        Scenario soul = elytra(flat("elytra_soul_sand").fill(-8, 63, 4, 8, 63, 40, s("soul_sand"), 0))
                .desc("glide over and land on soul sand, then sneak").spawn(0.5, 70, 0.5, 0, 0).run(3, 0).run(1, JUMP)
                .run(30, FORWARD, 0, 20).run(20, FORWARD | SNEAK);
        addEvent(soul, 4, new SharedFlags(FF, 0));
        addEvent(soul, 30, new SharedFlags(0, FF));
        l.add(soul);
        Scenario ladder = elytra(flat("elytra_ladder").fill(-1, 64, 12, 1, 90, 12, s("stone"), 0)
                .fill(-1, 64, 11, 1, 90, 11, s("ladder"), 2))
                .desc("glide into a ladder (no climbing while gliding); jump on the ladder").spawn(0.5, 85, 0.5, 0, 0)
                .run(3, 0).run(1, JUMP).run(40, FORWARD, 0, 5).run(1, 0).run(1, JUMP).run(10, 0);
        addEvent(ladder, 4, new SharedFlags(FF, 0));
        addEvent(ladder, 40, new SharedFlags(0, FF));
        l.add(ladder);
        Scenario wall = elytra(flat("elytra_wall").fill(-5, 64, 8, 5, 100, 8, s("stone"), 0))
                .desc("glide into a wall (horizontal collision)").spawn(0.5, 90, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(25, 0, 0, 10);
        addEvent(wall, 4, new SharedFlags(FF, 0));
        l.add(wall);
        Scenario creative = elytra(flat("elytra_creative")).desc("creative: double taps toggle flight; flying clears flag 7")
                .spawn(0.5, 80, 0.5, 0, 0).mode(Scenario.GameMode.CREATIVE);
        creative.run(3, 0).run(1, JUMP).run(2, 0).run(1, JUMP).run(10, 0).run(1, JUMP).run(5, 0).run(1, JUMP).run(2, 0)
                .run(1, JUMP).run(30, 0);
        addEvent(creative, 4, new SharedFlags(FF, 0));
        addEvent(creative, 20, new SharedFlags(FF, 0));
        l.add(creative);
        Scenario lev = elytra(flat("elytra_levitation")).desc("levitation: rising, no request; the flag set anyway: the glide ignores it")
                .spawn(0.5, 80, 0.5, 0, 0).effect(25, 0);
        lev.run(3, 0).run(1, JUMP).run(20, 0);
        addEvent(lev, 8, new SharedFlags(FF, 0));
        l.add(lev);
        Scenario ceil = elytra(flat("elytra_ceiling").fill(-4, 66, 2, 4, 66, 30, s("stone"), 0))
                .desc("glide under a low ceiling (0.6 high), then the flag is cleared under it (the size stays)")
                .spawn(0.5, 65.2, 0.5, 0, 0).velocity(0, 0, 1.2).run(1, 0).run(12, 0).run(10, FORWARD);
        addEvent(ceil, 0, new SharedFlags(FF, 0));
        addEvent(ceil, 8, new SharedFlags(0, FF));
        l.add(ceil);
        Scenario fw = elytra(flat("elytra_fireworks")).desc("two rockets: boost along the look, removed by the server")
                .spawn(0.5, 120, 0.5, 30, 0).run(3, 0).run(1, JUMP).run(10, 0).run(40, 0, 30, -20).run(30, 0, 60, 10);
        addEvent(fw, 4, new SharedFlags(FF, 0));
        addEvent(fw, 8, new Firework(1001));
        addEvent(fw, 20, new Firework(1002));
        addEvent(fw, 26, new Ent.Remove(1001));
        addEvent(fw, 40, new Ent.Remove(1002));
        addEvent(fw, 50, new Firework(1003));
        addEvent(fw, 75, new Ent.Remove(1003));
        l.add(fw);
        Scenario fwGround = elytra(flat("elytra_firework_not_gliding")).desc("a rocket on a player who is not gliding: no boost")
                .spawn(0.5, 64, 0.5, 0, -30).run(5, FORWARD);
        addEvent(fwGround, 1, new Firework(1001));
        l.add(fwGround.run(20, FORWARD).run(1, JUMP).run(10, 0));
        Scenario flags = elytra(flat("elytra_server_flags")).desc("server entity data: sprint and glide bits set and cleared")
                .spawn(0.5, 100, 0.5, 0, 0).run(3, FORWARD);
        flags.run(10, FORWARD).run(30, FORWARD | SPRINT).run(20, 0);
        addEvent(flags, 2, new SharedFlags(FF, 0));
        addEvent(flags, 12, new SharedFlags(0x08, 0));
        addEvent(flags, 20, new SharedFlags(0x10, 0x08));
        addEvent(flags, 30, new SharedFlags(0, 0x90));
        addEvent(flags, 40, new SharedFlags(0x88, 0));
        l.add(flags);
        Scenario started = elytra(flat("elytra_setup_gliding")).desc("join gliding (flag 7 at setup), sneak and strafe")
                .spawn(0.5, 100, 0.5, 45, 10).velocity(1.0, -0.2, 1.0);
        setup(started).fallFlying = true;
        l.add(started.run(10, 0).run(10, SNEAK).run(10, LEFT | FORWARD).run(10, BACK).run(10, JUMP));
        Scenario unequip = elytra(flat("elytra_unequip")).desc("elytra taken off while gliding: the client glides on; no request without it")
                .spawn(0.5, 110, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(15, 0).run(1, JUMP).run(10, 0);
        addEvent(unequip, 4, new SharedFlags(FF, 0));
        addEvent(unequip, 10, new Equip("chest", null, 0));
        addEvent(unequip, 15, new SharedFlags(0, FF));
        addEvent(unequip, 22, new Equip("chest", "elytra", 0));
        l.add(unequip.run(1, JUMP).run(15, 0));
        Scenario edge = elytra(new Scenario("elytra_rocket_chunk_edge").outside(Scenario.Outside.UNLOADED)
                .fill(-16, 63, -96, -1, 63, 15, s("stone"), 0))
                .desc("glide over a loaded / unloaded chunk border with a rocket").spawn(-0.3, 110, 8.5, 180, 0)
                .run(3, 0).run(1, JUMP).run(30, 0, 200, 0);
        addEvent(edge, 4, new SharedFlags(FF, 0));
        addEvent(edge, 6, new Firework(1001));
        addEvent(edge, 16, new Ent.Remove(1001));
        l.add(edge);
        Scenario steep = elytra(flat("elytra_steep")).desc("look straight down and straight up while gliding (lift, conversion terms)")
                .spawn(0.5, 200, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(20, 0, 0, 90).run(20, 0, 0, -90).run(20, 0, 0, -45);
        addEvent(steep, 4, new SharedFlags(FF, 0));
        l.add(steep);
        Scenario use = elytra(flat("elytra_firework_use")).desc("use a rocket from the hand: on the ground, then gliding (use_item)")
                .spawn(0.5, 110, 0.5, 0, 0);
        setup(use).hand = "fireworks";
        use.run(3, 0).run(1, Scenario.USE).run(2, 0).run(1, JUMP).run(5, 0).run(1, Scenario.USE).run(30, 0);
        addEvent(use, 7, new SharedFlags(FF, 0));
        addEvent(use, 14, new Firework(1001));
        addEvent(use, 33, new Ent.Remove(1001));
        l.add(use);
        return l;
    }

    /**
     * Randomized elytra cases: the {@code withExtras} case with an elytra (sometimes broken or missing),
     * often raised and gliding from the start, jump flips, wide pitch changes, the server's flag 7 set
     * and cleared (no server logic: random), sprint / swim bits, rockets alive 1-25 ticks, the chest
     * item swapped. Cases get an {@code _el} suffix.
     */
    static Scenario fuzz(long seed, int index, int ticks) {
        Scenario base = Fuzz.withExtras(seed, index, ticks, false);
        Scenario sc = Fuzz.renamedTo(base, base.name + "_el");
        Rng x = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x656c79747261L);
        Setup st = setup(sc);
        if (x.chance(0.92)) {
            st.chest = "elytra";
            if (x.chance(0.05)) {
                st.chestDamage = 431;
            } else if (x.chance(0.2)) {
                st.chestDamage = x.range(1, 430);
            }
        }
        if (x.chance(0.6)) {
            sc.y = Math.min(250, sc.y + x.range(2, 60) + x.nextDouble());
        }
        if (x.chance(0.4)) {
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
        int nextId = 2000;
        for (int i = 0; i < sc.ticks.size(); i++) {
            if (x.chance(0.035)) {
                addEvent(sc, i, new SharedFlags(0, FF));
            }
            if (x.chance(0.05)) {
                addEvent(sc, i, new SharedFlags(FF, 0));
            }
            if (x.chance(0.01)) {
                int bits = x.pick(new int[] {0x08, 0x10, 0x18});
                addEvent(sc, i, x.chance(0.5) ? new SharedFlags(bits, 0) : new SharedFlags(0, bits));
            }
            if (x.chance(0.04) && live.size() < 3) {
                int id = nextId++;
                addEvent(sc, i, new Firework(id));
                live.add(new int[] {id, i + x.range(1, 25)});
            }
            for (int k = live.size() - 1; k >= 0; k--) {
                if (live.get(k)[1] == i) {
                    addEvent(sc, i, new Ent.Remove(live.get(k)[0]));
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
