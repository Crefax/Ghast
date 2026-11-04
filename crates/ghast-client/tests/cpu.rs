//! CPU cost of one bot against a busy fake 1.8 server (ignored by default; run with
//! `cargo test -p ghast-client --release --test cpu -- --ignored --nocapture`).
//!
//! The server sends per tick: one Grim-style transaction (answered), 40 entity moves and 4
//! entity velocities (filtered / decoded and dropped), a time update every second and a
//! keep-alive every 15 s. The client runs on its own `current_thread` runtime thread; its
//! CPU time is read from `/proc/thread-self/stat`.

use std::time::{Duration, Instant};

use bytes::{BufMut, BytesMut};
use ghast_client::connect::TargetAddr;
use ghast_client::{Account, Client, GameVersion, Idle, SessionEnd};
use ghast_protocol::types::ByteString;
use ghast_protocol::v1_8::play::clientbound as cb;
use ghast_protocol::{FrameDecoder, FrameEncoder, RawPacket};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn thread_cpu() -> Duration {
    let s = std::fs::read_to_string("/proc/thread-self/stat").unwrap_or_default();
    // Fields after the parenthesised command name; utime and stime are fields 14 and 15.
    let rest = s.rsplit_once(')').map_or("", |(_, r)| r);
    // Token 0 is field 3 (state), so utime (14) and stime (15) are tokens 11 and 12.
    let f: Vec<&str> = rest.split_whitespace().collect();
    let field = |i: usize| f.get(i).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0);
    let ticks = field(11) + field(12);
    Duration::from_millis(ticks * 10)
}

#[test]
#[ignore = "measurement, run explicitly"]
fn busy_server_cpu() {
    let secs = 20;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = TargetAddr::from(listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            listener.set_nonblocking(true).unwrap();
            let l = tokio::net::TcpListener::from_std(listener).unwrap();
            let (mut s, _) = l.accept().await.unwrap();
            let mut dec = FrameDecoder::new();
            let mut enc = FrameEncoder::new();
            let mut buf = vec![0u8; 65536];
            // Handshake + login start.
            let mut got = 0;
            while got < 2 {
                let n = s.read(&mut buf).await.unwrap();
                dec.push(&buf[..n]);
                while dec.next_packet().unwrap().is_some() {
                    got += 1;
                }
            }
            enc.write_packet(&ghast_protocol::v1_8::login::clientbound::Compress { threshold: 256 })
                .unwrap();
            enc.set_compression(256);
            enc.write_packet(&ghast_protocol::v1_8::login::clientbound::Success {
                uuid: ByteString::from("00000000-0000-3000-8000-000000000000"),
                username: ByteString::from("CpuBot"),
            })
            .unwrap();
            enc.write_packet(&cb::Login {
                entity_id: 1,
                game_mode: 0,
                dimension: 0,
                difficulty: 1,
                max_players: 20,
                level_type: ByteString::from("default"),
                reduced_debug_info: false,
            })
            .unwrap();
            let mut d = BytesMut::new();
            for i in 0..4096 {
                d.put_u16_le(if i >> 8 == 15 { 1 << 4 } else { 0 });
            }
            d.put_bytes(0, 4096 + 256);
            enc.write_packet(&cb::MapChunk {
                x: 0,
                z: 0,
                ground_up: true,
                bit_map: 1 << 3,
                chunk_data: d.freeze(),
            })
            .unwrap();
            enc.write_packet(&cb::Position {
                x: 8.5,
                y: 64.0,
                z: 8.5,
                yaw: 0.0,
                pitch: 0.0,
                flags: 0,
            })
            .unwrap();
            s.write_all(&enc.take()).await.unwrap();
            let start = Instant::now();
            let mut tick = tokio::time::interval(Duration::from_millis(50));
            let mut n: u64 = 0;
            let mut action: i16 = -1;
            while start.elapsed() < Duration::from_secs(secs) {
                tokio::select! {
                    _ = tick.tick() => {
                        n += 1;
                        enc.write_packet(&cb::Transaction { window_id: 0, action, accepted: false }).unwrap();
                        action = action.wrapping_sub(1);
                        for e in 0..40 {
                            // rel_entity_move: entity id, dx, dy, dz, on ground.
                            let mut b = BytesMut::new();
                            ghast_protocol::McWrite::put_varint(&mut b, 100 + e);
                            b.put_slice(&[1, 0, 255, 1]);
                            enc.write_raw(&RawPacket::new(cb::id::REL_ENTITY_MOVE, b.freeze())).unwrap();
                        }
                        for e in 0..4 {
                            enc.write_packet(&cb::EntityVelocity {
                                entity_id: 100 + e,
                                velocity: ghast_protocol::v1_8::types::Vec3i16 { x: 10, y: -20, z: 30 },
                            }).unwrap();
                        }
                        if n.is_multiple_of(20) {
                            enc.write_packet(&cb::UpdateTime { age: n as i64, time: n as i64 }).unwrap();
                        }
                        if n.is_multiple_of(300) {
                            enc.write_packet(&cb::KeepAlive { keep_alive_id: n as i32 }).unwrap();
                        }
                        if s.write_all(&enc.take()).await.is_err() { break; }
                    }
                    r = s.read(&mut buf) => {
                        if !matches!(r, Ok(n) if n > 0) { break; }
                    }
                }
            }
            enc.write_packet(&cb::KickDisconnect { reason: ByteString::from(r#"{"text":"done"}"#) })
                .unwrap();
            let _ = s.write_all(&enc.take()).await;
            n
        })
    });
    let client = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let c = Client::new(
            addr,
            GameVersion::V1_8_9,
            Account::offline("CpuBot").unwrap(),
        );
        let cpu0 = thread_cpu();
        let t0 = Instant::now();
        let end = rt.block_on(c.run(&mut Idle));
        (end, thread_cpu() - cpu0, t0.elapsed())
    });
    let ticks = server.join().unwrap();
    let (end, cpu, wall) = client.join().unwrap();
    assert!(matches!(end, Ok(SessionEnd::Kicked { .. })), "{end:?}");
    println!(
        "client thread: {:.1} ms CPU over {:.1} s wall = {:.3} % of one core ({ticks} server ticks, \
         ~{} inbound packets/s)",
        cpu.as_secs_f64() * 1000.0,
        wall.as_secs_f64(),
        100.0 * cpu.as_secs_f64() / wall.as_secs_f64(),
        45 * 20
    );
}
