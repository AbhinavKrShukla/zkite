//! One value, moving once — with a delay in front of it, a repeat count and a
//! direction.
//!
//! A `Track` is a pure function of elapsed time. It holds no clock and no "current"
//! anything: [`sample`](Track::sample) is called with how long the track has been
//! alive and answers where the value is. All the mutable bookkeeping lives one level
//! up in [`Timeline`](crate::Timeline), which is what makes this type testable by
//! writing down a time and reading off a number.

use crate::easing::Easing;
use crate::lerp::Lerp;
use crate::spring::Spring;
use std::time::Duration;

/// How a value gets from `from` to `to`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Curve {
    /// Fixed duration, shaped by a timing function. Predictable, and what a designer
    /// hands you ("200ms ease-out").
    Eased(Easing),
    /// Physical. The duration is an *output* rather than an input, and an interruption
    /// carries real momentum across — see [`Spring`].
    Spring(Spring),
}

impl Default for Curve {
    fn default() -> Self {
        Curve::Eased(Easing::EASE_OUT)
    }
}

/// How many times the active interval runs. **Eased tracks only** — a spring's
/// duration is derived from its own settling, so repeating one is not a thing this
/// crate can express and [`Track::sample`] ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Once,
    /// `Times(0)` and `Times(1)` both mean once — zero iterations would be an
    /// animation that finishes before it starts, and the caller who wrote it meant
    /// "no repeats".
    Times(u32),
    /// Never finishes, so it never reports a completion to Python and never lets the
    /// render thread go idle. Correct for a spinner and wrong for anything else.
    Forever,
}

/// Which way round each iteration runs. **Eased tracks only**, for the same reason as
/// [`Repeat`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Normal,
    Reverse,
    /// Forwards, then backwards, then forwards…
    Alternate,
    /// Backwards, then forwards, then backwards…
    AlternateReverse,
}

/// Where a track is in its own life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Still inside `delay`. The value is held at the start, which is the same
    /// decision CSS's `animation-fill-mode: backwards` makes and the only one that
    /// avoids a visible pop when the delay ends: a sheet declared to slide up from
    /// 320dp must sit at 320dp during its delay, not at 0.
    Delayed,
    Running,
    /// Reached the end and stayed there. The value continues to read as the final
    /// value for as long as the track is sampled — see [`Timeline`](crate::Timeline)
    /// on why finished tracks are held rather than dropped.
    Finished,
}

/// A sampled track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub value: f32,
    /// Units per second. Exact for a spring, a finite difference for an eased track —
    /// see [`Track::sample`]. Consumed by [`Interrupt::Retarget`](crate::Interrupt).
    pub velocity: f32,
    pub status: Status,
}

/// One channel's motion. Build with [`Track::eased`] or [`Track::spring`] and refine
/// with the `with_*` methods.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Track {
    pub from: f32,
    pub to: f32,
    pub delay: Duration,
    /// Ignored for [`Curve::Spring`], whose duration is a consequence of its physics.
    pub duration: Duration,
    pub curve: Curve,
    pub repeat: Repeat,
    pub direction: Direction,
    /// Springs only: the velocity the value already had when this track took over.
    pub initial_velocity: f32,
}

impl Track {
    pub fn eased(from: f32, to: f32, duration: Duration, easing: Easing) -> Track {
        Track {
            from,
            to,
            delay: Duration::ZERO,
            duration,
            curve: Curve::Eased(easing),
            repeat: Repeat::Once,
            direction: Direction::Normal,
            initial_velocity: 0.0,
        }
    }

    pub fn spring(from: f32, to: f32, spring: Spring) -> Track {
        Track {
            from,
            to,
            delay: Duration::ZERO,
            duration: Duration::ZERO,
            curve: Curve::Spring(spring),
            repeat: Repeat::Once,
            direction: Direction::Normal,
            initial_velocity: 0.0,
        }
    }

