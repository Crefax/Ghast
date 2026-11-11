//! Packet ids of protocol 775 against the vanilla server's own report.
//!
//! `tests/data/packets_775.tsv` pairs every (state, direction, id) of the 26.1.2 server's
//! `--reports` `packets.json` with the minecraft-data name used by the generated code (see
//! `codegen/protocol/src/report.rs`). This test checks the generated packet sets against
//! that table: same ids, same names, nothing missing on either side.
//!
//! When the report itself is in the git-ignored cache
//! (`.cache/mojang/26.1.2/reports/reports/packets.json`, made by
//! `java -DbundlerMainClass=net.minecraft.data.Main -jar server.jar --reports`), the table
//! is also checked against it.

#![cfg(feature = "v26_1")]

use ghast_protocol::PacketSet;
use ghast_protocol::v26_1::{configuration, handshaking, login, play, status};

const TABLE: &str = include_str!("data/packets_775.tsv");

struct Row<'a> {
    state: &'a str,
    dir: &'a str,
    id: i32,
    vanilla: &'a str,
    mcdata: &'a str,
}

fn rows() -> Vec<Row<'static>> {
    TABLE
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            Row {
                state: f[0],
                dir: f[1],
                id: f[2].parse().unwrap(),
                vanilla: f[3],
                mcdata: f[4],
            }
        })
        .collect()
}

fn set_of<S: PacketSet>(ids: &[i32]) -> Vec<(i32, &'static str)> {
    ids.iter().map(|&i| (i, S::name_of(i).unwrap())).collect()
}

#[test]
fn generated_ids_match_the_vanilla_report() {
    let rows = rows();
    for r in &rows {
        assert!(
            r.vanilla != "-" && r.mcdata != "-",
            "{} {} {:#04x}: only one side has this id (vanilla {}, minecraft-data {})",
            r.state,
            r.dir,
            r.id,
            r.vanilla,
            r.mcdata
        );
    }
    type Set = (&'static str, &'static str, Vec<(i32, &'static str)>);
    let sets: [Set; 9] = [
        (
            "handshaking",
            "serverbound",
            set_of::<handshaking::serverbound::ServerboundHandshakingPacket>(
                handshaking::serverbound::ServerboundHandshakingPacket::IDS,
            ),
        ),
        (
            "status",
            "clientbound",
            set_of::<status::clientbound::ClientboundStatusPacket>(
                status::clientbound::ClientboundStatusPacket::IDS,
            ),
        ),
        (
            "status",
            "serverbound",
            set_of::<status::serverbound::ServerboundStatusPacket>(
                status::serverbound::ServerboundStatusPacket::IDS,
            ),
        ),
        (
            "login",
            "clientbound",
            set_of::<login::clientbound::ClientboundLoginPacket>(
                login::clientbound::ClientboundLoginPacket::IDS,
            ),
        ),
        (
            "login",
            "serverbound",
            set_of::<login::serverbound::ServerboundLoginPacket>(
                login::serverbound::ServerboundLoginPacket::IDS,
            ),
        ),
        (
            "configuration",
            "clientbound",
            set_of::<configuration::clientbound::ClientboundConfigurationPacket>(
                configuration::clientbound::ClientboundConfigurationPacket::IDS,
            ),
        ),
        (
            "configuration",
            "serverbound",
            set_of::<configuration::serverbound::ServerboundConfigurationPacket>(
                configuration::serverbound::ServerboundConfigurationPacket::IDS,
            ),
        ),
        (
            "play",
            "clientbound",
            set_of::<play::clientbound::ClientboundPlayPacket>(
                play::clientbound::ClientboundPlayPacket::IDS,
            ),
        ),
        (
            "play",
            "serverbound",
            set_of::<play::serverbound::ServerboundPlayPacket>(
                play::serverbound::ServerboundPlayPacket::IDS,
            ),
        ),
    ];
    for (state, dir, ours) in &sets {
        let want: Vec<(i32, &str)> = rows
            .iter()
            .filter(|r| r.state == *state && r.dir == *dir)
            .map(|r| (r.id, r.mcdata))
            .collect();
        // minecraft-data's handshaking set also has the pre-Netty legacy ping (0xfe),
        // which is not a packet of the vanilla protocol state machine.
        let ours: Vec<(i32, &str)> = ours
            .iter()
            .copied()
            .filter(|&(id, n)| !(id == 0xfe && n == "legacy_server_list_ping"))
            .collect();
        assert_eq!(&ours, &want, "{state} {dir}");
    }
}

#[test]
fn table_matches_cached_report() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.cache/mojang/26.1.2/reports/reports/packets.json"
    );
    let Ok(text) = std::fs::read_to_string(path) else {
        eprintln!("skipped: no {path}");
        return;
    };
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut n = 0;
    for r in rows() {
        let state = if r.state == "handshaking" {
            "handshake"
        } else {
            r.state
        };
        let id = report[state][r.dir][r.vanilla]["protocol_id"].as_i64();
        assert_eq!(
            id,
            Some(i64::from(r.id)),
            "{} {} {}",
            r.state,
            r.dir,
            r.vanilla
        );
        n += 1;
    }
    let total: usize = report
        .as_object()
        .unwrap()
        .values()
        .flat_map(|s| s.as_object().unwrap().values())
        .map(|d| d.as_object().unwrap().len())
        .sum();
    assert_eq!(n, total, "the report has packets the table does not");
}
