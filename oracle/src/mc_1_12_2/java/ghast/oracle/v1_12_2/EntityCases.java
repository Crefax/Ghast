package ghast.oracle.v1_12_2;

import ghast.oracle.Rng;
import ghast.oracle.Scenario;

import java.util.ArrayList;
import java.util.List;

import static ghast.oracle.Scenario.FORWARD;
import static ghast.oracle.Scenario.JUMP;
import static ghast.oracle.Scenario.LEFT;
import static ghast.oracle.Scenario.SNEAK;
import static ghast.oracle.Scenario.SPRINT;
import static ghast.oracle.v1_12_2.Blocks.*;

/**
 * 1.12.2 cases with other entities (see {@link Ent}): mobs, other players and boats pushing the player
 * from their client ticks, boats and shulkers it collides with, entities that do neither, team
 * collision rules and spectators. Hand-written cases plus a fuzz generator ({@link #fuzz}).
 */
final class EntityCases {
    private EntityCases() {}

    private static Scenario flat(String name) {
        return new Scenario(name).fill(-16, 63, -16, 16, 63, 16, STONE);
    }

    private static Scenario with(Scenario sc, Scenario.Event... events) {
        Ent.setup(sc).events.addAll(List.of(events));
        return sc;
    }

    private static Ent.Spawn sp(int id, String type, double x, double y, double z) {
        return new Ent.Spawn(id, type, x, y, z, 0.0F);
    }

    private static Ent.Tab tab(int id, int gameMode) {
        return new Ent.Tab(id, true, gameMode);
    }

    private static Ent.Team team(String name, String rule, String... members) {
        return new Ent.Team(name, 0, rule, List.of(members));
    }

    static void addEvent(Scenario sc, int tick, Scenario.Event e) {
        Scenario.Tick t = sc.ticks.get(tick);
        List<Scenario.Event> l = new ArrayList<>(t.events());
        l.add(e);
        sc.ticks.set(tick, new Scenario.Tick(t.keys(), t.yaw(), t.pitch(), List.copyOf(l)));
    }

    private static String mob(int id) {
        return Ent.member(id, false);
    }

