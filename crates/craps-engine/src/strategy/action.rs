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
use crate::trace::{BetEventKind, BetKind, RollObserver};

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
}

/// Why the table refused an action. Every one of these is emitted as a
/// [`BetEventKind::Rejected`] event, because a strategy that silently does
/// nothing is the worst thing this surface could allow (Principle 4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RejectReason {
    /// Not legal at this moment: a line bet with a point on, a come bet on
    /// the come-out, a place bet on the current point, odds with no flat
    /// behind them.
    NotAllowedNow,
    /// The odds policy allows nothing behind this point.
    NoOddsAllowed,
    /// Neither the requested stake nor the base fallback fit the bankroll.
    InsufficientBankroll,
}

impl RejectReason {
    /// Why the table said no, in the words the Replay ledger and the Bench
    /// show the user. Plain language, no jargon, no error codes.
    pub fn label(&self) -> &'static str {
        match self {
            RejectReason::NotAllowedNow => "not allowed right now",
            RejectReason::NoOddsAllowed => "odds policy allows none",
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
            Action::Bet(bet, amount) => self.apply_bet(bet, amount),
        }
    }

    #[inline]
    fn apply_bet(&mut self, bet: BetRef, amount: Amount) -> Adjudication {
        if is_odds(bet) {
            return if self.odds_flat_is_up(bet) {
                self.apply_odds(bet, amount)
            } else {
                self.reject(bet, RejectReason::NotAllowedNow)
            };
        }
        let Some(spec) = self.flat_spec(bet) else {
            return self.reject(bet, RejectReason::NotAllowedNow);
        };
        if *self.slot(spec.slot) != 0 {
            return Ok(0); // already up; `Bet` is idempotent
        }
        let want = self.resolve_amount(bet, amount, spec.base)?;
        match self.try_stake_or_base(want, spec.base) {
            Some(a) => {
                *self.slot_mut(spec.slot) = a;
                self.emit(bet_kind(bet), BetEventKind::Placed, a);
                Ok(a)
            }
            None => self.reject(bet, RejectReason::InsufficientBankroll),
        }
    }

    /// Resolve a flat bet to its slot and base stake, or `None` when the
    /// game state does not permit it right now. These are table rules; a
    /// strategy's own conditions (how many come bets it wants, which numbers
    /// it places) are not expressed here.
    #[inline]
    fn flat_spec(&self, bet: BetRef) -> Option<FlatSpec> {
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
        let allowed = match bet {
            BetRef::Pass | BetRef::DontPass => self.point.is_none(),
            BetRef::Come | BetRef::DontCome | BetRef::Hardway(_) => self.point.is_some(),
            // The point number is never placed, and place bets are made only
            // while a point is on.
            BetRef::Place(n) => self.point.is_some_and(|p| p != n),
            // One-roll bets work on every roll.
            _ => true,
        };
        allowed.then_some(FlatSpec { slot, base })
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
                    None => self.reject(bet, RejectReason::InsufficientBankroll),
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
                    None => self.reject(bet, RejectReason::InsufficientBankroll),
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
                    None => self.reject(bet, RejectReason::InsufficientBankroll),
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
                    None => self.reject(bet, RejectReason::InsufficientBankroll),
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

    /// Cents for an [`Amount`], rounded up to the bet's payout unit so every
    /// payout stays whole (Principle: money is integer cents end to end).
    /// Cents for an [`Amount`], rounded up to the bet's payout unit so every
    /// payout stays whole — money is integer cents end to end, and a stake
    /// that could pay $12.333… is not a stake this table takes.
    #[inline]
    fn resolve_amount(&self, bet: BetRef, amount: Amount, base: i64) -> Adjudication {
        let raw = match amount {
            Amount::Base => return Ok(base),
            Amount::Cents(c) => c,
            Amount::Units(n) => n.saturating_mul(self.min),
            // Only reachable when a strategy asks for max odds on a bet that
            // takes none; the caller has already handled the odds bets.
            Amount::MaxOdds => return Err(RejectReason::NoOddsAllowed),
        };
        Ok(match bet {
            BetRef::Place(n) => round_up(raw, place_unit_of(n)),
            _ => raw,
        })
    }

    #[inline]
    fn reject(&mut self, bet: BetRef, reason: RejectReason) -> Adjudication {
        self.emit(bet_kind(bet), BetEventKind::Rejected { reason }, 0);
        Err(reason)
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
                BetEventKind::Rejected { reason } => Some(reason),
                _ => None,
            })
            .collect()
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
        assert_eq!(
            t.apply(Action::Bet(BetRef::Pass, Amount::Units(1))),
            Err(RejectReason::NotAllowedNow)
        );
        assert_eq!(t.pass, 0);
        assert_eq!(t.cash, 100_000, "a refused bet costs nothing");
        assert_eq!(refusals(&t), vec![RejectReason::NotAllowedNow]);
    }

    #[test]
    fn come_bets_are_refused_on_the_come_out() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 100_000);
        assert_eq!(
            t.apply(Action::Bet(BetRef::Come, Amount::Units(1))),
            Err(RejectReason::NotAllowedNow)
        );
    }

    #[test]
    fn the_point_number_is_never_placed() {
        let (sel, r) = (BetSelection::default(), rules(OddsPolicy::None));
        let mut t = table(&sel, &r, 100_000);
        t.point = Some(6);
        assert_eq!(
            t.apply(Action::Bet(BetRef::Place(6), Amount::Units(1))),
            Err(RejectReason::NotAllowedNow)
        );
        // Any other box number is fine.
        assert!(t
            .apply(Action::Bet(BetRef::Place(8), Amount::Units(1)))
            .is_ok());
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
            t.apply(Action::Bet(BetRef::Place(6), Amount::Cents(1000))),
            Ok(1200)
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
