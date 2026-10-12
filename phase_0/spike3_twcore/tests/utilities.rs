//! Spike 3 pass criterion: "parse 40 common utilities into a `taffy::Style` + paint struct."
//!
//! `covered_utility_families` is the count that criterion is judged against — one entry
//! per distinct utility family, not per class string.

use taffy::prelude::*;
use taffy::style::{Display, FlexDirection, FlexWrap, Overflow, Position};
use twcore::style::{FontFamily, LineHeight, TextAlignment};
use twcore::{NodeState, PaintColor, Resolver};

fn r() -> Resolver {
    Resolver::new()
}

fn s(res: &mut Resolver, classes: &str) -> std::sync::Arc<twcore::ComputedStyle> {
    let out = res.resolve(classes, NodeState::default());
    assert!(
        res.unknown(classes).is_empty(),
        "unrecognised classes in {classes:?}: {:?}",
        res.unknown(classes)
    );
    out
}

// ---------------------------------------------------------------- layout

#[test]
fn display_family() {
    let mut r = r();
    assert_eq!(s(&mut r, "flex").layout.display, Display::Flex);
    assert_eq!(s(&mut r, "grid").layout.display, Display::Grid);
    assert_eq!(s(&mut r, "block").layout.display, Display::Block);
    assert_eq!(s(&mut r, "hidden").layout.display, Display::None);
}

#[test]
fn flex_direction_and_wrap() {
    let mut r = r();
    assert_eq!(s(&mut r, "flex-col").layout.flex_direction, FlexDirection::Column);
    assert_eq!(s(&mut r, "flex-row").layout.flex_direction, FlexDirection::Row);
    assert_eq!(
        s(&mut r, "flex-col-reverse").layout.flex_direction,
        FlexDirection::ColumnReverse
    );
    assert_eq!(s(&mut r, "flex-wrap").layout.flex_wrap, FlexWrap::Wrap);
    assert_eq!(s(&mut r, "flex-nowrap").layout.flex_wrap, FlexWrap::NoWrap);
}

#[test]
fn flex_item_shorthands() {
    let mut r = r();
    let one = s(&mut r, "flex-1");
    assert_eq!(one.layout.flex_grow, 1.0);
    assert_eq!(one.layout.flex_shrink, 1.0);
    assert_eq!(one.layout.flex_basis, Dimension::percent(0.0));

    let none = s(&mut r, "flex-none");
    assert_eq!(none.layout.flex_grow, 0.0);
    assert_eq!(none.layout.flex_shrink, 0.0);

    assert_eq!(s(&mut r, "grow").layout.flex_grow, 1.0);
    assert_eq!(s(&mut r, "grow-0").layout.flex_grow, 0.0);
    assert_eq!(s(&mut r, "shrink-0").layout.flex_shrink, 0.0);
    assert_eq!(s(&mut r, "basis-1/2").layout.flex_basis, Dimension::percent(0.5));
}

#[test]
fn alignment_family() {
    let mut r = r();
    assert_eq!(s(&mut r, "items-center").layout.align_items, Some(AlignItems::Center));
    assert_eq!(s(&mut r, "items-start").layout.align_items, Some(AlignItems::FlexStart));
    assert_eq!(
        s(&mut r, "justify-between").layout.justify_content,
        Some(JustifyContent::SpaceBetween)
    );
    assert_eq!(
        s(&mut r, "justify-center").layout.justify_content,
        Some(JustifyContent::Center)
    );
    assert_eq!(s(&mut r, "self-end").layout.align_self, Some(AlignSelf::FlexEnd));
    assert_eq!(
        s(&mut r, "content-around").layout.align_content,
        Some(AlignContent::SpaceAround)
    );
}

