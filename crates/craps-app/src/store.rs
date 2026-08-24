// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The run store: every byte of resident simulation data, and every query
//! the charts ask of it.
//!
//! One collector thread is the sole writer (see `stream.rs`); the UI thread
//! takes the mutex briefly to copy digests at poll cadence and to run
//! O(log n) interactive queries against the focused minimum's sorted views.
//! Canonical columns are session-ordered SoA vectors, always resident for
//! every minimum; sorted views exist for the focused minimum only and are
//! rebuilt on focus change.

use craps_engine::{
    budget_for_confidence, quantile_table, record_flags, summarize_horizon, summarize_ruin,
    ExploreRow, HorizonStats, RuinStats, SessionRecord, StreamHist, Welford, QUANTILE_POINTS,
};

use crate::config::SimConfig;

/// Everything a number needs to say where it came from.
#[derive(Clone, Debug)]
pub struct RunProvenance {
    pub seed: u64,
    pub config: SimConfig,
    pub fingerprint: u64,
    pub engine_version: &'static str,
    /// Sessions requested per table minimum.
    pub sessions: u64,
}

impl RunProvenance {
    /// Identity for per-chart interaction state: a new run (new settings or
    /// new seed) yields a new key, so zoom/pins/brush reset and the charts
    /// recenter on the fresh distribution. Never 0 (0 = "never reset").
    pub fn run_key(&self) -> u64 {
        (self.fingerprint ^ self.seed.rotate_left(17)) | 1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    /// Sessions still arriving; `n` is the count received so far (all mins).
    Streaming {
        n: u64,
    },
    Complete,
    /// Cancelled mid-run; `n` sessions were kept and summarized honestly.
    Cancelled {
        n: u64,
    },
}

/// How raw data is being retained for this run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetentionMode {
    /// Every session's record is resident.
    Full,
    /// Beyond [`RunStore::MAX_FULL_SLOTS`] total slots, only every `stride`-th
    /// session is kept raw (digests still see everything). Every affected
    /// chart wears its `sampled n of N` badge.
    Sampled { stride: u64 },
}

/// Canonical, session-ordered columns (SoA). Pre-sized at run start; the
/// collector writes each record at its session index, so order is restored
/// no matter how batches interleave.
#[derive(Clone, Debug, Default)]
pub struct Columns {
    pub rolls: Vec<u32>,
    pub finals: Vec<i64>,
    pub outlays: Vec<i64>,
    pub flags: Vec<u8>,
    /// Contiguous written ranges `[start, end)`, merged opportunistically.
    /// With full completion this collapses to one range; after a cancel it
    /// tells the finalizer exactly which slots are real.
    pub ranges: Vec<(u32, u32)>,
}

impl Columns {
    pub fn new(n: usize) -> Self {
        Self {
            rolls: vec![0; n],
            finals: vec![0; n],
            outlays: vec![0; n],
            flags: vec![0; n],
            ranges: Vec::new(),
        }
    }

    pub fn write(&mut self, rec: &SessionRecord) {
        let i = rec.session as usize;
        self.rolls[i] = rec.rolls;
        self.finals[i] = rec.final_cents;
        self.outlays[i] = rec.peak_outlay_cents;
        self.flags[i] = rec.flags;
    }

    pub fn note_range(&mut self, start: u32, end: u32) {
        if end <= start {
            return;
        }
        // Merge with an adjacent range if possible; batches arrive roughly
        // in order, so this keeps the list tiny.
        for r in &mut self.ranges {
            if r.1 == start {
                r.1 = end;
                return;
            }
            if end == r.0 {
                r.0 = start;
                return;
            }
        }
        self.ranges.push((start, end));
    }

    pub fn written(&self) -> u64 {
        self.ranges.iter().map(|&(s, e)| (e - s) as u64).sum()
    }

    /// Iterate the written session indices, in ascending order.
    pub fn written_indices(&self) -> impl Iterator<Item = usize> + '_ {
        let mut ranges = self.ranges.clone();
        ranges.sort_unstable();
        ranges
            .into_iter()
            .flat_map(|(s, e)| (s as usize)..(e as usize))
    }

    pub fn resident_bytes(&self) -> usize {
        self.rolls.capacity() * 4
            + self.finals.capacity() * 8
            + self.outlays.capacity() * 8
            + self.flags.capacity()
    }
}

