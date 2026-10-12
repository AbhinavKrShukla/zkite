"""Navigation: a back channel, a host protocol, and one stack navigator.

See phase_3/PLAN.md §16. plan1.md §8 names `Nav` in the v1 widget set and §10 files
"navigation with a real back stack" under phase 4; that ordering assumed phase 3
would not be shipping apps yet, and it now is (phase_3/PLAN.md §15.4 flipped the
default to the native renderer), so every new ZKite app lands on a backend where Back
exits the app and there is no second screen.

**This file is deliberately three layers, not one.** The instruction it was built to
was that a *different* navigation system has to be buildable on top of it later
without rewriting anything, and that none of that generality may cost anything per
frame. Those two pull in opposite directions unless mechanism and policy are
actually separated, so they are:

    1. The back channel  — a platform Back press reaches a chain of handlers,
       innermost first. Lives in `RenderContext` (see `widgets.py`) next to the
       event handlers, because it has exactly the same lifetime and the same
       "registered during a render, consulted after it" shape. No new lifecycle
       was invented for it.

    2. `NavigationHost`  — the contract a navigator satisfies: answer `handle_back`,
       and name the entry that should be rendered now. Anything satisfying it can
       be driven by `Nav`.

    3. `Navigator`       — the built-in *stack* policy, and the only policy here.

A tab navigator, a modal/sheet navigator, a nested-per-tab stack, or a URL router
are all further implementations of layer 2 and need no change to layers 1 or 3. A
router in particular falls out almost for free, because layer 2 already speaks in
*names and params* rather than in screen functions — see `Entry`.

**What this costs per frame: nothing measurable, by construction.** Only the top
entry of the stack is ever resolved and built (`Nav._current_widget`), so a screen
pushed five deep costs exactly what one pushed at the root does — a five-screen app
does not build five widget trees. The back channel is one list append per `Nav` per
render and is never touched again unless Back is actually pressed. Nothing here runs
on the render thread's hot path at all: a push is one `@store` update, which is the
same re-render any button tap already triggers (phase_3/PLAN.md §11.3's rule that
per-frame work must not grow applies here too).
"""

from __future__ import annotations

import dataclasses
import inspect
from typing import Any, Callable, Mapping, Protocol

from . import _store as _store_mod
from ._store import store
from .widgets import Child, RenderContext, Widget

# A screen is an ordinary view function: it takes stores by annotation exactly the
# way `view()` does, plus whatever params it was pushed with, and returns a Widget.
Screen = Callable[..., Widget]


# ---- layer 2: what a navigator has to be ------------------------------------------


@dataclasses.dataclass(frozen=True)
class Entry:
    """One item on a navigation stack: which screen, and what it was pushed with.

    **`screen` is a name, never a function object, and that is the load-bearing
    decision in this file.** Hot reload re-executes the module and replaces every
    function object in it (phase_1/PLAN.md §4), so a stack holding function objects
    would keep rendering the *old* code for every screen already pushed — the reload
    would appear to work on whichever screen happened to be the root and silently
    not on any other. Names are resolved late, by `Nav`, against the screen list the
    freshly-run `view()` just handed it, so a pushed screen picks up an edit exactly
    like the root one does.

    It is also what makes a URL router buildable on this later without touching any
    of it: `(name, params)` is already what a route resolves *to*, so a router is a
    `NavigationHost` that parses "/item/42" into this and nothing more.

    `params` is a plain dict rather than something frozen: it is passed straight to
    the screen function as keyword arguments, and requiring hashable values there
    would be a restriction with nothing behind it. The dataclass is frozen so the
    *entry* cannot be reseated in place, which is the part that would corrupt a
    stack; mutating a params dict after the fact is possible and is the caller's
    business, the same as any other value they put in a store.
    """

    screen: str
    params: Mapping[str, Any] = dataclasses.field(default_factory=dict)


class NavigationHost(Protocol):
    """The contract `Nav` drives. Implement this to build a different navigator.

    Deliberately two methods and no more. Everything else a navigator might want
    (depth, a `can_pop` flag, transitions, per-entry keep-alive) is that navigator's
    own business and not something `Nav` needs to know to render it.
    """

    def current_entry(self) -> Entry | None:
        """The entry to render right now, or `None` if nothing has been pushed."""

    def handle_back(self) -> bool:
        """Consume a Back press. `True` if it was consumed, `False` to pass it on.

        Returning `False` on the last screen is what lets Back exit the app, which
        is the behaviour a user expects and the reason this returns a bool rather
        than popping unconditionally.
        """


def screen_name(screen: Screen | str) -> str:
    """Accept either a screen function or its name, and return the name.

    Both forms exist because `nav.push(settings)` is the better thing to write (it
    is checkable by an editor and cannot be misspelled silently) while
    `nav.push("settings")` is what a router or a restored-from-disk stack will have.
    They resolve to the same entry, so the two are interchangeable at every call
    site rather than being two parallel APIs.
    """
    if isinstance(screen, str):
        return screen
    name = getattr(screen, "__name__", None)
    if not name:
        raise TypeError(
            f"navigation screen must be a function or a name, got {screen!r}"
        )
    return name


