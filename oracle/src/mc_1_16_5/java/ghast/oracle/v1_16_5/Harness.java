package ghast.oracle.v1_16_5;

import com.mojang.authlib.GameProfile;
import com.mojang.datafixers.util.Pair;
import ghast.oracle.Scenario;
import io.netty.buffer.Unpooled;
import net.minecraft.Bootstrap;
import net.minecraft.block.Block;
import net.minecraft.block.BlockState;
import net.minecraft.client.MinecraftClient;
import net.minecraft.client.input.KeyboardInput;
import net.minecraft.client.network.ClientPlayNetworkHandler;
import net.minecraft.client.network.ClientPlayerEntity;
import net.minecraft.client.network.ClientPlayerInteractionManager;
import net.minecraft.client.network.PlayerListEntry;
import net.minecraft.client.option.GameOptions;
import net.minecraft.client.option.KeyBinding;
import net.minecraft.client.option.StickyKeyBinding;
import net.minecraft.client.recipebook.ClientRecipeBook;
import net.minecraft.client.tutorial.TutorialManager;
import net.minecraft.client.world.ClientWorld;
import net.minecraft.enchantment.EnchantmentHelper;
import net.minecraft.enchantment.Enchantments;
import net.minecraft.entity.EntityPose;
import net.minecraft.entity.EquipmentSlot;
import net.minecraft.util.math.Box;
import net.minecraft.entity.attribute.EntityAttributes;
import net.minecraft.entity.effect.StatusEffect;
import net.minecraft.entity.effect.StatusEffectInstance;
import net.minecraft.fluid.Fluid;
import net.minecraft.item.Item;
import net.minecraft.item.ItemStack;
import net.minecraft.item.Items;
import net.minecraft.network.PacketByteBuf;
import net.minecraft.network.packet.s2c.play.ChunkDataS2CPacket;
import net.minecraft.network.packet.s2c.play.EntityAttributesS2CPacket;
import net.minecraft.network.packet.s2c.play.EntityEquipmentUpdateS2CPacket;
import net.minecraft.network.packet.s2c.play.EntityStatusEffectS2CPacket;
import net.minecraft.network.packet.s2c.play.EntityVelocityUpdateS2CPacket;
import net.minecraft.network.packet.s2c.play.HealthUpdateS2CPacket;
import net.minecraft.network.packet.s2c.play.PlayerAbilitiesS2CPacket;
import net.minecraft.network.packet.s2c.play.PlayerListS2CPacket;
import net.minecraft.network.packet.s2c.play.PlayerPositionLookS2CPacket;
import net.minecraft.network.packet.s2c.play.RemoveEntityStatusEffectS2CPacket;
import net.minecraft.resource.DefaultResourcePack;
import net.minecraft.resource.ReloadableResourceManagerImpl;
import net.minecraft.resource.ResourceType;
import net.minecraft.stat.StatHandler;
import net.minecraft.tag.TagGroupLoader;
import net.minecraft.tag.TagManager;
import net.minecraft.util.Hand;
import net.minecraft.util.Identifier;
import net.minecraft.util.math.ChunkPos;
import net.minecraft.util.profiler.DummyProfiler;
import net.minecraft.util.registry.DynamicRegistryManager;
import net.minecraft.util.registry.Registry;
import net.minecraft.world.Difficulty;
import net.minecraft.world.GameMode;
import net.minecraft.world.World;
import net.minecraft.world.biome.Biome;
import net.minecraft.world.biome.BiomeKeys;
import net.minecraft.world.biome.source.BiomeArray;
import net.minecraft.world.chunk.ChunkSection;
import net.minecraft.world.chunk.WorldChunk;
import net.minecraft.world.dimension.DimensionType;

import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.TreeMap;
import java.util.UUID;

/**
 * Runs a {@link Scenario} through the real 1.16.5 client code.
 *
 * <p>Real: {@code ClientWorld} + {@code ClientChunkManager} (chunks arrive through
 * {@code ChunkDataS2CPacket} serialisation and {@code loadChunkFromPacket}), the real
 * {@code ClientPlayNetworkHandler} (inbound setup packets and events are decoded from bytes and handed
 * to its real handlers), {@code ClientPlayerEntity} created by the real interaction manager,
 * {@code KeyboardInput} over real {@code KeyBinding}s, vanilla tags loaded from the jar's data pack,
 * and each tick is {@code ClientWorld.tickEntities()}.
 *
 * <p>Faked: the {@code MinecraftClient} singleton (allocated without its constructor; options, sound
 * manager, tutorial manager, thread, world, player, interaction manager and camera are set), a silent
 * sound manager, a no-op world renderer, and the connection (captures everything the client sends).
 */
