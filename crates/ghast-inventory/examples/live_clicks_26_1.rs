//! Lab tool: 26.1 click prediction and click hashes against a live vanilla 26.1.x server.
//!
//! ```text
//! cargo run --release -p ghast-inventory --example live_clicks_26_1 -- 127.0.0.1:25565 [name] [capture.bin.zz]
//! ```
//!
//! With a capture path, the configuration registries, the inventory packets of both directions
//! and nothing else are written there (record format of `ghast-protocol/examples/login_26_1.rs`)
//! for `tests/v26_1.rs`.
//!
//! Logs in offline, waits for the console to give the player items (see `lab` notes in the
//! crate README), then runs click steps in window 0 and in a chest next to the player. After
//! each click the server sends `container_set_slot` / `set_cursor_item` only for slots whose
//! hash differs from its own result, so every correction it sends is a misprediction or a hash
//! ghast got wrong; the step's line reports them.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use bytes::BytesMut;
use ghast_inventory::v26_1::HashRegistries;
use ghast_inventory::{Click, DragKind, DragStage, Inventory, OUTSIDE, Outgoing, Stack, Version};
use ghast_protocol::types::Uuid;
use ghast_protocol::v26_1::play::clientbound::{self as cb, ClientboundPlayPacket as P};
use ghast_protocol::v26_1::play::serverbound as sb;
use ghast_protocol::v26_1::{configuration, handshaking, login};
use ghast_protocol::{BlockPos, FrameDecoder, FrameEncoder, Packet, PacketSet, RawPacket, chat};

struct Conn {
    tcp: TcpStream,
    dec: FrameDecoder,
    enc: FrameEncoder,
    state: u8,
    capture: Option<BytesMut>,
}

impl Conn {
    fn record(&mut self, dir: u8, raw: &RawPacket) {
        use bytes::BufMut;
        use ghast_protocol::McWrite;
        let keep = match (self.state, dir) {
            (4, 0) => ghast_inventory::PACKET_IDS_26_1.contains(&raw.id),
            (4, 1) => raw.id == sb::id::WINDOW_CLICK || raw.id == sb::id::CLOSE_WINDOW,
            _ => false,
        };
        if let (true, Some(c)) = (keep, &mut self.capture) {
            c.put_u8(self.state);
            c.put_u8(dir);
            c.put_varint(raw.id);
            c.put_varint(raw.body.len() as i32);
            c.put_slice(&raw.body);
        }
    }

    fn record_registry(&mut self, raw: &RawPacket) {
        use bytes::BufMut;
        use ghast_protocol::McWrite;
        if let Some(c) = &mut self.capture {
            c.put_u8(3);
            c.put_u8(0);
            c.put_varint(raw.id);
            c.put_varint(raw.body.len() as i32);
            c.put_slice(&raw.body);
        }
    }

    fn send<T: Packet>(&mut self, p: &T) {
        let raw = RawPacket::from_packet(p).expect("encode");
        self.record(1, &raw);
        self.enc.write_raw(&raw).expect("frame");
        let out = self.enc.take();
        self.tcp.write_all(&out).expect("write");
    }

    fn recv(&mut self) -> Option<RawPacket> {
        loop {
            if let Some(p) = self.dec.next_packet().expect("frame") {
                self.record(0, &p);
                return Some(p);
            }
            let mut buf = [0u8; 65536];
            match self.tcp.read(&mut buf) {
                Ok(0) => panic!("server closed the connection"),
                Ok(n) => self.dec.push(&buf[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return None;
                }
                Err(e) => panic!("read: {e}"),
            }
        }
    }
}

struct Lab {
    c: Conn,
    inv: Inventory,
    pos: [f64; 3],
    corrections: usize,
    log_corrections: bool,
}

impl Lab {
    /// Handles inbound play packets for `ms` milliseconds.
    fn pump(&mut self, ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            let Some(raw) = self.c.recv() else { continue };
            if matches!(
                raw.id,
                cb::id::SET_SLOT
                    | cb::id::WINDOW_ITEMS
                    | cb::id::SET_CURSOR_ITEM
                    | cb::id::SET_PLAYER_INVENTORY
            ) {
                self.corrections += 1;
                if self.log_corrections {
                    println!("    <- correction {:#04x}", raw.id);
                }
            }
            if raw.id == cb::id::OPEN_WINDOW || raw.id == cb::id::CLOSE_WINDOW {
                println!(
                    "    <- {} ({} bytes)",
                    if raw.id == cb::id::OPEN_WINDOW {
                        "open_screen"
                    } else {
                        "container_close"
                    },
                    raw.body.len()
                );
            }
            if let Err(e) = self.inv.apply_raw_v26_1(&raw) {
                println!("DECODE ERROR {:#04x}: {e}", raw.id);
                let dir = std::env::var("GHAST_DUMP_DIR").unwrap_or_else(|_| ".".into());
                let path = format!("{dir}/ghast_bad_{:x}_{}.bin", raw.id, raw.body.len());
                let _ = std::fs::write(&path, &raw.body);
                println!("  body saved to {path}");
            }
            let Ok(p) = P::decode_raw(&raw) else { continue };
            match p {
                P::KeepAlive(k) => self.c.send(&sb::KeepAlive {
                    keep_alive_id: k.keep_alive_id,
                }),
                P::Ping(p) => self.c.send(&sb::Pong { id: p.id }),
                P::Position(pos) => {
                    self.pos = [pos.x, pos.y, pos.z];
                    self.c.send(&sb::TeleportConfirm {
                        teleport_id: pos.teleport_id,
                    });
                    self.c.send(&sb::PositionLook {
                        x: pos.x,
                        y: pos.y,
                        z: pos.z,
                        yaw: pos.yaw,
                        pitch: pos.pitch,
                        flags: sb::MovementFlags::ON_GROUND,
                    });
                    self.c.send(&sb::PlayerLoaded {});
                }
                P::ChunkBatchFinished(_) => self.c.send(&sb::ChunkBatchReceived {
                    chunks_per_tick: 20.0,
                }),
                P::KickDisconnect(d) => panic!("kicked: {}", chat::nbt_to_plain_text(&d.reason)),
                P::SystemChat(m) => println!("  chat: {}", chat::nbt_to_plain_text(&m.content)),
                _ => {}
            }
        }
    }

