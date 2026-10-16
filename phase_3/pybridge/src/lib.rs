//! Phase 3 — bridges `zkite_py`'s native tree export (`zkite.app.App.render_tree`,
//! `phase_1/zkite_py/src/zkite/widgets.py`'s `Widget.to_tree`) into
//! `zkite_renderer::Renderer`. See ../PLAN.md §5.
//!
//! Two halves: `embed` owns the CPython interpreter and calls `runtime.py`'s
//! `_zkite_setup_tree`/`_zkite_dispatch_tree`, returning JSON; this file deserializes
//! that JSON and walks it into `zkite_renderer::Renderer::text`/`container` calls,
//! tracking which taffy `NodeId` corresponds to which `zkite_py` node id (and which
//! events that id has a handler for) so a caller can turn a `hit_test` result back
//! into something dispatchable — the same target-vs-currentTarget ancestor walk
//! `zkite_renderer::Renderer::parent`'s doc comment already describes, just driven by
//! a real app's event map instead of one hardcoded button.

mod embed;
pub use embed::{
    back, dispatch, drain_log, framework_fingerprint, last_error, reload, scroll, set_frame_sink, setup,
};

pub mod anim;
pub use anim::{should_start, AnimSpec, SpecCurve, Trigger};

use std::sync::Arc;

use zkite_renderer::{NodeId, Renderer};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Deserialize;

/// One node of the tree `zkite_py`'s `Widget.to_tree` produces, deserialized straight
/// from its JSON. One flat struct with defaults covers both shapes that JSON actually
/// contains — every HTML-ish tag (`div`/`span`/`button`/`input`/`svg`) shares the same
/// fields, and only `#text` differs (`tag`+`text` alone) — rather than a tagged enum
/// per HTML tag, since there's no closed set of tags to enumerate up front (a user's
/// own `Widget` subclass can set `tag` to anything).
#[derive(Deserialize, Debug)]
pub struct RawNode {
    pub tag: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub classes: String,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub attrs: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub children: Vec<RawNode>,
    #[serde(default)]
    pub text: Option<String>,
}

impl RawNode {
    pub fn from_json(json: &str) -> Result<RawNode, String> {
        serde_json::from_str(json).map_err(|e| format!("malformed tree JSON: {e}"))
    }
}

/// A `TextField` currently owned by the native IME integration (../PLAN.md §5) —
/// while focused, its on-screen text comes from here (the live GameTextInput state),
/// not from `RawNode.attrs["value"]`, the same "uncontrolled while editing" reasoning
/// a plain HTML `<input>` already has relative to whatever a re-render's `value` prop
/// says. See `build`'s doc comment for why this has to be threaded through the whole
/// walk rather than patched in afterwards.
pub struct FocusedField<'a> {
    pub zkite_id: &'a str,
    pub text: &'a str,
}

/// A widget whose *value* a touch changes directly, rather than one that merely has
/// a handler attached (../PLAN.md §26).
///
/// This distinction is the whole of divergence 6. `TextField` already had it (in the
/// shape of `text_fields`) because focusing a field is obviously not the same act as
/// firing its handler. `Switch` and `Slider` are the same kind of thing and had no
/// equivalent, so `render_thread`'s tap walk had nothing to recognise them by: they
/// register `"change"`, the walk only ever asked `handles(id, "tap")`, and the answer
/// was no. The result was not an error — it was silence.
///
/// Derived from exactly the same `tag`/`attrs` disambiguation `a11y_role` already
/// does (see its own comment on why `"input"` alone is ambiguous across
/// `TextField`/`Switch`/`Slider`), so the two cannot disagree about what a node is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Control {
    /// A `Switch`. `checked` is its state *in this build*, so a caller toggling it
    /// sends `!checked` and never has to track the state itself.
    Switch { checked: bool },
    /// A `Radio`. Carries `checked` for symmetry, but the toggle rule is different and
    /// that is the entire reason it is a separate variant rather than a `Switch`
    /// (../PLAN.md §30): **a tap on a radio always sets `true`, never `!checked`.** A
    /// real `<input type=radio>` cannot be unchecked by clicking it, and a radio that
    /// turned itself off would leave its group with nothing selected — a state the
    /// group's own `@store` field usually cannot even represent.
    ///
    /// `Checkbox` is deliberately **not** here: its interaction is `Switch`'s exactly,
    /// so it maps to `Control::Switch` and only its accessible role differs
    /// (`a11y_role`). Adding a variant that behaved identically would be inviting the
    /// two to drift.
    Radio { checked: bool },
    /// A `Slider`. The full range is carried because the value a drag selects is
    /// computed from geometry (`zkite_renderer::slider_value_at`), not from the
    /// current value — a drag is absolute, not relative.
    Slider { min: f32, max: f32, step: f32, value: f32 },
}

/// The result of walking a `RawNode` tree into a `Renderer`: the root `NodeId`, and
/// enough bookkeeping to turn a `hit_test`/`parent`-walk result back into a
/// dispatchable `zkite_py` node id.
pub struct BuiltTree {
    pub root: NodeId,
    /// The taffy id of the app's own root widget (`view()`'s return value) — *not*
    /// `root`, which is `build`'s synthetic `flex flex-col w-full h-full` wrapper
    /// (needed for layout, meaningless for accessibility). AccessKit's own tree needs
    /// exactly one root node; the app's real root already gets an ordinary
    /// `accesskit::Node` (usually `Role::GenericContainer`, same as any other `div`)
    /// from `build_node`, and `common_filter`'s own `GenericContainer` handling
    /// already makes a non-interactive root transparent to assistive tech while
    /// still exposing its children — see ../PLAN.md §5 item 2's own write-up for why
    /// no synthetic `Role::Window` wrapper is needed here.
    pub a11y_root: NodeId,
    /// Every element node's taffy id -> its `zkite_py` node id. `#text` nodes have no
    /// id of their own and are never inserted — a hit on one always means "walk to
    /// its parent," exactly like `Renderer::hit_test`'s own doc comment already
    /// describes for any non-interactive node.
    node_ids: FxHashMap<NodeId, Arc<str>>,
    /// `zkite_py` node id -> the event names it has a handler registered for.
    events: FxHashMap<Arc<str>, Vec<Arc<str>>>,
    /// `zkite_py` node ids of every `TextField` in this build — a tap-dispatch walk
    /// (../gpu_harness/src/render_thread.rs) checks this the same way it checks
    /// `handles(id, "tap")`, to decide "focus this" vs. "call this node's handler."
    text_fields: FxHashSet<Arc<str>>,
    /// `zkite_py` node id -> the value-bearing control it is, for every `Switch` and
    /// `Slider` in this build. See `Control`, and `control`/`control_node` below.
    controls: FxHashMap<Arc<str>, Control>,
    /// The taffy `NodeId` of the *currently focused* field's own synthesized text
    /// child (see `build_node`'s `is_text_field` branch), if any — the render
    /// thread's caret painting looks this up in `positioned()`'s output to find
    /// where the live text actually ended up on screen. `None` whenever `build` was
    /// called with no `FocusedField` (nothing focused) or the given zkite id wasn't
    /// actually present in this tree.
    pub focused_text_node: Option<NodeId>,
    /// Every element node's taffy id -> its (real, but bounds-less — see
    /// `accessibility_tree_update`) `accesskit::Node` (../PLAN.md §5 item 2). Built
    /// alongside `node_ids`/`events` in the same `build_node` walk, since role/label/
    /// value/actions all come from the same `RawNode` data those two already read;
    /// bounds can't be filled in here because layout hasn't run yet when `build`
    /// returns — see `accessibility_tree_update`, which does that once `positioned()`
    /// is available.
    a11y_nodes: FxHashMap<NodeId, accesskit::Node>,
    /// The reverse of "hash `zkite_id` to get an `accesskit::NodeId`"
    /// (`accesskit_id`) — needed because that hash isn't invertible, and a caller
    /// handling an AccessKit action request (`../gpu_harness/src/render_thread.rs`'s
    /// `handle_a11y_action`) only has the `accesskit::NodeId` the platform adapter
    /// reported, not the `zkite_id` it came from. See `zkite_id_for_a11y`.
    a11y_ids: FxHashMap<accesskit::NodeId, Arc<str>>,
    /// Every animation this tree declares, in document order (../PLAN.md §47).
    ///
    /// A `Vec` and not a map: it holds the *animating* nodes rather than all of them,
    /// it is iterated whole exactly once per frame, and document order is the order an
    /// author would expect two declarations to be started in.
    anims: Vec<AnimDecl>,
    /// Declarations this tree carried that could not be parsed, already prefixed with
    /// the node they came from. Reported by the caller, which can change-gate them; a
    /// broken declaration is re-parsed on every rebuilt frame and logging from the
    /// parse site would print it sixty times a second (§7.14).
    anim_errors: Vec<String>,
}

/// One node's animation declarations, paired with the taffy id the driver applies
/// them to. See [`BuiltTree::animations`].
#[derive(Debug, Clone)]
pub struct AnimDecl {
    pub zkite_id: Arc<str>,
    pub node: NodeId,
    pub specs: Vec<AnimSpec>,
}

impl BuiltTree {
    /// Every animation this tree declares — the driver's whole input.
    pub fn animations(&self) -> &[AnimDecl] {
        &self.anims
    }

    /// Declarations this tree carried that could not be parsed.
    pub fn animation_errors(&self) -> &[String] {
        &self.anim_errors
    }

    /// The `zkite_py` node id for a taffy `NodeId`, if it has one.
    pub fn zkite_id(&self, node: NodeId) -> Option<&str> {
        self.node_ids.get(&node).map(|s| s.as_ref())
    }

    /// Whether `zkite_id` has a handler registered for `event` (`"tap"`, `"change"`,
    /// ...) — the render thread's ancestor-walk uses this to decide whether a hit
    /// counts as "this node handles it" or "keep walking up."
    pub fn handles(&self, zkite_id: &str, event: &str) -> bool {
        self.events.get(zkite_id).is_some_and(|evs| evs.iter().any(|e| e.as_ref() == event))
    }

    /// Whether `zkite_id` is a `TextField` — see `text_fields`'s doc comment.
    pub fn is_text_field(&self, zkite_id: &str) -> bool {
        self.text_fields.contains(zkite_id)
    }

    /// The value-bearing control `zkite_id` is, if it is one. See `Control`.
    pub fn control(&self, zkite_id: &str) -> Option<Control> {
        self.controls.get(zkite_id).copied()
    }

    /// The nearest `Control` at or above `node`, with the taffy id it was found on —
    /// the ancestor walk `render_thread`'s tap and drag paths both need, factored
    /// here so they cannot disagree about what counts as "inside a slider."
    ///
    /// Walking is not optional: `Switch` and `Slider` both synthesize child nodes
    /// (a thumb; a track, a fill and a thumb), so a touch essentially never lands on
    /// the control's own node — `hit_test` returns the deepest node under the point,
    /// which is one of those children. Same target-vs-currentTarget reasoning as
    /// `Renderer::parent`'s own doc comment.
    pub fn control_at(&self, renderer: &Renderer, node: NodeId) -> Option<(Arc<str>, Control, NodeId)> {
        let mut cur = Some(node);
        while let Some(n) = cur {
            if let Some(zkite_id) = self.node_ids.get(&n) {
                if let Some(control) = self.controls.get(zkite_id) {
                    return Some((zkite_id.clone(), *control, n));
                }
            }
            cur = renderer.parent(n);
        }
        None
    }

    /// Every element's taffy id -> its `zkite_py` node id (the reverse of `zkite_id`),
    /// for a caller that needs to go the other way — e.g. resolving a *persistent*,
    /// zkite-id-keyed scroll-offset map (offsets must survive across rebuilds; taffy
    /// `NodeId`s don't — see `../gpu_harness/src/render_thread.rs`'s `State`) into
    /// the taffy-id-keyed map `zkite_renderer::Renderer::positioned`/`hit_test` expect
    /// for *this* build.
    pub fn scroll_offsets_for(&self, persistent: &FxHashMap<Arc<str>, (f32, f32)>) -> FxHashMap<NodeId, (f32, f32)> {
        self.node_ids
            .iter()
            .filter_map(|(node, zkite_id)| persistent.get(zkite_id).map(|&off| (*node, off)))
            .collect()
    }

    /// Clamp retained offsets after filtering, resizing or replacing scroll content.
    /// Otherwise a shorter list can remain scrolled entirely past its last row.
    pub fn clamp_scroll_offsets(&self, renderer: &Renderer, persistent: &mut FxHashMap<Arc<str>, (f32, f32)>) {
        for (node, id) in &self.node_ids {
            if let Some(offset) = persistent.get_mut(id) {
                offset.0 = offset.0.clamp(0.0, renderer.max_scroll(*node, zkite_renderer::ScrollAxis::X));
                offset.1 = offset.1.clamp(0.0, renderer.max_scroll_y(*node));
            }
        }
    }

    /// The `zkite_py` node id an `accesskit::NodeId` (from an AccessKit
    /// `ActionRequest.target_node` — ../PLAN.md §5 item 2) was derived from, if this
    /// build has one. See `a11y_ids`'s own doc comment for why this can't just invert
    /// `accesskit_id` directly.
    pub fn zkite_id_for_a11y(&self, id: accesskit::NodeId) -> Option<&str> {
        self.a11y_ids.get(&id).map(|s| s.as_ref())
    }

    /// Assembles the `accesskit::TreeUpdate` for this build (../PLAN.md §5 item 2):
    /// stamps real screen bounds from `positioned` (must be `positioned(self.root,
    /// ...)`'s own output — same absolute coordinate space every other consumer of it
    /// uses, so a node's accessibility bounds always agree with where it's actually
    /// painted/hit-tested) onto the bounds-less nodes `build_node` already built, then
    /// wraps them with `Tree`/`focus`. `focused` is the currently-focused `TextField`'s
    /// `zkite_py` id, if any (`render_thread::State::focused` — not threaded through
    /// `build`/`FocusedField`, since a *value* override needs to affect `build_node`'s
    /// walk but a *focus* target only needs to affect this final assembly step, where
    /// the target's `accesskit::NodeId` is already cheap to derive from its zkite id).
    /// A full tree, not a diff, on every call — matching `zkite_renderer`'s own "callers
    /// rebuild fresh per render" convention (`accesskit_consumer::Tree::
    /// update_and_process_changes` does its own old-vs-new diffing internally to
    /// compute change events, so nothing upstream needs to hand-diff anything).
    pub fn accessibility_tree_update(
        &self,
        positioned: &[zkite_renderer::PositionedNode],
        focused: Option<&str>,
        scale: f32,
    ) -> accesskit::TreeUpdate {
        let mut nodes = Vec::with_capacity(self.a11y_nodes.len());
        for pn in positioned {
            let Some(base) = self.a11y_nodes.get(&pn.node) else { continue };
            let mut node = base.clone();
            // `positioned` is in dp; Android's accessibility bounds are view-local
            // *device* pixels — `AccessibilityNodeInfo.setBoundsInScreen` is what
            // TalkBack draws its focus rectangle from and what an assistive
            // technology taps through, so a dp rect here would put every green box
            // at `1/density` of the widget it names (../PLAN.md §13).
            //
            // Multiplied explicitly rather than set as an `accesskit` transform on
            // the root: §8.2 records that no node has ever carried a transform, so
            // accesskit's own composition is the identity and `Node::bounding_box()`
            // is exactly this rect. Keeping that invariant true is worth more than
            // the one saved multiply, and this is verifiable by reading one line.
            let (x, y, w, h) = pn.rect;
            let (x, y, w, h) = (x * scale, y * scale, w * scale, h * scale);
            node.set_bounds(accesskit::Rect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64));
            nodes.push((accesskit_id_for(self.zkite_id(pn.node)), node));
        }
        let root_id = accesskit_id_for(self.zkite_id(self.a11y_root));
        let focus_id = focused.map(accesskit_id).unwrap_or(root_id);
        accesskit::TreeUpdate {
            nodes,
            tree: Some(accesskit::Tree::new(root_id)),
            tree_id: accesskit::TreeId::ROOT,
            focus: focus_id,
        }
    }
}

