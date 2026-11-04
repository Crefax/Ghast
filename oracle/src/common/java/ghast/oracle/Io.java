package ghast.oracle;

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
import java.util.HexFormat;
import java.util.function.Consumer;
import java.util.zip.Deflater;
import java.util.zip.GZIPInputStream;
import java.util.zip.GZIPOutputStream;

/** Version-independent output helpers of the oracle CLIs (JSON-lines files, gzip, content hashes). */
public final class Io {
    private Io() {}

    /** Golden files are compressed harder; big fuzz runs favour speed. */
    public static boolean bestCompression;

    public interface LineSource {
        void emit(Consumer<String> sink) throws IOException;
    }

    /**
     * Writes the lines (gzip when the name ends in .gz; GZIPOutputStream writes mtime 0) atomically:
     * into a temporary file in the same directory, then renamed over the target.
     */
    public static void write(Path out, LineSource source) throws IOException {
        Path dir = out.toAbsolutePath().getParent();
        Files.createDirectories(dir);
        Path tmp = dir.resolve("." + out.getFileName() + ".partial");
        long[] n = {0};
        boolean done = false;
        try {
            OutputStream os = Files.newOutputStream(tmp);
            if (out.toString().endsWith(".gz")) {
                os = new GZIPOutputStream(os, 1 << 16) {
                    {
                        def.setLevel(bestCompression ? Deflater.BEST_COMPRESSION : Deflater.DEFAULT_COMPRESSION);
                    }
                };
            }
            try (BufferedWriter w = new BufferedWriter(new OutputStreamWriter(os, StandardCharsets.UTF_8), 1 << 16)) {
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

    /** Compares the content of every file of {@code a} with the same file in {@code b}. */
    public static boolean compare(Path a, Path b) throws Exception {
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
    public static String sha256(Path p) throws Exception {
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

    public static void deleteTree(Path base) throws IOException {
        try (var walk = Files.walk(base)) {
            for (Path p : walk.sorted(java.util.Comparator.reverseOrder()).toList()) {
                Files.delete(p);
            }
        }
    }

    /** Tiny {@code --flag value} parser (argument 0 is the command). */
    public record Args(String[] args) {
        public String str(String flag, String def) {
            for (int i = 1; i + 1 < args.length; i++) {
                if (args[i].equals(flag)) {
                    return args[i + 1];
                }
            }
            return def;
        }

        public long lng(String flag, long def) {
            String s = str(flag, null);
            return s == null ? def : Long.parseLong(s);
        }

        public Path path(String flag, Path def) {
            String s = str(flag, null);
            return s == null ? def : Path.of(s);
        }
    }
}
