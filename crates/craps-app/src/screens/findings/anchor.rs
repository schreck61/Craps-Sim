// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The Anchor (Gasp 4): a segmented control that hot-swaps the pressing
//! progression over the Horizon histogram. Every variant is re-simulated on
//! IDENTICAL dice (the pair-link semantics), reduced to a ~5 KB digest, and
//! cached; an uncached variant shows a real progress fill first. Through
//! every morph one object never moves: the amber per-dollar edge marker.
//! The expected-dollar-loss pin drifts with the handle — and the caption
//! states exactly why.

use std::collections::HashMap;

use craps_engine::Progression;
use egui::{FontId, RichText};

use crate::app::App;
use crate::config::progression_index;
use crate::store::AnchorVariant;
use crate::stream::AnchorJob;
use crate::ui::theme::{self, type_scale};

#[derive(Default)]
pub struct AnchorUi {
    /// The progression currently shown (None = the run's own).
    pub selected: Option<Progression>,
    /// The previously shown variant, kept for the 400 ms crossfade.
    pub prev: Option<AnchorVariant>,
    /// When the selection changed (drives the crossfade).
    pub swapped_at: Option<f64>,
    pub jobs: HashMap<(usize, u8), AnchorJob>,
}

/// The variant to draw for (mi, prog): cached digest, or None while its job
/// streams (the caller shows the progress fill). `cfg`/`seed` are the run's
/// provenance — passed in because callers already hold the store lock
/// (relocking here would self-deadlock).
pub fn variant_for(
    app: &mut App,
    cfg: &crate::config::SimConfig,
    seed: u64,
    mi: usize,
    prog: Progression,
) -> Option<AnchorVariant> {
    let key = (mi, progression_index(prog));
    if let Some(job) = app.anchor.jobs.get(&key) {
        return job.cell.lock().unwrap().clone();
    }
    // Spawn the job on demand — a real re-simulation on identical dice.
    let job = crate::stream::start_anchor_job(cfg, seed, mi, prog);
    app.anchor.jobs.insert(key, job);
    None
}

/// Progress fraction of the (mi, prog) job, for the fill.
pub fn job_progress(app: &App, mi: usize, prog: Progression) -> f32 {
    app.anchor
        .jobs
        .get(&(mi, progression_index(prog)))
        .map(|j| {
            j.ctl
                .sessions_done
                .load(std::sync::atomic::Ordering::Relaxed) as f32
                / j.total.max(1) as f32
        })
        .unwrap_or(0.0)
}

/// The segmented progression control. Returns the progression to display.
pub fn control(
    app: &mut App,
    ui: &mut egui::Ui,
    cfg: &crate::config::SimConfig,
    seed: u64,
    mi: usize,
    run_prog: Progression,
) -> Progression {
    let t = app.theme.clone();
    let current = app.anchor.selected.unwrap_or(run_prog);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("Pressing:")
                .font(FontId::new(type_scale::BODY, theme::sans()))
                .color(t.ink2),
        );
        for p in Progression::ALL {
            let is_run = p == run_prog;
            let selected = p == current;
            let label = if is_run {
                format!("{} (this run)", p.label())
            } else {
                p.label().to_owned()
            };
            if ui.selectable_label(selected, label).clicked() && p != current {
                // Keep the outgoing shape for the crossfade.
                app.anchor.prev = if current == run_prog {
                    None // the run's own histogram is drawn by the caller
                } else {
                    variant_for(app, cfg, seed, mi, current)
                };
                app.anchor.selected = if is_run { None } else { Some(p) };
                app.anchor.swapped_at = Some(ui.input(|i| i.time));
            }
        }
        // The pair-link glyph: every variant is computed under common
        // random numbers.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
        crate::ui::icons::paint(ui.painter(), crate::ui::icons::Icon::Duel, rect, t.ink2);
        ui.label(
            RichText::new("re-simulated on identical dice")
                .font(FontId::new(type_scale::CAPTION, theme::sans()))
                .color(t.ink2),
        );
    });
    current
}

/// Idle prefetch: after the run completes, warm the focused minimum's
/// variants one at a time in picker order — a low-priority job, one in
/// flight at most, cancelled implicitly by any new run (state is cleared).
pub fn idle_prefetch(app: &mut App, cfg: &crate::config::SimConfig, seed: u64, mi: usize) {
    let any_in_flight = app
        .anchor
        .jobs
        .values()
        .any(|j| j.cell.lock().unwrap().is_none());
    if any_in_flight {
        return;
    }
    let next = Progression::ALL
        .into_iter()
        .find(|&p| !app.anchor.jobs.contains_key(&(mi, progression_index(p))));
    if let Some(p) = next {
        variant_for(app, cfg, seed, mi, p);
    }
}

/// The drifting-pin caption: the shape convulses, the rate is bolted down,
/// and the drift itself is the lesson.
pub fn drift_caption() -> &'static str {
    "You wagered more, so you lost more. The house's cut per dollar never moved — you only changed how much you fed it."
}
