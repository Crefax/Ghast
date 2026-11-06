//! One page connection (or a recording) as the feed sees it.
//!
//! Columns go out nearest first under a flow-control window: the page ACKs the bytes it
//! received, at most `window` bytes of columns are unacknowledged, so a phone on a slow link is
//! never buried under a queue that delays the pose. The tick frame (shared with every stream)
//! always carries the reliable part (blocks, spawns, inventory …); its volatile part (pose,
//! moves) is dropped while the socket is backed up, and the page gets every entity's position
//! again once it caught up.

use std::collections::{HashSet, VecDeque};

use bytes::Bytes;

use crate::feed::{ColumnSlot, Feed, Notice};
use crate::protocol::{Writer, s};

/// Socket buffer above which the volatile part is dropped.
pub const SOFT_BUFFER: usize = 256 * 1024;
/// Socket buffer above which the page is cut off.
pub const HARD_BUFFER: usize = 8 * 1024 * 1024;
/// Columns being encoded for one stream at once.
const MAX_ENCODING: usize = 4;
/// Default unacknowledged column bytes in flight.
pub const DEFAULT_WINDOW: u64 = 384 * 1024;

/// Where a stream's frames go.
pub trait Out: Send {
    /// Queues a frame; false once closed.
    fn send(&mut self, frame: Bytes) -> bool;
    /// Bytes queued and not yet written to the socket.
    fn buffered(&self) -> usize;
    /// Still open.
    fn is_open(&self) -> bool;
    /// Closes at once (no close handshake: the page is too slow).
    fn terminate(&mut self);
}

struct Job {
    cx: i32,
    cz: i32,
    /// `None`: still encoding; `Some(None)`: not loaded.
    frame: Option<Option<Bytes>>,
    stale: bool,
}

/// One page (or recorder) stream.
pub struct PageStream {
    out: Box<dyn Out>,
    mode: String,
    loaded: HashSet<(i32, i32)>,
    encoding: HashSet<(i32, i32)>,
    pending: VecDeque<Job>,
    queue: VecDeque<(i32, i32)>,
    center: Option<(i32, i32)>,
    sent: u64,
    acked: u64,
    window: u64,
    dropped: bool,
    closed: bool,
    started: bool,
    /// Bytes sent (all frames).
    pub bytes: u64,
}

impl PageStream {
    /// A stream over `out`; `window` = `u64::MAX` disables flow control (recordings).
    #[must_use]
    pub fn new(out: Box<dyn Out>, mode: &str, window: u64) -> Self {
        PageStream {
            out,
            mode: mode.to_string(),
            loaded: HashSet::new(),
            encoding: HashSet::new(),
            pending: VecDeque::new(),
            queue: VecDeque::new(),
            center: None,
            sent: 0,
            acked: 0,
            window,
            dropped: false,
            closed: false,
            started: false,
            bytes: 0,
        }
    }

    /// Closed (by the page or for being too slow).
    #[must_use]
    pub fn closed(&self) -> bool {
        self.closed || !self.out.is_open()
    }

