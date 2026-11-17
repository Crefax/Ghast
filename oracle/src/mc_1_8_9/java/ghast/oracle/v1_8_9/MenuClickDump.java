package ghast.oracle.v1_8_9;

import ghast.oracle.Rng;
import io.netty.buffer.Unpooled;
import net.minecraft.entity.data.Merchant;
import net.minecraft.entity.passive.HorseBaseEntity;
import net.minecraft.inventory.AnimalInventory;
import net.minecraft.client.MinecraftClient;
import net.minecraft.entity.player.ClientPlayerEntity;
import net.minecraft.entity.player.PlayerInventory;
import net.minecraft.inventory.ClientNetworkSyncedInventory;
import net.minecraft.item.Item;
import net.minecraft.item.ItemStack;
import net.minecraft.network.Packet;
import net.minecraft.network.packet.c2s.play.ClickWindowC2SPacket;
import net.minecraft.screen.AnvilScreenHandler;
import net.minecraft.screen.BeaconScreenHandler;
import net.minecraft.screen.BrewingScreenHandler;
import net.minecraft.screen.EnchantingScreenHandler;
import net.minecraft.screen.FurnaceScreenHandler;
import net.minecraft.recipe.SmeltingRecipeRegistry;
import net.minecraft.screen.HorseScreenHandler;
import net.minecraft.screen.ScreenHandler;
import net.minecraft.screen.VillagerScreenHandler;
import net.minecraft.util.PacketByteBuf;
import net.minecraft.village.TradeOffer;
import net.minecraft.village.TraderOfferList;
import net.minecraft.text.LiteralText;

import java.io.IOException;
import java.io.UncheckedIOException;
import java.util.function.Consumer;

/**
 * Window click vectors for the special menus (brewing stand, enchanting table, anvil, beacon): the same click path and line
 * format as {@link ClickDump} ({@code "kind"} is the menu name), over contents biased to the items
 * the menu's slot rules look at, with clicks biased to the menu's own slots.
 *
 * <p>Each menu's set starts with one {@code {"facts":menu,...}} line: the slot rules read off the
 * real slots ({@code Slot.canInsert} over every item id and damage 0..15), which the Rust side
 * checks its own tables against.
 *
 * <p>What is faked: the screen handler is built directly over the same inventory class the client
 * uses for the window ({@code ClientNetworkSyncedInventory} for the brewing stand and the beacon, as
 * {@code ClientPlayNetworkHandler.onOpenScreen} does; the enchanting table's own inventory, over the
 * harness's client world at the origin; the anvil's own inventories, whose output the handler
 * computes on every input change: the harness has no {@code AnvilScreen}, which would also call
 * {@code rename} with the item's name and send {@code MC|ItemName} on every first-input change, so
 * the output differs from a real client's whenever names are involved; ghast does not model the
 * output, so the replay never compares it once an input changed); no server, no properties (brew time, fuel, enchantment
 * offers and the seed only drive the GUI). Contents are set slot by slot like {@code window_items}
 * does.
 */
final class MenuClickDump {
    private MenuClickDump() {}

    static final String[] MENUS = {"brewing", "enchanting", "anvil", "beacon", "merchant", "horse", "furnace"};

    /** Potion damage values (water, awkward, thick, mundane, a few effects, splash). */
    static final int[] POTION_DATA = {0, 16, 32, 64, 8193, 8194, 8197, 8201, 8225, 8257, 16385, 16388, 16417};

    /** Items that appear in brewing slots or are close to the rules (fish 349/350 meta 3). */
    static final int[] BREWING = {373, 373, 373, 374, 374, 289, 331, 348, 353, 370, 372, 375, 376, 377, 378, 382,
        396, 414, 349, 350, 351, 1, 3, 264};

    /** Enchantable items, books, dyes (damage 4 = lapis), a few others. */
    static final int[] ENCHANTING = {351, 351, 351, 276, 261, 310, 278, 340, 340, 403, 1, 3, 264, 35};

    /** Damaged tools and armor, their repair materials, enchanted books, a few others. */
    static final int[] ANVIL = {276, 276, 278, 310, 261, 359, 264, 264, 265, 266, 403, 403, 340, 1, 3, 35};

    /** Beacon payments (emerald, diamond, iron and gold ingot), near misses (nuggets, gold block), others. */
    static final int[] BEACON = {388, 388, 264, 264, 265, 266, 371, 41, 1, 3, 35, 276};

