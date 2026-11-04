package ghast.oracle.v1_16_5;

import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import net.minecraft.client.network.ClientPlayerEntity;
import net.minecraft.entity.player.PlayerInventory;
import net.minecraft.item.Item;
import net.minecraft.item.ItemStack;
import net.minecraft.nbt.NbtCompound;
import net.minecraft.nbt.NbtList;
import net.minecraft.network.Packet;
import net.minecraft.network.PacketByteBuf;
import net.minecraft.network.packet.c2s.play.ClickSlotC2SPacket;
import net.minecraft.screen.CraftingScreenHandler;
import net.minecraft.screen.FurnaceScreenHandler;
import net.minecraft.screen.Generic3x3ContainerScreenHandler;
import net.minecraft.screen.GenericContainerScreenHandler;
import net.minecraft.screen.HopperScreenHandler;
import net.minecraft.screen.ScreenHandler;
import net.minecraft.screen.ShulkerBoxScreenHandler;
import net.minecraft.screen.slot.SlotActionType;
import net.minecraft.util.Identifier;
import net.minecraft.util.registry.Registry;

import java.io.IOException;
import java.io.UncheckedIOException;
import java.util.function.Consumer;

/**
 * Window click vectors for 1.16.5: the real client click path ({@code ClientPlayerInteractionManager.clickSlot}:
 * next action id, {@code ScreenHandler.onSlotClick}, the {@code ClickSlotC2SPacket} it sends) on the screen
 * handlers the client builds for each menu type (via the same constructors {@code ScreenHandlerType} uses),
 * over seeded random contents and click sequences. Same line format as the 1.8.9 dump, plus 41 player
 * inventory slots (offhand at 40) and the menu type id in {@code "menu"}.
 */
final class ClickDump {
    private ClickDump() {}

    static final String[] KINDS = {"player", "player", "chest", "chest", "generic3x3", "hopper", "shulker", "crafting", "furnace"};

    static final String[] ITEMS = {
        "stone", "dirt", "cobblestone", "oak_planks", "oak_log", "white_wool", "orange_wool", "coal", "stick",
        "carved_pumpkin", "player_head", "leather_helmet", "iron_chestplate", "diamond_helmet", "golden_boots",
        "elytra", "shield", "diamond_sword", "bow", "ender_pearl", "snowball", "egg", "water_bucket", "bucket",
        "potion", "white_banner", "honey_bottle", "red_shulker_box", "shulker_box", "torch",
        "warped_fungus_on_a_stick",
    };
    static final String[] ARMOR = {"leather_helmet", "iron_chestplate", "diamond_helmet", "golden_boots", "elytra", "carved_pumpkin"};
    static final String[] CRAFT = {"oak_planks", "oak_planks", "oak_planks", "oak_log", "stick", "cobblestone", "cobblestone", "coal", "white_wool"};

    private static Harness.Run newRun() {
        return new Harness.Run(new Scenario("clicks").fill(-8, 63, -8, 8, 63, 8, States.id("stone"), 0).spawn(0.5, 64, 0.5, 0, 0));
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
     * The vanilla recipes are loaded the way an integrated server loads its data pack and sent to
     * the client as a real {@code SynchronizeRecipesS2CPacket} (its bytes are the file's first
     * line, {@code {"declare_recipes":hex}}). {@code craft}: crafting cases only (window 0 or a
     * crafting table), grids filled from those recipes and, after every click that changed the
     * grid, the server's new result put into the result slot (recorded as {@code "server"}: the
     * set_slot item), as the server would send it.
     */
    static void cases(long seed, int count, boolean craft, Consumer<String> sink) {
        Harness.Run run = newRun();
        // a real client always has the server's recipes (they decide the remainders of a taken result)
        sink.accept("{\"declare_recipes\":\"" + syncRecipes() + "\"}");
        for (int i = 0; i < count; i++) {
            sink.accept(caseLine(run, seed, i, craft));
        }
    }

    private static net.minecraft.recipe.RecipeManager serverRecipes;
    private static String declareHex;

