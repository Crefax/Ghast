//! HTTP + WebSocket server of a viewer.
//!
//! ```text
//! GET /                 the page (?mode=fp first person | tp third person | free; ?replay=<name>)
//! GET /status           {running, port, viewers, inGame, account}
//! GET /v2/<file>        script, worker, block data and texture pack (content-hashed names)
//! GET /skins/<sha>.png  player skins and capes the feed downloaded
//! GET /replays          the recordings as JSON, newest first
//! WS  /ws               the live stream; ?replay=<name>&t=<ms>&speed=<x>: a recording instead
//! ```
//!
//! The page may only send clock pings, flow-control ACKs and (on a recording) playback
//! controls; nothing reaches the Minecraft connection.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::sync::{Notify, mpsc};

use crate::Viewer;
use crate::assets::Assets;
use crate::protocol::{Reader, Writer, c, s};
use crate::replay::{ReplayCmd, ReplayStream};
use crate::skins::SkinStore;
use crate::stream::{DEFAULT_WINDOW, Out};

/// The running server of a viewer.
#[derive(Default)]
pub(crate) struct Control {
    state: Mutex<ControlState>,
    replays: AtomicUsize,
    start_lock: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct ControlState {
    addr: Option<SocketAddr>,
    shutdown: Option<Arc<Notify>>,
    done: Option<tokio::task::JoinHandle<()>>,
}

impl Control {
    fn lock(&self) -> std::sync::MutexGuard<'_, ControlState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn port(&self) -> Option<u16> {
        self.lock().addr.map(|a| a.port())
    }

    pub(crate) fn replays(&self) -> usize {
        self.replays.load(Ordering::Relaxed)
    }

    pub(crate) async fn stop(&self) {
        let (n, done) = {
            let mut g = self.lock();
            g.addr = None;
            (g.shutdown.take(), g.done.take())
        };
        if let Some(n) = n {
            n.notify_waiters();
            n.notify_one();
        }
        if let Some(d) = done {
            let _ = d.await;
        }
    }
}

static ASSETS: Mutex<Option<Arc<Assets>>> = Mutex::new(None);

async fn assets(v: &Viewer) -> std::io::Result<Arc<Assets>> {
    if let Some(a) = ASSETS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
    {
        return Ok(a);
    }
    let cfg = v.config().clone();
    let a = tokio::task::spawn_blocking(move || {
        Assets::load(&cfg.cache_dir, cfg.client_jar.as_deref())
    })
    .await
    .map_err(std::io::Error::other)??;
    let a = Arc::new(a);
    *ASSETS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&a));
    Ok(a)
}

#[derive(Clone)]
struct App {
    viewer: Viewer,
    assets: Arc<Assets>,
    /// Shutdown signal for the WebSocket tasks.
    quit: Arc<Notify>,
}

pub(crate) async fn start(v: &Viewer) -> std::io::Result<SocketAddr> {
    let ctl = v.control();
    let _guard = ctl.start_lock.lock().await;
    if let Some(a) = ctl.lock().addr {
        return Ok(a);
    }
    let assets = assets(v).await?;
    let cfg = v.config();
    let listener = tokio::net::TcpListener::bind((cfg.host, cfg.port)).await?;
    let addr = listener.local_addr()?;
    let quit = Arc::new(Notify::new());
    let app = App {
        viewer: v.clone(),
        assets,
        quit: Arc::clone(&quit),
    };
    let router = Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/status", get(status))
        .route("/replays", get(replays))
        .route("/v2/{file}", get(v2))
        .route("/skins/{file}", get(skin))
        .route("/ws", get(ws))
        .fallback(|| async { plain(StatusCode::NOT_FOUND, "not found") })
        .with_state(app);
    let q = Arc::clone(&quit);
    let server = tokio::spawn(async move {
        let r = axum::serve(listener, router)
            .with_graceful_shutdown(async move { q.notified().await })
            .await;
        if let Err(e) = r {
            log::warn!("[viewer] server: {e}");
        }
    });
    // the idle stop: no page for `idle`
    let weak = Arc::downgrade(&v.inner);
    let idle = cfg.idle;
    let q2 = Arc::clone(&quit);
    let done = tokio::spawn(async move {
        let mut quiet = Duration::ZERO;
        let step = Duration::from_millis(500);
        loop {
            tokio::select! {
                () = tokio::time::sleep(step) => {}
                () = q2.notified() => break,
            }
            let Some(inner) = weak.upgrade() else { break };
            let v = Viewer { inner };
            if v.status().viewers > 0 {
                quiet = Duration::ZERO;
                continue;
            }
            quiet += step;
            if quiet >= idle {
                log::info!("[viewer] stopped (nobody watching)");
                let mut g = v.control().lock();
                g.addr = None;
                g.shutdown = None;
                g.done = None;
                drop(g);
                q2.notify_waiters();
                break;
            }
        }
        q2.notify_waiters();
        let _ = server.await;
    });
    {
        let mut g = ctl.lock();
        g.addr = Some(addr);
        g.shutdown = Some(quit);
        g.done = Some(done);
    }
    log::info!(
        "[viewer] on http://{addr}/ (view {} chunks)",
        cfg.view_distance
    );
    Ok(addr)
}

