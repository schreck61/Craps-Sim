// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Embeds the application icon into the Windows executable's resources
//! (Explorer, taskbar, and Alt-Tab read it from there). Other platforms
//! carry the icon differently: the macOS bundle ships AppIcon.icns and the
//! running window sets its icon via eframe.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/icon.ico");
        res.compile().expect("embed Windows icon resource");
    }
}
