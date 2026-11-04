package ghast.oracle.v1_16_5;

import net.minecraft.block.Block;
import net.minecraft.block.BlockState;
import net.minecraft.command.argument.BlockArgumentParser;
import net.minecraft.state.property.Property;
import net.minecraft.util.Identifier;
import net.minecraft.util.registry.Registry;

import java.util.Optional;

/**
 * Block states by their global raw id (the id chunk data and block update packets carry), and a
 * {@code "name[prop=value,...]"} parser for writing scenarios. Unlisted properties keep the default.
 */
final class States {
    private States() {}

    static BlockState byId(int id) {
        BlockState s = Block.STATE_IDS.get(id);
        if (s == null) {
            throw new IllegalArgumentException("no block state with id " + id);
        }
        return s;
    }

    static int id(BlockState state) {
        return Block.getRawIdFromState(state);
    }

    /** Canonical string, e.g. {@code minecraft:oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]}. */
    static String name(BlockState state) {
        return BlockArgumentParser.stringifyBlockState(state);
    }

    /** {@code "oak_stairs[facing=north]"} or {@code "minecraft:stone"} -> raw state id. */
    static int id(String spec) {
        String name = spec;
        String props = "";
        int b = spec.indexOf('[');
        if (b >= 0) {
            name = spec.substring(0, b);
            props = spec.substring(b + 1, spec.length() - 1);
        }
        Identifier idf = new Identifier(name);
        if (!Registry.BLOCK.containsId(idf)) {
            throw new IllegalArgumentException("unknown block " + name);
        }
        BlockState state = Registry.BLOCK.get(idf).getDefaultState();
        if (!props.isEmpty()) {
            for (String kv : props.split(",")) {
                String[] p = kv.split("=", 2);
                Property<?> prop = state.getBlock().getStateManager().getProperty(p[0].trim());
                if (prop == null) {
                    throw new IllegalArgumentException(spec + ": no property " + p[0]);
                }
                state = with(state, prop, p[1].trim(), spec);
            }
        }
        return id(state);
    }

    private static <T extends Comparable<T>> BlockState with(BlockState state, Property<T> prop, String value, String spec) {
        Optional<T> v = prop.parse(value);
        if (v.isEmpty()) {
            throw new IllegalArgumentException(spec + ": bad value " + value + " for " + prop.getName());
        }
        return state.with(prop, v.get());
    }

    static int count() {
        return Block.STATE_IDS.size();
    }
}
