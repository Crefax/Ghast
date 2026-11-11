/**
 * Viewer wire protocol: plain binary WebSocket frames, little endian. This file is the page's side and the
 * reference encoder used by its tests; the server implements the same format (docs/protocol.md). No Node or
 * DOM imports here.
 *
 * Angles are radians: yaw 0 looks to −z (north), +yaw turns left (π/2 looks to −x); pitch > 0 looks up.
 * From vanilla degrees: yaw = π − yawDegrees·π/180, pitch = −pitchDegrees·π/180.
 *
 * Every frame is a run of messages: [u8 type][u32 length][payload]. The server sends one frame per
 * client tick (pose + entity moves + block changes of that tick) and one frame per chunk column.
 *
 * Server → page
 *   HELLO      json {protocol, version, account, mode, viewDistance, now}
 *   STATE      json {inGame, message, status, account}
 *   CHUNK      i32 cx, i32 cz, u8 deflated, column bytes (encodeColumn; deflate-raw when deflated = 1)
 *   UNLOAD     i32 cx, i32 cz
 *   BLOCKS     u16 n × (i32 x, u8 y, i32 z, u16 stateId)            stateId = id << 4 | meta
 *   POSE       f64 serverMs, u32 tick, f64 x, y, z, f32 yaw, pitch, u8 flags, f32 vx, vy, vz
 *   SPAWN      u32 id, u8 kind, str name, str label, f64 x, y, z, f32 yaw, pitch, headYaw, f32 width, height, u16 item, u16 meta,
 *              u16 variant (mobVariant; may be missing: 0)
 *   MOVES      u16 n × (u32 id, f64 x, y, z, f32 yaw, pitch, headYaw)
 *   REMOVE     u16 n × u32 id
 *   EQUIP      u32 id, u16 item, u16 meta                             (an entity's held item / a dropped item's stack)
 *   INVENTORY  u8 held, 9 × (u16 id, u16 meta, u8 count)              hotbar, id 0 = empty
 *   VITALS     f32 health, u8 food, f32 saturation, u8 xpLevel, f32 xpProgress
 *   TIME       f64 serverMs, i64-as-f64 age, f64 timeOfDay (0..24000, negative: frozen)
 *   PONG       f64 clientMs, f64 serverMs
 *   RESET      (empty) the bot changed or left the game: drop every column and entity
 *   CRACKS     u16 n × (i32 x, u8 y, i32 z, u16 stateId, u8 stage 0..9)   every block being broken now (the bot's own
 *              digging and block_break_animation of others); replaces the previous set
 *   WEATHER    f32 rain, f32 thunder (0..1)
 *   EVENTS     u16 n × (u32 id, u8 event)                              EVENT.*: arm swings, hurt, death (animation 0/1,
 *              entity_status 2/3) of others and the bot (id SELF)
 *   ESTATE     u16 n × (u32 id, u8 flags, u32 vehicle id + 1)          an entity's EFLAG bits and what it rides (0 = nothing),
 *              on change (entity_metadata index 0, attach_entity, bed/wake)
 *   COLLECT    u32 collected id, u32 collector id (SELF = the bot)     an item or orb flies to its collector (then REMOVE)
 *   CHESTS     u16 n × (i32 x, u8 y, i32 z, u8 viewers)                block_action of a chest / ender chest: the lid opens
 *              while viewers > 0
 *   SKIN       u32 id (SELF: the bot), u8 flags (SKIN_FLAG), str skin, str cape   a player's skin and cape as served
 *              paths (skins/<sha>.png, relative to the page); '' = the default skin (Alex when SLIM) / no cape.
 *              Sent after its SPAWN, and again when the download finished or the profile changed
 *   LOOKS      u32 id (SELF: the bot), u8 flags (LOOK), u8 n × (u8 slot 1..4 boots..helmet, u16 item, u16 meta, u32 rgb
 *              leather colour or NO_COLOR), then with LOOK.POSED 6 × 3 f32 degrees (armour stand head, body, left arm,
 *              right arm, left leg, right leg). After the SPAWN when it is not plain, and again on every change.
 *   PARTICLES  u16 n × (u8 id, f64 x, y, z, f32 ox, oy, oz, f32 speed, u16 count, u16 data)   world_particles (id = the
 *              1.8 ParticleType id; data = block state id << 4 | meta for blockcrack/blockdust/iconcrack) and
 *              world_event effects as id FX_EVENT + (effect − 2000) (data = the event's data)
 *   EXPLOSION  f64 x, y, z, f32 radius, u16 n × (i8 dx, dy, dz)     the affected blocks relative to the centre
 *   LIGHTNING  f64 x, y, z                                          spawn_entity_weather (a bolt)
 *   REPLAY     f64 cursor ms, f64 duration ms, f32 speed, f64 start   a recording's playback, ~4 per second
 *
 * Page → server (never reaches the Minecraft connection)
 *   PING       f64 clientMs
 *   ACK        f64 bytes received so far (flow control window for chunk columns)
 *   REPLAY     f64 speed (0 paused), f64 seek ms (−1 none)          only on a recording's socket
 */
