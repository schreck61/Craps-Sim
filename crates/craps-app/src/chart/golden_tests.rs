// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Shape goldens (plan §7's primary chart regression): platform-independent
//! because they check the TESSELLATION INPUT — the `epaint::Shape` list a
//! chart emits — not GPU output. Each golden paints one chart into an
//! `egui_kittest` harness with fixed synthetic data, serializes every shape
//! (variant, coordinates rounded to 0.1 px, colors as hex) into a stable
//! digest, and pins both the per-variant shape counts and a SplitMix64 fold
//! of the digest.
//!
//! Determinism guards: the app's own embedded fonts are installed before
//! any layout (galley geometry is part of the digest), `pixels_per_point`
//! is fixed at 1.0, and the harness size is fixed. Every case digests two
//! consecutive frames and asserts they match before comparing to the pins.
//!
//! The pinned constants freeze chart output. A failure means a chart's
//! emitted shapes changed; an INTENTIONAL visual change must consciously
//! re-pin (run the test and copy the reported counts/hash).

use std::collections::BTreeMap;

use craps_engine::splitmix64;
use egui::epaint::{ClippedShape, ColorMode, Mesh, Shape};
use egui::{vec2, Color32, Pos2, Stroke};
use egui_kittest::Harness;

use super::frame::{Badge, ChartFrame};
use super::histogram::{self, HistogramData, HistogramOverlays};
use super::ladder::{self, Ladder};
use super::survival::{self, SurvivalOptions, SurvivalSeries};
use crate::ui::theme::{self, Theme};

// ---------------------------------------------------------------------------
// Digest machinery
// ---------------------------------------------------------------------------

fn r1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

/// Quantize a coordinate to 0.1 px for hashing.
fn q1(v: f32) -> u64 {
    ((v * 10.0).round() as i64) as u64
}

fn hex(c: Color32) -> String {
    let [r, g, b, a] = c.to_array();
    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
}

fn pos(p: Pos2) -> String {
    format!("({:.1},{:.1})", r1(p.x), r1(p.y))
}

fn stroke(s: &Stroke) -> String {
    format!("{:.1}/{}", r1(s.width), hex(s.color))
}

/// Meshes are summarized as a vertex/index fold rather than a line per
/// vertex (the histogram alone emits hundreds of vertices).
fn mesh_sig(m: &Mesh) -> u64 {
    let mut h = splitmix64(m.vertices.len() as u64 ^ ((m.indices.len() as u64) << 32));
    for v in &m.vertices {
        h = splitmix64(h ^ q1(v.pos.x));
        h = splitmix64(h ^ q1(v.pos.y));
        h = splitmix64(h ^ u64::from(u32::from_le_bytes(v.color.to_array())));
    }
    for &i in &m.indices {
        h = splitmix64(h ^ u64::from(i));
    }
    h
}

fn variant_name(s: &Shape) -> &'static str {
    match s {
        Shape::Noop => "Noop",
        Shape::Vec(_) => "Vec",
        Shape::Circle(_) => "Circle",
        Shape::Ellipse(_) => "Ellipse",
        Shape::LineSegment { .. } => "LineSegment",
        Shape::Path(_) => "Path",
        Shape::Rect(_) => "Rect",
        Shape::Text(_) => "Text",
        Shape::Mesh(_) => "Mesh",
        Shape::QuadraticBezier(_) => "QuadraticBezier",
        Shape::CubicBezier(_) => "CubicBezier",
        Shape::Callback(_) => "Callback",
    }
}

