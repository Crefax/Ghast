package ghast.oracle.v26_1;

import com.mojang.datafixers.util.Pair;
import ghast.oracle.Json;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import io.netty.buffer.ByteBufUtil;
import it.unimi.dsi.fastutil.ints.IntArrayList;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.particles.ColorParticleOption;
import net.minecraft.core.particles.ParticleOptions;
import net.minecraft.core.particles.ParticleTypes;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.ComponentSerialization;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientGamePacketListener;
import net.minecraft.network.protocol.game.ClientboundAddEntityPacket;
import net.minecraft.network.protocol.game.ClientboundEntityPositionSyncPacket;
import net.minecraft.network.protocol.game.ClientboundForgetLevelChunkPacket;
import net.minecraft.network.protocol.game.ClientboundMoveEntityPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoRemovePacket;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket;
import net.minecraft.network.protocol.game.ClientboundRemoveEntitiesPacket;
import net.minecraft.network.protocol.game.ClientboundRotateHeadPacket;
import net.minecraft.network.protocol.game.ClientboundSetEntityDataPacket;
import net.minecraft.network.protocol.game.ClientboundSetEntityLinkPacket;
import net.minecraft.network.protocol.game.ClientboundSetEntityMotionPacket;
import net.minecraft.network.protocol.game.ClientboundSetPassengersPacket;
import net.minecraft.network.protocol.game.ClientboundTakeItemEntityPacket;
import net.minecraft.network.protocol.game.ClientboundTeleportEntityPacket;
import net.minecraft.network.protocol.game.ClientboundUpdateAttributesPacket;
import net.minecraft.network.syncher.EntityDataSerializer;
import net.minecraft.network.syncher.EntityDataSerializers;
import net.minecraft.network.syncher.SynchedEntityData;
import net.minecraft.resources.Identifier;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.Pose;
import net.minecraft.world.entity.PositionMoveRotation;
import net.minecraft.world.entity.Relative;
import net.minecraft.world.entity.ai.attributes.Attribute;
import net.minecraft.world.entity.ai.attributes.AttributeInstance;
import net.minecraft.world.entity.ai.attributes.AttributeModifier;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

import java.util.ArrayList;
import java.util.EnumSet;
import java.util.HashSet;
import java.util.List;
import java.util.Optional;
import java.util.OptionalInt;
import java.util.Set;
import java.util.TreeSet;
import java.util.UUID;
import java.util.function.Consumer;

/**
 * Entity tracking through the real client: random clientbound entity packets (spawns, moves, position
 * syncs, teleports, head rotations, velocities, metadata, attributes, passengers, removals, pickups,
 * tab list, chunk unloads) are built as wire bytes, decoded with the packets' own stream codecs and
 * handled by the real {@code ClientPacketListener}; between batches the level ticks its entities
 * ({@code ClientLevel.tickEntities}, inside {@link Harness.Run#step}). Each tick line lists the
 * packets (name + body hex) and then every entity as the client holds it at pick time (after the
 * packets, before the tick): position, rotation, head rotation, box, pickable, riding links, plus
 * the tab list. Schema: oracle/README.md, "entity_tracking.jsonl.gz".
 */
final class EntityTrackDump {
    static final int CASES = 24;
    static final int TICKS = 72;

    private EntityTrackDump() {}

    /** Entity types the fuzz spawns: interpolated living entities with size variants, and a few that move by snapping. */
    /**
     * Entity types the fuzz spawns: interpolated living entities with size variants, and a few that move by
     * snapping. Set after bootstrap.
     */
    private static EntityType<?>[] POOL;
    private static net.minecraft.network.syncher.EntityDataAccessor<Boolean> NO_GRAVITY;
    private static net.minecraft.network.syncher.EntityDataAccessor<ItemStack> ITEM_STACK;

