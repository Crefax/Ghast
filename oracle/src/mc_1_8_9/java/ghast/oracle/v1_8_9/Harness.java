package ghast.oracle.v1_8_9;

import com.mojang.authlib.GameProfile;
import ghast.oracle.Scenario;
import net.minecraft.Bootstrap;
import net.minecraft.block.Block;
import net.minecraft.client.MinecraftClient;
import net.minecraft.client.input.KeyboardInput;
import net.minecraft.client.network.ClientPlayNetworkHandler;
import net.minecraft.client.network.ClientPlayerInteractionManager;
import net.minecraft.client.network.PlayerListEntry;
import net.minecraft.enchantment.Enchantment;
import net.minecraft.network.packet.s2c.play.PlayerListS2CPacket;
import net.minecraft.client.option.GameOptions;
import net.minecraft.client.option.KeyBinding;
import net.minecraft.client.world.ClientWorld;
import net.minecraft.entity.Entity;
import net.minecraft.entity.attribute.AttributeModifier;
import net.minecraft.entity.attribute.EntityAttributeInstance;
import net.minecraft.entity.attribute.EntityAttributes;
import net.minecraft.entity.effect.StatusEffectInstance;
import net.minecraft.entity.player.ClientPlayerEntity;
import net.minecraft.item.ItemStack;
import net.minecraft.item.Items;
import net.minecraft.stat.StatHandler;
import net.minecraft.util.profiler.Profiler;
import net.minecraft.world.Difficulty;
import net.minecraft.world.chunk.Chunk;
import net.minecraft.world.chunk.ChunkSection;
import net.minecraft.world.level.LevelGeneratorType;
import net.minecraft.world.level.LevelInfo;

import java.util.ArrayList;
import java.util.Collections;
import java.util.IdentityHashMap;
import java.util.Set;
import java.util.List;
import java.util.Random;
import java.util.TreeSet;
import java.util.UUID;

/**
 * Runs a {@link Scenario} through the real 1.8.9 client code.
 *
 * <p>What is real: {@code ClientWorld} with its {@code ClientChunkProvider} and real {@code Chunk}s,
 * {@code ClientPlayerEntity} created by the real {@code ClientPlayerInteractionManager}, the real
 * {@code KeyboardInput} reading real {@code KeyBinding}s, and each tick is the real
 * {@code World.tickEntities()} (which runs {@code ClientPlayerEntity.tick()}: tickMovement with sprint /
 * fly toggles, travel, move + collisions, block effects, and {@code sendMovementPackets}).
 *
 * <p>What is faked: the {@code MinecraftClient} singleton is allocated without its constructor and only
 * the fields the tick touches are set (options, world, player, interaction manager, camera entity);
 * the network handler captures outgoing packets instead of sending them. Server packets that set up
 * the player (join, abilities, position) are applied the way their client handlers do it.
 */
final class Harness {
    static final String PLAYER_NAME = "oracle";
    static final UUID PLAYER_UUID = new UUID(0x6768617374L, 0x6f7261636c65L); // "ghast" / "oracle"

    private static MinecraftClient client;
    private static GameOptions options;

    private Harness() {}

    /** One-time JVM setup: registries + the fake client singleton. */
    static synchronized void init() {
        if (client != null) {
            return;
        }
        Bootstrap.initialize();
        client = Reflect.allocate(MinecraftClient.class);
        Reflect.set(MinecraftClient.class, null, "instance", client);
        options = new GameOptions();
        client.options = options;
        Reflect.set(MinecraftClient.class, client, "soundManager", SilentSoundManager.create());
        // The real client only calls this when options are loaded; it fills the code -> binding map.
        KeyBinding.updateKeysByCode();
        snapshotBlockBounds();
    }

