"""Deliberately-broken app for verifying the native error overlay (`zkite.errors
.format_error_tree`, wired through `App.render_tree`/`_boot_app_tree` in runtime.py).
`view()` raises on every call, so the very first render exercises the overlay path
with no interaction needed.
"""

from zkite import Column, Text, store


@store
class State:
    n: int = 0


def view(s: State):
    return Column(
        "flex-1 items-center justify-center",
        Text("text-white", f"count is {s.n}"),
        1 / 0,  # noqa: B018 - deliberate crash for the error-overlay repro
    )