    @SuppressWarnings("unchecked")
    private static void init() {
        Harness.init();
        if (POOL != null) {
            return;
        }
        Reflect.set(net.minecraft.client.Minecraft.class, Harness.client, "entityRenderDispatcher", Stubs.NoEntityRenderers.create());
        POOL = new EntityType<?>[] {
            EntityType.ZOMBIE, EntityType.PIG, EntityType.VILLAGER, EntityType.ARMOR_STAND, EntityType.SLIME,
            EntityType.MAGMA_CUBE, EntityType.CAMEL, EntityType.GOAT, EntityType.FOX, EntityType.COW, EntityType.PLAYER,
            EntityType.PLAYER, EntityType.ITEM, EntityType.END_CRYSTAL, EntityType.ENDERMAN, EntityType.CHICKEN,
        };
        NO_GRAVITY = (net.minecraft.network.syncher.EntityDataAccessor<Boolean>) Reflect.get(Entity.class, null, "DATA_NO_GRAVITY");
        ITEM_STACK = (net.minecraft.network.syncher.EntityDataAccessor<ItemStack>) Reflect.get(ItemEntity.class, null, "DATA_ITEM");
    }

    private static final int ID_BASE = 100;
    private static final int ID_COUNT = 18;
    private static final int PLAYERS = 4;

    static UUID playerUuid(int k) {
        return new UUID(0x7072_6f66_696c_6500L, k);
    }

    static void lines(long seed, int cases, int ticks, Consumer<String> sink) {
        init();
        for (int c = 0; c < cases; c++) {
            one(seed, c, ticks, sink);
        }
    }

    private static final class Case {
        final Rng rng;
        final Harness.Run run;
        final List<String[]> packets = new ArrayList<>();
        final Set<Integer> rode = new HashSet<>();
        final Set<Long> forgotten = new TreeSet<>();

        Case(Rng rng, Harness.Run run) {
            this.rng = rng;
            this.run = run;
        }
    }

    static void one(long seed, int index, int ticks, Consumer<String> sink) {
        Rng rng = new Rng(seed * 0x9E3779B97F4A7C15L + index * 7919L + 17);
        Scenario sc = new Scenario("entity_tracking_" + index).spawn(0.5, 61, 0.5, 0, 0);
        int stone = Block.getId(Blocks.STONE.defaultBlockState());
        sc.fill(-3, 60, -3, 3, 60, 3, stone, 0);
        for (int t = 0; t < ticks; t++) {
            sc.ticks.add(new Scenario.Tick(0, 0, 0, List.of()));
        }
        Harness.Run run = new Harness.Run(sc);
        Case c = new Case(rng, run);
        Json h = new Json().beginObject().field("type", "case").field("case", index).field("seed", seed)
                .field("player_id", Harness.PLAYER_ENTITY_ID)
                .field("scale_attribute", BuiltInRegistries.ATTRIBUTE.getId(Attributes.SCALE.value()));
        h.key("loaded").beginArray();
        for (int[] r : run.loadedRects) {
            h.beginArray().value(r[0]).value(r[1]).value(r[2]).value(r[3]).endArray();
        }
        h.endArray();
        sink.accept(h.endObject().toString());
        for (int t = 0; t < ticks; t++) {
            c.packets.clear();
            int actions = rng.range(0, 4);
            if (t == 0) {
                // Tab list entries first, so most player spawns find their profile.
                for (int k = 0; k < 3; k++) {
                    playerInfo(c);
                }
                actions = 6;
            }
            for (int a = 0; a < actions; a++) {
                action(c, t == 0);
            }
            sink.accept(record(c, index, t));
            run.step(sc.ticks.get(t));
        }
        Harness.releaseKeys();
    }

    // ---------------------------------------------------------------------------------------------
    // packets

    private static <P extends Packet<? super ClientGamePacketListener>> void send(Case c, String name,
            StreamCodec<? super RegistryFriendlyByteBuf, P> codec, Consumer<RegistryFriendlyByteBuf> body) {
        RegistryFriendlyByteBuf b = Harness.buf();
        body.accept(b);
        byte[] bytes = new byte[b.readableBytes()];
        b.getBytes(b.readerIndex(), bytes);
        P p = codec.decode(b);
        if (b.readableBytes() != 0) {
            throw new IllegalStateException("unread bytes in " + name);
        }
        c.packets.add(new String[] {name, ByteBufUtil.hexDump(bytes)});
        p.handle(Harness.listener);
    }

