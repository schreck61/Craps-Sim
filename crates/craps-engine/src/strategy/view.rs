// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! What a strategy may read, and the derived history that answers it.
//!
//! [`TableView`] is the read-only borrow a strategy is handed at its
//! decision point. Most of what it reports is already sitting in the
//! session — the point, the bankroll, what is on the layout — and costs
//! nothing to expose. The rest is *derived history*: how many times the 6
//! has shown this shooter, how many place bets in a row have lost, how far
//! the bankroll is off its peak. Those need accumulators, and accumulators
//! in the session's hot path are exactly the kind of tax that would make
//! the language too expensive to use.
//!
//! So they are gated. A compiled strategy declares which groups it reads as
//! a [`FeatureMask`], and the session maintains those and nothing else. The
//! built-in player reads none of them, and pays for none of them — the
//! single-thread benchmark is the proof, not the intention.

use crate::bets::{hard_index, place_index};
use crate::strategy::BetRef;
use crate::trace::BetKind;

/// One independent betting stream, keyed exactly like the session's
/// progression state: pass, don't pass, come, don't come, field, the six
/// place numbers, the four hardways, any seven, any craps.
///
/// Odds share their flat's stream. A player who says "the pass line has lost
/// three in a row" means the line, not the line and its odds counted twice.
pub(crate) const STREAMS: usize = 17;

pub(crate) const S_PASS: usize = 0;
pub(crate) const S_DONT: usize = 1;
pub(crate) const S_COME: usize = 2;
pub(crate) const S_DC: usize = 3;
pub(crate) const S_FIELD: usize = 4;
/// First of the six place streams, in [`PLACE_NUMS`](crate::PLACE_NUMS) order.
pub(crate) const S_PLACE: usize = 5;
/// First of the four hardway streams, in [`HARD_NUMS`](crate::HARD_NUMS) order.
pub(crate) const S_HARD: usize = 11;
pub(crate) const S_ANY7: usize = 15;
pub(crate) const S_ANYCRAPS: usize = 16;

/// The stream a bet event belongs to, or `None` for odds (which resolve
/// with the flat they back) and for events that are not wins or losses.
#[inline]
pub(crate) fn stream_index(bet: BetKind) -> Option<usize> {
    Some(match bet {
        BetKind::Pass => 0,
        BetKind::DontPass => 1,
        BetKind::Come | BetKind::ComePoint(_) => 2,
        BetKind::DontCome | BetKind::DontComePoint(_) => 3,
        BetKind::Field => 4,
        BetKind::Place(n) => 5 + place_index(n)?,
        BetKind::Hardway(n) => 11 + hard_index(n)?,
        BetKind::AnySeven => 15,
        BetKind::AnyCraps => 16,
        BetKind::PassOdds
        | BetKind::DontPassLay
        | BetKind::ComeOdds(_)
        | BetKind::DontComeLay(_) => return None,
    })
}

/// The stream a strategy's bet reference names, for reading win and loss
/// history back out.
#[inline]
pub(crate) fn stream_of(bet: BetRef) -> Option<usize> {
    Some(match bet {
        BetRef::Pass | BetRef::PassOdds => 0,
        BetRef::DontPass | BetRef::DontPassLay => 1,
        BetRef::Come | BetRef::ComeOdds(_) => 2,
        BetRef::DontCome | BetRef::DontComeLay(_) => 3,
        BetRef::Field => 4,
        BetRef::Place(n) => 5 + place_index(n)?,
        BetRef::Hardway(n) => 11 + hard_index(n)?,
        BetRef::AnySeven => 15,
        BetRef::AnyCraps => 16,
    })
}

/// Which groups of derived history a strategy reads. A compiled program
/// declares this once; the session maintains exactly these.
///
/// Hand-rolled bit flags rather than a dependency — there are four of them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FeatureMask(u8);

impl FeatureMask {
    /// The mask as its bits, for a content hash that wants structure rather
    /// than the spelling of a `Debug` impl.
    #[inline]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Reads nothing derived. What the built-in player declares, and the
    /// state in which the accumulators cost nothing.
    pub const NONE: FeatureMask = FeatureMask(0);
    /// The roll counter, the last total, and which shooter this is.
    pub const DICE: FeatureMask = FeatureMask(1 << 0);
    /// Per-total hit counts, for the session and for this shooter.
    pub const HITS: FeatureMask = FeatureMask(1 << 1);
    /// Per-stream wins, losses, and the current run of either.
    pub const STREAKS: FeatureMask = FeatureMask(1 << 2);
    /// Peak wealth, and the drawdown from it.
    pub const PEAK: FeatureMask = FeatureMask(1 << 3);
    /// Everything — what an interpreted strategy declares when it has not
    /// been analyzed, and what the tests exercise.
    pub const ALL: FeatureMask = FeatureMask(0b1111);

