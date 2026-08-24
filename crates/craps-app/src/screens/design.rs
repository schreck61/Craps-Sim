// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Design — build the player. A central bet rail (each bet priced with its
//! amber edge tick at the moment of choice, summing to the blended edge
//! strip) and a right column for bankroll, table, house rules, and the
//! Engine disclosure. Order-ticket validation runs live in the footer.

use craps_engine::{
    bet_edge, blended_edge, EdgeBet, OddsPolicy, Progression, HARD_NUMS, PLACE_NUMS,
};
use egui::{Align2, Color32, FontId, RichText, Sense, Stroke};

use crate::app::App;
use crate::config::parse_table_mins;
use crate::sentence::{self, FragmentId};
use crate::ui::numerals;
use crate::ui::theme::{self, type_scale};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    ui.label(
        RichText::new("Design")
            .font(FontId::new(type_scale::TITLE, theme::display()))
            .color(t.ink),
    );
    ui.add_space(8.0);

    // A sentence click focuses a control; the ring lingers long enough to
    // find (spec §5's click-to-focus, not a single-frame blink).
    let ring_key = egui::Id::new("design_focus_ring");
    // `now` is read BEFORE the data closure: Context::data holds the ctx
    // read lock while the closure runs, and Context::input wants the write
    // lock on the same RwLock — nesting them deadlocks the UI thread.
    let now = ui.input(|i| i.time);
    if let Some(frag) = app.focus_request.take() {
        ui.ctx()
            .data_mut(|d| d.insert_temp(ring_key, (frag, now + 1.4)));
    }
    let focus = ui.ctx().data(|d| {
        d.get_temp::<(FragmentId, f64)>(ring_key)
            .filter(|(_, until)| now < *until)
            .map(|(f, _)| f)
    });
    if focus.is_some() {
        ui.ctx().request_repaint();
    }
    let busy = app.anything_running();

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_enabled_ui(!busy, |ui| {
            ui.columns(2, |cols| {
                bet_rail(app, &mut cols[0], focus);
                right_column(app, &mut cols[1], focus);
            });
        });
        ui.add_space(10.0);
        order_ticket(app, ui);
        ui.add_space(10.0);
        paste_sentence(app, ui);
    });
}

/// A section's full-width rect from its start cursor to the current one.
/// (`ui.cursor().min` carries no width — the ring must span the column, not
/// collapse to a zero-width sliver at its left edge.)
fn section_rect(ui: &egui::Ui, start: egui::Pos2, y_pad: f32) -> egui::Rect {
    egui::Rect::from_min_max(
        start,
        egui::pos2(ui.max_rect().right(), ui.cursor().min.y + y_pad),
    )
}

/// Highlight ring + scroll for a section the sentence focused.
fn focus_ring(
    app: &App,
    ui: &mut egui::Ui,
    rect: egui::Rect,
    mine: FragmentId,
    focus: Option<FragmentId>,
) {
    if focus == Some(mine) {
        ui.scroll_to_rect(rect, Some(egui::Align::Center));
        ui.painter().rect_stroke(
            rect.expand(4.0),
            6.0,
            Stroke::new(1.0, app.theme.blue),
            egui::StrokeKind::Outside,
        );
    }
}

fn section_heading(ui: &mut egui::Ui, t: &crate::ui::theme::Theme, text: &str) {
    ui.add_space(10.0);
    ui.label(
        RichText::new(text)
            .font(FontId::new(type_scale::SECTION, theme::sans_semibold()))
            .color(t.ink),
    );
    ui.add_space(4.0);
}

/// One bet card row: the control plus its consequence — the amber edge tick.
fn edge_tick(ui: &mut egui::Ui, t: &crate::ui::theme::Theme, edge: f64) {
    // Bar length linear in |edge| (full width at 16.67%, the worst bet).
    let w = 72.0 * (edge.abs() / (1.0 / 6.0)).min(1.0) as f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(76.0, 12.0), Sense::hover());
    let bar =
        egui::Rect::from_min_size(rect.min + egui::vec2(0.0, 3.0), egui::vec2(w.max(2.0), 6.0));
    ui.painter().rect_filled(bar, 2.0, t.amber);
    ui.label(
        RichText::new(numerals::edge_pct(edge))
            .font(FontId::new(type_scale::CAPTION, theme::mono()))
            .color(t.amber),
    );
}

