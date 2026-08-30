// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Authoring a strategy, and reading one back.
//!
//! This module holds two halves that live on two screens, because a design
//! review found them wanting different things:
//!
//! - [`authoring`] is on **Design**, behind the `Checkboxes | Rules`
//!   control, because Design is where the player is built and choosing to
//!   play a strategy is the same act as choosing to write one.
//! - [`ledger`] is on **Replay**, because Replay already owns a transport,
//!   a dice strip, a keyboard, and the population envelope. `STRATEGY_DSL`
//!   §8 always said the Bench should reuse them; the first version
//!   reimplemented a worse transport instead.
//!
//! What the ledger is for: a rule set that quietly does nothing and returns
//! a confident distribution is the worst thing the strategy surface can
//! produce. Rules that fired are marked. Rules that never fired carry a
//! zero. Every refusal says why, in words, and the session's refusals are
//! listed together so one on roll 91 is not something you have to find.

use craps_engine::strategy::{
    compile, from_selection, parse, render, render_rule, BenchTrace, Program, Strategy,
};
use egui::{FontId, RichText, Stroke};

use crate::app::App;
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale};

/// What became of a strategy a sentence referred to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Resolution {
    /// Same name, same rules. The sentence describes this machine's run.
    Found,
    /// Same name, different rules. Never silently run: the sentence's
    /// numbers came from a strategy this machine no longer has.
    Changed { got: u32 },
    /// This machine has no strategy by that name.
    Missing,
    /// The strategy is here, but the editor is holding work that is not on
    /// disk. Opening over it would destroy it, so the author decides.
    Unsaved,
}

/// What a library entry holds, or nothing if it cannot be read right now.
fn entry_source(entry: &crate::store_strategies::SavedStrategy) -> String {
    crate::store_strategies::load(&entry.path).unwrap_or_default()
}

#[derive(Default)]
pub struct BenchState {
    /// The strategy under test, as text. Starts empty; the button fills it
    /// from whatever the bet rail is currently describing, which is also
    /// the fastest way to see what the language looks like.
    pub source: String,
    pub parsed: Option<Strategy>,
    /// Behind an `Arc` because the run path, the rail and the export all ask
    /// for it every frame, and a `Program` is a `Vec<Op>`, a name and
    /// seventeen progressions — copying that sixty times a second to answer
    /// "which player is live" is work nobody asked for.
    pub program: Option<std::sync::Arc<Program>>,
    /// Why the source could not be read or compiled, in the words the
    /// author needs — never a red outline alone.
    pub error: Option<String>,
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
}

impl BenchState {
    /// Read and compile the source, keeping whichever of the two failed as
    /// the message.
    pub fn build(&mut self) {
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
                    self.program = Some(std::sync::Arc::new(p));
                }
            },
        }
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

    /// Resolve a strategy a pasted sentence referred to.
    ///
    /// §10, and the reason the sentence carries a hash at all: a machine
    /// with a strategy of that name but different rules must say so rather
    /// than run the wrong player, and a machine without one must say that
    /// rather than fall back to the bet rail.
    pub fn resolve(&mut self, want: &crate::config::StrategyRef) -> Resolution {
        self.refresh_library();
        // The reference carries the name the strategy declares; the library
        // is keyed by file stems, which the store sanitizes. Matching the two
        // raw meant "44 Inside, regressed" saved itself under a stem its own
        // sentence could never find again — the spec's own flagship example,
        // permanently Missing on the machine that wrote it.
        let sanitized = crate::store_strategies::sanitize(&want.name);
        let Some(entry) = self
            .library
            .iter()
            .find(|e| e.name == want.name || e.name == sanitized)
            .cloned()
        else {
            return Resolution::Missing;
        };
        // Opening over unsaved work is how an author loses it. `dirty()`
        // exists for exactly this and was not being asked.
        if self.dirty() && self.source != entry_source(&entry) {
            self.library_note = Some(format!(
                "\"{}\" is in the library, but this editor has unsaved changes — save or clear them first.",
                entry.name
            ));
            return Resolution::Unsaved;
        }
        self.load_from(&entry);
        match &self.program {
            None => Resolution::Missing,
            Some(p) => {
                let got = crate::config::StrategyRef::of(p);
                if got.hash == want.hash {
                    Resolution::Found
                } else {
                    Resolution::Changed { got: got.hash }
                }
            }
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
                // A file on disk can be anything — hand-edited, half-saved,
                // written by an older grammar. Opening it must say what is
                // wrong with it rather than leave the editor holding text
                // the app will not run and no reason why.
                if let Some(e) = &self.error {
                    self.library_note = Some(format!("\"{}\" did not compile — {e}", entry.name));
                }
            }
            Err(e) => self.library_note = Some(e),
        }
    }
}

