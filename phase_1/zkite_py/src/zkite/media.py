"""Composable media controls; all returned nodes are ordinary ZKite widgets."""
from .widgets import Button, Column, Row, Slider, Text


def Controls(class_name="", *, player, state, parts=("time", "seek", "buttons", "volume", "speed"),
             button_class="px-4 py-2 rounded-xl bg-indigo-600 text-white",
             text_class="text-white", slider_class="w-full", row_class="gap-2",
             labels=None, skip_seconds=10, leading=None, trailing=None, on_close=None):
    """Render a state snapshot. Feed player.watch() into a store to update it.

    Customize classes, ordering, labels and slots, or build your own widget tree
    against the same playback handle. May be placed in an Overlay like any Column.
    """
    from .audio import _number
    _number(skip_seconds, 0, 3600, "skip_seconds")
    if "fullscreen" in parts and not hasattr(player, "fullscreen"):
        raise ValueError("fullscreen controls require a video player")
    labels = labels or {}
    position, duration = state.get("position", 0), state.get("duration") or 0
    playing = state.get("state") == "playing"
    def button(name, fallback, callback):
        return Button(button_class, labels.get(name, fallback), on_tap=callback)
    def clock(value):
        value = max(0, int(value))
        return f"{value // 60}:{value % 60:02}"
    async def close():
        import inspect
        await player.stop()
        if on_close:
            result = on_close()
            if inspect.isawaitable(result): await result
    if state.get("state") in ("closed", "error"):
        return Column(class_name, leading, Text(text_class, state.get("reason") or state["state"]), button("close", "Close", close), trailing)
    builders = {
        "time": lambda: Text(text_class, f"{clock(position)} / {clock(duration)}"),
        "seek": lambda: Slider(slider_class, min=0, max=max(1, duration), step=.1, value=position, on_change=player.seek),
        "buttons": lambda: Row(row_class,
            button("back", f"−{skip_seconds}s", lambda: player.seek(max(0, position-skip_seconds))),
            button("pause" if playing else "play", "Pause" if playing else "Play", player.pause if playing else player.resume),
            button("forward", f"+{skip_seconds}s", lambda: player.seek(min(duration, position+skip_seconds))),
            button("close", "Close", close)),
        "volume": lambda: Slider(slider_class, min=0, max=1, step=.01, value=state.get("volume", 1), on_change=lambda v: player.configure(volume=v)),
        "speed": lambda: Row(row_class, *[button(str(speed), f"{speed}×", lambda speed=speed: player.configure(speed=speed)) for speed in (.5, 1, 1.5, 2)]),
        "loop": lambda: button("loop", "Loop: " + ("on" if state.get("loop") else "off"), lambda: player.configure(loop=not state.get("loop", False))),
        "fullscreen": lambda: button("fullscreen", "Exit fullscreen" if state.get("fullscreen") else "Fullscreen", lambda: player.fullscreen(not state.get("fullscreen", False))),
        "status": lambda: Text(text_class, state.get("reason") or state.get("state", "")),
    }
    unknown = set(parts) - builders.keys()
    if unknown:
        raise ValueError(f"Unknown media control parts: {sorted(unknown)}")
    return Column(class_name, leading, *[builders[part]() for part in parts], trailing)
