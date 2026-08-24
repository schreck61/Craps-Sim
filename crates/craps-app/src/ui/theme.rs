// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The Long Run's visual tokens: the Lamplight (dark) and Reading Room
//! (light) palettes, the embedded type system, and the egui style bridge.
//!
//! Color is semantics, never mood (spec Part I §4.1): amber is reserved for
//! epistemic statements, red for ruin, teal for gain, blue for selection.
//! Charts read the [`Theme`] token struct only; stock egui widgets are
//! themed once through [`apply`].

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke};

/// Font family handles. The six embedded faces are renamed subsets (see
/// assets/fonts/FONTS-NOTICE.md); Sans is registered as a fallback inside
/// the mono and display families because thin space (U+2009) and ⌘ exist
/// only in the Sans sources.
pub fn display() -> FontFamily {
    FontFamily::Name("display".into())
}
pub fn sans() -> FontFamily {
    FontFamily::Proportional
}
pub fn sans_medium() -> FontFamily {
    FontFamily::Name("sans-medium".into())
}
pub fn sans_semibold() -> FontFamily {
    FontFamily::Name("sans-semibold".into())
}
pub fn mono() -> FontFamily {
    FontFamily::Monospace
}
pub fn mono_medium() -> FontFamily {
    FontFamily::Name("mono-medium".into())
}

/// Named type sizes from the spec (§4.2).
pub mod type_scale {
    /// Screen titles — Fraunces 26 px.
    pub const TITLE: f32 = 26.0;
    /// Story-register lead sentences — Inter Medium 19 px.
    pub const STORY: f32 = 19.0;
    /// Section heads — Inter SemiBold 15 px.
    pub const SECTION: f32 = 15.0;
    /// Body and controls — Inter 13 px.
    pub const BODY: f32 = 13.0;
    /// Captions and provenance — 11 px.
    pub const CAPTION: f32 = 11.0;
    /// Per-chart key stats — mono 22 px.
    pub const STAT: f32 = 22.0;
    /// Hero numbers — mono 34 px.
    pub const HERO: f32 = 34.0;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeKind {
    Lamplight,
    ReadingRoom,
}

/// Every color the product may use. Charts and components take colors from
/// here and nowhere else.
#[derive(Clone, Debug)]
pub struct Theme {
    pub kind: ThemeKind,
    /// App background.
    pub ground: Color32,
    /// Panels and cards — one lamp closer.
    pub surface: Color32,
    /// Drawers and popovers — closest to the lamp.
    pub surface2: Color32,
    /// Hairlines and chart grids.
    pub hairline: Color32,
    pub hairline_strong: Color32,
    /// Primary ink (text and data).
    pub ink: Color32,
    /// Secondary ink (axes, captions).
    pub ink2: Color32,
    /// Ruin / loss / bust. The loudest object on any screen it appears on.
    pub ruin: Color32,
    /// Ahead / gain — deliberately quieter than ruin.
    pub gain: Color32,
    /// Expectation & trust: closed-form markers, edge figures, STALE/PARTIAL.
    pub amber: Color32,
    /// Selection / probe / focus.
    pub blue: Color32,
    /// Duel identity B (paired comparisons only).
    pub violet: Color32,
}

impl Theme {
    pub fn lamplight() -> Self {
        Self {
            kind: ThemeKind::Lamplight,
            ground: Color32::from_rgb(0x12, 0x14, 0x17),
            surface: Color32::from_rgb(0x1B, 0x1A, 0x17),
            surface2: Color32::from_rgb(0x24, 0x20, 0x19),
            hairline: Color32::from_rgba_unmultiplied(0xED, 0xE9, 0xDF, 26), // 10%
            hairline_strong: Color32::from_rgba_unmultiplied(0xED, 0xE9, 0xDF, 56), // 22%
            ink: Color32::from_rgb(0xED, 0xE9, 0xDF),
            ink2: Color32::from_rgb(0x9C, 0xA3, 0xA8),
            ruin: Color32::from_rgb(0xE5, 0x48, 0x4D),
            gain: Color32::from_rgb(0x3E, 0xB8, 0xA5),
            amber: Color32::from_rgb(0xF0, 0xB4, 0x29),
            blue: Color32::from_rgb(0x58, 0xA6, 0xFF),
            violet: Color32::from_rgb(0xB4, 0x8E, 0xE0),
        }
    }

