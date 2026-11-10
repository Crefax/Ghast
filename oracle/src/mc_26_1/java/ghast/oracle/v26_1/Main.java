package ghast.oracle.v26_1;

import ghast.oracle.Io;
import ghast.oracle.Scenario;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;

/** Command line entry point of the 26.1 oracle. Run {@code help} for usage. */
public final class Main {
    static final String VERSION = "26.1";

    private Main() {}

    public static void main(String[] args) throws Exception {
        if (args.length == 0) {
            usage();
            return;
        }
        Path repo = Path.of(System.getProperty("ghast.oracle.repoRoot", ".."));
        Path golden = repo.resolve("tests/golden/" + VERSION);
        Io.Args a = new Io.Args(args);
        switch (args[0]) {
            case "golden" -> writeGolden(a.path("--out", golden));
            case "scenarios" -> {
                Path out = a.path("--out", golden.resolve("scenarios.jsonl.gz"));
                String only = a.str("--only", null);
                Io.write(out, sink -> runAll(Scenarios.all().stream().filter(sc -> only == null || only.equals(sc.name)).toList(), sink));
            }
            case "fuzz" -> {
                long seed = a.lng("--seed", 1);
                int count = (int) a.lng("--count", 100);
                int ticks = (int) a.lng("--ticks", 60);
                boolean useItem = Boolean.parseBoolean(a.str("--use-item", "false"));
                boolean edges = Boolean.parseBoolean(a.str("--edges", "false")) || "edges".equals(a.str("--palette", ""));
                boolean extras = Boolean.parseBoolean(a.str("--extras", "false"));
                Path out = a.path("--out", Path.of("cache", VERSION, "fuzz", "fuzz-seed" + seed + "-n" + count + ".jsonl.gz"));
                Io.write(out, sink -> {
                    for (int i = 0; i < count; i++) {
                        run(edges ? Fuzz.edge(seed, i, ticks)
                                : extras ? Fuzz.withExtras(seed, i, ticks, useItem) : Fuzz.one(seed, i, ticks, useItem), sink);
                    }
                });
            }
            case "replay" -> {
                Path in = Path.of(a.str("--in", null));
                Path out = a.path("--out", Path.of("cache", VERSION, "replay.jsonl.gz"));
                Io.write(out, sink -> ScenarioReader.read(in, sc -> run(sc, sink)));
            }
            case "boxes" -> Io.write(a.path("--out", golden.resolve("collision_shapes.jsonl.gz")), sink -> CollisionDump.lines(sink));
            case "boxes-far" -> Io.write(a.path("--out", golden.resolve("collision_shapes_far.jsonl.gz")), sink -> CollisionDump.farLines(sink));
            case "voxels" -> Io.write(a.path("--out", golden.resolve("voxel_shapes.jsonl.gz")), Main::voxels);
            case "entity-sizes" -> Io.write(a.path("--out", golden.resolve("entity_sizes.jsonl.gz")), EntitySizeDump::lines);
            case "selection" -> Io.write(a.path("--out", golden.resolve("selection_shapes.jsonl.gz")), Main::selection);
            case "rays" -> {
                long seed = a.lng("--seed", 1);
                int worlds = (int) a.lng("--worlds", SelectionDump.RAY_WORLDS);
                int rays = (int) a.lng("--rays", SelectionDump.RAY_RAYS);
                Io.write(a.path("--out", golden.resolve("raytrace.jsonl.gz")), sink -> SelectionDump.rays(seed, worlds, rays, sink));
            }
            case "list" -> {
                for (Scenario sc : Scenarios.all()) {
                    System.out.println(sc.name + "\t" + sc.ticks.size() + "\t" + sc.description);
                }
            }
            case "selftest" -> selftest();
            case "compare" -> System.exit(Io.compare(Path.of(args[1]), Path.of(args[2])) ? 0 : 1);
            default -> usage();
        }
    }

    private static void usage() {
        System.out.println("""
                ghast oracle (Minecraft %s)
                  golden   [--out DIR]                 write the committed golden set
                  scenarios [--out FILE] [--only NAME] run the hand-written scenarios
                  fuzz [--seed N] [--count N] [--ticks N] [--use-item true] [--palette edges] [--extras true] [--out FILE(.gz)]
                  replay --in FILE [--out FILE]        rerun scenarios read from golden-format JSON lines
                  boxes [--out FILE]                   collision shapes of every block state
                  boxes-far [--out FILE]               the same at far positions, absolute
                  voxels [--out FILE]                  voxel grids of every collision shape + per-state facts
                  entity-sizes [--out FILE]            dimensions / eye heights of every entity type, poses, variants
                  selection [--out FILE]               outline / interaction shapes of every block state
                  rays [--seed N] [--worlds N] [--rays N] [--out FILE]  crosshair picks in random worlds
                  list | selftest | compare DIR_A DIR_B
                """.formatted(VERSION));
    }

    static final long GOLDEN_FUZZ_SEED = 1;
    static final int GOLDEN_FUZZ_COUNT = 100;
    static final int GOLDEN_FUZZ_TICKS = 60;

