//! The bot screen wire protocol (see `docs/protocol.md`).
//!
//! Plain binary WebSocket frames, little endian. A frame is a run of messages, each
//! `[u8 type][u32 length][payload]`. The server sends one frame per client tick (the bot's pose,
//! entity moves, block changes of that tick) and one frame per chunk column; the page only sends
//! clock pings, flow-control acknowledgements and (on a recording) playback controls.
//!
//! This module has no I/O: [`Writer`] builds frames, [`Reader`] / [`messages`] take them apart,
//! [`encode_column`] / [`decode_column`] convert chunk columns.

/// Protocol revision sent in `HELLO`.
pub const PROTOCOL: u32 = 1;

/// Server → page message types.
pub mod s {
    pub const HELLO: u8 = 1;
    pub const STATE: u8 = 2;
    pub const CHUNK: u8 = 3;
    pub const UNLOAD: u8 = 4;
    pub const BLOCKS: u8 = 5;
    pub const POSE: u8 = 6;
    pub const SPAWN: u8 = 7;
    pub const MOVES: u8 = 8;
    pub const REMOVE: u8 = 9;
    pub const EQUIP: u8 = 10;
    pub const INVENTORY: u8 = 11;
    pub const VITALS: u8 = 12;
    pub const TIME: u8 = 13;
    pub const PONG: u8 = 14;
    pub const RESET: u8 = 15;
    pub const CRACKS: u8 = 16;
    pub const WEATHER: u8 = 17;
    pub const EVENTS: u8 = 18;
    pub const ESTATE: u8 = 19;
    pub const COLLECT: u8 = 20;
    pub const CHESTS: u8 = 21;
    pub const SKIN: u8 = 22;
    pub const LOOKS: u8 = 23;
    pub const PARTICLES: u8 = 24;
    pub const EXPLOSION: u8 = 25;
    pub const LIGHTNING: u8 = 26;
    pub const REPLAY: u8 = 27;
}

/// Page → server message types.
pub mod c {
    /// `f64` page clock (ms): answered with `PONG`.
    pub const PING: u8 = 1;
    /// `f64` bytes received so far (the column flow-control window).
    pub const ACK: u8 = 2;
    /// Recordings only: `f64` speed (0 = paused), `f64` seek ms from the start (−1: none).
    pub const REPLAY: u8 = 3;
}

/// `POSE` flags.
pub mod pose {
    pub const GROUND: u8 = 1;
    pub const FLYING: u8 = 2;
    pub const SNEAK: u8 = 4;
    pub const SPRINT: u8 = 8;
    /// The bot swung its arm since the last pose.
    pub const SWING: u8 = 16;
    /// The bot is using its held item.
    pub const USE: u8 = 32;
    /// The bot rides a vehicle.
    pub const RIDE: u8 = 64;
}

/// `SPAWN` kinds.
pub mod kind {
    pub const OTHER: u8 = 0;
    pub const PLAYER: u8 = 1;
    pub const ITEM: u8 = 2;
    pub const MOB: u8 = 3;
    pub const OBJECT: u8 = 4;
    /// An experience orb (its value in the `SPAWN` meta).
    pub const ORB: u8 = 5;
}

/// `EVENTS` kinds.
pub mod event {
    pub const SWING: u8 = 0;
    pub const HURT: u8 = 1;
    pub const DEATH: u8 = 2;
}

/// `ESTATE` flags: 1.8 entity metadata 0 (fire, sneak, sprint, using an item, invisible) plus
/// `SLEEP` (in a bed; bit 2 is unused in 1.8's flags byte).
pub mod eflag {
    pub const FIRE: u8 = 1;
    pub const SNEAK: u8 = 2;
    pub const SLEEP: u8 = 4;
    pub const SPRINT: u8 = 8;
    pub const USE: u8 = 16;
    pub const INVISIBLE: u8 = 32;
}