export const PROTOCOL = 1;

export const S = {
  HELLO: 1, STATE: 2, CHUNK: 3, UNLOAD: 4, BLOCKS: 5, POSE: 6, SPAWN: 7, MOVES: 8, REMOVE: 9,
  EQUIP: 10, INVENTORY: 11, VITALS: 12, TIME: 13, PONG: 14, RESET: 15, CRACKS: 16, WEATHER: 17,
  EVENTS: 18, ESTATE: 19, COLLECT: 20, CHESTS: 21, SKIN: 22, LOOKS: 23,
  PARTICLES: 24, EXPLOSION: 25, LIGHTNING: 26, REPLAY: 27, ITEMS: 28, ENTITY: 29,
} as const;
/** Page → server. REPLAY (recordings only): f64 speed (0 = paused), f64 seek ms from the start (−1: none). */
export const C = { PING: 1, ACK: 2, REPLAY: 3 } as const;

/** POSE flags (SWING: the bot swung its arm since the last pose; USE: it is using its held item; RIDE: on a vehicle). */
export const POSE = { GROUND: 1, FLYING: 2, SNEAK: 4, SPRINT: 8, SWING: 16, USE: 32, RIDE: 64 } as const;
/** SPAWN kinds (ORB: an experience orb, its value in the SPAWN meta). */
export const KIND = { OTHER: 0, PLAYER: 1, ITEM: 2, MOB: 3, OBJECT: 4, ORB: 5 } as const;
/** EVENTS kinds. */
export const EVENT = { SWING: 0, HURT: 1, DEATH: 2 } as const;
/** ESTATE flags: 1.8's entity metadata 0 (fire, sneak, sprint, using an item, invisible), plus SLEEP (in a bed; bit 4 is unused in 1.8). */
export const EFLAG = { FIRE: 1, SNEAK: 2, SLEEP: 4, SPRINT: 8, USE: 16, INVISIBLE: 32 } as const;
/** SKIN flags. */
export const SKIN_FLAG = { SLIM: 1 } as const;
/** The id that stands for the bot itself in EVENTS, COLLECT, SKIN and LOOKS (its own entity is never spawned on the page). */
export const SELF = 0xffffffff;

/** LOOKS flags: a baby (or a small armour stand), an armour stand with its pose. */
export const LOOK = { BABY: 1, POSED: 2 } as const;
export const NO_COLOR = 0xffffffff;

/** What a model wears and how big it is: the LOOKS message. */
export interface Looks {
  flags: number;
  /** [slot 1..4 (boots, leggings, chestplate, helmet as in entity_equipment), item, meta, leather rgb or NO_COLOR]. */
  armour: Array<[number, number, number, number]>;
  /** Armour stand rotations in degrees: head, body, left arm, right arm, left leg, right leg (x, y, z each). */
  pose?: number[];
}

export function writeLooks(w: Writer, id: number, l: Looks): void {
  const at = w.begin(S.LOOKS);
  w.u32(id).u8(l.flags | (l.pose ? LOOK.POSED : 0)).u8(l.armour.length);
  for (const [slot, item, meta, color] of l.armour) w.u8(slot).u16(item).u16(meta).u32(color);
  if (l.pose) for (let i = 0; i < 18; i++) w.f32(l.pose[i] ?? 0);
  w.end(at);
}

export function readLooks(r: Reader): { id: number } & Looks {
  const id = r.u32(), flags = r.u8(), n = r.u8();
  const armour: Looks['armour'] = [];
  for (let i = 0; i < n; i++) armour.push([r.u8(), r.u16(), r.u16(), r.u32()]);
  let pose: number[] | undefined;
  if (flags & LOOK.POSED) { pose = []; for (let i = 0; i < 18; i++) pose.push(r.f32()); }
  return { id, flags: flags & ~LOOK.POSED, armour, pose };
}

