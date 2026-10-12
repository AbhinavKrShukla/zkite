"""§28 — borders, shadows and rings on the native backend.

Written against the documented styling surface only: Tailwind class strings on the
ordinary widgets WIDGETS.md lists, no private modules, no `@store`, nothing adapted
from another demo app. §26.7 is what that rule is made of — `main.py` wired its
`Switch` with an undocumented `on_tap`, and that is how divergence 6 survived every
device run in a 7,000-line plan.

Everything here is readable from a screenshot with no log line, which matters because
none of these has any observable behaviour: a border either is on screen or it is not.
The screen is deliberately **light** (`bg-slate-100`) rather than the dark background
every other demo app uses, for one reason worth stating: Tailwind's whole shadow scale
is low-alpha black, and on `bg-slate-950` a correct shadow and an absent one look
identical. §28.4 found that the conformance harness has exactly this blindness in
numeric form.

Six rows, each answering a question the others cannot:

1. **`currentColor` borders.** Three swatches with a border width and *no border
   colour*, inside parents with different `text-*` colours. Tailwind v4 defaults
   `border-color` to `currentColor`; v3 defaulted it to gray-200, and that is what
   `twcore` had (§28.1). If these render gray instead of tracking the text colour, the
   v4 correction did not reach the device.

2. **Explicit border colours and widths**, including a `rounded-full` pill — the case
   whose corner radius has to be clamped to the box or the path is garbage.

3. **Per-side widths and colours.** The general case §18.6 warned is four mitred
   regions rather than one stroked rect. Each side is a different colour so a wrong
   mitre shows as colour bleeding across a corner diagonal, which is unmistakable.

4. **The shadow scale**, every size in order. v4 renamed it (`shadow-xs` is v3's
   `shadow-sm`) and made sm/md/lg/xl two-layer; a single-layer render is a visibly
   flatter shadow, not a slightly cheaper one.

5. **Rings**, including `ring` with no colour (v4: currentColor at 1px; v3: blue-500 at
   3px — both numbers changed) and a ring on a swatch immediately beside another, since
   a ring paints *outside* its own box and taffy reserves it no room. If rings are being
   drawn inside the border box, the spacing here is wrong in an obvious way.

6. **All three at once on one node**, which is the only row that checks paint *order*:
   the shadow must be under the background, and the border over it. A border under the
   background disappears entirely on an opaque node.
"""

from zkite import Column, Row, Text


def _swatch(cls: str) -> Column:
    return Column(f"w-[52px] h-[52px] {cls}")


def _label(text: str) -> Text:
    return Text("text-xs text-slate-600", text)


def view():
    return Column(
        "flex-1 min-h-0 bg-slate-100 p-4 gap-3",
        Text("text-base font-bold text-slate-900", "§28 borders / shadows / rings"),

        # 1. currentColor — no border-colour class anywhere in this row.
        _label("1  border, no colour class -> currentColor"),
        Row(
            "items-center gap-4",
            Column("text-emerald-600", _swatch("border-4")),
            Column("text-rose-600", _swatch("border-4")),
            Column("text-slate-900", _swatch("border-2")),
        ),

        # 2. explicit colours, widths, radii
        _label("2  explicit colour + radius"),
        Row(
            "items-center gap-4",
            _swatch("border-4 border-indigo-500 rounded-xl bg-white"),
            _swatch("border-2 border-slate-900 rounded-full bg-white"),
            _swatch("border-8 border-amber-400 bg-white"),
        ),

        # 3. per-side: each side a different colour, so a bad mitre is obvious
        _label("3  per-side widths and colours"),
        Row(
            "items-center gap-4",
            _swatch("border-t-8 border-b-4 border-l-2 border-red-500 bg-white"),
            _swatch("border-4 border-sky-500 border-t-amber-400 rounded-lg bg-white"),
        ),

        # 4. the whole shadow scale, on cards the same colour as the page
        _label("4  shadow xs / sm / md / lg / xl / 2xl"),
        Row(
            "items-center gap-4",
            _swatch("bg-slate-100 rounded-lg shadow-xs"),
            _swatch("bg-slate-100 rounded-lg shadow-sm"),
            _swatch("bg-slate-100 rounded-lg shadow-md"),
            _swatch("bg-slate-100 rounded-lg shadow-lg"),
            _swatch("bg-slate-100 rounded-lg shadow-xl"),
            _swatch("bg-slate-100 rounded-lg shadow-2xl"),
        ),

        # 5. rings — outside the border box, so spacing is part of the check
        _label("5  ring (no colour) / ring-4 / ring beside a neighbour"),
        Row(
            "items-center gap-4 text-fuchsia-600",
            _swatch("bg-white ring"),
            _swatch("bg-white ring-4 ring-indigo-500 rounded-lg"),
            _swatch("bg-white ring-2 ring-emerald-500"),
            _swatch("bg-slate-300"),
        ),

        # 6. paint order: shadow under the background, border over it
        _label("6  all three on one node"),
        Row(
            "items-center gap-4",
            _swatch("bg-white border-4 border-slate-900 rounded-xl shadow-lg ring-2 ring-indigo-400"),
        ),
    )
