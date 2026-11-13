package ghast.oracle.modern;

import ghast.oracle.Scenario;

import java.util.List;

import static ghast.oracle.Scenario.FORWARD;
import static ghast.oracle.Scenario.JUMP;
import static ghast.oracle.Scenario.LEFT;
import static ghast.oracle.Scenario.SNEAK;
import static ghast.oracle.Scenario.SPRINT;

/**
 * Scenarios for the collision restitution of 26.2 ({@code Entity.restituteMovementAfterCollisions}:
 * bounciness, block bounce restitution, gravity / drag compensation, {@code suppresses_bounce}), the
 * movement attributes that came with it ({@code friction_modifier}, {@code air_drag_modifier},
 * {@code bounciness}) and the blocks 26.2 added. Appended to {@link Scenarios#all()} only on versions
 * that have these (feature-detected), so earlier versions' golden files are unchanged.
 */
final class ScenariosRestitution {
    private ScenariosRestitution() {}

    private static int s(String spec) {
        return States.id(spec);
    }

    private static Scenario flat(String name) {
        return new Scenario(name).fill(-16, 63, -16, 16, 63, 16, s("stone"), 0);
    }

    /** Superflat floor, a 9 x 9 patch of {@code floor} at y 3 (a bed: its foot at (28, 4, 18)), idle drop. */
    private static Scenario superflatBounce(String name, String floor, int drop, double vx, double vz) {
        Scenario sc = new Scenario(name).fill(12, 0, 2, 44, 2, 34, s("stone"), 0).fill(12, 3, 2, 44, 3, 34, s("grass_block"), 0);
        double feet;
        if (floor.startsWith("white_bed")) {
            sc.block(28, 4, 18, s(floor), 0);
            feet = 4 + 0.5625;
        } else {
            sc.fill(24, 3, 14, 32, 3, 22, s(floor), 0);
            feet = 4;
        }
        sc.spawn(28.5, feet + drop, 18.5, 0, 0);
        if (vx != 0 || vz != 0) {
            sc.velocity(vx, 0, vz);
        }
        return sc.run(drop > 20 ? 700 : 500, 0);
    }

    private static Scenario attr(Scenario sc, String attribute, double base) {
        sc.attributeBases.put("minecraft:" + attribute, base);
        return sc;
    }

    static void add(List<Scenario> l) {
        if (!Reg.hasRestitution()) {
            return;
        }
        // ---- bounciness attribute: every colliding axis is reflected -------------------------------
        l.add(attr(flat("bounciness_wall").fill(-3, 64, 6, 3, 67, 6, s("stone"), 0), "bounciness", 0.5)
                .desc("bounciness 0.5: sprint into a wall (reflected), sprint-jump into it again")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, FORWARD | SPRINT).run(10, 0).run(20, FORWARD | SPRINT | JUMP));
        l.add(attr(flat("bounciness_fall"), "bounciness", 0.6)
                .desc("bounciness 0.6: fall 10 blocks onto stone and bounce until the bounce is below gravity")
                .spawn(0.5, 74, 0.5, 0, 0).run(60, 0));
        l.add(attr(flat("bounciness_full"), "bounciness", 1.0)
                .desc("bounciness 1.0: fall onto stone (gravity compensation keeps the height), walk while bouncing")
                .spawn(0.5, 70, 0.5, 0, 0).run(30, 0).run(30, FORWARD));
        l.add(attr(flat("bounciness_ceiling").fill(-3, 66, -3, 3, 66, 3, s("stone"), 0), "bounciness", 0.8)
                .desc("bounciness 0.8: jump into a low ceiling (reflected downwards), land (bounces)")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(25, JUMP));
        l.add(attr(flat("bounciness_sneak").fill(-3, 64, 6, 3, 66, 6, s("stone"), 0), "bounciness", 1.0)
                .desc("bounciness 1.0 while sneaking: no bounce (isSuppressingBounce)")
                .spawn(0.5, 68, 0.5, 0, 0).run(20, SNEAK).run(25, FORWARD | SNEAK));
        l.add(attr(flat("bounciness_honey").fill(-2, 63, -2, 2, 63, 2, s("honey_block"), 0), "bounciness", 0.7)
                .desc("bounciness 0.7 onto honey (suppresses_bounce tag): no bounce on landing")
                .spawn(0.5, 70, 0.5, 0, 0).run(30, 0));
        Scenario slowFall = attr(flat("bounciness_slow_falling"), "bounciness", 0.9)
                .desc("bounciness 0.9 with slow falling: the effective gravity 0.01 decides when bouncing stops")
                .spawn(0.5, 66, 0.5, 0, 0);
        slowFall.effect(Scenarios.eff("slow_falling"), 0);
        l.add(slowFall.run(80, 0));

