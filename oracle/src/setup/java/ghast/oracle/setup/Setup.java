package ghast.oracle.setup;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import net.fabricmc.tinyremapper.NonClassCopyMode;
import net.fabricmc.tinyremapper.OutputConsumerPath;
import net.fabricmc.tinyremapper.TinyRemapper;
import net.fabricmc.tinyremapper.TinyUtils;

import java.io.IOException;
import java.io.InputStream;
import java.io.Reader;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.time.Duration;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;
import java.util.Properties;
import java.util.zip.ZipEntry;
import java.util.zip.ZipInputStream;

/**
 * Downloads everything the oracle needs for one Minecraft version into the git-ignored cache and
 * produces a jar remapped to Yarn names.
 *
 * <p>Usage: {@code Setup prepare <version.properties> <cacheRoot>}
 *
 * <p>Layout written under {@code <cacheRoot>/<version>/}:
 * <pre>
 *   version.json            Mojang version json (SHA-1 checked against the manifest)
 *   client-official.jar     vanilla client jar (SHA-1 checked against the version json)
 *   libraries/...           runtime libraries (SHA-1 checked), natives skipped
 *   mappings/mappings.tiny  tiny v2 mappings (artifact SHA-1 checked against the maven .sha1)
 *   client-named.jar        client jar remapped mappings.from -> mappings.to
 *   classpath.txt           absolute paths of the library jars, one per line
 *   setup.properties        record of what was fetched (hashes, mapping build)
 * </pre>
 * Every step is idempotent: files whose hash already matches are not downloaded again.
 */
public final class Setup {
    private static final HttpClient HTTP = HttpClient.newBuilder()
            .followRedirects(HttpClient.Redirect.NORMAL)
            .connectTimeout(Duration.ofSeconds(30))
            .build();

    private Setup() {}

    public static void main(String[] args) throws Exception {
        if (args.length == 3 && args[0].equals("prepare")) {
            prepare(Path.of(args[1]), Path.of(args[2]));
        } else {
            System.err.println("usage: Setup prepare <version.properties> <cacheRoot>");
            System.exit(2);
        }
    }

