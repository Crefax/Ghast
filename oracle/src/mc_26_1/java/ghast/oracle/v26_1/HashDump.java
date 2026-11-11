package ghast.oracle.v26_1;

import com.google.gson.JsonElement;
import com.google.gson.JsonParser;
import com.mojang.serialization.Codec;
import com.mojang.serialization.DataResult;
import com.mojang.serialization.JsonOps;
import ghast.oracle.Json;
import ghast.oracle.Rng;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import net.minecraft.core.Holder;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.component.DataComponentPatch;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.component.TypedDataComponent;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.HashedPatchMap;
import net.minecraft.network.HashedStack;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.resources.RegistryOps;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.function.Consumer;

/**
 * 26.1 {@code HashedStack} vectors: random item stacks whose component patches cover every data
 * component type, with the stack's wire bytes as the server sends it
 * ({@code ItemStack.OPTIONAL_STREAM_CODEC}) and the {@code HashedStack} the real client computes for a
 * container click ({@code HashedStack.create} with {@code ClientPacketListener}'s hash generator:
 * CRC32C {@code HashOps} over the synced registries), plus the registry name lists that holders in
 * the bytes refer to. Schema: oracle/README.md, "hashed_stacks.jsonl.gz (26.1)".
 *
 * <p>Component values come from three pools, all validated through the type's own codecs (the
 * value must round-trip the network codec and hash without error): every item's default value of
 * the type, generic JSON values the type's codec accepts, and curated JSON for structured types.
 */
final class HashDump {
    static final int CASES = 4000;
    private static final boolean DEBUG = false;

    private HashDump() {}

    /** Generic JSON tried against every component codec. */
    private static final String[] GENERIC = {"0", "1", "3", "7", "16", "64", "99", "-1", "255", "1000", "2147483647", "-2147483648",
            "0.0", "0.5", "1.5", "-2.25", "20.0", "1e10", "true", "false", "{}", "[]", "\"\"", "\"x\"",
            "\"minecraft:stone\"", "\"minecraft:diamond\"", "\"minecraft:diamond_sword\"", "\"minecraft:sharpness\"",
            "\"minecraft:block.note_block.harp\"", "\"minecraft:entity.item.break\"", "\"minecraft:red\"", "\"red\"",
            "\"epic\"", "\"rare\"", "\"common\"", "\"uncommon\"", "\"white\"", "\"black\"", "\"minecraft:white\"",
            "\"#minecraft:planks\"", "\"#minecraft:mineable/pickaxe\"", "\"minecraft:creeper\"", "\"minecraft:mainhand\"",
            "[\"minecraft:stone\"]", "{\"text\":\"Hi\"}", "\"Hello\"", "[1,2,3]", "{\"a\":1}", "\"minecraft:husk\"",
            "\"minecraft:temperate\"", "\"minecraft:warm\"", "\"minecraft:cold\"", "\"minecraft:pale\"", "\"minecraft:kebab\"",
            "\"minecraft:ponder_goat_horn\"", "\"minecraft:13\"", "\"minecraft:iron\"", "\"minecraft:coast\"",
            "\"minecraft:swiftness\"", "\"minecraft:in_fire\"", "\"minecraft:oak\"", "\"minecraft:ashen\""};

    /** Curated JSON per component type (structured values the generic pool misses). */
    private static final Map<String, String[]> CURATED = new LinkedHashMap<>();

