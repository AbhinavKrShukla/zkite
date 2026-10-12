"""Every bundled `Icon` sprite, at three sizes and three colours.

Built for phase_3/PLAN.md §18's device verification, and shaped so a screenshot alone
answers the three questions that matter:

* **all twelve sprites**, so a paint loop that only ever drew the first shape of a
  multi-shape sprite (`search`, `settings`) is visible rather than inferred;
* **three text sizes on one row each**, because `Icon`'s size is `1em` of the *inherited*
  font size — the thing the old hardcoded `w-[20px]` box only got right at one size. Row
  one is deliberately `text-[20px]`, the old constant, so "it works" and "it happens to
  match the constant" are distinguishable on sight;
* **`currentColor`**, via a row that sets no icon colour at all and inherits it, next to
  rows that set one — a sprite painted in a fixed colour would look identical on one row
  and wrong on the others.
"""

from zkite import Column, Icon, Row, Text

NAMES = [
    "check", "chevron-left", "chevron-right", "close",
    "heart", "home", "minus", "plus",
    "search", "settings", "star", "trash",
]


def row(cls: str, label: str):
    return Column("gap-2",
        Text("text-slate-500 text-[13px]", label),
        Row("flex-wrap items-center gap-4 " + cls, *[Icon("", n) for n in NAMES]),
    )


def view():
    return Column("flex-1 gap-7 p-6 bg-slate-950",
        Text("text-white text-[22px] font-bold", "Icons"),
        row("text-[20px] text-white", "20px / white — the size the old hardcoded box assumed"),
        row("text-[32px] text-indigo-400", "32px / indigo-400 — inherited size and colour"),
        row("text-[13px] text-emerald-400", "13px / emerald-400"),
        # An explicit size must still beat the 1em default, on both backends.
        Column("gap-2",
            Text("text-slate-500 text-[13px]", "explicit w-[44px] h-[44px] inside 13px text"),
            Row("items-center gap-4 text-[13px] text-rose-400",
                Icon("w-[44px] h-[44px]", "star"),
                Icon("w-[44px] h-[44px]", "search"),
                Icon("w-[44px] h-[44px]", "settings"),
                Text("text-rose-400", "<- 13px text, 44px icons"),
            ),
        ),
        # A name nothing matches: must render as an empty box, not crash and not draw
        # some other sprite.
        Column("gap-2",
            Text("text-slate-500 text-[13px]", "unknown name — expect nothing drawn, no crash"),
            Row("items-center gap-4 text-[32px] text-white",
                Icon("", "no-such-icon"),
                Text("text-slate-400 text-[13px]", "<- an empty 32px box"),
            ),
        ),
    )