    pub fn with_delay(mut self, delay: Duration) -> Track {
        self.delay = delay;
        self
    }

    pub fn with_repeat(mut self, repeat: Repeat) -> Track {
        self.repeat = repeat;
        self
    }

    pub fn with_direction(mut self, direction: Direction) -> Track {
        self.direction = direction;
        self
    }

    pub fn with_initial_velocity(mut self, v: f32) -> Track {
        self.initial_velocity = v;
        self
    }

    /// Where this track is `elapsed` after it started — including its delay.
    ///
    /// **Velocity for an eased track is a central finite difference, and the analytic
    /// form was rejected rather than not considered.** The exact derivative is
    /// `(to - from) · By'(s)/Bx'(s) / duration`, which is correct except where
    /// `Bx'(s)` is zero — and that is not an exotic input, it is `ease-out` at `s = 0`
    /// and `ease-in` at `s = 1`, i.e. the start of the most common entrance curve in
    /// the framework. A singularity at the exact moment a retarget is most likely
    /// (the user grabbing something that just started moving) is worse than a
    /// difference that is a few parts in ten thousand off everywhere.
    pub fn sample(&self, elapsed: Duration) -> Sample {
        if elapsed < self.delay {
            return Sample { value: self.from, velocity: 0.0, status: Status::Delayed };
        }
        let t = (elapsed - self.delay).as_secs_f32();

        if let Curve::Spring(spring) = self.curve {
            let s = spring.sample(self.from, self.to, self.initial_velocity, t);
            // A settled spring reports its *target*, not the within-a-thousandth
            // value the exponential actually has there. The difference is invisible
            // on screen and load-bearing off it: `Timeline` ends an animation on this
            // status, and the last frame it paints has to be the final state exactly
            // — otherwise every spring in the app comes to rest a fraction of a unit
            // off and stays there until something unrelated redraws.
            return if s.settled {
                Sample { value: self.to, velocity: 0.0, status: Status::Finished }
            } else {
                Sample { value: s.value, velocity: s.velocity, status: Status::Running }
            };
        }

        let Curve::Eased(easing) = self.curve else { unreachable!() };
        let dur = self.duration.as_secs_f32();
        if !(dur > 0.0) {
            // A zero (or nonsense) duration is an instant jump, not a divide by zero.
            // Reachable from a wire format, so it is handled rather than asserted.
            return Sample { value: self.to, velocity: 0.0, status: Status::Finished };
        }

        let (progress, finished) = self.linear_progress(t / dur);
        let value = self.from.lerp(self.to, easing.eval(progress));
        let velocity = if finished {
            0.0
        } else {
            // The difference is taken over the interval that actually exists and
            // divided by *that* interval, so it degrades to a correctly-scaled
            // one-sided difference at `t = 0` instead of a two-sided one over a
            // half-empty window. Dividing by a fixed `2h` was the first form, and it
            // reports half the true velocity at the start of every animation —
            // `ease-out` is at its fastest exactly there, which is exactly where a
            // retarget wants a real number.
            let h = VELOCITY_EPSILON.min(dur * 0.25);
            let (t0, t1) = ((t - h).max(0.0), t + h);
            let at = |x: f32| self.from.lerp(self.to, easing.eval(self.linear_progress(x / dur).0));
            (at(t1) - at(t0)) / (t1 - t0)
        };
        Sample {
            value,
            velocity,
            status: if finished { Status::Finished } else { Status::Running },
        }
    }

