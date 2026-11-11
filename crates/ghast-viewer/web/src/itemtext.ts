/**
 * The text of a 26.1 session: the game draws a 26.1 world with the 1.8.9 assets (look-alike icons and models),
 * but every name, count, damage and enchantment the page shows is the real one from `ITEMS` / `ENTITY` / `HELLO`
 * (docs/protocol.md). Pure functions, so they are unit-tested without a DOM. 1.8 sessions send none of these
 * messages and nothing here runs for them.
 */

/** `ITEMS` entry. */
export interface ItemText {
  id: string; name: string; count: number;
  damage?: number; maxDamage?: number;
  enchantments?: { id?: string; registry?: number; legacy?: number; level: number }[];
  customName?: string; standIn: boolean; drawn?: [number, number];
}
/** `ITEMS` message. */
export interface Items { hotbar: (ItemText | null)[]; armour: (ItemText | null)[] }
/** `ENTITY` message. */
export interface EntityText { id: number; type: string; name: string; standIn: boolean; item?: { id: string; name: string; count: number; standIn: boolean } }

/** The mark of a stand-in icon / model (the 1.8.9 assets have no such thing). */
export const STAND_IN = '≈';

const ROMAN = ['', 'I', 'II', 'III', 'IV', 'V', 'VI', 'VII', 'VIII', 'IX', 'X'];
/** Enchantment levels the way the game writes them (roman up to X, then digits). */
export function roman(n: number): string { return n >= 0 && n < ROMAN.length ? ROMAN[n] : String(n); }

/** `minecraft:fire_aspect` → `Fire Aspect`. */
export function titleCase(id: string): string {
  return id.replace(/^[a-z0-9_.-]+:/, '').split('_').filter(Boolean).map(w => w[0].toUpperCase() + w.slice(1)).join(' ');
}

/** One enchantment: its name (or the registry index when the server's name is not known) and level. */
export function enchantmentText(e: NonNullable<ItemText['enchantments']>[number]): string {
  const name = e.id ? titleCase(e.id) : e.registry !== undefined ? `Enchantment #${e.registry}` : `Enchantment #${e.legacy ?? '?'}`;
  return e.level === 1 && e.id ? name : `${name} ${roman(e.level)}`;
}

/** The remaining durability as a fraction, `null` for items without damage. */
export function durability(t: ItemText): number | null {
  if (!t.maxDamage || t.maxDamage <= 0 || t.damage === undefined) return null;
  return Math.max(0, Math.min(1, (t.maxDamage - t.damage) / t.maxDamage));
}

/** The vanilla durability bar colour (hue from red at 0 to green at full). */
export function durabilityColor(f: number): string { return `hsl(${Math.round(f * 120)}, 100%, 50%)`; }

/** The item's name line: the custom name (with the item name), the count, the stand-in mark. */
export function itemTitle(t: ItemText): string {
  const name = t.customName ? `${t.customName} (${t.name || titleCase(t.id)})` : (t.name || titleCase(t.id));
  return `${name}${t.count > 1 ? ` ×${t.count}` : ''}${t.standIn ? ` ${STAND_IN}` : ''}`;
}

/** The tooltip: the name line, enchantments, durability, the id and what the icon really is. */
export function itemLines(t: ItemText): string[] {
  const lines = [itemTitle(t)];
  for (const e of t.enchantments ?? []) lines.push(enchantmentText(e));
  if (t.maxDamage) lines.push(`Durability: ${t.maxDamage - (t.damage ?? 0)} / ${t.maxDamage}`);
  lines.push(t.id);
  if (t.standIn) lines.push(`${STAND_IN} drawn with a 1.8.9 look-alike icon`);
  return lines;
}

/** The name tag of an entity whose model or item is a stand-in (others need none). */
export function entityTag(e: EntityText): string | null {
  if (e.item) return e.item.standIn ? `${e.item.name || titleCase(e.item.id)}${e.item.count > 1 ? ` ×${e.item.count}` : ''} ${STAND_IN}` : null;
  return e.standIn ? `${e.name || titleCase(e.type)} ${STAND_IN}` : null;
}

/** The header suffix of a session drawn with another version's assets. */
export function gameLine(hello: { game?: string; drawnWith?: string }): string {
  return hello.game ? ` · ${hello.game} (drawn with ${hello.drawnWith ?? '1.8.9'} assets)` : '';
}

/** The armour line (head first). */
export function armourLine(a: (ItemText | null)[]): string {
  const worn = a.filter((x): x is ItemText => !!x);
  return worn.length ? `Armour: ${worn.map(itemTitle).join(', ')}` : '';
}
