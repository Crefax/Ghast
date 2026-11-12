package ghast.oracle.v26_1;

import ghast.oracle.Json;
import ghast.oracle.Rng;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.level.biome.Biome;

import java.lang.reflect.Method;
import java.util.function.Consumer;

/**
 * 26.1 biome climate vectors: every biome of the client's synced {@code minecraft:worldgen/biome}
 * registry with its climate settings (precipitation flag, base temperature, temperature
 * modifier), and random positions with {@code Biome.getPrecipitationAt(pos, seaLevel)} and the
 * height-adjusted temperature the real classes compute (the temperature noise, the frozen-ocean
 * modifier, the snow line above sea level + 17). Schema: oracle/README.md,
 * "biome_climate.jsonl.gz (26.1)".
 */
final class BiomeClimateDump {
    static final int SAMPLES = 40;
    static final int ZOOM_SEEDS = 16;
    static final int ZOOM_SAMPLES = 200;

    private BiomeClimateDump() {}

    static void lines(Consumer<String> sink) {
        try {
            Harness.init();
            Registry<Biome> biomes = Harness.registries.lookupOrThrow(Registries.BIOME);
            Method temp = Biome.class.getDeclaredMethod("getHeightAdjustedTemperature", BlockPos.class, int.class);
            temp.setAccessible(true);
            java.lang.reflect.Field climateF = Biome.class.getDeclaredField("climateSettings");
            climateF.setAccessible(true);
            Rng rng = new Rng(0x62696f6d65L);
            for (int i = 0; i < biomes.size(); i++) {
                Biome b = biomes.byId(i);
                Object climate = climateF.get(b);
                Method mod = climate.getClass().getDeclaredMethod("temperatureModifier");
                mod.setAccessible(true);
                String modifier = mod.invoke(climate).toString().toLowerCase(java.util.Locale.ROOT);
                Json j = new Json().beginObject().field("type", "biome").field("index", i)
                        .field("name", biomes.getKey(b).toString())
                        .field("has_precipitation", b.hasPrecipitation())
                        .field("temperature", Integer.toHexString(Float.floatToRawIntBits(b.getBaseTemperature())))
                        .field("modifier", modifier);
                j.key("samples").beginArray();
                for (int k = 0; k < SAMPLES; k++) {
                    int x, y, z;
                    switch (k % 4) {
                        case 0 -> { x = rng.range(-30_000_000, 30_000_000); z = rng.range(-30_000_000, 30_000_000); }
                        case 1 -> { x = rng.range(-5000, 5000); z = rng.range(-5000, 5000); }
                        default -> { x = rng.range(-300, 300); z = rng.range(-300, 300); }
                    }
                    y = k % 3 == 0 ? rng.range(60, 320) : rng.range(-64, 320);
                    int sea = k % 5 == 0 ? rng.range(-64, 200) : 63;
                    BlockPos pos = new BlockPos(x, y, z);
                    float t = (float) temp.invoke(b, pos, sea);
                    String p = b.getPrecipitationAt(pos, sea).toString().toLowerCase(java.util.Locale.ROOT);
                    j.beginArray().value(x).value(y).value(z).value(sea)
                            .value(Integer.toHexString(Float.floatToRawIntBits(t))).value(p).endArray();
                }
                j.endArray();
                sink.accept(j.endObject().toString());
            }
            // BiomeManager.getBiome: which noise cell (quart position) a block position reads,
            // with the zoom seed the client gets as the spawn info's hashed seed.
            Biome any = biomes.byId(0);
            net.minecraft.core.Holder<Biome> holder = net.minecraft.core.Holder.direct(any);
            int[] cell = new int[3];
            net.minecraft.world.level.biome.BiomeManager.NoiseBiomeSource src = (qx, qy, qz) -> {
                cell[0] = qx;
                cell[1] = qy;
                cell[2] = qz;
                return holder;
            };
            Rng zr = new Rng(0x7a6f6f6dL);
            for (int i = 0; i < ZOOM_SEEDS; i++) {
                long seed = i == 0 ? 0L : zr.nextLong();
                var mgr = new net.minecraft.world.level.biome.BiomeManager(src, seed);
                Json j = new Json().beginObject().field("type", "zoom")
                        .field("seed", Long.toString(seed));
                j.key("samples").beginArray();
                for (int k = 0; k < ZOOM_SAMPLES; k++) {
                    int lim = k % 3 == 0 ? 30_000_000 : 64;
                    BlockPos pos = new BlockPos(zr.range(-lim, lim), zr.range(-64, 320), zr.range(-lim, lim));
                    mgr.getBiome(pos);
                    j.beginArray().value(pos.getX()).value(pos.getY()).value(pos.getZ())
                            .value(cell[0]).value(cell[1]).value(cell[2]).endArray();
                }
                j.endArray();
                sink.accept(j.endObject().toString());
            }
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }
}
