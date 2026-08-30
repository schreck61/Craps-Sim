// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! [`Strategy`] to [`Program`]: name resolution, group expansion, the
//! feature mask, the static checks, and the content hash.
//!
//! Everything a strategy can be told about itself is decided here, once,
//! before a single die rolls. A program that reaches the session has already
//! proven it terminates, fits its operand stack, reads only what it
//! declared, and bets something.

use crate::strategy::ast::{AmountExpr, BinOp, Expr, Read, Stmt, Strategy, Trigger};
use crate::strategy::program::{
    fired_bit, AmountKind, Guard, Op, Program, TriggerTest, MAX_ACTIONS, MAX_VARS, STACK_DEPTH,
};
use crate::strategy::view::stream_of;
use crate::strategy::{BetRef, FeatureMask};

/// Why a strategy could not be compiled. Each renders as a sentence in the
/// editor, in the register of the order-ticket validation strip — plain
/// words, never a red outline alone.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CompileError {
    /// No reachable rule places a bet. Run is disabled, and this is why.
    NeverBets,
    /// A memory slot that was never declared.
    UnknownVar(u16),
    /// More memory than the fixed per-session state holds.
    TooManyVars { asked: usize, limit: usize },
    /// An expression nests deeper than the operand stack.
    ExpressionTooDeep { limit: usize },
    /// One decision could propose more bets than the table will hear.
    TooManyActions { asked: usize, limit: usize },
    /// A win/loss trigger or a history read named a bet with no stream of
    /// its own — odds resolve with the flat they back.
    NoStreamOfItsOwn(BetRef),
    /// A come-point trigger named something that is not a box number.
    NotABoxNumber(u8),
    /// A statement named a place or hardway number that does not exist.
    NoSuchBet(BetRef),
    /// More rules than one decision could walk and still be a decision.
    TooManyRules { asked: usize, limit: usize },
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for CompileError {}

impl CompileError {
    /// The sentence the editor shows.
    pub fn message(&self) -> String {
        match self {
            CompileError::NeverBets => {
                "This strategy never places a bet, so there is nothing to simulate.".into()
            }
            CompileError::UnknownVar(i) => {
                format!("Memory slot {i} was used but never declared.")
            }
            CompileError::TooManyVars { asked, limit } => {
                format!("{asked} memory slots were declared; a strategy may hold {limit}.")
            }
            CompileError::ExpressionTooDeep { limit } => {
                format!("An expression nests deeper than {limit} values.")
            }
            CompileError::TooManyActions { asked, limit } => {
                format!("One decision could place {asked} bets; the table hears {limit}.")
            }
            CompileError::NotABoxNumber(n) => {
                format!("{n} is not a box number (4, 5, 6, 8, 9 or 10).")
            }
            CompileError::NoStreamOfItsOwn(b) => format!(
                "{} has no win/loss record of its own — odds resolve with the flat behind them.",
                crate::strategy::bet_name(*b)
            ),
            CompileError::NoSuchBet(b) => {
                format!("There is no such bet as {}.", crate::strategy::bet_name(*b))
            }
            CompileError::TooManyRules { asked, limit } => {
                format!("This strategy has {asked} rules; a strategy may hold {limit}.")
            }
        }
    }
}

/// How many rules one strategy may hold.
///
/// Every rule is walked at every decision point, so the count is a per-roll
/// cost paid across every session of every sweep. Blocks multiply rules by
/// their value lists, and a file can be pasted or hand-written, so the
/// ceiling is stated rather than left to whatever the machine survives.
const MAX_RULES: usize = 512;

/// A bet a statement named has to be a bet the table has.
///
/// The parser refuses these already; this catches a tree built any other
/// way — the rule editor, a test, a future importer — before it reaches an
/// adjudicator that would have to answer for a place 7.
fn check_bet_ref(bet: BetRef) -> Result<(), CompileError> {
    let ok = match bet {
        BetRef::Place(n) | BetRef::ComeOdds(n) | BetRef::DontComeLay(n) => {
            crate::place_index(n).is_some()
        }
        BetRef::Hardway(n) => crate::hard_index(n).is_some(),
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(CompileError::NoSuchBet(bet))
    }
}

