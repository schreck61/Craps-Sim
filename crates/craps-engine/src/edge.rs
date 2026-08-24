// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Closed-form house edges per resolved dollar, and the flat-stakes handle
//! rates that weight them into a blended edge and a wealth drift per roll.
//!
//! # The push convention (E3), stated once, here
//!
//! `Session::resolved_wagered_cents` accumulates the stake of every bet the
//! moment it is resolved by a **win or a loss**. Pushes are **excluded** from
//! resolved handle. Every push site in the engine:
//!
//! * **Don't pass, come-out 12** (`game.rs`, come-out arm `t == 12`): the bet
//!   is barred, stays up, and adds nothing to the handle.
//! * **Don't come flat, box 12** (`resolve_come_bets`, `dc_flat` arm
//!   `t == 12`): same bar, the flat stays in the box unresolved.
//! * **Come odds returned while off** (`resolve_come_bets`, both the
//!   seven-out and point-made arms when `come_odds_working` is false): the
//!   odds are handed back, not resolved.
//!
//! Every closed form below prices the house edge **per resolved dollar over
//! the bet's resolution set with pushes excluded**, so simulated
//! `-net / resolved_wagered_cents` converges onto these exact rationals. The
//! don't-pass figure is the push-exclusive −27/1925 ≈ −1.403%, not the
//! per-decision −27/1980.
//!
//! # Scope
//!
//! Handle rates model **flat stakes** (no progression) with an **unbounded
//! bankroll** — the same assumptions as the amber Horizon line (E5): bust at
//! $0 truncates losses, so a budgeted run's empirical mean sits above
//! `budget + drift × horizon` whenever bust mass is material.

use crate::bets::{
    dont_lay_for_win, place_index, place_stake, BetSelection, Rules, HARD_NUMS, PLACE_NUMS,
};

/// An exact rational, the form every closed-form edge is stated in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frac {
    pub num: i64,
    pub den: i64,
}

