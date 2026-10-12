//! Box decoration — shadows, the ring, and the border — resolved to plain geometry.
//!
//! ../PLAN.md §28. Until it, the native paint path drew exactly two things: a solid
//! rounded-rect background and glyph runs (§18.1 counted them). `twcore` resolved
//! `border-*`, `shadow-*` and `ring-*` into `Paint` and **nothing painted any of it**,
//! which made every one of them a silent divergence: the app renders, nothing errors,
//! and the effect is simply absent.
//!
//! **This module computes geometry, never pixels** — the same split `icons.rs` makes
//! and for the same reason (§18.3). Four consumers paint (`gpu_harness`'s GPU and CPU
//! paths, `conformance`, `spike1_render_pipeline`); each one re-deriving the corner
//! maths is exactly how the two backends' `Switch` designs drifted apart in §9.2 item
//! 3. So everything here is `kurbo` — a geometry crate with no rasteriser, which
//! `zkite_renderer` already depends on for icons, so this does not breach the crate's
//! "no vello dependency" rule (see the crate doc).
//!
//! # The coordinate contract
//!
//! Everything here is in **dp**, the same space as `PositionedNode::rect` (§13.1's
//! table). A backend applies its density transform to the scene and these paths scale
//! with it, exactly as icon paths do.
//!
//! # What CSS says, and where the primitives disagree with it
//!
//! Two conversions are load-bearing and both are easy to get subtly wrong:
//!
//! * **A CSS blur radius is not a Gaussian standard deviation.** `box-shadow`'s third
//!   length is defined so the shadow's edge transitions over roughly that distance,
//!   which is a Gaussian with `sigma = blur / 2`. Both vello backends'
//!   `fill_blurred_rounded_rect` take a `std_dev`. Passing the CSS blur straight
//!   through renders a shadow twice as soft as Chromium's — close enough to look
//!   deliberate and wrong everywhere.
//! * **A border is drawn *inside* the border box, and a ring *outside* it.** Both are
//!   painted here as strokes on a centreline, which is exact: a stroke of width `w`
//!   centred `w/2` inside the border box has its outer edge exactly on that box, and
//!   one centred `w/2` outside has its inner edge there.

use kurbo::{BezPath, Rect, RoundedRect, Shape};
use twcore::{BorderColors, Color, ComputedStyle, Corners, PaintColor};

/// One blurred rounded rect, ready for `fill_blurred_rounded_rect` on either backend.
///
/// `radius` and `std_dev` are single `f32`s because that is what the primitive takes —
/// see `BoxDecoration`'s own note on the non-uniform-radius approximation that forces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowRect {
    pub rect: Rect,
    pub radius: f64,
    /// Already converted from CSS's blur radius: `std_dev = blur / 2`.
    pub std_dev: f64,
    pub color: Color,
}

/// A stroked outline — the uniform border, and the ring.
#[derive(Debug, Clone)]
pub struct StrokedOutline {
    /// The centreline. Stroking this at `width` puts the band exactly where CSS does.
    pub path: BezPath,
    pub width: f64,
    pub color: Color,
}

/// One side of a per-side border: fill `band` clipped to `wedge`.
///
/// The general case needs this because Tailwind lets each side carry its own width and
/// colour (`border-t-2 border-b-4`, `border-l-red-500`), and `twcore` models both. Four
/// independent strokes would overlap at the corners and paint the wrong colour into the
/// overlap; CSS instead mitres the join, so each side owns the wedge of the corner
/// bounded by the diagonal from the outer corner to the inner one.
///
/// `band` is the full ring between the outer and inner rounded rects, as a single path
/// with the inner contour wound backwards so an ordinary non-zero fill leaves it
/// hollow. Clipping that to one side's `wedge` yields exactly that side's region,
/// corner arcs included, without needing boolean path operations (`kurbo` has none) or
/// an even-odd fill rule (the two backends' `fill_path` does not take one).
#[derive(Debug, Clone)]
pub struct BorderSide {
    pub band: BezPath,
    pub wedge: BezPath,
    pub color: Color,
}