    #[inline]
    pub const fn with(self, other: FeatureMask) -> FeatureMask {
        FeatureMask(self.0 | other.0)
    }

    #[inline]
    pub const fn has(self, feature: FeatureMask) -> bool {
        self.0 & feature.0 != 0
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// A session's *compile-time* upper bound on derived history: what it could
/// possibly maintain, decided when the session type is chosen rather than
/// when it runs.
///
/// This is the same trick [`RollObserver::WANTS_ROLL_END`](crate::trace)
/// plays. The runtime [`FeatureMask`] decides which groups a particular
/// strategy actually reads; this decides whether the checking code is
/// compiled at all. For the built-in player it is [`NoFeatures`], every
/// guard folds to `false`, and the accumulators vanish from the binary —
/// which matters because the per-event streak guard alone cost 2.7% on a
/// loaded table when it was a runtime test.
pub(crate) trait Features {
    const MASK: FeatureMask;
}

/// Maintains nothing. The built-in player, and the default.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoFeatures;

impl Features for NoFeatures {
    const MASK: FeatureMask = FeatureMask::NONE;
}

/// Compiles in every accumulator; the runtime mask then selects among them.
/// What an interpreted strategy uses, since its reads are data, not types.
#[derive(Clone, Copy, Debug, Default)]
pub struct AllFeatures;

impl Features for AllFeatures {
    const MASK: FeatureMask = FeatureMask::ALL;
}

/// The derived history itself. Zeroed at session start; every field is
/// maintained only while its group is in the session's [`FeatureMask`], so
/// reading a field whose group was not declared reports zero rather than a
/// stale or wrong number.
#[derive(Clone, Copy, Debug)]
pub(crate) struct History {
    /// The total of the last roll resolved (0 before the first).
    pub last_total: u8,
    /// Rolls resolved this session.
    pub roll: u64,
    /// Rolls resolved since this shooter took the dice.
    pub rolls_this_shooter: u32,
    /// How many shooters have held the dice, counting from 0.
    pub shooter: u32,
    /// Times each total has shown this session, indexed by the total.
    pub hits: [u16; 13],
    /// Times each total has shown since this shooter took the dice.
    pub hits_shooter: [u16; 13],
    pub wins: [u16; STREAMS],
    pub losses: [u16; STREAMS],
    /// Consecutive wins as a positive run, consecutive losses as a negative
    /// one; zero before a stream has resolved anything.
    pub streak: [i16; STREAMS],
    /// Highest total wealth seen, for the drawdown a stop-loss reads.
    pub peak_wealth: i64,
    /// Coarse triggers that fired on the roll just resolved, and the
    /// streams that won or lost on it. Cleared at the start of each roll;
    /// read once, at the decision point that follows.
    pub fired: u8,
    pub won: u32,
    pub lost: u32,
    /// What each stream was paid on the roll just resolved, in cents.
    /// Cleared every roll: "what did this bet just win" is a question about
    /// one roll, and a stale answer would be worse than none.
    pub paid: [i64; STREAMS],
    /// Box numbers a come (or don't come) flat travelled to on the roll just
    /// resolved — a come point being established, which is a different
    /// event from the table's point being established and had no way to be
    /// named before.
    pub come_established: u8,
    pub dont_come_established: u8,
    /// The total of the roll just resolved. Distinct from `last_total`,
    /// which only exists when a strategy declared it wanted dice history;
    /// a `total(7)` trigger needs the number whether or not it also wanted
    /// the counter.
    pub last_total_now: u8,
}

impl History {
    pub(crate) fn new(start_cash: i64) -> Self {
        Self {
            last_total: 0,
            roll: 0,
            rolls_this_shooter: 0,
            shooter: 0,
            hits: [0; 13],
            hits_shooter: [0; 13],
            wins: [0; STREAMS],
            losses: [0; STREAMS],
            streak: [0; STREAMS],
            peak_wealth: start_cash,
            fired: 0,
            won: 0,
            lost: 0,
            paid: [0; STREAMS],
            come_established: 0,
            dont_come_established: 0,
            last_total_now: 0,
        }
    }

    #[inline]
    pub(crate) fn record_win(&mut self, bet: BetKind, paid_cents: i64) {
        if let Some(i) = stream_index(bet) {
            self.wins[i] = self.wins[i].saturating_add(1);
            self.streak[i] = self.streak[i].max(0).saturating_add(1);
            self.won |= 1 << i;
            self.paid[i] = self.paid[i].saturating_add(paid_cents);
        }
    }

    /// A come or don't come flat reached a box number.
    #[inline]
    pub(crate) fn record_travel(&mut self, bet: BetKind, to: u8) {
        let Some(i) = place_index(to) else { return };
        match bet {
            BetKind::Come => self.come_established |= 1 << i,
            BetKind::DontCome => self.dont_come_established |= 1 << i,
            _ => {}
        }
    }

    #[inline]
    pub(crate) fn record_loss(&mut self, bet: BetKind) {
        if let Some(i) = stream_index(bet) {
            self.losses[i] = self.losses[i].saturating_add(1);
            self.streak[i] = self.streak[i].min(0).saturating_sub(1);
            self.lost |= 1 << i;
        }
    }
}

/// Everything a strategy may read at its decision point.
///
/// Constructed once per decision by [`Session::view`](crate::game), never by
/// the built-in player, which reads the session directly. Money is cents,
/// counts are counts; there is no floating point here, because a strategy
/// that could compute $12.333… would break payout exactness.
pub struct TableView<'s> {
    pub(crate) point: Option<u8>,
    pub(crate) cash: i64,
    pub(crate) start_cash: i64,
    /// The table's own numbers. A strategy that wants to say "half the
    /// buy-in" or "reset when the next step would not be taken" had to
    /// hard-code them, which made the sentence true at one table and quietly
    /// wrong at every other.
    pub(crate) table_min: i64,
    pub(crate) table_max: i64,
    pub(crate) handle: i64,
    pub(crate) stakes: Stakes<'s>,
    pub(crate) hist: &'s History,
}

/// What is on the layout right now, borrowed rather than copied.
pub(crate) struct Stakes<'s> {
    pub pass: i64,
    pub pass_odds: i64,
    pub dont: i64,
    pub dont_lay: i64,
    pub come_flat: i64,
    pub dc_flat: i64,
    pub place: &'s [i64; 6],
    pub hard: &'s [i64; 4],
    pub place_working: &'s [bool; 6],
    pub hard_working: &'s [bool; 4],
    pub come_points: &'s [i64; 6],
    pub come_odds: &'s [i64; 6],
    pub dc_points: &'s [i64; 6],
    pub dc_lay: &'s [i64; 6],
    pub field: i64,
    pub any7: i64,
    pub anycraps: i64,
}

