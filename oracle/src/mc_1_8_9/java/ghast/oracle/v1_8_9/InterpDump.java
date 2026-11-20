package ghast.oracle.v1_8_9;

import com.mojang.authlib.GameProfile;
import ghast.oracle.Json;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import io.netty.buffer.Unpooled;
import net.minecraft.client.MinecraftClient;
import net.minecraft.client.network.ClientPlayNetworkHandler;
import net.minecraft.client.network.PlayerListEntry;
import net.minecraft.entity.Entity;
import net.minecraft.network.Packet;
import net.minecraft.network.listener.ClientPlayPacketListener;
import net.minecraft.network.packet.s2c.play.EntityPositionS2CPacket;
import net.minecraft.network.packet.s2c.play.EntityS2CPacket;
import net.minecraft.network.packet.s2c.play.EntitySpawnS2CPacket;
import net.minecraft.network.packet.s2c.play.MobSpawnS2CPacket;
import net.minecraft.network.packet.s2c.play.PlayerListS2CPacket;
import net.minecraft.network.packet.s2c.play.PlayerSpawnS2CPacket;
import net.minecraft.util.PacketByteBuf;
import net.minecraft.world.level.LevelInfo;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.function.Consumer;
import java.util.function.Supplier;

/**
 * 1.8.9 client-side interpolation of other entities (golden {@code interp.jsonl.gz}): players
 * ({@code OtherClientPlayerEntity}), zombies and an armor stand ({@code LivingEntity}) spawned by
 * real spawn packets, then random relative-move, look, move-look, bare {@code entity} and teleport
 * packets (some within 1/32 of the drawn position) handled by the real
 * {@code ClientPlayNetworkHandler}, and {@code World.tickEntities()} every tick. Each line is one
 * case: per tick the packets (id and body, hex) and every entity's position and rotation after the
 * tick (what the next tick's crosshair pick reads).
 */
final class InterpDump {
    static final int CASES = 40, TICKS = 60;

    private InterpDump() {}

    private static final int SPAWN_PLAYER = 0x0C, SPAWN_OBJECT = 0x0E, SPAWN_MOB = 0x0F, ENTITY = 0x14,
            REL_MOVE = 0x15, LOOK = 0x16, MOVE_LOOK = 0x17, TELEPORT = 0x18;

    /** One packet: its id, the body as written, and the vanilla packet that reads it. */
    private record Sent(int id, byte[] body, Supplier<Packet<?>> packet) {}