    /** A packet built as a record and written with its own codec. */
    private static <P extends Packet<? super ClientGamePacketListener>> void sendPacket(Case c, String name,
            StreamCodec<? super RegistryFriendlyByteBuf, P> codec, P packet) {
        send(c, name, codec, b -> codec.encode(b, packet));
    }

    private static List<Entity> entities(Case c) {
        List<Entity> out = new ArrayList<>();
        for (Entity e : c.run.level.entitiesForRendering()) {
            if (e.getId() != Harness.PLAYER_ENTITY_ID) {
                out.add(e);
            }
        }
        out.sort((a, b) -> Integer.compare(a.getId(), b.getId()));
        return out;
    }

    private static Entity anyEntity(Case c) {
        List<Entity> es = entities(c);
        return es.isEmpty() ? null : es.get(c.rng.nextInt(es.size()));
    }

    /** An existing entity id most of the time, sometimes a random (maybe unknown) one. */
    private static int targetId(Case c) {
        Entity e = c.rng.chance(0.9) ? anyEntity(c) : null;
        return e != null ? e.getId() : ID_BASE + c.rng.nextInt(ID_COUNT + 2);
    }

    private static void action(Case c, boolean spawnOnly) {
        Rng r = c.rng;
        int n = entities(c).size();
        double w = r.nextDouble() * 100;
        if (spawnOnly || n < 3 || w < (n < 10 ? 12 : 4)) {
            spawn(c);
        } else if (w < 42) {
            move(c);
        } else if (w < 52) {
            sync(c);
        } else if (w < 58) {
            teleport(c);
        } else if (w < 63) {
            int id = targetId(c);
            byte head = (byte) r.nextInt(256);
            send(c, "rotate_head", ClientboundRotateHeadPacket.STREAM_CODEC, b -> {
                b.writeVarInt(id);
                b.writeByte(head);
            });
        } else if (w < 67) {
            velocity(c);
        } else if (w < 80) {
            metadata(c);
        } else if (w < 84) {
            attributes(c);
        } else if (w < 87) {
            remove(c);
        } else if (w < 89) {
            pickup(c);
        } else if (w < 92) {
            passengers(c);
        } else if (w < 96) {
            playerInfo(c);
        } else if (w < 97) {
            int id = targetId(c), holder = r.chance(0.3) ? 0 : targetId(c);
            send(c, "set_entity_link", ClientboundSetEntityLinkPacket.STREAM_CODEC, b -> {
                b.writeInt(id);
                b.writeInt(holder);
            });
        } else {
            chunk(c);
        }
    }

    private static double pickD(Rng r, double[] values) {
        return values[r.nextInt(values.length)];
    }

    private static double near(Rng r, double around, double spread) {
        return around + (r.nextDouble() * 2 - 1) * spread;
    }

    private static void spawn(Case c) {
        Rng r = c.rng;
        int id = ID_BASE + r.nextInt(ID_COUNT);
        EntityType<?> type = r.pick(POOL);
        UUID uuid = type == EntityType.PLAYER ? playerUuid(r.nextInt(PLAYERS)) : new UUID(r.nextLong(), r.nextLong());
        double x = near(r, 0.5, 24), y = 61 + r.nextDouble() * 6, z = near(r, 0.5, 24);
        if (r.chance(0.1)) {
            x = Math.floor(x) + 0.5;
            z = Math.floor(z) + 0.5;
            y = Math.floor(y);
        }
        Vec3 movement = type == EntityType.ITEM || r.chance(0.3) ? Vec3.ZERO : randomVelocity(r);
        byte xRot = (byte) r.nextInt(256), yRot = (byte) r.nextInt(256), head = (byte) r.nextInt(256);
        int data = r.chance(0.2) ? r.range(0, 5) : 0;
        double fx = x, fy = y, fz = z;
        Entity before = c.run.level.getEntity(id);
        send(c, "add_entity", ClientboundAddEntityPacket.STREAM_CODEC, b -> {
            b.writeVarInt(id);
            b.writeUUID(uuid);
            b.writeVarInt(BuiltInRegistries.ENTITY_TYPE.getId(type));
            b.writeDouble(fx);
            b.writeDouble(fy);
            b.writeDouble(fz);
            Vec3.LP_STREAM_CODEC.encode(b, movement);
            b.writeByte(xRot);
            b.writeByte(yRot);
            b.writeByte(head);
            b.writeVarInt(data);
        });
        if (c.run.level.getEntity(id) != before) {
            c.rode.remove(id); // a new entity under this id (a refused spawn keeps the old one)
        }
        if (type == EntityType.ITEM && c.run.level.getEntity(id) instanceof ItemEntity item) {
            // A floating stack: no gravity, so the client-side item physics leaves it where the packets put it.
            List<SynchedEntityData.DataValue<?>> values = new ArrayList<>();
            values.add(SynchedEntityData.DataValue.create(NO_GRAVITY, true));
            values.add(SynchedEntityData.DataValue.create(ITEM_STACK, randomStack(r)));
            sendPacket(c, "set_entity_data", ClientboundSetEntityDataPacket.STREAM_CODEC,
                    new ClientboundSetEntityDataPacket(item.getId(), values));
        }
    }

