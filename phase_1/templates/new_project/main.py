"""Your first ZKite app. Edit this file and watch your phone update."""
from zkite import Button, Column, Row, ScrollView, SystemBars, Tabs, Text, store


@store
class Welcome:
    tab: int = 0
    saved: bool = False


def feature(number, title, description):
    return Row("gap-4 items-start py-4",
        Text("text-orange-600 text-lg font-bold", number),
        Column("flex-1 min-w-0 gap-1",
            Text("w-full text-slate-900 text-base font-bold", title),
            Text("w-full text-slate-500 text-sm", description),
        ),
    )


def view(s: Welcome):
    home = ScrollView("flex-1",
        Column("p-6 gap-6",
            Column("bg-orange-50 rounded-3xl p-6 gap-4",
                Text("text-orange-700 text-xs font-bold tracking-widest", "SMALL IDEA. BIG POSSIBILITIES."),
                Text("w-full text-3xl font-bold text-slate-900", "Make something\npeople love."),
                Text("w-full text-base text-slate-600", "A little Python. A beautiful Android app. Your next idea starts here."),
                Button("bg-orange-600 rounded-2xl px-5 py-4 text-white font-bold",
                    "Explore the toolkit", on_tap=lambda: s.update(tab=1)),
            ),
            Row("justify-between items-center",
                Text("text-xl font-bold text-slate-900", "Made for your flow"),
                Text("text-xs font-semibold text-orange-700", "PYTHON"),
            ),
            Column("bg-white rounded-2xl p-4 border border-orange-100",
                feature("01", "Think it. Build it.", "Compose real screens with simple, readable Python."),
                feature("02", "Stay in the moment", "Save your code. See your changes on your phone."),
                feature("03", "Make it yours", "Thoughtful type, expressive color, and room to grow."),
            ),
            Text("text-sm text-slate-400 text-center", "Crafted with ZKite. Built around you."),
        ),
    )
    toolkit = ScrollView("flex-1",
        Column("p-6 gap-5",
            Text("w-full text-3xl font-bold text-slate-900", "A feel for the details."),
            Text("w-full text-base text-slate-500", "Swipe between these pages, or tap a tab above. Try a little interaction below."),
            Column("bg-orange-50 rounded-3xl p-6 gap-4",
                Text("text-2xl", "✨"),
                Text("text-xl font-bold text-slate-900", "Your next great idea"),
                Text("w-full text-sm text-slate-600", "A reading list. A daily ritual. A tool your team will love. What will you make?"),
                Button("rounded-xl py-4 px-5 font-bold " + (
                    "bg-orange-100 text-orange-800" if s.saved else "bg-orange-600 text-white"),
                    "Added to your collection" if s.saved else "Save this inspiration",
                    on_tap=lambda: s.update(saved=not s.saved)),
            ),
            feature("↗", "Ready to make your mark?", "Open main.py and change a headline, a color, or the whole screen."),
        ),
    )
    return SystemBars(
        Column("flex-1 bg-white",
            Row("px-6 pt-5 pb-3 justify-between items-center",
                Text("text-2xl font-bold text-orange-600", "zkite"),
                Text("text-xs font-semibold text-slate-500", "IDEAS IN FLIGHT")),
            Tabs("flex-1 min-h-0", home, toolkit, labels=["Discover", "Playground"],
                 index=s.tab, on_change=lambda tab: s.update(tab=tab), key="welcome"),
        ), status_color="#ffffff", navigation_color="#ffffff",
    )
