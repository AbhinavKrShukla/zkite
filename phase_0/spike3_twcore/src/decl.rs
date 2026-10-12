//! `Decl` is the normalised form of one utility, produced once at parse time.
//!
//! Applying a `Decl` is a `match` plus a field assignment. Nothing here allocates,
//! parses, or looks anything up — all of that happened in `parse.rs`.

use taffy::prelude::*;
use taffy::geometry::Point;
use taffy::style::{BoxSizing, GridPlacement, Overflow, Position};

use crate::style::{
    BoxShadow as BoxShadowValue, Color, ComputedStyle, ExtColor, FontFamily, GradientColorSpace,
    GradientHue, GradientLine as GradientLineValue, LineHeight, ObjectFit, PaintColor, StopSlot,
    TextAlignment,
};

/// Tailwind v4's `--default-ring-width`. v3's was 3px; see `Decl::RingColor`.
pub const DEFAULT_RING_WIDTH: f32 = 1.0;

/// Which physical edges a spacing utility touches. `p-4` is all four, `px-4` is two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sides(pub u8);

impl Sides {
    pub const TOP: Sides = Sides(1);
    pub const RIGHT: Sides = Sides(2);
    pub const BOTTOM: Sides = Sides(4);
    pub const LEFT: Sides = Sides(8);
    pub const X: Sides = Sides(2 | 8);
    pub const Y: Sides = Sides(1 | 4);
    pub const ALL: Sides = Sides(15);
    fn has(self, s: Sides) -> bool {
        self.0 & s.0 != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Corners4(pub u8);

impl Corners4 {
    pub const TL: Corners4 = Corners4(1);
    pub const TR: Corners4 = Corners4(2);
    pub const BR: Corners4 = Corners4(4);
    pub const BL: Corners4 = Corners4(8);
    pub const ALL: Corners4 = Corners4(15);
    pub const TOP: Corners4 = Corners4(1 | 2);
    pub const BOTTOM: Corners4 = Corners4(4 | 8);
    pub const LEFT: Corners4 = Corners4(1 | 8);
    pub const RIGHT: Corners4 = Corners4(2 | 4);
    fn has(self, c: Corners4) -> bool {
        self.0 & c.0 != 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    // ---- layout ----
    Display(Display),
    BoxSizing(BoxSizing),
    Position(Position),
    FlexDirection(FlexDirection),
    FlexWrap(FlexWrap),
    AlignItems(Option<AlignItems>),
    AlignSelf(Option<AlignSelf>),
    JustifyContent(Option<JustifyContent>),
    AlignContent(Option<AlignContent>),
    FlexGrow(f32),
    FlexShrink(f32),
    FlexBasis(Dimension),
    Gap(Axis, LengthPercentage),
    Padding(Sides, LengthPercentage),
    Margin(Sides, LengthPercentageAuto),
    Inset(Sides, LengthPercentageAuto),
    Size(Axis, Dimension),
    MinSize(Axis, Dimension),
    MaxSize(Axis, Dimension),
    AspectRatio(Option<f32>),
    OverflowAxis(Axis, Overflow),
    BorderWidth(Sides, f32),
    GridCols(u16),
    GridRows(u16),
    ColSpan(u16),
    RowSpan(u16),

    // ---- paint ----
    Background(Color),
    /// `bg-linear-to-r`, `bg-linear-45`, and v3's still-live `bg-gradient-to-r` — the
    /// gradient *line* plus the interpolation its modifier selected. `None` is
    /// `bg-none`, which clears it.
    ///
    /// Carries the interpolation because one utility declares both:
    /// `bg-linear-to-r/oklch` is a single class and v4 compiles it to a single
    /// `--tw-gradient-position: to right in oklch`.
    GradientLine(Option<(GradientLineValue, GradientColorSpace, GradientHue)>),
    /// `from-cyan-500`, `via-fuchsia-500/50`, `to-[#0000ff]`.
    GradientStopColor(StopSlot, ExtColor),
    /// `from-10%`, `via-30%`, `to-90%` — a fraction, independent of the colour.
    GradientStopOffset(StopSlot, f32),
    BorderColor(Sides, Color),
    Radius(Corners4, f32),
    Opacity(f32),
    /// Aliased on import because the variant and the value type share a name — the
    /// variant is `Decl::BoxShadow`, the value is `style::BoxShadow`.
    BoxShadow(Option<BoxShadowValue>),
    Ring(f32),
    RingColor(Color),
    ObjectFit(ObjectFit),

    // ---- text ----
    TextColor(Color),
    /// Tailwind's `text-lg` sets a size *and* a default line height.
    FontSize(f32, Option<LineHeight>),
    FontWeight(u16),
    Italic(bool),
    Leading(LineHeight),
    Tracking(f32),
    TextAlign(TextAlignment),
    Underline(bool),
    LineThrough(bool),
    FontFamily(FontFamily),
    TabularNums(bool),

    // ---- transform ----
    Rotate(f32),
    Scale(Axis, f32),
    Translate(Axis, f32),
}

fn set_sides_lp(rect: &mut Rect<LengthPercentage>, sides: Sides, v: LengthPercentage) {
    if sides.has(Sides::TOP) {
        rect.top = v;
    }
    if sides.has(Sides::RIGHT) {
        rect.right = v;
    }
    if sides.has(Sides::BOTTOM) {
        rect.bottom = v;
    }
    if sides.has(Sides::LEFT) {
        rect.left = v;
    }
}

fn set_sides_lpa(rect: &mut Rect<LengthPercentageAuto>, sides: Sides, v: LengthPercentageAuto) {
    if sides.has(Sides::TOP) {
        rect.top = v;
    }
    if sides.has(Sides::RIGHT) {
        rect.right = v;
    }
    if sides.has(Sides::BOTTOM) {
        rect.bottom = v;
    }
    if sides.has(Sides::LEFT) {
        rect.left = v;
    }
}

impl Decl {
    pub fn apply(&self, cs: &mut ComputedStyle) {
        let l = &mut cs.layout;
        match *self {
            Decl::Display(d) => l.display = d,
            Decl::BoxSizing(b) => l.box_sizing = b,
            Decl::Position(p) => l.position = p,
            Decl::FlexDirection(d) => l.flex_direction = d,
            Decl::FlexWrap(w) => l.flex_wrap = w,
            Decl::AlignItems(a) => l.align_items = a,
            Decl::AlignSelf(a) => l.align_self = a,
            Decl::JustifyContent(j) => l.justify_content = j,
            Decl::AlignContent(a) => l.align_content = a,
            Decl::FlexGrow(v) => l.flex_grow = v,
            Decl::FlexShrink(v) => l.flex_shrink = v,
            Decl::FlexBasis(d) => l.flex_basis = d,
            Decl::Gap(axis, v) => match axis {
                Axis::X => l.gap.width = v,
                Axis::Y => l.gap.height = v,
                Axis::Both => l.gap = Size { width: v, height: v },
            },
            Decl::Padding(s, v) => set_sides_lp(&mut l.padding, s, v),
            Decl::Margin(s, v) => set_sides_lpa(&mut l.margin, s, v),
            Decl::Inset(s, v) => set_sides_lpa(&mut l.inset, s, v),
            Decl::Size(axis, d) => match axis {
                Axis::X => l.size.width = d,
                Axis::Y => l.size.height = d,
                Axis::Both => l.size = Size { width: d, height: d },
            },
            Decl::MinSize(axis, d) => match axis {
                Axis::X => l.min_size.width = d,
                Axis::Y => l.min_size.height = d,
                Axis::Both => l.min_size = Size { width: d, height: d },
            },
            Decl::MaxSize(axis, d) => match axis {
                Axis::X => l.max_size.width = d,
                Axis::Y => l.max_size.height = d,
                Axis::Both => l.max_size = Size { width: d, height: d },
            },
            Decl::AspectRatio(r) => l.aspect_ratio = r,
            Decl::OverflowAxis(axis, o) => match axis {
                Axis::X => l.overflow.x = o,
                Axis::Y => l.overflow.y = o,
                Axis::Both => l.overflow = Point { x: o, y: o },
            },
            Decl::BorderWidth(s, w) => {
                set_sides_lp(&mut l.border, s, LengthPercentage::length(w));
                // Tailwind's border utilities default to a visible border; without a
                // colour the renderer would draw nothing, so seed the sides we touched.
                //
                // **The seed is `currentColor`, not gray-200.** v4's preflight resets
                // every element to `border: 0 solid` — no colour — so `border-color`
                // keeps its CSS initial value, which is `currentColor`. v3 reset it to
                // `#e5e7eb` and that is what this used to seed. Measured in real
                // Chromium rather than read from docs: `border-2` alone computes
                // `border-top-color: rgb(0, 0, 0)` and `border-2 text-red-500` computes
                // it to the red. See phase_3/PLAN.md §28.1.
                let bc = &mut cs.paint.border_color;
                let seed = PaintColor::Current;
                if s.has(Sides::TOP) && bc.top.is_none() {
                    bc.top = Some(seed);
                }
                if s.has(Sides::RIGHT) && bc.right.is_none() {
                    bc.right = Some(seed);
                }
                if s.has(Sides::BOTTOM) && bc.bottom.is_none() {
                    bc.bottom = Some(seed);
                }
                if s.has(Sides::LEFT) && bc.left.is_none() {
                    bc.left = Some(seed);
                }
            }
            Decl::GridCols(n) => {
                l.grid_template_columns = (0..n).map(|_| fr(1.0_f32)).collect();
            }
            Decl::GridRows(n) => {
                l.grid_template_rows = (0..n).map(|_| fr(1.0_f32)).collect();
            }
            Decl::ColSpan(n) => l.grid_column = Line { start: GridPlacement::Auto, end: span(n) },
            Decl::RowSpan(n) => l.grid_row = Line { start: GridPlacement::Auto, end: span(n) },

            Decl::Background(c) => cs.paint.background = Some(c),
            Decl::GradientLine(v) => {
                let g = &mut cs.paint.gradient;
                match v {
                    Some((line, space, hue)) => {
                        g.line = Some(line);
                        g.space = space;
                        g.hue = hue;
                    }
                    // `bg-none` clears the *line*, not the stops — which is what
                    // Chromium does with the underlying custom properties: they stay
                    // set and `background-image` resolves to `none`.
                    None => g.line = None,
                }
            }
            Decl::GradientStopColor(slot, c) => cs.paint.gradient.set_color(slot, c),
            Decl::GradientStopOffset(slot, v) => cs.paint.gradient.set_offset(slot, v),
            Decl::BorderColor(s, c) => {
                let bc = &mut cs.paint.border_color;
                let c = PaintColor::Fixed(c);
                if s.has(Sides::TOP) {
                    bc.top = Some(c);
                }
                if s.has(Sides::RIGHT) {
                    bc.right = Some(c);
                }
                if s.has(Sides::BOTTOM) {
                    bc.bottom = Some(c);
                }
                if s.has(Sides::LEFT) {
                    bc.left = Some(c);
                }
            }
            Decl::Radius(c, v) => {
                let r = &mut cs.paint.radius;
                if c.has(Corners4::TL) {
                    r.top_left = v;
                }
                if c.has(Corners4::TR) {
                    r.top_right = v;
                }
                if c.has(Corners4::BR) {
                    r.bottom_right = v;
                }
                if c.has(Corners4::BL) {
                    r.bottom_left = v;
                }
            }
            Decl::Opacity(v) => cs.paint.opacity = v,
            Decl::BoxShadow(s) => cs.paint.shadow = s,
            Decl::Ring(w) => {
                // **v4's default ring colour is `currentColor`, not `blue-500`.** §9.6
                // updated this literal from v3's blue-500 hex to v4's, which fixed the
                // wrong half: the *value* of blue-500 changed between versions, but so
                // did the fact that a bare `ring` uses blue-500 at all. Measured in real
                // Chromium: `ring-2` computes `box-shadow: rgb(0, 0, 0) 0 0 0 2px` and
                // `ring-2 text-emerald-500` computes the emerald. phase_3/PLAN.md §28.1.
                let color = cs.paint.ring.map(|(_, c)| c).unwrap_or(PaintColor::Current);
                cs.paint.ring = if w == 0.0 { None } else { Some((w, color)) };
            }
            Decl::RingColor(c) => {
                // **v4's default ring width is 1px, not 3px** — same measurement: a bare
                // `ring` computes `0 0 0 1px`. v3's was 3px.
                let w = cs.paint.ring.map(|(w, _)| w).unwrap_or(DEFAULT_RING_WIDTH);
                cs.paint.ring = Some((w, PaintColor::Fixed(c)));
            }

            Decl::ObjectFit(f) => cs.paint.object_fit = f,

            Decl::TextColor(c) => cs.text.color = Some(c),
            Decl::FontSize(size, lh) => {
                cs.text.size = size;
                if let Some(lh) = lh {
                    cs.text.line_height = lh;
                }
            }
            Decl::FontWeight(w) => cs.text.weight = w,
            Decl::Italic(v) => cs.text.italic = v,
            Decl::Leading(lh) => cs.text.line_height = lh,
            Decl::Tracking(v) => cs.text.letter_spacing = v,
            Decl::TextAlign(a) => cs.text.align = a,
            Decl::Underline(v) => cs.text.decoration.underline = v,
            Decl::LineThrough(v) => cs.text.decoration.line_through = v,
            Decl::FontFamily(f) => cs.text.family = f,
            Decl::TabularNums(v) => cs.text.tabular_nums = v,

            Decl::Rotate(d) => cs.transform.rotate_deg = d,
            Decl::Scale(axis, v) => match axis {
                Axis::X => cs.transform.scale_x = v,
                Axis::Y => cs.transform.scale_y = v,
                Axis::Both => {
                    cs.transform.scale_x = v;
                    cs.transform.scale_y = v;
                }
            },
            Decl::Translate(axis, v) => match axis {
                Axis::X => cs.transform.translate_x = v,
                Axis::Y => cs.transform.translate_y = v,
                Axis::Both => {
                    cs.transform.translate_x = v;
                    cs.transform.translate_y = v;
                }
            },
        }
    }
}

/// `no_underline` / `not-italic` need to clear both decoration bits.
pub fn no_underline() -> [Decl; 2] {
    [Decl::Underline(false), Decl::LineThrough(false)]
}
