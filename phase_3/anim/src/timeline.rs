//! The set of animations that are currently running, and the two questions the
//! render loop actually asks it.
//!
//! *What is everything worth right now* — [`advance`](Timeline::advance) followed by
//! [`value`](Timeline::value) per key — and *when do you next need a frame* —
//! [`next_deadline`](Timeline::next_deadline). The second one is why this type exists
//! rather than the driver keeping a `Vec<Track>`: `render_thread`'s loop blocks on
//! `recv_timeout(deadline)` and has exactly one thing to wait for today (a fling).
//! Giving it a real deadline instead of "wake me every frame in case" is what keeps a
//! 300ms-delayed toast from costing eighteen wakeups before it does anything.
//!
//! # Finished animations are held, not dropped
//!
//! When a track finishes, its entry stays, holding the final value, until the owner
//! explicitly removes it. Dropping it on completion is the obvious design and it has
//! a visible bug in it: the completion is reported to Python, Python re-renders with
//! the final state, and that re-render lands *one or more frames later* — during
//! which the node would paint at its class-resolved value again. A sheet would slide
//! up, snap back down for a frame, then reappear. Holding costs one map entry;
//! [`retain`](Timeline::retain) is how the owner evicts entries for nodes that have
//! actually left the tree.

use crate::track::{Sample, Status, Track};
use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// What happens when an animation is started for a key that already has one.
///
/// This is the policy an animation driver lives or dies by. All three are needed by
/// real cases, so none is a default in disguise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interrupt {
    /// Throw the old one away and run the new one exactly as written, from its own
    /// `from`. Right when the two animations are unrelated — a list row being
    /// recycled into a different row.
    Restart,
    /// Run the new track, but start it from wherever the old one had got to.
    ///
    /// **What is preserved depends on the curve, and the difference is worth knowing.**
    /// For a [`Spring`](crate::Spring) both position *and* velocity carry across, so
    /// an interrupted spring is genuinely continuous — the same integration heading
    /// somewhere new. For an eased track only position carries across: a fixed-
    /// duration curve has a shape of its own and cannot also honour an incoming
    /// velocity, so an interrupted ease is continuous in value and has a kink in its
    /// slope. That kink is small and a jump is not, which is why this is still the
    /// right policy for eased tracks — but it is the reason a sheet the user can grab
    /// mid-animation should be a spring.
    Retarget,
    /// Leave the running one alone; start the new one only if nothing is running.
    ///
    /// The policy for a *declared* animation, which is the common case here: the tree
    /// is rebuilt on every frame of a scroll, so the same `enter` spec arrives over
    /// and over, and neither restarting nor retargeting it every frame is wanted.
    /// A finished entry does not block a new run, so a second open of the same sheet
    /// animates again.
    Ignore,
}

struct Entry {
    track: Track,
    started: Instant,
    last: Sample,
    /// Whether this entry's completion has already been handed to the owner.
    /// Completion is reported exactly once per run, no matter how many times
    /// `advance` is called afterwards.
    completion_reported: bool,
}

/// What changed during one [`Timeline::advance`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advance<K> {
    /// Keys whose track reached [`Status::Finished`] during *this* call. The owner
    /// turns these into the one Python callback an animation is allowed (see the
    /// crate doc), so reporting one twice would fire an `on_end` twice.
    pub finished: Vec<K>,
}

/// A set of running animations, keyed by whatever identifies a channel to the owner.
///
/// The key is the caller's: `gpu_harness` uses `(node id, channel)`, tests use
/// `&'static str`. Nothing here interprets it.
pub struct Timeline<K: Eq + Hash + Clone> {
    entries: HashMap<K, Entry>,
}

impl<K: Eq + Hash + Clone> Default for Timeline<K> {
    fn default() -> Self {
        Timeline { entries: HashMap::new() }
    }
}

impl<K: Eq + Hash + Clone> Timeline<K> {
    pub fn new() -> Timeline<K> {
        Timeline::default()
    }