fn bet_rail(app: &mut App, ui: &mut egui::Ui, focus: Option<FragmentId>) {
    let t = app.theme.clone();
    let rules = app.cfg.rules();
    let start = ui.cursor().min;

    section_heading(ui, &t, "Bets");
    let sel = &mut app.cfg.sel;

    ui.horizontal(|ui| {
        ui.checkbox(&mut sel.pass_line, "Pass line");
        edge_tick(ui, &t, bet_edge(EdgeBet::Pass, &rules).as_f64());
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut sel.dont_pass, "Don't pass");
        edge_tick(ui, &t, bet_edge(EdgeBet::DontPass, &rules).as_f64());
    });
    ui.horizontal(|ui| {
        let mut on = sel.come_max > 0;
        ui.checkbox(&mut on, "Come");
        if on && sel.come_max == 0 {
            sel.come_max = 2;
        }
        if !on {
            sel.come_max = 0;
        }
        if on {
            ui.label("up to");
            let mut v = sel.come_max;
            ui.add(egui::DragValue::new(&mut v).range(1..=6));
            sel.come_max = v;
            ui.label("at once");
        }
        edge_tick(ui, &t, bet_edge(EdgeBet::Come, &rules).as_f64());
    });
    ui.horizontal(|ui| {
        let mut on = sel.dont_come_max > 0;
        ui.checkbox(&mut on, "Don't come");
        if on && sel.dont_come_max == 0 {
            sel.dont_come_max = 2;
        }
        if !on {
            sel.dont_come_max = 0;
        }
        if on {
            ui.label("up to");
            let mut v = sel.dont_come_max;
            ui.add(egui::DragValue::new(&mut v).range(1..=6));
            sel.dont_come_max = v;
            ui.label("at once");
        }
        edge_tick(ui, &t, bet_edge(EdgeBet::DontCome, &rules).as_f64());
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut sel.take_odds, "Take/lay odds (line + come)");
        if sel.take_odds {
            egui::ComboBox::from_id_salt("odds")
                .selected_text(app.cfg.odds_policy.label())
                .show_ui(ui, |ui| {
                    for p in OddsPolicy::ALL {
                        if p != OddsPolicy::None {
                            ui.selectable_value(&mut app.cfg.odds_policy, p, p.label());
                        }
                    }
                });
            ui.label(
                RichText::new("edge exactly 0 — the only free bet in the house")
                    .font(FontId::new(type_scale::CAPTION, theme::sans()))
                    .color(t.ink2),
            );
        }
    });
    let sel = &mut app.cfg.sel;
    ui.horizontal(|ui| {
        ui.checkbox(&mut sel.field, "Field (every roll)");
        edge_tick(ui, &t, bet_edge(EdgeBet::Field, &rules).as_f64());
    });

    ui.add_space(4.0);
    ui.label("Place bets:");
    ui.horizontal_wrapped(|ui| {
        for (i, &n) in PLACE_NUMS.iter().enumerate() {
            ui.checkbox(&mut sel.place[i], format!("{n}"));
        }
        edge_tick(ui, &t, bet_edge(EdgeBet::Place(6), &rules).as_f64());
        ui.label(
            RichText::new("(6/8 · 5/9 and 4/10 cost more)")
                .font(FontId::new(type_scale::CAPTION, theme::sans()))
                .color(t.ink2),
        );
    });
    ui.add_space(4.0);
    ui.label("Hardways:");
    ui.horizontal_wrapped(|ui| {
        for (i, &n) in HARD_NUMS.iter().enumerate() {
            ui.checkbox(&mut sel.hardways[i], format!("Hard {n}"));
        }
        edge_tick(ui, &t, bet_edge(EdgeBet::Hardway(6), &rules).as_f64());
    });
    ui.add_space(4.0);
    ui.label("One-roll props (every roll):");
    ui.horizontal(|ui| {
        ui.checkbox(&mut sel.any_seven, "Any seven (4:1)");
        edge_tick(ui, &t, bet_edge(EdgeBet::AnySeven, &rules).as_f64());
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut sel.any_craps, "Any craps (7:1)");
        edge_tick(ui, &t, bet_edge(EdgeBet::AnyCraps, &rules).as_f64());
    });

    let bets_rect = section_rect(ui, start, -4.0);
    focus_ring(app, ui, bets_rect, FragmentId::Bets, focus);

    // The blended edge strip: one amber bar for the whole player.
    ui.add_space(8.0);
    let min = app
        .cfg
        .table_mins_cents
        .get(app.focused_min)
        .copied()
        .unwrap_or(1000);
    match blended_edge(&app.cfg.sel, &rules, min) {
        Some(e) => {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width().min(340.0), 18.0),
                Sense::hover(),
            );
            let w = rect.width() * (e.abs() / (1.0 / 6.0)).min(1.0) as f32;
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    rect.min + egui::vec2(0.0, 5.0),
                    egui::vec2(w.max(2.0), 8.0),
                ),
                2.0,
                t.amber,
            );
            ui.painter().text(
                rect.right_center() + egui::vec2(-2.0, 0.0),
                Align2::RIGHT_CENTER,
                format!("blended {}", numerals::edge_pct(e)),
                FontId::new(type_scale::CAPTION, theme::mono()),
                t.amber,
            );
            ui.label(
                RichText::new("Every dollar you put down costs this on average, no matter what.")
                    .font(FontId::new(type_scale::CAPTION, theme::sans()))
                    .color(t.ink2),
            );
        }
        None => {
            ui.label(
                RichText::new("Select at least one bet.")
                    .font(FontId::new(type_scale::CAPTION, theme::sans()))
                    .color(t.ink2),
            );
        }
    }

    // Progression picker: 12 cards with a stake-sequence sparkline each.
    section_heading(ui, &t, "Bet Pressing");
    let prog_start = ui.cursor().min;
    egui::Grid::new("progressions")
        .num_columns(3)
        .spacing([8.0, 8.0])
        .show(ui, |ui| {
            for (k, p) in Progression::ALL.into_iter().enumerate() {
                progression_card(app, ui, p);
                if k % 3 == 2 {
                    ui.end_row();
                }
            }
        });
    let prog_rect = section_rect(ui, prog_start, 0.0);
    focus_ring(app, ui, prog_rect, FragmentId::Progression, focus);
    if app.cfg.sel.progression != Progression::Flat {
        ui.label(
            RichText::new(
                "Each bet type presses independently; odds are sized from the pressed flat bet.",
            )
            .font(FontId::new(type_scale::CAPTION, theme::sans()))
            .color(t.ink2),
        );
    }
}

