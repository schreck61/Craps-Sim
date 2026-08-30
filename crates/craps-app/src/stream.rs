// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Threading between the engine and the store.
//!
//! Four actors, one lock: the UI thread copies digests under a brief lock at
//! poll cadence; a coordinator thread owns the sweep (rayon inside); the
//! rayon pool talks only through the channel and atomics; a collector thread
//! is the sole writer to the [`RunStore`]. Heavy finalization work (sorting
//! 1.2M-session columns) happens on the collector thread OUTSIDE the lock —
//! the UI never waits on a sort.

use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use craps_engine::{run_explore, run_sweep, Batch, ExploreMin, SweepCtl};

use crate::config::SimConfig;
use crate::store::{
    ExploreStatus, ExploreStore, MinRun, RetentionMode, RunStatus, RunStore, SortedViews,
};

/// A running (or finished) main simulation.
pub struct MainRun {
    pub store: Arc<Mutex<RunStore>>,
    pub ctl: Arc<SweepCtl>,
    /// Sessions across all minimums (each session fuses both phases).
    pub total_sessions: u64,
    pub started: Instant,
}

impl MainRun {
    pub fn cancel(&self) {
        self.ctl
            .stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        matches!(
            self.store.lock().unwrap().status,
            RunStatus::Streaming { .. }
        )
    }

    /// Move focus to another minimum. The 40–70 ms sorted-view rebuild runs
    /// on a helper thread AND off-lock — the UI's brief per-frame locks
    /// never wait on a sort.
    pub fn focus(&self, mi: usize) {
        let store = self.store.clone();
        thread::spawn(move || {
            // Flip focus and snapshot the columns under a brief lock.
            let cols = {
                let mut st = store.lock().unwrap();
                st.set_focus(mi);
                match st.mins.get(mi) {
                    Some(m) if m.cols.written() > 0 && m.sorted.is_none() => m.cols.clone(),
                    _ => return,
                }
            };
            // Heavy sort off-lock.
            let sorted = SortedViews::build(&cols);
            // Install only if focus has not moved again meanwhile.
            let mut st = store.lock().unwrap();
            if st.focused == mi {
                if let Some(m) = st.mins.get_mut(mi) {
                    m.sorted = Some(sorted);
                }
            }
        });
    }
}

/// Start the fused main sweep: coordinator thread runs the engine, collector
/// thread reassembles batches into the store.
pub fn start_main_run(
    cfg: &SimConfig,
    seed: u64,
    program: Option<Arc<craps_engine::strategy::Program>>,
) -> MainRun {
    let store = Arc::new(Mutex::new(RunStore::new(cfg.clone(), seed)));
    let ctl = Arc::new(SweepCtl::default());
    let sweep_cfg = cfg.to_sweep(seed, program);
    let sessions = sweep_cfg.sessions;
    let n_mins = sweep_cfg.mins.len();
    let confidence = cfg.confidence;
    let (tx, rx) = sync_channel::<Batch>(256);

    {
        let ctl = ctl.clone();
        thread::spawn(move || run_sweep(&sweep_cfg, tx, &ctl));
    }
    {
        let store = store.clone();
        thread::spawn(move || collect(store, rx, sessions, n_mins, confidence));
    }

    MainRun {
        store,
        ctl,
        total_sessions: sessions * n_mins as u64,
        started: Instant::now(),
    }
}

