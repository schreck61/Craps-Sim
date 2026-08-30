// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The table itself: one [`Session`] holds every live bet and resolves dice
//! rolls against the selected strategy.

use crate::bets::{
    hard_index, hardway_win, pass_odds_win, place_index, place_stake, place_unit, place_win,
    BetSelection, OddsPolicy, ProgState, Progression, Rules, HARD_NUMS, PLACE_NUMS,
};
use crate::strategy::action::bet_kind;
use crate::strategy::view::{
    Features, History, NoFeatures, Stakes, STREAMS, S_ANY7, S_ANYCRAPS, S_COME, S_DC, S_DONT,
    S_FIELD, S_HARD, S_PASS, S_PLACE,
};
use crate::strategy::{Amount, BetRef, FeatureMask, TableView};
use crate::trace::{BetEvent, BetEventKind, BetKind, Noop, RollObserver};

pub(crate) struct Session<'a, O: RollObserver = Noop, F: Features = NoFeatures> {
    /// Observer for tracing; [`Noop`]'s empty inline hooks vanish entirely
    /// under monomorphization.
    pub(crate) obs: O,
    pub(crate) sel: &'a BetSelection,
    pub(crate) rules: &'a Rules,
    pub(crate) min: i64,
    /// Cash in hand. Never negative in ruin mode; may go negative in drawdown mode.
    pub(crate) cash: i64,
    pub(crate) allow_negative: bool,
    pub(crate) max_outlay: i64,
    /// Lowest cash seen so far. In budgeted sessions the peak cash outlay is
    /// `starting cash − min_cash`; in drawdown mode it mirrors `max_outlay`.
    pub(crate) min_cash: i64,
    /// Total stake resolved by wins and losses — the "resolved handle" the
    /// house edge applies to. Pushes are excluded by convention: bar-12 on
    /// the don't side and come odds returned while off never count. Every
    /// push site is enumerated in `edge.rs`, which documents the convention.
    pub(crate) resolved_wagered_cents: i64,
    pub(crate) point: Option<u8>,
    pub(crate) pass: i64,
    pub(crate) pass_odds: i64,
    pub(crate) pass_odds_point: u8,
    pub(crate) dont: i64,
    pub(crate) dont_lay: i64,
    pub(crate) dont_lay_win: i64,
    pub(crate) place: [i64; 6],
    pub(crate) hard: [i64; 4],
    // Come bets: flat in the come box (travels on the next roll), then flat
    // and odds per number, indexed like PLACE_NUMS.
    pub(crate) come_flat: i64,
    pub(crate) come_points: [i64; 6],
    pub(crate) come_odds: [i64; 6],
    // Don't come: flat in the DC box, then flat and lay odds per number.
    pub(crate) dc_flat: i64,
    pub(crate) dc_points: [i64; 6],
    pub(crate) dc_lay: [i64; 6],
    pub(crate) dc_lay_win: [i64; 6],
    // one-roll bets, placed pre-roll and resolved the same roll
    pub(crate) field_bet: i64,
    pub(crate) any7_bet: i64,
    pub(crate) anycraps_bet: i64,
    /// Set by any resolution that changes cash, bets, or the point — the
    /// only inputs `place_bets` reads. While clear, `place_bets` would be a
    /// no-op, so the pre-roll placement pass can be skipped entirely
    /// (except that one-roll bets always need re-placing).
    pub(crate) needs_placement: bool,
    /// Whether a place bet or hardway also resolves on a come-out roll.
    ///
    /// Separate from `place_working` rather than folded into it, because
    /// they answer different questions and have different defaults: a bet is
    /// working during a point cycle unless the player calls it off, and is
    /// *not* working on the come-out unless the player says so. Craps has
    /// always had both halves; the language could only say the first.
    ///
    /// Never set on the built-in path — nothing there emits the action that
    /// sets it — so the checkbox player resolves exactly as it always has.
    pub(crate) place_comeout: [bool; 6],
    pub(crate) hard_comeout: [bool; 4],
    pub(crate) one_roll_selected: bool,
    // Progression state, one stream per bet type. Come and don't come flats
    // share one stream each (the press applies to the next flat made).
    pub(crate) table_max: i64,
    pub(crate) p_pass: ProgState,
    pub(crate) p_dont: ProgState,
    pub(crate) p_come: ProgState,
    pub(crate) p_dc: ProgState,
    pub(crate) p_field: ProgState,
    pub(crate) p_place: [ProgState; 6],
    pub(crate) p_hard: [ProgState; 4],
    pub(crate) p_any7: ProgState,
    pub(crate) p_anycraps: ProgState,
    /// Whether each place bet and hardway is working. An off bet sits on
    /// the layout resolving nothing — still the player's money, still in
    /// what they would walk away with. Place bets and hardways are the only
    /// bets a table lets a player call off; the rest are contract bets, ride
    /// with what they back, or resolve before the question can be asked.
    ///
    /// The come-out convention is separate and unchanged: these bets are off
    /// on a come-out roll whatever this says.
    pub(crate) place_working: [bool; 6],
    pub(crate) hard_working: [bool; 4],
    /// Set when a strategy leaves the table on its own terms. The session
    /// loop reads it after the decision and ends there.
    pub(crate) leaving: bool,
    /// The pressing system on each bet stream, keyed like [`STREAMS`].
    ///
    /// One per stream rather than one for the table: a player who
    /// Martingales the don't pass while keeping the place bets flat is
    /// describing two systems at one table, and that is not sayable with a
    /// single enum. The checkbox player fills all seventeen with its one
    /// choice, which is why nothing about it changes.
    pub(crate) progressions: [Progression; STREAMS],
    /// What the player sat down with, so profit and drawdown have an
    /// origin. Read through [`Session::view`], which only the compiled
    /// strategies call — the built-in player reads the session directly.
    #[allow(
        dead_code,
        reason = "read by TableView; consumed by compiled strategies"
    )]
    pub(crate) start_cash: i64,
    /// Which groups of derived history this session maintains. Empty for
    /// the built-in player, which reads none of them and pays for none.
    pub(crate) features: FeatureMask,
    pub(crate) hist: History,
    /// A compiled strategy's memory. Fixed size and `Copy`, so a decision
    /// can take it out, run against an immutable view of the table, and put
    /// it back without fighting the borrow checker over the session.
    pub(crate) strat: crate::strategy::StratState,
    /// The decision's proposal buffer, reused rather than rebuilt.
    pub(crate) proposals: crate::strategy::program::Proposals,
    pub(crate) _features: core::marker::PhantomData<F>,
}