# ---- layer 3: the built-in stack policy -------------------------------------------


@store
class Navigator:
    """The built-in stack navigator. A `@store`, so it survives hot reload for free.

    Being a real `@store` is not a shortcut, it is the whole reason nav state
    behaves correctly across a reload: `reload.py` only re-executes and reshapes
    stores defined *in the reloaded module*, and this one is defined here in `zkite`,
    so it is simply never touched — you stay on the screen you were on while its
    code updates underneath you. That is the same property phase_1/PLAN.md §4 built
    for app state, inherited rather than reimplemented.

    Injected by annotation like any other store, so a screen that needs to navigate
    just asks for it:

        def home(nav: Navigator):
            return Button("...", "Settings", on_tap=lambda: nav.push(settings))

    Every mutating method goes through `update()` (`@store`'s own setter), so a
    navigation is exactly one state change and one re-render — the same path a
    counter increment already takes, with no separate notification mechanism.
    """

    stack: list[Entry] = dataclasses.field(default_factory=list)

    # ---- NavigationHost ----------------------------------------------------------

    def current_entry(self) -> Entry | None:
        return self.stack[-1] if self.stack else None

    def handle_back(self) -> bool:
        """Pop, unless this is the last screen — then let the platform have it."""
        if len(self.stack) <= 1:
            return False
        self.pop()
        return True

    # ---- the imperative API ------------------------------------------------------

    @property
    def depth(self) -> int:
        return len(self.stack)

    @property
    def can_pop(self) -> bool:
        return len(self.stack) > 1

    @property
    def current(self) -> str | None:
        entry = self.current_entry()
        return entry.screen if entry is not None else None

    def push(self, screen: Screen | str, **params: Any) -> None:
        """Put a screen on top of the stack."""
        self.update(stack=[*self.stack, Entry(screen_name(screen), params)])

    def pop(self) -> bool:
        """Remove the top screen. `False` (and no re-render) if it was the last one.

        Refusing to pop the root is deliberate rather than defensive: a `Nav` with
        an empty stack has nothing to render, and every way of reaching that state
        is a bug in the caller that would surface as a blank screen somewhere else
        entirely. `handle_back` returns `False` in the same situation so the
        platform can exit the app, which is the one correct response to "back out
        of the root screen".
        """
        if len(self.stack) <= 1:
            return False
        self.update(stack=self.stack[:-1])
        return True

    def replace(self, screen: Screen | str, **params: Any) -> None:
        """Swap the top screen without growing the stack."""
        self.update(stack=[*self.stack[:-1], Entry(screen_name(screen), params)])

    def pop_to_root(self) -> bool:
        if len(self.stack) <= 1:
            return False
        self.update(stack=self.stack[:1])
        return True

    def reset(self, screen: Screen | str, **params: Any) -> None:
        """Clear the stack down to a single screen — for a sign-out, or a deep link."""
        self.update(stack=[Entry(screen_name(screen), params)])

    # ---- seeding -----------------------------------------------------------------

    def seed(self, screen: Screen | str) -> None:
        """Set the root screen if the stack is empty, *without* re-rendering.

        Called by `Nav` mid-render, which is exactly why it bypasses `update()`:
        `update()` fires the store's render callback, and re-entering a render from
        inside one would recurse. Assigning the field directly is correct here
        because the render this is running inside is about to read the value anyway
        — the frame being built is the one that will show it.
        """
        if not self.stack:
            self.stack = [Entry(screen_name(screen), {})]


# ---- the widget --------------------------------------------------------------------


