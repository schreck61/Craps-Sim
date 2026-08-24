// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Shared mark builders: single-mesh bars, triangle-strip bands (CI bands
//! are concave wherever a curve bends — never `convex_polygon`), hatch
//! blocks for censoring, whiskers drawn before their dots, and the amber
//! mean diamond.

use egui::{Color32, Mesh, Pos2, Rect, Shape, Stroke, Vec2};

use super::frame::{ChartCx, Layer};

/// All bars of a histogram as ONE mesh (two triangles per bar, one draw
/// call). `bars` are (x_left, x_right, y_top) in screen space; baseline is
/// the bottom.
pub fn bars_mesh(
    bars: impl Iterator<Item = (f32, f32, f32)>,
    baseline: f32,
    fill: Color32,
) -> Mesh {
    let mut mesh = Mesh::default();
    for (x0, x1, y_top) in bars {
        if y_top >= baseline {
            continue;
        }
        mesh.add_colored_rect(
            Rect::from_min_max(Pos2::new(x0, y_top), Pos2::new(x1, baseline)),
            fill,
        );
    }
    mesh
}

/// A band between an upper and lower polyline as a triangle strip. Both
/// runs must share x positions, in order.
pub fn band_mesh(upper: &[Pos2], lower: &[Pos2], fill: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    debug_assert_eq!(upper.len(), lower.len());
    let n = upper.len().min(lower.len());
    if n < 2 {
        return mesh;
    }
    for i in 0..n {
        mesh.colored_vertex(upper[i], fill);
        mesh.colored_vertex(lower[i], fill);
    }
    for i in 0..(n as u32 - 1) {
        let (u0, l0, u1, l1) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
        mesh.add_triangle(u0, l0, u1);
        mesh.add_triangle(l0, l1, u1);
    }
    mesh
}

/// Diagonal hatching for censored regions ("survived cap"), 45°, 6 px pitch.
pub fn hatch(cx: &mut ChartCx<'_>, layer: Layer, rect: Rect, ink: Color32) {
    let stroke = Stroke::new(1.0, ink);
    let pitch = 6.0;
    let mut c = rect.left() - rect.height();
    let mut shapes = Vec::new();
    while c < rect.right() {
        // The 45° segment from (c, bottom) to (c + height, top), clipped.
        let a = Pos2::new(c, rect.bottom());
        let b = Pos2::new(c + rect.height(), rect.top());
        let (a, b) = clip_to_rect(a, b, rect);
        if (a - b).length_sq() > 1.0 {
            shapes.push(Shape::line_segment([a, b], stroke));
        }
        c += pitch;
    }
    for s in shapes {
        cx.shape(layer, s);
    }
}

fn clip_to_rect(a: Pos2, b: Pos2, r: Rect) -> (Pos2, Pos2) {
    // Liang–Barsky on the segment a→b.
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    let checks = [
        (-d.x, a.x - r.left()),
        (d.x, r.right() - a.x),
        (-d.y, a.y - r.top()),
        (d.y, r.bottom() - a.y),
    ];
    for (p, q) in checks {
        if p == 0.0 {
            if q < 0.0 {
                return (a, a);
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    if t0 > t1 {
        return (a, a);
    }
    (a + d * t0, a + d * t1)
}

/// A horizontal CI whisker: painted to [`Layer::Ribbon`] so it exists
/// BEFORE its point estimate (the eye learns the interval is the finding).
pub fn ci_whisker(cx: &mut ChartCx<'_>, y: f32, x0: f32, x1: f32, color: Color32) {
    let cap = 3.0;
    cx.line(
        Layer::Ribbon,
        vec![Pos2::new(x0, y), Pos2::new(x1, y)],
        Stroke::new(1.2, color),
    );
    for x in [x0, x1] {
        cx.line(
            Layer::Ribbon,
            vec![Pos2::new(x, y - cap), Pos2::new(x, y + cap)],
            Stroke::new(1.2, color),
        );
    }
}

/// The amber mean diamond (means are dashed/diamond, medians solid ticks —
/// skew legible at a glance).
pub fn mean_diamond(cx: &mut ChartCx<'_>, center: Pos2, r: f32) {
    let amber = cx.theme.amber;
    let pts = vec![
        center + Vec2::new(0.0, -r),
        center + Vec2::new(r, 0.0),
        center + Vec2::new(0.0, r),
        center + Vec2::new(-r, 0.0),
    ];
    cx.shape(
        Layer::Estimate,
        Shape::convex_polygon(pts, amber, Stroke::NONE),
    );
}

/// Decimate a polyline to at most ~2 vertices per pixel column, preserving
/// per-column min/max so spikes survive.
pub fn decimate(points: &[Pos2], px_width: f32) -> Vec<Pos2> {
    if points.len() <= (px_width as usize) * 2 || points.len() < 4 {
        return points.to_vec();
    }
    let mut out: Vec<Pos2> = Vec::with_capacity(px_width as usize * 2 + 2);
    let left = points.first().unwrap().x;
    let mut col = f32::NEG_INFINITY;
    let (mut lo, mut hi) = (Pos2::ZERO, Pos2::ZERO);
    let mut have = false;
    for &p in points {
        let c = (p.x - left).floor();
        if c != col {
            if have {
                if lo.y <= hi.y {
                    out.push(hi);
                    out.push(lo);
                } else {
                    out.push(lo);
                    out.push(hi);
                }
            }
            col = c;
            lo = p;
            hi = p;
            have = true;
        } else {
            if p.y > lo.y {
                lo = p;
            }
            if p.y < hi.y {
                hi = p;
            }
        }
    }
    if have {
        out.push(hi);
        out.push(lo);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_mesh_covers_the_strip() {
        let upper: Vec<Pos2> = (0..5).map(|i| Pos2::new(i as f32 * 10.0, 10.0)).collect();
        let lower: Vec<Pos2> = (0..5).map(|i| Pos2::new(i as f32 * 10.0, 30.0)).collect();
        let m = band_mesh(&upper, &lower, Color32::RED);
        assert_eq!(m.vertices.len(), 10);
        assert_eq!(m.indices.len(), 4 * 2 * 3);
    }

    #[test]
    fn bars_mesh_one_rect_per_visible_bar() {
        let bars = [
            (0.0f32, 5.0f32, 10.0f32),
            (5.0, 10.0, 20.0),
            (10.0, 15.0, 40.0),
        ];
        let m = bars_mesh(bars.iter().copied(), 40.0, Color32::WHITE);
        // Third bar has zero height (y_top == baseline) and is skipped;
        // 2 rects × 4 vertices.
        assert_eq!(m.vertices.len(), 8);
    }

    #[test]
    fn clip_keeps_segments_inside() {
        let r = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(100.0, 50.0));
        let (a, b) = clip_to_rect(Pos2::new(-50.0, 100.0), Pos2::new(150.0, -100.0), r);
        for p in [a, b] {
            assert!(r.expand(0.01).contains(p), "{p:?} outside {r:?}");
        }
    }

    #[test]
    fn decimation_preserves_extremes() {
        let mut pts: Vec<Pos2> = Vec::new();
        for i in 0..10_000 {
            let y = if i == 5_000 {
                0.0
            } else {
                50.0 + (i % 7) as f32
            };
            pts.push(Pos2::new(i as f32 / 100.0, y));
        }
        let dec = decimate(&pts, 100.0);
        assert!(dec.len() < 400);
        assert!(
            dec.iter().any(|p| p.y == 0.0),
            "the spike must survive decimation"
        );
    }
}
