package ghast.oracle.v1_12_2;

import com.mojang.authlib.GameProfile;
import ghast.oracle.Scenario;
import net.minecraft.Bootstrap;
import net.minecraft.block.Block;
import net.minecraft.block.BlockState;
import net.minecraft.block.Blocks;
import net.minecraft.class_3316;
import net.minecraft.class_3355;
import net.minecraft.client.MinecraftClient;
import net.minecraft.client.input.KeyboardInput;
import net.minecraft.client.network.ClientPlayNetworkHandler;
import net.minecraft.client.network.ClientPlayerInteractionManager;
import net.minecraft.client.network.PlayerListEntry;
import net.minecraft.client.option.GameOptions;
import net.minecraft.client.option.KeyBinding;
import net.minecraft.client.world.ClientWorld;
import net.minecraft.enchantment.Enchantments;
import net.minecraft.entity.Entity;
import net.minecraft.entity.attribute.AttributeModifier;
import net.minecraft.entity.attribute.EntityAttributeInstance;
import net.minecraft.entity.attribute.EntityAttributes;
import net.minecraft.entity.effect.StatusEffect;
import net.minecraft.entity.effect.StatusEffectInstance;
import net.minecraft.entity.player.ClientPlayerEntity;
import net.minecraft.item.ItemStack;
import net.minecraft.item.Items;
import net.minecraft.network.packet.c2s.play.PlayerMoveC2SPacket;
import net.minecraft.network.packet.c2s.play.TeleportConfirmC2SPacket;
import net.minecraft.network.packet.s2c.play.PlayerListS2CPacket;
import net.minecraft.stat.StatHandler;
import net.minecraft.util.Hand;
import net.minecraft.util.profiler.Profiler;
import net.minecraft.world.Difficulty;
import net.minecraft.world.GameMode;
import net.minecraft.world.chunk.Chunk;
import net.minecraft.world.chunk.ChunkSection;
import net.minecraft.world.level.LevelGeneratorType;
import net.minecraft.world.level.LevelInfo;

import java.util.ArrayList;
import java.util.List;
import java.util.Random;
import java.util.TreeSet;
import java.util.UUID;

