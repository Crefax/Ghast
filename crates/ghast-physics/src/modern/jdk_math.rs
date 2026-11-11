//! `Math.cos` as the HotSpot JVM on x86-64 computes it, for float arguments of at most pi/2.
//!
//! The JVM does not use the correctly rounded cosine: its intrinsic differs from it by one unit in
//! the last place for 44,355 of the 1,070,141,468 float arguments in `[0, pi/2]` (and is
//! symmetric). Gliding reads `Math.cos(pitch in radians)`, a float, so the result here is the
//! correctly rounded cosine (a double-double Taylor series, rounded once; no argument of the
//! domain is within the series' error of a rounding boundary) corrected by that exception table.
//!
//! The table (`jdk_cos_table.rs`) is generated from the JVM itself: `MC=26.1 oracle/run.sh
//! jdk-cos --out FILE` writes `Math.cos` of every float in the domain, and the ignored test
//! `jdk_cos_table` here compares it with [`cos_correct`] and writes the table (see its doc).

#[allow(clippy::unreadable_literal)]
mod table {
    include!("jdk_cos_table.rs");
}

/// The largest float argument (bits) the table covers: `(float) (PI / 2)` plus 64 ulps.
pub(super) const DOMAIN_MAX_BITS: u32 = 0x3FC9_0FDB + 64;

#[derive(Clone, Copy)]
struct Dd {
    hi: f64,
    lo: f64,
}

fn two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    let bb = s - a;
    Dd {
        hi: s,
        lo: (a - (s - bb)) + (b - bb),
    }
}

fn quick(a: f64, b: f64) -> Dd {
    let s = a + b;
    Dd {
        hi: s,
        lo: b - (s - a),
    }
}

fn add(a: Dd, b: Dd) -> Dd {
    let s = two_sum(a.hi, b.hi);
    let t = two_sum(a.lo, b.lo);
    let s2 = quick(s.hi, s.lo + t.hi);
    quick(s2.hi, s2.lo + t.lo)
}

fn mul(a: Dd, b: Dd) -> Dd {
    let p = a.hi * b.hi;
    let e = a.hi.mul_add(b.hi, -p);
    quick(p, e + (a.hi * b.lo + a.lo * b.hi))
}

fn div(a: Dd, d: f64) -> Dd {
    let q1 = a.hi / d;
    let p = q1 * d;
    let pe = q1.mul_add(d, -p);
    let r = ((a.hi - p) - pe) + a.lo;
    quick(q1, r / d)
}

/// The correctly rounded `cos(x)` for `|x| <= pi/2` (double-double series; `mul_add` is the
/// exactly rounded fused multiply-add on every platform).
pub(super) fn cos_correct(x: f64) -> f64 {
    let p = x * x;
    let x2 = Dd {
        hi: p,
        lo: x.mul_add(x, -p),
    };
    let mut term = Dd { hi: 1.0, lo: 0.0 };
    let mut sum = term;
    for n in 1..30u32 {
        term = div(mul(term, x2), f64::from((2 * n - 1) * (2 * n)));
        let t = if n % 2 == 1 {
            Dd {
                hi: -term.hi,
                lo: -term.lo,
            }
        } else {
            term
        };
        sum = add(sum, t);
    }
    quick(sum.hi, sum.lo).hi
}

/// The table's correction for a float argument in the domain: `+1`, `-1` (ulps of the result) or 0.
fn correction(bits: u32) -> i64 {
    let hi = bits >> 16;
    if hi < table::BASE || hi >= table::BASE + table::BUCKETS.len() as u32 - 1 {
        return 0;
    }
    let b = (hi - table::BASE) as usize;
    let (start, end) = (
        usize::from(table::BUCKETS[b]),
        usize::from(table::BUCKETS[b + 1]),
    );
    let low = (bits & 0xFFFF) as u16;
    match table::LOW[start..end].binary_search(&low) {
        Ok(i) => {
            let k = start + i;
            if table::SIGN[k / 8] & (1 << (k % 8)) != 0 {
                1
            } else {
                -1
            }
        }
        Err(_) => 0,
    }
}

