package ghast.oracle.modern;

import ghast.oracle.Json;
import ghast.oracle.Rng;
import ghast.oracle.Scenario;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.MultiPlayerGameMode;
import net.minecraft.client.multiplayer.prediction.BlockStatePredictionHandler;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ServerboundUseItemOnPacket;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;

import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;

/**
 * 26.x right clicks on a block through the real client ({@code MultiPlayerGameMode.useItemOn}):
 * random clicked / target states × held items × sneaking × game modes × player positions. Per case
 * the states before, the interaction result, every position the prediction changed (the level's
 * {@code BlockStatePredictionHandler} records), then the server's answer (an ack alone, a block
 * update then the ack, or the client's own state then the ack) with the player moved in between,
 * and the states and player position after it ({@code ClientLevel.syncBlockState}). Schema:
 * oracle/README.md, "place.jsonl.gz (26.x)".
 */
final class PlaceDump {
    static final int CASES = 6000;

    private PlaceDump() {}

    /** Clicked positions around the player at (0.5, 64, 0.5) (all within reach). */
    private static final BlockPos[] CLICKED = {
        new BlockPos(0, 64, 2), new BlockPos(0, 63, 1), new BlockPos(0, 63, 0), new BlockPos(1, 64, 0),
        new BlockPos(0, 66, 1), new BlockPos(-1, 65, 1), new BlockPos(1, 63, 1),
    };

    static void cases(long seed, int count, Consumer<String> sink) throws Exception {
        Harness.init();
        Scenario sc = new Scenario("place_dump").spawn(0.5, 64, 0.5, 0, 0);
        sc.fill(-4, 63, -4, 4, 63, 4, States.id("minecraft:stone"), 0);
        Harness.Run run = new Harness.Run(sc);
        // Level events (particles, sounds) need the renderer: no-op them.
        Reflect.set(ClientLevel.class, run.level, "levelEventHandler",
                new net.minecraft.client.renderer.LevelEventHandler(Harness.client, run.level) {
                    @Override
                    public void levelEvent(int type, BlockPos pos, int data) {}

                    @Override
                    public void globalLevelEvent(int type, BlockPos pos, int data) {}
                });
        BlockState stone = Blocks.STONE.defaultBlockState();
        BlockPos probe = new BlockPos(0, 80, 0);

        List<Item> plain = new ArrayList<>();
        List<Item> blockItems = new ArrayList<>();
        List<Item> others = new ArrayList<>();
        for (Item it : BuiltInRegistries.ITEM) {
            if (it instanceof BlockItem bi) {
                blockItems.add(it);
                Block b = bi.getBlock();
                if (b.getStateDefinition().getPossibleStates().size() == 1
                        && b.defaultBlockState().isCollisionShapeFullBlock(run.level, probe)) {
                    plain.add(it);
                }
            } else {
                others.add(it);
            }
        }
        List<BlockState> full = new ArrayList<>();
        int states = States.count();
        for (int id = 1; id < states; id++) {
            BlockState s = States.byId(id);
            if (s.isCollisionShapeFullBlock(run.level, probe)) {
                full.add(s);
            }
        }
        BlockStatePredictionHandler handler =
                (BlockStatePredictionHandler) Reflect.get(ClientLevel.class, run.level, "blockStatePredictionHandler");
        Rng rng = new Rng(seed ^ 0x706c6163655f6475L);
        Direction[] faces = Direction.values();
        for (int k = 0; k < count; k++) {
            BlockPos clicked = CLICKED[rng.nextInt(CLICKED.length)];
            Direction face = faces[rng.nextInt(faces.length)];
            BlockPos target = clicked.relative(face);
            double rc = rng.nextDouble();
            BlockState clickedState = rc < 0.45 ? full.get(rng.nextInt(full.size()))
                    : rc < 0.7 ? anyState(rng, states) : stone;
            double rt = rng.nextDouble();
            BlockState targetState = rt < 0.55 ? Blocks.AIR.defaultBlockState()
                    : rt < 0.65 ? Blocks.WATER.defaultBlockState().setValue(net.minecraft.world.level.block.LiquidBlock.LEVEL, rng.range(0, 15))
                    : rt < 0.7 ? Blocks.LAVA.defaultBlockState().setValue(net.minecraft.world.level.block.LiquidBlock.LEVEL, rng.range(0, 15))
                    : anyState(rng, states);
            double ri = rng.nextDouble();
            Item held = ri < 0.45 ? plain.get(rng.nextInt(plain.size()))
                    : ri < 0.75 ? blockItems.get(rng.nextInt(blockItems.size()))
                    : ri < 0.9 ? others.get(rng.nextInt(others.size())) : null;
            int countHeld = rng.chance(0.9) ? rng.range(1, 64) : 1;
            boolean sneaking = rng.chance(0.25);
            double rg = rng.nextDouble();
            GameType mode = rg < 0.8 ? GameType.SURVIVAL : rg < 0.9 ? GameType.CREATIVE : rg < 0.95 ? GameType.ADVENTURE : GameType.SPECTATOR;
            double px = rng.chance(0.7) ? 0.5 : 0.3 + rng.nextDouble() * 0.4;
            double pz = rng.chance(0.7) ? 0.5 : 0.3 + rng.nextDouble() * 0.4;
            double py = rng.chance(0.8) ? 64.0 : 64.0 + rng.nextDouble() * 1.2;
            // The hit point: on the clicked face, anywhere on it.
            double u = rng.nextDouble();
            double v = rng.nextDouble();
            Vec3 hit = facePoint(clicked, face, u, v);

            oneCase(run, handler, rng, states, k, clicked, face, hit, clickedState, targetState, held, countHeld, sneaking, mode, px, py, pz, sink);
        }
        // Every full-cube state clicked once on its top and once on a side with a plain block:
        // which clicks the clicked block consumes (useItemOn / useWithoutItem).
        Direction[] sides = {Direction.NORTH, Direction.SOUTH, Direction.WEST, Direction.EAST};
        BlockPos clicked = new BlockPos(1, 64, 2);
        int k = count;
        for (int i = 0; i < full.size(); i++) {
            for (Direction face : new Direction[] {Direction.UP, sides[i % 4]}) {
                Vec3 hit = facePoint(clicked, face, 0.5, 0.5);
                oneCase(run, handler, rng, states, k++, clicked, face, hit, full.get(i), Blocks.AIR.defaultBlockState(),
                        net.minecraft.world.item.Items.STONE, 32, false, GameType.SURVIVAL, 0.5, 64.0, 0.5, sink);
            }
        }
        // Every block item whose block has only an axis (logs, stems, pillars, hay) placed on
        // each face of stone: the placed axis.
        for (Item it : blockItems) {
            Block b = ((BlockItem) it).getBlock();
            var props = b.getStateDefinition().getProperties();
            if (props.size() != 1 || props.iterator().next() != net.minecraft.world.level.block.state.properties.BlockStateProperties.AXIS) {
                continue;
            }
            for (Direction face : faces) {
                Vec3 hit = facePoint(clicked, face, 0.5, 0.5);
                oneCase(run, handler, rng, states, k++, clicked, face, hit, stone, Blocks.AIR.defaultBlockState(),
                        it, 8, false, GameType.SURVIVAL, 0.5, 64.0, 0.5, sink);
            }
        }
        Harness.client.gameMode.setLocalMode(GameType.SURVIVAL, null);
        run.player.setShiftKeyDown(false);
        CollisionDump.sneak(run, false);
    }

