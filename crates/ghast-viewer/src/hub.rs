//! The hub: one feed per bot and the streams (pages, recordings) that read it.
//!
//! The bot's tick thread calls [`Hub::tick`] (through [`crate::Watched`]); page sockets call
//! [`Hub::ack`]; the column worker calls `Hub::finish`. Everything shares one mutex, which is
//! uncontended in practice (one bot thread, a few page tasks). With no stream attached,
//! [`Hub::tick`] only applies the few lasting-state packets and returns: no frames, no copies.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak, mpsc};

use bytes::Bytes;

use crate::feed::{BotView, ColumnJob, Feed, FeedConfig};
use crate::skins::SkinStore;
use crate::stream::{Out, PageStream};

/// Id of a stream in the hub.
pub type StreamId = u64;

pub(crate) struct HubState {
    pub feed: Feed,
    streams: Vec<(StreamId, PageStream)>,
    next: StreamId,
}

/// See the module docs.
pub struct Hub {
    state: Mutex<HubState>,
    active: AtomicBool,
    /// Pages watching (recordings are not counted).
    pub(crate) viewers: AtomicUsize,
    worker: Mutex<Option<mpsc::Sender<ColumnJob>>>,
    me: Weak<Hub>,
    /// Encode columns on the calling thread (tests).
    inline: bool,
}

impl Hub {
    /// A hub for one bot.
    pub(crate) fn new(cfg: FeedConfig, skins: Arc<SkinStore>, inline: bool) -> Arc<Hub> {
        Arc::new_cyclic(|me| Hub {
            state: Mutex::new(HubState {
                feed: Feed::new(cfg, skins),
                streams: Vec::new(),
                next: 1,
            }),
            active: AtomicBool::new(false),
            viewers: AtomicUsize::new(0),
            worker: Mutex::new(None),
            me: me.clone(),
            inline,
        })
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, HubState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether any stream is attached (the feed runs).
    #[must_use]
    pub fn active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    /// Attaches a stream; it gets HELLO and the snapshot at once.
    pub fn add(&self, out: Box<dyn Out>, mode: &str, window: u64, viewer: bool) -> StreamId {
        let mut g = self.lock();
        let id = g.next;
        g.next += 1;
        let mut st = PageStream::new(out, mode, window);
        st.start(&mut g.feed);
        g.streams.push((id, st));
        self.active.store(true, Ordering::Relaxed);
        if viewer {
            self.viewers.fetch_add(1, Ordering::Relaxed);
        }
        drop(g);
        id
    }

    /// Detaches a stream; returns its byte count. The feed goes idle with the last one.
    pub fn remove(&self, id: StreamId, viewer: bool) -> u64 {
        let mut g = self.lock();
        let mut bytes = 0;
        if let Some(i) = g.streams.iter().position(|(s, _)| *s == id) {
            bytes = g.streams.remove(i).1.bytes;
            if viewer {
                self.viewers.fetch_sub(1, Ordering::Relaxed);
            }
        }
        if g.streams.is_empty() {
            self.active.store(false, Ordering::Relaxed);
            g.feed.idle();
        }
        bytes
    }

    /// A page acknowledged bytes.
    pub fn ack(&self, id: StreamId, bytes: f64) {
        let mut g = self.lock();
        let HubState { feed, streams, .. } = &mut *g;
        if let Some((_, st)) = streams.iter_mut().find(|(s, _)| *s == id) {
            st.ack(bytes, feed);
        }
    }

    /// One client tick of the bot (called by [`crate::Watched`] with the tick context).
    pub fn tick(&self, view: &dyn BotView) {
        if !self.active() {
            let mut g = self.lock();
            g.feed.observe(view);
            return;
        }
        let mut g = self.lock();
        let HubState { feed, streams, .. } = &mut *g;
        let frames = feed.tick(view);
        for (_, st) in streams.iter_mut() {
            for n in &frames.notices {
                st.notice(*n, feed);
            }
            st.tick(frames.reliable.as_ref(), frames.volatile.as_ref(), feed);
        }
        let jobs = feed.take_jobs(view);
        drop(g);
        self.dispatch(jobs);
    }

    /// The session ended (or the bot left the world): pages show `message`.
    pub fn session_ended(&self, message: &str) {
        let mut g = self.lock();
        let HubState { feed, streams, .. } = &mut *g;
        let frames = feed.session_ended(message);
        for (_, st) in streams.iter_mut() {
            for n in &frames.notices {
                st.notice(*n, feed);
            }
            st.tick(frames.reliable.as_ref(), None, feed);
        }
    }

    /// Changes the status line (and the "not in game" message).
    pub fn set_status(&self, status: &str, message: &str) {
        let mut g = self.lock();
        let HubState { feed, streams, .. } = &mut *g;
        feed.set_status(status, message);
        if !feed.in_game() {
            // no ticks run: send it now
            let r = feed.take_reliable();
            for (_, st) in streams.iter_mut() {
                st.tick(r.as_ref(), None, feed);
            }
        }
    }

    fn dispatch(&self, jobs: Vec<ColumnJob>) {
        if jobs.is_empty() {
            return;
        }
        if self.inline {
            for j in jobs {
                let frame = j.encode();
                self.finish(j.cx, j.cz, j.version, frame);
            }
            return;
        }
        let mut w = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut jobs = jobs.into_iter();
        while let Some(j) = jobs.next() {
            let sent = match w.as_ref() {
                Some(tx) => tx.send(j).map_err(|e| e.0),
                None => Err(j),
            };
            if let Err(j) = sent {
                // (re)start the worker and hand it the job
                let (tx, rx) = mpsc::channel::<ColumnJob>();
                let me = self.me.clone();
                let ok = std::thread::Builder::new()
                    .name("viewer-columns".into())
                    .spawn(move || {
                        while let Ok(j) = rx.recv() {
                            let frame = j.encode();
                            let Some(hub) = me.upgrade() else { return };
                            hub.finish(j.cx, j.cz, j.version, frame);
                        }
                    })
                    .is_ok();
                if !ok {
                    // no thread: encode here
                    let frame = j.encode();
                    drop(w);
                    self.finish(j.cx, j.cz, j.version, frame);
                    for j in jobs {
                        let frame = j.encode();
                        self.finish(j.cx, j.cz, j.version, frame);
                    }
                    return;
                }
                let _ = tx.send(j);
                *w = Some(tx);
            }
        }
    }

    /// An encoding finished.
    pub(crate) fn finish(&self, cx: i32, cz: i32, version: u32, frame: Bytes) {
        let mut g = self.lock();
        let HubState { feed, streams, .. } = &mut *g;
        if feed.finish(cx, cz, version, frame) {
            for (_, st) in streams.iter_mut() {
                st.pump(feed);
            }
        }
    }

    /// Streams attached (pages and recordings).
    #[must_use]
    pub fn streams(&self) -> usize {
        self.lock().streams.len()
    }

    /// Whether the bot is in a world.
    #[must_use]
    pub fn in_game(&self) -> bool {
        self.lock().feed.session_in_game()
    }
}
