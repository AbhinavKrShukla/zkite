"""§33 — `Select`, and the `Overlay` layer it is built on.

Written against the documented signatures only — `Select(cls, options=, value=,
on_change=)` and the class-string props as WIDGETS.md states them — per §26.7's rule.
No private modules, and nothing reaching into the framework to observe itself.

The claim this fixture exists to test is the one no host test can reach: **a layer
emitted last in document order paints above the app and is tapped first**, on the
native backend, on real silicon. Everything else here is a consequence of that.

1. **The popup paints above the app.** Tapping the trigger must dim the whole screen
   and draw a panel over content that was previously visible. The row of colour blocks
   near the top is there to be *covered*: it is opaque, it sits above the trigger in
   document order, and before §31.1's paint-order fix nothing opaque could cover
   anything textual — so this is also the fixture that would catch a regression of it.

2. **Choosing an option applies it and closes.** The `chosen` line is the app's own
   `@store` field, so it proves the value reached `on_change` — and it is the *option's*
   value (`ke`), not the empty string every backend sends as a tap payload.

3. **A tap on the panel does nothing and the popup stays open.** The panel registers no
   handler at all; the scrim beneath it owns the dismiss. If the dismiss lived on the
   layer root instead, this tap would close the popup.

4. **A tap outside the panel closes it, and changes nothing.** `chosen` must be
   unchanged after it.

5. **Back closes the popup rather than exiting the app.** The overlay registers on the
   same back chain `Nav` uses (§16.1's layer 1), innermost first.

6. **`placement` moves the panel.** The second picker is a bottom sheet from the same
   widget — the centred default is a default *value*, not a baked-in geometry. Readable
   off a screenshot: the panel is flush to the bottom edge.

7. **An app-drawn option row keeps its semantics.** The third picker draws its own rows
   with `option=`; `uiautomator` must still report options with the row text as their
   name and `selected="true"` on the current one. Same argument as §32.5's
   `indicator=False`, one layer out.

8. **A `TextField` inside the popup takes focus and types.** The header slot is what a
   search-as-you-type picker needs, and a text field inside a layer is the one thing
   here that touches the IME path. Read off the screenshot (the field shows what was
   typed) rather than off a counter.

The counter lines are what make "exactly once" countable, the same way §32's `toggles`
line did.
"""

from zkite import Column, Row, ScrollView, Select, Text, TextField, store


@store
class S:
    chosen: str = "in"
    sheet: str = ""
    drawn: str = "b"
    query: str = ""
    changes: int = 0
    panel_taps: int = 0


COUNTRIES = [("br", "Brazil"), ("in", "India"), ("ke", "Kenya"), ("no", "Norway")]
SIZES = [("s", "Small"), ("m", "Medium"), ("l", "Large")]
ANSWERS = [("a", "A. Golgi"), ("b", "B. Mitochondria"), ("c", "C. Ribosome")]


def _pick(s: S, value: str) -> None:
    s.update(chosen=value, changes=s.changes + 1)


def view(s: S):
    return ScrollView(
        "flex-1 bg-slate-950 p-4 gap-3",
        # Case 1's target: opaque blocks the popup has to cover.
        Row(
            "gap-2 h-[40px] shrink-0",
            Row("flex-1 bg-rose-500 rounded", ""),
            Row("flex-1 bg-emerald-500 rounded", ""),
            Row("flex-1 bg-amber-400 rounded", ""),
        ),
        Text("text-white text-sm shrink-0", f"chosen: {s.chosen}"),
        Text("text-white text-sm shrink-0", f"changes: {s.changes}"),
        Text("text-white text-sm shrink-0", f"panel taps: {s.panel_taps}"),
        Text("text-white text-sm shrink-0", f"query: {s.query or '-'}"),

        # 1-5: the default, centred picker.
        Text("text-slate-400 text-xs shrink-0", "country"),
        Select(
            "px-4 py-3 rounded-xl bg-slate-800 text-white text-sm shrink-0",
            options=COUNTRIES,
            value=s.chosen,
            on_change=lambda v: _pick(s, v),
        ),

        # 6: same widget, bottom-sheet placement.
        Text("text-slate-400 text-xs shrink-0", "size (bottom sheet)"),
        Select(
            "px-4 py-3 rounded-xl bg-slate-800 text-white text-sm shrink-0",
            options=SIZES,
            value=s.sheet,
            placeholder="Choose a size",
            placement="items-end",
            panel="w-full rounded-t-3xl bg-slate-800 p-3 flex-col",
            on_change=lambda v: s.update(sheet=v),
        ),

        # 7: rows drawn entirely by the app.
        Text("text-slate-400 text-xs shrink-0", "answer (app-drawn rows)"),
        Select(
            "px-4 py-3 rounded-xl bg-slate-800 text-white text-sm shrink-0",
            options=ANSWERS,
            value=s.drawn,
            option_class="",
            option=lambda value, label, selected: Column(
                "px-4 py-3 my-1 rounded-xl border-2 "
                + ("border-indigo-500 bg-indigo-950" if selected else "border-slate-700"),
                Text("text-white text-sm", label),
            ),
            on_change=lambda v: s.update(drawn=v),
        ),

        # 8: a text field inside the layer.
        Text("text-slate-400 text-xs shrink-0", "searchable"),
        Select(
            "px-4 py-3 rounded-xl bg-slate-800 text-white text-sm shrink-0",
            options=[c for c in COUNTRIES if s.query.lower() in c[1].lower()],
            value=s.chosen,
            header=TextField(
                "w-full px-3 py-2 mb-2 rounded-lg bg-slate-900 text-white text-sm",
                value=s.query,
                placeholder="Search",
                on_change=lambda q: s.update(query=q),
            ),
            on_change=lambda v: _pick(s, v),
        ),

        # 3: a tappable row *behind* the popup. Its counter must not move when the
        # panel is tapped, which is the same "the walk stops at the nearest bound
        # ancestor" property §32.3 relies on — here proving the opposite direction:
        # nothing behind the layer can be reached through it.
        Row(
            "mt-2 px-4 py-3 rounded-xl bg-slate-900 shrink-0",
            Text("text-slate-300 text-sm", "tappable row behind the popup"),
            on_tap=lambda: s.update(panel_taps=s.panel_taps + 1),
        ),
    )
