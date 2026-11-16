package ghast.oracle.v1_16_5;

import ghast.oracle.Rng;
import net.minecraft.client.network.ClientPlayerEntity;
import net.minecraft.entity.player.PlayerInventory;
import net.minecraft.item.Item;
import net.minecraft.item.ItemStack;
import net.minecraft.network.Packet;
import net.minecraft.network.packet.c2s.play.ClickSlotC2SPacket;
import net.minecraft.screen.BrewingStandScreenHandler;
import net.minecraft.screen.EnchantmentScreenHandler;
import net.minecraft.screen.ScreenHandler;
import net.minecraft.screen.slot.SlotActionType;
import net.minecraft.util.Identifier;
import net.minecraft.util.registry.Registry;

import java.util.function.Consumer;

/**
 * Window click vectors for the special menus (brewing stand, ...): the same click path and line
 * format as {@link ClickDump} ({@code "kind"} is the menu name, {@code "menu"} its menu type id),
 * over contents biased to the items the menu's slot rules look at, with clicks biased to the menu's
 * own slots.
 *
 * <p>Each menu's set starts with one {@code {"facts":menu,...}} line: the slot rules read off the
 * real slots ({@code Slot.canInsert} over every item registry id; 1.16 stacks have no damage
 * variants, so an accepted id gets the mask of all 16 damage bits, as in the 1.8.9 / 1.12.2 dumps).
 *
 * <p>What is faked: the screen handler is built with the client constructor {@code ScreenHandlerType}
 * uses ({@code BrewingStandScreenHandler(syncId, playerInventory)}: a {@code SimpleInventory} and an
 * unsynced property delegate; {@code EnchantmentScreenHandler(syncId, playerInventory)}: the empty
 * client context); no server, no properties (brew time, fuel, enchantment offers and the seed only
 * drive the GUI).
 * Contents are set slot by slot like {@code window_items} does.
 */
final class MenuClickDump {
    private MenuClickDump() {}

    static final String[] MENUS = {"brewing", "enchanting"};

    /** Items that appear in brewing slots or are close to the rules. */
    static final String[] BREWING = {"potion", "potion", "splash_potion", "lingering_potion", "glass_bottle", "glass_bottle",
        "dragon_breath", "nether_wart", "redstone", "glowstone_dust", "gunpowder", "sugar", "ghast_tear", "spider_eye",
        "fermented_spider_eye", "blaze_powder", "blaze_powder", "magma_cream", "glistering_melon_slice", "golden_carrot",
        "rabbit_foot", "pufferfish", "turtle_helmet", "phantom_membrane", "cod", "stone", "dirt", "diamond"};

    /** Enchantable items, books, lapis, a dye, a few others. */
    static final String[] ENCHANTING = {"lapis_lazuli", "lapis_lazuli", "lapis_lazuli", "blue_dye", "diamond_sword", "bow",
        "diamond_helmet", "iron_pickaxe", "book", "book", "enchanted_book", "stone", "dirt", "diamond"};

    static void cases(long seed, String menu, int count, Consumer<String> sink) {
        Harness.Run run = ClickDump.newRun();
        sink.accept(facts(run, menu));
        long salt = menu.hashCode() & 0xffffffffL;
        for (int i = 0; i < count; i++) {
            sink.accept(one(run, new Rng(seed * 1_000_037L + salt * 7919L + i), menu + "_s" + seed + "_" + i, menu));
        }
    }

    static void all(long seed, int count, Consumer<String> sink) {
        for (String m : MENUS) {
            cases(seed, m, count, sink);
        }
    }

    private static int menuId(String menu) {
        return switch (menu) {
            case "brewing" -> 10;
            case "enchanting" -> 12;
            default -> throw new IllegalArgumentException(menu);
        };
    }

    private static ScreenHandler handler(int sync, PlayerInventory inv, String menu) {
        return switch (menu) {
            case "brewing" -> new BrewingStandScreenHandler(sync, inv);
            case "enchanting" -> new EnchantmentScreenHandler(sync, inv);
            default -> throw new IllegalArgumentException("unknown menu " + menu);
        };
    }

    private static String accepts(ScreenHandler h, int k) {
        StringBuilder b = new StringBuilder("[");
        boolean first = true;
        for (Item item : Registry.ITEM) {
            if (h.getSlot(k).canInsert(new ItemStack(item, 1))) {
                b.append(first ? "" : ",").append('[').append(Registry.ITEM.getRawId(item)).append(",65535]");
                first = false;
            }
        }
        return b.append(']').toString();
    }