    /// Linear progress within the current iteration, and whether the track is over.
    ///
    /// `iterations` is the number of times the interval runs; `raw` is how many
    /// intervals have elapsed. Splitting this out is what lets the finite-difference
    /// velocity above reuse the *same* repeat-and-direction arithmetic as the value —
    /// a second copy of it would make velocity disagree with position at exactly the
    /// iteration boundaries, which is where an `Alternate` track reverses.
    fn linear_progress(&self, raw: f32) -> (f32, bool) {
        let raw = raw.max(0.0);
        let iterations = match self.repeat {
            Repeat::Once => 1.0,
            Repeat::Times(n) => (n.max(1)) as f32,
            Repeat::Forever => f32::INFINITY,
        };
        let (index, frac, finished) = if raw >= iterations {
            // Land on the last iteration's end rather than wrapping to its start,
            // which is what `raw.fract()` would do for an integral `raw`.
            ((iterations - 1.0).max(0.0), 1.0, true)
        } else {
            (raw.floor(), raw - raw.floor(), false)
        };
        let even = (index as i64) % 2 == 0;
        let p = match self.direction {
            Direction::Normal => frac,
            Direction::Reverse => 1.0 - frac,
            Direction::Alternate => {
                if even {
                    frac
                } else {
                    1.0 - frac
                }
            }
            Direction::AlternateReverse => {
                if even {
                    1.0 - frac
                } else {
                    frac
                }
            }
        };
        (p, finished)
    }

    /// How long from start to finish, or `None` for a track that never finishes.
    ///
    /// This is what lets [`Timeline::next_deadline`](crate::Timeline::next_deadline)
    /// give the render loop a real deadline instead of "ask again next frame": a
    /// 300ms-delayed toast should cost zero wakeups for 300ms, not eighteen.
    pub fn total_duration(&self) -> Option<Duration> {
        match self.curve {
            Curve::Spring(spring) => {
                let t = spring.settle_time(self.from, self.to, self.initial_velocity);
                Some(self.delay + Duration::from_secs_f32(t.max(0.0)))
            }
            Curve::Eased(_) => match self.repeat {
                Repeat::Forever => None,
                Repeat::Once => Some(self.delay + self.duration),
                Repeat::Times(n) => Some(self.delay + self.duration * n.max(1)),
            },
        }
    }
}

