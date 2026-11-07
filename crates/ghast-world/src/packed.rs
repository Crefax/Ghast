//! 1.16+ "compact long arrays": fixed-width unsigned values packed into 64-bit words, lowest
//! bits first, where (since 1.16) a value never spans two words — `64 / bits` values per word,
//! the remaining high bits of each word are padding.

/// Number of `u64` words needed for `count` values of `bits` bits (1.16+ layout).
#[must_use]
pub const fn words_needed(bits: u32, count: usize) -> usize {
    let per = (64 / bits) as usize;
    count.div_ceil(per)
}

/// Unpacks `out.len()` values of `bits` (`1..=32`) bits from big-endian words in `data`
/// (as they appear on the wire), calling `put(i, value)` for each.
///
/// `data` must hold at least [`words_needed`] words (checked by the caller); extra words are
/// ignored.
#[inline]
pub fn unpack_be(bits: u32, data: &[u8], count: usize, mut put: impl FnMut(usize, u32)) {
    debug_assert!((1..=32).contains(&bits));
    let per = (64 / bits) as usize;
    let mask = (1u64 << bits) - 1;
    let mut i = 0;
    for w in data.as_chunks::<8>().0 {
        if i >= count {
            break;
        }
        let mut word = u64::from_be_bytes(*w);
        let n = per.min(count - i);
        for _ in 0..n {
            put(i, (word & mask) as u32);
            word >>= bits;
            i += 1;
        }
    }
}

/// Number of `u64` words for `count` values of `bits` bits in the 1.9 – 1.15 layout, where a
/// value may span two words (`ceil(count * bits / 64)`).
#[must_use]
pub const fn words_needed_spanning(bits: u32, count: usize) -> usize {
    (count * bits as usize).div_ceil(64)
}

/// Unpacks `count` values of `bits` (`1..=32`) bits from big-endian words in the 1.9 – 1.15
/// layout (vanilla `PaletteData`): value `i` starts at bit `i * bits` of the word stream, lowest
/// bits first, and may continue in the next word.
///
/// `data` must hold at least [`words_needed_spanning`] words (checked by the caller).
#[inline]
pub fn unpack_spanning_be(bits: u32, data: &[u8], count: usize, mut put: impl FnMut(usize, u32)) {
    debug_assert!((1..=32).contains(&bits));
    let mask = (1u64 << bits) - 1;
    let word = |w: usize| -> u64 {
        let b = &data[w * 8..w * 8 + 8];
        u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    };
    let bits = bits as usize;
    for i in 0..count {
        let start = i * bits;
        let (w, off) = (start / 64, start % 64);
        let lo = word(w) >> off;
        let v = if off + bits > 64 {
            lo | (word(w + 1) << (64 - off))
        } else {
            lo
        };
        put(i, (v & mask) as u32);
    }
}

/// Packs values into native `u64` words with the 1.9 – 1.15 spanning layout (inverse of
/// [`unpack_spanning_be`] apart from byte order; used by tests and encoders).
#[must_use]
pub fn pack_spanning(bits: u32, values: &[u32]) -> Vec<u64> {
    let mask = (1u64 << bits) - 1;
    let bits = bits as usize;
    let mut out = vec![0u64; words_needed_spanning(bits as u32, values.len())];
    for (i, &v) in values.iter().enumerate() {
        let v = u64::from(v) & mask;
        let start = i * bits;
        let (w, off) = (start / 64, start % 64);
        out[w] |= v << off;
        if off + bits > 64 {
            out[w + 1] |= v >> (64 - off);
        }
    }
    out
}

