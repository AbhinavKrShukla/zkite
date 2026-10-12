"""10,000 rows through the virtualised `List` widget (../../PLAN.md §11).

Paired with `list_plain.py`, which is this file with `List` swapped for a plain
`ScrollView` over the same 10,000 rows and nothing else changed — the two exist to
be measured against each other on the same physical device, since "virtualisation
makes lists fast" is the sort of claim this project's own conventions say to
demonstrate rather than assert.

Selected at APK build time with `ZKITE_APP_MAIN=.../list_virtualised.py
scripts/build_apk.sh`. Remember `adb shell pm clear dev.zkite.gpuharness` before
reinstalling — `ZKiteActivity`'s asset unpacking is stamp-guarded, so a reinstall
with a different app.zip otherwise silently keeps running the old app
(../../PLAN.md §7.9).
"""

from zkite import Column, List, Row, Text, store

ROWS = 10_000
ROW_HEIGHT = 56


@store
class DemoState:
    n: int = 0


def view(s: DemoState):
    def row(i: int):
        bg = "bg-slate-900" if i % 2 == 0 else "bg-slate-950"
        return Row(
            f"items-center justify-between px-4 h-full {bg}",
            Text("text-white", f"Item {i}"),
            Text("text-slate-500 text-sm", f"#{i}"),
        )

    return Column(
        "flex-1 bg-slate-950 min-h-0",
        Column(
            "shrink-0 items-center justify-center gap-2 py-4",
            Text("text-2xl font-bold text-white tabular-nums", f"{ROWS} rows (virtualised) · {s.n}"),
        ),
        List("flex-1", count=ROWS, item_height=ROW_HEIGHT, builder=row),
    )
