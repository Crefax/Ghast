package ghast.oracle.modern;

import net.minecraft.client.Camera;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.social.PlayerSocialManager;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.LevelLoadTracker;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.renderer.extract.LevelExtractor;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.resources.ResourceKey;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.monster.cubemob.AbstractCubeMob;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.dimension.DimensionType;

/**
 * The 26.2 shim of the shared modern harness (same static methods as every version's
 * {@code Compat}). What 26.2 changed for the harness:
 * <ul>
 * <li>The client level reports block / section changes to a {@code LevelExtractor} (split from
 * {@code LevelRenderer}); the stand-in is an extractor.</li>
 * <li>The level-load tracker no longer polls the renderer: the extractor runs the listener's
 * {@code getPlayerCompiledSectionCallback} once the player's section is compiled (a frame between
 * ticks), which sets the tracker's {@code playerSectionReady} flag for good.</li>
 * <li>{@code GameRenderer.getMainCamera()} is {@code mainCamera()}; slimes / magma cubes (and the
 * new sulfur cube) share {@code AbstractCubeMob} in {@code monster.cubemob}; the social manager's
 * constructor takes friend services (it is allocated without it anyway).</li>
 * </ul>
 */
final class Compat {
    private Compat() {}

    /** Whether the player's section counts as compiled and visible (scenarios set when). */
    private static boolean compiled;

    static void installStubs(Minecraft client) {
        Reflect.set(Minecraft.class, client, "levelExtractor", NoLevelExtractor.create());
        Reflect.set(Minecraft.class, client, "playerSocialManager", NoSocialManager.create());
    }

    /** An allocated {@code Gui} whose {@code Hud} holds an allocated debug overlay (26.2 moved the overlay into the hud). */
    static void installGui(Minecraft client) {
        net.minecraft.client.gui.Gui gui = Reflect.allocate(net.minecraft.client.gui.Gui.class);
        net.minecraft.client.gui.Hud hud = Reflect.allocate(net.minecraft.client.gui.Hud.class);
        Reflect.set(net.minecraft.client.gui.Hud.class, hud, "debugOverlay",
                Reflect.allocate(net.minecraft.client.gui.components.DebugScreenOverlay.class));
        Reflect.set(net.minecraft.client.gui.Gui.class, gui, "hud", hud);
        Reflect.set(Minecraft.class, client, "gui", gui);
    }

    /** The open screen, set without {@code setScreen}'s side effects (26.2: the screen lives in the {@code Gui}). */
    static void setScreen(Minecraft client, net.minecraft.client.gui.screens.Screen screen) {
        Reflect.set(net.minecraft.client.gui.Gui.class, client.gui, "screen", screen);
    }

    static Camera camera(GameRenderer renderer) {
        return renderer.mainCamera();
    }

    static ClientLevel newLevel(Minecraft client, ClientPacketListener listener, ClientLevel.ClientLevelData data,
            ResourceKey<Level> key, Holder<DimensionType> type, int loadDistance, int simulationDistance, int seaLevel) {
        return new ClientLevel(listener, data, key, type, loadDistance, simulationDistance, client.levelExtractor, false, 0L, seaLevel);
    }

    static void startClientLoad(Minecraft client, LevelLoadTracker tracker, LocalPlayer player, ClientLevel level) {
        tracker.startClientLoad(player, level);
    }

    static void setSectionsCompiled(boolean value) {
        compiled = value;
    }

    /**
     * The frame before the tick: once the player's section is compiled, the extractor runs the
     * tracker's callback (only while the tracker waits for the player's chunk; it returns null
     * otherwise).
     */
    static void beforeLoadTick(ClientPacketListener listener, LevelLoadTracker tracker) {
        if (compiled) {
            Runnable callback = listener.getPlayerCompiledSectionCallback();
            if (callback != null) {
                callback.run();
            }
        }
    }

    static boolean setCubeSize(Entity e, int size) {
        if (e instanceof AbstractCubeMob s) {
            s.setSize(size, true);
            return true;
        }
        return false;
    }

    static boolean isCubeMob(Entity e) {
        return e instanceof AbstractCubeMob;
    }

    /** The extractor the client level reports block / section changes to. */
    static final class NoLevelExtractor extends LevelExtractor {
        private NoLevelExtractor() {
            super(null, null, null); // never called
        }

        static NoLevelExtractor create() {
            return Reflect.allocate(NoLevelExtractor.class);
        }

        @Override
        public void setLevel(ClientLevel level) {
        }

        @Override
        public void allChanged() {
        }

        @Override
        public void blockChanged(BlockPos pos, int flags) {
        }

        @Override
        public void setBlocksDirty(int x0, int y0, int z0, int x1, int y1, int z1) {
        }

        @Override
        public void setBlockDirty(BlockPos pos, BlockState old, BlockState current) {
        }

        @Override
        public void setSectionDirtyWithNeighbors(int x, int y, int z) {
        }

        @Override
        public void setSectionRangeDirty(int x0, int y0, int z0, int x1, int y1, int z1) {
        }

        @Override
        public void setSectionDirty(int x, int y, int z) {
        }
    }

    /** The social interactions screen's player list (fed by player info packets). */
    static final class NoSocialManager extends PlayerSocialManager {
        private NoSocialManager() {
            super(null, null, null, null); // never called
        }

        static NoSocialManager create() {
            return Reflect.allocate(NoSocialManager.class);
        }

        @Override
        public void addPlayer(PlayerInfo info) {
        }

        @Override
        public void removePlayer(java.util.UUID id) {
        }
    }
}
