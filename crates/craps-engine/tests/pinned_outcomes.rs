// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Pinned outcome checksums (plan §3): every [`SessionOutcomes`] field for
//! three reference configurations, folded across seeds 0..10_000 into one
//! u64 per configuration through the engine's own SplitMix64 mixer.
//!
//! These constants freeze engine behavior. If one of these assertions
//! fails, the engine's simulated outcomes changed for at least one of
//! 10,000 sessions — either a bug was introduced or payout/bet semantics
//! were deliberately changed. Any INTENTIONAL change must consciously
//! update the pinned values (and note the behavior change in the
//! changelog); never update them to silence a failure you cannot explain.

use craps_engine::{
    quit_target_cents, run_session, splitmix64, BetSelection, OddsPolicy, Progression, Rules,
    SessionOutcomes,
};

const TABLE_MIN_CENTS: i64 = 1000; // $10
const BUDGET_CENTS: i64 = 30_000; // $300
const MAX_ROLLS: u64 = 2000;
const HORIZON_ROLLS: u64 = 400;
const SEEDS: u64 = 10_000;

fn fold(h: u64, v: u64) -> u64 {
    splitmix64(h ^ v)
}

/// Chain every field of one session's outcomes into the running checksum,
/// in a fixed documented order.
fn fold_outcomes(mut h: u64, o: &SessionOutcomes) -> u64 {
    h = fold(h, o.ruin.rolls);
    h = fold(h, o.ruin.censored as u64);
    h = fold(h, o.ruin.hit_target as u64);
    h = fold(h, o.horizon.final_cents as u64);
    h = fold(h, o.horizon.busted as u64);
    h = fold(h, o.horizon.hit_target as u64);
    h = fold(h, o.horizon.rolls);
    h = fold(h, o.peak_outlay_cents as u64);
    h = fold(h, o.horizon_handle_cents as u64);
    h
}

fn checksum(sel: &BetSelection, rules: &Rules, quit_target_cents: Option<i64>) -> u64 {
    let mut h = 0u64;
    for seed in 0..SEEDS {
        let o = run_session(
            sel,
            rules,
            TABLE_MIN_CENTS,
            BUDGET_CENTS,
            quit_target_cents,
            MAX_ROLLS,
            HORIZON_ROLLS,
            seed,
        );
        h = fold_outcomes(h, &o);
    }
    h
}

/// Standard rules, matching the app's defaults, at the given odds policy.
fn rules(odds_policy: OddsPolicy) -> Rules {
    Rules {
        odds_policy,
        field_12_triple: false,
        come_odds_work_on_comeout: false,
        prop_bet_cents: 500,
        table_max_mult: 500,
    }
}

#[test]
fn pass_line_flat_outcomes_are_pinned() {
    let sel = BetSelection::default(); // pass line only, flat betting
    assert_eq!(
        checksum(&sel, &rules(OddsPolicy::None), None),
        0x0FE1_7365_734E_6D4C,
        "pass-line flat outcome checksum drifted — engine behavior changed"
    );
}

#[test]
fn three_point_molly_outcomes_are_pinned() {
    // Pass + two come bets, 3-4-5x odds, quit at 2x the budget.
    let sel = BetSelection {
        come_max: 2,
        take_odds: true,
        ..Default::default()
    };
    let quit = Some(quit_target_cents(BUDGET_CENTS, 2.0));
    assert_eq!(
        checksum(&sel, &rules(OddsPolicy::X345), quit),
        0x6B1F_C73D_C556_0F4D,
        "3-pt Molly outcome checksum drifted — engine behavior changed"
    );
}

#[test]
fn iron_cross_martingale_outcomes_are_pinned() {
    // Field + place 5, 6, 8, pressed with a Martingale.
    let mut sel = BetSelection {
        pass_line: false,
        field: true,
        progression: Progression::Martingale,
        ..Default::default()
    };
    for n in [5, 6, 8] {
        sel.set_place(n, true);
    }
    assert_eq!(
        checksum(&sel, &rules(OddsPolicy::None), None),
        0x2F2F_3CEA_A375_9BFB,
        "iron-cross Martingale outcome checksum drifted — engine behavior changed"
    );
}
