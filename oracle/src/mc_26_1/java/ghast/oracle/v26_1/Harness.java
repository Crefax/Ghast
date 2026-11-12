package ghast.oracle.v26_1;

import com.mojang.authlib.GameProfile;
import com.mojang.datafixers.util.Pair;
import ghast.oracle.Scenario;
import io.netty.buffer.Unpooled;
import net.minecraft.SharedConstants;
import net.minecraft.client.Camera;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;
import net.minecraft.client.ClientRecipeBook;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.gui.components.DebugScreenOverlay;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.CommonListenerCookie;
import net.minecraft.client.multiplayer.LevelLoadTracker;
import net.minecraft.client.multiplayer.MultiPlayerGameMode;
import net.minecraft.client.player.KeyboardInput;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.tutorial.Tutorial;
import net.minecraft.core.Holder;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.PacketProcessor;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.game.ClientboundLevelChunkPacketData;
import net.minecraft.network.protocol.game.ClientboundPlayerAbilitiesPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket;
import net.minecraft.network.protocol.game.ClientboundGameEventPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerPositionPacket;
import net.minecraft.network.protocol.game.ClientboundRemoveMobEffectPacket;
import net.minecraft.network.protocol.game.ClientboundSetChunkCacheCenterPacket;
import net.minecraft.network.protocol.game.ClientboundSetEntityMotionPacket;
import net.minecraft.network.protocol.game.ClientboundSetEquipmentPacket;
import net.minecraft.network.protocol.game.ClientboundSetHealthPacket;
import net.minecraft.network.protocol.game.ClientboundUpdateAttributesPacket;
import net.minecraft.network.protocol.game.ClientboundUpdateMobEffectPacket;
import net.minecraft.network.protocol.game.ServerboundClientTickEndPacket;
import net.minecraft.resources.Identifier;
import net.minecraft.server.Bootstrap;
import net.minecraft.stats.StatsCounter;
import net.minecraft.world.Difficulty;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.Pose;
import net.minecraft.world.entity.PositionMoveRotation;
import net.minecraft.world.entity.Relative;
import net.minecraft.world.entity.ai.attributes.Attribute;
import net.minecraft.world.entity.ai.attributes.AttributeInstance;
import net.minecraft.world.entity.ai.attributes.AttributeModifier;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.enchantment.EnchantmentHelper;
import net.minecraft.world.item.enchantment.Enchantments;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.LevelChunk;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.level.dimension.BuiltinDimensionTypes;
import net.minecraft.world.level.dimension.DimensionType;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.EnumSet;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.TreeSet;
import java.util.UUID;

/**
 * Runs a {@link Scenario} through the real 26.1 client code. What is real and what is faked is listed in
 * oracle/README.md ("26.1"); in short: real configuration-phase registry/tag sync ({@link Vanilla}),
 * real {@code ClientPacketListener} with real inbound packets decoded from bytes, real
 * {@code ClientLevel} + {@code ClientChunkCache} fed by chunk packet data, a {@code LocalPlayer}
 * created by the real {@code MultiPlayerGameMode}, {@code KeyboardInput} over real {@code KeyMapping}s
 * of real {@code Options}, and each tick the movement-relevant part of {@code Minecraft.tick}.
 * Faked: the {@code Minecraft} singleton (allocated, only the fields the tick reads are set), sound,
 * particles, level renderer, social manager, and the connection (captures what the client sends).
 */
final class Harness {
    static final String PLAYER_NAME = "oracle";
    static final UUID PLAYER_UUID = new UUID(0x6768617374L, 0x6f7261636c65L);
    static final int PLAYER_ENTITY_ID = 1;
    /** Server view distance of the login packet: the client chunk storage then holds (2 * (12 + 3) + 1)^2 columns. */
    static final int LOAD_DISTANCE = 12;
    static final int SIMULATION_DISTANCE = 12;
    static final int SEA_LEVEL = 63;

    static Minecraft client;
    static Options options;
    static RegistryAccess.Frozen registries;
    static Holder<DimensionType> overworld;
    static CapturingConnection connection;
    static ClientPacketListener listener;

    private Harness() {}

