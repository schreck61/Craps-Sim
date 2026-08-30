// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Bet definitions: selections, table rules, payout arithmetic, and betting
//! progressions.

pub const PLACE_NUMS: [u8; 6] = [4, 5, 6, 8, 9, 10];
pub const HARD_NUMS: [u8; 4] = [4, 6, 8, 10];

/// Index of a place number in the `[_; 6]` arrays keyed like [`PLACE_NUMS`].
#[inline]
pub const fn place_index(num: u8) -> Option<usize> {
    match num {
        4 => Some(0),
        5 => Some(1),
        6 => Some(2),
        8 => Some(3),
        9 => Some(4),
        10 => Some(5),
        _ => None,
    }
}

/// Index of a hardway number in the `[_; 4]` arrays keyed like [`HARD_NUMS`].
#[inline]
pub const fn hard_index(num: u8) -> Option<usize> {
    match num {
        4 => Some(0),
        6 => Some(1),
        8 => Some(2),
        10 => Some(3),
        _ => None,
    }
}

/// The stake unit (in cents) a place bet must be a multiple of so its payout
/// is a whole number of cents: $6 units on 6/8 (7:6), $5 units elsewhere.
#[inline]
pub(crate) const fn place_unit(num: u8) -> i64 {
    match num {
        6 | 8 => 600,
        _ => 500,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum OddsPolicy {
    None,
    X1,
    X2,
    X345,
    X5,
    X10,
}

impl OddsPolicy {
    pub const ALL: [OddsPolicy; 6] = [
        OddsPolicy::None,
        OddsPolicy::X1,
        OddsPolicy::X2,
        OddsPolicy::X345,
        OddsPolicy::X5,
        OddsPolicy::X10,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            OddsPolicy::None => "No odds",
            OddsPolicy::X1 => "1x odds",
            OddsPolicy::X2 => "2x odds",
            OddsPolicy::X345 => "3-4-5x odds",
            OddsPolicy::X5 => "5x odds",
            OddsPolicy::X10 => "10x odds",
        }
    }

    /// Max odds multiple for a given point.
    pub(crate) fn multiple(&self, point: u8) -> i64 {
        match self {
            OddsPolicy::None => 0,
            OddsPolicy::X1 => 1,
            OddsPolicy::X2 => 2,
            OddsPolicy::X5 => 5,
            OddsPolicy::X10 => 10,
            OddsPolicy::X345 => match point {
                4 | 10 => 3,
                5 | 9 => 4,
                _ => 5, // 6 | 8
            },
        }
    }
}

/// Betting progression ("pressing") systems. Each recurring bet stream (the
/// pass line, each place number, the field, …) carries its own independent
/// progression state; odds are sized from the pressed flat bet automatically.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Progression {
    Flat,
    FullPress,
    HalfPress,
    PressAndPull,
    Paroli3,
    S1326,
    Martingale,
    GrandMartingale,
    DAlembert,
    ReverseDAlembert,
    Fibonacci,
    OscarsGrind,
}

impl Progression {
    pub const ALL: [Progression; 12] = [
        Progression::Flat,
        Progression::FullPress,
        Progression::HalfPress,
        Progression::PressAndPull,
        Progression::Paroli3,
        Progression::S1326,
        Progression::Martingale,
        Progression::GrandMartingale,
        Progression::DAlembert,
        Progression::ReverseDAlembert,
        Progression::Fibonacci,
        Progression::OscarsGrind,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Progression::Flat => "Flat (no press)",
            Progression::FullPress => "Full press",
            Progression::HalfPress => "Half press",
            Progression::PressAndPull => "Press and pull",
            Progression::Paroli3 => "Paroli (3-win)",
            Progression::S1326 => "1-3-2-6",
            Progression::Martingale => "Martingale",
            Progression::GrandMartingale => "Grand Martingale",
            Progression::DAlembert => "D'Alembert",
            Progression::ReverseDAlembert => "Reverse D'Alembert",
            Progression::Fibonacci => "Fibonacci",
            Progression::OscarsGrind => "Oscar's Grind",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Progression::Flat => "Same bet every time.",
            Progression::FullPress => "Double the bet after every win; back to base after a loss.",
            Progression::HalfPress => "Add half the bet after every win; back to base after a loss.",
            Progression::PressAndPull => "Press to 2x after the first win, pull back to base (collecting) after the second.",
            Progression::Paroli3 => "Let winnings ride, doubling up to 3 wins in a row, then reset. Loss resets.",
            Progression::S1326 => "Bet 1, 3, 2, 6 units on consecutive wins; any loss or completing the cycle resets.",
            Progression::Martingale => "Double after every loss to recover; win resets to base. Capped by the table max.",
            Progression::GrandMartingale => "Double plus one base unit after every loss; win resets to base.",
            Progression::DAlembert => "Up one base unit after a loss, down one after a win.",
            Progression::ReverseDAlembert => "Up one base unit after a win, down one after a loss.",
            Progression::Fibonacci => "After a loss move one step up the Fibonacci sequence of units; after a win move two steps back.",
            Progression::OscarsGrind => "Grind out one base unit per series: raise by one unit after wins (never betting more than needed), hold after losses, reset when the series is up one unit.",
        }
    }
}