    static {
        c("salmon/size", "\"small\"", "\"medium\"", "\"large\"");
        c("parrot/variant", "\"red_blue\"", "\"blue\"", "\"green\"", "\"yellow_blue\"", "\"gray\"");
        c("tropical_fish/pattern", "\"kob\"", "\"sunstreak\"", "\"snooper\"", "\"dasher\"", "\"brinely\"", "\"spotty\"",
                "\"flopper\"", "\"stripey\"", "\"glitter\"", "\"blockfish\"", "\"betty\"", "\"clayfish\"");
        c("tropical_fish/base_color", "\"orange\"", "\"lime\"");
        c("tropical_fish/pattern_color", "\"cyan\"", "\"purple\"");
        c("axolotl/variant", "\"lucy\"", "\"wild\"", "\"gold\"", "\"cyan\"", "\"blue\"");
        c("fox/variant", "\"red\"", "\"snow\"");
        c("mooshroom/variant", "\"red\"", "\"brown\"");
        c("rabbit/variant", "\"brown\"", "\"white\"", "\"black\"", "\"white_splotched\"", "\"gold\"", "\"salt\"", "\"evil\"");
        c("horse/variant", "\"white\"", "\"creamy\"", "\"chestnut\"", "\"brown\"", "\"black\"", "\"gray\"", "\"dark_brown\"");
        c("llama/variant", "\"creamy\"", "\"white\"", "\"brown\"", "\"gray\"");
        c("written_book_content", "{\"title\":{\"raw\":\"R\",\"filtered\":\"F\"},\"author\":\"B\",\"pages\":[{\"raw\":\"x\"}]}");
        c("trim", "{\"material\":\"minecraft:diamond\",\"pattern\":\"minecraft:sentry\"}");
        c("firework_explosion", "{\"shape\":\"creeper\"}", "{\"shape\":\"large_ball\",\"colors\":[1,2,3]}");
        c("container_loot", "{\"loot_table\":\"minecraft:chests/village/village_weaponsmith\"}");
        c("block_entity_data", "{\"id\":\"minecraft:sign\",\"is_waxed\":1b}");
        c("custom_data", "{\"a\":1,\"b\":\"x\",\"c\":[1,2],\"d\":{\"e\":1.5}}", "{\"Tag\":\"v\"}");
        c("custom_name", "\"Name\"", "{\"text\":\"Bold\",\"bold\":true,\"color\":\"red\"}", "[\"a\",{\"text\":\"b\",\"italic\":false}]");
        c("item_name", "\"Item\"", "{\"translate\":\"item.minecraft.diamond\"}");
        c("lore", "[\"one\",\"two\"]", "[{\"text\":\"x\",\"color\":\"#123456\"}]");
        // Enchantment maps: vanilla keeps them in an Object2IntOpenHashMap keyed by holders (identity
        // hash), so its own wire order varies between JVM runs; the dump writes them sorted by
        // registry id (see canonicalWire) and keeps multi-entry maps.
        c("enchantments", "{\"minecraft:sharpness\":5}", "{\"minecraft:efficiency\":3}", "{\"minecraft:protection\":255}",
                "{\"minecraft:efficiency\":5,\"minecraft:unbreaking\":3}",
                "{\"minecraft:sharpness\":5,\"minecraft:looting\":3,\"minecraft:fire_aspect\":2}",
                "{\"minecraft:protection\":4,\"minecraft:unbreaking\":3,\"minecraft:mending\":1,\"minecraft:thorns\":3}",
                "{\"minecraft:efficiency\":5,\"minecraft:fortune\":3,\"minecraft:unbreaking\":3,\"minecraft:mending\":1,\"minecraft:vanishing_curse\":1}",
                "{\"minecraft:binding_curse\":1,\"minecraft:vanishing_curse\":1}");
        c("stored_enchantments", "{\"minecraft:mending\":1}", "{\"minecraft:fortune\":3}",
                "{\"minecraft:fortune\":3,\"minecraft:looting\":2}",
                "{\"minecraft:power\":5,\"minecraft:punch\":2,\"minecraft:flame\":1,\"minecraft:infinity\":1}");
        c("debug_stick_state", "{\"minecraft:oak_stairs\":\"facing\",\"minecraft:lever\":\"powered\",\"minecraft:oak_door\":\"half\"}",
                "{\"minecraft:oak_stairs\":\"half\",\"minecraft:stone_slab\":\"type\"}");
        c("can_place_on", "{\"blocks\":\"minecraft:stone\"}", "[{\"blocks\":[\"minecraft:dirt\",\"minecraft:grass_block\"]}]");
        c("can_break", "{\"blocks\":\"#minecraft:logs\"}", "[{\"blocks\":\"minecraft:stone\",\"state\":{}}]");
        c("attribute_modifiers", "[{\"type\":\"minecraft:attack_damage\",\"id\":\"oracle:a\",\"amount\":2.5,\"operation\":\"add_value\",\"slot\":\"mainhand\"}]",
                "[{\"type\":\"minecraft:movement_speed\",\"id\":\"oracle:b\",\"amount\":-0.1,\"operation\":\"add_multiplied_total\"}]");
        c("custom_model_data", "{\"floats\":[1.0,2.5],\"flags\":[true],\"strings\":[\"a\"],\"colors\":[16711680]}");
        c("tooltip_display", "{\"hide_tooltip\":true}", "{\"hidden_components\":[\"minecraft:enchantments\",\"minecraft:damage\"]}");
        c("repair_cost", "5", "39");
        c("damage", "0", "10", "500");
        c("dyed_color", "16711680", "3949738");
        c("map_color", "4210752");
        c("map_id", "7");
        c("map_decorations", "{\"a\":{\"type\":\"minecraft:mansion\",\"x\":1.0,\"z\":2.0,\"rotation\":90.0}}");
        c("charged_projectiles", "[{\"id\":\"minecraft:arrow\",\"count\":1}]");
        c("bundle_contents", "[{\"id\":\"minecraft:stone\",\"count\":3},{\"id\":\"minecraft:diamond\",\"count\":1}]");
        c("potion_contents", "{\"potion\":\"minecraft:swiftness\"}", "{\"custom_color\":255,\"custom_effects\":[{\"id\":\"minecraft:speed\",\"amplifier\":1,\"duration\":200}]}");
        c("suspicious_stew_effects", "[{\"id\":\"minecraft:night_vision\",\"duration\":100}]");
        c("writable_book_content", "{\"pages\":[\"page one\",\"page two\"]}");
        c("written_book_content", "{\"title\":\"T\",\"author\":\"A\",\"generation\":1,\"pages\":[\"p\"],\"resolved\":true}");
        c("trim", "{\"material\":\"minecraft:gold\",\"pattern\":\"minecraft:coast\"}");
        c("debug_stick_state", "{\"minecraft:oak_stairs\":\"facing\"}");
        c("entity_data", "{\"id\":\"minecraft:pig\",\"NoAI\":1b}");
        c("bucket_entity_data", "{\"Health\":5.0f}");
        c("block_entity_data", "{\"id\":\"minecraft:chest\",\"Lock\":\"\"}");
        c("instrument", "\"minecraft:ponder_goat_horn\"");
        c("provides_trim_material", "\"minecraft:gold\"");
        c("ominous_bottle_amplifier", "2");
        c("jukebox_playable", "\"minecraft:13\"");
        c("provides_banner_patterns", "\"#minecraft:pattern_item/flower\"");
        c("recipes", "[\"minecraft:stick\"]");
        c("lodestone_tracker", "{\"target\":{\"dimension\":\"minecraft:overworld\",\"pos\":[1,2,3]},\"tracked\":false}", "{}");
        c("firework_explosion", "{\"shape\":\"star\",\"colors\":[255],\"fade_colors\":[65280],\"has_trail\":true,\"has_twinkle\":false}");
        c("fireworks", "{\"flight_duration\":2,\"explosions\":[{\"shape\":\"burst\",\"colors\":[1]}]}");
        c("profile", "\"Notch\"", "{\"name\":\"abc\",\"id\":[1,2,3,4]}");
        c("note_block_sound", "\"minecraft:block.note_block.bell\"");
        c("banner_patterns", "[{\"pattern\":\"minecraft:creeper\",\"color\":\"red\"}]");
        c("base_color", "\"blue\"");
        c("pot_decorations", "[\"minecraft:brick\",\"minecraft:angler_pottery_sherd\"]");
        c("container", "[{\"slot\":0,\"item\":{\"id\":\"minecraft:stone\",\"count\":2}}]");
        c("block_state", "{\"facing\":\"north\",\"half\":\"top\"}");
        c("bees", "[{\"entity_data\":{\"id\":\"minecraft:bee\"},\"ticks_in_hive\":10,\"min_ticks_in_hive\":20}]");
        c("lock", "{\"items\":\"minecraft:stick\"}");
        c("container_loot", "{\"loot_table\":\"minecraft:chests/simple_dungeon\",\"seed\":5}");
        c("food", "{\"nutrition\":3,\"saturation\":1.5,\"can_always_eat\":true}");
        c("consumable", "{\"consume_seconds\":0.8,\"animation\":\"drink\"}");
        c("use_remainder", "{\"id\":\"minecraft:bowl\",\"count\":1}");
        c("use_cooldown", "{\"seconds\":1.5}", "{\"seconds\":0.25,\"cooldown_group\":\"oracle:g\"}");
        c("tool", "{\"rules\":[{\"blocks\":\"#minecraft:mineable/pickaxe\",\"speed\":4.0,\"correct_for_drops\":true}],\"default_mining_speed\":1.5,\"damage_per_block\":2}");
        c("weapon", "{\"item_damage_per_attack\":3,\"disable_blocking_for_seconds\":2.0}");
        c("enchantable", "{\"value\":15}");
        c("equippable", "{\"slot\":\"head\"}", "{\"slot\":\"chest\",\"equip_sound\":\"minecraft:item.armor.equip_gold\",\"dispensable\":false}");
        c("repairable", "{\"items\":\"minecraft:diamond\"}", "{\"items\":\"#minecraft:planks\"}");
        c("glider", "{}");
        c("tooltip_style", "\"oracle:style\"");
        c("death_protection", "{}", "{\"death_effects\":[{\"type\":\"minecraft:clear_all_effects\"}]}");
        c("blocks_attacks", "{\"block_delay_seconds\":0.25}");
        c("damage_resistant", "{\"types\":\"#minecraft:is_fire\"}");
        c("break_sound", "\"minecraft:entity.item.break\"");
        c("rarity", "\"epic\"");
        c("item_model", "\"minecraft:stick\"");
        c("max_stack_size", "16", "99");
        c("max_damage", "100", "2031");
        c("enchantment_glint_override", "true", "false");
        c("unbreakable", "{}");
        c("intangible_projectile", "{}");
        c("creative_slot_lock", "{}");
        c("map_post_processing", "0");
    }

