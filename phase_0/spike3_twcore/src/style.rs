//! The output of the resolver: a fully computed style, ready for layout + paint.
//!
//! Note that `layout` is a real `taffy::Style` — not an intermediate representation
//! that later gets converted. That is the point of the spike: Tailwind semantics land
//! directly on the layout engine's own type with no CSS in between.

use taffy::Style as TaffyStyle;

/// Straight 8-bit-per-channel RGBA. Premultiplication is the renderer's business.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color { r: 0, g: 0, b: 0, a: 0 };
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0, a: 255 };
    pub const WHITE: Color = Color { r: 255, g: 255, b: 255, a: 255 };

    pub const fn rgb(hex: u32) -> Color {
        Color { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8, a: 255 }
    }

    /// Apply a Tailwind opacity modifier (`bg-black/50`), 0..=100.
    pub fn with_alpha_percent(self, pct: f32) -> Color {
        let a = (pct.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u8;
        Color { a, ..self }
    }

    pub fn to_rgba_u32(self) -> u32 {
        (self.r as u32) << 24 | (self.g as u32) << 16 | (self.b as u32) << 8 | self.a as u32
    }
}

/// A colour that may lie **outside** the sRGB gamut, in encoded sRGB, with the transfer
/// function odd-extended for negative channels (CSS Color 4's own rule, `-f(-c)`).
///
/// `Color` above is 8-bit and clamped, which is right for everything that *fills* — a
/// browser paints a solid `bg-yellow-400` as the clipped value. It is wrong for a
/// **gradient stop**: Chromium interpolates using the unclipped colour and clips only at
/// display time, so `from-yellow-400` (whose blue is -0.269, i.e. -68.6 on the 0..255
/// scale) holds the blue channel at 0 for the first fifth of its ramp. Interpolating the
/// clipped value instead starts the ramp 68.6 too high, which is exactly the per-channel
/// delta the conformance harness measured before this type existed
/// (phase_3/PLAN.md §38.3).
///
/// Most saturated Tailwind v4 colours are out of gamut in at least one channel, so this
/// is the ordinary case for a gradient rather than an edge one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExtColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    /// 0..=1. Alpha is always in gamut; only the colour channels can escape.
    pub a: f32,
}

