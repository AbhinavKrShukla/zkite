//! The gradient utilities, pinned against what Tailwind v4.3.3 actually emits.
//!
//! phase_3/PLAN.md §38. Every expectation here was measured in real Chromium against
//! the exact `@tailwindcss/browser` bundle `phase_1/webruntime/vendor/` pins, by
//! `phase_1/webruntime/test/probe_gradients.mjs` — not read from documentation. The
//! probe needs Chromium and Playwright, so it guards nothing on its own; this file is
//! the guard, and the comments name the measured value so a future re-vendor can
//! re-derive them.
//!
//! The four rules that are easy to get wrong, and are all measured:
//!
//! 1. A gradient needs a **line and at least one colour stop**. Either alone renders
//!    `background-image: none`, and so does a line plus a bare *position*.
//! 2. A declared slot with no colour is **transparent**, so a one-stop gradient fades
//!    out rather than being a solid fill.
//! 3. `background-color` and the gradient **coexist** — the gradient is
//!    `background-image` and paints over the colour.
//! 4. The default interpolation space is **oklab**, and every hue-direction modifier
//!    implies **oklch**.

use twcore::style::{
    Color, ExtColor, GradientColorSpace as Space, GradientHue as Hue, GradientLine as Line,
};
use twcore::{NodeState, Resolver};

fn s(res: &mut Resolver, classes: &str) -> std::sync::Arc<twcore::ComputedStyle> {
    let out = res.resolve(classes, NodeState::default());
    assert!(
        res.unknown(classes).is_empty(),
        "unrecognised classes in {classes:?}: {:?}",
        res.unknown(classes)
    );
    out
}

/// The palette entries the probe used, as `twcore` resolves them — in the **clipped**
/// 8-bit form, which is what a solid `bg-*` paints and what Chromium was confirmed
/// against: cyan-500 `#00b8db`, blue-500 `#2b7fff`.
///
/// A gradient stop is compared through `clipped()` below rather than directly, because a
/// stop carries the *unclipped* colour (`ExtColor`) and clipping it is exactly the
/// information loss §38.3 is about. Asserting on the clipped form keeps these tests about
/// *which colour reached the slot*; `stop_colours_are_unclipped` is the test that pins the
/// other half.
const CYAN_500: Color = Color::rgb(0x00b8db);
const BLUE_500: Color = Color::rgb(0x2b7fff);
const FUCHSIA_500: Color = Color::rgb(0xe12afb);

fn clipped(c: ExtColor) -> Color {
    c.to_srgb8()
}

#[test]
fn the_canonical_two_stop_gradient() {
    let mut r = Resolver::new();
    let g = s(&mut r, "bg-linear-to-r from-cyan-500 to-blue-500")
        .paint
        .gradient
        .resolve()
        .expect("a line plus two colour stops is a gradient");
    assert_eq!(g.line, Line::ToRight);
    assert_eq!(clipped(g.from.color), CYAN_500);
    assert_eq!(g.from.offset, 0.0);
    assert_eq!(clipped(g.to.color), BLUE_500);
    assert_eq!(g.to.offset, 1.0);
    assert!(g.via.is_none());
    // Measured: `--tw-gradient-position: to right in oklab`.
    assert_eq!(g.space, Space::Oklab);
    assert_eq!(g.hue, Hue::Shorter);
}

#[test]
fn all_eight_keyword_directions_in_both_spellings() {
    let mut r = Resolver::new();
    // Chromium's computed `background-image` for the v3 spelling is byte-identical to
    // the v4 one, measured for `to-r`; v3's whole point is that these are the same
    // utility under two names.
    let expect = [
        ("to-t", Line::ToTop),
        ("to-r", Line::ToRight),
        ("to-b", Line::ToBottom),
        ("to-l", Line::ToLeft),
        ("to-tr", Line::ToTopRight),
        ("to-tl", Line::ToTopLeft),
        ("to-br", Line::ToBottomRight),
        ("to-bl", Line::ToBottomLeft),
    ];
    for (tok, want) in expect {
        for head in ["bg-linear", "bg-gradient"] {
            let classes = format!("{head}-{tok} from-cyan-500");
            let g = s(&mut r, &classes).paint.gradient.resolve().expect(&classes);
            assert_eq!(g.line, want, "{classes}");
        }
    }
}

