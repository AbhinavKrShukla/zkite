//! Variants resolve by bitmask, not by re-parsing — and the caches prove it.

use twcore::{NodeState, Resolver, StateFlags, Theme};

const BUTTON: &str = "px-6 py-3 rounded-2xl bg-indigo-500 active:bg-indigo-600 text-white";

#[test]
fn variant_applies_only_when_state_is_set() {
    let mut r = Resolver::new();
    let idle = r.resolve(BUTTON, NodeState::default());
    assert_eq!(idle.paint.background.unwrap().to_rgba_u32(), 0x615fffff); // indigo-500

    let pressed = r.resolve(BUTTON, NodeState::default().with(StateFlags::ACTIVE));
    assert_eq!(pressed.paint.background.unwrap().to_rgba_u32(), 0x4f39f6ff); // indigo-600

    // Everything not behind a variant is identical in both states.
    assert_eq!(idle.layout.padding, pressed.layout.padding);
    assert_eq!(idle.paint.radius, pressed.paint.radius);
}

#[test]
fn variant_wins_regardless_of_author_order() {
    // In real Tailwind, class-attribute order is irrelevant; CSS emission order decides.
    let mut r = Resolver::new();
    let a = "bg-indigo-500 active:bg-indigo-600";
    let b = "active:bg-indigo-600 bg-indigo-500";
    let st = NodeState::default().with(StateFlags::ACTIVE);
    assert_eq!(
        r.resolve(a, st).paint.background,
        r.resolve(b, st).paint.background,
        "author order must not change the winner"
    );
    assert_eq!(r.resolve(b, st).paint.background.unwrap().to_rgba_u32(), 0x4f39f6ff);
}

#[test]
fn multiple_variants_must_all_match() {
    let mut r = Resolver::new();
    let cls = "bg-white dark:hover:bg-slate-800";
    let dark_only = NodeState::default().with(StateFlags::DARK);
    let dark_hover = NodeState::default().with(StateFlags::DARK).with(StateFlags::HOVER);

    assert_eq!(r.resolve(cls, dark_only).paint.background.unwrap().to_rgba_u32(), 0xffffffff);
    assert_eq!(r.resolve(cls, dark_hover).paint.background.unwrap().to_rgba_u32(), 0x1d293dff);
}

#[test]
fn breakpoints_are_min_width() {
    let mut r = Resolver::new();
    let cls = "flex-col md:flex-row";
    let phone = NodeState { viewport_width: 400.0, ..Default::default() };
    let tablet = NodeState { viewport_width: 800.0, ..Default::default() };

    assert_eq!(r.resolve(cls, phone).layout.flex_direction, taffy::style::FlexDirection::Column);
    assert_eq!(r.resolve(cls, tablet).layout.flex_direction, taffy::style::FlexDirection::Row);
}

#[test]
fn responsive_beats_state_matching_css_emission_order() {
    // `md:` lives in a media block emitted after the base layer, so it wins over `hover:`.
    let mut r = Resolver::new();
    let cls = "bg-red-500 hover:bg-green-500 md:bg-blue-500";
    let st = NodeState { viewport_width: 900.0, ..Default::default() }.with(StateFlags::HOVER);
    assert_eq!(r.resolve(cls, st).paint.background.unwrap().to_rgba_u32(), 0x2b7fffff);
}

#[test]
fn dark_variant_switches_theme_colors() {
    let mut r = Resolver::new();
    let cls = "bg-white text-slate-900 dark:bg-slate-950 dark:text-slate-100";
    let light = r.resolve(cls, NodeState::default());
    let dark = r.resolve(cls, NodeState::default().with(StateFlags::DARK));
    assert_eq!(light.paint.background.unwrap().to_rgba_u32(), 0xffffffff);
    assert_eq!(dark.paint.background.unwrap().to_rgba_u32(), 0x020618ff);
    assert_eq!(dark.text.color.unwrap().to_rgba_u32(), 0xf1f5f9ff);
}