    private static Vec3 randomVelocity(Rng r) {
        double s = pickD(r, new double[] {0.001, 0.05, 0.4, 2.0, 9.0});
        return new Vec3(near(r, 0, s), near(r, 0, s), near(r, 0, s));
    }

    private static short delta(Rng r) {
        return switch (r.nextInt(6)) {
            case 0 -> 0;
            case 1 -> (short) r.range(-32768, 32767);
            default -> (short) r.range(-1500, 1500);
        };
    }

    private static void move(Case c) {
        Rng r = c.rng;
        int id = targetId(c);
        boolean ground = r.chance(0.5);
        byte yRot = (byte) r.nextInt(256), xRot = (byte) r.nextInt(256);
        short xa = delta(r), ya = delta(r), za = delta(r);
        switch (r.nextInt(4)) {
            case 0 -> send(c, "move_entity_pos", ClientboundMoveEntityPacket.Pos.STREAM_CODEC, b -> {
                b.writeVarInt(id);
                b.writeShort(xa);
                b.writeShort(ya);
                b.writeShort(za);
                b.writeBoolean(ground);
            });
            case 1 -> send(c, "move_entity_rot", ClientboundMoveEntityPacket.Rot.STREAM_CODEC, b -> {
                b.writeVarInt(id);
                b.writeByte(yRot);
                b.writeByte(xRot);
                b.writeBoolean(ground);
            });
            default -> send(c, "move_entity_pos_rot", ClientboundMoveEntityPacket.PosRot.STREAM_CODEC, b -> {
                b.writeVarInt(id);
                b.writeShort(xa);
                b.writeShort(ya);
                b.writeShort(za);
                b.writeByte(yRot);
                b.writeByte(xRot);
                b.writeBoolean(ground);
            });
        }
    }

    private static Vec3 target(Case c, int id, boolean far) {
        Rng r = c.rng;
        Entity e = c.run.level.getEntity(id);
        Vec3 at = e != null ? e.getPositionCodec().getBase() : new Vec3(0.5, 62, 0.5);
        double s = far ? 90 : pickD(r, new double[] {0.01, 0.5, 3});
        return new Vec3(near(r, at.x, s), near(r, at.y, far ? 8 : s), near(r, at.z, s));
    }

    private static float angle(Rng r) {
        return r.chance(0.2) ? (float) near(r, 0, 720) : (float) near(r, 0, 180);
    }

    private static void sync(Case c) {
        Rng r = c.rng;
        int id = targetId(c);
        Vec3 pos = target(c, id, r.chance(0.12));
        Vec3 dm = r.chance(0.5) ? Vec3.ZERO : randomVelocity(r);
        PositionMoveRotation v = new PositionMoveRotation(pos, dm, angle(r), angle(r));
        sendPacket(c, "entity_position_sync", ClientboundEntityPositionSyncPacket.STREAM_CODEC,
                new ClientboundEntityPositionSyncPacket(id, v, r.chance(0.5)));
    }

