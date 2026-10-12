"""§46 arm O — `cmp_list_1k.py` under one full-screen opacity group.

§45.8 left the animation driver's one unpriced cost: *"An opacity layer is an offscreen
target per group. Nothing measures what `push_opacity_layer` costs on this GPU, and a
fade over a large subtree is exactly the case an animation will produce every frame."*

This is that case, at its worst: the whole scrolling `List` — every materialised row,
every glyph run, the full 1080x2223 surface — inside a single `opacity-75` group, so
the painter opens one offscreen target spanning the entire node list on **every frame**
of a drag. A `Sheet`/`Dialog`/`Toast` transition fading a full-screen scrim plus panel
is this shape.

Against arm A (`cmp_list_1k.py`), which is byte-identical but for the `opacity-75`
wrapper, so the difference is one layer and nothing else. The Rust is identical to
every other §46 arm.
"""

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
        # The one difference from arm A. `flex-1 min-h-0` so wrapping the list in a
        # group does not change its layout — the scroll extent, the viewport height and
        # therefore the rebuild rate all have to stay comparable, or this measures two
        # things at once.
        Column(
            "flex-1 min-h-0 opacity-75",
            Column(
                "shrink-0 items-center justify-center gap-2 py-4",
                Text("text-2xl font-bold text-white tabular-nums", f"{ROWS} rows (virtualised) · {s.n}"),
            ),
            List("flex-1", count=ROWS, item_height=ROW_HEIGHT, builder=row),
        ),
    )
