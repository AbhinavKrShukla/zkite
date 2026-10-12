//! A damped harmonic oscillator, solved in closed form.
//!
//! # Why a spring at all, when there are already easing curves
//!
//! Because of interruption, which is the thing that separates an animation driver
//! from a tween helper. A duration-based curve has no answer to "the user grabbed the
//! sheet while it was still opening": restarting the curve from the current value
//! produces a visible velocity discontinuity, and continuing the old one ignores the
//! new target. A spring's state is `(value, velocity)`, so retargeting mid-flight is
//! not a special case — it is the same integration continued toward a new rest
//! position, and the result is smooth by construction rather than by tuning.
//!
//! [`Timeline`](crate::Timeline)'s [`Retarget`](crate::Interrupt::Retarget) policy is
//! where that gets used, and it applies the same idea to eased tracks (carrying the
//! measured velocity into the replacement) — but for an eased track that is an
//! approximation, and here it is exact.
//!
//! # Closed form, not integration
//!
//! Every value is a function of `t` alone. That is not a micro-optimisation, it is
//! the same correctness property ../PLAN.md §41 built `tick_fling` around: a frame
//! that lands late must not cost the animation any distance or duration. A
//! per-frame numeric integrator (the usual `velocity += accel * dt` loop) makes the
//! result depend on *how the frame times fell*, so an animation that stutters also
//! silently ends up somewhere slightly different — and it can go unstable outright if
//! one `dt` is large, which on this render thread is not hypothetical (§39's 39ms
//! frames, §42's 235ms emulator frames).

/// Where a spring is at some instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringSample {
    pub value: f32,
    /// Units per second, signed. Carried into a retarget so an interrupted spring
    /// keeps its momentum instead of restarting from rest.
    pub velocity: f32,
    /// Whether the spring is close enough to rest that continuing to animate it would
    /// be wasted frames — see [`Spring::REST_DISPLACEMENT`].
    pub settled: bool,
}

/// A damped harmonic oscillator: `m·x'' + c·x' + k·x = 0`, where `x` is displacement
/// from the target.
///
/// Parameterised by the physical constants rather than by "duration and bounciness"
/// because those two derived knobs cannot express an over-damped spring at all, and
/// over-damped is the right answer for most UI (a sheet that overshoots its closed
/// position looks broken). Callers that want the friendly knobs build them on top;
/// [`Spring::critical`] is the one shortcut worth having in here, since critical
/// damping is a property of `k` and `m` and is therefore this type's own arithmetic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    /// `k` — how hard it pulls toward the target. Higher is faster and tighter.
    pub stiffness: f32,
    /// `c` — how strongly motion is resisted. Below critical the spring overshoots
    /// and rings; at or above it, it never crosses the target.
    pub damping: f32,
    /// `m`. Rarely worth changing; raising it slows and loosens everything at once.
    pub mass: f32,
}

impl Default for Spring {
    /// A brisk, slightly-overshooting spring — the shape most UI toolkits default to,
    /// and a reasonable answer for a sheet or a popup. `ζ ≈ 0.82`, so it overshoots
    /// once and comes back; the motion is visually over in roughly a third of a
    /// second. [`settle_time`](Spring::settle_time) reports considerably longer than
    /// that on purpose — it is a guaranteed bound, not the moment the eye stops
    /// seeing movement.
    fn default() -> Self {
        Spring { stiffness: 180.0, damping: 22.0, mass: 1.0 }
    }
}

impl Spring {
    /// Below this displacement (in the channel's own units) *and* below
    /// [`Spring::REST_VELOCITY`], a spring is called settled and stops asking for
    /// frames.
    ///
    /// A thousandth of a unit: for a translate in dp that is a thousandth of a dp,
    /// well under a device pixel at any density this framework will meet, and for a
    /// normalised channel like opacity it is a thousandth of the full range. A
    /// spring's tail is exponential and therefore never reaches zero, so *something*
    /// has to end it; the alternative to a threshold is animating forever at
    /// sub-pixel amplitude, which is what a naive implementation does and which shows
    /// up as a render thread that never sleeps.
    pub const REST_DISPLACEMENT: f32 = 1e-3;
    /// Units per second. Both bounds must hold: a spring passing *through* its target
    /// at speed has zero displacement and is nowhere near done.
    pub const REST_VELOCITY: f32 = 1e-2;

