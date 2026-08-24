// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The final-bankroll histogram (and its ECDF flip, and the paired-diff
//! variant). Raw counts, no smoothing, ever. The bust spike is sacred:
//! sessions at exactly $0 render as a separate ruin-red bar, physically
//! gapped from the continuous mass — the most visible object on the screen.

use egui::{Align2, Color32, FontId, Mesh, Pos2, Stroke};

use super::axis;
use super::frame::{ChartCx, Layer};
use super::marks;
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale};

pub struct HistogramData<'a> {
    /// Bin counts, excluding busts (they get the separated bar).
    pub counts: &'a [u32],
    pub bin_width: i64,
    /// Bust count (sessions at exactly $0) and fraction.
    pub busts: u64,
    pub bust_frac: f64,
}

pub struct HistogramOverlays {
    /// The pre-drawn closed-form expectation (cents) and its label.
    pub house_line: Option<(f64, String)>,
    /// Caption explaining the absorbing-barrier gap, when bust mass ≥ 2%.
    pub house_gap_caption: Option<String>,
    /// Empirical mean ± CI half (cents).
    pub mean_ci: Option<(f64, f64)>,
    pub median: Option<f64>,
    /// 1×-budget hairline: (cents, "P(ahead) = …").
    pub ahead_line: Option<(f64, String)>,
    /// Budget multiple for the twin top axis.
    pub budget_cents: i64,
    pub log_y: bool,
    /// 0..=1 crossfade toward ECDF (a real transform of the same data).
    pub ecdf_alpha: f32,
    /// Brush mass label, when a brush is active.
    pub brush_label: Option<String>,
    /// Anchor crossfade: scales the bar fill (two complete honest renders
    /// crossfading — never per-bar height interpolation).
    pub fill_alpha: f32,
    /// The pinned per-dollar edge marker: never moves through any morph.
    pub pinned_edge_label: Option<String>,
    /// The drifting expected-dollar-loss pin: (x cents, label).
    pub loss_pin: Option<(f64, String)>,
}

