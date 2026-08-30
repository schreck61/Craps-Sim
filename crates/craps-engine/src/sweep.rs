// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Batched, cancellable Monte Carlo sweeps.
//!
//! [`run_sweep`] answers the main questions (session length, ending bankroll,
//! peak outlay) by streaming fixed-size [`Batch`]es of per-session records
//! over a bounded channel — the UI's collector reassembles them into
//! canonical session order by index, so batching changes delivery, never
//! results. [`run_explore`] sweeps the strategy grid under common random
//! numbers, and [`run_pair`] replays two strategies on identical dice for
//! the Duel.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;

use rayon::prelude::*;

use crate::bets::{BetSelection, Progression, Rules};
use crate::session::{
    run_drawdown_session, run_horizon_session, run_program_drawdown_session, run_program_session,
    run_session, session_seed, SeedPhase,
};
use crate::stats::{median_ci_sorted, wald_ci_half, Welford};
use crate::strategy::Program;

/// Bit flags on [`SessionRecord::flags`].
pub mod record_flags {
    /// The ruin view hit the roll cap (censored).
    pub const CENSORED: u8 = 1;
    /// The ruin view ended by reaching the take-profit target.
    pub const RUIN_TARGET: u8 = 2;
    /// The horizon view busted before the horizon.
    pub const BUSTED: u8 = 4;
    /// The horizon view quit ahead at the take-profit target.
    pub const HORIZON_TARGET: u8 = 8;
}

/// One session's complete outcome across both fused phases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionRecord {
    /// Session index — the collector writes records at this index, restoring
    /// canonical order no matter how batches interleave across workers.
    pub session: u32,
    /// Rolls until the ruin view ended (bust, quit, or cap).
    pub rolls: u32,
    /// Ending bankroll of the horizon view, in cents.
    pub final_cents: i64,
    /// Peak cash outlay of the unconstrained drawdown phase, in cents.
    pub peak_outlay_cents: i64,
    /// See [`record_flags`].
    pub flags: u8,
}

/// A block of records for one table minimum.
#[derive(Clone, Debug)]
pub struct Batch {
    /// Index into [`SweepConfig::mins`].
    pub min_index: u32,
    pub records: Vec<SessionRecord>,
    /// Resolved handle (cents) summed over these records' horizon views —
    /// aggregated here so the 25-byte record stays lean.
    pub handle_cents_sum: i64,
    /// Rolls actually played by these records' horizon views, summed —
    /// the denominator for handle-per-hour.
    pub horizon_rolls_sum: u64,
    /// True when cancellation cut this batch short.
    pub partial: bool,
}

/// Shared cancellation and progress state for a running sweep.
#[derive(Debug, Default)]
pub struct SweepCtl {
    /// Set to cancel; checked once per session (a ~microsecond granularity).
    pub stop: AtomicBool,
    /// Sessions fully simulated (both phases) so far.
    pub sessions_done: AtomicU64,
    /// Total dice rolls simulated so far, across both phases — the numerator
    /// of the status odometer's rolls/s.
    pub rolls_done: AtomicU64,
}

#[derive(Clone, Debug)]
pub struct SweepConfig {
    pub sel: BetSelection,
    /// The compiled strategy to play, when one is live. `None` plays the
    /// checkbox selection above.
    ///
    /// Shared rather than cloned: one program serves every session on every
    /// thread, and it is immutable once compiled.
    pub program: Option<std::sync::Arc<Program>>,
    pub rules: Rules,
    /// Table minimums in cents, ascending.
    pub mins: Vec<i64>,
    pub budget_cents: i64,
    pub quit_target_cents: Option<i64>,
    pub sessions: u64,
    pub max_rolls: u64,
    pub horizon_rolls: u64,
    pub base_seed: u64,
}

/// Records per batch. Small enough that rayon load-balances the tail of a
/// sweep (ruin-session lengths vary wildly, so coarse chunks straggle),
/// large enough that channel sends stay amortized noise (~33 KB, a few
/// hundred sends per second at full speed).
pub const BATCH: u64 = 1024;

