//! Electronic cam table + interpolation kernel (pure math, no I/O).
//!
//! A cam relates a *master phase* to a *follower position* through a table of
//! knots, evaluated every bus cycle. This module is the interpolation
//! primitive only — validation, phase wrapping, and point interpolation — with
//! no engine, config, or fieldbus dependency. The in-cycle cam follower and
//! the flying-shear state machine both build on it (see MOTION-ROADMAP.md).
//!
//! Conventions match PLCopen Part 4 (MC_CamTableSelect / MC_CamIn): the table
//! is expressed in **normalized units** — master phase `u ∈ [0, 1)` over one
//! cam period, follower value `y` in the same normalized `[0, 1]` stroke units
//! — so one table is reusable across master periods and follower strokes. The
//! caller scales: `follower_counts = y * follower_stroke + offset`, and
//! `u = phase(master_counts)`.
//!
//! Interpolation is piecewise **linear** here. The knot/eval API is shaped so a
//! higher-order (cubic / quintic) kernel can slot in behind `CamTable::eval`
//! later without changing callers — that is a deliberate follow-up, not this
//! module.

/// One cam knot: master phase `x` (normalized, strictly increasing across the
/// table) → follower value `y` (normalized stroke).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CamKnot {
    pub x: f64,
    pub y: f64,
}

/// How the master phase is treated at/after the last knot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CamMode {
    /// Rotary / periodic cam: the master phase wraps modulo the period, so the
    /// curve repeats every cycle. Requires `y` to match at both ends
    /// (`first.y == last.y`) or the follower would step at the wrap.
    Cyclic,
    /// One-shot cam: the follower holds the last knot's `y` once the master
    /// passes the last knot (and holds the first knot's `y` before the first).
    OneShot,
}

/// A validated cam table. Construct via [`CamTable::new`], which rejects the
/// malformed shapes the evaluator can't serve.
#[derive(Debug, Clone)]
pub struct CamTable {
    knots: Vec<CamKnot>,
    mode: CamMode,
    period: f64, // x-span: knots.last().x - knots.first().x (>0 by validation)
}

/// Why a cam table was rejected at build time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CamError {
    /// Fewer than two knots — nothing to interpolate between.
    TooFewKnots,
    /// A knot `x` (or `y`) was NaN/inf — no finite curve.
    NonFinite,
    /// Knot `x` values are not strictly increasing (needed for a single-valued
    /// interpolant and an unambiguous phase lookup).
    NonMonotonicX,
    /// Cyclic table whose endpoints don't match in `y` — would step at the wrap.
    CyclicEndpointMismatch,
}

impl std::fmt::Display for CamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            CamError::TooFewKnots => "cam table needs at least two knots",
            CamError::NonFinite => "cam table has a non-finite knot",
            CamError::NonMonotonicX => "cam table x values must be strictly increasing",
            CamError::CyclicEndpointMismatch => {
                "cyclic cam table endpoints must have equal y (else it steps at the wrap)"
            }
        };
        f.write_str(s)
    }
}

impl CamTable {
    /// Validate and build. See [`CamError`] for the rejected shapes.
    pub fn new(knots: Vec<CamKnot>, mode: CamMode) -> Result<Self, CamError> {
        if knots.len() < 2 {
            return Err(CamError::TooFewKnots);
        }
        for k in &knots {
            if !k.x.is_finite() || !k.y.is_finite() {
                return Err(CamError::NonFinite);
            }
        }
        for w in knots.windows(2) {
            if w[1].x <= w[0].x {
                return Err(CamError::NonMonotonicX);
            }
        }
        let period = knots[knots.len() - 1].x - knots[0].x;
        // period > 0 follows from strictly-increasing x with >= 2 knots.
        if mode == CamMode::Cyclic && knots[0].y != knots[knots.len() - 1].y {
            return Err(CamError::CyclicEndpointMismatch);
        }
        Ok(CamTable {
            knots,
            mode,
            period,
        })
    }