    static synchronized void init() {
        if (client != null) {
            return;
        }
        java.io.PrintStream out = System.out, err = System.err;
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        System.setOut(out);
        System.setErr(err);
        registries = Vanilla.configure();
        overworld = registries.lookupOrThrow(Registries.DIMENSION_TYPE).getOrThrow(BuiltinDimensionTypes.OVERWORLD);

        client = Reflect.allocate(Minecraft.class);
        Reflect.set(Minecraft.class, null, "instance", client);
        Reflect.set(Minecraft.class, client, "gameThread", Thread.currentThread());
        Reflect.set(Minecraft.class, client, "packetProcessor", new PacketProcessor(Thread.currentThread()));
        options = headlessOptions();
        Reflect.set(Minecraft.class, client, "options", options);
        Reflect.set(Minecraft.class, client, "soundManager", Stubs.SilentSoundManager.create());
        Reflect.set(Minecraft.class, client, "tutorial", new Tutorial(client, options));
        Reflect.set(Minecraft.class, client, "levelRenderer", Stubs.NoLevelRenderer.create());
        Reflect.set(Minecraft.class, client, "particleEngine", Stubs.NoParticles.create());
        Reflect.set(Minecraft.class, client, "playerSocialManager", Stubs.NoSocialManager.create());
        Reflect.set(Minecraft.class, client, "blockEntityRenderDispatcher", Stubs.NoBlockEntityRenderers.create());
        // The camera (sound / particle distance, the camera entity) lives in the game renderer.
        GameRenderer gameRenderer = Reflect.allocate(GameRenderer.class);
        Reflect.set(GameRenderer.class, gameRenderer, "mainCamera", new Camera());
        Reflect.set(Minecraft.class, client, "gameRenderer", gameRenderer);
        // The packet listener's constructor asks the debug overlay for its ping logger.
        Gui gui = Reflect.allocate(Gui.class);
        Reflect.set(Gui.class, gui, "debugOverlay", Reflect.allocate(DebugScreenOverlay.class));
        Reflect.set(Minecraft.class, client, "gui", gui);
        // Chat abilities of the new player (MultiPlayerGameMode.createPlayer) read the account's properties.
        Reflect.set(Minecraft.class, client, "userPropertiesFuture",
                java.util.concurrent.CompletableFuture.completedFuture(com.mojang.authlib.minecraft.UserApiService.OFFLINE_PROPERTIES));
        Reflect.set(Minecraft.class, client, "profileFuture", java.util.concurrent.CompletableFuture.completedFuture(null));
        // Offline services (no profile keys): the player info handler asks whether chat can be signed.
        Reflect.set(Minecraft.class, client, "services", new net.minecraft.server.Services(null,
                com.mojang.authlib.yggdrasil.ServicesKeySet.EMPTY, null, null, null));
        connection = new CapturingConnection();
    }

    /**
     * Real {@code Options} (vanilla defaults: auto-jump off, sprint window 7, hold-to-sneak / sprint),
     * created on an empty directory so no options file is read.
     */
    private static Options headlessOptions() {
        try {
            Path dir = Files.createTempDirectory("ghast-oracle-options");
            Options o = new Options(client, dir.toFile());
            Files.deleteIfExists(dir.resolve("options.txt"));
            Files.deleteIfExists(dir);
            return o;
        } catch (java.io.IOException e) {
            throw new IllegalStateException(e);
        }
    }

    /** A fresh packet listener per run, as after a fresh login. */
    private static ClientPacketListener newListener() {
        CommonListenerCookie cookie = new CommonListenerCookie(new LevelLoadTracker(), new GameProfile(PLAYER_UUID, PLAYER_NAME),
                null, registries, FeatureFlags.DEFAULT_FLAGS, "vanilla", null, null, Map.of(), null, Map.of(), null, Map.of(), false);
        return new ClientPacketListener(client, connection, cookie);
    }

    /** A live run: level + player built from a scenario. */
    static final class Run {
        final Scenario sc;
        final ClientLevel level;
        final LocalPlayer player;
        final List<int[]> loadedRects = new ArrayList<>();
        final TreeMap<Integer, String> statesUsed = new TreeMap<>();
        int tick;
        int teleportId;
        int rightClickDelay;