/// Derives a stable `accesskit::NodeId` from a `zkite_py` node id — stable across
/// rebuilds (unlike taffy `NodeId`s, fresh every render) so AccessKit's own tree
/// diffing (`accesskit_consumer::Tree::update_and_process_changes`, which compares
/// old vs. new by `NodeId` to decide what changed) sees the *same* element as the
/// same node across renders, not a spurious remove+add every frame. Collisions are
/// possible in principle (64-bit hash of an unbounded string) but not a practical
/// risk for a UI tree — the same trust level this crate already places in `zkite_id`
/// strings being unique per render (`node_ids`/`events` are keyed the same way).
fn accesskit_id(zkite_id: &str) -> accesskit::NodeId {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    zkite_id.hash(&mut hasher);
    accesskit::NodeId(hasher.finish())
}

/// `accesskit_id`, but for a taffy `NodeId` that's expected to have a `zkite_py` id —
/// every node `build_node` ever inserts into `a11y_nodes` also has one (see that
/// function), so the `unwrap` only panics if this crate's own two maps have fallen
/// out of sync with each other, which would be a real bug in `build_node`, not a
/// possible-in-practice runtime condition.
fn accesskit_id_for(zkite_id: Option<&str>) -> accesskit::NodeId {
    accesskit_id(zkite_id.expect("every node in a11y_nodes must have a zkite_id"))
}

/// Walks `root` into `renderer`, replacing whatever tree it held before (calls
/// `Renderer::clear` first, matching `zkite_renderer`'s own "callers build a fresh
/// tree per render" convention). `focused` overrides a single `TextField`'s
/// displayed text with live IME state instead of its `RawNode` `value`/`placeholder`
/// attrs — see `FocusedField`'s doc comment; pass `None` when nothing has focus
/// (every call site outside `gpu_harness`'s render thread, and that one too whenever
/// `render_thread::State::focused` is `None`).
pub fn build(renderer: &mut Renderer, root: &RawNode, focused: Option<&FocusedField>) -> BuiltTree {
    renderer.clear();
    let mut ctx = BuildCtx {
        node_ids: FxHashMap::default(),
        events: FxHashMap::default(),
        text_fields: FxHashSet::default(),
        controls: FxHashMap::default(),
        focused,
        focused_text_node: None,
        a11y_nodes: FxHashMap::default(),
        a11y_ids: FxHashMap::default(),
        anims: Vec::new(),
        anim_errors: Vec::new(),
    };
    let app_root = build_node(renderer, root, &mut ctx);
    // Mirrors `phase_1/webruntime/index.html`'s `#zkite-root` CSS (`min-height:100%;
    // display:flex; flex-direction:column`), added there for the identical reason
    // (see that file's own comment, found on a real device in an earlier phase):
    // `view()`'s own root widget commonly carries `flex-1` to fill the screen (see
    // plan1.md §8's reference app), but `flex-grow` only does anything inside a flex
    // *parent* — and `view()`'s return value has none of its own, since nothing
    // wraps it. The WebView backend gets one for free from the static HTML shell
    // around `zkite_py`'s output (`#zkite-root`, not part of `zkite_py` itself); the
    // native renderer has no such shell, so this crate synthesizes the equivalent
    // wrapper here instead of leaving every app's root widget collapsed to its
    // content size in the corner of the screen — confirmed to reproduce exactly that
    // way on a real device before this fix, not a hypothetical. `hit_test`/`parent`
    // treat it like any other id-less wrapper (e.g. `Stack`'s), so dispatch is
    // unaffected.
    let root_id = renderer.container("flex flex-col w-full h-full", |_| vec![app_root]);
    BuiltTree {
        root: root_id,
        a11y_root: app_root,
        node_ids: ctx.node_ids,
        events: ctx.events,
        text_fields: ctx.text_fields,
        controls: ctx.controls,
        focused_text_node: ctx.focused_text_node,
        a11y_nodes: ctx.a11y_nodes,
        a11y_ids: ctx.a11y_ids,
        anims: ctx.anims,
        anim_errors: ctx.anim_errors,
    }
}

/// Threaded through the whole recursive walk instead of four separate `&mut`
/// parameters — `focused`/`focused_text_node` joined the original `node_ids`/
/// `events` pair for the same reason those two travel together already: every one of
/// them is populated (or read) once per node, deep inside `build_node`'s recursion.
#[derive(Default)]
struct BuildCtx<'a> {
    node_ids: FxHashMap<NodeId, Arc<str>>,
    events: FxHashMap<Arc<str>, Vec<Arc<str>>>,
    text_fields: FxHashSet<Arc<str>>,
    controls: FxHashMap<Arc<str>, Control>,
    focused: Option<&'a FocusedField<'a>>,
    focused_text_node: Option<NodeId>,
    a11y_nodes: FxHashMap<NodeId, accesskit::Node>,
    a11y_ids: FxHashMap<accesskit::NodeId, Arc<str>>,
    anims: Vec<AnimDecl>,
    anim_errors: Vec<String>,
}

fn build_node(renderer: &mut Renderer, node: &RawNode, ctx: &mut BuildCtx) -> NodeId {
    if node.tag == "#text" {
        return renderer.text(&node.classes, node.text.clone().unwrap_or_default());
    }

    // `TextField` (`phase_1/zkite_py/src/zkite/widgets.py`) deliberately emits no
    // children of its own — what to *display* (the value, a dimmed placeholder when
    // it's empty, or — as of ../PLAN.md §5 — the live IME text while focused) is
    // rendering policy tied to focus/IME state that only this side tracks, not
    // something Python can usefully bake into a snapshot. `"input"` alone isn't
    // specific enough (Switch/Slider are `input` too), so this checks for the
    // distinctively-named `input_type` key `TextField._tree_attrs` sets and
    // Switch/Slider don't.
    let is_text_field = node.tag == "input" && node.attrs.contains_key("input_type");
    let is_the_focused_field =
        is_text_field && node.id.as_deref().is_some_and(|id| ctx.focused.is_some_and(|f| f.zkite_id == id));

    // Hoisted out of the `container` closure below (it reads only `node`/`ctx`, never
    // `r`) so it's available afterward too, for the accessibility node's `value`/
    // `label` (../PLAN.md §5 item 2) — a pure refactor, no change in what gets shown.
    let (shown, dimmed) = if is_text_field {
        let (mut shown, dimmed) = if is_the_focused_field {
            // Live IME state overrides whatever `main.py`'s last render said —
            // see `FocusedField`'s doc comment. Never dimmed: a focused field
            // showing dimmed placeholder styling while the user is actively
            // typing into it would be a real, confusing visual bug, not a
            // cosmetic nit.
            (ctx.focused.expect("is_the_focused_field implies focused.is_some()").text.to_string(), false)
        } else {
            let value = attr_str(&node.attrs, "value");
            let placeholder = attr_str(&node.attrs, "placeholder");
            if !value.is_empty() { (value, false) } else { (placeholder, true) }
        };
        // `secure=True` (../../phase_3/PLAN.md §5/§7.5) must never paint the
        // real characters — a password field that renders its own value in the
        // clear defeats the entire point of the prop, regardless of what the
        // IME's own `inputType` does. Masks only the real value/live text, not
        // a placeholder (`dimmed`) — the placeholder is instructional copy
        // ("Password"), not user data. See `mask_password` for why the mask is
        // one `*` per *byte*, not per character.
        if !dimmed && node.attrs.get("input_type").and_then(|v| v.as_str()) == Some("password") {
            shown = mask_password(&shown);
        }
        (shown, dimmed)
    } else {
        (String::new(), false)
    };

    // Captured out of the closure below (`Renderer::container` only returns this
    // node's own `NodeId`, not its children's) so the accessibility node built after
    // it returns can still see which taffy children exist — needed to derive its
    // `accesskit` child list once `ctx.node_ids` has entries for all of them (see
    // below). A plain outer `let mut` + closure mutation, not `Rc<RefCell<_>>>`: the
    // closure is `FnOnce` and its mutable borrow of `kids_out` ends before `container`
    // returns, so reading it right after is fine, same as any other single-use
    // mutable capture.
    let mut kids_out: Vec<NodeId> = Vec::new();
    // `Icon` (`svg`) is the one widget whose content is neither children nor text but a
    // bundled vector, so it gets its own leaf constructor. `Icon._tree_children` emits
    // no children at all (`phase_1/zkite_py/src/zkite/widgets.py`) — the sprite *is* the
    // content — so nothing is dropped by not recursing here, and `kids_out` staying
    // empty is what the accessibility node below already expects for a leaf. The node
    // is still built from `node.classes` exactly as a container would be, so its box
    // (`w-[20px] h-[20px]`) is unchanged from before this widget painted anything.
    let id = if node.tag == "svg" {
        renderer.icon(&node.classes, &attr_str(&node.attrs, "icon_name"))
    } else if node.tag == "img" {
        // The second widget whose content is neither children nor text (../PLAN.md
        // §35). `Image._tree_children` emits no children — the bitmap *is* the
        // content — so nothing is dropped by not recursing, exactly as for `svg`.
        //
        // The node is still built from `node.classes` as a container would be, so its
        // box is byte-identical to what it was when `Image` painted nothing at all.
        // That is the property §35 has to preserve above every other: an `Image` has
        // always reserved correct layout space, and every app already positioned
        // around it.
        renderer.image(&node.classes, &attr_str(&node.attrs, "src"))
    } else {
        renderer.container(&node.classes, |r| {
            let mut kids: Vec<NodeId> = node.children.iter().map(|c| build_node(r, c, ctx)).collect();
            if is_text_field && (!shown.is_empty() || is_the_focused_field) {
                // Inherits the field's own typography otherwise; dimmed placeholder
                // text is the one deliberate exception, so it gets an explicit
                // override rather than inheriting the (likely full-opacity) value
                // color. The focused field always gets a text child even when
                // `shown` is empty (freshly cleared, mid-edit) — `render_thread.rs`'s
                // caret painting needs `focused_text_node` to resolve to a real,
                // positioned node to anchor an empty-field caret at its left edge,
                // not `None`.
                let text_id = r.text(if dimmed { "text-slate-500" } else { "" }, shown.clone());
                if is_the_focused_field {
                    ctx.focused_text_node = Some(text_id);
                }
                kids.push(text_id);
            }
            kids_out = kids.clone();
            kids
        })
    };

    if let Some(zkite_id) = &node.id {
        let zkite_id: Arc<str> = Arc::from(zkite_id.as_str());
        if is_text_field && !node.attrs.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false) {
            ctx.text_fields.insert(zkite_id.clone());
        }
        if let Some(control) = control_of(node) {
            ctx.controls.insert(zkite_id.clone(), control);
        }
        ctx.node_ids.insert(id, zkite_id.clone());
        if !node.events.is_empty() {
            ctx.events.insert(zkite_id.clone(), node.events.iter().map(|e| Arc::from(e.as_str())).collect());
        }
        // ../PLAN.md §47. Collected here, inside the walk that is already visiting
        // every node, rather than by a second pass over the `RawNode` tree — the
        // driver needs both the specs and the taffy id, and this is the one place
        // that has them together. A node with no `anim` attribute costs one map
        // lookup, which is every node in every app that does not animate.
        if let Some(raw) = node.attrs.get("anim") {
            match anim::parse(Some(raw)) {
                Ok(specs) if !specs.is_empty() => {
                    ctx.anims.push(AnimDecl { zkite_id: zkite_id.clone(), node: id, specs })
                }
                Ok(_) => {}
                // Carried out rather than logged from here: this crate is host-tested
                // and has no logger, and the caller change-gates the report so a
                // broken declaration does not print sixty times a second (§7.14).
                Err(e) => ctx.anim_errors.push(format!("node {zkite_id}: {e}")),
            }
        }
        // `build_a11y_node` must run and return *before* `ctx.a11y_nodes` is borrowed
        // mutably below — it takes `ctx: &BuildCtx` (the whole context, since it
        // reads `ctx.node_ids` to resolve children), which would otherwise overlap
        // with the `&mut ctx.a11y_nodes` the `insert` call needs.
        let a11y_node = build_a11y_node(node, &kids_out, &shown, dimmed, ctx);
        ctx.a11y_nodes.insert(id, a11y_node);
        ctx.a11y_ids.insert(accesskit_id(&zkite_id), zkite_id);
    }
    id
}

/// Which `Control`, if any, one `RawNode` element is (../PLAN.md §26).
///
/// The disambiguation is deliberately the *same* one `a11y_role` makes, in the same
/// order, for the same reason its own comment gives: `tag == "input"` covers
/// `TextField`, `Switch` and `Slider` alike, so the distinguishing attribute is what
/// decides. `input_type` is checked first and rejects, because a `TextField` is a
/// value-bearing control too but is driven by the IME path, not by touch geometry.
///
/// Reading these from attrs rather than from a new Python-side marker is what keeps
/// `zkite_py` free of native-renderer knowledge: `Switch._tree_attrs` already emits
/// `checked` and `Slider._tree_attrs` already emits `min`/`max`/`step`/`value`,
/// because the WebView backend needed exactly the same facts.
fn control_of(node: &RawNode) -> Option<Control> {
    // A presentational node is never a dispatch target. This is what keeps a
    // *labelled* `Checkbox`/`Radio`/`Switch` (../PLAN.md §32) to exactly one handler:
    // its box is still a real `input` carrying `checked` — so that the HTML path can
    // keep drawing it with the CSS it always used — but the semantics belong to the
    // container above it, which registers a plain `tap`. Without this, `control_at`'s
    // walk would find the box first and the render thread would synthesize a `change`
    // as well, which is the double-dispatch the label feature exists to remove.
    if is_aria_hidden(node) || node.attrs.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false) {
        return None;
    }
    if node.tag != "input" || node.attrs.contains_key("input_type") {
        return None;
    }
    if let Some(checked) = node.attrs.get("checked").and_then(|v| v.as_bool()) {
        // `role` distinguishes the three checkbox-shaped widgets, which `tag` and
        // `checked` cannot — all three are an `input` carrying `checked` (../PLAN.md
        // §30). Only `radio` changes the *interaction*; `switch` and `checkbox` toggle
        // identically and share a variant. An absent role means a tree that predates
        // the attribute (or a third-party `Widget`), and toggling is the safe reading.
        return Some(match node.attrs.get("role").and_then(|v| v.as_str()) {
            Some("radio") => Control::Radio { checked },
            _ => Control::Switch { checked },
        });
    }
    let num = |k: &str, default: f64| node.attrs.get(k).and_then(|v| v.as_f64()).unwrap_or(default);
    if node.attrs.contains_key("min") && node.attrs.contains_key("max") {
        return Some(Control::Slider {
            min: num("min", 0.0) as f32,
            max: num("max", 100.0) as f32,
            // A `step` of 0 means "unquantised" to `slider_value_at`, which is the
            // right reading of a missing key: `Slider`'s own default is 1, so an
            // absent `step` here means a tree that did not come from `Slider`.
            step: num("step", 0.0) as f32,
            value: num("value", 0.0) as f32,
        });
    }
    None
}

