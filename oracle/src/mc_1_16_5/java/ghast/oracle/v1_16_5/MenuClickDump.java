package ghast.oracle.v1_16_5;

import ghast.oracle.Rng;
import io.netty.buffer.Unpooled;
import net.minecraft.client.network.ClientPlayerEntity;
import net.minecraft.entity.EntityType;
import net.minecraft.entity.passive.AbstractDonkeyEntity;
import net.minecraft.entity.passive.HorseBaseEntity;
import net.minecraft.entity.passive.LlamaEntity;
import net.minecraft.inventory.SimpleInventory;
import net.minecraft.entity.player.PlayerInventory;
import net.minecraft.item.Item;
import net.minecraft.item.ItemStack;
import net.minecraft.nbt.NbtCompound;
import net.minecraft.network.Packet;
import net.minecraft.network.PacketByteBuf;
import net.minecraft.network.packet.s2c.play.SetTradeOffersS2CPacket;
import net.minecraft.network.packet.c2s.play.ClickSlotC2SPacket;
import net.minecraft.screen.AnvilScreenHandler;
import net.minecraft.screen.BeaconScreenHandler;
import net.minecraft.screen.BrewingStandScreenHandler;
import net.minecraft.screen.BlastFurnaceScreenHandler;
import net.minecraft.screen.CartographyTableScreenHandler;
import net.minecraft.screen.EnchantmentScreenHandler;
import net.minecraft.screen.GrindstoneScreenHandler;
import net.minecraft.screen.FurnaceScreenHandler;
import net.minecraft.screen.SmithingScreenHandler;
import net.minecraft.screen.SmokerScreenHandler;
import net.minecraft.screen.StonecutterScreenHandler;
import net.minecraft.screen.HorseScreenHandler;
import net.minecraft.screen.LoomScreenHandler;
import net.minecraft.screen.MerchantScreenHandler;
import net.minecraft.screen.ScreenHandler;
import net.minecraft.screen.slot.SlotActionType;
import net.minecraft.util.Identifier;
import net.minecraft.util.registry.Registry;
import net.minecraft.village.TradeOffer;
import net.minecraft.village.TradeOfferList;

import java.io.IOException;
import java.io.UncheckedIOException;
import java.util.function.Consumer;

/**
 * Window click vectors for the special menus (brewing stand, enchanting table, anvil, beacon): the same click path and line
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
 * client context; {@code AnvilScreenHandler(syncId, playerInventory)}, whose output the handler
 * computes on every input change: the harness has no {@code AnvilScreen}, which would also call
 * {@code setNewItemName} and send {@code rename_item} on every first-input change; ghast does not
 * model the output, so the replay never compares it once an input changed;
 * {@code BeaconScreenHandler(syncId, playerInventory)}: its own one-item payment inventory, tags as
 * the harness loads them from the jar's vanilla data); no server, no properties (brew time, fuel, enchantment offers and the seed only
 * drive the GUI).
 * Contents are set slot by slot like {@code window_items} does.
 */
final class MenuClickDump {
    private MenuClickDump() {}

    static final String[] MENUS = {"brewing", "enchanting", "anvil", "beacon", "merchant", "horse", "furnace", "blast_furnace", "smoker", "smithing", "stonecutter", "grindstone", "loom", "cartography",
        "beacon_datapack", "furnace_datapack", "horse_datapack"};

    /** Items that appear in brewing slots or are close to the rules. */
    static final String[] BREWING = {"potion", "potion", "splash_potion", "lingering_potion", "glass_bottle", "glass_bottle",
        "dragon_breath", "nether_wart", "redstone", "glowstone_dust", "gunpowder", "sugar", "ghast_tear", "spider_eye",
        "fermented_spider_eye", "blaze_powder", "blaze_powder", "magma_cream", "glistering_melon_slice", "golden_carrot",
        "rabbit_foot", "pufferfish", "turtle_helmet", "phantom_membrane", "cod", "stone", "dirt", "diamond"};

    /** Enchantable items, books, lapis, a dye, a few others. */
    static final String[] ENCHANTING = {"lapis_lazuli", "lapis_lazuli", "lapis_lazuli", "blue_dye", "diamond_sword", "bow",
        "diamond_helmet", "iron_pickaxe", "book", "book", "enchanted_book", "stone", "dirt", "diamond"};

    /** Damaged tools and armor, their repair materials, enchanted books, a few others. */
    static final String[] ANVIL = {"diamond_sword", "diamond_sword", "iron_pickaxe", "diamond_helmet", "bow", "shears",
        "diamond", "diamond", "iron_ingot", "gold_ingot", "enchanted_book", "enchanted_book", "book", "stone", "dirt",
        "white_wool"};

    /** Beacon payments (the vanilla tag), near misses (nuggets, blocks), others. */
    static final String[] BEACON = {"emerald", "emerald", "diamond", "diamond", "iron_ingot", "gold_ingot",
        "netherite_ingot", "gold_nugget", "netherite_scrap", "gold_block", "stone", "dirt", "white_wool", "diamond_sword"};

    /** Villager payments. */
    static final String[] PAY = {"emerald", "emerald", "wheat", "paper", "coal", "iron_ingot", "book", "diamond_sword",
        "gold_nugget"};

    /** Villager sales. */
    static final String[] SELL = {"emerald", "bread", "enchanted_book", "diamond_sword", "bow", "filled_map", "book",
        "diamond"};

