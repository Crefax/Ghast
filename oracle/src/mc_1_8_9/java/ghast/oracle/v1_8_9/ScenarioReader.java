package ghast.oracle.v1_8_9;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import ghast.oracle.Scenario;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;
import java.util.zip.GZIPInputStream;

/**
 * Reads scenarios back from golden-format JSON lines: the {@code scenario} header plus the
 * {@code input} of each tick record (states and packets are ignored). This lets another tool (the
 * Rust tests, a minimiser) write its own cases and have vanilla run them via {@code replay}.
 */
final class ScenarioReader {
    private ScenarioReader() {}

    static void read(Path in, Consumer<Scenario> out) throws IOException {
        InputStream is = Files.newInputStream(in);
        if (in.toString().endsWith(".gz")) {
            is = new GZIPInputStream(is, 1 << 16);
        }
        Scenario cur = null;
        int expected = 0;
        try (BufferedReader r = new BufferedReader(new InputStreamReader(is, StandardCharsets.UTF_8), 1 << 16)) {
            String line;
            JsonParser parser = new JsonParser();
            while ((line = r.readLine()) != null) {
                if (line.isBlank()) {
                    continue;
                }
                JsonObject o = parser.parse(line).getAsJsonObject();
                String type = o.get("type").getAsString();
                if (type.equals("scenario")) {
                    if (cur != null) {
                        finish(cur, expected, out);
                    }
                    cur = header(o);
                    expected = o.has("ticks") ? o.get("ticks").getAsInt() : -1;
                } else if (type.equals("tick") && o.has("input")) {
                    if (cur == null) {
                        throw new IOException("tick before scenario header");
                    }
                    cur.ticks.add(tick(o.getAsJsonObject("input")));
                }
            }
        }
        if (cur != null) {
            finish(cur, expected, out);
        }
    }

    private static void finish(Scenario sc, int expected, Consumer<Scenario> out) throws IOException {
        if (expected >= 0 && expected != sc.ticks.size()) {
            throw new IOException(sc.name + ": header says " + expected + " ticks, found " + sc.ticks.size());
        }
        out.accept(sc);
    }

    private static Scenario header(JsonObject o) {
        Scenario sc = new Scenario(o.get("name").getAsString());
        if (o.has("description")) {
            sc.desc(o.get("description").getAsString());
        }
        sc.outside(Scenario.Outside.valueOf(o.get("outside").getAsString().toUpperCase()));
        if (sc.outside == Scenario.Outside.UNLOADED && o.has("loaded_chunks")) {
            for (JsonElement e : o.getAsJsonArray("loaded_chunks")) {
                JsonArray r = e.getAsJsonArray();
                for (int x = r.get(0).getAsInt(); x <= r.get(2).getAsInt(); x++) {
                    for (int z = r.get(1).getAsInt(); z <= r.get(3).getAsInt(); z++) {
                        sc.extraChunks.add(new int[] {x, z});
                    }
                }
            }
        }
        for (JsonElement e : o.getAsJsonArray("fills")) {
            JsonArray f = e.getAsJsonArray();
            sc.fill(f.get(0).getAsInt(), f.get(1).getAsInt(), f.get(2).getAsInt(), f.get(3).getAsInt(),
                    f.get(4).getAsInt(), f.get(5).getAsInt(), f.get(6).getAsInt(), f.get(7).getAsInt());
        }
        JsonObject s = o.getAsJsonObject("setup");
        sc.gameMode = Scenario.GameMode.valueOf(s.get("game_mode").getAsString().toUpperCase());
        JsonArray pos = s.getAsJsonArray("pos");
        JsonArray vel = s.getAsJsonArray("vel");
        sc.spawn(f64(pos.get(0)), f64(pos.get(1)), f64(pos.get(2)), f32(s.get("yaw")), f32(s.get("pitch")));
        sc.velocity(f64(vel.get(0)), f64(vel.get(1)), f64(vel.get(2)));
        sc.onGround = s.get("on_ground").getAsBoolean();
        sc.fallDistance = f32(s.get("fall_distance"));
        sc.sprinting = s.get("sprinting").getAsBoolean();
        sc.flying = s.get("flying").getAsBoolean();
        if (s.has("allow_flying")) {
            sc.allowFlying = s.get("allow_flying").getAsBoolean();
        }
        sc.flySpeed = f32(s.get("fly_speed"));
        sc.walkSpeed = f32(s.get("walk_speed"));
        sc.movementSpeedBase = f64(s.get("movement_speed_base"));
        sc.foodLevel = s.get("food_level").getAsInt();
        if (s.has("speed_modifiers")) {
            for (JsonElement e : s.getAsJsonArray("speed_modifiers")) {
                sc.speedModifiers.add(new double[] {f64(e.getAsJsonArray().get(0)), e.getAsJsonArray().get(1).getAsInt()});
            }
        }
        if (s.has("depth_strider")) {
            sc.depthStrider = s.get("depth_strider").getAsInt();
        }
        if (s.has("bb")) {
            JsonArray b = s.getAsJsonArray("bb");
            sc.bb = new double[6];
            for (int i = 0; i < 6; i++) {
                sc.bb[i] = f64(b.get(i));
            }
        }
        if (s.has("pose")) {
            sc.pose = s.get("pose").getAsString();
        }
        for (JsonElement e : s.getAsJsonArray("effects")) {
            sc.effect(e.getAsJsonArray().get(0).getAsInt(), e.getAsJsonArray().get(1).getAsInt());
        }
        return sc;
    }