fn collect(
    store: Arc<Mutex<RunStore>>,
    rx: std::sync::mpsc::Receiver<Batch>,
    sessions: u64,
    n_mins: usize,
    confidence: f64,
) {
    let mut received = vec![0u64; n_mins];
    let mut finalized = vec![false; n_mins];
    let stride = match store.lock().unwrap().retention {
        RetentionMode::Full => 1,
        RetentionMode::Sampled { stride } => stride,
    };

    for batch in rx.iter() {
        let mi = batch.min_index as usize;
        let first = batch.records.first().map(|r| r.session).unwrap_or(0);
        let len = batch.records.len() as u32;
        {
            let mut st = store.lock().unwrap();
            let m = &mut st.mins[mi];
            for rec in &batch.records {
                m.digests.push(rec);
                if stride == 1 {
                    m.cols.write(rec);
                } else if (rec.session as u64).is_multiple_of(stride) {
                    let mut slot_rec = *rec;
                    slot_rec.session = (rec.session as u64 / stride) as u32;
                    m.cols.write(&slot_rec);
                    m.cols.note_range(slot_rec.session, slot_rec.session + 1);
                }
            }
            if stride == 1 {
                m.cols.note_range(first, first + len);
            }
            m.digests.handle_cents_sum += batch.handle_cents_sum;
            m.digests.horizon_rolls_sum += batch.horizon_rolls_sum;
            received[mi] += batch.records.len() as u64;
            st.status = RunStatus::Streaming {
                n: received.iter().sum(),
            };
        }
        if received[mi] == sessions && !finalized[mi] {
            finalized[mi] = true;
            finalize_min(&store, mi, confidence);
        }
    }

    // Channel closed: the sweep finished or was cancelled. Finalize anything
    // partial at achieved n — CIs genuinely widen because they are
    // recomputed — then publish the terminal status.
    for (mi, done) in finalized.iter_mut().enumerate() {
        if !*done && received[mi] > 0 {
            *done = true;
            finalize_min(&store, mi, confidence);
        }
    }
    let total: u64 = received.iter().sum();
    let mut st = store.lock().unwrap();
    st.status = if total == sessions * n_mins as u64 {
        RunStatus::Complete
    } else {
        RunStatus::Cancelled { n: total }
    };
}

/// Recompute one minimum's published summary from canonical columns. The
/// expensive part (cloning + sorting) runs outside the store lock.
fn finalize_min(store: &Mutex<RunStore>, mi: usize, confidence: f64) {
    // Snapshot the columns under a brief lock (a memcpy, ~10 ms at 1.2M).
    let (cols, min_cents, is_focused, budget, horizon) = {
        let st = store.lock().unwrap();
        let m = &st.mins[mi];
        (
            m.cols.clone(),
            m.min_cents,
            st.focused == mi,
            st.provenance.config.budget_cents,
            st.provenance.config.horizon_rolls(),
        )
    };
    // Heavy work off-lock.
    let mut scratch = MinRun {
        min_cents,
        cols,
        digests: crate::store::Digests::new(budget, horizon),
        summary: None,
        sorted: None,
        fan: None,
    };
    scratch.finalize(confidence);
    let sorted = is_focused.then(|| SortedViews::build(&scratch.cols));
    // The wealth fan: a deterministic 1,000-session traced subsample,
    // replayed exactly (same seeds as the sweep's Main phase).
    let fan = {
        let st = store.lock().unwrap();
        let p = &st.provenance;
        let cfg = p.config.clone();
        let seed = p.seed;
        let total_n = p.sessions;
        drop(st);
        build_fan(&cfg, seed, mi as u32, min_cents, total_n)
    };
    // Install under a brief lock.
    let mut st = store.lock().unwrap();
    let still_focused = st.focused == mi;
    let m = &mut st.mins[mi];
    m.summary = scratch.summary;
    m.digests.finals_q = scratch.digests.finals_q;
    m.digests.rolls_q = scratch.digests.rolls_q;
    m.digests.outlays_q = scratch.digests.outlays_q;
    m.fan = Some(fan);
    // Install the views only if focus has not moved during the build.
    if still_focused {
        if let Some(sorted) = sorted {
            m.sorted = Some(sorted);
        }
    }
}

/// Cancels its job's workers when dropped, so replacing app state (a new
/// run, a new Duel pair) never leaves orphan simulations running.
pub struct JobGuard(pub Arc<SweepCtl>);