    private static String facts(Harness.Run run, String menu) {
        ScreenHandler h = handler(1, run.player.inventory, menu);
        int n = h.slots.size() - 36;
        StringBuilder b = new StringBuilder("{\"facts\":\"").append(menu).append('"');
        b.append(",\"accepts\":[");
        for (int k = 0; k < n; k++) {
            b.append(k > 0 ? "," : "").append(accepts(h, k));
        }
        b.append("],\"max\":[");
        for (int k = 0; k < n; k++) {
            b.append(k > 0 ? "," : "").append(h.getSlot(k).getMaxItemCount());
        }
        return b.append("]}").toString();
    }

    private static ItemStack menuStack(Rng r, String menu) {
        if (menu.equals("enchanting")) {
            return ClickDump.stack(r, ENCHANTING);
        }
        ItemStack s = ClickDump.stack(r, BREWING);
        String id = Registry.ITEM.getId(s.getItem()).getPath();
        if (id.endsWith("potion") && r.chance(0.6)) {
            s.getOrCreateTag().putString("Potion", r.pick(new String[] {"minecraft:water", "minecraft:awkward", "minecraft:swiftness"}));
        }
        return s;
    }

    private static ItemStack anyStack(Rng r, String menu) {
        return r.chance(0.6) ? menuStack(r, menu) : ClickDump.stack(r, ClickDump.ITEMS);
    }

    private static String one(Harness.Run run, Rng r, String name, String menu) {
        ClientPlayerEntity player = run.player;
        PlayerInventory inv = player.inventory;
        ScreenHandler w0 = player.playerScreenHandler;
        player.currentScreenHandler = w0;
        Reflect.set(ScreenHandler.class, w0, "actionId", (short) 0);
        inv.setCursorStack(ItemStack.EMPTY);
        double fill = 0.2 + r.nextDouble() * 0.6;
        for (int k = 0; k < 36; k++) {
            inv.main.set(k, r.chance(fill) ? anyStack(r, menu) : ItemStack.EMPTY);
        }
        for (int k = 0; k < 4; k++) {
            inv.armor.set(k, r.chance(0.3) ? ClickDump.stack(r, ClickDump.ARMOR) : ItemStack.EMPTY);
        }
        inv.offHand.set(0, r.chance(0.3) ? anyStack(r, menu) : ItemStack.EMPTY);
        for (int k = 0; k <= 4; k++) {
            w0.getSlot(k).setStack(k > 0 && r.chance(0.3) ? ClickDump.stack(r, ClickDump.CRAFT) : ItemStack.EMPTY);
        }
        inv.selectedSlot = r.nextInt(9);

        int sync = r.range(1, 100);
        ScreenHandler h = handler(sync, inv, menu);
        int n = h.slots.size() - 36;
        double cfill = r.nextDouble();
        for (int k = 0; k < n; k++) {
            h.getSlot(k).setStack(r.chance(cfill) ? menuStack(r, menu) : ItemStack.EMPTY);
        }
        player.currentScreenHandler = h;
        if (r.chance(0.4)) {
            inv.setCursorStack(anyStack(r, menu));
        }

        StringBuilder b = new StringBuilder(4096);
        b.append("{\"case\":\"").append(name).append("\",\"kind\":\"").append(menu).append("\",\"menu\":").append(menuId(menu))
                .append(",\"sync\":").append(sync).append(",\"selected\":").append(inv.selectedSlot);
        ClickDump.state(b, player, w0, h);
        b.append(",\"clicks\":[");
        int clicks = r.range(1, 10);
        SlotActionType[] types = SlotActionType.values();
        for (int c = 0; c < clicks; c++) {
            int len = h.slots.size();
            int mode = r.pick(new int[] {0, 0, 0, 1, 1, 2, 4, 6});
            int slot = r.chance(0.45) ? r.nextInt(n) : r.nextInt(len);
            int button = r.nextInt(2);
            switch (mode) {
                case 0, 4 -> {
                    if (r.chance(0.06)) {
                        slot = -999;
                    }
                }
                case 2 -> button = r.chance(0.15) ? 40 : r.nextInt(9);
                case 6 -> button = 0;
                default -> {}
            }
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
                    .append(",\"packet\":\"").append(ClickDump.packetHex(p)).append('"');
            ClickDump.state(b, player, w0, h);
            b.append('}');
        }
        b.append("]}");
        player.currentScreenHandler = w0;
        return b.toString();
    }
}
