// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The executed form of a strategy, and the machine that runs it.
//!
//! A [`Program`] is what a [`Strategy`](crate::strategy::ast::Strategy)
//! becomes once names are resolved to slots, groups are expanded, and
//! amounts are laid out as a flat instruction stream. It is built once per
//! run and shared immutably across the rayon pool; the only per-session
//! state is a [`StratState`], which is a fixed-size array on the stack.
//!
//! The machine is a stack machine over `i64` with a fixed operand depth. It
//! does not allocate, does not hash, does not branch on anything it has to
//! look up, and cannot fail to terminate: the only backward jump the
//! compiler emits is a bounded `for each`, unrolled before it gets here. A
//! program that ran away would take a worker thread with it, so no program
//! can run away.

use crate::game::Session;
use crate::strategy::ast::{AmountExpr, BinOp, Read, Trigger, WorkingWhen};
use crate::strategy::view::{Features, Stakes, TableView, STREAMS};
use crate::strategy::{Action, Amount, BetRef, FeatureMask};
use crate::trace::RollObserver;

/// How deep an expression may nest before the compiler refuses it. Real
/// rules nest three or four deep; this is a backstop, not a budget.
pub(crate) const STACK_DEPTH: usize = 32;

/// How many actions one decision point may propose. A strategy that wants
/// more than this is asking to cover the layout twice over.
pub(crate) const MAX_ACTIONS: usize = 48;

/// Memory slots a strategy may declare.
pub(crate) const MAX_VARS: usize = 32;

/// The instruction stream is walked once per rule per decision, so its size
/// is cache pressure in the hot path.
///
/// A const assertion rather than a test: a size regression on an unusual
/// target, or in a build nobody runs tests on, should stop the compiler
/// rather than wait for CI. The test below stays, because it prints the
/// number when it fails and this does not.
const _: () = assert!(std::mem::size_of::<Op>() <= 16);

/// One instruction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    /// Begin a rule: unless its trigger fired and its guard holds, jump to
    /// `skip`.
    Rule {
        trigger: TriggerTest,
        guard: Guard,
        skip: u32,
    },
    /// Pop a condition; if it is false, jump to `skip`. Emitted only for
    /// guards the compiler could not fuse into [`Guard`].
    GuardFalse {
        skip: u32,
    },
    PushConst(i64),
    PushVar(u16),
    PushRead(Read),
    Bin(BinOp),
    Not,
    Neg,
    /// Pop a value and store it.
    SetVar(u16),
    /// Pop the amount (unless the kind needs none) and propose the bet.
    Bet(BetRef, AmountKind),
    /// Pop the amount and propose a new stake, raising only.
    Press(BetRef, AmountKind),
    /// Pop the amount and propose a new stake, lowering only.
    Regress(BetRef, AmountKind),
    Down(BetRef),
    Working(BetRef, bool, WorkingWhen),
    Leave,
}

/// The half of an [`AmountExpr`] that survives compilation: the shape, with
/// its expression already emitted as the ops that precede this one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AmountKind {
    Base,
    Pressed,
    Units,
    Cents,
    MaxOdds,
}

impl AmountKind {
    /// Whether the amount was pushed onto the stack by preceding ops.
    #[inline]
    const fn takes_operand(self) -> bool {
        matches!(self, AmountKind::Units | AmountKind::Cents)
    }

    pub(crate) fn of(e: &AmountExpr) -> Self {
        match e {
            AmountExpr::Base => AmountKind::Base,
            AmountExpr::Pressed => AmountKind::Pressed,
            AmountExpr::Units(_) => AmountKind::Units,
            AmountExpr::Cents(_) => AmountKind::Cents,
            AmountExpr::MaxOdds => AmountKind::MaxOdds,
        }
    }
}

/// A compiled trigger. The coarse ones are a bit in the decision's fired
/// set; the fine ones carry the datum they compare against, so that
/// `on total(7)` costs one comparison rather than a hook of its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TriggerTest {
    /// Matches when any of these bits are in the decision's fired set.
    Fired(u8),
    Total(u8),
    Win(u8),
    Loss(u8),
    /// A come point established on the place index carried here.
    ComeEstablished(u8),
    DontComeEstablished(u8),
}

