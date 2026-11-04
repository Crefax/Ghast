package ghast.oracle.v1_8_9;

import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import net.minecraft.client.MinecraftClient;
import net.minecraft.entity.player.ClientPlayerEntity;
import net.minecraft.entity.player.PlayerInventory;
import net.minecraft.inventory.ClientNetworkSyncedInventory;
import net.minecraft.inventory.SimpleInventory;
import net.minecraft.item.Item;
import net.minecraft.item.ItemStack;
import net.minecraft.nbt.NbtCompound;
import net.minecraft.nbt.NbtList;
import net.minecraft.network.Packet;
import net.minecraft.network.packet.c2s.play.ClickWindowC2SPacket;
import net.minecraft.screen.ChestScreenHandler;
import net.minecraft.screen.CraftingScreenHandler;
import net.minecraft.screen.FurnaceScreenHandler;
import net.minecraft.screen.Generic3x3ScreenHandler;
import net.minecraft.screen.HopperScreenHandler;
import net.minecraft.screen.ScreenHandler;
import net.minecraft.text.LiteralText;
import net.minecraft.util.PacketByteBuf;
import net.minecraft.util.math.BlockPos;

import java.io.IOException;
import java.io.UncheckedIOException;
import java.util.function.Consumer;

/**
 * Window click vectors: the real 1.8.9 client click path ({@code ClientPlayerInteractionManager.clickSlot}:
 * next action id, {@code ScreenHandler.onSlotClick}, the {@code ClickWindowC2SPacket} it sends) run on real
 * screen handlers built the way the client's screens build them, over seeded random contents and click
 * sequences. One JSON line per case:
 *
 * <pre>
 * {"case":..., "kind":"player"|"chest"|"generic3x3"|"hopper"|"crafting"|"furnace", "rows":R, "sync":S,
 *  "selected":H, "w0":[slot hex...], "screen":[slot hex...], "cursor":hex, "inv":[40 slot hex],
 *  "clicks":[{"slot":..,"button":..,"mode":..,"packet":hex,"screen":[...],"w0":[...],"cursor":..,"inv":[...]}]}
 * </pre>
 *
 * Every stack is the hex of its wire form ({@code PacketByteBuf.writeItemStack}: id, count, damage,
 * NBT), the packet the hex of {@code ClickWindowC2SPacket.write} (the body after the packet id). Item
 * entities that drops spawn are irrelevant to the inventory.
 */
final class ClickDump {
    private ClickDump() {}

    static final String[] KINDS = {"player", "player", "chest", "chest", "generic3x3", "hopper", "crafting", "furnace"};

    /** Items with the properties the click rules depend on (stack sizes, subtypes, armor, damage, remainders). */
    static final int[] ITEMS = {
        1, 3, 4, 5, 12, 17, 35, 263, 280, 86, 397, 298, 307, 310, 317, 276, 261, 368, 332, 344,
        326, 325, 373, 425, 351, 58, 54, 50,
    };
    /** Ingredients for crafting grids, so recipes match now and then. */
    static final int[] CRAFT = {5, 5, 5, 17, 280, 4, 4, 263, 35, 12};

    private static Harness.Run newRun() {
        return new Harness.Run(new Scenario("clicks").fill(-8, 63, -8, 8, 63, 8, 1).spawn(0.5, 64, 0.5, 0, 0));
    }

    /** Case {@code i} depends only on (seed, i, craft). */
    private static String caseLine(Harness.Run run, long seed, int i, boolean craft) {
        return craft ? one(run, new Rng(seed * 1_000_033L + i + 7), "craft_s" + seed + "_" + i, true)
                : one(run, new Rng(seed * 1_000_003L + i), "click_s" + seed + "_" + i, false);
    }

    static void cases(long seed, int count, Consumer<String> sink) {
        cases(seed, count, false, sink);
    }

    /**
     * {@code craft}: crafting cases only (window 0 or a crafting table), grids filled from real
     * recipes and the special recipes' ingredients, clicks biased to the result and the grid.
     */
    static void cases(long seed, int count, boolean craft, Consumer<String> sink) {
        Harness.Run run = newRun();
        for (int i = 0; i < count; i++) {
            sink.accept(caseLine(run, seed, i, craft));
        }
    }

