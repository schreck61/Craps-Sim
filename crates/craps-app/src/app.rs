// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The instrument shell: one window, four modes on a top strip, the pinned
//! Scenario Sentence, the left rail (read-back + run control + seed +
//! progress lanes), and the permanent status baseline. The instrument never
//! forgets what it trades.

use egui::{Align, Color32, FontId, Key, Layout, Modifiers, RichText, Sense, Stroke};

use crate::config::SimConfig;
use crate::prefs::Prefs;
use crate::screens;
use crate::sentence::{self, FragmentId};
use crate::store::RunStatus;
use crate::stream::{start_explore_run, start_main_run, ExploreRun, MainRun};
use crate::ui::icons::{self, Icon};
use crate::ui::motion::{MotionPrefs, Tweens};
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale, Theme};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Design,
    Findings,
    Explorer,
    Replay,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindingsTab {
    Endurance,
    Horizon,
    Stake,
}

pub struct App {
    pub cfg: SimConfig,
    pub table_mins_text: String,
    pub seed: u64,
    pub prefs: Prefs,
    pub theme: Theme,
    pub motion: MotionPrefs,
    pub tweens: Tweens,
    pub highlights: crate::chart::Highlights,
    /// Charts mounted this frame (Principle 1 enforcement for StatHero).
    pub mounted: crate::components::stat_hero::MountedCharts,
    pub mode: Mode,
    pub findings_tab: FindingsTab,
    pub main_run: Option<MainRun>,
    pub explore_run: Option<ExploreRun>,
    /// Focused table minimum, persistent across modes (`[`/`]`).
    pub focused_min: usize,
    pub error: Option<String>,
    pub last_elapsed: Option<f64>,
    was_running: bool,
    /// A sentence fragment was clicked: Design focuses that control.
    pub focus_request: Option<FragmentId>,
    pub show_shortcuts: bool,
    pub ledger_drawer: bool,
    pub small_multiples: bool,
    pub ecdf_mode: bool,
    pub log_y: bool,
    /// Stake-screen exploration: the draggable budget line's position
    /// (display-only; never mutates the configuration).
    pub stake_budget_probe: Option<i64>,
    /// Stake-screen confidence handle (display-only, detents 90/95/99).
    pub stake_confidence: Option<f64>,
    /// Transient status toast: (message, shown-at time).
    pub toast: Option<(String, f64)>,
    /// The Anchor: per-(minimum, progression) Horizon variants on the same
    /// dice, plus the currently selected swap.
    pub anchor: crate::screens::findings::anchor::AnchorUi,
    /// Debug self-drive (`CRAPS_SIM_TOUR=dir`), None in normal runs.
    pub tour: Option<crate::tour::Tour>,
    /// The Rain: interpolation between real successive streaming snapshots
    /// of the focused histogram (never extrapolated; Principle 4).
    pub rain: crate::ui::motion::StreamInterp,
    /// (n at last snapshot, time of last snapshot) for the Rain's cadence.
    pub rain_mark: (u64, f64),
    /// Replay screen state (session under the lens).
    pub replay: screens::replay::ReplayState,
    /// Duel state (armed from the Explorer).
    pub duel: screens::duel::DuelState,
    /// Explorer UI state.
    pub explorer_ui: screens::explorer::ExplorerUi,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let prefs = Prefs::load();
        let dark = prefs
            .dark
            .unwrap_or_else(|| cc.egui_ctx.theme() == egui::Theme::Dark);
        let theme = if dark {
            Theme::lamplight()
        } else {
            Theme::reading_room()
        };
        theme::apply(&cc.egui_ctx, &theme);
        let motion = MotionPrefs {
            reduced: prefs.reduced_motion || crate::ui::motion::os_reduced_motion(),
        };
        let cfg = SimConfig::default();
        let mins_text = cfg
            .table_mins_cents
            .iter()
            .map(|&m| format!("{}", m / 100))
            .collect::<Vec<_>>()
            .join(", ");
        Self {
            cfg,
            table_mins_text: mins_text,
            seed: fresh_seed(),
            prefs,
            theme,
            motion,
            tweens: Tweens::default(),
            highlights: Default::default(),
            mounted: Default::default(),
            mode: Mode::Design,
            findings_tab: FindingsTab::Horizon,
            main_run: None,
            explore_run: None,
            focused_min: 0,
            error: None,
            last_elapsed: None,
            was_running: false,
            focus_request: None,
            show_shortcuts: false,
            ledger_drawer: false,
            small_multiples: false,
            ecdf_mode: false,
            log_y: false,
            stake_budget_probe: None,
            stake_confidence: None,
            toast: None,
            anchor: Default::default(),
            tour: crate::tour::Tour::from_env(),
            rain: Default::default(),
            rain_mark: (0, 0.0),
            replay: Default::default(),
            duel: Default::default(),
            explorer_ui: Default::default(),
        }
    }

    pub fn anything_running(&self) -> bool {
        self.main_run.as_ref().is_some_and(|r| r.is_running())
            || self.explore_run.as_ref().is_some_and(|r| r.is_running())
    }

    pub fn start_run(&mut self) {
        self.error = None;
        if let Err(e) = self.cfg.validate() {
            self.error = Some(e);
            return;
        }
        self.last_elapsed = None;
        self.anchor = Default::default();
        self.stake_budget_probe = None;
        self.stake_confidence = None;
        self.focused_min = self.focused_min.min(self.cfg.table_mins_cents.len() - 1);
        let run = start_main_run(&self.cfg, self.seed);
        run.focus(self.focused_min);
        self.main_run = Some(run);
        if self.mode == Mode::Design {
            self.mode = Mode::Findings;
        }
    }

    pub fn start_explore(&mut self) {
        self.error = None;
        if self.cfg.table_mins_cents.is_empty() {
            self.error = Some("Enter at least one table minimum.".to_owned());
            return;
        }
        self.last_elapsed = None;
        self.explore_run = Some(start_explore_run(&self.cfg, self.seed));
        self.mode = Mode::Explorer;
    }

    pub fn cancel_all(&mut self) {
        if let Some(r) = &self.main_run {
            r.cancel();
        }
        if let Some(r) = &self.explore_run {
            r.cancel();
        }
    }

    fn set_theme(&mut self, ctx: &egui::Context, dark: bool) {
        self.theme = if dark {
            Theme::lamplight()
        } else {
            Theme::reading_room()
        };
        theme::apply(ctx, &self.theme);
        self.prefs.dark = Some(dark);
        self.prefs.save();
    }

    /// The provenance config of the results currently on screen (for stale
    /// diffing), if any.
    pub fn provenance_config(&self) -> Option<SimConfig> {
        self.main_run
            .as_ref()
            .map(|r| r.store.lock().unwrap().provenance.config.clone())
    }

    /// True when the on-screen main results and the live settings disagree.
    pub fn is_stale(&self) -> bool {
        self.main_run.as_ref().is_some_and(|r| {
            r.store.lock().unwrap().provenance.fingerprint != self.cfg.fingerprint()
        })
    }

    fn toast(&mut self, ctx: &egui::Context, msg: &str) {
        self.toast = Some((msg.to_owned(), ctx.input(|i| i.time)));
    }

    /// ⌘⇧C / ⌘⇧S: export the chart under the pointer. The frame captures
    /// its own shape list next frame; `ui()` rasterizes and delivers.
    fn request_export(&mut self, ctx: &egui::Context, save: bool) {
        let hovered: Option<egui::Id> =
            ctx.data(|d| d.get_temp(egui::Id::new("last_hovered_chart")));
        match hovered {
            Some(id) => {
                ctx.data_mut(|d| {
                    d.insert_temp(egui::Id::new("export_request"), (id, save));
                });
                ctx.request_repaint();
            }
            None => self.toast(ctx, "hover a chart first — ⌘⇧C copies it, ⌘⇧S saves it"),
        }
    }

    /// Jump to Replay with one exactly re-simulated session under the lens.
    pub fn open_replay(&mut self, min_index: usize, session: u64) {
        self.replay
            .load(&self.cfg, self.main_run.as_ref(), min_index, session);
        self.mode = Mode::Replay;
    }

    pub fn cycle_min(&mut self, dir: i64) {
        let n = self.cfg.table_mins_cents.len();
        if n == 0 {
            return;
        }
        self.focused_min = ((self.focused_min as i64 + dir).rem_euclid(n as i64)) as usize;
        if let Some(r) = &self.main_run {
            r.focus(self.focused_min);
        }
    }

    // ------------------------------------------------------------------
    // Input
    // ------------------------------------------------------------------

    fn keyboard(&mut self, ctx: &egui::Context) {
        // Single-letter shortcuts stand down while a widget is being typed
        // into; command-modified shortcuts always work.
        let typing = ctx.memory(|m| m.focused().is_some());
        let cmd = Modifiers::COMMAND;

        if ctx.input_mut(|i| i.consume_key(cmd, Key::L)) {
            let dark = !self.theme.dark();
            self.set_theme(ctx, dark);
            return;
        }
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        // Native winit folds every Cmd+C (shift included) into Event::Copy
        // before any Key event exists, so both copy paths listen for it;
        // the consume_key forms remain for the headless test harness.
        let copy_event = ctx.input_mut(|i| {
            let had = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
            if had {
                let shift = i.modifiers.shift;
                i.events.retain(|e| !matches!(e, egui::Event::Copy));
                Some(shift)
            } else {
                None
            }
        });
        match copy_event {
            Some(true) => self.request_export(ctx, false),
            Some(false) if !typing => {
                ctx.copy_text(crate::sentence::render_text(&self.cfg));
                self.toast(
                    ctx,
                    "Scenario sentence copied — paste it into Design to restore",
                );
            }
            _ => {}
        }
        if ctx.input_mut(|i| i.consume_key(cmd_shift, Key::C)) {
            self.request_export(ctx, false);
        }
        if ctx.input_mut(|i| i.consume_key(cmd_shift, Key::S)) {
            self.request_export(ctx, true);
        }
        if typing {
            return;
        }
        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::Num1) {
                self.mode = Mode::Design;
            }
            if i.consume_key(Modifiers::NONE, Key::Num2) {
                self.mode = Mode::Findings;
            }
            if i.consume_key(Modifiers::NONE, Key::Num3) {
                self.mode = Mode::Explorer;
            }
            if i.consume_key(Modifiers::NONE, Key::Num4) {
                self.mode = Mode::Replay;
            }
            if i.consume_key(Modifiers::NONE, Key::Q) {
                self.mode = Mode::Findings;
                self.findings_tab = FindingsTab::Endurance;
            }
            if i.consume_key(Modifiers::NONE, Key::W) {
                self.mode = Mode::Findings;
                self.findings_tab = FindingsTab::Horizon;
            }
            if i.consume_key(Modifiers::NONE, Key::E) {
                if self.mode == Mode::Findings && self.findings_tab == FindingsTab::Horizon {
                    self.ecdf_mode = !self.ecdf_mode;
                } else {
                    self.mode = Mode::Findings;
                    self.findings_tab = FindingsTab::Stake;
                }
            }
            if i.consume_key(Modifiers::NONE, Key::R) {
                self.mode = Mode::Findings;
                self.findings_tab = FindingsTab::Stake;
            }
            if i.consume_key(Modifiers::NONE, Key::OpenBracket) {
                if self.mode == Mode::Replay {
                    self.replay.speed_down();
                } else {
                    self.cycle_min(-1);
                }
            }
            if i.consume_key(Modifiers::NONE, Key::CloseBracket) {
                if self.mode == Mode::Replay {
                    self.replay.speed_up();
                } else {
                    self.cycle_min(1);
                }
            }
            if i.consume_key(Modifiers::NONE, Key::L) {
                self.prefs.ledger_register = !self.prefs.ledger_register;
                self.prefs.save();
            }
            if i.consume_key(Modifiers::NONE, Key::M) {
                self.small_multiples = !self.small_multiples;
            }
            if i.consume_key(Modifiers::NONE, Key::Y) {
                self.log_y = !self.log_y;
            }
            if i.consume_key(Modifiers::NONE, Key::T) {
                self.ledger_drawer = !self.ledger_drawer;
            }
            if i.consume_key(Modifiers::NONE, Key::Questionmark)
                || i.consume_key(Modifiers::SHIFT, Key::Slash)
            {
                self.show_shortcuts = !self.show_shortcuts;
            }
        });

        // Space and Esc are handled with plain (unconsumed) reads so Replay
        // transport and text fields keep their own behavior.
        let space = !typing && ctx.input(|i| i.key_pressed(Key::Space));
        let esc = ctx.input(|i| i.key_pressed(Key::Escape));
        if space {
            if self.mode == Mode::Replay {
                self.replay.toggle_play();
            } else if self.anything_running() {
                self.cancel_all();
            } else {
                self.start_run();
            }
        }
        if esc {
            if self.show_shortcuts {
                self.show_shortcuts = false;
            } else if self.anything_running() {
                self.cancel_all();
            }
        }
        if self.mode == Mode::Replay {
            ctx.input_mut(|i| {
                if i.consume_key(Modifiers::NONE, Key::ArrowLeft) {
                    self.replay.step(-1);
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowRight) {
                    self.replay.step(1);
                }
                if i.consume_key(Modifiers::NONE, Key::G) {
                    self.replay.ghost = !self.replay.ghost;
                }
            });
        }
        // The Stake budget line is keyboard-reachable: ←/→ nudge it $10
        // (⇧ for $100); the mouse is optional (spec §8/§11).
        if self.mode == Mode::Findings && self.findings_tab == FindingsTab::Stake {
            let base = self.stake_budget_probe.unwrap_or(self.cfg.budget_cents);
            let step = |shift: bool| if shift { 10_000 } else { 1_000 };
            ctx.input_mut(|i| {
                let shift = i.modifiers.shift;
                if i.consume_key(Modifiers::NONE, Key::ArrowLeft)
                    || i.consume_key(Modifiers::SHIFT, Key::ArrowLeft)
                {
                    self.stake_budget_probe = Some((base - step(shift)).max(0));
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowRight)
                    || i.consume_key(Modifiers::SHIFT, Key::ArrowRight)
                {
                    self.stake_budget_probe = Some(base + step(shift));
                }
            });
        }
        if self.mode == Mode::Explorer {
            ctx.input_mut(|i| {
                if i.consume_key(Modifiers::NONE, Key::D) {
                    self.duel.open_from_selection(&self.explorer_ui);
                }
            });
        }
    }

    // ------------------------------------------------------------------
    // Chrome
    // ------------------------------------------------------------------

    fn mode_strip(&mut self, ui: &mut egui::Ui) {
        let t = self.theme.clone();
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(
                RichText::new("Craps Sim")
                    .font(FontId::new(17.0, theme::display()))
                    .color(t.ink),
            );
            ui.add_space(16.0);
            let stale = self.is_stale();
            for (mode, label, key) in [
                (Mode::Design, "Design", "1"),
                (Mode::Findings, "Findings", "2"),
                (Mode::Explorer, "Explorer", "3"),
                (Mode::Replay, "Replay", "4"),
            ] {
                let selected = self.mode == mode;
                let label = if mode == Mode::Findings && stale {
                    format!("{label} — stale")
                } else {
                    label.to_owned()
                };
                let text = RichText::new(label)
                    .font(FontId::new(
                        type_scale::BODY,
                        if selected {
                            theme::sans_semibold()
                        } else {
                            theme::sans()
                        },
                    ))
                    .color(if selected { t.ink } else { t.ink2 });
                let resp = ui
                    .add(egui::Button::new(text).fill(if selected {
                        t.surface2
                    } else {
                        Color32::TRANSPARENT
                    }))
                    .on_hover_text(format!("key {key}"));
                if resp.clicked() {
                    self.mode = mode;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icons::button(ui, Icon::Theme, t.ink2, t.blue, "theme · ⌘L").clicked() {
                    let dark = !self.theme.dark();
                    self.set_theme(ui.ctx(), dark);
                }
                if icons::button(ui, Icon::Ledger, t.ink2, t.blue, "ledger drawer · T").clicked() {
                    self.ledger_drawer = !self.ledger_drawer;
                }
            });
        });
    }

    fn sentence_bar(&mut self, ui: &mut egui::Ui) {
        let t = self.theme.clone();
        let provenance = self.provenance_config();
        let spans = sentence::render_spans(&self.cfg, provenance.as_ref());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for span in &spans {
                match span.frag {
                    None => {
                        ui.label(
                            RichText::new(&span.text)
                                .font(FontId::new(type_scale::BODY, theme::sans()))
                                .color(t.ink2),
                        );
                    }
                    Some(frag) => {
                        let mut text = RichText::new(&span.text)
                            .font(FontId::new(type_scale::BODY, theme::sans_semibold()))
                            .color(t.ink);
                        if span.stale {
                            text = text.underline().color(t.amber);
                        }
                        let resp = ui.add(egui::Label::new(text).sense(Sense::click()));
                        if resp.clicked() {
                            self.focus_request = Some(frag);
                            self.mode = Mode::Design;
                        }
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }
                }
            }
            ui.add_space(8.0);
            let copy = ui
                .add(
                    egui::Label::new(
                        RichText::new("⌘C")
                            .font(FontId::new(type_scale::CAPTION, theme::mono()))
                            .color(t.ink2),
                    )
                    .sense(Sense::click()),
                )
                .on_hover_text(
                    "copy the scenario sentence — pasting one into Design reconstructs it",
                );
            if copy.clicked() {
                ui.ctx().copy_text(sentence::render_text(&self.cfg));
            }
        });
    }

    fn rail(&mut self, ui: &mut egui::Ui) {
        let t = self.theme.clone();
        let running = self.anything_running();

        // Run control.
        let label = if running {
            "⏹  Cancel"
        } else {
            "Run  ·  Space"
        };
        let btn = egui::Button::new(
            RichText::new(label)
                .font(FontId::new(15.0, theme::sans_semibold()))
                .color(if running { t.ruin } else { t.ground }),
        )
        .fill(if running { t.surface2 } else { t.amber })
        .min_size(egui::vec2(ui.available_width(), 34.0));
        if ui.add(btn).clicked() {
            if running {
                self.cancel_all();
            } else {
                self.start_run();
            }
        }
        if !running {
            let explore = egui::Button::new(
                RichText::new("Explore 528 combos")
                    .font(FontId::new(type_scale::BODY, theme::sans_medium()))
                    .color(t.ink),
            )
            .fill(t.surface2)
            .min_size(egui::vec2(ui.available_width(), 26.0));
            if ui.add(explore).clicked() {
                self.start_explore();
            }
        }
        if let Some(err) = self.error.clone() {
            ui.add_space(4.0);
            let diagnostic = format!(
                "{}\nseed {} · v{APP_VERSION} · engine v{}",
                sentence::render_text(&self.cfg),
                numerals::seed_hex(self.seed),
                craps_engine::ENGINE_VERSION,
            );
            let panel = crate::components::error_panel::ErrorPanel {
                message: &err,
                focus: crate::components::error_panel::ErrorPanel::focus_for(&err),
                diagnostic,
            };
            if let crate::components::error_panel::ErrorAction::FocusControl(frag) =
                panel.show(ui, &t)
            {
                self.focus_request = Some(frag);
                self.mode = Mode::Design;
            }
        }
        ui.add_space(10.0);

        // Seed row: hexagon glyph · hex · copy · reroll.
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), Icon::Seed, rect, t.ink2);
            let seed_text = numerals::seed_hex(self.seed);
            let resp = ui
                .add(
                    egui::Label::new(
                        RichText::new(&seed_text)
                            .font(FontId::new(type_scale::CAPTION, theme::mono()))
                            .color(t.ink),
                    )
                    .sense(Sense::click()),
                )
                .on_hover_text("click to copy the seed");
            if resp.clicked() {
                ui.ctx().copy_text(seed_text);
            }
            if icons::button(ui, Icon::Replay, t.ink2, t.blue, "reroll the seed").clicked()
                && !running
            {
                self.seed = fresh_seed();
            }
        });
        ui.label(
            RichText::new(format!(
                "n = {} /min",
                numerals::compact_n(self.cfg.sessions as u64)
            ))
            .font(FontId::new(type_scale::CAPTION, theme::mono()))
            .color(t.ink2),
        );
        ui.add_space(10.0);
        ui.separator();

        // Read-back: the player at a glance (click → Design). The labels
        // must not be selectable: selectable text registers its own
        // click-sense widgets that would win the hit test over the scope's
        // interact — leaving the big number the user aims at inert.
        let readback = ui
            .scope(|ui| {
                ui.style_mut().interaction.selectable_labels = false;
                ui.label(
                    RichText::new(numerals::money_text(self.cfg.budget_cents, false))
                        .font(FontId::new(type_scale::STAT, theme::mono_medium()))
                        .color(t.ink),
                );
                let frags = sentence::render_spans(&self.cfg, None);
                for f in frags.iter().filter(|s| s.frag.is_some()) {
                    if matches!(
                        f.frag,
                        Some(FragmentId::Bets)
                            | Some(FragmentId::Progression)
                            | Some(FragmentId::Quit)
                    ) {
                        ui.label(
                            RichText::new(&f.text)
                                .font(FontId::new(type_scale::CAPTION, theme::sans()))
                                .color(t.ink2),
                        );
                    }
                }
            })
            .response;
        // The read-back is a door, not a control: signal it (cursor +
        // tooltip) and land on the Budget field with its focus ring lit so
        // the eye goes straight to where the number is edited.
        let readback = readback
            .interact(Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("click to edit the bankroll in Design");
        if readback.hovered() {
            let r = readback.rect.expand2(egui::vec2(4.0, 2.0));
            ui.painter().rect_stroke(
                r,
                4.0,
                Stroke::new(1.0, t.hairline_strong),
                egui::StrokeKind::Outside,
            );
        }
        if readback.clicked() {
            self.mode = Mode::Design;
            self.focus_request = Some(FragmentId::Budget);
        }
        ui.add_space(10.0);

        // Per-minimum progress lanes in chip order, filling as batches land.
        if let Some(run) = &self.main_run {
            let st = run.store.lock().unwrap();
            let sessions = st.provenance.sessions.max(1);
            ui.separator();
            for (mi, m) in st.mins.iter().enumerate() {
                // Digest count sees every session even under sampled
                // retention (raw columns hold only a stride sample there).
                let frac = (m.digests.finals_mean.count() as f32 / sessions as f32).min(1.0);
                let chip = t.chip(m.min_cents);
                ui.horizontal(|ui| {
                    let label = RichText::new(numerals::money_text(m.min_cents, false))
                        .font(FontId::new(type_scale::CAPTION, theme::mono()))
                        .color(if mi == self.focused_min {
                            t.ink
                        } else {
                            t.ink2
                        });
                    let resp = ui.add(egui::Label::new(label).sense(Sense::click()));
                    if resp.clicked() {
                        self.focused_min = mi;
                        run.focus(mi);
                    }
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width() - 26.0, 6.0),
                        Sense::hover(),
                    );
                    ui.painter().rect_filled(rect, 3.0, t.surface2);
                    // Completed lanes settle from 70% ink to full over the
                    // structural tier (spec §9); the fill itself is data
                    // and is never eased.
                    let settle = self.tweens.drive(
                        egui::Id::new(("lane_settle", mi)),
                        if frac >= 1.0 { 1.0 } else { 0.0 },
                        self.motion.duration(crate::ui::motion::STRUCTURAL),
                        ui.input(|i| i.stable_dt.min(0.1)),
                    );
                    let ink_alpha = (178.0 + 77.0 * settle) as u8;
                    let chip =
                        Color32::from_rgba_unmultiplied(chip.r(), chip.g(), chip.b(), ink_alpha);
                    let mut fill = rect;
                    fill.set_width(rect.width() * frac);
                    ui.painter().rect_filled(fill, 3.0, chip);
                    if frac >= 1.0 {
                        ui.label(
                            RichText::new("✓")
                                .font(FontId::new(type_scale::CAPTION, theme::sans()))
                                .color(t.gain),
                        );
                    }
                });
            }
            // Corner odometer: it reports; it does not spin.
            let n = run
                .ctl
                .sessions_done
                .load(std::sync::atomic::Ordering::Relaxed);
            let rolls = run
                .ctl
                .rolls_done
                .load(std::sync::atomic::Ordering::Relaxed);
            let dt = run.started.elapsed().as_secs_f64().max(0.001);
            if matches!(st.status, RunStatus::Streaming { .. }) {
                ui.label(
                    RichText::new(format!(
                        "{} / {} sessions · {:.1}M rolls/s",
                        numerals::compact_n(n),
                        numerals::compact_n(run.total_sessions),
                        rolls as f64 / dt / 1e6
                    ))
                    .font(FontId::new(type_scale::CAPTION, theme::mono()))
                    .color(t.ink2),
                );
            } else if let Some(el) = self.last_elapsed {
                ui.label(
                    RichText::new(format!("finished in {el:.1}s"))
                        .font(FontId::new(type_scale::CAPTION, theme::mono()))
                        .color(t.ink2),
                );
            }
        }
    }

    fn status_baseline(&self, ui: &mut egui::Ui) {
        let t = &self.theme;
        let min = self
            .cfg
            .table_mins_cents
            .get(self.focused_min)
            .copied()
            .unwrap_or(1000);
        let rules = self.cfg.rules();
        let edge = craps_engine::blended_edge(&self.cfg.sel, &rules, min);
        let mono11 = FontId::new(type_scale::CAPTION, theme::mono());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "seed {} · n = {}/min",
                    numerals::seed_hex(self.seed),
                    numerals::compact_n(self.cfg.sessions as u64)
                ))
                .font(mono11.clone())
                .color(t.ink2),
            );
            ui.label(RichText::new("·").color(t.ink2));
            match edge {
                Some(e) => {
                    // Pre-run (or flat/no-quit): closed-form drift; after a
                    // run with a progression: edge × simulated handle/hour.
                    let per_hour = self.dollars_per_hour(e, min);
                    ui.label(
                        RichText::new(format!(
                            "EDGE {} per resolved $ · ≈ {} at {} rolls/h{}",
                            numerals::edge_pct(e),
                            numerals::money_per_hour(per_hour.0),
                            self.cfg.rolls_per_hour,
                            per_hour.1
                        ))
                        .font(mono11.clone())
                        .color(t.amber),
                    );
                }
                None => {
                    ui.label(
                        RichText::new("EDGE — (no bets selected)")
                            .font(mono11.clone())
                            .color(t.amber),
                    );
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("v{APP_VERSION}"))
                        .font(mono11.clone())
                        .color(t.ink2),
                );
                if let Some((msg, _)) = &self.toast {
                    ui.label(RichText::new(msg).font(mono11).color(t.blue));
                }
            });
        });
    }

    /// (cents per hour, label suffix). Flat stakes use the closed-form
    /// drift; after a run with a pressing progression the simulated mean
    /// handle per hour carries the edge (E3), pair-link semantics.
    fn dollars_per_hour(&self, edge: f64, min: i64) -> (f64, &'static str) {
        use craps_engine::{flat_drift_per_roll_cents, Progression};
        let rules = self.cfg.rules();
        let flat = self.cfg.sel.progression == Progression::Flat;
        if !flat {
            if let Some(run) = &self.main_run {
                let st = run.store.lock().unwrap();
                if let Some(m) = st.mins.get(self.focused_min) {
                    let handle = m.digests.handle_cents_sum;
                    let rolls = m.digests.horizon_rolls_sum;
                    if rolls > 0 {
                        let handle_per_roll = handle as f64 / rolls as f64;
                        return (
                            edge * handle_per_roll * self.cfg.rolls_per_hour as f64,
                            " (simulated handle)",
                        );
                    }
                }
            }
        }
        (
            flat_drift_per_roll_cents(&self.cfg.sel, &rules, min) * self.cfg.rolls_per_hour as f64,
            if flat {
                " at flat stakes"
            } else {
                " at flat stakes (run for simulated)"
            },
        )
    }

    fn shortcuts_overlay(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let t = self.theme.clone();
        egui::Area::new(egui::Id::new("shortcuts"))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(t.surface2)
                    .stroke(Stroke::new(1.0, t.hairline_strong))
                    .corner_radius(8)
                    .inner_margin(20.0)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Keyboard")
                                .font(FontId::new(type_scale::TITLE, theme::display()))
                                .color(t.ink),
                        );
                        ui.add_space(8.0);
                        egui::Grid::new("keys").spacing([24.0, 4.0]).show(ui, |ui| {
                            for (k, what) in [
                                ("Space", "run / cancel (Replay: play/pause)"),
                                ("Esc", "cancel · close · reset brush"),
                                ("1–4", "Design · Findings · Explorer · Replay"),
                                ("Q / W / E", "Endurance · Horizon · Stake"),
                                ("[ ]", "cycle table minimum (Replay: speed)"),
                                ("L", "Story ↔ Ledger register"),
                                ("M", "small multiples"),
                                ("E", "histogram ↔ ECDF (Horizon)"),
                                ("Y", "log y (labeled)"),
                                ("T", "ledger drawer"),
                                ("D", "open the Duel from a selection"),
                                ("G", "ghost strategy (Replay)"),
                                ("← →", "step one roll (Replay)"),
                                ("⌘L", "theme"),
                                ("⌘⇧C / ⌘⇧S", "export chart: copy / save"),
                                ("drag on a chart", "brush a range → zoom to it"),
                                ("⌘-scroll", "zoom at the cursor"),
                                ("⇧-scroll", "pan while zoomed"),
                                ("double-click", "reset zoom"),
                                ("click on a chart", "pin a reference (up to 3) · ⌫ removes"),
                                ("?", "this overlay"),
                            ] {
                                ui.label(
                                    RichText::new(k)
                                        .font(FontId::new(type_scale::BODY, theme::mono()))
                                        .color(t.amber),
                                );
                                ui.label(
                                    RichText::new(what)
                                        .font(FontId::new(type_scale::BODY, theme::sans()))
                                        .color(t.ink),
                                );
                                ui.end_row();
                            }
                        });
                    });
            });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        crate::tour::drive(self, &ctx);
        self.highlights.begin_frame();
        let mounted_last_frame: Vec<egui::Id> = ctx.data_mut(|d| {
            let key = egui::Id::new("mounted_charts");
            let v: Vec<egui::Id> = d.get_temp(key).unwrap_or_default();
            d.insert_temp(key, Vec::<egui::Id>::new());
            v
        });
        self.mounted.set_frame(mounted_last_frame);
        self.keyboard(&ctx);

        // Completion edge: latch elapsed once when a run stops streaming.
        let running = self.anything_running();
        if self.was_running && !running {
            if let Some(r) = &self.main_run {
                self.last_elapsed = Some(r.started.elapsed().as_secs_f64());
            } else if let Some(r) = &self.explore_run {
                self.last_elapsed = Some(r.started.elapsed().as_secs_f64());
            }
        }
        self.was_running = running;
        if running {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.tweens.any_active() || self.replay.playing {
            ctx.request_repaint();
        }

        // The provenance sentence, for export chrome (frames read this key).
        let export_sentence = self
            .provenance_config()
            .map(|c| sentence::render_text(&c))
            .unwrap_or_else(|| sentence::render_text(&self.cfg));
        ctx.data_mut(|d| {
            d.insert_temp(egui::Id::new("scenario_sentence"), export_sentence);
        });
        if let Some((_, t0)) = self.toast {
            if ctx.input(|i| i.time) - t0 > 4.0 {
                self.toast = None;
            }
        }

        let t = self.theme.clone();
        egui::Panel::top("mode_strip")
            .exact_size(40.0)
            .show_separator_line(true)
            .show(ui, |ui| {
                ui.painter().rect_filled(ui.max_rect(), 0.0, t.ground);
                self.mode_strip(ui);
            });
        egui::Panel::top("sentence")
            .show_separator_line(true)
            .show(ui, |ui| {
                ui.add_space(4.0);
                self.sentence_bar(ui);
                ui.add_space(4.0);
            });
        egui::Panel::bottom("baseline")
            .exact_size(24.0)
            .show_separator_line(true)
            .show(ui, |ui| {
                self.status_baseline(ui);
            });
        egui::Panel::left("rail")
            .resizable(false)
            .exact_size(230.0)
            .show(ui, |ui| {
                ui.add_space(8.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.rail(ui);
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(t.ground).inner_margin(16.0))
            .show(ui, |ui| match self.mode {
                Mode::Design => screens::design::show(self, ui),
                Mode::Findings => screens::findings::show(self, ui),
                Mode::Explorer => screens::explorer::show(self, ui),
                Mode::Replay => screens::replay::show(self, ui),
            });

        // Deliver a captured export: rasterize device-independently at 2×
        // and hand it to the clipboard or a save dialog.
        type Captured = std::sync::Arc<(crate::chart::export::ExportBundle, bool, String)>;
        let key = egui::Id::new("captured_export");
        let captured: Option<Captured> = ctx.data_mut(|d| {
            let v: Option<Captured> = d.get_temp(key);
            if v.is_some() {
                d.remove::<Captured>(key);
            }
            v
        });
        if let Some(cap) = captured {
            let (bundle, save, title) = &*cap;
            let img = crate::chart::export::rasterize(&ctx, bundle, 2.0);
            let outcome = if *save {
                let name = format!(
                    "{}-{}.png",
                    title.to_lowercase().replace([' ', '—', '·'], "-"),
                    numerals::seed_hex(self.seed)
                );
                crate::chart::export::save_with_dialog(&img, &name).map(|p| match p {
                    Some(path) => format!("Saved {}", path.display()),
                    None => "Save cancelled".to_owned(),
                })
            } else {
                crate::chart::export::copy_to_clipboard(&img)
                    .map(|_| "Chart copied as a 2× PNG — provenance baked in".to_owned())
            };
            match outcome {
                Ok(m) => self.toast(&ctx, &m),
                Err(e) => self.toast(&ctx, &format!("Export failed: {e}")),
            }
        }

        self.shortcuts_overlay(&ctx);
    }
}

/// Fresh entropy for a new seed — drawn only when the user rerolls or at
/// first launch; the seed in use is always visible and copyable.
pub fn fresh_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E3779B97F4A7C15)
}