    /// Start `track` for `key`, resolving any existing animation per `policy`.
    ///
    /// Returns whether a track is now running for this key as a result of this call —
    /// `false` only for an [`Interrupt::Ignore`] that declined.
    pub fn start(&mut self, key: K, track: Track, now: Instant, policy: Interrupt) -> bool {
        let existing = self.entries.get(&key);
        let track = match (policy, existing) {
            (Interrupt::Ignore, Some(e)) if e.last.status != Status::Finished => return false,
            (Interrupt::Retarget, Some(e)) => {
                let mut t = track;
                t.from = e.last.value;
                t.initial_velocity = e.last.velocity;
                t
            }
            _ => track,
        };
        // Sampled at zero immediately rather than left until the next `advance`, so
        // `value(&key)` is answerable the moment this returns. The alternative — an
        // `Option<Sample>` that is `None` for one frame — pushes that case into every
        // caller, and the caller is a paint path.
        let last = track.sample(Duration::ZERO);
        self.entries.insert(key, Entry { track, started: now, last, completion_reported: false });
        true
    }

    /// Re-sample every entry against `now`, and report the ones that just finished.
    pub fn advance(&mut self, now: Instant) -> Advance<K> {
        let mut finished = Vec::new();
        for (key, entry) in self.entries.iter_mut() {
            // `saturating_duration_since`: a clock that went backwards (or a `now`
            // from before this entry started, which a caller reusing one timestamp
            // across a frame can produce) reads as zero elapsed rather than as a
            // negative that would wrap into an enormous `Duration`.
            let elapsed = now.saturating_duration_since(entry.started);
            entry.last = entry.track.sample(elapsed);
            if entry.last.status == Status::Finished && !entry.completion_reported {
                entry.completion_reported = true;
                finished.push(key.clone());
            }
        }
        Advance { finished }
    }

    /// This key's value as of the last [`advance`](Timeline::advance) — including for
    /// a finished animation, which holds its final value.
    pub fn value(&self, key: &K) -> Option<f32> {
        self.entries.get(key).map(|e| e.last.value)
    }

    /// The full last sample, for an owner that also wants velocity or status.
    pub fn sample(&self, key: &K) -> Option<Sample> {
        self.entries.get(key).map(|e| e.last)
    }

    /// Whether anything is still moving — i.e. whether the render loop must keep
    /// producing frames. False when every entry has finished, which is what lets the
    /// thread go back to a blocking `recv()`.
    pub fn is_active(&self) -> bool {
        self.entries.values().any(|e| e.last.status != Status::Finished)
    }

    /// How long the owner may sleep before it needs to sample again.
    ///
    /// `Some(ZERO)` means "something is running, paint the next frame". A longer
    /// duration means everything unfinished is still inside its delay, and this is
    /// exactly how long until the earliest one starts. `None` means nothing is
    /// unfinished and the loop may block indefinitely.
    ///
    /// Deliberately *not* the frame period: pacing is the render loop's business
    /// (`next_frame_wait` already subtracts the current frame's own work from it, see
    /// ../PLAN.md §13.2), and a timeline that returned 16.67ms would be a second,
    /// disagreeing opinion about when a frame is due.
    pub fn next_deadline(&self, now: Instant) -> Option<Duration> {
        let mut best: Option<Duration> = None;
        for entry in self.entries.values() {
            match entry.last.status {
                Status::Finished => continue,
                Status::Running => return Some(Duration::ZERO),
                Status::Delayed => {
                    let starts_at = entry.started + entry.track.delay;
                    let wait = starts_at.saturating_duration_since(now);
                    best = Some(match best {
                        Some(b) if b <= wait => b,
                        _ => wait,
                    });
                }
            }
        }
        best
    }

    /// Drop this key's animation entirely. Returns whether there was one.
    pub fn cancel(&mut self, key: &K) -> bool {
        self.entries.remove(key).is_some()
    }

    /// Keep only the entries the owner still wants — how a node that has left the
    /// tree stops holding its final value forever. Without this, a `List` that
    /// animates its rows would accumulate one entry per row ever scrolled past.
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        self.entries.retain(|k, _| keep(k));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::easing::Easing;
    use crate::spring::Spring;
    use crate::track::{Repeat, Status};

    const MS: fn(u64) -> Duration = Duration::from_millis;