    /**
     * Order independence (selftest): the cases rerun in reverse order on the same player must give
     * the same lines, i.e. no state leaks from one case into the next. Returns the mismatches.
     */
    static int reverseMismatches(long seed, int count) {
        Harness.Run run = newRun();
        java.util.List<String> forward = new java.util.ArrayList<>(count);
        for (int i = 0; i < count; i++) {
            forward.add(caseLine(run, seed, i, (i & 1) == 1));
        }
        int bad = 0;
        for (int i = count - 1; i >= 0; i--) {
            if (!caseLine(run, seed, i, (i & 1) == 1).equals(forward.get(i))) {
                System.out.println("DIFF reverse-order rerun of click case " + i);
                bad++;
            }
        }
        return bad;
    }

    private static NbtCompound tag(Rng r) {
        NbtCompound c = new NbtCompound();
        switch (r.nextInt(4)) {
            case 0 -> {
                NbtCompound d = new NbtCompound();
                d.putString("Name", r.chance(0.5) ? "a" : "b");
                c.put("display", d);
            }
            case 1 -> c.putBoolean("Unbreakable", true);
            case 2 -> {
                NbtList l = new NbtList();
                NbtCompound e = new NbtCompound();
                e.putShort("id", (short) 16);
                e.putShort("lvl", (short) (1 + r.nextInt(2)));
                l.add(e);
                c.put("ench", l);
            }
            default -> {
                c.putInt("x", r.nextInt(2));
                c.putString("y", "z");
            }
        }
        return c;
    }

    private static ItemStack of(int id, int count, int data) {
        return new ItemStack(Item.byRawId(id), count, data);
    }

    private static int count(Rng r, int id) {
        int max = Item.byRawId(id).getMaxCount();
        return r.chance(0.5) ? 1 : r.range(1, max);
    }

    /** Fills a crafting grid (slots first.. of width x width) from a random recipe or special. */
    private static void fillGrid(Rng r, ScreenHandler h, int first, int width) {
        int cells = width * width;
        ItemStack[] g = new ItemStack[cells];
        double p = r.nextDouble();
        java.util.List<net.minecraft.recipe.RecipeType> all = net.minecraft.recipe.RecipeDispatcher.getInstance().getAllRecipes();
        net.minecraft.recipe.RecipeType rt = all.get(r.nextInt(all.size()));
        boolean done = false;
        if (p < 0.7 && rt.getClass() == net.minecraft.recipe.ShapedRecipeType.class) {
            int w = (int) Reflect.get(net.minecraft.recipe.ShapedRecipeType.class, rt, "width");
            int hh = (int) Reflect.get(net.minecraft.recipe.ShapedRecipeType.class, rt, "height");
            ItemStack[] in = (ItemStack[]) Reflect.get(net.minecraft.recipe.ShapedRecipeType.class, rt, "ingredients");
            if (w <= width && hh <= width) {
                int ox = r.nextInt(width - w + 1), oy = r.nextInt(width - hh + 1);
                boolean mirror = r.chance(0.3);
                for (int x = 0; x < w; x++) {
                    for (int y = 0; y < hh; y++) {
                        ItemStack ing = in[(mirror ? w - x - 1 : x) + y * w];
                        if (ing != null) {
                            g[ox + x + (oy + y) * width] = ingredient(r, ing);
                        }
                    }
                }
                done = true;
            }
        } else if (p < 0.7 && rt.getClass() == net.minecraft.recipe.ShapelessRecipeType.class) {
            @SuppressWarnings("unchecked")
            java.util.List<ItemStack> in = (java.util.List<ItemStack>) Reflect.get(net.minecraft.recipe.ShapelessRecipeType.class, rt, "stacks");
            if (in.size() <= cells) {
                for (ItemStack ing : in) {
                    int c;
                    do {
                        c = r.nextInt(cells);
                    } while (g[c] != null);
                    g[c] = ingredient(r, ing);
                }
                done = true;
            }
        }
        if (!done && p < 0.95) {
            java.util.List<ItemStack> items = special(r);
            if (items.size() <= cells) {
                for (ItemStack it : items) {
                    int c;
                    do {
                        c = r.nextInt(cells);
                    } while (g[c] != null);
                    g[c] = it;
                }
                done = true;
            }
        }
        if (!done) {
            for (int c = 0; c < cells; c++) {
                g[c] = r.chance(0.4) ? stack(r, CRAFT) : null;
            }
        }
        for (int c = 0; c < cells; c++) {
            h.getSlot(first + c).setStack(g[c]);
        }
    }

