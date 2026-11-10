package ghast.oracle.v1_8_9;

import ghast.oracle.Json;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import net.minecraft.block.Block;
import net.minecraft.block.BlockState;
import net.minecraft.client.MinecraftClient;
import net.minecraft.client.network.ClientPlayerInteractionManager;
import net.minecraft.enchantment.Enchantment;
import net.minecraft.entity.effect.StatusEffect;
import net.minecraft.entity.effect.StatusEffectInstance;
import net.minecraft.entity.player.ClientPlayerEntity;
import net.minecraft.item.Item;
import net.minecraft.item.ItemStack;
import net.minecraft.network.Packet;
import net.minecraft.network.packet.c2s.play.PlayerActionC2SPacket;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.Direction;
import net.minecraft.world.level.LevelInfo;

import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.function.Consumer;

/**
 * 1.8.9 block breaking through the real client.
 *
 * <p>{@code dig_facts.jsonl.gz}: per block id its strength (hardness) and whether its material needs
 * no tool; per item the blocks on which {@code ItemStack.getMiningSpeedMultiplier} differs from 1 and
 * those {@code isEffectiveOn} accepts (stack of the item, damage 0).
 *
 * <p>{@code dig.jsonl.gz}: random blocks x held items x conditions (efficiency, haste, mining
 * fatigue, eyes in water, aqua affinity, on ground, creative): {@code PlayerEntity.getMiningSpeed},
 * {@code isUsingEffectiveTool}, {@code Block.calcBlockBreakingData}, and the call on which
 * {@code ClientPlayerInteractionManager} sends {@code STOP_DESTROY_BLOCK} when driven like
 * {@code MinecraftClient} does ({@code attackBlock}, then {@code updateBlockBreakingProgress} in the
 * same tick and once per later tick); some cases dig a second block right after the first (the
 * 5-call cooldown). Schema: oracle/README.md, "Dig vectors (1.8.9, 1.12.2, 1.16.5)".
 */
final class DigDump {
    static final int CASES = 3000;
    static final int LIMIT = 2000;
    private static final BlockPos POS = new BlockPos(0, 65, 2);

    private DigDump() {}

    static void facts(Consumer<String> sink) {
        Harness.init();
        List<Block> blocks = blocks();
        for (Block b : blocks) {
            Json j = new Json().beginObject().field("type", "block").field("id", Block.getIdByBlock(b));
            j.field("name", Block.REGISTRY.getIdentifier(b).toString());
            j.fieldF32("strength", b.getStrength(null, POS));
            j.field("no_tool_needed", b.getMaterial().doesBlockMovement());
            sink.accept(j.endObject().toString());
        }
        for (Object o : Item.REGISTRY) {
            Item item = (Item) o;
            ItemStack stack = new ItemStack(item);
            Map<Integer, List<Integer>> speeds = new TreeMap<>();
            List<Integer> effective = new ArrayList<>();
            for (Block b : blocks) {
                float s = stack.getMiningSpeedMultiplier(b);
                if (s != 1.0F) {
                    speeds.computeIfAbsent(Float.floatToRawIntBits(s), k -> new ArrayList<>()).add(Block.getIdByBlock(b));
                }
                if (stack.isEffectiveOn(b)) {
                    effective.add(Block.getIdByBlock(b));
                }
            }
            Json j = new Json().beginObject().field("type", "item").field("id", Item.getRawId(item));
            j.field("name", Item.REGISTRY.getIdentifier(item).toString());
            j.key("speed_blocks").beginArray();
            speeds.forEach((bits, ids) -> {
                j.beginObject().fieldF32("speed", Float.intBitsToFloat(bits)).key("blocks").beginArray();
                ids.forEach(i -> j.value(i));
                j.endArray().endObject();
            });
            j.endArray().key("effective_blocks").beginArray();
            effective.forEach(i -> j.value(i));
            j.endArray();
            sink.accept(j.endObject().toString());
        }
    }

    private static List<Block> blocks() {
        List<Block> out = new ArrayList<>();
        for (int id = 0; id < 256; id++) {
            Block b = Block.getById(id);
            if (b != null && (id == 0 || Block.getIdByBlock(b) == id)) {
                out.add(b);
            }
        }
        return out;
    }

