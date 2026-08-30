// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Replay — the session theater. Any session from any run, exactly
//! re-simulated from its seed, played roll by roll against the population's
//! envelope. The replay is not a cartoon; it is evidence, and it says so.

use craps_engine::trace::{trace_session, SessionTrace};
use craps_engine::{session_seed, SeedPhase};
use egui::{FontId, RichText};

use crate::app::App;
use crate::chart::frame::ChartFrame;
use crate::chart::{dicestrip, trajectory};
use crate::config::SimConfig;
use crate::store::FanData;
use crate::stream::MainRun;
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale};

const SPEEDS: [f64; 4] = [1.0, 10.0, 60.0, 512.0];

pub struct ReplayState {
    pub trace: Option<SessionTrace>,
    /// The same night with its rules attributed, when a strategy played it.
    /// Everything else on this screen reads `trace`; only the Bench panel
    /// reads this.
    pub bench: Option<std::sync::Arc<craps_engine::strategy::BenchTrace>>,
    pub ghost_trace: Option<SessionTrace>,
    pub session: u64,
    pub min_index: usize,
    pub min_cents: i64,
    /// The provenance config the trace was cut from.
    pub config: Option<SimConfig>,
    pub seed: u64,
    /// Population envelope for "the one inside the many".
    pub envelope: Option<FanData>,
    pub playing: bool,
    /// Playhead in rolls (fractional accumulator).
    pub position: f64,
    pub speed_ix: usize,
    pub ghost: bool,
    /// The percentile description shown in the header, when known.
    pub header_note: String,
    /// Session-index entry buffer.
    pub entry: String,
    pub hovered_roll: Option<usize>,
}

impl Default for ReplayState {
    fn default() -> Self {
        Self {
            trace: None,
            bench: None,
            ghost_trace: None,
            session: 0,
            min_index: 0,
            min_cents: 0,
            config: None,
            seed: 0,
            envelope: None,
            playing: false,
            position: 0.0,
            // 60× out of the box: 1× is literal casino time (a roll every
            // ~36 s at 100 rolls/h), which reads as a broken play button.
            speed_ix: 2,
            ghost: false,
            header_note: String::new(),
            entry: String::new(),
            hovered_roll: None,
        }
    }
}

impl ReplayState {
    pub fn toggle_play(&mut self) {
        let Some(trace) = self.trace.as_ref() else {
            return;
        };
        if self.playing {
            self.playing = false;
            return;
        }
        // The theater opens fully revealed; play from the top when the
        // night is already complete.
        if self.position >= trace.events.len() as f64 {
            self.position = 0.0;
        }
        self.playing = true;
    }

    pub fn speed_up(&mut self) {
        self.speed_ix = (self.speed_ix + 1).min(SPEEDS.len() - 1);
    }

    pub fn speed_down(&mut self) {
        self.speed_ix = self.speed_ix.saturating_sub(1);
    }

    pub fn step(&mut self, dir: i64) {
        self.playing = false;
        let max = self.trace.as_ref().map(|t| t.events.len()).unwrap_or(0);
        self.position = (self.position.floor() + dir as f64).clamp(0.0, max as f64);
    }