final class Harness {
    static final String PLAYER_NAME = "oracle";
    static final UUID PLAYER_UUID = new UUID(0x6768617374L, 0x6f7261636c65L);
    static final int PLAYER_ENTITY_ID = 1;
    /** View distance of the join packet: the client chunk map then holds (2 * (12 + 3) + 1)^2 columns. */
    static final int LOAD_DISTANCE = 12;

    static MinecraftClient client;
    static GameOptions options;
    static DynamicRegistryManager.Impl registries;
    static DimensionType overworld;
    static CapturingConnection connection;
    static ClientPlayNetworkHandler net;
    static Biome plains;

    private Harness() {}

    static synchronized void init() {
        if (client != null) {
            return;
        }
        // Bootstrap redirects System.out/err into the game logger; keep the real streams.
        java.io.PrintStream out = System.out, err = System.err;
        Bootstrap.initialize();
        System.setOut(out);
        System.setErr(err);
        client = Reflect.allocate(MinecraftClient.class);
        Reflect.set(MinecraftClient.class, null, "instance", client);
        Reflect.set(MinecraftClient.class, client, "thread", Thread.currentThread());
        options = headlessOptions();
        Reflect.set(MinecraftClient.class, client, "options", options);
        Reflect.set(MinecraftClient.class, client, "soundManager", SilentSoundManager.create());
        Reflect.set(MinecraftClient.class, client, "tutorialManager", new TutorialManager(client));
        Reflect.set(MinecraftClient.class, client, "worldRenderer", StubWorldRenderer.create());
        // Sounds measure their distance to the camera; a game renderer with just its camera object.
        net.minecraft.client.render.GameRenderer gameRenderer = Reflect.allocate(net.minecraft.client.render.GameRenderer.class);
        Reflect.set(net.minecraft.client.render.GameRenderer.class, gameRenderer, "camera", new net.minecraft.client.render.Camera());
        Reflect.set(MinecraftClient.class, client, "gameRenderer", gameRenderer);
        registries = DynamicRegistryManager.create();
        overworld = registries.getDimensionTypes().get(DimensionType.OVERWORLD_REGISTRY_KEY);
        plains = registries.get(Registry.BIOME_KEY).get(BiomeKeys.PLAINS);
        loadVanillaTags().apply();

        connection = new CapturingConnection();
        net = new ClientPlayNetworkHandler(client, null, connection, new GameProfile(PLAYER_UUID, PLAYER_NAME));
        Reflect.set(ClientPlayNetworkHandler.class, net, "registryManager", registries);
        // The first position packet would open/close the terrain screen; there is no screen here.
        Reflect.set(ClientPlayNetworkHandler.class, net, "positionLookSetup", true);
    }

    /**
     * GameOptions without its constructor: the real one creates key bindings through InputUtil, whose
     * static initialiser loads the GLFW native library. The movement code only reads the movement key
     * bindings (pressed state), {@code autoJump} and the sprint/sneak toggle options, so those are the
     * fields set here; sprint and sneak are {@code StickyKeyBinding}s in hold mode (the default).
     */
    private static GameOptions headlessOptions() {
        GameOptions o = Reflect.allocate(GameOptions.class);
        for (String f : new String[] {"keyForward", "keyLeft", "keyBack", "keyRight", "keyJump", "keyUse", "keyAttack"}) {
            Reflect.set(GameOptions.class, o, f, Reflect.allocate(KeyBinding.class));
        }
        for (String f : new String[] {"keySneak", "keySprint"}) {
            StickyKeyBinding k = Reflect.allocate(StickyKeyBinding.class);
            java.util.function.BooleanSupplier hold = () -> false;
            Reflect.set(StickyKeyBinding.class, k, "toggleGetter", hold);
            Reflect.set(GameOptions.class, o, f, k);
        }
        o.autoJump = false;
        return o;
    }

