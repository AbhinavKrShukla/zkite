//! `src/palette.rs` is generated from the OKLCH palette in the *same*
//! `@tailwindcss/browser` bundle `phase_1/webruntime` vendors for the WebView backend.
//! These tests guard the generation, the sort order the binary search depends on, and a
//! few spot values a human can verify against the Tailwind docs.
//!
//! This used to be generated from `tailwindcss@3.4.17`'s hex table, which silently made
//! the two backends render different colours once phase 1 vendored v4 — v4 re-authored
//! the palette in OKLCH, so `indigo-500` went from `#6366f1` to `rgb(97, 95, 255)`.
//! `phase_3/PLAN.md` §9.5 has the measurement; the spot values below moved with it, and
//! are v4's, not v3's.

use twcore::palette::{HUES, PALETTE};
use twcore::{Color, NodeState, Resolver};

#[test]
fn palette_shape() {
    assert_eq!(HUES.len(), 26, "26 hues in Tailwind v4");
    assert_eq!(PALETTE.len(), 26 * 11, "11 shades (50..950) per hue");
}

#[test]
fn every_v3_hue_still_exists_so_no_existing_app_loses_a_colour() {
    // v4 adds four hues (mauve, mist, olive, taupe) and removes none. Worth asserting
    // rather than eyeballing: the point of this regeneration is to make the two
    // backends agree, and it would be a bad trade to fix that by dropping a colour
    // some app already uses.
    for hue in [
        "slate", "gray", "zinc", "neutral", "stone", "red", "orange", "amber", "yellow",
        "lime", "green", "emerald", "teal", "cyan", "sky", "blue", "indigo", "violet",
        "purple", "fuchsia", "pink", "rose",
    ] {
        assert!(HUES.contains(&hue), "v3 hue {hue} vanished from the v4 palette");
    }
}

#[test]
fn spot_values_match_chromiums_own_rendering_of_the_v4_oklch_palette() {
    // These two are not read off the Tailwind docs — they were sampled out of a real
    // Chromium screenshot by phase 3's conformance harness (`phase_3/PLAN.md` §9.5),
    // which is what makes them the reference for whether the OKLCH -> sRGB conversion
    // in `scripts/gen_palette.py` (matrices, transfer function, and clipping rather
    // than gamut-mapping out-of-range channels) actually matches a browser.
    let mut r = Resolver::new();
    for (name, expected) in [("indigo-500", 0x615fffu32), ("slate-950", 0x020618)] {
        let c = r.theme().color(name).unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(c, Color::rgb(expected), "{name} does not match Chromium");
    }
}

#[test]
fn palette_is_sorted_for_binary_search() {
    let mut prev: Option<(&str, u16)> = None;
    for (hue, shade, _, _) in PALETTE {
        if let Some(p) = prev {
            assert!((*hue, *shade) > p, "palette out of order at {hue}-{shade}");
        }
        prev = Some((hue, *shade));
    }
}

#[test]
fn spot_values_match_the_tailwind_docs() {
    // Tailwind **v4**'s published values. `gray-200`/`rose-100`/`zinc-800` are
    // unchanged from v3 — the near-achromatic end of the palette survived the OKLCH
    // re-authoring almost exactly, which is precisely why §9.5's divergence hid for so
    // long: only saturated hues moved far enough to notice.
    let cases: &[(&str, u32)] = &[
        ("slate-950", 0x020618),
        ("gray-200", 0xe5e7eb),
        ("red-500", 0xfb2c36),
        ("amber-400", 0xffb900),
        ("emerald-600", 0x009966),
        ("sky-300", 0x74d4ff),
        ("blue-500", 0x2b7fff),
        ("indigo-500", 0x615fff),
        ("indigo-600", 0x4f39f6),
        ("violet-700", 0x7008e7),
        ("rose-100", 0xffe4e6),
        ("zinc-800", 0x27272a),
    ];
    let mut r = Resolver::new();
    for (name, hex) in cases {
        let c = r.theme().color(name).unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(c, Color::rgb(*hex), "{name}");
    }
}

