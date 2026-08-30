// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The single source of truth for a simulation's configuration.
//!
//! [`SimConfig`] is what the Scenario Sentence renders and parses, what the
//! workers receive, and what [`SimConfig::fingerprint`] hashes for staleness
//! detection. The fingerprint is FNV-1a over a canonical little-endian byte
//! stream — hand-rolled so it is stable across platforms and releases (no
//! `std::hash`, whose byte order is platform-dependent; no serde).

use craps_engine::{
    cheapest_selected_stake, BetSelection, ExploreConfig, OddsPolicy, Progression, Rules,
    SweepConfig,
};

/// A strategy, as a configuration refers to one: by the name it was saved
/// under and the content hash of its compiled form.
///
/// `STRATEGY_DSL.md` §10. The body does not travel in the sentence — a rule
/// set does not fit in a line of prose, and pretending otherwise would break
/// the one contract this whole app leans on. The hash is what makes the
/// reference safe: a pasted sentence naming a strategy this machine has
/// under that name but with different rules is STALE, loudly, rather than
/// silently running the wrong player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrategyRef {
    pub name: String,
    /// The low 32 bits of the compiled program's FNV-1a hash.
    ///
    /// Short enough to read off a sentence and compare by eye, which is the
    /// point of it; long enough that an edited strategy will not collide
    /// with the one it replaced, which is the job of it. It is stored at the
    /// width it is written, so what the sentence carries is the whole
    /// reference and not a truncation of one.
    pub hash: u32,
}

impl StrategyRef {
    /// The hash as the sentence writes it.
    pub fn short(&self) -> String {
        format!("{:08x}", self.hash)
    }

    /// The reference to a compiled program.
    pub fn of(p: &craps_engine::strategy::Program) -> Self {
        Self {
            name: p.name.clone(),
            hash: p.hash as u32,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SimConfig {
    pub budget_cents: i64,
    /// Take-profit multiple of the starting budget, if the quit rule is on.
    pub quit_mult: Option<f64>,
    pub sessions: u32,
    pub rolls_per_hour: u32,
    pub target_hours: f64,
    /// Survival confidence for the recommended budget, in percent.
    pub confidence: f64,
    /// Table minimums in cents, ascending, deduped.
    pub table_mins_cents: Vec<i64>,
    pub max_rolls: u64,
    pub sel: BetSelection,
    /// The strategy playing, when one is. `None` means the bet rail above
    /// is the player, which is what every screen assumed before strategies
    /// existed and still assumes when this is `None`.
    pub strategy: Option<StrategyRef>,
    pub odds_policy: OddsPolicy,
    pub field_12_triple: bool,
    pub come_odds_work_on_comeout: bool,
    pub prop_bet_cents: i64,
    pub table_max_mult: i64,
    // --- explorer knobs (fingerprinted separately) ---
    pub explore_sessions: u32,
    pub explore_flat_only: bool,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            budget_cents: 30_000,
            quit_mult: None,
            sessions: 100_000,
            rolls_per_hour: 100,
            target_hours: 4.0,
            confidence: 95.0,
            table_mins_cents: vec![500, 1000, 1500, 2500, 5000, 10_000],
            max_rolls: 200_000,
            sel: BetSelection::default(),
            strategy: None,
            odds_policy: OddsPolicy::X345,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 500,
            explore_sessions: 5_000,
            explore_flat_only: false,
        }
    }
}

impl SimConfig {
    /// The configuration as the scenario it actually describes.
    ///
    /// When a strategy is the player, the bet rail is leftover interface
    /// state and not part of the scenario: the sentence does not carry it,
    /// changing it must not strike results stale, and two configurations
    /// differing only there are the same run.
    pub fn canonical(&self) -> SimConfig {
        let mut c = self.clone();
        if c.strategy.is_some() {
            c.sel = BetSelection {
                pass_line: false,
                ..Default::default()
            };
        }
        c
    }

    pub fn horizon_rolls(&self) -> u64 {
        (self.target_hours * self.rolls_per_hour as f64)
            .ceil()
            .max(1.0) as u64
    }

    pub fn quit_target_cents(&self) -> Option<i64> {
        self.quit_mult
            .map(|m| craps_engine::quit_target_cents(self.budget_cents, m))
    }