pub fn paint(cx: &mut ChartCx<'_>, d: &HistogramData<'_>, o: &HistogramOverlays) {
    let t = cx.theme;
    let x_max = (d.counts.len() as i64 * d.bin_width) as f64;
    // The bust bar owns real x-space left of $0 — physically gapped from
    // the continuous mass, never overlapping the axis gutter. The slot is
    // at least 2% of the axis: with hundreds of narrow bins, four bin
    // widths alone would render the most important bar as a sliver.
    let bust_slot = if d.busts > 0 {
        (4.0 * d.bin_width as f64).max(x_max * 0.02)
    } else {
        0.0
    };
    cx.set_x_domain(-bust_slot, x_max.max(1.0));

    let max_count = d.counts.iter().copied().max().unwrap_or(1).max(8) as f64;
    if o.log_y {
        cx.set_y_log(max_count);
    } else {
        cx.set_y_domain(0.0, max_count * 1.05);
    }
    let inner_bottom = cx.baseline();
    let n_total: u64 = d.counts.iter().map(|&c| c as u64).sum::<u64>() + d.busts;

    axis::x_axis(cx, |v| {
        if v < 0.0 {
            String::new()
        } else {
            numerals::money_text(v as i64, false)
        }
    });
    let budget = o.budget_cents.max(1) as f64;
    // The negative region is the synthetic bust slot: no multiple lives
    // there, so its twin ticks stay blank. `.abs()` scrubs IEEE -0.0,
    // which passes the `< 0.0` guard yet prints as "-0.0".
    axis::x_axis_twin_top(cx, move |v| {
        let m = v / budget;
        if m < 0.0 {
            String::new()
        } else {
            format!("{:.1}×", m.abs())
        }
    });
    if o.log_y {
        axis::y_axis(cx, |v| numerals::compact_n(v as u64));
        cx.text_pilled(
            Layer::Annotation,
            Pos2::new(
                cx.rect.left() + axis::MARGIN_LEFT + 4.0,
                cx.rect.top() + 2.0,
            ),
            Align2::LEFT_TOP,
            "log scale — visually shrinks ruin; you opted in",
            FontId::new(type_scale::CAPTION, theme::sans()),
            t.amber,
            t.pill(),
        );
    } else {
        axis::y_axis(cx, |v| numerals::compact_n(v as u64));
    }

    let hist_alpha = (1.0 - o.ecdf_alpha) * o.fill_alpha.clamp(0.0, 1.0);
    // Bars as one mesh (the gap for the bust bar lives left of bin zero).
    if hist_alpha > 0.01 {
        let fill = with_alpha(t.ink, (200.0 * hist_alpha) as u8);
        // A hairline gap between bars reads well when bars are wide; at
        // narrow widths it becomes a comb, so it goes away.
        let bin_px = (cx.x.to_screen(d.bin_width as f64) - cx.x.to_screen(0.0)).abs();
        let gap = if bin_px >= 3.0 { 0.5 } else { 0.0 };
        let bars = d
            .counts
            .iter()
            .enumerate()
            .filter(|(_, &c)| c > 0)
            .map(|(i, &c)| {
                let x0 = cx.x.to_screen((i as i64 * d.bin_width) as f64);
                let x1 = cx.x.to_screen(((i + 1) as i64 * d.bin_width) as f64);
                let y = cx.y.to_screen(c as f64).max(cx.y.r1);
                (x0 + gap, x1 - gap, y)
            });
        let mesh = marks::bars_mesh(bars, inner_bottom, fill);
        cx.mesh(Layer::Data, mesh);
    }
    // ECDF overlay (crossfade target): cumulative fraction on the same x.
    if o.ecdf_alpha > 0.01 {
        let total: u64 = d.counts.iter().map(|&c| c as u64).sum::<u64>() + d.busts;
        if total > 0 {
            let color = with_alpha(t.ink, (230.0 * o.ecdf_alpha) as u8);
            let mut cum = d.busts as f64;
            let mut pts = Vec::with_capacity(d.counts.len() + 1);
            let top = cx.y.r1;
            let y_of = |frac: f64| top + (inner_bottom - top) * (1.0 - frac) as f32;
            pts.push(Pos2::new(cx.x.to_screen(0.0), y_of(cum / total as f64)));
            for (i, &c) in d.counts.iter().enumerate() {
                cum += c as f64;
                pts.push(Pos2::new(
                    cx.x.to_screen(((i + 1) as i64 * d.bin_width) as f64),
                    y_of(cum / total as f64),
                ));
            }
            let pts = marks::decimate(&pts, cx.rect.width());
            cx.line(Layer::Data, pts, Stroke::new(1.8, color));
        }
    }

    // The bust bar: separated, gapped, red, labeled. Never smoothed, never
    // dimmed — Principle 3. It stays out of the y-axis gutter and below the
    // twin-axis strip, and its label sits at the bar's FOOT so it stays
    // readable even when the bar is clamped (bust counts routinely dwarf
    // every ordinary bin).
    if d.busts > 0 {
        let x0 = cx.rect.left() + axis::MARGIN_LEFT + 1.0;
        let x1 = cx.x.to_screen(0.0) - 6.0; // the physical gap
        if x1 > x0 {
            let y_raw = cx.y.to_screen(d.busts as f64);
            let y = y_raw.max(cx.rect.top() + 16.0);
            let rect = egui::Rect::from_min_max(Pos2::new(x0, y), Pos2::new(x1, inner_bottom));
            cx.rect_filled(Layer::Annotation, rect, t.ruin);
            // At the bar's foot, clear of the marker-label rows up top.
            let clamped = y_raw < cx.rect.top() + 16.0;
            cx.text_pilled(
                Layer::Annotation,
                Pos2::new(x1 + 6.0, inner_bottom - 6.0),
                Align2::LEFT_BOTTOM,
                format!(
                    "BUST {:.1}% — ended with $0{}",
                    d.bust_frac * 100.0,
                    if clamped { " · bar off scale" } else { "" }
                ),
                FontId::new(type_scale::CAPTION, theme::mono()),
                t.ruin,
                t.pill(),
            );
        }
    }

    // Mean CI ribbon FIRST (Ribbon layer), then the estimate lines.
    // Marker labels stack in fixed rows below the twin axis so they can
    // never collide; each label sits beside its own line's x, flipping to
    // the left of the line when it would run off the right edge.
    let label_font = FontId::new(type_scale::CAPTION, theme::mono());
    let rect_top = cx.rect.top();
    let rect_right = cx.rect.right();
    let row_y = move |row: f32| rect_top + 16.0 + 13.0 * row;
    let lf = label_font.clone();
    let pill = t.pill();
    let labeled_line =
        move |cx: &mut ChartCx<'_>, sx: f32, row: f32, text: &str, color: Color32| {
            let flip = sx > rect_right - 260.0;
            cx.text_pilled(
                Layer::Estimate,
                Pos2::new(if flip { sx - 4.0 } else { sx + 4.0 }, row_y(row)),
                if flip {
                    Align2::RIGHT_TOP
                } else {
                    Align2::LEFT_TOP
                },
                text,
                lf.clone(),
                color,
                pill,
            );
        };
    if let Some((mean, ci)) = o.mean_ci.filter(|_| n_total > 0) {
        let x0 = cx.x.to_screen(mean - ci);
        let x1 = cx.x.to_screen(mean + ci);
        cx.rect_filled(
            Layer::Ribbon,
            egui::Rect::from_min_max(
                Pos2::new(x0, cx.y.r1),
                Pos2::new(x1.max(x0 + 1.0), inner_bottom),
            ),
            t.ci_band(t.amber),
        );
        cx.dashed_vline(Layer::Estimate, cx.x.to_screen(mean), t.amber);
        labeled_line(cx, cx.x.to_screen(mean), 2.0, "mean (simulated)", t.amber);
    }
    if let Some(m) = o.median.filter(|_| n_total > 0) {
        let sx = cx.x.to_screen(m);
        cx.vline(Layer::Estimate, sx, Stroke::new(1.5, t.ink));
        labeled_line(cx, sx, 3.0, "median", t.ink);
    }
    if let Some((hl, label)) = &o.house_line {
        let sx = cx.x.to_screen(*hl);
        cx.vline(Layer::Estimate, sx, Stroke::new(1.2, t.amber));
        labeled_line(cx, sx, 0.0, label.trim(), t.amber);
        if let Some(cap) = &o.house_gap_caption {
            cx.text_pilled(
                Layer::Annotation,
                Pos2::new(cx.rect.right() - 8.0, inner_bottom - 30.0),
                Align2::RIGHT_BOTTOM,
                cap,
                FontId::new(type_scale::CAPTION, theme::sans()),
                t.ink2,
                t.pill(),
            );
        }
    }
    if let Some((x, label)) = &o.ahead_line {
        let sx = cx.x.to_screen(*x);
        cx.vline(Layer::Estimate, sx, Stroke::new(1.0, t.hairline_strong));
        labeled_line(cx, sx, 4.0, label, t.ink2);
    }
    if let Some(label) = &o.pinned_edge_label {
        // Pinned to the frame, not the data: it CANNOT drift.
        cx.text_pilled(
            Layer::Estimate,
            Pos2::new(
                cx.rect.right() - 8.0,
                cx.rect.bottom() - axis::MARGIN_BOTTOM - 16.0,
            ),
            Align2::RIGHT_BOTTOM,
            label,
            label_font.clone(),
            t.amber,
            t.pill(),
        );
    }
    if let Some((x, label)) = &o.loss_pin {
        let sx = cx.x.to_screen(*x);
        super::marks::mean_diamond(cx, Pos2::new(sx, row_y(1.0) + 5.0), 4.0);
        labeled_line(cx, sx + 6.0, 1.0, label, t.amber);
    }
    if let Some(label) = &o.brush_label {
        cx.text_pilled(
            Layer::Overlay,
            Pos2::new(cx.rect.center().x, cx.rect.top() + 4.0),
            Align2::CENTER_TOP,
            label,
            FontId::new(type_scale::BODY, theme::mono()),
            t.blue,
            t.pill(),
        );
    }
}