/// One progression card: name, always-visible description, and a 60×20
/// stake-sequence sparkline over the canonical W-W-L-W-L-L pattern.
fn progression_card(app: &mut App, ui: &mut egui::Ui, p: Progression) {
    let t = app.theme.clone();
    let selected = app.cfg.sel.progression == p;
    let fill = if selected { t.surface2 } else { t.surface };
    let stroke = if selected {
        Stroke::new(1.0, t.blue)
    } else {
        Stroke::new(1.0, t.hairline)
    };
    let resp = egui::Frame::NONE
        .fill(fill)
        .stroke(stroke)
        .corner_radius(6)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_width(210.0);
            ui.set_min_height(96.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(p.label())
                            .font(FontId::new(type_scale::BODY, theme::sans_medium()))
                            .color(t.ink),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        sparkline(ui, &t, p)
                    });
                });
                ui.add(
                    egui::Label::new(
                        RichText::new(p.description())
                            .font(FontId::new(type_scale::CAPTION, theme::sans()))
                            .color(t.ink2),
                    )
                    .wrap(),
                );
            });
        })
        .response
        .interact(Sense::click());
    if resp.clicked() {
        app.cfg.sel.progression = p;
    }
}

/// The stake sequence over W-W-L-W-L-L, drawn as a step line. Martingale's
/// staircase is visible before it is chosen.
fn sparkline(ui: &mut egui::Ui, t: &crate::ui::theme::Theme, p: Progression) {
    let stakes = craps_engine::progression_preview(p);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(60.0, 20.0), Sense::hover());
    let max = stakes.iter().copied().fold(1i64, i64::max) as f32;
    let n = stakes.len() as f32;
    let mut pts = Vec::with_capacity(stakes.len() * 2);
    for (i, &s) in stakes.iter().enumerate() {
        let x0 = rect.left() + rect.width() * i as f32 / n;
        let x1 = rect.left() + rect.width() * (i as f32 + 1.0) / n;
        let y = rect.bottom() - (rect.height() - 2.0) * (s as f32 / max);
        pts.push(egui::pos2(x0, y));
        pts.push(egui::pos2(x1, y));
    }
    ui.painter()
        .add(egui::Shape::line(pts, Stroke::new(1.2, t.ink2)));
}

