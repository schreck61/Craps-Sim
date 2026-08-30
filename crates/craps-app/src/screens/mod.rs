// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The four modes and their screens.

pub mod bench;
pub mod design;
pub mod duel;
pub mod explorer;
pub mod findings;
pub mod replay;

use egui::{FontId, RichText};

use crate::ui::theme::{self, type_scale, Theme};

/// First-run ghosted state: dimmed axes and one line of copy — no blank
/// panels, no modal tour.
pub fn ghost_panel(ui: &mut egui::Ui, t: &Theme, copy: &str) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 260.0),
        egui::Sense::hover(),
    );
    let p = ui.painter_at(rect);
    let axes = egui::Color32::from_rgba_unmultiplied(t.ink2.r(), t.ink2.g(), t.ink2.b(), 38);
    let inner = rect.shrink(24.0);
    p.line_segment(
        [inner.left_top(), inner.left_bottom()],
        egui::Stroke::new(1.0, axes),
    );
    p.line_segment(
        [inner.left_bottom(), inner.right_bottom()],
        egui::Stroke::new(1.0, axes),
    );
    for k in 1..5 {
        let y = inner.top() + inner.height() * k as f32 / 5.0;
        p.line_segment(
            [egui::pos2(inner.left(), y), egui::pos2(inner.right(), y)],
            egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(t.ink2.r(), t.ink2.g(), t.ink2.b(), 18),
            ),
        );
    }
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        copy,
        FontId::new(type_scale::STORY, theme::sans_medium()),
        t.ink2,
    );
}

/// The Findings ghost: dimmed axes PLUS the pre-drawn amber closed-form
/// expectation — the Truth drawn before a single die rolls (Gasp 1's
/// staging, spec §9). The amber line appears only where a closed form
/// exists (flat stakes, no quit target — the §10.2 scope guard).
pub fn ghost_panel_with_house_line(
    ui: &mut egui::Ui,
    t: &Theme,
    app: &crate::app::App,
    copy: &str,
) {
    use craps_engine::{flat_drift_per_roll_cents, Progression};
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 260.0),
        egui::Sense::hover(),
    );
    let p = ui.painter_at(rect);
    let axes = egui::Color32::from_rgba_unmultiplied(t.ink2.r(), t.ink2.g(), t.ink2.b(), 38);
    let inner = rect.shrink(24.0);
    p.line_segment(
        [inner.left_top(), inner.left_bottom()],
        egui::Stroke::new(1.0, axes),
    );
    p.line_segment(
        [inner.left_bottom(), inner.right_bottom()],
        egui::Stroke::new(1.0, axes),
    );
    for k in 1..5 {
        let y = inner.top() + inner.height() * k as f32 / 5.0;
        p.line_segment(
            [egui::pos2(inner.left(), y), egui::pos2(inner.right(), y)],
            egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(t.ink2.r(), t.ink2.g(), t.ink2.b(), 18),
            ),
        );
    }
    let cfg = &app.cfg;
    if cfg.sel.progression == Progression::Flat && cfg.quit_mult.is_none() && cfg.sel.any_selected()
    {
        let min = cfg
            .table_mins_cents
            .get(app.focused_min)
            .copied()
            .unwrap_or(1000);
        let drift = flat_drift_per_roll_cents(&cfg.sel, &cfg.rules(), min);
        let expected = cfg.budget_cents as f64 + drift * cfg.horizon_rolls() as f64;
        // The ghost axis spans 0..2× budget, like the live histogram will.
        let frac = (expected / (cfg.budget_cents as f64 * 2.0).max(1.0)).clamp(0.02, 0.98);
        let x = inner.left() + inner.width() * frac as f32;
        p.line_segment(
            [egui::pos2(x, inner.top()), egui::pos2(x, inner.bottom())],
            egui::Stroke::new(1.2, t.amber),
        );
        p.text(
            egui::pos2(x + 4.0, inner.top() + 2.0),
            egui::Align2::LEFT_TOP,
            format!(
                "expected mean — closed form: {}",
                crate::ui::numerals::money_text(expected as i64, false)
            ),
            FontId::new(type_scale::CAPTION, theme::mono()),
            t.amber,
        );
    }
    // On a ground pill: the house line's x resolves near center for any
    // realistic configuration and would slice through the sentence.
    let galley = p.layout_no_wrap(
        copy.to_owned(),
        FontId::new(type_scale::STORY, theme::sans_medium()),
        t.ink2,
    );
    let pos = egui::Align2::CENTER_CENTER
        .anchor_size(
            egui::pos2(rect.center().x, rect.center().y + 40.0),
            galley.size(),
        )
        .min;
    p.rect_filled(
        egui::Rect::from_min_size(pos, galley.size()).expand2(egui::vec2(6.0, 3.0)),
        4.0,
        t.pill(),
    );
    p.galley(pos, galley, t.ink2);
}

/// Screen title in the display face.
pub fn title(ui: &mut egui::Ui, t: &Theme, text: &str) {
    ui.label(
        RichText::new(text)
            .font(FontId::new(type_scale::TITLE, theme::display()))
            .color(t.ink),
    );
    ui.add_space(6.0);
}
