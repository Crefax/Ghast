package ghast.oracle.v1_16_5;

import ghast.oracle.Rng;
import ghast.oracle.Scenario;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

import static ghast.oracle.Scenario.FORWARD;
import static ghast.oracle.Scenario.JUMP;
import static ghast.oracle.Scenario.LEFT;
import static ghast.oracle.Scenario.SNEAK;
import static ghast.oracle.Scenario.SPRINT;

/**
 * 1.16.5 cases with other entities ({@link Ents}): hand-written scenarios and the randomized
 * entities fuzz. Golden file {@code entities.jsonl.gz}: the scenarios plus {@link #GOLDEN_FUZZ}
 * fuzz cases of seed 1.
 */
final class EntCases {
    private EntCases() {}

    static final int GOLDEN_FUZZ = 50;

    private static int s(String spec) {
        return States.id(spec);
    }

    private static Scenario flat(String name) {
        return new Scenario(name).fill(-16, 63, -16, 16, 63, 16, s("stone"), 0);
    }

    private static Scenario with(Scenario sc, Ents.Spawn... spawns) {
        Ents.setup(sc).entities.addAll(List.of(spawns));
        return sc;
    }

    private static Ents.Spawn sp(int id, String type, double x, double y, double z) {
        return new Ents.Spawn(id, type, x, y, z, 0.0F, 0);
    }

    private static Ents.Spawn player(int id, double x, double y, double z, int gameMode) {
        return new Ents.Spawn(id, "player", x, y, z, 0.0F, gameMode);
    }

    private static String uuid(int id) {
        return Ents.uuidOf(id).toString();
    }

    private static Ents.TeamOp team(String name, int mode, String rule, String... members) {
        return new Ents.TeamOp(name, mode, rule, List.of(members));
    }

    private static void addEvent(Scenario sc, int tick, Scenario.Event e) {
        Scenario.Tick tk = sc.ticks.get(tick);
        List<Scenario.Event> events = new ArrayList<>(tk.events());
        events.add(e);
        sc.ticks.set(tick, new Scenario.Tick(tk.keys(), tk.yaw(), tk.pitch(), List.copyOf(events)));
    }

    static List<Scenario> golden() {
        List<Scenario> l = new ArrayList<>(scenarios());
        for (int i = 0; i < GOLDEN_FUZZ; i++) {
            l.add(fuzz(1, i, 60));
        }
        return l;
    }