/// Sorted views for interactive exact queries — focused minimum only.
#[derive(Debug, Default)]
pub struct SortedViews {
    pub finals: Vec<i64>,
    pub finals_rank_to_id: Vec<u32>,
    pub outlays: Vec<i64>,
    pub outlays_rank_to_id: Vec<u32>,
    pub rolls: Vec<u32>,
    pub rolls_rank_to_id: Vec<u32>,
}

impl SortedViews {
    pub fn build(cols: &Columns) -> Self {
        fn sort_pairs<T: Copy + Ord>(
            cols: &Columns,
            get: impl Fn(&Columns, usize) -> T,
        ) -> (Vec<T>, Vec<u32>) {
            let mut idx: Vec<u32> = cols.written_indices().map(|i| i as u32).collect();
            idx.sort_by_key(|&i| get(cols, i as usize));
            let vals = idx.iter().map(|&i| get(cols, i as usize)).collect();
            (vals, idx)
        }
        let (finals, finals_rank_to_id) = sort_pairs(cols, |c, i| c.finals[i]);
        let (outlays, outlays_rank_to_id) = sort_pairs(cols, |c, i| c.outlays[i]);
        let (rolls, rolls_rank_to_id) = sort_pairs(cols, |c, i| c.rolls[i]);
        Self {
            finals,
            finals_rank_to_id,
            outlays,
            outlays_rank_to_id,
            rolls,
            rolls_rank_to_id,
        }
    }

    pub fn resident_bytes(&self) -> usize {
        self.finals.capacity() * 8
            + self.outlays.capacity() * 8
            + self.rolls.capacity() * 4
            + (self.finals_rank_to_id.capacity()
                + self.outlays_rank_to_id.capacity()
                + self.rolls_rank_to_id.capacity())
                * 4
    }

    /// Fraction of sessions whose value is at least `v` (exact, O(log n)).
    pub fn frac_at_least(sorted: &[i64], v: i64) -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        let idx = sorted.partition_point(|&x| x < v);
        (sorted.len() - idx) as f64 / sorted.len() as f64
    }
}

/// Streaming display digests, updated per record by the collector. These are
/// DISPLAY-ONLY during streaming: intermediate frames show whatever prefix
/// happened to arrive. Final published statistics are recomputed from the
/// canonical columns on completion, which is what makes them bit-deterministic
/// per seed.
#[derive(Debug)]
pub struct Digests {
    pub finals_hist: StreamHist,
    pub rolls_hist: StreamHist,
    pub outlays_hist: StreamHist,
    pub finals_mean: Welford,
    pub finals_q: Option<Box<[i64; QUANTILE_POINTS]>>,
    pub rolls_q: Option<Box<[i64; QUANTILE_POINTS]>>,
    pub outlays_q: Option<Box<[i64; QUANTILE_POINTS]>>,
    pub busted: u64,
    pub censored: u64,
    pub ruin_target: u64,
    pub horizon_target: u64,
    pub handle_cents_sum: i64,
    pub horizon_rolls_sum: u64,
}

impl Digests {
    pub fn new(budget_cents: i64, horizon_rolls: u64) -> Self {
        Self {
            // Whole-dollar bins: finals quantize to bet-size steps, and a
            // sub-dollar bin width aliases against them (uneven combs).
            finals_hist: StreamHist::new(((budget_cents * 4 / 512).max(1) + 99) / 100 * 100),
            rolls_hist: StreamHist::new(((horizon_rolls * 2 / 512) as i64).max(1)),
            outlays_hist: StreamHist::new(((budget_cents * 2 / 512).max(1) + 99) / 100 * 100),
            finals_mean: Welford::default(),
            finals_q: None,
            rolls_q: None,
            outlays_q: None,
            busted: 0,
            censored: 0,
            ruin_target: 0,
            horizon_target: 0,
            handle_cents_sum: 0,
            horizon_rolls_sum: 0,
        }
    }

