// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Ledger tables: every number of the Story register, exact, in mono,
//! decimal-aligned, CIs inline. Today's tables restyled — numerically
//! untouched, demoted never deleted.

use egui::{FontId, RichText};

use crate::app::App;
use crate::store::RunStore;
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale};

fn header(ui: &mut egui::Ui, t: &crate::ui::theme::Theme, cols: &[&str]) {
    for h in cols {
        ui.label(
            RichText::new(*h)
                .font(FontId::new(type_scale::CAPTION, theme::sans_semibold()))
                .color(t.ink2),
        );
    }
    ui.end_row();
}

fn mono_cell(ui: &mut egui::Ui, t: &crate::ui::theme::Theme, text: impl Into<String>) {
    ui.label(
        RichText::new(text.into())
            .font(FontId::new(type_scale::BODY, theme::mono()))
            .color(t.ink),
    );
}

/// Rolls-until-ruin percentile table per minimum (Q1's exact register).
pub fn endurance_table(app: &App, ui: &mut egui::Ui, st: &RunStore, rph: f64) {
    let t = app.theme.clone();
    let quit = st.provenance.config.quit_mult.is_some();
    egui::Grid::new("ruin_ledger")
        .striped(true)
        .spacing([18.0, 6.0])
        .show(ui, |ui| {
            let mut cols = vec![
                "Table min",
                "Mean rolls (95% CI)",
                "P5",
                "P25",
                "Median",
                "P75",
                "P95",
                "Median hours",
            ];
            if quit {
                cols.push("Quit ahead");
            }
            cols.push("Hit roll cap");
            header(ui, &t, &cols);
            for m in &st.mins {
                let Some(s) = &m.summary else { continue };
                mono_cell(ui, &t, numerals::money_text(m.min_cents, false));
                mono_cell(
                    ui,
                    &t,
                    format!("{:.0} ± {:.0}", s.ruin.mean, s.ruin.ci95_half),
                );
                for p in [s.ruin.p5, s.ruin.p25, s.ruin.p50, s.ruin.p75, s.ruin.p95] {
                    mono_cell(ui, &t, p.to_string());
                }
                mono_cell(ui, &t, format!("{:.1}", s.ruin.p50 as f64 / rph));
                if quit {
                    mono_cell(ui, &t, format!("{:.1}%", s.ruin.target_frac * 100.0));
                }
                mono_cell(ui, &t, format!("{:.1}%", s.ruin.censored_frac * 100.0));
                ui.end_row();
            }
        });
}

/// Ending-bankroll decile table (Q2's exact register): dollars and
/// multiples of budget in every cell.
pub fn horizon_table(app: &App, ui: &mut egui::Ui, st: &RunStore) {
    let t = app.theme.clone();
    let p = &st.provenance.config;
    let quit = p.quit_mult.is_some();
    let budget = p.budget_cents.max(1) as f64;
    egui::Grid::new("horizon_ledger")
        .striped(true)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            let mut cols = vec!["Table min", "Bust %"];
            if quit {
                cols.push("Quit ahead");
            }
            cols.push("Mean");
            let decile_names = [
                "P10", "P20", "P30", "P40", "P50", "P60", "P70", "P80", "P90",
            ];
            cols.extend(decile_names);
            header(ui, &t, &cols);
            for m in &st.mins {
                let Some(s) = &m.summary else { continue };
                mono_cell(ui, &t, numerals::money_text(m.min_cents, false));
                mono_cell(ui, &t, format!("{:.1}%", s.horizon.bust_frac * 100.0));
                if quit {
                    mono_cell(ui, &t, format!("{:.1}%", s.horizon.target_frac * 100.0));
                }
                mono_cell(
                    ui,
                    &t,
                    format!(
                        "{} ({})",
                        numerals::money_text(s.horizon.mean as i64, false),
                        numerals::multiple(s.horizon.mean / budget)
                    ),
                );
                for &d in &s.horizon.deciles {
                    mono_cell(
                        ui,
                        &t,
                        format!(
                            "{} ({})",
                            numerals::money_text(d, false),
                            numerals::multiple(d as f64 / budget)
                        ),
                    );
                }
                ui.end_row();
            }
        });
}

/// Recommended-budget table (Q3's exact register).
pub fn stake_table(app: &App, ui: &mut egui::Ui, st: &RunStore) {
    let t = app.theme.clone();
    egui::Grid::new("stake_ledger")
        .striped(true)
        .spacing([18.0, 6.0])
        .show(ui, |ui| {
            header(ui, &t, &["Table min", "Recommended budget", "× table min"]);
            for m in &st.mins {
                let Some(s) = &m.summary else { continue };
                mono_cell(ui, &t, numerals::money_text(m.min_cents, false));
                ui.label(
                    RichText::new(numerals::money_text(s.budget_cents, false))
                        .font(FontId::new(type_scale::BODY, theme::mono_medium()))
                        .color(t.ink),
                );
                mono_cell(
                    ui,
                    &t,
                    format!("{:.0}×", s.budget_cents as f64 / m.min_cents as f64),
                );
                ui.end_row();
            }
        });
}