/// Compile a strategy, or say why it cannot be.
pub fn compile(s: &Strategy) -> Result<Program, CompileError> {
    if s.vars.len() > MAX_VARS {
        return Err(CompileError::TooManyVars {
            asked: s.vars.len(),
            limit: MAX_VARS,
        });
    }
    if s.rules.len() > MAX_RULES {
        return Err(CompileError::TooManyRules {
            asked: s.rules.len(),
            limit: MAX_RULES,
        });
    }

    let mut ops: Vec<Op> = Vec::new();
    let mut features = FeatureMask::NONE;
    let mut bets = 0usize;
    let mut worst_actions = 0usize;

    for rule in &s.rules {
        let trigger = compile_trigger(rule.trigger, &mut features)?;
        let fused = rule.guard.as_ref().map_or(Guard::Always, fuse_guard);
        let rule_at = ops.len();
        ops.push(Op::Rule {
            trigger,
            guard: fused,
            skip: 0,
        }); // patched once the body is laid out

        // A recognized guard rides in the rule header; anything else still
        // compiles to general code.
        let general_guard = matches!(fused, Guard::General);
        if general_guard {
            let guard = rule.guard.as_ref().expect("General implies a guard");
            check_depth(guard)?;
            emit_expr(guard, &mut ops, &mut features, s)?;
            ops.push(Op::GuardFalse { skip: 0 });
        } else if let Some(guard) = &rule.guard {
            // Fused or not, the guard's reads still decide the mask.
            collect_features(guard, &mut features)?;
        }
        let guard_at = ops.len().saturating_sub(1);

        for stmt in &rule.body {
            match stmt {
                Stmt::Bet(bet, amount) => {
                    check_bet_ref(*bet)?;
                    if let Some(e) = amount_expr(amount) {
                        check_depth(e)?;
                        emit_expr(e, &mut ops, &mut features, s)?;
                    }
                    ops.push(Op::Bet(*bet, AmountKind::of(amount)));
                    bets += 1;
                }
                Stmt::Press(bet, amount) | Stmt::Regress(bet, amount) => {
                    check_bet_ref(*bet)?;
                    if let Some(e) = amount_expr(amount) {
                        check_depth(e)?;
                        emit_expr(e, &mut ops, &mut features, s)?;
                    }
                    let kind = AmountKind::of(amount);
                    ops.push(if matches!(stmt, Stmt::Press(..)) {
                        Op::Press(*bet, kind)
                    } else {
                        Op::Regress(*bet, kind)
                    });
                    // Deliberately not counted as a bet: pressing an empty
                    // slot is always refused, so a strategy whose only action
                    // is `press` can never put a cent at risk — which is
                    // exactly what the never-bets check exists to say.
                }
                Stmt::Down(bet) => {
                    check_bet_ref(*bet)?;
                    ops.push(Op::Down(*bet));
                }
                Stmt::Working(bet, on) => {
                    check_bet_ref(*bet)?;
                    ops.push(Op::Working(*bet, *on));
                }
                Stmt::Leave => ops.push(Op::Leave),
                Stmt::Set(slot, e) => {
                    if *slot as usize >= s.vars.len() {
                        return Err(CompileError::UnknownVar(*slot));
                    }
                    check_depth(e)?;
                    emit_expr(e, &mut ops, &mut features, s)?;
                    ops.push(Op::SetVar(*slot));
                }
            }
        }

        // Every rule that can fire contributes to the worst case, and every
        // statement that reaches the table counts — not just `bet`. A press,
        // a take-down, a working toggle and a leave all take a slot in the
        // proposal buffer, whose overflow is silently dropped; counting only
        // bets meant a decision could discard actions, including the `leave`
        // that was supposed to end the session.
        worst_actions += rule
            .body
            .iter()
            .filter(|b| !matches!(b, Stmt::Set(..)))
            .count();

        let end = ops.len() as u32;
        if let Op::Rule { skip, .. } = &mut ops[rule_at] {
            *skip = end;
        }
        if general_guard {
            if let Op::GuardFalse { skip } = &mut ops[guard_at] {
                *skip = end;
            }
        }
    }

    // A strategy that only takes bets down and leaves never puts anything
    // at risk, which is not a strategy anyone can learn anything from.
    if bets == 0 {
        return Err(CompileError::NeverBets);
    }
    if worst_actions > MAX_ACTIONS {
        return Err(CompileError::TooManyActions {
            asked: worst_actions,
            limit: MAX_ACTIONS,
        });
    }

    let hash = hash_program(&s.name, &ops, features, &s.progressions);
    // Slots start where the strategy says they start. Padded to the declared
    // count so the session can seed memory without consulting the AST.
    let mut var_init = vec![0i64; s.vars.len()];
    for (slot, v) in var_init.iter_mut().enumerate() {
        *v = s.var_init.get(slot).copied().unwrap_or(0);
    }
    Ok(Program {
        name: s.name.clone(),
        ops,
        vars: s.vars.len() as u16,
        var_init,
        features,
        progressions: s.progressions,
        hash,
    })
}