    /// The critically-damped spring for this stiffness and mass — the fastest
    /// approach to the target with no overshoot at all. `c = 2·√(k·m)`.
    pub fn critical(stiffness: f32, mass: f32) -> Spring {
        Spring { stiffness, damping: 2.0 * (stiffness * mass).sqrt(), mass }
    }

    /// Undamped angular frequency, `ω₀ = √(k/m)`.
    fn omega0(&self) -> f32 {
        (self.stiffness / self.mass).sqrt()
    }

    /// Damping ratio `ζ = c / (2·√(k·m))`. Below 1 is under-damped (overshoots and
    /// rings), exactly 1 is critical, above 1 is over-damped (creeps in).
    pub fn damping_ratio(&self) -> f32 {
        self.damping / (2.0 * (self.stiffness * self.mass).sqrt())
    }

    /// Where the spring is `t` seconds after being released at `from` with velocity
    /// `initial_velocity`, heading for `to`.
    ///
    /// Solved as displacement `x = value - to`, which is what makes all three damping
    /// regimes fall out of the same two initial conditions (`x(0) = from - to`,
    /// `x'(0) = v₀`) instead of needing the target threaded through each branch.
    pub fn sample(&self, from: f32, to: f32, initial_velocity: f32, t: f32) -> SpringSample {
        let x0 = from - to;
        let v0 = initial_velocity;
        // Degenerate parameters would divide by zero or take the square root of a
        // negative; answering "already there" degrades a nonsense spring to an
        // instant jump rather than to a NaN that reaches a layout.
        if !(self.stiffness > 0.0) || !(self.mass > 0.0) || self.damping < 0.0 || t < 0.0 {
            return SpringSample { value: to, velocity: 0.0, settled: true };
        }
        let w0 = self.omega0();
        let zeta = self.damping_ratio();

        let (x, v) = if zeta < 1.0 - Self::REGIME_EPSILON {
            // Under-damped: decaying sinusoid.
            let wd = w0 * (1.0 - zeta * zeta).sqrt();
            let decay = (-zeta * w0 * t).exp();
            let a = x0;
            let b = (v0 + zeta * w0 * x0) / wd;
            let (sin, cos) = (wd * t).sin_cos();
            let x = decay * (a * cos + b * sin);
            // d/dt of the above, kept as one expression rather than differentiated
            // numerically: velocity feeds retargeting, and a finite difference there
            // would inject the frame interval into a value whose whole purpose is to
            // be frame-rate independent.
            let v = decay * ((b * wd - zeta * w0 * a) * cos - (a * wd + zeta * w0 * b) * sin);
            (x, v)
        } else if zeta <= 1.0 + Self::REGIME_EPSILON {
            // Critically damped: `(A + B·t)·e^(-ω₀t)`.
            let decay = (-w0 * t).exp();
            let a = x0;
            let b = v0 + w0 * x0;
            let x = (a + b * t) * decay;
            let v = (b - w0 * (a + b * t)) * decay;
            (x, v)
        } else {
            // Over-damped: two real exponentials.
            let root = w0 * (zeta * zeta - 1.0).sqrt();
            let r1 = -w0 * zeta + root;
            let r2 = -w0 * zeta - root;
            let c1 = (v0 - r2 * x0) / (r1 - r2);
            let c2 = x0 - c1;
            let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
            let x = c1 * e1 + c2 * e2;
            let v = c1 * r1 * e1 + c2 * r2 * e2;
            (x, v)
        };

        if !x.is_finite() || !v.is_finite() {
            // Reachable only for absurd parameters over a long `t` (an `exp` that
            // overflowed). Same posture as above: end the animation on target.
            return SpringSample { value: to, velocity: 0.0, settled: true };
        }
        SpringSample {
            value: to + x,
            velocity: v,
            settled: x.abs() < Self::REST_DISPLACEMENT && v.abs() < Self::REST_VELOCITY,
        }
    }

