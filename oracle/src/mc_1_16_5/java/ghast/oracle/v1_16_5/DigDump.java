package ghast.oracle.v1_16_5;

import ghast.oracle.Json;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import net.minecraft.block.Block;
import net.minecraft.block.BlockState;
import net.minecraft.client.network.ClientPlayerInteractionManager;
import net.minecraft.enchantment.Enchantments;
import net.minecraft.entity.EquipmentSlot;
import net.minecraft.entity.effect.StatusEffect;
import net.minecraft.entity.effect.StatusEffectInstance;
import net.minecraft.entity.effect.StatusEffects;
import net.minecraft.item.Item;
import net.minecraft.item.ItemStack;
import net.minecraft.item.Items;
import net.minecraft.network.Packet;
import net.minecraft.network.packet.c2s.play.PlayerActionC2SPacket;
import net.minecraft.tag.FluidTags;
import net.minecraft.util.Hand;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.Direction;
import net.minecraft.util.registry.Registry;
import net.minecraft.world.GameMode;
import net.minecraft.world.chunk.ChunkSection;
import net.minecraft.world.chunk.WorldChunk;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.function.Consumer;

/**
 * 1.16.5 block breaking through the real client. {@code dig_facts.jsonl.gz}: per block its hardness
 * and {@code isToolRequired} (with the states that differ from the default state), per item the
 * blocks on which {@code ItemStack.getMiningSpeedMultiplier} differs from 1 and those
 * {@code isSuitableFor} accepts (checked on every state). {@code dig.jsonl.gz}: as 1.8.9 plus conduit
 * power. Schema: oracle/README.md, "Dig vectors (1.8.9, 1.12.2, 1.16.5)".
 */
final class DigDump {
    static final int CASES = 3000;
    static final int LIMIT = 2000;
    private static final BlockPos POS = new BlockPos(0, 65, 2);

    private DigDump() {}

    static void facts(Consumer<String> sink) {
        Harness.Run run = newRun();
        for (Block b : Registry.BLOCK) {
            BlockState d = b.getDefaultState();
            Json j = new Json().beginObject().field("type", "block").field("id", Registry.BLOCK.getRawId(b));
            j.field("name", Registry.BLOCK.getId(b).toString());
            j.fieldF32("hardness", d.getHardness(run.world, POS));
            j.field("requires_tool", d.isToolRequired());
            j.key("differing_states").beginArray();
            for (BlockState s : b.getStateManager().getStates()) {
                if (s.getHardness(run.world, POS) != d.getHardness(run.world, POS) || s.isToolRequired() != d.isToolRequired()) {
                    j.value(States.id(s));
                }
            }
            j.endArray();
            sink.accept(j.endObject().toString());
        }
        for (Item item : Registry.ITEM) {
            ItemStack stack = new ItemStack(item);
            Map<Integer, List<Integer>> speeds = new TreeMap<>();
            List<Integer> suitable = new ArrayList<>();
            List<Integer> odd = new ArrayList<>();
            for (Block b : Registry.BLOCK) {
                BlockState d = b.getDefaultState();
                float sp = stack.getMiningSpeedMultiplier(d);
                boolean ok = stack.isSuitableFor(d);
                for (BlockState s : b.getStateManager().getStates()) {
                    if (stack.getMiningSpeedMultiplier(s) != sp || stack.isSuitableFor(s) != ok) {
                        odd.add(States.id(s));
                    }
                }
                if (sp != 1.0F) {
                    speeds.computeIfAbsent(Float.floatToRawIntBits(sp), k -> new ArrayList<>()).add(Registry.BLOCK.getRawId(b));
                }
                if (ok) {
                    suitable.add(Registry.BLOCK.getRawId(b));
                }
            }
            Json j = new Json().beginObject().field("type", "item").field("id", Registry.ITEM.getRawId(item));
            j.field("name", Registry.ITEM.getId(item).toString());
            j.key("speed_blocks").beginArray();
            speeds.forEach((bits, ids) -> {
                j.beginObject().fieldF32("speed", Float.intBitsToFloat(bits)).key("blocks").beginArray();
                ids.forEach(i -> j.value(i));
                j.endArray().endObject();
            });
            j.endArray().key("effective_blocks").beginArray();
            suitable.forEach(i -> j.value(i));
            j.endArray().key("differing_states").beginArray();
            odd.forEach(i -> j.value(i));
            j.endArray();
            sink.accept(j.endObject().toString());
        }
    }