    /// Load a session: exact re-simulation from provenance (Main phase),
    /// the same dice the sweep played.
    /// Load a night, played by `program` when a strategy is live.
    ///
    /// A strategy night is benched rather than merely traced, so the rules
    /// that produced it can be read beside it — which is the whole reason
    /// this screen is where the Bench belongs.
    pub fn load_with(
        &mut self,
        live_cfg: &SimConfig,
        run: Option<&MainRun>,
        min_index: usize,
        session: u64,
        program: Option<std::sync::Arc<craps_engine::strategy::Program>>,
    ) {
        let (cfg, seed, envelope) = match run {
            Some(r) => {
                let st = r.store.lock().unwrap();
                (
                    st.provenance.config.clone(),
                    st.provenance.seed,
                    st.mins.get(min_index).and_then(|m| m.fan.clone()),
                )
            }
            None => (live_cfg.clone(), 0, None),
        };
        let Some(&min_cents) = cfg.table_mins_cents.get(min_index) else {
            return;
        };
        let s = session_seed(seed, min_index as u32, SeedPhase::Main, session);
        // The theater plays the night as the Findings framed it: the
        // horizon window (the ruin view's continuation past it belongs to
        // Endurance's numbers, not this stage).
        let (trace, bench) = match &program {
            Some(p) => {
                let b = craps_engine::strategy::bench_session(
                    p,
                    &cfg.rules(),
                    min_cents,
                    cfg.budget_cents,
                    cfg.quit_target_cents(),
                    cfg.horizon_rolls(),
                    cfg.horizon_rolls(),
                    s,
                );
                (b.as_session_trace(), Some(std::sync::Arc::new(b)))
            }
            None => (
                trace_session(
                    &cfg.sel,
                    &cfg.rules(),
                    min_cents,
                    cfg.budget_cents,
                    cfg.quit_target_cents(),
                    cfg.horizon_rolls(),
                    cfg.horizon_rolls(),
                    s,
                ),
                None,
            ),
        };
        self.position = trace.events.len() as f64;
        self.trace = Some(trace);
        self.bench = bench;
        self.ghost_trace = None;
        self.session = session;
        self.min_index = min_index;
        self.min_cents = min_cents;
        self.config = Some(cfg);
        self.seed = s;
        self.envelope = envelope;
        self.playing = false;
        self.header_note.clear();
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    super::title(ui, &t, "Replay");

    // Quick-pick chips, always visible in the header.
    quick_picks(app, ui);

    let Some(trace) = app.replay.trace.as_ref() else {
        super::ghost_panel(
            ui,
            &t,
            "Pick a night — a quick-pick above, a session number, or any \"Show me one\" in Findings.",
        );
        return;
    };
    let cfg = app.replay.config.clone().unwrap_or_else(|| app.cfg.clone());
    let events_len = trace.events.len();

    // Header: evidence, not a cartoon.
    ui.label(
        RichText::new(format!(
            "Night #{}{} · seed {} · exactly re-simulated",
            app.replay.session,
            if app.replay.header_note.is_empty() {
                String::new()
            } else {
                format!(" — {}", app.replay.header_note)
            },
            numerals::seed_hex(app.replay.seed),
        ))
        .font(FontId::new(type_scale::CAPTION, theme::mono()))
        .color(t.ink2),
    );
    ui.add_space(6.0);

    // Advance the playhead in real computed rolls (never synthesized).
    // Playback must drive its own frames: egui only repaints on input, so
    // without this the playhead moves only while the mouse does.
    if app.replay.playing {
        let dt = ui.input(|i| i.stable_dt.min(0.1)) as f64;
        let rolls_per_sec = cfg.rolls_per_hour as f64 / 3600.0 * SPEEDS[app.replay.speed_ix];
        app.replay.position = (app.replay.position + dt * rolls_per_sec).min(events_len as f64);
        if app.replay.position >= events_len as f64 {
            app.replay.playing = false;
        } else {
            ui.ctx().request_repaint();
        }
    }
    let revealed = app.replay.position.floor() as usize;

    // The trajectory, inside the many.
    let outcome = trace.outcome;
    let end = if outcome.horizon.busted {
        Some((t.ruin, "bust".to_owned()))
    } else if outcome.horizon.hit_target {
        Some((t.gain, "quit ahead".to_owned()))
    } else {
        Some((t.ink2, "horizon".to_owned()))
    };
    let wealth: Vec<i64> = trace.events.iter().map(|e| e.wealth_after).collect();
    let ghost_wealth: Option<Vec<i64>> = app
        .replay
        .ghost
        .then(|| {
            app.replay
                .ghost_trace
                .as_ref()
                .map(|g| g.events.iter().map(|e| e.wealth_after).collect())
        })
        .flatten();

    let mut tracks = vec![trajectory::Trajectory {
        wealth: &wealth,
        color: t.chip(app.replay.min_cents),
        label: format!("night #{}", app.replay.session),
        peak_outlay_cents: outcome.peak_outlay_cents,
        end,
    }];
    if let Some(gw) = &ghost_wealth {
        tracks.push(trajectory::Trajectory {
            wealth: gw,
            color: t.violet,
            label: "ghost".to_owned(),
            peak_outlay_cents: 0,
            end: None,
        });
    }

    use craps_engine::{flat_drift_per_roll_cents, Progression};
    let drift_line =
        (cfg.sel.progression == Progression::Flat && cfg.quit_mult.is_none()).then(|| {
            (
                cfg.budget_cents as f64,
                flat_drift_per_roll_cents(&cfg.sel, &cfg.rules(), app.replay.min_cents),
            )
        });
    let envelope = app.replay.envelope.clone();
    let opts = trajectory::TrajectoryOptions {
        envelope: envelope.as_ref(),
        budget_cents: cfg.budget_cents,
        horizon_rolls: cfg.horizon_rolls() as usize,
        revealed,
        hours_per_roll: 1.0 / cfg.rolls_per_hour as f64,
        drift_line,
        simulated_mean_label: drift_line.is_none().then(|| {
            format!(
                "simulated mean (n = {})",
                numerals::compact_n(envelope.as_ref().map(|e| e.sampled_n).unwrap_or(0))
            )
        }),
    };
    ChartFrame::new(egui::Id::new("replay_traj"), "The night, inside the many")
        .run_key(
            (app.replay.seed ^ app.replay.session.rotate_left(32) ^ app.replay.min_index as u64)
                | 1,
        )
        .provenance(format!(
            "seed {} · every roll below is the roll the sweep played",
            numerals::seed_hex(app.replay.seed)
        ))
        .height(300.0)
        .show(ui, &t, |cx| {
            trajectory::paint(cx, &tracks, &opts);
        });

    // Transport.
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let (icon, tip) = if app.replay.playing {
            (crate::ui::icons::Icon::Stop, "pause · Space")
        } else {
            (crate::ui::icons::Icon::Run, "play · Space")
        };
        if crate::ui::icons::button(ui, icon, t.ink, t.blue, tip).clicked() {
            app.replay.toggle_play();
        }
        if ui.button("−1 · ←").clicked() {
            app.replay.step(-1);
        }
        if ui.button("+1 · →").clicked() {
            app.replay.step(1);
        }
        ui.label(
            RichText::new(format!("{}× · [ ]", SPEEDS[app.replay.speed_ix] as u64))
                .font(FontId::new(type_scale::BODY, theme::mono()))
                .color(t.ink),
        );
        let mut pos = app.replay.position;
        let slider = egui::Slider::new(&mut pos, 0.0..=events_len as f64)
            .show_value(false)
            .text("");
        if ui.add(slider).changed() {
            app.replay.position = pos;
            app.replay.playing = false;
        }
        ui.label(
            RichText::new(format!("roll {revealed} / {events_len}"))
                .font(FontId::new(type_scale::CAPTION, theme::mono()))
                .color(t.ink2),
        );
        let ghost_btn = ui.selectable_label(app.replay.ghost, "ghost · G");
        if ghost_btn.clicked() {
            app.replay.ghost = !app.replay.ghost;
        }
    });

    // The Bench: when a strategy played this night, its rules and the
    // attribution of every cent sit under the transport that steps it.
    super::bench::ledger(app, ui, revealed);

    // Ghost strategy on the same dice: the live Design selection replayed
    // against this night's identical rolls.
    if app.replay.ghost && app.replay.ghost_trace.is_none() {
        let g = trace_session(
            &app.cfg.sel,
            &app.cfg.rules(),
            app.replay.min_cents,
            app.cfg.budget_cents,
            app.cfg.quit_target_cents(),
            cfg.max_rolls.max(1000),
            cfg.horizon_rolls(),
            app.replay.seed,
        );
        app.replay.ghost_trace = Some(g);
    }
    if !app.replay.ghost {
        app.replay.ghost_trace = None;
    }

    // The dice strip: dice as data.
    ui.add_space(6.0);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 64.0), egui::Sense::hover());
    let mut hovered = app.replay.hovered_roll.take();
    if let Some(trace) = app.replay.trace.as_ref() {
        dicestrip::paint(
            ui,
            &t,
            rect,
            &trace.events,
            revealed.saturating_sub(1),
            &mut hovered,
        );
    }
    app.replay.hovered_roll = hovered;

    // Session-index entry: any night, by number.
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Enter a night #:")
                .font(FontId::new(type_scale::CAPTION, theme::sans()))
                .color(t.ink2),
        );
        let resp = ui.add(
            egui::TextEdit::singleline(&mut app.replay.entry)
                .desired_width(120.0)
                .font(egui::TextStyle::Monospace),
        );
        let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if submit {
            let n_sessions = app
                .replay
                .config
                .as_ref()
                .map(|c| c.sessions as u64)
                .unwrap_or(app.cfg.sessions as u64);
            match app.replay.entry.trim().parse::<u64>() {
                Ok(ix) if ix < n_sessions => {
                    let mi = app.replay.min_index;
                    app.open_replay(mi, ix);
                }
                Ok(ix) => {
                    app.error = Some(format!(
                        "Night #{ix} is out of range — this run simulated {n_sessions} per minimum."
                    ));
                }
                Err(_) => app.error = Some("Night numbers are plain integers.".to_owned()),
            }
        }
    });
}

