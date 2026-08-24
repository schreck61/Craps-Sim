// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Survival/ECDF step curves: rolls-until-ruin (Endurance) and
//! budget-vs-peak-outlay (Stake). One step-curve per series, line-end
//! labeled — no detached legends; censoring rendered as a hatched block,
//! never silently truncated.

use egui::{Align2, FontId, Pos2, Stroke};

use super::axis;
use super::frame::{ChartCx, Layer};
use super::marks;
use crate::ui::theme::{self, type_scale};

pub struct SurvivalSeries {
    pub color: egui::Color32,
    pub label: String,
    /// (x, survival fraction 0..=1), ascending x, step-after semantics.
    pub points: Vec<(f64, f64)>,
    pub focused: bool,
}

pub struct SurvivalOptions {
    pub x_max: f64,
    /// Format the x axis as money (Stake) instead of counts (Endurance).
    pub money_x: bool,
    /// Droplines for the focused series: (x, label).
    pub droplines: Vec<(f64, String)>,
    /// Hatched censoring block: (x at cap, surviving fraction, label).
    pub censored: Option<(f64, f64, String)>,
    /// Twin top axis: hours per x unit (rolls → hours), if present.
    pub hours_per_x: Option<f64>,
    /// Probe readout lines builder output (one per series at probe x).
    pub probe_labels: Vec<String>,
}

pub fn paint(cx: &mut ChartCx<'_>, series: &[SurvivalSeries], opt: &SurvivalOptions) {
    let t = cx.theme;
    cx.set_x_domain(0.0, opt.x_max.max(1.0));
    cx.set_y_domain(0.0, 1.0);
    let inner_bottom = cx.baseline();

    if opt.money_x {
        axis::x_axis(cx, |v| crate::ui::numerals::money_text(v as i64, false));
    } else {
        axis::x_axis(cx, |v| crate::ui::numerals::compact_n(v.max(0.0) as u64));
    }
    axis::y_axis(cx, |v| format!("{:.0}%", v * 100.0));
    if let Some(hpx) = opt.hours_per_x {
        axis::x_axis_twin_top(cx, move |v| {
            let h = v * hpx;
            if h >= 10.0 {
                format!("{h:.0} h")
            } else {
                format!("{h:.1} h")
            }
        });
    }

    for s in series {
        let stroke = Stroke::new(
            if s.focused { 1.8 } else { 1.2 },
            if s.focused {
                s.color
            } else {
                t.dimmed(s.color)
            },
        );
        let mut pts: Vec<Pos2> = Vec::with_capacity(s.points.len() * 2);
        let mut last_y = None;
        for &(x, sv) in &s.points {
            let px = cx.x.to_screen(x);
            let py = cx.y.to_screen(sv).clamp(cx.rect.top(), inner_bottom);
            if let Some(ly) = last_y {
                pts.push(Pos2::new(px, ly));
            }
            pts.push(Pos2::new(px, py));
            last_y = Some(py);
        }
        let pts = marks::decimate(&pts, cx.rect.width());
        cx.line(Layer::Data, pts, stroke);
        // Line-end label.
        if let Some(&(x, sv)) = s.points.last() {
            cx.text(
                Layer::Annotation,
                Pos2::new(
                    cx.x.to_screen(x).min(cx.rect.right() - 4.0),
                    cx.y.to_screen(sv)
                        .clamp(cx.rect.top() + 8.0, inner_bottom - 8.0),
                ),
                Align2::RIGHT_BOTTOM,
                &s.label,
                FontId::new(type_scale::CAPTION, theme::mono()),
                if s.focused {
                    s.color
                } else {
                    t.dimmed(s.color)
                },
            );
        }
    }

    // Dropline labels climb to a free row when neighbors crowd them:
    // percentile lines bunch near zero on long-tail axes, so each label
    // takes the lowest row whose previous occupant ends left of it.
    let label_font = FontId::new(type_scale::CAPTION, theme::mono());
    let mut row_right: Vec<f32> = Vec::new();
    for (x, label) in &opt.droplines {
        let sx = cx.x.to_screen(*x);
        cx.dashed_vline(Layer::Annotation, sx, t.ink2);
        let w = cx.text_width(label, label_font.clone());
        let row = row_right
            .iter()
            .position(|&r| sx + 2.0 > r)
            .unwrap_or(row_right.len());
        if row == row_right.len() {
            row_right.push(0.0);
        }
        row_right[row] = sx + 2.0 + w + 8.0;
        cx.text_pilled(
            Layer::Annotation,
            Pos2::new(sx + 2.0, inner_bottom - 4.0 - 13.0 * row as f32),
            Align2::LEFT_BOTTOM,
            label,
            label_font.clone(),
            t.ink2,
            t.pill(),
        );
    }

    if let Some((cap_x, frac, label)) = opt.censored.as_ref().filter(|(_, f, _)| *f > 0.001) {
        let x1 = cx.x.to_screen(*cap_x).min(cx.rect.right());
        let x0 = (x1 - 26.0).max(cx.rect.left());
        let y0 = cx.y.to_screen(*frac);
        let rect = egui::Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, inner_bottom));
        marks::hatch(cx, Layer::Grid, rect, t.hairline_strong);
        cx.text(
            Layer::Annotation,
            Pos2::new(x1 - 2.0, y0 - 4.0),
            Align2::RIGHT_BOTTOM,
            label,
            FontId::new(type_scale::CAPTION, theme::mono()),
            t.ink2,
        );
    }

    // Probe readout: every curve at once.
    if let Some(px) = cx.probe() {
        let lines = opt.probe_labels.clone();
        let anchor = Pos2::new(
            cx.x.to_screen(px).min(cx.rect.right() - 150.0) + 8.0,
            cx.rect.top() + 8.0,
        );
        for (k, line) in lines.iter().enumerate() {
            cx.text(
                Layer::Overlay,
                anchor + egui::vec2(0.0, k as f32 * 14.0),
                Align2::LEFT_TOP,
                line,
                FontId::new(type_scale::CAPTION, theme::mono()),
                t.ink,
            );
        }
    }
}