/// A rule's condition, fused into the rule header when it matches a shape
/// the compiler recognizes.
///
/// Guards are where an interpreter spends its life. Almost every real rule
/// asks one of a handful of questions — is a point on, is this bet already
/// working, do I have a come point on the 6 — and spelling those as six or
/// seven stack operations apiece cost 5x against the hand-written player
/// when it was measured. Recognized shapes collapse to a single test;
/// anything else still compiles to general code and pays for itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Guard {
    /// No condition.
    Always,
    /// `point != 0`.
    PointOn,
    /// `point != 0 && point != n` — a place bet, which never covers the
    /// point number.
    PointOnExcept(u8),
    /// `point != 0 && up(bet)` — odds, which need a flat behind them.
    PointOnAndUp(BetRef),
    /// `point != 0 && <read> < n`. The bound is an `i32` because the
    /// counts this shape compares — live come bets, hits on a number — are
    /// small, and because widening it to `i64` grows every instruction in
    /// the stream by eight bytes.
    PointOnAndLess(Read, i32),
    /// `<read> != 0`.
    Truthy(Read),
    /// Evaluate the ops that follow, ending in [`Op::GuardFalse`].
    General,
}

/// Bits for the coarse triggers, as a set computed once per decision.
pub(crate) mod fired {
    pub const SESSION_START: u8 = 1 << 0;
    pub const COME_OUT: u8 = 1 << 1;
    pub const POINT_ESTABLISHED: u8 = 1 << 2;
    pub const POINT_MADE: u8 = 1 << 3;
    pub const SEVEN_OUT: u8 = 1 << 4;
    pub const ROLL: u8 = 1 << 5;
}

/// What happened since the last decision point — the input a trigger tests
/// against. Built by the session, never by a strategy.
#[derive(Clone, Copy, Debug, Default)]
pub struct Decision {
    // The won/lost sets below are `u32` bitsets over the streams.
    pub fired: u8,
    pub total: u8,
    /// One bit per stream, keyed like [`STREAMS`].
    pub won: u32,
    pub lost: u32,
    /// One bit per place index, for come points established this roll.
    pub come_established: u8,
    pub dont_come_established: u8,
}

const _: () = assert!(STREAMS <= 32);

/// A compiled strategy: immutable, shareable, and the same bytes on every
/// thread.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Program {
    pub name: String,
    pub(crate) ops: Vec<Op>,
    pub(crate) vars: u16,
    /// What each memory slot holds at session start.
    pub(crate) var_init: Vec<i64>,
    /// Which derived history this program reads, derived from the reads it
    /// actually makes rather than declared by hand.
    pub features: FeatureMask,
    /// The pressing system on each bet stream.
    pub(crate) progressions: [crate::bets::Progression; STREAMS],
    /// FNV-1a over the compiled form — the identity a Scenario Sentence
    /// carries so a strategy can never be silently swapped for another of
    /// the same name.
    pub hash: u64,
}

impl Program {
    /// The cheapest stake any bet in this program could put up — what
    /// "the bankroll can no longer sustain this strategy" means for a
    /// compiled player, computed from the bets it actually contains rather
    /// than from a checkbox set it does not have.
    ///
    /// Odds are excluded: they need a flat behind them, so a bankroll that
    /// can fund no flat is ruined whatever the odds policy says.
    pub fn cheapest_stake(&self, rules: &crate::Rules, table_min_cents: i64) -> i64 {
        let mut cheapest = i64::MAX;
        for op in &self.ops {
            let Op::Bet(bet, _) = op else { continue };
            let base = match bet {
                BetRef::Pass
                | BetRef::DontPass
                | BetRef::Come
                | BetRef::DontCome
                | BetRef::Field => table_min_cents,
                BetRef::Place(n) => crate::place_stake_for(table_min_cents, *n),
                BetRef::Hardway(_) | BetRef::AnySeven | BetRef::AnyCraps => rules.prop_bet_cents,
                BetRef::PassOdds
                | BetRef::DontPassLay
                | BetRef::ComeOdds(_)
                | BetRef::DontComeLay(_) => continue,
            };
            cheapest = cheapest.min(base);
        }
        cheapest
    }

    /// How many rules this program holds — the length the Bench's fire
    /// counts must cover, so that a rule which never fired is reported as
    /// zero rather than missing.
    pub fn rule_count(&self) -> usize {
        self.ops
            .iter()
            .filter(|op| matches!(op, Op::Rule { .. }))
            .count()
    }