impl TableView<'_> {
    // --- The table and the night ---

    /// What the player sat down with. A stop-loss written as a fraction of
    /// the buy-in travels between tables; one written in dollars does not.
    #[inline]
    pub fn buy_in(&self) -> i64 {
        self.start_cash
    }

    /// Whether a bet is working. Only place bets and hardways can be called
    /// off; everything else is working whenever it is up, so this answers
    /// the same question `up` does for them.
    #[inline]
    pub fn working(&self, bet: BetRef) -> i64 {
        let on = match bet {
            BetRef::Place(n) => crate::place_index(n)
                .is_some_and(|i| self.stakes.place[i] > 0 && self.stakes.place_working[i]),
            BetRef::Hardway(n) => crate::hard_index(n)
                .is_some_and(|i| self.stakes.hard[i] > 0 && self.stakes.hard_working[i]),
            other => self.up(other),
        };
        on as i64
    }

    /// The table minimum, in cents.
    #[inline]
    pub fn table_min(&self) -> i64 {
        self.table_min
    }

    /// The table maximum, in cents — the ceiling every progression meets
    /// eventually, and the one a Martingale has to see coming if it is to
    /// reset instead of pushing into a bet the table will truncate.
    #[inline]
    pub fn table_max(&self) -> i64 {
        self.table_max
    }

    // --- Dice and point ---

    /// The point, or 0 when the game is on a come-out. Zero rather than an
    /// `Option` because the language has one scalar type.
    #[inline]
    pub fn point(&self) -> i64 {
        self.point.unwrap_or(0) as i64
    }

    /// True on a come-out roll.
    #[inline]
    pub fn come_out(&self) -> bool {
        self.point.is_none()
    }

    /// The total of the last roll resolved; 0 before the first.
    #[inline]
    pub fn last_total(&self) -> i64 {
        self.hist.last_total as i64
    }

    /// Rolls resolved this session.
    #[inline]
    pub fn roll(&self) -> i64 {
        self.hist.roll as i64
    }

    /// Rolls resolved since this shooter took the dice.
    #[inline]
    pub fn rolls_this_shooter(&self) -> i64 {
        self.hist.rolls_this_shooter as i64
    }

    /// How many shooters have held the dice, counting from 0.
    #[inline]
    pub fn shooter(&self) -> i64 {
        self.hist.shooter as i64
    }

    // --- Money ---

    /// Cash in hand, in cents. Money on the layout is not cash.
    #[inline]
    pub fn cash(&self) -> i64 {
        self.cash
    }

    /// Cash plus the face value of every live bet — what the player would
    /// leave with if they picked everything up right now.
    #[inline]
    pub fn wealth(&self) -> i64 {
        self.cash + self.on_table_face()
    }

    /// Wealth less what the player sat down with. Negative is losing.
    #[inline]
    pub fn profit(&self) -> i64 {
        self.wealth() - self.start_cash
    }

    /// The best profit seen this session.
    #[inline]
    pub fn peak_profit(&self) -> i64 {
        self.hist.peak_wealth - self.start_cash
    }

    /// How far off the peak the player is now, as a positive number.
    #[inline]
    pub fn drawdown(&self) -> i64 {
        (self.hist.peak_wealth - self.wealth()).max(0)
    }

    /// Total stake resolved by wins and losses — the handle the house edge
    /// applies to. Pushes are excluded, per the convention in `edge.rs`.
    #[inline]
    pub fn handle(&self) -> i64 {
        self.handle
    }

    // --- Layout ---

    /// Whether this bet is up right now.
    #[inline]
    pub fn up(&self, bet: BetRef) -> bool {
        self.stake(bet) > 0
    }

    /// What is on this bet, in cents.
    pub fn stake(&self, bet: BetRef) -> i64 {
        let s = &self.stakes;
        match bet {
            BetRef::Pass => s.pass,
            BetRef::PassOdds => s.pass_odds,
            BetRef::DontPass => s.dont,
            BetRef::DontPassLay => s.dont_lay,
            // A come flat in its box and one established on a number are the
            // same bet at different moments; the strategy asks about both.
            BetRef::Come => s.come_flat + s.come_points.iter().sum::<i64>(),
            BetRef::DontCome => s.dc_flat + s.dc_points.iter().sum::<i64>(),
            BetRef::ComeOdds(n) => at(s.come_odds, n),
            BetRef::DontComeLay(n) => at(s.dc_lay, n),
            BetRef::Place(n) => at(s.place, n),
            BetRef::Hardway(n) => hard_index(n).map_or(0, |i| s.hard[i]),
            BetRef::Field => s.field,
            BetRef::AnySeven => s.any7,
            BetRef::AnyCraps => s.anycraps,
        }
    }

    /// What sits in the slot the table would write if this bet were made
    /// now — which is not always what [`TableView::stake`] reports.
    ///
    /// `stake(Come)` answers the question a player asks: how much do I have
    /// riding on come, box and numbers together. The table's slot is the
    /// come box alone, because a flat established on a number no longer
    /// blocks a new one. Betting idempotence is about the slot, so the
    /// interpreter's already-up check uses this and nothing else.
    pub(crate) fn slot_stake(&self, bet: BetRef) -> i64 {
        match bet {
            BetRef::Come => self.stakes.come_flat,
            BetRef::DontCome => self.stakes.dc_flat,
            other => self.stake(other),
        }
    }

    // The three below walk the layout, so they are computed when asked and
    // not before. A view is built for every decision of every session, and
    // a strategy that never asks what its whole layout is worth should not
    // pay for the sum.

    /// Come bets working, counting one still in the come box.
    #[inline]
    pub fn live_come(&self) -> i64 {
        let s = &self.stakes;
        (s.come_flat > 0) as i64 + s.come_points.iter().filter(|&&b| b > 0).count() as i64
    }

    /// The come flat established on this number, 0 if none. Distinct from
    /// the odds behind it, which `stake(ComeOdds(n))` reports.
    #[inline]
    pub fn come_point(&self, num: u8) -> i64 {
        at(self.stakes.come_points, num)
    }

    /// The don't come flat established on this number, 0 if none.
    #[inline]
    pub fn dont_come_point(&self, num: u8) -> i64 {
        at(self.stakes.dc_points, num)
    }

    /// Don't come bets working, same convention.
    #[inline]
    pub fn live_dont_come(&self) -> i64 {
        let s = &self.stakes;
        (s.dc_flat > 0) as i64 + s.dc_points.iter().filter(|&&b| b > 0).count() as i64
    }

    /// Face value of everything on the layout.
    pub fn on_table_face(&self) -> i64 {
        let s = &self.stakes;
        s.pass
            + s.pass_odds
            + s.dont
            + s.dont_lay
            + s.come_flat
            + s.dc_flat
            + s.field
            + s.any7
            + s.anycraps
            + s.place.iter().sum::<i64>()
            + s.hard.iter().sum::<i64>()
            + s.come_points.iter().sum::<i64>()
            + s.come_odds.iter().sum::<i64>()
            + s.dc_points.iter().sum::<i64>()
            + s.dc_lay.iter().sum::<i64>()
    }

    // --- Derived history ---
    //
    // Each reports 0 unless its feature group was declared, which is why a
    // compiled program's mask is derived from the reads it actually makes.

    /// Times this total has shown this session.
    #[inline]
    pub fn hits(&self, total: u8) -> i64 {
        self.hist.hits.get(total as usize).copied().unwrap_or(0) as i64
    }

    /// Times this total has shown since the shooter took the dice.
    #[inline]
    pub fn hits_this_shooter(&self, total: u8) -> i64 {
        self.hist
            .hits_shooter
            .get(total as usize)
            .copied()
            .unwrap_or(0) as i64
    }

    /// How many times this stream has won.
    #[inline]
    pub fn wins(&self, bet: BetRef) -> i64 {
        stream_of(bet).map_or(0, |i| self.hist.wins[i] as i64)
    }

    /// How many times this stream has lost.
    #[inline]
    pub fn losses(&self, bet: BetRef) -> i64 {
        stream_of(bet).map_or(0, |i| self.hist.losses[i] as i64)
    }

    /// The current run: positive for consecutive wins, negative for
    /// consecutive losses, zero before the stream has resolved anything.
    #[inline]
    pub fn streak(&self, bet: BetRef) -> i64 {
        stream_of(bet).map_or(0, |i| self.hist.streak[i] as i64)
    }

    /// What this stream was paid on the roll just resolved, in cents — the
    /// winnings, not the stake. Zero on any roll it did not win.
    ///
    /// This is what makes "press it up by half of what it paid" sayable as
    /// a rule; before it, that press existed only as a progression.
    #[inline]
    pub fn paid(&self, bet: BetRef) -> i64 {
        stream_of(bet).map_or(0, |i| self.hist.paid[i])
    }
}