        Run(Scenario sc) {
            init();
            this.sc = sc;
            releaseKeys();
            options.autoJump().set(sc.autoJump);
            options.sprintWindow().set(sc.sprintWindow != null ? sc.sprintWindow : 7);
            client.player = null;
            client.level = null;
            client.gameRenderer.getMainCamera().setEntity(null);
            connection.sent.clear();
            listener = newListener();

            // --- login (the movement-relevant part of ClientPacketListener.handleLogin) ---
            client.gameMode = new MultiPlayerGameMode(client, listener);
            ClientLevel.ClientLevelData levelData = new ClientLevel.ClientLevelData(Difficulty.NORMAL, false, false);
            Reflect.set(ClientPacketListener.class, listener, "levelData", levelData);
            Reflect.setInt(ClientPacketListener.class, listener, "serverChunkRadius", LOAD_DISTANCE);
            Reflect.setInt(ClientPacketListener.class, listener, "serverSimulationDistance", SIMULATION_DISTANCE);
            level = new ClientLevel(listener, levelData, Ext.dimensionKey(sc), Ext.dimensionType(sc), LOAD_DISTANCE, SIMULATION_DISTANCE,
                    client.levelRenderer, false, 0L, SEA_LEVEL);
            Reflect.set(ClientPacketListener.class, listener, "level", level);
            // Fixed seeds instead of time-seeded randoms (only particles and sounds draw from them).
            Reflect.set(Level.class, level, "random", net.minecraft.util.RandomSource.create(0L));
            client.level = level;
            player = client.gameMode.createPlayer(level, new StatsCounter(), new ClientRecipeBook());
            client.player = player;
            Reflect.set(net.minecraft.world.entity.Entity.class, player, "random", net.minecraft.util.RandomSource.create(0L));
            player.setYRot(-180.0F);
            player.resetPos();
            player.setId(PLAYER_ENTITY_ID);
            level.addEntity(player);
            player.input = new KeyboardInput(options);
            client.gameMode.adjustPlayer(player);
            client.gameRenderer.getMainCamera().setEntity(player);
            GameType gm = gameMode(sc.gameMode);
            client.gameMode.setLocalMode(gm, null);

            // Every state a fill uses (also ones later fills overwrite), so readers can map ids by name.
            for (Scenario.Fill f : sc.fills) {
                statesUsed.put(f.id(), States.name(States.byId(f.id())));
            }

            // --- tab list entry (isSpectator / isCreative of the client player read it) ---
            playerInfo(gm);

            // --- chunk cache centre + chunk data ---
            int ccx = floorDiv16(sc.x), ccz = floorDiv16(sc.z);
            listener.handleSetChunkCacheCenter(roundTrip(ClientboundSetChunkCacheCenterPacket.STREAM_CODEC,
                    new ClientboundSetChunkCacheCenterPacket(ccx, ccz)));
            loadChunks(ccx, ccz);

            // --- abilities (the server derives them from the game mode, then the scenario's overrides) ---
            Abilities ab = new Abilities();
            gm.updatePlayerAbilities(ab);
            ab.flying = sc.flying;
            if (sc.allowFlying != null) {
                ab.mayfly = sc.allowFlying;
            }
            ab.setFlyingSpeed(sc.flySpeed);
            ab.setWalkingSpeed(sc.walkSpeed);
            listener.handlePlayerAbilities(roundTrip(ClientboundPlayerAbilitiesPacket.STREAM_CODEC, new ClientboundPlayerAbilitiesPacket(ab)));

            // --- attributes (movement speed base + modifiers, plus 26.1 attribute overrides) ---
            List<AttributeInstance> attrs = new ArrayList<>();
            AttributeInstance speed = new AttributeInstance(Attributes.MOVEMENT_SPEED, i -> {});
            speed.setBaseValue(sc.movementSpeedBase);
            for (int i = 0; i < sc.speedModifiers.size(); i++) {
                double[] m = sc.speedModifiers.get(i);
                speed.addPermanentModifier(new AttributeModifier(Identifier.fromNamespaceAndPath("oracle", "speed_" + i), m[0],
                        AttributeModifier.Operation.BY_ID.apply((int) m[1])));
            }
            attrs.add(speed);
            Map<Holder<Attribute>, AttributeInstance> extra = new LinkedHashMap<>();
            for (Map.Entry<String, Double> e : sc.attributeBases.entrySet()) {
                Holder<Attribute> a = attribute(e.getKey());
                extra.computeIfAbsent(a, h -> copyBase(h)).setBaseValue(e.getValue());
            }
            for (Scenario.AttributeModifierSpec m : sc.attributeModifiers) {
                Holder<Attribute> a = attribute(m.attribute());
                extra.computeIfAbsent(a, h -> copyBase(h)).addPermanentModifier(new AttributeModifier(Identifier.parse(m.id()),
                        m.amount(), AttributeModifier.Operation.BY_ID.apply(m.operation())));
            }
            attrs.addAll(extra.values());
            listener.handleUpdateAttributes(roundTrip(ClientboundUpdateAttributesPacket.STREAM_CODEC,
                    new ClientboundUpdateAttributesPacket(PLAYER_ENTITY_ID, attrs)));

            // --- health / food, effects, equipment ---
            listener.handleSetHealth(roundTrip(ClientboundSetHealthPacket.STREAM_CODEC, new ClientboundSetHealthPacket(sc.health != null ? sc.health : 20.0F, sc.foodLevel, 5.0F)));
            for (Map.Entry<Integer, Integer> e : sc.effects.entrySet()) {
                addEffect(e.getKey(), e.getValue());
            }
            if (sc.soulSpeed > 0 || sc.depthStrider > 0 || sc.leatherBoots) {
                ItemStack boots = new ItemStack(sc.leatherBoots ? Items.LEATHER_BOOTS : Items.DIAMOND_BOOTS);
                var enchantments = registries.lookupOrThrow(Registries.ENCHANTMENT);
                if (sc.soulSpeed > 0) {
                    boots.enchant(enchantments.getOrThrow(Enchantments.SOUL_SPEED), sc.soulSpeed);
                }
                if (sc.depthStrider > 0) {
                    boots.enchant(enchantments.getOrThrow(Enchantments.DEPTH_STRIDER), sc.depthStrider);
                }
                listener.handleSetEquipment(roundTrip(ClientboundSetEquipmentPacket.STREAM_CODEC,
                        new ClientboundSetEquipmentPacket(PLAYER_ENTITY_ID, List.of(Pair.of(EquipmentSlot.FEET, boots)))));
                // The boots' enchantment attribute effects (depth strider: water movement efficiency) are
                // applied by the server, which then sends the player's attributes.
                sendEquipmentAttributes(boots);
            }
            if (sc.usesItem()) {
                // A shield in the selected hotbar slot: using it blocks (input x0.2, no sprinting) until released.
                ItemStack ext = Ext.handItem(sc);
                ItemStack shield = ext != null ? ext : new ItemStack(Items.SHIELD);
                if (sc.useSpeed != null || sc.useCanSprint != null) {
                    // An item with its own use effects (a server-defined component): use slowdown and
                    // whether sprinting continues come from it.
                    shield.set(net.minecraft.core.component.DataComponents.USE_EFFECTS, new net.minecraft.world.item.component.UseEffects(
                            sc.useCanSprint != null && sc.useCanSprint, true, sc.useSpeed != null ? sc.useSpeed : 0.2F));
                }
                player.getInventory().setItem(player.getInventory().getSelectedSlot(), shield);
            }

            Ext.applySetup(this);

            // --- position packet (absolute) ---
            teleport(sc.x, sc.y, sc.z, sc.yaw, sc.pitch, 0);
            if (sc.pose != null) {
                player.setPose(Pose.valueOf(sc.pose.toUpperCase()));
            }
            if (sc.bb != null) {
                player.setBoundingBox(new AABB(sc.bb[0], sc.bb[1], sc.bb[2], sc.bb[3], sc.bb[4], sc.bb[5]));
            }

            // --- scenario overrides of the dynamic state ---
            player.setDeltaMovement(sc.vx, sc.vy, sc.vz);
            player.setOnGround(sc.onGround);
            player.fallDistance = sc.fallDistance;
            if (sc.sprinting) {
                player.setSprinting(true);
            }
            Ext.finishSetup(this);
            if (sc.delayedLoad) {
                // The join as the client lives it: handleLogin's startWaitingForNewLevel started the
                // level-load tracker (multiplayer: no close delay); the player does not tick until it
                // reports the level ready. What the client sent so far stays in the first record.
                Stubs.NoLevelRenderer.compiled = false;
                LevelLoadTracker tracker = (LevelLoadTracker) Reflect.get(ClientPacketListener.class, listener, "levelLoadTracker");
                tracker.startClientLoad(player, level, client.levelRenderer);
                return;
            }
            // The level counts as loaded (LevelLoadTracker ready): the client tells the server and the
            // player starts ticking.
            invoke(ClientPacketListener.class, listener, "notifyPlayerLoaded");
            connection.sent.clear();
        }