/// Median night · P5 night · Best finish · Deepest drawdown.
fn quick_picks(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    let Some(run) = &app.main_run else { return };
    let store = run.store.clone();
    let st = store.lock().unwrap();
    let focused = app.focused_min.min(st.mins.len().saturating_sub(1));
    let Some(m) = st.mins.get(focused) else {
        return;
    };
    let Some(sv) = &m.sorted else {
        ui.label(
            RichText::new("quick-picks need the focused minimum's sorted views — run or wait")
                .font(FontId::new(type_scale::CAPTION, theme::sans()))
                .color(t.ink2),
        );
        return;
    };
    let n = sv.finals_rank_to_id.len();
    if n == 0 {
        return;
    }
    let picks = [
        (
            "Median night",
            sv.finals_rank_to_id[n / 2],
            "the median night",
        ),
        (
            "P5 night",
            sv.finals_rank_to_id[n / 20],
            "the 5th-percentile night",
        ),
        (
            "Best finish",
            sv.finals_rank_to_id[n - 1],
            "the best finish",
        ),
        (
            "Deepest drawdown",
            sv.outlays_rank_to_id[n - 1],
            "the deepest drawdown",
        ),
    ];
    drop(st);
    ui.horizontal(|ui| {
        for (label, id, note) in picks {
            if ui.button(label).clicked() {
                let mi = app.focused_min;
                app.open_replay(mi, id as u64);
                app.replay.header_note = note.to_owned();
            }
        }
    });
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded_state() -> ReplayState {
        let mut st = ReplayState::default();
        st.load_with(&SimConfig::default(), None, 0, 3, None);
        assert!(st.trace.is_some(), "trace loads without a run");
        st
    }

    /// The theater opens fully revealed and paused; play from the end
    /// rewinds to the top, play mid-night resumes in place, and a second
    /// press pauses.
    #[test]
    fn play_rewinds_from_the_end_and_resumes_mid_night() {
        let mut st = loaded_state();
        let len = st.trace.as_ref().unwrap().events.len() as f64;
        assert!(len > 0.0);
        assert_eq!(st.position, len, "opens fully revealed");
        assert!(!st.playing, "opens paused");

        st.toggle_play();
        assert!(st.playing);
        assert_eq!(st.position, 0.0, "play from the end starts over");

        st.position = len / 2.0;
        st.toggle_play();
        assert!(!st.playing, "second press pauses");
        assert_eq!(st.position, len / 2.0);

        st.toggle_play();
        assert!(st.playing, "mid-night play resumes in place");
        assert_eq!(st.position, len / 2.0);
    }

    /// 1× is literal casino time; the theater defaults to a speed a human
    /// can see moving.
    #[test]
    fn default_speed_is_visible() {
        assert!(SPEEDS[ReplayState::default().speed_ix] >= 10.0);
    }
}