    /** Smelting / blasting / smoking inputs, fuels, the bucket, outputs and others. */
    static final String[] FURNACE = {"iron_ore", "gold_ore", "oak_log", "sand", "cobblestone", "beef", "cod", "potato",
        "wet_sponge", "clay_ball", "cactus", "iron_sword", "coal", "charcoal", "stick", "oak_planks", "lava_bucket",
        "bucket", "blaze_rod", "oak_sapling", "wooden_pickaxe", "coal_block", "bamboo", "iron_ingot", "glass", "stone",
        "dirt"};

    /** Saddles, horse armor, carpets, chests, horse food, a few others. */
    static final String[] HORSE = {"saddle", "saddle", "leather_horse_armor", "iron_horse_armor", "golden_horse_armor",
        "diamond_horse_armor", "white_carpet", "red_carpet", "chest", "wheat", "golden_apple", "white_wool", "stone",
        "dirt"};

    /** The mounts the screen exists for. */
    static final String[] MOUNTS = {"horse", "donkey", "mule", "zombie_horse", "skeleton_horse", "llama", "trader_llama"};

    /** The mount of the current horse case: entity, chest, llama strength, tame, baby. */
    private static String mount = "horse";
    private static boolean mountChest;
    private static int mountStrength = 3;
    private static boolean mountTame = true;
    private static boolean mountBaby;

    /** Smithing: diamond gear (damaged, named, enchanted), netherite ingots, near misses. */
    static final String[] SMITHING = {"diamond_sword", "diamond_pickaxe", "diamond_helmet", "diamond_chestplate",
        "diamond_hoe", "netherite_ingot", "netherite_ingot", "netherite_ingot", "iron_sword", "netherite_scrap",
        "netherite_sword", "diamond", "stone"};

    /** Stonecutter inputs (with and without recipes) and a few others. */
    static final String[] STONECUTTER = {"stone", "stone", "cobblestone", "andesite", "granite", "sandstone",
        "quartz_block", "stone_bricks", "blackstone", "oak_planks", "dirt", "diamond"};

    /** Grindstone: damageable gear, enchanted books, books, a few others. */
    static final String[] GRINDSTONE = {"diamond_sword", "diamond_sword", "iron_pickaxe", "bow", "enchanted_book",
        "enchanted_book", "enchanted_book", "book", "golden_apple", "stone", "stick"};

    /** Loom: banners, dyes, pattern items, a few others. */
    static final String[] LOOM = {"white_banner", "white_banner", "red_banner", "black_banner", "red_dye", "blue_dye",
        "black_dye", "white_dye", "creeper_banner_pattern", "flower_banner_pattern", "globe_banner_pattern",
        "mojang_banner_pattern", "white_wool", "stone"};

    /** Cartography table: maps (some with the server's scale / lock flags), paper, glass panes. */
    static final String[] CARTOGRAPHY = {"filled_map", "filled_map", "filled_map", "paper", "map", "glass_pane",
        "stone", "compass"};

    /** Enchantments for the grindstone cases (incl. both curses). */
    static final net.minecraft.enchantment.Enchantment[] GRIND_ENCH = {
        net.minecraft.enchantment.Enchantments.UNBREAKING, net.minecraft.enchantment.Enchantments.SHARPNESS,
        net.minecraft.enchantment.Enchantments.MENDING, net.minecraft.enchantment.Enchantments.BINDING_CURSE,
        net.minecraft.enchantment.Enchantments.VANISHING_CURSE, net.minecraft.enchantment.Enchantments.EFFICIENCY};

    /** Anvil: enchanted and damaged gear, books, repair materials, a few others. */
    static final String[] ANVIL16 = {"diamond_sword", "diamond_sword", "iron_pickaxe", "diamond_helmet", "bow", "shears",
        "elytra", "diamond", "diamond", "iron_ingot", "phantom_membrane", "enchanted_book", "enchanted_book",
        "enchanted_book", "book", "stone", "oak_planks", "wooden_sword", "trident"};

    /** Enchantments for the anvil cases (incl. incompatible pairs and curses). */
    static final net.minecraft.enchantment.Enchantment[] ANVIL_ENCH = {
        net.minecraft.enchantment.Enchantments.SHARPNESS, net.minecraft.enchantment.Enchantments.SMITE,
        net.minecraft.enchantment.Enchantments.UNBREAKING, net.minecraft.enchantment.Enchantments.MENDING,
        net.minecraft.enchantment.Enchantments.INFINITY, net.minecraft.enchantment.Enchantments.PROTECTION,
        net.minecraft.enchantment.Enchantments.FIRE_PROTECTION, net.minecraft.enchantment.Enchantments.EFFICIENCY,
        net.minecraft.enchantment.Enchantments.SILK_TOUCH, net.minecraft.enchantment.Enchantments.FORTUNE,
        net.minecraft.enchantment.Enchantments.POWER, net.minecraft.enchantment.Enchantments.BINDING_CURSE,
        net.minecraft.enchantment.Enchantments.LOYALTY, net.minecraft.enchantment.Enchantments.RIPTIDE};

    /** Whether the emulated anvil screen's name field takes typing (it had an item last). */
    private static boolean anvilEditable;

    /** The rename packets the emulated anvil screen sent during the current step. */
    private static final java.util.List<String> renames = new java.util.ArrayList<>();

