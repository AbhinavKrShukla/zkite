"""Runtime-reported scroll state for virtualised widgets (`List`).

The one piece of state that has to flow *backwards*, from the renderer into the
widget tree. Everything else in `zkite_py` is a pure function of `@store` state:
`view()` runs, produces a tree, the backend paints it, and where that tree is
scrolled to is entirely the backend's business (phase_3/PLAN.md §6.1 — the native
renderer keeps scroll offsets in `render_thread::State::scroll_offsets`, and the
WebView backend keeps them in the DOM). A virtualised list breaks that one-way
flow by construction: it can't decide *which* rows to build without knowing where
its own viewport currently is.

So the backends report it here, keyed by the same `zkite_py` node id everything
else is keyed by, and `List` reads it during its own tree walk. Deliberately a
plain module-level dict rather than something threaded through `RenderContext`:
a `RenderContext` is recreated on every single render (see `app.App.render_tree`),
and scroll position must survive that, the same way it survives a tree rebuild on
the native side.

`set_viewport` returns whether the value actually changed, so a backend can push
on every scroll frame without that implying a re-render on every scroll frame —
see `phase_3/pybridge`'s `scroll()` and `runtime.py`'s `_zkite_scroll_tree`, which
re-run `view()` only when the rows already on screen stop covering what the
viewport can see, not merely when a pixel of scroll did.

The two questions this module answers are deliberately different functions, and
collapsing them is the bug phase_3/PLAN.md §40 fixed: `compute_window` decides
*what to build* (visible rows plus `overscan` either side), while
`window_would_change` decides *whether to build again* (has the viewport reached
the edge of what is built?). Asking the second question by re-running the first
and comparing for equality makes `overscan` pure cost — see
`window_would_change`.
"""

from __future__ import annotations

# Deliberately imports nothing at all — not even `math`.
#
# The first version of this module used `math.ceil`/`math.floor`, which every host
# test was perfectly happy with and which failed on the first real device launch:
# `ModuleNotFoundError: No module named 'math'`. `math` is an *extension* module, so
# it ships in `lib/<abi>/` rather than in `python-stdlib.zip` (phase_0's Android-10
# dlopen finding), reachable only through the `.pth` file `ZKiteActivity.
# linkExtensionModules` writes — and in the `gpu_harness` build that path does not
# currently work (phase_3/PLAN.md §11.6). `zkite` is the framework every app imports
# before any of its own code runs, so it is the worst possible place to depend on
# something that fragile, and floor division does the same job in the same number of
# operations. See §11.6 for the underlying gap, which is real and still open.

# node_id -> (scroll_y, viewport_height), both in device pixels.
_viewports: dict[str, tuple[float, float]] = {}

# node_id -> (count, item_height, overscan, first, last) for every virtualised
# container that has actually rendered. Recorded by `List` as it materialises a
# window (not merely as it computes one), so `window_would_change` compares
# against what is genuinely on screen right now.
_windows: dict[str, tuple[int, float, int, int, int]] = {}

#: Used only until a backend reports a list's real viewport height (see
#: `compute_window`). Chosen to comfortably exceed a tall phone screen so the
#: bootstrap window is a superset of the real first window rather than a subset —
#: an over-large first window costs one render of extra rows, an under-large one
#: would show a visibly short list for a frame.
BOOTSTRAP_VIEWPORT_PX = 1400.0


