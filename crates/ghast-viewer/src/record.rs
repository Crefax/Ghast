//! Recordings: what a page would have received, written to disk while the bot plays, and
//! played back later on the same page ([`crate::replay`]).
//!
//! A [`FeedRecorder`] is a stream of the viewer's hub that never backs up and needs no ACKs, so
//! the file holds exactly the live stream (HELLO, snapshot, columns nearest first, every tick
//! frame including the volatile part).
//!
//! ```text
//! file  <dir>/<name>.rec    gzip of records: u32 ms since the start, u32 length, the frame (LE)
//! meta  <dir>/<name>.json   {name, account, start, end?, bytes, raw, frames, note?}
//! ```
//!
//! The gzip stream is flushed every 10 s, so a recording under way (or cut off by a crash) plays
//! too; `end` is written when it stops. Names are the UTC start time: `YYYY-MM-DD_HH-MM[-n]`.

use std::fs::File;
use std::io::{BufWriter, Read as _, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use bytes::Bytes;
use flate2::Compression;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};

use crate::Viewer;
use crate::feed::now_ms;
use crate::hub::StreamId;
use crate::protocol::s;
use crate::stream::Out;

const FLUSH_EVERY: Duration = Duration::from_secs(10);
/// A runaway recording stops here (raw bytes).
const MAX_RAW: u64 = 400 << 20;

/// A recording's meta file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayMeta {
    pub name: String,
    pub account: String,
    /// Start, ms since the Unix epoch.
    pub start: f64,
    /// End (missing while recording or after a crash).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
    /// Compressed bytes.
    pub bytes: u64,
    /// Uncompressed record bytes.
    pub raw: u64,
    pub frames: u64,
    /// Why it was kept (e.g. a dash cam trigger).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Whether `name` is a recording name (`YYYY-MM-DD_HH-MM` with an optional `-n`).
#[must_use]
pub fn valid_recording_name(name: &str) -> bool {
    let b = name.as_bytes();
    if b.len() < 16 {
        return false;
    }
    let pat = b"dddd-dd-dd_dd-dd";
    for (c, p) in b.iter().zip(pat) {
        let ok = if *p == b'd' {
            c.is_ascii_digit()
        } else {
            c == p
        };
        if !ok {
            return false;
        }
    }
    match &b[16..] {
        [] => true,
        [b'-', rest @ ..] => {
            !rest.is_empty() && rest.len() <= 6 && rest.iter().all(u8::is_ascii_digit)
        }
        _ => false,
    }
}