    /// Instructions executed per decision in the worst case — every rule
    /// firing, every guard passing. The static cost bound Principle 3 buys.
    pub fn cost_bound(&self) -> usize {
        self.ops.len()
    }
}

/// Per-session strategy memory. Fixed size, stack-allocated, zeroed at
/// session start; there is nothing here to allocate or free.
#[derive(Clone, Copy, Debug)]
pub struct StratState {
    pub(crate) vars: [i64; MAX_VARS],
}

impl Default for StratState {
    fn default() -> Self {
        Self {
            vars: [0; MAX_VARS],
        }
    }
}

impl StratState {
    /// Seed memory from a program's declared initial values.
    ///
    /// Slots beyond what the program declares stay at zero, and a program
    /// declaring more than there is room for is impossible: the compiler
    /// refuses past [`MAX_VARS`] before this can be reached.
    pub(crate) fn seed(&mut self, init: &[i64]) {
        for (slot, v) in init.iter().enumerate().take(MAX_VARS) {
            self.vars[slot] = *v;
        }
    }
}

/// One thing a decision decided to ask the table for. Buffered rather than
/// applied as it is produced, so that every rule in a decision reads the
/// same state.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Proposal {
    Bet(BetRef, Amount),
    /// A stake change, with the direction the rule asked for: a `press` that
    /// would lower the bet is not a press, and does nothing.
    Stake(BetRef, Amount, Direction),
    Down(BetRef),
    Working(BetRef, bool, WorkingWhen),
    Leave,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Direction {
    Up,
    Down,
}

/// The proposal buffer: what a decision produced, before the table sees it.
///
/// It lives on the session rather than on the stack of each decision.
/// Clearing a length is free; initializing forty-eight slots on every roll
/// of every session is not, and that difference was worth 20% of the
/// interpreter when it was measured.
#[derive(Clone, Copy, Debug)]
pub struct Proposals {
    items: [Proposal; MAX_ACTIONS],
    /// Which rule asked for each proposal, in a parallel array rather than
    /// inside `Proposal` — carrying it in the enum would grow every slot to
    /// the next alignment step and cost 376 bytes on a struct built once
    /// per session, for a field only the Bench ever reads.
    rules: [u16; MAX_ACTIONS],
    len: usize,
}

impl Default for Proposals {
    fn default() -> Self {
        Self {
            items: [Proposal::Bet(BetRef::Pass, Amount::Base); MAX_ACTIONS],
            rules: [0; MAX_ACTIONS],
            len: 0,
        }
    }
}

impl Proposals {
    #[inline]
    fn push(&mut self, p: Proposal, rule: u16) {
        if self.len < MAX_ACTIONS {
            self.items[self.len] = p;
            self.rules[self.len] = rule;
            self.len += 1;
        }
    }
}

