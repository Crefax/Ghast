package ghast.oracle.v1_8_9;

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
import static ghast.oracle.v1_8_9.Blocks.*;

/**
 * Hand-written scenarios. Each starts like a fresh login (position packet) and usually spends a few
 * idle ticks settling onto the ground first. Yaw 0 faces +Z (south), yaw 90 faces -X (west).
 */
final class Scenarios {
    private Scenarios() {}

    /** A 33x33 stone floor whose top is at y = 64, centred on (ox, oz). */
    private static Scenario flat(String name, int ox, int oz) {
        return new Scenario(name).fill(ox - 16, 63, oz - 16, ox + 16, 63, oz + 16, STONE);
    }

    private static Scenario flat(String name) {
        return flat(name, 0, 0);
    }

    static List<Scenario> all() {
        List<Scenario> l = new ArrayList<>();

        // ---- walking / sprinting on flat ground ------------------------------------------------
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
        l.add(flat("sprint_double_tap_too_slow").desc("second tap after the 7 tick window -> no sprint")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(2, FORWARD).run(9, 0).run(15, FORWARD));
        l.add(flat("sprint_jump").desc("sprint-jump continuously (jump boost of sprinting, air speed)")
                .spawn(0.5, 64, 0.5, 12.5f, 0).run(3, 0).run(5, FORWARD | SPRINT).run(40, FORWARD | SPRINT | JUMP).run(10, 0));
        l.add(flat("sprint_start_in_air").desc("jump without sprint, start sprinting mid-air (late air speed)")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(6, FORWARD).run(1, FORWARD | JUMP).run(3, FORWARD)
                .run(12, FORWARD | SPRINT).run(5, 0));
        l.add(flat("sprint_stop_by_back").desc("sprint, then press back (forward < 0.8 stops sprint)")
                .spawn(0.5, 64, 0.5, 90, 0).run(3, 0).run(15, FORWARD | SPRINT).run(10, BACK | SPRINT).run(5, 0));
        l.add(flat("jump_in_place").desc("hold jump: jump cooldown of 10 ticks while held")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(40, JUMP).run(10, 0));
        l.add(flat("jump_tap").desc("tap jump repeatedly (cooldown reset on release)")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(13, 0).run(1, JUMP).run(11, 0).run(2, JUMP).run(10, 0));
        l.add(flat("idle_position_refresh").desc("stand still 45 ticks: flying packets, position every 20 ticks")
                .spawn(0.5, 64, 0.5, 0, 0).run(45, 0));
        Scenario hungry = flat("hungry_no_sprint").desc("food level 6 cannot sprint").spawn(0.5, 64, 0.5, 0, 0);
        hungry.foodLevel = 6;
        l.add(hungry.run(3, 0).run(15, FORWARD | SPRINT).run(2, FORWARD).run(1, 0).run(10, FORWARD));

        // ---- sneaking --------------------------------------------------------------------------
        l.add(flat("sneak_walk").desc("sneak + forward (0.3 input), sneak packets")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, FORWARD | SNEAK).run(5, FORWARD).run(5, 0));
        l.add(new Scenario("sneak_edge_x").desc("sneak toward the +X edge of a 3x3 platform: stops at the edge")
                .fill(-1, 63, -1, 1, 63, 1, STONE).spawn(0.5, 64, 0.5, -90, 0).run(3, 0).run(40, FORWARD | SNEAK));
        l.add(new Scenario("sneak_edge_diagonal").desc("sneak diagonally off a corner")
                .fill(-1, 63, -1, 1, 63, 1, STONE).spawn(0.5, 64, 0.5, -45, 0).run(3, 0).run(40, FORWARD | SNEAK)
                .run(5, FORWARD));
        l.add(new Scenario("sneak_edge_sprint_in").desc("arrive fast at an edge, then sneak")
                .fill(-1, 63, -6, 1, 63, 3, STONE).spawn(0.5, 64, -5.5, 0, 0).run(3, 0).run(8, FORWARD | SPRINT)
                .run(20, FORWARD | SNEAK));
        l.add(new Scenario("walk_off_edge").desc("walk off a ledge and fall 3 blocks")
                .fill(-2, 63, -2, 2, 63, 2, STONE).fill(-6, 60, -6, 6, 60, 6, STONE)
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(25, FORWARD).run(5, 0));

