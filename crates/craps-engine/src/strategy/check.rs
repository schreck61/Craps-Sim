// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! What a strategy can be told about itself before a single die rolls.
//!
//! [`STRATEGY_DSL.md`](../../../../docs/STRATEGY_DSL.md) §9 asks for six
//! compile-time checks, shown as plain sentences in the register the order
//! ticket already uses. One of them — never-bets — refuses to compile at all,
//! because a strategy that cannot put money at risk has nothing to simulate.
//! The rest are *diagnostics*: things worth saying about a strategy that
//! still runs.
//!
//! That distinction is the whole design of this module. A dead rule is legal.
//! A hedge that cannot hedge is legal, and Principle 5 says it must be —
//! the app exists to model a belief exactly and draw what happens, and a
//! compiler that refused unsound play could not refute unsound play. So
//! nothing here stops a run. It only means the author is never surprised by
//! their own rules.
//!
//! Two of the six need to know the table, which the compiler does not: the
//! worst outlay is measured against a budget and the clipping step against a
//! maximum. Those live in [`against_table`], asked separately by whoever has
//! the configuration in hand.

use crate::bets::{ProgState, Progression, Rules};
use crate::strategy::ast::{BinOp, Expr, Read, Rule, Stmt, Strategy};
use crate::strategy::program::Program;
use crate::strategy::view::{stream_of, STREAMS};
use crate::strategy::BetRef;

/// Something worth telling the author, about a strategy that compiles.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Diagnostic {
    /// A rule whose condition can never hold. It will sit in the rule table
    /// firing zero times, and the Bench will say so after a run — this says
    /// it before one.
    DeadRule { rule: u16, why: &'static str },
    /// Two rules act on the same bet at the same trigger. Ordering resolves
    /// it and the language says so, but the author should hear which one
    /// wins from the editor rather than from the money.
    Conflict {
        loser: u16,
        winner: u16,
        bet: BetRef,
    },
    /// The most this strategy could have on the layout at once, and whether
    /// the budget covers it.
    Exposure {
        worst_cents: i64,
        budget_cents: i64,
        /// False when an amount is computed rather than written, so the real
        /// worst case is this or more.
        exact: bool,
    },
    /// A pressing system that will hit the table maximum, and on which step.
    Clipping {
        progression: Progression,
        step: u32,
        stake_cents: i64,
        table_max_cents: i64,
    },
    /// Instructions walked per decision in the worst case — every rule
    /// firing, every guard passing. Principle 3 is what makes this a number
    /// at all, and a strategy that would halve throughput should say so
    /// before it runs rather than after.
    Cost { ops: usize, rules: usize },
}

impl Diagnostic {
    /// The sentence the editor shows.
    pub fn message(&self) -> String {
        match self {
            Diagnostic::DeadRule { rule, why } => {
                format!("Rule {} can never fire: {why}.", rule + 1)
            }
            Diagnostic::Conflict { loser, winner, bet } => format!(
                "Rules {} and {} both act on {} at the same trigger; rule {} is written later, so it wins.",
                loser + 1,
                winner + 1,
                crate::strategy::bet_name(*bet),
                winner + 1
            ),
            Diagnostic::Exposure {
                worst_cents,
                budget_cents,
                exact,
            } => {
                let at_least = if *exact { "" } else { "at least " };
                if worst_cents > budget_cents {
                    format!(
                        "This strategy can have {at_least}{} on the layout at once, and the budget is {}.",
                        money(*worst_cents),
                        money(*budget_cents)
                    )
                } else {
                    format!(
                        "Worst case {at_least}{} on the layout at once, within the {} budget.",
                        money(*worst_cents),
                        money(*budget_cents)
                    )
                }
            }
            Diagnostic::Clipping {
                progression,
                step,
                stake_cents,
                table_max_cents,
            } => format!(
                "table max {} < {} step {step} stake {} — the progression will clip.",
                money(*table_max_cents),
                progression.label(),
                money(*stake_cents)
            ),
            Diagnostic::Cost { ops, rules } => format!(
                "{ops} instructions per decision in the worst case, across {rules} rule{}.",
                if *rules == 1 { "" } else { "s" }
            ),
        }
    }