    /**
     * What {@code AnvilScreen} does on the client, faked without rendering: it listens to the
     * handler and, when slot 0 changes, puts the item's name into its name field, whose change
     * listener ({@code onRenamed}) sets the handler's new item name and sends {@code rename_item}.
     */
    private static void anvilScreen(AnvilScreenHandler h) {
        net.minecraft.screen.ScreenHandlerListener l = new net.minecraft.screen.ScreenHandlerListener() {
            @Override
            public void onHandlerRegistered(ScreenHandler handler, net.minecraft.util.collection.DefaultedList<ItemStack> stacks) {}

            @Override
            public void onSlotUpdate(ScreenHandler handler, int slotId, ItemStack stack) {
                if (slotId == 0) {
                    // setText, then setEditable(!stack.isEmpty())
                    anvilRenamed(h, stack.isEmpty() ? "" : stack.getName().getString());
                    anvilEditable = !stack.isEmpty();
                }
            }

            @Override
            public void onPropertyUpdate(ScreenHandler handler, int property, int value) {}
        };
        anvilEditable = false;
        h.addListener(l);
        // ForgingScreen.init
        l.onSlotUpdate(h, 0, h.getSlot(0).getStack());
    }

    /** {@code AnvilScreen.onRenamed} (the name field's change listener). */
    private static void anvilRenamed(AnvilScreenHandler h, String name) {
        if (!name.isEmpty()) {
            String string = name;
            net.minecraft.screen.slot.Slot slot = h.getSlot(0);
            if (slot != null && slot.hasStack() && !slot.getStack().hasCustomName()
                    && name.equals(slot.getStack().getName().getString())) {
                string = "";
            }
            h.setNewItemName(string);
            renames.add(string);
        }
    }

    /** The anvil's client state after a step: rename packets sent, level, cost. */
    private static void anvilState(StringBuilder b, AnvilScreenHandler h, ClientPlayerEntity player) {
        b.append(",\"renames\":[");
        for (int k = 0; k < renames.size(); k++) {
            b.append(k > 0 ? "," : "").append(new com.google.gson.JsonPrimitive(renames.get(k)).toString());
        }
        b.append("],\"level\":").append(player.experienceLevel).append(",\"cost\":").append(h.getLevelCost());
        renames.clear();
    }

    /** The output that already got the recipes line. */
    private static Consumer<String> recipesSentTo;

    /** The handler of the current merchant case. */
    private static MerchantScreenHandler trader;

