//! Live check against a real server: a minimal offline-mode client that only stays connected
//! (keep-alive, transactions, teleport confirms) and feeds every play packet to the tracker.
//! Every second it writes a snapshot of the tracked entities to `--out`.
//!
//! ```text
//! cargo run -p ghast-entity --example live_probe -- --port 25618 --version 1.8 \
//!     --user GhastBot3 --secs 60 --out /tmp/entities.txt
//! ```
//!
//! Snapshot lines: `id type name x y z vx vy vz yaw pitch on_ground vehicle passengers`
//! (`name` = custom name from metadata or `-`; positions with `{:?}` so they round-trip).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use ghast_entity::{Config, Entities, Entity};
use ghast_protocol::{FrameDecoder, FrameEncoder, RawPacket};

fn arg(name: &str, default: &str) -> String {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| default.to_string())
}

enum T {
    V18(ghast_entity::v1_8::Tracker),
    V16(ghast_entity::v1_16_5::Tracker),
}

impl T {
    fn store(&self) -> &Entities {
        match self {
            T::V18(t) => t,
            T::V16(t) => t,
        }
    }
}

fn name(t: &T, e: &Entity) -> String {
    let n = match t {
        T::V18(_) => ghast_entity::v1_8::custom_name(e).map(str::to_string),
        T::V16(_) => ghast_entity::v1_16_5::custom_name(e).map(|j| {
            // {"text":"x"} -> x
            j.split("\"text\":\"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .unwrap_or(j)
                .to_string()
        }),
    };
    n.filter(|s| !s.is_empty()).unwrap_or_else(|| "-".into())
}

fn main() {
    let host = arg("--host", "127.0.0.1");
    let port: u16 = arg("--port", "25618").parse().unwrap();
    let modern = arg("--version", "1.8") != "1.8";
    let user = arg("--user", "GhastBot3");
    let secs: u64 = arg("--secs", "60").parse().unwrap();
    let out = arg("--out", "/tmp/entities.txt");

    let mut sock = TcpStream::connect((host.as_str(), port)).unwrap();
    sock.set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let mut dec = FrameDecoder::new();
    let mut enc = FrameEncoder::new();

    // Handshake + login start (same layout in both versions).
    let proto = if modern { 754 } else { 47 };
    enc.write_packet(
        &ghast_protocol::v1_8::handshaking::serverbound::SetProtocol {
            protocol_version: proto,
            server_host: host.clone().into(),
            server_port: port,
            next_state: 2,
        },
    )
    .unwrap();
    enc.write_packet(&ghast_protocol::v1_8::login::serverbound::LoginStart {
        username: user.clone().into(),
    })
    .unwrap();
    sock.write_all(&enc.take()).unwrap();

    let mut tracker = if modern {
        T::V16(ghast_entity::v1_16_5::Tracker::new(Config::default()))
    } else {
        T::V18(ghast_entity::v1_8::Tracker::new(Config::default()))
    };
    let mut play = false;
    let mut buf = vec![0u8; 1 << 16];
    let start = Instant::now();
    let mut last_dump = Instant::now();
    let mut counts = (0u64, 0u64);
    while start.elapsed() < Duration::from_secs(secs) {
        match sock.read(&mut buf) {
            Ok(0) => {
                eprintln!("server closed the connection");
                break;
            }
            Ok(n) => dec.push(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => panic!("{e}"),
        }
        while let Some(raw) = dec.next_packet().unwrap() {
            if !play {
                match raw.id {
                    0x03 => {
                        let c: ghast_protocol::v1_8::login::clientbound::Compress =
                            raw.decode_as().unwrap();
                        dec.set_compression(c.threshold);
                        enc.set_compression(c.threshold);
                    }
                    0x02 => {
                        play = true;
                        eprintln!("logged in as {user}");
                    }
                    0x00 => panic!("login disconnect: {:?}", raw.body),
                    _ => {}
                }
                continue;
            }
            counts.0 += 1;
            let r = match &mut tracker {
                T::V18(t) => t.apply(&raw),
                T::V16(t) => t.apply(&raw),
            };
            match r {
                Ok(ghast_entity::Applied::NotEntityPacket) => {}
                Ok(_) => counts.1 += 1,
                Err(e) => eprintln!("tracker error on packet {:#x}: {e}", raw.id),
            }
            respond(modern, &raw, &mut enc);
        }
        let pending = enc.take();
        if !pending.is_empty() {
            sock.write_all(&pending).unwrap();
        }
        if last_dump.elapsed() >= Duration::from_secs(1) {
            last_dump = Instant::now();
            dump(&tracker, &out, counts);
        }
    }
    dump(&tracker, &out, counts);
}

fn dump(t: &T, path: &str, counts: (u64, u64)) {
    let s = t.store();
    let mut lines = vec![format!(
        "# packets {} entity-packets {}",
        counts.0, counts.1
    )];
    let mut es: Vec<&Entity> = s.iter().collect();
    es.sort_by_key(|e| e.id());
    for e in es {
        let p = e.position();
        let v = e.velocity();
        lines.push(format!(
            "{} {:?} {} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {} {:?} {:?}",
            e.id(),
            e.entity_type(),
            name(t, e),
            p[0],
            p[1],
            p[2],
            v[0],
            v[1],
            v[2],
            e.yaw(),
            e.pitch(),
            e.on_ground(),
            e.vehicle(),
            e.passengers()
        ));
    }
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, lines.join("\n") + "\n").unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

/// Keep-alive, transaction and teleport answers so the server keeps us connected.
fn respond(modern: bool, raw: &RawPacket, enc: &mut FrameEncoder) {
    if modern {
        use ghast_protocol::v1_16_5::play::{clientbound as cb, serverbound as sb};
        match raw.id {
            cb::id::KEEP_ALIVE => {
                let k: cb::KeepAlive = raw.decode_as().unwrap();
                enc.write_packet(&sb::KeepAlive {
                    keep_alive_id: k.keep_alive_id,
                })
                .unwrap();
            }
            cb::id::TRANSACTION => {
                let t: cb::Transaction = raw.decode_as().unwrap();
                enc.write_packet(&sb::Transaction {
                    window_id: t.window_id,
                    action: t.action,
                    accepted: true,
                })
                .unwrap();
            }
            cb::id::POSITION => {
                let p: cb::Position = raw.decode_as().unwrap();
                enc.write_packet(&sb::TeleportConfirm {
                    teleport_id: p.teleport_id,
                })
                .unwrap();
                if p.flags == 0 {
                    enc.write_packet(&sb::PositionLook {
                        x: p.x,
                        y: p.y,
                        z: p.z,
                        yaw: p.yaw,
                        pitch: p.pitch,
                        on_ground: false,
                    })
                    .unwrap();
                }
            }
            _ => {}
        }
    } else {
        use ghast_protocol::v1_8::play::{clientbound as cb, serverbound as sb};
        match raw.id {
            cb::id::KEEP_ALIVE => {
                let k: cb::KeepAlive = raw.decode_as().unwrap();
                enc.write_packet(&sb::KeepAlive {
                    keep_alive_id: k.keep_alive_id,
                })
                .unwrap();
            }
            cb::id::TRANSACTION => {
                let t: cb::Transaction = raw.decode_as().unwrap();
                enc.write_packet(&sb::Transaction {
                    window_id: t.window_id,
                    action: t.action,
                    accepted: true,
                })
                .unwrap();
            }
            cb::id::POSITION => {
                let p: cb::Position = raw.decode_as().unwrap();
                if p.flags == 0 {
                    enc.write_packet(&sb::PositionLook {
                        x: p.x,
                        y: p.y,
                        z: p.z,
                        yaw: p.yaw,
                        pitch: p.pitch,
                        on_ground: false,
                    })
                    .unwrap();
                }
            }
            _ => {}
        }
    }
}
