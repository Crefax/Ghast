//! Playing a recording to one page.
//!
//! The frames carry the bot's clock (POSE, TIME); they are rewritten onto the server's clock so
//! the page's interpolation and clock sync work as live: `t' = anchorWall + (t − start −
//! anchorRec) / speed`. Pausing stops the frames (the page freezes); seeking back resets the
//! page (RESET) and sends the state at the target at once, seeking ahead sends the gap's state
//! — both compacted by [`SeekCompactor`]. Recordings being played are read once and shared by
//! their pages (and kept 30 s after the last one left).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::mpsc;

use crate::feed::now_ms;
use crate::protocol::{PROTOCOL, Writer, s};
use crate::record::{ReplayMeta, read_recording};
use crate::stream::Out;

type Frames = Arc<Vec<(u32, Bytes)>>;

struct Loaded {
    key: (u64, Option<std::time::SystemTime>),
    frames: Frames,
    last: Instant,
}

static LOADED: Mutex<Option<HashMap<PathBuf, Loaded>>> = Mutex::new(None);
const KEEP: Duration = Duration::from_secs(30);

/// The frames of a recording, shared between pages (read again when the file changed: a
/// recording under way).
fn open_recording(file: &Path) -> std::io::Result<Frames> {
    let md = std::fs::metadata(file)?;
    let key = (md.len(), md.modified().ok());
    {
        let mut g = LOADED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let map = g.get_or_insert_with(HashMap::new);
        map.retain(|_, l| Arc::strong_count(&l.frames) > 1 || l.last.elapsed() < KEEP);
        if let Some(l) = map.get_mut(file)
            && l.key == key
        {
            l.last = Instant::now();
            return Ok(Arc::clone(&l.frames));
        }
    }
    let frames = Arc::new(read_recording(file)?);
    let mut g = LOADED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    g.get_or_insert_with(HashMap::new).insert(
        file.to_path_buf(),
        Loaded {
            key,
            frames: Arc::clone(&frames),
            last: Instant::now(),
        },
    );
    Ok(frames)
}

/// Message types a seek drops (one-off effects that would all play at once).
fn seek_drop(ty: u8) -> bool {
    matches!(
        ty,
        s::HELLO | s::PARTICLES | s::EXPLOSION | s::LIGHTNING | s::EVENTS | s::COLLECT | s::REPLAY
    )
}

const MOVE_BYTES: usize = 40;
const BLOCK_BYTES: usize = 11;

/// The page's lasting state after a run of recorded frames, in few bytes. Kept in order: every
/// message that holds state (columns, inventory, skins …). Compacted: block changes (the last
/// state per block; a newer column drops the older changes in it), entities spawned and gone
/// again (no SPAWN; their REMOVE only when the page may hold an older one), MOVES (each entity's
/// last), POSE and TIME (the last). Dropped: one-off effects. RESET and STATE are barriers: what
/// was gathered before them goes out first.
pub struct SeekCompactor {
    out: Vec<Bytes>,
    blocks: HashMap<(i32, i32), HashMap<u32, u16>>,
    block_order: Vec<(i32, i32)>,
    moves: HashMap<u32, Bytes>,
    move_order: Vec<u32>,
    spawned: HashMap<u32, usize>,
    removed: Vec<u32>,
    removed_set: HashSet<u32>,
    extras: HashMap<u32, Vec<usize>>,
    pose: Option<Bytes>,
    time: Option<Bytes>,
    page_empty: bool,
}

fn state_in_game(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body).map_or(true, |v| {
        v.get("inGame")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    })
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn le_i32(b: &[u8], at: usize) -> i32 {
    le_u32(b, at) as i32
}

impl SeekCompactor {
    /// `page_empty`: the page holds nothing yet (a RESET went first).
    #[must_use]
    pub fn new(page_empty: bool) -> Self {
        SeekCompactor {
            out: Vec::new(),
            blocks: HashMap::new(),
            block_order: Vec::new(),
            moves: HashMap::new(),
            move_order: Vec::new(),
            spawned: HashMap::new(),
            removed: Vec::new(),
            removed_set: HashSet::new(),
            extras: HashMap::new(),
            pose: None,
            time: None,
            page_empty,
        }
    }

