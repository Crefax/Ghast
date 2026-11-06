package ghast.oracle.v1_12_2;

import net.minecraft.client.sound.SoundInstance;
import net.minecraft.client.sound.SoundManager;

/**
 * Sound manager that drops every sound. Allocated without its constructor (no sound system); step and
 * fall sounds are requested by the movement code but have no effect on it.
 */
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
}
