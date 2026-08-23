// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Craps-Sim — GUI, simulation orchestration, and the strategy
//! explorer. The game engine itself lives in [`sim`].

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod rng;
mod sim;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use rayon::prelude::*;
use sim::{
    BetSelection, HorizonStats, OddsPolicy, Progression, RuinStats, Rules, HARD_NUMS, PLACE_NUMS,
};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Craps-Sim",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}

struct MinRow {
    min_cents: i64,
    ruin: RuinStats,
    horizon: HorizonStats,
    budget_cents: i64,
}

#[derive(Clone)]
struct ExploreRow {
    strategy: &'static str,
    progression: &'static str,
    quit: Option<f64>,
    p_double: f64,
    p_ahead: f64,
    p_nobust: f64,
    median_final: i64,
    mean_final: f64,
    median_rolls: u64,
}

enum Msg {
    MinResult(MinRow),
    ExploreMin(i64, Vec<ExploreRow>),
    Done { elapsed_secs: f64, cancelled: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RankBy {
    DoubleUp,
    Ahead,
    Survive,
    MedianFinal,
    MeanFinal,
}

impl RankBy {
    const ALL: [RankBy; 5] = [
        RankBy::DoubleUp,
        RankBy::Ahead,
        RankBy::Survive,
        RankBy::MedianFinal,
        RankBy::MeanFinal,
    ];

    fn label(&self) -> &'static str {
        match self {
            RankBy::DoubleUp => "Chance to walk out with ≥ 2× budget",
            RankBy::Ahead => "Chance to walk out ahead",
            RankBy::Survive => "Chance to not bust",
            RankBy::MedianFinal => "Median walk-out money",
            RankBy::MeanFinal => "Mean walk-out money (EV)",
        }
    }

    fn key(&self, r: &ExploreRow) -> f64 {
        match self {
            RankBy::DoubleUp => r.p_double,
            RankBy::Ahead => r.p_ahead,
            RankBy::Survive => r.p_nobust,
            RankBy::MedianFinal => r.median_final as f64,
            RankBy::MeanFinal => r.mean_final,
        }
    }
}

/// The curated betting strategies the explorer sweeps. Progression and quit
/// rule are crossed with these separately.
fn explore_strategies() -> Vec<(&'static str, BetSelection)> {
    let none = BetSelection {
        pass_line: false,
        ..Default::default()
    };
    let mk = |f: &dyn Fn(&mut BetSelection)| {
        let mut s = none.clone();
        f(&mut s);
        s
    };
    vec![
        ("Pass line", mk(&|s| s.pass_line = true)),
        (
            "Pass + odds",
            mk(&|s| {
                s.pass_line = true;
                s.take_odds = true;
            }),
        ),
        (
            "3-pt Molly (pass + 2 come, odds)",
            mk(&|s| {
                s.pass_line = true;
                s.come_max = 2;
                s.take_odds = true;
            }),
        ),
        ("Don't pass", mk(&|s| s.dont_pass = true)),
        (
            "Don't pass + lay odds",
            mk(&|s| {
                s.dont_pass = true;
                s.take_odds = true;
            }),
        ),
        (
            "Don't + 2 don't come, odds",
            mk(&|s| {
                s.dont_pass = true;
                s.dont_come_max = 2;
                s.take_odds = true;
            }),
        ),
        (
            "Place 6 & 8",
            mk(&|s| {
                s.set_place(6, true);
                s.set_place(8, true);
            }),
        ),
        (
            "Inside numbers (5,6,8,9)",
            mk(&|s| {
                for n in [5, 6, 8, 9] {
                    s.set_place(n, true);
                }
            }),
        ),
        (
            "Iron Cross (field + 5,6,8)",
            mk(&|s| {
                s.field = true;
                for n in [5, 6, 8] {
                    s.set_place(n, true);
                }
            }),
        ),
        ("Field only", mk(&|s| s.field = true)),
        ("Hardways (all four)", mk(&|s| s.hardways = [true; 4])),
    ]
}

const EXPLORE_QUITS: [Option<f64>; 4] = [None, Some(1.5), Some(2.0), Some(3.0)];

struct RunHandle {
    rx: Receiver<Msg>,
    progress: Arc<AtomicU64>,
    total_work: u64,
    stop: Arc<AtomicBool>,
    started: Instant,
}

struct App {
    // --- simulation setup ---
    budget_dollars: f64,
    quit_on: bool,
    quit_mult: f64,
    sessions: u32,
    rolls_per_hour: u32,
    target_hours: f64,
    confidence: f64,
    table_mins_text: String,
    max_rolls: u64,
    sel: BetSelection,
    come_on: bool,
    come_max: u8,
    dont_come_on: bool,
    dont_come_max: u8,
    odds_policy: OddsPolicy,
    field_12_triple: bool,
    come_odds_work_on_comeout: bool,
    prop_bet_dollars: f64,
    table_max_mult: i64,
    // --- explorer ---
    explore_sessions: u32,
    rank_by: RankBy,
    // --- run state ---
    running: Option<RunHandle>,
    results: Vec<MinRow>,
    explore_results: Vec<(i64, Vec<ExploreRow>)>,
    showing_explore: bool,
    last_cancelled: bool,
    error: Option<String>,
    last_elapsed: Option<f64>,
    // parameters the visible results were computed with
    result_params: Option<ResultParams>,
}

#[derive(Clone)]
struct ResultParams {
    budget_dollars: f64,
    quit_mult: Option<f64>,
    progression: &'static str,
    bets: String,
    rolls_per_hour: u32,
    target_hours: f64,
    confidence: f64,
    sessions: u32,
}

impl Default for App {
    fn default() -> Self {
        Self {
            budget_dollars: 300.0,
            quit_on: false,
            quit_mult: 2.0,
            sessions: 100_000,
            rolls_per_hour: 100,
            target_hours: 4.0,
            confidence: 95.0,
            table_mins_text: "5, 10, 15, 25, 50, 100".to_owned(),
            max_rolls: 200_000,
            sel: BetSelection::default(),
            come_on: false,
            come_max: 2,
            dont_come_on: false,
            dont_come_max: 2,
            odds_policy: OddsPolicy::X345,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_dollars: 5.0,
            table_max_mult: 500,
            explore_sessions: 5_000,
            rank_by: RankBy::DoubleUp,
            running: None,
            results: Vec::new(),
            explore_results: Vec::new(),
            showing_explore: false,
            last_cancelled: false,
            error: None,
            last_elapsed: None,
            result_params: None,
        }
    }
}

/// Human-readable summary of the configured bets for the results header.
fn bets_summary(sel: &BetSelection, odds: OddsPolicy) -> String {
    let mut parts: Vec<String> = Vec::new();
    if sel.pass_line {
        parts.push("Pass".into());
    }
    if sel.dont_pass {
        parts.push("Don't pass".into());
    }
    if sel.come_max > 0 {
        parts.push(format!("Come×{}", sel.come_max));
    }
    if sel.dont_come_max > 0 {
        parts.push(format!("Don't come×{}", sel.dont_come_max));
    }
    if sel.take_odds {
        parts.push(format!("odds ({})", odds.label()));
    }
    if sel.field {
        parts.push("Field".into());
    }
    let places: Vec<String> = PLACE_NUMS
        .iter()
        .enumerate()
        .filter(|&(i, _)| sel.place[i])
        .map(|(_, n)| n.to_string())
        .collect();
    if !places.is_empty() {
        parts.push(format!("Place {}", places.join("/")));
    }
    let hards: Vec<String> = HARD_NUMS
        .iter()
        .enumerate()
        .filter(|&(i, _)| sel.hardways[i])
        .map(|(_, n)| n.to_string())
        .collect();
    if !hards.is_empty() {
        parts.push(format!("Hard {}", hards.join("/")));
    }
    if sel.any_seven {
        parts.push("Any 7".into());
    }
    if sel.any_craps {
        parts.push("Any craps".into());
    }
    parts.join(", ")
}

fn parse_table_mins(text: &str) -> Result<Vec<i64>, String> {
    let mut mins = Vec::new();
    for tok in text.split(|c: char| c == ',' || c.is_whitespace()) {
        if tok.is_empty() {
            continue;
        }
        let v: f64 = tok
            .trim_start_matches('$')
            .parse()
            .map_err(|_| format!("Could not parse table minimum \"{tok}\""))?;
        if !v.is_finite() || v > 100_000.0 {
            return Err(format!("Table minimum \"{tok}\" is out of range"));
        }
        let cents = (v * 100.0).round() as i64;
        if cents < 1 {
            return Err(format!("Table minimum \"{tok}\" must be at least $0.01"));
        }
        mins.push(cents);
    }
    if mins.is_empty() {
        return Err("Enter at least one table minimum".to_owned());
    }
    mins.sort_unstable();
    mins.dedup();
    Ok(mins)
}

/// A probability cell with its 95% confidence half-width, e.g. "46.8 ±1.4%".
fn pct_ci(p: f64, sessions: u32) -> String {
    let half = 1.96 * (p * (1.0 - p) / sessions.max(1) as f64).sqrt();
    format!("{:.1} ±{:.1}%", p * 100.0, half * 100.0)
}

fn dollars(cents: i64) -> String {
    if cents % 100 == 0 {
        format!("${}", cents / 100)
    } else {
        format!("${:.2}", cents as f64 / 100.0)
    }
}

impl App {
    fn start_run(&mut self) {
        self.error = None;
        self.sel.come_max = if self.come_on { self.come_max } else { 0 };
        self.sel.dont_come_max = if self.dont_come_on {
            self.dont_come_max
        } else {
            0
        };
        if !self.sel.any_selected() {
            self.error = Some("Select at least one bet type.".to_owned());
            return;
        }
        if self.sel.take_odds
            && !self.sel.pass_line
            && !self.sel.dont_pass
            && self.sel.come_max == 0
            && self.sel.dont_come_max == 0
        {
            self.error =
                Some("Odds require a Pass, Don't Pass, Come, or Don't Come bet.".to_owned());
            return;
        }
        let mins = match parse_table_mins(&self.table_mins_text) {
            Ok(m) => m,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };

        let sel = self.sel.clone();
        let rules = Rules {
            odds_policy: if self.sel.take_odds {
                self.odds_policy
            } else {
                OddsPolicy::None
            },
            field_12_triple: self.field_12_triple,
            come_odds_work_on_comeout: self.come_odds_work_on_comeout,
            prop_bet_cents: (self.prop_bet_dollars * 100.0).round().max(100.0) as i64,
            table_max_mult: self.table_max_mult.max(1),
        };
        let budget_cents = (self.budget_dollars * 100.0).round() as i64;
        // A budget below the cheapest bet at some table minimum would produce
        // degenerate all-zero rows; catch it up front.
        let unplayable: Vec<&i64> = mins
            .iter()
            .filter(|&&min| budget_cents < sim::cheapest_selected_stake(&self.sel, &rules, min))
            .collect();
        if !unplayable.is_empty() {
            self.error = Some(format!(
                "Budget ${:.0} can't cover the first bet at table minimum(s) {} — raise the budget or remove them.",
                self.budget_dollars,
                unplayable
                    .iter()
                    .map(|&&m| dollars(m))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            return;
        }
        let quit_target = if self.quit_on {
            Some(
                ((self.budget_dollars * self.quit_mult * 100.0).round() as i64)
                    .max(budget_cents + 100),
            )
        } else {
            None
        };
        let sessions = self.sessions.max(100) as u64;
        let max_rolls = self.max_rolls.max(1000);
        let horizon_rolls = (self.target_hours * self.rolls_per_hour as f64)
            .ceil()
            .max(1.0) as u64;
        let confidence = self.confidence;

        let progress = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let total_work = mins.len() as u64 * sessions * 2;
        let (tx, rx) = std::sync::mpsc::channel::<Msg>();

        let progress_w = progress.clone();
        let stop_w = stop.clone();
        // Fresh entropy per run so repeated runs differ.
        let base_seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15);

        std::thread::spawn(move || {
            run_worker(
                sel,
                rules,
                mins,
                budget_cents,
                quit_target,
                sessions,
                max_rolls,
                horizon_rolls,
                confidence,
                base_seed,
                progress_w,
                stop_w,
                tx,
            );
        });

        self.results.clear();
        self.showing_explore = false;
        self.last_cancelled = false;
        self.result_params = Some(ResultParams {
            budget_dollars: self.budget_dollars,
            quit_mult: self.quit_on.then_some(self.quit_mult),
            progression: self.sel.progression.label(),
            bets: bets_summary(&self.sel, self.odds_policy),
            rolls_per_hour: self.rolls_per_hour,
            target_hours: self.target_hours,
            confidence,
            sessions: self.sessions,
        });
        self.last_elapsed = None;
        self.running = Some(RunHandle {
            rx,
            progress,
            total_work,
            stop,
            started: Instant::now(),
        });
    }

    fn start_explore(&mut self) {
        self.error = None;
        let mins = match parse_table_mins(&self.table_mins_text) {
            Ok(m) => m,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        // The explorer sweeps bet selections, progressions, and quit rules;
        // payout rules (including the odds policy) come from the setup panel.
        let rules = Rules {
            odds_policy: self.odds_policy,
            field_12_triple: self.field_12_triple,
            come_odds_work_on_comeout: self.come_odds_work_on_comeout,
            prop_bet_cents: (self.prop_bet_dollars * 100.0).round().max(100.0) as i64,
            table_max_mult: self.table_max_mult.max(1),
        };
        let budget_cents = (self.budget_dollars * 100.0).round() as i64;
        let sessions = self.explore_sessions.max(500) as u64;
        let horizon_rolls = (self.target_hours * self.rolls_per_hour as f64)
            .ceil()
            .max(1.0) as u64;

        let strategies = explore_strategies();
        let combos =
            strategies.len() as u64 * Progression::ALL.len() as u64 * EXPLORE_QUITS.len() as u64;
        let progress = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let total_work = mins.len() as u64 * combos * sessions;
        let (tx, rx) = std::sync::mpsc::channel::<Msg>();

        let progress_w = progress.clone();
        let stop_w = stop.clone();
        let base_seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15);

        std::thread::spawn(move || {
            run_explore_worker(
                rules,
                mins,
                budget_cents,
                sessions,
                horizon_rolls,
                base_seed,
                progress_w,
                stop_w,
                tx,
            );
        });

        self.explore_results.clear();
        self.showing_explore = true;
        self.last_cancelled = false;
        self.result_params = Some(ResultParams {
            budget_dollars: self.budget_dollars,
            quit_mult: None,
            progression: "all progressions",
            bets: "11 betting strategies × 12 progressions × 4 quit rules".to_owned(),
            rolls_per_hour: self.rolls_per_hour,
            target_hours: self.target_hours,
            confidence: self.confidence,
            sessions: sessions as u32,
        });
        self.last_elapsed = None;
        self.running = Some(RunHandle {
            rx,
            progress,
            total_work,
            stop,
            started: Instant::now(),
        });
    }

    fn poll_worker(&mut self) {
        let mut finished = false;
        if let Some(run) = &self.running {
            while let Ok(msg) = run.rx.try_recv() {
                match msg {
                    Msg::MinResult(row) => self.results.push(row),
                    Msg::ExploreMin(min, rows) => self.explore_results.push((min, rows)),
                    Msg::Done {
                        elapsed_secs,
                        cancelled,
                    } => {
                        self.last_elapsed = Some(elapsed_secs);
                        self.last_cancelled = cancelled;
                        finished = true;
                    }
                }
            }
        }
        if finished {
            self.running = None;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_explore_worker(
    rules: Rules,
    mins: Vec<i64>,
    budget_cents: i64,
    sessions: u64,
    horizon_rolls: u64,
    base_seed: u64,
    progress: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    tx: Sender<Msg>,
) {
    let started = Instant::now();
    let strategies = explore_strategies();
    'mins: for (mi, &min) in mins.iter().enumerate() {
        let mut rows: Vec<ExploreRow> = Vec::new();
        // Common random numbers: the seed depends on the table minimum and
        // the session index but NOT the combo, so every strategy/progression/
        // quit combo plays the exact same dice sequences. Luck cancels out of
        // head-to-head comparisons, making rankings far tighter at the same
        // session count.
        let seed_base = base_seed ^ ((mi as u64 + 1) << 32);
        for (sname, sel_base) in &strategies {
            for prog in Progression::ALL {
                let mut sel = sel_base.clone();
                sel.progression = prog;
                for quit in EXPLORE_QUITS {
                    if stop.load(Ordering::Relaxed) {
                        break 'mins;
                    }
                    let quit_cents = quit.map(|m| {
                        ((budget_cents as f64 * m).round() as i64).max(budget_cents + 100)
                    });
                    let outs: Vec<(i64, bool, u64)> = (0..sessions)
                        .into_par_iter()
                        .map(|i| {
                            if stop.load(Ordering::Relaxed) {
                                return (0, false, 0);
                            }
                            let o = sim::run_horizon_session(
                                &sel,
                                &rules,
                                min,
                                budget_cents,
                                quit_cents,
                                horizon_rolls,
                                seed_base ^ i,
                            );
                            progress.fetch_add(1, Ordering::Relaxed);
                            (o.final_cents, o.busted, o.rolls)
                        })
                        .collect();
                    // A cancel mid-combo leaves sentinel zeros in `outs`;
                    // discard the partial combo rather than publish a
                    // contaminated row.
                    if stop.load(Ordering::Relaxed) {
                        break 'mins;
                    }
                    let n = outs.len() as f64;
                    let mut finals: Vec<i64> = outs.iter().map(|&(v, _, _)| v).collect();
                    let mut rolls: Vec<u64> = outs.iter().map(|&(_, _, r)| r).collect();
                    let busted = outs.iter().filter(|&&(_, b, _)| b).count() as f64;
                    finals.sort_unstable();
                    rolls.sort_unstable();
                    let frac_at_least = |thresh: i64| {
                        let idx = finals.partition_point(|&v| v < thresh);
                        (finals.len() - idx) as f64 / n
                    };
                    rows.push(ExploreRow {
                        strategy: sname,
                        progression: prog.label(),
                        quit,
                        p_double: frac_at_least(budget_cents * 2),
                        p_ahead: frac_at_least(budget_cents + 1),
                        p_nobust: 1.0 - busted / n,
                        median_final: finals[finals.len() / 2],
                        mean_final: finals.iter().map(|&v| v as f64).sum::<f64>() / n,
                        median_rolls: rolls[rolls.len() / 2],
                    });
                }
            }
        }
        let _ = tx.send(Msg::ExploreMin(min, rows));
    }
    let _ = tx.send(Msg::Done {
        elapsed_secs: started.elapsed().as_secs_f64(),
        cancelled: stop.load(Ordering::Relaxed),
    });
}

#[allow(clippy::too_many_arguments)]
fn run_worker(
    sel: BetSelection,
    rules: Rules,
    mins: Vec<i64>,
    budget_cents: i64,
    quit_target: Option<i64>,
    sessions: u64,
    max_rolls: u64,
    horizon_rolls: u64,
    confidence: f64,
    base_seed: u64,
    progress: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    tx: Sender<Msg>,
) {
    let started = Instant::now();
    for (mi, &min) in mins.iter().enumerate() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let mode_seed = |mode: u64, i: u64| base_seed ^ ((mi as u64 + 1) << 48) ^ (mode << 40) ^ i;

        // Phase 1: one pass per session answers both the "plays until bust or
        // quit" question and the "bankroll after the horizon" question — the
        // horizon view is a snapshot of the same trajectory as it crosses the
        // horizon roll count, so simulating it separately would be pure waste.
        let outcomes: Vec<(sim::RuinOutcome, sim::HorizonOutcome)> = (0..sessions)
            .into_par_iter()
            .map(|i| {
                if stop.load(Ordering::Relaxed) {
                    let zero_r = sim::RuinOutcome {
                        rolls: 0,
                        censored: false,
                        hit_target: false,
                    };
                    let zero_h = sim::HorizonOutcome {
                        final_cents: 0,
                        busted: false,
                        hit_target: false,
                        rolls: 0,
                    };
                    return (zero_r, zero_h);
                }
                let o = sim::run_session(
                    &sel,
                    &rules,
                    min,
                    budget_cents,
                    quit_target,
                    max_rolls,
                    horizon_rolls,
                    mode_seed(1, i),
                );
                progress.fetch_add(1, Ordering::Relaxed);
                (o.ruin, o.horizon)
            })
            .collect();
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let censored = outcomes.iter().filter(|(r, _)| r.censored).count() as u64;
        let hit_target = outcomes.iter().filter(|(r, _)| r.hit_target).count() as u64;
        let busted = outcomes.iter().filter(|(_, h)| h.busted).count() as u64;
        let quit_hits = outcomes.iter().filter(|(_, h)| h.hit_target).count() as u64;
        let mut rolls: Vec<u64> = outcomes.iter().map(|(r, _)| r.rolls).collect();
        let mut finals: Vec<i64> = outcomes.iter().map(|(_, h)| h.final_cents).collect();
        let ruin = sim::summarize_ruin(&mut rolls, censored, hit_target);
        let horizon = sim::summarize_horizon(&mut finals, busted, quit_hits);

        // Phase 2: bankroll needed to survive the target horizon.
        let mut outlays: Vec<i64> = (0..sessions)
            .into_par_iter()
            .map(|i| {
                if stop.load(Ordering::Relaxed) {
                    return 0;
                }
                let o =
                    sim::run_drawdown_session(&sel, &rules, min, horizon_rolls, mode_seed(2, i));
                progress.fetch_add(1, Ordering::Relaxed);
                o
            })
            .collect();
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let budget = sim::budget_for_confidence(&mut outlays, confidence);
        // Round up to a whole dollar.
        let budget = ((budget + 99) / 100) * 100;

        let _ = tx.send(Msg::MinResult(MinRow {
            min_cents: min,
            ruin,
            horizon,
            budget_cents: budget,
        }));
    }
    let _ = tx.send(Msg::Done {
        elapsed_secs: started.elapsed().as_secs_f64(),
        cancelled: stop.load(Ordering::Relaxed),
    });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_worker();
        if self.running.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }

        egui::Panel::left("setup")
            .resizable(true)
            .default_size(340.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.setup_ui(ui);
                });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both().show(ui, |ui| {
                self.results_ui(ui);
            });
        });
    }
}

