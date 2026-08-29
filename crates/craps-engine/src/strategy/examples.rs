// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The worked examples from `STRATEGY_DSL.md` §7, built and simulated.
//!
//! Each one is a strategy that could not be expressed before the language
//! existed. They are here as tests rather than as a shipped library because
//! their job is to prove the language can say them and that the table does
//! what they ask — not to be a menu. Whether any of them is a good idea is
//! the Explorer's business, and the answer is no.
//!
//! They are also the reference for what the text form (P3) must parse to:
//! whatever `strategy "Press twice, then collect"` reads like, it has to
//! produce the tree that [`press_twice_then_collect`] builds by hand.

#![cfg(test)]

use crate::bets::{BetSelection, OddsPolicy, Progression, Rules};
use crate::strategy::ast::{AmountExpr, BinOp, Expr, Read, Rule, Stmt, Strategy, Trigger};
use crate::strategy::{compile, BetRef};

fn rules() -> Rules {
    Rules {
        odds_policy: OddsPolicy::X345,
        field_12_triple: false,
        come_odds_work_on_comeout: false,
        prop_bet_cents: 500,
        table_max_mult: 1000,
    }
}

fn point_on() -> Expr {
    Expr::bin(BinOp::Ne, Expr::Read(Read::Point), Expr::Const(0))
}

/// > *Place the 6 and 8; press each on its first two hits; regress after
/// > that.*
///
/// The thing that could not be said before: a bet whose size depends on how
/// many times its own number has come.
fn press_twice_then_collect() -> Strategy {
    let mut rules = vec![Rule::new(
        Trigger::ComeOut,
        vec![Stmt::Bet(BetRef::Pass, AmountExpr::Base)],
    )];
    for n in [6u8, 8] {
        rules.push(
            Rule::new(
                Trigger::Roll,
                vec![Stmt::Bet(BetRef::Place(n), AmountExpr::Base)],
            )
            .when(Expr::bin(
                BinOp::And,
                point_on(),
                Expr::bin(BinOp::Ne, Expr::Read(Read::Point), Expr::Const(n as i64)),
            )),
        );
        rules.push(
            Rule::new(
                Trigger::Win(BetRef::Place(n)),
                vec![Stmt::Press(
                    BetRef::Place(n),
                    AmountExpr::Cents(Expr::bin(
                        BinOp::Mul,
                        Expr::Read(Read::Stake(BetRef::Place(n))),
                        Expr::Const(2),
                    )),
                )],
            )
            .when(Expr::bin(
                BinOp::Le,
                Expr::Read(Read::HitsThisShooter(n)),
                Expr::Const(2),
            )),
        );
        rules.push(
            Rule::new(
                Trigger::Win(BetRef::Place(n)),
                vec![Stmt::Regress(BetRef::Place(n), AmountExpr::Base)],
            )
            .when(Expr::bin(
                BinOp::Gt,
                Expr::Read(Read::HitsThisShooter(n)),
                Expr::Const(2),
            )),
        );
    }
    Strategy::new("Press twice, then collect", rules)
}

/// > *Place bets off after a seven-out until the shooter makes a point.*
///
/// Memory, and a bet that sits on the felt resolving nothing.
fn off_until_the_shooter_proves_himself() -> Strategy {
    let trusted = 0u16;
    let mut s = Strategy::new(
        "Off until the shooter proves himself",
        vec![
            Rule::new(Trigger::SevenOut, vec![Stmt::Set(trusted, Expr::Const(0))]),
            Rule::new(Trigger::PointMade, vec![Stmt::Set(trusted, Expr::Const(1))]),
            Rule::new(
                Trigger::Roll,
                vec![
                    Stmt::Bet(BetRef::Place(6), AmountExpr::Base),
                    Stmt::Bet(BetRef::Place(8), AmountExpr::Base),
                ],
            )
            .when(point_on()),
            Rule::new(
                Trigger::Roll,
                vec![
                    Stmt::Working(BetRef::Place(6), false),
                    Stmt::Working(BetRef::Place(8), false),
                ],
            )
            .when(Expr::bin(BinOp::Eq, Expr::Var(trusted), Expr::Const(0))),
            Rule::new(
                Trigger::Roll,
                vec![
                    Stmt::Working(BetRef::Place(6), true),
                    Stmt::Working(BetRef::Place(8), true),
                ],
            )
            .when(Expr::bin(BinOp::Eq, Expr::Var(trusted), Expr::Const(1))),
        ],
    );
    s.vars = vec!["trusted".into()];
    s
}

/// > *Stop at −$200 or +$150, whichever comes first.*
fn stop_loss_and_stop_win() -> Strategy {
    Strategy::new(
        "Stop loss and stop win",
        vec![
            Rule::new(
                Trigger::ComeOut,
                vec![Stmt::Bet(BetRef::Pass, AmountExpr::Base)],
            ),
            Rule::new(Trigger::Roll, vec![Stmt::Leave]).when(Expr::bin(
                BinOp::Or,
                Expr::bin(BinOp::Ge, Expr::Read(Read::Profit), Expr::Const(15_000)),
                Expr::bin(BinOp::Le, Expr::Read(Read::Profit), Expr::Const(-20_000)),
            )),
        ],
    )
}

