"""The author-facing animation surface — phase_3/PLAN.md §47, Queue F layer 3.

An animation is **declared on a widget**, not commanded:

    Column("flex-1 items-center justify-center",
        Text("text-2xl", "Hello"),
        anim=Anim.enter(opacity=(0, 1), translate_y=(16, 0), ms=220),
    )

The declaration crosses to the renderer as one `anim` attribute, is started by the
render thread's timeline, and is sampled per frame **with no Python in the loop**.
That last part is the constraint the whole design is built around, and it was priced
rather than assumed (phase_3/PLAN.md §45.2): a frame that re-runs `view()` costs
~16.03ms of a 16.67ms budget, and `view()`'s cost scales with the *whole screen*, so
fading one 48dp icon from Python would cost what rebuilding a 25-row list costs.

# Why explicit channels, and not `transition-*` classes

The owner's decision (§45.3 principle 2). Tailwind's `transition-*` would have been
the smaller-looking API and would have bought CSS's semantics with it — its
interruption behaviour, its property-change triggers, its `transitionend` timing —
which is a new silent divergence between the two backends acquired for a feature meant
to remove one. §32.1 rejected HTML's `<label>` on exactly this argument, with a
measurement behind it.

# The two channels, and why one of them is not free

`opacity` and `translate_x`/`translate_y`. Measured on a Moto G54 (§46.6): one
`opacity` group spanning the full screen costs **1.88ms/frame**, 16% of the frame's
total work, because compositing a subtree at partial alpha needs an offscreen render
target. A translate opens nothing at all — it reuses the per-node offset channel the
renderer already had for scrolling.

So: a 200–300ms fade costs ~28ms spread over ~15 frames and is entirely affordable; a
*persistent* full-screen opacity group costs 1.9ms/frame forever against a frame with
~5ms of slack. `translate_*` is the right default for anything long-lived, and
[`Anim`] warns when a declaration looks like the expensive case.

`rotate` and `scale` are **not** here. They parse in `twcore` and work on the WebView
backend, and the native paint path drops them (§45.7) — so animating one would run,
complete, fire its callback and never move a pixel. That is §15.1's silent-divergence
shape exactly, and refusing is the whole reason this is an explicit channel list.
"""

from __future__ import annotations

import json
import sys
from typing import Any, Iterable

#: The channels the native renderer can actually paint. Kept in lockstep with
#: `zkite_renderer::Channel` — that enum is the authority and rejects anything else,
#: so this list exists to give the author a Python-level error at the point they
#: wrote the typo rather than a log line from the render thread.
CHANNELS = ("opacity", "translate_x", "translate_y")

#: Channels that are parsed by `twcore`, painted by the WebView backend, and dropped
#: by the native one (§45.7). Named separately so the error can say *why*.
UNPAINTED_CHANNELS = ("rotate", "scale", "scale_x", "scale_y")

_EASINGS = ("linear", "ease", "in", "out", "in_out")
_DIRECTIONS = ("normal", "reverse", "alternate", "alternate_reverse")
_TRIGGERS = ("enter", "change", "exit")

_warned_html_backend = False


class AnimEnd:
    """The payload an `on_animend` handler receives.

    A small object rather than a bare string because a handler almost always has to
    branch on one of the two fields, and `payload == "exit"` versus
    `payload["trigger"] == "exit"` is exactly the guessing game §26.2's string payload
    caused three separate times.
    """

    __slots__ = ("channel", "trigger")

    def __init__(self, channel: str, trigger: str) -> None:
        self.channel = channel
        self.trigger = trigger

    def __repr__(self) -> str:  # pragma: no cover - diagnostic only
        return f"AnimEnd(channel={self.channel!r}, trigger={self.trigger!r})"

    def __eq__(self, other: Any) -> bool:
        return (
            isinstance(other, AnimEnd)
            and other.channel == self.channel
            and other.trigger == self.trigger
        )


def parse_anim_end(payload: Any) -> AnimEnd:
    """Type an `animend` payload at the widget, the way `Switch`/`Slider` type theirs.

    Every backend's event bridge is string-in, string-out (see
    `RenderContext.register`), so without this a handler would receive the raw JSON
    text `'{"channel":"opacity","trigger":"exit"}'`. §26.2 is the reason this is not
    left to the caller: the same "the payload is a string" gap bit `Switch`, `Slider`
    and `Select` in turn, and each time the fix was to type it here.
    """
    if isinstance(payload, AnimEnd):
        return payload
    try:
        data = json.loads(payload) if isinstance(payload, str) else dict(payload or {})
    except (ValueError, TypeError):
        data = {}
    return AnimEnd(str(data.get("channel", "")), str(data.get("trigger", "")))