fn digest_shape(s: &Shape, counts: &mut BTreeMap<&'static str, u32>, lines: &mut String) {
    if let Shape::Vec(v) = s {
        // A shape group contributes its children, not itself.
        for child in v {
            digest_shape(child, counts, lines);
        }
        return;
    }
    *counts.entry(variant_name(s)).or_default() += 1;
    match s {
        Shape::Vec(_) => unreachable!("handled above"),
        Shape::Noop => lines.push_str("Noop\n"),
        Shape::Circle(c) => {
            lines.push_str(&format!(
                "Circle {} r{:.1} {} {}\n",
                pos(c.center),
                r1(c.radius),
                hex(c.fill),
                stroke(&c.stroke)
            ));
        }
        Shape::Ellipse(e) => {
            lines.push_str(&format!(
                "Ellipse {} ({:.1},{:.1}) {} {}\n",
                pos(e.center),
                r1(e.radius.x),
                r1(e.radius.y),
                hex(e.fill),
                stroke(&e.stroke)
            ));
        }
        Shape::LineSegment { points, stroke: st } => {
            lines.push_str(&format!(
                "LineSegment {} {} {}\n",
                pos(points[0]),
                pos(points[1]),
                stroke(st)
            ));
        }
        Shape::Path(p) => {
            let color = match &p.stroke.color {
                ColorMode::Solid(c) => hex(*c),
                ColorMode::UV(_) => "uv".to_owned(),
            };
            lines.push_str(&format!(
                "Path n={} closed={} {} {:.1}/{}",
                p.points.len(),
                p.closed,
                hex(p.fill),
                r1(p.stroke.width),
                color
            ));
            for pt in &p.points {
                lines.push_str(&format!(" {}", pos(*pt)));
            }
            lines.push('\n');
        }
        Shape::Rect(r) => {
            lines.push_str(&format!(
                "Rect {} {} {} {}\n",
                pos(r.rect.min),
                pos(r.rect.max),
                hex(r.fill),
                stroke(&r.stroke)
            ));
        }
        Shape::Text(t) => {
            lines.push_str(&format!(
                "Text {} {:.1}x{:.1} {} {:?}\n",
                pos(t.pos),
                r1(t.galley.size().x),
                r1(t.galley.size().y),
                hex(t.fallback_color),
                t.galley.text()
            ));
        }
        Shape::Mesh(m) => {
            lines.push_str(&format!(
                "Mesh v={} i={} sig={:016x}\n",
                m.vertices.len(),
                m.indices.len(),
                mesh_sig(m)
            ));
        }
        Shape::QuadraticBezier(b) => {
            lines.push_str("QuadraticBezier");
            for pt in b.points {
                lines.push_str(&format!(" {}", pos(pt)));
            }
            lines.push('\n');
        }
        Shape::CubicBezier(b) => {
            lines.push_str("CubicBezier");
            for pt in b.points {
                lines.push_str(&format!(" {}", pos(pt)));
            }
            lines.push('\n');
        }
        Shape::Callback(_) => lines.push_str("Callback\n"),
    }
}

/// Length-prefixed SplitMix64 fold over the digest text, 8 bytes at a time.
fn fold_bytes(bytes: &[u8]) -> u64 {
    let mut h = splitmix64(bytes.len() as u64);
    for chunk in bytes.chunks(8) {
        let mut buf = [0u8; 8];
        buf[..chunk.len()].copy_from_slice(chunk);
        h = splitmix64(h ^ u64::from_le_bytes(buf));
    }
    h
}

fn digest(shapes: &[ClippedShape]) -> (Vec<(&'static str, u32)>, u64) {
    let mut counts = BTreeMap::new();
    let mut lines = String::new();
    for cs in shapes {
        digest_shape(&cs.shape, &mut counts, &mut lines);
    }
    (counts.into_iter().collect(), fold_bytes(lines.as_bytes()))
}

/// Run one paint closure in a fixed harness with the app's fonts and return
/// the shape digest, after checking it is stable frame-to-frame.
fn run_case(paint: impl Fn(&mut egui::Ui) + 'static) -> (Vec<(&'static str, u32)>, u64) {
    let mut fonts_installed = false;
    let mut harness = Harness::builder()
        .with_size(vec2(880.0, 560.0))
        .with_pixels_per_point(1.0)
        .build_ui(move |ui| {
            // The app's own faces, so galley geometry is the product's, not
            // egui's defaults. Installed fonts activate on the NEXT pass;
            // the first frame paints nothing so no chart ever lays out in
            // the default fonts.
            if !fonts_installed {
                theme::install_fonts(ui.ctx());
                fonts_installed = true;
                ui.ctx().request_repaint();
                return;
            }
            paint(ui);
        });
    harness.step();
    harness.step();
    let first = digest(&harness.output().shapes);
    harness.step();
    let second = digest(&harness.output().shapes);
    assert_eq!(first, second, "shape digest is not stable frame-to-frame");
    first
}