    /**
     * The tags a vanilla server sends (synchronize tags packet), built with the vanilla tag loader from
     * the vanilla data pack inside the jar, then applied the way the client applies received tags.
     */
    private static TagManager loadVanillaTags() {
        ReloadableResourceManagerImpl resources = new ReloadableResourceManagerImpl(ResourceType.SERVER_DATA);
        resources.addPack(new DefaultResourcePack("minecraft"));
        TagGroupLoader<Block> blocks = new TagGroupLoader<>(Registry.BLOCK::getOrEmpty, "tags/blocks", "block");
        TagGroupLoader<Item> items = new TagGroupLoader<>(Registry.ITEM::getOrEmpty, "tags/items", "item");
        TagGroupLoader<Fluid> fluids = new TagGroupLoader<>(Registry.FLUID::getOrEmpty, "tags/fluids", "fluid");
        TagGroupLoader<net.minecraft.entity.EntityType<?>> entities =
                new TagGroupLoader<>(Registry.ENTITY_TYPE::getOrEmpty, "tags/entity_types", "entity_type");
        return TagManager.create(
                blocks.buildGroup(blocks.prepareReload(resources, Runnable::run).join()),
                items.buildGroup(items.prepareReload(resources, Runnable::run).join()),
                fluids.buildGroup(fluids.prepareReload(resources, Runnable::run).join()),
                entities.buildGroup(entities.prepareReload(resources, Runnable::run).join()));
    }

    /** A live run: world + player built from a scenario. */
    static final class Run {
        final Scenario sc;
        final ClientWorld world;
        final ClientPlayerEntity player;
        final List<int[]> loadedRects = new ArrayList<>();
        final TreeMap<Integer, String> statesUsed = new TreeMap<>();
        int tick;
        int teleportId;
        private boolean useHeld;

