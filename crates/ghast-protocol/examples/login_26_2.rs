//! Acceptance test for protocol 776 (26.2) using only `ghast-protocol` types:
//! status ping, offline login, the configuration state, then play — answering keep-alives,
//! pings, teleports and chunk batches — for a few seconds.
//!
//! ```text
//! cargo run -p ghast-protocol --example login_26_2 -- 127.0.0.1:25565 [name] [seconds] [capture.bin]
//! ```
//!
//! With a capture path, every packet of the login connection (both directions, after
//! decompression) is written to that file, zlib-compressed, for the decode tests
//! (`tests/capture_26_2.rs`). Record format: `u8 state` (0 handshaking, 1 status, 2 login,
//! 3 configuration, 4 play), `u8 direction` (0 clientbound, 1 serverbound), VarInt id,
//! VarInt body length, body.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use bytes::{BufMut, BytesMut};
use ghast_protocol::codec::McWrite;
use ghast_protocol::types::Uuid;
use ghast_protocol::v26_2::{configuration, handshaking, login, play, status};
use ghast_protocol::{FrameDecoder, FrameEncoder, Packet, PacketSet, RawPacket, chat};

const PROTOCOL: i32 = 776;

struct Conn {
    tcp: TcpStream,
    dec: FrameDecoder,
    enc: FrameEncoder,
    state: u8,
    capture: Option<BytesMut>,
}

impl Conn {
    fn connect(addr: &str) -> std::io::Result<Self> {
        let tcp = TcpStream::connect(addr)?;
        tcp.set_nodelay(true)?;
        tcp.set_read_timeout(Some(Duration::from_millis(50)))?;
        Ok(Conn {
            tcp,
            dec: FrameDecoder::new(),
            enc: FrameEncoder::new(),
            state: 0,
            capture: None,
        })
    }

    fn record(&mut self, dir: u8, raw: &RawPacket) {
        if let Some(c) = &mut self.capture {
            c.put_u8(self.state);
            c.put_u8(dir);
            c.put_varint(raw.id);
            c.put_varint(raw.body.len() as i32);
            c.put_slice(&raw.body);
        }
    }

    fn send<P: Packet>(&mut self, p: &P) {
        let raw = RawPacket::from_packet(p).expect("encode");
        self.record(1, &raw);
        self.enc.write_raw(&raw).expect("frame");
        let out = self.enc.take();
        self.tcp.write_all(&out).expect("write");
    }

