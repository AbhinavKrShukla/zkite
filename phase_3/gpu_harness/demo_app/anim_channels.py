"""§45 — the two channels an animation driver needs, on the native backend.

`opacity-*` and `translate-*` have been parsed by `twcore` into `ComputedStyle` since
phase 0 and read by **no** native paint code since. The WebView backend compiles the
same class strings with the real Tailwind browser build, so both have always worked
there — a §14.2 silent divergence that nothing could see, because the conformance
harness had no fixture for either and WIDGETS.md listed `opacity` under "works on
both". Measured against real Chromium before the fix: `opacity` 42.18% of pixels
wrong, `opacity_group` 78.17%, `translate` 15.19%.

Written against documented signatures only (§26.7's rule): `Column`/`Row`/`Stack`/
`Text` with class strings, and `on_tap` on a `Column`. Nothing here reaches into the
framework to observe itself.

**Rows 1-3 are readable off a screenshot. Row 4 is not, and that is why it is here.**

1. **Three opacity levels over one background.** `opacity-100`, `-50`, `-25` on the
   same emerald against the same slate. If opacity is dropped, all three bands are
   byte-identical — which is exactly what a band-compare of the pre-fix build shows.

2. **Group opacity, not inherited alpha.** A `Stack` at `opacity-50` holding an
   emerald panel with an opaque white panel over its left half. CSS composites the
   subtree and *then* fades it, so the left half reads 50% white over slate with no
   emerald in it at all. Multiplying each node's own alpha instead leaves 25% emerald
   mixed in and the left half comes out visibly green. Two implementations, one
   picture each; this row says which one shipped.

3. **A translate moves a node and its subtree, and displaces nothing.** The middle bar
   carries `translate-x-8`, the bottom one `translate-x-4 -translate-y-2` plus a rose
   child that must ride along with it. The top bar is the control: it must not move.

4. **A translated node is *tapped* where it is *drawn*.** The counter's own box is
   translated 8 units right. Tapping where it appears must increment it; tapping where
   it *would* have been without the transform must not. That distinction is invisible
   in a screenshot and is the §9.7/§13 failure shape — a coordinate-space slip only a
   finger finds — so the count is rendered as text and the two taps are the test.
"""

from zkite import Button, Column, Stack, Text, store


@store
class S:
    hits: int = 0
    misses: int = 0


def view(s: S):
    # Handlers close over the injected instance, exactly as WIDGETS.md's own reference
    # app writes them (`on_tap=lambda: c.update(n=c.n + 1)`). `update` is an *instance*
    # method; a first draft of this file called `S.update(...)` on the class, which
    # raises inside the handler — the framework caught it and painted its error
    # overlay, which is correct behaviour and cost a device run to notice.
    return Column(
        "flex-1 bg-slate-950 p-4 gap-4",
        Text("text-slate-400 text-xs", "1. opacity 100 / 50 / 25"),
        Column(
            "gap-2",
            Column("w-full h-[26px] rounded-md bg-emerald-500"),
            Column("w-full h-[26px] rounded-md bg-emerald-500 opacity-50"),
            Column("w-full h-[26px] rounded-md bg-emerald-500 opacity-25"),
        ),
        Text("text-slate-400 text-xs", "2. group opacity — left half must be grey, not green"),
        Stack(
            "w-full h-[60px] opacity-50",
            Column("w-full h-[60px] rounded-lg bg-emerald-500"),
            Column("w-[140px] h-[60px] rounded-lg bg-white"),
        ),
        Text("text-slate-400 text-xs", "3. translate — top bar is the control and must not move"),
        Column(
            "gap-2",
            Column("w-[120px] h-[24px] rounded bg-sky-500"),
            Column("w-[120px] h-[24px] rounded bg-sky-500 translate-x-8"),
            Column(
                "w-[120px] h-[24px] rounded bg-amber-400 translate-x-4",
                Column("w-[28px] h-[24px] rounded bg-rose-600"),
            ),
        ),
        Text("text-slate-400 text-xs", "4. tap the translated box, then tap its old position"),
        # The live target: drawn 8 units (2rem) right of where layout put it. A tap
        # landing here proves hit-testing followed the transform.
        Button(
            "w-[120px] h-[44px] rounded-lg bg-indigo-500 text-white text-sm translate-x-8",
            f"HIT {s.hits}",
            on_tap=lambda: s.update(hits=s.hits + 1),
        ),
        # The control, deliberately *not* translated and sitting under the space the
        # button vacated. Tapping it proves taps in that column reach the app at all —
        # without which "the old position did nothing" would be indistinguishable from
        # "nothing was working".
        Button(
            "w-[120px] h-[36px] rounded bg-slate-700 text-slate-300 text-xs",
            f"MISS {s.misses}",
            on_tap=lambda: s.update(misses=s.misses + 1),
        ),
    )