    fn send_out(&mut self, out: &Outgoing) {
        // the real client swings its arm for every stack a click drops, before the click
        if let Outgoing::ContainerClick(c) = out {
            for _ in 0..c.swings {
                self.c.send(&sb::ArmAnimation {
                    hand: sb::ArmAnimationHand(0),
                });
            }
        }
        let p = self.inv.encode_v26_1(out).expect("26.1 packet");
        match p {
            sb::ServerboundPlayPacket::WindowClick(c) => self.c.send(&c),
            sb::ServerboundPlayPacket::CloseWindow(c) => self.c.send(&c),
            other => panic!("{other:?}"),
        }
    }

    fn find(&self, name: &str) -> Option<i16> {
        let v = Version::V26_1;
        self.inv.find(|s| s.item_name(v) == Some(name))
    }

    fn empty_slot(&self, from: usize) -> Option<i16> {
        (from..self.inv.slot_count())
            .find(|&i| self.inv.slot(i).is_none())
            .map(|i| i as i16)
    }

    /// One click: predicts, sends, waits, reports the server's corrections.
    fn step(&mut self, label: &str, click: Click) -> bool {
        self.inv.tick();
        let out = match self.inv.click(click) {
            Ok(o) => o,
            Err(e) => {
                println!("{label:32} refused: {e}");
                return false;
            }
        };
        let Outgoing::ContainerClick(c) = &out else {
            panic!("{out:?}")
        };
        let exact = c
            .changed
            .iter()
            .map(|(_, s)| s.as_ref())
            .chain([c.carried.as_ref()])
            .all(|s| ghast_inventory::v26_1::hashed_slot(s, self.inv.registries_26_1()).1);
        let predicted = c.predicted;
        let changed = c.changed.len();
        self.send_out(&out);
        self.corrections = 0;
        self.pump(700);
        println!(
            "{label:32} predicted {predicted:5} hashes exact {exact:5} changed {changed:2} -> server corrections {}",
            self.corrections
        );
        self.corrections == 0
    }
}

fn dump(inv: &Inventory) {
    let v = Version::V26_1;
    for i in 0..inv.slot_count() {
        if let Some(s) = inv.slot(i) {
            let name = s
                .custom_name(v)
                .map(|t| format!(" \"{}\"", t.to_plain()))
                .unwrap_or_default();
            let ench = s.enchantments(v);
            println!(
                "  slot {i:2}: {} x{} dmg {}/{}{name} ench {:?}",
                s.item_name(v).unwrap_or("?"),
                s.count(),
                s.damage_value(v),
                s.max_damage(v).unwrap_or(0),
                ench.iter()
                    .map(|e| (
                        e.id.name(inv.registries_26_1().enchantments())
                            .unwrap_or("?")
                            .to_string(),
                        e.level
                    ))
                    .collect::<Vec<_>>()
            );
        }
    }
    if let Some(c) = inv.cursor() {
        println!("  cursor: {:?} x{}", c.item_name(v), c.count());
    }
}