#[test]
fn via_adds_a_middle_stop_at_fifty_percent() {
    let mut r = Resolver::new();
    let g = s(&mut r, "bg-linear-to-r from-cyan-500 via-fuchsia-500 to-blue-500")
        .paint
        .gradient
        .resolve()
        .unwrap();
    let via = g.via.expect("via-* creates a third stop");
    assert_eq!(clipped(via.color), FUCHSIA_500);
    // Measured: `linear-gradient(to right, … 0%, … 50%, … 100%)`.
    assert_eq!(via.offset, 0.5);
}

#[test]
fn via_alone_fades_out_at_both_ends() {
    // Measured: `linear-gradient(to right, rgba(0,0,0,0) 0%, <fuchsia> 50%,
    // rgba(0,0,0,0) 100%)`. This is rule 2 in the module doc, at both ends at once.
    let mut r = Resolver::new();
    let g = s(&mut r, "bg-linear-to-r via-fuchsia-500").paint.gradient.resolve().unwrap();
    assert_eq!(clipped(g.from.color), Color::TRANSPARENT);
    assert_eq!(clipped(g.via.unwrap().color), FUCHSIA_500);
    assert_eq!(clipped(g.to.color), Color::TRANSPARENT);
}

#[test]
fn explicit_stop_positions() {
    let mut r = Resolver::new();
    let g = s(&mut r, "bg-linear-to-r from-cyan-500 from-10% to-blue-500 to-90%")
        .paint
        .gradient
        .resolve()
        .unwrap();
    // Measured: `… oklch(…) 10%, oklch(…) 90%`.
    assert_eq!(g.from.offset, 0.1);
    assert_eq!(g.to.offset, 0.9);

    let g = s(&mut r, "bg-linear-to-r from-cyan-500 via-fuchsia-500 via-30% to-blue-500")
        .paint
        .gradient
        .resolve()
        .unwrap();
    assert_eq!(g.via.unwrap().offset, 0.3);
    // The other two keep their defaults — v4 sets each position variable
    // independently, so naming one must not move the others.
    assert_eq!(g.from.offset, 0.0);
    assert_eq!(g.to.offset, 1.0);
}

#[test]
fn angles_in_both_forms() {
    let mut r = Resolver::new();
    let bare = s(&mut r, "bg-linear-45 from-cyan-500").paint.gradient.resolve().unwrap();
    assert_eq!(bare.line, Line::Angle(45.0));
    let arbitrary = s(&mut r, "bg-linear-[30deg] from-cyan-500").paint.gradient.resolve().unwrap();
    assert_eq!(arbitrary.line, Line::Angle(30.0));
    // The arbitrary form emits a bare `30deg` with no `in oklab` (measured), and CSS
    // Images 4's default for a gradient with no interpolation keyword is oklab — so the
    // rendered ramp is the same and this must not fall back to sRGB.
    assert_eq!(arbitrary.space, Space::Oklab);
}

#[test]
fn interpolation_modifiers() {
    let mut r = Resolver::new();
    let cases = [
        ("bg-linear-to-r/srgb", Space::Srgb, Hue::Shorter),
        ("bg-linear-to-r/oklab", Space::Oklab, Hue::Shorter),
        ("bg-linear-to-r/oklch", Space::Oklch, Hue::Shorter),
        // Measured: `/longer` compiles to `to right in oklch longer hue` — the hue
        // modifiers change the *space* as well as the direction.
        ("bg-linear-to-r/shorter", Space::Oklch, Hue::Shorter),
        ("bg-linear-to-r/longer", Space::Oklch, Hue::Longer),
        ("bg-linear-to-r/increasing", Space::Oklch, Hue::Increasing),
        ("bg-linear-to-r/decreasing", Space::Oklch, Hue::Decreasing),
    ];
    for (head, space, hue) in cases {
        let classes = format!("{head} from-cyan-500 to-blue-500");
        let g = s(&mut r, &classes).paint.gradient.resolve().unwrap();
        assert_eq!(g.space, space, "{classes}");
        assert_eq!(g.hue, hue, "{classes}");
    }
}

