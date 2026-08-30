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
mod store_strategies;
mod stream;
mod tour;
mod ui;
#[cfg(test)]
mod visual_tests;

/// The bundled window/taskbar icon (macOS's dock uses the .app's .icns).
fn app_icon() -> egui::IconData {
    let bytes = include_bytes!("../assets/icon/icon-256.png");
    let decoder = png::Decoder::new(std::io::Cursor::new(&bytes[..]));
    let mut reader = decoder.read_info().expect("bundled icon header");
    let mut rgba = vec![0u8; reader.output_buffer_size().expect("bundled icon size")];
    let info = reader.next_frame(&mut rgba).expect("bundled icon frame");
    debug_assert_eq!(info.color_type, png::ColorType::Rgba);
    rgba.truncate(info.buffer_size());
    egui::IconData {
        rgba,
        width: info.width,
        height: info.height,
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Craps Sim")
            .with_icon(app_icon())
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

#[cfg(test)]
mod icon_tests {
    /// The bundled icon must decode: a bad asset would panic at startup.
    #[test]
    fn bundled_icon_decodes() {
        let icon = super::app_icon();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    }
}
