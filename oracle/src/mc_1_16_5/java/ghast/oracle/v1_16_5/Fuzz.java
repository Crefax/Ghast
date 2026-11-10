package ghast.oracle.v1_16_5;

import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import net.minecraft.block.Block;
import net.minecraft.block.BlockState;
import net.minecraft.util.Identifier;
import net.minecraft.util.registry.Registry;

import java.util.ArrayList;
import java.util.List;

/**
 * Seeded random 1.16.5 scenarios: random terrain from a palette of movement-relevant blocks (each
 * placed in a random one of its block states, so connections, halves, facings, waterlogging, levels
 * vary) and random held-key spans with turning. Case i of seed s depends only on (s, i).
 */
final class Fuzz {
    private Fuzz() {}

    private record Entry(String block, int weight) {}

    /** Floor surface materials. */
    private static final Entry[] FLOOR = {
            new Entry("stone", 30), new Entry("grass_block", 4), new Entry("ice", 4), new Entry("packed_ice", 3),
            new Entry("blue_ice", 3), new Entry("slime_block", 4), new Entry("honey_block", 4), new Entry("soul_sand", 5),
            new Entry("soul_soil", 3), new Entry("farmland", 3), new Entry("grass_path", 3),
            new Entry("glass", 2), new Entry("water", 4), new Entry("lava", 1), new Entry("air", 3), new Entry("magma_block", 1),
            new Entry("smooth_stone_slab", 3), new Entry("oak_leaves", 1), new Entry("snow_block", 1),
    };

    /** Obstacles placed on or above the floor (random state each). */
    private static final Entry[] FEATURE = {
            new Entry("stone", 12), new Entry("smooth_stone_slab", 8), new Entry("oak_slab", 3), new Entry("oak_stairs", 6),
            new Entry("stone_brick_stairs", 3), new Entry("oak_fence", 5), new Entry("nether_brick_fence", 2),
            new Entry("oak_fence_gate", 3), new Entry("cobblestone_wall", 3), new Entry("glass_pane", 4), new Entry("iron_bars", 2),
            new Entry("snow", 4), new Entry("white_carpet", 2), new Entry("cobweb", 3), new Entry("ladder", 4), new Entry("vine", 3),
            new Entry("scaffolding", 4), new Entry("sweet_berry_bush", 3), new Entry("honey_block", 2), new Entry("soul_sand", 2),
            new Entry("slime_block", 2), new Entry("cactus", 1), new Entry("cake", 1), new Entry("enchanting_table", 1),
            new Entry("daylight_detector", 1), new Entry("oak_trapdoor", 3), new Entry("iron_trapdoor", 1), new Entry("oak_door", 2),
            new Entry("hopper", 1), new Entry("anvil", 1), new Entry("chest", 1), new Entry("end_portal_frame", 1),
            new Entry("brewing_stand", 1), new Entry("cauldron", 1), new Entry("flower_pot", 1), new Entry("red_bed", 1),
            new Entry("lily_pad", 1), new Entry("water", 4), new Entry("bubble_column", 1), new Entry("kelp", 1),
            new Entry("seagrass", 1), new Entry("chain", 1), new Entry("lantern", 1), new Entry("campfire", 1),
            new Entry("lectern", 1), new Entry("grindstone", 1), new Entry("composter", 1), new Entry("bell", 1),
            new Entry("end_rod", 1), new Entry("sea_pickle", 1), new Entry("turtle_egg", 1), new Entry("stonecutter", 1),
            new Entry("twisting_vines", 1), new Entry("weeping_vines", 1), new Entry("bamboo", 1), new Entry("piston", 1),
            new Entry("piston_head", 1), new Entry("skeleton_skull", 1), new Entry("conduit", 1), new Entry("air", 3),
    };

    private static String pick(Rng r, Entry[] table) {
        int total = 0;
        for (Entry e : table) {
            total += e.weight;
        }
        int x = r.nextInt(total);
        for (Entry e : table) {
            x -= e.weight;
            if (x < 0) {
                return e.block;
            }
        }
        throw new AssertionError();
    }

