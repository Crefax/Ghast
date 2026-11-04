package ghast.oracle.v1_16_5;

import io.netty.util.concurrent.Future;
import io.netty.util.concurrent.GenericFutureListener;
import net.minecraft.network.ClientConnection;
import net.minecraft.network.NetworkSide;
import net.minecraft.network.Packet;

import java.util.ArrayList;
import java.util.List;

/**
 * The real client connection object with no channel: every packet the client sends (through the
 * network handler or directly, like the teleport confirmation) is captured in order.
 */
final class CapturingConnection extends ClientConnection {
    final List<Packet<?>> sent = new ArrayList<>();

    CapturingConnection() {
        super(NetworkSide.CLIENTBOUND);
    }

    @Override
    public void send(Packet<?> packet) {
        sent.add(packet);
    }

    @Override
    public void send(Packet<?> packet, GenericFutureListener<? extends Future<? super Void>> callback) {
        sent.add(packet);
    }

    @Override
    public boolean isOpen() {
        return true;
    }
}
