// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The dice strip: every roll as a monochrome pip-pair glyph — the one
//! place in the product where dice appear, because here dice are data.
//! Come-out rolls ringed, points boxed, seven-outs struck in red;
//! x-virtualized to the visible clip.

use craps_engine::trace::RollEvent;
use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke};

use crate::ui::numerals;
use crate::ui::theme::{self, type_scale, Theme};

const CELL: f32 = 26.0;

/// Paint the strip into `rect`, horizontally scrolled so `playhead` (roll
/// index) is visible. Returns the roll index under the pointer, if any.
pub fn paint(
    ui: &mut egui::Ui,
    t: &Theme,
    rect: Rect,
    events: &[RollEvent],
    playhead: usize,
    hovered_out: &mut Option<usize>,
) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, t.surface);
    let visible = (rect.width() / CELL).ceil() as usize;
    // Keep the playhead ~2/3 across when scrolling.
    let first = playhead.saturating_sub(visible * 2 / 3);
    let pointer = ui.ctx().pointer_hover_pos().filter(|p| rect.contains(*p));

    let mut was_comeout_next = true; // roll 1 is always a come-out
    if first > 0 {
        // Reconstruct whether the first visible roll is a come-out from the
        // previous roll's point_after.
        was_comeout_next = events
            .get(first - 1)
            .map(|e| e.point_after.is_none())
            .unwrap_or(true);
    }

    let mut comeout = was_comeout_next;
    for (col, ev) in events.iter().skip(first).take(visible + 1).enumerate() {
        let ix = first + col;
        let x = rect.left() + col as f32 * CELL + CELL / 2.0;
        let cy = rect.center().y;
        let total = ev.dice.0 + ev.dice.1;
        let seven_out = !comeout && total == 7;
        let ink = if ix == playhead {
            t.ink
        } else if ix > playhead {
            t.dimmed(t.ink2)
        } else {
            t.ink2
        };

        // Pip pair.
        pip_die(&painter, Pos2::new(x - 6.0, cy - 5.0), ev.dice.0, ink);
        pip_die(&painter, Pos2::new(x + 6.0, cy - 5.0), ev.dice.1, ink);

        // Come-out ring / point box / seven-out strike.
        if comeout {
            painter.circle_stroke(Pos2::new(x, cy - 5.0), 12.0, Stroke::new(1.0, t.ink2));
        }
        if ev.point_after.is_some() && comeout {
            // This roll set the point: box it.
            painter.rect_stroke(
                Rect::from_center_size(Pos2::new(x, cy - 5.0), egui::vec2(24.0, 22.0)),
                3.0,
                Stroke::new(1.0, t.hairline_strong),
                egui::StrokeKind::Outside,
            );
        }
        if seven_out {
            painter.line_segment(
                [
                    Pos2::new(x - 12.0, cy + 7.0),
                    Pos2::new(x + 12.0, cy - 17.0),
                ],
                Stroke::new(1.5, t.ruin),
            );
        }
        painter.text(
            Pos2::new(x, rect.bottom() - 4.0),
            Align2::CENTER_BOTTOM,
            format!("{total}"),
            FontId::new(9.0, theme::mono()),
            ink,
        );

        if let Some(p) = pointer {
            if (p.x - x).abs() < CELL / 2.0 {
                *hovered_out = Some(ix);
            }
        }
        comeout = ev.point_after.is_none();
    }

    // Hover lane: working bets and resolutions to the cent.
    if let Some(ix) = *hovered_out {
        if let Some(ev) = events.get(ix) {
            let mut lines = vec![format!(
                "roll {}: {}+{} = {}",
                ev.roll,
                ev.dice.0,
                ev.dice.1,
                ev.dice.0 + ev.dice.1
            )];
            for be in &ev.events {
                lines.push(format!(
                    "{:?} {} {}",
                    be.bet,
                    match be.kind {
                        craps_engine::trace::BetEventKind::Placed => "placed".to_owned(),
                        craps_engine::trace::BetEventKind::Won { paid_cents, .. } =>
                            format!("wins {}", numerals::money_text(paid_cents, false)),
                        craps_engine::trace::BetEventKind::Lost => "loses".to_owned(),
                        craps_engine::trace::BetEventKind::Pushed => "pushes".to_owned(),
                        craps_engine::trace::BetEventKind::Returned => "returned".to_owned(),
                        craps_engine::trace::BetEventKind::TakenDown => "taken down".to_owned(),
                        craps_engine::trace::BetEventKind::Traveled { to } =>
                            format!("travels to {to}"),
                        craps_engine::trace::BetEventKind::Rejected { reason, what } =>
                            format!("{} refused — {}", what.label(), reason.label()),
                        craps_engine::trace::BetEventKind::ClippedToMax =>
                            "clipped to table max".to_owned(),
                    },
                    numerals::money_text(be.stake_cents, false)
                ));
            }
            lines.push(format!(
                "cash {} · wealth {}",
                numerals::money_text(ev.cash_after, false),
                numerals::money_text(ev.wealth_after, false)
            ));
            if let Some(pos) = ui.ctx().pointer_hover_pos() {
                egui::Area::new(egui::Id::new("dice_tip"))
                    .fixed_pos(pos + egui::vec2(12.0, 12.0))
                    .order(egui::Order::Tooltip)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::NONE
                            .fill(t.surface2)
                            .stroke(Stroke::new(1.0, t.hairline_strong))
                            .corner_radius(4)
                            .inner_margin(8.0)
                            .show(ui, |ui| {
                                for l in lines {
                                    ui.label(
                                        egui::RichText::new(l)
                                            .font(FontId::new(type_scale::CAPTION, theme::mono()))
                                            .color(t.ink),
                                    );
                                }
                            });
                    });
            }
        }
    }
}

fn pip_die(painter: &egui::Painter, center: Pos2, face: u8, ink: Color32) {
    let r = 5.0;
    painter.rect_stroke(
        Rect::from_center_size(center, egui::vec2(2.0 * r, 2.0 * r)),
        2.0,
        Stroke::new(1.0, ink),
        egui::StrokeKind::Inside,
    );
    let p = 2.4f32;
    let dots: &[(f32, f32)] = match face {
        1 => &[(0.0, 0.0)],
        2 => &[(-p, -p), (p, p)],
        3 => &[(-p, -p), (0.0, 0.0), (p, p)],
        4 => &[(-p, -p), (p, -p), (-p, p), (p, p)],
        5 => &[(-p, -p), (p, -p), (0.0, 0.0), (-p, p), (p, p)],
        _ => &[(-p, -p), (p, -p), (-p, 0.0), (p, 0.0), (-p, p), (p, p)],
    };
    for &(dx, dy) in dots {
        painter.circle_filled(center + egui::vec2(dx, dy), 0.9, ink);
    }
}