    pub fn reading_room() -> Self {
        Self {
            kind: ThemeKind::ReadingRoom,
            ground: Color32::from_rgb(0xF6, 0xF3, 0xEC),
            surface: Color32::WHITE,
            surface2: Color32::WHITE,
            hairline: Color32::from_rgba_unmultiplied(0x22, 0x25, 0x1F, 31), // 12%
            hairline_strong: Color32::from_rgba_unmultiplied(0x22, 0x25, 0x1F, 71), // 28%
            ink: Color32::from_rgb(0x22, 0x25, 0x1F),
            ink2: Color32::from_rgb(0x6B, 0x71, 0x68),
            ruin: Color32::from_rgb(0xC2, 0x2D, 0x33),
            // Darker than the spec's sample teal so gain text holds 4.5:1
            // on the light ground (spec §11's contrast promise wins).
            gain: Color32::from_rgb(0x0F, 0x6E, 0x7D),
            amber: Color32::from_rgb(0x9A, 0x74, 0x15),
            blue: Color32::from_rgb(0x24, 0x67, 0xD6),
            violet: Color32::from_rgb(0x7C, 0x4F, 0xB8),
        }
    }

    pub fn dark(&self) -> bool {
        self.kind == ThemeKind::Lamplight
    }

    /// Near-opaque ground for label pills painted over data marks.
    pub fn pill(&self) -> Color32 {
        Color32::from_rgba_unmultiplied(self.ground.r(), self.ground.g(), self.ground.b(), 225)
    }

    /// A confidence-band fill from its parent hue: 18% alpha dark, 12% light.
    pub fn ci_band(&self, parent: Color32) -> Color32 {
        let a = if self.dark() { 46 } else { 31 };
        Color32::from_rgba_unmultiplied(parent.r(), parent.g(), parent.b(), a)
    }

    /// De-emphasized sibling series (35% ink).
    pub fn dimmed(&self, c: Color32) -> Color32 {
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 90)
    }

    /// The table-minimum series ink — real chip colors, the sole categorical
    /// encoding for minimums, identical in every chart, legend, and toggle
    /// (spec §4.1). $15 and $50 have no canonical real chip; violet and
    /// bronze are documented as ours. Minimums without a chip of their own
    /// draw from an extended, lightness-separated reserve.
    pub fn chip(&self, min_cents: i64) -> Color32 {
        let dark = self.dark();
        let rgb = |d: (u8, u8, u8), l: (u8, u8, u8)| {
            let c = if dark { d } else { l };
            Color32::from_rgb(c.0, c.1, c.2)
        };
        match min_cents {
            500 => rgb((0xC0, 0x57, 0x4A), (0xA6, 0x4B, 0x3E)), // $5 brick
            1000 => rgb((0x52, 0x86, 0xC4), (0x41, 0x76, 0xB4)), // $10 blue
            1500 => rgb((0xA9, 0x9C, 0xE3), (0x5A, 0x4B, 0x9F)), // $15 violet (ours; luminance-split for protanopia)
            2500 => rgb((0x4C, 0x9A, 0x75), (0x3F, 0x8F, 0x68)), // $25 green
            5000 => rgb((0xC9, 0xA4, 0x63), (0xA8, 0x85, 0x2E)), // $50 bronze (ours; luminance-split for protanopia)
            10_000 => rgb((0xAE, 0xB4, 0xBE), (0x2F, 0x33, 0x39)), // $100 edge-spot gray (deepened for protanopia gap)
            _ => {
                // Reserve ramp for custom minimums, keyed deterministically.
                type Rgb = (u8, u8, u8);
                const RESERVE: [(Rgb, Rgb); 4] = [
                    ((0xC8, 0x8A, 0xB0), (0xA8, 0x62, 0x8C)), // rose
                    ((0x7F, 0xA6, 0x66), (0x5E, 0x84, 0x47)), // moss
                    ((0x62, 0xB3, 0xC8), (0x3E, 0x8B, 0xA0)), // teal-gray
                    ((0xD0, 0xA0, 0x7A), (0xA8, 0x7A, 0x54)), // clay
                ];
                let k = (min_cents / 100) as usize % RESERVE.len();
                rgb(RESERVE[k].0, RESERVE[k].1)
            }
        }
    }
}

