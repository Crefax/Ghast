//! A 26.1 session through the feed: blocks and items mapped onto the page's 1.8 ones, the
//! dimension's rows shifted into the page's 256 (overworld: page y = world y + 64).
#![cfg(feature = "v26_1")]

use std::io::Read as _;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use ghast_client::{GameVersion, Health, Pose, RawInbound, Vec3};
use ghast_data::v26_1::{BlockState, Item};
use ghast_entity::Entities;
use ghast_inventory::Stack;
use ghast_protocol::BlockPos;
use ghast_viewer::protocol::{Reader, decode_column, messages, s};
use ghast_viewer::stream::Out;
use ghast_viewer::{BotView, Viewer, ViewerConfig};

struct Fake {
    tick: u64,
    pose: Pose,
    hotbar: Vec<Stack>,
    updates: Vec<(BlockPos, u32)>,
}

fn id(spec: &str) -> u32 {
    u32::from(BlockState::parse(spec).unwrap().id())
}

/// Deepslate below 0, stone 0..=2, grass at 3, a cherry log at (5, 4..=6, 5).
fn block(x: i32, y: i32, z: i32) -> u32 {
    match y {
        -64..=-1 => id("deepslate[axis=y]"),
        0..=2 => id("stone"),
        3 => id("grass_block[snowy=false]"),
        4..=6 if x == 5 && z == 5 => id("cherry_log[axis=y]"),
        _ => 0,
    }
}

impl BotView for Fake {
    fn tick(&self) -> u64 {
        self.tick
    }
    fn version(&self) -> GameVersion {
        GameVersion::V26_1
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
    fn hotbar(&self, k: usize) -> Option<&Stack> {
        self.hotbar.get(k)
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
        "minecraft:overworld"
    }
    fn is_column_loaded(&self, cx: i32, cz: i32) -> bool {
        (cx, cz) == (0, 0)
    }
    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        self.is_column_loaded(x >> 4, z >> 4)
            .then(|| block(x, y, z))
    }
    fn entities(&self) -> Option<&Entities> {
        None
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
    fn build_height(&self) -> Option<(i32, i32)> {
        Some((-64, 384))
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

fn item(name: &str, count: i32, damage: i16) -> Stack {
    Stack::new(
        i32::from(Item::from_name(name).unwrap().id()),
        count,
        damage,
    )
}

#[test]
fn a_26_1_session_is_drawn_with_1_8_blocks_and_items() {
    let viewer = Viewer::new_inline(ViewerConfig::new("Bot").view_distance(2));
    let hub = viewer.hub();
    let mut bot = Fake {
        tick: 1,
        pose: Pose {
            pos: Vec3::new(8.5, 4.0, 8.5),
            on_ground: true,
            ..Pose::default()
        },
        hotbar: vec![
            item("diamond_pickaxe", 1, 5),
            item("cherry_planks", 32, 0),
            item("netherite_ingot", 3, 0),
            item("music_disc_creator", 1, 0),
        ],
        updates: Vec::new(),
    };
    hub.tick(&bot);
    let cap = Capture::default();
    let _id = hub.add(Box::new(cap.clone()), "fp", 1 << 20, true);
    let first = cap.take();
    let state: serde_json::Value = serde_json::from_slice(&first[1].1).unwrap();
    assert_eq!(state["inGame"], true);
    hub.tick(&bot);
    bot.tick += 1;
    hub.tick(&bot);
    let msgs = cap.take();

    // The column: world -64 is page 0 (deepslate → stone), grass at world 3 = page 67, the
    // cherry log (→ birch log) at page 68..=70.
    let body = &msgs.iter().find(|m| m.0 == s::CHUNK).expect("a column").1;
    let mut raw = Vec::new();
    flate2::read::DeflateDecoder::new(&body[9..])
        .read_to_end(&mut raw)
        .unwrap();
    let col = decode_column(&raw).unwrap();
    let at = |y: usize, x: usize, z: usize| {
        col.sections
            .iter()
            .find(|s| usize::from(s.y) == y >> 4)
            .map_or(0, |s| s.states[((y & 15) << 8) | (z << 4) | x])
    };
    assert_eq!(at(0, 0, 0), 1 << 4, "deepslate as stone");
    assert_eq!(at(66, 0, 0), 1 << 4, "stone");
    assert_eq!(at(67, 0, 0), 2 << 4, "grass");
    assert_eq!(at(68, 5, 5), 17 << 4 | 2, "cherry log as a birch log");
    assert_eq!(at(68, 0, 0), 0, "air");

    // The pose is shifted by the same 64.
    let pose = &msgs.iter().find(|m| m.0 == s::POSE).unwrap().1;
    let mut r = Reader::new(pose);
    let _t = r.f64().unwrap();
    let _tick = r.u32().unwrap();
    assert_eq!(
        (r.f64().unwrap(), r.f64().unwrap(), r.f64().unwrap()),
        (8.5, 68.0, 8.5)
    );

    // The hotbar: 1.8 ids (netherite ingot as iron, paper for what has no look-alike).
    let inv = &msgs.iter().find(|m| m.0 == s::INVENTORY).unwrap().1;
    let mut r = Reader::new(inv);
    assert_eq!(r.u8().unwrap(), 0);
    let mut slots = Vec::new();
    for _ in 0..4 {
        slots.push((r.u16().unwrap(), r.u16().unwrap(), r.u8().unwrap()));
    }
    assert_eq!(
        slots,
        vec![(278, 5, 1), (5, 2, 32), (265, 0, 3), (339, 0, 1)]
    );

    // A block change: shifted and mapped.
    bot.updates = vec![(BlockPos::new(3, 4, 4), id("copper_block"))];
    bot.tick += 1;
    hub.tick(&bot);
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
        (3, 68, 4, 159 << 4 | 1)
    );
}