    static List<Scenario> scenarios() {
        Harness.init();
        List<Scenario> l = new ArrayList<>();
        String me = Harness.PLAYER_NAME;
        l.add(with(flat("push_zombie"), sp(100, "zombie", 0.8, 64, 0.7)).desc("a zombie overlapping the player pushes it away each tick")
                .spawn(0.5, 64, 0.5, 0, 0).run(30, 0).run(20, FORWARD));
        l.add(with(flat("push_crowd"), sp(100, "villager", 0.2, 64, 0.5), sp(101, "cow", 0.9, 64, 0.2), sp(102, "pig", 0.5, 64, 1.0),
                sp(103, "sheep", 0.45, 64, 0.55)).desc("four mobs around the player: pushes add up in the client's tick order")
                .spawn(0.5, 64, 0.5, 0, 0).run(40, 0));
        l.add(with(flat("push_walk_into_mobs"), sp(100, "cow", 0.5, 64, 3.0), sp(101, "cow", 0.9, 64, 4.2), sp(102, "iron_golem", 0.4, 64, 6))
                .desc("walk and sprint into cows and an iron golem").spawn(0.5, 64, 0.5, 0, 0).run(5, 0).run(40, FORWARD | SPRINT));
        l.add(with(flat("no_push_armor_stand_bat_parrot_items"), sp(100, "armor_stand", 0.6, 64, 0.6), sp(101, "bat", 0.4, 64.5, 0.4),
                sp(102, "parrot", 0.5, 64.2, 0.7), sp(103, "item", 0.55, 64, 0.45), sp(104, "minecart", 0.7, 64, 0.3), sp(105, "vex", 0.3, 64, 0.6))
                .desc("armor stands, bats, parrots, items, minecarts and vexes do not push players").spawn(0.5, 64, 0.5, 0, 0).run(30, 0));
        l.add(with(flat("push_big_mobs"), sp(100, "slime", 0.9, 64, 0.5), sp(101, "horse", 1.4, 64, 1.2), sp(102, "spider", -0.4, 64, 0.1),
                sp(103, "ravager", 0.5, 64, -1.1), sp(104, "magma_cube", 1.0, 64, -0.2))
                .desc("slimes, a horse, a spider, a ravager and a magma cube around the player").spawn(0.5, 64, 0.5, 0, 0).run(30, 0).run(15, LEFT));
        Scenario other = with(flat("push_other_player"), player(100, 0.8, 64, 0.6, 0), player(101, 0.2, 64, 0.3, 1))
                .desc("other players push (their client tick), not while in spectator mode").spawn(0.5, 64, 0.5, 0, 0).run(40, 0);
        addEvent(other, 10, new Ents.GameMode(100, 3));
        addEvent(other, 20, new Ents.GameMode(100, 0));
        addEvent(other, 25, new Ents.GameMode(101, 3));
        l.add(other);
        l.add(with(flat("push_spectator"), sp(100, "zombie", 0.8, 64, 0.7)).desc("spectators are not pushed")
                .spawn(0.5, 64, 0.5, 0, 0).mode(Scenario.GameMode.SPECTATOR).run(20, 0));
        l.add(with(flat("push_ladder").fill(0, 64, 1, 0, 70, 1, s("stone"), 0).fill(0, 64, 0, 0, 70, 0, s("ladder[facing=north]"), 0),
                sp(100, "zombie", 0.8, 64.5, 0.5)).desc("on a ladder the player is not pushable").spawn(0.5, 64, 0.5, 0, 0).run(3, 0)
                .run(20, FORWARD | JUMP).run(10, 0));
        l.add(with(flat("push_sneaking"), sp(100, "zombie", 0.75, 64, 0.6), sp(101, "cow", 0.3, 64, 0.2))
                .desc("sneaking: still pushed (crouching box)").spawn(0.5, 64, 0.5, 0, 0).run(20, SNEAK).run(15, SNEAK | FORWARD));
        l.add(with(new Scenario("push_in_water").fill(-6, 60, -6, 6, 60, 6, s("stone"), 0).fill(-6, 61, -6, 6, 63, 6, s("water"), 0),
                sp(100, "dolphin", 0.7, 62, 0.6), sp(101, "drowned", 0.3, 61, 0.4), sp(102, "cod", 0.5, 61.5, 0.9))
                .desc("in water among a dolphin, a drowned and a cod").spawn(0.5, 61, 0.5, 0, 0).run(30, 0).run(20, FORWARD));
        l.add(with(flat("boat_collide"), sp(100, "boat", 0.5, 64, 3.0)).desc("walk into a boat, jump on it, stand, walk off; sneak at its edge")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(12, FORWARD).run(1, FORWARD | JUMP).run(10, FORWARD).run(10, 0)
                .run(20, FORWARD | SNEAK).run(10, FORWARD));
        l.add(with(flat("boat_push"), sp(100, "boat", 0.5, 64, 1.55), sp(101, "boat", 1.8, 64, 0.5), sp(102, "boat", -0.9, 63.8, 0.5),
                sp(103, "boat", 0.4, 64.3, -0.9)).desc("boats next to the player push it (their box grown by 0.2), not one higher than its feet")
                .spawn(0.5, 64, 0.5, 0, 0).run(40, 0));
        Scenario boatStand = with(flat("boat_stand_on"), sp(100, "boat", 0.5, 64, 0.5)).desc("stand on a boat, the boat moves away (10-step interpolation)")
                .spawn(0.5, 64.5625, 0.5, 0, 0).run(30, 0);
        for (int i = 0; i < 10; i++) {
            addEvent(boatStand, 10 + i, new Ents.Move(100, 0.5 + i * 0.12, 64, 0.5, 0.0F, true));
        }
        l.add(boatStand);
        l.add(with(flat("shulker_block").fill(-3, 63, 2, 3, 63, 6, s("stone"), 0), sp(100, "shulker", 0.5, 64, 3.5), sp(101, "shulker", 0.5, 65, 4.5))
                .desc("shulkers collide like blocks: walk into one, jump up two").spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(10, FORWARD)
                .run(1, FORWARD | JUMP).run(12, FORWARD).run(1, FORWARD | JUMP).run(15, FORWARD));
        Scenario moving = with(flat("push_moving_zombie"), sp(100, "zombie", 3.0, 64, 0.5)).desc("a zombie walks through the standing player (teleport packets, interpolated)")
                .spawn(0.5, 64, 0.5, 0, 0).run(40, 0);
        for (int i = 0; i < 20; i++) {
            addEvent(moving, 2 + i, new Ents.Move(100, 3.0 - i * 0.25, 64, 0.5 + (i % 3) * 0.05, 90.0F, true));
        }
        l.add(moving);
        Scenario rel = with(flat("push_relative_moves"), sp(100, "pig", 2.0, 64, 0.6), sp(101, "boat", -1.5, 64, 0.5))
                .desc("relative moves and look-only packets (a look stops the interpolation where it is)").spawn(0.5, 64, 0.5, 0, 0).run(45, 0);
        for (int i = 0; i < 12; i++) {
            addEvent(rel, 2 + i * 2, new Ents.Rel(100, (short) -1100, (short) 0, (short) (i % 2 == 0 ? 37 : -21), true));
            addEvent(rel, 3 + i * 2, new Ents.Rel(101, (short) 700, (short) 0, (short) 0, true));
        }
        addEvent(rel, 9, new Ents.Look(100, 45.0F));
        addEvent(rel, 16, new Ents.Look(101, 90.0F));
        l.add(rel);
        Scenario spawnLate = with(flat("push_spawned_in")).desc("mobs spawned and removed mid-case, an id spawned twice").spawn(0.5, 64, 0.5, 0, 0).run(40, 0);
        addEvent(spawnLate, 5, new Ents.Spawn(200, "zombie", 0.6, 64, 0.6, 45.0F, 0));
        addEvent(spawnLate, 12, new Ents.Spawn(201, "slime", 0.3, 64, 0.3, 0.0F, 0));
        addEvent(spawnLate, 16, new Ents.Spawn(200, "cow", 0.7, 64, 0.2, 0.0F, 0));
        addEvent(spawnLate, 20, new Ents.Remove(200));
        addEvent(spawnLate, 30, new Ents.Remove(201));
        l.add(spawnLate);
        l.add(with(flat("push_unloaded_chunk").outside(Scenario.Outside.UNLOADED), sp(100, "zombie", -0.2, 64, 0.5), sp(101, "cow", 0.6, 64, 0.8))
                .desc("a mob in an unloaded chunk does not tick (no push), one in a loaded chunk does").spawn(0.1, 64, 0.5, 0, 0).run(25, 0));
        Scenario tp = with(flat("push_teleported_in"), sp(100, "zombie", 0.8, 64, 0.7), sp(101, "cow", 0.2, 64, 0.3),
                sp(102, "zombie", 12.8, 64, 12.7), sp(103, "cow", 12.2, 64, 12.3), sp(104, "boat", 13.4, 64, 12.5))
                .desc("teleported into mobs: those ticking before the player look for it where its last tick left it")
                .spawn(-14.5, 64, -14.5, 0, 0).run(30, 0);
        addEvent(tp, 3, new Scenario.Teleport(12.5, 64, 12.5, 0, 0, 0));
        addEvent(tp, 12, new Scenario.Teleport(0.5, 64, 0.5, 0, 0, 0));
        addEvent(tp, 20, new Scenario.Teleport(12.4, 64, 12.6, 0, 0, 0));
        l.add(tp);
        Scenario aj = with(flat("boat_auto_jump"), sp(100, "boat", 0.5, 64, 2.5), sp(101, "shulker", 3.5, 64, 2.5))
                .desc("auto-jump onto a boat and a shulker (entity boxes after the blocks)").spawn(0.5, 64, 0.5, 0, 0);
        aj.autoJump = true;
        l.add(aj.run(3, 0).run(25, FORWARD).run(5, 0).turn(10, 0, -9.0F, 0).run(25, FORWARD));
        // Teams: the pusher's and the player's collision rules.
        Scenario never = with(flat("team_never"), sp(100, "zombie", 0.8, 64, 0.7), sp(101, "cow", 0.2, 64, 0.3))
                .desc("a team with collision rule never: its members do not push; then the player joins it, then it is removed")
                .spawn(0.5, 64, 0.5, 0, 0).run(40, 0);
        Ents.setup(never).teams.add(team("nopush", 0, "never", uuid(100)));
        addEvent(never, 12, team("nopush", 3, null, me));
        addEvent(never, 24, team("nopush", 1, null));
        l.add(never);
        Scenario own = with(flat("team_push_own_team"), sp(100, "zombie", 0.8, 64, 0.7), sp(101, "pig", 0.2, 64, 0.3), player(102, 0.5, 64, 0.9, 0))
                .desc("pushOwnTeam: the same team does not push; the rule changes to pushOtherTeams and always")
                .spawn(0.5, 64, 0.5, 0, 0).run(45, 0);
        Ents.setup(own).teams.add(team("red", 0, "pushOwnTeam", me, uuid(100), Ents.playerNameOf(102)));
        Ents.setup(own).teams.add(team("blue", 0, "always", uuid(101)));
        addEvent(own, 15, team("red", 2, "pushOtherTeams"));
        addEvent(own, 25, team("red", 4, null, uuid(100)));
        addEvent(own, 35, team("red", 2, "always"));
        l.add(own);
        Scenario others = with(flat("team_push_other_teams"), sp(100, "zombie", 0.8, 64, 0.7), sp(101, "sheep", 0.2, 64, 0.3))
                .desc("pushOtherTeams on the player's team: only members of the same team push").spawn(0.5, 64, 0.5, 0, 0).run(40, 0);
        Ents.setup(others).teams.add(team("me", 0, "pushOtherTeams", me, uuid(100)));
        addEvent(others, 20, team("other", 0, "never", uuid(100)));
        addEvent(others, 30, team("me", 4, null, me));
        l.add(others);
        // The client's entity map: many entities (the table grows and shrinks), the id 0.
        Scenario many = flat("tick_order_many").desc("forty far armor stands, an id 0 mob and close mobs; the map grows, then shrinks")
                .spawn(0.5, 64, 0.5, 0, 0).run(50, 0);
        with(many, sp(0, "zombie", 0.85, 64, 0.5), sp(77, "cow", 0.2, 64, 0.6), sp(33, "pig", 0.5, 64, 0.15));
        for (int k = 0; k < 40; k++) {
            Ents.setup(many).entities.add(sp(500 + k * 37, "armor_stand", -12 + (k % 8) * 3, 64, -12 + (k / 8) * 3));
        }
        for (int k = 0; k < 34; k++) {
            addEvent(many, 10 + k, new Ents.Remove(500 + k * 37));
        }
        addEvent(many, 20, new Ents.Spawn(2, "sheep", 0.4, 64, 0.8, 0.0F, 0));
        l.add(many);
        return l;
    }

