package ghast.oracle.v1_12_2;

import io.netty.buffer.Unpooled;
import net.minecraft.item.Item;
import net.minecraft.item.ItemStack;
import net.minecraft.recipe.Ingredient;
import net.minecraft.recipe.RecipeDispatcher;
import net.minecraft.recipe.RecipeType;
import net.minecraft.recipe.ShapedRecipeType;
import net.minecraft.recipe.ShapelessRecipeType;
import net.minecraft.util.Identifier;
import net.minecraft.util.PacketByteBuf;

import java.util.List;
import java.util.function.Consumer;

/**
 * The 1.12.2 crafting recipe list in registry order ({@code RecipeDispatcher.REGISTRY}, the order
 * {@code RecipeDispatcher.matches} tries them: first match wins), one JSON line per recipe:
 *
 * <pre>
 * {"i":N, "name":"minecraft:...", "type":"shaped", "w":W, "h":H, "in":[[[id,data]...]...], "out":hex}
 * {"i":N, "name":"minecraft:...", "type":"shapeless", "in":[[[id,data]...]...], "out":hex}
 * {"i":N, "name":"minecraft:...", "type":"special", "class":"ArmorDyeRecipeType"}
 * </pre>
 *
 * Each ingredient is the list of stacks it accepts (empty = an empty cell); {@code data} 32767 is the
 * wildcard; {@code out} is the result stack's wire bytes. Note: the 1.12 client does not compute crafting
 * results itself (the server sends the result slot); the list is for tools and the recipe book.
 */
final class RecipeDump {
    private RecipeDump() {}

    static void lines(Consumer<String> sink) {
        Harness.init();
        int n = 0;
        for (RecipeType r : RecipeDispatcher.REGISTRY) {
            Identifier id = RecipeDispatcher.REGISTRY.getIdentifier(r);
            StringBuilder b = new StringBuilder("{\"i\":").append(n++).append(",\"name\":\"").append(id).append('"');
            if (r.getClass() == ShapedRecipeType.class && r instanceof ShapedRecipeType s) {
                int w = (int) Reflect.get(ShapedRecipeType.class, s, "width");
                int h = (int) Reflect.get(ShapedRecipeType.class, s, "height");
                b.append(",\"type\":\"shaped\",\"w\":").append(w).append(",\"h\":").append(h).append(",\"in\":");
                ingredients(b, s.method_14252());
                b.append(",\"out\":\"").append(hex(s.getOutput())).append('"');
            } else if (r.getClass() == ShapelessRecipeType.class && r instanceof ShapelessRecipeType s) {
                b.append(",\"type\":\"shapeless\",\"in\":");
                ingredients(b, s.method_14252());
                b.append(",\"out\":\"").append(hex(s.getOutput())).append('"');
            } else {
                b.append(",\"type\":\"special\",\"class\":\"").append(r.getClass().getSimpleName()).append('"');
            }
            sink.accept(b.append('}').toString());
        }
    }

    private static void ingredients(StringBuilder b, List<Ingredient> in) {
        b.append('[');
        for (int k = 0; k < in.size(); k++) {
            if (k > 0) {
                b.append(',');
            }
            b.append('[');
            ItemStack[] opts = in.get(k).method_14244();
            for (int o = 0; o < opts.length; o++) {
                if (o > 0) {
                    b.append(',');
                }
                b.append('[').append(Item.getRawId(opts[o].getItem())).append(',').append(opts[o].getData()).append(']');
            }
            b.append(']');
        }
        b.append(']');
    }

    private static String hex(ItemStack s) {
        PacketByteBuf buf = new PacketByteBuf(Unpooled.buffer());
        buf.writeItemStack(s);
        StringBuilder x = new StringBuilder();
        while (buf.isReadable()) {
            int v = buf.readUnsignedByte();
            x.append(Character.forDigit(v >> 4, 16)).append(Character.forDigit(v & 15, 16));
        }
        return x.toString();
    }
}
