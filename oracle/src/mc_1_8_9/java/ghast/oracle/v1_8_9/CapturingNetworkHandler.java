package ghast.oracle.v1_8_9;

import com.mojang.authlib.GameProfile;
import net.minecraft.client.MinecraftClient;
import net.minecraft.client.network.ClientPlayNetworkHandler;
import net.minecraft.network.Packet;

import java.util.ArrayList;
import java.util.List;

/**
 * The real client network handler with the connection cut off: every packet the client would send is
 * captured in order instead. Only {@code sendPacket} is overridden; everything else is vanilla.
 */
final class CapturingNetworkHandler extends ClientPlayNetworkHandler {
    final List<Packet<?>> sent = new ArrayList<>();

    CapturingNetworkHandler(MinecraftClient client, GameProfile profile) {
        super(client, null, null, profile);
    }

    @Override
    public void sendPacket(Packet packet) {
        sent.add(packet);
    }
}