        Run(Scenario sc) {
            init();
            this.sc = sc;
            releaseKeys();
            options.autoJump = sc.autoJump;
            client.player = null;
            client.world = null;
            client.cameraEntity = null;
            Reflect.set(ClientPlayNetworkHandler.class, net, "playerListEntries", new java.util.HashMap<>());

            // --- join game (ClientPlayNetworkHandler.onGameJoin) ---
            client.interactionManager = new ClientPlayerInteractionManager(client, net);
            ClientWorld.Properties props = new ClientWorld.Properties(Difficulty.NORMAL, false, false);
            Reflect.set(ClientPlayNetworkHandler.class, net, "worldProperties", props);
            world = new ClientWorld(net, props, World.OVERWORLD, overworld, LOAD_DISTANCE, () -> DummyProfiler.INSTANCE,
                    client.worldRenderer, false, 0L);
            world.random.setSeed(0L);
            Reflect.set(ClientPlayNetworkHandler.class, net, "world", world);
            client.world = world;
            player = client.interactionManager.createPlayer(world, new StatHandler(), new ClientRecipeBook());
            client.player = player;
            player.yaw = -180.0F;
            player.afterSpawn();
            world.addPlayer(PLAYER_ENTITY_ID, player);
            player.input = new KeyboardInput(options);
            client.interactionManager.copyAbilities(player);
            client.cameraEntity = player;
            player.setEntityId(PLAYER_ENTITY_ID);
            Reflect.set(net.minecraft.entity.Entity.class, player, "random", new Random(0L));
            GameMode gm = gameMode(sc.gameMode);
            client.interactionManager.setGameMode(gm);
            // Tab list entry (player list packet): isSpectator()/isCreative() of client players read it.
            PlayerListS2CPacket list = new PlayerListS2CPacket();
            PlayerListS2CPacket.Entry entry = list.new Entry(new GameProfile(PLAYER_UUID, PLAYER_NAME), 0, gm, null);
            netPlayerList().put(PLAYER_UUID, new PlayerListEntry(entry));

            // --- chunk render distance centre + chunk data ---
            int ccx = floorDiv16(sc.x), ccz = floorDiv16(sc.z);
            world.getChunkManager().setChunkMapCenter(ccx, ccz);
            loadChunks(ccx, ccz);

            // --- abilities packet ---
            PacketByteBuf ab = buf();
            boolean allowFlying = sc.allowFlying != null ? sc.allowFlying : player.abilities.allowFlying;
            ab.writeByte((player.abilities.invulnerable ? 1 : 0) | (sc.flying ? 2 : 0) | (allowFlying ? 4 : 0)
                    | (player.abilities.creativeMode ? 8 : 0));
            ab.writeFloat(sc.flySpeed);
            ab.writeFloat(sc.walkSpeed);
            net.onPlayerAbilities(read(new PlayerAbilitiesS2CPacket(), ab));

            // --- entity properties (movement speed base + modifiers) ---
            PacketByteBuf at = buf();
            at.writeVarInt(PLAYER_ENTITY_ID);
            at.writeInt(1);
            at.writeIdentifier(Registry.ATTRIBUTE.getId(EntityAttributes.GENERIC_MOVEMENT_SPEED));
            at.writeDouble(sc.movementSpeedBase);
            at.writeVarInt(sc.speedModifiers.size());
            for (int i = 0; i < sc.speedModifiers.size(); i++) {
                double[] m = sc.speedModifiers.get(i);
                at.writeUuid(new UUID(0x6f7261636c65L, i));
                at.writeDouble(m[0]);
                at.writeByte((int) m[1]);
            }
            net.onEntityAttributes(read(new EntityAttributesS2CPacket(), at));

            // --- health / food, effects, equipment ---
            PacketByteBuf hp = buf();
            hp.writeFloat(20.0F);
            hp.writeVarInt(sc.foodLevel);
            hp.writeFloat(5.0F);
            net.onHealthUpdate(read(new HealthUpdateS2CPacket(), hp));
            for (Map.Entry<Integer, Integer> e : sc.effects.entrySet()) {
                PacketByteBuf ef = buf();
                ef.writeVarInt(PLAYER_ENTITY_ID);
                ef.writeByte(e.getKey());
                ef.writeByte(e.getValue());
                ef.writeVarInt(1_000_000);
                ef.writeByte(0);
                net.onEntityPotionEffect(read(new EntityStatusEffectS2CPacket(), ef));
            }
            if (sc.soulSpeed > 0 || sc.depthStrider > 0) {
                ItemStack boots = new ItemStack(Items.DIAMOND_BOOTS);
                if (sc.soulSpeed > 0) {
                    boots.addEnchantment(Enchantments.SOUL_SPEED, sc.soulSpeed);
                }
                if (sc.depthStrider > 0) {
                    boots.addEnchantment(Enchantments.DEPTH_STRIDER, sc.depthStrider);
                }
                net.onEquipmentUpdate(new EntityEquipmentUpdateS2CPacket(PLAYER_ENTITY_ID,
                        List.of(Pair.of(EquipmentSlot.FEET, boots))));
            }
            if (sc.usesItem()) {
                // A shield in the selected hotbar slot: using it holds the item use (blocking) for 72000 ticks.
                player.inventory.main.set(player.inventory.selectedSlot, new ItemStack(Items.SHIELD));
            }

            // --- position packet (absolute) ---
            teleport(sc.x, sc.y, sc.z, sc.yaw, sc.pitch, 0);
            if (sc.pose != null) {
                player.setPose(EntityPose.valueOf(sc.pose.toUpperCase()));
            }
            if (sc.bb != null) {
                player.setBoundingBox(new Box(sc.bb[0], sc.bb[1], sc.bb[2], sc.bb[3], sc.bb[4], sc.bb[5]));
            }

            // --- scenario overrides of the dynamic state ---
            player.setVelocity(sc.vx, sc.vy, sc.vz);
            player.setOnGround(sc.onGround);
            player.fallDistance = sc.fallDistance;
            if (sc.sprinting) {
                player.setSprinting(true);
            }
            // Chunk bookkeeping an earlier tick would have done (player was added before its chunk existed).
            invoke(ClientWorld.class, world, "checkEntityChunkPos", player);
            connection.sent.clear();
        }

        private Map<UUID, PlayerListEntry> netPlayerList() {
            @SuppressWarnings("unchecked")
            Map<UUID, PlayerListEntry> m = (Map<UUID, PlayerListEntry>) Reflect.get(ClientPlayNetworkHandler.class, net,
                    "playerListEntries");
            return m;
        }

