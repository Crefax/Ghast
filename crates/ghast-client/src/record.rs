//! Movement recorder (1.8.9 and 1.16.5): writes a session in the oracle's replay format so
//! vanilla can replay exactly what the client did (`oracle/run.sh replay --in <file>`).
//!
//! - `setup` is the player at the first movement step (right after the first server
//!   position, before any move): pose, the exact bounding box (`bb`, so a replay needs no
//!   box rebuild), and on 1.16.5 the pose, with the abilities and food known then.
//! - `loaded_chunks` / `fills`: every chunk column that was loaded within 3 chunks of the
//!   player at any step of the session, with its blocks as first seen (non-air x-runs). The
//!   header is therefore written when the session ends (the tick lines are kept in memory).
//! - One `tick` line per movement step: `input` (keys, yaw, pitch used, and the inbound
//!   events applied before it — teleports with the reply the client sent (`ghast_reply`),
//!   our entity velocity, abilities) and ghast's own `state` after the step plus
//!   `ghast_packets`, so a replay diff shows the first tick where vanilla and ghast disagree.
//!
//! Limits: block changes after a column was first seen and explosion pushes are not
//! representable (`ghast_note` lines). Teleports are written with the
//! raw values and their `flags` byte (the oracle applies them like the client handler).

// One build may include only one of the two recorded versions.
#![cfg_attr(not(all(feature = "v1_8", feature = "v1_16_5")), allow(dead_code))]

use std::collections::BTreeMap;
use std::io::Write as _;

use ghast_physics::v1_8::PlayerState;

/// Hex-tagged value pairs as the oracle writes them.
fn f64p(v: f64) -> String {
    format!("[{v:?},\"{:016x}\"]", v.to_bits())
}

fn f32p(v: f32) -> String {
    format!("[{v:?},\"{:08x}\"]", v.to_bits())
}

fn f64x3(v: [f64; 3]) -> String {
    format!("[{},{},{}]", f64p(v[0]), f64p(v[1]), f64p(v[2]))
}

fn bb6(bb: &ghast_data::Aabb) -> String {
    format!(
        "[{},{},{},{},{},{}]",
        f64p(bb.min_x),
        f64p(bb.min_y),
        f64p(bb.min_z),
        f64p(bb.max_x),
        f64p(bb.max_y),
        f64p(bb.max_z)
    )
}

/// A run of equal blocks along x: `(x0, y, z, x1, state)`.
type Run = (i32, i32, i32, i32, u32);

/// Protocol version of a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ver {
    V1_8,
    V1_16,
}

/// The recorder of one session.
pub(crate) struct Recorder {
    path: std::path::PathBuf,
    ver: Option<Ver>,
    setup: String,
    /// Tick lines, streamed to `<path>.part` (the header needs the whole session).
    lines: std::io::BufWriter<std::fs::File>,
    part: std::path::PathBuf,
    chunks: BTreeMap<(i32, i32), Vec<Run>>,
    t: u64,
    events: Vec<String>,
    finished: bool,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder")
            .field("path", &self.path)
            .field("t", &self.t)
            .finish_non_exhaustive()
    }
}