        private void playerInfo(GameType gm) {
            RegistryFriendlyByteBuf b = buf();
            b.writeEnumSet(EnumSet.of(ClientboundPlayerInfoUpdatePacket.Action.ADD_PLAYER,
                    ClientboundPlayerInfoUpdatePacket.Action.UPDATE_GAME_MODE), ClientboundPlayerInfoUpdatePacket.Action.class);
            b.writeVarInt(1);
            b.writeUUID(PLAYER_UUID);
            b.writeUtf(PLAYER_NAME, 16);
            b.writeVarInt(0); // no profile properties
            b.writeVarInt(gm.getId());
            ClientboundPlayerInfoUpdatePacket p = ClientboundPlayerInfoUpdatePacket.STREAM_CODEC.decode(b);
            if (b.readableBytes() != 0) {
                throw new IllegalStateException("unread player info bytes");
            }
            listener.handlePlayerInfoUpdate(p);
        }

        private AttributeInstance copyBase(Holder<Attribute> a) {
            AttributeInstance cur = player.getAttribute(a);
            AttributeInstance inst = new AttributeInstance(a, i -> {});
            inst.setBaseValue(cur != null ? cur.getBaseValue() : a.value().getDefaultValue());
            return inst;
        }

        private void sendEquipmentAttributes(ItemStack boots) {
            Map<Holder<Attribute>, AttributeInstance> byAttr = new LinkedHashMap<>();
            EnchantmentHelper.forEachModifier(boots, EquipmentSlot.FEET, (attr, mod) ->
                    byAttr.computeIfAbsent(attr, this::copyBase).addTransientModifier(mod));
            if (!byAttr.isEmpty()) {
                listener.handleUpdateAttributes(roundTrip(ClientboundUpdateAttributesPacket.STREAM_CODEC,
                        new ClientboundUpdateAttributesPacket(PLAYER_ENTITY_ID, new ArrayList<>(byAttr.values()))));
            }
        }