    /// An **upper bound** on when this spring is at rest: from this time onward,
    /// [`sample`](Spring::sample) is guaranteed to report `settled`, for every one of
    /// the three regimes.
    ///
    /// **A bound rather than the first such moment, and by closed form rather than by
    /// search — both deliberate, and the first draft did neither.** That draft
    /// bisected for the first `t` at which `settled` holds, which quietly assumes the
    /// predicate is monotone in time. It is not: `settled` is a conjunction over
    /// displacement *and* velocity, and a lightly-damped spring satisfies both near
    /// every turning point of its ring long before it has actually stopped ringing.
    /// Bisection over that returns *a* crossing, not the first, and the value it
    /// returns depends on where the search happened to land. `Spring { stiffness:
    /// 400, damping: 4 }` is the case that exposed it — settled at 5.81s by search,
    /// demonstrably un-ringed only much later.
    ///
    /// So this inverts a decay envelope that bounds **both** `|x|` and `|v|`:
    ///
    /// - under-damped: both are `R·e^(-ζω₀t)` with `R` the amplitude of the relevant
    ///   sinusoid pair — exact, since the oscillation is bounded by its own envelope;
    /// - critically damped: both have the form `(P + Q·t)·e^(-ω₀t)`, bounded by
    ///   `(|P| + 2|Q|/(ω₀e))·e^(-ω₀t/2)` using `max_t t·e^(-ω₀t/2) = 2/(ω₀e)`;
    /// - over-damped: both are sums of two decaying exponentials, bounded by their
    ///   coefficient sums times the *slower* root.
    ///
    /// Being an upper bound makes it conservative: the spring is usually visually
    /// still well before this, and [`Timeline`](crate::Timeline) ends the animation
    /// on the first *sampled* `settled`, which is earlier. Nothing depends on the two
    /// agreeing — this one exists so a spring can answer
    /// [`Track::total_duration`](crate::Track::total_duration) at all, and answering
    /// it late costs a held entry, while answering it early would cut a spring off
    /// mid-flight.
    pub fn settle_time(&self, from: f32, to: f32, initial_velocity: f32) -> f32 {
        let x0 = from - to;
        let v0 = initial_velocity;
        if !(self.stiffness > 0.0) || !(self.mass > 0.0) || self.damping < 0.0 {
            return 0.0;
        }
        let w0 = self.omega0();
        let zeta = self.damping_ratio();
        if !(zeta > 0.0) {
            // Undamped: it oscillates forever and there is no honest finite answer.
            return SETTLE_CAP;
        }

        /// `t` such that `c·e^(-rate·t) ≤ eps`.
        fn invert(c: f32, eps: f32, rate: f32) -> f32 {
            if !(rate > 0.0) {
                return SETTLE_CAP;
            }
            if c <= eps {
                0.0
            } else {
                (c / eps).ln() / rate
            }
        }

        let t = if zeta < 1.0 - Self::REGIME_EPSILON {
            let sigma = zeta * w0;
            let wd = w0 * (1.0 - zeta * zeta).sqrt();
            let a = x0;
            let b = (v0 + sigma * x0) / wd;
            let rx = (a * a + b * b).sqrt();
            let (p, q) = (b * wd - sigma * a, a * wd + sigma * b);
            let rv = (p * p + q * q).sqrt();
            invert(rx, Self::REST_DISPLACEMENT, sigma).max(invert(rv, Self::REST_VELOCITY, sigma))
        } else if zeta <= 1.0 + Self::REGIME_EPSILON {
            let a = x0;
            let b = v0 + w0 * x0;
            let bound = |p: f32, q: f32| p.abs() + 2.0 * q.abs() / (w0 * std::f32::consts::E);
            let rate = w0 * 0.5;
            invert(bound(a, b), Self::REST_DISPLACEMENT, rate)
                .max(invert(bound(b - w0 * a, w0 * b), Self::REST_VELOCITY, rate))
        } else {
            let root = w0 * (zeta * zeta - 1.0).sqrt();
            let (r1, r2) = (-w0 * zeta + root, -w0 * zeta - root);
            let c1 = (v0 - r2 * x0) / (r1 - r2);
            let c2 = x0 - c1;
            // `r1` is the slower root, so it dominates the tail and bounds both terms.
            invert(c1.abs() + c2.abs(), Self::REST_DISPLACEMENT, -r1)
                .max(invert((c1 * r1).abs() + (c2 * r2).abs(), Self::REST_VELOCITY, -r1))
        };

        if !t.is_finite() {
            return SETTLE_CAP;
        }
        t.clamp(0.0, SETTLE_CAP)
    }