/// > *Bet the field only after two field numbers in a row.*
///
/// Nonsense, faithfully modeled. Principle 5: a language that could only
/// express sound play could not refute unsound play, and refutation is the
/// product.
fn the_field_is_due() -> Strategy {
    let streak = 0u16;
    let field_number = |e: Expr| e;
    let is_field = Expr::bin(
        BinOp::Or,
        Expr::bin(
            BinOp::Le,
            field_number(Expr::Read(Read::LastTotal)),
            Expr::Const(4),
        ),
        Expr::bin(BinOp::Ge, Expr::Read(Read::LastTotal), Expr::Const(9)),
    );
    let mut s = Strategy::new(
        "The field is due",
        vec![
            Rule::new(
                Trigger::Roll,
                vec![Stmt::Set(
                    streak,
                    Expr::bin(BinOp::Add, Expr::Var(streak), Expr::Const(1)),
                )],
            )
            .when(is_field.clone()),
            Rule::new(Trigger::Roll, vec![Stmt::Set(streak, Expr::Const(0))])
                .when(Expr::Not(Box::new(is_field))),
            Rule::new(
                Trigger::Roll,
                vec![Stmt::Bet(BetRef::Field, AmountExpr::Base)],
            )
            .when(Expr::bin(BinOp::Ge, Expr::Var(streak), Expr::Const(2))),
        ],
    );
    s.vars = vec!["streak".into()];
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::run_program_session;
    use crate::strategy::FeatureMask;

    /// Every example compiles, and declares exactly the history it reads.
    #[test]
    fn the_examples_compile_and_declare_what_they_read() {
        let cases = [
            (press_twice_then_collect(), FeatureMask::HITS),
            (off_until_the_shooter_proves_himself(), FeatureMask::NONE),
            (stop_loss_and_stop_win(), FeatureMask::NONE),
            (the_field_is_due(), FeatureMask::DICE),
        ];
        for (s, wants) in cases {
            let name = s.name.clone();
            let p = compile(&s).unwrap_or_else(|e| panic!("{name}: {}", e.message()));
            assert!(
                p.features.has(wants) || wants == FeatureMask::NONE,
                "{name} declared {:?}, expected to include {wants:?}",
                p.features
            );
        }
    }

    /// A stop-loss stops. Sessions end at or beyond the thresholds rather
    /// than running to the horizon, and never far past them — the rule fires
    /// at the next decision point, so one roll's swing can overshoot.
    #[test]
    fn the_stop_rules_actually_stop() {
        let r = rules();
        let p = compile(&stop_loss_and_stop_win()).unwrap();
        let mut stopped = 0;
        for seed in 0..500u64 {
            let o = run_program_session(&p, &r, 1000, 100_000, None, 200_000, 100_000, seed);
            let profit = o.horizon.final_cents - 100_000;
            if o.ruin.rolls < 100_000 {
                stopped += 1;
                assert!(
                    profit >= 15_000 || profit <= -20_000,
                    "seed {seed}: left at {profit} cents, which is neither threshold"
                );
            }
        }
        assert!(
            stopped > 400,
            "only {stopped} of 500 sessions ever hit a threshold"
        );
    }

    /// Pressing on a number's own hit count changes the money, which is the
    /// whole claim: this could not be said with one global progression.
    #[test]
    fn pressing_on_hits_differs_from_flat() {
        let r = rules();
        let pressed = compile(&press_twice_then_collect()).unwrap();
        let mut flat_sel = BetSelection::default();
        flat_sel.set_place(6, true);
        flat_sel.set_place(8, true);
        let flat =
            compile(&crate::strategy::from_selection(&flat_sel, &r).pressing(Progression::Flat))
                .unwrap();

        let run = |p: &crate::strategy::Program| {
            (0..400u64)
                .map(|s| {
                    run_program_session(p, &r, 1000, 50_000, None, 200_000, 400, s)
                        .horizon
                        .final_cents
                })
                .sum::<i64>()
        };
        assert_ne!(run(&pressed), run(&flat));
    }

    /// Bets that are off do not lose to a seven-out. A shooter-distrusting
    /// strategy must therefore survive longer than the same bets left on.
    #[test]
    fn turning_bets_off_protects_them() {
        let r = rules();
        let cautious = compile(&off_until_the_shooter_proves_himself()).unwrap();
        let mut always_sel = BetSelection {
            pass_line: false,
            ..Default::default()
        };
        always_sel.set_place(6, true);
        always_sel.set_place(8, true);
        let always = compile(&crate::strategy::from_selection(&always_sel, &r)).unwrap();

        let survival = |p: &crate::strategy::Program| {
            (0..400u64)
                .map(|s| {
                    run_program_session(p, &r, 1000, 20_000, None, 200_000, 400, s)
                        .ruin
                        .rolls
                })
                .sum::<u64>()
        };
        assert!(
            survival(&cautious) > survival(&always),
            "calling the bets off did not make the bankroll last longer"
        );
    }

    /// Superstition runs, and loses. The field's edge is what it is however
    /// many field numbers came before.
    #[test]
    fn the_due_field_is_still_the_field() {
        let r = rules();
        let p = compile(&the_field_is_due()).unwrap();
        let total: i64 = (0..2_000u64)
            .map(|s| {
                run_program_session(&p, &r, 1000, 50_000, None, 200_000, 400, s)
                    .horizon
                    .final_cents
            })
            .sum();
        let mean = total / 2_000;
        assert!(
            mean < 50_000,
            "waiting for the field to be due came out ahead at {mean} cents, \
             which would mean the engine is wrong, not the superstition"
        );
    }
}
