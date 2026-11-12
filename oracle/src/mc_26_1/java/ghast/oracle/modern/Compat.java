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
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.resources.ResourceKey;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.monster.Slime;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.dimension.DimensionType;

/**
 * The 26.1 shim of the shared modern harness: every call whose shape differs between modern
 * versions (renamed or moved classes, changed constructors) goes through here. Each version's
 * source set has a {@code Compat} with the same static methods.
 */
final class Compat {
    private Compat() {}

    /** Puts the stand-ins whose shape is version-specific into the allocated client. */
    static void installStubs(Minecraft client) {
        Reflect.set(Minecraft.class, client, "levelRenderer", NoLevelRenderer.create());
        Reflect.set(Minecraft.class, client, "playerSocialManager", NoSocialManager.create());
    }

    /** An allocated {@code Gui} holding an allocated debug overlay (the packet listener asks it for its ping logger). */
    static void installGui(Minecraft client) {
        net.minecraft.client.gui.Gui gui = Reflect.allocate(net.minecraft.client.gui.Gui.class);
        Reflect.set(net.minecraft.client.gui.Gui.class, gui, "debugOverlay",
                Reflect.allocate(net.minecraft.client.gui.components.DebugScreenOverlay.class));
        Reflect.set(Minecraft.class, client, "gui", gui);
    }

    /** The open screen, set without {@code setScreen}'s side effects. */
    static void setScreen(Minecraft client, net.minecraft.client.gui.screens.Screen screen) {
        Reflect.set(Minecraft.class, client, "screen", screen);
    }

    static Camera camera(GameRenderer renderer) {
        return renderer.getMainCamera();
    }

    static ClientLevel newLevel(Minecraft client, ClientPacketListener listener, ClientLevel.ClientLevelData data,
            ResourceKey<Level> key, Holder<DimensionType> type, int loadDistance, int simulationDistance, int seaLevel) {
        return new ClientLevel(listener, data, key, type, loadDistance, simulationDistance, client.levelRenderer, false, 0L, seaLevel);
    }

    static void startClientLoad(Minecraft client, LevelLoadTracker tracker, LocalPlayer player, ClientLevel level) {
        tracker.startClientLoad(player, level, client.levelRenderer);
    }

    /** Whether the player's section counts as compiled and visible (the level-load gate reads it). */
    static void setSectionsCompiled(boolean compiled) {
        NoLevelRenderer.compiled = compiled;
    }

    /** Before the level-load tracker ticks (26.1 polls the renderer from the tracker: nothing to do). */
    static void beforeLoadTick(ClientPacketListener listener, LevelLoadTracker tracker) {
    }

    /** Sets a size-variable cube mob's size; false if the entity is not one. */
    static boolean setCubeSize(Entity e, int size) {
        if (e instanceof Slime s) {
            s.setSize(size, true);
            return true;
        }
        return false;
    }

    static boolean isCubeMob(Entity e) {
        return e instanceof Slime;
    }

    /** The level renderer the client level reports block / section changes to. */
    static final class NoLevelRenderer extends LevelRenderer {
        private NoLevelRenderer() {
            super(null, null, null, null, null, null); // never called
        }

        static NoLevelRenderer create() {
            return Reflect.allocate(NoLevelRenderer.class);
        }

        /**
         * Whether the player's section counts as compiled and visible (the real renderer compiles
         * sections asynchronously some frames after their chunk arrived; scenarios set when).
         */
        static boolean compiled;

        @Override
        public boolean isSectionCompiledAndVisible(BlockPos pos) {
            return compiled;
        }

        @Override
        public void blockChanged(BlockGetter level, BlockPos pos, BlockState old, BlockState current, int flags) {
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

        @Override
        public void onSectionBecomingNonEmpty(long section) {
        }

        @Override
        public void destroyBlockProgress(int id, BlockPos pos, int progress) {
        }

        @Override
        public void onChunkReadyToRender(ChunkPos pos) {
        }

        @Override
        public void needsUpdate() {
        }
    }

    /** The social interactions screen's player list (fed by player info packets). */
    static final class NoSocialManager extends PlayerSocialManager {
        private NoSocialManager() {
            super(null, null); // never called
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