impl Frac {
    pub fn as_f64(&self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

/// A bet family the closed forms cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeBet {
    Pass,
    DontPass,
    Come,
    DontCome,
    Field,
    Place(u8),
    Hardway(u8),
    AnySeven,
    AnyCraps,
    PassOdds,
    DontPassLay,
}

/// House edge per RESOLVED dollar (pushes excluded), as an exact rational.
/// Negative for the player. `rules` matters for Field (triple 12).
///
/// Derivations (probability-weighted outcome over the resolution set, pushes
/// excluded; all stakes 1):
///
/// * **Pass / Come** — P(win) = 8/36 naturals + Σₚ (wₚ/36)·wₚ/(wₚ+6) over
///   points p with wₚ ways = 244/495; every decision resolves, so the edge is
///   (244 − 251)/495 = **−7/495** ≈ −1.414%.
/// * **Don't pass / Don't come** — per 1980 come-out 36ths×55: win 949
///   (2, 3, and seven-outs), lose 976 (7, 11, points made), push 55 (the
///   barred 12, excluded). Per resolved dollar: (949 − 976)/(949 + 976) =
///   **−27/1925** ≈ −1.403%.
/// * **Field** — per 36 rolls: 14 pay 1:1 (3,4,9,10,11), the 2 pays 2:1, the
///   12 pays 2:1 or 3:1, 20 lose. Double 12: (14+2+2−20)/36 = **−1/18**;
///   triple 12: (14+2+3−20)/36 = **−1/36**. Every roll resolves.
/// * **Place 6/8** — resolution set {6-or-8, 7} = 11 ways: 5 win at 7:6,
///   6 lose: (5·7/6 − 6)/11 = **−1/66** per resolved dollar per resolution.
/// * **Place 5/9** — 10 ways: 4 win at 7:5: (4·7/5 − 6)/10 = **−1/25**.
/// * **Place 4/10** — 9 ways: 3 win at 9:5: (3·9/5 − 6)/9 = **−1/15**.
/// * **Hardway 6/8** — resolution set {hard, easy, 7} = 1+4+6 = 11 ways:
///   (1·9 − 10)/11 = **−1/11**.
/// * **Hardway 4/10** — 1+2+6 = 9 ways: (1·7 − 8)/9 = **−1/9**.
/// * **Any seven** — (6·4 − 30)/36 = **−1/6**.
/// * **Any craps** — (4·7 − 32)/36 = **−1/9**.
/// * **Pass odds / Don't pass lay** — paid at true odds: exactly **0**.
///
/// # Panics
///
/// Panics if `Place`/`Hardway` carries a number that isn't one.
pub fn bet_edge(bet: EdgeBet, rules: &Rules) -> Frac {
    match bet {
        EdgeBet::Pass | EdgeBet::Come => Frac { num: -7, den: 495 },
        EdgeBet::DontPass | EdgeBet::DontCome => Frac {
            num: -27,
            den: 1925,
        },
        EdgeBet::Field => {
            if rules.field_12_triple {
                Frac { num: -1, den: 36 }
            } else {
                Frac { num: -1, den: 18 }
            }
        }
        EdgeBet::Place(n) => match n {
            6 | 8 => Frac { num: -1, den: 66 },
            5 | 9 => Frac { num: -1, den: 25 },
            4 | 10 => Frac { num: -1, den: 15 },
            _ => panic!("not a place number"),
        },
        EdgeBet::Hardway(n) => match n {
            6 | 8 => Frac { num: -1, den: 11 },
            4 | 10 => Frac { num: -1, den: 9 },
            _ => panic!("not a hardway number"),
        },
        EdgeBet::AnySeven => Frac { num: -1, den: 6 },
        EdgeBet::AnyCraps => Frac { num: -1, den: 9 },
        EdgeBet::PassOdds | EdgeBet::DontPassLay => Frac { num: 0, den: 1 },
    }
}

// ---------------------------------------------------------------------------
// Flat-stakes handle rates (resolved cents per roll), derived from the pass
// point cycle.
//
// One pass cycle = 1 come-out roll + point rolls. A point is established with
// probability 24/36; conditional on point p the point phase lasts 36/(wₚ+6)
// rolls, so E[extra rolls] = Σₚ (wₚ/36)·36/(wₚ+6) = 2/3 + 4/5 + 10/11 =
// 392/165, and a cycle averages 1 + 392/165 = 557/165 rolls. Hence:
//
// * line decisions per roll = 165/557,
// * exactly one come-out roll per cycle ⇒ point-on fraction = 392/557,
// * points established per roll = (165/557)·(24/36) = 110/557,
// * don't-pass resolutions per roll = (165/557)·(35/36): of all cycle ends,
//   the 1/36 that are come-out 12s bar the don't and are excluded.
// ---------------------------------------------------------------------------

/// Line-bet decisions per roll: 165/557.
const PASS_RES_PER_ROLL: f64 = 165.0 / 557.0;
/// Fraction of rolls thrown while a point is on: 392/557.
const POINT_ON_FRAC: f64 = 392.0 / 557.0;
/// Points established (equivalently pass-odds / don't-lay resolutions) per
/// roll: 110/557.
const POINT_EST_PER_ROLL: f64 = 110.0 / 557.0;
/// Don't-pass resolutions per roll: (165/557)·(35/36) — bar-12 come-outs end
/// a pass cycle but push (and are excluded from handle) on the don't side.
const DONT_RES_PER_ROLL: f64 = PASS_RES_PER_ROLL * (35.0 / 36.0);

/// Ways to roll a box number.
fn ways(num: u8) -> i64 {
    match num {
        6 | 8 => 5,
        5 | 9 => 4,
        _ => 3, // 4 | 10
    }
}

/// Probability that a selected place bet on `num` is still up (carried over)
/// at the moment a point is established.
///
/// The engine never makes a fresh place bet on the current point
/// (`num != point` in `place_bets`), but a bet already up from an earlier
/// point cycle stays up and keeps working even when the new point is its own
/// number. So the bet is up on *every* roll of a point-p cycle when p ≠ num,
/// and up with this carried probability π when p = num. π satisfies the
/// establishment-to-establishment recursion (the bet survives a cycle iff it
/// was effectively up and the point was made; a down bet at its own number
/// stays down all cycle):
///
///   π = Σ_{p≠N} aₚ·mₚ + a_N·π·m_N,   aₚ = wₚ/24,  mₚ = wₚ/(wₚ+6),
///
/// giving exactly π = 411/1195 for 6/8, 4/11 for 5/9, 481/1265 for 4/10.
fn place_carry_prob(num: u8) -> f64 {
    let mut made_except = 0.0; // Σ_{p≠num} aₚ·mₚ
    let mut made_own = 0.0; // a_num·m_num
    for &p in &PLACE_NUMS {
        let w = ways(p) as f64;
        let term = (w / 24.0) * (w / (w + 6.0));
        if p == num {
            made_own += term;
        } else {
            made_except += term;
        }
    }
    made_except / (1.0 - made_own)
}

/// Resolutions per roll of a selected place bet on `num`, at flat stakes.
///
/// While up during a point-p cycle the bet resolves on `num` or 7, i.e.
/// (w_N+6)/36 per point-on roll, and a point-p cycle averages 36/(wₚ+6)
/// point-on rolls — so it resolves (w_N+6)/(wₚ+6) times per point-p cycle.
/// Cycles establish a point with probability 2/3; the bet is always up when
/// p ≠ num and up with probability [`place_carry_prob`] when p = num:
///
///   rate = (165/557)·(2/3)·[ Σ_{p≠N} aₚ·(w_N+6)/(wₚ+6) + a_N·π_N ].
fn place_res_per_roll(num: u8) -> f64 {
    let pi = place_carry_prob(num);
    let rn = (ways(num) + 6) as f64;
    let mut per_point_cycle = 0.0;
    for &p in &PLACE_NUMS {
        let ap = ways(p) as f64 / 24.0;
        per_point_cycle += if p == num {
            ap * pi
        } else {
            ap * rn / ((ways(p) + 6) as f64)
        };
    }
    PASS_RES_PER_ROLL * (2.0 / 3.0) * per_point_cycle
}

// ---------------------------------------------------------------------------
// Come / don't-come queues.
//
// A capped come queue has no simple closed form, but at flat stakes with an
// unbounded bankroll its per-roll dynamics are an exact finite Markov chain,
// which we solve numerically for its stationary distribution (power
// iteration to < 1e-13; the only modeling assumptions are the flat/unbounded
// ones shared by this whole module — validated against simulation in the
// test suite). The chain replicates the engine exactly:
//
// * A come flat placed pre-roll always leaves the box on that same roll
//   (win 7/11, lose craps, or travel), so the box is empty at every pre-roll
//   and a new flat goes down whenever the point is on and fewer than
//   `come_max` numbers are occupied. Travel can never stack: an occupied
//   number rolled resolves (wins) before the flat travels onto it.
// * A don't-come flat can push in the box on the barred 12, so its chain
//   carries a box bit; DC points lose when their number repeats (before the
//   box travels) and all win on any 7.
// * State = (main point ∈ {off, 4,5,6,8,9,10}) × (occupied-number set), plus
//   the DC box bit: 448 / 896 states.
//
// From the stationary distribution: flat placements per roll (each placed
// flat resolves exactly once — DC box pushes only delay — so this is also
// the resolved-flat rate), and the odds/lay resolved-handle rate. Come odds
// resolve only when the come point decides on a non-come-out roll (else they
// are returned: a push, excluded); don't-come lay odds always work.
// ---------------------------------------------------------------------------

/// Ways to roll each total 2..=12, over 36.
const T_WAYS: [f64; 11] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0];