    fn linear(from: f32, to: f32, ms: u64) -> Track {
        Track::eased(from, to, MS(ms), Easing::Linear)
    }

    #[test]
    fn a_started_track_is_readable_before_the_first_advance() {
        let t0 = Instant::now();
        let mut tl: Timeline<&str> = Timeline::new();
        tl.start("fade", linear(0.0, 1.0, 200), t0, Interrupt::Restart);
        // No `advance` yet. A `None` here would push an "it might not exist for one
        // frame" case into the paint path.
        assert_eq!(tl.value(&"fade"), Some(0.0));
        assert!(tl.is_active());
    }

    #[test]
    fn advancing_moves_the_value_and_finishing_stops_the_timeline() {
        let t0 = Instant::now();
        let mut tl: Timeline<&str> = Timeline::new();
        tl.start("x", linear(0.0, 100.0, 200), t0, Interrupt::Restart);

        tl.advance(t0 + MS(100));
        assert!((tl.value(&"x").unwrap() - 50.0).abs() < 1e-3);
        assert!(tl.is_active());
        assert_eq!(tl.next_deadline(t0 + MS(100)), Some(Duration::ZERO));

        let done = tl.advance(t0 + MS(200));
        assert_eq!(done.finished, vec!["x"]);
        assert_eq!(tl.value(&"x"), Some(100.0));
        assert!(!tl.is_active(), "a finished timeline must let the loop sleep");
        assert_eq!(tl.next_deadline(t0 + MS(200)), None);
    }

    /// Completion is what fires an author's `on_end`, and it crosses into Python.
    /// Reporting it on every subsequent frame would fire the callback sixty times a
    /// second forever — and since the natural implementation is "status == Finished",
    /// that is exactly what a first draft does. This is the test that rejects it.
    #[test]
    fn completion_is_reported_exactly_once_however_often_we_advance() {
        let t0 = Instant::now();
        let mut tl: Timeline<&str> = Timeline::new();
        tl.start("x", linear(0.0, 1.0, 100), t0, Interrupt::Restart);
        let mut reports = 0;
        for ms in [50u64, 100, 150, 200, 5000] {
            reports += tl.advance(t0 + MS(ms)).finished.len();
        }
        assert_eq!(reports, 1, "completion fired {reports} times");
        // …and the value is still held, which is the other half of the contract.
        assert_eq!(tl.value(&"x"), Some(1.0));
    }

    /// The wakeup-saving property, and the reason `next_deadline` is not just a bool.
    /// A timeline holding nothing but delayed animations must report the real wait,
    /// not zero — otherwise a 300ms delay costs eighteen wakeups that paint nothing.
    #[test]
    fn a_purely_delayed_timeline_reports_its_real_wait_not_zero() {
        let t0 = Instant::now();
        let mut tl: Timeline<&str> = Timeline::new();
        tl.start("late", linear(0.0, 1.0, 100).with_delay(MS(300)), t0, Interrupt::Restart);
        tl.start("later", linear(0.0, 1.0, 100).with_delay(MS(500)), t0, Interrupt::Restart);
        tl.advance(t0);
        // The earliest of the two, measured from now.
        assert_eq!(tl.next_deadline(t0), Some(MS(300)));
        assert_eq!(tl.next_deadline(t0 + MS(100)), Some(MS(200)));
        assert!(tl.is_active(), "delayed is not finished");
        // Once one of them is actually running, every frame is needed again.
        tl.advance(t0 + MS(350));
        assert_eq!(tl.next_deadline(t0 + MS(350)), Some(Duration::ZERO));
        // A delay that has already elapsed is zero, not a negative that would wrap.
        let mut tl2: Timeline<&str> = Timeline::new();
        tl2.start("l", linear(0.0, 1.0, 100).with_delay(MS(300)), t0, Interrupt::Restart);
        tl2.advance(t0);
        assert_eq!(tl2.next_deadline(t0 + MS(999)), Some(Duration::ZERO));
    }