    /** The {@code tags} packet of a tag manager (what the server sends; the client applies it). */
    private static String tagsHex(net.minecraft.tag.TagManager tm) {
        PacketByteBuf buf = new PacketByteBuf(Unpooled.buffer());
        try {
            new net.minecraft.network.packet.s2c.play.SynchronizeTagsS2CPacket(tm).write(buf);
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
        StringBuilder h = new StringBuilder(buf.readableBytes() * 2);
        while (buf.isReadable()) {
            int x = buf.readUnsignedByte();
            h.append(Character.forDigit(x >> 4, 16)).append(Character.forDigit(x & 15, 16));
        }
        return h.toString();
    }

    /**
     * A data pack's item tags: beacon payments stone, dirt and emerald only; carpets stone and the
     * white carpet (fuel, llama armor); planks cobblestone and oak planks (fuel).
     */
    private static net.minecraft.tag.TagManager datapackTags() {
        net.minecraft.tag.TagManager v = Harness.vanillaTags();
        java.util.Map<Identifier, net.minecraft.tag.Tag<Item>> m = new java.util.HashMap<>(v.getItems().getTags());
        m.put(new Identifier("beacon_payment_items"), net.minecraft.tag.Tag.of(java.util.Set.of(
                net.minecraft.item.Items.STONE, net.minecraft.item.Items.DIRT, net.minecraft.item.Items.EMERALD)));
        m.put(new Identifier("carpets"), net.minecraft.tag.Tag.of(java.util.Set.of(
                net.minecraft.item.Items.STONE, net.minecraft.item.Items.WHITE_CARPET)));
        m.put(new Identifier("planks"), net.minecraft.tag.Tag.of(java.util.Set.of(
                net.minecraft.item.Items.COBBLESTONE, net.minecraft.item.Items.OAK_PLANKS)));
        return net.minecraft.tag.TagManager.create(v.getBlocks(), net.minecraft.tag.TagGroup.create(m), v.getFluids(),
                v.getEntityTypes());
    }

    static void cases(long seed, String menu, int count, Consumer<String> sink) {
        if (menu.endsWith("_datapack")) {
            // the same menu under a data pack's tags (the client reads the server's tags)
            String base = menu.substring(0, menu.length() - "_datapack".length());
            Harness.Run run = ClickDump.newRun();
            net.minecraft.tag.TagManager tm = datapackTags();
            tm.apply();
            try {
                if (base.equals("furnace") && sink != recipesSentTo) {
                    sink.accept("{\"declare_recipes\":\"" + ClickDump.syncRecipes() + "\"}");
                    recipesSentTo = sink;
                }
                sink.accept("{\"tags\":\"" + tagsHex(tm) + "\"}");
                long salt = menu.hashCode() & 0xffffffffL;
                for (int i = 0; i < count; i++) {
                    sink.accept(one(run, new Rng(seed * 1_000_037L + salt * 7919L + i), menu + "_s" + seed + "_" + i, base));
                }
            } finally {
                Harness.vanillaTags().apply();
            }
            return;
        }
        Harness.Run run = ClickDump.newRun();
        if (menu.equals("beacon") || menu.endsWith("furnace") || menu.equals("smoker") || menu.equals("horse")) {
            // the server's tags (vanilla here): beacon payments, fuels, llama carpets read them
            sink.accept("{\"tags\":\"" + tagsHex(Harness.vanillaTags()) + "\"}");
        }
        if ((menu.endsWith("furnace") || menu.equals("smoker") || menu.equals("smithing")
                || menu.equals("stonecutter")) && sink != recipesSentTo) {
            // the server's recipes (a real client always has them; isSmeltable reads them), once
            // per output
            sink.accept("{\"declare_recipes\":\"" + ClickDump.syncRecipes() + "\"}");
            recipesSentTo = sink;
        }
        if (menu.equals("anvil")) {
            // what updateResult reads: every enchantment's max level, rarity, incompatibilities
            // and acceptable items; every damageable item's repair materials
            String f = facts(run, menu);
            StringBuilder b = new StringBuilder(f.substring(0, f.length() - 1)).append(",\"enchantments\":[");
            boolean first = true;
            for (net.minecraft.enchantment.Enchantment e : Registry.ENCHANTMENT) {
                b.append(first ? "" : ",").append("[\"").append(Registry.ENCHANTMENT.getId(e)).append("\",")
                        .append(e.getMaxLevel()).append(",\"").append(e.getRarity()).append("\",[");
                boolean f2 = true;
                for (net.minecraft.enchantment.Enchantment o : Registry.ENCHANTMENT) {
                    if (!e.canCombine(o)) {
                        b.append(f2 ? "" : ",").append(Registry.ENCHANTMENT.getRawId(o));
                        f2 = false;
                    }
                }
                b.append("],[");
                f2 = true;
                for (Item item : Registry.ITEM) {
                    if (e.isAcceptableItem(new ItemStack(item))) {
                        b.append(f2 ? "" : ",").append(Registry.ITEM.getRawId(item));
                        f2 = false;
                    }
                }
                b.append("]]");
                first = false;
            }
            b.append("],\"repair\":[");
            first = true;
            for (Item item : Registry.ITEM) {
                if (item.getMaxDamage() <= 0) {
                    continue;
                }
                b.append(first ? "" : ",").append("[").append(Registry.ITEM.getRawId(item)).append(",[");
                boolean f2 = true;
                for (Item m : Registry.ITEM) {
                    if (item.canRepair(new ItemStack(item), new ItemStack(m))) {
                        b.append(f2 ? "" : ",").append(Registry.ITEM.getRawId(m));
                        f2 = false;
                    }
                }
                b.append("]]");
                first = false;
            }
            sink.accept(b.append("]}").toString());
        } else if (menu.equals("loom")) {
            // the loom's data: patterns (id, has an item), banner / dye colours, pattern items
            String f = facts(run, menu);
            StringBuilder b = new StringBuilder(f.substring(0, f.length() - 1)).append(",\"patterns\":[");
            net.minecraft.block.entity.BannerPattern[] ps = net.minecraft.block.entity.BannerPattern.values();
            for (int k = 0; k < ps.length; k++) {
                b.append(k > 0 ? "," : "").append("[\"").append(ps[k].getId()).append("\",")
                        .append(k >= net.minecraft.block.entity.BannerPattern.COUNT
                                - net.minecraft.block.entity.BannerPattern.field_24417).append("]");
            }
            b.append("],\"items\":[");
            boolean first = true;
            for (Item item : Registry.ITEM) {
                String what = null;
                int value = 0;
                if (item instanceof net.minecraft.item.BannerItem bi) {
                    what = "banner";
                    value = bi.getColor().getId();
                } else if (item instanceof net.minecraft.item.DyeItem di) {
                    what = "dye";
                    value = di.getColor().getId();
                } else if (item instanceof net.minecraft.item.BannerPatternItem pi) {
                    what = "pattern";
                    value = pi.getPattern().ordinal();
                }
                if (what != null) {
                    b.append(first ? "" : ",").append("[").append(Registry.ITEM.getRawId(item)).append(",\"").append(what)
                            .append("\",").append(value).append("]");
                    first = false;
                }
            }
            sink.accept(b.append("]}").toString());
        } else if (menu.equals("grindstone")) {
            // the enchantment registry: names and curses (the result keeps only curses)
            String f = facts(run, menu);
            StringBuilder b = new StringBuilder(f.substring(0, f.length() - 1)).append(",\"enchantments\":[");
            boolean first = true;
            for (net.minecraft.enchantment.Enchantment e : Registry.ENCHANTMENT) {
                b.append(first ? "" : ",").append("[\"").append(Registry.ENCHANTMENT.getId(e)).append("\",")
                        .append(e.isCursed()).append("]");
                first = false;
            }
            sink.accept(b.append("]}").toString());
        } else if (menu.equals("stonecutter")) {
            // the recipe list is sorted by the outputs' translation keys: every item's key
            String f = facts(run, menu);
            StringBuilder b = new StringBuilder(f.substring(0, f.length() - 1)).append(",\"keys\":[");
            boolean first = true;
            for (Item item : Registry.ITEM) {
                b.append(first ? "" : ",").append("[").append(Registry.ITEM.getRawId(item)).append(",\"")
                        .append(item.getTranslationKey()).append("\"]");
                first = false;
            }
            sink.accept(b.append("]}").toString());
        } else if (menu.equals("horse")) {
            // one facts line per mount, tame and baby (the slot rules ask the entity)
            for (String m : MOUNTS) {
                for (int t = 0; t < 4; t++) {
                    mount = m;
                    mountChest = false;
                    mountTame = (t & 1) != 0;
                    mountBaby = (t & 2) != 0;
                    String f = facts(run, menu);
                    sink.accept(f.substring(0, f.length() - 1) + ",\"entity\":\"" + m + "\",\"tame\":" + mountTame
                            + ",\"baby\":" + mountBaby + "}");
                }
            }
        } else {
            sink.accept(facts(run, menu));
        }
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
            case "anvil" -> 7;
            case "beacon" -> 8;
            case "merchant" -> 18;
            case "horse" -> -1; // open_horse_window, not a menu type
            case "furnace" -> 13;
            case "blast_furnace" -> 9;
            case "smoker" -> 21;
            case "smithing" -> 20;
            case "stonecutter" -> 23;
            case "grindstone" -> 14;
            case "loom" -> 17;
            case "cartography" -> 22;
            default -> throw new IllegalArgumentException(menu);
        };
    }

