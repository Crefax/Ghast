package ghast.oracle;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Version-agnostic description of one oracle run: a sparse world, the initial player state and the
 * per-tick input. Version harnesses turn it into real vanilla objects.
 */
public final class Scenario {
    /** Key bits (what a human holds down on that tick). */
    /**
     * Key bits (what a human holds down on that tick). {@code USE} is the use key (right mouse button)
     * held while looking at nothing; the harness puts a sword in the hand for it.
     */
    public static final int FORWARD = 1, BACK = 2, LEFT = 4, RIGHT = 8, JUMP = 16, SNEAK = 32, SPRINT = 64, USE = 128;
    public static final String[] KEY_NAMES = {"forward", "back", "left", "right", "jump", "sneak", "sprint", "use"};

    /** What the world contains outside explicitly loaded chunks. */
    public enum Outside {
        /** Every chunk column is loaded; anything not listed is air. */
        AIR,
        /** Only chunks that contain a listed block (or are listed in {@link #extraChunks}) are loaded. */
        UNLOADED
    }

    public enum GameMode { SURVIVAL, CREATIVE, ADVENTURE, SPECTATOR }

    public record Block(int x, int y, int z, int id, int meta) {}

    /** Inclusive cuboid fill; fills are applied in order, later ones overwrite earlier ones. */
    public record Fill(int x0, int y0, int z0, int x1, int y1, int z1, int id, int meta) {}

    /** A server packet applied at the start of a tick (before the keys / tick). */
    public sealed interface Event permits Velocity, Teleport, Abilities, SpeedAttribute, Effects {}

    /** Entity velocity packet: raw protocol units (1/8000 block per tick). */
    public record Velocity(int x, int y, int z) implements Event {}

    /**
     * Position/look packet. {@code flags} is the packet's relative-flags byte (bit 0 x, 1 y, 2 z, 3 yaw,
     * 4 pitch): set bits make that value relative to the current one; 0 = absolute.
     */
    public record Teleport(double x, double y, double z, float yaw, float pitch, int flags) implements Event {
        public Teleport(double x, double y, double z, float yaw, float pitch) {
            this(x, y, z, yaw, pitch, 0);
        }
    }

    /** Player abilities packet. */
    public record Abilities(boolean invulnerable, boolean flying, boolean allowFlying, boolean creative,
                            float flySpeed, float walkSpeed) implements Event {}

    /** One attribute modifier as sent by the server. */
    public record Modifier(java.util.UUID uuid, double amount, int operation) {}

    /**
     * Entity properties packet for the player's movement speed: the base value is set and ALL modifiers
     * are replaced by these, in this order (the server's copy of the sprint modifier included if sent).
     */
    public record SpeedAttribute(double base, List<Modifier> modifiers) implements Event {}

    /** The player's full active effect set after an add/remove effect packet: {id, amplifier} pairs. */
    public record Effects(List<int[]> effects) implements Event {}

    public record Tick(int keys, float yaw, float pitch, List<Event> events) {}

    public final String name;
    public String description = "";
    public Outside outside = Outside.AIR;
    /** Chunk columns (x, z) loaded in addition to the ones holding blocks (UNLOADED mode only). */
    public final List<int[]> extraChunks = new ArrayList<>();
    /** World edits in order (a single block is a 1x1x1 fill). */
    public final List<Fill> fills = new ArrayList<>();

    // Initial player state (applied like the join + abilities + position packets of a real login).
    public GameMode gameMode = GameMode.SURVIVAL;
    public double x, y, z;
    public float yaw, pitch;
    public double vx, vy, vz;
    public boolean onGround;
    public float fallDistance;
    public boolean sprinting;
    public boolean flying;
    /** null = what the game mode implies. */
    public Boolean allowFlying;
    public float flySpeed = 0.05f;
    public float walkSpeed = 0.1f;
    /** Base value of the movement speed attribute (what the server's entity properties packet sets). */
    public double movementSpeedBase = 0.1;
    public int foodLevel = 20;
    /** Status effects: id -> amplifier (speed = 1, slowness = 2, jump boost = 8). */
    public final Map<Integer, Integer> effects = new LinkedHashMap<>();

    /**
     * Optional exact bounding box {minX, minY, minZ, maxX, maxY, maxZ} after the position packet. When
     * set it replaces the box the game derives from the position, so a run restarted from a recorded
     * state continues bit for bit (the box carries rounding history the position does not).
     */
    public double[] bb;
    /** 1.16.5+: optional pose name ("standing", "crouching", "swimming", ...) set before the box. */
    public String pose;

    // Used by the 1.16.5+ harnesses only (ignored by 1.8.9).
    /** The client's auto-jump option (vanilla default true; bots turn it off). */
    public boolean autoJump;
    /** Enchantment levels of the boots the player wears (0 = no boots). */
    public int soulSpeed, depthStrider;
    /** Extra movement speed attribute modifiers {amount, operation} as an entity properties packet sends them. */
    public final List<double[]> speedModifiers = new ArrayList<>();