    static void lines(long seed, int cases, Consumer<String> sink) {
        try {
            run(seed, cases, sink);
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    private static void run(long seed, int cases, Consumer<String> sink) throws Exception {
        Harness.init();
        for (int c = 0; c < cases; c++) {
            Rng rng = new Rng(seed * 1_000_003L + c ^ 0x696e74657270L);
            Scenario sc = new Scenario("interp").spawn(0.5, 65, 0.5, 0, 0);
            Harness.Run run = new Harness.Run(sc);
            run.player.setEntityId(1);
            client().interactionManager.setGameMode(LevelInfo.GameMode.SURVIVAL);
            @SuppressWarnings("unchecked")
            Map<UUID, PlayerListEntry> tab =
                    (Map<UUID, PlayerListEntry>) Reflect.get(ClientPlayNetworkHandler.class, run.net, "playerListEntries");
            Json j = new Json().beginObject().field("type", "interp_case").field("case", c);
            j.key("ticks").beginArray();
            List<Integer> ids = new ArrayList<>();
            for (int t = 0; t < TICKS; t++) {
                List<Sent> packets = new ArrayList<>();
                if (t == 0) {
                    int n = 0;
                    for (int p = 0; p < 2; p++) {
                        int id = 100 + n++;
                        UUID uuid = new UUID(0x696e74L, id);
                        PlayerListS2CPacket list = new PlayerListS2CPacket();
                        tab.put(uuid, new PlayerListEntry(list.new Entry(new GameProfile(uuid, "P" + id), 0,
                                LevelInfo.GameMode.SURVIVAL, null)));
                        PacketByteBuf b = buf();
                        b.writeVarInt(id);
                        b.writeUUID(uuid);
                        fixed(b, rng);
                        b.writeByte(rng.range(-128, 127)).writeByte(rng.range(-128, 127));
                        b.writeShort(0);
                        // The flags byte (an empty list would leave the client's packet without
                        // entries: servers always send some).
                        b.writeByte(0).writeByte(0).writeByte(0x7f);
                        packets.add(new Sent(SPAWN_PLAYER, bytes(b), PlayerSpawnS2CPacket::new));
                        ids.add(id);
                    }
                    for (int m = 0; m < 2; m++) {
                        int id = 100 + n++;
                        PacketByteBuf b = buf();
                        b.writeVarInt(id);
                        b.writeByte(54); // zombie
                        fixed(b, rng);
                        b.writeByte(rng.range(-128, 127)).writeByte(rng.range(-128, 127)).writeByte(0);
                        b.writeShort(0).writeShort(0).writeShort(0);
                        b.writeByte(0).writeByte(0).writeByte(0x7f);
                        packets.add(new Sent(SPAWN_MOB, bytes(b), MobSpawnS2CPacket::new));
                        ids.add(id);
                    }
                    int id = 100 + n;
                    PacketByteBuf b = buf();
                    b.writeVarInt(id);
                    b.writeByte(78); // armor stand
                    fixed(b, rng);
                    b.writeByte(0).writeByte(rng.range(-128, 127));
                    b.writeInt(0);
                    packets.add(new Sent(SPAWN_OBJECT, bytes(b), EntitySpawnS2CPacket::new));
                    ids.add(id);
                } else {
                    for (int id : ids) {
                        int events = rng.chance(0.15) ? 2 : rng.chance(0.55) ? 1 : 0;
                        for (int k = 0; k < events; k++) {
                            packets.add(event(run, rng, id));
                        }
                    }
                }
                j.beginObject();
                j.key("packets").beginArray();
                for (Sent s : packets) {
                    j.beginArray().value(s.id()).value(hex(s.body())).endArray();
                    PacketByteBuf b = new PacketByteBuf(Unpooled.wrappedBuffer(s.body()));
                    handle(run, s.packet().get(), b);
                }
                j.endArray();
                run.world.tickEntities();
                j.key("entities").beginArray();
                for (int id : ids) {
                    Entity e = run.world.getEntityById(id);
                    j.beginArray().value(id).f64(e.x).f64(e.y).f64(e.z).f32(e.yaw).f32(e.pitch).endArray();
                }
                j.endArray().endObject();
            }
            j.endArray();
            sink.accept(j.endObject().toString());
        }
    }

    private static MinecraftClient client() {
        return MinecraftClient.getInstance();
    }

    /** A spawn position in 1/32 blocks around the player. */
    private static void fixed(PacketByteBuf b, Rng rng) {
        b.writeInt(rng.range(-160, 160)).writeInt(64 * 32 + rng.range(-16, 32)).writeInt(rng.range(-160, 160));
    }

    /** One random movement packet for entity {@code id}. */
    private static Sent event(Harness.Run run, Rng rng, int id) {
        PacketByteBuf b = buf();
        b.writeVarInt(id);
        int kind = rng.nextInt(100);
        int big = rng.chance(0.2) ? 127 : 24;
        if (kind < 30) {
            b.writeByte(rng.range(-big, big)).writeByte(rng.range(-8, 8)).writeByte(rng.range(-big, big));
            b.writeBoolean(rng.chance(0.5));
            return new Sent(REL_MOVE, bytes(b), EntityS2CPacket.MoveRelative::new);
        }
        if (kind < 45) {
            b.writeByte(rng.range(-128, 127)).writeByte(rng.range(-64, 64)).writeBoolean(rng.chance(0.5));
            return new Sent(LOOK, bytes(b), EntityS2CPacket.Rotate::new);
        }
        if (kind < 65) {
            b.writeByte(rng.range(-big, big)).writeByte(rng.range(-8, 8)).writeByte(rng.range(-big, big));
            b.writeByte(rng.range(-128, 127)).writeByte(rng.range(-64, 64)).writeBoolean(rng.chance(0.5));
            return new Sent(MOVE_LOOK, bytes(b), EntityS2CPacket.RotateAndMoveRelative::new);
        }
        if (kind < 72) {
            return new Sent(ENTITY, bytes(b), EntityS2CPacket::new);
        }
        // Teleport: near the drawn position (the 1/32 rule keeps it) or anywhere around.
        Entity e = run.world.getEntityById(id);
        int x, y, z;
        if (kind < 86) {
            x = (int) Math.floor(e.x * 32.0) + rng.range(-1, 1);
            y = (int) Math.floor(e.y * 32.0) + rng.range(-1, 1);
            z = (int) Math.floor(e.z * 32.0) + rng.range(-1, 1);
        } else {
            x = rng.range(-320, 320);
            y = 64 * 32 + rng.range(-32, 64);
            z = rng.range(-320, 320);
        }
        b.writeInt(x).writeInt(y).writeInt(z);
        b.writeByte(rng.range(-128, 127)).writeByte(rng.range(-64, 64)).writeBoolean(rng.chance(0.5));
        return new Sent(TELEPORT, bytes(b), EntityPositionS2CPacket::new);
    }

    @SuppressWarnings("unchecked")
    private static void handle(Harness.Run run, Packet<?> p, PacketByteBuf b) {
        Object thread = Reflect.get(MinecraftClient.class, client(), "currentThread");
        Reflect.set(MinecraftClient.class, client(), "currentThread", Thread.currentThread());
        try {
            p.read(b);
            if (b.readableBytes() != 0) {
                throw new IllegalStateException(p.getClass().getSimpleName() + ": " + b.readableBytes() + " bytes left");
            }
            ((Packet<ClientPlayPacketListener>) p).apply(run.net);
        } catch (java.io.IOException ex) {
            throw new IllegalStateException(ex);
        } finally {
            Reflect.set(MinecraftClient.class, client(), "currentThread", thread);
        }
    }

    private static PacketByteBuf buf() {
        return new PacketByteBuf(Unpooled.buffer());
    }

    private static byte[] bytes(PacketByteBuf b) {
        byte[] out = new byte[b.readableBytes()];
        b.getBytes(b.readerIndex(), out);
        return out;
    }

    private static String hex(byte[] a) {
        StringBuilder sb = new StringBuilder(a.length * 2);
        for (byte x : a) {
            sb.append(Character.forDigit((x >> 4) & 15, 16)).append(Character.forDigit(x & 15, 16));
        }
        return sb.toString();
    }
}
