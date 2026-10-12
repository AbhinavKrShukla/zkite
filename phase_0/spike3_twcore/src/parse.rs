//! Utility string -> `Vec<Decl>`. This is the only place that does string work.
//!
//! Returning `None` means "unrecognised utility". The resolver counts those and can
//! surface them, which is how `zkite doctor` will tell a user their class was a typo
//! rather than silently rendering nothing.

use taffy::prelude::*;
use taffy::style::Overflow;

use crate::decl::{Axis, Corners4, Decl, Sides};
use crate::style::{
    Color, ExtColor, GradientColorSpace, GradientHue, GradientLine, LineHeight, StopSlot,
    TextAlignment,
};
use crate::theme::Theme;

/// A length in whatever form the utility gave us.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Len {
    Px(f32),
    /// Fraction in 0..=1, matching taffy's percent convention.
    Pct(f32),
    Auto,
}

impl Len {
    fn to_dimension(self) -> Dimension {
        match self {
            Len::Px(v) => Dimension::length(v),
            Len::Pct(v) => Dimension::percent(v),
            Len::Auto => Dimension::auto(),
        }
    }
    fn to_lp(self) -> LengthPercentage {
        match self {
            Len::Px(v) => LengthPercentage::length(v),
            Len::Pct(v) => LengthPercentage::percent(v),
            Len::Auto => LengthPercentage::length(0.0),
        }
    }
    fn to_lpa(self) -> LengthPercentageAuto {
        match self {
            Len::Px(v) => LengthPercentageAuto::length(v),
            Len::Pct(v) => LengthPercentageAuto::percent(v),
            Len::Auto => LengthPercentageAuto::auto(),
        }
    }
    fn negate(self) -> Len {
        match self {
            Len::Px(v) => Len::Px(-v),
            Len::Pct(v) => Len::Pct(-v),
            Len::Auto => Len::Auto,
        }
    }
}

/// `.parse::<f32>()` accepts "inf"/"-inf"/"nan" — reject them here so a class like
/// `w-[inf]` surfaces as "unknown utility" instead of a non-finite `taffy::Style` that
/// corrupts or hangs layout.
fn finite_f32(s: &str) -> Option<f32> {
    s.parse::<f32>().ok().filter(|v| v.is_finite())
}

/// `[137px]`, `[50%]`, `[2rem]`, `[1.5]` (unitless -> px).
fn arbitrary_len(body: &str) -> Option<Len> {
    if let Some(n) = body.strip_suffix("px") {
        return finite_f32(n).map(Len::Px);
    }
    if let Some(n) = body.strip_suffix("rem") {
        return finite_f32(n).map(|v| Len::Px(v * 16.0));
    }
    if let Some(n) = body.strip_suffix("dp") {
        return finite_f32(n).map(Len::Px);
    }
    if let Some(n) = body.strip_suffix('%') {
        return finite_f32(n).map(|v| Len::Pct(v / 100.0));
    }
    finite_f32(body).map(Len::Px)
}

fn arbitrary_color(body: &str) -> Option<Color> {
    let hex = body.strip_prefix('#')?;
    let v = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        6 => Some(Color::rgb(v)),
        8 => Some(Color {
            r: (v >> 24) as u8,
            g: (v >> 16) as u8,
            b: (v >> 8) as u8,
            a: v as u8,
        }),
        3 => {
            let e = |sh: u32| {
                let d = ((v >> sh) & 0xf) as u8;
                d << 4 | d
            };
            Some(Color { r: e(8), g: e(4), b: e(0), a: 255 })
        }
        _ => None,
    }
}

/// Strip a bracketed arbitrary value: `w-[137px]` -> `("w", "137px")`.
fn split_arbitrary(u: &str) -> Option<(&str, &str)> {
    let open = u.find("-[")?;
    let body = u[open + 2..].strip_suffix(']')?;
    Some((&u[..open], body))
}

