"""Minimal §7.6 repro (phase_3/PLAN.md §7.9's bisection): the smallest known tree
that reproduces "an unrelated text node vanishes once a TextField can be focused" —
3 text-bearing nodes (the "0", the TextField's own synthesized empty text child, and
one more sibling Text). Point ZKITE_APP_MAIN at this file (see
scripts/build_apk.sh) instead of demo_app/main.py to build a harness around just
this tree.
"""

from zkite import Column, Row, Text, TextField, store


@store
class State:
    name: str = ""


def view(s: State):
    return Column(
        "flex-1 items-center justify-center gap-6 bg-slate-950",
        Text("text-7xl font-bold text-white tabular-nums", "0"),
        TextField("px-4 py-3 rounded-xl bg-slate-800 text-white", placeholder="Type", value=s.name),
        Row("shrink-0 items-center px-4 py-3", Text("text-white", "Item 0")),
    )
