// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Session runners: play out whole sessions and report their outcomes.

use crate::bets::{BetSelection, Rules};
use crate::game::Session;
use crate::rng::Xoshiro256pp;
use crate::strategy::NoFeatures;
use crate::trace::{Noop, RollObserver};

/// Which simulation phase a session belongs to. Part of the seed so the two
/// phases of the fused sweep (and the explorer) draw independent dice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum SeedPhase {
    /// The budgeted ruin + horizon pass.
    Main = 1,
    /// The unconstrained peak-outlay pass.
    Drawdown = 2,
    /// The strategy explorer (and the Duel, which replays explorer dice).
    Explore = 3,
}

/// THE determinism contract: the seed for one simulated session.
///
/// Every consumer — the sweep, the explorer, the Duel, session replay, the
/// dot field, the wealth fan — derives its per-session seed through this
/// function and nothing else. Common random numbers fall out of it: two
/// configurations simulated with the same `(base, min_index, phase, session)`
/// play identical dice, because the RNG stream feeds dice only.
#[inline]
pub fn session_seed(base: u64, min_index: u32, phase: SeedPhase, session: u64) -> u64 {
    base ^ ((min_index as u64 + 1) << 48) ^ ((phase as u64) << 40) ^ session
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionOutcomes {
    /// The full play-until-bust/quit/cap trajectory.
    pub ruin: RuinOutcome,
    /// The fixed-horizon snapshot: exactly what the session looked like as it
    /// crossed `horizon_rolls` (or how it had already ended before that).
    pub horizon: HorizonOutcome,
    /// Peak cash outlay of this budgeted session over its full trajectory:
    /// starting cash minus the lowest cash seen. (The Stake question uses the
    /// unconstrained [`run_drawdown_session`] statistic instead; this one
    /// feeds session replay's outlay shading.)
    pub peak_outlay_cents: i64,
    /// Resolved handle (stakes settled by a win or loss; pushes excluded) at
    /// the moment the horizon view ended — the denominator behind the
    /// "loss ≈ edge × handle" readout.
    pub horizon_handle_cents: i64,
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
    run_session_impl(
        sel,
        rules,
        table_min_cents,
        budget_cents,
        quit_target_cents,
        max_rolls,
        horizon_rolls,
        seed,
        Noop,
    )
    .0
}

/// The one session loop, generic over the observer. [`Noop`] monomorphizes
/// this to exactly the untraced code (the equivalence suite pins behavior);
/// tracing observers receive every bet event plus a `roll_end` per roll.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_session_impl<O: RollObserver>(
    sel: &BetSelection,
    rules: &Rules,
    table_min_cents: i64,
    budget_cents: i64,
    quit_target_cents: Option<i64>,
    max_rolls: u64,
    horizon_rolls: u64,
    seed: u64,
    obs: O,
) -> (SessionOutcomes, O) {
    let mut rng = Xoshiro256pp::seed_from_u64(seed);
    // The built-in player reads no derived history, so the session type
    // compiles every accumulator out. A runner generic over the strategy
    // arrives with the compiled programs.
    let mut s: Session<'_, O, NoFeatures> =
        Session::with_observer(sel, rules, table_min_cents, budget_cents, false, obs);
    let cheapest = s.cheapest_selected_stake();
    let mut rolls = 0u64;
    let mut ruin: Option<RuinOutcome> = None;
    let mut horizon: Option<HorizonOutcome> = None;
    // Resolved handle captured the moment the horizon view froze; when the
    // session ends both views at once, the current handle is the capture.
    let mut horizon_handle: Option<i64> = None;
    loop {
        // The horizon can be due before any roll (a zero-roll horizon reads
        // the starting wealth), so the freeze check leads the loop.
        if horizon.is_none() && rolls >= horizon_rolls {
            horizon = Some(HorizonOutcome {
                final_cents: s.cash + s.on_table_face(),
                busted: false,
                hit_target: false,
                rolls,
            });
            horizon_handle = Some(s.resolved_wagered_cents);
        }
        if let (Some(r), Some(h)) = (ruin, horizon) {
            let out = SessionOutcomes {
                ruin: r,
                horizon: h,
                peak_outlay_cents: budget_cents - s.min_cash,
                horizon_handle_cents: horizon_handle.unwrap_or(s.resolved_wagered_cents),
            };
            return (out, s.into_observer());
        }
        if s.needs_placement || s.one_roll_selected {
            s.place_bets();
        }
        if !s.has_multi_roll_bets() && !s.has_one_roll_bets() && s.cash < cheapest {
            let out = SessionOutcomes {
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
                peak_outlay_cents: budget_cents - s.min_cash,
                horizon_handle_cents: horizon_handle.unwrap_or(s.resolved_wagered_cents),
            };
            return (out, s.into_observer());
        }
        let (d1, d2) = rng.dice();
        rolls += 1;
        s.resolve(d1, d2);
        if O::WANTS_ROLL_END {
            let wealth = s.cash + s.on_table_face();
            let (point, cash) = (s.point, s.cash);
            s.obs.roll_end(rolls, (d1, d2), point, cash, wealth);
        }
        if let Some(target) = quit_target_cents {
            let wealth = s.cash + s.on_table_face();
            if wealth >= target {
                let out = SessionOutcomes {
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
                    peak_outlay_cents: budget_cents - s.min_cash,
                    horizon_handle_cents: horizon_handle.unwrap_or(s.resolved_wagered_cents),
                };
                return (out, s.into_observer());
            }
        }
        if ruin.is_none() && rolls >= max_rolls {
            ruin = Some(RuinOutcome {
                rolls,
                censored: true,
                hit_target: false,
            });
        }
        if let (Some(r), Some(h)) = (ruin, horizon) {
            let out = SessionOutcomes {
                ruin: r,
                horizon: h,
                peak_outlay_cents: budget_cents - s.min_cash,
                horizon_handle_cents: horizon_handle.unwrap_or(s.resolved_wagered_cents),
            };
            return (out, s.into_observer());
        }
    }
}