/// The authoring half, on the Design screen.
pub fn authoring(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    ui.label(
        RichText::new(
            "Write a strategy. It plays instead of the bet rail, and Replay \
             steps through a night of it rule by rule.",
        )
        .font(FontId::new(type_scale::BODY, theme::sans()))
        .color(t.ink2),
    );
    ui.add_space(6.0);
    library_row(app, ui);
    ui.add_space(6.0);
    controls(app, ui);
    ui.add_space(6.0);
    status(app, ui);
    ui.add_space(10.0);

    // The rows are the first editor; the text is the second. Both edit the
    // same tree, which is what keeps them from being two languages.
    super::rules::show(app, ui);

    ui.add_space(10.0);
    egui::CollapsingHeader::new(
        RichText::new("As text")
            .font(FontId::new(type_scale::BODY, theme::sans()))
            .color(t.ink2),
    )
    .id_salt("bench_as_text")
    .default_open(false)
    .show(ui, |ui| {
        source_box(app, ui);
        ui.label(
            RichText::new(
                "Paste a strategy here, or edit anything the rows above \
                 leave as text. Press Compile when you are done.",
            )
            .font(FontId::new(type_scale::CAPTION, theme::sans()))
            .color(t.ink2),
        );
    });
}

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
    ui.horizontal(|ui| {
        if ui
            .button("Take the current player")
            .on_hover_text("Write the bet rail's own configuration out as rules")
            .clicked()
        {
            let s = from_selection(&app.cfg.sel, &app.cfg.rules());
            app.bench.source = render(&s);
            app.bench.save_name.clear();
            app.bench.saved_source = None;
            app.bench.build();
        }
        if ui.button("Compile").clicked() {
            app.bench.build();
        }
        if ui
            .button("Examples…")
            .on_hover_text("Strategies from the specification, ready to edit")
            .clicked()
        {
            let id = egui::Id::new("bench_examples");
            let open: bool = ui.ctx().data(|d| d.get_temp(id).unwrap_or(false));
            ui.ctx().data_mut(|d| d.insert_temp(id, !open));
        }
    });
    examples_menu(app, ui);
}

/// The worked examples, shipped rather than left in the test suite.
///
/// These are demonstrations of syntax, not recommendations of play — the
/// app names no best strategy and these are mostly bad bets, one of them
/// deliberately superstitious. But a language with no examples and no
/// reference is a language nobody can start writing in.
fn examples_menu(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    let id = egui::Id::new("bench_examples");
    if !ui.ctx().data(|d| d.get_temp::<bool>(id).unwrap_or(false)) {
        return;
    }
    egui::Frame::NONE
        .fill(t.surface2)
        .stroke(Stroke::new(1.0, t.hairline_strong))
        .corner_radius(6)
        .inner_margin(8.0)
        .show(ui, |ui| {
            for (name, src) in craps_engine::strategy::EXAMPLES {
                if ui.selectable_label(false, *name).clicked() {
                    app.bench.source = (*src).to_owned();
                    app.bench.save_name.clear();
                    app.bench.saved_source = None;
                    app.bench.build();
                    ui.ctx().data_mut(|d| d.insert_temp(id, false));
                }
            }
            ui.label(
                RichText::new("Examples of the language, not advice.")
                    .font(FontId::new(type_scale::CAPTION, theme::sans()))
                    .color(t.ink2),
            );
        });
}

