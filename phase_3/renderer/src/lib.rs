//! The shared layout + text-measurement engine for phase 3's native renderer.
//!
//! `twcore` class string --> `ComputedStyle` (unchanged from phase 0) --> a `taffy`
//! tree whose text leaves are measured for real, via `parley` shaping, using taffy's
//! own `NodeContext` + `compute_layout_with_measure` mechanism — see ../PLAN.md §3's
//! "known gap" write-up for why this exists: without real measurement, a text leaf's
//! box is whatever the caller happened to hardcode, and centering it inside a flex
//! parent centers the *box*, not the *line*, which is wrong the moment the box and
//! the shaped text disagree (they always do; box dimensions were a workaround, not a
//! fix). With measurement, `items-center`/`justify-center` on an ordinary flex parent
//! is sufficient and correct, exactly as it would be for the WebView backend.
//!
//! Deliberately has **no dependency on `vello_cpu` or `vello_hybrid`**. Layout is one
//! engine shared by every paint backend; paint is the one thing that legitimately
//! differs per backend (CPU pixels vs. a GPU scene graph). Each backend's own ~30
//! lines of "walk `PositionedNode`s and call my own fill_rect/glyph_run" stays in that
//! backend's own crate. See ../PLAN.md §3/§4.

use std::sync::Arc;

use parley::fontique::GenericFamily;
use parley::style::{FontFamily as ParleyFontFamily, FontFamilyName};
use parley::{Affinity, Cursor, FontContext, LayoutContext, PositionedLayoutItem, StyleProperty};
use rustc_hash::FxHashMap;
use taffy::prelude::*;
pub use taffy::NodeId;
use twcore::Decl;
pub use twcore::{Color, ComputedStyle, Corners, NodeState, TextStyle};

pub mod decoration;
pub mod gradient;
pub mod icons;
pub use decoration::{
    box_decoration, Border, BorderSide, BoxDecoration, ShadowRect, StrokedOutline,
};
pub use gradient::background_gradient;
pub use icons::{IconCap, IconJoin, IconPaint, IconShape, IconSprite, IconStroke};

/// Per-leaf data needed to measure and, later, paint a run of text. Containers carry
/// no context at all (`taffy::TaffyTree::new_leaf`/`new_with_children`) — only text
/// leaves do, which is exactly the set of nodes that need a `parley` shaping pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextOptions {
    pub wrap: bool,
    pub max_lines: Option<usize>,
    pub ellipsis: bool,
}
impl Default for TextOptions {
    fn default() -> Self { Self { wrap: true, max_lines: None, ellipsis: false } }
}

struct TextContext {
    options: TextOptions,
    content: Arc<str>,
    style: Arc<ComputedStyle>,
}

/// One shaped glyph, already positioned relative to its own text node's top-left
/// corner (i.e. add the node's `rect.0`/`rect.1` to place it on the full canvas).
/// Deliberately the same three fields as `glifo::Glyph` (which both `vello_cpu` and
/// `vello_hybrid` build on) so a consumer's paint loop is a one-line `.map()`, but
/// defined locally so this crate doesn't have to depend on either vello flavor to
/// produce it.
#[derive(Clone, Copy, Debug)]
pub struct PositionedGlyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

/// Which axis a scroll question is being asked about.
///
/// Exists because every scroll query in this crate used to be y-only by construction
/// (../PLAN.md §6.3 said so about fling; it was true of dragging, of `max_scroll_y`
/// and of `is_scrollable_y` too), and a caller that wants horizontal scrolling needs
/// to ask the *same* questions about x. The ordering of the two variants is not
/// meaningful; `ScrollAxis` is deliberately not `Copy`-into-an-index anywhere, so a
/// caller has to name the axis it means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScrollAxis {
    X,
    Y,
}

/// One sprite shape with its path data already parsed, in the sprite's own viewBox
/// units. A paint backend places it with a transform rather than by rebuilding the
/// path — see `ResolvedIcon`.
pub struct IconShapeGeometry {
    pub path: kurbo::BezPath,
    pub fill: IconPaint,
    pub stroke: Option<IconStroke>,
}

/// A sprite with every `d` string parsed into a `kurbo::BezPath`, **once per process**
/// rather than once per frame.
///
/// The paths stay in the sprite's own viewBox space and are never rebuilt at a node's
/// size. Both `vello_hybrid::Scene` and `vello_cpu::RenderContext` apply the current
/// transform to a path handed to `fill_path`/`stroke_path`, so a backend sets one
/// `Affine` (translate to the node, scale `extent` -> the node's box) and reuses this
/// exact path for every icon of that name on screen. Rebuilding geometry per node per
/// frame would put SVG parsing on the render hot path, which is the mistake ../PLAN.md
/// §11.9 spent a whole section removing from text shaping.
pub struct ResolvedIcon {
    pub extent: f32,
    pub shapes: Vec<IconShapeGeometry>,
}

impl ResolvedIcon {
    /// The transform placing this sprite inside a node's `rect`, to be composed *on top
    /// of* whatever transform the backend already has set (the density scale, ../PLAN.md
    /// §13.1) rather than replacing it.
    ///
    /// Uniform scale plus centring — SVG's own `preserveAspectRatio="xMidYMid meet"`
    /// default, which is what the WebView's `<use href="#icon-...">` gets. Scaling x and
    /// y independently would be the easy mistake and would visibly distort every icon in
    /// a non-square box.
    ///
    /// Lives here rather than in each paint backend because it is geometry, not paint:
    /// four consumers (`gpu_harness`'s two paths, `conformance`, `spike1`) would
    /// otherwise each re-derive it, which is exactly how the two backends' `Switch`
    /// designs drifted apart in ../PLAN.md §9.2 item 3.
    pub fn transform(&self, rect: (f32, f32, f32, f32)) -> kurbo::Affine {
        let (x, y, w, h) = rect;
        let scale = (w.min(h) / self.extent) as f64;
        let drawn = self.extent as f64 * scale;
        kurbo::Affine::translate((
            x as f64 + (w as f64 - drawn) / 2.0,
            y as f64 + (h as f64 - drawn) / 2.0,
        )) * kurbo::Affine::scale(scale)
    }
}

impl IconPaint {
    /// The colour to paint with, or `None` for "don't paint this at all".
    ///
    /// `current` is the node's own resolved text colour — the same value its glyphs
    /// would be painted in — which is what SVG's `currentColor` means and is why
    /// `Icon("text-white", "star")` is white. Falling back to black when a node has no
    /// resolved colour matches both SVG's initial `color` and what the text paint path
    /// in each backend already does for a glyph run.
    pub fn resolve(self, current: Option<Color>) -> Option<Color> {
        match self {
            IconPaint::None => None,
            IconPaint::Current => Some(current.unwrap_or(Color { r: 0, g: 0, b: 0, a: 255 })),
            IconPaint::Rgb(r, g, b) => Some(Color { r, g, b, a: 255 }),
        }
    }
}

impl IconStroke {
    /// The `kurbo::Stroke` for this shape, in the sprite's own viewBox units.
    ///
    /// The width is deliberately *not* pre-scaled: both backends stroke through the
    /// current transform, so `ResolvedIcon::transform` scales the stroke along with the
    /// geometry, exactly as a browser scales a stroke declared inside a `viewBox`.
    pub fn to_kurbo(self) -> kurbo::Stroke {
        let cap = match self.cap {
            IconCap::Butt => kurbo::Cap::Butt,
            IconCap::Round => kurbo::Cap::Round,
            IconCap::Square => kurbo::Cap::Square,
        };
        let join = match self.join {
            IconJoin::Miter => kurbo::Join::Miter,
            IconJoin::Round => kurbo::Join::Round,
            IconJoin::Bevel => kurbo::Join::Bevel,
        };
        kurbo::Stroke::new(self.width as f64).with_caps(cap).with_join(join)
    }
}

/// One same-font, same-size run of glyphs — the unit both vello backends' `glyph_run`
/// builder wants.
#[derive(Clone)]
pub struct GlyphRun {
    pub font: parley::FontData,
    pub font_size: f32,
    pub glyphs: Vec<PositionedGlyph>,
}

/// A node after layout: its absolute (canvas-space) rect, its resolved style, and —
/// for text leaves — the glyph runs shaped during measurement (not re-shaped during
/// paint; measurement and paint must agree by construction, not by convention).
pub struct PositionedNode {
    pub node: NodeId,
    /// (x, y, width, height), absolute — already walked down from the root, unlike
    /// `taffy::Layout::location`, which is parent-relative. Already reflects any
    /// ancestor scroll offset passed to `positioned`/`hit_test` — this is where a
    /// node actually is on screen right now, not its unscrolled layout position.
    pub rect: (f32, f32, f32, f32),
    pub style: Arc<ComputedStyle>,
    pub text: Option<Vec<GlyphRun>>,
    /// For an `Icon` leaf, the parsed sprite geometry to draw inside `rect`; `None`
    /// for every other node, and also for an `Icon` whose name matches no bundled
    /// sprite (which renders as the empty box every icon rendered as before this
    /// existed, rather than panicking on a string that came from user code).
    ///
    /// Shared, not cloned — an `Arc` per node costs a refcount bump where cloning the
    /// paths would copy real geometry for every icon on screen.
    pub icon: Option<Arc<ResolvedIcon>>,
    /// For an `Image` leaf, the `src` it was given; `None` for every other node, and
    /// for an `Image` with an empty `src`.
    ///
    /// **A source string, not pixels.** This crate never opens the file — see
    /// `Renderer::image`. The paint layer owns decoding, caching and the
    /// `object-fit` geometry (which needs the bitmap's intrinsic size, and therefore
    /// cannot be computed here), so what crosses this boundary is the identity of the
    /// image and nothing else. `style.paint.object_fit` travels alongside it in the
    /// ordinary `ComputedStyle`.
    pub image: Option<Arc<str>>,
    /// The rect (absolute, same coordinate space as `rect`) this node must be
    /// painted within, if any ancestor has `overflow: scroll/auto/hidden`. `None` —
    /// the common case — means unclipped. A paint backend that ignores this renders
    /// scrolled-past content bleeding outside its container instead of hidden by it;
    /// see `../gpu_harness/src/render_thread.rs`'s paint functions for how to use it
    /// (`push_clip_path`/`pop_clip_path`, present on both `vello_cpu::RenderContext`
    /// and `vello_hybrid::Scene` with an identical signature).
    pub clip: Option<(f32, f32, f32, f32)>,
    /// How many entries of the returned `Vec` this node's subtree occupies, counting
    /// itself — so the subtree is exactly `nodes[i..i + subtree_len]`.
    ///
    /// `positioned()` returns a flat, depth-first, pre-order list, which is all a
    /// paint loop needs for anything a node draws *by itself*. CSS `opacity` is the
    /// first property that is not like that: it composites a node **and everything
    /// under it** and fades the result, so a painter has to know where the subtree
    /// ends in order to `pop_layer` there (../PLAN.md §45.3). Recorded here rather
    /// than recovered by a second tree walk in each of the three paint loops, and
    /// exact rather than approximate because the pre-order walk makes a subtree a
    /// contiguous range by construction.
    ///
    /// Always at least 1. A leaf's is exactly 1.
    pub subtree_len: usize,
}

/// Turns CSS `opacity` into `push_opacity_layer`/`pop_layer` calls over a flat,
/// pre-order [`PositionedNode`] list — ../PLAN.md §45.3.
///
/// **This is a shared decision, not a shared painter.** The paint loop exists three
/// times in this repo — `gpu_harness`'s `paint_nodes_into_scene` (vello_hybrid) and
/// `paint_cpu_fallback` (vello_cpu), plus `conformance`'s own — because each talks to
/// a different scene type with no common trait between them. That is exactly how a
/// property comes to be implemented in one and absent in the others, which is the
/// class of bug §45 exists to close rather than to widen. The *arithmetic* of "which
/// layers are open right now" lives here, once, tested once; each painter contributes
/// only the two calls its own backend spells differently.
///
/// Usage, per painter:
///
/// ```ignore
/// let mut groups = OpacityGroups::new();
/// for (i, n) in nodes.iter().enumerate() {
///     for _ in 0..groups.close_before(i) { scene.pop_layer(); }
///     if let Some(alpha) = groups.open_at(i, n) { scene.push_opacity_layer(alpha); }
///     // ... paint node `n` ...
/// }
/// for _ in 0..groups.close_all() { scene.pop_layer(); }
/// ```
///
/// `close_before` must be called for **every** index, including nodes a painter is
/// about to skip for having nothing to draw — a layer opened by an ancestor has to be
/// closed at the right place whether or not the node sitting there paints anything.
#[derive(Debug, Default)]
pub struct OpacityGroups {
    /// Indices at which an open layer ends, innermost last. Pushed in walk order, so
    /// an inner group's end is always ≤ its enclosing group's and the stack pops in
    /// the right order without sorting.
    pop_at: Vec<usize>,
}

impl OpacityGroups {
    pub fn new() -> OpacityGroups {
        OpacityGroups::default()
    }

    /// How many layers must be popped before painting `nodes[i]`.
    pub fn close_before(&mut self, i: usize) -> usize {
        let mut n = 0;
        while self.pop_at.last() == Some(&i) {
            self.pop_at.pop();
            n += 1;
        }
        n
    }

    /// The alpha to open a group with at `i`, if this node declares one.
    ///
    /// A fully transparent node still opens a layer rather than skipping its subtree.
    /// Skipping would be faster and would silently change hit-testing's neighbour —
    /// nothing here paints, so the only thing that could go wrong is a painter
    /// growing a "so don't bother walking it" shortcut that `hit_test` does not
    /// share. Kept honest; `opacity-0` is rare and an animation passes through it for
    /// at most one frame.
    pub fn open_at(&mut self, i: usize, node: &PositionedNode) -> Option<f32> {
        let alpha = node.style.paint.opacity;
        if alpha >= 1.0 {
            return None;
        }
        self.pop_at.push(i + node.subtree_len);
        Some(alpha.clamp(0.0, 1.0))
    }

    /// How many layers are still open once the list is exhausted. Should be zero for
    /// a well-formed list — a node's subtree cannot extend past the end of the tree —
    /// but a painter that leaves a layer open corrupts every subsequent frame, so
    /// closing out is cheap insurance rather than an assertion.
    pub fn close_all(&mut self) -> usize {
        let n = self.pop_at.len();
        self.pop_at.clear();
        n
    }
}

/// Everything one `parley` shaping pass consumes, and therefore everything that can
/// change its result. Two text leaves with equal `ShapeKey`s shape to byte-identical
/// glyph runs and an identical measured size, which is what makes reusing the result
/// sound rather than merely plausible.
///
/// **The whole `TextStyle` is held, not the subset that currently reaches parley.**
/// `color` and `decoration` are ignored by every shaping call today, so including
/// them costs a few redundant entries when two nodes differ only in colour — and
/// that is the deliberate trade: over-keying wastes cache slots, under-keying returns
/// glyphs shaped for someone else's style. A `TextStyle` field added later is covered
/// automatically by the derived `PartialEq` instead of needing to be remembered here,
/// which is exactly the failure mode ../PLAN.md §9.7 found twice (`family` and
/// `line_height` were both parsed, inherited, and then read by nothing).
///
/// `font_cx`'s registered families are *not* part of this key because `Renderer::new`
/// is the only thing that ever registers a face and it runs once, before any shaping.
/// Registering fonts after construction would invalidate every entry here.
#[derive(Clone, PartialEq)]
struct ShapeKey {
    options: TextOptions,
    content: Arc<str>,
    style: TextStyle,
    /// `break_all_lines`'s `max_advance`: the width text may wrap within, or `None`
    /// for an unconstrained (min/max-content) measure pass.
    wrap_width: Option<f32>,
    /// Whether the caller's width was *known* rather than merely available, which is
    /// what decides if `plyt.align(..)` runs — see `layout`'s own comment on why
    /// alignment is conditional. Not derivable from `wrap_width` alone: a definite
    /// available space and a known dimension both produce `Some(w)` here.
    aligned: bool,
}

/// Reflexive for every value this renderer actually produces. A `NaN` font size or
/// letter spacing would break that (`NaN != NaN`), and the consequence is a key that
/// never equals itself — a permanent cache *miss* for that one node, i.e. exactly
/// today's behaviour, never a wrong answer for a different one.
impl Eq for ShapeKey {}

impl std::hash::Hash for ShapeKey {
    /// Deliberately hashes a cheap, discriminating subset while `Eq` compares
    /// everything. That is the contract a hash map needs (equal keys must hash
    /// equal; unequal keys are allowed to collide and are then separated by `Eq`),
    /// and it keeps a per-node, per-frame hash down to a short string plus four
    /// scalars instead of walking ten style fields, most of which are identical
    /// across every node in a real app anyway.
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.options.hash(state);
        self.content.hash(state);
        self.style.size.to_bits().hash(state);
        self.style.weight.hash(state);
        // By discriminant rather than by value, so this needs no `Hash` derive added
        // to `twcore::FontFamily` — a fieldless enum's discriminant *is* its value.
        std::mem::discriminant(&self.style.family).hash(state);
        self.wrap_width.map(f32::to_bits).hash(state);
        self.aligned.hash(state);
    }
}

/// The reusable half of a shaping pass: the measured size taffy asked for, and the
/// glyph runs paint will need. `Arc` so a cache hit hands the same runs to `shaped`
/// without copying every glyph — `positioned()` still deep-clones per node, so the
/// public `PositionedNode::text` contract is unchanged.
#[derive(Clone)]
struct ShapedText {
    width: f32,
    height: f32,
    runs: Arc<Vec<GlyphRun>>,
}

/// How the shaped-text cache is doing, for the callers whose whole reason to exist is
/// measuring this (`../gpu_harness`'s per-frame redraw log, the host benchmark).
///
/// Kept as permanent instrumentation rather than a temporary hack, for the reason
/// ../PLAN.md §11.5 records: a cache that silently stopped hitting would look exactly
/// like a slow device, and nothing else in the frame would report it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShapeCacheStats {
    /// Shaping passes served from the cache since this `Renderer` was created.
    pub hits: u64,
    /// Shaping passes that had to run `parley` for real.
    pub misses: u64,
    /// Entries currently retained across both generations — the live working set,
    /// which is what bounds this cache's memory.
    pub entries: usize,
}

impl ShapeCacheStats {
    /// Fraction of shaping passes served from cache, 0.0–1.0. `0.0` when nothing has
    /// been shaped at all, rather than a division by zero.
    pub fn hit_rate(&self) -> f32 {
        let total = self.hits + self.misses;
        if total == 0 {
            return 0.0;
        }
        self.hits as f32 / total as f32
    }
}

pub struct Renderer {
    tree: TaffyTree<TextContext>,
    resolver: twcore::Resolver,
    font_cx: FontContext,
    layout_cx: LayoutContext<()>,
    /// Shaped text reused across frames, in two generations — see `layout`'s own
    /// account of why rotation rather than an LRU. Deliberately *not* cleared by
    /// `clear()`: this is warm state in the same bucket as `font_cx`/`layout_cx`, and
    /// clearing it per render would defeat its entire purpose.
    shape_cache_cur: FxHashMap<ShapeKey, ShapedText>,
    shape_cache_prev: FxHashMap<ShapeKey, ShapedText>,
    shape_cache_hits: u64,
    shape_cache_misses: u64,
    /// Populated during `compute_layout`'s measure pass, read back during
    /// `positioned()`. Keyed by taffy `NodeId` so a node created, measured, then
    /// dropped and replaced (a future re-render) never reads stale glyphs — callers
    /// build a fresh `Renderer` per render, matching how `zkite_py` re-renders the
    /// whole tree on every `@store.update()` rather than patching in place.
    ///
    /// Holds the *same* `Arc` the shape cache does, so a cache hit costs a refcount
    /// bump rather than a glyph copy. `positioned()` still hands each consumer its own
    /// `Vec<GlyphRun>`, so nothing downstream can observe the sharing.
    shaped: FxHashMap<NodeId, Arc<Vec<GlyphRun>>>,
    /// Containers carry no taffy `NodeContext` (only text leaves need one, for
    /// measurement), so their resolved style — needed later for paint, e.g. a
    /// `Button`'s own background/radius — is tracked here instead.
    container_styles: FxHashMap<NodeId, Arc<ComputedStyle>>,
    /// Which nodes are `Icon` leaves, and what geometry each draws. Rebuilt per render
    /// like every other `NodeId`-keyed map.
    icons: FxHashMap<NodeId, Arc<ResolvedIcon>>,
    /// Which nodes are `Image` leaves, and what `src` each names. Rebuilt per render.
    ///
    /// There is deliberately no `image_geometry` twin of `icon_geometry` next to this:
    /// a sprite parses to geometry this crate can own, whereas a bitmap decodes to
    /// pixels only a paint backend can use (and, on the GPU path, has to *upload*
    /// before it can be drawn). The warm cache for images therefore lives in the paint
    /// layer, keyed by the same `src` strings this map holds — see
    /// `../gpu_harness/src/images.rs`.
    images: FxHashMap<NodeId, Arc<str>>,
    /// Sprite name -> parsed geometry, **not** cleared by `clear()`. Same bucket as
    /// `font_cx` and the shape cache: expensive to build, identical every render, and
    /// bounded by the size of the bundled sprite set (12) rather than by anything an
    /// app does — so unlike `twcore`'s own unbounded caches (../PLAN.md §11.7) this one
    /// cannot grow with use.
    icon_geometry: FxHashMap<&'static str, Arc<ResolvedIcon>>,
    /// Stack of resolved `TextStyle`s, innermost (current) container last — the
    /// mechanism behind `resolve_inherited`. CSS inherits `color`/`font-*`/etc. from
    /// the nearest ancestor by default (that's how `Button("... text-white
    /// font-medium ...", "Increment")` colors its bare-string "Increment" child in
    /// the WebView backend, with no class string of its own — see
    /// `phase_1/zkite_py/tests/test_app_counter.py`'s reference app, which relies on
    /// exactly this). `twcore::ComputedStyle` has no notion of ancestry at all — every
    /// `resolve()` starts from `ComputedStyle::default()` — so without this, every
    /// bare-string child anywhere in a real app would render in default black/16px/400
    /// instead of inheriting its container's typography. `container()` pushes before
    /// building its children and pops after; `text()` and nested `container()` calls
    /// read the top of the stack. Starts with one `TextStyle::default()` so a
    /// top-level `text()`/`container()` call always has something to read.
    inherited: Vec<TextStyle>,
}

