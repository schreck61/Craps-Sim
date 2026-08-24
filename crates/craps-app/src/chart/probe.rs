// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Probe and pin overlays: the crosshair, up to three numbered pins, and
//! the brush fill. Bodies draw their own readouts (they know their series);
//! the frame draws the shared geometry so every chart's probe looks and
//! behaves identically.

use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke};

use super::scale::LinearScale;
use crate::ui::theme::{self, type_scale, Theme};

pub struct ProbePins<'a> {
    pub pins: &'a [f64],
    pub probe: Option<f64>,
    pub brush: Option<(f64, f64)>,
    /// The chart's x-axis formatter: pins print their position in axis
    /// units, and two-plus pins print the Δ between the last two.
    pub x_fmt: Option<&'a dyn Fn(f64) -> String>,
}

impl ProbePins<'_> {
    /// The overlay geometry as shapes — the screen painter AND the export
    /// capture both consume this, so pins survive into exported PNGs
    /// (spec §7: pinned probes appear in exports).
    pub fn shapes(
        &self,
        ctx: &egui::Context,
        rect: Rect,
        x: &LinearScale,
        t: &Theme,
    ) -> Vec<egui::Shape> {
        let mut out = Vec::new();
        if let Some((a, b)) = self.brush {
            let (sa, sb) = (x.to_screen(a), x.to_screen(b));
            let (lo, hi) = if sa <= sb { (sa, sb) } else { (sb, sa) };
            out.push(egui::Shape::rect_filled(
                Rect::from_min_max(Pos2::new(lo, rect.top()), Pos2::new(hi, rect.bottom())),
                0.0,
                Color32::from_rgba_unmultiplied(t.blue.r(), t.blue.g(), t.blue.b(), 26),
            ));
            for sx in [lo, hi] {
                out.push(egui::Shape::line_segment(
                    [Pos2::new(sx, rect.top()), Pos2::new(sx, rect.bottom())],
                    Stroke::new(1.0, t.blue),
                ));
            }
        }
        for (k, &px) in self.pins.iter().enumerate() {
            let sx = x.to_screen(px);
            if !(rect.left()..=rect.right()).contains(&sx) {
                continue;
            }
            out.push(egui::Shape::line_segment(
                [Pos2::new(sx, rect.top()), Pos2::new(sx, rect.bottom())],
                Stroke::new(1.0, t.blue),
            ));
            let tag =
                Rect::from_center_size(Pos2::new(sx, rect.top() + 8.0), egui::vec2(14.0, 13.0));
            out.push(egui::Shape::rect_filled(tag, 3.0, t.blue));
            let galley = ctx.fonts_mut(|f| {
                f.layout_job(egui::text::LayoutJob::simple_singleline(
                    (k + 1).to_string(),
                    FontId::new(type_scale::CAPTION, theme::mono()),
                    t.ground,
                ))
            });
            let pos = Align2::CENTER_CENTER
                .anchor_size(tag.center(), galley.size())
                .min;
            out.push(egui::Shape::galley(pos, galley, t.ground));
        }

        // Value pills: each pin prints its position in axis units, centered
        // under its tag, climbing to a free row when neighbors crowd (pins
        // live in data space, so zoom can push them together). The NEWEST
        // pin's pill also carries the Δ from the pin before it — pins are
        // how the reader measures, without a connector line that would
        // strike through the charts' own marker labels. Positions the axis
        // formatter declines to name (the synthetic bust slot) get no pill
        // and join no Δ.
        if let Some(fmt) = self.x_fmt {
            let newest = self.pins.len().wrapping_sub(1);
            let delta = (self.pins.len() >= 2
                && !fmt(self.pins[newest]).is_empty()
                && !fmt(self.pins[newest - 1]).is_empty())
            .then(|| (self.pins[newest] - self.pins[newest - 1]).abs());
            let mut placed: Vec<Rect> = Vec::new();
            for (k, &px) in self.pins.iter().enumerate() {
                let sx = x.to_screen(px);
                if !(rect.left()..=rect.right()).contains(&sx) {
                    continue;
                }
                let mut text = fmt(px);
                if text.is_empty() {
                    continue;
                }
                if k == newest {
                    if let Some(d) = delta {
                        text = format!("{text} · Δ {}", fmt(d));
                    }
                }
                let vg = ctx.fonts_mut(|f| {
                    f.layout_job(egui::text::LayoutJob::simple_singleline(
                        text,
                        FontId::new(type_scale::CAPTION, theme::mono()),
                        t.ink,
                    ))
                });
                let half = vg.size().x / 2.0;
                let cxx = sx.clamp(rect.left() + half + 4.0, rect.right() - half - 4.0);
                for row in 0..3u32 {
                    let vp = Pos2::new(cxx - half, rect.top() + 16.0 + 17.0 * row as f32);
                    let pill = Rect::from_min_size(vp, vg.size()).expand2(egui::vec2(3.0, 1.0));
                    if placed.iter().any(|r| r.intersects(pill)) {
                        continue;
                    }
                    placed.push(pill);
                    out.push(egui::Shape::rect_filled(pill, 3.0, t.pill()));
                    out.push(egui::Shape::galley(vp, vg.clone(), t.ink));
                    break;
                }
            }
        }
        if let Some(px) = self.probe {
            let sx = x.to_screen(px);
            if (rect.left()..=rect.right()).contains(&sx) {
                out.push(egui::Shape::line_segment(
                    [Pos2::new(sx, rect.top()), Pos2::new(sx, rect.bottom())],
                    Stroke::new(
                        1.0,
                        Color32::from_rgba_unmultiplied(t.blue.r(), t.blue.g(), t.blue.b(), 140),
                    ),
                ));
            }
        }
        out
    }
}
