// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The intent surface: what a strategy may ask the table to do, and the one
//! place where money moves onto the layout.
//!
//! Principle 1 of [`STRATEGY_DSL.md`](../../../../docs/STRATEGY_DSL.md): a
//! strategy never mutates the layout. It proposes [`Action`]s; the table
//! validates each against point state, odds policy, payout units, the table
//! maximum, and the bankroll, then applies it or refuses it with a
//! [`RejectReason`] the observer can see. Every decision about *whether* a
//! bet is legal or affordable lives here; a strategy decides only what to
//! ask for.
//!
//! There is no `Vec<Action>` anywhere, by design: the built-in player calls
//! [`Session::apply`] in sequence, and the compiled interpreter will emit
//! into a fixed-size stack buffer. Nothing allocates per roll.

use crate::bets::{
    dont_lay_for_win, hard_index, place_index, place_stake, place_unit_of, round_up,
};
use crate::game::Session;
use crate::strategy::view::Features;
use crate::trace::{Attempted, BetEventKind, BetKind, RollObserver};

/// Which bet an action concerns. Numbers ride along where a bet lives on a
/// box number, exactly as in [`BetKind`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BetRef {
    Pass,
    PassOdds,
    DontPass,
    DontPassLay,
    Come,
    DontCome,
    ComeOdds(u8),
    DontComeLay(u8),
    Place(u8),
    Hardway(u8),
    Field,
    AnySeven,
    AnyCraps,
}

/// How much to put up. Resolution to cents happens inside the table, because
/// payout units and the odds policy are table rules, not strategy choices.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Amount {
    /// The table's own base stake for this bet: the table minimum rounded
    /// up to the bet's payout unit, or the configured prop stake. What a
    /// flat player bets without having to name a number.
    Base,
    /// Whatever this stream's pressing system calls for right now, clipped
    /// to the table maximum and rounded the way the table would take it.
    /// Identical to [`Amount::Base`] under a flat progression, which is why
    /// a flat player never has to say which it meant.
    Pressed,
    /// Exactly this many cents, rounded up to the bet's payout unit.
    Cents(i64),
    /// This many table minimums, rounded up to the bet's payout unit.
    Units(i64),
    /// The most the configured odds policy allows behind this point. Only
    /// meaningful for the four odds bets.
    MaxOdds,
}

/// What a strategy asks the table to do.
///
/// P0 defines the placement half of the surface. Press, regress, take-down,
/// working on/off, set, and leave arrive with the language itself, and each
/// is an arm added here rather than a new pathway through the session.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Put the bet up if it is not already up; top odds up to the target.
    /// Idempotent: asking for a bet that is already up is a no-op, not a
    /// rejection.
    Bet(BetRef, Amount),
    /// Move a working bet to a new stake, taking the difference from the
    /// rail or returning it. This is the decision-point counterpart to a
    /// progression, which presses at resolution; when both touch the same
    /// bet on the same roll the progression sets the stake first and the
    /// rule overrides it, the same last-write-wins ordering that governs two
    /// rules touching one bet.
    SetStake(BetRef, Amount),
    /// Take the bet down; the stake comes back to the rail. Refused for
    /// contract bets, which cannot be removed once the point is on.
    Down(BetRef),
    /// Turn a working bet off or back on. An off bet sits on the layout
    /// resolving nothing — still the player's money, still counted in what
    /// they would walk away with.
    Working(BetRef, bool, crate::strategy::ast::WorkingWhen),
    /// End the session and pick everything up. The take-profit rule in the
    /// configuration is the Explorer's axis; this is a strategy leaving on
    /// its own terms, which is how a stop-loss is said.
    Leave,
}

/// Why the table refused an action. Every one of these is emitted as a
/// [`BetEventKind::Rejected`] event, because a strategy that silently does
/// nothing is the worst thing this surface could allow (Principle 4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RejectReason {
    /// Not legal at this moment, for a reason none of the sharper variants
    /// names: odds with no flat behind them, calling off a bet that cannot
    /// be called off.
    NotAllowedNow,
    /// A line bet once the point is established. The pass line and the don't
    /// pass are made on the come-out and nowhere else.
    LineBetWithPointOn,
    /// A come, don't come, place bet or hardway while the puck is off. These
    /// need a point to be about.
    NeedsPointOn,
    /// A place bet on the number that is currently the point. The point is
    /// covered by the line, and no table takes a place bet on it.
    NumberIsThePoint,
    /// A contract bet: once the point is on it cannot come down, and no
    /// table will take it back because the player changed their mind.
    ContractBet,
    /// Nothing is on this bet to press, take down, or turn off.
    NothingThere,
    /// The odds policy allows nothing behind this point.
    NoOddsAllowed,
    /// A named stake the table would not take: below the minimum for this
    /// bet. Refused rather than quietly rounded up into one it would take,
    /// because a strategy that asked for $0.12 and got $6 is not the
    /// strategy anyone wrote.
    BelowTableMinimum,
    /// A `press` whose target sits at or below the current stake, or a
    /// `regress` whose target sits at or above it. The rule fired and asked
    /// for something that is not what the verb means.
    WrongDirection,
    /// Neither the requested stake nor the base fallback fit the bankroll.
    InsufficientBankroll,
}

impl RejectReason {
    /// Why the table said no, in the words the Replay ledger and the Bench
    /// show the user. Plain language, no jargon, no error codes.
    pub fn label(&self) -> &'static str {
        match self {
            RejectReason::NotAllowedNow => "not allowed right now",
            RejectReason::LineBetWithPointOn => "the point is already established",
            RejectReason::NeedsPointOn => "there's no point yet",
            RejectReason::NumberIsThePoint => "that number is the point",
            RejectReason::ContractBet => "a contract bet can't come down",
            RejectReason::NothingThere => "there's nothing on that bet",
            RejectReason::NoOddsAllowed => "odds policy allows none",
            RejectReason::BelowTableMinimum => "below the table minimum for that bet",
            RejectReason::WrongDirection => "a press can't lower, a regress can't raise",
            RejectReason::InsufficientBankroll => "bankroll won't cover it",
        }
    }
}

/// The result of an adjudicated action: the cents that went onto the layout.
/// `Ok(0)` is a no-op — the bet was already up, or odds were already full.
pub type Adjudication = Result<i64, RejectReason>;

/// A flat bet resolved to the slot it occupies, with its box index already
/// looked up. Resolving once and carrying the index keeps the adjudicator
/// from re-deriving what the caller already knew — the loaded-table
/// configurations place ten bets a roll, and three `place_index` matches
/// apiece is not free.
#[derive(Clone, Copy)]
enum Slot {
    Pass,
    DontPass,
    Come,
    DontCome,
    Place(usize),
    Hardway(usize),
    Field,
    AnySeven,
    AnyCraps,
}

