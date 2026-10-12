//! Timing functions: linear, cubic Bézier, and steps.
//!
//! **The named presets are CSS's control points, to the digit, and that is a design
//! decision rather than a convenience.** ../PLAN.md §45 ships one animation model
//! implemented twice — once here for the native backend and once in
//! `phase_1/webruntime/zkite-runtime.js` for the WebView — and §14.2 is a list of what
//! happens when two backends implement the same feature from two descriptions. Two
//! implementations of *one* published curve is a very different risk from two
//! implementations of two curves: `cubic-bezier(0.42, 0, 0.58, 1)` has a single
//! correct answer that both sides can be checked against, and the parity test does
//! exactly that.
//!
//! This is *not* the same as driving the WebView with CSS transitions. Sharing a
//! curve's four control points shares arithmetic; using `transition-*` would import
//! the browser's interruption behaviour, its property-change triggers and its
//! `transitionend` timing as well, which is the shape §32.1 rejected `<label>` for.

/// Where a stepped timing function jumps, matching CSS `steps()`'s second argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StepPosition {
    /// `steps(n, jump-end)` — the CSS default. Holds the start value for the first
    /// interval and reaches 1 only at the very end.
    #[default]
    End,
    /// `steps(n, jump-start)` — jumps immediately, so the start value is never held.
    Start,
    /// `steps(n, jump-none)` — both 0 and 1 are held; there are `n - 1` jumps.
    None,
    /// `steps(n, jump-both)` — neither 0 nor 1 is held; there are `n + 1` jumps.
    Both,
}

/// A timing function: input is linear progress in `[0, 1]`, output is eased progress.
///
/// Output is **not** clamped to `[0, 1]`. A Bézier with a control point outside the
/// unit square overshoots on purpose — that is how "back out" and "anticipate" curves
/// are expressed, and clamping here would silently flatten them. Callers that need a
/// clamped channel (an opacity, say) clamp the *value*, not the progress, because
/// which channels can legally overshoot is the caller's question.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Easing {
    Linear,
    /// CSS `cubic-bezier(x1, y1, x2, y2)` with the endpoints fixed at (0,0) and (1,1).
    CubicBezier { x1: f32, y1: f32, x2: f32, y2: f32 },
    /// CSS `steps(count, position)`. `count` is clamped to at least 1.
    Steps { count: u32, position: StepPosition },
}

impl Default for Easing {
    fn default() -> Self {
        Easing::Linear
    }
}

impl Easing {
    /// CSS `ease` — the browser default for a bare `transition`.
    pub const EASE: Easing = Easing::CubicBezier { x1: 0.25, y1: 0.1, x2: 0.25, y2: 1.0 };
    /// CSS `ease-in`.
    pub const EASE_IN: Easing = Easing::CubicBezier { x1: 0.42, y1: 0.0, x2: 1.0, y2: 1.0 };
    /// CSS `ease-out` — the right default for anything entering the screen, since it
    /// arrives decelerating.
    pub const EASE_OUT: Easing = Easing::CubicBezier { x1: 0.0, y1: 0.0, x2: 0.58, y2: 1.0 };
    /// CSS `ease-in-out`.
    pub const EASE_IN_OUT: Easing = Easing::CubicBezier { x1: 0.42, y1: 0.0, x2: 0.58, y2: 1.0 };

    /// Eased progress for linear progress `t`.
    ///
    /// `t` outside `[0, 1]` is clamped: a `Track` never asks for one, and answering
    /// an extrapolated value would turn a caller's arithmetic slip into a value that
    /// looks plausible and is wrong.
    pub fn eval(&self, t: f32) -> f32 {
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        match *self {
            Easing::Linear => t,
            Easing::CubicBezier { x1, y1, x2, y2 } => cubic_bezier(x1, y1, x2, y2, t),
            Easing::Steps { count, position } => steps(count.max(1), position, t),
        }
    }
}

/// One axis of a cubic Bézier with endpoints (0,0) and (1,1), in polynomial form:
/// `((a*s + b)*s + c)*s`. Returned as the three coefficients so the solver can
/// evaluate the curve and its derivative without recomputing them.
fn coefficients(p1: f32, p2: f32) -> (f32, f32, f32) {
    let c = 3.0 * p1;
    let b = 3.0 * (p2 - p1) - c;
    let a = 1.0 - c - b;
    (a, b, c)
}

fn eval_poly(a: f32, b: f32, c: f32, s: f32) -> f32 {
    ((a * s + b) * s + c) * s
}

fn eval_slope(a: f32, b: f32, c: f32, s: f32) -> f32 {
    (3.0 * a * s + 2.0 * b) * s + c
}