    private static ItemStack ingredient(Rng r, ItemStack ing) {
        Item item = ing.getItem();
        int data = ing.getData() == 32767 ? (item.isUnbreakable() ? r.nextInt(3) : 0) : ing.getData();
        int id = Item.getRawId(item);
        ItemStack s = of(id, count(r, id), data);
        if (r.chance(0.05)) {
            s.setNbt(tag(r));
        }
        return s;
    }

    private static NbtCompound explosion(Rng r) {
        NbtCompound e = new NbtCompound();
        e.putByte("Type", (byte) r.nextInt(5));
        e.putIntArray("Colors", new int[] {r.nextInt(16777216)});
        if (r.chance(0.3)) {
            e.putBoolean("Trail", true);
        }
        NbtCompound c = new NbtCompound();
        c.put("Explosion", e);
        return c;
    }

    /** Ingredients for one of the special recipes (armor dye, fireworks, books, maps, repair, banners). */
    private static java.util.List<ItemStack> special(Rng r) {
        java.util.List<ItemStack> l = new java.util.ArrayList<>();
        switch (r.nextInt(9)) {
            case 0 -> { // armor dye
                ItemStack a = of(298 + r.nextInt(r.chance(0.9) ? 4 : 8), 1, 0);
                if (r.chance(0.5)) {
                    NbtCompound d = new NbtCompound();
                    d.putInt("color", r.nextInt(16777216));
                    NbtCompound c = new NbtCompound();
                    c.put("display", d);
                    a.setNbt(c);
                }
                l.add(a);
                for (int k = r.range(1, 3); k > 0; k--) {
                    l.add(of(351, 1, r.nextInt(16)));
                }
            }
            case 1 -> { // rocket
                l.add(of(339, 1, 0));
                for (int k = r.range(1, 3); k > 0; k--) {
                    l.add(of(289, 1, 0));
                }
                for (int k = r.nextInt(3); k > 0; k--) {
                    ItemStack ch = of(402, 1, 0);
                    if (r.chance(0.8)) {
                        ch.setNbt(explosion(r));
                    }
                    l.add(ch);
                }
            }
            case 2 -> { // star
                l.add(of(289, 1, 0));
                for (int k = r.range(1, 2); k > 0; k--) {
                    l.add(of(351, 1, r.nextInt(16)));
                }
                if (r.chance(0.5)) {
                    l.add(of(r.chance(0.5) ? 348 : 264, 1, 0));
                }
                if (r.chance(0.5)) {
                    l.add(of(r.pick(new int[] {385, 288, 371, 397}), 1, 0));
                }
            }
            case 3 -> { // fade
                ItemStack ch = of(402, 1, 0);
                if (r.chance(0.85)) {
                    ch.setNbt(explosion(r));
                }
                l.add(ch);
                for (int k = r.range(1, 3); k > 0; k--) {
                    l.add(of(351, 1, r.nextInt(16)));
                }
            }
            case 4 -> { // book cloning
                ItemStack b = of(387, r.chance(0.9) ? 1 : 2, 0);
                NbtCompound c = new NbtCompound();
                c.putString("title", "t");
                c.putString("author", "a");
                c.putInt("generation", r.nextInt(3));
                NbtList pages = new NbtList();
                pages.add(new net.minecraft.nbt.NbtString("p"));
                c.put("pages", pages);
                if (r.chance(0.3)) {
                    NbtCompound d = new NbtCompound();
                    d.putString("Name", "n");
                    c.put("display", d);
                }
                b.setNbt(c);
                l.add(b);
                for (int k = r.range(1, 3); k > 0; k--) {
                    l.add(of(386, 1, 0));
                }
            }
            case 5 -> { // map cloning
                ItemStack m = of(358, 1, r.nextInt(5));
                if (r.chance(0.3)) {
                    m.setCustomName("m");
                }
                l.add(m);
                for (int k = r.range(1, 3); k > 0; k--) {
                    l.add(of(395, 1, 0));
                }
            }
            case 6 -> { // repair
                int id = r.pick(new int[] {276, 261, 256, 310, 359});
                int max = Item.byRawId(id).getMaxDamage();
                l.add(of(id, 1, r.nextInt(max)));
                l.add(of(r.chance(0.9) ? id : 267, r.chance(0.9) ? 1 : 2, r.nextInt(max)));
            }
            case 7 -> { // banner
                l.add(of(425, 1, r.nextInt(16)));
                l.add(of(351, 1, r.nextInt(16)));
            }
            default -> { // map upscale shape
                for (int k = 0; k < 8; k++) {
                    l.add(of(339, 1, 0));
                }
                l.add(of(358, 1, 0));
            }
        }
        return l;
    }

