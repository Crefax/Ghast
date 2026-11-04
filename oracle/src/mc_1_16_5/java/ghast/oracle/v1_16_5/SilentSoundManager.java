package ghast.oracle.v1_16_5;

import net.minecraft.client.sound.SoundInstance;
import net.minecraft.client.sound.SoundManager;
import net.minecraft.client.sound.TickableSoundInstance;

/** Sound manager that drops every sound; allocated without its constructor (no sound system). */
final class SilentSoundManager extends SoundManager {
    private SilentSoundManager() {
        super(null, null); // never called: instances come from Reflect.allocate
    }

    static SilentSoundManager create() {
        return Reflect.allocate(SilentSoundManager.class);
    }

    @Override
    public void play(SoundInstance sound) {
    }

    @Override
    public void play(SoundInstance sound, int delay) {
    }

    @Override
    public void playNextTick(TickableSoundInstance sound) {
    }

    @Override
    public void stop(SoundInstance sound) {
    }

    @Override
    public boolean isPlaying(SoundInstance sound) {
        return false;
    }
}