/**
 * Runs a {@link Scenario} through the real 1.12.2 client code.
 *
 * <p>What is real: {@code ClientWorld} with its {@code ClientChunkProvider} and real {@code Chunk}s,
 * {@code ClientPlayerEntity} created by the real {@code ClientPlayerInteractionManager}, the real
 * {@code KeyboardInput} reading real {@code KeyBinding}s, and each tick is the real
 * {@code World.tickEntities()} (which runs {@code ClientPlayerEntity.tick()}: tickMovement with sprint /
 * fly toggles, travel, move + collisions, block effects, the size update and {@code sendMovementPackets}).
 *
 * <p>What is faked: the {@code MinecraftClient} singleton is allocated without its constructor and only
 * the fields the tick touches are set (options, world, player, interaction manager, camera entity, the
 * tutorial manager); the network handler captures outgoing packets instead of sending them. Server
 * packets that set up the player (join, abilities, position) are applied the way their client handlers
 * do it. The auto-jump option is off (bots never auto-jump; vanilla's default is on).
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
        // 1.12 Bootstrap redirects System.out / System.err into its logger (which the oracle's log
        // configuration silences); keep the real streams for the CLI's own output.
        java.io.PrintStream out = System.out, err = System.err;
        Bootstrap.initialize();
        System.setOut(out);
        System.setErr(err);
        client = Reflect.allocate(MinecraftClient.class);
        Reflect.set(MinecraftClient.class, null, "instance", client);
        options = new GameOptions();
        // The auto-jump option (vanilla default true). Bots never auto-jump.
        options.field_14902 = false;
        client.options = options;
        Reflect.set(MinecraftClient.class, client, "soundManager", SilentSoundManager.create());
        // The tutorial manager ClientPlayerEntity.tickMovement feeds the input to (no tutorial step: no-op).
        Reflect.set(MinecraftClient.class, client, "field_15869", new class_3316(client));
        // The real client only calls this when options are loaded; it fills the code -> binding map.
        KeyBinding.updateKeysByCode();
    }

    /** 1.12 blocks have no mutable bounds; kept so the dump code can call it like the 1.8.9 harness. */
    static void restoreBlockBounds() {
    }

    /** A live run: world + player built from a scenario. */
    static final class Run {
        final Scenario sc;
        final ClientWorld world;
        final ClientPlayerEntity player;
        final CapturingNetworkHandler net;
        final List<int[]> loadedRects = new ArrayList<>();
        int tick;
        /** Teleport ids of the scenario's position/look packets, counting from 1 like a server after login. */
        int nextTeleportId = 1;

        Run(Scenario sc) {
            init();
            this.sc = sc;
            releaseKeys();
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
            Reflect.set(ClientPlayNetworkHandler.class, net, "world", world);
            client.world = world;
            player = client.interactionManager.method_9658(world, new StatHandler(), new class_3355());
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
                boots.addEnchantment(Enchantments.DEPTH_STRIDER, sc.depthStrider);
                player.inventory.field_15083.set(0, boots);
            }
            sc.effects.forEach((id, amp) -> player.addStatusEffect(new StatusEffectInstance(StatusEffect.byIndex(id), 1_000_000, amp)));
            player.getHungerManager().setFoodLevel(sc.foodLevel);
            if (sc.usesItem()) {
                // Held in the selected hotbar slot (0). A shield: using it starts at once, lasts 72000
                // ticks, needs no ammunition or hunger (1.9+ swords no longer block).
                player.inventory.field_15082.set(player.inventory.selectedSlot, new ItemStack(Items.SHIELD));
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
         * Writes raw {@code id << 4 | meta} values into the chunk sections the way chunk data packets
         * decode them: the global palette maps the raw value through {@code Block.BLOCK_STATES}, and a
         * value no registered state has decodes to air.
         */
        private void placeBlocks() {
            for (Scenario.Block b : sc.blocks().values()) {
                writeRaw(b.x(), b.y(), b.z(), b.id() << 4 | (b.meta() & 15));
            }
        }

        /** The state a chunk data packet decodes {@code raw} to. */
        static BlockState decode(int raw) {
            BlockState s = Block.BLOCK_STATES.fromId(raw);
            return s == null ? Blocks.AIR.getDefaultState() : s;
        }

        /** Writes one raw block value into its (loaded) chunk. */
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
            s.setBlockState(x & 15, y & 15, z & 15, decode(raw));
            return s;
        }

        void setRaw(int x, int y, int z, int raw) {
            writeRaw(x, y, z, raw);
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
         * nothing: pressing it uses the main-hand item (interactItem: shield -> active hand), releasing
         * it while a hand is active calls stopUsingItem. Both send their packets like the real client.
         */
        private void useKey(boolean held) {
            if (held && !useHeld && !player.method_13061()) {
                ItemStack stack = player.getMainHandStack();
                if (!stack.isEmpty()) {
                    client.interactionManager.method_12234(player, world, Hand.MAIN_HAND);
                }
            } else if (!held && useHeld && player.method_13061()) {
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
                // current value, an absolute one zeroes that velocity component. Then the client sends
                // the teleport confirmation and a position_look with on_ground = false.
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
                net.sent.add(new TeleportConfirmC2SPacket(nextTeleportId++));
                net.sent.add(new PlayerMoveC2SPacket.Both(
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
                for (StatusEffectInstance inst : new ArrayList<>(player.getStatusEffectInstances())) {
                    if (!keep.contains(StatusEffect.getIndex(inst.getStatusEffect()))) {
                        player.method_13052(inst.getStatusEffect());
                    }
                }
                for (int[] f : fx.effects()) {
                    player.addStatusEffect(new StatusEffectInstance(StatusEffect.byIndex(f[0]), 1_000_000, f[1]));
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

    static GameMode gameMode(Scenario.GameMode m) {
        return switch (m) {
            case SURVIVAL -> GameMode.SURVIVAL;
            case CREATIVE -> GameMode.CREATIVE;
            case ADVENTURE -> GameMode.ADVENTURE;
            case SPECTATOR -> GameMode.SPECTATOR;
        };
    }

    static long chunkKey(int cx, int cz) {
        return ((long) cx << 32) | (cz & 0xffffffffL);
    }
}