/// Register the six embedded faces. Fallback chains: mono falls back to Sans
/// for U+2009/⌘, and every family ends in egui's defaults so no glyph ever
/// renders as tofu.
pub fn install_fonts(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        defs.font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add(
        "LongrunDisplay-Medium",
        include_bytes!("../../assets/fonts/LongrunDisplay-Medium.ttf"),
    );
    add(
        "LongrunSans-Regular",
        include_bytes!("../../assets/fonts/LongrunSans-Regular.ttf"),
    );
    add(
        "LongrunSans-Medium",
        include_bytes!("../../assets/fonts/LongrunSans-Medium.ttf"),
    );
    add(
        "LongrunSans-SemiBold",
        include_bytes!("../../assets/fonts/LongrunSans-SemiBold.ttf"),
    );
    add(
        "LongrunMono-Regular",
        include_bytes!("../../assets/fonts/LongrunMono-Regular.ttf"),
    );
    add(
        "LongrunMono-Medium",
        include_bytes!("../../assets/fonts/LongrunMono-Medium.ttf"),
    );

    type Families = BTreeMap<FontFamily, Vec<String>>;
    let defaults: Families = defs.families.clone();
    let default_prop = defaults
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let default_mono = defaults
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();

    let chain = |head: &[&str], tail: &[String]| -> Vec<String> {
        head.iter()
            .map(|s| s.to_string())
            .chain(tail.iter().cloned())
            .collect()
    };

    defs.families.insert(
        FontFamily::Proportional,
        chain(&["LongrunSans-Regular"], &default_prop),
    );
    defs.families.insert(
        FontFamily::Monospace,
        chain(
            &["LongrunMono-Regular", "LongrunSans-Regular"],
            &default_mono,
        ),
    );
    defs.families.insert(
        FontFamily::Name("display".into()),
        chain(
            &["LongrunDisplay-Medium", "LongrunSans-Medium"],
            &default_prop,
        ),
    );
    defs.families.insert(
        FontFamily::Name("sans-medium".into()),
        chain(&["LongrunSans-Medium"], &default_prop),
    );
    defs.families.insert(
        FontFamily::Name("sans-semibold".into()),
        chain(&["LongrunSans-SemiBold"], &default_prop),
    );
    defs.families.insert(
        FontFamily::Name("mono-medium".into()),
        chain(&["LongrunMono-Medium", "LongrunSans-Medium"], &default_mono),
    );
    ctx.set_fonts(defs);
}