/// `HASH_BATCHES=n`: the driver gives batches of items (one component kind each, see the crate
/// README); every stack is picked up and put back, and the server's corrections are counted per
/// stack. Prints `BATCH DONE k` after each batch and waits for the inventory to empty and refill.
fn hash_batches(lab: &mut Lab, batches: usize) {
    use ghast_inventory::ItemData;
    use ghast_inventory::v26_1::{COMPONENT_NAMES, hash_covers};
    let v = Version::V26_1;
    lab.log_corrections = false;
    for k in 0..batches {
        // items arrive, then 2 s of quiet
        let start = Instant::now();
        loop {
            lab.corrections = 0;
            lab.pump(2000);
            let any = (9..45).any(|i| lab.inv.slot(i).is_some());
            if any && lab.corrections == 0 {
                break;
            }
            if start.elapsed() > Duration::from_secs(60) {
                println!("BATCH TIMEOUT {k}");
                return;
            }
        }
        lab.inv.tick();
        let _ = lab.inv.open_inventory();
        for slot in 9..45i16 {
            let Some(s) = lab.inv.slot(slot as usize).cloned() else {
                continue;
            };
            let names: Vec<String> = match s.data() {
                ItemData::Components(p) => p
                    .iter()
                    .map(|(t, _)| {
                        let n = COMPONENT_NAMES.get(t as usize).copied().unwrap_or("?");
                        format!("{n}{}", if hash_covers(t) { "" } else { "(uncovered)" })
                    })
                    .chain(p.removed().map(|t| {
                        format!(
                            "!{}",
                            COMPONENT_NAMES.get(t as usize).copied().unwrap_or("?")
                        )
                    }))
                    .collect(),
                _ => Vec::new(),
            };
            let exact = ghast_inventory::v26_1::hashed_slot(Some(&s), lab.inv.registries_26_1()).1;
            let mut corr = 0;
            for _ in 0..2 {
                lab.inv.tick();
                lab.inv.tick();
                let out = match lab.inv.click(Click::Pickup { slot, right: false }) {
                    Ok(o) => o,
                    Err(e) => {
                        println!("  click refused: {e}");
                        break;
                    }
                };
                lab.send_out(&out);
                lab.corrections = 0;
                lab.pump(400);
                corr += lab.corrections;
            }
            println!(
                "ITEM {:<24} exact {exact:5} corrections {corr} [{}]",
                s.item_name(v).unwrap_or("?"),
                names.join(", ")
            );
        }
        println!("BATCH DONE {k}");
        // wait until the driver cleared the inventory
        let start = Instant::now();
        while (9..45).any(|i| lab.inv.slot(i).is_some())
            && start.elapsed() < Duration::from_secs(30)
        {
            lab.pump(200);
        }
    }
}

/// Every slot of the current screen and the cursor.
fn snapshot(inv: &Inventory) -> (Vec<Option<Stack>>, Option<Stack>) {
    (
        (0..inv.slot_count())
            .map(|i| inv.slot(i).cloned())
            .collect(),
        inv.cursor().cloned(),
    )
}

