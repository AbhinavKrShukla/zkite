"""The control for `list_virtualised.py`: the same 10,000 rows, same styling, same
row height, through a plain non-virtualised `ScrollView` (../../PLAN.md §11).

Every row is a real, permanently-live node here — built by `view()`, serialised
into the tree JSON, laid out by taffy, and text-shaped by parley on every single
frame. That is exactly the wall §5 item 3 describes, and running this on the real
device is how the "before" number in §11's table was measured rather than guessed.

`ROW_HEIGHT` is applied as an explicit `h-[56px]` rather than left to the row's
natural content height, so the two apps lay out identically and the comparison is
about windowing alone.
"""

from zkite import Column, Row, ScrollView, Text, store

ROWS = 10_000
ROW_HEIGHT = 56


@store
class DemoState:
    n: int = 0


def view(s: DemoState):
    rows = []
    for i in range(ROWS):
        bg = "bg-slate-900" if i % 2 == 0 else "bg-slate-950"
        rows.append(
            Row(
                f"shrink-0 items-center justify-between px-4 h-[{ROW_HEIGHT}px] {bg}",
                Text("text-white", f"Item {i}"),
                Text("text-slate-500 text-sm", f"#{i}"),
            )
        )

    return Column(
        "flex-1 bg-slate-950 min-h-0",
        Column(
            "shrink-0 items-center justify-center gap-2 py-4",
            Text("text-2xl font-bold text-white tabular-nums", f"{ROWS} rows (plain) · {s.n}"),
        ),
        ScrollView("flex-1", *rows),
    )