#[test]
fn spacing_scale_is_four_dp_per_unit() {
    let mut r = r();
    // Tailwind's scale: n * 4px.
    assert_eq!(s(&mut r, "p-4").layout.padding.left, LengthPercentage::length(16.0));
    assert_eq!(s(&mut r, "p-0.5").layout.padding.top, LengthPercentage::length(2.0));
    assert_eq!(s(&mut r, "p-px").layout.padding.top, LengthPercentage::length(1.0));

    let px = s(&mut r, "px-6");
    assert_eq!(px.layout.padding.left, LengthPercentage::length(24.0));
    assert_eq!(px.layout.padding.right, LengthPercentage::length(24.0));
    assert_eq!(px.layout.padding.top, LengthPercentage::length(0.0));

    let py = s(&mut r, "py-3");
    assert_eq!(py.layout.padding.top, LengthPercentage::length(12.0));
    assert_eq!(py.layout.padding.bottom, LengthPercentage::length(12.0));

    assert_eq!(s(&mut r, "pt-2").layout.padding.top, LengthPercentage::length(8.0));
    assert_eq!(s(&mut r, "pl-8").layout.padding.left, LengthPercentage::length(32.0));
}

#[test]
fn margins_including_auto_and_negative() {
    let mut r = r();
    assert_eq!(s(&mut r, "m-4").layout.margin.top, LengthPercentageAuto::length(16.0));
    assert_eq!(s(&mut r, "mx-auto").layout.margin.left, LengthPercentageAuto::auto());
    assert_eq!(s(&mut r, "mx-auto").layout.margin.top, LengthPercentageAuto::length(0.0));
    assert_eq!(s(&mut r, "-mt-4").layout.margin.top, LengthPercentageAuto::length(-16.0));
    assert_eq!(s(&mut r, "mb-2").layout.margin.bottom, LengthPercentageAuto::length(8.0));
}

#[test]
fn gap_family() {
    let mut r = r();
    let g = s(&mut r, "gap-6");
    assert_eq!(g.layout.gap.width, LengthPercentage::length(24.0));
    assert_eq!(g.layout.gap.height, LengthPercentage::length(24.0));

    let gx = s(&mut r, "gap-x-2 gap-y-8");
    assert_eq!(gx.layout.gap.width, LengthPercentage::length(8.0));
    assert_eq!(gx.layout.gap.height, LengthPercentage::length(32.0));
}

#[test]
fn sizing_family() {
    let mut r = r();
    assert_eq!(s(&mut r, "w-full").layout.size.width, Dimension::percent(1.0));
    assert_eq!(s(&mut r, "w-1/2").layout.size.width, Dimension::percent(0.5));
    assert_eq!(s(&mut r, "w-24").layout.size.width, Dimension::length(96.0));
    assert_eq!(s(&mut r, "h-auto").layout.size.height, Dimension::auto());
    assert_eq!(s(&mut r, "size-10").layout.size.height, Dimension::length(40.0));
    assert_eq!(s(&mut r, "min-h-screen").layout.min_size.height, Dimension::percent(1.0));
    assert_eq!(s(&mut r, "max-w-full").layout.max_size.width, Dimension::percent(1.0));
    assert_eq!(s(&mut r, "min-w-0").layout.min_size.width, Dimension::length(0.0));
}

#[test]
fn position_and_inset() {
    let mut r = r();
    assert_eq!(s(&mut r, "absolute").layout.position, Position::Absolute);
    assert_eq!(s(&mut r, "relative").layout.position, Position::Relative);

    let inset = s(&mut r, "absolute inset-0");
    assert_eq!(inset.layout.inset.left, LengthPercentageAuto::length(0.0));
    assert_eq!(inset.layout.inset.bottom, LengthPercentageAuto::length(0.0));

    assert_eq!(s(&mut r, "top-4").layout.inset.top, LengthPercentageAuto::length(16.0));
    assert_eq!(
        s(&mut r, "-left-2").layout.inset.left,
        LengthPercentageAuto::length(-8.0)
    );
    let ix = s(&mut r, "inset-x-3");
    assert_eq!(ix.layout.inset.left, LengthPercentageAuto::length(12.0));
    assert_eq!(ix.layout.inset.top, LengthPercentageAuto::auto());
}