/** Minecart display blocks by class (AbstractMinecartEntity*.getDefaultContainedBlock, facing north), as id << 4 | meta. */
const CART_BLOCKS: Record<string, number> = {
  MinecartChest: (54 << 4) | 2, MinecartFurnace: (61 << 4) | 2, MinecartTNT: 46 << 4, MinecartSpawner: 52 << 4,
  MinecartHopper: 154 << 4, MinecartCommandBlock: 137 << 4,
};

/**
 * The SPAWN variant of a mob or object from its 1.8 entity metadata (values by metadata index):
 * slime / magma cube size, villager profession, sheep fleece colour | 16 when sheared, horse type << 8 | colour,
 * skeleton type (1 = wither), zombie villager (1), wolf flags (1 sitting, 2 angry, 4 tame) | collar dye << 4,
 * ocelot and rabbit type, a hanging bat (1), a shooting ghast (1), an elder guardian (1), armour stand flags,
 * a minecart's block (id << 4 | meta: its own display tile or its class's, lit furnace when powered).
 * 0 for everything else.
 */
export function mobVariant(name: string, meta: ReadonlyArray<unknown> | Record<number, unknown> | undefined): number {
  const at = (i: number): number => { const v = Number((meta as any)?.[i] ?? 0); return Number.isFinite(v) ? v : 0; };
  switch (name) {
    case 'Slime': case 'LavaSlime': return Math.max(0, Math.min(255, at(16)));
    case 'Villager': return Math.max(0, Math.min(255, at(16)));
    case 'Sheep': return at(16) & 31;
    case 'EntityHorse': return ((at(19) & 7) << 8) | (at(20) & 0xff);
    case 'Skeleton': return at(13) & 1;
    case 'Zombie': return at(13) & 1;
    case 'Wolf': return (at(16) & 7) | ((at(20) & 15) << 4);
    case 'Ozelot': return at(18) & 3;
    case 'Rabbit': return at(18) === 99 ? 99 : at(18) & 7;
    case 'Bat': case 'Ghast': return at(16) & 1;
    // GuardianEntity: flag 4 of the int at 16 is the elder
    case 'Guardian': return (at(16) & 4) ? 1 : 0;
    case 'ArmorStand': return at(10) & 31;
    default: {
      if (!name.startsWith('Minecart')) return 0;
      // 1.8 display tile: block id | meta << 12, used when byte 22 is set
      if (at(22)) { const s = at(20); return (((s & 0xfff) << 4) | ((s >> 12) & 15)) & 0xffff; }
      if (name === 'MinecartFurnace' && (at(16) & 1)) return (62 << 4) | 2;
      return CART_BLOCKS[name] ?? 0;
    }
  }
}

/** One world_particles packet (or a world_event effect, id ≥ FX_EVENT). */
export interface ParticleMsg { id: number; x: number; y: number; z: number; ox: number; oy: number; oz: number; speed: number; count: number; data: number }
/** world_event effects in PARTICLES: FX_EVENT + 0 smoke (2000), + 4 spawner flames (2004), + 5 bonemeal (2005). */
export const FX_EVENT = 100;

export function writeParticles(w: Writer, list: ReadonlyArray<ParticleMsg>): void {
  const at = w.begin(S.PARTICLES);
  w.u16(list.length);
  for (const p of list) {
    w.u8(p.id).f64(p.x).f64(p.y).f64(p.z).f32(p.ox).f32(p.oy).f32(p.oz).f32(p.speed);
    w.u16(Math.max(0, Math.min(65535, p.count | 0))).u16(p.data & 0xffff);
  }
  w.end(at);
}
export function readParticles(r: Reader): ParticleMsg[] {
  const out: ParticleMsg[] = [];
  for (let n = r.u16(), i = 0; i < n; i++) {
    out.push({ id: r.u8(), x: r.f64(), y: r.f64(), z: r.f64(), ox: r.f32(), oy: r.f32(), oz: r.f32(), speed: r.f32(), count: r.u16(), data: r.u16() });
  }
  return out;
}