/// Paired-difference histogram (the Duel): teal mass right of zero, ruin
/// red left, bold zero line, a gray tie band when the median-Δ CI includes
/// zero — rendered as large as any verdict would be.
pub struct DiffHistogram<'a> {
    /// Symmetric bins around zero: bin i covers
    /// [origin + i·width, origin + (i+1)·width).
    pub counts: &'a [u32],
    pub origin: i64,
    pub bin_width: i64,
    pub median_ci: (i64, i64),
    pub tie: bool,
    pub tie_label: String,
}

pub fn paint_diff(cx: &mut ChartCx<'_>, d: &DiffHistogram<'_>) {
    let t = cx.theme;
    let x0 = d.origin as f64;
    let x1 = (d.origin + d.counts.len() as i64 * d.bin_width) as f64;
    cx.set_x_domain(x0, x1);
    let inner_bottom = cx.rect.bottom() - axis::MARGIN_BOTTOM;
    let max_count = d.counts.iter().copied().max().unwrap_or(1).max(1) as f64;
    cx.set_y_domain(0.0, max_count * 1.05);
    // Unlike the main histogram (whose negative x is the synthetic bust
    // slot), the negative half here is real data: A−B losses get labels.
    axis::x_axis(cx, |v| numerals::money_text(v as i64, false));
    axis::y_axis(cx, |v| numerals::compact_n(v as u64));

    let mut gain_mesh = Mesh::default();
    let mut loss_mesh = Mesh::default();
    for (i, &c) in d.counts.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let lo = d.origin + i as i64 * d.bin_width;
        let sx0 = cx.x.to_screen(lo as f64) + 0.5;
        let sx1 = cx.x.to_screen((lo + d.bin_width) as f64) - 0.5;
        let y = cx.y.to_screen(c as f64).max(cx.rect.top());
        let rect = egui::Rect::from_min_max(Pos2::new(sx0, y), Pos2::new(sx1, inner_bottom));
        if lo >= 0 {
            gain_mesh.add_colored_rect(rect, t.gain);
        } else {
            loss_mesh.add_colored_rect(rect, t.ruin);
        }
    }
    cx.mesh(Layer::Data, gain_mesh);
    cx.mesh(Layer::Data, loss_mesh);

    // Tie band spans the median CI, stamped at verdict size.
    if d.tie {
        let bx0 = cx.x.to_screen(d.median_ci.0 as f64);
        let bx1 = cx.x.to_screen(d.median_ci.1 as f64);
        cx.rect_filled(
            Layer::Ribbon,
            egui::Rect::from_min_max(
                Pos2::new(bx0.min(bx1), cx.rect.top()),
                Pos2::new(bx0.max(bx1).max(bx0.min(bx1) + 2.0), inner_bottom),
            ),
            with_alpha(t.ink2, 40),
        );
        cx.text_pilled(
            Layer::Annotation,
            Pos2::new(cx.rect.center().x, cx.rect.top() + 8.0),
            Align2::CENTER_TOP,
            &d.tie_label,
            FontId::new(type_scale::SECTION, theme::sans_semibold()),
            t.ink2,
            t.pill(),
        );
    }
    // Bold zero.
    cx.vline(
        Layer::Estimate,
        cx.x.to_screen(0.0),
        Stroke::new(2.0, t.ink),
    );
}

fn with_alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}
