//! What it means to be part-way between two values.
//!
//! Scalars are obvious and colours are not, which is the whole content of this
//! module. ../PLAN.md §38.3 is the precedent: a colour that was right for a fill was
//! wrong for a gradient stop, because the space the arithmetic happens in is not a
//! detail. Getting it wrong here is quieter still — a mid-transition frame is on
//! screen for 8ms and nobody screenshots it.

/// A value that can be sampled part-way between two of its own kind.
///
/// `t` is *eased* progress, not linear progress, and it is **not** clamped: a curve
/// with an overshooting control point legitimately hands this a `t` above 1, and
/// clamping here would flatten every such curve after [`Easing`](crate::Easing) went
/// to the trouble of not doing so. Implementations that cannot represent an
/// out-of-range result (a colour channel, an alpha) clamp the *result*, because that
/// is a fact about the channel rather than about the curve.
pub trait Lerp: Copy {
    fn lerp(self, other: Self, t: f32) -> Self;
}

impl Lerp for f32 {
    fn lerp(self, other: f32, t: f32) -> f32 {
        // `a + (b - a) * t` rather than `a*(1-t) + b*t`: the second form is more
        // numerically symmetric but does not reproduce the endpoints exactly in
        // `f32`, and landing exactly on the target at t=1 is a property `Track`
        // depends on to decide an animation is over.
        self + (other - self) * t
    }
}

/// A colour in **non-premultiplied sRGB**, components in `[0, 1]`.
///
/// Non-premultiplied on the outside because that is what every caller has (a
/// `twcore::Color` is four `u8`s of straight alpha) and premultiplied only inside
/// [`Lerp::lerp`], which is where it matters. Kept as `f32` rather than `u8` so a
/// 200ms fade across 12 frames does not quantise to the same byte twice in a row.
///
/// This crate deliberately does not depend on `twcore` — see the manifest — so
/// converting is the caller's one line.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Rgba {
        Rgba { r, g, b, a }
    }

    /// From four straight-alpha bytes — `twcore::Color`'s representation.
    pub fn from_u8(r: u8, g: u8, b: u8, a: u8) -> Rgba {
        Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 }
    }

    /// Back to four straight-alpha bytes, rounded rather than truncated.
    pub fn to_u8(self) -> (u8, u8, u8, u8) {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        (q(self.r), q(self.g), q(self.b), q(self.a))
    }
}

