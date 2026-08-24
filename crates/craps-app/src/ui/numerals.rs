// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Every numeral in the product is formatted here, in mono, exact.
//!
//! Money is integer cents end to end: thin-space thousands separators, a
//! true minus sign (−, never a hyphen and never parentheses), and cents at
//! 70% alpha — the engine's exactness displayed, not rounded away. No
//! numeral may ever appear in a proportional face; a CI grep enforces that
//! no other module formats money.

use egui::text::LayoutJob;
use egui::{Color32, FontId, TextFormat};

use super::theme::{mono, mono_medium, type_scale};

pub const THIN_SPACE: char = '\u{2009}';
pub const MINUS: char = '\u{2212}';

/// Group an unsigned digit string with thin spaces.
fn group(digits: &str) -> String {
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(THIN_SPACE);
        }
        out.push(*b as char);
    }
    out
}

/// Exact dollars-and-cents text: `$1 240.50`, `−$23.40`. Whole-dollar
/// amounts keep their `.00` only when `force_cents` is set.
pub fn money_text(cents: i64, force_cents: bool) -> String {
    let neg = cents < 0;
    let abs = cents.unsigned_abs();
    let dollars = group(&(abs / 100).to_string());
    let rem = abs % 100;
    let mut s = String::new();
    if neg {
        s.push(MINUS);
    }
    s.push('$');
    s.push_str(&dollars);
    if rem != 0 || force_cents {
        s.push('.');
        s.push_str(&format!("{rem:02}"));
    }
    s
}

/// Money as a two-section layout job: the dollars at full ink, the cents at
/// 70% alpha (spec §4.2). `size` is the mono point size.
pub fn money_job(cents: i64, size: f32, ink: Color32) -> LayoutJob {
    let neg = cents < 0;
    let abs = cents.unsigned_abs();
    let head = format!(
        "{}${}",
        if neg {
            MINUS.to_string()
        } else {
            String::new()
        },
        group(&(abs / 100).to_string())
    );
    let tail = format!(".{:02}", abs % 100);
    let mut job = LayoutJob::default();
    let font = FontId::new(
        size,
        if size >= type_scale::STAT {
            mono_medium()
        } else {
            mono()
        },
    );
    job.append(&head, 0.0, TextFormat::simple(font.clone(), ink));
    let faded = Color32::from_rgba_unmultiplied(ink.r(), ink.g(), ink.b(), 178);
    job.append(&tail, 0.0, TextFormat::simple(font, faded));
    job
}

/// A probability with its n: `41.2% (n = 1.2M)`.
pub fn prob(p: f64, n: u64) -> String {
    format!("{:.1}% (n = {})", p * 100.0, compact_n(n))
}

/// A probability with its 95% CI half-width: `46.8 ±1.4%`.
pub fn prob_ci(p: f64, ci_half: f64) -> String {
    format!("{:.1} ±{:.1}%", p * 100.0, ci_half * 100.0)
}

/// Compact counts for provenance lines: `1.2M`, `50k`, `847,213` stays
/// grouped with thin spaces below 10k.
pub fn compact_n(n: u64) -> String {
    if n >= 10_000_000 {
        format!("{:.0}M", n as f64 / 1e6)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 10_000 {
        format!("{:.0}k", n as f64 / 1e3)
    } else {
        group(&n.to_string())
    }
}

/// Signed compact money for captions: `−$23.40/h`.
pub fn money_per_hour(cents_per_hour: f64) -> String {
    let neg = cents_per_hour < 0.0;
    let abs = cents_per_hour.abs();
    format!(
        "{}${:.2}/h",
        if neg {
            MINUS.to_string()
        } else {
            String::new()
        },
        abs / 100.0
    )
}

/// An edge per resolved dollar: `−1.41% per resolved $`.
pub fn edge_pct(edge: f64) -> String {
    let neg = edge < 0.0;
    format!(
        "{}{:.2}%",
        if neg {
            MINUS.to_string()
        } else {
            String::new()
        },
        edge.abs() * 100.0
    )
}

/// A multiple of budget: `2.15×`.
pub fn multiple(x: f64) -> String {
    format!("{x:.2}×")
}

/// Hours from rolls at a pace: `2 h 58 m`.
pub fn hours(rolls: f64, rolls_per_hour: f64) -> String {
    let h = rolls / rolls_per_hour.max(1.0);
    let whole = h.floor() as u64;
    let mins = ((h - whole as f64) * 60.0).round() as u64;
    if whole == 0 {
        format!("{mins} m")
    } else {
        format!("{whole} h {mins:02} m")
    }
}

/// A seed in the product's canonical hex form.
pub fn seed_hex(seed: u64) -> String {
    format!("0x{seed:016X}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_uses_thin_space_and_true_minus() {
        assert_eq!(money_text(124_050, false), format!("$1{THIN_SPACE}240.50"));
        assert_eq!(money_text(-2_340, false), format!("{MINUS}$23.40"));
        assert_eq!(
            money_text(50_000_000, false),
            format!("$500{THIN_SPACE}000")
        );
        assert_eq!(money_text(0, false), "$0");
        assert_eq!(money_text(0, true), "$0.00");
        assert_eq!(
            money_text(123_456_789, false),
            format!("$1{THIN_SPACE}234{THIN_SPACE}567.89")
        );
        // Never a hyphen-minus, never parentheses.
        assert!(!money_text(-100, false).contains('-'));
        assert!(!money_text(-100, false).contains('('));
    }

    #[test]
    fn compact_counts() {
        assert_eq!(compact_n(1_200_000), "1.2M");
        assert_eq!(compact_n(50_000), "50k");
        assert_eq!(compact_n(9_999), format!("9{THIN_SPACE}999"));
        assert_eq!(compact_n(100), "100");
    }

    #[test]
    fn hours_reads_naturally() {
        assert_eq!(hours(297.0, 100.0), "2 h 58 m");
        assert_eq!(hours(43.0, 100.0), "26 m");
    }

    #[test]
    fn probability_and_edge_forms() {
        assert_eq!(prob(0.412, 1_200_000), "41.2% (n = 1.2M)");
        assert_eq!(prob_ci(0.468, 0.014), "46.8 ±1.4%");
        assert_eq!(edge_pct(-0.01414), format!("{MINUS}1.41%"));
        assert_eq!(multiple(2.146), "2.15×");
    }
}

#[cfg(test)]
mod lint {
    /// Product rule: money is formatted HERE and nowhere else. This walks
    /// the source tree and rejects ad-hoc `format!("${…}")`-style dollar
    /// interpolation outside the allowlist (this module; the sentence codec,
    /// which renders through money_text; tests and docs).
    #[test]
    fn no_ad_hoc_money_formatting() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        visit(&root, &mut offenders);
        assert!(
            offenders.is_empty(),
            "ad-hoc dollar formatting found (use ui::numerals):\n{}",
            offenders.join("\n")
        );
    }

    fn visit(dir: &std::path::Path, offenders: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, offenders);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if name == "numerals.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for (ln, line) in text.lines().enumerate() {
                let t = line.trim_start();
                if t.starts_with("//") || t.starts_with("///") || t.starts_with("//!") {
                    continue;
                }
                // `"$…{…}"` inside a string literal = interpolated dollars.
                if line.contains("\"$") && line.contains('{') && line.contains("format!") {
                    offenders.push(format!("{}:{}: {}", path.display(), ln + 1, line.trim()));
                }
            }
        }
    }
}
