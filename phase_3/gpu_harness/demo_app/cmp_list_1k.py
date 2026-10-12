"""§12 backend comparison — 1,000 rows through the virtualised `List`.

The middle of §12.2 item 2's three sizes. Row shape, height and styling are
identical to `cmp_list_60.py` and `cmp_list_10k.py`; only `ROWS` and the
`ScrollView`-vs-`List` choice differ, and that choice follows §12.2's own wording.
See `cmp_counter.py` for how these run identically through both shells.
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
        Column(
            "shrink-0 items-center justify-center gap-2 py-4",
            Text("text-2xl font-bold text-white tabular-nums", f"{ROWS} rows (virtualised) · {s.n}"),
        ),
        List("flex-1", count=ROWS, item_height=ROW_HEIGHT, builder=row),
    )
