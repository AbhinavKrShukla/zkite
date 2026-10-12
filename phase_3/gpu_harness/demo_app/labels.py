"""§32 — labelled `Checkbox`/`Radio`, and the decided `tap`-before-`change` order.

Written against the documented signatures only — `Checkbox(cls, label, checked=,
on_change=)` as WIDGETS.md states it — and nothing else, per §26.7's rule. No private
modules, no reaching into the framework to observe itself.

Every fact is readable **off a screenshot**, because each is about state rather than
timing. The one exception is case 5, which is about *order*, and that is read off the
log — the two dispatch lines `render_thread.rs` already prints.

1. **Tapping the LABEL toggles the control.** This is §30.8's actual complaint: in
   `[x] Wi-Fi`, the live area was a 20x20dp box and tapping the text did nothing. The
   tap coordinate for this case is deliberately far to the right of the box.

2. **Tapping the BOX toggles it too, exactly once.** The box is presentational now
   (`aria_hidden`), so the container's `tap` is the only dispatch — if `control_of`'s
   guard were missing, this row would also fire a `change` and the counter would move
   by two instead of one. The counter is what makes that countable.

3. **A labelled `Radio` group selects, and re-tapping the selected one does not clear
   it.** Same rule as §30's case 5, on the labelled path — where it is enforced by
   `Radio._next_checked` in Python rather than by `Control::Radio` in Rust, so it is a
   genuinely different code path and needs its own device evidence.

4. **A labelled control inside a tappable `Row` absorbs the tap.** The row's counter
   must NOT move: both runtimes stop the walk at the nearest `tap`-bound ancestor,
   which is the control. This is the structural reason §30.8's ordering trap is
   unreachable from the labelled form.

5. **The BARE control inside a tappable row still fires both handlers — now in the
   decided order.** §32.3: the owner's call is that native matches the WebView, so the
   row's bubbled `tap` runs BEFORE the control's own `change`. Both counters move; the
   log must read `dispatching tap to ...` and *then* `toggling switch ...`. Before §32
   native printed those two the other way round.

6. **`indicator=False` builds an MCQ out of the same widget.** The option is styled
   entirely by the app, there is no circle at all — and `uiautomator` must still report
   `android.widget.RadioButton` with the option text as its name. That is the whole
   reason not to build an MCQ out of tappable `Row`s (§32.5).

The `dispatch order` line renders the two writes in the order they arrived, so case 5
is checkable from a screenshot as well as from the log: both handlers append to it.

The background is dark like every other demo app; nothing here depends on shadow
visibility, so §28's light-background reasoning does not apply.
"""

from zkite import Column, Checkbox, Radio, Row, Switch, Text, store


@store
class S:
    wifi: bool = False
    bluetooth: bool = True
    choice: str = "medium"
    toggles: int = 0
    row_taps: int = 0
    bare_row_taps: int = 0
    bare_on: bool = False
    answer: str = "-"
    order: str = "-"


def _label(text: str) -> Text:
    return Text("text-xs text-slate-400", text)


def view(s: S):
    def set_wifi(v):
        s.update(wifi=v, toggles=s.toggles + 1)

    def set_bt(v):
        s.update(bluetooth=v, toggles=s.toggles + 1)

    def pick(name):
        # A labelled `Radio`'s payload is `True` whenever it is tapped, including when
        # it is already selected, so the *handler* names the choice. Same controlled
        # contract as the bare form; different implementation (`_next_checked`).
        def go(v):
            s.update(choice=name)
        return go

    # Case 5's two writers. Each appends its own name, so the rendered string is the
    # real arrival order rather than a claim about it.
    def bare_change(v):
        s.update(bare_on=v, order=(s.order if s.order != "-" else "") + "change ")

    def bare_row_tap():
        s.update(bare_row_taps=s.bare_row_taps + 1,
                 order=(s.order if s.order != "-" else "") + "tap ")

    return Column(
        "flex-1 min-h-0 bg-slate-950 p-4 gap-2",
        Text("text-base font-bold text-white", "§32 labelled controls"),

        _label("1/2  tap the TEXT or the BOX — both toggle, once each"),
        Checkbox("text-sm text-white h-[36px]", f"Wi-Fi: {'ON' if s.wifi else 'OFF'}",
                 checked=s.wifi, on_change=set_wifi),
        Checkbox("text-sm text-white h-[36px]", f"Bluetooth: {'ON' if s.bluetooth else 'OFF'}",
                 checked=s.bluetooth, on_change=set_bt),
        Text("text-sm text-amber-400", f"toggles: {s.toggles}"),

        _label("3  a labelled radio group — re-tapping the selected one must not clear it"),
        Radio("text-sm text-white h-[32px]", "Low", checked=s.choice == "low", on_change=pick("low")),
        Radio("text-sm text-white h-[32px]", "Medium", checked=s.choice == "medium", on_change=pick("medium")),
        Radio("text-sm text-white h-[32px]", "High", checked=s.choice == "high", on_change=pick("high")),
        Text("text-sm text-emerald-400", f"choice: {s.choice}"),

        _label("4  labelled, inside a tappable row — the ROW must NOT also fire"),
        Row(
            "items-center gap-3 h-[40px] bg-slate-800 rounded px-2",
            Checkbox("text-sm text-white", "inner", checked=s.wifi, on_change=set_wifi),
            Text("text-sm text-white", f"row taps: {s.row_taps}"),
            on_tap=lambda: s.update(row_taps=s.row_taps + 1),
        ),

        _label("5  BARE control in a tappable row — both fire, tap BEFORE change (§32.3)"),
        Row(
            "items-center gap-3 h-[40px] bg-slate-800 rounded px-2",
            Switch("", checked=s.bare_on, on_change=bare_change),
            Text("text-sm text-white", f"row: {s.bare_row_taps}  on: {s.bare_on}"),
            on_tap=bare_row_tap,
        ),
        _label("6  indicator=False — an MCQ, styled by the app, still a radio to a11y"),
        Row(
            "gap-2 h-[44px]",
            *[Radio(
                "px-3 py-2 rounded-lg border-2 text-sm " + (
                    "border-indigo-500 bg-indigo-950 text-white" if s.answer == k
                    else "border-slate-700 text-slate-300"),
                text, checked=s.answer == k, indicator=False,
                on_change=(lambda k=k: (lambda v: s.update(answer=k)))(),
            ) for k, text in (("a", "A. Golgi"), ("b", "B. Mitochondria"))]
        ),
        Text("text-sm text-emerald-400", f"answer: {s.answer}"),
        Text("text-sm text-cyan-400", f"dispatch order: {s.order}"),
    )