impl ExtColor {
    pub const TRANSPARENT: ExtColor = ExtColor { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

    /// An in-gamut 8-bit colour — an arbitrary `[#rrggbb]` stop, a theme keyword, or a
    /// custom colour from `zkite.tailwind.json`. Exact: every 8-bit value is inside the
    /// gamut by construction, so nothing is lost in this direction.
    pub fn from_srgb8(c: Color) -> ExtColor {
        ExtColor {
            r: c.r as f32 / 255.0,
            g: c.g as f32 / 255.0,
            b: c.b as f32 / 255.0,
            a: c.a as f32 / 255.0,
        }
    }

    /// Apply a Tailwind opacity modifier (`from-cyan-500/50`), 0..=100.
    pub fn with_alpha_percent(self, pct: f32) -> ExtColor {
        ExtColor { a: pct.clamp(0.0, 100.0) / 100.0, ..self }
    }

    /// Clip back to 8-bit sRGB — what this colour would *fill* as. Only for callers that
    /// have nowhere to put an out-of-gamut value; a gradient stop must not use it.
    pub fn to_srgb8(self) -> Color {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color { r: q(self.r), g: q(self.g), b: q(self.b), a: q(self.a) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Corners {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl Corners {
    pub fn all(v: f32) -> Corners {
        Corners { top_left: v, top_right: v, bottom_right: v, bottom_left: v }
    }
    pub fn is_uniform(&self) -> bool {
        self.top_left == self.top_right
            && self.top_right == self.bottom_right
            && self.bottom_right == self.bottom_left
    }
    /// Whether this box is a plain rectangle — i.e. whether a painter can take the
    /// cheap `fill_rect` path instead of building a rounded-rect `BezPath`.
    ///
    /// Named rather than open-coded because the four-way comparison was written out
    /// three separate times across the two paint backends, and §35 was about to make
    /// it five.
    pub fn is_zero(&self) -> bool {
        self.top_left == 0.0
            && self.top_right == 0.0
            && self.bottom_right == 0.0
            && self.bottom_left == 0.0
    }
}

/// A paint colour that may be CSS's `currentColor` — "whatever this node's own
/// resolved text colour is", which is not knowable here because a `ComputedStyle` is
/// ancestry-free by design and `color` inherits.
///
/// This exists because **Tailwind v4 made `currentColor` the default for both
/// `border-color` and `ring-color`**, where v3 used `gray-200` and `blue-500`
/// respectively. Measured in real Chromium against the very bundle the WebView
/// backend runs, not read from docs: `border-2` alone resolves
/// `border-top-color: rgb(0, 0, 0)`, and `border-2 text-red-500` resolves it to the
/// red — i.e. it tracks `color`. See phase_3/PLAN.md §28.1.
///
/// Deliberately the same shape as `zkite_renderer`'s `IconPaint`, which already exists
/// for exactly this reason (SVG's own `currentColor`) and resolves the same way,
/// including the fallback to black when no colour is inherited from anywhere — which
/// is also what Chromium does, per the measurement above.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaintColor {
    /// CSS `currentColor` — the initial value of both `border-color` and `ring-color`.
    Current,
    Fixed(Color),
}

impl PaintColor {
    /// `current` is the node's own resolved (inherited) text colour, i.e. the same
    /// value its glyphs would be painted in. Black when there is none, matching both
    /// CSS's initial `color` and `IconPaint::resolve`'s existing behaviour.
    pub fn resolve(self, current: Option<Color>) -> Color {
        match self {
            PaintColor::Current => current.unwrap_or(Color::BLACK),
            PaintColor::Fixed(c) => c,
        }
    }
}

/// One `box-shadow` layer, in CSS's own terms.
///
/// `blur` is the **CSS blur radius**, not a Gaussian standard deviation — the two
/// differ by a factor of two and both vello backends' `fill_blurred_rounded_rect`
/// wants the latter. Keeping CSS's units here and converting at the paint site is
/// deliberate: this crate's whole job is to be a faithful record of what Tailwind
/// said, and a `std_dev` field would bake one renderer's API into the style model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowLayer {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

/// A `box-shadow`, which is a *list* of layers painted first-on-top.
///
/// **This was a single layer until phase_3/PLAN.md §28**, with a comment calling that
/// a documented simplification. It stopped being acceptable the moment anything
/// painted it: every one of Tailwind v4's `shadow-sm`/`md`/`lg`/`xl` is two layers,
/// so a one-layer render is a visibly different shadow, not a slightly cheaper one.
///
/// Fixed capacity rather than a `Vec` so `ComputedStyle` stays allocation-free on the
/// resolve path (it is cloned per cache miss), and **never silently truncating** —
/// `push` returns whether it fitted and the theme table is asserted against it, since
/// a dropped layer would render as a plausible-looking wrong shadow rather than as an
/// obvious failure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxShadow {
    layers: [ShadowLayer; BoxShadow::MAX_LAYERS],
    len: u8,
}

impl BoxShadow {
    /// Tailwind v4's built-in scale uses at most two layers. Raising this is free;
    /// what must not happen is a third layer being dropped without anyone noticing.
    pub const MAX_LAYERS: usize = 2;

    pub fn new(layers: &[ShadowLayer]) -> Option<BoxShadow> {
        if layers.is_empty() || layers.len() > Self::MAX_LAYERS {
            return None;
        }
        let blank = ShadowLayer {
            offset_x: 0.0,
            offset_y: 0.0,
            blur: 0.0,
            spread: 0.0,
            color: Color::TRANSPARENT,
        };
        let mut out = [blank; Self::MAX_LAYERS];
        out[..layers.len()].copy_from_slice(layers);
        Some(BoxShadow { layers: out, len: layers.len() as u8 })
    }

    /// The layers in CSS source order — **first is painted on top of second**, which is
    /// `box-shadow`'s own rule and is why a painter must walk this in reverse.
    pub fn layers(&self) -> &[ShadowLayer] {
        &self.layers[..self.len as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BorderColors {
    pub top: Option<PaintColor>,
    pub right: Option<PaintColor>,
    pub bottom: Option<PaintColor>,
    pub left: Option<PaintColor>,
}

impl BorderColors {
    /// `Some(c)` only when all four sides are present and equal — the case a painter
    /// can draw as one stroked path rather than four separate edge shapes.
    pub fn uniform(&self) -> Option<PaintColor> {
        match (self.top, self.right, self.bottom, self.left) {
            (Some(t), Some(r), Some(b), Some(l)) if t == r && r == b && b == l => Some(t),
            _ => None,
        }
    }
}

/// Which of Tailwind's three gradient stop slots a `from-*`/`via-*`/`to-*` utility
/// names. Deliberately not "first/middle/last": `via-*` is what *creates* a middle
/// stop, and its absence is the two-stop case rather than a stop with no colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopSlot {
    From,
    Via,
    To,
}

/// The gradient line, in CSS's own terms — `linear-gradient()`'s first argument.
///
/// The eight keyword directions are kept as keywords rather than pre-converted to
/// angles because **the four corner directions are not fixed angles**: CSS defines
/// `to top right` so that the line perpendicular to the gradient line passes through
/// the other two corners, which depends on the box's aspect ratio and is therefore not
/// knowable here (`ComputedStyle` is laid out later). See
/// `zkite_renderer::gradient::line_endpoints`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GradientLine {
    ToTop,
    ToRight,
    ToBottom,
    ToLeft,
    ToTopRight,
    ToTopLeft,
    ToBottomRight,
    ToBottomLeft,
    /// Degrees clockwise from "to top", CSS's `<angle>` convention — `bg-linear-45`,
    /// `bg-linear-[30deg]`.
    Angle(f32),
}

/// The colour space a gradient's ramp is interpolated in.
///
/// **The default is Oklab, not sRGB**, and that is measured rather than assumed: v4
/// emits `in oklab` on every `bg-linear-*` (`--tw-gradient-position: to right in
/// oklab`), and CSS Images 4 makes oklab the default for a gradient with no explicit
/// space, which is why Chromium's own computed `background-image` omits the keyword.
/// It is not cosmetic — `from-yellow-400 to-blue-600` differs between the two spaces
/// by **81/255 in blue** at the midpoint. phase_3/PLAN.md §38.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GradientColorSpace {
    #[default]
    Oklab,
    Oklch,
    Srgb,
}

/// Which way round a cylindrical space's hue travels — CSS Color 4 § 12.4. Only
/// meaningful for `Oklch`; Tailwind's `/longer`, `/shorter`, `/increasing` and
/// `/decreasing` modifiers select it (and all four *imply* `oklch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GradientHue {
    #[default]
    Shorter,
    Longer,
    Increasing,
    Decreasing,
}

/// One resolved stop: a colour and where it sits on the gradient line, as a fraction.
///
/// The colour is an `ExtColor` and not a `Color` — see that type's doc; it is the whole
/// difference between a ramp that matches Chromium and one that is 68/255 off it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub color: ExtColor,
    /// 0..=1 along the gradient line. Tailwind's defaults are 0 / 0.5 / 1.
    pub offset: f32,
}

/// A gradient that will actually paint: a line, an interpolation, and two or three
/// stops.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gradient {
    pub line: GradientLine,
    pub space: GradientColorSpace,
    pub hue: GradientHue,
    /// Always `From` then (optionally) `Via` then `To`, offsets non-decreasing.
    pub from: GradientStop,
    pub via: Option<GradientStop>,
    pub to: GradientStop,
}