/// Sizing tokens shared by `w-*`, `h-*`, `min-*`, `max-*`, `basis-*`.
fn size_token(token: &str, theme: &Theme) -> Option<Len> {
    match token {
        "auto" => return Some(Len::Auto),
        "full" => return Some(Len::Pct(1.0)),
        // No viewport units in taffy: the root node *is* the screen, so 100% is exact.
        "screen" | "dvw" | "dvh" => return Some(Len::Pct(1.0)),
        "px" => return Some(Len::Px(1.0)),
        _ => {}
    }
    if let Some((num, den)) = token.split_once('/') {
        let n = finite_f32(num)?;
        let d = finite_f32(den)?;
        if d != 0.0 {
            return Some(Len::Pct(n / d));
        }
        return None;
    }
    theme.spacing(token).map(Len::Px)
}

/// Split a trailing `/50` opacity modifier off a colour token.
fn split_alpha(token: &str) -> (&str, Option<f32>) {
    match token.rsplit_once('/') {
        Some((name, pct)) => match pct.parse::<f32>() {
            Ok(v) => (name, Some(v)),
            Err(_) => (token, None),
        },
        None => (token, None),
    }
}

/// The interpolation an `/…` modifier on a `bg-linear-*` selects.
///
/// All four hue-direction modifiers *imply* `oklch`, which is measured rather than
/// assumed: `bg-linear-to-r/longer` compiles to `to right in oklch longer hue`, not to
/// oklab with a hue rule. phase_3/PLAN.md §38.1.
fn interpolation(modifier: Option<&str>) -> Option<(GradientColorSpace, GradientHue)> {
    use GradientColorSpace as S;
    use GradientHue as H;
    Some(match modifier {
        None => (S::Oklab, H::Shorter),
        Some("srgb") => (S::Srgb, H::Shorter),
        Some("oklab") => (S::Oklab, H::Shorter),
        Some("oklch") => (S::Oklch, H::Shorter),
        Some("shorter") => (S::Oklch, H::Shorter),
        Some("longer") => (S::Oklch, H::Longer),
        Some("increasing") => (S::Oklch, H::Increasing),
        Some("decreasing") => (S::Oklch, H::Decreasing),
        // An unrecognised modifier is an unrecognised utility, not a silent default —
        // `bg-linear-to-r/wat` must reach the unknown-class report.
        Some(_) => return None,
    })
}

/// The eight keyword directions, in both v4's spelling and v3's.
///
/// v3's `bg-gradient-to-r` is **still live in v4.3.3** — the vendored bundle compiles it
/// to a byte-identical `background-image`, measured, so refusing it here would break
/// class strings that work on the WebView backend. It is the same shape as `rounded-sm`
/// in §36: two spellings, one meaning, and the divergence is in what we accept.
fn gradient_direction(token: &str) -> Option<GradientLine> {
    Some(match token {
        "to-t" => GradientLine::ToTop,
        "to-r" => GradientLine::ToRight,
        "to-b" => GradientLine::ToBottom,
        "to-l" => GradientLine::ToLeft,
        "to-tr" => GradientLine::ToTopRight,
        "to-tl" => GradientLine::ToTopLeft,
        "to-br" => GradientLine::ToBottomRight,
        "to-bl" => GradientLine::ToBottomLeft,
        // `bg-linear-45`. Not accepted for the `bg-gradient-*` spelling, which v3 never
        // had and v4 does not add.
        _ => GradientLine::Angle(finite_f32(token)?),
    })
}

/// `bg-linear-to-r`, `bg-linear-45`, `bg-linear-to-br/srgb`, `bg-gradient-to-r`.
/// Returns `None` for anything else, including `bg-radial`/`bg-conic` — see §38.5.
fn gradient_line_utility(token: &str) -> Option<Decl> {
    let (body, modifier) = match token.split_once('/') {
        Some((b, m)) => (b, Some(m)),
        None => (token, None),
    };
    let dir = if let Some(rest) = body.strip_prefix("linear-") {
        gradient_direction(rest)?
    } else if let Some(rest) = body.strip_prefix("gradient-") {
        match rest {
            // Only the keyword forms; `bg-gradient-45` is not a v3 or a v4 utility.
            "to-t" | "to-r" | "to-b" | "to-l" | "to-tr" | "to-tl" | "to-br" | "to-bl" => {
                gradient_direction(rest)?
            }
            _ => return None,
        }
    } else {
        return None;
    };
    let (space, hue) = interpolation(modifier)?;
    Some(Decl::GradientLine(Some((dir, space, hue))))
}