    pub fn rules(&self) -> Rules {
        Rules {
            // `take_odds` is a bet-rail control: it says whether *that*
            // player backs its line bets. A strategy says so in its own
            // rules, so gating the table's odds policy on a checkbox that
            // is not in play would refuse every `max` a strategy asked for.
            odds_policy: if self.strategy.is_some() || self.sel.take_odds {
                self.odds_policy
            } else {
                OddsPolicy::None
            },
            field_12_triple: self.field_12_triple,
            come_odds_work_on_comeout: self.come_odds_work_on_comeout,
            prop_bet_cents: self.prop_bet_cents.max(100),
            table_max_mult: self.table_max_mult.max(1),
        }
    }

    /// The sweep this configuration describes, played by `program` when a
    /// strategy is live and by the checkbox selection otherwise.
    pub fn to_sweep(
        &self,
        base_seed: u64,
        program: Option<std::sync::Arc<craps_engine::strategy::Program>>,
    ) -> SweepConfig {
        SweepConfig {
            sel: self.sel.clone(),
            program,
            rules: self.rules(),
            mins: self.table_mins_cents.clone(),
            budget_cents: self.budget_cents,
            quit_target_cents: self.quit_target_cents(),
            sessions: self.sessions.max(100) as u64,
            max_rolls: self.max_rolls.max(1000),
            horizon_rolls: self.horizon_rolls(),
            base_seed,
        }
    }

    /// The explorer's rules: payout knobs from the setup, with the odds
    /// policy always available to the strategies that take odds (the main
    /// selection's take_odds gate does not apply). The Duel re-simulates
    /// explorer combos, so it MUST use these rules, not [`Self::rules`].
    pub fn explore_rules(&self) -> Rules {
        Rules {
            odds_policy: self.odds_policy,
            field_12_triple: self.field_12_triple,
            come_odds_work_on_comeout: self.come_odds_work_on_comeout,
            prop_bet_cents: self.prop_bet_cents.max(100),
            table_max_mult: self.table_max_mult.max(1),
        }
    }

    pub fn to_explore(
        &self,
        base_seed: u64,
        program: Option<std::sync::Arc<craps_engine::strategy::Program>>,
    ) -> ExploreConfig {
        ExploreConfig {
            program,
            rules: self.explore_rules(),
            mins: self.table_mins_cents.clone(),
            budget_cents: self.budget_cents,
            sessions: self.explore_sessions.max(500) as u64,
            horizon_rolls: self.horizon_rolls(),
            flat_only: self.explore_flat_only,
            base_seed,
        }
    }

    /// Plain-language validation, or Ok. The Design screen's order-ticket
    /// strip renders these; Run is disabled while any exist.
    pub fn validate(&self) -> Result<(), String> {
        // Which player is live decides what there is to validate. A strategy
        // brings its own bets, and a pasted strategy sentence deliberately
        // blanks the rail — so interrogating the bet rail here refused a
        // perfectly good scenario with "Select at least one bet type", and
        // the paste-a-sentence-and-run loop dead-ended at its last step.
        let playing_strategy = self.strategy.is_some();
        if !playing_strategy && !self.sel.any_selected() {
            return Err("Select at least one bet type.".to_owned());
        }
        if !playing_strategy
            && self.sel.take_odds
            && !self.sel.pass_line
            && !self.sel.dont_pass
            && self.sel.come_max == 0
            && self.sel.dont_come_max == 0
        {
            return Err("Odds require a Pass, Don't Pass, Come, or Don't Come bet.".to_owned());
        }
        if self.table_mins_cents.is_empty() {
            return Err("Enter at least one table minimum.".to_owned());
        }
        let rules = self.rules();
        // A strategy's cheapest bet is computed from its own rules, by the
        // compiled program, at the moment it runs. Asking the bet rail what
        // this costs would be asking a player who is not at the table.
        let unplayable: Vec<String> = if playing_strategy {
            Vec::new()
        } else {
            self.table_mins_cents
                .iter()
                .filter(|&&min| self.budget_cents < cheapest_selected_stake(&self.sel, &rules, min))
                .map(|&m| crate::ui::numerals::money_text(m, false))
                .collect()
        };
        if !unplayable.is_empty() {
            return Err(format!(
                "Budget ${} can't cover the first bet at table minimum(s) {} — raise the budget or remove them.",
                self.budget_cents / 100,
                unplayable.join(", ")
            ));
        }
        Ok(())
    }