/// `SKIN` flags.
pub mod skin_flag {
    /// The slim (Alex) arm model.
    pub const SLIM: u8 = 1;
}

/// `LOOKS` flags.
pub mod look {
    /// A baby (or a small armour stand).
    pub const BABY: u8 = 1;
    /// An armour stand with its pose (18 floats follow).
    pub const POSED: u8 = 2;
}

/// The id that stands for the bot itself in `EVENTS`, `COLLECT`, `SKIN` and `LOOKS` (its own
/// entity is never spawned on the page).
pub const SELF: u32 = 0xffff_ffff;
/// "No leather colour" in `LOOKS`.
pub const NO_COLOR: u32 = 0xffff_ffff;
/// `PARTICLES` ids at and above this are world events: `FX_EVENT + (effect − 2000)`.
pub const FX_EVENT: u8 = 100;

// ---- writer / reader -----------------------------------------------------------------------

/// Growable little-endian frame writer.
#[derive(Debug, Default, Clone)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    /// An empty writer with room for `cap` bytes.
    #[must_use]
    pub fn with_capacity(cap: usize) -> Self {
        Writer {
            buf: Vec::with_capacity(cap),
        }
    }
    /// Bytes written so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    /// Nothing written yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn f64(&mut self, v: f64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(b);
        self
    }
    /// `u16` byte length + UTF-8 (cut at 65535 bytes on a character boundary).
    pub fn str(&mut self, s: &str) -> &mut Self {
        let s = cut_bytes(s, 0xffff);
        self.u16(s.len() as u16);
        self.bytes(s.as_bytes())
    }
    /// Starts a message; [`Writer::end`] patches its length.
    pub fn begin(&mut self, ty: u8) -> usize {
        self.buf.push(ty);
        let at = self.buf.len();
        self.buf.extend_from_slice(&[0; 4]);
        at
    }
    /// Ends the message started at `at`.
    pub fn end(&mut self, at: usize) {
        let len = (self.buf.len() - at - 4) as u32;
        self.buf[at..at + 4].copy_from_slice(&len.to_le_bytes());
    }
    /// A message with an empty payload.
    pub fn empty(&mut self, ty: u8) -> &mut Self {
        let at = self.begin(ty);
        self.end(at);
        self
    }
    /// A message whose payload is JSON.
    pub fn json(&mut self, ty: u8, value: &serde_json::Value) -> &mut Self {
        let at = self.begin(ty);
        // serde_json writes to a Vec without failing
        let _ = serde_json::to_writer(&mut self.buf, value);
        self.end(at);
        self
    }
    /// The bytes written.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }
    /// Takes the bytes, leaving the writer empty (its capacity goes with them).
    pub fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }
    /// The bytes written.
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }
}

/// The longest prefix of `s` of at most `max` bytes ending on a character boundary.
pub(crate) fn cut_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// The first `n` UTF-16 code units of `s` (JavaScript's `slice(0, n)`), never splitting a
/// character.
pub(crate) fn cut_utf16(s: &str, n: usize) -> &str {
    let mut units = 0;
    for (i, ch) in s.char_indices() {
        units += ch.len_utf16();
        if units > n {
            return &s[..i];
        }
    }
    s
}

/// A message or frame was shorter than its fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Truncated;

impl std::fmt::Display for Truncated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("truncated message")
    }
}
impl std::error::Error for Truncated {}

/// Bounds-checked little-endian reader over a byte slice.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

macro_rules! read_le {
    ($name:ident, $t:ty) => {
        pub fn $name(&mut self) -> Result<$t, Truncated> {
            const N: usize = size_of::<$t>();
            let b = self.take(N)?;
            Ok(<$t>::from_le_bytes(b.try_into().map_err(|_| Truncated)?))
        }
    };
}

