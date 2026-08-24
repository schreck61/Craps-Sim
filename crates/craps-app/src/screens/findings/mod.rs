// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Findings — the four core questions as three sub-tabs sharing one run:
//! Endurance (how long the money lasts), Horizon (where the bankroll ends
//! up), and Stake (how big a bankroll you need).

pub mod anchor;
pub mod charts;
pub mod endurance;
pub mod horizon;
pub mod stake;

use egui::{FontId, RichText};

use crate::app::{App, FindingsTab};
use crate::chart::Badge;
use crate::store::{RunStatus, RunStore};
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    ui.horizontal(|ui| {
        super::title(ui, &t, "Findings");
        ui.add_space(16.0);
        for (tab, label, key) in [
            (FindingsTab::Endurance, "Endurance", "Q"),
            (FindingsTab::Horizon, "Horizon", "W"),
            (FindingsTab::Stake, "Stake", "E"),
        ] {
            let selected = app.findings_tab == tab;
            let text = RichText::new(label)
                .font(FontId::new(
                    type_scale::BODY,
                    if selected {
                        theme::sans_semibold()
                    } else {
                        theme::sans()
                    },
                ))
                .color(if selected { t.ink } else { t.ink2 });
            let resp = ui
                .add(egui::Button::new(text).fill(if selected {
                    t.surface2
                } else {
                    egui::Color32::TRANSPARENT
                }))
                .on_hover_text(format!("key {key}"));
            if resp.clicked() {
                app.findings_tab = tab;
            }
        }
    });
    ui.add_space(8.0);

    if app.main_run.is_none() {
        let nights = app.cfg.sessions as u64 * app.cfg.table_mins_cents.len().max(1) as u64;
        let nights_text = if nights >= 1_000_000 {
            format!("{:.1} million", nights as f64 / 1e6)
        } else {
            format!("{}", nights / 1000 * 1000)
        };
        let copy =
            format!("Describe a player, then press Space — {nights_text} nights, a few seconds.");
        super::ghost_panel_with_house_line(&mut *ui, &t, app, &copy);
        ui.vertical_centered(|ui| {
            if ui
                .button(
                    RichText::new("Run the example")
                        .font(FontId::new(type_scale::BODY, theme::sans_semibold())),
                )
                .on_hover_text("$500, $10 table, pass line flat, 4 hours")
                .clicked()
            {
                app.cfg = crate::config::SimConfig::default();
                app.cfg.budget_cents = 50_000;
                app.cfg.table_mins_cents = vec![1000];
                app.table_mins_text = "10".into();
                app.focused_min = 0;
                app.start_run();
            }
        });
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| match app.findings_tab {
        FindingsTab::Endurance => endurance::show(app, ui),
        FindingsTab::Horizon => horizon::show(app, ui),
        FindingsTab::Stake => stake::show(app, ui),
    });
}

/// The trust badge for the current run state: STALE when settings diverged,
/// PARTIAL after a cancel.
pub fn run_badge(app: &App, store: &RunStore) -> Option<Badge> {
    if store.provenance.fingerprint != app.cfg.fingerprint() {
        return Some(Badge::Stale);
    }
    match store.status {
        RunStatus::Cancelled { n } => {
            Some(Badge::Partial(format!("n = {}", numerals::compact_n(n))))
        }
        _ => None,
    }
}

/// The provenance corner every chart carries.
pub fn provenance_line(store: &RunStore) -> String {
    let p = &store.provenance;
    let n = match store.status {
        RunStatus::Streaming { n } => n,
        RunStatus::Cancelled { n } => n,
        RunStatus::Complete => p.sessions * store.mins.len() as u64,
    };
    let sampled = match store.retention {
        crate::store::RetentionMode::Sampled { stride } => {
            format!(" · raw columns sampled 1/{stride} (digests see every session)")
        }
        crate::store::RetentionMode::Full => String::new(),
    };
    format!(
        "n = {} · seed {} · fp {:08X} · engine v{}{sampled}",
        numerals::compact_n(n),
        numerals::seed_hex(p.seed),
        (p.fingerprint >> 32) as u32,
        p.engine_version
    )
}

/// Cancelled-run banner (partial results persist, honestly recomputed)
/// under a diagonal-hatched margin stripe (spec §9).
pub fn partial_banner(app: &App, ui: &mut egui::Ui, store: &RunStore) {
    if let RunStatus::Cancelled { n } = store.status {
        let t = &app.theme;
        let total = store.provenance.sessions * store.mins.len() as u64;
        let (stripe, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), egui::Sense::hover());
        let p = ui.painter_at(stripe);
        let mut x = stripe.left() - stripe.height();
        while x < stripe.right() {
            p.line_segment(
                [
                    egui::pos2(x, stripe.bottom()),
                    egui::pos2(x + stripe.height(), stripe.top()),
                ],
                egui::Stroke::new(1.0, t.amber),
            );
            x += 6.0;
        }
        ui.label(
            RichText::new(format!(
                "Partial run — {} of {} sessions. Intervals below are recomputed at this n; treat rankings as provisional.",
                numerals::compact_n(n),
                numerals::compact_n(total)
            ))
            .font(FontId::new(type_scale::BODY, theme::sans_medium()))
            .color(t.amber),
        );
        ui.add_space(6.0);
    }
}