fn assert_golden(
    got: (Vec<(&'static str, u32)>, u64),
    want_counts: &[(&str, u32)],
    want_hash: u64,
    what: &str,
) {
    let (counts, hash) = got;
    assert_eq!(
        counts, want_counts,
        "{what}: per-variant shape counts drifted — if intentional, re-pin"
    );
    assert_eq!(
        hash, want_hash,
        "{what}: shape digest drifted (got {hash:#018X}) — if intentional, re-pin"
    );
}

// ---------------------------------------------------------------------------
// Synthetic fixtures
// ---------------------------------------------------------------------------

/// A deterministic 64-bin distribution: a broad hump plus keyed noise.
fn synth_hist_counts() -> Vec<u32> {
    (0u64..64)
        .map(|i| {
            let d = i as i64 - 30;
            let hump = (900 - d * d).max(0) as u32 / 3;
            let noise = (splitmix64(i * 2 + 1) % 90) as u32;
            hump + noise
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Goldens
// ---------------------------------------------------------------------------

#[test]
fn histogram_shapes_golden() {
    let got = run_case(|ui| {
        let t = Theme::lamplight();
        let counts = synth_hist_counts();
        let data = HistogramData {
            counts: &counts,
            bin_width: 2500,
            busts: 1234,
            bust_frac: 0.104,
        };
        let overlays = HistogramOverlays {
            house_line: Some((28_600.0, "house expects −$14".to_owned())),
            house_gap_caption: Some("gap = bust mass absorbed at $0".to_owned()),
            mean_ci: Some((33_000.0, 1_800.0)),
            median: Some(31_500.0),
            ahead_line: Some((30_000.0, "P(ahead) = 43.1%".to_owned())),
            budget_cents: 30_000,
            log_y: false,
            ecdf_alpha: 0.35,
            brush_label: Some("52.3% of nights in view".to_owned()),
            fill_alpha: 1.0,
            pinned_edge_label: Some("−1.41¢ per $1".to_owned()),
            loss_pin: Some((28_600.0, "expected loss −$14".to_owned())),
        };
        ChartFrame::new(egui::Id::new("golden_histogram"), "Where the night ends")
            .story("Synthetic distribution — golden fixture.")
            .provenance("n = 11,910 · seed 7 · golden")
            .badge(Some(Badge::Partial("11.9k".to_owned())))
            .show(ui, &t, |cx| histogram::paint(cx, &data, &overlays));
    });
    assert_golden(
        got,
        &[
            ("LineSegment", 46),
            ("Mesh", 1),
            ("Path", 21),
            ("Rect", 15),
            ("Text", 36),
        ],
        0xEEEA_EA33_61DE_B423,
        "histogram golden",
    );
}

#[test]
fn survival_shapes_golden() {
    let got = run_case(|ui| {
        let t = Theme::lamplight();
        let series: Vec<SurvivalSeries> = [
            (1.6f64, 500i64, "$5", true),
            (2.4, 1000, "$10", false),
            (3.4, 2500, "$25", false),
        ]
        .into_iter()
        .map(|(k, min_cents, label, focused)| SurvivalSeries {
            color: t.chip(min_cents),
            label: label.to_owned(),
            points: (0..=40)
                .map(|i| {
                    let x = f64::from(i) * 50.0;
                    (x, (1.0 - f64::from(i) / 40.0).powf(k))
                })
                .collect(),
            focused,
        })
        .collect();
        let opt = SurvivalOptions {
            x_max: 2000.0,
            money_x: false,
            droplines: vec![
                (400.0, "median night ends".to_owned()),
                (1200.0, "P25".to_owned()),
            ],
            censored: Some((2000.0, 0.07, "7% still going at cap".to_owned())),
            hours_per_x: Some(0.0098),
            probe_labels: Vec::new(),
        };
        ChartFrame::new(egui::Id::new("golden_survival"), "How long the money lasts")
            .story("Synthetic survival curves — golden fixture.")
            .provenance("n = 3 × 41 pts · golden")
            .show(ui, &t, |cx| survival::paint(cx, &series, &opt));
    });
    assert_golden(
        got,
        &[("LineSegment", 99), ("Path", 22), ("Rect", 9), ("Text", 37)],
        0x09FF_B55A_A0FA_1D04,
        "survival golden",
    );
}

#[test]
fn ladder_shapes_golden() {
    let got = run_case(|ui| {
        let t = Theme::lamplight();
        let deciles: [i64; 9] = [
            4_000, 11_000, 17_000, 22_500, 26_000, 30_000, 35_500, 43_000, 61_000,
        ];
        let l = Ladder {
            deciles: &deciles,
            mean: 28_400.0,
            budget_cents: 30_000,
        };
        ChartFrame::new(egui::Id::new("golden_ladder"), "The decile ladder")
            .story("Fixed deciles — golden fixture.")
            .provenance("golden")
            .height(120.0)
            .show(ui, &t, |cx| ladder::paint(cx, &l));
    });
    assert_golden(
        got,
        &[("Path", 14), ("Rect", 8), ("Text", 8)],
        0x6643_D807_B783_DD25,
        "ladder golden",
    );
}

// ---------------------------------------------------------------------------
// Explorer tie bracket
// ---------------------------------------------------------------------------

#[test]
fn explorer_tie_bracket_golden() {
    use crate::screens::explorer::RankBy;
    use craps_engine::{ExploreRow, Progression};

    fn row(p_double: f64, ci: f64, mean_final: f64) -> ExploreRow {
        ExploreRow {
            strategy_idx: 0,
            strategy: "Pass line",
            progression: Progression::Flat,
            quit_idx: 0,
            quit: None,
            p_double,
            p_double_ci: ci,
            p_ahead: 0.4,
            p_ahead_ci: 0.005,
            p_nobust: 0.8,
            p_nobust_ci: 0.004,
            median_final: 25_000,
            median_final_ci: (24_000, 26_000),
            mean_final,
            mean_final_ci: 400.0,
            median_rolls: 180,
            sessions: 20_000,
        }
    }

    // Rows 0–2 are statistically tied on p_double: every interval overlaps
    // the leader's [0.136, 0.144]. Rows 3–4 are cleanly below it.
    let rows = [
        row(0.140, 0.004, 31_000.0),
        row(0.138, 0.004, 30_500.0),
        row(0.135, 0.004, 30_000.0),
        row(0.120, 0.004, 29_000.0),
        row(0.100, 0.004, 28_000.0),
    ];

    // The leaderboard's tie rule, reproduced through its public seams
    // (RankBy::key / RankBy::interval). The sort-then-prefix-count step is
    // duplicated from screens::explorer's private leaderboard fn — keep in
    // sync with it.
    let rank = RankBy::DoubleUp;
    let mut sorted: Vec<&ExploreRow> = rows.iter().collect();
    sorted.sort_by(|a, b| {
        rank.key(b)
            .partial_cmp(&rank.key(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.mean_final
                    .partial_cmp(&a.mean_final)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    let leader_iv = rank.interval(sorted[0]);
    let tie_count = sorted
        .iter()
        .map(|r| {
            let iv = rank.interval(r);
            iv.1 >= leader_iv.0 && leader_iv.1 >= iv.0
        })
        .take_while(|&tied| tied)
        .count();
    assert_eq!(tie_count, 3, "exactly rows 0-2 are tied with the leader");
}