    /** 26.1+: one attribute modifier of an update-attributes packet (attribute and modifier ids are identifiers). */
    public record AttributeModifierSpec(String attribute, String id, double amount, int operation) {}

    /** 26.1+: attribute base values the server sends at setup (attribute id -> base), e.g. step height, gravity, scale. */
    public final Map<String, Double> attributeBases = new LinkedHashMap<>();
    /** 26.1+: extra attribute modifiers the server sends at setup (other than the movement speed ones above). */
    public final List<AttributeModifierSpec> attributeModifiers = new ArrayList<>();

    public final List<Tick> ticks = new ArrayList<>();

    /** Whether any tick holds the use key. */
    public boolean usesItem() {
        for (Tick t : ticks) {
            if ((t.keys() & USE) != 0) {
                return true;
            }
        }
        return false;
    }

    public Scenario(String name) {
        this.name = name;
    }

    public static long posKey(int x, int y, int z) {
        return ((long) (x & 0x3ffffff) << 38) | ((long) (z & 0x3ffffff) << 12) | (y & 0xfff);
    }

    // ---- builder helpers -------------------------------------------------------------------------

    public Scenario desc(String d) {
        this.description = d;
        return this;
    }

    public Scenario outside(Outside o) {
        this.outside = o;
        return this;
    }

    public Scenario block(int x, int y, int z, int id, int meta) {
        return fill(x, y, z, x, y, z, id, meta);
    }

    public Scenario block(int x, int y, int z, int id) {
        return block(x, y, z, id, 0);
    }

    /** Fills the inclusive box with one block. */
    public Scenario fill(int x0, int y0, int z0, int x1, int y1, int z1, int id, int meta) {
        fills.add(new Fill(Math.min(x0, x1), Math.min(y0, y1), Math.min(z0, z1),
                Math.max(x0, x1), Math.max(y0, y1), Math.max(z0, z1), id, meta));
        return this;
    }

    /** The resulting non-air blocks (after applying all fills in order). */
    public Map<Long, Block> blocks() {
        Map<Long, Block> m = new LinkedHashMap<>();
        for (Fill f : fills) {
            for (int x = f.x0(); x <= f.x1(); x++) {
                for (int y = f.y0(); y <= f.y1(); y++) {
                    for (int z = f.z0(); z <= f.z1(); z++) {
                        long k = posKey(x, y, z);
                        m.remove(k);
                        m.put(k, new Block(x, y, z, f.id(), f.meta()));
                    }
                }
            }
        }
        return m;
    }

    public Scenario fill(int x0, int y0, int z0, int x1, int y1, int z1, int id) {
        return fill(x0, y0, z0, x1, y1, z1, id, 0);
    }

    public Scenario spawn(double x, double y, double z, float yaw, float pitch) {
        this.x = x;
        this.y = y;
        this.z = z;
        this.yaw = yaw;
        this.pitch = pitch;
        return this;
    }

    public Scenario mode(GameMode m) {
        this.gameMode = m;
        return this;
    }

    public Scenario flying(boolean f) {
        this.flying = f;
        return this;
    }

    public Scenario velocity(double vx, double vy, double vz) {
        this.vx = vx;
        this.vy = vy;
        this.vz = vz;
        return this;
    }

    public Scenario effect(int id, int amplifier) {
        effects.put(id, amplifier);
        return this;
    }

    /** Appends {@code n} ticks with the same keys and the current look. */
    public Scenario run(int n, int keys) {
        return run(n, keys, lastYaw(), lastPitch());
    }

    public Scenario run(int n, int keys, float yaw, float pitch) {
        for (int i = 0; i < n; i++) {
            ticks.add(new Tick(keys, yaw, pitch, List.of()));
        }
        return this;
    }

    /** Appends one tick that first applies the given server events. */
    public Scenario event(int keys, Event... events) {
        ticks.add(new Tick(keys, lastYaw(), lastPitch(), List.of(events)));
        return this;
    }

    /** Appends {@code n} ticks turning the yaw by {@code dyaw} each tick. */
    public Scenario turn(int n, int keys, float dyaw, float dpitch) {
        float yaw = lastYaw();
        float pitch = lastPitch();
        for (int i = 0; i < n; i++) {
            yaw += dyaw;
            pitch += dpitch;
            ticks.add(new Tick(keys, yaw, pitch, List.of()));
        }
        return this;
    }

    public float lastYaw() {
        return ticks.isEmpty() ? yaw : ticks.get(ticks.size() - 1).yaw();
    }

    public float lastPitch() {
        return ticks.isEmpty() ? pitch : ticks.get(ticks.size() - 1).pitch();
    }

    public static String keyNames(int keys) {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < KEY_NAMES.length; i++) {
            if ((keys & (1 << i)) != 0) {
                if (sb.length() > 0) {
                    sb.append(',');
                }
                sb.append(KEY_NAMES[i]);
            }
        }
        return sb.toString();
    }
}