export interface ExplosionMsg { x: number; y: number; z: number; radius: number; blocks: Array<[number, number, number]> }
export function writeExplosion(w: Writer, e: ExplosionMsg): void {
  const at = w.begin(S.EXPLOSION);
  const blocks = e.blocks.slice(0, 0xffff);
  w.f64(e.x).f64(e.y).f64(e.z).f32(e.radius).u16(blocks.length);
  for (const [dx, dy, dz] of blocks) w.u8(dx & 255).u8(dy & 255).u8(dz & 255);
  w.end(at);
}
export function readExplosion(r: Reader): ExplosionMsg {
  const x = r.f64(), y = r.f64(), z = r.f64(), radius = r.f32();
  const blocks: Array<[number, number, number]> = [];
  const s8 = (v: number) => (v << 24) >> 24;
  for (let n = r.u16(), i = 0; i < n; i++) blocks.push([s8(r.u8()), s8(r.u8()), s8(r.u8())]);
  return { x, y, z, radius, blocks };
}

const enc = new TextEncoder();
const dec = new TextDecoder();

/** Growable little-endian writer. */
export class Writer {
  buf: Uint8Array;
  view: DataView;
  pos = 0;
  constructor(size = 256) { this.buf = new Uint8Array(size); this.view = new DataView(this.buf.buffer); }

  private need(n: number): void {
    if (this.pos + n <= this.buf.length) return;
    let size = this.buf.length * 2;
    while (size < this.pos + n) size *= 2;
    const next = new Uint8Array(size);
    next.set(this.buf.subarray(0, this.pos));
    this.buf = next; this.view = new DataView(next.buffer);
  }
  u8(v: number): this { this.need(1); this.view.setUint8(this.pos, v); this.pos += 1; return this; }
  u16(v: number): this { this.need(2); this.view.setUint16(this.pos, v, true); this.pos += 2; return this; }
  u32(v: number): this { this.need(4); this.view.setUint32(this.pos, v >>> 0, true); this.pos += 4; return this; }
  i32(v: number): this { this.need(4); this.view.setInt32(this.pos, v, true); this.pos += 4; return this; }
  f32(v: number): this { this.need(4); this.view.setFloat32(this.pos, v, true); this.pos += 4; return this; }
  f64(v: number): this { this.need(8); this.view.setFloat64(this.pos, v, true); this.pos += 8; return this; }
  bytes(b: Uint8Array): this { this.need(b.length); this.buf.set(b, this.pos); this.pos += b.length; return this; }
  str(s: string): this { const b = enc.encode(s); this.u16(b.length); return this.bytes(b); }

  /** Starts a message; end() patches its length. */
  begin(type: number): number { this.u8(type); const at = this.pos; this.u32(0); return at; }
  end(at: number): void { this.view.setUint32(at, this.pos - at - 4, true); }

  done(): Uint8Array { return this.buf.subarray(0, this.pos); }
}

export class Reader {
  view: DataView;
  pos: number;
  readonly end: number;
  constructor(readonly buf: Uint8Array, start = 0, end = buf.length) {
    this.view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
    this.pos = start; this.end = end;
  }
  u8(): number { const v = this.view.getUint8(this.pos); this.pos += 1; return v; }
  u16(): number { const v = this.view.getUint16(this.pos, true); this.pos += 2; return v; }
  u32(): number { const v = this.view.getUint32(this.pos, true); this.pos += 4; return v; }
  i32(): number { const v = this.view.getInt32(this.pos, true); this.pos += 4; return v; }
  f32(): number { const v = this.view.getFloat32(this.pos, true); this.pos += 4; return v; }
  f64(): number { const v = this.view.getFloat64(this.pos, true); this.pos += 8; return v; }
  bytes(n: number): Uint8Array { const b = this.buf.subarray(this.pos, this.pos + n); this.pos += n; return b; }
  str(): string { return dec.decode(this.bytes(this.u16())); }
  rest(): Uint8Array { return this.bytes(this.end - this.pos); }
}

/** Splits a frame into its messages: [type, reader over the payload]. */
export function* messages(frame: Uint8Array): Generator<[number, Reader]> {
  const view = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
  let pos = 0;
  while (pos + 5 <= frame.length) {
    const type = view.getUint8(pos);
    const len = view.getUint32(pos + 1, true);
    const start = pos + 5;
    if (start + len > frame.length) throw new Error(`truncated message ${type}`);
    yield [type, new Reader(frame, start, start + len)];
    pos = start + len;
  }
}

export function json(w: Writer, type: number, value: unknown): void {
  const at = w.begin(type);
  w.bytes(enc.encode(JSON.stringify(value)));
  w.end(at);
}
export function readJson<T = any>(r: Reader): T { return JSON.parse(dec.decode(r.rest())) as T; }

// ---- chunk columns ---------------------------------------------------------------------------