/// Run the fused main + drawdown sweep, streaming batches as they complete.
/// Deterministic per `base_seed`: batching affects delivery order only —
/// every record's content depends solely on its [`session_seed`]. Returns
/// when all minimums are done or `ctl.stop` is observed.
pub fn run_sweep(cfg: &SweepConfig, tx: SyncSender<Batch>, ctl: &SweepCtl) {
    for (mi, &min) in cfg.mins.iter().enumerate() {
        if ctl.stop.load(Ordering::Relaxed) {
            return;
        }
        let starts: Vec<u64> = (0..cfg.sessions).step_by(BATCH as usize).collect();
        starts
            .into_par_iter()
            .for_each_with(tx.clone(), |tx, start| {
                if ctl.stop.load(Ordering::Relaxed) {
                    return;
                }
                let end = (start + BATCH).min(cfg.sessions);
                let mut records = Vec::with_capacity((end - start) as usize);
                let mut handle_sum = 0i64;
                let mut horizon_rolls_sum = 0u64;
                let mut rolls_played = 0u64;
                let mut partial = false;
                for i in start..end {
                    if ctl.stop.load(Ordering::Relaxed) {
                        partial = true;
                        break;
                    }
                    let main_seed = session_seed(cfg.base_seed, mi as u32, SeedPhase::Main, i);
                    let dd_seed = session_seed(cfg.base_seed, mi as u32, SeedPhase::Drawdown, i);
                    // One branch per session on a pointer that never changes
                    // mid-sweep; the dice, the seeds, and everything else
                    // about the run are identical either way.
                    let (o, outlay) = match &cfg.program {
                        Some(p) => (
                            run_program_session(
                                p,
                                &cfg.rules,
                                min,
                                cfg.budget_cents,
                                cfg.quit_target_cents,
                                cfg.max_rolls,
                                cfg.horizon_rolls,
                                main_seed,
                            ),
                            run_program_drawdown_session(
                                p,
                                &cfg.rules,
                                min,
                                cfg.horizon_rolls,
                                dd_seed,
                            ),
                        ),
                        None => (
                            run_session(
                                &cfg.sel,
                                &cfg.rules,
                                min,
                                cfg.budget_cents,
                                cfg.quit_target_cents,
                                cfg.max_rolls,
                                cfg.horizon_rolls,
                                main_seed,
                            ),
                            run_drawdown_session(
                                &cfg.sel,
                                &cfg.rules,
                                min,
                                cfg.horizon_rolls,
                                dd_seed,
                            ),
                        ),
                    };
                    let mut flags = 0u8;
                    if o.ruin.censored {
                        flags |= record_flags::CENSORED;
                    }
                    if o.ruin.hit_target {
                        flags |= record_flags::RUIN_TARGET;
                    }
                    if o.horizon.busted {
                        flags |= record_flags::BUSTED;
                    }
                    if o.horizon.hit_target {
                        flags |= record_flags::HORIZON_TARGET;
                    }
                    records.push(SessionRecord {
                        session: i as u32,
                        rolls: o.ruin.rolls.min(u32::MAX as u64) as u32,
                        final_cents: o.horizon.final_cents,
                        peak_outlay_cents: outlay,
                        flags,
                    });
                    handle_sum += o.horizon_handle_cents;
                    horizon_rolls_sum += o.horizon.rolls;
                    rolls_played += o.ruin.rolls.max(o.horizon.rolls) + cfg.horizon_rolls;
                    ctl.sessions_done.fetch_add(1, Ordering::Relaxed);
                }
                ctl.rolls_done.fetch_add(rolls_played, Ordering::Relaxed);
                if !records.is_empty() {
                    // A closed channel means the collector is gone; just stop.
                    let _ = tx.send(Batch {
                        min_index: mi as u32,
                        records,
                        handle_cents_sum: handle_sum,
                        horizon_rolls_sum,
                        partial,
                    });
                }
            });
    }
}

// ---------------------------------------------------------------------------
// Strategy explorer
// ---------------------------------------------------------------------------

