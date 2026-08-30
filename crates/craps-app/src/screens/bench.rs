// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The Bench: one session, stepped, with every decision attributed.
//!
//! `STRATEGY_DSL.md` §8 calls this the feature that decides whether the
//! language is usable, and not optional. A rule set that quietly does
//! nothing and returns a confident distribution is the worst thing the
//! strategy surface can produce; this is where that becomes impossible to
//! miss. Rules that fired are lit. Rules that never fired carry a zero.
//! Every refusal says why, in words.
//!
//! It shows one seed, not a distribution — deliberately. The Findings
//! screens are where a million nights live; this is where one night is
//! read line by line.

use craps_engine::strategy::{
    bench_session, compile, from_selection, parse, render, render_rule, BenchTrace, Program,
    Strategy,
};
use egui::{FontId, RichText, Stroke};

use crate::app::App;
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale};

pub struct BenchState {
    /// The strategy under test, as text. Starts empty; the button fills it
    /// from whatever the bet rail is currently describing, which is also
    /// the fastest way to see what the language looks like.
    pub source: String,
    pub seed_text: String,
    pub parsed: Option<Strategy>,
    pub program: Option<Program>,
    /// Why the source could not be read or compiled, in the words the
    /// author needs — never a red outline alone.
    pub error: Option<String>,
    pub trace: Option<BenchTrace>,
    /// Playhead in rolls; 0 is the table before the first throw.
    pub position: usize,
    /// The name this strategy is saved under, and the text it had when it
    /// was last written or read. Together they answer "is there unsaved
    /// work here", which is the only thing standing between a user and
    /// losing what they wrote.
    pub save_name: String,
    pub saved_source: Option<String>,
    /// Saved strategies on disk, refreshed when the panel needs them.
    pub library: Vec<crate::store_strategies::SavedStrategy>,
    pub library_loaded: bool,
    /// The last thing the library did, in words.
    pub library_note: Option<String>,
    /// Whether the panel starts expanded. It does not, because the Design
    /// screen belongs to the bet rail; the Bench is for when something is
    /// not doing what its author expected.
    pub open: bool,
}

impl Default for BenchState {
    fn default() -> Self {
        Self {
            source: String::new(),
            seed_text: "1".into(),
            parsed: None,
            program: None,
            error: None,
            trace: None,
            position: 0,
            save_name: String::new(),
            saved_source: None,
            library: Vec::new(),
            library_loaded: false,
            library_note: None,
            open: false,
        }
    }
}

impl BenchState {
    /// Read and compile the source, keeping whichever of the two failed as
    /// the message. Clears any trace: a ledger from a strategy that no
    /// longer exists is the kind of stale truth this app refuses.
    pub fn build(&mut self) {
        self.trace = None;
        self.position = 0;
        self.parsed = None;
        self.program = None;
        if self.source.trim().is_empty() {
            self.error =
                Some("Nothing to run yet — take the current player, or paste a strategy.".into());
            return;
        }
        match parse(&self.source) {
            Err(e) => self.error = Some(e.message()),
            Ok(s) => match compile(&s) {
                Err(e) => {
                    self.error = Some(e.message());
                    self.parsed = Some(s);
                }
                Ok(p) => {
                    self.error = None;
                    self.parsed = Some(s);
                    self.program = Some(p);
                }
            },
        }
    }

    pub fn step(&mut self, dir: i64) {
        let max = self.trace.as_ref().map_or(0, |t| t.rolls.len());
        let next = self.position as i64 + dir;
        self.position = next.clamp(0, max as i64) as usize;
    }

    fn seed(&self) -> u64 {
        self.seed_text.trim().parse().unwrap_or(1)
    }

    /// Whether the editor holds work that is not on disk.
    pub fn dirty(&self) -> bool {
        !self.source.trim().is_empty() && self.saved_source.as_deref() != Some(self.source.as_str())
    }