/// How a node's border should be drawn, if at all.
#[derive(Debug, Clone)]
pub enum Border {
    /// Every side the same width and colour: one stroke, and it is *exact* — the
    /// per-side path below has anti-aliased seams along the mitre diagonals, so this
    /// is a fidelity fast path, not only a cheap one. It is also the overwhelmingly
    /// common case (`border`, `border-2 border-slate-700`).
    Uniform(StrokedOutline),
    /// Differing widths or colours. One entry per side that actually has both a
    /// non-zero width and a colour.
    PerSide(Vec<BorderSide>),
}

/// Everything to paint for one node *besides* its background, icon and text.
///
/// `None` throughout is the common case and costs three `Option` checks — most nodes
/// in a real tree carry no decoration at all, and building paths for them would put
/// allocation on the render hot path for no reason (the mistake §11.9 removed from
/// text shaping and §18.3 avoided for icon paths).
#[derive(Debug, Clone, Default)]
pub struct BoxDecoration {
    /// **Painted in reverse order**, because `box-shadow`'s first layer is on top.
    /// A painter walking this forwards puts the wrong layer above the other — visible
    /// wherever two layers have different alphas, which is every multi-layer Tailwind
    /// shadow.
    pub shadows: Vec<ShadowRect>,
    /// Tailwind's `ring-*`. Sits entirely outside the border box, so **it is not
    /// bounded by the node's own rect** and a clip is what stops it escaping its
    /// scroll container.
    pub ring: Option<StrokedOutline>,
    pub border: Option<Border>,
}

impl BoxDecoration {
    /// True when there is genuinely nothing to draw, so a painter can skip the node
    /// without inspecting three fields.
    pub fn is_empty(&self) -> bool {
        self.shadows.is_empty() && self.ring.is_none() && self.border.is_none()
    }
}

/// CSS `box-shadow`'s blur radius to the Gaussian standard deviation both vello
/// backends want. See this module's doc — the factor of two is the whole point.
pub fn blur_to_std_dev(blur: f32) -> f64 {
    blur as f64 / 2.0
}

/// The single radius the blurred-rect primitive accepts, for a possibly non-uniform
/// `Corners`, clamped to what fits in `rect`.
///
/// Exact when the corners agree, which is every `rounded-*` that is not a per-corner
/// utility. Otherwise the mean — a real approximation, named in §28's known gaps
/// rather than hidden, because neither backend exposes a per-corner blurred rect and
/// building one by hand would mean writing a blur.
///
/// **The clamp is not optional.** `rounded-full` is 9999dp, and handing that to
/// `fill_blurred_rounded_rect` asks for corner arcs far larger than the rect. `rounded`
/// already clamps for the stroked paths; a test caught this path not doing the same,
/// and the symptom would have been a garbled shadow under every pill-shaped button.
fn shadow_radius(rect: Rect, c: &Corners) -> f64 {
    let mean = if c.is_uniform() {
        c.top_left as f64
    } else {
        ((c.top_left + c.top_right + c.bottom_right + c.bottom_left) / 4.0) as f64
    };
    mean.clamp(0.0, (rect.width().min(rect.height()) / 2.0).max(0.0))
}

/// A rounded rect with every corner clamped to what actually fits.
///
/// Without the clamp a `rounded-full` (9999dp) box produces corner arcs larger than the
/// box, which `kurbo` will happily build and which paints as a garbled shape. CSS's own
/// rule is to scale all radii down by the same factor until no edge is over-subscribed;
/// clamping each corner to half the shorter side is the common case of that and is
/// exact for the uniform radii every Tailwind utility produces.
fn rounded(rect: Rect, c: &Corners) -> RoundedRect {
    let limit = (rect.width().min(rect.height()) / 2.0).max(0.0);
    let r = |v: f32| (v as f64).clamp(0.0, limit);
    RoundedRect::new(
        rect.x0,
        rect.y0,
        rect.x1,
        rect.y1,
        (r(c.top_left), r(c.top_right), r(c.bottom_right), r(c.bottom_left)),
    )
}

/// Grow (or, for a negative `d`, shrink) a rounded rect by `d` on every side, the way
/// CSS's `spread` and a stroke centreline both need.
///
/// The radius moves with the edge — a corner inset by `d` has radius `r - d` — which is
/// what keeps concentric rounded rects actually concentric. Clamped at zero: CSS says a
/// zero radius stays zero under positive spread, and a radius cannot go negative.
fn inflate(rect: Rect, c: &Corners, d: f64) -> (Rect, Corners) {
    let out = Rect::new(rect.x0 - d, rect.y0 - d, rect.x1 + d, rect.y1 + d);
    let adj = |v: f32| ((v as f64 + d).max(0.0)) as f32;
    (
        out,
        Corners {
            top_left: adj(c.top_left),
            top_right: adj(c.top_right),
            bottom_right: adj(c.bottom_right),
            bottom_left: adj(c.bottom_left),
        },
    )
}