def compute_window(
    scroll_y: float, viewport_h: float, count: int, item_height: float, overscan: int
) -> tuple[int, int]:
    """The half-open item range `[first, last)` a virtualised container at this
    scroll position should materialise.

    Lives here rather than on `List` so the scroll glue (`runtime.py`'s
    `_zkite_scroll_tree`) can answer "would this scroll change anything?" without
    constructing widgets — i.e. without running the app's `view()` or any row
    `builder` on every scroll frame. Running user callbacks 60 times a second to
    discover that nothing changed would be a real footgun, not just wasted work.

    `first` is clamped into range *before* `last` is derived from it, so the
    window keeps its full size when clamping moves it. Clamping `last`
    independently instead lets a negative `scroll_y` (which a rubber-band
    overscroll would produce — phase_3/PLAN.md §5 item 4) collapse the window to
    nothing and render a blank list at the top of every list. Found by a host
    test, not on a device.
    """
    if viewport_h <= 0:
        # Either nothing has been laid out yet, or this container really is a
        # zero-height box (collapsed by a flex parent, or off-screen). Dividing
        # by it would produce a single overscan-sized window at whatever scroll
        # was last reported — plausible-looking and useless.
        viewport_h = BOOTSTRAP_VIEWPORT_PX

    # +1 because a viewport rarely lands on an exact row boundary, so one extra
    # row is partially visible at the bottom. `-(-a // b)` is `ceil(a / b)` for
    # positive values, and `a // b` is `floor(a / b)` including for negative `a`
    # (Python floors rather than truncating) — see this module's own note on why
    # neither is spelled with `math`.
    visible_rows = int(-(-viewport_h // item_height)) + 1
    window_rows = visible_rows + 2 * overscan

    first = int(scroll_y // item_height) - overscan
    first = max(0, min(first, count))
    last = min(count, first + window_rows)
    return first, last


def visible_range(scroll_y: float, viewport_h: float, count: int, item_height: float) -> tuple[int, int]:
    """The half-open item range `[v0, v1)` the viewport can actually *see* at this
    scroll position — `compute_window` without the overscan.

    Separate from `compute_window` because the two answer different questions, and
    conflating them is what made `overscan` cost work without buying any (see
    `window_would_change`). This one is "what must be on screen"; `compute_window`
    is "what to build so that stays true for a while".
    """
    if viewport_h <= 0:
        viewport_h = BOOTSTRAP_VIEWPORT_PX
    visible_rows = int(-(-viewport_h // item_height)) + 1
    v0 = int(scroll_y // item_height)
    v0 = max(0, min(v0, count))
    v1 = min(count, v0 + visible_rows)
    return v0, v1


def note_window(node_id: str, count: int, item_height: float, overscan: int, first: int, last: int) -> None:
    """Record what a virtualised container just materialised, and the parameters
    needed to recompute it. Called as rows are built, not as a window is merely
    computed — see `_windows`.
    """
    _windows[str(node_id)] = (count, item_height, overscan, first, last)


def window_would_change(node_id: str) -> bool:
    """Whether this container's materialised window no longer covers what the
    viewport can see — i.e. whether this scroll actually needs a re-render.

    The actual question behind "does this scroll need a re-render?", and it is a
    question about *coverage*, not about equality. `False` for any node that isn't
    a virtualised container at all (a plain `ScrollView` scrolls without Python
    ever hearing about it, exactly as before this existed).

    **This used to ask whether `compute_window` would return a different range,
    and that made `overscan` a pure cost.** `compute_window`'s `first` is
    `scroll_y // item_height - overscan`, so it moves on every row boundary
    crossed *no matter what `overscan` is* — the offset shifts the window, it does
    not stall it. Measured over 2000dp of mid-list scroll (889dp viewport, 56dp
    rows): 36 rebuilds at `overscan=0` and 36 at `overscan=32`, with the rows
    built per rebuild growing 17 → 81. So the documented mitigation for the
    phase_3/PLAN.md §39.5 `sync` spike — "a larger overscan, fewer costlier
    rebuilds" — bought the *same* number of spikes, each one bigger.

    Asking about coverage instead is what makes those overscan rows do the job
    they were named for: the window already on screen is kept until the viewport
    reaches its edge, so a rebuild happens once per `overscan + 1` rows rather
    than once per row, and `overscan` becomes a real frequency knob.

    A window `compute_window` just produced always covers its own visible range
    (`first <= v0` and `v1 <= last` hold under every clamping branch), so this can
    never return `True` for a freshly built window — without that, a scroll would
    re-render on every single frame. `test_a_freshly_built_window_never_asks_to_be
    _rebuilt` pins it.
    """
    recorded = _windows.get(str(node_id))
    if recorded is None:
        return False
    # `overscan` is recorded for provenance — which parameters produced this
    # window — but deliberately not read here: whether to rebuild is now decided
    # by what is on screen, not by what a recomputation would have chosen.
    count, item_height, _overscan, first, last = recorded
    scroll_y, viewport_h = _viewports.get(str(node_id), (0.0, BOOTSTRAP_VIEWPORT_PX))
    v0, v1 = visible_range(scroll_y, viewport_h, count, item_height)
    if v0 >= v1:
        # Nothing visible to cover: an empty list, or scrolled past the end (which
        # `List._window` documents as unreachable from either renderer, both of
        # which clamp to `max_scroll_y`). Keeping the last good window beats
        # rebuilding to the empty one `compute_window` would return.
        return False
    return not (first <= v0 and v1 <= last)


def set_viewport(node_id: str, scroll_y: float, viewport_h: float) -> bool:
    """Record a scroll container's current position and visible height.

    Returns whether this differs from what was already recorded — a cheap
    pre-filter for callers, *not* the question that decides a re-render (a
    one-pixel scroll changes this and usually changes no materialised window;
    see `List.window_signature`).
    """
    key = str(node_id)
    new = (float(scroll_y), float(viewport_h))
    old = _viewports.get(key)
    if old == new:
        return False
    _viewports[key] = new
    return True


def get_viewport(node_id: str) -> tuple[float, float] | None:
    """This container's `(scroll_y, viewport_height)`, or `None` if no backend has
    reported one yet — which is the normal state on the very first render, before
    anything has been laid out and therefore before any viewport height exists to
    report. `List` handles that case explicitly rather than treating it as zero.
    """
    return _viewports.get(str(node_id))


def clear_viewports() -> None:
    """Drop every recorded viewport. For tests, and for a backend that needs to
    reset between apps.

    Deliberately *not* called on hot reload, even though a reload can change the
    tree enough that a node id no longer means the same widget. Scroll position
    surviving a reload is an explicit phase-1 promise (plan1.md §6: "keyboard
    state and scroll position intact"), and the position itself lives in the
    backend, not here — clearing this would leave the backend still scrolled to
    5000px while `List` rebuilt the window at the top, i.e. a blank screen until
    the next scroll report. Stale entries for ids that no longer exist are inert
    and get overwritten the next time the same id is reported.
    """
    _viewports.clear()
    _windows.clear()


__all__ = [
    "BOOTSTRAP_VIEWPORT_PX",
    "compute_window",
    "visible_range",
    "note_window",
    "window_would_change",
    "set_viewport",
    "get_viewport",
    "clear_viewports",
]
