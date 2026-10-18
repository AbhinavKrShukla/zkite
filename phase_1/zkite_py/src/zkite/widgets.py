"""Widget tree -> HTML string.

Design note on argument order (see ../PLAN.md and plan1.md §8): the class string is
*always* the first positional argument, on every widget, with no special case for
widgets that happen to hold text. plan1.md §8's own design notes state this rule
explicitly ("First positional arg is always the class string; children follow as
*args"), but its one code example writes `Text(f"{c.n}", "text-7xl ...")` — content
first. That's inconsistent with the stated rule and, read against every other example
in the document (`Column("...", child, child)`), looks like the example wasn't updated
rather than the rule being wrong. A single uniform calling convention across all
widgets is also simply better DX — no per-widget special case to remember — so this
implementation follows the rule, not that one example:

    Text("text-7xl font-bold text-white tabular-nums", f"{c.n}")
    Button("px-6 py-3 rounded-2xl bg-indigo-500 ...", "Increment", on_tap=...)
"""

from __future__ import annotations

import html
from typing import Any, Callable, Iterable, Union

from . import _anim
from . import _store as _store_mod
from ._viewport import BOOTSTRAP_VIEWPORT_PX, compute_window, get_viewport, note_window

Child = Union["Widget", str, int, float, None]


class RenderContext:
    """Per-render registry of event handlers, keyed by node id — plus the back chain.

    The back chain (see `_nav.py`) lives here rather than anywhere new because it
    has exactly the same lifetime as the event handlers: registered during a render,
    consulted after it, discarded when the next render replaces this context. Giving
    it its own storage would have meant inventing a second lifecycle that had to be
    kept in sync with this one for no gain.
    """

    __slots__ = ("handlers", "back_handlers", "overlays", "system_bars", "key_scope", "overlay_scopes")

    def __init__(self) -> None:
        self.handlers: dict[str, dict[str, tuple[Callable[..., Any], Callable[[Any], Any] | None]]] = {}
        self.back_handlers: list[Callable[[], bool]] = []
        self.overlays: list[Overlay] = []
        self.overlay_scopes: list[str] = []
        self.key_scope = ""
        self.system_bars: dict | None = None

    def node_id(self, path: str, key: str | None) -> str:
        if key is None:
            return path
        key = str(key)
        if self.key_scope and not key.startswith(self.key_scope + "/"):
            return f"{self.key_scope}/{key}"
        return key

    def register(
        self,
        node_id: str,
        event: str,
        handler: Callable[..., Any],
        parse: Callable[[Any], Any] | None = None,
    ) -> None:
        """Register `handler` for `event` on `node_id`, with an optional payload parser.

        `parse` exists because **every backend's event bridge is string-in,
        string-out** — `zkite-runtime.js` sends `String(payload)` over
        `ZKiteNative.dispatch`, and the native render thread calls
        `zkite_pybridge::dispatch(id, event, &str)`. Neither can carry a type, and
        retyping either bridge would mean touching Java and JNI in three shells to
        move information that the widget already knows.

        So the payload is typed *here*, at the widget that raised the event, which is
        the one place that both backends already share (`Widget._register_events`
        feeds this from `Widget._payload_parser`). See `Switch`/`Slider` below and
        phase_3/PLAN.md §26.
        """
        self.handlers.setdefault(node_id, {})[event] = (handler, parse)

    def register_back(self, handler: Callable[[], bool]) -> None:
        """Offer `handler` a Back press before anything registered earlier gets it.

        Registration happens in tree order (a parent before its children), so the
        *last* registration is the innermost widget — which is why `dispatch_back`
        walks this list in reverse. A nested navigator, a modal sheet, or a widget
        with a dismissible overlay therefore beats the screen stack that contains
        it without any of them having to know about each other.
        """
        self.back_handlers.append(handler)

    def register_overlay(self, overlay: "Overlay") -> None:
        """Hand `overlay` to the app root, to be rendered *after* the whole tree.

        The one mechanism in this framework that paints above the normal tree
        (phase_3/PLAN.md §33). It exists because "above" is a property of document
        order on **both** backends and of nothing else: `zkite_renderer` paints in
        document order and hit-tests it in reverse (`Renderer::hit_test`), and the DOM
        does the same for positioned elements in tree order. A layer emitted last is
        therefore drawn last and tapped first, on both, with no renderer change, no
        `z-index` (which `twcore` does not parse — §31.8) and no stacking context.

        Registration happens where the overlay *sits in the tree* — so the back-chain
        ordering below stays innermost-first — while the rendering happens at the root.
        `app.App` drains this list by index rather than iterating it, because an overlay
        may itself contain a widget that opens another one.
        """
        self.overlays.append(overlay)
        self.overlay_scopes.append(self.key_scope)

    def dispatch_back(self) -> bool:
        """Offer a Back press to each handler, innermost first. `True` if consumed.

        `False` means nothing wanted it and the platform should do its default
        thing — which on Android is to exit the app, and is correct: backing out of
        the root screen of a one-screen app has always exited it and must keep
        doing so (phase_3/PLAN.md §16).
        """
        for handler in reversed(self.back_handlers):
            if handler():
                return True
        return False

    def dispatch(self, node_id: str, event: str, payload: Any = None) -> bool:
        entry = self.handlers.get(node_id, {}).get(event)
        if entry is None:
            return False
        handler, parse = entry
        if payload is not None and parse is not None:
            payload = parse(payload)
        if payload is not None:
            invoke(handler, payload)
        else:
            handler()
        return True


def invoke(handler: Callable[..., Any], value: Any) -> Any:
    """Call `handler` with `value` if it takes an argument, without one if it doesn't.

    Every user-supplied callback in this framework is allowed to ignore its payload —
    `on_change=lambda: s.update(n=s.n+1)` is as valid as `on_change=lambda v: ...` —
    so the arity check is a rule, not a convenience, and it lives here so that
    `RenderContext.dispatch` and any widget that delivers a value *without* going
    through the bridge (`Select`, whose chosen value never crosses one) apply the
    identical rule rather than two nearly-identical ones.
    """
    import inspect

    if inspect.signature(handler).parameters:
        return handler(value)
    return handler()


def _flatten(children: Iterable[Child]) -> list[Child]:
    """Allow nested lists and drop Nones, so conditionals (`cond and Widget(...)`) work."""
    out: list[Child] = []
    for c in children:
        if c is None or c is False:
            continue
        if isinstance(c, (list, tuple)):
            out.extend(_flatten(c))
        else:
            out.append(c)
    return out


def _as_bool(v: Any) -> bool:
    """Payload -> `bool`, for `Switch.on_change`. See `RenderContext.register`.

    Accepts what each backend actually sends. The WebView side is
    `String(el.checked)`, i.e. JavaScript's `"true"`/`"false"`; the native side sends
    Rust's `bool::to_string`, which is the same two spellings — that is not a
    coincidence and it is why one parser covers both. A real `bool` passes straight
    through so a host test (or any future non-string transport) is unaffected.

    Anything unrecognised is `False` rather than truthy, which is the specific bug
    this exists to make impossible: `bool("false")` is `True`, so before
    phase_3/PLAN.md §26 a `Switch` on the WebView backend could be turned on and
    never off.
    """
    if isinstance(v, bool):
        return v
    return str(v).strip().lower() in ("true", "1", "on", "yes")


def _as_float(v: Any, fallback: float = 0.0) -> float:
    """Payload -> `float`, for `Slider.on_change`. See `RenderContext.register`.

    `fallback` is the slider's current value, so an unparseable payload means "no
    change" instead of a silent jump to zero — the payload can only be malformed if a
    bridge is broken, and a bug in the bridge should not also move the user's data.
    """
    if isinstance(v, bool):
        return fallback
    if isinstance(v, (int, float)):
        return float(v)
    try:
        return float(str(v).strip())
    except (TypeError, ValueError):
        return fallback


def _attr_str(attrs: dict[str, Any]) -> str:
    parts = []
    for k, v in attrs.items():
        if v is None or v is False:
            continue
        if v is True:
            parts.append(f' {k}')
            continue
        parts.append(f' {k}="{html.escape(str(v), quote=True)}"')
    return "".join(parts)


