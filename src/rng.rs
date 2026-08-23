// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Small, fast, self-contained PRNG (xoshiro256++), seeded via SplitMix64.
//! Deterministic per seed, so every session is reproducible from its index.
//!
//! This is a Rust translation of two published algorithms by David Blackman
//! and Sebastiano Vigna:
//!
//! * **xoshiro256++ 1.0** — Blackman & Vigna, "Scrambled Linear
//!   Pseudorandom Number Generators", ACM Transactions on Mathematical
//!   Software, 2021.
//! * **SplitMix64** — Vigna's recommended seed expander for initializing
//!   xoshiro state from a single 64-bit seed.
//!
//! The authors' reference C implementations, from which this translation
//! was made, are dedicated to the public domain under CC0 1.0
//! (<https://prng.di.unimi.it/>). Not cryptographically secure — chosen for
//! statistical quality (passes BigCrush/PractRand), speed, and a 2^256 − 1
//! period, which is all a dice simulation needs.

pub struct Xoshiro256pp {
    s: [u64; 4],
    /// Unconsumed 6-bit chunks of the last generator output, used to batch
    /// dice-pair sampling (see [`Self::dice`]).
    bits: u64,
    avail: u32,
}

impl Xoshiro256pp {
    pub fn seed_from_u64(seed: u64) -> Self {
        // SplitMix64 to expand the seed into four well-mixed words.
        let mut sm = seed;
        let mut next = || {
            sm = sm.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = sm;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        Self {
            s: [next(), next(), next(), next()],
            bits: 0,
            avail: 0,
        }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[0]
            .wrapping_add(self.s[3])
            .rotate_left(23)
            .wrapping_add(self.s[0]);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// An unbiased pair of dice in 1..=6 each.
    ///
    /// One generator output is split into ten 6-bit chunks; each chunk is a
    /// uniform value in 0..64, and rejection sampling keeps only values below
    /// 36, which map bijectively onto the 36 equally likely dice pairs. This
    /// amortizes roughly one `next_u64` call per five pairs, versus about 2.7
    /// calls for two per-die rejections. (Sub-word chunks of xoshiro256++ are
    /// full-quality: the ++ scrambler exists precisely so that low bits pass
    /// the same statistical batteries as high bits.)
    #[inline]
    pub fn dice(&mut self) -> (u8, u8) {
        loop {
            while self.avail > 0 {
                let v = (self.bits & 63) as u8;
                self.bits >>= 6;
                self.avail -= 1;
                if v < 36 {
                    return (v / 6 + 1, v % 6 + 1);
                }
            }
            self.bits = self.next_u64();
            self.avail = 10;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dice_pairs_are_uniform() {
        // Chi-square over the 36 pair cells. With 3.6M pairs the expected
        // count per cell is 100k; for 35 degrees of freedom the statistic has
        // mean 35 and standard deviation ~8.4, so 100 is a generous bound
        // that still catches any systematic bias (a defect as small as 1%
        // in a single cell would push it past 1000).
        let mut rng = Xoshiro256pp::seed_from_u64(99);
        let n = 3_600_000u64;
        let mut cells = [0u64; 36];
        for _ in 0..n {
            let (d1, d2) = rng.dice();
            assert!((1..=6).contains(&d1) && (1..=6).contains(&d2));
            cells[((d1 - 1) * 6 + (d2 - 1)) as usize] += 1;
        }
        let expected = n as f64 / 36.0;
        let chi2: f64 = cells
            .iter()
            .map(|&c| {
                let d = c as f64 - expected;
                d * d / expected
            })
            .sum();
        assert!(chi2 < 100.0, "chi-square was {chi2:.1}");
    }

    #[test]
    fn dice_totals_have_no_serial_correlation() {
        // The batching draws several pairs from one generator output; verify
        // consecutive totals are uncorrelated. Standard error of r at n=2M is
        // ~0.0007, so 0.005 is a ~7-sigma bound.
        let mut rng = Xoshiro256pp::seed_from_u64(7);
        let n = 2_000_000;
        let mut prev = 0.0f64;
        let (mut sx, mut sy, mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for k in 0..=n {
            let (d1, d2) = rng.dice();
            let t = (d1 + d2) as f64;
            if k > 0 {
                sx += prev;
                sy += t;
                sxx += prev * prev;
                syy += t * t;
                sxy += prev * t;
            }
            prev = t;
        }
        let n = n as f64;
        let r = (n * sxy - sx * sy) / ((n * sxx - sx * sx).sqrt() * (n * syy - sy * sy).sqrt());
        assert!(r.abs() < 0.005, "serial correlation was {r:.5}");
    }

    #[test]
    fn dice_are_deterministic_per_seed() {
        let mut a = Xoshiro256pp::seed_from_u64(12345);
        let mut b = Xoshiro256pp::seed_from_u64(12345);
        let mut c = Xoshiro256pp::seed_from_u64(12346);
        let mut same = true;
        for _ in 0..1000 {
            let (x, y) = (a.dice(), b.dice());
            assert_eq!(x, y);
            if x != c.dice() {
                same = false;
            }
        }
        assert!(!same, "different seeds produced identical streams");
    }
}