    private static Harness.Run newRun() {
        Scenario sc = new Scenario("dig_dump").spawn(0.5, 64, 0.5, 0, 0);
        sc.fill(-3, 63, -3, 3, 63, 3, States.id("minecraft:stone"), 0);
        return new Harness.Run(sc);
    }

    private static void place(Harness.Run run, int x, int y, int z, BlockState state) {
        WorldChunk chunk = run.world.getChunk(x >> 4, z >> 4);
        ChunkSection[] sections = chunk.getSectionArray();
        if (sections[y >> 4] == null) {
            sections[y >> 4] = new ChunkSection((y >> 4) << 4);
        }
        sections[y >> 4].setBlockState(x & 15, y & 15, z & 15, state, false);
    }

    static void cases(long seed, int count, Consumer<String> sink) throws Exception {
        Harness.init();
        Harness.Run run = newRun();
        Field breaking = ClientPlayerInteractionManager.class.getDeclaredField("breakingBlock");
        breaking.setAccessible(true);
        Method submerged = net.minecraft.entity.Entity.class.getDeclaredMethod("updateSubmergedInWaterState");
        submerged.setAccessible(true);
        List<Item> tools = new ArrayList<>();
        List<Item> others = new ArrayList<>();
        for (Item it : Registry.ITEM) {
            (it instanceof net.minecraft.item.ToolItem || it instanceof net.minecraft.item.ShearsItem ? tools : others).add(it);
        }
        int states = States.count();
        Rng rng = new Rng(seed ^ 0x6469675f64756d70L);
        for (int k = 0; k < count; k++) {
            BlockState state = States.byId(1 + rng.nextInt(states - 1));
            while (state.isAir() || (rng.chance(0.8) && (state.getHardness(run.world, POS) > 5.0F || state.getHardness(run.world, POS) < 0.0F))) {
                state = States.byId(1 + rng.nextInt(states - 1));
            }
            double ri = rng.nextDouble();
            Item held = ri < 0.15 ? null : ri < 0.8 ? tools.get(rng.nextInt(tools.size())) : others.get(rng.nextInt(others.size()));
            int efficiency = rng.chance(0.55) ? 0 : rng.range(1, 5);
            int haste = rng.chance(0.7) ? -1 : rng.range(0, 3);
            int conduit = rng.chance(0.85) ? -1 : rng.range(0, 3);
            int fatigue = rng.chance(0.88) ? -1 : rng.chance(0.7) ? rng.range(0, 1) : rng.range(2, 5);
            boolean water = rng.chance(0.15);
            boolean aqua = rng.chance(0.3);
            boolean onGround = rng.chance(0.8);
            boolean creative = rng.chance(0.05);
            boolean second = rng.chance(0.15);

            run.player.getActiveStatusEffects().clear();
            addEffect(run, StatusEffects.HASTE, haste);
            addEffect(run, StatusEffects.CONDUIT_POWER, conduit);
            addEffect(run, StatusEffects.MINING_FATIGUE, fatigue);
            BlockState around = water ? net.minecraft.block.Blocks.WATER.getDefaultState() : net.minecraft.block.Blocks.AIR.getDefaultState();
            place(run, 0, 64, 0, around);
            place(run, 0, 65, 0, around);
            place(run, 0, 66, 0, around);
            run.player.setPosition(0.5, 64.0, 0.5);
            submerged.invoke(run.player);
            run.player.setOnGround(onGround);
            ItemStack stack = held == null ? ItemStack.EMPTY : new ItemStack(held);
            if (held != null && efficiency > 0) {
                stack.addEnchantment(Enchantments.EFFICIENCY, efficiency);
            }
            run.player.setStackInHand(Hand.MAIN_HAND, stack);
            ItemStack helmet = ItemStack.EMPTY;
            if (aqua) {
                helmet = new ItemStack(Items.DIAMOND_HELMET);
                helmet.addEnchantment(Enchantments.AQUA_AFFINITY, 1);
            }
            run.player.equipStack(EquipmentSlot.HEAD, helmet);
            Harness.client.interactionManager.setGameMode(creative ? GameMode.CREATIVE : GameMode.SURVIVAL);

            Json j = new Json().beginObject().field("type", "dig").field("k", k);
            j.field("state", States.id(state)).field("name", States.name(state));
            j.field("held", held == null ? null : Registry.ITEM.getId(held).toString());
            j.field("held_id", held == null ? -1 : Registry.ITEM.getRawId(held));
            j.field("efficiency", efficiency).field("haste", haste).field("conduit_power", conduit).field("mining_fatigue", fatigue);
            j.field("eye_in_water", run.player.isSubmergedIn(FluidTags.WATER));
            j.field("aqua_affinity", aqua).field("on_ground", run.player.isOnGround()).field("creative", creative);
            place(run, POS.getX(), POS.getY(), POS.getZ(), state);
            j.fieldF32("mining_speed", run.player.getBlockBreakingSpeed(state));
            j.field("effective_tool", run.player.canHarvest(state));
            j.fieldF32("progress", state.calcBlockBreakingDelta(run.player, run.world, POS));
            j.key("first").raw(dig(run, breaking, state, second ? 2 : 1));
            sink.accept(j.endObject().toString());
            Harness.client.interactionManager.cancelBlockBreaking();
            Reflect.setInt(ClientPlayerInteractionManager.class, Harness.client.interactionManager, "blockBreakingCooldown", 0);
            place(run, POS.getX(), POS.getY(), POS.getZ(), net.minecraft.block.Blocks.AIR.getDefaultState());
        }
        Harness.client.interactionManager.setGameMode(GameMode.SURVIVAL);
        run.player.getActiveStatusEffects().clear();
        run.player.setStackInHand(Hand.MAIN_HAND, ItemStack.EMPTY);
        run.player.equipStack(EquipmentSlot.HEAD, ItemStack.EMPTY);
    }

