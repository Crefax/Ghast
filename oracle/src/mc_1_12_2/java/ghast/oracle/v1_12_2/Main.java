package ghast.oracle.v1_12_2;

import ghast.oracle.Scenario;

import java.io.BufferedWriter;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.io.OutputStreamWriter;
import java.io.UncheckedIOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;
import java.util.function.Consumer;
import java.util.zip.Deflater;
import java.util.zip.GZIPInputStream;
import java.util.zip.GZIPOutputStream;

/** Command line entry point of the 1.12.2 oracle. Run {@code help} for usage. */
public final class Main {
    static final String VERSION = "1.12.2";

    private Main() {}

    public static void main(String[] args) throws Exception {
        if (args.length == 0) {
            usage();
            return;
        }
        Path repo = Path.of(System.getProperty("ghast.oracle.repoRoot", ".."));
        Path golden = repo.resolve("tests/golden/" + VERSION);
        Args a = new Args(args);
        switch (args[0]) {
            case "golden" -> {
                Path out = a.path("--out", golden);
                writeGolden(out);
            }
            case "scenarios" -> {
                Path out = a.path("--out", golden.resolve("scenarios.jsonl.gz"));
                String only = a.str("--only", null);
                write(out, sink -> runAll(Scenarios.all().stream().filter(sc -> only == null || only.equals(sc.name)).toList(), sink));
            }
            case "fuzz" -> {
                long seed = a.lng("--seed", 1);
                int count = (int) a.lng("--count", 100);
                int ticks = (int) a.lng("--ticks", 60);
                boolean useItem = Boolean.parseBoolean(a.str("--use-item", "false"));
                boolean edges = Boolean.parseBoolean(a.str("--edges", "false")) || "edges".equals(a.str("--palette", ""));
                boolean extras = Boolean.parseBoolean(a.str("--extras", "false"));
                Path out = a.path("--out", Path.of("cache", VERSION, "fuzz", "fuzz-seed" + seed + "-n" + count + ".jsonl.gz"));
                // Streamed: cases are generated and written one at a time (millions of ticks fit).
                write(out, sink -> {
                    for (int i = 0; i < count; i++) {
                        run(edges ? Fuzz.edge(seed, i, ticks)
                                : extras ? Fuzz.withExtras(seed, i, ticks, useItem) : Fuzz.one(seed, i, ticks, useItem), sink);
                    }
                });
            }
            case "boxes" -> {
                Path out = a.path("--out", golden.resolve("collision_boxes.jsonl.gz"));
                write(out, sink -> CollisionDump.lines().forEach(sink));
            }
            case "replay" -> {
                Path in = Path.of(a.str("--in", null));
                Path out = a.path("--out", Path.of("cache", VERSION, "replay.jsonl.gz"));
                write(out, sink -> ScenarioReader.read(in, sc -> run(sc, sink)));
            }
            case "boxes-far" -> {
                Path out = a.path("--out", golden.resolve("collision_boxes_far.jsonl.gz"));
                write(out, sink -> CollisionDump.farLines().forEach(sink));
            }
            case "faces" -> {
                Path out = a.path("--out", golden.resolve("face_shapes.jsonl.gz"));
                write(out, sink -> FaceDump.lines().forEach(sink));
            }
            case "selection" -> {
                Path out = a.path("--out", golden.resolve("selection_boxes.jsonl.gz"));
                write(out, sink -> SelectionDump.lines().forEach(sink));
            }
            case "rays" -> {
                long seed = a.lng("--seed", GOLDEN_RAY_SEED);
                int worlds = (int) a.lng("--worlds", GOLDEN_RAY_WORLDS);
                int rays = (int) a.lng("--rays", GOLDEN_RAYS);
                Path out = a.path("--out", Path.of("cache", VERSION, "rays-seed" + seed + ".jsonl.gz"));
                write(out, sink -> SelectionDump.rays(seed, worlds, rays, sink));
            }
            case "recipes" -> write(a.path("--out", golden.resolve("recipes.jsonl.gz")), RecipeDump::lines);
            case "clicks" -> {
                // window click vectors (ClickDump): --seed, --count cases (1..10 clicks each)
                long seed = a.lng("--seed", 1);
                int count = (int) a.lng("--count", 1000);
                boolean craft = Boolean.parseBoolean(a.str("--craft", "false"));
                Path out = a.path("--out", Path.of("cache", VERSION, "clicks",
                        (craft ? "craft" : "clicks") + "-seed" + seed + "-n" + count + ".jsonl.gz"));
                write(out, sink -> ClickDump.cases(seed, count, craft, sink));
            }
            case "entity-sizes" -> write(a.path("--out", golden.resolve("entity_sizes.jsonl.gz")), EntitySizeDump::lines);
            case "list" -> {
                for (Scenario sc : Scenarios.all()) {
                    System.out.println(sc.name + "\t" + sc.ticks.size() + "\t" + sc.description);
                }
            }
            case "selftest" -> selftest();
            case "compare" -> System.exit(compare(Path.of(args[1]), Path.of(args[2])) ? 0 : 1);
            default -> usage();
        }
    }