    private static final String[] BOUNDS = {"boundingBoxMinX", "boundingBoxMinY", "boundingBoxMinZ",
            "boundingBoxMaxX", "boundingBoxMaxY", "boundingBoxMaxZ"};
    private static final List<Block> BOUNDS_BLOCKS = new ArrayList<>();
    private static final List<double[]> BOUNDS_VALUES = new ArrayList<>();

    /**
     * 1.8 blocks keep their shape in mutable per-Block fields that some code paths overwrite and do not
     * reset (anvils are the case that matters: their collision box is whatever bounds were set last).
     * The state right after bootstrap is snapshotted and restored before every run, so every scenario
     * sees the same block state as a fresh JVM, independent of what ran before it.
     */
    private static void snapshotBlockBounds() {
        for (Block b : Block.REGISTRY) {
            double[] v = new double[6];
            for (int i = 0; i < 6; i++) {
                v[i] = Reflect.getDouble(Block.class, b, BOUNDS[i]);
            }
            BOUNDS_BLOCKS.add(b);
            BOUNDS_VALUES.add(v);
        }
    }

    static void restoreBlockBounds() {
        for (int k = 0; k < BOUNDS_BLOCKS.size(); k++) {
            double[] v = BOUNDS_VALUES.get(k);
            Block b = BOUNDS_BLOCKS.get(k);
            for (int i = 0; i < 6; i++) {
                Reflect.setDouble(Block.class, b, BOUNDS[i], v[i]);
            }
            // Stairs keep their multi-pass ray test state in the Block too.
            if (b instanceof net.minecraft.block.StairsBlock) {
                Reflect.set(net.minecraft.block.StairsBlock.class, b, "isRayTraced", false);
                Reflect.setInt(net.minecraft.block.StairsBlock.class, b, "rayTracePass", 0);
            }
        }
    }

    /** A live run: world + player built from a scenario. */
    static final class Run {
        final Scenario sc;
        final ClientWorld world;
        final ClientPlayerEntity player;
        final CapturingNetworkHandler net;
        final List<int[]> loadedRects = new ArrayList<>();
        int tick;