/// Recognize a guard's shape, or fall back to general code.
///
/// The shapes here are not arbitrary: they are the questions every real rule
/// asks. Each collapses six or seven stack operations into one test, which
/// is the difference between a language that costs 5x the hand-written
/// player and one that costs a fraction of that.
fn fuse_guard(e: &Expr) -> Guard {
    // `point != 0` on its own — the commonest single condition in the
    // language, and the one this was missing. Checked ahead of `truthy_read`
    // so it lands on the guard written for it rather than on the generic
    // is-it-nonzero test.
    if is_point_on(e) {
        return Guard::PointOn;
    }
    // `<read> != 0`
    if let Some(r) = truthy_read(e) {
        return Guard::Truthy(r);
    }
    let Expr::Bin(BinOp::And, a, b) = e else {
        return Guard::General;
    };
    if !is_point_on(a) {
        return Guard::General;
    }
    match b.as_ref() {
        // `point != n`
        Expr::Bin(BinOp::Ne, l, r) => match (l.as_ref(), r.as_ref()) {
            (Expr::Read(Read::Point), Expr::Const(n)) if (0..=12).contains(n) => {
                Guard::PointOnExcept(*n as u8)
            }
            _ => Guard::General,
        },
        // `<read> < n`
        Expr::Bin(BinOp::Lt, l, r) => match (l.as_ref(), r.as_ref()) {
            (Expr::Read(read), Expr::Const(n)) => match i32::try_from(*n) {
                Ok(n) => Guard::PointOnAndLess(*read, n),
                Err(_) => Guard::General,
            },
            _ => Guard::General,
        },
        // `up(bet)`
        Expr::Read(Read::Up(bet)) => Guard::PointOnAndUp(*bet),
        _ => Guard::General,
    }
}

fn is_point_on(e: &Expr) -> bool {
    matches!(e, Expr::Bin(BinOp::Ne, l, r)
        if matches!(l.as_ref(), Expr::Read(Read::Point))
            && matches!(r.as_ref(), Expr::Const(0)))
}

/// A bare read used as a condition — `on roll when come_point(6)`.
fn truthy_read(e: &Expr) -> Option<Read> {
    match e {
        Expr::Read(r) => Some(*r),
        _ => None,
    }
}

/// Walk a fused guard for its reads, so the feature mask stays derived from
/// what the strategy reads even when the ops for it were never emitted.
fn collect_features(e: &Expr, features: &mut FeatureMask) -> Result<(), CompileError> {
    match e {
        Expr::Const(_) | Expr::Var(_) => {}
        Expr::Read(r) => {
            if let Read::Wins(b) | Read::Losses(b) | Read::Streak(b) | Read::Paid(b) = r {
                stream_bit(*b)?;
            }
            *features = features.with(feature_of(*r));
        }
        Expr::Bin(_, a, b) => {
            collect_features(a, features)?;
            collect_features(b, features)?;
        }
        Expr::Not(a) | Expr::Neg(a) => collect_features(a, features)?,
    }
    Ok(())
}