    private static void teleport(Case c) {
        Rng r = c.rng;
        int id = targetId(c);
        int flags = r.chance(0.3) ? 0 : r.nextInt(512);
        boolean item = c.run.level.getEntity(id) instanceof ItemEntity;
        if (item) {
            flags &= 0x1f; // no velocity for items (their client physics would move them)
        }
        Set<Relative> rel = Relative.unpack(flags);
        Vec3 pos = rel.contains(Relative.X) || rel.contains(Relative.Y) || rel.contains(Relative.Z)
                ? new Vec3(near(r, 0, 2), near(r, 0, 2), near(r, 0, 2)) : target(c, id, r.chance(0.1));
        if (rel.contains(Relative.X) != rel.contains(Relative.Z) || rel.contains(Relative.X) != rel.contains(Relative.Y)) {
            Vec3 abs = target(c, id, false);
            pos = new Vec3(rel.contains(Relative.X) ? pos.x : abs.x, rel.contains(Relative.Y) ? pos.y : abs.y,
                    rel.contains(Relative.Z) ? pos.z : abs.z);
        }
        Vec3 dm = item || r.chance(0.5) ? Vec3.ZERO : randomVelocity(r);
        PositionMoveRotation v = new PositionMoveRotation(pos, dm, angle(r), angle(r));
        sendPacket(c, "teleport_entity", ClientboundTeleportEntityPacket.STREAM_CODEC,
                new ClientboundTeleportEntityPacket(id, v, rel, r.chance(0.5)));
    }

    private static void velocity(Case c) {
        Rng r = c.rng;
        Entity e = anyEntity(c);
        if (e == null || e instanceof ItemEntity) {
            return; // items would start their own client-side physics
        }
        Vec3 v = r.chance(0.1) ? Vec3.ZERO : randomVelocity(r);
        sendPacket(c, "set_entity_motion", ClientboundSetEntityMotionPacket.STREAM_CODEC, new ClientboundSetEntityMotionPacket(e.getId(), v));
    }

    private static ItemStack randomStack(Rng r) {
        ItemStack s = new ItemStack(r.pick(new net.minecraft.world.item.Item[] {Items.STONE, Items.DIAMOND_SWORD, Items.WHEAT, Items.OAK_LOG}),
                r.range(1, 64));
        if (r.chance(0.3)) {
            s.set(net.minecraft.core.component.DataComponents.CUSTOM_NAME, Component.literal("stack " + r.nextInt(100)));
        }
        if (s.isDamageableItem() && r.chance(0.5)) {
            s.setDamageValue(r.range(1, 100));
        }
        return s;
    }

    /** A random value of the item's own serializer, or its current value where a random one is awkward. */
    @SuppressWarnings({"unchecked", "rawtypes"})
    private static SynchedEntityData.DataValue<?> randomValue(Case c, Entity e, SynchedEntityData.DataItem<?> item) {
        Rng r = c.rng;
        EntityDataSerializer s = item.getAccessor().serializer();
        int id = item.getAccessor().id();
        Object v = item.getValue();
        if (s == EntityDataSerializers.BYTE) {
            v = (byte) (id == 0 ? r.nextInt(256) & ~0x40 : r.nextInt(256));
        } else if (s == EntityDataSerializers.INT) {
            v = r.range(0, 5);
        } else if (s == EntityDataSerializers.FLOAT) {
            v = (float) (r.nextDouble() * 20);
        } else if (s == EntityDataSerializers.BOOLEAN) {
            v = r.chance(0.5);
        } else if (s == EntityDataSerializers.POSE) {
            v = r.pick(Pose.values());
        } else if (s == EntityDataSerializers.OPTIONAL_COMPONENT) {
            v = r.chance(0.5) ? Optional.empty() : Optional.of(Component.literal("name " + r.nextInt(1000)));
        } else if (s == EntityDataSerializers.COMPONENT) {
            v = Component.literal("text " + r.nextInt(1000));
        } else if (s == EntityDataSerializers.STRING) {
            v = "s" + r.nextInt(1000);
        } else if (s == EntityDataSerializers.LONG) {
            v = r.nextLong();
        } else if (s == EntityDataSerializers.ITEM_STACK) {
            v = r.chance(0.2) ? ItemStack.EMPTY : randomStack(r);
        } else if (s == EntityDataSerializers.PARTICLES) {
            List<ParticleOptions> ps = new ArrayList<>();
            for (int i = r.nextInt(3); i > 0; i--) {
                ps.add(ColorParticleOption.create(ParticleTypes.ENTITY_EFFECT, (int) r.nextLong()));
            }
            v = ps;
        } else if (s == EntityDataSerializers.OPTIONAL_BLOCK_POS) {
            v = r.chance(0.5) ? Optional.empty() : Optional.of(new BlockPos(r.range(-100, 100), r.range(0, 100), r.range(-100, 100)));
        } else if (s == EntityDataSerializers.OPTIONAL_UNSIGNED_INT) {
            v = r.chance(0.5) ? OptionalInt.empty() : OptionalInt.of(r.range(0, 1000));
        }
        return new SynchedEntityData.DataValue(id, s, v);
    }