        private void teleport(double x, double y, double z, float yaw, float pitch, int flags) {
            PacketByteBuf pl = buf();
            pl.writeDouble(x);
            pl.writeDouble(y);
            pl.writeDouble(z);
            pl.writeFloat(yaw);
            pl.writeFloat(pitch);
            pl.writeByte(flags);
            pl.writeVarInt(teleportId++);
            net.onPlayerPositionLook(read(new PlayerPositionLookS2CPacket(), pl));
        }

        private void loadChunks(int ccx, int ccz) {
            java.util.TreeSet<Long> chunks = new java.util.TreeSet<>();
            int radius = LOAD_DISTANCE + 3;
            if (sc.outside == Scenario.Outside.AIR) {
                // Every column the client chunk map can hold around the centre.
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
            // Blocks per chunk column.
            Map<Long, List<Scenario.Block>> byChunk = new TreeMap<>();
            for (Scenario.Block b : sc.blocks().values()) {
                if (b.y() < 0 || b.y() > 255) {
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
                    throw new IllegalArgumentException("chunk " + cx + "," + cz + " outside the client chunk map");
                }
                loadChunk(cx, cz, byChunk.getOrDefault(k, List.of()));
            }
        }

        /**
         * Builds the column like a server would hold it, serialises it with the real chunk data packet
         * and lets the client chunk manager decode it, exactly like ClientPlayNetworkHandler.onChunkData
         * (minus render scheduling).
         */
        private void loadChunk(int cx, int cz, List<Scenario.Block> blocks) {
            Biome[] biomes = new Biome[BiomeArray.DEFAULT_LENGTH];
            Arrays.fill(biomes, plains);
            WorldChunk source = new WorldChunk(world, new ChunkPos(cx, cz), new BiomeArray(registries.get(Registry.BIOME_KEY), biomes));
            ChunkSection[] sections = source.getSectionArray();
            for (Scenario.Block b : blocks) {
                BlockState state = States.byId(b.id());
                statesUsed.put(b.id(), States.name(state));
                int si = b.y() >> 4;
                if (sections[si] == null) {
                    sections[si] = new ChunkSection(si << 4);
                }
                sections[si].setBlockState(b.x() & 15, b.y() & 15, b.z() & 15, state, false);
            }
            ChunkDataS2CPacket packet = new ChunkDataS2CPacket(source, 65535);
            BiomeArray biomeArray = new BiomeArray(registries.get(Registry.BIOME_KEY), packet.getBiomeArray());
            WorldChunk chunk = world.getChunkManager().loadChunkFromPacket(cx, cz, biomeArray, packet.getReadBuffer(),
                    packet.getHeightmaps(), packet.getVerticalStripBitmask(), packet.isFullChunk());
            if (chunk == null) {
                throw new IllegalStateException("chunk not accepted: " + cx + "," + cz);
            }
            world.addEntitiesToChunk(chunk);
        }

        /** One client tick: inbound packets, keys, use key, mouse look, then the world tick. */
        void step(Scenario.Tick t) {
            for (Scenario.Event e : t.events()) {
                applyEvent(e);
            }
            setKeys(t.keys());
            useKey((t.keys() & Scenario.USE) != 0);
            player.yaw = t.yaw();
            player.pitch = t.pitch();
            world.tickEntities();
            tick++;
        }

        /** MinecraftClient.doItemUse / handleInputEvents with the crosshair on nothing. */
        private void useKey(boolean held) {
            if (held && !useHeld && !player.isUsingItem()) {
                for (Hand hand : Hand.values()) {
                    if (!player.getStackInHand(hand).isEmpty()
                            && client.interactionManager.interactItem(player, world, hand).isAccepted()) {
                        break;
                    }
                }
            } else if (!held && player.isUsingItem()) {
                client.interactionManager.stopUsingItem(player);
            }
            useHeld = held;
        }

        private void applyEvent(Scenario.Event e) {
            if (e instanceof Scenario.Velocity v) {
                PacketByteBuf b = buf();
                b.writeVarInt(PLAYER_ENTITY_ID);
                b.writeShort(v.x());
                b.writeShort(v.y());
                b.writeShort(v.z());
                net.onVelocityUpdate(read(new EntityVelocityUpdateS2CPacket(), b));
            } else if (e instanceof Scenario.Teleport tp) {
                teleport(tp.x(), tp.y(), tp.z(), tp.yaw(), tp.pitch(), tp.flags());
            } else if (e instanceof Scenario.SpeedAttribute sa) {
                PacketByteBuf b = buf();
                b.writeVarInt(PLAYER_ENTITY_ID);
                b.writeInt(1);
                b.writeIdentifier(Registry.ATTRIBUTE.getId(EntityAttributes.GENERIC_MOVEMENT_SPEED));
                b.writeDouble(sa.base());
                b.writeVarInt(sa.modifiers().size());
                for (Scenario.Modifier m : sa.modifiers()) {
                    b.writeUuid(m.uuid());
                    b.writeDouble(m.amount());
                    b.writeByte(m.operation());
                }
                net.onEntityAttributes(read(new EntityAttributesS2CPacket(), b));
            } else if (e instanceof Scenario.Effects fx) {
                java.util.Set<Integer> keep = new java.util.HashSet<>();
                for (int[] f : fx.effects()) {
                    keep.add(f[0]);
                }
                for (StatusEffectInstance inst : new java.util.ArrayList<>(player.getStatusEffects())) {
                    int id = StatusEffect.getRawId(inst.getEffectType());
                    if (!keep.contains(id)) {
                        PacketByteBuf b = buf();
                        b.writeVarInt(PLAYER_ENTITY_ID);
                        b.writeByte(id);
                        net.onRemoveEntityEffect(read(new RemoveEntityStatusEffectS2CPacket(), b));
                    }
                }
                for (int[] f : fx.effects()) {
                    PacketByteBuf b = buf();
                    b.writeVarInt(PLAYER_ENTITY_ID);
                    b.writeByte(f[0]);
                    b.writeByte(f[1]);
                    b.writeVarInt(1_000_000);
                    b.writeByte(0);
                    net.onEntityPotionEffect(read(new EntityStatusEffectS2CPacket(), b));
                }
            } else if (e instanceof Scenario.Abilities a) {
                PacketByteBuf b = buf();
                b.writeByte((a.invulnerable() ? 1 : 0) | (a.flying() ? 2 : 0) | (a.allowFlying() ? 4 : 0) | (a.creative() ? 8 : 0));
                b.writeFloat(a.flySpeed());
                b.writeFloat(a.walkSpeed());
                net.onPlayerAbilities(read(new PlayerAbilitiesS2CPacket(), b));
            }
        }

        int soulSpeedLevel() {
            return EnchantmentHelper.getEquipmentLevel(Enchantments.SOUL_SPEED, player);
        }
    }