/// `Math.cos((double) x)` of HotSpot x86-64 for a float `|x| <= pi/2` (+ 64 ulps); other
/// arguments get the correctly rounded value.
pub(super) fn cos(x: f32) -> f64 {
    let bits = x.to_bits() & 0x7FFF_FFFF;
    let r = cos_correct(f64::from(f32::from_bits(bits)));
    if bits > DOMAIN_MAX_BITS || !r.is_finite() {
        return r;
    }
    match correction(bits) {
        0 => r,
        d => f64::from_bits((r.to_bits() as i64 + d) as u64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_and_counts_match() {
        assert_eq!(
            usize::from(*table::BUCKETS.last().unwrap()),
            table::LOW.len()
        );
        for w in table::BUCKETS.windows(2) {
            assert!(w[0] <= w[1]);
            let s = &table::LOW[usize::from(w[0])..usize::from(w[1])];
            assert!(s.windows(2).all(|p| p[0] < p[1]));
        }
        assert_eq!(table::LOW.len(), 44_355);
    }

    #[test]
    fn known_values() {
        assert_eq!(cos(0.0), 1.0);
        // Exceptions seen against the JVM (argument bits, JVM result bits).
        assert_eq!(
            cos(f32::from_bits(0x3a46_9f90)).to_bits(),
            (cos_correct(f64::from(f32::from_bits(0x3a46_9f90)))).to_bits() + 1
        );
        assert_eq!(cos(-1.0), cos(1.0));
    }

    /// Regenerates `jdk_cos_table.rs`: `GHAST_JDK_COS` names a file (or FIFO) holding the JVM's
    /// `Math.cos` of every float argument from bits 0 to [`DOMAIN_MAX_BITS`], each as 8 bytes
    /// little-endian (what `MC=26.1 oracle/run.sh jdk-cos --out FILE` writes):
    ///
    /// ```sh
    /// mkfifo /tmp/cos.fifo
    /// MC=26.1 oracle/run.sh jdk-cos --out /tmp/cos.fifo &
    /// GHAST_JDK_COS=/tmp/cos.fifo cargo test --release -p ghast-physics --lib jdk_cos_table -- --ignored
    /// ```
    #[test]
    #[ignore = "needs the JVM stream (about 8.6 GB, 10 minutes)"]
    fn jdk_cos_table() {
        use std::io::Read;
        let path = std::env::var("GHAST_JDK_COS").expect("GHAST_JDK_COS");
        let mut inp =
            std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(path).unwrap());
        let mut buf = [0u8; 8];
        let mut ex: Vec<(u32, bool)> = Vec::new();
        for bits in 0..=DOMAIN_MAX_BITS {
            inp.read_exact(&mut buf).expect("stream too short");
            let jvm = f64::from_le_bytes(buf);
            let r = cos_correct(f64::from(f32::from_bits(bits)));
            if jvm.to_bits() != r.to_bits() {
                let d = jvm.to_bits() as i64 - r.to_bits() as i64;
                assert!(d.abs() == 1, "bits {bits:08x}: {d} ulps");
                ex.push((bits, d > 0));
            }
        }
        let base = ex[0].0 >> 16;
        let top = ex.last().unwrap().0 >> 16;
        let mut buckets = vec![0u16; (top - base + 2) as usize];
        for &(b, _) in &ex {
            buckets[((b >> 16) - base + 1) as usize] += 1;
        }
        for i in 1..buckets.len() {
            buckets[i] += buckets[i - 1];
        }
        let mut sign = vec![0u8; ex.len().div_ceil(8)];
        for (k, &(_, plus)) in ex.iter().enumerate() {
            if plus {
                sign[k / 8] |= 1 << (k % 8);
            }
        }
        let join = |v: Vec<String>| {
            v.chunks(16)
                .map(|c| format!("    {},\n", c.join(", ")))
                .collect::<String>()
        };
        let src = format!(
            "// @generated by the ignored test `jdk_cos_table` in `jdk_math.rs` from the JVM's own \
             `Math.cos` (HotSpot x86-64);\n// do not edit.\n\n\
             /// High 16 bits of the first bucket.\npub const BASE: u32 = {base:#x};\n\n\
             /// Start of each high-16-bit bucket in [`LOW`] (one more entry than buckets).\n\
             pub static BUCKETS: [u16; {}] = [\n{}];\n\n\
             /// Low 16 bits of the exceptional arguments, sorted within each bucket.\n\
             pub static LOW: [u16; {}] = [\n{}];\n\n\
             /// Bit k set: exception k is one ulp above the correctly rounded value, else below.\n\
             pub static SIGN: [u8; {}] = [\n{}];\n",
            buckets.len(),
            join(buckets.iter().map(|b| b.to_string()).collect()),
            ex.len(),
            join(
                ex.iter()
                    .map(|&(b, _)| ((b & 0xFFFF) as u16).to_string())
                    .collect()
            ),
            sign.len(),
            join(sign.iter().map(|b| b.to_string()).collect()),
        );
        let out =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/modern/jdk_cos_table.rs");
        std::fs::write(out, src).unwrap();
    }
}
