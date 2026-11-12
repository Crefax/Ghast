package ghast.oracle.modern;

import ghast.oracle.Scenario;

import java.util.ArrayList;
import java.util.List;

import static ghast.oracle.Scenario.BACK;
import static ghast.oracle.Scenario.FORWARD;
import static ghast.oracle.Scenario.JUMP;
import static ghast.oracle.Scenario.LEFT;
import static ghast.oracle.Scenario.RIGHT;
import static ghast.oracle.Scenario.SNEAK;
import static ghast.oracle.Scenario.SPRINT;
import static ghast.oracle.Scenario.USE;

/**
 * Hand-written 26.1 scenarios: the 1.16.5 set (same names, same inputs; block names updated where 26.1
 * renamed them) plus scenarios for mechanics 1.16.5 does not have. Fill "ids" are 26.1 global block state
 * ids (built from state strings); effect ids are 26.1 registry ids (looked up by name).
 * Yaw 0 faces +Z (south), yaw 90 faces -X (west).
 */
final class Scenarios {
    private Scenarios() {}

    private static int s(String spec) {
        return States.id(spec);
    }

    /** A 33x33 stone floor whose top is at y = 64, centred on (ox, oz). */
    private static Scenario flat(String name, int ox, int oz) {
        return new Scenario(name).fill(ox - 16, 63, oz - 16, ox + 16, 63, oz + 16, s("stone"), 0);
    }

    private static Scenario flat(String name) {
        return flat(name, 0, 0);
    }

    /** 26.1 registry (network) id of a mob effect. */
    static int eff(String name) {
        return net.minecraft.core.registries.BuiltInRegistries.MOB_EFFECT.getId(
                net.minecraft.core.registries.BuiltInRegistries.MOB_EFFECT.getValue(net.minecraft.resources.Identifier.withDefaultNamespace(name)));
    }

    private static Scenario put(Scenario sc, int x, int y, int z, String spec) {
        return sc.block(x, y, z, s(spec), 0);
    }

    private static Scenario fill(Scenario sc, int x0, int y0, int z0, int x1, int y1, int z1, String spec) {
        return sc.fill(x0, y0, z0, x1, y1, z1, s(spec), 0);
    }

    static List<Scenario> all() {
        Harness.init();
        List<Scenario> l = new ArrayList<>();
        int air = s("air");

        // ---- walking / sprinting ---------------------------------------------------------------
        l.add(flat("walk_forward").desc("walk +Z, then release and slide to a stop")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(30, FORWARD).run(12, 0));
        l.add(flat("walk_strafe_diagonal").desc("forward+left at an odd yaw, then back+right")
                .spawn(0.5, 64, 0.5, 37.25f, 10).run(3, 0).run(25, FORWARD | LEFT).run(20, BACK | RIGHT).run(5, 0));
        l.add(flat("walk_turning").desc("walk while yaw and pitch change every tick (look packets)")
                .spawn(0.5, 64, 0.5, -170, 0).run(3, 0).turn(40, FORWARD, 7.35f, 0.9f).turn(10, 0, -3.3f, -2.1f));
        l.add(flat("sprint_key").desc("hold sprint + forward, release sprint, release forward")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(30, FORWARD | SPRINT).run(10, FORWARD).run(10, 0));
        l.add(flat("sprint_double_tap").desc("tap forward, release, press again within 7 ticks -> sprint")
                .spawn(0.5, 64, 0.5, -45, 0).run(3, 0).run(2, FORWARD).run(3, 0).run(25, FORWARD).run(8, 0));
        l.add(flat("sprint_jump").desc("sprint-jump continuously")
                .spawn(0.5, 64, 0.5, 12.5f, 0).run(3, 0).run(5, FORWARD | SPRINT).run(40, FORWARD | SPRINT | JUMP).run(10, 0));
        l.add(flat("sprint_start_in_air").desc("jump without sprint, start sprinting mid-air")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(6, FORWARD).run(1, FORWARD | JUMP).run(3, FORWARD)
                .run(12, FORWARD | SPRINT).run(5, 0));
        l.add(flat("jump_in_place").desc("hold jump: jump cooldown while held")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(40, JUMP).run(10, 0));
        l.add(flat("idle_position_refresh").desc("stand still 45 ticks: position packet every 20 ticks")
                .spawn(0.5, 64, 0.5, 0, 0).run(45, 0));
        Scenario hungry = flat("hungry_no_sprint").desc("food level 6 cannot sprint").spawn(0.5, 64, 0.5, 0, 0);
        hungry.foodLevel = 6;
        l.add(hungry.run(3, 0).run(15, FORWARD | SPRINT));

        // ---- sneaking and poses ------------------------------------------------------------------
        l.add(flat("sneak_walk").desc("sneak + forward: crouching pose (1.5 high), input x0.3")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, FORWARD | SNEAK).run(5, FORWARD).run(5, 0));
        l.add(fill(new Scenario("sneak_edge_diagonal"), -1, 63, -1, 1, 63, 1, "stone").desc("sneak diagonally off a corner")
                .spawn(0.5, 64, 0.5, -45, 0).run(3, 0).run(40, FORWARD | SNEAK).run(5, FORWARD));
        l.add(fill(fill(flat("crouch_under_gap"), -3, 65, 2, 3, 65, 4, "smooth_stone_slab[type=top]"), -3, 66, 2, 3, 66, 4, "stone")
                .desc("sneak into a 1.5-high gap, release sneak inside (stays crouching), walk out and stand up")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(35, FORWARD | SNEAK).run(50, FORWARD).run(5, 0));
        l.add(fill(fill(flat("forced_swimming_pose"), -3, 65, -3, 3, 65, 3, "stone"), -3, 64, 6, 3, 66, 6, "stone")
                .desc("spawned in a 1-high space: swimming (crawl) pose, walk")
                .spawn(0.5, 64, 0.5, 0, 0).run(5, 0).run(20, FORWARD).run(5, FORWARD | SNEAK));

