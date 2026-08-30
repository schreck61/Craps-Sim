// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Roll-by-roll session tracing (E4).
//!
//! [`Session`](crate::game) is generic over a [`RollObserver`]; the default
//! [`Noop`] observer has empty inline hooks, so untraced simulation
//! monomorphizes to exactly the code it was before tracing existed — the
//! equivalence suite pins that. [`trace_session`] replays one session with a
//! recording observer and returns its complete ledger; because the engine is
//! deterministic per seed, any session from any run can be reproduced
//! exactly, which is what the replay theater and the Duel's trajectory view
//! rely on.

use crate::bets::{BetSelection, Rules};
use crate::session::{run_session_impl, SessionOutcomes};
use crate::strategy::RejectReason;

/// Which bet an event concerns. Numbers ride along where a bet lives on a
/// box number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BetKind {
    Pass,
    PassOdds,
    DontPass,
    DontPassLay,
    /// A come flat still in the come box.
    Come,
    /// A come flat established on a number.
    ComePoint(u8),
    ComeOdds(u8),
    /// A don't come flat still in the box.
    DontCome,
    DontComePoint(u8),
    DontComeLay(u8),
    Place(u8),
    Hardway(u8),
    Field,
    AnySeven,
    AnyCraps,
}

impl BetKind {
    /// How this bet is spoken. One place, so the Replay ledger, the Bench,
    /// and the editor all call the same bet the same thing.
    pub fn label(&self) -> String {
        match self {
            BetKind::Pass => "pass".into(),
            BetKind::PassOdds => "odds on pass".into(),
            BetKind::DontPass => "dont pass".into(),
            BetKind::DontPassLay => "odds on dont pass".into(),
            BetKind::Come => "come".into(),
            BetKind::ComePoint(n) => format!("come {n}"),
            BetKind::ComeOdds(n) => format!("odds on come {n}"),
            BetKind::DontCome => "dont come".into(),
            BetKind::DontComePoint(n) => format!("dont come {n}"),
            BetKind::DontComeLay(n) => format!("odds on dont come {n}"),
            BetKind::Place(n) => format!("place {n}"),
            BetKind::Hardway(n) => format!("hard {n}"),
            BetKind::Field => "field".into(),
            BetKind::AnySeven => "any seven".into(),
            BetKind::AnyCraps => "any craps".into(),
        }
    }
}

/// What a strategy was trying to do when the table said no.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attempted {
    Bet,
    Press,
    Regress,
    Down,
    Working,
}

impl Attempted {
    /// The verb, in the words the ledger shows.
    pub fn label(self) -> &'static str {
        match self {
            Attempted::Bet => "betting",
            Attempted::Press => "pressing",
            Attempted::Regress => "regressing",
            Attempted::Down => "taking down",
            Attempted::Working => "turning off or on",
        }
    }
}

/// What happened to a bet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BetEventKind {
    /// Money left the rail onto the layout.
    Placed,
    /// The bet won. `paid_cents` is the winnings; `stake_returned` says
    /// whether the stake came back to the rail with them.
    ///
    /// It has to be said rather than implied by the bet's rules, because a
    /// ledger that cannot tell a pass-line win (stake and winnings both
    /// return) from a place-bet win (the winner stays up and only the
    /// winnings come back) cannot account for the rail — and the Bench
    /// promises exactly that accounting, to the cent, at every step.
    Won {
        paid_cents: i64,
        stake_returned: bool,
    },
    /// The bet lost its stake.
    Lost,
    /// A push: the bet stays where it is (bar-12 on the don't side).
    Pushed,
    /// The stake came back without resolving (odds off on a come-out).
    Returned,
    /// A press-down: part of a working bet taken back to the rail.
    TakenDown,
    /// A come/don't come flat traveled to a box number.
    Traveled { to: u8 },
    /// The table refused the strategy's request. `stake_cents` is 0 — no
    /// money moved. A strategy that quietly does nothing is the worst
    /// outcome the intent surface could allow, so every refusal is an
    /// event (STRATEGY_DSL.md Principle 4).
    /// `what` and the event's `stake_cents` say which action was refused and
    /// how much it asked for. Without them a ledger could only report that
    /// *something* about a bet was refused: a zero-stake bet from an
    /// uninitialized counter read as "bankroll won't cover it" beside a full
    /// bankroll, and a refusal on a place bet could have been the bet, a
    /// press, a take-down or a working toggle from any of six rules.
    Rejected {
        reason: RejectReason,
        what: Attempted,
    },
    /// The requested stake exceeded the table maximum and was truncated to
    /// it; `stake_cents` is the amount actually bet. This is how a real
    /// table stops a Martingale, and it is shown rather than inferred from
    /// a flat spot in a curve.
    ClippedToMax,
}

