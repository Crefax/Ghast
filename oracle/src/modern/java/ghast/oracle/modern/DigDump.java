package ghast.oracle.modern;

import ghast.oracle.Json;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ServerboundPlayerActionPacket;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.effect.MobEffects;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;

/**
 * 26.1 block breaking through the real client: random block states × held items × mining
 * conditions; per case the per-tick destroy progress ({@code BlockState.getDestroyProgress}), the
 * player's destroy speed, and the tick on which {@code MultiPlayerGameMode} sends
 * {@code STOP_DESTROY_BLOCK} when driven like {@code Minecraft.handleKeybinds} does
 * ({@code startDestroyBlock} then {@code continueDestroyBlock} in the start tick, one
 * {@code continueDestroyBlock} per later tick). Some cases dig a second block right after the
 * first (the 5-tick destroy delay). Schema: oracle/README.md, "dig.jsonl.gz (26.1)".
 */
final class DigDump {
    static final int CASES = 3000;
    static final int LIMIT = 2000;

    private DigDump() {}

    private static final BlockPos POS = new BlockPos(0, 65, 2);

    static void cases(long seed, int count, Consumer<String> sink) throws Exception {
        Harness.init();
        Scenario sc = new Scenario("dig_dump").spawn(0.5, 64, 0.5, 0, 0);
        sc.fill(-3, 63, -3, 3, 63, 3, States.id("minecraft:stone"), 0);
        Harness.Run run = new Harness.Run(sc);
        // Level events (block break particles and sounds) need the renderer's models: no-op them.
        Reflect.set(net.minecraft.client.multiplayer.ClientLevel.class, run.level, "levelEventHandler",
                new net.minecraft.client.renderer.LevelEventHandler(Harness.client, run.level) {
                    @Override
                    public void levelEvent(int type, BlockPos pos, int data) {}

                    @Override
                    public void globalLevelEvent(int type, BlockPos pos, int data) {}
                });
        Field fi = net.minecraft.world.entity.Entity.class.getDeclaredField("fluidInteraction");
        fi.setAccessible(true);
        Object fluid = fi.get(run.player);
        Method update = fluid.getClass().getDeclaredMethod("update", net.minecraft.world.entity.Entity.class, boolean.class);
        update.setAccessible(true);

        List<Item> tools = new ArrayList<>();
        List<Item> others = new ArrayList<>();
        for (Item it : BuiltInRegistries.ITEM) {
            (it.components().has(net.minecraft.core.component.DataComponents.TOOL) ? tools : others).add(it);
        }
        int states = States.count();
        Rng rng = new Rng(seed ^ 0x6469675f64756d70L);
        for (int k = 0; k < count; k++) {
            // Block: any state (shulker boxes / moving pistons are fine here: breaking reads only state facts).
            BlockState state = States.byId(1 + rng.nextInt(states - 1));
            // Mostly blocks that break within the tick limit by hand or tool (destroy time <= 5).
            while (state.isAir() || (rng.chance(0.8) && state.getDestroySpeed(run.level, POS) > 5.0F)) {
                state = States.byId(1 + rng.nextInt(states - 1));
            }
            double ri = rng.nextDouble();
            Item held = ri < 0.15 ? null : ri < 0.8 ? tools.get(rng.nextInt(tools.size())) : others.get(rng.nextInt(others.size()));
            double eff = rng.chance(0.5) ? 0.0 : rng.chance(0.5) ? rng.pick(new Double[] {2.0, 5.0, 10.0, 17.0, 26.0}) : rng.nextDouble() * 40;
            double breakSpeed = rng.chance(0.6) ? 1.0 : rng.chance(0.05) ? 0.0 : 0.2 + rng.nextDouble() * 4;
            double submerged = rng.chance(0.6) ? 0.2 : rng.chance(0.5) ? 1.0 : rng.nextDouble() * 2;
            int haste = rng.chance(0.7) ? -1 : rng.range(0, 5);
            int conduit = rng.chance(0.85) ? -1 : rng.range(0, 3);
            int fatigue = rng.chance(0.88) ? -1 : rng.chance(0.7) ? rng.range(0, 1) : rng.range(2, 6);
            boolean water = rng.chance(0.15);
            boolean onGround = rng.chance(0.8);
            boolean creative = rng.chance(0.05);
            boolean second = rng.chance(0.15);

            clearEffects(run);
            if (haste >= 0) {
                addEffect(run, MobEffects.HASTE, haste);
            }
            if (conduit >= 0) {
                addEffect(run, MobEffects.CONDUIT_POWER, conduit);
            }
            if (fatigue >= 0) {
                addEffect(run, MobEffects.MINING_FATIGUE, fatigue);
            }
            run.player.getAttribute(Attributes.MINING_EFFICIENCY).setBaseValue(eff);
            run.player.getAttribute(Attributes.BLOCK_BREAK_SPEED).setBaseValue(breakSpeed);
            run.player.getAttribute(Attributes.SUBMERGED_MINING_SPEED).setBaseValue(submerged);
            BlockState around = water ? Blocks.WATER.defaultBlockState() : Blocks.AIR.defaultBlockState();
            CollisionDump.place(run, 0, 64, 0, around);
            CollisionDump.place(run, 0, 65, 0, around);
            CollisionDump.place(run, 0, 66, 0, around);
            run.player.setPos(0.5, 64.0, 0.5);
            update.invoke(fluid, run.player, false);
            run.player.setOnGround(onGround);
            run.player.getInventory().setItem(run.player.getInventory().getSelectedSlot(), held == null ? ItemStack.EMPTY : new ItemStack(held));
            Harness.client.gameMode.setLocalMode(creative ? GameType.CREATIVE : GameType.SURVIVAL, null);

            Json j = new Json().beginObject().field("type", "dig").field("k", k);
            j.field("state", States.id(state)).field("name", States.name(state));
            j.field("held", held == null ? null : BuiltInRegistries.ITEM.getKey(held).getPath());
            j.fieldF64("mining_efficiency", eff).fieldF64("block_break_speed", breakSpeed).fieldF64("submerged_mining_speed", submerged);
            j.field("haste", haste).field("conduit_power", conduit).field("mining_fatigue", fatigue);
            j.field("eye_in_water", run.player.isEyeInFluid(net.minecraft.tags.FluidTags.WATER)).field("on_ground", run.player.onGround());
            j.field("creative", creative);
            CollisionDump.place(run, POS.getX(), POS.getY(), POS.getZ(), state);
            j.fieldF32("destroy_speed", run.player.getDestroySpeed(state));
            j.field("correct_tool", run.player.hasCorrectToolForDrops(state));
            j.fieldF32("progress", state.getDestroyProgress(run.player, run.level, POS));
            j.key("first").raw(dig(run, state, second ? 2 : 1));
            sink.accept(j.endObject().toString());
            // Reset the game mode's dig state for the next case.
            Harness.client.gameMode.stopDestroyBlock();
            Reflect.setInt(net.minecraft.client.multiplayer.MultiPlayerGameMode.class, Harness.client.gameMode, "destroyDelay", 0);
            CollisionDump.place(run, POS.getX(), POS.getY(), POS.getZ(), Blocks.AIR.defaultBlockState());
        }
        Harness.client.gameMode.setLocalMode(GameType.SURVIVAL, null);
        clearEffects(run);
    }

