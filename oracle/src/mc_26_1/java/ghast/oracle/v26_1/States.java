package ghast.oracle.v26_1;

import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;

import java.util.Optional;

/**
 * Block states by their global id (the id chunk data and block update packets carry), and a
 * {@code "name[prop=value,...]"} parser for writing scenarios. Unlisted properties keep the default.
 */
final class States {
    private States() {}

    static BlockState byId(int id) {
        BlockState s = Block.BLOCK_STATE_REGISTRY.byId(id);
        if (s == null) {
            throw new IllegalArgumentException("no block state with id " + id);
        }
        return s;
    }

    static int id(BlockState state) {
        return Block.getId(state);
    }

    /** Canonical string, e.g. {@code minecraft:oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]}. */
    static String name(BlockState state) {
        return BlockStateParser.serialize(state);
    }

    static Block block(String name) {
        Identifier idf = Identifier.parse(name);
        if (!BuiltInRegistries.BLOCK.containsKey(idf)) {
            throw new IllegalArgumentException("unknown block " + name);
        }
        return BuiltInRegistries.BLOCK.getValue(idf);
    }

    /** {@code "oak_stairs[facing=north]"} or {@code "minecraft:stone"} -> state id. */
    static int id(String spec) {
        String name = spec;
        String props = "";
        int b = spec.indexOf('[');
        if (b >= 0) {
            name = spec.substring(0, b);
            props = spec.substring(b + 1, spec.length() - 1);
        }
        BlockState state = block(name).defaultBlockState();
        if (!props.isEmpty()) {
            for (String kv : props.split(",")) {
                String[] p = kv.split("=", 2);
                Property<?> prop = state.getBlock().getStateDefinition().getProperty(p[0].trim());
                if (prop == null) {
                    throw new IllegalArgumentException(spec + ": no property " + p[0]);
                }
                state = with(state, prop, p[1].trim(), spec);
            }
        }
        return id(state);
    }

    private static <T extends Comparable<T>> BlockState with(BlockState state, Property<T> prop, String value, String spec) {
        Optional<T> v = prop.getValue(value);
        if (v.isEmpty()) {
            throw new IllegalArgumentException(spec + ": bad value " + value + " for " + prop.getName());
        }
        return state.setValue(prop, v.get());
    }

    static int count() {
        return Block.BLOCK_STATE_REGISTRY.size();
    }
}
