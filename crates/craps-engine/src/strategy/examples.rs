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
//! They are written in the language itself rather than assembled as trees,
//! which is the point: if the text form could not say them, it would not be
//! the second editor Principle 2 claims it is — it would be a subset with a
//! nicer face.
//!
//! They ship, via [`EXAMPLES`]. The first version kept them behind
//! `cfg(test)` on the grounds that their job was to prove the language can
//! say them and not to be a menu — which conflated a menu of *recommended
//! play*, which this app rightly refuses to offer, with a menu of *syntax
//! demonstrations*, without which nobody can start writing. Most of these
//! are bad bets and one is deliberately superstitious; none of them is
//! advice.

#[cfg(test)]
use crate::bets::{BetSelection, OddsPolicy, Progression, Rules};
#[cfg(test)]
use crate::strategy::ast::Strategy;
#[cfg(test)]
use crate::strategy::{compile, parse};

/// The examples, named, for an app that has to teach this language to
/// somebody who has never seen it.
pub const EXAMPLES: &[(&str, &str)] = &[
    ("Press twice, then collect", PRESS_TWICE),
    ("Off until the shooter proves himself", OFF_UNTIL_TRUSTED),
    ("Stop loss and stop win", STOP_RULES),
    ("The field is due (superstition)", FIELD_IS_DUE),
];

#[cfg(test)]
fn rules() -> Rules {
    Rules {
        odds_policy: OddsPolicy::X345,
        field_12_triple: false,
        come_odds_work_on_comeout: false,
        prop_bet_cents: 500,
        table_max_mult: 1000,
        place_the_point: false,
    }
}

/// > *Place the 6 and 8; press each on its first two hits; regress after
/// > that.*
///
/// The thing that could not be said before: a bet whose size depends on how
/// many times its own number has come.
///
/// *And for two milestones it did not climb, and then it climbed too far.*
/// A progression re-prices its bet where the bet resolves, and a flat stream
/// re-priced it back to base — so every press was torn down by the very win
/// it was riding. Fixing that by writing the rule's figure into the stream
/// then let a pressed level outlive the bet it belonged to: a seven-out took
/// the bet, the next roll put a fresh one up at base, and resolution topped
/// it straight back to the stale level, from which the rule pressed again.
/// The stake ratcheted past $384 with nothing asking it to.
///
/// A flat stream does not re-price a winner at all, which is what "flat"
/// meant before anyone wrote it down. Two tests hold both ends: one walks
/// away once the 6 reaches $24, and one fails if it ever passes it.
pub const PRESS_TWICE: &str = r#"
strategy "Press twice, then collect" language 1

on come-out:
    bet pass base

for each of 6, 8 as n {
    on roll when point != 0 and point != n:
        bet place n base
}

for each of 6, 8 as n {
    on win of place n when hits-this-shooter(n) <= 2:
        press place n to stake(place n) * 2

    on win of place n when hits-this-shooter(n) > 2:
        regress place n to base
}
"#;

/// > *Place bets off after a seven-out until the shooter makes a point.*
///
/// Memory, and a bet that sits on the felt resolving nothing.
pub const OFF_UNTIL_TRUSTED: &str = r#"
strategy "Off until the shooter proves himself" language 1

var trusted = 0

on seven-out:
    set trusted = 0

on point-made:
    set trusted = 1

on roll when point != 0:
    bet place 6 base
    bet place 8 base

on roll when trusted == 0:
    working place 6 off
    working place 8 off

on roll when trusted == 1:
    working place 6 on
    working place 8 on
"#;

/// > *Stop at −$200 or +$150, whichever comes first.*
pub const STOP_RULES: &str = r#"
strategy "Stop loss and stop win" language 1

on come-out:
    bet pass base

on roll when profit >= $150 or profit <= -$200:
    leave "enough"
"#;

/// > *Bet the field only after two field numbers in a row.*
///
/// Nonsense, faithfully modeled. Principle 5: a language that could only
/// express sound play could not refute unsound play, and refutation is the
/// product.
///
/// The low half of the field is written `last-total >= 2 and last-total <= 4`
/// rather than `last-total <= 4`, and the lower bound is load-bearing. A
/// decision point comes before every roll including the session's first,
/// where `last-total` reads 0 — and 0 is under 4. The shorter condition
/// counted a field number that had not happened, so the streak began at one
/// and the strategy bet after a single real field number rather than two: a
/// superstition modelled one roll more eagerly than the superstition itself.
/// Two is not a total the dice can fail to reach, so saying so costs nothing
/// and stops the phantom.
pub const FIELD_IS_DUE: &str = r#"
strategy "The field is due" language 1

var streak = 0

on roll when last-total >= 2 and last-total <= 4 or last-total >= 9:
    set streak = streak + 1

on roll when not (last-total >= 2 and last-total <= 4 or last-total >= 9):
    set streak = 0

on roll when streak >= 2:
    bet field base
"#;

/// Every example, as written above.
#[cfg(test)]
fn all() -> Vec<Strategy> {
    [PRESS_TWICE, OFF_UNTIL_TRUSTED, STOP_RULES, FIELD_IS_DUE]
        .iter()
        .map(|src| parse(src).unwrap_or_else(|e| panic!("{}", e.message())))
        .collect()
}

#[cfg(test)]
fn press_twice_then_collect() -> Strategy {
    parse(PRESS_TWICE).unwrap()
}