    /// Whether this is something to act on rather than merely to know.
    ///
    /// The Cost line is an engine disclosure and the in-budget Exposure line
    /// is a reassurance; neither is a problem. The editor uses this to decide
    /// which sentences earn the amber it reserves for things that are wrong.
    pub fn is_warning(&self) -> bool {
        match self {
            Diagnostic::Cost { .. } => false,
            Diagnostic::Exposure {
                worst_cents,
                budget_cents,
                ..
            } => worst_cents > budget_cents,
            _ => true,
        }
    }
}

fn money(cents: i64) -> String {
    let whole = cents / 100;
    let frac = (cents % 100).abs();
    if frac == 0 {
        format!("${whole}")
    } else {
        format!("${whole}.{frac:02}")
    }
}

/// The checks that need only the strategy: dead rules, conflicts, and the
/// cost bound.
pub fn check(s: &Strategy, p: &Program) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for (i, rule) in s.rules.iter().enumerate() {
        if let Some(why) = never_holds(rule) {
            out.push(Diagnostic::DeadRule {
                rule: i as u16,
                why,
            });
        }
    }
    out.extend(conflicts(s));
    out.push(Diagnostic::Cost {
        ops: p.cost_bound(),
        rules: p.rule_count(),
    });
    out
}

/// The checks that need to know the table as well as the strategy.
pub fn against_table(
    s: &Strategy,
    rules: &Rules,
    table_min_cents: i64,
    budget_cents: i64,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let (worst, exact) = worst_outlay(s, rules, table_min_cents);
    out.push(Diagnostic::Exposure {
        worst_cents: worst,
        budget_cents,
        exact,
    });
    out.extend(clipping(s, rules, table_min_cents));
    out
}

// ------------------------------------------------------------- dead rules

/// Why this rule's condition can never hold, if it never can.
///
/// Deliberately not a theorem prover. It folds the arithmetic a person
/// actually writes and looks for the two contradictions they actually make —
/// asking one number to be two things, and asking for a window with nothing
/// in it. Anything subtler is left to the Bench's fire counts, which answer
/// the same question by running.
fn never_holds(rule: &Rule) -> Option<&'static str> {
    let guard = rule.guard.as_ref()?;
    if fold(guard) == Some(0) {
        return Some("its condition is never true");
    }
    if contradicts(guard) {
        return Some("its conditions cannot all hold at once");
    }
    None
}

/// Evaluate an expression that mentions nothing the table decides.
fn fold(e: &Expr) -> Option<i64> {
    match e {
        Expr::Const(v) => Some(*v),
        Expr::Var(_) | Expr::Read(_) => None,
        Expr::Not(a) => Some((fold(a)? == 0) as i64),
        Expr::Neg(a) => Some(fold(a)?.saturating_neg()),
        Expr::Bin(op, a, b) => {
            // `and` and `or` decide on one side often enough to be worth
            // asking: `when 0 and <anything>` is dead however complicated
            // the other half is.
            match op {
                BinOp::And if fold(a) == Some(0) || fold(b) == Some(0) => return Some(0),
                BinOp::Or if fold(a).is_some_and(|v| v != 0) || fold(b).is_some_and(|v| v != 0) => {
                    return Some(1)
                }
                _ => {}
            }
            let (x, y) = (fold(a)?, fold(b)?);
            Some(crate::strategy::program::fold_bin(*op, x, y))
        }
    }
}

/// Every `and`-joined term of a condition, flattened.
fn conjuncts<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) {
    match e {
        Expr::Bin(BinOp::And, a, b) => {
            conjuncts(a, out);
            conjuncts(b, out);
        }
        other => out.push(other),
    }
}

