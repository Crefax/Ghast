package ghast.oracle.v1_8_9;

import io.netty.buffer.Unpooled;
import net.minecraft.item.Item;
import net.minecraft.item.ItemStack;
import net.minecraft.recipe.RecipeDispatcher;
import net.minecraft.recipe.RecipeType;
import net.minecraft.recipe.ShapedRecipeType;
import net.minecraft.recipe.ShapelessRecipeType;
import net.minecraft.util.PacketByteBuf;

import java.util.List;
import java.util.function.Consumer;

/**
 * The 1.8.9 crafting recipe list exactly as the client matches it: {@code RecipeDispatcher.getAllRecipes()}
 * after its sort (first match wins), one JSON line per recipe:
 *
 * <pre>
 * {"i":N, "type":"shaped", "w":W, "h":H, "in":[[id,data]|null...], "out":hex, "copy_nbt":B}
 * {"i":N, "type":"shapeless", "in":[[id,data]...], "out":hex}
 * {"i":N, "type":"special", "class":"ArmorDyeRecipeType", "size":S}
 * </pre>
 *
 * {@code data} 32767 is the wildcard; {@code out} is the result stack's wire bytes.
 */
final class RecipeDump {
    private RecipeDump() {}

    static void lines(Consumer<String> sink) {
        Harness.init();
        List<RecipeType> all = RecipeDispatcher.getInstance().getAllRecipes();
        for (int n = 0; n < all.size(); n++) {
            RecipeType r = all.get(n);
            StringBuilder b = new StringBuilder("{\"i\":").append(n);
            if (r.getClass() == ShapedRecipeType.class && r instanceof ShapedRecipeType s) {
                int w = (int) Reflect.get(ShapedRecipeType.class, s, "width");
                int h = (int) Reflect.get(ShapedRecipeType.class, s, "height");
                ItemStack[] in = (ItemStack[]) Reflect.get(ShapedRecipeType.class, s, "ingredients");
                boolean copy = (boolean) Reflect.get(ShapedRecipeType.class, s, "copyIngredientsNbt");
                b.append(",\"type\":\"shaped\",\"w\":").append(w).append(",\"h\":").append(h).append(",\"in\":");
                ingredients(b, java.util.Arrays.asList(in));
                b.append(",\"out\":\"").append(hex(s.getOutput())).append("\",\"copy_nbt\":").append(copy);
            } else if (r.getClass() == ShapelessRecipeType.class && r instanceof ShapelessRecipeType s) {
                @SuppressWarnings("unchecked")
                List<ItemStack> in = (List<ItemStack>) Reflect.get(ShapelessRecipeType.class, s, "stacks");
                b.append(",\"type\":\"shapeless\",\"in\":");
                ingredients(b, in);
                b.append(",\"out\":\"").append(hex(s.getOutput())).append('"');
            } else {
                b.append(",\"type\":\"special\",\"class\":\"").append(r.getClass().getSimpleName())
                        .append("\",\"size\":").append(r.getSize());
            }
            sink.accept(b.append('}').toString());
        }
    }

    private static void ingredients(StringBuilder b, List<ItemStack> in) {
        b.append('[');
        for (int k = 0; k < in.size(); k++) {
            if (k > 0) {
                b.append(',');
            }
            ItemStack s = in.get(k);
            if (s == null) {
                b.append("null");
            } else {
                b.append('[').append(Item.getRawId(s.getItem())).append(',').append(s.getData()).append(']');
            }
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
