package ghast.oracle.v26_1;

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
}
