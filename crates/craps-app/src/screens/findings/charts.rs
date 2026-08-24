// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Composition: feeds store data into the chart painters inside their
//! frames. All exact interactive queries hit the focused minimum's sorted
//! views; overlays for other minimums draw from quantile digests.

use egui::Id;

use craps_engine::{flat_drift_per_roll_cents, Progression, QUANTILE_POINTS};

use crate::app::App;
use crate::chart::frame::ChartFrame;
use crate::chart::{brush, dotfield, fan as fan_chart, histogram, ladder, survival};
use crate::store::{MinRun, RunStore};
use crate::ui::numerals;

/// Survival points from a quantile table: (value at rank, survival).
fn survival_points_from_q(q: &[i64; QUANTILE_POINTS]) -> Vec<(f64, f64)> {
    q.iter()
        .enumerate()
        .map(|(k, &v)| (v as f64, 1.0 - k as f64 / (QUANTILE_POINTS - 1) as f64))
        .collect()
}

/// Streaming fallback: survival from a StreamHist's cumulative counts.
fn survival_points_from_hist(h: &craps_engine::StreamHist) -> Vec<(f64, f64)> {
    let total = h.total().max(1) as f64;
    let mut cum = 0u64;
    let mut pts = Vec::with_capacity(h.counts().len());
    for (i, &c) in h.counts().iter().enumerate() {
        cum += c as u64;
        pts.push((
            ((i as i64 + 1) * h.width()) as f64,
            1.0 - cum as f64 / total,
        ));
    }
    pts
}

/// Q1 hero: the survival curve, every minimum overlaid in its chip color.
pub fn survival_chart(app: &mut App, ui: &mut egui::Ui, st: &RunStore, focused: usize) {
    let t = app.theme.clone();
    let p = &st.provenance.config;
    let rph = p.rolls_per_hour as f64;
    let cap = p.max_rolls as f64;

    let build_series = |m: &MinRun, is_focused: bool| -> survival::SurvivalSeries {
        // The focused minimum draws from its exact sorted rolls when built;
        // overlays draw from 1024-point quantile digests (visually exact).
        let points = match (is_focused, &m.sorted, &m.digests.rolls_q) {
            (true, Some(sv), _) if !sv.rolls.is_empty() => {
                let n = sv.rolls.len() as f64;
                let step = (sv.rolls.len() / 2048).max(1);
                sv.rolls
                    .iter()
                    .step_by(step)
                    .enumerate()
                    .map(|(k, &r)| (r as f64, 1.0 - (k * step) as f64 / n))
                    .collect()
            }
            (_, _, Some(q)) => survival_points_from_q(q),
            _ => survival_points_from_hist(&m.digests.rolls_hist),
        };
        survival::SurvivalSeries {
            color: t.chip(m.min_cents),
            label: numerals::money_text(m.min_cents, false),
            points,
            focused: is_focused,
        }
    };

    let mins: Vec<usize> = if app.small_multiples {
        (0..st.mins.len()).collect()
    } else {
        vec![usize::MAX] // sentinel: one overlaid panel
    };

    for panel in mins {
        let overlay = panel == usize::MAX;
        let series: Vec<survival::SurvivalSeries> = if overlay {
            st.mins
                .iter()
                .enumerate()
                .map(|(i, m)| build_series(m, i == focused))
                .collect()
        } else {
            vec![build_series(&st.mins[panel], true)]
        };
        // Clip the domain at the 0.5% survival tail: the shape stays whole
        // and the interesting region fills the panel; the censor hatch pins
        // at the cap when sessions survived it.
        let x_max = series
            .iter()
            .map(|s| x_at_survival(&s.points, 0.005))
            .fold(1.0f64, f64::max)
            .mul_add(1.05, 0.0)
            .min(cap);

        let focus_ix = if overlay { focused } else { panel };
        let fm = &st.mins[focus_ix];
        let droplines = fm
            .summary
            .as_ref()
            .map(|s| {
                vec![
                    (s.ruin.p5 as f64, format!("P5 {}", s.ruin.p5)),
                    (s.ruin.p25 as f64, format!("P25 {}", s.ruin.p25)),
                    (s.ruin.p50 as f64, format!("median {}", s.ruin.p50)),
                    (s.ruin.p75 as f64, format!("P75 {}", s.ruin.p75)),
                    (s.ruin.p95 as f64, format!("P95 {}", s.ruin.p95)),
                ]
            })
            .unwrap_or_default();
        let censored = fm.summary.as_ref().and_then(|s| {
            (s.ruin.censored_frac > 0.0).then(|| {
                (
                    cap,
                    s.ruin.censored_frac,
                    format!("survived cap — {:.1}%", s.ruin.censored_frac * 100.0),
                )
            })
        });

        let id = Id::new(("survival", panel));
        let frame = ChartFrame::new(
            id,
            if overlay {
                "Survival — sessions still solvent"
            } else {
                &series[0].label
            },
        )
        .run_key(st.provenance.run_key())
        .provenance(super::provenance_line(st))
        .badge(super::run_badge(app, st))
        .height(if overlay { 300.0 } else { 190.0 })
        .interactive(overlay);
        let probe_series: Vec<(String, Vec<(f64, f64)>)> = series
            .iter()
            .map(|s| (s.label.clone(), s.points.clone()))
            .collect();
        frame.show(ui, &t, |cx| {
            let mut probe_labels = Vec::new();
            if let Some(px) = cx.probe() {
                probe_labels.push(format!(
                    "after {} rolls ({})",
                    px as u64,
                    numerals::hours(px, rph)
                ));
                for (label, pts) in &probe_series {
                    let s = survival_at(pts, px);
                    probe_labels.push(format!("{label}  {:.0}% alive", s * 100.0));
                }
            }
            let opt = survival::SurvivalOptions {
                x_max,
                money_x: false,
                droplines: droplines.clone(),
                censored: censored.clone(),
                hours_per_x: Some(1.0 / rph),
                probe_labels,
            };
            survival::paint(cx, &series, &opt);
        });
    }
}