/// A `from-*`/`via-*`/`to-*` utility: either a colour or a bare percentage.
///
/// The percentage form is a *position*, not a colour, and it does not on its own make
/// anything paint — `bg-linear-to-r from-10%` renders nothing in Chromium (§38.1).
fn gradient_stop_utility(slot: StopSlot, token: &str, theme: &Theme) -> Option<Vec<Decl>> {
    if let Some(pct) = token.strip_suffix('%') {
        return Some(vec![Decl::GradientStopOffset(slot, finite_f32(pct)? / 100.0)]);
    }
    // An arbitrary bracketed percentage, `from-[12.5%]`, for symmetry with `from-[#hex]`
    // which `arbitrary()` handles.
    if let Some(body) = token.strip_prefix('[').and_then(|b| b.strip_suffix(']')) {
        if let Some(pct) = body.strip_suffix('%') {
            return Some(vec![Decl::GradientStopOffset(slot, finite_f32(pct)? / 100.0)]);
        }
    }
    Some(vec![Decl::GradientStopColor(slot, ext_color_token(token, theme)?)])
}

/// `color_token`'s unclipped twin, for gradient stops only. Same token grammar, same
/// alpha modifier; the only difference is which palette column it reads — see
/// `ExtColor`'s doc for why a stop must not use the clipped one.
fn ext_color_token(token: &str, theme: &Theme) -> Option<ExtColor> {
    let (name, alpha) = split_alpha(token);
    let base = if let Some(body) = name.strip_prefix('[').and_then(|b| b.strip_suffix(']')) {
        ExtColor::from_srgb8(arbitrary_color(body)?)
    } else {
        theme.ext_color(name)?
    };
    Some(match alpha {
        Some(pct) => base.with_alpha_percent(pct),
        None => base,
    })
}

fn color_token(token: &str, theme: &Theme) -> Option<Color> {
    let (name, alpha) = split_alpha(token);
    let base = if let Some(body) = name.strip_prefix('[').and_then(|b| b.strip_suffix(']')) {
        arbitrary_color(body)?
    } else {
        theme.color(name)?
    };
    Some(match alpha {
        Some(pct) => base.with_alpha_percent(pct),
        None => base,
    })
}

/// Parse one utility (variants already stripped) into zero or more declarations.
pub fn parse_utility(utility: &str, theme: &Theme) -> Option<Vec<Decl>> {
    let (negative, u) = match utility.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, utility),
    };

    // ---- exact matches (cheap, and they disambiguate `flex` from `flex-1`) ----
    if !negative {
        if let Some(d) = exact(u, theme) {
            return Some(d);
        }
    }

    // ---- arbitrary values ----
    if let Some((head, body)) = split_arbitrary(u) {
        if let Some(d) = arbitrary(head, body, negative, theme) {
            return Some(d);
        }
    }

    // ---- prefixed utilities ----
    let (head, token) = u.split_once('-')?;
    prefixed(head, token, negative, theme)
}

