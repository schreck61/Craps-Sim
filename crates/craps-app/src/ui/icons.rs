// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The 10-glyph line icon set (spec §4.4): 1.5 px stroke on a 16 px grid,
//! painted as vectors — no icon font, no emoji, no fills except the two
//! dots of the duel/pair-link glyph and the seed hexagon's hash.
//!
//! Dice pips appear in exactly one place in the product — the Replay dice
//! strip — where dice are data; they are painted there, not here.

use egui::{pos2, vec2, Color32, Painter, Rect, Stroke};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Run,
    Stop,
    Pin,
    /// Two dots joined — doubles as the pair-link glyph stamped on anything
    /// computed under common random numbers.
    Duel,
    Replay,
    ExportImage,
    /// Hexagon around a hash mark.
    Seed,
    Theme,
    Ledger,
    ChevronDown,
    ChevronRight,
}

/// Paint `icon` centered in `rect` with the standard 1.5 px stroke.
pub fn paint(painter: &Painter, icon: Icon, rect: Rect, ink: Color32) {
    let s = Stroke::new(1.5, ink);
    // Normalize to a centered 16-unit box.
    let side = rect.width().min(rect.height());
    let c = rect.center();
    let u = side / 16.0;
    let p = |x: f32, y: f32| pos2(c.x + (x - 8.0) * u, c.y + (y - 8.0) * u);

    match icon {
        Icon::Run => {
            painter.line(
                vec![p(5.0, 3.5), p(12.5, 8.0), p(5.0, 12.5), p(5.0, 3.5)],
                s,
            );
        }
        Icon::Stop => {
            painter.rect_stroke(
                Rect::from_min_max(p(4.5, 4.5), p(11.5, 11.5)),
                0.0,
                s,
                egui::StrokeKind::Inside,
            );
        }
        Icon::Pin => {
            painter.line(vec![p(8.0, 2.5), p(8.0, 9.5)], s);
            painter.circle_stroke(p(8.0, 5.0), 2.5 * u, s);
            painter.line(vec![p(8.0, 9.5), p(8.0, 13.5)], s);
            painter.line(vec![p(6.0, 13.5), p(10.0, 13.5)], s);
        }
        Icon::Duel => {
            painter.circle_filled(p(4.5, 8.0), 1.8 * u, ink);
            painter.circle_filled(p(11.5, 8.0), 1.8 * u, ink);
            painter.line(vec![p(6.2, 8.0), p(9.8, 8.0)], s);
        }
        Icon::Replay => {
            // Open circular arrow.
            let r = 4.5 * u;
            let mut pts = Vec::with_capacity(24);
            for i in 0..=20 {
                let a = -0.4 + 4.9 * i as f32 / 20.0; // ~281°, gap at upper right
                pts.push(pos2(c.x + r * a.cos(), c.y + r * a.sin()));
            }
            painter.line(pts, s);
            // Arrow head at the gap end.
            let a = -0.4f32;
            let tip = pos2(c.x + r * a.cos(), c.y + r * a.sin());
            painter.line(
                vec![
                    tip + vec2(-2.2 * u, -1.2 * u),
                    tip,
                    tip + vec2(-0.6 * u, 2.4 * u),
                ],
                s,
            );
        }
        Icon::ExportImage => {
            painter.rect_stroke(
                Rect::from_min_max(p(3.0, 5.0), p(13.0, 13.0)),
                0.0,
                s,
                egui::StrokeKind::Inside,
            );
            painter.line(vec![p(8.0, 9.5), p(8.0, 1.5)], s);
            painter.line(vec![p(5.8, 3.7), p(8.0, 1.5), p(10.2, 3.7)], s);
        }
        Icon::Seed => {
            let mut hexpts = Vec::with_capacity(7);
            for i in 0..=6 {
                let a = std::f32::consts::FRAC_PI_6 + i as f32 * std::f32::consts::FRAC_PI_3;
                hexpts.push(pos2(c.x + 6.0 * u * a.cos(), c.y + 6.0 * u * a.sin()));
            }
            painter.line(hexpts, s);
            // Hash mark.
            painter.line(vec![p(6.5, 5.5), p(6.0, 10.5)], s);
            painter.line(vec![p(10.0, 5.5), p(9.5, 10.5)], s);
            painter.line(vec![p(5.2, 7.0), p(11.0, 7.0)], s);
            painter.line(vec![p(5.0, 9.0), p(10.8, 9.0)], s);
        }
        Icon::Theme => {
            painter.circle_stroke(p(8.0, 8.0), 5.0 * u, s);
            // Half fill: a filled semicircle suggested by three chords.
            for dy in [-2.5f32, 0.0, 2.5] {
                let half = (25.0 - dy * dy).sqrt() * u;
                painter.line(
                    vec![pos2(c.x, c.y + dy * u), pos2(c.x + half, c.y + dy * u)],
                    Stroke::new(1.5, ink),
                );
            }
        }
        Icon::Ledger => {
            for y in [4.5f32, 8.0, 11.5] {
                painter.line(vec![p(3.5, y), p(12.5, y)], s);
            }
            painter.line(vec![p(5.5, 3.0), p(5.5, 13.0)], s);
        }
        Icon::ChevronDown => {
            painter.line(vec![p(4.5, 6.0), p(8.0, 10.0), p(11.5, 6.0)], s);
        }
        Icon::ChevronRight => {
            painter.line(vec![p(6.0, 4.5), p(10.0, 8.0), p(6.0, 11.5)], s);
        }
    }
}

/// A clickable icon button of the standard 20 px hit size, with the focus
/// ring the spec demands (1 px blue, never a glow).
pub fn button(
    ui: &mut egui::Ui,
    icon: Icon,
    ink: Color32,
    focus_ring: Color32,
    tooltip: &str,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let ink = if resp.hovered() {
            ink
        } else {
            Color32::from_rgba_unmultiplied(ink.r(), ink.g(), ink.b(), 200)
        };
        paint(ui.painter(), icon, rect.shrink(2.0), ink);
        if resp.has_focus() {
            ui.painter().rect_stroke(
                rect,
                3.0,
                Stroke::new(1.0, focus_ring),
                egui::StrokeKind::Outside,
            );
        }
    }
    resp.on_hover_text(tooltip)
}