        private void addEffect(int id, int amplifier) {
            Holder<MobEffect> effect = BuiltInRegistries.MOB_EFFECT.get(id).orElseThrow(() -> new IllegalArgumentException("effect " + id));
            listener.handleUpdateMobEffect(roundTrip(ClientboundUpdateMobEffectPacket.STREAM_CODEC,
                    new ClientboundUpdateMobEffectPacket(PLAYER_ENTITY_ID, new MobEffectInstance(effect, 1_000_000, amplifier), false)));
        }

        private void teleport(double x, double y, double z, float yaw, float pitch, int flags) {
            teleport(x, y, z, Vec3.ZERO, yaw, pitch, flags);
        }

        private void teleport(double x, double y, double z, Vec3 delta, float yaw, float pitch, int flags) {
            Set<Relative> rel = Relative.unpack(flags);
            ClientboundPlayerPositionPacket p = new ClientboundPlayerPositionPacket(teleportId++,
                    new PositionMoveRotation(new Vec3(x, y, z), delta, yaw, pitch), rel);
            listener.handleMovePlayer(roundTrip(ClientboundPlayerPositionPacket.STREAM_CODEC, p));
        }

        private void loadChunks(int ccx, int ccz) {
            TreeSet<Long> chunks = new TreeSet<>();
            int radius = LOAD_DISTANCE + 3;
            if (sc.outside == Scenario.Outside.AIR) {
                int r = Math.min(radius, 2 + sc.ticks.size() / 12);
                for (int cx = ccx - r; cx <= ccx + r; cx++) {
                    for (int cz = ccz - r; cz <= ccz + r; cz++) {
                        chunks.add(chunkKey(cx, cz));
                    }
                }
                loadedRects.add(new int[] {ccx - r, ccz - r, ccx + r, ccz + r});
            } else {
                for (Scenario.Block b : sc.blocks().values()) {
                    chunks.add(chunkKey(b.x() >> 4, b.z() >> 4));
                }
                for (int[] c : sc.extraChunks) {
                    chunks.add(chunkKey(c[0], c[1]));
                }
                for (long k : chunks) {
                    loadedRects.add(new int[] {(int) (k >> 32), (int) k, (int) (k >> 32), (int) k});
                }
            }
            Map<Long, List<Scenario.Block>> byChunk = new TreeMap<>();
            for (Scenario.Block b : sc.blocks().values()) {
                if (b.y() < level.getMinY() || b.y() > level.getMaxY()) {
                    throw new IllegalArgumentException("block outside world height: " + b);
                }
                long k = chunkKey(b.x() >> 4, b.z() >> 4);
                if (!chunks.contains(k)) {
                    throw new IllegalArgumentException("block outside the loaded chunk area: " + b);
                }
                byChunk.computeIfAbsent(k, kk -> new ArrayList<>()).add(b);
            }
            for (long k : chunks) {
                int cx = (int) (k >> 32), cz = (int) k;
                if (Math.abs(cx - ccx) > radius || Math.abs(cz - ccz) > radius) {
                    throw new IllegalArgumentException("chunk " + cx + "," + cz + " outside the client chunk storage");
                }
                loadChunk(cx, cz, byChunk.getOrDefault(k, List.of()));
            }
        }

