//! The render loop's time-domain arithmetic — fling decay, and where a frame landed
//! against the display's refresh budget.
//!
//! **Why this is a crate and not two functions in `gpu_harness`.** `gpu_harness` is a
//! cdylib that cross-compiles to Android and does not build for the host at all
//! (../PLAN.md §15.5), so nothing in it can carry a unit test. §41 shipped
//! `fling_tau` — the whole of that section's change — with no test, and recorded the
//! absence in its own known-gaps list along with the observation that moving it
//! somewhere host-testable would be cheap. §42 needed a second function of exactly
//! the same shape (`frame_periods`), which made it two, which makes this the §35
//! move: `zkite_imagestore` exists for the identical reason.
//!
//! Everything here is a pure function of its arguments. No platform, no state, no
//! dependencies — if something in this crate needs a device to run, it is in the
//! wrong crate.

use std::time::Duration;

/// Exponential-decay time constant, in seconds, **at `FLING_REFERENCE_VELOCITY`**:
/// velocity at time `t` after release is `v0 * exp(-t/tau)` and total remaining travel
/// is the exact integral of that, `v0 * tau`. A typical fast flick (~3000dp/s) travels
/// roughly 3000*0.325 ≈ 975dp, decaying below the stop threshold in a bit under a
/// second — chosen to feel right, not derived from a platform constant.
pub const FLING_TAU_AT_REFERENCE: f32 = 0.325;

/// The release speed (dp/s) at which `fling_tau` returns `FLING_TAU_AT_REFERENCE`
/// exactly. Picked as a "typical fast flick" so the calibration the renderer already
/// had is the fixed point of the adaptive curve — ../PLAN.md §41 changed the *shape*
/// of the momentum response without moving the one point on it that had been tuned.
pub const FLING_REFERENCE_VELOCITY: f32 = 3000.0;

/// How strongly the time constant grows with release speed. Fling *distance* is
/// `v0 * tau(v0)`, so distance scales as `v0^(1 + FLING_TAU_EXPONENT)` — see
/// `fling_tau` for why this specific number.
pub const FLING_TAU_EXPONENT: f32 = 0.74;

/// Clamps on the adaptive time constant. The floor keeps a barely-past-threshold
/// release from gliding an invisible distance; the ceiling bounds how long a very hard
/// flick can coast, since exponential decay has no natural end and a 3-second fling
/// reads as "the list ran away" rather than as momentum.
pub const FLING_TAU_MIN: f32 = 0.10;
pub const FLING_TAU_MAX: f32 = 0.55;

/// The decay time constant for a fling released at `v0` dp/s.
///
/// **Why this is not a constant** (../PLAN.md §41). With a fixed `tau`, total travel is
/// `v0 * tau` — *exactly linear* in release speed. Every flick then has the same
/// "gearing": a flick twice as fast goes precisely twice as far, at every speed. That
/// is the flat response the owner reported, and it is wrong at both ends at once —
/// a gentle flick coasts too far to place precisely, and a hard flick refuses to cover
/// a long list no matter how hard it is thrown.
///
/// Real scrollers are superlinear. Android's `OverScroller` fits a spline whose
/// distance goes as `v^(d/(d-1))` with `d = ln(0.78)/ln(0.9) ≈ 2.358`, i.e. `v^1.736`.
/// Scaling `tau` by `(v0/ref)^0.74` makes distance go as `v0^1.74` and reproduces that
/// curve to within ~1% over 600–6000 dp/s (checked numerically against the spline
/// exponent before this was written, not fitted by feel):
///
/// | speed ratio | this | `OverScroller` | fixed `tau` |
/// |---|---|---|---|
/// | 600 → 1200 (2x) | 3.30x | 3.33x | 2.00x |
/// | 1000 → 3000 (3x) | 6.76x | 6.74x | 3.00x |
/// | 1500 → 6000 (4x) | 11.16x | 11.10x | 4.00x |
///
/// Outside `[610, 6108]` dp/s the clamps bite and the response goes linear again,
/// deliberately: below that a fling is a nudge, above it the duration matters more
/// than the curve.
///
/// `tau` is resolved **once, at release**, and stored on the `Fling` — not recomputed
/// per tick from the current velocity. Recomputing would make `tau` shrink as the
/// fling slows, which is no longer exponential decay, and `tick_fling`'s exact-integral
/// property (correct travel regardless of when a tick actually lands) depends on `tau`
/// being constant for the animation's lifetime.
pub fn fling_tau(v0: f32) -> f32 {
    let ratio = v0.abs() / FLING_REFERENCE_VELOCITY;
    (FLING_TAU_AT_REFERENCE * ratio.powf(FLING_TAU_EXPONENT)).clamp(FLING_TAU_MIN, FLING_TAU_MAX)
}