#[test]
fn overflow_family() {
    let mut r = r();
    assert_eq!(s(&mut r, "overflow-hidden").layout.overflow.x, Overflow::Hidden);
    assert_eq!(s(&mut r, "overflow-scroll").layout.overflow.y, Overflow::Scroll);
    let y = s(&mut r, "overflow-y-scroll");
    assert_eq!(y.layout.overflow.y, Overflow::Scroll);
    assert_eq!(y.layout.overflow.x, Overflow::Visible);
}

#[test]
fn grid_family() {
    let mut r = r();
    let g = s(&mut r, "grid grid-cols-3");
    assert_eq!(g.layout.display, Display::Grid);
    assert_eq!(g.layout.grid_template_columns.len(), 3);
    assert_eq!(s(&mut r, "grid-rows-2").layout.grid_template_rows.len(), 2);
    assert_eq!(s(&mut r, "col-span-2").layout.grid_column.end, span(2));
}

#[test]
fn aspect_ratio_family() {
    let mut r = r();
    assert_eq!(s(&mut r, "aspect-square").layout.aspect_ratio, Some(1.0));
    assert_eq!(s(&mut r, "aspect-video").layout.aspect_ratio, Some(16.0 / 9.0));
}

// ---------------------------------------------------------------- paint

#[test]
fn background_colors_match_upstream_tailwind() {
    let mut r = r();
    assert_eq!(s(&mut r, "bg-indigo-500").paint.background.unwrap().to_rgba_u32(), 0x615fffff);
    assert_eq!(s(&mut r, "bg-slate-950").paint.background.unwrap().to_rgba_u32(), 0x020618ff);
    assert_eq!(s(&mut r, "bg-white").paint.background.unwrap().to_rgba_u32(), 0xffffffff);
    assert_eq!(s(&mut r, "bg-transparent").paint.background.unwrap().to_rgba_u32(), 0x00000000);
}

#[test]
fn alpha_modifier() {
    let mut r = r();
    let c = s(&mut r, "bg-black/50").paint.background.unwrap();
    assert_eq!(c.a, 128);
    assert_eq!((c.r, c.g, c.b), (0, 0, 0));
}

#[test]
fn border_width_and_color() {
    let mut r = r();
    let b = s(&mut r, "border");
    assert_eq!(b.layout.border.top, LengthPercentage::length(1.0));
    // A bare `border` must still be visible, and in **v4** the colour it defaults to is
    // `currentColor` — v4's preflight is `border: 0 solid`, with no colour, where v3's
    // named `#e5e7eb`. This assertion used to be that gray-200 literal. Measured in
    // real Chromium against the vendored bundle; see the dedicated test below.
    assert_eq!(b.paint.border_color.uniform().unwrap(), PaintColor::Current);

    let b2 = s(&mut r, "border-2 border-red-500");
    assert_eq!(b2.layout.border.left, LengthPercentage::length(2.0));
    assert_eq!(
        b2.paint.border_color.uniform().unwrap().resolve(None).to_rgba_u32(),
        0xfb2c36ff
    );

    let bt = s(&mut r, "border-t-4");
    assert_eq!(bt.layout.border.top, LengthPercentage::length(4.0));
    assert_eq!(bt.layout.border.bottom, LengthPercentage::length(0.0));
}