        // ---- step-up, walls, ceilings, falling ---------------------------------------------------------
        l.add(put(put(flat("step_up_slab"), 0, 64, 3, "smooth_stone_slab"), 0, 64, 7, "stone")
                .desc("walk into a bottom slab (step), then a full block (blocked)")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(30, FORWARD));
        l.add(fill(flat("wall_slide"), -8, 64, 4, 8, 66, 4, "stone").desc("walk diagonally into a wall")
                .spawn(0.5, 64, 0.5, 30, 0).run(3, 0).run(30, FORWARD | SPRINT));
        l.add(fill(flat("ceiling_bonk"), -2, 66, -2, 2, 66, 2, "stone").desc("jump under a 2-high ceiling")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, JUMP));
        l.add(flat("fall_20").desc("fall 20 blocks onto stone").spawn(0.5, 84, 0.5, 0, 0).run(45, 0));
        l.add(put(put(flat("stairs_and_fences"), 0, 64, 3, "oak_stairs[facing=south]"), 1, 64, 6,
                "oak_fence[north=true]").desc("walk up a stair, into a fence")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(30, FORWARD).run(10, FORWARD | JUMP));
        l.add(fill(flat("auto_jump"), -3, 64, 4, 3, 64, 4, "stone").desc("auto-jump option on: walking into a step jumps")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(25, FORWARD));
        l.get(l.size() - 1).autoJump = true;

        // ---- block materials ---------------------------------------------------------------------
        l.add(fill(new Scenario("soul_sand_walk"), -3, 63, -3, 3, 63, 12, "soul_sand").desc("walk and sprint-jump on soul sand")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, FORWARD).run(15, FORWARD | SPRINT | JUMP));
        // Soul speed: on the client the boots only cancel soul sand's velocity multiplier; the speed boost
        // itself is a server-side attribute modifier that reaches the client in an entity properties packet.
        Scenario soulSpeed = fill(new Scenario("soul_speed_boots"), -3, 63, -3, 3, 63, 20, "soul_sand")
                .desc("soul speed III boots on soul sand (client: no soul sand slowdown)").spawn(0.5, 64, 0.5, 0, 0);
        soulSpeed.soulSpeed = 3;
        l.add(soulSpeed.run(3, 0).run(25, FORWARD | SPRINT).run(10, 0));
        Scenario soulBoost = fill(new Scenario("soul_speed_server_boost"), -3, 63, -3, 3, 63, 20, "soul_soil")
                .desc("soul speed III on soul soil with the server's soul speed attribute modifier").spawn(0.5, 64, 0.5, 0, 0);
        soulBoost.soulSpeed = 3;
        soulBoost.speedModifiers.add(new double[] {(double) (0.03F * (1.0F + 3 * 0.35F)), 0});
        l.add(soulBoost.run(3, 0).run(25, FORWARD | SPRINT).run(10, 0));
        l.add(fill(new Scenario("blue_ice_slide"), -3, 63, -3, 3, 63, 40, "blue_ice").desc("sprint on blue ice, release")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(15, FORWARD | SPRINT).run(30, 0));
        l.add(fill(new Scenario("slime_bounce"), -2, 63, -2, 2, 63, 2, "slime_block").desc("fall onto slime and bounce")
                .spawn(0.5, 75, 0.5, 0, 0).run(60, 0));
        l.add(put(put(new Scenario("bed_bounce"), 0, 63, 0, "red_bed[facing=south,part=foot]"), 0, 63, 1,
                "red_bed[facing=south,part=head]").desc("fall onto a bed (bounces)").spawn(0.5, 70, 0.5, 0, 0).run(40, 0));
        l.add(fill(fill(flat("honey_block"), -3, 63, 3, 3, 63, 10, "honey_block"), -3, 64, 12, 3, 70, 12, "honey_block")
                .desc("walk and jump on honey, then slide down a honey wall")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, FORWARD | SPRINT).run(10, FORWARD | JUMP).run(5, 0));
        l.add(new Scenario("honey_wall_slide").fill(-3, 50, -3, 3, 50, 3, s("stone"), 0)
                .fill(-3, 51, 2, 3, 70, 2, s("honey_block"), 0).desc("fall pressed against a honey wall")
                .spawn(0.5, 68, 1.2, 0, 0).run(40, FORWARD));
        l.add(fill(flat("sweet_berry_bush"), -2, 64, 3, 2, 64, 5, "sweet_berry_bush[age=3]").desc("walk through berry bushes")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(30, FORWARD).run(10, FORWARD | JUMP));
        l.add(fill(flat("cobweb"), 0, 64, 3, 0, 65, 4, "cobweb").desc("walk into cobweb, jump inside, walk out")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(25, FORWARD).run(10, FORWARD | JUMP).run(20, FORWARD));
        l.add(fill(fill(flat("scaffolding"), 0, 64, 3, 0, 70, 3, "scaffolding[distance=0]"), 1, 64, 3, 1, 70, 3, "scaffolding[distance=1]")
                .desc("walk into scaffolding, climb with jump, hold, descend with sneak, walk off the top")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(10, FORWARD).run(35, FORWARD | JUMP).run(10, 0)
                .run(15, SNEAK).run(10, 0));
        l.add(fill(fill(flat("scaffolding_top"), -1, 64, 2, 1, 67, 4, "scaffolding[distance=0]"), -1, 63, -1, 1, 63, 1, "stone")
                .desc("stand on top of scaffolding, sneak to sink through")
                .spawn(0.5, 68, 3.5, 0, 0).run(10, 0).run(20, SNEAK).run(10, 0));

        // ---- climbing ------------------------------------------------------------------------------
        l.add(fill(fill(fill(new Scenario("ladder_climb"), -2, 63, -2, 2, 63, 4, "stone"), 0, 64, 3, 0, 72, 3, "stone"),
                0, 64, 2, 0, 71, 2, "ladder[facing=north]").desc("walk into a ladder, climb, hold with sneak, descend")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(40, FORWARD).run(10, SNEAK).run(25, 0));
        l.add(fill(fill(fill(new Scenario("vine_climb"), -2, 63, -2, 2, 63, 4, "stone"), -1, 64, 3, 1, 72, 3, "stone"),
                0, 64, 2, 0, 71, 2, "vine[south=true]").desc("climb a vine")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(40, FORWARD).run(15, 0));
        l.add(put(fill(fill(fill(new Scenario("trapdoor_ladder"), -2, 63, -2, 2, 63, 4, "stone"), 0, 64, 3, 0, 72, 3, "stone"),
                0, 64, 2, 0, 66, 2, "ladder[facing=north]"), 0, 67, 2, "oak_trapdoor[facing=north,open=true]")
                .desc("climb a ladder into an open trapdoor above it (climbable)")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(40, FORWARD).run(10, 0));
        l.add(fill(fill(new Scenario("twisting_vines"), -2, 63, -2, 2, 63, 2, "stone"), 0, 64, 0, 0, 70, 0, "twisting_vines_plant")
                .desc("jump-climb inside twisting vines").spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(30, JUMP).run(20, 0));

        // ---- water and lava (1.13+ water physics) ------------------------------------------------------
        l.add(fill(fill(fill(new Scenario("swim_sprint"), -6, 50, -6, 6, 50, 30, "stone"), -6, 51, -6, 6, 60, 30, "water"),
                -6, 61, -6, 6, 61, 30, "air").desc("sprint-swim submerged (swimming pose), surface, tread water")
                .spawn(0.5, 55, 0.5, 0, 20).run(5, 0).run(30, FORWARD | SPRINT).turn(10, FORWARD | SPRINT, 0, -6)
                .run(15, FORWARD | SPRINT).run(10, FORWARD).run(20, JUMP).run(10, SNEAK));
        l.add(fill(fill(fill(new Scenario("water_walk_in"), -3, 60, -3, 3, 60, 12, "stone"), -3, 61, 4, 3, 63, 12, "water"),
                -3, 61, -3, 3, 63, 3, "stone").desc("walk into still water, jump to float, walk out")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, FORWARD).run(25, FORWARD | JUMP).run(10, BACK | JUMP));
        Scenario current = new Scenario("water_current").fill(-6, 62, -2, 6, 62, 2, s("stone"), 0);
        for (int i = 0; i < 8; i++) {
            current.fill(-5 + i, 63, -2, -5 + i, 63, 2, s(i == 0 ? "water" : "water[level=" + i + "]"), 0);
        }
        l.add(current.desc("stand in flowing water (levels 0..7) and get pushed").spawn(-3.5, 63, 0.5, 0, 0).run(40, 0));
        l.add(fill(fill(fill(new Scenario("bubble_column_up"), -2, 50, -2, 2, 50, 2, "stone"), -2, 51, -2, 2, 60, 2, "water"),
                0, 51, 0, 0, 60, 0, "bubble_column[drag=false]").desc("bubble column over soul sand pushes up")
                .block(0, 50, 0, s("soul_sand"), 0).spawn(0.5, 52, 0.5, 0, 0).run(50, 0));
        l.add(fill(fill(fill(new Scenario("bubble_column_down"), -2, 50, -2, 2, 50, 2, "stone"), -2, 51, -2, 2, 60, 2, "water"),
                0, 51, 0, 0, 60, 0, "bubble_column[drag=true]").desc("bubble column over magma drags down")
                .block(0, 50, 0, s("magma_block"), 0).spawn(0.5, 59, 0.5, 0, 0).run(40, JUMP));
        Scenario strider = fill(fill(new Scenario("depth_strider"), -3, 60, -3, 3, 60, 20, "stone"), -3, 61, -3, 3, 62, 20, "water")
                .desc("walk on the bottom with depth strider III").spawn(0.5, 61, 0.5, 0, 0);
        strider.depthStrider = 3;
        l.add(strider.run(5, 0).run(30, FORWARD));
        l.add(fill(fill(fill(new Scenario("waterlogged_slabs"), -3, 60, -3, 3, 60, 12, "stone"), -3, 61, -3, 3, 61, 12,
                "smooth_stone_slab[waterlogged=true]"), -3, 61, 6, 3, 61, 12, "water").desc("walk on waterlogged slabs into water")
                .spawn(0.5, 61.5, 0.5, 0, 0).run(3, 0).run(25, FORWARD));
        l.add(fill(fill(fill(new Scenario("lava_basic"), -3, 60, -3, 3, 60, 12, "stone"), -3, 61, -3, 3, 63, 3, "stone"),
                -3, 61, 4, 3, 63, 12, "lava").desc("walk into lava and jump in it")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, FORWARD).run(20, FORWARD | JUMP));

        // ---- creative flight -----------------------------------------------------------------------------
        l.add(flat("creative_flight_toggle").desc("double-tap jump to fly, rise, sneak down, land")
                .mode(Scenario.GameMode.CREATIVE).spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(2, 0)
                .run(1, JUMP).run(15, JUMP).run(10, 0).run(30, SNEAK).run(5, 0));
        l.add(flat("creative_flight_sprint").desc("sprint-fly forward and diagonally, stop in the air")
                .mode(Scenario.GameMode.CREATIVE).flying(true).spawn(0.5, 70, 0.5, 33, -10).run(3, 0)
                .run(25, FORWARD | SPRINT).run(10, FORWARD | LEFT | SPRINT).run(15, 0));
        l.add(flat("spectator_noclip").desc("spectator: flying with no collision through a wall")
                .fill(-5, 64, 4, 5, 70, 4, s("stone"), 0).mode(Scenario.GameMode.SPECTATOR).flying(true)
                .spawn(0.5, 65, 0.5, 0, 0).run(3, 0).run(20, FORWARD).run(5, SNEAK));

        // ---- chunks, coordinates, server packets ----------------------------------------------------------
        l.add(new Scenario("walk_into_unloaded").desc("walk off loaded terrain into an unloaded chunk")
                .outside(Scenario.Outside.UNLOADED).fill(0, 63, 0, 15, 63, 15, s("stone"), 0)
                .spawn(8.5, 64, 12.5, 0, 0).run(3, 0).run(30, FORWARD | SPRINT).run(10, 0));
        l.add(flat("far_coordinates", 29_999_000, -29_999_000).desc("walk and sprint-jump near the world border")
                .spawn(29_999_000.5, 64, -29_999_000.5, -135, 0).run(3, 0).run(30, FORWARD | SPRINT | JUMP));
        l.add(flat("world_border", 29_999_970, 0).desc("sprint into the default world border at x = 29,999,984 (collides like a wall)")
                .spawn(29_999_979.5, 64, 0.5, -90, 0).run(3, 0).run(20, FORWARD | SPRINT).run(10, FORWARD | SPRINT | JUMP));
        l.add(flat("negative_coordinates", -1000, -1000).desc("walk across chunk borders at negative coords")
                .spawn(-1000.3, 64, -1000.7, 135.5f, 0).run(3, 0).run(40, FORWARD | SPRINT));
        l.add(flat("velocity_packet").desc("knockback velocity packets while walking")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(5, FORWARD).event(FORWARD, new Scenario.Velocity(3200, 3000, -1600))
                .run(20, FORWARD).event(0, new Scenario.Velocity(-8000, 2000, 0)).run(20, 0));
        l.add(flat("teleport_packet").desc("server teleport while sprinting (teleport_confirm)")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(10, FORWARD | SPRINT)
                .event(FORWARD | SPRINT, new Scenario.Teleport(5.5, 64, -3.5, 180, 10)).run(15, FORWARD | SPRINT));
        l.add(flat("abilities_packet").desc("server grants flight mid-jump, then revokes it")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(2, 0)
                .event(0, new Scenario.Abilities(false, true, true, false, 0.05f, 0.1f)).run(10, JUMP)
                .event(0, new Scenario.Abilities(false, false, false, false, 0.05f, 0.1f)).run(25, 0));

        // ---- attributes, effects, item use ---------------------------------------------------------------
        Scenario speed = flat("speed_modifier").desc("speed II as the server sends it (attribute modifier, op 2)")
                .spawn(0.5, 64, 0.5, 0, 0);
        speed.speedModifiers.add(new double[] {0.4000000059604645, 2});
        l.add(speed.run(3, 0).run(20, FORWARD).run(20, FORWARD | SPRINT | JUMP));
        l.add(flat("jump_boost").desc("jump boost II").effect(eff("jump_boost"), 1).spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(30, JUMP | FORWARD));
        l.add(flat("slow_falling").desc("slow falling from a height").effect(eff("slow_falling"), 0).spawn(0.5, 80, 0.5, 0, 0).run(60, FORWARD));
        l.add(flat("levitation").desc("levitation I then sneak").effect(eff("levitation"), 0).spawn(0.5, 64, 0.5, 0, 0).run(30, 0).run(10, SNEAK));
        l.add(fill(fill(fill(new Scenario("dolphins_grace"), -6, 50, -6, 6, 50, 30, "stone"), -6, 51, -6, 6, 60, 30, "water"),
                -6, 61, -6, 6, 61, 30, "air").desc("swim with dolphin's grace").effect(eff("dolphins_grace"), 0)
                .spawn(0.5, 55, 0.5, 0, 10).run(5, 0).run(30, FORWARD | SPRINT).run(10, FORWARD));
        l.add(flat("use_item_shield").desc("walk, raise a shield (input x0.2), sprint attempt while blocking, lower")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(8, FORWARD).run(12, FORWARD | USE)
                .run(8, FORWARD | SPRINT | USE).run(10, FORWARD | SPRINT | JUMP | USE).run(10, FORWARD | SPRINT).run(5, 0));
        l.add(flat("blindness_no_sprint").desc("blindness blocks sprinting").effect(eff("blindness"), 0)
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(15, FORWARD | SPRINT));
        Scenarios26.add(l);
        if (air != 0) {
            throw new IllegalStateException("air is not state 0");
        }
        return l;
    }
}