fn exact(u: &str, theme: &Theme) -> Option<Vec<Decl>> {
    use Decl::*;
    let one = |d: Decl| Some(vec![d]);
    match u {
        // display
        "flex" | "inline-flex" => one(Display(taffy::style::Display::Flex)),
        "grid" | "inline-grid" => one(Display(taffy::style::Display::Grid)),
        "block" | "inline-block" | "inline" => one(Display(taffy::style::Display::Block)),
        "hidden" => one(Display(taffy::style::Display::None)),

        // flex container
        "flex-row" => one(FlexDirection(taffy::style::FlexDirection::Row)),
        "flex-row-reverse" => one(FlexDirection(taffy::style::FlexDirection::RowReverse)),
        "flex-col" => one(FlexDirection(taffy::style::FlexDirection::Column)),
        "flex-col-reverse" => one(FlexDirection(taffy::style::FlexDirection::ColumnReverse)),
        "flex-wrap" => one(FlexWrap(taffy::style::FlexWrap::Wrap)),
        "flex-wrap-reverse" => one(FlexWrap(taffy::style::FlexWrap::WrapReverse)),
        "flex-nowrap" => one(FlexWrap(taffy::style::FlexWrap::NoWrap)),

        // flex item shorthands
        "flex-1" => Some(vec![FlexGrow(1.0), FlexShrink(1.0), FlexBasis(Dimension::percent(0.0))]),
        "flex-auto" => Some(vec![FlexGrow(1.0), FlexShrink(1.0), FlexBasis(Dimension::auto())]),
        "flex-initial" => Some(vec![FlexGrow(0.0), FlexShrink(1.0), FlexBasis(Dimension::auto())]),
        "flex-none" => Some(vec![FlexGrow(0.0), FlexShrink(0.0), FlexBasis(Dimension::auto())]),
        "grow" => one(FlexGrow(1.0)),
        "grow-0" => one(FlexGrow(0.0)),
        "shrink" => one(FlexShrink(1.0)),
        "shrink-0" => one(FlexShrink(0.0)),

        // alignment
        "items-start" => one(AlignItems(Some(taffy::style::AlignItems::FlexStart))),
        "items-end" => one(AlignItems(Some(taffy::style::AlignItems::FlexEnd))),
        "items-center" => one(AlignItems(Some(taffy::style::AlignItems::Center))),
        "items-baseline" => one(AlignItems(Some(taffy::style::AlignItems::Baseline))),
        "items-stretch" => one(AlignItems(Some(taffy::style::AlignItems::Stretch))),
        "self-auto" => one(AlignSelf(None)),
        "self-start" => one(AlignSelf(Some(taffy::style::AlignSelf::FlexStart))),
        "self-end" => one(AlignSelf(Some(taffy::style::AlignSelf::FlexEnd))),
        "self-center" => one(AlignSelf(Some(taffy::style::AlignSelf::Center))),
        "self-stretch" => one(AlignSelf(Some(taffy::style::AlignSelf::Stretch))),
        "self-baseline" => one(AlignSelf(Some(taffy::style::AlignSelf::Baseline))),
        "justify-start" => one(JustifyContent(Some(taffy::style::JustifyContent::FlexStart))),
        "justify-end" => one(JustifyContent(Some(taffy::style::JustifyContent::FlexEnd))),
        "justify-center" => one(JustifyContent(Some(taffy::style::JustifyContent::Center))),
        "justify-between" => one(JustifyContent(Some(taffy::style::JustifyContent::SpaceBetween))),
        "justify-around" => one(JustifyContent(Some(taffy::style::JustifyContent::SpaceAround))),
        "justify-evenly" => one(JustifyContent(Some(taffy::style::JustifyContent::SpaceEvenly))),
        "content-start" => one(AlignContent(Some(taffy::style::AlignContent::FlexStart))),
        "content-end" => one(AlignContent(Some(taffy::style::AlignContent::FlexEnd))),
        "content-center" => one(AlignContent(Some(taffy::style::AlignContent::Center))),
        "content-between" => one(AlignContent(Some(taffy::style::AlignContent::SpaceBetween))),
        "content-around" => one(AlignContent(Some(taffy::style::AlignContent::SpaceAround))),

        // position
        "static" => one(Position(taffy::style::Position::Relative)),
        "relative" => one(Position(taffy::style::Position::Relative)),
        // taffy has no `sticky`; absolute is the closest honest approximation.
        "absolute" | "fixed" | "sticky" => one(Position(taffy::style::Position::Absolute)),
        "inset-auto" => one(Inset(Sides::ALL, LengthPercentageAuto::auto())),

        // box model
        "box-border" => one(BoxSizing(taffy::style::BoxSizing::BorderBox)),
        "box-content" => one(BoxSizing(taffy::style::BoxSizing::ContentBox)),

        // overflow
        "overflow-auto" => one(OverflowAxis(Axis::Both, Overflow::Scroll)),
        "overflow-scroll" => one(OverflowAxis(Axis::Both, Overflow::Scroll)),
        "overflow-hidden" => one(OverflowAxis(Axis::Both, Overflow::Hidden)),
        "overflow-clip" => one(OverflowAxis(Axis::Both, Overflow::Clip)),
        "overflow-visible" => one(OverflowAxis(Axis::Both, Overflow::Visible)),
        "overflow-x-auto" | "overflow-x-scroll" => one(OverflowAxis(Axis::X, Overflow::Scroll)),
        "overflow-y-auto" | "overflow-y-scroll" => one(OverflowAxis(Axis::Y, Overflow::Scroll)),
        "overflow-x-hidden" => one(OverflowAxis(Axis::X, Overflow::Hidden)),
        "overflow-y-hidden" => one(OverflowAxis(Axis::Y, Overflow::Hidden)),

        // aspect ratio
        "aspect-square" => one(AspectRatio(Some(1.0))),
        "aspect-video" => one(AspectRatio(Some(16.0 / 9.0))),
        "aspect-auto" => one(AspectRatio(None)),

        // borders: bare `border` is 1dp on all sides
        "border" => one(BorderWidth(Sides::ALL, 1.0)),
        "border-x" => one(BorderWidth(Sides::X, 1.0)),
        "border-y" => one(BorderWidth(Sides::Y, 1.0)),
        "border-t" => one(BorderWidth(Sides::TOP, 1.0)),
        "border-r" => one(BorderWidth(Sides::RIGHT, 1.0)),
        "border-b" => one(BorderWidth(Sides::BOTTOM, 1.0)),
        "border-l" => one(BorderWidth(Sides::LEFT, 1.0)),

        // radii / shadow / ring bare forms
        "rounded" => one(Radius(Corners4::ALL, theme.radius("")?)),
        "shadow" => one(BoxShadow(theme.shadow("")?)),
        // v4's `--default-ring-width` is 1px; v3's was 3px. See `decl::DEFAULT_RING_WIDTH`.
        "ring" => one(Ring(crate::decl::DEFAULT_RING_WIDTH)),
        "ring-0" => one(Ring(0.0)),

        // `bg-none` — v4's "no background image", i.e. clear any gradient.
        "bg-none" => one(GradientLine(None)),

        // object-fit — how a replaced element's bitmap fills its own box. Purely a
        // paint-time property: taffy sizes the box identically whichever of these is
        // set, which is why adding them changes no layout anywhere.
        "object-fill" => one(ObjectFit(crate::style::ObjectFit::Fill)),
        "object-contain" => one(ObjectFit(crate::style::ObjectFit::Contain)),
        "object-cover" => one(ObjectFit(crate::style::ObjectFit::Cover)),
        "object-none" => one(ObjectFit(crate::style::ObjectFit::None)),
        "object-scale-down" => one(ObjectFit(crate::style::ObjectFit::ScaleDown)),

        // typography
        "italic" => one(Italic(true)),
        "not-italic" => one(Italic(false)),
        "underline" => one(Underline(true)),
        "line-through" => one(LineThrough(true)),
        "no-underline" => Some(crate::decl::no_underline().to_vec()),
        "text-left" | "text-start" => one(TextAlign(TextAlignment::Start)),
        "text-center" => one(TextAlign(TextAlignment::Center)),
        "text-right" | "text-end" => one(TextAlign(TextAlignment::End)),
        "text-justify" => one(TextAlign(TextAlignment::Justify)),
        "font-sans" => one(FontFamily(crate::style::FontFamily::Sans)),
        "font-serif" => one(FontFamily(crate::style::FontFamily::Serif)),
        "font-mono" => one(FontFamily(crate::style::FontFamily::Mono)),
        "tabular-nums" => one(TabularNums(true)),
        "normal-nums" => one(TabularNums(false)),

        // transforms
        "scale-100" => one(Scale(Axis::Both, 1.0)),
        _ => None,
    }
}