/// The three Tailwind **v4** defaults that this crate had wrong from v3, each measured
/// in real Chromium against `phase_1/webruntime/vendor/tailwind-browser.js` — the same
/// bundle the WebView backend actually renders with. See phase_3/PLAN.md §28.1.
///
/// These are the *semantics* rather than the table, which is why they get their own
/// test: §9.6 already updated the blue-500 hex literal that `ring` used to default to,
/// and that fixed the wrong half — v4 does not default a ring to blue-500 at all.
#[test]
fn v4_defaults_for_border_and_ring_are_currentcolor_not_v3s_named_colours() {
    let mut r = r();

    // `border-2` -> Chromium computes `border-top-color: rgb(0, 0, 0)`;
    // `border-2 text-red-500` -> it computes the red. i.e. it tracks `color`.
    let plain = s(&mut r, "border-2");
    assert_eq!(plain.paint.border_color.top.unwrap(), PaintColor::Current);
    assert_eq!(plain.paint.border_color.top.unwrap().resolve(None).to_rgba_u32(), 0x000000ff);
    let inherited = twcore::Color::rgb(0xfb2c36);
    assert_eq!(
        plain.paint.border_color.top.unwrap().resolve(Some(inherited)),
        inherited,
        "a bare border must take the node's own resolved text colour"
    );

    // A bare `ring` -> Chromium computes `box-shadow: rgb(0, 0, 0) 0 0 0 1px`.
    // Both numbers changed from v3, which was 3px and blue-500.
    let ring = s(&mut r, "ring").paint.ring.unwrap();
    assert_eq!(ring.0, 1.0, "v4's --default-ring-width is 1px; v3's was 3px");
    assert_eq!(ring.1, PaintColor::Current, "v4's default ring colour is currentColor");

    // An explicit ring colour with no explicit width still picks up the *new* default
    // width — the path that used to hardcode 3.0.
    let coloured = s(&mut r, "ring-indigo-500").paint.ring.unwrap();
    assert_eq!(coloured.0, 1.0);
    assert_eq!(coloured.1.resolve(None).to_rgba_u32(), 0x615fffff);
}

#[test]
fn radius_family() {
    let mut r = r();
    assert_eq!(s(&mut r, "rounded").paint.radius.top_left, 4.0);
    assert_eq!(s(&mut r, "rounded-xl").paint.radius.top_left, 12.0);
    assert_eq!(s(&mut r, "rounded-2xl").paint.radius.bottom_right, 16.0);
    assert_eq!(s(&mut r, "rounded-full").paint.radius.top_right, 9999.0);

    let t = s(&mut r, "rounded-t-lg");
    assert_eq!(t.paint.radius.top_left, 8.0);
    assert_eq!(t.paint.radius.bottom_left, 0.0);
    assert!(!t.paint.radius.is_uniform());
}

#[test]
fn the_radius_scale_is_v4s_and_not_v3s() {
    // phase_3/PLAN.md §36. v4 shifted every name in the scale one step, exactly as it
    // did to the shadows in §28.1, and this table was still v3's: `rounded-sm` was
    // 2dp where the WebView backend draws 4dp, and the two names v4 added at either
    // end did not parse at all. Every number below was read out of real Chromium
    // driving the vendored bundle — `phase_1/webruntime/test/probe_theme_defaults.mjs`.
    //
    // This is the guard, not the probe: the probe needs Chromium and Playwright, so it
    // is a thing someone runs when re-vendoring Tailwind. This runs in `cargo test`.
    let mut r = r();
    for (cls, expected) in [
        ("rounded-none", 0.0),
        ("rounded-xs", 2.0), // v4-only; was unparsed, so the corner came out square
        ("rounded-sm", 4.0), // v3 said 2.0 — the one that rendered confidently wrong
        ("rounded", 4.0),
        ("rounded-md", 6.0),
        ("rounded-lg", 8.0),
        ("rounded-xl", 12.0),
        ("rounded-2xl", 16.0),
        ("rounded-3xl", 24.0),
        ("rounded-4xl", 32.0), // v4-only; was unparsed
    ] {
        let st = s(&mut r, cls);
        assert_eq!(st.paint.radius.top_left, expected, "{cls}");
        assert!(st.paint.radius.is_uniform(), "{cls} must set all four corners");
    }

    // The two new names have to reach the per-corner path too — `Theme::radius` is
    // the only lookup, but that is a property of `parse.rs` worth pinning rather than
    // assuming, since a future corner table could grow its own scale.
    assert_eq!(s(&mut r, "rounded-tl-xs").paint.radius.top_left, 2.0);
    assert_eq!(s(&mut r, "rounded-b-4xl").paint.radius.bottom_right, 32.0);
}