/// Main-point transition: 0 = off, 1..=6 = index+1 into [`PLACE_NUMS`].
fn next_main(mp: usize, t: u8) -> usize {
    if mp == 0 {
        match place_index(t) {
            Some(i) => i + 1,
            None => 0,
        }
    } else if t == 7 || t == PLACE_NUMS[mp - 1] {
        0
    } else {
        mp
    }
}

/// Stationary distribution of a per-roll chain by power iteration.
fn stationary(trans: &[Vec<(usize, f64)>], start: usize) -> Vec<f64> {
    let n = trans.len();
    let mut v = vec![0.0; n];
    v[start] = 1.0;
    let mut w = vec![0.0; n];
    for _ in 0..20_000 {
        w.iter_mut().for_each(|x| *x = 0.0);
        for (i, row) in trans.iter().enumerate() {
            let p = v[i];
            if p > 0.0 {
                for &(j, q) in row {
                    w[j] += p * q;
                }
            }
        }
        let delta: f64 = v.iter().zip(&w).map(|(a, b)| (a - b).abs()).sum();
        std::mem::swap(&mut v, &mut w);
        if delta < 1e-14 {
            break;
        }
    }
    v
}

/// Flat placements per roll and odds resolved-handle cents per roll for the
/// come queue with `come_max = cap`.
fn come_queue_rates(cap: u8, rules: &Rules, min: i64, take_odds: bool) -> (f64, f64) {
    // |S| ≤ 6, so any cap ≥ 7 never binds.
    let cap = cap.min(7) as u32;
    let mut trans: Vec<Vec<(usize, f64)>> = Vec::with_capacity(7 * 64);
    for mp in 0..7usize {
        for s in 0..64usize {
            let placing = mp != 0 && (s.count_ones()) < cap;
            let mut row = Vec::with_capacity(11);
            for (ti, &w) in T_WAYS.iter().enumerate() {
                let t = (ti + 2) as u8;
                let mut s2 = s;
                if t == 7 {
                    s2 = 0; // every come point loses on any 7
                } else if let Some(i) = place_index(t) {
                    s2 &= !(1 << i); // occupied number rolled: point wins, down
                }
                if placing {
                    if let Some(i) = place_index(t) {
                        s2 |= 1 << i; // the box flat travels (7/11/craps end it)
                    }
                }
                row.push((next_main(mp, t) * 64 + s2, w / 36.0));
            }
            trans.push(row);
        }
    }
    let v = stationary(&trans, 0);
    let mut placements = 0.0;
    let mut odds_handle = 0.0;
    for (state, &p) in v.iter().enumerate() {
        if p <= 0.0 {
            continue;
        }
        let (mp, s) = (state / 64, state % 64);
        if mp != 0 && (s.count_ones()) < cap {
            placements += p;
        }
        // Come odds resolve with their point only while working: on come-out
        // rolls they are returned (a push) unless the rules keep them on.
        if take_odds && (mp != 0 || rules.come_odds_work_on_comeout) {
            for (i, &num) in PLACE_NUMS.iter().enumerate() {
                if s & (1 << i) != 0 {
                    let mult = rules.odds_policy.multiple(num);
                    if mult > 0 {
                        let res = (ways(num) + 6) as f64 / 36.0;
                        odds_handle += p * res * (min * mult) as f64;
                    }
                }
            }
        }
    }
    (placements, odds_handle)
}