/// The finite-difference half-step for eased velocity, **in seconds**. Small enough
/// that the difference stays close to the true derivative for every curve in
/// [`Easing`], large enough that `f32` cancellation in the numerator is not the
/// dominant error. Capped at a quarter of the track's duration so a very short track
/// is not differenced over its own whole length.
const VELOCITY_EPSILON: f32 = 1e-3;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spring::Spring;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn an_eased_track_holds_its_start_through_its_delay_then_runs() {
        let t = Track::eased(0.0, 100.0, MS(200), Easing::Linear).with_delay(MS(100));
        assert_eq!(t.sample(MS(0)).status, Status::Delayed);
        assert_eq!(t.sample(MS(0)).value, 0.0);
        assert_eq!(t.sample(MS(99)).status, Status::Delayed);
        // The value during the delay is the *start*, not zero and not the target.
        // A track from 320 to 0 that sat at 0 during its delay would pop into place
        // and then slide, which is the bug this pins.
        let up = Track::eased(320.0, 0.0, MS(200), Easing::Linear).with_delay(MS(100));
        assert_eq!(up.sample(MS(50)).value, 320.0);

        assert_eq!(t.sample(MS(100)).status, Status::Running);
        assert!((t.sample(MS(200)).value - 50.0).abs() < 1e-3, "{}", t.sample(MS(200)).value);
        assert_eq!(t.sample(MS(300)).status, Status::Finished);
        assert_eq!(t.sample(MS(300)).value, 100.0);
        assert_eq!(t.sample(MS(9999)).value, 100.0, "a finished track holds its final value");
    }

    #[test]
    fn repeat_runs_the_interval_that_many_times_and_then_stops() {
        let t = Track::eased(0.0, 100.0, MS(100), Easing::Linear).with_repeat(Repeat::Times(3));
        assert_eq!(t.sample(MS(299)).status, Status::Running);
        assert_eq!(t.sample(MS(300)).status, Status::Finished);
        assert_eq!(t.sample(MS(300)).value, 100.0);
        // Mid-second-iteration is back near the start, not two thirds of the way
        // through one long run — the negative control for treating repeat as a
        // duration multiplier.
        assert!((t.sample(MS(150)).value - 50.0).abs() < 1e-3, "{}", t.sample(MS(150)).value);
        assert_eq!(t.total_duration(), Some(MS(300)));
    }

    #[test]
    fn forever_never_finishes_and_has_no_total_duration() {
        let t = Track::eased(0.0, 1.0, MS(100), Easing::Linear).with_repeat(Repeat::Forever);
        assert_eq!(t.total_duration(), None);
        for ms in [0u64, 99, 100, 5_000, 3_600_000] {
            assert_ne!(t.sample(MS(ms)).status, Status::Finished, "at {ms}ms");
        }
    }

    #[test]
    fn the_four_directions_are_four_different_tracks() {
        let base = Track::eased(0.0, 100.0, MS(100), Easing::Linear).with_repeat(Repeat::Times(2));
        let at = |d: Direction, ms: u64| base.with_direction(d).sample(MS(ms)).value;
        // First iteration, a quarter in.
        assert!((at(Direction::Normal, 25) - 25.0).abs() < 1e-3);
        assert!((at(Direction::Reverse, 25) - 75.0).abs() < 1e-3);
        assert!((at(Direction::Alternate, 25) - 25.0).abs() < 1e-3);
        assert!((at(Direction::AlternateReverse, 25) - 75.0).abs() < 1e-3);
        // Second iteration, a quarter in — where the alternating pair diverge from
        // their non-alternating namesakes.
        assert!((at(Direction::Normal, 125) - 25.0).abs() < 1e-3);
        assert!((at(Direction::Reverse, 125) - 75.0).abs() < 1e-3);
        assert!((at(Direction::Alternate, 125) - 75.0).abs() < 1e-3, "{}", at(Direction::Alternate, 125));
        assert!((at(Direction::AlternateReverse, 125) - 25.0).abs() < 1e-3);
    }

    /// A track that ends exactly on an iteration boundary must report the *end* of
    /// the last iteration, not the start of a nonexistent next one. `raw.fract()` is
    /// zero at both, so the natural implementation reads 0.0 here and the animation
    /// snaps back to its start on its final frame — one wrong frame, and the one the
    /// user's eye is on.
    #[test]
    fn a_track_ending_on_an_iteration_boundary_lands_on_its_target() {
        for rep in [Repeat::Once, Repeat::Times(1), Repeat::Times(4)] {
            let t = Track::eased(0.0, 100.0, MS(100), Easing::Linear).with_repeat(rep);
            let end = t.total_duration().unwrap();
            assert_eq!(t.sample(end).value, 100.0, "{rep:?}");
            assert_eq!(t.sample(end).status, Status::Finished, "{rep:?}");
        }
        // …and an alternating track ends where its last iteration ends, which for an
        // even count is back at the start.
        let alt = Track::eased(0.0, 100.0, MS(100), Easing::Linear)
            .with_repeat(Repeat::Times(2))
            .with_direction(Direction::Alternate);
        assert_eq!(alt.sample(MS(200)).value, 0.0);
    }

    #[test]
    fn a_zero_duration_track_is_an_instant_jump_not_a_divide_by_zero() {
        let t = Track::eased(0.0, 100.0, Duration::ZERO, Easing::EASE_OUT);
        let s = t.sample(Duration::ZERO);
        assert!(s.value.is_finite());
        assert_eq!(s.value, 100.0);
        assert_eq!(s.status, Status::Finished);
        // …and it still honours its delay, which is the case that makes this more
        // than a shortcut: a zero-duration change after 100ms is a discrete switch.
        let delayed = t.with_delay(MS(100));
        assert_eq!(delayed.sample(MS(50)).status, Status::Delayed);
        assert_eq!(delayed.sample(MS(50)).value, 0.0);
        assert_eq!(delayed.sample(MS(100)).value, 100.0);
    }

    #[test]
    fn a_spring_track_finishes_when_the_spring_settles() {
        let t = Track::spring(0.0, 100.0, Spring::critical(180.0, 1.0));
        assert_eq!(t.sample(Duration::ZERO).status, Status::Running);
        let total = t.total_duration().expect("a spring has a settle time");
        assert!(total > MS(50) && total < MS(2000), "{total:?}");
        assert_eq!(t.sample(total).status, Status::Finished);
        // Exactly on target once settled, not merely within the rest threshold —
        // otherwise the last painted frame is a thousandth of a unit off and stays
        // there until something else redraws.
        assert_eq!(t.sample(total).value, 100.0);
        assert_eq!(t.sample(total).velocity, 0.0);
    }

    #[test]
    fn a_spring_track_honours_its_delay_and_its_initial_velocity() {
        let t = Track::spring(0.0, 100.0, Spring::critical(180.0, 1.0))
            .with_delay(MS(100))
            .with_initial_velocity(500.0);
        assert_eq!(t.sample(MS(50)).status, Status::Delayed);
        assert_eq!(t.sample(MS(50)).value, 0.0);
        let thrown = t.sample(MS(150)).value;
        let from_rest = Track::spring(0.0, 100.0, Spring::critical(180.0, 1.0))
            .with_delay(MS(100))
            .sample(MS(150))
            .value;
        assert!(thrown > from_rest, "initial velocity did nothing: {thrown} vs {from_rest}");
        // The delay must shift the spring's whole timeline, not just gate it: total
        // duration includes it.
        assert!(t.total_duration().unwrap() > MS(100));
    }

    /// Velocity must agree with the value's own slope, including through the eased
    /// path where it is a finite difference. Checked against a difference taken at a
    /// deliberately *different* step size, so the two are not the same arithmetic
    /// twice.
    #[test]
    fn eased_velocity_tracks_the_slope_of_the_value() {
        for easing in [Easing::Linear, Easing::EASE_OUT, Easing::EASE_IN_OUT] {
            let t = Track::eased(0.0, 100.0, MS(200), easing);
            for ms in [20u64, 60, 100, 150, 190] {
                let reported = t.sample(MS(ms)).velocity;
                let coarse = (t.sample(MS(ms + 5)).value - t.sample(MS(ms - 5)).value) / 0.010;
                assert!(
                    (reported - coarse).abs() < 30.0 + coarse.abs() * 0.05,
                    "{easing:?} at {ms}ms: reported {reported}, coarse {coarse}"
                );
            }
        }
        // A linear 100 units over 200ms is 500 units/second, everywhere.
        let lin = Track::eased(0.0, 100.0, MS(200), Easing::Linear);
        assert!((lin.sample(MS(100)).velocity - 500.0).abs() < 1.0, "{}", lin.sample(MS(100)).velocity);
        // …and a delayed or finished track is not moving.
        assert_eq!(lin.with_delay(MS(50)).sample(MS(10)).velocity, 0.0);
        assert_eq!(lin.sample(MS(500)).velocity, 0.0);
    }

    /// `ease-out` starts with a zero `x` slope, which is where the rejected analytic
    /// derivative divides by zero. The finite difference must stay finite there — and
    /// large, because `ease-out` is at its fastest at `t = 0`.
    #[test]
    fn velocity_is_finite_at_the_singular_point_of_the_analytic_form() {
        let t = Track::eased(0.0, 100.0, MS(200), Easing::EASE_OUT);
        let v = t.sample(Duration::ZERO).velocity;
        assert!(v.is_finite(), "{v}");
        assert!(v > 500.0, "ease-out should be fastest at the start, got {v}");
        let t_in = Track::eased(0.0, 100.0, MS(200), Easing::EASE_IN);
        assert!(t_in.sample(MS(199)).velocity.is_finite());
    }
}
