package ghast.oracle.modern;

import net.minecraft.client.Camera;
import net.minecraft.client.particle.Particle;
import net.minecraft.client.particle.ParticleEngine;
import net.minecraft.client.resources.sounds.SoundInstance;
import net.minecraft.client.resources.sounds.TickableSoundInstance;
import net.minecraft.client.sounds.SoundEngine;
import net.minecraft.client.sounds.SoundManager;
import net.minecraft.core.particles.ParticleOptions;
import net.minecraft.world.entity.Entity;

/**
 * Headless stand-ins for the client subsystems the movement tick reaches but that have no effect on
 * movement: sound, particles, block entity renderers and entity renderers (the level renderer and the
 * social manager, whose shapes change between versions, are in the per-version {@code Compat}). Each is a real subclass
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