/// The curated betting strategies the explorer sweeps. Progression and quit
/// rule are crossed with these separately.
pub fn explore_strategies() -> Vec<(&'static str, BetSelection)> {
    let none = BetSelection {
        pass_line: false,
        ..Default::default()
    };
    let mk = |f: &dyn Fn(&mut BetSelection)| {
        let mut s = none.clone();
        f(&mut s);
        s
    };
    vec![
        ("Pass line", mk(&|s| s.pass_line = true)),
        (
            "Pass + odds",
            mk(&|s| {
                s.pass_line = true;
                s.take_odds = true;
            }),
        ),
        (
            "3-pt Molly (pass + 2 come, odds)",
            mk(&|s| {
                s.pass_line = true;
                s.come_max = 2;
                s.take_odds = true;
            }),
        ),
        ("Don't pass", mk(&|s| s.dont_pass = true)),
        (
            "Don't pass + lay odds",
            mk(&|s| {
                s.dont_pass = true;
                s.take_odds = true;
            }),
        ),
        (
            "Don't + 2 don't come, odds",
            mk(&|s| {
                s.dont_pass = true;
                s.dont_come_max = 2;
                s.take_odds = true;
            }),
        ),
        (
            "Place 6 & 8",
            mk(&|s| {
                s.set_place(6, true);
                s.set_place(8, true);
            }),
        ),
        (
            "Inside numbers (5,6,8,9)",
            mk(&|s| {
                for n in [5, 6, 8, 9] {
                    s.set_place(n, true);
                }
            }),
        ),
        (
            "Iron Cross (field + 5,6,8)",
            mk(&|s| {
                s.field = true;
                for n in [5, 6, 8] {
                    s.set_place(n, true);
                }
            }),
        ),
        ("Field only", mk(&|s| s.field = true)),
        ("Hardways (all four)", mk(&|s| s.hardways = [true; 4])),
    ]
}

/// The quit rules the explorer crosses with every strategy and progression.
pub const EXPLORE_QUITS: [Option<f64>; 4] = [None, Some(1.5), Some(2.0), Some(3.0)];

/// The take-profit target a quit multiple implies for a given budget.
pub fn quit_target_cents(budget_cents: i64, mult: f64) -> i64 {
    ((budget_cents as f64 * mult).round() as i64).max(budget_cents + 100)
}

/// One strategy × progression × quit-rule combination's results at one table
/// minimum, with a 95% CI for every objective.
#[derive(Clone, Debug)]
pub struct ExploreRow {
    /// Index into [`explore_strategies`] — with `progression` and `quit_idx`,
    /// enough to reconstruct the combo for the Duel.
    pub strategy_idx: u16,
    pub strategy: &'static str,
    pub progression: Progression,
    /// Index into [`EXPLORE_QUITS`].
    pub quit_idx: u8,
    pub quit: Option<f64>,
    pub p_double: f64,
    pub p_double_ci: f64,
    pub p_ahead: f64,
    pub p_ahead_ci: f64,
    pub p_nobust: f64,
    pub p_nobust_ci: f64,
    pub median_final: i64,
    pub median_final_ci: (i64, i64),
    pub mean_final: f64,
    pub mean_final_ci: f64,
    pub median_rolls: u64,
    /// Sessions this row was computed from.
    pub sessions: u64,
}

#[derive(Clone, Debug)]
pub struct ExploreConfig {
    pub rules: Rules,
    pub mins: Vec<i64>,
    pub budget_cents: i64,
    pub sessions: u64,
    pub horizon_rolls: u64,
    /// Pin every combo to flat betting so bet selections are compared on
    /// their own merits.
    pub flat_only: bool,
    pub base_seed: u64,
}

/// One table minimum's completed explorer rows.
#[derive(Clone, Debug)]
pub struct ExploreMin {
    pub min_index: u32,
    pub min_cents: i64,
    pub rows: Vec<ExploreRow>,
    /// True when cancellation interrupted this minimum: `rows` holds only the
    /// combos completed before the cancel (honest partial provenance), never
    /// a contaminated partial combo.
    pub partial: bool,
}

