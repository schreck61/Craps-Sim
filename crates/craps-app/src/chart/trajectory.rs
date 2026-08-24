// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Session trajectories: one bankroll's step line inside the whole
//! population's percentile envelope — the one always seen inside the many,
//! so a lucky replay reads as the outlier it is, never a promise. Winning
//! and ruinous sessions render with identical choreography; there is no
//! celebration state in any code path.

use egui::{Align2, FontId, Pos2, Stroke};

use super::axis;
use super::frame::{ChartCx, Layer};
use super::marks;
use crate::store::FanData;
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale};

pub struct Trajectory<'a> {
    /// Wealth after each roll, cents.
    pub wealth: &'a [i64],
    pub color: egui::Color32,
    pub label: String,
    /// Peak outlay of this session, for the shaded depth band.
    pub peak_outlay_cents: i64,
    /// How the session ended: tick color and label at the end point.
    pub end: Option<(egui::Color32, String)>,
}

pub struct TrajectoryOptions<'a> {
    /// The population envelope, always drawn behind (P10–P90 at 12% alpha
    /// uses the fan's P5–P95/P25–P75; close enough bands, disclosed).
    pub envelope: Option<&'a FanData>,
    pub budget_cents: i64,
    pub horizon_rolls: usize,
    /// Playhead: rolls currently revealed (scrub clip).
    pub revealed: usize,
    pub hours_per_roll: f64,
    /// The amber analytic drift line, only for closed-form configs.
    pub drift_line: Option<(f64, f64)>, // (start cents, per-roll drift)
    /// Label for the envelope center when no closed form exists.
    pub simulated_mean_label: Option<String>,
}