        // ---- step-up, walls, corners, ceilings ---------------------------------------------------
        l.add(flat("step_up_slab").desc("walk into a bottom slab (step 0.5), then a full block (blocked)")
                .block(0, 64, 3, STONE_SLAB, 0).block(0, 64, 7, STONE).block(0, 65, 7, STONE)
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(40, FORWARD));
        l.add(flat("step_up_slab_diagonal").desc("diagonal approach to a row of slabs")
                .fill(-5, 64, 3, 5, 64, 3, STONE_SLAB, 0).fill(-5, 64, 4, 5, 64, 6, DOUBLE_STONE_SLAB, 0)
                .spawn(0.5, 64, 0.5, 30, 0).run(3, 0).run(30, FORWARD));
        l.add(flat("jump_onto_block").desc("walk + jump onto a full block")
                .fill(-3, 64, 3, 3, 64, 6, STONE).spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(6, FORWARD)
                .run(12, FORWARD | JUMP).run(10, FORWARD));
        l.add(flat("stairs_walk_up").desc("walk up a staircase of oak stairs")
                .block(0, 64, 3, OAK_STAIRS, 2).block(0, 64, 4, PLANKS).block(0, 65, 4, OAK_STAIRS, 2)
                .fill(0, 64, 5, 0, 65, 7, PLANKS).block(0, 66, 5, OAK_STAIRS, 2)
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(35, FORWARD));
        l.add(flat("wall_slide").desc("walk diagonally into a wall: slide along it")
                .fill(-8, 64, 4, 8, 66, 4, STONE).spawn(0.5, 64, 0.5, 30, 0).run(3, 0).run(30, FORWARD | SPRINT));
        l.add(flat("inner_corner").desc("walk diagonally into an inner corner")
                .fill(-8, 64, 4, 3, 66, 4, STONE).fill(3, 64, -8, 3, 66, 4, STONE)
                .spawn(0.5, 64, 0.5, -45, 0).run(3, 0).run(25, FORWARD));
        l.add(flat("outer_corner_clip").desc("brush past an outer corner")
                .fill(1, 64, 3, 3, 66, 5, STONE).spawn(0.2, 64, 0.5, -8, 0).run(3, 0).run(25, FORWARD));
        l.add(flat("ceiling_bonk").desc("jump under a 2-high ceiling (head hits at y+2)")
                .fill(-2, 66, -2, 2, 66, 2, STONE).spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, JUMP));
        l.add(flat("ceiling_bonk_sprint").desc("sprint-jump in a 2.5-high tunnel of slabs")
                .fill(-1, 66, -3, 1, 66, 12, STONE_SLAB, 8).spawn(0.5, 64, -2.5, 0, 0).run(3, 0)
                .run(25, FORWARD | SPRINT | JUMP));
        l.add(flat("two_high_gap").desc("walk through a 1x2 doorway")
                .fill(-4, 64, 3, 4, 66, 3, STONE).fill(0, 64, 3, 0, 65, 3, AIR)
                .spawn(0.4, 64, 0.5, 3, 0).run(3, 0).run(25, FORWARD));

        // ---- falling and landing -----------------------------------------------------------------
        l.add(flat("fall_20").desc("fall 20 blocks onto stone").spawn(0.5, 84, 0.5, 0, 0).run(45, 0));
        l.add(new Scenario("fall_onto_slab").desc("fall onto a bottom slab (landing at y + 0.5)")
                .block(0, 63, 0, STONE_SLAB, 0).spawn(0.5, 70, 0.5, 0, 0).run(30, 0));
        l.add(new Scenario("fall_onto_soul_sand").desc("land on soul sand (0.875 high)")
                .block(0, 63, 0, SOUL_SAND).spawn(0.5, 70, 0.5, 0, 0).run(30, 0));
        l.add(new Scenario("fall_onto_snow_layers").desc("land on 3 snow layers, then walk onto 1 layer")
                .block(0, 63, 0, SNOW_LAYER, 2).fill(0, 63, 1, 0, 63, 4, SNOW_LAYER, 0).fill(0, 62, 0, 0, 62, 4, STONE)
                .spawn(0.5, 68, 0.5, 0, 0).run(20, 0).run(20, FORWARD));
        l.add(new Scenario("fall_onto_fence").desc("land on a fence post (1.5 high)")
                .block(0, 63, 0, FENCE).block(0, 62, 0, STONE).spawn(0.5, 70, 0.5, 0, 0).run(30, 0));
        l.add(new Scenario("fall_with_horizontal").desc("fall from a ledge with initial horizontal velocity")
                .fill(-3, 50, -3, 3, 50, 12, STONE).spawn(0.5, 70, 0.5, 0, 0).velocity(0.05, 0.3, 0.4).run(40, 0));

        // ---- block materials ---------------------------------------------------------------------
        l.add(new Scenario("soul_sand_walk").desc("walk and sprint on soul sand")
                .fill(-3, 63, -3, 3, 63, 12, SOUL_SAND).spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, FORWARD)
                .run(15, FORWARD | SPRINT | JUMP));
        l.add(new Scenario("ice_slide").desc("sprint on ice, release and slide")
                .fill(-3, 63, -3, 3, 63, 30, ICE).spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(15, FORWARD | SPRINT)
                .run(30, 0));
        l.add(new Scenario("packed_ice_jump").desc("sprint-jump on packed ice")
                .fill(-3, 63, -3, 3, 63, 40, PACKED_ICE).spawn(0.5, 64, 0.5, 0, 0).run(3, 0)
                .run(30, FORWARD | SPRINT | JUMP).run(10, 0));
        l.add(new Scenario("ice_to_stone").desc("slide from ice onto stone")
                .fill(-3, 63, -3, 3, 63, 5, ICE).fill(-3, 63, 6, 3, 63, 20, STONE).spawn(0.5, 64, 0.5, 0, 0)
                .run(3, 0).run(10, FORWARD | SPRINT).run(20, 0));
        l.add(new Scenario("slime_bounce").desc("fall onto slime and bounce")
                .fill(-2, 63, -2, 2, 63, 2, SLIME).spawn(0.5, 75, 0.5, 0, 0).run(60, 0));
        l.add(new Scenario("slime_bounce_sneak").desc("fall onto slime while sneaking (no bounce)")
                .fill(-2, 63, -2, 2, 63, 2, SLIME).spawn(0.5, 72, 0.5, 0, 0).run(30, SNEAK));
        l.add(new Scenario("slime_walk").desc("walk on slime (slow when onGround)")
                .fill(-2, 63, -2, 2, 63, 12, SLIME).spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, FORWARD));
        l.add(new Scenario("cobweb_walk").desc("walk into cobweb, jump inside it, walk out")
                .fill(-3, 63, -3, 3, 63, 10, STONE).fill(0, 64, 3, 0, 65, 4, WEB).spawn(0.5, 64, 0.5, 0, 0)
                .run(3, 0).run(25, FORWARD).run(10, FORWARD | JUMP).run(20, FORWARD));
        l.add(new Scenario("cobweb_fall").desc("fall into cobweb")
                .fill(-1, 63, -1, 1, 63, 1, STONE).fill(0, 64, 0, 0, 66, 0, WEB).spawn(0.5, 75, 0.5, 0, 0).run(50, 0));
        l.add(new Scenario("farmland_walk").desc("walk on farmland (0.9375 high) and step back to grass")
                .fill(-2, 63, -2, 2, 63, 2, GRASS).fill(-2, 63, 3, 2, 63, 8, FARMLAND).spawn(0.5, 64, 0.5, 0, 0)
                .run(3, 0).run(25, FORWARD).run(25, BACK));
        l.add(new Scenario("carpet_and_snow_steps").desc("walk over carpet, snow layers and a cake")
                .fill(-2, 63, -2, 2, 63, 14, STONE).block(0, 64, 2, CARPET, 3).block(0, 64, 4, SNOW_LAYER, 3)
                .block(0, 64, 6, SNOW_LAYER, 6).block(0, 64, 8, CAKE, 0).block(0, 64, 10, ENCHANTING_TABLE)
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(50, FORWARD));

        // ---- climbing ----------------------------------------------------------------------------
        l.add(new Scenario("ladder_climb").desc("walk into a ladder, climb, stop (hold), descend")
                .fill(-2, 63, -2, 2, 63, 4, STONE).fill(0, 64, 3, 0, 72, 3, STONE).fill(0, 64, 2, 0, 71, 2, LADDER, 2)
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(40, FORWARD).run(10, SNEAK).run(25, 0));
        l.add(new Scenario("ladder_jump_climb").desc("jump onto a ladder and climb with jump held")
                .fill(-2, 63, -2, 2, 63, 4, STONE).fill(-1, 64, 3, 1, 70, 3, STONE).fill(0, 64, 2, 0, 69, 2, LADDER, 2)
                .spawn(0.5, 64, 0.5, 5, 0).run(3, 0).run(30, FORWARD | JUMP).run(10, 0));
        l.add(new Scenario("ladder_side_facing").desc("ladder facing west, approached from the side")
                .fill(-3, 63, -3, 3, 63, 3, STONE).fill(-1, 64, 0, -1, 70, 0, STONE).fill(0, 64, 0, 0, 69, 0, LADDER, 5)
                .spawn(1.5, 64, 0.5, 90, 0).run(3, 0).run(35, FORWARD).run(15, 0));
        l.add(new Scenario("vine_climb").desc("climb a vine on a wall")
                .fill(-2, 63, -2, 2, 63, 4, STONE).fill(-1, 64, 3, 1, 72, 3, STONE).fill(0, 64, 2, 0, 71, 2, VINE, 1)
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(40, FORWARD).run(15, 0));

        // ---- shapes: fences, panes, walls, stairs ---------------------------------------------------
        l.add(flat("fence_line").desc("walk and jump against a fence line (1.5 high)")
                .fill(-4, 64, 3, 4, 64, 3, FENCE).spawn(0.5, 64, 0.5, 10, 0).run(3, 0).run(15, FORWARD)
                .run(20, FORWARD | JUMP));
        l.add(flat("fence_corner_gap").desc("squeeze along fence posts and a gate")
                .block(-1, 64, 3, FENCE).block(1, 64, 3, FENCE).block(0, 64, 3, FENCE_GATE, 0).block(2, 64, 3, STONE)
                .spawn(0.5, 64, 0.5, -20, 0).run(3, 0).run(30, FORWARD).run(10, LEFT));
        l.add(flat("glass_panes").desc("walk into a glass pane wall and along single panes")
                .fill(-4, 64, 3, 4, 65, 3, GLASS_PANE).block(-2, 64, 0, GLASS_PANE).block(-2, 64, 1, GLASS_PANE)
                .spawn(0.5, 64, 0.5, 25, 0).run(3, 0).run(30, FORWARD).run(15, RIGHT));
        l.add(flat("iron_bars_cross").desc("run into a cross of iron bars")
                .fill(-3, 64, 3, 3, 64, 3, IRON_BARS).fill(0, 64, 0, 0, 64, 6, IRON_BARS)
                .spawn(1.5, 64, 0.5, 15, 0).run(3, 0).run(30, FORWARD | SPRINT));
        l.add(flat("cobblestone_wall").desc("walk against a cobblestone wall and jump")
                .fill(-3, 64, 3, 3, 64, 3, COBBLESTONE_WALL).block(0, 64, 4, COBBLESTONE_WALL)
                .spawn(0.5, 64, 0.5, -10, 0).run(3, 0).run(15, FORWARD).run(15, FORWARD | JUMP));
        l.add(flat("stairs_corners").desc("walk over inner/outer stair corners")
                .block(0, 64, 3, OAK_STAIRS, 2).block(1, 64, 3, OAK_STAIRS, 1).block(-1, 64, 3, OAK_STAIRS, 3)
                .block(0, 64, 5, STONE_STAIRS, 6).block(1, 64, 5, STONE_STAIRS, 0)
                .spawn(0.5, 64, 0.5, -15, 0).run(3, 0).run(40, FORWARD));

        // ---- creative flight ------------------------------------------------------------------------
        l.add(flat("creative_flight_toggle").desc("double-tap jump to fly, rise, sneak down, land")
                .mode(Scenario.GameMode.CREATIVE).spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(2, 0)
                .run(1, JUMP).run(15, JUMP).run(10, 0).run(30, SNEAK).run(5, 0));
        l.add(flat("creative_flight_sprint").desc("sprint-fly forward, diagonal, and stop in the air")
                .mode(Scenario.GameMode.CREATIVE).flying(true).spawn(0.5, 70, 0.5, 33, -10).run(3, 0)
                .run(25, FORWARD | SPRINT).run(10, FORWARD | LEFT | SPRINT).run(15, 0).run(10, FORWARD | JUMP | SNEAK));
        l.add(flat("creative_flight_into_wall").desc("fly into a wall while rising")
                .fill(-5, 64, 5, 5, 80, 5, STONE).mode(Scenario.GameMode.CREATIVE).flying(true)
                .spawn(0.5, 66, 0.5, 20, 0).run(3, 0).run(30, FORWARD | JUMP | SPRINT));
        l.add(flat("creative_double_tap_off").desc("toggle flight off with a double tap mid-air")
                .mode(Scenario.GameMode.CREATIVE).flying(true).spawn(0.5, 70, 0.5, 0, 0).run(3, 0).run(1, JUMP)
                .run(3, 0).run(1, JUMP).run(30, 0));
        l.add(flat("survival_double_tap").desc("survival: double-tap jump does not fly")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(2, 0).run(1, JUMP).run(20, 0));
        Scenario flySpeed = flat("fly_speed_custom").desc("custom fly speed 0.1 (abilities packet)")
                .mode(Scenario.GameMode.CREATIVE).flying(true).spawn(0.5, 70, 0.5, 0, 0);
        flySpeed.flySpeed = 0.1f;
        l.add(flySpeed.run(3, 0).run(20, FORWARD | JUMP).run(10, 0));

        // ---- liquids ----------------------------------------------------------------------------------
        l.add(new Scenario("water_walk_swim").desc("walk into still water, swim up, float")
                .fill(-3, 60, -3, 3, 60, 12, STONE).fill(-3, 61, 4, 3, 63, 12, WATER).fill(-3, 61, -3, 3, 63, 3, STONE)
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, FORWARD).run(25, FORWARD | JUMP).run(20, 0));
        l.add(new Scenario("water_fall_in").desc("fall into deep water")
                .fill(-2, 50, -2, 2, 50, 2, STONE).fill(-2, 51, -2, 2, 58, 2, WATER).spawn(0.5, 66, 0.5, 0, 0)
                .run(40, 0).run(20, JUMP));
        l.add(new Scenario("lava_basic").desc("walk into lava and jump in it")
                .fill(-3, 60, -3, 3, 60, 12, STONE).fill(-3, 61, -3, 3, 63, 3, STONE).fill(-3, 61, 4, 3, 63, 12, LAVA)
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, FORWARD).run(20, FORWARD | JUMP));

        // ---- chunks, coordinates, server packets ----------------------------------------------------------
        l.add(new Scenario("walk_into_unloaded").desc("walk off loaded terrain into an unloaded chunk")
                .outside(Scenario.Outside.UNLOADED).fill(0, 63, 0, 15, 63, 15, STONE)
                .spawn(8.5, 64, 12.5, 0, 0).run(3, 0).run(30, FORWARD | SPRINT).run(10, 0));
        l.add(new Scenario("spawn_in_unloaded").desc("spawn above an unloaded chunk (gravity -0.1)")
                .outside(Scenario.Outside.UNLOADED).fill(16, 63, 0, 31, 63, 15, STONE).spawn(5.5, 70, 5.5, 0, 0)
                .run(15, 0));
        l.add(flat("far_coordinates", 29_999_000, -29_999_000).desc("walk and sprint-jump near the world border")
                .spawn(29_999_000.5, 64, -29_999_000.5, -135, 0).run(3, 0).run(30, FORWARD | SPRINT | JUMP));
        l.add(flat("world_border", 29_999_990, 0).desc("walk and sprint-jump into the world border at x = 30,000,000 (collides like stone)")
                .spawn(29_999_996.5, 64, 0.5, -90, 0).run(3, 0).run(20, FORWARD | SPRINT).run(10, FORWARD | SPRINT | JUMP)
                .turn(10, FORWARD, 9, 0));
        l.add(flat("negative_coordinates", -1000, -1000).desc("walk across chunk borders at negative coords")
                .spawn(-1000.3, 64, -1000.7, 135.5f, 0).run(3, 0).run(40, FORWARD | SPRINT));
        l.add(flat("velocity_packet").desc("knockback velocity packets while walking")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(5, FORWARD).event(FORWARD, new Scenario.Velocity(3200, 3000, -1600))
                .run(20, FORWARD).event(0, new Scenario.Velocity(-8000, 2000, 0)).run(20, 0));
        l.add(flat("teleport_packet").desc("server teleport while sprinting")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(10, FORWARD | SPRINT)
                .event(FORWARD | SPRINT, new Scenario.Teleport(5.5, 64, -3.5, 180, 10)).run(15, FORWARD | SPRINT));
        l.add(flat("abilities_packet").desc("server grants flight mid-jump, then revokes it")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(1, JUMP).run(2, 0)
                .event(0, new Scenario.Abilities(false, true, true, false, 0.05f, 0.1f)).run(10, JUMP)
                .event(0, new Scenario.Abilities(false, false, false, false, 0.05f, 0.1f)).run(25, 0));

        // ---- attributes and effects -------------------------------------------------------------------------
        Scenario speed = flat("speed_attribute").desc("movement speed base 0.13 (as sent by entity properties)")
                .spawn(0.5, 64, 0.5, 0, 0);
        speed.movementSpeedBase = 0.13;
        l.add(speed.run(3, 0).run(20, FORWARD).run(20, FORWARD | SPRINT | JUMP));
        l.add(flat("blindness_no_sprint").desc("blindness (client-side effect) blocks sprinting").effect(15, 0)
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(15, FORWARD | SPRINT));
        l.add(flat("push_out_of_blocks").desc("spawned overlapping a wall: pushOutOfBlocks nudges the player out")
                .fill(1, 64, -2, 1, 65, 2, STONE).spawn(1.15, 64, 0.5, 0, 0).run(15, 0).run(10, LEFT));
        l.add(flat("jump_boost").desc("jump boost II (client-side effect)").effect(8, 1)
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(30, JUMP | FORWARD));

        // ---- item use (use key with a sword: blocking) -- appended last so earlier cases keep their lines
        l.add(flat("use_item_walk").desc("walk, block with a sword (input x0.2), sprint and jump while blocking, release")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(8, FORWARD).run(12, FORWARD | Scenario.USE)
                .run(8, FORWARD | SPRINT | Scenario.USE).run(10, FORWARD | SPRINT | JUMP | Scenario.USE)
                .run(10, FORWARD | SPRINT).run(5, 0));
        l.add(flat("use_item_sprint_cancel").desc("sprinting, then start blocking: forward < 0.8 stops the sprint")
                .spawn(0.5, 64, 0.5, 30, 0).run(3, 0).run(10, FORWARD | SPRINT).run(10, FORWARD | SPRINT | Scenario.USE)
                .run(3, FORWARD | Scenario.USE).run(10, FORWARD | SPRINT));
        l.add(flat("use_item_double_tap").desc("double-tap forward while blocking (toggle timer reset), then without")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(2, FORWARD | Scenario.USE).run(2, Scenario.USE)
                .run(10, FORWARD | Scenario.USE).run(2, 0).run(2, FORWARD).run(2, 0).run(10, FORWARD));
        l.add(flat("use_item_sneak_strafe").desc("sneak + strafe while blocking, in the air too")
                .spawn(0.5, 64, 0.5, -60, 0).run(3, 0).run(10, LEFT | SNEAK | Scenario.USE)
                .run(12, FORWARD | RIGHT | JUMP | Scenario.USE).run(5, 0));

        // ---- landing step-up (step-up also runs when the move lands: dy clipped and dy < 0) -----------
        // Appended last so earlier cases keep their lines. Bunny hops (no sprint) into an edge that is a
        // little higher than the block the player lands on.
        l.add(new Scenario("landing_step_soul_sand_to_grass").desc("bunny-hop over soul sand into a grass edge 1/8 higher")
                .fill(-8, 61, -8, 31, 62, 40, DIRT).fill(-8, 63, -8, 31, 63, 40, GRASS).fill(16, 63, 8, 23, 63, 23, SOUL_SAND)
                .spawn(18.23573647601667, 63.875, 21.13838376006911, 2.4791226f, 10.0044775f).run(30, FORWARD | JUMP).run(5, 0));
        l.add(new Scenario("landing_step_slab_to_block").desc("bunny-hop over bottom slabs into a full block edge")
                .fill(-4, 62, -4, 4, 62, 12, STONE).fill(-4, 63, -4, 4, 63, 5, STONE_SLAB, 0).fill(-4, 63, 6, 4, 63, 12, STONE)
                .spawn(0.5, 63.5, 1.95, -3.5f, 0).run(40, FORWARD | JUMP).run(5, 0));
        l.add(new Scenario("landing_step_snow_layers").desc("hop over 2-layer snow into 4-layer snow and a full block")
                .fill(-4, 62, -4, 4, 62, 14, STONE).fill(-4, 63, -4, 4, 63, 4, SNOW_LAYER, 1).fill(-4, 63, 5, 4, 63, 9, SNOW_LAYER, 3)
                .fill(-4, 63, 10, 4, 63, 14, STONE).spawn(0.5, 63.25, 0.8, 5, 0).run(45, FORWARD | JUMP).run(5, 0));
        l.add(new Scenario("farmland_hop_into_grass").desc("sprint-hop over farmland into grass (farmland collides as a full block in 1.8.9: no step)")
                .fill(-4, 62, -4, 4, 62, 14, DIRT).fill(-4, 63, -4, 4, 63, 6, FARMLAND).fill(-4, 63, 7, 4, 63, 14, GRASS)
                .spawn(0.5, 63.9375, 0.5, 0, 0).run(30, FORWARD | SPRINT | JUMP).run(5, 0));
        l.add(new Scenario("landing_step_carpet_ledge").desc("fall from a ledge onto carpet next to a slab wall, walking into it")
                .fill(-4, 62, -4, 4, 62, 10, STONE).fill(-4, 63, -4, 4, 65, 0, STONE).fill(-4, 63, 1, 4, 63, 5, CARPET, 0)
                .fill(-4, 63, 6, 4, 63, 10, STONE_SLAB, 0).spawn(0.5, 66, 0.3, 0, 0).run(30, FORWARD).run(15, FORWARD | JUMP));
        landingVariants(l);
        extras(l);
        return l;
    }

    // ---- server attribute modifiers, depth strider, spectator (appended last) ------------------------

    private static void extras(List<Scenario> l) {
        Scenario speed2 = flat("speed_modifier_op2_sprint").desc("speed II as an attribute modifier (op 2) next to the client's sprint modifier")
                .spawn(0.5, 64, 0.5, 0, 0);
        speed2.speedModifiers.add(new double[] {0.4000000059604645, 2});
        l.add(speed2.run(3, 0).run(15, FORWARD).run(20, FORWARD | SPRINT).run(15, FORWARD | SPRINT | JUMP).run(10, 0));
        Scenario ops = flat("speed_modifiers_ops").desc("modifiers with operations 0, 1 and 2 at once, walking and sprinting")
                .spawn(0.5, 64, 0.5, 20, 0);
        ops.speedModifiers.add(new double[] {0.05, 0});
        ops.speedModifiers.add(new double[] {0.3, 1});
        ops.speedModifiers.add(new double[] {0.2, 2});
        l.add(ops.run(3, 0).run(15, FORWARD).run(20, FORWARD | SPRINT | JUMP).run(10, 0));
        Scenario slow = flat("slowness_modifier").desc("slowness I as an attribute modifier (op 2, -0.15)").spawn(0.5, 64, 0.5, 0, 0);
        slow.speedModifiers.add(new double[] {-0.15000000596046448, 2});
        l.add(slow.run(3, 0).run(20, FORWARD | SPRINT).run(10, FORWARD | SPRINT | JUMP));
        for (int level = 1; level <= 3; level++) {
            Scenario ds = new Scenario("depth_strider_" + level).fill(-3, 60, -3, 3, 60, 20, STONE).fill(-3, 61, -3, 3, 62, 20, WATER)
                    .desc("walk on the bottom of 2-deep water with depth strider " + level + ", then swim up")
                    .spawn(0.5, 61, 0.5, 0, 0);
            ds.depthStrider = level;
            l.add(ds.run(5, 0).run(30, FORWARD).run(15, FORWARD | JUMP).run(10, FORWARD | SPRINT));
        }
        l.add(flat("spectator_noclip").desc("spectator: flight without collision through a wall and the floor")
                .fill(-5, 64, 4, 5, 70, 4, STONE).mode(Scenario.GameMode.SPECTATOR).flying(true)
                .spawn(0.5, 65, 0.5, 0, 0).run(3, 0).run(20, FORWARD).run(15, SNEAK).run(10, FORWARD | JUMP));
    }

    // ---- tuned landing step-up variants ------------------------------------------------------------

    /** Builds one candidate of a family for a spawn offset along the walking direction (+z). */
    private interface Family {
        Scenario build(double dz, float yaw);
    }

    /**
     * Picks the first (yaw, spawn offset) - yaw 0, 6, ..., 30 degrees, dz = 0, 0.05, ..., 3.95 - for which vanilla performs at
     * least one landing step-up: falling into the move (velocityY < 0, not on ground before) and ending
     * on the ground higher than it started. The search runs the real harness, so it is deterministic;
     * the chosen offset is part of the case (its spawn), the search is only how it was found.
     */
    private static Scenario tuned(Family f) {
        for (int y = 0; y <= 5; y++) {
            for (int k = 0; k < 80; k++) {
                Scenario sc = f.build(0.05 * k, 6.0f * y);
                if (hasLandingStep(sc)) {
                    return sc;
                }
            }
        }
        throw new IllegalStateException("no landing step-up found for " + f.build(0, 0).name);
    }

    static boolean hasLandingStep(Scenario sc) {
        Harness.Run run = new Harness.Run(sc);
        boolean found = false;
        for (Scenario.Tick t : sc.ticks) {
            double y0 = run.player.y, vy0 = run.player.velocityY;
            boolean ground0 = run.player.onGround;
            run.step(t);
            if (!ground0 && vy0 < 0 && run.player.onGround && run.player.y > y0) {
                found = true;
                break;
            }
        }
        Harness.releaseKeys();
        return found;
    }

    /** A strip world: [floor block (lower), edge block (higher)] crossed along +z, on stone at y 62. */
    private static Scenario strip(String name, int low, int lowMeta, int high, int highMeta) {
        Scenario sc = new Scenario(name).fill(-4, 61, -4, 4, 62, 30, STONE).fill(-4, 63, -4, 4, 63, 9, low, lowMeta)
                .fill(-4, 63, 10, 4, 63, 30, high, highMeta);
        if (low == FARMLAND) {
            sc.fill(-4, 64, 10, 4, 64, 30, STONE_SLAB, 0);
        }
        return sc;
    }

    /**
     * Walking (if no walking case lands at the edge: sprinting, hopping, sprint-hopping) off a ledge of height h onto a strip
     * of the low block that ends at the high block. The strip width (1..4 blocks) is searched as well,
     * so that the landing tick is the tick the player reaches the edge.
     */
    private static Scenario walkFall(String name, int h, int low, int lowMeta, int high, int highMeta, String desc) {
        for (int keys : new int[] {FORWARD, FORWARD | SPRINT, FORWARD | JUMP, FORWARD | SPRINT | JUMP}) {
            for (int w = 1; w <= 4; w++) {
                final int fw = w;
                final String how = (keys & SPRINT) != 0 ? ((keys & JUMP) != 0 ? "sprint-jumping" : "sprinting")
                        : ((keys & JUMP) != 0 ? "bunny-hopping" : "walking");
                try {
                    return tuned((dz, yaw) -> strip(name, low, lowMeta, high, highMeta)
                            .fill(-4, 63, -4, 4, 62 + h, 9 - fw, STONE).desc(desc + ", " + how + ", strip " + fw + " wide")
                            .spawn(0.5, 63 + h, 9.0 - fw - 2.5 + dz, yaw, 0).run(40, keys).run(5, 0));
                } catch (IllegalStateException e) {
                    // try a wider strip / sprinting
                }
            }
        }
        throw new IllegalStateException("no landing step-up found for " + name);
    }

    private static void landingVariants(List<Scenario> l) {
        // Soul sand -> grass with different arrivals: sprint-jump, plain walk off a ledge (falls of 1-3).
        l.add(tuned((dz, yaw) -> strip("landing_soul_sand_sprint_jump", SOUL_SAND, 0, GRASS, 0)
                .desc("sprint-jump over soul sand into a grass edge (landing step-up)")
                .spawn(0.5, 63.875, 3.0 + dz, yaw, 0).run(45, FORWARD | SPRINT | JUMP).run(5, 0)));
        for (int h = 1; h <= 3; h++) {
            l.add(walkFall("landing_soul_sand_walk_fall_" + h, h, SOUL_SAND, 0, GRASS, 0,
                    "walk off a " + h + "-block ledge onto soul sand against a grass edge (landing step-up)"));
        }
        // Snow layers k (1..7) -> an edge at most 0.5 higher (step height is 0.6): a full block for k >= 4,
        // k + 4 layers below that. Bunny hops.
        for (int k = 1; k <= 7; k++) {
            final int meta = k - 1;
            final int layers = k;
            final int highId = k >= 4 ? STONE : SNOW_LAYER;
            final int highMeta = k >= 4 ? 0 : k + 3;
            final String high = k >= 4 ? "a full block" : (k + 4) + " snow layers";
            l.add(tuned((dz, yaw) -> strip("landing_snow_" + layers, SNOW_LAYER, meta, highId, highMeta)
                    .desc("bunny-hop over " + layers + " snow layer(s) into " + high + " (landing step-up)")
                    .spawn(0.5, 63 + layers * 0.125, 3.0 + dz, yaw, 0).run(45, FORWARD | JUMP).run(5, 0)));
        }
        // Farmland (15/16) -> grass, carpet -> slab, slab -> block: falls from a 2-block ledge while walking.
        // Farmland collides as a full block in 1.8.9, so its edge is a block with a bottom slab on top.
        l.add(walkFall("landing_farmland_walk_fall", 2, FARMLAND, 0, STONE, 0,
                "walk off a 2-block ledge onto farmland against a block topped with a slab (landing step-up)"));
        l.add(walkFall("landing_carpet_to_slab_fall", 2, CARPET, 0, STONE_SLAB, 0,
                "walk off a 2-block ledge onto carpet against a bottom slab (landing step-up)"));
        l.add(tuned((dz, yaw) -> strip("landing_slab_to_block_sprint_jump", STONE_SLAB, 0, STONE, 0)
                .desc("sprint-jump over bottom slabs into a full block edge (landing step-up)")
                .spawn(0.5, 63.5, 3.0 + dz, yaw, 0).run(45, FORWARD | SPRINT | JUMP).run(5, 0)));
    }
}