/// A comparison of one read against one number, which is the shape almost
/// every real condition takes.
struct Claim<'e> {
    read: &'e Read,
    op: BinOp,
    value: i64,
}

fn claim(e: &Expr) -> Option<Claim<'_>> {
    let Expr::Bin(op, a, b) = e else { return None };
    // Written either way round: `point == 6` and `6 == point` are the same
    // claim, and a person writes both.
    match (a.as_ref(), b.as_ref()) {
        (Expr::Read(r), rhs) => Some(Claim {
            read: r,
            op: *op,
            value: fold(rhs)?,
        }),
        (lhs, Expr::Read(r)) => Some(Claim {
            read: r,
            op: mirror(*op),
            value: fold(lhs)?,
        }),
        _ => None,
    }
}

/// The same comparison with its sides swapped.
const fn mirror(op: BinOp) -> BinOp {
    match op {
        BinOp::Lt => BinOp::Gt,
        BinOp::Le => BinOp::Ge,
        BinOp::Gt => BinOp::Lt,
        BinOp::Ge => BinOp::Le,
        other => other,
    }
}

/// Whether two claims about the same read cannot both be true.
fn contradicts(e: &Expr) -> bool {
    let mut terms = Vec::new();
    conjuncts(e, &mut terms);
    let claims: Vec<Claim<'_>> = terms.iter().filter_map(|t| claim(t)).collect();
    for (i, a) in claims.iter().enumerate() {
        for b in claims.iter().skip(i + 1) {
            if a.read != b.read {
                continue;
            }
            let impossible = match (a.op, b.op) {
                // One number cannot be two numbers.
                (BinOp::Eq, BinOp::Eq) => a.value != b.value,
                (BinOp::Eq, BinOp::Ne) | (BinOp::Ne, BinOp::Eq) => a.value == b.value,
                // A window with nothing in it.
                (BinOp::Lt, BinOp::Gt) => a.value <= b.value + 1,
                (BinOp::Gt, BinOp::Lt) => b.value <= a.value + 1,
                (BinOp::Le, BinOp::Ge) => a.value < b.value,
                (BinOp::Ge, BinOp::Le) => b.value < a.value,
                (BinOp::Lt, BinOp::Ge) => a.value <= b.value,
                (BinOp::Ge, BinOp::Lt) => b.value <= a.value,
                (BinOp::Gt, BinOp::Le) => b.value <= a.value,
                (BinOp::Le, BinOp::Gt) => a.value <= b.value,
                _ => false,
            };
            if impossible {
                return true;
            }
        }
    }
    false
}

// -------------------------------------------------------------- conflicts

/// Rules that act on the same bet at the same trigger.
///
/// Only where both are unconditional: two rules that each carry a condition
/// may well be describing different moments, and calling that a conflict
/// would cry wolf at the most ordinary way there is to write a strategy.
fn conflicts(s: &Strategy) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for (i, a) in s.rules.iter().enumerate() {
        if a.guard.is_some() {
            continue;
        }
        for (j, b) in s.rules.iter().enumerate().skip(i + 1) {
            if b.guard.is_some() || b.trigger != a.trigger {
                continue;
            }
            for bet in touched(a) {
                if touched(b).contains(&bet) {
                    out.push(Diagnostic::Conflict {
                        loser: i as u16,
                        winner: j as u16,
                        bet,
                    });
                }
            }
        }
    }
    out
}

/// The bets a rule's body puts a stake on. `working` and `leave` are not
/// stakes and do not conflict with anything.
fn touched(r: &Rule) -> Vec<BetRef> {
    r.body
        .iter()
        .filter_map(|st| match st {
            Stmt::Bet(b, _) | Stmt::Press(b, _) | Stmt::Regress(b, _) | Stmt::Down(b) => Some(*b),
            Stmt::Working(..) | Stmt::Leave | Stmt::Set(..) => None,
        })
        .collect()
}

