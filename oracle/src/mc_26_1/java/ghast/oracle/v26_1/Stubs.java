package ghast.oracle.v26_1;

import net.minecraft.client.Camera;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.client.particle.Particle;
import net.minecraft.client.particle.ParticleEngine;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.resources.sounds.SoundInstance;
import net.minecraft.client.resources.sounds.TickableSoundInstance;
import net.minecraft.client.sounds.SoundEngine;
import net.minecraft.client.sounds.SoundManager;
import net.minecraft.client.gui.screens.social.PlayerSocialManager;
import net.minecraft.core.BlockPos;
import net.minecraft.core.particles.ParticleOptions;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.block.state.BlockState;

/**
 * Headless stand-ins for the client subsystems the movement tick reaches but that have no effect on
 * movement: sound, particles, the level renderer, block entity renderers and the social manager. Each is a real subclass
 * allocated without its constructor (no audio device, no GPU), with every hook the tick or the packet
 * handlers can call turned into a no-op.
 */
final class Stubs {
    private Stubs() {}

    /** Drops every sound. */
    static final class SilentSoundManager extends SoundManager {
        private SilentSoundManager() {
            super(null); // never called: instances come from Reflect.allocate
        }

        static SilentSoundManager create() {
            return Reflect.allocate(SilentSoundManager.class);
        }

        @Override
        public SoundEngine.PlayResult play(SoundInstance sound) {
            return SoundEngine.PlayResult.NOT_STARTED;
        }

        @Override
        public void playDelayed(SoundInstance sound, int delay) {
        }

        @Override
        public void queueTickingSound(TickableSoundInstance sound) {
        }

        @Override
        public void stop(SoundInstance sound) {
        }

        @Override
        public boolean isActive(SoundInstance sound) {
            return false;
        }

        @Override
        public void updateSource(Camera camera) {
        }

        @Override
        public void tick(boolean paused) {
        }
    }

    /** Creates no particles (the client level calls it for sprint, landing, bubble particles). */
    static final class NoParticles extends ParticleEngine {
        private NoParticles() {
            super(null, null); // never called
        }

        static NoParticles create() {
            return Reflect.allocate(NoParticles.class);
        }

        @Override
        public Particle createParticle(ParticleOptions options, double x, double y, double z, double xd, double yd, double zd) {
            return null;
        }

        @Override
        public void add(Particle particle) {
        }

        @Override
        public void createTrackingEmitter(Entity entity, ParticleOptions options) {
        }

        @Override
        public void createTrackingEmitter(Entity entity, ParticleOptions options, int lifeTime) {
        }
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

    /** Block entity renderers: none (the client level asks for one when a block entity is created). */
    static final class NoBlockEntityRenderers extends net.minecraft.client.renderer.blockentity.BlockEntityRenderDispatcher {
        private NoBlockEntityRenderers() {
            super(null, null, null, null, null, null, null); // never called
        }

        static NoBlockEntityRenderers create() {
            return Reflect.allocate(NoBlockEntityRenderers.class);
        }

        @Override
        public <E extends net.minecraft.world.level.block.entity.BlockEntity, S extends net.minecraft.client.renderer.blockentity.state.BlockEntityRenderState>
                net.minecraft.client.renderer.blockentity.BlockEntityRenderer<E, S> getRenderer(E blockEntity) {
            return null;
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

    /**
     * An entity render dispatcher that extracts an empty render state (the pickup particle of
     * {@code handleTakeItemEntity} asks for one); nothing renders.
     */
    static final class NoEntityRenderers extends net.minecraft.client.renderer.entity.EntityRenderDispatcher {
        private NoEntityRenderers() {
            super(null, null, null, null, null, null, null, null, null, null, null); // never called
        }

        static NoEntityRenderers create() {
            return Reflect.allocate(NoEntityRenderers.class);
        }

        @Override
        public <E extends Entity> net.minecraft.client.renderer.entity.state.EntityRenderState extractEntity(E entity, float partialTicks) {
            return new net.minecraft.client.renderer.entity.state.EntityRenderState();
        }
    }
}
