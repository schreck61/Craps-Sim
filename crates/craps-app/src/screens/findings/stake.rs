// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Stake — how big a bankroll do you need? The peak-outlay survival curve
//! with the draggable budget line and confidence handle, the dot field of
//! a thousand fates, and the recommended-budget hero.

use egui::{FontId, RichText};

use crate::app::App;
use crate::components::insight_card::InsightCard;
use crate::components::ledger;
use crate::components::stat_hero::StatHero;
use crate::sentence;
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    let Some(run) = &app.main_run else { return };
    let store = run.store.clone();
    let st = store.lock().unwrap();
    super::partial_banner(app, ui, &st);

    let focused = app.focused_min.min(st.mins.len().saturating_sub(1));
    let scenario = sentence::render_text(&st.provenance.config);
    let p = st.provenance.config.clone();

    // Hero: the recommended budget at the handle's confidence.
    let confidence = app.stake_confidence.unwrap_or(p.confidence);
    if let Some(m) = st.mins.get(focused) {
        if let Some(s) = &m.summary {
            // Exact recompute at the live handle from sorted outlays where
            // available; the summary's figure at the run's confidence
            // otherwise. Instant response IS the exactness.
            let rec = m
                .sorted
                .as_ref()
                .map(|sv| {
                    let idx =
                        ((confidence / 100.0) * (sv.outlays.len() as f64 - 1.0)).ceil() as usize;
                    let raw = sv.outlays[idx.min(sv.outlays.len() - 1)];
                    ((raw + 99) / 100) * 100
                })
                .unwrap_or(s.budget_cents);
            let chart_id = egui::Id::new("stake_survival");
            StatHero::money(
                rec,
                format!("survives {:.0} h at {confidence:.0}%", p.target_hours),
            )
            .ci(format!(
                "with this budget, {:.0} of 100 nights like yours never need more",
                confidence
            ))
            .requires(chart_id)
            .show(ui, &t, &app.mounted);
            ui.label(
                RichText::new(
                    "Mildly conservative: covers the worst moment of each night at full strategy.",
                )
                .font(FontId::new(type_scale::CAPTION, theme::sans()))
                .color(t.ink2),
            );
            if p.quit_mult.is_some() {
                ui.label(
                    RichText::new(
                        "Computed without the quit-while-ahead rule: quitting early can only lower the chance of busting, so this guarantee still holds (slightly conservatively).",
                    )
                    .font(FontId::new(type_scale::CAPTION, theme::sans()))
                    .color(t.ink2),
                );
            }
            ui.add_space(8.0);

            // Insight card: the inverse reading at the ACTUAL budget.
            if let Some(sv) = &m.sorted {
                let covers = sv.outlays.partition_point(|&o| o <= p.budget_cents) as f64
                    / sv.outlays.len().max(1) as f64;
                let action = InsightCard {
                    group: egui::Id::new("stake_inverse"),
                    lead: format!(
                        "Your actual {} covers {} of 100 nights.",
                        numerals::money_text(p.budget_cents, false),
                        (covers * 100.0).round() as u32
                    ),
                    exact: format!(
                        "{} ± {:.1}",
                        numerals::prob(covers, s.n),
                        craps_engine::wald_ci_half(covers, s.n) * 100.0,
                    ),
                    show_one: None,
                    scenario: &scenario,
                }
                .show(ui, &t, &mut app.highlights);
                let _ = action;
                ui.add_space(8.0);
            }
        }
    }

    // M: a small-multiple grid of the peak-outlay curves, one per minimum,
    // above the full interactive focused chart (spec §6.5).
    if app.small_multiples && st.mins.len() > 1 {
        super::charts::stake_small_multiples(app, ui, &st, focused);
    }

    // The hero chart (returns a clicked session for Enter one ▸).
    if let Some(session) = super::charts::stake_chart(app, ui, &st, focused) {
        let min_ix = focused;
        drop(st);
        app.open_replay(min_ix, session);
        return;
    }

    // The tail-price caption: what the last points of certainty cost.
    if let Some(m) = st.mins.get(focused) {
        if let Some(sv) = &m.sorted {
            let at = |c: f64| -> i64 {
                let idx = ((c / 100.0) * (sv.outlays.len() as f64 - 1.0)).ceil() as usize;
                sv.outlays[idx.min(sv.outlays.len() - 1)]
            };
            let (b95, b99) = (at(95.0), at(99.0));
            ui.label(
                RichText::new(format!(
                    "The last 4 points of certainty cost {}.",
                    numerals::money_text(b99 - b95, false)
                ))
                .font(FontId::new(type_scale::BODY, theme::sans_medium()))
                .color(t.ink),
            );
        }
    }

    ui.add_space(12.0);
    if app.prefs.ledger_register || app.ledger_drawer {
        ui.label(
            RichText::new("Ledger")
                .font(FontId::new(type_scale::SECTION, theme::sans_semibold()))
                .color(t.ink),
        );
        ledger::stake_table(app, ui, &st);
    }
}
