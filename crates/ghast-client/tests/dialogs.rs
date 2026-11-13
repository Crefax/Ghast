//! Server dialogs (26.1, 26.2) through the whole client against a fake server that replays a
//! real vanilla server session (the captures of `ghast-protocol/tests/fixtures`) and adds
//! `show_dialog` packets: a dialog in the configuration state answered by
//! `Behavior::configuration_dialog` (the login then goes on into play), then dialogs in the
//! play state answered with every action type. The packet bodies the client sends must equal
//! the ones the vanilla client classes write for the same dialog and inputs
//! (`tests/golden/26.2/dialog_answers.txt`, see `oracle/dialogs/`; the dialog classes of 26.1 are
//! the same).

#![cfg(all(feature = "v26_1", feature = "v26_2"))]

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::{BufMut, Bytes, BytesMut};
use ghast_client::connect::TargetAddr;
use ghast_client::dialog::{
    Dialog, DialogAnswer, DialogEvent, DialogOutcome, DialogSent, InputKind, InputValue,
};
use ghast_client::{Account, Behavior, Client, GameVersion, Interests, SessionEnd, TickContext};
use ghast_protocol::codec::Reader;
use ghast_protocol::nbt::{AnonNbt, Tag};
use ghast_protocol::{FrameDecoder, FrameEncoder, McWrite, PacketSet, RawPacket};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// One vanilla answer vector.
#[derive(Clone)]
struct Case {
    name: String,
    dialog: Bytes,
    pick: String,
    values: Bytes,
    sent: String,
    body: Option<Vec<u8>>,
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn cases() -> Vec<Case> {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/golden/26.2/dialog_answers.txt"
    ))
    .unwrap();
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            Case {
                name: f[0].into(),
                dialog: hex(f[1]).into(),
                pick: f[2].into(),
                values: hex(f[3]).into(),
                sent: f[4].into(),
                body: f.get(5).filter(|_| f[4] != "client").map(|h| hex(h)),
            }
        })
        .collect()
}

/// The answer of a case for the decoded dialog.
fn answer(case: &Case, d: &Dialog, confirm: bool) -> DialogAnswer {
    let mut a = match case.pick.as_str() {
        "exit" => DialogAnswer::exit(),
        "close" => DialogAnswer::close(),
        p => DialogAnswer::button(p.strip_prefix("button_").unwrap().parse().unwrap()),
    };
    let values = AnonNbt::from_bytes(case.values.clone()).unwrap();
    let Some(Tag::Compound(values)) = values.parse().unwrap() else {
        panic!("values")
    };
    for (k, v) in values.0 {
        let kind = &d.inputs.iter().find(|i| i.key == k).unwrap().kind;
        let v = match (kind, v) {
            (InputKind::Text { .. }, Tag::String(s)) => InputValue::Text(s),
            (InputKind::SingleOption { .. }, Tag::String(s)) => InputValue::Option(s),
            (InputKind::Bool { .. }, Tag::Byte(b)) => InputValue::Bool(b != 0),
            (InputKind::NumberRange { .. }, Tag::Float(x)) => InputValue::Number(x),
            (k, v) => panic!("{}: {k:?} {v:?}", case.name),
        };
        a = a.with_value(k, v);
    }
    a.confirm_commands(confirm)
}

struct Record {
    state: u8,
    dir: u8,
    raw: RawPacket,
}

fn records(fixture: &[u8]) -> Vec<Record> {
    let mut data = Vec::new();
    flate2::read::ZlibDecoder::new(fixture)
        .read_to_end(&mut data)
        .unwrap();
    let data = Bytes::from(data);
    let mut r = Reader::new(&data);
    let mut out = Vec::new();
    while r.remaining() > 0 {
        let state = r.read_u8().unwrap();
        let dir = r.read_u8().unwrap();
        let id = r.read_varint().unwrap();
        let len = r.read_varint().unwrap() as usize;
        let body = r.read_bytes(len).unwrap();
        out.push(Record {
            state,
            dir,
            raw: RawPacket::new(id, body),
        });
    }
    out
}

struct Conn {
    stream: TcpStream,
    dec: FrameDecoder,
    enc: FrameEncoder,
}

impl Conn {
    async fn recv(&mut self) -> RawPacket {
        loop {
            if let Some(p) = self.dec.next_packet().unwrap() {
                return p;
            }
            let mut buf = [0u8; 8192];
            let n = tokio::time::timeout(Duration::from_secs(5), self.stream.read(&mut buf))
                .await
                .expect("client went quiet")
                .unwrap();
            assert!(n > 0, "client closed");
            self.dec.push(&buf[..n]);
        }
    }