#[test]
fn every_palette_entry_is_reachable_through_a_class_string() {
    let mut r = Resolver::new();
    for (hue, shade, hex, _) in PALETTE {
        let class = format!("bg-{hue}-{shade}");
        let s = r.resolve(&class, NodeState::default());
        assert_eq!(
            s.paint.background,
            Some(Color::rgb(*hex)),
            "{class} did not resolve to its palette entry"
        );
    }
}

/// The second column, checked against the first for all 286 entries: clipping the
/// unclipped form must reproduce the hex exactly.
///
/// This is what makes the two columns one fact rather than two. They are generated from
/// the same OKLCH source in the same pass (`scripts/gen_palette.py`), so the failure this
/// guards against is not a typo — it is a change to one column's derivation that forgets
/// the other, which is the shape §9.5, §28.1 and §36 all had.
#[test]
fn the_unclipped_column_clips_back_to_the_hex_column() {
    for (hue, shade, hex, srgb) in PALETTE {
        let [r, g, b] = *srgb;
        let ext = twcore::style::ExtColor { r, g, b, a: 1.0 };
        assert_eq!(
            ext.to_srgb8(),
            Color::rgb(*hex),
            "{hue}-{shade}: unclipped {srgb:?} does not clip to #{hex:06x}"
        );
    }
}

/// …and the entries really are out of gamut, so the test above is not vacuous.
///
/// Without this, a generator bug that clipped *both* columns would leave every assertion
/// above passing and silently restore the 68/255 gradient divergence §38.3 fixed.
///
/// **95 of the 286 entries are outside the sRGB gamut** — a third of the palette, which
/// is the measured basis for calling this the ordinary case for a gradient rather than an
/// exotic one. Asserted as a lower bound: the exact count is a property of Tailwind's
/// palette, not of this code, and re-vendoring the bundle may move it.
#[test]
fn the_palette_really_does_leave_the_srgb_gamut() {
    let out_of_gamut = PALETTE
        .iter()
        .filter(|(_, _, _, srgb)| srgb.iter().any(|c| *c < 0.0 || *c > 1.0))
        .count();
    assert!(
        out_of_gamut >= 90,
        "only {out_of_gamut} of {} entries are out of gamut — the unclipped column is \
         probably clipped, which would silently undo phase_3/PLAN.md §38.3",
        PALETTE.len()
    );
    // The specific entry §38.3 measured, named so a failure points at the measurement.
    let yellow_400 = PALETTE
        .iter()
        .find(|(h, s, _, _)| *h == "yellow" && *s == 400)
        .expect("yellow-400");
    assert!(
        (yellow_400.3[2] - -0.269112).abs() < 1e-5,
        "yellow-400's blue should be -0.269112, got {}",
        yellow_400.3[2]
    );
}

#[test]
fn arbitrary_hex_forms() {
    let mut r = Resolver::new();
    let st = NodeState::default();
    assert_eq!(r.resolve("bg-[#0f172a]", st).paint.background.unwrap().to_rgba_u32(), 0x0f172aff);
    assert_eq!(r.resolve("bg-[#fff]", st).paint.background.unwrap().to_rgba_u32(), 0xffffffff);
    assert_eq!(r.resolve("bg-[#12345678]", st).paint.background.unwrap().to_rgba_u32(), 0x12345678);
}

#[test]
fn arbitrary_lengths() {
    use taffy::prelude::*;
    let mut r = Resolver::new();
    let st = NodeState::default();
    assert_eq!(r.resolve("w-[137px]", st).layout.size.width, Dimension::length(137.0));
    assert_eq!(r.resolve("h-[50%]", st).layout.size.height, Dimension::percent(0.5));
    assert_eq!(r.resolve("p-[2rem]", st).layout.padding.top, LengthPercentage::length(32.0));
    assert_eq!(r.resolve("text-[13px]", st).text.size, 13.0);
    assert_eq!(r.resolve("rounded-[3px]", st).paint.radius.top_left, 3.0);
    assert_eq!(
        r.resolve("-mt-[10px]", st).layout.margin.top,
        LengthPercentageAuto::length(-10.0)
    );
}