/// A flat bet's slot and its base stake — the amount bet when the bankroll
/// cannot cover what the strategy asked for.
struct FlatSpec {
    slot: Slot,
    base: i64,
}

impl<O: RollObserver, F: Features> Session<'_, O, F> {
    /// Adjudicate one action. The single site where a strategy's intent
    /// becomes money on the layout.
    #[inline]
    pub(crate) fn apply(&mut self, action: Action) -> Adjudication {
        match action {
            Action::Bet(bet, amount) => self.apply_rule_bet(bet, amount),
            Action::SetStake(bet, amount) => self.apply_set_stake(bet, amount),
            Action::Down(bet) => self.apply_down(bet),
            Action::Working(bet, on, when) => self.apply_working(bet, on, when),
            Action::Leave => {
                self.leaving = true;
                Ok(0)
            }
        }
    }

    /// Move a working bet to a new stake. Raising takes the difference from
    /// the rail; lowering returns it.
    #[inline(never)]
    fn apply_set_stake(&mut self, bet: BetRef, amount: Amount) -> Adjudication {
        if is_odds(bet) {
            // Odds already top up toward a target, which is the same thing.
            return self.apply_bet(bet, amount);
        }
        let asked = asked_cents(amount);
        let spec = match self.flat_spec(bet) {
            Ok(s) => s,
            Err(r) => return self.reject_asking(bet, Attempted::Press, asked, r),
        };
        let cur = *self.slot(spec.slot);
        if cur == 0 {
            return self.reject_asking(bet, Attempted::Press, asked, RejectReason::NothingThere);
        }
        let want = match amount {
            Amount::Pressed => self.pressed_stake(bet, spec.base),
            other => match self.resolve_amount(bet, other, spec.base) {
                Ok(v) => v,
                Err(r) => return self.reject_asking(bet, Attempted::Press, asked, r),
            },
        };
        // A stake below the table's own minimum for this bet is not a stake;
        // a player who wants nothing there takes it down.
        let want = want.max(spec.base);
        // `pressed` is the stream's own answer, so it neither needs holding
        // to the table again nor has anything to tell the stream. Every other
        // amount is the rule naming a figure, and the stream is told: without
        // that, the progression re-prices the bet at the next resolution and
        // the press is undone by the very win it was riding — which is why no
        // ladder could ever climb.
        let want = if matches!(amount, Amount::Pressed) {
            want
        } else {
            let want = self.clip_to_table_max(bet, want, spec.base);
            self.set_stream_stake(bet, want);
            want
        };
        if want > cur {
            match self.try_stake(want - cur) {
                Some(a) => {
                    *self.slot_mut(spec.slot) = cur + a;
                    self.emit(bet_kind(bet), BetEventKind::Placed, a);
                    Ok(a)
                }
                None => self.reject_asking(
                    bet,
                    Attempted::Press,
                    want - cur,
                    RejectReason::InsufficientBankroll,
                ),
            }
        } else if want < cur {
            let back = cur - want;
            self.cash += back;
            *self.slot_mut(spec.slot) = want;
            self.emit(bet_kind(bet), BetEventKind::TakenDown, back);
            Ok(0)
        } else {
            Ok(0)
        }
    }

    /// Take a bet down. The pass line and a come flat are contract bets once
    /// the point is on: the player is committed, and the table will not give
    /// it back.
    #[inline(never)]
    fn apply_down(&mut self, bet: BetRef) -> Adjudication {
        if is_odds(bet) {
            return self.take_down_odds(bet);
        }
        if matches!(bet, BetRef::Pass | BetRef::Come) && self.point.is_some() {
            return self.reject_asking(bet, Attempted::Down, 0, RejectReason::ContractBet);
        }
        let spec = match self.flat_spec(bet) {
            Ok(s) => s,
            Err(r) => return self.reject_asking(bet, Attempted::Down, 0, r),
        };
        let cur = *self.slot(spec.slot);
        if cur == 0 {
            return self.reject_asking(bet, Attempted::Down, 0, RejectReason::NothingThere);
        }
        *self.slot_mut(spec.slot) = 0;
        self.cash += cur;
        self.emit(bet_kind(bet), BetEventKind::TakenDown, cur);
        Ok(0)
    }

    /// Odds are never contract bets — they come down whenever the player
    /// asks, which is most of why they are the best bet on the table.
    #[inline(never)]
    fn take_down_odds(&mut self, bet: BetRef) -> Adjudication {
        let (cur, win) = match bet {
            BetRef::PassOdds => (self.pass_odds, 0),
            BetRef::DontPassLay => (self.dont_lay, self.dont_lay_win),
            BetRef::ComeOdds(n) => (place_index(n).map_or(0, |i| self.come_odds[i]), 0),
            BetRef::DontComeLay(n) => {
                let i = place_index(n);
                (
                    i.map_or(0, |i| self.dc_lay[i]),
                    i.map_or(0, |i| self.dc_lay_win[i]),
                )
            }
            // Exhaustive rather than a wildcard: a bet type added later must
            // make this stop compiling, not fall silently into "there is
            // nothing there" at a table that would have taken it.
            BetRef::Pass
            | BetRef::DontPass
            | BetRef::Come
            | BetRef::DontCome
            | BetRef::Place(_)
            | BetRef::Hardway(_)
            | BetRef::Field
            | BetRef::AnySeven
            | BetRef::AnyCraps => (0, 0),
        };
        let _ = win;
        if cur == 0 {
            return self.reject_asking(bet, Attempted::Down, 0, RejectReason::NothingThere);
        }
        match bet {
            BetRef::PassOdds => self.pass_odds = 0,
            BetRef::DontPassLay => {
                self.dont_lay = 0;
                self.dont_lay_win = 0;
            }
            BetRef::ComeOdds(n) => {
                if let Some(i) = place_index(n) {
                    self.come_odds[i] = 0;
                }
            }
            BetRef::DontComeLay(n) => {
                if let Some(i) = place_index(n) {
                    self.dc_lay[i] = 0;
                    self.dc_lay_win[i] = 0;
                }
            }
            _ => {}
        }
        self.cash += cur;
        self.emit(bet_kind(bet), BetEventKind::TakenDown, cur);
        Ok(0)
    }

    /// Turn a place bet or hardway off or on. Only these can be called off:
    /// line and come bets are contract bets, odds ride with what they back,
    /// and a one-roll bet resolves before the question could be asked.
    #[inline(never)]
    fn apply_working(
        &mut self,
        bet: BetRef,
        on: bool,
        when: crate::strategy::ast::WorkingWhen,
    ) -> Adjudication {
        use crate::strategy::ast::WorkingWhen;
        match bet {
            BetRef::Place(n) => match place_index(n) {
                Some(i) => {
                    match when {
                        WorkingWhen::PointCycle => self.place_working[i] = on,
                        WorkingWhen::ComeOut => self.place_comeout[i] = on,
                    }
                    Ok(0)
                }
                None => self.reject_asking(bet, Attempted::Working, 0, RejectReason::NotAllowedNow),
            },
            BetRef::Hardway(n) => match hard_index(n) {
                Some(i) => {
                    match when {
                        WorkingWhen::PointCycle => self.hard_working[i] = on,
                        WorkingWhen::ComeOut => self.hard_comeout[i] = on,
                    }
                    Ok(0)
                }
                None => self.reject_asking(bet, Attempted::Working, 0, RejectReason::NotAllowedNow),
            },
            _ => self.reject_asking(bet, Attempted::Working, 0, RejectReason::NotAllowedNow),
        }
    }

    /// The placement arm of [`Session::apply`], reachable directly.
    ///
    /// The built-in player only ever places, and routing it through the
    /// dispatcher meant building an `Action` at twelve call sites purely to
    /// take it apart again. Compiled strategies still go through `apply`,
    /// which is where the surface stays honest: this is the same
    /// adjudication either way.
    #[inline]
    pub(crate) fn apply_bet(&mut self, bet: BetRef, amount: Amount) -> Adjudication {
        self.place_bet(bet, amount, false)
    }

    /// The same placement, asked for by a *rule* rather than by the built-in
    /// player.
    ///
    /// The difference is `shape`: a rule names a figure out of the language
    /// and the table has to hold it to the minimum, the maximum, and the
    /// stream it belongs to. The built-in player arrives with a stake its
    /// progression has already shaped, and re-shaping it here would round a
    /// second time and drift the two players apart — which the equivalence
    /// battery exists to catch.
    #[inline]
    pub(crate) fn apply_rule_bet(&mut self, bet: BetRef, amount: Amount) -> Adjudication {
        self.place_bet(bet, amount, true)
    }

    #[inline]
    fn place_bet(&mut self, bet: BetRef, amount: Amount, shape: bool) -> Adjudication {
        let asked = asked_cents(amount);
        if is_odds(bet) {
            return if self.odds_flat_is_up(bet) {
                self.apply_odds(bet, amount)
            } else {
                self.reject_asking(bet, Attempted::Bet, asked, RejectReason::NotAllowedNow)
            };
        }
        let spec = match self.flat_spec(bet) {
            Ok(s) => s,
            Err(r) => return self.reject_asking(bet, Attempted::Bet, asked, r),
        };
        if *self.slot(spec.slot) != 0 {
            return Ok(0); // already up; `Bet` is idempotent
        }
        let want = match amount {
            Amount::Pressed => self.pressed_stake(bet, spec.base),
            other => match self.resolve_amount(bet, other, spec.base) {
                Ok(v) => v,
                Err(r) => return self.reject_asking(bet, Attempted::Bet, asked, r),
            },
        };
        // Only a figure the rule named itself is shaped here. `base` and
        // `pressed` arrive already held to the table by the progression
        // machinery that produced them, and shaping them a second time would
        // round twice and drift the compiled player away from the built-in
        // one — which is what the equivalence battery is watching for.
        let named = matches!(amount, Amount::Cents(_) | Amount::Units(_));
        let want = if shape && named {
            let want = self.clip_to_table_max(bet, want, spec.base);
            // A rule that names its own figure has said what this bet is
            // worth from now on; the stream is told, so the progression does
            // not re-price it back at the next resolution.
            self.set_stream_stake(bet, want);
            want
        } else {
            want
        };
        match self.try_stake_or_base(want, spec.base) {
            Some(a) => {
                *self.slot_mut(spec.slot) = a;
                self.emit(bet_kind(bet), BetEventKind::Placed, a);
                Ok(a)
            }
            None => self.reject_asking(
                bet,
                Attempted::Bet,
                want,
                RejectReason::InsufficientBankroll,
            ),
        }
    }

    /// Resolve a flat bet to its slot and base stake, or say why the game
    /// state does not permit it right now. These are table rules; a
    /// strategy's own conditions (how many come bets it wants, which numbers
    /// it places) are not expressed here.
    ///
    /// The refusal is named rather than lumped under one reason, because
    /// "not allowed right now" is three different rules of craps wearing one
    /// label, and an author reading it back has no way to tell which one
    /// they broke.
    #[inline]
    fn flat_spec(&self, bet: BetRef) -> Result<FlatSpec, RejectReason> {
        // Deliberately not routed through `flat_spec_any`: this is the hot
        // placement path, and the extra call cost it measurably. The two
        // matches over the same enum are both exhaustive, so the compiler
        // keeps them honest with each other.
        let (slot, base) = match bet {
            BetRef::Pass => (Slot::Pass, self.min),
            BetRef::DontPass => (Slot::DontPass, self.min),
            BetRef::Come => (Slot::Come, self.min),
            BetRef::DontCome => (Slot::DontCome, self.min),
            BetRef::Field => (Slot::Field, self.min),
            BetRef::Place(n) => (
                Slot::Place(place_index(n).ok_or(RejectReason::NotAllowedNow)?),
                place_stake(self.min, n),
            ),
            BetRef::Hardway(n) => (
                Slot::Hardway(hard_index(n).ok_or(RejectReason::NotAllowedNow)?),
                self.rules.prop_bet_cents,
            ),
            BetRef::AnySeven => (Slot::AnySeven, self.rules.prop_bet_cents),
            BetRef::AnyCraps => (Slot::AnyCraps, self.rules.prop_bet_cents),
            _ => return Err(RejectReason::NotAllowedNow),
        };
        let spec = FlatSpec { slot, base };
        match bet {
            BetRef::Pass | BetRef::DontPass if self.point.is_some() => {
                Err(RejectReason::LineBetWithPointOn)
            }
            BetRef::Come | BetRef::DontCome | BetRef::Hardway(_) if self.point.is_none() => {
                Err(RejectReason::NeedsPointOn)
            }
            // The point number is never placed, and place bets are made only
            // while a point is on.
            BetRef::Place(_) if self.point.is_none() => Err(RejectReason::NeedsPointOn),
            // A real table will usually sell you the point as a place bet on
            // top of the line bet covering it. This engine refused it from
            // the start, which is a divergence rather than a rule of craps —
            // so it is now the table's answer to give.
            BetRef::Place(n) if self.point == Some(n) && !self.rules.place_the_point => {
                Err(RejectReason::NumberIsThePoint)
            }
            _ => Ok(spec),
        }
    }

    #[inline]
    fn slot(&self, slot: Slot) -> &i64 {
        match slot {
            Slot::Pass => &self.pass,
            Slot::DontPass => &self.dont,
            Slot::Come => &self.come_flat,
            Slot::DontCome => &self.dc_flat,
            Slot::Place(i) => &self.place[i],
            Slot::Hardway(i) => &self.hard[i],
            Slot::Field => &self.field_bet,
            Slot::AnySeven => &self.any7_bet,
            Slot::AnyCraps => &self.anycraps_bet,
        }
    }

    #[inline]
    fn slot_mut(&mut self, slot: Slot) -> &mut i64 {
        match slot {
            Slot::Pass => &mut self.pass,
            Slot::DontPass => &mut self.dont,
            Slot::Come => &mut self.come_flat,
            Slot::DontCome => &mut self.dc_flat,
            Slot::Place(i) => &mut self.place[i],
            Slot::Hardway(i) => &mut self.hard[i],
            Slot::Field => &mut self.field_bet,
            Slot::AnySeven => &mut self.any7_bet,
            Slot::AnyCraps => &mut self.anycraps_bet,
        }
    }

    /// Odds need a flat behind them; without one there is nothing to back.
    #[inline]
    fn odds_flat_is_up(&self, bet: BetRef) -> bool {
        match bet {
            BetRef::PassOdds => self.point.is_some() && self.pass > 0,
            BetRef::DontPassLay => self.point.is_some() && self.dont > 0,
            BetRef::ComeOdds(n) => place_index(n).is_some_and(|i| self.come_points[i] > 0),
            BetRef::DontComeLay(n) => place_index(n).is_some_and(|i| self.dc_points[i] > 0),
            _ => false,
        }
    }

    /// Odds top up toward a target rather than being set once, because a
    /// come flat that stacks on a repeated number raises the odds the policy
    /// allows behind it.
    fn apply_odds(&mut self, bet: BetRef, amount: Amount) -> Adjudication {
        let point = self.odds_point(bet);
        let mult = self.rules.odds_policy.multiple(point);
        if mult == 0 {
            return self.reject(bet, RejectReason::NoOddsAllowed);
        }
        match bet {
            BetRef::PassOdds => {
                let target = match amount {
                    Amount::MaxOdds => self.pass * mult,
                    other => self.resolve_amount(bet, other, 0)?,
                };
                if target <= self.pass_odds {
                    return Ok(0);
                }
                match self.try_stake(target - self.pass_odds) {
                    Some(a) => {
                        self.pass_odds += a;
                        self.pass_odds_point = point;
                        self.emit(BetKind::PassOdds, BetEventKind::Placed, a);
                        Ok(a)
                    }
                    None => self.reject_asking(
                        bet,
                        Attempted::Bet,
                        target,
                        RejectReason::InsufficientBankroll,
                    ),
                }
            }
            BetRef::DontPassLay => {
                // Lays are sized by what they win, not what they risk.
                let target_win = match amount {
                    Amount::MaxOdds => self.dont * mult,
                    other => self.resolve_amount(bet, other, 0)?,
                };
                if target_win <= self.dont_lay_win {
                    return Ok(0);
                }
                let stake = dont_lay_for_win(target_win - self.dont_lay_win, point);
                match self.try_stake(stake) {
                    Some(a) => {
                        self.dont_lay += a;
                        self.dont_lay_win = target_win;
                        self.emit(BetKind::DontPassLay, BetEventKind::Placed, a);
                        Ok(a)
                    }
                    None => self.reject_asking(
                        bet,
                        Attempted::Bet,
                        stake,
                        RejectReason::InsufficientBankroll,
                    ),
                }
            }
            BetRef::ComeOdds(num) => {
                let i = place_index(num).expect("come odds on a non-point number");
                let target = match amount {
                    Amount::MaxOdds => self.come_points[i] * mult,
                    other => self.resolve_amount(bet, other, 0)?,
                };
                if target <= self.come_odds[i] {
                    return Ok(0);
                }
                match self.try_stake(target - self.come_odds[i]) {
                    Some(a) => {
                        self.come_odds[i] += a;
                        self.emit(BetKind::ComeOdds(num), BetEventKind::Placed, a);
                        Ok(a)
                    }
                    None => self.reject_asking(
                        bet,
                        Attempted::Bet,
                        target,
                        RejectReason::InsufficientBankroll,
                    ),
                }
            }
            BetRef::DontComeLay(num) => {
                let i = place_index(num).expect("dc lay on a non-point number");
                let target_win = match amount {
                    Amount::MaxOdds => self.dc_points[i] * mult,
                    other => self.resolve_amount(bet, other, 0)?,
                };
                if target_win <= self.dc_lay_win[i] {
                    return Ok(0);
                }
                let stake = dont_lay_for_win(target_win - self.dc_lay_win[i], num);
                match self.try_stake(stake) {
                    Some(a) => {
                        self.dc_lay[i] += a;
                        self.dc_lay_win[i] = target_win;
                        self.emit(BetKind::DontComeLay(num), BetEventKind::Placed, a);
                        Ok(a)
                    }
                    None => self.reject_asking(
                        bet,
                        Attempted::Bet,
                        stake,
                        RejectReason::InsufficientBankroll,
                    ),
                }
            }
            _ => unreachable!("not an odds bet"),
        }
    }

    /// The point an odds bet sits behind — the box number for come odds, the
    /// table point for line odds.
    fn odds_point(&self, bet: BetRef) -> u8 {
        match bet {
            BetRef::ComeOdds(n) | BetRef::DontComeLay(n) => n,
            _ => self.point.unwrap_or(0),
        }
    }

    /// What is on this bet now — the slot the table would write, which for
    /// a come flat is the box and not the numbers.
    pub(crate) fn current_stake_of(&self, bet: BetRef) -> i64 {
        match self.flat_spec_any(bet) {
            Some(spec) => *self.slot(spec.slot),
            None => match bet {
                BetRef::PassOdds => self.pass_odds,
                BetRef::DontPassLay => self.dont_lay,
                BetRef::ComeOdds(n) => place_index(n).map_or(0, |i| self.come_odds[i]),
                BetRef::DontComeLay(n) => place_index(n).map_or(0, |i| self.dc_lay[i]),
                // `flat_spec_any` answered for every other shape already, so
                // this arm is the four odds bets and nothing else. Spelled
                // out so a new bet type has to be routed rather than quietly
                // reading as empty.
                BetRef::Pass
                | BetRef::DontPass
                | BetRef::Come
                | BetRef::DontCome
                | BetRef::Place(_)
                | BetRef::Hardway(_)
                | BetRef::Field
                | BetRef::AnySeven
                | BetRef::AnyCraps => 0,
            },
        }
    }

    /// What a stake proposal would come to, without applying it — so a
    /// `press` can be told from a `regress` before either reaches the table.
    pub(crate) fn resolve_target(&mut self, bet: BetRef, amount: Amount) -> i64 {
        let base = self.flat_spec_any(bet).map_or(self.min, |spec| spec.base);
        match amount {
            Amount::Pressed => self.pressed_stake(bet, base),
            other => self.resolve_amount(bet, other, base).unwrap_or(base),
        }
    }

    /// [`Session::flat_spec`] without the legality check — for reading what
    /// is on a bet, which is a question with an answer whatever the point is
    /// doing.
    ///
    /// Kept separate from [`Session::flat_spec`] rather than shared with
    /// it: the placement path is the hot one, and folding the two together
    /// cost it a call it did not need. A little duplication in two matches
    /// over the same enum is the cheaper trade, and the compiler checks
    /// both are exhaustive.
    #[inline(never)]
    fn flat_spec_any(&self, bet: BetRef) -> Option<FlatSpec> {
        let (slot, base) = match bet {
            BetRef::Pass => (Slot::Pass, self.min),
            BetRef::DontPass => (Slot::DontPass, self.min),
            BetRef::Come => (Slot::Come, self.min),
            BetRef::DontCome => (Slot::DontCome, self.min),
            BetRef::Field => (Slot::Field, self.min),
            BetRef::Place(n) => (Slot::Place(place_index(n)?), place_stake(self.min, n)),
            BetRef::Hardway(n) => (Slot::Hardway(hard_index(n)?), self.rules.prop_bet_cents),
            BetRef::AnySeven => (Slot::AnySeven, self.rules.prop_bet_cents),
            BetRef::AnyCraps => (Slot::AnyCraps, self.rules.prop_bet_cents),
            _ => return None,
        };
        Some(FlatSpec { slot, base })
    }

    /// What this bet's progression stream currently calls for. Place bets
    /// round to their payout unit; everything else rounds to whole dollars,
    /// because a real table does not take a $33.75 flat.
    /// Box numbers that are not box numbers cannot reach here from the
    /// grammar — the parser and the compiler both refuse them — but a
    /// strategy is data, and data that took an unexpected shape must not
    /// panic a worker thread mid-sweep. An unknown number falls back to the
    /// base stake rather than asserting its own impossibility.
    fn pressed_stake(&mut self, bet: BetRef, base: i64) -> i64 {
        if let BetRef::Place(n) = bet {
            return match place_index(n) {
                Some(i) => self.prog_place_stake(i),
                None => base,
            };
        }
        let want = match bet {
            BetRef::Pass => self.p_pass.stake,
            BetRef::DontPass => self.p_dont.stake,
            BetRef::Come => self.p_come.stake,
            BetRef::DontCome => self.p_dc.stake,
            BetRef::Field => self.p_field.stake,
            BetRef::Hardway(n) => match hard_index(n) {
                Some(i) => self.p_hard[i].stake,
                None => return base,
            },
            BetRef::AnySeven => self.p_any7.stake,
            BetRef::AnyCraps => self.p_anycraps.stake,
            // Odds have no stream of their own; the flat they sit behind
            // presses for both. Named rather than wildcarded so a new bet
            // type must decide which it is.
            BetRef::PassOdds
            | BetRef::DontPassLay
            | BetRef::ComeOdds(_)
            | BetRef::DontComeLay(_)
            | BetRef::Place(_) => base,
        };
        self.prog_stake(want, base, bet)
    }

    /// Cents for an [`Amount`], rounded up to the bet's payout unit so every
    /// payout stays whole (Principle: money is integer cents end to end).
    /// Cents for an [`Amount`], rounded up to the bet's payout unit so every
    /// payout stays whole — money is integer cents end to end, and a stake
    /// that could pay $12.333… is not a stake this table takes.
    #[inline]
    fn resolve_amount(&self, bet: BetRef, amount: Amount, base: i64) -> Adjudication {
        let raw = match amount {
            Amount::Base => return Ok(base),
            // Handled before this point, where `&mut self` is available.
            Amount::Pressed => return Ok(base),
            // A named figure below what this table takes for this bet is
            // refused, not quietly rounded up into one it would take. The
            // author asked for a stake the table does not sell.
            Amount::Cents(c) if c < base => return Err(RejectReason::BelowTableMinimum),
            Amount::Cents(c) => c,
            // Units are table minimums by definition, so rounding a short
            // one up to the bet's own base is the documented meaning rather
            // than a silent correction — but asking for none of them is not
            // a bet.
            Amount::Units(n) if n <= 0 => return Err(RejectReason::BelowTableMinimum),
            Amount::Units(n) => n.saturating_mul(self.min),
            // Only reachable when a strategy asks for max odds on a bet that
            // takes none; the caller has already handled the odds bets.
            Amount::MaxOdds => return Err(RejectReason::NoOddsAllowed),
        };
        let shaped = match bet {
            BetRef::Place(n) => round_up(raw, place_unit_of(n)),
            _ => raw,
        };
        Ok(shaped.max(base))
    }

    /// Hold an explicit stake to the table maximum, telling the observer when
    /// it bites. Principle 4: the truncation that stops a Martingale is an
    /// event, not a flat spot in a curve.
    #[inline]
    fn clip_to_table_max(&mut self, bet: BetRef, want: i64, base: i64) -> i64 {
        let max = self.table_max.max(base);
        if want > max {
            self.emit(bet_kind(bet), BetEventKind::ClippedToMax, max);
            return max;
        }
        want
    }

    /// Tell this bet's progression stream what the bet is worth now.
    ///
    /// A rule acting at the decision point and a progression acting at
    /// resolution are two hands on the same bet. Without this the
    /// progression wins every argument: it re-prices the bet from its own
    /// stake on the next win, so a press that raised the felt is torn back
    /// down by the very hit it was riding, and no ladder can ever climb.
    /// Odds have no stream of their own — they top up toward a target the
    /// flat decides — so they are left alone.
    fn set_stream_stake(&mut self, bet: BetRef, stake: i64) {
        let st = match bet {
            BetRef::Pass => &mut self.p_pass,
            BetRef::DontPass => &mut self.p_dont,
            BetRef::Come => &mut self.p_come,
            BetRef::DontCome => &mut self.p_dc,
            BetRef::Field => &mut self.p_field,
            BetRef::AnySeven => &mut self.p_any7,
            BetRef::AnyCraps => &mut self.p_anycraps,
            BetRef::Place(n) => match place_index(n) {
                Some(i) => &mut self.p_place[i],
                None => return,
            },
            BetRef::Hardway(n) => match hard_index(n) {
                Some(i) => &mut self.p_hard[i],
                None => return,
            },
            BetRef::PassOdds
            | BetRef::DontPassLay
            | BetRef::ComeOdds(_)
            | BetRef::DontComeLay(_) => return,
        };
        st.stake = stake;
    }

    /// Refuse, and say what was being attempted and for how much.
    ///
    /// `asked` is the stake the strategy wanted, not the zero it ended up
    /// with — a refusal that reports nothing costs the reader the one number
    /// that would have told them which rule they are looking at.
    #[inline]
    pub(crate) fn reject_asking(
        &mut self,
        bet: BetRef,
        what: Attempted,
        asked: i64,
        reason: RejectReason,
    ) -> Adjudication {
        self.emit(
            bet_kind(bet),
            BetEventKind::Rejected { reason, what },
            asked,
        );
        Err(reason)
    }

    #[inline]
    pub(crate) fn reject(&mut self, bet: BetRef, reason: RejectReason) -> Adjudication {
        self.reject_asking(bet, Attempted::Bet, 0, reason)
    }
}