    static void cases(long seed, int count, Consumer<String> sink) throws Exception {
        Harness.init();
        Scenario sc = new Scenario("dig_dump").spawn(0.5, 64, 0.5, 0, 0);
        sc.fill(-3, 63, -3, 3, 63, 3, 1, 0);
        Harness.Run run = new Harness.Run(sc);
        MinecraftClient client = MinecraftClient.getInstance();
        ClientPlayerEntity player = run.player;
        Field breaking = ClientPlayerInteractionManager.class.getDeclaredField("breakingBlock");
        breaking.setAccessible(true);

        List<Block> blocks = blocks();
        List<Item> tools = new ArrayList<>();
        List<Item> others = new ArrayList<>();
        for (Object o : Item.REGISTRY) {
            Item it = (Item) o;
            (it instanceof net.minecraft.item.ToolItem || it instanceof net.minecraft.item.SwordItem
                    || it instanceof net.minecraft.item.ShearsItem ? tools : others).add(it);
        }
        Rng rng = new Rng(seed ^ 0x6469675f64756d70L);
        for (int k = 0; k < count; k++) {
            Block block = blocks.get(1 + rng.nextInt(blocks.size() - 1));
            while (rng.chance(0.8) && (block.getStrength(null, POS) > 5.0F || block.getStrength(null, POS) < 0.0F)) {
                block = blocks.get(1 + rng.nextInt(blocks.size() - 1));
            }
            int meta = rng.nextInt(16);
            BlockState state;
            try {
                state = block.stateFromData(meta);
            } catch (IllegalArgumentException e) {
                state = block.getDefaultState();
            }
            int raw = Block.getIdByBlock(block) << 4 | block.getData(state);
            double ri = rng.nextDouble();
            Item held = ri < 0.15 ? null : ri < 0.8 ? tools.get(rng.nextInt(tools.size())) : others.get(rng.nextInt(others.size()));
            int efficiency = rng.chance(0.55) ? 0 : rng.range(1, 5);
            int haste = rng.chance(0.7) ? -1 : rng.range(0, 3);
            int fatigue = rng.chance(0.88) ? -1 : rng.chance(0.7) ? rng.range(0, 1) : rng.range(2, 5);
            boolean water = rng.chance(0.15);
            boolean aqua = rng.chance(0.3);
            boolean onGround = rng.chance(0.8);
            boolean creative = rng.chance(0.05);
            boolean second = rng.chance(0.15);

            for (int id : new int[] {StatusEffect.HASTE.id, StatusEffect.MINING_FATIGUE.id}) {
                player.removeEffect(id);
            }
            if (haste >= 0) {
                player.addStatusEffect(new StatusEffectInstance(StatusEffect.HASTE.id, 1_000_000, haste));
            }
            if (fatigue >= 0) {
                player.addStatusEffect(new StatusEffectInstance(StatusEffect.MINING_FATIGUE.id, 1_000_000, fatigue));
            }
            int around = water ? 9 << 4 : 0;
            run.setRaw(0, 64, 0, around);
            run.setRaw(0, 65, 0, around);
            run.setRaw(0, 66, 0, around);
            player.updatePosition(0.5, 64.0, 0.5);
            player.onGround = onGround;
            ItemStack stack = held == null ? null : new ItemStack(held);
            if (stack != null && efficiency > 0) {
                stack.addEnchantment(Enchantment.EFFICIENCY, efficiency);
            }
            player.inventory.main[player.inventory.selectedSlot] = stack;
            ItemStack helmet = null;
            if (aqua) {
                helmet = new ItemStack(net.minecraft.item.Items.DIAMOND_HELMET);
                helmet.addEnchantment(Enchantment.AQUA_AFFINITY, 1);
            }
            player.inventory.armor[3] = helmet;
            client.interactionManager.setGameMode(creative ? LevelInfo.GameMode.CREATIVE : LevelInfo.GameMode.SURVIVAL);

            Json j = new Json().beginObject().field("type", "dig").field("k", k);
            j.field("raw", raw).field("name", Block.REGISTRY.getIdentifier(block).toString());
            j.field("held", held == null ? null : Item.REGISTRY.getIdentifier(held).toString());
            j.field("held_id", held == null ? -1 : Item.getRawId(held));
            j.field("efficiency", efficiency).field("haste", haste).field("mining_fatigue", fatigue);
            j.field("eye_in_water", player.isSubmergedIn(net.minecraft.block.material.Material.WATER));
            j.field("aqua_affinity", aqua).field("on_ground", player.onGround).field("creative", creative);
            run.setRaw(POS.getX(), POS.getY(), POS.getZ(), raw);
            j.fieldF32("mining_speed", player.getMiningSpeed(block));
            j.field("effective_tool", player.isUsingEffectiveTool(block));
            j.fieldF32("progress", block.calcBlockBreakingData(player, run.world, POS));
            j.key("first").raw(dig(run, client, breaking, raw, second ? 2 : 1));
            sink.accept(j.endObject().toString());
            client.interactionManager.cancelBlockBreaking();
            Reflect.setInt(ClientPlayerInteractionManager.class, client.interactionManager, "blockBreakingCooldown", 0);
            run.setRaw(POS.getX(), POS.getY(), POS.getZ(), 0);
        }
        client.interactionManager.setGameMode(LevelInfo.GameMode.SURVIVAL);
        player.removeEffect(StatusEffect.HASTE.id);
        player.removeEffect(StatusEffect.MINING_FATIGUE.id);
        player.inventory.armor[3] = null;
        player.inventory.main[player.inventory.selectedSlot] = null;
    }

    private static String dig(Harness.Run run, MinecraftClient client, Field breaking, int raw, int times) throws Exception {
        Json out = new Json().beginArray();
        int tick = 0;
        for (int t = 0; t < times; t++) {
            run.setRaw(POS.getX(), POS.getY(), POS.getZ(), raw);
            run.net.sent.clear();
            int startTick = tick;
            client.interactionManager.attackBlock(POS, Direction.UP);
            String result = null;
            int calls = 0;
            if (action(run, PlayerActionC2SPacket.Action.START_DESTROY_BLOCK) && !breaking.getBoolean(client.interactionManager)) {
                result = "instant";
            } else {
                while (calls < LIMIT) {
                    calls++;
                    client.interactionManager.updateBlockBreakingProgress(POS, Direction.UP);
                    if (action(run, PlayerActionC2SPacket.Action.STOP_DESTROY_BLOCK)) {
                        result = "ticks";
                        break;
                    }
                    if (!breaking.getBoolean(client.interactionManager)) {
                        result = "stopped";
                        break;
                    }
                }
            }
            tick += Math.max(calls, 1);
            out.raw(new Json().beginObject().field("result", result == null ? "none" : result).field("calls", calls)
                    .field("start_tick", startTick).endObject().toString());
            if (!"ticks".equals(result) && !"instant".equals(result)) {
                break;
            }
        }
        return out.endArray().toString();
    }

    private static boolean action(Harness.Run run, PlayerActionC2SPacket.Action a) {
        for (Packet<?> p : run.net.sent) {
            if (p instanceof PlayerActionC2SPacket ap && ap.getAction() == a) {
                return true;
            }
        }
        return false;
    }
}
