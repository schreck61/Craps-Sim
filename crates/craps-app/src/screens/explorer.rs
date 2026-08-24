// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Explorer — the strip of 528 and the leaderboard. Every dot wears its CI
//! whisker (drawn first); objective flips re-sort with a glide so "best"
//! visibly dissolves and reforms under a different definition of best.
//! The EDGE column is fixed, amber, never sortable into flattery; rows
//! statistically tied with the leader live inside a drawn bracket. The
//! footer never leaves: best of a losing field.

use craps_engine::{blended_edge, explore_strategies, ExploreRow, Progression};
use egui::{Align2, Color32, FontId, Pos2, RichText, Stroke};

use crate::app::App;
use crate::chart::frame::ChartFrame;
use crate::chart::Layer;
use crate::store::ExploreStatus;
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RankBy {
    DoubleUp,
    Ahead,
    Survive,
    MedianFinal,
    MeanFinal,
}

impl RankBy {
    pub const ALL: [RankBy; 5] = [
        RankBy::DoubleUp,
        RankBy::Ahead,
        RankBy::Survive,
        RankBy::MedianFinal,
        RankBy::MeanFinal,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            RankBy::DoubleUp => "Chance to walk out with ≥ 2× budget",
            RankBy::Ahead => "Chance to walk out ahead",
            RankBy::Survive => "Chance to not bust",
            RankBy::MedianFinal => "Median walk-out money",
            RankBy::MeanFinal => "Mean walk-out money (EV)",
        }
    }

    /// Format a row's objective value with its CI, in the objective's own
    /// units (money objectives are money, never percent).
    pub fn fmt(&self, r: &ExploreRow) -> String {
        match self {
            RankBy::DoubleUp => numerals::prob_ci(r.p_double, r.p_double_ci),
            RankBy::Ahead => numerals::prob_ci(r.p_ahead, r.p_ahead_ci),
            RankBy::Survive => numerals::prob_ci(r.p_nobust, r.p_nobust_ci),
            RankBy::MedianFinal => format!(
                "{} ({}–{})",
                numerals::money_text(r.median_final, false),
                numerals::money_text(r.median_final_ci.0, false),
                numerals::money_text(r.median_final_ci.1, false)
            ),
            RankBy::MeanFinal => format!(
                "{} ± {}",
                numerals::money_text(r.mean_final as i64, false),
                numerals::money_text(r.mean_final_ci as i64, false)
            ),
        }
    }

    pub fn key(&self, r: &ExploreRow) -> f64 {
        match self {
            RankBy::DoubleUp => r.p_double,
            RankBy::Ahead => r.p_ahead,
            RankBy::Survive => r.p_nobust,
            RankBy::MedianFinal => r.median_final as f64,
            RankBy::MeanFinal => r.mean_final,
        }
    }

    /// The CI interval on the objective, for tie brackets.
    pub fn interval(&self, r: &ExploreRow) -> (f64, f64) {
        match self {
            RankBy::DoubleUp => (r.p_double - r.p_double_ci, r.p_double + r.p_double_ci),
            RankBy::Ahead => (r.p_ahead - r.p_ahead_ci, r.p_ahead + r.p_ahead_ci),
            RankBy::Survive => (r.p_nobust - r.p_nobust_ci, r.p_nobust + r.p_nobust_ci),
            RankBy::MedianFinal => (r.median_final_ci.0 as f64, r.median_final_ci.1 as f64),
            RankBy::MeanFinal => (
                r.mean_final - r.mean_final_ci,
                r.mean_final + r.mean_final_ci,
            ),
        }
    }
}

/// A combo's identity, sufficient to reconstruct both Duel sides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ComboKey {
    pub min_index: usize,
    pub min_cents: i64,
    pub strategy_idx: u16,
    pub progression: Progression,
    pub quit_idx: u8,
}

#[derive(Default)]
pub struct ExplorerUi {
    pub rank_by: Option<RankBy>,
    pub selected_a: Option<ComboKey>,
    pub selected_b: Option<ComboKey>,
    pub show_all: bool,
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    super::title(ui, &t, "Explorer");

    if app.duel.open {
        // The Duel is taller than one viewport: it scrolls.
        egui::ScrollArea::vertical()
            .id_salt("duel_scroll")
            .show(ui, |ui| super::duel::show(app, ui));
        return;
    }