pub fn paint(cx: &mut ChartCx<'_>, tracks: &[Trajectory<'_>], o: &TrajectoryOptions<'_>) {
    let t = cx.theme;
    let rolls = o
        .horizon_rolls
        .max(tracks.iter().map(|tr| tr.wealth.len()).max().unwrap_or(1));
    cx.set_x_domain(0.0, rolls as f64);
    let y_max = tracks
        .iter()
        .flat_map(|tr| tr.wealth.iter().copied())
        .chain(o.envelope.iter().flat_map(|f| f.bands.iter().map(|b| b[4])))
        .max()
        .unwrap_or(o.budget_cents)
        .max(o.budget_cents) as f64
        * 1.05;
    cx.set_y_domain(0.0, y_max);
    let inner_bottom = cx.baseline();
    let top = cx.y.r1;
    let clampy = move |y: f32| y.clamp(top, inner_bottom);

    axis::x_axis(cx, |v| format!("{v:.0}"));
    axis::x_axis_twin_top(cx, move |v| format!("{:.1} h", v * o.hours_per_roll));
    axis::y_axis(cx, |v| numerals::money_text(v as i64, false));

    // The many behind the one.
    if let Some(env) = o.envelope {
        let step = (env.bands.len() / cx.rect.width().max(1.0) as usize).max(1);
        let band = |lo: usize, hi: usize| {
            let mut u = Vec::new();
            let mut l = Vec::new();
            for (k, b) in env.bands.iter().enumerate().step_by(step) {
                let x = cx.x.to_screen(k as f64);
                u.push(Pos2::new(x, clampy(cx.y.to_screen(b[hi] as f64))));
                l.push(Pos2::new(x, clampy(cx.y.to_screen(b[lo] as f64))));
            }
            (u, l)
        };
        let (u, l) = band(0, 4);
        cx.mesh(Layer::Ribbon, marks::band_mesh(&u, &l, t.ci_band(t.ink2)));
        cx.text(
            Layer::Ribbon,
            Pos2::new(cx.rect.right() - 8.0, cx.rect.top() + 16.0),
            Align2::RIGHT_TOP,
            format!(
                "population envelope · sampled n = {} of {}",
                numerals::compact_n(env.sampled_n),
                numerals::compact_n(env.total_n)
            ),
            FontId::new(type_scale::CAPTION, theme::mono()),
            t.ink2,
        );
    }

    // The amber drift line — only where a closed form exists.
    if let Some((start, drift)) = o.drift_line {
        let p0 = Pos2::new(cx.x.to_screen(0.0), clampy(cx.y.to_screen(start)));
        let p1 = Pos2::new(
            cx.x.to_screen(rolls as f64),
            clampy(cx.y.to_screen(start + drift * rolls as f64)),
        );
        cx.line(Layer::Estimate, vec![p0, p1], Stroke::new(1.2, t.amber));
        cx.text(
            Layer::Estimate,
            p1 + egui::vec2(-4.0, -20.0),
            Align2::RIGHT_BOTTOM,
            "expected drift — closed form",
            FontId::new(type_scale::CAPTION, theme::mono()),
            t.amber,
        );
    } else if let (Some(env), Some(label)) = (o.envelope, &o.simulated_mean_label) {
        // The envelope's center path in secondary ink, labeled honestly.
        let step = (env.bands.len() / cx.rect.width().max(1.0) as usize).max(1);
        let med: Vec<Pos2> = env
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
        cx.line(Layer::Estimate, med, Stroke::new(1.0, t.ink2));
        cx.text(
            Layer::Estimate,
            Pos2::new(cx.rect.right() - 4.0, cx.rect.top() + 18.0),
            Align2::RIGHT_TOP,
            label,
            FontId::new(type_scale::CAPTION, theme::mono()),
            t.ink2,
        );
    }

    // Budget line, etched.
    let by = clampy(cx.y.to_screen(o.budget_cents as f64));
    cx.hline(Layer::Grid, by, Stroke::new(1.0, t.hairline_strong));

    for tr in tracks {
        let revealed = o.revealed.min(tr.wealth.len());
        if revealed == 0 {
            continue;
        }
        // Peak-outlay depth band: budget down to the session's lowest cash
        // commitment, a shaded floor under the line.
        let floor = (o.budget_cents - tr.peak_outlay_cents).max(0);
        let fy = clampy(cx.y.to_screen(floor as f64));
        if fy > by {
            let faint =
                egui::Color32::from_rgba_unmultiplied(tr.color.r(), tr.color.g(), tr.color.b(), 22);
            cx.rect_filled(
                Layer::Ribbon,
                egui::Rect::from_min_max(
                    Pos2::new(cx.rect.left() + axis::MARGIN_LEFT, by),
                    Pos2::new(cx.rect.right() - 8.0, fy),
                ),
                faint,
            );
        }

        let mut pts = Vec::with_capacity(revealed * 2 + 1);
        let mut last = clampy(cx.y.to_screen(o.budget_cents as f64));
        pts.push(Pos2::new(cx.x.to_screen(0.0), last));
        for (k, &w) in tr.wealth[..revealed].iter().enumerate() {
            let x = cx.x.to_screen((k + 1) as f64);
            pts.push(Pos2::new(x, last));
            let y = clampy(cx.y.to_screen(w as f64));
            pts.push(Pos2::new(x, y));
            last = y;
        }
        let pts = marks::decimate(&pts, cx.rect.width());
        cx.line(Layer::Data, pts, Stroke::new(1.6, tr.color));

        // End tick — identical choreography for wins and ruins.
        if revealed == tr.wealth.len() {
            if let Some((color, label)) = &tr.end {
                let x = cx.x.to_screen(revealed as f64);
                cx.line(
                    Layer::Annotation,
                    vec![Pos2::new(x, last - 10.0), Pos2::new(x, last + 10.0)],
                    Stroke::new(2.0, *color),
                );
                cx.text(
                    Layer::Annotation,
                    Pos2::new(x - 4.0, last - 12.0),
                    Align2::RIGHT_BOTTOM,
                    label,
                    FontId::new(type_scale::CAPTION, theme::mono()),
                    *color,
                );
            }
        }
        // Series label at line end.
        cx.text(
            Layer::Annotation,
            Pos2::new(
                cx.x.to_screen(revealed as f64).min(cx.rect.right() - 4.0),
                (last - 14.0).max(cx.rect.top() + 2.0),
            ),
            Align2::RIGHT_BOTTOM,
            &tr.label,
            FontId::new(type_scale::CAPTION, theme::mono()),
            tr.color,
        );
    }
}
