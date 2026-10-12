"""The app `e2e.mjs` drives for its `Select` checks (phase_3/PLAN.md §33).

A real module rendered by a real `App`, so the HTML that reaches Chromium is whatever
the widget emits today rather than a transcription that can go stale. Written against
the documented signature only, per §26.7.
"""

from zkite import Column, Select, Text, store


@store
class S:
    country: str = "in"


def view(s: S):
    return Column(
        "flex-1 bg-slate-950 p-6 gap-4",
        Text("text-white", f"Country: {s.country}"),
        Select(
            "px-4 py-3 rounded-xl bg-slate-800 text-white",
            options=[("br", "Brazil"), ("in", "India"), ("ke", "Kenya")],
            value=s.country,
            on_change=lambda v: s.update(country=v),
        ),
    )