/// Flat placements per roll and lay resolved-handle cents per roll for the
/// don't-come queue with `dont_come_max = cap`.
fn dc_queue_rates(cap: u8, rules: &Rules, min: i64, take_odds: bool) -> (f64, f64) {
    let cap = cap.min(7) as u32;
    let mut trans: Vec<Vec<(usize, f64)>> = Vec::with_capacity(7 * 64 * 2);
    for mp in 0..7usize {
        for s in 0..64usize {
            for b in 0..2usize {
                let place_now = mp != 0 && b == 0 && (s.count_ones()) < cap;
                let boxed = b == 1 || place_now;
                let mut row = Vec::with_capacity(11);
                for (ti, &w) in T_WAYS.iter().enumerate() {
                    let t = (ti + 2) as u8;
                    let mut s2 = s;
                    if t == 7 {
                        s2 = 0; // every DC point wins on any 7
                    } else if let Some(i) = place_index(t) {
                        s2 &= !(1 << i); // number repeated: DC point loses
                    }
                    let mut b2 = 0usize;
                    if boxed {
                        match t {
                            2 | 3 | 7 | 11 => {} // box flat decides
                            12 => b2 = 1,        // barred: pushes, stays in the box
                            _ => s2 |= 1 << place_index(t).expect("box number"),
                        }
                    }
                    row.push(((next_main(mp, t) * 64 + s2) * 2 + b2, w / 36.0));
                }
                trans.push(row);
            }
        }
    }
    let v = stationary(&trans, 0);
    let mut placements = 0.0;
    let mut lay_handle = 0.0;
    for (state, &p) in v.iter().enumerate() {
        if p <= 0.0 {
            continue;
        }
        let (rest, b) = (state / 2, state % 2);
        let (mp, s) = (rest / 64, rest % 64);
        if mp != 0 && b == 0 && (s.count_ones()) < cap {
            placements += p;
        }
        if take_odds {
            // Lay odds always work; they resolve whenever their number or a
            // 7 rolls, sized by the engine's dont_lay_for_win.
            for (i, &num) in PLACE_NUMS.iter().enumerate() {
                if s & (1 << i) != 0 {
                    let mult = rules.odds_policy.multiple(num);
                    if mult > 0 {
                        let stake = dont_lay_for_win(min * mult, num);
                        let res = (ways(num) + 6) as f64 / 36.0;
                        lay_handle += p * res * stake as f64;
                    }
                }
            }
        }
    }
    (placements, lay_handle)
}

// ---------------------------------------------------------------------------
// Assembly.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Acc {
    drift: f64,
    handle: f64,
}

impl Acc {
    fn add(&mut self, edge: Frac, handle_cents_per_roll: f64) {
        self.handle += handle_cents_per_roll;
        self.drift += edge.as_f64() * handle_cents_per_roll;
    }
}

const ZERO: Frac = Frac { num: 0, den: 1 };

