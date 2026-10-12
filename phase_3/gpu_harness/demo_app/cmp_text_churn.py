"""§12 backend comparison — ~200 text nodes whose content all changes at once.

§12.2 item 5. This is the measurement aimed squarely at the native backend's known
weak spot: `Renderer::layout` shapes text through parley, and §11.9's shaped-text
cache only helps text that *hasn't* changed (98.8% hit rate while scrolling, because
scrolling reuses rows). Here every string is different on every render, so the cache
is guaranteed to miss on all 200 — this is the shape that prices what shaping
actually costs, against Blink's own text handling doing the same job.

Deliberately a worst case, not a typical app. Reading it as "what a ZKite app costs"
would be wrong in both directions; it is here to bound one specific axis.

**Driven by taps, not by a timer.** Neither backend has a per-frame Python tick, so
"changes every frame" is produced the only way either backend can produce it: one
tap re-renders every node, and a burst of taps sustains it. That is also the honest
shape — it is a re-render benchmark, and both backends re-render on exactly the same
trigger, through the same CPython.

Every label derives from `s.n`, so all 200 strings change on every increment and
none of them can be served from any cache on either side.
"""

from zkite import Button, Column, Grid, Text, store

CELLS = 200


@store
class Churn:
    n: int = 0


def view(s: Churn):
    # Distinct per cell *and* per tap: `i` alone would let a cache serve a repeat, and
    # a shared value would let one shaped run be reused across all 200. The width is
    # held constant (`:04d`) so a changing string never changes the layout, which
    # would make this measure reflow rather than text.
    cells = [
        Text("text-slate-200 text-sm tabular-nums", f"{i:03d}:{(s.n * 7 + i * 13) % 1000:04d}")
        for i in range(CELLS)
    ]
    return Column(
        "flex-1 bg-slate-950 min-h-0 gap-2 p-2",
        Column(
            "shrink-0 items-center justify-center gap-2",
            Text("text-xl font-bold text-white tabular-nums", f"{CELLS} texts · render {s.n}"),
            Button(
                "px-6 py-3 rounded-2xl bg-indigo-500 active:bg-indigo-600 text-white font-medium",
                "Re-render all",
                on_tap=lambda: s.update(n=s.n + 1),
            ),
        ),
        Grid("grid-cols-4 gap-1 flex-1 min-h-0", *cells),
    )