/// Builds the (bounds-less — see `BuiltTree::accessibility_tree_update`) `accesskit`
/// node for one `RawNode` element (../PLAN.md §5 item 2). `kids` are this node's
/// already-built taffy children (containers *and* `#text` leaves alike — see
/// `build_node`'s `kids_out` capture); only the ones with their own `zkite_py` id
/// (i.e. not a bare `#text` leaf, which folds into *this* node's own label/value
/// instead, per `text_label_from_children` below — the same "innerText" collapsing
/// real accessibility trees already do for a plain `<button>text</button>`, not a
/// ZKite-specific choice) become real `accesskit` children.
fn build_a11y_node(node: &RawNode, kids: &[NodeId], shown: &str, dimmed: bool, ctx: &BuildCtx) -> accesskit::Node {
    let is_text_field = node.tag == "input" && node.attrs.contains_key("input_type");
    let mut a11y = accesskit::Node::new(a11y_role(node));

    let children: Vec<accesskit::NodeId> =
        kids.iter().filter_map(|k| ctx.node_ids.get(k)).map(|id| accesskit_id(id)).collect();
    if !children.is_empty() {
        a11y.set_children(children);
    }

    if is_text_field {
        // See `build_node`'s hoisted `shown`/`dimmed`: `dimmed` means `shown` is a
        // placeholder (nothing real typed yet), which maps to an accessible *label*
        // (what the field is), not a *value* (what's in it) — this field has no
        // separate label prop of its own (`phase_1/zkite_py/src/zkite/widgets.py`'s
        // `TextField` only exposes `value`/`placeholder`/`type`/`secure`), so the
        // placeholder is the closest real accessible name available, the same
        // fallback a plain HTML `<input placeholder=...>` with no `<label>` gets in
        // a real browser's own accessibility tree.
        if dimmed {
            if !shown.is_empty() {
                a11y.set_label(shown.to_string());
            }
        } else {
            a11y.set_value(shown.to_string());
        }
        a11y.add_action(accesskit::Action::Focus);
    } else if let Some(checked) = node.attrs.get("checked").and_then(|v| v.as_bool()) {
        a11y.set_toggled(if checked { accesskit::Toggled::True } else { accesskit::Toggled::False });
        // A *labelled* control carries its own accessible name (../PLAN.md §32) — the
        // thing that makes TalkBack announce "Wi-Fi, checkbox, checked" instead of an
        // unnamed checkbox followed by a loose piece of text. Deep rather than
        // direct-children-only (`text_label_from_children`, used by the generic branch
        // below) because the label is ordinary content and may well be a `Text` widget
        // the caller styled: `Checkbox("", Text("font-bold", "High"), ...)`.
        //
        // A *bare* control has no text descendants at all, so this sets nothing and
        // that path is unchanged.
        if let Some(label) = text_label_deep(node) {
            a11y.set_label(label);
        }
    } else if let Some(selected) = node.attrs.get("selected").and_then(|v| v.as_bool()) {
        // One option in a `Select`'s popup (../PLAN.md §33). `selected` reaches
        // Android as `AccessibilityNodeInfo.setSelected`, so which option is current
        // is announced rather than merely drawn — the same argument §32.5 makes for
        // `indicator=False`: the app owns the styling, the platform owns the meaning.
        //
        // Deep rather than direct-children-only for the same reason the `checked`
        // branch above is: an option's content is ordinary widgets (a `Text`, and a
        // tick `Icon` when it is the current one), and with `option=` it is whatever
        // the app returned.
        a11y.set_selected(selected);
        if let Some(label) = text_label_deep(node) {
            a11y.set_label(label);
        }
    } else if matches!(node.attrs.get("role").and_then(|v| v.as_str()), Some("combobox")) {
        // A `Select`'s trigger holds its label in a child `span` and a chevron in a
        // sibling `svg`, so `text_label_from_children` (direct `#text` only) finds
        // nothing and the control would be announced unnamed. Deliberately matched on
        // the role rather than widening the generic branch below, which would change
        // the announced name of every `Button` holding widget children — a real
        // pre-existing gap, named in §32's own known gaps, and still not this
        // section's to change.
        //
        // Deliberately *not* extended to `listbox`: a list's accessible name is not
        // its items' text run together, and `text_label_deep` on the panel would
        // produce exactly that.
        if let Some(label) = text_label_deep(node) {
            a11y.set_label(label);
        }
    } else if let (Some(min), Some(max)) = (
        node.attrs.get("min").and_then(|v| v.as_f64()),
        node.attrs.get("max").and_then(|v| v.as_f64()),
    ) {
        // Slider (../../phase_1/zkite_py/src/zkite/widgets.py) — `value`, deliberately
        // not set here: `accesskit_android::node`'s own glue only emits a numeric
        // `RangeInfo` when `data().value()` is *absent* (checked directly against
        // that crate's `populate_node_info` — see phase_3/PLAN.md §5 item 2), so
        // setting both would silently suppress the range Android actually renders.
        a11y.set_min_numeric_value(min);
        a11y.set_max_numeric_value(max);
        if let Some(value) = node.attrs.get("value").and_then(|v| v.as_f64()) {
            a11y.set_numeric_value(value);
        }
    } else {
        let label = text_label_from_children(node)
            // Icon (`svg`) has no real alt-text prop yet (a real gap — see
            // ../PLAN.md §5 item 2's known-gaps note) — its bundled sprite name is a
            // better-than-nothing fallback for the common case of a self-descriptive
            // name like "close"/"search", not a substitute for a real `label=`/
            // `decorative=` prop on the widget itself.
            .or_else(|| (node.tag == "svg").then(|| attr_str(&node.attrs, "icon_name")).filter(|s| !s.is_empty()))
            // `Image`'s `alt` is a *real* alt-text prop, unlike `Icon`'s sprite-name
            // fallback above — `Image(alt="...")` has been in the widget's
            // `_tree_attrs` since phase 1 and reached nothing. A screen reader now
            // hears it (../PLAN.md §35.3).
            //
            // **An empty `alt` stays unlabelled, and that is the accessible answer,
            // not a shortcut**: `alt=""` is HTML's way of saying "decorative, skip
            // me", so filtering it here makes native agree with what the WebView
            // backend already does with the identical attribute.
            .or_else(|| (node.tag == "img").then(|| attr_str(&node.attrs, "alt")).filter(|s| !s.is_empty()));
        if let Some(label) = label {
            a11y.set_label(label);
        }
    }

    if node.attrs.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false) {
        a11y.set_disabled();
    }
    // Whether a `Select`'s popup is open. Set outside the branch chain above because
    // it is orthogonal to how the node is *named* — the same shape as `disabled`.
    // **This one does not reach Android**: `accesskit_android` 0.7.5 consumes
    // `is_selected` but never reads `is_expanded`, so the state is correct in the
    // accesskit tree and invisible to TalkBack. Set anyway rather than skipped — the
    // tree is the thing this crate is responsible for, and the WebView backend gets
    // the equivalent from Chromium's own `aria-expanded` handling. §33's known gaps.
    if let Some(expanded) = node.attrs.get("expanded").and_then(|v| v.as_bool()) {
        a11y.set_expanded(expanded);
    }
    if node.events.iter().any(|e| e == "tap") {
        a11y.add_action(accesskit::Action::Click);
    }

    a11y
}

/// Concatenates every *direct*, bare (`#text`) child's content — real HTML/AOM
/// accessibility trees collapse plain inline text the same way (a `<button>Go</button>`
/// exposes one accessible node named "Go", not a button node containing a separate
/// text node), and `zkite_renderer`'s own paint tree already treats a bare string
/// child identically regardless of which widget it's under (see
/// `zkite_renderer::Renderer::text`'s doc comment). `None` (not `Some("")`) when there
/// are none, so callers can `.or_else`/`.unwrap_or` a fallback cleanly.
fn text_label_from_children(node: &RawNode) -> Option<String> {
    let parts: Vec<&str> =
        node.children.iter().filter(|c| c.tag == "#text").filter_map(|c| c.text.as_deref()).collect();
    (!parts.is_empty()).then(|| parts.concat())
}

/// Whether `node` is marked presentational. Emitted on **both** render paths by
/// `phase_1/zkite_py/src/zkite/widgets.py` — `aria-hidden="true"` in HTML, `aria_hidden:
/// true` in the tree — rather than inferred here from "has a control ancestor", so the
/// two backends read the same stated fact (../PLAN.md §30.1's role-not-marker
/// principle, applied to state instead of to type).
fn is_aria_hidden(node: &RawNode) -> bool {
    node.attrs.get("aria_hidden").and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Every `#text` in `node`'s subtree, concatenated — i.e. exactly what the DOM's
/// `textContent` would produce for the same tree, which is what an accessible name
/// computed from element contents is in a real browser.
///
/// Skips `aria-hidden` subtrees, so a labelled control's own presentational box can
/// never contribute to its name. `text_label_from_children` above stays
/// direct-children-only on purpose: widening *it* would change the accessible name of
/// every `Button` that holds widget children rather than a bare string, which is a
/// real (pre-existing) gap but not this section's — see ../PLAN.md §32's known gaps.
fn text_label_deep(node: &RawNode) -> Option<String> {
    fn walk(node: &RawNode, out: &mut Vec<String>) {
        for child in &node.children {
            if is_aria_hidden(child) {
                continue;
            }
            if child.tag == "#text" {
                if let Some(text) = child.text.as_deref().filter(|t| !t.is_empty()) {
                    out.push(text.to_string());
                }
            } else {
                walk(child, out);
            }
        }
    }
    let mut parts = Vec::new();
    walk(node, &mut parts);
    (!parts.is_empty()).then(|| parts.concat())
}

/// Maps a `RawNode`'s `tag`/`attrs` (`phase_1/zkite_py/src/zkite/widgets.py`'s own
/// per-widget shape) to the `accesskit::Role` that best matches what Android's real
/// platform widgets expose for the same control — `accesskit_android::node`'s own
/// `class_name()` mapping (../PLAN.md §5 item 2) is what actually turns each of these
/// into e.g. `android.widget.EditText`/`SeekBar`/`ToggleButton` for TalkBack, so the
/// role chosen here is what drives that, not cosmetic. `"input"` alone is ambiguous
/// (`TextField`/`Switch`/`Slider` are all `input` — see `build_node`'s own
/// `is_text_field` comment on why a second attribute has to disambiguate) — checked
/// in the same order every other place in this crate already checks for these
/// distinguishing attrs.
fn a11y_role(node: &RawNode) -> accesskit::Role {
    use accesskit::Role;
    // A presentational node is transparent to assistive tech. `common_filter`
    // (`accesskit_consumer`) excludes a `GenericContainer` from what is exposed while
    // still exposing its children, which is exactly `aria-hidden`'s meaning here — a
    // labelled control's box must not be announced as a *second* checkbox next to the
    // container that really is one (../PLAN.md §32).
    if is_aria_hidden(node) {
        return Role::GenericContainer;
    }
    // A declared `role` beats the tag, for **any** tag. Before ../PLAN.md §32 the role
    // was consulted only inside the `input` arm below, because the only widgets that
    // emitted one were the three checkbox-shaped `input`s; a labelled control is a
    // container that is genuinely a checkbox, so the role has to be readable off it
    // too. This is behaviour-preserving for every tree that predates §32 — `role` is
    // emitted by `Switch`/`Checkbox`/`Radio` alone, all of them `input`, and the arm
    // below produced these same three answers for them
    // (`a11y_role_reads_the_role_attribute_off_any_tag` pins both halves).
    match node.attrs.get("role").and_then(|v| v.as_str()) {
        Some("switch") => return Role::Switch,
        Some("checkbox") => return Role::CheckBox,
        Some("radio") => return Role::RadioButton,
        // `Select` and its popup (../PLAN.md §33). The first two are what Android's
        // own glue turns into real platform classes — `accesskit_android::node`'s
        // `class_name()` maps `ComboBox` to `android.widget.Spinner` and `ListBox` to
        // `android.widget.ListView`, which is the same "the role is what reaches the
        // platform" property §30.1 chose a role for. `ListBoxOption` has no mapping
        // there and lands on the generic `android.view.View`; its *selected* state
        // does reach Android (`is_selected` -> `setSelected`), which is the half that
        // matters for announcing which option is current. See §33's known gaps.
        Some("combobox") => return Role::ComboBox,
        Some("listbox") => return Role::ListBox,
        Some("option") => return Role::ListBoxOption,
        _ => {}
    }
    match node.tag.as_str() {
        "button" => Role::Button,
        "span" => Role::Label,
        "img" => Role::Image,
        "svg" => Role::Image,
        "input" => {
            if node.attrs.contains_key("input_type") {
                match node.attrs.get("input_type").and_then(|v| v.as_str()).unwrap_or("text") {
                    "password" => Role::PasswordInput,
                    "email" => Role::EmailInput,
                    "url" => Role::UrlInput,
                    "tel" | "phone" => Role::PhoneNumberInput,
                    "number" => Role::NumberInput,
                    _ => Role::TextInput,
                }
            } else if node.attrs.contains_key("checked") {
                // ../PLAN.md §30. TalkBack announces these three differently and a
                // user relies on that difference — "switch" implies on/off, "checkbox"
                // implies an independent choice, "radio button" implies one-of-many —
                // so getting it from the widget's own declared role beats inferring it
                // from geometry, which is all that distinguishes them on screen. The
                // role itself is now read above, for any tag; reaching here means a
                // tree that carries no role at all (one predating §30, or a
                // third-party `Widget`), and a switch is the safe reading.
                Role::Switch
            } else if node.attrs.contains_key("min") {
                Role::Slider
            } else {
                // Shouldn't happen against `zkite_py`'s own widget set (every `input`
                // tag it emits is one of the three above) — a plain container is the
                // safe, non-crashing default for a third-party `Widget` subclass that
                // sets `tag = "input"` for its own reasons.
                Role::GenericContainer
            }
        }
        // `Column`/`Row`/`Stack`/`Grid`/`ScrollView` — every one of them a plain
        // `div` (`phase_1/zkite_py/src/zkite/widgets.py`) with no accessible meaning of
        // its own beyond grouping. `common_filter` (`accesskit_consumer`) excludes a
        // `GenericContainer` from what's actually exposed to assistive tech while
        // still exposing its children — see ../PLAN.md §5 item 2 — so this is safe
        // even for the app's own root widget, not just internal wrappers.
        _ => Role::GenericContainer,
    }
}

/// Android selection indices count UTF-16 units; the renderer counts UTF-8 bytes.
/// Clamp to a character boundary, including malformed indices inside a surrogate pair.
pub fn utf16_to_utf8_offset(text: &str, offset: i32) -> i32 {
    let target = offset.max(0) as usize;
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units + ch.len_utf16() > target { return byte as i32; }
        units += ch.len_utf16();
    }
    text.len() as i32
}