/// What the gradient utilities *declared*, before it is known whether that adds up to
/// anything paintable.
///
/// This is a bag of independent fields rather than an `Option<Gradient>` because the
/// utilities are independent: `from-cyan-500` and `bg-linear-to-r` can appear in either
/// order, in different variants, or without each other, exactly as the CSS custom
/// properties they compile to can. `resolve()` is the one place that decides whether
/// the bag amounts to a gradient — see its doc for the rule, which is measured.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GradientSpec {
    /// `None` until a `bg-linear-*`/`bg-gradient-to-*` is seen — and set back to `None`
    /// by `bg-none`.
    pub line: Option<GradientLine>,
    pub space: GradientColorSpace,
    pub hue: GradientHue,
    pub from: Option<ExtColor>,
    pub via: Option<ExtColor>,
    pub to: Option<ExtColor>,
    /// Fractions, `None` meaning Tailwind's own default for that slot (0 / 0.5 / 1).
    /// Settable without a colour (`from-10%`), which is why these are separate.
    pub from_offset: Option<f32>,
    pub via_offset: Option<f32>,
    pub to_offset: Option<f32>,
}

impl GradientSpec {
    /// Tailwind's own default offsets, from `--tw-gradient-{from,via,to}-position`.
    const DEFAULT_OFFSETS: (f32, f32, f32) = (0.0, 0.5, 1.0);

