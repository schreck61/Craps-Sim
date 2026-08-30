// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The Bench: one session, stepped, with every decision attributed.
//!
//! [`STRATEGY_DSL.md`](../../../../docs/STRATEGY_DSL.md) §8 calls this the
//! feature that decides whether the language is usable, and not optional.
//! An editor without a debugger is where a language like this dies: complex
//! rule sets fail silently, Principle 4 cannot be enforced by reading, and
//! the author is left to infer from a distribution that something never
//! fired.
//!
//! So the Bench answers, for one seed, roll by roll: which rules fired, in
//! order; what each one asked the table for; and what the table did about
//! it — applied, clipped, or refused, with the reason in words. Plus a
//! fire count per rule for the whole run, which is how a rule that never
//! fires announces itself instead of waiting to be found.
//!
//! It costs nothing when it is not attached. Every hook it uses is behind
//! `RollObserver::WANTS_RULES`, and the `Noop` discipline is the same one
//! the trace observer has always followed.

use crate::bets::{BetSelection, Rules};
use crate::session::{run_with_player, SessionOutcomes};
use crate::strategy::player::Compiled;
use crate::strategy::Program;
use crate::trace::{BetEvent, RollEvent, RollObserver, SessionTrace};

/// One thing the table did, and the rule that asked for it.
///
/// `rule` is `None` for events the strategy did not cause: a bet winning, a
/// come flat traveling, a seven-out taking the layout down. Those are the
/// table acting on its own, and attributing them to whichever rule fired
/// last would be a lie the ledger tells quietly.
#[derive(Clone, Copy, Debug)]
pub struct BenchEvent {
    pub rule: Option<u16>,
    pub event: BetEvent,
}

/// One roll: what the dice did, what the strategy did about it, and what
/// everything was worth afterwards.
#[derive(Clone, Debug)]
pub struct BenchRoll {
    /// 1-based roll number.
    pub roll: u32,
    pub dice: (u8, u8),
    pub point_after: Option<u8>,
    pub cash_after: i64,
    pub wealth_after: i64,
    /// Rules whose trigger matched and whose guard held, in the order they
    /// were written — the highlight the editor draws.
    pub fired: Vec<u16>,
    pub events: Vec<BenchEvent>,
}

/// A complete, reproducible session ledger.
#[derive(Clone, Debug)]
pub struct BenchTrace {
    pub seed: u64,
    pub rolls: Vec<BenchRoll>,
    /// How many times each rule fired across the whole session, indexed by
    /// rule. A zero here is the dead-rule diagnostic §9 promises, arrived at
    /// by running rather than by proving.
    pub fire_counts: Vec<u32>,
    pub outcome: SessionOutcomes,
    /// The horizon this night was run against, kept so the trace can tell a
    /// night that ran out from a night that walked.
    pub horizon_rolls: u64,
}

impl BenchTrace {
    /// Rules that never fired. The first question to ask of a strategy that
    /// is not doing what its author expected.
    pub fn never_fired(&self) -> Vec<u16> {
        self.fire_counts
            .iter()
            .enumerate()
            .filter(|(_, n)| **n == 0)
            .map(|(i, _)| i as u16)
            .collect()
    }

    /// Every refusal in the session, with the roll it happened on. A
    /// strategy asking for something the table will not give it is the
    /// other way a rule set silently does nothing.
    pub fn refusals(&self) -> Vec<(u32, BenchEvent)> {
        self.rolls
            .iter()
            .flat_map(|r| {
                r.events
                    .iter()
                    .filter(|e| matches!(e.event.kind, crate::trace::BetEventKind::Rejected { .. }))
                    .map(move |e| (r.roll, *e))
            })
            .collect()
    }
}

impl BenchTrace {
    /// The same session as a plain [`SessionTrace`], so everything already
    /// built to read one — the dice strip, the trajectory, the population
    /// envelope — reads a strategy's night without knowing it is one.
    ///
    /// The rule attribution is what is dropped; it lives on in the
    /// `BenchTrace` beside this, which is what the ledger reads.
    pub fn as_session_trace(&self) -> SessionTrace {
        SessionTrace {
            seed: self.seed,
            events: self
                .rolls
                .iter()
                .map(|r| RollEvent {
                    roll: r.roll,
                    dice: r.dice,
                    point_after: r.point_after,
                    cash_after: r.cash_after,
                    wealth_after: r.wealth_after,
                    events: r.events.iter().map(|e| e.event).collect(),
                })
                .collect(),
            outcome: self.outcome,
        }
    }