    /** Loads the vanilla data pack's recipes (once) and syncs them to the client; returns the packet hex. */
    private static String syncRecipes() {
        try {
            if (serverRecipes == null) {
                net.minecraft.resource.ServerResourceManager srm = net.minecraft.resource.ServerResourceManager.reload(
                        java.util.List.of(new net.minecraft.resource.DefaultResourcePack("minecraft")),
                        net.minecraft.server.command.CommandManager.RegistrationEnvironment.INTEGRATED, 2,
                        net.minecraft.util.Util.getMainWorkerExecutor(), Runnable::run).get();
                serverRecipes = srm.getRecipeManager();
                PacketByteBuf buf = new PacketByteBuf(Unpooled.buffer());
                // RecipeManager.values() iterates a map keyed by RecipeType objects (identity hash
                // codes: a different order in every JVM); the packet is sent sorted by type and id
                java.util.List<net.minecraft.recipe.Recipe<?>> sorted = new java.util.ArrayList<>(serverRecipes.values());
                sorted.sort(java.util.Comparator.comparing((net.minecraft.recipe.Recipe<?> rc) -> String.valueOf(
                        net.minecraft.util.registry.Registry.RECIPE_TYPE.getId(rc.getType()))).thenComparing(rc -> rc.getId().toString()));
                new net.minecraft.network.packet.s2c.play.SynchronizeRecipesS2CPacket(sorted).write(buf);
                byte[] bytes = new byte[buf.readableBytes()];
                buf.getBytes(buf.readerIndex(), bytes);
                declareHex = hex(buf);
                // the client side: the packet read back, its recipes into the client's recipe manager
                net.minecraft.network.packet.s2c.play.SynchronizeRecipesS2CPacket back =
                        new net.minecraft.network.packet.s2c.play.SynchronizeRecipesS2CPacket();
                back.read(new PacketByteBuf(Unpooled.wrappedBuffer(bytes)));
                Harness.net.getRecipeManager().setRecipes(back.getRecipes());
            }
            return declareHex;
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    /**
     * Order independence (selftest): the cases rerun in reverse order on the same player must give
     * the same lines, i.e. no state leaks from one case into the next. Returns the mismatches.
     */
    static int reverseMismatches(long seed, int count) {
        Harness.Run run = newRun();
        syncRecipes();
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

    private static NbtCompound tag(Rng r, Item item) {
        NbtCompound c = new NbtCompound();
        switch (r.nextInt(5)) {
            case 0 -> {
                NbtCompound d = new NbtCompound();
                d.putString("Name", r.chance(0.5) ? "{\"text\":\"a\"}" : "{\"text\":\"b\"}");
                c.put("display", d);
            }
            case 1 -> c.putBoolean("Unbreakable", true);
            case 2 -> {
                NbtList l = new NbtList();
                NbtCompound e = new NbtCompound();
                e.putString("id", r.chance(0.5) ? "minecraft:binding_curse" : "minecraft:sharpness");
                e.putShort("lvl", (short) r.nextInt(2));
                l.add(e);
                c.put("Enchantments", l);
            }
            case 3 -> c.putInt("Damage", item.getMaxDamage() > 0 ? r.range(0, 5) : 1);
            default -> {
                c.putInt("x", r.nextInt(2));
                c.putString("y", "z");
            }
        }
        return c;
    }

    private static java.util.List<ItemStack> grid(ScreenHandler h, int cells) {
        java.util.List<ItemStack> l = new java.util.ArrayList<>();
        for (int k = 1; k <= cells; k++) {
            l.add(h.getSlot(k).getStack().copy());
        }
        return l;
    }

    /** What the server computes for the grid and sends as the result slot (set into slot 0). */
    private static ItemStack serverResult(Harness.Run run, ScreenHandler h, int width) {
        net.minecraft.inventory.CraftingInventory inv = (net.minecraft.inventory.CraftingInventory) Reflect.get(
                h.getClass(), h, h instanceof net.minecraft.screen.PlayerScreenHandler ? "craftingInput" : "input");
        ItemStack res = serverRecipes.getFirstMatch(net.minecraft.recipe.RecipeType.CRAFTING, inv, run.world)
                .map(rc -> rc.craft(inv)).orElse(ItemStack.EMPTY);
        h.getSlot(0).setStack(res);
        return res;
    }

    /** Fills the grid (slots 1.. of width x width) from a random vanilla crafting recipe. */
    private static void fillGrid(Rng r, ScreenHandler h, int width) {
        int cells = width * width;
        ItemStack[] g = new ItemStack[cells];
        java.util.Arrays.fill(g, ItemStack.EMPTY);
        java.util.List<net.minecraft.recipe.Recipe<?>> all = new java.util.ArrayList<>();
        for (net.minecraft.recipe.Recipe<?> rc : serverRecipes.values()) {
            if (rc.getType() == net.minecraft.recipe.RecipeType.CRAFTING) {
                all.add(rc);
            }
        }
        all.sort(java.util.Comparator.comparing(rc -> rc.getId().toString()));
        net.minecraft.recipe.Recipe<?> rc = all.get(r.nextInt(all.size()));
        boolean done = false;
        if (r.chance(0.85) && rc instanceof net.minecraft.recipe.ShapedRecipe sr && sr.getWidth() <= width && sr.getHeight() <= width) {
            int w = sr.getWidth(), hh = sr.getHeight();
            int ox = r.nextInt(width - w + 1), oy = r.nextInt(width - hh + 1);
            boolean mirror = r.chance(0.3);
            for (int x = 0; x < w; x++) {
                for (int y = 0; y < hh; y++) {
                    g[ox + x + (oy + y) * width] = pick(r, sr.getIngredients().get((mirror ? w - x - 1 : x) + y * w));
                }
            }
            done = true;
        } else if (rc instanceof net.minecraft.recipe.ShapelessRecipe sl && sl.getIngredients().size() <= cells) {
            for (net.minecraft.recipe.Ingredient ing : sl.getIngredients()) {
                int c;
                do {
                    c = r.nextInt(cells);
                } while (!g[c].isEmpty());
                g[c] = pick(r, ing);
            }
            done = true;
        }
        if (!done) {
            for (int c = 0; c < cells; c++) {
                g[c] = r.chance(0.4) ? stack(r, CRAFT) : ItemStack.EMPTY;
            }
        }
        for (int c = 0; c < cells; c++) {
            h.getSlot(1 + c).setStack(g[c]);
        }
    }

    private static ItemStack pick(Rng r, net.minecraft.recipe.Ingredient ing) {
        ItemStack[] m = ing.getMatchingStacksClient();
        if (m.length == 0) {
            return ItemStack.EMPTY;
        }
        ItemStack s = m[r.nextInt(m.length)].copy();
        s.setCount(r.chance(0.5) ? 1 : r.range(1, s.getMaxCount()));
        return s;
    }

    private static ItemStack stack(Rng r, String[] pool) {
        Item item = Registry.ITEM.get(new Identifier(r.pick(pool)));
        int max = item.getMaxCount();
        int count = r.chance(0.08) ? max : r.chance(0.04) ? r.range(1, 70) : r.range(1, max);
        ItemStack s = new ItemStack(item, count);
        if (item.getMaxDamage() > 0 && r.chance(0.4)) {
            s.setDamage(r.range(1, 20));
        }
        if (r.chance(0.12)) {
            s.setTag(tag(r, item));
        }
        return s;
    }

    private static ItemStack maybe(Rng r, double p, String[] pool) {
        return r.chance(p) ? stack(r, pool) : ItemStack.EMPTY;
    }

    private static String one(Harness.Run run, Rng r, String name, boolean craft) {
        ClientPlayerEntity player = run.player;
        PlayerInventory inv = player.inventory;
        ScreenHandler w0 = player.playerScreenHandler;
        player.currentScreenHandler = w0;
        Reflect.set(ScreenHandler.class, w0, "actionId", (short) 0);
        inv.setCursorStack(ItemStack.EMPTY);
        double fill = 0.2 + r.nextDouble() * 0.6;
        for (int k = 0; k < 36; k++) {
            inv.main.set(k, maybe(r, fill, ITEMS));
        }
        for (int k = 0; k < 4; k++) {
            inv.armor.set(k, r.chance(0.3) ? stack(r, ARMOR) : ItemStack.EMPTY);
        }
        inv.offHand.set(0, maybe(r, 0.3, ITEMS));
        for (int k = 1; k <= 4; k++) {
            w0.getSlot(k).setStack(r.chance(0.3) ? stack(r, CRAFT) : ItemStack.EMPTY);
        }
        // The 1.16.5 client never computes crafting results (the server sends them as set_slot) and
        // this harness has no synced recipes, so taking a result over a non-empty grid would run the
        // "no recipe matches" remainder path a real client never takes. A result is only put over an
        // empty grid (taking it then consumes nothing on both sides).
        boolean gridEmpty = true;
        for (int k = 1; k <= 4; k++) {
            gridEmpty &= w0.getSlot(k).getStack().isEmpty();
        }
        w0.getSlot(0).setStack(gridEmpty && r.chance(0.4) ? stack(r, CRAFT) : ItemStack.EMPTY);
        inv.selectedSlot = r.nextInt(9);

        String kind = craft ? (r.chance(0.5) ? "player" : "crafting") : r.pick(KINDS);
        if (craft && kind.equals("player")) {
            fillGrid(r, w0, 2);
            serverResult(run, w0, 2);
        }
        int menu = -1;
        int sync = 0;
        ScreenHandler h = w0;
        if (!kind.equals("player")) {
            sync = r.range(1, 100);
            switch (kind) {
                case "chest" -> {
                    menu = r.range(0, 5);
                    h = switch (menu) {
                        case 0 -> GenericContainerScreenHandler.createGeneric9x1(sync, inv);
                        case 1 -> GenericContainerScreenHandler.createGeneric9x2(sync, inv);
                        case 2 -> GenericContainerScreenHandler.createGeneric9x3(sync, inv);
                        case 3 -> GenericContainerScreenHandler.createGeneric9x4(sync, inv);
                        case 4 -> GenericContainerScreenHandler.createGeneric9x5(sync, inv);
                        default -> GenericContainerScreenHandler.createGeneric9x6(sync, inv);
                    };
                }
                case "generic3x3" -> {
                    menu = 6;
                    h = new Generic3x3ContainerScreenHandler(sync, inv);
                }
                case "hopper" -> {
                    menu = 15;
                    h = new HopperScreenHandler(sync, inv);
                }
                case "shulker" -> {
                    menu = 19;
                    h = new ShulkerBoxScreenHandler(sync, inv);
                }
                case "crafting" -> {
                    menu = 11;
                    h = new CraftingScreenHandler(sync, inv);
                }
                default -> {
                    menu = 13;
                    h = new FurnaceScreenHandler(sync, inv);
                }
            }
            int n = h.slots.size() - 36;
            for (int k = 0; k < n; k++) {
                h.getSlot(k).setStack(kind.equals("crafting") ? (k > 0 && r.chance(0.4) ? stack(r, CRAFT) : ItemStack.EMPTY)
                        : maybe(r, fill, ITEMS));
            }
            if (kind.equals("crafting")) {
                boolean empty = true;
                for (int k = 1; k < 10; k++) {
                    empty &= h.getSlot(k).getStack().isEmpty();
                }
                if (empty && r.chance(0.5)) {
                    h.getSlot(0).setStack(stack(r, CRAFT));
                }
            }
            if (craft) {
                fillGrid(r, h, 3);
                serverResult(run, h, 3);
            }
            player.currentScreenHandler = h;
        }
        if (r.chance(0.3)) {
            inv.setCursorStack(stack(r, ITEMS));
        }

        StringBuilder b = new StringBuilder(4096);
        b.append("{\"case\":\"").append(name).append("\",\"kind\":\"").append(kind).append("\",\"menu\":").append(menu)
                .append(",\"sync\":").append(sync).append(",\"selected\":").append(inv.selectedSlot);
        state(b, player, w0, h);
        b.append(",\"clicks\":[");
        int clicks = r.range(1, 10);
        SlotActionType[] types = SlotActionType.values();
        for (int c = 0; c < clicks; c++) {
            int len = h.slots.size();
            int mode = r.pick(new int[] {0, 0, 0, 1, 1, 2, 4, 6});
            int slot = r.nextInt(len);
            int gridCells = kind.equals("player") ? 4 : 9;
            if (craft) {
                double q = r.nextDouble();
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
                case 2 -> button = r.chance(0.15) ? 40 : r.nextInt(9);
                case 6 -> button = 0;
                default -> {}
            }
            java.util.List<ItemStack> gridBefore = craft ? grid(h, gridCells) : null;
            Harness.connection.sent.clear();
            Harness.client.interactionManager.clickSlot(sync, slot, button, types[mode], player);
            ClickSlotC2SPacket p = null;
            for (Packet<?> q : Harness.connection.sent) {
                if (q instanceof ClickSlotC2SPacket cs) {
                    p = cs;
                }
            }
            if (c > 0) {
                b.append(',');
            }
            b.append("{\"slot\":").append(slot).append(",\"button\":").append(button).append(",\"mode\":").append(mode)
                    .append(",\"packet\":\"").append(packetHex(p)).append('"');
            state(b, player, w0, h);
            if (craft) {
                boolean changed = false;
                java.util.List<ItemStack> after = grid(h, gridCells);
                for (int k = 0; k < gridCells; k++) {
                    changed |= !ItemStack.areEqual(gridBefore.get(k), after.get(k));
                }
                if (changed) {
                    ItemStack res = serverResult(run, h, gridCells == 4 ? 2 : 3);
                    b.append(",\"server\":\"").append(hex(res)).append('"');
                }
            }
            b.append('}');
        }
        b.append("]}");
        player.currentScreenHandler = w0;
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
        for (int k = 0; k < 41; k++) {
            if (k > 0) {
                b.append(',');
            }
            b.append('"').append(hex(player.inventory.getStack(k))).append('"');
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

    private static String packetHex(ClickSlotC2SPacket p) {
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