    /** A random state of the block (all of its states are equally likely). */
    private static int randomState(Rng r, String block) {
        Block b = Registry.BLOCK.get(new Identifier(block));
        List<BlockState> states = b.getStateManager().getStates();
        return States.id(states.get(r.nextInt(states.size())));
    }

    private static int defaultState(String block) {
        return States.id(Registry.BLOCK.get(new Identifier(block)).getDefaultState());
    }

    private static final int[][] ORIGINS = {{0, 0}, {0, 0}, {137, -54}, {-1000, 3000}, {123_456, -654_321},
            {-4_000_000, 8_000_000}, {29_999_900, -29_999_900}};

    private static final int[] KEY_COMBOS = {
            0, Scenario.FORWARD, Scenario.FORWARD, Scenario.FORWARD | Scenario.SPRINT,
            Scenario.FORWARD | Scenario.SPRINT, Scenario.FORWARD | Scenario.JUMP,
            Scenario.FORWARD | Scenario.SPRINT | Scenario.JUMP, Scenario.FORWARD | Scenario.SPRINT | Scenario.JUMP,
            Scenario.JUMP, Scenario.SNEAK, Scenario.FORWARD | Scenario.SNEAK, Scenario.BACK,
            Scenario.LEFT, Scenario.RIGHT, Scenario.FORWARD | Scenario.LEFT, Scenario.FORWARD | Scenario.RIGHT,
            Scenario.BACK | Scenario.LEFT | Scenario.JUMP, Scenario.FORWARD | Scenario.SNEAK | Scenario.JUMP,
            Scenario.FORWARD | Scenario.BACK, Scenario.LEFT | Scenario.RIGHT | Scenario.SPRINT,
    };

    static List<Scenario> cases(long seed, int count, int ticks) {
        Harness.init();
        List<Scenario> out = new ArrayList<>(count);
        for (int i = 0; i < count; i++) {
            out.add(one(seed, i, ticks, false));
        }
        return out;
    }

    /** Surfaces for the edge mode (random state each: slab types, snow layers, bites, facings, ...). */
    private static final Entry[] PARTIAL = {
            new Entry("soul_sand", 6), new Entry("soul_soil", 1), new Entry("smooth_stone_slab", 6), new Entry("oak_slab", 2),
            new Entry("snow", 6), new Entry("farmland", 4), new Entry("grass_path", 4), new Entry("white_carpet", 3),
            new Entry("honey_block", 3), new Entry("slime_block", 1), new Entry("stone", 4), new Entry("grass_block", 3),
            new Entry("cake", 1), new Entry("daylight_detector", 1), new Entry("enchanting_table", 1), new Entry("oak_stairs", 2),
            new Entry("stone_brick_stairs", 1), new Entry("oak_trapdoor", 2), new Entry("iron_trapdoor", 1),
            new Entry("end_portal_frame", 1), new Entry("red_bed", 1), new Entry("scaffolding", 2), new Entry("sea_pickle", 1),
            new Entry("lantern", 1), new Entry("lily_pad", 1), new Entry("stonecutter", 1), new Entry("chest", 1),
            new Entry("hopper", 1), new Entry("composter", 1), new Entry("campfire", 1), new Entry("lectern", 1),
            new Entry("grindstone", 1), new Entry("bell", 1), new Entry("conduit", 1), new Entry("flower_pot", 1),
            new Entry("turtle_egg", 1), new Entry("cauldron", 1), new Entry("brewing_stand", 1), new Entry("anvil", 1),
            new Entry("oak_fence", 1), new Entry("cobblestone_wall", 1), new Entry("oak_fence_gate", 1), new Entry("ice", 1),
            new Entry("packed_ice", 1), new Entry("blue_ice", 1), new Entry("frosted_ice", 1), new Entry("magma_block", 1),
            new Entry("water", 1), new Entry("cobweb", 1), new Entry("sweet_berry_bush", 1), new Entry("chain", 1),
            new Entry("skeleton_skull", 1), new Entry("piston_head", 1), new Entry("end_rod", 1), new Entry("bamboo", 1),
    };