/// Which `TextStyle` fields a class string's own (variant-matched) declarations set
/// explicitly — the rest inherit from the parent. See `Renderer::resolve_inherited`.
#[derive(Default)]
struct TouchedText {
    color: bool,
    size: bool,
    line_height: bool,
    weight: bool,
    italic: bool,
    letter_spacing: bool,
    align: bool,
    decoration: bool,
    family: bool,
    tabular_nums: bool,
}

impl TouchedText {
    fn all(&self) -> bool {
        self.color
            && self.size
            && self.line_height
            && self.weight
            && self.italic
            && self.letter_spacing
            && self.align
            && self.decoration
            && self.family
            && self.tabular_nums
    }
}

/// The regular + bold faces of one typeface family, as font file bytes (TTF/OTF).
///
/// Both are required. They must be genuine regular/bold variants of the *same* family:
/// fontique groups fonts into one `FamilyId` by the family name parsed out of each
/// file, not by call site, so registering them through separate `register_fonts` calls
/// still lands them together (confirmed directly against fontique 0.11's
/// `CommonData::register_fonts`, which resolves the family by
/// `family_names.get_or_insert(family_name)` regardless of how many times it's called).
/// `Renderer::new` checks this rather than trusting it, because getting it wrong
/// produces no error at all — just text that silently never goes bold.
#[derive(Clone)]
pub struct FontFaces {
    pub regular: Vec<u8>,
    pub bold: Vec<u8>,
}

impl FontFaces {
    pub fn new(regular: Vec<u8>, bold: Vec<u8>) -> Self {
        Self { regular, bold }
    }
}

/// The typefaces a `Renderer` picks between for `font-sans` / `font-serif` /
/// `font-mono` — the three families `twcore::FontFamily` can express.
///
/// `sans` is required because it is the default every text node inherits when no
/// `font-*` class appears anywhere above it. `serif`/`mono` are optional: a backend
/// that only wants to ship one typeface passes `FontSet::sans_only(..)` and every
/// family resolves to it, which is exactly the behaviour this crate had before font
/// families were resolved at all — so "only bundled one font" degrades to the old
/// behaviour rather than to missing text.
///
/// Deliberately not discovered from the system — see ../PLAN.md §3: there is no
/// fontconfig on Android, so a real ZKite app bundles its own font assets, and every
/// caller of this crate (host spike, GPU harness, conformance) does the same rather
/// than relying on whatever happens to be on the machine running it.
pub struct FontSet {
    pub sans: FontFaces,
    pub serif: Option<FontFaces>,
    pub mono: Option<FontFaces>,
}

impl FontSet {
    /// One family for all three generics — see the note on `serif`/`mono` above.
    pub fn sans_only(regular: Vec<u8>, bold: Vec<u8>) -> Self {
        Self { sans: FontFaces::new(regular, bold), serif: None, mono: None }
    }
}

/// Registers one family's two faces and returns the `FamilyId` they share.
///
/// The equality check is the load-bearing part: handing this a regular face and some
/// *other* typeface's bold face registers two unrelated families, and the only symptom
/// is that `font-bold` silently never changes anything — no error, no warning, and a
/// rendered result that looks plausible. `which` names the offending family in the
/// message so a caller with three of them knows which one it got wrong.
fn register_family(
    font_cx: &mut FontContext,
    faces: &FontFaces,
    which: &str,
) -> Result<parley::fontique::FamilyId, String> {
    let regular = font_cx
        .collection
        .register_fonts(parley::fontique::Blob::new(Arc::new(faces.regular.clone())), None);
    let family_id = regular
        .first()
        .ok_or_else(|| format!("{which} regular font bytes contained no usable font face"))?
        .0;
    let bold = font_cx
        .collection
        .register_fonts(parley::fontique::Blob::new(Arc::new(faces.bold.clone())), None);
    let bold_family_id = bold
        .first()
        .ok_or_else(|| format!("{which} bold font bytes contained no usable font face"))?
        .0;
    if bold_family_id != family_id {
        return Err(format!(
            "{which} bold font registered under a different family ({bold_family_id:?}) than its regular ({family_id:?}) — they must be regular/bold variants of the same font family for weight matching to find the bold face"
        ));
    }
    Ok(family_id)
}

/// `twcore`'s three generic families, in parley's vocabulary. Total and lossless —
/// `twcore::FontFamily` has exactly these three variants and does not parse arbitrary
/// (`font-[...]`) families at all, so there is no fallback case to get wrong here.
fn generic_family(family: twcore::FontFamily) -> GenericFamily {
    match family {
        twcore::FontFamily::Sans => GenericFamily::SansSerif,
        twcore::FontFamily::Serif => GenericFamily::Serif,
        twcore::FontFamily::Mono => GenericFamily::Monospace,
    }
}

/// Every `TextStyle` field parley can act on, pushed in one place so the measure pass
/// and `shape_isolated` cannot drift apart.
///
/// This is also the definition `ShapeKey` is checked against: if a shaping result is
/// to be reused for equal `(content, TextStyle, wrap_width, aligned)`, then *nothing
/// outside those four* may reach the shaper, and this being the single site that
/// pushes anything is what makes that checkable by reading one function instead of
/// auditing every call site.
///
/// The tail (`letter_spacing`/`italic`/`tabular_nums`) had all silently gone unpushed
/// for the same reason line height had — tracked in `TouchedText`, inherited
/// correctly, and then read by nothing. See ../PLAN.md §9.7.
fn push_text_style(builder: &mut parley::RangedBuilder<'_, ()>, style: &TextStyle) {
    builder.push_default(font_family_property(style.family));
    builder.push_default(line_height_property(style));
    builder.push_default(StyleProperty::FontSize(style.size));
    builder.push_default(StyleProperty::FontWeight(parley::fontique::FontWeight::new(
        style.weight as f32,
    )));

    // twcore stores tracking in em (Tailwind's own `tracking-*` unit); parley wants
    // the same units as the font size, so this is a multiply, not a cast. Getting it
    // wrong would be near-invisible at `tracking-tight` (-0.025em -> -0.4px at 16px)
    // and obvious only at large sizes, which is a bad way to find out.
    builder.push_default(StyleProperty::LetterSpacing(style.letter_spacing * style.size));

    // Pushed even though no italic face is registered today, so that registering one
    // later is sufficient on its own — exactly how `font-bold` behaved before §9.2
    // item 1 added the second face. Until then fontique's nearest-match picks the
    // upright face, which is the same thing it did before this line existed.
    builder.push_default(StyleProperty::FontStyle(if style.italic {
        parley::style::FontStyle::Italic
    } else {
        parley::style::FontStyle::Normal
    }));

    // `tabular-nums` is a real OpenType feature, not a font choice — plan1.md §8's own
    // counter app asks for it (`text-7xl font-bold text-white tabular-nums`) precisely
    // so a ticking number doesn't shift its own width every increment.
    if style.tabular_nums {
        builder.push_default(StyleProperty::FontFeatures(parley::style::FontFeatures::Source(
            std::borrow::Cow::Borrowed("tnum"),
        )));
    }
}

/// `twcore`'s text alignment, in parley's vocabulary.
fn parley_alignment(align: twcore::TextAlignment) -> parley::Alignment {
    match align {
        twcore::TextAlignment::Start => parley::Alignment::Start,
        twcore::TextAlignment::Center => parley::Alignment::Center,
        twcore::TextAlignment::End => parley::Alignment::End,
        twcore::TextAlignment::Justify => parley::Alignment::Justify,
    }
}

/// `twcore`'s line height, in parley's vocabulary.
///
/// `Relative` is a multiple of the font size — which is exactly CSS's unitless
/// `line-height`, and therefore `FontSizeRelative`, **not** `MetricsRelative`.
/// Getting that wrong is silent: `MetricsRelative` is parley's default, so a text box
/// simply comes out the font's natural line height instead of the one the class
/// string asked for, and nothing anywhere reports a problem.
///
/// This was the bug. `line_height` parsed, inherited through `resolve_inherited`, and
/// reached `TextStyle::line_height` correctly — and then, exactly like family, no
/// shaping site read it. Every native text node was sized by DejaVu's own metrics
/// while the WebView sized the same node by Tailwind's `text-base`/`leading-*`
/// values, making every native text box a few pixels shorter than its web
/// counterpart. See ../PLAN.md §9.7.
fn line_height_property<'a>(style: &TextStyle) -> StyleProperty<'a, ()> {
    StyleProperty::LineHeight(match style.line_height {
        twcore::LineHeight::Relative(m) => parley::style::LineHeight::FontSizeRelative(m),
        twcore::LineHeight::Absolute(px) => parley::style::LineHeight::Absolute(px),
    })
}

/// One real `parley` shaping pass: the measured size and the glyph runs, for a given
/// content + style + wrap width. The only place in this crate that shapes text for
/// *layout* (`shape_isolated` shapes for caret/hit-test geometry, which needs a live
/// `parley::Layout` this summary deliberately doesn't keep).
///
/// A free function rather than a method because `layout()` calls it from inside
/// taffy's measure closure, which already holds `&mut self.tree` — so it takes the
/// two contexts it needs directly instead of `&mut self`.
fn shape_for_layout(
    layout_cx: &mut LayoutContext<()>,
    font_cx: &mut FontContext,
    content: &str,
    style: &TextStyle,
    wrap_width: Option<f32>,
    aligned: bool,
    options: TextOptions,
) -> ShapedText {
    use unicode_segmentation::UnicodeSegmentation;
    let single_line = content.replace(['\n', '\r'], " ");
    let content = if options.wrap { content } else { single_line.as_str() };
    let mut make_layout = |text: &str| {
        let mut builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
        push_text_style(&mut builder, style);
        for (start, cluster) in text.grapheme_indices(true) {
            if !cluster.contains('\u{fe0e}') && cluster.chars().any(|c|
                matches!(c as u32, 0x1f000..=0x1faff | 0x2600..=0x27bf | 0xfe0f | 0x20e3)) {
                builder.push(StyleProperty::FontFamily(ParleyFontFamily::List(std::borrow::Cow::Owned(vec![
                    FontFamilyName::Generic(GenericFamily::Emoji),
                    FontFamilyName::Generic(generic_family(style.family)),
                ]))), start..start + cluster.len());
            }
        }
        builder.push_default(StyleProperty::OverflowWrap(parley::OverflowWrap::Anywhere));
        let mut layout: parley::Layout<()> = builder.build(text);
        layout.break_all_lines(if options.wrap { wrap_width } else { None });
        layout
    };
    let mut plyt = make_layout(content);
    let limit = options.max_lines.unwrap_or(usize::MAX);
    if options.ellipsis && wrap_width.is_some_and(|w| w > 0.0) &&
        (plyt.lines().count() > limit || plyt.width() > wrap_width.unwrap() + 0.1) {
        // Shape candidates at grapheme boundaries: never split accents or ZWJ emoji.
        let bounds: Vec<usize> = content.grapheme_indices(true).map(|(i, _)| i)
            .chain(std::iter::once(content.len())).collect();
        let (mut low, mut high) = (0, bounds.len());
        let mut best = make_layout("…");
        while low < high {
            let mid = (low + high) / 2;
            let candidate = format!("{}…", content[..bounds[mid]].trim_end());
            let layout = make_layout(&candidate);
            if layout.lines().count() <= limit && layout.width() <= wrap_width.unwrap() + 0.1 {
                best = layout;
                low = mid + 1;
            } else { high = mid; }
        }
        plyt = best;
    }
    // Align only when the box width is externally fixed. parley aligns against the
    // `max_advance` given to `break_all_lines`, which equals this node's own final
    // width *only* in that case — when the box is content-sized instead, `wrap_width`
    // is the space available rather than the box, and aligning against it would push
    // glyphs outside the node's own rect. A content-sized single line has nothing to
    // align within anyway; content-sized *wrapped* text is the one case this leaves
    // unaligned (../PLAN.md §9.7).
    if aligned {
        plyt.align(parley_alignment(style.align), parley::AlignmentOptions::default());
    }

    let mut runs: Vec<GlyphRun> = Vec::new();
    let mut height = 0.0f32;
    let mut width = 0.0f32;
    for line in plyt.lines().take(limit) {
        height = height.max(line.metrics().block_max_coord);
        width = width.max(line.metrics().advance);
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else { continue };
            let run = glyph_run.run();
            let glyphs = glyph_run
                .positioned_glyphs()
                .map(|g| PositionedGlyph { id: g.id, x: g.x, y: g.y })
                .collect();
            runs.push(GlyphRun { font: run.font().clone(), font_size: run.font_size(), glyphs });
        }
    }

    ShapedText { width, height, runs: Arc::new(runs) }
}

/// Parses one sprite's path data into `kurbo::BezPath`s, once — see `ResolvedIcon`.
///
/// A `d` string that fails to parse is **dropped rather than propagated**: the table it
/// comes from is generated by `scripts/gen_icons.py` from the WebView backend's own
/// sprite sheet, so a parse failure means the generator and `kurbo` disagree about SVG,
/// which is a build-time problem for `icons_parse_and_stay_inside_their_viewbox` to
/// catch (it asserts every bundled shape parses to a non-empty path) rather than
/// something to make a running app panic over.
fn resolve_sprite(sprite: &'static IconSprite) -> ResolvedIcon {
    let shapes = sprite
        .shapes
        .iter()
        .filter_map(|s| {
            kurbo::BezPath::from_svg(s.d)
                .ok()
                .map(|path| IconShapeGeometry { path, fill: s.fill, stroke: s.stroke })
        })
        .collect();
    ResolvedIcon { extent: sprite.extent, shapes }
}

/// The parley style property that pins shaping to one generic family.
///
/// Without this, parley falls back to its own default (`FontFamily::Source("sans-serif")`)
/// on every shaping pass, which is why `font-serif`/`font-mono` did nothing at all
/// before — the class parsed, inherited, and reached `TextStyle::family` correctly, and
/// then no shaping site ever read it. See ../PLAN.md §9.4.
fn font_family_property<'a>(family: twcore::FontFamily) -> StyleProperty<'a, ()> {
    StyleProperty::FontFamily(ParleyFontFamily::Single(FontFamilyName::Generic(generic_family(
        family,
    ))))
}

/// Android owns the emoji font and updates its supported Unicode sequences.
#[cfg(target_os = "android")]
fn register_android_emoji(font_cx: &mut FontContext) {
    let mut ids = Vec::new();
    for path in ["/system/fonts/NotoColorEmoji.ttf", "/system/fonts/NotoColorEmojiFlags.ttf",
                 "/system/fonts/NotoColorEmojiLegacy.ttf"] {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let families = font_cx.collection.register_fonts(
            parley::fontique::Blob::new(Arc::new(bytes)), None);
        ids.extend(families.iter().map(|(id, _)| *id));
    }
    font_cx.collection.set_generic_families(GenericFamily::Emoji, ids.into_iter());
}

impl Renderer {
    /// Registers one to three typeface families and makes them selectable by
    /// `font-sans` / `font-serif` / `font-mono`.
    ///
    /// Once a family's two weights are registered, `layout()`'s existing
    /// `StyleProperty::FontWeight(weight)` push is sufficient on its own to select
    /// the bold face for `font-bold`/`font-semibold`/etc. — fontique's own
    /// nearest-weight matching does the picking, no extra logic needed here (verified
    /// experimentally before writing this: shaping the same string at weight 400 vs.
    /// 700 with both faces registered selects two different, differently-advancing
    /// font blobs). Without a second face, every weight silently rendered as
    /// whichever one face was registered — see ../PLAN.md §9.2 item 1.
    ///
    /// An absent `serif`/`mono` is pointed at the sans family rather than left unset,
    /// so `font-serif` on a one-font backend renders in the one font it has instead of
    /// falling through to whatever fontique's own last-resort matching finds — which,
    /// on a device with no system fonts registered in this collection, is nothing.
    pub fn new(fonts: FontSet) -> Result<Self, String> {
        let mut font_cx = FontContext::new();

        let sans = register_family(&mut font_cx, &fonts.sans, "sans")?;
        let serif = match &fonts.serif {
            Some(faces) => register_family(&mut font_cx, faces, "serif")?,
            None => sans,
        };
        let mono = match &fonts.mono {
            Some(faces) => register_family(&mut font_cx, faces, "mono")?,
            None => sans,
        };

        for (generic, id) in [
            (GenericFamily::SansSerif, sans),
            (GenericFamily::Serif, serif),
            (GenericFamily::Monospace, mono),
        ] {
            font_cx.collection.set_generic_families(generic, std::iter::once(id));
        }

        #[cfg(target_os = "android")]
        register_android_emoji(&mut font_cx);

        Ok(Renderer {
            tree: TaffyTree::new(),
            resolver: twcore::Resolver::new(),
            font_cx,
            layout_cx: LayoutContext::new(),
            shape_cache_cur: FxHashMap::default(),
            shape_cache_prev: FxHashMap::default(),
            shape_cache_hits: 0,
            shape_cache_misses: 0,
            shaped: FxHashMap::default(),
            container_styles: FxHashMap::default(),
            icons: FxHashMap::default(),
            images: FxHashMap::default(),
            icon_geometry: FxHashMap::default(),
            inherited: vec![TextStyle::default()],
        })
    }

    /// Shaped-text cache counters — see `ShapeCacheStats`.
    pub fn shape_cache_stats(&self) -> ShapeCacheStats {
        ShapeCacheStats {
            hits: self.shape_cache_hits,
            misses: self.shape_cache_misses,
            entries: self.shape_cache_cur.len() + self.shape_cache_prev.len(),
        }
    }

    /// Classes `twcore` didn't recognise for the given class string — surfaced, not
    /// swallowed, matching `twcore`'s own convention (see phase_0's README). A
    /// library shouldn't panic on a string it doesn't control the origin of; callers
    /// that do control it (every test harness so far) can assert this is empty.
    pub fn unknown_classes(&mut self, classes: &str) -> Vec<String> {
        self.resolver.unknown(classes)
    }

    /// Resets the node tree for a fresh render pass — an interactive consumer (§4's
    /// GPU harness) calls this before rebuilding the tree on every re-render, instead
    /// of either (a) leaking one stale generation of nodes into the `taffy` tree per
    /// render forever, or (b) building a whole new `Renderer` per frame, which would
    /// re-register the bundled font and cold-start `twcore`'s resolver cache every
    /// time. `font_cx`/`layout_cx`/`resolver`/the shaped-text cache — the
    /// expensive-to-rebuild state — are deliberately left warm; only the tree itself
    /// and its per-`NodeId` derived maps reset.
    ///
    /// The shaped-text cache specifically *must* survive this, and that is the whole
    /// reason it is keyed by content+style rather than by `NodeId`: every render
    /// rebuilds the tree from scratch, so every node's id is new, and a cache keyed on
    /// ids would miss on every frame by construction.
    pub fn clear(&mut self) {
        self.tree.clear();
        self.shaped.clear();
        self.container_styles.clear();
        // `icons` is per-render (keyed by `NodeId`); `icon_geometry` is warm state
        // keyed by sprite name and deliberately survives, for the same reason the
        // shape cache does.
        self.icons.clear();
        self.images.clear();
        self.inherited.clear();
        self.inherited.push(TextStyle::default());
    }