/// Per-bet-stream progression state.
#[derive(Clone, Copy, Debug)]
pub struct ProgState {
    /// Next stake to bet, in cents (before table-max clamping / unit rounding).
    pub stake: i64,
    /// Sequence position (consecutive wins for Paroli/1-3-2-6, Fibonacci index).
    pub(crate) step: u32,
    /// Net profit of the current series, used by Oscar's Grind.
    pub(crate) profit: i64,
}

impl ProgState {
    pub(crate) fn new(base: i64) -> Self {
        Self {
            stake: base,
            step: 0,
            profit: 0,
        }
    }
}

pub(crate) fn fib_units(n: u32) -> i64 {
    let (mut a, mut b) = (1i64, 1i64);
    for _ in 0..n {
        let c = a + b;
        a = b;
        b = c;
    }
    a
}

// Progression stakes can't run away past this even before table-max clamping.
pub(crate) const PROG_STAKE_CAP: i64 = 1 << 40;

impl Progression {
    pub(crate) fn on_win(&self, st: &mut ProgState, base: i64, won: i64) {
        st.profit += won;
        match self {
            Progression::Flat => {}
            Progression::FullPress => st.stake = (st.stake * 2).min(PROG_STAKE_CAP),
            Progression::HalfPress => st.stake = (st.stake + st.stake / 2).min(PROG_STAKE_CAP),
            Progression::PressAndPull => {
                if st.step == 0 {
                    st.stake = base * 2;
                    st.step = 1;
                } else {
                    st.stake = base;
                    st.step = 0;
                }
            }
            Progression::Paroli3 => {
                st.step += 1;
                if st.step >= 3 {
                    st.stake = base;
                    st.step = 0;
                } else {
                    st.stake *= 2;
                }
            }
            Progression::S1326 => {
                const SEQ: [i64; 4] = [1, 3, 2, 6];
                st.step += 1;
                if st.step >= 4 {
                    st.step = 0;
                }
                st.stake = base * SEQ[st.step as usize];
            }
            Progression::Martingale | Progression::GrandMartingale => {
                st.stake = base;
            }
            Progression::DAlembert => st.stake = (st.stake - base).max(base),
            Progression::ReverseDAlembert => st.stake = (st.stake + base).min(PROG_STAKE_CAP),
            Progression::Fibonacci => {
                st.step = st.step.saturating_sub(2);
                st.stake = base * fib_units(st.step);
            }
            Progression::OscarsGrind => {
                if st.profit >= base {
                    // Series goal reached: one base unit of profit.
                    st.stake = base;
                    st.profit = 0;
                } else {
                    // Raise one unit, but never bet more than needed to close
                    // the series (even-money approximation for odd payouts).
                    let needed = base - st.profit;
                    st.stake = (st.stake + base).min(needed.max(base));
                }
            }
        }
    }