    static void prepare(Path versionFile, Path cacheRoot) throws Exception {
        Properties cfg = new Properties();
        try (Reader r = Files.newBufferedReader(versionFile, StandardCharsets.UTF_8)) {
            cfg.load(r);
        }
        String mc = require(cfg, "minecraft.version");
        Path dir = cacheRoot.resolve(mc).toAbsolutePath();
        Files.createDirectories(dir);

        // 1. Mojang manifest -> version json (manifest is re-fetched only if the version json is missing).
        Path versionJson = dir.resolve("version.json");
        if (!Files.exists(versionJson)) {
            JsonObject manifest = JsonParser.parseString(fetchString(require(cfg, "minecraft.manifest"))).getAsJsonObject();
            JsonObject entry = null;
            for (JsonElement e : manifest.getAsJsonArray("versions")) {
                if (e.getAsJsonObject().get("id").getAsString().equals(mc)) {
                    entry = e.getAsJsonObject();
                }
            }
            if (entry == null) {
                throw new IllegalStateException("version " + mc + " not in Mojang manifest");
            }
            download(entry.get("url").getAsString(), entry.get("sha1").getAsString(), versionJson);
        }
        JsonObject version = JsonParser.parseString(Files.readString(versionJson)).getAsJsonObject();

        // 2. Client jar.
        JsonObject client = version.getAsJsonObject("downloads").getAsJsonObject("client");
        Path clientJar = dir.resolve("client-official.jar");
        String clientSha1 = client.get("sha1").getAsString();
        ensure(client.get("url").getAsString(), clientSha1, clientJar);

        // 3. Libraries (plain artifacts only; natives are not needed headless). Rules are evaluated
        //    for a fixed OS ("linux") so the classpath is the same on every machine.
        List<Path> libs = new ArrayList<>();
        Path libDir = dir.resolve("libraries");
        for (JsonElement e : version.getAsJsonArray("libraries")) {
            JsonObject lib = e.getAsJsonObject();
            if (!rulesAllow(lib.getAsJsonArray("rules"))) {
                continue;
            }
            JsonObject downloads = lib.getAsJsonObject("downloads");
            if (downloads == null || !downloads.has("artifact")) {
                continue;
            }
            JsonObject art = downloads.getAsJsonObject("artifact");
            Path target = libDir.resolve(art.get("path").getAsString());
            ensure(art.get("url").getAsString(), art.get("sha1").getAsString(), target);
            libs.add(target);
        }

        // 4. Mappings from maven (hash from the repository's .sha1 file).
        String repo = require(cfg, "mappings.repo");
        if (!repo.endsWith("/")) {
            repo += "/";
        }
        String group = require(cfg, "mappings.group");
        String artifact = require(cfg, "mappings.artifact");
        String mver = require(cfg, "mappings.version");
        String classifier = cfg.getProperty("mappings.classifier", "");
        String fileName = artifact + "-" + mver + (classifier.isEmpty() ? "" : "-" + classifier) + ".jar";
        String mavenUrl = repo + group.replace('.', '/') + "/" + artifact + "/" + mver + "/" + fileName;
        Path mappingJar = dir.resolve("mappings").resolve(fileName);
        String mappingSha1 = fetchString(mavenUrl + ".sha1").trim().split("\\s+")[0];
        ensure(mavenUrl, mappingSha1, mappingJar);
        Path tiny = dir.resolve("mappings").resolve("mappings.tiny");
        extractEntry(mappingJar, "mappings/mappings.tiny", tiny);

        // 5. Remap. Re-done whenever the stamp (inputs) changes.
        String from = require(cfg, "mappings.from");
        String to = require(cfg, "mappings.to");
        Path named = dir.resolve("client-" + to + ".jar");
        boolean strip = Boolean.parseBoolean(cfg.getProperty("remap.stripParameterAnnotations", "false"));
        boolean fixAccess = Boolean.parseBoolean(cfg.getProperty("remap.fixPackageAccess", "false"));
        boolean publicProtected = Boolean.parseBoolean(cfg.getProperty("remap.publicProtected", "false"));
        String stamp = clientSha1 + "|" + mappingSha1 + "|" + from + "->" + to + (strip ? "|strip-param-annotations" : "")
                + (fixAccess ? "|fix-package-access" : "") + (publicProtected ? "|public-protected" : "");
        Path stampFile = dir.resolve("client-" + to + ".stamp");
        if (!Files.exists(named) || !Files.exists(stampFile) || !Files.readString(stampFile).equals(stamp)) {
            System.out.println("[setup] remapping " + clientJar.getFileName() + " " + from + " -> " + to);
            remap(clientJar, named, tiny, from, to, libs, fixAccess);
            if (strip || publicProtected) {
                rewriteClasses(named, strip, publicProtected);
            }
            Files.writeString(stampFile, stamp);
        }

        // 6. Classpath + record.
        StringBuilder cp = new StringBuilder();
        for (Path p : libs) {
            cp.append(p).append('\n');
        }
        Files.writeString(dir.resolve("classpath.txt"), cp.toString());
        Properties rec = new Properties();
        rec.setProperty("minecraft.version", mc);
        rec.setProperty("client.sha1", clientSha1);
        rec.setProperty("mappings.url", mavenUrl);
        rec.setProperty("mappings.sha1", mappingSha1);
        rec.setProperty("mappings.version", mver);
        rec.setProperty("named.jar", named.toString());
        try (var w = Files.newBufferedWriter(dir.resolve("setup.properties"))) {
            rec.store(w, "written by ghast oracle setup");
        }
        System.out.println("[setup] ready: " + named);
    }

