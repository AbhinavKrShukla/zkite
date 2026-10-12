"""§26 — `Switch` and `Slider` through the API WIDGETS.md actually documents.

This exists because `demo_app/main.py` does **not** exercise these widgets the way a
user writes them, which is why divergence 6 (../../PLAN.md §15.1) survived every
device run in this project. `main.py:58` wires its `Switch` with **`on_tap`** — an
undocumented prop that happens to land on `dispatch_tap`'s one working path — and its
`Slider` has no handler at all. So the harness has always shown a switch that toggles
and a slider that never had to move.

Every control here uses the documented signature and nothing else:

    Switch(cls, checked=, on_change=)
    Slider(cls, min=, max=, step=, value=, on_change=)

Four things are on screen, each chosen so a single screenshot answers a question that
would otherwise need a log line:

1. **A bare `Switch`.** State is rendered as text *and* as the background colour of
   its own row, so "did it toggle" is a histogram lookup rather than an OCR problem —
   the same trick `nav.py` uses for which screen is on top.
2. **A `Switch` inside a row that itself has `on_tap`.** This is the ancestor-walk
   case: `dispatch_tap` walks *past* a node it does not match, so before the fix a tap
   on this switch fires the row's handler instead of the switch's. The row's own tap
   count is rendered, so "the wrong handler fired" is visible rather than inferred.
3. **A `Slider` with `on_change`**, value rendered as a number.
4. **A `Slider` inside a `ScrollView` of 40 rows**, which is §6.2's disambiguation
   case: dragging the slider must move the slider and must *not* scroll the list, in
   either direction. The list's own scroll position is observable from the row numbers
   on screen, so a drag that leaked into the scroll container is visible too.

The payload types are rendered as well (`bool`/`float` vs `str`), because §26's other
half is that the `change` payload arrived as a string on *both* backends and
`bool("false")` is `True` — a `Switch` that reports `str` here is the bug even if the
value looks right.
"""

from zkite import Column, Row, ScrollView, Slider, Switch, Text, store


@store
class Form:
    notifications: bool = False
    wifi: bool = False
    volume: float = 40.0
    brightness: float = 70.0
    row_taps: int = 0
    last_payload_type: str = "-"


def _type_of(v) -> str:
    return type(v).__name__


def view(s: Form):
    return Column(
        "flex-1 min-h-0 bg-slate-950 gap-2 p-3",

        # 1. a bare Switch, documented API. The row's background *is* the state.
        Row(
            "shrink-0 items-center justify-between px-4 py-3 rounded-xl "
            + ("bg-emerald-900" if s.notifications else "bg-slate-800"),
            Text("text-white", f"Notifications: {'ON' if s.notifications else 'OFF'}"),
            Switch(
                checked=s.notifications,
                on_change=lambda v: s.update(notifications=v, last_payload_type=_type_of(v)),
            ),
        ),

        # 2. the same thing inside a row that also handles `tap` — the ancestor-walk
        #    case. Tapping the switch must not increment `row_taps`.
        Row(
            "shrink-0 items-center justify-between px-4 py-3 rounded-xl "
            + ("bg-indigo-900" if s.wifi else "bg-slate-800"),
            Text("text-white", f"Wi-Fi: {'ON' if s.wifi else 'OFF'}  rowtaps={s.row_taps}"),
            Switch(
                checked=s.wifi,
                on_change=lambda v: s.update(wifi=v, last_payload_type=_type_of(v)),
            ),
            on_tap=lambda: s.update(row_taps=s.row_taps + 1),
        ),

        # 3. a Slider with a handler, value rendered.
        Column(
            "shrink-0 gap-2 px-4 py-3 rounded-xl bg-slate-800",
            Text("text-white text-sm tabular-nums", f"Volume: {s.volume:.0f}"),
            Slider(
                "",
                min=0, max=100, step=1, value=s.volume,
                on_change=lambda v: s.update(volume=v, last_payload_type=_type_of(v)),
            ),
        ),

        Text("text-amber-400 text-xs shrink-0 px-4", f"last payload type: {s.last_payload_type}"),

        # 4. a Slider *inside* a scrollable list — §6.2 disambiguation. A drag that
        #    starts on this slider must move it and must not scroll the rows, and the
        #    row numbers make a leak visible.
        ScrollView(
            "flex-1 min-h-0 rounded-xl bg-slate-900",
            Column(
                "shrink-0 gap-2 px-4 py-3",
                Text("text-white text-sm tabular-nums", f"Brightness: {s.brightness:.0f}"),
                Slider(
                    "",
                    min=0, max=100, step=1, value=s.brightness,
                    on_change=lambda v: s.update(brightness=v, last_payload_type=_type_of(v)),
                ),
            ),
            *[
                Row("shrink-0 px-4 py-2", Text("text-slate-300 text-sm", f"Row {i}"))
                for i in range(40)
            ],
        ),
    )