    pub fn refresh_library(&mut self) {
        if let Some(dir) = crate::store_strategies::strategies_dir() {
            self.library = crate::store_strategies::list(&dir);
        }
        self.library_loaded = true;
    }

    /// Write the editor's contents under `save_name`, defaulting to the
    /// strategy's own declared name so the common case needs no typing.
    pub fn save(&mut self) {
        let name = if self.save_name.trim().is_empty() {
            self.parsed
                .as_ref()
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "untitled".into())
        } else {
            self.save_name.trim().to_owned()
        };
        let Some(dir) = crate::store_strategies::strategies_dir() else {
            self.library_note = Some("No place to save on this system.".into());
            return;
        };
        match crate::store_strategies::save(&dir, &name, &self.source) {
            Ok(_) => {
                self.save_name = crate::store_strategies::sanitize(&name);
                self.saved_source = Some(self.source.clone());
                self.library_note = Some(format!("Saved as \"{}\"", self.save_name));
                self.refresh_library();
            }
            Err(e) => self.library_note = Some(e),
        }
    }

    pub fn load_from(&mut self, entry: &crate::store_strategies::SavedStrategy) {
        match crate::store_strategies::load(&entry.path) {
            Ok(text) => {
                self.source = text.clone();
                self.saved_source = Some(text);
                self.save_name = entry.name.clone();
                self.library_note = None;
                self.build();
            }
            Err(e) => self.library_note = Some(e),
        }
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    let heading = RichText::new("The Bench")
        .font(FontId::new(type_scale::SECTION, theme::sans()))
        .color(t.ink);
    let resp = egui::CollapsingHeader::new(heading)
        .id_salt("bench_panel")
        .default_open(app.bench.open);
    resp.show(ui, |ui| {
        ui.label(
            RichText::new(
                "One session, stepped. Which rules fired, what each asked for, \
                 and what the table did about it.",
            )
            .font(FontId::new(type_scale::CAPTION, theme::sans()))
            .color(t.ink2),
        );
        ui.add_space(6.0);
        source_box(app, ui);
        ui.add_space(6.0);
        library_row(app, ui);
        ui.add_space(6.0);
        controls(app, ui);
        ui.add_space(6.0);
        status(app, ui);
        if app.bench.trace.is_some() {
            ui.add_space(8.0);
            transport(app, ui);
            ui.add_space(8.0);
            ui.columns(2, |cols| {
                rules_column(app, &mut cols[0]);
                ledger_column(app, &mut cols[1]);
            });
        }
    });
}

