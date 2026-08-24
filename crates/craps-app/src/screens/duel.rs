// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The Duel — the app's only comparison surface, paired by construction:
//! both strategies replay identical dice (common random numbers). CRN
//! removes common luck, not all sampling noise; the caption says exactly
//! that, and a tie band as large as any verdict stamps indistinguishable
//! results as indistinguishable.

use std::sync::{Arc, Mutex};

use craps_engine::trace::trace_wealth;
use craps_engine::{
    explore_strategies, hexbin, median_ci_sorted, quit_target_cents, run_pair, session_seed,
    HexBin, PairSide, PairedFinal, SeedPhase, SweepCtl, EXPLORE_QUITS,
};
use egui::{FontId, RichText};

use super::explorer::ComboKey;
use crate::app::App;
use crate::chart::frame::ChartFrame;
use crate::chart::{hexbin as hexbin_chart, histogram, ladder, trajectory};
use crate::config::SimConfig;
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale};

pub struct DuelData {
    pub diff_counts: Vec<u32>,
    pub diff_origin: i64,
    pub diff_width: i64,
    pub median_delta: i64,
    pub median_ci: (i64, i64),
    pub tie: bool,
    /// (a higher, b higher, within $1).
    pub quadrants: (u64, u64, u64),
    pub deciles_a: [i64; 9],
    pub deciles_b: [i64; 9],
    pub hexbins: Vec<HexBin>,
    pub extent: (f64, f64),
    pub median_gap_session: u64,
    pub max_gap_session: u64,
    pub n: u64,
}

/// Chart-state identity for this pairing: a fresh Duel resets each chart's
/// zoom and pins. (`| 1` keeps it nonzero — 0 means "never reset".)
fn pair_key(a: &super::explorer::ComboKey, b: &super::explorer::ComboKey) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (a, b).hash(&mut h);
    h.finish() | 1
}

#[derive(Default)]
pub struct DuelState {
    pub open: bool,
    pub a: Option<ComboKey>,
    pub b: Option<ComboKey>,
    pub data: Option<Arc<Mutex<Option<DuelData>>>>,
    /// Cancels the pair computation when replaced or dropped.
    pub job: Option<crate::stream::JobGuard>,
    pub job_ctl: Option<Arc<SweepCtl>>,
    pub job_total: u64,
    /// Config + seed the pair was computed with.
    pub basis: Option<(SimConfig, u64)>,
    /// Trajectory panel: which session, and its two wealth tracks.
    pub focus_session: Option<u64>,
    pub tracks: Option<(Vec<i64>, Vec<i64>)>,
    /// Side A's full trace: the identical dice both lives were fed.
    pub dice: Option<Vec<craps_engine::trace::RollEvent>>,
    /// Scrub position in rolls over the focused paired session.
    pub revealed: f64,
}

impl DuelState {
    pub fn open_from_selection(&mut self, ex: &super::explorer::ExplorerUi) {
        if let (Some(a), Some(b)) = (ex.selected_a, ex.selected_b) {
            if a.min_index == b.min_index {
                self.a = Some(a);
                self.b = Some(b);
                self.open = true;
                self.data = None;
                self.job = None; // dropping the guard cancels any old worker
                self.job_ctl = None;
                self.tracks = None;
                self.dice = None;
                self.focus_session = None;
                // Fresh-load sentinel: the replayed night opens fully
                // revealed (derive(Default) would leave 0.0 = roll zero,
                // an EMPTY trajectory panel).
                self.revealed = f64::MAX;
            }
        }
    }
}

fn side_of(key: &ComboKey) -> PairSide {
    let strategies = explore_strategies();
    let mut sel = strategies[key.strategy_idx as usize].1.clone();
    sel.progression = key.progression;
    PairSide {
        sel,
        quit_mult: EXPLORE_QUITS[key.quit_idx as usize],
    }
}