impl<'a> Session<'a, Noop, NoFeatures> {
    /// An untraced session with the built-in player's feature set. Only the
    /// tests build one directly now — the runners go through
    /// [`Session::with_observer`] so the player decides the feature bound.
    #[cfg(test)]
    pub(crate) fn new(
        sel: &'a BetSelection,
        rules: &'a Rules,
        min: i64,
        cash: i64,
        allow_negative: bool,
    ) -> Self {
        Session::with_observer(sel, rules, min, cash, allow_negative, Noop)
    }
}

impl<'a, O: RollObserver, F: Features> Session<'a, O, F> {
    pub(crate) fn with_observer(
        sel: &'a BetSelection,
        rules: &'a Rules,
        min: i64,
        cash: i64,
        allow_negative: bool,
        obs: O,
    ) -> Self {
        Self {
            obs,
            sel,
            rules,
            min,
            cash,
            allow_negative,
            max_outlay: 0,
            min_cash: cash,
            resolved_wagered_cents: 0,
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
            place_working: [true; 6],
            hard_working: [true; 4],
            place_comeout: [false; 6],
            hard_comeout: [false; 4],
            leaving: false,
            progressions: [sel.progression; STREAMS],
            start_cash: cash,
            features: F::MASK,
            hist: History::new(cash),
            strat: crate::strategy::StratState::default(),
            proposals: crate::strategy::program::Proposals::default(),
            _features: core::marker::PhantomData,
        }
    }

    pub(crate) fn into_observer(self) -> O {
        self.obs
    }

    #[inline(always)]
    pub(crate) fn emit(&mut self, bet: BetKind, kind: BetEventKind, stake_cents: i64) {
        if !F::MASK.is_empty() {
            if let BetEventKind::Traveled { to } = kind {
                self.hist.record_travel(bet, to);
            }
        }
        if F::MASK.has(FeatureMask::STREAKS) && self.features.has(FeatureMask::STREAKS) {
            match kind {
                BetEventKind::Won { paid_cents, .. } => self.hist.record_win(bet, paid_cents),
                BetEventKind::Lost => self.hist.record_loss(bet),
                _ => {}
            }
        }
        self.obs.event(BetEvent {
            bet,
            kind,
            stake_cents,
        });
    }

    /// The stake a progression stream calls for, clamped to [base, table max].
    /// Pressed stakes are rounded to whole dollars — a real table would not
    /// accept a $33.75 flat bet, and whole-dollar flats keep every true-odds
    /// payout exact in cents (progressions like Half Press otherwise produce
    /// sub-cent-precision stakes).
    ///
    /// The stake arrives by value rather than as a `&ProgState` borrow so
    /// the clip can be emitted from here — this is where the table maximum
    /// actually bites, and per STRATEGY_DSL.md Principle 4 the truncation
    /// that stops a Martingale is an event, not a silent flat spot in a
    /// curve. Stake shaping moves into the adjudicator with the compiled
    /// progressions (P2); until then it emits from where it happens.
    pub(crate) fn prog_stake(&mut self, stake: i64, base: i64, bet: BetRef) -> i64 {
        let max = self.table_max.max(base);
        if stake > max {
            self.emit(bet_kind(bet), BetEventKind::ClippedToMax, max);
        }
        let raw = stake.clamp(base, max);
        if raw == base {
            return base;
        }
        ((raw + 50) / 100 * 100).clamp(base, max)
    }

    /// Same, rounded to the nearest payout unit for place bets.
    pub(crate) fn prog_place_stake(&mut self, i: usize) -> i64 {
        let num = PLACE_NUMS[i];
        let unit = place_unit(num);
        let base = place_stake(self.min, num);
        let v = self.prog_stake(self.p_place[i].stake, base, BetRef::Place(num));
        ((v + unit / 2) / unit).max(1) * unit
    }

    /// Try the progression stake; if the bankroll can't cover it, fall back to
    /// the base bet (bet what you can rather than sitting out). The
    /// progression stream still advances from its own intended stake — the
    /// player is behind on the system, not restarting it.
    pub(crate) fn try_stake_or_base(&mut self, desired: i64, base: i64) -> Option<i64> {
        self.try_stake(desired).or_else(|| {
            if base < desired {
                self.try_stake(base)
            } else {
                None
            }
        })
    }