    private static final int[] EDGE_KEYS = {
            Scenario.FORWARD | Scenario.JUMP, Scenario.FORWARD | Scenario.JUMP, Scenario.FORWARD | Scenario.SPRINT | Scenario.JUMP,
            Scenario.FORWARD, Scenario.FORWARD | Scenario.SPRINT, Scenario.FORWARD | Scenario.LEFT | Scenario.JUMP,
            Scenario.FORWARD | Scenario.RIGHT | Scenario.JUMP, Scenario.FORWARD | Scenario.SNEAK, Scenario.BACK | Scenario.JUMP, 0,
    };

    /**
     * Edge mode: a floor of partial-height blocks (soul sand, slabs, snow layers, farmland, carpets,
     * honey, scaffolding, ...) in strips next to each other and next to full blocks and one-block
     * steps, walked over with mostly forward + jump input, so landings, step-ups and auto-jumps at
     * small height differences are frequent. Case i of seed s depends only on (s, i).
     */
    static Scenario edge(long seed, int index, int ticks) {
        Harness.init();
        Rng r = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x65646765L);
        r.nextLong();
        Scenario sc = new Scenario("edge_s" + seed + "_" + index);
        int[] origin = ORIGINS[r.nextInt(ORIGINS.length)];
        int ox = origin[0] + r.range(-20, 20), oz = origin[1] + r.range(-20, 20);
        int floorY = 63;
        sc.desc("edge terrain at " + ox + "," + oz);
        sc.fill(ox - 16, floorY - 1, oz - 16, ox + 16, floorY - 1, oz + 16, defaultState("stone"), 0);
        sc.fill(ox - 16, floorY, oz - 16, ox + 16, floorY, oz + 16, defaultState("grass_block"), 0);
        boolean alongX = r.chance(0.3);
        int pos = -14;
        while (pos < 14) {
            int w = r.range(1, 4);
            int st = randomState(r, pick(r, PARTIAL));
            if (alongX) {
                sc.fill(ox + pos, floorY, oz - 14, ox + pos + w - 1, floorY, oz + 14, st, 0);
            } else {
                sc.fill(ox - 14, floorY, oz + pos, ox + 14, floorY, oz + pos + w - 1, st, 0);
            }
            if (r.chance(0.25)) {
                // A raised step on top of the strip: partial or full block.
                int up = randomState(r, pick(r, PARTIAL));
                if (alongX) {
                    sc.fill(ox + pos, floorY + 1, oz - 14, ox + pos + w - 1, floorY + 1, oz + 14, up, 0);
                } else {
                    sc.fill(ox - 14, floorY + 1, oz + pos, ox + 14, floorY + 1, oz + pos + w - 1, up, 0);
                }
            }
            pos += w;
        }
        double across = 0.05 + 0.9 * r.nextDouble() - 12;
        double side = 0.05 + 0.9 * r.nextDouble() + r.range(-3, 3);
        double sx = alongX ? ox + across : ox + side;
        double sz = alongX ? oz + side : oz + across;
        float yaw = alongX ? -90 + (float) ((r.nextDouble() - 0.5) * 50) : (float) ((r.nextDouble() - 0.5) * 50);
        sc.spawn(sx, floorY + 3, sz, yaw, (float) (r.nextDouble() * 30));
        // 1.16 extras that change landings and step-ups: auto-jump, soul speed boots, jump boost.
        if (r.chance(0.2)) {
            sc.autoJump = true;
        }
        if (r.chance(0.1)) {
            sc.soulSpeed = r.range(1, 3);
        }
        if (r.chance(0.12)) {
            sc.effect(8, r.range(0, 2));
        }
        if (r.chance(0.15)) {
            // A low ceiling over the strips: head bumps, crouch-only gaps, blocked auto-jumps.
            int cy = floorY + r.range(3, 4);
            String c = r.pick(new String[] {"stone", "smooth_stone_slab", "smooth_stone_slab[type=top]", "scaffolding"});
            sc.fill(ox - 14, cy, oz - 14, ox + 14, cy, oz + 14, States.id(c), 0);
        }
        int t = 0;
        while (t < ticks) {
            int keys = r.pick(EDGE_KEYS);
            int span = Math.min(ticks - t, r.range(3, 15));
            float dyaw = r.chance(0.6) ? 0 : (float) ((r.nextDouble() - 0.5) * 6);
            for (int k = 0; k < span; k++) {
                sc.ticks.add(new Scenario.Tick(keys, sc.lastYaw() + dyaw, sc.lastPitch(), List.of()));
                t++;
            }
        }
        return sc;
    }

    /** The client's own sprint modifier (a server echoes it in entity properties while the player sprints). */
    private static final java.util.UUID SPRINT_UUID = java.util.UUID.fromString("662A6B8D-DA3E-4C1C-8813-96EA6097278D");

    /** Effects the 1.16.5 client reads itself: jump boost, blindness, levitation, slow falling, dolphin's grace. */
    private static List<int[]> randomEffects(Rng x) {
        List<int[]> fx = new ArrayList<>();
        int[][] pool = {{8, 3}, {15, 0}, {25, 2}, {28, 0}, {30, 0}};
        for (int[] e : pool) {
            if (x.chance(0.3)) {
                fx.add(new int[] {e[0], x.range(0, e[1])});
            }
        }
        return fx;
    }

    private static List<Scenario.Modifier> randomModifiers(Rng x, boolean sprintEcho) {
        List<Scenario.Modifier> mods = new ArrayList<>();
        int n = x.range(0, 3);
        for (int i = 0; i < n; i++) {
            int op = x.nextInt(3);
            double amount = (x.nextDouble() - 0.3) * (op == 0 ? 0.1 : 0.8);
            mods.add(new Scenario.Modifier(new java.util.UUID(0x7365727665724dL, x.nextLong()), amount, op));
        }
        if (sprintEcho) {
            mods.add(x.nextInt(mods.size() + 1), new Scenario.Modifier(SPRINT_UUID, 0.30000001192092896, 2));
        }
        return mods;
    }

    private static void addEvent(Scenario sc, int tick, Scenario.Event e) {
        Scenario.Tick tk = sc.ticks.get(tick);
        List<Scenario.Event> events = new ArrayList<>(tk.events());
        events.add(e);
        sc.ticks.set(tick, new Scenario.Tick(tk.keys(), tk.yaw(), tk.pitch(), List.copyOf(events)));
    }

    /**
     * Like {@link #one(long, int, int, boolean)}, plus (from a separate stream, so the terrain and the
     * input are the same case) what else the 1.16.5 client reads: server speed modifiers of every
     * operation, depth strider / soul speed boots, spectator mode, fly speeds, food at the sprint
     * threshold, client-read effects (blindness, jump boost, levitation, slow falling, dolphin's grace),
     * a bubble column in a pool at the spawn, and inbound packets during the case: entity properties
     * (with or without the sprint modifier echoed), effect changes, abilities, teleports with relative
     * flags. Cases get an {@code _x} suffix.
     */
    static Scenario withExtras(long seed, int index, int ticks, boolean useItem) {
        Scenario sc = one(seed, index, ticks, useItem);
        Scenario out = new Scenario(sc.name + "_x");
        copyInto(sc, out);
        sc = out;
        Rng x = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x6578747261L);
        if (x.chance(0.5)) {
            int n = x.range(1, 3);
            for (int i = 0; i < n; i++) {
                int op = x.nextInt(3);
                double amount = (x.nextDouble() - 0.3) * (op == 0 ? 0.1 : 0.8);
                sc.speedModifiers.add(new double[] {amount, op});
            }
        }
        if (x.chance(0.15)) {
            sc.movementSpeedBase = 0.05 + 0.15 * x.nextDouble();
        }
        if (x.chance(0.3)) {
            sc.depthStrider = x.range(1, 5); // vanilla caps the effect at 3; commands give more
        }
        if (x.chance(0.2)) {
            sc.soulSpeed = x.range(1, 5);
        }
        if (x.chance(0.08)) {
            // Move the whole case next to the default world border (radius 29,999,984).
            int bx = x.chance(0.5) ? 29_999_984 : -29_999_984, bz = x.chance(0.5) ? 29_999_984 : -29_999_984;
            boolean both = x.chance(0.3), alongX = x.chance(0.5);
            int dx = both || alongX ? bx + x.range(-4, 4) - (int) Math.floor(sc.x) : 0;
            int dz = both || !alongX ? bz + x.range(-4, 4) - (int) Math.floor(sc.z) : 0;
            List<Scenario.Fill> moved = new ArrayList<>();
            for (Scenario.Fill f : sc.fills) {
                moved.add(new Scenario.Fill(f.x0() + dx, f.y0(), f.z0() + dz, f.x1() + dx, f.y1(), f.z1() + dz, f.id(), f.meta()));
            }
            sc.fills.clear();
            sc.fills.addAll(moved);
            sc.x += dx;
            sc.z += dz;
            sc.description += ", at the world border";
        }
        if (x.chance(0.15)) {
            sc.foodLevel = x.range(5, 7);
        }
        if (x.chance(0.2)) {
            sc.flySpeed = (float) (0.01 + 0.2 * x.nextDouble());
        }
        if (x.chance(0.3)) {
            for (int[] e : randomEffects(x)) {
                sc.effect(e[0], e[1]);
            }
        }
        if (x.chance(0.12)) {
            sc.mode(Scenario.GameMode.SPECTATOR);
            sc.flying(true);
        }
        if (x.chance(0.15)) {
            // A pool with a bubble column through the spawn (drag up or down).
            int bx = (int) Math.floor(sc.x), bz = (int) Math.floor(sc.z), floorY = 63;
            int depth = x.range(3, 7);
            sc.fill(bx - 2, floorY - depth - 1, bz - 2, bx + 2, floorY - depth - 1, bz + 2, defaultState("stone"), 0);
            sc.fill(bx - 2, floorY - depth, bz - 2, bx + 2, floorY + x.range(0, 2), bz + 2, defaultState("water"), 0);
            int top = floorY + x.range(-1, 1);
            String drag = x.chance(0.5) ? "true" : "false";
            sc.fill(bx, floorY - depth, bz, bx, top, bz, States.id("bubble_column[drag=" + drag + "]"), 0);
            sc.y = floorY - depth + x.nextInt(depth) + x.nextDouble();
        }
        boolean creative = sc.gameMode == Scenario.GameMode.CREATIVE, spectator = sc.gameMode == Scenario.GameMode.SPECTATOR;
        int n = sc.ticks.size();
        if (n > 1) {
            int events = x.chance(0.5) ? x.range(1, 4) : 0;
            for (int k = 0; k < events; k++) {
                int at = x.nextInt(n);
                switch (x.nextInt(4)) {
                    case 0 -> addEvent(sc, at, new Scenario.SpeedAttribute(x.chance(0.7) ? 0.1 : 0.05 + 0.15 * x.nextDouble(),
                            randomModifiers(x, x.chance(0.5))));
                    case 1 -> addEvent(sc, at, new Scenario.Effects(randomEffects(x)));
                    case 2 -> {
                        boolean allow = creative || spectator || x.chance(0.5);
                        // Spectators may be sent flying=false: the client turns flight back on.
                        boolean flying = allow && x.chance(0.5);
                        float fly = x.chance(0.6) ? 0.05f : (float) (0.01 + 0.2 * x.nextDouble());
                        addEvent(sc, at, new Scenario.Abilities(creative || spectator, flying, allow, creative, fly, 0.1f));
                    }
                    default -> {
                        int flags = x.chance(0.3) ? 0 : x.nextInt(32);
                        double dx = (flags & 1) != 0 ? (x.nextDouble() - 0.5) * 2 : sc.x + (x.nextDouble() - 0.5) * 4;
                        double dy = (flags & 2) != 0 ? (x.nextDouble() - 0.3) * 2 : sc.y + x.nextDouble() * 2;
                        double dz = (flags & 4) != 0 ? (x.nextDouble() - 0.5) * 2 : sc.z + (x.nextDouble() - 0.5) * 4;
                        float yaw = (flags & 8) != 0 ? (float) ((x.nextDouble() - 0.5) * 90) : (float) (x.nextDouble() * 720 - 360);
                        float pitch = (flags & 16) != 0 ? (float) ((x.nextDouble() - 0.5) * 60) : (float) (x.nextDouble() * 180 - 90);
                        addEvent(sc, at, new Scenario.Teleport(dx, dy, dz, yaw, pitch, flags));
                    }
                }
            }
        }
        sc.description += ", extras";
        return sc;
    }

    private static void copyInto(Scenario src, Scenario sc) {
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
        sc.effects.putAll(src.effects);
        sc.speedModifiers.addAll(src.speedModifiers);
        sc.depthStrider = src.depthStrider;
        sc.soulSpeed = src.soulSpeed;
        sc.autoJump = src.autoJump;
        sc.bb = src.bb;
        sc.pose = src.pose;
        sc.ticks.addAll(src.ticks);
    }

    static Scenario one(long seed, int index, int ticks, boolean useItem) {
        Harness.init();
        Rng r = new Rng(seed * 0x9E3779B97F4A7C15L + index);
        r.nextLong();
        Scenario sc = new Scenario("fuzz_s" + seed + "_" + index + (useItem ? "_use" : ""));
        int[] origin = ORIGINS[r.nextInt(ORIGINS.length)];
        int ox = origin[0] + r.range(-20, 20), oz = origin[1] + r.range(-20, 20);
        int size = r.range(4, 9);
        int floorY = 63;
        int stone = defaultState("stone");
        int air = 0;
        sc.desc("random terrain " + (2 * size + 1) + "x" + (2 * size + 1) + " at " + ox + "," + oz);

        int apron = size + 10;
        sc.fill(ox - apron, floorY - 1, oz - apron, ox + apron, floorY, oz + apron, stone, 0);
        int patches = r.range(1, 8);
        for (int p = 0; p < patches; p++) {
            String b = pick(r, FLOOR);
            int x0 = ox + r.range(-size, size), z0 = oz + r.range(-size, size);
            sc.fill(x0, floorY, z0, x0 + r.range(0, 4), floorY, z0 + r.range(0, 4), randomState(r, b), 0);
        }
        // Sometimes a pool: water two deep with a possible column of bubbles.
        if (r.chance(0.2)) {
            int x0 = ox + r.range(-size, size - 3), z0 = oz + r.range(-size, size - 3);
            sc.fill(x0, floorY - 1, z0, x0 + 3, floorY + r.range(0, 2), z0 + 3, defaultState("water"), 0);
        }
        int features = r.range(5, 40);
        for (int f = 0; f < features; f++) {
            String b = pick(r, FEATURE);
            int x = ox + r.range(-size, size), z = oz + r.range(-size, size), y = floorY + 1 + r.nextInt(3);
            int st = randomState(r, b);
            switch (r.nextInt(6)) {
                case 0 -> sc.fill(x, y, z, x + r.range(0, 4), y, z, st, 0);
                case 1 -> sc.fill(x, y, z, x, y, z + r.range(0, 4), st, 0);
                case 2 -> sc.fill(x, floorY + 1, z, x, floorY + r.range(1, 5), z, st, 0);
                default -> sc.block(x, y, z, st, 0);
            }
        }
        if (r.chance(0.4)) {
            int wx = ox + r.range(-2, 2), wz = oz + r.range(-2, 2), h = r.range(2, 6);
            int dir = r.nextInt(4);
            int[][] off = {{0, 1}, {0, -1}, {1, 0}, {-1, 0}};
            String[] ladder = {"ladder[facing=north]", "ladder[facing=south]", "ladder[facing=west]", "ladder[facing=east]"};
            String[] vine = {"vine[south=true]", "vine[north=true]", "vine[east=true]", "vine[west=true]"};
            sc.fill(wx + off[dir][0], floorY + 1, wz + off[dir][1], wx + off[dir][0], floorY + h + 1, wz + off[dir][1], stone, 0);
            int kind = r.nextInt(3);
            String climb = kind == 0 ? vine[dir] : kind == 1 ? ladder[dir] : "scaffolding[distance=0]";
            sc.fill(wx, floorY + 1, wz, wx, floorY + h, wz, States.id(climb), 0);
        }
        if (r.chance(0.2)) {
            int cy = floorY + r.range(2, 4);
            sc.fill(ox - r.range(0, size), cy, oz - r.range(0, size), ox + r.range(0, size), cy, oz + r.range(0, size),
                    r.chance(0.5) ? stone : States.id(r.chance(0.5) ? "smooth_stone_slab[type=top]" : "smooth_stone_slab"), 0);
        }
        double sx = ox + r.range(-2, 2) + 0.05 + 0.9 * r.nextDouble();
        double sz = oz + r.range(-2, 2) + 0.05 + 0.9 * r.nextDouble();
        int bx = (int) Math.floor(sx), bz = (int) Math.floor(sz);
        double sy = floorY + 1 + (r.chance(0.6) ? 0 : r.nextInt(6) + r.nextDouble());
        sc.fill(bx - 1, floorY + 1, bz - 1, bx + 1, floorY + 3 + (int) (sy - floorY), bz + 1, air, 0);
        if (r.chance(0.7)) {
            sc.block(bx, floorY, bz, stone, 0);
        }

        // Deep water around the spawn (swimming, sprint-swimming, surfacing).
        if (r.chance(0.15)) {
            int w = r.range(3, 7);
            sc.fill(ox - w, floorY - 9, oz - w, ox + w, floorY - 9, oz + w, stone, 0);
            sc.fill(ox - w, floorY - 8, oz - w, ox + w, floorY + r.range(-1, 1), oz + w, defaultState("water"), 0);
            sy = floorY - 7 + r.nextInt(7) + r.nextDouble();
        }
        if (r.chance(0.1)) {
            sc.outside(Scenario.Outside.UNLOADED);
        }
        float yaw = (float) (r.nextDouble() * 720 - 360);
        float pitch = (float) (r.nextDouble() * 180 - 90);
        sc.spawn(sx, sy, sz, yaw, pitch);
        if (r.chance(0.15)) {
            sc.mode(Scenario.GameMode.CREATIVE);
            if (r.chance(0.6)) {
                sc.flying(true);
                sc.y += 1 + r.nextInt(3);
            }
        }
        if (r.chance(0.2)) {
            sc.velocity((r.nextDouble() - 0.5) * 0.6, (r.nextDouble() - 0.3) * 0.6, (r.nextDouble() - 0.5) * 0.6);
        }
        if (r.chance(0.1)) {
            sc.foodLevel = r.range(0, 8);
        }
        if (r.chance(0.1)) {
            sc.speedModifiers.add(new double[] {0.2 * r.range(1, 3), 2});
        }
        if (r.chance(0.08)) {
            sc.effect(r.pick(new int[] {8, 25, 28, 30}), r.range(0, 2));
        }
        if (r.chance(0.08)) {
            sc.soulSpeed = r.range(1, 3);
        }
        if (r.chance(0.05)) {
            sc.depthStrider = r.range(1, 3);
        }
        if (r.chance(0.05)) {
            sc.autoJump = true;
        }

        int t = 0;
        float dyaw = 0, dpitch = 0;
        while (t < ticks) {
            int keys = r.pick(KEY_COMBOS);
            if (r.chance(0.15)) {
                keys |= 1 << r.nextInt(7);
            }
            int span = Math.min(ticks - t, r.range(1, 12));
            if (r.chance(0.5)) {
                dyaw = r.chance(0.5) ? 0 : (float) ((r.nextDouble() - 0.5) * 30);
                dpitch = r.chance(0.7) ? 0 : (float) ((r.nextDouble() - 0.5) * 10);
            }
            for (int k = 0; k < span; k++) {
                float ny = sc.lastYaw() + dyaw;
                float np = Math.max(-90f, Math.min(90f, sc.lastPitch() + dpitch));
                if (r.chance(0.02)) {
                    sc.ticks.add(new Scenario.Tick(keys, ny, np, List.of(new Scenario.Velocity(
                            r.range(-4000, 4000), r.range(-1000, 4000), r.range(-4000, 4000)))));
                } else {
                    sc.ticks.add(new Scenario.Tick(keys, ny, np, List.of()));
                }
                t++;
            }
        }
        if (useItem) {
            Rng u = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x7573655f6974656dL);
            boolean held = u.chance(0.5);
            for (int i = 0; i < sc.ticks.size(); i++) {
                if (u.chance(0.12)) {
                    held = !held;
                }
                if (held) {
                    Scenario.Tick tk = sc.ticks.get(i);
                    sc.ticks.set(i, new Scenario.Tick(tk.keys() | Scenario.USE, tk.yaw(), tk.pitch(), tk.events()));
                }
            }
            sc.description += ", use key";
        }
        return sc;
    }
}