class Anim:
    """One trigger's worth of animation, over one or more channels.

    Build one with [`enter`](Anim.enter), [`change`](Anim.change) or
    [`exit_`](Anim.exit_) rather than calling this directly — the trigger is the thing
    an author is choosing between, so it belongs in the name.

    A channel is given either as a target (`opacity=1`) or as a `(from, to)` pair
    (`opacity=(0, 1)`). An unstated `from` means **"wherever this node's class string
    already puts it"**, resolved on the renderer side, because that value depends on
    `twcore`, on style inheritance, and — for `1em`-relative sizes — on ancestry that
    Python cannot see (§18.4). Defaulting it to `0` here would make every unstated
    fade start from fully transparent.
    """

    __slots__ = ("trigger", "channels", "ms", "ease", "spring", "delay", "repeat", "dir")

    def __init__(
        self,
        trigger: str,
        *,
        ms: float | None = None,
        ease: str | tuple[float, float, float, float] | list[float] | None = None,
        spring: tuple[float, float] | list[float] | None = None,
        delay: float = 0.0,
        repeat: int | str | None = None,
        dir: str | None = None,
        **channels: Any,
    ) -> None:
        if trigger not in _TRIGGERS:
            raise ValueError(f"unknown animation trigger {trigger!r}; use one of {', '.join(_TRIGGERS)}")
        if not channels:
            raise ValueError(
                f"Anim.{_ctor_name(trigger)}() declares no channels; "
                f"pass at least one of {', '.join(CHANNELS)}"
            )
        for name in channels:
            if name in UNPAINTED_CHANNELS:
                raise ValueError(
                    f"{name!r} is parsed by twcore and painted by nothing on the native "
                    f"renderer (phase_3/PLAN.md §45.7), so animating it would run, finish "
                    f"and move no pixels. Animatable channels: {', '.join(CHANNELS)}"
                )
            if name not in CHANNELS:
                raise ValueError(
                    f"unknown animation channel {name!r}; use one of {', '.join(CHANNELS)}"
                )
        # Both or neither is an error rather than a default, because an eased curve and
        # a spring are genuinely different animations and picking one silently is how a
        # framework acquires a surprising default nobody can find later.
        if (ms is None) == (spring is None):
            raise ValueError(
                "an animation needs exactly one of ms=<duration> or spring=(stiffness, damping)"
            )
        if spring is not None and ease is not None:
            raise ValueError("a spring has no easing — ease= and spring= are exclusive")
        if isinstance(ease, str) and ease not in _EASINGS:
            raise ValueError(f"unknown easing {ease!r}; use one of {', '.join(_EASINGS)}, or [x1,y1,x2,y2]")
        if dir is not None and dir not in _DIRECTIONS:
            raise ValueError(f"unknown dir {dir!r}; use one of {', '.join(_DIRECTIONS)}")
        if isinstance(repeat, str) and repeat != "forever":
            raise ValueError(f"unknown repeat {repeat!r}; use a count or \"forever\"")
        if trigger == "exit" and repeat == "forever":
            raise ValueError(
                "an exit animation cannot repeat forever — its completion is what tells "
                "your app the node can be removed, and a forever animation never completes"
            )

        self.trigger = trigger
        # Normalised here, not in `to_wire`, so that a malformed value — a three-long
        # tuple, a string where a number belongs — raises at the call site the author
        # typed rather than on the first render of a screen they may not be looking
        # at. `to_wire` is then pure formatting, which is the point of doing it once.
        self.channels = {name: _from_to(name, value) for name, value in channels.items()}
        self.ms = ms
        self.ease = ease
        self.spring = spring
        self.delay = delay
        self.repeat = repeat
        self.dir = dir

    @classmethod
    def enter(cls, **kw: Any) -> "Anim":
        """Run once, when this node first appears with this declaration on it.

        Idempotent by construction: the tree is rebuilt on every frame of a scroll, so
        this declaration arrives ~60 times a second and the driver starts it once.
        """
        return cls("enter", **kw)

    @classmethod
    def change(cls, **kw: Any) -> "Anim":
        """Follow a moving target — re-runs whenever the target value changes.

        Interrupting carries the current position across (and, for a spring, the
        current *velocity*), so the value never jumps. A control the user can grab
        mid-flight should therefore be a `spring=`.
        """
        return cls("change", **kw)

    @classmethod
    def exit_(cls, **kw: Any) -> "Anim":
        """Run once, and tell the app when it is done so the node can be removed.

        The renderer cannot animate a node out of a tree it is no longer in — the tree
        is authoritative, so a removed node is simply gone. The contract is therefore
        the other way round: **your `view()` keeps rendering the node** with this
        declaration on it, and drops it when `on_animend` fires. That is one Python
        call per animation rather than one per frame (§45.2).

        Named `exit_` because `Anim.exit` would shadow nothing but reads as the
        builtin at a call site.
        """
        return cls("exit", **kw)

    def to_wire(self) -> list[dict]:
        """This declaration as the renderer's `anim` attribute — one entry per channel.

        Flattened per channel rather than sent as one grouped record because the
        driver's unit of work is `(node, channel)`: two channels on one node can be
        retargeted, completed and evicted independently, and a grouped wire format
        would have to be un-grouped on arrival anyway.
        """
        out = []
        for name, (frm, to) in self.channels.items():
            entry: dict[str, Any] = {"ch": name}
            entry["to"] = to
            if frm is not None:
                entry["from"] = frm
            if self.ms is not None:
                entry["ms"] = float(self.ms)
                if self.ease is not None:
                    entry["ease"] = self.ease if isinstance(self.ease, str) else [float(v) for v in self.ease]
            else:
                assert self.spring is not None  # guaranteed by __init__
                entry["spring"] = [float(v) for v in self.spring]
            if self.delay:
                entry["delay"] = float(self.delay)
            if self.repeat is not None:
                entry["repeat"] = self.repeat
            if self.dir is not None:
                entry["dir"] = self.dir
            # `on` is omitted for the default so the common case sends fewer bytes —
            # this attribute rides inside every rebuilt tree, and §43 measured what
            # tree JSON costs on a rebuild frame.
            if self.trigger != "enter":
                entry["on"] = self.trigger
            out.append(entry)
        return out

    def opens_a_layer(self) -> bool:
        """Whether this declaration animates `opacity`, which costs an offscreen
        render target for the node's whole subtree — §46.6 measured 1.88ms/frame for
        one spanning the full screen."""
        return "opacity" in self.channels