/** One decoded 16×16×16 section: states in x + 16 * (z + 16 * y) order, light nibbles as in 1.8. */
export interface Section {
  y: number;
  states: Uint16Array;
  /** 2048 bytes, or null: all 15 (sky) / all 0 (block). */
  sky: Uint8Array | null;
  skyFull: boolean;
  block: Uint8Array | null;
}
/** A sign in a column: chunk-local position and its four lines (plain text). */
export interface SignText { x: number; y: number; z: number; lines: string[] }
/**
 * Other tile entities the page draws (chunk-local position): a skull (SkullType 0 skeleton, 1 wither,
 * 2 zombie, 3 player, 4 creeper; Rot 0..15 on the floor; `skin`: a player skull owner's served skin path,
 * none = Steve), a banner (base dye and pattern layers, dye damage values as in the NBT) or a flower pot's
 * contents (the blockstates "contents" value).
 */
export type Tile =
  | { x: number; y: number; z: number; kind: 'skull'; skull: number; rot: number; skin?: string }
  | { x: number; y: number; z: number; kind: 'banner'; base: number; patterns: Array<[string, number]> }
  | { x: number; y: number; z: number; kind: 'pot'; contents: string };
export interface Column { cx: number; cz: number; sections: Section[]; biomes: Uint8Array; hasSky: boolean; signs: SignText[]; tiles: Tile[] }

const TILE_KIND = { skull: 1, banner: 2, pot: 3 } as const;

const LIGHT_SKY_FULL = 1, LIGHT_SKY_DARK = 2, LIGHT_BLOCK_DARK = 4;

function allBytes(b: Uint8Array, start: number, len: number, v: number): boolean {
  for (let i = start; i < start + len; i++) if (b[i] !== v) return false;
  return true;
}

/**
 * Column body from 1.8 section buffers (per section 12288 bytes: 4096 × u16 LE states, 2048 block light,
 * 2048 sky light): u16 mask, u8 hasSky, per section in the mask: u8 bits, u16 palette n, palette u16[],
 * indices packed LSB first at `bits` (0 = one state), u8 light flags, the light arrays not covered by a flag;
 * then 256 biome bytes, then u16 n signs × (u8 x, y, z, 4 × str), then u16 n tiles × (u8 x, y, z, u8 kind, body):
 * skull u8 type, u8 rot, str skin; banner u8 base, u8 n × (str pattern, u8 dye); pot str contents. Empty sections
 * (all air) are left out of the mask: the page lights them as open sky.
 */
export function encodeColumn(sections: ReadonlyArray<Uint8Array | null>, biomes: Uint8Array, hasSky: boolean, signs: SignText[] = [], tiles: Tile[] = []): Uint8Array {
  const w = new Writer(32 * 1024);
  let mask = 0;
  const empty: boolean[] = [];
  for (let y = 0; y < 16; y++) {
    const d = sections[y];
    const isEmpty = !d || allBytes(d, 0, 8192, 0);
    empty.push(isEmpty);
    if (!isEmpty) mask |= 1 << y;
  }
  w.u16(mask).u8(hasSky ? 1 : 0);
  const index = new Map<number, number>();
  const palette: number[] = [];
  const ids = new Uint16Array(4096);
  for (let y = 0; y < 16; y++) {
    if (empty[y]) continue;
    const d = sections[y]!;
    const dv = new DataView(d.buffer, d.byteOffset, d.byteLength);
    index.clear(); palette.length = 0;
    for (let i = 0; i < 4096; i++) {
      const s = dv.getUint16(i * 2, true);
      let k = index.get(s);
      if (k === undefined) { k = palette.length; index.set(s, k); palette.push(s); }
      ids[i] = k;
    }
    const bits = palette.length <= 1 ? 0 : palette.length <= 2 ? 1 : palette.length <= 4 ? 2 : palette.length <= 16 ? 4 : palette.length <= 256 ? 8 : 16;
    w.u8(bits).u16(palette.length);
    for (const s of palette) w.u16(s);
    if (bits === 16) for (let i = 0; i < 4096; i++) w.u16(ids[i]);
    else if (bits > 0) {
      const perByte = 8 / bits;
      for (let i = 0; i < 4096; i += perByte) {
        let b = 0;
        for (let j = 0; j < perByte; j++) b |= ids[i + j] << (j * bits);
        w.u8(b);
      }
    }
    const blockDark = allBytes(d, 8192, 2048, 0);
    const skyFull = hasSky && d.length >= 12288 && allBytes(d, 10240, 2048, 0xff);
    const skyDark = !hasSky || d.length < 12288 || allBytes(d, 10240, 2048, 0);
    w.u8((skyFull ? LIGHT_SKY_FULL : 0) | (skyDark && !skyFull ? LIGHT_SKY_DARK : 0) | (blockDark ? LIGHT_BLOCK_DARK : 0));
    if (!skyFull && !skyDark) w.bytes(d.subarray(10240, 12288));
    if (!blockDark) w.bytes(d.subarray(8192, 10240));
  }
  w.bytes(biomes.length === 256 ? biomes : new Uint8Array(256));
  w.u16(signs.length);
  for (const sg of signs) { w.u8(sg.x).u8(sg.y).u8(sg.z); for (let i = 0; i < 4; i++) w.str((sg.lines[i] ?? '').slice(0, 64)); }
  w.u16(tiles.length);
  for (const t of tiles) {
    w.u8(t.x).u8(t.y).u8(t.z).u8(TILE_KIND[t.kind]);
    if (t.kind === 'skull') w.u8(t.skull & 255).u8(t.rot & 15).str(t.skin ?? '');
    else if (t.kind === 'banner') {
      const pats = t.patterns.slice(0, 16);
      w.u8(t.base & 15).u8(pats.length);
      for (const [p, c] of pats) w.str(p.slice(0, 8)).u8(c & 15);
    } else w.str(t.contents.slice(0, 32));
  }
  return w.done();
}