pub(crate) fn combo_name(key: &ComboKey) -> String {
    let strategies = explore_strategies();
    format!(
        "{} · {} · {}",
        strategies[key.strategy_idx as usize].0,
        key.progression.label(),
        match EXPLORE_QUITS[key.quit_idx as usize] {
            Some(m) => format!("quit {m:.1}×"),
            None => "no quit".to_owned(),
        }
    )
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    let (Some(a_key), Some(b_key)) = (app.duel.a, app.duel.b) else {
        app.duel.open = false;
        return;
    };

    ui.horizontal(|ui| {
        if ui.button("← back to the strip").clicked() {
            app.duel.open = false;
        }
        let (glyph_rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
        crate::ui::icons::paint(
            ui.painter(),
            crate::ui::icons::Icon::Duel,
            glyph_rect,
            t.ink2,
        );
        ui.label(
            RichText::new(combo_name(&a_key))
                .font(FontId::new(type_scale::BODY, theme::sans_semibold()))
                .color(t.chip(a_key.min_cents)),
        );
        ui.label(RichText::new("vs").color(t.ink2));
        ui.label(
            RichText::new(combo_name(&b_key))
                .font(FontId::new(type_scale::BODY, theme::sans_semibold()))
                .color(t.violet),
        );
    });
    ui.label(
        RichText::new(
            "Both strategies played identical dice sequences. Differences below are strategy and residual variance — not table luck.",
        )
        .font(FontId::new(type_scale::CAPTION, theme::sans()))
        .color(t.ink2),
    );
    ui.add_space(6.0);

    // Kick off (or read) the paired computation.
    let basis = app
        .explore_run
        .as_ref()
        .map(|r| {
            let st = r.store.lock().unwrap();
            (st.config.clone(), st.seed)
        })
        .unwrap_or_else(|| (app.cfg.clone(), app.seed));
    if app.duel.data.is_none() || app.duel.basis.as_ref() != Some(&basis) {
        let cell: Arc<Mutex<Option<DuelData>>> = Arc::new(Mutex::new(None));
        let ctl = Arc::new(SweepCtl::default());
        app.duel.data = Some(cell.clone());
        app.duel.basis = Some(basis.clone());
        app.duel.job = Some(crate::stream::JobGuard(ctl.clone()));
        app.duel.job_ctl = Some(ctl.clone());
        app.duel.job_total = basis.0.explore_sessions.max(500) as u64;
        let (cfg, seed) = basis.clone();
        std::thread::spawn(move || {
            let data = compute(&cfg, seed, a_key, b_key, &ctl);
            *cell.lock().unwrap() = Some(data);
        });
    }
    // Chart run keys carry the pairing AND the run basis: re-running the
    // explorer (new settings or seed) rescales every distribution below.
    let basis_key = basis.0.explore_fingerprint() ^ basis.1;
    let Some(cell) = app.duel.data.clone() else {
        return;
    };
    let guard = cell.lock().unwrap();
    // The Duel opens ON a replayed session (Gasp 3): default panel 1 to
    // the median-gap night as soon as the pair computation lands.
    if let Some(d) = guard.as_ref() {
        if app.duel.focus_session.is_none() {
            app.duel.focus_session = Some(d.median_gap_session);
            app.duel.tracks = None;
            app.duel.dice = None;
            app.duel.revealed = f64::MAX;
        }
    }
    let Some(d) = guard.as_ref() else {
        // A real progress fill, never a decorative spinner (spec §6.7).
        let frac = app
            .duel
            .job_ctl
            .as_ref()
            .map(|c| {
                c.sessions_done.load(std::sync::atomic::Ordering::Relaxed) as f32
                    / app.duel.job_total.max(1) as f32
            })
            .unwrap_or(0.0);
        ui.add(
            egui::ProgressBar::new(frac.min(1.0))
                .desired_width(280.0)
                .text(format!(
                    "re-simulating the pair on shared dice — {:.0}%",
                    (frac * 100.0).min(100.0)
                ))
                .animate(false),
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
        return;
    };

    // Verdict line, always present, in prose — honest about edges.
    let n = d.n.max(1) as f64;
    let (a_hi, b_hi, close) = d.quadrants;
    let (cfg, _) = app.duel.basis.clone().unwrap();
    let rules = cfg.explore_rules();
    let ea = craps_engine::blended_edge(&side_of(&a_key).sel, &rules, a_key.min_cents);
    let eb = craps_engine::blended_edge(&side_of(&b_key).sel, &rules, b_key.min_cents);
    let edge_clause = match (ea, eb) {
        (Some(x), Some(y)) if (x - y).abs() < 1e-4 => {
            "Both have identical per-dollar expectation — the difference is shape.".to_owned()
        }
        (Some(x), Some(y)) => format!(
            "Per-dollar edges differ: A {} vs B {} — shape AND cost differ.",
            numerals::edge_pct(x),
            numerals::edge_pct(y)
        ),
        _ => String::new(),
    };
    ui.label(
        RichText::new(format!(
            "B ends higher in {:.1} of 100 shared nights, lower in {:.1}, within $1 in {:.1}. {}",
            b_hi as f64 / n * 100.0,
            a_hi as f64 / n * 100.0,
            close as f64 / n * 100.0,
            edge_clause
        ))
        .font(FontId::new(type_scale::STORY, theme::sans_medium()))
        .color(t.ink),
    );
    ui.add_space(8.0);

    // Panel 1: same dice, two lives. The active pick is highlighted and
    // the shown night is named, so there is never doubt about what the
    // trajectories below are.
    ui.horizontal(|ui| {
        for (label, s) in [
            ("median-gap night", d.median_gap_session),
            ("biggest-gap night", d.max_gap_session),
        ] {
            let active = app.duel.focus_session == Some(s);
            if ui.selectable_label(active, label).clicked() && !active {
                app.duel.focus_session = Some(s);
                app.duel.tracks = None;
                app.duel.dice = None;
                app.duel.revealed = f64::MAX;
            }
        }
        if let Some(sess) = app.duel.focus_session {
            ui.label(
                RichText::new(format!("showing night #{sess}"))
                    .font(FontId::new(type_scale::CAPTION, theme::mono()))
                    .color(t.ink2),
            );
        }
    });
    if let Some(sess) = app.duel.focus_session {
        if app.duel.tracks.is_none() {
            let (cfg, seed) = app.duel.basis.clone().unwrap();
            let s = session_seed(seed, a_key.min_index as u32, SeedPhase::Explore, sess);
            let mk = |key: &ComboKey| {
                let side = side_of(key);
                let quit = side
                    .quit_mult
                    .map(|m| quit_target_cents(cfg.budget_cents, m));
                trace_wealth(
                    &side.sel,
                    &cfg.explore_rules(),
                    key.min_cents,
                    cfg.budget_cents,
                    quit,
                    cfg.horizon_rolls(),
                    cfg.horizon_rolls(),
                    s,
                )
                .1
            };
            app.duel.tracks = Some((mk(&a_key), mk(&b_key)));
        }
        if let Some((wa, wb)) = app.duel.tracks.clone() {
            let max_rolls = wa.len().max(wb.len());
            // f64::MAX is the fresh-load sentinel; a scrub to roll 0 is a
            // legitimate position and stays put.
            if app.duel.revealed < 0.0 || app.duel.revealed > max_rolls as f64 {
                app.duel.revealed = max_rolls as f64;
            }
            let revealed = app.duel.revealed.floor() as usize;
            let tracks = vec![
                trajectory::Trajectory {
                    wealth: &wa,
                    color: t.chip(a_key.min_cents),
                    label: "A".to_owned(),
                    peak_outlay_cents: 0,
                    end: None,
                },
                trajectory::Trajectory {
                    wealth: &wb,
                    color: t.violet,
                    label: "B".to_owned(),
                    peak_outlay_cents: 0,
                    end: None,
                },
            ];
            let opts = trajectory::TrajectoryOptions {
                envelope: None,
                budget_cents: cfg.budget_cents,
                horizon_rolls: cfg.horizon_rolls() as usize,
                revealed,
                hours_per_roll: 1.0 / cfg.rolls_per_hour as f64,
                drift_line: None,
                simulated_mean_label: None,
            };
            ChartFrame::new(egui::Id::new("duel_traj"), "Same dice, two lives")
                .run_key((pair_key(&a_key, &b_key) ^ basis_key ^ sess) | 1)
                .provenance(format!(
                    "night #{sess} · seed-shared · exactly re-simulated"
                ))
                .height(240.0)
                .show(ui, &t, |cx| {
                    trajectory::paint(cx, &tracks, &opts);
                });
            // Scrubber, synchronized to the shared dice strip below.
            ui.horizontal(|ui| {
                let mut pos = app.duel.revealed;
                if ui
                    .add(egui::Slider::new(&mut pos, 0.0..=max_rolls as f64).show_value(false))
                    .changed()
                {
                    app.duel.revealed = pos;
                }
                ui.label(
                    RichText::new(format!("roll {revealed} / {max_rolls}"))
                        .font(FontId::new(type_scale::CAPTION, theme::mono()))
                        .color(t.ink2),
                );
            });
            // The shared dice: side A's full trace carries the identical
            // rolls both lives were fed. Computed once per focused session,
            // never per frame.
            if app.duel.dice.is_none() {
                let (_, basis_seed) = app.duel.basis.clone().unwrap();
                let s = session_seed(basis_seed, a_key.min_index as u32, SeedPhase::Explore, sess);
                let side = side_of(&a_key);
                let quit = side
                    .quit_mult
                    .map(|m| quit_target_cents(cfg.budget_cents, m));
                let full = craps_engine::trace::trace_session(
                    &side.sel,
                    &cfg.explore_rules(),
                    a_key.min_cents,
                    cfg.budget_cents,
                    quit,
                    cfg.horizon_rolls(),
                    cfg.horizon_rolls(),
                    s,
                );
                app.duel.dice = Some(full.events);
            }
        }
    }

    let playhead = (app.duel.revealed.floor() as usize).saturating_sub(1);
    if let Some(events) = app.duel.dice.as_ref() {
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 64.0), egui::Sense::hover());
        let mut hovered = None;
        crate::chart::dicestrip::paint(ui, &t, rect, events, playhead, &mut hovered);
    }

    // Panel 2 + 3 side by side: y=x hexbin and the paired-diff histogram.
    ui.columns(2, |cols| {
        ChartFrame::new(egui::Id::new("duel_hexbin"), "Every night: A vs B")
            .run_key((pair_key(&a_key, &b_key) ^ basis_key) | 1)
            .provenance(format!("n = {} paired nights", numerals::compact_n(d.n)))
            .height(280.0)
            .show(&mut cols[0], &t, |cx| {
                hexbin_chart::paint(
                    cx,
                    &hexbin_chart::HexbinPlot {
                        bins: &d.hexbins,
                        extent: d.extent,
                        a_label: "A",
                        b_label: "B",
                        quadrants: d.quadrants,
                        ramp_to: t.chip(a_key.min_cents),
                    },
                );
            });
        let tie_label = format!(
            "statistically indistinguishable at n = {}",
            numerals::compact_n(d.n)
        );
        ChartFrame::new(
            egui::Id::new("duel_diff"),
            "Per-night Δ = A − B, identical dice",
        )
        .run_key((pair_key(&a_key, &b_key) ^ basis_key) | 1)
        .story(format!(
            "median Δ = {} (95% CI {} to {})",
            numerals::money_text(d.median_delta, false),
            numerals::money_text(d.median_ci.0, false),
            numerals::money_text(d.median_ci.1, false)
        ))
        .height(280.0)
        .show(&mut cols[1], &t, |cx| {
            histogram::paint_diff(
                cx,
                &histogram::DiffHistogram {
                    counts: &d.diff_counts,
                    origin: d.diff_origin,
                    bin_width: d.diff_width,
                    median_ci: d.median_ci,
                    tie: d.tie,
                    tie_label: tie_label.clone(),
                },
            );
        });
    });

    // Panel 4: the paired decile ladder.
    ChartFrame::new(egui::Id::new("duel_ladder"), "Where the lives diverge")
        .height(240.0)
        .interactive(false)
        .show(ui, &t, |cx| {
            ladder::paint_paired(
                cx,
                &ladder::PairedLadder {
                    a: &d.deciles_a,
                    b: &d.deciles_b,
                    a_color: t.chip(a_key.min_cents),
                    b_color: t.violet,
                    a_label: "A",
                    b_label: "B",
                },
            );
        });
}