/// Packs values into native `u64` words with the 1.16+ layout (inverse of [`unpack_be`] apart
/// from byte order; used by tests and encoders).
#[must_use]
pub fn pack(bits: u32, values: &[u32]) -> Vec<u64> {
    let per = (64 / bits) as usize;
    let mask = (1u64 << bits) - 1;
    values
        .chunks(per)
        .map(|c| {
            c.iter().enumerate().fold(0u64, |w, (j, &v)| {
                w | ((u64::from(v) & mask) << (j as u32 * bits))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be(words: &[u64]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    /// Known answers computed by hand from the 1.16 layout.
    #[test]
    fn known_answers() {
        // bits 4: value i at bits 4*i of word 0 (16 per word).
        let w = 0xFEDC_BA98_7654_3210u64;
        let mut out = vec![0u32; 16];
        unpack_be(4, &be(&[w]), 16, |i, v| out[i] = v);
        assert_eq!(out, (0..16).collect::<Vec<_>>());

        // bits 5: 12 per word, top 4 bits padding (set them to check they are ignored).
        let vals: Vec<u32> = (0..12).map(|i| (i * 3 + 1) % 32).collect();
        let mut w = 0xF000_0000_0000_0000u64;
        for (j, v) in vals.iter().enumerate() {
            w |= u64::from(*v) << (5 * j);
        }
        let mut out = vec![0u32; 12];
        unpack_be(5, &be(&[w]), 12, |i, v| out[i] = v);
        assert_eq!(out, vals);

        // bits 14 (4 per word, 8 padding bits): value 4 is the low 14 bits of word 1.
        let mut out = vec![0u32; 5];
        unpack_be(14, &be(&[0, 0x2ABC]), 5, |i, v| out[i] = v);
        assert_eq!(out, [0, 0, 0, 0, 0x2ABC]);

        // bits 15: 4 per word, value 3 occupies bits 45..60.
        let mut out = vec![0u32; 4];
        unpack_be(15, &be(&[0x7FFFu64 << 45]), 4, |i, v| out[i] = v);
        assert_eq!(out, [0, 0, 0, 0x7FFF]);
    }

    #[test]
    fn words() {
        assert_eq!(words_needed(4, 4096), 256);
        assert_eq!(words_needed(5, 4096), 342);
        assert_eq!(words_needed(6, 4096), 410);
        assert_eq!(words_needed(7, 4096), 456);
        assert_eq!(words_needed(8, 4096), 512);
        assert_eq!(words_needed(14, 4096), 1024);
        assert_eq!(words_needed(15, 4096), 1024);
        assert_eq!(words_needed(16, 4096), 1024);
        assert_eq!(words_needed(13, 4096), 1024);
        assert_eq!(words_needed(9, 4096), 586);
        assert_eq!(words_needed(10, 4096), 683);
        assert_eq!(words_needed(11, 4096), 820);
        assert_eq!(words_needed(12, 4096), 820);
    }

    #[test]
    fn spanning_round_trip_and_known_answer() {
        // bits 13: the fifth value starts at bit 52 and spans into word 1.
        let vals: Vec<u32> = vec![1, 2, 3, 4, 0x1FFF];
        let words = pack_spanning(13, &vals);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0] >> 52, 0xFFF);
        assert_eq!(words[1] & 1, 1);
        let mut out = vec![0u32; 5];
        unpack_spanning_be(13, &be(&words), 5, |i, v| out[i] = v);
        assert_eq!(out, vals);
        for bits in 1..=32u32 {
            let max = if bits == 32 {
                u32::MAX
            } else {
                (1u32 << bits) - 1
            };
            let vals: Vec<u32> = (0..4096u32)
                .map(|i| i.wrapping_mul(2_654_435_761) & max)
                .collect();
            let words = pack_spanning(bits, &vals);
            assert_eq!(words.len(), (4096 * bits as usize) / 64, "bits {bits}");
            let mut out = vec![0u32; 4096];
            unpack_spanning_be(bits, &be(&words), 4096, |i, v| out[i] = v);
            assert_eq!(out, vals, "bits {bits}");
        }
    }

    #[test]
    fn round_trip_all_widths() {
        for bits in 1..=32u32 {
            let max = if bits == 32 {
                u32::MAX
            } else {
                (1u32 << bits) - 1
            };
            let vals: Vec<u32> = (0..4096u32)
                .map(|i| i.wrapping_mul(2_654_435_761) & max)
                .collect();
            let words = pack(bits, &vals);
            assert_eq!(words.len(), words_needed(bits, 4096), "bits {bits}");
            let mut out = vec![0u32; 4096];
            unpack_be(bits, &be(&words), 4096, |i, v| out[i] = v);
            assert_eq!(out, vals, "bits {bits}");
        }
    }
}
