package ghast.oracle.modern;

import ghast.oracle.Json;
import ghast.oracle.Rng;
import io.netty.buffer.Unpooled;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.protocol.login.ClientboundHelloPacket;
import net.minecraft.network.protocol.login.ServerboundKeyPacket;
import net.minecraft.util.Crypt;

import java.math.BigInteger;
import java.security.KeyPair;
import java.security.KeyPairGenerator;
import java.security.SecureRandom;
import java.util.Base64;
import java.util.HexFormat;
import java.util.function.Consumer;
import javax.crypto.Cipher;
import javax.crypto.SecretKey;
import javax.crypto.spec.SecretKeySpec;

/**
 * 26.1 login encryption vectors, computed with the real classes the client uses in
 * {@code ClientHandshakePacketListenerImpl.handleHello}: the session-server hash
 * ({@code new BigInteger(Crypt.digestData(serverId, publicKey, secretKey)).toString(16)}), the
 * AES/CFB8 stream ciphers of both directions ({@code Crypt.getCipher}), the {@code ClientboundHelloPacket}
 * wire form (with {@code shouldAuthenticate}) and a {@code ServerboundKeyPacket} built by the real
 * constructor (checked to decrypt to the secret and challenge; the vector's key packet uses the same RSA
 * cipher with seeded padding, through the packet codec, and carries the server's private key so the reader
 * can decrypt it). Schema: oracle/README.md, "net_crypto.jsonl.gz (26.1)".
 *
 * <p>Deterministic: server keys and the key packet's padding come from seeded {@code SHA1PRNG}s,
 * everything else from {@link Rng}.
 */
final class NetCryptoDump {
    static final int CASES = 12;

    private NetCryptoDump() {}

    static void lines(Consumer<String> sink) {
        try {
            Rng rng = new Rng(0x6e657463727970L);
            HexFormat hex = HexFormat.of();
            Base64.Encoder b64 = Base64.getEncoder();
            for (int i = 0; i < CASES; i++) {
                SecureRandom keyRandom = SecureRandom.getInstance("SHA1PRNG");
                keyRandom.setSeed(1000L + i);
                KeyPairGenerator gen = KeyPairGenerator.getInstance("RSA");
                // Crypt.generateKeyPair uses 1024 bits (the vanilla server key); a few cases use 2048.
                gen.initialize(i % 6 == 5 ? 2048 : 1024, keyRandom);
                KeyPair server = gen.generateKeyPair();

                byte[] secretBytes = new byte[16];
                for (int k = 0; k < 16; k++) {
                    secretBytes[k] = (byte) rng.nextLong();
                }
                SecretKey secret = new SecretKeySpec(secretBytes, "AES");
                String serverId = switch (i % 4) {
                    case 0 -> "";
                    case 1 -> Long.toHexString(rng.nextLong()).substring(0, 8);
                    case 2 -> "éèÿ" + i;
                    default -> "server" + (rng.nextInt(1_000_000));
                };
                byte[] challenge = new byte[4];
                for (int k = 0; k < 4; k++) {
                    challenge[k] = (byte) rng.nextLong();
                }
                boolean authenticate = i % 3 != 2;

                String digest = new BigInteger(Crypt.digestData(serverId, server.getPublic(), secret)).toString(16);

                FriendlyByteBuf hello = new FriendlyByteBuf(Unpooled.buffer());
                ClientboundHelloPacket.STREAM_CODEC.encode(hello,
                        new ClientboundHelloPacket(serverId, server.getPublic().getEncoded(), challenge, authenticate));

                // The real constructor (Crypt.encryptUsingKey: RSA PKCS#1 v1.5, random padding) must
                // decrypt to the secret and the challenge ...
                ServerboundKeyPacket real = new ServerboundKeyPacket(secret, server.getPublic(), challenge);
                if (!java.util.Arrays.equals(real.getSecretKey(server.getPrivate()).getEncoded(), secretBytes)
                        || !real.isChallengeValid(challenge, server.getPrivate())) {
                    throw new IllegalStateException("key packet does not decrypt");
                }
                // ... and the vector carries a deterministic one: the same cipher with seeded padding,
                // through the packet's own codec.
                SecureRandom pad = SecureRandom.getInstance("SHA1PRNG");
                pad.setSeed(5000L + i);
                Cipher rsa = Cipher.getInstance("RSA");
                rsa.init(Cipher.ENCRYPT_MODE, server.getPublic(), pad);
                FriendlyByteBuf raw = new FriendlyByteBuf(Unpooled.buffer());
                raw.writeByteArray(rsa.doFinal(secretBytes));
                raw.writeByteArray(rsa.doFinal(challenge));
                FriendlyByteBuf key = new FriendlyByteBuf(Unpooled.buffer());
                ServerboundKeyPacket.STREAM_CODEC.encode(key, ServerboundKeyPacket.STREAM_CODEC.decode(raw));

                // Stream ciphers: a few chunks per direction, one cipher object each (the stream continues).
                Cipher encrypt = Crypt.getCipher(Cipher.ENCRYPT_MODE, secret);
                Cipher decrypt = Crypt.getCipher(Cipher.DECRYPT_MODE, secret);
                Json j = new Json().beginObject().field("type", "case").field("index", i)
                        .field("server_id", serverId)
                        .field("should_authenticate", authenticate)
                        .field("server_public_der", b64.encodeToString(server.getPublic().getEncoded()))
                        .field("server_private_pkcs8", b64.encodeToString(server.getPrivate().getEncoded()))
                        .field("shared_secret", hex.formatHex(secretBytes))
                        .field("challenge", hex.formatHex(challenge))
                        .field("server_hash", digest)
                        .field("hello_body", hex.formatHex(bytes(hello)))
                        .field("key_body", hex.formatHex(bytes(key)));
                j.key("encrypt").beginArray();
                for (int c = 0; c < 4; c++) {
                    byte[] plain = random(rng, c == 0 ? 1 : rng.nextInt(300) + 1);
                    byte[] out = encrypt.update(plain);
                    j.beginArray().value(hex.formatHex(plain)).value(hex.formatHex(out)).endArray();
                }
                j.endArray();
                j.key("decrypt").beginArray();
                for (int c = 0; c < 4; c++) {
                    byte[] wire = random(rng, rng.nextInt(300) + 1);
                    byte[] out = decrypt.update(wire);
                    j.beginArray().value(hex.formatHex(wire)).value(hex.formatHex(out)).endArray();
                }
                j.endArray();
                sink.accept(j.endObject().toString());
            }
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    private static byte[] random(Rng rng, int n) {
        byte[] b = new byte[n];
        for (int k = 0; k < n; k++) {
            b[k] = (byte) rng.nextLong();
        }
        return b;
    }

    private static byte[] bytes(FriendlyByteBuf buf) {
        byte[] b = new byte[buf.readableBytes()];
        buf.readBytes(b);
        return b;
    }
}
