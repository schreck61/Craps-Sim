// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The chart kernel: an owned, ~1.4k-line painting layer on epaint — no
//! plotting crate. Frames, scales, axes, probes, brushes, and mark builders
//! shared by every chart in the product.

pub mod axis;
pub mod brush;
pub mod dicestrip;
pub mod dotfield;
pub mod export;
pub mod fan;
pub mod frame;
#[cfg(test)]
mod golden_tests;
pub mod hexbin;
pub mod histogram;
pub mod ladder;
pub mod marks;
pub mod probe;
pub mod scale;
pub mod survival;
pub mod trajectory;

pub use frame::{Badge, ChartCx, Layer};

use std::collections::HashSet;

/// Per-screen bidirectional hover linkage (spec §7): insight cards publish
/// highlight groups, charts read them, and vice versa. Double-buffered so
/// reads see last frame's writes without ordering constraints.
#[derive(Default)]
pub struct Highlights {
    cur: HashSet<egui::Id>,
    prev: HashSet<egui::Id>,
}

impl Highlights {
    /// Call once at the top of each frame.
    pub fn begin_frame(&mut self) {
        self.prev = std::mem::take(&mut self.cur);
    }

    pub fn set(&mut self, group: egui::Id) {
        self.cur.insert(group);
    }

    pub fn is_lit(&self, group: egui::Id) -> bool {
        self.prev.contains(&group)
    }
}