    private static void addEffect(Harness.Run run, StatusEffect effect, int amplifier) {
        if (amplifier >= 0) {
            run.player.addStatusEffect(new StatusEffectInstance(effect, 1_000_000, amplifier));
        }
    }

    private static String dig(Harness.Run run, Field breaking, BlockState state, int times) throws Exception {
        Json out = new Json().beginArray();
        int tick = 0;
        for (int t = 0; t < times; t++) {
            place(run, POS.getX(), POS.getY(), POS.getZ(), state);
            Harness.connection.sent.clear();
            int startTick = tick;
            Harness.client.interactionManager.attackBlock(POS, Direction.UP);
            String result = null;
            int calls = 0;
            if (action(PlayerActionC2SPacket.Action.START_DESTROY_BLOCK) && !breaking.getBoolean(Harness.client.interactionManager)) {
                result = "instant";
            } else {
                while (calls < LIMIT) {
                    calls++;
                    Harness.client.interactionManager.updateBlockBreakingProgress(POS, Direction.UP);
                    if (action(PlayerActionC2SPacket.Action.STOP_DESTROY_BLOCK)) {
                        result = "ticks";
                        break;
                    }
                    if (!breaking.getBoolean(Harness.client.interactionManager)) {
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

    private static boolean action(PlayerActionC2SPacket.Action a) {
        for (Packet<?> p : Harness.connection.sent) {
            if (p instanceof PlayerActionC2SPacket && ((PlayerActionC2SPacket) p).getAction() == a) {
                return true;
            }
        }
        return false;
    }
}