    static PacketByteBuf buf() {
        return new PacketByteBuf(Unpooled.buffer());
    }

    static <P extends net.minecraft.network.Packet<?>> P read(P packet, PacketByteBuf buf) {
        try {
            packet.read(buf);
        } catch (java.io.IOException e) {
            throw new IllegalStateException(e);
        }
        if (buf.readableBytes() != 0) {
            throw new IllegalStateException("unread bytes decoding " + packet.getClass().getSimpleName());
        }
        return packet;
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

    static void releaseKeys() {
        setKeys(0);
    }

    static void setKeys(int keys) {
        KeyBinding[] order = {options.keyForward, options.keyBack, options.keyLeft, options.keyRight,
                options.keyJump, options.keySneak, options.keySprint};
        for (int i = 0; i < order.length; i++) {
            order[i].setPressed((keys & (1 << i)) != 0);
        }
        options.keyUse.setPressed((keys & Scenario.USE) != 0);
    }

    static GameMode gameMode(Scenario.GameMode m) {
        return switch (m) {
            case SURVIVAL -> GameMode.SURVIVAL;
            case CREATIVE -> GameMode.CREATIVE;
            case ADVENTURE -> GameMode.ADVENTURE;
            case SPECTATOR -> GameMode.SPECTATOR;
        };
    }

    static int floorDiv16(double v) {
        return Math.floorDiv((int) Math.floor(v), 16);
    }

    static long chunkKey(int cx, int cz) {
        return ((long) cx << 32) | (cz & 0xffffffffL);
    }
}