    // ---- fuzz --------------------------------------------------------------------------------------

    private static final String[] POOL = {"zombie", "villager", "cow", "pig", "sheep", "chicken", "armor_stand", "bat", "parrot",
            "boat", "boat", "shulker", "iron_golem", "horse", "slime", "spider", "wolf", "minecart", "skeleton", "creeper", "llama",
            "item", "player", "player", "dolphin", "vex", "magma_cube", "ravager", "rabbit", "fox"};

    private static final String[] RULES = {"always", "never", "pushOtherTeams", "pushOwnTeam"};

    /**
     * Randomized entity cases: the {@code withExtras} case with 1-6 entities of a mixed pool (mobs,
     * armor stands, bats, parrots, boats, shulkers, minecarts, items, other players, ...) within two
     * blocks of the spawn, random ids (the client's map order), moved by teleport, relative and look
     * packets, spawned and removed during the case, teams with random collision rules, other players
     * switching game mode. Cases get an {@code _en} suffix.
     */
    static Scenario fuzz(long seed, int index, int ticks) {
        Scenario base = Fuzz.withExtras(seed, index, ticks, false);
        Scenario sc = new Scenario(base.name + "_en");
        Fuzz.copyInto(base, sc);
        Rng x = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x656e7431L);
        Ents.Setup st = Ents.setup(sc);
        Set<Integer> used = new HashSet<>();
        used.add(Harness.PLAYER_ENTITY_ID);
        int n = x.range(1, 6);
        List<double[]> live = new ArrayList<>();
        List<String> names = new ArrayList<>();
        names.add(Harness.PLAYER_NAME);
        for (int i = 0; i < n; i++) {
            int id = freshId(x, used);
            String type = x.pick(POOL);
            double ex = sc.x + (x.nextDouble() - 0.5) * 4, ey = sc.y + (x.chance(0.6) ? 0 : (x.nextDouble() - 0.5) * 2),
                    ez = sc.z + (x.nextDouble() - 0.5) * 4;
            st.entities.add(new Ents.Spawn(id, type, ex, ey, ez, (float) (x.nextDouble() * 360), x.chance(0.2) ? 3 : x.range(0, 2)));
            live.add(new double[] {id, ex, ey, ez, type.equals("player") ? 1 : 0});
            names.add(type.equals("player") ? Ents.playerNameOf(id) : Ents.uuidOf(id).toString());
        }
        if (x.chance(0.35)) {
            // Something solid right next to the player's feet (boats, shulkers at block centres).
            int id = freshId(x, used);
            boolean shulker = x.chance(0.4);
            double ex = shulker ? Math.floor(sc.x + (x.nextDouble() - 0.5) * 3) + 0.5 : sc.x + (x.nextDouble() - 0.5) * 2.5;
            double ez = shulker ? Math.floor(sc.z + (x.nextDouble() - 0.5) * 3) + 0.5 : sc.z + (x.nextDouble() - 0.5) * 2.5;
            double ey = shulker ? Math.floor(sc.y) : sc.y + (x.chance(0.7) ? 0 : (x.nextDouble() - 0.5));
            st.entities.add(new Ents.Spawn(id, shulker ? "shulker" : "boat", ex, ey, ez, (float) (x.nextDouble() * 360), 0));
            if (!shulker) {
                live.add(new double[] {id, ex, ey, ez, 0});
            }
            names.add(Ents.uuidOf(id).toString());
        }
        if (x.chance(0.25)) {
            // Far entities: the map grows past its first size.
            int far = x.range(10, 30);
            for (int i = 0; i < far; i++) {
                int id = freshId(x, used);
                st.entities.add(new Ents.Spawn(id, "armor_stand", sc.x + 8 + x.nextDouble() * 6, sc.y, sc.z + (x.nextDouble() - 0.5) * 12, 0.0F, 0));
            }
        }
        // Scoreboard name -> team (the client throws on a leave for a name not on that team).
        java.util.Map<String, String> teamOf = new java.util.HashMap<>();
        if (x.chance(0.35)) {
            int teams = x.range(1, 2);
            for (int t = 0; t < teams; t++) {
                List<String> members = new ArrayList<>();
                for (String nm : names) {
                    if (!teamOf.containsKey(nm) && x.chance(0.5)) {
                        members.add(nm);
                        teamOf.put(nm, "t" + t);
                    }
                }
                st.teams.add(new Ents.TeamOp("t" + t, 0, x.pick(RULES), List.copyOf(members)));
            }
        }
        for (int t = 0; t < sc.ticks.size(); t++) {
            for (double[] e : live) {
                int id = (int) e[0];
                if (x.chance(0.15)) {
                    double step = x.chance(0.2) ? 1.0 : 0.25;
                    e[1] += (x.nextDouble() - 0.5) * step;
                    e[2] += x.chance(0.8) ? 0 : (x.nextDouble() - 0.5) * step;
                    e[3] += (x.nextDouble() - 0.5) * step;
                    if (x.chance(0.3)) {
                        e[1] += (sc.x - e[1]) * 0.3;
                        e[3] += (sc.z - e[3]) * 0.3;
                    }
                    addEvent(sc, t, new Ents.Move(id, e[1], e[2], e[3], (float) (x.nextDouble() * 360), x.chance(0.7)));
                } else if (x.chance(0.1)) {
                    short dx = (short) ((x.nextDouble() - 0.5) * 2400), dz = (short) ((x.nextDouble() - 0.5) * 2400);
                    short dy = x.chance(0.8) ? 0 : (short) ((x.nextDouble() - 0.5) * 1200);
                    // The client adds the deltas to its own lfloor(x * 4096) (see Ents.Rel).
                    e[1] = (Math.floor(e[1] * 4096.0) + dx) / 4096.0;
                    e[2] = (Math.floor(e[2] * 4096.0) + dy) / 4096.0;
                    e[3] = (Math.floor(e[3] * 4096.0) + dz) / 4096.0;
                    addEvent(sc, t, new Ents.Rel(id, dx, dy, dz, x.chance(0.7)));
                } else if (x.chance(0.04)) {
                    addEvent(sc, t, new Ents.Look(id, (float) (x.nextDouble() * 360)));
                }
                if (e[4] == 1 && x.chance(0.02)) {
                    addEvent(sc, t, new Ents.GameMode(id, x.chance(0.5) ? 3 : 0));
                }
            }
            if (x.chance(0.02) && !live.isEmpty()) {
                double[] e = live.remove(x.nextInt(live.size()));
                addEvent(sc, t, new Ents.Remove((int) e[0]));
            }
            if (x.chance(0.03) && live.size() < 8) {
                int id = freshId(x, used);
                String type = x.pick(POOL);
                double ex = sc.x + (x.nextDouble() - 0.5) * 3, ez = sc.z + (x.nextDouble() - 0.5) * 3;
                addEvent(sc, t, new Ents.Spawn(id, type, ex, sc.y, ez, (float) (x.nextDouble() * 360), x.range(0, 2)));
                live.add(new double[] {id, ex, sc.y, ez, type.equals("player") ? 1 : 0});
                names.add(type.equals("player") ? Ents.playerNameOf(id) : Ents.uuidOf(id).toString());
            }
            if (x.chance(0.015) && !st.teams.isEmpty()) {
                int k = x.nextInt(st.teams.size());
                String team = st.teams.get(k).name();
                String who = x.pick(names.toArray(new String[0]));
                switch (x.nextInt(3)) {
                    case 0 -> addEvent(sc, t, new Ents.TeamOp(team, 2, x.pick(RULES), List.of()));
                    case 1 -> {
                        addEvent(sc, t, new Ents.TeamOp(team, 3, null, List.of(who)));
                        teamOf.put(who, team);
                    }
                    default -> {
                        if (team.equals(teamOf.get(who))) {
                            addEvent(sc, t, new Ents.TeamOp(team, 4, null, List.of(who)));
                            teamOf.remove(who);
                        }
                    }
                }
            }
        }
        sc.description += ", entities";
        return sc;
    }

    private static int freshId(Rng x, Set<Integer> used) {
        while (true) {
            int id = x.chance(0.1) ? x.range(0, 40) : x.range(2, 5000);
            if (used.add(id)) {
                return id;
            }
        }
    }
}