/// One thing that happened to one bet, to the cent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BetEvent {
    pub bet: BetKind,
    pub kind: BetEventKind,
    pub stake_cents: i64,
}

/// Observer hooks called from inside the session hot path. Every hook has an
/// empty inline default so the [`Noop`] observer compiles to nothing.
pub trait RollObserver {
    /// Set true to receive [`RollObserver::roll_end`]; when false the runner
    /// skips computing per-roll wealth entirely.
    const WANTS_ROLL_END: bool = false;

    /// A bet was placed, resolved, pushed, returned, or moved.
    #[inline(always)]
    fn event(&mut self, ev: BetEvent) {
        let _ = ev;
    }

    /// Set true to be told which rules fired and what each one asked for.
    /// Only the Bench wants this; every other observer leaves it false and
    /// the attribution never runs.
    const WANTS_RULES: bool = false;

    /// A compiled rule's trigger matched and its guard held. `rule` indexes
    /// the strategy's rules in the order they were written.
    #[inline(always)]
    fn rule_fired(&mut self, rule: u16) {
        let _ = rule;
    }

    /// The bet events that follow came from this rule, or from the table
    /// itself when `None`. Emitted before each proposal reaches the table
    /// and once more when the decision is over, so the ledger can say which
    /// line of a strategy moved which money — and can say honestly that a
    /// bet winning was nobody's doing.
    #[inline(always)]
    fn acting_for(&mut self, rule: Option<u16>) {
        let _ = rule;
    }

    /// A roll finished resolving. `roll` counts from 1; `wealth_after` is
    /// cash plus face value of live bets.
    #[inline(always)]
    fn roll_end(
        &mut self,
        roll: u64,
        dice: (u8, u8),
        point_after: Option<u8>,
        cash_after: i64,
        wealth_after: i64,
    ) {
        let _ = (roll, dice, point_after, cash_after, wealth_after);
    }
}

/// The default observer: observes nothing, costs nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Noop;

impl RollObserver for Noop {}

/// Everything one roll did: the dice, the resulting state, and every bet
/// event it caused, in resolution order.
#[derive(Clone, Debug)]
pub struct RollEvent {
    /// 1-based roll number.
    pub roll: u32,
    pub dice: (u8, u8),
    pub point_after: Option<u8>,
    pub cash_after: i64,
    /// Cash plus face value of live bets.
    pub wealth_after: i64,
    pub events: Vec<BetEvent>,
}

/// A complete, exactly-reproducible session ledger.
#[derive(Clone, Debug)]
pub struct SessionTrace {
    pub seed: u64,
    pub events: Vec<RollEvent>,
    pub outcome: SessionOutcomes,
}

/// Records the full per-roll ledger.
#[derive(Default)]
struct Recorder {
    rolls: Vec<RollEvent>,
    pending: Vec<BetEvent>,
}

impl RollObserver for Recorder {
    const WANTS_ROLL_END: bool = true;

    fn event(&mut self, ev: BetEvent) {
        self.pending.push(ev);
    }

    fn roll_end(
        &mut self,
        roll: u64,
        dice: (u8, u8),
        point_after: Option<u8>,
        cash_after: i64,
        wealth_after: i64,
    ) {
        self.rolls.push(RollEvent {
            roll: roll as u32,
            dice,
            point_after,
            cash_after,
            wealth_after,
            events: std::mem::take(&mut self.pending),
        });
    }
}

/// Records only per-roll wealth — one `i64` per roll — for the wealth fan's
/// trajectory envelope.
#[derive(Default)]
pub struct WealthOnlyObserver {
    /// `wealth[k]` is total wealth after roll `k + 1`.
    pub wealth: Vec<i64>,
}

impl RollObserver for WealthOnlyObserver {
    const WANTS_ROLL_END: bool = true;

    fn roll_end(
        &mut self,
        _roll: u64,
        _dice: (u8, u8),
        _point_after: Option<u8>,
        _cash_after: i64,
        wealth_after: i64,
    ) {
        self.wealth.push(wealth_after);
    }
}

/// Replay one session exactly, recording its complete roll-by-roll ledger.
/// Same semantics as [`run_session`](crate::run_session) with the same
/// arguments — the outcome is bit-identical to the untraced run at the same
/// seed (the test suite pins this).
#[allow(clippy::too_many_arguments)]
pub fn trace_session(
    sel: &BetSelection,
    rules: &Rules,
    table_min_cents: i64,
    budget_cents: i64,
    quit_target_cents: Option<i64>,
    max_rolls: u64,
    horizon_rolls: u64,
    seed: u64,
) -> SessionTrace {
    let (outcome, rec) = run_session_impl(
        sel,
        rules,
        table_min_cents,
        budget_cents,
        quit_target_cents,
        max_rolls,
        horizon_rolls,
        seed,
        Recorder::default(),
    );
    SessionTrace {
        seed,
        events: rec.rolls,
        outcome,
    }
}

