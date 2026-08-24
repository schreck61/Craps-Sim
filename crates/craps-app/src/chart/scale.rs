// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Linear (and opt-in log-y) data↔screen scales with nice ticks.
//!
//! One scale, one axis: the $/×-of-budget twin axis is one scale with a
//! second labeler, structurally incapable of becoming two scales.

/// A monotone map from a data domain to a screen range. `log` is the
/// spec's opt-in, always-labeled log-y (§6.4): counts below the floor of
/// 0.8 clamp so a zero bin has no height rather than −∞.
#[derive(Clone, Copy, Debug)]
pub struct LinearScale {
    pub d0: f64,
    pub d1: f64,
    pub r0: f32,
    pub r1: f32,
    pub log: bool,
}

const LOG_FLOOR: f64 = 0.8;

// `to_screen(&self)`/`from_screen(&self)` read better as scale methods than
// clippy's Copy-self/from-free-function conventions here.
#[allow(clippy::wrong_self_convention)]
impl LinearScale {
    pub fn new(domain: (f64, f64), range: (f32, f32)) -> Self {
        let (d0, d1) = if domain.0 == domain.1 {
            (domain.0, domain.0 + 1.0)
        } else {
            domain
        };
        Self {
            d0,
            d1,
            r0: range.0,
            r1: range.1,
            log: false,
        }
    }

    pub fn log_y(domain_max: f64, range: (f32, f32)) -> Self {
        Self {
            d0: LOG_FLOOR,
            d1: domain_max.max(LOG_FLOOR * 10.0),
            r0: range.0,
            r1: range.1,
            log: true,
        }
    }

    fn fwd(&self, v: f64) -> f64 {
        if self.log {
            v.max(LOG_FLOOR).ln()
        } else {
            v
        }
    }

    fn inv(&self, t: f64) -> f64 {
        if self.log {
            t.exp()
        } else {
            t
        }
    }

    pub fn to_screen(&self, v: f64) -> f32 {
        let (a, b) = (self.fwd(self.d0), self.fwd(self.d1));
        let t = ((self.fwd(v) - a) / (b - a)) as f32;
        self.r0 + (self.r1 - self.r0) * t
    }

    pub fn from_screen(&self, s: f32) -> f64 {
        let (a, b) = (self.fwd(self.d0), self.fwd(self.d1));
        let t = ((s - self.r0) / (self.r1 - self.r0)) as f64;
        self.inv(a + (b - a) * t)
    }

    /// Nice tick positions (1/2/5 steps), roughly `target` of them. Log
    /// scales tick at powers of ten within the domain.
    pub fn ticks(&self, target: usize) -> Vec<f64> {
        if self.log {
            let mut out = Vec::new();
            let mut p = 1.0f64;
            while p <= self.d1 {
                if p >= self.d0 {
                    out.push(p);
                }
                p *= 10.0;
            }
            return out;
        }
        let span = (self.d1 - self.d0).abs();
        if span == 0.0 || target == 0 {
            return vec![self.d0];
        }
        let raw = span / target as f64;
        let mag = 10f64.powf(raw.log10().floor());
        let norm = raw / mag;
        let step = if norm <= 1.5 {
            1.0
        } else if norm <= 3.5 {
            2.0
        } else if norm <= 7.5 {
            5.0
        } else {
            10.0
        } * mag;
        let lo = self.d0.min(self.d1);
        let hi = self.d0.max(self.d1);
        let first = (lo / step).ceil() * step;
        let mut out = Vec::new();
        let mut v = first;
        let mut guard = 0;
        while v <= hi + step * 1e-9 && guard < 1000 {
            // Snap floating error to the step grid.
            out.push((v / step).round() * step);
            v += step;
            guard += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_orients() {
        // y axes run top-down: range (bottom, top) with r1 < r0 must work.
        let s = LinearScale::new((0.0, 100.0), (400.0, 0.0));
        assert_eq!(s.to_screen(0.0), 400.0);
        assert_eq!(s.to_screen(100.0), 0.0);
        let v = s.from_screen(s.to_screen(37.5));
        assert!((v - 37.5).abs() < 1e-6);
    }

    #[test]
    fn ticks_are_nice_and_cover() {
        let s = LinearScale::new((0.0, 1234.0), (0.0, 800.0));
        let t = s.ticks(6);
        assert!(t.len() >= 4 && t.len() <= 9, "{t:?}");
        assert!(t.iter().all(|v| (v / 200.0).fract().abs() < 1e-9), "{t:?}");
        // Degenerate domain must not loop or panic.
        let s = LinearScale::new((5.0, 5.0), (0.0, 100.0));
        assert!(!s.ticks(5).is_empty());
    }

    #[test]
    fn log_scale_floors_instead_of_diverging() {
        let s = LinearScale::log_y(1_000_000.0, (400.0, 0.0));
        assert!(s.to_screen(0.0) >= 399.9, "zero clamps to the floor");
        assert!(s.to_screen(1_000_000.0) <= 0.1);
        let ticks = s.ticks(0);
        assert!(ticks.contains(&1.0) && ticks.contains(&1000.0));
    }
}