impl<'a> Reader<'a> {
    /// A reader over `buf`.
    #[must_use]
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }
    /// Bytes left.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }
    /// The next `n` bytes.
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], Truncated> {
        if self.remaining() < n {
            return Err(Truncated);
        }
        let b = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(b)
    }
    pub fn u8(&mut self) -> Result<u8, Truncated> {
        Ok(self.take(1)?[0])
    }
    read_le!(u16, u16);
    read_le!(u32, u32);
    read_le!(i32, i32);
    read_le!(f32, f32);
    read_le!(f64, f64);
    /// `u16` length + UTF-8 (invalid UTF-8 is replaced).
    pub fn str(&mut self) -> Result<String, Truncated> {
        let n = self.u16()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    /// The rest of the payload.
    pub fn rest(&mut self) -> &'a [u8] {
        let b = &self.buf[self.pos..];
        self.pos = self.buf.len();
        b
    }
}

/// One message of a frame: its type and payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message<'a> {
    pub ty: u8,
    pub body: &'a [u8],
    /// The whole message, header included.
    pub raw: &'a [u8],
}

/// Splits a frame into its messages. A truncated tail yields one `Err` and ends the iteration.
pub fn messages(frame: &[u8]) -> impl Iterator<Item = Result<Message<'_>, Truncated>> + '_ {
    let mut pos = 0usize;
    let mut failed = false;
    std::iter::from_fn(move || {
        if failed || pos + 5 > frame.len() {
            return None;
        }
        let ty = frame[pos];
        let len = u32::from_le_bytes([
            frame[pos + 1],
            frame[pos + 2],
            frame[pos + 3],
            frame[pos + 4],
        ]) as usize;
        let start = pos + 5;
        if start + len > frame.len() {
            failed = true;
            return Some(Err(Truncated));
        }
        let m = Message {
            ty,
            body: &frame[start..start + len],
            raw: &frame[pos..start + len],
        };
        pos = start + len;
        Some(Ok(m))
    })
}

// ---- composite messages --------------------------------------------------------------------

/// What a model wears and how big it is: the `LOOKS` message.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Looks {
    /// [`look::BABY`] (`POSED` is set by the encoder when `pose` is there).
    pub flags: u8,
    /// `(slot 1..4 boots..helmet, item, meta, leather rgb or NO_COLOR)`.
    pub armour: Vec<(u8, u16, u16, u32)>,
    /// Armour stand rotations in degrees: head, body, left arm, right arm, left leg, right leg
    /// (x, y, z each).
    pub pose: Option<[f32; 18]>,
}

impl Looks {
    /// Nothing to show beyond the plain model.
    #[must_use]
    pub fn is_plain(&self) -> bool {
        self.flags == 0 && self.armour.is_empty() && self.pose.is_none()
    }
}

pub fn write_looks(w: &mut Writer, id: u32, l: &Looks) {
    let at = w.begin(s::LOOKS);
    let flags = l.flags | if l.pose.is_some() { look::POSED } else { 0 };
    let n = l.armour.len().min(255);
    w.u32(id).u8(flags).u8(n as u8);
    for &(slot, item, meta, color) in &l.armour[..n] {
        w.u8(slot).u16(item).u16(meta).u32(color);
    }
    if let Some(p) = &l.pose {
        for v in p {
            w.f32(*v);
        }
    }
    w.end(at);
}

pub fn read_looks(r: &mut Reader<'_>) -> Result<(u32, Looks), Truncated> {
    let id = r.u32()?;
    let flags = r.u8()?;
    let n = r.u8()?;
    let mut armour = Vec::with_capacity(n as usize);
    for _ in 0..n {
        armour.push((r.u8()?, r.u16()?, r.u16()?, r.u32()?));
    }
    let pose = if flags & look::POSED != 0 {
        let mut p = [0f32; 18];
        for v in &mut p {
            *v = r.f32()?;
        }
        Some(p)
    } else {
        None
    };
    Ok((
        id,
        Looks {
            flags: flags & !look::POSED,
            armour,
            pose,
        },
    ))
}