def _ctor_name(trigger: str) -> str:
    return "exit_" if trigger == "exit" else trigger


def _from_to(name: str, value: Any) -> tuple[float | None, float]:
    """`opacity=1` -> (None, 1.0); `opacity=(0, 1)` -> (0.0, 1.0)."""
    if isinstance(value, (tuple, list)):
        if len(value) != 2:
            raise ValueError(
                f"channel {name!r} takes a target or a (from, to) pair, got {len(value)} values"
            )
        return float(value[0]), float(value[1])
    return None, float(value)


def to_wire(anim: Any) -> list[dict]:
    """Normalise whatever an author passed as `anim=` into the wire list.

    Accepts one `Anim`, a sequence of them, or `None`. A sequence is the multi-trigger
    case — `[Anim.enter(...), Anim.exit_(...)]` on one node is how a sheet declares
    both halves of its transition.
    """
    if anim is None:
        return []
    if isinstance(anim, Anim):
        return anim.to_wire()
    if isinstance(anim, Iterable) and not isinstance(anim, (str, bytes, dict)):
        out: list[dict] = []
        for item in anim:
            if not isinstance(item, Anim):
                raise TypeError(f"anim= takes Anim objects, got {type(item).__name__}")
            out.extend(item.to_wire())
        return out
    raise TypeError(f"anim= takes an Anim or a list of them, got {type(anim).__name__}")


def warn_html_backend_once() -> None:
    """Say, exactly once per process, that animations do not run on this backend.

    **This is the honest state of Queue F after part 2, made loud instead of silent.**
    The driver is native-only; the WebView port is F3 (§45.3's layer 4) and does not
    exist, so a declaration renders a static node there and animates on native. That is
    a §15.1 divergence, and this document's own record is that the dangerous ones are
    the *quiet* ones — every entry in that section is something that worked on one
    backend and did nothing on the other with no error anywhere.

    Written to stderr, which every shell already tees to logcat and which
    `zkite.errors.log_error` uses for the same reason (§19.3). Once per process rather
    than per render, because `view()` runs sixty times a second.
    """
    global _warned_html_backend
    if _warned_html_backend:
        return
    _warned_html_backend = True
    print(
        "ZKITEANIM this app declares an animation and is running on the WebView backend, "
        "where the animation driver does not exist yet (phase_3/PLAN.md §47, layer 4). "
        "The node renders in its final state and no on_animend will fire. "
        "Run with --backend native to see it animate.",
        file=sys.stderr,
    )


def _reset_html_warning_for_tests() -> None:
    global _warned_html_backend
    _warned_html_backend = False


__all__ = ["Anim", "AnimEnd", "CHANNELS", "parse_anim_end", "to_wire", "warn_html_backend_once"]
