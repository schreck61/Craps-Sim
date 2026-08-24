// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Endurance — how long does the money last? Survival curves per table
//! minimum with censoring made visible; the Ledger register keeps the exact
//! percentile table one keystroke away.

use egui::{FontId, RichText};

use crate::app::App;
use crate::components::insight_card::{CardAction, InsightCard};
use crate::components::ledger;
use crate::sentence;
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    let Some(run) = &app.main_run else { return };
    let store = run.store.clone();
    let st = store.lock().unwrap();
    super::partial_banner(app, ui, &st);

    let rph = st.provenance.config.rolls_per_hour as f64;
    let focused = app.focused_min.min(st.mins.len().saturating_sub(1));

    // Story register: three insight cards, each a door into a real night.
    let scenario = sentence::render_text(&st.provenance.config);
    let mut enter: Option<u64> = None;
    if !app.prefs.ledger_register {
        if let Some(m) = st.mins.get(focused) {
            if let (Some(s), Some(sv)) = (&m.summary, &m.sorted) {
                let n = sv.rolls_rank_to_id.len();
                let pick = |rank: usize| {
                    (n > 0).then(|| st.true_session(sv.rolls_rank_to_id[rank.min(n - 1)]))
                };
                let min_txt = numerals::money_text(m.min_cents, false);
                let cards = [
                    (
                        egui::Id::new("endurance_typical"),
                        format!(
                            "A typical night at the {} table lasts {} (about {} rolls).",
                            min_txt,
                            numerals::hours(s.ruin.p50 as f64, rph),
                            s.ruin.p50
                        ),
                        format!(
                            "median {} rolls · mean {} ± {} · n = {}",
                            s.ruin.p50,
                            s.ruin.mean.round(),
                            s.ruin.ci95_half.round(),
                            numerals::compact_n(s.n)
                        ),
                        pick(n / 2),
                    ),
                    (
                        egui::Id::new("endurance_p5"),
                        format!(
                            "5 of 100 nights are over inside {}.",
                            numerals::hours(s.ruin.p5 as f64, rph)
                        ),
                        format!(
                            "P5 = {} rolls · n = {}",
                            s.ruin.p5,
                            numerals::compact_n(s.n)
                        ),
                        pick(n / 20),
                    ),
                    (
                        egui::Id::new("endurance_p95"),
                        format!(
                            "5 of 100 nights outlast {}.",
                            numerals::hours(s.ruin.p95 as f64, rph)
                        ),
                        format!(
                            "P95 = {} rolls · n = {}",
                            s.ruin.p95,
                            numerals::compact_n(s.n)
                        ),
                        pick(n - 1 - n / 20),
                    ),
                ];
                ui.columns(3, |cols| {
                    for (col, (group, lead, exact, show_one)) in cols.iter_mut().zip(cards) {
                        let action = InsightCard {
                            group,
                            lead,
                            exact,
                            show_one,
                            scenario: &scenario,
                        }
                        .show(col, &t, &mut app.highlights);
                        if let CardAction::ShowOne(id) = action {
                            enter = Some(id);
                        }
                    }
                });
                ui.add_space(8.0);
            } else if let Some(s) = &m.summary {
                // Sorted views still building: the sentence without doors.
                ui.label(
                    RichText::new(format!(
                        "A typical night at the {} table lasts {} (about {} rolls).",
                        numerals::money_text(m.min_cents, false),
                        numerals::hours(s.ruin.p50 as f64, rph),
                        s.ruin.p50
                    ))
                    .font(FontId::new(type_scale::STORY, theme::sans_medium()))
                    .color(t.ink),
                );
                ui.add_space(8.0);
            }
        }
    }
    if let Some(session) = enter {
        drop(st);
        app.open_replay(focused, session);
        return;
    }

    // Hero: the survival curve (all minimums overlaid, focused at full ink).
    super::charts::survival_chart(app, ui, &st, focused);

    ui.add_space(12.0);
    ledger::endurance_table(app, ui, &st, rph);

    if st
        .mins
        .iter()
        .filter_map(|m| m.summary.as_ref())
        .any(|s| s.ruin.censored_frac > 0.05)
    {
        ui.label(
            RichText::new(
                "Capped sessions bias mean and upper percentiles low; they are counted, shown, and never dropped.",
            )
            .font(FontId::new(type_scale::CAPTION, theme::sans()))
            .color(t.ink2),
        );
    }
}
