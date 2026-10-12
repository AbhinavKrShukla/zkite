"""§47 — the animation driver, end to end. The device fixture for Queue F part 2.

Every element here exists to make one claim readable off a screenshot band, because
`screencap` cannot be timed precisely enough to check "what value at what millisecond"
(§45.4's method: read bands, and always read one that must *not* change).

    row            what it proves                                   how it is read
    ------------   ----------------------------------------------   -----------------
    SLIDE          a translate animates, and drags its subtree       the emerald bar's
                   with it                                          left edge moves
    FADE           an opacity animates                              the sky bar's
                                                                     colour lightens
    CONTROL        neither of the above touched anything else       band is identical
                                                                     in every shot
    done=N         `on_animend` reached Python — exactly once per   the digit
                   run, not once per frame
    TAP ME         a `change` spring follows a moving target, and   the amber bar
                   a translated node is tapped where it is drawn     moves right
    EXIT ME        an exit animation completes and *then* Python    the rose bar
                   drops the node                                    disappears

**Both entrance animations are 6000ms and `linear` on purpose.** A real entrance is
200-300ms, which is over before `adb shell screencap` has finished writing a PNG. Six
seconds of a straight line means any two samples are ordered, the value between them is
checkable arithmetic, and "it animated" is distinguishable from "it snapped to the end
on frame one" — which is the failure a short animation and a slow camera cannot tell
apart.

`done` is the load-bearing number. It is written by `on_animend`, so a non-zero value
is proof a completion crossed back into Python; and it must settle at exactly **2**
(one per entrance) rather than climbing, because reporting a completion once per frame
instead of once per run is the natural bug and a counter is the only thing that can see
the difference.

**Geometry is chosen so nothing leaves the screen.** The surface is 1080x2223 device px
at density 2.5, i.e. **432dp** wide; a 200dp bar centred starts at 116dp, and a 100dp
translate ends its right edge at 416dp — inside the display, so the bar's own edges stay
measurable rather than being clipped.

No `opacity` is declared on any node that spans the screen. §46.6 measured a full-screen
opacity group at **1.88ms/frame**, and the FADE bar is deliberately a 200x56dp box so
this fixture costs a layer over 2% of the surface rather than over all of it.
"""

from zkite import Anim, Button, Column, Row, Text, store

ENTER_MS = 6000
#: The exit is long for the same reason the entrances are (see the module doc): a
#: realistic 200-300ms exit is over before `screencap` has written a PNG, which makes
#: "it animated out, then Python dropped it" indistinguishable from "it vanished the
#: instant it was tapped" — and those are two very different bugs.
EXIT_MS = 4000
TRAVEL = 100.0


@store
class AnimDemo:
    #: Bumped by each entrance animation's completion. Must settle at 2.
    done: int = 0
    #: Which side the `change` spring currently targets. Flipped by tapping the bar.
    shifted: bool = False
    #: Set by tapping the rose bar. The row then keeps rendering *with* an exit
    #: declaration until its completion arrives, and only then is it dropped — the
    #: renderer cannot animate a node out of a tree it has already left, so Python
    #: holds it. That ordering is the contract (§45.2).
    leaving: bool = False
    gone: bool = False


def _bar(colour: str, label: str, **kw):
    return Row(f"h-[56px] w-[200px] items-center pl-4 {colour}", Text("text-white text-lg", label), **kw)


def view(s: AnimDemo):
    rows = [
        Text("text-slate-400 text-sm", f"done={s.done} shifted={int(s.shifted)} gone={int(s.gone)}"),
        # A translate moves the node *and its subtree*, so the label rides along —
        # which a paint-time rect offset would not do (§45.3). `on_animend` sits on
        # the same node as `anim=`, because that is the node the completion is
        # dispatched to.
        _bar(
            "bg-emerald-500",
            "SLIDE",
            anim=Anim.enter(translate_x=(0, TRAVEL), ms=ENTER_MS, ease="linear"),
            on_animend=lambda e: s.update(done=s.done + 1),
        ),
        _bar(
            "bg-sky-500",
            "FADE",
            anim=Anim.enter(opacity=(0.1, 1.0), ms=ENTER_MS, ease="linear"),
            on_animend=lambda e: s.update(done=s.done + 1),
        ),
        # No declaration at all. Every screenshot must show this band byte-identical;
        # without it, a bar that moved would prove only that *something* changed.
        _bar("bg-slate-600", "CONTROL"),
        # `change` re-runs whenever the target moves, carrying position *and* velocity
        # across, so tapping twice in quick succession is continuous rather than a
        # jump. It is also the tap test: the bar is translated, and it must respond
        # where it is drawn.
        Button(
            "h-[56px] w-[200px] items-center justify-start pl-4 bg-amber-500",
            Text("text-white text-lg", "TAP ME"),
            on_tap=lambda: s.update(shifted=not s.shifted),
            anim=Anim.change(translate_x=TRAVEL if s.shifted else 0.0, spring=(180, 26)),
        ),
    ]

    if not s.gone:
        rows.append(
            _bar(
                "bg-rose-500",
                "LEAVING" if s.leaving else "EXIT ME",
                anim=Anim.exit_(opacity=(1.0, 0.0), translate_x=(0.0, -160.0), ms=EXIT_MS) if s.leaving else None,
                on_animend=lambda e: s.update(gone=True),
                on_tap=lambda: s.update(leaving=True),
            )
        )

    return Column("flex-1 items-center justify-center gap-3 bg-slate-950", *rows)