/// Bridge the theme into egui's style for stock widgets. Charts never read
/// egui visuals — they read the [`Theme`] directly.
pub fn apply(ctx: &egui::Context, t: &Theme) {
    use egui::style::{Selection, TextStyle, WidgetVisuals, Widgets};

    let mut visuals = if t.dark() {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = Some(t.ink);
    visuals.window_fill = t.surface2;
    visuals.panel_fill = t.ground;
    visuals.extreme_bg_color = if t.dark() {
        Color32::from_rgb(0x0D, 0x0F, 0x11)
    } else {
        Color32::from_rgb(0xEF, 0xEB, 0xE1)
    };
    visuals.faint_bg_color = t.surface;
    visuals.selection = Selection {
        bg_fill: t.ci_band(t.blue),
        stroke: Stroke::new(1.0, t.blue),
    };
    visuals.hyperlink_color = t.blue;
    visuals.warn_fg_color = t.amber;
    visuals.error_fg_color = t.ruin;

    let base = |bg: Color32| WidgetVisuals {
        bg_fill: bg,
        weak_bg_fill: bg,
        bg_stroke: Stroke::new(1.0, t.hairline),
        corner_radius: CornerRadius::same(4),
        fg_stroke: Stroke::new(1.0, t.ink),
        expansion: 0.0,
    };
    let mut w = Widgets {
        noninteractive: base(t.surface),
        inactive: base(t.surface2),
        hovered: base(t.surface2),
        open: base(t.surface2),
        active: base(t.surface2),
    };
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.ink2);
    w.hovered.bg_stroke = Stroke::new(1.0, t.hairline_strong);
    w.active.bg_stroke = Stroke::new(1.0, t.blue);
    w.open.bg_stroke = Stroke::new(1.0, t.hairline_strong);
    visuals.widgets = w;

    // BOTH egui theme slots get the app's visuals. egui resolves its
    // active slot from the OS theme, and the first frame's raw input can
    // flip that slot AFTER startup applied ours — leaving the app's colors
    // in the inactive slot and egui's dull defaults on screen (the
    // "dim text until you toggle themes" bug on light-mode systems).
    ctx.set_visuals_of(egui::Theme::Dark, visuals.clone());
    ctx.set_visuals_of(egui::Theme::Light, visuals);
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(type_scale::SECTION, sans_semibold()),
            ),
            (TextStyle::Body, FontId::new(type_scale::BODY, sans())),
            (
                TextStyle::Button,
                FontId::new(type_scale::BODY, sans_medium()),
            ),
            (TextStyle::Small, FontId::new(type_scale::CAPTION, sans())),
            (TextStyle::Monospace, FontId::new(type_scale::BODY, mono())),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        // Spec §4.6: objects arrive and settle; stock widget animations stay
        // at the micro tier.
        s.animation_time = crate::ui::motion::MICRO;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance.
    fn lum(c: Color32) -> f64 {
        let ch = |v: u8| {
            let v = v as f64 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (l1, l2) = (lum(a).max(lum(b)), lum(a).min(lum(b)));
        (l1 + 0.05) / (l2 + 0.05)
    }

    /// Spec §11: text ≥ 4.5:1 on its grounds; data inks ≥ 3:1 on chart
    /// grounds — including the $100 series (the matte-black-on-felt failure
    /// is designed out).
    #[test]
    fn contrast_meets_wcag_in_both_themes() {
        for t in [Theme::lamplight(), Theme::reading_room()] {
            for ground in [t.ground, t.surface, t.surface2] {
                assert!(
                    contrast(t.ink, ground) >= 4.5,
                    "{:?}: primary ink {:.2}",
                    t.kind,
                    contrast(t.ink, ground)
                );
                assert!(
                    contrast(t.ink2, ground) >= 4.5,
                    "{:?}: secondary ink {:.2}",
                    t.kind,
                    contrast(t.ink2, ground)
                );
            }
            for (name, c) in [
                ("ruin", t.ruin),
                ("gain", t.gain),
                ("amber", t.amber),
                ("blue", t.blue),
                ("violet", t.violet),
                ("$5", t.chip(500)),
                ("$10", t.chip(1000)),
                ("$15", t.chip(1500)),
                ("$25", t.chip(2500)),
                ("$50", t.chip(5000)),
                ("$100", t.chip(10_000)),
            ] {
                let r = contrast(c, t.ground);
                assert!(r >= 3.0, "{:?}: data ink {name} is {r:.2} (< 3.0)", t.kind);
            }
        }
    }

    /// The five spec'd chip inks must differ in lightness as well as hue so
    /// the series survives grayscale and CVD (full CVD simulation runs in
    /// the accessibility suite).
    #[test]
    fn chip_series_lightness_separation() {
        let t = Theme::lamplight();
        let mins = [500, 1000, 1500, 2500, 5000, 10_000];
        let mut lums: Vec<(i64, f64)> = mins.iter().map(|&m| (m, lum(t.chip(m)))).collect();
        lums.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        for pair in lums.windows(2) {
            let gap = pair[1].1 - pair[0].1;
            assert!(
                gap > 0.008,
                "chips ${} and ${} too close in luminance ({:.4})",
                pair[0].0 / 100,
                pair[1].0 / 100,
                gap
            );
        }
    }
}

