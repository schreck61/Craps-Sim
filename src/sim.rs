// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Craps simulation engine. All money is in integer cents so payouts are exact.

use crate::rng::Xoshiro256pp;

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
const fn place_unit(num: u8) -> i64 {
    match num {
        6 | 8 => 600,
        _ => 500,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    fn multiple(&self, point: u8) -> i64 {
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    step: u32,
    /// Net profit of the current series, used by Oscar's Grind.
    profit: i64,
}

impl ProgState {
    fn new(base: i64) -> Self {
        Self {
            stake: base,
            step: 0,
            profit: 0,
        }
    }
}

fn fib_units(n: u32) -> i64 {
    let (mut a, mut b) = (1i64, 1i64);
    for _ in 0..n {
        let c = a + b;
        a = b;
        b = c;
    }
    a
}

// Progression stakes can't run away past this even before table-max clamping.
const PROG_STAKE_CAP: i64 = 1 << 40;

impl Progression {
    fn on_win(&self, st: &mut ProgState, base: i64, won: i64) {
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

    fn on_loss(&self, st: &mut ProgState, base: i64, lost: i64) {
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

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
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
fn round_up(amount: i64, unit: i64) -> i64 {
    ((amount + unit - 1) / unit) * unit
}

/// Place bets must be sized so payouts are whole: 6/8 in $6 units, others in $5 units.
fn place_stake(table_min_cents: i64, num: u8) -> i64 {
    round_up(table_min_cents, place_unit(num))
}

fn place_win(stake: i64, num: u8) -> i64 {
    match num {
        4 | 10 => stake * 9 / 5,
        5 | 9 => stake * 7 / 5,
        _ => stake * 7 / 6, // 6 | 8
    }
}

/// True-odds win for a pass odds stake on a given point.
fn pass_odds_win(stake: i64, point: u8) -> i64 {
    match point {
        4 | 10 => stake * 2,
        5 | 9 => stake * 3 / 2,
        _ => stake * 6 / 5, // 6 | 8
    }
}

/// For don't pass lay odds: amount to lay so that the lay wins `win` cents.
fn dont_lay_for_win(win: i64, point: u8) -> i64 {
    match point {
        4 | 10 => win * 2,
        5 | 9 => win * 3 / 2,
        _ => win * 6 / 5, // 6 | 8
    }
}

fn hardway_win(stake: i64, num: u8) -> i64 {
    match num {
        4 | 10 => stake * 7,
        _ => stake * 9, // 6 | 8
    }
}

struct Session<'a> {
    sel: &'a BetSelection,
    rules: &'a Rules,
    min: i64,
    /// Cash in hand. Never negative in ruin mode; may go negative in drawdown mode.
    cash: i64,
    allow_negative: bool,
    max_outlay: i64,
    point: Option<u8>,
    pass: i64,
    pass_odds: i64,
    pass_odds_point: u8,
    dont: i64,
    dont_lay: i64,
    dont_lay_win: i64,
    place: [i64; 6],
    hard: [i64; 4],
    // Come bets: flat in the come box (travels on the next roll), then flat
    // and odds per number, indexed like PLACE_NUMS.
    come_flat: i64,
    come_points: [i64; 6],
    come_odds: [i64; 6],
    // Don't come: flat in the DC box, then flat and lay odds per number.
    dc_flat: i64,
    dc_points: [i64; 6],
    dc_lay: [i64; 6],
    dc_lay_win: [i64; 6],
    // one-roll bets, placed pre-roll and resolved the same roll
    field_bet: i64,
    any7_bet: i64,
    anycraps_bet: i64,
    /// Set by any resolution that changes cash, bets, or the point — the
    /// only inputs `place_bets` reads. While clear, `place_bets` would be a
    /// no-op, so the pre-roll placement pass can be skipped entirely
    /// (except that one-roll bets always need re-placing).
    needs_placement: bool,
    one_roll_selected: bool,
    // Progression state, one stream per bet type. Come and don't come flats
    // share one stream each (the press applies to the next flat made).
    table_max: i64,
    p_pass: ProgState,
    p_dont: ProgState,
    p_come: ProgState,
    p_dc: ProgState,
    p_field: ProgState,
    p_place: [ProgState; 6],
    p_hard: [ProgState; 4],
    p_any7: ProgState,
    p_anycraps: ProgState,
}

impl<'a> Session<'a> {
    fn new(
        sel: &'a BetSelection,
        rules: &'a Rules,
        min: i64,
        cash: i64,
        allow_negative: bool,
    ) -> Self {
        Self {
            sel,
            rules,
            min,
            cash,
            allow_negative,
            max_outlay: 0,
            point: None,
            pass: 0,
            pass_odds: 0,
            pass_odds_point: 0,
            dont: 0,
            dont_lay: 0,
            dont_lay_win: 0,
            place: [0; 6],
            hard: [0; 4],
            come_flat: 0,
            come_points: [0; 6],
            come_odds: [0; 6],
            dc_flat: 0,
            dc_points: [0; 6],
            dc_lay: [0; 6],
            dc_lay_win: [0; 6],
            field_bet: 0,
            any7_bet: 0,
            anycraps_bet: 0,
            needs_placement: true,
            one_roll_selected: sel.field || sel.any_seven || sel.any_craps,
            table_max: min.saturating_mul(rules.table_max_mult.max(1)),
            p_pass: ProgState::new(min),
            p_dont: ProgState::new(min),
            p_come: ProgState::new(min),
            p_dc: ProgState::new(min),
            p_field: ProgState::new(min),
            p_place: [
                ProgState::new(place_stake(min, 4)),
                ProgState::new(place_stake(min, 5)),
                ProgState::new(place_stake(min, 6)),
                ProgState::new(place_stake(min, 8)),
                ProgState::new(place_stake(min, 9)),
                ProgState::new(place_stake(min, 10)),
            ],
            p_hard: [ProgState::new(rules.prop_bet_cents); 4],
            p_any7: ProgState::new(rules.prop_bet_cents),
            p_anycraps: ProgState::new(rules.prop_bet_cents),
        }
    }

    /// The stake a progression stream calls for, clamped to [base, table max].
    /// Pressed stakes are rounded to whole dollars — a real table would not
    /// accept a $33.75 flat bet, and whole-dollar flats keep every true-odds
    /// payout exact in cents (progressions like Half Press otherwise produce
    /// sub-cent-precision stakes).
    fn prog_stake(&self, st: &ProgState, base: i64) -> i64 {
        let max = self.table_max.max(base);
        let raw = st.stake.clamp(base, max);
        if raw == base {
            return base;
        }
        ((raw + 50) / 100 * 100).clamp(base, max)
    }

    /// Same, rounded to the nearest payout unit for place bets.
    fn prog_place_stake(&self, i: usize) -> i64 {
        let num = PLACE_NUMS[i];
        let unit = place_unit(num);
        let base = place_stake(self.min, num);
        let v = self.prog_stake(&self.p_place[i], base);
        ((v + unit / 2) / unit).max(1) * unit
    }

    /// Try the progression stake; if the bankroll can't cover it, fall back to
    /// the base bet (bet what you can rather than sitting out). The
    /// progression stream still advances from its own intended stake — the
    /// player is behind on the system, not restarting it.
    fn try_stake_or_base(&mut self, desired: i64, base: i64) -> Option<i64> {
        self.try_stake(desired).or_else(|| {
            if base < desired {
                self.try_stake(base)
            } else {
                None
            }
        })
    }

    #[inline]
    fn try_stake(&mut self, amount: i64) -> Option<i64> {
        if amount <= 0 {
            return None;
        }
        if self.allow_negative || self.cash >= amount {
            self.cash -= amount;
            if -self.cash > self.max_outlay {
                self.max_outlay = -self.cash;
            }
            Some(amount)
        } else {
            None
        }
    }

    fn has_multi_roll_bets(&self) -> bool {
        self.pass > 0
            || self.pass_odds > 0
            || self.dont > 0
            || self.dont_lay > 0
            || self.come_flat > 0
            || self.dc_flat > 0
            || self.come_points.iter().any(|&b| b > 0)
            || self.dc_points.iter().any(|&b| b > 0)
            || self.place.iter().any(|&b| b > 0)
            || self.hard.iter().any(|&b| b > 0)
    }

    fn live_come_bets(&self) -> u8 {
        (self.come_flat > 0) as u8 + self.come_points.iter().filter(|&&b| b > 0).count() as u8
    }

    fn live_dc_bets(&self) -> u8 {
        (self.dc_flat > 0) as u8 + self.dc_points.iter().filter(|&&b| b > 0).count() as u8
    }

    fn has_one_roll_bets(&self) -> bool {
        self.field_bet > 0 || self.any7_bet > 0 || self.anycraps_bet > 0
    }

    /// Face value of everything still on the table (what the player walks
    /// away with, treating live bets as taken down at the end of a session).
    fn on_table_face(&self) -> i64 {
        self.pass
            + self.pass_odds
            + self.dont
            + self.dont_lay
            + self.come_flat
            + self.come_points.iter().sum::<i64>()
            + self.come_odds.iter().sum::<i64>()
            + self.dc_flat
            + self.dc_points.iter().sum::<i64>()
            + self.dc_lay.iter().sum::<i64>()
            + self.place.iter().sum::<i64>()
            + self.hard.iter().sum::<i64>()
            + self.field_bet
            + self.any7_bet
            + self.anycraps_bet
    }

    /// Cheapest stake among selected recurring bets — used to decide ruin.
    fn cheapest_selected_stake(&self) -> i64 {
        cheapest_selected_stake(self.sel, self.rules, self.min)
    }

    /// Place all bets the strategy calls for at this moment, as affordable.
    fn place_bets(&mut self) {
        match self.point {
            None => {
                // Come-out: line bets only. Place bets and hardways are "off".
                if self.sel.pass_line && self.pass == 0 {
                    let want = self.prog_stake(&self.p_pass, self.min);
                    if let Some(a) = self.try_stake_or_base(want, self.min) {
                        self.pass = a;
                    }
                }
                if self.sel.dont_pass && self.dont == 0 {
                    let want = self.prog_stake(&self.p_dont, self.min);
                    if let Some(a) = self.try_stake_or_base(want, self.min) {
                        self.dont = a;
                    }
                }
            }
            Some(point) => {
                // New come / don't come bets are only made while a point is on.
                if self.sel.come_max > 0
                    && self.come_flat == 0
                    && self.live_come_bets() < self.sel.come_max
                {
                    let want = self.prog_stake(&self.p_come, self.min);
                    if let Some(a) = self.try_stake_or_base(want, self.min) {
                        self.come_flat = a;
                    }
                }
                if self.sel.dont_come_max > 0
                    && self.dc_flat == 0
                    && self.live_dc_bets() < self.sel.dont_come_max
                {
                    let want = self.prog_stake(&self.p_dc, self.min);
                    if let Some(a) = self.try_stake_or_base(want, self.min) {
                        self.dc_flat = a;
                    }
                }
                // Odds behind an established line bet.
                if self.sel.take_odds {
                    let mult = self.rules.odds_policy.multiple(point);
                    if mult > 0 {
                        if self.pass > 0 && self.pass_odds == 0 {
                            let stake = self.pass * mult;
                            if let Some(a) = self.try_stake(stake) {
                                self.pass_odds = a;
                                self.pass_odds_point = point;
                            }
                        }
                        if self.dont > 0 && self.dont_lay == 0 {
                            let win = self.dont * mult;
                            let stake = dont_lay_for_win(win, point);
                            if let Some(a) = self.try_stake(stake) {
                                self.dont_lay = a;
                                self.dont_lay_win = win;
                            }
                        }
                    }
                }
                // Place bets on selected numbers other than the current point.
                for (i, &num) in PLACE_NUMS.iter().enumerate() {
                    if self.sel.place[i] && num != point && self.place[i] == 0 {
                        let want = self.prog_place_stake(i);
                        let base = place_stake(self.min, num);
                        if let Some(a) = self.try_stake_or_base(want, base) {
                            self.place[i] = a;
                        }
                    }
                }
                // Hardways.
                for (i, _) in HARD_NUMS.iter().enumerate() {
                    if self.sel.hardways[i] && self.hard[i] == 0 {
                        let base = self.rules.prop_bet_cents;
                        let want = self.prog_stake(&self.p_hard[i], base);
                        if let Some(a) = self.try_stake_or_base(want, base) {
                            self.hard[i] = a;
                        }
                    }
                }
            }
        }
        // Odds behind established come / don't come points can be taken (or
        // topped up after a new flat stacks on) at any time.
        if self.sel.take_odds {
            for (i, &num) in PLACE_NUMS.iter().enumerate() {
                let mult = self.rules.odds_policy.multiple(num);
                if mult == 0 {
                    continue;
                }
                if self.come_points[i] > 0 {
                    let want = self.come_points[i] * mult;
                    if self.come_odds[i] < want {
                        if let Some(a) = self.try_stake(want - self.come_odds[i]) {
                            self.come_odds[i] += a;
                        }
                    }
                }
                if self.dc_points[i] > 0 {
                    let want_win = self.dc_points[i] * mult;
                    if self.dc_lay_win[i] < want_win {
                        let stake = dont_lay_for_win(want_win - self.dc_lay_win[i], num);
                        if let Some(a) = self.try_stake(stake) {
                            self.dc_lay[i] += a;
                            self.dc_lay_win[i] += want_win - self.dc_lay_win[i];
                        }
                    }
                }
            }
        }
        // One-roll bets, working on every roll.
        if self.sel.field && self.field_bet == 0 {
            let want = self.prog_stake(&self.p_field, self.min);
            if let Some(a) = self.try_stake_or_base(want, self.min) {
                self.field_bet = a;
            }
        }
        if self.sel.any_seven && self.any7_bet == 0 {
            let base = self.rules.prop_bet_cents;
            let want = self.prog_stake(&self.p_any7, base);
            if let Some(a) = self.try_stake_or_base(want, base) {
                self.any7_bet = a;
            }
        }
        if self.sel.any_craps && self.anycraps_bet == 0 {
            let base = self.rules.prop_bet_cents;
            let want = self.prog_stake(&self.p_anycraps, base);
            if let Some(a) = self.try_stake_or_base(want, base) {
                self.anycraps_bet = a;
            }
        }
        // Placement is a pure function of cash, bets, and the point; until a
        // resolution changes one of those, running it again is a no-op.
        self.needs_placement = false;
    }

    /// Resolve come and don't come bets for a roll of `t`. `was_comeout` is
    /// whether the main game was on a come-out roll (come odds off then, by
    /// default; don't come lay odds always work).
    fn resolve_come_bets(&mut self, t: u8, was_comeout: bool) {
        let come_odds_working = !was_comeout || self.rules.come_odds_work_on_comeout;
        let prog = self.sel.progression;

        // Established come points resolve before the new flat travels.
        if t == 7 {
            for i in 0..6 {
                if self.come_points[i] > 0 {
                    self.needs_placement = true;
                    prog.on_loss(&mut self.p_come, self.min, self.come_points[i]);
                    self.come_points[i] = 0;
                    if self.come_odds[i] > 0 {
                        if !come_odds_working {
                            self.cash += self.come_odds[i]; // off: returned
                        }
                        self.come_odds[i] = 0;
                    }
                }
                if self.dc_points[i] > 0 {
                    self.needs_placement = true;
                    // Don't come wins: flat even money plus the lay.
                    self.cash += self.dc_points[i] * 2 + self.dc_lay[i] + self.dc_lay_win[i];
                    prog.on_win(&mut self.p_dc, self.min, self.dc_points[i]);
                    self.dc_points[i] = 0;
                    self.dc_lay[i] = 0;
                    self.dc_lay_win[i] = 0;
                }
            }
        } else if let Some(i) = place_index(t) {
            if self.come_points[i] > 0 {
                self.needs_placement = true;
                // Come point made: flat wins even money, bet comes down.
                self.cash += self.come_points[i] * 2;
                prog.on_win(&mut self.p_come, self.min, self.come_points[i]);
                self.come_points[i] = 0;
                if self.come_odds[i] > 0 {
                    if come_odds_working {
                        self.cash += self.come_odds[i] + pass_odds_win(self.come_odds[i], t);
                    } else {
                        self.cash += self.come_odds[i]; // off: returned
                    }
                    self.come_odds[i] = 0;
                }
            }
            if self.dc_points[i] > 0 {
                self.needs_placement = true;
                // Number repeated: don't come loses flat and lay.
                prog.on_loss(&mut self.p_dc, self.min, self.dc_points[i]);
                self.dc_points[i] = 0;
                self.dc_lay[i] = 0;
                self.dc_lay_win[i] = 0;
            }
        }

        // A flat in the come box has its own come-out on this roll.
        if self.come_flat > 0 {
            self.needs_placement = true;
            match t {
                7 | 11 => {
                    self.cash += self.come_flat * 2;
                    prog.on_win(&mut self.p_come, self.min, self.come_flat);
                    self.come_flat = 0;
                }
                2 | 3 | 12 => {
                    prog.on_loss(&mut self.p_come, self.min, self.come_flat);
                    self.come_flat = 0;
                }
                _ => {
                    // 4/5/6/8/9/10 are the only sums left after the arms above.
                    let i = place_index(t).expect("come flat travels to a box number");
                    self.come_points[i] += self.come_flat;
                    self.come_flat = 0;
                }
            }
        }
        if self.dc_flat > 0 {
            self.needs_placement = true;
            match t {
                2 | 3 => {
                    self.cash += self.dc_flat * 2;
                    prog.on_win(&mut self.p_dc, self.min, self.dc_flat);
                    self.dc_flat = 0;
                }
                7 | 11 => {
                    prog.on_loss(&mut self.p_dc, self.min, self.dc_flat);
                    self.dc_flat = 0;
                }
                12 => {} // bar 12: push, stays in the box
                _ => {
                    let i = place_index(t).expect("don't come flat travels to a box number");
                    self.dc_points[i] += self.dc_flat;
                    self.dc_flat = 0;
                }
            }
        }
    }

    /// Resolve one roll of the dice.
    fn resolve(&mut self, d1: u8, d2: u8) {
        let t = d1 + d2;
        let is_hard = d1 == d2;
        let was_comeout = self.point.is_none();
        self.resolve_come_bets(t, was_comeout);

        // --- One-roll bets ---
        let prog = self.sel.progression;
        if self.field_bet > 0 {
            self.needs_placement = true;
            let b = self.field_bet;
            self.field_bet = 0;
            let win_mult = match t {
                3 | 4 | 9 | 10 | 11 => 1,
                2 => 2,
                12 => {
                    if self.rules.field_12_triple {
                        3
                    } else {
                        2
                    }
                }
                _ => 0,
            };
            if win_mult > 0 {
                self.cash += b * (1 + win_mult);
                prog.on_win(&mut self.p_field, self.min, b * win_mult);
            } else {
                prog.on_loss(&mut self.p_field, self.min, b);
            }
        }
        if self.any7_bet > 0 {
            self.needs_placement = true;
            let b = self.any7_bet;
            self.any7_bet = 0;
            if t == 7 {
                self.cash += b * 5; // 4:1
                prog.on_win(&mut self.p_any7, self.rules.prop_bet_cents, b * 4);
            } else {
                prog.on_loss(&mut self.p_any7, self.rules.prop_bet_cents, b);
            }
        }
        if self.anycraps_bet > 0 {
            self.needs_placement = true;
            let b = self.anycraps_bet;
            self.anycraps_bet = 0;
            if t == 2 || t == 3 || t == 12 {
                self.cash += b * 8; // 7:1
                prog.on_win(&mut self.p_anycraps, self.rules.prop_bet_cents, b * 7);
            } else {
                prog.on_loss(&mut self.p_anycraps, self.rules.prop_bet_cents, b);
            }
        }

        match self.point {
            None => {
                // --- Come-out roll. Place bets & hardways are off. ---
                self.needs_placement = true;
                match t {
                    7 | 11 => {
                        if self.pass > 0 {
                            self.cash += self.pass * 2;
                            prog.on_win(&mut self.p_pass, self.min, self.pass);
                            self.pass = 0;
                        }
                        if self.dont > 0 {
                            prog.on_loss(&mut self.p_dont, self.min, self.dont);
                            self.dont = 0; // loses (nothing laid yet on come-out)
                        }
                    }
                    2 | 3 => {
                        if self.pass > 0 {
                            prog.on_loss(&mut self.p_pass, self.min, self.pass);
                            self.pass = 0;
                        }
                        if self.dont > 0 {
                            self.cash += self.dont * 2;
                            prog.on_win(&mut self.p_dont, self.min, self.dont);
                            self.dont = 0;
                        }
                    }
                    12 => {
                        // Bar 12: pass loses, don't pass pushes (stays up).
                        if self.pass > 0 {
                            prog.on_loss(&mut self.p_pass, self.min, self.pass);
                            self.pass = 0;
                        }
                    }
                    _ => {
                        self.point = Some(t);
                    }
                }
            }
            Some(point) => {
                if t == 7 {
                    // --- Seven-out ---
                    self.needs_placement = true;
                    if self.pass > 0 {
                        prog.on_loss(&mut self.p_pass, self.min, self.pass);
                        self.pass = 0;
                    }
                    self.pass_odds = 0;
                    if self.dont > 0 {
                        self.cash += self.dont * 2;
                        prog.on_win(&mut self.p_dont, self.min, self.dont);
                        self.dont = 0;
                    }
                    if self.dont_lay > 0 {
                        self.cash += self.dont_lay + self.dont_lay_win;
                        self.dont_lay = 0;
                        self.dont_lay_win = 0;
                    }
                    for (i, &num) in PLACE_NUMS.iter().enumerate() {
                        if self.place[i] > 0 {
                            // working place bets lose
                            let base = place_stake(self.min, num);
                            prog.on_loss(&mut self.p_place[i], base, self.place[i]);
                            self.place[i] = 0;
                        }
                    }
                    for i in 0..4 {
                        if self.hard[i] > 0 {
                            prog.on_loss(
                                &mut self.p_hard[i],
                                self.rules.prop_bet_cents,
                                self.hard[i],
                            );
                            self.hard[i] = 0;
                        }
                    }
                    self.point = None;
                    return;
                }

                // Place bets (working while a point is on). A winning place
                // bet stays up, and the progression presses or regresses it
                // in place out of (or back into) the player's rail.
                if let Some(i) = place_index(t) {
                    if self.place[i] > 0 {
                        self.needs_placement = true;
                        self.cash += place_win(self.place[i], t);
                        let base = place_stake(self.min, t);
                        prog.on_win(&mut self.p_place[i], base, place_win(self.place[i], t));
                        let cur = self.place[i];
                        let desired = self.prog_place_stake(i);
                        if desired > cur {
                            if self.try_stake(desired - cur).is_some() {
                                self.place[i] = desired;
                            }
                        } else if desired < cur {
                            self.cash += cur - desired;
                            self.place[i] = desired;
                        }
                    }
                }
                // Hardways: winners stay up, pressed the same way.
                if let Some(i) = hard_index(t) {
                    if self.hard[i] > 0 {
                        self.needs_placement = true;
                        let base = self.rules.prop_bet_cents;
                        if is_hard {
                            self.cash += hardway_win(self.hard[i], t);
                            prog.on_win(&mut self.p_hard[i], base, hardway_win(self.hard[i], t));
                            let cur = self.hard[i];
                            let desired = self.prog_stake(&self.p_hard[i], base);
                            if desired > cur {
                                if self.try_stake(desired - cur).is_some() {
                                    self.hard[i] = desired;
                                }
                            } else if desired < cur {
                                self.cash += cur - desired;
                                self.hard[i] = desired;
                            }
                        } else {
                            prog.on_loss(&mut self.p_hard[i], base, self.hard[i]);
                            self.hard[i] = 0; // easy way loses
                        }
                    }
                }

                if t == point {
                    // --- Point made ---
                    self.needs_placement = true;
                    if self.pass > 0 {
                        self.cash += self.pass * 2;
                        prog.on_win(&mut self.p_pass, self.min, self.pass);
                        self.pass = 0;
                    }
                    if self.pass_odds > 0 {
                        self.cash +=
                            self.pass_odds + pass_odds_win(self.pass_odds, self.pass_odds_point);
                        self.pass_odds = 0;
                    }
                    if self.dont > 0 {
                        prog.on_loss(&mut self.p_dont, self.min, self.dont);
                        self.dont = 0;
                    }
                    self.dont_lay = 0;
                    self.dont_lay_win = 0;
                    self.point = None;
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuinOutcome {
    pub rolls: u64,
    pub censored: bool,
    /// True if the session ended by reaching the take-profit target.
    pub hit_target: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HorizonOutcome {
    /// Cash plus face value of live bets when the session ends.
    pub final_cents: i64,
    /// True if the bankroll ran out before the horizon.
    pub busted: bool,
    /// True if the session ended early by reaching the take-profit target.
    pub hit_target: bool,
    /// Rolls actually played (equal to the horizon unless the session ended early).
    pub rolls: u64,
}

/// One budgeted session, reported two ways from the same dice.
pub struct SessionOutcomes {
    /// The full play-until-bust/quit/cap trajectory.
    pub ruin: RuinOutcome,
    /// The fixed-horizon snapshot: exactly what the session looked like as it
    /// crossed `horizon_rolls` (or how it had already ended before that).
    pub horizon: HorizonOutcome,
}

/// Play one session and report both the ruin trajectory and the fixed-horizon
/// snapshot. The horizon result is identical to running a separate horizon
/// session with the same seed — the horizon is a strict prefix of the ruin
/// trajectory — so one pass answers both questions, halving simulation work.
///
/// * Bust and take-profit end both views at once.
/// * Crossing `horizon_rolls` freezes the horizon view (wealth = cash plus
///   live bets at face value); play continues for the ruin view.
/// * Reaching `max_rolls` freezes the ruin view as censored; if the horizon
///   lies beyond the cap, play continues (with full budget semantics) until
///   the horizon view is decided too.
#[allow(clippy::too_many_arguments)]
pub fn run_session(
    sel: &BetSelection,
    rules: &Rules,
    table_min_cents: i64,
    budget_cents: i64,
    quit_target_cents: Option<i64>,
    max_rolls: u64,
    horizon_rolls: u64,
    seed: u64,
) -> SessionOutcomes {
    let mut rng = Xoshiro256pp::seed_from_u64(seed);
    let mut s = Session::new(sel, rules, table_min_cents, budget_cents, false);
    let cheapest = s.cheapest_selected_stake();
    let mut rolls = 0u64;
    let mut ruin: Option<RuinOutcome> = None;
    let mut horizon: Option<HorizonOutcome> = None;
    loop {
        if s.needs_placement || s.one_roll_selected {
            s.place_bets();
        }
        if !s.has_multi_roll_bets() && !s.has_one_roll_bets() && s.cash < cheapest {
            return SessionOutcomes {
                ruin: ruin.unwrap_or(RuinOutcome {
                    rolls,
                    censored: false,
                    hit_target: false,
                }),
                horizon: horizon.unwrap_or(HorizonOutcome {
                    final_cents: s.cash,
                    busted: true,
                    hit_target: false,
                    rolls,
                }),
            };
        }
        let (d1, d2) = rng.dice();
        rolls += 1;
        s.resolve(d1, d2);
        if let Some(target) = quit_target_cents {
            let wealth = s.cash + s.on_table_face();
            if wealth >= target {
                return SessionOutcomes {
                    ruin: ruin.unwrap_or(RuinOutcome {
                        rolls,
                        censored: false,
                        hit_target: true,
                    }),
                    horizon: horizon.unwrap_or(HorizonOutcome {
                        final_cents: wealth,
                        busted: false,
                        hit_target: true,
                        rolls,
                    }),
                };
            }
        }
        if horizon.is_none() && rolls >= horizon_rolls {
            horizon = Some(HorizonOutcome {
                final_cents: s.cash + s.on_table_face(),
                busted: false,
                hit_target: false,
                rolls,
            });
        }
        if ruin.is_none() && rolls >= max_rolls {
            ruin = Some(RuinOutcome {
                rolls,
                censored: true,
                hit_target: false,
            });
        }
        if let (Some(r), Some(h)) = (ruin, horizon) {
            return SessionOutcomes {
                ruin: r,
                horizon: h,
            };
        }
    }
}

/// Play until the bankroll can no longer sustain the strategy, until the
/// take-profit target (cash + face value of live bets) is reached, or until
/// `max_rolls`.
///
/// The app's worker uses [`run_session`] to get this and the horizon view in
/// one pass; this single-view form is kept as API surface and test harness.
#[allow(dead_code)]
pub fn run_ruin_session(
    sel: &BetSelection,
    rules: &Rules,
    table_min_cents: i64,
    budget_cents: i64,
    quit_target_cents: Option<i64>,
    max_rolls: u64,
    seed: u64,
) -> RuinOutcome {
    // A zero-roll horizon is decided immediately, so only the ruin view runs.
    run_session(
        sel,
        rules,
        table_min_cents,
        budget_cents,
        quit_target_cents,
        max_rolls,
        0,
        seed,
    )
    .ruin
}

/// Play a fixed number of rolls with the real budget (or until ruin, or until
/// the take-profit target is reached) and report the money in hand at the end.
pub fn run_horizon_session(
    sel: &BetSelection,
    rules: &Rules,
    table_min_cents: i64,
    budget_cents: i64,
    quit_target_cents: Option<i64>,
    horizon_rolls: u64,
    seed: u64,
) -> HorizonOutcome {
    // Cap the ruin view at the horizon so both views finish together.
    run_session(
        sel,
        rules,
        table_min_cents,
        budget_cents,
        quit_target_cents,
        horizon_rolls,
        horizon_rolls,
        seed,
    )
    .horizon
}

/// Play a fixed number of rolls with an unconstrained bankroll and report the
/// maximum outlay (peak cash deficit). A starting budget B >= max outlay is
/// sufficient to survive this dice sequence while placing the full strategy
/// on every roll, so the p-th percentile of this statistic is the budget that
/// survives the horizon with probability p. (A smaller budget might still
/// scrape through by falling back to base bets, so the recommendation errs on
/// the safe side.)
pub fn run_drawdown_session(
    sel: &BetSelection,
    rules: &Rules,
    table_min_cents: i64,
    horizon_rolls: u64,
    seed: u64,
) -> i64 {
    let mut rng = Xoshiro256pp::seed_from_u64(seed);
    let mut s = Session::new(sel, rules, table_min_cents, 0, true);
    for _ in 0..horizon_rolls {
        if s.needs_placement || s.one_roll_selected {
            s.place_bets();
        }
        let (d1, d2) = rng.dice();
        s.resolve(d1, d2);
    }
    s.max_outlay
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct RuinStats {
    pub mean: f64,
    pub ci95_half: f64,
    pub p5: u64,
    pub p25: u64,
    pub p50: u64,
    pub p75: u64,
    pub p95: u64,
    pub censored_frac: f64,
    /// Fraction of sessions that ended by hitting the take-profit target.
    pub target_frac: f64,
}

/// Summarize a non-empty set of session lengths (sorted in place).
/// `censored`/`hit_target` are counts out of `rolls.len()`. Censored sessions
/// enter at the cap value, biasing the mean and upper percentiles low — the
/// censored fraction is reported so callers can surface that.
///
/// # Panics
///
/// Panics if `rolls` is empty.
pub fn summarize_ruin(rolls: &mut [u64], censored: u64, hit_target: u64) -> RuinStats {
    assert!(
        !rolls.is_empty(),
        "summarize_ruin needs at least one session"
    );
    rolls.sort_unstable();
    let n = rolls.len() as f64;
    let mean = rolls.iter().map(|&r| r as f64).sum::<f64>() / n;
    let var = rolls
        .iter()
        .map(|&r| {
            let d = r as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / (n - 1.0).max(1.0);
    let se = (var / n).sqrt();
    let pct = |p: f64| -> u64 {
        let idx = ((p / 100.0) * (rolls.len() as f64 - 1.0)).round() as usize;
        rolls[idx.min(rolls.len() - 1)]
    };
    RuinStats {
        mean,
        ci95_half: 1.96 * se,
        p5: pct(5.0),
        p25: pct(25.0),
        p50: pct(50.0),
        p75: pct(75.0),
        p95: pct(95.0),
        censored_frac: censored as f64 / n,
        target_frac: hit_target as f64 / n,
    }
}

#[derive(Clone, Debug)]
pub struct HorizonStats {
    pub mean: f64,
    pub bust_frac: f64,
    /// Fraction of sessions that quit early at the take-profit target.
    pub target_frac: f64,
    /// P10 through P90 of the ending bankroll, in cents.
    pub deciles: [i64; 9],
}

/// Summarize a non-empty set of ending bankrolls (sorted in place).
///
/// # Panics
///
/// Panics if `finals` is empty.
pub fn summarize_horizon(finals: &mut [i64], busted: u64, hit_target: u64) -> HorizonStats {
    assert!(
        !finals.is_empty(),
        "summarize_horizon needs at least one session"
    );
    finals.sort_unstable();
    let n = finals.len();
    let mut deciles = [0i64; 9];
    for (k, d) in deciles.iter_mut().enumerate() {
        let idx = (((k + 1) as f64 / 10.0) * (n as f64 - 1.0)).round() as usize;
        *d = finals[idx.min(n - 1)];
    }
    HorizonStats {
        mean: finals.iter().map(|&v| v as f64).sum::<f64>() / n as f64,
        bust_frac: busted as f64 / n as f64,
        target_frac: hit_target as f64 / n as f64,
        deciles,
    }
}

/// Percentile of the max-outlay distribution = budget needed to survive the
/// horizon with that probability.
/// `confidence` is a percentage in (0, 100]; `outlays` is sorted in place.
///
/// # Panics
///
/// Panics if `outlays` is empty.
pub fn budget_for_confidence(outlays: &mut [i64], confidence: f64) -> i64 {
    assert!(
        !outlays.is_empty(),
        "budget_for_confidence needs at least one session"
    );
    outlays.sort_unstable();
    let idx = ((confidence / 100.0) * (outlays.len() as f64 - 1.0)).ceil() as usize;
    outlays[idx.min(outlays.len() - 1)]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Rules {
        Rules {
            odds_policy: OddsPolicy::None,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 1000,
        }
    }

    fn only(f: impl FnOnce(&mut BetSelection)) -> BetSelection {
        let mut s = BetSelection {
            pass_line: false,
            ..Default::default()
        };
        f(&mut s);
        s
    }

    /// Run `n` rolls with a huge bankroll; return (net result, total wagered).
    fn grind(sel: &BetSelection, rules: &Rules, n: u64) -> (f64, f64) {
        grind_seeded(sel, rules, n, 12345)
    }

    fn grind_seeded(sel: &BetSelection, rules: &Rules, n: u64, seed: u64) -> (f64, f64) {
        let mut rng = Xoshiro256pp::seed_from_u64(seed);
        let start: i64 = 1_000_000_000_000;
        let mut s = Session::new(sel, rules, 1000, start, false);
        let mut wagered: i64 = 0;
        for _ in 0..n {
            let before = s.cash;
            s.place_bets();
            wagered += before - s.cash;
            let (d1, d2) = rng.dice();
            s.resolve(d1, d2);
        }
        // Settle: money still on the table is not lost; add it back at face value
        // (approximation only matters at the margin of a huge sample).
        let on_table = s.pass
            + s.pass_odds
            + s.dont
            + s.dont_lay
            + s.come_flat
            + s.come_points.iter().sum::<i64>()
            + s.come_odds.iter().sum::<i64>()
            + s.dc_flat
            + s.dc_points.iter().sum::<i64>()
            + s.dc_lay.iter().sum::<i64>()
            + s.place.iter().sum::<i64>()
            + s.hard.iter().sum::<i64>()
            + s.field_bet
            + s.any7_bet
            + s.anycraps_bet;
        ((s.cash + on_table - start) as f64, wagered as f64)
    }

    #[test]
    fn pass_line_house_edge() {
        let sel = only(|s| s.pass_line = true);
        let (net, wagered) = grind(&sel, &rules(), 4_000_000);
        let edge = -net / wagered;
        // True pass line edge is 1.414% per bet resolved.
        assert!((edge - 0.01414).abs() < 0.004, "edge was {edge}");
    }

    #[test]
    fn dont_pass_house_edge() {
        let sel = only(|s| s.dont_pass = true);
        let (net, wagered) = grind(&sel, &rules(), 4_000_000);
        let edge = -net / wagered;
        // True don't pass edge is 1.364% per bet resolved.
        assert!((edge - 0.01364).abs() < 0.004, "edge was {edge}");
    }

    #[test]
    fn field_house_edge() {
        let sel = only(|s| s.field = true);
        let (net, wagered) = grind(&sel, &rules(), 2_000_000);
        let edge = -net / wagered;
        // Field with 2x on 12 has a 5.556% edge.
        assert!((edge - 0.05556).abs() < 0.005, "edge was {edge}");
        let r3 = Rules {
            field_12_triple: true,
            ..rules()
        };
        let (net, wagered) = grind(&sel, &r3, 2_000_000);
        let edge = -net / wagered;
        // Triple-12 field edge is 2.778%.
        assert!((edge - 0.02778).abs() < 0.005, "edge was {edge}");
    }

    #[test]
    fn place_six_house_edge() {
        let sel = only(|s| s.place[2] = true); // place 6
        let (net, wagered) = grind(&sel, &rules(), 2_000_000);
        let edge = -net / wagered;
        // A place bet stays up after a win, so one placement resolves 11/6
        // times on average: per-placement edge = 1.515% * 11/6 = 2.778%.
        assert!((edge - 0.02778).abs() < 0.006, "edge was {edge}");
    }

    #[test]
    fn pass_with_full_odds_reduces_combined_edge() {
        let sel = only(|s| {
            s.pass_line = true;
            s.take_odds = true;
        });
        let r = Rules {
            odds_policy: OddsPolicy::X345,
            ..rules()
        };
        // Average over several seeds: a single sample's noise is comparable to
        // the tiny combined edge we are trying to detect.
        let mut net_sum = 0.0;
        let mut wagered_sum = 0.0;
        for seed in 0..8u64 {
            let (net, wagered) = grind_seeded(&sel, &r, 2_000_000, 1000 + seed);
            net_sum += net;
            wagered_sum += wagered;
        }
        let edge = -net_sum / wagered_sum;
        // Pass + 3-4-5x odds combined edge is ~0.374% of total wagered.
        assert!((edge - 0.00374).abs() < 0.0025, "edge was {edge}");
    }

    #[test]
    fn come_house_edge() {
        // A come bet is a pass line bet made mid-hand: 1.414% per bet, and
        // each flat placed resolves exactly once.
        let sel = only(|s| s.come_max = 3);
        let (net, wagered) = grind(&sel, &rules(), 4_000_000);
        let edge = -net / wagered;
        assert!((edge - 0.01414).abs() < 0.004, "edge was {edge}");
    }

    #[test]
    fn dont_come_house_edge() {
        let sel = only(|s| s.dont_come_max = 3);
        let (net, wagered) = grind(&sel, &rules(), 4_000_000);
        let edge = -net / wagered;
        assert!((edge - 0.01364).abs() < 0.004, "edge was {edge}");
    }

    #[test]
    fn come_bet_mechanics() {
        let sel = only(|s| {
            s.come_max = 1;
            s.take_odds = true;
        });
        let r = Rules {
            odds_policy: OddsPolicy::X2,
            ..rules()
        };
        // $10 min. Establish a main point so the come bet can go down.
        let mut s = Session::new(&sel, &r, 1000, 100_000, false);
        s.place_bets(); // no line bets selected, nothing placed on come-out
        assert_eq!(s.cash, 100_000);
        s.resolve(2, 6); // point 8 (main game)
        s.place_bets(); // come flat goes into the box
        assert_eq!(s.come_flat, 1000);
        s.resolve(2, 3); // come flat travels to 5
        assert_eq!(s.come_points[1], 1000);
        s.place_bets(); // 2x odds on the come point: $20
        assert_eq!(s.come_odds[1], 2000);
        // max 1 live come bet: the point counts, so no fresh flat goes down
        assert_eq!(s.come_flat, 0);
        let cash_before = s.cash;
        s.resolve(1, 4); // 5 rolls: come point made
                         // Flat $10 wins even (+$20 back), odds $20 pay 3:2 (+$50 back).
        assert_eq!(s.cash, cash_before + 2000 + 2000 + 3000);
    }

    #[test]
    fn come_odds_off_on_comeout() {
        let sel = only(|s| {
            s.come_max = 1;
            s.take_odds = true;
        });
        let r = Rules {
            odds_policy: OddsPolicy::X1,
            ..rules()
        };
        let mut s = Session::new(&sel, &r, 1000, 100_000, false);
        s.resolve(3, 3); // main point 6
        s.place_bets();
        s.resolve(1, 3); // come travels to 4
        s.place_bets(); // 1x odds on come 4
        assert_eq!(s.come_odds[0], 1000);
        s.resolve(2, 4); // main point 6 made -> next roll is a come-out
        assert_eq!(s.point, None);
        let cash_before = s.cash;
        s.resolve(3, 4); // come-out 7: come flat loses, odds off -> returned
        assert_eq!(s.cash, cash_before + 1000);
        assert_eq!(s.come_points[0], 0);
    }

    #[test]
    fn dont_come_mechanics() {
        let sel = only(|s| {
            s.dont_come_max = 1;
            s.take_odds = true;
        });
        let r = Rules {
            odds_policy: OddsPolicy::X1,
            ..rules()
        };
        let mut s = Session::new(&sel, &r, 1000, 100_000, false);
        s.resolve(3, 3); // main point 6
        s.place_bets();
        assert_eq!(s.dc_flat, 1000);
        s.resolve(6, 6); // 12: don't come pushes, stays in the box
        assert_eq!(s.dc_flat, 1000);
        s.resolve(1, 3); // DC travels to 4
        assert_eq!(s.dc_points[0], 1000);
        s.place_bets(); // lay odds: win $10 requires laying $20 on the 4
        assert_eq!(s.dc_lay[0], 2000);
        assert_eq!(s.dc_lay_win[0], 1000);
        let cash_before = s.cash;
        s.resolve(3, 4); // seven: DC wins flat + lay
        assert_eq!(s.cash, cash_before + 2000 + 2000 + 1000);
    }

    #[test]
    fn molly_combined_edge_matches_closed_form() {
        // The 3-pt Molly is three simultaneous pass-equivalent bets, so its
        // per-dollar edge equals the pass + 3-4-5x odds closed form:
        // (7/495) / (34/9) = 0.3743% of total money wagered.
        let sel = only(|s| {
            s.pass_line = true;
            s.come_max = 2;
            s.take_odds = true;
        });
        let r = Rules {
            odds_policy: OddsPolicy::X345,
            ..rules()
        };
        let mut net_sum = 0.0;
        let mut wagered_sum = 0.0;
        for seed in 0..8u64 {
            let (net, wagered) = grind_seeded(&sel, &r, 2_000_000, 9_000 + seed);
            net_sum += net;
            wagered_sum += wagered;
        }
        let edge = -net_sum / wagered_sum;
        assert!((edge - 0.003743).abs() < 0.0025, "edge was {edge}");
    }

    #[test]
    fn payout_exactness() {
        // Deterministic check of core payouts via direct resolution.
        let sel = only(|s| s.pass_line = true);
        let r = rules();
        let mut s = Session::new(&sel, &r, 1000, 10_000, false);
        s.place_bets();
        assert_eq!(s.cash, 9_000);
        s.resolve(3, 4); // natural 7 wins even money
        assert_eq!(s.cash, 11_000);

        // Point cycle: 6 then made the hard way.
        let mut s = Session::new(&sel, &r, 1000, 10_000, false);
        s.place_bets();
        s.resolve(2, 4); // point 6
        assert_eq!(s.point, Some(6));
        s.resolve(3, 3); // point made
        assert_eq!(s.cash, 11_000);
        assert_eq!(s.point, None);
    }

    #[test]
    fn odds_pay_true() {
        let sel = only(|s| {
            s.pass_line = true;
            s.take_odds = true;
        });
        let r = Rules {
            odds_policy: OddsPolicy::X2,
            ..rules()
        };
        // Table min $10, point 4: flat 1000, odds 2000, odds pay 2:1.
        let mut s = Session::new(&sel, &r, 1000, 100_000, false);
        s.place_bets();
        s.resolve(1, 3); // point 4
        s.place_bets(); // takes odds
        assert_eq!(s.cash, 100_000 - 1000 - 2000);
        s.resolve(2, 2); // point made (hard 4, no hardway bet)
                         // flat: +2000 back; odds: 2000 stake + 4000 win.
        assert_eq!(s.cash, 100_000 - 3000 + 2000 + 6000);
    }

    #[test]
    fn dont_pass_bar_twelve_pushes() {
        let sel = only(|s| s.dont_pass = true);
        let r = rules();
        let mut s = Session::new(&sel, &r, 1000, 10_000, false);
        s.place_bets();
        assert_eq!(s.cash, 9_000);
        s.resolve(6, 6); // 12 on come-out: push, bet stays
        assert_eq!(s.cash, 9_000);
        assert_eq!(s.dont, 1000);
        s.resolve(1, 2); // 3 craps: don't wins
        assert_eq!(s.cash, 11_000);
    }

    #[test]
    fn horizon_mean_matches_expected_loss() {
        // $10 pass line for 400 rolls from $300: pass decisions resolve every
        // 557/165 = 3.376 rolls on average, losing 1.414% of $10 each, so the
        // expected ending bankroll is 300 - 400/3.376 * 0.1414 = $283.2.
        let sel = only(|s| s.pass_line = true);
        let r = rules();
        let n = 200_000u64;
        let mut sum = 0i64;
        let mut busts = 0u64;
        for seed in 0..n {
            let o = run_horizon_session(&sel, &r, 1000, 30_000, None, 400, seed);
            sum += o.final_cents;
            busts += o.busted as u64;
        }
        let mean = sum as f64 / n as f64 / 100.0;
        assert!((mean - 283.2).abs() < 3.0, "mean final was ${mean:.2}");
        // Busting 30 units in ~118 decisions is possible but rare.
        assert!((busts as f64 / n as f64) < 0.01);
    }

    #[test]
    fn martingale_doubles_after_loss() {
        let sel = only(|s| {
            s.pass_line = true;
            s.progression = Progression::Martingale;
        });
        let r = rules();
        let mut s = Session::new(&sel, &r, 1000, 100_000, false);
        s.place_bets();
        assert_eq!(s.pass, 1000);
        s.resolve(1, 1); // craps: lose
        s.place_bets();
        assert_eq!(s.pass, 2000); // doubled
        s.resolve(1, 2); // craps again
        s.place_bets();
        assert_eq!(s.pass, 4000);
        s.resolve(3, 4); // natural: win $40
        s.place_bets();
        assert_eq!(s.pass, 1000); // reset to base
                                  // Net: -10 - 20 + 40 = +10.
        assert_eq!(s.cash, 100_000 + 1000 - s.pass);
    }

    #[test]
    fn full_press_place_six_presses_in_place() {
        let sel = only(|s| {
            s.place[2] = true; // place 6
            s.progression = Progression::FullPress;
        });
        let r = rules();
        let mut s = Session::new(&sel, &r, 1000, 100_000, false);
        s.resolve(2, 3); // point 5
        s.place_bets(); // place 6 for $12
        assert_eq!(s.place[2], 1200);
        let cash_before = s.cash;
        s.resolve(2, 4); // 6 rolls: win $14, press to $24
        assert_eq!(s.place[2], 2400);
        // $14 winnings minus the $12 press leaves $2 to the rail.
        assert_eq!(s.cash, cash_before + 1400 - 1200);
        s.resolve(3, 4); // seven-out: pressed bet lost, progression resets
        assert_eq!(s.place[2], 0);
        s.resolve(2, 3); // new point 5
        s.place_bets();
        assert_eq!(s.place[2], 1200); // back to base
    }

    #[test]
    fn progression_capped_at_table_max() {
        let sel = only(|s| {
            s.pass_line = true;
            s.progression = Progression::Martingale;
        });
        let r = Rules {
            table_max_mult: 4,
            ..rules()
        };
        let mut s = Session::new(&sel, &r, 1000, 10_000_000, false);
        for _ in 0..6 {
            s.place_bets();
            s.resolve(1, 1); // lose repeatedly
        }
        s.place_bets();
        assert_eq!(s.pass, 4000); // clamped at 4x the minimum
    }

    #[test]
    fn martingale_does_not_change_house_edge() {
        // Progressions change variance, not the edge per dollar wagered.
        // The wager-weighted estimator is dominated by rare deep-doubling
        // streaks, so it converges far slower than the flat-bet edge tests:
        // bound the tail with a realistic table max and average over seeds.
        let sel = only(|s| {
            s.pass_line = true;
            s.progression = Progression::Martingale;
        });
        let r = Rules {
            table_max_mult: 50,
            ..rules()
        };
        let mut net_sum = 0.0;
        let mut wagered_sum = 0.0;
        for seed in 0..8u64 {
            let (net, wagered) = grind_seeded(&sel, &r, 2_000_000, 500 + seed);
            net_sum += net;
            wagered_sum += wagered;
        }
        let edge = -net_sum / wagered_sum;
        assert!((edge - 0.01414).abs() < 0.005, "edge was {edge}");
    }

    #[test]
    fn oscars_grind_series_closes_at_one_unit() {
        let sel = only(|s| {
            s.pass_line = true;
            s.progression = Progression::OscarsGrind;
        });
        let r = rules();
        let mut s = Session::new(&sel, &r, 1000, 100_000, false);
        s.place_bets();
        s.resolve(1, 1); // lose $10 (stake holds at $10, profit -10)
        s.place_bets();
        assert_eq!(s.pass, 1000);
        s.resolve(3, 4); // win $10 (profit back to 0, raise to $20 capped at needed $10)
        s.place_bets();
        assert_eq!(s.pass, 1000); // capped: only $10 needed to close the series
        s.resolve(3, 4); // win: series +$10, reset
        s.place_bets();
        assert_eq!(s.pass, 1000);
        assert_eq!(s.cash, 100_000 + 1000 - s.pass);
    }

    /// Roll-by-roll trace of one session — a readable ledger of placements,
    /// resolutions, and the running bankroll. Run with:
    ///   cargo test --release -- --ignored trace_session --nocapture
    #[test]
    #[ignore]
    fn trace_session() {
        let seeds: Vec<u64> = std::env::var("TRACE_SEEDS")
            .ok()
            .map(|v| v.split(',').filter_map(|t| t.parse().ok()).collect())
            .unwrap_or_else(|| vec![11]);
        let sel = only(|s| {
            s.pass_line = true;
            s.come_max = 2;
            s.take_odds = true;
            s.progression = Progression::DAlembert;
        });
        let r = Rules {
            odds_policy: OddsPolicy::X345,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 500,
        };
        let min: i64 = std::env::var("TRACE_MIN")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .map(|d| d * 100)
            .unwrap_or(10_000); // dollars; default $100 table
        let budget = 100_000i64; // $1,000
        let target = budget * 2;
        for seed in seeds {
            println!();
            println!(
                "=== seed {seed}: ${} table, $1,000 budget, 3-pt Molly + 3-4-5x odds, D'Alembert, quit at $2,000 ===",
                min / 100
            );
            println!(
                "{:>4} {:>6} {:>7} {:>9} {:>9} {:>9} {:>9}  next flat (pass/come)",
                "roll", "dice", "point", "placed", "resolved", "cash", "total"
            );
            let mut rng = Xoshiro256pp::seed_from_u64(seed);
            let mut s = Session::new(&sel, &r, min, budget, false);
            let cheapest = s.cheapest_selected_stake();
            for roll in 1..=400u32 {
                let cash_before = s.cash;
                if s.needs_placement || s.one_roll_selected {
                    s.place_bets();
                }
                let placed = cash_before - s.cash;
                if !s.has_multi_roll_bets() && !s.has_one_roll_bets() && s.cash < cheapest {
                    println!(
                        "     BUSTED with ${} in hand after {} rolls",
                        s.cash / 100,
                        roll - 1
                    );
                    break;
                }
                let pre_wealth = s.cash + s.on_table_face();
                let (d1, d2) = rng.dice();
                s.resolve(d1, d2);
                let wealth = s.cash + s.on_table_face();
                let point = match s.point {
                    Some(p) => format!("on {p}"),
                    None => "off".to_owned(),
                };
                println!(
                    "{:>4} {:>6} {:>7} {:>9} {:>9} {:>9} {:>9}  ${}/{}",
                    roll,
                    format!("{d1}+{d2}={}", d1 + d2),
                    point,
                    format!("${}", placed / 100),
                    format!("{:+}", (wealth - pre_wealth) / 100),
                    format!("${}", s.cash / 100),
                    format!("${}", wealth / 100),
                    s.p_pass.stake / 100,
                    s.p_come.stake / 100,
                );
                if wealth >= target {
                    println!(
                        "     QUIT AHEAD with ${} total after {} rolls",
                        wealth / 100,
                        roll
                    );
                    break;
                }
            }
        }
    }

    /// Verbatim copy of the pre-merge `run_ruin_session` loop, kept as the
    /// behavioral reference for the merged `run_session`.
    fn reference_ruin_session(
        sel: &BetSelection,
        rules: &Rules,
        table_min_cents: i64,
        budget_cents: i64,
        quit_target_cents: Option<i64>,
        max_rolls: u64,
        seed: u64,
    ) -> RuinOutcome {
        let mut rng = Xoshiro256pp::seed_from_u64(seed);
        let mut s = Session::new(sel, rules, table_min_cents, budget_cents, false);
        let cheapest = s.cheapest_selected_stake();
        let mut rolls = 0u64;
        loop {
            s.place_bets();
            if !s.has_multi_roll_bets() && !s.has_one_roll_bets() && s.cash < cheapest {
                return RuinOutcome {
                    rolls,
                    censored: false,
                    hit_target: false,
                };
            }
            let (d1, d2) = rng.dice();
            rolls += 1;
            s.resolve(d1, d2);
            if let Some(target) = quit_target_cents {
                if s.cash + s.on_table_face() >= target {
                    return RuinOutcome {
                        rolls,
                        censored: false,
                        hit_target: true,
                    };
                }
            }
            if rolls >= max_rolls {
                return RuinOutcome {
                    rolls,
                    censored: true,
                    hit_target: false,
                };
            }
        }
    }

    /// Verbatim copy of the pre-merge `run_horizon_session` loop.
    fn reference_horizon_session(
        sel: &BetSelection,
        rules: &Rules,
        table_min_cents: i64,
        budget_cents: i64,
        quit_target_cents: Option<i64>,
        horizon_rolls: u64,
        seed: u64,
    ) -> HorizonOutcome {
        let mut rng = Xoshiro256pp::seed_from_u64(seed);
        let mut s = Session::new(sel, rules, table_min_cents, budget_cents, false);
        let cheapest = s.cheapest_selected_stake();
        let mut rolls = 0u64;
        while rolls < horizon_rolls {
            s.place_bets();
            if !s.has_multi_roll_bets() && !s.has_one_roll_bets() && s.cash < cheapest {
                return HorizonOutcome {
                    final_cents: s.cash,
                    busted: true,
                    hit_target: false,
                    rolls,
                };
            }
            let (d1, d2) = rng.dice();
            rolls += 1;
            s.resolve(d1, d2);
            if let Some(target) = quit_target_cents {
                let wealth = s.cash + s.on_table_face();
                if wealth >= target {
                    return HorizonOutcome {
                        final_cents: wealth,
                        busted: false,
                        hit_target: true,
                        rolls,
                    };
                }
            }
        }
        HorizonOutcome {
            final_cents: s.cash + s.on_table_face(),
            busted: false,
            hit_target: false,
            rolls,
        }
    }

    /// A varied battery of player configurations for equivalence proofs:
    /// every bet family, several progressions, quit rules on and off.
    fn equivalence_battery() -> Vec<(BetSelection, Rules, Option<i64>)> {
        let mut cfgs = Vec::new();
        cfgs.push((only(|s| s.pass_line = true), rules(), None));
        cfgs.push((
            only(|s| {
                s.pass_line = true;
                s.take_odds = true;
            }),
            Rules {
                odds_policy: OddsPolicy::X345,
                ..rules()
            },
            None,
        ));
        cfgs.push((
            only(|s| {
                s.pass_line = true;
                s.come_max = 2;
                s.take_odds = true;
            }),
            Rules {
                odds_policy: OddsPolicy::X2,
                ..rules()
            },
            Some(20_000),
        ));
        cfgs.push((
            only(|s| {
                s.dont_pass = true;
                s.dont_come_max = 2;
                s.take_odds = true;
            }),
            Rules {
                odds_policy: OddsPolicy::X345,
                come_odds_work_on_comeout: true,
                ..rules()
            },
            None,
        ));
        cfgs.push((
            only(|s| {
                s.set_place(6, true);
                s.set_place(8, true);
                s.progression = Progression::FullPress;
            }),
            rules(),
            None,
        ));
        cfgs.push((
            only(|s| {
                s.field = true;
                s.set_place(5, true);
                s.set_place(6, true);
                s.set_place(8, true);
                s.progression = Progression::Martingale;
            }),
            Rules {
                field_12_triple: true,
                table_max_mult: 20,
                ..rules()
            },
            Some(15_000),
        ));
        cfgs.push((
            only(|s| {
                s.hardways = [true; 4];
                s.any_seven = true;
                s.any_craps = true;
                s.progression = Progression::GrandMartingale;
            }),
            rules(),
            None,
        ));
        cfgs.push((
            only(|s| {
                s.pass_line = true;
                s.dont_pass = true;
                s.come_max = 3;
                s.dont_come_max = 1;
                s.take_odds = true;
                s.field = true;
                s.place = [true; 6];
                s.hardways = [true; 4];
                s.any_seven = true;
                s.any_craps = true;
                s.progression = Progression::Fibonacci;
            }),
            Rules {
                odds_policy: OddsPolicy::X10,
                ..rules()
            },
            Some(30_000),
        ));
        cfgs
    }

    #[test]
    fn merged_session_matches_split_references() {
        // The merged run_session must reproduce the pre-merge ruin and
        // horizon loops exactly, seed for seed, field for field — including
        // when the ruin cap falls before the horizon.
        let min = 1000;
        let horizon = 400;
        for (ci, (sel, r, quit)) in equivalence_battery().iter().enumerate() {
            for &budget in &[5_000i64, 30_000] {
                for seed in 0..300u64 {
                    let ctx = format!("config {ci} budget {budget} seed {seed}");

                    // Standard shape: cap well beyond the horizon.
                    let m = run_session(sel, r, min, budget, *quit, 2_000, horizon, seed);
                    let rr = reference_ruin_session(sel, r, min, budget, *quit, 2_000, seed);
                    let rh = reference_horizon_session(sel, r, min, budget, *quit, horizon, seed);
                    assert_eq!(m.ruin, rr, "ruin mismatch: {ctx}");
                    assert_eq!(m.horizon, rh, "horizon mismatch: {ctx}");

                    // Cap before the horizon: ruin censors early, horizon
                    // plays on.
                    let m = run_session(sel, r, min, budget, *quit, 300, horizon, seed);
                    let rr = reference_ruin_session(sel, r, min, budget, *quit, 300, seed);
                    assert_eq!(m.ruin, rr, "capped ruin mismatch: {ctx}");
                    assert_eq!(m.horizon, rh, "capped horizon mismatch: {ctx}");

                    // Public wrappers.
                    let w = run_ruin_session(sel, r, min, budget, *quit, 2_000, seed);
                    assert_eq!(
                        w,
                        rr_standard(sel, r, min, budget, *quit, seed),
                        "wrapper ruin: {ctx}"
                    );
                    let w = run_horizon_session(sel, r, min, budget, *quit, horizon, seed);
                    assert_eq!(w, rh, "wrapper horizon: {ctx}");
                }
            }
        }

        fn rr_standard(
            sel: &BetSelection,
            r: &Rules,
            min: i64,
            budget: i64,
            quit: Option<i64>,
            seed: u64,
        ) -> RuinOutcome {
            reference_ruin_session(sel, r, min, budget, quit, 2_000, seed)
        }
    }

    #[test]
    fn sessions_share_dice_across_rule_variants() {
        // Common-random-numbers invariant: sessions with the same seed play
        // the same dice, so a session that never triggers its quit rule must
        // end exactly like the same session with no quit rule at all. This
        // is what makes explorer comparisons luck-free.
        let sel = only(|s| {
            s.pass_line = true;
            s.take_odds = true;
        });
        let r = Rules {
            odds_policy: OddsPolicy::X345,
            ..rules()
        };
        let mut hits = 0;
        for seed in 0..500u64 {
            let with_quit = run_horizon_session(&sel, &r, 1000, 30_000, Some(60_000), 400, seed);
            let without = run_horizon_session(&sel, &r, 1000, 30_000, None, 400, seed);
            if with_quit.hit_target {
                hits += 1;
                assert!(with_quit.final_cents >= 60_000, "seed {seed}");
                assert!(with_quit.rolls <= without.rolls, "seed {seed}");
            } else {
                assert_eq!(with_quit, without, "seed {seed}");
            }
        }
        // The invariant must have been exercised from both sides.
        assert!(
            hits > 0 && hits < 500,
            "degenerate battery: {hits}/500 hits"
        );
    }

    #[test]
    fn quit_target_matches_gamblers_ruin() {
        // $10 pass line, $100 budget, quit at $200: a ±1-unit random walk from
        // 10 aiming for 20 with win probability p = 244/495. Classic gambler's
        // ruin: P(hit target) = (1 - r^10)/(1 - r^20) with r = q/p ≈ 1.0287,
        // which evaluates to ≈ 0.4298.
        let sel = only(|s| s.pass_line = true);
        let r = rules();
        let n = 100_000u64;
        let mut hits = 0u64;
        let mut quit_finals_ok = true;
        for seed in 0..n {
            let o = run_ruin_session(&sel, &r, 1000, 10_000, Some(20_000), 1_000_000, seed);
            assert!(!o.censored);
            hits += o.hit_target as u64;
            // The horizon variant must report the same end state and include
            // the quit winnings in the final bankroll.
            let h = run_horizon_session(&sel, &r, 1000, 10_000, Some(20_000), 1_000_000, seed);
            if h.hit_target != o.hit_target || (h.hit_target && h.final_cents < 20_000) {
                quit_finals_ok = false;
            }
        }
        let frac = hits as f64 / n as f64;
        assert!(
            (frac - 0.4298).abs() < 0.01,
            "target-hit fraction was {frac}"
        );
        assert!(quit_finals_ok);
    }

    #[test]
    fn drawdown_matches_ruin_boundary() {
        // With budget exactly equal to a session's max outlay the player can
        // fund the full strategy on every roll of that same dice sequence, so
        // they must survive the horizon. (A smaller budget may still survive
        // by falling back to base bets, so sufficiency is the guarantee.)
        let sel = only(|s| {
            s.pass_line = true;
            s.field = true;
            s.come_max = 2;
            s.take_odds = true;
        });
        let r = Rules {
            odds_policy: OddsPolicy::X345,
            ..rules()
        };
        for seed in 0..50u64 {
            let horizon = 400;
            let outlay = run_drawdown_session(&sel, &r, 1000, horizon, seed);
            let survive = run_ruin_session(&sel, &r, 1000, outlay, None, horizon, seed);
            assert!(
                survive.rolls >= horizon,
                "seed {seed}: died at {} with budget = outlay {outlay}",
                survive.rolls
            );
        }
    }

    #[test]
    fn place_bet_payouts_and_stake_units() {
        // A $12 minimum forces unit rounding: 4/5/9/10 round up to $15 ($5
        // units), 6/8 stay at $12 ($6 units).
        assert_eq!(place_stake(1200, 4), 1500);
        assert_eq!(place_stake(1200, 5), 1500);
        assert_eq!(place_stake(1200, 6), 1200);
        assert_eq!(place_stake(1200, 8), 1200);
        assert_eq!(place_stake(1200, 9), 1500);
        assert_eq!(place_stake(1200, 10), 1500);
        // Payout ratios: 9:5 on 4/10, 7:5 on 5/9, 7:6 on 6/8, exact in cents.
        assert_eq!(place_win(1500, 4), 2700);
        assert_eq!(place_win(1500, 10), 2700);
        assert_eq!(place_win(1500, 5), 2100);
        assert_eq!(place_win(1500, 9), 2100);
        assert_eq!(place_win(1200, 6), 1400);
        assert_eq!(place_win(1200, 8), 1400);

        // Full resolution paths for the untested numbers: place 4 and place 9.
        let sel = only(|s| {
            s.set_place(4, true);
            s.set_place(9, true);
        });
        let r = rules();
        let mut s = Session::new(&sel, &r, 1200, 100_000, false);
        s.resolve(3, 3); // point 6 so place bets go working
        s.place_bets();
        assert_eq!(s.place[0], 1500); // the 4
        assert_eq!(s.place[4], 1500); // the 9
        let cash = s.cash;
        s.resolve(2, 2); // 4 rolls: 9:5 on $15 pays $27, bet stays up
        assert_eq!(s.cash, cash + 2700);
        assert_eq!(s.place[0], 1500);
        let cash = s.cash;
        s.resolve(4, 5); // 9 rolls: 7:5 on $15 pays $21
        assert_eq!(s.cash, cash + 2100);
        s.resolve(2, 5); // seven-out: both working place bets lose
        assert_eq!(s.place[0], 0);
        assert_eq!(s.place[4], 0);
    }

    #[test]
    fn hardway_mechanics() {
        let sel = only(|s| s.hardways = [true; 4]);
        let r = rules(); // $5 props
        let mut s = Session::new(&sel, &r, 1000, 100_000, false);
        s.resolve(2, 3); // point 5: hardways go working
        s.place_bets();
        assert_eq!(s.hard, [500; 4]);
        let cash = s.cash;
        s.resolve(4, 4); // hard 8: 9:1, bet stays up
        assert_eq!(s.cash, cash + 4500);
        assert_eq!(s.hard[2], 500);
        let cash = s.cash;
        s.resolve(2, 2); // hard 4: 7:1
        assert_eq!(s.cash, cash + 3500);
        s.resolve(6, 4); // easy 10: hard 10 loses, others unaffected
        assert_eq!(s.hard[3], 0);
        assert_eq!(s.hard[1], 500);
        s.resolve(2, 5); // seven-out: all remaining hardways lose
        assert_eq!(s.hard, [0; 4]);

        // Off on come-out: an easy 6 during come-out must not take the bet.
        let mut s = Session::new(&sel, &r, 1000, 100_000, false);
        s.resolve(2, 4); // point 6... wait: that's the come-out, sets point
        s.place_bets();
        assert_eq!(s.hard[1], 500);
        s.resolve(3, 3); // hard 6 while point 6: hardway wins AND point made
        assert_eq!(s.point, None);
        let hard6 = s.hard[1];
        assert_eq!(hard6, 500); // stays up
        let cash = s.cash;
        s.resolve(2, 4); // easy 6 on the come-out: hardways are OFF — no loss
        assert_eq!(s.hard[1], 500);
        assert_eq!(s.cash, cash);
    }

    #[test]
    fn one_roll_prop_payouts() {
        let sel = only(|s| {
            s.any_seven = true;
            s.any_craps = true;
        });
        let r = rules(); // $5 props
        let mut s = Session::new(&sel, &r, 1000, 100_000, false);
        s.place_bets();
        assert_eq!(s.cash, 100_000 - 1000);
        let cash = s.cash;
        s.resolve(3, 4); // seven: any-seven pays 4:1 ($25 back), any-craps loses
        assert_eq!(s.cash, cash + 2500);
        s.place_bets();
        let cash = s.cash;
        s.resolve(1, 1); // craps: any-craps pays 7:1 ($40 back), any-seven loses
        assert_eq!(s.cash, cash + 4000);
        s.place_bets();
        let cash = s.cash;
        s.resolve(2, 3); // 5: both lose
        assert_eq!(s.cash, cash);
    }

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

    #[test]
    fn pressed_flats_round_to_whole_dollars() {
        // Half Press raw stakes go 1000 -> 1500 -> 2250; the table takes
        // whole-dollar flats, so the third bet must be $23, not $22.50.
        let sel = only(|s| {
            s.pass_line = true;
            s.progression = Progression::HalfPress;
        });
        let r = rules();
        let mut s = Session::new(&sel, &r, 1000, 1_000_000, false);
        s.place_bets();
        assert_eq!(s.pass, 1000);
        s.resolve(3, 4); // natural: win
        s.place_bets();
        assert_eq!(s.pass, 1500);
        s.resolve(5, 6); // 11: win
        s.place_bets();
        assert_eq!(s.pass, 2300); // 2250 rounded to the nearest dollar
    }

    #[test]
    fn stats_functions_on_known_inputs() {
        // Percentiles over 1..=100: index = round(p/100 * 99).
        let mut rolls: Vec<u64> = (1..=100).collect();
        let st = summarize_ruin(&mut rolls, 10, 5);
        assert_eq!(st.mean, 50.5);
        assert_eq!(st.p5, 6);
        assert_eq!(st.p25, 26);
        assert_eq!(st.p50, 51);
        assert_eq!(st.p75, 75);
        assert_eq!(st.p95, 95);
        assert_eq!(st.censored_frac, 0.10);
        assert_eq!(st.target_frac, 0.05);

        let mut one = vec![42u64];
        let st = summarize_ruin(&mut one, 0, 0);
        assert_eq!(st.mean, 42.0);
        assert_eq!(st.ci95_half, 0.0);
        assert_eq!((st.p5, st.p50, st.p95), (42, 42, 42));

        // Deciles over 1..=100: index = round(k/10 * 99).
        let mut finals: Vec<i64> = (1..=100).collect();
        let h = summarize_horizon(&mut finals, 20, 30);
        assert_eq!(h.mean, 50.5);
        assert_eq!(h.bust_frac, 0.20);
        assert_eq!(h.target_frac, 0.30);
        assert_eq!(h.deciles, [11, 21, 31, 41, 51, 60, 70, 80, 90]);

        // Budget percentile: index = ceil(conf/100 * 99).
        let mut outlays: Vec<i64> = (1..=100).collect();
        assert_eq!(budget_for_confidence(&mut outlays, 95.0), 96);
        assert_eq!(budget_for_confidence(&mut outlays, 100.0), 100);
        let mut single = vec![777i64];
        assert_eq!(budget_for_confidence(&mut single, 95.0), 777);
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use rayon::prelude::*;

    #[test]
    #[ignore] // manual benchmark: cargo test --release -- --ignored --nocapture
    fn throughput() {
        let sel = BetSelection::default(); // pass line only
        let r = Rules {
            odds_policy: OddsPolicy::None,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 1000,
        };
        let n: u64 = 100_000;
        let start = std::time::Instant::now();
        let mut rolls: Vec<u64> = (0..n)
            .into_par_iter()
            .map(|i| run_ruin_session(&sel, &r, 1000, 30_000, None, 200_000, i).rolls)
            .collect();
        let censored = rolls.iter().filter(|&&x| x >= 200_000).count() as u64;
        let stats = summarize_ruin(&mut rolls, censored, 0);
        let dt = start.elapsed().as_secs_f64();
        let total_rolls: u64 = rolls.iter().sum();
        println!(
            "{n} sessions, {total_rolls} rolls in {dt:.2}s ({:.1}M rolls/s) mean={:.0}±{:.0} median={} p5={} p95={} censored={:.2}%",
            total_rolls as f64 / dt / 1e6,
            stats.mean, stats.ci95_half, stats.p50, stats.p5, stats.p95,
            stats.censored_frac * 100.0
        );
    }
}