#[cfg(test)]
fn off_until_the_shooter_proves_himself() -> Strategy {
    parse(OFF_UNTIL_TRUSTED).unwrap()
}

#[cfg(test)]
fn stop_loss_and_stop_win() -> Strategy {
    parse(STOP_RULES).unwrap()
}

#[cfg(test)]
fn the_field_is_due() -> Strategy {
    parse(FIELD_IS_DUE).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::run_program_session;
    use crate::strategy::FeatureMask;

    /// Every example compiles, and declares exactly the history it reads.
    /// Every example survives the round trip, so what is written above is
    /// exactly what the tree holds.
    #[test]
    fn the_examples_round_trip_through_their_own_text() {
        for s in all() {
            let back = crate::strategy::parse(&crate::strategy::render(&s)).unwrap();
            assert_eq!(s, back, "{}", s.name);
        }
    }

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
            // `has(NONE)` is false for every mask, so the old `|| wants ==
            // NONE` rescued the two NONE cases unconditionally and this
            // asserted nothing about them at all.
            if wants == FeatureMask::NONE {
                assert!(
                    p.features.is_empty(),
                    "{name} declared {:?}, and reads nothing that needs it",
                    p.features
                );
            } else {
                assert!(
                    p.features.has(wants),
                    "{name} declared {:?}, expected to include {wants:?}",
                    p.features
                );
            }
        }
    }

    /// *Press each on its first two hits* means two presses, and then no
    /// more.
    ///
    /// It climbed without limit. Making a decision-point press survive the
    /// next resolution was first done by writing the rule's figure into the
    /// bet's progression stream — and a pressed level then outlived the bet
    /// it belonged to. A seven-out took the bet, `bet place n base` put a
    /// fresh one up, and the next resolution topped it straight back to the
    /// stale level, from which the rule pressed again. The stake ratcheted
    /// with nothing asking it to, past $384 within forty sessions.
    ///
    /// A flat stream does not re-price a winner at all now, which is what
    /// "flat" meant before anyone wrote it down.
    #[test]
    fn pressing_twice_presses_exactly_twice() {
        let r = rules();
        // At a $5 table the 6 takes $6, so two presses stand at $24 and a
        // third would show at $48.
        let src = format!(
            "{PRESS_TWICE}\non roll when stake(place 6) > $24 or stake(place 8) > $24:\n    leave\n"
        );
        let p = compile(&parse(&src).unwrap()).unwrap();
        let over: u32 = (0..40u64)
            .map(|seed| {
                crate::strategy::bench_session(&p, &r, 500, 100_000, None, 300, 300, seed)
                    .fire_counts[p.rule_count() - 1]
            })
            .sum();
        assert_eq!(over, 0, "a bet pressed past its second press");
    }

    /// The prose says *press each on its first two hits*, which means the
    /// bet climbs: base, then twice base, then four times.
    ///
    /// It did not. A progression re-prices a bet where it resolves, and the
    /// stream still thought this one was worth its base — so every press was
    /// torn back down by the very win it was riding, and the second press
    /// recomputed the first one forever. The example was the counterexample
    /// to its own caption, and this is the test that would have said so.
    #[test]
    fn pressing_twice_actually_climbs() {
        let r = rules();
        // A tripwire rule appended to the example itself: at a $5 table the
        // 6 takes $6, so a bet that has been pressed twice stands at $24.
        let src = format!("{PRESS_TWICE}\non roll when stake(place 6) >= $24:\n    leave\n");
        let s = parse(&src).unwrap_or_else(|e| panic!("{}", e.message()));
        let p = compile(&s).unwrap();
        let tripwire = p.rule_count() - 1;
        let climbed: u32 = (0..40u64)
            .map(|seed| {
                crate::strategy::bench_session(&p, &r, 500, 100_000, None, 300, 300, seed)
                    .fire_counts[tripwire]
            })
            .sum();
        assert!(
            climbed > 0,
            "the 6 never reached twice-pressed in forty sessions"
        );
    }

    /// The superstition waits for *two* field numbers, not one.
    ///
    /// A decision point comes before every roll including the session's
    /// first, where `last-total` reads 0 — and 0 is under 4, so a condition
    /// written as `last-total <= 4` counted a field number that had not
    /// happened. The streak began at one and the example bet after a single
    /// real field number: a superstition modelled more eagerly than the
    /// superstition itself, in the one example whose whole job is to be
    /// faithful to a belief so the app can refute it.
    #[test]
    fn the_due_field_waits_for_two_of_them() {
        // The strategy's own counter, with a tripwire on the phantom: if the
        // streak is ever above zero before a roll has resolved, the count is
        // of something that did not happen.
        let src = FIELD_IS_DUE.replace(
            "on roll when streak >= 2:\n    bet field base",
            "on roll when roll == 0 and streak >= 1:\n    leave\n\non roll when streak >= 2:\n    bet field base",
        );
        let s = parse(&src).unwrap_or_else(|e| panic!("{}", e.message()));
        let p = compile(&s).unwrap();
        let phantom: u32 = (0..40u64)
            .map(|seed| {
                crate::strategy::bench_session(&p, &rules(), 500, 100_000, None, 200, 200, seed)
                    .fire_counts[2]
            })
            .sum();
        assert_eq!(
            phantom, 0,
            "the streak counted a field number before any roll had happened"
        );
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