pub fn utf8_to_utf16_offset(text: &str, offset: i32) -> i32 {
    let mut byte = (offset.max(0) as usize).min(text.len());
    while !text.is_char_boundary(byte) { byte -= 1; }
    text[..byte].encode_utf16().count() as i32
}

/// Masks a `TextField(secure=True)`'s real text for display — one `*` per *byte*
/// of `text`, not one per character. Deliberately not character-accurate: every
/// byte-offset this native renderer computes against a field's text (IME selection
/// state, `hit_test_text_offset`, `text_caret_geometry` — see `../gpu_harness/src/
/// render_thread.rs`, which applies this exact same transform before shaping, so
/// caret/tap geometry lines up with what's actually painted) is a **byte** offset
/// into the real string, and `*` is one byte in UTF-8 while the mask character a
/// password would "obviously" use (`•`, U+2022) is three — masking per character
/// would leave every byte-offset downstream silently wrong the instant a password
/// contains any multi-byte character. Per-byte masking keeps the transform a
/// trivial index-preserving map (same length, same offsets, in both directions) at
/// the cost of a purely cosmetic one: a multi-byte character shows as more than
/// one `*`. Given passwords are overwhelmingly ASCII in practice, that trade is a
/// deliberate, documented simplification, not an oversight — see ../../phase_3/
/// PLAN.md §5/§7.5's known-gaps note if this ever needs revisiting for real
/// non-ASCII password use.
pub fn mask_password(text: &str) -> String {
    "*".repeat(text.len())
}

