// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! The motion system (spec §4.6): three named durations, one settle curve,
//! and two DISJOINT channels enforced by type.
//!
//! * [`Tweens`] animates layout and structural motion only — drawers, FLIP
//!   re-sorts, crossfades. Statistical marks have no way in.
//! * [`StreamInterp`] is the only motion a statistical mark may use: linear
//!   interpolation between two REAL successive snapshots, alpha clamped to
//!   1 — never extrapolation, never a value that was not computed. Count-ups
//!   are impossible by construction.
//!
//! Nothing moves that didn't happen (Principle 4): when reduced motion is
//! on, every duration collapses to zero and streams update discretely.

/// Hover, toggle, focus ring.
pub const MICRO: f32 = 0.12;
/// Panel/drawer transitions, re-ranks, tab cuts.
pub const STRUCTURAL: f32 = 0.24;
/// Histogram↔ECDF flips, progression morphs, Explorer FLIP re-sorts.
pub const MORPH: f32 = 0.40;

/// The one easing curve: cubic-bezier(0.22, 1.0, 0.36, 1.0). Objects arrive
/// and settle like a placed chip; overshoot is impossible (control points
/// keep y within [0, 1]).
pub fn settle(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t == 0.0 || t == 1.0 {
        return t;
    }
    // Solve the bezier x(u) = t for u by Newton iteration, then evaluate
    // y(u). x(u) = 3(1-u)²u·0.22 + 3(1-u)u²·0.36 + u³.
    let (x1, y1, x2, y2) = (0.22f32, 1.0f32, 0.36f32, 1.0f32);
    let bez = |a: f32, b: f32, u: f32| {
        let v = 1.0 - u;
        3.0 * v * v * u * a + 3.0 * v * u * u * b + u * u * u
    };
    let dbez = |a: f32, b: f32, u: f32| {
        let v = 1.0 - u;
        3.0 * v * v * a + 6.0 * v * u * (b - a) + 3.0 * u * u * (1.0 - b)
    };
    let mut u = t;
    for _ in 0..8 {
        let err = bez(x1, x2, u) - t;
        let d = dbez(x1, x2, u);
        if d.abs() < 1e-6 {
            break;
        }
        u = (u - err / d).clamp(0.0, 1.0);
    }
    bez(y1, y2, u)
}

/// Global motion switches. `reduced` collapses every tier to 0 ms and makes
/// [`StreamInterp`] return raw snapshots.
#[derive(Clone, Copy, Debug, Default)]
pub struct MotionPrefs {
    /// In-app override (always available) OR'd with the OS setting where
    /// detectable.
    pub reduced: bool,
}

impl MotionPrefs {
    pub fn duration(&self, base: f32) -> f32 {
        if self.reduced {
            0.0
        } else {
            base
        }
    }
}

