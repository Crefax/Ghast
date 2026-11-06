//! The feed → hub → stream pipeline with a fake bot: HELLO and snapshot, columns nearest first
//! under the ACK window, block changes, the pose, entities, recordings.

use std::io::Read as _;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use ghast_client::{GameVersion, Health, Pose, RawInbound, Vec3};
use ghast_entity::Entities;
use ghast_inventory::Stack;
use ghast_protocol::BlockPos;
use ghast_viewer::protocol::{Reader, decode_column, messages, s};
use ghast_viewer::stream::Out;
use ghast_viewer::{BotView, Viewer, ViewerConfig};

struct Fake {
    tick: u64,
    pose: Pose,
    loaded: Vec<(i32, i32)>,
    updates: Vec<(BlockPos, u32)>,
    entities: Option<Entities>,
}

impl Fake {
    fn new() -> Self {
        Fake {
            tick: 1,
            pose: Pose {
                pos: Vec3::new(8.5, 65.0, 8.5),
                yaw: 90.0,
                pitch: 10.0,
                on_ground: true,
                ..Pose::default()
            },
            loaded: vec![(0, 0), (1, 0), (-1, 0), (3, 3)],
            updates: Vec::new(),
            entities: None,
        }
    }
}

/// Stone up to y 63 (grass on top), with the column's x as the stone's meta (to tell them apart).
fn block(x: i32, y: i32, _z: i32) -> u32 {
    match y {
        0..=62 => (1 << 4) | ((x >> 4) & 3) as u32,
        63 => 2 << 4,
        _ => 0,
    }
}

impl BotView for Fake {
    fn tick(&self) -> u64 {
        self.tick
    }
    fn version(&self) -> GameVersion {
        GameVersion::V1_8_9
    }
    fn pose(&self) -> Pose {
        self.pose
    }
    fn entity_id(&self) -> i32 {
        1
    }
    fn health(&self) -> Health {
        Health::default()
    }
    fn held_slot(&self) -> u8 {
        0
    }
    fn hotbar(&self, _k: usize) -> Option<&Stack> {
        None
    }
    fn armor(&self, _a: usize) -> Option<&Stack> {
        None
    }
    fn mining(&self) -> Option<(BlockPos, f32)> {
        None
    }
    fn broken(&self) -> u64 {
        0
    }
    fn using_item(&self) -> bool {
        false
    }
    fn dimension(&self) -> &str {
        "0"
    }
    fn is_column_loaded(&self, cx: i32, cz: i32) -> bool {
        self.loaded.contains(&(cx, cz))
    }
    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        self.is_column_loaded(x >> 4, z >> 4)
            .then(|| block(x, y, z))
    }
    fn entities(&self) -> Option<&Entities> {
        self.entities.as_ref()
    }
    fn block_updates(&self) -> &[(BlockPos, u32)] {
        &self.updates
    }
    fn raw(&self) -> &[RawInbound] {
        &[]
    }
    fn joined(&self) -> bool {
        false
    }
    fn respawned(&self) -> bool {
        false
    }
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<Bytes>>>);

impl Out for Capture {
    fn send(&mut self, frame: Bytes) -> bool {
        self.0.lock().unwrap().push(frame);
        true
    }
    fn buffered(&self) -> usize {
        0
    }
    fn is_open(&self) -> bool {
        true
    }
    fn terminate(&mut self) {}
}