    private static void oneCase(Harness.Run run, BlockStatePredictionHandler handler, Rng rng, int states, int k,
            BlockPos clicked, Direction face, Vec3 hit, BlockState clickedState, BlockState targetState, Item held,
            int countHeld, boolean sneaking, GameType mode, double px, double py, double pz, Consumer<String> sink) {
        BlockState stone = Blocks.STONE.defaultBlockState();
        Direction[] faces = Direction.values();
        BlockPos target = clicked.relative(face);
        // Set up: the floor, the clicked and the target block, the player.
        List<BlockPos> area = new ArrayList<>(List.of(CLICKED));
        area.add(clicked);
        area.add(target);
        for (BlockPos p : area) {
            CollisionDump.place(run, p.getX(), p.getY(), p.getZ(), p.getY() == 63 ? stone : Blocks.AIR.defaultBlockState());
            for (Direction d : faces) {
                BlockPos q = p.relative(d);
                CollisionDump.place(run, q.getX(), q.getY(), q.getZ(), q.getY() == 63 ? stone : Blocks.AIR.defaultBlockState());
            }
        }
        CollisionDump.place(run, clicked.getX(), clicked.getY(), clicked.getZ(), clickedState);
        CollisionDump.place(run, target.getX(), target.getY(), target.getZ(), targetState);
        Harness.client.gameMode.setLocalMode(mode, null);
        run.player.setPos(px, py, pz);
        run.player.setShiftKeyDown(sneaking);
        CollisionDump.sneak(run, sneaking);
        int slot = run.player.getInventory().getSelectedSlot();
        run.player.getInventory().setItem(slot, held == null ? ItemStack.EMPTY : new ItemStack(held, countHeld));
        run.player.getInventory().setItem(40, ItemStack.EMPTY);
        // Seen by the client as the server's: nothing predicted from before.
        Reflect.setInt(MultiPlayerGameMode.class, Harness.client.gameMode, "carriedIndex", slot);

        Json j = new Json().beginObject().field("type", "place").field("k", k);
        j.key("clicked").raw(pos(clicked)).field("face", face.get3DDataValue());
        j.key("hit").beginArray().f64(hit.x).f64(hit.y).f64(hit.z).endArray();
        j.field("clicked_state", States.id(clickedState)).field("target_state", States.id(targetState));
        j.field("held", held == null ? null : BuiltInRegistries.ITEM.getKey(held).getPath()).field("count", countHeld);
        j.field("sneaking", sneaking).field("mode", mode.getName());
        j.key("player").beginArray().f64(px).f64(py).f64(pz).endArray();

        Harness.connection.sent.clear();
        int before = handler.currentSequence();
        InteractionResult r = Harness.client.gameMode.useItemOn(run.player, InteractionHand.MAIN_HAND,
                new BlockHitResult(hit, face, clicked, false));
        int sequence = -1;
        for (Packet<?> p : Harness.connection.sent) {
            if (p instanceof ServerboundUseItemOnPacket up) {
                sequence = up.getSequence();
            }
        }
        j.field("result", r.getClass().getSimpleName()).field("sequence", sequence - before);
        ItemStack after = run.player.getInventory().getItem(slot);
        j.field("count_after", after.isEmpty() ? 0 : after.getCount());
        // Every position the prediction changed: (pos, state now, server state remembered).
        List<long[]> changed = predicted(handler);
        j.key("changed").beginArray();
        for (long[] c : changed) {
            BlockPos p = BlockPos.of(c[0]);
            j.beginArray().raw(pos(p)).value(States.id(run.level.getBlockState(p))).value(c[1]).endArray();
        }
        j.endArray();

        // The server's answer, with the player moved in between (falling into a hole).
        int mode2 = rng.nextInt(3);
        BlockState update = mode2 == 1 ? (rng.chance(0.5) ? targetState : anyState(rng, states)) : null;
        if (mode2 == 2) {
            update = run.level.getBlockState(target);
        }
        double moved = rng.chance(0.5) ? 0.0 : rng.nextDouble() * 1.5;
        run.player.setPos(px, py - moved, pz);
        j.field("answer", mode2 == 0 ? "ack" : mode2 == 1 ? "update" : "accept");
        j.field("update", update == null ? -1 : States.id(update)).fieldF64("moved", moved);
        if (update != null) {
            run.level.setServerVerifiedBlockState(target, update, 19);
        }
        if (sequence >= 0) {
            run.level.handleBlockChangedAck(sequence);
        }
        j.field("clicked_after", States.id(run.level.getBlockState(clicked)))
                .field("target_after", States.id(run.level.getBlockState(target)));
        j.key("player_after").beginArray().f64(run.player.getX()).f64(run.player.getY()).f64(run.player.getZ()).endArray();
        sink.accept(j.endObject().toString());
        // Nothing may stay predicted for the next case.
        run.level.handleBlockChangedAck(Integer.MAX_VALUE >> 1);
    }

