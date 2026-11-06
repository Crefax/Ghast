//! Byte-for-byte checks against the TypeScript encoder (the web client's `protocol.ts`).
//! The reference bytes come from `tests/ts/known_frames.mts` (same synthetic inputs).

use ghast_viewer::entities::mob_variant;
use ghast_viewer::protocol::{
    Explosion, Looks, NO_COLOR, Particle, SELF, SectionIn, SignText, Sky, Tile, TileKind, Writer,
    encode_column, s, write_explosion, write_looks, write_particles,
};

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/ts_frames.json")).unwrap()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn check(name: &str, got: &[u8]) {
    let want = fixture()[name].as_str().unwrap().to_string();
    let got = hex(got);
    if got != want {
        let at = got
            .bytes()
            .zip(want.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(got.len().min(want.len()));
        panic!(
            "{name}: first difference at byte {} (lengths {} vs {})",
            at / 2,
            got.len() / 2,
            want.len() / 2
        );
    }
}

enum Light {
    None,
    Data,
    Full,
}

struct Sec {
    states: Box<[u16; 4096]>,
    block: Option<Box<[u8; 2048]>>,
    sky: Option<Box<[u8; 2048]>>,
}

fn section(seed: u32, kinds: u32, light: Light) -> Sec {
    let mut states = Box::new([0u16; 4096]);
    let mut x = seed;
    for (i, s) in states.iter_mut().enumerate() {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
        *s = ((((x >> 16) % kinds) + 1) << 4) as u16 | (i as u16 & 3);
    }
    let (block, sky) = match light {
        Light::None => (None, None),
        Light::Full => (None, Some(Box::new([0xff; 2048]))),
        Light::Data => {
            let mut b = Box::new([0u8; 2048]);
            let mut k = Box::new([0u8; 2048]);
            for i in 0..2048 {
                b[i] = ((i * 7) & 255) as u8;
                k[i] = (255 - (i & 15)) as u8;
            }
            (Some(b), Some(k))
        }
    };
    Sec { states, block, sky }
}

fn input(s: &Sec) -> SectionIn<'_> {
    SectionIn {
        states: &s.states,
        block_light: s.block.as_deref(),
        // a prismarine section without a sky array reads as dark
        sky: s.sky.as_deref().map_or(Sky::Dark, Sky::Data),
    }
}