    pub fn push(&mut self, rec: &SessionRecord) {
        self.finals_hist.push(rec.final_cents);
        self.rolls_hist.push(rec.rolls as i64);
        self.outlays_hist.push(rec.peak_outlay_cents);
        self.finals_mean.push(rec.final_cents as f64);
        if rec.flags & record_flags::BUSTED != 0 {
            self.busted += 1;
        }
        if rec.flags & record_flags::CENSORED != 0 {
            self.censored += 1;
        }
        if rec.flags & record_flags::RUIN_TARGET != 0 {
            self.ruin_target += 1;
        }
        if rec.flags & record_flags::HORIZON_TARGET != 0 {
            self.horizon_target += 1;
        }
    }
}

/// One progression's Horizon digest for the Anchor (Gasp 4): the same
/// dice, only the pressing changed — ~5 KB per variant.
#[derive(Clone, Debug)]
pub struct AnchorVariant {
    pub counts: Vec<u32>,
    pub bin_width: i64,
    pub busts: u64,
    pub n: u64,
    pub mean: f64,
    pub mean_ci: f64,
    pub median: i64,
    /// Mean resolved handle per session, cents — what drifts the
    /// expected-loss pin.
    pub mean_handle_cents: f64,
}

/// The wealth fan's precomputed envelope: percentiles per roll over a
/// deterministic 1,000-session traced subsample, plus a few full paths.
/// Sessions that end early are carried at their final wealth (money in
/// hand is frozen, not deleted).
#[derive(Clone, Debug)]
pub struct FanData {
    /// Per roll: [P5, P25, P50, P75, P95] of wealth, cents.
    pub bands: Vec<[i64; 5]>,
    pub paths: Vec<Vec<i64>>,
    pub sampled_n: u64,
    pub total_n: u64,
}

/// Final published summary for one minimum, recomputed from canonical
/// columns via the engine's `summarize_*` path on completion.
#[derive(Clone, Debug)]
pub struct MinSummary {
    pub ruin: RuinStats,
    pub horizon: HorizonStats,
    /// Recommended budget at the run's confidence, rounded up to a dollar.
    pub budget_cents: i64,
    /// Sessions actually summarized (differs from requested after a cancel).
    pub n: u64,
}

#[derive(Debug)]
pub struct MinRun {
    pub min_cents: i64,
    pub cols: Columns,
    pub digests: Digests,
    pub summary: Option<MinSummary>,
    /// Sorted views, present only while this minimum is focused.
    pub sorted: Option<SortedViews>,
    /// Wealth-fan envelope, built after the minimum completes.
    pub fan: Option<FanData>,
}

impl MinRun {
    /// Recompute the published summary from canonical columns (exact,
    /// order-canonical: `summarize_*` sorts internally).
    pub fn finalize(&mut self, confidence: f64) {
        let n = self.cols.written();
        if n == 0 {
            self.summary = None;
            return;
        }
        let mut rolls: Vec<u64> = Vec::with_capacity(n as usize);
        let mut finals: Vec<i64> = Vec::with_capacity(n as usize);
        let mut outlays: Vec<i64> = Vec::with_capacity(n as usize);
        let mut censored = 0u64;
        let mut ruin_target = 0u64;
        let mut busted = 0u64;
        let mut horizon_target = 0u64;
        for i in self.cols.written_indices() {
            rolls.push(self.cols.rolls[i] as u64);
            finals.push(self.cols.finals[i]);
            outlays.push(self.cols.outlays[i]);
            let f = self.cols.flags[i];
            censored += (f & record_flags::CENSORED != 0) as u64;
            ruin_target += (f & record_flags::RUIN_TARGET != 0) as u64;
            busted += (f & record_flags::BUSTED != 0) as u64;
            horizon_target += (f & record_flags::HORIZON_TARGET != 0) as u64;
        }
        let ruin = summarize_ruin(&mut rolls, censored, ruin_target);
        let horizon = summarize_horizon(&mut finals, busted, horizon_target);
        let budget = budget_for_confidence(&mut outlays, confidence);
        let budget = ((budget + 99) / 100) * 100;
        // The sorted scratch vectors double as quantile-digest sources.
        self.digests.rolls_q = Some(quantile_table(
            &rolls.iter().map(|&r| r as i64).collect::<Vec<_>>(),
        ));
        self.digests.finals_q = Some(quantile_table(&finals));
        self.digests.outlays_q = Some(quantile_table(&outlays));
        self.summary = Some(MinSummary {
            ruin,
            horizon,
            budget_cents: budget,
            n,
        });
    }
}

