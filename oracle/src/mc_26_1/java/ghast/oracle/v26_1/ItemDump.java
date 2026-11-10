package ghast.oracle.v26_1;

import ghast.oracle.Json;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderSet;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.tags.TagKey;
import net.minecraft.world.food.FoodProperties;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.component.Tool;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.TreeSet;
import java.util.function.Consumer;

/**
 * 26.1 item and dig facts from the live registries (tags bound by the harness's real tag load):
 * per item its default max stack size, max damage, food, block form and tool component; the block
 * tags the tool rules name, resolved; per block its destroy time, whether it needs the correct tool
 * for drops, and its item form; and, as a check of the rule logic, per item the blocks on which
 * {@code ItemStack.getDestroySpeed} differs from 1 and those {@code isCorrectToolForDrops} accepts.
 * Schema: oracle/README.md, "items.jsonl.gz (26.1)".
 */
final class ItemDump {
    private ItemDump() {}

    static void lines(Consumer<String> sink) {
        Harness.Run run = new Harness.Run(new ghast.oracle.Scenario("item_dump").spawn(0.5, 100, 0.5, 0, 0));
        BlockPos pos = new BlockPos(0, 64, 0);
        TreeSet<String> tags = new TreeSet<>();
        for (Item item : BuiltInRegistries.ITEM) {
            Json j = new Json().beginObject().field("type", "item").field("id", BuiltInRegistries.ITEM.getId(item));
            j.field("name", BuiltInRegistries.ITEM.getKey(item).getPath());
            j.field("max_stack_size", item.getDefaultMaxStackSize());
            Integer maxDamage = item.components().get(DataComponents.MAX_DAMAGE);
            if (maxDamage != null) {
                j.field("max_damage", maxDamage);
            } else {
                j.key("max_damage").raw("null");
            }
            FoodProperties food = item.components().get(DataComponents.FOOD);
            if (food != null) {
                j.key("food").beginObject().field("nutrition", food.nutrition()).fieldF32("saturation", food.saturation())
                        .field("can_always_eat", food.canAlwaysEat()).endObject();
            } else {
                j.key("food").raw("null");
            }
            if (item instanceof BlockItem bi) {
                j.field("block", BuiltInRegistries.BLOCK.getKey(bi.getBlock()).getPath());
            } else {
                j.key("block").raw("null");
            }
            Tool tool = item.components().get(DataComponents.TOOL);
            if (tool != null) {
                j.key("tool").beginObject();
                j.key("rules").beginArray();
                for (Tool.Rule r : tool.rules()) {
                    j.beginObject();
                    HolderSet<Block> set = r.blocks();
                    if (set.unwrapKey().isPresent()) {
                        String t = set.unwrapKey().get().location().toString();
                        tags.add(t);
                        j.field("tag", t);
                    } else {
                        j.key("blocks").beginArray();
                        set.stream().forEach(h -> j.value(h.unwrapKey().orElseThrow().identifier().getPath()));
                        j.endArray();
                    }
                    if (r.speed().isPresent()) {
                        j.fieldF32("speed", r.speed().get());
                    }
                    if (r.correctForDrops().isPresent()) {
                        j.field("correct_for_drops", r.correctForDrops().get());
                    }
                    j.endObject();
                }
                j.endArray();
                j.fieldF32("default_mining_speed", tool.defaultMiningSpeed());
                j.field("damage_per_block", tool.damagePerBlock());
                j.field("can_destroy_blocks_in_creative", tool.canDestroyBlocksInCreative());
                j.endObject();
            } else {
                j.key("tool").raw("null");
            }
            // The rule logic, checked through the real item stack on every block's default state.
            ItemStack stack = new ItemStack(item);
            Map<Integer, List<Integer>> speeds = new TreeMap<>();
            List<Integer> correct = new ArrayList<>();
            for (Block b : BuiltInRegistries.BLOCK) {
                BlockState s = b.defaultBlockState();
                float sp = stack.getDestroySpeed(s);
                if (sp != 1.0F) {
                    speeds.computeIfAbsent(Float.floatToRawIntBits(sp), k -> new ArrayList<>()).add(BuiltInRegistries.BLOCK.getId(b));
                }
                if (stack.isCorrectToolForDrops(s)) {
                    correct.add(BuiltInRegistries.BLOCK.getId(b));
                }
            }
            j.key("speed_blocks").beginArray();
            speeds.forEach((bits, ids) -> {
                j.beginObject().fieldF32("speed", Float.intBitsToFloat(bits)).key("blocks").beginArray();
                ids.forEach(j::value);
                j.endArray().endObject();
            });
            j.endArray();
            j.key("correct_blocks").beginArray();
            correct.forEach(j::value);
            j.endArray();
            sink.accept(j.endObject().toString());
        }
        for (String t : tags) {
            TagKey<Block> key = TagKey.create(net.minecraft.core.registries.Registries.BLOCK, net.minecraft.resources.Identifier.parse(t));
            TreeSet<Integer> ids = new TreeSet<>();
            for (Holder<Block> h : BuiltInRegistries.BLOCK.getTagOrEmpty(key)) {
                ids.add(BuiltInRegistries.BLOCK.getId(h.value()));
            }
            Json j = new Json().beginObject().field("type", "tag").field("name", t).key("blocks").beginArray();
            ids.forEach(j::value);
            sink.accept(j.endArray().endObject().toString());
        }
        for (Block b : BuiltInRegistries.BLOCK) {
            Json j = new Json().beginObject().field("type", "block").field("id", BuiltInRegistries.BLOCK.getId(b));
            j.field("name", BuiltInRegistries.BLOCK.getKey(b).getPath());
            BlockState d = b.defaultBlockState();
            j.fieldF32("destroy_time", d.getDestroySpeed(run.level, pos));
            j.field("requires_correct_tool", d.requiresCorrectToolForDrops());
            // States that differ from the default state in either fact (expected: none).
            Map<String, Object> odd = new LinkedHashMap<>();
            for (BlockState s : b.getStateDefinition().getPossibleStates()) {
                if (s.getDestroySpeed(run.level, pos) != d.getDestroySpeed(run.level, pos)
                        || s.requiresCorrectToolForDrops() != d.requiresCorrectToolForDrops()) {
                    odd.put(Integer.toString(States.id(s)), Boolean.TRUE);
                }
            }
            j.key("differing_states").beginArray();
            odd.keySet().forEach(j::value);
            j.endArray();
            Item asItem = b.asItem();
            j.field("item", BuiltInRegistries.ITEM.getKey(asItem).getPath());
            sink.accept(j.endObject().toString());
        }
    }
}