    /// Resolves `classes` the way `Resolver::resolve` always has (no ancestry
    /// awareness of its own), then fills in any `TextStyle` field this class string's
    /// own declarations didn't touch from the current top of `inherited` — CSS-style
    /// inheritance, implemented here rather than in `twcore` because `ComputedStyle`
    /// is a resolved, ancestry-free snapshot by design (see `phase_0/spike3_twcore`'s
    /// doc comment), and only text properties inherit in CSS at all (layout/paint
    /// never do — a `Column`'s `bg-slate-950` must never leak onto its children).
    /// `Compiled.decls` is walked a second time (cheap: `compile()` is cache-hit after
    /// `resolve()` just primed it) rather than changing `Decl::apply`'s signature,
    /// which would ripple every declaration variant for a change local to this crate.
    fn resolve_inherited(&mut self, classes: &str) -> Arc<ComputedStyle> {
        let parent_text = self.inherited.last().cloned().unwrap_or_default();
        let style = self.resolver.resolve(classes, NodeState::default());
        let compiled = self.resolver.compile(classes);
        let theme = self.resolver.theme();
        let mut touched = TouchedText::default();
        for (variants, decl) in &compiled.decls {
            if !variants.matches(&NodeState::default(), theme) {
                continue;
            }
            match decl {
                Decl::TextColor(_) => touched.color = true,
                Decl::FontSize(..) => {
                    touched.size = true;
                    touched.line_height = true;
                }
                Decl::FontWeight(_) => touched.weight = true,
                Decl::Italic(_) => touched.italic = true,
                Decl::Leading(_) => touched.line_height = true,
                Decl::Tracking(_) => touched.letter_spacing = true,
                Decl::TextAlign(_) => touched.align = true,
                Decl::Underline(_) | Decl::LineThrough(_) => touched.decoration = true,
                Decl::FontFamily(_) => touched.family = true,
                Decl::TabularNums(_) => touched.tabular_nums = true,
                _ => {}
            }
        }
        if touched.all() {
            return style;
        }
        let mut merged = (*style).clone();
        if !touched.color {
            merged.text.color = parent_text.color;
        }
        if !touched.size {
            merged.text.size = parent_text.size;
        }
        if !touched.line_height {
            merged.text.line_height = parent_text.line_height;
        }
        if !touched.weight {
            merged.text.weight = parent_text.weight;
        }
        if !touched.italic {
            merged.text.italic = parent_text.italic;
        }
        if !touched.letter_spacing {
            merged.text.letter_spacing = parent_text.letter_spacing;
        }
        if !touched.align {
            merged.text.align = parent_text.align;
        }
        if !touched.decoration {
            merged.text.decoration = parent_text.decoration;
        }
        if !touched.family {
            merged.text.family = parent_text.family;
        }
        if !touched.tabular_nums {
            merged.text.tabular_nums = parent_text.tabular_nums;
        }
        Arc::new(merged)
    }

    /// A leaf that lays out and paints a single run of shaped text. `content` becomes
    /// this node's whole text — mirroring how any bare string child of *any* HTML
    /// element becomes a text node (not just inside `<span>`/`Text`), every plain
    /// string child in `zkite_py`'s widget tree should eventually become one of these,
    /// not just `Text` widgets specifically. See ../PLAN.md §3. `classes` may be empty
    /// (the common case for a bare string child) — inheritance via
    /// `resolve_inherited` fills in typography from the nearest ancestor either way.
    pub fn text(&mut self, classes: &str, content: impl Into<Arc<str>>) -> NodeId {
        self.text_with_options(classes, content, TextOptions::default())
    }

    pub fn text_with_options(&mut self, classes: &str, content: impl Into<Arc<str>>, options: TextOptions) -> NodeId {
        let style = self.resolve_inherited(classes);
        let ctx = TextContext { content: content.into(), style: style.clone(), options };
        self.tree.new_leaf_with_context(style.layout.clone(), ctx).unwrap()
    }

    /// A leaf that paints one bundled sprite (`zkite_py`'s `Icon`).
    ///
    /// **Defaults to `1em` on each axis the class string leaves `auto`**, which is what
    /// makes this backend agree with the WebView one. `Icon.implied_classes` is
    /// `w-[1em] h-[1em]` (`phase_1/zkite_py/src/zkite/widgets.py`), i.e. an icon is one
    /// line tall and scales with the text around it. `em` resolves against the
    /// *inherited* font size, which `twcore` deliberately cannot see — every
    /// `ComputedStyle` is ancestry-free by design — so `_tree_classes` used to
    /// substitute a hardcoded `w-[20px] h-[20px]` and documented it as an approximation
    /// that "native cannot do".
    ///
    /// This is the one place in the pipeline that *can*: `resolve_inherited` has just
    /// walked the inheritance stack, so `style.text.size` is the real, resolved font
    /// size for this node. Substituting it here is a correct `1em` rather than an
    /// approximation of one, and it is why an `Icon` inside `text-[32px]` is now 32px on
    /// both backends instead of 32 on one and 20 on the other. An explicit `w-8 h-8` on
    /// the widget still wins, because those resolve to a definite `Dimension` and only
    /// `auto` is filled in.
    ///
    /// An unknown `name` is not an error. It records no geometry and the node paints as
    /// the empty box it always did, so a typo degrades to the previous behaviour rather
    /// than panicking on a string that came from user code — the same call
    /// `unknown_classes` makes for a class `twcore` doesn't recognise.
    pub fn icon(&mut self, classes: &str, name: &str) -> NodeId {
        let style = self.resolve_inherited(classes);
        let mut layout = style.layout.clone();
        let em = style.text.size;
        if layout.size.width.is_auto() {
            layout.size.width = Dimension::length(em);
        }
        if layout.size.height.is_auto() {
            layout.size.height = Dimension::length(em);
        }
        let id = self.tree.new_leaf(layout).unwrap();
        self.container_styles.insert(id, style);
        if let Some(sprite) = icons::sprite(name) {
            let geometry = self
                .icon_geometry
                .entry(sprite.name)
                .or_insert_with(|| Arc::new(resolve_sprite(sprite)))
                .clone();
            self.icons.insert(id, geometry);
        }
        id
    }

    /// A leaf that paints one bitmap (`zkite_py`'s `Image`).
    ///
    /// **This crate does not decode anything, on purpose.** It carries the `src`
    /// through to the paint layer and stops there — the same split the module doc
    /// draws between layout (shared, backend-agnostic, no vello dependency) and paint
    /// (legitimately per-backend). Decoding here would put a PNG decoder in the
    /// dependency tree of a crate whose entire job is "class strings to positioned
    /// boxes", and would put the answer to "what pixels" in the layer that has no way
    /// to hand them to a GPU atlas.
    ///
    /// **The consequence worth stating: layout never depends on the bitmap.** An
    /// `Image`'s box comes from its `w-`/`h-`/flex classes exactly as any other node's
    /// does, so a missing file, a corrupt file and a 4000x3000 photo all lay out
    /// identically, and a decode that finishes on a later frame cannot reflow the
    /// page. That is a deliberate difference from a browser, where an `<img>` with no
    /// dimensions is sized by its intrinsic size once loaded. ZKite has no intrinsic
    /// sizing for images and this is where that decision lives; `../PLAN.md` §35.6
    /// records it as a known gap rather than an accident.
    ///
    /// An empty `src` records nothing, so `Image()` with no source paints as the empty
    /// box it always did — the same degrade-to-previous-behaviour call `icon` makes
    /// for a sprite name that matches nothing.
    pub fn image(&mut self, classes: &str, src: &str) -> NodeId {
        let style = self.resolve_inherited(classes);
        let id = self.tree.new_leaf(style.layout.clone()).unwrap();
        self.container_styles.insert(id, style);
        if !src.is_empty() {
            self.images.insert(id, Arc::from(src));
        }
        id
    }

    /// A node with children and no text of its own (`Column`, `Row`, `Button`'s own
    /// box, ...). Its size comes from layout, never from measurement. `build` is
    /// called with this container's own resolved typography already pushed onto the
    /// inheritance stack (popped again before returning), so every `text()`/nested
    /// `container()` call made inside it inherits correctly — this is why `container`
    /// takes a closure rather than a pre-built `&[NodeId]`: children must be built
    /// *after* this node's own style is resolved and pushed, not before.
    pub fn container(&mut self, classes: &str, build: impl FnOnce(&mut Self) -> Vec<NodeId>) -> NodeId {
        let style = self.resolve_inherited(classes);
        self.inherited.push(style.text.clone());
        let children = build(self);
        self.inherited.pop();
        let id = self.tree.new_with_children(style.layout.clone(), &children).unwrap();
        // Containers carry no taffy `NodeContext` (that's reserved for text leaves —
        // see the struct doc), so the resolved style — needed later for paint, e.g. a
        // `Button`'s own background/radius — is tracked here instead.
        self.container_styles.insert(id, style);
        id
    }

    /// Computes layout for `root` against a `width` x `height` viewport, shaping text
    /// as needed. Must be called before `positioned()`.
    ///
    /// **Text whose content and style haven't changed is not re-shaped.** Before this
    /// cache existed, every text leaf went through a full `parley` pass on every
    /// frame — measured at 3.2ms of a 3.25ms frame for a 48-row list window on an
    /// x86_64 dev box, i.e. 99% of this crate's per-frame cost, and identical whether
    /// the strings had changed or not. That is what held a 10,000-row list at ~34fps
    /// on real hardware (../PLAN.md §11.5).
    ///
    /// **Two generations, rotated here, rather than an LRU.** On entry the current
    /// generation becomes the previous one and a fresh current starts; a lookup checks
    /// current, then previous (promoting a hit), then shapes. An entry therefore
    /// survives exactly as long as something keeps asking for it, and is dropped one
    /// rotation after it stops — no per-entry timestamps, no eviction sweep, and a
    /// working set bounded by two consecutive passes' worth of text rather than by
    /// scroll distance. That bound is the point: `twcore`'s own caches are unbounded
    /// and grow with how far a list has been scrolled (../PLAN.md §11.7), and a glyph
    /// cache doing the same would be a genuine leak on a 10,000-row list, not a
    /// footnote.
    ///
    /// Rotating per `layout()` call rather than per frame is deliberate and safe even
    /// though `../gpu_harness`'s `redraw` can lay out twice in one frame (the viewport
    /// sync pass, ../PLAN.md §11.3): the second pass hits the first pass's entries out
    /// of `previous` and promotes them, so a steady state of repeated identical passes
    /// hits every time regardless of how many passes a frame runs.
    pub fn layout(&mut self, root: NodeId, width: f32, height: f32) {
        self.shaped.clear();
        // Rotate generations. The map being dropped here is everything that went two
        // full passes without being asked for.
        self.shape_cache_prev = std::mem::take(&mut self.shape_cache_cur);

        let font_cx = &mut self.font_cx;
        let layout_cx = &mut self.layout_cx;
        let shaped = &mut self.shaped;
        let cache_cur = &mut self.shape_cache_cur;
        let cache_prev = &mut self.shape_cache_prev;
        let hits = &mut self.shape_cache_hits;
        let misses = &mut self.shape_cache_misses;

        self.tree
            .compute_layout_with_measure(
                root,
                Size { width: AvailableSpace::Definite(width), height: AvailableSpace::Definite(height) },
                |known_dimensions, available_space, node_id, node_context, _style| {
                    let Some(ctx) = node_context else {
                        return Size::ZERO;
                    };

                    let wrap_width = known_dimensions.width.or(match available_space.width {
                        AvailableSpace::Definite(w) => Some(w),
                        AvailableSpace::MaxContent | AvailableSpace::MinContent => None,
                    });
                    let key = ShapeKey {
                        options: ctx.options,
                        content: ctx.content.clone(),
                        style: ctx.style.text.clone(),
                        wrap_width,
                        aligned: known_dimensions.width.is_some(),
                    };

                    // Taffy calls this several times per node per pass (min-content,
                    // max-content, then the definite size), usually with different
                    // available space — so `cache_cur` earns hits *within* a single
                    // layout as well as across frames.
                    let entry = if let Some(hit) = cache_cur.get(&key) {
                        *hits += 1;
                        hit.clone()
                    } else if let Some(hit) = cache_prev.remove(&key) {
                        *hits += 1;
                        cache_cur.insert(key, hit.clone());
                        hit
                    } else {
                        *misses += 1;
                        let shaped_text = shape_for_layout(
                            layout_cx,
                            font_cx,
                            &key.content,
                            &key.style,
                            key.wrap_width,
                            key.aligned,
                            key.options,
                        );
                        cache_cur.insert(key, shaped_text.clone());
                        shaped_text
                    };

                    shaped.insert(node_id, entry.runs.clone());

                    // `known_dimensions.height` is deliberately not part of the cache
                    // key: it never reaches the shaper, it only overrides what this
                    // measure call reports back, so it is applied here instead.
                    Size {
                        width: known_dimensions.width.unwrap_or(entry.width),
                        height: known_dimensions.height.unwrap_or(entry.height),
                    }
                },
            )
            .unwrap();
    }

    /// Flattens the tree (depth-first, parent before children — i.e. paint order) into
    /// absolute-rect nodes. Must be called after `layout()`. `scroll_offsets` is
    /// (typically) a small map of scrollable container `NodeId` -> how far its
    /// content is currently scrolled on each axis (`(x, y)`, positive = scrolled
    /// down/right); pass `&FxHashMap::default()` for a tree with no scrolling at all
    /// — the common case, and the previous unconditional behavior of this method.
    /// Every descendant of a scrolled container is shifted by that offset (and, if
    /// the container's own `overflow` clips, gains a `clip` rect); nodes with no
    /// scrolling ancestor are completely unaffected either way.
    pub fn positioned(&self, root: NodeId, scroll_offsets: &FxHashMap<NodeId, (f32, f32)>) -> Vec<PositionedNode> {
        let mut out = Vec::new();
        self.walk(root, (0.0, 0.0), None, scroll_offsets, &mut out);
        out
    }

    fn walk(
        &self,
        node: NodeId,
        origin: (f32, f32),
        clip: Option<(f32, f32, f32, f32)>,
        scroll_offsets: &FxHashMap<NodeId, (f32, f32)>,
        out: &mut Vec<PositionedNode>,
    ) {
        let l = self.tree.layout(node).unwrap();

        let style = self
            .container_styles
            .get(&node)
            .cloned()
            .or_else(|| self.tree.get_node_context(node).map(|c| c.style.clone()))
            .expect("every node has either a container style or a text context");

        // **A transform is applied here, to the node's resolved position, and not at
        // paint time** (../PLAN.md §45.3). Two things follow from that and both are
        // the reason for the choice.
        //
        // It moves the node *and its whole subtree*, because the shifted `x`/`y` is
        // what every child's `origin` is derived from below — which is what CSS
        // `translate` does, and what a paint-time rect offset would not do.
        //
        // And `hit_test` resolves against `positioned()`'s output, so a translated
        // node is *tapped* where it is *drawn*, for free. Offsetting in the painter
        // instead would leave touch targets at the untransformed position — the
        // §13/§9.7 failure shape exactly: invisible in a screenshot, only
        // reproducible under a finger.
        //
        // Layout is untouched: `l.location` is taffy's answer and nothing here feeds
        // back into it, so a translated node overlaps its siblings rather than
        // displacing them. `scroll_extent` reads each child's *unshifted* layout for
        // the same reason and is therefore unaffected — a divergence from CSS, which
        // does grow the scrollable overflow area for a transformed child. Filed in
        // §45's known gaps rather than fixed here; nothing in `zkite_py` can produce
        // that shape today.
        //
        // `rotate`/`scale` are parsed into the same `Transform` and are still
        // dropped — they need a real affine on an axis-aligned rect list and a
        // hit-test that can invert it, which is a different piece of work.
        let x = origin.0 + l.location.x + style.transform.translate_x;
        let y = origin.1 + l.location.y + style.transform.translate_y;
        let rect = (x, y, l.size.width, l.size.height);

        let text = self.shaped_clone(node);

        // `overflow: scroll/auto/hidden` on *either* axis clips this node's content
        // to its own box — `visible`/`clip` (taffy's fourth variant, rarely used and
        // not exposed by any `zkite_py` widget today) don't. Scroll*ability* (whether
        // a drag gesture should move this node's content) is a separate question the
        // caller decides — see `../gpu_harness/src/render_thread.rs`'s touch
        // handling and `Overflow::Scroll` specifically — clipping applies regardless
        // of whether the content is *currently* scrolled at all, since unscrolled
        // overflow still has to be hidden, not just future-scrolled overflow.
        use taffy::style::Overflow;
        let clips = matches!(style.layout.overflow.x, Overflow::Scroll | Overflow::Hidden)
            || matches!(style.layout.overflow.y, Overflow::Scroll | Overflow::Hidden);
        let own_clip = if clips { Some(intersect_rects(clip, rect)) } else { clip };

        let icon = self.icons.get(&node).cloned();
        let image = self.images.get(&node).cloned();

        // Backfilled once the subtree is complete — the length cannot be known before
        // the children have been walked, and the index is stable because nothing ever
        // removes from `out`.
        let index = out.len();
        out.push(PositionedNode { node, rect, style, text, icon, image, clip, subtree_len: 1 });

        let (off_x, off_y) = scroll_offsets.get(&node).copied().unwrap_or((0.0, 0.0));
        if let Ok(children) = self.tree.children(node) {
            for child in children {
                self.walk(child, (x - off_x, y - off_y), own_clip, scroll_offsets, out);
            }
        }
        out[index].subtree_len = out.len() - index;
    }

    /// Each `PositionedNode` owns its glyph runs outright, even though the cache and
    /// `shaped` share one `Arc` behind the scenes — consumers paint from (and are free
    /// to mutate) their own copy, exactly as before the cache existed.
    fn shaped_clone(&self, node: NodeId) -> Option<Vec<GlyphRun>> {
        self.shaped.get(&node).map(|runs| runs.as_ref().clone())
    }

    /// Finds the deepest (most specific) node whose rect contains `(x, y)`, if any —
    /// for turning a raw touch coordinate into "which node was tapped." Callers own
    /// the mapping from `NodeId` to an actual action (this crate has no concept of
    /// event handlers — see the module doc's paint/layout split, same reasoning).
    /// `scroll_offsets` must match whatever was last passed to `positioned`/`layout`
    /// so a tap resolves against what's actually on screen right now, not an
    /// unscrolled layout — see `positioned`'s doc comment.
    pub fn hit_test(&self, root: NodeId, x: f32, y: f32, scroll_offsets: &FxHashMap<NodeId, (f32, f32)>) -> Option<NodeId> {
        let positioned = self.positioned(root, scroll_offsets);
        positioned
            .iter()
            .rev()
            .find(|n| {
                let (nx, ny, nw, nh) = n.rect;
                let inside_clip = n.clip.is_none_or(|(cx, cy, cw, ch)| {
                    x >= cx && x < cx + cw && y >= cy && y < cy + ch
                });
                inside_clip && x >= nx && x < nx + nw && y >= ny && y < ny + nh
            })
            .map(|n| n.node)
    }

    /// The resolved style for any node, container or text leaf — for a caller (the
    /// touch/gesture layer) that needs to inspect e.g. `overflow` while walking
    /// ancestors, without re-deriving `walk`'s own container-vs-text-context merge
    /// logic. `None` only for a `NodeId` that isn't actually in this tree.
    pub fn style(&self, node: NodeId) -> Option<Arc<ComputedStyle>> {
        self.container_styles
            .get(&node)
            .cloned()
            .or_else(|| self.tree.get_node_context(node).map(|c| c.style.clone()))
    }

    /// How far `node`'s content can scroll along `axis` before running out (content
    /// extent minus this node's own viewport extent, floored at zero) — computed
    /// from each direct child's *unshifted* layout (parent-relative, ignoring any
    /// scroll offset already applied elsewhere), since taffy already lays out an
    /// `overflow: scroll` container's children at their full natural size regardless
    /// of the container's own, possibly smaller, box — that's what makes "how far can
    /// this scroll" a well-defined question in the first place, *provided* `node`
    /// itself was actually allowed to be smaller than its content: a `flex: 1` node
    /// (as any real scroll region needs to be, to fill available space) defaults to
    /// `min-height: auto` in real flexbox everywhere, this renderer included, which
    /// silently forces it to *at least* content size unless something overrides it —
    /// this returns `0` in exactly that case, correctly reflecting that there's
    /// nothing to scroll *because nothing is currently clipping*, not a bug in this
    /// method. See `phase_1/zkite_py/src/zkite/widgets.py`'s `ScrollView` for the real
    /// fix (`min-h-0`) and the real-device bug that found it.
    ///
    /// **The x axis has the exact same trap, one word different** (`min-w-0`, and a
    /// `flex-row` child's default `min-width: auto`), which is why `ScrollView`'s
    /// horizontal form bakes in `min-w-0` the same way the vertical one bakes in
    /// `min-h-0`. See ../PLAN.md §27.
    /// **The end padding is part of the scrollable area**, and leaving it out was a
    /// real (small) divergence from every browser — found on a device in §27 by
    /// flinging a `px-2` strip to its end and measuring the last card against the
    /// screen edge: it sat flush at x=1079 of 1080, i.e. the strip's own right
    /// padding had been scrolled away. CSS puts the scrollable overflow area at the
    /// content edges *plus* the end padding, and the scrollport is the padding box,
    /// so the arithmetic below is `content_end + padding_end − (size − border_end)`.
    ///
    /// This was never x-specific — `max_scroll_y` had the identical shortfall from
    /// §6.1 onwards, so a `p-2` vertical `ScrollView` lost its bottom padding at the
    /// end of every scroll. Nothing caught it because the conformance harness only
    /// ever renders offset 0, and a missing 8dp at a scroll bound is invisible unless
    /// you go looking at the bound.
    pub fn max_scroll(&self, node: NodeId, axis: ScrollAxis) -> f32 {
        let Ok(l) = self.tree.layout(node) else { return 0.0 };
        let (viewport, pad_end, border_end) = match axis {
            ScrollAxis::X => (l.size.width, l.padding.right, l.border.right),
            ScrollAxis::Y => (l.size.height, l.padding.bottom, l.border.bottom),
        };
        let mut content_end: f32 = 0.0;
        if let Ok(children) = self.tree.children(node) {
            for child in children {
                if let Ok(l) = self.tree.layout(child) {
                    let end = match axis {
                        ScrollAxis::X => l.location.x + l.size.width,
                        ScrollAxis::Y => l.location.y + l.size.height,
                    };
                    content_end = content_end.max(end);
                }
            }
        }
        (content_end + pad_end - (viewport - border_end)).max(0.0)
    }

    /// `max_scroll(node, ScrollAxis::Y)`. Kept as a named method because it is what
    /// most of this crate's callers and tests actually mean.
    pub fn max_scroll_y(&self, node: NodeId) -> f32 {
        self.max_scroll(node, ScrollAxis::Y)
    }