    /// Next packet, or `None` on a read timeout.
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

fn status_ping(addr: &str, host: &str, port: u16) {
    let mut c = Conn::connect(addr).expect("connect");
    c.send(&handshaking::serverbound::SetProtocol {
        protocol_version: PROTOCOL,
        server_host: host.into(),
        server_port: port,
        next_state: 1,
    });
    c.state = 1;
    c.send(&status::serverbound::PingStart {});
    let t0 = Instant::now();
    loop {
        let Some(raw) = c.recv() else {
            assert!(t0.elapsed() < Duration::from_secs(5), "no status response");
            continue;
        };
        match status::clientbound::ClientboundStatusPacket::decode_raw(&raw).expect("decode") {
            status::clientbound::ClientboundStatusPacket::ServerInfo(i) => {
                println!("status: {}", i.response);
                c.send(&status::serverbound::Ping { time: 1234 });
            }
            status::clientbound::ClientboundStatusPacket::Ping(p) => {
                assert_eq!(p.time, 1234);
                println!("status ping ok ({:?})", t0.elapsed());
                return;
            }
        }
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
    let seconds: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let capture = args.get(3).cloned();
    let (host, port) = addr.rsplit_once(':').expect("host:port");
    let port: u16 = port.parse().expect("port");

    status_ping(&addr, host, port);

    let mut c = Conn::connect(&addr).expect("connect");
    if capture.is_some() {
        c.capture = Some(BytesMut::new());
    }
    // Recorded as "localhost": the fixture must not depend on the address used.
    c.send(&handshaking::serverbound::SetProtocol {
        protocol_version: PROTOCOL,
        server_host: "localhost".into(),
        server_port: port,
        next_state: 2,
    });
    c.state = 2;
    // Offline-mode UUID derivation is the server's business; any UUID is accepted here.
    c.send(&login::serverbound::LoginStart {
        username: name.as_str().into(),
        player_uuid: Uuid::from_u128(0x6768_6173_745f_7465_7374_0000_0000_0001),
    });

    let mut counts = std::collections::BTreeMap::<&'static str, usize>::new();
    let start = Instant::now();
    let mut play_since: Option<Instant> = None;
    let mut keepalives = 0;
    loop {
        if let Some(t) = play_since
            && t.elapsed() > Duration::from_secs(seconds)
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds + 30),
            "did not reach play in time"
        );
        let Some(raw) = c.recv() else { continue };
        match c.state {
            2 => {
                use login::clientbound::ClientboundLoginPacket as L;
                let p = L::decode_raw(&raw).expect("login decode");
                *counts.entry(p.name()).or_default() += 1;
                match p {
                    L::Compress(t) => {
                        c.dec.set_compression(t.threshold);
                        c.enc.set_compression(t.threshold);
                    }
                    L::Success(s) => {
                        println!("login success: {} {}", s.username, s.uuid);
                        c.send(&login::serverbound::LoginAcknowledged {});
                        c.state = 3;
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
                    L::EncryptionBegin(_) => panic!("server is in online mode"),
                    other => println!("login: {}", other.name()),
                }
            }
            3 => {
                use configuration::clientbound::ClientboundConfigurationPacket as C;
                use configuration::serverbound as sb;
                let p = C::decode_raw(&raw).expect("configuration decode");
                *counts.entry(p.name()).or_default() += 1;
                match p {
                    C::SelectKnownPacks(k) => {
                        // Claim the same packs: the server then sends registry entries
                        // without their (vanilla) data.
                        let packs = k
                            .packs
                            .into_iter()
                            .map(|p| sb::SelectKnownPacksPacks {
                                namespace: p.namespace,
                                id: p.id,
                                version: p.version,
                            })
                            .collect();
                        c.send(&sb::SelectKnownPacks { packs });
                    }
                    C::KeepAlive(k) => c.send(&sb::KeepAlive {
                        keep_alive_id: k.keep_alive_id,
                    }),
                    C::Ping(p) => c.send(&sb::Pong { id: p.id }),
                    C::FinishConfiguration(_) => {
                        c.send(&sb::FinishConfiguration {});
                        c.state = 4;
                        play_since = Some(Instant::now());
                        println!("configuration done");
                    }
                    C::Disconnect(d) => {
                        panic!(
                            "configuration disconnect: {}",
                            chat::nbt_to_plain_text(&d.reason)
                        )
                    }
                    _ => {}
                }
            }
            _ => {
                use play::clientbound::ClientboundPlayPacket as P;
                use play::serverbound as sb;
                let p = match P::decode_raw(&raw) {
                    Ok(p) => p,
                    Err(e) => panic!(
                        "play decode of {:#04x} ({:?}): {e}",
                        raw.id,
                        P::name_of(raw.id)
                    ),
                };
                *counts.entry(p.name()).or_default() += 1;
                match p {
                    P::KeepAlive(k) => {
                        keepalives += 1;
                        c.send(&sb::KeepAlive {
                            keep_alive_id: k.keep_alive_id,
                        });
                    }
                    P::Ping(p) => c.send(&sb::Pong { id: p.id }),
                    P::Position(pos) => {
                        c.send(&sb::TeleportConfirm {
                            teleport_id: pos.teleport_id,
                        });
                        c.send(&sb::PositionLook {
                            x: pos.x,
                            y: pos.y,
                            z: pos.z,
                            yaw: pos.yaw,
                            pitch: pos.pitch,
                            flags: play::serverbound::MovementFlags::ON_GROUND,
                        });
                        c.send(&sb::PlayerLoaded {});
                    }
                    P::ChunkBatchFinished(_) => c.send(&sb::ChunkBatchReceived {
                        chunks_per_tick: 20.0,
                    }),
                    P::StartConfiguration(_) => {
                        c.send(&sb::ConfigurationAcknowledged {});
                        c.state = 3;
                    }
                    P::KickDisconnect(d) => {
                        panic!("kicked: {}", chat::nbt_to_plain_text(&d.reason))
                    }
                    P::SystemChat(m) => println!("chat: {}", chat::nbt_to_plain_text(&m.content)),
                    _ => {}
                }
            }
        }
    }
    println!("play for {seconds} s, {keepalives} keep-alives answered");
    for (k, v) in &counts {
        println!("  {k}: {v}");
    }
    if let (Some(path), Some(buf)) = (capture, c.capture.take()) {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(&buf).unwrap();
        let data = z.finish().unwrap();
        std::fs::write(&path, &data).unwrap();
        println!(
            "capture: {} bytes raw, {} compressed -> {path}",
            buf.len(),
            data.len()
        );
    }
}