    private static void usage() {
        System.out.println("""
                ghast oracle (Minecraft %s)
                  golden   [--out DIR]                 write the committed golden set (scenarios, fuzz, boxes)
                  scenarios [--out FILE] [--only NAME] run the hand-written scenarios
                  fuzz [--seed N] [--count N] [--ticks N] [--use-item true] [--palette edges] [--extras true] [--out FILE(.gz)]
                                                       randomized cases (default into cache/%s/fuzz/)
                  boxes [--out FILE]                   collision box dump for every block id/meta
                  boxes-far [--out FILE]               the same (alone/settled) at far positions, absolute
                  replay --in FILE [--out FILE]        rerun scenarios read from golden-format JSON lines
                                                       (header + tick inputs; states are ignored)
                  faces [--out FILE]                   1.12 face shapes (BlockRenderLayer) of every id/meta + exclusion lists
                  selection [--out FILE]               ray-test bounds (selection boxes) for every block id/meta
                  rays [--seed N] [--worlds N] [--rays N] [--out FILE]  crosshair raytrace vectors
                  recipes [--out FILE]                 crafting recipes in the client's match order
                  clicks [--seed N] [--count N] [--craft true] [--out FILE]  window click vectors (cache/<mc>/clicks/);
                                                       --craft: crafting grids from recipes, clicks on result/grid
                  entity-sizes [--out FILE]            width/height/eye height of every entity type + variants
                  list                                 list hand-written scenarios
                  selftest                             generate everything twice and compare content
                  compare DIR_A DIR_B                  compare the content of golden files (exit 1 on diff)
                """.formatted(VERSION, VERSION));
    }

    /** The committed set; kept small (see README for sizes). */
    static final long GOLDEN_FUZZ_SEED = 1;
    static final int GOLDEN_FUZZ_COUNT = 100;
    static final int GOLDEN_FUZZ_TICKS = 60;
    static final long GOLDEN_RAY_SEED = 1;
    static final int GOLDEN_RAY_WORLDS = 50;
    static final int GOLDEN_RAYS = 40;
    static final long GOLDEN_CLICK_SEED = 1;
    static final int GOLDEN_CLICK_CASES = 3000;

    /** Golden files are compressed harder; big fuzz runs favour speed. */
    private static boolean bestCompression;