    private static void remap(Path in, Path out, Path tiny, String from, String to, List<Path> libs, boolean fixAccess)
            throws IOException {
        // OutputConsumerPath writes a zip only when the target name ends in .jar/.zip.
        Path tmp = out.resolveSibling("partial-" + out.getFileName());
        Files.deleteIfExists(tmp);
        TinyRemapper remapper = TinyRemapper.newRemapper()
                .withMappings(TinyUtils.createTinyMappingProvider(tiny, from, to))
                .renameInvalidLocals(true)
                .rebuildSourceFilenames(true)
                // The official jar has every class in one package; after remapping into Yarn packages,
                // protected / package-private cross-package access would fail at run time. This widens
                // those access flags (what Fabric Loom does); method bodies are unchanged.
                .fixPackageAccess(fixAccess)
                .build();
        try (OutputConsumerPath consumer = new OutputConsumerPath.Builder(tmp).build()) {
            consumer.addNonClassFiles(in, NonClassCopyMode.FIX_META_INF, remapper);
            remapper.readInputs(in);
            remapper.readClassPath(libs.toArray(new Path[0]));
            remapper.apply(consumer);
        } finally {
            remapper.finish();
        }
        Files.move(tmp, out, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
    }

    /**
     * Rewrites every class of the jar.
     *
     * <p>{@code strip}: removes the parameter annotation tables (@Nullable markers). Some classes carry
     * tables whose length does not match the descriptor (the synthetic outer-instance parameter of
     * inner-class constructors), and javac refuses to compile against them ("bad
     * RuntimeInvisibleParameterAnnotations attribute"). Nothing in the game reads them at run time.
     *
     * <p>{@code publicProtected}: makes protected fields and methods public. The official jar has
     * (almost) every class in one package, so a protected member was also package-visible; after
     * remapping into Yarn packages the verifier rejects protected access on another instance's type
     * from a subclass in a different package (e.g. 1.12.2's horses). Widening is what an access
     * widener does; method bodies and dispatch are unchanged.
     */
    private static void rewriteClasses(Path jar, boolean strip, boolean publicProtected) throws IOException {
        Path tmp = jar.resolveSibling("partial-strip-" + jar.getFileName());
        try (ZipInputStream in = new ZipInputStream(Files.newInputStream(jar));
             java.util.zip.ZipOutputStream out = new java.util.zip.ZipOutputStream(Files.newOutputStream(tmp))) {
            ZipEntry e;
            while ((e = in.getNextEntry()) != null) {
                byte[] data = in.readAllBytes();
                if (e.getName().endsWith(".class")) {
                    org.objectweb.asm.ClassReader cr = new org.objectweb.asm.ClassReader(data);
                    org.objectweb.asm.ClassWriter cw = new org.objectweb.asm.ClassWriter(0);
                    cr.accept(new org.objectweb.asm.ClassVisitor(org.objectweb.asm.Opcodes.ASM9, cw) {
                        @Override
                        public org.objectweb.asm.FieldVisitor visitField(int access, String name, String desc, String sig, Object value) {
                            return super.visitField(widen(access, publicProtected), name, desc, sig, value);
                        }

                        @Override
                        public org.objectweb.asm.MethodVisitor visitMethod(int access, String name, String desc, String sig, String[] ex) {
                            org.objectweb.asm.MethodVisitor mv = super.visitMethod(widen(access, publicProtected), name, desc, sig, ex);
                            if (!strip) {
                                return mv;
                            }
                            return new org.objectweb.asm.MethodVisitor(org.objectweb.asm.Opcodes.ASM9, mv) {
                                @Override
                                public void visitAnnotableParameterCount(int parameterCount, boolean visible) {
                                }

                                @Override
                                public org.objectweb.asm.AnnotationVisitor visitParameterAnnotation(int parameter, String descriptor, boolean visible) {
                                    return null;
                                }
                            };
                        }
                    }, 0);
                    data = cw.toByteArray();
                }
                ZipEntry ne = new ZipEntry(e.getName());
                ne.setTime(0L);
                out.putNextEntry(ne);
                out.write(data);
                out.closeEntry();
            }
        }
        Files.move(tmp, jar, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
    }

    private static int widen(int access, boolean publicProtected) {
        if (publicProtected && (access & org.objectweb.asm.Opcodes.ACC_PROTECTED) != 0) {
            return (access & ~org.objectweb.asm.Opcodes.ACC_PROTECTED) | org.objectweb.asm.Opcodes.ACC_PUBLIC;
        }
        return access;
    }

    private static boolean rulesAllow(JsonArray rules) {
        if (rules == null) {
            return true;
        }
        boolean allowed = false;
        for (JsonElement e : rules) {
            JsonObject rule = e.getAsJsonObject();
            boolean matches = true;
            if (rule.has("os")) {
                JsonObject os = rule.getAsJsonObject("os");
                matches = !os.has("name") || os.get("name").getAsString().equals("linux");
            }
            if (matches) {
                allowed = rule.get("action").getAsString().equals("allow");
            }
        }
        return allowed;
    }

    private static void extractEntry(Path zip, String entryName, Path target) throws IOException {
        try (ZipInputStream zin = new ZipInputStream(Files.newInputStream(zip))) {
            ZipEntry e;
            while ((e = zin.getNextEntry()) != null) {
                if (e.getName().equals(entryName)) {
                    Files.createDirectories(target.getParent());
                    Files.copy(zin, target, StandardCopyOption.REPLACE_EXISTING);
                    return;
                }
            }
        }
        throw new IOException(entryName + " not found in " + zip);
    }

    /** Downloads {@code url} to {@code target} unless it already exists with the expected SHA-1. */
    private static void ensure(String url, String sha1, Path target) throws Exception {
        if (Files.exists(target) && sha1(target).equalsIgnoreCase(sha1)) {
            return;
        }
        download(url, sha1, target);
    }

    private static void download(String url, String sha1, Path target) throws Exception {
        System.out.println("[setup] GET " + url);
        Files.createDirectories(target.getParent());
        Path tmp = target.resolveSibling(target.getFileName() + ".part");
        HttpResponse<InputStream> resp = HTTP.send(HttpRequest.newBuilder(URI.create(url)).GET().build(),
                HttpResponse.BodyHandlers.ofInputStream());
        if (resp.statusCode() != 200) {
            throw new IOException("HTTP " + resp.statusCode() + " for " + url);
        }
        try (InputStream in = resp.body()) {
            Files.copy(in, tmp, StandardCopyOption.REPLACE_EXISTING);
        }
        String got = sha1(tmp);
        if (!got.equalsIgnoreCase(sha1)) {
            Files.delete(tmp);
            throw new IOException("SHA-1 mismatch for " + url + ": expected " + sha1 + ", got " + got);
        }
        Files.move(tmp, target, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
    }

    private static String fetchString(String url) throws Exception {
        HttpResponse<String> resp = HTTP.send(HttpRequest.newBuilder(URI.create(url)).GET().build(),
                HttpResponse.BodyHandlers.ofString());
        if (resp.statusCode() != 200) {
            throw new IOException("HTTP " + resp.statusCode() + " for " + url);
        }
        return resp.body();
    }

    static String sha1(Path p) throws IOException {
        try {
            MessageDigest md = MessageDigest.getInstance("SHA-1");
            try (InputStream in = Files.newInputStream(p)) {
                byte[] buf = new byte[1 << 16];
                int n;
                while ((n = in.read(buf)) > 0) {
                    md.update(buf, 0, n);
                }
            }
            return HexFormat.of().formatHex(md.digest());
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
        }
    }

    private static String require(Properties p, String key) {
        String v = p.getProperty(key);
        if (v == null || v.isBlank()) {
            throw new IllegalArgumentException("missing property " + key);
        }
        return v.trim();
    }
}