fn status(app: &App, ui: &mut egui::Ui) {
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
    // Instructions per decision belonged in an engine disclosure, not in
    // front of someone who opened this on a Saturday; and `{:?}` of a
    // feature mask is a debug format leaking onto their screen.
    let rules = p.rule_count();
    let reads = read_summary(p);
    ui.label(
        RichText::new(format!(
            "{rules} rule{} · {reads}",
            if rules == 1 { "" } else { "s" }
        ))
        .font(FontId::new(type_scale::BODY, theme::sans()))
        .color(t.ink2),
    );
}

/// The ledger half, on the Replay screen. `position` is Replay's playhead.
pub fn ledger(app: &mut App, ui: &mut egui::Ui, position: usize) {
    // An `Arc` clone rather than the whole night: this runs every frame the
    // Replay screen is showing a strategy, and a `BenchTrace` owns two
    // vectors per roll.
    let Some(trace) = app.replay.bench.clone() else {
        return;
    };
    ui.add_space(8.0);
    run_conditions(app, ui, &trace);
    ui.add_space(6.0);
    refusal_index(app, ui, &trace, position);
    ui.add_space(6.0);
    ui.columns(2, |cols| {
        rules_column(app, &mut cols[0], &trace, position);
        events_column(app, &mut cols[1], &trace, position);
    });
}

/// What this night was played under. Refusals like "below the table
/// minimum" are decided entirely by numbers the panel used to leave unsaid.
fn run_conditions(app: &App, ui: &mut egui::Ui, trace: &BenchTrace) {
    let t = app.theme.clone();
    let cfg = app.replay.config.clone().unwrap_or_else(|| app.cfg.clone());
    let min = app.replay.min_cents;
    // The reference the sentence carries, spelled the way the sentence
    // spells it. This printed four hex digits of a hash the rest of the app
    // prints eight of, so a reader checking a benched night against its own
    // sentence had to know one was a truncation of the other.
    let name = app
        .bench
        .program
        .as_ref()
        .map(|p| {
            let r = crate::config::StrategyRef::of(p);
            format!("{} #{}", r.name, r.short())
        })
        .unwrap_or_else(|| "strategy".into());
    ui.label(
        RichText::new(format!(
            "{name} · {} table · budget {} · {} · ended: {}",
            numerals::money_text(min, false),
            numerals::money_text(cfg.budget_cents, false),
            match cfg.quit_mult {
                Some(m) => format!("quit at {}×", numerals::multiple(m)),
                None => "no quit rule".to_owned(),
            },
            trace.ending(),
        ))
        .font(FontId::new(type_scale::CAPTION, theme::mono()))
        .color(t.ink2),
    );
}

/// Every refusal in the session, together, each one a way to get to it.
///
/// Without this a refusal on roll 91 of 137 is invisible unless you step
/// there — the panel would be telling you an answer exists without giving
/// you any way to reach it.
fn refusal_index(app: &mut App, ui: &mut egui::Ui, trace: &BenchTrace, position: usize) {
    let t = app.theme.clone();
    let refusals = trace.refusals();
    if refusals.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!(
                "{} refusal{} this night:",
                refusals.len(),
                if refusals.len() == 1 { "" } else { "s" }
            ))
            .font(FontId::new(type_scale::CAPTION, theme::sans()))
            .color(t.ink),
        );
        for (roll, e) in refusals.iter().take(12) {
            let here = *roll as usize == position;
            let label = format!("{} · {}", roll, e.event.bet.label());
            if ui
                .selectable_label(
                    here,
                    RichText::new(label).font(FontId::new(type_scale::CAPTION, theme::mono())),
                )
                .on_hover_text(match e.event.kind {
                    craps_engine::trace::BetEventKind::Rejected { reason, .. } => reason.label(),
                    _ => "",
                })
                .clicked()
            {
                app.replay.position = *roll as f64;
                app.replay.playing = false;
            }
        }
        if refusals.len() > 12 {
            ui.label(
                RichText::new(format!("and {} more", refusals.len() - 12))
                    .font(FontId::new(type_scale::CAPTION, theme::sans()))
                    .color(t.ink2),
            );
        }
    });
}