        /**
         * Builds the column like a server holds it (sections + primed heightmaps), serialises it as
         * chunk packet data and lets the client chunk cache decode it, as
         * {@code ClientPacketListener.handleLevelChunkWithLight} does (minus light and render scheduling).
         */
        void loadChunk(int cx, int cz, List<Scenario.Block> blocks) {
            LevelChunk source = new LevelChunk(level, new ChunkPos(cx, cz));
            for (Scenario.Block b : blocks) {
                BlockState state = States.byId(b.id());
                statesUsed.put(b.id(), States.name(state));
                LevelChunkSection section = source.getSection(source.getSectionIndex(b.y()));
                section.setBlockState(b.x() & 15, b.y() & 15, b.z() & 15, state, false);
            }
            Heightmap.primeHeightmaps(source, EnumSet.allOf(Heightmap.Types.class));
            ClientboundLevelChunkPacketData data = new ClientboundLevelChunkPacketData(source);
            RegistryFriendlyByteBuf wire = buf();
            data.write(wire);
            ClientboundLevelChunkPacketData decoded = new ClientboundLevelChunkPacketData(wire, cx, cz);
            if (wire.readableBytes() != 0) {
                throw new IllegalStateException("unread chunk bytes");
            }
            LevelChunk chunk = level.getChunkSource().replaceWithPacketData(cx, cz, decoded.getReadBuffer(), decoded.getHeightmaps(),
                    decoded.getBlockEntitiesTagsConsumer(cx, cz));
            if (chunk == null) {
                throw new IllegalStateException("chunk not accepted: " + cx + "," + cz);
            }
        }

        /**
         * One client tick, in {@code Minecraft.tick} order: tick rate manager, game mode (held item sync),
         * key bindings (use key), the level's entity tick (the player's tick, input, movement and
         * movement packets), then the tick-end packet. Inbound packets (events) come first, as the
         * packet processor runs them before the tick; the yaw/pitch is set before the tick, as mouse
         * turning happens between ticks.
         */
        void step(Scenario.Tick t) {
            for (Scenario.Event e : t.events()) {
                applyEvent(e);
            }
            setKeys(t.keys());
            // While the level-loading screen is open the mouse is not grabbed: no turning until the
            // tick after the gate (the screen closes during the gate tick).
            if (!sc.delayedLoad || listener.hasClientLoaded()) {
                player.setYRot(t.yaw());
                player.setXRot(t.pitch());
            }
            level.tickRateManager().tick();
            if (rightClickDelay > 0) {
                rightClickDelay--;
            }
            client.gameMode.tick();
            // MultiPlayerGameMode.tick -> Connection.tick -> ClientPacketListener.tick: the level-load
            // part of the listener tick.
            tickLevelLoad();
            useKey();
            Ext.beforeEntities(this);
            level.tickEntities();
            Ext.afterEntities(this);
            listener.send(ServerboundClientTickEndPacket.INSTANCE);
            tick++;
        }

        private void tickLevelLoad() {
            LevelLoadTracker tracker = (LevelLoadTracker) Reflect.get(ClientPacketListener.class, listener, "levelLoadTracker");
            if (tracker != null) {
                tracker.tickClientLoad();
                if (tracker.isLevelReady()) {
                    invoke(ClientPacketListener.class, listener, "notifyPlayerLoaded");
                    Reflect.set(ClientPacketListener.class, listener, "levelLoadTracker", null);
                }
            }
        }

        /** The 30 s timeout of the level-load tracker runs out: its client state's deadline becomes 0. */
        private void loadTimeout() {
            LevelLoadTracker tracker = (LevelLoadTracker) Reflect.get(ClientPacketListener.class, listener, "levelLoadTracker");
            if (tracker == null) {
                return;
            }
            Object state = Reflect.get(LevelLoadTracker.class, tracker, "clientState");
            if (state == null || !state.getClass().isRecord() || state.getClass().getRecordComponents().length != 4) {
                return; // ready already
            }
            try {
                java.lang.reflect.RecordComponent[] rc = state.getClass().getRecordComponents();
                Object[] args = new Object[4];
                Class<?>[] types = new Class<?>[4];
                for (int i = 0; i < 4; i++) {
                    types[i] = rc[i].getType();
                    rc[i].getAccessor().setAccessible(true);
                    args[i] = rc[i].getAccessor().invoke(state);
                }
                args[3] = 0L;
                java.lang.reflect.Constructor<?> c = state.getClass().getDeclaredConstructor(types);
                c.setAccessible(true);
                Reflect.set(LevelLoadTracker.class, tracker, "clientState", c.newInstance(args));
            } catch (ReflectiveOperationException ex) {
                throw new IllegalStateException(ex);
            }
        }