/// `YYYY-MM-DD_HH-MM` (UTC) of a time in ms since the epoch.
#[must_use]
pub fn stamp(ms: f64) -> String {
    let secs = (ms / 1000.0).floor() as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // civil date from days since 1970-01-01 (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}_{:02}-{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// A free recording name in `dir` for a start at `ms`.
#[must_use]
pub fn recording_name(ms: f64, dir: &Path) -> String {
    let base = stamp(ms);
    let mut name = base.clone();
    let mut i = 2;
    while dir.join(format!("{name}.rec")).exists() {
        name = format!("{base}-{i}");
        i += 1;
    }
    name
}

/// Where the gzip output goes: memory until spilled, then the file.
struct Spill {
    held: Vec<u8>,
    file: Option<BufWriter<File>>,
    failed: bool,
    bytes: u64,
}

impl Write for Spill {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes += buf.len() as u64;
        match &mut self.file {
            Some(f) if !self.failed => {
                if f.write_all(buf).is_err() {
                    self.failed = true;
                }
            }
            Some(_) => {}
            None => self.held.extend_from_slice(buf),
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(f) = &mut self.file
            && f.flush().is_err()
        {
            self.failed = true;
        }
        Ok(())
    }
}

struct RecState {
    gz: Option<GzEncoder<Spill>>,
    meta: ReplayMeta,
    dir: PathBuf,
    t0: Instant,
    /// The bot was in a world in some frame: time runs from that frame on. Frames before it (a
    /// recording started before the join) are stamped 0, so 0:00 is the first in-game moment.
    started: bool,
    last_flush: Instant,
    closed: bool,
}

/// Whether a frame shows the bot in a world (a POSE, or a STATE with `inGame`).
fn shows_in_game(frame: &[u8]) -> bool {
    crate::protocol::messages(frame)
        .map_while(Result::ok)
        .any(|m| match m.ty {
            s::POSE => true,
            s::STATE => serde_json::from_slice::<serde_json::Value>(m.body)
                .is_ok_and(|v| v["inGame"].as_bool() == Some(true)),
            _ => false,
        })
}

impl RecState {
    fn write(&mut self, frame: &[u8]) -> bool {
        if self.closed || frame.is_empty() {
            return !self.closed;
        }
        let Some(gz) = self.gz.as_mut() else {
            return false;
        };
        if !self.started && shows_in_game(frame) {
            self.started = true;
            self.t0 = Instant::now();
            self.meta.start = now_ms();
        }
        let ms = if self.started {
            self.t0.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
        } else {
            0
        };
        let mut head = [0u8; 8];
        head[..4].copy_from_slice(&ms.to_le_bytes());
        head[4..].copy_from_slice(&(frame.len() as u32).to_le_bytes());
        let _ = gz.write_all(&head);
        let _ = gz.write_all(frame);
        self.meta.raw += frame.len() as u64 + 8;
        self.meta.frames += 1;
        self.meta.bytes = gz.get_ref().bytes;
        if gz.get_ref().failed {
            log::warn!("[record] {}: write failed, stopping", self.meta.name);
            self.close();
            return false;
        }
        if self.meta.raw > MAX_RAW {
            log::warn!("[record] {}: too big, stopping", self.meta.name);
            self.close();
            return false;
        }
        if self.last_flush.elapsed() >= FLUSH_EVERY && self.on_disk() {
            self.flush();
        }
        true
    }

    fn on_disk(&self) -> bool {
        self.gz.as_ref().is_some_and(|g| g.get_ref().file.is_some())
    }

    fn flush(&mut self) {
        self.last_flush = Instant::now();
        if let Some(gz) = self.gz.as_mut() {
            let _ = gz.flush();
            self.meta.bytes = gz.get_ref().bytes;
        }
        self.write_meta();
    }

    fn write_meta(&self) {
        if self.meta.name.is_empty() || !self.on_disk() && self.gz.is_some() {
            return;
        }
        if let Ok(j) = serde_json::to_vec(&self.meta) {
            let _ = std::fs::write(self.dir.join(format!("{}.json", self.meta.name)), j);
        }
    }

    /// Puts the recording on disk from its start; returns its name ('' on failure).
    fn spill(&mut self, note: Option<&str>) -> String {
        if self.closed {
            return String::new();
        }
        if self.on_disk() {
            return self.meta.name.clone();
        }
        let start = self.meta.start;
        let opened = std::fs::create_dir_all(&self.dir).and_then(|()| {
            let name = recording_name(start, &self.dir);
            File::create(self.dir.join(format!("{name}.rec"))).map(|f| (name, f))
        });
        let (name, f) = match opened {
            Ok(x) => x,
            Err(e) => {
                log::warn!("[record] {}: {e}", self.dir.display());
                self.close();
                return String::new();
            }
        };
        self.meta.name = name;
        if let Some(n) = note {
            self.meta.note = Some(n.to_string());
        }
        if let Some(gz) = self.gz.as_mut() {
            let sp = gz.get_mut();
            let mut w = BufWriter::new(f);
            if w.write_all(&sp.held).is_err() {
                sp.failed = true;
            }
            sp.held = Vec::new();
            sp.file = Some(w);
        }
        self.flush();
        self.meta.name.clone()
    }

    fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.meta.end = Some(now_ms());
        if let Some(gz) = self.gz.take() {
            match gz.finish() {
                Ok(mut sp) => {
                    self.meta.bytes = sp.bytes;
                    if let Some(mut f) = sp.file.take() {
                        let _ = f.flush();
                        let ok = f.into_inner().map(|f| f.sync_data()).is_ok();
                        let _ = ok;
                        if let Ok(j) = serde_json::to_vec(&self.meta) {
                            let _ = std::fs::write(
                                self.dir.join(format!("{}.json", self.meta.name)),
                                j,
                            );
                        }
                    }
                }
                Err(e) => log::warn!("[record] {}: {e}", self.meta.name),
            }
        }
    }
}

struct RecOut(Arc<Mutex<RecState>>);

fn lock(s: &Arc<Mutex<RecState>>) -> MutexGuard<'_, RecState> {
    s.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Out for RecOut {
    fn send(&mut self, frame: Bytes) -> bool {
        lock(&self.0).write(&frame)
    }
    fn buffered(&self) -> usize {
        0
    }
    fn is_open(&self) -> bool {
        !lock(&self.0).closed
    }
    fn terminate(&mut self) {
        lock(&self.0).close();
    }
}

/// A recording of a viewer's stream. Dropping it stops it (the file is finished).
pub struct FeedRecorder {
    state: Arc<Mutex<RecState>>,
    viewer: Viewer,
    id: Option<StreamId>,
}

impl std::fmt::Debug for FeedRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FeedRecorder")
            .field("meta", &self.meta())
            .finish()
    }
}

impl FeedRecorder {
    /// Starts recording `viewer`'s stream into `dir` (on disk at once).
    #[must_use]
    pub fn start(viewer: &Viewer, dir: impl Into<PathBuf>) -> Self {
        let r = Self::new(viewer, dir.into());
        r.spill(None);
        r
    }

    /// Starts recording in memory: no file until [`FeedRecorder::spill`] (the dash cam's segments).
    #[must_use]
    pub fn in_memory(viewer: &Viewer, dir: impl Into<PathBuf>) -> Self {
        Self::new(viewer, dir.into())
    }

