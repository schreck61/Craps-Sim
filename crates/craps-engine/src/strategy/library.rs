// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The checkbox player, written in the language.
//!
//! [`from_selection`] turns a [`BetSelection`] into the rule set that plays
//! it. This is not a convenience: it is how the language proves it can say
//! everything the engine could already say. Every curated strategy is fed
//! through it and simulated against its hand-written twin on the same dice,
//! and the two must agree to the cent (`compiled_matches_builtin`).
//!
//! The rules come out in the order the built-in player places bets, because
//! order is observable: when a bankroll runs short, which bet gets the last
//! dollar depends on who asked first.

use crate::bets::{BetSelection, OddsPolicy, Rules, HARD_NUMS, PLACE_NUMS};
use crate::strategy::ast::{AmountExpr, BinOp, Expr, Read, Rule, Stmt, Strategy, Trigger};
use crate::strategy::BetRef;

/// `point != n` — the guard that keeps the point number off the layout and
/// keeps a place bet from being refused once a roll for the whole session.
fn point_is_not(n: u8) -> Expr {
    Expr::bin(BinOp::Ne, Expr::Read(Read::Point), Expr::Const(n as i64))
}

/// `point != 0` — "a point is on".
fn point_is_on() -> Expr {
    Expr::bin(BinOp::Ne, Expr::Read(Read::Point), Expr::Const(0))
}

fn and(a: Expr, b: Expr) -> Expr {
    Expr::bin(BinOp::And, a, b)
}

fn bet(b: BetRef) -> Stmt {
    Stmt::Bet(b, AmountExpr::Base)
}

fn odds(b: BetRef) -> Stmt {
    Stmt::Bet(b, AmountExpr::MaxOdds)
}

