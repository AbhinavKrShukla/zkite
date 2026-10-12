"""§12 backend comparison — 10,000 rows through the virtualised `List`.

The largest of §12.2 item 2's three sizes, and the one with a known native number
to check this whole comparison against: §11.9 measured this shape at 23.8ms per
scroll frame (~42fps) on the same device, so a native result far from that would
mean the measurement is wrong before it means the backend is.

Deliberately *not* the same file as `demo_app/list_virtualised.py`, which §11 owns
and which should keep reporting §11's own numbers unchanged. Row shape, height and
styling here match `cmp_list_60.py`/`cmp_list_1k.py` so row count is the only
variable across the three. See `cmp_counter.py` for how these run identically
through both shells.
"""

from zkite import Column, List, Row, Text, store

ROWS = 10_000
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