/// Per-roll resolved handle and drift, in cents, for a whole selection at
/// flat stakes. Component stakes and rates:
///
/// * pass/don't flats: table min at 165/557 (don't: ×35/36) per roll;
/// * pass odds: min × policy multiple of the established point, resolving at
///   110/557 per roll (points 4/10 w.p. 6/24, 5/9 8/24, 6/8 10/24 — for
///   3-4-5× the point-conditional expected stake is min·(18+32+50)/24 =
///   min·25/6); don't lay the same rate with `dont_lay_for_win` sizing;
/// * field every roll at min; any seven / any craps every roll at the prop
///   stake; hardways at the prop stake on the point-on fraction 392/557 of
///   rolls, resolving (ways+6)/36 of those;
/// * place bets at their unit-rounded stake, at [`place_res_per_roll`];
/// * come / don't-come flats and odds from the exact queue chains.
fn flat_rates(sel: &BetSelection, rules: &Rules, min: i64) -> Acc {
    let mut a = Acc::default();
    if sel.pass_line {
        a.add(
            bet_edge(EdgeBet::Pass, rules),
            PASS_RES_PER_ROLL * min as f64,
        );
        if sel.take_odds {
            for &num in &PLACE_NUMS {
                let mult = rules.odds_policy.multiple(num);
                if mult > 0 {
                    let ap = ways(num) as f64 / 24.0;
                    a.add(ZERO, POINT_EST_PER_ROLL * ap * (min * mult) as f64);
                }
            }
        }
    }
    if sel.dont_pass {
        a.add(
            bet_edge(EdgeBet::DontPass, rules),
            DONT_RES_PER_ROLL * min as f64,
        );
        if sel.take_odds {
            for &num in &PLACE_NUMS {
                let mult = rules.odds_policy.multiple(num);
                if mult > 0 {
                    let ap = ways(num) as f64 / 24.0;
                    let stake = dont_lay_for_win(min * mult, num);
                    a.add(ZERO, POINT_EST_PER_ROLL * ap * stake as f64);
                }
            }
        }
    }
    if sel.come_max > 0 {
        let (flats, odds) = come_queue_rates(sel.come_max, rules, min, sel.take_odds);
        a.add(bet_edge(EdgeBet::Come, rules), flats * min as f64);
        a.add(ZERO, odds);
    }
    if sel.dont_come_max > 0 {
        let (flats, lay) = dc_queue_rates(sel.dont_come_max, rules, min, sel.take_odds);
        a.add(bet_edge(EdgeBet::DontCome, rules), flats * min as f64);
        a.add(ZERO, lay);
    }
    if sel.field {
        a.add(bet_edge(EdgeBet::Field, rules), min as f64);
    }
    for (i, &on) in sel.place.iter().enumerate() {
        if on {
            let num = PLACE_NUMS[i];
            a.add(
                bet_edge(EdgeBet::Place(num), rules),
                place_res_per_roll(num) * place_stake(min, num) as f64,
            );
        }
    }
    for (i, &on) in sel.hardways.iter().enumerate() {
        if on {
            let num = HARD_NUMS[i];
            let res = POINT_ON_FRAC * (ways(num) + 6) as f64 / 36.0;
            a.add(
                bet_edge(EdgeBet::Hardway(num), rules),
                res * rules.prop_bet_cents as f64,
            );
        }
    }
    if sel.any_seven {
        a.add(
            bet_edge(EdgeBet::AnySeven, rules),
            rules.prop_bet_cents as f64,
        );
    }
    if sel.any_craps {
        a.add(
            bet_edge(EdgeBet::AnyCraps, rules),
            rules.prop_bet_cents as f64,
        );
    }
    a
}

/// Handle-weighted per-resolved-dollar edge of a whole selection (f64,
/// negative), or None if nothing is selected. Weights are per-roll resolved-
/// handle rates at flat stakes at the given table minimum (place-bet stakes
/// use their payout-unit rounding, props use rules.prop_bet_cents).
pub fn blended_edge(sel: &BetSelection, rules: &Rules, table_min_cents: i64) -> Option<f64> {
    if !sel.any_selected() {
        return None;
    }
    let a = flat_rates(sel, rules, table_min_cents);
    if a.handle <= 0.0 {
        return None;
    }
    Some(a.drift / a.handle)
}

/// Expected wealth drift per roll, in cents (negative), at flat stakes with
/// an unbounded bankroll. The Horizon "House Line" is
/// budget + drift × horizon_rolls; the status baseline's $/h is
/// drift × rolls_per_hour.
pub fn flat_drift_per_roll_cents(sel: &BetSelection, rules: &Rules, table_min_cents: i64) -> f64 {
    flat_rates(sel, rules, table_min_cents).drift
}