    #[inline]
    pub(crate) fn try_stake(&mut self, amount: i64) -> Option<i64> {
        if amount <= 0 {
            return None;
        }
        if self.allow_negative || self.cash >= amount {
            self.cash -= amount;
            if self.cash < self.min_cash {
                self.min_cash = self.cash;
            }
            if -self.cash > self.max_outlay {
                self.max_outlay = -self.cash;
            }
            Some(amount)
        } else {
            None
        }
    }

    pub(crate) fn has_multi_roll_bets(&self) -> bool {
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

    pub(crate) fn live_come_bets(&self) -> u8 {
        (self.come_flat > 0) as u8 + self.come_points.iter().filter(|&&b| b > 0).count() as u8
    }

    pub(crate) fn live_dc_bets(&self) -> u8 {
        (self.dc_flat > 0) as u8 + self.dc_points.iter().filter(|&&b| b > 0).count() as u8
    }

    pub(crate) fn has_one_roll_bets(&self) -> bool {
        self.field_bet > 0 || self.any7_bet > 0 || self.anycraps_bet > 0
    }

    /// Face value of everything still on the table (what the player walks
    /// away with, treating live bets as taken down at the end of a session).
    pub(crate) fn on_table_face(&self) -> i64 {
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

    /// Cheapest stake among selected recurring bets — what the built-in
    /// player answers when the session asks what ruin means for it. A
    /// compiled program answers the same question from its own bets.
    #[cfg(test)]
    pub(crate) fn cheapest_selected_stake(&self) -> i64 {
        crate::bets::cheapest_selected_stake(self.sel, self.rules, self.min)
    }

    /// The built-in player, expressed as intents.
    ///
    /// Every condition in this function is a *strategy* condition: which
    /// bets this player wants, how many come bets it will carry at once,
    /// which numbers it places, what its progression asks for. Whether an
    /// ask is legal right now, affordable, or clipped is the table's
    /// business, decided once in [`Session::apply`].
    ///
    /// This is the first strategy expressed against the intent surface, and
    /// it must decide exactly what the hand-written version decided, to the
    /// cent — the pinned outcome vectors and the equivalence battery are the
    /// proof. Nothing here allocates: intents are adjudicated in sequence,
    /// never collected.
    pub(crate) fn place_bets(&mut self) {
        // `multiple()` returns 0 for every point under `OddsPolicy::None`
        // and nonzero under every other policy, so this single test is
        // exactly the per-point `mult > 0` guard, hoisted. A player whose
        // policy allows no odds does not ask for them, rather than being
        // refused once per roll.
        let takes_odds = self.sel.take_odds && self.rules.odds_policy != OddsPolicy::None;

        match self.point {
            None => {
                // Come-out: line bets only. Place bets and hardways are "off".
                if self.sel.pass_line && self.pass == 0 {
                    let want = self.prog_stake(self.p_pass.stake, self.min, BetRef::Pass);
                    let _ = self.apply_bet(BetRef::Pass, Amount::Cents(want));
                }
                if self.sel.dont_pass && self.dont == 0 {
                    let want = self.prog_stake(self.p_dont.stake, self.min, BetRef::DontPass);
                    let _ = self.apply_bet(BetRef::DontPass, Amount::Cents(want));
                }
            }
            Some(point) => {
                // New come / don't come bets are only made while a point is on.
                if self.sel.come_max > 0
                    && self.come_flat == 0
                    && self.live_come_bets() < self.sel.come_max
                {
                    let want = self.prog_stake(self.p_come.stake, self.min, BetRef::Come);
                    let _ = self.apply_bet(BetRef::Come, Amount::Cents(want));
                }
                if self.sel.dont_come_max > 0
                    && self.dc_flat == 0
                    && self.live_dc_bets() < self.sel.dont_come_max
                {
                    let want = self.prog_stake(self.p_dc.stake, self.min, BetRef::DontCome);
                    let _ = self.apply_bet(BetRef::DontCome, Amount::Cents(want));
                }
                // Odds behind an established line bet.
                if takes_odds {
                    if self.pass > 0 && self.pass_odds == 0 {
                        let _ = self.apply_bet(BetRef::PassOdds, Amount::MaxOdds);
                    }
                    if self.dont > 0 && self.dont_lay == 0 {
                        let _ = self.apply_bet(BetRef::DontPassLay, Amount::MaxOdds);
                    }
                }
                // Place bets on selected numbers other than the current point.
                for (i, &num) in PLACE_NUMS.iter().enumerate() {
                    if self.sel.place[i] && num != point && self.place[i] == 0 {
                        let want = self.prog_place_stake(i);
                        let _ = self.apply_bet(BetRef::Place(num), Amount::Cents(want));
                    }
                }
                // Hardways.
                for (i, &num) in HARD_NUMS.iter().enumerate() {
                    if self.sel.hardways[i] && self.hard[i] == 0 {
                        let base = self.rules.prop_bet_cents;
                        let want =
                            self.prog_stake(self.p_hard[i].stake, base, BetRef::Hardway(num));
                        let _ = self.apply_bet(BetRef::Hardway(num), Amount::Cents(want));
                    }
                }
            }
        }
        // Odds behind established come / don't come points can be taken (or
        // topped up after a new flat stacks on) at any time.
        if takes_odds {
            for (i, &num) in PLACE_NUMS.iter().enumerate() {
                if self.come_points[i] > 0 {
                    let _ = self.apply_bet(BetRef::ComeOdds(num), Amount::MaxOdds);
                }
                if self.dc_points[i] > 0 {
                    let _ = self.apply_bet(BetRef::DontComeLay(num), Amount::MaxOdds);
                }
            }
        }
        // One-roll bets, working on every roll.
        if self.sel.field && self.field_bet == 0 {
            let want = self.prog_stake(self.p_field.stake, self.min, BetRef::Field);
            let _ = self.apply_bet(BetRef::Field, Amount::Cents(want));
        }
        if self.sel.any_seven && self.any7_bet == 0 {
            let base = self.rules.prop_bet_cents;
            let want = self.prog_stake(self.p_any7.stake, base, BetRef::AnySeven);
            let _ = self.apply_bet(BetRef::AnySeven, Amount::Cents(want));
        }
        if self.sel.any_craps && self.anycraps_bet == 0 {
            let base = self.rules.prop_bet_cents;
            let want = self.prog_stake(self.p_anycraps.stake, base, BetRef::AnyCraps);
            let _ = self.apply_bet(BetRef::AnyCraps, Amount::Cents(want));
        }
        // Placement is a pure function of cash, bets, and the point; until a
        // resolution changes one of those, running it again is a no-op.
        self.needs_placement = false;
    }

    /// Resolve come and don't come bets for a roll of `t`. `was_comeout` is
    /// whether the main game was on a come-out roll (come odds off then, by
    /// default; don't come lay odds always work).
    pub(crate) fn resolve_come_bets(&mut self, t: u8, was_comeout: bool) {
        let come_odds_working = !was_comeout || self.rules.come_odds_work_on_comeout;
        let progs = self.progressions;

        // Established come points resolve before the new flat travels.
        if t == 7 {
            for (i, &num) in PLACE_NUMS.iter().enumerate() {
                if self.come_points[i] > 0 {
                    self.needs_placement = true;
                    let stake = self.come_points[i];
                    self.resolved_wagered_cents += stake;
                    progs[S_COME].on_loss(&mut self.p_come, self.min, stake);
                    self.come_points[i] = 0;
                    self.emit(BetKind::ComePoint(num), BetEventKind::Lost, stake);
                    if self.come_odds[i] > 0 {
                        let odds = self.come_odds[i];
                        self.come_odds[i] = 0;
                        if come_odds_working {
                            // Working odds lose with the flat: resolved.
                            self.resolved_wagered_cents += odds;
                            self.emit(BetKind::ComeOdds(num), BetEventKind::Lost, odds);
                        } else {
                            self.cash += odds; // off: returned, not resolved
                            self.emit(BetKind::ComeOdds(num), BetEventKind::Returned, odds);
                        }
                    }
                }
                if self.dc_points[i] > 0 {
                    self.needs_placement = true;
                    // Don't come wins: flat even money plus the lay.
                    let flat = self.dc_points[i];
                    let lay = self.dc_lay[i];
                    let lay_win = self.dc_lay_win[i];
                    self.resolved_wagered_cents += flat + lay;
                    self.cash += flat * 2 + lay + lay_win;
                    progs[S_DC].on_win(&mut self.p_dc, self.min, flat);
                    self.dc_points[i] = 0;
                    self.dc_lay[i] = 0;
                    self.dc_lay_win[i] = 0;
                    self.emit(
                        BetKind::DontComePoint(num),
                        BetEventKind::Won {
                            paid_cents: flat,
                            stake_returned: true,
                        },
                        flat,
                    );
                    if lay > 0 {
                        self.emit(
                            BetKind::DontComeLay(num),
                            BetEventKind::Won {
                                paid_cents: lay_win,
                                stake_returned: true,
                            },
                            lay,
                        );
                    }
                }
            }
        } else if let Some(i) = place_index(t) {
            if self.come_points[i] > 0 {
                self.needs_placement = true;
                // Come point made: flat wins even money, bet comes down.
                let flat = self.come_points[i];
                self.resolved_wagered_cents += flat;
                self.cash += flat * 2;
                progs[S_COME].on_win(&mut self.p_come, self.min, flat);
                self.come_points[i] = 0;
                self.emit(
                    BetKind::ComePoint(t),
                    BetEventKind::Won {
                        paid_cents: flat,
                        stake_returned: true,
                    },
                    flat,
                );
                if self.come_odds[i] > 0 {
                    let odds = self.come_odds[i];
                    self.come_odds[i] = 0;
                    if come_odds_working {
                        self.resolved_wagered_cents += odds;
                        let paid = pass_odds_win(odds, t);
                        self.cash += odds + paid;
                        self.emit(
                            BetKind::ComeOdds(t),
                            BetEventKind::Won {
                                paid_cents: paid,
                                stake_returned: true,
                            },
                            odds,
                        );
                    } else {
                        self.cash += odds; // off: returned, not resolved
                        self.emit(BetKind::ComeOdds(t), BetEventKind::Returned, odds);
                    }
                }
            }
            if self.dc_points[i] > 0 {
                self.needs_placement = true;
                // Number repeated: don't come loses flat and lay.
                let flat = self.dc_points[i];
                let lay = self.dc_lay[i];
                self.resolved_wagered_cents += flat + lay;
                progs[S_DC].on_loss(&mut self.p_dc, self.min, flat);
                self.dc_points[i] = 0;
                self.dc_lay[i] = 0;
                self.dc_lay_win[i] = 0;
                self.emit(BetKind::DontComePoint(t), BetEventKind::Lost, flat);
                if lay > 0 {
                    self.emit(BetKind::DontComeLay(t), BetEventKind::Lost, lay);
                }
            }
        }

        // A flat in the come box has its own come-out on this roll.
        if self.come_flat > 0 {
            self.needs_placement = true;
            let flat = self.come_flat;
            match t {
                7 | 11 => {
                    self.resolved_wagered_cents += flat;
                    self.cash += flat * 2;
                    progs[S_COME].on_win(&mut self.p_come, self.min, flat);
                    self.come_flat = 0;
                    self.emit(
                        BetKind::Come,
                        BetEventKind::Won {
                            paid_cents: flat,
                            stake_returned: true,
                        },
                        flat,
                    );
                }
                2 | 3 | 12 => {
                    self.resolved_wagered_cents += flat;
                    progs[S_COME].on_loss(&mut self.p_come, self.min, flat);
                    self.come_flat = 0;
                    self.emit(BetKind::Come, BetEventKind::Lost, flat);
                }
                _ => {
                    // 4/5/6/8/9/10 are the only sums left after the arms above.
                    let i = place_index(t).expect("come flat travels to a box number");
                    self.come_points[i] += flat;
                    self.come_flat = 0;
                    self.emit(BetKind::Come, BetEventKind::Traveled { to: t }, flat);
                }
            }
        }
        if self.dc_flat > 0 {
            self.needs_placement = true;
            let flat = self.dc_flat;
            match t {
                2 | 3 => {
                    self.resolved_wagered_cents += flat;
                    self.cash += flat * 2;
                    progs[S_DC].on_win(&mut self.p_dc, self.min, flat);
                    self.dc_flat = 0;
                    self.emit(
                        BetKind::DontCome,
                        BetEventKind::Won {
                            paid_cents: flat,
                            stake_returned: true,
                        },
                        flat,
                    );
                }
                7 | 11 => {
                    self.resolved_wagered_cents += flat;
                    progs[S_DC].on_loss(&mut self.p_dc, self.min, flat);
                    self.dc_flat = 0;
                    self.emit(BetKind::DontCome, BetEventKind::Lost, flat);
                }
                12 => {
                    // Bar 12: push, stays in the box — not resolved handle.
                    self.emit(BetKind::DontCome, BetEventKind::Pushed, flat);
                }
                _ => {
                    let i = place_index(t).expect("don't come flat travels to a box number");
                    self.dc_points[i] += flat;
                    self.dc_flat = 0;
                    self.emit(BetKind::DontCome, BetEventKind::Traveled { to: t }, flat);
                }
            }
        }
    }

    /// Resolve one roll of the dice.
    pub(crate) fn resolve(&mut self, d1: u8, d2: u8) {
        let t = d1 + d2;
        let is_hard = d1 == d2;
        let was_comeout = self.point.is_none();
        if !F::MASK.is_empty() {
            self.hist.fired = 0;
            self.hist.come_established = 0;
            self.hist.dont_come_established = 0;
            self.hist.last_total_now = t;
            // The win/loss window is only ever written under this same
            // runtime gate, so clearing it under any other one was a hundred
            // and fifty bytes zeroed every roll on behalf of a strategy that
            // never asks what won.
            if self.features.has(FeatureMask::STREAKS) {
                self.hist.won = 0;
                self.hist.lost = 0;
                self.hist.paid = [0; crate::strategy::view::STREAMS];
            }
            if !self.features.is_empty() {
                self.record_roll(t);
            }
        }
        self.resolve_come_bets(t, was_comeout);

        // --- One-roll bets ---
        let progs = self.progressions;
        if self.field_bet > 0 {
            self.needs_placement = true;
            let b = self.field_bet;
            self.field_bet = 0;
            self.resolved_wagered_cents += b;
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
                progs[S_FIELD].on_win(&mut self.p_field, self.min, b * win_mult);
                self.emit(
                    BetKind::Field,
                    BetEventKind::Won {
                        paid_cents: b * win_mult,
                        stake_returned: true,
                    },
                    b,
                );
            } else {
                progs[S_FIELD].on_loss(&mut self.p_field, self.min, b);
                self.emit(BetKind::Field, BetEventKind::Lost, b);
            }
        }
        if self.any7_bet > 0 {
            self.needs_placement = true;
            let b = self.any7_bet;
            self.any7_bet = 0;
            self.resolved_wagered_cents += b;
            if t == 7 {
                self.cash += b * 5; // 4:1
                progs[S_ANY7].on_win(&mut self.p_any7, self.rules.prop_bet_cents, b * 4);
                self.emit(
                    BetKind::AnySeven,
                    BetEventKind::Won {
                        paid_cents: b * 4,
                        stake_returned: true,
                    },
                    b,
                );
            } else {
                progs[S_ANY7].on_loss(&mut self.p_any7, self.rules.prop_bet_cents, b);
                self.emit(BetKind::AnySeven, BetEventKind::Lost, b);
            }
        }
        if self.anycraps_bet > 0 {
            self.needs_placement = true;
            let b = self.anycraps_bet;
            self.anycraps_bet = 0;
            self.resolved_wagered_cents += b;
            if t == 2 || t == 3 || t == 12 {
                self.cash += b * 8; // 7:1
                progs[S_ANYCRAPS].on_win(&mut self.p_anycraps, self.rules.prop_bet_cents, b * 7);
                self.emit(
                    BetKind::AnyCraps,
                    BetEventKind::Won {
                        paid_cents: b * 7,
                        stake_returned: true,
                    },
                    b,
                );
            } else {
                progs[S_ANYCRAPS].on_loss(&mut self.p_anycraps, self.rules.prop_bet_cents, b);
                self.emit(BetKind::AnyCraps, BetEventKind::Lost, b);
            }
        }

        match self.point {
            None => {
                // --- Come-out roll. Place bets & hardways are off. ---
                self.needs_placement = true;
                match t {
                    7 | 11 => {
                        if self.pass > 0 {
                            let stake = self.pass;
                            self.resolved_wagered_cents += stake;
                            self.cash += stake * 2;
                            progs[S_PASS].on_win(&mut self.p_pass, self.min, stake);
                            self.pass = 0;
                            self.emit(
                                BetKind::Pass,
                                BetEventKind::Won {
                                    paid_cents: stake,
                                    stake_returned: true,
                                },
                                stake,
                            );
                        }
                        if self.dont > 0 {
                            let stake = self.dont;
                            self.resolved_wagered_cents += stake;
                            progs[S_DONT].on_loss(&mut self.p_dont, self.min, stake);
                            self.dont = 0; // loses (nothing laid yet on come-out)
                            self.emit(BetKind::DontPass, BetEventKind::Lost, stake);
                        }
                    }
                    2 | 3 => {
                        if self.pass > 0 {
                            let stake = self.pass;
                            self.resolved_wagered_cents += stake;
                            progs[S_PASS].on_loss(&mut self.p_pass, self.min, stake);
                            self.pass = 0;
                            self.emit(BetKind::Pass, BetEventKind::Lost, stake);
                        }
                        if self.dont > 0 {
                            let stake = self.dont;
                            self.resolved_wagered_cents += stake;
                            self.cash += stake * 2;
                            progs[S_DONT].on_win(&mut self.p_dont, self.min, stake);
                            self.dont = 0;
                            self.emit(
                                BetKind::DontPass,
                                BetEventKind::Won {
                                    paid_cents: stake,
                                    stake_returned: true,
                                },
                                stake,
                            );
                        }
                    }
                    12 => {
                        // Bar 12: pass loses, don't pass pushes (stays up,
                        // not resolved handle).
                        if self.pass > 0 {
                            let stake = self.pass;
                            self.resolved_wagered_cents += stake;
                            progs[S_PASS].on_loss(&mut self.p_pass, self.min, stake);
                            self.pass = 0;
                            self.emit(BetKind::Pass, BetEventKind::Lost, stake);
                        }
                        if self.dont > 0 {
                            let stake = self.dont;
                            self.emit(BetKind::DontPass, BetEventKind::Pushed, stake);
                        }
                    }
                    _ => {
                        self.point = Some(t);
                        if !F::MASK.is_empty() {
                            self.hist.fired |= crate::strategy::program::fired::POINT_ESTABLISHED;
                        }
                    }
                }
                // Place bets and hardways a strategy has called on for the
                // come-out. Off is the default and the arm above is written
                // for it; this is the other half, which the language could
                // ask for and the table then ignored — the bet sat there
                // through the one roll its author had said it should work.
                //
                // The seven that wins the line takes them, which is exactly
                // the trade being made.
                if t == 7 {
                    for (i, &num) in PLACE_NUMS.iter().enumerate() {
                        if self.place[i] > 0 && self.place_works_comeout(i) {
                            let stake = self.place[i];
                            self.resolved_wagered_cents += stake;
                            let base = place_stake(self.min, num);
                            progs[S_PLACE + i].on_loss(&mut self.p_place[i], base, stake);
                            self.place[i] = 0;
                            self.emit(BetKind::Place(num), BetEventKind::Lost, stake);
                        }
                    }
                    for (i, &num) in HARD_NUMS.iter().enumerate() {
                        if self.hard[i] > 0 && self.hard_works_comeout(i) {
                            let stake = self.hard[i];
                            self.resolved_wagered_cents += stake;
                            progs[S_HARD + i].on_loss(
                                &mut self.p_hard[i],
                                self.rules.prop_bet_cents,
                                stake,
                            );
                            self.hard[i] = 0;
                            self.emit(BetKind::Hardway(num), BetEventKind::Lost, stake);
                        }
                    }
                } else {
                    if let Some(i) = place_index(t) {
                        if self.place[i] > 0 && self.place_works_comeout(i) {
                            self.resolve_place_hit(i, t, &progs);
                        }
                    }
                    if let Some(i) = hard_index(t) {
                        if self.hard[i] > 0 && self.hard_works_comeout(i) {
                            self.resolve_hardway(i, t, is_hard, &progs);
                        }
                    }
                }
            }
            Some(point) => {
                if t == 7 {
                    // --- Seven-out ---
                    self.needs_placement = true;
                    if self.pass > 0 {
                        let stake = self.pass;
                        self.resolved_wagered_cents += stake;
                        progs[S_PASS].on_loss(&mut self.p_pass, self.min, stake);
                        self.pass = 0;
                        self.emit(BetKind::Pass, BetEventKind::Lost, stake);
                    }
                    if self.pass_odds > 0 {
                        let stake = self.pass_odds;
                        self.resolved_wagered_cents += stake;
                        self.pass_odds = 0;
                        self.emit(BetKind::PassOdds, BetEventKind::Lost, stake);
                    }
                    if self.dont > 0 {
                        let stake = self.dont;
                        self.resolved_wagered_cents += stake;
                        self.cash += stake * 2;
                        progs[S_DONT].on_win(&mut self.p_dont, self.min, stake);
                        self.dont = 0;
                        self.emit(
                            BetKind::DontPass,
                            BetEventKind::Won {
                                paid_cents: stake,
                                stake_returned: true,
                            },
                            stake,
                        );
                    }
                    if self.dont_lay > 0 {
                        let stake = self.dont_lay;
                        let paid = self.dont_lay_win;
                        self.resolved_wagered_cents += stake;
                        self.cash += stake + paid;
                        self.dont_lay = 0;
                        self.dont_lay_win = 0;
                        self.emit(
                            BetKind::DontPassLay,
                            BetEventKind::Won {
                                paid_cents: paid,
                                stake_returned: true,
                            },
                            stake,
                        );
                    }
                    for (i, &num) in PLACE_NUMS.iter().enumerate() {
                        if self.place[i] > 0 && self.place_is_working(i) {
                            // working place bets lose
                            let stake = self.place[i];
                            self.resolved_wagered_cents += stake;
                            let base = place_stake(self.min, num);
                            progs[S_PLACE + i].on_loss(&mut self.p_place[i], base, stake);
                            self.place[i] = 0;
                            self.emit(BetKind::Place(num), BetEventKind::Lost, stake);
                        }
                    }
                    for (i, &num) in HARD_NUMS.iter().enumerate() {
                        if self.hard[i] > 0 && self.hard_is_working(i) {
                            let stake = self.hard[i];
                            self.resolved_wagered_cents += stake;
                            progs[S_HARD + i].on_loss(
                                &mut self.p_hard[i],
                                self.rules.prop_bet_cents,
                                stake,
                            );
                            self.hard[i] = 0;
                            self.emit(BetKind::Hardway(num), BetEventKind::Lost, stake);
                        }
                    }
                    self.point = None;
                    if !F::MASK.is_empty() {
                        self.hist.fired |= crate::strategy::program::fired::SEVEN_OUT;
                    }
                    if F::MASK.has(FeatureMask::DICE.with(FeatureMask::HITS))
                        && self.features.has(FeatureMask::DICE.with(FeatureMask::HITS))
                    {
                        self.end_shooter();
                    }
                    if F::MASK.has(FeatureMask::PEAK) && self.features.has(FeatureMask::PEAK) {
                        self.record_peak();
                    }
                    return;
                }

                // Place bets (working while a point is on). A winning place
                // bet stays up, and the progression presses or regresses it
                // in place out of (or back into) the player's rail.
                if let Some(i) = place_index(t) {
                    if self.place[i] > 0 && self.place_is_working(i) {
                        self.resolve_place_hit(i, t, &progs);
                    }
                }
                // Hardways: winners stay up, pressed the same way.
                if let Some(i) = hard_index(t) {
                    if self.hard[i] > 0 && self.hard_is_working(i) {
                        self.resolve_hardway(i, t, is_hard, &progs);
                    }
                }

                if t == point {
                    // --- Point made ---
                    self.needs_placement = true;
                    if self.pass > 0 {
                        let stake = self.pass;
                        self.resolved_wagered_cents += stake;
                        self.cash += stake * 2;
                        progs[S_PASS].on_win(&mut self.p_pass, self.min, stake);
                        self.pass = 0;
                        self.emit(
                            BetKind::Pass,
                            BetEventKind::Won {
                                paid_cents: stake,
                                stake_returned: true,
                            },
                            stake,
                        );
                    }
                    if self.pass_odds > 0 {
                        let stake = self.pass_odds;
                        let paid = pass_odds_win(stake, self.pass_odds_point);
                        self.resolved_wagered_cents += stake;
                        self.cash += stake + paid;
                        self.pass_odds = 0;
                        self.emit(
                            BetKind::PassOdds,
                            BetEventKind::Won {
                                paid_cents: paid,
                                stake_returned: true,
                            },
                            stake,
                        );
                    }
                    if self.dont > 0 {
                        let stake = self.dont;
                        self.resolved_wagered_cents += stake;
                        progs[S_DONT].on_loss(&mut self.p_dont, self.min, stake);
                        self.dont = 0;
                        self.emit(BetKind::DontPass, BetEventKind::Lost, stake);
                    }
                    if self.dont_lay > 0 {
                        let stake = self.dont_lay;
                        self.resolved_wagered_cents += stake;
                        self.dont_lay = 0;
                        self.dont_lay_win = 0;
                        self.emit(BetKind::DontPassLay, BetEventKind::Lost, stake);
                    }
                    self.point = None;
                    if !F::MASK.is_empty() {
                        self.hist.fired |= crate::strategy::program::fired::POINT_MADE;
                    }
                }
            }
        }
        if F::MASK.has(FeatureMask::PEAK) && self.features.has(FeatureMask::PEAK) {
            self.record_peak();
        }
    }

    /// Whether a place bet is working. The built-in player has no way to
    /// call a bet off — nothing in `BetSelection` can say it — so for that
    /// player `F::MASK` is empty, this is a constant `true`, and the flag
    /// is never loaded. Leaving it as a runtime read cost the loaded table
    /// a third of its throughput, which is what the paired benchmark is for.
    #[inline]
    /// A place bet's number came. The winner stays up, and the progression
    /// presses or regresses it in place out of (or back into) the rail.
    ///
    /// Extracted so the come-out path and the point-on path resolve a hit
    /// through one definition. They differ in *when* a bet is working, which
    /// their callers decide; what a hit is worth is the same question at
    /// either end of the table, and two answers to it would drift.
    fn resolve_place_hit(&mut self, i: usize, t: u8, progs: &[Progression; STREAMS]) {
        self.needs_placement = true;
        // The winning stake resolves once; the press that follows is a fresh
        // placement, not a resolution.
        let cur = self.place[i];
        let paid = place_win(cur, t);
        self.resolved_wagered_cents += cur;
        self.cash += paid;
        let base = place_stake(self.min, t);
        progs[S_PLACE + i].on_win(&mut self.p_place[i], base, paid);
        self.emit(
            BetKind::Place(t),
            BetEventKind::Won {
                paid_cents: paid,
                // The winner stays up.
                stake_returned: false,
            },
            cur,
        );
        let desired = self.prog_place_stake(i);
        if desired > cur {
            if self.try_stake(desired - cur).is_some() {
                self.place[i] = desired;
                self.emit(BetKind::Place(t), BetEventKind::Placed, desired - cur);
            }
        } else if desired < cur {
            self.cash += cur - desired;
            self.place[i] = desired;
            self.emit(BetKind::Place(t), BetEventKind::TakenDown, cur - desired);
        }
    }

    /// A hardway's number came, hard or easy. Hard wins and stays up; easy
    /// loses.
    fn resolve_hardway(&mut self, i: usize, t: u8, is_hard: bool, progs: &[Progression; STREAMS]) {
        self.needs_placement = true;
        let cur = self.hard[i];
        self.resolved_wagered_cents += cur;
        let base = self.rules.prop_bet_cents;
        if !is_hard {
            progs[S_HARD + i].on_loss(&mut self.p_hard[i], base, cur);
            self.hard[i] = 0; // easy way loses
            self.emit(BetKind::Hardway(t), BetEventKind::Lost, cur);
            return;
        }
        let paid = hardway_win(cur, t);
        self.cash += paid;
        progs[S_HARD + i].on_win(&mut self.p_hard[i], base, paid);
        self.emit(
            BetKind::Hardway(t),
            BetEventKind::Won {
                paid_cents: paid,
                // The winner stays up.
                stake_returned: false,
            },
            cur,
        );
        let desired = self.prog_stake(self.p_hard[i].stake, base, BetRef::Hardway(t));
        if desired > cur {
            if self.try_stake(desired - cur).is_some() {
                self.hard[i] = desired;
                self.emit(BetKind::Hardway(t), BetEventKind::Placed, desired - cur);
            }
        } else if desired < cur {
            self.cash += cur - desired;
            self.hard[i] = desired;
            self.emit(BetKind::Hardway(t), BetEventKind::TakenDown, cur - desired);
        }
    }

    /// Whether this bet resolves on a come-out roll.
    ///
    /// A bet that has been called off is off everywhere, so this is the two
    /// flags together. `place_is_working` short-circuits to true when there
    /// are no features, because the checkbox player has no way to call a bet
    /// off; this one must not, because it would then say every place bet
    /// works on every come-out and move every outcome this engine has ever
    /// pinned.
    #[inline]
    fn place_works_comeout(&self, i: usize) -> bool {
        !F::MASK.is_empty() && self.place_working[i] && self.place_comeout[i]
    }

    #[inline]
    fn hard_works_comeout(&self, i: usize) -> bool {
        !F::MASK.is_empty() && self.hard_working[i] && self.hard_comeout[i]
    }

    #[inline]
    fn place_is_working(&self, i: usize) -> bool {
        F::MASK.is_empty() || self.place_working[i]
    }

    #[inline]
    fn hard_is_working(&self, i: usize) -> bool {
        F::MASK.is_empty() || self.hard_working[i]
    }

    /// Per-roll derived history. Split out of [`Session::resolve`] and
    /// called only when something reads it, so the built-in player's hot
    /// path is exactly what it was before this existed.
    fn record_roll(&mut self, total: u8) {
        if self.features.has(FeatureMask::DICE) {
            self.hist.last_total = total;
            self.hist.roll += 1;
            self.hist.rolls_this_shooter += 1;
        }
        if self.features.has(FeatureMask::HITS) {
            let i = total as usize;
            self.hist.hits[i] = self.hist.hits[i].saturating_add(1);
            self.hist.hits_shooter[i] = self.hist.hits_shooter[i].saturating_add(1);
        }
    }

    /// A seven-out passes the dice: this shooter's counts start over, the
    /// session's do not.
    fn end_shooter(&mut self) {
        self.hist.shooter += 1;
        self.hist.rolls_this_shooter = 0;
        self.hist.hits_shooter = [0; 13];
    }

    fn record_peak(&mut self) {
        let wealth = self.cash + self.on_table_face();
        if wealth > self.hist.peak_wealth {
            self.hist.peak_wealth = wealth;
        }
    }

    /// The read-only borrow a strategy is handed at its decision point.
    /// Never called by the built-in player, which reads the session
    /// directly; the compiled interpreter is its first caller.
    #[allow(dead_code, reason = "the interpreter is its first non-test caller")]
    pub(crate) fn view(&self) -> TableView<'_> {
        TableView {
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bets::{OddsPolicy, Progression};
    use crate::rng::Xoshiro256pp;

    fn rules() -> Rules {
        Rules {
            odds_policy: OddsPolicy::None,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 1000,
            place_the_point: false,
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
            place_the_point: false,
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
            place_the_point: false,
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
}
