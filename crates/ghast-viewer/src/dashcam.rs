//! Dash cam: the last minutes are always in memory, and [`DashCam::clip`] (the app decides when,
//! e.g. a player came near) puts them on disk as a recording with a little more after it.
//!
//! A recording must start with HELLO, the snapshot and the columns, so the buffer is a chain of
//! overlapping in-memory [`FeedRecorder`]s: a new segment every `every`, each dropped `life` after
//! it began. The oldest live segment therefore reaches back at least `life − every`; at most
//! `life / every` run at once. A clip spills the oldest one to `<dir>/<name>.rec` and lets it run
//! `after` longer (another clip meanwhile extends it). While `active()` is false (not in a world,
//! the bot paused) no segment runs.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::Viewer;
use crate::record::{FeedRecorder, prune_recordings};

/// Dash cam options.
pub struct DashCamOptions {
    /// A new segment this often (default 2 min).
    pub every: Duration,
    /// A segment lives this long (default 4 min).
    pub life: Duration,
    /// A clip goes on this long after the trigger (default 1 min).
    pub after: Duration,
    /// Folder limits applied after each clip.
    pub max_bytes: u64,
    pub max_age: Duration,
    /// Buffer only while this holds.
    pub active: Box<dyn Fn() -> bool + Send + Sync>,
}

impl Default for DashCamOptions {
    fn default() -> Self {
        DashCamOptions {
            every: Duration::from_secs(120),
            life: Duration::from_secs(240),
            after: Duration::from_secs(60),
            max_bytes: 512 << 20,
            max_age: Duration::from_secs(14 * 86_400),
            active: Box::new(|| true),
        }
    }
}

struct Clip {
    rec: FeedRecorder,
    until: Instant,
}

struct State {
    segments: Vec<(FeedRecorder, Instant)>,
    clip: Option<Clip>,
    stopped: bool,
}

/// See the module docs. Needs a tokio runtime (a task checks the segments every second).
pub struct DashCam {
    viewer: Viewer,
    dir: PathBuf,
    opts: DashCamOptions,
    state: Mutex<State>,
}

impl DashCam {
    /// Starts buffering `viewer`'s stream.
    pub fn start(viewer: &Viewer, dir: impl Into<PathBuf>, opts: DashCamOptions) -> Arc<DashCam> {
        let cam = Arc::new(DashCam {
            viewer: viewer.clone(),
            dir: dir.into(),
            opts,
            state: Mutex::new(State {
                segments: Vec::new(),
                clip: None,
                stopped: false,
            }),
        });
        let weak = Arc::downgrade(&cam);
        tokio::spawn(async move {
            let mut t = tokio::time::interval(Duration::from_secs(1));
            loop {
                t.tick().await;
                let Some(cam) = weak.upgrade() else { break };
                if !cam.check(Instant::now()) {
                    break;
                }
            }
        });
        cam
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Segments buffering now.
    #[must_use]
    pub fn buffering(&self) -> usize {
        self.lock().segments.len()
    }

    /// One check (the task calls it every second); false once stopped.
    pub fn check(&self, now: Instant) -> bool {
        let mut ended = None;
        {
            let mut g = self.lock();
            if g.stopped {
                return false;
            }
            if g.clip
                .as_ref()
                .is_some_and(|c| now >= c.until || !c.rec.running())
            {
                ended = g.clip.take();
            }
            if !(self.opts.active)() {
                g.segments.clear();
            } else {
                g.segments
                    .retain(|(_, at)| now.duration_since(*at) < self.opts.life);
                let need = g
                    .segments
                    .last()
                    .is_none_or(|(_, at)| now.duration_since(*at) >= self.opts.every);
                if need {
                    let rec = FeedRecorder::in_memory(&self.viewer, &self.dir);
                    g.segments.push((rec, now));
                }
            }
        }
        if let Some(c) = ended {
            self.end_clip(c);
        }
        true
    }

    /// Keeps what the buffer holds (from its oldest segment on) plus `after`. Returns the
    /// recording's name, `None` when nothing was buffering or it could not be written.
    pub fn clip(&self, note: &str) -> Option<String> {
        let now = Instant::now();
        let mut g = self.lock();
        if let Some(c) = g.clip.as_mut()
            && c.rec.running()
        {
            c.until = c.until.max(now + self.opts.after);
            return Some(c.rec.meta().name);
        }
        if g.segments.is_empty() {
            // nothing buffered yet (buffering just began): record from now on
            if g.stopped || !(self.opts.active)() {
                return None;
            }
            g.segments
                .push((FeedRecorder::in_memory(&self.viewer, &self.dir), now));
        }
        let (rec, at) = g.segments.remove(0);
        let name = rec.spill(Some(note));
        if name.is_empty() {
            return None;
        }
        log::info!(
            "[dashcam] {name} kept ({note}): {} s before, {} s after",
            now.duration_since(at).as_secs(),
            self.opts.after.as_secs()
        );
        g.clip = Some(Clip {
            rec,
            until: now + self.opts.after,
        });
        Some(name)
    }

    /// Stops buffering and finishes a running clip.
    pub fn stop(&self) {
        let clip = {
            let mut g = self.lock();
            g.stopped = true;
            g.segments.clear();
            g.clip.take()
        };
        if let Some(c) = clip {
            self.end_clip(c);
        }
    }

    fn end_clip(&self, mut c: Clip) {
        let m = c.rec.stop();
        log::info!(
            "[dashcam] {} ended: {:.0} s, {:.2} MB",
            m.name,
            (m.end.unwrap_or(m.start) - m.start) / 1000.0,
            m.bytes as f64 / 1_048_576.0
        );
        let gone = prune_recordings(
            &self.dir,
            self.opts.max_bytes,
            self.opts.max_age,
            Some(&m.name),
        );
        if !gone.is_empty() {
            log::info!("[dashcam] removed old recordings: {}", gone.join(", "));
        }
    }
}

impl Drop for DashCam {
    fn drop(&mut self) {
        self.stop();
    }
}