    private static void metadata(Case c) {
        Rng r = c.rng;
        Entity e = anyEntity(c);
        if (e == null) {
            return;
        }
        SynchedEntityData.DataItem<?>[] items = (SynchedEntityData.DataItem<?>[]) Reflect.get(SynchedEntityData.class, e.getEntityData(), "itemsById");
        List<SynchedEntityData.DataValue<?>> values = new ArrayList<>();
        double p = r.chance(0.2) ? 0.9 : 0.25;
        for (SynchedEntityData.DataItem<?> item : items) {
            int id = item.getAccessor().id();
            if (e instanceof ItemEntity && id == NO_GRAVITY.id()) {
                continue; // keep the floating stack floating
            }
            // Size-relevant entries (pose, the type's baby / size / armor stand flags) more often.
            boolean sizeKey = id == 6 || (id >= 15 && id <= 17);
            if (r.chance(sizeKey ? 0.5 : p)) {
                values.add(randomValue(c, e, item));
            }
        }
        if (values.isEmpty()) {
            return;
        }
        sendPacket(c, "set_entity_data", ClientboundSetEntityDataPacket.STREAM_CODEC, new ClientboundSetEntityDataPacket(e.getId(), values));
    }

    private static void attributes(Case c) {
        Rng r = c.rng;
        Entity e = anyEntity(c);
        if (!(e instanceof LivingEntity living)) {
            return;
        }
        List<AttributeInstance> list = new ArrayList<>();
        List<Holder<Attribute>> attrs = new ArrayList<>(List.of(Attributes.MOVEMENT_SPEED, Attributes.SCALE, Attributes.MAX_HEALTH));
        for (Holder<Attribute> a : attrs) {
            if (living.getAttributes().getInstance(a) == null || r.chance(0.3)) {
                continue;
            }
            AttributeInstance inst = new AttributeInstance(a, i -> {});
            inst.setBaseValue(a == Attributes.SCALE ? pickD(r, new double[] {1.0, 0.5, 0.25 + r.nextDouble() * 3, 20, 0.01}) : r.nextDouble());
            for (AttributeModifier.Operation op : AttributeModifier.Operation.values()) {
                if (r.chance(0.3)) {
                    inst.addTransientModifier(new AttributeModifier(Identifier.fromNamespaceAndPath("oracle", "m" + op.ordinal()),
                            near(r, 0, 0.6), op));
                }
            }
            list.add(inst);
        }
        if (!list.isEmpty()) {
            sendPacket(c, "update_attributes", ClientboundUpdateAttributesPacket.STREAM_CODEC,
                    new ClientboundUpdateAttributesPacket(e.getId(), list));
        }
    }

    private static void remove(Case c) {
        Rng r = c.rng;
        IntArrayList ids = new IntArrayList();
        for (int i = r.range(1, 3); i > 0; i--) {
            ids.add(targetId(c));
        }
        sendPacket(c, "remove_entities", ClientboundRemoveEntitiesPacket.STREAM_CODEC, new ClientboundRemoveEntitiesPacket(ids));
    }