fn arbitrary(head: &str, body: &str, negative: bool, _theme: &Theme) -> Option<Vec<Decl>> {
    use Decl::*;
    let one = |d: Decl| Some(vec![d]);

    // Colour-shaped arbitrary values.
    if body.starts_with('#') {
        let c = arbitrary_color(body)?;
        return match head {
            "bg" => one(Background(c)),
            "text" => one(TextColor(c)),
            "border" => one(BorderColor(Sides::ALL, c)),
            "ring" => one(RingColor(c)),
            "from" => one(GradientStopColor(StopSlot::From, ExtColor::from_srgb8(c))),
            "via" => one(GradientStopColor(StopSlot::Via, ExtColor::from_srgb8(c))),
            "to" => one(GradientStopColor(StopSlot::To, ExtColor::from_srgb8(c))),
            _ => None,
        };
    }

    // `bg-linear-[30deg]`. Handled before `arbitrary_len`, which would read the same
    // body as a bare number and hand back a length.
    if head == "bg-linear" {
        let deg = body.strip_suffix("deg").unwrap_or(body);
        return one(GradientLine(Some((
            crate::style::GradientLine::Angle(finite_f32(deg)?),
            // No modifier is possible on this form — v4 emits a bare `30deg` with no
            // `in oklab` (measured), and CSS Images 4's own default for a gradient with
            // no interpolation keyword *is* oklab, so the rendered ramp is the same.
            GradientColorSpace::Oklab,
            GradientHue::Shorter,
        ))));
    }

    let raw = arbitrary_len(body)?;
    let len = if negative { raw.negate() } else { raw };

    match head {
        "w" => one(Size(Axis::X, len.to_dimension())),
        "h" => one(Size(Axis::Y, len.to_dimension())),
        "size" => one(Size(Axis::Both, len.to_dimension())),
        "min-w" => one(MinSize(Axis::X, len.to_dimension())),
        "min-h" => one(MinSize(Axis::Y, len.to_dimension())),
        "max-w" => one(MaxSize(Axis::X, len.to_dimension())),
        "max-h" => one(MaxSize(Axis::Y, len.to_dimension())),
        "basis" => one(FlexBasis(len.to_dimension())),
        "p" => one(Padding(Sides::ALL, len.to_lp())),
        "px" => one(Padding(Sides::X, len.to_lp())),
        "py" => one(Padding(Sides::Y, len.to_lp())),
        "pt" => one(Padding(Sides::TOP, len.to_lp())),
        "pr" => one(Padding(Sides::RIGHT, len.to_lp())),
        "pb" => one(Padding(Sides::BOTTOM, len.to_lp())),
        "pl" => one(Padding(Sides::LEFT, len.to_lp())),
        "m" => one(Margin(Sides::ALL, len.to_lpa())),
        "mx" => one(Margin(Sides::X, len.to_lpa())),
        "my" => one(Margin(Sides::Y, len.to_lpa())),
        "mt" => one(Margin(Sides::TOP, len.to_lpa())),
        "mr" => one(Margin(Sides::RIGHT, len.to_lpa())),
        "mb" => one(Margin(Sides::BOTTOM, len.to_lpa())),
        "ml" => one(Margin(Sides::LEFT, len.to_lpa())),
        "gap" => one(Gap(Axis::Both, len.to_lp())),
        "gap-x" => one(Gap(Axis::X, len.to_lp())),
        "gap-y" => one(Gap(Axis::Y, len.to_lp())),
        "inset" => one(Inset(Sides::ALL, len.to_lpa())),
        "top" => one(Inset(Sides::TOP, len.to_lpa())),
        "right" => one(Inset(Sides::RIGHT, len.to_lpa())),
        "bottom" => one(Inset(Sides::BOTTOM, len.to_lpa())),
        "left" => one(Inset(Sides::LEFT, len.to_lpa())),
        "text" => match len {
            Len::Px(v) => one(FontSize(v, None)),
            _ => None,
        },
        "leading" => match len {
            Len::Px(v) => one(Leading(LineHeight::Absolute(v))),
            _ => None,
        },
        "rounded" => match len {
            Len::Px(v) => one(Radius(Corners4::ALL, v)),
            _ => None,
        },
        "border" => match len {
            Len::Px(v) => one(BorderWidth(Sides::ALL, v)),
            _ => None,
        },
        "rotate" => match len {
            Len::Px(v) => one(Rotate(v)),
            _ => None,
        },
        _ => None,
    }
}

