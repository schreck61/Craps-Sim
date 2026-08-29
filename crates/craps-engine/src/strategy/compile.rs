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
}

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
            CompileError::NoStreamOfItsOwn(b) => format!(
                "{b:?} has no win/loss record of its own — odds resolve with the flat behind them."
            ),
        }
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
                    if let Some(e) = amount_expr(amount) {
                        check_depth(e)?;
                        emit_expr(e, &mut ops, &mut features, s)?;
                    }
                    ops.push(Op::Bet(*bet, AmountKind::of(amount)));
                    bets += 1;
                }
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

        // Every rule that can fire contributes its bets to the worst case.
        worst_actions += rule
            .body
            .iter()
            .filter(|b| matches!(b, Stmt::Bet(..)))
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

    if bets == 0 {
        return Err(CompileError::NeverBets);
    }
    if worst_actions > MAX_ACTIONS {
        return Err(CompileError::TooManyActions {
            asked: worst_actions,
            limit: MAX_ACTIONS,
        });
    }

    // A program is placement-only when nothing it consults can change
    // without a resolution: no derived history, no memory, and no trigger
    // that asks what the dice showed.
    let placement_only = features.is_empty()
        && s.vars.is_empty()
        && ops
            .iter()
            .all(|op| !matches!(op, Op::Rule { trigger, .. } if !matches!(trigger, TriggerTest::Fired(_))));
    let bets_one_roll = ops.iter().any(|op| {
        matches!(
            op,
            Op::Bet(BetRef::Field | BetRef::AnySeven | BetRef::AnyCraps, _)
        )
    });

    let hash = hash_program(&s.name, &ops, features);
    Ok(Program {
        name: s.name.clone(),
        ops,
        vars: s.vars.len() as u16,
        features,
        placement_only,
        bets_one_roll,
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
            if let Read::Wins(b) | Read::Losses(b) | Read::Streak(b) = r {
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
        AmountExpr::Base | AmountExpr::MaxOdds => None,
    }
}

/// The feature groups a read needs. This is where the mask comes from: what
/// the strategy actually reads, never what someone remembered to declare.
fn feature_of(r: Read) -> FeatureMask {
    match r {
        Read::LastTotal | Read::Roll | Read::RollsThisShooter | Read::Shooter => FeatureMask::DICE,
        Read::Hits(_) | Read::HitsThisShooter(_) => FeatureMask::HITS,
        Read::Wins(_) | Read::Losses(_) | Read::Streak(_) => FeatureMask::STREAKS,
        Read::PeakProfit | Read::Drawdown => FeatureMask::PEAK,
        _ => FeatureMask::NONE,
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
fn hash_program(name: &str, ops: &[Op], features: FeatureMask) -> u64 {
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
    h
}
