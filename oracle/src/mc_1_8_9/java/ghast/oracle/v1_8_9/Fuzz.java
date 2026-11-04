package ghast.oracle.v1_8_9;

import ghast.oracle.Rng;
import ghast.oracle.Scenario;

import java.util.ArrayList;
import java.util.List;

import static ghast.oracle.v1_8_9.Blocks.*;

/**
 * Seeded random scenarios: a small random terrain of movement-relevant blocks and random held-key
 * sequences with turning. Case {@code i} of seed {@code s} depends only on (s, i).
 */
final class Fuzz {
    private Fuzz() {}

    /** Block palette: id, metas to pick from, weight. */
    private record Entry(int id, int[] metas, int weight) {}

    private static int[] range(int lo, int hi) {
        int[] r = new int[hi - lo + 1];
        for (int i = 0; i < r.length; i++) {
            r[i] = lo + i;
        }
        return r;
    }

    private static final int[] M0 = {0};

    /** Floor materials (top surface the player walks on). */
    private static final Entry[] FLOOR = {
            new Entry(STONE, M0, 30), new Entry(GRASS, M0, 4), new Entry(ICE, M0, 5), new Entry(PACKED_ICE, M0, 4),
            new Entry(SLIME, M0, 4), new Entry(SOUL_SAND, M0, 5), new Entry(FARMLAND, range(0, 7), 3),
            new Entry(DOUBLE_STONE_SLAB, M0, 2), new Entry(GLASS, M0, 2), new Entry(STAINED_GLASS, range(0, 15), 1),
            new Entry(WATER, M0, 2), new Entry(LAVA, M0, 1), new Entry(AIR, M0, 3), new Entry(GLOWSTONE, M0, 1),
            new Entry(LEAVES, range(0, 3), 1), new Entry(SAND, M0, 1),
    };

    /**
     * Obstacles placed on or above the floor. Anvils and chests are left out: their collision box is
     * history-dependent in vanilla (README, history-dependent shapes) and the harness does not settle
     * bounds before each tick.
     */
    private static final Entry[] FEATURE = {
            new Entry(STONE, M0, 12), new Entry(STONE_SLAB, range(0, 15), 8), new Entry(WOODEN_SLAB, range(0, 13), 3),
            new Entry(OAK_STAIRS, range(0, 7), 6), new Entry(STONE_BRICK_STAIRS, range(0, 7), 2),
            new Entry(FENCE, M0, 5), new Entry(NETHER_BRICK_FENCE, M0, 2), new Entry(SPRUCE_FENCE, M0, 1),
            new Entry(FENCE_GATE, range(0, 7), 3), new Entry(COBBLESTONE_WALL, range(0, 1), 3),
            new Entry(GLASS_PANE, M0, 4), new Entry(STAINED_GLASS_PANE, range(0, 15), 1), new Entry(IRON_BARS, M0, 2),
            new Entry(SNOW_LAYER, range(0, 7), 4), new Entry(CARPET, range(0, 15), 2), new Entry(WEB, M0, 3),
            new Entry(LADDER, range(2, 5), 4), new Entry(VINE, range(0, 15), 3), new Entry(SOUL_SAND, M0, 2),
            new Entry(SLIME, M0, 2), new Entry(ICE, M0, 1), new Entry(CACTUS, range(0, 15), 1),
            new Entry(CAKE, range(0, 6), 1), new Entry(ENCHANTING_TABLE, M0, 1), new Entry(DAYLIGHT_DETECTOR, range(0, 15), 1),
            new Entry(TRAPDOOR, range(0, 15), 3), new Entry(IRON_TRAPDOOR, range(0, 15), 1),
            new Entry(HOPPER, range(0, 5), 1), new Entry(END_PORTAL_FRAME, range(0, 7), 1),
            new Entry(BREWING_STAND, range(0, 7), 1), new Entry(CAULDRON, range(0, 3), 1), new Entry(FLOWER_POT, M0, 1),
            new Entry(BED, range(0, 15), 1), new Entry(LILY_PAD, M0, 1), new Entry(WATER, range(0, 7), 2),
            new Entry(FLOWING_WATER, range(0, 7), 1), new Entry(FARMLAND, M0, 1), new Entry(GLOWSTONE, M0, 1),
            new Entry(SKULL, range(1, 5), 1), new Entry(PISTON, range(0, 13), 1), new Entry(PISTON_HEAD, range(0, 13), 1),
            new Entry(AIR, M0, 3),
    };