/// Replay one session recording only its wealth trajectory (cheap: one `i64`
/// per roll).
#[allow(clippy::too_many_arguments)]
pub fn trace_wealth(
    sel: &BetSelection,
    rules: &Rules,
    table_min_cents: i64,
    budget_cents: i64,
    quit_target_cents: Option<i64>,
    max_rolls: u64,
    horizon_rolls: u64,
    seed: u64,
) -> (SessionOutcomes, Vec<i64>) {
    let (outcome, obs) = run_session_impl(
        sel,
        rules,
        table_min_cents,
        budget_cents,
        quit_target_cents,
        max_rolls,
        horizon_rolls,
        seed,
        WealthOnlyObserver::default(),
    );
    (outcome, obs.wealth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bets::{OddsPolicy, Progression};
    use crate::session::run_session;

    fn rules() -> Rules {
        Rules {
            odds_policy: OddsPolicy::X345,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 500,
            place_the_point: false,
        }
    }

    fn molly() -> BetSelection {
        let mut sel = BetSelection {
            pass_line: true,
            ..Default::default()
        };
        sel.come_max = 2;
        sel.take_odds = true;
        sel.field = true;
        sel.progression = Progression::DAlembert;
        sel
    }

    /// The trace must be an exact replay: outcome fields bit-identical to the
    /// untraced run, and the last roll's wealth equal to the final bankroll
    /// when the session ended on a roll.
    #[test]
    fn trace_equals_untraced_outcome_across_seeds() {
        let sel = molly();
        let r = rules();
        for seed in 0..1000u64 {
            let plain = run_session(&sel, &r, 1000, 30_000, Some(60_000), 2_000, 400, seed);
            let traced = trace_session(&sel, &r, 1000, 30_000, Some(60_000), 2_000, 400, seed);
            assert_eq!(traced.outcome.ruin, plain.ruin, "seed {seed}");
            assert_eq!(traced.outcome.horizon, plain.horizon, "seed {seed}");
            assert_eq!(
                traced.outcome.peak_outlay_cents, plain.peak_outlay_cents,
                "seed {seed}"
            );
            assert_eq!(
                traced.events.len() as u64,
                plain.ruin.rolls.max(plain.horizon.rolls),
                "seed {seed}: one RollEvent per roll played"
            );
            // Roll numbers are 1-based and contiguous.
            for (k, ev) in traced.events.iter().enumerate() {
                assert_eq!(ev.roll as usize, k + 1, "seed {seed}");
                assert!((1..=6).contains(&ev.dice.0) && (1..=6).contains(&ev.dice.1));
            }
        }
    }

    #[test]
    fn wealth_trace_matches_full_trace() {
        let sel = molly();
        let r = rules();
        for seed in 0..100u64 {
            let full = trace_session(&sel, &r, 1000, 30_000, None, 400, 400, seed);
            let (outcome, wealth) = trace_wealth(&sel, &r, 1000, 30_000, None, 400, 400, seed);
            assert_eq!(outcome.horizon, full.outcome.horizon);
            assert_eq!(wealth.len(), full.events.len());
            for (w, ev) in wealth.iter().zip(&full.events) {
                assert_eq!(*w, ev.wealth_after, "seed {seed} roll {}", ev.roll);
            }
        }
    }

    /// The ledger's cash bookkeeping must reconcile: every roll's cash equals
    /// the previous roll's cash plus placements taken and winnings paid.
    #[test]
    fn trace_events_reconcile_to_the_cent() {
        let sel = molly();
        let r = rules();
        for seed in 0..100u64 {
            let t = trace_session(&sel, &r, 1000, 30_000, None, 400, 400, seed);
            for ev in &t.events {
                for be in &ev.events {
                    assert!(be.stake_cents >= 0, "seed {seed}: negative stake");
                    if let BetEventKind::Won { paid_cents, .. } = be.kind {
                        assert!(paid_cents > 0, "seed {seed}: non-positive win");
                    }
                }
            }
            // The final wealth in the ledger equals the outcome (session ran
            // to the horizon un-busted here, so wealth is well-defined).
            if !t.outcome.horizon.busted && !t.outcome.horizon.hit_target {
                assert_eq!(
                    t.events.last().unwrap().wealth_after,
                    t.outcome.horizon.final_cents,
                    "seed {seed}"
                );
            }
        }
    }
}
