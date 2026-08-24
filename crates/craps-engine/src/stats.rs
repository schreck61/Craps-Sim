// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Small, exact statistics utilities shared by the sweep, the explorer, and
//! the app's display layer: confidence intervals, a streaming mean, an
//! exactly-merging streaming histogram, quantile tables, and hex binning.

/// Half-width of the Wald 95% confidence interval for a proportion.
pub fn wald_ci_half(p: f64, n: u64) -> f64 {
    1.96 * (p * (1.0 - p) / n.max(1) as f64).sqrt()
}

/// Distribution-free 95% confidence interval for the median, from order
/// statistics: the interval between ranks `n/2 ± 1.96·√n/2` covers the true
/// median with ≥95% probability regardless of the distribution's shape.
/// Input must be sorted ascending.
pub fn median_ci_sorted(sorted: &[i64]) -> (i64, i64) {
    if sorted.is_empty() {
        return (0, 0);
    }
    let n = sorted.len();
    let half = 1.96 * (n as f64).sqrt() / 2.0;
    let mid = n as f64 / 2.0;
    let lo = (mid - half).floor().max(0.0) as usize;
    let hi = ((mid + half).ceil() as usize).min(n - 1);
    (sorted[lo], sorted[hi])
}

/// Welford's online mean/variance accumulator. Numerically stable; used for
/// the streaming empirical mean and its CI ribbon, and for explorer rows.
#[derive(Clone, Copy, Debug, Default)]
pub struct Welford {
    n: u64,
    mean: f64,
    m2: f64,
}

impl Welford {
    pub fn push(&mut self, x: f64) {
        self.n += 1;
        let d = x - self.mean;
        self.mean += d / self.n as f64;
        self.m2 += d * (x - self.mean);
    }

    pub fn count(&self) -> u64 {
        self.n
    }

    pub fn mean(&self) -> f64 {
        self.mean
    }

    /// Sample variance (n − 1 denominator).
    pub fn variance(&self) -> f64 {
        if self.n < 2 {
            0.0
        } else {
            self.m2 / (self.n - 1) as f64
        }
    }

    /// Half-width of the 95% CI on the mean.
    pub fn ci95_half(&self) -> f64 {
        if self.n < 2 {
            0.0
        } else {
            1.96 * (self.variance() / self.n as f64).sqrt()
        }
    }
}

/// Number of bins in every streaming histogram. Chosen once, product-wide:
/// enough for a smooth shape at chart widths up to ~2000 px, small enough
/// that a digest is ~2 KB.
pub const HIST_BINS: usize = 512;

/// A fixed-bin streaming histogram over non-negative values whose range is
/// unknown up front. When a value lands past the top bin the width doubles
/// and adjacent bins merge pairwise — counts merge exactly, so the histogram
/// is always a true (if coarser) binning of everything pushed so far.
#[derive(Clone, Debug)]
pub struct StreamHist {
    width: i64,
    counts: Vec<u32>,
    total: u64,
}

impl StreamHist {
    /// `initial_width` is the starting bin width (≥ 1); pick it so the
    /// expected range fills a good fraction of the 512 bins.
    pub fn new(initial_width: i64) -> Self {
        Self {
            width: initial_width.max(1),
            counts: vec![0; HIST_BINS],
            total: 0,
        }
    }

    /// Push one non-negative value (negatives clamp to bin zero).
    pub fn push(&mut self, v: i64) {
        let v = v.max(0);
        while v >= self.width.saturating_mul(HIST_BINS as i64) {
            self.fold();
        }
        self.counts[(v / self.width) as usize] += 1;
        self.total += 1;
    }

    /// Double the bin width, merging adjacent counts exactly.
    fn fold(&mut self) {
        for i in 0..HIST_BINS / 2 {
            self.counts[i] = self.counts[2 * i] + self.counts[2 * i + 1];
        }
        for c in &mut self.counts[HIST_BINS / 2..] {
            *c = 0;
        }
        self.width *= 2;
    }

    pub fn width(&self) -> i64 {
        self.width
    }

    pub fn counts(&self) -> &[u32] {
        &self.counts
    }

    pub fn total(&self) -> u64 {
        self.total
    }
}

