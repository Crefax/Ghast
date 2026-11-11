package ghast.oracle.v26_1;

import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.tree.CommandNode;
import com.mojang.brigadier.tree.RootCommandNode;
import ghast.oracle.Json;
import ghast.oracle.Rng;
import io.netty.buffer.Unpooled;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.ClientSuggestionProvider;
import net.minecraft.commands.CommandBuildContext;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.commands.arguments.ArgumentSignatures;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.chat.LastSeenMessagesTracker;
import net.minecraft.network.chat.LocalChatSession;
import net.minecraft.network.chat.MessageSignature;
import net.minecraft.network.chat.SignableCommand;
import net.minecraft.network.chat.SignedMessageBody;
import net.minecraft.network.chat.SignedMessageChain;
import net.minecraft.network.protocol.game.ClientboundCommandsPacket;
import net.minecraft.network.protocol.game.ClientboundEntityEventPacket;
import net.minecraft.network.protocol.game.ServerboundChatAckPacket;
import net.minecraft.network.protocol.game.ServerboundChatCommandPacket;
import net.minecraft.network.protocol.game.ServerboundChatCommandSignedPacket;
import net.minecraft.network.protocol.game.ServerboundChatPacket;
import net.minecraft.network.protocol.game.ServerboundChatSessionUpdatePacket;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.world.entity.player.ProfileKeyPair;
import net.minecraft.world.entity.player.ProfilePublicKey;
import net.minecraft.world.flag.FeatureFlags;

import java.lang.reflect.Field;
import java.nio.ByteBuffer;
import java.security.KeyPair;
import java.security.KeyPairGenerator;
import java.security.SecureRandom;
import java.security.Signature;
import java.time.Instant;
import java.util.ArrayList;
import java.util.Base64;
import java.util.HashMap;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.function.Consumer;

/**
 * 26.1 signed chat vectors, made with the real client classes and exactly the calls of
 * {@code ClientPacketListener}: {@code setKeyPair} ({@code LocalChatSession} → {@code chat_session_update}),
 * {@code sendChat} ({@code LastSeenMessagesTracker.generateAndApplyUpdate}, the session's
 * {@code SignedMessageChain} encoder over a {@code SignedMessageBody}, {@code ServerboundChatPacket}),
 * {@code sendCommand} ({@code SignableCommand.of} on the client's command dispatcher built from a
 * {@code ClientboundCommandsPacket} by {@code handleCommands}, {@code ArgumentSignatures.signCommand},
 * {@code ServerboundChatCommandSignedPacket} or {@code ServerboundChatCommandPacket}) and
 * {@code markMessageAsProcessed} ({@code addPending}, the acknowledgement after 64).
 * {@code Instant.now()} and the salt are the only inputs that differ from the client: they are
 * chosen by the dump. Schema: oracle/README.md, "chat_signing.jsonl.gz (26.1)".
 *
 * <p>The command tree is the vanilla dedicated server's ({@code Commands(DEDICATED)}) as sent to an
 * operator (permission level 4) by {@code Commands.sendCommands}.
 */
final class ChatSigningDump {
    static final int CHAINS = 14;

    private ChatSigningDump() {}

    /** Commands tried in every chain (signable or not, valid or not). */
    private static final String[] COMMANDS = {
            "msg Steve hello there", "tell Alex hi", "w Bob   spaced  text ", "say hello world", "me waves",
            "teammsg for the team", "tm short", "msg @a[distance=..5] to everyone near", "msg @p hi",
            "execute as @a run say hi", "execute at @p positioned ~ ~1 ~ run me jumps",
            "execute as Steve run msg Bob nested", "execute run execute run say deep",
            "execute if entity @e[type=minecraft:zombie,limit=1] run say zombie",
            "execute positioned 1 2 3 rotated 90 0 run tell Alex rotated",
            "say a \"quoted\" b", "say ünicöde ☃", "msg", "say", "msg Steve",
            "gamemode creative", "tp 1 2 3", "give @s minecraft:stone 64", "tellraw @a \"x\"",
            "warp spawn", "spawn", "list", "help 2", "minecraft:say namespaced", "trigger x",
            "execute as @a at @s run say via root", "execute as Steve", "msg @a[name=\"a b\",limit=2] quoted selector",
            "msg @e[type=minecraft:pig] pigs", "teammsg @p not a selector here", "me", "w  Bob double space"};

