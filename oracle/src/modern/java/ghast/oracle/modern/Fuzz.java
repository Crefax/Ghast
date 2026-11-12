package ghast.oracle.modern;

import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import net.minecraft.world.level.block.state.BlockState;

import java.util.ArrayList;
import java.util.List;

/**
 * Seeded random 26.1 scenarios: the 1.16.5 generator (same palette, same random stream, same structure)
 * with 26.1 block names (grass_path is dirt_path, chain is iron_chain) and 26.1 effect ids, so the cases
 * exercise the same mechanics in both versions; block states are drawn from the 26.1 state lists.
 * Case i of seed s depends only on (s, i).
 */
final class Fuzz {
    private Fuzz() {}

    private record Entry(String block, int weight) {}

    /** Floor surface materials. */
    private static final Entry[] FLOOR = {
            new Entry("stone", 30), new Entry("grass_block", 4), new Entry("ice", 4), new Entry("packed_ice", 3),
            new Entry("blue_ice", 3), new Entry("slime_block", 4), new Entry("honey_block", 4), new Entry("soul_sand", 5),
            new Entry("soul_soil", 3), new Entry("farmland", 3), new Entry("dirt_path", 3),
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
            new Entry("seagrass", 1), new Entry("iron_chain", 1), new Entry("lantern", 1), new Entry("campfire", 1),
            new Entry("lectern", 1), new Entry("grindstone", 1), new Entry("composter", 1), new Entry("bell", 1),
            new Entry("end_rod", 1), new Entry("sea_pickle", 1), new Entry("turtle_egg", 1), new Entry("stonecutter", 1),
            new Entry("twisting_vines", 1), new Entry("weeping_vines", 1), new Entry("bamboo", 1), new Entry("piston", 1),
            new Entry("piston_head", 1), new Entry("skeleton_skull", 1), new Entry("conduit", 1), new Entry("air", 3),
    };