    private static ItemStack stack(Rng r, int[] pool) {
        int id = r.pick(pool);
        Item item = Item.byRawId(id);
        int max = item.getMaxCount();
        int count = r.chance(0.08) ? max : r.chance(0.04) ? r.range(0, 70) : r.range(1, max);
        int damage = 0;
        if (item.isUnbreakable()) { // hasSubtypes
            damage = r.nextInt(3);
        } else if (item.getMaxDamage() > 0 && r.chance(0.4)) {
            damage = r.range(1, 20);
        } else if (r.chance(0.03)) {
            damage = 1; // a damage value on an item without subtypes
        }
        ItemStack s = new ItemStack(item, count, damage);
        if (r.chance(0.12)) {
            s.setNbt(tag(r));
        }
        return s;
    }

    private static ItemStack maybe(Rng r, double p, int[] pool) {
        return r.chance(p) ? stack(r, pool) : null;
    }

    private static String one(Harness.Run run, Rng r, String name, boolean craft) {
        ClientPlayerEntity player = run.player;
        PlayerInventory inv = player.inventory;
        ScreenHandler w0 = player.playerScreenHandler;
        // Fresh player state.
        player.openScreenHandler = w0;
        Reflect.set(ScreenHandler.class, w0, "actionId", (short) 0);
        inv.setCursorStack(null);
        double fill = 0.2 + r.nextDouble() * 0.6;
        for (int k = 0; k < 36; k++) {
            inv.main[k] = maybe(r, fill, ITEMS);
        }
        for (int k = 0; k < 4; k++) {
            inv.armor[k] = r.chance(0.3) ? stack(r, new int[] {298, 307, 310, 317, 86, 397}) : null;
        }
        for (int k = 1; k <= 4; k++) {
            w0.getSlot(k).setStack(r.chance(0.3) ? stack(r, CRAFT) : null);
        }
        inv.selectedSlot = r.nextInt(9);

        String kind = craft ? (r.chance(0.5) ? "player" : "crafting") : r.pick(KINDS);
        if (craft && kind.equals("player")) {
            fillGrid(r, w0, 1, 2);
        }
        int rows = 0;
        int sync = 0;
        ScreenHandler h = w0;
        if (!kind.equals("player")) {
            sync = r.range(1, 100);
            switch (kind) {
                case "chest" -> {
                    rows = r.range(1, 6);
                    h = new ChestScreenHandler(inv, new SimpleInventory("c", false, rows * 9), player);
                }
                case "generic3x3" -> h = new Generic3x3ScreenHandler(inv, new SimpleInventory("d", false, 9));
                case "hopper" -> h = new HopperScreenHandler(inv, new SimpleInventory("h", false, 5), player);
                case "crafting" -> h = new CraftingScreenHandler(inv, run.world, BlockPos.ORIGIN);
                default -> h = new FurnaceScreenHandler(inv,
                        new ClientNetworkSyncedInventory("minecraft:furnace", new LiteralText("f"), 3));
            }
            h.syncId = sync;
            int n = h.slots.size() - 36;
            for (int k = 0; k < n; k++) {
                if (kind.equals("crafting") && k > 0) {
                    h.getSlot(k).setStack(r.chance(0.4) ? stack(r, CRAFT) : null);
                } else if (!kind.equals("crafting")) {
                    h.getSlot(k).setStack(maybe(r, fill, ITEMS));
                }
            }
            if (craft) {
                fillGrid(r, h, 1, 3);
            }
            player.openScreenHandler = h;
        }
        if (r.chance(0.3)) {
            inv.setCursorStack(stack(r, ITEMS));
        }

        StringBuilder b = new StringBuilder(4096);
        b.append("{\"case\":\"").append(name).append("\",\"kind\":\"").append(kind).append("\",\"rows\":").append(rows)
                .append(",\"sync\":").append(sync).append(",\"selected\":").append(inv.selectedSlot);
        state(b, player, w0, h);
        b.append(",\"clicks\":[");
        int clicks = r.range(1, 10);
        CapturingNetworkHandler net = run.net;
        for (int c = 0; c < clicks; c++) {
            int len = h.slots.size();
            int mode = r.pick(new int[] {0, 0, 0, 1, 1, 2, 4, 6});
            int slot = r.nextInt(len);
            if (craft) {
                double q = r.nextDouble();
                int gridCells = kind.equals("player") ? 4 : 9;
                if (q < 0.4) {
                    slot = 0;
                } else if (q < 0.7) {
                    slot = 1 + r.nextInt(gridCells);
                }
            }
            int button = r.nextInt(2);
            switch (mode) {
                case 0, 4 -> {
                    if (r.chance(0.08)) {
                        slot = -999;
                    }
                }
                case 2 -> button = r.nextInt(9);
                case 6 -> button = 0;
                default -> {}
            }
            net.sent.clear();
            MinecraftClient.getInstance().interactionManager.clickSlot(sync, slot, button, mode, player);
            ClickWindowC2SPacket p = null;
            for (Packet<?> q : net.sent) {
                if (q instanceof ClickWindowC2SPacket cw) {
                    p = cw;
                }
            }
            if (c > 0) {
                b.append(',');
            }
            b.append("{\"slot\":").append(slot).append(",\"button\":").append(button).append(",\"mode\":").append(mode)
                    .append(",\"packet\":\"").append(packetHex(p)).append('"');
            state(b, player, w0, h);
            b.append('}');
        }
        b.append("]}");
        player.openScreenHandler = w0;
        return b.toString();
    }

