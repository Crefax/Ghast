package ghast.oracle.v26_1;

import ghast.oracle.Io;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;

import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

import static ghast.oracle.Scenario.BACK;
import static ghast.oracle.Scenario.FORWARD;
import static ghast.oracle.Scenario.JUMP;
import static ghast.oracle.Scenario.LEFT;
import static ghast.oracle.Scenario.SNEAK;
import static ghast.oracle.Scenario.SPRINT;
import static ghast.oracle.Scenario.USE;

/** Hand-written and randomized cases for the mechanics of {@link Ext} (one group per mechanic). */
final class ExtCases {
    private ExtCases() {}

    private static int s(String spec) {
        return States.id(spec);
    }

    private static Scenario flat(String name) {
        return new Scenario(name).fill(-48, 63, -48, 48, 63, 48, s("stone"), 0);
    }

    private static Scenario elytra(Scenario sc) {
        Ext.setup(sc).chest = "elytra";
        return sc;
    }

    private static void addEvent(Scenario sc, int tick, Scenario.Event e) {
        Scenario.Tick tk = sc.ticks.get(tick);
        List<Scenario.Event> events = new ArrayList<>(tk.events());
        events.add(e);
        sc.ticks.set(tick, new Scenario.Tick(tk.keys(), tk.yaw(), tk.pitch(), List.copyOf(events)));
    }

    private static final int FF = 0x80;

    /** Mechanics with a golden file {@code <name>.jsonl.gz}: hand-written cases + the golden fuzz cases. */
    static final String[] MECHANICS = {"elytra", "riptide", "entities"};

    static List<Scenario> scenarios(String mechanic) {
        return switch (mechanic) {
            case "elytra" -> elytra();
            case "riptide" -> riptide();
            case "entities" -> entities();
            default -> throw new IllegalArgumentException("mechanic " + mechanic);
        };
    }

    static Scenario fuzz(String mechanic, long seed, int index, int ticks) {
        return switch (mechanic) {
            case "elytra" -> elytraFuzz(seed, index, ticks);
            case "riptide" -> riptideFuzz(seed, index, ticks);
            case "entities" -> entitiesFuzz(seed, index, ticks);
            default -> throw new IllegalArgumentException("mechanic " + mechanic);
        };
    }

    static List<Scenario> golden(String mechanic) {
        List<Scenario> l = new ArrayList<>(scenarios(mechanic));
        for (int i = 0; i < Main.GOLDEN_FUZZ_COUNT; i++) {
            l.add(fuzz(mechanic, Main.GOLDEN_FUZZ_SEED, i, Main.GOLDEN_FUZZ_TICKS));
        }
        return l;
    }

    static void writeGolden(Path dir) throws IOException {
        for (String m : MECHANICS) {
            Io.write(dir.resolve(m + ".jsonl.gz"), sink -> Main.runAll(golden(m), sink));
        }
    }

    /**
     * Commands: {@code <mechanic> [--out FILE]} (its golden file) and {@code ext-fuzz --mechanic M [--seed N]
     * [--count N] [--ticks N] [--out FILE]}. Returns false for an unknown command.
     */
    static boolean command(String cmd, Io.Args a, Path golden) throws IOException {
        if (List.of(MECHANICS).contains(cmd)) {
            Io.bestCompression = true;
            Io.write(a.path("--out", golden.resolve(cmd + ".jsonl.gz")), sink -> Main.runAll(golden(cmd), sink));
            return true;
        }
        if (cmd.equals("jdk-cos")) {
            jdkCos(Path.of(a.str("--out", null)));
            return true;
        }
        if (cmd.equals("ext-fuzz")) {
            String m = a.str("--mechanic", "elytra");
            long seed = a.lng("--seed", 1);
            int count = (int) a.lng("--count", 100);
            int ticks = (int) a.lng("--ticks", 100);
            Path out = a.path("--out", Path.of("cache", Main.VERSION, "fuzz-" + m, m + "_s" + seed + ".jsonl.gz"));
            Io.write(out, sink -> {
                for (int i = 0; i < count; i++) {
                    Main.run(fuzz(m, seed, i, ticks), sink);
                }
            });
            return true;
        }
        return false;
    }

    // ---- elytra ------------------------------------------------------------------------------------