    /// Adds a recorded frame.
    pub fn add(&mut self, frame: &Bytes) {
        let mut pos = 0usize;
        while pos + 5 <= frame.len() {
            let ty = frame[pos];
            let len = le_u32(frame, pos + 1) as usize;
            let body = pos + 5;
            let end = body + len;
            if end > frame.len() {
                break;
            }
            let msg = frame.slice(pos..end);
            pos = end;
            if seek_drop(ty) {
                continue;
            }
            if ty == s::RESET || ty == s::STATE {
                self.flush();
                self.out.push(msg);
                self.page_empty = ty == s::RESET || !state_in_game(&frame[body..end]);
                continue;
            }
            match ty {
                s::BLOCKS => {
                    let mut i = body + 2;
                    while i + BLOCK_BYTES <= end {
                        let (x, y, z) = (le_i32(frame, i), frame[i + 4], le_i32(frame, i + 5));
                        let st = u16::from_le_bytes([frame[i + 9], frame[i + 10]]);
                        let k = (x >> 4, z >> 4);
                        let col = self.blocks.entry(k).or_insert_with(|| {
                            self.block_order.push(k);
                            HashMap::new()
                        });
                        col.insert(
                            ((x & 15) as u32) | (((z & 15) as u32) << 4) | (u32::from(y) << 8),
                            st,
                        );
                        i += BLOCK_BYTES;
                    }
                }
                s::CHUNK | s::UNLOAD if len >= 8 => {
                    let k = (le_i32(frame, body), le_i32(frame, body + 4));
                    self.blocks.remove(&k);
                    self.out.push(msg);
                }
                s::SPAWN if len >= 4 => {
                    let id = le_u32(frame, body);
                    if let Some(&before) = self.spawned.get(&id) {
                        self.out[before] = Bytes::new();
                    }
                    if self.removed_set.remove(&id) {
                        self.removed.retain(|r| *r != id);
                    }
                    self.spawned.insert(id, self.out.len());
                    self.out.push(msg);
                }
                s::REMOVE => {
                    let mut i = body + 2;
                    while i + 4 <= end {
                        let id = le_u32(frame, i);
                        let at = self.spawned.remove(&id);
                        if let Some(at) = at {
                            self.out[at] = Bytes::new();
                            for j in self.extras.remove(&id).unwrap_or_default() {
                                self.out[j] = Bytes::new();
                            }
                        }
                        self.moves.remove(&id);
                        if (at.is_none() || !self.page_empty) && self.removed_set.insert(id) {
                            self.removed.push(id);
                        }
                        i += 4;
                    }
                }
                s::MOVES => {
                    let mut i = body + 2;
                    while i + MOVE_BYTES <= end {
                        let id = le_u32(frame, i);
                        if self
                            .moves
                            .insert(id, frame.slice(i..i + MOVE_BYTES))
                            .is_none()
                        {
                            self.move_order.push(id);
                        }
                        i += MOVE_BYTES;
                    }
                }
                s::EQUIP | s::SKIN | s::LOOKS if len >= 4 => {
                    let id = le_u32(frame, body);
                    if self.spawned.contains_key(&id) {
                        self.extras.entry(id).or_default().push(self.out.len());
                    }
                    self.out.push(msg);
                }
                s::POSE => self.pose = Some(msg),
                s::TIME => self.time = Some(msg),
                _ => self.out.push(msg),
            }
        }
    }

    /// The messages so far (POSE and TIME still on the recording's clock); empties the
    /// compactor.
    pub fn take(&mut self) -> Vec<Bytes> {
        self.flush();
        std::mem::take(&mut self.out)
            .into_iter()
            .filter(|m| !m.is_empty())
            .collect()
    }