    private static void state(StringBuilder b, ClientPlayerEntity player, ScreenHandler w0, ScreenHandler h) {
        b.append(",\"w0\":");
        slots(b, w0);
        b.append(",\"screen\":");
        if (h == w0) {
            b.append("null");
        } else {
            slots(b, h);
        }
        b.append(",\"cursor\":\"").append(hex(player.inventory.getCursorStack())).append('"');
        b.append(",\"inv\":[");
        for (int k = 0; k < 40; k++) {
            if (k > 0) {
                b.append(',');
            }
            ItemStack s = k < 36 ? player.inventory.main[k] : player.inventory.armor[k - 36];
            b.append('"').append(hex(s)).append('"');
        }
        b.append(']');
    }

    private static void slots(StringBuilder b, ScreenHandler h) {
        b.append('[');
        for (int k = 0; k < h.slots.size(); k++) {
            if (k > 0) {
                b.append(',');
            }
            b.append('"').append(hex(h.getSlot(k).getStack())).append('"');
        }
        b.append(']');
    }

    private static String hex(ItemStack s) {
        PacketByteBuf buf = new PacketByteBuf(Unpooled.buffer());
        buf.writeItemStack(s);
        return hex(buf);
    }

    private static String packetHex(ClickWindowC2SPacket p) {
        if (p == null) {
            return "";
        }
        PacketByteBuf buf = new PacketByteBuf(Unpooled.buffer());
        try {
            p.write(buf);
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
        return hex(buf);
    }

    private static String hex(ByteBuf buf) {
        StringBuilder s = new StringBuilder(buf.readableBytes() * 2);
        while (buf.isReadable()) {
            int v = buf.readUnsignedByte();
            s.append(Character.forDigit(v >> 4, 16)).append(Character.forDigit(v & 15, 16));
        }
        return s.toString();
    }
}