    let Some(run) = &app.explore_run else {
        super::ghost_panel(
            ui,
            &t,
            "528 ways to lose, ranked honestly — press Explore in the rail.",
        );
        return;
    };
    let store = run.store.clone();
    let st = store.lock().unwrap();

    let rank = app.explorer_ui.rank_by.unwrap_or(RankBy::DoubleUp);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Rank by")
                .font(FontId::new(type_scale::BODY, theme::sans()))
                .color(t.ink2),
        );
        for r in RankBy::ALL {
            let selected = rank == r;
            if ui.selectable_label(selected, r.label()).clicked() {
                app.explorer_ui.rank_by = Some(r);
            }
        }
        if st.status == ExploreStatus::Streaming {
            let done = run
                .ctl
                .sessions_done
                .load(std::sync::atomic::Ordering::Relaxed);
            let frac = done as f32 / run.total_sessions.max(1) as f32;
            ui.add(
                egui::ProgressBar::new(frac)
                    .desired_width(160.0)
                    .text(format!("{:.0}%", frac * 100.0))
                    .animate(false),
            );
            ui.label(
                RichText::new("sweeping the grid — every combo plays identical dice")
                    .font(FontId::new(type_scale::CAPTION, theme::sans()))
                    .color(t.ink2),
            );
        }
    });
    ui.label(
        RichText::new(format!(
            "for {} over {:.1} h · rankings re-sort instantly — all five metrics are already in memory",
            numerals::money_text(st.config.budget_cents, false),
            st.config.target_hours
        ))
        .font(FontId::new(type_scale::CAPTION, theme::sans()))
        .color(t.ink2),
    );

    // The Duel bar: always visible, so the comparison surface is
    // discoverable by mouse alone — click picks A, shift-click picks B,
    // and the button (or D) opens it.
    ui.horizontal(|ui| {
        let cap = FontId::new(type_scale::CAPTION, theme::sans());
        match (app.explorer_ui.selected_a, app.explorer_ui.selected_b) {
            (None, _) => {
                ui.label(
                    RichText::new(
                        "Duel: click any combo to pick side A, ⇧-click another for side B — both replay identical dice.",
                    )
                    .font(cap)
                    .color(t.ink2),
                );
            }
            (Some(a), None) => {
                ui.label(
                    RichText::new(format!("Duel · A: {}", super::duel::combo_name(&a)))
                        .font(cap.clone())
                        .color(t.chip(a.min_cents)),
                );
                ui.label(
                    RichText::new("— ⇧-click another combo for side B")
                        .font(cap)
                        .color(t.ink2),
                );
            }
            (Some(a), Some(b)) => {
                ui.label(
                    RichText::new(format!("Duel · A: {}", super::duel::combo_name(&a)))
                        .font(cap.clone())
                        .color(t.chip(a.min_cents)),
                );
                ui.label(
                    RichText::new(format!("vs B: {}", super::duel::combo_name(&b)))
                        .font(cap.clone())
                        .color(t.violet),
                );
                if a.min_index == b.min_index {
                    if ui
                        .button(
                            RichText::new("Open the Duel ▶")
                                .font(FontId::new(type_scale::CAPTION, theme::sans_semibold())),
                        )
                        .on_hover_text("key D")
                        .clicked()
                    {
                        let ex = std::mem::take(&mut app.explorer_ui);
                        app.duel.open_from_selection(&ex);
                        app.explorer_ui = ex;
                    }
                } else {
                    ui.label(
                        RichText::new("— both sides must share a table minimum")
                            .font(cap)
                            .color(t.amber),
                    );
                }
            }
        }
        if app.explorer_ui.selected_a.is_some() {
            let clear = ui.add(
                egui::Label::new(
                    RichText::new("clear")
                        .font(FontId::new(type_scale::CAPTION, theme::sans()))
                        .color(t.blue),
                )
                .sense(egui::Sense::click()),
            );
            if clear.clicked() {
                app.explorer_ui.selected_a = None;
                app.explorer_ui.selected_b = None;
            }
        }
    });
    ui.add_space(6.0);

    if st.mins.is_empty() {
        drop(st);
        return;
    }
    let stale = st.fingerprint != app.cfg.explore_fingerprint();

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (mi, (min_cents, rows, partial)) in st.mins.iter().enumerate() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{} tables",
                        numerals::money_text(*min_cents, false)
                    ))
                    .font(FontId::new(type_scale::SECTION, theme::sans_semibold()))
                    .color(t.chip(*min_cents)),
                );
                if *partial {
                    ui.label(
                        RichText::new("PARTIAL — combos completed before the cancel")
                            .font(FontId::new(type_scale::CAPTION, theme::mono()))
                            .color(t.amber),
                    );
                }
                if stale {
                    ui.label(
                        RichText::new("STALE — settings changed since this sweep")
                            .font(FontId::new(type_scale::CAPTION, theme::mono()))
                            .color(t.amber),
                    );
                }
            });

            strip_of_combos(app, ui, mi, *min_cents, rows, rank);
            leaderboard(app, ui, mi, *min_cents, rows, rank, &st.config);
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Best of a losing field — every option here has negative expected value.",
            )
            .font(FontId::new(type_scale::CAPTION, theme::sans()))
            .color(t.ink2),
        );
        ui.label(
            RichText::new(
                "Click a row to select it, shift-click a second, then D opens the Duel — both replayed on identical dice.",
            )
            .font(FontId::new(type_scale::CAPTION, theme::sans()))
            .color(t.ink2),
        );
    });
}