    private static final String[] CHATS = {
            "hello", "a", "çığşöü", "emoji 😀 test", " leading and trailing ",
            "x".repeat(256), "with \"quotes\" and \\ backslash", "1234567890", "/not a command (starts with space)"};

    static void lines(Consumer<String> sink) {
        try {
            HexFormat hex = HexFormat.of();
            Base64.Encoder b64 = Base64.getEncoder();
            Rng rng = new Rng(0x636861747369676eL);

            // The profile key pair (2048 bits like Mojang's) and a "services" key that signs it.
            KeyPair profileKeys = rsa(2048, 7001);
            KeyPair servicesKeys = rsa(2048, 7002);
            UUID profileId = new UUID(rng.nextLong(), rng.nextLong());
            UUID sessionId = new UUID(rng.nextLong(), rng.nextLong());
            Instant expiresAt = Instant.ofEpochMilli(1_800_000_000_123L);
            byte[] keyBytes = profileKeys.getPublic().getEncoded();
            ByteBuffer payload = ByteBuffer.allocate(24 + keyBytes.length);
            payload.putLong(profileId.getMostSignificantBits()).putLong(profileId.getLeastSignificantBits())
                    .putLong(expiresAt.toEpochMilli()).put(keyBytes);
            Signature services = Signature.getInstance("SHA1withRSA");
            services.initSign(servicesKeys.getPrivate());
            services.update(payload.array());
            byte[] keySignature = services.sign();
            ProfilePublicKey publicKey = new ProfilePublicKey(new ProfilePublicKey.Data(expiresAt, profileKeys.getPublic(), keySignature));
            ProfileKeyPair keyPair = new ProfileKeyPair(profileKeys.getPrivate(), publicKey, expiresAt.minusSeconds(3600));
            LocalChatSession session = new LocalChatSession(sessionId, keyPair);
            FriendlyByteBuf update = buf();
            ServerboundChatSessionUpdatePacket.STREAM_CODEC.encode(update, new ServerboundChatSessionUpdatePacket(session.asRemote().asData()));
            sink.accept(new Json().beginObject().field("type", "session")
                    .field("profile_id", profileId.toString()).field("session_id", sessionId.toString())
                    .field("private_pkcs8", b64.encodeToString(profileKeys.getPrivate().getEncoded()))
                    .field("public_spki", b64.encodeToString(keyBytes))
                    .field("expires_at_ms", expiresAt.toEpochMilli())
                    .field("key_signature", b64.encodeToString(keySignature))
                    .field("services_public_spki", b64.encodeToString(servicesKeys.getPublic().getEncoded()))
                    .field("session_update_body", hex.formatHex(bytes(update)))
                    .endObject().toString());

            // The command tree an operator receives, and the client dispatcher made from it.
            Harness.Run run = new Harness.Run(new ghast.oracle.Scenario("chat_signing").spawn(0.5, 100, 0.5, 0, 0));
            byte[] tree = commandsPacket();
            sink.accept(new Json().beginObject().field("type", "commands").field("body", hex.formatHex(tree)).endObject().toString());
            CommandDispatcher<ClientSuggestionProvider> dispatcher = clientDispatcher(tree);
            ClientSuggestionProvider source = Harness.listener.getSuggestionsProvider();

            List<byte[]> pool = new ArrayList<>();
            for (int i = 0; i < 40; i++) {
                pool.add(random(rng, 256));
            }
            long clock = 1_790_000_000_000L;
            for (int chain = 0; chain < CHAINS; chain++) {
                boolean signed = chain % 2 == 0;
                // The player's permission level (entity event 24 + level): selectors need 2.
                int opLevel = chain % 4 < 2 ? 4 : 0;
                Harness.listener.handleEntityEvent(new ClientboundEntityEventPacket(Harness.client.player, (byte) (24 + opLevel)));
                SignedMessageChain.Encoder encoder = signed ? session.createMessageEncoder(profileId) : SignedMessageChain.Encoder.UNSIGNED;
                LastSeenMessagesTracker tracker = new LastSeenMessagesTracker(20);
                int events = chain == 1 || chain == 2 ? 160 : 40 + rng.nextInt(40);
                Json j = new Json().beginObject().field("type", "chain").field("index", chain).field("signed", signed).field("op_level", opLevel);
                j.key("events").beginArray();
                byte[] last = null;
                for (int e = 0; e < events; e++) {
                    int pick = rng.nextInt(10);
                    if (chain == 1 || chain == 2) {
                        pick = e % 40 == 39 ? 9 : 0; // long runs of received messages: acknowledgements
                    }
                    j.beginObject();
                    if (pick < 5) {
                        // markMessageAsProcessed(signature, wasShown)
                        byte[] sig = last != null && rng.nextInt(8) == 0 ? last : pool.get(rng.nextInt(pool.size()));
                        last = sig;
                        boolean shown = rng.nextInt(10) != 0;
                        j.field("recv", b64.encodeToString(sig)).field("shown", shown);
                        String ack = null;
                        if (tracker.addPending(new MessageSignature(sig), shown) && tracker.offset() > 64) {
                            int offset = tracker.getAndClearOffset();
                            if (offset > 0) {
                                FriendlyByteBuf b = buf();
                                ServerboundChatAckPacket.STREAM_CODEC.encode(b, new ServerboundChatAckPacket(offset));
                                ack = hex.formatHex(bytes(b));
                            }
                        }
                        if (ack != null) {
                            j.field("ack_body", ack);
                        }
                    } else if (pick < 7) {
                        String content = CHATS[rng.nextInt(CHATS.length)];
                        clock += 1 + rng.nextInt(5000);
                        Instant ts = Instant.ofEpochMilli(clock);
                        long salt = rng.nextLong();
                        LastSeenMessagesTracker.Update u = tracker.generateAndApplyUpdate();
                        MessageSignature signature = encoder.pack(new SignedMessageBody(content, ts, salt, u.lastSeen()));
                        FriendlyByteBuf b = buf();
                        ServerboundChatPacket.STREAM_CODEC.encode(b, new ServerboundChatPacket(content, ts, salt, signature, u.update()));
                        j.field("chat", content).field("timestamp_ms", clock).field("salt", salt)
                                .field("body", hex.formatHex(bytes(b)));
                    } else {
                        String command = COMMANDS[rng.nextInt(COMMANDS.length)];
                        clock += 1 + rng.nextInt(5000);
                        j.field("command", command);
                        SignableCommand<ClientSuggestionProvider> signable = SignableCommand.of(dispatcher.parse(command, source));
                        j.key("signable").beginArray();
                        for (SignableCommand.Argument<ClientSuggestionProvider> a : signable.arguments()) {
                            j.beginArray().value(a.name()).value(a.value()).endArray();
                        }
                        j.endArray();
                        FriendlyByteBuf b = buf();
                        if (signable.arguments().isEmpty()) {
                            ServerboundChatCommandPacket.STREAM_CODEC.encode(b, new ServerboundChatCommandPacket(command));
                            j.field("packet", "chat_command");
                        } else {
                            Instant ts = Instant.ofEpochMilli(clock);
                            long salt = rng.nextLong();
                            LastSeenMessagesTracker.Update u = tracker.generateAndApplyUpdate();
                            ArgumentSignatures sigs = ArgumentSignatures.signCommand(signable,
                                    argument -> encoder.pack(new SignedMessageBody(argument, ts, salt, u.lastSeen())));
                            ServerboundChatCommandSignedPacket.STREAM_CODEC.encode(b,
                                    new ServerboundChatCommandSignedPacket(command, ts, salt, sigs, u.update()));
                            j.field("packet", "chat_command_signed").field("timestamp_ms", clock).field("salt", salt);
                        }
                        j.field("body", hex.formatHex(bytes(b)));
                    }
                    j.endObject();
                }
                j.endArray();
                sink.accept(j.endObject().toString());
            }

        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    /** {@code Commands.sendCommands} for an operator: the usable nodes of the dedicated server's tree. */
    @SuppressWarnings({"unchecked", "rawtypes"})
    private static byte[] commandsPacket() throws Exception {
        CommandBuildContext context = CommandBuildContext.simple(Harness.registries, FeatureFlags.DEFAULT_FLAGS);
        Commands commands = new Commands(Commands.CommandSelection.DEDICATED, context);
        CommandSourceStack op = Commands.createCompilationContext(LevelBasedPermissionSet.OWNER);
        Map<CommandNode<CommandSourceStack>, CommandNode<CommandSourceStack>> converted = new HashMap<>();
        RootCommandNode<CommandSourceStack> root = new RootCommandNode<>();
        converted.put(commands.getDispatcher().getRoot(), root);
        fill(commands.getDispatcher().getRoot(), root, op, converted);
        Field inspector = Commands.class.getDeclaredField("COMMAND_NODE_INSPECTOR");
        inspector.setAccessible(true);
        FriendlyByteBuf b = buf();
        ClientboundCommandsPacket.STREAM_CODEC.encode(b,
                new ClientboundCommandsPacket(root, (ClientboundCommandsPacket.NodeInspector) inspector.get(null)));
        return bytes(b);
    }

    /**
     * {@code Commands.fillUsableCommands}, with each node's children in name order: some subtrees
     * (the game rules) come from hash-ordered registries, so their order differs between runs. The
     * order does not change what a parse reads (literals are looked up by name).
     */
    private static <S> void fill(CommandNode<S> source, CommandNode<S> target, S filter,
            Map<CommandNode<S>, CommandNode<S>> converted) {
        List<CommandNode<S>> children = new ArrayList<>(source.getChildren());
        children.sort(java.util.Comparator.comparing(CommandNode::getName));
        for (CommandNode<S> child : children) {
            if (child.canUse(filter)) {
                var builder = child.createBuilder();
                if (builder.getRedirect() != null) {
                    CommandNode<S> to = converted.get(builder.getRedirect());
                    if (to == null) {
                        throw new IllegalStateException("redirect of " + child.getName() + " before its target");
                    }
                    builder.redirect(to);
                }
                CommandNode<S> node = builder.build();
                converted.put(child, node);
                target.addChild(node);
                if (!child.getChildren().isEmpty()) {
                    fill(child, node, filter, converted);
                }
            }
        }
    }

    /** {@code ClientPacketListener.handleCommands}. */
    @SuppressWarnings({"unchecked", "rawtypes"})
    private static CommandDispatcher<ClientSuggestionProvider> clientDispatcher(byte[] tree) throws Exception {
        ClientboundCommandsPacket packet = ClientboundCommandsPacket.STREAM_CODEC.decode(new FriendlyByteBuf(Unpooled.wrappedBuffer(tree)));
        Field builder = ClientPacketListener.class.getDeclaredField("COMMAND_NODE_BUILDER");
        builder.setAccessible(true);
        CommandBuildContext context = CommandBuildContext.simple(Harness.listener.registryAccess(), FeatureFlags.DEFAULT_FLAGS);
        return new CommandDispatcher<>(packet.getRoot(context, (ClientboundCommandsPacket.NodeBuilder) builder.get(null)));
    }

    private static KeyPair rsa(int bits, long seed) throws Exception {
        SecureRandom r = SecureRandom.getInstance("SHA1PRNG");
        r.setSeed(seed);
        KeyPairGenerator gen = KeyPairGenerator.getInstance("RSA");
        gen.initialize(bits, r);
        return gen.generateKeyPair();
    }

    private static byte[] random(Rng rng, int n) {
        byte[] b = new byte[n];
        for (int k = 0; k < n; k++) {
            b[k] = (byte) rng.nextLong();
        }
        return b;
    }

    private static FriendlyByteBuf buf() {
        return new FriendlyByteBuf(Unpooled.buffer());
    }

    private static byte[] bytes(FriendlyByteBuf buf) {
        byte[] b = new byte[buf.readableBytes()];
        buf.readBytes(b);
        return b;
    }
}