// ---- access ------------------------------------------------------------------------------------

const COOKIE: &str = "ghast_viewer_token";

/// Whether the request carries the token; the cookie to set when it came as a query parameter.
#[allow(clippy::result_large_err)]
fn authorized(
    app: &App,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Result<bool, Response> {
    let Some(token) = app.viewer.config().token.as_deref() else {
        return Ok(false);
    };
    let eq = |a: &str| constant_eq(a.as_bytes(), token.as_bytes());
    if let Some(t) = query.get("token")
        && eq(t)
    {
        return Ok(true);
    }
    if let Some(h) = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        && h.strip_prefix("Bearer ").is_some_and(eq)
    {
        return Ok(false);
    }
    for c in headers.get_all(header::COOKIE) {
        let Ok(c) = c.to_str() else { continue };
        for part in c.split(';') {
            if let Some(v) = part
                .trim()
                .strip_prefix(COOKIE)
                .and_then(|r| r.strip_prefix('='))
                && eq(v)
            {
                return Ok(false);
            }
        }
    }
    Err(plain(StatusCode::UNAUTHORIZED, "token required"))
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

fn plain(code: StatusCode, text: &'static str) -> Response {
    (
        code,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        text,
    )
        .into_response()
}

fn no_store(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    r
}

// ---- routes ------------------------------------------------------------------------------------

async fn index(
    State(app): State<App>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let set_cookie = match authorized(&app, &headers, &q) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let mut r = no_store(
        (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            app.assets.page().to_string(),
        )
            .into_response(),
    );
    if set_cookie
        && let Some(t) = &app.viewer.config().token
        && let Ok(v) =
            HeaderValue::from_str(&format!("{COOKIE}={t}; Path=/; HttpOnly; SameSite=Strict"))
    {
        r.headers_mut().insert(header::SET_COOKIE, v);
    }
    r
}

async fn status(
    State(app): State<App>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = authorized(&app, &headers, &q) {
        return r;
    }
    let st = app.viewer.status();
    let body = serde_json::json!({
        "running": st.running, "port": st.port, "viewers": st.viewers, "inGame": st.in_game,
        "account": app.viewer.config().account,
    });
    no_store(
        (
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            body.to_string(),
        )
            .into_response(),
    )
}

async fn replays(
    State(app): State<App>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = authorized(&app, &headers, &q) {
        return r;
    }
    let list = match &app.viewer.config().replay_dir {
        Some(d) => {
            let d = d.clone();
            tokio::task::spawn_blocking(move || crate::record::list_recordings(&d))
                .await
                .unwrap_or_default()
        }
        None => Vec::new(),
    };
    let out: Vec<serde_json::Value> = list
        .iter()
        .map(|m| {
            let mut j = serde_json::json!({ "name": m.name, "start": m.start, "end": m.end, "bytes": m.bytes });
            if let Some(n) = &m.note {
                j["note"] = serde_json::Value::String(n.clone());
            }
            j
        })
        .collect();
    no_store(
        (
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            serde_json::Value::Array(out).to_string(),
        )
            .into_response(),
    )
}

async fn v2(
    State(app): State<App>,
    Path(file): Path<String>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = authorized(&app, &headers, &q) {
        return r;
    }
    let Some(a) = app.assets.get(&file) else {
        return plain(StatusCode::NOT_FOUND, "not found");
    };
    let gz_ok = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| h.split(',').any(|p| p.trim().starts_with("gzip")));
    let (body, gz) = match (&a.gzip, gz_ok) {
        (Some(g), true) => (g.clone(), true),
        _ => (a.body.clone(), false),
    };
    let mut r = Response::new(Body::from(body));
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(a.content_type),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=604800, immutable"),
    );
    h.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if gz {
        h.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    }
    r
}