#[cfg(test)]
mod cvd_tests {
    use super::*;

    /// Viénot/Brettel-style dichromacy simulation (linear-RGB matrix form).
    fn simulate(c: Color32, kind: &str) -> (f64, f64, f64) {
        fn lin(v: u8) -> f64 {
            let v = v as f64 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        let (r, g, b) = (lin(c.r()), lin(c.g()), lin(c.b()));
        // Viénot 1999 protanopia / deuteranopia; Brettel-derived tritanopia
        // approximation. Rows are linear-RGB → linear-RGB.
        let m: [[f64; 9]; 1] = match kind {
            "protan" => [[
                0.11238, 0.88762, 0.0, 0.11238, 0.88762, 0.0, 0.00401, -0.00401, 1.0,
            ]],
            "deutan" => [[
                0.29275, 0.70725, 0.0, 0.29275, 0.70725, 0.0, -0.02234, 0.02234, 1.0,
            ]],
            _ => [[
                1.0, 0.15236, -0.15236, 0.0, 0.86717, 0.13283, 0.0, 0.86717, 0.13283,
            ]],
        };
        let m = m[0];
        (
            m[0] * r + m[1] * g + m[2] * b,
            m[3] * r + m[4] * g + m[5] * b,
            m[6] * r + m[7] * g + m[8] * b,
        )
    }

    fn dist(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
        // Weighted linear-RGB distance with a luminance term: crude but
        // monotone enough to police "distinguishable".
        let dy = (0.2126 * (a.0 - b.0) + 0.7152 * (a.1 - b.1) + 0.0722 * (a.2 - b.2)).abs();
        let dc = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)).sqrt();
        0.6 * dy + 0.4 * dc
    }

    /// Spec §11: the chip series must stay pairwise distinguishable under
    /// simulated protanopia, deuteranopia, and tritanopia (position/labels
    /// double every encoding, but color must not actively lie).
    #[test]
    fn chip_series_survives_cvd() {
        for theme in [Theme::lamplight(), Theme::reading_room()] {
            let mins = [500i64, 1000, 1500, 2500, 5000, 10_000];
            for kind in ["protan", "deutan", "tritan"] {
                for i in 0..mins.len() {
                    for j in (i + 1)..mins.len() {
                        let a = simulate(theme.chip(mins[i]), kind);
                        let b = simulate(theme.chip(mins[j]), kind);
                        let d = dist(a, b);
                        assert!(
                            d > 0.045,
                            "{:?} {kind}: chips ${} vs ${} too close ({d:.3})",
                            theme.kind,
                            mins[i] / 100,
                            mins[j] / 100
                        );
                    }
                }
            }
        }
    }

    /// The money pairing is red/teal, not red/green: ruin and gain must
    /// stay far apart under every simulated dichromacy.
    #[test]
    fn ruin_vs_gain_survives_cvd() {
        for theme in [Theme::lamplight(), Theme::reading_room()] {
            for kind in ["protan", "deutan", "tritan"] {
                let d = dist(simulate(theme.ruin, kind), simulate(theme.gain, kind));
                assert!(
                    d > 0.10,
                    "{:?} {kind}: ruin vs gain only {d:.3} apart",
                    theme.kind
                );
            }
        }
    }
}