    private static void c(String type, String... json) {
        CURATED.put("minecraft:" + type, json);
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static void lines(Consumer<String> sink) {
        Harness.Run run = new Harness.Run(new ghast.oracle.Scenario("hash_dump").spawn(0.5, 100, 0.5, 0, 0));
        RegistryAccess access = Harness.listener.registryAccess();
        HashedPatchMap.HashGenerator hasher = Harness.listener.decoratedHashOpsGenenerator();
        RegistryOps<JsonElement> jsonOps = access.createSerializationContext(JsonOps.INSTANCE);

        // Registry name lists (ids in order) of every registry the client holds.
        access.registries().sorted((a, b) -> a.key().identifier().toString().compareTo(b.key().identifier().toString()))
                .forEach(e -> {
                    @SuppressWarnings("unchecked")
                    Registry<Object> reg = (Registry<Object>) e.value();
                    Json j = new Json().beginObject().field("type", "registry").field("name", e.key().identifier().toString());
                    j.key("entries").beginArray();
                    for (int i = 0; i < reg.size(); i++) {
                        j.value(reg.getKey(reg.byId(i)).toString());
                    }
                    j.endArray();
                    sink.accept(j.endObject().toString());
                });

        List<String> registryNames = new ArrayList<>();
        access.registries().sorted((a, b) -> a.key().identifier().toString().compareTo(b.key().identifier().toString())).forEach(e -> {
            if (e.value().size() <= 300) {
                @SuppressWarnings("unchecked")
                Registry<Object> reg = (Registry<Object>) e.value();
                for (int i = 0; i < reg.size(); i++) {
                    registryNames.add("\"" + reg.getKey(reg.byId(i)) + "\"");
                }
            }
        });

        // Value pools per component type.
        List<DataComponentType<?>> types = new ArrayList<>();
        Map<DataComponentType<?>, Map<String, Object>> pools = new LinkedHashMap<>();
        Map<DataComponentType<?>, int[]> sources = new LinkedHashMap<>();
        for (DataComponentType<?> t : BuiltInRegistries.DATA_COMPONENT_TYPE) {
            types.add(t);
            pools.put(t, new TreeMap<>());
            sources.put(t, new int[3]);
        }
        for (Item item : BuiltInRegistries.ITEM) {
            for (TypedDataComponent<?> tc : item.components()) {
                if (addValue(pools.get(tc.type()), (DataComponentType) tc.type(), tc.value(), access, hasher, jsonOps)) {
                    sources.get(tc.type())[0]++;
                }
            }
        }
        for (DataComponentType<?> t : types) {
            if (t.isTransient()) {
                continue;
            }
            String name = BuiltInRegistries.DATA_COMPONENT_TYPE.getKey(t).toString();
            for (String g : GENERIC) {
                if (tryJson(t, g, pools.get(t), access, hasher, jsonOps)) {
                    sources.get(t)[1]++;
                }
            }
            // Every registry entry name as a string (holder-valued components: variants, instruments, ...).
            for (String g : registryNames) {
                if (tryJson(t, g, pools.get(t), access, hasher, jsonOps)) {
                    sources.get(t)[1]++;
                }
            }
            for (String g : CURATED.getOrDefault(name, new String[0])) {
                if (tryJson(t, g, pools.get(t), access, hasher, jsonOps)) {
                    sources.get(t)[2]++;
                } else if (DEBUG) {
                    System.err.println("curated value rejected: " + name + " " + g);
                }
            }
        }

        // Random stacks: case k guarantees type k mod n (when it has values), plus random others and removals.
        List<DataComponentType<?>> usable = new ArrayList<>();
        for (DataComponentType<?> t : types) {
            if (!pools.get(t).isEmpty()) {
                usable.add(t);
            }
        }
        List<Item> items = new ArrayList<>();
        for (Item it : BuiltInRegistries.ITEM) {
            if (it != net.minecraft.world.item.Items.AIR) {
                items.add(it);
            }
        }
        Map<DataComponentType<?>, int[]> coverage = new LinkedHashMap<>();
        for (DataComponentType<?> t : types) {
            coverage.put(t, new int[2]);
        }
        Rng rng = new Rng(0x6861736865645f73L);
        int written = 0;
        int skipped = 0;
        for (int k = 0; written < CASES && k < CASES * 4; k++) {
            Item item = items.get(rng.nextInt(items.size()));
            int count = rng.chance(0.6) ? rng.range(1, 64) : rng.chance(0.7) ? rng.range(65, 99) : rng.range(100, 5000);
            DataComponentPatch.Builder b = DataComponentPatch.builder();
            List<DataComponentType<?>> chosen = new ArrayList<>();
            if (k % 7 != 6) {
                chosen.add(usable.get(k % usable.size()));
            }
            int extra = rng.chance(0.3) ? 0 : rng.range(1, rng.chance(0.2) ? 12 : 4);
            for (int i = 0; i < extra; i++) {
                chosen.add(usable.get(rng.nextInt(usable.size())));
            }
            for (DataComponentType<?> t : chosen) {
                List<Object> vals = new ArrayList<>(pools.get(t).values());
                b.set((DataComponentType) t, vals.get(rng.nextInt(vals.size())));
            }
            List<TypedDataComponent<?>> defaults = new ArrayList<>();
            item.components().forEach(defaults::add);
            // DataComponentMap iteration order is identity-hash based: sort for a deterministic pick.
            defaults.sort(java.util.Comparator.comparingInt(tc -> BuiltInRegistries.DATA_COMPONENT_TYPE.getId(tc.type())));
            int removals = rng.chance(0.6) ? 0 : rng.range(1, 3);
            for (int i = 0; i < removals && !defaults.isEmpty(); i++) {
                b.remove(defaults.get(rng.nextInt(defaults.size())).type());
            }
            if (rng.chance(0.25)) {
                // A removal of a type the item does not have by default.
                b.remove(usable.get(rng.nextInt(usable.size())));
            }
            ItemStack stack;
            byte[] wire;
            try {
                stack = new ItemStack(BuiltInRegistries.ITEM.wrapAsHolder(item), count, b.build());
                byte[] vanillaWire = encode(access, buf -> ItemStack.OPTIONAL_STREAM_CODEC.encode(buf, stack));
                wire = encode(access, buf -> canonicalWire(buf, stack));
                if (!holderMapsWithSeveralEntries(stack) && !java.util.Arrays.equals(wire, vanillaWire)) {
                    throw new IllegalStateException("canonical wire differs from vanilla's for " + stack);
                }
                // The client's view: the stack decoded from the server's bytes.
                RegistryFriendlyByteBuf in = new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(wire), access);
                ItemStack decoded = ItemStack.OPTIONAL_STREAM_CODEC.decode(in);
                if (in.readableBytes() != 0 || !ItemStack.matches(stack, decoded)) {
                    continue;
                }
                HashedStack hashed = HashedStack.create(decoded, hasher);
                // The hash does not depend on the map's iteration order: the original stack (built
                // from the pool, its own map order) hashes the same as the decoded one.
                HashedPatchMap orig = ((HashedStack.ActualItem) HashedStack.create(stack, hasher)).components();
                HashedPatchMap dec = ((HashedStack.ActualItem) hashed).components();
                // (IdentityHashMap equality compares the Integer values by reference: compare as HashMaps.)
                if (!new java.util.HashMap<>(orig.addedComponents()).equals(new java.util.HashMap<>(dec.addedComponents()))
                        || !new java.util.HashSet<>(orig.removedComponents()).equals(new java.util.HashSet<>(dec.removedComponents()))) {
                    throw new IllegalStateException("hash depends on map order: " + stack + " original "
                            + ((HashedStack.ActualItem) HashedStack.create(stack, hasher)).components() + " decoded "
                            + ((HashedStack.ActualItem) hashed).components() + " patch " + stack.getComponentsPatch() + " / " + decoded.getComponentsPatch());
                }
                // Vanilla writes the added map / removed set in IdentityHashMap / HashSet order (arbitrary;
                // the server compares contents). The vector uses type id order.
                HashedStack.ActualItem actual = (HashedStack.ActualItem) hashed;
                Map<DataComponentType<?>, Integer> sortedAdded = new LinkedHashMap<>();
                actual.components().addedComponents().entrySet().stream()
                        .sorted(java.util.Comparator.comparingInt(e -> BuiltInRegistries.DATA_COMPONENT_TYPE.getId(e.getKey())))
                        .forEach(e -> sortedAdded.put(e.getKey(), e.getValue()));
                java.util.Set<DataComponentType<?>> sortedRemoved = new java.util.LinkedHashSet<>();
                actual.components().removedComponents().stream()
                        .sorted(java.util.Comparator.comparingInt(t -> BuiltInRegistries.DATA_COMPONENT_TYPE.getId(t)))
                        .forEach(sortedRemoved::add);
                HashedStack canonical = new HashedStack.ActualItem(actual.item(), actual.count(), new HashedPatchMap(sortedAdded, sortedRemoved));
                byte[] hashedWire = encode(access, buf -> HashedStack.STREAM_CODEC.encode(buf, canonical));
                DataComponentPatch patch = decoded.getComponentsPatch();
                Json j = new Json().beginObject().field("type", "stack").field("k", written);
                j.field("item", BuiltInRegistries.ITEM.getKey(item).toString()).field("item_id", BuiltInRegistries.ITEM.getId(item));
                j.field("count", count);
                j.field("wire", ByteBufUtil.hexDump(wire));
                j.key("added").beginArray();
                HashedPatchMap map = ((HashedStack.ActualItem) hashed).components();
                Map<String, Object[]> added = new TreeMap<>();
                for (Map.Entry<DataComponentType<?>, java.util.Optional<?>> e : patch.entrySet()) {
                    DataComponentType<?> t = e.getKey();
                    String name = BuiltInRegistries.DATA_COMPONENT_TYPE.getKey(t).toString();
                    if (e.getValue().isPresent()) {
                        Integer hash = map.addedComponents().get(t);
                        String json = jsonOf((DataComponentType) t, e.getValue().get(), jsonOps);
                        if (json != null) {
                            json = canonical(JsonParser.parseString(json)).toString();
                        }
                        added.put(name, new Object[] {BuiltInRegistries.DATA_COMPONENT_TYPE.getId(t), hash, json});
                        coverage.get(t)[0]++;
                    } else {
                        coverage.get(t)[1]++;
                    }
                }
                added.forEach((name, v) -> {
                    j.beginObject().field("component", name).field("id", (int) v[0]).field("hash", (int) v[1]);
                    j.key("json").raw(v[2] == null ? "null" : (String) v[2]);
                    j.endObject();
                });
                j.endArray().key("removed").beginArray();
                map.removedComponents().stream().map(t -> BuiltInRegistries.DATA_COMPONENT_TYPE.getKey(t).toString()).sorted()
                        .forEach(j::value);
                j.endArray();
                j.field("hashed_wire", ByteBufUtil.hexDump(hashedWire));
                sink.accept(j.endObject().toString());
                written++;
            } catch (IllegalStateException e) {
                throw e;
            } catch (RuntimeException e) {
                // A combination the codecs reject (e.g. a value only valid for other items): skip.
                skipped++;
            }
        }
        // Per component type: id, name, transient, value pool size by source, cases where added / removed.
        for (DataComponentType<?> t : types) {
            int[] s = sources.get(t);
            int[] c = coverage.get(t);
            Json j = new Json().beginObject().field("type", "component_type").field("id", BuiltInRegistries.DATA_COMPONENT_TYPE.getId(t));
            j.field("name", BuiltInRegistries.DATA_COMPONENT_TYPE.getKey(t).toString()).field("transient", t.isTransient());
            j.field("values", pools.get(t).size()).field("from_defaults", s[0]).field("from_generic", s[1]).field("from_curated", s[2]);
            j.field("cases_added", c[0]).field("cases_removed", c[1]);
            sink.accept(j.endObject().toString());
        }
    }

