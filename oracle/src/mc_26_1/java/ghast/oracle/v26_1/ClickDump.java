package ghast.oracle.v26_1;

import com.google.gson.JsonElement;
import com.google.gson.JsonParser;
import com.mojang.datafixers.util.Pair;
import com.mojang.serialization.JsonOps;
import ghast.oracle.Json;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import it.unimi.dsi.fastutil.ints.Int2ObjectMap;
import net.minecraft.client.gui.screens.inventory.ContainerScreen;
import net.minecraft.client.gui.screens.inventory.InventoryScreen;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.LayeredRegistryAccess;
import net.minecraft.core.NonNullList;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.component.DataComponentPatch;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.HashedPatchMap;
import net.minecraft.network.HashedStack;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientboundContainerSetContentPacket;
import net.minecraft.network.protocol.game.ClientboundContainerSetDataPacket;
import net.minecraft.network.protocol.game.ClientboundGameEventPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerAbilitiesPacket;
import net.minecraft.network.protocol.game.ClientboundSetExperiencePacket;
import net.minecraft.network.protocol.game.ClientboundSetHeldSlotPacket;
import net.minecraft.network.protocol.game.ClientboundUpdateRecipesPacket;
import net.minecraft.network.protocol.game.ServerboundContainerClickPacket;
import net.minecraft.network.protocol.game.ServerboundSetCreativeModeSlotPacket;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.RegistryOps;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.WorldLoader;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.CloseableResourceManager;
import net.minecraft.tags.ItemTags;
import net.minecraft.tags.TagLoader;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.inventory.ContainerInput;
import net.minecraft.world.inventory.MenuType;
import net.minecraft.world.item.BannerItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.crafting.CraftingInput;
import net.minecraft.world.item.crafting.CraftingRecipe;
import net.minecraft.world.item.crafting.RecipeManager;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.WorldDataConfiguration;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.function.Consumer;
import java.util.stream.Stream;

/**
 * 26.1 window click vectors: the real client click path ({@code MultiPlayerGameMode.handleContainerInput}:
 * the menu's {@code clicked} — {@code AbstractContainerMenu.doClick} with every {@code ContainerInput},
 * quick-craft drags included — then the slots whose stack changed and the carried stack as
 * {@code HashedStack}s in the {@code ServerboundContainerClickPacket} it sends) on every menu type a
 * server can open with {@code open_screen}, each built the way the client builds it
 * ({@code MenuType.create(containerId, inventory)}, what {@code MenuScreens.create} does) and filled
 * through real {@code container_set_content} packets. Schema: oracle/README.md, "Window clicks (26.1)".
 *
 * <p>Also real: the server's recipe sync ({@code ClientboundUpdateRecipesPacket} from the vanilla data
 * pack's recipes, the first line of the file) whose item sets decide furnace / smithing / stonecutter
 * slot rules; game mode changes (game event, player info, abilities packets: creative enables
 * {@code CLONE}, clone drags, and lifts the binding curse lock); experience (anvil cost check); held
 * slot; container data (crafter slot locks, anvil cost, stonecutter / loom selection).
 *
 * <p>Faked: {@code Minecraft.screen} is an allocated {@code InventoryScreen} / {@code ContainerScreen}
 * (never initialised or rendered): the click code only asks whether a container screen is open (a
 * creative-mode drop sends {@code set_creative_mode_slot} only without one).
 */
final class ClickDump {
    static final int CASES = 3000;

    private ClickDump() {}

    /** Menus by registry id (index = id); -1 is window 0. */
    private static List<MenuType<?>> MENUS;

    private static List<MenuType<?>> menus() {
        if (MENUS == null) {
            MENUS = List.of(MenuType.GENERIC_9x1, MenuType.GENERIC_9x2, MenuType.GENERIC_9x3,
            MenuType.GENERIC_9x4, MenuType.GENERIC_9x5, MenuType.GENERIC_9x6, MenuType.GENERIC_3x3, MenuType.CRAFTER_3x3,
            MenuType.ANVIL, MenuType.BEACON, MenuType.BLAST_FURNACE, MenuType.BREWING_STAND, MenuType.CRAFTING,
            MenuType.ENCHANTMENT, MenuType.FURNACE, MenuType.GRINDSTONE, MenuType.HOPPER, MenuType.LECTERN, MenuType.LOOM,
            MenuType.MERCHANT, MenuType.SHULKER_BOX, MenuType.SMITHING, MenuType.SMOKER, MenuType.CARTOGRAPHY_TABLE,
            MenuType.STONECUTTER);
        }
        return MENUS;
    }

    /** Items whose rules differ (stack sizes, armor, menus' slot rules, remainders, bundles, shulker boxes). */
    static final String[] ITEMS = {
        "stone", "dirt", "cobblestone", "oak_planks", "oak_log", "white_wool", "coal", "charcoal", "stick", "torch",
        "ender_pearl", "snowball", "egg", "bucket", "water_bucket", "lava_bucket", "milk_bucket", "honey_bottle",
        "dragon_breath", "potion", "splash_potion", "lingering_potion", "glass_bottle", "nether_wart", "blaze_powder",
        "blaze_rod", "sugar", "redstone", "gunpowder", "lapis_lazuli", "book", "enchanted_book", "iron_ingot", "gold_ingot",
        "emerald", "diamond", "netherite_ingot", "raw_iron", "beef", "sand", "white_banner", "red_banner", "red_dye",
        "creeper_banner_pattern", "flower_banner_pattern", "filled_map", "map", "paper", "glass_pane",
        "netherite_upgrade_smithing_template", "coast_armor_trim_smithing_template", "diamond_sword", "diamond_pickaxe",
        "iron_axe", "bow", "fishing_rod", "shears", "leather_helmet", "iron_chestplate", "diamond_leggings", "golden_boots",
        "turtle_helmet", "elytra", "carved_pumpkin", "player_head", "shield", "totem_of_undying", "saddle",
        "diamond_horse_armor", "white_carpet", "wolf_armor", "shulker_box", "red_shulker_box", "bundle", "red_bundle",
        "beehive", "chest", "warped_fungus_on_a_stick", "golden_apple", "cake",
    };
    static final String[] ARMOR = {"leather_helmet", "iron_chestplate", "diamond_leggings", "golden_boots", "elytra", "carved_pumpkin",
        "turtle_helmet", "player_head"};
    static final String[] BUNDLED = {"stone", "dirt", "ender_pearl", "diamond_sword", "egg", "red_bundle", "beehive", "torch", "potion"};

    private static RecipeManager recipes;
    /** The server's registry layers of the recipe load (registry sync and tags come from them). */
    private static LayeredRegistryAccess<RegistryLayer> serverLayers;
    private static List<String> syncLines;
    private static String recipesHex;
    private static RegistryOps<JsonElement> jsonOps;

    /** Case {@code i} depends only on (seed, i). */
    private static String caseLine(Harness.Run run, long seed, int i) {
        return one(run, new Rng(seed * 1_000_211L + i + 3), "click26_s" + seed + "_" + i);
    }

    static Harness.Run newRun() {
        Harness.init();
        return new Harness.Run(new Scenario("clicks").fill(-8, 63, -8, 8, 63, 8, States.id("stone"), 0).spawn(0.5, 64, 0.5, 0, 0));
    }

    static void lines(long seed, int count, Consumer<String> sink) {
        Harness.Run run = newRun();
        header(sink);
        for (int i = 0; i < count; i++) {
            sink.accept(caseLine(run, seed, i));
        }
    }

    /** selftest: the cases rerun in reverse order on the same player give the same lines. */
    static int reverseMismatches(long seed, int count) {
        Harness.Run run = newRun();
        header(s -> {});
        List<String> forward = new ArrayList<>(count);
        for (int i = 0; i < count; i++) {
            forward.add(caseLine(run, seed, i));
        }
        int bad = 0;
        for (int i = count - 1; i >= 0; i--) {
            if (!caseLine(run, seed, i).equals(forward.get(i))) {
                System.out.println("DIFF reverse-order rerun of 26.1 click case " + i);
                bad++;
            }
        }
        return bad;
    }

    // ------------------------------------------------------------------ header: recipes, registries, item facts

