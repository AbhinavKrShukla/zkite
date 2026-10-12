//! `twcore`'s gradient spec plus a laid-out rect -> one `peniko::Gradient`, ready to
//! hand to `set_paint` on either vello backend.
//!
//! ../PLAN.md §38. Before it, `bg-linear-to-r from-cyan-500 to-blue-500` rendered a
//! correct gradient on the WebView backend (it is real Tailwind) and a **plain
//! background** on the native one, because `twcore` had no gradient parsing and the
//! paint path only filled solid colours — §15.1 divergence 3, reported from a real app.
//!
//! # Why this returns a `peniko` type, when the crate refuses to depend on a backend
//!
//! The crate doc's rule is "no `vello_cpu` or `vello_hybrid` dependency" — paint differs
//! per backend, layout must not. `peniko` is neither: it is the paint *vocabulary* both
//! backends re-export (`vello_common::paint::PaintType` is literally
//! `peniko::Brush<Image, Gradient>`), with no rasteriser in it, exactly as `kurbo` —
//! already a dependency for icon and decoration geometry — is the geometry vocabulary.
//!
//! Returning the finished brush rather than loose geometry is deliberate, and it is the
//! §9.2 lesson: **four consumers paint** (`gpu_harness`'s GPU and CPU paths,
//! `conformance`, `spike1_render_pipeline`), and three of the fields on a gradient are
//! silent-divergence-shaped if one of them forgets to set them:
//!
//! * `interpolation_cs` defaults to **sRGB** in `peniko` and must be **Oklab** to match
//!   Tailwind v4. Measured: `from-yellow-400 to-blue-600` differs between the two spaces
//!   by **81/255 in the blue channel** at the midpoint (§38.1). A consumer that built its
//!   own `Gradient` and left this alone would render a plausible, wrong ramp.
//! * `interpolation_alpha_space` must be `Premultiplied`, which is CSS's rule for
//!   interpolating *to* a transparent stop — and a one-stop Tailwind gradient
//!   (`bg-linear-to-r from-cyan-500`) fades to transparent, so this is the common case,
//!   not an exotic one. Measured: the midpoint of cyan → transparent over white reads
//!   (128, 219, 236); interpolating unpremultiplied would give (127, 173, 182).
//! * `extend` must be `Pad`, because CSS gradients hold their end colours.
//!
//! Handing back a value with all three already correct is what makes a wrong one
//! impossible rather than merely unlikely, and it means this module's own host tests
//! guard every backend at once.
//!
//! # The coordinate contract
//!
//! Points are in **dp**, the same space as `PositionedNode::rect` and
//! `decoration.rs`'s paths. vello applies the scene's density transform to the paint as
//! well as to the geometry, so a gradient built here lands correctly at any dpr — see
//! `render_thread.rs`'s note on `set_paint_transform` for the one place that is not
//! automatic (images, which pre-scale their own transform).

use kurbo::Point;
use peniko::color::{ColorSpaceTag, HueDirection};
use peniko::{ColorStop, ColorStops, Extend, Gradient, InterpolationAlphaSpace};
use twcore::style::{
    ComputedStyle, ExtColor, Gradient as TwGradient, GradientColorSpace, GradientHue, GradientLine,
    GradientStop,
};

/// The gradient to fill this node's box with, or `None` when the node declared none.
///
/// `rect` is `(x, y, w, h)` absolute dp — `PositionedNode::rect` verbatim.
pub fn background_gradient(rect: (f32, f32, f32, f32), style: &ComputedStyle) -> Option<Gradient> {
    let g = style.paint.gradient.resolve()?;
    Some(build(rect, &g))
}