    fn flush(&mut self) {
        let mut w = Writer::with_capacity(64);
        let list = |w: &mut Writer, ty: u8, items: &[&[u8]]| {
            for part in items.chunks(0xffff) {
                let at = w.begin(ty);
                w.u16(part.len() as u16);
                for b in part {
                    w.bytes(b);
                }
                w.end(at);
            }
        };
        if !self.removed.is_empty() {
            let ids: Vec<[u8; 4]> = self.removed.iter().map(|id| id.to_le_bytes()).collect();
            let refs: Vec<&[u8]> = ids.iter().map(|b| &b[..]).collect();
            list(&mut w, s::REMOVE, &refs);
        }
        if !self.blocks.is_empty() {
            let mut all: Vec<[u8; BLOCK_BYTES]> = Vec::new();
            for k in &self.block_order {
                let Some(col) = self.blocks.get(k) else {
                    continue;
                };
                let mut keys: Vec<_> = col.iter().collect();
                keys.sort_unstable_by_key(|(i, _)| **i);
                for (&i, &st) in keys {
                    let mut b = [0u8; BLOCK_BYTES];
                    b[..4].copy_from_slice(&(k.0 * 16 + (i & 15) as i32).to_le_bytes());
                    b[4] = (i >> 8) as u8;
                    b[5..9].copy_from_slice(&(k.1 * 16 + ((i >> 4) & 15) as i32).to_le_bytes());
                    b[9..].copy_from_slice(&st.to_le_bytes());
                    all.push(b);
                }
            }
            let refs: Vec<&[u8]> = all.iter().map(|b| &b[..]).collect();
            list(&mut w, s::BLOCKS, &refs);
        }
        if !self.moves.is_empty() {
            let refs: Vec<&[u8]> = self
                .move_order
                .iter()
                .filter_map(|id| self.moves.get(id).map(|b| &b[..]))
                .collect();
            list(&mut w, s::MOVES, &refs);
        }
        if !w.is_empty() {
            self.out.push(Bytes::from(w.into_vec()));
        }
        if let Some(t) = self.time.take() {
            self.out.push(t);
        }
        if let Some(p) = self.pose.take() {
            self.out.push(p);
        }
        self.blocks.clear();
        self.block_order.clear();
        self.moves.clear();
        self.move_order.clear();
        self.spawned.clear();
        self.removed.clear();
        self.removed_set.clear();
        self.extras.clear();
    }
}

/// Page → replay commands.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum ReplayCmd {
    /// Clock ping (answered with PONG).
    Ping(f64),
    /// Speed (0 = paused) and seek target in ms (negative: none).
    Control { speed: f64, seek: f64 },
    /// The page left.
    Close,
}

/// One page watching a recording.
pub struct ReplayStream {
    out: Box<dyn Out>,
    meta: ReplayMeta,
    file: PathBuf,
    mode: String,
    from: f64,
    speed: f64,
    frames: Frames,
    duration: f64,
    next: usize,
    cursor: f64,
    anchor_wall: f64,
    anchor_rec: f64,
    status_at: f64,
    bytes: u64,
    closed: bool,
}

impl ReplayStream {
    /// A replay of `file` (described by `meta`) to `out`, starting at `from` ms at `speed`.
    #[must_use]
    pub fn new(
        out: Box<dyn Out>,
        meta: ReplayMeta,
        file: PathBuf,
        mode: &str,
        from: f64,
        speed: f64,
    ) -> Self {
        ReplayStream {
            out,
            meta,
            file,
            mode: mode.to_string(),
            from: if from.is_finite() { from } else { 0.0 },
            speed: if speed.is_finite() {
                speed.clamp(0.0, 32.0)
            } else {
                1.0
            },
            frames: Arc::new(Vec::new()),
            duration: 0.0,
            next: 0,
            cursor: 0.0,
            anchor_wall: 0.0,
            anchor_rec: 0.0,
            status_at: 0.0,
            bytes: 0,
            closed: false,
        }
    }

    fn send(&mut self, b: Bytes) {
        if self.closed || b.is_empty() {
            return;
        }
        let n = b.len() as u64;
        if self.out.send(b) {
            self.bytes += n;
        } else {
            self.closed = true;
        }
    }

    fn anchor(&mut self, rec: f64) {
        self.anchor_wall = now_ms();
        self.anchor_rec = rec;
        self.cursor = rec;
    }