/// One `world_particles` packet (or a world event, `id >= FX_EVENT`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    pub id: u8,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub ox: f32,
    pub oy: f32,
    pub oz: f32,
    pub speed: f32,
    pub count: u16,
    pub data: u16,
}

pub fn write_particles(w: &mut Writer, list: &[Particle]) {
    let at = w.begin(s::PARTICLES);
    let n = list.len().min(0xffff);
    w.u16(n as u16);
    for p in &list[..n] {
        w.u8(p.id).f64(p.x).f64(p.y).f64(p.z);
        w.f32(p.ox).f32(p.oy).f32(p.oz).f32(p.speed);
        w.u16(p.count).u16(p.data);
    }
    w.end(at);
}

pub fn read_particles(r: &mut Reader<'_>) -> Result<Vec<Particle>, Truncated> {
    let n = r.u16()?;
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        out.push(Particle {
            id: r.u8()?,
            x: r.f64()?,
            y: r.f64()?,
            z: r.f64()?,
            ox: r.f32()?,
            oy: r.f32()?,
            oz: r.f32()?,
            speed: r.f32()?,
            count: r.u16()?,
            data: r.u16()?,
        });
    }
    Ok(out)
}

/// An explosion and the blocks it affected (offsets from the centre).
#[derive(Debug, Clone, PartialEq)]
pub struct Explosion {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub radius: f32,
    pub blocks: Vec<[i8; 3]>,
}

pub fn write_explosion(w: &mut Writer, e: &Explosion) {
    let at = w.begin(s::EXPLOSION);
    let n = e.blocks.len().min(0xffff);
    w.f64(e.x).f64(e.y).f64(e.z).f32(e.radius).u16(n as u16);
    for b in &e.blocks[..n] {
        w.u8(b[0] as u8).u8(b[1] as u8).u8(b[2] as u8);
    }
    w.end(at);
}

pub fn read_explosion(r: &mut Reader<'_>) -> Result<Explosion, Truncated> {
    let (x, y, z, radius) = (r.f64()?, r.f64()?, r.f64()?, r.f32()?);
    let n = r.u16()?;
    let mut blocks = Vec::with_capacity(n as usize);
    for _ in 0..n {
        blocks.push([r.u8()? as i8, r.u8()? as i8, r.u8()? as i8]);
    }
    Ok(Explosion {
        x,
        y,
        z,
        radius,
        blocks,
    })
}

// ---- chunk columns -------------------------------------------------------------------------

/// Sky light of a section for [`encode_column`].
#[derive(Debug, Clone, Copy)]
pub enum Sky<'a> {
    /// All 15 (open sky; also the stand-in when light is not kept).
    Full,
    /// All 0 (or a dimension without sky).
    Dark,
    /// The 2048 nibble bytes as sent by the server.
    Data(&'a [u8; 2048]),
}

/// One 16×16×16 section for [`encode_column`]: states in `x + 16 * (z + 16 * y)` order
/// (1.8 `id << 4 | meta`), light nibbles in 1.8 order.
#[derive(Debug, Clone, Copy)]
pub struct SectionIn<'a> {
    pub states: &'a [u16; 4096],
    /// `None`: all 0.
    pub block_light: Option<&'a [u8; 2048]>,
    pub sky: Sky<'a>,
}

/// A sign in a column: chunk-local position and its four lines (plain text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignText {
    pub x: u8,
    pub y: u8,
    pub z: u8,
    pub lines: [String; 4],
}

/// Other tile entities the page draws (chunk-local position).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TileKind {
    /// Skull type (0 skeleton, 1 wither, 2 zombie, 3 player, 4 creeper), floor rotation 0..15 and
    /// a player skull's served skin path ('' = Steve).
    Skull { skull: u8, rot: u8, skin: String },
    /// Base dye and pattern layers (dye damage values as in the NBT).
    Banner {
        base: u8,
        patterns: Vec<(String, u8)>,
    },
    /// A flower pot's blockstates "contents" value.
    Pot { contents: String },
}