/// The strip of 528: every combo one dot on the objective axis, CI whisker
/// beneath it, color = strategy family, shape = quit rule, ring = pressing
/// class. Re-sorts glide (a real FLIP on the x position).
fn strip_of_combos(
    app: &mut App,
    ui: &mut egui::Ui,
    mi: usize,
    min_cents: i64,
    rows: &[ExploreRow],
    rank: RankBy,
) {
    let t = app.theme.clone();
    let dt = ui.input(|i| i.stable_dt.min(0.1));
    let morph = app.motion.duration(crate::ui::motion::MORPH);
    let id = egui::Id::new(("strip", mi));
    let selected = (app.explorer_ui.selected_a, app.explorer_ui.selected_b);

    // Objective extent across the strip.
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for r in rows {
        let (a, b) = rank.interval(r);
        lo = lo.min(a);
        hi = hi.max(b);
    }
    if !lo.is_finite() || !hi.is_finite() || lo == hi {
        return;
    }

    let mut clicked: Option<(ComboKey, bool)> = None;
    ChartFrame::new(id, "The strip — every combo, one dot, its interval first")
        .height(120.0)
        .interactive(false)
        .clickable(true)
        .show(ui, &t, |cx| {
            cx.set_x_domain(lo, hi);
            let strategies = explore_strategies();
            let n_strats = strategies.len().max(1);
            let pointer = cx.response.hover_pos();
            let mut best_hover: Option<(f32, usize)> = None;

            for (k, r) in rows.iter().enumerate() {
                let target_x = cx.x.to_screen(rank.key(r));
                // FLIP: the dot glides to its new objective position.
                let x = app.tweens.drive(
                    id.with((
                        "dot",
                        r.strategy_idx,
                        r.quit_idx,
                        crate::config::progression_index(r.progression),
                    )),
                    target_x,
                    morph,
                    dt,
                );
                // Lane by strategy family keeps 528 dots legible.
                let lane_h = (cx.rect.height() - 30.0) / n_strats as f32;
                let y = cx.rect.top() + 8.0 + r.strategy_idx as f32 * lane_h + lane_h / 2.0;

                // Whisker FIRST (ribbon layer), then the dot.
                let (ci_lo, ci_hi) = rank.interval(r);
                let hue = family_color(&t, r.strategy_idx, n_strats);
                crate::chart::marks::ci_whisker(
                    cx,
                    y,
                    cx.x.to_screen(ci_lo),
                    cx.x.to_screen(ci_hi),
                    t.ci_band(hue),
                );

                let key = ComboKey {
                    min_index: mi,
                    min_cents,
                    strategy_idx: r.strategy_idx,
                    progression: r.progression,
                    quit_idx: r.quit_idx,
                };
                let is_sel = selected.0 == Some(key) || selected.1 == Some(key);
                // Shape by quit rule: circle none, square 1.5×, diamond 2×,
                // triangle 3×; ring style by pressing class.
                draw_combo_dot(cx, Pos2::new(x, y), hue, r.quit_idx, r.progression, is_sel);

                if let Some(p) = pointer {
                    let d2 = (p.x - x).powi(2) + (p.y - y).powi(2);
                    if d2 < 49.0 && best_hover.is_none_or(|(bd, _)| d2 < bd) {
                        best_hover = Some((d2, k));
                    }
                }
            }

            if let Some((_, k)) = best_hover {
                let r = &rows[k];
                // On a pill: the readout sits over the densest dot lanes
                // and is unreadable bare.
                cx.text_pilled(
                    Layer::Overlay,
                    Pos2::new(cx.rect.left() + 8.0, cx.rect.bottom() - 6.0),
                    Align2::LEFT_BOTTOM,
                    format!(
                        "{} · {} · {} — {}",
                        r.strategy,
                        r.progression.label(),
                        quit_label(r.quit),
                        rank.fmt(r)
                    ),
                    FontId::new(type_scale::CAPTION, theme::mono()),
                    t.ink,
                    t.pill(),
                );
                if cx.response.clicked() {
                    let shift = cx.response.ctx.input(|i| i.modifiers.shift);
                    clicked = Some((
                        ComboKey {
                            min_index: mi,
                            min_cents,
                            strategy_idx: r.strategy_idx,
                            progression: r.progression,
                            quit_idx: r.quit_idx,
                        },
                        shift,
                    ));
                }
            }
        });
    if let Some((key, shift)) = clicked {
        select_combo(app, key, shift);
    }
}

