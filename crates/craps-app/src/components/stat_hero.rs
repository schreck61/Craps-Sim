// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The hero stat: a 34 px mono figure with a display-face unit label and
//! its CI at half size. Principle 1 is enforced by the API: a hero never
//! renders without naming the on-screen chart its shape lives in — the
//! debug build panics if the chart was not mounted this frame.

use egui::{FontId, RichText};

use crate::ui::numerals;
use crate::ui::theme::{self, type_scale, Theme};

/// Charts mounted this frame register here; heroes assert membership.
#[derive(Default)]
pub struct MountedCharts {
    ids: std::collections::HashSet<egui::Id>,
    prev: std::collections::HashSet<egui::Id>,
    /// Consecutive frames a required chart was absent — one frame of grace
    /// covers the hero-above-chart layout order on a screen's first frame.
    misses: std::cell::RefCell<std::collections::HashMap<egui::Id, u32>>,
}

impl MountedCharts {
    /// Install the ids the frames registered LAST frame (drained from egui
    /// temp data by the app shell) — heroes assert against real mounts, not
    /// self-attestation.
    pub fn set_frame(&mut self, last_frame: impl IntoIterator<Item = egui::Id>) {
        self.prev = std::mem::take(&mut self.ids);
        self.ids = last_frame.into_iter().collect();
    }

    fn was_mounted(&self, id: egui::Id) -> bool {
        let hit = self.prev.contains(&id) || self.ids.contains(&id);
        let mut misses = self.misses.borrow_mut();
        if hit {
            misses.remove(&id);
            true
        } else {
            let m = misses.entry(id).or_insert(0);
            *m += 1;
            *m < 2
        }
    }
}

pub struct StatHero {
    money_cents: i64,
    unit: String,
    ci: Option<String>,
    requires: Option<egui::Id>,
}

impl StatHero {
    pub fn money(cents: i64, unit: impl Into<String>) -> Self {
        Self {
            money_cents: cents,
            unit: unit.into(),
            ci: None,
            requires: None,
        }
    }

    pub fn ci(mut self, ci: impl Into<String>) -> Self {
        self.ci = Some(ci.into());
        self
    }

    /// No summary without its shape: name the chart this number was cut from.
    pub fn requires(mut self, chart: egui::Id) -> Self {
        self.requires = Some(chart);
        self
    }

    pub fn show(self, ui: &mut egui::Ui, t: &Theme, mounted: &MountedCharts) {
        if let Some(chart) = self.requires {
            debug_assert!(
                mounted.was_mounted(chart),
                "StatHero requires chart {chart:?} on screen (Principle 1)"
            );
        } else {
            debug_assert!(
                false,
                "StatHero without .requires(chart) — Principle 1 violation"
            );
        }
        ui.horizontal(|ui| {
            ui.label(numerals::money_job(
                self.money_cents,
                type_scale::HERO,
                t.ink,
            ));
            ui.vertical(|ui| {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(&self.unit)
                        .font(FontId::new(15.0, theme::display()))
                        .color(t.ink2),
                );
                if let Some(ci) = &self.ci {
                    ui.label(
                        RichText::new(ci)
                            .font(FontId::new(type_scale::CAPTION, theme::mono()))
                            .color(t.ink2),
                    );
                }
            });
        });
    }
}