#[test]
fn column_matches_typescript() {
    let secs = [
        (0, section(1, 1, Light::Full)),
        (1, section(2, 2, Light::Data)),
        (3, section(3, 4, Light::None)),
        (4, section(4, 9, Light::Full)),
        (5, section(5, 100, Light::Data)),
        (6, section(6, 300, Light::Full)),
    ];
    let air = Sec {
        states: Box::new([0; 4096]),
        block: None,
        sky: None,
    };
    let mut sections: [Option<SectionIn<'_>>; 16] = [None; 16];
    for (y, s) in &secs {
        sections[*y] = Some(input(s));
    }
    sections[7] = Some(input(&air));
    let mut biomes = [0u8; 256];
    for (i, b) in biomes.iter_mut().enumerate() {
        *b = (i % 7) as u8;
    }
    let signs = [SignText {
        x: 1,
        y: 64,
        z: 2,
        lines: ["Hello".into(), String::new(), "ünï".into(), "x".repeat(70)],
    }];
    let tiles = [
        Tile {
            x: 3,
            y: 65,
            z: 4,
            kind: TileKind::Skull {
                skull: 3,
                rot: 9,
                skin: "skins/abc.png".into(),
            },
        },
        Tile {
            x: 5,
            y: 66,
            z: 6,
            kind: TileKind::Banner {
                base: 14,
                patterns: vec![("bri".into(), 1), ("cr".into(), 15)],
            },
        },
        Tile {
            x: 7,
            y: 67,
            z: 8,
            kind: TileKind::Pot {
                contents: "red_tulip".into(),
            },
        },
    ];
    check(
        "column",
        &encode_column(&sections, Some(&biomes), true, &signs, &tiles),
    );

    let only = section(9, 3, Light::Data);
    let mut sections: [Option<SectionIn<'_>>; 16] = [None; 16];
    sections[0] = Some(input(&only));
    check(
        "columnNoSky",
        &encode_column(&sections, None, false, &[], &[]),
    );
}

#[test]
fn composite_messages_match_typescript() {
    let mut w = Writer::default();
    write_looks(
        &mut w,
        42,
        &Looks {
            flags: 1,
            armour: vec![(1, 301, 0, 0xa0_6540), (4, 310, 0, NO_COLOR)],
            pose: None,
        },
    );
    write_looks(
        &mut w,
        SELF,
        &Looks {
            flags: 0,
            armour: vec![],
            pose: Some([
                1.0, 2.0, 3.0, -10.0, 0.0, -10.0, -15.0, 0.0, 10.0, -1.0, 0.0, -1.0, 1.0, 0.0, 1.0,
                0.1, 0.2, 0.3,
            ]),
        },
    );
    check("looks", w.as_slice());

    let mut w = Writer::default();
    write_particles(
        &mut w,
        &[
            Particle {
                id: 37,
                x: 1.5,
                y: 64.1,
                z: -3.3,
                ox: 0.1,
                oy: 0.2,
                oz: 0.3,
                speed: 0.05,
                count: 12,
                data: (1 << 4) | 2,
            },
            Particle {
                id: 104,
                x: -100.25,
                y: 5.0,
                z: 1e6,
                ox: 0.0,
                oy: 0.0,
                oz: 0.0,
                speed: 0.0,
                count: 1,
                data: 7,
            },
        ],
    );
    write_explosion(
        &mut w,
        &Explosion {
            x: 0.5,
            y: 70.3,
            z: 0.5,
            radius: 4.0,
            blocks: vec![[1, -1, 0], [-128, 127, 3]],
        },
    );
    check("fx", w.as_slice());
}

#[test]
fn inline_messages_match_typescript() {
    let mut w = Writer::default();
    let at = w.begin(s::POSE);
    w.f64(1_730_000_000_123.5)
        .u32(4567)
        .f64(100.5)
        .f64(64.0)
        .f64(-200.25);
    w.f32(std::f64::consts::PI as f32).f32(-0.5).u8(1 | 8 | 16);
    w.f32(0.1).f32(-0.0784).f32(0.0);
    w.end(at);
    let at = w.begin(s::SPAWN);
    w.u32(12).u8(3).str("PigZombie").str("Zombie Pigman");
    w.f64(1.5)
        .f64(2.5)
        .f64(3.5)
        .f32(1.0)
        .f32(2.0)
        .f32(3.0)
        .f32(0.6)
        .f32(1.95);
    w.u16(283).u16(0).u16(0);
    w.end(at);
    let at = w.begin(s::MOVES);
    w.u16(2);
    w.u32(12)
        .f64(1.0)
        .f64(2.0)
        .f64(3.0)
        .f32(0.5)
        .f32(0.25)
        .f32(0.125);
    w.u32(13)
        .f64(-1.0)
        .f64(-2.0)
        .f64(-3.0)
        .f32(6.2)
        .f32(-1.5)
        .f32(6.2);
    w.end(at);
    let at = w.begin(s::REMOVE);
    w.u16(1).u32(12);
    w.end(at);
    let at = w.begin(s::BLOCKS);
    w.u16(2);
    w.i32(-17).u8(64).i32(33).u16(1 << 4);
    w.i32(5).u8(255).i32(-1).u16(0);
    w.end(at);
    let at = w.begin(s::EQUIP);
    w.u32(13).u16(276).u16(0);
    w.end(at);
    let at = w.begin(s::INVENTORY);
    w.u8(3);
    for i in 0..9 {
        w.u16(if i == 0 { 278 } else { 0 })
            .u16(if i == 2 { 5 } else { 0 })
            .u8(u8::from(i == 0));
    }
    w.end(at);
    let at = w.begin(s::VITALS);
    w.f32(19.5).u8(17).f32(2.5).u8(30).f32(0.42);
    w.end(at);
    let at = w.begin(s::TIME);
    w.f64(1_730_000_000_999.0).f64(123_456_789.0).f64(-6000.0);
    w.end(at);
    let at = w.begin(s::PONG);
    w.f64(55.5).f64(1_730_000_001_000.0);
    w.end(at);
    w.empty(s::RESET);
    let at = w.begin(s::UNLOAD);
    w.i32(-4).i32(9);
    w.end(at);
    let at = w.begin(s::CRACKS);
    w.u16(1).i32(10).u8(70).i32(-10).u16(56 << 4).u8(4);
    w.end(at);
    let at = w.begin(s::WEATHER);
    w.f32(0.7).f32(0.0);
    w.end(at);
    let at = w.begin(s::EVENTS);
    w.u16(2).u32(SELF).u8(0).u32(13).u8(1);
    w.end(at);
    let at = w.begin(s::ESTATE);
    w.u16(1).u32(13).u8(2 | 4).u32(15);
    w.end(at);
    let at = w.begin(s::COLLECT);
    w.u32(99).u32(SELF);
    w.end(at);
    let at = w.begin(s::CHESTS);
    w.u16(1).i32(1).u8(64).i32(2).u8(1);
    w.end(at);
    let at = w.begin(s::SKIN);
    w.u32(13).u8(1).str("skins/0123.png").str("");
    w.end(at);
    let at = w.begin(s::LIGHTNING);
    w.f64(1.0).f64(2.0).f64(3.0);
    w.end(at);
    let at = w.begin(s::REPLAY);
    w.f64(1500.0).f64(60000.0).f32(2.0).f64(1_730_000_000_000.0);
    w.end(at);
    let at = w.begin(s::CHUNK);
    w.i32(-1).i32(2).u8(1).bytes(&[1, 2, 3]);
    w.end(at);
    check("inline", w.as_slice());
}

#[test]
fn mob_variants_match_typescript() {
    for case in fixture()["variants"].as_array().unwrap() {
        let name = case[0].as_str().unwrap();
        let meta: Vec<(u8, i32)> = case[1]
            .as_array()
            .unwrap()
            .iter()
            .map(|kv| {
                (
                    kv[0].as_u64().unwrap() as u8,
                    kv[1].as_i64().unwrap() as i32,
                )
            })
            .collect();
        let want = case[2].as_u64().unwrap() as u16;
        let got = mob_variant(name, |i| {
            meta.iter().find(|(k, _)| *k == i).map_or(0, |(_, v)| *v)
        });
        assert_eq!(got, want, "{name} {meta:?}");
    }
}