/// Save, load, and delete. A strategy the user wrote and cannot get back is
/// worse than one they never wrote, so this row exists before the editor
/// does.
fn library_row(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    if !app.bench.library_loaded {
        app.bench.refresh_library();
    }
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("saved as")
                .font(FontId::new(type_scale::CAPTION, theme::sans()))
                .color(t.ink2),
        );
        ui.add(
            egui::TextEdit::singleline(&mut app.bench.save_name)
                .desired_width(170.0)
                .hint_text("name")
                .font(FontId::new(type_scale::CAPTION, theme::mono())),
        );
        let has_source = !app.bench.source.trim().is_empty();
        if ui
            .add_enabled(has_source, egui::Button::new("Save"))
            .clicked()
        {
            app.bench.save();
        }

        let entries = app.bench.library.clone();
        let current = if app.bench.save_name.is_empty() {
            "Open…".to_owned()
        } else {
            app.bench.save_name.clone()
        };
        egui::ComboBox::from_id_salt("bench_library")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for e in &entries {
                    if ui.selectable_label(false, &e.name).clicked() {
                        app.bench.load_from(e);
                    }
                }
                if entries.is_empty() {
                    ui.label(
                        RichText::new("nothing saved yet")
                            .font(FontId::new(type_scale::CAPTION, theme::sans()))
                            .color(t.ink2),
                    );
                }
            });

        // Deleting is the one thing here that cannot be undone, so it asks.
        let named = entries
            .iter()
            .find(|e| e.name == app.bench.save_name)
            .cloned();
        if let Some(entry) = named {
            let confirm_key = egui::Id::new("bench_delete_armed");
            let armed: bool = ui.ctx().data(|d| d.get_temp(confirm_key).unwrap_or(false));
            let label = if armed { "Delete — sure?" } else { "Delete" };
            if ui.button(label).clicked() {
                if armed {
                    match crate::store_strategies::delete(&entry.path) {
                        Ok(()) => {
                            app.bench.library_note = Some(format!("Deleted \"{}\"", entry.name));
                            app.bench.saved_source = None;
                            app.bench.refresh_library();
                        }
                        Err(e) => app.bench.library_note = Some(e),
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(confirm_key, false));
                } else {
                    ui.ctx().data_mut(|d| d.insert_temp(confirm_key, true));
                }
            }
        }

        if app.bench.dirty() {
            // Not amber: amber is reserved for statements about whether a
            // number can be trusted (STALE, PARTIAL). Unsaved work is a
            // fact about the file, not about the arithmetic.
            ui.label(
                RichText::new("• unsaved")
                    .font(FontId::new(type_scale::CAPTION, theme::sans()))
                    .color(t.ink),
            );
        }
    });
    if let Some(note) = &app.bench.library_note {
        ui.label(
            RichText::new(note)
                .font(FontId::new(type_scale::CAPTION, theme::sans()))
                .color(t.ink2),
        );
    }
}

fn source_box(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    egui::Frame::NONE
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.hairline))
        .corner_radius(6)
        .inner_margin(8.0)
        .show(ui, |ui| {
            let edit = egui::TextEdit::multiline(&mut app.bench.source)
                .font(FontId::new(type_scale::CAPTION, theme::mono()))
                .desired_rows(8)
                .desired_width(f32::INFINITY)
                .hint_text("strategy \"…\" language 1");
            ui.add(edit);
        });
}

fn controls(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    ui.horizontal(|ui| {
        if ui
            .button("Take the current player")
            .on_hover_text("Write the bet rail's own configuration out as rules")
            .clicked()
        {
            let s = from_selection(&app.cfg.sel, &app.cfg.rules());
            app.bench.source = render(&s);
            app.bench.build();
        }
        if ui.button("Compile").clicked() {
            app.bench.build();
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new("seed")
                .font(FontId::new(type_scale::CAPTION, theme::sans()))
                .color(t.ink2),
        );
        ui.add(
            egui::TextEdit::singleline(&mut app.bench.seed_text)
                .desired_width(70.0)
                .font(FontId::new(type_scale::CAPTION, theme::mono())),
        );
        let ready = app.bench.program.is_some();
        if ui
            .add_enabled(ready, egui::Button::new("Run one session"))
            .clicked()
        {
            run(app);
        }
    });
    ui.horizontal(|ui| {
        let ready = app.bench.program.is_some();
        ui.add_enabled(
            ready,
            egui::Checkbox::new(&mut app.use_strategy, "Play this instead of the bet rail"),
        )
        .on_hover_text("Space, and every Findings number, will come from this strategy");
        if app.use_strategy && !ready {
            // Selected but not compiled: say so here rather than let Run
            // fail with a message far from the control that caused it.
            ui.label(
                RichText::new("— compile it first")
                    .font(FontId::new(type_scale::CAPTION, theme::sans()))
                    .color(t.ink),
            );
        }
    });
}

fn run(app: &mut App) {
    let Some(p) = app.bench.program.clone() else {
        return;
    };
    let min = app.cfg.table_mins_cents.first().copied().unwrap_or(1000);
    let trace = bench_session(
        &p,
        &app.cfg.rules(),
        min,
        app.cfg.budget_cents,
        app.cfg.quit_target_cents(),
        app.cfg.max_rolls,
        app.cfg.horizon_rolls(),
        app.bench.seed(),
    );
    // Open on the first roll: the table before the first throw is a
    // legitimate position to step back to, but not one to start from.
    app.bench.position = 1.min(trace.rolls.len());
    app.bench.trace = Some(trace);
}