        Run(Scenario sc) {
            init();
            this.sc = sc;
            releaseKeys();
            restoreBlockBounds();
            client.player = null;
            client.world = null;
            Reflect.set(MinecraftClient.class, client, "cameraEntity", null);

            GameProfile profile = new GameProfile(PLAYER_UUID, PLAYER_NAME);
            net = new CapturingNetworkHandler(client, profile);

            // --- join game (ClientPlayNetworkHandler.onGameJoin + MinecraftClient.connect) ---
            client.interactionManager = new ClientPlayerInteractionManager(client, net);
            LevelInfo info = new LevelInfo(0L, gameMode(sc.gameMode), false, false, LevelGeneratorType.FLAT);
            world = new ClientWorld(net, info, 0, Difficulty.NORMAL, new Profiler());
            world.random.setSeed(0L);
            Reflect.set(net.getClass().getSuperclass(), net, "world", world);
            client.world = world;
            player = client.interactionManager.createPlayer(world, new StatHandler());
            client.interactionManager.flipPlayer(player);
            client.player = player;
            player.afterSpawn();
            world.spawnEntity(player);
            player.input = new KeyboardInput(options);
            client.interactionManager.copyAbilities(player);
            Reflect.set(MinecraftClient.class, client, "cameraEntity", player);
            Reflect.set(Entity.class, player, "random", new Random(0L));
            client.interactionManager.setGameMode(gameMode(sc.gameMode));
            // Tab list entry (player list packet): AbstractClientPlayerEntity.isSpectator() reads it.
            @SuppressWarnings("unchecked")
            java.util.Map<UUID, PlayerListEntry> tab =
                    (java.util.Map<UUID, PlayerListEntry>) Reflect.get(ClientPlayNetworkHandler.class, net, "playerListEntries");
            PlayerListS2CPacket list = new PlayerListS2CPacket();
            tab.put(PLAYER_UUID, new PlayerListEntry(list.new Entry(profile, 0, gameMode(sc.gameMode), null)));

            // --- chunk data ---
            loadChunks();
            placeBlocks();

            // --- abilities packet (what the server sends after join) ---
            player.abilities.flying = sc.flying;
            if (sc.allowFlying != null) {
                player.abilities.allowFlying = sc.allowFlying;
            }
            player.abilities.setFlySpeed(sc.flySpeed);
            player.abilities.setWalkSpeed(sc.walkSpeed);

            // --- entity properties / effects / health packets ---
            EntityAttributeInstance speed = player.initializeAttribute(EntityAttributes.GENERIC_MOVEMENT_SPEED);
            // ClientPlayNetworkHandler.onEntityAttributes: base value, all modifiers replaced by the packet's.
            speed.setBaseValue(sc.movementSpeedBase);
            if (!sc.speedModifiers.isEmpty()) {
                speed.clearModifiers();
                for (int i = 0; i < sc.speedModifiers.size(); i++) {
                    double[] m = sc.speedModifiers.get(i);
                    speed.addModifier(new AttributeModifier(new UUID(0x6f7261636c65L, i), "Unknown synced attribute modifier",
                            m[0], (int) m[1]));
                }
            }
            if (sc.depthStrider > 0) {
                // Boots in the armor slot (window items / equipment from the server).
                ItemStack boots = new ItemStack(Items.DIAMOND_BOOTS);
                boots.addEnchantment(Enchantment.DEPTH_STRIDER, sc.depthStrider);
                player.inventory.armor[0] = boots;
            }
            sc.effects.forEach((id, amp) -> player.addStatusEffect(new StatusEffectInstance(id, 1_000_000, amp)));
            player.getHungerManager().setFoodLevel(sc.foodLevel);
            if (sc.usesItem()) {
                // Held in the selected hotbar slot (0); swords block for 72000 ticks when used.
                player.inventory.main[player.inventory.selectedSlot] = new ItemStack(Items.DIAMOND_SWORD);
            }

            // --- position packet (absolute, like a login teleport) ---
            player.updatePositionAndAngles(sc.x, sc.y, sc.z, sc.yaw, sc.pitch);
            player.prevX = player.x;
            player.prevY = player.y;
            player.prevZ = player.z;

            if (sc.bb != null) {
                player.setBoundingBox(new net.minecraft.util.math.Box(sc.bb[0], sc.bb[1], sc.bb[2], sc.bb[3], sc.bb[4], sc.bb[5]));
            }

            // --- scenario overrides of the dynamic state ---
            player.velocityX = sc.vx;
            player.velocityY = sc.vy;
            player.velocityZ = sc.vz;
            player.onGround = sc.onGround;
            player.fallDistance = sc.fallDistance;
            if (sc.sprinting) {
                player.setSprinting(true);
            }
            // The player was spawned before its chunk existed (like a real join, where it sits in the
            // empty chunk until terrain arrives). Do the chunk bookkeeping an earlier tick would have
            // done, without ticking, so tick 1 of the scenario is a normal tick.
            world.checkChunk(player, false);
            net.sent.clear();
        }

        private void loadChunks() {
            TreeSet<Long> chunks = new TreeSet<>();
            if (sc.outside == Scenario.Outside.AIR) {
                int minX = (int) Math.floor(sc.x) >> 4, maxX = minX, minZ = (int) Math.floor(sc.z) >> 4, maxZ = minZ;
                for (Scenario.Fill f : sc.fills) {
                    minX = Math.min(minX, f.x0() >> 4);
                    maxX = Math.max(maxX, f.x1() >> 4);
                    minZ = Math.min(minZ, f.z0() >> 4);
                    maxZ = Math.max(maxZ, f.z1() >> 4);
                }
                int margin = Math.min(16, 2 + sc.ticks.size() / 12);
                minX -= margin;
                minZ -= margin;
                maxX += margin;
                maxZ += margin;
                for (int cx = minX; cx <= maxX; cx++) {
                    for (int cz = minZ; cz <= maxZ; cz++) {
                        chunks.add(chunkKey(cx, cz));
                    }
                }
                loadedRects.add(new int[] {minX, minZ, maxX, maxZ});
            } else {
                for (Scenario.Block b : sc.blocks().values()) {
                    chunks.add(chunkKey(b.x() >> 4, b.z() >> 4));
                }
                for (int[] c : sc.extraChunks) {
                    chunks.add(chunkKey(c[0], c[1]));
                }
                for (long k : chunks) {
                    int cx = (int) (k >> 32), cz = (int) k;
                    loadedRects.add(new int[] {cx, cz, cx, cz});
                }
            }
            for (long k : chunks) {
                world.handleChunk((int) (k >> 32), (int) k, true);
            }
        }

