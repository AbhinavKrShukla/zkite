"""The GPU harness's bundled demo app (../../PLAN.md §5) — exercises every native-
renderer feature this phase actually built, not just the counter: a scrollable list
(ScrollView + gesture disambiguation), Switch (native track/thumb synthesis), Slider
(native track/fill/thumb synthesis), and TextField (focus/value display), alongside
the original counter/Button/tap-dispatch path. Ordinary `zkite_py` — nothing here is
native-renderer-specific; the WebView backend renders this exact file identically.
"""

from zkite import Button, Column, Row, ScrollView, Slider, Switch, Text, TextField, store


@store
class DemoState:
    n: int = 0
    notifications: bool = True
    # `float`, not `int`: `Slider.on_change` delivers a float on both backends
    # (WIDGETS.md, phase_3/PLAN.md §26), matching `Number(el.value)` on the
    # WebView side. A dataclass does not enforce annotations, so an `int` here
    # would have quietly held a float and only shown up in an f-string.
    volume: float = 40.0
    name: str = ""
    password: str = ""


def view(s: DemoState):
    rows = []
    for i in range(60):
        bg = "bg-slate-900" if i % 2 == 0 else "bg-slate-950"
        rows.append(
            Row(
                f"shrink-0 items-center px-4 py-3 {bg}",
                Text("text-white", f"Item {i}"),
            )
        )

    return Column(
        # `min-h-0` here is load-bearing, not decorative: this Column is itself a
        # `flex-1` item (of the implicit full-screen root), and it contains a nested
        # `flex-1 ScrollView` — every real flexbox implementation (this one and every
        # browser) gives a flex item `min-height: auto` by default, meaning "at least
        # as big as my content," which silently defeats the inner ScrollView's own
        # `flex-1` (it just grows to fit all 60 rows instead of the space actually
        # available). `ScrollView` bakes `min-h-0` into itself
        # (`zkite/widgets.py`), but that alone isn't enough — every intermediate
        # `flex-1` ancestor between it and its nearest non-growing container needs
        # the same override, exactly as in real Tailwind/CSS. See
        # `phase_3/PLAN.md` §5 for how this was actually found (a real device, a
        # ScrollView that refused to scroll at all).
        "flex-1 bg-slate-950 min-h-0",
        Column(
            "shrink-0 items-center justify-center gap-4 py-8",
            Text("text-6xl font-bold text-white tabular-nums", f"{s.n}"),
            Button(
                "px-6 py-3 rounded-2xl bg-indigo-500 text-white font-medium",
                "Increment",
                on_tap=lambda: s.update(n=s.n + 1),
            ),
        ),
        Row(
            "shrink-0 items-center justify-between px-4 py-3 gap-4",
            Text("text-white", "Notifications"),
            # `on_change`, not `on_tap`. This demo used to say `on_tap` — an
            # undocumented prop that happened to land on `dispatch_tap`'s one working
            # path — and the `Slider` below had no handler at all. That is *why*
            # ../../PLAN.md §15.1's divergence 6 survived every device run in this
            # project: the harness exercised a switch through an API no user would
            # write, and never had to move a slider. See §26.
            Switch(
                checked=s.notifications,
                on_change=lambda v: s.update(notifications=v),
            ),
        ),
        Column(
            "shrink-0 gap-2 px-4 py-3",
            Text("text-white text-sm tabular-nums", f"Volume: {s.volume:.0f}"),
            Slider("", min=0, max=100, value=s.volume, on_change=lambda v: s.update(volume=v)),
        ),
        ScrollView("flex-1", *rows),
        # Two fields, side by side, deliberately placed *below* the scrollable
        # list rather than above it (../../PLAN.md §5/§7.5) — a bottom-pinned
        # input bar, and also the one layout shape that actually puts a focused
        # field low enough on screen to be covered by the keyboard, which is what
        # exercises IME insets/resize compensation at all. Side by side rather
        # than stacked specifically so both stay simultaneously reachable by touch
        # once one is focused and panned into view (a stacked pair would put the
        # second one right where the pan just revealed the first one, behind the
        # keyboard) — needed to exercise per-field input-type switching and the
        # cross-field selection-overlay race (§7.8's own known-gaps note) for
        # real, not just by code tracing.
        Row(
            "shrink-0 gap-2 mx-4 my-3",
            TextField(
                "flex-1 px-3 py-2 rounded-lg bg-slate-900 text-white",
                placeholder="Your name",
                value=s.name,
                on_change=lambda v: s.update(name=v),
            ),
            TextField(
                "flex-1 px-3 py-2 rounded-lg bg-slate-900 text-white",
                placeholder="Password",
                secure=True,
                value=s.password,
                on_change=lambda v: s.update(password=v),
            ),
        ),
    )