/// The unit vector the gradient runs along, in dp-space (y grows downward, as CSS's own
/// coordinates do).
///
/// CSS defines the four **corner** keywords geometrically rather than as fixed angles:
/// `to top right` means the line perpendicular to the gradient line passes through the
/// other two corners, so the direction depends on the box's aspect ratio and only a
/// laid-out rect can produce it. That is why `GradientLine` keeps keywords as keywords.
///
/// Confirmed against Chromium's own pixels, not just against the spec text. For a
/// 200x100 box with `bg-linear-to-tr from-cyan-500 to-blue-500`, this direction puts the
/// midline's left edge at offset **0.25** and its centre at **0.50** along the ramp; the
/// probe reads (0, 171, 229) and (0, 157, 240) there, against (0, 171, 230) and
/// (0, 157, 241) at the same two offsets of the same ramp drawn `to-r`. §38.2.
fn direction(w: f32, h: f32, line: GradientLine) -> (f64, f64) {
    let (dx, dy) = match line {
        GradientLine::ToTop => (0.0, -1.0),
        GradientLine::ToRight => (1.0, 0.0),
        GradientLine::ToBottom => (0.0, 1.0),
        GradientLine::ToLeft => (-1.0, 0.0),
        // Perpendicular to the diagonal joining the *other* two corners: for
        // `to top right` that diagonal is top-left → bottom-right, i.e. `(w, h)`, and
        // `(h, -w)` is perpendicular to it and points up and to the right.
        GradientLine::ToTopRight => (h as f64, -(w as f64)),
        GradientLine::ToTopLeft => (-(h as f64), -(w as f64)),
        GradientLine::ToBottomRight => (h as f64, w as f64),
        GradientLine::ToBottomLeft => (-(h as f64), w as f64),
        // CSS's `<angle>`: 0deg points to the top and the angle runs clockwise, so a
        // 90deg gradient points to the right. In a y-down space that is
        // `(sin a, -cos a)`.
        GradientLine::Angle(deg) => {
            let a = (deg as f64).to_radians();
            (a.sin(), -a.cos())
        }
    };
    let len = (dx * dx + dy * dy).sqrt();
    // A zero-area box has no diagonal to be perpendicular to. Fall back to "to bottom",
    // CSS's own default direction, rather than emitting a NaN unit vector — a non-finite
    // gradient point would reach the rasteriser and there is no useful pixel either way.
    if !len.is_finite() || len == 0.0 {
        return (0.0, 1.0);
    }
    (dx / len, dy / len)
}

/// CSS's gradient line: through the centre of the box, long enough that the
/// perpendiculars at its two ends pass through the two corners furthest along it.
///
/// The length is `|w·dx| + |h·dy|`, which is `abs(w·sin a) + abs(h·cos a)` written in
/// terms of the unit direction. **The endpoints can lie outside the box** — for a corner
/// gradient on a non-square box they always do — and that is correct: what has to land
/// on the corners is the *perpendicular through* each endpoint, not the endpoint itself.
pub fn line_endpoints(rect: (f32, f32, f32, f32), line: GradientLine) -> (Point, Point) {
    let (x, y, w, h) = rect;
    let (dx, dy) = direction(w, h, line);
    let half = ((w as f64 * dx).abs() + (h as f64 * dy).abs()) / 2.0;
    let cx = x as f64 + w as f64 / 2.0;
    let cy = y as f64 + h as f64 / 2.0;
    (
        Point::new(cx - dx * half, cy - dy * half),
        Point::new(cx + dx * half, cy + dy * half),
    )
}

fn stop(s: GradientStop) -> ColorStop {
    ColorStop::from((s.offset, ext(s.color)))
}

/// A possibly-out-of-gamut stop colour as a `peniko::Color`.
///
/// `peniko::Color` is `AlphaColor<Srgb>`, whose components are **f32 and unclamped**, so
/// it can carry the negative channel an out-of-gamut Tailwind colour has. That is the
/// whole reason `twcore` keeps an `ExtColor` for stops: `from_rgba8` here instead would
/// clip `from-yellow-400`'s blue from -0.269 to 0 and start the ramp 68/255 off
/// Chromium's (../PLAN.md §38.3).
///
/// Measured limit, and it is vello's rather than ours: this survives an **Oklab**
/// interpolation, and is clipped by an **sRGB** one, because `vello_common` builds a u8
/// LUT for the sRGB path. See §38.3's second table.
fn ext(c: ExtColor) -> peniko::Color {
    peniko::Color::new([c.r, c.g, c.b, c.a])
}

fn space(s: GradientColorSpace) -> ColorSpaceTag {
    match s {
        GradientColorSpace::Oklab => ColorSpaceTag::Oklab,
        GradientColorSpace::Oklch => ColorSpaceTag::Oklch,
        GradientColorSpace::Srgb => ColorSpaceTag::Srgb,
    }
}

fn hue(h: GradientHue) -> HueDirection {
    match h {
        GradientHue::Shorter => HueDirection::Shorter,
        GradientHue::Longer => HueDirection::Longer,
        GradientHue::Increasing => HueDirection::Increasing,
        GradientHue::Decreasing => HueDirection::Decreasing,
    }
}