#[inline]
fn at(arr: &[i64; 6], num: u8) -> i64 {
    place_index(num).map_or(0, |i| arr[i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bets::{BetSelection, OddsPolicy, Rules};
    use crate::game::Session;
    use crate::trace::Noop;

    fn rules() -> Rules {
        Rules {
            odds_policy: OddsPolicy::None,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 1000,
        }
    }

    /// A $10 table with $1,000 in hand. The session type compiles in every
    /// accumulator; `features` then selects which are actually kept, which
    /// is exactly the arrangement an interpreted strategy runs under.
    fn table<'a>(
        sel: &'a BetSelection,
        r: &'a Rules,
        features: FeatureMask,
    ) -> Session<'a, Noop, AllFeatures> {
        let mut s: Session<'a, Noop, AllFeatures> =
            Session::with_observer(sel, r, 1000, 100_000, false, Noop);
        s.features = features;
        s
    }

    /// Play one roll the way the session loop does: place, then resolve.
    fn play(s: &mut Session<'_, Noop, AllFeatures>, d1: u8, d2: u8) {
        s.place_bets();
        s.resolve(d1, d2);
    }

    #[test]
    fn hits_count_per_session_and_per_shooter() {
        let (sel, r) = (BetSelection::default(), rules());
        let mut s = table(&sel, &r, FeatureMask::ALL);
        play(&mut s, 3, 3); // 6 — point established
        play(&mut s, 2, 4); // 6 again — point made, same shooter
        assert_eq!(s.view().hits(6), 2);
        assert_eq!(s.view().hits_this_shooter(6), 2);
        assert_eq!(s.view().shooter(), 0);

        play(&mut s, 3, 3); // 6 — a new point
        play(&mut s, 3, 4); // 7 — seven out, dice pass
        assert_eq!(s.view().hits(6), 3, "the session's count keeps running");
        assert_eq!(
            s.view().hits_this_shooter(6),
            0,
            "the new shooter starts from nothing"
        );
        assert_eq!(s.view().shooter(), 1);
        assert_eq!(s.view().rolls_this_shooter(), 0);
        assert_eq!(s.view().roll(), 4);
        assert_eq!(s.view().last_total(), 7);
    }

    #[test]
    fn streaks_run_positive_on_wins_and_negative_on_losses() {
        let (sel, r) = (BetSelection::default(), rules()); // pass line
        let mut s = table(&sel, &r, FeatureMask::ALL);
        assert_eq!(s.view().streak(BetRef::Pass), 0, "nothing resolved yet");

        play(&mut s, 3, 4); // 7 on the come-out — pass wins
        play(&mut s, 5, 6); // 11 — pass wins again
        assert_eq!(s.view().streak(BetRef::Pass), 2);
        assert_eq!(s.view().wins(BetRef::Pass), 2);

        play(&mut s, 1, 1); // 2 — craps, pass loses
        assert_eq!(
            s.view().streak(BetRef::Pass),
            -1,
            "a loss flips the run, it does not decrement it"
        );
        assert_eq!(s.view().losses(BetRef::Pass), 1);
        assert_eq!(s.view().wins(BetRef::Pass), 2, "wins are not forgotten");
    }

    #[test]
    fn profit_peak_and_drawdown_track_the_bankroll() {
        let (sel, r) = (BetSelection::default(), rules());
        let mut s = table(&sel, &r, FeatureMask::ALL);
        assert_eq!(s.view().profit(), 0);

        play(&mut s, 3, 4); // pass wins $10
        assert_eq!(s.view().profit(), 1000);
        assert_eq!(s.view().peak_profit(), 1000);
        assert_eq!(s.view().drawdown(), 0);

        play(&mut s, 1, 1); // craps, pass loses $10
        assert_eq!(s.view().profit(), 0);
        assert_eq!(s.view().peak_profit(), 1000, "the peak is remembered");
        assert_eq!(s.view().drawdown(), 1000);
    }

    #[test]
    fn an_undeclared_feature_reports_zero_rather_than_a_wrong_number() {
        // The point of the mask: a strategy that never asked for hit counts
        // does not pay for them, and must not be able to read a half-kept
        // one either.
        let (sel, r) = (BetSelection::default(), rules());
        let mut s = table(&sel, &r, FeatureMask::NONE);
        play(&mut s, 3, 3); // 6 — point established
        play(&mut s, 2, 3); // 5 — nothing happens, the point stands
        assert_eq!(s.view().hits(6), 0);
        assert_eq!(s.view().roll(), 0);
        assert_eq!(s.view().streak(BetRef::Pass), 0);
        assert_eq!(s.view().peak_profit(), 0);
        // What is not derived is always available: it is simply the session.
        assert_eq!(s.view().point(), 6);
        assert!(s.view().up(BetRef::Pass));
    }

    #[test]
    fn features_are_maintained_one_group_at_a_time() {
        let (sel, r) = (BetSelection::default(), rules());
        let mut s = table(&sel, &r, FeatureMask::HITS);
        play(&mut s, 3, 3);
        assert_eq!(s.view().hits(6), 1, "declared");
        assert_eq!(s.view().roll(), 0, "DICE was not declared");
        assert_eq!(s.view().streak(BetRef::Pass), 0, "STREAKS was not declared");
    }

    #[test]
    fn the_layout_reads_back_what_is_on_it() {
        let mut sel = BetSelection {
            pass_line: true,
            take_odds: true,
            ..Default::default()
        };
        sel.come_max = 1;
        sel.set_place(8, true);
        let r = Rules {
            odds_policy: OddsPolicy::X345,
            ..rules()
        };
        let mut s = table(&sel, &r, FeatureMask::ALL);
        play(&mut s, 2, 2); // point 4
        s.place_bets();

        let v = s.view();
        assert_eq!(v.point(), 4);
        assert!(!v.come_out());
        assert_eq!(v.stake(BetRef::Pass), 1000);
        assert_eq!(v.stake(BetRef::PassOdds), 3000, "3x behind the 4");
        assert_eq!(v.stake(BetRef::Place(8)), 1200, "$6 units on the 8");
        assert!(v.up(BetRef::Come), "a flat waiting in the come box");
        assert_eq!(v.live_come(), 1);
        assert!(!v.up(BetRef::Place(4)), "the point is never placed");
        assert_eq!(v.wealth(), v.cash() + v.on_table_face());
    }
}