fn status(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    if let Some(e) = &app.bench.error {
        ui.label(
            RichText::new(e)
                .font(FontId::new(type_scale::BODY, theme::sans()))
                .color(t.ink),
        );
        return;
    }
    let Some(p) = &app.bench.program else {
        return;
    };
    let rules = p.rule_count();
    let reads = if p.features.is_empty() {
        "reads nothing beyond the layout".to_owned()
    } else {
        format!("reads {:?}", p.features)
    };
    ui.label(
        RichText::new(format!(
            "{rules} rules · {} instructions per decision · {reads}",
            p.cost_bound()
        ))
        .font(FontId::new(type_scale::CAPTION, theme::mono()))
        .color(t.ink2),
    );
}

fn transport(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    let total = app.bench.trace.as_ref().map_or(0, |x| x.rolls.len());
    ui.horizontal(|ui| {
        if ui.button("◀").on_hover_text("previous roll").clicked() {
            app.bench.step(-1);
        }
        if ui.button("▶").on_hover_text("next roll").clicked() {
            app.bench.step(1);
        }
        let mut pos = app.bench.position;
        ui.spacing_mut().slider_width = 260.0;
        ui.add(egui::Slider::new(&mut pos, 0..=total).show_value(false));
        if pos != app.bench.position {
            app.bench.position = pos;
        }
        ui.label(
            RichText::new(if app.bench.position == 0 {
                "before the first roll".to_owned()
            } else {
                format!("roll {} of {total}", app.bench.position)
            })
            .font(FontId::new(type_scale::CAPTION, theme::mono()))
            .color(t.ink2),
        );
    });
}

fn rules_column(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    let (Some(strategy), Some(trace)) = (&app.bench.parsed, &app.bench.trace) else {
        return;
    };
    let fired: Vec<u16> = if app.bench.position == 0 {
        Vec::new()
    } else {
        trace
            .rolls
            .get(app.bench.position - 1)
            .map(|r| r.fired.clone())
            .unwrap_or_default()
    };
    ui.label(
        RichText::new("Rules")
            .font(FontId::new(type_scale::CAPTION, theme::sans()))
            .color(t.ink2),
    );
    ui.add_space(4.0);
    for i in 0..strategy.rules.len() {
        let count = trace.fire_counts.get(i).copied().unwrap_or(0);
        let lit = fired.contains(&(i as u16));
        // A rule that never fires in the whole session is the first thing
        // to look at, so it is marked amber — the register this app
        // reserves for "trust this less than you were about to".
        let ink = if lit {
            t.blue
        } else if count == 0 {
            t.amber
        } else {
            t.ink
        };
        ui.horizontal(|ui| {
            // The ledger attributes money to "rule 3"; without the number
            // here that is a reference to nothing.
            ui.label(
                RichText::new(format!("{i:>2}"))
                    .font(FontId::new(type_scale::CAPTION, theme::mono()))
                    .color(t.ink2),
            );
            ui.label(
                RichText::new(format!("{count:>4}×"))
                    .font(FontId::new(type_scale::CAPTION, theme::mono()))
                    .color(if count == 0 { t.amber } else { t.ink2 }),
            );
            ui.label(
                RichText::new(render_rule(strategy, i))
                    .font(FontId::new(type_scale::CAPTION, theme::mono()))
                    .color(ink),
            );
        });
    }
    let dead = trace.never_fired();
    if !dead.is_empty() {
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "{} rule{} never fired on this seed.",
                dead.len(),
                if dead.len() == 1 { "" } else { "s" }
            ))
            .font(FontId::new(type_scale::CAPTION, theme::sans()))
            .color(t.amber),
        );
    }
}