    static List<Scenario> elytra() {
        Harness.init();
        List<Scenario> l = new ArrayList<>();
        l.add(elytra(flat("elytra_start_glide")).desc("fall, press jump in the air: START_FALL_FLYING, glide level, dive, climb")
                .spawn(0.5, 110, 0.5, 0, 0).run(4, 0).run(1, JUMP).run(30, 0).turn(10, 0, 0, 4).run(15, 0, 0, 40)
                .turn(20, 0, 0, -5).run(30, 0, 3, -50));
        l.add(elytra(flat("elytra_jump_from_ground")).desc("jump on the ground (no glide), keep holding (no retrigger), press again in the air")
                .spawn(0.5, 64, 0.5, 90, 10).run(2, 0).run(8, JUMP | FORWARD).run(1, FORWARD).run(1, JUMP | FORWARD).run(40, FORWARD));
        Scenario broken = elytra(flat("elytra_broken")).desc("elytra at max damage - 1: jump in the air does not glide")
                .spawn(0.5, 90, 0.5, 0, 0);
        Ext.setup(broken).chestDamage = 431;
        l.add(broken.run(3, 0).run(1, JUMP).run(20, 0).run(1, JUMP).run(10, 0));
        l.add(new Scenario("elytra_no_chest").fill(-8, 63, -8, 8, 63, 8, s("stone"), 0)
                .desc("no elytra: jump in the air does nothing").spawn(0.5, 80, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(20, 0));
        l.add(elytra(flat("elytra_water").fill(-6, 64, -6, 6, 70, 6, s("water"), 0))
                .desc("in water: no glide; fall into water while gliding (fluid travel first)")
                .spawn(0.5, 68, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(10, 0));
        l.add(elytra(flat("elytra_into_water").fill(-6, 60, 10, 6, 66, 30, s("water"), 0))
                .desc("glide down into a pool: fluid travel while the flag stays set; sprint-swim attempts")
                .spawn(0.5, 80, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(40, 0, 0, 30).run(20, FORWARD | SPRINT, 0, 30));
        Scenario land = elytra(flat("elytra_landing")).desc("glide to the ground: the client keeps gliding until the server clears flag 7")
                .spawn(0.5, 72, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(40, 0, 0, 15).run(20, FORWARD);
        addEvent(land, 52, new Ext.SharedFlags(0, FF));
        l.add(land);
        l.add(elytra(flat("elytra_ladder").fill(-1, 64, 12, 1, 90, 12, s("stone"), 0).fill(-1, 64, 11, 1, 90, 11, s("ladder[facing=north]"), 0))
                .desc("glide into a ladder: onClimbable stops gliding on the client (stopFallFlying)")
                .spawn(0.5, 85, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(40, FORWARD, 0, 5));
        l.add(elytra(flat("elytra_vines").fill(-3, 64, 10, 3, 90, 13, s("vine[south=true]"), 0))
                .desc("glide through vines (#can_glide_through): not climbable while gliding")
                .spawn(0.5, 85, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(30, 0, 0, 5).run(1, JUMP).run(10, 0));
        l.add(elytra(flat("elytra_wall").fill(-5, 64, 8, 5, 100, 8, s("stone"), 0))
                .desc("glide into a wall (horizontal collision)").spawn(0.5, 90, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(25, 0, 0, 10));
        Scenario creative = elytra(flat("elytra_creative")).desc("creative: a double tap toggles flight (no glide); flying: cannot glide")
                .spawn(0.5, 80, 0.5, 0, 0).mode(Scenario.GameMode.CREATIVE);
        l.add(creative.run(3, 0).run(1, JUMP).run(2, 0).run(1, JUMP).run(10, 0).run(1, JUMP).run(5, 0).run(1, JUMP).run(2, 0)
                .run(1, JUMP).run(30, 0));
        Scenario lev = elytra(flat("elytra_levitation")).desc("levitation: cannot glide").spawn(0.5, 80, 0.5, 0, 0)
                .effect(Scenarios.eff("levitation"), 0);
        l.add(lev.run(3, 0).run(1, JUMP).run(20, 0));
        Scenario slow = elytra(flat("elytra_slow_falling")).desc("slow falling while gliding (gravity 0.01 when falling)")
                .spawn(0.5, 100, 0.5, 0, 0).effect(Scenarios.eff("slow_falling"), 0);
        l.add(slow.run(3, 0).run(1, JUMP).run(40, 0, 0, 20).run(20, 0, 0, -30));
        l.add(elytra(flat("elytra_ceiling").fill(-4, 66, 2, 4, 66, 30, s("stone"), 0))
                .desc("glide under a low ceiling (pose 0.6 high), then the flag is cleared under it")
                .spawn(0.5, 65.2, 0.5, 0, 0).velocity(0, 0, 1.2).run(1, JUMP).run(12, 0).run(10, FORWARD));
        Scenario ceil = l.get(l.size() - 1);
        addEvent(ceil, 8, new Ext.SharedFlags(0, FF));
        Scenario fw = elytra(flat("elytra_fireworks")).desc("two rockets attached: boost along the look vector, removed by the server")
                .spawn(0.5, 120, 0.5, 30, 0).run(3, 0).run(1, JUMP).run(10, 0).run(40, 0, 30, -20).run(30, 0, 60, 10);
        addEvent(fw, 8, new Ext.Firework(1001));
        addEvent(fw, 20, new Ext.Firework(1002));
        addEvent(fw, 26, new Ext.Remove(1001));
        addEvent(fw, 40, new Ext.Remove(1002));
        addEvent(fw, 50, new Ext.Firework(1003));
        addEvent(fw, 75, new Ext.Remove(1003));
        l.add(fw);
        Scenario fwGround = elytra(flat("elytra_firework_not_gliding")).desc("a rocket attached to a player who is not gliding: no boost")
                .spawn(0.5, 64, 0.5, 0, -30).run(5, FORWARD);
        addEvent(fwGround, 1, new Ext.Firework(1001));
        l.add(fwGround.run(20, FORWARD).run(1, JUMP).run(10, 0));
        Scenario use = elytra(flat("elytra_firework_use")).desc("use a rocket while gliding: use_item + swing, the server spawns it 2 ticks later")
                .spawn(0.5, 110, 0.5, 0, 0);
        Ext.setup(use).hand = "firework_rocket";
        use.run(3, 0).run(1, USE).run(2, 0).run(1, JUMP).run(5, 0).run(1, USE).run(30, 0);
        addEvent(use, 14, new Ext.Firework(1001));
        addEvent(use, 33, new Ext.Remove(1001));
        l.add(use);
        Scenario flags = elytra(flat("elytra_server_flags")).desc("server entity data: sprint / swim bits, flag 7 set and cleared from the server")
                .spawn(0.5, 100, 0.5, 0, 0).run(3, FORWARD);
        flags.run(10, FORWARD).run(30, FORWARD | SPRINT).run(20, 0);
        addEvent(flags, 2, new Ext.SharedFlags(FF, 0));
        addEvent(flags, 12, new Ext.SharedFlags(0x08, 0));
        addEvent(flags, 20, new Ext.SharedFlags(0x10, 0x08));
        addEvent(flags, 30, new Ext.SharedFlags(0, 0x90));
        addEvent(flags, 40, new Ext.SharedFlags(0x88, 0));
        l.add(flags);
        Scenario started = elytra(flat("elytra_setup_gliding")).desc("join gliding (flag 7 in the setup entity data), sneak and strafe")
                .spawn(0.5, 100, 0.5, 45, 10).velocity(1.0, -0.2, 1.0);
        Ext.setup(started).fallFlying = true;
        l.add(started.run(10, 0).run(10, SNEAK).run(10, LEFT | FORWARD).run(10, BACK).run(10, JUMP));
        Scenario unequip = elytra(flat("elytra_unequip")).desc("elytra taken off while gliding: the client keeps gliding; no restart")
                .spawn(0.5, 110, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(15, 0).run(1, JUMP).run(10, 0);
        addEvent(unequip, 10, new Ext.Equip("chest", null, 0));
        addEvent(unequip, 15, new Ext.SharedFlags(0, FF));
        addEvent(unequip, 22, new Ext.Equip("chest", "elytra", 0));
        l.add(unequip.run(1, JUMP).run(15, 0));
        Scenario edge = elytra(new Scenario("elytra_rocket_chunk_edge").outside(Scenario.Outside.UNLOADED)
                .fill(-16, 63, -96, -1, 63, 15, s("stone"), 0))
                .desc("glide along a loaded / unloaded chunk border: the rocket at the right hand moves into the unloaded chunk and stops ticking; left-handed next")
                .spawn(-0.3, 110, 8.5, 180, 0).run(3, 0).run(1, JUMP).run(30, 0);
        addEvent(edge, 6, new Ext.Firework(1001));
        addEvent(edge, 16, new Ext.Remove(1001));
        l.add(edge);
        Scenario edgeLeft = elytra(new Scenario("elytra_rocket_chunk_edge_left").outside(Scenario.Outside.UNLOADED)
                .fill(-16, 63, -96, -1, 63, 15, s("stone"), 0))
                .desc("left-handed: the rocket stays on the loaded side and keeps boosting").spawn(-0.3, 110, 8.5, 180, 0);
        Ext.setup(edgeLeft).leftHanded = true;
        edgeLeft.run(3, 0).run(1, JUMP).run(30, 0);
        addEvent(edgeLeft, 6, new Ext.Firework(1001));
        addEvent(edgeLeft, 16, new Ext.Remove(1001));
        l.add(edgeLeft);
        Scenario steep = elytra(flat("elytra_steep")).desc("look straight down and straight up while gliding (lift, conversion terms)")
                .spawn(0.5, 200, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(20, 0, 0, 90).run(20, 0, 0, -90).run(20, 0, 0, -45);
        l.add(steep);
        return l;
    }

    /**
     * Randomized elytra cases: the {@code withExtras} case (terrain, attributes, effects, inbound packets)
     * with an elytra (sometimes broken or missing), often raised or started gliding, frequent jump presses,
     * wide pitch changes, and server packets: flag 7 cleared or set, sprint / swim bits, attached rockets
     * (added and removed), the chest item swapped; big dripleaf is removed from the terrain (see the
     * oracle README). Cases get an {@code _el} suffix.
     */
    static Scenario elytraFuzz(long seed, int index, int ticks) {
        Scenario base = Fuzz.withExtras(seed, index, ticks, false);
        Scenario sc = new Scenario(base.name + "_el");
        copy(base, sc);
        // No big dripleaf: an attached rocket's ray that hits one tilts it on the client (the block's
        // projectile hook runs client-side too), a world change the physics does not model.
        sc.fills.removeIf(f -> States.name(States.byId(f.id())).startsWith("minecraft:big_dripleaf["));
        Rng x = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x656c79747261L);
        Ext.Setup st = Ext.setup(sc);
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
        }
        if (x.chance(0.3)) {
            st.fallFlying = true;
        }
        if (x.chance(0.3)) {
            st.leftHanded = true;
        }
        if (x.chance(0.3)) {
            sc.velocity((x.nextDouble() - 0.5) * 3, (x.nextDouble() - 0.7) * 2, (x.nextDouble() - 0.5) * 3);
        }
        // Jump presses and look changes over the base input.
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
        int nextId = 2000;
        List<int[]> live = new ArrayList<>();
        for (int i = 0; i < sc.ticks.size(); i++) {
            if (x.chance(0.035)) {
                addEvent(sc, i, new Ext.SharedFlags(0, FF));
            }
            if (x.chance(0.015)) {
                addEvent(sc, i, new Ext.SharedFlags(FF, 0));
            }
            if (x.chance(0.01)) {
                int bits = x.pick(new int[] {0x08, 0x10, 0x18});
                addEvent(sc, i, x.chance(0.5) ? new Ext.SharedFlags(bits, 0) : new Ext.SharedFlags(0, bits));
            }
            if (x.chance(0.04) && live.size() < 3) {
                int id = nextId++;
                addEvent(sc, i, new Ext.Firework(id));
                live.add(new int[] {id, i + x.range(1, 25)});
            }
            for (int k = live.size() - 1; k >= 0; k--) {
                if (live.get(k)[1] == i) {
                    addEvent(sc, i, new Ext.Remove(live.get(k)[0]));
                    live.remove(k);
                }
            }
            if (x.chance(0.01)) {
                boolean on = x.chance(0.6);
                addEvent(sc, i, new Ext.Equip("chest", on ? "elytra" : null, on && x.chance(0.2) ? 431 : 0));
            }
        }
        sc.description += ", elytra";
        return sc;
    }

    /**
     * {@code Math.cos} of every float argument from bits 0 to {@code (float) (PI / 2)} + 64 ulps, each as 8
     * bytes little-endian, for the table of {@code ghast_physics::modern::jdk_math} (gliding reads
     * {@code Math.cos} of the pitch; this JVM's intrinsic is not the correctly rounded cosine).
     */
    static void jdkCos(Path out) throws IOException {
        int hi = Float.floatToIntBits((float) (Math.PI / 2)) + 64;
        try (java.io.OutputStream o = new java.io.BufferedOutputStream(java.nio.file.Files.newOutputStream(out), 1 << 20)) {
            byte[] buf = new byte[8 * 4096];
            int k = 0;
            for (int b = 0; b <= hi; b++) {
                long c = Double.doubleToRawLongBits(Math.cos((double) Float.intBitsToFloat(b)));
                for (int s = 0; s < 8; s++) {
                    buf[k++] = (byte) (c >>> (8 * s));
                }
                if (k == buf.length) {
                    o.write(buf);
                    k = 0;
                }
            }
            o.write(buf, 0, k);
        }
    }

    // ---- riptide -----------------------------------------------------------------------------------

    private static Scenario trident(Scenario sc, int level) {
        Ext.Setup st = Ext.setup(sc);
        st.hand = "trident";
        st.riptide = level;
        return sc;
    }

    private static Scenario pool(String name) {
        return flat(name).fill(-8, 60, -8, 8, 62, 8, s("stone"), 0).fill(-7, 61, -7, 7, 63, 7, s("water"), 0);
    }

    static List<Scenario> riptide() {
        Harness.init();
        List<Scenario> l = new ArrayList<>();
        Scenario w = trident(pool("riptide_water"), 3).desc("riptide III in water: hold 12 ticks, release looking up; the server's spin flag a tick later")
                .spawn(0.5, 62, 0.5, 0, -40).run(3, 0).run(12, USE).run(30, 0);
        addEvent(w, 16, new Ext.LivingFlags(4, 0));
        addEvent(w, 36, new Ext.LivingFlags(0, 4));
        l.add(w);
        l.add(trident(pool("riptide_short_hold"), 3).desc("released after 6 ticks: nothing").spawn(0.5, 62, 0.5, 0, -40)
                .run(3, 0).run(6, USE).run(10, 0).run(9, USE).run(10, 0).run(10, USE).run(15, 0));
        l.add(trident(flat("riptide_dry"), 3).desc("on land without rain: the trident cannot be used").spawn(0.5, 64, 0.5, 0, -30)
                .run(3, 0).run(15, USE).run(10, 0));
        Scenario rain = trident(flat("riptide_rain"), 2).desc("in the rain on open ground: move up 1.2 first, then the push").spawn(0.5, 64, 0.5, 45, -20);
        Ext.setup(rain).raining = true;
        l.add(rain.run(3, 0).run(12, USE).run(40, 0));
        Scenario roof = trident(flat("riptide_rain_roof").fill(-3, 67, -3, 3, 67, 3, s("stone"), 0), 2)
                .desc("rain under a roof: no riptide").spawn(0.5, 64, 0.5, 45, -20);
        Ext.setup(roof).raining = true;
        l.add(roof.run(3, 0).run(12, USE).run(10, 0));
        Scenario wall = trident(pool("riptide_wall").fill(-7, 61, 5, 7, 75, 5, s("stone"), 0), 3)
                .desc("launch into a wall: a horizontal collision ends the spin counter").spawn(0.5, 62, 0.5, 0, -10);
        wall.run(3, 0).run(11, USE).run(25, 0);
        addEvent(wall, 15, new Ext.LivingFlags(4, 0));
        addEvent(wall, 30, new Ext.LivingFlags(0, 4));
        l.add(wall);
        for (int lvl = 1; lvl <= 3; lvl++) {
            Scenario c = trident(pool("riptide_level" + lvl), lvl).desc("riptide " + lvl + " straight up, then down into the pool")
                    .spawn(0.5, 62, 0.5, 0, -90).run(2, 0).run(10, USE).run(40, 0, 0, 30);
            addEvent(c, 13, new Ext.LivingFlags(4, 0));
            addEvent(c, 33, new Ext.LivingFlags(0, 4));
            l.add(c);
        }
        Scenario low = trident(pool("riptide_spin_pose").fill(-7, 65, -7, 7, 65, 7, s("stone"), 0), 3)
                .desc("spin pose (0.6 high) under a ceiling until the flag clears").spawn(0.5, 62, 0.5, 90, 0);
        low.run(2, 0).run(10, USE).run(30, FORWARD);
        addEvent(low, 13, new Ext.LivingFlags(4, 0));
        addEvent(low, 25, new Ext.LivingFlags(0, 4));
        l.add(low);
        Scenario glide = trident(pool("riptide_then_glide"), 3).desc("riptide out of the water, then glide (elytra)")
                .spawn(0.5, 62, 0.5, 0, -60);
        Ext.setup(glide).chest = "elytra";
        glide.run(2, 0).run(10, USE).run(8, 0).run(1, JUMP).run(30, 0, 0, 10);
        addEvent(glide, 13, new Ext.LivingFlags(4, 0));
        addEvent(glide, 33, new Ext.LivingFlags(0, 4));
        l.add(glide);
        return l;
    }

    /**
     * Randomized riptide cases: the {@code withExtras} case, often with a pool at the spawn and rain, a
     * trident with riptide I-III in the hand, use-key spans of 3-25 ticks, wide pitch changes, the
     * server's spin flag set and cleared, sometimes an elytra. Cases get an {@code _rt} suffix.
     */
    static Scenario riptideFuzz(long seed, int index, int ticks) {
        Scenario base = Fuzz.withExtras(seed, index, ticks, false);
        Scenario sc = new Scenario(base.name + "_rt");
        copy(base, sc);
        Rng x = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x7269707469L);
        trident(sc, x.range(1, 3));
        Ext.Setup st = Ext.setup(sc);
        if (x.chance(0.7)) {
            int bx = (int) Math.floor(sc.x), by = (int) Math.floor(sc.y), bz = (int) Math.floor(sc.z);
            int r = x.range(2, 6), d = x.range(1, 4);
            sc.fill(bx - r, by - d, bz - r, bx + r, by + x.range(0, 2), bz + r, s("water"), 0);
        }
        if (x.chance(0.3)) {
            st.raining = true;
        }
        if (x.chance(0.2)) {
            st.chest = "elytra";
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
                addEvent(sc, i, new Ext.LivingFlags(4, 0));
            }
            if (x.chance(0.04)) {
                addEvent(sc, i, new Ext.LivingFlags(0, 4));
            }
            if (st.chest != null && x.chance(0.02)) {
                addEvent(sc, i, new Ext.SharedFlags(0, FF));
            }
        }
        sc.description += ", riptide";
        return sc;
    }

    // ---- other entities: collision and pushing ------------------------------------------------------

    private static Scenario with(Scenario sc, Ext.Spawn... spawns) {
        Ext.Setup st = Ext.setup(sc);
        if (st.entities == null) {
            st.entities = new ArrayList<>();
        }
        st.entities.addAll(List.of(spawns));
        return sc;
    }

    private static Ext.Spawn sp(int id, String type, double x, double y, double z) {
        return new Ext.Spawn(id, type, x, y, z, 0.0F);
    }

    static List<Scenario> entities() {
        Harness.init();
        List<Scenario> l = new ArrayList<>();
        l.add(with(flat("push_zombie"), sp(100, "zombie", 0.8, 64, 0.7)).desc("a zombie overlapping the player pushes it away each tick")
                .spawn(0.5, 64, 0.5, 0, 0).run(30, 0).run(20, FORWARD));
        l.add(with(flat("push_crowd"), sp(100, "villager", 0.2, 64, 0.5), sp(101, "cow", 0.9, 64, 0.2), sp(102, "pig", 0.5, 64, 1.0),
                sp(103, "sheep", 0.45, 64, 0.55)).desc("four mobs around the player: pushes add up in tick order")
                .spawn(0.5, 64, 0.5, 0, 0).run(40, 0));
        l.add(with(flat("push_walk_into_mobs"), sp(100, "cow", 0.5, 64, 3.0), sp(101, "cow", 0.9, 64, 4.2), sp(102, "iron_golem", 0.4, 64, 6))
                .desc("walk and sprint into cows and an iron golem").spawn(0.5, 64, 0.5, 0, 0).run(5, 0).run(40, FORWARD | SPRINT));
        l.add(with(flat("no_push_armor_stand_bat_parrot"), sp(100, "armor_stand", 0.6, 64, 0.6), sp(101, "bat", 0.4, 64.5, 0.4),
                sp(102, "parrot", 0.5, 64.2, 0.7)).desc("armor stands, bats and parrots do not push players").spawn(0.5, 64, 0.5, 0, 0).run(30, 0));
        l.add(with(flat("boat_collide"), sp(100, "oak_boat", 0.5, 64, 3.0)).desc("walk into a boat, jump on it, stand, walk off; sneak at its edge")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(12, FORWARD).run(1, FORWARD | JUMP).run(10, FORWARD).run(10, 0)
                .run(20, FORWARD | SNEAK).run(10, FORWARD));
        l.add(with(flat("boat_push"), sp(100, "oak_boat", 0.5, 64, 1.55), sp(101, "bamboo_raft", 1.8, 64, 0.5), sp(102, "birch_chest_boat", -0.9, 63.8, 0.5))
                .desc("boats next to the player push it (their box grown by 0.2)").spawn(0.5, 64, 0.5, 0, 0).run(40, 0));
        l.add(with(flat("shulker_block").fill(-3, 63, 2, 3, 63, 6, s("stone"), 0), sp(100, "shulker", 0.5, 64, 3.5), sp(101, "shulker", 0.5, 65, 4.5))
                .desc("shulkers collide like blocks: walk into one, jump up two").spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(10, FORWARD)
                .run(1, FORWARD | JUMP).run(12, FORWARD).run(1, FORWARD | JUMP).run(15, FORWARD));
        l.add(with(flat("minecart_near"), sp(100, "minecart", 0.7, 64, 0.6), sp(101, "furnace_minecart", 0.5, 64, 1.3))
                .desc("minecarts overlapping the player").spawn(0.5, 64, 0.5, 0, 0).run(30, 0));
        Scenario moving = with(flat("push_moving_zombie"), sp(100, "zombie", 3.0, 64, 0.5)).desc("a zombie walks through the standing player (teleport packets, interpolated)")
                .spawn(0.5, 64, 0.5, 0, 0).run(40, 0);
        for (int i = 0; i < 20; i++) {
            addEvent(moving, 2 + i, new Ext.Move(100, 3.0 - i * 0.25, 64, 0.5 + (i % 3) * 0.05, 90.0F, true));
        }
        l.add(moving);
        l.add(with(flat("push_ladder").fill(0, 64, 1, 0, 70, 1, s("stone"), 0).fill(0, 64, 0, 0, 70, 0, s("ladder[facing=north]"), 0),
                sp(100, "zombie", 0.8, 64.5, 0.5)).desc("on a ladder the player is not pushable").spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, FORWARD | JUMP).run(10, 0));
        Scenario spec = with(flat("push_spectator"), sp(100, "zombie", 0.8, 64, 0.7)).desc("spectators are not pushed").spawn(0.5, 64, 0.5, 0, 0);
        l.add(spec.mode(Scenario.GameMode.SPECTATOR).run(20, 0));
        Scenario boatStack = with(flat("boat_stand_on"), sp(100, "oak_boat", 0.5, 64, 0.5)).desc("stand on a boat (spawned in it, pushed up by the step), the boat moves away")
                .spawn(0.5, 64.5625, 0.5, 0, 0).run(15, 0);
        for (int i = 0; i < 10; i++) {
            addEvent(boatStack, 15 - 10 + i, new Ext.Move(100, 0.5 + i * 0.12, 64, 0.5, 0.0F, true));
        }
        l.add(boatStack.run(15, 0));
        Scenario spawnLate = flat("push_spawned_in").desc("mobs spawned and removed mid-case").spawn(0.5, 64, 0.5, 0, 0).run(40, 0);
        with(spawnLate);
        addEvent(spawnLate, 5, new Ext.Spawn(200, "zombie", 0.6, 64, 0.6, 45.0F));
        addEvent(spawnLate, 12, new Ext.Spawn(201, "slime", 0.3, 64, 0.3, 0.0F));
        addEvent(spawnLate, 20, new Ext.Remove(200));
        addEvent(spawnLate, 30, new Ext.Remove(201));
        l.add(spawnLate);
        return l;
    }

    private static final String[] ENTITY_POOL = {"zombie", "villager", "cow", "pig", "sheep", "chicken", "armor_stand", "bat", "parrot",
            "oak_boat", "spruce_boat", "birch_chest_boat", "bamboo_raft", "shulker", "iron_golem", "horse", "slime", "spider", "wolf",
            "minecart", "item_frame", "skeleton", "creeper", "llama", "camel"};

    /**
     * Randomized entity cases: the {@code withExtras} case with 1-6 entities of a mixed pool (mobs, armor
     * stands, bats, parrots, boats, rafts, shulkers, minecarts, ...) within two blocks of the spawn, moved
     * by teleport packets, spawned and removed during the case. Cases get an {@code _en} suffix.
     */
    static Scenario entitiesFuzz(long seed, int index, int ticks) {
        Scenario base = Fuzz.withExtras(seed, index, ticks, false);
        Scenario sc = new Scenario(base.name + "_en");
        copy(base, sc);
        Rng x = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x656e74697479L);
        with(sc);
        int n = x.range(1, 6);
        int nextId = 300;
        List<double[]> live = new ArrayList<>();
        for (int i = 0; i < n; i++) {
            double ex = sc.x + (x.nextDouble() - 0.5) * 4, ey = sc.y + (x.chance(0.6) ? 0 : (x.nextDouble() - 0.5) * 2), ez = sc.z + (x.nextDouble() - 0.5) * 4;
            Ext.Setup st = Ext.setup(sc);
            st.entities.add(new Ext.Spawn(nextId, x.pick(ENTITY_POOL), ex, ey, ez, (float) (x.nextDouble() * 360)));
            live.add(new double[] {nextId++, ex, ey, ez});
        }
        for (int t = 0; t < sc.ticks.size(); t++) {
            for (double[] e : live) {
                if (x.chance(0.15)) {
                    double step = x.chance(0.2) ? 1.0 : 0.25;
                    e[1] += (x.nextDouble() - 0.5) * step;
                    e[2] += x.chance(0.8) ? 0 : (x.nextDouble() - 0.5) * step;
                    e[3] += (x.nextDouble() - 0.5) * step;
                    if (x.chance(0.3)) {
                        // towards the player's spawn column
                        e[1] += (sc.x - e[1]) * 0.3;
                        e[3] += (sc.z - e[3]) * 0.3;
                    }
                    addEvent(sc, t, new Ext.Move((int) e[0], e[1], e[2], e[3], (float) (x.nextDouble() * 360), x.chance(0.7)));
                }
            }
            if (x.chance(0.02) && !live.isEmpty()) {
                double[] e = live.remove(x.nextInt(live.size()));
                addEvent(sc, t, new Ext.Remove((int) e[0]));
            }
            if (x.chance(0.03) && live.size() < 8) {
                double ex = sc.x + (x.nextDouble() - 0.5) * 3, ez = sc.z + (x.nextDouble() - 0.5) * 3;
                addEvent(sc, t, new Ext.Spawn(nextId, x.pick(ENTITY_POOL), ex, sc.y, ez, (float) (x.nextDouble() * 360)));
                live.add(new double[] {nextId++, ex, sc.y, ez});
            }
        }
        sc.description += ", entities";
        return sc;
    }

    /** Every field a fuzz case sets. */
    static void copy(Scenario src, Scenario sc) {
        sc.description = src.description;
        sc.outside = src.outside;
        sc.extraChunks.addAll(src.extraChunks);
        sc.fills.addAll(src.fills);
        sc.gameMode = src.gameMode;
        sc.x = src.x;
        sc.y = src.y;
        sc.z = src.z;
        sc.yaw = src.yaw;
        sc.pitch = src.pitch;
        sc.vx = src.vx;
        sc.vy = src.vy;
        sc.vz = src.vz;
        sc.onGround = src.onGround;
        sc.fallDistance = src.fallDistance;
        sc.sprinting = src.sprinting;
        sc.flying = src.flying;
        sc.allowFlying = src.allowFlying;
        sc.flySpeed = src.flySpeed;
        sc.walkSpeed = src.walkSpeed;
        sc.movementSpeedBase = src.movementSpeedBase;
        sc.foodLevel = src.foodLevel;
        sc.health = src.health;
        sc.effects.putAll(src.effects);
        sc.speedModifiers.addAll(src.speedModifiers);
        sc.attributeBases.putAll(src.attributeBases);
        sc.attributeModifiers.addAll(src.attributeModifiers);
        sc.depthStrider = src.depthStrider;
        sc.soulSpeed = src.soulSpeed;
        sc.autoJump = src.autoJump;
        sc.bb = src.bb;
        sc.pose = src.pose;
        sc.leatherBoots = src.leatherBoots;
        sc.delayedLoad = src.delayedLoad;
        sc.useSpeed = src.useSpeed;
        sc.useCanSprint = src.useCanSprint;
        sc.sprintWindow = src.sprintWindow;
        sc.ticks.addAll(src.ticks);
    }
}