    /// Fingerprint of every setting that affects the MAIN run's results.
    /// Explorer-only knobs are excluded, so tuning them never marks the main
    /// findings stale (and vice versa via [`Self::explore_fingerprint`]).
    pub fn fingerprint(&self) -> u64 {
        let this = self.canonical();
        let self_ = &this;
        let mut f = Fnv::new();
        f.tag(0x01);
        f.i64(self_.budget_cents);
        match self_.quit_mult {
            None => f.tag(0),
            Some(m) => {
                f.tag(1);
                f.f64(m);
            }
        }
        f.u32(self_.sessions);
        f.u32(self_.rolls_per_hour);
        f.f64(self_.target_hours);
        f.f64(self_.confidence);
        f.u64(self_.max_rolls);
        f.tag(self_.table_mins_cents.len() as u8);
        for &m in &self_.table_mins_cents {
            f.i64(m);
        }
        self_.write_shared(&mut f);
        // Which player is live is part of what a run was cut from, so a
        // change of strategy has to strike results stale exactly as a
        // change of bets does.
        match &self_.strategy {
            None => f.tag(0),
            Some(r) => {
                f.tag(1);
                for b in r.name.as_bytes() {
                    f.tag(*b);
                }
                f.u32(r.hash);
            }
        }
        f.finish()
    }

    /// Fingerprint of every setting that affects the EXPLORER's results.
    pub fn explore_fingerprint(&self) -> u64 {
        let mut f = Fnv::new();
        f.tag(0x02);
        f.i64(self.budget_cents);
        f.u32(self.rolls_per_hour);
        f.f64(self.target_hours);
        f.tag(self.table_mins_cents.len() as u8);
        for &m in &self.table_mins_cents {
            f.i64(m);
        }
        f.u32(self.explore_sessions);
        f.tag(self.explore_flat_only as u8);
        // Payout rules are shared with the main run; the explorer always has
        // the odds policy available regardless of sel.take_odds.
        f.tag(match self.odds_policy {
            OddsPolicy::None => 0,
            OddsPolicy::X1 => 1,
            OddsPolicy::X2 => 2,
            OddsPolicy::X345 => 3,
            OddsPolicy::X5 => 4,
            OddsPolicy::X10 => 5,
        });
        f.tag(self.field_12_triple as u8);
        f.tag(self.come_odds_work_on_comeout as u8);
        f.i64(self.prop_bet_cents);
        f.i64(self.table_max_mult);
        f.finish()
    }

    /// The settings shared by the fingerprint: bet selection and house rules.
    fn write_shared(&self, f: &mut Fnv) {
        let s = &self.sel;
        f.tag(s.pass_line as u8);
        f.tag(s.dont_pass as u8);
        f.tag(s.come_max);
        f.tag(s.dont_come_max);
        f.tag(s.take_odds as u8);
        f.tag(s.field as u8);
        for &p in &s.place {
            f.tag(p as u8);
        }
        for &h in &s.hardways {
            f.tag(h as u8);
        }
        f.tag(s.any_seven as u8);
        f.tag(s.any_craps as u8);
        f.tag(progression_index(s.progression));
        f.tag(match self.odds_policy {
            OddsPolicy::None => 0,
            OddsPolicy::X1 => 1,
            OddsPolicy::X2 => 2,
            OddsPolicy::X345 => 3,
            OddsPolicy::X5 => 4,
            OddsPolicy::X10 => 5,
        });
        f.tag(self.field_12_triple as u8);
        f.tag(self.come_odds_work_on_comeout as u8);
        f.i64(self.prop_bet_cents);
        f.i64(self.table_max_mult);
    }
}

/// Parse the table-minimums editor text: dollar amounts, comma or space
/// separated, optional `$`. Sorted, deduped, cents-exact.
pub fn parse_table_mins(text: &str) -> Result<Vec<i64>, String> {
    let mut mins = Vec::new();
    for tok in text.split(|c: char| c == ',' || c.is_whitespace()) {
        if tok.is_empty() {
            continue;
        }
        let v: f64 = tok
            .trim_start_matches('$')
            .parse()
            .map_err(|_| format!("Could not parse table minimum \"{tok}\""))?;
        if !v.is_finite() || v > 100_000.0 {
            return Err(format!("Table minimum \"{tok}\" is out of range"));
        }
        let cents = (v * 100.0).round() as i64;
        if cents < 1 {
            return Err(format!("Table minimum \"{tok}\" must be at least $0.01"));
        }
        mins.push(cents);
    }
    if mins.is_empty() {
        return Err("Enter at least one table minimum".to_owned());
    }
    mins.sort_unstable();
    mins.dedup();
    Ok(mins)
}

/// Stable index of a progression (its position in [`Progression::ALL`]).
pub fn progression_index(p: Progression) -> u8 {
    Progression::ALL
        .iter()
        .position(|&q| q == p)
        .expect("progression in ALL") as u8
}

/// Hand-rolled FNV-1a over an explicit little-endian byte stream: stable
/// across platforms, compilers, and releases.
pub struct Fnv(u64);

impl Fnv {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    pub fn new() -> Self {
        Fnv(Self::OFFSET)
    }

