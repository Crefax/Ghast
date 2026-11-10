package ghast.oracle.v26_1;

import ghast.oracle.Scenario;

import java.util.List;

import static ghast.oracle.Scenario.FORWARD;
import static ghast.oracle.Scenario.JUMP;
import static ghast.oracle.Scenario.LEFT;
import static ghast.oracle.Scenario.SNEAK;
import static ghast.oracle.Scenario.SPRINT;

/** Scenarios for 26.1 movement mechanics that 1.16.5 does not have (appended to {@link Scenarios#all()}). */
final class Scenarios26 {
    private Scenarios26() {}

    private static int s(String spec) {
        return States.id(spec);
    }

    private static Scenario flat(String name) {
        return new Scenario(name).fill(-16, 63, -16, 16, 63, 16, s("stone"), 0);
    }

    static void add(List<Scenario> l) {
        // ---- powder snow (1.17) ----------------------------------------------------------------------
        l.add(flat("powder_snow_walk").fill(-2, 64, 3, 2, 65, 8, s("powder_snow"), 0)
                .desc("walk into powder snow (no leather boots: sink in, slowed), jump inside, walk out")
                .spawn(0.5, 64, 0.5, 0, 0).run(3, 0).run(30, FORWARD).run(15, FORWARD | JUMP).run(20, FORWARD));
        l.add(flat("powder_snow_fall").fill(-2, 64, -2, 2, 64, 2, s("powder_snow"), 0)
                .desc("fall onto powder snow from 6 blocks (falls through, fall distance reset inside)")
                .spawn(0.5, 71, 0.5, 0, 0).run(40, 0));

        // ---- movement attributes (1.20.5+: step height, gravity, jump strength, sneaking speed, scale) ----
        Scenario step = flat("attr_step_height").fill(-3, 64, 4, 3, 64, 4, s("stone"), 0)
                .desc("step_height attribute 1.0: walk up a full block without jumping").spawn(0.5, 64, 0.5, 0, 0);
        step.attributeBases.put("minecraft:step_height", 1.0);
        l.add(step.run(3, 0).run(25, FORWARD));
        Scenario grav = flat("attr_gravity").desc("gravity attribute 0.04: jump and fall slowly").spawn(0.5, 64, 0.5, 0, 0);
        grav.attributeBases.put("minecraft:gravity", 0.04);
        l.add(grav.run(3, 0).run(1, JUMP).run(40, FORWARD));
        Scenario jump = flat("attr_jump_strength").desc("jump_strength attribute 0.6 (sprint-jump)").spawn(0.5, 64, 0.5, 0, 0);
        jump.attributeBases.put("minecraft:jump_strength", 0.6);
        l.add(jump.run(3, 0).run(30, FORWARD | SPRINT | JUMP));
        Scenario sneak = flat("attr_sneaking_speed").desc("sneaking speed as swift sneak III sends it (+0.45)")
                .spawn(0.5, 64, 0.5, 0, 0);
        sneak.attributeModifiers.add(new Scenario.AttributeModifierSpec("minecraft:sneaking_speed",
                "minecraft:enchantment.swift_sneak", (double) 0.45F, 0));
        l.add(sneak.run(3, 0).run(25, FORWARD | SNEAK).run(10, FORWARD | LEFT | SNEAK));
        Scenario small = flat("attr_scale_small").fill(-3, 65, 3, 3, 65, 9, s("stone"), 0)
                .desc("scale 0.5: a 0.9 x 0.3 player walks under a 1-high gap").spawn(0.5, 64, 0.5, 0, 0);
        small.attributeBases.put("minecraft:scale", 0.5);
        l.add(small.run(3, 0).run(30, FORWARD | SPRINT).run(5, JUMP));
        Scenario big = flat("attr_scale_big").fill(-3, 64, 5, 3, 64, 5, s("stone"), 0)
                .desc("scale 2.0: bigger box, step-up and jump scale with it").spawn(0.5, 64, 0.5, 0, 0);
        big.attributeBases.put("minecraft:scale", 2.0);
        l.add(big.run(3, 0).run(25, FORWARD).run(10, FORWARD | JUMP));
        Scenario eff = new Scenario("attr_movement_efficiency").fill(-3, 63, -3, 3, 63, 20, s("soul_sand"), 0)
                .desc("soul speed III on soul sand as the server sends it: movement_efficiency +1, movement speed +0.0615")
                .spawn(0.5, 64, 0.5, 0, 0);
        eff.soulSpeed = 3;
        eff.attributeModifiers.add(new Scenario.AttributeModifierSpec("minecraft:movement_efficiency",
                "minecraft:enchantment.soul_speed", 1.0, 0));
        eff.speedModifiers.add(new double[] {(double) (0.0405F + 0.0105F * 2), 0});
        l.add(eff.run(3, 0).run(25, FORWARD | SPRINT).run(10, 0));
        Scenario water = new Scenario("attr_water_movement_efficiency").fill(-3, 60, -3, 3, 60, 20, s("stone"), 0)
                .fill(-3, 61, -3, 3, 62, 20, s("water"), 0)
                .desc("walk on the bottom with depth strider III (water_movement_efficiency from the boots)")
                .spawn(0.5, 61, 0.5, 0, 0);
        water.depthStrider = 3;
        l.add(water.run(5, 0).run(30, FORWARD));

        // ---- sprinting rules (1.21.x) ----------------------------------------------------------------
        l.add(new Scenario("sprint_shallow_water").fill(-3, 62, -3, 3, 62, 20, s("stone"), 0)
                .fill(-3, 63, 3, 3, 63, 20, s("water"), 0)
                .desc("sprint into one-deep water (no sprint start while in shallow water), keep holding")
                .spawn(0.5, 63, 0.5, 0, 0).run(3, 0).run(12, FORWARD).run(25, FORWARD | SPRINT));
        l.add(flat("sprint_minor_collision").fill(-16, 64, 1, 16, 66, 1, s("stone"), 0)
                .desc("sprint almost parallel to a wall: a minor horizontal collision keeps the sprint")
                .spawn(0.5, 64, 0.5, 85, 0).run(3, 0).run(40, FORWARD | SPRINT));
    }
}
