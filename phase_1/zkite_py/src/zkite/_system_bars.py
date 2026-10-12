"""Native Android system-bar styling, declared with the current screen."""
from __future__ import annotations

import re

from .widgets import RenderContext, Widget


def _color(value: str) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})", value):
        raise ValueError("system bar colors must be #RGB or #RRGGBB strings")
    if len(value) == 4:
        value = "#" + "".join(c * 2 for c in value[1:])
    return value.lower()


def _dark_icons(color: str, icons: str) -> bool:
    if icons not in ("auto", "light", "dark"):
        raise ValueError("system bar icons must be 'auto', 'light' or 'dark'")
    if icons != "auto":
        return icons == "dark"
    rgb = [int(color[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    linear = [v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4 for v in rgb]
    luminance = sum(v * weight for v, weight in zip(linear, (0.2126, 0.7152, 0.0722)))
    # Pick the greater contrast ratio against black or white icons.
    return (luminance + 0.05) / 0.05 >= 1.05 / (luminance + 0.05)


class SystemBars(Widget):
    """Wrap a screen without changing its layout or event IDs.

    ``SystemBars(screen, status_color="#020617", navigation_color="#020617")``
    paints behind Android's bars and chooses contrasting icons. Explicit icon
    choices are ``auto``, ``light`` and ``dark``. Settings follow store updates and
    hot reload. Native Android only; the WebView backend renders the child normally.
    """

    def __init__(self, child: Widget, *, status_color: str = "#ffffff",
                 navigation_color: str | None = None, status_icons: str = "auto",
                 navigation_icons: str = "auto"):
        if not isinstance(child, Widget):
            raise TypeError("SystemBars expects one screen widget")
        super().__init__()
        self.child = child
        status = _color(status_color)
        navigation = _color(navigation_color if navigation_color is not None else status)
        self.style = {
            "status_color": status,
            "navigation_color": navigation,
            "status_dark_icons": _dark_icons(status, status_icons),
            "navigation_dark_icons": _dark_icons(navigation, navigation_icons),
        }

    def render(self, ctx: RenderContext, path: str) -> str:
        return self.child.render(ctx, path)

    def to_tree(self, ctx: RenderContext, path: str) -> dict:
        ctx.system_bars = self.style.copy()
        return self.child.to_tree(ctx, path)
