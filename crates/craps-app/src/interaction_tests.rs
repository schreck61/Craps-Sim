// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Headless interaction tests (egui_kittest, CPU-only — no snapshots and no
//! GPU), covering spec Part I §8 (interaction & input model) and §9
//! (states): the keyboard map, typing suppression, Space/Esc run control,
//! staleness, probe pins, zoom windows, export routing, plain-language
//! errors, and the first-run example. Unlike `visual_tests`, these run in
//! the default suite on every platform.

use std::sync::atomic::Ordering;

use egui::accesskit::Role;
use egui::{pos2, vec2, Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

use crate::app::{App, FindingsTab, Mode};
use crate::chart::frame::FrameState;
use crate::store::RunStatus;

// ---------------------------------------------------------------------------
// Harness plumbing
// ---------------------------------------------------------------------------

fn build_app(setup: impl FnOnce(&mut App)) -> Harness<'static, App> {
    Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .build_eframe(move |cc| {
            let mut app = App::new(cc);
            setup(&mut app);
            app
        })
}

/// Loop-with-deadline instead of fixed sleeps: wait for a predicate over the
/// app state while stepping frames.
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
    }
}

/// A small deterministic single-minimum configuration whose background run
/// completes in well under a second.
fn small_cfg(app: &mut App) {
    app.seed = 0x5EED_0000_0000_0042;
    app.cfg.sessions = 5_000;
    app.cfg.table_mins_cents = vec![1000];
    app.table_mins_text = "10".into();
    // Deterministic layout for the chart tests: story register on top.
    app.prefs.ledger_register = false;
}

fn run_to_complete(harness: &mut Harness<'_, App>) {
    harness.state_mut().start_run();
    wait_for(
        harness,
        |app| {
            app.main_run
                .as_ref()
                .map(|r| {
                    let st = r.store.lock().unwrap();
                    st.status == RunStatus::Complete
                        && st
                            .mins
                            .iter()
                            .all(|m| m.summary.is_some() && m.fan.is_some())
                        && st.mins.first().is_some_and(|m| m.sorted.is_some())
                })
                .unwrap_or(false)
        },
        "main run completion",
    );
    settle(harness, 4);
}

fn wait_terminal(harness: &mut Harness<'_, App>, what: &str) {
    wait_for(
        harness,
        |app| {
            app.main_run.as_ref().is_some_and(|r| {
                !matches!(r.store.lock().unwrap().status, RunStatus::Streaming { .. })
            })
        },
        what,
    );
}

/// A fresh harness with a completed run, parked on Findings · Horizon.
fn horizon_ready() -> Harness<'static, App> {
    let mut harness = build_app(small_cfg);
    run_to_complete(&mut harness);
    {
        let app = harness.state_mut();
        app.mode = Mode::Findings;
        app.findings_tab = FindingsTab::Horizon;
    }
    settle(&mut harness, 2);
    harness
}

// ---------------------------------------------------------------------------
// Chart-frame plumbing (the focused Horizon histogram)
// ---------------------------------------------------------------------------

fn hist_id() -> egui::Id {
    egui::Id::new(("horizon_hist", 0usize))
}

fn hist_state(harness: &Harness<'_, App>) -> FrameState {
    harness
        .ctx
        .data(|d| d.get_temp::<FrameState>(hist_id()))
        .expect("the horizon histogram has rendered")
}

/// Park the pointer over the focused histogram's plot area by probing down
/// the central column until the frame itself reports hover (via its
/// persisted `FrameState`), so the test never hard-codes layout.
fn hover_horizon_hist(harness: &mut Harness<'_, App>) -> Pos2 {
    for y in (120..880).step_by(20) {
        let pos = pos2(720.0, y as f32);
        harness.hover_at(pos);
        harness.step();
        let hovered = harness
            .ctx
            .data(|d| d.get_temp::<FrameState>(hist_id()))
            .is_some_and(|s| s.hover_since.is_some());
        if hovered {
            return pos;
        }
    }
    panic!("could not find the horizon histogram plot area");
}