class Widget:
    tag: str = "div"
    implied_classes: str = ""
    void: bool = False
    #: Whether this widget produces a node where it was written. `False` only for
    #: `Overlay`, which hands itself to the app root instead (phase_3/PLAN.md §33) —
    #: a container that composes children generically has to skip it rather than
    #: reserve it a slot. Declared here rather than sniffed from a render result so
    #: the rule is one greppable fact instead of two coincidences.
    renders_in_place: bool = True

    def __init__(self, class_name: str = "", *children: Child, key: str | None = None, **props: Any):
        self.class_name = class_name or ""
        self.children: list[Child] = _flatten(children)
        self.key = key
        self.props = props

    def classes(self) -> str:
        if self.implied_classes:
            return f"{self.implied_classes} {self.class_name}".strip()
        return self.class_name

    def _tag_attrs(self) -> dict[str, Any]:
        return {}

    def _state_classes(self, classes: str) -> str:
        # Keep explicit app styling; otherwise disabled controls need a visible cue.
        if self.props.get("disabled", False) and not any(c.startswith("opacity-") for c in classes.split()):
            return f"{classes} opacity-50".strip()
        return classes

    def _disabled_events(self, ctx: RenderContext, node_id: str) -> list[str] | None:
        if not self.props.get("disabled", False):
            return None
        # Consume taps so a disabled control cannot activate a tappable ancestor.
        ctx.register(node_id, "tap", lambda: None)
        return ["tap"]

    def _register_events(self, ctx: RenderContext, node_id: str) -> list[str]:
        disabled = self._disabled_events(ctx, node_id)
        if disabled is not None:
            return disabled
        events = []
        for k, v in self.props.items():
            if k.startswith("on_") and callable(v):
                event = k[3:]
                ctx.register(node_id, event, v, self._payload_parser(event))
                events.append(event)
        return events

    def _anim_wire(self) -> list[dict]:
        """This widget's `anim=` declaration, as the renderer's wire format.

        Validated at *construction* (`_anim.Anim.__init__`) rather than here, so an
        author's typo raises where they wrote it — which the error overlay then shows
        with the right source line — instead of on the first render of a screen they
        may not be looking at.
        """
        return _anim.to_wire(self.props.get("anim"))

    def _payload_parser(self, event: str) -> Callable[[Any], Any] | None:
        """How this widget's `event` payload should be typed before it reaches a handler.

        `None` — the default — means "hand the handler whatever the bridge delivered",
        which for every backend is a `str` (see `RenderContext.register`). That is
        correct for `TextField`, whose value genuinely is text, and wrong for anything
        whose value is not — see `Switch`/`Slider`, which override this.

        Declared per-widget rather than per-backend on purpose: both backends route
        through `_register_events`, so a widget that types its payload here types it
        identically on both, and cannot be fixed on one and left broken on the other.
        That is exactly what happened before phase_3/PLAN.md §26 — `Switch` delivered
        the *string* `"false"` on the WebView backend, and `bool("false")` is `True`,
        so a switch could be turned on and never off.
        """
        # `animend` is typed on the *base*, not per widget, because `anim=` is
        # accepted by every widget and so is its completion. §26.2's string payload
        # has now bitten `Switch`, `Slider` and `Select` in turn; typing this one at
        # birth is the cheap version of not making it a fourth.
        if event == "animend":
            return _anim.parse_anim_end
        return None

    def _render_children(self, ctx: RenderContext, node_id: str) -> str:
        parts = []
        for i, c in enumerate(self.children):
            child_path = f"{node_id}.{i}"
            if isinstance(c, Widget):
                parts.append(c.render(ctx, child_path))
            else:
                parts.append(html.escape(str(c)))
        return "".join(parts)

    def render(self, ctx: RenderContext, path: str) -> str:
        node_id = ctx.node_id(path, self.key)
        attrs: dict[str, Any] = {"id": node_id}
        classes = self._state_classes(self.classes())
        if classes:
            attrs["class"] = classes
        attrs.update(self._tag_attrs())
        if self.props.get("disabled", False):
            attrs.update({"disabled": True, "aria-disabled": "true"})
        events = self._register_events(ctx, node_id)
        if events:
            attrs["data-zkite-on"] = " ".join(sorted(events))
        # The HTML backend has no animation driver — that is layer 4 (phase_3/PLAN.md
        # §45.3) and it is not built. Rather than emit an attribute nothing reads, say
        # so once: a declaration that silently does nothing on one backend is the
        # §15.1 failure this whole feature is careful about, and the difference between
        # a filed divergence and a hidden one is a single line in a log.
        if self._anim_wire():
            _anim.warn_html_backend_once()
        attr_str = _attr_str(attrs)
        if self.void:
            return f"<{self.tag}{attr_str}>"
        return f"<{self.tag}{attr_str}>{self._render_children(ctx, node_id)}</{self.tag}>"

    # ---- native (phase 3) tree export ----------------------------------------------
    # Same node-id assignment and event-registration logic as `render()` above (they
    # share `_register_events`, so a tap dispatches against the *same* handler either
    # backend registered it under), producing plain data (dicts/lists/strings) instead
    # of an HTML string. `phase_3/pybridge` walks this tree via the CPython C-API,
    # after it crosses the JNI boundary as JSON — the same "one glue function, one
    # string in, one string out" shape `runtime.py`'s existing `_zkite_*` functions
    # already use for HTML, just a different string. See phase_3/PLAN.md §5.

    def to_tree(self, ctx: RenderContext, path: str) -> dict:
        node_id = ctx.node_id(path, self.key)
        events = self._register_events(ctx, node_id)
        attrs = self._tree_attrs()
        if self.props.get("disabled", False):
            attrs = {**attrs, "disabled": True}
        # Merged here rather than inside `_tree_attrs` because eleven widgets override
        # that method and build their own dict; folding `anim` in at the one shared
        # call site is what makes it work on all of them instead of on the ones that
        # remembered. Copied rather than mutated in place so a widget returning a
        # cached or shared dict is not modified under it.
        anim = self._anim_wire()
        if anim:
            attrs = {**attrs, "anim": anim}
        return {
            "tag": self.tag,
            "id": node_id,
            "classes": self._state_classes(self._tree_classes()),
            "events": sorted(events),
            "attrs": attrs,
            "children": self._tree_children(ctx, node_id),
        }

    def _tree_classes(self) -> str:
        """Usually identical to `classes()` (HTML). Overridden only where a widget's
        HTML/CSS mechanism has no `twcore` equivalent at all — verified per-class
        against `phase_0/spike3_twcore/src/parse.rs`, not assumed — and a different
        (not just absent) set of classes achieves the same visual another way; see
        `Stack`/`Icon` below. Left as `classes()` everywhere else specifically so a
        gap in one widget doesn't silently mask a real typo in another's classes —
        `zkite_renderer`'s `unknown_classes()` sweep (`phase_3/renderer/src/lib.rs`
        tests) still has to explain every unknown class it finds.
        """
        return self.classes()

    def _tree_attrs(self) -> dict[str, Any]:
        return {}

    def _tree_children(self, ctx: RenderContext, node_id: str) -> list[dict]:
        out: list[dict] = []
        for i, c in enumerate(self.children):
            child_path = f"{node_id}.{i}"
            if isinstance(c, Widget):
                node = c.to_tree(ctx, child_path)
                # `None` means "this widget renders somewhere other than here" — today
                # that is `Overlay` alone, which registers itself as an app-root layer
                # and occupies no space where it was written (phase_3/PLAN.md §33). The
                # HTML path needs no equivalent branch: `render()` returns `""` there
                # and an empty string concatenates to nothing on its own.
                if node is not None:
                    out.append(node)
            else:
                # Every bare string/int/float child becomes a `#text` node with no
                # classes of its own — `zkite_renderer::Renderer::container` inherits
                # typography (color/size/weight/...) from the nearest styled ancestor,
                # matching plain CSS inheritance. See ../../phase_3/renderer/src/lib.rs.
                out.append({"tag": "#text", "text": str(c)})
        return out


# ---- containers -------------------------------------------------------------------

class Column(Widget):
    tag = "div"
    implied_classes = "flex flex-col"


class Row(Widget):
    tag = "div"
    implied_classes = "flex flex-row"


class Grid(Widget):
    tag = "div"
    implied_classes = "grid"


class Stack(Widget):
    """Overlays children on top of each other via a 1x1 CSS grid."""

    tag = "div"
    implied_classes = "grid"

    def _render_children(self, ctx: RenderContext, node_id: str) -> str:
        parts = []
        for i, c in enumerate(self.children):
            child_path = f"{node_id}.{i}"
            inner = c.render(ctx, child_path) if isinstance(c, Widget) else html.escape(str(c))
            # Rendered (so it still registers itself) but given no cell of its own —
            # the tree path skips it in the same place and for the same reason. An
            # `Overlay` occupies no space where it was written; see
            # `RenderContext.register_overlay`.
            if isinstance(c, Widget) and not c.renders_in_place:
                continue
            parts.append(f'<div class="col-start-1 row-start-1">{inner}</div>')
        return "".join(parts)

    def _tree_classes(self) -> str:
        # `grid` (this class's own HTML mechanism) has no native equivalent for what
        # `_tree_children` below needs: `col-start-1`/`row-start-1` — real CSS grid
        # *line placement*, not a span — isn't implemented in `twcore` at all (checked
        # against phase_0/spike3_twcore/src/decl.rs: only `ColSpan`/`RowSpan` exist).
        # `position:relative` + `absolute inset-0` on every child achieves the
        # identical "every child occupies the same rect" result with utilities that
        # already exist, so the native tree substitutes it wholesale instead of
        # emitting classes that would silently no-op (each child landing in its own
        # default-flow position instead of overlapping). See phase_3/PLAN.md §5.
        return f"relative {self.class_name}".strip()

    def _tree_children(self, ctx: RenderContext, node_id: str) -> list[dict]:
        out: list[dict] = []
        for i, c in enumerate(self.children):
            child_path = f"{node_id}.{i}"
            inner = c.to_tree(ctx, child_path) if isinstance(c, Widget) else {"tag": "#text", "text": str(c)}
            # An `Overlay` renders as an app-root layer rather than in place, so it has
            # no cell in this stack — see `Widget._tree_children`. Without this it would
            # get a wrapper `div` containing `None`, which `zkite_pybridge` would reject.
            if inner is None:
                continue
            out.append({
                # `flex flex-col` for exactly the reason `ScrollView` below documents,
                # and this is the third instance of it (phase_3/PLAN.md §31.2): the
                # HTML wrapper is a *grid item*, i.e. `display:block`, so its single
                # child fills the wrapper's width and takes its content height. This
                # wrapper carried no display class at all, so taffy applied its own
                # default of `flex`/`row` and the child was sized to its *content*
                # width instead — a `Stack` child without an explicit `w-` did not fill
                # the stack on the native backend and did on the WebView. `flex-col`
                # reproduces block layout for one child: cross-axis stretch gives the
                # full width, main-axis start gives the content height.
                #
                # The pre-§31 `stack` fixture could not see this, because both its
                # children carry `w-[200px]`/`w-[80px]` — an explicit width overrides
                # the stretch either way, so the fixture agreed by construction. Same
                # shape as §18.4's `text-[20px]` icon fixture.
                "tag": "div",
                "id": f"{node_id}.{i}.wrap",
                "classes": "absolute inset-0 flex flex-col",
                "events": [],
                "attrs": {},
                "children": [inner],
            })
        return out