    fn byte(&mut self, b: u8) {
        self.0 ^= b as u64;
        self.0 = self.0.wrapping_mul(Self::PRIME);
    }

    pub fn tag(&mut self, b: u8) {
        self.byte(b);
    }

    pub fn u32(&mut self, v: u32) {
        for b in v.to_le_bytes() {
            self.byte(b);
        }
    }

    pub fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.byte(b);
        }
    }

    pub fn i64(&mut self, v: i64) {
        self.u64(v as u64);
    }

    pub fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl Default for Fnv {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_pinned_for_the_default_config() {
        // Stability contract: if this value changes, stored provenance from
        // earlier builds would silently read as stale (or worse, fresh).
        // Update it ONLY for a deliberate, documented fingerprint change.
        let fp = SimConfig::default().fingerprint();
        assert_eq!(fp, {
            // Self-check that the value is deterministic within a process.
            SimConfig::default().fingerprint()
        });
        // Cross-platform stability is by construction (explicit LE bytes);
        // pin the actual value so any accidental format change fails loudly.
        // Changed deliberately when the configuration learned which
        // player is live (STRATEGY_DSL.md §10): a run cut from a strategy
        // and one cut from the bet rail are different runs, so the
        // fingerprint has to tell them apart or staleness cannot.
        assert_eq!(fp, 9031665053774321689, "fingerprint format changed");
    }

    #[test]
    fn fingerprint_sees_every_main_field() {
        let base = SimConfig::default();
        let fp = base.fingerprint();
        let mut c = base.clone();
        c.budget_cents += 100;
        assert_ne!(c.fingerprint(), fp);
        let mut c = base.clone();
        c.quit_mult = Some(2.0);
        assert_ne!(c.fingerprint(), fp);
        let mut c = base.clone();
        c.sel.set_place(6, true);
        assert_ne!(c.fingerprint(), fp);
        let mut c = base.clone();
        c.sel.progression = Progression::Martingale;
        assert_ne!(c.fingerprint(), fp);
        let mut c = base.clone();
        c.field_12_triple = true;
        assert_ne!(c.fingerprint(), fp);
        let mut c = base.clone();
        c.table_mins_cents.push(20_000);
        assert_ne!(c.fingerprint(), fp);
        // Explorer knobs must NOT disturb the main fingerprint…
        let mut c = base.clone();
        c.explore_sessions = 9_999;
        c.explore_flat_only = true;
        assert_eq!(c.fingerprint(), fp);
        // …but must disturb the explorer fingerprint.
        assert_ne!(c.explore_fingerprint(), base.explore_fingerprint());
    }

    #[test]
    fn parse_table_mins_happy_path() {
        assert_eq!(
            parse_table_mins("5, 10, 25").unwrap(),
            vec![500, 1000, 2500]
        );
        assert_eq!(parse_table_mins("$5 $10").unwrap(), vec![500, 1000]);
        assert_eq!(parse_table_mins("10, 5, 10").unwrap(), vec![500, 1000]); // sorted, deduped
        assert_eq!(parse_table_mins("7.50").unwrap(), vec![750]);
    }

    #[test]
    fn parse_table_mins_rejects_bad_input() {
        assert!(parse_table_mins("").is_err());
        assert!(parse_table_mins("abc").is_err());
        assert!(parse_table_mins("-5").is_err());
        assert!(parse_table_mins("0").is_err());
        assert!(parse_table_mins("nan").is_err());
        assert!(parse_table_mins("NaN").is_err());
        assert!(parse_table_mins("inf").is_err());
        assert!(parse_table_mins("0.001").is_err()); // rounds to zero cents
        assert!(parse_table_mins("1e9").is_err()); // out of range
        assert!(parse_table_mins("5, nan, 10").is_err());
    }

    #[test]
    fn validation_matches_the_old_ui_rules() {
        let mut c = SimConfig::default();
        c.sel.pass_line = false;
        assert!(c.validate().is_err());
        c.sel.take_odds = true;
        assert!(c.validate().is_err());
        c.sel.pass_line = true;
        assert!(c.validate().is_ok());
        c.budget_cents = 100; // $1 can't cover a $5 line bet
        assert!(c.validate().unwrap_err().contains("can't cover"));
    }
}
