package ghast.oracle.modern;

import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.item.Item;

/**
 * Registry lookups by name. The constant holders move between modern versions (26.2 moved the
 * entity type constants from {@code EntityType} to {@code EntityTypes} and the dyed bundles into a
 * colour collection); a lookup by id reaches the same object in every version.
 */
final class Reg {
    private Reg() {}

    static EntityType<?> entity(String path) {
        return BuiltInRegistries.ENTITY_TYPE.getOptional(Identifier.withDefaultNamespace(path))
                .orElseThrow(() -> new IllegalStateException("no entity type " + path));
    }

    static Item item(String path) {
        return BuiltInRegistries.ITEM.getOptional(Identifier.withDefaultNamespace(path))
                .orElseThrow(() -> new IllegalStateException("no item " + path));
    }
}