fn right_column(app: &mut App, ui: &mut egui::Ui, focus: Option<FragmentId>) {
    let t = app.theme.clone();

    section_heading(ui, &t, "Bankroll & Session");
    let r1 = ui.cursor().min;
    egui::Grid::new("bankroll")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Starting budget");
            let mut dollars = app.cfg.budget_cents as f64 / 100.0;
            if ui
                .add(
                    egui::DragValue::new(&mut dollars)
                        .speed(10.0)
                        .range(1.0..=1_000_000.0)
                        .prefix("$"),
                )
                .changed()
            {
                app.cfg.budget_cents = (dollars * 100.0).round() as i64;
            }
            ui.end_row();

            ui.label("Quit while ahead");
            ui.horizontal(|ui| {
                let mut on = app.cfg.quit_mult.is_some();
                ui.checkbox(&mut on, "at");
                match (on, app.cfg.quit_mult) {
                    (true, None) => app.cfg.quit_mult = Some(2.0),
                    (false, Some(_)) => app.cfg.quit_mult = None,
                    _ => {}
                }
                if let Some(m) = &mut app.cfg.quit_mult {
                    ui.add(
                        egui::DragValue::new(m)
                            .speed(0.1)
                            .range(1.1..=100.0)
                            .suffix("× budget"),
                    );
                }
            });
            ui.end_row();

            ui.label("Target session length");
            ui.add(
                egui::DragValue::new(&mut app.cfg.target_hours)
                    .speed(0.25)
                    .range(0.5..=24.0)
                    .suffix(" h"),
            );
            ui.end_row();

            ui.label("Rolls per hour");
            ui.add(
                egui::DragValue::new(&mut app.cfg.rolls_per_hour)
                    .speed(1.0)
                    .range(30..=300),
            );
            ui.end_row();
        });
    let r1_rect = section_rect(ui, r1, 0.0);
    focus_ring(app, ui, r1_rect, FragmentId::Budget, focus);
    focus_ring(app, ui, r1_rect, FragmentId::Quit, focus);
    focus_ring(app, ui, r1_rect, FragmentId::Horizon, focus);

    section_heading(ui, &t, "Table");
    let r2 = ui.cursor().min;
    // The Chip Toggle (spec §7): canonical minimums as their real chips —
    // click to include or drop one; the text field remains the +custom path.
    ui.horizontal_wrapped(|ui| {
        for &m in &[500i64, 1000, 1500, 2500, 5000, 10_000] {
            let on = app.cfg.table_mins_cents.contains(&m);
            let chip = t.chip(m);
            // Tall enough that the amount clears the selection ring — the
            // label sits in its own band under the chip, never on it.
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(50.0, 48.0), Sense::click());
            let center = egui::pos2(rect.center().x, rect.top() + 14.0);
            if m == 10_000 && t.dark() {
                // The black chip draws as its real self: black disc, ivory
                // edge spots (its data ink).
                ui.painter()
                    .circle_filled(center, 11.0, Color32::from_rgb(0x16, 0x16, 0x18));
                for k in 0..4 {
                    let a = k as f32 * std::f32::consts::FRAC_PI_2 + 0.4;
                    ui.painter().circle_filled(
                        center + egui::vec2(a.cos() * 8.4, a.sin() * 8.4),
                        2.2,
                        chip,
                    );
                }
            } else {
                ui.painter().circle_filled(center, 11.0, chip);
                for k in 0..4 {
                    let a = k as f32 * std::f32::consts::FRAC_PI_2 + 0.4;
                    ui.painter().circle_filled(
                        center + egui::vec2(a.cos() * 8.4, a.sin() * 8.4),
                        2.0,
                        t.surface,
                    );
                }
            }
            if on {
                ui.painter()
                    .circle_stroke(center, 13.5, Stroke::new(1.5, t.blue));
            }
            ui.painter().text(
                egui::pos2(rect.center().x, rect.bottom() - 2.0),
                Align2::CENTER_BOTTOM,
                numerals::money_text(m, false),
                FontId::new(9.5, theme::mono()),
                if on { t.ink } else { t.ink2 },
            );
            if resp.clicked() && !app.anything_running() {
                if on {
                    if app.cfg.table_mins_cents.len() > 1 {
                        app.cfg.table_mins_cents.retain(|&x| x != m);
                    }
                } else {
                    app.cfg.table_mins_cents.push(m);
                    app.cfg.table_mins_cents.sort_unstable();
                    app.cfg.table_mins_cents.dedup();
                }
                app.focused_min = app.focused_min.min(app.cfg.table_mins_cents.len() - 1);
                app.table_mins_text = app
                    .cfg
                    .table_mins_cents
                    .iter()
                    .map(|&c| {
                        if c % 100 == 0 {
                            format!("{}", c / 100)
                        } else {
                            format!("{:.2}", c as f64 / 100.0)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
            }
        }
    });
    ui.label("Custom minimums ($, comma-separated):");
    let resp = ui.text_edit_singleline(&mut app.table_mins_text);
    if resp.changed() {
        match parse_table_mins(&app.table_mins_text) {
            Ok(mins) => {
                app.cfg.table_mins_cents = mins;
                app.focused_min = app.focused_min.min(app.cfg.table_mins_cents.len() - 1);
                app.error = None;
            }
            Err(e) => app.error = Some(e),
        }
    }
    // Chip swatches read back the parsed minimums with their data inks.
    ui.horizontal_wrapped(|ui| {
        for &m in &app.cfg.table_mins_cents {
            let chip = t.chip(m);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 6.0, chip);
            if m == 10_000 && t.dark() {
                // The black chip draws as its real self: black disc, ivory
                // edge spots — its DATA ink is the edge-spot gray.
                ui.painter()
                    .circle_filled(rect.center(), 6.0, Color32::from_rgb(0x16, 0x16, 0x18));
                for k in 0..4 {
                    let a = k as f32 * std::f32::consts::FRAC_PI_2 + 0.4;
                    ui.painter().circle_filled(
                        rect.center() + egui::vec2(a.cos() * 4.6, a.sin() * 4.6),
                        1.3,
                        chip,
                    );
                }
            }
            ui.label(
                RichText::new(numerals::money_text(m, false))
                    .font(FontId::new(type_scale::CAPTION, theme::mono()))
                    .color(t.ink),
            );
            ui.add_space(6.0);
        }
    });
    let r2_rect = section_rect(ui, r2, 0.0);
    focus_ring(app, ui, r2_rect, FragmentId::Tables, focus);

    section_heading(ui, &t, "House Rules");
    let r3 = ui.cursor().min;
    egui::Grid::new("house")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Field pays on 12:");
            egui::ComboBox::from_id_salt("field12")
                .selected_text(if app.cfg.field_12_triple {
                    "3:1"
                } else {
                    "2:1"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.cfg.field_12_triple, false, "2:1");
                    ui.selectable_value(&mut app.cfg.field_12_triple, true, "3:1");
                });
            ui.end_row();

            ui.label("Come odds on come-out");
            ui.checkbox(&mut app.cfg.come_odds_work_on_comeout, "working");
            ui.end_row();

            ui.label("Prop/hardway stake");
            let mut prop = app.cfg.prop_bet_cents as f64 / 100.0;
            if ui
                .add(
                    egui::DragValue::new(&mut prop)
                        .speed(1.0)
                        .range(1.0..=1000.0)
                        .prefix("$"),
                )
                .changed()
            {
                app.cfg.prop_bet_cents = (prop * 100.0).round() as i64;
            }
            ui.end_row();

            ui.label("Table maximum");
            ui.add(
                egui::DragValue::new(&mut app.cfg.table_max_mult)
                    .speed(10.0)
                    .range(10..=10_000)
                    .suffix("× min"),
            )
            .on_hover_text("Caps progressions the way a real table does.");
            ui.end_row();
        });
    let r3_rect = section_rect(ui, r3, 0.0);
    for f in [
        FragmentId::Field12,
        FragmentId::ComeOddsComeout,
        FragmentId::Props,
        FragmentId::TableMax,
    ] {
        focus_ring(app, ui, r3_rect, f, focus);
    }

    ui.add_space(10.0);
    let r4 = ui.cursor().min;
    egui::CollapsingHeader::new("Engine")
        .default_open(focus == Some(FragmentId::Engine))
        .show(ui, |ui| {
            egui::Grid::new("engine")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Sessions per table min");
                    ui.add(
                        egui::DragValue::new(&mut app.cfg.sessions)
                            .speed(1000.0)
                            .range(1_000..=10_000_000),
                    );
                    ui.end_row();
                    ui.label("Roll cap per session");
                    ui.add(
                        egui::DragValue::new(&mut app.cfg.max_rolls)
                            .speed(1000.0)
                            .range(1_000..=5_000_000),
                    );
                    ui.end_row();
                    ui.label("Survival confidence");
                    egui::ComboBox::from_id_salt("conf")
                        .selected_text(format!("{:.0}%", app.cfg.confidence))
                        .show_ui(ui, |ui| {
                            for c in [90.0, 95.0, 99.0] {
                                ui.selectable_value(&mut app.cfg.confidence, c, format!("{c:.0}%"));
                            }
                        });
                    ui.end_row();
                    ui.label("Explorer sessions/combo");
                    ui.add(
                        egui::DragValue::new(&mut app.cfg.explore_sessions)
                            .speed(500.0)
                            .range(500..=100_000),
                    );
                    ui.end_row();
                    ui.label("Reduce motion");
                    let mut rm = app.prefs.reduced_motion;
                    if ui
                        .checkbox(&mut rm, "")
                        .on_hover_text(
                            "Collapses every animation to an instant crossfade; streaming \
                         charts update at the raw poll cadence. The OS setting is \
                         honored automatically where the platform exposes it.",
                        )
                        .changed()
                    {
                        app.prefs.reduced_motion = rm;
                        app.prefs.save();
                        app.motion.reduced = rm || crate::ui::motion::os_reduced_motion();
                    }
                    ui.end_row();
                    ui.label("Resident run data");
                    match app.main_run.as_ref() {
                        Some(r) => {
                            let bytes = r.store.lock().unwrap().resident_bytes();
                            ui.label(
                                RichText::new(format!("{:.0} MB", bytes as f64 / 1e6))
                                    .font(FontId::new(type_scale::BODY, theme::mono())),
                            );
                        }
                        None => {
                            ui.label(RichText::new("—").color(t.ink2));
                        }
                    }
                    ui.end_row();
                    ui.label("Explorer flat betting only");
                    ui.checkbox(&mut app.cfg.explore_flat_only, "")
                        .on_hover_text(
                            "Pin every combo to flat betting so bet selections are compared on \
                         their own merits — pressing progressions reshape the outcome \
                         distribution, which dominates goal-probability rankings.",
                        );
                    ui.end_row();
                });
        });
    let r4_rect = section_rect(ui, r4, 0.0);
    focus_ring(app, ui, r4_rect, FragmentId::Engine, focus);
}