    static void writeGolden(Path dir) throws IOException {
        Io.bestCompression = true;
        Io.write(dir.resolve("scenarios.jsonl.gz"), sink -> runAll(Scenarios.all(), sink));
        Io.write(dir.resolve("fuzz_seed" + GOLDEN_FUZZ_SEED + ".jsonl.gz"),
                sink -> runAll(Fuzz.cases(GOLDEN_FUZZ_SEED, GOLDEN_FUZZ_COUNT, GOLDEN_FUZZ_TICKS), sink));
        Io.write(dir.resolve("fuzz_edges_seed" + GOLDEN_FUZZ_SEED + ".jsonl.gz"), sink -> {
            for (int i = 0; i < GOLDEN_FUZZ_COUNT; i++) {
                run(Fuzz.edge(GOLDEN_FUZZ_SEED, i, GOLDEN_FUZZ_TICKS), sink);
            }
        });
        Io.write(dir.resolve("fuzz_extras_seed" + GOLDEN_FUZZ_SEED + ".jsonl.gz"), sink -> {
            for (int i = 0; i < GOLDEN_FUZZ_COUNT; i++) {
                run(Fuzz.withExtras(GOLDEN_FUZZ_SEED, i, GOLDEN_FUZZ_TICKS, i % 2 == 0), sink);
            }
        });
        Io.write(dir.resolve("collision_shapes.jsonl.gz"), CollisionDump::lines);
        Io.write(dir.resolve("collision_shapes_far.jsonl.gz"), CollisionDump::farLines);
        Io.write(dir.resolve("voxel_shapes.jsonl.gz"), Main::voxels);
        Io.write(dir.resolve("entity_sizes.jsonl.gz"), EntitySizeDump::lines);
        Io.write(dir.resolve("selection_shapes.jsonl.gz"), Main::selection);
        Io.write(dir.resolve("raytrace.jsonl.gz"), sink -> SelectionDump.rays(1, SelectionDump.RAY_WORLDS, SelectionDump.RAY_RAYS, sink));
        Io.bestCompression = false;
    }

    static void voxels(Consumer<String> sink) {
        try {
            VoxelDump.lines(sink);
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    static void selection(Consumer<String> sink) {
        try {
            SelectionDump.selection(sink);
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    static void runAll(List<Scenario> scenarios, Consumer<String> sink) {
        for (Scenario sc : scenarios) {
            run(sc, sink);
        }
    }

    static void run(Scenario sc, Consumer<String> sink) {
        Harness.Run run = new Harness.Run(sc);
        sink.accept(Recorder.header(sc, run, VERSION));
        sink.accept(Recorder.tick(run, null));
        for (Scenario.Tick t : sc.ticks) {
            run.step(t);
            sink.accept(Recorder.tick(run, t));
        }
        Harness.releaseKeys();
    }

    static List<String> run(Scenario sc) {
        List<String> lines = new ArrayList<>(sc.ticks.size() + 2);
        run(sc, lines::add);
        return lines;
    }

    static void selftest() throws Exception {
        Path base = Files.createTempDirectory("ghast-oracle-selftest");
        Path a = base.resolve("a"), b = base.resolve("b");
        writeGolden(a);
        writeGolden(b);
        boolean ok = Io.compare(a, b);
        List<Scenario> all = new ArrayList<>(Scenarios.all());
        all.addAll(Fuzz.cases(GOLDEN_FUZZ_SEED, GOLDEN_FUZZ_COUNT, GOLDEN_FUZZ_TICKS));
        for (int i = 0; i < GOLDEN_FUZZ_COUNT; i++) {
            all.add(Fuzz.edge(GOLDEN_FUZZ_SEED, i, GOLDEN_FUZZ_TICKS));
            all.add(Fuzz.withExtras(GOLDEN_FUZZ_SEED, i, GOLDEN_FUZZ_TICKS, i % 2 == 0));
        }
        List<List<String>> forward = new ArrayList<>();
        for (Scenario sc : all) {
            forward.add(run(sc));
        }
        int mismatches = 0;
        for (int i = all.size() - 1; i >= 0; i--) {
            if (!run(all.get(i)).equals(forward.get(i))) {
                System.out.println("DIFF reverse-order run of " + all.get(i).name);
                mismatches++;
            }
        }
        System.out.println((mismatches == 0 ? "same " : "DIFF ") + "reverse-order rerun of " + all.size() + " scenarios");
        ok &= mismatches == 0;
        for (String f : new String[] {"scenarios.jsonl.gz", "fuzz_seed" + GOLDEN_FUZZ_SEED + ".jsonl.gz",
                "fuzz_edges_seed" + GOLDEN_FUZZ_SEED + ".jsonl.gz", "fuzz_extras_seed" + GOLDEN_FUZZ_SEED + ".jsonl.gz"}) {
            Path replayed = base.resolve("replayed-" + f);
            Io.write(replayed, sink -> ScenarioReader.read(a.resolve(f), sc -> run(sc, sink)));
            boolean same = Io.sha256(replayed).equals(Io.sha256(a.resolve(f)));
            System.out.println((same ? "same " : "DIFF ") + "replay of " + f);
            ok &= same;
        }
        Io.deleteTree(base);
        if (!ok) {
            System.out.println("[oracle] selftest FAILED: output is not deterministic");
            System.exit(1);
        }
        System.out.println("[oracle] selftest passed");
    }
}