#[test]
fn opacity_shadow_ring() {
    let mut r = r();
    assert_eq!(s(&mut r, "opacity-50").paint.opacity, 0.5);
    assert!(s(&mut r, "shadow-none").paint.shadow.is_none());
    let sh = s(&mut r, "shadow-lg").paint.shadow.unwrap();
    assert_eq!(sh.layers()[0].offset_y, 10.0);
    assert_eq!(sh.layers()[0].blur, 15.0);

    let ring = s(&mut r, "ring-2 ring-indigo-500").paint.ring.unwrap();
    assert_eq!(ring.0, 2.0);
    assert_eq!(ring.1.resolve(None).to_rgba_u32(), 0x615fffff);
}

/// v4 **renamed the whole shadow scale** and made most of it two-layer. Both are
/// invisible until something paints a shadow, which is why this went unnoticed for
/// three phases and why §9.6's own known-gaps entry predicted it
/// ("v4 also changed defaults that are not colours ... `shadow-*` values").
///
/// Every value here is what real Chromium reports as the computed `box-shadow` for the
/// vendored bundle. phase_3/PLAN.md §28.1.
#[test]
fn v4_shadow_scale_is_renamed_and_mostly_two_layer() {
    let mut r = r();
    let layers = |res: &mut twcore::Resolver, cls: &str| {
        s(res, cls).paint.shadow.unwrap().layers().to_vec()
    };

    // The rename. v4's `xs` is v3's `sm`; v4's `sm` is v3's bare `shadow`.
    let xs = layers(&mut r, "shadow-xs");
    assert_eq!(xs.len(), 1);
    assert_eq!((xs[0].offset_y, xs[0].blur, xs[0].spread), (1.0, 2.0, 0.0));
    assert_eq!(xs[0].color.a, 13, "rgb(0 0 0 / 0.05)");

    let sm = layers(&mut r, "shadow-sm");
    assert_eq!(sm.len(), 2, "v4's shadow-sm is two layers; this used to return one");
    assert_eq!((sm[0].offset_y, sm[0].blur, sm[0].spread), (1.0, 3.0, 0.0));
    assert_eq!((sm[1].offset_y, sm[1].blur, sm[1].spread), (1.0, 2.0, -1.0));

    // A bare `shadow` is an alias of `sm`, not its own value.
    assert_eq!(layers(&mut r, "shadow"), sm);

    // `2xs` did not exist in v3's table here at all.
    let xxs = layers(&mut r, "shadow-2xs");
    assert_eq!(xxs.len(), 1);
    assert_eq!((xxs[0].offset_y, xxs[0].blur), (1.0, 0.0), "2xs has no blur at all");

    for (cls, first, second) in [
        ("shadow-md", (4.0, 6.0, -1.0), (2.0, 4.0, -2.0)),
        ("shadow-lg", (10.0, 15.0, -3.0), (4.0, 6.0, -4.0)),
        ("shadow-xl", (20.0, 25.0, -5.0), (8.0, 10.0, -6.0)),
    ] {
        let l = layers(&mut r, cls);
        assert_eq!(l.len(), 2, "{cls} is two layers in v4");
        assert_eq!((l[0].offset_y, l[0].blur, l[0].spread), first, "{cls} layer 0");
        assert_eq!((l[1].offset_y, l[1].blur, l[1].spread), second, "{cls} layer 1");
    }

    // 2xl is the one that genuinely is a single layer.
    let xxl = layers(&mut r, "shadow-2xl");
    assert_eq!(xxl.len(), 1);
    assert_eq!((xxl[0].offset_y, xxl[0].blur, xxl[0].spread), (25.0, 50.0, -12.0));
    assert_eq!(xxl[0].color.a, 64, "rgb(0 0 0 / 0.25)");
}