#[test]
fn rtl_and_landscape_variants() {
    let mut r = Resolver::new();
    let rtl = r.resolve("ml-4 rtl:mr-4", NodeState::default().with(StateFlags::RTL));
    assert_eq!(rtl.layout.margin.right, taffy::prelude::LengthPercentageAuto::length(16.0));

    let land = r.resolve("h-40 landscape:h-20", NodeState::default().with(StateFlags::LANDSCAPE));
    assert_eq!(land.layout.size.height, taffy::prelude::Dimension::length(80.0));
}

// ---------------------------------------------------------------- caching

#[test]
fn class_string_is_parsed_exactly_once() {
    let mut r = Resolver::new();
    let st = NodeState::default();
    for _ in 0..10_000 {
        let _ = r.resolve(BUTTON, st);
    }
    let stats = r.stats();
    assert_eq!(stats.compiles, 1, "the class string must be parsed once, ever");
    assert_eq!(stats.style_builds, 1);
    assert_eq!(stats.style_cache_hits, 9_999);
}

#[test]
fn each_distinct_state_builds_at_most_one_style() {
    let mut r = Resolver::new();
    let states = [
        NodeState::default(),
        NodeState::default().with(StateFlags::ACTIVE),
        NodeState::default().with(StateFlags::HOVER),
        NodeState::default().with(StateFlags::ACTIVE).with(StateFlags::HOVER),
    ];
    for _ in 0..1_000 {
        for st in states {
            let _ = r.resolve(BUTTON, st);
        }
    }
    assert_eq!(r.stats().compiles, 1);
    assert_eq!(r.stats().style_builds, states.len() as u64);
}

#[test]
fn viewport_widths_in_the_same_bucket_share_a_cache_entry() {
    let mut r = Resolver::new();
    // 300..639 are all below `sm`, so they cannot differ.
    for w in 300..640 {
        let _ = r.resolve("flex md:flex-row", NodeState { viewport_width: w as f32, ..Default::default() });
    }
    assert_eq!(r.stats().style_builds, 1);
}

#[test]
fn resolving_is_fast_enough_to_do_per_frame() {
    // Not a benchmark, a regression fence: 100k cached resolves must stay well
    // under a frame budget even in a debug build.
    let mut r = Resolver::new();
    let st = NodeState::default();
    let _ = r.resolve(BUTTON, st);
    let t0 = std::time::Instant::now();
    for _ in 0..100_000 {
        let _ = r.resolve(BUTTON, st);
    }
    let per_resolve_ns = t0.elapsed().as_nanos() as f64 / 100_000.0;
    println!("cached resolve: {per_resolve_ns:.0} ns");
    assert!(per_resolve_ns < 2_000.0, "cached resolve took {per_resolve_ns:.0} ns");
}

// ---------------------------------------------------------------- theming

#[test]
fn theme_overrides_change_the_output_without_touching_the_parser() {
    let mut theme = Theme::default();
    theme.spacing_unit = 8.0; // a chunkier scale
    theme.custom_colors.push(("brand".into(), twcore::Color::rgb(0xff5500)));
    let mut r = Resolver::with_theme(theme);

    let s = r.resolve("p-4 bg-brand", NodeState::default());
    assert_eq!(s.layout.padding.top, taffy::prelude::LengthPercentage::length(32.0));
    assert_eq!(s.paint.background.unwrap().to_rgba_u32(), 0xff5500ff);
    assert!(r.unknown("bg-brand").is_empty());
}

#[test]
fn custom_breakpoints_apply() {
    let mut theme = Theme::default();
    theme.screens.md = 500.0;
    let mut r = Resolver::with_theme(theme);
    let st = NodeState { viewport_width: 600.0, ..Default::default() };
    assert_eq!(r.resolve("md:flex-row flex-col", st).layout.flex_direction, taffy::style::FlexDirection::Row);
}