impl<O: RollObserver, F: Features> Session<'_, O, F> {
    /// Run one decision point: evaluate every rule against a single snapshot
    /// of the table, then hand the table what they proposed.
    ///
    /// Every rule in one decision sees the same state. That is deliberate:
    /// rules are a description of what the player wants, not a sequence of
    /// table actions, and a rule whose condition silently depended on an
    /// earlier rule's bet in the same breath would be unreadable. The
    /// *table* still applies the proposals in order, so bankroll runs out
    /// in the order the rules were written.
    pub(crate) fn run_program(&mut self, p: &Program, d: Decision) {
        self.proposals.len = 0;
        {
            let view = TableView {
                point: self.point,
                cash: self.cash,
                start_cash: self.start_cash,
                table_min: self.min,
                table_max: self.table_max,
                rules: self.rules,
                handle: self.resolved_wagered_cents,
                stakes: Stakes {
                    pass: self.pass,
                    pass_odds: self.pass_odds,
                    dont: self.dont,
                    dont_lay: self.dont_lay,
                    come_flat: self.come_flat,
                    dc_flat: self.dc_flat,
                    place: &self.place,
                    hard: &self.hard,
                    place_working: &self.place_working,
                    hard_working: &self.hard_working,
                    place_comeout: &self.place_comeout,
                    hard_comeout: &self.hard_comeout,
                    come_points: &self.come_points,
                    come_odds: &self.come_odds,
                    dc_points: &self.dc_points,
                    dc_lay: &self.dc_lay,
                    field: self.field_bet,
                    any7: self.any7_bet,
                    anycraps: self.anycraps_bet,
                },
                hist: &self.hist,
            };
            evaluate(
                &view,
                p,
                &mut self.strat,
                d,
                &mut self.proposals,
                &mut self.obs,
            );
        }
        for i in 0..self.proposals.len {
            if O::WANTS_RULES {
                let rule = self.proposals.rules[i];
                self.obs.acting_for(Some(rule));
            }
            let action = match self.proposals.items[i] {
                Proposal::Bet(b, a) => Action::Bet(b, a),
                Proposal::Down(b) => Action::Down(b),
                Proposal::Working(b, on, when) => Action::Working(b, on, when),
                Proposal::Leave => Action::Leave,
                Proposal::Stake(b, a, dir) => {
                    // `press` and `regress` each mean one direction. The
                    // table is told the target; the direction is checked
                    // here, against the layout as it stands after the
                    // proposals before this one.
                    let cur = self.current_stake_of(b);
                    let target = self.resolve_target(b, a);
                    let wrong_way = match dir {
                        Direction::Up => target <= cur,
                        Direction::Down => target >= cur,
                    };
                    if wrong_way {
                        // Refused, and said so. This was the one refusal in
                        // the language that emitted nothing, which left a
                        // rule visibly firing in the Bench beside a layout
                        // that never moved and no reason given — exactly the
                        // silence Principle 4 exists to forbid.
                        let _ = self.reject(b, crate::strategy::RejectReason::WrongDirection);
                        continue;
                    }
                    Action::SetStake(b, a)
                }
            };
            let _ = self.apply(action);
        }
        if O::WANTS_RULES {
            // The decision is over; whatever the dice do next is the
            // table's own doing and belongs to no rule.
            self.obs.acting_for(None);
        }
    }
}

/// The machine. Split from the session so it borrows the view immutably and
/// can be tested against a view alone.
fn evaluate<O: RollObserver>(
    view: &TableView<'_>,
    p: &Program,
    st: &mut StratState,
    d: Decision,
    out: &mut Proposals,
    obs: &mut O,
) {
    let mut stack = [0i64; STACK_DEPTH];
    let mut sp = 0usize;
    let mut pc = 0usize;
    // Rules are laid out in order and each begins with exactly one
    // `Op::Rule`, so counting them as they are walked identifies them
    // without carrying an index in the instruction — which would have
    // pushed `Op` past sixteen bytes.
    let mut rule = u16::MAX;

    macro_rules! pop {
        () => {{
            sp = sp.saturating_sub(1);
            stack[sp]
        }};
    }
    macro_rules! push {
        ($v:expr) => {{
            if sp < STACK_DEPTH {
                stack[sp] = $v;
                sp += 1;
            }
        }};
    }

    while pc < p.ops.len() {
        match p.ops[pc] {
            Op::Rule {
                trigger,
                guard,
                skip,
            } => {
                rule = rule.wrapping_add(1);
                if !trigger_matches(trigger, d) || !guard_holds(guard, view) {
                    pc = skip as usize;
                    continue;
                }
                if O::WANTS_RULES && !matches!(guard, Guard::General) {
                    // A general guard has not been evaluated yet — its ops
                    // come next — so the rule is not reported as fired
                    // until `GuardFalse` declines to jump. Reporting here
                    // would have called every conditional rule fired, which
                    // is precisely the lie the Bench exists to prevent.
                    obs.rule_fired(rule);
                }
                sp = 0; // each rule starts with a clean stack
            }
            Op::GuardFalse { skip } => {
                if pop!() == 0 {
                    pc = skip as usize;
                    continue;
                }
                if O::WANTS_RULES {
                    obs.rule_fired(rule);
                }
            }
            Op::PushConst(v) => push!(v),
            Op::PushVar(i) => push!(st.vars[i as usize]),
            Op::PushRead(r) => push!(read(view, r)),
            Op::Bin(op) => {
                let b = pop!();
                let a = pop!();
                push!(apply_bin(op, a, b));
            }
            Op::Not => {
                let a = pop!();
                push!((a == 0) as i64);
            }
            Op::Neg => {
                let a = pop!();
                push!(a.saturating_neg());
            }
            Op::SetVar(i) => {
                let v = pop!();
                st.vars[i as usize] = v;
            }
            Op::Down(bet) => out.push(Proposal::Down(bet), rule),
            Op::Working(bet, on, when) => out.push(Proposal::Working(bet, on, when), rule),
            Op::Leave => out.push(Proposal::Leave, rule),
            Op::Press(bet, kind) | Op::Regress(bet, kind) => {
                let value = if kind.takes_operand() { pop!() } else { 0 };
                let dir = if matches!(p.ops[pc], Op::Press(..)) {
                    Direction::Up
                } else {
                    Direction::Down
                };
                out.push(Proposal::Stake(bet, amount_of(kind, value), dir), rule);
            }
            Op::Bet(bet, kind) => {
                // The amount is popped first either way. Skipping the pop on
                // the already-up path would leave its operand on the stack
                // for the rest of the rule, and a rule with enough of them
                // would silently start dropping later values against the
                // depth clamp.
                let value = if kind.takes_operand() { pop!() } else { 0 };
                // A flat that is already working is a no-op at the table, so
                // there is no reason to walk it all the way there. Odds are
                // exempt: they top up toward a target that moves when a come
                // flat stacks.
                if !matches!(kind, AmountKind::MaxOdds) && view.slot_stake(bet) != 0 {
                    pc += 1;
                    continue;
                }
                out.push(Proposal::Bet(bet, amount_of(kind, value)), rule);
            }
        }
        pc += 1;
    }
}