export function decodeColumn(cx: number, cz: number, body: Uint8Array): Column {
  const r = new Reader(body);
  const mask = r.u16();
  const hasSky = r.u8() === 1;
  const sections: Section[] = [];
  for (let y = 0; y < 16; y++) {
    if (!(mask & (1 << y))) continue;
    const bits = r.u8();
    const n = r.u16();
    const palette = new Uint16Array(n);
    for (let i = 0; i < n; i++) palette[i] = r.u16();
    const states = new Uint16Array(4096);
    if (bits === 0) states.fill(palette[0] ?? 0);
    else if (bits === 16) for (let i = 0; i < 4096; i++) states[i] = palette[r.u16()];
    else {
      const perByte = 8 / bits, m = (1 << bits) - 1;
      for (let i = 0; i < 4096; i += perByte) {
        const b = r.u8();
        for (let j = 0; j < perByte; j++) states[i + j] = palette[(b >> (j * bits)) & m];
      }
    }
    const flags = r.u8();
    const skyFull = (flags & LIGHT_SKY_FULL) !== 0;
    const sky = flags & (LIGHT_SKY_FULL | LIGHT_SKY_DARK) ? null : r.bytes(2048).slice();
    const block = flags & LIGHT_BLOCK_DARK ? null : r.bytes(2048).slice();
    sections.push({ y, states, sky, skyFull, block });
  }
  const biomes = r.bytes(256).slice();
  const signs: SignText[] = [];
  if (r.pos + 2 <= r.end) {
    const n = r.u16();
    for (let i = 0; i < n; i++) { const x = r.u8(), y = r.u8(), z = r.u8(); signs.push({ x, y, z, lines: [r.str(), r.str(), r.str(), r.str()] }); }
  }
  const tiles: Tile[] = [];
  if (r.pos + 2 <= r.end) {
    const n = r.u16();
    for (let i = 0; i < n; i++) {
      const x = r.u8(), y = r.u8(), z = r.u8(), kind = r.u8();
      if (kind === TILE_KIND.skull) {
        const skull = r.u8(), rot = r.u8(), skin = r.str();
        tiles.push({ x, y, z, kind: 'skull', skull, rot, ...(skin ? { skin } : {}) });
      }
      else if (kind === TILE_KIND.banner) {
        const base = r.u8(), k = r.u8();
        const patterns: Array<[string, number]> = [];
        for (let j = 0; j < k; j++) patterns.push([r.str(), r.u8()]);
        tiles.push({ x, y, z, kind: 'banner', base, patterns });
      } else if (kind === TILE_KIND.pot) tiles.push({ x, y, z, kind: 'pot', contents: r.str() });
      else break; // a kind from a newer server: the rest cannot be parsed
    }
  }
  return { cx, cz, sections, biomes, hasSky, signs, tiles };
}

/** Light nibble of a section array (1.8 order: even index = low nibble). */
export function nibble(arr: Uint8Array, i: number): number {
  const b = arr[i >> 1];
  return i & 1 ? b >> 4 : b & 15;
}