    private static void pickup(Case c) {
        Rng r = c.rng;
        Entity item = null;
        for (Entity e : entities(c)) {
            if (e instanceof ItemEntity) {
                item = e;
            }
        }
        int id = item != null && r.chance(0.8) ? item.getId() : targetId(c);
        // The client casts the collector to a living entity (a missing one means the client player).
        int collector = Harness.PLAYER_ENTITY_ID;
        if (r.chance(0.5)) {
            int other = targetId(c);
            Entity o = c.run.level.getEntity(other);
            collector = o == null || o instanceof LivingEntity ? other : collector;
        }
        int amount = r.range(0, 40);
        sendPacket(c, "take_item_entity", ClientboundTakeItemEntityPacket.STREAM_CODEC, new ClientboundTakeItemEntityPacket(id, collector, amount));
    }

    private static void passengers(Case c) {
        Rng r = c.rng;
        int vehicle = targetId(c);
        List<Integer> riders = new ArrayList<>();
        // No riding cycles (a server never sends one; the client would recurse forever).
        Set<Integer> above = new HashSet<>();
        for (Entity v = c.run.level.getEntity(vehicle); v != null; v = v.getVehicle()) {
            above.add(v.getId());
        }
        above.add(vehicle);
        for (int i = r.nextInt(3); i > 0; i--) {
            int p = targetId(c);
            if (!above.contains(p) && !riders.contains(p)) {
                riders.add(p);
            }
        }
        for (int p : riders) {
            c.rode.add(p);
        }
        send(c, "set_passengers", ClientboundSetPassengersPacket.STREAM_CODEC, b -> {
            b.writeVarInt(vehicle);
            b.writeVarInt(riders.size());
            for (int p : riders) {
                b.writeVarInt(p);
            }
        });
    }

    private static void playerInfo(Case c) {
        Rng r = c.rng;
        int k = r.nextInt(PLAYERS);
        UUID uuid = playerUuid(k);
        if (r.chance(0.15)) {
            sendPacket(c, "player_info_remove", ClientboundPlayerInfoRemovePacket.STREAM_CODEC, new ClientboundPlayerInfoRemovePacket(List.of(uuid)));
            return;
        }
        EnumSet<ClientboundPlayerInfoUpdatePacket.Action> actions = EnumSet.noneOf(ClientboundPlayerInfoUpdatePacket.Action.class);
        boolean known = Harness.listener.getPlayerInfo(uuid) != null;
        if (!known || r.chance(0.2)) {
            actions.add(ClientboundPlayerInfoUpdatePacket.Action.ADD_PLAYER);
        }
        for (ClientboundPlayerInfoUpdatePacket.Action a : ClientboundPlayerInfoUpdatePacket.Action.values()) {
            if (a != ClientboundPlayerInfoUpdatePacket.Action.ADD_PLAYER && a != ClientboundPlayerInfoUpdatePacket.Action.INITIALIZE_CHAT
                    && r.chance(0.5)) {
                actions.add(a);
            }
        }
        if (actions.isEmpty()) {
            actions.add(ClientboundPlayerInfoUpdatePacket.Action.UPDATE_LATENCY);
        }
        int gm = r.nextInt(4);
        boolean props = r.chance(0.5);
        Component display = r.chance(0.5) ? Component.literal("Player " + k) : null;
        send(c, "player_info_update", ClientboundPlayerInfoUpdatePacket.STREAM_CODEC, b -> {
            b.writeEnumSet(actions, ClientboundPlayerInfoUpdatePacket.Action.class);
            b.writeVarInt(1);
            b.writeUUID(uuid);
            for (ClientboundPlayerInfoUpdatePacket.Action a : actions) {
                switch (a) {
                    case ADD_PLAYER -> {
                        b.writeUtf("player" + k, 16);
                        if (props) {
                            b.writeVarInt(1);
                            b.writeUtf("textures");
                            b.writeUtf("dGV4dHVyZXM=");
                            b.writeBoolean(true);
                            b.writeUtf("c2lnbmF0dXJl");
                        } else {
                            b.writeVarInt(0);
                        }
                    }
                    case UPDATE_GAME_MODE -> b.writeVarInt(gm);
                    case UPDATE_LISTED -> b.writeBoolean(r.chance(0.7));
                    case UPDATE_LATENCY -> b.writeVarInt(r.range(0, 900));
                    case UPDATE_DISPLAY_NAME -> {
                        b.writeBoolean(display != null);
                        if (display != null) {
                            ComponentSerialization.TRUSTED_STREAM_CODEC.encode(b, display);
                        }
                    }
                    case UPDATE_LIST_ORDER -> b.writeVarInt(r.range(-3, 300));
                    case UPDATE_HAT -> b.writeBoolean(r.chance(0.5));
                    default -> throw new IllegalStateException();
                }
            }
        });
    }

