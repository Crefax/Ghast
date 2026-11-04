//! Never-panic tests on random and mutated chunk data, and a model check of block changes.

mod common;

#[cfg(feature = "v1_8")]
mod v1_8 {
    use super::*;
    use bytes::Bytes;
    use common::{LegacySection, Rng, encode_1_8};
    use ghast_protocol::v1_8::play::clientbound::{MapChunkBulk, MapChunkBulkMeta};
    use ghast_world::WorldConfig;
    use ghast_world::v1_8::World;
    use proptest::prelude::*;

    fn valid(seed: u64) -> (u16, Vec<u8>) {
        let mut rng = Rng(seed | 1);
        let secs: [Option<LegacySection>; 16] = std::array::from_fn(|_| {
            (rng.below(3) == 0).then(|| LegacySection {
                blocks: (0..4096).map(|_| rng.next() as u16).collect(),
                block_light: vec![0x12; 2048],
                sky_light: vec![0x34; 2048],
            })
        });
        encode_1_8(&secs, true, Some(&[3; 256]))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(300))]

        #[test]
        fn random_chunk_data(ground_up: bool, mask: u16, data in proptest::collection::vec(any::<u8>(), 0..70_000), light: bool) {
            let mut w = World::new(WorldConfig { biomes: true, light });
            w.apply_chunk_data(0, 0, true, 1, &[0; 8192 + 2048 + 2048 + 256]).unwrap();
            let r = w.apply_chunk_data(0, 0, ground_up, mask, &data);
            if r.is_err() {
                prop_assert!(!w.is_column_loaded(0, 0));
            }
            let _ = w.block_state(5, 5, 5);
        }

        #[test]
        fn mutated_chunk_data(seed: u64, cuts in proptest::collection::vec((any::<usize>(), any::<u8>()), 0..20), trunc in any::<usize>(), mask_flip: u16) {
            let (mask, mut data) = valid(seed);
            for (pos, v) in cuts {
                if !data.is_empty() {
                    let n = data.len();
                    data[pos % n] = v;
                }
            }
            if trunc % 3 == 0 && !data.is_empty() {
                let n = data.len();
                data.truncate(trunc % n);
            }
            let mut w = World::new(WorldConfig { biomes: true, light: true });
            let r = w.apply_chunk_data(1, -1, true, mask ^ mask_flip, &data);
            if r.is_err() {
                prop_assert!(!w.is_column_loaded(1, -1));
            }
        }

        #[test]
        fn random_bulk(sky: bool, metas in proptest::collection::vec((any::<i32>(), any::<i32>(), any::<u16>()), 0..8), data in proptest::collection::vec(any::<u8>(), 0..100_000)) {
            let mut w = World::new(WorldConfig::default());
            let p = MapChunkBulk {
                sky_light_sent: sky,
                meta: metas.into_iter().map(|(x, z, bit_map)| MapChunkBulkMeta { x, z, bit_map }).collect(),
                data: Bytes::from(data),
            };
            let _ = w.apply_map_chunk_bulk(&p);
        }
    }
}

#[cfg(feature = "v1_16_5")]
mod v1_16_5 {
    use super::*;
    use common::{Palette16, Rng, encode_section_1_16};
    use ghast_world::WorldConfig;
    use ghast_world::v1_16_5::World;
    use proptest::prelude::*;

    fn valid(seed: u64) -> (i32, Vec<u8>) {
        let mut rng = Rng(seed | 1);
        let mut data = Vec::new();
        let mut mask = 0;
        for j in 0..16 {
            if rng.below(3) != 0 {
                continue;
            }
            mask |= 1 << j;
            let mode = match rng.below(3) {
                0 => Palette16::Indirect(4),
                1 => Palette16::Indirect(8),
                _ => Palette16::Direct {
                    header: 15,
                    bits: 15,
                },
            };
            let distinct = match mode {
                Palette16::Indirect(b) => 1u64 << b,
                Palette16::Direct { .. } => 20_000,
            };
            let ids: Vec<u16> = (0..4096).map(|_| rng.below(distinct) as u16).collect();
            encode_section_1_16(&mut data, &ids, mode, &[]);
        }
        (mask, data)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(300))]

        #[test]
        fn random_chunk_data(full: bool, mask: i32, data in proptest::collection::vec(any::<u8>(), 0..40_000)) {
            let mut w = World::new(WorldConfig::default());
            w.apply_chunk_data(0, 0, true, 0, None, &[]).unwrap();
            let r = w.apply_chunk_data(0, 0, full, mask, None, &data);
            if r.is_err() {
                prop_assert!(!w.is_column_loaded(0, 0));
            }
        }

        #[test]
        fn mutated_chunk_data(seed: u64, cuts in proptest::collection::vec((any::<usize>(), any::<u8>()), 0..20), trunc in any::<usize>(), mask_flip: i32) {
            let (mask, mut data) = valid(seed);
            for (pos, v) in cuts {
                if !data.is_empty() {
                    let n = data.len();
                    data[pos % n] = v;
                }
            }
            if trunc % 3 == 0 && !data.is_empty() {
                let n = data.len();
                data.truncate(trunc % n);
            }
            let mut w = World::new(WorldConfig::default());
            let r = w.apply_chunk_data(1, -1, true, mask ^ mask_flip, Some(&[0; 3]), &data);
            if r.is_err() {
                prop_assert!(!w.is_column_loaded(1, -1));
            }
            for y in [0, 100, 255] {
                let _ = w.block_state(16, y, -16);
            }
        }
    }
}

/// Block changes against a dense model: every representation transition (uniform, indexed,
/// palette compaction, direct, dropping all-air sections) must keep every block correct.
#[cfg(feature = "v1_8")]
#[test]
fn block_changes_match_model() {
    use common::Rng;
    use ghast_world::Column;
    let mut rng = Rng(0xDEAD_BEEF);
    for round in 0..20 {
        let mut col = Column::empty();
        let mut model = vec![0u16; 16 * 256 * 16];
        let distinct = [2u64, 30, 300, 4000][round % 4];
        for _ in 0..30_000 {
            let i = rng.below(model.len() as u64) as usize;
            let (x, y, z) = ((i & 15) as i32, (i >> 8) as i32, ((i >> 4) & 15) as i32);
            let v = if rng.below(4) == 0 {
                0
            } else {
                rng.below(distinct) as u16
            };
            col.set_raw(x, y, z, v);
            model[i] = v;
        }
        for (i, &v) in model.iter().enumerate() {
            let (x, y, z) = ((i & 15) as i32, (i >> 8) as i32, ((i >> 4) & 15) as i32);
            assert_eq!(col.raw(x, y, z), v, "round {round} index {i}");
        }
        for j in 0..16 {
            let n = model[j * 4096..(j + 1) * 4096]
                .iter()
                .filter(|&&v| v != 0)
                .count();
            assert_eq!(col.section(j).map_or(0, |s| usize::from(s.non_air())), n);
        }
    }
}