/// How many display refresh periods a frame actually occupied. `1` is on time; `2`
/// means exactly one frame was dropped.
///
/// The counterpart to ../PLAN.md §40.4's `scroll:` line, one level down: §39 added a
/// worst-case because a mean of sixty frames cannot see one hitch, and `avg 17.48ms
/// max 39.41ms` still cannot distinguish *"one frame in twenty takes two refreshes"*
/// from *"most frames run slightly over"*. Those two have different fixes, so the
/// histogram is the measurement rather than a nicer presentation of one.
///
/// **Rounded to the nearest period, not floored with a tolerance.** The interval this
/// is handed is measured between the ends of two `redraw()`s rather than between two
/// vsyncs, so it carries the render thread's own scheduling jitter on both sides; the
/// question is "how many refreshes did this frame occupy", and the nearest whole
/// number is the answer to it. A tolerance-and-floor puts a 30ms interval — which
/// cannot be a single 16.67ms refresh under any amount of jitter — in the on-time
/// bucket, i.e. it errs toward reporting no problem, which is the one direction an
/// instrument built to find dropped frames must not lean. That form was written
/// first and `thirty_ms_is_two_refreshes_not_one` is what rejected it.
///
/// Never returns 0: a skipped frame (`PaintOutcome::Skip`) can end two redraws less
/// than a period apart, and arriving early is not *better* than on time. A
/// non-positive `frame_period` is a device that lied about its refresh rate, and
/// answering `1` degrades to "no drops detected" rather than to a divide-by-zero.
pub fn frame_periods(interval: Duration, frame_period: Duration) -> u32 {
    let period = frame_period.as_secs_f32();
    if !(period > 0.0) {
        return 1;
    }
    let periods = (interval.as_secs_f32() / period).round();
    if periods < 1.0 {
        1
    } else {
        periods as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distance travelled by a fling released at `v0`, under this crate's curve.
    fn travel(v0: f32) -> f32 {
        v0 * fling_tau(v0)
    }

    /// `OverScroller`'s own spline exponent: `d = ln(0.78)/ln(0.9)`, distance goes as
    /// `v^(d/(d-1))`. Written out from the derivation rather than as the rounded
    /// 1.736, so the test is checking against Android's constant and not against
    /// §41's arithmetic about it.
    fn overscroller_travel(v0: f32) -> f32 {
        let d = (0.78f32).ln() / (0.9f32).ln();
        v0.powf(d / (d - 1.0))
    }

    #[test]
    fn the_reference_velocity_is_the_fixed_point() {
        // ../PLAN.md §41's central claim about compatibility: at 3000dp/s the
        // adaptive curve returns exactly the hand-calibrated constant that existed
        // before it, so the one point on this curve anybody had ever tuned by feel
        // did not move. If this fails, §41 changed more than the shape.
        assert!((fling_tau(FLING_REFERENCE_VELOCITY) - FLING_TAU_AT_REFERENCE).abs() < 1e-6);
        assert!((travel(3000.0) - 975.0).abs() < 1.0);
    }

    #[test]
    fn distance_reproduces_the_overscroller_exponent() {
        // The whole point of §41: distance must be superlinear in release speed and
        // must match Android's spline, not merely be "more than linear". Each row is
        // a ratio, so the arbitrary scale of `overscroller_travel` cancels.
        for (slow, fast) in [(600.0f32, 1200.0f32), (1000.0, 3000.0), (1500.0, 6000.0)] {
            let ours = travel(fast) / travel(slow);
            let theirs = overscroller_travel(fast) / overscroller_travel(slow);
            assert!(
                (ours / theirs - 1.0).abs() < 0.015,
                "v {slow}->{fast}: ours {ours:.3}x vs OverScroller {theirs:.3}x"
            );
            // …and the fixed-tau behaviour §41 replaced is the linear ratio, which
            // this must *not* be. Without this the test above would pass for a
            // curve that was still flat at low speeds, since 2x is within 15% of
            // nothing in particular — see `..._is_not_linear` for the sharp form.
            let linear = fast / slow;
            assert!(ours > linear * 1.5, "v {slow}->{fast}: {ours:.3}x is barely superlinear");
        }
    }

    #[test]
    fn a_harder_flick_gains_more_than_proportionally() {
        // The negative control for the whole section, stated as the property the old
        // code had: travel(2v) == 2 * travel(v), exactly, at every v. It must now be
        // false everywhere inside the clamps.
        for v in [700.0f32, 1000.0, 1500.0, 2000.0, 3000.0] {
            let doubled = travel(v * 2.0);
            assert!(
                doubled > 2.0 * travel(v) * 1.2,
                "travel({}) = {:.0} is too close to twice travel({v}) = {:.0}",
                v * 2.0,
                doubled,
                travel(v)
            );
        }
    }

    #[test]
    fn the_clamps_bite_and_restore_linearity_at_the_extremes() {
        // Below the floor and above the ceiling `tau` is constant again, which makes
        // travel linear again — deliberately (§41.2). Pinned because the clamp
        // values are the two numbers most likely to be "tidied" later.
        assert_eq!(fling_tau(100.0), FLING_TAU_MIN);
        assert_eq!(fling_tau(400.0), FLING_TAU_MIN);
        assert_eq!(fling_tau(9000.0), FLING_TAU_MAX);
        assert_eq!(fling_tau(20000.0), FLING_TAU_MAX);
        // Two nudges an octave apart, both under the floor: exactly 2x, i.e. linear.
        assert!((travel(400.0) / travel(200.0) - 2.0).abs() < 1e-4);
    }

    #[test]
    fn tau_is_symmetric_in_direction() {
        // A fling up a list and a fling down it decay identically; `fling_tau` takes
        // a signed velocity and only the magnitude may matter.
        for v in [80.0f32, 750.0, 3000.0, 5100.0] {
            assert_eq!(fling_tau(v), fling_tau(-v));
        }
    }

    #[test]
    fn tau_never_leaves_its_clamps_for_any_velocity() {
        // Including the degenerate inputs a real velocity estimator can produce: 0
        // (a stationary release) and a NaN/inf out of a division by a near-zero dt.
        // `powf` is happy to return NaN, and `clamp` panics on a NaN bound but not a
        // NaN value — it returns NaN, which would then silently become a fling that
        // never ends. This is the one input worth being explicit about.
        for v in [0.0f32, 1.0, 12345.0, f32::MAX] {
            let tau = fling_tau(v);
            assert!((FLING_TAU_MIN..=FLING_TAU_MAX).contains(&tau), "v={v} -> tau={tau}");
        }
        assert!(fling_tau(f32::NAN).is_nan(), "documented: a NaN velocity is the caller's to reject");
    }

    const HZ60: Duration = Duration::from_nanos(16_666_667);

    #[test]
    fn an_on_time_frame_occupies_one_refresh() {
        for ms in [0.1f32, 8.0, 14.0, 16.7, 18.0, 20.0] {
            let d = Duration::from_secs_f32(ms / 1000.0);
            assert_eq!(frame_periods(d, HZ60), 1, "{ms}ms");
        }
    }

    #[test]
    fn thirty_ms_is_two_refreshes_not_one() {
        // The test that rejected the first implementation. `(interval + 2.5ms) /
        // period`, floored, classifies 30ms as 1.95 periods → "on time"; 30ms cannot
        // be a single 16.67ms refresh under any amount of jitter, and an instrument
        // for finding dropped frames must not round that away. Run against the
        // tolerance-and-floor form, this asserts 2 and gets 1.
        assert_eq!(frame_periods(Duration::from_millis(30), HZ60), 2);
        assert_eq!(frame_periods(Duration::from_millis(25), HZ60), 2);
        assert_eq!(frame_periods(Duration::from_micros(33_333), HZ60), 2);
        assert_eq!(frame_periods(Duration::from_millis(40), HZ60), 2);
    }

    #[test]
    fn a_long_stall_counts_every_refresh_it_covered() {
        assert_eq!(frame_periods(Duration::from_millis(45), HZ60), 3);
        assert_eq!(frame_periods(Duration::from_millis(50), HZ60), 3);
        assert_eq!(frame_periods(Duration::from_millis(100), HZ60), 6);
    }

    #[test]
    fn the_period_comes_from_the_panel_not_from_sixty_hz() {
        // §13.2's other half: the frame period is read from `Display.getRefreshRate()`
        // because a hardcoded 16ms under-drives a 120Hz panel by half. The bucketing
        // has to follow, or every frame on a 120Hz device reads as on time.
        const HZ120: Duration = Duration::from_nanos(8_333_333);
        assert_eq!(frame_periods(Duration::from_micros(8_300), HZ120), 1);
        assert_eq!(frame_periods(Duration::from_micros(16_667), HZ120), 2);
        // The same 16.67ms interval is on time at 60Hz and a dropped frame at 120Hz.
        assert_eq!(frame_periods(Duration::from_micros(16_667), HZ60), 1);
    }

    #[test]
    fn a_frame_can_never_occupy_less_than_one_refresh() {
        // `PaintOutcome::Skip` can end two redraws a fraction of a period apart.
        // Zero would make the on-time bucket unreachable by division and would let a
        // window of skipped frames read as better than perfect.
        assert_eq!(frame_periods(Duration::ZERO, HZ60), 1);
        assert_eq!(frame_periods(Duration::from_micros(500), HZ60), 1);
        // …and a device that reported a nonsense refresh rate degrades to "no drops
        // detected" rather than dividing by zero.
        assert_eq!(frame_periods(Duration::from_millis(30), Duration::ZERO), 1);
    }
}

/// How long the render loop may wait for its next command, or `None` to block
/// indefinitely — ../PLAN.md §47.
///
/// This generalises what used to be "is a fling running" into "does anything need the
/// next frame", now that an animation driver is a second source of self-paced frames.
/// It lives here rather than in `gpu_harness` for this crate's whole reason for
/// existing: it is pure arithmetic over three arguments, and `gpu_harness` does not
/// build for the host (../PLAN.md §15.5), so a copy of it there could only ever be
/// checked on a device.
///
/// - `has_fling` — a momentum scroll is in flight.
/// - `anim_deadline` — `zkite_anim::Timeline::next_deadline`'s answer. `Some(ZERO)`
///   means *"something is running, a frame is wanted"*; a longer `Some` means
///   everything unfinished is still inside its delay and this is how long until the
///   earliest one starts; `None` means nothing is unfinished.
/// - `frame_due` — how long until the next frame is due, measured from the **start**
///   of the last one (../PLAN.md §13.2). Already zero when the loop is behind.
///
/// # The mistake this exists to prevent
///
/// The obvious implementation is `min(frame_due, anim_deadline)`. That spins the
/// render thread at 100% CPU for the entire length of every animation, because a
/// running timeline reports `ZERO` and `min(anything, 0)` is `0` — the loop would wake
/// instantly, paint, and wake instantly again with no pacing at all.
/// `Timeline::next_deadline`'s own doc says why it reports it that way: pacing is the
/// render loop's business, and a timeline that returned a frame period would be a
/// second, disagreeing opinion about when a frame is due. So `ZERO` selects
/// `frame_due`; it is not combined with it.
///
/// The delayed case is the other half, and it is the reason this is not just a
/// boolean: a 300ms-delayed toast must cost **one** wakeup, not eighteen that paint
/// nothing.
pub fn next_wake(has_fling: bool, anim_deadline: Option<Duration>, frame_due: Duration) -> Option<Duration> {
    let animating = matches!(anim_deadline, Some(d) if d.is_zero());
    match (has_fling || animating, anim_deadline) {
        (true, _) => Some(frame_due),
        (false, Some(delay)) => Some(delay),
        (false, None) => None,
    }
}

#[cfg(test)]
mod wake_tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;
    const FRAME_DUE: Duration = Duration::from_micros(11_000);

    /// Nothing self-paced: the loop blocks, exactly as it did before an animation
    /// driver existed. This is the state a shipped app is in almost all the time, and
    /// getting it wrong means a thread that never sleeps.
    #[test]
    fn an_idle_loop_blocks_rather_than_polling() {
        assert_eq!(next_wake(false, None, FRAME_DUE), None);
    }

    /// **The one that rejects `min(frame_due, anim_deadline)`.** A running timeline
    /// reports `ZERO`, and a literal `min` would return `ZERO` — a busy-wait for the
    /// whole length of every animation. The wait must be the frame pacing.
    #[test]
    fn a_running_animation_is_paced_by_the_frame_and_never_spins() {
        assert_eq!(next_wake(false, Some(Duration::ZERO), FRAME_DUE), Some(FRAME_DUE));
        // The control that makes the above a measurement rather than a coincidence:
        // a naive `min` returns ZERO here and this asserts it does not.
        assert_ne!(next_wake(false, Some(Duration::ZERO), FRAME_DUE), Some(Duration::ZERO));
        // …unless the loop is genuinely behind, where zero means "render now, we are
        // late" and the swapchain's own Fifo present is what actually blocks (§13.2).
        assert_eq!(next_wake(false, Some(Duration::ZERO), Duration::ZERO), Some(Duration::ZERO));
    }

    /// A purely delayed timeline sleeps the real delay. Eighteen wakeups that paint
    /// nothing is precisely what `Timeline::next_deadline` exists to avoid, and
    /// returning `frame_due` here would reintroduce every one of them.
    #[test]
    fn a_delayed_animation_sleeps_the_delay_not_a_frame() {
        assert_eq!(next_wake(false, Some(MS(300)), FRAME_DUE), Some(MS(300)));
        assert!(next_wake(false, Some(MS(300)), FRAME_DUE).unwrap() > FRAME_DUE);
    }

    /// A fling paces frames whatever the animator says — including while a *delayed*
    /// animation is pending, which is the case a `(false, Some(delay))` arm reached
    /// first would get wrong by sleeping 300ms through a live momentum scroll.
    #[test]
    fn a_fling_paces_frames_even_while_an_animation_is_still_delayed() {
        assert_eq!(next_wake(true, None, FRAME_DUE), Some(FRAME_DUE));
        assert_eq!(next_wake(true, Some(MS(300)), FRAME_DUE), Some(FRAME_DUE));
        assert_eq!(next_wake(true, Some(Duration::ZERO), FRAME_DUE), Some(FRAME_DUE));
    }

    /// A wait is never longer than the frame period while anything is moving — the
    /// property that says an animation cannot be starved by this function however the
    /// three inputs combine.
    #[test]
    fn nothing_moving_ever_waits_longer_than_the_frame_it_is_waiting_for() {
        for fling in [true, false] {
            for anim in [None, Some(Duration::ZERO), Some(MS(1)), Some(MS(300))] {
                let moving = fling || anim == Some(Duration::ZERO);
                if let Some(w) = next_wake(fling, anim, FRAME_DUE) {
                    if moving {
                        assert!(w <= FRAME_DUE, "fling={fling} anim={anim:?} waited {w:?}");
                    }
                }
            }
        }
    }
}

