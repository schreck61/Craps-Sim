// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Brush readout: the exact probability-mass label a body renders while a
//! range is brushed. The brush geometry itself lives in the frame
//! ([`super::frame::FrameState`]); this is the shared reporting format so
//! every chart states mass the same way.

use crate::ui::numerals;

/// `P($200–$400) = 11.3% · 135 891 sessions`, computed exactly from a
/// sorted column via `partition_point`.
pub fn mass_label_money(sorted: &[i64], lo_cents: i64, hi_cents: i64) -> String {
    let (count, frac) = mass_between(sorted, lo_cents, hi_cents);
    format!(
        "P({}–{}) = {:.1}% · {} sessions",
        numerals::money_text(lo_cents, false),
        numerals::money_text(hi_cents, false),
        frac * 100.0,
        numerals::compact_n(count)
    )
}

/// Exact count and fraction of values in `[lo, hi]`.
pub fn mass_between(sorted: &[i64], lo: i64, hi: i64) -> (u64, f64) {
    if sorted.is_empty() {
        return (0, 0.0);
    }
    let a = sorted.partition_point(|&v| v < lo);
    let b = sorted.partition_point(|&v| v <= hi);
    let count = (b - a) as u64;
    (count, count as f64 / sorted.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mass_is_inclusive_and_exact() {
        let sorted: Vec<i64> = (0..1000).map(|i| i * 10).collect();
        let (n, f) = mass_between(&sorted, 100, 199);
        assert_eq!(n, 10); // 100,110,…,190
        assert!((f - 0.01).abs() < 1e-12);
        let (n, _) = mass_between(&sorted, -50, 5);
        assert_eq!(n, 1); // just 0
        assert_eq!(mass_between(&[], 0, 10).0, 0);
    }
}
