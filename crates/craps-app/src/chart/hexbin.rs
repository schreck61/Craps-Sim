// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The Duel's y = x scatter as a hexbin density plot — never 1.2M raw
//! circles. Bone→identity ramp, the diagonal etched, quadrant counts in
//! the corners.

use craps_engine::HexBin;
use egui::{Align2, Color32, FontId, Pos2, Shape, Stroke};

use super::axis;
use super::frame::{ChartCx, Layer};
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale};

pub struct HexbinPlot<'a> {
    pub bins: &'a [HexBin],
    /// Shared axis extent (both axes), cents.
    pub extent: (f64, f64),
    pub a_label: &'a str,
    pub b_label: &'a str,
    /// (a higher, b higher, within a dollar) session counts.
    pub quadrants: (u64, u64, u64),
    pub ramp_to: Color32,
}

pub fn paint(cx: &mut ChartCx<'_>, p: &HexbinPlot<'_>) {
    let t = cx.theme;
    cx.set_x_domain(p.extent.0, p.extent.1);
    cx.set_y_domain(p.extent.0, p.extent.1);
    let inner_bottom = cx.rect.bottom() - axis::MARGIN_BOTTOM;
    axis::x_axis(cx, |v| numerals::money_text(v as i64, false));
    axis::y_axis(cx, |v| numerals::money_text(v as i64, false));

    let max_count = p.bins.iter().map(|b| b.count).max().unwrap_or(1) as f32;
    // Hex size in screen px from the data pitch.
    let span = (p.extent.1 - p.extent.0).max(1.0);
    let size_px = (cx.rect.width() / (span as f32)) * (span as f32 / 40.0 / 3f32.sqrt());
    let size_px = size_px.clamp(2.0, 14.0);

    for b in p.bins {
        let cxy = cx.xy(b.center_x, b.center_y);
        if cxy.y < cx.rect.top() || cxy.y > inner_bottom {
            continue;
        }
        let k = (b.count as f32 / max_count).powf(0.5);
        let color = lerp_color(t.surface2, p.ramp_to, k);
        let mut pts = Vec::with_capacity(6);
        for i in 0..6 {
            let a = std::f32::consts::FRAC_PI_6 + i as f32 * std::f32::consts::FRAC_PI_3;
            pts.push(Pos2::new(
                cxy.x + size_px * a.cos(),
                cxy.y + size_px * a.sin(),
            ));
        }
        cx.shape(Layer::Data, Shape::convex_polygon(pts, color, Stroke::NONE));
    }

    // The y = x diagonal, etched.
    let d0 = cx.xy(p.extent.0, p.extent.0);
    let d1 = cx.xy(p.extent.1, p.extent.1);
    cx.line(
        Layer::Estimate,
        vec![d0, d1],
        Stroke::new(1.0, t.hairline_strong),
    );

    // Quadrant counts printed in the corners.
    let (a_hi, b_hi, tie) = p.quadrants;
    cx.text(
        Layer::Annotation,
        Pos2::new(cx.rect.right() - 6.0, inner_bottom - 6.0),
        Align2::RIGHT_BOTTOM,
        format!("{} higher: {}", p.a_label, numerals::compact_n(a_hi)),
        FontId::new(type_scale::CAPTION, theme::mono()),
        t.ink,
    );
    cx.text(
        Layer::Annotation,
        Pos2::new(
            cx.rect.left() + axis::MARGIN_LEFT + 6.0,
            cx.rect.top() + 6.0,
        ),
        Align2::LEFT_TOP,
        format!("{} higher: {}", p.b_label, numerals::compact_n(b_hi)),
        FontId::new(type_scale::CAPTION, theme::mono()),
        t.ink,
    );
    cx.text(
        Layer::Annotation,
        Pos2::new(cx.rect.right() - 6.0, cx.rect.top() + 6.0),
        Align2::RIGHT_TOP,
        format!("within $1: {}", numerals::compact_n(tie)),
        FontId::new(type_scale::CAPTION, theme::mono()),
        t.ink2,
    );
}

fn lerp_color(a: Color32, b: Color32, k: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}