/// `BoxShadow` must refuse an over-capacity layer list rather than truncate it. A
/// dropped layer renders as a plausible-looking wrong shadow, which is the failure
/// mode this whole section exists to stop repeating.
#[test]
fn box_shadow_refuses_to_silently_truncate() {
    let l = twcore::ShadowLayer {
        offset_x: 0.0,
        offset_y: 1.0,
        blur: 1.0,
        spread: 0.0,
        color: twcore::Color::BLACK,
    };
    assert!(twcore::BoxShadow::new(&[]).is_none(), "an empty list is not a shadow");
    assert!(twcore::BoxShadow::new(&[l]).is_some());
    assert!(twcore::BoxShadow::new(&[l, l]).is_some());
    assert!(
        twcore::BoxShadow::new(&[l, l, l]).is_none(),
        "three layers must fail, not quietly become two"
    );
}

// ---------------------------------------------------------------- text

#[test]
fn font_size_sets_size_and_line_height() {
    let mut r = r();
    let t = s(&mut r, "text-sm");
    assert_eq!(t.text.size, 14.0);
    assert_eq!(t.text.line_height, LineHeight::Absolute(20.0));

    let big = s(&mut r, "text-7xl");
    assert_eq!(big.text.size, 72.0);
    assert_eq!(big.text.line_height, LineHeight::Relative(1.0));
}

#[test]
fn text_prefix_disambiguates_size_from_color() {
    let mut r = r();
    let both = s(&mut r, "text-7xl text-white");
    assert_eq!(both.text.size, 72.0);
    assert_eq!(both.text.color.unwrap().to_rgba_u32(), 0xffffffff);
}

#[test]
fn font_weight_leading_tracking() {
    let mut r = r();
    assert_eq!(s(&mut r, "font-bold").text.weight, 700);
    assert_eq!(s(&mut r, "font-medium").text.weight, 500);
    assert_eq!(s(&mut r, "leading-tight").text.line_height, LineHeight::Relative(1.25));
    assert_eq!(s(&mut r, "leading-6").text.line_height, LineHeight::Absolute(24.0));
    assert_eq!(s(&mut r, "tracking-wide").text.letter_spacing, 0.025);
    assert_eq!(s(&mut r, "tracking-tighter").text.letter_spacing, -0.05);
}

#[test]
fn text_decoration_align_family_nums() {
    let mut r = r();
    assert!(s(&mut r, "italic").text.italic);
    assert!(!s(&mut r, "not-italic").text.italic);
    assert!(s(&mut r, "underline").text.decoration.underline);
    assert!(s(&mut r, "line-through").text.decoration.line_through);
    assert!(!s(&mut r, "underline no-underline").text.decoration.underline);
    assert_eq!(s(&mut r, "text-center").text.align, TextAlignment::Center);
    assert_eq!(s(&mut r, "font-mono").text.family, FontFamily::Mono);
    assert!(s(&mut r, "tabular-nums").text.tabular_nums);
}

// ---------------------------------------------------------------- transform

#[test]
fn transform_family() {
    let mut r = r();
    assert_eq!(s(&mut r, "rotate-45").transform.rotate_deg, 45.0);
    assert_eq!(s(&mut r, "-rotate-90").transform.rotate_deg, -90.0);
    assert_eq!(s(&mut r, "scale-95").transform.scale_x, 0.95);
    assert_eq!(s(&mut r, "scale-x-50").transform.scale_x, 0.5);
    assert_eq!(s(&mut r, "scale-x-50").transform.scale_y, 1.0);
    assert_eq!(s(&mut r, "translate-y-4").transform.translate_y, 16.0);
    assert_eq!(s(&mut r, "-translate-x-2").transform.translate_x, -8.0);
}