    private static Scenario.Tick tick(JsonObject in) {
        int keys = 0;
        String k = in.get("keys").getAsString();
        if (!k.isEmpty()) {
            for (String name : k.split(",")) {
                int bit = List.of(Scenario.KEY_NAMES).indexOf(name);
                if (bit < 0) {
                    throw new IllegalArgumentException("unknown key " + name);
                }
                keys |= 1 << bit;
            }
        }
        List<Scenario.Event> events = new ArrayList<>();
        if (in.has("events")) {
            for (JsonElement e : in.getAsJsonArray("events")) {
                JsonObject ev = e.getAsJsonObject();
                switch (ev.get("type").getAsString()) {
                    case "velocity" -> events.add(new Scenario.Velocity(ev.get("x").getAsInt(), ev.get("y").getAsInt(),
                            ev.get("z").getAsInt()));
                    case "teleport" -> {
                        JsonArray p = ev.getAsJsonArray("pos");
                        events.add(new Scenario.Teleport(f64(p.get(0)), f64(p.get(1)), f64(p.get(2)),
                                f32(ev.get("yaw")), f32(ev.get("pitch")), ev.has("flags") ? ev.get("flags").getAsInt() : 0));
                    }
                    case "abilities" -> events.add(new Scenario.Abilities(ev.get("invulnerable").getAsBoolean(),
                            ev.get("flying").getAsBoolean(), ev.get("allow_flying").getAsBoolean(),
                            ev.get("creative").getAsBoolean(), f32(ev.get("fly_speed")), f32(ev.get("walk_speed"))));
                    case "speed_attribute" -> {
                        List<Scenario.Modifier> mods = new ArrayList<>();
                        for (JsonElement m : ev.getAsJsonArray("modifiers")) {
                            JsonArray a = m.getAsJsonArray();
                            mods.add(new Scenario.Modifier(java.util.UUID.fromString(a.get(0).getAsString()), f64(a.get(1)),
                                    a.get(2).getAsInt()));
                        }
                        events.add(new Scenario.SpeedAttribute(f64(ev.get("base")), List.copyOf(mods)));
                    }
                    case "effects" -> {
                        List<int[]> fx = new ArrayList<>();
                        for (JsonElement f : ev.getAsJsonArray("effects")) {
                            fx.add(new int[] {f.getAsJsonArray().get(0).getAsInt(), f.getAsJsonArray().get(1).getAsInt()});
                        }
                        events.add(new Scenario.Effects(List.copyOf(fx)));
                    }
                    default -> throw new IllegalArgumentException("unknown event " + ev);
                }
            }
        }
        return new Scenario.Tick(keys, f32(in.get("yaw")), f32(in.get("pitch")), List.copyOf(events));
    }

    /** Accepts {@code [decimal, "hex"]} (hex wins) or a plain number. */
    static double f64(JsonElement e) {
        if (e.isJsonArray()) {
            return Double.longBitsToDouble(Long.parseUnsignedLong(e.getAsJsonArray().get(1).getAsString(), 16));
        }
        return e.getAsDouble();
    }

    static float f32(JsonElement e) {
        if (e.isJsonArray()) {
            return Float.intBitsToFloat(Integer.parseUnsignedInt(e.getAsJsonArray().get(1).getAsString(), 16));
        }
        return e.getAsFloat();
    }
}