#[test]
fn a_stop_takes_an_opacity_modifier_and_an_arbitrary_colour() {
    let mut r = Resolver::new();
    let g = s(&mut r, "bg-linear-to-r from-cyan-500/50 to-blue-500").paint.gradient.resolve().unwrap();
    assert_eq!(clipped(g.from.color), Color { r: 0x00, g: 0xb8, b: 0xdb, a: 128 });

    let g = s(&mut r, "bg-linear-to-r from-[#ff0000] to-[#0000ff]").paint.gradient.resolve().unwrap();
    assert_eq!(clipped(g.from.color), Color::rgb(0xff0000));
    assert_eq!(clipped(g.to.color), Color::rgb(0x0000ff));
}

// ------------------------------------------------------------------ rule 1

#[test]
fn stops_without_a_line_paint_nothing() {
    // Measured: `from-cyan-500 to-blue-500` → `background-image: none`. The custom
    // properties are set and the `background-image` declaration does not exist, so
    // there is nothing to draw. A resolver that painted a default-direction gradient
    // here would render a gradient the WebView backend does not.
    let mut r = Resolver::new();
    let cs = s(&mut r, "from-cyan-500 to-blue-500");
    assert!(cs.paint.gradient.resolve().is_none());
    // …but the stops are *recorded*, because a later `bg-linear-to-r` in a variant, or
    // in a class string built by concatenation, must still find them.
    assert_eq!(cs.paint.gradient.from.map(clipped), Some(CYAN_500));
}

#[test]
fn a_line_without_a_colour_stop_paints_nothing() {
    // Measured, twice over: `bg-linear-to-r` alone is `background-image: none`, and so
    // is `bg-linear-to-r from-10%`. Tailwind's `background-image` refers to
    // `var(--tw-gradient-stops)`, which only a *colour* utility defines; an undefined
    // var makes the whole declaration invalid at computed-value time.
    let mut r = Resolver::new();
    assert!(s(&mut r, "bg-linear-to-r").paint.gradient.resolve().is_none());
    assert!(s(&mut r, "bg-linear-to-r from-10%").paint.gradient.resolve().is_none());
    // The position is still recorded, for the same reason as above.
    assert_eq!(s(&mut r, "bg-linear-to-r from-10%").paint.gradient.from_offset, Some(0.1));
}

#[test]
fn one_colour_stop_is_enough_and_the_far_end_is_transparent() {
    // Measured: `bg-linear-to-r from-cyan-500` →
    // `linear-gradient(to right, <cyan> 0%, rgba(0,0,0,0) 100%)`.
    let mut r = Resolver::new();
    let g = s(&mut r, "bg-linear-to-r from-cyan-500").paint.gradient.resolve().unwrap();
    assert_eq!(clipped(g.from.color), CYAN_500);
    assert_eq!(clipped(g.to.color), Color::TRANSPARENT);

    let g = s(&mut r, "bg-linear-to-r to-blue-500").paint.gradient.resolve().unwrap();
    assert_eq!(clipped(g.from.color), Color::TRANSPARENT);
    assert_eq!(clipped(g.to.color), BLUE_500);
}

#[test]
fn the_utilities_are_order_independent() {
    // Tailwind compiles these to custom properties, so class order cannot matter. In
    // `twcore` the declarations apply in class order, which is why this needs asserting
    // rather than arguing: a model that only recorded stops *after* a line was seen
    // would work in one order and silently not in the other.
    let mut r = Resolver::new();
    let a = s(&mut r, "bg-linear-to-r from-cyan-500 to-blue-500").paint.gradient.resolve();
    let b = s(&mut r, "from-cyan-500 to-blue-500 bg-linear-to-r").paint.gradient.resolve();
    assert_eq!(a, b);
    assert!(a.is_some());
}

// ------------------------------------------------------------------ rule 3