impl Drop for JobGuard {
    fn drop(&mut self) {
        self.0
            .stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Trace a deterministic stride subsample of sessions (identical dice to
/// the sweep) and reduce to per-roll wealth percentiles. Ended sessions
/// carry their final wealth forward — money in hand, frozen.
fn build_fan(
    cfg: &SimConfig,
    seed: u64,
    min_index: u32,
    min_cents: i64,
    total_n: u64,
) -> crate::store::FanData {
    use craps_engine::trace::trace_wealth;
    use craps_engine::{session_seed, SeedPhase};
    let horizon = cfg.horizon_rolls() as usize;
    let sample = 1_000u64.min(total_n.max(1));
    let stride = (total_n / sample).max(1);
    let rules = cfg.rules();
    let quit = cfg.quit_target_cents();
    let mut wealths: Vec<Vec<i64>> = Vec::with_capacity(sample as usize);
    for k in 0..sample {
        let i = k * stride;
        let (_, mut w) = trace_wealth(
            &cfg.sel,
            &rules,
            min_cents,
            cfg.budget_cents,
            quit,
            cfg.horizon_rolls(),
            cfg.horizon_rolls(),
            session_seed(seed, min_index, SeedPhase::Main, i),
        );
        let last = w.last().copied().unwrap_or(cfg.budget_cents);
        w.resize(horizon, last);
        wealths.push(w);
    }
    let mut bands = Vec::with_capacity(horizon);
    let mut column: Vec<i64> = vec![0; wealths.len()];
    for r in 0..horizon {
        for (j, w) in wealths.iter().enumerate() {
            column[j] = w[r];
        }
        column.sort_unstable();
        let pick = |p: f64| {
            column[((p * (column.len() - 1) as f64).round() as usize).min(column.len() - 1)]
        };
        bands.push([pick(0.05), pick(0.25), pick(0.50), pick(0.75), pick(0.95)]);
    }
    let paths = wealths.into_iter().take(12).collect();
    crate::store::FanData {
        bands,
        paths,
        sampled_n: sample,
        total_n,
    }
}

// ---------------------------------------------------------------------------
// The Anchor (Gasp 4): per-progression Horizon variants on identical dice
// ---------------------------------------------------------------------------

/// A running (or finished) Anchor variant computation. The morph is a
/// redraw of real data — an uncached variant shows this job's real
/// progress fill, never a fake instant re-simulation.
pub struct AnchorJob {
    pub cell: Arc<Mutex<Option<crate::store::AnchorVariant>>>,
    pub ctl: Arc<SweepCtl>,
    pub total: u64,
    /// Cancels the worker if the job is dropped (run restart, app close).
    /// Held for its `Drop`: dropping the job stops its worker.
    pub _guard: JobGuard,
}

/// Re-simulate the focused minimum's horizon with one progression swapped,
/// on the SAME dice (Main-phase seeds), reduced to a display digest.
pub fn start_anchor_job(
    cfg: &SimConfig,
    seed: u64,
    min_index: usize,
    prog: craps_engine::Progression,
) -> AnchorJob {
    use craps_engine::{run_session, session_seed, SeedPhase, StreamHist, Welford};
    use rayon::prelude::*;

    let cell: Arc<Mutex<Option<crate::store::AnchorVariant>>> = Arc::new(Mutex::new(None));
    let ctl = Arc::new(SweepCtl::default());
    let total = cfg.sessions.max(100) as u64;
    let out_cell = cell.clone();
    let job_ctl = ctl.clone();
    let mut sel = cfg.sel.clone();
    sel.progression = prog;
    let rules = cfg.rules();
    let budget = cfg.budget_cents;
    let quit = cfg.quit_target_cents();
    let horizon = cfg.horizon_rolls();
    let max_rolls = cfg.max_rolls.max(1000);
    let min_cents = cfg.table_mins_cents.get(min_index).copied().unwrap_or(1000);
    let mi = min_index as u32;

    thread::spawn(move || {
        let outs: Vec<(i64, bool, i64)> = (0..total)
            .into_par_iter()
            .map(|i| {
                if job_ctl.stop.load(std::sync::atomic::Ordering::Relaxed) {
                    return (0, false, 0);
                }
                let o = run_session(
                    &sel,
                    &rules,
                    min_cents,
                    budget,
                    quit,
                    max_rolls.min(horizon),
                    horizon,
                    session_seed(seed, mi, SeedPhase::Main, i),
                );
                // Coarse progress (every 1024th session) keeps the fill
                // honest without contended atomics.
                if i % 1024 == 0 {
                    job_ctl
                        .sessions_done
                        .fetch_add(1024, std::sync::atomic::Ordering::Relaxed);
                }
                (
                    o.horizon.final_cents,
                    o.horizon.busted,
                    o.horizon_handle_cents,
                )
            })
            .collect();
        if job_ctl.stop.load(std::sync::atomic::Ordering::Relaxed) {
            return; // cancelled: publish nothing, honestly
        }
        let mut hist = StreamHist::new(((budget * 4 / 512).max(1) + 99) / 100 * 100);
        let mut mean = Welford::default();
        let mut busts = 0u64;
        let mut handle_sum = 0i64;
        let mut finals: Vec<i64> = Vec::with_capacity(outs.len());
        for &(f, b, h) in &outs {
            hist.push(f);
            mean.push(f as f64);
            busts += b as u64;
            handle_sum += h;
            finals.push(f);
        }
        finals.sort_unstable();
        let variant = crate::store::AnchorVariant {
            counts: hist.counts().to_vec(),
            bin_width: hist.width(),
            busts,
            n: outs.len() as u64,
            mean: mean.mean(),
            mean_ci: mean.ci95_half(),
            median: finals[finals.len() / 2],
            mean_handle_cents: handle_sum as f64 / outs.len().max(1) as f64,
        };
        *out_cell.lock().unwrap() = Some(variant);
    });

    AnchorJob {
        cell,
        ctl: ctl.clone(),
        total,
        _guard: JobGuard(ctl),
    }
}

// ---------------------------------------------------------------------------
// Explorer
// ---------------------------------------------------------------------------

pub struct ExploreRun {
    pub store: Arc<Mutex<ExploreStore>>,
    pub ctl: Arc<SweepCtl>,
    /// Total sessions the full sweep will simulate (combos × sessions × mins).
    pub total_sessions: u64,
    pub started: Instant,
}

impl ExploreRun {
    pub fn cancel(&self) {
        self.ctl
            .stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        self.store.lock().unwrap().status == ExploreStatus::Streaming
    }
}

pub fn start_explore_run(
    cfg: &SimConfig,
    seed: u64,
    program: Option<Arc<craps_engine::strategy::Program>>,
) -> ExploreRun {
    let store = Arc::new(Mutex::new(ExploreStore {
        seed,
        fingerprint: cfg.explore_fingerprint(),
        config: cfg.clone(),
        status: ExploreStatus::Streaming,
        mins: Vec::new(),
    }));
    let ctl = Arc::new(SweepCtl::default());
    let explore_cfg = cfg.to_explore(seed, program);
    let n_mins = explore_cfg.mins.len() as u64;
    let combos = 11u64 * if explore_cfg.flat_only { 1 } else { 12 } * 4;
    let total_sessions = n_mins * combos * explore_cfg.sessions;
    let (tx, rx) = sync_channel::<ExploreMin>(16);

    {
        let ctl = ctl.clone();
        thread::spawn(move || run_explore(&explore_cfg, tx, &ctl));
    }
    {
        let store = store.clone();
        let ctl = ctl.clone();
        let expected_mins = n_mins as usize;
        thread::spawn(move || {
            let mut any_partial = false;
            let mut received = 0usize;
            for m in rx.iter() {
                let mut st = store.lock().unwrap();
                any_partial |= m.partial;
                received += 1;
                st.mins.push((m.min_cents, m.rows, m.partial));
            }
            // A cancel can land between minimums (nothing partial arrives)
            // — the stop flag and the shortfall both mean Cancelled.
            let cancelled = any_partial
                || received < expected_mins
                || ctl.stop.load(std::sync::atomic::Ordering::Relaxed);
            let mut st = store.lock().unwrap();
            st.status = if cancelled {
                ExploreStatus::Cancelled
            } else {
                ExploreStatus::Complete
            };
        });
    }

    ExploreRun {
        store,
        ctl,
        total_sessions,
        started: Instant::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end pipeline test: run a small sweep through the real threads
    /// and channels, wait for completion, and check the published summaries
    /// against a direct engine computation.
    #[test]
    fn pipeline_end_to_end_matches_direct_summaries() {
        let cfg = SimConfig {
            sessions: 5_000,
            table_mins_cents: vec![500, 1000],
            ..Default::default()
        };
        let run = start_main_run(&cfg, 0xDECAF, None);

        // Wait for completion (bounded).
        let deadline = Instant::now() + std::time::Duration::from_secs(60);
        loop {
            {
                let st = run.store.lock().unwrap();
                if st.status == RunStatus::Complete {
                    break;
                }
            }
            assert!(Instant::now() < deadline, "pipeline timed out");
            thread::sleep(std::time::Duration::from_millis(20));
        }

        let st = run.store.lock().unwrap();
        assert_eq!(st.mins.len(), 2);
        for (mi, m) in st.mins.iter().enumerate() {
            let s = m.summary.as_ref().expect("summary published");
            assert_eq!(s.n, 5_000);
            assert_eq!(m.cols.written(), 5_000);

            // Direct recomputation from the engine, same seeds.
            use craps_engine::{run_session, session_seed, SeedPhase};
            let sweep = cfg.to_sweep(0xDECAF, None);
            let mut finals: Vec<i64> = (0..5_000u64)
                .map(|i| {
                    run_session(
                        &sweep.sel,
                        &sweep.rules,
                        sweep.mins[mi],
                        sweep.budget_cents,
                        sweep.quit_target_cents,
                        sweep.max_rolls,
                        sweep.horizon_rolls,
                        session_seed(0xDECAF, mi as u32, SeedPhase::Main, i),
                    )
                    .horizon
                    .final_cents
                })
                .collect();
            finals.sort_unstable();
            let median = finals[finals.len() / 2];
            assert_eq!(
                s.horizon.deciles[4], median,
                "published median must be exact"
            );
            // Focused minimum carries sorted views; the other does not.
            if mi == 0 {
                let sv = m.sorted.as_ref().expect("focused sorted views");
                assert_eq!(sv.finals, finals);
            } else {
                assert!(m.sorted.is_none());
            }
        }
        drop(st);

        // Focus change rebuilds views on the other minimum.
        run.focus(1);
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        loop {
            {
                let st = run.store.lock().unwrap();
                if st.mins[1].sorted.is_some() && st.mins[0].sorted.is_none() {
                    break;
                }
            }
            assert!(Instant::now() < deadline, "focus change timed out");
            thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn cancelled_pipeline_publishes_partial_honestly() {
        let cfg = SimConfig {
            sessions: 2_000_000, // big enough to cancel mid-flight
            table_mins_cents: vec![500],
            ..Default::default()
        };
        let run = start_main_run(&cfg, 7, None);
        // Cancel only after batches have demonstrably landed: a fixed sleep
        // raced cold CI runners to an honest-but-untestable n == 0.
        let deadline = Instant::now() + std::time::Duration::from_secs(60);
        loop {
            {
                let st = run.store.lock().unwrap();
                if let RunStatus::Streaming { n } = st.status {
                    if n > 0 {
                        break;
                    }
                }
                if st.status == RunStatus::Complete {
                    panic!("run completed before any streaming snapshot");
                }
            }
            assert!(Instant::now() < deadline, "no sessions ever arrived");
            thread::sleep(std::time::Duration::from_millis(5));
        }
        run.cancel();
        loop {
            {
                let st = run.store.lock().unwrap();
                if let RunStatus::Cancelled { n } = st.status {
                    assert!(n > 0, "cancel kept nothing");
                    assert!(n < 2_000_000, "cancel too late to test partials");
                    let m = &st.mins[0];
                    let s = m.summary.as_ref().expect("partial summary");
                    assert_eq!(s.n, m.cols.written());
                    assert!(s.n <= n);
                    break;
                }
                if st.status == RunStatus::Complete {
                    panic!("run completed before cancel took effect");
                }
            }
            assert!(Instant::now() < deadline, "cancel timed out");
            thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn explorer_pipeline_completes_with_ci_rows() {
        let cfg = SimConfig {
            explore_sessions: 500,
            explore_flat_only: true,
            table_mins_cents: vec![1000],
            ..Default::default()
        };
        let run = start_explore_run(&cfg, 0xFACE, None);
        let deadline = Instant::now() + std::time::Duration::from_secs(60);
        loop {
            {
                let st = run.store.lock().unwrap();
                if st.status == ExploreStatus::Complete {
                    assert_eq!(st.mins.len(), 1);
                    let (min, rows, partial) = &st.mins[0];
                    assert_eq!(*min, 1000);
                    assert!(!partial);
                    assert_eq!(rows.len(), 44);
                    break;
                }
            }
            assert!(Instant::now() < deadline, "explorer timed out");
            thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