// --------------------------------------------------------------- exposure

/// The most this strategy could have on the layout at once, and whether that
/// number is exact.
///
/// One stake per distinct bet, because a bet is one slot however many rules
/// mention it. Computed amounts have no static answer, so they contribute
/// their base and the total is reported as a floor rather than a figure.
fn worst_outlay(s: &Strategy, rules: &Rules, table_min: i64) -> (i64, bool) {
    let mut worst: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut exact = true;
    for rule in &s.rules {
        for st in &rule.body {
            let (bet, amount) = match st {
                Stmt::Bet(b, a) | Stmt::Press(b, a) | Stmt::Regress(b, a) => (*b, a),
                _ => continue,
            };
            let base = base_stake(bet, rules, table_min);
            let cents = match amount {
                crate::strategy::AmountExpr::Cents(e) => match fold(e) {
                    Some(v) => v.max(base),
                    None => {
                        exact = false;
                        base
                    }
                },
                crate::strategy::AmountExpr::Units(e) => match fold(e) {
                    Some(v) => v.saturating_mul(table_min).max(base),
                    None => {
                        exact = false;
                        base
                    }
                },
                // The table answers these, and its answer moves with the
                // progression and the point.
                _ => {
                    exact = false;
                    base
                }
            };
            let key = format!("{bet:?}");
            let slot = worst.entry(key).or_insert(0);
            *slot = (*slot).max(cents);
        }
    }
    (worst.values().sum(), exact)
}

fn base_stake(bet: BetRef, rules: &Rules, table_min: i64) -> i64 {
    match bet {
        BetRef::Place(n) => crate::place_stake_for(table_min, n),
        BetRef::Hardway(_) | BetRef::AnySeven | BetRef::AnyCraps => rules.prop_bet_cents,
        // Odds are sized by the policy behind a flat that may not be up yet;
        // the flat's own stake is what this can honestly claim.
        BetRef::PassOdds | BetRef::DontPassLay | BetRef::ComeOdds(_) | BetRef::DontComeLay(_) => 0,
        _ => table_min,
    }
}

// --------------------------------------------------------------- clipping

/// Pressing systems that will meet the table maximum, and on which step.
///
/// Walked forward the way the losing side of each system walks — which is
/// where a Martingale finds the ceiling, and finding it is the whole reason
/// a table has one.
fn clipping(s: &Strategy, rules: &Rules, table_min: i64) -> Vec<Diagnostic> {
    let table_max = table_min.saturating_mul(rules.table_max_mult.max(1));
    let mut seen: Vec<Progression> = Vec::new();
    let mut out = Vec::new();
    for (i, prog) in s.progressions.iter().enumerate().take(STREAMS) {
        if *prog == Progression::Flat || seen.contains(prog) {
            continue;
        }
        seen.push(*prog);
        let base = stream_base(i, rules, table_min);
        let mut st = ProgState::new(base);
        // Twenty steps is far past where any bankroll survives; a system
        // that has not met the ceiling by then never will.
        for step in 1..=20u32 {
            let lost = st.stake;
            prog.on_loss(&mut st, base, lost);
            if st.stake > table_max {
                out.push(Diagnostic::Clipping {
                    progression: *prog,
                    step,
                    stake_cents: st.stake,
                    table_max_cents: table_max,
                });
                break;
            }
        }
    }
    out
}

