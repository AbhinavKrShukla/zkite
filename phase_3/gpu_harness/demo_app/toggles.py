"""§30 — `Checkbox` and `Radio` on the native backend.

Written against the documented signatures only — `Checkbox(cls, checked=,
on_change=)` and `Radio(cls, checked=, on_change=)`, exactly as WIDGETS.md states
them — and nothing else. §26.7 is what that rule is made of: `demo_app/main.py` wired
its `Switch` with an undocumented `on_tap`, so every device verification in a
7,000-line plan exercised an API no user would write, and divergence 6 survived three
phases because of it. No private modules, no reaching into the framework to observe
itself.

Every fact this app is meant to establish is readable **off a screenshot**, with no log
line, because the questions are about state rather than about timing:

1. **A `Checkbox` toggles both ways.** The `False` direction is the one that matters —
   §26.2's payload bug was that `bool("false")` is `True`, so a control could be turned
   on and never off. Two independent checkboxes, so "did the right one change" is
   answerable too.

2. **A `Radio` group selects, and a second tap on the selected one does not clear it.**
   That is the one behavioural difference from `Checkbox` (`Control::Radio` sends `True`
   rather than `!checked`), and it is invisible in a static render — the conformance
   harness cannot tap anything, so this row exists precisely because a pixel diff cannot
   answer it.

3. **Exclusivity is the app's own store field**, not framework magic: each `Radio` gets
   `checked=(s.choice == ...)`. If selecting one leaves two filled, the controlled
   contract is broken.

4. **The payload type is rendered on screen.** `bool` is the assertion — WIDGETS.md has
   claimed `bool` since phase 1 and it was a `str` on both backends until §26.2.

5. **A `Checkbox` inside a tappable `Row`.** Both handlers must fire, on both backends
   (§26.1: that bubbling is conformant, not a bug). The row counter going up *and* the
   box changing is the pass.

The background is dark like every other demo app; nothing here depends on shadow
visibility, so §28's light-background reasoning does not apply.
"""

from zkite import Column, Checkbox, Radio, Row, Text, store


@store
class S:
    wifi: bool = False
    bluetooth: bool = True
    choice: str = "medium"
    last_type: str = "-"
    row_taps: int = 0


def _label(text: str) -> Text:
    return Text("text-xs text-slate-400", text)


def _row(*children) -> Row:
    return Row("items-center gap-3 h-[36px]", *children)


def view(s: S):
    def set_wifi(v):
        s.update(wifi=v, last_type=type(v).__name__)

    def set_bt(v):
        s.update(bluetooth=v, last_type=type(v).__name__)

    def pick(name):
        # A `Radio`'s payload is `True` whenever it is tapped, including when it is
        # already selected — so the *handler* is what names which option was chosen.
        # That is the controlled contract: the widget reports an event, the store
        # decides the state.
        def go(v):
            s.update(choice=name, last_type=type(v).__name__)
        return go

    return Column(
        "flex-1 min-h-0 bg-slate-950 p-4 gap-2",
        Text("text-base font-bold text-white", "§30 checkbox / radio"),

        _label("1  two independent checkboxes"),
        _row(Checkbox("", checked=s.wifi, on_change=set_wifi),
             Text("text-sm text-white", f"Wi-Fi: {'ON' if s.wifi else 'OFF'}")),
        _row(Checkbox("", checked=s.bluetooth, on_change=set_bt),
             Text("text-sm text-white", f"Bluetooth: {'ON' if s.bluetooth else 'OFF'}")),

        _label("2  a radio group — one of three, and re-tapping must not clear it"),
        _row(Radio("", checked=s.choice == "low", on_change=pick("low")),
             Text("text-sm text-white", "Low")),
        _row(Radio("", checked=s.choice == "medium", on_change=pick("medium")),
             Text("text-sm text-white", "Medium")),
        _row(Radio("", checked=s.choice == "high", on_change=pick("high")),
             Text("text-sm text-white", "High")),
        Text("text-sm text-emerald-400", f"choice: {s.choice}"),

        _label("3  a checkbox inside a tappable row — both handlers must fire"),
        Row(
            "items-center gap-3 h-[36px] bg-slate-800 rounded px-2",
            Checkbox("", checked=s.wifi, on_change=set_wifi),
            Text("text-sm text-white", f"row taps: {s.row_taps}"),
            on_tap=lambda: s.update(row_taps=s.row_taps + 1),
        ),

        Text("text-sm text-amber-400", f"last payload type: {s.last_type}"),
    )
