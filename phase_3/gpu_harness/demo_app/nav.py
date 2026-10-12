"""Navigation demo — three screens, a real back stack, per-screen state.

phase_3/PLAN.md §16. Built for device verification, so every claim it makes is
readable off a screenshot without a log line:

  - each screen's background is a *different* colour, so which screen is on top is
    unambiguous in a screenshot even if the text is unreadable;
  - the home counter is shown on the home screen and nowhere else, so "state
    survived a push and a pop" is checkable by eye;
  - the detail screen takes a pushed param, so param passing is exercised rather
    than assumed;
  - the depth is rendered, so a stack that silently grew (a double-push from one
    tap) or silently emptied is visible.

Run: ZKITE_APP_MAIN=demo_app/nav.py ANDROID_HOME=~/Android/Sdk ABI=arm64-v8a \
     phase_3/gpu_harness/scripts/build_apk.sh
"""

from zkite import Button, Column, Nav, Navigator, Row, Text, store


@store
class Counter:
    n: int = 0


def _title(text):
    return Text("text-3xl font-bold text-white", text)


def _crumb(nav: Navigator, label):
    return Text("text-sm font-mono text-slate-400", f"{label}  ·  depth {nav.depth}")


def home(nav: Navigator, c: Counter):
    return Column(
        "flex-1 items-center justify-center gap-6 bg-slate-950 p-8",
        _title("Home"),
        _crumb(nav, "home"),
        # Rendered only here, so seeing it unchanged after a push+pop is direct
        # evidence the store survived rather than the screen being rebuilt from
        # defaults.
        Text("text-7xl font-bold text-white tabular-nums", f"{c.n}"),
        Button(
            "px-6 py-3 rounded-2xl bg-indigo-500 active:bg-indigo-600 text-white font-medium",
            "Increment",
            on_tap=lambda: c.update(n=c.n + 1),
        ),
        Button(
            "px-6 py-3 rounded-2xl bg-emerald-600 active:bg-emerald-700 text-white font-medium",
            "Open settings",
            on_tap=lambda: nav.push(settings),
        ),
    )


def settings(nav: Navigator):
    return Column(
        "flex-1 items-center justify-center gap-6 bg-emerald-950 p-8",
        _title("Settings"),
        _crumb(nav, "settings"),
        Button(
            "px-6 py-3 rounded-2xl bg-indigo-500 active:bg-indigo-600 text-white font-medium",
            "Open item 42",
            on_tap=lambda: nav.push(detail, item_id=42),
        ),
        Button(
            "px-6 py-3 rounded-2xl bg-slate-700 active:bg-slate-600 text-white font-medium",
            "Back (in-app)",
            on_tap=nav.pop,
        ),
    )


def detail(nav: Navigator, item_id: int):
    return Column(
        "flex-1 items-center justify-center gap-6 bg-rose-950 p-8",
        _title(f"Item {item_id}"),
        _crumb(nav, "detail"),
        # The pushed param, rendered — so "params reach the screen" is a screenshot
        # claim, not a log claim.
        Text("text-5xl font-bold text-rose-200 tabular-nums", str(item_id)),
        Row(
            "gap-3",
            Button(
                "px-5 py-3 rounded-2xl bg-slate-700 active:bg-slate-600 text-white font-medium",
                "Pop",
                on_tap=nav.pop,
            ),
            Button(
                "px-5 py-3 rounded-2xl bg-slate-700 active:bg-slate-600 text-white font-medium",
                "To root",
                on_tap=nav.pop_to_root,
            ),
        ),
    )


def view():
    return Nav("flex-1", home, settings, detail)