/// Density-independent Android-style spline fling. Sampling absolute elapsed time
/// makes the trajectory independent of refresh rate and dropped frames.
#[derive(Clone, Copy, Debug)]
pub struct SplineFling { pub duration: f32, pub distance: f32 }
impl SplineFling {
    pub fn new(velocity: f32) -> Self {
        let velocity = if velocity.is_finite() { velocity.clamp(-12000.0, 12000.0) } else { 0.0 };
        if velocity.abs() < 1.0 { return Self { duration: 0.0, distance: 0.0 }; }
        let rate = 0.78f32.ln() / 0.9f32.ln();
        let friction = 0.015 * 9.80665 * 39.37 * 160.0 * 0.84;
        let deceleration = (0.35 * velocity.abs() / friction).ln();
        Self {
            duration: (deceleration / (rate - 1.0)).exp(),
            distance: velocity.signum() * friction * (rate * deceleration / (rate - 1.0)).exp(),
        }
    }
    pub fn position(self, elapsed: f32) -> f32 {
        if self.duration <= 0.0 { return self.distance; }
        let t = (elapsed / self.duration).clamp(0.0, 1.0);
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..18 {
            let u = (lo + hi) * 0.5;
            let x = 3.0 * u * (1.0-u) * ((1.0-u)*0.175 + u*0.35) + u*u*u;
            if x < t { lo = u; } else { hi = u; }
        }
        let u = (lo + hi) * 0.5;
        self.distance * (3.0*u*(1.0-u)*((1.0-u)*0.5+u)+u*u*u)
    }
}