    async fn send(&mut self, p: &RawPacket) {
        self.enc.write_raw(p).unwrap();
        let out = self.enc.take();
        self.stream.write_all(&out).await.unwrap();
    }
}

/// What the behaviour saw and did in the play state.
#[derive(Default)]
struct Log {
    /// The configuration dialog's title.
    config_title: Option<String>,
    /// Play answers in order: case name, outcome.
    outcomes: Vec<(String, DialogOutcome)>,
    /// A `clear_dialog` closed the open dialog.
    cleared: bool,
}

/// Answers the configuration dialog with case `config`, then the play dialogs in the order
/// of `play` (each followed by a marker chat line).
struct Answerer {
    config: Case,
    play: Vec<(Case, bool)>,
    next: usize,
    log: Arc<Mutex<Log>>,
}

impl Behavior for Answerer {
    fn interests(&self) -> Interests {
        Interests::DIALOGS
    }

    fn configuration_dialog(&mut self, dialog: &Dialog) -> Option<DialogAnswer> {
        self.log.lock().unwrap().config_title = Some(dialog.title.clone());
        Some(answer(&self.config, dialog, false))
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        let events = ctx.dialog_events().to_vec();
        for ev in events {
            match ev {
                DialogEvent::Shown(d) => {
                    if self.next < self.play.len() {
                        let (case, confirm) = self.play[self.next].clone();
                        assert_eq!(ctx.dialog().unwrap(), &*d);
                        let out = ctx.answer_dialog(&answer(&case, &d, confirm)).unwrap();
                        self.log
                            .lock()
                            .unwrap()
                            .outcomes
                            .push((case.name.clone(), out));
                        ctx.chat(format!("done {}", self.next));
                        self.next += 1;
                    }
                }
                DialogEvent::Cleared => {
                    assert!(ctx.dialog().is_none());
                    self.log.lock().unwrap().cleared = true;
                    ctx.chat("cleared");
                }
                _ => {}
            }
        }
    }
}

/// The play `show_dialog` body of an inline dialog (holder: 0, then the NBT).
fn play_dialog(nbt: &[u8]) -> Bytes {
    let mut b = BytesMut::new();
    b.put_varint(0);
    b.put_slice(nbt);
    b.freeze()
}

/// `chat_message`'s text.
fn chat_text(body: &Bytes) -> String {
    let mut r = Reader::new(body);
    r.read_string().unwrap().to_string()
}