    static void writeGolden(Path dir) throws IOException {
        bestCompression = true;
        write(dir.resolve("scenarios.jsonl.gz"), sink -> runAll(Scenarios.all(), sink));
        write(dir.resolve("fuzz_seed" + GOLDEN_FUZZ_SEED + ".jsonl.gz"),
                sink -> runAll(Fuzz.cases(GOLDEN_FUZZ_SEED, GOLDEN_FUZZ_COUNT, GOLDEN_FUZZ_TICKS), sink));
        write(dir.resolve("collision_boxes.jsonl.gz"), sink -> CollisionDump.lines().forEach(sink));
        write(dir.resolve("collision_boxes_far.jsonl.gz"), sink -> CollisionDump.farLines().forEach(sink));
        write(dir.resolve("fuzz_edges_seed" + GOLDEN_FUZZ_SEED + ".jsonl.gz"), sink -> {
            for (int i = 0; i < GOLDEN_FUZZ_COUNT; i++) {
                run(Fuzz.edge(GOLDEN_FUZZ_SEED, i, GOLDEN_FUZZ_TICKS), sink);
            }
        });
        write(dir.resolve("entity_sizes.jsonl.gz"), EntitySizeDump::lines);
        write(dir.resolve("face_shapes.jsonl.gz"), sink -> FaceDump.lines().forEach(sink));
        write(dir.resolve("selection_boxes.jsonl.gz"), sink -> SelectionDump.lines().forEach(sink));
        write(dir.resolve("raytrace.jsonl.gz"), sink -> SelectionDump.rays(GOLDEN_RAY_SEED, GOLDEN_RAY_WORLDS, GOLDEN_RAYS, sink));
        write(dir.resolve("recipes.jsonl.gz"), RecipeDump::lines);
        write(dir.resolve("clicks_seed" + GOLDEN_CLICK_SEED + ".jsonl.gz"), sink -> ClickDump.cases(GOLDEN_CLICK_SEED, GOLDEN_CLICK_CASES, sink));
        write(dir.resolve("clicks_craft_seed" + GOLDEN_CLICK_SEED + ".jsonl.gz"), sink -> ClickDump.cases(GOLDEN_CLICK_SEED, GOLDEN_CLICK_CASES, true, sink));
        bestCompression = false;
    }

    static void runAll(List<Scenario> scenarios, Consumer<String> sink) {
        for (Scenario sc : scenarios) {
            run(sc, sink);
        }
    }

    /** Runs one scenario: header line, tick 0 (state after setup), then one line per tick. */
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

    interface LineSource {
        void emit(Consumer<String> sink) throws IOException;
    }

    /**
     * Writes the lines atomically: into a temporary file in the same directory, then renamed over the
     * target, so a reader (a test run) never sees a half-written file.
     */
    static void write(Path out, LineSource source) throws IOException {
        Path dir = out.toAbsolutePath().getParent();
        Files.createDirectories(dir);
        Path tmp = dir.resolve("." + out.getFileName() + ".partial");
        long[] n = {0};
        boolean done = false;
        try {
            try (OutputStream os = open(tmp, out.toString().endsWith(".gz"));
                 BufferedWriter w = new BufferedWriter(new OutputStreamWriter(os, StandardCharsets.UTF_8), 1 << 16)) {
                source.emit(line -> {
                    try {
                        w.write(line);
                        w.write('\n');
                        n[0]++;
                    } catch (IOException e) {
                        throw new UncheckedIOException(e);
                    }
                });
            }
            Files.move(tmp, out, java.nio.file.StandardCopyOption.REPLACE_EXISTING, java.nio.file.StandardCopyOption.ATOMIC_MOVE);
            done = true;
        } finally {
            if (!done) {
                Files.deleteIfExists(tmp);
            }
        }
        System.out.println("[oracle] wrote " + n[0] + " lines to " + out);
    }

    private static OutputStream open(Path file, boolean gzip) throws IOException {
        OutputStream os = Files.newOutputStream(file);
        if (gzip) {
            // GZIPOutputStream writes mtime 0, so the bytes depend only on the content and the zlib
            // version; compare decompressed content across machines (see selftest / run.sh verify).
            return new GZIPOutputStream(os, 1 << 16) {
                {
                    def.setLevel(bestCompression ? Deflater.BEST_COMPRESSION : Deflater.DEFAULT_COMPRESSION);
                }
            };
        }
        return os;
    }