/// `MENUS=1`: drags, bundles, a crafting table, a furnace, an anvil, a shulker box and creative
/// clicks (the driver gives the items and places the blocks around the player). A step's
/// mispredictions are the slots (and the cursor) the server's packets changed after ghast's
/// prediction; crafting results the server sends after a grid change are not counted (the
/// real client receives them too).
fn menus(lab: &mut Lab) {
    let v = Version::V26_1;
    let mut total = (0usize, 0usize, 0usize); // clicks, predicted, mispredicted clicks
    let mut step = |lab: &mut Lab, label: &str, click: Option<Click>, ignore: &[usize]| {
        let Some(click) = click else {
            println!("{label:40} skipped (setup)");
            return;
        };
        lab.inv.tick();
        lab.inv.tick();
        let out = match lab.inv.click(click) {
            Ok(o) => o,
            Err(e) => {
                println!("{label:40} refused: {e}");
                return;
            }
        };
        let Outgoing::ContainerClick(c) = &out else {
            panic!("{out:?}")
        };
        let (predicted, swings, changed) = (c.predicted, c.swings, c.changed.len());
        let before = snapshot(&lab.inv);
        lab.send_out(&out);
        lab.corrections = 0;
        lab.pump(700);
        let after = snapshot(&lab.inv);
        let mut diff: Vec<String> = Vec::new();
        for (i, (a, b)) in before.0.iter().zip(&after.0).enumerate() {
            if a != b && !ignore.contains(&i) {
                diff.push(format!("slot {i}"));
            }
        }
        if before.1 != after.1 {
            diff.push("cursor".into());
        }
        total.0 += 1;
        total.1 += usize::from(predicted);
        if predicted && !diff.is_empty() {
            total.2 += 1;
        }
        println!(
            "{label:40} predicted {predicted:5} changed {changed:2} swings {swings} -> server changed {:?}",
            diff
        );
    };
    let find = |lab: &Lab, name: &str| lab.inv.find(|s| s.item_name(v) == Some(name));
    let empty = |lab: &Lab, from: usize, skip: &[usize]| {
        (from..lab.inv.slot_count())
            .filter(|i| !skip.contains(i))
            .find(|&i| lab.inv.slot(i).is_none())
            .map(|i| i as i16)
    };
    let drag =
        |slot: i16, stage: DragStage, kind: DragKind| Some(Click::QuickCraft { slot, stage, kind });
    lab.inv.tick();
    lab.inv.open_inventory().unwrap();

    // window 0: drags
    let stone = find(lab, "stone");
    step(
        lab,
        "w0: pickup 64 stone",
        stone.map(|slot| Click::Pickup { slot, right: false }),
        &[],
    );
    let e: Vec<i16> = (9..36)
        .filter(|&i| lab.inv.slot(i).is_none())
        .take(3)
        .map(|i| i as i16)
        .collect();
    step(
        lab,
        "w0: drag split start",
        drag(OUTSIDE, DragStage::Start, DragKind::Split),
        &[],
    );
    for &slot in &e {
        step(
            lab,
            "w0: drag split slot",
            drag(slot, DragStage::Slot, DragKind::Split),
            &[],
        );
    }
    step(
        lab,
        "w0: drag split end (3 x 21)",
        drag(OUTSIDE, DragStage::End, DragKind::Split),
        &[],
    );
    step(
        lab,
        "w0: pickup 21",
        e.first().map(|&slot| Click::Pickup { slot, right: false }),
        &[],
    );
    let e2: Vec<i16> = (9..36)
        .filter(|&i| lab.inv.slot(i).is_none())
        .take(2)
        .map(|i| i as i16)
        .collect();
    step(
        lab,
        "w0: drag one start",
        drag(OUTSIDE, DragStage::Start, DragKind::One),
        &[],
    );
    for &slot in &e2 {
        step(
            lab,
            "w0: drag one slot",
            drag(slot, DragStage::Slot, DragKind::One),
            &[],
        );
    }
    step(
        lab,
        "w0: drag one end",
        drag(OUTSIDE, DragStage::End, DragKind::One),
        &[],
    );
    step(
        lab,
        "w0: double click gather",
        e.first().map(|&slot| Click::PickupAll { slot }),
        &[],
    );
    step(
        lab,
        "w0: put stone down",
        e.first().map(|&slot| Click::Pickup { slot, right: false }),
        &[],
    );

    // window 0: bundle
    let bundle = find(lab, "bundle");
    step(
        lab,
        "w0: pickup bundle",
        bundle.map(|slot| Click::Pickup { slot, right: false }),
        &[],
    );
    let pearl = find(lab, "ender_pearl");
    step(
        lab,
        "w0: bundle onto pearls (insert)",
        pearl.map(|slot| Click::Pickup { slot, right: false }),
        &[],
    );
    let egg = find(lab, "egg");
    step(
        lab,
        "w0: bundle onto eggs (insert)",
        egg.map(|slot| Click::Pickup { slot, right: false }),
        &[],
    );
    let e = empty(lab, 9, &[]);
    step(
        lab,
        "w0: bundle right-click empty (take one out)",
        e.map(|slot| Click::Pickup { slot, right: true }),
        &[],
    );
    let e = empty(lab, 9, &[]);
    step(
        lab,
        "w0: put bundle down",
        e.map(|slot| Click::Pickup { slot, right: false }),
        &[],
    );
    let bundle = find(lab, "bundle");
    step(
        lab,
        "w0: right-click bundle (take one)",
        bundle.map(|slot| Click::Pickup { slot, right: true }),
        &[],
    );
    let e = empty(lab, 9, &[]);
    step(
        lab,
        "w0: put the taken stack down",
        e.map(|slot| Click::Pickup { slot, right: false }),
        &[],
    );
    let pearl = find(lab, "ender_pearl");
    step(
        lab,
        "w0: pickup pearls",
        pearl.map(|slot| Click::Pickup { slot, right: false }),
        &[],
    );
    let bundle = find(lab, "bundle");
    step(
        lab,
        "w0: pearls onto bundle (insert)",
        bundle.map(|slot| Click::Pickup { slot, right: false }),
        &[],
    );
    let e = empty(lab, 9, &[]);
    step(
        lab,
        "w0: put the rest down",
        e.map(|slot| Click::Pickup { slot, right: false }),
        &[],
    );

    let p = lab.pos;
    let at = |dx: i32, dz: i32| BlockPos {
        x: p[0].floor() as i32 + dx,
        y: p[1].floor() as i32,
        z: p[2].floor() as i32 + dz,
    };
    let open = |lab: &mut Lab, pos: BlockPos| -> bool {
        lab.inv.tick();
        if let Ok(o) = lab.inv.close() {
            lab.send_out(&o);
        }
        lab.pump(300);
        lab.c.send(&sb::BlockPlace {
            hand: sb::BlockPlaceHand(0),
            location: pos,
            direction: 1,
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            world_border_hit: false,
            sequence: 2,
        });
        lab.c.send(&sb::ArmAnimation {
            hand: sb::ArmAnimationHand(0),
        });
        lab.pump(1000);
        match lab.inv.window() {
            Some(w) => {
                println!(
                    "opened window {} {:?} menu {:?}",
                    w.id(),
                    w.kind(),
                    w.menu_26_1()
                );
                true
            }
            None => {
                println!("no window opened at {pos:?}");
                false
            }
        }
    };

    // crafting table: planks -> crafting table; honey bottle -> sugar (glass bottle remains)
    if open(lab, at(2, 0)) {
        let planks = find(lab, "oak_planks");
        step(
            lab,
            "craft: pickup planks",
            planks.map(|slot| Click::Pickup { slot, right: false }),
            &[0],
        );
        for g in [1i16, 2, 4, 5] {
            step(
                lab,
                "craft: one plank into the grid",
                Some(Click::Pickup {
                    slot: g,
                    right: true,
                }),
                &[0],
            );
        }
        let e = empty(lab, 10, &[]);
        step(
            lab,
            "craft: put planks down",
            e.map(|slot| Click::Pickup { slot, right: false }),
            &[0],
        );
        println!(
            "  result: {:?}",
            lab.inv.slot(0).map(|s| (s.item_name(v), s.count()))
        );
        step(
            lab,
            "craft: take the result",
            Some(Click::Pickup {
                slot: 0,
                right: false,
            }),
            &[0],
        );
        let e = empty(lab, 10, &[]);
        step(
            lab,
            "craft: put the result down",
            e.map(|slot| Click::Pickup { slot, right: false }),
            &[0],
        );
        let honey = find(lab, "honey_bottle");
        step(
            lab,
            "craft: quick move honey into the grid",
            honey.map(|slot| Click::QuickMove { slot, right: false }),
            &[0],
        );
        println!(
            "  result: {:?}",
            lab.inv.slot(0).map(|s| (s.item_name(v), s.count()))
        );
        step(
            lab,
            "craft: shift-click the result (sugar)",
            Some(Click::QuickMove {
                slot: 0,
                right: false,
            }),
            &[0],
        );
        let bottle = (1..10)
            .find(|&i| lab.inv.slot(i).is_some())
            .map(|i| i as i16);
        step(
            lab,
            "craft: quick move the bottle out",
            bottle.map(|slot| Click::QuickMove { slot, right: false }),
            &[0],
        );
    }
    // furnace: shift-click into input and fuel (never both at once: nothing smelts)
    if open(lab, at(2, 2)) {
        let iron = find(lab, "raw_iron");
        step(
            lab,
            "furnace: quick move raw iron -> input",
            iron.map(|slot| Click::QuickMove { slot, right: false }),
            &[],
        );
        step(
            lab,
            "furnace: quick move input back",
            Some(Click::QuickMove {
                slot: 0,
                right: false,
            }),
            &[],
        );
        let coal = find(lab, "coal");
        step(
            lab,
            "furnace: quick move coal -> fuel",
            coal.map(|slot| Click::QuickMove { slot, right: false }),
            &[],
        );
        step(
            lab,
            "furnace: quick move fuel back",
            Some(Click::QuickMove {
                slot: 1,
                right: false,
            }),
            &[],
        );
        let bucket = find(lab, "bucket");
        step(
            lab,
            "furnace: pickup bucket",
            bucket.map(|slot| Click::Pickup { slot, right: false }),
            &[],
        );
        step(
            lab,
            "furnace: bucket into the fuel slot",
            Some(Click::Pickup {
                slot: 1,
                right: false,
            }),
            &[],
        );
        step(
            lab,
            "furnace: take the bucket",
            Some(Click::Pickup {
                slot: 1,
                right: false,
            }),
            &[],
        );
        let e = empty(lab, 3, &[]);
        step(
            lab,
            "furnace: put the bucket down",
            e.map(|slot| Click::Pickup { slot, right: false }),
            &[],
        );
    }
    // anvil: its result is computed by the client from data ghast does not model (unpredicted)
    if open(lab, at(0, 2)) {
        let sword = find(lab, "diamond_sword");
        step(
            lab,
            "anvil: quick move sword in (unpredicted)",
            sword.map(|slot| Click::QuickMove { slot, right: false }),
            &[],
        );
        step(
            lab,
            "anvil: quick move sword out (unpredicted)",
            Some(Click::QuickMove {
                slot: 0,
                right: false,
            }),
            &[],
        );
        let stone = find(lab, "stone");
        step(
            lab,
            "anvil: pickup stone",
            stone.map(|slot| Click::Pickup { slot, right: false }),
            &[],
        );
        let e = empty(lab, 3, &[]);
        step(
            lab,
            "anvil: put stone down",
            e.map(|slot| Click::Pickup { slot, right: false }),
            &[],
        );
    }
    // shulker box: shulker boxes do not go in
    if open(lab, at(3, 0)) {
        let boxed = find(lab, "red_shulker_box");
        step(
            lab,
            "shulker: quick move a shulker box (stays)",
            boxed.map(|slot| Click::QuickMove { slot, right: false }),
            &[],
        );
        step(
            lab,
            "shulker: pickup the shulker box",
            boxed.map(|slot| Click::Pickup { slot, right: false }),
            &[],
        );
        step(
            lab,
            "shulker: shulker box onto slot 0 (refused)",
            Some(Click::Pickup {
                slot: 0,
                right: false,
            }),
            &[],
        );
        step(
            lab,
            "shulker: put it back",
            boxed.map(|slot| Click::Pickup { slot, right: false }),
            &[],
        );
        let stone = find(lab, "stone");
        step(
            lab,
            "shulker: quick move stone in",
            stone.map(|slot| Click::QuickMove { slot, right: false }),
            &[],
        );
        step(
            lab,
            "shulker: drag-free double click",
            Some(Click::Pickup {
                slot: 0,
                right: false,
            }),
            &[],
        );
        let e = empty(lab, 27, &[]);
        step(
            lab,
            "shulker: put stone down",
            e.map(|slot| Click::Pickup { slot, right: false }),
            &[],
        );
        // creative: middle click clones, clone drags (the driver switches the game mode now)
        println!("waiting for creative mode");
        lab.pump(3000);
        let stone = find(lab, "stone");
        step(
            lab,
            "creative: clone stone",
            stone.map(|slot| Click::Clone { slot }),
            &[],
        );
        let e: Vec<i16> = (0..27)
            .filter(|&i| lab.inv.slot(i).is_none())
            .take(2)
            .map(|i| i as i16)
            .collect();
        step(
            lab,
            "creative: drag clone start",
            drag(OUTSIDE, DragStage::Start, DragKind::Clone),
            &[],
        );
        for &slot in &e {
            step(
                lab,
                "creative: drag clone slot",
                drag(slot, DragStage::Slot, DragKind::Clone),
                &[],
            );
        }
        step(
            lab,
            "creative: drag clone end",
            drag(OUTSIDE, DragStage::End, DragKind::Clone),
            &[],
        );
        step(
            lab,
            "creative: drop the cursor outside",
            Some(Click::Pickup {
                slot: OUTSIDE,
                right: false,
            }),
            &[],
        );
        let e = e.first().copied();
        step(
            lab,
            "creative: Q a cloned stack",
            e.map(|slot| Click::Drop { slot, all: true }),
            &[],
        );
        lab.inv.tick();
        if let Ok(o) = lab.inv.close() {
            lab.send_out(&o);
        }
        lab.pump(500);
    }
    println!(
        "RESULT MENUS: {} clicks, {} predicted, {} predicted clicks the server corrected",
        total.0, total.1, total.2
    );
}