    /// Whether `node` *declares* itself scrollable along `axis` (`overflow-{axis}-scroll`/
    /// `-auto` in Tailwind terms — `twcore` maps both to `taffy::style::Overflow::Scroll`,
    /// confirmed against `phase_0/spike3_twcore/src/parse.rs`'s `overflow` handling).
    ///
    /// **Declaring is not the same as having room, and for the x axis the difference
    /// is the whole ballgame.** `overflow-auto` parses to `Axis::Both`
    /// (`parse.rs:243`), and `ScrollView`/`List` both bake `overflow-auto` into their
    /// implied classes — so *every scroll region that has ever existed in this
    /// framework already answers `true` here for `ScrollAxis::X`*, while having
    /// exactly zero horizontal room. A gesture layer that picked its scroll target
    /// off this method alone would resolve a horizontal drag to a vertical list and
    /// then move nothing. Use `scrollable_ancestor`, which filters on room.
    pub fn is_scrollable(&self, node: NodeId, axis: ScrollAxis) -> bool {
        self.style(node).is_some_and(|s| {
            let overflow = match axis {
                ScrollAxis::X => s.layout.overflow.x,
                ScrollAxis::Y => s.layout.overflow.y,
            };
            matches!(overflow, taffy::style::Overflow::Scroll)
        })
    }

    /// `is_scrollable(node, ScrollAxis::Y)`.
    pub fn is_scrollable_y(&self, node: NodeId) -> bool {
        self.is_scrollable(node, ScrollAxis::Y)
    }

    /// The nearest node at or above `from` that would actually move if dragged along
    /// `axis`, with how far it can go — the one question a gesture layer resolving a
    /// touch-down actually has. Walks up via `parent` exactly as the tap
    /// target-vs-currentTarget walk does.
    ///
    /// **The filter is room, not declaration**, for the reason `is_scrollable`
    /// documents: `overflow-auto` declares both axes, so declaration alone would make
    /// every vertical `ScrollView` claim every horizontal drag and swallow it. A node
    /// with no room is *skipped*, and the walk continues to its ancestors — so a
    /// horizontal strip that has hit its end hands the drag to whatever is above it,
    /// which is what a browser's scroll chaining does.
    ///
    /// This lives here rather than in the render thread because it is pure tree
    /// arithmetic and `gpu_harness` cannot be host-tested at all (its `build.rs`
    /// compiles gametextinput against NDK headers) — the same split ../PLAN.md §26.3
    /// made for `slider_value_at`.
    ///
    /// **Not modelled: chaining *mid-gesture*.** The answer is resolved once at
    /// touch-down and held for the gesture's lifetime, so a drag that scrolls its
    /// target to a bound stops there rather than continuing on the ancestor.
    pub fn scrollable_ancestor(&self, from: NodeId, axis: ScrollAxis) -> Option<(NodeId, f32)> {
        let mut node = Some(from);
        while let Some(n) = node {
            if self.is_scrollable(n, axis) {
                let max = self.max_scroll(n, axis);
                if max > 0.0 {
                    return Some((n, max));
                }
            }
            node = self.parent(n);
        }
        None
    }

    /// A node's parent, if any — lets a caller walk *up* from `hit_test`'s result.
    /// Needed because `hit_test` deliberately returns the deepest match: a tap lands
    /// on whatever's visually topmost at that point (e.g. a button's own text label,
    /// which is a child of the button, not the button itself), but "was the button
    /// tapped" has to mean "did the tap land on the button *or any of its
    /// descendants*" — the same target-vs-currentTarget distinction DOM event
    /// bubbling makes, reimplemented here because there's no DOM to inherit it from.
    /// Also how `TouchDown` finds a scrollable ancestor above whatever's directly
    /// under a touch-down point — see `../gpu_harness/src/render_thread.rs`.
    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        self.tree.parent(node)
    }

    /// Shapes `content` in isolation — same font size/weight a `text()` leaf with
    /// this `style` would use, but without going through the taffy tree at all —
    /// and returns the resulting `parley::Layout`. Shared by
    /// `hit_test_text_offset`/`text_caret_geometry` (../../gpu_harness/PLAN.md §7.7,
    /// tap-to-position-cursor): both need a real `parley::Layout` to hand to
    /// `parley::Cursor`, which the `GlyphRun`/`PositionedGlyph` summary `layout()`
    /// normally keeps (glyph ids + absolute x/y, no cluster/text-offset mapping at
    /// all) can't provide. Re-shaping one field's short string on demand, only when
    /// a tap or a real selection update needs a text-offset answer, is cheap enough
    /// not to need this in `layout()`'s own regular per-render measure pass.
    /// Deliberately *not* served from the shaped-text cache: that cache stores the
    /// `GlyphRun`/size summary, and this method's whole reason to exist is needing the
    /// live `parley::Layout` a summary throws away. It runs at most once or twice per
    /// frame (the focused field's caret, a tap) against one short string, against
    /// ~100 text leaves for the measure pass — so it is not what §11.5 measured and
    /// caching it would buy nothing worth the second mechanism.
    fn shape_isolated(&mut self, style: &TextStyle, content: &str) -> parley::Layout<()> {
        let mut builder = self.layout_cx.ranged_builder(&mut self.font_cx, content, 1.0, true);
        // Same pushes the regular measure pass makes, via the same function — caret
        // geometry and tap hit-testing have to shape exactly what was painted, or a
        // tap lands at the wrong byte offset in any field whose style differs from
        // the default.
        push_text_style(&mut builder, style);
        let mut plyt: parley::Layout<()> = builder.build(content);
        plyt.break_all_lines(None);
        plyt
    }

    /// The byte offset a tap at `(x, y)` — relative to the text node's own
    /// top-left, same convention as `PositionedNode.rect` — would place a cursor
    /// at, per `parley::Cursor::from_point`. For tap-to-position-cursor: a caller
    /// hit-tests a touch against the focused `TextField`'s own text node first
    /// (unrelated to this method — ordinary node/rect matching), then converts the
    /// touch point to this node-local space and calls this to get an actual text
    /// offset, not just "which node."
    pub fn hit_test_text_offset(&mut self, style: &TextStyle, content: &str, x: f32, y: f32) -> usize {
        let layout = self.shape_isolated(style, content);
        Cursor::from_point(&layout, x, y).index()
    }

    /// The caret rect (x, y, w, h — node-local, same convention as
    /// `hit_test_text_offset`'s point) for a real byte offset into `content`,
    /// `caret_width` wide. Replaces an end-of-text-only approximation once a real
    /// selection (from a tap, or from the IME's own `GameTextInputState`) is known.
    pub fn text_caret_geometry(&mut self, style: &TextStyle, content: &str, byte_offset: usize, caret_width: f32) -> (f32, f32, f32, f32) {
        let layout = self.shape_isolated(style, content);
        let cursor = Cursor::from_byte_index(&layout, byte_offset, Affinity::default());
        let bbox = cursor.geometry(&layout, caret_width);
        (bbox.x0 as f32, bbox.y0 as f32, (bbox.x1 - bbox.x0) as f32, (bbox.y1 - bbox.y0) as f32)
    }

    // ------------------------------------------------------------------ §47, Queue F2
    //
    // The animation driver overrides a node's *resolved* style for one frame. It runs
    // between `container`/`icon`/`image` and `layout`, i.e. after the class string has
    // been resolved and before anything reads the result.

    /// Override one animated channel on an already-built node, for this frame only.
    ///
    /// **Why this mutates a resolved style rather than being threaded through
    /// `container`.** Every element node's style is produced inside
    /// `container`/`icon`/`image` from its class string, and there are nineteen call
    /// sites of `zkite_pybridge::build` across this workspace's tests, harnesses and the
    /// conformance renderer. Adding a parameter to the constructors would touch all of
    /// them to express something none of them has an opinion about; overriding
    /// afterwards is one call, made only for nodes that are actually animating.
    ///
    /// **Neither channel affects layout**, which is what makes "afterwards" safe.
    /// `paint.opacity` is read at composite time by [`OpacityGroups`] and
    /// `transform.translate_*` by `walk`, from the same `container_styles` entry this
    /// writes; taffy has already been handed `style.layout` and never sees either.
    /// Called before `layout()` anyway, so a future channel that *did* move a box
    /// would be applied early enough rather than silently one frame late.
    ///
    /// `Arc::make_mut` rather than an unconditional clone: `resolve_inherited` hands
    /// out a fresh `Arc` per node today, so this is usually an in-place write, and it
    /// stays correct by construction if that ever becomes a shared cache entry.
    ///
    /// A `NodeId` this renderer has no style for is ignored rather than panicking —
    /// the caller is matching ids across a tree that Python rebuilt, and a node that
    /// left the tree between the sample and the apply is an ordinary race, not a bug.
    pub fn animate(&mut self, id: NodeId, channel: Channel, value: f32) {
        let Some(style) = self.container_styles.get_mut(&id) else { return };
        let style = Arc::make_mut(style);
        match channel {
            Channel::Opacity => style.paint.opacity = value,
            Channel::TranslateX => style.transform.translate_x = value,
            Channel::TranslateY => style.transform.translate_y = value,
        }
    }

    /// This node's current value on `channel`, as resolved from its class string —
    /// the `from` an animation gets when the author does not state one.
    ///
    /// Returns `None` for a text leaf (which carries no `container_styles` entry) and
    /// for an unknown id; both are "there is nothing to animate here", which the
    /// caller treats the same way.
    pub fn channel_value(&self, id: NodeId, channel: Channel) -> Option<f32> {
        let style = self.container_styles.get(&id)?;
        Some(match channel {
            Channel::Opacity => style.paint.opacity,
            Channel::TranslateX => style.transform.translate_x,
            Channel::TranslateY => style.transform.translate_y,
        })
    }
}

/// One animatable property (../PLAN.md §47, Queue F).
///
/// **Three, and not the five `twcore` parses.** `rotate-*` and `scale-*` compile into
/// the same [`twcore::Transform`](zkite_twcore::style::Transform) and are still painted
/// by nothing (§45.7): they need a real affine over an axis-aligned rect list *and* a
/// `hit_test` that can invert it, where a translate reuses the per-node offset channel
/// `positioned` already had for scroll and IME pan. Animating a channel that does not
/// paint would be a silent divergence of exactly the §15.1 kind — the animation would
/// run, complete, fire its callback, and never move a pixel — so they are absent here
/// rather than accepted and dropped.
///
/// **The two that are here are not equally cheap, and an API has to say so.**
/// Measured on the Moto G54 (§46.6): one `opacity` group spanning the full 1080x2223
/// surface costs **1.88ms/frame**, 16% of the frame's total work, because it opens an
/// offscreen render target. A translate opens nothing at all. That is the whole reason
/// [`opens_a_layer`](Channel::opens_a_layer) exists — see its doc comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Channel {
    Opacity,
    TranslateX,
    TranslateY,
}

impl Channel {
    /// The wire name, as it appears in a node's `anim` attribute. Snake case to match
    /// the `zkite_py` keyword arguments an author actually types.
    pub fn name(self) -> &'static str {
        match self {
            Channel::Opacity => "opacity",
            Channel::TranslateX => "translate_x",
            Channel::TranslateY => "translate_y",
        }
    }

    /// Parse a wire name. Unknown names are rejected loudly by the caller rather than
    /// dropped, unlike an unknown Tailwind class — an author who writes
    /// `rotate=...` has asked for something specific and getting silence is the §15.1
    /// failure this whole channel enum exists to avoid.
    pub fn parse(s: &str) -> Option<Channel> {
        match s {
            "opacity" => Some(Channel::Opacity),
            "translate_x" => Some(Channel::TranslateX),
            "translate_y" => Some(Channel::TranslateY),
            _ => None,
        }
    }

    /// Whether animating this channel forces an offscreen render target for the
    /// node's whole subtree.
    ///
    /// **This is a performance fact with an API consequence, which is why it is on the
    /// type rather than in a comment.** §46.6 measured a full-screen `opacity` group at
    /// +1.88ms/frame against a frame with ~5ms of slack, and the cost scales with the
    /// *layer's area*, not with the animated node. A 200–300ms transition spends ~28ms
    /// spread over ~15 frames and is affordable; a persistent group is not. The
    /// `zkite_py` surface uses this to decide what to warn about.
    pub fn opens_a_layer(self) -> bool {
        matches!(self, Channel::Opacity)
    }

    /// Every channel, for a caller enumerating them (`zkite_py`'s keyword set, a test
    /// that must cover all of them). Deliberately a function rather than a `const`
    /// slice a caller could forget to extend.
    pub fn all() -> [Channel; 3] {
        [Channel::Opacity, Channel::TranslateX, Channel::TranslateY]
    }
}