    /** Villager payments (emerald, wheat, paper, coal, iron, book, diamond sword, gold nugget). */
    static final int[] PAY = {388, 388, 296, 339, 263, 265, 340, 276, 371};

    /** Villager sales (emerald, bread, enchanted book, diamond sword, bow, filled map, book, diamond). */
    static final int[] SELL = {388, 297, 403, 276, 261, 358, 340, 264};

    /** Smelting inputs (ores, logs, sand, food, fish types, wet sponge), fuels (coal / charcoal, sticks, planks,
     * lava bucket, blaze rod, sapling, wooden tools, coal block), the bucket, outputs and others. */
    static final int[] FURNACE = {15, 14, 17, 12, 4, 363, 349, 19, 337, 81, 263, 263, 280, 5, 327, 325, 369, 6, 270,
        173, 265, 20, 1, 3};

    /** Saddles, horse armor, carpets, chests, horse food, a few others. */
    static final int[] HORSE = {329, 329, 417, 418, 419, 171, 54, 296, 322, 1, 3};

    /** The mount of the current horse case: type (0 horse, 1 donkey, 2 mule, 3 zombie, 4 skeleton) and chest. */
    private static int horseType;
    private static boolean horseChest;

    /** The trader of the current merchant case (the client's {@code Merchant}). */
    private static Merchant trader;

    /** All lines for {@code menu}: the facts line, then {@code count} cases. */
    static void cases(long seed, String menu, int count, Consumer<String> sink) {
        Harness.Run run = ClickDump.newRun();
        if (menu.equals("horse")) {
            // one facts line per mount (the slot rules ask the entity)
            for (int t = 0; t < 5; t++) {
                for (int c = 0; c < (t == 1 || t == 2 ? 2 : 1); c++) {
                    horseType = t;
                    horseChest = c == 1;
                    String f = facts(run, menu);
                    sink.accept(f.substring(0, f.length() - 1) + ",\"type\":" + t + ",\"chest\":" + horseChest + "}");
                }
            }
        } else if (menu.equals("furnace")) {
            // the furnace's own rules: the smelting inputs (SmeltingRecipeRegistry) besides the slots
            String f = facts(run, menu);
            StringBuilder b = new StringBuilder(f.substring(0, f.length() - 1)).append(",\"smeltable\":[");
            boolean first = true;
            for (int id = 0; id < 4096; id++) {
                Item item = Item.byRawId(id);
                if (item == null) {
                    continue;
                }
                int mask = 0;
                for (int d = 0; d < 16; d++) {
                    ItemStack r = SmeltingRecipeRegistry.getInstance().getResult(new ItemStack(item, 1, d));
                    if (!(r == null)) {
                        mask |= 1 << d;
                    }
                }
                if (mask != 0) {
                    b.append(first ? "" : ",").append('[').append(id).append(',').append(mask).append(']');
                    first = false;
                }
            }
            sink.accept(b.append("]}").toString());
        } else {
            sink.accept(facts(run, menu));
        }
        long salt = menu.hashCode() & 0xffffffffL;
        for (int i = 0; i < count; i++) {
            sink.accept(one(run, new Rng(seed * 1_000_037L + salt * 7919L + i), menu + "_s" + seed + "_" + i, menu));
        }
    }

    /** Every menu, {@code count} cases each. */
    static void all(long seed, int count, Consumer<String> sink) {
        for (String m : MENUS) {
            cases(seed, m, count, sink);
        }
    }

    private static ScreenHandler handler(Harness.Run run, String menu) {
        PlayerInventory inv = run.player.inventory;
        return switch (menu) {
            case "brewing" -> new BrewingScreenHandler(inv,
                    new ClientNetworkSyncedInventory("minecraft:brewing_stand", new LiteralText("b"), 4));
            case "enchanting" -> new EnchantingScreenHandler(inv, run.world);
            case "anvil" -> new AnvilScreenHandler(inv, run.world, run.player);
            case "beacon" -> new BeaconScreenHandler(inv,
                    new ClientNetworkSyncedInventory("minecraft:beacon", new LiteralText("b"), 1));
            case "furnace" -> new FurnaceScreenHandler(inv,
                    new ClientNetworkSyncedInventory("minecraft:furnace", new LiteralText("f"), 3));
            case "horse" -> {
                // ClientPlayNetworkHandler.onOpenScreen "EntityHorse": the entity's screen over an
                // AnimalInventory of the packet's slot count
                HorseBaseEntity horse = new HorseBaseEntity(run.world);
                horse.setType(horseType);
                horse.setHasChest(horseChest);
                yield new HorseScreenHandler(inv, new AnimalInventory("h", containerSlots(menu)), horse, run.player);
            }
            case "merchant" -> {
                // ClientPlayNetworkHandler.onOpenScreen: openTradingScreen(new Merchant(player, title))
                trader = new Merchant(run.player, new LiteralText("v"));
                yield new VillagerScreenHandler(inv, trader, run.world);
            }
            default -> throw new IllegalArgumentException("unknown menu " + menu);
        };
    }

