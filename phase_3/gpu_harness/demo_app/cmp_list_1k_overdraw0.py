"""§46 arm L — `cmp_list_1k.py` with **zero** row backgrounds.

One of three arms that vary *only* overdraw — the number of opaque rects painted
over the same pixels — while holding the scroll mechanics, the row count, the row
height, the header, the `List` configuration and every glyph run identical. The Rust
is identical too, so all three carry the same provenance digest and the only variable
is the app (§42.4's own method).

  arm L (this file)  0 row backgrounds  — the root's `bg-slate-950` is all that covers
  arm A (cmp_list_1k) 1 row background
  arm H (…overdraw4)  5 row backgrounds — the same pixels painted five times over

§39.5 named overdraw as the mechanism behind `present`, and §44.7 left it as the last
large item on Queue H. This is the discriminating control for that claim: if painting
the same pixels five times costs what painting them once costs, there is nothing for
occlusion culling to remove.
"""

from zkite import Column, List, Row, Text, store

ROWS = 1_000
ROW_HEIGHT = 56


@store
class DemoState:
    n: int = 0


def row(i: int):
    return Row(
        "items-center justify-between px-4 h-full",
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