/// Play until the bankroll can no longer sustain the strategy, until the
/// take-profit target (cash + face value of live bets) is reached, or until
/// `max_rolls`.
///
/// The app's worker uses [`run_session`] to get this and the horizon view in
/// one pass; this single-view form is kept as API surface and test harness.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bets::{OddsPolicy, Progression};

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
}

#[cfg(test)]
mod bench {
    use super::*;
    use crate::bets::{OddsPolicy, Progression};
    use crate::summary::summarize_ruin;
    use rayon::prelude::*;

    /// Configurations the strategy-surface gates are measured against
    /// ([`STRATEGY_DSL.md`](../../../docs/STRATEGY_DSL.md) Part II §3): the
    /// cheapest player, a full multi-bet player, and a pressed one. A change
    /// that is free on the pass line but not on a loaded table has to show up
    /// somewhere, so it shows up here.
    pub(crate) fn bench_configs() -> Vec<(&'static str, BetSelection, Rules, u64)> {
        let rules = |odds| Rules {
            odds_policy: odds,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 1000,
        };
        let mut molly = BetSelection {
            pass_line: true,
            take_odds: true,
            ..Default::default()
        };
        molly.come_max = 2;
        let mut loaded = BetSelection {
            pass_line: true,
            take_odds: true,
            field: true,
            hardways: [true; 4],
            ..Default::default()
        };
        loaded.come_max = 2;
        loaded.dont_come_max = 1;
        for n in [4, 5, 6, 8, 9, 10] {
            loaded.set_place(n, true);
        }
        let mut pressed = loaded.clone();
        pressed.progression = Progression::FullPress;
        // Session counts differ because a loaded table busts a $300
        // bankroll in a few dozen rolls; these equalize simulated rolls,
        // not sessions, so every row times a comparable amount of work.
        vec![
            (
                "pass line",
                BetSelection::default(),
                rules(OddsPolicy::None),
                2_500,
            ),
            ("3-pt molly", molly, rules(OddsPolicy::X345), 6_000),
            ("loaded table", loaded, rules(OddsPolicy::X345), 65_000),
            (
                "loaded + full press",
                pressed,
                rules(OddsPolicy::X345),
                190_000,
            ),
        ]
    }