    fn new(viewer: &Viewer, dir: PathBuf) -> Self {
        let state = Arc::new(Mutex::new(RecState {
            gz: Some(GzEncoder::new(
                Spill {
                    held: Vec::new(),
                    file: None,
                    failed: false,
                    bytes: 0,
                },
                Compression::new(6),
            )),
            meta: ReplayMeta {
                name: String::new(),
                account: viewer.config().account.clone(),
                start: now_ms(),
                end: None,
                bytes: 0,
                raw: 0,
                frames: 0,
                note: None,
            },
            dir,
            t0: Instant::now(),
            started: false,
            last_flush: Instant::now(),
            closed: false,
        }));
        let id = viewer
            .hub()
            .add(Box::new(RecOut(Arc::clone(&state))), "tp", u64::MAX, false);
        FeedRecorder {
            state,
            viewer: viewer.clone(),
            id: Some(id),
        }
    }

    /// Puts the recording on disk (from its start) and keeps writing there; `note` is shown in
    /// the list. Returns its name ('' when it could not be written).
    pub fn spill(&self, note: Option<&str>) -> String {
        lock(&self.state).spill(note)
    }

    /// Still recording.
    #[must_use]
    pub fn running(&self) -> bool {
        !lock(&self.state).closed
    }

    /// Only in memory so far.
    #[must_use]
    pub fn in_memory_only(&self) -> bool {
        !lock(&self.state).on_disk()
    }

    /// The meta as it is now.
    #[must_use]
    pub fn meta(&self) -> ReplayMeta {
        lock(&self.state).meta.clone()
    }

    /// Ends the recording (a recording only in memory is dropped) and returns its meta.
    pub fn stop(&mut self) -> ReplayMeta {
        if let Some(id) = self.id.take() {
            self.viewer.hub().remove(id, false);
        }
        let mut g = lock(&self.state);
        g.close();
        g.meta.clone()
    }
}

impl Drop for FeedRecorder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The recordings in `dir`, newest first.
#[must_use]
pub fn list_recordings(dir: &Path) -> Vec<ReplayMeta> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<ReplayMeta> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice::<ReplayMeta>(&std::fs::read(e.path()).ok()?).ok())
        .filter(|m| valid_recording_name(&m.name) && dir.join(format!("{}.rec", m.name)).exists())
        .collect();
    out.sort_by(|a, b| b.start.total_cmp(&a.start));
    out
}

/// Removes the oldest recordings until the folder holds at most `max_bytes` and none is older
/// than `max_age`; `keep` is never removed. Returns the removed names.
pub fn prune_recordings(
    dir: &Path,
    max_bytes: u64,
    max_age: Duration,
    keep: Option<&str>,
) -> Vec<String> {
    let mut list = list_recordings(dir);
    list.reverse(); // oldest first
    let mut total: u64 = list.iter().map(|m| m.bytes).sum();
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as f64;
    let cut = now - max_age.as_millis() as f64;
    let mut gone = Vec::new();
    for m in list {
        if Some(m.name.as_str()) == keep {
            continue;
        }
        if total <= max_bytes && m.start >= cut {
            continue;
        }
        for ext in ["rec", "json"] {
            let _ = std::fs::remove_file(dir.join(format!("{}.{ext}", m.name)));
        }
        total = total.saturating_sub(m.bytes);
        gone.push(m.name);
    }
    gone
}

/// The frames of a recording `(ms since start, frame)`; a cut-off tail (a recording under way)
/// is dropped.
pub fn read_recording(file: &Path) -> std::io::Result<Vec<(u32, Bytes)>> {
    let f = File::open(file)?;
    let mut gz = flate2::read::GzDecoder::new(std::io::BufReader::with_capacity(256 * 1024, f));
    let mut frames = Vec::new();
    let mut buf: Vec<u8> = Vec::with_capacity(1 << 20);
    let mut chunk = vec![0u8; 256 * 1024];
    let parse = |buf: &mut Vec<u8>, frames: &mut Vec<(u32, Bytes)>| {
        let mut pos = 0;
        while pos + 8 <= buf.len() {
            let t = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap_or_default());
            let len =
                u32::from_le_bytes(buf[pos + 4..pos + 8].try_into().unwrap_or_default()) as usize;
            if pos + 8 + len > buf.len() {
                break;
            }
            frames.push((t, Bytes::copy_from_slice(&buf[pos + 8..pos + 8 + len])));
            pos += 8 + len;
        }
        buf.drain(..pos);
    };
    loop {
        match gz.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() >= 1 << 20 {
                    parse(&mut buf, &mut frames);
                }
            }
            // a recording under way ends without the gzip trailer
            Err(_) => break,
        }
    }
    parse(&mut buf, &mut frames);
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_recording_name("2026-11-05_14-03"));
        assert!(valid_recording_name("2026-11-05_14-03-2"));
        assert!(!valid_recording_name("2026-11-05_14-03-"));
        assert!(!valid_recording_name("../etc/passwd"));
        assert!(!valid_recording_name("2026-11-05_14-03.rec"));
        assert_eq!(stamp(0.0), "1970-01-01_00-00");
        assert_eq!(stamp(1_762_351_380_000.0), "2025-11-05_14-03");
    }
}
