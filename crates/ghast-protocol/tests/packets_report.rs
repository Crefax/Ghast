//! Packet ids of protocols 775 and 776 against the vanilla server's own report.
//!
//! `tests/data/packets_<N>.tsv` pairs every (state, direction, id) of the server's
//! `--reports` `packets.json` (26.1.2 for 775, 26.2 for 776) with the name used by the
//! generated code (see `codegen/protocol/src/report.rs`). These tests check the generated
//! packet sets against that table: same ids, same names, nothing missing on either side.
//!
//! When the report itself is in the git-ignored cache
//! (`.cache/mojang/26.1.2/reports/reports/packets.json`,
//! `.cache/mojang/26.2/generated/reports/packets.json`, made by
//! `java -DbundlerMainClass=net.minecraft.data.Main -jar server.jar --reports`), the table
//! is also checked against it.

use ghast_protocol::PacketSet;

struct Row<'a> {
    state: &'a str,
    dir: &'a str,
    id: i32,
    vanilla: &'a str,
    mcdata: &'a str,
}

fn rows(table: &'static str) -> Vec<Row<'static>> {
    table
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

#[allow(dead_code)]
fn set_of<S: PacketSet>(ids: &[i32]) -> Vec<(i32, &'static str)> {
    ids.iter().map(|&i| (i, S::name_of(i).unwrap())).collect()
}

type Set = (&'static str, &'static str, Vec<(i32, &'static str)>);

#[allow(dead_code)]
fn check_sets(table: &'static str, sets: &[Set]) {
    let rows = rows(table);
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
    for (state, dir, ours) in sets {
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

#[allow(dead_code)]
fn check_report(table: &'static str, path: &str) {
    let Ok(text) = std::fs::read_to_string(path) else {
        eprintln!("skipped: no {path}");
        return;
    };
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut n = 0;
    for r in rows(table) {
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

macro_rules! version {
    ($feature:literal, $m:ident, $table:literal, $report:literal) => {
        #[cfg(feature = $feature)]
        mod $m {
            use super::*;
            use ghast_protocol::$m::{configuration, handshaking, login, play, status};

            const TABLE: &str = include_str!($table);

            #[test]
            fn generated_ids_match_the_vanilla_report() {
                check_sets(
                    TABLE,
                    &[
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
                    ],
                );
            }

            #[test]
            fn table_matches_cached_report() {
                check_report(TABLE, concat!(env!("CARGO_MANIFEST_DIR"), $report));
            }
        }
    };
}

version!(
    "v26_1",
    v26_1,
    "data/packets_775.tsv",
    "/../../.cache/mojang/26.1.2/reports/reports/packets.json"
);
version!(
    "v26_2",
    v26_2,
    "data/packets_776.tsv",
    "/../../.cache/mojang/26.2/generated/reports/packets.json"
);
