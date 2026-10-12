"""§27 — horizontal scrolling, through the API WIDGETS.md documents.

`ScrollView(cls, *children, horizontal=True)` and nothing else. Written against the
documented signature rather than adapted from another demo app, for the reason §26.7
records: `main.py` wired its `Switch` with an undocumented `on_tap`, and that is how
divergence 6 survived every device run in a 7,000-line plan.

Four things are on screen, each chosen so a screenshot answers a question that would
otherwise need a log line. The hard part of observing this is that **horizontal scroll
position is not reported to Python** — `sync_viewports` is deliberately y-only (there
is no horizontal virtualisation), so unlike the vertical case a store field cannot
simply hold the x offset. Every horizontal readout here is therefore *geometric*: the
cards are numbered and alternately coloured, so which card sits at the strip's left
edge, and how much of it is cut, is the scroll position.

1. **A bare horizontal strip** of 20 numbered cards. Drag it left/right; fling it.
   Card 0 flush at the left edge means offset 0; the strip is at its end when card 19
   is flush right.
2. **A horizontal strip inside a vertical `ScrollView`** — the case horizontal
   scrolling exists for, and the one the two-targets-per-axis design is built around
   (`Gesture::scroll_x`/`scroll_y`). From a touch on one of its cards the x answer is
   the strip and the y answer is the list. The three questions it has to answer:
     - a *horizontal* drag on the strip moves the strip and does **not** scroll the list
     - a *vertical* drag on the strip scrolls the list and does **not** move the strip
     - a drag anywhere else in the list still scrolls the list, as it always did
   The list's own position is observable from the row numbers, which *are* reported.
3. **A `Row` with `overflow-x-auto` written as a plain class**, not via `horizontal=`.
   The gesture layer resolves scroll targets off the resolved `overflow` style, not off
   which widget class produced it, so this must scroll identically. If it ever does not,
   the front door and the underlying mechanism have drifted.
4. **A full-width strip whose content does not overflow** (three cards in a wide box).
   This is the control for the room-vs-declaration rule: it *declares* itself
   x-scrollable and has zero room, so a horizontal drag on it must fall through and do
   nothing at all rather than swallowing the gesture. Before the room filter existed,
   a plain vertical `ScrollView` behaved exactly like this and ate every horizontal drag.

There is no `@store` and no state at all, deliberately. The obvious way to make the
vertical position readable would be to render it as a number, but the only way to *get*
it is `zkite._viewport`, a private module — and a demo app reaching past the public API
to observe itself is the exact mistake §26.7 is about. Both axes are read the same way
instead: off the numbered content, from a screenshot.
"""

from zkite import Column, Row, ScrollView, Text


# Two alternating card colours so a screenshot reads as a ruler rather than as a row of
# identical boxes: at a glance, which colour is at the left edge gives the parity of the
# leading card, and the number gives the rest.
_CARD_BG = ("bg-indigo-600", "bg-emerald-600")


def _card(i: int, size: str = "w-[72px]") -> Row:
    # `shrink-0` is not decorative. A flex-row child's default `min-width: auto` saves a
    # *content*-sized child, but one with an explicit width has no content minimum to be
    # saved by, so without this every card would shrink to fit the viewport and the
    # strip would have nothing to scroll — the x-axis twin of the `min-h-0` trap, and it
    # fails the same silent way (`max_scroll` computes 0, nothing errors).
    return Row(
        f"{size} h-[64px] shrink-0 rounded-xl items-center justify-center {_CARD_BG[i % 2]}",
        Text("text-white text-xl font-bold tabular-nums", str(i)),
    )


def _strip(count: int, size: str = "w-[72px]") -> ScrollView:
    return ScrollView(
        "gap-2 px-2 py-2 bg-slate-900 rounded-2xl shrink-0",
        *[_card(i, size) for i in range(count)],
        horizontal=True,
    )


def _label(text: str) -> Text:
    return Text("text-slate-400 text-xs px-2 pt-2", text)


def view():
    return Column(
        # `min-h-0` on the screen root, from the rule WIDGETS.md states: a screen with a
        # scroll region *and* anything below it needs it on every `flex-1` container
        # from the root down. Without it the vertical list below grows to fit its
        # content and silently pushes everything after it off the display.
        "flex-1 min-h-0 bg-slate-950 gap-1",
        _label("1. bare horizontal strip (drag / fling)"),
        _strip(20),
        _label("3. plain Row + overflow-x-auto, no horizontal= flag"),
        Row(
            "overflow-x-auto min-w-0 gap-2 px-2 py-2 bg-slate-900 rounded-2xl shrink-0",
            *[_card(i, "w-[56px]") for i in range(20)],
        ),
        _label("4. strip with no room — a horizontal drag here must do nothing"),
        _strip(3, size="w-[80px]"),
        _label("2. strips inside a vertical list"),
        ScrollView(
            "flex-1 min-h-0 gap-2 px-2 pb-2",
            *[
                Column(
                    "shrink-0 gap-1 bg-slate-800 rounded-2xl p-2",
                    Text("text-slate-300 text-xs tabular-nums", f"row {r}"),
                    _strip(20, size="w-[64px]"),
                )
                for r in range(12)
            ],
        ),
    )