/// A tile entity at a chunk-local position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pub x: u8,
    pub y: u8,
    pub z: u8,
    pub kind: TileKind,
}

const TILE_SKULL: u8 = 1;
const TILE_BANNER: u8 = 2;
const TILE_POT: u8 = 3;

const LIGHT_SKY_FULL: u8 = 1;
const LIGHT_SKY_DARK: u8 = 2;
const LIGHT_BLOCK_DARK: u8 = 4;

fn all(b: &[u8], v: u8) -> bool {
    b.iter().all(|&x| x == v)
}

/// The column body: `u16` section mask, `u8` has sky, per section in the mask `u8` bits,
/// `u16` palette size, palette, indices packed LSB first at `bits` (0 = one state), `u8` light
/// flags and the light arrays no flag covers; then 256 biome bytes, signs and tiles.
/// All-air sections are left out of the mask (the page lights them as open sky).
#[must_use]
pub fn encode_column(
    sections: &[Option<SectionIn<'_>>; 16],
    biomes: Option<&[u8; 256]>,
    has_sky: bool,
    signs: &[SignText],
    tiles: &[Tile],
) -> Vec<u8> {
    let mut w = Writer::with_capacity(32 * 1024);
    let mut mask = 0u16;
    for (y, s) in sections.iter().enumerate() {
        if s.is_some_and(|s| s.states.iter().any(|&v| v != 0)) {
            mask |= 1 << y;
        }
    }
    w.u16(mask).u8(u8::from(has_sky));
    // palette index of a state: a flat table is faster than a map for 16-bit keys
    let mut index = vec![u16::MAX; 65536];
    let mut palette: Vec<u16> = Vec::with_capacity(64);
    let mut ids = [0u16; 4096];
    for (y, s) in sections.iter().enumerate() {
        if mask & (1 << y) == 0 {
            continue;
        }
        let Some(s) = s else { continue };
        for &p in &palette {
            index[p as usize] = u16::MAX;
        }
        palette.clear();
        for (i, &st) in s.states.iter().enumerate() {
            let mut k = index[st as usize];
            if k == u16::MAX {
                k = palette.len() as u16;
                index[st as usize] = k;
                palette.push(st);
            }
            ids[i] = k;
        }
        let n = palette.len();
        let bits: u8 = match n {
            0..=1 => 0,
            2 => 1,
            3..=4 => 2,
            5..=16 => 4,
            17..=256 => 8,
            _ => 16,
        };
        w.u8(bits).u16(n as u16);
        for &p in &palette {
            w.u16(p);
        }
        if bits == 16 {
            for &k in &ids {
                w.u16(k);
            }
        } else if bits > 0 {
            let per = 8 / bits as usize;
            for chunk in ids.chunks_exact(per) {
                let mut b = 0u8;
                for (j, &k) in chunk.iter().enumerate() {
                    b |= (k as u8) << (j * bits as usize);
                }
                w.u8(b);
            }
        }
        let block_dark = s.block_light.is_none_or(|b| all(b, 0));
        let (sky_full, sky_dark) = if !has_sky {
            (false, true)
        } else {
            match s.sky {
                Sky::Full => (true, false),
                Sky::Dark => (false, true),
                Sky::Data(d) => (all(d, 0xff), all(d, 0)),
            }
        };
        let flags = (if sky_full { LIGHT_SKY_FULL } else { 0 })
            | (if sky_dark && !sky_full {
                LIGHT_SKY_DARK
            } else {
                0
            })
            | (if block_dark { LIGHT_BLOCK_DARK } else { 0 });
        w.u8(flags);
        if !sky_full
            && !sky_dark
            && let Sky::Data(d) = s.sky
        {
            w.bytes(d);
        }
        if !block_dark && let Some(b) = s.block_light {
            w.bytes(b);
        }
    }
    match biomes {
        Some(b) => w.bytes(b),
        None => w.bytes(&[0; 256]),
    };
    let n = signs.len().min(0xffff);
    w.u16(n as u16);
    for sg in &signs[..n] {
        w.u8(sg.x).u8(sg.y).u8(sg.z);
        for l in &sg.lines {
            w.str(cut_utf16(l, 64));
        }
    }
    let n = tiles.len().min(0xffff);
    w.u16(n as u16);
    for t in &tiles[..n] {
        w.u8(t.x).u8(t.y).u8(t.z);
        match &t.kind {
            TileKind::Skull { skull, rot, skin } => {
                w.u8(TILE_SKULL).u8(*skull).u8(rot & 15).str(skin);
            }
            TileKind::Banner { base, patterns } => {
                let k = patterns.len().min(16);
                w.u8(TILE_BANNER).u8(base & 15).u8(k as u8);
                for (p, c) in &patterns[..k] {
                    w.str(cut_utf16(p, 8)).u8(c & 15);
                }
            }
            TileKind::Pot { contents } => {
                w.u8(TILE_POT).str(cut_utf16(contents, 32));
            }
        }
    }
    w.into_vec()
}

/// A decoded section (tests, tools).
#[derive(Debug, Clone, PartialEq)]
pub struct SectionOut {
    pub y: u8,
    pub states: Box<[u16; 4096]>,
    /// `None`: all 15 (`sky_full`) or all 0.
    pub sky: Option<Box<[u8; 2048]>>,
    pub sky_full: bool,
    pub block: Option<Box<[u8; 2048]>>,
}

/// A decoded column (tests, tools).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnOut {
    pub sections: Vec<SectionOut>,
    pub biomes: [u8; 256],
    pub has_sky: bool,
    pub signs: Vec<SignText>,
    pub tiles: Vec<Tile>,
}