class ScrollView(Widget):
    tag = "div"
    # `flex flex-col` is not decorative — same "no UA stylesheet" gap the Button fix
    # above documents, a second real instance of it, found the same way (running the
    # native renderer on a real scrollable list, not just reading the code): a plain
    # `<div>` defaults to `display:block` in a browser, so `overflow-auto` alone
    # already stacks its children vertically there. `twcore`/`zkite_renderer` has no
    # block layout mode at all — only flex/grid — so without an explicit
    # `flex-direction`, taffy's own default (`row`) laid every child out
    # side-by-side instead, confirmed visually on a real device before this fix (a
    # 30-row list rendered as 30 narrow vertical strips). `flex-col` makes the two
    # backends agree for the common case (full-width rows); the one remaining, minor
    # difference is that flex children default to `align-items:stretch` where block
    # children are simply 100%-width already — invisible for typical
    # `w-full`/full-width scroll content, a real (if narrow) gap for a child that
    # intentionally wants to stay content-sized.
    #
    # `min-h-0` is a second, independent real fix, for a real bug found the same way
    # (a native-rendered ScrollView with 60 real rows simply refused to scroll on a
    # real device, `max_scroll_y` computing exactly 0). Root cause: a flex item's
    # default `min-height` is `auto` — "at least as big as my content" — not `0`, in
    # *every* real browser, not just this renderer (this is the exact, extremely
    # common "why won't my nested flexbox scroll" gotcha web developers hit
    # constantly; `twcore`/taffy implement the same real flexbox spec behavior here,
    # confirmed directly against an isolated `zkite_renderer` reproduction before
    # concluding it wasn't a taffy bug). Without overriding it, a `flex-1` ScrollView
    # grows to fit *all* of its content instead of the space actually available to
    # it, so its own "viewport" ends up exactly as tall as its content and there is
    # nothing left to scroll. Baked in here because a widget whose entire purpose is
    # scrolling should not require every caller to already know this CSS-flexbox
    # subtlety — see plan1.md §5's own promise that ZKite widgets should not leak
    # implementation gotchas like this.
    #
    # This does **not** fully solve nested `flex-1` chains by itself: any
    # intermediate `flex-1` container *between* a `ScrollView` and its nearest
    # non-growing ancestor needs the same `min-h-0` fix, in real Tailwind/CSS too,
    # not just here — confirmed by reproducing the exact same failure with `min-h-0`
    # on the `ScrollView` alone but not an intermediate wrapper `Column`. Document
    # this in app code (see `phase_3/gpu_harness/demo_app/main.py`'s own comment)
    # rather than trying to bake a general fix into every `Column`/`Row`, which would
    # make native-only layout diverge from the WebView backend for any app that
    # deliberately wants a flex item's real content-based minimum size. See
    # phase_3/PLAN.md §5.
    implied_classes = "flex flex-col overflow-auto min-h-0"

    # The horizontal form is a constructor flag rather than "just write the classes
    # yourself", and the reason is a real divergence rather than ergonomics.
    #
    # `classes()` concatenates `implied_classes` first and the caller's string second,
    # so on the *native* backend `ScrollView("flex-row overflow-x-auto")` would work:
    # `twcore` resolves a class string left-to-right and the later token wins. On the
    # **WebView** backend it would not. Real Tailwind emits every utility into one
    # stylesheet at the same specificity, so `flex-col` vs `flex-row` is decided by
    # which rule comes later *in Tailwind's own output*, not by the order the classes
    # appear in the attribute — the well-known Tailwind footgun. A caller who
    # overrode the direction by hand would get a horizontal strip on native and a
    # vertical one on WebView, which is precisely the silent-divergence class
    # phase_3/PLAN.md §15.1 exists to track.
    #
    # Choosing the implied set up front means neither backend is ever asked to
    # resolve a conflict, so there is nothing for them to disagree about.
    def __init__(self, class_name: str = "", *children: Child, horizontal: bool = False, **kw: Any):
        super().__init__(class_name, *children, **kw)
        if horizontal:
            # `min-w-0` is the x-axis twin of `min-h-0` and is load-bearing for the
            # identical reason: a `flex-1` item's default `min-width: auto` grows it
            # to fit its content, so the strip's own "viewport" ends up exactly as
            # wide as its content and `max_scroll` computes 0 — nothing clips, so
            # nothing scrolls. Same real flexbox behaviour, same silent failure.
            #
            # Only `overflow-x-auto`, deliberately: it leaves `overflow-y` visible, so
            # `Renderer::is_scrollable(_, Y)` is false for a strip and a *vertical*
            # drag on it passes through to whatever list is underneath. Using
            # `overflow-auto` here would declare both axes and make the strip claim
            # vertical drags it has no room to satisfy.
            self.implied_classes = "flex flex-row overflow-x-auto min-w-0"


def _px(v: float) -> str:
    """Format a pixel length for an arbitrary-value class (`h-[56px]`).

    Integral values render without a decimal point on purpose: `twcore` memoises
    by the *class string* (see `phase_0/spike3_twcore/src/lib.rs`'s two caches),
    so `h-[56px]` and `h-[56.0px]` would be two separate cache entries for one
    height. `List` already pushes more distinct class strings through that cache
    than any other widget (see its own note on cache growth) — no reason to
    double it for formatting.
    """
    return f"{int(v)}" if float(v).is_integer() else f"{v:g}"


class List(Widget):
    """A virtualised list: builds only the rows currently on screen.

    plan1.md §8 names `List` (virtualised) in the v1 widget set. Phase 1 shipped
    the other 12 widgets and deliberately deferred this one — "needs
    virtualization to be honest about performance; building a non-virtualized
    stand-in would teach a bad habit" (phase_1/PLAN.md §3) — so this is that
    widget, built once the native renderer existed to need it. See
    phase_3/PLAN.md §11.

        List("flex-1", count=10_000, item_height=56,
             builder=lambda i: Row("items-center px-4", Text("", f"Item {i}")))

    **How it stays correct.** The rows actually emitted are only those in the
    visible window (plus `overscan` rows either side); everything above and below
    is collapsed into two fixed-height spacer nodes. That keeps the container's
    total content height at exactly `count * item_height` no matter which window
    is materialised, which is what makes the scrollbar position, `max_scroll_y`,
    and every row's on-screen y-coordinate identical to a non-virtualised list —
    the windowing is invisible to the layout engine, by construction, rather than
    by anything the renderer has to know about lists.

    That invariant is also why this widget's own height can't feed back into its
    own window: content height doesn't depend on the window, so a `List` sized by
    its content is stable rather than oscillating. It's still almost always wrong
    to size one that way (a `List` as tall as its content has nothing to scroll —
    the same trap `ScrollView` documents), hence `min-h-0` in the implied classes
    and `flex-1` in every example.

    **`item_height` is a contract, not a hint.** Each row is wrapped in a
    fixed-height, `overflow-hidden` box, so a row that renders taller is clipped
    rather than silently desynchronising the spacer math from reality. Variable
    row heights are a real, documented gap (phase_3/PLAN.md §11) — the same call
    Flutter's `itemExtent` and react-window's `itemSize` make for their own first
    cut.

    **Where the scroll position comes from.** `_viewport.py` — the backend
    reports it, this reads it. Before any backend has reported one (the very
    first render, when nothing has been laid out yet and so no viewport height
    exists), a deliberately generous bootstrap window is built instead; the real
    window replaces it as soon as the viewport grows past what the bootstrap
    already covers. See `_window`.

    **`overscan` is the re-render frequency knob.** The rows built beyond the
    visible ones are slack: the window on screen is kept until the viewport
    reaches its edge, so scrolling re-runs `view()` about once per
    `overscan + 1` rows rather than once per row. Raising it trades rarer
    rebuilds for bigger ones. It did not work this way before
    phase_3/PLAN.md §40 — the window was recomputed and compared for equality,
    which moved it on every row boundary regardless of `overscan`, so the
    parameter bought nothing and cost a wider tree on every rebuild.

    **4 is the default because the trade is priced almost exactly evenly**, which
    §42 measured on a device rather than leaving to taste: over a 1,000-row list on
    a 60Hz phone, raising it to 8 halved the rebuild rate (6.7% -> 3.6% of frames)
    and cut late frames from 8.4% to 6.0% — and moved the average frame interval by
    0.03ms, because the rebuilds that remain are bigger and every ordinary frame
    now carries more nodes. At 16 it is worse outright (17.76ms against 17.37ms).
    Raise it if your `builder` is unusually cheap per row and you want fewer,
    larger pauses; it is not a way to make scrolling faster.
    """

    tag = "div"
    implied_classes = "flex flex-col overflow-auto min-h-0"

    def __init__(
        self,
        class_name: str = "",
        *,
        count: int,
        item_height: float,
        builder: Callable[[int], Child],
        overscan: int = 4,
        key: str | None = None,
        **props: Any,
    ):
        super().__init__(class_name, key=key, **props)
        if count < 0:
            raise ValueError(f"List(count=...) must not be negative, got {count}")
        if item_height <= 0:
            raise ValueError(f"List(item_height=...) must be positive, got {item_height}")
        if overscan < 0:
            raise ValueError(f"List(overscan=...) must not be negative, got {overscan}")
        if not callable(builder):
            raise TypeError("List(builder=...) must be callable: builder(index) -> Widget")
        # `gap-*` would silently break the spacer math rather than merely look
        # different. A non-virtualised list of N rows has N-1 gaps; a windowed one
        # has two spacers plus W rows, so W+1 gaps — so the container's real
        # scrollable extent would stop matching `count * item_height`, and every
        # row's on-screen position, `max_scroll_y`, and the window computed from
        # the resulting scroll offset would all drift apart from each other. That
        # is a genuinely confusing failure to debug from the symptom, and it is
        # cheap to refuse outright. Row spacing belongs *inside* `item_height`
        # (pad the row, not the list), the same call react-window's `itemSize` and
        # Flutter's `itemExtent` make.
        offending = [c for c in self.classes().split() if c == "gap" or c.startswith("gap-")]
        if offending:
            raise ValueError(
                f"List does not support {' '.join(offending)}: a gap between rows would make the "
                "container's scrollable height differ from count * item_height. Put the spacing "
                "inside each row instead (e.g. builder=lambda i: Row('py-2', ...)) and include it "
                "in item_height."
            )
        self.count = count
        self.item_height = float(item_height)
        self.builder = builder
        self.overscan = overscan

    # ---- window math (pure, host-testable without a RenderContext) ------------------

    def _window(self, node_id: str) -> tuple[int, int, float, float]:
        """`(first, last, top_pad_px, bottom_pad_px)` for this list's current
        scroll position — `[first, last)` half-open, in item indices.

        The range itself comes from `_viewport.compute_window`, which the scroll
        glue also calls to decide whether a scroll needs a re-render at all; see
        that function for the clamping rules and why they live there.

        A `scroll_y` past the end of the content yields an empty window whose
        padding still sums to the full content height. That's unreachable from
        the renderer (both scroll paths clamp to `max_scroll_y` — see
        `render_thread.rs`) and is left as the simple answer rather than
        special-cased into "show the last window".
        """
        reported = get_viewport(node_id)
        scroll_y, viewport_h = reported if reported is not None else (0.0, BOOTSTRAP_VIEWPORT_PX)
        first, last = compute_window(scroll_y, viewport_h, self.count, self.item_height, self.overscan)
        top_pad = first * self.item_height
        bottom_pad = (self.count - last) * self.item_height
        return first, last, top_pad, bottom_pad

    def window_signature(self, node_id: str) -> tuple[int, int]:
        """The `[first, last)` range this list would materialise right now."""
        first, last, _, _ = self._window(node_id)
        return first, last

    def _note(self, node_id: str, first: int, last: int) -> None:
        note_window(node_id, self.count, self.item_height, self.overscan, first, last)

    def _spacer_classes(self, pad: float) -> str:
        # `shrink-0` is load-bearing: this is a flex item in a flex-col container
        # that is deliberately shorter than its content, so without it flexbox
        # shrinks the spacers to make everything fit and the list stops scrolling
        # (the exact failure `ScrollView`'s own `min-h-0` note describes, from the
        # other direction).
        return f"shrink-0 h-[{_px(pad)}px]"

    def _row_classes(self) -> str:
        # `flex flex-col` is not decorative — it is the third instance in this file
        # of the same "no UA stylesheet" divergence `Button.implied_classes` and
        # `ScrollView.implied_classes` already document, and it was found the same
        # way: by looking at a conformance diff, not by reading the code.
        #
        # A plain `<div>` is `display:block` in a browser, so this wrapper's single
        # child is a block-level box that fills its width for free. `twcore`/
        # `zkite_renderer` has no block layout mode at all, and taffy's default is
        # `display:flex` with `flex-direction:row` — so natively the wrapper became a
        # *row* container and its one child was sized to its own content instead of
        # stretched, rendering every row visibly narrower than the WebView backend's.
        # Declaring the direction explicitly makes both backends agree, and makes the
        # cross-axis `stretch` that gives a row its full width the same in both.
        return f"shrink-0 overflow-hidden flex flex-col h-[{_px(self.item_height)}px]"

    def _build_row(self, index: int) -> Child:
        return self.builder(index)

    # ---- HTML (WebView backend) -----------------------------------------------------

    def _render_children(self, ctx: RenderContext, node_id: str) -> str:
        first, last, top_pad, bottom_pad = self._window(node_id)
        self._note(node_id, first, last)
        parts: list[str] = []
        if top_pad > 0:
            parts.append(f'<div id="{node_id}.pad-top" class="{self._spacer_classes(top_pad)}"></div>')
        for i in range(first, last):
            child = self._build_row(i)
            inner = child.render(ctx, f"{node_id}.i{i}.0") if isinstance(child, Widget) else html.escape(str(child))
            parts.append(f'<div id="{node_id}.i{i}" class="{self._row_classes()}">{inner}</div>')
        if bottom_pad > 0:
            parts.append(f'<div id="{node_id}.pad-bottom" class="{self._spacer_classes(bottom_pad)}"></div>')
        return "".join(parts)

    def _tag_attrs(self) -> dict[str, Any]:
        # Marks this element as one the WebView runtime must report scroll
        # position for (`zkite-runtime.js`'s `installScrollReporting`). A plain
        # `data-` attribute rather than an event in `data-zkite-on`, because
        # nothing in the *app* registered a handler for it — this is the runtime
        # talking to itself about a widget that needs feedback, not a user event.
        return {"data-zkite-virtual": "1"}

    # ---- native tree (phase 3 renderer) ---------------------------------------------

    def _tree_attrs(self) -> dict[str, Any]:
        # The native side already knows which containers scroll (it reads
        # `overflow` off the resolved style — see `zkite_renderer::
        # Renderer::is_scrollable_y`), so unlike the HTML path this needs no
        # marker attribute to be *found*. It reports its geometry anyway, because
        # a backend that reports every scrollable container's viewport uniformly
        # doesn't have to special-case which ones happen to care.
        return {"virtual_item_height": self.item_height, "virtual_count": self.count}

    def _tree_children(self, ctx: RenderContext, node_id: str) -> list[dict]:
        first, last, top_pad, bottom_pad = self._window(node_id)
        self._note(node_id, first, last)
        out: list[dict] = []
        if top_pad > 0:
            out.append(self._spacer_tree(f"{node_id}.pad-top", top_pad))
        for i in range(first, last):
            child = self._build_row(i)
            inner = (
                child.to_tree(ctx, f"{node_id}.i{i}.0")
                if isinstance(child, Widget)
                else {"tag": "#text", "text": str(child)}
            )
            # The wrapper's id is keyed on the *item index*, not on its position
            # in this window — so a row keeps one identity as the window slides
            # over it. That matters concretely on both backends: morphdom patches
            # rather than replaces an unchanged row (phase_1/PLAN.md §4), and
            # AccessKit derives a node id by hashing this string, so a screen
            # reader sees one stable node per item instead of every row appearing
            # to be replaced on every scroll (phase_3/PLAN.md §8.2).
            out.append({
                "tag": "div",
                "id": f"{node_id}.i{i}",
                "classes": self._row_classes(),
                "events": [],
                "attrs": {},
                "children": [inner],
            })
        if bottom_pad > 0:
            out.append(self._spacer_tree(f"{node_id}.pad-bottom", bottom_pad))
        return out

    def _spacer_tree(self, node_id: str, pad: float) -> dict:
        return {
            "tag": "div",
            "id": node_id,
            "classes": self._spacer_classes(pad),
            "events": [],
            "attrs": {},
            "children": [],
        }