    private static ScreenHandler handler(int sync, PlayerInventory inv, String menu) {
        return switch (menu) {
            case "brewing" -> new BrewingStandScreenHandler(sync, inv);
            case "enchanting" -> new EnchantmentScreenHandler(sync, inv);
            case "anvil" -> new AnvilScreenHandler(sync, inv);
            case "beacon" -> new BeaconScreenHandler(sync, inv);
            // the client constructors; isSmeltable asks the client's recipe manager (the server's recipes)
            case "furnace" -> new FurnaceScreenHandler(sync, inv);
            case "blast_furnace" -> new BlastFurnaceScreenHandler(sync, inv);
            case "smoker" -> new SmokerScreenHandler(sync, inv);
            case "smithing" -> new SmithingScreenHandler(sync, inv);
            case "stonecutter" -> new StonecutterScreenHandler(sync, inv);
            case "grindstone" -> new GrindstoneScreenHandler(sync, inv);
            case "loom" -> new LoomScreenHandler(sync, inv);
            // the client constructor: no world context, so the client never computes a result
            case "cartography" -> new CartographyTableScreenHandler(sync, inv);
            case "horse" -> {
                // ClientPlayNetworkHandler.onOpenHorseScreen: the entity's screen over a
                // SimpleInventory of the packet's slot count
                net.minecraft.world.World w = inv.player.world;
                HorseBaseEntity e = switch (mount) {
                    case "horse" -> new net.minecraft.entity.passive.HorseEntity(EntityType.HORSE, w);
                    case "donkey" -> new net.minecraft.entity.passive.DonkeyEntity(EntityType.DONKEY, w);
                    case "mule" -> new net.minecraft.entity.passive.MuleEntity(EntityType.MULE, w);
                    case "zombie_horse" -> new net.minecraft.entity.mob.ZombieHorseEntity(EntityType.ZOMBIE_HORSE, w);
                    case "skeleton_horse" -> new net.minecraft.entity.mob.SkeletonHorseEntity(EntityType.SKELETON_HORSE, w);
                    case "trader_llama" -> new net.minecraft.entity.passive.TraderLlamaEntity(EntityType.TRADER_LLAMA, w);
                    default -> new LlamaEntity(EntityType.LLAMA, w);
                };
                if (e instanceof LlamaEntity l) {
                    try {
                        java.lang.reflect.Method m = LlamaEntity.class.getDeclaredMethod("setStrength", int.class);
                        m.setAccessible(true);
                        m.invoke(l, mountStrength);
                    } catch (ReflectiveOperationException x) {
                        throw new IllegalStateException(x);
                    }
                }
                if (e instanceof AbstractDonkeyEntity d) {
                    d.setHasChest(mountChest);
                }
                e.setTame(mountTame);
                e.setBaby(mountBaby);
                int slots = mountChest ? 2 + 3 * (e instanceof LlamaEntity ? mountStrength : 5) : 2;
                yield new HorseScreenHandler(sync, inv, new SimpleInventory(slots), e);
            }
            // the client constructor: a SimpleMerchant whose offers the server sends
            case "merchant" -> trader = new MerchantScreenHandler(sync, inv);
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
        if (menu.equals("smithing")) {
            ItemStack s = ClickDump.stack(r, SMITHING);
            if (s.isDamageable() && r.chance(0.3)) {
                s.addEnchantment(net.minecraft.enchantment.Enchantments.UNBREAKING, 2);
            }
            return s;
        }
        if (menu.equals("stonecutter")) {
            return ClickDump.stack(r, STONECUTTER);
        }
        if (menu.equals("cartography")) {
            ItemStack s = ClickDump.stack(r, CARTOGRAPHY);
            if (s.getItem() == net.minecraft.item.Items.FILLED_MAP) {
                s.getOrCreateTag().putInt("map", r.nextInt(3));
                if (r.chance(0.15)) {
                    s.getOrCreateTag().putInt("map_scale_direction", 1);
                } else if (r.chance(0.15)) {
                    s.getOrCreateTag().putBoolean("map_to_lock", true);
                }
            }
            return s;
        }
        if (menu.equals("loom")) {
            ItemStack s = ClickDump.stack(r, LOOM);
            if (s.getItem() instanceof net.minecraft.item.BannerItem && r.chance(0.5)) {
                // a banner with patterns already (up to 7)
                NbtCompound be = s.getOrCreateSubTag("BlockEntityTag");
                net.minecraft.nbt.NbtList l = new net.minecraft.nbt.NbtList();
                for (int k = r.nextInt(8); k > 0; k--) {
                    NbtCompound p = new NbtCompound();
                    p.putString("Pattern", r.pick(new String[] {"bl", "cr", "mc", "glb"}));
                    p.putInt("Color", r.nextInt(16));
                    l.add(p);
                }
                be.put("Patterns", l);
            }
            return s;
        }
        if (menu.equals("grindstone")) {
            ItemStack s = ClickDump.stack(r, GRINDSTONE);
            if (r.chance(0.3)) {
                s.setCount(1); // the grindstone only takes single items
            }
            boolean book = s.getItem() == net.minecraft.item.Items.ENCHANTED_BOOK;
            for (int k = r.nextInt(book ? 3 : 4); k > 0; k--) {
                net.minecraft.enchantment.Enchantment e = r.pick(GRIND_ENCH);
                int lvl = r.range(1, 3);
                if (book) {
                    net.minecraft.item.EnchantedBookItem.addEnchantment(s,
                            new net.minecraft.enchantment.EnchantmentLevelEntry(e, lvl));
                } else {
                    s.addEnchantment(e, lvl);
                }
            }
            if (r.chance(0.1)) {
                s.setRepairCost(r.range(1, 7));
            }
            return s;
        }
        if (menu.equals("furnace") || menu.equals("blast_furnace") || menu.equals("smoker")) {
            return ClickDump.stack(r, FURNACE);
        }
        if (menu.equals("horse")) {
            return ClickDump.stack(r, HORSE);
        }
        if (menu.equals("merchant")) {
            TradeOfferList offers = trader == null ? null : trader.getRecipes();
            if (offers != null && !offers.isEmpty() && r.chance(0.7)) {
                TradeOffer o = offers.get(r.nextInt(offers.size()));
                ItemStack want = !o.getSecondBuyItem().isEmpty() && r.chance(0.4) ? o.getSecondBuyItem()
                        : o.getAdjustedFirstBuyItem();
                ItemStack s = want.copy();
                s.setCount(r.chance(0.2) ? want.getCount() : r.range(1, want.getMaxCount()));
                if (s.hasTag() && r.chance(0.2)) {
                    s.setTag(null);
                } else if (r.chance(0.2)) {
                    s.setCustomName(new net.minecraft.text.LiteralText(r.chance(0.5) ? "a" : "b"));
                }
                return s;
            }
            return offerStack(r, PAY);
        }
        if (menu.equals("beacon")) {
            ItemStack s = ClickDump.stack(r, BEACON);
            if (r.chance(0.4)) {
                s.setCount(1); // a single payment item is what shift-click moves into the slot
            }
            return s;
        }
        if (menu.equals("enchanting")) {
            return ClickDump.stack(r, ENCHANTING);
        }
        if (menu.equals("anvil")) {
            ItemStack s = ClickDump.stack(r, ANVIL16);
            boolean book = s.getItem() == net.minecraft.item.Items.ENCHANTED_BOOK;
            for (int k = r.nextInt(book ? 3 : 4); k > 0; k--) {
                net.minecraft.enchantment.Enchantment e = r.pick(ANVIL_ENCH);
                int lvl = r.range(1, 6);
                if (book) {
                    net.minecraft.item.EnchantedBookItem.addEnchantment(s,
                            new net.minecraft.enchantment.EnchantmentLevelEntry(e, lvl));
                } else {
                    s.addEnchantment(e, lvl);
                }
            }
            if (r.chance(0.3)) {
                s.setCustomName(new net.minecraft.text.LiteralText(r.chance(0.5) ? "a" : "b"));
            }
            if (r.chance(0.2)) {
                s.setRepairCost(r.range(0, 40));
            }
            if (r.chance(0.3)) {
                s.setCount(1);
            }
            return s;
        }
        ItemStack s = ClickDump.stack(r, BREWING);
        String id = Registry.ITEM.getId(s.getItem()).getPath();
        if (id.endsWith("potion") && r.chance(0.6)) {
            s.getOrCreateTag().putString("Potion", r.pick(new String[] {"minecraft:water", "minecraft:awkward", "minecraft:swiftness"}));
        }
        return s;
    }

    /** A villager offer's stack from {@code pool}: tools one at a time, maybe damaged or named. */
    private static ItemStack offerStack(Rng r, String[] pool) {
        Item item = Registry.ITEM.get(new Identifier(r.pick(pool)));
        ItemStack s = new ItemStack(item, item.getMaxCount() == 1 ? 1 : r.range(1, 20));
        if (item.getMaxDamage() > 0 && r.chance(0.3)) {
            s.setDamage(r.nextInt(4));
        }
        if (r.chance(0.2)) {
            s.setCustomName(new net.minecraft.text.LiteralText(r.chance(0.5) ? "a" : "b"));
        }
        return s;
    }

    /**
     * Random offers with prices (demand, multiplier, special price), sent as the server's
     * {@code trade_list} and stored like {@code ClientPlayNetworkHandler.onSetTradeOffers} does.
     */
    private static String offers(Rng r, int sync) {
        TradeOfferList list = new TradeOfferList();
        for (int k = r.range(1, 4); k > 0; k--) {
            ItemStack second = r.chance(0.4) ? offerStack(r, PAY) : ItemStack.EMPTY;
            int max = r.range(1, 6);
            TradeOffer o = new TradeOffer(offerStack(r, PAY), second, offerStack(r, SELL), r.range(0, max + 1), max,
                    r.nextInt(10), r.chance(0.5) ? 0.05F : 0.2F, r.chance(0.5) ? 0 : r.range(-5, 10));
            if (r.chance(0.4)) {
                o.setSpecialPrice(r.range(-4, 4));
            }
            list.add(o);
        }
        PacketByteBuf buf = new PacketByteBuf(Unpooled.buffer());
        try {
            new SetTradeOffersS2CPacket(sync, list, r.nextInt(5), r.nextInt(100), r.chance(0.5), r.chance(0.5)).write(buf);
            byte[] bytes = new byte[buf.readableBytes()];
            buf.getBytes(0, bytes);
            SetTradeOffersS2CPacket p = new SetTradeOffersS2CPacket();
            p.read(new PacketByteBuf(Unpooled.wrappedBuffer(bytes)));
            trader.setOffers(new TradeOfferList(p.getOffers().toNbt()));
            StringBuilder h = new StringBuilder(bytes.length * 2);
            for (byte x : bytes) {
                h.append(Character.forDigit((x >> 4) & 15, 16)).append(Character.forDigit(x & 15, 16));
            }
            return h.toString();
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
    }

    /** A payment for {@code want}: its item and NBT, at least as many (up to a stack). */
    private static ItemStack pay(Rng r, ItemStack want) {
        if (want.isEmpty()) {
            return ItemStack.EMPTY;
        }
        ItemStack s = want.copy();
        s.setCount(Math.min(want.getMaxCount(), want.getCount() * r.range(1, 3) + r.nextInt(3)));
        return s;
    }

    /** The client's count of uses per offer. */
    private static void uses(StringBuilder b) {
        TradeOfferList list = trader.getRecipes();
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
        player.currentScreenHandler = w0;
        Reflect.set(ScreenHandler.class, w0, "actionId", (short) 0);
        inv.setCursorStack(ItemStack.EMPTY);
        boolean merchant = menu.equals("merchant");
        boolean known = true;
        if (menu.equals("horse")) {
            mount = r.pick(MOUNTS);
            mountChest = !mount.endsWith("horse") && r.chance(0.6);
            mountStrength = r.range(1, 5);
            mountTame = r.chance(0.8);
            mountBaby = r.chance(0.15);
            // whether the replay is told the mount's rules (else it predicts without them)
            known = r.chance(0.75);
        }
        int sync = 0;
        ScreenHandler h = null;
        String offers = null;
        if (merchant) {
            // the trader first: the player's items are drawn from its offers
            sync = r.range(1, 100);
            h = handler(sync, inv, menu);
            // the server's offers arrive before the payments are placed
            offers = r.chance(0.92) ? offers(r, sync) : null;
        }
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

        if (!merchant) {
            sync = r.range(1, 100);
            h = handler(sync, inv, menu);
        }
        player.experienceLevel = 0;
        player.abilities.creativeMode = false;
        renames.clear();
        if (h instanceof AnvilScreenHandler a) {
            player.experienceLevel = r.range(0, 45);
            player.abilities.creativeMode = r.chance(0.1);
            anvilScreen(a);
        }
        int n = h.slots.size() - 36;
        double cfill = r.nextDouble();
        // the anvil's / villager's output is what the handler computes from the inputs (a server
        // sends the same; without offers the client keeps the server's)
        int filled = menu.equals("anvil") || offers != null ? 2 : n;
        if (h instanceof AnvilScreenHandler && r.chance(0.6)) {
            // a pair the anvil works with: gear and the same gear, a book or its repair material
            ItemStack a0 = ClickDump.stack(r, new String[] {"diamond_sword", "iron_pickaxe", "diamond_helmet", "bow",
                "elytra", "trident"});
            for (int k = r.nextInt(3); k > 0; k--) {
                a0.addEnchantment(r.pick(ANVIL_ENCH), r.range(1, 4));
            }
            a0.setCount(1);
            if (a0.isDamageable() && r.chance(0.7)) {
                a0.setDamage(r.nextInt(a0.getMaxDamage()));
            }
            ItemStack b0;
            int pick = r.nextInt(3);
            if (pick == 0) {
                b0 = new ItemStack(a0.getItem());
                for (int k = r.nextInt(3); k > 0; k--) {
                    b0.addEnchantment(r.pick(ANVIL_ENCH), r.range(1, 4));
                }
                if (b0.isDamageable() && r.chance(0.7)) {
                    b0.setDamage(r.nextInt(b0.getMaxDamage()));
                }
            } else if (pick == 1) {
                b0 = new ItemStack(net.minecraft.item.Items.ENCHANTED_BOOK);
                for (int k = r.range(1, 2); k > 0; k--) {
                    net.minecraft.item.EnchantedBookItem.addEnchantment(b0,
                            new net.minecraft.enchantment.EnchantmentLevelEntry(r.pick(ANVIL_ENCH), r.range(1, 4)));
                }
            } else {
                String m = a0.getItem() == net.minecraft.item.Items.ELYTRA ? "phantom_membrane"
                        : a0.getItem() == net.minecraft.item.Items.IRON_PICKAXE ? "iron_ingot" : "diamond";
                b0 = new ItemStack(Registry.ITEM.get(new Identifier(m)), r.range(1, 5));
            }
            if (r.chance(0.2)) {
                a0.setCustomName(new net.minecraft.text.LiteralText("a"));
            }
            h.getSlot(0).setStack(a0);
            h.getSlot(1).setStack(b0);
        } else if (offers != null && r.chance(0.5)) {
            // an offer's payments, enough of them (sometimes swapped)
            TradeOfferList list = trader.getRecipes();
            TradeOffer o = list.get(r.nextInt(list.size()));
            ItemStack a = pay(r, o.getAdjustedFirstBuyItem());
            ItemStack b2 = pay(r, o.getSecondBuyItem());
            boolean swap = r.chance(0.2);
            h.getSlot(0).setStack(swap ? b2 : a);
            h.getSlot(1).setStack(swap ? a : b2);
        } else {
            for (int k = 0; k < filled; k++) {
                ItemStack put = r.chance(cfill) ? menuStack(r, menu) : ItemStack.EMPTY;
                if ((menu.equals("loom") && k < 3 || menu.equals("cartography") && k < 2) && !put.isEmpty()
                        && !h.getSlot(k).canInsert(put)) {
                    // a vanilla server only puts what the slot takes (a non-dye in the dye slot
                    // crashes the client's loom)
                    put = ItemStack.EMPTY;
                }
                h.getSlot(k).setStack(put);
            }
        }
        player.currentScreenHandler = h;
        if (r.chance(0.4)) {
            inv.setCursorStack(anyStack(r, menu));
        }

        StringBuilder b = new StringBuilder(4096);
        b.append("{\"case\":\"").append(name).append("\",\"kind\":\"").append(menu).append("\",\"menu\":").append(menuId(menu))
                .append(",\"sync\":").append(sync).append(",\"selected\":").append(inv.selectedSlot);
        if (offers != null) {
            b.append(",\"offers\":\"").append(offers).append('"');
        }
        if (h instanceof AnvilScreenHandler) {
            b.append(",\"creative\":").append(player.abilities.creativeMode);
        }
        if (menu.equals("horse")) {
            b.append(",\"slots\":").append(n).append(",\"mount\":{\"entity\":\"").append(mount).append("\",\"chest\":")
                    .append(mountChest).append(",\"tame\":").append(mountTame).append(",\"baby\":").append(mountBaby)
                    .append(",\"known\":").append(known).append('}');
        }
        ClickDump.state(b, player, w0, h);
        if (merchant) {
            uses(b);
        }
        if (h instanceof AnvilScreenHandler a) {
            anvilState(b, a, player);
        }
        b.append(",\"clicks\":[");
        int clicks = r.range(1, 10);
        SlotActionType[] types = SlotActionType.values();
        for (int c = 0; c < clicks; c++) {
            if (h instanceof AnvilScreenHandler a && anvilEditable && r.chance(0.15)) {
                // typing into the name field (its change listener: onRenamed)
                String typed = r.pick(new String[] {"", "a", "b", "Named", "a b", "x".repeat(35)});
                anvilRenamed(a, typed);
                b.append(c > 0 ? "," : "").append("{\"rename\":").append(new com.google.gson.JsonPrimitive(typed));
                ClickDump.state(b, player, w0, h);
                anvilState(b, a, player);
                b.append('}');
                continue;
            }
            if (h instanceof LoomScreenHandler lm && r.chance(0.25)) {
                // LoomScreen.mouseClicked: onButtonClick for a pattern button (+ the button packet
                // when it accepted it, not captured)
                int k = r.range(0, net.minecraft.block.entity.BannerPattern.LOOM_APPLICABLE_COUNT + 1);
                boolean ok = lm.onButtonClick(player, k);
                b.append(c > 0 ? "," : "").append("{\"menu_button\":").append(k).append(",\"accepted\":").append(ok);
                ClickDump.state(b, player, w0, h);
                b.append(",\"selected\":").append(lm.getSelectedPattern());
                b.append('}');
                continue;
            }
            if (h instanceof StonecutterScreenHandler sc && sc.getAvailableRecipeCount() > 0 && r.chance(0.25)) {
                // StonecutterScreen.mouseClicked: onButtonClick (+ the button packet, not captured)
                int k = r.nextInt(sc.getAvailableRecipeCount());
                sc.onButtonClick(player, k);
                b.append(c > 0 ? "," : "").append("{\"menu_button\":").append(k);
                ClickDump.state(b, player, w0, h);
                b.append('}');
                continue;
            }
            if (offers != null && r.chance(0.15)) {
                // MerchantScreen.syncRecipeIndex: setRecipeIndex, switchTo (+ select_trade, not captured)
                int k = r.nextInt(trader.getRecipes().size());
                trader.setRecipeIndex(k);
                trader.switchTo(k);
                b.append(c > 0 ? "," : "").append("{\"select\":").append(k);
                ClickDump.state(b, player, w0, h);
                uses(b);
                b.append('}');
                continue;
            }
            int len = h.slots.size();
            int mode = r.pick(new int[] {0, 0, 0, 1, 1, 2, 4, 6});
            int slot = (merchant || h instanceof AnvilScreenHandler) && r.chance(0.25) ? 2
                    : r.chance(0.45) ? r.nextInt(n) : r.nextInt(len);
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
            if (merchant) {
                uses(b);
            }
            if (h instanceof AnvilScreenHandler a) {
                anvilState(b, a, player);
            }
            b.append('}');
        }
        b.append("]}");
        player.currentScreenHandler = w0;
        return b.toString();
    }
}