    /// Whether the session stopped short of the horizon it was given without
    /// busting or reaching its target — which leaves one way it can have
    /// happened: a rule said `leave`.
    fn left_early(&self) -> bool {
        self.outcome.horizon.rolls < self.horizon_rolls
    }

    /// How the night ended, in words. The Bench computed this from the
    /// first version and then discarded it; a session you can step through
    /// without being told whether it busted is a story with no last page.
    pub fn ending(&self) -> &'static str {
        let h = &self.outcome.horizon;
        if h.busted {
            "the bankroll could no longer cover a bet"
        } else if h.hit_target {
            "the quit-while-ahead target was reached"
        } else if self.left_early() {
            // A `leave` rule ended the night on its own terms. Calling that
            // "ran its full length" contradicted the roll count sitting
            // beside it and had readers believing their stop-loss was dead.
            "the strategy left the table on its own terms"
        } else {
            // The roll cap and the session length coincide whenever a night
            // is replayed over its own horizon, so naming the cap here would
            // be wrong more often than right. From the player's side both
            // mean the same thing: the night was over.
            "the night ran its full length"
        }
    }
}

/// Records the ledger. Only ever attached by [`bench_session`].
#[derive(Default)]
struct Bench {
    rolls: Vec<BenchRoll>,
    fire_counts: Vec<u32>,
    pending_fired: Vec<u16>,
    pending_events: Vec<BenchEvent>,
    /// The rule whose proposals are currently reaching the table.
    acting: Option<u16>,
}

impl RollObserver for Bench {
    const WANTS_ROLL_END: bool = true;
    const WANTS_RULES: bool = true;

    fn rule_fired(&mut self, rule: u16) {
        let i = rule as usize;
        if self.fire_counts.len() <= i {
            self.fire_counts.resize(i + 1, 0);
        }
        self.fire_counts[i] += 1;
        self.pending_fired.push(rule);
    }

    fn acting_for(&mut self, rule: Option<u16>) {
        self.acting = rule;
    }

    fn event(&mut self, event: BetEvent) {
        self.pending_events.push(BenchEvent {
            rule: self.acting,
            event,
        });
    }

    fn roll_end(
        &mut self,
        roll: u64,
        dice: (u8, u8),
        point_after: Option<u8>,
        cash_after: i64,
        wealth_after: i64,
    ) {
        self.rolls.push(BenchRoll {
            roll: roll as u32,
            dice,
            point_after,
            cash_after,
            wealth_after,
            fired: std::mem::take(&mut self.pending_fired),
            events: std::mem::take(&mut self.pending_events),
        });
    }
}