    /// Fixed work on one thread, best of `REPS` — the only throughput
    /// number stable enough to gate on. [`throughput`] measures the machine
    /// as much as the engine (94–193 M rolls/s across four runs on one
    /// commit); best-of-N single-threaded holds within a few percent,
    /// because scheduling noise only ever adds time. Session counts are
    /// tuned so every config simulates ≥15 M rolls, and the `spread` column
    /// reports max/min across reps — anything above ~1.15 means the machine
    /// was busy and the run should not be used as a gate.
    ///
    /// Record a baseline before an engine change, run it again after,
    /// compare the `rolls/s` column.
    ///
    ///   cargo test --release -p craps-engine -- --ignored bench_single_thread --nocapture
    #[test]
    #[ignore]
    fn bench_single_thread() {
        const REPS: u32 = 7;
        println!(
            "\n{:<22} {:>12} {:>11} {:>11} {:>8}",
            "config", "rolls", "best", "rolls/s", "spread"
        );
        for (name, sel, rules, sessions) in bench_configs() {
            let (mut best, mut worst) = (f64::INFINITY, 0.0f64);
            let mut rolls = 0u64;
            for _ in 0..REPS {
                let start = std::time::Instant::now();
                let mut total = 0u64;
                let mut sink = 0i64;
                for i in 0..sessions {
                    let o = run_session(&sel, &rules, 1000, 30_000, None, 200_000, 400, i);
                    total += o.ruin.rolls;
                    sink += o.horizon.final_cents;
                }
                let dt = start.elapsed().as_secs_f64();
                std::hint::black_box(sink);
                best = best.min(dt);
                worst = worst.max(dt);
                rolls = total;
            }
            println!(
                "{name:<22} {rolls:>12} {best:>10.4}s {:>10.1}M {:>8.2}",
                rolls as f64 / best / 1e6,
                worst / best
            );
        }
    }

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

#[cfg(test)]
mod seed_contract_tests {
    use super::*;

    /// The seed formula is a public determinism contract: these exact values
    /// are pinned so any change to the formula fails loudly. Replays, the
    /// Duel, and every stored provenance depend on it never drifting.
    #[test]
    fn session_seed_is_pinned() {
        assert_eq!(
            session_seed(0, 0, SeedPhase::Main, 0),
            0x0001_0100_0000_0000
        );
        assert_eq!(
            session_seed(0, 0, SeedPhase::Drawdown, 0),
            0x0001_0200_0000_0000
        );
        assert_eq!(
            session_seed(0xFF, 2, SeedPhase::Explore, 5),
            0x0003_0300_0000_00FA
        );
        // XOR structure: session index perturbs only the low bits.
        assert_eq!(
            session_seed(0, 7, SeedPhase::Main, 1234) ^ session_seed(0, 7, SeedPhase::Main, 0),
            1234
        );
    }

    #[test]
    fn budgeted_peak_outlay_matches_drawdown_at_the_boundary() {
        // With budget exactly equal to the unconstrained max outlay, the
        // budgeted session can fund the identical placement sequence, so its
        // recorded peak outlay must equal the drawdown statistic exactly.
        let mut sel = BetSelection {
            pass_line: true,
            ..Default::default()
        };
        sel.come_max = 2;
        sel.take_odds = true;
        sel.field = true;
        let rules = Rules {
            odds_policy: crate::OddsPolicy::X345,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 1000,
        };
        let horizon = 400;
        for seed in 0..50u64 {
            let outlay = run_drawdown_session(&sel, &rules, 1000, horizon, seed);
            let o = run_session(&sel, &rules, 1000, outlay, None, horizon, horizon, seed);
            assert_eq!(
                o.peak_outlay_cents, outlay,
                "seed {seed}: budgeted peak != drawdown outlay"
            );
            assert!(
                o.horizon_handle_cents > 0,
                "seed {seed}: no handle resolved"
            );
        }
    }

    #[test]
    fn handle_snapshot_freezes_at_the_horizon() {
        // The handle reported for a 200-roll horizon must not exceed the one
        // for a 400-roll horizon on the same dice, and both must be positive.
        let sel = BetSelection::default(); // pass line
        let rules = Rules {
            odds_policy: crate::OddsPolicy::None,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 1000,
        };
        for seed in 0..50u64 {
            let short = run_session(&sel, &rules, 1000, 100_000, None, 2_000, 200, seed);
            let long = run_session(&sel, &rules, 1000, 100_000, None, 2_000, 400, seed);
            assert!(short.horizon_handle_cents > 0);
            assert!(
                short.horizon_handle_cents <= long.horizon_handle_cents,
                "seed {seed}: handle shrank with a longer horizon"
            );
        }
    }
}
