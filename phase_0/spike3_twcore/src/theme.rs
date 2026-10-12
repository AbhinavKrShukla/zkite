//! The token tables. This is the `@theme`-equivalent from plan1.md §4 — the thing a
//! `theme.py` / `[tool.zkite.theme]` block overrides. Everything the utility parser needs
//! to turn a name into a number lives here, so a user override is a data change, not a
//! code change.

use crate::palette::PALETTE;
use crate::style::{BoxShadow, Color, ExtColor, LineHeight, ShadowLayer};

#[derive(Debug, Clone)]
pub struct Theme {
    /// dp per spacing unit. Tailwind's scale is `n * 4px`, so `p-4` == 16dp.
    pub spacing_unit: f32,
    /// Breakpoint minimum widths in dp, ascending.
    pub screens: Screens,
    /// User colour overrides, consulted before the built-in palette.
    pub custom_colors: Vec<(String, Color)>,
    /// Base font size for `text-base`.
    pub root_font_size: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct Screens {
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    pub xxl: f32,
}

impl Default for Screens {
    fn default() -> Self {
        Screens { sm: 640.0, md: 768.0, lg: 1024.0, xl: 1280.0, xxl: 1536.0 }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            spacing_unit: 4.0,
            screens: Screens::default(),
            custom_colors: Vec::new(),
            root_font_size: 16.0,
        }
    }
}

impl Theme {
    /// `p-4` -> 16.0, `p-0.5` -> 2.0, `p-px` -> 1.0.
    pub fn spacing(&self, token: &str) -> Option<f32> {
        if token == "px" {
            return Some(1.0);
        }
        // `.parse::<f32>()` accepts "inf"/"nan" — reject them rather than feed a
        // non-finite dimension into taffy, which corrupts layout instead of erroring.
        let n: f32 = token.parse().ok().filter(|v: &f32| v.is_finite())?;
        Some(n * self.spacing_unit)
    }

    /// Resolve `indigo-500`, `white`, `black`, `transparent`, or a user override.
    /// The caller has already split off any `/opacity` modifier.
    pub fn color(&self, name: &str) -> Option<Color> {
        for (k, v) in &self.custom_colors {
            if k == name {
                return Some(*v);
            }
        }
        match name {
            "transparent" => return Some(Color::TRANSPARENT),
            "black" => return Some(Color::BLACK),
            "white" => return Some(Color::WHITE),
            _ => {}
        }
        let (hue, shade) = name.rsplit_once('-')?;
        let shade: u16 = shade.parse().ok()?;
        Self::row(hue, shade).map(|i| Color::rgb(PALETTE[i].2))
    }

    /// The same lookup as `color`, returning the **unclipped** form — what a gradient
    /// stop needs. See `ExtColor`'s doc for why the two are not interchangeable.
    ///
    /// Reads the same palette row as `color`, so the two cannot disagree about *which*
    /// colour a name means, only about how it is represented. Custom colours and the
    /// three keywords come back through `ExtColor::from_srgb8`, which is exact for them:
    /// an 8-bit value is in gamut by construction.
    pub fn ext_color(&self, name: &str) -> Option<ExtColor> {
        for (k, v) in &self.custom_colors {
            if k == name {
                return Some(ExtColor::from_srgb8(*v));
            }
        }
        match name {
            "transparent" => return Some(ExtColor::TRANSPARENT),
            "black" => return Some(ExtColor::from_srgb8(Color::BLACK)),
            "white" => return Some(ExtColor::from_srgb8(Color::WHITE)),
            _ => {}
        }
        let (hue, shade) = name.rsplit_once('-')?;
        let shade: u16 = shade.parse().ok()?;
        Self::row(hue, shade).map(|i| {
            let [r, g, b] = PALETTE[i].3;
            ExtColor { r, g, b, a: 1.0 }
        })
    }

    fn row(hue: &str, shade: u16) -> Option<usize> {
        PALETTE.binary_search_by(|(h, s, _, _)| h.cmp(&hue).then(s.cmp(&shade))).ok()
    }

    /// `text-lg` sets both a size and a default line height, exactly as Tailwind does.
    pub fn font_size(&self, token: &str) -> Option<(f32, LineHeight)> {
        use LineHeight::{Absolute, Relative};
        Some(match token {
            "xs" => (12.0, Absolute(16.0)),
            "sm" => (14.0, Absolute(20.0)),
            "base" => (16.0, Absolute(24.0)),
            "lg" => (18.0, Absolute(28.0)),
            "xl" => (20.0, Absolute(28.0)),
            "2xl" => (24.0, Absolute(32.0)),
            "3xl" => (30.0, Absolute(36.0)),
            "4xl" => (36.0, Absolute(40.0)),
            "5xl" => (48.0, Relative(1.0)),
            "6xl" => (60.0, Relative(1.0)),
            "7xl" => (72.0, Relative(1.0)),
            "8xl" => (96.0, Relative(1.0)),
            "9xl" => (128.0, Relative(1.0)),
            _ => return None,
        })
    }