fn compile_trigger(t: Trigger, features: &mut FeatureMask) -> Result<TriggerTest, CompileError> {
    if let Some(bit) = fired_bit(t) {
        return Ok(TriggerTest::Fired(bit));
    }
    Ok(match t {
        Trigger::Total(n) => {
            *features = features.with(FeatureMask::DICE);
            TriggerTest::Total(n)
        }
        Trigger::ComePointEstablished(n) => TriggerTest::ComeEstablished(
            crate::place_index(n).ok_or(CompileError::NotABoxNumber(n))? as u8,
        ),
        Trigger::DontComePointEstablished(n) => TriggerTest::DontComeEstablished(
            crate::place_index(n).ok_or(CompileError::NotABoxNumber(n))? as u8,
        ),
        Trigger::Win(b) => {
            *features = features.with(FeatureMask::STREAKS);
            TriggerTest::Win(stream_bit(b)?)
        }
        Trigger::Loss(b) => {
            *features = features.with(FeatureMask::STREAKS);
            TriggerTest::Loss(stream_bit(b)?)
        }
        _ => unreachable!("fired_bit covers the coarse triggers"),
    })
}

fn stream_bit(b: BetRef) -> Result<u8, CompileError> {
    stream_of(b)
        .map(|i| i as u8)
        .ok_or(CompileError::NoStreamOfItsOwn(b))
}

fn amount_expr(a: &AmountExpr) -> Option<&Expr> {
    match a {
        AmountExpr::Units(e) | AmountExpr::Cents(e) => Some(e),
        AmountExpr::Base | AmountExpr::Pressed | AmountExpr::MaxOdds => None,
    }
}

/// The feature groups a read needs. This is where the mask comes from: what
/// the strategy actually reads, never what someone remembered to declare.
fn feature_of(r: Read) -> FeatureMask {
    match r {
        Read::LastTotal | Read::Roll | Read::RollsThisShooter | Read::Shooter => FeatureMask::DICE,
        Read::Hits(_) | Read::HitsThisShooter(_) => FeatureMask::HITS,
        Read::Wins(_) | Read::Losses(_) | Read::Streak(_) | Read::Paid(_) => FeatureMask::STREAKS,
        Read::PeakProfit | Read::Drawdown => FeatureMask::PEAK,
        // Exhaustive rather than wildcarded: a read added later must be
        // asked which accumulator it needs, not quietly answered "none" and
        // then report a stale zero to a strategy that trusted it.
        Read::Point
        | Read::ComeOut
        | Read::Cash
        | Read::Wealth
        | Read::Profit
        | Read::Handle
        | Read::BuyIn
        | Read::TableMin
        | Read::TableMax
        | Read::Stake(_)
        | Read::Up(_)
        | Read::Working(_)
        | Read::LiveCome
        | Read::LiveDontCome
        | Read::ComePoint(_)
        | Read::DontComePoint(_)
        | Read::OnTableFace => FeatureMask::NONE,
    }
}

fn emit_expr(
    e: &Expr,
    ops: &mut Vec<Op>,
    features: &mut FeatureMask,
    s: &Strategy,
) -> Result<(), CompileError> {
    match e {
        Expr::Const(v) => ops.push(Op::PushConst(*v)),
        Expr::Var(i) => {
            if *i as usize >= s.vars.len() {
                return Err(CompileError::UnknownVar(*i));
            }
            ops.push(Op::PushVar(*i));
        }
        Expr::Read(r) => {
            if let Read::Wins(b) | Read::Losses(b) | Read::Streak(b) = r {
                stream_bit(*b)?;
            }
            *features = features.with(feature_of(*r));
            ops.push(Op::PushRead(*r));
        }
        Expr::Bin(op, a, b) => {
            emit_expr(a, ops, features, s)?;
            emit_expr(b, ops, features, s)?;
            ops.push(Op::Bin(*op));
        }
        Expr::Not(a) => {
            emit_expr(a, ops, features, s)?;
            ops.push(Op::Not);
        }
        Expr::Neg(a) => {
            emit_expr(a, ops, features, s)?;
            ops.push(Op::Neg);
        }
    }
    Ok(())
}

