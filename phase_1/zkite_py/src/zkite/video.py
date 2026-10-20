"""Native foreground video surface; compose Python controls beside its bounds."""
from . import _device
from .audio import Playback, _number, _source_uri


def _bounds(value):
    if len(value) != 4:
        raise ValueError("bounds must be (x, y, width, height) in Android dp")
    for i, number in enumerate(value):
        _number(number, 0 if i < 2 else 1, 100000, "bounds")
    return list(value)


class VideoPlayback(Playback):
    async def fullscreen(self, enabled=True):
        """Toggle immersive fullscreen, restoring previous bounds/bars on exit."""
        self._check()
        await _device.request("video.fullscreen", handle=self.id, enabled=bool(enabled))

    async def bounds(self, x, y, width, height):
        """Move/resize the surface in content-relative dp (also exits fullscreen)."""
        self._check()
        await _device.request("video.bounds", handle=self.id, bounds=_bounds((x, y, width, height)))


async def play(source, *, volume=1.0, loop=False, controls=True, bounds=None, fit="contain", control_style=None):
    """Play a local file, FileRef, content URI or HTTP(S) URL.

    None bounds fills the Activity. Native controls appear on tap. For a custom
    interface use controls=False, bounded video and ordinary ZKite widgets outside
    that rectangle. Android surfaces sit above the Python renderer.
    """
    _number(volume, 0, 1, "volume")
    if fit not in ("contain", "cover", "fill"):
        raise ValueError("fit must be contain, cover or fill")
    args = dict(uri=_source_uri(source), volume=volume, loop=bool(loop), controls=bool(controls), fit=fit)
    if control_style is not None:
        args["control_style"] = _control_style(control_style)
    if bounds is not None:
        args["bounds"] = _bounds(bounds)
    return VideoPlayback(await _device.request("video.play", **args))


def _control_style(style):
    import re
    allowed = {"foreground", "background", "accent", "text_size",
               "padding", "placement", "auto_hide_ms", "skip_seconds", "show_time",
               "show_seek", "show_skip", "show_mute", "show_speed", "show_close", "show_fullscreen"}
    if not isinstance(style, dict) or set(style) - allowed:
        raise ValueError("Unknown control_style setting")
    for key, value in style.items():
        if key in {"foreground", "background", "accent"}:
            if not isinstance(value, str) or not re.fullmatch(r"#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?", value):
                raise ValueError("Colors must be #RRGGBB or #AARRGGBB")
        elif key.startswith("show_"):
            if not isinstance(value, bool): raise ValueError(f"{key} must be boolean")
        elif key == "placement":
            if value not in ("top", "bottom"): raise ValueError("placement must be top or bottom")
        else:
            limits = {"text_size": (8, 72), "padding": (0, 100), "auto_hide_ms": (0, 60000), "skip_seconds": (1, 3600)}
            _number(value, *limits[key], key)
    return dict(style)