    /** Any state but the air ones (a section holding only air reads as plain air: LevelChunk.getBlockState). */
    private static BlockState anyState(Rng rng, int states) {
        BlockState s;
        do {
            s = States.byId(1 + rng.nextInt(states - 1));
        } while (s.isAir());
        return s;
    }

    private static Vec3 facePoint(BlockPos p, Direction face, double u, double v) {
        double x = p.getX();
        double y = p.getY();
        double z = p.getZ();
        return switch (face) {
            case DOWN -> new Vec3(x + u, y, z + v);
            case UP -> new Vec3(x + u, y + 1.0, z + v);
            case NORTH -> new Vec3(x + u, y + v, z);
            case SOUTH -> new Vec3(x + u, y + v, z + 1.0);
            case WEST -> new Vec3(x, y + v, z + u);
            case EAST -> new Vec3(x + 1.0, y + v, z + u);
        };
    }

    private static String pos(BlockPos p) {
        return "[" + p.getX() + "," + p.getY() + "," + p.getZ() + "]";
    }

    /** The handler's records as {pos, remembered server state}, sorted by position. */
    @SuppressWarnings("unchecked")
    private static List<long[]> predicted(BlockStatePredictionHandler h) {
        var map = (it.unimi.dsi.fastutil.longs.Long2ObjectOpenHashMap<Object>) Reflect.get(BlockStatePredictionHandler.class, h, "serverVerifiedStates");
        List<long[]> out = new ArrayList<>();
        for (var e : map.long2ObjectEntrySet()) {
            Object rec = e.getValue();
            BlockState s = (BlockState) Reflect.get(rec.getClass(), rec, "blockState");
            out.add(new long[] {e.getLongKey(), States.id(s)});
        }
        out.sort((a, b) -> Long.compare(a[0], b[0]));
        return out;
    }
}