fn rules_column(app: &App, ui: &mut egui::Ui, trace: &BenchTrace, position: usize) {
    let t = app.theme.clone();
    let Some(strategy) = app.bench.parsed.as_ref() else {
        return;
    };
    let fired: Vec<u16> = if position == 0 {
        Vec::new()
    } else {
        trace
            .rolls
            .get(position - 1)
            .map(|r| r.fired.clone())
            .unwrap_or_default()
    };
    ui.label(
        RichText::new("Rules")
            .font(FontId::new(type_scale::SECTION, theme::sans_semibold()))
            .color(t.ink),
    );
    ui.add_space(4.0);
    for i in 0..strategy.rules.len() {
        let count = trace.fire_counts.get(i).copied().unwrap_or(0);
        let lit = fired.contains(&(i as u16));
        // Emphasis without colour: a rule that fired on this roll is raised
        // onto the next surface, which is this product's own elevation
        // channel. A rule that never fired is demoted to secondary ink and
        // carries a 0 — the count is the doubled non-colour channel, so the
        // reading does not depend on seeing a shade at all.
        let ink = if count == 0 { t.ink2 } else { t.ink };
        let row = egui::Frame::NONE
            .fill(if lit {
                t.surface2
            } else {
                egui::Color32::TRANSPARENT
            })
            .inner_margin(egui::Margin::symmetric(4, 1))
            .corner_radius(3);
        row.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{i:>2}"))
                        .font(FontId::new(type_scale::CAPTION, theme::mono()))
                        .color(t.ink2),
                );
                ui.label(
                    RichText::new(format!("{count:>4}×"))
                        .font(FontId::new(type_scale::CAPTION, theme::mono()))
                        .color(t.ink2),
                );
                ui.label(
                    RichText::new(render_rule(strategy, i))
                        .font(FontId::new(type_scale::BODY, theme::mono()))
                        .color(ink),
                );
            });
        });
    }
    let dead = trace.never_fired();
    if !dead.is_empty() {
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "{} rule{} never fired on this night.",
                dead.len(),
                if dead.len() == 1 { "" } else { "s" }
            ))
            .font(FontId::new(type_scale::BODY, theme::sans()))
            .color(t.ink),
        );
    }
}

fn events_column(app: &App, ui: &mut egui::Ui, trace: &BenchTrace, position: usize) {
    let t = app.theme.clone();
    ui.label(
        RichText::new("What happened")
            .font(FontId::new(type_scale::SECTION, theme::sans_semibold()))
            .color(t.ink),
    );
    ui.add_space(4.0);
    if position == 0 {
        ui.label(
            RichText::new("The table before the first throw.")
                .font(FontId::new(type_scale::BODY, theme::sans()))
                .color(t.ink2),
        );
        return;
    }
    let Some(roll) = trace.rolls.get(position - 1) else {
        return;
    };
    let budget = app
        .replay
        .config
        .as_ref()
        .map(|c| c.budget_cents)
        .unwrap_or(app.cfg.budget_cents);
    let profit = roll.wealth_after - budget;
    ui.label(
        RichText::new(format!(
            "cash {} · wealth {} · {}{}",
            numerals::money_text(roll.cash_after, false),
            numerals::money_text(roll.wealth_after, false),
            if profit >= 0 { "up " } else { "down " },
            numerals::money_text(profit.abs(), false)
        ))
        .font(FontId::new(type_scale::BODY, theme::mono()))
        .color(t.ink),
    );
    ui.add_space(4.0);
    if roll.events.is_empty() {
        ui.label(
            RichText::new("Nothing moved.")
                .font(FontId::new(type_scale::BODY, theme::sans()))
                .color(t.ink2),
        );
    }
    for e in &roll.events {
        let (words, ink) = describe(&t, e);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(match e.rule {
                    Some(i) => format!("rule {i:>2}"),
                    None => "  table".to_owned(),
                })
                .font(FontId::new(type_scale::CAPTION, theme::mono()))
                .color(t.ink2),
            );
            ui.label(
                RichText::new(words)
                    .font(FontId::new(type_scale::BODY, theme::mono()))
                    .color(ink),
            );
        });
    }
}