    /// `Retarget` on a spring is the case the whole policy exists for: interrupting a
    /// spring mid-flight must be continuous in *both* position and velocity. The
    /// negative control is `Restart` on the identical input, which jumps.
    #[test]
    fn retargeting_a_spring_is_continuous_where_restarting_jumps() {
        let t0 = Instant::now();
        let s = Spring::critical(180.0, 1.0);
        let mut tl: Timeline<&str> = Timeline::new();
        tl.start("y", Track::spring(0.0, 320.0, s), t0, Interrupt::Restart);
        tl.advance(t0 + MS(80));
        let (mid_value, mid_velocity) = {
            let s = tl.sample(&"y").unwrap();
            (s.value, s.velocity)
        };
        assert!(mid_value > 1.0 && mid_velocity > 1.0, "the probe never got the spring moving: {mid_value} / {mid_velocity}");

        // Retarget to a new destination from the same state.
        let mut retargeted = tl;
        retargeted.start("y", Track::spring(0.0, 0.0, s), t0 + MS(80), Interrupt::Retarget);
        let after = retargeted.value(&"y").unwrap();
        assert!((after - mid_value).abs() < 1e-4, "retarget jumped: {mid_value} -> {after}");
        assert!(
            (retargeted.sample(&"y").unwrap().velocity - mid_velocity).abs() < 1e-3,
            "retarget dropped the velocity"
        );

        // Restart, same inputs, is the control: it snaps back to the track's own
        // `from` and forgets the momentum.
        let mut restarted: Timeline<&str> = Timeline::new();
        restarted.start("y", Track::spring(0.0, 320.0, s), t0, Interrupt::Restart);
        restarted.advance(t0 + MS(80));
        restarted.start("y", Track::spring(0.0, 0.0, s), t0 + MS(80), Interrupt::Restart);
        let jumped = restarted.value(&"y").unwrap();
        assert!((jumped - 0.0).abs() < 1e-6, "expected a jump to 0.0, got {jumped}");
        assert!((jumped - mid_value).abs() > 1.0, "the control did not discriminate");
    }

    /// `Retarget` on an *eased* track preserves position and cannot preserve
    /// velocity. Documented behaviour, pinned so it stays documented rather than
    /// becoming a surprise.
    #[test]
    fn retargeting_an_eased_track_preserves_position_but_not_slope() {
        let t0 = Instant::now();
        let mut tl: Timeline<&str> = Timeline::new();
        tl.start("x", linear(0.0, 100.0, 200), t0, Interrupt::Restart);
        tl.advance(t0 + MS(100));
        let mid = tl.value(&"x").unwrap();
        assert!((mid - 50.0).abs() < 1e-3);
        tl.start("x", linear(0.0, 0.0, 200), t0 + MS(100), Interrupt::Retarget);
        assert!((tl.value(&"x").unwrap() - mid).abs() < 1e-4, "eased retarget must not jump");
        // The new track runs from the current value to the new target over its own
        // duration, so it is at half of *that* after half of it.
        tl.advance(t0 + MS(200));
        assert!((tl.value(&"x").unwrap() - 25.0).abs() < 1e-2, "{}", tl.value(&"x").unwrap());
    }

    /// `Ignore` is what makes a *declared* animation idempotent: the tree is rebuilt
    /// on every frame of a scroll, so the same `enter` spec arrives ~60 times a
    /// second. Without it, a 300ms entrance restarts every frame and never finishes.
    #[test]
    fn ignore_leaves_a_running_animation_alone_but_not_a_finished_one() {
        let t0 = Instant::now();
        let mut tl: Timeline<&str> = Timeline::new();
        tl.start("enter", linear(0.0, 1.0, 200), t0, Interrupt::Restart);
        for ms in [16u64, 32, 48, 64, 80] {
            tl.advance(t0 + MS(ms));
            assert!(!tl.start("enter", linear(0.0, 1.0, 200), t0 + MS(ms), Interrupt::Ignore));
        }
        tl.advance(t0 + MS(100));
        assert!(
            (tl.value(&"enter").unwrap() - 0.5).abs() < 1e-2,
            "restarted every frame: {}",
            tl.value(&"enter").unwrap()
        );
        // The negative control: `Restart` in the same loop never gets anywhere.
        let mut restarted: Timeline<&str> = Timeline::new();
        restarted.start("enter", linear(0.0, 1.0, 200), t0, Interrupt::Restart);
        for ms in [16u64, 32, 48, 64, 80] {
            restarted.advance(t0 + MS(ms));
            restarted.start("enter", linear(0.0, 1.0, 200), t0 + MS(ms), Interrupt::Restart);
        }
        restarted.advance(t0 + MS(100));
        assert!(restarted.value(&"enter").unwrap() < 0.15, "the control did not discriminate");

        // Once finished, the same spec starts a fresh run — a sheet reopening.
        tl.advance(t0 + MS(500));
        assert!(tl.start("enter", linear(0.0, 1.0, 200), t0 + MS(500), Interrupt::Ignore));
        assert_eq!(tl.value(&"enter"), Some(0.0));
    }