// ---------------------------------------------------------------- coverage gate

/// One representative class per utility family the spike claims to support.
/// The pass criterion in plan1.md §10 is 40; this asserts we are at or above it
/// and that every single one round-trips without landing in `unknown`.
const FAMILIES: &[&str] = &[
    "flex",            // 1  display
    "flex-col",        // 2  flex-direction
    "flex-wrap",       // 3  flex-wrap
    "flex-1",          // 4  flex shorthand
    "grow",            // 5  flex-grow
    "shrink-0",        // 6  flex-shrink
    "basis-1/2",       // 7  flex-basis
    "items-center",    // 8  align-items
    "justify-center",  // 9  justify-content
    "self-end",        // 10 align-self
    "content-between", // 11 align-content
    "gap-6",           // 12 gap
    "gap-x-2",         // 13 gap axis
    "p-4",             // 14 padding
    "px-6",            // 15 padding axis
    "pt-2",            // 16 padding side
    "m-4",             // 17 margin
    "mx-auto",         // 18 margin auto
    "-mt-4",           // 19 negative margin
    "w-full",          // 20 width
    "h-24",            // 21 height
    "size-10",         // 22 size
    "min-w-0",         // 23 min-width
    "max-w-full",      // 24 max-width
    "min-h-screen",    // 25 min-height
    "absolute",        // 26 position
    "inset-0",         // 27 inset
    "top-4",           // 28 inset side
    "overflow-hidden", // 29 overflow
    "overflow-y-auto", // 30 overflow axis
    "box-border",      // 31 box-sizing
    "aspect-square",   // 32 aspect-ratio
    "grid-cols-3",     // 33 grid template
    "col-span-2",      // 34 grid placement
    "bg-indigo-500",   // 35 background colour
    "bg-black/50",     // 36 alpha modifier
    "border",          // 37 border width
    "border-red-500",  // 38 border colour
    "rounded-xl",      // 39 radius
    "rounded-t-lg",    // 40 radius corner subset
    "opacity-50",      // 41 opacity
    "shadow-lg",       // 42 shadow
    "ring-2",          // 43 ring
    "text-7xl",        // 44 font size
    "text-white",      // 45 text colour
    "font-bold",       // 46 font weight
    "leading-tight",   // 47 line height
    "tracking-wide",   // 48 letter spacing
    "text-center",     // 49 text align
    "underline",       // 50 decoration
    "font-mono",       // 51 font family
    "italic",          // 52 italic
    "tabular-nums",    // 53 numeric variant
    "rotate-45",       // 54 rotate
    "scale-95",        // 55 scale
    "translate-y-4",   // 56 translate
    "hidden",          // 57 display none
    "w-[137px]",       // 58 arbitrary length
    "bg-[#0f172a]",    // 59 arbitrary colour
    "text-[13px]",     // 60 arbitrary font size
];

#[test]
fn covered_utility_families() {
    let mut r = r();
    let mut failures = vec![];
    for class in FAMILIES {
        let compiled = r.compile(class);
        if !compiled.unknown.is_empty() || compiled.decls.is_empty() {
            failures.push(*class);
        }
    }
    assert!(failures.is_empty(), "utilities not resolving: {failures:?}");
    assert!(
        FAMILIES.len() >= 40,
        "spike 3 pass criterion is 40 utilities, table has {}",
        FAMILIES.len()
    );
    println!("utility families covered: {}", FAMILIES.len());
}

#[test]
fn unknown_classes_are_reported_not_swallowed() {
    let mut r = r();
    let unknown = r.unknown("flex bg-indigo-500 not-a-real-class hovr:hidden bg-notacolor-500");
    assert_eq!(unknown, vec!["not-a-real-class", "hovr:hidden", "bg-notacolor-500"]);
}