    private static void header(Consumer<String> sink) {
        syncRecipes();
        sink.accept(new Json().beginObject().field("type", "update_recipes").field("packet", recipesHex).endObject().toString());
        for (String line : syncLines()) {
            sink.accept(line);
        }
        RegistryAccess access = Harness.listener.registryAccess();
        jsonOps = access.createSerializationContext(JsonOps.INSTANCE);
        access.registries().sorted(Comparator.comparing(e -> e.key().identifier().toString())).forEach(e -> {
            @SuppressWarnings("unchecked")
            Registry<Object> reg = (Registry<Object>) e.value();
            Json j = new Json().beginObject().field("type", "registry").field("name", e.key().identifier().toString());
            j.key("entries").beginArray();
            for (int i = 0; i < reg.size(); i++) {
                j.value(reg.getKey(reg.byId(i)).toString());
            }
            sink.accept(j.endArray().endObject().toString());
        });
        // Item facts the slot rules read (data-driven ones as the client has them after configuration).
        var level = Harness.client.level;
        for (Item item : BuiltInRegistries.ITEM) {
            ItemStack st = new ItemStack(item);
            Json j = new Json().beginObject().field("type", "item").field("id", BuiltInRegistries.ITEM.getId(item))
                    .field("name", BuiltInRegistries.ITEM.getKey(item).toString());
            if (!st.isEmpty()) {
                j.field("fuel", level.fuelValues().isFuel(st));
                j.field("brewing_ingredient", level.potionBrewing().isIngredient(st));
                j.field("brewing_fuel", st.is(ItemTags.BREWING_FUEL));
                j.field("beacon_payment", st.is(ItemTags.BEACON_PAYMENT_ITEMS));
                j.field("loom_dye_tag", st.is(ItemTags.LOOM_DYES));
                j.field("loom_pattern_tag", st.is(ItemTags.LOOM_PATTERNS));
                j.field("banner", item instanceof BannerItem);
                j.field("fits_container", item.canFitInsideContainerItems());
                j.field("has_dye", st.has(DataComponents.DYE));
                j.field("has_patterns", st.has(DataComponents.PROVIDES_BANNER_PATTERNS));
                NonNullList<ItemStack> rem = CraftingRecipe.defaultCraftingReminder(CraftingInput.of(1, 1, List.of(st)));
                j.field("remainder", hex(rem.get(0)));
            }
            sink.accept(j.endObject().toString());
        }
    }