    /// A finished entry keeps answering with its final value until it is evicted.
    /// This is the "sheet slides up, snaps down for one frame, reappears" bug stated
    /// as a test: if `advance` dropped finished entries, `value` would be `None`
    /// here and the paint path would fall back to the class-resolved value.
    #[test]
    fn a_finished_animation_holds_its_value_until_it_is_evicted() {
        let t0 = Instant::now();
        let mut tl: Timeline<&str> = Timeline::new();
        tl.start("slide", linear(320.0, 0.0, 100), t0, Interrupt::Restart);
        tl.advance(t0 + MS(150));
        assert_eq!(tl.value(&"slide"), Some(0.0));
        assert_eq!(tl.len(), 1);
        for ms in [200u64, 400, 10_000] {
            tl.advance(t0 + MS(ms));
            assert_eq!(tl.value(&"slide"), Some(0.0), "dropped its held value at {ms}ms");
        }
        assert!(tl.cancel(&"slide"));
        assert_eq!(tl.value(&"slide"), None);
        assert!(!tl.cancel(&"slide"));
    }

    /// `retain` is how the owner stops a long-lived timeline from growing without
    /// bound — one held entry per row a virtualised `List` has ever scrolled past.
    #[test]
    fn retain_evicts_the_keys_the_owner_no_longer_has() {
        let t0 = Instant::now();
        let mut tl: Timeline<u32> = Timeline::new();
        for i in 0..10 {
            tl.start(i, linear(0.0, 1.0, 50), t0, Interrupt::Restart);
        }
        tl.advance(t0 + MS(100));
        assert_eq!(tl.len(), 10);
        tl.retain(|k| *k >= 7);
        assert_eq!(tl.len(), 3);
        assert_eq!(tl.value(&0), None);
        assert_eq!(tl.value(&7), Some(1.0));
    }

    /// A `Forever` track never finishes, never reports a completion, and never lets
    /// the loop sleep. Pinned because all three follow from one flag and a spinner
    /// that silently stopped after one rotation would be reported as a rendering bug.
    #[test]
    fn a_forever_track_never_finishes_and_never_lets_the_loop_idle() {
        let t0 = Instant::now();
        let mut tl: Timeline<&str> = Timeline::new();
        tl.start("spin", linear(0.0, 360.0, 1000).with_repeat(Repeat::Forever), t0, Interrupt::Restart);
        for ms in [500u64, 1000, 5_000, 60_000] {
            assert!(tl.advance(t0 + MS(ms)).finished.is_empty(), "reported completion at {ms}ms");
            assert!(tl.is_active());
            assert_eq!(tl.next_deadline(t0 + MS(ms)), Some(Duration::ZERO));
            assert_eq!(tl.sample(&"spin").unwrap().status, Status::Running);
        }
    }

    /// A `now` from before an entry started — which an owner reusing one timestamp
    /// across a frame can produce — must read as zero elapsed. `Instant`'s plain
    /// subtraction panics on that in debug builds, and this is a render thread.
    #[test]
    fn a_backwards_clock_reads_as_zero_elapsed_rather_than_panicking() {
        let t0 = Instant::now() + MS(1000);
        let mut tl: Timeline<&str> = Timeline::new();
        tl.start("x", linear(0.0, 100.0, 200), t0, Interrupt::Restart);
        let out = tl.advance(t0 - MS(500));
        assert!(out.finished.is_empty());
        assert_eq!(tl.value(&"x"), Some(0.0));
    }
}