/// Sweep the strategy grid. Common random numbers: the per-session seed
/// depends on the table minimum and session index but NOT the combo, so every
/// combo plays identical dice and head-to-head rankings are luck-free.
/// On cancellation the completed combos of the current minimum are flushed
/// with `partial: true` rather than discarded.
pub fn run_explore(cfg: &ExploreConfig, tx: SyncSender<ExploreMin>, ctl: &SweepCtl) {
    let strategies = explore_strategies();
    let progressions: &[Progression] = if cfg.flat_only {
        &[Progression::Flat]
    } else {
        &Progression::ALL
    };
    for (mi, &min) in cfg.mins.iter().enumerate() {
        if ctl.stop.load(Ordering::Relaxed) {
            return;
        }
        let mut rows: Vec<ExploreRow> = Vec::new();
        let mut cancelled = false;
        'combos: for (si, (sname, sel_base)) in strategies.iter().enumerate() {
            for &prog in progressions {
                let mut sel = sel_base.clone();
                sel.progression = prog;
                for (qi, &quit) in EXPLORE_QUITS.iter().enumerate() {
                    if ctl.stop.load(Ordering::Relaxed) {
                        cancelled = true;
                        break 'combos;
                    }
                    let quit_cents = quit.map(|m| quit_target_cents(cfg.budget_cents, m));
                    let outs: Vec<(i64, bool, u64)> = (0..cfg.sessions)
                        .into_par_iter()
                        .map(|i| {
                            if ctl.stop.load(Ordering::Relaxed) {
                                return (0, false, 0);
                            }
                            let o = run_horizon_session(
                                &sel,
                                &cfg.rules,
                                min,
                                cfg.budget_cents,
                                quit_cents,
                                cfg.horizon_rolls,
                                session_seed(cfg.base_seed, mi as u32, SeedPhase::Explore, i),
                            );
                            (o.final_cents, o.busted, o.rolls)
                        })
                        .collect();
                    ctl.sessions_done.fetch_add(cfg.sessions, Ordering::Relaxed);
                    ctl.rolls_done.fetch_add(
                        outs.iter().map(|&(_, _, r)| r).sum::<u64>(),
                        Ordering::Relaxed,
                    );
                    // A cancel mid-combo leaves sentinel zeros in `outs`;
                    // discard the contaminated combo, flush the clean ones.
                    if ctl.stop.load(Ordering::Relaxed) {
                        cancelled = true;
                        break 'combos;
                    }
                    rows.push(summarize_combo(
                        si as u16,
                        sname,
                        prog,
                        qi as u8,
                        quit,
                        cfg.budget_cents,
                        &outs,
                    ));
                }
            }
        }
        if rows.is_empty() && cancelled {
            return;
        }
        let _ = tx.send(ExploreMin {
            min_index: mi as u32,
            min_cents: min,
            rows,
            partial: cancelled,
        });
        if cancelled {
            return;
        }
    }
}

fn summarize_combo(
    strategy_idx: u16,
    strategy: &'static str,
    progression: Progression,
    quit_idx: u8,
    quit: Option<f64>,
    budget_cents: i64,
    outs: &[(i64, bool, u64)],
) -> ExploreRow {
    let n = outs.len() as f64;
    let sessions = outs.len() as u64;
    let mut finals: Vec<i64> = outs.iter().map(|&(v, _, _)| v).collect();
    let mut rolls: Vec<u64> = outs.iter().map(|&(_, _, r)| r).collect();
    let busted = outs.iter().filter(|&&(_, b, _)| b).count() as f64;
    finals.sort_unstable();
    rolls.sort_unstable();
    let frac_at_least = |thresh: i64| {
        let idx = finals.partition_point(|&v| v < thresh);
        (finals.len() - idx) as f64 / n
    };
    let mut mean = Welford::default();
    for &v in &finals {
        mean.push(v as f64);
    }
    let p_double = frac_at_least(budget_cents * 2);
    let p_ahead = frac_at_least(budget_cents + 1);
    let p_nobust = 1.0 - busted / n;
    ExploreRow {
        strategy_idx,
        strategy,
        progression,
        quit_idx,
        quit,
        p_double,
        p_double_ci: wald_ci_half(p_double, sessions),
        p_ahead,
        p_ahead_ci: wald_ci_half(p_ahead, sessions),
        p_nobust,
        p_nobust_ci: wald_ci_half(p_nobust, sessions),
        median_final: finals[finals.len() / 2],
        median_final_ci: median_ci_sorted(&finals),
        mean_final: mean.mean(),
        mean_final_ci: mean.ci95_half(),
        median_rolls: rolls[rolls.len() / 2],
        sessions,
    }
}

// ---------------------------------------------------------------------------
// Paired sweep (the Duel)
// ---------------------------------------------------------------------------

/// One side of a Duel: a bet selection (progression included) and a quit
/// multiple.
#[derive(Clone, Debug)]
pub struct PairSide {
    pub sel: BetSelection,
    pub quit_mult: Option<f64>,
}

/// One session's ending bankrolls for both sides, under identical dice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PairedFinal {
    pub a_cents: i64,
    pub b_cents: i64,
}