/// Per-side border widths in dp, resolved from taffy's own `border` rect.
///
/// Read from `ComputedStyle::layout` rather than from a paint field because that is
/// where `Decl::BorderWidth` puts it — and because taffy has *already reserved* that
/// space in the layout, which is what makes painting a border purely additive and
/// unable to move a single pixel of geometry (§18.1).
fn border_widths(style: &ComputedStyle) -> [f32; 4] {
    let lp = |v: taffy::LengthPercentage| match v.into_raw().tag() {
        taffy::CompactLength::LENGTH_TAG => v.into_raw().value(),
        // A percentage border resolves against the containing block's width, which is
        // not known here. Tailwind emits no percentage border widths, so this is
        // unreachable from a class string; treated as zero rather than guessed.
        _ => 0.0,
    };
    let b = &style.layout.border;
    [lp(b.top), lp(b.right), lp(b.bottom), lp(b.left)]
}

/// The wedge of the corner-mitred box owned by one side.
///
/// Sides are indexed top/right/bottom/left. Each wedge is the quadrilateral from the
/// side's two outer corners in to its two inner corners, so adjacent wedges meet along
/// the corner diagonal — CSS's own mitre — and together they tile the whole band.
fn side_wedge(outer: Rect, inner: Rect, side: usize) -> BezPath {
    let (ox0, oy0, ox1, oy1) = (outer.x0, outer.y0, outer.x1, outer.y1);
    let (ix0, iy0, ix1, iy1) = (inner.x0, inner.y0, inner.x1, inner.y1);
    let pts = match side {
        0 => [(ox0, oy0), (ox1, oy0), (ix1, iy0), (ix0, iy0)], // top
        1 => [(ox1, oy0), (ox1, oy1), (ix1, iy1), (ix1, iy0)], // right
        2 => [(ox1, oy1), (ox0, oy1), (ix0, iy1), (ix1, iy1)], // bottom
        _ => [(ox0, oy1), (ox0, oy0), (ix0, iy0), (ix0, iy1)], // left
    };
    let mut p = BezPath::new();
    p.move_to(pts[0]);
    for q in &pts[1..] {
        p.line_to(*q);
    }
    p.close_path();
    p
}

/// The hollow band between two rounded rects, as one non-zero-fillable path.
///
/// The inner contour is reversed so its winding opposes the outer one; filling the
/// result with the default non-zero rule leaves the middle empty. This is the standard
/// donut construction and it is what lets a per-side border be drawn without boolean
/// path ops or a fill-rule parameter, neither of which is available here.
fn band_path(outer: RoundedRect, inner: RoundedRect) -> BezPath {
    const TOL: f64 = 0.05;
    let mut p = outer.to_path(TOL);
    p.extend(inner.to_path(TOL).reverse_subpaths());
    p
}

