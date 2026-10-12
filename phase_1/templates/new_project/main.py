from zkite import Column, Text, Button, SystemBars, store


@store
class Counter:
    n: int = 0


def view(c: Counter):
    return SystemBars(Column("flex-1 items-center justify-center gap-6 bg-slate-950",
        Text("text-7xl font-bold text-white tabular-nums", f"{c.n}"),
        Button("px-6 py-3 rounded-2xl bg-indigo-500 active:bg-indigo-600 text-white font-medium",
               "Increment", on_tap=lambda: c.update(n=c.n + 1)),
    ), status_color="#020617")
