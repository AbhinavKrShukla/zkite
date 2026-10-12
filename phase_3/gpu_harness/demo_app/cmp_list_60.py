"""§12 backend comparison — 60 rows through a plain `ScrollView`.

The smallest of §12.2 item 2's three sizes, and the only one *not* virtualised: at
60 rows a real app would not reach for `List`, and §12.2's own wording ("60 rows,
1,000 rows, and 10,000 rows, the last two through `List`") says so. It is the
control for the two `List` sizes — whatever the backends' scroll frame times are
here is what they cost before virtualisation is in the picture at all.

Row shape, height and styling are identical across `cmp_list_60`/`_1k`/`_10k` so
the only variable across the three is row count. See `cmp_counter.py` for how these
are run identically through both shells.
"""

from zkite import Column, Row, ScrollView, Text, store

ROWS = 60
ROW_HEIGHT = 56


@store
class DemoState:
    n: int = 0


def row(i: int):
    bg = "bg-slate-900" if i % 2 == 0 else "bg-slate-950"
    return Row(
        f"items-center justify-between px-4 shrink-0 h-[{ROW_HEIGHT}px] {bg}",
        Text("text-white", f"Item {i}"),
        Text("text-slate-500 text-sm", f"#{i}"),
    )


def view(s: DemoState):
    return Column(
        "flex-1 bg-slate-950 min-h-0",
        Column(
            "shrink-0 items-center justify-center gap-2 py-4",
            Text("text-2xl font-bold text-white tabular-nums", f"{ROWS} rows (plain) · {s.n}"),
        ),
        ScrollView("flex-1", *[row(i) for i in range(ROWS)]),
    )