/// The overlapping region of `a` (if any — `None` means "no clip yet," i.e. the
/// unbounded canvas) and `b`, in the same absolute coordinate space `PositionedNode`
/// rects use. Empty (zero width/height, not negative) when they don't overlap at
/// all, rather than panicking or producing a nonsensical negative-size rect.
fn intersect_rects(a: Option<(f32, f32, f32, f32)>, b: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    let Some(a) = a else { return b };
    let x0 = a.0.max(b.0);
    let y0 = a.1.max(b.1);
    let x1 = (a.0 + a.2).min(b.0 + b.2);
    let y1 = (a.1 + a.3).min(b.1 + b.3);
    (x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// Where a bitmap of intrinsic size `intrinsic` actually lands inside an `Image`
/// leaf's box, under CSS `object-fit` (../PLAN.md §35). Returns a rect **relative to
/// the box's top-left**, which the caller offsets by the node's own absolute
/// position.
///
/// Pure geometry, and here rather than in `gpu_harness` for exactly the reason
/// `slider_value_at` above is: that crate does not build for the host, so nothing in
/// it can carry a test (../PLAN.md §15.5). It also has **four** consumers that must
/// agree — the GPU paint path, the CPU paint path, the conformance harness, and this
/// crate's own tests — which is the same argument `icons.rs` makes for sharing its
/// geometry rather than reimplementing it per backend (../PLAN.md §18.3).
///
/// **The returned rect can be larger than the box, and that is not an error** —
/// `Cover` and `None` both overflow by construction. Every caller must clip to the
/// box; CSS clips a replaced element's content to its content box and a painter that
/// forgets draws a `Cover` image spilling over its neighbours.
///
/// `object-position` is not implemented: this always centres, which is CSS's own
/// `50% 50%` default. A non-positive box or intrinsic size yields a zero-area rect
/// rather than a NaN or an inverted one — all four are reachable from ordinary user
/// code (`Image("w-0")`, a 0x0 PNG) and none should reach a rasteriser.
pub fn object_fit_rect(
    fit: twcore::style::ObjectFit,
    box_size: (f32, f32),
    intrinsic: (f32, f32),
) -> (f32, f32, f32, f32) {
    use twcore::style::ObjectFit;
    let (bw, bh) = box_size;
    let (iw, ih) = intrinsic;
    if !(bw > 0.0) || !(bh > 0.0) || !(iw > 0.0) || !(ih > 0.0) {
        return (0.0, 0.0, 0.0, 0.0);
    }
    // `Fill` is the one mode that does not preserve the aspect ratio, so it never
    // goes through the uniform-scale path below.
    if fit == ObjectFit::Fill {
        return (0.0, 0.0, bw, bh);
    }
    let contain = (bw / iw).min(bh / ih);
    let scale = match fit {
        ObjectFit::Fill => unreachable!("handled above"),
        ObjectFit::Contain => contain,
        ObjectFit::Cover => (bw / iw).max(bh / ih),
        ObjectFit::None => 1.0,
        // `min(none, contain)` — never scales *up*. The `.min(1.0)` is the whole
        // difference from `Contain`, and it is why a small image in a large box is
        // the case that discriminates the two.
        ObjectFit::ScaleDown => contain.min(1.0),
    };
    let (w, h) = (iw * scale, ih * scale);
    ((bw - w) / 2.0, (bh - h) / 2.0, w, h)
}

/// The value a touch at absolute x `x` selects on a `Slider` occupying `rect`
/// (../PLAN.md §26). Pure geometry — no `Renderer` state, no taffy, no parley — which
/// is the point of it living here rather than in `gpu_harness`: that crate's
/// `build.rs` compiles gametextinput's C++ against NDK headers, so it does not build
/// for the host at all and nothing in it can carry a test (../PLAN.md §15.5). The
/// same call `hit_test_text_offset` already makes for tap-to-position-cursor.
///
/// **The fraction is taken against the node's full width, which is what the *fill*
/// shows.** `Slider._tree_children` (`phase_1/zkite_py/src/zkite/widgets.py`) paints the
/// fill as `w-[pct%]` of a `w-full` track, so mapping this way makes the filled
/// portion end exactly under the finger. The *thumb* is `left-[pct%]` and left-edge
/// aligned, so it reads half a thumb-width (8dp) to the right of that — a
/// pre-existing, already-documented cosmetic gap (real Tailwind centres it with
/// `-translate-x-1/2`, a percent-of-own-size translate `twcore` does not implement),
/// deliberately not changed here: it is a visual question, it would move §9's
/// conformance numbers, and it is not what "the slider does not respond" meant.
///
/// `step` quantises from `min`, matching `<input type=range>`'s own definition, so a
/// slider declared `min=5 step=10` yields 5/15/25/... and never a bare multiple of 10.
/// A non-finite or non-positive `step` means unquantised. A zero-width rect or a
/// `max <= min` yields `min` rather than a NaN, because both are reachable from
/// ordinary user code (`Slider("w-0")`, `Slider(min=1, max=1)`) and neither should be
/// able to poison a store with a NaN.
pub fn slider_value_at(rect: (f32, f32, f32, f32), x: f32, min: f32, max: f32, step: f32) -> f32 {
    let (rx, _, rw, _) = rect;
    if !(max > min) || !rw.is_finite() || rw <= 0.0 {
        return min;
    }
    let fraction = ((x - rx) / rw).clamp(0.0, 1.0);
    let raw = min + fraction * (max - min);
    let quantised = if step.is_finite() && step > 0.0 {
        min + ((raw - min) / step).round() * step
    } else {
        raw
    };
    quantised.clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_renderer() -> Renderer {
        // DejaVu Sans (+ its bundled Bold face) is what every host-side spike/test in
        // this repo already relies on being present (see spike1's `ZKITE_SPIKE_FONT`
        // default) — not bundled here since these tests never rasterize, only
        // measure/layout, and a missing font should fail loudly rather than silently
        // skip.
        // All three real DejaVu families, so the font-family tests below have
        // genuinely different typefaces to distinguish rather than three aliases of
        // one. Not bundled here since these tests never rasterize, only measure and
        // lay out, and a missing font should fail loudly rather than silently skip.
        Renderer::new(FontSet {
            sans: read_faces("DejaVuSans"),
            serif: Some(read_faces("DejaVuSerif")),
            mono: Some(read_faces("DejaVuSansMono")),
        })
        .unwrap()
    }

    fn read_faces(stem: &str) -> FontFaces {
        let dir = std::env::var("ZKITE_TEST_FONT_DIR")
            .unwrap_or_else(|_| "/usr/share/fonts/truetype/dejavu".to_string());
        let read = |path: String| {
            std::fs::read(&path)
                .unwrap_or_else(|_| panic!("{path} must be installed for zkite_renderer's tests"))
        };
        FontFaces::new(read(format!("{dir}/{stem}.ttf")), read(format!("{dir}/{stem}-Bold.ttf")))
    }

    /// Every class string introduced by `zkite_py`'s tree export (`Widget.to_tree`,
    /// `phase_1/zkite_py/src/zkite/widgets.py`) and its Switch/Slider/error-overlay
    /// synthesis must actually parse — an unknown class silently renders as if it
    /// were never written, which for e.g. a missing `rounded-full` is a real visual
    /// bug, not a crash, so nothing else would catch it. This sweep already caught
    /// two real ones this session: `Stack`'s original `col-start-1`/`row-start-1`
    /// (twcore has no grid *line placement*, only spans — fixed by switching the
    /// native tree to `absolute inset-0`) and `Icon`'s original `w-[1em]`/`h-[1em]`
    /// (twcore's arbitrary-length parser has no `em` unit — fixed with a fixed-px
    /// fallback). Both fixes live in `Widget._tree_classes()` overrides, not here.
    #[test]
    fn ellipsis_and_line_limits_are_part_of_shaping() {
        let mut renderer = test_renderer();
        let node = renderer.text_with_options("text-base w-[100px]", "one two three four five six seven eight", TextOptions {
            wrap: true, max_lines: Some(2), ellipsis: true,
        });
        renderer.layout(node, 100.0, 400.0);
        let nodes = renderer.positioned(node, &FxHashMap::default());
        assert!(nodes[0].rect.3 <= 49.0);
        assert!(nodes[0].rect.2 <= 100.0);
    }

    #[test]
    fn zkite_py_native_classes_all_resolve() {
        let mut r = test_renderer();
        let samples = [
            // Switch track + thumb (the leading `appearance-none` is intentionally
            // excluded below — see `expected_html_only_classes_are_unknown`)
            "flex items-center w-[44px] h-[24px] rounded-full p-[2px] bg-indigo-500 justify-end",
            "flex items-center w-[44px] h-[24px] rounded-full p-[2px] bg-slate-700 justify-start",
            "w-[20px] h-[20px] rounded-full bg-white",
            // Slider container + track + fill + thumb
            "w-full relative flex items-center h-[16px]",
            "w-full h-[6px] rounded-full bg-slate-700 relative",
            "absolute left-0 top-0 bottom-0 rounded-full bg-indigo-500 w-[30.0000%]",
            "absolute top-0 bottom-0 w-[16px] rounded-full bg-white left-[30.0000%]",
            // Icon, after the `_tree_classes` em -> fixed-px fix
            "inline-block w-[20px] h-[20px]",
            // Stack, after the `_tree_classes`/`_tree_children` grid -> absolute fix
            "relative",
            "absolute inset-0",
            // error overlay
            "relative w-full h-full",
            "absolute inset-0 bg-black p-4 text-red-400 text-xs",
            // ScrollView, both orientations — verbatim `implied_classes`, because the
            // horizontal set introduced two tokens (`overflow-x-auto`, `min-w-0`) that
            // nothing in this list had exercised before (../PLAN.md §27).
            "flex flex-col overflow-auto min-h-0",
            "flex flex-row overflow-x-auto min-w-0",
        ];
        for classes in samples {
            let unknown = r.unknown_classes(classes);
            assert!(unknown.is_empty(), "unrecognised classes in {classes:?}: {unknown:?}");
        }
    }

    /// The classes documented (not silently dropped) as permanently HTML-only: they
    /// reset a browser's native form-control chrome (`appearance-none`) or a focus
    /// ring (`outline-none`) that a from-scratch native renderer has no equivalent
    /// concept of painting in the first place. If either ever starts resolving
    /// (twcore grows the feature), this test should start failing loudly — that's the
    /// point of asserting the negative explicitly instead of just omitting them from
    /// the sweep above.
    ///
    /// **`object-cover` used to be on this list and this test is what caught its
    /// removal** (../PLAN.md §35). It was listed as "bitmap scaling for a decoder that
    /// doesn't exist yet", with the reasoning that `Image` — unlike `Icon` — doesn't
    /// need the class's *absence* compensated for, because taffy sizes its box from
    /// the ordinary `w-`/`h-` rules either way. That reasoning was right and is why
    /// adding `object-*` to `twcore` changed no layout anywhere; what changed is that
    /// there is now a decoder to scale a bitmap *for*. The alarm firing on the first
    /// `cargo test` after the parser grew the utility is the behaviour this test was
    /// written for, and it is recorded rather than quietly edited away.
    #[test]
    fn expected_html_only_classes_are_unknown() {
        let mut r = test_renderer();
        for classes in ["appearance-none", "outline-none"] {
            assert_eq!(r.unknown_classes(classes), vec![classes.to_string()]);
        }
    }

    /// `object_fit_rect` against a box and a bitmap whose aspect ratios differ, which
    /// is the only shape where the five modes are distinguishable at all.
    ///
    /// A 200x100 box (2:1) and a 100x100 bitmap (1:1) — chosen so every expected
    /// number is exact in binary and none of the five answers coincides with another.
    #[test]
    fn object_fit_places_a_square_bitmap_in_a_wide_box_five_different_ways() {
        use twcore::style::ObjectFit;
        let boxed = (200.0, 100.0);
        let img = (100.0, 100.0);
        let f = |fit| object_fit_rect(fit, boxed, img);

        // Stretched to the box on both axes; the only mode that distorts.
        assert_eq!(f(ObjectFit::Fill), (0.0, 0.0, 200.0, 100.0));
        // Fits inside: scale 1.0 (height-bound), centred horizontally, letterboxed.
        assert_eq!(f(ObjectFit::Contain), (50.0, 0.0, 100.0, 100.0));
        // Covers: scale 2.0 (width-bound), so it overflows 100dp above and below.
        assert_eq!(f(ObjectFit::Cover), (0.0, -50.0, 200.0, 200.0));
        // Intrinsic size, centred on both axes.
        assert_eq!(f(ObjectFit::None), (50.0, 0.0, 100.0, 100.0));
        // Contain's scale here is exactly 1.0, so scale-down agrees with both.
        assert_eq!(f(ObjectFit::ScaleDown), (50.0, 0.0, 100.0, 100.0));
    }

    /// The one case that separates `ScaleDown` from `Contain`, which the fixture above
    /// deliberately cannot: a bitmap *smaller* than its box. `Contain` scales it up,
    /// `ScaleDown` refuses to.
    ///
    /// Written because a `ScaleDown` arm that simply delegated to `Contain` would pass
    /// every assertion in the test above.
    #[test]
    fn scale_down_never_enlarges_where_contain_does() {
        use twcore::style::ObjectFit;
        let boxed = (400.0, 400.0);
        let img = (100.0, 50.0);
        assert_eq!(object_fit_rect(ObjectFit::Contain, boxed, img), (0.0, 100.0, 400.0, 200.0));
        assert_eq!(object_fit_rect(ObjectFit::ScaleDown, boxed, img), (150.0, 175.0, 100.0, 50.0));
        // ...and where the image is bigger than the box, they agree again.
        let big = (800.0, 400.0);
        assert_eq!(
            object_fit_rect(ObjectFit::ScaleDown, boxed, big),
            object_fit_rect(ObjectFit::Contain, boxed, big)
        );
    }

    /// Degenerate inputs reachable from ordinary user code — `Image("w-0")`, a 0x0 or
    /// corrupt-header bitmap — must produce a zero-area rect, never a NaN and never an
    /// inverted one. A NaN here reaches a rasteriser; §26's slider test exists for the
    /// same class of reason.
    #[test]
    fn object_fit_degenerate_inputs_yield_an_empty_rect_and_never_a_nan() {
        use twcore::style::ObjectFit;
        let bad = [
            ((0.0, 100.0), (10.0, 10.0)),
            ((100.0, 0.0), (10.0, 10.0)),
            ((100.0, 100.0), (0.0, 10.0)),
            ((100.0, 100.0), (10.0, 0.0)),
            ((-100.0, 100.0), (10.0, 10.0)),
            ((100.0, 100.0), (f32::NAN, 10.0)),
        ];
        for fit in [
            ObjectFit::Fill,
            ObjectFit::Contain,
            ObjectFit::Cover,
            ObjectFit::None,
            ObjectFit::ScaleDown,
        ] {
            for (boxed, img) in bad {
                let r = object_fit_rect(fit, boxed, img);
                assert_eq!(r, (0.0, 0.0, 0.0, 0.0), "{fit:?} {boxed:?} {img:?}");
                assert!(!r.0.is_nan() && !r.1.is_nan(), "{fit:?} produced a NaN");
            }
        }
    }

    /// An `Image` leaf occupies the same box it did before anything painted, and
    /// carries its `src` through to `positioned()`.
    ///
    /// The box half is the regression guard that matters: `Image` has always reserved
    /// correct layout space (that is what "paints an empty box of the correct size"
    /// meant), so every app that already positions one must be unmoved by this work.
    /// Asserted against a plain container with the identical class string rather than
    /// against hardcoded numbers, so it keeps holding if the spacing scale is ever
    /// corrected underneath it.
    #[test]
    fn an_image_leaf_carries_its_src_and_occupies_the_same_box_as_a_plain_container() {
        let classes = "w-32 h-24 rounded-lg";
        let (image_rect, container_rect) = {
            let mut r = test_renderer();
            let img = r.image(classes, "logo.png");
            let root = r.container("flex flex-col", |_| vec![img]);
            r.layout(root, 400.0, 400.0);
            let nodes = r.positioned(root, &FxHashMap::default());
            let img_node = nodes.iter().find(|n| n.node == img).expect("the image is positioned");
            assert_eq!(
                img_node.image.as_deref(),
                Some("logo.png"),
                "the src reaches the paint layer"
            );
            let image_rect = img_node.rect;

            let mut r2 = test_renderer();
            let plain = r2.container(classes, |_| vec![]);
            let root2 = r2.container("flex flex-col", |_| vec![plain]);
            r2.layout(root2, 400.0, 400.0);
            let nodes2 = r2.positioned(root2, &FxHashMap::default());
            let rect2 = nodes2.iter().find(|n| n.node == plain).unwrap().rect;
            (image_rect, rect2)
        };
        assert_eq!(image_rect, container_rect, "an image must not lay out differently");
    }

    /// An `Image` with no `src` records nothing, so it degrades to the empty box it
    /// was before rather than reaching the paint layer with an empty path to open.
    #[test]
    fn an_image_with_no_src_carries_no_image() {
        let mut r = test_renderer();
        let img = r.image("w-8 h-8", "");
        let root = r.container("flex flex-col", |_| vec![img]);
        r.layout(root, 100.0, 100.0);
        let nodes = r.positioned(root, &FxHashMap::default());
        assert!(nodes.iter().find(|n| n.node == img).unwrap().image.is_none());
    }

    /// The other half of the claim above: every `object-*` utility now resolves, and
    /// resolves to a *distinct* value rather than all collapsing onto the default.
    ///
    /// The distinctness half is the one that matters. A `match` arm that returned
    /// `ObjectFit::Fill` for all five would pass `unknown_classes` — the class is
    /// recognised — while painting every image stretched, and no other test in this
    /// crate reads `object_fit` at all.
    #[test]
    fn every_object_fit_utility_resolves_to_its_own_distinct_value() {
        use twcore::style::ObjectFit;
        let mut r = test_renderer();
        let cases = [
            ("object-fill", ObjectFit::Fill),
            ("object-contain", ObjectFit::Contain),
            ("object-cover", ObjectFit::Cover),
            ("object-none", ObjectFit::None),
            ("object-scale-down", ObjectFit::ScaleDown),
        ];
        for (classes, expected) in cases {
            assert!(r.unknown_classes(classes).is_empty(), "{classes} should resolve");
            let id = r.image(classes, "x.png");
            let style = r.container_styles.get(&id).expect("an image leaf carries a style");
            assert_eq!(style.paint.object_fit, expected, "{classes}");
        }
        // CSS's default for a replaced element, and deliberately *not* `Cover` even
        // though every `zkite_py` `Image` carries `object-cover` in `implied_classes`.
        let bare = r.image("", "x.png");
        let style = r.container_styles.get(&bare).unwrap();
        assert_eq!(style.paint.object_fit, ObjectFit::Fill, "an unclassed image is CSS's `fill`");
    }

    /// The bug `resolve_inherited` exists to fix: a bare string child (no classes of
    /// its own) inside a styled container must inherit that container's typography —
    /// this is exactly `phase_1/zkite_py/tests/test_app_counter.py`'s reference
    /// `Button("... text-white font-medium ...", "Increment")` shape.
    #[test]
    fn bare_text_child_inherits_container_typography() {
        let mut r = test_renderer();
        let mut label = None;
        let _button = r.container("bg-indigo-500 text-white font-medium", |r| {
            label = Some(r.text("", "Increment"));
            vec![label.unwrap()]
        });
        let style = r.tree.get_node_context(label.unwrap()).unwrap().style.clone();
        assert_eq!(style.text.color, Some(Color::WHITE));
        assert_eq!(style.text.weight, 500, "font-medium should be inherited, not default 400");
    }

    /// A text leaf's *own* classes must still win over whatever it would otherwise
    /// inherit — inheritance only fills in what a node didn't set itself.
    #[test]
    fn text_own_classes_override_inherited() {
        let mut r = test_renderer();
        let mut label = None;
        let _row = r.container("text-white", |r| {
            label = Some(r.text("text-red-500", "Alert"));
            vec![label.unwrap()]
        });
        let style = r.tree.get_node_context(label.unwrap()).unwrap().style.clone();
        assert_ne!(style.text.color, Some(Color::WHITE));
    }

    /// Inheritance must chain through more than one level of nesting (grandparent ->
    /// child container with no typography of its own -> bare text grandchild).
    #[test]
    fn inheritance_chains_through_nested_containers() {
        let mut r = test_renderer();
        let mut leaf = None;
        let _outer = r.container("text-white font-bold", |r| {
            vec![r.container("flex", |r| {
                leaf = Some(r.text("", "deep"));
                vec![leaf.unwrap()]
            })]
        });
        let style = r.tree.get_node_context(leaf.unwrap()).unwrap().style.clone();
        assert_eq!(style.text.color, Some(Color::WHITE));
        assert_eq!(style.text.weight, 700);
    }

    /// A scroll container (`overflow-y-scroll`) whose children overflow its own
    /// height: `max_scroll_y` must report real, positive room to scroll, and once an
    /// offset is applied via `positioned`, children shift up while the container's
    /// *own* rect stays put, and downstream content gets clipped to the container's
    /// box rather than bleeding past it.
    #[test]
    fn clipped_scroll_rows_do_not_intercept_header_or_footer_taps() {
        let mut r = test_renderer();
        let mut header = None;
        let mut footer = None;
        let mut list = None;
        let mut rows = Vec::new();
        let root = r.container("flex flex-col w-[100px] h-[180px]", |r| {
            header = Some(r.container("h-[40px] shrink-0", |_| vec![]));
            list = Some(r.container("flex flex-col overflow-auto h-[100px] shrink-0", |r| {
                for _ in 0..5 {
                    rows.push(r.container("h-[40px] shrink-0", |_| vec![]));
                }
                rows.clone()
            }));
            footer = Some(r.container("h-[40px] shrink-0", |_| vec![]));
            vec![header.unwrap(), list.unwrap(), footer.unwrap()]
        });
        r.layout(root, 100.0, 180.0);
        let mut offsets = FxHashMap::default();
        offsets.insert(list.unwrap(), (0.0, 40.0));
        assert_eq!(r.hit_test(root, 20.0, 20.0, &offsets), header);
        assert_eq!(r.hit_test(root, 20.0, 60.0, &offsets), Some(rows[1]));
        assert_eq!(r.hit_test(root, 20.0, 160.0, &offsets), footer);
    }

    #[test]
    fn overflow_hidden_clips_hit_targets_on_both_axes() {
        let mut r = test_renderer();
        let mut child = None;
        let root = r.container("overflow-hidden w-[50px] h-[50px]", |r| {
            child = Some(r.container("w-[100px] h-[100px]", |_| vec![]));
            vec![child.unwrap()]
        });
        r.layout(root, 100.0, 100.0);
        assert_eq!(r.hit_test(root, 25.0, 25.0, &Default::default()), child);
        assert_eq!(r.hit_test(root, 75.0, 25.0, &Default::default()), None);
        assert_eq!(r.hit_test(root, 25.0, 75.0, &Default::default()), None);
    }

    #[test]
    fn scroll_offset_shifts_children_and_clips_without_moving_the_container() {
        let mut r = test_renderer();
        let mut rows = Vec::new();
        let list = r.container("flex flex-col overflow-y-scroll h-[100px] w-[100px]", |r| {
            for _ in 0..5 {
                // Each row is a fixed 40px box — five of them (200px) inside a 100px
                // viewport is a real, unambiguous overflow, *if* they're not allowed
                // to flex-shrink to fit first. `shrink-0` isn't decorative here: this
                // is standard flexbox behavior (matching real CSS, not a taffy quirk
                // — confirmed by first writing this test without it and watching
                // `max_scroll_y` come back 0, the children shrunk to fit instead of
                // overflowing), and it's exactly the trap a real `ScrollView`'s
                // children fall into without it. Worth a usage note — see
                // ../PLAN.md §5.
                rows.push(r.container("h-[40px] w-full shrink-0", |_| vec![]));
            }
            rows.clone()
        });
        let root = r.container("w-[100px] h-[100px]", |_| vec![list]);
        r.layout(root, 100.0, 100.0);

        let max_scroll = r.max_scroll_y(list);
        assert_eq!(max_scroll, 100.0, "200px of content in a 100px box scrolls exactly 100px");

        // Unscrolled: the container's own rect is (0,0,100,100), and its first
        // child's top is flush with it.
        let unscrolled = r.positioned(root, &Default::default());
        let container_rect = unscrolled.iter().find(|n| n.node == list).unwrap().rect;
        assert_eq!(container_rect, (0.0, 0.0, 100.0, 100.0));
        let first_row_y = unscrolled.iter().find(|n| n.node == rows[0]).unwrap().rect.1;
        assert_eq!(first_row_y, 0.0);

        // Scrolled by 40px: the container itself hasn't moved, but its first child
        // has, and the fifth row (now the only one even partially visible) is clipped
        // to the container's unmoved box, not its own (now off-screen-shifted) rect.
        let mut offsets = FxHashMap::default();
        offsets.insert(list, (0.0, 40.0));
        let scrolled = r.positioned(root, &offsets);
        let container_rect2 = scrolled.iter().find(|n| n.node == list).unwrap().rect;
        assert_eq!(container_rect2, (0.0, 0.0, 100.0, 100.0), "the viewport itself never moves");
        let first_row = scrolled.iter().find(|n| n.node == rows[0]).unwrap();
        assert_eq!(first_row.rect.1, -40.0, "content shifts up by the scroll offset");
        assert_eq!(first_row.clip, Some((0.0, 0.0, 100.0, 100.0)), "clipped to the container's box");
    }

    /// The x-axis mirror of the test above, and the reason `max_scroll` is
    /// axis-generic rather than two copies: a `flex-row overflow-x-scroll` container
    /// whose children overflow its *width* reports real horizontal room, and applying
    /// an x offset shifts children left without moving the viewport.
    ///
    /// `shrink-0` on the children is load-bearing for exactly the reason the vertical
    /// test documents, one word different — a flex-row child's default `min-width:
    /// auto` is what usually saves content-sized children, and an explicitly-sized
    /// one has no content minimum to be saved by.
    #[test]
    fn horizontal_scroll_offset_shifts_children_and_clips_without_moving_the_container() {
        let mut r = test_renderer();
        let mut cards = Vec::new();
        let strip = r.container("flex flex-row overflow-x-scroll w-[100px] h-[60px]", |r| {
            for _ in 0..5 {
                cards.push(r.container("w-[40px] h-full shrink-0", |_| vec![]));
            }
            cards.clone()
        });
        let root = r.container("w-[100px] h-[100px]", |_| vec![strip]);
        r.layout(root, 100.0, 100.0);

        assert_eq!(
            r.max_scroll(strip, ScrollAxis::X),
            100.0,
            "200px of content in a 100px box scrolls exactly 100px"
        );
        assert_eq!(r.max_scroll(strip, ScrollAxis::Y), 0.0, "nothing overflows vertically");

        let mut offsets = FxHashMap::default();
        offsets.insert(strip, (40.0, 0.0));
        let scrolled = r.positioned(root, &offsets);
        let strip_rect = scrolled.iter().find(|n| n.node == strip).unwrap().rect;
        assert_eq!(strip_rect, (0.0, 0.0, 100.0, 60.0), "the viewport itself never moves");
        let first = scrolled.iter().find(|n| n.node == cards[0]).unwrap();
        assert_eq!(first.rect.0, -40.0, "content shifts left by the scroll offset");
        assert_eq!(first.clip, Some((0.0, 0.0, 100.0, 60.0)), "clipped to the container's box");
    }

    /// A padded scroll container's **end padding is part of what scrolls**, on both
    /// axes — found on a real device (§27) by flinging a `px-2` strip to its bound and
    /// finding the last card flush against the screen edge with the padding gone.
    ///
    /// Both axes are asserted here because the bug was never x-specific: `max_scroll_y`
    /// had the identical shortfall from §6.1 onwards and no fixture could see it, since
    /// the conformance harness only ever renders offset 0.
    #[test]
    fn a_padded_scroll_container_can_scroll_its_end_padding_into_view() {
        for (axis, dir, size) in [
            (ScrollAxis::X, "flex-row overflow-x-scroll", "w-[100px] h-[60px]"),
            (ScrollAxis::Y, "flex-col overflow-y-scroll", "w-[60px] h-[100px]"),
        ] {
            let mut r = test_renderer();
            // p-2 is 8px a side. Five 40px children on the scroll axis = 200px of
            // content inside a 100px box that also carries 8px of padding each side.
            let child = if axis == ScrollAxis::X { "w-[40px] h-full shrink-0" } else { "h-[40px] w-full shrink-0" };
            let scroller = r.container(&format!("flex {dir} p-2 {size}"), |r| {
                (0..5).map(|_| r.container(child, |_| vec![])).collect()
            });
            let root = r.container("w-[200px] h-[200px]", |_| vec![scroller]);
            r.layout(root, 200.0, 200.0);

            // Content runs from 8 (past the start padding) to 208; the scrollable area
            // ends at 208 + 8 of end padding = 216; the scrollport is 100 wide.
            assert_eq!(
                r.max_scroll(scroller, axis),
                116.0,
                "{axis:?}: 216 of scrollable area in a 100 box — the 8px end padding included"
            );
        }
    }

    /// **The trap this whole axis-generic API exists to avoid.** `overflow-auto`
    /// parses to `Axis::Both` (`phase_0/spike3_twcore/src/parse.rs:243`) and
    /// `ScrollView`/`List` bake it into their implied classes — so an ordinary
    /// *vertical* list declares itself horizontally scrollable and always has.
    ///
    /// A gesture layer that resolved its scroll target off `is_scrollable` alone
    /// would therefore hand every horizontal drag to a vertical list, which would
    /// then move nothing: not a crash, not an error, just a dead gesture. That is the
    /// same silent-divergence shape as every entry in ../PLAN.md §15.1, and it is why
    /// `scrollable_ancestor` filters on room.
    #[test]
    fn a_vertical_scrollview_declares_x_scrollable_but_is_not_a_horizontal_scroll_target() {
        let mut r = test_renderer();
        let mut rows = Vec::new();
        // Exactly `ScrollView.implied_classes` plus a size, not an approximation of it.
        let list = r.container("flex flex-col overflow-auto min-h-0 h-[100px] w-[100px]", |r| {
            for _ in 0..5 {
                rows.push(r.container("h-[40px] w-full shrink-0", |_| vec![]));
            }
            rows.clone()
        });
        let root = r.container("w-[100px] h-[100px]", |_| vec![list]);
        r.layout(root, 100.0, 100.0);

        // The declaration really is there on both axes. This assert is the point of
        // the test: if a future `twcore` change made `overflow-auto` y-only, this
        // fails and the reader is sent here rather than quietly losing the coverage.
        assert!(r.is_scrollable(list, ScrollAxis::X), "overflow-auto declares Axis::Both");
        assert!(r.is_scrollable(list, ScrollAxis::Y));

        assert_eq!(r.max_scroll(list, ScrollAxis::X), 0.0, "but there is no horizontal room");

        let deepest = rows[0];
        assert_eq!(
            r.scrollable_ancestor(deepest, ScrollAxis::Y),
            Some((list, 100.0)),
            "a vertical drag resolves to the list"
        );
        assert_eq!(
            r.scrollable_ancestor(deepest, ScrollAxis::X),
            None,
            "a horizontal drag must resolve to nothing at all, not to the list"
        );
    }

    /// The nesting case horizontal scrolling exists for: a horizontal strip inside a
    /// vertical list. The two axes must resolve to *different* containers from the
    /// same touch point — which is why the gesture layer resolves a target per axis
    /// rather than one target and an axis.
    #[test]
    fn a_horizontal_strip_inside_a_vertical_list_resolves_a_different_target_per_axis() {
        let mut r = test_renderer();
        let mut card = None;
        let mut strip = None;
        let list = r.container("flex flex-col overflow-auto min-h-0 h-[100px] w-[100px]", |r| {
            let s = r.container("flex flex-row overflow-x-auto min-w-0 w-full h-[60px] shrink-0", |r| {
                let mut cards = Vec::new();
                for _ in 0..5 {
                    cards.push(r.container("w-[40px] h-full shrink-0", |_| vec![]));
                }
                card = Some(cards[0]);
                cards
            });
            strip = Some(s);
            let mut kids = vec![s];
            for _ in 0..3 {
                kids.push(r.container("h-[40px] w-full shrink-0", |_| vec![]));
            }
            kids
        });
        let root = r.container("w-[100px] h-[100px]", |_| vec![list]);
        r.layout(root, 100.0, 100.0);
        let (card, strip) = (card.unwrap(), strip.unwrap());

        assert_eq!(
            r.scrollable_ancestor(card, ScrollAxis::X).map(|(n, _)| n),
            Some(strip),
            "a horizontal drag on a card belongs to the strip"
        );
        assert_eq!(
            r.scrollable_ancestor(card, ScrollAxis::Y).map(|(n, _)| n),
            Some(list),
            "a vertical drag on the same card belongs to the list underneath it"
        );
    }

    /// `scrollable_ancestor` skips a declared-but-full container and keeps walking —
    /// the browser's scroll-chaining behaviour, resolved at touch-down. Without the
    /// skip, a horizontal strip with nothing to scroll would swallow a vertical drag
    /// intended for the list around it.
    #[test]
    fn scrollable_ancestor_skips_a_container_with_no_room_and_keeps_walking() {
        let mut r = test_renderer();
        let mut card = None;
        // One card, 40px, in a 100px-wide strip: declared x-scrollable, no room.
        let list = r.container("flex flex-col overflow-auto min-h-0 h-[100px] w-[100px]", |r| {
            let s = r.container("flex flex-row overflow-x-auto min-w-0 w-full h-[60px] shrink-0", |r| {
                let c = r.container("w-[40px] h-full shrink-0", |_| vec![]);
                card = Some(c);
                vec![c]
            });
            let mut kids = vec![s];
            for _ in 0..3 {
                kids.push(r.container("h-[40px] w-full shrink-0", |_| vec![]));
            }
            kids
        });
        let root = r.container("w-[100px] h-[100px]", |_| vec![list]);
        r.layout(root, 100.0, 100.0);

        assert_eq!(
            r.scrollable_ancestor(card.unwrap(), ScrollAxis::X),
            None,
            "a full strip is not a horizontal target, and nothing above it is either"
        );
        assert_eq!(
            r.scrollable_ancestor(card.unwrap(), ScrollAxis::Y).map(|(n, _)| n),
            Some(list),
            "the vertical drag still reaches the list through the full strip"
        );
    }

    /// Quick host-side reproduction attempt for a real device bug (../../phase_3/
    /// PLAN.md §5, IME): after a `TextField` gains focus, an *unrelated* text node
    /// elsewhere in the same tree (a large counter digit) stops rendering. Single-
    /// shot (one `layout()` call) with an empty sibling didn't reproduce it — this
    /// version instead mirrors `render_thread.rs::redraw()`'s *real* pattern:
    /// the same long-lived `Renderer` across multiple `clear()`+build+`layout()`
    /// cycles (unfocused, unfocused, then focused — the exact sequence a real
    /// device run showed the bug appearing on the *third* redraw, not the first),
    /// checking whether something in `font_cx`/`layout_cx` — the state `clear()`
    /// deliberately does *not* reset, since it's meant to stay warm across renders
    /// — accumulates a problem specifically from shaping an empty string.
    #[test]
    fn empty_text_leaf_does_not_corrupt_a_sibling_after_repeated_layouts() {
        let mut r = test_renderer();
        let build = |r: &mut Renderer, empty: bool| -> (NodeId, NodeId) {
            let mut real_text = None;
            let mut empty_text = None;
            let root = r.container("flex flex-col", |r| {
                real_text = Some(r.text("text-7xl", "0"));
                empty_text = Some(r.text("", if empty { "" } else { "placeholder" }));
                vec![real_text.unwrap(), empty_text.unwrap()]
            });
            (root, real_text.unwrap())
        };
        let glyph_count = |r: &Renderer, root: NodeId, node: NodeId| -> usize {
            let nodes = r.positioned(root, &Default::default());
            nodes
                .iter()
                .find(|n| n.node == node)
                .and_then(|n| n.text.as_ref())
                .map(|runs| runs.iter().map(|run| run.glyphs.len()).sum())
                .unwrap_or(0)
        };

        // Pass 1 & 2: sibling has real placeholder content (unfocused state).
        for _ in 0..2 {
            let (root, real_text) = build(&mut r, false);
            r.layout(root, 400.0, 800.0);
            assert!(glyph_count(&r, root, real_text) > 0, "real text must render before focus");
        }
        // Pass 3: sibling becomes empty (focused, nothing typed yet) — this is the
        // exact transition that broke on a real device.
        let (root, real_text) = build(&mut r, true);
        r.layout(root, 400.0, 800.0);
        assert!(
            glyph_count(&r, root, real_text) > 0,
            "the real text's glyphs must survive its sibling becoming empty on a *later* layout pass, using the same long-lived Renderer"
        );
    }

    /// `hit_test_text_offset`/`text_caret_geometry` (../../gpu_harness/PLAN.md §7.7,
    /// tap-to-position-cursor): a tap near the start of "hello world" should offset
    /// near 0, a tap near the end should offset near the string's byte length, and
    /// the caret geometry for a middle offset should sit strictly between the two
    /// x-extents — basic sanity a hand-rolled glyph-x-guess could get backwards
    /// (e.g. RTL-style reversal) without failing to compile.
    #[test]
    fn tap_to_offset_and_caret_geometry_agree_with_text_order() {
        let mut r = test_renderer();
        let style = TextStyle { size: 32.0, ..Default::default() };
        let content = "hello world";

        let start_offset = r.hit_test_text_offset(&style, content, 1.0, 10.0);
        let end_offset = r.hit_test_text_offset(&style, content, 10_000.0, 10.0);
        assert!(start_offset <= 1, "a tap at the very start should offset at/near 0, got {start_offset}");
        assert_eq!(end_offset, content.len(), "a tap far past the end should clamp to the text's full byte length");

        let (start_x, _, _, _) = r.text_caret_geometry(&style, content, 0, 2.0);
        let (mid_x, _, _, _) = r.text_caret_geometry(&style, content, 5, 2.0);
        let (end_x, _, _, _) = r.text_caret_geometry(&style, content, content.len(), 2.0);
        assert!(start_x < mid_x, "caret x must increase left-to-right for plain LTR text: start={start_x} mid={mid_x}");
        assert!(mid_x < end_x, "caret x must increase left-to-right for plain LTR text: mid={mid_x} end={end_x}");
    }

    /// The bug ../PLAN.md §9.2 item 1 found by pixel-diffing against the WebView
    /// backend: with only one font face ever registered, `font-bold`'s
    /// `StyleProperty::FontWeight(700)` had nothing heavier to select, so every
    /// weight silently rendered identically to `font-normal`. `Renderer::new` now
    /// registers a second (bold) face under the same family — this locks in that a
    /// `font-bold` text leaf actually shapes with a *different* font blob than a
    /// `font-normal` one, through the real `text()`/`layout()` path (not the
    /// isolated `shape_isolated` helper the tap/caret tests use), and that the bold
    /// run is visibly wider, matching real bold rendering rather than a same-width
    /// coincidence.
    #[test]
    fn font_bold_selects_a_different_wider_font_face_than_font_normal() {
        let mut r = test_renderer();
        let mut normal = None;
        let mut bold = None;
        // `items-start` matters: default flex-col cross-axis alignment is `stretch`,
        // which would force both leaves to the container's own (max-child) width and
        // hide a real width difference behind an identical stretched box — this test
        // needs each leaf's *natural* (content) width, so cross-axis stretch must be
        // off.
        let root = r.container("flex flex-col items-start", |r| {
            normal = Some(r.text("text-2xl font-normal", "Hello"));
            bold = Some(r.text("text-2xl font-bold", "Hello"));
            vec![normal.unwrap(), bold.unwrap()]
        });
        r.layout(root, 400.0, 200.0);

        let positioned = r.positioned(root, &Default::default());

        let normal_node = positioned.iter().find(|n| n.node == normal.unwrap()).unwrap();
        let bold_node = positioned.iter().find(|n| n.node == bold.unwrap()).unwrap();
        let normal_runs = normal_node.text.as_ref().unwrap();
        let bold_runs = bold_node.text.as_ref().unwrap();
        assert_eq!(normal_runs.len(), 1);
        assert_eq!(bold_runs.len(), 1);
        assert_ne!(
            normal_runs[0].font, bold_runs[0].font,
            "font-bold must select a different registered font face than font-normal, not silently fall back to the only face registered"
        );
        assert!(
            bold_node.rect.2 > normal_node.rect.2,
            "the same string set bold should measure wider than set regular: bold width={:.4} normal width={:.4}",
            bold_node.rect.2,
            normal_node.rect.2
        );
    }

    /// `font-sans`/`font-serif`/`font-mono` must each select their own registered
    /// typeface. Before this, `layout()`'s measure closure pushed size and weight but
    /// never a family, so parley fell back to its own default on every shaping pass
    /// and all three rendered identically — the class parsed, inherited, and reached
    /// `TextStyle::family` correctly, and then nothing read it (../PLAN.md §9.4).
    ///
    /// Asserting on the selected `FontData` rather than on width is deliberate: three
    /// typefaces *could* coincidentally measure the same string to the same width,
    /// and a width assertion would then pass while the bug was fully present.
    #[test]
    fn each_generic_family_selects_its_own_registered_typeface() {
        let mut r = test_renderer();
        let (mut sans, mut serif, mut mono) = (None, None, None);
        let root = r.container("flex flex-col items-start", |r| {
            sans = Some(r.text("text-2xl font-sans", "Hello"));
            serif = Some(r.text("text-2xl font-serif", "Hello"));
            mono = Some(r.text("text-2xl font-mono", "Hello"));
            vec![sans.unwrap(), serif.unwrap(), mono.unwrap()]
        });
        r.layout(root, 400.0, 300.0);
        let positioned = r.positioned(root, &Default::default());

        let font_of = |node: NodeId| {
            let n = positioned.iter().find(|n| n.node == node).unwrap();
            let runs = n.text.as_ref().unwrap();
            assert_eq!(runs.len(), 1);
            runs[0].font.clone()
        };
        let (s, se, m) = (font_of(sans.unwrap()), font_of(serif.unwrap()), font_of(mono.unwrap()));
        assert_ne!(s, se, "font-serif must not resolve to the sans face");
        assert_ne!(s, m, "font-mono must not resolve to the sans face");
        assert_ne!(se, m, "font-serif and font-mono must not resolve to the same face");
    }

    /// The family has to reach the *isolated* shaping path too, not just the measure
    /// pass. `shape_isolated` is what backs caret geometry and tap-to-offset
    /// hit-testing (§7.7); if it shaped a different typeface than the one painted,
    /// every tap in a non-default-family field would land at the wrong byte offset —
    /// a bug that is invisible in a screenshot and only shows up under a finger.
    #[test]
    fn isolated_shaping_uses_the_same_family_as_the_measure_pass() {
        let mut r = test_renderer();
        let mut mono_style = TextStyle::default();
        mono_style.family = twcore::FontFamily::Mono;
        mono_style.size = 24.0;
        let mut sans_style = mono_style.clone();
        sans_style.family = twcore::FontFamily::Sans;

        // A proportional face puts "iiii" far narrower than "MMMM"; a monospace one
        // makes them identical. That difference is the observable proof the family
        // reached the shaper, and it needs no access to the private font handle.
        let width = |r: &mut Renderer, style: &TextStyle, text: &str| {
            let layout = r.shape_isolated(style, text);
            layout.width()
        };
        let mono_i = width(&mut r, &mono_style, "iiii");
        let mono_m = width(&mut r, &mono_style, "MMMM");
        let sans_i = width(&mut r, &sans_style, "iiii");
        let sans_m = width(&mut r, &sans_style, "MMMM");

        assert!(
            (mono_i - mono_m).abs() < 0.01,
            "font-mono must shape every glyph to the same advance: iiii={mono_i:.3} MMMM={mono_m:.3}"
        );
        assert!(
            sans_m > sans_i + 1.0,
            "font-sans must stay proportional: iiii={sans_i:.3} MMMM={sans_m:.3}"
        );
    }

    /// `tracking-*` must actually change advances, and it must be interpreted as em
    /// (twcore's unit, matching Tailwind) rather than passed to parley — which wants
    /// the same units as the font size — as a raw number. A unit mix-up here is
    /// nearly invisible at small sizes and only obvious at large ones.
    #[test]
    fn letter_spacing_widens_text_and_is_scaled_by_font_size() {
        let mut r = test_renderer();
        let mut base = TextStyle::default();
        base.size = 40.0;
        let mut wide = base.clone();
        wide.letter_spacing = 0.1; // em

        let plain = r.shape_isolated(&base, "iiiiiiiiii").width();
        let spaced = r.shape_isolated(&wide, "iiiiiiiiii").width();
        // 10 glyphs at 0.1em on a 40px font is ~40px of added advance; assert the
        // magnitude, not just "bigger", because a px/em mix-up would still be bigger
        // (by 10 x 0.1px) and would sail through a naive comparison.
        let added = spaced - plain;
        assert!(
            (added - 40.0).abs() < 6.0,
            "expected ~40px added from 10 glyphs at 0.1em on a 40px font, got {added:.2}"
        );
    }

    /// `tabular-nums` is in plan1.md §8's own reference app (`text-7xl font-bold
    /// text-white tabular-nums`) for a concrete reason: a counter that reshapes to a
    /// different width on every increment visibly jitters. This asserts the feature
    /// reaches the shaper, using the property that defines it — every digit takes the
    /// same advance, so two same-length digit strings measure identically.
    #[test]
    fn tabular_nums_makes_every_digit_the_same_width() {
        let mut r = test_renderer();
        let mut proportional = TextStyle::default();
        proportional.size = 48.0;
        let mut tabular = proportional.clone();
        tabular.tabular_nums = true;

        let (a, b) = ("111111", "000000");
        let tab_a = r.shape_isolated(&tabular, a).width();
        let tab_b = r.shape_isolated(&tabular, b).width();
        assert!(
            (tab_a - tab_b).abs() < 0.01,
            "tabular-nums must give every digit the same advance: {a}={tab_a:.3} {b}={tab_b:.3}"
        );
    }

    /// The shaped-text cache's central claim: it changes *nothing observable*. A
    /// renderer that has laid this tree out before must produce byte-identical
    /// geometry and glyphs to one seeing it for the first time.
    ///
    /// Compares a cold renderer against a warm one rather than a renderer against
    /// itself, because the failure mode worth catching is a key that under-specifies
    /// what parley consumed — and that only shows up as *stale* output, i.e. output
    /// that differs from what a cold shape would have produced. Asserting a warm
    /// renderer agrees with itself would pass with any key at all, including a
    /// constant one.
    ///
    /// The tree deliberately mixes size, weight, family, letter spacing, alignment
    /// and line height, and repeats one string at two different styles, so a key that
    /// dropped any of those fields would return the wrong entry here rather than in
    /// some app nobody runs on the host.
    #[test]
    fn shape_cache_is_observationally_transparent() {
        let build = |r: &mut Renderer| -> NodeId {
            r.clear();
            r.container("flex flex-col items-start w-[300px]", |r| {
                vec![
                    r.text("text-2xl font-bold", "Hello"),
                    r.text("text-sm", "Hello"),
                    r.text("font-serif leading-loose", "Hello"),
                    r.text("font-mono tracking-wide", "Hello"),
                    r.text("text-center w-[280px]", "aligned in a fixed box"),
                    r.text("w-[120px]", "a longer string that must wrap inside its box"),
                    r.text("tabular-nums", "1234567890"),
                ]
            })
        };
        // Deliberately excludes `NodeId`: two different `Renderer`s allocate from
        // their own slotmaps, so ids never match and comparing them would only ever
        // report that. `positioned` is depth-first paint order, which is stable, so
        // position in this vector identifies the node.
        let snapshot = |r: &Renderer, root: NodeId| -> Vec<((f32, f32, f32, f32), Vec<(u32, u32, u32)>)> {
            r.positioned(root, &Default::default())
                .iter()
                .map(|n| {
                    let glyphs = n
                        .text
                        .as_ref()
                        .map(|runs| {
                            runs.iter()
                                .flat_map(|run| {
                                    run.glyphs
                                        .iter()
                                        .map(|g| (g.id, g.x.to_bits(), g.y.to_bits()))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    (n.rect, glyphs)
                })
                .collect()
        };

        let mut cold = test_renderer();
        let cold_root = build(&mut cold);
        cold.layout(cold_root, 400.0, 600.0);
        let cold_snapshot = snapshot(&cold, cold_root);
        // Not `hits == 0`: taffy calls the measure closure more than once per node in
        // a single pass (min-content, max-content, then the definite size), so even a
        // brand-new `Renderer` serves some of its own first layout from cache. That
        // this renderer has never seen the tree is guaranteed by `Renderer::new`, not
        // by the hit count.
        assert!(
            cold.shape_cache_stats().misses > 0,
            "a fresh renderer's first layout must shape for real at least once"
        );

        // Warm: the same tree laid out repeatedly, so the second and later passes are
        // served from cache — including across a `clear()`, which is what every real
        // frame does.
        let mut warm = test_renderer();
        for _ in 0..3 {
            let root = build(&mut warm);
            warm.layout(root, 400.0, 600.0);
        }
        let before = warm.shape_cache_stats();
        let warm_root = build(&mut warm);
        warm.layout(warm_root, 400.0, 600.0);
        let after = warm.shape_cache_stats();
        assert_eq!(
            after.misses, before.misses,
            "the compared pass must be served entirely from cache, or this proves nothing"
        );
        assert!(after.hits > before.hits, "and must actually have read from it");

        assert_eq!(
            cold_snapshot,
            snapshot(&warm, warm_root),
            "cached shaping must reproduce cold shaping exactly — rects and every glyph id/x/y"
        );
    }

    /// The specific way a shaping cache goes wrong: the same string at two different
    /// styles collapsing onto one entry. Asserted through the real `text()`/`layout()`
    /// path with both leaves in one tree, so they share a cache generation and would
    /// genuinely collide if the style weren't in the key.
    ///
    /// Each pair below is a field that reaches parley, checked separately rather than
    /// as one combined tree, so a failure names which field fell out of the key.
    #[test]
    fn same_content_at_a_different_style_never_shares_a_cache_entry() {
        // Two class strings that must measure differently for the same content, each
        // isolating one text property.
        for (a, b, field) in [
            ("text-sm", "text-2xl", "font size"),
            ("font-normal", "font-bold", "font weight"),
            ("font-sans", "font-mono", "font family"),
            ("tracking-normal", "tracking-widest", "letter spacing"),
            ("leading-none", "leading-loose", "line height"),
        ] {
            let mut r = test_renderer();
            let (mut first, mut second) = (None, None);
            let root = r.container("flex flex-col items-start", |r| {
                first = Some(r.text(a, "Hello"));
                second = Some(r.text(b, "Hello"));
                vec![first.unwrap(), second.unwrap()]
            });
            r.layout(root, 400.0, 400.0);
            let positioned = r.positioned(root, &Default::default());
            let rect_of = |id: NodeId| positioned.iter().find(|n| n.node == id).unwrap().rect;
            let (a_rect, b_rect) = (rect_of(first.unwrap()), rect_of(second.unwrap()));
            assert_ne!(
                (a_rect.2, a_rect.3),
                (b_rect.2, b_rect.3),
                "{field}: {a:?} and {b:?} must not share a shaped result for identical content \
                 (got {a_rect:?} and {b_rect:?}) — the cache key is missing this field"
            );
        }
    }

    /// The cache must survive `clear()` and hit against a *rebuilt* tree, because
    /// that is the only situation a real frame ever presents: `../gpu_harness`'s
    /// `redraw` rebuilds from scratch every frame, so every `NodeId` is new. A cache
    /// keyed on `NodeId`, or one cleared by `clear()`, would miss 100% of the time
    /// while still passing every other test in this file.
    #[test]
    fn shape_cache_hits_across_a_rebuilt_tree_with_all_new_node_ids() {
        let mut r = test_renderer();
        let mut build_and_layout = |r: &mut Renderer| -> NodeId {
            r.clear();
            let root = r.container("flex flex-col items-start", |r| {
                (0..20).map(|i| r.text("text-white", format!("Item {i}"))).collect()
            });
            r.layout(root, 400.0, 800.0);
            root
        };

        let first_root = build_and_layout(&mut r);
        let after_first = r.shape_cache_stats();
        assert!(after_first.misses > 0, "the first pass must shape for real");

        let second_root = build_and_layout(&mut r);
        assert_ne!(first_root, second_root, "a rebuilt tree must genuinely allocate new node ids");
        let after_second = r.shape_cache_stats();
        assert_eq!(
            after_second.misses, after_first.misses,
            "an unchanged tree rebuilt from scratch must re-shape nothing at all"
        );
        assert!(after_second.hits > after_first.hits, "and must serve those measures from cache");
    }

    /// Memory, which is the half of a cache that isn't free. A list scrolled through
    /// thousands of distinct rows must not retain a glyph run per row ever seen —
    /// `twcore`'s own class-string caches already grow that way (../PLAN.md §11.7),
    /// and doing it again with glyphs would be a real leak on a 10,000-row list.
    ///
    /// Asserts the *bound*, not an exact number: two generations of a ~21-node window
    /// is ~42 entries, so anything near 200 distinct strings would mean rotation isn't
    /// evicting.
    #[test]
    fn shape_cache_working_set_is_bounded_by_the_window_not_by_strings_ever_seen() {
        let mut r = test_renderer();
        let window = 20;
        let total = 200;
        for first in 0..total {
            r.clear();
            let root = r.container("flex flex-col items-start", |r| {
                (first..first + window).map(|i| r.text("text-white", format!("Item {i}"))).collect()
            });
            r.layout(root, 400.0, 800.0);
        }
        let entries = r.shape_cache_stats().entries;
        assert!(
            entries <= 3 * window,
            "{total} distinct strings passed through a {window}-row window should retain about two \
             generations' worth ({}), not everything ever shaped — got {entries}",
            2 * window
        );
    }

    /// A real, non-obvious corollary of the fix above, found while re-running
    /// ../conformance/ after it landed: `Button`'s own fixture uses `font-medium`
    /// (weight 500), not `font-bold` — and with only two faces registered (400
    /// regular, 700 bold), weight 500 selects the *regular* face, not the bold
    /// one. This is not a bug in this crate's font matching: it's the CSS Fonts
    /// spec's own documented fallback rule (weight 500's fallback order checks 400
    /// before checking heavier weights), and empirically, the WebView backend's
    /// own default sans-serif on this host falls back the same way for the same
    /// reason (confirmed via `fc-match "sans-serif:weight=medium"` resolving to a
    /// Regular, not a Medium, face) — so this isn't a native-only quirk to route
    /// around. Locked in here so the conformance suite's own `button` fixture
    /// (`../conformance/fixtures.py`, `font-medium`) staying numerically unchanged
    /// by the bold-face fix is never mistaken for the fix not having worked — see
    /// ../PLAN.md §9.2 item 1's updated account.
    #[test]
    fn font_medium_falls_back_to_the_regular_face_not_bold_per_css_weight_fallback_rules() {
        let mut r = test_renderer();
        let mut normal = None;
        let mut medium = None;
        let mut bold = None;
        let root = r.container("flex flex-col items-start", |r| {
            normal = Some(r.text("font-normal", "Hello"));
            medium = Some(r.text("font-medium", "Hello"));
            bold = Some(r.text("font-bold", "Hello"));
            vec![normal.unwrap(), medium.unwrap(), bold.unwrap()]
        });
        r.layout(root, 400.0, 200.0);
        let positioned = r.positioned(root, &Default::default());
        let font_of = |id: NodeId| {
            positioned.iter().find(|n| n.node == id).unwrap().text.as_ref().unwrap()[0]
                .font
                .clone()
        };
        assert_eq!(
            font_of(normal.unwrap()),
            font_of(medium.unwrap()),
            "font-medium (500) should fall back to the regular (400) face, not the bold (700) one, with only two faces registered"
        );
        assert_ne!(
            font_of(medium.unwrap()),
            font_of(bold.unwrap()),
            "font-medium and font-bold must still select genuinely different faces"
        );
    }

    /// The generated sprite table is only as good as `kurbo`'s agreement with the
    /// generator, and a sprite that parses to garbage would paint as garbage on a
    /// device with nothing host-side to see it. This is the check that makes
    /// `resolve_sprite` safe to write as `filter_map` rather than a propagating error:
    /// every bundled shape parses, is non-empty, and lands inside its own viewBox.
    ///
    /// The bounds half specifically covers `gen_icons.py`'s circle -> two-arc
    /// conversion, which is the one piece of geometry the generator *derives* rather
    /// than copies — an arc with the wrong sweep or radius still parses fine and would
    /// silently draw a circle in the wrong place.
    #[test]
    fn icons_parse_and_stay_inside_their_viewbox() {
        use kurbo::Shape;
        assert!(!icons::SPRITES.is_empty(), "the bundled sprite set should not be empty");
        for sprite in icons::SPRITES {
            let resolved = resolve_sprite(sprite);
            assert_eq!(
                resolved.shapes.len(),
                sprite.shapes.len(),
                "every shape of `{}` should parse; a dropped one means kurbo and gen_icons.py disagree about SVG",
                sprite.name
            );
            for (i, shape) in resolved.shapes.iter().enumerate() {
                assert!(
                    !shape.path.is_empty(),
                    "`{}` shape {i} parsed to an empty path",
                    sprite.name
                );
                // A stroke straddles the path, so allow half a stroke width of
                // overhang — exactly what a browser lets a `viewBox`-relative stroke do.
                let slack = shape.stroke.map(|s| s.width as f64 / 2.0).unwrap_or(0.0);
                let bbox = shape.path.bounding_box();
                let (lo, hi) = (-slack, sprite.extent as f64 + slack);
                assert!(
                    bbox.x0 >= lo && bbox.y0 >= lo && bbox.x1 <= hi && bbox.y1 <= hi,
                    "`{}` shape {i} has bbox {bbox:?}, outside its 0..{} viewBox (slack {slack})",
                    sprite.name,
                    sprite.extent
                );
            }
        }
    }

    /// Two things at once, because they are the same claim from both sides: an `Icon`
    /// leaf carries geometry, and adding that geometry changes **no layout at all**.
    ///
    /// The second half is why this feature could be added without re-running the
    /// conformance suite's geometry expectations: `Icon._tree_classes` already reserves
    /// a definite box, so the sprite is drawn *into* space that was always reserved. A
    /// future change that made an icon contribute to its own sizing would fail here.
    #[test]
    fn an_icon_leaf_carries_geometry_and_occupies_the_same_box_as_before() {
        let rect_of = |name: &str| {
            let mut r = test_renderer();
            let mut icon = None;
            let root = r.container("flex flex-col items-start", |r| {
                icon = Some(r.icon("inline-block w-[20px] h-[20px] text-white", name));
                vec![icon.unwrap()]
            });
            r.layout(root, 400.0, 200.0);
            let positioned = r.positioned(root, &Default::default());
            let n = positioned.iter().find(|n| n.node == icon.unwrap()).unwrap();
            (n.rect, n.icon.is_some())
        };

        let (known_rect, known_has_geometry) = rect_of("star");
        let (unknown_rect, unknown_has_geometry) = rect_of("no-such-sprite");

        assert!(known_has_geometry, "a bundled sprite name should resolve to real geometry");
        assert!(
            !unknown_has_geometry,
            "an unknown sprite name should render as an empty box, not panic or draw something else"
        );
        assert_eq!(
            known_rect.2, 20.0,
            "the icon box comes from its classes, not from the sprite"
        );
        assert_eq!(
            known_rect, unknown_rect,
            "resolving a sprite must not change the node's box — the geometry is drawn into space layout already reserved"
        );
    }

    /// An `Icon` with no explicit size is `1em` — the *inherited* font size, resolved
    /// here because this is the only layer that can resolve it.
    ///
    /// This is what `Icon.implied_classes`' `w-[1em] h-[1em]` means on the WebView
    /// backend, and what `_tree_classes`' old hardcoded `w-[20px] h-[20px]` only
    /// approximated. Two sizes are checked, neither of them 20, because the conformance
    /// `icon` fixture happens to use `text-[20px]` and therefore agreed with the old
    /// hardcoded box by coincidence — exactly the "passes for a reason unrelated to what
    /// it claims to check" shape ../PLAN.md §12.5.3 found in the list fixture.
    #[test]
    fn an_unsized_icon_is_one_em_of_the_inherited_font_size() {
        let width_at = |text_class: &str, icon_class: &str| {
            let mut r = test_renderer();
            let mut icon = None;
            let root = r.container(&format!("flex flex-col items-start {text_class}"), |r| {
                icon = Some(r.icon(icon_class, "star"));
                vec![icon.unwrap()]
            });
            r.layout(root, 400.0, 200.0);
            let positioned = r.positioned(root, &Default::default());
            let rect = positioned.iter().find(|n| n.node == icon.unwrap()).unwrap().rect;
            (rect.2, rect.3)
        };

        assert_eq!(width_at("text-[32px]", "inline-block"), (32.0, 32.0));
        assert_eq!(
            width_at("text-[12px]", "inline-block"),
            (12.0, 12.0),
            "the icon must track the inherited size, not a constant that happens to match one case"
        );
        assert_eq!(
            width_at("text-[32px]", "inline-block w-[8px] h-[8px]"),
            (8.0, 8.0),
            "an explicit size from the caller must still win over the 1em default"
        );
    }

    /// The sprite cache is keyed by name and survives `clear()`, so a second render
    /// reuses the first render's parsed paths rather than re-parsing SVG per frame.
    /// Checked by pointer identity, because "it looks the same" is exactly what a
    /// re-parse would also produce — this is the icon equivalent of
    /// `shape_cache_hits_across_a_rebuilt_tree_with_all_new_node_ids`.
    #[test]
    fn icon_geometry_is_parsed_once_and_survives_a_rebuilt_tree() {
        let mut r = test_renderer();

        let geometry_after_a_render = |r: &mut Renderer| {
            r.clear();
            let mut icon = None;
            let root = r.container("flex flex-col", |r| {
                icon = Some(r.icon("w-[20px] h-[20px]", "star"));
                vec![icon.unwrap()]
            });
            r.layout(root, 400.0, 200.0);
            let positioned = r.positioned(root, &Default::default());
            positioned.iter().find(|n| n.node == icon.unwrap()).unwrap().icon.clone().unwrap()
        };

        let first = geometry_after_a_render(&mut r);
        let second = geometry_after_a_render(&mut r);
        assert!(
            Arc::ptr_eq(&first, &second),
            "the second render should reuse the first render's parsed paths, not re-parse them"
        );
    }

    // ---- slider geometry (../PLAN.md §26) ------------------------------------------

    /// A 200dp-wide slider at x=50, the shape `Slider` actually lays out to.
    const SL: (f32, f32, f32, f32) = (50.0, 300.0, 200.0, 16.0);

    #[test]
    fn slider_value_tracks_the_touch_across_the_track() {
        // Ends and midpoint, against the node's own left edge — not against 0, which
        // is the mistake a rect-relative/absolute mix-up would make and which would
        // still look plausible for a slider that happens to start at x=0.
        assert_eq!(slider_value_at(SL, 50.0, 0.0, 100.0, 0.0), 0.0);
        assert_eq!(slider_value_at(SL, 150.0, 0.0, 100.0, 0.0), 50.0);
        assert_eq!(slider_value_at(SL, 250.0, 0.0, 100.0, 0.0), 100.0);
    }

    #[test]
    fn slider_value_clamps_outside_the_track_rather_than_extrapolating() {
        // A drag continues past the edge constantly — a finger does not stop at the
        // rect. Extrapolating here would push the store past `max`.
        assert_eq!(slider_value_at(SL, -1000.0, 0.0, 100.0, 0.0), 0.0);
        assert_eq!(slider_value_at(SL, 1000.0, 0.0, 100.0, 0.0), 100.0);
    }

    #[test]
    fn slider_step_quantises_from_min_not_from_zero() {
        // `<input type=range min=5 step=10>` yields 5, 15, 25 — never 10. Quantising
        // from zero would pass a `min=0` test and be wrong for every other min.
        let v = slider_value_at(SL, 150.0, 5.0, 105.0, 10.0);
        assert_eq!(v, 55.0, "midpoint of 5..105 quantised from min");
        for x in [50.0, 90.0, 150.0, 210.0, 250.0] {
            let v = slider_value_at(SL, x, 5.0, 105.0, 10.0);
            assert_eq!((v - 5.0) % 10.0, 0.0, "value {v} at x={x} is not on the step grid from min");
        }
    }

    #[test]
    fn slider_respects_a_non_zero_min_and_a_negative_range() {
        assert_eq!(slider_value_at(SL, 50.0, -50.0, 50.0, 0.0), -50.0);
        assert_eq!(slider_value_at(SL, 150.0, -50.0, 50.0, 0.0), 0.0);
        assert_eq!(slider_value_at(SL, 250.0, -50.0, 50.0, 0.0), 50.0);
    }

    #[test]
    fn slider_degenerate_inputs_yield_min_and_never_a_nan() {
        // All three are reachable from ordinary user code — `Slider("w-0")`,
        // `Slider(min=1, max=1)`, a slider laid out before its first real layout —
        // and a NaN here would reach a `@store` through `on_change` and render as
        // "NaN" forever after, with nothing pointing back at a division.
        for (rect, min, max) in [
            ((50.0f32, 0.0f32, 0.0f32, 16.0f32), 0.0f32, 100.0f32),
            (SL, 1.0, 1.0),
            (SL, 100.0, 0.0),
        ] {
            let v = slider_value_at(rect, 123.0, min, max, 1.0);
            assert!(v.is_finite(), "expected a finite value, got {v}");
            assert_eq!(v, min);
        }
    }

    #[test]
    fn slider_never_returns_a_value_outside_min_max_even_when_step_overshoots() {
        // A step that does not divide the range evenly rounds the top end *past*
        // `max` before clamping — `max=100 step=30` rounds 100 to 90 or 120.
        for x in [50.0, 100.0, 150.0, 200.0, 250.0] {
            let v = slider_value_at(SL, x, 0.0, 100.0, 30.0);
            assert!((0.0..=100.0).contains(&v), "value {v} at x={x} escaped 0..=100");
        }
    }

    // ------------------------------------------------------- §45 transform / subtree
    //
    // Both of these exist because `ComputedStyle` has carried a `transform` and a
    // `paint.opacity` since phase 0 and **no native paint code has ever read either**
    // — parsed, stored, dropped, while the WebView backend compiled the same class
    // strings with real Tailwind and painted them. Measured against real Chromium
    // before the fix: `opacity` 42.18% of pixels wrong, `opacity_group` 78.17%,
    // `translate` 15.19% (../PLAN.md §45.1).

    /// A translate moves the node **and its subtree**, and moves nothing else.
    ///
    /// The three claims are asserted together because an implementation can satisfy
    /// any two: offsetting the rect at paint time moves the node without its subtree;
    /// feeding the offset back into taffy moves the node *and* displaces its
    /// siblings, which is a margin, not a transform.
    #[test]
    fn a_translate_moves_the_node_and_its_subtree_and_displaces_nothing() {
        let mut r = test_renderer();
        let child = r.container("w-[20px] h-[20px]", |_| vec![]);
        let moved = r.container("w-[60px] h-[30px] translate-x-8 -translate-y-2", |_| vec![child]);
        let sibling = r.container("w-[60px] h-[30px]", |_| vec![]);
        let root = r.container("flex flex-col w-[200px] h-[200px]", |_| vec![moved, sibling]);
        r.layout(root, 200.0, 200.0);
        let nodes = r.positioned(root, &FxHashMap::default());
        let at = |n: NodeId| nodes.iter().find(|p| p.node == n).unwrap().rect;

        // `translate-x-8` is 2rem = 32px and `-translate-y-2` is -0.5rem = -8px on
        // Tailwind's spacing scale — pinned independently by twcore's own suite, so
        // this is checking the painting, not the parse.
        assert_eq!(at(moved), (32.0, -8.0, 60.0, 30.0));
        // The child sits at its parent's *transformed* origin: it rode along.
        assert_eq!(at(child), (32.0, -8.0, 20.0, 20.0));
        // …and the sibling is exactly where it would be with no transform anywhere,
        // i.e. immediately below the untransformed 30px-tall box.
        assert_eq!(at(sibling), (0.0, 30.0, 60.0, 30.0));
    }

    /// The half a paint-time offset would get wrong invisibly: a translated node must
    /// be *tapped* where it is *drawn*. §9.7/§13 record this exact failure shape —
    /// a coordinate-space slip that no screenshot can see and only a finger finds.
    #[test]
    fn a_translated_node_is_hit_tested_where_it_is_painted() {
        let mut r = test_renderer();
        let moved = r.container("w-[40px] h-[20px] translate-x-8", |_| vec![]);
        let root = r.container("flex flex-col w-[200px] h-[200px]", |_| vec![moved]);
        r.layout(root, 200.0, 200.0);
        let offsets = FxHashMap::default();
        // Inside the drawn box.
        assert_eq!(r.hit_test(root, 40.0, 10.0, &offsets), Some(moved));
        // Where it *would* have been without the transform — the root, not the node.
        assert_eq!(r.hit_test(root, 10.0, 10.0, &offsets), Some(root));
        // The control: with no transform, that same point hits the node.
        let mut r2 = test_renderer();
        let still = r2.container("w-[40px] h-[20px]", |_| vec![]);
        let root2 = r2.container("flex flex-col w-[200px] h-[200px]", |_| vec![still]);
        r2.layout(root2, 200.0, 200.0);
        assert_eq!(r2.hit_test(root2, 10.0, 10.0, &offsets), Some(still));
    }

    // ------------------------------------------------------------- §47 `animate`
    //
    // The driver overrides a resolved style for one frame. These pin the two things
    // that make that safe: it reaches everything that reads the style, and it reaches
    // nothing else.

    /// An override must move the node the driver named **and nothing else** — the
    /// same three-way claim `a_translate_moves_the_node_and_its_subtree_and_displaces_
    /// nothing` makes about a class-declared transform, now for a driven one, because
    /// the two go through different code and only one of them was ever tested.
    #[test]
    fn an_animated_translate_moves_the_node_and_its_subtree_and_no_sibling() {
        let mut r = test_renderer();
        let child = r.container("w-[20px] h-[20px]", |_| vec![]);
        let moved = r.container("w-[60px] h-[30px]", |_| vec![child]);
        let sibling = r.container("w-[60px] h-[30px]", |_| vec![]);
        let root = r.container("flex flex-col w-[200px] h-[200px]", |_| vec![moved, sibling]);
        r.animate(moved, Channel::TranslateX, 32.0);
        r.animate(moved, Channel::TranslateY, -8.0);
        r.layout(root, 200.0, 200.0);
        let nodes = r.positioned(root, &FxHashMap::default());
        let at = |n: NodeId| nodes.iter().find(|p| p.node == n).unwrap().rect;

        // Exactly the numbers the class-string test asserts for `translate-x-8
        // -translate-y-2`, reached without either class — so the driven path and the
        // declared path land in the same place rather than merely both "working".
        assert_eq!(at(moved), (32.0, -8.0, 60.0, 30.0));
        assert_eq!(at(child), (32.0, -8.0, 20.0, 20.0), "the subtree must ride along");
        assert_eq!(at(sibling), (0.0, 30.0, 60.0, 30.0), "layout must be untouched");
        // …and it is tapped where it is drawn, for free, because `hit_test` resolves
        // against `positioned()`. This is the half a paint-time offset gets wrong
        // invisibly (§9.7/§13). The probe is (60, 15): inside `moved`'s translated box
        // (32..92, -8..22) and outside `child`'s (32..52, -8..12), because a point in
        // both correctly resolves to the child and would say nothing about the parent.
        let offsets = FxHashMap::default();
        assert_eq!(r.hit_test(root, 60.0, 15.0, &offsets), Some(moved));
        // The control: the untranslated position now belongs to nobody but the root.
        assert_eq!(r.hit_test(root, 10.0, 15.0, &offsets), Some(root));
    }

    /// **An override replaces the class-resolved value; it does not add to it.**
    ///
    /// A real decision, not an implementation detail. CSS `transform` and `opacity`
    /// are single properties, so a node carrying `translate-x-8` that is also being
    /// driven has one value, not two composed — and the driver's `from` is read off
    /// the class value (`channel_value`) precisely so that "animate from where the
    /// class puts it" is expressible without composition. Adding instead would make
    /// an animation to 0 land at 32px, which reads as "the animation didn't run".
    #[test]
    fn an_override_replaces_the_class_value_rather_than_composing_with_it() {
        let mut r = test_renderer();
        let node = r.container("w-[40px] h-[20px] translate-x-8 opacity-50", |_| vec![]);
        let root = r.container("flex flex-col w-[200px] h-[200px]", |_| vec![node]);
        // What the class alone says, which is also what an animation would use as its
        // default `from`.
        assert_eq!(r.channel_value(node, Channel::TranslateX), Some(32.0));
        assert_eq!(r.channel_value(node, Channel::Opacity), Some(0.5));

        r.animate(node, Channel::TranslateX, 0.0);
        r.animate(node, Channel::Opacity, 1.0);
        r.layout(root, 200.0, 200.0);
        let nodes = r.positioned(root, &FxHashMap::default());
        let pn = nodes.iter().find(|p| p.node == node).unwrap();
        assert_eq!(pn.rect.0, 0.0, "override must win outright, not add to 32px");
        assert_eq!(pn.style.paint.opacity, 1.0);
    }

    /// The negative control for the pair above: overriding one node must not touch a
    /// second node that resolved from the *same class string*.
    ///
    /// This is the one that rejects the obvious cheap implementation — mutating a
    /// shared `Arc<ComputedStyle>` in place without `make_mut`. `resolve_inherited`
    /// hands out a fresh `Arc` per node today, so a naive version passes; it would
    /// start fading unrelated nodes the moment that became a cache, and it would do
    /// it only for nodes that happened to share a class string.
    #[test]
    fn animating_one_node_leaves_an_identically_classed_sibling_alone() {
        let mut r = test_renderer();
        let driven = r.container("w-[40px] h-[20px] opacity-50", |_| vec![]);
        let twin = r.container("w-[40px] h-[20px] opacity-50", |_| vec![]);
        let root = r.container("flex flex-col w-[200px] h-[200px]", |_| vec![driven, twin]);
        r.animate(driven, Channel::Opacity, 0.125);
        r.layout(root, 200.0, 200.0);
        let nodes = r.positioned(root, &FxHashMap::default());
        let alpha = |n: NodeId| nodes.iter().find(|p| p.node == n).unwrap().style.paint.opacity;
        assert_eq!(alpha(driven), 0.125);
        assert_eq!(alpha(twin), 0.5, "an identically-classed sibling was dragged along");
        assert_eq!(alpha(root), 1.0);
    }

    /// A node that is gone, or was never a container, is ignored rather than fatal.
    ///
    /// The driver samples a timeline keyed by `zkite_py` node id and applies against a
    /// tree Python may have rebuilt in between, so "this id is not in this tree" is an
    /// ordinary race on the happy path. A panic here is a dead render thread, which
    /// §4's own panic hook exists because it is otherwise *silent*.
    #[test]
    fn animating_a_node_that_is_not_a_container_is_a_no_op() {
        let mut r = test_renderer();
        let leaf = r.text("text-base", "hello");
        let root = r.container("flex flex-col w-[100px] h-[100px]", |_| vec![leaf]);
        // A text leaf carries a `TextContext`, not a `container_styles` entry.
        r.animate(leaf, Channel::Opacity, 0.0);
        assert_eq!(r.channel_value(leaf, Channel::Opacity), None);
        r.layout(root, 100.0, 100.0);
        let nodes = r.positioned(root, &FxHashMap::default());
        assert_eq!(nodes.iter().find(|p| p.node == leaf).unwrap().style.paint.opacity, 1.0);
        // A stale id from a previous tree: `clear()` drops every style, and applying
        // against the emptied renderer must not panic.
        r.clear();
        r.animate(root, Channel::TranslateY, 99.0);
        assert_eq!(r.channel_value(root, Channel::TranslateY), None);
    }

    /// Every channel round-trips its wire name, and an unknown one is rejected rather
    /// than silently mapped. Enumerated from `Channel::all()` so adding a fourth
    /// channel without a name fails here instead of in an app.
    #[test]
    fn every_channel_round_trips_its_wire_name_and_unknown_ones_are_rejected() {
        for ch in Channel::all() {
            assert_eq!(Channel::parse(ch.name()), Some(ch), "{} did not round-trip", ch.name());
        }
        // `rotate`/`scale` parse in `twcore` and paint in nothing (§45.7), so they must
        // not be animatable — an animation that runs, completes and fires its callback
        // without moving a pixel is the §15.1 divergence shape exactly.
        for unknown in ["rotate", "scale", "translate", "opacity ", "Opacity", ""] {
            assert_eq!(Channel::parse(unknown), None, "{unknown:?} was accepted");
        }
        assert!(Channel::Opacity.opens_a_layer(), "§46.6: an opacity group costs 1.88ms/frame");
        assert!(!Channel::TranslateX.opens_a_layer());
        assert!(!Channel::TranslateY.opens_a_layer());
    }

    /// `subtree_len` must span exactly the subtree — the range a painter pushes an
    /// opacity layer over. One too few leaves the last descendant outside the group;
    /// one too many pops the layer after an unrelated sibling has been painted into
    /// it, which fades a node that declared no opacity at all.
    #[test]
    fn subtree_len_spans_exactly_the_subtree() {
        let mut r = test_renderer();
        let leaf_a = r.container("w-[10px] h-[10px]", |_| vec![]);
        let leaf_b = r.container("w-[10px] h-[10px]", |_| vec![]);
        let inner = r.container("w-[20px] h-[20px]", |_| vec![leaf_a, leaf_b]);
        let group = r.container("w-[40px] h-[40px]", |_| vec![inner]);
        let after = r.container("w-[10px] h-[10px]", |_| vec![]);
        let root = r.container("flex flex-col w-[100px] h-[100px]", |_| vec![group, after]);
        r.layout(root, 100.0, 100.0);
        let nodes = r.positioned(root, &FxHashMap::default());

        let idx = |n: NodeId| nodes.iter().position(|p| p.node == n).unwrap();
        let len = |n: NodeId| nodes[idx(n)].subtree_len;

        assert_eq!(nodes.len(), 6);
        assert_eq!(len(root), 6, "the root spans everything");
        assert_eq!(len(group), 4, "group + inner + two leaves");
        assert_eq!(len(inner), 3);
        assert_eq!(len(leaf_a), 1, "a leaf is exactly itself");
        assert_eq!(len(after), 1);

        // The property a painter actually depends on, stated directly: the half-open
        // range is contiguous and `after` is outside `group`'s.
        let (g, a) = (idx(group), idx(after));
        assert!(a >= g + len(group), "the next sibling fell inside the group's range");
        for n in [inner, leaf_a, leaf_b] {
            assert!((g..g + len(group)).contains(&idx(n)), "{n:?} should be inside the group");
        }
    }

    /// The whole of `OpacityGroups`' contract, driven the way a painter drives it and
    /// checked as a *balanced sequence*: every push is matched, the nesting order is
    /// right, and each layer covers exactly the nodes it should.
    ///
    /// Simulated rather than painted, because none of the three real painters builds
    /// for the host with a scene this test could inspect — the same constraint §15.5
    /// records and the reason this arithmetic was worth extracting in the first place.
    #[test]
    fn opacity_groups_open_and_close_over_exactly_their_own_subtrees() {
        let mut r = test_renderer();
        let inner_leaf = r.container("w-[10px] h-[10px] bg-white", |_| vec![]);
        let faded_inner = r.container("w-[20px] h-[20px] opacity-25", |_| vec![inner_leaf]);
        let plain_leaf = r.container("w-[10px] h-[10px] bg-red-500", |_| vec![]);
        let faded = r.container("w-[40px] h-[40px] opacity-50", |_| vec![faded_inner, plain_leaf]);
        let after = r.container("w-[10px] h-[10px] bg-blue-500", |_| vec![]);
        let root = r.container("flex flex-col w-[100px] h-[100px]", |_| vec![faded, after]);
        r.layout(root, 100.0, 100.0);
        let nodes = r.positioned(root, &FxHashMap::default());

        // Replay the documented usage, recording what a painter would have done.
        let mut groups = OpacityGroups::new();
        let mut depth = 0usize;
        let mut depth_at: Vec<usize> = Vec::new();
        let mut opened: Vec<(usize, f32)> = Vec::new();
        for (i, n) in nodes.iter().enumerate() {
            depth -= groups.close_before(i);
            if let Some(alpha) = groups.open_at(i, n) {
                opened.push((i, alpha));
                depth += 1;
            }
            depth_at.push(depth);
        }
        depth -= groups.close_all();
        assert_eq!(depth, 0, "a painter would have left {depth} layers open");

        let idx = |n: NodeId| nodes.iter().position(|p| p.node == n).unwrap();
        assert_eq!(
            opened.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![idx(faded), idx(faded_inner)],
            "exactly the two nodes that declared an opacity open a layer"
        );
        assert!((opened[0].1 - 0.5).abs() < 1e-6 && (opened[1].1 - 0.25).abs() < 1e-6);

        // One layer over `faded` and `plain_leaf`, two over the nested pair, none
        // over the root or the sibling that follows the group.
        assert_eq!(depth_at[idx(root)], 0);
        assert_eq!(depth_at[idx(faded)], 1);
        assert_eq!(depth_at[idx(faded_inner)], 2);
        assert_eq!(depth_at[idx(inner_leaf)], 2);
        assert_eq!(depth_at[idx(plain_leaf)], 1, "the nested group closed before its uncle");
        assert_eq!(depth_at[idx(after)], 0, "the group must not still be open here");
    }

    /// The negative control for the above: with no `opacity-*` anywhere, a painter
    /// makes no layer calls at all. Without this, an `open_at` that opened a layer for
    /// every node would pass every assertion in the previous test that is about
    /// *relative* depth, while wrapping every node in the tree in its own offscreen
    /// target.
    #[test]
    fn a_tree_with_no_opacity_opens_no_layers_at_all() {
        let mut r = test_renderer();
        let leaf = r.container("w-[10px] h-[10px] bg-white", |_| vec![]);
        let mid = r.container("w-[20px] h-[20px] bg-slate-800", |_| vec![leaf]);
        let root = r.container("flex flex-col w-[100px] h-[100px]", |_| vec![mid]);
        r.layout(root, 100.0, 100.0);
        let nodes = r.positioned(root, &FxHashMap::default());

        let mut groups = OpacityGroups::new();
        let mut pushes = 0;
        let mut pops = 0;
        for (i, n) in nodes.iter().enumerate() {
            pops += groups.close_before(i);
            if groups.open_at(i, n).is_some() {
                pushes += 1;
            }
        }
        pops += groups.close_all();
        assert_eq!((pushes, pops), (0, 0), "an opacity-free tree cost {pushes} layers");
    }

    // ---------------------------------------------------------------- §28 decoration
    //
    // Deliberately driven through **real class strings and a real `Renderer`** rather
    // than a hand-built `ComputedStyle`: §27.6's lesson one layer down — a test built
    // on a shape the framework does not actually produce cannot fail for the right
    // reason. `resolve_style` is how a real node's style is obtained.

    use kurbo::Shape as _; // `bounding_box` on a BezPath

    /// The node rect every decoration test below uses. Deliberately non-square so a
    /// transposed x/y would show up rather than cancel.
    const DECO_RECT: (f32, f32, f32, f32) = (10.0, 20.0, 100.0, 60.0);

    fn deco(classes: &str) -> decoration::BoxDecoration {
        deco_with_color(classes, None)
    }

    fn deco_with_color(classes: &str, current: Option<Color>) -> decoration::BoxDecoration {
        let mut r = test_renderer();
        let style = r.resolver.resolve(classes, NodeState::default());
        decoration::box_decoration(DECO_RECT, &style, current)
    }

    #[test]
    fn an_undecorated_node_produces_no_decoration_at_all() {
        // The early-out that keeps this free for the overwhelming majority of nodes in
        // a real tree. If this ever regresses, every frame allocates paths for every
        // plain `div` — the class of mistake §11.9 spent a section removing from text.
        let d = deco("bg-slate-800 rounded-lg p-4 flex flex-col");
        assert!(d.is_empty(), "a node with no border/shadow/ring must build no geometry");
        assert!(d.shadows.is_empty() && d.ring.is_none() && d.border.is_none());
    }

    #[test]
    fn a_css_blur_radius_is_half_a_gaussian_std_dev() {
        // The factor-of-two trap this module's doc names. Passing CSS's blur straight
        // into `fill_blurred_rounded_rect` renders every shadow twice as soft as
        // Chromium's — which looks deliberate, so nothing would report it.
        assert_eq!(decoration::blur_to_std_dev(0.0), 0.0);
        assert_eq!(decoration::blur_to_std_dev(6.0), 3.0);
        assert_eq!(decoration::blur_to_std_dev(15.0), 7.5);

        // And it is actually applied, not merely available: `shadow-lg`'s first layer
        // is `0 10px 15px -3px`, so std_dev must be 7.5 and not 15.
        let d = deco("shadow-lg");
        assert_eq!(d.shadows[0].std_dev, 7.5);
    }

    #[test]
    fn a_multi_layer_shadow_keeps_every_layer_in_css_order() {
        // v4's `shadow-md` is two layers; a one-layer render is a different shadow.
        // The order matters because `box-shadow` paints the *first* layer on top, so a
        // painter must walk this in reverse — the ordering is part of the contract and
        // is asserted here rather than left to a comment.
        let d = deco("shadow-md");
        assert_eq!(d.shadows.len(), 2, "shadow-md is two layers in Tailwind v4");

        // Layer 0: `0 4px 6px -1px`. Offset moves the rect down 4; spread -1 shrinks it
        // by 1 on every side. So against a 100x60 box at (10,20): x0 = 10-(-1) = 11.
        let a = &d.shadows[0];
        assert_eq!((a.rect.x0, a.rect.y0), (11.0, 25.0));
        assert_eq!((a.rect.width(), a.rect.height()), (98.0, 58.0));
        assert_eq!(a.std_dev, 3.0);

        // Layer 1: `0 2px 4px -2px` — a *different* offset and spread, which is the
        // whole reason one layer is not an approximation of two.
        let b = &d.shadows[1];
        assert_eq!((b.rect.x0, b.rect.y0), (12.0, 24.0));
        assert_eq!((b.rect.width(), b.rect.height()), (96.0, 56.0));
        assert_eq!(b.std_dev, 2.0);
    }

    #[test]
    fn shadow_spread_moves_the_corner_radius_with_the_edge() {
        // Concentric rounded rects stay concentric only if the radius tracks the
        // inset. A spread that grew the box but left the radius alone would round the
        // shadow more tightly than the box it belongs to — subtly, at every corner.
        let d = deco("rounded-lg shadow-xl"); // rounded-lg is 8dp; xl spreads -5 then -6
        assert_eq!(d.shadows[0].radius, 3.0, "8 + (-5)");
        assert_eq!(d.shadows[1].radius, 2.0, "8 + (-6)");

        // And it never goes negative: a spread larger than the radius clamps at square.
        let d = deco("rounded-sm shadow-2xl"); // rounded-sm 4dp (v4, §36), spread -12
        assert_eq!(d.shadows[0].radius, 0.0);
    }

    #[test]
    fn a_uniform_border_strokes_a_centreline_so_its_outer_edge_is_the_border_box() {
        // This is the property that makes the uniform fast path *exact* rather than
        // merely cheap: taffy reserves the border inside the node's rect, so CSS draws
        // the band from the rect inward. A stroke of width w centred w/2 inside has its
        // outer edge exactly on the rect.
        let d = deco("border-4 border-red-500");
        let Some(decoration::Border::Uniform(o)) = &d.border else {
            panic!("uniform width + uniform colour must take the single-stroke path");
        };
        assert_eq!(o.width, 4.0);
        let bb = o.path.bounding_box();
        // Centreline is inset by half the width on every side.
        assert!((bb.x0 - 12.0).abs() < 1e-6, "centreline x0 {} != 12", bb.x0);
        assert!((bb.y0 - 22.0).abs() < 1e-6, "centreline y0 {} != 22", bb.y0);
        // Stroked at width 4, that puts the outer edge back on the border box at x=10.
        assert!((bb.x0 - o.width / 2.0 - 10.0).abs() < 1e-6);
        assert!((bb.x1 + o.width / 2.0 - 110.0).abs() < 1e-6);
    }

    #[test]
    fn differing_widths_or_colours_fall_out_of_the_uniform_path_into_real_side_geometry() {
        // §18.6's correction, asserted: Tailwind allows per-side widths and colours and
        // `twcore` models both, so the general case is four mitred regions rather than
        // one stroke. Two independent triggers, checked separately so a failure names
        // which one stopped working.
        let by_width = deco("border-t-4 border-b-2 border-red-500");
        assert!(
            matches!(by_width.border, Some(decoration::Border::PerSide(_))),
            "differing widths must not take the uniform stroke"
        );

        let by_colour = deco("border-2 border-red-500 border-t-blue-500");
        let Some(decoration::Border::PerSide(sides)) = &by_colour.border else {
            panic!("differing colours must not take the uniform stroke");
        };
        assert_eq!(sides.len(), 4, "all four sides have a width and a colour here");
        // Top is blue-500, the rest red-500 — i.e. the per-side colour actually reaches
        // the geometry, rather than one last-wins colour being applied to everything.
        assert_eq!(sides[0].color.to_rgba_u32(), 0x2b7fffff, "top must be blue-500");
        assert!(
            sides[1..].iter().all(|s| s.color.to_rgba_u32() == 0xfb2c36ff),
            "right/bottom/left must all be red-500"
        );
    }

    #[test]
    fn a_side_with_a_width_but_no_paintable_colour_is_skipped_not_drawn_black() {
        // `border-t-4` alone seeds only the top side's colour (to currentColor). The
        // other three have zero width, so a per-side build must emit exactly one side —
        // emitting four would paint three hairlines that CSS does not draw.
        let d = deco("border-t-4");
        let Some(decoration::Border::PerSide(sides)) = &d.border else {
            panic!("one side with width is not the uniform case");
        };
        assert_eq!(sides.len(), 1);
    }

    #[test]
    fn border_and_ring_default_to_the_nodes_own_resolved_text_colour() {
        // Tailwind v4's default for both is `currentColor` (§28.1, measured in real
        // Chromium). Getting this wrong paints every undecorated border black on a
        // light-text app, which reads as "borders are broken" rather than as a colour
        // bug.
        let white = Color { r: 255, g: 255, b: 255, a: 255 };

        let d = deco_with_color("border-2", Some(white));
        let Some(decoration::Border::Uniform(o)) = &d.border else { panic!("uniform") };
        assert_eq!(o.color, white, "a bare border takes the inherited text colour");

        let d = deco_with_color("ring-2", Some(white));
        assert_eq!(d.ring.as_ref().unwrap().color, white);

        // With nothing inherited it falls back to black, which is what Chromium
        // computes for `border-2` on a page with no `color` set.
        let d = deco_with_color("border-2", None);
        let Some(decoration::Border::Uniform(o)) = &d.border else { panic!("uniform") };
        assert_eq!(o.color.to_rgba_u32(), 0x000000ff);

        // An explicit colour still wins over the inherited one.
        let d = deco_with_color("border-2 border-red-500", Some(white));
        let Some(decoration::Border::Uniform(o)) = &d.border else { panic!("uniform") };
        assert_eq!(o.color.to_rgba_u32(), 0xfb2c36ff);
    }

    #[test]
    fn a_ring_sits_entirely_outside_the_border_box() {
        // The property that makes a ring different from a border and that nothing else
        // in this renderer has: it paints outside its own node's rect, so it is not
        // bounded by the node and only an ancestor's clip stops it escaping.
        let d = deco("ring-4");
        let r = d.ring.as_ref().unwrap();
        assert_eq!(r.width, 4.0);
        let bb = r.path.bounding_box();
        // Centreline is *outside* the border box by half the width.
        assert!((bb.x0 - 8.0).abs() < 1e-6, "ring centreline x0 {} != 8", bb.x0);
        // Inner edge lands exactly on the border box; outer edge is 4dp beyond it.
        assert!((bb.x0 + r.width / 2.0 - 10.0).abs() < 1e-6, "inner edge must touch the box");
        assert!((bb.x0 - r.width / 2.0 - 6.0).abs() < 1e-6, "outer edge must clear it by 4");
    }

    #[test]
    fn a_rounded_full_box_clamps_its_radius_instead_of_building_a_garbled_path() {
        // `rounded-full` is 9999dp. Unclamped, the corner arcs are larger than the box
        // and `kurbo` builds a shape that paints as visible garbage rather than
        // failing. The clamp is half the shorter side, which for a 100x60 box is 30.
        let d = deco("rounded-full border-2 border-red-500");
        let Some(decoration::Border::Uniform(o)) = &d.border else { panic!("uniform") };
        let bb = o.path.bounding_box();
        // A sane path stays inside its own centreline rect.
        assert!(bb.width() <= 99.0 && bb.height() <= 59.0, "path {bb:?} escaped its box");
        assert!(bb.width() > 0.0 && bb.height() > 0.0);

        // And the shadow radius clamps the same way rather than reporting 9999.
        let d = deco("rounded-full shadow-md");
        assert!(d.shadows[0].radius <= 30.0, "radius {} unclamped", d.shadows[0].radius);
    }

    #[test]
    fn a_border_wider_than_the_box_does_not_produce_an_inverted_rect() {
        // Reachable from ordinary user code (`Column("border-[80px]", ...)` inside a
        // narrow parent). An inverted rect is normalised by the paint primitives into a
        // visible artefact, so it is refused here instead.
        let mut r = test_renderer();
        let style = r.resolver.resolve("border-[80px] border-red-500", NodeState::default());
        let d = decoration::box_decoration((0.0, 0.0, 20.0, 20.0), &style, None);
        assert!(d.border.is_none(), "a border that cannot fit must draw nothing");
    }
}