    /** Any enchantment map in the stack's patch has two or more entries. */
    private static boolean holderMapsWithSeveralEntries(ItemStack stack) {
        for (Map.Entry<DataComponentType<?>, java.util.Optional<?>> e : stack.getComponentsPatch().entrySet()) {
            if (e.getValue().orElse(null) instanceof net.minecraft.world.item.enchantment.ItemEnchantments ench && ench.size() > 1) {
                return true;
            }
        }
        return false;
    }

    /**
     * {@code ItemStack.OPTIONAL_STREAM_CODEC}'s layout (count, item holder, patch: added count,
     * removed count, added (type, value)..., removed type...) with enchantment maps written sorted by
     * registry id instead of vanilla's identity-hash order. Byte-identical to vanilla for every stack
     * without a multi-entry enchantment map (checked).
     */
    @SuppressWarnings({"unchecked", "rawtypes"})
    private static void canonicalWire(RegistryFriendlyByteBuf buf, ItemStack stack) {
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
        for (Map.Entry<DataComponentType<?>, java.util.Optional<?>> e : patch.entrySet()) {
            if (e.getValue().isPresent()) {
                DataComponentType.STREAM_CODEC.encode(buf, e.getKey());
                Object v = e.getValue().get();
                if (v instanceof net.minecraft.world.item.enchantment.ItemEnchantments ench) {
                    List<it.unimi.dsi.fastutil.objects.Object2IntMap.Entry<Holder<net.minecraft.world.item.enchantment.Enchantment>>> es =
                            new ArrayList<>(ench.entrySet());
                    Registry<net.minecraft.world.item.enchantment.Enchantment> reg =
                            buf.registryAccess().lookupOrThrow(net.minecraft.core.registries.Registries.ENCHANTMENT);
                    es.sort(java.util.Comparator.comparingInt(x -> reg.getId(x.getKey().value())));
                    buf.writeVarInt(es.size());
                    for (var x : es) {
                        net.minecraft.world.item.enchantment.Enchantment.STREAM_CODEC.encode(buf, x.getKey());
                        buf.writeVarInt(x.getIntValue());
                    }
                } else {
                    ((net.minecraft.network.codec.StreamCodec) e.getKey().streamCodec()).encode(buf, v);
                }
            }
        }
        for (Map.Entry<DataComponentType<?>, java.util.Optional<?>> e : patch.entrySet()) {
            if (e.getValue().isEmpty()) {
                DataComponentType.STREAM_CODEC.encode(buf, e.getKey());
            }
        }
    }

