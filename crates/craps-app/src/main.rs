// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Craps-Sim. Boot only; the instrument lives in [`app`].

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod chart;
mod components;
mod config;
#[cfg(test)]
mod interaction_tests;
mod prefs;
mod screens;
mod sentence;
mod store;
mod stream;
mod tour;
mod ui;
#[cfg(test)]
mod visual_tests;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Craps Sim")
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([1100.0, 740.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Craps-Sim",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