    /// The x-span of the table (`last.x - first.x`). For a cyclic cam this is
    /// the phase period; for scaling `master_counts → x` the caller maps one
    /// `master_period` of counts onto this span.
    pub fn period(&self) -> f64 {
        self.period
    }

    pub fn mode(&self) -> CamMode {
        self.mode
    }

    /// Map an absolute master phase `x` into the table's domain:
    /// - `Cyclic`: wrapped into `[first.x, last.x)` (handles negative x too).
    /// - `OneShot`: clamped to `[first.x, last.x]`.
    ///
    /// Non-finite input maps to the first knot (defensive; upstream already
    /// rejects non-finite params, so this is belt-and-suspenders).
    pub fn wrap(&self, x: f64) -> f64 {
        if !x.is_finite() {
            return self.knots[0].x;
        }
        let x0 = self.knots[0].x;
        match self.mode {
            CamMode::OneShot => x.clamp(x0, self.knots[self.knots.len() - 1].x),
            CamMode::Cyclic => {
                // rem_euclid on the offset keeps the result in [0, period)
                // for positive AND negative x.
                x0 + (x - x0).rem_euclid(self.period)
            }
        }
    }

    /// Evaluate the follower value at absolute master phase `x`. Applies
    /// [`wrap`](Self::wrap) then piecewise-linear interpolation between the two
    /// bracketing knots. Exact at knots.
    pub fn eval(&self, x: f64) -> f64 {
        let xw = self.wrap(x);
        // Bracket: last knot with x <= xw. Small tables (tens of knots), a
        // linear scan is cheaper than a branch-heavy binary search and keeps
        // the hot path allocation- and panic-free.
        let n = self.knots.len();
        // xw is within [first.x, last.x] after wrap (cyclic gives [first,last)).
        let mut i = 0usize;
        while i + 1 < n && self.knots[i + 1].x <= xw {
            i += 1;
        }
        if i + 1 >= n {
            // xw == last.x exactly (OneShot upper clamp): hold last y.
            return self.knots[n - 1].y;
        }
        let a = self.knots[i];
        let b = self.knots[i + 1];
        let span = b.x - a.x; // > 0 by validation
        let t = (xw - a.x) / span;
        a.y + t * (b.y - a.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tbl(pts: &[(f64, f64)], mode: CamMode) -> CamTable {
        CamTable::new(pts.iter().map(|&(x, y)| CamKnot { x, y }).collect(), mode).unwrap()
    }

    #[test]
    fn rejects_malformed_tables() {
        assert_eq!(
            CamTable::new(vec![CamKnot { x: 0.0, y: 0.0 }], CamMode::OneShot).unwrap_err(),
            CamError::TooFewKnots
        );
        assert_eq!(
            CamTable::new(
                vec![
                    CamKnot { x: 0.0, y: 0.0 },
                    CamKnot { x: 0.0, y: 1.0 } // non-increasing x
                ],
                CamMode::OneShot
            )
            .unwrap_err(),
            CamError::NonMonotonicX
        );
        assert_eq!(
            CamTable::new(
                vec![
                    CamKnot { x: 0.0, y: 0.0 },
                    CamKnot {
                        x: f64::NAN,
                        y: 1.0
                    }
                ],
                CamMode::OneShot
            )
            .unwrap_err(),
            CamError::NonFinite
        );
        // Cyclic with mismatched endpoints steps at the wrap → rejected.
        assert_eq!(
            CamTable::new(
                vec![
                    CamKnot { x: 0.0, y: 0.0 },
                    CamKnot { x: 1.0, y: 0.5 } // y(0) != y(1)
                ],
                CamMode::Cyclic
            )
            .unwrap_err(),
            CamError::CyclicEndpointMismatch
        );
    }

    #[test]
    fn exact_at_knots() {
        let c = tbl(&[(0.0, 0.0), (0.5, 2.0), (1.0, 0.0)], CamMode::Cyclic);
        assert_eq!(c.eval(0.0), 0.0);
        assert_eq!(c.eval(0.5), 2.0);
        // 1.0 wraps to 0.0 for cyclic → first knot y.
        assert_eq!(c.eval(1.0), 0.0);
    }

    #[test]
    fn linear_between_knots_is_monotonic() {
        let c = tbl(&[(0.0, 0.0), (1.0, 10.0)], CamMode::OneShot);
        assert_eq!(c.eval(0.25), 2.5);
        assert_eq!(c.eval(0.5), 5.0);
        assert_eq!(c.eval(0.75), 7.5);
        // strictly increasing across the segment
        let mut prev = f64::NEG_INFINITY;
        for i in 0..=10 {
            let v = c.eval(i as f64 / 10.0);
            assert!(v > prev - 1e-12, "monotonic at {i}");
            prev = v;
        }
    }

    #[test]
    fn cyclic_wraps_with_period() {
        let c = tbl(&[(0.0, 0.0), (0.5, 2.0), (1.0, 0.0)], CamMode::Cyclic);
        assert_eq!(c.period(), 1.0);
        // one period later == same value
        assert_eq!(c.eval(0.25), c.eval(1.25));
        assert_eq!(c.eval(0.5), c.eval(2.5));
        // negative phase wraps too
        assert_eq!(c.eval(-0.75), c.eval(0.25));
    }

    #[test]
    fn oneshot_holds_endpoints() {
        let c = tbl(&[(0.0, 1.0), (1.0, 5.0)], CamMode::OneShot);
        // before the first knot → hold first y
        assert_eq!(c.eval(-3.0), 1.0);
        // after the last knot → hold last y (no wrap)
        assert_eq!(c.eval(2.0), 5.0);
        assert_eq!(c.eval(100.0), 5.0);
    }

    #[test]
    fn eval_is_bounded_by_knot_extremes() {
        let c = tbl(
            &[(0.0, 0.0), (0.3, 3.0), (0.7, -1.0), (1.0, 0.0)],
            CamMode::Cyclic,
        );
        let (lo, hi) = (-1.0, 3.0);
        for i in 0..1000 {
            let v = c.eval(i as f64 / 500.0 - 1.0); // sweep [-1, 1)
            assert!(v >= lo - 1e-9 && v <= hi + 1e-9, "in bounds at {i}: {v}");
        }
    }

    #[test]
    fn wrap_maps_into_domain() {
        let c = tbl(&[(0.0, 0.0), (1.0, 0.0)], CamMode::Cyclic);
        // cyclic wrap stays in [0, 1)
        for x in [-2.5, -0.1, 0.0, 0.4, 1.0, 3.7] {
            let w = c.wrap(x);
            assert!((0.0..1.0).contains(&w), "wrap({x}) = {w} not in [0,1)");
        }
        // one-shot clamps to [0, 1]
        let o = tbl(&[(0.0, 0.0), (1.0, 1.0)], CamMode::OneShot);
        assert_eq!(o.wrap(-5.0), 0.0);
        assert_eq!(o.wrap(5.0), 1.0);
    }

    #[test]
    fn non_finite_phase_is_defensive_not_a_panic() {
        let c = tbl(&[(0.0, 7.0), (0.5, 1.0), (1.0, 7.0)], CamMode::Cyclic);
        // NaN/inf must not panic; maps to the first knot.
        assert_eq!(c.eval(f64::NAN), 7.0);
        assert_eq!(c.eval(f64::INFINITY), 7.0);
    }

    #[test]
    fn nonzero_origin_table() {
        // x domain need not start at 0 — a shear "cut window" often starts mid-line.
        let c = tbl(&[(10.0, 0.0), (12.0, 4.0), (14.0, 0.0)], CamMode::Cyclic);
        assert_eq!(c.period(), 4.0);
        assert_eq!(c.eval(10.0), 0.0);
        assert_eq!(c.eval(12.0), 4.0);
        assert_eq!(c.eval(11.0), 2.0);
        assert_eq!(c.eval(14.0), c.eval(10.0)); // wraps
    }
}
