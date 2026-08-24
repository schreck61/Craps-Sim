// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The wealth fan: P5–P95 and P25–P75 bands (triangle-strip meshes — CI
//! bands are concave wherever the curve bends), the median path, and a few
//! faint individual trajectories from a deterministic traced subsample.
//! Sampling is disclosed on the frame, always.

use egui::{Pos2, Stroke};

use super::axis;
use super::frame::{ChartCx, Layer};
use super::marks;

pub use crate::store::FanData;

pub fn paint(cx: &mut ChartCx<'_>, fan: &FanData, budget_cents: i64, hours_per_roll: f64) {
    let t = cx.theme;
    let rolls = fan.bands.len().max(1);
    cx.set_x_domain(0.0, rolls as f64);
    let y_max = fan
        .bands
        .iter()
        .map(|b| b[4])
        .max()
        .unwrap_or(budget_cents)
        .max(budget_cents) as f64
        * 1.05;
    cx.set_y_domain(0.0, y_max);
    let inner_bottom = cx.baseline();

    axis::x_axis(cx, |v| format!("{:.0}", v));
    axis::x_axis_twin_top(cx, move |v| format!("{:.1} h", v * hours_per_roll));
    axis::y_axis(cx, |v| crate::ui::numerals::money_text(v as i64, false));

    let top = cx.y.r1;
    let clampy = move |y: f32| y.clamp(top, inner_bottom);
    let band = |cx: &ChartCx<'_>, lo: usize, hi: usize| -> (Vec<Pos2>, Vec<Pos2>) {
        let step = (rolls / cx.rect.width().max(1.0) as usize).max(1);
        let mut upper = Vec::with_capacity(rolls / step + 1);
        let mut lower = Vec::with_capacity(rolls / step + 1);
        for (k, b) in fan.bands.iter().enumerate().step_by(step) {
            let x = cx.x.to_screen(k as f64);
            upper.push(Pos2::new(x, clampy(cx.y.to_screen(b[hi] as f64))));
            lower.push(Pos2::new(x, clampy(cx.y.to_screen(b[lo] as f64))));
        }
        (upper, lower)
    };

    let (u, l) = band(cx, 0, 4);
    cx.mesh(Layer::Ribbon, marks::band_mesh(&u, &l, t.ci_band(t.ink2)));
    let (u, l) = band(cx, 1, 3);
    cx.mesh(Layer::Ribbon, marks::band_mesh(&u, &l, t.ci_band(t.ink)));

    // Faint individual trajectories: real sessions, not decoration.
    for path in fan.paths.iter().take(12) {
        let step = (path.len() / cx.rect.width().max(1.0) as usize).max(1);
        let pts: Vec<Pos2> = path
            .iter()
            .enumerate()
            .step_by(step)
            .map(|(k, &w)| Pos2::new(cx.x.to_screen(k as f64), clampy(cx.y.to_screen(w as f64))))
            .collect();
        cx.line(Layer::Data, pts, Stroke::new(0.8, t.dimmed(t.ink2)));
    }

    // Median path.
    let step = (rolls / cx.rect.width().max(1.0) as usize).max(1);
    let med: Vec<Pos2> = fan
        .bands
        .iter()
        .enumerate()
        .step_by(step)
        .map(|(k, b)| {
            Pos2::new(
                cx.x.to_screen(k as f64),
                clampy(cx.y.to_screen(b[2] as f64)),
            )
        })
        .collect();
    cx.line(Layer::Estimate, med, Stroke::new(1.8, t.ink));

    // The budget line, etched.
    let by = cx.y.to_screen(budget_cents as f64);
    cx.hline(Layer::Grid, by, Stroke::new(1.0, t.hairline_strong));
}