    /// Columns on the page, queued, unacknowledged bytes.
    #[must_use]
    pub fn stats(&self) -> (usize, usize, u64) {
        (
            self.loaded.len(),
            self.queue.len(),
            self.sent.saturating_sub(self.acked),
        )
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

    /// HELLO, the snapshot, then columns.
    pub fn start(&mut self, feed: &mut Feed) {
        self.started = true;
        let hello = feed.hello(&self.mode);
        self.send(hello);
        let snap = feed.snapshot();
        self.send(snap);
        self.pump(feed);
    }

    /// Whether [`PageStream::start`] ran.
    #[must_use]
    pub fn started(&self) -> bool {
        self.started
    }

    /// A notice from the feed.
    pub fn notice(&mut self, n: Notice, feed: &mut Feed) {
        match n {
            Notice::Reset => self.reset(),
            Notice::ColumnChanged(cx, cz) => self.column_changed(cx, cz, feed),
            Notice::ColumnUnloaded(cx, cz) => self.column_unloaded(cx, cz),
        }
    }

    /// The shared tick frame.
    pub fn tick(&mut self, reliable: Option<&Bytes>, volatile: Option<&Bytes>, feed: &mut Feed) {
        if self.closed() {
            self.closed = true;
            return;
        }
        let buffered = self.out.buffered();
        if buffered > HARD_BUFFER {
            log::info!("[viewer] page too slow, closing");
            self.closed = true;
            self.out.terminate();
            return;
        }
        let congested = buffered > SOFT_BUFFER;
        let mut parts: Vec<&Bytes> = Vec::with_capacity(3);
        if let Some(r) = reliable {
            parts.push(r);
        }
        let all;
        if congested {
            if volatile.is_some() {
                self.dropped = true;
            }
        } else {
            if self.dropped {
                self.dropped = false;
                if let Some(a) = feed.all_moves() {
                    all = a;
                    parts.push(&all);
                }
            }
            if let Some(v) = volatile {
                parts.push(v);
            }
        }
        match parts.len() {
            0 => {}
            1 => {
                let b = parts[0].clone();
                self.send(b);
            }
            _ => {
                let mut joined = Vec::with_capacity(parts.iter().map(|p| p.len()).sum());
                for p in parts {
                    joined.extend_from_slice(p);
                }
                self.send(Bytes::from(joined));
            }
        }
        self.pump(feed);
    }

    /// The bot changed or left: RESET, everything is sent again.
    pub fn reset(&mut self) {
        let mut w = Writer::with_capacity(8);
        w.empty(s::RESET);
        self.send(Bytes::from(w.into_vec()));
        self.loaded.clear();
        self.queue.clear();
        self.center = None;
        // columns of the old world still being encoded must not land in the new one
        for j in &mut self.pending {
            j.stale = true;
        }
    }

    fn in_range(&self, feed: &Feed, cx: i32, cz: i32) -> bool {
        let Some((bx, bz)) = feed.center() else {
            return false;
        };
        let vd = feed.config().view_distance;
        (cx - bx).abs() < vd && (cz - bz).abs() < vd
    }

    fn column_changed(&mut self, cx: i32, cz: i32, feed: &Feed) {
        if !self.in_range(feed, cx, cz) {
            return;
        }
        for j in &mut self.pending {
            if j.cx == cx && j.cz == cz {
                j.stale = true;
            }
        }
        self.loaded.remove(&(cx, cz));
        self.queue.retain(|&k| k != (cx, cz));
        self.queue.push_front((cx, cz));
    }

    fn column_unloaded(&mut self, cx: i32, cz: i32) {
        for j in &mut self.pending {
            if j.cx == cx && j.cz == cz {
                j.stale = true;
            }
        }
        if !self.loaded.remove(&(cx, cz)) {
            return;
        }
        let mut w = Writer::with_capacity(16);
        let at = w.begin(s::UNLOAD);
        w.i32(cx).i32(cz);
        w.end(at);
        self.send(Bytes::from(w.into_vec()));
    }

    /// The page acknowledged `bytes` received so far.
    pub fn ack(&mut self, bytes: f64, feed: &mut Feed) {
        if bytes.is_finite() {
            let v = bytes.max(0.0).min(self.sent as f64) as u64;
            self.acked = self.acked.max(v);
        }
        self.pump(feed);
    }

    /// Sends what can go now (after an ACK, a tick or a finished encoding).
    pub fn pump(&mut self, feed: &mut Feed) {
        self.follow(feed);
        loop {
            self.drain(feed);
            if !self.flush(feed) {
                break;
            }
        }
    }

    /// On a chunk change: unload what fell out of range, queue what came in (nearest first).
    fn follow(&mut self, feed: &Feed) {
        let Some((bx, bz)) = feed.center() else {
            return;
        };
        if self.center == Some((bx, bz)) {
            return;
        }
        self.center = Some((bx, bz));
        let mut w = Writer::with_capacity(64);
        let out: Vec<(i32, i32)> = self
            .loaded
            .iter()
            .copied()
            .filter(|&(x, z)| !self.in_range(feed, x, z))
            .collect();
        for (x, z) in out {
            self.loaded.remove(&(x, z));
            let at = w.begin(s::UNLOAD);
            w.i32(x).i32(z);
            w.end(at);
        }
        if !w.is_empty() {
            self.send(Bytes::from(w.into_vec()));
        }
        let vd = feed.config().view_distance;
        let mut want: Vec<(i32, i32)> = Vec::new();
        for dx in -vd + 1..vd {
            for dz in -vd + 1..vd {
                let k = (bx + dx, bz + dz);
                if !self.loaded.contains(&k) {
                    want.push(k);
                }
            }
        }
        want.sort_by_key(|&(x, z)| {
            let (dx, dz) = (i64::from(x - bx), i64::from(z - bz));
            dx * dx + dz * dz
        });
        self.queue = want.into();
    }

    fn drain(&mut self, feed: &mut Feed) {
        while !self.closed
            && self.encoding.len() < MAX_ENCODING
            && self.sent.saturating_sub(self.acked) < self.window
            && self.out.buffered() < SOFT_BUFFER
        {
            let Some((cx, cz)) = self.queue.pop_front() else {
                break;
            };
            let k = (cx, cz);
            if self.loaded.contains(&k)
                || self.encoding.contains(&k)
                || !self.in_range(feed, cx, cz)
            {
                continue;
            }
            if !feed.has_column(cx, cz) {
                continue; // queued again once the server sends it
            }
            self.encoding.insert(k);
            let frame = match feed.column(cx, cz) {
                ColumnSlot::Ready(f) => Some(Some(f)),
                ColumnSlot::Missing => Some(None),
                ColumnSlot::Pending => None,
            };
            self.pending.push_back(Job {
                cx,
                cz,
                frame,
                stale: false,
            });
        }
    }

    /// Sends the finished columns at the head of `pending`; returns whether any job ended.
    fn flush(&mut self, feed: &mut Feed) -> bool {
        let mut any = false;
        while let Some(head) = self.pending.front_mut() {
            if !head.stale && head.frame.is_none() {
                match feed.column(head.cx, head.cz) {
                    ColumnSlot::Ready(f) => head.frame = Some(Some(f)),
                    ColumnSlot::Missing => head.frame = Some(None),
                    ColumnSlot::Pending => break,
                }
            }
            let Some(job) = self.pending.pop_front() else {
                break;
            };
            any = true;
            let k = (job.cx, job.cz);
            self.encoding.remove(&k);
            if self.closed {
                continue;
            }
            let frame = job.frame.flatten();
            let current = frame
                .as_ref()
                .is_none_or(|f| feed.is_current(job.cx, job.cz, f));
            if job.stale || !current {
                // asked for again (drain skipped it while this job ran)
                if self.in_range(feed, job.cx, job.cz)
                    && !self.loaded.contains(&k)
                    && !self.queue.contains(&k)
                {
                    self.queue.push_front(k);
                }
                continue;
            }
            if let Some(f) = frame
                && self.in_range(feed, job.cx, job.cz)
                && !self.loaded.contains(&k)
            {
                self.sent += f.len() as u64;
                self.send(f);
                self.loaded.insert(k);
            }
        }
        any
    }
}