    /** Loads the vanilla data pack's recipes (once) the way a server does and syncs them to the client. */
    private static void syncRecipes() {
        if (recipes == null) {
            PackRepository packs = ServerPacksSource.createVanillaTrustedRepository();
            WorldLoader.PackConfig packConfig = new WorldLoader.PackConfig(packs, WorldDataConfiguration.DEFAULT, false, true);
            Pair<WorldDataConfiguration, CloseableResourceManager> opened = packConfig.createResourceManager();
            try (CloseableResourceManager resources = opened.getSecond()) {
                LayeredRegistryAccess<RegistryLayer> layers = RegistryLayer.createRegistryAccess();
                List<Registry.PendingTags<?>> staticTags = TagLoader.loadTagsForExistingRegistries(resources,
                        layers.getLayer(RegistryLayer.STATIC));
                RegistryAccess.Frozen worldgenContext = layers.getAccessForLoading(RegistryLayer.WORLDGEN);
                List<HolderLookup.RegistryLookup<?>> worldgenLookups = TagLoader.buildUpdatedLookups(worldgenContext, staticTags);
                RegistryAccess.Frozen worldgen = RegistryDataLoader.load(resources, worldgenLookups,
                        RegistryDataLoader.WORLDGEN_REGISTRIES, Runnable::run).join();
                List<HolderLookup.RegistryLookup<?>> dimensionLookups = Stream.concat(worldgenLookups.stream(),
                        worldgen.listRegistries()).toList();
                RegistryAccess.Frozen dimensions = RegistryDataLoader.load(resources, dimensionLookups,
                        RegistryDataLoader.DIMENSION_REGISTRIES, Runnable::run).join();
                layers = layers.replaceFrom(RegistryLayer.WORLDGEN, worldgen, dimensions);
                List<HolderLookup.RegistryLookup<?>> reloadLookups = Stream.concat(dimensionLookups.stream(),
                        dimensions.listRegistries()).toList();
                HolderLookup.Provider provider = HolderLookup.Provider.create(reloadLookups.stream());
                RecipeManager rm = new RecipeManager(provider);
                Object prepared = call(RecipeManager.class, rm, "prepare",
                        new Class<?>[] {net.minecraft.server.packs.resources.ResourceManager.class, net.minecraft.util.profiling.ProfilerFiller.class},
                        resources, net.minecraft.util.profiling.InactiveProfiler.INSTANCE);
                call(RecipeManager.class, rm, "apply",
                        new Class<?>[] {net.minecraft.world.item.crafting.RecipeMap.class, net.minecraft.server.packs.resources.ResourceManager.class,
                                net.minecraft.util.profiling.ProfilerFiller.class},
                        prepared, resources, net.minecraft.util.profiling.InactiveProfiler.INSTANCE);
                // the recipes' item tags are the pending static tags just loaded: bind them as a server
                // does (the same vanilla contents the client received in configuration)
                staticTags.forEach(Registry.PendingTags::apply);
                rm.finalizeRecipeLoading(net.minecraft.world.flag.FeatureFlags.DEFAULT_FLAGS);
                recipes = rm;
                serverLayers = layers;
            }
            // RecipePropertySet keys sorted: getSynchronizedItemProperties is a plain map
            Map<net.minecraft.resources.ResourceKey<net.minecraft.world.item.crafting.RecipePropertySet>,
                    net.minecraft.world.item.crafting.RecipePropertySet> sets = new LinkedHashMap<>();
            // ...and their items are hash sets of holders, the stonecutter entries in recipe map
            // order: both written sorted (item id, recipe id) so every run writes the same bytes
            recipes.getSynchronizedItemProperties().entrySet().stream()
                    .sorted(Comparator.comparing(e -> e.getKey().identifier().toString()))
                    .forEach(e -> sets.put(e.getKey(), sortedSet(e.getValue())));
            var cutter = recipes.getSynchronizedStonecutterRecipes();
            List<net.minecraft.world.item.crafting.SelectableRecipe.SingleInputEntry<net.minecraft.world.item.crafting.StonecutterRecipe>> entries =
                    new ArrayList<>(cutter.entries());
            entries.sort(Comparator.comparing(e -> e.recipe().recipe().map(h -> h.id().identifier().toString()).orElse("")));
            ClientboundUpdateRecipesPacket p = new ClientboundUpdateRecipesPacket(sets,
                    new net.minecraft.world.item.crafting.SelectableRecipe.SingleInputSet<>(entries));
            RegistryFriendlyByteBuf buf = Harness.buf();
            ClientboundUpdateRecipesPacket.STREAM_CODEC.encode(buf, p);
            byte[] bytes = new byte[buf.readableBytes()];
            buf.getBytes(buf.readerIndex(), bytes);
            recipesHex = ByteBufUtil.hexDump(bytes);
        }
        RegistryFriendlyByteBuf in = new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(unhex(recipesHex)), Harness.listener.registryAccess());
        ClientboundUpdateRecipesPacket back = ClientboundUpdateRecipesPacket.STREAM_CODEC.decode(in);
        Harness.listener.handleUpdateRecipes(back);
    }

    @SuppressWarnings("unchecked")
    private static net.minecraft.world.item.crafting.RecipePropertySet sortedSet(net.minecraft.world.item.crafting.RecipePropertySet set) {
        Set<Holder<Item>> items = (Set<Holder<Item>>) Reflect.get(net.minecraft.world.item.crafting.RecipePropertySet.class, set, "items");
        List<Holder<Item>> sorted = new ArrayList<>(items);
        sorted.sort(Comparator.comparingInt(h -> BuiltInRegistries.ITEM.getId(h.value())));
        try {
            var c = net.minecraft.world.item.crafting.RecipePropertySet.class.getDeclaredConstructor(Set.class);
            c.setAccessible(true);
            return c.newInstance(new LinkedHashSet<>(sorted));
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
    }

    /**
     * The configuration data the anvil, grindstone and loom rules read, as the server sends it:
     * the enchantment and banner pattern {@code registry_data} packets (every element in full,
     * no known packs) and the {@code update_tags} packet (sorted by registry and tag name; the
     * server's own map order depends on hash codes). The harness client received the same data in
     * {@code Vanilla.configure}.
     */
    private static List<String> syncLines() {
        if (syncLines != null) {
            return syncLines;
        }
        List<String> out = new ArrayList<>();
        RegistryAccess.Frozen serverAccess = serverLayers.compositeAccess();
        net.minecraft.core.RegistrySynchronization.packRegistries(serverAccess.createSerializationContext(net.minecraft.nbt.NbtOps.INSTANCE),
                serverAccess, Set.of(), (key, entries) -> {
                    String name = key.identifier().toString();
                    if (!name.equals("minecraft:enchantment") && !name.equals("minecraft:banner_pattern")) {
                        return;
                    }
                    var buf = new net.minecraft.network.FriendlyByteBuf(Unpooled.buffer());
                    net.minecraft.network.protocol.configuration.ClientboundRegistryDataPacket.STREAM_CODEC.encode(buf,
                            new net.minecraft.network.protocol.configuration.ClientboundRegistryDataPacket(key, entries));
                    out.add(new Json().beginObject().field("type", "registry_data").field("registry", name)
                            .field("packet", ByteBufUtil.hexDump(buf)).endObject().toString());
                });
        Map<net.minecraft.resources.ResourceKey<? extends Registry<?>>, net.minecraft.tags.TagNetworkSerialization.NetworkPayload> tags =
                net.minecraft.tags.TagNetworkSerialization.serializeTagsToNetwork(serverLayers);
        Map<net.minecraft.resources.ResourceKey<? extends Registry<?>>, net.minecraft.tags.TagNetworkSerialization.NetworkPayload> sorted =
                new LinkedHashMap<>();
        try {
            var ctor = net.minecraft.tags.TagNetworkSerialization.NetworkPayload.class.getDeclaredConstructor(Map.class);
            ctor.setAccessible(true);
            List<net.minecraft.resources.ResourceKey<? extends Registry<?>>> keys = new ArrayList<>(tags.keySet());
            keys.sort(Comparator.comparing(k -> k.identifier().toString()));
            for (var k : keys) {
                @SuppressWarnings("unchecked")
                Map<net.minecraft.resources.Identifier, it.unimi.dsi.fastutil.ints.IntList> m =
                        (Map<net.minecraft.resources.Identifier, it.unimi.dsi.fastutil.ints.IntList>) Reflect.get(
                                net.minecraft.tags.TagNetworkSerialization.NetworkPayload.class, tags.get(k), "tags");
                Map<net.minecraft.resources.Identifier, it.unimi.dsi.fastutil.ints.IntList> ms = new LinkedHashMap<>();
                m.entrySet().stream().sorted(Comparator.comparing(e -> e.getKey().toString())).forEach(e -> ms.put(e.getKey(), e.getValue()));
                sorted.put(k, ctor.newInstance(ms));
            }
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
        var buf = new net.minecraft.network.FriendlyByteBuf(Unpooled.buffer());
        net.minecraft.network.protocol.common.ClientboundUpdateTagsPacket.STREAM_CODEC.encode(buf,
                new net.minecraft.network.protocol.common.ClientboundUpdateTagsPacket(sorted));
        out.add(new Json().beginObject().field("type", "update_tags").field("packet", ByteBufUtil.hexDump(buf)).endObject().toString());
        syncLines = out;
        return out;
    }

    // ------------------------------------------------------------------ random stacks

    private static ItemStack stack(Rng r, String[] pool) {
        Item item = BuiltInRegistries.ITEM.getValue(net.minecraft.resources.Identifier.withDefaultNamespace(r.pick(pool)));
        ItemStack s = new ItemStack(item);
        int max = s.getMaxStackSize();
        int count = r.chance(0.1) ? max : r.chance(0.04) ? r.range(1, 99) : r.range(1, max);
        s.setCount(count);
        if (s.isDamageableItem() && r.chance(0.4)) {
            s.setDamageValue(r.range(1, Math.max(1, s.getMaxDamage() - 1)));
        }
        if (item == Items.BUNDLE || item == Items.RED_BUNDLE) {
            if (r.chance(0.7)) {
                s.set(DataComponents.BUNDLE_CONTENTS, bundle(r));
            }
        }
        if (item == Items.FILLED_MAP) {
            s.set(DataComponents.MAP_ID, new net.minecraft.world.level.saveddata.maps.MapId(r.nextInt(4)));
        }
        if (r.chance(0.15)) {
            component(r, s);
        }
        return s;
    }

    private static net.minecraft.world.item.component.BundleContents bundle(Rng r) {
        List<net.minecraft.world.item.ItemStackTemplate> items = new ArrayList<>();
        int n = r.range(0, 4);
        int weight64 = 0;
        for (int k = 0; k < n; k++) {
            ItemStack in = stack(r, BUNDLED);
            if (in.getItem() == Items.RED_BUNDLE || in.isDamageableItem() && in.isDamaged() || in.getCount() > in.getMaxStackSize()) {
                in = new ItemStack(Items.STONE, r.range(1, 8));
            }
            if (in.getItem() == Items.BEEHIVE) {
                continue;
            }
            int w = 64 / in.getMaxStackSize() * in.getCount();
            if (weight64 + w > 64) {
                break;
            }
            weight64 += w;
            items.add(net.minecraft.world.item.ItemStackTemplate.fromNonEmptyStack(in));
        }
        return new net.minecraft.world.item.component.BundleContents(items);
    }

    /** One random component change, as a server could send it. */
    private static void component(Rng r, ItemStack s) {
        switch (r.nextInt(10)) {
            case 0 -> set(s, DataComponents.CUSTOM_NAME, r.chance(0.5) ? "\"a\"" : "\"b\"");
            case 1 -> {
                if (!s.isDamageableItem()) {
                    s.set(DataComponents.MAX_STACK_SIZE, r.pick(new int[] {1, 2, 16, 64, 99}));
                    if (s.getCount() > s.getMaxStackSize() && r.chance(0.7)) {
                        s.setCount(s.getMaxStackSize());
                    }
                }
            }
            case 2 -> set(s, s.is(Items.ENCHANTED_BOOK) || s.is(Items.BOOK) ? DataComponents.STORED_ENCHANTMENTS : DataComponents.ENCHANTMENTS,
                    r.pick(new String[] {"{\"minecraft:sharpness\":2}", "{\"minecraft:binding_curse\":1}", "{\"minecraft:vanishing_curse\":1}",
                            "{\"minecraft:unbreaking\":3}"}));
            case 3 -> s.set(DataComponents.REPAIR_COST, r.range(0, 5));
            case 4 -> set(s, DataComponents.UNBREAKABLE, "{}");
            case 5 -> set(s, DataComponents.EQUIPPABLE, r.pick(new String[] {"{\"slot\":\"head\"}", "{\"slot\":\"chest\"}",
                    "{\"slot\":\"offhand\"}", "{\"slot\":\"feet\",\"allowed_entities\":\"minecraft:player\"}",
                    "{\"slot\":\"head\",\"allowed_entities\":\"minecraft:zombie\"}", "{\"slot\":\"legs\",\"allowed_entities\":[\"minecraft:zombie\",\"minecraft:player\"]}",
                    "{\"slot\":\"body\"}"}));
            case 6 -> set(s, DataComponents.CUSTOM_DATA, r.chance(0.5) ? "{\"x\":1}" : "{\"y\":\"z\"}");
            case 7 -> {
                if (s.has(DataComponents.MAX_DAMAGE) || s.isDamageableItem()) {
                    s.set(DataComponents.DAMAGE, r.range(0, 5));
                } else {
                    s.remove(DataComponents.MAX_STACK_SIZE);
                }
            }
            case 8 -> s.remove(r.chance(0.5) ? DataComponents.EQUIPPABLE : DataComponents.ENCHANTMENTS);
            default -> set(s, DataComponents.LORE, "[\"l\"]");
        }
    }

    /** One or two random enchantments (stored on books), sometimes a repair cost or a name. */
    private static void enchant(Rng r, ItemStack s) {
        String[] e = {"minecraft:sharpness", "minecraft:smite", "minecraft:unbreaking", "minecraft:efficiency", "minecraft:mending",
            "minecraft:binding_curse", "minecraft:vanishing_curse", "minecraft:protection", "minecraft:fire_protection", "minecraft:power",
            "minecraft:infinity", "minecraft:silk_touch", "minecraft:fortune"};
        StringBuilder j = new StringBuilder("{");
        int n = r.range(1, 2);
        Set<String> used = new LinkedHashSet<>();
        for (int k = 0; k < n; k++) {
            String id = r.pick(e);
            if (used.add(id)) {
                j.append(used.size() > 1 ? "," : "").append('"').append(id).append("\":").append(r.range(1, 4));
            }
        }
        j.append('}');
        boolean book = s.is(Items.BOOK) || s.is(Items.ENCHANTED_BOOK);
        if (s.is(Items.BOOK)) {
            return;
        }
        set(s, book ? DataComponents.STORED_ENCHANTMENTS : DataComponents.ENCHANTMENTS, j.toString());
        if (r.chance(0.3)) {
            s.set(DataComponents.REPAIR_COST, r.range(0, 7));
        }
        if (r.chance(0.2)) {
            set(s, DataComponents.CUSTOM_NAME, "\"Old\"");
        }
    }

    private static <T> void set(ItemStack s, DataComponentType<T> t, String json) {
        T v = t.codec().parse(jsonOps, JsonParser.parseString(json)).getOrThrow();
        s.set(t, v);
    }

    private static ItemStack maybe(Rng r, double p, String[] pool) {
        return r.chance(p) ? stack(r, pool) : ItemStack.EMPTY;
    }

    // ------------------------------------------------------------------ cases

    private static GameType mode = GameType.SURVIVAL;

    /** What a server sends on a game mode change: game event, tab list entry, abilities. */
    private static void setMode(Harness.Run run, GameType gm) {
        Harness.listener.handleGameEvent(Harness.roundTrip(ClientboundGameEventPacket.STREAM_CODEC,
                new ClientboundGameEventPacket(ClientboundGameEventPacket.CHANGE_GAME_MODE, gm.getId())));
        RegistryFriendlyByteBuf b = Harness.buf();
        b.writeEnumSet(java.util.EnumSet.of(net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket.Action.UPDATE_GAME_MODE),
                net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket.Action.class);
        b.writeVarInt(1);
        b.writeUUID(Harness.PLAYER_UUID);
        b.writeVarInt(gm.getId());
        Harness.listener.handlePlayerInfoUpdate(net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket.STREAM_CODEC.decode(b));
        Abilities ab = new Abilities();
        gm.updatePlayerAbilities(ab);
        Harness.listener.handlePlayerAbilities(Harness.roundTrip(ClientboundPlayerAbilitiesPacket.STREAM_CODEC, new ClientboundPlayerAbilitiesPacket(ab)));
        mode = gm;
    }

    private static void content(int containerId, int stateId, List<ItemStack> items, ItemStack carried) {
        Harness.listener.handleContainerContent(Harness.roundTrip(ClientboundContainerSetContentPacket.STREAM_CODEC,
                new ClientboundContainerSetContentPacket(containerId, stateId, items, carried)));
    }

    private static String one(Harness.Run run, Rng r, String name) {
        LocalPlayer player = run.player;
        AbstractContainerMenu w0 = player.inventoryMenu;
        player.containerMenu = w0;
        call(AbstractContainerMenu.class, w0, "resetQuickCraft", new Class<?>[0]);
        resetDropThrottle(player);
        GameType gm = r.chance(0.15) ? GameType.CREATIVE : GameType.SURVIVAL;
        int menuId = r.chance(0.2) ? -1 : r.nextInt(menus().size());
        if (gm == GameType.CREATIVE && menuId == -1) {
            // window 0 in creative is the creative inventory screen (set_creative_mode_slot, not container_click)
            menuId = r.nextInt(menus().size());
        }
        if (gm != mode) {
            setMode(run, gm);
        }
        Harness.listener.handleSetHeldSlot(Harness.roundTrip(ClientboundSetHeldSlotPacket.STREAM_CODEC, new ClientboundSetHeldSlotPacket(r.nextInt(9))));
        int xp = r.chance(0.5) ? 0 : r.range(1, 40);
        Harness.listener.handleSetExperience(Harness.roundTrip(ClientboundSetExperiencePacket.STREAM_CODEC, new ClientboundSetExperiencePacket(0.0F, 0, xp)));

        // window 0: 46 slots (result, grid, armor head..feet, main, hotbar, offhand)
        double fill = 0.2 + r.nextDouble() * 0.6;
        List<ItemStack> w0Items = new ArrayList<>();
        for (int k = 0; k < 46; k++) {
            ItemStack s;
            if (k == 0) {
                s = r.chance(0.3) ? stack(r, ITEMS) : ItemStack.EMPTY;
            } else if (k < 5) {
                s = maybe(r, 0.3, ITEMS);
            } else if (k < 9) {
                s = r.chance(0.3) ? stack(r, ARMOR) : ItemStack.EMPTY;
                if (s.getCount() > 1 && r.chance(0.8)) {
                    s.setCount(1);
                }
            } else {
                s = maybe(r, fill, ITEMS);
            }
            w0Items.add(s);
        }
        int w0State = r.nextInt(32768);
        content(0, w0State, w0Items, menuId == -1 && r.chance(0.35) ? stack(r, ITEMS) : ItemStack.EMPTY);

        AbstractContainerMenu h = w0;
        int containerId = 0;
        if (menuId >= 0) {
            containerId = r.range(1, 100);
            h = menus().get(menuId).create(containerId, player.getInventory());
            player.containerMenu = h;
            Reflect.set(net.minecraft.client.Minecraft.class, Harness.client, "screen", Reflect.allocate(ContainerScreen.class));
            List<ItemStack> items = new ArrayList<>();
            for (int k = 0; k < h.slots.size(); k++) {
                var slot = h.getSlot(k);
                if (slot.container == player.getInventory()) {
                    items.add(slot.getItem().copy());
                } else {
                    String[] pool = menuPool(menuId, k);
                    ItemStack it = pool != null && r.chance(0.6) ? maybe(r, 0.7, pool) : maybe(r, fill, ITEMS);
                    if ((menuId == 8 || menuId == 15) && !it.isEmpty() && r.chance(0.5)) {
                        enchant(r, it);
                    }
                    items.add(it);
                }
            }
            content(containerId, r.nextInt(32768), items, r.chance(0.35) ? stack(r, ITEMS) : ItemStack.EMPTY);
            if (menuId == 19) {
                offersHex = offers(r, containerId);
            } else {
                offersHex = null;
            }
            // container data: crafter slot locks, anvil cost, stonecutter / loom selection
            if (menuId == 7) {
                for (int k = 0; k < 9; k++) {
                    if (r.chance(0.25)) {
                        data(containerId, k, 1);
                    }
                }
            } else if (menuId == 8) {
                data(containerId, 0, r.chance(0.3) ? 0 : r.range(1, 45));
            } else if (menuId == 18 || menuId == 24) {
                data(containerId, 0, r.range(-1, 3));
            }
        } else {
            Reflect.set(net.minecraft.client.Minecraft.class, Harness.client, "screen", Reflect.allocate(InventoryScreen.class));
        }

        StringBuilder b = new StringBuilder(8192);
        b.append("{\"type\":\"case\",\"case\":\"").append(name).append("\",\"menu\":").append(menuId).append(",\"window\":").append(containerId)
                .append(",\"creative\":").append(gm == GameType.CREATIVE).append(",\"xp\":").append(xp)
                .append(",\"selected\":").append(player.getInventory().getSelectedSlot());
        if (menuId == 19) {
            b.append(",\"offers\":\"").append(offersHex).append('"');
        }
        if (menuId == 7 || menuId == 8 || menuId == 18 || menuId == 24) {
            b.append(",\"data\":[");
            int n = menuId == 7 ? 10 : 1;
            for (int k = 0; k < n; k++) {
                b.append(k > 0 ? "," : "").append(dataSlot(h, k));
            }
            b.append(']');
        }
        state(b, player, w0, h);
        b.append(",\"clicks\":[");
        int clicks = r.range(1, 10);
        int written = 0;
        for (int c = 0; c < clicks; c++) {
            List<int[]> seq = gesture(r, h, gm == GameType.CREATIVE);
            String typed = null;
            if (menuId == 19 && r.chance(0.15)) {
                seq = List.of(new int[] {r.range(-1, 6), 0, ACTION_SELECT_TRADE});
            } else if (menuId == 18 && r.chance(0.15)) {
                seq = List.of(new int[] {r.range(0, 8), 0, ACTION_BUTTON});
            } else if (menuId == 8 && r.chance(0.2)) {
                seq = List.of(new int[] {0, 0, ACTION_RENAME});
                typed = r.pick(NAMES);
                if (typed.equals("=") && h.getSlot(0).hasItem()) {
                    typed = h.getSlot(0).getItem().getHoverName().getString();
                }
            }
            for (int[] g : seq) {
                Harness.connection.sent.clear();
                if (g[2] >= 100) {
                    sentName = null;
                    action(h, player, containerId, g, typed);
                    if (written++ > 0) {
                        b.append(',');
                    }
                    b.append("{\"slot\":").append(g[0]).append(",\"button\":").append(g[1]).append(",\"mode\":").append(g[2]);
                    if (sentName != null) {
                        b.append(",\"name\":");
                        b.append(new Json().value(sentName).toString());
                    }
                    b.append(",\"packets\":[");
                    packets(b);
                    b.append(']');
                    state(b, player, w0, h);
                    b.append('}');
                    continue;
                }
                Harness.client.gameMode.handleContainerInput(containerId, g[0], g[1], ContainerInput.values()[g[2]], player);
                if (written++ > 0) {
                    b.append(',');
                }
                b.append("{\"slot\":").append(g[0]).append(",\"button\":").append(g[1]).append(",\"mode\":").append(g[2]);
                b.append(",\"packets\":[");
                int pk = 0;
                for (Packet<?> q : Harness.connection.sent) {
                    if (pk++ > 0) {
                        b.append(',');
                    }
                    b.append('"').append(packetHex(q)).append('"');
                }
                b.append(']');
                state(b, player, w0, h);
                b.append('}');
            }
        }
        b.append("]}");
        player.containerMenu = w0;
        Reflect.set(net.minecraft.client.Minecraft.class, Harness.client, "screen", null);
        return b.toString();
    }

    /** Items a menu's own slot usually holds (container slot k). */
    private static String[] menuPool(int menuId, int k) {
        return switch (menuId) {
            case 10, 14, 22 -> switch (k) {
                case 0 -> new String[] {"raw_iron", "beef", "sand", "cobblestone", "oak_log"};
                case 1 -> new String[] {"coal", "charcoal", "oak_planks", "lava_bucket", "bucket", "stick", "blaze_rod"};
                default -> new String[] {"iron_ingot", "charcoal", "stone"};
            };
            case 11 -> switch (k) {
                case 0, 1, 2 -> new String[] {"potion", "splash_potion", "lingering_potion", "glass_bottle"};
                case 3 -> new String[] {"nether_wart", "sugar", "blaze_powder", "redstone", "gunpowder", "dragon_breath", "stone"};
                default -> new String[] {"blaze_powder"};
            };
            case 13 -> k == 0 ? new String[] {"diamond_sword", "book", "diamond_pickaxe", "bow"} : new String[] {"lapis_lazuli"};
            case 9 -> new String[] {"iron_ingot", "gold_ingot", "emerald", "diamond", "netherite_ingot"};
            case 8 -> k == 1 ? new String[] {"diamond_sword", "enchanted_book", "diamond", "iron_ingot", "stick", "diamond_pickaxe", "bow"}
                    : new String[] {"diamond_sword", "diamond_pickaxe", "enchanted_book", "iron_axe", "book", "bow", "shears"};
            case 15 -> new String[] {"diamond_sword", "diamond_pickaxe", "enchanted_book", "iron_axe", "book", "bow", "shears"};
            case 21 -> switch (k) {
                case 0 -> new String[] {"netherite_upgrade_smithing_template", "coast_armor_trim_smithing_template"};
                case 1 -> new String[] {"diamond_sword", "iron_chestplate", "diamond_leggings"};
                default -> new String[] {"netherite_ingot", "gold_ingot", "diamond"};
            };
            case 24 -> new String[] {"stone", "cobblestone"};
            case 18 -> switch (k) {
                case 0 -> new String[] {"white_banner", "red_banner"};
                case 1 -> new String[] {"red_dye"};
                case 2 -> new String[] {"creeper_banner_pattern", "flower_banner_pattern"};
                default -> new String[] {"red_banner"};
            };
            case 23 -> switch (k) {
                case 0 -> new String[] {"filled_map"};
                case 1 -> new String[] {"paper", "map", "glass_pane"};
                default -> new String[] {"filled_map"};
            };
            case 19 -> new String[] {"emerald", "paper", "book"};
            case 17 -> new String[] {"book", "enchanted_book"};
            default -> null;
        };
    }

    /**
     * The player's drop spam throttle ({@code LocalPlayer.dropSpamThrottler}: +20 per creative
     * drop, -1 per player tick, drops refused from 1280) back to 0: cases never tick the player,
     * so without this the creative drops of earlier cases would add up.
     */
    private static void resetDropThrottle(LocalPlayer player) {
        Object t = Reflect.get(LocalPlayer.class, player, "dropSpamThrottler");
        Reflect.setInt(net.minecraft.util.TickThrottler.class, t, "count", 0);
    }

    static final int ACTION_SELECT_TRADE = 100;
    static final int ACTION_BUTTON = 101;
    static final int ACTION_RENAME = 102;
    /** Anvil names: "=" is the item's current hover name (the screen sends "" for it). */
    static final String[] NAMES = {"", "x", "Named", "=", "a b", "Bob"};
    private static String offersHex;
    private static String sentName;

    /**
     * A screen action that is not a container click: a trade picked in the villager screen
     * ({@code MerchantScreen.postButtonClick}: selection hint, {@code tryMoveItems}, the
     * {@code select_trade} packet), a loom pattern button ({@code clickMenuButton} +
     * {@code container_button_click}), a name typed in the anvil ({@code AnvilScreen.onNameChanged}).
     */
    private static void action(AbstractContainerMenu h, LocalPlayer player, int containerId, int[] g, String name) {
        switch (g[2]) {
            case ACTION_SELECT_TRADE -> {
                var m = (net.minecraft.world.inventory.MerchantMenu) h;
                m.setSelectionHint(g[0]);
                m.tryMoveItems(g[0]);
                Harness.connection.send(new net.minecraft.network.protocol.game.ServerboundSelectTradePacket(g[0]));
            }
            case ACTION_BUTTON -> {
                if (h.clickMenuButton(player, g[0])) {
                    Harness.client.gameMode.handleInventoryButtonClick(containerId, g[0]);
                }
            }
            default -> {
                var anvil = (net.minecraft.world.inventory.AnvilMenu) h;
                var slot = anvil.getSlot(0);
                if (slot.hasItem()) {
                    String n = name;
                    if (!slot.getItem().has(DataComponents.CUSTOM_NAME) && n.equals(slot.getItem().getHoverName().getString())) {
                        n = "";
                    }
                    sentName = n;
                    if (anvil.setItemName(n)) {
                        Harness.connection.send(new net.minecraft.network.protocol.game.ServerboundRenameItemPacket(n));
                    }
                }
            }
        }
    }

    /** Random villager offers sent as the server sends them (merchant_offers); returns the packet hex. */
    private static String offers(Rng r, int containerId) {
        net.minecraft.world.item.trading.MerchantOffers list = new net.minecraft.world.item.trading.MerchantOffers();
        String[] costs = {"emerald", "paper", "book", "stone", "coal", "iron_ingot"};
        int n = r.range(0, 6);
        for (int k = 0; k < n; k++) {
            Item a = BuiltInRegistries.ITEM.getValue(net.minecraft.resources.Identifier.withDefaultNamespace(r.pick(costs)));
            var costA = new net.minecraft.world.item.trading.ItemCost(a, r.range(1, 20));
            java.util.Optional<net.minecraft.world.item.trading.ItemCost> costB = r.chance(0.4)
                    ? java.util.Optional.of(new net.minecraft.world.item.trading.ItemCost(
                            BuiltInRegistries.ITEM.getValue(net.minecraft.resources.Identifier.withDefaultNamespace(r.pick(costs))), r.range(1, 10)))
                    : java.util.Optional.empty();
            ItemStack result = stack(r, ITEMS);
            if (result.getCount() > result.getMaxStackSize()) {
                result.setCount(result.getMaxStackSize());
            }
            var offer = new net.minecraft.world.item.trading.MerchantOffer(costA, costB, result, r.range(0, 4), r.range(1, 6), r.range(0, 10),
                    r.chance(0.5) ? 0.05F : 0.2F, r.chance(0.5) ? 0 : r.range(-5, 10));
            if (r.chance(0.3)) {
                offer.setSpecialPriceDiff(r.range(-5, 5));
            }
            list.add(offer);
        }
        var p = new net.minecraft.network.protocol.game.ClientboundMerchantOffersPacket(containerId, list, r.range(1, 5), r.range(0, 100), r.chance(0.5), r.chance(0.5));
        RegistryFriendlyByteBuf buf = Harness.buf();
        net.minecraft.network.protocol.game.ClientboundMerchantOffersPacket.STREAM_CODEC.encode(buf, p);
        byte[] bytes = new byte[buf.readableBytes()];
        buf.getBytes(buf.readerIndex(), bytes);
        RegistryFriendlyByteBuf in = new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(bytes), Harness.listener.registryAccess());
        Harness.listener.handleMerchantOffers(net.minecraft.network.protocol.game.ClientboundMerchantOffersPacket.STREAM_CODEC.decode(in));
        return ByteBufUtil.hexDump(bytes);
    }

    private static void data(int containerId, int id, int value) {
        Harness.listener.handleContainerSetData(Harness.roundTrip(ClientboundContainerSetDataPacket.STREAM_CODEC,
                new ClientboundContainerSetDataPacket(containerId, id, value)));
    }

    @SuppressWarnings("unchecked")
    private static int dataSlot(AbstractContainerMenu h, int k) {
        List<net.minecraft.world.inventory.DataSlot> l = (List<net.minecraft.world.inventory.DataSlot>) Reflect.get(AbstractContainerMenu.class, h, "dataSlots");
        return l.get(k).get();
    }

    /**
     * One GUI gesture as packets: [slot, button, mode]. Mostly what the GUI sends (drags as start /
     * slots / end with slot -999 at both ends), sometimes broken sequences.
     */
    private static List<int[]> gesture(Rng r, AbstractContainerMenu h, boolean creative) {
        int len = h.slots.size();
        List<int[]> out = new ArrayList<>();
        int slot = r.nextInt(len);
        double q = r.nextDouble();
        if (q < 0.30) {
            out.add(new int[] {r.chance(0.07) ? -999 : slot, r.nextInt(2), 0});
        } else if (q < 0.45) {
            out.add(new int[] {slot, r.nextInt(2), 1});
        } else if (q < 0.55) {
            out.add(new int[] {slot, r.chance(0.15) ? 40 : r.nextInt(9), 2});
        } else if (q < 0.60) {
            out.add(new int[] {slot, 2, 3});
        } else if (q < 0.68) {
            out.add(new int[] {r.chance(0.08) ? -999 : slot, r.nextInt(2), 4});
        } else if (q < 0.90) {
            out.add(new int[] {slot, 0, 6});
        } else {
            // QUICK_CRAFT: type 0 (split), 1 (one each), 2 (clone, creative)
            int type = creative && r.chance(0.3) ? 2 : r.nextInt(2);
            if (r.chance(0.05)) {
                type = 2; // invalid outside creative
            }
            boolean broken = r.chance(0.12);
            if (!broken || r.chance(0.5)) {
                out.add(new int[] {-999, AbstractContainerMenu.getQuickcraftMask(0, type), 5});
            }
            int n = r.chance(0.15) ? 1 : r.range(2, 6);
            Set<Integer> seen = new LinkedHashSet<>();
            for (int k = 0; k < n; k++) {
                int s = r.nextInt(len);
                if (r.chance(0.15) && !seen.isEmpty()) {
                    s = seen.iterator().next(); // the same slot again
                }
                seen.add(s);
                int t = broken && r.chance(0.3) ? r.nextInt(3) : type;
                out.add(new int[] {s, AbstractContainerMenu.getQuickcraftMask(1, t), 5});
            }
            if (broken && r.chance(0.3)) {
                out.add(new int[] {slot, r.nextInt(2), 0}); // another click in the middle of a drag
            }
            if (!broken || r.chance(0.5)) {
                out.add(new int[] {-999, AbstractContainerMenu.getQuickcraftMask(2, type), 5});
            }
        }
        return out;
    }

    // ------------------------------------------------------------------ state + packets

    private static void state(StringBuilder b, LocalPlayer player, AbstractContainerMenu w0, AbstractContainerMenu h) {
        b.append(",\"w0\":");
        slots(b, w0);
        b.append(",\"screen\":");
        if (h == w0) {
            b.append("null");
        } else {
            slots(b, h);
        }
        b.append(",\"carried\":\"").append(hex(h.getCarried())).append('"');
        b.append(",\"inv\":[");
        for (int k = 0; k < 41; k++) {
            b.append(k > 0 ? "," : "").append('"').append(hex(player.getInventory().getItem(k))).append('"');
        }
        b.append("],\"state_id\":").append(h.getStateId());
        b.append(",\"drag\":").append(Reflect.getInt(AbstractContainerMenu.class, h, "quickcraftStatus"));
        @SuppressWarnings("unchecked")
        List<net.minecraft.world.inventory.DataSlot> ds = (List<net.minecraft.world.inventory.DataSlot>) Reflect.get(AbstractContainerMenu.class, h, "dataSlots");
        if (!ds.isEmpty()) {
            b.append(",\"slots_data\":[");
            for (int k = 0; k < ds.size(); k++) {
                b.append(k > 0 ? "," : "").append(ds.get(k).get());
            }
            b.append(']');
        }
    }

    private static void slots(StringBuilder b, AbstractContainerMenu h) {
        b.append('[');
        for (int k = 0; k < h.slots.size(); k++) {
            b.append(k > 0 ? "," : "").append('"').append(hex(h.getSlot(k).getItem())).append('"');
        }
        b.append(']');
    }

    /** A stack as the server writes it, with enchantment maps in registry id order (see HashDump). */
    static String hex(ItemStack s) {
        RegistryFriendlyByteBuf buf = Harness.buf();
        canonicalWire(buf, s);
        return ByteBufUtil.hexDump(buf);
    }

    private static void canonicalWire(RegistryFriendlyByteBuf buf, ItemStack stack) {
        canonicalWire(buf, stack, false);
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    private static void canonicalWire(RegistryFriendlyByteBuf buf, ItemStack stack, boolean untrusted) {
        if (stack.isEmpty()) {
            buf.writeVarInt(0);
            return;
        }
        buf.writeVarInt(stack.getCount());
        Item.STREAM_CODEC.encode(buf, stack.typeHolder());
        DataComponentPatch patch = stack.getComponentsPatch();
        int added = 0;
        int removed = 0;
        for (Map.Entry<DataComponentType<?>, java.util.Optional<?>> e : patch.entrySet()) {
            if (e.getValue().isPresent()) {
                added++;
            } else {
                removed++;
            }
        }
        buf.writeVarInt(added);
        buf.writeVarInt(removed);
        // type id order: the patch map's own order depends on how the stack was built
        List<Map.Entry<DataComponentType<?>, java.util.Optional<?>>> entries = new ArrayList<>(patch.entrySet());
        entries.sort(Comparator.comparingInt(e -> BuiltInRegistries.DATA_COMPONENT_TYPE.getId(e.getKey())));
        for (Map.Entry<DataComponentType<?>, java.util.Optional<?>> e : entries) {
            if (e.getValue().isPresent()) {
                net.minecraft.core.component.DataComponentType.STREAM_CODEC.encode(buf, e.getKey());
                if (untrusted) {
                    // the value as a length-prefixed byte array of its trusted encoding
                    RegistryFriendlyByteBuf value = new RegistryFriendlyByteBuf(Unpooled.buffer(), buf.registryAccess());
                    writeValue(value, e.getKey(), e.getValue().get());
                    byte[] bytes = new byte[value.readableBytes()];
                    value.readBytes(bytes);
                    buf.writeByteArray(bytes);
                } else {
                    writeValue(buf, e.getKey(), e.getValue().get());
                }
            }
        }
        for (Map.Entry<DataComponentType<?>, java.util.Optional<?>> e : entries) {
            if (e.getValue().isEmpty()) {
                net.minecraft.core.component.DataComponentType.STREAM_CODEC.encode(buf, e.getKey());
            }
        }
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    private static void writeValue(RegistryFriendlyByteBuf buf, DataComponentType<?> type, Object v) {
        if (v instanceof net.minecraft.world.item.enchantment.ItemEnchantments ench) {
            var es = new ArrayList<>(ench.entrySet());
            var reg = buf.registryAccess().lookupOrThrow(Registries.ENCHANTMENT);
            es.sort(Comparator.comparingInt(x -> reg.getId(x.getKey().value())));
            buf.writeVarInt(es.size());
            for (var x : es) {
                net.minecraft.world.item.enchantment.Enchantment.STREAM_CODEC.encode(buf, x.getKey());
                buf.writeVarInt(x.getIntValue());
            }
        } else {
            ((net.minecraft.network.codec.StreamCodec) type.streamCodec()).encode(buf, v);
        }
    }

    /**
     * The packet as the client encodes it, with each {@code HashedStack}'s added components and
     * removed types in type id order (vanilla's IdentityHashMap / HashSet order is arbitrary; the
     * server compares contents). The changed-slot map keeps its own (deterministic) order.
     */
    private static String packetHex(Packet<?> q) {
        RegistryFriendlyByteBuf buf = Harness.buf();
        if (q instanceof ServerboundContainerClickPacket p) {
            // ServerboundContainerClickPacket.STREAM_CODEC's layout, written by hand so the hashed
            // stacks can be canonical; the changed-slot map in its own iteration order
            buf.writeVarInt(0);
            buf.writeVarInt(p.containerId());
            buf.writeVarInt(p.stateId());
            buf.writeShort(p.slotNum());
            buf.writeByte(p.buttonNum());
            ContainerInput.STREAM_CODEC.encode(buf, p.containerInput());
            buf.writeVarInt(p.changedSlots().size());
            for (Int2ObjectMap.Entry<HashedStack> e : p.changedSlots().int2ObjectEntrySet()) {
                buf.writeShort(e.getIntKey());
                HashedStack.STREAM_CODEC.encode(buf, canonical(e.getValue()));
            }
            HashedStack.STREAM_CODEC.encode(buf, canonical(p.carriedItem()));
            // the same bytes decode to the client's packet
            RegistryFriendlyByteBuf in = new RegistryFriendlyByteBuf(buf.copy(), buf.registryAccess());
            in.readVarInt();
            ServerboundContainerClickPacket back = ServerboundContainerClickPacket.STREAM_CODEC.decode(in);
            RegistryFriendlyByteBuf vanilla = Harness.buf();
            ServerboundContainerClickPacket.STREAM_CODEC.encode(vanilla, p);
            if (in.readableBytes() != 0 || vanilla.readableBytes() != buf.readableBytes() - 1 || !same(back, p)) {
                throw new IllegalStateException("hand-written container_click differs: " + p + " / " + back);
            }
        } else if (q instanceof ServerboundSetCreativeModeSlotPacket p) {
            // STREAM_CODEC: short slot, ItemStack.OPTIONAL_UNTRUSTED_STREAM_CODEC (each component
            // value length-prefixed), components in type id order
            buf.writeVarInt(1);
            buf.writeShort(p.slotNum());
            canonicalWire(buf, p.itemStack(), true);
            RegistryFriendlyByteBuf in = new RegistryFriendlyByteBuf(buf.copy(), buf.registryAccess());
            in.readVarInt();
            ServerboundSetCreativeModeSlotPacket back = ServerboundSetCreativeModeSlotPacket.STREAM_CODEC.decode(in);
            if (in.readableBytes() != 0 || back.slotNum() != p.slotNum() || !ItemStack.matches(back.itemStack(), p.itemStack())) {
                throw new IllegalStateException("hand-written set_creative_mode_slot differs: " + p + " / " + back);
            }
        } else if (q instanceof net.minecraft.network.protocol.game.ServerboundSelectTradePacket p) {
            buf.writeVarInt(3);
            buf.writeVarInt(p.getItem());
        } else if (q instanceof net.minecraft.network.protocol.game.ServerboundContainerButtonClickPacket p) {
            buf.writeVarInt(4);
            buf.writeVarInt(p.containerId());
            buf.writeVarInt(p.buttonId());
        } else if (q instanceof net.minecraft.network.protocol.game.ServerboundRenameItemPacket p) {
            buf.writeVarInt(5);
            buf.writeUtf(p.getName());
        } else {
            buf.writeVarInt(2);
            buf.writeUtf(q.type().toString());
        }
        return ByteBufUtil.hexDump(buf);
    }

    private static boolean same(ServerboundContainerClickPacket a, ServerboundContainerClickPacket b) {
        if (a.containerId() != b.containerId() || a.stateId() != b.stateId() || a.slotNum() != b.slotNum() || a.buttonNum() != b.buttonNum()
                || a.containerInput() != b.containerInput() || a.changedSlots().size() != b.changedSlots().size()
                || !canonical(a.carriedItem()).toString().equals(canonical(b.carriedItem()).toString())) {
            return false;
        }
        for (Int2ObjectMap.Entry<HashedStack> e : a.changedSlots().int2ObjectEntrySet()) {
            HashedStack o = b.changedSlots().get(e.getIntKey());
            if (o == null || !canonical(o).toString().equals(canonical(e.getValue()).toString())) {
                return false;
            }
        }
        return true;
    }

    private static HashedStack canonical(HashedStack h) {
        if (!(h instanceof HashedStack.ActualItem a)) {
            return h;
        }
        Map<DataComponentType<?>, Integer> added = new LinkedHashMap<>();
        a.components().addedComponents().entrySet().stream()
                .sorted(Comparator.comparingInt(e -> BuiltInRegistries.DATA_COMPONENT_TYPE.getId(e.getKey())))
                .forEach(e -> added.put(e.getKey(), e.getValue()));
        Set<DataComponentType<?>> removed = new LinkedHashSet<>();
        a.components().removedComponents().stream().sorted(Comparator.comparingInt(t -> BuiltInRegistries.DATA_COMPONENT_TYPE.getId(t)))
                .forEach(removed::add);
        return new HashedStack.ActualItem(a.item(), a.count(), new HashedPatchMap(added, removed));
    }

    private static byte[] unhex(String s) {
        byte[] out = new byte[s.length() / 2];
        for (int i = 0; i < out.length; i++) {
            out[i] = (byte) Integer.parseInt(s.substring(2 * i, 2 * i + 2), 16);
        }
        return out;
    }

    // ------------------------------------------------------------------ the creative inventory screen

    static final int CREATIVE_CASES = 1000;

    private static String creativeLine(Harness.Run run, long seed, int i) {
        return creativeCase(run, new Rng(seed * 1_000_303L + i + 11), "creative26_s" + seed + "_" + i);
    }

    /**
     * Window 0 in creative mode: the real {@code CreativeModeInventoryScreen} (allocated; the fields
     * its click code reads are set: minecraft, its {@code ItemPickerMenu}, the inventory tab with
     * the {@code SlotWrapper}s it builds, the destroy slot) with its {@code CreativeInventoryListener}
     * on the player's {@code InventoryMenu}: opening it broadcasts (every slot that differs from
     * the menu's last broadcast is sent as {@code set_creative_mode_slot}), clicks go through its
     * {@code slotClicked} (THROW handled by the screen, the rest through
     * {@code inventoryMenu.clicked} + {@code broadcastChanges}), and a server {@code container_set_slot}
     * while it is open is echoed back the same way.
     */
    static void creativeLines(long seed, int count, Consumer<String> sink) {
        Harness.Run run = newRun();
        // the creative packets carry whole stacks: no registries or item facts needed
        header(s -> {});
        for (int i = 0; i < count; i++) {
            sink.accept(creativeLine(run, seed, i));
        }
    }

    static int creativeReverseMismatches(long seed, int count) {
        Harness.Run run = newRun();
        header(s -> {});
        List<String> forward = new ArrayList<>(count);
        for (int i = 0; i < count; i++) {
            forward.add(creativeLine(run, seed, i));
        }
        int bad = 0;
        for (int i = count - 1; i >= 0; i--) {
            if (!creativeLine(run, seed, i).equals(forward.get(i))) {
                System.out.println("DIFF reverse-order rerun of 26.1 creative case " + i);
                bad++;
            }
        }
        return bad;
    }

    @SuppressWarnings("unchecked")
    private static String creativeCase(Harness.Run run, Rng r, String name) {
        LocalPlayer player = run.player;
        AbstractContainerMenu w0 = player.inventoryMenu;
        player.containerMenu = w0;
        call(AbstractContainerMenu.class, w0, "resetQuickCraft", new Class<?>[0]);
        if (mode != GameType.CREATIVE) {
            setMode(run, GameType.CREATIVE);
        }
        resetDropThrottle(player);
        // a fresh player's menu: nothing broadcast yet
        NonNullList<ItemStack> last = (NonNullList<ItemStack>) Reflect.get(AbstractContainerMenu.class, w0, "lastSlots");
        for (int k = 0; k < last.size(); k++) {
            last.set(k, ItemStack.EMPTY);
        }
        Harness.listener.handleSetHeldSlot(Harness.roundTrip(ClientboundSetHeldSlotPacket.STREAM_CODEC, new ClientboundSetHeldSlotPacket(r.nextInt(9))));
        double fill = 0.2 + r.nextDouble() * 0.6;
        List<ItemStack> w0Items = new ArrayList<>();
        for (int k = 0; k < 46; k++) {
            ItemStack s = k < 5 ? (r.chance(0.15) ? stack(r, ITEMS) : ItemStack.EMPTY)
                    : k < 9 ? (r.chance(0.3) ? stack(r, ARMOR) : ItemStack.EMPTY) : maybe(r, fill, ITEMS);
            w0Items.add(s);
        }
        content(0, r.nextInt(32768), w0Items, ItemStack.EMPTY);
        // when the screen opened, earlier broadcasts may have happened (some slots already known)
        if (r.chance(0.5)) {
            for (int k = 0; k < 46; k++) {
                if (r.chance(0.5)) {
                    last.set(k, w0.getSlot(k).getItem().copy());
                }
            }
        }
        StringBuilder b = new StringBuilder(8192);
        b.append("{\"type\":\"creative\",\"case\":\"").append(name).append("\",\"selected\":").append(player.getInventory().getSelectedSlot());
        b.append(",\"last\":[");
        for (int k = 0; k < 46; k++) {
            b.append(k > 0 ? "," : "").append('"').append(hex(last.get(k))).append('"');
        }
        b.append(']');
        state(b, player, w0, w0);
        // open the screen
        net.minecraft.client.gui.screens.inventory.CreativeModeInventoryScreen screen =
                Reflect.allocate(net.minecraft.client.gui.screens.inventory.CreativeModeInventoryScreen.class);
        Reflect.set(net.minecraft.client.gui.screens.Screen.class, screen, "minecraft", Harness.client);
        var picker = new net.minecraft.client.gui.screens.inventory.CreativeModeInventoryScreen.ItemPickerMenu(player);
        Reflect.set(net.minecraft.client.gui.screens.inventory.AbstractContainerScreen.class, screen, "menu", picker);
        Reflect.set(net.minecraft.client.gui.screens.inventory.AbstractContainerScreen.class, screen, "itemSlotMouseActions", new ArrayList<>());
        net.minecraft.world.item.CreativeModeTab inventoryTab = null;
        for (net.minecraft.world.item.CreativeModeTab t : BuiltInRegistries.CREATIVE_MODE_TAB) {
            if (t.getType() == net.minecraft.world.item.CreativeModeTab.Type.INVENTORY) {
                inventoryTab = t;
            }
        }
        Reflect.set(net.minecraft.client.gui.screens.inventory.CreativeModeInventoryScreen.class, null, "selectedTab", inventoryTab);
        picker.slots.clear();
        List<net.minecraft.world.inventory.Slot> wrappers = new ArrayList<>();
        try {
            Class<?> wc = Class.forName("net.minecraft.client.gui.screens.inventory.CreativeModeInventoryScreen$SlotWrapper");
            var ctor = wc.getDeclaredConstructor(net.minecraft.world.inventory.Slot.class, int.class, int.class, int.class);
            ctor.setAccessible(true);
            for (int k = 0; k < w0.slots.size(); k++) {
                var wrap = (net.minecraft.world.inventory.Slot) ctor.newInstance(w0.slots.get(k), k, 0, 0);
                wrappers.add(wrap);
                picker.slots.add(wrap);
            }
        } catch (ReflectiveOperationException ex) {
            throw new IllegalStateException(ex);
        }
        Object container = Reflect.get(net.minecraft.client.gui.screens.inventory.CreativeModeInventoryScreen.class, null, "CONTAINER");
        Reflect.set(net.minecraft.client.gui.screens.inventory.CreativeModeInventoryScreen.class, screen, "destroyItemSlot",
                new net.minecraft.world.inventory.Slot((net.minecraft.world.Container) container, 0, 173, 112));
        Reflect.set(net.minecraft.client.Minecraft.class, Harness.client, "screen", screen);
        var listener = new net.minecraft.client.gui.screens.inventory.CreativeInventoryListener(Harness.client);
        Harness.connection.sent.clear();
        w0.addSlotListener(listener);
        b.append(",\"open\":[");
        packets(b);
        b.append("],\"events\":[");
        int events = r.range(1, 10);
        int written = 0;
        for (int e = 0; e < events; e++) {
            List<int[]> seq;
            boolean server = r.chance(0.2);
            if (server) {
                int slot = 5 + r.nextInt(41);
                ItemStack st = maybe(r, 0.7, ITEMS);
                Harness.connection.sent.clear();
                Harness.listener.handleContainerSetSlot(Harness.roundTrip(
                        net.minecraft.network.protocol.game.ClientboundContainerSetSlotPacket.STREAM_CODEC,
                        new net.minecraft.network.protocol.game.ClientboundContainerSetSlotPacket(0, w0.incrementStateId(), slot, st)));
                b.append(written++ > 0 ? "," : "").append("{\"server_slot\":").append(slot).append(",\"item\":\"").append(hex(st)).append("\",\"packets\":[");
                packets(b);
                b.append(']');
                state(b, player, w0, w0);
                b.append('}');
                continue;
            }
            // the screen only shows window 0's slots 5..45 (the crafting grid is off screen)
            seq = gesture(r, w0, true);
            for (int[] g : seq) {
                int slotId = g[0] >= 0 && g[0] < 5 ? 5 + r.nextInt(41) : g[0];
                var slot = slotId >= 0 ? wrappers.get(slotId) : null;
                Harness.connection.sent.clear();
                call(net.minecraft.client.gui.screens.inventory.CreativeModeInventoryScreen.class, screen, "slotClicked",
                        new Class<?>[] {net.minecraft.world.inventory.Slot.class, int.class, int.class, ContainerInput.class},
                        slot, slotId, g[1], ContainerInput.values()[g[2]]);
                b.append(written++ > 0 ? "," : "").append("{\"slot\":").append(slotId).append(",\"button\":").append(g[1])
                        .append(",\"mode\":").append(g[2]).append(",\"packets\":[");
                packets(b);
                b.append(']');
                state(b, player, w0, w0);
                b.append('}');
            }
        }
        b.append("]}");
        w0.removeSlotListener(listener);
        Reflect.set(net.minecraft.client.Minecraft.class, Harness.client, "screen", null);
        return b.toString();
    }

    private static void packets(StringBuilder b) {
        int pk = 0;
        for (Packet<?> q : Harness.connection.sent) {
            b.append(pk++ > 0 ? "," : "").append('"').append(packetHex(q)).append('"');
        }
    }

    /** Calls a (possibly protected) method with exactly these parameter types. */
    static Object call(Class<?> owner, Object target, String name, Class<?>[] types, Object... args) {
        try {
            java.lang.reflect.Method m = owner.getDeclaredMethod(name, types);
            m.setAccessible(true);
            return m.invoke(target, args);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
    }
}
