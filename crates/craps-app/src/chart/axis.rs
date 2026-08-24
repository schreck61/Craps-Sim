// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Axis helpers: hairline grids, tick labels in secondary ink, and the
//! twin-axis labeler (one scale, two units — a relabel, never a second
//! scale).

use egui::{Align2, FontId, Pos2, Stroke};

use super::frame::{ChartCx, Layer};
use crate::ui::theme::{self, type_scale};

/// Standard margins a body should reserve inside the plot rect: left for
/// y labels, bottom for x labels (and a second line when a twin axis is on
/// top).
pub const MARGIN_LEFT: f32 = 46.0;
pub const MARGIN_BOTTOM: f32 = 22.0;

/// Draw x ticks along the bottom edge with hairline gridlines. The
/// formatter is kept on the context so the frame can print pin values and
/// pin-to-pin Δ in the same units as the axis.
pub fn x_axis(cx: &mut ChartCx<'_>, fmt: impl Fn(f64) -> String + 'static) {
    let t = cx.theme;
    let fmt = std::sync::Arc::new(fmt);
    let ticks = cx.x.ticks(((cx.rect.width() / 90.0) as usize).max(2));
    let font = FontId::new(type_scale::CAPTION, theme::mono());
    for v in ticks {
        let sx = cx.x.to_screen(v);
        cx.line(
            Layer::Grid,
            vec![
                Pos2::new(sx, cx.rect.top()),
                Pos2::new(sx, cx.rect.bottom() - MARGIN_BOTTOM + 4.0),
            ],
            Stroke::new(1.0, t.hairline),
        );
        cx.text(
            Layer::Grid,
            Pos2::new(sx, cx.rect.bottom() - MARGIN_BOTTOM + 6.0),
            Align2::CENTER_TOP,
            fmt(v),
            font.clone(),
            t.ink2,
        );
    }
    cx.x_fmt = Some(fmt);
}

/// A second labeler along the TOP edge for the same x scale (hours over
/// rolls, ×-of-budget over dollars).
pub fn x_axis_twin_top(cx: &mut ChartCx<'_>, fmt: impl Fn(f64) -> String) {
    let t = cx.theme;
    let ticks = cx.x.ticks(((cx.rect.width() / 110.0) as usize).max(2));
    let font = FontId::new(type_scale::CAPTION, theme::mono());
    for v in ticks {
        let sx = cx.x.to_screen(v);
        cx.text(
            Layer::Grid,
            Pos2::new(sx, cx.rect.top() + 2.0),
            Align2::CENTER_TOP,
            fmt(v),
            font.clone(),
            t.ink2,
        );
    }
}

/// Draw y ticks along the left edge with hairline gridlines.
pub fn y_axis(cx: &mut ChartCx<'_>, fmt: impl Fn(f64) -> String) {
    let t = cx.theme;
    let ticks = cx.y.ticks(((cx.rect.height() / 60.0) as usize).max(2));
    let font = FontId::new(type_scale::CAPTION, theme::mono());
    for v in ticks {
        let sy = cx.y.to_screen(v);
        cx.line(
            Layer::Grid,
            vec![
                Pos2::new(cx.rect.left() + MARGIN_LEFT - 4.0, sy),
                Pos2::new(cx.rect.right(), sy),
            ],
            Stroke::new(1.0, t.hairline),
        );
        cx.text(
            Layer::Grid,
            Pos2::new(cx.rect.left() + MARGIN_LEFT - 8.0, sy),
            Align2::RIGHT_CENTER,
            fmt(v),
            font.clone(),
            t.ink2,
        );
    }
}