/// Run one session on one seed and return everything that happened.
///
/// Because the engine is deterministic per seed, any session from any run
/// can be reproduced here exactly — which is what makes a strategy's
/// behavior something to look at rather than something to infer.
#[allow(clippy::too_many_arguments)]
pub fn bench_session(
    program: &Program,
    rules: &Rules,
    table_min_cents: i64,
    budget_cents: i64,
    quit_target_cents: Option<i64>,
    max_rolls: u64,
    horizon_rolls: u64,
    seed: u64,
) -> BenchTrace {
    let idle = BetSelection {
        pass_line: false,
        ..Default::default()
    };
    let cheapest = program.cheapest_stake(rules, table_min_cents);
    let (outcome, mut bench) = run_with_player(
        &Compiled::new(program, cheapest),
        &idle,
        rules,
        table_min_cents,
        budget_cents,
        quit_target_cents,
        max_rolls,
        horizon_rolls,
        seed,
        Bench::default(),
    );
    // A rule that never fired has no entry yet; the count vector must cover
    // every rule so that "never fired" is answerable rather than absent.
    bench.fire_counts.resize(program.rule_count(), 0);
    BenchTrace {
        seed,
        rolls: bench.rolls,
        fire_counts: bench.fire_counts,
        outcome,
        horizon_rolls,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bets::OddsPolicy;
    use crate::session::run_program_session;
    use crate::strategy::{compile, parse, BetRef};
    use crate::trace::BetEventKind;

    fn rules() -> Rules {
        Rules {
            odds_policy: OddsPolicy::X345,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 1000,
        }
    }

    fn program(src: &str) -> Program {
        compile(&parse(src).unwrap_or_else(|e| panic!("{}", e.message()))).unwrap()
    }

    /// A press the table accepted is a press the next win pays.
    ///
    /// It was not. A progression re-prices a bet where it resolves, and a
    /// rule pressing at the decision point left the stream still thinking
    /// the bet was worth its base — so the winner was torn back down to base
    /// on the very hit it was riding, and `press to stake(…) * 2` recomputed
    /// six-times-two forever. Every press-and-ride ladder in craps was
    /// unreachable, and nothing said so.
    #[test]
    fn a_press_survives_the_win_it_rides() {
        let r = rules();
        let p = program(
            "strategy \"ladder\" language 1\n\
             on roll when point != 0 and point != 6:\n    bet place 6 base\n\
             on win of place 6:\n    press place 6 to stake(place 6) * 2\n\
             on roll when stake(place 6) >= $24:\n    leave\n",
        );
        let climbed: u32 = (0..40u64)
            .map(|seed| bench_session(&p, &r, 500, 100_000, None, 300, 300, seed).fire_counts[2])
            .sum();
        assert!(
            climbed > 0,
            "the 6 never got past twice its base in forty sessions"
        );
    }

    /// A press that would lower the bet is refused, and says so.
    ///
    /// This was the one refusal in the language that emitted nothing: the
    /// rule fired, the Bench showed it firing, the layout did not move, and
    /// no reason was given anywhere — which is the exact silence Principle 4
    /// exists to forbid.
    #[test]
    fn a_press_in_the_wrong_direction_is_refused_out_loud() {
        let r = rules();
        let p = program(
            "strategy \"backwards\" language 1\n\
             on roll when point != 0 and point != 6:\n    bet place 6 base\n\
             on win of place 6:\n    press place 6 to $1\n",
        );
        let mut refusals = 0;
        let mut fired = 0;
        for seed in 0..40u64 {
            let t = bench_session(&p, &r, 500, 100_000, None, 300, 300, seed);
            fired += t.fire_counts[1];
            refusals += t
                .refusals()
                .iter()
                .filter(|(_, e)| {
                    matches!(
                        e.event.kind,
                        BetEventKind::Rejected {
                            reason: crate::strategy::RejectReason::WrongDirection
                        }
                    )
                })
                .count();
        }
        assert!(fired > 0, "the press rule never fired at all");
        assert_eq!(
            refusals as u32, fired,
            "every firing of a backwards press should have left a reason behind"
        );
    }

    /// `on session-start` is a trigger the language spells, the compiler
    /// compiles, and the editor offers. It fired never: no code path set the
    /// bit, so every rule written on it compiled clean and did nothing.
    #[test]
    fn session_start_fires_once_at_the_start_of_a_session() {
        let r = rules();
        let p = program(
            "strategy \"s\" language 1\n\
             var buyin = 0\n\
             on session-start:\n    set buyin = cash\n\
             on come-out:\n    bet pass base\n",
        );
        for seed in 0..20u64 {
            let t = bench_session(&p, &r, 500, 100_000, None, 300, 300, seed);
            assert_eq!(t.fire_counts[0], 1, "seed {seed}: once, and exactly once");
        }
    }

    /// Watching costs nothing that could change what is watched: the Bench
    /// session must reach the same end as the same seed run untraced. The
    /// `Noop` discipline is only a claim until something checks it.
    #[test]
    fn the_bench_does_not_change_the_session_it_watches() {
        let r = rules();
        let p = program(
            "strategy \"m\" language 1\n\
             on come-out:\n    bet pass base\n\
             on roll when point != 0 and live-come < 2:\n    bet come base\n\
             on roll when point != 0 and up(pass):\n    bet odds on pass max\n",
        );
        for seed in 0..200u64 {
            let plain = run_program_session(&p, &r, 1000, 30_000, None, 200_000, 400, seed);
            let bench = bench_session(&p, &r, 1000, 30_000, None, 200_000, 400, seed);
            assert_eq!(plain.ruin.rolls, bench.outcome.ruin.rolls, "seed {seed}");
            assert_eq!(
                plain.horizon.final_cents, bench.outcome.horizon.final_cents,
                "seed {seed}"
            );
        }
    }

    /// The ledger reconciles: every roll's cash is the previous cash plus
    /// what the events on that roll moved. If this drifts, the ledger is
    /// decorative.
    #[test]
    fn the_ledger_accounts_for_every_cent() {
        let r = rules();
        let p = program(
            "strategy \"p\" language 1\n\
             on come-out:\n    bet pass base\n\
             on roll when point != 0 and point != 6:\n    bet place 6 base\n",
        );
        let t = bench_session(&p, &r, 1000, 30_000, None, 200_000, 200, 7);
        let mut cash = 30_000i64;
        for roll in &t.rolls {
            for e in &roll.events {
                cash += match e.event.kind {
                    BetEventKind::Placed => -e.event.stake_cents,
                    BetEventKind::Won {
                        paid_cents,
                        stake_returned,
                    } => {
                        paid_cents
                            + if stake_returned {
                                e.event.stake_cents
                            } else {
                                0
                            }
                    }
                    BetEventKind::TakenDown | BetEventKind::Returned => e.event.stake_cents,
                    // A losing stake left the rail when it was placed.
                    BetEventKind::Lost
                    | BetEventKind::Pushed
                    | BetEventKind::Traveled { .. }
                    | BetEventKind::Rejected { .. }
                    | BetEventKind::ClippedToMax => 0,
                };
            }
            assert_eq!(
                cash, roll.cash_after,
                "roll {}: the ledger and the rail disagree",
                roll.roll
            );
        }
    }

    /// Money is attributed to the rule that asked for it, and the table's
    /// own doings are attributed to nobody.
    #[test]
    fn placements_name_their_rule_and_resolutions_name_none() {
        let r = rules();
        let p = program(
            "strategy \"two\" language 1\n\
             on come-out:\n    bet pass base\n\
             on roll when point != 0 and point != 8:\n    bet place 8 base\n",
        );
        let t = bench_session(&p, &r, 1000, 30_000, None, 200_000, 200, 3);
        let mut saw_pass_from_rule_0 = false;
        let mut saw_place_from_rule_1 = false;
        for roll in &t.rolls {
            for e in &roll.events {
                match (e.event.bet, e.event.kind) {
                    (crate::trace::BetKind::Pass, BetEventKind::Placed) => {
                        assert_eq!(e.rule, Some(0));
                        saw_pass_from_rule_0 = true;
                    }
                    (crate::trace::BetKind::Place(8), BetEventKind::Placed) => {
                        assert_eq!(e.rule, Some(1));
                        saw_place_from_rule_1 = true;
                    }
                    (_, BetEventKind::Won { .. } | BetEventKind::Lost) => {
                        assert_eq!(e.rule, None, "the table's own doing has no author");
                    }
                    _ => {}
                }
            }
        }
        assert!(saw_pass_from_rule_0 && saw_place_from_rule_1);
    }

    /// The come-point trigger fires when a come flat reaches a number, and
    /// only then — not every roll the point stands, which is all the
    /// language could approximate before.
    #[test]
    fn a_come_point_trigger_fires_on_establishment_only() {
        let r = rules();
        let standing = program(
            "strategy \"standing\" language 1\n\
             on come-out:\n    bet pass\n\
             on roll when point != 0 and live-come < 2:\n    bet come\n\
             on roll when come-point(6):\n    bet odds on come 6 max\n",
        );
        let edge = program(
            "strategy \"edge\" language 1\n\
             on come-out:\n    bet pass\n\
             on roll when point != 0 and live-come < 2:\n    bet come\n\
             on come point on 6:\n    bet odds on come 6 max\n",
        );
        let a = bench_session(&standing, &r, 1000, 100_000, None, 200_000, 300, 4);
        let b = bench_session(&edge, &r, 1000, 100_000, None, 200_000, 300, 4);
        assert!(
            b.fire_counts[2] > 0,
            "the come-point trigger never fired at all"
        );
        // A come point on the 6 resolves on a 6 or a 7 — 11 of 36 — so it
        // stands 36/11 ≈ 3.27 rolls on average. The state test fires once
        // per roll it stands; the event test fires once per establishment.
        // Their ratio is therefore that expected duration, which is a
        // sharper claim than "fewer" and would catch the trigger firing on
        // the wrong roll as well as on every roll.
        let ratio = a.fire_counts[2] as f64 / b.fire_counts[2] as f64;
        assert!(
            (2.6..4.0).contains(&ratio),
            "the state test fired {} times and the event test {}, a ratio of \
             {ratio:.2}; a come point stands 36/11 ≈ 3.27 rolls, so this is \
             not the event it claims to be",
            a.fire_counts[2],
            b.fire_counts[2]
        );
    }

    /// A payout can be read, so "press it by half of what it paid" is a
    /// rule rather than only a progression.
    #[test]
    fn a_payout_can_be_pressed_with() {
        let r = rules();
        let p = program(
            "strategy \"halfpress\" language 1\n\
             on roll when point != 0 and point != 6:\n    bet place 6\n\
             on win of place 6:\n\
                 press place 6 to stake(place 6) + paid(place 6) / 2\n",
        );
        let t = bench_session(&p, &r, 1000, 100_000, None, 200_000, 400, 9);
        assert!(t.fire_counts[1] > 0, "the press rule never fired");
        // A place 6 pays 7:6 on a $12 base — $14 — so half of it is $7, and
        // the pressed bet rounds to the $6 unit above $19.
        let pressed = t.rolls.iter().flat_map(|x| &x.events).any(|e| {
            matches!(e.event.bet, crate::trace::BetKind::Place(6))
                && matches!(e.event.kind, BetEventKind::Placed)
                && e.event.stake_cents > 0
                && e.rule == Some(1)
        });
        assert!(pressed, "no press was ever attributed to the rule");
    }

    /// A rule that cannot fire is reported, which is the whole point: the
    /// author sees a zero instead of inferring one from a distribution.
    #[test]
    fn a_rule_that_never_fires_is_named() {
        let r = rules();
        let p = program(
            "strategy \"dead\" language 1\n\
             on come-out:\n    bet pass base\n\
             on roll when point == 11:\n    bet field base\n",
        );
        let t = bench_session(&p, &r, 1000, 30_000, None, 200_000, 300, 1);
        assert_eq!(t.never_fired(), vec![1], "the 11 is never a point");
        assert!(t.fire_counts[0] > 0);
        assert_eq!(
            t.fire_counts.len(),
            2,
            "every rule is counted, fired or not"
        );
    }

    /// A refusal is visible with its reason in words — the other way a rule
    /// set does nothing while looking busy.
    #[test]
    fn refusals_are_visible_with_their_reason() {
        let r = Rules {
            odds_policy: OddsPolicy::None,
            ..rules()
        };
        // Asking for odds at a table that allows none: legal to write, and
        // refused every time it is asked.
        let p = program(
            "strategy \"hopeful\" language 1\n\
             on come-out:\n    bet pass base\n\
             on roll when point != 0 and up(pass):\n    bet odds on pass max\n",
        );
        let t = bench_session(&p, &r, 1000, 30_000, None, 200_000, 200, 5);
        let refusals = t.refusals();
        assert!(!refusals.is_empty(), "the refusals never surfaced");
        let (_, first) = refusals[0];
        assert_eq!(first.rule, Some(1), "and the refusal names who asked");
        let BetEventKind::Rejected { reason } = first.event.kind else {
            panic!("not a refusal")
        };
        assert_eq!(reason.label(), "odds policy allows none");
    }

    /// The whole point of stepping: the layout at any roll is inspectable,
    /// and the dice that got there are recorded.
    #[test]
    fn the_session_can_be_walked_roll_by_roll() {
        let r = rules();
        let p = program("strategy \"s\" language 1\non come-out:\n    bet pass base\n");
        let t = bench_session(&p, &r, 1000, 30_000, None, 200_000, 50, 11);
        assert!(t.rolls.len() > 10);
        for (i, roll) in t.rolls.iter().enumerate() {
            assert_eq!(roll.roll as usize, i + 1, "rolls are numbered in order");
            assert!((1..=6).contains(&roll.dice.0) && (1..=6).contains(&roll.dice.1));
            assert!(
                roll.wealth_after >= roll.cash_after,
                "wealth includes what is still on the felt"
            );
        }
        let _ = BetRef::Pass;
    }
}
