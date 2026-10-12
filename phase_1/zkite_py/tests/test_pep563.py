"""`from __future__ import annotations` (PEP 563), as a *class* of bug rather than two.

phase_3/PLAN.md §25.3 names this gap outright:

    **The PEP 563 class is still untested.** §25.1 fixes the one instance that
    crashed. No fixture in `zkite_py`, the conformance harness or either demo app
    uses `from __future__ import annotations`, so the next divergence of that
    shape is equally unguarded.

Two instances have been found so far, in two different layers, a phase apart:

  - **§16.8** — `view()`'s stores are injected *by annotation*, and under PEP 563
    every annotation is the string `"Counter"` rather than the class, so injection
    silently stopped working and reported that a parameter which visibly is a
    `@store` is not one. Fixed in `_store.signature_with_resolved_annotations`.
  - **§25.1** — `dataclasses` resolves annotation *names* textually to find
    `KW_ONLY`, through an unguarded `sys.modules.get(cls.__module__).__dict__`, so a
    `@store` in a module not yet registered in `sys.modules` could not be
    constructed at all. Fixed in `reload.load_module`.

Neither fix covers the other, and neither was found by a test — both were found by
someone's real app failing to boot. What they have in common is not a line of code,
it is that **every annotation ZKite reads becomes a string**, so the guard has to be
an app that actually uses the import and actually exercises every path that reads an
annotation, end to end. That is what this file is: one realistic `main.py`, run
through the real loader, the real `App`, both render paths, a `Nav` screen, and a
hot reload.

`test_reload.py::SysModulesRegistrationTests` still covers §25.1 at the loader level
and is deliberately not merged into this — it is a unit test of one function, and
this is a fixture over the stack.
"""

from __future__ import annotations

import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

import zkite._store as store_mod
from zkite.app import App
from zkite.reload import load_module, reload_module
from zkite._store import set_render_callback


# A deliberately ordinary app. Everything in it is something a real `main.py` does,
# and every one of them reads an annotation somewhere:
#
#   - `@store` on a class whose fields are annotated  -> dataclasses (§25.1)
#   - `view(s: AppState)`                             -> view injection (§16.8)
#   - `home(nav: Navigator, s: AppState)`             -> screen injection, the second
#                                                        injection site, and the one
#                                                        §16.8 was actually found by
#   - `detail(nav: Navigator, item_id: int)`          -> a pushed param sitting next
#                                                        to a store annotation, which
#                                                        is the case that has to tell
#                                                        the two apart by annotation
APP_V1 = """
from __future__ import annotations

from zkite import Button, Column, Nav, Navigator, Slider, Switch, Text, store


@store
class AppState:
    n: int = 0
    on: bool = False
    volume: float = 40.0


def home(nav: Navigator, s: AppState):
    return Column(
        "home",
        Text("", f"n={s.n} on={s.on} volume={s.volume}"),
        Button("", "inc", on_tap=lambda: s.update(n=s.n + 1)),
        Switch(checked=s.on, on_change=lambda v: s.update(on=v)),
        Slider("", min=0, max=100, value=s.volume, on_change=lambda v: s.update(volume=v)),
        Button("", "open", on_tap=lambda: nav.push(detail, item_id=7)),
    )


def detail(nav: Navigator, s: AppState, item_id: int):
    return Column("detail", Text("", f"item={item_id} n={s.n}"))


def view():
    return Nav("flex-1", home, detail)
"""

# Same store shape, different rendering — a hot reload that must migrate rather than
# reset, with the future-import still in place on both sides.
APP_V2 = APP_V1.replace('Text("", f"n={s.n}', 'Text("", f"RELOADED n={s.n}')