async fn skin(
    State(app): State<App>,
    Path(file): Path<String>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = authorized(&app, &headers, &q) {
        return r;
    }
    let Some(png) = SkinStore::global().file(&format!("skins/{file}")) else {
        return plain(StatusCode::NOT_FOUND, "not found");
    };
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=604800, immutable"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        Bytes::copy_from_slice(&png),
    )
        .into_response()
}

async fn ws(
    State(app): State<App>,
    upgrade: WebSocketUpgrade,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = authorized(&app, &headers, &q) {
        return r;
    }
    let mode = q.get("mode").cloned().unwrap_or_else(|| "fp".into());
    let upgrade = upgrade.max_message_size(1024).max_frame_size(1024);
    if let Some(name) = q.get("replay").cloned() {
        let from = q
            .get("t")
            .and_then(|t| t.parse::<f64>().ok())
            .unwrap_or(0.0);
        let speed = q
            .get("speed")
            .and_then(|t| t.parse::<f64>().ok())
            .unwrap_or(1.0);
        return upgrade.on_upgrade(move |sock| replay_socket(app, sock, name, mode, from, speed));
    }
    upgrade.on_upgrade(move |sock| live_socket(app, sock, mode))
}

// ---- sockets -----------------------------------------------------------------------------------

enum WsCmd {
    Frame(Bytes),
    Close(u16, &'static str),
    Terminate,
}

/// A page socket as a stream output: frames go to the writer task, which counts what is queued.
struct WsOut {
    tx: mpsc::UnboundedSender<WsCmd>,
    queued: Arc<AtomicUsize>,
    open: Arc<AtomicBool>,
}

impl Out for WsOut {
    fn send(&mut self, frame: Bytes) -> bool {
        if !self.open.load(Ordering::Relaxed) {
            return false;
        }
        self.queued.fetch_add(frame.len(), Ordering::Relaxed);
        self.tx.send(WsCmd::Frame(frame)).is_ok()
    }
    fn buffered(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }
    fn is_open(&self) -> bool {
        self.open.load(Ordering::Relaxed)
    }
    fn terminate(&mut self) {
        self.open.store(false, Ordering::Relaxed);
        let _ = self.tx.send(WsCmd::Terminate);
    }
}

fn ws_out() -> (WsOut, mpsc::UnboundedReceiver<WsCmd>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (
        WsOut {
            tx,
            queued: Arc::new(AtomicUsize::new(0)),
            open: Arc::new(AtomicBool::new(true)),
        },
        rx,
    )
}

/// Writes queued frames to the socket until closed.
async fn writer(
    mut sink: futures_util::stream::SplitSink<WebSocket, Message>,
    mut rx: mpsc::UnboundedReceiver<WsCmd>,
    queued: Arc<AtomicUsize>,
    open: Arc<AtomicBool>,
) {
    while let Some(cmd) = rx.recv().await {
        match cmd {
            WsCmd::Frame(b) => {
                let n = b.len();
                let ok = sink.send(Message::Binary(b)).await.is_ok();
                queued.fetch_sub(n, Ordering::Relaxed);
                if !ok {
                    break;
                }
            }
            WsCmd::Close(code, reason) => {
                let _ = sink
                    .send(Message::Close(Some(CloseFrame {
                        code,
                        reason: reason.into(),
                    })))
                    .await;
                break;
            }
            WsCmd::Terminate => break,
        }
    }
    open.store(false, Ordering::Relaxed);
    let _ = sink.close().await;
}

fn pong(client: f64) -> Bytes {
    let mut w = Writer::with_capacity(24);
    let at = w.begin(s::PONG);
    w.f64(client).f64(crate::now_ms());
    w.end(at);
    Bytes::from(w.into_vec())
}

async fn live_socket(app: App, sock: WebSocket, mode: String) {
    let (sink, mut stream) = sock.split();
    let (out, rx) = ws_out();
    let (tx, queued, open) = (
        out.tx.clone(),
        Arc::clone(&out.queued),
        Arc::clone(&out.open),
    );
    let write = tokio::spawn(writer(sink, rx, Arc::clone(&queued), Arc::clone(&open)));
    let hub = Arc::clone(app.viewer.hub());
    let id = hub.add(Box::new(out), &mode, DEFAULT_WINDOW, true);
    log::info!(
        "[viewer] page connected ({} watching)",
        app.viewer.status().viewers
    );
    let quit = Arc::clone(&app.quit);
    loop {
        let msg = tokio::select! {
            m = stream.next() => m,
            () = quit.notified() => {
                let _ = tx.send(WsCmd::Close(1001, "server stopping"));
                break;
            }
        };
        let Some(Ok(msg)) = msg else { break };
        match msg {
            Message::Binary(b) => {
                if b.is_empty() || b.len() > 64 {
                    let _ = tx.send(WsCmd::Close(1003, "unexpected message"));
                    break;
                }
                let mut r = Reader::new(&b);
                let ty = r.u8().unwrap_or(0);
                match (ty, r.f64()) {
                    (c::PING, Ok(t)) => {
                        let p = pong(t);
                        queued.fetch_add(p.len(), Ordering::Relaxed);
                        let _ = tx.send(WsCmd::Frame(p));
                    }
                    (c::ACK, Ok(n)) => hub.ack(id, n),
                    _ => {
                        let _ = tx.send(WsCmd::Close(1003, "unexpected message"));
                        break;
                    }
                }
            }
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Text(_) => {
                let _ = tx.send(WsCmd::Close(1003, "unexpected message"));
                break;
            }
        }
        if !open.load(Ordering::Relaxed) {
            break;
        }
    }
    open.store(false, Ordering::Relaxed);
    let bytes = hub.remove(id, true);
    let _ = tx.send(WsCmd::Terminate);
    let _ = write.await;
    log::info!(
        "[viewer] page left ({} watching, {} KB sent)",
        app.viewer.status().viewers,
        bytes / 1024
    );
}

async fn replay_socket(
    app: App,
    sock: WebSocket,
    name: String,
    mode: String,
    from: f64,
    speed: f64,
) {
    let (sink, mut stream) = sock.split();
    let (out, rx) = ws_out();
    let (tx, queued, open) = (
        out.tx.clone(),
        Arc::clone(&out.queued),
        Arc::clone(&out.open),
    );
    let write = tokio::spawn(writer(sink, rx, Arc::clone(&queued), Arc::clone(&open)));
    let ctl = app.viewer.control();
    let meta = match &app.viewer.config().replay_dir {
        Some(dir) if crate::record::valid_recording_name(&name) => {
            let d = dir.clone();
            let n = name.clone();
            tokio::task::spawn_blocking(move || {
                crate::record::list_recordings(&d)
                    .into_iter()
                    .find(|m| m.name == n)
                    .map(|m| (m, d))
            })
            .await
            .ok()
            .flatten()
        }
        _ => None,
    };
    let Some((meta, dir)) = meta else {
        let _ = tx.send(WsCmd::Close(4004, "recording not found"));
        let _ = write.await;
        return;
    };
    ctl.replays.fetch_add(1, Ordering::Relaxed);
    log::info!("[viewer] replay {name} opened");
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<ReplayCmd>();
    let file = dir.join(format!("{name}.rec"));
    let player = ReplayStream::new(Box::new(out), meta, file, &mode, from, speed);
    let play = tokio::spawn(player.run(cmd_rx));
    let quit = Arc::clone(&app.quit);
    loop {
        let msg = tokio::select! {
            m = stream.next() => m,
            () = quit.notified() => break,
        };
        let Some(Ok(msg)) = msg else { break };
        match msg {
            Message::Binary(b) => {
                if b.is_empty() || b.len() > 64 {
                    let _ = tx.send(WsCmd::Close(1003, "unexpected message"));
                    break;
                }
                let mut r = Reader::new(&b);
                match r.u8().unwrap_or(0) {
                    c::PING => {
                        if let Ok(t) = r.f64() {
                            let _ = cmd_tx.send(ReplayCmd::Ping(t));
                        }
                    }
                    c::ACK => {}
                    c::REPLAY => {
                        if let (Ok(sp), Ok(seek)) = (r.f64(), r.f64()) {
                            let _ = cmd_tx.send(ReplayCmd::Control { speed: sp, seek });
                        }
                    }
                    _ => {
                        let _ = tx.send(WsCmd::Close(1003, "unexpected message"));
                        break;
                    }
                }
            }
            Message::Close(_) => break,
            Message::Text(_) => {
                let _ = tx.send(WsCmd::Close(1003, "unexpected message"));
                break;
            }
            _ => {}
        }
        if !open.load(Ordering::Relaxed) {
            break;
        }
    }
    let _ = cmd_tx.send(ReplayCmd::Close);
    let bytes = play.await.unwrap_or(0);
    let _ = tx.send(WsCmd::Terminate);
    let _ = write.await;
    ctl.replays.fetch_sub(1, Ordering::Relaxed);
    log::info!("[viewer] replay {name} closed ({} KB sent)", bytes / 1024);
}