fn compute(
    cfg: &SimConfig,
    seed: u64,
    a_key: ComboKey,
    b_key: ComboKey,
    ctl: &SweepCtl,
) -> DuelData {
    let pairs = run_pair(
        &side_of(&a_key),
        &side_of(&b_key),
        &cfg.explore_rules(),
        a_key.min_cents,
        a_key.min_index as u32,
        cfg.budget_cents,
        cfg.explore_sessions.max(500) as u64,
        cfg.horizon_rolls(),
        seed,
        ctl,
    );
    let n = pairs.len() as u64;
    let _keep: &Vec<PairedFinal> = &pairs;

    // Paired differences, exact.
    let mut diffs: Vec<i64> = pairs.iter().map(|p| p.a_cents - p.b_cents).collect();
    let mut by_gap: Vec<(i64, u64)> = diffs
        .iter()
        .enumerate()
        .map(|(i, &d)| (d.abs(), i as u64))
        .collect();
    by_gap.sort_unstable();
    let median_gap_session = by_gap[by_gap.len() / 2].1;
    let max_gap_session = by_gap.last().map(|&(_, i)| i).unwrap_or(0);

    diffs.sort_unstable();
    let median_delta = diffs[diffs.len() / 2];
    let median_ci = median_ci_sorted(&diffs);
    let tie = median_ci.0 <= 0 && 0 <= median_ci.1;
    let a_hi = diffs.iter().filter(|&&d| d > 100).count() as u64;
    let b_hi = diffs.iter().filter(|&&d| d < -100).count() as u64;
    let close = n - a_hi - b_hi;

    // Diff histogram: 256 bins symmetric around zero.
    let spread = diffs
        .last()
        .copied()
        .unwrap_or(0)
        .abs()
        .max(diffs.first().copied().unwrap_or(0).abs())
        .max(100);
    let bins = 256i64;
    let width = (2 * spread / bins).max(1);
    let origin = -spread;
    let mut counts = vec![0u32; bins as usize];
    for &d in &diffs {
        let ix = ((d - origin) / width).clamp(0, bins - 1) as usize;
        counts[ix] += 1;
    }

    // Deciles per side.
    let mut a_sorted: Vec<i64> = pairs.iter().map(|p| p.a_cents).collect();
    let mut b_sorted: Vec<i64> = pairs.iter().map(|p| p.b_cents).collect();
    a_sorted.sort_unstable();
    b_sorted.sort_unstable();
    let deciles = |v: &[i64]| -> [i64; 9] {
        let mut out = [0i64; 9];
        for (k, o) in out.iter_mut().enumerate() {
            let idx = (((k + 1) as f64 / 10.0) * (v.len() as f64 - 1.0)).round() as usize;
            *o = v[idx.min(v.len() - 1)];
        }
        out
    };

    // Hexbin over the shared extent.
    let hi = *a_sorted
        .last()
        .unwrap_or(&1)
        .max(b_sorted.last().unwrap_or(&1)) as f64;
    let extent = (0.0, hi.max(1.0));
    let points: Vec<(i64, i64)> = pairs.iter().map(|p| (p.a_cents, p.b_cents)).collect();
    let hexbins = hexbin(&points, extent, 40);

    DuelData {
        diff_counts: counts,
        diff_origin: origin,
        diff_width: width,
        median_delta,
        median_ci,
        tie,
        quadrants: (a_hi, b_hi, close),
        deciles_a: deciles(&a_sorted),
        deciles_b: deciles(&b_sorted),
        hexbins,
        extent,
        median_gap_session,
        max_gap_session,
        n,
    }
}
