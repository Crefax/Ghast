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

    /**
     * Position packets with a non-zero delta movement (golden {@code teleports.jsonl.gz}, not part of
     * {@link Scenarios#all()} so the scenario golden stays unchanged).
     */
    static List<Scenario> teleports() {
        Harness.init();
        List<Scenario> l = new java.util.ArrayList<>();
        // ---- position packets with delta movement (26.1: PositionMoveRotation.deltaMovement) ----------
        int[] deltaFlags = {0, 0x20, 0x40, 0x80, 0xE0, 0x100, 0x1E0, 0x118, 0x1FF, 0x07};
        for (int f : deltaFlags) {
            Scenario t = flat("teleport_delta_" + Integer.toHexString(f))
                    .desc("position packet with delta movement (0.3, 0.42, -0.25), relative bits 0x" + Integer.toHexString(f)
                            + ", while sprinting forward")
                    .spawn(0.5, 64, 0.5, 30, 0).run(8, FORWARD | SPRINT);
            boolean relPos = (f & 7) != 0;
            t.event(FORWARD | SPRINT, new Scenario.Teleport(relPos ? 0.5 : 2.5, relPos ? 0.0 : 64.0, relPos ? -0.5 : 1.5,
                    (f & 8) != 0 ? 45.0F : 120.0F, (f & 16) != 0 ? 10.0F : -20.0F, f, 0.3, 0.42, -0.25)).run(20, FORWARD | SPRINT);
            l.add(t);
        }
        l.add(flat("teleport_delta_motion_before").desc("entity motion, then a position packet with delta (same tick)")
                .spawn(0.5, 64, 0.5, 0, 0).run(5, FORWARD)
                .event(FORWARD, new Scenario.Velocity(4000, 3000, -2000), new Scenario.Teleport(0, 0, 0, 0, 0, 0x1FF, 0.1, 0.2, 0.3))
                .run(20, FORWARD));
        l.add(flat("teleport_delta_motion_after").desc("a position packet with delta, then entity motion (same tick)")
                .spawn(0.5, 64, 0.5, 0, 0).run(5, FORWARD)
                .event(FORWARD, new Scenario.Teleport(1.5, 66, 0.5, 90, 0, 0x20, -0.2, 0.5, 0.0), new Scenario.Velocity(-4000, 2000, 1000))
                .run(25, FORWARD));
        l.add(flat("teleport_delta_rotate_falling").desc("rotate-delta teleport while falling with sideways speed")
                .spawn(0.5, 70, 0.5, 0, 0).event(0, new Scenario.Velocity(6000, 0, 0)).run(4, 0)
                .event(0, new Scenario.Teleport(0, 0, 0, 90, 0, 0x1F8, 0.0, 0.0, 0.0)).run(25, 0));

        return l;
    }

    /**
     * Joins with the level-load gate (the real {@code LevelLoadTracker}): the player is placed by the
     * login position packet, the server's {@code LEVEL_CHUNKS_LOAD_START} arrives, the player's
     * section compiles {@code delay} ticks later (or the 30 s timeout fires), then the player ticks.
     */
    static List<Scenario> joins() {
        Harness.init();
        List<Scenario> l = new java.util.ArrayList<>();
        int stone = s("stone");
        int water = s("water");
        // (name, feet y, water?)
        Object[][] places = {
            {"ground", 64.0, false}, {"float1", 65.0, false}, {"float5", 69.0, false}, {"float1_5", 65.5, false},
            {"water", 65.0, true},
        };
        for (Object[] pl : places) {
            for (int delay : new int[] {0, 1, 5, 20}) {
                Scenario sc = flat("join_" + pl[0] + "_ready" + delay).spawn(0.5, (double) pl[1], 0.5, 90, 10);
                if ((boolean) pl[2]) {
                    sc.fill(-16, 64, -16, 16, 66, 16, water, 0);
                }
                sc.delayedLoad = true;
                sc.description = "join at y " + pl[1] + (((boolean) pl[2]) ? " in water" : "")
                        + ", level-load start at tick 2, section compiled " + delay + " ticks later";
                sc.run(1, 0).event(0, new Scenario.LoadStart());
                if (delay == 0) {
                    sc.ticks.set(1, new Scenario.Tick(0, 90, 10, List.of(new Scenario.LoadStart(), new Scenario.SectionCompiled(true))));
                } else {
                    sc.run(delay - 1, 0).event(0, new Scenario.SectionCompiled(true));
                }
                sc.run(12, 0);
                l.add(sc);
            }
        }
        // Compiled before the server's load start; the timeout path; a second position packet while
        // loading; load start never arriving within the run.
        l.add(flat("join_float1_compiled_first").spawn(0.5, 65, 0.5, 90, 10).run(1, 0)
                .event(0, new Scenario.SectionCompiled(true)).run(3, 0).event(0, new Scenario.LoadStart()).run(12, 0));
        l.add(flat("join_float1_timeout").spawn(0.5, 65, 0.5, 90, 10).event(0, new Scenario.LoadStart()).run(8, 0)
                .event(0, new Scenario.LoadTimeout()).run(12, 0));
        l.add(flat("join_float1_teleport_while_loading").spawn(0.5, 65, 0.5, 90, 10).event(0, new Scenario.LoadStart())
                .run(2, 0).event(0, new Scenario.Teleport(2.5, 66.0, 0.5, 0, 0, 0)).run(2, 0)
                .event(0, new Scenario.SectionCompiled(true)).run(12, 0));
        l.add(flat("join_float1_never_loaded").spawn(0.5, 65, 0.5, 90, 10).run(2, 0)
                .event(0, new Scenario.SectionCompiled(true)).run(20, 0));
        // The engine's case: yaw 0, pitch 36.75, three ticks before the gate, then until well after landing.
        l.add(flat("join_float1_pitch36_75").spawn(0.5, 65, 0.5, 0, 36.75F).event(0, new Scenario.LoadStart()).run(2, 0)
                .event(0, new Scenario.SectionCompiled(true)).run(40, 0));
        for (Scenario sc : l.subList(l.size() - 5, l.size())) {
            sc.delayedLoad = true;
        }
        l.get(l.size() - 1).description = "fresh player at y 65 (1 block up), yaw 0 pitch 36.75; load start at tick 1, "
                + "section compiled at tick 4, 40 ticks after";
        l.get(l.size() - 5).description = "section compiled at tick 2, level-load start at tick 6";
        l.get(l.size() - 4).description = "never compiled; the 30 s timeout fires at tick 10";
        l.get(l.size() - 3).description = "a second position packet (2.5, 66, 0.5) while the level loads";
        l.get(l.size() - 2).description = "compiled, but the server never sends the level-load start";
        return l;
    }
}
