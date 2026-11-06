package ghast.oracle.v1_12_2;

import ghast.oracle.Json;
import ghast.oracle.Scenario;
import net.minecraft.block.Block;
import net.minecraft.block.BlockState;
import net.minecraft.block.FenceBlock;
import net.minecraft.block.PaneBlock;
import net.minecraft.block.WallBlock;
import net.minecraft.util.Identifier;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.Direction;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

import static ghast.oracle.v1_12_2.Blocks.*;

/**
 * 1.12 face shapes: what {@code BlockState.getRenderLayer(view, pos, direction)} returns for each side
 * (Legacy Yarn names 1.12's face-shape enum {@code BlockRenderLayer}: SOLID, BOWL, CENTER_SMALL, ...).
 * Fences, panes and walls connect to a neighbour by this value plus a per-block exclusion list.
 * Schema: oracle/README.md (1.12.2 section, {@code face_shapes.jsonl.gz}).
 */
final class FaceDump {
    private FaceDump() {}

    private static final Direction[] ORDER = {Direction.DOWN, Direction.UP, Direction.NORTH, Direction.SOUTH,
            Direction.WEST, Direction.EAST};

    private static Harness.Run run;

    static List<String> lines() {
        Scenario sc = new Scenario("face_dump").spawn(40.5, 100, 40.5, 0, 0);
        sc.fill(-2, 60, -2, 2, 60, 2, AIR); // loads the chunks around the reference position
        run = new Harness.Run(sc);
        List<String> out = new ArrayList<>();

        // Per block: the connection exclusion lists of fences, walls and panes (the real static tests).
        for (int id = 0; id < 256; id++) {
            Block b = CollisionDump.registered(id);
            if (b == null) {
                continue;
            }
            Identifier name = Block.REGISTRY.getIdentifier(b);
            Json j = new Json().beginObject().field("type", "faces_block").field("id", id).field("name", name.toString());
            j.field("fence_excluded", test(FenceBlock.class, "method_14325", b));
            j.field("wall_excluded", test(WallBlock.class, "method_14353", b));
            j.field("pane_excluded", test(PaneBlock.class, "method_14348", b));
            out.add(j.endObject().toString());
        }

        // Every id/meta alone (air neighbours).
        for (int id = 0; id < 256; id++) {
            if (CollisionDump.registered(id) == null) {
                continue;
            }
            for (int meta = 0; meta < 16; meta++) {
                out.add(faces(id, meta, "alone", List.of()));
            }
        }

        // Stairs: the collision dump's stair_neighbour configurations (stair shape decides the faces).
        int[][] sides = {{0, 0, -1}, {1, 0, 0}, {0, 0, 1}, {-1, 0, 0}};
        for (int id : new int[] {OAK_STAIRS, STONE_STAIRS, PURPUR_STAIRS}) {
            for (int meta = 0; meta < 8; meta++) {
                for (int s = 0; s < 4; s++) {
                    for (int nm = 0; nm < 8; nm++) {
                        out.add(faces(id, meta, "stair_neighbour",
                                List.of(new int[] {sides[s][0], 0, sides[s][2], OAK_STAIRS, nm})));
                    }
                }
            }
        }
        return out;
    }

    private static boolean test(Class<?> owner, String method, Block b) {
        try {
            java.lang.reflect.Method m = owner.getDeclaredMethod(method, Block.class);
            m.setAccessible(true);
            return (boolean) m.invoke(null, b);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
    }

    private static String faces(int id, int meta, String config, List<int[]> neighbours) {
        int raw = id << 4 | meta;
        int rx = CollisionDump.RX, ry = CollisionDump.RY, rz = CollisionDump.RZ;
        for (int[] n : neighbours) {
            run.setRaw(rx + n[0], ry + n[1], rz + n[2], n[3] << 4 | n[4]);
        }
        run.setRaw(rx, ry, rz, raw);
        BlockPos pos = new BlockPos(rx, ry, rz);
        BlockState state = run.world.getBlockState(pos);
        BlockState decoded = Block.BLOCK_STATES.fromId(raw);

        Json j = new Json().beginObject().field("type", "faces").field("id", id).field("meta", meta);
        j.field("config", config);
        j.key("neighbours").beginArray();
        for (int[] n : neighbours) {
            j.beginArray().value(n[0]).value(n[1]).value(n[2]).value(n[3]).value(n[4]).endArray();
        }
        j.endArray();
        j.field("state", decoded == null ? null : decoded.toString());
        j.key("faces").beginObject();
        for (Direction d : ORDER) {
            j.field(d.name().toLowerCase(Locale.ROOT), state.getRenderLayer(run.world, pos, d).name());
        }
        j.endObject();

        run.setRaw(rx, ry, rz, 0);
        for (int[] n : neighbours) {
            run.setRaw(rx + n[0], ry + n[1], rz + n[2], 0);
        }
        return j.endObject().toString();
    }
}