        /**
         * Writes raw {@code id << 4 | meta} values into the chunk sections, exactly like chunk data
         * packets do: a meta the block does not use decodes to air, as on a real client.
         */
        private void placeBlocks() {
            Set<ChunkSection> touched = Collections.newSetFromMap(new IdentityHashMap<>());
            for (Scenario.Block b : sc.blocks().values()) {
                touched.add(writeRaw(b.x(), b.y(), b.z(), b.id() << 4 | (b.meta() & 15)));
            }
            for (ChunkSection s : touched) {
                s.calculateCounts();
            }
        }

        /** Writes one raw block value into its (loaded) chunk; call calculateCounts() on the result. */
        ChunkSection writeRaw(int x, int y, int z, int raw) {
            if (y < 0 || y > 255) {
                throw new IllegalArgumentException("block outside world height: " + x + "," + y + "," + z);
            }
            Chunk chunk = world.getChunk(x >> 4, z >> 4);
            if (chunk.isEmpty()) {
                throw new IllegalArgumentException("chunk not loaded for " + x + "," + y + "," + z);
            }
            ChunkSection[] sections = chunk.getBlockStorage();
            ChunkSection s = sections[y >> 4];
            if (s == null) {
                s = new ChunkSection(y & ~15, !world.dimension.hasNoSkylight());
                sections[y >> 4] = s;
            }
            s.getBlockStates()[(y & 15) << 8 | (z & 15) << 4 | (x & 15)] = (char) raw;
            return s;
        }

        void setRaw(int x, int y, int z, int raw) {
            writeRaw(x, y, z, raw).calculateCounts();
        }

        /** One client tick. Events are inbound packets handled before the tick (phase 1 of the client loop). */
        void step(Scenario.Tick t) {
            for (Scenario.Event e : t.events()) {
                applyEvent(e);
            }
            setKeys(t.keys());
            useKey((t.keys() & Scenario.USE) != 0);
            // Mouse look is applied by the renderer between ticks.
            player.yaw = t.yaw();
            player.pitch = t.pitch();
            world.tickEntities();
            tick++;
        }

        private boolean useHeld;

        /**
         * The use key as MinecraftClient.tick handles it (before the world tick) when the crosshair hits
         * nothing: pressing it uses the held item (interactItem: sword -> blocking), releasing it while
         * the item is in use calls stopUsingItem. Both send their packets like the real client.
         */
        private void useKey(boolean held) {
            if (held && !useHeld && !player.isUsingItem()) {
                ItemStack stack = player.inventory.getMainHandStack();
                if (stack != null) {
                    client.interactionManager.interactItem(player, world, stack);
                }
            } else if (!held && useHeld && player.isUsingItem()) {
                client.interactionManager.stopUsingItem(player);
            }
            useHeld = held;
        }

