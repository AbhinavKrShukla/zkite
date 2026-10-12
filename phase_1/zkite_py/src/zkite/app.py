"""Finds `view`, injects `@store` instances by annotation, renders, dispatches events.

This is the runtime that will eventually be driven from the JNI boot path (see
../PLAN.md M4) — everything here is transport-agnostic on purpose, so it can be
exercised entirely on the host first.
"""

from __future__ import annotations

import inspect
import threading
import traceback
import types
from typing import Any, Callable

from . import _store as _store_mod
from .errors import format_error_overlay, format_error_tree, log_error, log_recovered
from .widgets import RenderContext, Widget

_EMPTY_TREE: dict = {"tag": "div", "id": "r", "classes": "", "events": [], "attrs": {}, "children": []}


class App:
    def __init__(self, module: types.ModuleType):
        self.module = module
        self.view_fn = _find_view(module)
        self.ctx = RenderContext()
        self.last_html = ""
        self.last_tree: dict | None = None
        # The transport reads last_* after dispatch, including when the store update
        # failed to render. Keep the last good frame separately so repeated failures
        # do not recursively append error overlays to earlier error overlays.
        self._last_good_html = ""
        self._last_good_tree: dict | None = None
        self.last_error: str | None = None
        self._on_render: Callable[[str], None] | None = None
        self._on_render_tree: Callable[[dict], None] | None = None
        # Which representation `_render_all` (the store's callback, fired on every
        # `@store.update()`) recomputes. Sniffed from whichever of `render()`/
        # `render_tree()` runs first — `_zkite_setup` calls `render()` for the WebView
        # backends, `_zkite_setup_tree` calls `render_tree()` for the native one (see
        # runtime.py) — rather than always doing both: `view()` (a user function, and
        # potentially expensive/side-effectful) would otherwise run twice per state
        # change for every backend, even though only one representation is ever
        # actually consumed by a given process. Defaults to "html" so a caller that
        # never renders at all (shouldn't happen, but not worth a crash over) still
        # matches this class's pre-phase-3 behavior exactly.
        self._mode: str = "html"
        # Serialises `render`/`render_tree`/`dispatch` against each other.
        #
        # **Needed only since the push path existed** (phase_3/PLAN.md §34): before
        # it, every render was driven by one shell thread and a second one was
        # impossible by construction. Now a background thread's `@store.update()`
        # renders too, and `render_tree` is three statements that are individually
        # atomic and jointly not — `self.ctx = RenderContext()`, walk the widget tree
        # registering handlers into it, `self.last_tree = ...`. Two threads
        # interleaving there is not a torn *frame*, which would merely look wrong; it
        # is a torn **`ctx`**, where half the handlers a tap needs were registered
        # into an object that has already been replaced. The app renders correctly and
        # then silently ignores taps on some of its nodes, which is the §15.1 failure
        # shape exactly.
        #
        # Re-entrant because the common path is re-entrant: `dispatch` takes it, calls
        # a handler, the handler calls `update()`, and that renders. Plain `Lock` would
        # deadlock every event in the framework.
        #
        # The tradeoff, stated rather than discovered later: this is held across
        # `view()`, so a `view()` that blocks (a network call inline, rather than on a
        # thread) stalls the other thread's frame for exactly as long. That is already
        # a broken app — `view()` runs on every state change — and a stall is a
        # strictly better failure than a tap that does nothing.
        self._lock = threading.RLock()
        # Whether a render is on the stack right now. Only ever read or written under
        # `_lock`, so a plain bool is enough — another thread cannot be rendering
        # concurrently by construction.
        #
        # Guards the *re-entrant* case, which `_lock` deliberately does not: a `view()`
        # that calls `@store.update()` (starting a timer, kicking off a first fetch)
        # re-enters `_render_all` from inside `_run_view`, and a nested render would
        # replace `self.ctx` half way through the walk that is still using it — the
        # same torn-`ctx` failure `_lock` exists to prevent, arriving from one thread
        # instead of two. Skipping is not a compromise: the state was mutated *before*
        # the callback fired, so the render already in progress is the one that will
        # produce a tree containing it, and producing one tree instead of two
        # interleaved halves is exactly right. It also makes an unconditional
        # `update()` inside `view()` a no-op rather than unbounded recursion.
        #
        # Found by writing §34's own demo app, which called `update()` from `view()`
        # on its first draft.
        self._rendering = False
        _store_mod.set_render_callback(self._render_all)

    def on_render(self, callback: Callable[[str], None] | None) -> None:
        """Registered by the transport layer (dev-loop socket / JNI bridge in M4)."""
        self._on_render = callback

    def on_render_tree(self, callback: Callable[[dict], None] | None) -> None:
        """Registered by the native (phase 3) transport layer — `phase_3/pybridge` —
        parallel to `on_render` for the WebView/HTML one.
        """
        self._on_render_tree = callback

    def _render_all(self) -> None:
        """The store's render callback (`@store`'s `update()` calls this on every
        state change — see _store.py). Recomputes whichever representation this app
        is actually running as (see `_mode`) — `last_html`/`last_tree` must stay fresh
        after every state change regardless of whether a push callback (`on_render`/
        `on_render_tree`) happens to be registered, since `_zkite_dispatch`/
        `_zkite_dispatch_tree` (runtime.py) read `last_html`/`last_tree` directly rather
        than relying on the push path.
        """
        with self._lock:
            if self._rendering:
                return
            if self._mode == "tree":
                self.render_tree()
            else:
                self.render()

    def use_module(self, module: types.ModuleType) -> None:
        """Point the app at a freshly reloaded module (see reload.py)."""
        self.module = module
        self.view_fn = _find_view(module)

    def _run_view(self) -> Widget:
        kwargs = _resolve_store_args(self.view_fn)
        tree = self.view_fn(**kwargs)
        if not isinstance(tree, Widget):
            raise TypeError(f"view() must return a Widget, got {tree!r}")
        return tree

    def _overlay_layers(self, tree_mode: bool) -> list:
        """Render every overlay the view just registered, after the whole tree.

        Drained by index rather than iterated, because rendering one overlay can
        register another (a `Select` inside a `Sheet`, say) and the list grows while
        this runs. Ids are positional and stable across frames for a stable tree, the
        same assumption `List`'s viewport state already makes.

        The layers go *last* on purpose and that is the entire mechanism: both
        backends paint in document order and hit-test it in reverse, so last means on
        top and tapped first. See `widgets.RenderContext.register_overlay` and
        phase_3/PLAN.md §33.
        """
        out: list = []
        i = 0
        while i < len(self.ctx.overlays):
            overlay = self.ctx.overlays[i]
            path = f"__zkite_overlay.{i}"
            out.append(overlay.layer_tree(self.ctx, path) if tree_mode else overlay.layer_html(self.ctx, path))
            i += 1
        return out

    def _note_error(self, tb: str) -> None:
        """Record this frame's traceback, logging it only when it is a *new* one.

        The guard is the whole design. A broken `view()` raises on **every** frame, so
        logging unconditionally would put sixty tracebacks a second into logcat during
        a scroll on an app that is already unusable. Comparing against `last_error` —
        which this class already tracked for `zkite dev`'s socket — makes it one burst
        per distinct failure, and a *different* exception (or the same one from a new
        line after a hot reload) is a new burst because its formatted text differs.

        Deliberately not called from `dispatch`: a handler that raises is a different
        failure with a different surface, and `render_tree` sees whatever it does to
        the next frame anyway.
        """
        if tb != self.last_error:
            log_error(tb)
        self.last_error = tb

    def _note_rendered(self) -> None:
        """Record that this frame rendered cleanly, logging only the *recovery*."""
        if self.last_error is not None:
            log_recovered()
        self.last_error = None

    def render(self) -> str:
        # The lock and the `_rendering` flag are held *here*, around the original body,
        # rather than in a wrapper method — and that is not a style preference.
        # `view()`'s traceback is user-facing: it is what the on-device error overlay
        # shows, and every helper frame between this method and `_run_view` appears in
        # it by name. A first draft used two wrapper methods and moved the overlay's
        # top frame from `in render` to `in _render_html_body`, caught by diffing
        # `demo_app/error_repro.py`'s rendered overlay against HEAD (§34.6).
        with self._lock:
            self._rendering = True
            self._mode = "html"
            previous_ctx = self.ctx
            self.ctx = RenderContext()
            try:
                html_out = self._run_view().render(self.ctx, "r")
                # Appended as a sibling of the root inside `#zkite-root`, exactly where
                # `format_error_overlay` has always put the error layer — a
                # `fixed inset-0` node is out of flow, so the root's own layout is
                # untouched.
                html_out += "".join(self._overlay_layers(tree_mode=False))
                self._note_rendered()
                self._last_good_html = html_out
            except Exception:  # noqa: BLE001 - render loop must survive app bugs
                tb = traceback.format_exc()
                self._note_error(tb)
                self.ctx = previous_ctx
                html_out = (self._last_good_html or "<div></div>") + format_error_overlay(tb)
            finally:
                # Cleared before the push callback runs, not after: a sink is free to
                # do anything, including something that ends in another `update()`,
                # and that one is a genuinely new frame rather than a nested render.
                self._rendering = False
            self.last_html = html_out
        if self._on_render is not None:
            self._on_render(html_out)
        return html_out

    def render_tree(self) -> dict:
        """Native (phase 3) counterpart of `render()`: same view()/store resolution,
        same `RenderContext` (so a tap dispatches against the same handlers either
        backend registered), a structured tree instead of an HTML string. See
        `zkite.widgets.Widget.to_tree` and phase_3/PLAN.md §5.
        """
        with self._lock:
            self._rendering = True
            self._mode = "tree"
            previous_ctx = self.ctx
            self.ctx = RenderContext()
            try:
                tree_out = self._run_view().to_tree(self.ctx, "r")
                layers = self._overlay_layers(tree_mode=True)
                if layers:
                    # Wrapped only when there is something to layer, so an app with no
                    # overlay open emits byte-identical output to one built before this
                    # existed. The wrapper carries no id the view can see and the view's
                    # own root keeps its `"r"`, so no node id shifts when a popup opens
                    # — which matters because scroll offsets, `List` windows and IME
                    # focus are all keyed by node id and would reset if they did.
                    #
                    # `flex flex-col w-full h-full` mirrors `zkite_pybridge::build`'s own
                    # wrapper rather than the error layer's `relative w-full h-full`: a
                    # definite-size flex column is what makes the view's `flex-1` root
                    # resolve against the viewport exactly as it does without this
                    # wrapper.
                    tree_out = {
                        "tag": "div",
                        "id": "__zkite_root",
                        "classes": "flex flex-col w-full h-full",
                        "events": [],
                        "attrs": {},
                        "children": [tree_out, *layers],
                    }
                if self.ctx.system_bars is not None:
                    tree_out["attrs"] = {**tree_out.get("attrs", {}), "system_bars": self.ctx.system_bars}
                self._note_rendered()
                self._last_good_tree = tree_out
            except Exception:  # noqa: BLE001 - render loop must survive app bugs
                tb = traceback.format_exc()
                self._note_error(tb)
                self.ctx = previous_ctx
                base = self._last_good_tree or _EMPTY_TREE
                tree_out = {
                    "tag": "div", "id": "__zkite_root", "classes": "relative w-full h-full",
                    "events": [], "attrs": {}, "children": [base, format_error_tree(tb)],
                }
                if "system_bars" in base.get("attrs", {}):
                    tree_out["attrs"]["system_bars"] = base["attrs"]["system_bars"]
            finally:
                # Cleared before the push callback runs, not after: a sink is free to
                # do anything, including something that ends in another `update()`,
                # and that one is a genuinely new frame rather than a nested render.
                self._rendering = False
            self.last_tree = tree_out
        if self._on_render_tree is not None:
            self._on_render_tree(tree_out)
        return tree_out

    def dispatch(self, node_id: str, event: str, payload: Any = None) -> bool:
        with self._lock:
            return self.ctx.dispatch(node_id, event, payload)

    def dispatch_back(self) -> bool:
        """Offer a platform Back press to the current render's back chain.

        Returns `True` if something consumed it (a navigator popped a screen, say),
        `False` if the shell should do the platform default and exit the app. Both
        shells depend on that distinction being answered by Python rather than
        guessed at in Java — see `_nav.py` and phase_3/PLAN.md §16.

        Note that a consuming handler will already have re-rendered by the time this
        returns, because every navigator mutation goes through `@store.update()` and
        that is what fires the render callback. So the caller reads `last_tree`/
        `last_html` for the new frame rather than being handed one here, exactly as
        `dispatch` already works for a tap.
        """
        with self._lock:
            return self.ctx.dispatch_back()


def _find_view(module: types.ModuleType) -> Callable[..., Widget]:
    view_fn = getattr(module, "view", None)
    if view_fn is None or not callable(view_fn):
        raise AttributeError(
            f"{module.__name__} has no module-level `view` function "
            "(zkite's entry point — see plan1.md §8)"
        )
    return view_fn


def _resolve_store_args(view_fn: Callable[..., Widget]) -> dict[str, Any]:
    sig = _store_mod.signature_with_resolved_annotations(view_fn)
    kwargs: dict[str, Any] = {}
    for name, param in sig.parameters.items():
        ann = param.annotation
        if ann is inspect.Parameter.empty:
            raise TypeError(
                f"view() parameter '{name}' has no type annotation; zkite injects "
                "stores by annotation, e.g. `def view(c: Counter)`"
            )
        if not _store_mod.is_store(ann):
            raise TypeError(f"view() parameter '{name}: {ann}' is not a @store class")
        kwargs[name] = _store_mod.get_instance(ann)
    return kwargs


__all__ = ["App"]
