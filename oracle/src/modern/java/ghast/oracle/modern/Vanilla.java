package ghast.oracle.modern;

import com.mojang.datafixers.util.Pair;
import io.netty.buffer.Unpooled;
import net.minecraft.client.multiplayer.ClientRegistryLayer;
import net.minecraft.client.multiplayer.RegistryDataCollector;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.LayeredRegistryAccess;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.RegistrySynchronization;
import net.minecraft.nbt.NbtOps;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.protocol.common.ClientboundUpdateTagsPacket;
import net.minecraft.network.protocol.configuration.ClientboundRegistryDataPacket;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.WorldLoader;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.CloseableResourceManager;
import net.minecraft.server.packs.resources.ResourceProvider;
import net.minecraft.tags.TagLoader;
import net.minecraft.tags.TagNetworkSerialization;
import net.minecraft.world.level.WorldDataConfiguration;

import java.util.List;
import java.util.Set;
import java.util.stream.Stream;

/**
 * The data a vanilla client receives during the configuration phase, produced the way a vanilla server
 * produces it and consumed the way the client consumes it.
 *
 * <p>Server side: the vanilla data pack inside the jar ({@code ServerPacksSource}'s vanilla pack, the
 * same one a dedicated or integrated server selects by default) is loaded with the server's own loaders,
 * in {@code WorldLoader.load}'s order: static-registry tags, the worldgen registries, the dimension
 * registries. (The reloadable server resources - recipes, loot tables, advancements, functions - are not
 * loaded: they are not part of the configuration phase.)
 *
 * <p>Wire: {@code RegistrySynchronization.packRegistries} (the server's registry sync task, with no
 * packs known to the client, so every element is sent in full) and
 * {@code TagNetworkSerialization.serializeTagsToNetwork}, each encoded into real
 * {@code ClientboundRegistryDataPacket} / {@code ClientboundUpdateTagsPacket} bytes and decoded again.
 *
 * <p>Client side: {@code RegistryDataCollector} turns them into the client's registry access exactly as
 * {@code ClientConfigurationPacketListenerImpl.handleConfigurationFinished} does for a remote
 * (non-memory) connection, which also binds the received tags of the static registries (blocks, fluids,
 * items, entity types) - the climbable / fluid / soul speed tags the movement code reads.
 */
final class Vanilla {
    private Vanilla() {}

    /** The client's registry access after the configuration phase. */
    static RegistryAccess.Frozen configure() {
        LayeredRegistryAccess<RegistryLayer> server = loadServerRegistries();

        RegistryDataCollector collector = new RegistryDataCollector();
        RegistryAccess.Frozen serverAccess = server.compositeAccess();
        RegistrySynchronization.packRegistries(serverAccess.createSerializationContext(NbtOps.INSTANCE), serverAccess, Set.of(),
                (key, entries) -> {
                    ClientboundRegistryDataPacket packet = roundTrip(ClientboundRegistryDataPacket.STREAM_CODEC,
                            new ClientboundRegistryDataPacket(key, entries));
                    collector.appendContents(packet.registry(), packet.entries());
                });
        ClientboundUpdateTagsPacket tags = roundTrip(ClientboundUpdateTagsPacket.STREAM_CODEC,
                new ClientboundUpdateTagsPacket(TagNetworkSerialization.serializeTagsToNetwork(server)));
        collector.appendTags(tags.getTags());
        RegistryAccess.Frozen base = ClientRegistryLayer.createRegistryAccess().compositeAccess();
        return collector.collectGameRegistries(ResourceProvider.EMPTY, base, false);
    }

    private static LayeredRegistryAccess<RegistryLayer> loadServerRegistries() {
        PackRepository packs = ServerPacksSource.createVanillaTrustedRepository();
        WorldLoader.PackConfig packConfig = new WorldLoader.PackConfig(packs, WorldDataConfiguration.DEFAULT, false, true);
        Pair<WorldDataConfiguration, CloseableResourceManager> opened = packConfig.createResourceManager();
        try (CloseableResourceManager resources = opened.getSecond()) {
            LayeredRegistryAccess<RegistryLayer> layers = RegistryLayer.createRegistryAccess();
            List<Registry.PendingTags<?>> staticTags = TagLoader.loadTagsForExistingRegistries(resources,
                    layers.getLayer(RegistryLayer.STATIC));
            RegistryAccess.Frozen worldgenContext = layers.getAccessForLoading(RegistryLayer.WORLDGEN);
            List<HolderLookup.RegistryLookup<?>> worldgenLookups = TagLoader.buildUpdatedLookups(worldgenContext, staticTags);
            RegistryAccess.Frozen worldgen = RegistryDataLoader.load(resources, worldgenLookups,
                    RegistryDataLoader.WORLDGEN_REGISTRIES, Runnable::run).join();
            List<HolderLookup.RegistryLookup<?>> dimensionLookups = Stream.concat(worldgenLookups.stream(),
                    worldgen.listRegistries()).toList();
            RegistryAccess.Frozen dimensions = RegistryDataLoader.load(resources, dimensionLookups,
                    RegistryDataLoader.DIMENSION_REGISTRIES, Runnable::run).join();
            layers = layers.replaceFrom(RegistryLayer.WORLDGEN, worldgen, dimensions);
            // ReloadableServerResources.updateComponentsAndStaticRegistryTags: the server binds the static
            // tags before it serialises them for the client.
            staticTags.forEach(Registry.PendingTags::apply);
            return layers;
        }
    }

    /** Encodes a packet with its stream codec and decodes the bytes again (all bytes must be read). */
    static <P> P roundTrip(net.minecraft.network.codec.StreamCodec<? super FriendlyByteBuf, P> codec, P packet) {
        FriendlyByteBuf buf = new FriendlyByteBuf(Unpooled.buffer());
        codec.encode(buf, packet);
        P back = codec.decode(buf);
        if (buf.readableBytes() != 0) {
            throw new IllegalStateException("unread bytes decoding " + packet.getClass().getSimpleName());
        }
        return back;
    }
}
