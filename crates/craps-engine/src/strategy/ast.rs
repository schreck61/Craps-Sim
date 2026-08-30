// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The one abstract syntax tree.
//!
//! Everything a strategy is passes through this shape: the rule-table editor
//! builds it, the text form parses to it and renders from it, and the
//! compiler turns it into the program the session executes. Principle 2 of
//! [`STRATEGY_DSL.md`](../../../../docs/STRATEGY_DSL.md) — one AST, two
//! editors — is a claim about this module being the only representation
//! there is.
//!
//! The tree is deliberately small. There is no loop that is not bounded at
//! compile time, no recursion, no way to name something that was not
//! declared up front: a strategy is total and cost-bounded before it runs,
//! which is what makes it safe to execute unreviewed across every core.

use crate::bets::Progression;
use crate::strategy::view::STREAMS;
use crate::strategy::BetRef;

/// A slot in a strategy's memory, resolved at compile time. There is no
/// dynamic namespace; the count is fixed when the program is built.
pub type VarId = u16;

/// When a rule is considered.
///
/// The coarse triggers are properties of the moment; `Total`, `Win`, and
/// `Loss` are properties of what just happened. Both are decided once per
/// decision point and neither is a separate hook into the session — a real
/// player acts between rolls, and so does a rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trigger {
    /// Before the first roll, once.
    SessionStart,
    /// Any decision point with no point established.
    ComeOut,
    /// The roll just resolved established a point.
    PointEstablished,
    /// The shooter just made the point.
    PointMade,
    /// The shooter just sevened out and the dice pass.
    SevenOut,
    /// Every decision point, unconditionally.
    Roll,
    /// The roll just resolved totaled this.
    Total(u8),
    /// A come point was established on this number — a come flat reached
    /// it. Distinct from [`Trigger::PointEstablished`], which is the
    /// table's own point.
    ComePointEstablished(u8),
    /// A don't come point was established on this number.
    DontComePointEstablished(u8),
    /// This stream just won.
    Win(BetRef),
    /// This stream just lost.
    Loss(BetRef),
}

/// Something a strategy can read. Every variant is an `i64` — cents for
/// money, plain counts for everything else, booleans as 0 or 1. There is no
/// floating point in the language.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Read {
    Point,
    ComeOut,
    LastTotal,
    Roll,
    RollsThisShooter,
    Shooter,
    Cash,
    Wealth,
    Profit,
    PeakProfit,
    Drawdown,
    Handle,
    Stake(BetRef),
    Up(BetRef),
    LiveCome,
    LiveDontCome,
    /// The come flat established on this number, 0 if none.
    ComePoint(u8),
    /// The don't come flat established on this number, 0 if none.
    DontComePoint(u8),
    OnTableFace,
    Hits(u8),
    HitsThisShooter(u8),
    Wins(BetRef),
    Losses(BetRef),
    Streak(BetRef),
    /// What this stream was paid on the roll just resolved.
    Paid(BetRef),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

/// An integer expression. Division truncates toward zero and division by
/// zero yields zero rather than trapping — a strategy is data, and data must
/// not be able to kill a worker thread mid-sweep.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Expr {
    Const(i64),
    Var(VarId),
    Read(Read),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
}

impl Expr {
    pub fn bin(op: BinOp, a: Expr, b: Expr) -> Expr {
        Expr::Bin(op, Box::new(a), Box::new(b))
    }
}

/// How much to put up, as an expression the strategy computes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AmountExpr {
    /// The table's own base stake for this bet — the table minimum rounded
    /// to the bet's payout unit, or the configured prop stake. What every
    /// flat player bets without saying so.
    Base,
    /// Whatever this stream's pressing system calls for. The same as
    /// [`AmountExpr::Base`] under a flat progression.
    Pressed,
    /// This many table minimums.
    Units(Expr),
    /// Exactly this many cents.
    Cents(Expr),
    /// The most the odds policy allows behind this point.
    MaxOdds,
}

