// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Small, fast, self-contained PRNG (xoshiro256++), seeded via SplitMix64.
//! Deterministic per seed, so every session is reproducible from its index.

pub struct Xoshiro256pp {
    s: [u64; 4],
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

    /// Unbiased die roll in 1..=6 via rejection sampling on the top bits.
    #[inline]
    pub fn die(&mut self) -> u8 {
        loop {
            // 3 random bits give 0..8; reject 6 and 7.
            let v = (self.next_u64() >> 61) as u8;
            if v < 6 {
                return v + 1;
            }
        }
    }
}
