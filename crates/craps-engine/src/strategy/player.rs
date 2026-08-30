// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Who the session consults, and when.
//!
//! The session loop knows how to deal dice, resolve bets, and decide when a
//! session is over. It does not know how anyone plays. [`Player`] is the
//! seam: the built-in checkbox player and a compiled program are two
//! implementations of the same three questions — *should I be asked now*,
//! *what do you want*, and *what is the cheapest bet you might make* (which
//! is what "the bankroll can no longer sustain this" means).
//!
//! The trait is used statically, never behind a pointer, so both players
//! monomorphize to exactly the loop they need.

use crate::bets::{cheapest_selected_stake, BetSelection, Progression};
use crate::game::Session;
use crate::strategy::program::{decision_from, fired, Program};
use crate::strategy::view::{AllFeatures, Features, NoFeatures, STREAMS};
use crate::trace::RollObserver;

pub(crate) trait Player {
    /// The compile-time bound on derived history this player needs.
    type Feat: Features;

    /// Whether the session should consult this player before the next roll.
    fn wants_decision<O: RollObserver>(&self, s: &Session<'_, O, Self::Feat>) -> bool;

    /// Decide. Everything this puts on the layout goes through the table.
    fn decide<O: RollObserver>(&self, s: &mut Session<'_, O, Self::Feat>);

    /// The cheapest stake this player might put up — the amount below which
    /// a bankroll with nothing working is ruined.
    fn cheapest_stake<O: RollObserver>(&self, s: &Session<'_, O, Self::Feat>) -> i64;

    /// How each bet stream presses. The checkbox player answers with its one
    /// choice seventeen times; a compiled strategy may answer differently
    /// per stream.
    fn progressions(&self, sel: &BetSelection) -> [Progression; STREAMS] {
        [sel.progression; STREAMS]
    }

    /// Put the session's strategy memory where this player wants to start.
    /// The checkbox player has no memory and wants nothing.
    fn init_state(&self, _st: &mut crate::strategy::StratState) {}
}

/// The checkbox player: a [`BetSelection`](crate::BetSelection) and one
/// progression, exactly as it has always been.
pub(crate) struct Builtin;

impl Player for Builtin {
    type Feat = NoFeatures;

    #[inline]
    fn wants_decision<O: RollObserver>(&self, s: &Session<'_, O, NoFeatures>) -> bool {
        // Placement is a pure function of cash, bets, and the point; until a
        // resolution changes one of those it would be a no-op. One-roll bets
        // are the exception — they resolve every roll and need re-placing.
        s.needs_placement || s.one_roll_selected
    }

    #[inline]
    fn decide<O: RollObserver>(&self, s: &mut Session<'_, O, NoFeatures>) {
        s.place_bets();
    }

    #[inline]
    fn cheapest_stake<O: RollObserver>(&self, s: &Session<'_, O, NoFeatures>) -> i64 {
        cheapest_selected_stake(s.sel, s.rules, s.min)
    }
}

/// A compiled strategy.
pub(crate) struct Compiled<'p> {
    pub program: &'p Program,
    /// The cheapest stake any bet in the program could take, computed once
    /// at construction from the program's own bets rather than from a
    /// checkbox set it does not have.
    pub cheapest: i64,
}

impl<'p> Compiled<'p> {
    pub fn new(program: &'p Program, cheapest: i64) -> Self {
        Self { program, cheapest }
    }
}

impl Player for Compiled<'_> {
    type Feat = AllFeatures;

    #[inline]
    fn wants_decision<O: RollObserver>(&self, s: &Session<'_, O, AllFeatures>) -> bool {
        // A rule may want to act on any roll, so by default there is no
        // skipping the decision. A program the compiler proved depends only
        // on the point, the layout, and the bankroll is the exception: until
        // a resolution moves one of those, it would decide the same thing.
        if self.program.placement_only {
            s.needs_placement || self.program.bets_one_roll
        } else {
            true
        }
    }

    fn init_state(&self, st: &mut crate::strategy::StratState) {
        st.seed(&self.program.var_init);
    }

    fn decide<O: RollObserver>(&self, s: &mut Session<'_, O, AllFeatures>) {
        let d = decision_from(
            s.hist.fired
                | if s.point.is_none() {
                    fired::COME_OUT
                } else {
                    0
                }
                // The first decision of a session happens before any roll,
                // and is the one moment a player is standing at the table
                // with nothing having happened yet. `on session-start` was
                // spelled, compiled, and offered in the editor without this
                // line, so every rule written on it fired never.
                | if s.hist.roll == 0 {
                    fired::SESSION_START
                } else {
                    0
                }
                | fired::ROLL,
            s.hist.last_total_now,
            s.hist.won,
            s.hist.lost,
            s.hist.come_established,
            s.hist.dont_come_established,
        );
        s.run_program(self.program, d);
    }

    #[inline]
    fn cheapest_stake<O: RollObserver>(&self, _s: &Session<'_, O, AllFeatures>) -> i64 {
        self.cheapest
    }

    fn progressions(&self, _sel: &BetSelection) -> [Progression; STREAMS] {
        self.program.progressions
    }
}
