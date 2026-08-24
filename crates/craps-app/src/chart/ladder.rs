// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The decile ladder: ten ticks P10–P90 on a shared dollar axis, the amber
//! mean diamond, and a solid median tick — skew legible in one glance.
//! Used under every money histogram; the paired variant joins two ladders
//! rung by rung so WHERE two strategies diverge is structural.

use egui::{Align2, FontId, Pos2, Stroke};

use super::frame::{ChartCx, Layer};
use super::marks;
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale};

pub struct Ladder<'a> {
    pub deciles: &'a [i64; 9],
    pub mean: f64,
    pub budget_cents: i64,
}

pub fn paint(cx: &mut ChartCx<'_>, ladder: &Ladder<'_>) {
    let t = cx.theme;
    let lo = 0.0f64;
    let hi = (ladder.deciles[8]
        .max(ladder.mean as i64)
        .max(ladder.budget_cents) as f64)
        * 1.08;
    cx.set_x_domain(lo, hi);
    let y = cx.rect.center().y;

    // Axis line.
    cx.hline(Layer::Grid, y, Stroke::new(1.0, t.hairline_strong));
    // Budget marker.
    let bx = cx.x.to_screen(ladder.budget_cents as f64);
    cx.line(
        Layer::Grid,
        vec![Pos2::new(bx, y - 12.0), Pos2::new(bx, y + 12.0)],
        Stroke::new(1.0, t.hairline_strong),
    );
    cx.text_pilled(
        Layer::Annotation,
        Pos2::new(bx, y + 14.0),
        Align2::CENTER_TOP,
        "1×",
        FontId::new(type_scale::CAPTION, theme::mono()),
        t.ink2,
        t.pill(),
    );

    for (k, &d) in ladder.deciles.iter().enumerate() {
        let x = cx.x.to_screen(d as f64);
        let is_median = k == 4;
        cx.line(
            Layer::Data,
            vec![Pos2::new(x, y - 8.0), Pos2::new(x, y + 8.0)],
            Stroke::new(if is_median { 2.2 } else { 1.2 }, t.ink),
        );
        if k == 0 || is_median || k == 8 {
            cx.text_pilled(
                Layer::Annotation,
                Pos2::new(x, y - 10.0),
                Align2::CENTER_BOTTOM,
                format!("P{} {}", (k + 1) * 10, numerals::money_text(d, false)),
                FontId::new(type_scale::CAPTION, theme::mono()),
                t.ink2,
                t.pill(),
            );
        }
    }
    marks::mean_diamond(cx, Pos2::new(cx.x.to_screen(ladder.mean), y), 5.0);
    // A row lower when the mean sits near 1× budget (the common case),
    // so "mean" and "1×" never print on top of each other.
    let mean_x = cx.x.to_screen(ladder.mean);
    let mean_y = if (mean_x - bx).abs() < 40.0 {
        y + 28.0
    } else {
        y + 14.0
    };
    cx.text_pilled(
        Layer::Annotation,
        Pos2::new(mean_x, mean_y),
        Align2::CENTER_TOP,
        "mean",
        FontId::new(type_scale::CAPTION, theme::mono()),
        t.amber,
        t.pill(),
    );
}

/// Two vertical decile strips joined rung by rung (the Duel).
pub struct PairedLadder<'a> {
    pub a: &'a [i64; 9],
    pub b: &'a [i64; 9],
    pub a_color: egui::Color32,
    pub b_color: egui::Color32,
    pub a_label: &'a str,
    pub b_label: &'a str,
}

pub fn paint_paired(cx: &mut ChartCx<'_>, p: &PairedLadder<'_>) {
    let t = cx.theme;
    let hi = p.a.iter().chain(p.b.iter()).copied().max().unwrap_or(1) as f64 * 1.08;
    cx.set_y_domain(0.0, hi);
    let xa = cx.rect.left() + cx.rect.width() * 0.30;
    let xb = cx.rect.left() + cx.rect.width() * 0.70;

    for (x, label, color) in [(xa, p.a_label, p.a_color), (xb, p.b_label, p.b_color)] {
        cx.line(
            Layer::Grid,
            vec![
                Pos2::new(x, cx.rect.top() + 14.0),
                Pos2::new(x, cx.rect.bottom() - 6.0),
            ],
            Stroke::new(1.0, t.hairline_strong),
        );
        cx.text(
            Layer::Annotation,
            Pos2::new(x, cx.rect.top()),
            Align2::CENTER_TOP,
            label,
            FontId::new(type_scale::CAPTION, theme::sans_semibold()),
            color,
        );
    }
    let mut last_label_y = f32::MIN;
    for k in 0..9 {
        let ya = cx.y.to_screen(p.a[k] as f64);
        let yb = cx.y.to_screen(p.b[k] as f64);
        let w = if k == 4 { 2.2 } else { 1.2 };
        cx.line(
            Layer::Data,
            vec![Pos2::new(xa - 8.0, ya), Pos2::new(xa + 8.0, ya)],
            Stroke::new(w, p.a_color),
        );
        cx.line(
            Layer::Data,
            vec![Pos2::new(xb - 8.0, yb), Pos2::new(xb + 8.0, yb)],
            Stroke::new(w, p.b_color),
        );
        // The connecting rung: where along the distribution they diverge.
        cx.line(
            Layer::Ribbon,
            vec![Pos2::new(xa + 8.0, ya), Pos2::new(xb - 8.0, yb)],
            Stroke::new(1.0, t.hairline_strong),
        );
        // Tight distributions bunch the rungs; a label prints only when
        // it clears the previous one (the median always prints).
        if k == 4 || (ya - last_label_y).abs() >= 11.0 {
            last_label_y = ya;
            cx.text_pilled(
                Layer::Annotation,
                Pos2::new(xa - 12.0, ya),
                Align2::RIGHT_CENTER,
                format!("P{}", (k + 1) * 10),
                FontId::new(type_scale::CAPTION, theme::mono()),
                t.ink2,
                t.pill(),
            );
        }
    }
}