#[derive(Debug)]
pub struct RunStore {
    pub provenance: RunProvenance,
    pub status: RunStatus,
    pub retention: RetentionMode,
    pub mins: Vec<MinRun>,
    /// Index of the focused minimum (sorted views live there).
    pub focused: usize,
}

impl RunStore {
    /// Above this many total session-slots (sessions × minimums), raw
    /// retention switches to stride sampling: 2.5M sessions × 6 minimums.
    pub const MAX_FULL_SLOTS: u64 = 15_000_000;
    /// Target raw records per minimum in sampled mode.
    pub const SAMPLED_TARGET: u64 = 100_000;

    pub fn new(config: SimConfig, seed: u64) -> Self {
        let sessions = config.sessions.max(100) as u64;
        let mins = config.table_mins_cents.clone();
        let slots = sessions * mins.len() as u64;
        let retention = if slots > Self::MAX_FULL_SLOTS {
            RetentionMode::Sampled {
                stride: sessions.div_ceil(Self::SAMPLED_TARGET),
            }
        } else {
            RetentionMode::Full
        };
        let col_slots = match retention {
            RetentionMode::Full => sessions as usize,
            RetentionMode::Sampled { stride } => (sessions.div_ceil(stride)) as usize,
        };
        let budget = config.budget_cents;
        let horizon = config.horizon_rolls();
        let fingerprint = config.fingerprint();
        let min_runs = mins
            .iter()
            .map(|&m| MinRun {
                min_cents: m,
                cols: Columns::new(col_slots),
                digests: Digests::new(budget, horizon),
                summary: None,
                sorted: None,
                fan: None,
            })
            .collect();
        Self {
            provenance: RunProvenance {
                seed,
                config,
                fingerprint,
                engine_version: craps_engine::ENGINE_VERSION,
                sessions,
            },
            status: RunStatus::Streaming { n: 0 },
            retention,
            mins: min_runs,
            focused: 0,
        }
    }

    /// The retention stride (1 in Full mode).
    pub fn stride(&self) -> u64 {
        match self.retention {
            RetentionMode::Full => 1,
            RetentionMode::Sampled { stride } => stride,
        }
    }

    /// Map a raw-column slot index back to its TRUE session index. In
    /// Sampled retention the columns hold every `stride`-th session at
    /// compressed slots; anything that replays a session by id must route
    /// through this.
    pub fn true_session(&self, slot: u32) -> u64 {
        slot as u64 * self.stride()
    }

    /// Point focus at a minimum, dropping the old sorted views. The heavy
    /// rebuild happens OFF-LOCK in `stream::MainRun::focus`; this only
    /// flips the index (the UI must never wait on a sort).
    pub fn set_focus(&mut self, mi: usize) {
        if mi >= self.mins.len() {
            return;
        }
        if self.focused != mi {
            self.mins[self.focused].sorted = None;
        }
        self.focused = mi;
    }

    /// Total resident bytes of raw columns, sorted views, and digests —
    /// asserted against the documented memory budget in tests.
    pub fn resident_bytes(&self) -> usize {
        self.mins
            .iter()
            .map(|m| {
                m.cols.resident_bytes()
                    + m.sorted.as_ref().map_or(0, |s| s.resident_bytes())
                    + 3 * 512 * 4 // stream histograms
                    + 3 * QUANTILE_POINTS * 8 // quantile tables (when built)
            })
            .sum()
    }
}

// ---------------------------------------------------------------------------
// Explorer store
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExploreStatus {
    Streaming,
    Complete,
    Cancelled,
}

#[derive(Debug)]
pub struct ExploreStore {
    pub seed: u64,
    pub fingerprint: u64,
    pub config: SimConfig,
    pub status: ExploreStatus,
    /// Completed minimums: (min_cents, rows, partial).
    pub mins: Vec<(i64, Vec<ExploreRow>, bool)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use craps_engine::record_flags;

    fn rec(session: u32, rolls: u32, fin: i64, outlay: i64, flags: u8) -> SessionRecord {
        SessionRecord {
            session,
            rolls,
            final_cents: fin,
            peak_outlay_cents: outlay,
            flags,
        }
    }