fn write_capture(path: Option<&str>, lab: &mut Lab) {
    if let (Some(path), Some(buf)) = (path, lab.c.capture.take()) {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(&buf).unwrap();
        let data = z.finish().unwrap();
        std::fs::write(path, &data).unwrap();
        println!("capture: {} bytes -> {path}", data.len());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let addr = args
        .first()
        .map_or("127.0.0.1:25565", String::as_str)
        .to_string();
    let name = args
        .get(1)
        .map_or("ghast_probe", String::as_str)
        .to_string();
    let (_, port) = addr.rsplit_once(':').expect("host:port");
    let port: u16 = port.parse().expect("port");

    let tcp = TcpStream::connect(&addr).expect("connect");
    tcp.set_nodelay(true).unwrap();
    tcp.set_read_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    let capture = args.get(2).cloned();
    let mut c = Conn {
        tcp,
        dec: FrameDecoder::new(),
        enc: FrameEncoder::new(),
        state: 2,
        capture: capture.as_ref().map(|_| BytesMut::new()),
    };
    c.send(&handshaking::serverbound::SetProtocol {
        protocol_version: 775,
        server_host: "localhost".into(),
        server_port: port,
        next_state: 2,
    });
    c.send(&login::serverbound::LoginStart {
        username: name.as_str().into(),
        player_uuid: Uuid::from_u128(0x6768_6173_745f_7465_7374_0000_0000_0002),
    });
    let mut reg = HashRegistries::default();
    // login + configuration
    let mut state = 2;
    while state != 4 {
        let Some(raw) = c.recv() else { continue };
        if state == 2 {
            use login::clientbound::ClientboundLoginPacket as L;
            match L::decode_raw(&raw).expect("login") {
                L::Compress(t) => {
                    c.dec.set_compression(t.threshold);
                    c.enc.set_compression(t.threshold);
                }
                L::Success(_) => {
                    c.send(&login::serverbound::LoginAcknowledged {});
                    state = 3;
                    c.send(&configuration::serverbound::Settings {
                        locale: "en_us".into(),
                        view_distance: 2,
                        chat_flags: 0,
                        chat_colors: true,
                        skin_parts: 0x7f,
                        main_hand: 1,
                        enable_text_filtering: false,
                        enable_server_listing: true,
                        particle_status: configuration::serverbound::SettingsParticleStatus(0),
                    });
                }
                L::Disconnect(d) => panic!("login disconnect: {}", d.reason),
                _ => {}
            }
        } else {
            use configuration::clientbound::ClientboundConfigurationPacket as C;
            use configuration::serverbound as csb;
            match C::decode_raw(&raw).expect("configuration") {
                C::SelectKnownPacks(k) => {
                    let packs = k
                        .packs
                        .into_iter()
                        .map(|p| csb::SelectKnownPacksPacks {
                            namespace: p.namespace,
                            id: p.id,
                            version: p.version,
                        })
                        .collect();
                    c.send(&csb::SelectKnownPacks { packs });
                }
                C::RegistryData(r) => {
                    reg.add(&r.id, r.entries.iter().map(|e| &*e.key));
                    c.record_registry(&raw);
                }
                C::KeepAlive(k) => c.send(&csb::KeepAlive {
                    keep_alive_id: k.keep_alive_id,
                }),
                C::Ping(p) => c.send(&csb::Pong { id: p.id }),
                C::FinishConfiguration(_) => {
                    c.send(&csb::FinishConfiguration {});
                    state = 4;
                    c.state = 4;
                }
                _ => {}
            }
        }
    }
    println!(
        "in play; {} enchantments in the registry",
        reg.enchantments().len()
    );
    let mut inv = Inventory::new(Version::V26_1).unwrap();
    inv.set_registries_26_1(reg.clone());
    // each step waits 700 ms for corrections: one quiet tick settles a click
    inv.set_click_settle_ticks(1);
    let mut lab = Lab {
        c,
        inv,
        pos: [0.0; 3],
        corrections: 0,
        log_corrections: true,
    };
    // The inventory reset on login drops the registries: set them again after login.
    lab.pump(2000);
    lab.inv.set_registries_26_1(reg);
    if std::env::var("MENUS").is_ok() {
        println!("waiting for items");
        lab.pump(9000);
        dump(&lab.inv);
        menus(&mut lab);
        write_capture(capture.as_deref(), &mut lab);
        return;
    }
    if let Ok(n) = std::env::var("HASH_BATCHES") {
        hash_batches(&mut lab, n.parse().unwrap_or(1));
        write_capture(capture.as_deref(), &mut lab);
        return;
    }
    // Wait for the console to give items (the driver script gives them now).
    println!("waiting for items");
    lab.pump(9000);
    dump(&lab.inv);
    lab.inv.tick();
    lab.inv.open_inventory().unwrap();
    let mut ok = 0;
    let mut total = 0;
    let mut run = |lab: &mut Lab, label: &str, click: Option<Click>| {
        let Some(click) = click else {
            println!("{label:32} skipped (setup)");
            return;
        };
        total += 1;
        if lab.step(label, click) {
            ok += 1;
        }
    };
    let stone = lab.find("stone");
    run(
        &mut lab,
        "pickup stone stack",
        stone.map(|slot| Click::Pickup { slot, right: false }),
    );
    let e = lab.empty_slot(9);
    run(
        &mut lab,
        "place one (right)",
        e.map(|slot| Click::Pickup { slot, right: true }),
    );
    run(
        &mut lab,
        "place rest (merge)",
        e.map(|slot| Click::Pickup { slot, right: false }),
    );
    run(
        &mut lab,
        "take half (right)",
        e.map(|slot| Click::Pickup { slot, right: true }),
    );
    run(
        &mut lab,
        "drop cursor outside",
        Some(Click::Pickup {
            slot: OUTSIDE,
            right: false,
        }),
    );
    run(
        &mut lab,
        "quick move main->hotbar",
        e.map(|slot| Click::QuickMove { slot, right: false }),
    );
    let sword = lab.find("diamond_sword");
    run(
        &mut lab,
        "pickup sword (components)",
        sword.map(|slot| Click::Pickup { slot, right: false }),
    );
    let e2 = lab.empty_slot(9);
    run(
        &mut lab,
        "place sword",
        e2.map(|slot| Click::Pickup { slot, right: false }),
    );
    let sword = lab.find("diamond_sword");
    run(
        &mut lab,
        "swap sword with offhand",
        sword.map(|slot| Click::Swap { slot, hotbar: 40 }),
    );
    run(
        &mut lab,
        "swap offhand back",
        Some(Click::Swap {
            slot: 45,
            hotbar: 40,
        }),
    );
    let chest = lab.find("iron_chestplate");
    run(
        &mut lab,
        "quick move chestplate->armor",
        chest.map(|slot| Click::QuickMove { slot, right: false }),
    );
    run(
        &mut lab,
        "quick move armor->inventory",
        Some(Click::QuickMove {
            slot: 6,
            right: false,
        }),
    );
    let pearl = lab.find("ender_pearl");
    run(
        &mut lab,
        "drop one pearl (Q)",
        pearl.map(|slot| Click::Drop { slot, all: false }),
    );
    let log = lab.find("oak_log");
    run(
        &mut lab,
        "drop all logs (ctrl+Q)",
        log.map(|slot| Click::Drop { slot, all: true }),
    );
    let pick = lab.find("diamond_pickaxe");
    run(
        &mut lab,
        "pickup pickaxe (repair/glint)",
        pick.map(|slot| Click::Pickup { slot, right: false }),
    );
    run(
        &mut lab,
        "put pickaxe back",
        pick.map(|slot| Click::Pickup { slot, right: false }),
    );
    let book = lab.find("enchanted_book");
    run(
        &mut lab,
        "number key swap book->hotbar 8",
        book.map(|slot| Click::Swap { slot, hotbar: 8 }),
    );
    let named = lab.find("paper");
    run(
        &mut lab,
        "pickup named paper (lore)",
        named.map(|slot| Click::Pickup { slot, right: false }),
    );
    run(
        &mut lab,
        "put paper back",
        named.map(|slot| Click::Pickup { slot, right: false }),
    );
    let stone = lab.find("stone");
    run(
        &mut lab,
        "pickup stone again",
        stone.map(|slot| Click::Pickup { slot, right: false }),
    );
    run(
        &mut lab,
        "double click gather",
        stone.map(|slot| Click::PickupAll { slot }),
    );
    run(
        &mut lab,
        "put stone down",
        stone.map(|slot| Click::Pickup { slot, right: false }),
    );

    // A component stack size above 64 (the 99 container cap) and a cursed helmet.
    let sticks = lab.find("stick");
    run(
        &mut lab,
        "pickup 99 sticks (max 99)",
        sticks.map(|slot| Click::Pickup { slot, right: false }),
    );
    let e = lab.empty_slot(9);
    run(
        &mut lab,
        "place 99 sticks",
        e.map(|slot| Click::Pickup { slot, right: false }),
    );
    run(
        &mut lab,
        "split 99 sticks (right)",
        e.map(|slot| Click::Pickup { slot, right: true }),
    );
    run(
        &mut lab,
        "merge sticks back",
        e.map(|slot| Click::Pickup { slot, right: false }),
    );
    let helmet = lab.find("golden_helmet");
    run(
        &mut lab,
        "quick move cursed helmet->head",
        helmet.map(|slot| Click::QuickMove { slot, right: false }),
    );
    run(
        &mut lab,
        "quick move cursed helmet out",
        Some(Click::QuickMove {
            slot: 5,
            right: false,
        }),
    );
    run(
        &mut lab,
        "pickup cursed helmet (refused)",
        Some(Click::Pickup {
            slot: 5,
            right: false,
        }),
    );
    // A bundle runs its own click rules: ghast sends it unpredicted; the server's
    // corrections are expected here.
    lab.log_corrections = false;
    let bundle = lab.find("bundle");
    run(
        &mut lab,
        "pickup bundle",
        bundle.map(|slot| Click::Pickup { slot, right: false }),
    );
    let pearl = lab.find("ender_pearl");
    run(
        &mut lab,
        "bundle onto pearls (unpredicted)",
        pearl.map(|slot| Click::Pickup { slot, right: false }),
    );
    lab.pump(500);
    println!(
        "  after the corrections: cursor {:?}, pearls {:?}",
        lab.inv.cursor().map(|c| (
            c.item_name(Version::V26_1),
            c.count(),
            *c.data() != ghast_inventory::ItemData::None
        )),
        lab.find("ender_pearl")
            .and_then(|i| lab.inv.slot(i as usize))
            .map(|s| s.count())
    );
    let e = lab.empty_slot(9);
    run(
        &mut lab,
        "put bundle down",
        e.map(|slot| Click::Pickup { slot, right: false }),
    );
    lab.log_corrections = true;

    // A chest next to the player (placed by the driver script at +2 x).
    lab.inv.tick();
    if let Ok(o) = lab.inv.close() {
        lab.send_out(&o);
    }
    let p = lab.pos;
    let chest_pos = BlockPos {
        x: p[0].floor() as i32 + 2,
        y: p[1].floor() as i32,
        z: p[2].floor() as i32,
    };
    lab.pump(200);
    lab.c.send(&sb::BlockPlace {
        hand: sb::BlockPlaceHand(0),
        location: chest_pos,
        direction: 1,
        cursor_x: 0.5,
        cursor_y: 1.0,
        cursor_z: 0.5,
        inside_block: false,
        world_border_hit: false,
        sequence: 1,
    });
    lab.c.send(&sb::ArmAnimation {
        hand: sb::ArmAnimationHand(0),
    });
    lab.pump(1000);
    match lab.inv.window() {
        Some(w) => println!("opened window {} {:?} \"{}\"", w.id(), w.kind(), w.title()),
        None => println!("no chest window opened"),
    }
    if lab.inv.window().is_some() {
        let n = lab.inv.container_len();
        let stone = lab.find("stone");
        run(
            &mut lab,
            "chest: quick move stone in",
            stone.map(|slot| Click::QuickMove { slot, right: false }),
        );
        let sword = lab.find("diamond_sword");
        run(
            &mut lab,
            "chest: quick move sword in",
            sword.map(|slot| Click::QuickMove { slot, right: false }),
        );
        run(
            &mut lab,
            "chest: pickup slot 0",
            Some(Click::Pickup {
                slot: 0,
                right: false,
            }),
        );
        let e = (1..n)
            .find(|&i| lab.inv.slot(i).is_none())
            .map(|i| i as i16);
        run(
            &mut lab,
            "chest: place in empty",
            e.map(|slot| Click::Pickup { slot, right: false }),
        );
        run(
            &mut lab,
            "chest: quick move back out",
            e.map(|slot| Click::QuickMove { slot, right: false }),
        );
        lab.inv.tick();
        if let Ok(o) = lab.inv.close() {
            lab.send_out(&o);
        }
        lab.pump(500);
    }
    dump(&lab.inv);
    println!("RESULT: {ok} of {total} clicks without server corrections");
    write_capture(capture.as_deref(), &mut lab);
    let _: Option<Stack> = None;
}