    private static double pickD(Rng r, double[] values) {
        return values[r.nextInt(values.length)];
    }

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
        List<BlockState> states = States.block(block).getStateDefinition().getPossibleStates();
        return States.id(states.get(r.nextInt(states.size())));
    }

    private static int defaultState(String block) {
        return States.id(States.block(block).defaultBlockState());
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
            sc.effect(r.pick(new int[] {Scenarios.eff("jump_boost"), Scenarios.eff("levitation"), Scenarios.eff("slow_falling"),
                    Scenarios.eff("dolphins_grace")}), r.range(0, 2));
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

    /** Surfaces for the edge mode (random state each): the 1.16.5 list plus partial blocks added since. */
    private static final Entry[] PARTIAL = {
            new Entry("soul_sand", 6), new Entry("soul_soil", 1), new Entry("smooth_stone_slab", 6), new Entry("oak_slab", 2),
            new Entry("snow", 6), new Entry("farmland", 4), new Entry("dirt_path", 4), new Entry("white_carpet", 3),
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
            new Entry("water", 1), new Entry("cobweb", 1), new Entry("sweet_berry_bush", 1), new Entry("iron_chain", 1),
            new Entry("skeleton_skull", 1), new Entry("piston_head", 1), new Entry("end_rod", 1), new Entry("bamboo", 1),
            // 1.17+
            new Entry("powder_snow", 4), new Entry("mud", 3), new Entry("pointed_dripstone", 2), new Entry("big_dripleaf", 2),
            new Entry("sculk_sensor", 1), new Entry("calibrated_sculk_sensor", 1), new Entry("sculk_shrieker", 1),
            new Entry("decorated_pot", 1), new Entry("copper_grate", 1), new Entry("moss_carpet", 1), new Entry("pale_moss_carpet", 1),
            new Entry("mangrove_roots", 1), new Entry("amethyst_cluster", 1), new Entry("candle", 1), new Entry("lightning_rod", 1),
            new Entry("heavy_core", 1), new Entry("vault", 1), new Entry("trial_spawner", 1), new Entry("azalea", 1),
            new Entry("dried_ghast", 1), new Entry("copper_chain", 1), new Entry("copper_lantern", 1), new Entry("oak_shelf", 1),
            new Entry("creaking_heart", 1), new Entry("copper_trapdoor", 1), new Entry("tuff_slab", 1), new Entry("cave_vines", 1),
            new Entry("sniffer_egg", 1), new Entry("copper_golem_statue", 1), new Entry("big_dripleaf_stem", 1),
    };

    private static final int[] EDGE_KEYS = {
            Scenario.FORWARD | Scenario.JUMP, Scenario.FORWARD | Scenario.JUMP, Scenario.FORWARD | Scenario.SPRINT | Scenario.JUMP,
            Scenario.FORWARD, Scenario.FORWARD | Scenario.SPRINT, Scenario.FORWARD | Scenario.LEFT | Scenario.JUMP,
            Scenario.FORWARD | Scenario.RIGHT | Scenario.JUMP, Scenario.FORWARD | Scenario.SNEAK, Scenario.BACK | Scenario.JUMP, 0,
    };

    /**
     * Edge mode (the 1.16.5 generator with the 26.x palette): strips of partial-height blocks next to each
     * other and to one-block steps, walked over with mostly forward + jump input; plus the 26.x
     * attributes that change landings and step-ups (step height, scale, jump strength), leather boots
     * (powder snow), auto-jump, jump boost and low ceilings. Case i of seed s depends only on (s, i).
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
        if (r.chance(0.2)) {
            sc.autoJump = true;
        }
        if (r.chance(0.1)) {
            sc.soulSpeed = r.range(1, 3);
        }
        if (r.chance(0.12)) {
            sc.effect(Scenarios.eff("jump_boost"), r.range(0, 2));
        }
        if (r.chance(0.2)) {
            sc.leatherBoots = true;
        }
        if (r.chance(0.12)) {
            sc.attributeBases.put("minecraft:step_height", pickD(r, new double[] {0.0, 0.5, 1.0, 1.25, 2.0}));
        }
        if (r.chance(0.12)) {
            sc.attributeBases.put("minecraft:scale", pickD(r, new double[] {0.25, 0.5, 0.75, 1.5, 2.0}));
        }
        if (r.chance(0.08)) {
            sc.attributeBases.put("minecraft:jump_strength", 0.3 + 0.5 * r.nextDouble());
        }
        if (r.chance(0.15)) {
            int cy = floorY + r.range(3, 4);
            String c = r.pick(new String[] {"stone", "smooth_stone_slab", "smooth_stone_slab[type=top]", "scaffolding", "powder_snow"});
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

    /** The client's own sprint modifier (a server echoes it in attribute packets while the player sprints). */
    private static final java.util.UUID SPRINT_UUID = java.util.UUID.fromString("662A6B8D-DA3E-4C1C-8813-96EA6097278D");

    /** The movement attributes with ranges for random bases. */
    private static final String[] ATTRS = {"minecraft:step_height", "minecraft:gravity", "minecraft:jump_strength",
            "minecraft:sneaking_speed", "minecraft:movement_efficiency", "minecraft:water_movement_efficiency", "minecraft:scale",
            "minecraft:safe_fall_distance", "minecraft:movement_speed"};

    private static double randomBase(Rng x, String attr) {
        return switch (attr) {
            case "minecraft:step_height" -> pickD(x, new double[] {0.0, 0.3, 0.6, 1.0, 1.5, 2.5});
            case "minecraft:gravity" -> x.chance(0.1) ? -0.02 : 0.01 + 0.14 * x.nextDouble();
            case "minecraft:jump_strength" -> 0.1 + 0.8 * x.nextDouble();
            case "minecraft:sneaking_speed", "minecraft:movement_efficiency", "minecraft:water_movement_efficiency" -> x.nextDouble();
            case "minecraft:scale" -> x.chance(0.1) ? pickD(x, new double[] {0.0625, 0.1, 4.0, 16.0}) : 0.3 + 2.2 * x.nextDouble();
            case "minecraft:safe_fall_distance" -> x.range(-2, 10);
            default -> 0.05 + 0.15 * x.nextDouble();
        };
    }

    private static List<Scenario.AttributeModifierSpec> randomAttrModifiers(Rng x, String attr) {
        List<Scenario.AttributeModifierSpec> mods = new ArrayList<>();
        int n = x.range(0, 3);
        for (int i = 0; i < n; i++) {
            int op = x.nextInt(3);
            double amount = (x.nextDouble() - 0.3) * (op == 0 ? (attr.endsWith("scale") || attr.endsWith("step_height") ? 1.0 : 0.1) : 0.8);
            mods.add(new Scenario.AttributeModifierSpec(attr, "oracle:m" + Long.toHexString(x.nextLong() >>> 16), amount, op));
        }
        return mods;
    }

    /** Effects the 26.1 client reads: jump boost, blindness, levitation, slow falling, dolphin's grace, weaving. */
    private static List<int[]> randomEffects(Rng x) {
        List<int[]> fx = new ArrayList<>();
        String[] names = {"jump_boost", "blindness", "levitation", "slow_falling", "dolphins_grace", "weaving"};
        int[] max = {3, 0, 2, 0, 0, 0};
        for (int i = 0; i < names.length; i++) {
            if (x.chance(0.3)) {
                fx.add(new int[] {Scenarios.eff(names[i]), x.range(0, max[i])});
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
     * The plain case plus, from a separate random stream (same terrain and input), what else the 26.1
     * client reads: speed modifiers of every operation, the other movement attributes (bases and
     * modifiers of all three operations: step height, gravity, jump strength, sneaking speed, movement /
     * water movement efficiency, scale, safe fall distance), boots enchantments up to V, leather boots,
     * a used item with its own use effects, the sprint window option, food at the sprint threshold, fly
     * speeds, client-read effects, spectator / creative / adventure, partial 26.x blocks around the
     * spawn, a bubble-column pool, the world border, and inbound packets during the case: speed and
     * other attribute packets, effect changes, abilities, teleports with all relative flags, and large
     * motion packets. Cases get an {@code _x} suffix.
     */
    static Scenario withExtras(long seed, int index, int ticks, boolean useItem) {
        Scenario base = one(seed, index, ticks, useItem);
        Scenario sc = new Scenario(base.name + "_x");
        copyInto(base, sc);
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
        if (x.chance(0.5)) {
            int n = x.range(1, 3);
            for (int i = 0; i < n; i++) {
                String attr = ATTRS[x.nextInt(ATTRS.length - 1)];
                if (x.chance(0.6)) {
                    sc.attributeBases.put(attr, randomBase(x, attr));
                }
                sc.attributeModifiers.addAll(randomAttrModifiers(x, attr));
            }
        }
        if (x.chance(0.3)) {
            sc.depthStrider = x.range(1, 5);
        }
        if (x.chance(0.2)) {
            sc.soulSpeed = x.range(1, 5);
        }
        if (x.chance(0.2)) {
            sc.leatherBoots = true;
        }
        if (useItem && x.chance(0.5)) {
            sc.useCanSprint = x.chance(0.5);
            sc.useSpeed = x.chance(0.2) ? (x.chance(0.5) ? 0.0F : 1.0F) : (float) x.nextDouble();
        }
        if (x.chance(0.3)) {
            sc.sprintWindow = x.range(0, 10);
        }
        if (x.chance(0.4)) {
            // Partial 26.x blocks and powder snow around the spawn.
            int n = x.range(2, 8);
            int bx = (int) Math.floor(sc.x), bz = (int) Math.floor(sc.z);
            for (int i = 0; i < n; i++) {
                String b = x.chance(0.3) ? "powder_snow" : pick(x, PARTIAL);
                int px = bx + x.range(-3, 3), pz = bz + x.range(-3, 3), py = 63 + x.range(0, 2);
                if (x.chance(0.5)) {
                    sc.fill(px, py, pz, px + x.range(0, 2), py + x.range(0, 1), pz + x.range(0, 2), randomState(x, b), 0);
                } else {
                    sc.block(px, py, pz, randomState(x, b), 0);
                }
            }
        }
        if (x.chance(0.08)) {
            int bx = x.chance(0.5) ? 29_999_984 : -29_999_984, bz = x.chance(0.5) ? 29_999_984 : -29_999_984;
            boolean both = x.chance(0.3), alongX = x.chance(0.5);
            int inX = x.chance(0.7) ? -x.range(1, 4) : x.range(0, 3), inZ = x.chance(0.7) ? -x.range(1, 4) : x.range(0, 3);
            int dx = both || alongX ? bx + (bx > 0 ? inX : -inX - 1) - (int) Math.floor(sc.x) : 0;
            int dz = both || !alongX ? bz + (bz > 0 ? inZ : -inZ - 1) - (int) Math.floor(sc.z) : 0;
            if (x.chance(0.6)) {
                float face = alongX && !both ? (bx > 0 ? -90 : 90) : (bz > 0 ? 0 : 180);
                float turn = face + (float) ((x.nextDouble() - 0.5) * 40) - sc.yaw;
                sc.yaw += turn;
                for (int i = 0; i < sc.ticks.size(); i++) {
                    Scenario.Tick tk = sc.ticks.get(i);
                    sc.ticks.set(i, new Scenario.Tick(tk.keys(), tk.yaw() + turn, tk.pitch(), tk.events()));
                }
            }
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
        double mode = x.nextDouble();
        if (mode < 0.12) {
            sc.mode(Scenario.GameMode.SPECTATOR);
            sc.flying(true);
        } else if (mode < 0.17) {
            sc.mode(Scenario.GameMode.ADVENTURE);
        }
        if (x.chance(0.15)) {
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
            int events = x.chance(0.6) ? x.range(1, 5) : 0;
            for (int k = 0; k < events; k++) {
                int at = x.nextInt(n);
                switch (x.nextInt(6)) {
                    case 0 -> addEvent(sc, at, new Scenario.SpeedAttribute(x.chance(0.7) ? 0.1 : 0.05 + 0.15 * x.nextDouble(),
                            randomModifiers(x, x.chance(0.5))));
                    case 1 -> addEvent(sc, at, new Scenario.Effects(randomEffects(x)));
                    case 2 -> {
                        boolean allow = creative || spectator || x.chance(0.5);
                        boolean flying = allow && x.chance(0.5);
                        float fly = x.chance(0.6) ? 0.05f : (float) (0.01 + 0.2 * x.nextDouble());
                        addEvent(sc, at, new Scenario.Abilities(creative || spectator, flying, allow, creative, fly, 0.1f));
                    }
                    case 3 -> {
                        String attr = ATTRS[x.nextInt(ATTRS.length - 1)];
                        addEvent(sc, at, new Scenario.Attribute(attr, randomBase(x, attr), randomAttrModifiers(x, attr)));
                    }
                    case 4 -> addEvent(sc, at, new Scenario.Velocity(x.range(-24000, 24000), x.range(-24000, 24000),
                            x.range(-24000, 24000)));
                    default -> {
                        // All nine relative flags: x, y, z, yaw, pitch, delta x / y / z, rotate delta.
                        int flags = x.chance(0.3) ? 0 : x.nextInt(512);
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

    /**
     * The plain case plus, from a separate random stream, position packets that carry a non-zero
     * delta movement under every combination of the nine relative bits (x, y, z, yaw, pitch, delta
     * x / y / z, rotate delta), and entity-motion packets right before or after them (same tick, in
     * either packet order, or the neighbouring ticks). Cases get a {@code _tp} suffix.
     */
    static Scenario withTeleports(long seed, int index, int ticks) {
        Scenario base = one(seed, index, ticks, false);
        Scenario sc = new Scenario(base.name + "_tp");
        copyInto(base, sc);
        Rng x = new Rng(seed * 0x2545F4914F6CDD1DL + index ^ 0x74656c65L);
        int n = x.range(1, 5);
        for (int i = 0; i < n && sc.ticks.size() > 2; i++) {
            int at = x.range(1, sc.ticks.size() - 2);
            int flags = x.nextInt(512);
            if (x.chance(0.4)) {
                flags |= 0x1E0 & x.nextInt(512); // more delta / rotate-delta bits
            }
            double[] d = new double[3];
            for (int a = 0; a < 3; a++) {
                double r = x.nextDouble();
                d[a] = r < 0.15 ? 0.0 : r < 0.6 ? (x.nextDouble() - 0.5) * 0.6 : r < 0.9 ? (x.nextDouble() - 0.5) * 4 : (x.nextDouble() - 0.5) * 40;
            }
            double px = (flags & 1) != 0 ? (x.nextDouble() - 0.5) * 2 : sc.x + (x.nextDouble() - 0.5) * 4;
            double py = (flags & 2) != 0 ? (x.nextDouble() - 0.3) * 2 : sc.y + x.nextDouble() * 2;
            double pz = (flags & 4) != 0 ? (x.nextDouble() - 0.5) * 2 : sc.z + (x.nextDouble() - 0.5) * 4;
            float yaw = (flags & 8) != 0 ? (float) ((x.nextDouble() - 0.5) * 360) : (float) (x.nextDouble() * 720 - 360);
            float pitch = (flags & 16) != 0 ? (float) ((x.nextDouble() - 0.5) * 120) : (float) (x.nextDouble() * 180 - 90);
            Scenario.Event tp = new Scenario.Teleport(px, py, pz, yaw, pitch, flags, d[0], d[1], d[2]);
            Scenario.Event vel = new Scenario.Velocity(x.range(-30000, 30000), x.range(-30000, 30000), x.range(-30000, 30000));
            double r = x.nextDouble();
            if (r < 0.2) {
                addEvent(sc, at, vel);
                addEvent(sc, at, tp);
            } else if (r < 0.4) {
                addEvent(sc, at, tp);
                addEvent(sc, at, vel);
            } else if (r < 0.5) {
                addEvent(sc, at - 1, vel);
                addEvent(sc, at, tp);
            } else if (r < 0.6) {
                addEvent(sc, at, tp);
                addEvent(sc, at + 1, vel);
            } else {
                addEvent(sc, at, tp);
            }
        }
        sc.description += ", teleports with delta movement";
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
        sc.attributeBases.putAll(src.attributeBases);
        sc.attributeModifiers.addAll(src.attributeModifiers);
        sc.depthStrider = src.depthStrider;
        sc.soulSpeed = src.soulSpeed;
        sc.autoJump = src.autoJump;
        sc.bb = src.bb;
        sc.pose = src.pose;
        sc.ticks.addAll(src.ticks);
    }
}