fn ledger_column(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    let Some(trace) = &app.bench.trace else {
        return;
    };
    ui.label(
        RichText::new("What happened")
            .font(FontId::new(type_scale::CAPTION, theme::sans()))
            .color(t.ink2),
    );
    ui.add_space(4.0);
    if app.bench.position == 0 {
        ui.label(
            RichText::new("The table before the first throw.")
                .font(FontId::new(type_scale::CAPTION, theme::sans()))
                .color(t.ink2),
        );
        return;
    }
    let Some(roll) = trace.rolls.get(app.bench.position - 1) else {
        return;
    };
    let budget = app.cfg.budget_cents;
    ui.label(
        RichText::new(format!(
            "{}+{} = {}   ·   {}",
            roll.dice.0,
            roll.dice.1,
            roll.dice.0 + roll.dice.1,
            match roll.point_after {
                Some(p) => format!("point {p}"),
                None => "come-out".to_owned(),
            }
        ))
        .font(FontId::new(type_scale::BODY, theme::mono()))
        .color(t.ink),
    );
    let profit = roll.wealth_after - budget;
    ui.label(
        RichText::new(format!(
            "cash {} · wealth {} · {}{}",
            numerals::money_text(roll.cash_after, false),
            numerals::money_text(roll.wealth_after, false),
            if profit >= 0 { "up " } else { "down " },
            numerals::money_text(profit.abs(), false)
        ))
        .font(FontId::new(type_scale::CAPTION, theme::mono()))
        .color(if profit >= 0 { t.gain } else { t.ruin }),
    );
    ui.add_space(4.0);
    if roll.events.is_empty() {
        ui.label(
            RichText::new("Nothing moved.")
                .font(FontId::new(type_scale::CAPTION, theme::sans()))
                .color(t.ink2),
        );
    }
    for e in &roll.events {
        let (words, ink) = describe(&t, e);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(match e.rule {
                    Some(i) => format!("rule {i}"),
                    None => "table".to_owned(),
                })
                .font(FontId::new(type_scale::CAPTION, theme::mono()))
                .color(t.ink2),
            );
            ui.label(
                RichText::new(words)
                    .font(FontId::new(type_scale::CAPTION, theme::mono()))
                    .color(ink),
            );
        });
    }
}