impl App {
    fn setup_ui(&mut self, ui: &mut egui::Ui) {
        let busy = self.running.is_some();
        ui.add_space(4.0);
        ui.heading("Simulation Setup");
        ui.add_space(8.0);

        ui.add_enabled_ui(!busy, |ui| {
            egui::Grid::new("params")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Starting budget");
                    ui.add(
                        egui::DragValue::new(&mut self.budget_dollars)
                            .speed(10.0)
                            .range(1.0..=1_000_000.0)
                            .prefix("$"),
                    );
                    ui.end_row();

                    ui.label("Quit while ahead");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.quit_on, "at");
                        ui.add_enabled_ui(self.quit_on, |ui| {
                            ui.add(
                                egui::DragValue::new(&mut self.quit_mult)
                                    .speed(0.1)
                                    .range(1.1..=100.0)
                                    .suffix("× budget"),
                            );
                        });
                    });
                    ui.end_row();

                    ui.label("Sessions per table min");
                    ui.add(
                        egui::DragValue::new(&mut self.sessions)
                            .speed(1000.0)
                            .range(1_000..=10_000_000),
                    );
                    ui.end_row();

                    ui.label("Rolls per hour");
                    ui.add(
                        egui::DragValue::new(&mut self.rolls_per_hour)
                            .speed(1.0)
                            .range(30..=300),
                    );
                    ui.end_row();