    /** Digs {@code times} blocks in a row (the same state, replaced after each break); per dig the outcome. */
    private static String dig(Harness.Run run, BlockState state, int times) {
        Json out = new Json().beginArray();
        int tick = 0;
        for (int t = 0; t < times; t++) {
            CollisionDump.place(run, POS.getX(), POS.getY(), POS.getZ(), state);
            Harness.connection.sent.clear();
            int startTick = tick;
            Harness.client.gameMode.startDestroyBlock(POS, Direction.UP);
            String result = null;
            int calls = 0;
            if (action(ServerboundPlayerActionPacket.Action.START_DESTROY_BLOCK) && !Harness.client.gameMode.isDestroying()) {
                result = "instant";
            } else {
                // The start tick continues too (Minecraft.continueAttack), then one call per tick.
                while (calls < LIMIT) {
                    calls++;
                    Harness.client.gameMode.continueDestroyBlock(POS, Direction.UP);
                    if (action(ServerboundPlayerActionPacket.Action.STOP_DESTROY_BLOCK)) {
                        result = "ticks";
                        break;
                    }
                    if (!Harness.client.gameMode.isDestroying()) {
                        result = "stopped";
                        break;
                    }
                }
            }
            tick += Math.max(calls, 1);
            Json r = new Json().beginObject().field("result", result == null ? "none" : result).field("calls", calls)
                    .field("start_tick", startTick);
            out.raw(r.endObject().toString());
            if (!"ticks".equals(result) && !"instant".equals(result)) {
                break; // a second dig only follows a finished one
            }
        }
        return out.endArray().toString();
    }

    private static boolean action(ServerboundPlayerActionPacket.Action a) {
        for (Packet<?> p : Harness.connection.sent) {
            if (p instanceof ServerboundPlayerActionPacket ap && ap.getAction() == a) {
                return true;
            }
        }
        return false;
    }

    private static void addEffect(Harness.Run run, Holder<MobEffect> effect, int amplifier) {
        // As ClientPacketListener.handleUpdateMobEffect does (addEffect is server-side only).
        run.player.forceAddEffect(new MobEffectInstance(effect, 1_000_000, amplifier), null);
    }

    /** The client cannot remove effects through removeAllEffects (server-side only). */
    private static void clearEffects(Harness.Run run) {
        for (Holder<MobEffect> e : new ArrayList<>(run.player.getActiveEffectsMap().keySet())) {
            run.player.removeEffectNoUpdate(e);
        }
    }
}