/// One event, in words, with the ink its meaning has earned.
fn describe(
    t: &crate::ui::theme::Theme,
    e: &craps_engine::strategy::BenchEvent,
) -> (String, egui::Color32) {
    use craps_engine::trace::BetEventKind as K;
    let bet = e.event.bet.label();
    let amount = numerals::money_text(e.event.stake_cents, false);
    match e.event.kind {
        K::Placed => (format!("{bet} — placed {amount}"), t.ink),
        K::Won { paid_cents, .. } => (
            format!("{bet} — wins {}", numerals::money_text(paid_cents, false)),
            t.gain,
        ),
        K::Lost => (format!("{bet} — loses {amount}"), t.ruin),
        K::Pushed => (format!("{bet} — pushes"), t.ink2),
        K::Returned => (format!("{bet} — {amount} returned, unresolved"), t.ink2),
        K::TakenDown => (format!("{bet} — {amount} taken down"), t.ink),
        K::Traveled { to } => (format!("{bet} — travels to {to}"), t.ink2),
        K::Rejected { reason } => (format!("{bet} — refused: {}", reason.label()), t.amber),
        K::ClippedToMax => (
            format!("{bet} — clipped to the table max, {amount}"),
            t.amber,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SimConfig;

    fn from_source(source: String) -> BenchState {
        let mut b = BenchState {
            source,
            ..Default::default()
        };
        b.build();
        b
    }

    fn taken_from(cfg: &SimConfig) -> BenchState {
        from_source(render(&from_selection(&cfg.sel, &cfg.rules())))
    }

    #[test]
    fn the_current_player_compiles_straight_off_the_bet_rail() {
        let mut cfg = SimConfig::default();
        cfg.sel.take_odds = true;
        cfg.sel.set_place(6, true);
        let b = taken_from(&cfg);
        assert!(b.error.is_none(), "{:?}", b.error);
        assert!(b.program.is_some());
        assert!(
            b.source
                .starts_with("strategy \"checkbox player\" language 1"),
            "{}",
            b.source
        );
    }

    #[test]
    fn a_broken_paste_says_where_and_what() {
        let b = from_source("strategy \"x\" language 1\non roll:\n    bet wobble base\n".into());
        let e = b.error.expect("should refuse");
        assert!(e.contains("wobble"), "{e}");
        assert!(e.contains("line 3"), "{e}");
        assert!(b.program.is_none());
    }

    #[test]
    fn an_empty_bench_asks_for_something_to_run() {
        let b = from_source(String::new());
        assert!(b.error.unwrap().contains("Nothing to run"));
    }

    #[test]
    fn a_strategy_that_never_bets_is_refused_in_words() {
        // Legal to write, and nothing at risk.
        let b = from_source("strategy \"idle\" language 1\non roll:\n    down place 6\n".into());
        assert!(
            b.error.as_ref().unwrap().contains("never places a bet"),
            "{:?}",
            b.error
        );
    }

    #[test]
    fn stepping_stays_inside_the_session() {
        let cfg = SimConfig::default();
        let mut b = taken_from(&cfg);
        let p = b.program.clone().unwrap();
        b.trace = Some(bench_session(
            &p,
            &cfg.rules(),
            1000,
            cfg.budget_cents,
            None,
            cfg.max_rolls,
            200,
            1,
        ));
        let total = b.trace.as_ref().unwrap().rolls.len();
        assert!(total > 0);
        b.position = 0;
        b.step(-1);
        assert_eq!(b.position, 0, "cannot step before the table existed");
        b.position = total;
        b.step(1);
        assert_eq!(b.position, total, "cannot step past the last roll");
        b.step(-1);
        assert_eq!(b.position, total - 1);
    }

    #[test]
    fn unsaved_work_is_visible_and_saving_settles_it() {
        let dir = std::env::temp_dir().join("craps-bench-dirty-test");
        let _ = std::fs::remove_dir_all(&dir);
        let src = "strategy \"Mine\" language 1\non come-out:\n    bet pass\n";

        let mut b = from_source(src.into());
        assert!(b.dirty(), "typed but never saved");

        // Saving through the store settles it; editing dirties it again.
        crate::store_strategies::save(&dir, "Mine", &b.source).unwrap();
        b.saved_source = Some(b.source.clone());
        assert!(!b.dirty());
        b.source.push_str("\non seven-out:\n    leave\n");
        assert!(b.dirty(), "edited after saving");

        // Reading it back settles it and restores the name.
        let entry = crate::store_strategies::list(&dir)
            .into_iter()
            .next()
            .unwrap();
        b.load_from(&entry);
        assert!(!b.dirty());
        assert_eq!(b.save_name, "Mine");
        assert_eq!(b.source, src);
        assert!(b.program.is_some(), "loading compiles what it read");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_editor_is_not_unsaved_work() {
        let b = BenchState::default();
        assert!(!b.dirty(), "nothing typed is nothing to lose");
    }

    /// Editing the source retires the ledger. A trace from a strategy that
    /// no longer exists is exactly the stale truth this app refuses
    /// everywhere else.
    #[test]
    fn rebuilding_clears_a_stale_ledger() {
        let cfg = SimConfig::default();
        let mut b = taken_from(&cfg);
        let p = b.program.clone().unwrap();
        b.trace = Some(bench_session(
            &p,
            &cfg.rules(),
            1000,
            cfg.budget_cents,
            None,
            cfg.max_rolls,
            200,
            1,
        ));
        b.position = 5;
        b.build();
        assert!(b.trace.is_none());
        assert_eq!(b.position, 0);
    }
}