# ---- text / leaf --------------------------------------------------------------------

class Text(Widget):
    tag = "span"
    implied_classes = "shrink-0"

    def __init__(self, class_name="", *children, wrap=True, multiline=True,
                 max_lines=None, overflow="clip", **props):
        if overflow not in ("clip", "ellipsis", "visible"):
            raise ValueError("Text overflow must be clip, ellipsis, or visible")
        if max_lines is not None and (isinstance(max_lines, bool) or not isinstance(max_lines, int) or max_lines < 1):
            raise ValueError("Text max_lines must be a positive integer")
        self.wrap = bool(wrap and multiline)
        self.max_lines = max_lines if multiline else 1
        self.overflow = overflow
        super().__init__(class_name, *children, **props)

    def _tree_classes(self):
        clip = "overflow-visible" if self.overflow == "visible" else "overflow-hidden"
        return f"flex flex-col min-w-0 {self.classes()} {clip}".strip()

    def _tree_children(self, ctx, node_id):
        children = super()._tree_children(ctx, node_id)
        if self.wrap and self.max_lines is None and self.overflow != "ellipsis":
            return children
        for child in children:
            if child["tag"] == "#text":
                child["attrs"] = {"text_wrap": self.wrap, "text_max_lines": self.max_lines,
                                  "text_ellipsis": self.overflow == "ellipsis"}
        return children

    def _tag_attrs(self):
        style = ["min-width:0", "max-width:100%", "overflow-wrap:anywhere",
                 "white-space:pre-wrap" if self.wrap else "white-space:nowrap",
                 "overflow:visible" if self.overflow == "visible" else "overflow:hidden"]
        if self.max_lines and self.wrap:
            style += ["display:-webkit-box", "-webkit-box-orient:vertical",
                      f"-webkit-line-clamp:{self.max_lines}"]
        else:
            style += ["display:block"]
        if self.overflow == "ellipsis":
            style += ["text-overflow:ellipsis"]
        return {"style": ";".join(style)}


class Button(Widget):
    tag = "button"
    # `flex items-center justify-center` is not decorative — it's replacing a UA
    # default. Every browser centers a plain <button>'s content, both axes, via
    # internal layout that predates and is independent of author CSS (confirmed
    # against Chromium's html.css and cross-engine bug reports on the behavior,
    # e.g. https://bugzilla.mozilla.org/show_bug.cgi?id=1397768 — "flex children of
    # <button> are always vertically centered, despite align-items or
    # justify-content"). The WebView backend inherits that for free. The native
    # renderer (phase_3) has no UA stylesheet to inherit anything from, so it must
    # be an explicit implied class here or the two backends silently diverge on
    # every button in every app — exactly the kind of gap plan1.md §5's
    # per-widget conformance testing exists to catch. See phase_3/PLAN.md §3.
    implied_classes = "appearance-none flex items-center justify-center"

    def _tag_attrs(self) -> dict[str, Any]:
        return {"type": "button", "disabled": bool(self.props.get("disabled", False))}

    def _tree_attrs(self) -> dict[str, Any]:
        return {"disabled": bool(self.props.get("disabled", False))}


class Icon(Widget):
    """No children; renders a single <use> against a bundled sprite sheet."""

    tag = "svg"
    implied_classes = "inline-block w-[1em] h-[1em] fill-current"
    void = False

    def __init__(self, class_name: str = "", name: str = "", *, key: str | None = None, **props: Any):
        super().__init__(class_name, key=key, name=name, **props)
        self.name = name

    def _render_children(self, ctx: RenderContext, node_id: str) -> str:
        return f'<use href="#icon-{html.escape(self.name, quote=True)}"></use>'

    def _tree_classes(self) -> str:
        # `w-[1em]`/`h-[1em]`/`fill-current` are dropped rather than translated, and the
        # native side supplies both meanings itself — which is why this is no longer the
        # approximation it used to be.
        #
        # `twcore`'s arbitrary-value parser genuinely has no `em` unit (checked against
        # `arbitrary_len` in ../../phase_0/spike3_twcore/src/parse.rs — px/rem/dp/%/
        # unitless only), and it cannot usefully have one: `em` resolves against the
        # *inherited* font size, and a `ComputedStyle` is ancestry-free by design. So
        # this used to emit a hardcoded `w-[20px] h-[20px]`, which was right only when
        # the surrounding text happened to be 20px and silently wrong at every other
        # size — the conformance `icon` fixture used `text-[20px]` and so could never
        # have caught it (phase_3/PLAN.md §18). Emitting nothing instead lets
        # `zkite_renderer::Renderer::icon` — the one layer that has actually resolved
        # inheritance by this point — default each `auto` axis to the real resolved font
        # size, i.e. a true `1em`. An explicit `w-8 h-8` from the caller still wins, on
        # both backends.
        #
        # `fill-current` is likewise native-side rather than translated: the sprite is
        # painted in the node's own resolved text colour (SVG's `currentColor`), so
        # `Icon("text-white", "star")` works without a class `twcore` would have to
        # invent.
        return f"inline-block {self.class_name}".strip()

    def _tree_attrs(self) -> dict[str, Any]:
        return {"icon_name": self.name}

    def _tree_children(self, ctx: RenderContext, node_id: str) -> list[dict]:
        # Deliberately empty on both backends, and for the same reason on each: the
        # sprite is the content, and it is named rather than inlined. The WebView emits
        # `<use href="#icon-NAME">` against the sheet in `webruntime/index.html`; the
        # native side passes `_tree_attrs`' `icon_name` to `zkite_renderer::Renderer::
        # icon`, which looks up the same sheet transcribed by
        # `phase_3/renderer/scripts/gen_icons.py`. Neither backend needs a child node,
        # and shipping the path data through the tree JSON on every render would put
        # kilobytes of geometry on the hot path for something that never changes.
        return []