fn draw_combo_dot(
    cx: &mut crate::chart::ChartCx<'_>,
    pos: Pos2,
    hue: Color32,
    quit_idx: u8,
    prog: Progression,
    selected: bool,
) {
    let t = cx.theme;
    let r = 2.5f32;
    match quit_idx {
        0 => cx.circle(Layer::Data, pos, r, hue),
        1 => cx.rect_filled(
            Layer::Data,
            egui::Rect::from_center_size(pos, egui::vec2(2.0 * r, 2.0 * r)),
            hue,
        ),
        2 => {
            let pts = vec![
                pos + egui::vec2(0.0, -r * 1.3),
                pos + egui::vec2(r * 1.3, 0.0),
                pos + egui::vec2(0.0, r * 1.3),
                pos + egui::vec2(-r * 1.3, 0.0),
            ];
            cx.shape(
                Layer::Data,
                egui::Shape::convex_polygon(pts, hue, Stroke::NONE),
            );
        }
        _ => {
            let pts = vec![
                pos + egui::vec2(0.0, -r * 1.4),
                pos + egui::vec2(r * 1.4, r * 1.1),
                pos + egui::vec2(-r * 1.4, r * 1.1),
            ];
            cx.shape(
                Layer::Data,
                egui::Shape::convex_polygon(pts, hue, Stroke::NONE),
            );
        }
    }
    // Ring = pressing class: none (flat), thin (positive), double (chase).
    let ring = match prog {
        Progression::Flat => None,
        Progression::Martingale
        | Progression::GrandMartingale
        | Progression::DAlembert
        | Progression::Fibonacci
        | Progression::OscarsGrind => Some(2.0),
        _ => Some(1.0),
    };
    if let Some(w) = ring {
        cx.shape(
            Layer::Data,
            egui::Shape::circle_stroke(pos, r + 2.0, Stroke::new(w * 0.6, hue)),
        );
    }
    if selected {
        cx.shape(
            Layer::Overlay,
            egui::Shape::circle_stroke(pos, r + 4.0, Stroke::new(1.0, t.blue)),
        );
    }
}

/// An 11-hue categorical ramp reserved to this screen, lightness-varied.
fn family_color(t: &crate::ui::theme::Theme, idx: u16, n: usize) -> Color32 {
    let k = idx as f32 / n.max(1) as f32;
    let (h, s) = (k * 330.0, 0.45 + 0.15 * ((idx % 3) as f32));
    let l = if t.dark() {
        0.62 + 0.12 * ((idx % 2) as f32)
    } else {
        0.38 + 0.10 * ((idx % 2) as f32)
    };
    hsl(h, s, l)
}

fn hsl(h: f32, s: f32, l: f32) -> Color32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    Color32::from_rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

fn quit_label(q: Option<f64>) -> String {
    match q {
        Some(m) => format!("quit at {m:.1}×"),
        None => "no quit".to_owned(),
    }
}