fn arr2048(b: &[u8]) -> Box<[u8; 2048]> {
    let mut a = Box::new([0u8; 2048]);
    a.copy_from_slice(b);
    a
}

/// Decodes a column body ([`encode_column`]'s output).
pub fn decode_column(body: &[u8]) -> Result<ColumnOut, Truncated> {
    let mut r = Reader::new(body);
    let mask = r.u16()?;
    let has_sky = r.u8()? == 1;
    let mut sections = Vec::new();
    for y in 0..16u8 {
        if mask & (1 << y) == 0 {
            continue;
        }
        let bits = r.u8()?;
        let n = r.u16()? as usize;
        let mut palette = Vec::with_capacity(n);
        for _ in 0..n {
            palette.push(r.u16()?);
        }
        let at = |k: usize| palette.get(k).copied().unwrap_or(0);
        let mut states = Box::new([0u16; 4096]);
        match bits {
            0 => states.fill(at(0)),
            16 => {
                for s in states.iter_mut() {
                    *s = at(r.u16()? as usize);
                }
            }
            1 | 2 | 4 | 8 => {
                let per = 8 / bits as usize;
                let m = (1u16 << bits) - 1;
                for i in (0..4096).step_by(per) {
                    let b = u16::from(r.u8()?);
                    for j in 0..per {
                        states[i + j] = at(((b >> (j * bits as usize)) & m) as usize);
                    }
                }
            }
            _ => return Err(Truncated),
        }
        let flags = r.u8()?;
        let sky_full = flags & LIGHT_SKY_FULL != 0;
        let sky = if flags & (LIGHT_SKY_FULL | LIGHT_SKY_DARK) != 0 {
            None
        } else {
            Some(arr2048(r.take(2048)?))
        };
        let block = if flags & LIGHT_BLOCK_DARK != 0 {
            None
        } else {
            Some(arr2048(r.take(2048)?))
        };
        sections.push(SectionOut {
            y,
            states,
            sky,
            sky_full,
            block,
        });
    }
    let mut biomes = [0u8; 256];
    biomes.copy_from_slice(r.take(256)?);
    let mut signs = Vec::new();
    if r.remaining() >= 2 {
        for _ in 0..r.u16()? {
            let (x, y, z) = (r.u8()?, r.u8()?, r.u8()?);
            let lines = [r.str()?, r.str()?, r.str()?, r.str()?];
            signs.push(SignText { x, y, z, lines });
        }
    }
    let mut tiles = Vec::new();
    if r.remaining() >= 2 {
        for _ in 0..r.u16()? {
            let (x, y, z, k) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
            let kind = match k {
                TILE_SKULL => TileKind::Skull {
                    skull: r.u8()?,
                    rot: r.u8()?,
                    skin: r.str()?,
                },
                TILE_BANNER => {
                    let base = r.u8()?;
                    let n = r.u8()?;
                    let mut patterns = Vec::new();
                    for _ in 0..n {
                        patterns.push((r.str()?, r.u8()?));
                    }
                    TileKind::Banner { base, patterns }
                }
                TILE_POT => TileKind::Pot { contents: r.str()? },
                _ => break,
            };
            tiles.push(Tile { x, y, z, kind });
        }
    }
    Ok(ColumnOut {
        sections,
        biomes,
        has_sky,
        signs,
        tiles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_patches_lengths() {
        let mut w = Writer::default();
        let at = w.begin(s::UNLOAD);
        w.i32(-3).i32(7);
        w.end(at);
        w.empty(s::RESET);
        let f = w.into_vec();
        assert_eq!(f[..5], [4, 8, 0, 0, 0]);
        let m: Vec<_> = messages(&f).collect::<Result<_, _>>().unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].ty, s::UNLOAD);
        let mut r = Reader::new(m[0].body);
        assert_eq!((r.i32().unwrap(), r.i32().unwrap()), (-3, 7));
        assert_eq!(m[1].ty, s::RESET);
        assert!(m[1].body.is_empty());
    }

    #[test]
    fn truncated_frames_are_reported() {
        let mut w = Writer::default();
        let at = w.begin(s::POSE);
        w.f64(1.0);
        w.end(at);
        let mut f = w.into_vec();
        f.pop();
        let m: Vec<_> = messages(&f).collect();
        assert_eq!(m, vec![Err(Truncated)]);
    }

    #[test]
    fn looks_round_trip() {
        let mut pose = [0f32; 18];
        pose[3] = -10.5;
        let l = Looks {
            flags: look::BABY,
            armour: vec![(1, 301, 0, 0xa06540), (4, 310, 0, NO_COLOR)],
            pose: Some(pose),
        };
        let mut w = Writer::default();
        write_looks(&mut w, 7, &l);
        let f = w.into_vec();
        let m = messages(&f).next().unwrap().unwrap();
        assert_eq!(m.ty, s::LOOKS);
        assert_eq!(read_looks(&mut Reader::new(m.body)).unwrap(), (7, l));
    }

    #[test]
    fn particles_and_explosions_round_trip() {
        let p = Particle {
            id: 37,
            x: 1.5,
            y: 64.0,
            z: -3.25,
            ox: 0.1,
            oy: 0.2,
            oz: 0.3,
            speed: 0.5,
            count: 12,
            data: (1 << 4) | 2,
        };
        let mut w = Writer::default();
        write_particles(&mut w, &[p, p]);
        let e = Explosion {
            x: 0.5,
            y: 70.0,
            z: 0.5,
            radius: 4.0,
            blocks: vec![[1, -1, 0], [-128, 127, 3]],
        };
        write_explosion(&mut w, &e);
        let f = w.into_vec();
        let ms: Vec<_> = messages(&f).map(Result::unwrap).collect();
        assert_eq!(
            read_particles(&mut Reader::new(ms[0].body)).unwrap(),
            vec![p, p]
        );
        assert_eq!(read_explosion(&mut Reader::new(ms[1].body)).unwrap(), e);
    }

    fn states(f: impl Fn(usize) -> u16) -> Box<[u16; 4096]> {
        let mut s = Box::new([0u16; 4096]);
        for (i, v) in s.iter_mut().enumerate() {
            *v = f(i);
        }
        s
    }

    #[test]
    fn column_round_trip_every_palette_width() {
        let widths: [Box<[u16; 4096]>; 6] = [
            states(|_| 1 << 4),
            states(|i| if i % 2 == 0 { 1 << 4 } else { 2 << 4 }),
            states(|i| ((i % 3) as u16 + 1) << 4),
            states(|i| ((i % 16) as u16 + 1) << 4),
            states(|i| ((i % 200) as u16 + 1) << 4 | 3),
            states(|i| (i as u16 % 1000) + 1),
        ];
        let mut light = Box::new([0u8; 2048]);
        light[100] = 0xf3;
        let mut sky = Box::new([0xffu8; 2048]);
        sky[7] = 0x12;
        let mut sections: [Option<SectionIn<'_>>; 16] = [None; 16];
        for (i, st) in widths.iter().enumerate() {
            sections[i * 2] = Some(SectionIn {
                states: st,
                block_light: if i == 1 { Some(&light) } else { None },
                sky: match i {
                    0 => Sky::Full,
                    2 => Sky::Dark,
                    _ => Sky::Data(&sky),
                },
            });
        }
        let zero = Box::new([0u16; 4096]);
        sections[15] = Some(SectionIn {
            states: &zero,
            block_light: None,
            sky: Sky::Full,
        });
        let mut biomes = [0u8; 256];
        biomes[5] = 4;
        let signs = vec![SignText {
            x: 1,
            y: 70,
            z: 15,
            lines: ["a".into(), "".into(), "ü".into(), "x".repeat(70)],
        }];
        let tiles = vec![
            Tile {
                x: 0,
                y: 1,
                z: 2,
                kind: TileKind::Skull {
                    skull: 3,
                    rot: 17,
                    skin: "skins/ab.png".into(),
                },
            },
            Tile {
                x: 3,
                y: 4,
                z: 5,
                kind: TileKind::Banner {
                    base: 2,
                    patterns: vec![("bri".into(), 4)],
                },
            },
            Tile {
                x: 6,
                y: 7,
                z: 8,
                kind: TileKind::Pot {
                    contents: "fern".into(),
                },
            },
        ];
        let body = encode_column(&sections, Some(&biomes), true, &signs, &tiles);
        let out = decode_column(&body).unwrap();
        assert!(out.has_sky);
        assert_eq!(out.biomes, biomes);
        assert_eq!(out.sections.len(), 6, "the all-air section is left out");
        for (i, s) in out.sections.iter().enumerate() {
            assert_eq!(s.y as usize, i * 2);
            assert_eq!(*s.states, *widths[i]);
        }
        assert!(out.sections[0].sky_full && out.sections[0].sky.is_none());
        assert!(!out.sections[1].sky_full);
        assert_eq!(out.sections[1].block.as_deref(), Some(&*light));
        assert!(out.sections[2].sky.is_none() && !out.sections[2].sky_full);
        assert_eq!(out.sections[3].sky.as_deref(), Some(&*sky));
        assert_eq!(out.signs[0].lines[3].len(), 64);
        assert_eq!(out.signs[0].lines[2], "ü");
        let mut want = tiles.clone();
        want[0].kind = TileKind::Skull {
            skull: 3,
            rot: 1,
            skin: "skins/ab.png".into(),
        };
        assert_eq!(out.tiles, want);
    }

    #[test]
    fn utf16_cut_matches_js_slice() {
        assert_eq!(cut_utf16("abc", 2), "ab");
        assert_eq!(cut_utf16("a😀b", 2), "a");
        assert_eq!(cut_utf16("a😀b", 3), "a😀");
        assert_eq!(cut_bytes("aü", 2), "a");
    }
}