    private static int containerSlots(String menu) {
        return switch (menu) {
            case "brewing" -> 4;
            case "enchanting" -> 2;
            case "anvil" -> 3;
            case "beacon" -> 1;
            case "merchant", "furnace" -> 3;
            case "horse" -> horseChest ? 17 : 2;
            default -> throw new IllegalArgumentException(menu);
        };
    }

    /**
     * {@code [[id,mask],...]}: the stacks (count 1, no NBT) slot {@code k} of the menu accepts, bit
     * {@code d} of {@code mask} set when damage {@code d} is accepted (ids without any are left out).
     */
    private static String accepts(ScreenHandler h, int k) {
        StringBuilder b = new StringBuilder("[");
        boolean first = true;
        for (int id = 0; id < 4096; id++) {
            Item item = Item.byRawId(id);
            if (item == null) {
                continue;
            }
            int mask = 0;
            for (int d = 0; d < 16; d++) {
                if (h.getSlot(k).canInsert(new ItemStack(item, 1, d))) {
                    mask |= 1 << d;
                }
            }
            if (mask != 0) {
                b.append(first ? "" : ",").append('[').append(id).append(',').append(mask).append(']');
                first = false;
            }
        }
        return b.append(']').toString();
    }

    private static String facts(Harness.Run run, String menu) {
        ScreenHandler h = handler(run, menu);
        StringBuilder b = new StringBuilder("{\"facts\":\"").append(menu).append('"');
        int n = containerSlots(menu);
        b.append(",\"accepts\":[");
        for (int k = 0; k < n; k++) {
            b.append(k > 0 ? "," : "").append(accepts(h, k));
        }
        b.append("],\"max\":[");
        for (int k = 0; k < n; k++) {
            b.append(k > 0 ? "," : "").append(h.getSlot(k).getMaxStackAmount());
        }
        return b.append("]}").toString();
    }

    private static ItemStack menuStack(Rng r, String menu) {
        if (menu.equals("furnace")) {
            int id = r.pick(FURNACE);
            ItemStack s = ClickDump.stack(r, new int[] {id});
            if (id == 263 || id == 19 || id == 17 || id == 5 || id == 6) {
                s.setDamage(r.nextInt(id == 263 || id == 19 ? 2 : 4));
            } else if (id == 349) {
                s.setDamage(r.nextInt(4));
            }
            return s;
        }
        if (menu.equals("horse")) {
            ItemStack s = ClickDump.stack(r, new int[] {r.pick(HORSE)});
            if (s.getItem() == Item.byRawId(171)) {
                s.setDamage(r.nextInt(16));
            }
            return s;
        }
        if (menu.equals("merchant")) {
            TraderOfferList offers = trader == null ? null : trader.getOffers(null);
            if (offers != null && !offers.isEmpty() && r.chance(0.7)) {
                TradeOffer o = offers.get(r.nextInt(offers.size()));
                ItemStack want = o.hasSecondStack() && r.chance(0.4) ? o.getSecondStack() : o.getFirstStack();
                ItemStack s = want.copy();
                s.count = r.chance(0.2) ? want.count : r.range(1, want.getMaxCount());
                if (s.hasNbt() && r.chance(0.2)) {
                    s.setNbt(null);
                } else if (r.chance(0.2)) {
                    s.setCustomName(r.chance(0.5) ? "a" : "b"); // an extra or a different name
                }
                return s;
            }
            return offerStack(r, PAY);
        }
        if (menu.equals("beacon")) {
            ItemStack s = ClickDump.stack(r, BEACON);
            if (r.chance(0.4)) {
                s.count = 1; // a single payment item is what shift-click moves into the slot
            }
            return s;
        }
        if (menu.equals("anvil")) {
            ItemStack s = ClickDump.stack(r, new int[] {r.pick(ANVIL)});
            if (r.chance(0.3)) {
                s.setCustomName(r.chance(0.5) ? "a" : "b");
            }
            return s;
        }
        if (menu.equals("enchanting")) {
            int id = r.pick(ENCHANTING);
            ItemStack s = ClickDump.stack(r, new int[] {id});
            if (id == 351 && r.chance(0.7)) {
                s.setDamage(4);
            }
            return s;
        }
        int id = r.pick(BREWING);
        ItemStack s = ClickDump.stack(r, new int[] {id});
        if (id == 373) {
            s.setDamage(r.pick(POTION_DATA));
        } else if ((id == 349 || id == 350) && r.chance(0.5)) {
            s.setDamage(3);
        }
        return s;
    }