    private static void chunk(Case c) {
        Rng r = c.rng;
        if (!c.forgotten.isEmpty() && r.chance(0.5)) {
            long k = c.forgotten.iterator().next();
            c.forgotten.remove(k);
            int cx = (int) (k >> 32), cz = (int) k;
            c.run.loadChunk(cx, cz, List.of());
            // The tracker only reads the chunk coordinates of a chunk packet.
            c.packets.add(new String[] {"level_chunk_with_light", String.format("%08x%08x", cx, cz)});
            return;
        }
        Entity e = anyEntity(c);
        if (e == null) {
            return;
        }
        ChunkPos pos = e.chunkPosition();
        if (Math.abs(pos.x()) <= 1 && Math.abs(pos.z()) <= 1) {
            return; // keep the floor under the client player
        }
        c.forgotten.add(Harness.chunkKey(pos.x(), pos.z()));
        sendPacket(c, "forget_level_chunk", ClientboundForgetLevelChunkPacket.STREAM_CODEC, new ClientboundForgetLevelChunkPacket(pos));
    }

    // ---------------------------------------------------------------------------------------------
    // record

    private static String hex64(double d) {
        return String.format("%016x", Double.doubleToRawLongBits(d));
    }

    private static String hex32(float f) {
        return String.format("%08x", Float.floatToRawIntBits(f));
    }

    private static String record(Case c, int index, int tick) {
        Json j = new Json().beginObject().field("type", "tick").field("case", index).field("tick", tick);
        j.key("packets").beginArray();
        for (String[] p : c.packets) {
            j.beginArray().value(p[0]).value(p[1]).endArray();
        }
        j.endArray();
        j.key("entities").beginArray();
        for (Entity e : entities(c)) {
            j.beginObject().field("id", e.getId()).field("type", BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).getPath());
            Vec3 p = e.position();
            j.key("pos").beginArray().value(hex64(p.x)).value(hex64(p.y)).value(hex64(p.z)).endArray();
            j.key("rot").beginArray().value(hex32(e.getYRot())).value(hex32(e.getXRot())).endArray();
            if (e instanceof LivingEntity l) {
                j.field("head", hex32(l.getYHeadRot())).field("body", hex32(l.yBodyRot));
            }
            j.field("ground", e.onGround());
            AABB b = e.getBoundingBox();
            j.key("box").beginArray().value(hex64(b.minX)).value(hex64(b.minY)).value(hex64(b.minZ))
                    .value(hex64(b.maxX)).value(hex64(b.maxY)).value(hex64(b.maxZ)).endArray();
            j.field("pickable", e.isPickable()).field("spectator", e.isSpectator());
            j.field("ticking", c.run.level.isTickingEntity(e)).field("rode", c.rode.contains(e.getId()));
            j.field("vehicle", e.getVehicle() == null ? -1 : e.getVehicle().getId());
            j.key("passengers").beginArray();
            for (Entity q : e.getPassengers()) {
                j.value(q.getId());
            }
            j.endArray();
            if (e instanceof ItemEntity item) {
                j.field("count", item.getItem().getCount());
            }
            j.endObject();
        }
        j.endArray();
        j.key("tab").beginArray();
        for (int k = 0; k < PLAYERS; k++) {
            PlayerInfo info = Harness.listener.getPlayerInfo(playerUuid(k));
            if (info != null) {
                GameType gm = info.getGameMode();
                j.beginArray().value(k).value(info.getProfile().name()).value(gm == null ? -1 : gm.getId()).value(info.getLatency())
                        .value(info.getTabListDisplayName() != null).endArray();
            }
        }
        j.endArray();
        return j.endObject().toString();
    }
}
