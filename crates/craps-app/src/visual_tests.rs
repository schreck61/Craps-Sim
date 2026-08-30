// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Headless pixel snapshots of every screen (egui_kittest + wgpu).
//!
//! `#[ignore]`d in the default suite: GPU output differs across
//! drivers/platforms, so CI runs these on one pinned Linux backend only
//! (see ci.yml's snapshot job) and they are generated/updated locally with
//! `UPDATE_SNAPSHOTS=1 cargo test -p craps-app --release -- --ignored visual_ --nocapture`.
//! Unlike the window tour (`CRAPS_SIM_TOUR`), these run with no window and
//! no display — they are the review artifacts and the regression net.

use egui::vec2;
use egui_kittest::Harness;

use crate::app::{App, FindingsTab, Mode};
use crate::store::RunStatus;

fn wait_for<F: Fn(&App) -> bool>(harness: &mut Harness<'_, App>, pred: F, what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    loop {
        harness.step();
        if pred(harness.state()) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
}

fn settle(harness: &mut Harness<'_, App>, frames: usize) {
    for _ in 0..frames {
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
#[ignore] // GPU-dependent: pinned-backend CI job + local generation only.
fn visual_all_screens() {
    let mut harness = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .build_eframe(|cc| {
            let mut app = App::new(cc);
            // Deterministic seed so the snapshots are reproducible.
            app.seed = 0x5EED_CAFE_F00D_0001;
            app.cfg.sessions = 120_000;
            app.cfg.table_mins_cents = vec![500, 1000, 2500];
            app.cfg.sel.take_odds = true;
            app.cfg.explore_sessions = 1_000;
            app.table_mins_text = "5, 10, 25".into();
            app
        });

    // 01 — Design, cold.
    settle(&mut harness, 3);
    harness.snapshot("01-design");

    // Run; catch the settled Horizon (streaming frames are nondeterministic
    // by design, so the snapshot net starts at completion).
    harness.state_mut().start_run();
    harness.state_mut().mode = Mode::Findings;
    harness.state_mut().findings_tab = FindingsTab::Horizon;
    wait_for(
        &mut harness,
        |app| {
            app.main_run
                .as_ref()
                .map(|r| {
                    let st = r.store.lock().unwrap();
                    st.status == RunStatus::Complete && st.mins.iter().all(|m| m.fan.is_some())
                })
                .unwrap_or(false)
        },
        "main run completion",
    );
    settle(&mut harness, 8);
    harness.snapshot("02-horizon");

    // 03 — Endurance.
    harness.state_mut().findings_tab = FindingsTab::Endurance;
    settle(&mut harness, 4);
    harness.snapshot("03-endurance");

    // 04 — Stake.
    harness.state_mut().findings_tab = FindingsTab::Stake;
    settle(&mut harness, 4);
    harness.snapshot("04-stake");

    // 05 — the Anchor mid-swap is nondeterministic; snapshot the settled
    // Martingale variant instead.
    harness.state_mut().findings_tab = FindingsTab::Horizon;
    harness.state_mut().anchor.selected = Some(craps_engine::Progression::Martingale);
    wait_for(
        &mut harness,
        |app| {
            app.anchor
                .jobs
                .get(&(
                    0,
                    crate::config::progression_index(craps_engine::Progression::Martingale),
                ))
                .map(|j| j.cell.lock().unwrap().is_some())
                .unwrap_or(false)
        },
        "anchor variant",
    );
    settle(&mut harness, 12);
    harness.snapshot("05-horizon-anchor");
    harness.state_mut().anchor.selected = None;

    // 06 — Replay, the median night.
    let median = harness.state().main_run.as_ref().and_then(|r| {
        let st = r.store.lock().unwrap();
        st.mins.first().and_then(|m| {
            m.sorted
                .as_ref()
                .map(|sv| sv.finals_rank_to_id[sv.finals_rank_to_id.len() / 2])
        })
    });
    let median = median.expect("sorted views for the focused minimum");
    harness.state_mut().open_replay(0, median as u64);
    settle(&mut harness, 4);
    harness.snapshot("06-replay");

    // 07 — Explorer.
    harness.state_mut().start_explore();
    wait_for(
        &mut harness,
        |app| {
            app.explore_run
                .as_ref()
                .map(|r| r.store.lock().unwrap().status == crate::store::ExploreStatus::Complete)
                .unwrap_or(false)
        },
        "explorer completion",
    );
    settle(&mut harness, 6);
    harness.snapshot("07-explorer");

    // 08 — the Duel: top two combos by P(double).
    let keys = harness.state().explore_run.as_ref().and_then(|r| {
        let st = r.store.lock().unwrap();
        st.mins.first().map(|(min_cents, rows, _)| {
            let mut sorted: Vec<_> = rows.iter().collect();
            sorted.sort_by(|a, b| {
                b.p_double
                    .partial_cmp(&a.p_double)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let mk = |r: &craps_engine::ExploreRow| crate::screens::explorer::ComboKey {
                min_index: 0,
                min_cents: *min_cents,
                strategy_idx: r.strategy_idx,
                progression: r.progression,
                quit_idx: r.quit_idx,
            };
            (mk(sorted[0]), mk(sorted[1]))
        })
    });
    let (a, b) = keys.expect("explorer rows");
    {
        let app = harness.state_mut();
        app.explorer_ui.selected_a = Some(a);
        app.explorer_ui.selected_b = Some(b);
        let ex = std::mem::take(&mut app.explorer_ui);
        app.duel.open_from_selection(&ex);
        app.explorer_ui = ex;
        app.mode = Mode::Explorer;
    }
    wait_for(
        &mut harness,
        |app| {
            app.duel
                .data
                .as_ref()
                .map(|c| c.lock().unwrap().is_some())
                .unwrap_or(false)
        },
        "duel pair computation",
    );
    // Focus the median-gap session so panel 1 has its two lives.
    {
        let app = harness.state_mut();
        let sess = app
            .duel
            .data
            .as_ref()
            .and_then(|c| c.lock().unwrap().as_ref().map(|d| d.median_gap_session));
        app.duel.focus_session = sess;
    }
    wait_for(
        &mut harness,
        |app| app.duel.tracks.is_some() && app.duel.dice.is_some(),
        "duel traces",
    );
    settle(&mut harness, 4);
    harness.snapshot("08-duel");

    // 09 — Reading Room (light) Horizon.
    {
        let app = harness.state_mut();
        app.duel.open = false;
        app.mode = Mode::Findings;
        app.findings_tab = FindingsTab::Horizon;
        app.prefs.dark = Some(false);
        app.theme = crate::ui::theme::Theme::reading_room();
    }
    // Theme applies via ctx on the next frame in App::ui? The shell applies
    // visuals only on toggle; force it through the public path.
    harness.step();
    let ctx = harness.ctx.clone();
    crate::ui::theme::apply(&ctx, &crate::ui::theme::Theme::reading_room());
    settle(&mut harness, 4);
    harness.snapshot("09-horizon-light");
}

/// The Bench, open with a session stepped onto it. Its own snapshot rather
/// than part of the Design shot, because the panel is collapsed by default
/// and the point of the picture is what it looks like in use.
#[test]
#[ignore] // GPU-dependent: pinned-backend CI job + local generation only.
fn visual_bench() {
    let mut harness = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .build_eframe(|cc| {
            let mut app = App::new(cc);
            app.seed = 0x5EED_CAFE_F00D_0001;
            app.cfg.sel.take_odds = true;
            app.cfg.sel.set_place(6, true);
            app.cfg.sel.set_place(8, true);
            app.bench.open = true;
            app
        });
    settle(&mut harness, 2);
    {
        let app = harness.state_mut();
        // What the "Take the current player" button does, then a run — the
        // panel is worth looking at full, not empty.
        let s = craps_engine::strategy::from_selection(&app.cfg.sel, &app.cfg.rules());
        app.bench.source = craps_engine::strategy::render(&s);
        app.bench.build();
        let p = app.bench.program.clone().unwrap();
        app.bench.trace = Some(craps_engine::strategy::bench_session(
            &p,
            &app.cfg.rules(),
            1000,
            app.cfg.budget_cents,
            None,
            app.cfg.max_rolls,
            app.cfg.horizon_rolls(),
            7,
        ));
        app.bench.position = 12;
    }
    // The Design screen scrolls, and the Bench sits under the bet rail, so
    // the picture has to be taken where the panel actually is.
    settle(&mut harness, 2);
    // The wheel goes where the pointer is; without this it scrolls nothing.
    harness.event(egui::Event::PointerMoved(egui::pos2(800.0, 500.0)));
    harness.step();
    for _ in 0..40 {
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -120.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        harness.step();
    }
    settle(&mut harness, 4);
    harness.snapshot("10-bench");
}