/// Detect the OS reduced-motion setting where the platform exposes one
/// cheaply. macOS and Windows are wired in the hardening milestone via their
/// native queries; everywhere else the in-app override governs.
pub fn os_reduced_motion() -> bool {
    #[cfg(target_os = "macos")]
    {
        // `defaults read com.apple.universalaccess reduceMotion` mirrors
        // NSWorkspace.accessibilityDisplayShouldReduceMotion without an
        // objc dependency; absence of the key means "off".
        static ONCE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ONCE.get_or_init(|| {
            std::process::Command::new("defaults")
                .args(["read", "com.apple.universalaccess", "reduceMotion"])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
                .unwrap_or(false)
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Interpolation between two real successive stream snapshots. Alpha is
/// derived from wall time between polls and CLAMPED to 1: the displayed
/// value never passes the newest real estimate.
#[derive(Clone, Debug, Default)]
pub struct StreamInterp {
    prev: Vec<f32>,
    cur: Vec<f32>,
    /// Seconds since `cur` arrived, advanced by the frame clock.
    age: f32,
    /// Seconds between the last two snapshots (the poll cadence actually
    /// observed), floor 1 ms.
    gap: f32,
}

impl StreamInterp {
    /// Install a new snapshot; the previous current becomes the lerp start.
    pub fn push(&mut self, snapshot: Vec<f32>, gap_seconds: f32) {
        std::mem::swap(&mut self.prev, &mut self.cur);
        self.cur = snapshot;
        if self.prev.len() != self.cur.len() {
            // Shape changed (bin fold, new run): jump, never tween across
            // incompatible spaces.
            self.prev = self.cur.clone();
        }
        self.age = 0.0;
        self.gap = gap_seconds.max(0.001);
    }

    pub fn tick(&mut self, dt: f32) {
        self.age += dt;
    }

    /// The displayed values this frame. With reduced motion, the raw newest
    /// snapshot.
    pub fn values(&self, prefs: MotionPrefs) -> Vec<f32> {
        if prefs.reduced || self.prev.is_empty() {
            return self.cur.clone();
        }
        let alpha = (self.age / self.gap).clamp(0.0, 1.0);
        self.cur
            .iter()
            .zip(&self.prev)
            .map(|(&c, &p)| p + (c - p) * alpha)
            .collect()
    }

    pub fn is_settled(&self) -> bool {
        self.age >= self.gap
    }
}

/// Structural tween registry: layout-only animation state keyed by id.
/// Statistical marks cannot reach this — it hands out positions/opacities,
/// not data values.
#[derive(Default)]
pub struct Tweens {
    items: std::collections::HashMap<egui::Id, Tween>,
}

#[derive(Clone, Copy, Debug)]
struct Tween {
    from: f32,
    to: f32,
    t: f32,
    dur: f32,
}

impl Tweens {
    /// Drive `id` toward `target`; returns the eased current value. Call
    /// every frame while the element is visible.
    pub fn drive(&mut self, id: egui::Id, target: f32, dur: f32, dt: f32) -> f32 {
        let tw = self.items.entry(id).or_insert(Tween {
            from: target,
            to: target,
            t: 1.0,
            dur: dur.max(1e-6),
        });
        if (tw.to - target).abs() > f32::EPSILON {
            let cur = tw.from + (tw.to - tw.from) * settle((tw.t / tw.dur).clamp(0.0, 1.0));
            *tw = Tween {
                from: cur,
                to: target,
                t: 0.0,
                dur: dur.max(1e-6),
            };
        }
        tw.t = (tw.t + dt).min(tw.dur);
        let v = tw.from + (tw.to - tw.from) * settle((tw.t / tw.dur).clamp(0.0, 1.0));
        if dur <= 0.0 {
            tw.t = tw.dur;
            return tw.to;
        }
        v
    }

    /// True while any tween is mid-flight (the app requests repaint then).
    pub fn any_active(&self) -> bool {
        self.items.values().any(|t| t.t < t.dur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settle_is_monotone_and_never_overshoots() {
        let mut prev = 0.0f32;
        for i in 0..=100 {
            let t = i as f32 / 100.0;
            let y = settle(t);
            assert!((0.0..=1.0001).contains(&y), "y({t}) = {y}");
            assert!(y >= prev - 1e-4, "not monotone at {t}: {y} < {prev}");
            prev = y;
        }
        assert_eq!(settle(0.0), 0.0);
        assert!((settle(1.0) - 1.0).abs() < 1e-6);
        // Ease-out character: the first half covers most of the distance.
        assert!(settle(0.5) > 0.8);
    }

    #[test]
    fn stream_interp_clamps_and_never_extrapolates() {
        let mut s = StreamInterp::default();
        s.push(vec![0.0, 10.0], 0.1);
        s.push(vec![10.0, 20.0], 0.1);
        s.tick(0.05);
        let v = s.values(MotionPrefs::default());
        assert!((v[0] - 5.0).abs() < 1e-4);
        // Long past the gap: clamped at the newest real value, no overshoot.
        s.tick(10.0);
        let v = s.values(MotionPrefs::default());
        assert_eq!(v, vec![10.0, 20.0]);
        // Reduced motion: raw snapshot immediately.
        let v = s.values(MotionPrefs { reduced: true });
        assert_eq!(v, vec![10.0, 20.0]);
    }

    #[test]
    fn stream_interp_jumps_on_shape_change() {
        let mut s = StreamInterp::default();
        s.push(vec![1.0; 4], 0.1);
        s.push(vec![2.0; 8], 0.1); // bin fold doubled the shape
        let v = s.values(MotionPrefs::default());
        assert_eq!(v, vec![2.0; 8], "no tween across incompatible spaces");
    }

    #[test]
    fn tweens_settle_to_target_and_report_activity() {
        let mut tw = Tweens::default();
        let id = egui::Id::new("x");
        let v0 = tw.drive(id, 100.0, 0.24, 0.0);
        assert_eq!(v0, 100.0, "first sight snaps (no entrance dance)");
        tw.drive(id, 200.0, 0.24, 0.016);
        assert!(tw.any_active());
        let mut v = 0.0;
        for _ in 0..30 {
            v = tw.drive(id, 200.0, 0.24, 0.016);
        }
        assert!((v - 200.0).abs() < 0.5, "settled at {v}");
        assert!(!tw.any_active());
    }
}
