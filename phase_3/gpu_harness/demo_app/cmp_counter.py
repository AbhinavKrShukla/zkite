"""§12 backend comparison — the minimal app, for cold start and tap latency.

One of the five `cmp_*.py` apps phase_3/PLAN.md §12.2 calls for. Every one of them
is deliberately **plain `zkite_py` with no backend-specific anything**, and is run
byte-for-byte identically through both shells:

  native  — ZKITE_APP_MAIN=.../cmp_counter.py phase_3/gpu_harness/scripts/build_apk.sh
  WebView — copied to a `zkite new` project's main.py, then `zkite build apk`

That "same file, both shells" property is the whole point (§12.1: *"Both backends
must run the same main.py, on the same device, back to back"*) — a comparison
between two apps that differ at all measures the difference between the apps.

This one is kept as small as a ZKite app can be, on purpose. §12.2 item 1 (cold
start) and item 3 (tap → visible change) both want the backend's own overhead, not
an app's; anything else on screen would be paid by both backends and would only dilute
the thing being measured. Item 4 (memory) reads this as the floor — the smallest
resident footprint each backend can have while running a real app.
"""

from zkite import Button, Column, Text, store


@store
class Counter:
    n: int = 0


def view(c: Counter):
    return Column(
        "flex-1 items-center justify-center gap-6 bg-slate-950",
        Text("text-7xl font-bold text-white tabular-nums", f"{c.n}"),
        Button(
            "px-6 py-3 rounded-2xl bg-indigo-500 active:bg-indigo-600 text-white font-medium",
            "Increment",
            on_tap=lambda: c.update(n=c.n + 1),
        ),
    )
