"""§38 — gradients, on the device, through the classes WIDGETS.md documents.

Written against the class strings a user would type, not against `twcore`'s internals,
for the reason §26.7 records. Everything on this screen is measurable from a single
screenshot with `phase_3/conformance/pngtool.py`, and every band is *labelled by
position* so a wrong direction cannot be mistaken for a wrong colour:

1. **Eight directions**, two rows of four, `from-yellow-400 to-blue-600` throughout —
   the pair whose oklab-vs-sRGB midpoint differs by 81/255 in blue (§38.1), so this
   band is also the on-device check of the interpolation space. The last box uses v3's
   `bg-gradient-to-bl` spelling, which is still live in v4.3.3.
2. **The stop model**: `via-*`, explicit `from-25%`/`to-75%` positions, a one-stop fade
   to transparent, and that same fade over a `bg-red-500` — which is the band that shows
   `background-color` and `background-image` are two properties, not one.
3. **The gamut band**: `from-cyan-500 to-blue-500`. cyan-500's red channel is
   **-0.230** in encoded sRGB, i.e. outside the gamut, and §38.3 is the finding that a
   gradient stop must carry that rather than the clipped palette hex. On a device this
   is the band that would look wrong first if the `ExtColor` column were lost.
4. **A pill and a rounded card**, because a gradient has to be clipped by its own
   `rounded-*` corners like any other background — a fill that ignores the radius is a
   different bug from a fill with the wrong ramp, and this band separates them.

The header counter is tapped by the device script to prove the app is live rather than
a stale frame (§34.7's control), and it is `tabular-nums` so the digits do not reflow.
"""

from zkite import Button, Column, Row, Text, store

PAIR = "from-yellow-400 to-blue-600"


@store
class DemoState:
    taps: int = 0


def _box(cls: str):
    return Column(f"w-[76px] h-[56px] rounded-sm {cls}")


def view(s: DemoState):
    return Column(
        "flex-1 bg-slate-950 p-3 gap-3 min-h-0",
        Row(
            "shrink-0 items-center justify-between",
            Text("text-white text-lg font-bold", "gradients"),
            # A `Button`, not a `Text` with `on_tap` — WIDGETS.md's table gives `Text` no
            # events at all, and the first draft of this file used `Text(..., on_tap=)`,
            # which is accepted as an unknown prop and silently does nothing. That is
            # §26.7's rule (write it against the documented signature) failing in the
            # direction the rule exists for, caught by the liveness control not firing.
            Button(
                "text-emerald-400 text-lg font-bold tabular-nums",
                f"taps {s.taps}",
                on_tap=lambda: s.set(taps=s.taps + 1),
            ),
        ),
        # 1. the eight directions
        Row("shrink-0 gap-2", *[_box(f"bg-linear-to-{d} {PAIR}") for d in ("t", "r", "b", "l")]),
        Row(
            "shrink-0 gap-2",
            _box(f"bg-linear-to-tr {PAIR}"),
            _box(f"bg-linear-to-tl {PAIR}"),
            _box(f"bg-linear-to-br {PAIR}"),
            _box(f"bg-gradient-to-bl {PAIR}"),
        ),
        # 2. the stop model
        Column(f"shrink-0 w-full h-[36px] rounded-sm bg-linear-to-r from-yellow-400 via-fuchsia-500 to-blue-600"),
        Column(f"shrink-0 w-full h-[36px] rounded-sm bg-linear-to-r from-yellow-400 from-25% to-blue-600 to-75%"),
        Column("shrink-0 w-full h-[36px] rounded-sm bg-linear-to-r from-yellow-400"),
        Column("shrink-0 w-full h-[36px] rounded-sm bg-red-500 bg-linear-to-r from-yellow-400"),
        # 3. the gamut band
        Column("shrink-0 w-full h-[36px] rounded-sm bg-linear-to-r from-cyan-500 to-blue-500"),
        # 4. the radius has to clip the ramp
        Row(
            "shrink-0 gap-3 items-center",
            Column("w-[160px] h-[44px] rounded-full bg-linear-to-r from-fuchsia-500 to-cyan-400"),
            Column("w-[120px] h-[44px] rounded-2xl bg-linear-to-b from-white to-slate-700"),
        ),
        # An angle, and the same angle written as an arbitrary value — they must agree.
        Row(
            "shrink-0 gap-3",
            Column(f"w-[150px] h-[40px] rounded-sm bg-linear-45 {PAIR}"),
            Column(f"w-[150px] h-[40px] rounded-sm bg-linear-[45deg] {PAIR}"),
        ),
    )