    /// How close `ζ` must be to 1 before the critically-damped branch is used
    /// instead of the under/over-damped ones. Both neighbouring forms divide by a
    /// quantity that goes to zero as `ζ → 1` (`ωd` on one side, `r1 - r2` on the
    /// other), so this is a numerical guard, not a modelling choice: `Spring::
    /// critical` lands *exactly* on 1 in exact arithmetic and lands next to it in
    /// `f32`.
    const REGIME_EPSILON: f32 = 1e-4;
}

/// Seconds. The answer for a spring with no damping at all, which never settles, and
/// the ceiling on every other answer. Ten seconds is far longer than any UI animation
/// and far shorter than an infinity a caller would have to special-case.
const SETTLE_CAP: f32 = 10.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever the damping, a spring starts where it was put and ends on target.
    #[test]
    fn every_regime_starts_at_from_and_converges_to_to() {
        let springs = [
            Spring { stiffness: 180.0, damping: 12.0, mass: 1.0 },  // under
            Spring::critical(180.0, 1.0),                           // critical
            Spring { stiffness: 180.0, damping: 60.0, mass: 1.0 },  // over
        ];
        for s in springs {
            let at0 = s.sample(0.0, 100.0, 0.0, 0.0);
            assert!((at0.value - 0.0).abs() < 1e-4, "{s:?} at t=0: {}", at0.value);
            assert!(!at0.settled, "{s:?} claimed to be settled before it moved");
            let late = s.sample(0.0, 100.0, 0.0, 10.0);
            assert!((late.value - 100.0).abs() < 1e-2, "{s:?} at t=10: {}", late.value);
            assert!(late.settled, "{s:?} never settled");
        }
    }

    /// The three regimes must actually behave like three regimes. This is the
    /// negative control for the branch selection in `sample`: if the critical branch
    /// were used everywhere (the easiest way to get all the other tests passing) the
    /// under-damped spring would stop overshooting and this fails.
    #[test]
    fn under_damped_overshoots_and_the_other_two_never_do() {
        let target = 100.0;
        let under = Spring { stiffness: 180.0, damping: 12.0, mass: 1.0 };
        let peak = (0..600)
            .map(|i| under.sample(0.0, target, 0.0, i as f32 / 100.0).value)
            .fold(f32::MIN, f32::max);
        assert!(peak > target + 1.0, "under-damped peaked at {peak}, never overshot");

        for s in [Spring::critical(180.0, 1.0), Spring { stiffness: 180.0, damping: 60.0, mass: 1.0 }] {
            let peak = (0..600).map(|i| s.sample(0.0, target, 0.0, i as f32 / 100.0).value).fold(f32::MIN, f32::max);
            assert!(peak <= target + 1e-3, "{s:?} overshot to {peak}");
        }
    }

    /// `damping_ratio` and `critical` must agree, since one is the inverse of the
    /// other. Pinned because `critical` is the shortcut most likely to be "simplified"
    /// into `damping = stiffness / 2` or similar.
    #[test]
    fn critical_damping_has_a_ratio_of_exactly_one() {
        for (k, m) in [(180.0f32, 1.0f32), (10.0, 0.5), (2000.0, 3.0)] {
            let r = Spring::critical(k, m).damping_ratio();
            assert!((r - 1.0).abs() < 1e-5, "k={k} m={m}: ζ={r}");
        }
        assert!(Spring::default().damping_ratio() < 1.0, "the default is meant to overshoot slightly");
    }

    /// Initial velocity is the whole reason this type exists — it is what a retarget
    /// carries across. A spring launched *toward* its target must arrive sooner than
    /// one launched from rest, and one launched away must first travel away.
    #[test]
    fn initial_velocity_changes_the_trajectory_in_the_right_direction() {
        let s = Spring::critical(180.0, 1.0);
        let from_rest = s.sample(0.0, 100.0, 0.0, 0.05);
        let thrown_at = s.sample(0.0, 100.0, 400.0, 0.05);
        assert!(thrown_at.value > from_rest.value, "{} vs {}", thrown_at.value, from_rest.value);
        // Thrown *away* from the target, the spring must first travel the wrong way.
        // Asserted over the whole trajectory rather than at one instant: this spring
        // is stiff enough (ω₀ ≈ 13.4) that it has already turned round and passed
        // zero again by 50ms, so a single late sample reads positive and says nothing.
        let min_of = |v0: f32| {
            (0..200).map(|i| s.sample(0.0, 100.0, v0, i as f32 / 1000.0).value).fold(f32::MAX, f32::min)
        };
        let excursion = min_of(-400.0);
        assert!(excursion < -1.0, "thrown away from the target but never left: min {excursion}");
        // The control: released from rest, this spring never goes below its start at
        // all, so the excursion above is the initial velocity's doing and not a
        // property of the spring.
        assert!(min_of(0.0) >= -1e-4, "the control itself dipped below zero: {}", min_of(0.0));
        // …and all three still converge, which is what makes the above a trajectory
        // difference rather than a broken spring.
        for v0 in [0.0f32, 400.0, -400.0] {
            assert!((s.sample(0.0, 100.0, v0, 10.0).value - 100.0).abs() < 1e-2);
        }
    }

    /// Reported velocity must be the derivative of reported position. Checked against
    /// a central finite difference — an independent method, so this catches a sign
    /// slip or a dropped term in any of the three analytic derivatives. (The finite
    /// difference is the test's tool, not the implementation's: `sample` returns the
    /// analytic form precisely so velocity is frame-rate independent.)
    #[test]
    fn reported_velocity_is_the_derivative_of_reported_position() {
        let h = 1e-4f32;
        for s in [
            Spring { stiffness: 180.0, damping: 12.0, mass: 1.0 },
            Spring::critical(180.0, 1.0),
            Spring { stiffness: 180.0, damping: 60.0, mass: 1.0 },
        ] {
            for t in [0.01f32, 0.05, 0.2, 0.5] {
                let analytic = s.sample(0.0, 100.0, 250.0, t).velocity;
                let numeric = (s.sample(0.0, 100.0, 250.0, t + h).value
                    - s.sample(0.0, 100.0, 250.0, t - h).value)
                    / (2.0 * h);
                assert!(
                    (analytic - numeric).abs() < 0.5 + numeric.abs() * 1e-2,
                    "{s:?} at t={t}: analytic {analytic} vs numeric {numeric}"
                );
            }
        }
    }

    /// `settled` needs both bounds. A spring passing through its target at speed has
    /// zero displacement and is very much still animating; a displacement-only test
    /// calls it done, the timeline stops asking for frames, and the animation freezes
    /// mid-flight at exactly the target. This is that test — it fails if the velocity
    /// term is dropped from the conjunction.
    #[test]
    fn a_spring_crossing_its_target_at_speed_is_not_settled() {
        let ringy = Spring { stiffness: 400.0, damping: 4.0, mass: 1.0 };
        // Detected by a *sign change* of the displacement rather than by sampling
        // within `REST_DISPLACEMENT` of the target: this spring crosses at roughly
        // 100 units/second, so it spends about ten microseconds inside a
        // thousandth-of-a-unit window and a fixed grid walks straight past it. The
        // first draft of this test did exactly that and reported "not discriminating",
        // which is the failure mode a control is supposed to have.
        let mut crossings = 0;
        let mut prev = ringy.sample(0.0, 100.0, 0.0, 0.0);
        for i in 1..4000 {
            let t = i as f32 / 2000.0;
            let s = ringy.sample(0.0, 100.0, 0.0, t);
            if (prev.value - 100.0).signum() != (s.value - 100.0).signum() {
                crossings += 1;
                // Straddling the target at speed. Neither side may be called settled,
                // because `Timeline` ends the animation on that flag and the value it
                // would freeze at is the target — i.e. the animation would silently
                // stop early, mid-ring, at a plausible-looking place.
                assert!(!prev.settled && !s.settled, "settled while crossing at t={t}: {prev:?} {s:?}");
                assert!(s.velocity.abs() > Spring::REST_VELOCITY, "t={t}: {s:?}");
            }
            prev = s;
        }
        assert!(crossings >= 3, "only {crossings} crossings — the probe is not discriminating");
    }

    /// `settle_time`'s actual contract: from that moment **onwards** the spring is
    /// settled, in every regime. Checked densely rather than at a handful of probes,
    /// because the failure it exists to prevent is a lightly-damped spring that dips
    /// under both thresholds near a turning point and rings again afterwards — which
    /// a sparse check walks straight past. `stiffness: 400, damping: 4` (ζ = 0.1) is
    /// the case that failed against the bisecting first draft.
    #[test]
    fn a_spring_is_settled_from_its_settle_time_onwards_in_every_regime() {
        for s in [
            Spring::default(),
            Spring { stiffness: 180.0, damping: 12.0, mass: 1.0 },
            Spring::critical(180.0, 1.0),
            Spring { stiffness: 180.0, damping: 60.0, mass: 1.0 },
            Spring { stiffness: 400.0, damping: 4.0, mass: 1.0 },
        ] {
            for (from, to, v0) in [(0.0f32, 100.0f32, 0.0f32), (320.0, 0.0, -900.0), (0.0, 1.0, 4.0)] {
                let ts = s.settle_time(from, to, v0);
                assert!(ts > 0.0 && ts < SETTLE_CAP, "{s:?} settle_time = {ts}");
                for i in 0..=400 {
                    let t = ts + i as f32 * 0.02;
                    let out = s.sample(from, to, v0, t);
                    assert!(out.settled, "{s:?} un-settled at {t} (settle_time {ts}): {out:?}");
                }
            }
        }
    }

    /// …and the bound is not vacuous. A bound is trivially satisfiable by returning
    /// the cap, so this asserts the other direction: the spring is genuinely still
    /// moving at a fraction of the reported time. Without this, `settle_time` could
    /// be `SETTLE_CAP` for everything and the test above would still pass.
    #[test]
    fn settle_time_is_a_tight_enough_bound_to_be_worth_reporting() {
        for s in [Spring::default(), Spring::critical(180.0, 1.0), Spring { stiffness: 180.0, damping: 60.0, mass: 1.0 }] {
            let ts = s.settle_time(0.0, 100.0, 0.0);
            assert!(ts < SETTLE_CAP * 0.5, "{s:?} claims {ts}s — that is the cap, not a bound");
            assert!(!s.sample(0.0, 100.0, 0.0, ts * 0.25).settled, "{s:?} was already done at a quarter of {ts}");
        }
        // **The heavily over-damped tail is real, and this records its size rather
        // than hiding it behind a loose bound.** `ζ = 2.24` puts the slow root at
        // -3.17, a 0.32s time constant, so travelling 100 units and then getting
        // within `REST_DISPLACEMENT` of the target takes ~9 time constants — 3.7s,
        // during which the last ~1% of the motion is invisible and the render thread
        // is still painting. That is a property of the spring the author chose, not
        // of the threshold: `Spring::critical` covers the same 100 units in 1.8s and
        // the default in 1.1s. Filed in ../PLAN.md §45's known gaps as the argument
        // for a per-channel rest threshold.
        let sluggish = Spring { stiffness: 180.0, damping: 60.0, mass: 1.0 };
        let ts = sluggish.settle_time(0.0, 100.0, 0.0);
        assert!((3.0..4.5).contains(&ts), "the over-damped tail moved: {ts}");
        // …and half that time already covers 99.7% of the travel, which is the
        // measurement behind calling the rest of it invisible.
        let half_way_through = sluggish.sample(0.0, 100.0, 0.0, ts * 0.5).value;
        assert!(half_way_through > 99.5, "at half the settle time it is only at {half_way_through}");
        // An undamped spring is the one honest `SETTLE_CAP`: it rings forever, and
        // reporting a finite settle would be a lie rather than a conservative bound.
        assert_eq!(Spring { stiffness: 180.0, damping: 0.0, mass: 1.0 }.settle_time(0.0, 100.0, 0.0), SETTLE_CAP);
    }

    /// Degenerate parameters must not produce a NaN. A NaN value here would reach a
    /// `ComputedStyle` override and from there a taffy layout, where it does not
    /// crash — it silently produces a zero-sized or absent box, which is the hardest
    /// class of bug this framework has (§15.1's "renders, nothing errors, behaviour
    /// absent"). Every branch degrades to "already on target" instead.
    #[test]
    fn absurd_parameters_degrade_to_on_target_rather_than_to_nan() {
        let cases = [
            Spring { stiffness: 0.0, damping: 10.0, mass: 1.0 },
            Spring { stiffness: -5.0, damping: 10.0, mass: 1.0 },
            Spring { stiffness: 180.0, damping: 10.0, mass: 0.0 },
            Spring { stiffness: 180.0, damping: -1.0, mass: 1.0 },
        ];
        for s in cases {
            for t in [0.0f32, 0.016, 1.0, 100.0] {
                let out = s.sample(0.0, 42.0, 0.0, t);
                assert!(out.value.is_finite() && out.velocity.is_finite(), "{s:?} at {t} -> {out:?}");
                assert_eq!(out.value, 42.0, "{s:?} at {t}");
            }
            assert_eq!(s.settle_time(0.0, 42.0, 0.0), 0.0);
        }
        // A negative time is a caller bug, not a spring parameter, and gets the same
        // treatment rather than an extrapolation backwards through the exponential.
        assert_eq!(Spring::default().sample(0.0, 42.0, 0.0, -1.0).value, 42.0);
    }

    /// `Spring::critical` lands next to ζ=1 in `f32`, not on it, so the regime guard
    /// has to be a band rather than an equality — otherwise `critical` takes the
    /// under- or over-damped branch and divides by something near zero. This walks a
    /// tight band around critical and asserts the output stays finite and monotone,
    /// which is what that division failing would break.
    #[test]
    fn the_regime_boundary_is_a_band_and_nothing_blows_up_inside_it() {
        let k = 180.0f32;
        let m = 1.0f32;
        let c_crit = 2.0 * (k * m).sqrt();
        for delta in [-1e-3f32, -1e-5, -1e-7, 0.0, 1e-7, 1e-5, 1e-3] {
            let s = Spring { stiffness: k, damping: c_crit + delta, mass: m };
            let mut prev = f32::MIN;
            for i in 0..300 {
                let out = s.sample(0.0, 100.0, 0.0, i as f32 / 300.0);
                assert!(out.value.is_finite(), "delta={delta} t={i}: {out:?}");
                assert!(out.value >= prev - 1e-2, "delta={delta}: non-monotonic {prev} -> {}", out.value);
                prev = out.value;
            }
        }
    }
}
