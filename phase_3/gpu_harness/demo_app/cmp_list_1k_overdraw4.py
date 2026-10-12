"""§46 arm H — `cmp_list_1k.py` with **five** stacked opaque row backgrounds.

The heavy end of the three-arm overdraw curve; see `cmp_list_1k_overdraw0.py` for the
full design. Each row paints its own background and then four full-bleed opaque
children directly on top of it, so the same pixels are covered five times over and the
final image is *identical* to arm A's — which is the point. Same rows, same glyph
runs, same scroll mechanics, same Rust.

The four extra layers are `absolute inset-0`-shaped by construction rather than by a
class `twcore` does not have: each is a `w-full h-full` sibling inside a `Stack`, which
overlays its children in one grid cell (WIDGETS.md, `Stack`). `Stack` was chosen over
four nested `Column`s because nesting would also deepen the taffy tree and change
`layout`, confounding the one variable this arm exists to move.
"""

from zkite import Column, List, Row, Stack, Text, store

ROWS = 1_000
ROW_HEIGHT = 56

# Five opaque layers of the *same* colour a row would have had, so the rendered image
# is indistinguishable from arm A's and only the paint work differs.
LAYERS = 5


def _bg(i: int) -> str:
    return "bg-slate-900" if i % 2 == 0 else "bg-slate-950"


@store
class DemoState:
    n: int = 0


def row(i: int):
    bg = _bg(i)
    return Stack(
        "h-full",
        *[Column(f"w-full h-full {bg}") for _ in range(LAYERS)],
        Row(
            "w-full h-full items-center justify-between px-4",
            Text("text-white", f"Item {i}"),
            Text("text-slate-500 text-sm", f"#{i}"),
        ),
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