#[test]
fn a_background_colour_and_a_gradient_coexist() {
    // Measured: `bg-red-500 bg-linear-to-r from-cyan-500` keeps
    // `background-color: oklch(0.637 …)` *and* paints the gradient over it — the right
    // edge of the swatch reads (251, 45, 55), i.e. red-500 through the transparent end.
    let mut r = Resolver::new();
    let cs = s(&mut r, "bg-red-500 bg-linear-to-r from-cyan-500");
    assert_eq!(cs.paint.background, Some(Color::rgb(0xfb2c36)));
    assert!(cs.paint.gradient.resolve().is_some());
}

#[test]
fn bg_none_clears_the_line_but_keeps_the_stops() {
    // Measured: `… from-cyan-500 to-blue-500 bg-none` → `background-image: none` with
    // `--tw-gradient-stops` still set.
    let mut r = Resolver::new();
    let cs = s(&mut r, "bg-linear-to-r from-cyan-500 to-blue-500 bg-none");
    assert!(cs.paint.gradient.resolve().is_none());
    assert_eq!(cs.paint.gradient.from.map(clipped), Some(CYAN_500));
}

// ------------------------------------------------------------------ controls

#[test]
fn the_unparsed_families_are_reported_as_unknown_rather_than_ignored() {
    // `bg-radial`, `bg-conic` and a mistyped direction or modifier must all reach the
    // unknown-class report, because that is the only thing standing between a user and
    // a silently blank background. §38.5 files the two families that are deliberately
    // not implemented; this test is what keeps them *loud* while they are missing.
    let mut r = Resolver::new();
    for cls in [
        "bg-radial",
        "bg-conic",
        "bg-conic-180",
        "bg-linear-to-nowhere",
        "bg-linear-to-r/wat",
        "bg-gradient-45",
        "from-notacolour",
        "via-notacolour",
        "to-notacolour",
    ] {
        r.resolve(cls, NodeState::default());
        assert!(!r.unknown(cls).is_empty(), "{cls:?} should be reported as unknown");
    }
}

#[test]
fn stop_colours_are_unclipped_and_a_solid_fill_is_still_clipped() {
    // The §38.3 finding, pinned from both ends. `yellow-400` is `oklch(85.2% 0.199
    // 91.936)`, which is outside the sRGB gamut: its blue is **-0.269** in encoded sRGB.
    //
    // A solid `bg-yellow-400` must still be the clipped `#fdc700` — that is what a
    // browser paints — while `from-yellow-400` must keep the negative channel, because
    // Chromium interpolates the unclipped value and clips only at display time. Feeding
    // the clipped value to the interpolation was measured at 68/255 off Chromium.
    let mut r = Resolver::new();
    let cs = s(&mut r, "bg-yellow-400 bg-linear-to-r from-yellow-400 to-blue-600");
    assert_eq!(cs.paint.background, Some(Color::rgb(0xfdc700)));

    let g = cs.paint.gradient.resolve().unwrap();
    assert!(
        (g.from.color.b - -0.269112).abs() < 1e-5,
        "yellow-400's blue must survive as {}, got {}",
        -0.269112,
        g.from.color.b
    );
    // And the in-gamut channels are the same colour by another route: clipping the stop
    // reproduces the palette hex exactly.
    assert_eq!(clipped(g.from.color), Color::rgb(0xfdc700));

    // An arbitrary hex stop is in gamut by construction and must round-trip exactly.
    let g = s(&mut r, "bg-linear-to-r from-[#ff8000]").paint.gradient.resolve().unwrap();
    assert_eq!(clipped(g.from.color), Color::rgb(0xff8000));
    assert!(g.from.color.r <= 1.0 && g.from.color.b >= 0.0);
}

#[test]
fn a_gradient_class_does_not_disturb_a_plain_background() {
    // The control for the whole feature: nothing about a node with no gradient class
    // may change. `Paint::default()`'s new field is the only thing that moved.
    let mut r = Resolver::new();
    let cs = s(&mut r, "bg-slate-950 rounded-lg p-4");
    assert_eq!(cs.paint.background, Some(Color::rgb(0x020618)));
    assert!(cs.paint.gradient.resolve().is_none());
    assert_eq!(cs.paint.gradient, twcore::style::GradientSpec::default());
}