/// Newton–Raphson with a bisection fallback, the standard solve for this curve.
///
/// The curve is parametric — `x` and `y` are both functions of an internal parameter
/// `s` that is *not* the progress — so evaluating it means first inverting `x(s) = t`
/// and only then reading `y(s)`. Newton converges in a handful of iterations over
/// most of the range and fails exactly where the curve is flat in `x`, which is not a
/// pathological input: `ease-out` is `cubic-bezier(0, 0, 0.58, 1)`, whose `x` slope at
/// `s = 0` is precisely zero. Bisection cannot fail on a monotonic function, so it is
/// the fallback rather than the primary.
///
/// **Control-point `x` is clamped to `[0, 1]` and `y` is not.** CSS requires the same
/// thing for the same reason: an `x` outside the unit interval makes `x(s)`
/// non-monotonic, and a non-monotonic `x` has no single inverse — the curve stops
/// being a timing function at all. `y` outside it is just overshoot.
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let x1 = x1.clamp(0.0, 1.0);
    let x2 = x2.clamp(0.0, 1.0);
    // The identity curve, which `Linear` also produces — worth short-circuiting
    // because `cubic-bezier(0, 0, 1, 1)` is what a "no easing" spec deserializes to
    // and it is the one case where the solver would do real work for nothing.
    if x1 == 0.0 && y1 == 0.0 && x2 == 1.0 && y2 == 1.0 {
        return t;
    }
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }

    let (ax, bx, cx) = coefficients(x1, x2);
    let (ay, by, cy) = coefficients(y1, y2);

    // Newton, seeded at `s = t`. That seed is exact for the identity curve and close
    // for every curve near it, which is most of them.
    let mut s = t;
    for _ in 0..NEWTON_ITERATIONS {
        let err = eval_poly(ax, bx, cx, s) - t;
        if err.abs() < SOLVE_EPSILON {
            return eval_poly(ay, by, cy, s);
        }
        let slope = eval_slope(ax, bx, cx, s);
        if slope.abs() < 1e-6 {
            break;
        }
        s -= err / slope;
    }

    // Bisection over the whole domain rather than around Newton's last guess: that
    // guess can have left `[0, 1]` entirely (a Newton step off a near-flat slope is
    // unbounded), and restarting from the full interval is both correct and cheap.
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    let mut s = t.clamp(0.0, 1.0);
    for _ in 0..BISECTION_ITERATIONS {
        let x = eval_poly(ax, bx, cx, s);
        if (x - t).abs() < SOLVE_EPSILON {
            break;
        }
        if x > t {
            hi = s;
        } else {
            lo = s;
        }
        s = (lo + hi) * 0.5;
    }
    eval_poly(ay, by, cy, s)
}

/// Enough for the residual to be well under a device pixel on any plausible channel:
/// a 1000dp translate at this tolerance is off by 1e-3 dp.
const SOLVE_EPSILON: f32 = 1e-6;
const NEWTON_ITERATIONS: u32 = 8;
/// `2^-30` of the unit interval — far past `f32`'s ability to represent the
/// difference, so this bound is never the reason an answer is imprecise.
const BISECTION_ITERATIONS: u32 = 30;