# ---- form controls (void elements) --------------------------------------------------

class TextField(Widget):
    tag = "input"
    implied_classes = "appearance-none outline-none"
    void = True

    def _tag_attrs(self) -> dict[str, Any]:
        return {
            "type": "password" if self.props.get("secure") else self.props.get("type", "text"),
            "value": self.props.get("value", ""),
            "placeholder": self.props.get("placeholder", ""),
        }

    def _tree_attrs(self) -> dict[str, Any]:
        # No synthetic `#text` child here (unlike Switch/Slider below): what to
        # *display* — the value, or a dimmed placeholder when it's empty — and where
        # to draw a cursor/selection is IME/focus state that only the native side
        # tracks (phase_3/PLAN.md §5's IME item), so the Rust bridge owns rendering
        # this node's text itself rather than Python baking a snapshot into the tree.
        return {
            "input_type": "password" if self.props.get("secure") else self.props.get("type", "text"),
            "value": str(self.props.get("value", "")),
            "placeholder": str(self.props.get("placeholder", "")),
        }


class Image(Widget):
    """A bitmap. Paints on both backends as of phase_3/PLAN.md §35.

        Image("w-32 h-24 rounded-lg", src="logo.png", alt="Our logo")

    HTTPS PNG/JPEG URLs load asynchronously (up to 2 MiB on native).
    Declare the Android INTERNET permission. Give the widget explicit dimensions.

    `src` is either a path relative to the app's own directory — a file sitting next
    to `main.py`, which `zkite build` now stages (it silently dropped every non-`.py`
    file before §35) — or a `data:` URI, which is the one form that needs no staging
    at all.

    **The box is sized by its classes, never by the bitmap.** A missing file, a
    corrupt file and a 4000x3000 photo all lay out identically, and a decode cannot
    reflow the page. That is a deliberate difference from a browser, where an `<img>`
    with no dimensions takes its intrinsic size once loaded; give an `Image` a `w-`
    and an `h-` (or let a flex parent size it) as you would any other box.
    """

    tag = "img"
    #: `object-cover` matches what a browser's own `<img>` sizing most resembles once
    #: a box is imposed on it, and it is what every `Image` gets unless the author
    #: says otherwise — see `classes()` for why "unless" needs code.
    implied_classes = "object-cover"
    void = True

    def classes(self) -> str:
        """`Widget.classes` minus the implied `object-cover` when the author set their
        own `object-*`.

        **This one line is a real divergence fix, not tidiness** (phase_3/PLAN.md
        §35.4). `Widget.classes` emits `implied_classes` first and the author's string
        second. On native that is enough — `twcore` applies declarations in string
        order, so the later one wins — but on the WebView *both* classes reach the DOM
        and the **CSS cascade** decides, which has nothing to do with attribute order.
        Measured against the real vendored Tailwind bundle in real Chromium:
        `class="object-cover object-contain"` and `class="object-contain object-cover"`
        **both** compute `object-fit: cover`, because Tailwind emits `.object-cover`
        later in its stylesheet.

        So before this, `Image("object-contain", ...)` was letterboxed on native and
        cropped on the WebView, from identical user code — the silent-divergence shape
        §15.1 exists to track, and it was found by the conformance fixture added in the
        same session rather than by anyone reading this class.

        Dropping the implied class instead of reordering is what makes both backends
        agree: with only one `object-*` in the string there is no cascade question to
        get wrong, on either side.
        """
        authored = self.class_name or ""
        if any(c.startswith("object-") for c in authored.split()):
            return authored.strip()
        return super().classes()

    def _tag_attrs(self) -> dict[str, Any]:
        return {"src": self.props.get("src", ""), "alt": self.props.get("alt", "")}

    def _tree_attrs(self) -> dict[str, Any]:
        from ._remote_images import source
        return {"src": source(self.props.get("src", "")), "alt": self.props.get("alt", "")}


class Switch(Widget):
    """A switch. Optionally carries its own label as content (phase_3/PLAN.md §32).

        Switch("text-sm text-white", "Wi-Fi", checked=s.on, on_change=...)

    **The labelled form is composed out of two things that already worked on both
    backends, not out of HTML's `<label>`.** §30.8 recorded the one-line pattern that
    made a label tappable — a `Row(on_tap=...)` wrapping a *presentational* control and
    a `Text` — and measured it live on the Moto G54 (§30.4 cases 5/6). That is exactly
    what this renders: a tappable container, this same box drawn presentationally, and
    the caller's content. The user no longer writes it, so the double-handler ordering
    trap §30.8 documents cannot arise — there is only ever one handler.

    `<label>` was the obvious alternative and was rejected on a *measurement*, not on
    taste. Driven against real Chromium (§32.1): activating a `<label>` dispatches a
    **second, `isTrusted` click** on the contained input, so an ancestor `on_tap` would
    have fired twice on the WebView backend and once on native — a new silent
    divergence, in precisely the class §15.1 exists to track — and it would have had to
    be policed with a `detail === 0` guard in `zkite-runtime.js`. Composing the control
    in Python instead needs no runtime change on either backend.

    **What the two forms share, and why that is structural.** The box is drawn by
    `_box_classes`/`_box_children`, which both forms call, so a labelled control and a
    bare one cannot drift in appearance — the same reason §30.1 made `Checkbox`/`Radio`
    subclasses rather than copies. The labelled form keeps the real `<input>` on the
    HTML path so `index.html`'s existing CSS still draws it; only its *semantics* move
    outwards, via `aria-hidden`.
    """

    tag = "input"
    implied_classes = "appearance-none"
    void = True

    def __init__(self, class_name: str = "", *children: Child, **props: Any):
        super().__init__(class_name, *children, **props)
        if self._is_container():
            # Set on the *instance*, so `Widget.render`/`Widget.to_tree` need no
            # labelled-vs-bare branch of their own — the generic walk already reads
            # `self.tag`/`self.void`. An `<input>` is a void element and cannot contain
            # a label, so the labelled form has to be a container; everything else
            # about it is decided by the overrides below.
            self.tag = "span"
            self.void = False

    def _indicator(self) -> bool:
        """Whether to draw the built-in box/circle/track. `indicator=False` drops it.

        The case this exists for is a multiple-choice question, a segmented control or
        a colour swatch — where the *selection* is shown by styling the option itself
        rather than by a 20dp circle beside it. Without this the only way to build one
        was a tappable `Row`, which throws away the entire reason these widgets exist:
        a `Row` is announced to a screen reader as generic clickable text, while this
        keeps `role`, `aria-checked`/`checked` and the accessible name, so TalkBack
        still says "Option B, radio button, selected".

        It composes with the controlled contract rather than adding anything to it:
        `checked=` already tells the widget its state, and the caller derives their own
        class string from the same store field, so there is no new mechanism here.
        """
        return bool(self.props.get("indicator", True))

    def _is_container(self) -> bool:
        """Whether this renders as a container rather than as a bare `<input>`.

        True when there is content to hold, and also when the indicator is suppressed —
        a control with no box *is* nothing but its container, and a bare `<input>` has
        nowhere to put the caller's styling. `indicator=False` with no content is
        legitimate and is the colour-swatch case: the container is the whole widget.
        """
        return bool(self.children) or not self._indicator()

    # ARIA's own role for a checkbox presented as a switch, and the discriminator both
    # backends use to tell the three checkbox-shaped controls apart (phase_3/PLAN.md
    # §30). It is emitted on *both* paths deliberately: on the HTML side it scopes
    # `index.html`'s switch CSS so a `Checkbox` does not inherit a track and a thumb,
    # and it is the correct accessible role for this control besides; on the tree side
    # `zkite_pybridge::a11y_role` reads it to choose between `Role::Switch`,
    # `Role::CheckBox` and `Role::RadioButton`, which `tag`+`checked` alone cannot
    # distinguish — all three are `<input type=checkbox|radio>` carrying `checked`.
    #
    # Reading a *role* rather than a ZKite-specific marker is the §26 principle held to:
    # it means something to the platform on both sides rather than only to the native
    # renderer.
    aria_role = "switch"

    def _box_tag_attrs(self) -> dict[str, Any]:
        """The real `<input>`'s HTML attributes. Overridden by `Radio` for its `type`."""
        return {
            "type": "checkbox",
            "role": self.aria_role,
            "checked": bool(self.props.get("checked", False)),
        }

    def _tag_attrs(self) -> dict[str, Any]:
        if self._is_container():
            # The container is the control now, so it states the same two facts in
            # ARIA's own spelling for a non-`<input>` element. This is the standard
            # accessible-checkbox contract, not an imitation of one.
            return {
                "role": self.aria_role,
                "aria-checked": "true" if self.props.get("checked", False) else "false",
            }
        return self._box_tag_attrs()

    def _tree_attrs(self) -> dict[str, Any]:
        return {"role": self.aria_role, "checked": bool(self.props.get("checked", False))}

    def _payload_parser(self, event: str) -> Callable[[Any], Any] | None:
        # Delegates rather than returning `None`: the base types `animend` for every
        # widget (§47), and an override that answered for *all* events would quietly
        # un-type it here — a `Switch` with an `anim=` would hand its handler raw JSON
        # text while every other widget handed it an `AnimEnd`.
        return _as_bool if event == "change" else super()._payload_parser(event)

    # ---- the box, shared by both forms ---------------------------------------------
    # Split out of `classes()`/`_tree_children()` so the labelled and bare forms draw
    # the *same* box from the *same* declaration. `Checkbox`/`Radio` override only
    # these two, which is why neither of them needs a `classes()` of its own any more.

    def _box_classes(self) -> str:
        # Native-only visual: the WebView backend renders a real `<input
        # type=checkbox>` and never sees these; the native side has no default
        # checkbox control to fall back on, so the track *is* this node (sized/
        # colored by state here) and the thumb is one synthetic child below —
        # positioned by plain `justify-end`/`justify-start`, not a percentage
        # transform or new Rust paint primitive. See phase_3/PLAN.md §5.
        checked = bool(self.props.get("checked", False))
        state = "bg-indigo-500 justify-end" if checked else "bg-slate-700 justify-start"
        return f"flex items-center w-[44px] h-[24px] rounded-full p-[2px] {state}"

    def _box_children(self, box_id: str) -> list[dict]:
        return [{
            "tag": "div",
            "id": f"{box_id}.thumb",
            "classes": "w-[20px] h-[20px] rounded-full bg-white",
            "events": [],
            "attrs": {},
            "children": [],
        }]

    # ---- the labelled form ----------------------------------------------------------

    def _next_checked(self) -> bool:
        """What a tap on this control sets `checked` to.

        Overridden by `Radio`, which always sets `True`. The *bare* form's copy of this
        rule lives in `zkite_pybridge::Control` (native) and in the DOM (WebView), which
        is two implementations of one rule already; this is a third and it is pinned on
        its own side (`test_a_labelled_radio_always_sends_true`) exactly the way
        §31.6's digest pins its two. One spec, tested per implementation.
        """
        return not bool(self.props.get("checked", False))

    def _box_id(self, node_id: str) -> str:
        return f"{node_id}.box"

    def _box_full_classes(self) -> str:
        # `shrink-0` so a long label cannot squeeze the fixed-size box; `mr-2` for the
        # gap. The gap is a **margin on the box**, deliberately not `gap-2` on the
        # container: `twcore` resolves a class string last-wins while real Tailwind
        # resolves by stylesheet order, so an implied `gap-2` that a caller overrode
        # with `gap-1` would be 4px on native and 8px on the WebView — the exact
        # footgun `ScrollView.__init__` documents. A margin has nothing to conflict
        # with, so a caller's own `gap-*` is simply additive and identical on both.
        return f"{self.implied_classes} {self._box_classes()} shrink-0 mr-2".strip()

    def classes(self) -> str:
        if self._is_container():
            # `implied_classes` (`appearance-none`) deliberately absent: the outer node
            # is a container, not an input, and the box below carries it instead.
            return f"{self.class_name} inline-flex flex-row items-center".strip()
        return f"{Widget.classes(self)} {self._box_classes()}".strip()

    def _register_events(self, ctx: RenderContext, node_id: str) -> list[str]:
        """Bare form: `on_change` -> a `"change"` event, exactly as before.

        **Labelled form: `on_change` -> a `"tap"` event**, because the whole node —
        box and label together — is one tap target, and `"tap"` is the event both
        runtimes already dispatch to an arbitrary container. Nothing in `zkite_pybridge`,
        `render_thread.rs` or `zkite-runtime.js` had to learn anything for this.

        The *new value* is computed here, at tree-build time, and delivered through the
        payload parser — so the handler receives the same real `bool` it receives from
        the bare form, and the `""` payload both backends send for a tap (see
        `render_thread.rs::dispatch_tap_to` and `zkite-runtime.js`'s `extractPayload`,
        which produce the identical empty string) is discarded rather than parsed.
        """
        disabled = self._disabled_events(ctx, node_id)
        if disabled is not None:
            return disabled
        if not self._is_container():
            return super()._register_events(ctx, node_id)
        # Deliberately *not* `super()._register_events()` plus a filter on its return
        # value: that method registers into `ctx` as a side effect, so filtering its
        # result would leave a live `"change"` handler on this node that nothing
        # dispatches and that a future caller could reach. Caught by
        # `test_the_labelled_form_registers_no_change_event`.
        #
        # `change` and `tap` are both owned by the control itself; any other `on_*`
        # registers exactly as it would on any widget.
        events = []
        for prop, handler in self.props.items():
            if not (prop.startswith("on_") and callable(handler)):
                continue
            event = prop[3:]
            if event in ("change", "tap"):
                continue
            ctx.register(node_id, event, handler, self._payload_parser(event))
            events.append(event)
        on_change = self.props.get("on_change")
        if callable(on_change):
            next_value = self._next_checked()
            ctx.register(node_id, "tap", on_change, lambda _payload: next_value)
            events.append("tap")
        return sorted(events)

    def _render_children(self, ctx: RenderContext, node_id: str) -> str:
        # The real `<input>` is kept on the HTML path so `index.html`'s existing
        # switch/checkbox CSS still draws it — the labelled and bare forms therefore
        # render the *same* pixels on this backend, rather than the labelled one
        # becoming a second hand-drawn approximation (§30.2's seam is not widened).
        # `aria-hidden` is what moves the *semantics* out to the container, so the
        # accessibility tree has exactly one checkbox rather than two nested ones.
        if not self._indicator():
            # `indicator=False` — the caller is drawing the selected state themselves
            # (an MCQ option, a segmented control, a colour swatch). The container
            # keeps the role, the state and the tap; only the box goes away.
            return super()._render_children(ctx, node_id)
        box = {
            "id": self._box_id(node_id),
            "class": self._box_full_classes(),
            **self._box_tag_attrs(),
            "aria-hidden": "true",
            "tabindex": "-1",
        }
        return f"<input{_attr_str(box)}>" + super()._render_children(ctx, node_id)

    def _tree_children(self, ctx: RenderContext, node_id: str) -> list[dict]:
        if not self._is_container():
            return self._box_children(node_id)
        if not self._indicator():
            return Widget._tree_children(self, ctx, node_id)
        box_id = self._box_id(node_id)
        return [
            {
                "tag": "input",
                "id": box_id,
                "classes": self._box_full_classes(),
                "events": [],
                # Same `aria_hidden` fact the HTML path states as `aria-hidden`, read
                # by `zkite_pybridge`'s `control_of` (so a tap on the box is never a
                # second dispatch target) and `build_a11y_node` (so it is transparent
                # to a screen reader). Stated on both paths rather than inferred from
                # "has a labelled ancestor", per §30.1's role-not-marker principle.
                "attrs": {**self._tree_attrs(), "aria_hidden": True},
                "children": self._box_children(box_id),
            },
            *Widget._tree_children(self, ctx, node_id),
        ]