/// The rule set that plays a checkbox selection.
///
/// `rules` is needed because the odds policy decides whether odds rules
/// exist at all: a player whose table allows no odds does not stand there
/// asking for them and being refused once a roll.
pub fn from_selection(sel: &BetSelection, rules: &Rules) -> Strategy {
    let takes_odds = sel.take_odds && rules.odds_policy != OddsPolicy::None;
    let mut out: Vec<Rule> = Vec::new();

    // --- Come-out: line bets only. ---
    if sel.pass_line {
        out.push(Rule::new(Trigger::ComeOut, vec![bet(BetRef::Pass)]));
    }
    if sel.dont_pass {
        out.push(Rule::new(Trigger::ComeOut, vec![bet(BetRef::DontPass)]));
    }

    // --- With a point on, in the order the built-in player asks. ---
    if sel.come_max > 0 {
        out.push(Rule::new(Trigger::Roll, vec![bet(BetRef::Come)]).when(and(
            point_is_on(),
            Expr::bin(
                BinOp::Lt,
                Expr::Read(Read::LiveCome),
                Expr::Const(sel.come_max as i64),
            ),
        )));
    }
    if sel.dont_come_max > 0 {
        out.push(
            Rule::new(Trigger::Roll, vec![bet(BetRef::DontCome)]).when(and(
                point_is_on(),
                Expr::bin(
                    BinOp::Lt,
                    Expr::Read(Read::LiveDontCome),
                    Expr::Const(sel.dont_come_max as i64),
                ),
            )),
        );
    }
    if takes_odds {
        // Odds need a flat behind them; asking without one is a refusal, so
        // the rule asks only when there is something to back.
        out.push(
            Rule::new(Trigger::Roll, vec![odds(BetRef::PassOdds)])
                .when(and(point_is_on(), Expr::Read(Read::Up(BetRef::Pass)))),
        );
        out.push(
            Rule::new(Trigger::Roll, vec![odds(BetRef::DontPassLay)])
                .when(and(point_is_on(), Expr::Read(Read::Up(BetRef::DontPass)))),
        );
    }
    for (i, &num) in PLACE_NUMS.iter().enumerate() {
        if sel.place[i] {
            out.push(
                Rule::new(Trigger::Roll, vec![bet(BetRef::Place(num))])
                    .when(and(point_is_on(), point_is_not(num))),
            );
        }
    }
    for (i, &num) in HARD_NUMS.iter().enumerate() {
        if sel.hardways[i] {
            out.push(Rule::new(Trigger::Roll, vec![bet(BetRef::Hardway(num))]).when(point_is_on()));
        }
    }

    // --- Odds behind come points, toppable at any time. ---
    if takes_odds {
        for &num in PLACE_NUMS.iter() {
            if sel.come_max > 0 {
                out.push(
                    Rule::new(Trigger::Roll, vec![odds(BetRef::ComeOdds(num))])
                        .when(Expr::Read(Read::ComePoint(num))),
                );
            }
            if sel.dont_come_max > 0 {
                out.push(
                    Rule::new(Trigger::Roll, vec![odds(BetRef::DontComeLay(num))])
                        .when(Expr::Read(Read::DontComePoint(num))),
                );
            }
        }
    }

    // --- One-roll bets, working every roll. ---
    if sel.field {
        out.push(Rule::new(Trigger::Roll, vec![bet(BetRef::Field)]));
    }
    if sel.any_seven {
        out.push(Rule::new(Trigger::Roll, vec![bet(BetRef::AnySeven)]));
    }
    if sel.any_craps {
        out.push(Rule::new(Trigger::Roll, vec![bet(BetRef::AnyCraps)]));
    }

    Strategy::new("checkbox player", out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bets::OddsPolicy;
    use crate::session::{run_program_session, run_session};
    use crate::strategy::compile;
    use crate::sweep::explore_strategies;

    fn rules(odds: OddsPolicy) -> Rules {
        Rules {
            odds_policy: odds,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 1000,
        }
    }

    /// The milestone's own gate: every curated strategy, compiled from its
    /// checkbox form, must live the same session on the same dice as the
    /// hand-written player — to the cent, to the roll, over every seed.
    ///
    /// This is what earns the right to eventually delete `place_bets`.
    #[test]
    fn compiled_matches_builtin_across_the_curated_strategies() {
        for policy in [OddsPolicy::None, OddsPolicy::X345, OddsPolicy::X10] {
            let r = rules(policy);
            for (name, sel) in explore_strategies() {
                let program = compile(&from_selection(&sel, &r))
                    .unwrap_or_else(|e| panic!("{name}: {}", e.message()));
                for seed in 0..400u64 {
                    let want = run_session(&sel, &r, 1000, 30_000, None, 200_000, 400, seed);
                    let got =
                        run_program_session(&program, &r, 1000, 30_000, None, 200_000, 400, seed);
                    assert_eq!(
                        want.ruin.rolls, got.ruin.rolls,
                        "{name} @ {policy:?} seed {seed}: died on a different roll"
                    );
                    assert_eq!(
                        want.horizon.final_cents, got.horizon.final_cents,
                        "{name} @ {policy:?} seed {seed}: walked out with a different number"
                    );
                    assert_eq!(
                        want.peak_outlay_cents, got.peak_outlay_cents,
                        "{name} @ {policy:?} seed {seed}: different peak outlay"
                    );
                    assert_eq!(
                        want.horizon_handle_cents, got.horizon_handle_cents,
                        "{name} @ {policy:?} seed {seed}: different handle"
                    );
                }
            }
        }
    }

    /// The same gate, deep enough to mean it: 10,000 seeds per strategy.
    /// Kept out of the default tier because it is a minute of work, not
    /// because it is optional — it runs before the milestone ships.
    ///
    ///   cargo test --release -p craps-engine -- --ignored deep_equivalence --nocapture
    #[test]
    #[ignore]
    fn deep_equivalence_compiled_matches_builtin() {
        let r = rules(OddsPolicy::X345);
        for (name, sel) in explore_strategies() {
            let program = compile(&from_selection(&sel, &r)).unwrap();
            for seed in 0..10_000u64 {
                let want = run_session(&sel, &r, 1000, 30_000, None, 200_000, 400, seed);
                let got = run_program_session(&program, &r, 1000, 30_000, None, 200_000, 400, seed);
                assert_eq!(
                    (
                        want.ruin.rolls,
                        want.horizon.final_cents,
                        want.peak_outlay_cents,
                        want.horizon_handle_cents
                    ),
                    (
                        got.ruin.rolls,
                        got.horizon.final_cents,
                        got.peak_outlay_cents,
                        got.horizon_handle_cents
                    ),
                    "{name} seed {seed}"
                );
            }
            println!("{name}: 10,000 seeds identical");
        }
    }

    /// A compiled checkbox player reads no derived history: the mask it
    /// declares is empty, so it pays for none of the accumulators.
    #[test]
    fn the_checkbox_player_declares_no_features() {
        let r = rules(OddsPolicy::X345);
        for (name, sel) in explore_strategies() {
            let p = compile(&from_selection(&sel, &r)).unwrap();
            assert!(p.features.is_empty(), "{name} declared {:?}", p.features);
        }
    }

    /// Two identical strategies hash the same; one changed bet does not.
    #[test]
    fn the_hash_identifies_the_compiled_form() {
        let r = rules(OddsPolicy::X345);
        let mut a = BetSelection::default();
        a.set_place(6, true);
        let mut b = a.clone();
        b.set_place(8, true);
        let pa = compile(&from_selection(&a, &r)).unwrap();
        let pa2 = compile(&from_selection(&a, &r)).unwrap();
        let pb = compile(&from_selection(&b, &r)).unwrap();
        assert_eq!(pa.hash, pa2.hash);
        assert_ne!(pa.hash, pb.hash);
    }

    #[test]
    fn a_strategy_that_never_bets_is_refused() {
        let r = rules(OddsPolicy::None);
        let none = BetSelection {
            pass_line: false,
            ..Default::default()
        };
        assert_eq!(
            compile(&from_selection(&none, &r)),
            Err(crate::strategy::CompileError::NeverBets)
        );
    }
}
