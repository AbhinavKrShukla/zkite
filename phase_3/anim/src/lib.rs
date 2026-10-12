//! The animation driver's time-domain core — ../PLAN.md §45.
//!
//! # Why this shape, and not the obvious one
//!
//! The obvious design for an animation API in a Python framework is a controller the
//! app holds and reads inside `view()`:
//!
//! ```text
//! fade = Animation(0, 1, duration=300)   # app state
//! def view(s: S):
//!     return Box(f"opacity-{int(fade.value * 100)}", ...)
//! ```
//!
//! **That architecture is ruled out by a measurement that already exists**, which is
//! why it is recorded here rather than discovered later. It requires `view()` to run
//! on every animation frame, and ../PLAN.md §44.3's table prices that exactly: total
//! frame work is 12.438ms, of which a rebuild costs `py` 2.047 + `parse` 0.14 +
//! `relayout` 1.701 = 3.888ms, paid on 46 of 594 frames (7.74%). So an ordinary frame
//! is ~12.14ms, and a frame that re-runs `view()` is **~16.03ms against a 16.67ms
//! budget** — before §44.4's finding that the same CPython call costs 21% more at the
//! top of a frame than in the middle of one. An animation driven from Python would
//! consume the entire budget on the *first* frame of the *first* animation.
//!
//! The structural half of that matters more than the number: `view()`'s cost scales
//! with the whole screen, not with the node being animated. Fading one 48dp icon
//! would cost exactly what rebuilding a 25-row list costs.
//!
//! **So: no Python in an animation frame.** The app declares an animation once; this
//! crate is sampled per frame by whoever owns the frame (the render thread on native,
//! `zkite-runtime.js` on the WebView); Python hears back exactly once, when the
//! animation finishes, down the event path it already has.
//!
//! # What is in here and what is deliberately not
//!
//! In: how a value moves through time. [`Easing`] and [`Spring`] are the two shapes a
//! value can take; [`Track`] is one value moving once (with delay, repeat and
//! direction); [`Timeline`] owns a set of tracks, samples them against a clock, and
//! answers the two questions the render loop actually asks — *what is everything
//! worth right now* and *when do you next need a frame*.
//!
//! Not in: what a channel *means*. This crate never hears the words `opacity` or
//! `translate_y`; it moves `f32`s and colours between numbers. Mapping a channel onto
//! a `ComputedStyle` override is `gpu_harness`'s business, and keeping that out of
//! here is what lets the whole crate be tested with no renderer, no tree and no
//! device.
//!
//! Also not in: the fling. ../PLAN.md §41's exponential decay lives in
//! `zkite_frametiming` and stays there for this section. A fling *is* an animation and
//! the two will likely merge when the driver subsumes `tick_fling` (§45.9), but
//! moving a shipped, calibrated, device-measured curve at the same time as
//! introducing a new one would make any change in scroll feel unattributable.
//!
//! # The clock is a parameter
//!
//! Nothing here calls `Instant::now()`. Every entry point that needs the time is
//! handed one, so a test can advance 16.67ms at a time and get an exactly reproducible
//! answer. That is the same reason `zkite_frametiming` takes a `frame_period` rather
//! than reading the display: an instrument that reads the world cannot be pinned.

mod easing;
mod lerp;
mod spring;
mod timeline;
mod track;

pub use easing::{Easing, StepPosition};
pub use lerp::{Lerp, Rgba};
pub use spring::{Spring, SpringSample};
pub use timeline::{Advance, Interrupt, Timeline};
pub use track::{Curve, Direction, Repeat, Sample, Status, Track};