class Checkbox(Switch):
    """A checkbox: a square box that shows a tick when checked.

    Deliberately a subclass of `Switch`, because everything except the drawing and the
    accessible role is genuinely identical — same `<input type=checkbox>` tag, same
    `checked` attribute, same `bool` payload, same controlled-widget contract, same
    native toggle dispatch (`zkite_pybridge::Control::Switch` covers both). Subclassing
    is what makes "they cannot diverge in the parts that are the same" structural
    rather than a thing to remember, which is the lesson phase_3/PLAN.md §26.2 paid
    for when `Switch`'s payload was broken on one backend only.

    The tick is `Icon("check")` on the *tree* path, i.e. the same sprite table
    `phase_3/renderer/scripts/gen_icons.py` transcribes from `index.html`'s own
    `<symbol>` sheet (§18.2), so the two backends draw the same geometry by
    construction. The HTML path cannot nest anything (an `<input>` is a void element)
    and draws its tick in CSS instead — the one place these two do have to be kept in
    step, which is why `checkbox_checked` is a conformance fixture (§30).
    """

    aria_role = "checkbox"

    def _box_classes(self) -> str:
        checked = bool(self.props.get("checked", False))
        state = "bg-indigo-500 border-indigo-500" if checked else "bg-transparent border-slate-500"
        return (
            "flex items-center justify-center "
            f"w-[20px] h-[20px] rounded-[4px] border-2 text-white {state}"
        )

    def _box_children(self, box_id: str) -> list[dict]:
        if not bool(self.props.get("checked", False)):
            return []
        # `w-[14px] h-[14px]` rather than the inherited `1em`: this box is a fixed
        # 20px regardless of surrounding text size (a checkbox that grew with body
        # text would stop lining up with its own label), so the tick has to be fixed
        # too. `fill-current` picks up `text-white` from `_box_classes()` above —
        # which is why the tick's colour is unaffected by whatever text colour the
        # caller puts on a *labelled* control's class string.
        return [{
            "tag": "svg",
            "id": f"{box_id}.tick",
            "classes": "w-[14px] h-[14px] fill-current",
            "events": [],
            "attrs": {"icon_name": "check"},
            "children": [],
        }]


class Radio(Switch):
    """One radio button: a circle with a filled dot when selected.

    **Controlled, and there is no group.** ZKite has no `name`-style grouping and does
    not need one: which radio is selected is a fact in your `@store`, so exclusivity is
    `checked=(s.choice == "a")` on each and the re-render deselects the others. That is
    the same contract every other ZKite control has (phase_3/PLAN.md §26.3), and it is
    why a radio in a list works without the framework tracking anything.

    The one behavioural difference from `Checkbox`, and it is real: **tapping a
    selected radio sends `True`, not `False`.** A real `<input type=radio>` cannot be
    unchecked by clicking it, and a radio that turned itself off would leave a group
    with nothing selected — a state the group's own store field usually cannot even
    represent. Enforced on the native side by `Control::Radio` rather than here,
    because the toggle-vs-set decision is made where the touch is handled.
    """

    aria_role = "radio"

    def _box_tag_attrs(self) -> dict[str, Any]:
        return {**super()._box_tag_attrs(), "type": "radio"}

    def _next_checked(self) -> bool:
        # The labelled form's copy of the rule this class's docstring states, and the
        # one behaviour that makes `Radio` more than a differently-drawn `Checkbox`.
        # The bare form's copy is `zkite_pybridge::Control::Radio` on native and the
        # DOM's own on the WebView; each is pinned by a test on its own side.
        return True

    def _box_classes(self) -> str:
        checked = bool(self.props.get("checked", False))
        border = "border-indigo-500" if checked else "border-slate-500"
        return (
            "flex items-center justify-center "
            f"w-[20px] h-[20px] rounded-full border-2 bg-transparent {border}"
        )

    def _box_children(self, box_id: str) -> list[dict]:
        if not bool(self.props.get("checked", False)):
            return []
        return [{
            "tag": "div",
            "id": f"{box_id}.dot",
            "classes": "w-[10px] h-[10px] rounded-full bg-indigo-500",
            "events": [],
            "attrs": {},
            "children": [],
        }]