fn prefixed(head: &str, token: &str, negative: bool, theme: &Theme) -> Option<Vec<Decl>> {
    use Decl::*;
    let one = |d: Decl| Some(vec![d]);
    let len = |t: &str| -> Option<Len> {
        let l = size_token(t, theme)?;
        Some(if negative { l.negate() } else { l })
    };

    match head {
        // ---- sizing ----
        "w" => one(Size(Axis::X, len(token)?.to_dimension())),
        "h" => one(Size(Axis::Y, len(token)?.to_dimension())),
        "size" => one(Size(Axis::Both, len(token)?.to_dimension())),
        "basis" => one(FlexBasis(len(token)?.to_dimension())),
        "min" | "max" => {
            let (axis_tok, rest) = token.split_once('-')?;
            let axis = match axis_tok {
                "w" => Axis::X,
                "h" => Axis::Y,
                _ => return None,
            };
            let d = len(rest)?.to_dimension();
            one(if head == "min" { MinSize(axis, d) } else { MaxSize(axis, d) })
        }

        // ---- spacing ----
        "p" => one(Padding(Sides::ALL, len(token)?.to_lp())),
        "px" => one(Padding(Sides::X, len(token)?.to_lp())),
        "py" => one(Padding(Sides::Y, len(token)?.to_lp())),
        "pt" => one(Padding(Sides::TOP, len(token)?.to_lp())),
        "pr" => one(Padding(Sides::RIGHT, len(token)?.to_lp())),
        "pb" => one(Padding(Sides::BOTTOM, len(token)?.to_lp())),
        "pl" => one(Padding(Sides::LEFT, len(token)?.to_lp())),
        "m" => one(Margin(Sides::ALL, len(token)?.to_lpa())),
        "mx" => one(Margin(Sides::X, len(token)?.to_lpa())),
        "my" => one(Margin(Sides::Y, len(token)?.to_lpa())),
        "mt" => one(Margin(Sides::TOP, len(token)?.to_lpa())),
        "mr" => one(Margin(Sides::RIGHT, len(token)?.to_lpa())),
        "mb" => one(Margin(Sides::BOTTOM, len(token)?.to_lpa())),
        "ml" => one(Margin(Sides::LEFT, len(token)?.to_lpa())),
        "gap" => match token.split_once('-') {
            Some(("x", rest)) => one(Gap(Axis::X, len(rest)?.to_lp())),
            Some(("y", rest)) => one(Gap(Axis::Y, len(rest)?.to_lp())),
            _ => one(Gap(Axis::Both, len(token)?.to_lp())),
        },

        // ---- position offsets ----
        "inset" => match token.split_once('-') {
            Some(("x", rest)) => one(Inset(Sides::X, len(rest)?.to_lpa())),
            Some(("y", rest)) => one(Inset(Sides::Y, len(rest)?.to_lpa())),
            _ => one(Inset(Sides::ALL, len(token)?.to_lpa())),
        },
        "top" => one(Inset(Sides::TOP, len(token)?.to_lpa())),
        "right" => one(Inset(Sides::RIGHT, len(token)?.to_lpa())),
        "bottom" => one(Inset(Sides::BOTTOM, len(token)?.to_lpa())),
        "left" => one(Inset(Sides::LEFT, len(token)?.to_lpa())),

        // ---- grid ----
        "grid" => {
            let (kind, n) = token.split_once('-')?;
            let n: u16 = n.parse().ok()?;
            match kind {
                "cols" => one(GridCols(n)),
                "rows" => one(GridRows(n)),
                _ => None,
            }
        }
        "col" => one(ColSpan(token.strip_prefix("span-")?.parse().ok()?)),
        "row" => one(RowSpan(token.strip_prefix("span-")?.parse().ok()?)),

        // ---- colours & paint ----
        "bg" => {
            // A gradient line first: `linear-*`/`gradient-to-*` are not colours, and
            // `color_token` would reject them anyway — but trying it first keeps the
            // unknown-utility report honest for `bg-linear-to-nowhere`.
            if token.starts_with("linear-") || token.starts_with("gradient-") {
                return gradient_line_utility(token).map(|d| vec![d]);
            }
            one(Background(color_token(token, theme)?))
        }
        "from" => gradient_stop_utility(StopSlot::From, token, theme),
        "via" => gradient_stop_utility(StopSlot::Via, token, theme),
        "to" => gradient_stop_utility(StopSlot::To, token, theme),
        "border" => {
            // `border-2` (width), `border-t-2`, `border-red-500`, `border-t-red-500`.
            let (sides, rest) = match token.split_once('-') {
                Some(("x", r)) => (Sides::X, r),
                Some(("y", r)) => (Sides::Y, r),
                Some(("t", r)) => (Sides::TOP, r),
                Some(("r", r)) => (Sides::RIGHT, r),
                Some(("b", r)) => (Sides::BOTTOM, r),
                Some(("l", r)) => (Sides::LEFT, r),
                _ => match token {
                    "x" | "y" | "t" | "r" | "b" | "l" => return None, // handled in exact()
                    _ => (Sides::ALL, token),
                },
            };
            if let Some(w) = finite_f32(rest) {
                return one(BorderWidth(sides, w));
            }
            one(BorderColor(sides, color_token(rest, theme)?))
        }
        "rounded" => {
            let (corners, rest) = match token.split_once('-') {
                Some(("t", r)) => (Corners4::TOP, r),
                Some(("b", r)) => (Corners4::BOTTOM, r),
                Some(("l", r)) => (Corners4::LEFT, r),
                Some(("r", r)) => (Corners4::RIGHT, r),
                Some(("tl", r)) => (Corners4::TL, r),
                Some(("tr", r)) => (Corners4::TR, r),
                Some(("br", r)) => (Corners4::BR, r),
                Some(("bl", r)) => (Corners4::BL, r),
                _ => match token {
                    "t" => (Corners4::TOP, ""),
                    "b" => (Corners4::BOTTOM, ""),
                    "l" => (Corners4::LEFT, ""),
                    "r" => (Corners4::RIGHT, ""),
                    "tl" => (Corners4::TL, ""),
                    "tr" => (Corners4::TR, ""),
                    "br" => (Corners4::BR, ""),
                    "bl" => (Corners4::BL, ""),
                    _ => (Corners4::ALL, token),
                },
            };
            one(Radius(corners, theme.radius(rest)?))
        }
        "opacity" => {
            let v = finite_f32(token)?;
            one(Opacity((v / 100.0).clamp(0.0, 1.0)))
        }
        "shadow" => one(BoxShadow(theme.shadow(token)?)),
        "ring" => {
            if let Some(w) = finite_f32(token) {
                return one(Ring(w));
            }
            one(RingColor(color_token(token, theme)?))
        }

        // ---- typography ----
        // `text-*` is size, then colour. Tailwind resolves the same way.
        "text" => {
            if let Some((size, lh)) = theme.font_size(token) {
                return Some(vec![FontSize(size, Some(lh))]);
            }
            one(TextColor(color_token(token, theme)?))
        }
        "font" => one(FontWeight(theme.font_weight(token)?)),
        "leading" => one(Leading(theme.leading(token)?)),
        "tracking" => one(Tracking(theme.tracking(token)?)),

        // ---- transforms ----
        "rotate" => {
            let v: f32 = token.parse().ok()?;
            one(Rotate(if negative { -v } else { v }))
        }
        "scale" => {
            let (axis, rest) = match token.split_once('-') {
                Some(("x", r)) => (Axis::X, r),
                Some(("y", r)) => (Axis::Y, r),
                _ => (Axis::Both, token),
            };
            let v: f32 = rest.parse().ok()?;
            one(Scale(axis, v / 100.0))
        }
        "translate" => {
            let (axis, rest) = token.split_once('-')?;
            let axis = match axis {
                "x" => Axis::X,
                "y" => Axis::Y,
                _ => return None,
            };
            match len(rest)? {
                Len::Px(v) => one(Translate(axis, v)),
                _ => None,
            }
        }

        // ---- overflow with explicit axis and a spacing-free token ----
        "overflow" => {
            let (axis, rest) = match token.split_once('-') {
                Some(("x", r)) => (Axis::X, r),
                Some(("y", r)) => (Axis::Y, r),
                _ => (Axis::Both, token),
            };
            let o = match rest {
                "auto" | "scroll" => Overflow::Scroll,
                "hidden" => Overflow::Hidden,
                "clip" => Overflow::Clip,
                "visible" => Overflow::Visible,
                _ => return None,
            };
            one(OverflowAxis(axis, o))
        }

        _ => None,
    }
}