/// Replay `sessions` explorer sessions for both sides on identical dice
/// (session `i` of both sides uses `session_seed(base, min_index, Explore, i)`,
/// exactly the seed the explorer used). Returns index-aligned pairs. Honors
/// `ctl.stop` by returning what completed (possibly short).
#[allow(clippy::too_many_arguments)]
pub fn run_pair(
    a: &PairSide,
    b: &PairSide,
    rules: &Rules,
    min_cents: i64,
    min_index: u32,
    budget_cents: i64,
    sessions: u64,
    horizon_rolls: u64,
    base_seed: u64,
    ctl: &SweepCtl,
) -> Vec<PairedFinal> {
    let quit_a = a.quit_mult.map(|m| quit_target_cents(budget_cents, m));
    let quit_b = b.quit_mult.map(|m| quit_target_cents(budget_cents, m));
    let pairs: Vec<Option<PairedFinal>> = (0..sessions)
        .into_par_iter()
        .map(|i| {
            if ctl.stop.load(Ordering::Relaxed) {
                return None;
            }
            if i % 256 == 0 {
                ctl.sessions_done.fetch_add(256, Ordering::Relaxed);
            }
            let seed = session_seed(base_seed, min_index, SeedPhase::Explore, i);
            let oa = run_horizon_session(
                &a.sel,
                rules,
                min_cents,
                budget_cents,
                quit_a,
                horizon_rolls,
                seed,
            );
            let ob = run_horizon_session(
                &b.sel,
                rules,
                min_cents,
                budget_cents,
                quit_b,
                horizon_rolls,
                seed,
            );
            ctl.rolls_done
                .fetch_add(oa.rolls + ob.rolls, Ordering::Relaxed);
            Some(PairedFinal {
                a_cents: oa.final_cents,
                b_cents: ob.final_cents,
            })
        })
        .collect();
    // Keep the aligned prefix: a cancel mid-run yields trailing `None`s in
    // arbitrary positions; truncate at the first gap so indices stay aligned
    // with their seeds.
    let mut out = Vec::with_capacity(pairs.len());
    for p in pairs {
        match p {
            Some(v) => out.push(v),
            None => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bets::OddsPolicy;
    use std::sync::mpsc::sync_channel;

    fn cfg() -> SweepConfig {
        SweepConfig {
            program: None,
            sel: BetSelection::default(),
            rules: Rules {
                odds_policy: OddsPolicy::None,
                field_12_triple: false,
                come_odds_work_on_comeout: false,
                prop_bet_cents: 500,
                table_max_mult: 1000,
            },
            mins: vec![500, 1000],
            budget_cents: 30_000,
            quit_target_cents: None,
            sessions: 10_000,
            max_rolls: 5_000,
            horizon_rolls: 400,
            base_seed: 0xC0FFEE,
        }
    }

    /// The M1 gate: the batched sweep must equal a one-shot serial loop
    /// bitwise, record for record, after reassembly by session index.
    /// A swept strategy plays the same sweep the checkbox player does, when
    /// the strategy *is* the checkbox player compiled. This is the
    /// equivalence proof extended from one session to the whole run — every
    /// minimum, every session index, the main view and the drawdown, which
    /// is what Q3's recommended budget is cut from.
    #[test]
    fn a_swept_program_equals_the_swept_selection() {
        use crate::strategy::{compile, from_selection};
        let mut sel = BetSelection {
            pass_line: true,
            take_odds: true,
            ..Default::default()
        };
        sel.come_max = 1;
        sel.set_place(6, true);
        let rules = Rules {
            odds_policy: OddsPolicy::X345,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 1000,
        };
        let program = compile(&from_selection(&sel, &rules)).unwrap();

        let base = SweepConfig {
            sel: sel.clone(),
            program: None,
            rules: rules.clone(),
            mins: vec![500, 1000, 2500],
            budget_cents: 30_000,
            quit_target_cents: None,
            sessions: 600,
            max_rolls: 200_000,
            horizon_rolls: 400,
            base_seed: 0xC0FFEE,
        };
        let with_program = SweepConfig {
            program: Some(std::sync::Arc::new(program)),
            ..base.clone()
        };

        let collect = |cfg: &SweepConfig| {
            let (tx, rx) = sync_channel::<Batch>(64);
            let ctl = SweepCtl::default();
            std::thread::scope(|s| {
                s.spawn(|| run_sweep(cfg, tx, &ctl));
                let mut rows: Vec<(u32, u32, u32, i64, i64, u8)> = Vec::new();
                for b in rx {
                    for r in &b.records {
                        rows.push((
                            b.min_index,
                            r.session,
                            r.rolls,
                            r.final_cents,
                            r.peak_outlay_cents,
                            r.flags,
                        ));
                    }
                }
                // Batches arrive in whatever order rayon finishes them.
                // Every record carries its own session index, so sorting by
                // (minimum, session) restores the canonical order the
                // collector reassembles and makes this an exact per-session
                // comparison rather than a comparison of sorted sets.
                rows.sort_unstable();
                rows
            })
        };
        let a = collect(&base);
        let b = collect(&with_program);
        assert_eq!(a.len(), b.len(), "different number of sessions");
        assert_eq!(a, b, "the strategy and the selection swept differently");
    }

    #[test]
    fn batched_sweep_equals_one_shot_bitwise() {
        let cfg = cfg();
        let (tx, rx) = sync_channel::<Batch>(256);
        let ctl = SweepCtl::default();
        run_sweep(&cfg, tx, &ctl);

        let mut got: Vec<Vec<Option<SessionRecord>>> =
            vec![vec![None; cfg.sessions as usize]; cfg.mins.len()];
        let mut handle_sums = vec![0i64; cfg.mins.len()];
        for batch in rx.iter() {
            assert!(!batch.partial);
            handle_sums[batch.min_index as usize] += batch.handle_cents_sum;
            for rec in batch.records {
                let slot = &mut got[batch.min_index as usize][rec.session as usize];
                assert!(
                    slot.is_none(),
                    "duplicate record for session {}",
                    rec.session
                );
                *slot = Some(rec);
            }
        }

        for (mi, &min) in cfg.mins.iter().enumerate() {
            let mut expected_handle = 0i64;
            for i in 0..cfg.sessions {
                let o = run_session(
                    &cfg.sel,
                    &cfg.rules,
                    min,
                    cfg.budget_cents,
                    cfg.quit_target_cents,
                    cfg.max_rolls,
                    cfg.horizon_rolls,
                    session_seed(cfg.base_seed, mi as u32, SeedPhase::Main, i),
                );
                let outlay = run_drawdown_session(
                    &cfg.sel,
                    &cfg.rules,
                    min,
                    cfg.horizon_rolls,
                    session_seed(cfg.base_seed, mi as u32, SeedPhase::Drawdown, i),
                );
                let rec = got[mi][i as usize].expect("missing record");
                assert_eq!(rec.rolls as u64, o.ruin.rolls);
                assert_eq!(rec.final_cents, o.horizon.final_cents);
                assert_eq!(rec.peak_outlay_cents, outlay);
                assert_eq!(rec.flags & record_flags::CENSORED != 0, o.ruin.censored);
                assert_eq!(rec.flags & record_flags::BUSTED != 0, o.horizon.busted);
                expected_handle += o.horizon_handle_cents;
            }
            assert_eq!(handle_sums[mi], expected_handle, "handle sum, min {min}");
        }
        assert_eq!(
            ctl.sessions_done.load(Ordering::Relaxed),
            cfg.sessions * cfg.mins.len() as u64
        );
        assert!(ctl.rolls_done.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn cancelled_sweep_stops_and_flags_nothing_dirty() {
        let cfg = cfg();
        let (tx, rx) = sync_channel::<Batch>(256);
        let ctl = SweepCtl::default();
        ctl.stop.store(true, Ordering::Relaxed);
        run_sweep(&cfg, tx, &ctl);
        assert_eq!(rx.iter().count(), 0, "pre-cancelled sweep sent batches");
    }

    #[test]
    fn explorer_rows_carry_cis_and_flush_cleanly() {
        let cfg = ExploreConfig {
            rules: Rules {
                odds_policy: OddsPolicy::X345,
                field_12_triple: false,
                come_odds_work_on_comeout: false,
                prop_bet_cents: 500,
                table_max_mult: 500,
            },
            mins: vec![1000],
            budget_cents: 30_000,
            sessions: 500,
            horizon_rolls: 200,
            flat_only: true,
            base_seed: 0xFEED,
        };
        let (tx, rx) = sync_channel::<ExploreMin>(16);
        let ctl = SweepCtl::default();
        run_explore(&cfg, tx, &ctl);
        let mins: Vec<ExploreMin> = rx.iter().collect();
        assert_eq!(mins.len(), 1);
        let m = &mins[0];
        assert!(!m.partial);
        // 11 strategies × 1 progression (flat only) × 4 quits.
        assert_eq!(m.rows.len(), 44);
        for row in &m.rows {
            assert_eq!(row.sessions, 500);
            assert!(row.p_double_ci > 0.0 || row.p_double == 0.0 || row.p_double == 1.0);
            assert!(row.median_final_ci.0 <= row.median_final);
            assert!(row.median_final <= row.median_final_ci.1);
            assert!(row.mean_final_ci > 0.0);
            // Reconstructability for the Duel.
            assert_eq!(
                explore_strategies()[row.strategy_idx as usize].0,
                row.strategy
            );
            assert_eq!(EXPLORE_QUITS[row.quit_idx as usize], row.quit);
        }
    }

    #[test]
    fn pair_replays_identical_dice() {
        // A strategy dueled against itself must produce identical sides.
        let rules = Rules {
            odds_policy: OddsPolicy::X345,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 500,
        };
        let side = PairSide {
            sel: BetSelection::default(),
            quit_mult: Some(2.0),
        };
        let ctl = SweepCtl::default();
        let pairs = run_pair(
            &side, &side, &rules, 1000, 0, 30_000, 2_000, 400, 0xAB, &ctl,
        );
        assert_eq!(pairs.len(), 2_000);
        assert!(pairs.iter().all(|p| p.a_cents == p.b_cents));

        // And the sides must match a direct explorer-seeded run.
        for (i, p) in pairs.iter().take(50).enumerate() {
            let o = run_horizon_session(
                &side.sel,
                &rules,
                1000,
                30_000,
                Some(quit_target_cents(30_000, 2.0)),
                400,
                session_seed(0xAB, 0, SeedPhase::Explore, i as u64),
            );
            assert_eq!(p.a_cents, o.final_cents);
        }
    }

    /// Release-mode throughput gate: the batched sweep must keep ≥97% of the
    /// throughput of a plain parallel collect. Run manually:
    ///   cargo test --release -p craps-engine -- --ignored throughput_gate --nocapture
    #[test]
    #[ignore]
    fn throughput_gate_batched_vs_collect() {
        let mut cfg = cfg();
        cfg.mins = vec![1000];
        cfg.sessions = 400_000;
        let start = std::time::Instant::now();
        let _baseline: Vec<i64> = (0..cfg.sessions)
            .into_par_iter()
            .map(|i| {
                let o = run_session(
                    &cfg.sel,
                    &cfg.rules,
                    1000,
                    cfg.budget_cents,
                    None,
                    cfg.max_rolls,
                    cfg.horizon_rolls,
                    session_seed(cfg.base_seed, 0, SeedPhase::Main, i),
                );
                let d = run_drawdown_session(
                    &cfg.sel,
                    &cfg.rules,
                    1000,
                    cfg.horizon_rolls,
                    session_seed(cfg.base_seed, 0, SeedPhase::Drawdown, i),
                );
                o.horizon.final_cents + d
            })
            .collect();
        let collect_time = start.elapsed().as_secs_f64();

        let (tx, rx) = sync_channel::<Batch>(256);
        let ctl = SweepCtl::default();
        let drain = std::thread::spawn(move || rx.iter().map(|b| b.records.len()).sum::<usize>());
        let start = std::time::Instant::now();
        run_sweep(&cfg, tx, &ctl);
        let sweep_time = start.elapsed().as_secs_f64();
        let n = drain.join().unwrap();
        assert_eq!(n as u64, cfg.sessions);
        println!(
            "collect: {collect_time:.3}s  batched: {sweep_time:.3}s  ratio {:.3}",
            collect_time / sweep_time
        );
        assert!(
            sweep_time <= collect_time / 0.97,
            "batched sweep lost more than 3%: {sweep_time:.3}s vs {collect_time:.3}s"
        );
    }
}

#[cfg(test)]
mod investigation {
    //! Investigation harnesses carried over from the pre-workspace app:
    //! near-ground-truth explorer leaderboards and the progression effect on
    //! doubling. Run with `--ignored --nocapture`.
    use super::*;
    use crate::bets::OddsPolicy;
    use crate::session::run_horizon_session;

    /// Does the pressing strategy really change P(double)? Same bets, same
    /// quit rule, same dice — only the progression differs. Run with:
    ///   cargo test --release -p craps-engine -- --ignored progression_effect --nocapture
    #[test]
    #[ignore]
    fn progression_effect_on_doubling() {
        let rules = Rules {
            odds_policy: OddsPolicy::X345,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 500,
        };
        let budget = 100_000i64;
        let sessions = 400_000u64;
        println!(
            "3-pt Molly, $5 min, $1000 budget, quit 2x, 400 rolls, {}k sessions:",
            sessions / 1000
        );
        for prog in [
            Progression::Flat,
            Progression::DAlembert,
            Progression::ReverseDAlembert,
            Progression::Paroli3,
            Progression::Martingale,
            Progression::OscarsGrind,
        ] {
            let mut sel = BetSelection {
                pass_line: true,
                ..Default::default()
            };
            sel.come_max = 2;
            sel.take_odds = true;
            sel.progression = prog;
            let (doubles, busts) = (0..sessions)
                .into_par_iter()
                .map(|i| {
                    let o = run_horizon_session(
                        &sel,
                        &rules,
                        500,
                        budget,
                        Some(budget * 2),
                        400,
                        0xFEED ^ i,
                    );
                    ((o.final_cents >= budget * 2) as u64, o.busted as u64)
                })
                .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
            let p = doubles as f64 / sessions as f64;
            let se = (p * (1.0 - p) / sessions as f64).sqrt();
            println!(
                "  {:22} P(double) = {:5.2}% +-{:.2}%   bust = {:5.2}%",
                prog.label(),
                p * 100.0,
                se * 100.0,
                busts as f64 / sessions as f64 * 100.0
            );
        }
    }

    /// Near-ground-truth explorer leaderboard for a $1,000 budget at low
    /// table minimums, plus rank-churn analysis at the default explorer
    /// precision. Run with:
    ///   cargo test --release -p craps-engine -- --ignored explorer_ --nocapture
    #[test]
    #[ignore]
    fn explorer_ground_truth_1000_budget() {
        let rules = Rules {
            odds_policy: OddsPolicy::X345,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 500,
        };
        let budget = 100_000i64;
        let horizon = 400u64;
        let strategies = explore_strategies();

        let leaderboard = |min: i64, sessions: u64, base_seed: u64| -> Vec<(String, f64)> {
            let mut rows = Vec::new();
            for (sname, sel_base) in &strategies {
                for prog in Progression::ALL {
                    let mut sel = sel_base.clone();
                    sel.progression = prog;
                    for quit in EXPLORE_QUITS {
                        let quit_cents = quit.map(|m| quit_target_cents(budget, m));
                        let doubles = (0..sessions)
                            .into_par_iter()
                            .filter(|&i| {
                                run_horizon_session(
                                    &sel,
                                    &rules,
                                    min,
                                    budget,
                                    quit_cents,
                                    horizon,
                                    base_seed ^ ((min as u64) << 40) ^ i,
                                )
                                .final_cents
                                    >= budget * 2
                            })
                            .count();
                        let q = match quit {
                            Some(m) => format!("quit {m:.1}x"),
                            None => "no quit".to_owned(),
                        };
                        rows.push((
                            format!("{sname} | {} | {q}", prog.label()),
                            doubles as f64 / sessions as f64,
                        ));
                    }
                }
            }
            rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            rows
        };

        for &min in &[500i64, 1000, 1500] {
            let sessions = 50_000u64;
            let rows = leaderboard(min, sessions, 0xC0FFEE);
            println!(
                "=== ${} min, {}k sessions/combo (ground truth) ===",
                min / 100,
                sessions / 1000
            );
            for (k, (name, p)) in rows.iter().take(8).enumerate() {
                let se = (p * (1.0 - p) / sessions as f64).sqrt();
                println!(
                    "  {}. {:5.2}% +-{:.2}%  {}",
                    k + 1,
                    p * 100.0,
                    se * 100.0,
                    name
                );
            }
        }

        println!();
        println!("=== $5 min, default 5k sessions, five independent runs (rank churn) ===");
        for run in 0..5u64 {
            let rows = leaderboard(500, 5_000, 0x1111_2222 ^ (run << 48));
            let top: Vec<String> = rows
                .iter()
                .take(3)
                .map(|(n, p)| format!("{n} ({:.1}%)", p * 100.0))
                .collect();
            println!("  run {}:", run + 1);
            for t in top {
                println!("     {t}");
            }
        }
    }
}