#[inline]
fn amount_of(kind: AmountKind, value: i64) -> Amount {
    match kind {
        AmountKind::Base => Amount::Base,
        AmountKind::Pressed => Amount::Pressed,
        AmountKind::Units => Amount::Units(value),
        AmountKind::Cents => Amount::Cents(value),
        AmountKind::MaxOdds => Amount::MaxOdds,
    }
}

#[inline]
fn trigger_matches(t: TriggerTest, d: Decision) -> bool {
    match t {
        TriggerTest::Fired(bits) => d.fired & bits != 0,
        TriggerTest::Total(n) => d.total == n,
        TriggerTest::Win(stream) => d.won & (1 << stream) != 0,
        TriggerTest::Loss(stream) => d.lost & (1 << stream) != 0,
        TriggerTest::ComeEstablished(i) => d.come_established & (1 << i) != 0,
        TriggerTest::DontComeEstablished(i) => d.dont_come_established & (1 << i) != 0,
    }
}

#[inline]
fn guard_holds(g: Guard, v: &TableView<'_>) -> bool {
    match g {
        Guard::Always | Guard::General => true,
        Guard::PointOn => !v.come_out(),
        Guard::PointOnExcept(n) => !v.come_out() && v.point() != n as i64,
        Guard::PointOnAndUp(b) => !v.come_out() && v.up(b),
        Guard::PointOnAndLess(r, n) => !v.come_out() && read(v, r) < n as i64,
        Guard::Truthy(r) => read(v, r) != 0,
    }
}

/// The same arithmetic the machine does, for a compiler that wants to know
/// what an expression comes to before anything runs. One definition, so a
/// static check can never disagree with the interpreter it is checking.
#[inline]
pub(crate) fn fold_bin(op: BinOp, a: i64, b: i64) -> i64 {
    apply_bin(op, a, b)
}

#[inline]
fn apply_bin(op: BinOp, a: i64, b: i64) -> i64 {
    match op {
        BinOp::Add => a.saturating_add(b),
        BinOp::Sub => a.saturating_sub(b),
        BinOp::Mul => a.saturating_mul(b),
        // A strategy is data; no arithmetic it can write may take down a
        // worker thread mid-sweep. Division has two ways to trap: by zero,
        // which yields zero, and `i64::MIN / -1`, whose true answer is one
        // past the top of the type — so it saturates there, like every other
        // operator here.
        BinOp::Div => {
            if b == 0 {
                0
            } else {
                a.saturating_div(b)
            }
        }
        BinOp::Min => a.min(b),
        BinOp::Max => a.max(b),
        BinOp::Lt => (a < b) as i64,
        BinOp::Le => (a <= b) as i64,
        BinOp::Gt => (a > b) as i64,
        BinOp::Ge => (a >= b) as i64,
        BinOp::Eq => (a == b) as i64,
        BinOp::Ne => (a != b) as i64,
        BinOp::And => (a != 0 && b != 0) as i64,
        BinOp::Or => (a != 0 || b != 0) as i64,
    }
}