impl Capture {
    /// `(type, body)` of every message received so far, then cleared.
    fn take(&self) -> Vec<(u8, Vec<u8>)> {
        let frames = std::mem::take(&mut *self.0.lock().unwrap());
        frames
            .iter()
            .flat_map(|f| {
                messages(f)
                    .map(|m| {
                        let m = m.unwrap();
                        (m.ty, m.body.to_vec())
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

fn chunk_pos(body: &[u8]) -> (i32, i32) {
    let mut r = Reader::new(body);
    (r.i32().unwrap(), r.i32().unwrap())
}

fn inflate(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::DeflateDecoder::new(&body[9..])
        .read_to_end(&mut out)
        .unwrap();
    out
}

#[test]
fn page_gets_hello_snapshot_columns_blocks_and_pose() {
    let viewer = Viewer::new_inline(ViewerConfig::new("Bot").view_distance(3));
    let hub = viewer.hub();
    let mut bot = Fake::new();
    // nobody watches: nothing happens
    hub.tick(&bot);
    assert!(
        viewer.status().in_game,
        "the status knows the bot is in a world while nobody watches"
    );
    let cap = Capture::default();
    let id = hub.add(Box::new(cap.clone()), "fp", 1 << 20, true);
    let first = cap.take();
    assert_eq!(first[0].0, s::HELLO);
    let hello: serde_json::Value = serde_json::from_slice(&first[0].1).unwrap();
    assert_eq!(hello["account"], "Bot");
    assert_eq!(hello["viewDistance"], 3);
    assert_eq!(first[1].0, s::STATE);
    let state: serde_json::Value = serde_json::from_slice(&first[1].1).unwrap();
    assert_eq!(
        state["inGame"], true,
        "the session ticked before the page came"
    );

    // first tick: the feed binds, columns are asked for, encoded, sent nearest first
    hub.tick(&bot);
    bot.tick += 1;
    hub.tick(&bot);
    let msgs = cap.take();
    let types: Vec<u8> = msgs.iter().map(|m| m.0).collect();
    assert!(types.contains(&s::POSE));
    let chunks: Vec<(i32, i32)> = msgs
        .iter()
        .filter(|m| m.0 == s::CHUNK)
        .map(|m| chunk_pos(&m.1))
        .collect();
    assert_eq!(chunks[0], (0, 0), "nearest first");
    assert_eq!(chunks.len(), 3, "(3, 3) is out of range: {chunks:?}");
    let body = &msgs.iter().find(|m| m.0 == s::CHUNK).unwrap().1;
    let col = decode_column(&inflate(body)).unwrap();
    assert_eq!(col.sections.len(), 4);
    assert_eq!(
        col.sections[3].states[(15 << 8) | 5],
        2 << 4,
        "grass at y 63"
    );
    assert_eq!(col.sections[0].states[0], 1 << 4);

    // the pose: radians, flags
    let pose = &msgs.iter().find(|m| m.0 == s::POSE).unwrap().1;
    let mut r = Reader::new(pose);
    let _t = r.f64().unwrap();
    assert_eq!(r.u32().unwrap(), 1);
    assert_eq!(
        (r.f64().unwrap(), r.f64().unwrap(), r.f64().unwrap()),
        (8.5, 65.0, 8.5)
    );
    let yaw = r.f32().unwrap();
    assert!((yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    let _pitch = r.f32().unwrap();
    assert_eq!(r.u8().unwrap() & 1, 1, "on ground");

    // a block change: BLOCKS at the next tick, and the cached column is stale
    bot.updates = vec![(BlockPos::new(3, 64, 4), 4 << 4)];
    bot.tick += 1;
    hub.tick(&bot);
    bot.updates.clear();
    let msgs = cap.take();
    let blocks = &msgs.iter().find(|m| m.0 == s::BLOCKS).unwrap().1;
    let mut r = Reader::new(blocks);
    assert_eq!(r.u16().unwrap(), 1);
    assert_eq!(
        (
            r.i32().unwrap(),
            r.u8().unwrap(),
            r.i32().unwrap(),
            r.u16().unwrap()
        ),
        (3, 64, 4, 4 << 4)
    );
    assert!(
        !msgs.iter().any(|m| m.0 == s::POSE),
        "pose unchanged: not sent again within a second"
    );

    // a column the server unloads goes away on the page too
    bot.loaded.retain(|c| *c != (1, 0));
    bot.tick += 1;
    hub.tick(&bot);
    let msgs = cap.take();
    let unload = msgs
        .iter()
        .find(|m| m.0 == s::UNLOAD)
        .map(|m| chunk_pos(&m.1));
    assert_eq!(unload, Some((1, 0)));

    // a second page gets the cached columns without new encoding
    let cap2 = Capture::default();
    let id2 = hub.add(Box::new(cap2.clone()), "tp", 1 << 20, true);
    // (0, 0) changed since its encoding: it is encoded again at the next tick, and the cached
    // (-1, 0) waits behind it (columns go out in order)
    assert_eq!(cap2.take().iter().filter(|m| m.0 == s::CHUNK).count(), 0);
    bot.tick += 1;
    hub.tick(&bot);
    let got: Vec<(i32, i32)> = cap2
        .take()
        .iter()
        .filter(|m| m.0 == s::CHUNK)
        .map(|m| chunk_pos(&m.1))
        .collect();
    assert_eq!(got, vec![(0, 0), (-1, 0)]);
    hub.remove(id2, true);
    hub.remove(id, true);
    assert!(!hub.active());
}

#[test]
fn the_window_holds_columns_until_acked() {
    let viewer = Viewer::new_inline(ViewerConfig::new("Bot").view_distance(3));
    let hub = viewer.hub();
    let mut bot = Fake::new();
    bot.loaded = (-2..=2)
        .flat_map(|x| (-2..=2).map(move |z| (x, z)))
        .collect();
    let cap = Capture::default();
    // a window smaller than one column: the columns asked for while the window was open
    // (at most 4 in flight), then nothing until the ACK
    let id = hub.add(Box::new(cap.clone()), "fp", 10, true);
    for _ in 0..6 {
        hub.tick(&bot);
    }
    let sent: usize = cap.0.lock().unwrap().iter().map(Bytes::len).sum();
    let n = cap.take().iter().filter(|m| m.0 == s::CHUNK).count();
    assert!(
        (1..=4).contains(&n),
        "only the columns in flight when the window closed: {n}"
    );
    hub.ack(id, sent as f64);
    hub.tick(&bot);
    hub.tick(&bot);
    assert!(cap.take().iter().filter(|m| m.0 == s::CHUNK).count() >= 1);
}

#[test]
fn entities_spawn_move_and_go() {
    use ghast_protocol::v1_8::play::clientbound as cb;
    use ghast_protocol::v1_8::types::Vec3i16;
    let mut tracker = ghast_entity::v1_8::Tracker::new(ghast_entity::Config::default());
    let spawn = cb::SpawnEntityLiving {
        entity_id: 7,
        r#type: 54, // zombie
        x: 10 * 32,
        y: 64 * 32,
        z: 10 * 32,
        yaw: 64,
        pitch: 0,
        head_pitch: 64,
        velocity: Vec3i16 { x: 0, y: 0, z: 0 },
        metadata: Vec::new(),
    };
    let body = ghast_protocol::codec::encode_to_bytes(&spawn)
        .unwrap()
        .freeze();
    tracker
        .apply_body(cb::id::SPAWN_ENTITY_LIVING, &body)
        .unwrap();

    let viewer = Viewer::new_inline(ViewerConfig::new("Bot").view_distance(2));
    let hub = viewer.hub();
    let mut bot = Fake::new();
    bot.entities = Some((*tracker).clone());
    let cap = Capture::default();
    let _id = hub.add(Box::new(cap.clone()), "fp", 1 << 20, true);
    hub.tick(&bot);
    let msgs = cap.take();
    let sp = &msgs.iter().find(|m| m.0 == s::SPAWN).expect("a SPAWN").1;
    let mut r = Reader::new(sp);
    assert_eq!(r.u32().unwrap(), 7);
    assert_eq!(r.u8().unwrap(), ghast_viewer::protocol::kind::MOB);
    assert_eq!(r.str().unwrap(), "Zombie");
    assert_eq!(r.str().unwrap(), "Zombie");
    assert_eq!(
        (r.f64().unwrap(), r.f64().unwrap(), r.f64().unwrap()),
        (10.0, 64.0, 10.0)
    );

    // gone: REMOVE
    bot.entities = Some(Entities::default());
    hub.tick(&bot);
    let msgs = cap.take();
    let rm = &msgs.iter().find(|m| m.0 == s::REMOVE).expect("a REMOVE").1;
    assert_eq!(rm[..6], [1, 0, 7, 0, 0, 0]);
}

#[test]
fn recordings_replay_from_disk() {
    let dir = std::env::temp_dir().join(format!("ghast-viewer-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let viewer = Viewer::new_inline(ViewerConfig::new("Bot").view_distance(2));
    let bot = Fake::new();
    let mut rec = ghast_viewer::FeedRecorder::start(&viewer, &dir);
    for _ in 0..5 {
        viewer.hub().tick(&bot);
    }
    let meta = rec.stop();
    assert!(meta.frames >= 3, "{meta:?}");
    assert!(meta.end.is_some());
    let list = ghast_viewer::list_recordings(&dir);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, meta.name);
    let frames =
        ghast_viewer::record::read_recording(&dir.join(format!("{}.rec", meta.name))).unwrap();
    assert_eq!(frames.len() as u64, meta.frames);
    let first = messages(&frames[0].1).next().unwrap().unwrap();
    assert_eq!(first.ty, s::HELLO);
    let all: Vec<u8> = frames.iter().flat_map(|f| f.1.to_vec()).collect();
    assert!(messages(&all).any(|m| m.unwrap().ty == s::CHUNK));
    assert!(!viewer.hub().active(), "the recorder detached");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recording_time_starts_in_the_world() {
    let dir = std::env::temp_dir().join(format!("ghast-viewer-test-join-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let viewer = Viewer::new_inline(ViewerConfig::new("Bot").view_distance(2));
    // started before the bot is in a world (as an app does before connecting)
    let mut rec = ghast_viewer::FeedRecorder::start(&viewer, &dir);
    std::thread::sleep(std::time::Duration::from_millis(120));
    let bot = Fake::new();
    for _ in 0..3 {
        viewer.hub().tick(&bot);
    }
    let meta = rec.stop();
    let frames =
        ghast_viewer::record::read_recording(&dir.join(format!("{}.rec", meta.name))).unwrap();
    let in_game = frames
        .iter()
        .position(|(_, f)| messages(f).any(|m| m.unwrap().ty == s::POSE))
        .expect("a POSE");
    assert!(
        frames[..=in_game].iter().all(|(t, _)| *t == 0),
        "0:00 is the join"
    );
    assert!(
        frames.iter().all(|(t, _)| *t < 100),
        "the wait before the join is not in the recording"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "current_thread")]
async fn a_dash_cam_clip_with_nothing_buffered_records_from_now() {
    let dir = std::env::temp_dir().join(format!("ghast-viewer-test-cam-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let viewer = Viewer::new_inline(ViewerConfig::new("Bot").view_distance(2));
    let cam = ghast_viewer::DashCam::start(&viewer, &dir, ghast_viewer::DashCamOptions::default());
    // the first check (which starts buffering) has not run: nothing is buffered
    assert_eq!(cam.buffering(), 0);
    let name = cam.clip("player near: Alex").expect("a clip");
    assert_eq!(
        cam.clip("again").as_deref(),
        Some(name.as_str()),
        "a second trigger extends it"
    );
    let bot = Fake::new();
    for _ in 0..3 {
        viewer.hub().tick(&bot);
    }
    cam.stop();
    let list = ghast_viewer::list_recordings(&dir);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, name);
    assert_eq!(list[0].note.as_deref(), Some("player near: Alex"));
    assert!(
        list[0].frames >= 2 && list[0].end.is_some(),
        "{:?}",
        list[0]
    );
    let _ = std::fs::remove_dir_all(&dir);
}