/// The resolved `twcore` gradient as a brush over `rect`.
///
/// Separate from `background_gradient` so a test can drive a gradient value directly
/// without building a whole `ComputedStyle`, and so a future `border-image`-shaped
/// consumer has the same entry point.
pub fn build(rect: (f32, f32, f32, f32), g: &TwGradient) -> Gradient {
    let (start, end) = line_endpoints(rect, g.line);
    let mut stops = ColorStops::default();
    stops.push(stop(g.from));
    if let Some(via) = g.via {
        stops.push(stop(via));
    }
    stops.push(stop(g.to));
    Gradient {
        kind: peniko::LinearGradientPosition { start, end }.into(),
        // CSS gradients hold their first and last colour beyond the stop positions,
        // which is visible whenever a stop is not at 0% or 100% (`from-10%`).
        extend: Extend::Pad,
        interpolation_cs: space(g.space),
        hue_direction: hue(g.hue),
        // CSS Color 4 § 12.3. See the module doc: a one-stop Tailwind gradient fades to
        // transparent, so this is the ordinary case.
        interpolation_alpha_space: InterpolationAlphaSpace::Premultiplied,
        stops,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use twcore::{NodeState, Resolver};

    fn resolved(classes: &str) -> std::sync::Arc<ComputedStyle> {
        let mut r = Resolver::new();
        let cs = r.resolve(classes, NodeState::default());
        assert!(r.unknown(classes).is_empty(), "unknown classes in {classes:?}");
        cs
    }

    fn approx(a: Point, b: (f64, f64)) {
        assert!(
            (a.x - b.0).abs() < 1e-6 && (a.y - b.1).abs() < 1e-6,
            "got {a:?}, want {b:?}"
        );
    }

    #[test]
    fn axis_aligned_keywords_span_the_box() {
        let rect = (0.0, 0.0, 200.0, 100.0);
        let (s, e) = line_endpoints(rect, GradientLine::ToRight);
        approx(s, (0.0, 50.0));
        approx(e, (200.0, 50.0));
        let (s, e) = line_endpoints(rect, GradientLine::ToLeft);
        approx(s, (200.0, 50.0));
        approx(e, (0.0, 50.0));
        let (s, e) = line_endpoints(rect, GradientLine::ToBottom);
        approx(s, (100.0, 0.0));
        approx(e, (100.0, 100.0));
        let (s, e) = line_endpoints(rect, GradientLine::ToTop);
        approx(s, (100.0, 100.0));
        approx(e, (100.0, 0.0));
    }

    #[test]
    fn the_endpoints_are_absolute_not_box_relative() {
        // A gradient on a node that is not at the origin must run across *that* node.
        // Getting this wrong paints every gradient in the tree with the top-left node's
        // ramp, which looks plausible on a full-screen background and wrong everywhere
        // else.
        let (s, e) = line_endpoints((40.0, 500.0, 100.0, 20.0), GradientLine::ToRight);
        approx(s, (40.0, 510.0));
        approx(e, (140.0, 510.0));
    }

    #[test]
    fn a_corner_keyword_on_a_square_runs_corner_to_corner() {
        let (s, e) = line_endpoints((0.0, 0.0, 100.0, 100.0), GradientLine::ToTopRight);
        approx(s, (0.0, 100.0));
        approx(e, (100.0, 0.0));
        let (s, e) = line_endpoints((0.0, 0.0, 100.0, 100.0), GradientLine::ToBottomLeft);
        approx(s, (100.0, 0.0));
        approx(e, (0.0, 100.0));
    }

    /// The measurement that settles the corner rule, restated as arithmetic: on a
    /// 200x100 box the `to-tr` ramp must reach 0.25 at the midline's left edge and 0.50
    /// at its centre, which is what Chromium's pixels read there (§38.2).
    #[test]
    fn a_corner_keyword_on_a_wide_box_matches_chromiums_offsets() {
        let rect = (0.0, 0.0, 200.0, 100.0);
        let (s, e) = line_endpoints(rect, GradientLine::ToTopRight);
        let offset_at = |p: (f64, f64)| {
            let d = (e.x - s.x, e.y - s.y);
            let len2 = d.0 * d.0 + d.1 * d.1;
            ((p.0 - s.x) * d.0 + (p.1 - s.y) * d.1) / len2
        };
        assert!((offset_at((0.0, 50.0)) - 0.25).abs() < 1e-6);
        assert!((offset_at((100.0, 50.0)) - 0.50).abs() < 1e-6);
        // And the two corners the perpendiculars are defined by sit exactly at the ends.
        assert!(offset_at((0.0, 100.0)).abs() < 1e-6);
        assert!((offset_at((200.0, 0.0)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn an_angle_agrees_with_the_keyword_it_names() {
        // 90deg *is* `to right`, 180deg *is* `to bottom` — a sign or an axis swap in the
        // angle formula would make one of these four disagree.
        let rect = (0.0, 0.0, 200.0, 100.0);
        for (deg, kw) in [
            (0.0, GradientLine::ToTop),
            (90.0, GradientLine::ToRight),
            (180.0, GradientLine::ToBottom),
            (270.0, GradientLine::ToLeft),
        ] {
            let (a1, a2) = line_endpoints(rect, GradientLine::Angle(deg));
            let (b1, b2) = line_endpoints(rect, kw);
            approx(a1, (b1.x, b1.y));
            approx(a2, (b2.x, b2.y));
        }
    }

    #[test]
    fn a_45_degree_line_on_a_wide_box_is_the_css_length() {
        // |w sin a| + |h cos a| = (200 + 100)/sqrt(2) = 212.13, centred on (100, 50).
        let (s, e) = line_endpoints((0.0, 0.0, 200.0, 100.0), GradientLine::Angle(45.0));
        let len = ((e.x - s.x).powi(2) + (e.y - s.y).powi(2)).sqrt();
        assert!((len - 300.0 / 2.0_f64.sqrt()).abs() < 1e-6, "len {len}");
        approx(s, (25.0, 125.0));
        approx(e, (175.0, -25.0));
    }

    #[test]
    fn a_zero_area_box_produces_finite_points() {
        // A corner keyword divides by the box diagonal. `h-0` is reachable from ordinary
        // layout (an empty flex child), and a NaN point would reach the rasteriser.
        for line in [GradientLine::ToTopRight, GradientLine::ToBottomLeft, GradientLine::Angle(45.0)] {
            let (s, e) = line_endpoints((10.0, 10.0, 0.0, 0.0), line);
            assert!(s.x.is_finite() && s.y.is_finite() && e.x.is_finite() && e.y.is_finite());
        }
    }

    #[test]
    fn the_brush_carries_the_three_fields_a_consumer_would_forget() {
        // The module doc's whole argument. `peniko::Gradient::default()` uses sRGB, so
        // an omitted `interpolation_cs` is a wrong ramp, not a missing one.
        let cs = resolved("bg-linear-to-r from-cyan-500 to-blue-500");
        let g = background_gradient((0.0, 0.0, 200.0, 100.0), &cs).unwrap();
        assert_eq!(g.interpolation_cs, ColorSpaceTag::Oklab);
        assert_eq!(g.interpolation_alpha_space, InterpolationAlphaSpace::Premultiplied);
        assert_eq!(g.extend, Extend::Pad);
        assert_eq!(g.stops.len(), 2);
        assert_eq!(g.stops[0].offset, 0.0);
        assert_eq!(g.stops[1].offset, 1.0);
    }

    #[test]
    fn the_interpolation_modifiers_reach_the_brush() {
        let cs = resolved("bg-linear-to-r/srgb from-yellow-400 to-blue-600");
        let g = background_gradient((0.0, 0.0, 10.0, 10.0), &cs).unwrap();
        assert_eq!(g.interpolation_cs, ColorSpaceTag::Srgb);

        let cs = resolved("bg-linear-to-r/longer from-cyan-500 to-blue-500");
        let g = background_gradient((0.0, 0.0, 10.0, 10.0), &cs).unwrap();
        assert_eq!(g.interpolation_cs, ColorSpaceTag::Oklch);
        assert_eq!(g.hue_direction, HueDirection::Longer);
    }

    #[test]
    fn via_becomes_a_third_stop_in_order() {
        let cs = resolved("bg-linear-to-r from-cyan-500 via-fuchsia-500 via-30% to-blue-500");
        let g = background_gradient((0.0, 0.0, 10.0, 10.0), &cs).unwrap();
        assert_eq!(g.stops.len(), 3);
        assert_eq!(g.stops[1].offset, 0.3);
    }

    #[test]
    fn a_node_with_no_gradient_class_has_no_brush() {
        // The negative control for every paint site: `if let Some(g) = …` must not fire
        // for the overwhelming majority of nodes, or every solid background in the tree
        // becomes a two-stop gradient of itself.
        assert!(background_gradient((0.0, 0.0, 10.0, 10.0), &resolved("bg-slate-950")).is_none());
        assert!(background_gradient((0.0, 0.0, 10.0, 10.0), &resolved("flex p-4")).is_none());
        assert!(
            background_gradient((0.0, 0.0, 10.0, 10.0), &resolved("from-cyan-500 to-blue-500"))
                .is_none(),
            "stops with no line paint nothing — twcore's rule, measured in Chromium"
        );
    }

    #[test]
    fn a_one_stop_gradient_fades_to_transparent() {
        let cs = resolved("bg-linear-to-r from-cyan-500");
        let g = background_gradient((0.0, 0.0, 10.0, 10.0), &cs).unwrap();
        assert_eq!(g.stops.len(), 2);
        let last = g.stops[1].color.to_alpha_color::<peniko::color::Srgb>();
        assert_eq!(last.components[3], 0.0, "the far stop is transparent");
    }
}