    /// Tailwind v4's `--radius-*` scale, in dp.
    ///
    /// **Regenerated from v4 in phase_3/PLAN.md §36; this table was v3's**, and it
    /// carried the identical mistake §28.1 found in the shadow scale — v4 renamed the
    /// scale by shifting every name one step:
    ///
    /// | class | v3 | v4 |
    /// |---|---|---|
    /// | `rounded-xs` | *absent* | **2px** |
    /// | `rounded-sm` | 2px | **4px** |
    /// | `rounded-4xl` | *absent* | **32px** |
    ///
    /// So `rounded-sm` on the WebView backend and `rounded-sm` here were not the same
    /// corner drawn two ways — the native one was **half** the radius, on every
    /// background, border, ring and shadow that node carried. `rounded-xs` and
    /// `rounded-4xl` did not parse at all, which is quieter: an unknown utility is
    /// dropped, so the corner came out square rather than merely wrong.
    ///
    /// `md` through `3xl`, `none` and bare `rounded` were already v4's and are
    /// unchanged. Measured, not read: `phase_1/webruntime/test/probe_theme_defaults.mjs`
    /// drives the very `@tailwindcss/browser` bundle `phase_1/webruntime/vendor/` pins
    /// in real Chromium and reads back the computed `border-top-left-radius`.
    ///
    /// `full` is `calc(infinity * 1px)` in v4, which Chromium computes as 3.36e7 px.
    /// 9999 is kept: both mean "a pill", every consumer clamps the radius to what the
    /// rect can fit (§28.2), and a near-infinite float would make that clamp the only
    /// thing standing between `kurbo` and a non-finite arc.
    pub fn radius(&self, token: &str) -> Option<f32> {
        Some(match token {
            "none" => 0.0,
            "xs" => 2.0,
            "sm" => 4.0,
            "" => 4.0, // bare `rounded`
            "md" => 6.0,
            "lg" => 8.0,
            "xl" => 12.0,
            "2xl" => 16.0,
            "3xl" => 24.0,
            "4xl" => 32.0,
            "full" => 9999.0,
            _ => return None,
        })
    }

    pub fn font_weight(&self, token: &str) -> Option<u16> {
        Some(match token {
            "thin" => 100,
            "extralight" => 200,
            "light" => 300,
            "normal" => 400,
            "medium" => 500,
            "semibold" => 600,
            "bold" => 700,
            "extrabold" => 800,
            "black" => 900,
            _ => return None,
        })
    }

    pub fn leading(&self, token: &str) -> Option<LineHeight> {
        use LineHeight::Relative;
        Some(match token {
            "none" => Relative(1.0),
            "tight" => Relative(1.25),
            "snug" => Relative(1.375),
            "normal" => Relative(1.5),
            "relaxed" => Relative(1.625),
            "loose" => Relative(2.0),
            // Numeric leading uses the spacing scale: leading-6 == 24dp.
            n => LineHeight::Absolute(self.spacing(n)?),
        })
    }

    /// Returned in em.
    pub fn tracking(&self, token: &str) -> Option<f32> {
        Some(match token {
            "tighter" => -0.05,
            "tight" => -0.025,
            "normal" => 0.0,
            "wide" => 0.025,
            "wider" => 0.05,
            "widest" => 0.1,
            _ => return None,
        })
    }

    /// Tailwind v4's `--shadow-*` scale.
    ///
    /// **Regenerated from v4 in phase_3/PLAN.md §28.1; this table was v3's.** Two
    /// things changed between the versions and both were invisible until something
    /// painted a shadow:
    ///
    /// 1. **v4 renamed the whole scale.** v4's `shadow-xs` is v3's `shadow-sm`, and
    ///    v4's `shadow-sm` is v3's bare `shadow`. So `shadow-sm` on the WebView
    ///    backend and `shadow-sm` here were different shadows, not the same one drawn
    ///    two ways.
    /// 2. **v4's sm/md/lg/xl are each *two* layers**, where this returned one.
    ///
    /// The values are what real Chromium reports as the computed `box-shadow` for the
    /// very `@tailwindcss/browser` bundle `phase_1/webruntime/vendor/` pins — the same
    /// source-of-truth discipline §9.6 applied to the palette, for the same reason:
    /// two hand-maintained copies of one design is how these drift.
    pub fn shadow(&self, token: &str) -> Option<Option<BoxShadow>> {
        let black = |a: f32| Color { a: (a * 255.0).round() as u8, ..Color::BLACK };
        let l = |y: f32, blur: f32, spread: f32, alpha: f32| ShadowLayer {
            offset_x: 0.0,
            offset_y: y,
            blur,
            spread,
            color: black(alpha),
        };
        // `BoxShadow::new` returns None on an over-capacity list rather than
        // truncating, so a third layer added here fails loudly instead of rendering as
        // a plausible-looking wrong shadow. `expect` is sound: this table is a
        // constant, so a failure is a programming error, not user input.
        let s = |layers: &[ShadowLayer]| {
            Some(BoxShadow::new(layers).expect("built-in shadow scale fits BoxShadow::MAX_LAYERS"))
        };
        Some(match token {
            "none" => None,
            "2xs" => s(&[l(1.0, 0.0, 0.0, 0.05)]),
            "xs" => s(&[l(1.0, 2.0, 0.0, 0.05)]),
            "sm" | "" => s(&[l(1.0, 3.0, 0.0, 0.1), l(1.0, 2.0, -1.0, 0.1)]),
            "md" => s(&[l(4.0, 6.0, -1.0, 0.1), l(2.0, 4.0, -2.0, 0.1)]),
            "lg" => s(&[l(10.0, 15.0, -3.0, 0.1), l(4.0, 6.0, -4.0, 0.1)]),
            "xl" => s(&[l(20.0, 25.0, -5.0, 0.1), l(8.0, 10.0, -6.0, 0.1)]),
            "2xl" => s(&[l(25.0, 50.0, -12.0, 0.25)]),
            _ => return None,
        })
    }

    pub fn breakpoint(&self, name: &str) -> Option<f32> {
        Some(match name {
            "sm" => self.screens.sm,
            "md" => self.screens.md,
            "lg" => self.screens.lg,
            "xl" => self.screens.xl,
            "2xl" => self.screens.xxl,
            _ => return None,
        })
    }
}