impl Recorder {
    /// A recorder writing to `path` when the session ends (checked for writability now).
    pub(crate) fn create(path: &std::path::Path) -> std::io::Result<Recorder> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::File::create(path)?;
        let part = path.with_extension("part");
        Ok(Recorder {
            path: path.to_owned(),
            ver: None,
            setup: String::new(),
            lines: std::io::BufWriter::new(std::fs::File::create(&part)?),
            part,
            chunks: BTreeMap::new(),
            t: 0,
            events: Vec::new(),
            finished: false,
        })
    }

    /// Whether the setup has been taken.
    pub(crate) fn started(&self) -> bool {
        self.ver.is_some()
    }

    /// Takes the 1.8.9 setup at the first movement step.
    pub(crate) fn start_1_8(&mut self, p: &PlayerState) {
        let a = &p.abilities;
        self.setup = format!(
            r#"{{"game_mode":"{}","pos":{},"bb":{},"yaw":{},"pitch":{},"vel":{},"on_ground":{},"fall_distance":{},"sprinting":false,"flying":{},"allow_flying":{},"fly_speed":{},"walk_speed":{},"movement_speed_base":{},"food_level":{},"effects":[]}}"#,
            if a.creative { "creative" } else { "survival" },
            f64x3(p.pos),
            bb6(&p.bb),
            f32p(p.yaw),
            f32p(p.pitch),
            f64x3(p.vel),
            p.on_ground,
            f32p(p.fall_distance),
            a.flying,
            a.allow_flying,
            f32p(a.fly_speed),
            f32p(a.walk_speed),
            f64p(p.movement_speed_base),
            p.food_level,
        );
        self.ver = Some(Ver::V1_8);
        self.events.clear();
    }

    /// Takes the 1.16.5 setup at the first movement step.
    pub(crate) fn start_1_16(&mut self, p: &ghast_physics::v1_16_5::PlayerState) {
        let a = &p.abilities;
        self.setup = format!(
            r#"{{"game_mode":"{}","pos":{},"bb":{},"pose":"{}","yaw":{},"pitch":{},"vel":{},"on_ground":{},"fall_distance":{},"sprinting":false,"flying":{},"allow_flying":{},"fly_speed":{},"walk_speed":{},"movement_speed_base":{},"food_level":{},"effects":[],"speed_modifiers":[],"auto_jump":{},"soul_speed":0,"depth_strider":0}}"#,
            if a.creative { "creative" } else { "survival" },
            f64x3(p.pos),
            bb6(&p.bb),
            format!("{:?}", p.pose).to_lowercase(),
            f32p(p.yaw),
            f32p(p.pitch),
            f64x3(p.vel),
            p.on_ground,
            f32p(p.fall_distance),
            a.flying,
            a.allow_flying,
            f32p(a.fly_speed),
            f32p(a.walk_speed),
            f64p(p.movement_speed_base),
            p.food_level,
            p.auto_jump,
        );
        self.ver = Some(Ver::V1_16);
        self.events.clear();
    }

    /// Snapshots the loaded columns within 3 chunks of `pos` not seen before. `block`
    /// returns the raw state (1.8 `id << 4 | meta`, 1.16 global id), `None` when the column
    /// is not loaded.
    pub(crate) fn observe(&mut self, pos: [f64; 3], block: impl Fn(i32, i32, i32) -> Option<u32>) {
        if !self.started() {
            return;
        }
        let cx = (pos[0].floor() as i32) >> 4;
        let cz = (pos[2].floor() as i32) >> 4;
        for x in cx - 3..=cx + 3 {
            for z in cz - 3..=cz + 3 {
                if self.chunks.contains_key(&(x, z)) || block(x * 16, 0, z * 16).is_none() {
                    continue;
                }
                self.chunks.insert((x, z), runs(x, z, &block));
            }
        }
    }

    /// A server teleport applied before the next step: the raw packet values (`raw_*`, with
    /// `flags`, which the oracle applies like the client handler) and the resolved pose the
    /// client then had and replied with.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn teleport(
        &mut self,
        raw_pos: [f64; 3],
        raw_yaw: f32,
        raw_pitch: f32,
        flags: i8,
        teleport_id: Option<i32>,
        pos: [f64; 3],
        yaw: f32,
        pitch: f32,
    ) {
        if !self.started() {
            return;
        }
        let confirm = teleport_id
            .map(|id| format!(r#""TeleportConfirm({id})","#))
            .unwrap_or_default();
        self.events.push(format!(
            r#"{{"type":"teleport","pos":{},"yaw":{},"pitch":{},"flags":{},"ghast_reply":[{confirm}"PositionLook {{ x: {:?}, y: {:?}, z: {:?}, yaw: {:?}, pitch: {:?}, on_ground: false }}"]}}"#,
            f64x3(raw_pos),
            f32p(raw_yaw),
            f32p(raw_pitch),
            flags as u8,
            pos[0],
            pos[1],
            pos[2],
            yaw,
            pitch
        ));
    }

    /// Our entity velocity (raw 1/8000 units).
    pub(crate) fn velocity(&mut self, raw: [i32; 3]) {
        if self.started() {
            self.events.push(format!(
                r#"{{"type":"velocity","x":{},"y":{},"z":{}}}"#,
                raw[0], raw[1], raw[2]
            ));
        }
    }

    /// Our movement speed attribute (entity properties): base and every modifier.
    pub(crate) fn speed_attribute(&mut self, base: f64, modifiers: &[(u128, f64, u8)]) {
        if self.started() {
            let mods: Vec<String> = modifiers
                .iter()
                .map(|(id, amount, op)| {
                    format!(
                        "[\"{}\",{},{op}]",
                        uuid::Uuid::from_u128(*id),
                        f64p(*amount)
                    )
                })
                .collect();
            self.events.push(format!(
                r#"{{"type":"speed_attribute","base":{},"modifiers":[{}]}}"#,
                f64p(base),
                mods.join(",")
            ));
        }
    }

    /// Our status effects after a change (id → amplifier), as the full set.
    pub(crate) fn effects(&mut self, effects: &std::collections::BTreeMap<i8, i8>) {
        if self.started() {
            let e: Vec<String> = effects
                .iter()
                .map(|(id, a)| format!("[{id},{a}]"))
                .collect();
            self.events.push(format!(
                r#"{{"type":"effects","effects":[{}]}}"#,
                e.join(",")
            ));
        }
    }

    /// Server abilities.
    pub(crate) fn abilities(&mut self, a: &ghast_physics::Abilities) {
        if self.started() {
            self.events.push(format!(
                r#"{{"type":"abilities","invulnerable":{},"flying":{},"allow_flying":{},"creative":{},"fly_speed":{},"walk_speed":{}}}"#,
                a.invulnerable,
                a.flying,
                a.allow_flying,
                a.creative,
                f32p(a.fly_speed),
                f32p(a.walk_speed)
            ));
        }
    }

    /// Something the format cannot express.
    pub(crate) fn note(&mut self, what: &str) {
        if self.started() {
            let _ = writeln!(
                self.lines,
                r#"{{"type":"ghast_note","before_t":{},"note":"{what}"}}"#,
                self.t + 1
            );
        }
    }

    /// One movement step: its input and ghast's result (`state`: a JSON object).
    pub(crate) fn tick(
        &mut self,
        input: &ghast_physics::TickInput,
        state: &str,
        packets: &ghast_physics::Packets,
    ) {
        if !self.started() {
            return;
        }
        self.t += 1;
        let k = &input.keys;
        let keys: Vec<&str> = [
            (k.forward, "forward"),
            (k.back, "back"),
            (k.left, "left"),
            (k.right, "right"),
            (k.jump, "jump"),
            (k.sneak, "sneak"),
            (k.sprint, "sprint"),
            (input.using_item, "use"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, n)| *n)
        .collect();
        let events = if self.events.is_empty() {
            String::new()
        } else {
            format!(r#","events":[{}]"#, self.events.join(","))
        };
        self.events.clear();
        let pk: Vec<String> = packets
            .as_slice()
            .iter()
            .map(|p| format!("\"{p:?}\""))
            .collect();
        let _ = writeln!(
            self.lines,
            r#"{{"type":"tick","t":{},"input":{{"keys":"{}","yaw":{},"pitch":{}{events}}},"state":{state},"ghast_packets":[{}]}}"#,
            self.t,
            keys.join(","),
            f32p(input.yaw),
            f32p(input.pitch),
            pk.join(","),
        );
    }

    /// Writes the file (header + tick lines). Called when the session ends.
    pub(crate) fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        let Some(ver) = self.ver else {
            let _ = std::fs::remove_file(&self.part);
            return;
        };
        let loaded: Vec<String> = self
            .chunks
            .keys()
            .map(|(x, z)| format!("[{x},{z},{x},{z}]"))
            .collect();
        let mut fills = Vec::new();
        let mut ids = Vec::new();
        for runs in self.chunks.values() {
            for &(x0, y, z, x1, r) in runs {
                fills.push(match ver {
                    Ver::V1_8 => format!("[{x0},{y},{z},{x1},{y},{z},{},{}]", r >> 4, r & 15),
                    Ver::V1_16 => {
                        ids.push(r);
                        format!("[{x0},{y},{z},{x1},{y},{z},{r}]")
                    }
                });
            }
        }
        let (version, states) = match ver {
            Ver::V1_8 => ("1.8.9", String::new()),
            Ver::V1_16 => {
                ids.sort_unstable();
                ids.dedup();
                let names: Vec<String> = ids
                    .iter()
                    .filter_map(|&id| {
                        let s = ghast_data::v1_16_5::BlockState::from_id(u16::try_from(id).ok()?)?;
                        Some(format!("\"{id}\":\"{s}\""))
                    })
                    .collect();
                ("1.16.5", format!(r#""states":{{{}}},"#, names.join(",")))
            }
        };
        let mut write = || -> std::io::Result<()> {
            let mut out = std::io::BufWriter::new(std::fs::File::create(&self.path)?);
            writeln!(
                out,
                r#"{{"type":"scenario","format":1,"version":"{version}","name":"ghast_client_recording","description":"recorded by ghast-client: setup = first movement step","outside":"unloaded","loaded_chunks":[{}],"fills":[{}],{states}"setup":{},"ticks":{}}}"#,
                loaded.join(","),
                fills.join(","),
                self.setup,
                self.t
            )?;
            self.lines.flush()?;
            std::io::copy(&mut std::fs::File::open(&self.part)?, &mut out)?;
            out.flush()
        };
        if let Err(e) = write() {
            log::warn!("recording {}: {e}", self.path.display());
        }
        let _ = std::fs::remove_file(&self.part);
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Non-air x-runs of one chunk column.
fn runs(cx: i32, cz: i32, block: &impl Fn(i32, i32, i32) -> Option<u32>) -> Vec<Run> {
    let mut out = Vec::new();
    for y in 0..256 {
        for bz in 0..16 {
            let z = cz * 16 + bz;
            let mut run: Option<(i32, u32)> = None;
            for bx in 0..=16 {
                let x = cx * 16 + bx;
                let raw = if bx < 16 {
                    block(x, y, z).unwrap_or(0)
                } else {
                    u32::MAX
                };
                match run {
                    Some((_, r)) if r == raw => {}
                    Some((x0, r)) => {
                        if r != 0 {
                            out.push((x0, y, z, x - 1, r));
                        }
                        run = Some((x, raw));
                    }
                    None => run = Some((x, raw)),
                }
            }
        }
    }
    out
}

/// The 1.8 state fields the oracle writes (subset).
pub(crate) fn state_1_8(p: &PlayerState) -> String {
    format!(
        r#"{{"pos":{},"bb":{},"vel":{},"yaw":{},"pitch":{},"on_ground":{},"horizontal_collision":{},"fall_distance":{},"sprinting":{}}}"#,
        f64x3(p.pos),
        bb6(&p.bb),
        f64x3(p.vel),
        f32p(p.yaw),
        f32p(p.pitch),
        p.on_ground,
        p.horizontal_collision,
        f32p(p.fall_distance),
        p.sprinting,
    )
}

/// The 1.16.5 state fields the oracle writes (subset).
pub(crate) fn state_1_16(p: &ghast_physics::v1_16_5::PlayerState) -> String {
    format!(
        r#"{{"pos":{},"bb":{},"vel":{},"yaw":{},"pitch":{},"on_ground":{},"horizontal_collision":{},"vertical_collision":{},"fall_distance":{},"sprinting":{},"swimming":{},"touching_water":{}}}"#,
        f64x3(p.pos),
        bb6(&p.bb),
        f64x3(p.vel),
        f32p(p.yaw),
        f32p(p.pitch),
        p.on_ground,
        p.horizontal_collision,
        p.vertical_collision,
        f32p(p.fall_distance),
        p.sprinting,
        p.swimming,
        p.touching_water,
    )
}