    /**
     * Runs the whole golden generation twice in this JVM (the second run starts with whatever static
     * state the first left behind) and compares the SHA-256 of the decompressed content of every file.
     */
    static void selftest() throws Exception {
        Path base = Files.createTempDirectory("ghast-oracle-selftest");
        Path a = base.resolve("a"), b = base.resolve("b");
        writeGolden(a);
        writeGolden(b);
        boolean ok = compare(a, b);

        // Order independence: every scenario must give the same lines when run after a different set
        // of scenarios (no static state leaks from one run into the next).
        List<Scenario> all = new ArrayList<>(Scenarios.all());
        all.addAll(Fuzz.cases(GOLDEN_FUZZ_SEED, GOLDEN_FUZZ_COUNT, GOLDEN_FUZZ_TICKS));
        for (int i = 0; i < GOLDEN_FUZZ_COUNT; i++) {
            all.add(Fuzz.edge(GOLDEN_FUZZ_SEED, i, GOLDEN_FUZZ_TICKS));
        }
        List<List<String>> forward = new ArrayList<>();
        for (Scenario sc : all) {
            forward.add(run(sc));
        }
        CollisionDump.lines(); // perturbs block state on purpose
        int mismatches = 0;
        for (int i = all.size() - 1; i >= 0; i--) {
            if (!run(all.get(i)).equals(forward.get(i))) {
                System.out.println("DIFF reverse-order run of " + all.get(i).name);
                mismatches++;
            }
        }
        System.out.println((mismatches == 0 ? "same " : "DIFF ") + "reverse-order rerun of " + all.size() + " scenarios");
        ok &= mismatches == 0;
        int clickMismatches = ClickDump.reverseMismatches(GOLDEN_CLICK_SEED, 500);
        System.out.println((clickMismatches == 0 ? "same " : "DIFF ") + "reverse-order rerun of 500 click cases");
        ok &= clickMismatches == 0;

        // Round trip: the written files, read back as inputs and replayed, reproduce themselves.
        for (String f : new String[] {"scenarios.jsonl.gz", "fuzz_seed" + GOLDEN_FUZZ_SEED + ".jsonl.gz",
                "fuzz_edges_seed" + GOLDEN_FUZZ_SEED + ".jsonl.gz"}) {
            Path replayed = base.resolve("replayed-" + f);
            write(replayed, sink -> ScenarioReader.read(a.resolve(f), sc -> run(sc, sink)));
            boolean same = sha256(replayed).equals(sha256(a.resolve(f)));
            System.out.println((same ? "same " : "DIFF ") + "replay of " + f);
            ok &= same;
        }
        try (var walk = Files.walk(base)) {
            for (Path p : walk.sorted(java.util.Comparator.reverseOrder()).toList()) {
                Files.delete(p);
            }
        }
        if (!ok) {
            System.out.println("[oracle] selftest FAILED: output is not deterministic");
            System.exit(1);
        }
        System.out.println("[oracle] selftest passed");
    }

    /** Compares the content of every file of {@code a} with the same file in {@code b}. */
    static boolean compare(Path a, Path b) throws Exception {
        boolean ok = true;
        try (var files = Files.list(a)) {
            for (Path f : files.sorted().toList()) {
                Path other = b.resolve(f.getFileName());
                String ha = sha256(f), hb = Files.exists(other) ? sha256(other) : "missing";
                System.out.println((ha.equals(hb) ? "same " : "DIFF ") + ha + "  " + f.getFileName());
                ok &= ha.equals(hb);
            }
        }
        return ok;
    }

    /** SHA-256 of the content (decompressed for .gz files). */
    static String sha256(Path p) throws Exception {
        byte[] bytes;
        if (p.toString().endsWith(".gz")) {
            try (InputStream in = new GZIPInputStream(Files.newInputStream(p))) {
                bytes = in.readAllBytes();
            }
        } else {
            bytes = Files.readAllBytes(p);
        }
        return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
    }

    /** Tiny {@code --flag value} parser. */
    private record Args(String[] args) {
        String str(String flag, String def) {
            for (int i = 1; i + 1 < args.length; i++) {
                if (args[i].equals(flag)) {
                    return args[i + 1];
                }
            }
            return def;
        }

        long lng(String flag, long def) {
            String s = str(flag, null);
            return s == null ? def : Long.parseLong(s);
        }

        Path path(String flag, Path def) {
            String s = str(flag, null);
            return s == null ? def : Path.of(s);
        }
    }
}