/// Live order-ticket validation: worst-case per-roll exposure at each
/// minimum, cross-field collisions in plain words, and why Run is disabled.
fn order_ticket(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    egui::Frame::NONE
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.hairline))
        .corner_radius(6)
        .inner_margin(10.0)
        .show(ui, |ui| {
            match app.cfg.validate() {
                Err(e) => {
                    ui.label(
                        RichText::new(format!("Run is disabled: {e}"))
                            .font(FontId::new(type_scale::BODY, theme::sans()))
                            .color(t.ruin),
                    );
                }
                Ok(()) => {
                    // Worst-case pre-roll exposure at flat stakes, per min.
                    let rules = app.cfg.rules();
                    let mut parts: Vec<String> = Vec::new();
                    for &min in &app.cfg.table_mins_cents {
                        let e = worst_case_exposure(&app.cfg, min);
                        parts.push(format!(
                            "{}: {}",
                            numerals::money_text(min, false),
                            numerals::money_text(e, false)
                        ));
                    }
                    ui.label(
                        RichText::new(format!(
                            "Worst-case money on the layout per roll (flat stakes) — {}",
                            parts.join(" · ")
                        ))
                        .font(FontId::new(type_scale::CAPTION, theme::mono()))
                        .color(t.ink2),
                    );
                    // Progression vs table max collision, in words.
                    if app.cfg.sel.progression == Progression::Martingale
                        || app.cfg.sel.progression == Progression::GrandMartingale
                    {
                        if let Some(&min) = app.cfg.table_mins_cents.first() {
                            let steps =
                                (app.cfg.table_max_mult as f64).log2().floor() as u32;
                            ui.label(
                                RichText::new(format!(
                                    "table max {}× min clips a losing streak after {} doubles — the progression will flatten there",
                                    app.cfg.table_max_mult, steps
                                ))
                                .font(FontId::new(type_scale::CAPTION, theme::sans()))
                                .color(t.amber),
                            );
                            let _ = min;
                        }
                    }
                    let _ = rules;
                }
            }
        });
}