/// What an amount asked for in cents, where that is a number at all.
///
/// `base`, `pressed` and `max` are answered by the table, so there is no
/// figure to report back until it has answered — but a named one belongs in
/// the refusal, because "bankroll won't cover it" beside a full bankroll is
/// how a zero-stake bet from an uninitialized counter reads.
#[inline]
const fn asked_cents(amount: Amount) -> i64 {
    match amount {
        Amount::Cents(c) => c,
        _ => 0,
    }
}

/// Whether this reference names one of the four odds bets, which top up
/// toward a target rather than being set once.
#[inline]
const fn is_odds(bet: BetRef) -> bool {
    matches!(
        bet,
        BetRef::PassOdds | BetRef::DontPassLay | BetRef::ComeOdds(_) | BetRef::DontComeLay(_)
    )
}

/// The trace vocabulary for a bet reference. Come and don't come flats sit in
/// their box until they travel, which is why the box and the number are
/// distinct [`BetKind`]s.
pub(crate) fn bet_kind(bet: BetRef) -> BetKind {
    match bet {
        BetRef::Pass => BetKind::Pass,
        BetRef::PassOdds => BetKind::PassOdds,
        BetRef::DontPass => BetKind::DontPass,
        BetRef::DontPassLay => BetKind::DontPassLay,
        BetRef::Come => BetKind::Come,
        BetRef::DontCome => BetKind::DontCome,
        BetRef::ComeOdds(n) => BetKind::ComeOdds(n),
        BetRef::DontComeLay(n) => BetKind::DontComeLay(n),
        BetRef::Place(n) => BetKind::Place(n),
        BetRef::Hardway(n) => BetKind::Hardway(n),
        BetRef::Field => BetKind::Field,
        BetRef::AnySeven => BetKind::AnySeven,
        BetRef::AnyCraps => BetKind::AnyCraps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bets::{BetSelection, OddsPolicy, Rules};
    use crate::strategy::ast::WorkingWhen;
    use crate::trace::BetEvent;

    /// Collects every bet event, so a test can assert that a refusal was
    /// *seen* and not merely returned — Principle 4 is about what reaches
    /// the observer, not what the function signature says.
    #[derive(Default)]
    struct Spy {
        events: Vec<BetEvent>,
    }

    impl RollObserver for Spy {
        fn event(&mut self, ev: BetEvent) {
            self.events.push(ev);
        }
    }

    fn rules(odds: OddsPolicy) -> Rules {
        Rules {
            odds_policy: odds,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 1000,
            place_the_point: false,
        }
    }

    /// A table with `cash` in hand at a $10 minimum, ready to adjudicate.
    fn table<'a>(sel: &'a BetSelection, rules: &'a Rules, cash: i64) -> Session<'a, Spy> {
        Session::with_observer(sel, rules, 1000, cash, false, Spy::default())
    }

    fn refusals(s: &Session<'_, Spy>) -> Vec<RejectReason> {
        s.obs
            .events
            .iter()
            .filter_map(|e| match e.kind {
                BetEventKind::Rejected { reason, .. } => Some(reason),
                _ => None,
            })
            .collect()
    }

    /// A refusal says what was attempted and for how much.
    ///
    /// Without both, a ledger could only report that *something* about a bet
    /// was refused: a place bet refusal could have been the bet, a press, a
    /// take-down or a working toggle, and every one of them reported a stake
    /// of zero — so a $0 bet from a counter that never got its starting
    /// value read as "bankroll won't cover it" beside a full bankroll.
    #[test]
    fn a_refusal_names_the_action_and_the_amount() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 100_000);
        let _ = t.apply(Action::Bet(BetRef::Pass, Amount::Cents(12)));
        t.point = Some(4);
        let _ = t.apply(Action::SetStake(BetRef::Place(6), Amount::Cents(5000)));
        let _ = t.apply(Action::Down(BetRef::Place(8)));
        let _ = t.apply(Action::Working(
            BetRef::Field,
            false,
            WorkingWhen::PointCycle,
        ));
        let seen: Vec<(Attempted, RejectReason, i64)> = t
            .obs
            .events
            .iter()
            .filter_map(|e| match e.kind {
                BetEventKind::Rejected { reason, what } => Some((what, reason, e.stake_cents)),
                _ => None,
            })
            .collect();
        assert_eq!(
            seen,
            vec![
                (Attempted::Bet, RejectReason::BelowTableMinimum, 12),
                (Attempted::Press, RejectReason::NothingThere, 5000),
                (Attempted::Down, RejectReason::NothingThere, 0),
                (Attempted::Working, RejectReason::NotAllowedNow, 0),
            ]
        );
    }

    #[test]
    fn bet_is_idempotent() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 100_000);
        assert_eq!(
            t.apply(Action::Bet(BetRef::Pass, Amount::Units(1))),
            Ok(1000)
        );
        // Asking again is a no-op, not a refusal: no second stake leaves the
        // rail, and nothing is emitted.
        assert_eq!(t.apply(Action::Bet(BetRef::Pass, Amount::Units(1))), Ok(0));
        assert_eq!(t.pass, 1000);
        assert_eq!(t.cash, 99_000);
        assert!(refusals(&t).is_empty());
    }

    #[test]
    fn line_bets_are_refused_once_a_point_is_on() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 100_000);
        t.point = Some(6);
        // The refusal names the rule of craps it broke, not a shrug: an
        // author reading "not allowed right now" off three different
        // prohibitions has no way to tell which one they hit.
        assert_eq!(
            t.apply(Action::Bet(BetRef::Pass, Amount::Units(1))),
            Err(RejectReason::LineBetWithPointOn)
        );
        assert_eq!(t.pass, 0);
        assert_eq!(t.cash, 100_000, "a refused bet costs nothing");
        assert_eq!(refusals(&t), vec![RejectReason::LineBetWithPointOn]);
    }

    #[test]
    fn come_bets_are_refused_on_the_come_out() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 100_000);
        assert_eq!(
            t.apply(Action::Bet(BetRef::Come, Amount::Units(1))),
            Err(RejectReason::NeedsPointOn)
        );
    }

    #[test]
    fn the_point_number_is_never_placed() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 100_000);
        t.point = Some(6);
        assert_eq!(
            t.apply(Action::Bet(BetRef::Place(6), Amount::Units(1))),
            Err(RejectReason::NumberIsThePoint)
        );
        // Any other box number is fine.
        assert!(t
            .apply(Action::Bet(BetRef::Place(8), Amount::Units(1)))
            .is_ok());
    }

    /// The point may be placed where the table allows it.
    ///
    /// This engine refused it from the start, which is a divergence from a
    /// real table rather than a rule of craps — five of the twelve strategies
    /// written against this language in review wanted the number the line
    /// already covers. It is the table's answer to give now, and the default
    /// keeps the old one so no saved result changes meaning.
    #[test]
    fn the_point_may_be_placed_where_the_table_allows_it() {
        let sel = BetSelection::default();
        let default_rules = rules(OddsPolicy::None);
        let mut strict = table(&sel, &default_rules, 100_000);
        strict.point = Some(6);
        assert_eq!(
            strict.apply(Action::Bet(BetRef::Place(6), Amount::Units(1))),
            Err(RejectReason::NumberIsThePoint),
            "the default is what it always was"
        );

        let permissive = Rules {
            place_the_point: true,
            ..default_rules
        };
        let mut t = table(&sel, &permissive, 100_000);
        t.point = Some(6);
        assert_eq!(
            t.apply(Action::Bet(BetRef::Place(6), Amount::Units(1))),
            Ok(1200)
        );
        assert_eq!(t.place[2], 1200);
        // And it pays like any other place bet when the number comes.
        let cash = t.cash;
        t.resolve(3, 3);
        assert!(t.cash > cash, "a placed point that hits is paid");
    }

    #[test]
    fn odds_need_a_flat_behind_them() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::X345));
        let mut t = table(&sel, &r, 100_000);
        t.point = Some(4);
        assert_eq!(
            t.apply(Action::Bet(BetRef::PassOdds, Amount::MaxOdds)),
            Err(RejectReason::NotAllowedNow)
        );
        // With a flat down, max odds on the 4 is 3× the flat.
        t.pass = 1000;
        assert_eq!(
            t.apply(Action::Bet(BetRef::PassOdds, Amount::MaxOdds)),
            Ok(3000)
        );
    }

    #[test]
    fn odds_are_refused_when_the_policy_allows_none() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 100_000);
        t.point = Some(4);
        t.pass = 1000;
        assert_eq!(
            t.apply(Action::Bet(BetRef::PassOdds, Amount::MaxOdds)),
            Err(RejectReason::NoOddsAllowed)
        );
        assert_eq!(refusals(&t), vec![RejectReason::NoOddsAllowed]);
    }

    #[test]
    fn a_bankroll_that_cannot_cover_the_base_is_refused() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 500); // $5 in hand at a $10 table
        assert_eq!(
            t.apply(Action::Bet(BetRef::Pass, Amount::Units(1))),
            Err(RejectReason::InsufficientBankroll)
        );
        assert_eq!(t.cash, 500, "a refused bet costs nothing");
        assert_eq!(refusals(&t), vec![RejectReason::InsufficientBankroll]);
    }

    #[test]
    fn an_unaffordable_ask_falls_back_to_the_base_bet() {
        // Bet what you can rather than sitting out: the pressed stake does
        // not fit, the base does, so the base goes up.
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 1500);
        assert_eq!(
            t.apply(Action::Bet(BetRef::Pass, Amount::Cents(8000))),
            Ok(1000)
        );
        assert_eq!(t.cash, 500);
        assert!(refusals(&t).is_empty(), "a fallback is not a refusal");
    }

    #[test]
    fn place_stakes_round_up_to_their_payout_unit() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 100_000);
        t.point = Some(4);
        // 6 and 8 pay 7:6, so they take $6 units; everything else takes $5.
        assert_eq!(
            t.apply(Action::Bet(BetRef::Place(6), Amount::Cents(1300))),
            Ok(1800)
        );
        assert_eq!(
            t.apply(Action::Bet(BetRef::Place(5), Amount::Cents(1000))),
            Ok(1000)
        );
        assert_eq!(
            t.apply(Action::Bet(BetRef::Place(9), Amount::Cents(1100))),
            Ok(1500)
        );
    }

    #[test]
    fn a_stake_the_table_would_not_take_is_refused_not_rounded_up() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 100_000);
        // The trap this closes: a bare number in the language is cents, so
        // somebody writing `bet place 6 12` means twelve dollars and used to
        // get a six-dollar bet — quietly, and off by fifty times. Asking for
        // less than the table takes is now a refusal with a reason.
        assert_eq!(
            t.apply(Action::Bet(BetRef::Pass, Amount::Cents(12))),
            Err(RejectReason::BelowTableMinimum)
        );
        assert_eq!(t.pass, 0, "and nothing went up");
        t.point = Some(4);
        assert_eq!(
            t.apply(Action::Bet(BetRef::Place(6), Amount::Cents(12))),
            Err(RejectReason::BelowTableMinimum)
        );
        assert_eq!(t.place[2], 0, "and nothing went up");
        // Units are table minimums by definition, so a short one rounds up to
        // the bet's own base — that is what the word means, not a silent
        // correction of a figure somebody wrote.
        assert_eq!(
            t.apply(Action::Bet(BetRef::Place(6), Amount::Units(1))),
            Ok(1200)
        );
    }

    #[test]
    fn an_explicit_stake_is_held_to_the_table_maximum() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 10_000_000);
        t.point = Some(4);
        let max = t.table_max;
        // Clipped, and said so: this is how a real table stops a Martingale,
        // and the spec makes seeing the truncation the whole point.
        t.point = None;
        assert_eq!(
            t.apply(Action::Bet(BetRef::Pass, Amount::Cents(max * 10))),
            Ok(max)
        );
        assert!(t
            .obs
            .events
            .iter()
            .any(|e| matches!(e.kind, BetEventKind::ClippedToMax)));
    }

    #[test]
    fn odds_top_up_rather_than_being_set_once() {
        // A come flat that stacks on a repeated number raises the odds the
        // policy allows behind it; the second ask pays only the difference.
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::X345));
        let mut t = table(&sel, &r, 100_000);
        t.point = Some(9);
        t.come_points[2] = 1000; // a come point on the 6
        assert_eq!(
            t.apply(Action::Bet(BetRef::ComeOdds(6), Amount::MaxOdds)),
            Ok(5000)
        );
        assert_eq!(
            t.apply(Action::Bet(BetRef::ComeOdds(6), Amount::MaxOdds)),
            Ok(0),
            "already full"
        );
        t.come_points[2] = 2000; // the flat stacked
        assert_eq!(
            t.apply(Action::Bet(BetRef::ComeOdds(6), Amount::MaxOdds)),
            Ok(5000),
            "tops up to 5x the new flat, paying only the difference"
        );
        assert_eq!(t.come_odds[2], 10_000);
    }

    #[test]
    fn a_contract_bet_cannot_come_down() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 100_000);
        t.apply(Action::Bet(BetRef::Pass, Amount::Base)).unwrap();
        // On the come-out the bet is not yet a contract — the point has not
        // been established, and nothing has been committed to.
        assert_eq!(t.apply(Action::Down(BetRef::Pass)), Ok(0));
        assert_eq!(t.pass, 0);

        t.apply(Action::Bet(BetRef::Pass, Amount::Base)).unwrap();
        t.point = Some(6);
        assert_eq!(
            t.apply(Action::Down(BetRef::Pass)),
            Err(RejectReason::ContractBet)
        );
        assert_eq!(t.pass, 1000, "still out there, as it must be");
    }

    #[test]
    fn place_bets_and_odds_come_down_on_request() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::X345));
        let mut t = table(&sel, &r, 100_000);
        t.point = Some(4);
        t.pass = 1000;
        t.apply(Action::Bet(BetRef::Place(6), Amount::Base))
            .unwrap();
        t.apply(Action::Bet(BetRef::PassOdds, Amount::MaxOdds))
            .unwrap();
        let before = t.cash;

        assert_eq!(t.apply(Action::Down(BetRef::Place(6))), Ok(0));
        assert_eq!(t.place[2], 0);
        assert_eq!(t.cash, before + 1200);

        // Odds are never a contract bet, which is most of why they are the
        // best bet on the table.
        assert_eq!(t.apply(Action::Down(BetRef::PassOdds)), Ok(0));
        assert_eq!(t.pass_odds, 0);
        assert_eq!(t.cash, before + 1200 + 3000);
    }

    #[test]
    fn taking_down_what_is_not_there_is_refused() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 100_000);
        t.point = Some(4);
        assert_eq!(
            t.apply(Action::Down(BetRef::Place(6))),
            Err(RejectReason::NothingThere)
        );
        assert_eq!(refusals(&t), vec![RejectReason::NothingThere]);
    }

    #[test]
    fn a_stake_moves_up_and_down_against_the_rail() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 100_000);
        t.point = Some(4);
        t.apply(Action::Bet(BetRef::Place(6), Amount::Base))
            .unwrap();
        let start = t.cash;

        // Up: the difference comes off the rail.
        assert_eq!(
            t.apply(Action::SetStake(BetRef::Place(6), Amount::Cents(2400))),
            Ok(1200)
        );
        assert_eq!(t.place[2], 2400);
        assert_eq!(t.cash, start - 1200);

        // Down: it comes back.
        assert_eq!(
            t.apply(Action::SetStake(BetRef::Place(6), Amount::Cents(1200))),
            Ok(0)
        );
        assert_eq!(t.place[2], 1200);
        assert_eq!(t.cash, start);

        // A figure the table would not take is refused rather than quietly
        // rounded into one it would: a player who wants nothing there takes
        // it down, and the layout does not move meanwhile.
        assert_eq!(
            t.apply(Action::SetStake(BetRef::Place(6), Amount::Cents(1))),
            Err(RejectReason::BelowTableMinimum)
        );
        assert_eq!(t.place[2], 1200);
    }

    #[test]
    fn a_bet_turned_off_resolves_nothing() {
        // Calling a bet off is something only a compiled strategy can do, so
        // this needs the session type that compiles the working flags in —
        // the built-in player's type folds them away to a constant `true`.
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t: Session<'_, Spy, crate::strategy::AllFeatures> =
            Session::with_observer(&sel, &r, 1000, 100_000, false, Spy::default());
        t.point = Some(4);
        t.apply(Action::Bet(BetRef::Place(6), Amount::Base))
            .unwrap();
        t.apply(Action::Working(
            BetRef::Place(6),
            false,
            WorkingWhen::PointCycle,
        ))
        .unwrap();
        let cash = t.cash;

        t.resolve(3, 3); // the 6 hits, but the bet is off
        assert_eq!(t.cash, cash, "an off bet does not get paid");
        assert_eq!(t.place[2], 1200, "and it is still there");

        t.resolve(3, 4); // seven out
        assert_eq!(t.place[2], 1200, "and it does not lose either");
        assert_eq!(t.cash, cash);
    }

    #[test]
    fn leaving_is_recorded_for_the_session_loop() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 100_000);
        assert!(!t.leaving);
        assert_eq!(t.apply(Action::Leave), Ok(0));
        assert!(t.leaving);
    }

    #[test]
    fn one_roll_bets_are_legal_at_any_point_in_the_game() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 100_000);
        assert!(t
            .apply(Action::Bet(BetRef::Field, Amount::Units(1)))
            .is_ok());
        assert!(t
            .apply(Action::Bet(BetRef::AnySeven, Amount::Units(1)))
            .is_ok());

        let mut t = table(&sel, &r, 100_000);
        t.point = Some(10);
        assert!(t
            .apply(Action::Bet(BetRef::Field, Amount::Units(1)))
            .is_ok());
        assert!(t
            .apply(Action::Bet(BetRef::AnyCraps, Amount::Units(1)))
            .is_ok());
    }
}