fn attr_str(attrs: &serde_json::Map<String, serde_json::Value>, key: &str) -> String {
    attrs.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialises every test that boots the embedded interpreter.
    ///
    /// There is exactly one CPython per process (`embed`'s `INIT: Once`) and exactly
    /// one live app inside it (`runtime.py`'s `_app` global) — both correct for the
    /// real thing, where a process hosts one app, and both shared by every test in
    /// this binary, which `cargo test` runs on parallel threads by default. A test
    /// that boots a *different* app therefore replaces the one another test is
    /// mid-way through using.
    ///
    /// This was latent until the virtualised-list test arrived: the two tests that
    /// booted an interpreter before it wrote *byte-identical* counter-app source, so
    /// whichever won the race produced the answer both were asserting anyway. Worth
    /// recording rather than just fixing — "the tests pass" was, for those two, not
    /// evidence that they couldn't interfere.
    ///
    /// Poisoning is deliberately ignored (`into_inner`): one genuinely failing test
    /// should report one failure, not cascade into every other test that touches
    /// Python.
    static PY: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_python() -> std::sync::MutexGuard<'static, ()> {
        PY.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn test_renderer() -> Renderer {
        let dir = std::env::var_os("ZKITE_TEST_FONT_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "/usr/share/fonts/truetype/dejavu".into());
        let bytes = std::fs::read(dir.join("DejaVuSans.ttf"))
            .expect("DejaVu Sans must be installed for zkite_pybridge's tests");
        let bold_bytes = std::fs::read(dir.join("DejaVuSans-Bold.ttf"))
            .expect("DejaVu Sans Bold must be installed for zkite_pybridge's tests");
        Renderer::new(zkite_renderer::FontSet::sans_only(bytes, bold_bytes)).unwrap()
    }

    #[test]
    fn android_selection_offsets_round_trip_unicode_boundaries() {
        let text = "Aéह😀z";
        for byte in text.char_indices().map(|(i, _)| i).chain([text.len()]) {
            let units = utf8_to_utf16_offset(text, byte as i32);
            assert_eq!(utf16_to_utf8_offset(text, units), byte as i32);
        }
        assert_eq!(utf16_to_utf8_offset(text, 4), 6); // inside emoji's surrogate pair
        assert_eq!(utf8_to_utf16_offset(text, 8), 3); // inside emoji's UTF-8 bytes
        assert_eq!(utf16_to_utf8_offset(text, -1), 0);
        assert_eq!(utf8_to_utf16_offset(text, 999), 6);
    }

    #[test]
    fn shrinking_scroll_content_clamps_retained_offsets_before_paint() {
        let mut renderer = test_renderer();
        let tree = |height| RawNode::from_json(&format!(r#"{{
            "tag":"div", "id":"list", "classes":"flex flex-col overflow-auto w-[100px] h-[100px]",
            "children":[{{"tag":"div", "id":"content", "classes":"shrink-0 w-[300px] h-[{height}px]"}}]
        }}"#)).unwrap();
        let mut offsets = FxHashMap::default();
        offsets.insert(Arc::<str>::from("list"), (180.0, 450.0));
        let built = build(&mut renderer, &tree(600), None);
        renderer.layout(built.root, 400.0, 800.0);
        built.clamp_scroll_offsets(&renderer, &mut offsets);
        assert_eq!(offsets["list"], (180.0, 450.0));
        let built = build(&mut renderer, &tree(150), None);
        renderer.layout(built.root, 400.0, 800.0);
        built.clamp_scroll_offsets(&renderer, &mut offsets);
        assert_eq!(offsets["list"], (180.0, 50.0));
        let built = build(&mut renderer, &tree(20), None);
        renderer.layout(built.root, 400.0, 800.0);
        built.clamp_scroll_offsets(&renderer, &mut offsets);
        assert_eq!(offsets["list"], (180.0, 0.0));
    }

    #[test]
    fn disabled_inputs_cannot_capture_a_gesture_or_focus() {
        let mut renderer = test_renderer();
        for attrs in [r#""checked":true"#, r#""min":0,"max":100,"step":1,"value":20"#, r#""input_type":"text","value":"abc""#] {
            let tree = RawNode::from_json(&format!(r#"{{"tag":"input","id":"control","attrs":{{"disabled":true,{attrs}}}}}"#)).unwrap();
            let built = build(&mut renderer, &tree, None);
            assert!(!built.is_text_field("control"));
            assert!(built.control("control").is_none());
        }
    }

    /// Reproduces, and locks in the fix for, a real bug found on a physical device
    /// this session: `view()`'s root widget (`Column("flex-1 ...", ...)`, straight
    /// from plan1.md §8's reference app) rendered collapsed into the top-left corner
    /// instead of filling the screen. Root cause: `flex-1` (`flex-grow`) only does
    /// anything inside a flex *parent*, and a `RawNode` tree's root has none of its
    /// own — the WebView backend never shows this because
    /// `phase_1/webruntime/index.html`'s `#zkite-root` CSS wrapper supplies one for
    /// free; `build`'s synthetic `flex flex-col w-full h-full` wrapper is this
    /// crate's equivalent. See `build`'s doc comment.
    #[test]
    fn build_wraps_the_root_so_flex_1_fills_the_viewport() {
        let mut r = test_renderer();
        let app_root = RawNode {
            tag: "div".into(),
            id: Some("r".into()),
            classes: "flex flex-col flex-1 items-center justify-center bg-slate-950".into(),
            events: Vec::new(),
            attrs: serde_json::Map::new(),
            children: vec![RawNode {
                tag: "#text".into(),
                id: None,
                classes: String::new(),
                events: Vec::new(),
                attrs: serde_json::Map::new(),
                children: Vec::new(),
                text: Some("hi".into()),
            }],
            text: None,
        };

        let built = build(&mut r, &app_root, None);
        r.layout(built.root, 400.0, 800.0);
        let nodes = r.positioned(built.root, &Default::default());

        // The synthetic wrapper itself must be the full viewport.
        let wrapper = &nodes[0];
        assert_eq!(wrapper.rect, (0.0, 0.0, 400.0, 800.0));
        // And the app's own root, now a flex item of that wrapper, must actually
        // have grown to fill it too — not just sit sized-to-content in the corner.
        let app = &nodes[1];
        assert_eq!(app.rect, (0.0, 0.0, 400.0, 800.0));
    }

    // ------------------------------------------------ §47 declaration collection

    /// A node carrying `anim` in its attrs, with children, for the tests below.
    fn node_with_anim(id: &str, anim: serde_json::Value) -> RawNode {
        let mut attrs = serde_json::Map::new();
        attrs.insert("anim".into(), anim);
        RawNode {
            tag: "div".into(),
            id: Some(id.into()),
            classes: "w-[40px] h-[20px]".into(),
            events: Vec::new(),
            attrs,
            children: Vec::new(),
            text: None,
        }
    }

    fn root_of(children: Vec<RawNode>) -> RawNode {
        RawNode {
            tag: "div".into(),
            id: Some("r".into()),
            classes: "flex flex-col w-full h-full".into(),
            events: Vec::new(),
            attrs: serde_json::Map::new(),
            children,
            text: None,
        }
    }

    /// A declaration reaches the driver paired with the taffy id it applies to — the
    /// two facts that only exist together here, which is why collection happens
    /// inside `build`'s walk rather than in a second pass over the `RawNode` tree.
    #[test]
    fn a_declared_animation_is_collected_with_its_taffy_node() {
        let mut r = test_renderer();
        let root = root_of(vec![
            node_with_anim("a", serde_json::json!([{"ch":"opacity","to":1,"from":0,"ms":200}])),
            // The control, in the same tree: an ordinary node contributes nothing.
            RawNode {
                tag: "div".into(),
                id: Some("b".into()),
                classes: "w-[40px] h-[20px]".into(),
                events: Vec::new(),
                attrs: serde_json::Map::new(),
                children: Vec::new(),
                text: None,
            },
        ]);
        let built = build(&mut r, &root, None);

        assert_eq!(built.animations().len(), 1, "exactly the node that declared one");
        let decl = &built.animations()[0];
        assert_eq!(&*decl.zkite_id, "a");
        assert_eq!(decl.specs.len(), 1);
        assert_eq!(decl.specs[0].channel, zkite_renderer::Channel::Opacity);
        // The taffy id must be the node's own, so `animate` lands on the right box.
        assert_eq!(built.zkite_id(decl.node), Some("a"));
        assert!(built.animation_errors().is_empty());

        // …and a tree with nothing declared collects nothing at all, which is the
        // condition the whole per-frame path is guarded on.
        let mut r2 = test_renderer();
        let plain = build(&mut r2, &root_of(vec![]), None);
        assert!(plain.animations().is_empty());
    }

    /// **A broken declaration must not take the app down with it.** The parse is
    /// strict on purpose (an unknown channel is an error, not a silent drop), and the
    /// failure mode that strictness could buy is far worse than the one it fixes: a
    /// single typo blanking the whole screen. So the error is collected and every
    /// other node — including a *valid* animation elsewhere in the same tree — still
    /// builds and still runs.
    #[test]
    fn a_malformed_declaration_is_reported_and_the_rest_of_the_tree_still_builds() {
        let mut r = test_renderer();
        let root = root_of(vec![
            node_with_anim("bad", serde_json::json!([{"ch":"rotate","to":90,"ms":200}])),
            node_with_anim("good", serde_json::json!([{"ch":"translate_x","to":8,"from":0,"ms":200}])),
        ]);
        let built = build(&mut r, &root, None);

        assert_eq!(built.animation_errors().len(), 1);
        let e = &built.animation_errors()[0];
        assert!(e.contains("node bad"), "the message must name the node: {e}");
        assert!(e.contains("not painted by the native renderer"), "{e}");

        // The valid one is unaffected…
        assert_eq!(built.animations().len(), 1);
        assert_eq!(&*built.animations()[0].zkite_id, "good");
        // …and the tree lays out normally, which is the claim that matters.
        r.layout(built.root, 400.0, 800.0);
        let nodes = r.positioned(built.root, &Default::default());
        assert_eq!(nodes.len(), 4, "wrapper + root + two children");
        assert!(built.zkite_id(nodes[2].node).is_some());
    }

    /// The `from`-defaulting contract, end to end: an unstated `from` resolves
    /// against the node's *own* class string, through the same `twcore` resolution
    /// the paint path uses. This is why `from` is optional on the wire at all —
    /// Python cannot compute this value, because it depends on `twcore`, on
    /// inheritance, and on ancestry `zkite_py` does not see (§18.4).
    #[test]
    fn an_unstated_from_resolves_against_the_nodes_own_resolved_class_value() {
        let mut r = test_renderer();
        let mut faded = node_with_anim("f", serde_json::json!([{"ch":"opacity","to":1,"ms":200}]));
        faded.classes = "w-[40px] h-[20px] opacity-25".into();
        let built = build(&mut r, &root_of(vec![faded]), None);

        let decl = &built.animations()[0];
        assert_eq!(decl.specs[0].from, None, "the wire said nothing");
        let class_value = r.channel_value(decl.node, zkite_renderer::Channel::Opacity).unwrap();
        assert_eq!(class_value, 0.25, "…and the class string says 0.25");
        assert_eq!(decl.specs[0].track(class_value).from, 0.25);
    }

    /// End-to-end against real `zkite_py` JSON (not a hand-typed fixture) — exercises
    /// the exact contract `phase_1/devshell/src/runtime.py`'s `_zkite_setup_tree`
    /// actually returns for the plan1.md §8 reference app, via the same `pyo3-ffi`
    /// embedding `embed.rs` uses. Needs a Python 3 on `PATH` with `zkite_py` importable
    /// — true for this dev box (see phase_1/zkite_py's own test suite), not assumed on
    /// every future CI runner; see ../PLAN.md §5 if this ever needs skipping.
    #[test]
    fn builds_the_real_counter_app_and_dispatches_a_tap() {
        let _py = lock_python();
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let zkite_pkg_dir = repo.join("phase_1/zkite_py/src");
        let app_dir = std::env::temp_dir().join(format!("zkite_pybridge_test_{}", std::process::id()));
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(
            app_dir.join("main.py"),
            r#"
from zkite import Column, Text, Button, store

@store
class Counter:
    n: int = 0

def view(c: Counter):
    return Column("flex-1 items-center justify-center gap-6 bg-slate-950",
        Text("text-7xl font-bold text-white tabular-nums", f"{c.n}"),
        Button("px-6 py-3 rounded-2xl bg-indigo-500 text-white font-medium",
               "Increment", on_tap=lambda: c.update(n=c.n + 1)),
    )
"#,
        )
        .unwrap();

        // `Py_InitializeFromConfig` needs a real prefix on disk; the host's system
        // Python is close enough for this test's purposes (it never touches
        // Android-specific paths) — mirrors how `phase_0`'s own probes work against
        // the host interpreter before anything cross-compiles.
        let prefix = python_prefix();

        let logs = std::sync::Mutex::new(Vec::<String>::new());
        let log = |line: &str| logs.lock().unwrap().push(line.to_string());

        let json = embed::setup(&prefix, zkite_pkg_dir.to_str().unwrap(), app_dir.to_str().unwrap(), &log)
            .expect("setup should succeed against a valid app dir");
        let root = RawNode::from_json(&json).expect("valid JSON");

        let mut renderer = test_renderer();
        let built = build(&mut renderer, &root, None);
        renderer.layout(built.root, 400.0, 800.0);
        let nodes = renderer.positioned(built.root, &Default::default());
        assert!(!nodes.is_empty());

        // Find the button's zkite id via the event map (mirrors how the render thread
        // will do it after a real `hit_test`), then dispatch a tap through it exactly
        // as a real touch would.
        let button_id = built
            .node_ids
            .values()
            .find(|id| built.handles(id, "tap"))
            .expect("Button's tap handler should be in the event map")
            .clone();

        let json2 = embed::dispatch(&button_id, "tap", "", &log).expect("tap should be handled");
        let root2 = RawNode::from_json(&json2).unwrap();
        let built2 = build(&mut renderer, &root2, None);
        renderer.layout(built2.root, 400.0, 800.0);
        let nodes2 = renderer.positioned(built2.root, &Default::default());
        let count_text = nodes2.iter().find_map(|n| n.text.as_ref()).is_some();
        assert!(count_text, "the counter's text node should still be present after a tap");

        std::fs::remove_dir_all(&app_dir).ok();
    }

    /// **The seam that can silently diverge, driven end to end** (../PLAN.md §47).
    ///
    /// `zkite.Anim` writes the `anim` attribute and `anim.rs` reads it, and they are in
    /// different languages in different directories with no shared schema. Every test
    /// either side of this boundary can pass while the two disagree about a key name,
    /// a unit, or whether an omitted field means "zero" or "unstated" — which is
    /// exactly the shape of §26.2's payload bug (three widgets, three sessions) and of
    /// §15.1's whole divergence category.
    ///
    /// So this runs **real `zkite_py`** through the real interpreter, takes the JSON it
    /// actually emits, and asserts on what the real parser makes of it. Nothing here
    /// is hand-typed.
    #[test]
    fn a_real_zkite_py_anim_declaration_parses_into_the_track_it_describes() {
        let _py = lock_python();
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let zkite_pkg_dir = repo.join("phase_1/zkite_py/src");
        let app_dir = std::env::temp_dir().join(format!("zkite_anim_test_{}", std::process::id()));
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(
            app_dir.join("main.py"),
            r#"
from zkite import Anim, Column, Text, store

# Deliberately not `S`, `Counter` or `A11yCounter` — `@store` keys its one live
# instance by module-qualified name and every test app here loads as module `main`,
# so a shared name means a shared instance across tests (../PLAN.md §34.6). This
# collided with `a_touch_on_a_synthesized_thumb_resolves_to_the_control_itself`'s
# own `S` the first time it was written, deterministically, in exactly the way that
# section documents — which is the **second** time this trap has fired.
@store
class AnimDemo:
    n: int = 0

def view(s: AnimDemo):
    return Column("flex-1 opacity-25",
        Text("text-base", "hello"),
        anim=[Anim.enter(opacity=(0, 1), translate_y=(16, 0), ms=220, ease="out"),
              Anim.change(translate_x=8, spring=(180, 26))],
        on_animend=lambda e: None,
    )
"#,
        )
        .unwrap();

        let prefix = python_prefix();
        let logs = std::sync::Mutex::new(Vec::<String>::new());
        let log = |line: &str| logs.lock().unwrap().push(line.to_string());
        let json = embed::setup(&prefix, zkite_pkg_dir.to_str().unwrap(), app_dir.to_str().unwrap(), &log)
            .expect("setup should succeed");
        let root = RawNode::from_json(&json).expect("valid JSON");

        let mut renderer = test_renderer();
        let built = build(&mut renderer, &root, None);

        assert!(
            built.animation_errors().is_empty(),
            "real zkite_py output must parse cleanly: {:?}",
            built.animation_errors()
        );
        assert_eq!(built.animations().len(), 1, "one node declared, three channels on it");
        let decl = &built.animations()[0];
        assert_eq!(decl.specs.len(), 3);

        use zkite_renderer::Channel;
        let by_channel = |c: Channel| decl.specs.iter().find(|s| s.channel == c).unwrap();

        // The eased pair: units survive (220 *milliseconds*, not seconds), the named
        // easing resolves to CSS's own control points, and `enter` is the trigger the
        // Python side omitted from the wire because it is the default.
        let fade = by_channel(Channel::Opacity);
        assert_eq!(fade.trigger, Trigger::Enter);
        assert_eq!(fade.from, Some(0.0));
        assert_eq!(fade.to, 1.0);
        assert_eq!(
            fade.curve,
            SpecCurve::Eased { duration: std::time::Duration::from_millis(220), easing: zkite_anim::Easing::EASE_OUT }
        );
        let slide = by_channel(Channel::TranslateY);
        assert_eq!((slide.from, slide.to), (Some(16.0), 0.0));

        // The spring, on a different trigger, from the second `Anim` in the list.
        let follow = by_channel(Channel::TranslateX);
        assert_eq!(follow.trigger, Trigger::Change);
        assert_eq!(follow.from, None, "no `from` was written, so none must arrive");
        assert_eq!(
            follow.curve,
            SpecCurve::Spring(zkite_anim::Spring { stiffness: 180.0, damping: 26.0, mass: 1.0 })
        );
        // …and an unstated `from` resolves against the node's own class string, which
        // is `opacity-25` on this widget. This is the round trip that cannot be
        // checked on either side alone.
        assert_eq!(renderer.channel_value(decl.node, Channel::Opacity), Some(0.25));

        // The completion event is registered under the name the render thread
        // dispatches — one string, spelled in `zkite_py`, in `render_thread.rs`'s
        // `ANIM_END_EVENT`, and here.
        assert!(built.handles(&decl.zkite_id, "animend"), "on_animend must reach the event map");

        // And the sampled track actually moves, so this is a runnable animation and
        // not merely a well-formed record.
        let t = fade.track(0.25);
        let mid = t.sample(std::time::Duration::from_millis(110)).value;
        assert!(mid > 0.0 && mid < 1.0, "the fade did not move: {mid}");
        assert_eq!(t.sample(std::time::Duration::from_millis(220)).value, 1.0);

        std::fs::remove_dir_all(&app_dir).ok();
    }

    /// ../PLAN.md §11 (virtualised lists), end to end through real CPython: a
    /// 10,000-row `List` must materialise a screenful, must not re-run `view()` for
    /// a scroll that doesn't move its window, and — the property everything rests on
    /// — must present the renderer with the same scrollable range no matter which
    /// window is materialised.
    ///
    /// Deliberately checks `max_scroll_y` rather than just counting nodes: node
    /// counts prove the list is *virtualised*, but only an unchanged scrollable
    /// range proves it's virtualised *correctly*, i.e. that `zkite_renderer` and the
    /// gesture layer never have to know a list is windowed at all.
    #[test]
    fn virtualised_list_windows_without_changing_the_scrollable_range() {
        let _py = lock_python();
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let zkite_pkg_dir = repo.join("phase_1/zkite_py/src");
        let app_dir = std::env::temp_dir().join(format!("zkite_pybridge_list_test_{}", std::process::id()));
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(
            app_dir.join("main.py"),
            r#"
from zkite import List, Row, Text, store

@store
class S:
    n: int = 0

def view(s: S):
    return List("flex-1 min-h-0", count=10_000, item_height=50, overscan=4,
                builder=lambda i: Row("shrink-0 items-center px-4",
                                      Text("text-white", f"Item {i}")))
"#,
        )
        .unwrap();

        let prefix = python_prefix();
        let logs = std::sync::Mutex::new(Vec::<String>::new());
        let log = |line: &str| logs.lock().unwrap().push(line.to_string());

        let json = embed::setup(&prefix, zkite_pkg_dir.to_str().unwrap(), app_dir.to_str().unwrap(), &log)
            .expect("setup should succeed");
        let root = RawNode::from_json(&json).expect("valid JSON");

        // A 10k-row list must not ship 10k nodes across the bridge at all.
        fn count_nodes(n: &RawNode) -> usize {
            1 + n.children.iter().map(count_nodes).sum::<usize>()
        }
        let n_nodes = count_nodes(&root);
        assert!(n_nodes < 200, "a 10,000-row list produced {n_nodes} nodes; it should be a screenful");

        let mut renderer = test_renderer();
        let built = build(&mut renderer, &root, None);
        renderer.layout(built.root, 400.0, 800.0);
        // The list is `view()`'s own root widget, so it has zkite id "r".
        let list_node = built
            .node_ids
            .iter()
            .find(|(_, id)| id.as_ref() == "r")
            .map(|(node, _)| *node)
            .expect("the List should be in the node map");
        let range_before = renderer.max_scroll_y(list_node);
        assert!(range_before > 400_000.0, "10,000 x 50px minus a 800px viewport, got {range_before}");

        // Reporting the real viewport height (800) instead of the bootstrap one
        // (1400) does **not** re-render, as of ../../phase_3/PLAN.md §40. The
        // bootstrap window is deliberately a *superset* of the real first one — see
        // `BOOTSTRAP_VIEWPORT_PX` — so every row the 800px viewport can see is
        // already built, and rebuilding would only discard rows. Before §40 this
        // returned a tree, because the window was compared for equality rather than
        // for coverage: a full `view()` + JSON round trip on the first scroll frame
        // of every app containing a `List`, to build strictly fewer rows.
        assert!(
            embed::scroll("r", 0.0, 800.0, &log).is_none(),
            "a viewport smaller than the bootstrap is already covered; rebuilding only throws rows away"
        );

        // A one-pixel scroll cannot change which rows are needed: no re-render.
        assert!(embed::scroll("r", 1.0, 800.0, &log).is_none(), "a sub-row scroll must not re-run view()");
        assert!(embed::scroll("r", 49.0, 800.0, &log).is_none(), "still inside the same row");

        // Crossing far enough does.
        let scrolled = embed::scroll("r", 5000.0, 800.0, &log).expect("a 100-row scroll must move the window");
        let root_far = RawNode::from_json(&scrolled).unwrap();

        fn has_text(n: &RawNode, needle: &str) -> bool {
            n.text.as_deref() == Some(needle) || n.children.iter().any(|c| has_text(c, needle))
        }
        assert!(has_text(&root, "Item 0"), "the unscrolled window should hold row 0");
        assert!(!has_text(&root, "Item 100"), "row 100 is far off-screen and must not be built");
        assert!(has_text(&root_far, "Item 100"), "the scrolled window should hold row 100");
        assert!(!has_text(&root_far, "Item 0"), "row 0 is now far off-screen and must not be built");

        // The load-bearing invariant: a completely different window, identical
        // scrollable range, because the spacers preserve total content height.
        let built_far = build(&mut renderer, &root_far, None);
        renderer.layout(built_far.root, 400.0, 800.0);
        let list_node_far = built_far
            .node_ids
            .iter()
            .find(|(_, id)| id.as_ref() == "r")
            .map(|(node, _)| *node)
            .expect("the List should still be in the node map");
        let range_after = renderer.max_scroll_y(list_node_far);
        assert_eq!(
            range_before, range_after,
            "materialising a different window must not change how far the list can scroll"
        );

        // §40's contract, end to end through real CPython rather than only in the
        // `zkite_py` host tests. The window is currently centred on row 100 with
        // `overscan=4` rows of slack below it. Scrolling *into* that slack must not
        // re-run `view()`; scrolling one row past it must. Before §40 the first of
        // these rebuilt too — `compute_window`'s `first` moves on every row boundary
        // no matter how much overscan it carries — which is what made the §39.5
        // `sync` spike fire on ~35% of a drag's frames instead of ~9%.
        assert!(
            embed::scroll("r", 5000.0 + 4.0 * 50.0, 800.0, &log).is_none(),
            "four rows is exactly the overscan: every visible row is already materialised"
        );
        assert!(
            embed::scroll("r", 5000.0 + 5.0 * 50.0, 800.0, &log).is_some(),
            "one row past the overscan, the bottom visible row is not built and must be"
        );

        std::fs::remove_dir_all(&app_dir).ok();
    }

    /// The mechanism ../PLAN.md §43.5 names, checked before anything is built on it:
    /// **a scroll container's own viewport height does not change when its window
    /// does.**
    ///
    /// §43.5 proposes removing `relayout` — the second `build`+`layout`+`positioned`
    /// that `redraw()` runs because `sync_viewports` changed the tree — by reporting
    /// viewports *before* the first layout, using the height the previous frame
    /// measured. That is only sound if the height is window-invariant, and only
    /// *useful* if it is invariant in the steady state rather than merely eventually.
    /// This document's rule is that a named mitigation is a hypothesis until the
    /// mechanism is measured (§40.1, §42.3, §43), so it is measured here.
    ///
    /// The sibling test above pins the neighbouring invariant — `max_scroll_y`, the
    /// *content* height — which is what §11.2 actually states and what the spacers
    /// exist to preserve. That one does not imply this one: content height is a
    /// property of the list's children, viewport height a property of how its
    /// *parent* sized it, and a `flex-1` list in a column with a `shrink-0` header is
    /// exactly the shape where a header that resized would move it.
    ///
    /// So the fixture is `gpu_harness/demo_app/cmp_list_1k.py`'s shape rather than a
    /// bare `List` — same header, same `flex-1`, same row height — because that is the
    /// app every device measurement from §40 onwards was taken on, and a mechanism
    /// checked on a different tree would not speak to those numbers.
    #[test]
    fn a_lists_viewport_height_does_not_change_when_its_window_does() {
        let _py = lock_python();
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let zkite_pkg_dir = repo.join("phase_1/zkite_py/src");
        let app_dir = std::env::temp_dir().join(format!("zkite_pybridge_viewport_test_{}", std::process::id()));
        std::fs::create_dir_all(&app_dir).unwrap();
        // `r##"…"##`, not `r#"…"#`: the fixture's own `f"#{i}"` contains `"#`, which
        // closes a single-hash raw string in the middle of the Python source.
        std::fs::write(
            app_dir.join("main.py"),
            r##"
from zkite import Column, List, Row, Text, store

ROWS = 1_000
ROW_HEIGHT = 56


@store
class DemoState:
    n: int = 0


def row(i: int):
    bg = "bg-slate-900" if i % 2 == 0 else "bg-slate-950"
    return Row(
        f"items-center justify-between px-4 h-full {bg}",
        Text("text-white", f"Item {i}"),
        Text("text-slate-500 text-sm", f"#{i}"),
    )


def view(s: DemoState):
    return Column(
        "flex-1 bg-slate-950 min-h-0",
        Column(
            "shrink-0 items-center justify-center gap-2 py-4",
            Text("text-2xl font-bold text-white tabular-nums", f"{ROWS} rows (virtualised) · {s.n}"),
        ),
        List("flex-1", count=ROWS, item_height=ROW_HEIGHT, builder=row),
    )
"##,
        )
        .unwrap();

        let prefix = python_prefix();
        let logs = std::sync::Mutex::new(Vec::<String>::new());
        let log = |line: &str| logs.lock().unwrap().push(line.to_string());

        let json = embed::setup(&prefix, zkite_pkg_dir.to_str().unwrap(), app_dir.to_str().unwrap(), &log)
            .expect("setup should succeed");
        let root = RawNode::from_json(&json).expect("valid JSON");
        let mut renderer = test_renderer();

        // The device surface, in logical px at density 2.5 — `1080x2223` / 2.5. The
        // absolute numbers do not matter to the invariant, but keeping them the real
        // ones means a height printed by this test is directly comparable to one
        // logged by `sync_viewports` on the phone.
        let (w, h) = (432.0, 889.2);

        // Exactly what `sync_viewports` does to find what to report: walk the
        // positioned nodes, keep the ones the renderer calls y-scrollable, and read
        // `rect.3`. Deliberately not "find the List by zkite id" — the render thread
        // has no such shortcut, and a helper that disagreed with it would be checking
        // something other than the mechanism.
        fn scrollable_viewports(renderer: &mut Renderer, root: &RawNode, w: f32, h: f32) -> Vec<(String, f32)> {
            let built = build(renderer, root, None);
            renderer.layout(built.root, w, h);
            let nodes = renderer.positioned(built.root, &Default::default());
            let mut out: Vec<(String, f32)> = nodes
                .iter()
                .filter(|pn| renderer.is_scrollable_y(pn.node))
                .filter_map(|pn| built.zkite_id(pn.node).map(|id| (id.to_string(), pn.rect.3)))
                .collect();
            out.sort_by(|a, b| a.0.cmp(&b.0));
            out
        }

        let before = scrollable_viewports(&mut renderer, &root, w, h);
        assert_eq!(before.len(), 1, "the fixture should have exactly one y-scrollable node, got {before:?}");

        // A window a long way from the one above, and from both clamps — the same
        // move §40.1 records getting wrong by measuring from the top of the list,
        // where the `first = max(0, ...)` clamp hides the real behaviour.
        let scrolled = embed::scroll(&before[0].0, 5000.0, before[0].1, &log)
            .expect("a 5000px scroll must move the window");
        let root_far = RawNode::from_json(&scrolled).unwrap();
        let after = scrollable_viewports(&mut renderer, &root_far, w, h);

        assert_eq!(
            before, after,
            "a `List`'s viewport height must not depend on which rows are materialised; \
             §43.5's pre-layout viewport sync is unsound if it does"
        );

        // **Two controls, because an equality assertion has two ways to pass for the
        // wrong reason**, and §37's `is_stale()` control is the precedent for saying
        // so out loud: it passed, and passed vacuously.
        //
        // First — the window really did move. An `embed::scroll` that returned a tree
        // identical to the first one would satisfy the equality above trivially.
        fn has_text(n: &RawNode, needle: &str) -> bool {
            n.text.as_deref() == Some(needle) || n.children.iter().any(|c| has_text(c, needle))
        }
        assert!(has_text(&root, "Item 0") && !has_text(&root_far, "Item 0"), "the window did not actually move");

        // Second — the height this reads is a real layout output and not something
        // inert. A helper that returned a constant, or that read a field layout never
        // fills, would report "invariant" for every tree ever laid out and could
        // never fail. Laying the *same* tree out on a shorter surface must move it,
        // and by the whole difference, since the list is the only `flex-1` child.
        let shorter = scrollable_viewports(&mut renderer, &root_far, w, h - 100.0);
        assert_eq!(shorter.len(), 1);
        assert!(
            (after[0].1 - shorter[0].1 - 100.0).abs() < 0.01,
            "viewport height must track the surface it is laid out in: {} at {h}px vs {} at {}px",
            after[0].1,
            shorter[0].1,
            h - 100.0
        );

        std::fs::remove_dir_all(&app_dir).ok();
    }

    fn python_prefix() -> std::path::PathBuf {
        let out = std::process::Command::new("python3")
            .arg("-c")
            .arg("import sys; print(sys.base_prefix)")
            .output()
            .expect("python3 must be on PATH for this test");
        std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
    }

    /// ../../phase_3/PLAN.md §5 item 2 (AccessKit) — end-to-end against the same real
    /// `zkite_py` counter app `builds_the_real_counter_app_and_dispatches_a_tap` above
    /// already exercises, checking that `accessibility_tree_update` produces a real,
    /// usable tree: the button is clickable with a label from its bare-string child,
    /// the counter digit is a plain labeled node, and every node's bounds actually
    /// came from `positioned()` (not left zeroed).
    #[test]
    fn accessibility_tree_has_labeled_clickable_button_and_text() {
        let _py = lock_python();
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let zkite_pkg_dir = repo.join("phase_1/zkite_py/src");
        let app_dir = std::env::temp_dir().join(format!("zkite_pybridge_a11y_test_{}", std::process::id()));
        std::fs::create_dir_all(&app_dir).unwrap();
        // The store class is deliberately NOT called `Counter`. `@store` keys its one
        // live instance by module-qualified name, and every test here loads its app as
        // module `main` — so a `Counter` here would share `"main.Counter"` with
        // `builds_the_real_counter_app_and_dispatches_a_tap`, which *taps the button*
        // and leaves `n = 1` behind. This test asserts the digit is `"0"`, so it fails
        // whenever it happens to run second. `lock_python()` serialises these tests but
        // never resets the registry, so it never fixed this — the ordering just used to
        // fall the lucky way (0 failures in 8 runs). Enabling the bytecode cache
        // (../PLAN.md §17.1) changed the timing and made it fire ~1 run in 6, which is
        // how a latent test-isolation bug surfaced. A distinct name is the fix: a
        // distinct registry key cannot inherit another test's state.
        std::fs::write(
            app_dir.join("main.py"),
            r#"
from zkite import Column, Text, Button, store

@store
class A11yCounter:
    n: int = 0

def view(c: A11yCounter):
    return Column("flex-1 items-center justify-center gap-6 bg-slate-950",
        Text("text-7xl font-bold text-white tabular-nums", f"{c.n}"),
        Button("px-6 py-3 rounded-2xl bg-indigo-500 text-white font-medium",
               "Increment", on_tap=lambda: c.update(n=c.n + 1)),
    )
"#,
        )
        .unwrap();

        let prefix = python_prefix();
        let logs = std::sync::Mutex::new(Vec::<String>::new());
        let log = |line: &str| logs.lock().unwrap().push(line.to_string());
        let json = embed::setup(&prefix, zkite_pkg_dir.to_str().unwrap(), app_dir.to_str().unwrap(), &log)
            .expect("setup should succeed against a valid app dir");
        let root = RawNode::from_json(&json).expect("valid JSON");

        let mut renderer = test_renderer();
        let built = build(&mut renderer, &root, None);
        renderer.layout(built.root, 400.0, 800.0);
        let nodes = renderer.positioned(built.root, &Default::default());

        let button_zkite_id =
            built.node_ids.values().find(|id| built.handles(id, "tap")).expect("button should be in the event map");
        let update = built.accessibility_tree_update(&nodes, None, 1.0);

        let button_a11y_id = accesskit_id(button_zkite_id);
        let (_, button_node) =
            update.nodes.iter().find(|(id, _)| *id == button_a11y_id).expect("button must be in the a11y tree");
        assert_eq!(button_node.role(), accesskit::Role::Button);
        assert_eq!(button_node.label(), Some("Increment"));
        assert!(button_node.supports_action(accesskit::Action::Click), "a tap-handled node must support Click");
        let bounds = button_node.bounds().expect("bounds must be stamped from positioned()");
        assert!(bounds.x1 > bounds.x0 && bounds.y1 > bounds.y0, "button bounds must be non-empty: {bounds:?}");

        // Accessibility bounds are device pixels, layout is dp (../PLAN.md §13). The
        // same layout at density 2.5 must report bounds exactly 2.5x larger, or
        // TalkBack's focus rectangle lands at a fraction of the widget it names —
        // a bug that is invisible in a screenshot of the app itself, since the app
        // renders correctly either way. Checked against the *same* `nodes` so this
        // can only be measuring the scaling, not a re-layout.
        let scaled = built.accessibility_tree_update(&nodes, None, 2.5);
        let (_, scaled_button) =
            scaled.nodes.iter().find(|(id, _)| *id == button_a11y_id).expect("button must be in the a11y tree");
        let sb = scaled_button.bounds().expect("bounds must be stamped from positioned()");
        for (got, want) in [(sb.x0, bounds.x0), (sb.y0, bounds.y0), (sb.x1, bounds.x1), (sb.y1, bounds.y1)] {
            assert!((got - want * 2.5).abs() < 1e-3, "a11y bounds must scale with density: got {got}, want {}", want * 2.5);
        }

        // The counter digit ("0") is a bare #text child of a plain `Text` (span) —
        // it must fold into that span's own label, not become a separate node.
        let counter_node = update
            .nodes
            .iter()
            .map(|(_, n)| n)
            .find(|n| n.role() == accesskit::Role::Label && n.label().as_deref() == Some("0"))
            .expect("the counter digit should be exposed as a labeled node");
        assert!(!counter_node.supports_action(accesskit::Action::Click), "plain text isn't clickable");

        std::fs::remove_dir_all(&app_dir).ok();
    }

    /// A *labelled* `Checkbox` (../PLAN.md §32) — the tree `widgets.py` actually
    /// emits for `Checkbox("text-sm", "Wi-Fi", checked=True, on_change=...)`, kept as
    /// the literal wire JSON so this fixture states the shape rather than
    /// reconstructing it. Its `zkite_py` twin is
    /// `LabelledControlTests.test_the_container_carries_the_role_and_the_state_on_both_paths`.
    fn labelled_checkbox() -> RawNode {
        RawNode::from_json(
            r##"{"tag":"span","id":"c.0","classes":"text-sm inline-flex flex-row items-center",
                "events":["tap"],"attrs":{"role":"checkbox","checked":true},
                "children":[
                  {"tag":"input","id":"c.0.box","classes":"appearance-none",
                   "events":[],"attrs":{"role":"checkbox","checked":true,"aria_hidden":true},
                   "children":[{"tag":"svg","id":"c.0.box.tick","classes":"","events":[],
                                "attrs":{"icon_name":"check"},"children":[]}]},
                  {"tag":"#text","text":"Wi-Fi"}]}"##,
        )
        .expect("fixture must parse")
    }

    /// ../PLAN.md §32. The container is the control; the box is presentational. Three
    /// separate facts have to hold together for a labelled control to behave as one
    /// widget, and each has its own negative control (§32.4).
    #[test]
    fn a_labelled_control_is_one_checkbox_not_a_checkbox_next_to_some_text() {
        let node = labelled_checkbox();
        let box_node = &node.children[0];

        // 1. The role is read off a `span`. Before §32 the role was only consulted
        //    inside the `input` arm, so this returned `Role::Label` — a labelled
        //    control would have been announced as a piece of text.
        assert_eq!(a11y_role(&node), accesskit::Role::CheckBox);

        // 2. The box is transparent, so there is exactly one checkbox in the tree
        //    rather than two nested ones.
        assert_eq!(a11y_role(box_node), accesskit::Role::GenericContainer);

        // 3. The box is not a dispatch target. Without this the render thread's
        //    `control_at` walk would find it and synthesize a `change` *as well as*
        //    the container's `tap` — the double dispatch this feature removes.
        assert_eq!(control_of(box_node), None);
        assert_eq!(control_of(&node), None, "the container dispatches a plain tap, not a change");
    }

    /// ../PLAN.md §35 — an `Image` is named by its `alt`, and an `alt=""` image stays
    /// deliberately unnamed.
    ///
    /// The empty half is the one worth a test. `alt=""` is HTML's "decorative, skip
    /// me", and `Image`'s `_tree_attrs` emits `alt` unconditionally — so it is `""`
    /// for every image whose author did not write one. Without the `filter`, every
    /// such image would announce itself with an empty name, which reads to a screen
    /// reader user as an unlabelled control rather than as something to pass over.
    #[test]
    fn an_image_is_named_by_its_alt_and_an_empty_alt_stays_unnamed() {
        let ctx = BuildCtx::default();
        let described = RawNode::from_json(
            r##"{"tag":"img","id":"r.1","classes":"w-32 h-24 object-cover",
                 "attrs":{"src":"logo.png","alt":"The ZKite logo"},"children":[]}"##,
        )
        .expect("fixture must parse");
        assert_eq!(
            build_a11y_node(&described, &[], "", false, &ctx).label().as_deref(),
            Some("The ZKite logo")
        );

        let decorative = RawNode::from_json(
            r##"{"tag":"img","id":"r.2","classes":"w-8 h-8","attrs":{"src":"sep.png","alt":""},
                 "children":[]}"##,
        )
        .expect("fixture must parse");
        assert_eq!(build_a11y_node(&decorative, &[], "", false, &ctx).label(), None);
    }

    /// An `img` node builds the image leaf, and — the property every existing app
    /// depends on — occupies exactly the box it did when `Image` painted nothing.
    ///
    /// Checked against a `div` carrying the identical class string rather than
    /// against literal numbers, so it stays true if the spacing scale is ever
    /// corrected underneath it (../PLAN.md §9.6 predicts exactly that).
    #[test]
    fn an_img_node_builds_an_image_leaf_without_moving_its_box() {
        fn box_of(json: &str) -> (f32, f32, f32, f32) {
            let node = RawNode::from_json(json).expect("fixture must parse");
            let mut renderer = test_renderer();
            let mut ctx = BuildCtx::default();
            let root = build_node(&mut renderer, &node, &mut ctx);
            renderer.layout(root, 400.0, 400.0);
            let nodes = renderer.positioned(root, &Default::default());
            nodes.iter().find(|n| n.node == root).expect("root is positioned").rect
        }
        let classes = "w-32 h-24 rounded-lg";
        let img = format!(
            r##"{{"tag":"img","id":"i","classes":"{classes}","attrs":{{"src":"logo.png","alt":"x"}},"children":[]}}"##
        );
        let div = format!(r##"{{"tag":"div","id":"d","classes":"{classes}","attrs":{{}},"children":[]}}"##);
        assert_eq!(box_of(&img), box_of(&div), "an img must not lay out differently from a div");
    }

    /// ../PLAN.md §33 — `Select`. The trigger, the panel and one option, each of which
    /// Android's own layer turns into something different.
    #[test]
    fn a_select_is_a_spinner_a_listbox_and_named_options() {
        let ctx = BuildCtx::default();

        // The trigger. Its label lives in a child `span` and a chevron in a sibling
        // `svg`, so `text_label_from_children` (direct `#text` only) finds nothing —
        // an unnamed control is what this arm exists to prevent.
        let trigger = RawNode::from_json(
            r##"{"tag":"div","id":"r.1","classes":"flex flex-row items-center justify-between",
                 "events":["tap"],"attrs":{"role":"combobox","expanded":true},
                 "children":[
                   {"tag":"span","id":"r.1.0","children":[{"tag":"#text","text":"India"}]},
                   {"tag":"svg","id":"r.1.1","attrs":{"icon_name":"chevron-down"},"children":[]}]}"##,
        )
        .expect("fixture must parse");
        let a11y = build_a11y_node(&trigger, &[], "", false, &ctx);
        assert_eq!(a11y_role(&trigger), accesskit::Role::ComboBox);
        assert_eq!(a11y.label().as_deref(), Some("India"));
        assert!(a11y.is_expanded().unwrap_or(false));
        assert!(a11y.supports_action(accesskit::Action::Click));
        // It is not a touch-driven control: the render thread dispatches a plain tap,
        // exactly as it does for a labelled checkbox's container.
        assert_eq!(control_of(&trigger), None);

        // The panel. Deliberately *not* named from its subtree — a list's accessible
        // name is not its items' text run together.
        let listbox = RawNode::from_json(
            r##"{"tag":"div","id":"L","attrs":{"role":"listbox"},
                 "children":[{"tag":"div","id":"o","attrs":{"role":"option","selected":true},
                              "children":[{"tag":"#text","text":"India"}]}]}"##,
        )
        .expect("fixture must parse");
        assert_eq!(a11y_role(&listbox), accesskit::Role::ListBox);
        assert_eq!(build_a11y_node(&listbox, &[], "", false, &ctx).label(), None);

        // One option, in both states. `selected` is the half that actually reaches
        // Android (`accesskit_android`'s `is_selected` -> `setSelected`).
        let option = &listbox.children[0];
        let opt_a11y = build_a11y_node(option, &[], "", false, &ctx);
        assert_eq!(a11y_role(option), accesskit::Role::ListBoxOption);
        assert_eq!(opt_a11y.label().as_deref(), Some("India"));
        assert_eq!(opt_a11y.is_selected(), Some(true));

        let unselected = RawNode::from_json(
            r##"{"tag":"div","id":"o2","attrs":{"role":"option","selected":false},
                 "children":[{"tag":"#text","text":"Kenya"}]}"##,
        )
        .expect("fixture must parse");
        assert_eq!(build_a11y_node(&unselected, &[], "", false, &ctx).is_selected(), Some(false));
    }

    /// An option drawn entirely by the app's own `option=` builder is still an option.
    /// Same argument §32.5 makes for `indicator=False`, one layer out: the framework
    /// keeps role, selected state and name; the app keeps the drawing.
    #[test]
    fn an_app_drawn_option_keeps_its_role_and_its_name() {
        let node = RawNode::from_json(
            r##"{"tag":"div","id":"r.0.opt.1","classes":"","events":["tap"],
                 "attrs":{"role":"option","selected":true},
                 "children":[{"tag":"div","id":"r.0.opt.1.0","classes":"border-2 border-indigo-500",
                              "children":[{"tag":"span","id":"x","classes":"text-lg",
                                           "children":[{"tag":"#text","text":"Beta"}]}]}]}"##,
        )
        .expect("fixture must parse");
        let a11y = build_a11y_node(&node, &[], "", false, &BuildCtx::default());
        assert_eq!(a11y_role(&node), accesskit::Role::ListBoxOption);
        // Deep, because with a builder the label is arbitrarily nested — a tappable
        // `Row` here would have been generic clickable text with no name at all.
        assert_eq!(a11y.label().as_deref(), Some("Beta"));
        assert_eq!(a11y.is_selected(), Some(true));
        assert!(a11y.supports_action(accesskit::Action::Click));
    }

    /// The accessible *name* — what makes TalkBack say "Wi-Fi, checkbox, checked"
    /// rather than announcing an unnamed checkbox and a loose label beside it. This is
    /// the a11y half of the feature and the half a screenshot cannot check.
    #[test]
    fn a_labelled_control_announces_its_label_and_its_state() {
        let node = labelled_checkbox();
        let ctx = BuildCtx::default();
        let a11y = build_a11y_node(&node, &[], "", false, &ctx);

        assert_eq!(a11y.label().as_deref(), Some("Wi-Fi"));
        assert_eq!(a11y.toggled(), Some(accesskit::Toggled::True));
        assert!(a11y.supports_action(accesskit::Action::Click), "a labelled control is activatable");

        // A label nested inside a styled `Text` is still the name — it is ordinary
        // content, so `Checkbox("", Text("font-bold", "High"), ...)` must work too.
        let nested = RawNode::from_json(
            r##"{"tag":"span","id":"r.1","attrs":{"role":"radio","checked":false},"events":["tap"],
                "children":[
                  {"tag":"input","id":"r.1.box","attrs":{"role":"radio","checked":false,"aria_hidden":true}},
                  {"tag":"span","id":"r.1.0","classes":"font-bold",
                   "children":[{"tag":"#text","text":"High"}]}]}"##,
        )
        .unwrap();
        assert_eq!(a11y_role(&nested), accesskit::Role::RadioButton);
        assert_eq!(build_a11y_node(&nested, &[], "", false, &ctx).label().as_deref(), Some("High"));

        // A *bare* control has no text descendants, so it gets no name — unchanged
        // from before §32, and the reason that path could not regress.
        let bare = RawNode::from_json(r#"{"tag":"input","id":"b","attrs":{"role":"checkbox","checked":false}}"#).unwrap();
        assert_eq!(build_a11y_node(&bare, &[], "", false, &ctx).label(), None);

        // `indicator=False` — an MCQ option drawn entirely by the caller's own
        // classes, with no box node at all. This is the shape whose whole point is
        // that it is still a *radio button* to a screen reader rather than the
        // generic clickable text a tappable `Row` would have produced.
        let mcq = RawNode::from_json(
            r##"{"tag":"span","id":"q.1","classes":"px-4 py-3 rounded-xl border-2",
                 "events":["tap"],"attrs":{"role":"radio","checked":true},
                 "children":[{"tag":"#text","text":"B. Mitochondria"}]}"##,
        )
        .unwrap();
        let mcq_a11y = build_a11y_node(&mcq, &[], "", false, &ctx);
        assert_eq!(a11y_role(&mcq), accesskit::Role::RadioButton);
        assert_eq!(mcq_a11y.label().as_deref(), Some("B. Mitochondria"));
        assert_eq!(mcq_a11y.toggled(), Some(accesskit::Toggled::True));
        assert!(mcq_a11y.supports_action(accesskit::Action::Click));
        assert_eq!(control_of(&mcq), None, "it dispatches a tap, like every labelled control");
    }

    /// Hand-built fixtures (no CPython embedding needed — fast, targeted) for the
    /// three `input`-tagged widgets that share one HTML tag and must be told apart by
    /// a second attribute (../PLAN.md §5 item 2; see `a11y_role`'s own doc comment).
    #[test]
    fn a11y_role_disambiguates_the_three_input_tagged_widgets() {
        let text_field = RawNode {
            tag: "input".into(),
            id: Some("f".into()),
            classes: String::new(),
            events: Vec::new(),
            attrs: serde_json::json!({"input_type": "password", "value": "", "placeholder": ""})
                .as_object()
                .unwrap()
                .clone(),
            children: Vec::new(),
            text: None,
        };
        assert_eq!(a11y_role(&text_field), accesskit::Role::PasswordInput);

        let switch = RawNode {
            tag: "input".into(),
            id: Some("s".into()),
            classes: String::new(),
            events: Vec::new(),
            attrs: serde_json::json!({"checked": true}).as_object().unwrap().clone(),
            children: Vec::new(),
            text: None,
        };
        assert_eq!(a11y_role(&switch), accesskit::Role::Switch);

        let slider = RawNode {
            tag: "input".into(),
            id: Some("sl".into()),
            classes: String::new(),
            events: Vec::new(),
            attrs: serde_json::json!({"min": 0.0, "max": 100.0, "step": 1.0, "value": 42.0})
                .as_object()
                .unwrap()
                .clone(),
            children: Vec::new(),
            text: None,
        };
        assert_eq!(a11y_role(&slider), accesskit::Role::Slider);
    }

    /// ../PLAN.md §30. `Switch`, `Checkbox` and `Radio` are all an `input` carrying
    /// `checked`, so `role` is the only thing that can tell them apart — on *both*
    /// sides, since a screen reader announcing "switch" for a radio button tells the
    /// user the wrong thing about what selecting it will do.
    ///
    /// The negative control is the pre-§30 behaviour: `a11y_role` returning
    /// `Role::Switch` for anything with `checked`. Under it, both new assertions fail.
    #[test]
    fn role_distinguishes_switch_from_checkbox_from_radio() {
        let node = |role: Option<&str>| {
            let mut attrs = serde_json::Map::new();
            attrs.insert("checked".into(), serde_json::Value::Bool(true));
            if let Some(r) = role {
                attrs.insert("role".into(), serde_json::Value::String(r.into()));
            }
            RawNode {
                tag: "input".into(),
                id: Some("c".into()),
                classes: String::new(),
                events: Vec::new(),
                attrs,
                children: Vec::new(),
                text: None,
            }
        };
        assert_eq!(a11y_role(&node(Some("switch"))), accesskit::Role::Switch);
        assert_eq!(a11y_role(&node(Some("checkbox"))), accesskit::Role::CheckBox);
        assert_eq!(a11y_role(&node(Some("radio"))), accesskit::Role::RadioButton);
        // A tree with no role at all predates the attribute; a switch is the reading
        // that keeps every pre-§30 tree behaving exactly as it did.
        assert_eq!(a11y_role(&node(None)), accesskit::Role::Switch);
    }

    /// The interaction half of the same distinction: only `Radio` gets its own
    /// `Control`, because only its toggle rule differs. A `Checkbox` sharing
    /// `Control::Switch` is the point, not an oversight — two variants that behaved
    /// identically would be an invitation to drift.
    #[test]
    fn only_radio_gets_its_own_control_variant() {
        let node = |role: &str| {
            let mut attrs = serde_json::Map::new();
            attrs.insert("checked".into(), serde_json::Value::Bool(true));
            attrs.insert("role".into(), serde_json::Value::String(role.into()));
            RawNode {
                tag: "input".into(),
                id: Some("c".into()),
                classes: String::new(),
                events: Vec::new(),
                attrs,
                children: Vec::new(),
                text: None,
            }
        };
        assert!(matches!(control_of(&node("switch")), Some(Control::Switch { checked: true })));
        assert!(matches!(control_of(&node("checkbox")), Some(Control::Switch { checked: true })));
        assert!(matches!(control_of(&node("radio")), Some(Control::Radio { checked: true })));
    }

    /// Divergence 6 (../PLAN.md §26): a touch never lands on a `Switch`/`Slider`'s own
    /// node, because both synthesize children. Built through the **real** `zkite_py`
    /// widgets rather than hand-written JSON, so the synthesized child structure this
    /// depends on is the one `widgets.py` actually emits — a hand-built fixture would
    /// keep passing if `Slider._tree_children` changed shape underneath it.
    #[test]
    fn a_touch_on_a_synthesized_thumb_resolves_to_the_control_itself() {
        let _py = lock_python();
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let zkite_pkg_dir = repo.join("phase_1/zkite_py/src");
        let app_dir = std::env::temp_dir().join(format!("zkite_pybridge_controls_{}", std::process::id()));
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(
            app_dir.join("main.py"),
            r#"
from zkite import Column, Slider, Switch, Text, store

@store
class S:
    on: bool = True
    v: float = 40.0

def view(s: S):
    return Column("w-full h-full",
        Text("", f"on={s.on!r} v={s.v!r}"),
        Switch(checked=s.on, on_change=lambda x: s.update(on=x)),
        Slider("", min=0, max=100, step=1, value=s.v, on_change=lambda x: s.update(v=x)),
    )
"#,
        )
        .unwrap();

        let logs = std::sync::Mutex::new(Vec::<String>::new());
        let log = |line: &str| logs.lock().unwrap().push(line.to_string());
        let json = embed::setup(&python_prefix(), zkite_pkg_dir.to_str().unwrap(), app_dir.to_str().unwrap(), &log)
            .expect("setup should succeed");
        let root = RawNode::from_json(&json).expect("tree JSON");

        let mut r = test_renderer();
        let built = build(&mut r, &root, None);
        r.layout(built.root, 400.0, 200.0);
        let positioned = r.positioned(built.root, &Default::default());

        // Every node that is *not* itself a control, but sits under one. For a
        // `Switch` that is its thumb; for a `Slider`, its track, the fill inside the
        // track, and its thumb. Each must resolve upward to the control.
        let mut resolved = Vec::new();
        for pn in &positioned {
            let kid = built.zkite_id(pn.node);
            let is_control_itself = kid.is_some_and(|k| built.control(k).is_some());
            if is_control_itself {
                continue;
            }
            if let Some((owner, control, _)) = built.control_at(&r, pn.node) {
                resolved.push((owner.to_string(), control));
            }
        }
        assert!(
            resolved.iter().any(|(_, c)| matches!(c, Control::Switch { checked: true })),
            "a descendant of the Switch must resolve to it; resolved = {resolved:?}"
        );
        assert!(
            resolved.iter().any(|(_, c)| matches!(c, Control::Slider { min, max, value, .. }
                if *min == 0.0 && *max == 100.0 && *value == 40.0)),
            "a descendant of the Slider must resolve to it with its real range; resolved = {resolved:?}"
        );
        assert!(
            resolved.len() >= 4,
            "expected the Switch's thumb plus the Slider's track/fill/thumb, got {}: {resolved:?}",
            resolved.len()
        );

        // And the thing that made this invisible for three phases: these register
        // `change`, never `tap`, so the tap-handler question the render thread used
        // to ask has always answered "no".
        for (owner, _) in &resolved {
            assert!(!built.handles(owner, "tap"), "{owner} should not handle tap");
            assert!(built.handles(owner, "change"), "{owner} should handle change");
        }

        // The whole native path minus the touch itself, through real CPython: the
        // exact strings `render_thread` sends, and the *types* they must arrive as.
        // `on=False` is the assertion that matters — `bool("false")` is `True`, so a
        // switch that could be turned on and never off would still pass a check that
        // only looked at whether the tree changed.
        let switch_id = resolved
            .iter()
            .find(|(_, c)| matches!(c, Control::Switch { .. }))
            .map(|(id, _)| id.clone())
            .expect("a Switch");
        let slider_id = resolved
            .iter()
            .find(|(_, c)| matches!(c, Control::Slider { .. }))
            .map(|(id, _)| id.clone())
            .expect("a Slider");

        let after_switch = embed::dispatch(&switch_id, "change", "false", &log).expect("change must be handled");
        assert!(
            after_switch.contains("on=False"),
            "a `change` of \"false\" must store the bool False, not the truthy string; tree was {after_switch}"
        );

        let after_slider = embed::dispatch(&slider_id, "change", "72", &log).expect("change must be handled");
        assert!(
            after_slider.contains("v=72.0"),
            "a `change` of \"72\" must store the float 72.0, not the string; tree was {after_slider}"
        );

        std::fs::remove_dir_all(&app_dir).ok();
    }

    /// §15.3 bugs 2 and 3, together, because they are two halves of one boot failure
    /// and the fix for each is visible in the same call (../PLAN.md §31.4, §31.5).
    ///
    /// An `ImportError` in the user's `main.py` raises inside `load_module`, which
    /// runs *before* `runtime.py`'s `_app` is assigned. So `_app` stayed `None`,
    /// `_zkite_last_error` returned `None`, `reload.rs` wrote no `OutFrame::Error`, and
    /// `zkite dev`'s terminal was silent while the phone displayed the traceback —
    /// against plan1.md §6's explicit "simultaneously in the terminal".
    ///
    /// **Ordering inside this test is load-bearing**, and it is why this is one test
    /// rather than three: `_dev_connected` is a process global in the one shared
    /// interpreter (see `lock_python`'s doc), so the not-yet-connected case can only
    /// be observed before anything drives a reload. It ends by booting a good app,
    /// which clears `_glue_error` — leaving it set would make `last_error()` report a
    /// stale traceback to whichever test ran next, which is exactly the leak §17.1.1
    /// records.
    #[test]
    fn a_boot_failure_reaches_the_terminal_and_stops_claiming_to_be_disconnected() {
        let _py = lock_python();
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let zkite_pkg_dir = repo.join("phase_1/zkite_py/src");
        let zkite_pkg_dir = zkite_pkg_dir.to_str().unwrap();
        let app_dir = std::env::temp_dir().join(format!("zkite_glueerr_{}", std::process::id()));
        std::fs::create_dir_all(&app_dir).unwrap();
        let app = app_dir.to_str().unwrap();
        let log = |_: &str| {};
        let bad = "from zkite import NoSuchWidget\ndef view():\n    return None\n";
        let good = "from zkite import Text\ndef view():\n    return Text('', 'ok')\n";

        // --- before any `zkite dev` reload: the waiting screen is still the truth ---
        std::fs::write(app_dir.join("main.py"), bad).unwrap();
        let tree = embed::setup(&python_prefix(), zkite_pkg_dir, app, &log).expect("setup returns a frame");
        assert!(tree.contains("__zkite_error_overlay"), "a boot failure must render the overlay");
        assert!(
            tree.contains("waiting for `zkite dev`"),
            "with no dev server having spoken, the waiting screen is correct: {tree}"
        );

        // The bug: this was `None`, so nothing was ever written to the socket.
        let err = embed::last_error().expect("a glue-level failure must be reportable to `zkite dev`");
        assert!(
            err.contains("ImportError") && err.contains("NoSuchWidget"),
            "the reported error must be the real traceback, got: {err}"
        );

        // --- after a reload, i.e. `zkite dev` is demonstrably connected and serving ---
        let tree = embed::reload(&log).expect("reload returns a frame");
        assert!(tree.contains("__zkite_error_overlay"), "still broken, still an overlay");
        assert!(
            !tree.contains("waiting for `zkite dev`"),
            "a dev server that just drove this reload is not disconnected; the placeholder \
             read as one and is what sent the owner looking for a dropped connection: {tree}"
        );

        // --- and a fixed app clears it, so a stale error is not reported forever ---
        std::fs::write(app_dir.join("main.py"), good).unwrap();
        let tree = embed::reload(&log).expect("reload returns a frame");
        assert!(!tree.contains("__zkite_error_overlay"), "a fixed app renders normally: {tree}");
        assert_eq!(embed::last_error(), None, "a good frame must clear the reported error");

        std::fs::remove_dir_all(&app_dir).ok();
    }

    /// The Python half of the framework-fingerprint contract (../PLAN.md §31.6).
    ///
    /// `phase_1/cli/src/dev_cmd.rs`'s `FIXTURE`/`FIXTURE_DIGEST` are the *same* two
    /// files and the *same* literal. The digest is computed by neither test: it is a
    /// constant both sides are held against, so if `runtime.py` and `dev_cmd.rs` ever
    /// stop agreeing, one of these two tests goes red instead of `zkite dev` silently
    /// reporting every Dev Shell as stale — which is a failure mode strictly worse
    /// than the one the handshake exists to fix.
    ///
    /// This drives the real glue function through the real interpreter, so it also
    /// covers the thing a pure-Rust test could not: that `hashlib` is importable and
    /// that the function is reachable by name from `call_str_fn`.
    #[test]
    fn the_framework_fingerprint_matches_the_hosts_own_algorithm() {
        let _py = lock_python();
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let zkite_pkg_dir = repo.join("phase_1/zkite_py/src");
        let app_dir = std::env::temp_dir().join(format!("zkite_fp_app_{}", std::process::id()));
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(app_dir.join("main.py"), "from zkite import Text\ndef view():\n    return Text('', 'x')\n").unwrap();
        let log = |_: &str| {};
        embed::setup(&python_prefix(), zkite_pkg_dir.to_str().unwrap(), app_dir.to_str().unwrap(), &log)
            .expect("setup should succeed");

        // Same two files as dev_cmd.rs's FIXTURE, laid out as a `zkite` package.
        let fixture = std::env::temp_dir().join(format!("zkite_fp_fixture_{}", std::process::id()));
        let pkg = fixture.join("zkite");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("__init__.py"), "A\n").unwrap();
        std::fs::write(pkg.join("widgets.py"), "B\n").unwrap();

        assert_eq!(
            embed::framework_fingerprint(fixture.to_str().unwrap()).as_deref(),
            Some("4db0afb50304"),
            "runtime.py's digest must equal the literal dev_cmd.rs pins for the same files",
        );

        // Fails open rather than reporting a bogus mismatch, matching the Rust half.
        let empty = std::env::temp_dir().join(format!("zkite_fp_empty_{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(embed::framework_fingerprint(empty.to_str().unwrap()), None);

        // And it really does discriminate: the `Checkbox` case, a file short.
        std::fs::remove_file(pkg.join("widgets.py")).unwrap();
        let partial = embed::framework_fingerprint(fixture.to_str().unwrap());
        assert!(partial.is_some());
        assert_ne!(partial.as_deref(), Some("4db0afb50304"));

        std::fs::remove_dir_all(&fixture).ok();
        std::fs::remove_dir_all(&empty).ok();
        std::fs::remove_dir_all(&app_dir).ok();
    }

    #[test]
    fn a_text_field_is_not_a_touch_driven_control() {
        // `TextField` is value-bearing too, but it is driven by the IME path — if it
        // were a `Control`, a tap on one would set a value instead of focusing it.
        let text_field = RawNode {
            tag: "input".into(),
            id: Some("f".into()),
            classes: String::new(),
            events: Vec::new(),
            attrs: serde_json::json!({"input_type": "text", "value": "", "placeholder": ""})
                .as_object()
                .unwrap()
                .clone(),
            children: Vec::new(),
            text: None,
        };
        assert_eq!(control_of(&text_field), None);
    }

    /// A `Slider` node must carry `min`/`max`/`numeric_value`, never `value` — see
    /// `build_a11y_node`'s own comment on why (`accesskit_android::node`'s glue only
    /// emits Android's `RangeInfo` when `value()` is absent, checked directly against
    /// its source; see ../PLAN.md §5 item 2).
    #[test]
    fn slider_gets_numeric_range_not_a_text_value() {
        let ctx = BuildCtx {
            node_ids: FxHashMap::default(),
            events: FxHashMap::default(),
            text_fields: FxHashSet::default(),
            controls: FxHashMap::default(),
            focused: None,
            focused_text_node: None,
            a11y_nodes: FxHashMap::default(),
            a11y_ids: FxHashMap::default(),
            anims: Vec::new(),
            anim_errors: Vec::new(),
        };
        let slider = RawNode {
            tag: "input".into(),
            id: Some("sl".into()),
            classes: String::new(),
            events: Vec::new(),
            attrs: serde_json::json!({"min": 0.0, "max": 100.0, "step": 1.0, "value": 42.0})
                .as_object()
                .unwrap()
                .clone(),
            children: Vec::new(),
            text: None,
        };
        let node = build_a11y_node(&slider, &[], "", false, &ctx);
        assert_eq!(node.role(), accesskit::Role::Slider);
        assert_eq!(node.min_numeric_value(), Some(0.0));
        assert_eq!(node.max_numeric_value(), Some(100.0));
        assert_eq!(node.numeric_value(), Some(42.0));
        assert_eq!(node.value(), None, "a Slider must not carry a text `value`, or Android's RangeInfo is suppressed");
    }

    /// A tiny app whose button spawns a worker thread that updates the store a moment
    /// later — the shape of every "fetch and display" app, reduced to its skeleton.
    ///
    /// **The store is called `PushState` and not `S`, and that is load-bearing.** It
    /// was `S` first, and it failed `a_touch_on_a_synthesized_thumb_resolves_to_the_
    /// control_itself` **5 runs out of 5** under `--test-threads=1` — deterministically,
    /// which is what distinguishes it from §29.5's genuinely intermittent one.
    /// `zkite._store`'s instance registry is process-global and keyed by *qualified
    /// name*, every test app is loaded as module `main`, and `runtime.py` only clears
    /// it on a shape change — so the second app to declare `main.S` was handed the
    /// first app's instance, of a class with entirely different fields. Its `view()`
    /// raised, the error tree has no `Switch` in it, and the failure surfaced three
    /// layers away as "a descendant of the Switch must resolve to it". Renaming this
    /// one identifier is the whole fix, and flipping it back is the negative control.
    /// A new test that boots an app through CPython needs a store name no other test
    /// uses. See ../PLAN.md §34.6.
    const PUSH_APP: &str = r#"
import threading, time
from zkite import Column, Text, Button, store

@store
class PushState:
    status: str = "idle"

def work(s):
    time.sleep(0.05)
    s.update(status="done-from-thread")

def view(s: PushState):
    return Column("flex-1 items-center justify-center bg-slate-950",
        Text("text-white", s.status),
        Button("px-6 py-3 bg-indigo-500 text-white", "Go",
               on_tap=lambda: threading.Thread(target=work, args=(s,), daemon=True).start()),
    )
"#;

    /// Boot `PUSH_APP` in a fresh app dir, returning (initial tree JSON, button id).
    fn boot_push_app(renderer: &mut Renderer, tag: &str, log: &dyn Fn(&str)) -> (String, String) {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let zkite_pkg_dir = repo.join("phase_1/zkite_py/src");
        let app_dir = std::env::temp_dir().join(format!("zkite_pybridge_{tag}_{}", std::process::id()));
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(app_dir.join("main.py"), PUSH_APP).unwrap();
        let json = embed::setup(&python_prefix(), zkite_pkg_dir.to_str().unwrap(), app_dir.to_str().unwrap(), log)
            .expect("setup should succeed");
        let root = RawNode::from_json(&json).unwrap();
        let built = build(renderer, &root, None);
        let button_id = built
            .node_ids
            .values()
            .find(|id| built.handles(id, "tap"))
            .expect("the Button should be in the event map")
            .clone();
        (json, button_id.to_string())
    }

    /// ../PLAN.md §34 — the gap this whole section exists for. Until the push path,
    /// `App` had `on_render_tree` and nothing registered it, so a `@store.update()`
    /// from a worker thread refreshed `last_tree` and **no frame was ever produced**.
    ///
    /// This is the end-to-end proof, through the real embedded interpreter and the
    /// real `runtime.py` glue: a genuine `threading.Thread` calls `update()` with
    /// nobody asking for anything, and a complete tree arrives at the shell's sink.
    #[test]
    fn a_background_thread_pushes_a_frame_nobody_asked_for() {
        let _py = lock_python();
        let mut renderer = test_renderer();
        let logs = std::sync::Mutex::new(Vec::<String>::new());
        let log = |line: &str| logs.lock().unwrap().push(line.to_string());

        let frames: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        let sink_frames = frames.clone();
        embed::set_frame_sink(Some(Box::new(move |f: &str| {
            sink_frames.lock().unwrap().push(f.to_string());
        })));

        let (initial, button_id) = boot_push_app(&mut renderer, "push", &log);
        assert!(initial.contains("idle"));
        assert!(
            frames.lock().unwrap().is_empty(),
            "boot is a pull — it must not also push the frame its caller is already holding"
        );

        // The tap itself is a pull: it returns the new tree to the caller.
        let after_tap = embed::dispatch(&button_id, "tap", "", &log).expect("tap should be handled");
        assert!(after_tap.contains("idle"), "the worker has not finished yet");
        assert!(
            frames.lock().unwrap().is_empty(),
            "a dispatched tap must not push: its caller already has the tree"
        );

        // Now wait for the worker. Nothing here asks Python for anything.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while frames.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let got = frames.lock().unwrap().clone();
        embed::set_frame_sink(None);

        assert_eq!(got.len(), 1, "exactly one unrequested frame, got {got:?}");
        assert!(
            got[0].contains("done-from-thread"),
            "the pushed frame must be the *new* tree, got: {}",
            &got[0]
        );
        // And it is a real tree, not a fragment: it parses and builds like any other.
        let pushed = RawNode::from_json(&got[0]).expect("a pushed frame is a complete tree");
        let built = build(&mut renderer, &pushed, None);
        renderer.layout(built.root, 400.0, 800.0);
        assert!(!renderer.positioned(built.root, &Default::default()).is_empty());
    }

    /// The push path must be **inert unless a shell opted in**. `runtime.py` is
    /// `include_str!`'d by three shells and this crate's own host tests run it with no
    /// sink at all; a missing hook has to be an ordinary quiet no-op rather than a
    /// traceback surfacing out of an unrelated `update()` deep in user code.
    #[test]
    fn with_no_sink_installed_a_worker_thread_update_is_a_silent_no_op() {
        let _py = lock_python();
        embed::set_frame_sink(None);
        let mut renderer = test_renderer();
        let logs = std::sync::Mutex::new(Vec::<String>::new());
        let log = |line: &str| logs.lock().unwrap().push(line.to_string());

        let (_, button_id) = boot_push_app(&mut renderer, "nosink", &log);
        embed::dispatch(&button_id, "tap", "", &log).expect("tap should be handled");
        std::thread::sleep(std::time::Duration::from_millis(300));

        // The worker ran and its state landed; a later pull sees it. Nothing raised.
        let pulled = embed::dispatch(&button_id, "tap", "", &log).expect("tap should be handled");
        assert!(
            pulled.contains("done-from-thread"),
            "the worker's update must still reach `last_tree` with no sink installed"
        );
        let joined = logs.lock().unwrap().join("\n");
        assert!(
            !joined.contains("Traceback"),
            "a missing frame hook must not raise into user code: {joined}"
        );
    }
}