/// The operand depth an expression needs, refused if it would not fit.
fn check_depth(e: &Expr) -> Result<(), CompileError> {
    fn depth(e: &Expr) -> usize {
        match e {
            Expr::Const(_) | Expr::Var(_) | Expr::Read(_) => 1,
            Expr::Not(a) | Expr::Neg(a) => depth(a),
            // The left operand stays on the stack while the right is built.
            Expr::Bin(_, a, b) => depth(a).max(depth(b) + 1),
        }
    }
    if depth(e) > STACK_DEPTH {
        return Err(CompileError::ExpressionTooDeep { limit: STACK_DEPTH });
    }
    Ok(())
}

/// FNV-1a over the compiled form, hand-rolled so it is stable across
/// platforms and releases — the same reason
/// [`SimConfig::fingerprint`](../../../craps-app/src/config.rs) is.
fn hash_program(
    name: &str,
    ops: &[Op],
    features: FeatureMask,
    progressions: &[crate::bets::Progression],
) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    let byte = |b: u8, h: &mut u64| {
        *h ^= b as u64;
        *h = h.wrapping_mul(PRIME);
    };
    for b in name.as_bytes() {
        byte(*b, &mut h);
    }
    for op in ops {
        // Debug formatting is stable for these plain data enums and keeps
        // the hash honest without hand-writing a serializer per variant.
        for b in format!("{op:?}").as_bytes() {
            byte(*b, &mut h);
        }
    }
    for b in format!("{features:?}").as_bytes() {
        byte(*b, &mut h);
    }
    for p in progressions {
        for b in format!("{p:?}").as_bytes() {
            byte(*b, &mut h);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::parse;

    /// A strategy's identity is pinned to a value, not just to itself.
    ///
    /// The hash is taken over `Debug` formatting of the compiled ops, which
    /// is stable for plain data enums but is coupled to how their variants
    /// and fields are *spelled*. A rename that changes nothing about what a
    /// strategy does would re-key every saved one, turning every Scenario
    /// Sentence in the wild STALE against a strategy that had not changed —
    /// silently, since nothing compared the hash to a known answer. This is
    /// that comparison. If it fails after a refactor, the refactor changed
    /// what a strategy *is*, and every saved sentence needs to know.
    #[test]
    fn the_program_identity_is_pinned_to_a_value() {
        let s = parse(
            "strategy \"pinned\" language 1\n\
             on come-out:\n    bet pass base\n\
             on roll when point != 0:\n    bet odds on pass max\n",
        )
        .unwrap_or_else(|e| panic!("{}", e.message()));
        let p = compile(&s).unwrap();
        assert_eq!(
            p.hash, 0xdc8a_8fff_8cac_ec35,
            "the compiled identity of a fixed strategy changed; \
             every saved sentence referring to one now reads STALE"
        );
    }

    /// Two strategies that differ only in a name are different strategies,
    /// and two that differ only in a rule are too. This is the whole job of
    /// the hash: a sentence can never silently run the wrong player.
    #[test]
    fn the_identity_moves_when_the_strategy_does() {
        let one =
            compile(&parse("strategy \"a\" language 1\non come-out:\n bet pass base\n").unwrap())
                .unwrap();
        let renamed =
            compile(&parse("strategy \"b\" language 1\non come-out:\n bet pass base\n").unwrap())
                .unwrap();
        let rewritten = compile(
            &parse("strategy \"a\" language 1\non come-out:\n bet dont pass base\n").unwrap(),
        )
        .unwrap();
        let pressed = compile(
            &parse("strategy \"a\" language 1\npress martingale\non come-out:\n bet pass base\n")
                .unwrap(),
        )
        .unwrap();
        assert_ne!(one.hash, renamed.hash, "a different name");
        assert_ne!(one.hash, rewritten.hash, "a different bet");
        assert_ne!(one.hash, pressed.hash, "a different pressing system");
    }
}