/// Least-squares speed from timestamped finger samples (seconds, dp).
/// Duplicate samples at release correctly turn a held finger into zero velocity.
pub fn release_velocity(samples: &[(f32, f32)]) -> f32 {
    let Some(&(last, _)) = samples.last() else { return 0.0 };
    let samples: Vec<_> = samples.iter().filter(|(t, _)| last - t <= 0.1).collect();
    if samples.len() < 2 { return 0.0; }
    let n = samples.len() as f32;
    let t = samples.iter().map(|p| p.0 - last).sum::<f32>() / n;
    let x = samples.iter().map(|p| p.1).sum::<f32>() / n;
    let variance = samples.iter().map(|p| (p.0-last-t).powi(2)).sum::<f32>();
    if variance < 0.000032 { return 0.0; }
    (-samples.iter().map(|p| (p.0-last-t)*(p.1-x)).sum::<f32>() / variance).clamp(-12000.0, 12000.0)
}

#[cfg(test)]
mod spline_tests {
    use super::*;
    #[test]
    fn spline_is_monotonic_and_finishes_at_every_refresh_rate() {
        for hz in [60.0, 90.0, 120.0] {
            let fling = SplineFling::new(3000.0);
            let mut previous = 0.0;
            for frame in 0..(fling.duration*hz).ceil() as usize + 2 {
                let current = fling.position(frame as f32/hz);
                assert!(current >= previous && current <= fling.distance + 0.01);
                previous = current;
            }
            assert!((previous-fling.distance).abs() < 0.01);
        }
    }
    #[test]
    fn velocity_handles_pause_noise_and_direction() {
        assert_eq!(release_velocity(&[(0.0, 100.0), (0.2, 0.0)]), 0.0);
        assert!((release_velocity(&[(0.0, 100.0), (0.02, 80.0), (0.04, 60.0)])-1000.0).abs()<1.0);
        assert!(SplineFling::new(6000.0).distance > SplineFling::new(3000.0).distance * 3.0);
        assert_eq!(SplineFling::new(f32::NAN).distance, 0.0);
    }
}