class Nav(Widget):
    """Renders the current screen of a `NavigationHost`, and pops it on Back.

        def view():
            return Nav("flex-1", home, settings)

    The first screen listed is the root. Screens are ordinary view functions and
    are identified by their own `__name__`, so `nav.push(settings)` and
    `nav.push("settings")` both reach the second one.

    **Only the current screen is built.** `Nav`'s children are screen *functions*,
    not widgets, and exactly one of them is called per render — so stack depth
    costs nothing, and a screen that is three levels down is not paying to keep the
    two below it alive. This is also why there are no transition animations here:
    animating between screens needs both trees alive at once, and there is no
    animation driver anywhere in the framework yet (phase_3/PLAN.md §15.1). Building
    half of it would have meant keeping every screen materialised for a feature that
    cannot currently run.

    A different navigator is passed with `host=`; the default is the process-wide
    `Navigator` singleton, which is the same instance a screen gets injected by
    annotation.
    """

    tag = "div"
    # `min-h-0` is here for the same reason `ScrollView` and `List` carry it: a flex
    # item's default `min-height: auto` refuses to shrink below its content, so a
    # `Nav("flex-1", ...)` whose screen contains a tall scroll region grows to that
    # region's *full* height instead of the viewport's — and the screen's own footer
    # (a tab bar, typically) is pushed off the bottom of the display with nothing to
    # indicate it. Real CSS behaviour, identical on both backends, and silent.
    #
    # Found in a real app: a `List` of 600 rows inside a `Nav` laid the screen out
    # 30614dp tall against a 960dp viewport. `Nav` is a full-screen host by
    # construction, so it should never be the ancestor that forgets this.
    #
    # Note this fixes only *this* level. WIDGETS.md's rule still stands for the
    # user's own containers: every intermediate `flex-1` ancestor between a scroll
    # region and its nearest non-growing parent needs `min-h-0` too, including the
    # `Column` a screen returns.
    implied_classes = "flex flex-col min-h-0"

    def __init__(
        self,
        class_name: str = "",
        *screens: Screen,
        host: NavigationHost | None = None,
        key: str | None = None,
        **props: Any,
    ):
        super().__init__(class_name, key=key, **props)
        if not screens:
            raise ValueError(
                "Nav(...) needs at least one screen: Nav('flex-1', home, settings)"
            )
        self.screens: dict[str, Screen] = {}
        for s in screens:
            if not callable(s):
                raise TypeError(
                    f"Nav(...) screens must be view functions, got {s!r} — "
                    "a screen takes stores by annotation and returns a Widget"
                )
            name = screen_name(s)
            if name in self.screens:
                raise ValueError(
                    f"Nav(...) has two screens named {name!r}; screens are identified "
                    "by name, so they have to be distinct"
                )
            self.screens[name] = s
        self.root_name = screen_name(screens[0])
        self.host: NavigationHost = (
            host if host is not None else _store_mod.get_instance(Navigator)
        )

    # ---- resolution ----------------------------------------------------------------

    def _current_widget(self) -> Child:
        """Resolve the host's current entry to a built widget.

        The `seed` call is what makes a first render work without the app having to
        push anything: an untouched `Navigator` has an empty stack, and the root is
        whichever screen was listed first.
        """
        seed = getattr(self.host, "seed", None)
        if callable(seed):
            seed(self.root_name)
        entry = self.host.current_entry()
        if entry is None:
            # A custom host that declines to seed itself. Nothing to render is a
            # legitimate state for it to be in (an empty tab, say), so this is an
            # empty box rather than an error.
            return None
        fn = self.screens.get(entry.screen)
        if fn is None:
            raise KeyError(
                f"Nav(...) was asked for screen {entry.screen!r}, which it does not "
                f"have. Known screens: {', '.join(sorted(self.screens)) or '(none)'}. "
                "Every screen a navigator can reach has to be listed on the Nav that "
                "renders it."
            )
        return fn(**_resolve_screen_args(fn, entry.params))

    def _register_back(self, ctx: RenderContext) -> None:
        ctx.register_back(self.host.handle_back)

    # ---- the two render paths ------------------------------------------------------
    # Registration happens *before* the screen is built, so a nested Nav inside that
    # screen registers after this one and is therefore consulted first — see
    # `RenderContext.dispatch_back`. Nesting works out for free from the walk order
    # rather than needing anything to track depth.

    def _render_children(self, ctx: RenderContext, node_id: str) -> str:
        self._register_back(ctx)
        child = self._current_widget()
        if child is None:
            return ""
        return child.render(ctx, f"{node_id}.0")

    def _tree_children(self, ctx: RenderContext, node_id: str) -> list[dict]:
        self._register_back(ctx)
        child = self._current_widget()
        if child is None:
            return []
        return [child.to_tree(ctx, f"{node_id}.0")]


def _resolve_screen_args(fn: Screen, params: Mapping[str, Any]) -> dict[str, Any]:
    """Bind a screen function's parameters: stores by annotation, the rest by name.

    A screen is a view function, so store injection has to work identically to
    `view()`'s (`app.py`) — that is the property that makes "a screen is just a
    view" true rather than nearly true. The difference is only that a screen may
    also have been pushed with params, so a parameter whose annotation is *not* a
    store is looked up there instead.

    The two cannot collide: a name is either annotated with a `@store` class or it
    is not, and that decides which source it comes from. A missing param with a
    default is left to the default; a missing one without is an error naming the
    screen, since the alternative is a `TypeError` from deep inside a call the user
    did not write.
    """
    bound: dict[str, Any] = {}
    sig = _store_mod.signature_with_resolved_annotations(fn)
    for name, param in sig.parameters.items():
        ann = param.annotation
        if ann is not inspect.Parameter.empty and _store_mod.is_store(ann):
            bound[name] = _store_mod.get_instance(ann)
            continue
        if name in params:
            bound[name] = params[name]
            continue
        if param.default is not inspect.Parameter.empty:
            continue
        raise TypeError(
            f"screen {fn.__name__}() needs a value for '{name}': annotate it with a "
            f"@store class to have it injected, or push it — "
            f"nav.push({fn.__name__}, {name}=...)"
        )
    return bound


__all__ = ["Entry", "Nav", "NavigationHost", "Navigator", "Screen", "screen_name"]