    #[test]
    fn columns_restore_canonical_order_from_shuffled_batches() {
        let mut cols = Columns::new(100);
        // Write two out-of-order ranges plus a straggler in between.
        for i in (50..100).rev() {
            cols.write(&rec(i, i, i as i64 * 10, i as i64, 0));
        }
        cols.note_range(50, 100);
        for i in 0..50 {
            cols.write(&rec(i, i, i as i64 * 10, i as i64, 0));
        }
        cols.note_range(0, 50);
        assert_eq!(cols.written(), 100);
        assert_eq!(cols.ranges.len(), 1, "adjacent ranges merged");
        let idx: Vec<usize> = cols.written_indices().collect();
        assert_eq!(idx, (0..100).collect::<Vec<_>>());
        assert_eq!(cols.finals[7], 70);
    }

    #[test]
    fn sorted_views_are_consistent_with_columns() {
        let mut cols = Columns::new(10);
        let vals = [5i64, 3, 9, 1, 7, 2, 8, 0, 6, 4];
        for (i, &v) in vals.iter().enumerate() {
            cols.write(&rec(i as u32, (10 - i) as u32, v * 100, v, 0));
        }
        cols.note_range(0, 10);
        let sv = SortedViews::build(&cols);
        assert_eq!(sv.finals, (0..10).map(|v| v * 100).collect::<Vec<i64>>());
        // rank_to_id round-trips: the session at rank k holds the k-th value.
        for (k, &id) in sv.finals_rank_to_id.iter().enumerate() {
            assert_eq!(cols.finals[id as usize], sv.finals[k]);
        }
        assert_eq!(SortedViews::frac_at_least(&sv.finals, 500), 0.5);
    }

    #[test]
    fn finalize_summarizes_only_written_slots() {
        let mut m = MinRun {
            min_cents: 500,
            cols: Columns::new(100),
            digests: Digests::new(30_000, 400),
            summary: None,
            sorted: None,
            fan: None,
        };
        // Only 40 sessions arrived before a cancel.
        for i in 0..40u32 {
            let r = rec(
                i,
                100 + i,
                (i as i64 + 1) * 1000,
                500,
                if i < 4 { record_flags::BUSTED } else { 0 },
            );
            m.cols.write(&r);
            m.digests.push(&r);
        }
        m.cols.note_range(0, 40);
        m.finalize(95.0);
        let s = m.summary.as_ref().unwrap();
        assert_eq!(s.n, 40);
        assert!((s.horizon.bust_frac - 0.10).abs() < 1e-12);
        // The zero-filled unwritten slots must not contaminate the median.
        assert!(s.horizon.deciles[4] >= 20_000, "median from written only");
    }

    #[test]
    fn retention_switches_to_sampling_above_the_guardrail() {
        let c = SimConfig {
            sessions: 3_000_000, // × 6 minimums = 18M slots > 15M
            ..Default::default()
        };
        let store = RunStore::new(c, 1);
        match store.retention {
            RetentionMode::Sampled { stride } => {
                assert!(stride >= 30, "stride {stride}");
            }
            RetentionMode::Full => panic!("expected sampled retention"),
        }
        let store = RunStore::new(SimConfig::default(), 1);
        assert_eq!(store.retention, RetentionMode::Full);
    }

    /// The documented memory envelope: six minimums at 1.2M sessions ≈
    /// 190 MB with one focused minimum's sorted views.
    #[test]
    fn resident_bytes_matches_the_documented_envelope() {
        let c = SimConfig {
            sessions: 1_200_000,
            ..Default::default()
        };
        let mut store = RunStore::new(c, 1);
        // Simulate full columns + focused sorted views.
        for m in &mut store.mins {
            m.cols.note_range(0, 1_200_000);
        }
        store.set_focus(0);
        let bytes = store.resident_bytes();
        let mb = bytes as f64 / 1e6;
        // 6 mins × 1.2M × 21 B canonical = 151.2 MB; focused sorted views
        // add ~44 MB; digests are noise. Documented envelope: 150–200 MB.
        assert!(
            (150.0..205.0).contains(&mb),
            "resident {mb:.1} MB outside the documented envelope"
        );
    }
}