        /**
         * The use-key part of {@code Minecraft.handleKeybinds} with the crosshair on nothing: while using,
         * a released key stops the use and queued clicks are dropped; otherwise every queued click (a
         * press, see {@link #setKeys}) starts a use, and a held key retries once the right-click delay
         * has run out.
         */
        private void useKey() {
            if (player.isUsingItem()) {
                if (!options.keyUse.isDown()) {
                    client.gameMode.releaseUsingItem(player);
                }
                while (options.keyUse.consumeClick()) {
                    // dropped, as in vanilla
                }
            } else {
                while (options.keyUse.consumeClick()) {
                    startUseItem();
                }
            }
            if (options.keyUse.isDown() && rightClickDelay == 0 && !player.isUsingItem()) {
                startUseItem();
            }
        }

        /** {@code Minecraft.startUseItem} with a miss hit result. */
        private void startUseItem() {
            if (client.gameMode.isDestroying()) {
                return;
            }
            rightClickDelay = 4;
            if (player.isHandsBusy()) {
                return;
            }
            for (InteractionHand hand : InteractionHand.values()) {
                ItemStack held = player.getItemInHand(hand);
                if (!held.isItemEnabled(level.enabledFeatures())) {
                    return;
                }
                if (!held.isEmpty() && client.gameMode.useItem(player, hand) instanceof InteractionResult.Success success) {
                    if (success.swingSource() == InteractionResult.SwingSource.CLIENT) {
                        player.swing(hand);
                    }
                    return;
                }
            }
        }

        private void applyEvent(Scenario.Event e) {
            if (e instanceof Scenario.LoadStart) {
                listener.handleGameEvent(roundTrip(ClientboundGameEventPacket.STREAM_CODEC,
                        new ClientboundGameEventPacket(ClientboundGameEventPacket.LEVEL_CHUNKS_LOAD_START, 0.0F)));
            } else if (e instanceof Scenario.SectionCompiled sc) {
                Stubs.NoLevelRenderer.compiled = sc.compiled();
            } else if (e instanceof Scenario.LoadTimeout) {
                loadTimeout();
            } else if (e instanceof Scenario.Velocity v) {
                // The scenario format keeps the classic 1/8000 units; 26.1 sends the vector as an LpVec3.
                listener.handleSetEntityMotion(roundTrip(ClientboundSetEntityMotionPacket.STREAM_CODEC,
                        new ClientboundSetEntityMotionPacket(PLAYER_ENTITY_ID, new Vec3(v.x() / 8000.0, v.y() / 8000.0, v.z() / 8000.0))));
            } else if (e instanceof Scenario.Teleport tp) {
                teleport(tp.x(), tp.y(), tp.z(), new Vec3(tp.dx(), tp.dy(), tp.dz()), tp.yaw(), tp.pitch(), tp.flags());
            } else if (e instanceof Scenario.SpeedAttribute sa) {
                AttributeInstance speed = new AttributeInstance(Attributes.MOVEMENT_SPEED, i -> {});
                speed.setBaseValue(sa.base());
                for (Scenario.Modifier m : sa.modifiers()) {
                    speed.addPermanentModifier(new AttributeModifier(modifierId(m.uuid()), m.amount(),
                            AttributeModifier.Operation.BY_ID.apply(m.operation())));
                }
                listener.handleUpdateAttributes(roundTrip(ClientboundUpdateAttributesPacket.STREAM_CODEC,
                        new ClientboundUpdateAttributesPacket(PLAYER_ENTITY_ID, List.of(speed))));
            } else if (e instanceof Scenario.Effects fx) {
                Set<Integer> keep = new java.util.HashSet<>();
                for (int[] f : fx.effects()) {
                    keep.add(f[0]);
                }
                for (MobEffectInstance inst : new ArrayList<>(player.getActiveEffects())) {
                    int id = BuiltInRegistries.MOB_EFFECT.getId(inst.getEffect().value());
                    if (!keep.contains(id)) {
                        listener.handleRemoveMobEffect(roundTrip(ClientboundRemoveMobEffectPacket.STREAM_CODEC,
                                new ClientboundRemoveMobEffectPacket(PLAYER_ENTITY_ID, inst.getEffect())));
                    }
                }
                for (int[] f : fx.effects()) {
                    addEffect(f[0], f[1]);
                }
            } else if (e instanceof Scenario.Attribute at) {
                AttributeInstance inst = new AttributeInstance(attribute(at.attribute()), i -> {});
                inst.setBaseValue(at.base());
                for (Scenario.AttributeModifierSpec m : at.modifiers()) {
                    inst.addPermanentModifier(new AttributeModifier(Identifier.parse(m.id()), m.amount(),
                            AttributeModifier.Operation.BY_ID.apply(m.operation())));
                }
                listener.handleUpdateAttributes(roundTrip(ClientboundUpdateAttributesPacket.STREAM_CODEC,
                        new ClientboundUpdateAttributesPacket(PLAYER_ENTITY_ID, List.of(inst))));
            } else if (e instanceof Scenario.Abilities a) {
                Abilities ab = new Abilities();
                ab.invulnerable = a.invulnerable();
                ab.flying = a.flying();
                ab.mayfly = a.allowFlying();
                ab.instabuild = a.creative();
                ab.setFlyingSpeed(a.flySpeed());
                ab.setWalkingSpeed(a.walkSpeed());
                listener.handlePlayerAbilities(roundTrip(ClientboundPlayerAbilitiesPacket.STREAM_CODEC, new ClientboundPlayerAbilitiesPacket(ab)));
            } else if (e instanceof Scenario.VersionEvent ve) {
                Ext.apply(this, ve);
            }
        }
    }