fn select_combo(app: &mut App, key: ComboKey, shift: bool) {
    if shift && app.explorer_ui.selected_a.is_some() {
        app.explorer_ui.selected_b = Some(key);
    } else {
        app.explorer_ui.selected_a = Some(key);
        app.explorer_ui.selected_b = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn leaderboard(
    app: &mut App,
    ui: &mut egui::Ui,
    mi: usize,
    min_cents: i64,
    rows: &[ExploreRow],
    rank: RankBy,
    cfg: &crate::config::SimConfig,
) {
    let t = app.theme.clone();
    let mut sorted: Vec<&ExploreRow> = rows.iter().collect();
    sorted.sort_by(|a, b| {
        rank.key(b)
            .partial_cmp(&rank.key(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.mean_final
                    .partial_cmp(&a.mean_final)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    if sorted.is_empty() {
        return;
    }

    // Tie bracket: rows whose interval overlaps the leader's.
    let leader_iv = rank.interval(sorted[0]);
    let tied: Vec<bool> = sorted
        .iter()
        .map(|r| {
            let iv = rank.interval(r);
            iv.1 >= leader_iv.0 && leader_iv.1 >= iv.0
        })
        .collect();
    let tie_count = tied.iter().take_while(|&&x| x).count();

    let strategies = explore_strategies();
    let rules = cfg.rules();
    let take = sorted.len().min(10);
    let row_h = 18.0f32;
    let header_h = 20.0f32;
    let tie_note_h = if tie_count > 1 { 18.0 } else { 0.0 };
    let frame_h = header_h + take as f32 * row_h + tie_note_h + 8.0;

    // The exportable leaderboard: a ChartFrame whose body draws rows and
    // the tie bracket as shapes, so ⌘⇧C/⌘⇧S carry it with provenance —
    // and rows still click-select for the Duel.
    let id = egui::Id::new(("leaderboard", mi));
    let mut clicked: Option<(ComboKey, bool)> = None;
    let mono11 = FontId::new(type_scale::CAPTION, theme::mono());
    let head_font = FontId::new(type_scale::CAPTION, theme::sans_semibold());
    let n_per_combo = sorted[0].sessions;
    ChartFrame::new(id, "Leaderboard — top 10")
        .provenance(format!(
            "n = {}/combo · objective: {}",
            crate::ui::numerals::compact_n(n_per_combo),
            rank.label()
        ))
        .height(frame_h)
        .interactive(false)
        .clickable(true)
        .show(ui, &t, |cx| {
            // Column plan: label columns then the five objectives + EDGE.
            let x0 = cx.rect.left() + 26.0;
            let cols: [(f32, &str); 10] = [
                (0.0, "#"),
                (34.0, "Strategy"),
                (258.0, "Pressing"),
                (392.0, "Quit"),
                (486.0, "≥2×"),
                (586.0, "Ahead"),
                (686.0, "No bust"),
                (786.0, "Median end"),
                (890.0, "Mean end"),
                (990.0, "EDGE"),
            ];
            for (dx, h) in cols {
                cx.text(
                    Layer::Grid,
                    Pos2::new(x0 + dx, cx.rect.top() + 2.0),
                    Align2::LEFT_TOP,
                    h,
                    head_font.clone(),
                    if h == "EDGE" { t.amber } else { t.ink2 },
                );
            }
            let pointer = cx.response.hover_pos();
            for (k, r) in sorted.iter().take(take).enumerate() {
                let y = cx.rect.top() + header_h + k as f32 * row_h;
                let key = ComboKey {
                    min_index: mi,
                    min_cents,
                    strategy_idx: r.strategy_idx,
                    progression: r.progression,
                    quit_idx: r.quit_idx,
                };
                let is_sel = app.explorer_ui.selected_a == Some(key)
                    || app.explorer_ui.selected_b == Some(key);
                let ink = if is_sel { t.blue } else { t.ink };
                let rank_text = if tied[k] {
                    "T-1".to_owned()
                } else {
                    format!("{}", k + 1)
                };
                let cells: [(f32, String); 9] = [
                    (0.0, rank_text),
                    (34.0, r.strategy.chars().take(30).collect()),
                    (258.0, r.progression.label().to_owned()),
                    (392.0, quit_label(r.quit)),
                    (486.0, numerals::prob_ci(r.p_double, r.p_double_ci)),
                    (586.0, numerals::prob_ci(r.p_ahead, r.p_ahead_ci)),
                    (686.0, numerals::prob_ci(r.p_nobust, r.p_nobust_ci)),
                    (786.0, numerals::money_text(r.median_final, false)),
                    (890.0, numerals::money_text(r.mean_final as i64, false)),
                ];
                for (dx, text) in cells {
                    cx.text(
                        Layer::Data,
                        Pos2::new(x0 + dx, y),
                        Align2::LEFT_TOP,
                        text,
                        mono11.clone(),
                        ink,
                    );
                }
                // The anti-snake-oil column: identical and negative for
                // every pressing of the same bets; never hidden.
                let sel = &strategies[r.strategy_idx as usize].1;
                let edge = blended_edge(sel, &rules, min_cents)
                    .map(numerals::edge_pct)
                    .unwrap_or_else(|| "—".to_owned());
                cx.text(
                    Layer::Data,
                    Pos2::new(x0 + 990.0, y),
                    Align2::LEFT_TOP,
                    edge,
                    mono11.clone(),
                    t.amber,
                );
                if let Some(p) = pointer {
                    if p.y >= y && p.y < y + row_h && cx.response.clicked() {
                        let shift = cx.response.ctx.input(|i| i.modifiers.shift);
                        clicked = Some((key, shift));
                    }
                }
            }
            // The brass tie bracket: a drawn enclosure, not a footnote —
            // and it ships inside every export of this frame.
            if tie_count > 1 {
                let y0 = cx.rect.top() + header_h - 2.0;
                let y1 = y0 + tie_count.min(take) as f32 * row_h + 2.0;
                let bx = cx.rect.left() + 14.0;
                cx.line(
                    Layer::Annotation,
                    vec![
                        Pos2::new(bx + 5.0, y0),
                        Pos2::new(bx, y0),
                        Pos2::new(bx, y1),
                        Pos2::new(bx + 5.0, y1),
                    ],
                    egui::Stroke::new(2.0, t.amber),
                );
                cx.text(
                    Layer::Annotation,
                    Pos2::new(x0, cx.rect.top() + header_h + take as f32 * row_h + 2.0),
                    Align2::LEFT_TOP,
                    format!(
                        "Rank 1 — statistically tied ({tie_count} combos) · n = {}/combo — increase sessions to separate",
                        crate::ui::numerals::compact_n(n_per_combo)
                    ),
                    mono11.clone(),
                    t.amber,
                );
            }
        });
    if let Some((key, shift)) = clicked {
        select_combo(app, key, shift);
    }

    // Column-header re-rank: the five objective headers double as rank
    // buttons ("clicking a header re-ranks instantly", spec §6.6).
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("re-rank by:")
                .font(FontId::new(type_scale::CAPTION, theme::sans()))
                .color(t.ink2),
        );
        for (r, short) in [
            (RankBy::DoubleUp, "≥2×"),
            (RankBy::Ahead, "Ahead"),
            (RankBy::Survive, "No bust"),
            (RankBy::MedianFinal, "Median end"),
            (RankBy::MeanFinal, "Mean end"),
        ] {
            if ui.selectable_label(rank == r, short).clicked() {
                app.explorer_ui.rank_by = Some(r);
            }
        }
    });

    // The full table stays reachable (interactive grid, not exported).
    let more = sorted.len().saturating_sub(take);
    if more > 0 || app.explorer_ui.show_all {
        ui.horizontal(|ui| {
            let icon = if app.explorer_ui.show_all {
                crate::ui::icons::Icon::ChevronDown
            } else {
                crate::ui::icons::Icon::ChevronRight
            };
            let label = if app.explorer_ui.show_all {
                "show top 10 only".to_owned()
            } else {
                format!("show all {}", sorted.len())
            };
            let chev = crate::ui::icons::button(ui, icon, t.ink2, t.blue, "expand / collapse");
            if chev.clicked() || ui.link(label).clicked() {
                app.explorer_ui.show_all = !app.explorer_ui.show_all;
            }
        });
        if app.explorer_ui.show_all {
            egui::ScrollArea::horizontal()
                .id_salt(("leader_scroll", mi))
                .show(ui, |ui| {
                    egui::Grid::new(("leader_all", mi))
                        .striped(true)
                        .spacing([14.0, 3.0])
                        .show(ui, |ui| {
                            for r in sorted.iter() {
                                for text in [
                                    r.strategy.to_owned(),
                                    r.progression.label().to_owned(),
                                    quit_label(r.quit),
                                    rank.fmt(r),
                                ] {
                                    ui.label(
                                        RichText::new(text)
                                            .font(FontId::new(type_scale::CAPTION, theme::mono()))
                                            .color(t.ink),
                                    );
                                }
                                ui.end_row();
                            }
                        });
                });
        }
    }
}
