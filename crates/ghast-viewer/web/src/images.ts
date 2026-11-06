/**
 * Every image the page takes from the texture pack besides the block and item layers: entity skins and
 * models, HUD, sky, weather, particles, biome colour maps. `npm run build` writes this list to
 * dist/images.json; the server's pack builder reads it, so a new model here needs no server change.
 * Paths are below assets/minecraft/textures/ without the .png extension.
 */
import { armourTextures } from './armour.js';
import { BANNER_PATTERNS, mobTextures } from './mobs.js';
import { objectTextures } from './objects.js';

export function pageImages(): string[] {
  const list = [
    'entity/steve', 'entity/alex', 'gui/widgets', 'gui/icons', 'environment/sun', 'environment/moon_phases', 'environment/clouds',
    'colormap/grass', 'colormap/foliage', 'font/ascii', 'misc/shadow', 'entity/chest/normal', 'entity/chest/trapped', 'entity/chest/ender',
    'entity/pig/pig', 'entity/cow/cow', 'entity/sheep/sheep', 'entity/chicken', 'entity/zombie/zombie', 'entity/skeleton/skeleton',
    'entity/creeper/creeper', 'entity/spider/spider', 'entity/villager/villager', 'entity/enderman/enderman', 'entity/wolf/wolf',
    'entity/banner_base', 'entity/experience_orb', ...BANNER_PATTERNS.map(p => `entity/banner/${p[1]}`), 'environment/rain', 'environment/snow',
    'particle/particles', 'entity/explosion', ...mobTextures(), ...objectTextures(), ...armourTextures(),
  ];
  return list.filter((v, i, a) => a.indexOf(v) === i);
}