    /**
     * Scenario modifier ids are UUIDs (the 1.16 format); 26.1 identifies modifiers by name. The client's
     * own sprint modifier keeps its vanilla id, every other UUID becomes {@code oracle:<uuid>}.
     */
    static Identifier modifierId(UUID uuid) {
        if (uuid.equals(UUID.fromString("662a6b8d-da3e-4c1c-8813-96ea6097278d"))) {
            return Identifier.withDefaultNamespace("sprinting");
        }
        return Identifier.fromNamespaceAndPath("oracle", uuid.toString());
    }

    static Holder<Attribute> attribute(String name) {
        return BuiltInRegistries.ATTRIBUTE.get(Identifier.parse(name))
                .orElseThrow(() -> new IllegalArgumentException("unknown attribute " + name));
    }

    static RegistryFriendlyByteBuf buf() {
        return new RegistryFriendlyByteBuf(Unpooled.buffer(), registries);
    }

    /** Encodes a packet with its stream codec into registry-aware bytes and decodes it again. */
    static <B extends io.netty.buffer.ByteBuf, P> P roundTrip(StreamCodec<? super RegistryFriendlyByteBuf, P> codec, P packet) {
        RegistryFriendlyByteBuf b = buf();
        codec.encode(b, packet);
        P back = codec.decode(b);
        if (b.readableBytes() != 0) {
            throw new IllegalStateException("unread bytes decoding " + packet.getClass().getSimpleName());
        }
        return back;
    }

    static void invoke(Class<?> owner, Object target, String name, Object... args) {
        try {
            for (Method m : owner.getDeclaredMethods()) {
                if (m.getName().equals(name) && m.getParameterCount() == args.length) {
                    m.setAccessible(true);
                    m.invoke(target, args);
                    return;
                }
            }
            throw new IllegalStateException("no method " + name);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
    }

    /**
     * Key states as the keyboard / mouse handler sets them: a press also queues a click
     * ({@code KeyMapping.click}), which {@code consumeClick} hands out (only the use key's clicks are read
     * here; movement keys are read as held states).
     */
    static void setKeys(int keys) {
        KeyMapping[] order = {options.keyUp, options.keyDown, options.keyLeft, options.keyRight, options.keyJump,
                options.keyShift, options.keySprint, options.keyUse};
        for (int i = 0; i < order.length; i++) {
            boolean down = (keys & (1 << i)) != 0;
            if (down && !order[i].isDown()) {
                Reflect.setInt(KeyMapping.class, order[i], "clickCount", Reflect.getInt(KeyMapping.class, order[i], "clickCount") + 1);
            }
            order[i].setDown(down);
        }
    }

    /** Releases every key and drops queued clicks (between runs). */
    static void releaseKeys() {
        setKeys(0);
        for (KeyMapping k : new KeyMapping[] {options.keyUp, options.keyDown, options.keyLeft, options.keyRight, options.keyJump,
                options.keyShift, options.keySprint, options.keyUse}) {
            Reflect.setInt(KeyMapping.class, k, "clickCount", 0);
        }
    }

    static GameType gameMode(Scenario.GameMode m) {
        return switch (m) {
            case SURVIVAL -> GameType.SURVIVAL;
            case CREATIVE -> GameType.CREATIVE;
            case ADVENTURE -> GameType.ADVENTURE;
            case SPECTATOR -> GameType.SPECTATOR;
        };
    }

    static int floorDiv16(double v) {
        return Math.floorDiv((int) Math.floor(v), 16);
    }

    static long chunkKey(int cx, int cz) {
        return ((long) cx << 32) | (cz & 0xffffffffL);
    }
}