impl Lerp for Rgba {
    /// Interpolates in **premultiplied** sRGB, which is what CSS specifies for a
    /// colour transition and — more to the point — the only form that gets a fade
    /// from transparent right.
    ///
    /// The case that decides it: `transparent` → `red`. `transparent` is
    /// `rgba(0,0,0,0)`, so interpolating the channels straight gives `rgb(127,0,0)`
    /// at the midpoint — a *dark* red fading in, because half of black got mixed into
    /// it. Premultiplying first gives `rgb(255,0,0)` at half alpha, which is the same
    /// red getting more opaque. Both look plausible in isolation; only one matches
    /// the WebView, and "looks plausible in isolation" is how §14.2's list gets
    /// longer.
    ///
    /// sRGB rather than a linear or perceptual space, for the same reason: it is what
    /// the other backend does. A prettier space would be a divergence bought for
    /// taste.
    fn lerp(self, other: Rgba, t: f32) -> Rgba {
        let (a0, a1) = (self.a, other.a);
        let a = a0.lerp(a1, t);
        if a <= 0.0 {
            // Fully transparent: hue is unobservable, and un-premultiplying would
            // divide by zero. Carrying the alpha alone keeps the endpoints exact.
            return Rgba { r: 0.0, g: 0.0, b: 0.0, a: a.clamp(0.0, 1.0) };
        }
        let pm = |c: f32, alpha: f32| c * alpha;
        let r = pm(self.r, a0).lerp(pm(other.r, a1), t) / a;
        let g = pm(self.g, a0).lerp(pm(other.g, a1), t) / a;
        let b = pm(self.b, a0).lerp(pm(other.b, a1), t) / a;
        // Clamped because a channel is a channel — an overshooting curve may take
        // `t` past 1, and 1.04 of red is not a colour. The overshoot survives on
        // channels that can express it (a translate, a scale); this one cannot.
        Rgba { r: r.clamp(0.0, 1.0), g: g.clamp(0.0, 1.0), b: b.clamp(0.0, 1.0), a: a.clamp(0.0, 1.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_reproduce_their_endpoints_exactly() {
        // Exactly, not nearly: `Track` decides an animation has finished by landing
        // on the target, and `a*(1-t) + b*t` — the other obvious spelling — does not
        // return `b` for t=1 at every pair of floats.
        for (a, b) in [(0.0f32, 1.0f32), (-320.0, 0.0), (0.1, 0.7), (1e6, -1e6)] {
            assert_eq!(a.lerp(b, 0.0), a);
            assert_eq!(a.lerp(b, 1.0), b);
        }
        assert_eq!(0.0f32.lerp(100.0, 0.25), 25.0);
    }

    #[test]
    fn a_scalar_follows_an_overshooting_curve_past_its_target() {
        // The counterpart to `Easing`'s overshoot test, one layer down: nothing in
        // the scalar path may clamp, or a "back out" translate stops backing out.
        assert_eq!(0.0f32.lerp(100.0, 1.1), 110.0);
        assert_eq!(0.0f32.lerp(100.0, -0.1), -10.0);
    }

    /// The test that decides the premultiplied question, and the one that fails
    /// against the naive implementation. Fading in from `transparent` must not drag
    /// the colour through black.
    #[test]
    fn fading_in_from_transparent_keeps_its_hue() {
        let transparent = Rgba::new(0.0, 0.0, 0.0, 0.0);
        let red = Rgba::new(1.0, 0.0, 0.0, 1.0);
        let mid = transparent.lerp(red, 0.5);
        assert!((mid.a - 0.5).abs() < 1e-6, "alpha {}", mid.a);
        assert!((mid.r - 1.0).abs() < 1e-4, "premultiplied red should stay full red, got {}", mid.r);
        // The naive answer is 0.5 here; assert we are nowhere near it, so the test
        // discriminates rather than merely passing.
        assert!(mid.r > 0.9, "got {} — this is the straight-alpha answer", mid.r);
        assert_eq!(mid.to_u8(), (255, 0, 0, 128));
    }

    /// Two opaque colours are the case where premultiplication must make no
    /// difference at all — the common one, and the control for the test above.
    #[test]
    fn opaque_colours_interpolate_channelwise() {
        let black = Rgba::new(0.0, 0.0, 0.0, 1.0);
        let white = Rgba::new(1.0, 1.0, 1.0, 1.0);
        let mid = black.lerp(white, 0.5);
        assert!((mid.r - 0.5).abs() < 1e-6 && (mid.g - 0.5).abs() < 1e-6 && (mid.b - 0.5).abs() < 1e-6);
        assert_eq!(mid.a, 1.0);
    }

    #[test]
    fn colour_endpoints_are_exact_and_round_trip_through_bytes() {
        let a = Rgba::from_u8(59, 130, 246, 255);
        let b = Rgba::from_u8(239, 68, 68, 255);
        assert_eq!(a.lerp(b, 0.0).to_u8(), (59, 130, 246, 255));
        assert_eq!(a.lerp(b, 1.0).to_u8(), (239, 68, 68, 255));
    }

    /// Both endpoints transparent: no hue exists to preserve and the un-premultiply
    /// would divide by zero. Must stay finite — a NaN colour reaches the paint path.
    #[test]
    fn interpolating_between_two_transparent_colours_does_not_divide_by_zero() {
        let a = Rgba::new(1.0, 0.0, 0.0, 0.0);
        let b = Rgba::new(0.0, 0.0, 1.0, 0.0);
        for i in 0..=10 {
            let out = a.lerp(b, i as f32 / 10.0);
            assert!(out.r.is_finite() && out.g.is_finite() && out.b.is_finite() && out.a.is_finite(), "{out:?}");
            assert_eq!(out.a, 0.0);
        }
    }

    /// An overshooting curve applied to a colour must clamp — the channel cannot
    /// represent 1.1 of red — while leaving the scalar path free to overshoot. Both
    /// halves are asserted here so the pair cannot drift.
    #[test]
    fn an_overshooting_curve_clamps_a_colour_but_not_a_scalar() {
        let a = Rgba::new(0.0, 0.0, 0.0, 1.0);
        let b = Rgba::new(1.0, 1.0, 1.0, 1.0);
        let over = a.lerp(b, 1.3);
        assert_eq!(over.to_u8(), (255, 255, 255, 255));
        assert!(over.r <= 1.0 && over.a <= 1.0);
        assert_eq!(0.0f32.lerp(1.0, 1.3), 1.3);
    }
}
