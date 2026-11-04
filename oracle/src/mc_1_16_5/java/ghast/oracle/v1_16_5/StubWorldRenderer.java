package ghast.oracle.v1_16_5;

import net.minecraft.block.BlockState;
import net.minecraft.client.render.WorldRenderer;
import net.minecraft.entity.player.PlayerEntity;
import net.minecraft.particle.ParticleEffect;
import net.minecraft.util.math.BlockPos;
import net.minecraft.world.BlockView;

/**
 * The world renderer the client world reports block updates, particles and world events to. Nothing is
 * rendered headless, so every hook the movement tick can reach is a no-op. Allocated without its
 * constructor.
 */
final class StubWorldRenderer extends WorldRenderer {
    private StubWorldRenderer() {
        super(null, null); // never called
    }

    static StubWorldRenderer create() {
        return Reflect.allocate(StubWorldRenderer.class);
    }

    @Override
    public void updateBlock(BlockView world, BlockPos pos, BlockState oldState, BlockState newState, int flags) {
    }

    @Override
    public void scheduleBlockRenders(int minX, int minY, int minZ, int maxX, int maxY, int maxZ) {
    }

    @Override
    public void scheduleBlockRerenderIfNeeded(BlockPos pos, BlockState old, BlockState updated) {
    }

    @Override
    public void scheduleBlockRenders(int x, int y, int z) {
    }

    @Override
    public void scheduleBlockRender(int x, int y, int z) {
    }

    @Override
    public void addParticle(ParticleEffect parameters, boolean shouldAlwaysSpawn, double x, double y, double z,
                            double velocityX, double velocityY, double velocityZ) {
    }

    @Override
    public void addParticle(ParticleEffect parameters, boolean shouldAlwaysSpawn, boolean important, double x, double y,
                            double z, double velocityX, double velocityY, double velocityZ) {
    }

    @Override
    public void processGlobalEvent(int eventId, BlockPos pos, int data) {
    }

    @Override
    public void processWorldEvent(PlayerEntity source, int eventId, BlockPos pos, int data) {
    }

    @Override
    public void setBlockBreakingInfo(int entityId, BlockPos pos, int stage) {
    }
}