/// What history a strategy reads, in words rather than a debug print.
fn read_summary(p: &Program) -> String {
    use craps_engine::strategy::FeatureMask as F;
    let mut parts: Vec<&str> = Vec::new();
    if p.features.has(F::DICE) {
        parts.push("the dice so far");
    }
    if p.features.has(F::HITS) {
        parts.push("how often each number has come");
    }
    if p.features.has(F::STREAKS) {
        parts.push("wins and losses per bet");
    }
    if p.features.has(F::PEAK) {
        parts.push("the high-water mark");
    }
    if parts.is_empty() {
        "reads only the table in front of it".to_owned()
    } else {
        format!("reads {}", parts.join(", "))
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
        // The verb and the figure, because "refused: bankroll won't cover
        // it" beside a full bankroll is a zero-stake bet from a counter that
        // never got its starting value, and a refusal on a place bet could
        // otherwise have been any of four things from any of six rules.
        K::Rejected { reason, what } => (
            if e.event.stake_cents > 0 {
                format!(
                    "{bet} — {} {amount} refused: {}",
                    what.label(),
                    reason.label()
                )
            } else {
                format!("{bet} — {} refused: {}", what.label(), reason.label())
            },
            t.amber,
        ),
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

    /// Restored: this and the three below were removed by an over-broad
    /// edit when the panel split across two screens, and the deletion went
    /// unnoticed because other tests were being added at the same time.
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

    /// §10's whole reason for carrying a hash: a sentence naming a strategy
    /// this machine has under that name but with different rules must say
    /// so, and one naming a strategy this machine does not have must say
    /// that — never quietly run something else.
    #[test]
    fn a_reference_is_found_changed_or_missing_and_never_guessed() {
        let dir = crate::store_strategies::strategies_dir().unwrap();
        let name = "resolve-test-strategy";
        let src = "strategy \"Mine\" language 1\non come-out:\n    bet pass\n";
        crate::store_strategies::save(&dir, name, src).unwrap();

        let mut b = from_source(src.into());
        let here = crate::config::StrategyRef {
            name: name.to_owned(),
            hash: crate::config::StrategyRef::of(b.program.as_ref().unwrap()).hash,
        };
        assert_eq!(b.resolve(&here), Resolution::Found);

        let elsewhere = crate::config::StrategyRef {
            name: name.to_owned(),
            hash: here.hash ^ 0xFFFF,
        };
        assert!(matches!(b.resolve(&elsewhere), Resolution::Changed { .. }));

        let absent = crate::config::StrategyRef {
            name: "no-such-strategy-here".to_owned(),
            hash: 1,
        };
        assert_eq!(b.resolve(&absent), Resolution::Missing);

        let _ = crate::store_strategies::delete(&dir.join(format!("{name}.craps")));
    }

    /// A file on disk can be anything. Opening a broken one says what is
    /// wrong with it rather than leaving the editor holding text the app
    /// will not run and no reason why.
    #[test]
    fn a_broken_file_in_the_library_is_refused_in_words() {
        let dir = std::env::temp_dir().join("craps-bench-broken-test");
        let _ = std::fs::remove_dir_all(&dir);
        crate::store_strategies::save(&dir, "Broken", "strategy \"b\" language 1\nnonsense\n")
            .unwrap();
        let entry = crate::store_strategies::list(&dir)
            .into_iter()
            .next()
            .unwrap();

        let mut b = BenchState::default();
        b.load_from(&entry);
        assert!(b.program.is_none(), "it must not appear to have compiled");
        let note = b.library_note.expect("a reason");
        assert!(note.contains("Broken"), "{note}");
        assert!(note.contains("line"), "{note}");
        assert!(b.source.contains("nonsense"), "the text is there to fix");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A strategy written under a grammar this build does not know is
    /// refused, not guessed at.
    #[test]
    fn a_future_grammar_is_refused() {
        let b = from_source("strategy \"future\" language 99\non come-out:\n    bet pass\n".into());
        let e = b.error.expect("refused");
        assert!(e.contains("version 99"), "{e}");
        assert!(b.program.is_none());
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
}