    fn rec_now(&self) -> f64 {
        if self.speed > 0.0 {
            self.anchor_rec + (now_ms() - self.anchor_wall) * self.speed
        } else {
            self.anchor_rec
        }
    }

    /// Plays until the page leaves; returns the bytes sent.
    pub async fn run(mut self, mut cmds: mpsc::UnboundedReceiver<ReplayCmd>) -> u64 {
        let file = self.file.clone();
        let frames = match tokio::task::spawn_blocking(move || open_recording(&file)).await {
            Ok(Ok(f)) => f,
            Ok(Err(e)) => {
                log::warn!("[viewer] replay {}: {e}", self.meta.name);
                self.out.terminate();
                return 0;
            }
            Err(_) => return 0,
        };
        self.duration = frames.last().map_or(0.0, |f| f64::from(f.0));
        self.frames = frames;
        let mut w = Writer::with_capacity(256);
        w.json(
            s::HELLO,
            &serde_json::json!({
                "protocol": PROTOCOL, "version": "1.8.8", "account": self.meta.account, "mode": self.mode,
                "viewDistance": 8, "now": now_ms(),
                "replay": { "name": self.meta.name, "start": self.meta.start, "duration": self.duration },
            }),
        );
        self.send(Bytes::from(w.into_vec()));
        self.anchor(0.0);
        let from = self.from.clamp(0.0, self.duration);
        self.seek(from).await;
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = tick.tick() => self.pump(),
                c = cmds.recv() => match c {
                    None | Some(ReplayCmd::Close) => break,
                    Some(ReplayCmd::Ping(t)) => {
                        let mut w = Writer::with_capacity(24);
                        let at = w.begin(s::PONG);
                        w.f64(t).f64(now_ms());
                        w.end(at);
                        self.send(Bytes::from(w.into_vec()));
                    }
                    Some(ReplayCmd::Control { speed, seek }) => {
                        let rec = self.rec_now();
                        if speed.is_finite() {
                            self.speed = speed.clamp(0.0, 32.0);
                            self.anchor(rec.min(self.duration));
                        }
                        if seek.is_finite() && seek >= 0.0 {
                            self.seek(seek.min(self.duration)).await;
                        }
                        self.status(true);
                    }
                },
            }
            if self.closed || !self.out.is_open() {
                break;
            }
        }
        self.bytes
    }

    async fn seek(&mut self, to: f64) {
        let back = to < self.cursor || self.next == 0;
        if back {
            let mut w = Writer::with_capacity(8);
            w.empty(s::RESET);
            self.send(Bytes::from(w.into_vec()));
            self.next = 0;
        }
        // compacting minutes of frames takes a while: off the async threads
        let frames = Arc::clone(&self.frames);
        let start = self.next;
        let (msgs, next) = tokio::task::spawn_blocking(move || {
            let mut c = SeekCompactor::new(back);
            let mut i = start;
            while i < frames.len() && f64::from(frames[i].0) <= to {
                c.add(&frames[i].1);
                i += 1;
            }
            (c.take(), i)
        })
        .await
        .unwrap_or((Vec::new(), start));
        self.next = next;
        self.anchor(to);
        let mut part: Vec<u8> = Vec::new();
        for m in msgs {
            let m = if m.first().is_some_and(|t| *t == s::POSE || *t == s::TIME) {
                match self.rewrite(&m) {
                    Some(b) => b,
                    None => continue,
                }
            } else {
                m
            };
            part.extend_from_slice(&m);
            if part.len() > 1 << 20 {
                self.send(Bytes::from(std::mem::take(&mut part)));
            }
        }
        if !part.is_empty() {
            self.send(Bytes::from(part));
        }
        self.status(true);
    }

    fn pump(&mut self) {
        if self.closed {
            return;
        }
        let t = self.duration.min(self.rec_now());
        let mut out: Vec<u8> = Vec::new();
        while self.next < self.frames.len() && f64::from(self.frames[self.next].0) <= t {
            let f = self.frames[self.next].1.clone();
            if let Some(b) = self.rewrite(&f) {
                out.extend_from_slice(&b);
            }
            self.next += 1;
        }
        self.cursor = t;
        if !out.is_empty() {
            self.send(Bytes::from(out));
        }
        if t >= self.duration && self.speed > 0.0 {
            // the end: stand still there
            self.anchor_rec = self.duration;
            self.speed = 0.0;
            self.status(true);
        }
        self.status(false);
    }

    /// The frame with its clock fields on the playback clock; `None` when only HELLO is left.
    fn rewrite(&self, frame: &Bytes) -> Option<Bytes> {
        let mut out = frame.to_vec();
        let speed = self.speed.max(1e-3);
        let map = |t: f64| self.anchor_wall + (t - self.meta.start - self.anchor_rec) / speed;
        let mut keep = false;
        let mut pos = 0usize;
        while pos + 5 <= out.len() {
            let ty = out[pos];
            let len = le_u32(&out, pos + 1) as usize;
            let body = pos + 5;
            if body + len > out.len() {
                break;
            }
            if ty == s::HELLO {
                // an unknown type: the page skips it
                out[pos..body + len].fill(0);
                out[pos] = 0xff;
            } else {
                keep = true;
                if (ty == s::POSE || ty == s::TIME) && len >= 8 {
                    let t = f64::from_le_bytes(out[body..body + 8].try_into().unwrap_or_default());
                    out[body..body + 8].copy_from_slice(&map(t).to_le_bytes());
                }
            }
            pos = body + len;
        }
        keep.then(|| Bytes::from(out))
    }

    fn status(&mut self, force: bool) {
        let t = now_ms();
        if !force && t - self.status_at < 250.0 {
            return;
        }
        self.status_at = t;
        let mut w = Writer::with_capacity(40);
        let at = w.begin(s::REPLAY);
        w.f64(self.duration.min(self.rec_now()))
            .f64(self.duration)
            .f32(self.speed as f32)
            .f64(self.meta.start);
        w.end(at);
        self.send(Bytes::from(w.into_vec()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::messages;

    fn msg(ty: u8, f: impl FnOnce(&mut Writer)) -> Vec<u8> {
        let mut w = Writer::default();
        let at = w.begin(ty);
        f(&mut w);
        w.end(at);
        w.into_vec()
    }

    #[test]
    fn compactor_keeps_the_last_state() {
        let mut c = SeekCompactor::new(true);
        let mut f = Vec::new();
        f.extend(msg(s::SPAWN, |w| {
            w.u32(5);
        }));
        f.extend(msg(s::SPAWN, |w| {
            w.u32(6);
        }));
        f.extend(msg(s::BLOCKS, |w| {
            w.u16(2)
                .i32(1)
                .u8(64)
                .i32(2)
                .u16(16)
                .i32(1)
                .u8(64)
                .i32(2)
                .u16(32);
        }));
        f.extend(msg(s::MOVES, |w| {
            w.u16(1)
                .u32(6)
                .f64(1.0)
                .f64(2.0)
                .f64(3.0)
                .f32(0.0)
                .f32(0.0)
                .f32(0.0);
        }));
        f.extend(msg(s::PARTICLES, |w| {
            w.u16(0);
        }));
        f.extend(msg(s::REMOVE, |w| {
            w.u16(1).u32(5);
        }));
        f.extend(msg(s::POSE, |w| {
            w.f64(1.0);
        }));
        f.extend(msg(s::POSE, |w| {
            w.f64(2.0);
        }));
        c.add(&Bytes::from(f));
        let out: Vec<u8> = c.take().concat();
        let types: Vec<u8> = messages(&out).map(|m| m.unwrap().ty).collect();
        // entity 5 came and went: nothing of it; one BLOCKS with the last state; one POSE
        assert_eq!(types, vec![s::SPAWN, s::BLOCKS, s::MOVES, s::POSE]);
        let m: Vec<_> = messages(&out).map(Result::unwrap).collect();
        assert_eq!(le_u32(m[0].body, 0), 6);
        assert_eq!(m[1].body[..2], [1, 0]);
        assert_eq!(u16::from_le_bytes([m[1].body[11], m[1].body[12]]), 32);
        assert_eq!(f64::from_le_bytes(m[3].body[..8].try_into().unwrap()), 2.0);
    }
}