/// Resolve everything paintable about a node's box, in dp.
///
/// `rect` is the node's own `PositionedNode::rect` — the **border box**, which is what
/// taffy computes and what CSS calls the outer edge of the border.
///
/// `current_color` is the node's resolved (inherited) text colour, needed because
/// Tailwind v4 defaults both `border-color` and `ring-color` to `currentColor`
/// (§28.1). Passing `None` where a colour was inherited would silently paint every
/// undecorated border black.
pub fn box_decoration(
    rect: (f32, f32, f32, f32),
    style: &ComputedStyle,
    current_color: Option<Color>,
) -> BoxDecoration {
    let paint = &style.paint;
    let widths = border_widths(style);
    let has_border = widths.iter().any(|w| *w > 0.0);

    // The early-out that keeps this free for the overwhelming majority of nodes.
    if paint.shadow.is_none() && paint.ring.is_none() && !has_border {
        return BoxDecoration::default();
    }

    let (x, y, w, h) = rect;
    let border_box = Rect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64);
    let radius = &paint.radius;

    let mut out = BoxDecoration::default();

    // ---- shadows -------------------------------------------------------------
    if let Some(shadow) = paint.shadow {
        for layer in shadow.layers() {
            // CSS: offset the border box, then inflate by `spread`, then blur. The
            // radius follows the spread, which `inflate` handles.
            let offset = border_box + kurbo::Vec2::new(layer.offset_x as f64, layer.offset_y as f64);
            let (r, c) = inflate(offset, radius, layer.spread as f64);
            if r.width() <= 0.0 || r.height() <= 0.0 {
                // A spread negative enough to collapse the rect draws nothing, rather
                // than an inverted rect the primitive would normalise into a visible
                // artefact.
                continue;
            }
            out.shadows.push(ShadowRect {
                rect: r,
                radius: shadow_radius(r, &c),
                std_dev: blur_to_std_dev(layer.blur),
                color: layer.color,
            });
        }
    }

    // ---- ring ----------------------------------------------------------------
    // A spread-only box-shadow outside the border box: inner edge exactly on the box,
    // outer edge `w` beyond it. Stroking the centreline at `w` gives precisely that.
    if let Some((rw, rc)) = paint.ring {
        if rw > 0.0 {
            let (r, c) = inflate(border_box, radius, rw as f64 / 2.0);
            out.ring = Some(StrokedOutline {
                path: rounded(r, &c).to_path(0.05),
                width: rw as f64,
                color: rc.resolve(current_color),
            });
        }
    }

    // ---- border --------------------------------------------------------------
    if has_border {
        out.border = build_border(border_box, radius, widths, &paint.border_color, current_color);
    }

    out
}

fn build_border(
    border_box: Rect,
    radius: &Corners,
    widths: [f32; 4],
    colors: &BorderColors,
    current_color: Option<Color>,
) -> Option<Border> {
    let uniform_width = widths.iter().all(|w| *w == widths[0]) && widths[0] > 0.0;
    let side_colors = [colors.top, colors.right, colors.bottom, colors.left];

    if uniform_width {
        if let Some(c) = colors.uniform() {
            let w = widths[0] as f64;
            // Centreline: inset half the width, radius following the edge, so the outer
            // edge lands exactly on the border box and the inner edge exactly on the
            // padding box — which is what CSS draws.
            let (r, rc) = inflate(border_box, radius, -w / 2.0);
            if r.width() <= 0.0 || r.height() <= 0.0 {
                return None;
            }
            return Some(Border::Uniform(StrokedOutline {
                path: rounded(r, &rc).to_path(0.05),
                width: w,
                color: c.resolve(current_color),
            }));
        }
    }

    // General case. The inner edge is per-side, so the padding box is not a uniform
    // inset of the border box and the band has to be built from two real rects.
    let inner = Rect::new(
        border_box.x0 + widths[3] as f64,
        border_box.y0 + widths[0] as f64,
        border_box.x1 - widths[1] as f64,
        border_box.y1 - widths[2] as f64,
    );
    if inner.width() < 0.0 || inner.height() < 0.0 {
        return None;
    }
    let outer_rr = rounded(border_box, radius);
    // Each inner corner's radius shrinks by the *adjacent* border width, which is what
    // makes a thick border on one side alone still meet its neighbours cleanly.
    let inner_radius = Corners {
        top_left: (radius.top_left - widths[3].max(widths[0])).max(0.0),
        top_right: (radius.top_right - widths[1].max(widths[0])).max(0.0),
        bottom_right: (radius.bottom_right - widths[1].max(widths[2])).max(0.0),
        bottom_left: (radius.bottom_left - widths[3].max(widths[2])).max(0.0),
    };
    let inner_rr = rounded(inner, &inner_radius);
    let band = band_path(outer_rr, inner_rr);

    let mut sides = Vec::new();
    for (i, width) in widths.iter().enumerate() {
        if *width <= 0.0 {
            continue;
        }
        let Some(color) = side_colors[i] else { continue };
        sides.push(BorderSide {
            band: band.clone(),
            wedge: side_wedge(border_box, inner, i),
            color: color.resolve(current_color),
        });
    }
    if sides.is_empty() {
        return None;
    }
    Some(Border::PerSide(sides))
}

/// Resolve a border colour without building any geometry — for a caller that only
/// needs to know what colour a side would be (tests, diagnostics).
pub fn resolve_border_color(c: Option<PaintColor>, current: Option<Color>) -> Option<Color> {
    c.map(|c| c.resolve(current))
}