    private interface Writer {
        void write(RegistryFriendlyByteBuf buf);
    }

    private static byte[] encode(RegistryAccess access, Writer w) {
        RegistryFriendlyByteBuf buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), access);
        w.write(buf);
        byte[] out = new byte[buf.readableBytes()];
        buf.readBytes(out);
        return out;
    }

    private static <T> String jsonOf(DataComponentType<T> t, Object value, RegistryOps<JsonElement> ops) {
        Codec<T> codec = t.codec();
        if (codec == null) {
            return null;
        }
        @SuppressWarnings("unchecked")
        DataResult<JsonElement> r = codec.encodeStart(ops, (T) value);
        return r.result().map(JsonElement::toString).orElse(null);
    }

    private static <T> boolean tryJson(DataComponentType<T> t, String json, Map<String, Object> pool, RegistryAccess access,
                                       HashedPatchMap.HashGenerator hasher, RegistryOps<JsonElement> ops) {
        Codec<T> codec = t.codec();
        if (codec == null) {
            return false;
        }
        JsonElement el;
        try {
            el = JsonParser.parseString(json);
        } catch (RuntimeException e) {
            return false;
        }
        DataResult<T> r = codec.parse(ops, el);
        return r.result().map(v -> addValue(pool, t, v, access, hasher, ops)).orElse(false);
    }

    /** Adds a value when it round-trips the network codec and hashes; keyed by its JSON form. */
    @SuppressWarnings("unchecked")
    private static <T> boolean addValue(Map<String, Object> pool, DataComponentType<T> t, Object value, RegistryAccess access,
                                        HashedPatchMap.HashGenerator hasher, RegistryOps<JsonElement> ops) {
        if (t.isTransient()) {
            return false;
        }
        try {
            T v = (T) value;
            byte[] bytes = encode(access, buf -> ((net.minecraft.network.codec.StreamCodec<RegistryFriendlyByteBuf, T>) t.streamCodec()).encode(buf, v));
            RegistryFriendlyByteBuf in = new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(bytes), access);
            T back = ((net.minecraft.network.codec.StreamCodec<RegistryFriendlyByteBuf, T>) t.streamCodec()).decode(in);
            if (in.readableBytes() != 0) {
                return false;
            }
            hasher.apply(new TypedDataComponent<>(t, back));
            String key = jsonOf(t, back, ops);
            if (key != null) {
                key = canonical(JsonParser.parseString(key)).toString();
            }
            if (key == null) {
                key = String.valueOf(back);
            }
            if (pool.containsKey(key)) {
                return false;
            }
            pool.put(key, back);
            return true;
        } catch (RuntimeException e) {
            return false;
        }
    }

    /** JSON with object keys sorted (map-valued components encode in their map's order). */
    private static JsonElement canonical(JsonElement e) {
        if (e.isJsonObject()) {
            com.google.gson.JsonObject out = new com.google.gson.JsonObject();
            new TreeMap<>(e.getAsJsonObject().asMap()).forEach((k, v) -> out.add(k, canonical(v)));
            return out;
        }
        if (e.isJsonArray()) {
            com.google.gson.JsonArray out = new com.google.gson.JsonArray();
            e.getAsJsonArray().forEach(v -> out.add(canonical(v)));
            return out;
        }
        return e;
    }

    @SuppressWarnings("unused")
    private static String hex(ByteBuf b) {
        return ByteBufUtil.hexDump(b);
    }

    @SuppressWarnings("unused")
    private static Holder<Item> holder(Item i) {
        return BuiltInRegistries.ITEM.wrapAsHolder(i);
    }
}
