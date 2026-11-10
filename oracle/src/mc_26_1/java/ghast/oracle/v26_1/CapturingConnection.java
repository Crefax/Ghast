package ghast.oracle.v26_1;

import io.netty.channel.ChannelFutureListener;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.PacketFlow;

import java.util.ArrayList;
import java.util.List;

/**
 * The real client connection object with no channel: every packet the client sends (through the
 * packet listener or directly, like the teleport confirmation) is captured in order.
 */
final class CapturingConnection extends Connection {
    final List<Packet<?>> sent = new ArrayList<>();

    CapturingConnection() {
        super(PacketFlow.CLIENTBOUND);
    }

    @Override
    public void send(Packet<?> packet) {
        sent.add(packet);
    }

    @Override
    public void send(Packet<?> packet, ChannelFutureListener listener) {
        sent.add(packet);
    }

    @Override
    public void send(Packet<?> packet, ChannelFutureListener listener, boolean flush) {
        sent.add(packet);
    }

    @Override
    public boolean isConnected() {
        return true;
    }

    @Override
    public boolean isMemoryConnection() {
        return false;
    }

    @Override
    public void tick() {
    }
}
