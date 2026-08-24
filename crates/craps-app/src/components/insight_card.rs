// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Insight cards: a plain-language frequency sentence, the exact mono
//! figures with CI beneath it, an optional `Enter one ▸` affordance into
//! Replay, and bidirectional hover linkage with the adjacent chart.

use egui::{FontId, RichText, Sense, Stroke};

use crate::chart::Highlights;
use crate::ui::theme::{self, type_scale, Theme};

pub struct InsightCard<'a> {
    /// Highlight group shared with chart marks.
    pub group: egui::Id,
    /// The frequency-register lead sentence.
    pub lead: String,
    /// Exact register: mono figures with CI and n.
    pub exact: String,
    /// `Show me one ▸` target: a TRUE session index to replay.
    pub show_one: Option<u64>,
    /// The Scenario Sentence stamp.
    pub scenario: &'a str,
}

pub enum CardAction {
    None,
    ShowOne(u64),
}

impl InsightCard<'_> {
    pub fn show(self, ui: &mut egui::Ui, t: &Theme, highlights: &mut Highlights) -> CardAction {
        let lit = highlights.is_lit(self.group);
        let mut action = CardAction::None;
        let resp = egui::Frame::NONE
            .fill(t.surface2)
            .stroke(Stroke::new(1.0, if lit { t.blue } else { t.hairline }))
            .corner_radius(6)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(&self.lead)
                        .font(FontId::new(type_scale::STORY, theme::sans_medium()))
                        .color(t.ink),
                );
                ui.add_space(2.0);
                ui.label(
                    RichText::new(&self.exact)
                        .font(FontId::new(type_scale::CAPTION, theme::mono()))
                        .color(t.ink2),
                );
                if let Some(id) = self.show_one {
                    let link = ui.add(
                        egui::Label::new(
                            RichText::new("Show me one ▶")
                                .font(FontId::new(type_scale::CAPTION, theme::sans_medium()))
                                .color(t.blue),
                        )
                        .sense(Sense::click()),
                    );
                    if link.clicked() {
                        action = CardAction::ShowOne(id);
                    }
                }
                ui.label(
                    RichText::new(self.scenario)
                        .font(FontId::new(9.5, theme::sans()))
                        .color(t.ink2),
                );
            })
            .response;
        if resp.hovered() {
            highlights.set(self.group);
        }
        action
    }
}
