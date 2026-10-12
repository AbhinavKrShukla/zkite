"""Per-widget conformance fixtures — plan1.md §5's "every widget gets a conformance
test that must produce pixel-comparable output on both backends before the switch
flips" (phase_3/PLAN.md §5 item 5).

One fixture per one of phase 1's 12 widgets (`zkite.__all__` minus `App`/`store`/
`Widget`), each a small, self-contained scene with an *explicit* pixel width/height on
its own root node — deliberately not relying on either backend's own root-stretch
wrapper (`phase_1/webruntime/index.html`'s `#zkite-root` CSS / `zkite_pybridge::build`'s
synthetic `flex flex-col w-full h-full`, see that function's own doc comment) so a
screenshot of *just* this fixture's root element is exactly W×H on both sides, with no
dependency on how either shell happens to stretch an unconstrained root.

For each fixture this writes, into `manifest.json`:
  - `html`: the exact `Widget.render()` fragment (WebView backend input)
  - `tree`: the exact `Widget.to_tree()` structure (native backend input, same shape
    `zkite_pybridge::RawNode` deserializes)
  - `width`/`height`: the fixture's own canvas size

Both `html` and `tree` come from the *same* `RenderContext`/node-id walk
(`Widget.render`/`Widget.to_tree` share `_register_events`/node-id assignment — see
widgets.py's own module doc) so a real behavioral difference between the two backends'
inputs, not an artifact of this script building them two different ways.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

HERE = Path(__file__).parent
sys.path.insert(0, str(HERE.parent.parent / "phase_1" / "zkite_py" / "src"))

from zkite._viewport import set_viewport  # noqa: E402
from zkite.widgets import (  # noqa: E402
    Button, Checkbox, Column, Grid, Icon, Image, List, Overlay, Radio, RenderContext, Row,
    ScrollView, Select, Slider, Stack, Switch, Text, TextField, Widget,
)

# The bitmaps the `Image` fixtures draw, inlined as `data:` URIs.
#
# A `data:` URI rather than a file for a reason that matters to this harness
# specifically: it is the one `src` form that needs no app directory, so Chromium and
# `zkite_imagestore` decode *identical bytes* with no staging step in between, and any
# difference the diff reports is a difference in painting (phase_3/PLAN.md §35.4).
#
# **Four distinct quadrants, not a flat colour.** Until §35 this was a single red
# pixel scaled up by the `<img>` box, which is the one image whose diff is blind to
# almost everything worth checking: a vertical flip, a horizontal mirror, a transposed
# axis and *any* wrong `object-fit` mode all produce a byte-identical flat rectangle.
# `object-cover` on a 1x1 source is also indistinguishable from `object-fill`. The
# quadrant bitmap is deliberately non-square (2:1) so a mode that ignores the aspect
# ratio cannot pass.
def _quadrant_png_data_uri(w: int, h: int) -> str:
    import base64

    from pngtool import quadrants

    path = HERE / "_fixture_quadrants.png"
    quadrants(str(path), w, h)
    data = path.read_bytes()
    path.unlink()
    return "data:image/png;base64," + base64.b64encode(data).decode()


# 2:1, so `contain`/`cover`/`fill` are three visibly different answers in a square box.
_QUADRANTS_DATA_URI = _quadrant_png_data_uri(40, 20)

Fixture = tuple[Widget, int, int]


def _fixtures() -> dict[str, Fixture]:
    return {
        "column": (
            Column(
                "w-[240px] h-[220px] bg-slate-950 p-3 gap-2",
                Button("bg-indigo-500 rounded-lg h-[40px] text-white text-sm", "A"),
                Button("bg-emerald-500 rounded-lg h-[40px] text-white text-sm", "B"),
                Button("bg-rose-500 rounded-lg h-[40px] text-white text-sm", "C"),
            ),
            240, 220,
        ),
        "row": (
            Row(
                "w-[240px] h-[100px] bg-slate-950 p-3 gap-2 items-center",
                Button("bg-indigo-500 rounded-lg w-[50px] h-[50px] text-white text-sm", "1"),
                Button("bg-emerald-500 rounded-lg w-[50px] h-[50px] text-white text-sm", "2"),
                Button("bg-rose-500 rounded-lg w-[50px] h-[50px] text-white text-sm", "3"),
            ),
            240, 100,
        ),
        "grid": (
            Grid(
                "w-[240px] h-[160px] bg-slate-950 p-3 gap-2 grid-cols-3",
                Button("bg-indigo-500 rounded-md", ""),
                Button("bg-emerald-500 rounded-md", ""),
                Button("bg-rose-500 rounded-md", ""),
                Button("bg-amber-500 rounded-md", ""),
                Button("bg-sky-500 rounded-md", ""),
                Button("bg-violet-500 rounded-md", ""),
            ),
            240, 160,
        ),
        "stack": (
            Stack(
                "w-[200px] h-[200px]",
                Button("bg-indigo-500 rounded-2xl w-[200px] h-[200px]", ""),
                Button("bg-white rounded-full w-[80px] h-[80px]", ""),
            ),
            200, 200,
        ),
        # The two fixtures below exist because the one above agreed with Chromium by
        # construction rather than by being right, and hid two separate `Stack` bugs
        # for the whole of phase 3 (phase_3/PLAN.md §31.1, §31.2). It overlays two
        # opaque rectangles, both with an explicit width and height and *neither with
        # any text in it* — which is exactly the shape that cannot see either defect.
        # Each fixture below isolates one of them, so a failure names its own cause.
        #
        # 1. Paint order. Both children explicitly sized, so no sizing question is in
        #    play; the only thing that can differ is whether the second child's opaque
        #    background covers the first child's glyphs. It did not: the native paint
        #    path ran three whole-tree passes (all boxes, all icons, all glyphs), so
        #    *every* glyph in the tree landed above *every* background in it. 2.77%
        #    before the fix, 1.06% after, max channel delta 255 -> 176.
        "stack_paint_order": (
            Stack(
                "w-[240px] h-[120px] bg-slate-950",
                Column("w-[240px] h-[120px] p-3", Text("text-white text-lg", "UNDERNEATH")),
                Column("w-[240px] h-[120px] bg-emerald-600 p-3", Text("text-white text-lg", "ON TOP")),
            ),
            240, 120,
        ),
        # 2. A `Stack` child with no explicit width. The HTML wrapper is a grid item
        #    (`display:block`) so the child fills it; the native wrapper carried no
        #    display class, so taffy's own `flex`/`row` default sized the child to its
        #    content instead. No text overlaps here, so this fixture is unaffected by
        #    the paint order above and moves only with the wrapper's classes.
        "stack_fill": (
            Stack(
                "w-[240px] h-[120px] bg-slate-950",
                Column("bg-emerald-600 p-3", Text("text-white text-lg", "FILLS THE STACK")),
            ),
            240, 120,
        ),
        "scrollview": (
            ScrollView(
                "w-[200px] h-[160px] bg-slate-950 gap-2 p-2",
                *[
                    Row(
                        "bg-slate-800 rounded h-[28px] items-center px-2 shrink-0",
                        Text("text-white text-sm", f"Item {i}"),
                    )
                    for i in range(8)
                ],
            ),
            200, 160,
        ),
        # The horizontal form of the one above (phase_3/PLAN.md §27). This harness
        # cannot scroll anything — it renders one state and never touches the canvas —
        # so what this fixture pins is the half it *can* see, and that half is not
        # nothing: at offset 0 it checks that both backends lay a `flex-row` overflow
        # strip out identically and, critically, **clip it in the same place**. A
        # horizontal strip is the first thing in this project whose content
        # deliberately runs past its own right edge, and "where does the fourth card
        # get cut" is a real, pixel-visible answer that a vertical fixture cannot ask.
        #
        # Six 60px cards plus gaps in a 200px box overflows by design; `shrink-0` is
        # load-bearing for the same reason it is on `scrollview`'s rows, one axis over.
        "scrollview_horizontal": (
            ScrollView(
                "w-[200px] h-[80px] bg-slate-950 gap-2 p-2",
                *[
                    Row(
                        "bg-slate-800 rounded w-[60px] h-full items-center justify-center shrink-0",
                        Text("text-white text-sm", f"{i}"),
                    )
                    for i in range(6)
                ],
                horizontal=True,
            ),
            200, 80,
        ),
        "text": (
            Column(
                "w-[220px] h-[80px] bg-slate-950 items-center justify-center",
                Text("text-3xl font-bold text-white", "Hello ZKite"),
            ),
            220, 80,
        ),
        # The three generic families and an explicit line-height, together — both were
        # parsed, inherited, and then ignored by every shaping site until
        # ../PLAN.md §9.4/§9.7. A fixture that only used the default family would have
        # gone on passing throughout, which is exactly how both gaps survived this long.
        "typography": (
            Column(
                "w-[240px] h-[140px] bg-slate-950 p-2 gap-1",
                Text("text-base font-sans text-white", "Sans Hamburgefonstiv"),
                Text("text-base font-serif text-white", "Serif Hamburgefonstiv"),
                Text("text-base font-mono text-white", "Mono Hamburgefonstiv"),
                Text("text-base leading-loose text-white", "Loose leading"),
            ),
            240, 140,
        ),
        "button": (
            Column(
                "w-[200px] h-[80px] bg-slate-950 items-center justify-center",
                Button("px-6 py-3 rounded-2xl bg-indigo-500 text-white font-medium", "Increment"),
            ),
            200, 80,
        ),
        "icon": (
            Column(
                "w-[100px] h-[100px] bg-slate-950 items-center justify-center text-[20px] text-white",
                Icon("", "star"),
            ),
            100, 100,
        ),
        # A second icon fixture, and not for coverage's sake. `star` is one `<path>`
        # copied verbatim out of the sprite sheet, so it only proves the copy is
        # faithful. These two are the cases where something is *derived*:
        # `search`/`settings` are the only sprites containing a `<circle>`, which
        # `scripts/gen_icons.py` converts into two SVG arcs, and they are also the only
        # multi-shape sprites (so a paint loop that drew just the first shape would pass
        # the `star` fixture). Both are drawn against real Chromium rendering the same
        # `<symbol>`, which is the only check that can catch an arc with a plausible
        # bounding box and the wrong curve — `icons_parse_and_stay_inside_their_viewbox`
        # deliberately only bounds it. Rendered in `text-slate-300` rather than white so
        # `currentColor` inheritance is exercised at a value nothing else defaults to.
        "icon_circles": (
            Row(
                "w-[160px] h-[100px] bg-slate-950 items-center justify-center gap-6 text-[32px] text-slate-300",
                Icon("", "search"),
                Icon("", "settings"),
            ),
            160, 100,
        ),
        "textfield": (
            Column(
                "w-[220px] h-[60px] bg-slate-950 items-center justify-center p-2",
                TextField(
                    "bg-slate-800 text-white px-3 py-2 rounded w-full",
                    placeholder="Type here",
                ),
            ),
            220, 60,
        ),
        "image": (
            Column(
                "w-[120px] h-[90px] bg-slate-950 items-center justify-center",
                Image("w-[80px] h-[60px] rounded bg-white", src=_QUADRANTS_DATA_URI, alt="quadrants"),
            ),
            120, 90,
        ),
        # All five `object-fit` modes side by side, each in an identical square box
        # with a 2:1 source — the shape where every mode is a different answer
        # (phase_3/PLAN.md §35.4). `Image.implied_classes` already carries
        # `object-cover`, and a later class in the string wins, so each of these
        # genuinely overrides it.
        #
        # The white background is load-bearing: `contain` and `scale-down` letterbox,
        # and letterboxing is only visible against something. On a slate ground the
        # unfilled part of the box would be indistinguishable from a `fill` that got
        # the colours wrong.
        # **288, not 280.** At 280 the five 48px boxes plus four 8px gaps plus 16px of
        # padding overflow by 8px, flex-shrink engages, and the two backends resolve
        # the overflow differently — boxes landed at x=4..51 (web) against x=8..53
        # (native). That reported 14.88%, none of which was about images: the fixture
        # was measuring a flex-shrink divergence through an `Image`-shaped hole. Sized
        # so nothing shrinks, so a number here means what it says.
        "image_object_fit": (
            Row(
                "w-[288px] h-[64px] bg-slate-950 items-center justify-center gap-2 p-2",
                Image("w-[48px] h-[48px] bg-white object-fill", src=_QUADRANTS_DATA_URI, alt=""),
                Image("w-[48px] h-[48px] bg-white object-contain", src=_QUADRANTS_DATA_URI, alt=""),
                Image("w-[48px] h-[48px] bg-white object-cover", src=_QUADRANTS_DATA_URI, alt=""),
                Image("w-[48px] h-[48px] bg-white object-none", src=_QUADRANTS_DATA_URI, alt=""),
                Image("w-[48px] h-[48px] bg-white object-scale-down", src=_QUADRANTS_DATA_URI, alt=""),
            ),
            288, 64,
        ),
        "switch": (
            Row(
                "w-[140px] h-[60px] bg-slate-950 items-center justify-center gap-4",
                Switch("", checked=False),
                Switch("", checked=True),
            ),
            140, 60,
        ),
        # ---- Checkbox / Radio (phase_3/PLAN.md §30) ----
        #
        # Both states in one canvas, on purpose. The box itself is drawn by Tailwind
        # classes that both backends resolve independently (real Tailwind here,
        # `twcore` there), but the *mark inside it* is the one deliberate seam in these
        # widgets: an `<input>` is a void element, so the WebView draws its tick and dot
        # in CSS while the tree path nests an `Icon("check")` and a `div`. That seam is
        # exactly the two-hand-maintained-copies shape §9.5 (palette), §9.7 (fonts) and
        # §28.1 (v4 defaults) each paid for — and unlike a touch or a scroll, a static
        # checked state is something §9's harness genuinely can see and discriminate,
        # which is the whole argument §28 made for picking work a pixel diff can check.
        #
        # Unchecked is included rather than assumed: an unchecked box is where a
        # border-colour or fill regression shows, since there is no mark on top of it.
        "checkbox": (
            Row(
                "w-[140px] h-[60px] bg-slate-950 items-center justify-center gap-4",
                Checkbox("", checked=False),
                Checkbox("", checked=True),
            ),
            140, 60,
        ),
        "radio": (
            Row(
                "w-[140px] h-[60px] bg-slate-950 items-center justify-center gap-4",
                Radio("", checked=False),
                Radio("", checked=True),
            ),
            140, 60,
        ),
        # ---- labelled controls (phase_3/PLAN.md §32) ----
        #
        # A label makes the control a *container*: box, gap and text laid out together
        # and aligned to each other. That is new geometry, and unlike the tap
        # behaviour it is something this harness can genuinely see — which is why it
        # is here rather than only on a device.
        #
        # What it is built to discriminate, since §31.7's lesson is that a fixture
        # which cannot move for the thing it is named after is worthless: the `mr-2`
        # gap, the `items-center` cross-axis alignment of a 20px box against a text
        # line, and — because the box is emitted by the same `_box_classes` the bare
        # form uses — that a labelled box is drawn identically to the bare `checkbox`
        # and `radio` fixtures above. Both checked states are present for the same
        # reason those two include unchecked: an empty box is where a border or fill
        # regression has nothing to hide behind.
        #
        # Note this fixture also removes a seam rather than adding one on the HTML
        # side: the real `<input>` is still there and still drawn by `index.html`'s
        # CSS, so labelled and bare controls cannot render differently on this backend.
        "labelled_controls": (
            Column(
                "w-[220px] h-[120px] bg-slate-950 justify-center gap-3 p-4",
                Checkbox("text-sm text-white", "Wi-Fi", checked=True),
                Checkbox("text-sm text-white", "Bluetooth", checked=False),
                Radio("text-sm text-white", "High", checked=True),
            ),
            220, 120,
        ),
        # `indicator=False` (§32.5) — an MCQ option, where the *option itself* shows
        # the selection and there is no 20dp circle at all. The node is still a real
        # `role="radio"` carrying `checked`, so this is also the fixture that proves
        # dropping the box does not drag any of the box's styling along with it: if
        # `_box_full_classes` leaked onto the container, or the container's own
        # `border-2` resolved differently between real Tailwind and `twcore`, the two
        # cards would stop agreeing here.
        "radio_custom": (
            Column(
                "w-[220px] h-[130px] bg-slate-950 justify-center gap-2 p-4",
                Radio("px-3 py-2 rounded-lg border-2 border-indigo-500 bg-indigo-950 text-sm text-white",
                      "A. Selected", checked=True, indicator=False),
                Radio("px-3 py-2 rounded-lg border-2 border-slate-700 text-sm text-slate-300",
                      "B. Not selected", checked=False, indicator=False),
            ),
            220, 130,
        ),
        "slider": (
            Column(
                "w-[200px] h-[60px] bg-slate-950 items-center justify-center p-4",
                Slider("w-full", min=0, max=100, value=30),
            ),
            200, 60,
        ),
        # The value -> geometry mapping across the whole range, which the single
        # `slider` fixture above cannot see (phase_3/PLAN.md §26, §9.3's own "one
        # `Slider` value" gap).
        #
        # This is the half of divergence 6 that a *rendering* harness can check.
        # Interaction is still invisible to it — nothing here touches anything — but
        # a drag is only correct if the position it computes and the position both
        # backends *paint* agree, and now that a drag actually reaches 0 and 100
        # those two ends are where a fill-width or thumb-offset error stops being
        # subtle: a thumb that hangs off the track, or a fill that never empties,
        # shows here and nowhere else in the suite.
        "slider_range": (
            Column(
                "w-[200px] h-[130px] bg-slate-950 justify-center gap-3 p-4",
                Slider("w-full", min=0, max=100, value=0),
                Slider("w-full", min=0, max=100, value=50),
                Slider("w-full", min=0, max=100, value=100),
            ),
            200, 130,
        ),
        # ---- box decoration (phase_3/PLAN.md §28) ----
        #
        # Four fixtures rather than one, because border/shadow/ring fail in different
        # ways and a single scene would let one of them hide inside another's number.
        #
        # These are also the fixtures that pin §28.1's Tailwind-v4 default corrections.
        # The harness *can* see those, unlike the interaction gaps §26.6 and §27.5 had
        # to concede — a border is a static thing at offset 0 — which is the reason
        # this item was picked over the rest of the queue.
        #
        # `border`'s first child deliberately carries **no border-colour class at all**,
        # inside a `text-emerald-400` parent. That is the whole v4 `currentColor`
        # question in one node: with v3's default it renders gray-200, and Chromium
        # renders emerald. Nothing else here would catch it.
        "border": (
            Row(
                "w-[260px] h-[100px] bg-slate-950 items-center justify-center gap-4 p-4 text-emerald-400",
                Column("w-[60px] h-[60px] border-4"),
                Column("w-[60px] h-[60px] border-4 border-indigo-500 rounded-xl"),
                Column("w-[60px] h-[60px] border border-white rounded-full"),
            ),
            260, 100,
        ),
        # The general case §18.6 correctly warned was not one stroked rect: per-side
        # widths and per-side colours, both of which `twcore` models and Tailwind
        # allows. The second child mixes them — a uniform width with one side recoloured
        # — which is the case that must *not* take the uniform fast path.
        "border_sides": (
            Row(
                "w-[240px] h-[110px] bg-slate-950 items-center justify-center gap-6 p-4",
                Column("w-[70px] h-[70px] border-t-8 border-b-4 border-l-2 border-red-500"),
                Column("w-[70px] h-[70px] border-4 border-sky-500 border-t-amber-400 rounded-lg"),
            ),
            240, 110,
        ),
        # A light background, and cards the same colour as it, so that what is being
        # compared is the shadow and nothing else. Four sizes because v4's scale is
        # renamed *and* mostly two-layer (§28.1): `xs` is one layer, `sm`/`md`/`lg` are
        # two, and rendering only the first layer of each is exactly the plausible-
        # looking wrong answer this fixture exists to reject.
        "shadow": (
            Row(
                "w-[370px] h-[150px] bg-white items-center justify-center gap-5 p-5",
                Column("w-[50px] h-[50px] bg-white rounded-lg shadow-xs"),
                Column("w-[50px] h-[50px] bg-white rounded-lg shadow-sm"),
                Column("w-[50px] h-[50px] bg-white rounded-lg shadow-md"),
                Column("w-[50px] h-[50px] bg-white rounded-lg shadow-lg"),
                # `shadow-2xl` is the only entry in the scale dark enough (alpha 0.25,
                # against 0.05–0.1 for the rest) for this harness to see a shadow error
                # at all — see §28.4, where the negative control for the blur/std_dev
                # conversion passed against the four faint ones.
                Column("w-[50px] h-[50px] bg-white rounded-lg shadow-2xl"),
            ),
            370, 150,
        ),
        # The whole `--radius-*` scale, every named entry (phase_3/PLAN.md §36).
        #
        # Nothing exercised the scale before this: `border`/`shadow`/`ring` above each
        # pick one convenient `rounded-*` to have a corner at all, and between them
        # they happened to name only tokens v3 and v4 agree on — so `rounded-sm` being
        # *half* the v4 radius, and `rounded-xs`/`rounded-4xl` not parsing at all, were
        # invisible to a clean 21-fixture run. That is §28.4's blindness lesson in a
        # second place, and the reason this fixture is the whole table rather than a
        # representative sample.
        #
        # **White on black, and boxes no larger than they need to be.** A radius error
        # only changes pixels inside the corner arc, so the diff is a few dozen pixels
        # per box; the harness's `CHANNEL_TOLERANCE` of 40 means those pixels have to
        # be at maximum contrast to register at all. Every box is 70x70, which is wide
        # enough that even `4xl`'s 32dp corner is a true arc rather than a clamped one
        # (a box under 64dp would clamp, and a clamped corner hides the value being
        # tested behind the clamp).
        "radius_scale": (
            Column(
                "w-[320px] h-[164px] bg-black p-2 gap-2",
                Row(
                    "gap-2",
                    Column("w-[70px] h-[70px] bg-white rounded-xs"),
                    Column("w-[70px] h-[70px] bg-white rounded-sm"),
                    Column("w-[70px] h-[70px] bg-white rounded-md"),
                    Column("w-[70px] h-[70px] bg-white rounded-lg"),
                ),
                Row(
                    "gap-2",
                    Column("w-[70px] h-[70px] bg-white rounded-xl"),
                    Column("w-[70px] h-[70px] bg-white rounded-2xl"),
                    Column("w-[70px] h-[70px] bg-white rounded-3xl"),
                    Column("w-[70px] h-[70px] bg-white rounded-4xl"),
                ),
            ),
            320, 164,
        ),
        # A ring is not a border: it sits entirely *outside* the border box, so it
        # overlaps whatever is next to it and nothing in taffy reserves room for it.
        # The first child again carries no ring colour, pinning v4's `currentColor`
        # default (v3's was blue-500) and its 1px default width (v3's was 3px).
        "ring": (
            Row(
                "w-[240px] h-[100px] bg-slate-950 items-center justify-center gap-8 p-4 text-fuchsia-400",
                Column("w-[56px] h-[56px] bg-slate-800 ring"),
                Column("w-[56px] h-[56px] bg-slate-800 ring-4 ring-indigo-500 rounded-lg"),
            ),
            240, 100,
        ),
        # The virtualised list (phase_3/PLAN.md §11). Deliberately shaped like the
        # `scrollview` fixture above — same canvas, same row look — so the two are
        # readable side by side, but with 500 rows instead of 8 and no `gap-*` (a
        # `List` refuses one; row spacing lives inside `item_height`, see the
        # widget's own note). What this fixture is actually checking is that both
        # backends materialise the *same window* and place its rows identically:
        # the spacers are the mechanism, and a divergence in either backend's
        # handling of them would show up here as an obvious vertical offset rather
        # than as anti-aliasing noise.
        "list": (
            List(
                "w-[200px] h-[160px] bg-slate-950 p-2",
                count=500,
                item_height=28,
                overscan=1,
                builder=lambda i: Row(
                    "bg-slate-800 rounded h-full items-center px-2",
                    Text("text-white text-sm", f"Item {i}"),
                ),
            ),
            200, 160,
        ),
        # ---- gradients (phase_3/PLAN.md §38) ----
        #
        # Three fixtures, each isolating one thing that can be wrong, because a single
        # "gradients look right" fixture would report one number for four independent
        # failures — §31.7's lesson, and §28.4's.
        #
        # `from-yellow-400 to-blue-600` throughout, and that pair is chosen rather than
        # a pretty one: it is the pair whose midpoint differs by **81/255 in blue**
        # between oklab and sRGB interpolation (measured, §38.1). Both of this harness's
        # tolerances (40 loose, 12 strict) can see that, so these fixtures are the first
        # in the table that can catch a wrong *value* rather than only an absent
        # feature — §28.4/§36.4's blindness does not apply to them, and the control table
        # in §38.4 is the proof.
        #
        # All eight keyword directions in one fixture, because the failure mode a
        # direction table has is a swapped axis or an inverted sign, which is invisible
        # in any single box and unmissable in a grid of eight.
        "gradient_linear": (
            Column(
                "w-[240px] h-[124px] bg-slate-950 p-2 gap-2",
                Row(
                    "gap-2",
                    Column("w-[54px] h-[54px] bg-linear-to-t from-yellow-400 to-blue-600"),
                    Column("w-[54px] h-[54px] bg-linear-to-r from-yellow-400 to-blue-600"),
                    Column("w-[54px] h-[54px] bg-linear-to-b from-yellow-400 to-blue-600"),
                    Column("w-[54px] h-[54px] bg-linear-to-l from-yellow-400 to-blue-600"),
                ),
                Row(
                    "gap-2",
                    Column("w-[54px] h-[54px] bg-linear-to-tr from-yellow-400 to-blue-600"),
                    Column("w-[54px] h-[54px] bg-linear-to-tl from-yellow-400 to-blue-600"),
                    Column("w-[54px] h-[54px] bg-linear-to-br from-yellow-400 to-blue-600"),
                    # v3's spelling, still live in v4.3.3 — the one box that would go
                    # black if `bg-gradient-to-*` were dropped as an unknown utility.
                    Column("w-[54px] h-[54px] bg-gradient-to-bl from-yellow-400 to-blue-600"),
                ),
            ),
            240, 124,
        ),
        # The angle and corner geometry, on boxes that are **not square**. A square box
        # cannot distinguish CSS's corner rule (perpendicular through the other two
        # corners) from a plain 45deg line, and it cannot distinguish CSS's gradient-line
        # length (|w·sin a| + |h·cos a|) from the box diagonal. Every box here is 2:1.
        "gradient_angle": (
            Column(
                "w-[240px] h-[164px] bg-slate-950 p-2 gap-2",
                Column("w-[224px] h-[48px] bg-linear-45 from-yellow-400 to-blue-600"),
                Column("w-[224px] h-[48px] bg-linear-[30deg] from-yellow-400 to-blue-600"),
                Column("w-[224px] h-[48px] bg-linear-to-tr from-yellow-400 to-blue-600"),
            ),
            240, 164,
        ),
        # The stop model: a middle stop, explicit positions, the transparent far end of a
        # one-stop gradient, and a gradient over a background *colour*.
        #
        # The third and fourth rows are the ones that check something no amount of
        # geometry does. A one-stop gradient fades to `transparent`, and CSS interpolates
        # to it in **premultiplied** alpha — read unpremultiplied, the midpoint of
        # yellow → transparent over red goes muddy instead of staying yellow-ish
        # (measured: (128, 219, 236) vs (127, 173, 182) for the cyan case in §38.1).
        # Row 4 is `bg-red-500` *under* that fade, so the colour beneath a transparent
        # stop is part of the picture rather than a claim about it.
        "gradient_stops": (
            Column(
                "w-[240px] h-[164px] bg-slate-950 p-2 gap-2",
                Column(
                    "w-[224px] h-[32px] bg-linear-to-r from-yellow-400 via-fuchsia-500 to-blue-600"
                ),
                Column(
                    "w-[224px] h-[32px] bg-linear-to-r from-yellow-400 from-25% to-blue-600 to-75%"
                ),
                Column("w-[224px] h-[32px] bg-linear-to-r from-yellow-400"),
                Column("w-[224px] h-[32px] bg-red-500 bg-linear-to-r from-yellow-400"),
            ),
            240, 164,
        ),
        # The interpolation *space*, isolated from the gamut question entirely.
        #
        # white → black is in gamut at every point of the ramp, so nothing here can be
        # explained by clipping — and the two spaces disagree loudly about it, because
        # oklab is perceptual: the midpoint is (145, 145, 145) in oklab and (128, 128, 128)
        # in sRGB. Both rows must match Chromium, and swapping the default space makes
        # *both* fail, which is what makes this the fixture that guards §38.1's
        # `interpolation_cs` finding.
        "gradient_space": (
            Column(
                "w-[240px] h-[88px] bg-slate-950 p-2 gap-2",
                Column("w-[224px] h-[32px] bg-linear-to-r from-white to-black"),
                Column("w-[224px] h-[32px] bg-linear-to-r/srgb from-white to-black"),
            ),
            240, 88,
        ),
        # ---- opacity and transform (phase_3/PLAN.md §45) ----
        #
        # **Neither property has ever had a fixture, and that is why both could be
        # parsed into `ComputedStyle` and read by no native paint code at all.**
        # `opacity-*` compiles to `paint.opacity` (twcore `decl.rs`) and
        # `translate-*`/`rotate-*`/`scale-*` compile to `ComputedStyle::transform`;
        # the WebView backend compiles the identical class strings with the real
        # Tailwind browser build, so all of them have always worked there. That is a
        # §15.1 divergence of exactly the §14.2 shape — renders, nothing errors, the
        # behaviour is simply absent — and it stayed invisible because this harness
        # never asked. WIDGETS.md listed `opacity` under "works on both".
        #
        # They matter now because they are the two channels an animation driver needs
        # first: a fade and a slide (§45.2). A driver that can only animate values
        # nothing paints is not a driver.
        "opacity": (
            Column(
                "w-[240px] h-[136px] bg-slate-950 p-3 gap-2",
                Column("w-[216px] h-[32px] rounded-md bg-emerald-500"),
                Column("w-[216px] h-[32px] rounded-md bg-emerald-500 opacity-50"),
                Column("w-[216px] h-[32px] rounded-md bg-emerald-500 opacity-25"),
            ),
            240, 136,
        ),
        # The one that distinguishes *group* opacity from inherited alpha, which are
        # the two available implementations and are not the same picture.
        #
        # CSS `opacity` composites the subtree first and fades the result, so the
        # white panel completely hides the emerald behind it and the left half reads
        # 50% white over slate. Multiplying each node's own alpha instead paints
        # emerald at 50% over slate and then white at 50% over *that*, i.e. 50% white
        # + 25% emerald + 25% slate — a visibly greener left half. Inherited alpha is
        # the easier implementation and the one a first draft reaches for, so the
        # fixture that can see the difference is the one worth having. §31.8 files
        # "no opacity groups" as a known gap; this is the half of it that a fade
        # actually needs.
        "opacity_group": (
            Column(
                "w-[240px] h-[100px] bg-slate-950 p-2",
                Stack(
                    "w-[224px] h-[84px] opacity-50",
                    Column("w-[224px] h-[84px] rounded-lg bg-emerald-500"),
                    Column("w-[120px] h-[84px] rounded-lg bg-white"),
                ),
            ),
            240, 100,
        ),
        # A transform moves a node **and its subtree** without moving anything else's
        # layout — which is why the third row here overlaps the second rather than
        # pushing it aside, and why the nested child rides along with its parent. Both
        # are properties an implementation that merely offset a rect at paint time
        # would get wrong, so both are in the picture.
        #
        # Spacing-scale translates rather than arbitrary `[60px]` ones: `twcore`'s
        # `translate-y-4 == 16.0` is already pinned by its own test suite, so this
        # fixture measures painting rather than re-testing the parser.
        "translate": (
            Column(
                "w-[240px] h-[120px] bg-slate-950 p-2 gap-2",
                Column("w-[100px] h-[28px] rounded bg-sky-500"),
                Column("w-[100px] h-[28px] rounded bg-sky-500 translate-x-8"),
                Column(
                    "w-[100px] h-[28px] rounded bg-amber-400 translate-x-4 -translate-y-2",
                    Column("w-[24px] h-[28px] rounded bg-rose-600"),
                ),
            ),
            240, 120,
        ),
        # ---- Select (phase_3/PLAN.md §33) ----
        #
        # Two fixtures, because a picker has two rendered states and only one of them
        # is a widget in the tree. The closed one is the trigger: a label, a chevron
        # sprite and `justify-between` deciding where the chevron lands. Both backends
        # resolve those independently — real Tailwind here, `twcore` there — and the
        # chevron is a *new* sprite, so this is also the check that
        # `scripts/gen_icons.py` transcribed it faithfully rather than plausibly.
        "select_closed": (
            Column(
                "w-[280px] h-[80px] bg-slate-950 p-4 justify-center",
                Select(
                    "px-4 py-3 rounded-xl bg-slate-800 text-white text-sm",
                    options=[("br", "Brazil"), ("in", "India")],
                    value="in",
                ),
            ),
            280, 80,
        ),
        # The open one is the *layer*, which is not a node in the view tree at all —
        # `build_manifest` renders it the way `app.App` does. This is the fixture that
        # can see the popup's own composition: a full-bleed scrim, a panel inset from
        # it, a selected row with its own background and tick, and two unselected rows.
        #
        # Rendered from the real `Select.popup()` rather than a hand-written copy of
        # its markup, so it cannot agree with a widget that no longer emits this. That
        # is the §18.2/§9.5 lesson: point the second consumer at the thing the first
        # one actually runs.
        "select_open": (
            Select(
                "",
                options=[("br", "Brazil"), ("in", "India"), ("ke", "Kenya")],
                value="in",
                # An opaque backdrop on the layer root: a fixture needs one anyway (the
                # harness waits for a real background to resolve before screenshotting),
                # and it makes the scrim's own alpha measurable against a known colour.
                placement="items-center justify-center p-4 bg-slate-950",
                panel="w-full max-h-[220px] flex-col rounded-2xl bg-slate-800 p-2",
            ).popup("r"),
            280, 260,
        ),
    }


def build_manifest() -> dict:
    manifest = {}
    for name, (widget, width, height) in _fixtures().items():
        # An `Overlay` fixture is rendered as a *layer*, exactly as `app.App` does it
        # (phase_3/PLAN.md §33): an overlay produces nothing where it is written, so
        # the ordinary `render()`/`to_tree()` below would emit an empty fixture. The
        # layer's own root becomes `#r`, which is what both halves of this harness
        # screenshot.
        if isinstance(widget, Overlay):
            set_viewport("r", 0.0, float(height))
            manifest[name] = {
                "width": width,
                "height": height,
                "html": widget.layer_html(RenderContext(), "r"),
                "tree": widget.layer_tree(RenderContext(), "r"),
            }
            continue
        # Stand in for the runtime's own scroll reporting (phase_3/PLAN.md §11):
        # both backends normally tell `zkite_py` how tall a scroll container turned
        # out to be, and a virtualised `List` picks its window from that. Nothing
        # here lays anything out, so this asserts the answer that layout would have
        # produced — the fixture's own canvas — which is both the truthful value
        # and the one that makes `List`'s window representative rather than the
        # deliberately over-large bootstrap one. Every fixture gets it; only `List`
        # reads it.
        set_viewport("r", 0.0, float(height))
        html_ctx = RenderContext()
        html = widget.render(html_ctx, "r")
        tree_ctx = RenderContext()
        tree = widget.to_tree(tree_ctx, "r")
        manifest[name] = {"width": width, "height": height, "html": html, "tree": tree}
    return manifest


if __name__ == "__main__":
    out_path = HERE / "manifest.json"
    manifest = build_manifest()
    out_path.write_text(json.dumps(manifest, indent=2))
    print(f"wrote {out_path} ({len(manifest)} fixtures: {', '.join(sorted(manifest))})")
