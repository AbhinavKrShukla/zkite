"""The three typeface families and a line-height, rendered natively (§9.4/§9.7).

Until this session the native renderer had no font-*family* resolution at all and
never applied `line-height` either: both parsed in twcore, inherited correctly, and
reached `TextStyle` — and then no shaping site read either one, so every text node
rendered in one hardcoded family at the font's own natural line height.

This app is the on-device counterpart of the conformance harness's `typography`
fixture. The three families are visually distinguishable from each other by design
(a proportional sans, a serif with visible brackets, and a fixed-pitch mono), so a
single screenshot answers "did family resolution reach real hardware" without
needing any measurement — the failure mode it guards against is three identical
lines.

`Hamburgefonstiv` is the conventional type-specimen string: it exercises ascenders,
descenders, round and square counters, and diagonals, which is where two typefaces
differ most visibly.

Selected at APK build time with `ZKITE_APP_MAIN=.../typography.py
scripts/build_apk.sh`. Remember `adb shell pm clear dev.zkite.gpuharness` before
reinstalling — asset unpacking is stamp-guarded (../../PLAN.md §7.9).
"""

from zkite import Column, Row, Text, store

SPECIMEN = "Hamburgefonstiv"


@store
class DemoState:
    n: int = 0


def view(s: DemoState):
    def spec(label, family_class):
        return Column(
            "gap-1 p-3 bg-slate-900 rounded-lg",
            Text("text-xs text-slate-500 font-mono", label),
            Text(f"text-2xl text-white {family_class}", SPECIMEN),
            Text(f"text-2xl font-bold text-white {family_class}", SPECIMEN),
        )

    return Column(
        "flex-1 bg-slate-950 gap-3 p-4 justify-center",
        spec("font-sans", "font-sans"),
        spec("font-serif", "font-serif"),
        spec("font-mono", "font-mono"),
        # leading-loose is 2x the font size; leading-none is 1x. Two lines of the
        # same string at the same size, differing only in the space around them,
        # is the cheapest visual check that line-height reached the shaper at all.
        Row(
            "gap-2",
            Column(
                "flex-1 bg-slate-900 rounded-lg p-2",
                Text("text-xs text-slate-500 font-mono", "leading-none"),
                Text("text-base text-white leading-none", "one\ntwo\nthree"),
            ),
            Column(
                "flex-1 bg-slate-900 rounded-lg p-2",
                Text("text-xs text-slate-500 font-mono", "leading-loose"),
                Text("text-base text-white leading-loose", "one\ntwo\nthree"),
            ),
        ),
    )
