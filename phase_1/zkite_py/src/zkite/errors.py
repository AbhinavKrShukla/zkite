"""On-device error overlay (plan1.md §6: "a Python traceback renders as an overlay on
the device with the failing frame and source line ... simultaneously in the terminal").
"""

from __future__ import annotations

import html
import sys

try:
    from ._build_mode import DEBUG
except ImportError:
    DEBUG = True

_RELEASE_ERROR = "Something went wrong. Please restart the app."

#: Prefix on the line `log_error`/`log_recovered` write. Grepped by any device gate
#: that wants to know whether an app is actually working — see `log_error`.
MARKER = "ZKITEERROR"


def format_error_overlay(traceback_text: str) -> str:
    escaped = html.escape(traceback_text if DEBUG else _RELEASE_ERROR)
    return (
        '<div id="__zkite_error_overlay" '
        'style="position:fixed;inset:0;background:rgba(20,0,0,.92);color:#ff8080;'
        'font-family:ui-monospace,monospace;font-size:13px;line-height:1.5;'
        'white-space:pre-wrap;padding:16px;overflow:auto;z-index:2147483647;">'
        f"{escaped}"
        "</div>"
    )


def format_error_tree(traceback_text: str) -> dict:
    """Native (phase 3) counterpart of `format_error_overlay`: a full-screen `div`
    using `twcore`'s `absolute inset-0` (there's no DOM/CSS `position:fixed`-relative-
    to-viewport to reach for), wrapping the traceback. See `zkite.app.App.render_tree`
    and phase_3/PLAN.md §5.

    **The traceback is split into one `#text` child per source line** (phase_3/PLAN.md
    §31.3). It used to be a single `#text` carrying the whole blob, which renders as
    one very long line running off the right edge of the phone — the HTML twin above
    sets `white-space:pre-wrap`, so only the native rendering was unreadable, and it
    is unreadable on the one screen a user is looking at *because* something already
    went wrong. Native text does wrap (parley's `break_all_lines` gets a `max_advance`
    whenever the box has a definite width, which `inset-0` gives it), but it wraps at
    word boundaries within a single run — and a traceback's own newlines are not
    boundaries the shaper honours, so the whole blob shaped as one continuous line.

    `flex flex-col` because a `div`'s children would otherwise lay out along taffy's
    default row axis; `break-all`-style mid-word breaking is not available in `twcore`,
    so a single very long path still overflows its own line. That is a narrower gap
    than the original and is recorded in §31.6 rather than fixed here.
    """
    lines = (traceback_text if DEBUG else _RELEASE_ERROR).split("\n")
    # A trailing newline would otherwise contribute an empty final line, which shapes
    # to a zero-run text node the paint path skips anyway — dropped here so the tree
    # says what it means.
    if lines and lines[-1] == "":
        lines.pop()
    return {
        "tag": "div",
        "id": "__zkite_error_overlay",
        "classes": "absolute inset-0 flex flex-col bg-black p-4 text-red-400 text-xs",
        "events": [],
        "attrs": {},
        "children": [{"tag": "#text", "text": line} for line in lines],
    }


def log_error(traceback_text: str) -> None:
    """Write a failing `view()`'s traceback to stderr, so it reaches a log as well as
    the screen (phase_3/PLAN.md §19.3, owed as NEXT_SESSION.md item 1d).

    **Why this is not redundant with the overlay.** Until this existed, an app whose
    `view()` raised wrote *nothing* to logcat on any tag — the traceback went only to
    `format_error_tree`/`format_error_overlay` and onto the screen, and `zkite dev`'s
    reload socket was the sole other channel. §46.8.1 measured the consequence: its
    27-fixture device sweep gates each app on `FATAL EXCEPTION`/`E AndroidRuntime`
    being zero, and `demo_app/error_repro.py` — an app that is broken by construction
    — passed that gate looking healthy, with `fatal=0` and `nodes=14`. **A
    logcat-based gate could not tell a working app from one displaying a traceback**,
    which is exactly the shape of check plan1.md §14.5's absent CI has to be built
    from. One line closes that.

    stderr rather than a logging framework because stderr is the channel every shell
    already carries: `phase_1/devshell/src/runtime.py` tees it into a buffer that
    `zkite_pybridge::embed::drain_output` flushes to `__android_log_write` on every
    glue call, and Android drops a bare `write(2, ...)` from an app process on the
    floor without it. So this reaches logcat on **both** backends and the host with no
    new plumbing on any of them.

    The caller is responsible for only calling this when the error is *new* — see
    `zkite.app.App.render_tree`. A `view()` that raises does so on every frame, and at
    60fps an unguarded write is 60 tracebacks a second.
    """
    print(f"{MARKER} view() raised:", file=sys.stderr)
    for line in traceback_text.rstrip("\n").split("\n"):
        print(f"{MARKER}   {line}", file=sys.stderr)


def log_recovered() -> None:
    """Counterpart of `log_error`: the app rendered cleanly after having failed.

    Without this a gate can see that an app *broke* and never that it recovered, so a
    hot reload back to a working `view()` would leave the last word in the log saying
    the app is broken. §19.3's third bullet names that round trip as untested.
    """
    print(f"{MARKER} cleared: view() rendered without raising", file=sys.stderr)


__all__ = ["MARKER", "format_error_overlay", "format_error_tree", "log_error", "log_recovered"]


class DeviceError(Exception):
    """An Android device operation failed; safe to handle in application UI."""


class DeviceUnavailable(DeviceError):
    pass


class PermissionDenied(DeviceError):
    pass


class PermissionNotDeclared(DeviceError):
    pass


class UnsupportedFeature(DeviceError):
    pass


class ResourceBusy(DeviceError):
    pass


class OperationInterrupted(DeviceError):
    pass


class DeviceIOError(DeviceError):
    pass


class RuntimeMismatch(DeviceError):
    pass