/// Sum of every selected bet's flat stake — the most the strategy can ask
/// for before a single roll.
fn worst_case_exposure(cfg: &crate::config::SimConfig, min: i64) -> i64 {
    use craps_engine::cheapest_selected_stake;
    let rules = cfg.rules();
    let sel = &cfg.sel;
    let mut total = 0i64;
    if sel.pass_line {
        total += min * (1 + odds_mult_max(cfg));
    }
    if sel.dont_pass {
        total += min * (1 + 2 * odds_mult_max(cfg)); // lays put up more
    }
    total += min * sel.come_max as i64 * (1 + odds_mult_max(cfg));
    total += min * sel.dont_come_max as i64 * (1 + 2 * odds_mult_max(cfg));
    if sel.field {
        total += min;
    }
    for (i, &n) in PLACE_NUMS.iter().enumerate() {
        if sel.place[i] {
            total += craps_engine::place_stake_for(min, n);
        }
    }
    total += rules.prop_bet_cents
        * (sel.hardways.iter().filter(|&&b| b).count() as i64
            + sel.any_seven as i64
            + sel.any_craps as i64);
    // Guard against a zero-bet call (validate() already refuses it).
    let _ = cheapest_selected_stake(sel, &rules, min);
    total
}

fn odds_mult_max(cfg: &crate::config::SimConfig) -> i64 {
    if !cfg.sel.take_odds {
        return 0;
    }
    match cfg.odds_policy {
        OddsPolicy::None => 0,
        OddsPolicy::X1 => 1,
        OddsPolicy::X2 => 2,
        OddsPolicy::X345 => 5,
        OddsPolicy::X5 => 5,
        OddsPolicy::X10 => 10,
    }
}