/// CSS Easing Functions Level 1's step algorithm, transcribed rather than
/// re-derived — the four positions differ only in two of its lines, and writing them
/// as four separate formulas is how `jump-start` ends up spelled `ceil(t*n)/n`, which
/// agrees with the spec everywhere *except* exactly on a step boundary. That is a
/// one-frame disagreement at the one input a test is most likely to probe.
///
/// The spec's "before flag" step is omitted: it exists for an animation being sampled
/// in the reverse direction at an exact boundary, and [`Track`](crate::Track) handles
/// direction by reflecting progress before it reaches here, so the flag is never set.
fn steps(count: u32, position: StepPosition, t: f32) -> f32 {
    let n = count as f32;
    let mut step = (t * n).floor();
    if matches!(position, StepPosition::Start | StepPosition::Both) {
        step += 1.0;
    }
    if t >= 0.0 && step < 0.0 {
        step = 0.0;
    }
    let jumps = match position {
        StepPosition::Start | StepPosition::End => n,
        StepPosition::None => n - 1.0,
        StepPosition::Both => n + 1.0,
    };
    if t <= 1.0 && step > jumps {
        step = jumps;
    }
    if jumps <= 0.0 {
        // `steps(1, jump-none)` asks for one step with nowhere to jump — the spec's
        // own formula divides by zero and browsers reject the declaration. Nothing
        // here can reject anything (the value arrived over a wire), so it degrades to
        // a constant rather than to a NaN that would propagate into a layout.
        return 0.0;
    }
    step / jumps
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Every** timing function must land on exactly 1 at the end, or an animation
    /// settles a fraction of a pixel off its target and stays there — the kind of
    /// defect that is invisible in a screenshot and obvious in a diff of two.
    ///
    /// The *start* is deliberately not universal, and that asymmetry is the point:
    /// `jump-start` and `jump-both` are defined to jump before the first interval, so
    /// they begin at 1/4 and 1/5 rather than at 0. Asserting 0 for all ten curves is
    /// the natural first draft of this test, it fails on those two, and "fixing" it
    /// by clamping `t <= 0` to zero would silently turn both into `jump-end`.
    #[test]
    fn every_curve_lands_exactly_on_one_and_only_some_start_at_zero() {
        let all = [
            Easing::Linear,
            Easing::EASE,
            Easing::EASE_IN,
            Easing::EASE_OUT,
            Easing::EASE_IN_OUT,
            Easing::CubicBezier { x1: 0.34, y1: 1.56, x2: 0.64, y2: 1.0 },
            Easing::Steps { count: 4, position: StepPosition::End },
            Easing::Steps { count: 4, position: StepPosition::Start },
            Easing::Steps { count: 4, position: StepPosition::None },
            Easing::Steps { count: 4, position: StepPosition::Both },
        ];
        for c in all {
            assert_eq!(c.eval(1.0), 1.0, "{c:?} at 1");
        }
        for c in &all[..7] {
            assert_eq!(c.eval(0.0), 0.0, "{c:?} at 0");
        }
        assert_eq!(Easing::Steps { count: 4, position: StepPosition::None }.eval(0.0), 0.0);
        assert_eq!(Easing::Steps { count: 4, position: StepPosition::Start }.eval(0.0), 0.25);
        assert_eq!(Easing::Steps { count: 4, position: StepPosition::Both }.eval(0.0), 0.2);
    }

    /// The named presets are checked against externally-known values rather than
    /// against this file's own arithmetic: `cubic-bezier(0.42, 0, 0.58, 1)` at the
    /// midpoint is 0.5 by symmetry, and `ease` is famously *not* 0.5 there. If the
    /// solver were subtly wrong these two would move together; the point of pairing
    /// them is that they cannot.
    #[test]
    fn the_presets_match_their_published_shape() {
        assert!((Easing::EASE_IN_OUT.eval(0.5) - 0.5).abs() < 1e-4, "{}", Easing::EASE_IN_OUT.eval(0.5));
        // `ease` at the midpoint, solved by hand from its own polynomial rather than
        // read out of this implementation: x(s) = s³ - 0.75s² + 0.75s = 0.5 gives
        // s ≈ 0.6999, and y(s) = -1.7s³ + 2.4s² + 0.3s is then ≈ 0.8028.
        let ease_mid = Easing::EASE.eval(0.5);
        assert!((ease_mid - 0.8028).abs() < 1e-3, "ease(0.5) = {ease_mid}");
        // ease-out leads linear everywhere inside the interval; ease-in trails it.
        for t in [0.1f32, 0.25, 0.5, 0.75, 0.9] {
            assert!(Easing::EASE_OUT.eval(t) > t, "ease-out({t}) = {}", Easing::EASE_OUT.eval(t));
            assert!(Easing::EASE_IN.eval(t) < t, "ease-in({t}) = {}", Easing::EASE_IN.eval(t));
        }
    }

    /// The negative control for the solver, and the one that would fail if `eval`
    /// simply returned `t`. `ease-out` at a quarter of the way through is already
    /// nearly half done, which is the entire reason to use it.
    #[test]
    fn easing_is_not_the_identity() {
        assert!((Easing::EASE_OUT.eval(0.25) - 0.4).abs() < 0.06, "{}", Easing::EASE_OUT.eval(0.25));
        assert!(Easing::EASE_OUT.eval(0.25) - 0.25 > 0.1);
        assert_eq!(Easing::Linear.eval(0.25), 0.25);
    }

    /// `ease-out`'s `x` slope at `s = 0` is exactly zero, which is where Newton
    /// divides by (almost) nothing. Without the bisection fallback this returns
    /// garbage or NaN for small `t`; with it, the curve stays monotonic and finite.
    /// This is the test that fails against a Newton-only solver.
    #[test]
    fn a_zero_slope_control_point_does_not_break_the_solver() {
        let flat_start = Easing::CubicBezier { x1: 0.0, y1: 0.0, x2: 0.58, y2: 1.0 };
        let mut prev = 0.0;
        for i in 0..=1000 {
            let t = i as f32 / 1000.0;
            let y = flat_start.eval(t);
            assert!(y.is_finite(), "t={t} produced {y}");
            assert!(y >= prev - 1e-4, "not monotonic at t={t}: {prev} -> {y}");
            prev = y;
        }
        // …and the same at the other end, where `x2 == 1.0` flattens the finish.
        let flat_end = Easing::CubicBezier { x1: 0.42, y1: 0.0, x2: 1.0, y2: 1.0 };
        for i in 0..=1000 {
            assert!(flat_end.eval(i as f32 / 1000.0).is_finite());
        }
    }

    /// Overshoot is a feature and must survive `eval` — a "back out" curve returns
    /// values above 1 in the middle of its run. Clamping progress into `[0, 1]` here
    /// (which is the tempting one-line "safety" edit) silently flattens every such
    /// curve into an ordinary ease, which looks fine and is wrong.
    #[test]
    fn an_overshooting_curve_is_allowed_to_exceed_one() {
        let back_out = Easing::CubicBezier { x1: 0.34, y1: 1.56, x2: 0.64, y2: 1.0 };
        let peak = (0..=100).map(|i| back_out.eval(i as f32 / 100.0)).fold(f32::MIN, f32::max);
        assert!(peak > 1.02, "peak {peak} — overshoot was flattened");
        assert_eq!(back_out.eval(1.0), 1.0, "and it still lands exactly on target");
    }

    /// Input outside the domain is clamped rather than extrapolated. A `Track` never
    /// produces such a `t`; this pins what happens if one ever does, so the failure
    /// is a held value rather than a plausible-looking wrong one.
    #[test]
    fn out_of_range_and_nan_progress_are_clamped_not_extrapolated() {
        assert_eq!(Easing::EASE_OUT.eval(-1.0), 0.0);
        assert_eq!(Easing::EASE_OUT.eval(2.0), 1.0);
        assert_eq!(Easing::EASE_OUT.eval(f32::NAN), 0.0);
        assert_eq!(Easing::Linear.eval(f32::INFINITY), 1.0);
    }

    /// The four `steps()` positions differ from each other in exactly the way CSS
    /// says, checked at the sample points where they disagree. Written as one table
    /// because the failure mode worth catching is two of them collapsing into the
    /// same function.
    #[test]
    fn the_four_step_positions_are_four_different_functions() {
        let t = 0.5f32;
        let n = 4;
        let end = Easing::Steps { count: n, position: StepPosition::End }.eval(t);
        let start = Easing::Steps { count: n, position: StepPosition::Start }.eval(t);
        let none = Easing::Steps { count: n, position: StepPosition::None }.eval(t);
        let both = Easing::Steps { count: n, position: StepPosition::Both }.eval(t);
        assert_eq!(end, 0.5);
        assert_eq!(start, 0.75);
        assert!((none - 2.0 / 3.0).abs() < 1e-6, "{none}");
        assert_eq!(both, 0.6);
        // jump-start never holds 0; jump-end never reaches 1 before the end.
        assert!(Easing::Steps { count: n, position: StepPosition::Start }.eval(0.01) > 0.0);
        assert!(Easing::Steps { count: n, position: StepPosition::End }.eval(0.99) < 1.0);
    }

    /// A degenerate `steps(0, ..)` would divide by zero. Clamped to one step, which
    /// is a hard cut from 0 to 1 — the nearest sensible animation rather than a NaN
    /// that propagates into a layout.
    #[test]
    fn zero_steps_degrades_to_one_step_rather_than_dividing_by_zero() {
        let e = Easing::Steps { count: 0, position: StepPosition::End };
        assert_eq!(e.eval(0.5), 0.0);
        assert_eq!(e.eval(1.0), 1.0);
        assert!(e.eval(0.5).is_finite());
        // `jump-none` with one step is the documented constant-zero case, not a
        // divide by `n - 1 == 0`.
        let none = Easing::Steps { count: 1, position: StepPosition::None };
        assert!(none.eval(0.5).is_finite());
        assert_eq!(none.eval(0.5), 0.0);
    }

    /// Control-point `x` outside `[0, 1]` makes `x(s)` non-monotonic and the inverse
    /// ambiguous, so it is clamped — the same rule CSS applies. The property that
    /// must survive is monotonicity of the *output*, since a timing function that
    /// runs backwards makes an animation visibly stutter.
    #[test]
    fn an_out_of_range_control_point_is_clamped_and_stays_monotonic() {
        let wild = Easing::CubicBezier { x1: -3.0, y1: 0.0, x2: 4.0, y2: 1.0 };
        let mut prev = -1.0;
        for i in 0..=200 {
            let y = wild.eval(i as f32 / 200.0);
            assert!(y.is_finite());
            assert!(y >= prev - 1e-4, "not monotonic at {i}: {prev} -> {y}");
            prev = y;
        }
    }
}