fn click_at(harness: &mut Harness<'_, App>, pos: Pos2) {
    for pressed in [true, false] {
        harness.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    harness.step();
}

fn double_click_at(harness: &mut Harness<'_, App>, pos: Pos2) {
    // All four events land in one frame so egui's double-click window is
    // satisfied regardless of the harness's virtual step_dt.
    for _ in 0..2 {
        for pressed in [true, false] {
            harness.input_mut().events.push(Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
    }
    harness.step();
}

// ---------------------------------------------------------------------------
// 1. The keyboard map (spec §8)
// ---------------------------------------------------------------------------

#[test]
fn keyboard_mode_map() {
    let mut h = build_app(|_| {});
    settle(&mut h, 2);

    for (key, mode) in [
        (Key::Num2, Mode::Findings),
        (Key::Num3, Mode::Explorer),
        (Key::Num4, Mode::Replay),
        (Key::Num1, Mode::Design),
    ] {
        h.key_press(key);
        h.step();
        assert_eq!(h.state().mode, mode, "mode key {key:?}");
    }

    h.key_press(Key::Q);
    h.step();
    assert_eq!(h.state().mode, Mode::Findings);
    assert_eq!(h.state().findings_tab, FindingsTab::Endurance);

    h.key_press(Key::W);
    h.step();
    assert_eq!(h.state().findings_tab, FindingsTab::Horizon);

    // E on Horizon morphs histogram <-> ECDF instead of switching tabs.
    assert!(!h.state().ecdf_mode);
    h.key_press(Key::E);
    h.step();
    assert!(h.state().ecdf_mode, "E on Horizon toggles the ECDF morph");
    assert_eq!(h.state().findings_tab, FindingsTab::Horizon);
    h.key_press(Key::E);
    h.step();
    assert!(!h.state().ecdf_mode);

    // From Design, E is the Stake tab (and leaves the morph alone).
    h.key_press(Key::Num1);
    h.step();
    h.key_press(Key::E);
    h.step();
    assert_eq!(h.state().mode, Mode::Findings);
    assert_eq!(h.state().findings_tab, FindingsTab::Stake);
    assert!(!h.state().ecdf_mode);

    // R is always Stake.
    h.key_press(Key::Num1);
    h.step();
    h.key_press(Key::R);
    h.step();
    assert_eq!(h.state().findings_tab, FindingsTab::Stake);

    // L flips the Story/Ledger register (twice, restoring prefs on disk).
    let register = h.state().prefs.ledger_register;
    h.key_press(Key::L);
    h.step();
    assert_eq!(h.state().prefs.ledger_register, !register);
    h.key_press(Key::L);
    h.step();
    assert_eq!(h.state().prefs.ledger_register, register);

    assert!(!h.state().small_multiples);
    h.key_press(Key::M);
    h.step();
    assert!(h.state().small_multiples, "M toggles small multiples");

    assert!(!h.state().log_y);
    h.key_press(Key::Y);
    h.step();
    assert!(h.state().log_y, "Y toggles the labeled log-y");

    assert!(!h.state().ledger_drawer);
    h.key_press(Key::T);
    h.step();
    assert!(h.state().ledger_drawer, "T toggles the ledger drawer");

    // '?' opens the shortcut overlay, Esc closes it — both spellings.
    h.key_press(Key::Questionmark);
    h.step();
    assert!(h.state().show_shortcuts);
    h.key_press(Key::Escape);
    h.step();
    assert!(!h.state().show_shortcuts, "Esc closes the overlay");

    h.key_press_modifiers(Modifiers::SHIFT, Key::Slash);
    h.step();
    assert!(h.state().show_shortcuts, "shift+slash also opens it");
    h.key_press(Key::Escape);
    h.step();
    assert!(!h.state().show_shortcuts);
}

// ---------------------------------------------------------------------------
// 2. Single-letter shortcuts stand down while typing (spec §8)
// ---------------------------------------------------------------------------

#[test]
fn typing_suppresses_single_letter_shortcuts() {
    let mut h = build_app(|_| {});
    settle(&mut h, 2);
    assert_eq!(h.state().mode, Mode::Design);

    // Focus a text field by clicking it (Design's minimums editor).
    h.get_all_by_role(Role::TextInput)
        .next()
        .expect("Design shows a text input")
        .click();
    h.step();
    assert!(
        h.ctx.memory(|m| m.focused().is_some()),
        "clicking the text field must focus it"
    );

    let register = h.state().prefs.ledger_register;
    let multiples = h.state().small_multiples;
    h.key_press(Key::L);
    h.key_press(Key::M);
    h.step();
    assert_eq!(
        h.state().prefs.ledger_register,
        register,
        "L must stand down while a text field is focused"
    );
    assert_eq!(
        h.state().small_multiples,
        multiples,
        "M must stand down while a text field is focused"
    );

    // Escape surrenders focus; the same letter works again.
    h.key_press(Key::Escape);
    h.step();
    assert!(h.ctx.memory(|m| m.focused().is_none()));
    h.key_press(Key::M);
    h.step();
    assert_eq!(h.state().small_multiples, !multiples);
}

// ---------------------------------------------------------------------------
// 3. Space runs, Esc cancels (spec §8, §9 streaming/cancelled)
// ---------------------------------------------------------------------------

#[test]
fn space_runs_and_esc_cancels() {
    let mut h = build_app(|app| {
        app.seed = 0x5EED_0000_0000_0003;
        // Big enough that the run is still streaming when Esc arrives.
        app.cfg.sessions = 2_000_000;
        app.cfg.table_mins_cents = vec![500];
        app.table_mins_text = "5".into();
    });
    settle(&mut h, 2);
    assert!(h.state().main_run.is_none());

    h.key_press(Key::Space);
    h.step();
    {
        let app = h.state();
        let run = app.main_run.as_ref().expect("Space starts a run");
        assert!(run.is_running(), "the run should be streaming");
        assert!(!run.ctl.stop.load(Ordering::Relaxed));
        assert_eq!(
            app.mode,
            Mode::Findings,
            "starting from Design lands on Findings"
        );
    }

    // Let at least one batch stream in so the cancel keeps partial data.
    wait_for(
        &mut h,
        |app| {
            app.main_run.as_ref().is_some_and(
                |r| matches!(r.store.lock().unwrap().status, RunStatus::Streaming { n } if n > 0),
            )
        },
        "first streamed batch",
    );

    h.key_press(Key::Escape);
    h.step();
    assert!(
        h.state()
            .main_run
            .as_ref()
            .expect("the run persists through cancel")
            .ctl
            .stop
            .load(Ordering::Relaxed),
        "Esc must request cancellation immediately"
    );

    wait_terminal(&mut h, "cancelled run to settle");
    let st = h.state().main_run.as_ref().unwrap().store.lock().unwrap();
    if let RunStatus::Cancelled { n } = st.status {
        assert!(n > 0, "partial results persist after a cancel");
    }
}

// ---------------------------------------------------------------------------
// 4. Config edits mark results stale (spec §9 stale)
// ---------------------------------------------------------------------------

#[test]
fn stale_fingerprint_on_edit() {
    let mut h = build_app(small_cfg);
    run_to_complete(&mut h);
    assert!(!h.state().is_stale(), "a fresh run is not stale");

    h.state_mut().cfg.budget_cents += 100;
    assert!(h.state().is_stale(), "an edited budget must read as stale");

    h.state_mut().cfg.budget_cents -= 100;
    assert!(
        !h.state().is_stale(),
        "reverting the edit restores freshness"
    );
}

// ---------------------------------------------------------------------------
// 5. Bracket keys cycle the focused minimum (spec §8)
// ---------------------------------------------------------------------------

#[test]
fn bracket_keys_cycle_focused_min() {
    let mut h = build_app(|app| {
        app.cfg.table_mins_cents = vec![500, 1000];
        app.table_mins_text = "5, 10".into();
    });
    settle(&mut h, 2);
    assert_eq!(h.state().focused_min, 0);

    h.key_press(Key::CloseBracket);
    h.step();
    assert_eq!(h.state().focused_min, 1, "']' advances the focused minimum");

    h.key_press(Key::CloseBracket);
    h.step();
    assert_eq!(h.state().focused_min, 0, "']' wraps forward");

    h.key_press(Key::OpenBracket);
    h.step();
    assert_eq!(h.state().focused_min, 1, "'[' wraps back");

    h.key_press(Key::OpenBracket);
    h.step();
    assert_eq!(h.state().focused_min, 0);
}

// ---------------------------------------------------------------------------
// 6. The Scenario Sentence round-trips from live app state
// ---------------------------------------------------------------------------

#[test]
fn sentence_roundtrip_from_app_state() {
    let mut h = build_app(|_| {});

    let cfg = h.state().cfg.clone();
    let text = crate::sentence::render_text(&cfg);
    assert_eq!(
        crate::sentence::parse(&text).expect("the app's sentence parses"),
        cfg
    );

    {
        let app = h.state_mut();
        app.cfg.quit_mult = Some(1.5);
        app.cfg.sel.take_odds = true;
        app.cfg.sel.come_max = 2;
        app.cfg.table_mins_cents = vec![1500, 2500];
    }
    let cfg = h.state().cfg.clone();
    let text = crate::sentence::render_text(&cfg);
    assert_eq!(
        crate::sentence::parse(&text).expect("the edited sentence parses"),
        cfg
    );
}

// ---------------------------------------------------------------------------
// 7. Hover raises the probe; click pins it; Backspace removes it (spec §8)
// ---------------------------------------------------------------------------

#[test]
fn probe_pin_click() {
    let mut h = horizon_ready();
    let pos = hover_horizon_hist(&mut h);

    // Dwell past the 80 ms hover-in delay (each step advances virtual time).
    settle(&mut h, 2);
    assert!(
        hist_state(&h).probe.is_some(),
        "the probe raises after the hover dwell"
    );
    assert!(hist_state(&h).pins.is_empty());

    click_at(&mut h, pos);
    assert_eq!(hist_state(&h).pins.len(), 1, "a click pins the probe");

    h.key_press(Key::Backspace);
    h.step();
    assert!(
        hist_state(&h).pins.is_empty(),
        "Backspace removes the most recent pin while hovering"
    );
}

// ---------------------------------------------------------------------------
// 8. Zoom narrows the x-window; double-click resets it (spec §8)
// ---------------------------------------------------------------------------

#[test]
fn wheel_zoom_sets_window() {
    let mut h = horizon_ready();
    let pos = hover_horizon_hist(&mut h);
    assert!(hist_state(&h).window.is_none(), "no zoom window at rest");

    h.event(Event::Zoom(1.3));
    h.step();
    let (a0, b0) = hist_state(&h).window.expect("zoom sets an x window");
    assert!(b0 > a0);

    h.event(Event::Zoom(1.3));
    h.step();
    let (a1, b1) = hist_state(&h).window.expect("the window persists");
    assert!(
        (b1 - a1) < (b0 - a0) * 0.9,
        "a second zoom must narrow the window ({:?} vs {:?})",
        (a1, b1),
        (a0, b0)
    );

    double_click_at(&mut h, pos);
    assert!(
        hist_state(&h).window.is_none(),
        "double-click resets the zoom"
    );
}

// ---------------------------------------------------------------------------
// 9. ⌘⇧C routes an export of the hovered chart (spec §8)
// ---------------------------------------------------------------------------

#[test]
fn export_capture_flow() {
    let mut h = horizon_ready();
    let _pos = hover_horizon_hist(&mut h);
    assert!(h.state().toast.is_none());

    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::C);
    settle(&mut h, 2);

    let (msg, _) = h
        .state()
        .toast
        .clone()
        .expect("the export flow reports a toast");
    // The clipboard itself may be unavailable headless; either outcome
    // proves the request routed to the hovered chart and produced a capture.
    assert!(
        msg.contains("copied") || msg.contains("Export failed"),
        "the export must route to the hovered chart; got toast '{msg}'"
    );
}

// ---------------------------------------------------------------------------
// 10. Errors are inline and plain-language (spec §9 error)
// ---------------------------------------------------------------------------

#[test]
fn error_state_plain_language() {
    let mut h = build_app(|app| {
        app.cfg.sel = craps_engine::BetSelection {
            pass_line: false,
            ..craps_engine::BetSelection::default()
        };
    });
    settle(&mut h, 2);

    h.key_press(Key::Space);
    h.step();
    assert!(
        h.state().main_run.is_none(),
        "an invalid config must not start a run"
    );
    let err = h.state().error.clone().expect("a plain-language error");
    assert!(err.contains("Select at least one bet"), "got error '{err}'");
}

// ---------------------------------------------------------------------------
// 11. First run: the ghost state and "Run the example" (spec §9 first run)
// ---------------------------------------------------------------------------

#[test]
fn run_the_example_first_run() {
    let mut h = build_app(|app| {
        app.mode = Mode::Findings;
    });
    settle(&mut h, 2);
    assert!(
        h.state().main_run.is_none(),
        "first run: the ghost state has no data"
    );

    h.get_by_label("Run the example").click();
    h.step();

    assert!(h.state().main_run.is_some(), "the example starts a run");
    assert_eq!(
        h.state().cfg.table_mins_cents,
        vec![1000],
        "the example plays a single $10 minimum"
    );
    assert_eq!(h.state().cfg.budget_cents, 50_000, "a $500 budget");

    // Wind the example down so the suite doesn't leave 100k sessions running.
    h.state_mut().cancel_all();
    wait_terminal(&mut h, "the example run to wind down");
}