/// One thing a rule does.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Stmt {
    /// Put the bet up if it is not up; top odds up to the target.
    Bet(BetRef, AmountExpr),
    /// Raise a working bet to this stake. A press that would lower the bet
    /// does nothing, because `press` should mean what it says; use
    /// [`Stmt::Regress`] to come down.
    Press(BetRef, AmountExpr),
    /// Lower a working bet to this stake, taking the difference back.
    Regress(BetRef, AmountExpr),
    /// Take the bet down entirely.
    Down(BetRef),
    /// Turn a place bet or hardway off, or back on.
    Working(BetRef, bool),
    /// Leave the table.
    Leave,
    /// Write a value into strategy memory.
    Set(VarId, Expr),
}

/// `on <trigger> [when <guard>]: <body>`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rule {
    pub trigger: Trigger,
    pub guard: Option<Expr>,
    pub body: Vec<Stmt>,
}

impl Rule {
    pub fn new(trigger: Trigger, body: Vec<Stmt>) -> Self {
        Self {
            trigger,
            guard: None,
            body,
        }
    }

    pub fn when(mut self, guard: Expr) -> Self {
        self.guard = Some(guard);
        self
    }
}

/// A whole strategy, before compilation.
///
/// Rules are evaluated top to bottom at each decision point; every matching
/// rule contributes its actions, and the table applies them in that order.
/// Two rules touching the same bet are resolved by order — last write wins —
/// which the Bench shows firing rather than leaving to be debugged.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Strategy {
    pub name: String,
    /// Declared memory slots, by name. Index is the [`VarId`].
    pub vars: Vec<String>,
    pub rules: Vec<Rule>,
    /// The pressing system on each bet stream, keyed like the session's
    /// streams. This is a declaration rather than a rule, and the reason is
    /// in the game: when a place bet hits, the dealer pays and asks whether
    /// to press it *then*, before the next roll. A progression is a standing
    /// answer to that question, applied where the bet resolves. Rules act at
    /// the decision point between rolls, which is the right place for
    /// "take everything down after two hits" and the wrong place for "press
    /// this winner out of its own winnings".
    pub progressions: [Progression; STREAMS],
}

impl Strategy {
    pub fn new(name: impl Into<String>, rules: Vec<Rule>) -> Self {
        Self {
            name: name.into(),
            vars: Vec::new(),
            rules,
            progressions: [Progression::Flat; STREAMS],
        }
    }

    /// Press every stream the same way — what a checkbox player means by
    /// choosing one progression.
    pub fn pressing(mut self, p: Progression) -> Self {
        self.progressions = [p; STREAMS];
        self
    }

    /// Press one stream differently from the rest.
    pub fn pressing_stream(mut self, stream: usize, p: Progression) -> Self {
        if stream < STREAMS {
            self.progressions[stream] = p;
        }
        self
    }
}

/// Sugar the compiler expands: a group of bets named as one.
///
/// Groups exist because "place the inside numbers" is how the bet is spoken,
/// and a language that made the user write four rules for it would be
/// describing the engine rather than the game.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Inside,
    Outside,
    AllPlace,
    AllHardways,
}

impl Group {
    pub fn members(self) -> &'static [BetRef] {
        const INSIDE: [BetRef; 4] = [
            BetRef::Place(5),
            BetRef::Place(6),
            BetRef::Place(8),
            BetRef::Place(9),
        ];
        const OUTSIDE: [BetRef; 2] = [BetRef::Place(4), BetRef::Place(10)];
        const ALL_PLACE: [BetRef; 6] = [
            BetRef::Place(4),
            BetRef::Place(5),
            BetRef::Place(6),
            BetRef::Place(8),
            BetRef::Place(9),
            BetRef::Place(10),
        ];
        const ALL_HARD: [BetRef; 4] = [
            BetRef::Hardway(4),
            BetRef::Hardway(6),
            BetRef::Hardway(8),
            BetRef::Hardway(10),
        ];
        match self {
            Group::Inside => &INSIDE,
            Group::Outside => &OUTSIDE,
            Group::AllPlace => &ALL_PLACE,
            Group::AllHardways => &ALL_HARD,
        }
    }
}
