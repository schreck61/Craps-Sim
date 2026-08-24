// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Statistics over batches of session outcomes.

#[derive(Clone, Debug)]
pub struct RuinStats {
    pub mean: f64,
    pub ci95_half: f64,
    pub p5: u64,
    pub p25: u64,
    pub p50: u64,
    pub p75: u64,
    pub p95: u64,
    pub censored_frac: f64,
    /// Fraction of sessions that ended by hitting the take-profit target.
    pub target_frac: f64,
}

/// Summarize a non-empty set of session lengths (sorted in place).
/// `censored`/`hit_target` are counts out of `rolls.len()`. Censored sessions
/// enter at the cap value, biasing the mean and upper percentiles low — the
/// censored fraction is reported so callers can surface that.
///
/// # Panics
///
/// Panics if `rolls` is empty.
pub fn summarize_ruin(rolls: &mut [u64], censored: u64, hit_target: u64) -> RuinStats {
    assert!(
        !rolls.is_empty(),
        "summarize_ruin needs at least one session"
    );
    rolls.sort_unstable();
    let n = rolls.len() as f64;
    let mean = rolls.iter().map(|&r| r as f64).sum::<f64>() / n;
    let var = rolls
        .iter()
        .map(|&r| {
            let d = r as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / (n - 1.0).max(1.0);
    let se = (var / n).sqrt();
    let pct = |p: f64| -> u64 {
        let idx = ((p / 100.0) * (rolls.len() as f64 - 1.0)).round() as usize;
        rolls[idx.min(rolls.len() - 1)]
    };
    RuinStats {
        mean,
        ci95_half: 1.96 * se,
        p5: pct(5.0),
        p25: pct(25.0),
        p50: pct(50.0),
        p75: pct(75.0),
        p95: pct(95.0),
        censored_frac: censored as f64 / n,
        target_frac: hit_target as f64 / n,
    }
}

#[derive(Clone, Debug)]
pub struct HorizonStats {
    pub mean: f64,
    pub bust_frac: f64,
    /// Fraction of sessions that quit early at the take-profit target.
    pub target_frac: f64,
    /// P10 through P90 of the ending bankroll, in cents.
    pub deciles: [i64; 9],
}

/// Summarize a non-empty set of ending bankrolls (sorted in place).
///
/// # Panics
///
/// Panics if `finals` is empty.
pub fn summarize_horizon(finals: &mut [i64], busted: u64, hit_target: u64) -> HorizonStats {
    assert!(
        !finals.is_empty(),
        "summarize_horizon needs at least one session"
    );
    finals.sort_unstable();
    let n = finals.len();
    let mut deciles = [0i64; 9];
    for (k, d) in deciles.iter_mut().enumerate() {
        let idx = (((k + 1) as f64 / 10.0) * (n as f64 - 1.0)).round() as usize;
        *d = finals[idx.min(n - 1)];
    }
    HorizonStats {
        mean: finals.iter().map(|&v| v as f64).sum::<f64>() / n as f64,
        bust_frac: busted as f64 / n as f64,
        target_frac: hit_target as f64 / n as f64,
        deciles,
    }
}

/// Percentile of the max-outlay distribution = budget needed to survive the
/// horizon with that probability.
/// `confidence` is a percentage in (0, 100]; `outlays` is sorted in place.
///
/// # Panics
///
/// Panics if `outlays` is empty.
pub fn budget_for_confidence(outlays: &mut [i64], confidence: f64) -> i64 {
    assert!(
        !outlays.is_empty(),
        "budget_for_confidence needs at least one session"
    );
    outlays.sort_unstable();
    let idx = ((confidence / 100.0) * (outlays.len() as f64 - 1.0)).ceil() as usize;
    outlays[idx.min(outlays.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_functions_on_known_inputs() {
        // Percentiles over 1..=100: index = round(p/100 * 99).
        let mut rolls: Vec<u64> = (1..=100).collect();
        let st = summarize_ruin(&mut rolls, 10, 5);
        assert_eq!(st.mean, 50.5);
        assert_eq!(st.p5, 6);
        assert_eq!(st.p25, 26);
        assert_eq!(st.p50, 51);
        assert_eq!(st.p75, 75);
        assert_eq!(st.p95, 95);
        assert_eq!(st.censored_frac, 0.10);
        assert_eq!(st.target_frac, 0.05);

        let mut one = vec![42u64];
        let st = summarize_ruin(&mut one, 0, 0);
        assert_eq!(st.mean, 42.0);
        assert_eq!(st.ci95_half, 0.0);
        assert_eq!((st.p5, st.p50, st.p95), (42, 42, 42));

        // Deciles over 1..=100: index = round(k/10 * 99).
        let mut finals: Vec<i64> = (1..=100).collect();
        let h = summarize_horizon(&mut finals, 20, 30);
        assert_eq!(h.mean, 50.5);
        assert_eq!(h.bust_frac, 0.20);
        assert_eq!(h.target_frac, 0.30);
        assert_eq!(h.deciles, [11, 21, 31, 41, 51, 60, 70, 80, 90]);

        // Budget percentile: index = ceil(conf/100 * 99).
        let mut outlays: Vec<i64> = (1..=100).collect();
        assert_eq!(budget_for_confidence(&mut outlays, 95.0), 96);
        assert_eq!(budget_for_confidence(&mut outlays, 100.0), 100);
        let mut single = vec![777i64];
        assert_eq!(budget_for_confidence(&mut single, 95.0), 777);
    }
}