/// The base stake of the bets on a stream, which is what its progression
/// counts in.
fn stream_base(stream: usize, rules: &Rules, table_min: i64) -> i64 {
    for n in [4, 5, 6, 8, 9, 10] {
        if stream_of(BetRef::Place(n)) == Some(stream) {
            return crate::place_stake_for(table_min, n);
        }
    }
    for n in [4, 6, 8, 10] {
        if stream_of(BetRef::Hardway(n)) == Some(stream) {
            return rules.prop_bet_cents;
        }
    }
    if stream_of(BetRef::AnySeven) == Some(stream) || stream_of(BetRef::AnyCraps) == Some(stream) {
        return rules.prop_bet_cents;
    }
    table_min
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bets::OddsPolicy;
    use crate::strategy::ast::Trigger;

    /// Whether this trigger is one the session can raise.
    ///
    /// Exhaustive on purpose and consulted only by the test below: adding a
    /// variant makes this stop compiling, which is the moment to wire it up.
    /// `session-start` was spelled, compiled and offered in the editor while
    /// nothing raised it, and every rule written on it fired never.
    const fn trigger_is_dispatched(t: Trigger) -> bool {
        match t {
            Trigger::SessionStart
            | Trigger::ComeOut
            | Trigger::PointEstablished
            | Trigger::PointMade
            | Trigger::SevenOut
            | Trigger::Roll
            | Trigger::Total(_)
            | Trigger::ComePointEstablished(_)
            | Trigger::DontComePointEstablished(_)
            | Trigger::Win(_)
            | Trigger::Loss(_) => true,
        }
    }

    use crate::strategy::{compile, parse};

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

    fn diagnose(src: &str) -> Vec<Diagnostic> {
        let s = parse(src).unwrap_or_else(|e| panic!("{}", e.message()));
        let p = compile(&s).unwrap_or_else(|e| panic!("{}", e.message()));
        check(&s, &p)
    }

    #[test]
    fn a_condition_that_can_never_hold_is_named() {
        for (src, why) in [
            (
                "strategy \"x\" language 1\non roll when 1 == 2:\n bet pass\non roll:\n bet pass\n",
                "never true",
            ),
            (
                "strategy \"x\" language 1\non roll when point == 4 and point == 6:\n bet pass\n\
                 on roll:\n bet pass\n",
                "cannot all hold",
            ),
            (
                "strategy \"x\" language 1\non roll when profit > $100 and profit < $50:\n bet pass\n\
                 on roll:\n bet pass\n",
                "cannot all hold",
            ),
        ] {
            let d = diagnose(src);
            assert!(
                d.iter().any(|x| matches!(x, Diagnostic::DeadRule { rule: 0, why: w } if w.contains(why))),
                "{src}\ngot {:?}",
                d.iter().map(|x| x.message()).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn a_condition_that_can_hold_is_left_alone() {
        // The check must not cry wolf: these are ordinary conditions, and a
        // diagnostic on any of them would teach authors to ignore the strip.
        for src in [
            "strategy \"x\" language 1\non roll when point != 0 and point != 6:\n bet place 6\n",
            "strategy \"x\" language 1\non roll when profit > $100 or profit < -$50:\n bet pass\n",
            "strategy \"x\" language 1\non roll when hits(6) >= 2 and hits(8) >= 2:\n bet pass\n",
            "strategy \"x\" language 1\nvar a = 0\non roll when a == 1 and point == 6:\n bet pass\n",
        ] {
            let d = diagnose(src);
            assert!(
                !d.iter().any(|x| matches!(x, Diagnostic::DeadRule { .. })),
                "{src}\nfalsely dead: {:?}",
                d.iter().map(|x| x.message()).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn two_rules_on_one_bet_at_one_trigger_say_which_wins() {
        let d = diagnose(
            "strategy \"x\" language 1\n\
             on come-out:\n    bet pass $10\n\
             on come-out:\n    bet pass $20\n",
        );
        let c = d
            .iter()
            .find(|x| matches!(x, Diagnostic::Conflict { .. }))
            .expect("the conflict should be named");
        assert!(
            c.message().contains("rule 2 is written later"),
            "{}",
            c.message()
        );

        // A condition on either side means they may be describing different
        // moments, and the check stays quiet.
        let d = diagnose(
            "strategy \"x\" language 1\n\
             on come-out when profit > 0:\n    bet pass $10\n\
             on come-out:\n    bet pass $20\n",
        );
        assert!(!d.iter().any(|x| matches!(x, Diagnostic::Conflict { .. })));
    }

    #[test]
    fn the_cost_bound_is_reported() {
        let d = diagnose("strategy \"x\" language 1\non roll:\n bet pass\n");
        let c = d
            .iter()
            .find(|x| matches!(x, Diagnostic::Cost { .. }))
            .expect("the cost bound is an engine disclosure, always present");
        assert!(c.message().contains("per decision"), "{}", c.message());
        assert!(!c.is_warning(), "cost is a disclosure, not a problem");
    }

    #[test]
    fn exposure_is_measured_against_the_budget() {
        let s = parse(
            "strategy \"x\" language 1\n\
             on roll when point != 0:\n    bet place 6 $60\n    bet place 8 $60\n",
        )
        .unwrap();
        let d = against_table(&s, &rules(), 500, 20_000);
        let e = d
            .iter()
            .find(|x| matches!(x, Diagnostic::Exposure { .. }))
            .expect("exposure is always reported");
        assert!(e.message().contains("$120"), "{}", e.message());
        assert!(!e.is_warning(), "$120 fits a $200 budget: {}", e.message());

        // The same strategy against a budget that cannot cover it.
        let d = against_table(&s, &rules(), 500, 5_000);
        let e = d
            .iter()
            .find(|x| matches!(x, Diagnostic::Exposure { .. }))
            .unwrap();
        assert!(e.is_warning(), "{}", e.message());
        assert!(e.message().contains("budget is $50"), "{}", e.message());
    }

    #[test]
    fn a_computed_amount_makes_the_exposure_a_floor() {
        let s = parse(
            "strategy \"x\" language 1\nvar n = 1\n\
             on roll when point != 0:\n    bet place 6 $6 * n\n",
        )
        .unwrap();
        let d = against_table(&s, &rules(), 500, 100_000);
        let e = d
            .iter()
            .find(|x| matches!(x, Diagnostic::Exposure { .. }))
            .unwrap();
        assert!(e.message().contains("at least"), "{}", e.message());
    }

    #[test]
    fn a_progression_that_will_clip_says_which_step() {
        // A $5 table with a 4x maximum: the Martingale meets it fast.
        let r = Rules {
            table_max_mult: 4,
            place_the_point: false,
            ..rules()
        };
        let s = parse("strategy \"x\" language 1\npress martingale\non come-out:\n bet pass\n")
            .unwrap();
        let d = against_table(&s, &r, 500, 100_000);
        let c = d
            .iter()
            .find(|x| matches!(x, Diagnostic::Clipping { .. }))
            .expect("a Martingale under a 4x maximum clips");
        assert!(c.message().contains("will clip"), "{}", c.message());
        assert!(c.message().contains("Martingale"), "{}", c.message());

        // A flat player never meets a ceiling.
        let s = parse("strategy \"x\" language 1\non come-out:\n bet pass\n").unwrap();
        let d = against_table(&s, &r, 500, 100_000);
        assert!(!d.iter().any(|x| matches!(x, Diagnostic::Clipping { .. })));
    }

    #[test]
    fn every_trigger_the_grammar_spells_is_dispatched() {
        // `session-start` parsed, compiled, and was offered in the editor
        // while nothing raised it. This is the check that would have said so.
        for t in [
            Trigger::SessionStart,
            Trigger::ComeOut,
            Trigger::PointEstablished,
            Trigger::PointMade,
            Trigger::SevenOut,
            Trigger::Roll,
            Trigger::Total(7),
            Trigger::ComePointEstablished(6),
            Trigger::DontComePointEstablished(6),
            Trigger::Win(BetRef::Pass),
            Trigger::Loss(BetRef::Pass),
        ] {
            assert!(
                trigger_is_dispatched(t),
                "{t:?} is spelled but never raised"
            );
        }
    }
}