    private static Entry pick(Rng r, Entry[] table) {
        int total = 0;
        for (Entry e : table) {
            total += e.weight;
        }
        int x = r.nextInt(total);
        for (Entry e : table) {
            x -= e.weight;
            if (x < 0) {
                return e;
            }
        }
        throw new AssertionError();
    }

    /** Origins: small, medium and very large coordinates (precision behaves differently). */
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
        List<Scenario> out = new ArrayList<>(count);
        for (int i = 0; i < count; i++) {
            out.add(one(seed, i, ticks));
        }
        return out;
    }

    /**
     * Like {@link #one(long, int, int)}, optionally with the use key (sword blocking) held for random
     * spans. The use spans come from a separate stream, so the rest of the case is identical.
     */
    static Scenario one(long seed, int index, int ticks, boolean useItem) {
        Scenario sc = one(seed, index, ticks);
        if (useItem) {
            Rng u = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x7573655f6974656dL);
            boolean held = u.chance(0.5);
            for (int i = 0; i < sc.ticks.size(); i++) {
                if (u.chance(0.12)) {
                    held = !held;
                }
                if (held) {
                    Scenario.Tick t = sc.ticks.get(i);
                    sc.ticks.set(i, new Scenario.Tick(t.keys() | Scenario.USE, t.yaw(), t.pitch(), t.events()));
                }
            }
            sc = renamed(sc, sc.name + "_use");
        }
        return sc;
    }

    private static Scenario renamed(Scenario src, String name) {
        Scenario sc = new Scenario(name);
        sc.description = src.description + ", use key";
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
        return sc;
    }

    /** Partial-height surfaces for the edge mode: id, metas, weight. */
    private static final Entry[] PARTIAL = {
            new Entry(SOUL_SAND, M0, 6), new Entry(STONE_SLAB, range(0, 7), 6), new Entry(WOODEN_SLAB, range(0, 5), 2),
            new Entry(SNOW_LAYER, range(0, 7), 6), new Entry(FARMLAND, range(0, 7), 4), new Entry(CARPET, range(0, 15), 3),
            new Entry(CAKE, range(0, 6), 1), new Entry(DAYLIGHT_DETECTOR, M0, 1), new Entry(ENCHANTING_TABLE, M0, 1),
            new Entry(OAK_STAIRS, range(0, 7), 2), new Entry(TRAPDOOR, range(0, 7), 2), new Entry(END_PORTAL_FRAME, range(0, 3), 1),
            new Entry(BED, range(0, 15), 1), new Entry(STONE, M0, 4), new Entry(GRASS, M0, 3), new Entry(SLIME, M0, 1),
    };

    private static final int[] EDGE_KEYS = {
            Scenario.FORWARD | Scenario.JUMP, Scenario.FORWARD | Scenario.JUMP, Scenario.FORWARD | Scenario.SPRINT | Scenario.JUMP,
            Scenario.FORWARD, Scenario.FORWARD | Scenario.SPRINT, Scenario.FORWARD | Scenario.LEFT | Scenario.JUMP,
            Scenario.FORWARD | Scenario.RIGHT | Scenario.JUMP, Scenario.FORWARD | Scenario.SNEAK, Scenario.BACK | Scenario.JUMP, 0,
    };

    /**
     * Edge mode: a floor of partial-height blocks (soul sand, slabs, snow layers, farmland, carpets, ...)
     * in strips next to each other and next to full blocks and one-block steps, walked over with mostly
     * forward + jump input, so landings and step-ups at height differences are frequent.
     */
    static Scenario edge(long seed, int index, int ticks) {
        Rng r = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x65646765L);
        r.nextLong();
        Scenario sc = new Scenario("edge_s" + seed + "_" + index);
        int[] origin = ORIGINS[r.nextInt(ORIGINS.length)];
        int ox = origin[0] + r.range(-20, 20), oz = origin[1] + r.range(-20, 20);
        int floorY = 63;
        sc.desc("edge terrain at " + ox + "," + oz);
        sc.fill(ox - 16, floorY - 1, oz - 16, ox + 16, floorY - 1, oz + 16, STONE);
        sc.fill(ox - 16, floorY, oz - 16, ox + 16, floorY, oz + 16, GRASS);
        // Strips across the walking direction (+z mostly), each one surface type.
        boolean alongX = r.chance(0.3);
        int pos = -14;
        while (pos < 14) {
            int w = r.range(1, 4);
            Entry e = pick(r, PARTIAL);
            int meta = r.pick(e.metas);
            if (alongX) {
                sc.fill(ox + pos, floorY, oz - 14, ox + pos + w - 1, floorY, oz + 14, e.id, meta);
            } else {
                sc.fill(ox - 14, floorY, oz + pos, ox + 14, floorY, oz + pos + w - 1, e.id, meta);
            }
            if (r.chance(0.25)) {
                // A raised step on top of the strip: partial or full block.
                Entry up = pick(r, PARTIAL);
                int um = r.pick(up.metas);
                if (alongX) {
                    sc.fill(ox + pos, floorY + 1, oz - 14, ox + pos + w - 1, floorY + 1, oz + 14, up.id, um);
                } else {
                    sc.fill(ox - 14, floorY + 1, oz + pos, ox + 14, floorY + 1, oz + pos + w - 1, up.id, um);
                }
            }
            pos += w;
        }
        // Start before the first strip, walking across the strips (+z, or +x when they run along z).
        double across = 0.05 + 0.9 * r.nextDouble() - 12;
        double side = 0.05 + 0.9 * r.nextDouble() + r.range(-3, 3);
        double sx = alongX ? ox + across : ox + side;
        double sz = alongX ? oz + side : oz + across;
        float yaw = alongX ? -90 + (float) ((r.nextDouble() - 0.5) * 50) : (float) ((r.nextDouble() - 0.5) * 50);
        sc.spawn(sx, floorY + 3, sz, yaw, (float) (r.nextDouble() * 30));
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

    /**
     * Like {@link #one(long, int, int, boolean)}, plus (from a separate stream, so the rest of the case
     * is identical) server speed modifiers, depth strider boots and spectator mode now and then.
     */
    static Scenario withExtras(long seed, int index, int ticks, boolean useItem) {
        Scenario sc = one(seed, index, ticks, useItem);
        Rng x = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x6578747261L);
        if (x.chance(0.5)) {
            int n = x.range(1, 3);
            for (int i = 0; i < n; i++) {
                int op = x.nextInt(3);
                double amount = (x.nextDouble() - 0.3) * (op == 0 ? 0.1 : 0.8);
                sc.speedModifiers.add(new double[] {amount, op});
            }
        }
        if (x.chance(0.3)) {
            sc.depthStrider = x.range(1, 3);
        }
        if (x.chance(0.12)) {
            sc.mode(Scenario.GameMode.SPECTATOR);
            sc.flying(true);
        }
        sc.description += ", extras";
        return renamed(sc, sc.name + "_x");
    }

    static Scenario one(long seed, int index, int ticks) {
        Rng r = new Rng(seed * 0x9E3779B97F4A7C15L + index);
        r.nextLong();
        Scenario sc = new Scenario("fuzz_s" + seed + "_" + index);
        int[] origin = ORIGINS[r.nextInt(ORIGINS.length)];
        int ox = origin[0] + r.range(-20, 20), oz = origin[1] + r.range(-20, 20);
        int size = r.range(4, 9);
        int floorY = 63;
        sc.desc("random terrain " + (2 * size + 1) + "x" + (2 * size + 1) + " at " + ox + "," + oz);

        // Floor: a wide stone base (so cases rarely fall into the void) with patches of other materials.
        int apron = size + 10;
        sc.fill(ox - apron, floorY - 1, oz - apron, ox + apron, floorY, oz + apron, STONE);
        int patches = r.range(1, 8);
        for (int p = 0; p < patches; p++) {
            Entry e = pick(r, FLOOR);
            int x0 = ox + r.range(-size, size), z0 = oz + r.range(-size, size);
            sc.fill(x0, floorY, z0, x0 + r.range(0, 4), floorY, z0 + r.range(0, 4), e.id, r.pick(e.metas));
        }
        // Features: single blocks, short walls and pillars, occasional ceilings.
        int features = r.range(5, 40);
        for (int f = 0; f < features; f++) {
            Entry e = pick(r, FEATURE);
            int meta = r.pick(e.metas);
            int x = ox + r.range(-size, size), z = oz + r.range(-size, size), y = floorY + 1 + r.nextInt(3);
            switch (r.nextInt(6)) {
                case 0 -> sc.fill(x, y, z, x + r.range(0, 4), y, z, e.id, meta);
                case 1 -> sc.fill(x, y, z, x, y, z + r.range(0, 4), e.id, meta);
                case 2 -> sc.fill(x, floorY + 1, z, x, floorY + r.range(1, 5), z, e.id, meta);
                default -> sc.block(x, y, z, e.id, meta);
            }
        }
        // Climbable wall near the spawn.
        if (r.chance(0.4)) {
            int wx = ox + r.range(-2, 2), wz = oz + r.range(-2, 2), h = r.range(2, 6);
            int dir = r.nextInt(4);
            int[][] off = {{0, 1}, {0, -1}, {1, 0}, {-1, 0}}; // wall offset from the ladder: south, north, east, west
            int[] ladderMeta = {2, 3, 4, 5};                   // ladder facing away from that wall
            int[] vineMeta = {1, 4, 8, 2};                     // vine attached to that wall face
            sc.fill(wx + off[dir][0], floorY + 1, wz + off[dir][1], wx + off[dir][0], floorY + h + 1, wz + off[dir][1], STONE);
            boolean vine = r.chance(0.35);
            sc.fill(wx, floorY + 1, wz, wx, floorY + h, wz, vine ? VINE : LADDER, vine ? vineMeta[dir] : ladderMeta[dir]);
        }
        if (r.chance(0.2)) {
            int cy = floorY + r.range(2, 4);
            sc.fill(ox - r.range(0, size), cy, oz - r.range(0, size), ox + r.range(0, size), cy, oz + r.range(0, size),
                    r.chance(0.5) ? STONE : STONE_SLAB, r.chance(0.5) ? 0 : 8);
        }
        // Keep the spawn column clear so the case starts free.
        double sx = ox + r.range(-2, 2) + 0.05 + 0.9 * r.nextDouble();
        double sz = oz + r.range(-2, 2) + 0.05 + 0.9 * r.nextDouble();
        int bx = (int) Math.floor(sx), bz = (int) Math.floor(sz);
        double sy = floorY + 1 + (r.chance(0.6) ? 0 : r.nextInt(6) + r.nextDouble());
        sc.fill(bx - 1, floorY + 1, bz - 1, bx + 1, floorY + 3 + (int) (sy - floorY), bz + 1, AIR);
        if (r.chance(0.7)) {
            sc.block(bx, floorY, bz, STONE);
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
            sc.movementSpeedBase = 0.1 * (1 + 0.2 * r.range(1, 3));
        }
        if (r.chance(0.08)) {
            sc.effect(8, r.range(0, 2));
        }

        // Inputs: held key combos for random spans, with smooth turns and occasional events.
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
        return sc;
    }
}
