// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Debug-only self-drive: `CRAPS_SIM_TOUR=<dir>` runs the example, walks
//! every mode, and saves real window screenshots into `<dir>`, then quits.
//! This is the sanctioned use of `ViewportCommand::Screenshot` (a debug
//! tool, never the export path) — it exists so the interface can be
//! verified visually and regression-shot in development without OS-level
//! screen permissions.

use std::path::PathBuf;

use crate::app::{App, FindingsTab, Mode};
use crate::store::RunStatus;

pub struct Tour {
    dir: PathBuf,
    step: usize,
    frames_in_step: u32,
    pending_shot: Option<&'static str>,
}

impl Tour {
    pub fn from_env() -> Option<Self> {
        let dir = std::env::var_os("CRAPS_SIM_TOUR")?;
        let dir = PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        Some(Self {
            dir,
            step: 0,
            frames_in_step: 0,
            pending_shot: None,
        })
    }
}

/// Advance the tour one frame. Runs before the frame is built so mode
/// switches land this frame; screenshots are requested AFTER a mode has
/// had a few frames to settle.
pub fn drive(app: &mut App, ctx: &egui::Context) {
    let Some(mut tour) = app.tour.take() else {
        return;
    };
    if tour.step == 0 && tour.frames_in_step == 0 {
        eprintln!("tour: armed, dir {}", tour.dir.display());
    }
    step(app, ctx, &mut tour);
    app.tour = Some(tour);
}

fn step(app: &mut App, ctx: &egui::Context, tour: &mut Tour) {
    // Save any screenshot that arrived.
    if let Some(name) = tour.pending_shot {
        let img = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = img {
            let png = crate::chart::export::encode_png(&image);
            let path = tour.dir.join(format!("{name}.png"));
            let _ = std::fs::write(&path, png);
            eprintln!("tour: wrote {}", path.display());
            tour.pending_shot = None;
            tour.step += 1;
            tour.frames_in_step = 0;
            return;
        }
    }
    if tour.pending_shot.is_some() {
        ctx.request_repaint();
        return; // waiting for the screenshot event
    }
    tour.frames_in_step += 1;
    ctx.request_repaint();

    // Helper: request a shot once the current screen has settled.
    macro_rules! shot_after {
        ($frames:expr, $name:expr) => {{
            if tour.frames_in_step >= $frames {
                tour.pending_shot = Some($name);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            return;
        }};
    }

    let step = tour.step;
    match step {
        // Design, cold.
        0 => {
            app.mode = Mode::Design;
            shot_after!(20, "01-design");
        }
        // Start the run, catch the Rain mid-stream.
        1 => {
            if tour.frames_in_step == 1 {
                app.cfg.sessions = 400_000;
                app.mode = Mode::Findings;
                app.findings_tab = FindingsTab::Horizon;
                app.start_run();
            }
            shot_after!(4, "02-horizon-streaming");
        }
        // Wait for completion, then the settled Horizon.
        2 => {
            let complete = app
                .main_run
                .as_ref()
                .map(|r| {
                    let st = r.store.lock().unwrap();
                    st.status == RunStatus::Complete && st.mins.iter().all(|m| m.fan.is_some())
                })
                .unwrap_or(false);
            if complete {
                shot_after!(30, "03-horizon-complete");
            }
        }
        3 => {
            app.findings_tab = FindingsTab::Endurance;
            shot_after!(15, "04-endurance");
        }
        4 => {
            app.findings_tab = FindingsTab::Stake;
            shot_after!(15, "05-stake");
        }
        // The Anchor: swap the progression, wait for the variant.
        5 => {
            if tour.frames_in_step == 1 {
                app.findings_tab = FindingsTab::Horizon;
                app.anchor.selected = Some(craps_engine::Progression::Martingale);
                app.anchor.swapped_at = Some(ctx.input(|i| i.time));
            }
            let ready = app
                .anchor
                .jobs
                .values()
                .any(|j| j.cell.lock().unwrap().is_some());
            if ready {
                shot_after!(40, "06-horizon-anchor-martingale");
            }
        }
        // Replay: the median night.
        6 => {
            if tour.frames_in_step == 1 {
                let median = app.main_run.as_ref().and_then(|r| {
                    let st = r.store.lock().unwrap();
                    st.mins.first().and_then(|m| {
                        m.sorted.as_ref().and_then(|sv| {
                            sv.finals_rank_to_id
                                .get(sv.finals_rank_to_id.len() / 2)
                                .copied()
                        })
                    })
                });
                if let Some(id) = median {
                    app.open_replay(0, id as u64);
                    app.replay.header_note = "the median night".to_owned();
                }
            }
            shot_after!(15, "07-replay");
        }
        // Explorer sweep (fast) + strip/leaderboard.
        7 => {
            if tour.frames_in_step == 1 {
                app.cfg.explore_sessions = 1_000;
                app.start_explore();
            }
            let complete = app
                .explore_run
                .as_ref()
                .map(|r| r.store.lock().unwrap().status == crate::store::ExploreStatus::Complete)
                .unwrap_or(false);
            if complete {
                shot_after!(20, "08-explorer");
            }
        }
        // The Duel: first two leaderboard combos.
        8 => {
            if tour.frames_in_step == 1 {
                let keys = app.explore_run.as_ref().and_then(|r| {
                    let st = r.store.lock().unwrap();
                    st.mins.first().map(|(min_cents, rows, _)| {
                        let mut sorted: Vec<_> = rows.iter().collect();
                        sorted.sort_by(|a, b| {
                            b.p_double
                                .partial_cmp(&a.p_double)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                        let mk =
                            |r: &craps_engine::ExploreRow| crate::screens::explorer::ComboKey {
                                min_index: 0,
                                min_cents: *min_cents,
                                strategy_idx: r.strategy_idx,
                                progression: r.progression,
                                quit_idx: r.quit_idx,
                            };
                        (mk(sorted[0]), mk(sorted[1]))
                    })
                });
                if let Some((a, b)) = keys {
                    app.explorer_ui.selected_a = Some(a);
                    app.explorer_ui.selected_b = Some(b);
                    app.duel.open_from_selection(&app.explorer_ui);
                    app.mode = Mode::Explorer;
                }
            }
            let ready = app
                .duel
                .data
                .as_ref()
                .map(|c| c.lock().unwrap().is_some())
                .unwrap_or(false);
            if ready && app.duel.focus_session.is_none() {
                // Focus the median-gap session so panel 1 renders.
                let sess = app
                    .duel
                    .data
                    .as_ref()
                    .and_then(|c| c.lock().unwrap().as_ref().map(|d| d.median_gap_session));
                app.duel.focus_session = sess;
            }
            if ready && app.duel.tracks.is_some() {
                shot_after!(20, "09-duel");
            }
        }
        // Light theme, then done.
        9 => {
            if tour.frames_in_step == 1 {
                app.prefs.dark = Some(false);
                app.theme = crate::ui::theme::Theme::reading_room();
                crate::ui::theme::apply(ctx, &app.theme);
                app.mode = Mode::Findings;
                app.findings_tab = FindingsTab::Horizon;
                app.anchor.selected = None;
            }
            shot_after!(20, "10-horizon-light");
        }
        _ => {
            eprintln!("tour: complete");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