    pub fn set_color(&mut self, slot: StopSlot, c: ExtColor) {
        match slot {
            StopSlot::From => self.from = Some(c),
            StopSlot::Via => self.via = Some(c),
            StopSlot::To => self.to = Some(c),
        }
    }

    pub fn set_offset(&mut self, slot: StopSlot, v: f32) {
        match slot {
            StopSlot::From => self.from_offset = Some(v),
            StopSlot::Via => self.via_offset = Some(v),
            StopSlot::To => self.to_offset = Some(v),
        }
    }

    /// The gradient this spec paints, or `None`.
    ///
    /// **Two things must both be present: a line *and* at least one colour stop.**
    /// That is measured, not inferred — a `bg-linear-*` compiles to
    /// `background-image: linear-gradient(var(--tw-gradient-stops))`, and
    /// `--tw-gradient-stops` is defined only by a *colour* stop utility. So in real
    /// Chromium, against the bundle the WebView backend runs:
    ///
    /// | classes | `background-image` |
    /// |---|---|
    /// | `from-cyan-500 to-blue-500` | `none` |
    /// | `bg-linear-to-r` | `none` |
    /// | `bg-linear-to-r from-10%` | `none` |
    /// | `bg-linear-to-r from-cyan-500` | a gradient, cyan → **transparent** |
    ///
    /// The last row is the other half of the rule: a slot with no colour of its own is
    /// `transparent`, not "absent" — a one-stop gradient still fades. phase_3/PLAN.md
    /// §38.1.
    pub fn resolve(&self) -> Option<Gradient> {
        let line = self.line?;
        if self.from.is_none() && self.via.is_none() && self.to.is_none() {
            return None;
        }
        let (df, dv, dt) = Self::DEFAULT_OFFSETS;
        let stop = |c: Option<ExtColor>, off: Option<f32>, default: f32| GradientStop {
            color: c.unwrap_or(ExtColor::TRANSPARENT),
            offset: off.unwrap_or(default),
        };
        Some(Gradient {
            line,
            space: self.space,
            hue: self.hue,
            from: stop(self.from, self.from_offset, df),
            via: self.via.map(|c| stop(Some(c), self.via_offset, dv)),
            to: stop(self.to, self.to_offset, dt),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Paint {
    pub background: Option<Color>,
    /// `background-image`'s gradient, which is a **separate CSS property from
    /// `background-color`** and paints on top of it rather than replacing it.
    ///
    /// That is measured: `bg-red-500 bg-linear-to-r from-cyan-500` renders red at the
    /// right-hand edge in Chromium, because the gradient's `to` slot defaults to
    /// transparent and the background colour shows through it. Modelling this as one
    /// `enum Background { Solid, Gradient }` would have lost that, and it is the shape
    /// a first draft naturally reaches for. phase_3/PLAN.md §38.1.
    pub gradient: GradientSpec,
    pub border_color: BorderColors,
    pub radius: Corners,
    pub opacity: f32,
    pub shadow: Option<BoxShadow>,
    /// (width, colour) — Tailwind `ring-*`. Not a CSS `outline`: v4 implements it as a
    /// spread-only, zero-blur `box-shadow` sitting entirely *outside* the border box,
    /// which is why it can paint outside its own node's rect and why nothing in
    /// taffy's layout reserves room for it.
    pub ring: Option<(f32, PaintColor)>,
    /// How a replaced element's bitmap fills its own (already laid out) box —
    /// Tailwind `object-*`. Paint-time only: the box is sized by the ordinary
    /// `w-`/`h-`/flex rules whatever this says, which is exactly why an `Image` has
    /// always reserved correct layout space while painting nothing.
    pub object_fit: ObjectFit,
}

impl Default for Paint {
    fn default() -> Self {
        Paint {
            background: None,
            gradient: GradientSpec::default(),
            border_color: BorderColors::default(),
            radius: Corners::default(),
            opacity: 1.0,
            shadow: None,
            ring: None,
            object_fit: ObjectFit::default(),
        }
    }
}

/// CSS `object-fit`, for the one widget that has intrinsic content of its own.
///
/// **The default is `Fill`, matching CSS, not `Cover`** — even though every
/// `zkite_py` `Image` carries `object-cover` in its `implied_classes` and so never
/// reaches a painter with this value. Defaulting to `Cover` here would agree with
/// ZKite's own widget and silently disagree with the browser for anyone who writes
/// `object-fill` or drops the implied class, which is the divergence class
/// `../../phase_3/PLAN.md` §15.1 exists to track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectFit {
    /// Stretch to the box on both axes, ignoring the aspect ratio.
    #[default]
    Fill,
    /// Scale to the largest size that fits *inside* the box, preserving aspect —
    /// letterboxed, nothing cropped.
    Contain,
    /// Scale to the smallest size that *covers* the box, preserving aspect — the
    /// overflow is cropped.
    Cover,
    /// Intrinsic size, centred, cropped if it doesn't fit.
    None,
    /// `min(None, Contain)` — never scales *up*, only down.
    ScaleDown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineHeight {
    /// Multiple of the font size (`leading-tight`, and the implicit value of `text-lg`).
    Relative(f32),
    /// Absolute dp (`leading-6`, `leading-[22px]`).
    Absolute(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontFamily {
    #[default]
    Sans,
    Serif,
    Mono,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlignment {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Decoration {
    pub underline: bool,
    pub line_through: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub color: Option<Color>,
    pub size: f32,
    pub line_height: LineHeight,
    pub weight: u16,
    pub italic: bool,
    /// In em, matching Tailwind's `tracking-*` scale.
    pub letter_spacing: f32,
    pub align: TextAlignment,
    pub decoration: Decoration,
    pub family: FontFamily,
    pub tabular_nums: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            color: None,
            size: 16.0,
            line_height: LineHeight::Relative(1.5),
            weight: 400,
            italic: false,
            letter_spacing: 0.0,
            align: TextAlignment::Start,
            decoration: Decoration::default(),
            family: FontFamily::Sans,
            tabular_nums: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub rotate_deg: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub translate_x: f32,
    pub translate_y: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Transform {
            rotate_deg: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            translate_x: 0.0,
            translate_y: 0.0,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComputedStyle {
    pub layout: TaffyStyle,
    pub paint: Paint,
    pub text: TextStyle,
    pub transform: Transform,
}
