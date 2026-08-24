// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The dot field: ~1,000 sampled sessions as dots positioned by peak
//! outlay, y-jittered in a shallow band, re-coloring live under the budget
//! line — dots that die flick to ruin red, dots that live settle to bone.
//! One mesh; color decided at paint time, so the recolor is a free repaint.

use craps_engine::splitmix64;
use egui::Mesh;

use super::frame::{ChartCx, Layer};

pub struct DotField<'a> {
    /// (session id, peak outlay cents), deterministic stride sample.
    pub dots: &'a [(u32, i64)],
}

/// Paints into a horizontal band `[y0, y1)` of the plot; x uses the cx's
/// existing x scale (shared with the survival curve above). Returns the id
/// of the dot under the pointer, for `Enter one ▸`.
pub fn paint(
    cx: &mut ChartCx<'_>,
    field: &DotField<'_>,
    y0: f32,
    y1: f32,
    budget_cents: i64,
) -> Option<u32> {
    let t = cx.theme;
    let mut mesh = Mesh::default();
    let r = 2.2f32;
    let mut hovered: Option<(f32, u32)> = None;
    let pointer = cx.response.hover_pos();
    for &(id, outlay) in field.dots {
        let x = cx.x.to_screen(outlay as f64);
        if x < cx.rect.left() || x > cx.rect.right() {
            continue;
        }
        // Deterministic jitter keyed by session id — display randomness,
        // never dice randomness.
        let jitter = (splitmix64(id as u64) % 1000) as f32 / 1000.0;
        let y = y0 + (y1 - y0 - 2.0 * r) * jitter + r;
        let dies = outlay > budget_cents;
        let color = if dies { t.ruin } else { t.ink };
        let rect = egui::Rect::from_center_size(egui::pos2(x, y), egui::vec2(2.0 * r, 2.0 * r));
        mesh.add_colored_rect(rect, color);
        if let Some(p) = pointer {
            let d2 = (p.x - x) * (p.x - x) + (p.y - y) * (p.y - y);
            if d2 < 36.0 && hovered.is_none_or(|(hd, _)| d2 < hd) {
                hovered = Some((d2, id));
            }
        }
    }
    cx.mesh(Layer::Data, mesh);
    hovered.map(|(_, id)| id)
}
