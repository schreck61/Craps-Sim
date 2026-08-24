// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Horizon — where does the bankroll end up? The final-bankroll histogram
//! with the sacred bust bar and the pre-drawn House Line, the decile
//! ladder, the wealth fan, and the two-register story.

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

    let focused = app.focused_min.min(st.mins.len().saturating_sub(1));
    let scenario = sentence::render_text(&st.provenance.config);

    // Story register: the lead sentence, integers-of-100 with the exact
    // figure footnoted.
    if !app.prefs.ledger_register {
        if let Some(m) = st.mins.get(focused) {
            if let Some(s) = &m.summary {
                let budget = st.provenance.config.budget_cents;
                let behind = m
                    .sorted
                    .as_ref()
                    .map(|sv| {
                        1.0 - crate::store::SortedViews::frac_at_least(&sv.finals, budget + 1)
                    })
                    .unwrap_or_else(|| {
                        // Deciles bracket it when sorted views are elsewhere.
                        s.horizon
                            .deciles
                            .iter()
                            .position(|&d| d > budget)
                            .map(|i| (i + 1) as f64 / 10.0)
                            .unwrap_or(0.9)
                    });
                let lead = format!(
                    "Out of 100 nights like yours, {} go home with less than they brought. {} lose everything. {} come out ahead.",
                    (behind * 100.0).round() as u32,
                    (s.horizon.bust_frac * 100.0).round() as u32,
                    ((1.0 - behind) * 100.0).round() as u32,
                );
                let exact = format!(
                    "{:.1}% behind ± {:.1} · bust {:.2}% · n = {}",
                    behind * 100.0,
                    craps_engine::wald_ci_half(behind, s.n) * 100.0,
                    s.horizon.bust_frac * 100.0,
                    numerals::compact_n(s.n)
                );
                let median_id = m.sorted.as_ref().and_then(|sv| {
                    sv.finals_rank_to_id
                        .get(sv.finals_rank_to_id.len() / 2)
                        .map(|&slot| st.true_session(slot))
                });
                let action = InsightCard {
                    group: egui::Id::new("horizon_lead"),
                    lead,
                    exact,
                    show_one: median_id,
                    scenario: &scenario,
                }
                .show(ui, &t, &mut app.highlights);
                if let CardAction::ShowOne(id) = action {
                    let min_ix = focused;
                    drop(st);
                    app.open_replay(min_ix, id);
                    return;
                }
                ui.add_space(8.0);
            }
        }
    }

    if st.status == crate::store::RunStatus::Complete {
        let cfg = st.provenance.config.clone();
        let seed = st.provenance.seed;
        super::anchor::idle_prefetch(app, &cfg, seed, focused);
    }
    // Wayfinding for the other tables: the hero shows one minimum at a
    // time; say which, and offer every table in one click.
    if st.mins.len() > 1 {
        ui.horizontal(|ui| {
            let focused_txt = numerals::money_text(st.mins[focused].min_cents, false);
            ui.label(
                RichText::new(if app.small_multiples {
                    format!("every table · {focused_txt} keeps the anchor row",)
                } else {
                    format!("showing the {focused_txt} table — switch via the chips in the rail")
                })
                .font(FontId::new(11.0, theme::sans()))
                .color(t.ink2),
            );
            let toggle = ui.add(
                egui::Label::new(
                    RichText::new(if app.small_multiples {
                        "focus one table (M)"
                    } else {
                        "show every table (M)"
                    })
                    .font(FontId::new(11.0, theme::sans_medium()))
                    .color(t.blue),
                )
                .sense(egui::Sense::click()),
            );
            if toggle.clicked() {
                app.small_multiples = !app.small_multiples;
            }
        });
        ui.add_space(4.0);
    }

    let hover_bin = super::charts::horizon_chart(app, ui, &st, focused);
    // The hovered bar is a door: its count, and one real night inside it.
    let mut enter_session: Option<u64> = None;
    if let Some(hb) = hover_bin {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "bar {}–{} · {} nights",
                    numerals::money_text(hb.lo_cents, false),
                    numerals::money_text(hb.hi_cents, false),
                    numerals::compact_n(hb.count as u64)
                ))
                .font(FontId::new(11.0, theme::mono()))
                .color(t.ink2),
            );
            if let Some(session) = hb.session {
                let link = ui.add(
                    egui::Label::new(
                        RichText::new("Enter one ▶")
                            .font(FontId::new(11.0, theme::sans_medium()))
                            .color(t.blue),
                    )
                    .sense(egui::Sense::click()),
                );
                if link.clicked() {
                    enter_session = Some(session);
                }
            }
        });
    } else {
        // Reserve the line so the layout doesn't jump while probing.
        ui.add_space(18.0);
    }
    if let Some(session) = enter_session {
        let min_ix = focused;
        drop(st);
        app.open_replay(min_ix, session);
        return;
    }
    ui.add_space(10.0);
    super::charts::fan_chart_ui(app, ui, &st, focused);

    ui.add_space(12.0);
    if app.prefs.ledger_register || app.ledger_drawer {
        ui.label(
            RichText::new("Ledger")
                .font(FontId::new(type_scale::SECTION, theme::sans_semibold()))
                .color(t.ink),
        );
        ledger::horizon_table(app, ui, &st);
    }
}