    pub(crate) fn on_loss(&self, st: &mut ProgState, base: i64, lost: i64) {
        st.profit -= lost;
        match self {
            Progression::Flat => {}
            Progression::FullPress
            | Progression::HalfPress
            | Progression::PressAndPull
            | Progression::Paroli3
            | Progression::S1326 => {
                st.stake = base;
                st.step = 0;
            }
            Progression::Martingale => st.stake = (st.stake * 2).min(PROG_STAKE_CAP),
            Progression::GrandMartingale => st.stake = (st.stake * 2 + base).min(PROG_STAKE_CAP),
            Progression::DAlembert => st.stake = (st.stake + base).min(PROG_STAKE_CAP),
            Progression::ReverseDAlembert => st.stake = (st.stake - base).max(base),
            Progression::Fibonacci => {
                st.step = (st.step + 1).min(40);
                st.stake = (base * fib_units(st.step)).min(PROG_STAKE_CAP);
            }
            Progression::OscarsGrind => {} // stake unchanged after a loss
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BetSelection {
    pub pass_line: bool,
    pub dont_pass: bool,
    /// Come bets: keep making them (when a point is on) until this many are
    /// live at once (traveling or on a number). 0 = no come bets.
    pub come_max: u8,
    /// Don't come bets, same convention.
    pub dont_come_max: u8,
    pub take_odds: bool,
    pub field: bool,
    pub place: [bool; 6],    // indexed like PLACE_NUMS: 4,5,6,8,9,10
    pub hardways: [bool; 4], // indexed like HARD_NUMS: 4,6,8,10
    pub any_seven: bool,
    pub any_craps: bool,
    pub progression: Progression,
}

impl Default for BetSelection {
    fn default() -> Self {
        Self {
            pass_line: true,
            dont_pass: false,
            come_max: 0,
            dont_come_max: 0,
            take_odds: false,
            field: false,
            place: [false; 6],
            hardways: [false; 4],
            any_seven: false,
            any_craps: false,
            progression: Progression::Flat,
        }
    }
}

impl BetSelection {
    /// Select or clear a place bet by its number (4, 5, 6, 8, 9, or 10)
    /// instead of by raw array index.
    ///
    /// # Panics
    ///
    /// Panics if `num` is not a place number.
    pub fn set_place(&mut self, num: u8, on: bool) {
        self.place[place_index(num).expect("not a place number")] = on;
    }

    pub fn any_selected(&self) -> bool {
        self.pass_line
            || self.dont_pass
            || self.come_max > 0
            || self.dont_come_max > 0
            || self.field
            || self.place.iter().any(|&b| b)
            || self.hardways.iter().any(|&b| b)
            || self.any_seven
            || self.any_craps
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Rules {
    pub odds_policy: OddsPolicy,
    /// true: field 12 pays 3:1 (2 always pays 2:1); false: 12 pays 2:1
    pub field_12_triple: bool,
    /// true: come odds stay working on come-out rolls; false (standard): come
    /// odds are off on come-out and returned unresolved. Don't come lay odds
    /// always work.
    pub come_odds_work_on_comeout: bool,
    /// Stake for hardways and one-roll props, in cents.
    pub prop_bet_cents: i64,
    /// Table maximum bet, as a multiple of the table minimum. Progressions
    /// (Martingale especially) are clamped here, as at a real table.
    pub table_max_mult: i64,
    /// true: the box number that is the point may be placed as well as
    /// covered by the line, which is what a real table will usually sell
    /// you; false (this engine's long-standing behaviour): it is refused.
    ///
    /// A flag rather than a change, because it moves money in every session
    /// that touches it — five of the twelve strategies written against this
    /// language in review wanted to place the point, and every one of them
    /// would have played differently under a table that allowed it.
    pub place_the_point: bool,
}

/// Cheapest base stake among the selected recurring bets at a given table
/// minimum — the amount below which a bankroll (with no live bets) is ruined.
/// Returns `i64::MAX` when no bet is selected.
pub fn cheapest_selected_stake(sel: &BetSelection, rules: &Rules, table_min_cents: i64) -> i64 {
    let mut cheapest = i64::MAX;
    if sel.pass_line || sel.dont_pass || sel.field || sel.come_max > 0 || sel.dont_come_max > 0 {
        cheapest = cheapest.min(table_min_cents);
    }
    for (i, &on) in sel.place.iter().enumerate() {
        if on {
            cheapest = cheapest.min(place_stake(table_min_cents, PLACE_NUMS[i]));
        }
    }
    if sel.hardways.iter().any(|&b| b) || sel.any_seven || sel.any_craps {
        cheapest = cheapest.min(rules.prop_bet_cents);
    }
    cheapest
}

#[inline]
pub(crate) fn round_up(amount: i64, unit: i64) -> i64 {
    ((amount + unit - 1) / unit) * unit
}

/// Place bets must be sized so payouts are whole: 6/8 in $6 units, others in $5 units.
pub(crate) fn place_stake(table_min_cents: i64, num: u8) -> i64 {
    round_up(table_min_cents, place_unit(num))
}

pub(crate) fn place_win(stake: i64, num: u8) -> i64 {
    match num {
        4 | 10 => stake * 9 / 5,
        5 | 9 => stake * 7 / 5,
        _ => stake * 7 / 6, // 6 | 8
    }
}

/// True-odds win for a pass odds stake on a given point.
pub(crate) fn pass_odds_win(stake: i64, point: u8) -> i64 {
    match point {
        4 | 10 => stake * 2,
        5 | 9 => stake * 3 / 2,
        _ => stake * 6 / 5, // 6 | 8
    }
}

/// For don't pass lay odds: amount to lay so that the lay wins `win` cents.
pub(crate) fn dont_lay_for_win(win: i64, point: u8) -> i64 {
    match point {
        4 | 10 => win * 2,
        5 | 9 => win * 3 / 2,
        _ => win * 6 / 5, // 6 | 8
    }
}

pub(crate) fn hardway_win(stake: i64, num: u8) -> i64 {
    match num {
        4 | 10 => stake * 7,
        _ => stake * 9, // 6 | 8
    }
}
/// The payout unit a place bet must be a multiple of, by number. The
/// adjudicator rounds every requested place stake up to this so payouts
/// stay whole cents.
#[inline]
pub(crate) const fn place_unit_of(num: u8) -> i64 {
    place_unit(num)
}

/// Public sizing helper: the actual stake a place bet takes at a table
/// minimum (rounded up to its payout unit) — the Design screen prices
/// worst-case exposure with it.
pub fn place_stake_for(table_min_cents: i64, num: u8) -> i64 {
    place_stake(table_min_cents, num)
}

/// The stake sequence a progression produces over the canonical
/// W-W-L-W-L-L pattern, in cents at a 100-cent base — the Design screen's
/// preview sparklines. The first entry is the opening stake.
pub fn progression_preview(p: Progression) -> Vec<i64> {
    let base = 100;
    let mut st = ProgState::new(base);
    let mut out = vec![st.stake];
    for win in [true, true, false, true, false, false] {
        if win {
            p.on_win(&mut st, base, base);
        } else {
            p.on_loss(&mut st, base, base);
        }
        out.push(st.stake);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progression_transition_sequences() {
        let base = 1000;
        let run = |prog: Progression, events: &[(bool, i64)]| {
            let mut st = ProgState::new(base);
            for (k, &(win, expect)) in events.iter().enumerate() {
                if win {
                    prog.on_win(&mut st, base, base);
                } else {
                    prog.on_loss(&mut st, base, base);
                }
                assert_eq!(
                    st.stake, expect,
                    "{prog:?} step {k}: got {} want {expect}",
                    st.stake
                );
            }
        };
        // (event, expected stake after the event); W = win, L = loss
        const W: bool = true;
        const L: bool = false;
        run(
            Progression::HalfPress,
            &[(W, 1500), (W, 2250), (W, 3375), (L, 1000)],
        );
        run(
            Progression::PressAndPull,
            &[(W, 2000), (W, 1000), (W, 2000), (L, 1000)],
        );
        run(
            Progression::Paroli3,
            &[(W, 2000), (W, 4000), (W, 1000), (W, 2000), (L, 1000)],
        );
        run(
            Progression::S1326,
            &[
                (W, 3000),
                (W, 2000),
                (W, 6000),
                (W, 1000),
                (W, 3000),
                (L, 1000),
            ],
        );
        run(
            Progression::GrandMartingale,
            &[(L, 3000), (L, 7000), (W, 1000)],
        );
        run(
            Progression::DAlembert,
            &[(L, 2000), (L, 3000), (W, 2000), (W, 1000), (W, 1000)],
        );
        run(
            Progression::ReverseDAlembert,
            &[(W, 2000), (W, 3000), (L, 2000), (L, 1000), (L, 1000)],
        );
        run(
            Progression::Fibonacci,
            &[
                (L, 1000),
                (L, 2000),
                (L, 3000),
                (L, 5000),
                (W, 2000),
                (W, 1000),
            ],
        );
    }
}