    /** A villager offer's stack from {@code pool}: tools one at a time, maybe damaged or named. */
    private static ItemStack offerStack(Rng r, int[] pool) {
        int id = r.pick(pool);
        Item item = Item.byRawId(id);
        ItemStack s = new ItemStack(item, item.getMaxCount() == 1 ? 1 : r.range(1, 20), 0);
        if (item.getMaxDamage() > 0 && r.chance(0.3)) {
            s.setDamage(r.nextInt(4));
        }
        if (r.chance(0.2)) {
            s.setCustomName(r.chance(0.5) ? "a" : "b");
        }
        return s;
    }

    /** Random offers, sent like the server's {@code MC|TrList} (window id, then the list) and read back like the client does. */
    private static String offers(Rng r, int sync) {
        TraderOfferList list = new TraderOfferList();
        for (int k = r.range(1, 4); k > 0; k--) {
            ItemStack second = r.chance(0.4) ? offerStack(r, PAY) : null;
            int max = r.range(1, 6);
            list.add(new TradeOffer(offerStack(r, PAY), second, offerStack(r, SELL), r.range(0, max + 1), max));
        }
        PacketByteBuf buf = new PacketByteBuf(Unpooled.buffer());
        buf.writeInt(sync);
        list.toPacket(buf);
        byte[] bytes = new byte[buf.readableBytes()];
        buf.getBytes(0, bytes);
        PacketByteBuf in = new PacketByteBuf(Unpooled.wrappedBuffer(bytes));
        in.readInt();
        try {
            trader.setTraderOfferList(TraderOfferList.fromPacket(in));
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
        StringBuilder h = new StringBuilder(bytes.length * 2);
        for (byte x : bytes) {
            h.append(Character.forDigit((x >> 4) & 15, 16)).append(Character.forDigit(x & 15, 16));
        }
        return h.toString();
    }

    /** A payment for {@code want}: its item, damage and NBT, at least as many (up to a stack). */
    private static ItemStack pay(Rng r, ItemStack want) {
        ItemStack s = want.copy();
        s.count = Math.min(want.getMaxCount(), want.count * r.range(1, 3) + r.nextInt(3));
        return s;
    }

    /** The client's count of uses per offer. */
    private static void uses(StringBuilder b) {
        TraderOfferList list = trader.getOffers(null);
        if (list == null) {
            return;
        }
        b.append(",\"uses\":[");
        for (int k = 0; k < list.size(); k++) {
            b.append(k > 0 ? "," : "").append(list.get(k).getUses());
        }
        b.append(']');
    }

    private static ItemStack anyStack(Rng r, String menu) {
        return r.chance(0.6) ? menuStack(r, menu) : ClickDump.stack(r, ClickDump.ITEMS);
    }

    private static String one(Harness.Run run, Rng r, String name, String menu) {
        ClientPlayerEntity player = run.player;
        PlayerInventory inv = player.inventory;
        ScreenHandler w0 = player.playerScreenHandler;
        // Fresh player state (as ClickDump.one).
        player.openScreenHandler = w0;
        Reflect.set(ScreenHandler.class, w0, "actionId", (short) 0);
        inv.setCursorStack(null);
        boolean merchant = menu.equals("merchant");
        boolean known = true;
        if (menu.equals("horse")) {
            horseType = r.nextInt(5);
            horseChest = (horseType == 1 || horseType == 2) && r.chance(0.6);
            // whether the replay is told the mount's rules (else it predicts without them)
            known = r.chance(0.75);
        }
        int sync = 0;
        ScreenHandler h = null;
        String offers = null;
        if (merchant) {
            // the trader first: the player's items are drawn from its offers
            sync = r.range(1, 100);
            h = handler(run, menu);
            // the server's offers arrive before the payments are placed
            offers = r.chance(0.92) ? offers(r, sync) : null;
        }
        double fill = 0.2 + r.nextDouble() * 0.6;
        for (int k = 0; k < 36; k++) {
            inv.main[k] = r.chance(fill) ? anyStack(r, menu) : null;
        }
        for (int k = 0; k < 4; k++) {
            inv.armor[k] = r.chance(0.3) ? ClickDump.stack(r, new int[] {298, 307, 310, 317, 86, 397}) : null;
        }
        for (int k = 1; k <= 4; k++) {
            w0.getSlot(k).setStack(r.chance(0.3) ? ClickDump.stack(r, ClickDump.CRAFT) : null);
        }
        inv.selectedSlot = r.nextInt(9);

        if (!merchant) {
            sync = r.range(1, 100);
            h = handler(run, menu);
        }
        h.syncId = sync;
        int n = containerSlots(menu);
        double cfill = r.nextDouble();
        // the anvil's / villager's output is what the handler computes from the inputs (a server
        // sends the same; without offers the client keeps the server's)
        int filled = menu.equals("anvil") || offers != null ? 2 : n;
        if (offers != null && r.chance(0.5)) {
            // an offer's payments, enough of them (sometimes swapped)
            TraderOfferList list = trader.getOffers(null);
            TradeOffer o = list.get(r.nextInt(list.size()));
            ItemStack a = pay(r, o.getFirstStack());
            ItemStack b2 = o.hasSecondStack() ? pay(r, o.getSecondStack()) : null;
            boolean swap = r.chance(0.2);
            h.getSlot(0).setStack(swap ? b2 : a);
            h.getSlot(1).setStack(swap ? a : b2);
        } else {
            for (int k = 0; k < filled; k++) {
                h.getSlot(k).setStack(r.chance(cfill) ? menuStack(r, menu) : null);
            }
        }
        player.openScreenHandler = h;
        if (r.chance(0.4)) {
            inv.setCursorStack(anyStack(r, menu));
        }

        StringBuilder b = new StringBuilder(4096);
        b.append("{\"case\":\"").append(name).append("\",\"kind\":\"").append(menu).append("\",\"rows\":0")
                .append(",\"sync\":").append(sync).append(",\"selected\":").append(inv.selectedSlot);
        if (offers != null) {
            b.append(",\"offers\":\"").append(offers).append('"');
        }
        if (menu.equals("horse")) {
            b.append(",\"slots\":").append(n).append(",\"mount\":{\"type\":").append(horseType).append(",\"chest\":")
                    .append(horseChest).append(",\"known\":").append(known).append('}');
        }
        ClickDump.state(b, player, w0, h);
        if (merchant) {
            uses(b);
        }
        b.append(",\"clicks\":[");
        int clicks = r.range(1, 10);
        CapturingNetworkHandler net = run.net;
        for (int c = 0; c < clicks; c++) {
            if (offers != null && r.chance(0.15)) {
                // the trading screen's arrows: setRecipeIndex (+ MC|TrSel, not captured)
                int k = r.nextInt(trader.getOffers(null).size());
                ((VillagerScreenHandler) h).setRecipeIndex(k);
                b.append(c > 0 ? "," : "").append("{\"select\":").append(k);
                ClickDump.state(b, player, w0, h);
                uses(b);
                b.append('}');
                continue;
            }
            int len = h.slots.size();
            int mode = r.pick(new int[] {0, 0, 0, 1, 1, 2, 4, 6});
            int slot = merchant && r.chance(0.25) ? 2 : r.chance(0.45) ? r.nextInt(n) : r.nextInt(len);
            int button = r.nextInt(2);
            switch (mode) {
                case 0, 4 -> {
                    if (r.chance(0.06)) {
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
                    .append(",\"packet\":\"").append(ClickDump.packetHex(p)).append('"');
            ClickDump.state(b, player, w0, h);
            if (merchant) {
                uses(b);
            }
            b.append('}');
        }
        b.append("]}");
        player.openScreenHandler = w0;
        return b.toString();
    }
}