    static List<Scenario> all() {
        List<Scenario> l = new ArrayList<>();
        l.add(with(flat("push_zombie"), sp(100, "zombie", 0.8, 64, 0.7)).desc("a zombie overlapping the player pushes it away each tick")
                .spawn(0.5, 64, 0.5, 0, 0).run(30, 0).run(20, FORWARD));
        l.add(with(flat("push_crowd"), sp(100, "villager", 0.2, 64, 0.5), sp(101, "cow", 0.9, 64, 0.2), sp(102, "pig", 0.5, 64, 1.0),
                sp(103, "sheep", 0.45, 64, 0.55)).desc("four mobs around the player: pushes add up in tick order")
                .spawn(0.5, 64, 0.5, 0, 0).run(40, 0));
        l.add(with(flat("push_sizes"), sp(100, "spider", 1.4, 64, 0.5), sp(101, "chicken", 0.3, 64, 0.2), sp(102, "horse", -0.4, 64, 1.1),
                sp(103, "slime", 0.6, 64.9, 0.9), sp(104, "wolf", 0.5, 64, -0.3)).desc("mobs of several sizes (spider, chicken, horse, slime, wolf)")
                .spawn(0.5, 64, 0.5, 0, 0).run(30, 0).run(15, FORWARD | LEFT));
        l.add(with(flat("push_walk_into_mobs"), sp(100, "cow", 0.5, 64, 3.0), sp(101, "cow", 0.9, 64, 4.2), sp(102, "villager_golem", 0.4, 64, 6))
                .desc("walk and sprint into cows and an iron golem").spawn(0.5, 64, 0.5, 0, 0).run(5, 0).run(40, FORWARD | SPRINT));
        l.add(with(flat("no_push_armor_stand_bat_parrot"), sp(100, "armor_stand", 0.6, 64, 0.6), sp(101, "bat", 0.4, 64.5, 0.4),
                sp(102, "parrot", 0.5, 64.2, 0.7), sp(103, "item", 0.4, 64, 0.5), sp(104, "minecart", 0.6, 64, 0.3))
                .desc("armor stands, bats, parrots, items and minecarts neither push nor block the player").spawn(0.5, 64, 0.5, 0, 0)
                .run(20, 0).run(20, FORWARD));
        l.add(with(flat("boat_collide"), sp(100, "boat", 0.5, 64, 3.0)).desc("walk into a boat, jump on it, stand, walk off; sneak at its edge")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(12, FORWARD).run(1, FORWARD | JUMP).run(10, FORWARD).run(10, 0)
                .run(20, FORWARD | SNEAK).run(10, FORWARD));
        l.add(with(flat("boat_push"), sp(100, "boat", 0.5, 64, 1.55), sp(101, "boat", 1.8, 64, 0.5), sp(102, "boat", -0.9, 63.8, 0.5))
                .desc("boats next to the player push it (their box grown by 0.2, feet not above the boat's bottom)").spawn(0.5, 64, 0.5, 0, 0)
                .run(40, 0));
        l.add(with(flat("shulker_block").fill(-3, 63, 2, 3, 63, 6, STONE, 0), sp(100, "shulker", 0.5, 64, 3.5), sp(101, "shulker", 0.5, 65, 4.5))
                .desc("shulkers collide like blocks (and push): walk into one, jump up two").spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(10, FORWARD)
                .run(1, FORWARD | JUMP).run(12, FORWARD).run(1, FORWARD | JUMP).run(15, FORWARD));
        Scenario moving = with(flat("push_moving_zombie"), sp(100, "zombie", 3.0, 64, 0.5)).desc("a zombie walks through the standing player (teleport packets, interpolated over 3 ticks)")
                .spawn(0.5, 64, 0.5, 0, 0).run(40, 0);
        for (int i = 0; i < 20; i++) {
            addEvent(moving, 2 + i, new Ent.Move(100, 3.0 - i * 0.25, 64, 0.5 + (i % 3) * 0.05, 90.0F, true));
        }
        l.add(moving);
        Scenario rel = with(flat("push_relative_moves"), sp(100, "cow", 2.0, 64, 0.6), sp(101, "zombie", -1.5, 64, 0.4))
                .desc("mobs moved by relative move packets (1/4096 deltas, positions snap to the grid)").spawn(0.5, 64, 0.5, 0, 0).run(40, 0);
        for (int i = 0; i < 16; i++) {
            addEvent(rel, 1 + 2 * i, new Ent.Rel(100, -1000 - 37 * i, 0, 13 * i, true));
            addEvent(rel, 2 + 2 * i, new Ent.Rel(101, 700 + 11 * i, (i % 4 == 0) ? 1 : 0, -5 * i, true));
        }
        l.add(rel);
        Scenario tiny = with(flat("push_teleport_tiny"), sp(100, "zombie", 0.85, 64, 0.5)).desc("teleports under 1/32 block cancel the interpolation")
                .spawn(0.5, 64, 0.5, 0, 0).run(30, 0);
        addEvent(tiny, 2, new Ent.Move(100, 2.0, 64, 0.5, 0.0F, true));
        addEvent(tiny, 3, new Ent.Move(100, 2.01, 64, 0.51, 0.0F, true));
        addEvent(tiny, 10, new Ent.Move(100, 0.7, 64, 0.6, 0.0F, true));
        addEvent(tiny, 12, new Ent.Move(100, 0.72, 64.005, 0.6, 0.0F, true));
        l.add(tiny);
        l.add(with(flat("push_ladder").fill(0, 64, 1, 0, 70, 1, STONE, 0).fill(0, 64, 0, 0, 70, 0, LADDER, 2),
                sp(100, "zombie", 0.8, 64.5, 0.5)).desc("on a ladder the player is not pushable").spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, FORWARD | JUMP).run(10, 0));
        Scenario spec = with(flat("push_spectator"), sp(100, "zombie", 0.8, 64, 0.7), sp(101, "boat", 0.5, 64, 1.4)).desc("spectators are not pushed");
        l.add(spec.spawn(0.5, 64, 0.5, 0, 0).mode(Scenario.GameMode.SPECTATOR).run(20, 0).run(10, FORWARD));
        l.add(with(flat("push_sneaking"), sp(100, "pig", 0.7, 64, 0.8)).desc("a sneaking player is pushed too").spawn(0.5, 64, 0.5, 0, 0)
                .run(30, SNEAK).run(10, SNEAK | FORWARD));
        l.add(with(new Scenario("push_in_water").fill(-8, 62, -8, 8, 62, 8, STONE).fill(-8, 63, -8, 8, 64, 8, WATER),
                sp(100, "zombie", 0.8, 63, 0.7), sp(101, "boat", 0.5, 63.5, 1.6)).desc("pushes while standing in water")
                .spawn(0.5, 63, 0.5, 0, 0).run(40, 0));
        Scenario boatStack = with(flat("boat_stand_on"), sp(100, "boat", 0.5, 64, 0.5)).desc("stand on a boat, the boat moves away (10-tick interpolation)")
                .spawn(0.5, 64.5625, 0.5, 0, 0).run(30, 0);
        for (int i = 0; i < 10; i++) {
            addEvent(boatStack, 5 + i, new Ent.Move(100, 0.5 + i * 0.12, 64, 0.5, 0.0F, true));
        }
        l.add(boatStack);
        Scenario spawnLate = flat("push_spawned_in").desc("mobs spawned and removed mid-case").spawn(0.5, 64, 0.5, 0, 0).run(40, 0);
        with(spawnLate);
        addEvent(spawnLate, 5, new Ent.Spawn(200, "zombie", 0.6, 64, 0.6, 45.0F));
        addEvent(spawnLate, 12, new Ent.Spawn(201, "slime", 0.3, 64, 0.3, 0.0F));
        addEvent(spawnLate, 20, new Ent.Remove(200));
        addEvent(spawnLate, 30, new Ent.Remove(201));
        l.add(spawnLate);
        // Other players: they push from OtherClientPlayerEntity.tickMovement; a spectator does not
        // (noClip), whatever its team.
        Scenario players = with(flat("push_other_players"), tab(300, 0), tab(301, 3), tab(302, 1),
                sp(300, "player", 0.9, 64, 0.6), sp(301, "player", 0.2, 64, 0.4), sp(302, "player", 0.5, 64, -0.1))
                .desc("other players push (creative too), a spectator does not; one changes to spectator, one leaves spectator")
                .spawn(0.5, 64, 0.5, 0, 0).run(40, 0);
        addEvent(players, 15, new Ent.Tab(300, false, 3));
        addEvent(players, 25, new Ent.Tab(301, false, 0));
        for (int i = 0; i < 8; i++) {
            addEvent(players, 28 + i, new Ent.Move(302, 0.5 + 0.05 * i, 64, -0.1 + 0.08 * i, 0.0F, true));
        }
        l.add(players);
        // Team collision rules (EntityPredicate.method_13025): the pusher's and the player's.
        l.add(with(flat("team_never_pusher"), sp(100, "zombie", 0.8, 64, 0.7), sp(101, "cow", 0.2, 64, 0.4), team("red", "never", mob(100)))
                .desc("a mob on a team with rule never does not push; another mob does").spawn(0.5, 64, 0.5, 0, 0).run(30, 0));
        l.add(with(flat("team_never_player"), sp(100, "zombie", 0.8, 64, 0.7), sp(101, "boat", 0.5, 64, 1.5), team("blue", "never", "oracle"))
                .desc("the player on a team with rule never is pushed by nothing (boats still collide)").spawn(0.5, 64, 0.5, 0, 0)
                .run(20, 0).run(15, FORWARD));
        l.add(with(flat("team_push_other_teams"), sp(100, "zombie", 0.8, 64, 0.7), sp(101, "cow", 0.2, 64, 0.4), sp(102, "pig", 0.5, 64, 0.95),
                team("a", "pushOtherTeams", "oracle", mob(100)), team("b", "always", mob(101)))
                .desc("pushOtherTeams (HIDE_FOR_OTHER_TEAMS): only the team mate pushes").spawn(0.5, 64, 0.5, 0, 0).run(30, 0));
        l.add(with(flat("team_push_own_team"), sp(100, "zombie", 0.8, 64, 0.7), sp(101, "cow", 0.2, 64, 0.4), sp(102, "pig", 0.5, 64, 0.95),
                team("a", "pushOwnTeam", "oracle", mob(100)), team("b", "always", mob(101)))
                .desc("pushOwnTeam (HIDE_FOR_OWN_TEAM): the team mate does not push, the other team's mob and the teamless pig do").spawn(0.5, 64, 0.5, 0, 0).run(30, 0));
        Scenario changes = with(flat("team_changes"), tab(300, 0), sp(100, "zombie", 0.8, 64, 0.7), sp(300, "player", 0.2, 64, 0.45),
                team("a", "always", "oracle", "p300"))
                .desc("rule and members change mid-case (update, add, remove members, remove the team)").spawn(0.5, 64, 0.5, 0, 0).run(50, 0);
        addEvent(changes, 8, new Ent.Team("a", 2, "pushOwnTeam", List.of()));
        addEvent(changes, 16, new Ent.Team("a", 3, "", List.of(mob(100))));
        addEvent(changes, 24, new Ent.Team("a", 2, "never", List.of()));
        addEvent(changes, 32, new Ent.Team("a", 4, "", List.of("oracle")));
        addEvent(changes, 40, new Ent.Team("a", 1, "", List.of()));
        l.add(changes);
        return l;
    }

    // ---- fuzz -------------------------------------------------------------------------------------

    private static final String[] POOL = {"zombie", "villager", "cow", "pig", "sheep", "chicken", "armor_stand", "bat", "parrot",
            "boat", "boat", "shulker", "villager_golem", "horse", "slime", "spider", "wolf", "minecart", "skeleton", "creeper",
            "llama", "item", "player", "player"};
    private static final String[] RULES = {"always", "never", "pushOtherTeams", "pushOwnTeam"};

    /**
     * Randomized entity cases: the {@link Fuzz#withExtras} case (always with loaded chunks around it) with
     * 1-6 entities of a mixed pool within two blocks, moved by teleport and relative move packets,
     * spawned and removed during the case; other players with tab entries whose game mode changes; teams
     * with random collision rules and members. Shulkers are spawned at block centres and never moved.
     * Cases get an {@code _en} suffix.
     */
    static Scenario fuzz(long seed, int index, int ticks) {
        Scenario base = Fuzz.withExtras(seed, index, ticks, false);
        Scenario sc = Fuzz.renamedTo(base, base.name + "_en");
        sc.outside = Scenario.Outside.AIR;
        sc.extraChunks.clear();
        Rng x = new Rng(seed * 0x9E3779B97F4A7C15L + index ^ 0x656e74697479L);
        Ent.Setup st = Ent.setup(sc);
        int n = x.range(1, 6);
        int nextId = 300;
        List<double[]> live = new ArrayList<>();
        List<String> names = new ArrayList<>();
        names.add("oracle");
        for (int i = 0; i < n; i++) {
            spawnInto(sc, x, nextId++, st.events, live, names, -1);
        }
        if (x.chance(0.35)) {
            int teams = x.range(1, 2);
            for (int k = 0; k < teams; k++) {
                List<String> members = new ArrayList<>();
                for (String m : names) {
                    if (x.chance(0.5)) {
                        members.add(m);
                    }
                }
                st.events.add(new Ent.Team("t" + k, 0, x.pick(RULES), List.copyOf(members)));
            }
        }
        for (int t = 0; t < sc.ticks.size(); t++) {
            for (double[] e : live) {
                if (e[4] != 0 || !x.chance(0.15)) {
                    continue;
                }
                if (x.chance(0.4)) {
                    // Relative move: short deltas in 1/4096 blocks.
                    int dx = (int) ((x.nextDouble() - 0.5) * 4096), dz = (int) ((x.nextDouble() - 0.5) * 4096);
                    int dy = x.chance(0.8) ? 0 : (int) ((x.nextDouble() - 0.5) * 2048);
                    if (x.chance(0.3)) {
                        dx = (int) ((sc.x - e[1]) * 4096 * 0.3);
                        dz = (int) ((sc.z - e[3]) * 4096 * 0.3);
                    }
                    dx = Math.max(-32768, Math.min(32767, dx));
                    dz = Math.max(-32768, Math.min(32767, dz));
                    // The client position follows the server's fixed-point one.
                    e[5] += dx;
                    e[6] += dy;
                    e[7] += dz;
                    e[1] = e[5] / 4096.0;
                    e[2] = e[6] / 4096.0;
                    e[3] = e[7] / 4096.0;
                    addEvent(sc, t, new Ent.Rel((int) e[0], dx, dy, dz, x.chance(0.7)));
                } else {
                    double step = x.chance(0.2) ? 1.0 : x.chance(0.3) ? 0.02 : 0.25;
                    e[1] += (x.nextDouble() - 0.5) * step;
                    e[2] += x.chance(0.8) ? 0 : (x.nextDouble() - 0.5) * step;
                    e[3] += (x.nextDouble() - 0.5) * step;
                    if (x.chance(0.3)) {
                        e[1] += (sc.x - e[1]) * 0.3;
                        e[3] += (sc.z - e[3]) * 0.3;
                    }
                    e[5] = Math.floor(e[1] * 4096.0);
                    e[6] = Math.floor(e[2] * 4096.0);
                    e[7] = Math.floor(e[3] * 4096.0);
                    addEvent(sc, t, new Ent.Move((int) e[0], e[1], e[2], e[3], (float) (x.nextDouble() * 360), x.chance(0.7)));
                }
            }
            if (x.chance(0.02) && !live.isEmpty()) {
                double[] e = live.remove(x.nextInt(live.size()));
                addEvent(sc, t, new Ent.Remove((int) e[0]));
            }
            if (x.chance(0.03) && live.size() < 8) {
                List<Scenario.Event> evs = new ArrayList<>();
                spawnInto(sc, x, nextId++, evs, live, names, t);
                for (Scenario.Event e : evs) {
                    addEvent(sc, t, e);
                }
            }
            if (x.chance(0.01)) {
                // A player's game mode changes (spectators do not push).
                for (double[] e : live) {
                    if (e[8] == 1) {
                        addEvent(sc, t, new Ent.Tab((int) e[0], false, x.chance(0.5) ? 3 : 0));
                        break;
                    }
                }
            }
        }
        sc.description += ", entities";
        return sc;
    }

    /**
     * Adds a spawn (and a tab entry first for a player) of a random pool type near the spawn. Entry:
     * {id, x, y, z, fixed (1: shulker, never moved), tracked x, y, z (1/4096), player}.
     */
    private static void spawnInto(Scenario sc, Rng x, int id, List<Scenario.Event> out, List<double[]> live, List<String> names, int tick) {
        String type = x.pick(POOL);
        double ex = sc.x + (x.nextDouble() - 0.5) * 4, ez = sc.z + (x.nextDouble() - 0.5) * 4;
        double ey = tick < 0 ? sc.y + (x.chance(0.6) ? 0 : (x.nextDouble() - 0.5) * 2) : sc.y;
        boolean fixed = type.equals("shulker");
        if (fixed) {
            ex = Math.floor(ex) + 0.5;
            ey = Math.floor(ey);
            ez = Math.floor(ez) + 0.5;
        }
        boolean player = type.equals("player");
        if (player) {
            out.add(new Ent.Tab(id, true, x.chance(0.2) ? 3 : x.chance(0.2) ? 1 : 0));
            names.add("p" + id);
        } else if (x.chance(0.5)) {
            names.add(Ent.member(id, false));
        }
        out.add(new Ent.Spawn(id, type, ex, ey, ez, (float) (x.nextDouble() * 360)));
        live.add(new double[] {id, ex, ey, ez, fixed ? 1 : 0, Math.floor(ex * 4096.0), Math.floor(ey * 4096.0), Math.floor(ez * 4096.0),
                player ? 1 : 0});
    }
}