class Slider(Widget):
    tag = "input"
    implied_classes = "appearance-none w-full"
    void = True

    def _tag_attrs(self) -> dict[str, Any]:
        # `--zkite-pct` feeds `webruntime/index.html`'s `::-webkit-slider-runnable-track`
        # gradient (see that file's own comment) — a plain range input exposes no other
        # way for CSS to know how far "filled" the track should be. HTML-only: the
        # native (`_tree_attrs`) path paints its own real fill node instead, so it has
        # no equivalent need.
        return {
            "type": "range",
            "min": self.props.get("min", 0),
            "max": self.props.get("max", 100),
            "step": self.props.get("step", 1),
            "value": self.props.get("value", 0),
            "style": f"--zkite-pct: {self._percent():.4f}%",
        }

    def _tree_attrs(self) -> dict[str, Any]:
        return {
            "min": float(self.props.get("min", 0)),
            "max": float(self.props.get("max", 100)),
            "step": float(self.props.get("step", 1)),
            "value": float(self.props.get("value", 0)),
        }

    def _payload_parser(self, event: str) -> Callable[[Any], Any] | None:
        if event != "change":
            return super()._payload_parser(event)  # see `Switch`'s note
        # Falls back to this slider's *current* value rather than to a constant: an
        # unparseable payload can only come from a bug in a bridge, and "no change"
        # is the one interpretation that can never be a wrong number.
        current = float(self.props.get("value", 0))
        return lambda v: _as_float(v, current)

    def classes(self) -> str:
        return f"{super().classes()} relative flex items-center h-[16px]".strip()

    def _percent(self) -> float:
        lo = float(self.props.get("min", 0))
        hi = float(self.props.get("max", 100))
        val = float(self.props.get("value", 0))
        if hi <= lo:
            return 0.0
        return max(0.0, min(1.0, (val - lo) / (hi - lo))) * 100.0

    def _tree_children(self, ctx: RenderContext, node_id: str) -> list[dict]:
        # Track (full width, this node's own height) -> fill (absolute within the
        # track, width = percent) -> thumb (absolute within *this* node, left =
        # percent, pulled back by a percentage of its own width).
        #
        # **That pull-back is what makes the thumb match a real `<input
        # type=range>`.** A browser's thumb travels over `track_width - thumb_width`,
        # so its left edge is at `pct% * (W - 16)` and it stays inside the track at
        # both ends; a bare `left-[pct%]` puts it at `pct% * W`, which is half a thumb
        # too far right everywhere and, at 100%, entirely off the end of the track.
        # Real Tailwind writes that as `-translate-x-1/2` — a percent-of-own-size
        # translate `twcore` does not implement (checked against
        # ../../phase_0/spike3_twcore/src/parse.rs's `translate` arm, pixel offsets
        # only) — and it was carried as a documented cosmetic gap for exactly that
        # reason.
        #
        # It stopped being cosmetic when phase_3/PLAN.md §26 made a drag able to reach
        # 100 on the native backend at all: a slider whose thumb leaves its own track
        # at maximum is a broken control, not a nit. The gap is closable without any
        # new `twcore` feature because `pct` is known *here*, at tree-build time, so
        # the percentage-of-own-size translate can be emitted as a plain pixel margin
        # (`-ml-[Npx]`, which twcore does parse). Measured against real Chromium
        # through §9's harness at 0/50/100: thumb x = 16..31 / 92..107 / 168..183 on
        # both backends, identical.
        #
        # HTML is unaffected — the WebView renders a real range input and never sees
        # these children.
        pct = self._percent()
        thumb_pull_px = pct * 0.16  # pct% of the 16px thumb width
        return [
            {
                "tag": "div", "id": f"{node_id}.track",
                "classes": "w-full h-[6px] rounded-full bg-slate-700 relative",
                "events": [], "attrs": {},
                "children": [{
                    "tag": "div", "id": f"{node_id}.fill",
                    "classes": f"absolute left-0 top-0 bottom-0 rounded-full bg-indigo-500 w-[{pct:.4f}%]",
                    "events": [], "attrs": {}, "children": [],
                }],
            },
            {
                "tag": "div", "id": f"{node_id}.thumb",
                "classes": (
                    f"absolute top-0 bottom-0 w-[16px] rounded-full bg-white "
                    f"left-[{pct:.4f}%] -ml-[{thumb_pull_px:.4f}px]"
                ),
                "events": [], "attrs": {}, "children": [],
            },
        ]


# ---- layers -----------------------------------------------------------------------

class Overlay(Widget):
    """A layer painted above the whole app, dismissed by a tap outside it.

        Overlay("rounded-2xl bg-slate-800 p-4",      # <- the panel: your classes
                Text("text-white", "Anything"),
                placement="items-center justify-center p-6",
                scrim="bg-black/50",
                on_dismiss=lambda: s.update(open=False))

    The substrate under `Select` (phase_3/PLAN.md §33), and the thing `Sheet`,
    `Dialog`, a menu or a tooltip would each be one composition over. It is public for
    that reason: a picker this framework does not ship should be buildable without
    reimplementing "paint above everything and dismiss on an outside tap".

    **Why it needs no renderer support at all.** Both backends already agree that a
    node emitted *later* in document order paints on top and is hit first —
    `zkite_renderer` paints in document order and `Renderer::hit_test` walks it in
    reverse, and the DOM does the same for positioned elements in tree order. So an
    overlay is not a new kind of thing, it is an ordinary subtree that `app.App`
    emits last. There is no `z-index` anywhere in this (`twcore` parses none — §31.8),
    no stacking context, and nothing was added to either paint path.

    **Three nodes, and each one is load-bearing:**

    - the **root** is full-bleed and carries `placement` — which is what makes the
      centred default a *default value* rather than a geometry baked into the widget.
      `items-center justify-center` centres it; `items-end` is a bottom sheet;
      `items-start` is a top menu. Anchoring to a trigger is the one placement out of
      reach, and deliberately so: nothing tells Python where a node landed on screen.
    - the **scrim** is a full-bleed sibling *under* the panel that carries the dismiss
      handler. Putting the handler here rather than on the root is what lets a tap on
      the panel do nothing without the panel having to register a no-op `tap` to
      absorb it — which would have made a listbox announce itself as clickable.
    - the **panel** is `relative`, and that word is doing real work. The scrim is
      absolutely positioned, and CSS paints *positioned* elements above non-positioned
      ones regardless of tree order — so a non-positioned panel would sit correctly on
      native (document order) and be covered by its own scrim on the WebView. Exactly
      the silent-divergence class §15.1 tracks. It is implied here rather than
      documented as a caller's responsibility, because a rule a caller can forget is a
      divergence waiting to happen.
    """

    tag = "div"
    # Only what must never change. Direction and alignment are deliberately absent:
    # `implied_classes` are concatenated *before* the caller's string, and real
    # Tailwind resolves a conflict by its own stylesheet order while `twcore` resolves
    # it last-wins — so an implied `flex-col` that a caller overrode with `flex-row`
    # would lay out differently on the two backends. `ScrollView.__init__` documents
    # the same footgun; `Switch` (§32.2) avoids it with a margin. Here the answer is
    # simply not to state a direction: taffy's default and CSS's `display:flex`
    # default are both `row`, so silence already agrees.
    implied_classes = "fixed inset-0 flex"
    #: Rendered by `app.App` at the root, not where it was written.
    renders_in_place = False

    def __init__(
        self,
        class_name: str = "",
        *children: Child,
        placement: str = "items-center justify-center p-6",
        scrim: str = "bg-black/50",
        on_dismiss: Callable[[], Any] | None = None,
        key: str | None = None,
        **props: Any,
    ):
        super().__init__(class_name, *children, key=key, **props)
        self.placement = placement
        self.scrim = scrim
        self.on_dismiss = on_dismiss

    # ---- in place: register, occupy nothing -----------------------------------------

    def _enlist(self, ctx: RenderContext) -> None:
        ctx.register_overlay(self)
        if self.on_dismiss is not None:
            # Back closes the overlay before anything outer sees the press. The
            # innermost-first ordering falls out of *where this was written* — `Nav`
            # registers as it renders a screen, so an overlay opened from inside that
            # screen registers afterwards and `dispatch_back` (which walks in reverse)
            # reaches it first. Nothing here tracks depth; see §16.1's layer 1.
            ctx.register_back(self._dismiss)

    def _dismiss(self) -> bool:
        if self.on_dismiss is None:
            return False
        self.on_dismiss()
        return True

    def render(self, ctx: RenderContext, path: str) -> str:
        self._enlist(ctx)
        return ""

    def to_tree(self, ctx: RenderContext, path: str) -> dict | None:  # type: ignore[override]
        self._enlist(ctx)
        return None

    # ---- as a layer, from the app root ----------------------------------------------

    def layer_html(self, ctx: RenderContext, path: str) -> str:
        return Widget.render(self._layer(), ctx, path)

    def layer_tree(self, ctx: RenderContext, path: str) -> dict:
        return Widget.to_tree(self._layer(), ctx, path)

    def _layer(self) -> Widget:
        """This overlay as an ordinary widget tree — root, scrim, panel.

        Built out of plain `Widget`s rather than by hand-writing HTML and a tree dict
        separately, so the two render paths cannot drift: there is one structure and
        each backend's existing walk emits it in its own spelling.

        `implied_classes` is read explicitly rather than inherited, because the root
        emitted here is a `Widget` and not an `Overlay` — an `Overlay` never renders
        itself in place, so its own `classes()` is never the one that reaches a
        backend.
        """
        scrim = Widget(f"absolute inset-0 {self.scrim}".strip(), on_tap=self._dismiss)
        panel = Widget(f"{self.class_name} relative flex".strip(), *self.children, **self.props)
        return Widget(f"{self.implied_classes} {self.placement}".strip(), scrim, panel)


# ---- Select -----------------------------------------------------------------------

@_store_mod.store
class _SelectState:
    """Which uncontrolled `Select` is currently open, by node id (`""` for none).

    A real `@store` for two reasons beyond convenience. It re-renders on `update()`,
    which is the only way anything in this framework causes a frame — and it is
    defined in `zkite`, so `reload.py` (which re-executes and reshapes only the stores
    in the *reloaded* module) never touches it, exactly as `Navigator` is never
    touched (§16.2). An open picker therefore survives a hot reload.

    One field, not a set: only one picker can be open at a time, which is what a
    picker means. A `Select` given an explicit `open=` bypasses this entirely.
    """

    open_id: str = ""


class _ListBox(ScrollView):
    """The option list: a `ScrollView` that also says what it is.

    A `ScrollView` so a long option set scrolls rather than running off the panel, and
    so this inherits its `min-h-0`/`flex-col` fixes (§27.4, §31.2) instead of
    rediscovering them from a hand-written `overflow-auto`. The role is emitted on both
    paths for §30.1's reason: it means something to the platform on each side rather
    than only to this framework.
    """

    def _tag_attrs(self) -> dict[str, Any]:
        return {"role": "listbox"}

    def _tree_attrs(self) -> dict[str, Any]:
        return {"role": "listbox"}