/// The x where a descending survival curve first reaches `level`.
fn x_at_survival(points: &[(f64, f64)], level: f64) -> f64 {
    points
        .iter()
        .find(|&&(_, sv)| sv <= level)
        .or(points.last())
        .map(|&(x, _)| x)
        .unwrap_or(1.0)
}

fn survival_at(points: &[(f64, f64)], x: f64) -> f64 {
    match points.binary_search_by(|&(px, _)| px.partial_cmp(&x).unwrap()) {
        Ok(i) => points[i].1,
        Err(0) => 1.0,
        Err(i) => points[i - 1].1,
    }
}

/// Q2 hero: the final-bankroll histogram with the sacred bust bar, the
/// pre-drawn House Line, mean/median markers, the P(ahead) hairline, the
/// E-morph to ECDF, and the decile ladder beneath.
/// The bar under the probe: its dollar range, count, and one real session
/// inside it — the door from a bar into Replay (spec §6.4).
pub struct HoverBin {
    pub lo_cents: i64,
    pub hi_cents: i64,
    pub count: u32,
    pub session: Option<u64>,
}

pub fn horizon_chart(
    app: &mut App,
    ui: &mut egui::Ui,
    st: &RunStore,
    focused: usize,
) -> Option<HoverBin> {
    let mut hover_bin: Option<HoverBin> = None;
    let t = app.theme.clone();
    let p = st.provenance.config.clone();
    let budget = p.budget_cents;

    let panels: Vec<usize> = if app.small_multiples {
        (0..st.mins.len()).collect()
    } else {
        vec![focused]
    };

    // The invariant the Anchor displays: the per-dollar edge of this
    // selection, pinned through every progression swap.
    let focused_min_cents = st.mins[focused].min_cents;
    let pinned_edge = craps_engine::blended_edge(&p.sel, &p.rules(), focused_min_cents);

    for mi in panels {
        let m = &st.mins[mi];

        // The Anchor: hot-swap the progression on the focused panel only.
        let run_prog = p.sel.progression;
        let mut variant: Option<crate::store::AnchorVariant> = None;
        let mut display_prog = run_prog;
        if mi == focused && !app.small_multiples && m.summary.is_some() {
            let seed = st.provenance.seed;
            display_prog = super::anchor::control(app, ui, &p, seed, mi, run_prog);
            if display_prog != run_prog {
                variant = super::anchor::variant_for(app, &p, seed, mi, display_prog);
                if variant.is_none() {
                    // A real progress fill — the swap is a redraw of real
                    // data, never a fake instant re-simulation.
                    let frac = super::anchor::job_progress(app, mi, display_prog);
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .text(format!(
                                "re-simulating {} on identical dice — {:.0}%",
                                display_prog.label(),
                                frac * 100.0
                            ))
                            .animate(false),
                    );
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }

        let (mut counts, bin_width, busts, n, mean, mean_ci, median, loss_pin_handle) =
            match &variant {
                Some(v) => (
                    v.counts.clone(),
                    v.bin_width,
                    v.busts,
                    v.n,
                    v.mean,
                    v.mean_ci,
                    Some(v.median as f64),
                    Some(v.mean_handle_cents),
                ),
                None => (
                    m.digests.finals_hist.counts().to_vec(),
                    m.digests.finals_hist.width(),
                    m.digests.busted,
                    m.digests.finals_hist.total(),
                    m.digests.finals_mean.mean(),
                    m.digests.finals_mean.ci95_half(),
                    m.summary.as_ref().map(|s| s.horizon.deciles[4] as f64),
                    (m.digests.horizon_rolls_sum > 0).then(|| {
                        m.digests.handle_cents_sum as f64
                            / m.digests.finals_hist.total().max(1) as f64
                    }),
                ),
            };
        // The bust spike is its own object: pull the exact-$0 sessions out
        // of bin zero so the continuous mass never absorbs ruin.
        counts[0] = counts[0].saturating_sub(busts.min(counts[0] as u64) as u32);
        let bust_frac = if n > 0 { busts as f64 / n as f64 } else { 0.0 };

        // The Rain: while streaming, the focused histogram interpolates
        // between REAL successive 10 Hz snapshots — clamped, never
        // extrapolated; reduced motion shows raw snapshots (Principle 4).
        if mi == focused
            && variant.is_none()
            && matches!(st.status, crate::store::RunStatus::Streaming { .. })
        {
            let now = ui.input(|i| i.time);
            if n != app.rain_mark.0 {
                let gap = (now - app.rain_mark.1).clamp(0.02, 0.5) as f32;
                app.rain
                    .push(counts.iter().map(|&c| c as f32).collect(), gap);
                app.rain_mark = (n, now);
            }
            app.rain.tick(ui.input(|i| i.stable_dt.min(0.1)));
            if !app.rain.is_settled() {
                ui.ctx().request_repaint();
            }
            let vals = app.rain.values(app.motion);
            if vals.len() == counts.len() {
                for (c, v) in counts.iter_mut().zip(vals) {
                    *c = v.round().max(0.0) as u32;
                }
            }
        }

        // The House Line draws only where the engine computes a closed form:
        // flat stakes, no quit target (spec scope guard).
        let flat_no_quit = display_prog == Progression::Flat && p.quit_mult.is_none();
        let house_line = flat_no_quit.then(|| {
            let drift = flat_drift_per_roll_cents(&p.sel, &p.rules(), m.min_cents);
            let expected = budget as f64 + drift * p.horizon_rolls() as f64;
            (
                expected,
                " expected mean — closed form, unbounded bankroll".to_owned(),
            )
        });
        let house_gap_caption = house_line.as_ref().and_then(|(hl, _)| {
            (bust_frac >= 0.02).then(|| {
                format!(
                    "Busted nights stop losing: the empirical mean sits above the drift line by {}.",
                    numerals::money_text((mean - hl) as i64, false)
                )
            })
        });

        // The Anchor's two pins: the per-dollar edge label never moves;
        // the expected-dollar-loss pin drifts with the simulated handle.
        let pinned_edge_label = (mi == focused).then(|| {
            pinned_edge
                .map(|e| format!("house edge: {} per resolved dollar", numerals::edge_pct(e)))
                .unwrap_or_default()
        });
        let loss_pin = match (pinned_edge, loss_pin_handle) {
            (Some(e), Some(h)) if mi == focused => Some((
                budget as f64 + e * h,
                format!(
                    "expected end (edge × handle): {}",
                    numerals::money_text((budget as f64 + e * h) as i64, false)
                ),
            )),
            _ => None,
        };
        if variant.is_some() {
            ui.label(
                egui::RichText::new(super::anchor::drift_caption())
                    .font(egui::FontId::new(11.0, crate::ui::theme::sans()))
                    .color(t.ink2),
            );
        }

        // Anchor crossfade: two complete honest renders, 400 ms.
        let swap_alpha = match app.anchor.swapped_at {
            Some(t0) if mi == focused => {
                let dur = app.motion.duration(crate::ui::motion::MORPH).max(1e-6);
                let a = ((ui.input(|i| i.time) - t0) as f32 / dur).clamp(0.0, 1.0);
                if a < 1.0 {
                    ui.ctx().request_repaint();
                }
                crate::ui::motion::settle(a)
            }
            _ => 1.0,
        };
        let prev_variant = (swap_alpha < 1.0)
            .then(|| app.anchor.prev.clone())
            .flatten();

        // P(ahead): exact from sorted views when focused and built.
        let ahead = if mi == focused {
            m.sorted.as_ref().map(|sv| {
                let frac = crate::store::SortedViews::frac_at_least(&sv.finals, budget + 1);
                (budget as f64, format!("P(ahead) = {:.1}%", frac * 100.0))
            })
        } else {
            None
        };

        // E-morph: a crossfade of two complete honest renderings.
        let target = if app.ecdf_mode { 1.0 } else { 0.0 };
        let ecdf_alpha = app.tweens.drive(
            Id::new(("ecdf_alpha", mi)),
            target,
            app.motion.duration(crate::ui::motion::MORPH),
            ui.input(|i| i.stable_dt.min(0.1)),
        );

        let id = Id::new(("horizon_hist", mi));
        let brush_sorted = (mi == focused && variant.is_none())
            .then(|| m.sorted.as_ref().map(|sv| sv.finals.clone()))
            .flatten();
        let title = if display_prog != run_prog && variant.is_none() {
            format!(
                "Ending bankroll — {} table · {} (still {})",
                numerals::money_text(m.min_cents, false),
                display_prog.label(),
                run_prog.label()
            )
        } else {
            format!(
                "Ending bankroll — {} table · {}",
                numerals::money_text(m.min_cents, false),
                display_prog.label()
            )
        };
        let frame = ChartFrame::new(id, &title)
            .run_key(st.provenance.run_key())
            .provenance(super::provenance_line(st))
            .badge(super::run_badge(app, st))
            .height(if app.small_multiples { 200.0 } else { 300.0 })
            .interactive(mi == focused);
        let lead_lit = app.highlights.is_lit(egui::Id::new("horizon_lead"));
        let mut probe_below_budget = false;
        frame.show(ui, &t, |cx| {
            if let Some(px) = cx.probe() {
                if px >= 0.0 && px < budget as f64 {
                    probe_below_budget = true;
                }
                // The hovered bar: count + a real session behind it.
                if mi == focused && px >= 0.0 {
                    let bin = (px as i64 / bin_width).clamp(0, counts.len() as i64 - 1) as usize;
                    let lo = bin as i64 * bin_width;
                    let hi = lo + bin_width;
                    let session = brush_sorted.as_ref().and_then(|sv| {
                        let a = sv.partition_point(|&v| v < lo);
                        let b = sv.partition_point(|&v| v < hi);
                        (a < b)
                            .then(|| {
                                let rank = (a + b) / 2;
                                st.mins[mi]
                                    .sorted
                                    .as_ref()
                                    .map(|views| st.true_session(views.finals_rank_to_id[rank]))
                            })
                            .flatten()
                    });
                    hover_bin = Some(HoverBin {
                        lo_cents: lo,
                        hi_cents: hi,
                        count: counts[bin],
                        session,
                    });
                }
            }
            // The outgoing shape, fading — a complete honest render.
            if let Some(pv) = &prev_variant {
                let mut pc = pv.counts.clone();
                pc[0] = pc[0].saturating_sub(pv.busts.min(pc[0] as u64) as u32);
                let pd = histogram::HistogramData {
                    counts: &pc,
                    bin_width: pv.bin_width,
                    busts: 0,
                    bust_frac: 0.0,
                };
                let po = histogram::HistogramOverlays {
                    house_line: None,
                    house_gap_caption: None,
                    mean_ci: None,
                    median: None,
                    ahead_line: None,
                    budget_cents: budget,
                    log_y: app.log_y,
                    ecdf_alpha,
                    brush_label: None,
                    fill_alpha: 1.0 - swap_alpha,
                    pinned_edge_label: None,
                    loss_pin: None,
                };
                histogram::paint(cx, &pd, &po);
            }
            let brush_label = cx.brush().and_then(|(a, b)| {
                brush_sorted
                    .as_ref()
                    .map(|sv| brush::mass_label_money(sv, a as i64, b as i64))
            });
            let d = histogram::HistogramData {
                counts: &counts,
                bin_width,
                busts,
                bust_frac,
            };
            let o = histogram::HistogramOverlays {
                house_line: house_line.clone(),
                house_gap_caption: house_gap_caption.clone(),
                mean_ci: Some((mean, mean_ci)),
                median,
                ahead_line: ahead.clone(),
                budget_cents: budget,
                log_y: app.log_y,
                ecdf_alpha,
                brush_label,
                fill_alpha: if prev_variant.is_some() {
                    swap_alpha
                } else {
                    1.0
                },
                pinned_edge_label: pinned_edge_label.clone().filter(|s| !s.is_empty()),
                loss_pin: loss_pin.clone(),
            };
            histogram::paint(cx, &d, &o);

            // Everything below reads cx.x, so it must run AFTER paint has
            // set the real domain (before it, cx.x is the frame's 0..1
            // placeholder and screen positions land astronomically off).

            // Bidirectional card linkage: when the lead insight card is
            // hovered, tint the below-budget mass it describes (layers
            // flush by enum order, so a Ribbon emitted here still renders
            // beneath the bars).
            if lead_lit && mi == focused {
                let x0 = cx.x.to_screen(0.0);
                let x1 = cx.x.to_screen(budget as f64);
                cx.rect_filled(
                    crate::chart::Layer::Ribbon,
                    egui::Rect::from_min_max(
                        egui::Pos2::new(x0, cx.y.r1),
                        egui::Pos2::new(x1, cx.baseline()),
                    ),
                    t.ci_band(t.blue),
                );
            }

            // Pinned probes read exact mass below each pin (the frame draws
            // the tags and value pills up top; the mass readout sits at the
            // plot foot, clear of them).
            if let Some(sv) = brush_sorted.as_ref() {
                let base = cx.baseline();
                for (k, &px) in cx.pins().to_vec().iter().enumerate() {
                    let below = sv.partition_point(|&v| (v as f64) < px);
                    let frac = below as f64 / sv.len().max(1) as f64;
                    cx.text_pilled(
                        crate::chart::Layer::Overlay,
                        egui::Pos2::new(cx.x.to_screen(px) + 4.0, base - 18.0 - 14.0 * k as f32),
                        egui::Align2::LEFT_BOTTOM,
                        format!("pin {}: {:.1}% of nights end below", k + 1, frac * 100.0),
                        egui::FontId::new(11.0, crate::ui::theme::mono()),
                        t.blue,
                        t.pill(),
                    );
                }
            }
        });
        if probe_below_budget {
            app.highlights.set(egui::Id::new("horizon_lead"));
        }

        // Decile ladder beneath each money histogram.
        if let Some(s) = &m.summary {
            let lid = Id::new(("ladder", mi));
            ChartFrame::new(lid, "Deciles")
                .height(64.0)
                .interactive(false)
                .show(ui, &t, |cx| {
                    ladder::paint(
                        cx,
                        &ladder::Ladder {
                            deciles: &s.horizon.deciles,
                            mean: s.horizon.mean,
                            budget_cents: budget,
                        },
                    );
                });
        }
    }
    hover_bin
}

/// The wealth fan: the one inside the many.
pub fn fan_chart_ui(app: &mut App, ui: &mut egui::Ui, st: &RunStore, focused: usize) {
    let t = app.theme.clone();
    let p = &st.provenance.config;
    let m = &st.mins[focused];
    let Some(fan) = &m.fan else {
        if m.summary.is_some() {
            ui.label(
                egui::RichText::new("assembling trajectories…")
                    .color(t.ink2)
                    .italics(),
            );
        }
        return;
    };
    let id = Id::new(("fan", focused));
    let frame = ChartFrame::new(id, "The wealth fan — 1,000 nights traced")
        .run_key(st.provenance.run_key())
        .provenance(format!(
            "sampled n = {} of {} · {}",
            numerals::compact_n(fan.sampled_n),
            numerals::compact_n(fan.total_n),
            super::provenance_line(st)
        ))
        .badge(super::run_badge(app, st))
        .height(260.0);
    let budget = p.budget_cents;
    let hours_per_roll = 1.0 / p.rolls_per_hour as f64;
    frame.show(ui, &t, |cx| {
        fan_chart::paint(cx, fan, budget, hours_per_roll);
    });
}

/// Q3 hero: the peak-outlay survival curve with the draggable budget line,
/// the confidence handle, and the dot field beneath. Returns a session to
/// replay if a dot was clicked.
pub fn stake_chart(app: &mut App, ui: &mut egui::Ui, st: &RunStore, focused: usize) -> Option<u64> {
    let t = app.theme.clone();
    let p = st.provenance.config.clone();
    let m = &st.mins[focused];

    // Survival-of-budget: P(outlay ≤ b) per minimum.
    let series: Vec<survival::SurvivalSeries> = st
        .mins
        .iter()
        .enumerate()
        .map(|(i, mm)| {
            let pts: Vec<(f64, f64)> = match &mm.digests.outlays_q {
                Some(q) => q
                    .iter()
                    .enumerate()
                    .map(|(k, &v)| (v as f64, k as f64 / (QUANTILE_POINTS - 1) as f64))
                    .collect(),
                None => {
                    let h = &mm.digests.outlays_hist;
                    let total = h.total().max(1) as f64;
                    let mut cum = 0u64;
                    h.counts()
                        .iter()
                        .enumerate()
                        .map(|(k, &c)| {
                            cum += c as u64;
                            (((k as i64 + 1) * h.width()) as f64, cum as f64 / total)
                        })
                        .collect()
                }
            };
            survival::SurvivalSeries {
                color: t.chip(mm.min_cents),
                label: numerals::money_text(mm.min_cents, false),
                points: pts,
                focused: i == focused,
            }
        })
        .collect();

    // Clip at the 99.5% coverage point: the tail of one catastrophic
    // outlay must not crush the curves (it stays reachable by zooming out
    // with a brush or reading the ledger).
    let x_max = series
        .iter()
        .map(|s| {
            s.points
                .iter()
                .find(|&&(_, c)| c >= 0.995)
                .or(s.points.last())
                .map(|&(x, _)| x)
                .unwrap_or(1.0)
        })
        .fold(1.0f64, f64::max)
        * 1.1;

    // The exploration line starts at the configured budget.
    let line_budget = app.stake_budget_probe.unwrap_or(p.budget_cents);
    let confidence = app.stake_confidence.unwrap_or(p.confidence);

    // Sampled dots: deterministic stride over written sessions.
    let dots: Vec<(u32, i64)> = {
        let written = m.cols.written();
        if written == 0 {
            Vec::new()
        } else {
            let stride = (written / 1000).max(1);
            m.cols
                .written_indices()
                .step_by(stride as usize)
                .take(1000)
                .map(|i| (i as u32, m.cols.outlays[i]))
                .collect()
        }
    };
    let sampled_n = dots.len() as u64;

    let focused_pts = &series[focused].points;
    let survive_at = |b: f64| -> f64 {
        match focused_pts.binary_search_by(|&(px, _)| px.partial_cmp(&b).unwrap()) {
            Ok(i) => focused_pts[i].1,
            Err(0) => 0.0,
            Err(i) => focused_pts[i - 1].1,
        }
    };

    let id = Id::new("stake_survival");
    let mut new_budget_probe = None;
    let mut new_confidence = None;
    let mut clicked_dot = None;
    let field = dotfield::DotField { dots: &dots };
    let frame = ChartFrame::new(id, "Budget vs. fate — peak outlay survival")
        .run_key(st.provenance.run_key())
        .story(format!(
            "{} survives the {:.0}-hour session in {:.1}% of nights.",
            numerals::money_text(line_budget, false),
            p.target_hours,
            survive_at(line_budget as f64) * 100.0
        ))
        .provenance(format!(
            "dots: sampled n = {} of {} · {}",
            numerals::compact_n(sampled_n),
            numerals::compact_n(m.cols.written()),
            super::provenance_line(st)
        ))
        .badge(super::run_badge(app, st))
        .height(340.0);
    let inverse_lit = app.highlights.is_lit(egui::Id::new("stake_inverse"));
    let mut probe_on_actual = false;
    let resp = frame.show(ui, &t, |cx| {
        // Reserve the lower band for the dot field.
        let band_h = 64.0;
        let full = cx.rect;
        let plot_bottom = full.bottom() - band_h;
        cx.rect.set_bottom(plot_bottom);
        cx.hint_bottom = Some(plot_bottom);
        let opt = survival::SurvivalOptions {
            x_max,
            money_x: true,
            droplines: vec![],
            censored: None,
            hours_per_x: None,
            probe_labels: vec![],
        };
        survival::paint(cx, &series, &opt);

        // Pre-marked confidence dots at 90/95/99 on the focused curve. Their
        // labels form one fixed column beside the rightmost (99%) dot — the
        // curve's slope makes per-dot label offsets collide on shallow
        // shoulders, and the region right of the 99% dot is always clear.
        let dots: Vec<(f64, f64, bool)> = [(90.0, false), (95.0, true), (99.0, false)]
            .into_iter()
            .map(|(c, big)| (c, budget_at_confidence(focused_pts, c / 100.0), big))
            .collect();
        let col = dots
            .iter()
            .map(|&(c, b, _)| cx.xy(b, c / 100.0))
            .fold(egui::Pos2::new(f32::MIN, f32::MAX), |acc, p| {
                egui::Pos2::new(acc.x.max(p.x), acc.y.min(p.y))
            });
        for (k, &(c, b, big)) in dots.iter().enumerate() {
            let pos = cx.xy(b, c / 100.0);
            cx.circle(
                crate::chart::Layer::Estimate,
                pos,
                if big { 4.0 } else { 3.0 },
                t.ink,
            );
            cx.text_pilled(
                crate::chart::Layer::Annotation,
                egui::Pos2::new(col.x + 10.0, col.y + 13.0 * k as f32),
                egui::Align2::LEFT_TOP,
                format!("{c:.0}% → {}", numerals::money_text(b as i64, false)),
                egui::FontId::new(11.0, crate::ui::theme::mono()),
                t.ink2,
                t.pill(),
            );
        }

        // The draggable budget line (amber-edged, grabbable anywhere along
        // its height). The drag LATCHES at drag-start so a fast pull cannot
        // slip off the line mid-gesture.
        let lx = cx.x.to_screen(line_budget as f64);
        let latch_key = egui::Id::new("stake_line_latched");
        let near_line = cx
            .response
            .hover_pos()
            .is_some_and(|hp| (hp.x - lx).abs() < 8.0);
        let mut latched: bool = cx
            .response
            .ctx
            .data(|d| d.get_temp(latch_key))
            .unwrap_or(false);
        if cx.response.drag_started() && near_line {
            latched = true;
        }
        if cx.response.drag_stopped() {
            latched = false;
        }
        cx.response
            .ctx
            .data_mut(|d| d.insert_temp(latch_key, latched));
        let dragging_line = cx.response.dragged() && latched;
        if dragging_line {
            cx.suppress_brush = true;
            if let Some(hp) = cx.response.interact_pointer_pos() {
                let b = cx.x.from_screen(hp.x).max(0.0) as i64;
                new_budget_probe = Some(b);
            }
        }
        cx.vline(
            crate::chart::Layer::Estimate,
            lx,
            egui::Stroke::new(2.0, t.amber),
        );
        cx.text(
            crate::chart::Layer::Annotation,
            egui::Pos2::new(lx + 4.0, cx.rect.top() + 2.0),
            egui::Align2::LEFT_TOP,
            format!(
                "{} → survives in {:.1}%",
                numerals::money_text(line_budget, false),
                survive_at(line_budget as f64) * 100.0
            ),
            egui::FontId::new(11.0, crate::ui::theme::mono()),
            t.amber,
        );

        // Confidence handle riding the y-axis, detents at 90/95/99.
        let hy = cx.y.to_screen(confidence / 100.0);
        let handle = egui::Rect::from_center_size(
            egui::Pos2::new(cx.rect.left() + 46.0, hy),
            egui::vec2(14.0, 14.0),
        );
        cx.circle(crate::chart::Layer::Overlay, handle.center(), 5.0, t.blue);
        if cx.response.dragged() {
            if let Some(hp) = cx.response.interact_pointer_pos() {
                if (hp.x - handle.center().x).abs() < 24.0 && !dragging_line {
                    cx.suppress_brush = true;
                    let mut c = (cx.y.from_screen(hp.y) * 100.0).clamp(50.0, 99.9);
                    for detent in [90.0, 95.0, 99.0] {
                        if (c - detent).abs() < 0.6 {
                            c = detent;
                        }
                    }
                    new_confidence = Some(c);
                }
            }
        }
        let rec = budget_at_confidence(focused_pts, confidence / 100.0);
        cx.hline(
            crate::chart::Layer::Overlay,
            hy,
            egui::Stroke::new(1.0, t.blue),
        );
        cx.text(
            crate::chart::Layer::Overlay,
            egui::Pos2::new(cx.rect.right() - 4.0, hy - 4.0),
            egui::Align2::RIGHT_BOTTOM,
            format!(
                "{confidence:.0}% → {}",
                numerals::money_text(rec as i64, false)
            ),
            egui::FontId::new(11.0, crate::ui::theme::mono()),
            t.blue,
        );

        // Bidirectional linkage with the inverse insight card: its dot on
        // the curve brightens when the card is hovered, and hovering the
        // actual-budget region lights the card back.
        {
            let actual = p.budget_cents as f64;
            let coverage = survive_at(actual);
            let dot = cx.xy(actual, coverage);
            cx.circle(
                crate::chart::Layer::Estimate,
                dot,
                if inverse_lit { 6.0 } else { 3.5 },
                t.blue,
            );
            if inverse_lit {
                cx.vline(
                    crate::chart::Layer::Ribbon,
                    cx.x.to_screen(actual),
                    egui::Stroke::new(3.0, t.ci_band(t.blue)),
                );
            }
            if let Some(px) = cx.probe() {
                if (px - actual).abs() < (cx.x.d1 - cx.x.d0) * 0.02 {
                    probe_on_actual = true;
                }
            }
        }

        // The dot field in the reserved band: lives recolor under the line.
        cx.rect =
            egui::Rect::from_min_max(egui::Pos2::new(full.left(), plot_bottom + 6.0), full.max);
        clicked_dot = dotfield::paint(cx, &field, cx.rect.top(), cx.rect.bottom(), line_budget);
        cx.rect = full;
    });
    if probe_on_actual {
        app.highlights.set(egui::Id::new("stake_inverse"));
    }
    if resp.double_clicked() {
        app.stake_budget_probe = None;
    }
    if let Some(b) = new_budget_probe {
        app.stake_budget_probe = Some(b);
    }
    if let Some(c) = new_confidence {
        app.stake_confidence = Some(c);
    }
    if resp.clicked() {
        if let Some(id) = clicked_dot {
            return Some(st.true_session(id));
        }
    }
    None
}

/// M-toggle small multiples for Stake: one compact survival-of-budget
/// panel per minimum, shared scale, budget line etched — the focused
/// minimum keeps the full interactive chart below.
pub fn stake_small_multiples(app: &mut App, ui: &mut egui::Ui, st: &RunStore, focused: usize) {
    let t = app.theme.clone();
    let p = &st.provenance.config;
    let per_row = st.mins.len().min(3);
    egui::Grid::new("stake_multiples")
        .num_columns(per_row)
        .spacing([8.0, 8.0])
        .show(ui, |ui| {
            for (i, m) in st.mins.iter().enumerate() {
                let pts: Vec<(f64, f64)> = match &m.digests.outlays_q {
                    Some(q) => q
                        .iter()
                        .enumerate()
                        .map(|(k, &v)| (v as f64, k as f64 / (QUANTILE_POINTS - 1) as f64))
                        .collect(),
                    None => continue,
                };
                let x_max = pts
                    .iter()
                    .find(|&&(_, c)| c >= 0.995)
                    .or(pts.last())
                    .map(|&(x, _)| x)
                    .unwrap_or(1.0)
                    * 1.1;
                let series = vec![survival::SurvivalSeries {
                    color: t.chip(m.min_cents),
                    label: numerals::money_text(m.min_cents, false),
                    points: pts,
                    focused: i == focused,
                }];
                let budget = p.budget_cents;
                ChartFrame::new(
                    Id::new(("stake_multi", i)),
                    &numerals::money_text(m.min_cents, false),
                )
                .height(130.0)
                .interactive(false)
                .show(ui, &t, |cx| {
                    let opt = survival::SurvivalOptions {
                        x_max,
                        money_x: true,
                        droplines: vec![],
                        censored: None,
                        hours_per_x: None,
                        probe_labels: vec![],
                    };
                    survival::paint(cx, &series, &opt);
                    cx.vline(
                        crate::chart::Layer::Estimate,
                        cx.x.to_screen(budget as f64),
                        egui::Stroke::new(1.5, t.amber),
                    );
                });
                if (i + 1) % per_row == 0 {
                    ui.end_row();
                }
            }
        });
    ui.add_space(6.0);
}

/// The budget at a survival confidence, from the focused curve's points.
fn budget_at_confidence(points: &[(f64, f64)], conf01: f64) -> f64 {
    for &(x, s) in points {
        if s >= conf01 {
            return x;
        }
    }
    points.last().map(|&(x, _)| x).unwrap_or(0.0)
}