                    ui.label("Target session length");
                    ui.add(
                        egui::DragValue::new(&mut self.target_hours)
                            .speed(0.25)
                            .range(0.5..=24.0)
                            .suffix(" h"),
                    );
                    ui.end_row();

                    ui.label("Survival confidence");
                    egui::ComboBox::from_id_salt("conf")
                        .selected_text(format!("{:.0}%", self.confidence))
                        .show_ui(ui, |ui| {
                            for c in [90.0, 95.0, 99.0] {
                                ui.selectable_value(&mut self.confidence, c, format!("{c:.0}%"));
                            }
                        });
                    ui.end_row();

                    ui.label("Roll cap per session");
                    ui.add(
                        egui::DragValue::new(&mut self.max_rolls)
                            .speed(1000.0)
                            .range(1_000..=5_000_000),
                    );
                    ui.end_row();
                });

            ui.add_space(6.0);
            ui.label("Table minimums ($, comma-separated):");
            ui.text_edit_singleline(&mut self.table_mins_text);

            ui.add_space(10.0);
            ui.separator();
            ui.heading("Bets");
            ui.add_space(4.0);

            ui.checkbox(&mut self.sel.pass_line, "Pass line");
            ui.checkbox(&mut self.sel.dont_pass, "Don't pass");
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.come_on, "Come");
                ui.add_enabled_ui(self.come_on, |ui| {
                    ui.label("up to");
                    ui.add(egui::DragValue::new(&mut self.come_max).range(1..=6));
                    ui.label("at once");
                });
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.dont_come_on, "Don't come");
                ui.add_enabled_ui(self.dont_come_on, |ui| {
                    ui.label("up to");
                    ui.add(egui::DragValue::new(&mut self.dont_come_max).range(1..=6));
                    ui.label("at once");
                });
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.sel.take_odds, "Take/lay odds (line + come)");
                ui.add_enabled_ui(self.sel.take_odds, |ui| {
                    egui::ComboBox::from_id_salt("odds")
                        .selected_text(self.odds_policy.label())
                        .show_ui(ui, |ui| {
                            for p in OddsPolicy::ALL {
                                if p != OddsPolicy::None {
                                    ui.selectable_value(&mut self.odds_policy, p, p.label());
                                }
                            }
                        });
                });
            });
            ui.checkbox(&mut self.sel.field, "Field (every roll)");

            ui.add_space(4.0);
            ui.label("Place bets:");
            ui.horizontal_wrapped(|ui| {
                for (i, n) in PLACE_NUMS.iter().enumerate() {
                    ui.checkbox(&mut self.sel.place[i], format!("{n}"));
                }
            });

            ui.add_space(4.0);
            ui.label("Hardways:");
            ui.horizontal_wrapped(|ui| {
                for (i, n) in HARD_NUMS.iter().enumerate() {
                    ui.checkbox(&mut self.sel.hardways[i], format!("Hard {n}"));
                }
            });

            ui.add_space(4.0);
            ui.label("One-roll props (every roll):");
            ui.checkbox(&mut self.sel.any_seven, "Any seven (4:1)");
            ui.checkbox(&mut self.sel.any_craps, "Any craps (7:1)");

            ui.add_space(10.0);
            ui.separator();
            ui.heading("Bet Pressing");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Progression:");
                egui::ComboBox::from_id_salt("progression")
                    .selected_text(self.sel.progression.label())
                    .show_ui(ui, |ui| {
                        for p in Progression::ALL {
                            ui.selectable_value(&mut self.sel.progression, p, p.label())
                                .on_hover_text(p.description());
                        }
                    });
            });
            ui.small(self.sel.progression.description());
            if self.sel.progression != Progression::Flat {
                ui.small(
                    "Each bet type presses independently; odds are sized from the pressed flat bet.",
                );
            }

            ui.add_space(10.0);
            ui.separator();
            ui.heading("Payout Rules");
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Field pays on 12:");
                egui::ComboBox::from_id_salt("field12")
                    .selected_text(if self.field_12_triple { "3:1" } else { "2:1" })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.field_12_triple, false, "2:1");
                        ui.selectable_value(&mut self.field_12_triple, true, "3:1");
                    });
            });
            ui.checkbox(
                &mut self.come_odds_work_on_comeout,
                "Come odds work on come-out",
            );
            ui.horizontal(|ui| {
                ui.label("Prop/hardway stake");
                ui.add(
                    egui::DragValue::new(&mut self.prop_bet_dollars)
                        .speed(1.0)
                        .range(1.0..=1000.0)
                        .prefix("$"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Table maximum");
                ui.add(
                    egui::DragValue::new(&mut self.table_max_mult)
                        .speed(10.0)
                        .range(10..=10_000)
                        .suffix("× min"),
                )
                .on_hover_text("Caps progressions the way a real table does.");
            });
        });

        ui.add_space(12.0);
        ui.separator();

        if let Some(err) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, err);
            ui.add_space(4.0);
        }

        if let Some(run) = &self.running {
            let done = run.progress.load(Ordering::Relaxed);
            let frac = done as f32 / run.total_work.max(1) as f32;
            ui.add(egui::ProgressBar::new(frac).show_percentage().animate(true));
            let elapsed = run.started.elapsed().as_secs_f64();
            let rate = done as f64 / elapsed.max(0.001);
            ui.label(format!(
                "{done} / {} sessions across all phases  ({:.0}k/s, {elapsed:.1}s elapsed)",
                run.total_work,
                rate / 1000.0
            ));
            if ui.button("⏹ Cancel").clicked() {
                run.stop.store(true, Ordering::Relaxed);
            }
        } else {
            let run_btn =
                egui::Button::new(egui::RichText::new("▶ Run Simulation").size(16.0).strong())
                    .min_size(egui::vec2(200.0, 34.0));
            if ui.add(run_btn).clicked() {
                self.start_run();
            }

            ui.add_space(10.0);
            ui.separator();
            ui.heading("Strategy Explorer");
            ui.small(
                "Sweeps 11 classic betting strategies × 12 progressions × quit rules (none/1.5×/2×/3×) \
                 for your budget and session length, then ranks the combos per table minimum. \
                 Uses the payout rules above; ignores the bet/pressing/quit selections.",
            );
            ui.small(format!(
                "Strategies that take odds will use: {}",
                self.odds_policy.label()
            ));
            ui.small(
                "Every combo plays the same dice (common random numbers), so \
                 head-to-head rankings aren't distorted by luck.",
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Sessions per combo");
                ui.add(
                    egui::DragValue::new(&mut self.explore_sessions)
                        .speed(500.0)
                        .range(500..=100_000),
                );
            });
            let explore_btn = egui::Button::new(
                egui::RichText::new("🔍 Explore Strategies")
                    .size(16.0)
                    .strong(),
            )
            .min_size(egui::vec2(200.0, 34.0));
            if ui.add(explore_btn).clicked() {
                self.start_explore();
            }

            if let Some(t) = self.last_elapsed {
                ui.label(format!(
                    "Last run finished in {t:.1}s on {} threads",
                    rayon::current_num_threads()
                ));
            }
        }
    }

    fn results_ui(&mut self, ui: &mut egui::Ui) {
        if self.showing_explore {
            self.explore_results_ui(ui);
            return;
        }
        ui.add_space(4.0);
        if self.result_params.is_none() {
            ui.heading("Welcome");
            ui.label("Configure the player on the left and press Run Simulation, or press Explore Strategies to sweep the strategy grid.");
            return;
        }
        let quit_mult = self.result_params.as_ref().and_then(|p| p.quit_mult);
        if let Some(m) = quit_mult {
            ui.heading(format!("Session Length (bust or quit at {m:.1}× budget)"));
        } else {
            ui.heading("Plays Until Bankroll Exhausted");
        }
        if self.last_cancelled && self.running.is_none() {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Run cancelled — showing partial results.",
            );
        }
        if let Some(p) = &self.result_params {
            ui.label(format!(
                "Budget {} · {} · {} · {} sessions per table minimum · a \"play\" is one dice roll · hours assume {} rolls/hour",
                format_args!("${:.0}", p.budget_dollars),
                p.bets,
                p.progression,
                p.sessions,
                p.rolls_per_hour,
            ));
        }
        ui.add_space(8.0);

        if self.results.is_empty() && self.running.is_some() {
            ui.spinner();
            ui.label("Simulating…");
            return;
        }

        let rph = self
            .result_params
            .as_ref()
            .map(|p| p.rolls_per_hour)
            .unwrap_or(100) as f64;

        egui::Grid::new("ruin_table")
            .striped(true)
            .spacing([18.0, 6.0])
            .show(ui, |ui| {
                for h in [
                    "Table min",
                    "Mean rolls (95% CI)",
                    "P5",
                    "P25",
                    "Median",
                    "P75",
                    "P95",
                    "Median hours",
                ] {
                    ui.strong(h);
                }
                if quit_mult.is_some() {
                    ui.strong("Quit ahead");
                }
                ui.strong("Hit roll cap");
                ui.end_row();

                for row in &self.results {
                    ui.label(dollars(row.min_cents));
                    ui.label(format!("{:.0} ± {:.0}", row.ruin.mean, row.ruin.ci95_half));
                    ui.label(row.ruin.p5.to_string());
                    ui.label(row.ruin.p25.to_string());
                    ui.label(row.ruin.p50.to_string());
                    ui.label(row.ruin.p75.to_string());
                    ui.label(row.ruin.p95.to_string());
                    ui.label(format!("{:.1}", row.ruin.p50 as f64 / rph));
                    if quit_mult.is_some() {
                        ui.label(format!("{:.1}%", row.ruin.target_frac * 100.0));
                    }
                    ui.label(format!("{:.1}%", row.ruin.censored_frac * 100.0));
                    ui.end_row();
                }
            });

        if self.results.iter().any(|r| r.ruin.censored_frac > 0.05) {
            ui.add_space(4.0);
            ui.small(
                "Rows where many sessions hit the roll cap under-state the true mean/upper percentiles; raise the cap for exact tails.",
            );
        }

        ui.add_space(18.0);
        if let Some(p) = &self.result_params {
            ui.heading(format!("Bankroll After {:.1} Hours", p.target_hours));
            let quit_note = match p.quit_mult {
                Some(m) => format!(
                    " Sessions that quit ahead at {m:.1}× keep their winnings for the rest of the horizon."
                ),
                None => String::new(),
            };
            ui.label(format!(
                "Money in hand at the end (live bets counted at face value), starting from ${:.0}. Each decile also shows the multiple of the starting budget.{quit_note}",
                p.budget_dollars
            ));
            ui.add_space(8.0);

            let budget_cents = (p.budget_dollars * 100.0).round().max(1.0);
            egui::Grid::new("horizon_table")
                .striped(true)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.strong("Table min");
                    ui.strong("Bust %");
                    if p.quit_mult.is_some() {
                        ui.strong("Quit ahead");
                    }
                    ui.strong("Mean");
                    for k in 1..=9 {
                        ui.strong(format!("P{}", k * 10));
                    }
                    ui.end_row();

                    for row in &self.results {
                        ui.label(dollars(row.min_cents));
                        ui.label(format!("{:.1}%", row.horizon.bust_frac * 100.0));
                        if p.quit_mult.is_some() {
                            ui.label(format!("{:.1}%", row.horizon.target_frac * 100.0));
                        }
                        ui.vertical(|ui| {
                            ui.label(format!("${:.0}", row.horizon.mean / 100.0));
                            ui.small(format!("{:.2}×", row.horizon.mean / budget_cents));
                        });
                        for &d in &row.horizon.deciles {
                            ui.vertical(|ui| {
                                ui.label(dollars(d));
                                ui.small(format!("{:.2}×", d as f64 / budget_cents));
                            });
                        }
                        ui.end_row();
                    }
                });
        }

        ui.add_space(18.0);
        if let Some(p) = &self.result_params {
            ui.heading(format!(
                "Recommended Budget for {:.1} Hours at {:.0}% Confidence",
                p.target_hours, p.confidence
            ));
            ui.label(format!(
                "Smallest bankroll that survives {:.0} rolls ({:.1} h × {} rolls/h) of this strategy in {:.0}% of sessions.",
                p.target_hours * rph,
                p.target_hours,
                p.rolls_per_hour,
                p.confidence
            ));
            if p.quit_mult.is_some() {
                ui.small(
                    "Computed without the quit-while-ahead rule: quitting early can only lower the chance of busting, so this guarantee still holds (slightly conservatively).",
                );
            }
        }
        ui.add_space(8.0);

        egui::Grid::new("budget_table")
            .striped(true)
            .spacing([18.0, 6.0])
            .show(ui, |ui| {
                ui.strong("Table min");
                ui.strong("Recommended budget");
                ui.strong("× table min");
                ui.end_row();
                for row in &self.results {
                    ui.label(dollars(row.min_cents));
                    ui.strong(dollars(row.budget_cents));
                    ui.label(format!(
                        "{:.0}×",
                        row.budget_cents as f64 / row.min_cents as f64
                    ));
                    ui.end_row();
                }
            });

        if self.running.is_some() && !self.results.is_empty() {
            ui.add_space(8.0);
            ui.spinner();
        }
    }

    fn explore_results_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.heading("Strategy Explorer");
        let (budget_dollars, hours, rph, sessions) = match &self.result_params {
            Some(p) => (
                p.budget_dollars,
                p.target_hours,
                p.rolls_per_hour as f64,
                p.sessions,
            ),
            None => return,
        };
        ui.label(format!(
            "Best strategy / pressing / quit combos for a ${budget_dollars:.0} budget over {hours:.1} hours. \
             \"Walk-out\" money counts early quits and busts at their ending value."
        ));
        ui.small(
            "Rows whose intervals overlap are statistically tied — raise sessions per combo to separate them.",
        );
        if self.last_cancelled && self.running.is_none() {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Run cancelled — table minimums finished before the cancel are shown; the rest were skipped.",
            );
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Rank by:");
            egui::ComboBox::from_id_salt("rank_by")
                .selected_text(self.rank_by.label())
                .show_ui(ui, |ui| {
                    for r in RankBy::ALL {
                        ui.selectable_value(&mut self.rank_by, r, r.label());
                    }
                });
        });
        ui.add_space(10.0);

        if self.explore_results.is_empty() {
            if self.running.is_some() {
                ui.spinner();
                ui.label("Sweeping the strategy grid…");
            }
            return;
        }

        let rank = self.rank_by;
        let quit_label = |q: Option<f64>| match q {
            Some(m) => format!("quit at {m:.1}×"),
            None => "no quit".to_owned(),
        };

        // Headline: the single best combo per table minimum.
        ui.strong("Best combo per table minimum");
        ui.add_space(4.0);
        egui::Grid::new("explore_best")
            .striped(true)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                for h in [
                    "Table min",
                    "Strategy",
                    "Pressing",
                    "Quit rule",
                    "≥2× budget",
                    "Ahead",
                    "No bust",
                    "Median end",
                    "Mean end",
                    "Median hrs",
                ] {
                    ui.strong(h);
                }
                ui.end_row();
                for (min, rows) in &self.explore_results {
                    if let Some(best) = rows.iter().max_by(|a, b| {
                        rank.key(a)
                            .partial_cmp(&rank.key(b))
                            .unwrap_or(std::cmp::Ordering::Equal)
                            .then_with(|| {
                                a.mean_final
                                    .partial_cmp(&b.mean_final)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            })
                    }) {
                        ui.label(dollars(*min));
                        ui.label(best.strategy);
                        ui.label(best.progression);
                        ui.label(quit_label(best.quit));
                        ui.label(pct_ci(best.p_double, sessions));
                        ui.label(pct_ci(best.p_ahead, sessions));
                        ui.label(pct_ci(best.p_nobust, sessions));
                        ui.label(dollars(best.median_final));
                        ui.label(format!("${:.0}", best.mean_final / 100.0));
                        ui.label(format!("{:.1}", best.median_rolls as f64 / rph));
                        ui.end_row();
                    }
                }
            });

        // Full top-10 per table minimum.
        for (min, rows) in &self.explore_results {
            ui.add_space(10.0);
            egui::CollapsingHeader::new(format!("Top 10 at {} tables", dollars(*min)))
                .id_salt(("explore_min", *min))
                .show(ui, |ui| {
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
                    egui::Grid::new(("explore_grid", *min))
                        .striped(true)
                        .spacing([16.0, 4.0])
                        .show(ui, |ui| {
                            for h in [
                                "#",
                                "Strategy",
                                "Pressing",
                                "Quit rule",
                                "≥2× budget",
                                "Ahead",
                                "No bust",
                                "Median end",
                                "Mean end",
                                "Median hrs",
                            ] {
                                ui.strong(h);
                            }
                            ui.end_row();
                            for (k, r) in sorted.iter().take(10).enumerate() {
                                ui.label(format!("{}", k + 1));
                                ui.label(r.strategy);
                                ui.label(r.progression);
                                ui.label(quit_label(r.quit));
                                ui.label(pct_ci(r.p_double, sessions));
                                ui.label(pct_ci(r.p_ahead, sessions));
                                ui.label(pct_ci(r.p_nobust, sessions));
                                ui.label(dollars(r.median_final));
                                ui.label(format!("${:.0}", r.mean_final / 100.0));
                                ui.label(format!("{:.1}", r.median_rolls as f64 / rph));
                                ui.end_row();
                            }
                        });
                });
        }

        if self.running.is_some() {
            ui.add_space(8.0);
            ui.spinner();
            ui.label("More table minimums on the way…");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_table_mins_happy_path() {
        assert_eq!(
            parse_table_mins("5, 10, 25").unwrap(),
            vec![500, 1000, 2500]
        );
        assert_eq!(parse_table_mins("$5 $10").unwrap(), vec![500, 1000]);
        assert_eq!(parse_table_mins("10, 5, 10").unwrap(), vec![500, 1000]); // sorted, deduped
        assert_eq!(parse_table_mins("7.50").unwrap(), vec![750]);
    }

    #[test]
    fn parse_table_mins_rejects_bad_input() {
        assert!(parse_table_mins("").is_err());
        assert!(parse_table_mins("abc").is_err());
        assert!(parse_table_mins("-5").is_err());
        assert!(parse_table_mins("0").is_err());
        assert!(parse_table_mins("nan").is_err());
        assert!(parse_table_mins("NaN").is_err());
        assert!(parse_table_mins("inf").is_err());
        assert!(parse_table_mins("0.001").is_err()); // rounds to zero cents
        assert!(parse_table_mins("1e9").is_err()); // out of range
        assert!(parse_table_mins("5, nan, 10").is_err());
    }

    #[test]
    fn bets_summary_reads_naturally() {
        let mut sel = BetSelection {
            take_odds: true,
            come_max: 2,
            ..Default::default()
        };
        sel.set_place(6, true);
        sel.set_place(8, true);
        let s = bets_summary(&sel, OddsPolicy::X345);
        assert_eq!(s, "Pass, Come×2, odds (3-4-5x odds), Place 6/8");
    }

    /// Investigation harness: near-ground-truth explorer leaderboard for a
    /// $1,000 budget at low table minimums, plus rank-churn analysis at the
    /// default explorer precision. Run with:
    ///   cargo test --release -- --ignored explorer_ --nocapture
    #[test]
    #[ignore]
    fn explorer_ground_truth_1000_budget() {
        use rayon::prelude::*;
        let rules = Rules {
            odds_policy: OddsPolicy::X345,
            field_12_triple: false,
            come_odds_work_on_comeout: false,
            prop_bet_cents: 500,
            table_max_mult: 500,
        };
        let budget = 100_000i64; // $1,000
        let horizon = 400u64;
        let strategies = explore_strategies();

        let leaderboard = |min: i64, sessions: u64, base_seed: u64| -> Vec<(String, f64)> {
            let mut rows = Vec::new();
            for (sname, sel_base) in &strategies {
                for prog in Progression::ALL {
                    let mut sel = sel_base.clone();
                    sel.progression = prog;
                    for quit in EXPLORE_QUITS {
                        let quit_cents =
                            quit.map(|m| ((budget as f64 * m).round() as i64).max(budget + 100));
                        let doubles = (0..sessions)
                            .into_par_iter()
                            .filter(|&i| {
                                sim::run_horizon_session(
                                    &sel,
                                    &rules,
                                    min,
                                    budget,
                                    quit_cents,
                                    horizon,
                                    base_seed ^ ((min as u64) << 40) ^ i,
                                )
                                .final_cents
                                    >= budget * 2
                            })
                            .count();
                        let q = match quit {
                            Some(m) => format!("quit {m:.1}x"),
                            None => "no quit".to_owned(),
                        };
                        rows.push((
                            format!("{sname} | {} | {q}", prog.label()),
                            doubles as f64 / sessions as f64,
                        ));
                    }
                }
            }
            rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            rows
        };

        for &min in &[500i64, 1000, 1500] {
            let sessions = 50_000u64;
            let rows = leaderboard(min, sessions, 0xC0FFEE);
            println!(
                "=== ${} min, {}k sessions/combo (ground truth) ===",
                min / 100,
                sessions / 1000
            );
            for (k, (name, p)) in rows.iter().take(8).enumerate() {
                let se = (p * (1.0 - p) / sessions as f64).sqrt();
                println!(
                    "  {}. {:5.2}% +-{:.2}%  {}",
                    k + 1,
                    p * 100.0,
                    se * 100.0,
                    name
                );
            }
        }

        println!();
        println!("=== $5 min, default 5k sessions, five independent runs (rank churn) ===");
        for run in 0..5u64 {
            let rows = leaderboard(500, 5_000, 0x1111_2222 ^ (run << 48));
            let top: Vec<String> = rows
                .iter()
                .take(3)
                .map(|(n, p)| format!("{n} ({:.1}%)", p * 100.0))
                .collect();
            println!("  run {}:", run + 1);
            for t in top {
                println!("     {t}");
            }
        }
    }
}