/// Paste a Scenario Sentence to reconstruct the whole configuration.
fn paste_sentence(app: &mut App, ui: &mut egui::Ui) {
    let t = app.theme.clone();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Paste a scenario sentence:")
                .font(FontId::new(type_scale::CAPTION, theme::sans()))
                .color(t.ink2),
        );
        let id = egui::Id::new("sentence_paste");
        let mut buf: String = ui
            .ctx()
            .data_mut(|d| d.get_temp::<String>(id))
            .unwrap_or_default();
        let resp = ui.add(
            egui::TextEdit::singleline(&mut buf)
                .desired_width(420.0)
                .hint_text("$500 at $10 tables · pass · flat · …"),
        );
        if resp.changed() {
            ui.ctx().data_mut(|d| d.insert_temp(id, buf.clone()));
        }
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !buf.is_empty() {
            match sentence::parse(&buf) {
                Ok(cfg) => {
                    app.table_mins_text = cfg
                        .table_mins_cents
                        .iter()
                        .map(|&m| {
                            if m % 100 == 0 {
                                format!("{}", m / 100)
                            } else {
                                format!("{:.2}", m as f64 / 100.0)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    app.cfg = cfg;
                    app.error = None;
                    ui.ctx().data_mut(|d| d.insert_temp(id, String::new()));
                }
                Err(e) => app.error = Some(format!("Couldn't parse that sentence: {e}")),
            }
        }
    });
}