/// Resolved handle per roll, in cents, at flat stakes (the blended_edge
/// denominator, exposed for the status baseline).
pub fn flat_handle_per_roll_cents(sel: &BetSelection, rules: &Rules, table_min_cents: i64) -> f64 {
    flat_rates(sel, rules, table_min_cents).handle
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bets::{OddsPolicy, Progression};
    use crate::game::Session;
    use crate::rng::Xoshiro256pp;

    const START: i64 = 1_000_000_000_000;

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

    /// Unconstrained grind: net result (live bets settled at face value) and
    /// the engine's push-exclusive resolved-handle accumulator.
    fn grind(sel: &BetSelection, r: &Rules, min: i64, rolls: u64, seed: u64) -> (i64, i64) {
        let mut rng = Xoshiro256pp::seed_from_u64(seed);
        let mut s = Session::new(sel, r, min, START, false);
        for _ in 0..rolls {
            if s.needs_placement || s.one_roll_selected {
                s.place_bets();
            }
            let (d1, d2) = rng.dice();
            s.resolve(d1, d2);
        }
        (s.cash + s.on_table_face() - START, s.resolved_wagered_cents)
    }

    /// Simulated -net/resolved must sit on the closed form. The denominator
    /// is `resolved_wagered_cents` — the push-exclusive convention under
    /// test. Tolerances are documented ≥4σ bounds at the given roll counts.
    fn assert_family_edge(
        sel: &BetSelection,
        r: &Rules,
        rolls: u64,
        seed: u64,
        bet: EdgeBet,
        tol: f64,
    ) {
        let (net, resolved) = grind(sel, r, 1000, rolls, seed);
        let hat = net as f64 / resolved as f64;
        let want = bet_edge(bet, r).as_f64();
        assert!(
            (hat - want).abs() < tol,
            "{bet:?}: simulated {hat:.5} vs closed form {want:.5}"
        );
    }

    #[test]
    fn bet_edges_are_exact() {
        let r2 = rules();
        let r3 = Rules {
            field_12_triple: true,
            ..rules()
        };
        for r in [&r2, &r3] {
            assert_eq!(bet_edge(EdgeBet::Pass, r), Frac { num: -7, den: 495 });
            assert_eq!(bet_edge(EdgeBet::Come, r), Frac { num: -7, den: 495 });
            assert_eq!(
                bet_edge(EdgeBet::DontPass, r),
                Frac {
                    num: -27,
                    den: 1925
                }
            );
            assert_eq!(
                bet_edge(EdgeBet::DontCome, r),
                Frac {
                    num: -27,
                    den: 1925
                }
            );
            for n in [6u8, 8] {
                assert_eq!(bet_edge(EdgeBet::Place(n), r), Frac { num: -1, den: 66 });
                assert_eq!(bet_edge(EdgeBet::Hardway(n), r), Frac { num: -1, den: 11 });
            }
            for n in [5u8, 9] {
                assert_eq!(bet_edge(EdgeBet::Place(n), r), Frac { num: -1, den: 25 });
            }
            for n in [4u8, 10] {
                assert_eq!(bet_edge(EdgeBet::Place(n), r), Frac { num: -1, den: 15 });
                assert_eq!(bet_edge(EdgeBet::Hardway(n), r), Frac { num: -1, den: 9 });
            }
            assert_eq!(bet_edge(EdgeBet::AnySeven, r), Frac { num: -1, den: 6 });
            assert_eq!(bet_edge(EdgeBet::AnyCraps, r), Frac { num: -1, den: 9 });
            assert_eq!(bet_edge(EdgeBet::PassOdds, r), Frac { num: 0, den: 1 });
            assert_eq!(bet_edge(EdgeBet::DontPassLay, r), Frac { num: 0, den: 1 });
        }
        assert_eq!(bet_edge(EdgeBet::Field, &r2), Frac { num: -1, den: 18 });
        assert_eq!(bet_edge(EdgeBet::Field, &r3), Frac { num: -1, den: 36 });
    }

    #[test]
    fn line_bet_edges_match_simulation() {
        let r = rules();
        assert_family_edge(
            &only(|s| s.pass_line = true),
            &r,
            4_000_000,
            11,
            EdgeBet::Pass,
            0.004,
        );
        assert_family_edge(
            &only(|s| s.dont_pass = true),
            &r,
            4_000_000,
            12,
            EdgeBet::DontPass,
            0.004,
        );
    }

    #[test]
    fn field_edges_match_simulation() {
        let sel = only(|s| s.field = true);
        assert_family_edge(&sel, &rules(), 2_000_000, 13, EdgeBet::Field, 0.005);
        let r3 = Rules {
            field_12_triple: true,
            ..rules()
        };
        assert_family_edge(&sel, &r3, 2_000_000, 14, EdgeBet::Field, 0.005);
    }

    #[test]
    fn place_edges_match_simulation() {
        let r = rules();
        for (num, seed, tol) in [(6u8, 15u64, 0.005), (5, 16, 0.006), (4, 17, 0.008)] {
            let sel = only(|s| s.set_place(num, true));
            assert_family_edge(&sel, &r, 4_000_000, seed, EdgeBet::Place(num), tol);
        }
    }

    #[test]
    fn prop_edges_match_simulation() {
        let r = rules();
        for (num, seed) in [(6u8, 18u64), (4, 19)] {
            let sel = only(|s| s.hardways[crate::bets::hard_index(num).unwrap()] = true);
            assert_family_edge(&sel, &r, 4_000_000, seed, EdgeBet::Hardway(num), 0.015);
        }
        assert_family_edge(
            &only(|s| s.any_seven = true),
            &r,
            2_000_000,
            20,
            EdgeBet::AnySeven,
            0.006,
        );
        assert_family_edge(
            &only(|s| s.any_craps = true),
            &r,
            2_000_000,
            21,
            EdgeBet::AnyCraps,
            0.008,
        );
    }

    /// The E3 invariant Gasp 4 displays: mean loss ≈ edge × resolved handle,
    /// for every progression — pressing changes the handle, never the rate.
    /// 6M rolls per progression; 0.008 covers ≥4σ even for the wager-weighted
    /// Martingale-family estimators at table_max_mult 50.
    fn assert_progressions_hold_edge(base: &BetSelection, bet: EdgeBet, seed0: u64) {
        let r = Rules {
            table_max_mult: 50,
            ..rules()
        };
        let want = bet_edge(bet, &r).as_f64();
        for (pi, &prog) in Progression::ALL.iter().enumerate() {
            let mut sel = base.clone();
            sel.progression = prog;
            let (mut net, mut resolved) = (0i64, 0i64);
            for seed in 0..4u64 {
                let (n, w) = grind(&sel, &r, 1000, 1_500_000, seed0 + 100 * pi as u64 + seed);
                net += n;
                resolved += w;
            }
            let hat = net as f64 / resolved as f64;
            assert!(
                (hat - want).abs() < 0.008,
                "{prog:?}: loss per resolved dollar {hat:.5} vs edge {want:.5}"
            );
        }
    }

    #[test]
    fn pass_loss_equals_edge_times_handle_for_all_progressions() {
        assert_progressions_hold_edge(&only(|s| s.pass_line = true), EdgeBet::Pass, 40_000);
    }

    #[test]
    fn place_loss_equals_edge_times_handle_for_all_progressions() {
        // Place 6 and 8 share the −1/66 rate, so every resolved dollar of
        // this selection carries the same edge.
        let base = only(|s| {
            s.set_place(6, true);
            s.set_place(8, true);
        });
        assert_progressions_hold_edge(&base, EdgeBet::Place(6), 50_000);
    }

    /// Simulated mean wealth drift per roll vs the closed form, unconstrained
    /// at a $10 minimum. The tolerance is 4× the standard error estimated
    /// from 24 block means (plus a 0.05¢ numerical floor).
    fn assert_drift(sel: &BetSelection, r: &Rules, seed: u64) {
        const BLOCKS: usize = 24;
        const BLOCK_ROLLS: u64 = 250_000;
        let min = 1000;
        let mut rng = Xoshiro256pp::seed_from_u64(seed);
        let mut s = Session::new(sel, r, min, START, false);
        let mut prev = START as f64;
        let mut drifts = [0.0f64; BLOCKS];
        for d in drifts.iter_mut() {
            for _ in 0..BLOCK_ROLLS {
                if s.needs_placement || s.one_roll_selected {
                    s.place_bets();
                }
                let (d1, d2) = rng.dice();
                s.resolve(d1, d2);
            }
            let w = (s.cash + s.on_table_face()) as f64;
            *d = (w - prev) / BLOCK_ROLLS as f64;
            prev = w;
        }
        let mean = drifts.iter().sum::<f64>() / BLOCKS as f64;
        let var = drifts.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / (BLOCKS - 1) as f64;
        let se = (var / BLOCKS as f64).sqrt();
        let want = flat_drift_per_roll_cents(sel, r, min);
        assert!(want < 0.0, "drift must be negative, was {want}");
        assert!(
            (mean - want).abs() <= 4.0 * se + 0.05,
            "simulated drift {mean:.4}¢/roll vs closed form {want:.4}¢/roll (4σ = {:.4})",
            4.0 * se
        );
    }

    #[test]
    fn drift_matches_simulation_pass_only() {
        assert_drift(&only(|s| s.pass_line = true), &rules(), 61);
    }

    #[test]
    fn drift_matches_simulation_pass_with_345_odds() {
        let sel = only(|s| {
            s.pass_line = true;
            s.take_odds = true;
        });
        let r = Rules {
            odds_policy: OddsPolicy::X345,
            ..rules()
        };
        assert_drift(&sel, &r, 62);
    }

    #[test]
    fn drift_matches_simulation_place_six_eight() {
        let sel = only(|s| {
            s.set_place(6, true);
            s.set_place(8, true);
        });
        assert_drift(&sel, &rules(), 63);
    }

    #[test]
    fn drift_matches_simulation_iron_cross() {
        let sel = only(|s| {
            s.field = true;
            s.set_place(5, true);
            s.set_place(6, true);
            s.set_place(8, true);
        });
        assert_drift(&sel, &rules(), 64);
    }

    #[test]
    fn drift_matches_simulation_three_point_molly() {
        let sel = only(|s| {
            s.pass_line = true;
            s.come_max = 2;
            s.take_odds = true;
        });
        let r = Rules {
            odds_policy: OddsPolicy::X345,
            ..rules()
        };
        assert_drift(&sel, &r, 65);
    }

    #[test]
    fn drift_matches_simulation_dont_side_queue() {
        let sel = only(|s| {
            s.dont_pass = true;
            s.dont_come_max = 2;
            s.take_odds = true;
        });
        let r = Rules {
            odds_policy: OddsPolicy::X345,
            ..rules()
        };
        assert_drift(&sel, &r, 66);
    }

    /// The chains and cycle rates predict the resolved handle itself, not
    /// just the drift: simulated resolved_wagered_cents per roll must match
    /// flat_handle_per_roll_cents to 1% (observed deviation ≪ 0.5%).
    fn assert_handle_rate(sel: &BetSelection, r: &Rules, seed: u64) {
        let rolls = 4_000_000u64;
        let (_, resolved) = grind(sel, r, 1000, rolls, seed);
        let sim = resolved as f64 / rolls as f64;
        let want = flat_handle_per_roll_cents(sel, r, 1000);
        assert!(
            ((sim - want) / want).abs() < 0.01,
            "handle {sim:.3}¢/roll vs closed form {want:.3}¢/roll"
        );
    }

    #[test]
    fn handle_rates_match_simulation() {
        let x345 = Rules {
            odds_policy: OddsPolicy::X345,
            ..rules()
        };
        assert_handle_rate(
            &only(|s| {
                s.pass_line = true;
                s.take_odds = true;
            }),
            &x345,
            71,
        );
        assert_handle_rate(
            &only(|s| {
                s.set_place(6, true);
                s.set_place(8, true);
            }),
            &rules(),
            72,
        );
        assert_handle_rate(
            &only(|s| {
                s.hardways[1] = true; // hard 6
                s.hardways[0] = true; // hard 4
            }),
            &rules(),
            73,
        );
        assert_handle_rate(
            &only(|s| {
                s.come_max = 2;
                s.take_odds = true;
            }),
            &x345,
            74,
        );
        assert_handle_rate(
            &only(|s| {
                s.dont_come_max = 2;
                s.take_odds = true;
            }),
            &x345,
            75,
        );
        assert_handle_rate(
            &only(|s| {
                s.pass_line = true;
                s.come_max = 2;
                s.take_odds = true;
            }),
            &x345,
            76,
        );
    }

    #[test]
    fn blended_edge_is_drift_over_handle() {
        let x345 = Rules {
            odds_policy: OddsPolicy::X345,
            ..rules()
        };
        let selections: Vec<(BetSelection, Rules)> = vec![
            (only(|s| s.pass_line = true), rules()),
            (
                only(|s| {
                    s.pass_line = true;
                    s.come_max = 2;
                    s.take_odds = true;
                }),
                x345.clone(),
            ),
            (
                only(|s| {
                    s.field = true;
                    s.set_place(5, true);
                    s.set_place(6, true);
                    s.set_place(8, true);
                }),
                rules(),
            ),
            (
                only(|s| {
                    s.dont_pass = true;
                    s.dont_come_max = 3;
                    s.take_odds = true;
                    s.any_craps = true;
                }),
                x345,
            ),
        ];
        for (sel, r) in &selections {
            let b = blended_edge(sel, r, 1500).expect("selection is non-empty");
            let ratio =
                flat_drift_per_roll_cents(sel, r, 1500) / flat_handle_per_roll_cents(sel, r, 1500);
            assert!(
                (b - ratio).abs() < 1e-12,
                "blended {b} vs drift/handle {ratio}"
            );
            assert!(b < 0.0);
        }
    }

    #[test]
    fn blended_edge_of_single_families_is_the_bet_edge() {
        let r2 = rules();
        let r3 = Rules {
            field_12_triple: true,
            ..rules()
        };
        let cases: Vec<(BetSelection, &Rules, EdgeBet)> = vec![
            (only(|s| s.pass_line = true), &r2, EdgeBet::Pass),
            (only(|s| s.dont_pass = true), &r2, EdgeBet::DontPass),
            (only(|s| s.field = true), &r2, EdgeBet::Field),
            (only(|s| s.field = true), &r3, EdgeBet::Field),
            (only(|s| s.set_place(4, true)), &r2, EdgeBet::Place(4)),
            (only(|s| s.set_place(6, true)), &r2, EdgeBet::Place(6)),
            (only(|s| s.hardways[0] = true), &r2, EdgeBet::Hardway(4)),
            (only(|s| s.any_seven = true), &r2, EdgeBet::AnySeven),
            (only(|s| s.any_craps = true), &r2, EdgeBet::AnyCraps),
            // Come/don't-come without odds: every resolved dollar is a flat,
            // so the queue rate cancels out of the blend exactly.
            (only(|s| s.come_max = 3), &r2, EdgeBet::Come),
            (only(|s| s.dont_come_max = 3), &r2, EdgeBet::DontCome),
        ];
        for (sel, r, bet) in &cases {
            let b = blended_edge(sel, r, 1000).expect("selection is non-empty");
            let want = bet_edge(*bet, r).as_f64();
            assert!((b - want).abs() < 1e-12, "{bet:?}: blended {b} vs {want}");
        }
        assert_eq!(blended_edge(&only(|_| {}), &r2, 1000), None);
    }
}