class _Option(Widget):
    """One row of a `Select`'s popup. Carries the role, the selected state and the tap.

    Separate from the row's *appearance*, which is entirely the caller's (`option_class`
    / `option_selected_class` / the `option=` builder). That split is §32.5's
    `indicator=False` argument in another shape: the thing developers reach for instead
    — a tappable `Row` — is announced to a screen reader as generic clickable text, so
    the framework holds the semantics and hands back the drawing.
    """

    tag = "div"

    def _tag_attrs(self) -> dict[str, Any]:
        return {"role": "option", "aria-selected": "true" if self.props.get("selected") else "false"}

    def _tree_attrs(self) -> dict[str, Any]:
        return {"role": "option", "selected": bool(self.props.get("selected", False))}


class Select(Widget):
    """A picker: a trigger that opens a popup list of options.

        Select("px-4 py-3 rounded-xl bg-slate-800 text-white",
               options=["Brazil", "India", "Kenya"],
               value=s.country,
               on_change=lambda v: s.update(country=v))

    Options are either bare values (used as their own label) or `(value, label)`
    pairs. `value=` says which one is selected — the same controlled contract
    `Checkbox`/`Switch`/`Slider` already use, so the app's `@store` field stays the
    single source of truth.

    **Open state is uncontrolled by default**, because requiring a second store field
    and three more callbacks to show a dropdown is ceremony, not control. Pass `open=`
    to take it over; then `on_open`/`on_dismiss` are yours to handle and the widget
    stops touching its own state entirely.

    **Nothing about the popup's look is baked in.** `placement` decides where the panel
    sits (centred by default, `items-end` for a bottom sheet), `scrim` dims behind it
    (`scrim=""` keeps outside-tap dismissal with no dimming), `panel` styles the panel,
    `option_class`/`option_selected_class` style a row, and `option=` replaces a row's
    contents outright while keeping its role, selected state and tap. `header=` puts a
    widget above the list — a search field, typically. Passing children replaces the
    trigger's default content (the selected label and a chevron) the same way a label
    replaces a `Checkbox`'s (§32).

    The popup is an `Overlay`, which is public: a picker shaped differently from this
    one is buildable directly on it without reimplementing the layer.
    """

    tag = "div"
    aria_role = "combobox"
    #: Resolved once per render by `_prepare`, because whether an *uncontrolled*
    #: `Select` is open is a question about its own node id — which the generic
    #: `Widget` walk does not hand to `classes()`/`_tag_attrs()`/`_tree_attrs()`.
    _open: bool = False

    DEFAULT_PANEL = "w-full max-w-[420px] max-h-[420px] flex-col rounded-2xl bg-slate-800 p-2"
    DEFAULT_OPTION = "flex flex-row items-center justify-between rounded-xl px-4 py-3 text-white"
    DEFAULT_OPTION_SELECTED = "bg-slate-700"

    # ---- state ----------------------------------------------------------------------

    def _is_open(self, node_id: str) -> bool:
        declared = self.props.get("open")
        if declared is not None:
            return bool(declared)
        return _store_mod.get_instance(_SelectState).open_id == node_id

    def _set_open(self, node_id: str, want_open: bool) -> None:
        """Open or close, honouring whichever mode this `Select` is in.

        Controlled: call the caller's own `on_open`/`on_dismiss` and touch no state —
        the app owns the flag. Uncontrolled: write the shared store, whose `update()`
        is what actually causes the re-render.
        """
        if self.props.get("open") is not None:
            hook = self.props.get("on_open") if want_open else self.props.get("on_dismiss")
            if callable(hook):
                hook()
            return
        _store_mod.get_instance(_SelectState).update(open_id=node_id if want_open else "")

    # ---- options --------------------------------------------------------------------

    def _options(self) -> list[tuple[Any, str]]:
        out: list[tuple[Any, str]] = []
        for opt in self.props.get("options") or ():
            if isinstance(opt, (tuple, list)) and len(opt) == 2:
                out.append((opt[0], str(opt[1])))
            else:
                out.append((opt, str(opt)))
        return out

    def _selected_label(self) -> str | None:
        value = self.props.get("value")
        for opt_value, label in self._options():
            if opt_value == value:
                return label
        return None

    # ---- the trigger ----------------------------------------------------------------

    def classes(self) -> str:
        # The widget's own layout last, so it wins on both backends: `twcore` resolves
        # last-wins, and real Tailwind resolves by stylesheet order — stating these
        # after the caller's string is the arrangement §32's labelled container already
        # uses for exactly this reason.
        return f"{self.class_name} flex flex-row items-center justify-between".strip()

    def _tag_attrs(self) -> dict[str, Any]:
        return {
            "role": self.aria_role,
            "aria-haspopup": "listbox",
            "aria-expanded": "true" if self._open else "false",
        }

    def _tree_attrs(self) -> dict[str, Any]:
        return {"role": self.aria_role, "expanded": self._open}

    def _register_events(self, ctx: RenderContext, node_id: str) -> list[str]:
        """The trigger owns exactly one event: a `tap` that opens the popup.

        `on_change` belongs to the option rows, not here — registering it on the
        trigger too would give a tap on the trigger a second meaning. Any other `on_*`
        a caller passes registers normally, the same latitude `Switch` allows.
        """
        disabled = self._disabled_events(ctx, node_id)
        if disabled is not None:
            return disabled
        events = []
        for prop, handler in self.props.items():
            if not (prop.startswith("on_") and callable(handler)):
                continue
            event = prop[3:]
            if event in ("change", "tap", "open", "dismiss"):
                continue
            ctx.register(node_id, event, handler, self._payload_parser(event))
            events.append(event)
        is_open = self._open
        ctx.register(node_id, "tap", lambda: self._set_open(node_id, not is_open))
        events.append("tap")
        return sorted(events)

    def _trigger_children(self) -> list[Child]:
        if self.children:
            return list(self.children)
        label = self._selected_label()
        if label is None:
            label = str(self.props.get("placeholder", "Select…"))
        # `ml-2` rather than `gap-2` on the trigger: a margin has nothing to conflict
        # with, so a caller's own `gap-*` stays additive and identical on both backends
        # (§32.2 — the same reason the labelled control's box carries `mr-2`).
        return [Text("", label), Icon("ml-2 shrink-0", "chevron-down")]

    # ---- the popup ------------------------------------------------------------------

    def _option_rows(self, node_id: str) -> list[Child]:
        rows: list[Child] = []
        builder = self.props.get("option")
        base = str(self.props.get("option_class", self.DEFAULT_OPTION))
        selected_class = str(self.props.get("option_selected_class", self.DEFAULT_OPTION_SELECTED))
        for i, (value, label) in enumerate(self._options()):
            selected = value == self.props.get("value")
            classes = f"{base} {selected_class}".strip() if selected else base
            if callable(builder):
                content: list[Child] = [builder(value, label, selected)]
            else:
                content = [Text("", label)]
                if selected:
                    content.append(Icon("ml-2 shrink-0", "check"))
            rows.append(_Option(
                classes, *content,
                key=f"{node_id}.opt.{i}",
                selected=selected,
                on_tap=self._chooser(node_id, value),
            ))
        return rows

    def _chooser(self, node_id: str, value: Any) -> Callable[[], None]:
        """A **zero-argument** handler that already knows which option it belongs to.

        Deliberately not a one-parameter lambda closing over the value: `dispatch`
        hands a one-parameter handler the *bridge's* payload, and a tap's payload is
        the empty string on both backends — so the chosen value would arrive as `""`
        and `on_change` would be told the user picked nothing. That is §26.2's bug
        shape (`bool("false") is True`) for a third time, and it is why the value
        travels in a closure here instead of through a payload that has no room for
        it. Caught by the very first end-to-end run of this widget, not by a test.
        """
        return lambda: self._choose(node_id, value)

    def _choose(self, node_id: str, value: Any) -> None:
        on_change = self.props.get("on_change")
        if callable(on_change):
            invoke(on_change, value)
        self._set_open(node_id, False)

    def popup(self, node_id: str) -> Overlay:
        """The `Overlay` this `Select` opens, as it would be opened at `node_id`.

        Public so that a conformance fixture and a test can render *the real popup*
        rather than a second hand-authored copy of its markup — the failure mode
        §9.5 (a v3 palette against a v4 Tailwind) and §18.2 (two drawings of one
        sprite) both cost this project a session each. `node_id` is what the option
        ids are derived from, which is why it is a parameter rather than state.
        """
        header = self.props.get("header")
        body = _ListBox("w-full", *self._option_rows(node_id))
        children: list[Child] = [header, body] if header is not None else [body]
        return Overlay(
            str(self.props.get("panel", self.DEFAULT_PANEL)),
            *children,
            placement=str(self.props.get("placement", "items-center justify-center p-6")),
            scrim=str(self.props.get("scrim", "bg-black/50")),
            on_dismiss=lambda: self._set_open(node_id, False),
        )

    # ---- both render paths ------------------------------------------------------------

    def _prepare(self, ctx: RenderContext, node_id: str) -> None:
        """Resolve open state once, then enlist the popup if it is open.

        `_open` is an instance attribute because `classes()`/`_tag_attrs()`/
        `_tree_attrs()` are called by the generic `Widget` walk with no node id in
        hand, and the answer depends on one — an uncontrolled `Select` knows whether
        it is open only by comparing its own id against the store.
        """
        self._open = self._is_open(node_id)
        if self._open:
            self.popup(node_id).render(ctx, f"{node_id}.popup")

    def render(self, ctx: RenderContext, path: str) -> str:
        self._prepare(ctx, ctx.node_id(path, self.key))
        return super().render(ctx, path)

    def to_tree(self, ctx: RenderContext, path: str) -> dict:
        self._prepare(ctx, ctx.node_id(path, self.key))
        return super().to_tree(ctx, path)

    def _render_children(self, ctx: RenderContext, node_id: str) -> str:
        parts = []
        for i, c in enumerate(self._trigger_children()):
            parts.append(c.render(ctx, f"{node_id}.{i}") if isinstance(c, Widget) else html.escape(str(c)))
        return "".join(parts)

    def _tree_children(self, ctx: RenderContext, node_id: str) -> list[dict]:
        out: list[dict] = []
        for i, c in enumerate(self._trigger_children()):
            if isinstance(c, Widget):
                node = c.to_tree(ctx, f"{node_id}.{i}")
                if node is not None:
                    out.append(node)
            else:
                out.append({"tag": "#text", "text": str(c)})
        return out


__all__ = [
    "Widget",
    "RenderContext",
    "Column",
    "Row",
    "Grid",
    "Stack",
    "ScrollView",
    "Text",
    "Button",
    "Icon",
    "TextField",
    "Image",
    "Switch",
    "Checkbox",
    "Radio",
    "Slider",
    "List",
    "Overlay",
    "Select",
]