        private void applyEvent(Scenario.Event e) {
            if (e instanceof Scenario.Velocity v) {
                // ClientPlayNetworkHandler.onVelocityUpdate
                player.setVelocityClient(v.x() / 8000.0, v.y() / 8000.0, v.z() / 8000.0);
            } else if (e instanceof Scenario.Teleport tp) {
                // ClientPlayNetworkHandler.onPlayerPositionLook: a relative component is added to the
                // current value, an absolute one zeroes that velocity component.
                int f = tp.flags();
                double x = tp.x(), y = tp.y(), z = tp.z();
                float yaw = tp.yaw(), pitch = tp.pitch();
                if ((f & 1) != 0) {
                    x += player.x;
                } else {
                    player.velocityX = 0.0;
                }
                if ((f & 2) != 0) {
                    y += player.y;
                } else {
                    player.velocityY = 0.0;
                }
                if ((f & 4) != 0) {
                    z += player.z;
                } else {
                    player.velocityZ = 0.0;
                }
                if ((f & 16) != 0) {
                    pitch += player.pitch;
                }
                if ((f & 8) != 0) {
                    yaw += player.yaw;
                }
                player.updatePositionAndAngles(x, y, z, yaw, pitch);
                net.sent.add(new net.minecraft.network.packet.c2s.play.PlayerMoveC2SPacket.Both(
                        player.x, player.getBoundingBox().minY, player.z, player.yaw, player.pitch, false));
            } else if (e instanceof Scenario.SpeedAttribute sa) {
                // ClientPlayNetworkHandler.onEntityAttributes for generic.movementSpeed
                EntityAttributeInstance speed = player.initializeAttribute(EntityAttributes.GENERIC_MOVEMENT_SPEED);
                speed.setBaseValue(sa.base());
                speed.clearModifiers();
                for (Scenario.Modifier m : sa.modifiers()) {
                    speed.addModifier(new AttributeModifier(m.uuid(), "Unknown synced attribute modifier", m.amount(), m.operation()));
                }
            } else if (e instanceof Scenario.Effects fx) {
                // onRemoveEntityEffect for effects no longer present, onEntityPotionEffect for the set.
                java.util.Set<Integer> keep = new java.util.HashSet<>();
                for (int[] f : fx.effects()) {
                    keep.add(f[0]);
                }
                for (Object o : new java.util.ArrayList<>(player.getStatusEffectInstances())) {
                    StatusEffectInstance inst = (StatusEffectInstance) o;
                    if (!keep.contains(inst.getEffectId())) {
                        player.removeEffect(inst.getEffectId());
                    }
                }
                for (int[] f : fx.effects()) {
                    player.addStatusEffect(new StatusEffectInstance(f[0], 1_000_000, f[1]));
                }
            } else if (e instanceof Scenario.Abilities a) {
                // ClientPlayNetworkHandler.onPlayerAbilities
                player.abilities.flying = a.flying();
                player.abilities.creativeMode = a.creative();
                player.abilities.invulnerable = a.invulnerable();
                player.abilities.allowFlying = a.allowFlying();
                player.abilities.setFlySpeed(a.flySpeed());
                player.abilities.setWalkSpeed(a.walkSpeed());
            }
        }
    }

    static void releaseKeys() {
        setKeys(0);
    }

    static void setKeys(int keys) {
        KeyBinding[] order = {options.forwardKey, options.backKey, options.leftKey, options.rightKey,
                options.jumpKey, options.sneakKey, options.sprintKey};
        for (int i = 0; i < order.length; i++) {
            // The same call the LWJGL keyboard loop makes.
            KeyBinding.setKeyPressed(order[i].getCode(), (keys & (1 << i)) != 0);
        }
    }

    static LevelInfo.GameMode gameMode(Scenario.GameMode m) {
        return switch (m) {
            case SURVIVAL -> LevelInfo.GameMode.SURVIVAL;
            case CREATIVE -> LevelInfo.GameMode.CREATIVE;
            case ADVENTURE -> LevelInfo.GameMode.ADVENTURE;
            case SPECTATOR -> LevelInfo.GameMode.SPECTATOR;
        };
    }

    static long chunkKey(int cx, int cz) {
        return ((long) cx << 32) | (cz & 0xffffffffL);
    }
}
