// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Inline errors (spec §9): plain-language cause in the panel that failed,
//! the offending control one click away through the Scenario Sentence, an
//! expandable mono detail block, and Copy diagnostic. No modal alerts, no
//! focus traps, nothing lost from prior runs.

use egui::{FontId, RichText, Sense, Stroke};

use crate::sentence::FragmentId;
use crate::ui::theme::{self, type_scale, Theme};

pub struct ErrorPanel<'a> {
    /// The plain-language cause.
    pub message: &'a str,
    /// The control to focus when the user clicks "fix it".
    pub focus: Option<FragmentId>,
    /// Diagnostic payload: canonical sentence + seed + version.
    pub diagnostic: String,
}

pub enum ErrorAction {
    None,
    FocusControl(FragmentId),
}

impl ErrorPanel<'_> {
    /// Guess which control an engine/config error message points at.
    pub fn focus_for(message: &str) -> Option<FragmentId> {
        let m = message.to_ascii_lowercase();
        if m.contains("bet type") || m.contains("odds require") {
            Some(FragmentId::Bets)
        } else if m.contains("table minimum") {
            Some(FragmentId::Tables)
        } else if m.contains("budget") {
            Some(FragmentId::Budget)
        } else if m.contains("roll cap") || m.contains("session") {
            Some(FragmentId::Engine)
        } else {
            None
        }
    }

    pub fn show(self, ui: &mut egui::Ui, t: &Theme) -> ErrorAction {
        let mut action = ErrorAction::None;
        egui::Frame::NONE
            .fill(t.surface2)
            .stroke(Stroke::new(1.0, t.ruin))
            .corner_radius(6)
            .inner_margin(10.0)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(self.message)
                        .font(FontId::new(type_scale::BODY, theme::sans_medium()))
                        .color(t.ruin),
                );
                ui.horizontal(|ui| {
                    if let Some(frag) = self.focus {
                        let link = ui.add(
                            egui::Label::new(
                                RichText::new("fix it ▶")
                                    .font(FontId::new(type_scale::CAPTION, theme::sans_medium()))
                                    .color(t.blue),
                            )
                            .sense(Sense::click()),
                        );
                        if link.clicked() {
                            action = ErrorAction::FocusControl(frag);
                        }
                    }
                    let copy = ui.add(
                        egui::Label::new(
                            RichText::new("Copy diagnostic")
                                .font(FontId::new(type_scale::CAPTION, theme::sans_medium()))
                                .color(t.blue),
                        )
                        .sense(Sense::click()),
                    );
                    if copy.clicked() {
                        ui.ctx().copy_text(self.diagnostic.clone());
                    }
                });
                egui::CollapsingHeader::new(
                    RichText::new("detail")
                        .font(FontId::new(type_scale::CAPTION, theme::sans()))
                        .color(t.ink2),
                )
                .id_salt("error_detail")
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(&self.diagnostic)
                            .font(FontId::new(type_scale::CAPTION, theme::mono()))
                            .color(t.ink2),
                    );
                });
            });
        action
    }
}