        // ---- block bounce restitution (slime 1, beds 0.75) --------------------------------------------
        l.add(flat("slime_ceiling").fill(-3, 66, -3, 3, 66, 3, s("slime_block"), 0)
                .desc("jump into a slime ceiling (26.2: the vertical axis is restituted like any block's)")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, JUMP));
        l.add(flat("slime_low_drop").fill(-2, 63, -2, 2, 63, 2, s("slime_block"), 0)
                .desc("drop from just above slime: falling slower than gravity, no bounce")
                .spawn(0.5, 64.05, 0.5, 0, 0).run(15, 0).run(10, FORWARD));
        l.add(flat("slime_sneak_land").fill(-2, 63, -2, 2, 63, 2, s("slime_block"), 0)
                .desc("fall onto slime while sneaking (no bounce), then release sneak and jump")
                .spawn(0.5, 72, 0.5, 0, 0).run(30, SNEAK).run(25, JUMP));
        l.add(attr(flat("slime_bounciness").fill(-2, 63, -2, 2, 63, 2, s("slime_block"), 0), "bounciness", 0.4)
                .desc("bounciness 0.4 onto slime: the larger of entity and block restitution")
                .spawn(0.5, 72, 0.5, 0, 0).run(50, 0));
        l.add(flat("bed_bounce_high").block(0, 64, 0, s("white_bed[facing=south,part=foot]"), 0)
                .block(0, 64, 1, s("white_bed[facing=south,part=head]"), 0)
                .desc("fall 12 blocks onto a bed (restitution 0.75), bounce out sideways")
                .spawn(0.5, 77, 0.5, 0, 0).run(30, 0).run(20, FORWARD | LEFT));

        // Long idle bounces on a superflat floor (stone y 0-2, grass y 3, the lab's layout) until the
        // bounce dies out: the tail lands with moves below the position-packet threshold, which decides
        // whether a tick sends a position or only the on-ground flag. Slime drops of 6-30 blocks, the
        // 20-block drop with a tiny horizontal drift, and a 20-block drop onto a bed.
        for (int drop = 6; drop <= 30; drop += 2) {
            l.add(superflatBounce("slime_drop_" + drop + "_long", "slime_block", drop, 0, 0)
                    .desc("idle drop of " + drop + " blocks onto a slime floor (superflat), bouncing until it dies out"));
        }
        l.add(superflatBounce("slime_drop_20_drift", "slime_block", 20, 0.003, 0.002)
                .desc("idle drop of 20 blocks onto a slime floor with a tiny horizontal drift (0.003, 0.002)"));
        l.add(superflatBounce("bed_drop_20_long", "white_bed[facing=south,part=foot]", 20, 0, 0)
                .block(28, 4, 19, s("white_bed[facing=south,part=head]"), 0)
                .desc("idle drop of 20 blocks onto a bed (superflat), bouncing until it dies out"));

        // ---- friction_modifier / air_drag_modifier -----------------------------------------------------
        l.add(attr(flat("friction_modifier_high"), "friction_modifier", 2.0)
                .desc("friction_modifier 2.0 on stone: friction 0.2 (no friction scaling of the walk speed below 0.6)")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, FORWARD).run(15, 0).run(15, FORWARD | SPRINT | JUMP));
        l.add(attr(new Scenario("friction_modifier_ice").fill(-16, 63, -16, 16, 63, 16, s("ice"), 0), "friction_modifier", 0.5)
                .desc("friction_modifier 0.5 on ice (friction 0.99)").spawn(0.5, 64, 0.5, 0, 0)
                .run(3, 0).run(20, FORWARD | SPRINT).run(30, 0));
        l.add(attr(new Scenario("friction_modifier_zero").fill(-16, 63, -16, 16, 63, 16, s("stone"), 0), "friction_modifier", 0.0)
                .desc("friction_modifier 0: frictionless ground").spawn(0.5, 64, 0.5, 0, 0)
                .run(3, 0).run(5, FORWARD).run(30, 0));
        l.add(attr(flat("air_drag_low"), "air_drag_modifier", 0.5)
                .desc("air_drag_modifier 0.5: less drag; sprint-jumps and a fall").spawn(0.5, 64, 0.5, 0, 0)
                .run(3, 0).run(30, FORWARD | SPRINT | JUMP).run(5, 0));
        l.add(attr(flat("air_drag_high"), "air_drag_modifier", 3.0)
                .desc("air_drag_modifier 3.0 (horizontal drag 0.73, vertical 0.94)").spawn(0.5, 70, 0.5, 0, 0)
                .run(25, FORWARD).run(25, FORWARD | SPRINT | JUMP));
        Scenario both = attr(attr(attr(flat("modifiers_all").fill(-3, 64, 7, 3, 66, 7, s("stone"), 0),
                "air_drag_modifier", 0.25), "friction_modifier", 1.5), "bounciness", 0.3)
                .desc("all three: bouncing off a wall and the floor with modified drags").spawn(0.5, 68, 0.5, 0, 0);
        l.add(both.run(10, 0).run(30, FORWARD | SPRINT | JUMP));

        // ---- blocks added in 26.2 --------------------------------------------------------------------
        if (Reg.hasBlock("sulfur_spike")) {
            l.add(flat("sulfur_blocks")
                    .fill(-1, 64, 2, 1, 64, 2, s("sulfur_slab[type=bottom]"), 0)
                    .fill(-1, 64, 3, 1, 64, 3, s("cinnabar_stairs[facing=south,half=bottom]"), 0)
                    .fill(-1, 65, 4, 1, 65, 4, s("polished_sulfur"), 0)
                    .fill(-1, 64, 5, 1, 64, 5, s("polished_sulfur"), 0)
                    .fill(-1, 66, 6, 1, 66, 6, s("sulfur_brick_wall"), 0)
                    .fill(2, 64, 2, 2, 64, 6, s("potent_sulfur"), 0)
                    .desc("walk up sulfur / cinnabar slabs, stairs, blocks; jump at a sulfur brick wall")
                    .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(25, FORWARD).run(15, FORWARD | JUMP));
            l.add(flat("sulfur_spikes")
                    .block(0, 64, 2, s("sulfur_spike[vertical_direction=up,thickness=tip]"), 0)
                    .block(1, 64, 3, s("sulfur_spike[vertical_direction=up,thickness=frustum]"), 0)
                    .block(1, 65, 3, s("sulfur_spike[vertical_direction=up,thickness=tip]"), 0)
                    .block(0, 66, 4, s("sulfur_spike[vertical_direction=down,thickness=tip]"), 0)
                    .desc("walk and jump through sulfur spikes (offset shapes)")
                    .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(20, FORWARD).run(15, FORWARD | JUMP | LEFT));
        }
    }
}