macro_rules! dialog_session {
    ($name:ident, $proto:ident, $fixture:literal, $game:expr) => {
        #[tokio::test]
        async fn $name() {
            use ghast_protocol::$proto::{configuration, play};
            let recs = records(include_bytes!(concat!(
                "../../ghast-protocol/tests/fixtures/",
                $fixture
            )));
            let all = cases();
            let case = |n: &str| all.iter().find(|c| c.name == n).unwrap().clone();
            let config = case("notice_custom_defaults");
            // (case, confirm commands): every vanilla vector; the commands confirmed (vanilla's
            // check holds them for the replayed server's command tree).
            let mut play_cases: Vec<(Case, bool)> = all.iter().map(|c| (c.clone(), true)).collect();
            // A command the tree does not know, not confirmed: held, nothing sent.
            play_cases.push((case("template_login_run"), false));

            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = TargetAddr::from(l.local_addr().unwrap());
            let log = Arc::new(Mutex::new(Log::default()));
            let mut behaviour = Answerer {
                config: config.clone(),
                play: play_cases.clone(),
                next: 0,
                log: log.clone(),
            };
            let client = Client::new(addr, $game, Account::offline("DialogBot").unwrap())
                .read_timeout(Duration::from_secs(10))
                .login_timeout(Duration::from_secs(10));
            let run = tokio::spawn(async move { client.run(&mut behaviour).await });

            let (stream, _) = l.accept().await.unwrap();
            let mut c = Conn {
                stream,
                dec: FrameDecoder::new(),
                enc: FrameEncoder::new(),
            };
            // Handshake, login start.
            c.recv().await;
            c.recv().await;
            for r in recs.iter().filter(|r| r.state == 2 && r.dir == 0) {
                c.send(&r.raw).await;
                if r.raw.id == 0x03 {
                    // set_compression
                    let t = Reader::new(&r.raw.body).read_varint().unwrap();
                    c.enc.set_compression(t);
                    c.dec.set_compression(t);
                }
            }
            // login_acknowledged
            assert_eq!(c.recv().await.id, 0x03);
            // The configuration as the server sent it, a dialog before its end.
            let cs = configuration::clientbound::id::FINISH_CONFIGURATION;
            for r in recs.iter().filter(|r| r.state == 3 && r.dir == 0 && r.raw.id != cs) {
                c.send(&r.raw).await;
            }
            c.send(&RawPacket::new(
                configuration::clientbound::id::SHOW_DIALOG,
                config.dialog.clone(),
            ))
            .await;
            let click = loop {
                let p = c.recv().await;
                if p.id == configuration::serverbound::id::CUSTOM_CLICK_ACTION {
                    break p;
                }
                assert_ne!(p.id, configuration::serverbound::id::FINISH_CONFIGURATION);
            };
            assert_eq!(
                Some(click.body.to_vec()),
                config.body,
                "configuration custom_click_action"
            );
            // The login goes on: finish, then play.
            c.send(&RawPacket::new(cs, Bytes::new())).await;
            loop {
                let p = c.recv().await;
                if p.id == configuration::serverbound::id::FINISH_CONFIGURATION {
                    break;
                }
            }
            let skip = [
                play::clientbound::id::KICK_DISCONNECT,
                play::clientbound::id::START_CONFIGURATION,
                play::clientbound::id::TRANSFER,
            ];
            for r in recs
                .iter()
                .filter(|r| r.state == 4 && r.dir == 0 && !skip.contains(&r.raw.id))
            {
                c.send(&r.raw).await;
            }
            // In the world once the level-load gate opened.
            loop {
                if c.recv().await.id == play::serverbound::id::PLAYER_LOADED {
                    break;
                }
            }
            let name = |p: &RawPacket| {
                play::serverbound::ServerboundPlayPacket::name_of(p.id).unwrap_or("?")
            };
            for (i, (case, _)) in play_cases.iter().enumerate() {
                c.send(&RawPacket::new(
                    play::clientbound::id::SHOW_DIALOG,
                    play_dialog(&case.dialog),
                ))
                .await;
                let mut sent = Vec::new();
                loop {
                    let p = c.recv().await;
                    if p.id == play::serverbound::id::CHAT_MESSAGE {
                        assert_eq!(chat_text(&p.body), format!("done {i}"));
                        break;
                    }
                    if matches!(
                        name(&p),
                        "custom_click_action" | "chat_command" | "chat_command_signed"
                    ) {
                        sent.push((name(&p), p.body.to_vec()));
                    }
                }
                let held = i + 1 == play_cases.len();
                match (case.sent.as_str(), &sent[..]) {
                    (_, []) if held => {}
                    ("none" | "client", []) => {}
                    (kind, [(got, body)]) if kind == *got => {
                        assert_eq!(Some(body), case.body.as_ref(), "{}", case.name);
                    }
                    // A signable command, confirmed: the chat line's signed path.
                    ("chat_command", [("chat_command_signed", _)]) => {
                        assert_eq!(case.name, "confirmation_yes");
                    }
                    (kind, got) => panic!("{}: vanilla {kind}, ours {got:?}", case.name),
                }
            }
            // A dialog the server clears.
            c.send(&RawPacket::new(
                play::clientbound::id::SHOW_DIALOG,
                play_dialog(&case("template_login").dialog),
            ))
            .await;
            c.send(&RawPacket::new(play::clientbound::id::CLEAR_DIALOG, Bytes::new()))
                .await;
            loop {
                let p = c.recv().await;
                if p.id == play::serverbound::id::CHAT_MESSAGE && chat_text(&p.body) == "cleared" {
                    break;
                }
            }
            drop(c);
            let end = run.await.unwrap().unwrap();
            assert_eq!(end, SessionEnd::ServerClosed);

            let log = log.lock().unwrap();
            assert_eq!(log.config_title.as_deref(), Some("Hello"));
            assert!(log.cleared);
            assert_eq!(log.outcomes.len(), play_cases.len());
            for (name, out) in &log.outcomes {
                match name.as_str() {
                    "open_url" => assert_eq!(out.sent, DialogSent::Nothing),
                    n if n.starts_with("notice_custom") || n.starts_with("custom_") => {
                        assert!(matches!(out.sent, DialogSent::Custom(_)), "{name} {out:?}");
                    }
                    _ => {}
                }
            }
            let (_, held) = log.outcomes.last().unwrap();
            assert!(
                matches!(&held.sent, DialogSent::CommandHeld { command, .. } if command == "login fixed"),
                "{held:?}"
            );
            assert!(held.open.is_some(), "a held command keeps the dialog open");
        }
    };
}

dialog_session!(v26_1, v26_1, "capture_26_1_2.bin.zz", GameVersion::V26_1);
dialog_session!(v26_2, v26_2, "capture_26_2.bin.zz", GameVersion::V26_2);
