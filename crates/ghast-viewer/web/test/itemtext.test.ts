/** The text of a 26.1 session (itemtext.ts): real names, counts, damage, enchantments, stand-in marks. */
import { describe, expect, it } from 'vitest';
import { armourLine, durability, enchantmentText, entityTag, gameLine, itemLines, itemTitle, roman, titleCase, type ItemText } from '../src/itemtext.js';

const sword: ItemText = { id: 'minecraft:netherite_sword', name: 'Netherite Sword', count: 1, damage: 31, maxDamage: 2031, standIn: true, drawn: [276, 0] };
const pick: ItemText = { id: 'minecraft:diamond_pickaxe', name: 'Diamond Pickaxe', count: 1, damage: 100, maxDamage: 1561, customName: 'Digger',
  enchantments: [{ id: 'minecraft:efficiency', level: 3 }, { id: 'minecraft:mending', level: 1 }, { registry: 8, level: 2 }], standIn: false, drawn: [278, 100] };

describe('item text', () => {
  it('names the real item, marks stand-ins', () => {
    expect(itemTitle(sword)).toBe('Netherite Sword ≈');
    expect(itemTitle(pick)).toBe('Digger (Diamond Pickaxe)');
    expect(itemTitle({ id: 'minecraft:copper_ingot', name: 'Copper Ingot', count: 5, standIn: true })).toBe('Copper Ingot ×5 ≈');
    expect(itemTitle({ id: 'minecraft:cherry_planks', name: '', count: 1, standIn: true })).toBe('Cherry Planks ≈');
  });
  it('lists enchantments, durability, the id and the stand-in note', () => {
    expect(itemLines(pick)).toEqual(['Digger (Diamond Pickaxe)', 'Efficiency III', 'Mending', 'Enchantment #8 II', 'Durability: 1461 / 1561', 'minecraft:diamond_pickaxe']);
    expect(itemLines(sword)).toEqual(['Netherite Sword ≈', 'Durability: 2000 / 2031', 'minecraft:netherite_sword', '≈ drawn with a 1.8.9 look-alike icon']);
  });
  it('durability from the real damage', () => {
    expect(durability(pick)).toBeCloseTo(1461 / 1561);
    expect(durability({ id: 'minecraft:stone', name: 'Stone', count: 64, standIn: false })).toBeNull();
  });
  it('helpers', () => {
    expect(roman(4)).toBe('IV'); expect(roman(12)).toBe('12');
    expect(titleCase('minecraft:fire_aspect')).toBe('Fire Aspect');
    expect(enchantmentText({ id: 'minecraft:sharpness', level: 5 })).toBe('Sharpness V');
    expect(armourLine([{ id: 'minecraft:turtle_helmet', name: 'Turtle Shell', count: 1, standIn: true }, null, null, null])).toBe('Armour: Turtle Shell ≈');
    expect(armourLine([null, null, null, null])).toBe('');
  });
  it('entity tags only for stand-ins, the header names the game', () => {
    expect(entityTag({ id: 9, type: 'minecraft:allay', name: 'Allay', standIn: true })).toBe('Allay ≈');
    expect(entityTag({ id: 11, type: 'minecraft:sheep', name: 'Sheep', standIn: false })).toBeNull();
    expect(entityTag({ id: 10, type: 'minecraft:item', name: 'Item', standIn: false, item: { id: 'minecraft:cherry_planks', name: 'Cherry Planks', count: 3, standIn: true } })).toBe('Cherry Planks ×3 ≈');
    expect(entityTag({ id: 5, type: 'minecraft:item', name: 'Item', standIn: false, item: { id: 'minecraft:oak_planks', name: 'Oak Planks', count: 64, standIn: false } })).toBeNull();
    expect(gameLine({ game: '26.1', drawnWith: '1.8.9' })).toBe(' · 26.1 (drawn with 1.8.9 assets)');
    expect(gameLine({})).toBe('');
  });
});