class Pep563FixtureTests(unittest.TestCase):
    def setUp(self):
        store_mod.reset_registry()
        set_render_callback(None)
        self._tmpdir = tempfile.TemporaryDirectory()
        self.main_path = Path(self._tmpdir.name) / "main.py"
        sys.modules.pop("main", None)

    def tearDown(self):
        self._tmpdir.cleanup()
        sys.modules.pop("main", None)
        store_mod.reset_registry()
        set_render_callback(None)

    def _boot(self, source: str) -> App:
        self.main_path.write_text(textwrap.dedent(source))
        return App(load_module(self.main_path, "main"))

    def test_the_whole_app_boots_and_renders_under_future_annotations(self):
        # §25.1's failure was at *construction* — this app could not reach a first
        # frame at all. Both render paths, because the two backends call different
        # entry points and only one of them was ever exercised by the app that found
        # this.
        app = self._boot(APP_V1)
        html = app.render()
        tree = app.render_tree()
        self.assertIn("n=0", html)
        self.assertIn("n=0", _all_text(tree))

    def test_stores_are_injected_into_a_screen_not_just_into_view(self):
        # §16.8's failure. `view()` here takes *no* store at all, so the only way
        # `s.n` can be rendered is if the screen's own `s: AppState` annotation was
        # resolved — which under PEP 563 is the string "AppState".
        app = self._boot(APP_V1)
        self.assertIn("on=False", app.render())

    def test_a_pushed_param_and_a_store_annotation_are_told_apart(self):
        # `detail(nav, s: AppState, item_id: int)` — one param must come from the
        # store registry and the other from what it was pushed with, decided purely
        # by the annotation. Under PEP 563 both are strings, so a resolver that
        # merely checks "is this annotated" rather than "does this resolve to a
        # @store" would mix them up.
        app = self._boot(APP_V1)
        app.render()
        tap(app, "open")
        rendered = app.render()
        self.assertIn("item=7", rendered)
        self.assertIn("n=0", rendered)

    def test_events_still_dispatch_and_mutate_the_store(self):
        # A store that cannot be constructed also cannot be mutated; this is the
        # end-to-end version of "the app actually works", not just "it rendered".
        app = self._boot(APP_V1)
        app.render()
        tap(app, "inc")
        self.assertIn("n=1", app.render())

    def test_typed_change_payloads_survive_the_future_import(self):
        # `_payload_parser` (§26) is looked up per widget and does not read
        # annotations, but the *store field types* it feeds do — and a `bool` field
        # holding the string "false" is exactly the §26 bug wearing §25's clothes.
        app = self._boot(APP_V1)
        app.render()
        app.dispatch(_find_id(app, "change", "checked"), "change", "true")
        self.assertIn("on=True", app.render())
        app.dispatch(_find_id(app, "change", "min"), "change", "62")
        self.assertIn("volume=62.0", app.render())

    def test_hot_reload_of_a_future_annotations_module_migrates_state(self):
        # The reload path re-executes the module, so it re-runs `@store` — i.e. it
        # re-enters §25.1's failure on every save, not only at boot.
        app = self._boot(APP_V1)
        app.render()
        tap(app, "inc")
        tap(app, "inc")

        self.main_path.write_text(textwrap.dedent(APP_V2))
        result = reload_module(self.main_path, "main")
        app.use_module(result.module)

        self.assertEqual(result.reset_stores, [], "same-shape store must migrate, not reset")
        rendered = app.render()
        self.assertIn("RELOADED", rendered, "the edit must be live")
        self.assertIn("n=2", rendered, "and the state must survive it")


def _all_text(node: dict) -> str:
    if node.get("tag") == "#text":
        return node.get("text", "")
    return "".join(_all_text(c) for c in node.get("children", []))


def _find_id(app: App, event: str, attr: str) -> str:
    """The node id of the one widget registering `event` and carrying `attr`.

    Reads the tree rather than hardcoding a path, so this does not silently start
    testing a different widget if the fixture app's layout changes.
    """
    found: list[str] = []

    def walk(node: dict) -> None:
        if node.get("tag") == "#text":
            return
        if event in (node.get("events") or []) and attr in (node.get("attrs") or {}):
            found.append(node["id"])
        for c in node.get("children", []):
            walk(c)

    walk(app.render_tree())
    assert len(found) == 1, f"expected exactly one {event}/{attr} widget, got {found}"
    return found[0]


def tap(app: App, label: str) -> None:
    """Dispatch a tap to the button rendering `label`, found by walking the tree."""
    target: list[str] = []

    def walk(node: dict) -> None:
        if node.get("tag") == "#text":
            return
        if node.get("tag") == "button" and label in _all_text(node):
            target.append(node["id"])
        for c in node.get("children", []):
            walk(c)

    walk(app.render_tree())
    assert target, f"no button labelled {label!r}"
    assert app.dispatch(target[0], "tap"), f"tap on {label!r} was not handled"


if __name__ == "__main__":
    unittest.main()