/// Number of points in a quantile table digest.
pub const QUANTILE_POINTS: usize = 1024;

/// Exact 1024-point quantile table from sorted data: entry `k` is the value
/// at rank `round(k/1023 · (n−1))`. Non-focused chart overlays draw from
/// these — visually exact at any width up to ~800 px per panel.
pub fn quantile_table(sorted: &[i64]) -> Box<[i64; QUANTILE_POINTS]> {
    let mut q = Box::new([0i64; QUANTILE_POINTS]);
    if sorted.is_empty() {
        return q;
    }
    let n = sorted.len();
    for (k, slot) in q.iter_mut().enumerate() {
        let idx = ((k as f64 / (QUANTILE_POINTS - 1) as f64) * (n as f64 - 1.0)).round() as usize;
        *slot = sorted[idx.min(n - 1)];
    }
    q
}

/// One hexagonal bin in axial coordinates, with its center in data space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HexBin {
    pub center_x: f64,
    pub center_y: f64,
    pub count: u32,
}

/// Bin points into pointy-top hexagons for the Duel's y=x density plot.
/// `extent` is the shared (min, max) of both axes (the plot is square by
/// construction); `target_cols` is the approximate number of hex columns.
/// Returns the non-empty bins with centers in data space.
pub fn hexbin(points: &[(i64, i64)], extent: (f64, f64), target_cols: usize) -> Vec<HexBin> {
    use std::collections::HashMap;
    let (lo, hi) = extent;
    let span = (hi - lo).max(1.0);
    // Pointy-top hex: horizontal pitch = √3·size, vertical pitch = 1.5·size.
    let size = span / (target_cols.max(4) as f64 * 3f64.sqrt());
    let mut cells: HashMap<(i32, i32), u32> = HashMap::new();
    for &(x, y) in points {
        let (px, py) = (x as f64 - lo, y as f64 - lo);
        // Pixel → fractional axial coordinates (pointy-top orientation).
        let qf = (3f64.sqrt() / 3.0 * px - py / 3.0) / size;
        let rf = (2.0 / 3.0 * py) / size;
        let (q, r) = axial_round(qf, rf);
        *cells.entry((q, r)).or_insert(0) += 1;
    }
    let mut out: Vec<HexBin> = cells
        .into_iter()
        .map(|((q, r), count)| {
            let x = size * 3f64.sqrt() * (q as f64 + r as f64 / 2.0) + lo;
            let y = size * 1.5 * r as f64 + lo;
            HexBin {
                center_x: x,
                center_y: y,
                count,
            }
        })
        .collect();
    // Deterministic order for stable rendering and tests.
    out.sort_by(|a, b| {
        a.center_y
            .partial_cmp(&b.center_y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                a.center_x
                    .partial_cmp(&b.center_x)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    out
}

/// Round fractional axial hex coordinates to the containing hex (via cube
/// coordinates, standard rounding).
fn axial_round(qf: f64, rf: f64) -> (i32, i32) {
    let sf = -qf - rf;
    let (mut q, mut r, s) = (qf.round(), rf.round(), sf.round());
    let (dq, dr, ds) = ((q - qf).abs(), (r - rf).abs(), (s - sf).abs());
    if dq > dr && dq > ds {
        q = -r - s;
    } else if dr > ds {
        r = -q - s;
    }
    (q as i32, r as i32)
}

/// SplitMix64 — the same mixer the RNG seeds with, exposed for deterministic
/// display-side jitter (dot fields) keyed by session index. Never used for
/// dice.
pub fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wald_ci_matches_hand_computation() {
        // p=0.5, n=10000: 1.96·√(0.25/10000) = 0.0098.
        assert!((wald_ci_half(0.5, 10_000) - 0.0098).abs() < 1e-9);
        assert_eq!(wald_ci_half(0.0, 100), 0.0);
        // n=0 must not divide by zero.
        assert!(wald_ci_half(0.5, 0).is_finite());
    }

    #[test]
    fn median_ci_brackets_the_median() {
        let sorted: Vec<i64> = (1..=1001).collect();
        let (lo, hi) = median_ci_sorted(&sorted);
        assert!(lo < 501 && 501 < hi, "({lo}, {hi}) should bracket 501");
        // n=1001: half-width 1.96·√1001/2 ≈ 31 ranks.
        assert!((501 - lo) <= 33 && (hi - 501) <= 33, "({lo}, {hi})");
        assert_eq!(median_ci_sorted(&[]), (0, 0));
        assert_eq!(median_ci_sorted(&[7]), (7, 7));
    }

    #[test]
    fn welford_matches_two_pass() {
        let xs: Vec<f64> = (0..1000).map(|i| ((i * 37) % 101) as f64).collect();
        let mut w = Welford::default();
        for &x in &xs {
            w.push(x);
        }
        let mean = xs.iter().sum::<f64>() / xs.len() as f64;
        let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (xs.len() as f64 - 1.0);
        assert!((w.mean() - mean).abs() < 1e-9);
        assert!((w.variance() - var).abs() < 1e-6);
        assert_eq!(w.count(), 1000);
    }

    #[test]
    fn stream_hist_folds_exactly() {
        let mut h = StreamHist::new(1);
        // Values 0..512 fit at width 1; then 5000 forces folds.
        for v in 0..512i64 {
            h.push(v);
        }
        assert_eq!(h.width(), 1);
        assert_eq!(h.total(), 512);
        h.push(5000);
        // Width doubled until 5000 fits: 1→2→4→8→16 (512·16 = 8192 > 5000).
        assert_eq!(h.width(), 16);
        assert_eq!(h.total(), 513);
        // Every original count survived the folds.
        let sum: u64 = h.counts().iter().map(|&c| c as u64).sum();
        assert_eq!(sum, 513);
        // First 32 bins now hold 16 each of the original 512 values.
        assert!(h.counts()[..32].iter().all(|&c| c == 16));
        assert_eq!(h.counts()[(5000 / 16) as usize], 1);
    }

    #[test]
    fn quantile_table_endpoints_and_median() {
        let sorted: Vec<i64> = (0..10_000).collect();
        let q = quantile_table(&sorted);
        assert_eq!(q[0], 0);
        assert_eq!(q[QUANTILE_POINTS - 1], 9999);
        let mid = q[QUANTILE_POINTS / 2];
        assert!((mid - 5000).abs() < 20, "median point was {mid}");
    }

    #[test]
    fn hexbin_conserves_count_and_separates_clusters() {
        let mut pts: Vec<(i64, i64)> = Vec::new();
        for i in 0..500 {
            pts.push((100 + (i % 7), 100 + (i % 5))); // tight cluster A
            pts.push((900 + (i % 7), 900 + (i % 5))); // tight cluster B
        }
        let bins = hexbin(&pts, (0.0, 1000.0), 20);
        let total: u32 = bins.iter().map(|b| b.count).sum();
        assert_eq!(total as usize, pts.len());
        // Two far-apart clusters must not share a hex.
        let near_a: u32 = bins
            .iter()
            .filter(|b| b.center_x < 500.0)
            .map(|b| b.count)
            .sum();
        assert_eq!(near_a, 500);
    }

    #[test]
    fn splitmix64_is_deterministic_and_mixing() {
        assert_eq!(splitmix64(1), splitmix64(1));
        assert_ne!(splitmix64(1), splitmix64(2));
        // Low bits of consecutive indices decorrelate.
        let a = (0..64).filter(|&i| splitmix64(i) & 1 == 1).count();
        assert!(a > 16 && a < 48, "bit bias: {a}/64");
    }
}

#[cfg(test)]
mod perf_smoke {
    use super::*;

    /// The collector's per-batch histogram work must stay far below the
    /// frame budget: 4,096 pushes in under a millisecond (release). Debug
    /// builds get a slack bound so the default suite stays green.
    #[test]
    fn bin_accumulation_smoke() {
        let mut h = StreamHist::new(100);
        let t0 = std::time::Instant::now();
        for i in 0..4096i64 {
            h.push((i * 37) % 120_000);
        }
        let dt = t0.elapsed();
        let bound = if cfg!(debug_assertions) { 20 } else { 1 };
        assert!(
            dt.as_millis() < bound,
            "4096 pushes took {dt:?} (bound {bound} ms)"
        );
        assert_eq!(h.total(), 4096);
    }
}