#[inline]
fn read(v: &TableView<'_>, r: Read) -> i64 {
    match r {
        Read::Point => v.point(),
        Read::ComeOut => v.come_out() as i64,
        Read::LastTotal => v.last_total(),
        Read::Roll => v.roll(),
        Read::RollsThisShooter => v.rolls_this_shooter(),
        Read::Shooter => v.shooter(),
        Read::Cash => v.cash(),
        Read::Wealth => v.wealth(),
        Read::Profit => v.profit(),
        Read::PeakProfit => v.peak_profit(),
        Read::Drawdown => v.drawdown(),
        Read::Handle => v.handle(),
        Read::BuyIn => v.buy_in(),
        Read::TableMin => v.table_min(),
        Read::TableMax => v.table_max(),
        Read::Field12Triple => v.field_12_triple(),
        Read::ComeOddsWorkOnComeout => v.come_odds_work_on_comeout(),
        Read::Stake(b) => v.stake(b),
        Read::Up(b) => v.up(b) as i64,
        Read::Working(b) => v.working(b),
        Read::WorkingComeOut(b) => v.working_comeout(b),
        Read::LiveCome => v.live_come(),
        Read::LiveDontCome => v.live_dont_come(),
        Read::ComePoint(n) => v.come_point(n),
        Read::DontComePoint(n) => v.dont_come_point(n),
        Read::OnTableFace => v.on_table_face(),
        Read::Hits(n) => v.hits(n),
        Read::HitsThisShooter(n) => v.hits_this_shooter(n),
        Read::Wins(b) => v.wins(b),
        Read::Losses(b) => v.losses(b),
        Read::Streak(b) => v.streak(b),
        Read::Paid(b) => v.paid(b),
    }
}

/// The coarse trigger a [`Trigger`] compiles to, when it has one.
pub(crate) fn fired_bit(t: Trigger) -> Option<u8> {
    Some(match t {
        Trigger::SessionStart => fired::SESSION_START,
        Trigger::ComeOut => fired::COME_OUT,
        Trigger::PointEstablished => fired::POINT_ESTABLISHED,
        Trigger::PointMade => fired::POINT_MADE,
        Trigger::SevenOut => fired::SEVEN_OUT,
        Trigger::Roll => fired::ROLL,
        _ => return None,
    })
}

#[cfg(test)]
mod size_tests {
    use super::*;

    /// The instruction stream is walked once per rule per decision, so its
    /// size is cache pressure in the hot path. This is a tripwire, not a
    /// target: if `Op` grows, someone should have decided that it should.
    #[test]
    fn the_instruction_stays_small() {
        assert!(
            std::mem::size_of::<Op>() <= 16,
            "Op is {} bytes",
            std::mem::size_of::<Op>()
        );
    }

    /// A strategy is data, and no arithmetic data can express may take a
    /// worker thread with it.
    ///
    /// Division has two ways to trap and only one of them was guarded:
    /// `i64::MIN / -1` panics in every build profile, both constants are
    /// writable in the language, and the interpreter runs on every core with
    /// nothing to catch it.
    #[test]
    fn no_arithmetic_a_strategy_can_write_will_panic() {
        let extremes = [i64::MIN, i64::MIN + 1, -1, 0, 1, i64::MAX];
        let ops = [
            BinOp::Add,
            BinOp::Sub,
            BinOp::Mul,
            BinOp::Div,
            BinOp::Min,
            BinOp::Max,
            BinOp::Lt,
            BinOp::Le,
            BinOp::Gt,
            BinOp::Ge,
            BinOp::Eq,
            BinOp::Ne,
            BinOp::And,
            BinOp::Or,
        ];
        for op in ops {
            for a in extremes {
                for b in extremes {
                    // Saturating, wrapping or zero — the value is the
                    // language's business. That it returns at all is this
                    // test's business.
                    let _ = apply_bin(op, a, b);
                }
            }
        }
        assert_eq!(apply_bin(BinOp::Div, i64::MIN, -1), i64::MAX);
        assert_eq!(apply_bin(BinOp::Div, 7, 0), 0);
    }
}
