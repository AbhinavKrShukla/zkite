"""Navigation: the stack policy, the back chain, and the things that would break it.

See phase_3/PLAN.md §16. Several of these exist because a navigation system can be
plausible and wrong in ways that only show up much later:

  - `test_pushed_screen_picks_up_a_reloaded_module` is the reason `Entry` holds a
    name rather than a function object. It fails outright against the "obvious"
    design.
  - `test_deeper_stack_builds_exactly_one_screen` pins the performance property the
    whole design is built around.
  - `test_nested_nav_gets_back_before_the_outer_one` pins the ordering that makes
    layer 1 composable, which is the part a future navigator depends on.
"""

from __future__ import annotations

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "src"))

from zkite import Column, Nav, Navigator, Text  # noqa: E402
from zkite._nav import Entry, _resolve_screen_args, screen_name  # noqa: E402
from zkite._store import get_instance, reset_registry, store  # noqa: E402
from zkite.widgets import RenderContext  # noqa: E402


def home():
    return Column("home", Text("", "Home"))


def settings():
    return Column("settings", Text("", "Settings"))


def detail(item_id: int):
    return Column("detail", Text("", f"Item {item_id}"))


# Module level, because that is where a `@store` actually lives: `@store` keys its
# one instance by module-qualified name, so a class defined inside a function would
# be a fresh class (and a fresh singleton) on every call. It also matters for
# `test_string_annotations_still_inject_stores` — this module has
# `from __future__ import annotations`, so the annotation below is the string
# "Session" and has to be resolved against these globals to be recognised at all.
@store
class Session:
    user: str = "ana"


def profile(session: Session, tab: str):
    return Text("", f"{session.user}/{tab}")


def whoami(session: Session):
    return Text("", session.user)


class NavStackTest(unittest.TestCase):
    def setUp(self):
        reset_registry()

    def nav(self) -> Navigator:
        return get_instance(Navigator)

    def test_first_render_seeds_the_root_screen(self):
        ctx = RenderContext()
        tree = Nav("flex-1", home, settings).to_tree(ctx, "r")
        self.assertEqual(self.nav().current, "home")
        self.assertEqual(tree["children"][0]["classes"], "flex flex-col home")

    def test_push_renders_the_new_screen_and_pop_returns(self):
        nav = self.nav()
        Nav("flex-1", home, settings).to_tree(RenderContext(), "r")

        nav.push(settings)
        tree = Nav("flex-1", home, settings).to_tree(RenderContext(), "r")
        self.assertEqual(nav.depth, 2)
        self.assertEqual(tree["children"][0]["classes"], "flex flex-col settings")

        self.assertTrue(nav.pop())
        tree = Nav("flex-1", home, settings).to_tree(RenderContext(), "r")
        self.assertEqual(tree["children"][0]["classes"], "flex flex-col home")

    def test_push_accepts_a_name_as_well_as_a_function(self):
        nav = self.nav()
        Nav("flex-1", home, settings).to_tree(RenderContext(), "r")
        nav.push("settings")
        self.assertEqual(nav.current, "settings")
        self.assertEqual(screen_name(settings), screen_name("settings"))

    def test_pop_refuses_to_empty_the_stack(self):
        nav = self.nav()
        Nav("flex-1", home).to_tree(RenderContext(), "r")
        self.assertEqual(nav.depth, 1)
        self.assertFalse(nav.pop())
        self.assertEqual(nav.depth, 1)

    def test_replace_and_reset_and_pop_to_root(self):
        nav = self.nav()
        Nav("flex-1", home, settings, detail).to_tree(RenderContext(), "r")
        nav.push(settings)
        nav.replace(detail, item_id=1)
        self.assertEqual((nav.depth, nav.current), (2, "detail"))
        nav.push(settings)
        self.assertTrue(nav.pop_to_root())
        self.assertEqual((nav.depth, nav.current), (1, "home"))
        nav.reset(settings)
        self.assertEqual((nav.depth, nav.current), (1, "settings"))

    def test_params_reach_the_screen_function(self):
        nav = self.nav()
        Nav("flex-1", home, detail).to_tree(RenderContext(), "r")
        nav.push(detail, item_id=42)
        tree = Nav("flex-1", home, detail).to_tree(RenderContext(), "r")
        text = tree["children"][0]["children"][0]["children"][0]["text"]
        self.assertEqual(text, "Item 42")

    def test_stores_and_params_are_injected_from_their_own_sources(self):
        bound = _resolve_screen_args(profile, {"tab": "posts"})
        self.assertEqual(bound["tab"], "posts")
        self.assertIs(bound["session"], get_instance(Session))

    def test_string_annotations_still_inject_stores(self):
        """`from __future__ import annotations` must not turn injection off.

        This module has that import at the top, so `Session` below arrives as the
        *string* `"Session"` rather than the class — which is exactly what a user
        writing an ordinary modern `main.py` gets. Without
        `signature_with_resolved_annotations` the store is not recognised and the
        screen fails with "needs a value for 'session'" about a parameter that
        visibly has an annotation. Pre-existing in `view()`'s own injection and
        fixed in the same place for both.
        """
        bound = _resolve_screen_args(whoami, {})
        self.assertIs(bound["session"], get_instance(Session))

    def test_a_missing_param_names_the_screen_rather_than_failing_deep_inside(self):
        with self.assertRaises(TypeError) as caught:
            _resolve_screen_args(detail, {})
        self.assertIn("detail()", str(caught.exception))
        self.assertIn("item_id", str(caught.exception))

    def test_unknown_screen_names_what_is_available(self):
        nav = self.nav()
        Nav("flex-1", home).to_tree(RenderContext(), "r")
        before = list(nav.stack)
        with self.assertRaises(KeyError) as caught:
            nav.push("nope")
        self.assertEqual(nav.stack, before)
        self.assertIn("home", str(caught.exception))

    def test_invalid_parameters_leave_stack_unchanged(self):
        nav = self.nav()
        Nav("", home, detail).to_tree(RenderContext(), "r")
        before = list(nav.stack)
        for params in ({}, {"item_id": 1, "typo": True}):
            with self.assertRaises(TypeError):
                nav.push(detail, **params)
            self.assertEqual(nav.stack, before)

    def test_entries_isolate_explicit_keys_and_restore_on_back(self):
        def page(item_id: int = 0):
            return Text("", str(item_id), key="shared")
        nav = self.nav()
        widget = Nav("", page)
        first = widget.to_tree(RenderContext(), "r")["children"][0]
        nav.push(page, item_id=1)
        second = widget.to_tree(RenderContext(), "r")["children"][0]
        self.assertNotEqual(first["id"], second["id"])
        nav.pop()
        self.assertEqual(first["id"], widget.to_tree(RenderContext(), "r")["children"][0]["id"])

    def test_screen_receives_its_own_host(self):
        seen = []
        def page(nav: Navigator):
            seen.append(nav)
            return Text("", "Page")
        host = Navigator()
        Nav("", page, host=host).to_tree(RenderContext(), "r")
        self.assertIs(seen[-1], host)
        self.assertIsNot(host, self.nav())

    def test_duplicate_screen_names_are_refused_at_construction(self):
        def home2():  # noqa: D401
            return Text("", "")

        home2.__name__ = "home"
        with self.assertRaises(ValueError):
            Nav("flex-1", home, home2)

    def test_nav_needs_at_least_one_screen(self):
        with self.assertRaises(ValueError):
            Nav("flex-1")


class BackChannelTest(unittest.TestCase):
    def setUp(self):
        reset_registry()

    def test_back_pops_until_the_root_then_declines(self):
        nav = get_instance(Navigator)
        ctx = RenderContext()
        Nav("flex-1", home, settings).to_tree(ctx, "r")
        nav.push(settings)

        ctx = RenderContext()
        Nav("flex-1", home, settings).to_tree(ctx, "r")
        self.assertTrue(ctx.dispatch_back())
        self.assertEqual(nav.current, "home")

        ctx = RenderContext()
        Nav("flex-1", home, settings).to_tree(ctx, "r")
        self.assertFalse(
            ctx.dispatch_back(),
            "back on the root screen must decline so the platform can exit the app",
        )

    def test_back_with_no_navigator_at_all_is_declined(self):
        ctx = RenderContext()
        Column("").to_tree(ctx, "r")
        self.assertFalse(ctx.dispatch_back())

    def test_nested_nav_gets_back_before_the_outer_one(self):
        """The ordering that makes layer 1 composable.

        An inner navigator registers later in the tree walk and must therefore be
        consulted first — otherwise an outer stack would swallow a Back that the
        screen currently on top wanted, which is the bug every nested-navigation
        system has to get right.
        """
        outer = Navigator(stack=[Entry("a", {}), Entry("b", {})])
        inner = Navigator(stack=[Entry("x", {}), Entry("y", {})])

        def screen_with_inner_nav():
            return Nav("", lambda: Text("", "x"), host=inner)

        ctx = RenderContext()
        ctx.register_back(outer.handle_back)
        ctx.register_back(inner.handle_back)

        self.assertTrue(ctx.dispatch_back())
        self.assertEqual(
            (inner.depth, outer.depth), (1, 2), "the inner stack should have popped"
        )

        self.assertTrue(ctx.dispatch_back())
        self.assertEqual(
            (inner.depth, outer.depth),
            (1, 1),
            "with the inner stack at its root, the outer one takes the next back",
        )

        self.assertFalse(ctx.dispatch_back())

    def test_a_custom_host_needs_only_two_methods(self):
        """Layer 2 is the extension point; prove it is actually reachable.

        This is a whole alternative navigation policy — no stack at all, just a
        latch — implemented against the documented protocol and driven by the real
        `Nav` widget. If this stops working, `NavigationHost` has quietly stopped
        being a contract anyone else can implement.
        """

        class Toggle:
            def __init__(self):
                self.showing = "home"

            def current_entry(self):
                return Entry(self.showing, {})

            def handle_back(self):
                if self.showing == "home":
                    return False
                self.showing = "home"
                return True

        toggle = Toggle()
        ctx = RenderContext()
        tree = Nav("flex-1", home, settings, host=toggle).to_tree(ctx, "r")
        self.assertEqual(tree["children"][0]["classes"], "flex flex-col home")
        self.assertFalse(ctx.dispatch_back())

        toggle.showing = "settings"
        ctx = RenderContext()
        tree = Nav("flex-1", home, settings, host=toggle).to_tree(ctx, "r")
        self.assertEqual(tree["children"][0]["classes"], "flex flex-col settings")
        self.assertTrue(ctx.dispatch_back())
        self.assertEqual(toggle.showing, "home")


class PerformanceShapeTest(unittest.TestCase):
    """The property the design exists to have, asserted rather than argued."""

    def setUp(self):
        reset_registry()

    def test_deeper_stack_builds_exactly_one_screen(self):
        built: list[str] = []

        def a():
            built.append("a")
            return Text("", "a")

        def b():
            built.append("b")
            return Text("", "b")

        def c():
            built.append("c")
            return Text("", "c")

        nav = get_instance(Navigator)
        Nav("flex-1", a, b, c).to_tree(RenderContext(), "r")
        nav.push(b)
        nav.push(c)

        built.clear()
        Nav("flex-1", a, b, c).to_tree(RenderContext(), "r")
        self.assertEqual(
            built,
            ["c"],
            "a three-deep stack must build one screen, not three — screens below "
            "the top are not materialised at all",
        )

    def test_back_chain_is_empty_when_no_navigator_rendered(self):
        ctx = RenderContext()
        Column("", Text("", "x")).to_tree(ctx, "r")
        self.assertEqual(
            ctx.back_handlers, [], "an app with no Nav pays nothing for the back chain"
        )


class HotReloadIdentityTest(unittest.TestCase):
    """Why `Entry` holds a name and not a function object."""

    def setUp(self):
        reset_registry()

    def test_pushed_screen_picks_up_a_reloaded_module(self):
        """A pushed screen must render the *new* code after a hot reload.

        This is the test that fails against the obvious design. If `Entry` held the
        function object it was pushed with, the stack would pin the pre-reload
        closure forever and `zkite dev` would appear to work only on whichever screen
        happened to be the root — a bug that shows up as "hot reload is flaky"
        rather than as anything pointing at navigation.

        Simulated the way a reload actually behaves: the module is re-executed, so
        the *same name* now refers to a different function object.
        """
        nav = get_instance(Navigator)

        def settings_v1():
            return Text("", "v1")

        settings_v1.__name__ = "settings"

        def settings_v2():
            return Text("", "v2")

        settings_v2.__name__ = "settings"

        Nav("flex-1", home, settings_v1).to_tree(RenderContext(), "r")
        # Pushed by *function*, which is the form a real app writes and the only
        # form where a naive design could capture the stale object — pushing by
        # name would be safe however the entry stored it, so it would not exercise
        # this at all.
        nav.push(settings_v1)
        tree = Nav("flex-1", home, settings_v1).to_tree(RenderContext(), "r")
        self.assertEqual(tree["children"][0]["children"][0]["text"], "v1")

        # The reload: view() re-runs and hands Nav the freshly-executed functions.
        tree = Nav("flex-1", home, settings_v2).to_tree(RenderContext(), "r")
        self.assertEqual(
            tree["children"][0]["children"][0]["text"],
            "v2",
            "a pushed screen must pick up reloaded code, not the object it was "
            "pushed with",
        )
        self.assertEqual(nav.current, "settings", "and must stay on the same screen")


class BothBackendsAgreeTest(unittest.TestCase):
    """`Nav` has two render paths and they must not drift (phase_1/PLAN.md §3)."""

    def setUp(self):
        reset_registry()

    def test_html_and_tree_render_the_same_screen(self):
        nav = get_instance(Navigator)
        Nav("flex-1", home, settings).to_tree(RenderContext(), "r")
        nav.push(settings)

        html = Nav("flex-1", home, settings).render(RenderContext(), "r")
        tree = Nav("flex-1", home, settings).to_tree(RenderContext(), "r")

        self.assertIn("settings", html)
        self.assertNotIn(">Home<", html)
        self.assertEqual(tree["children"][0]["classes"], "flex flex-col settings")

    def test_both_paths_register_the_back_handler(self):
        html_ctx = RenderContext()
        Nav("flex-1", home, settings).render(html_ctx, "r")
        tree_ctx = RenderContext()
        Nav("flex-1", home, settings).to_tree(tree_ctx, "r")
        self.assertEqual(len(html_ctx.back_handlers), 1)
        self.assertEqual(len(tree_ctx.back_handlers), 1)

    def test_nav_implies_min_h_0_so_a_tall_screen_cannot_push_its_footer_offscreen(self):
        """Found in a real app: a `List` of 600 rows inside a `Nav` laid the screen
        out 30614dp tall against a 960dp viewport, pushing the app's tab bar off the
        bottom of the display with no error anywhere.

        A flex item's default `min-height: auto` will not shrink below its content,
        so `Nav("flex-1", ...)` grows to its screen's full content height unless
        `min-h-0` says otherwise. `ScrollView` and `List` already carry it for the
        same reason; `Nav` is a full-screen host and had been missing it.
        """
        self.assertIn("min-h-0", Nav("flex-1", home, settings).classes())
        # Both render paths, since the divergence would otherwise be backend-specific.
        tree = Nav("flex-1", home, settings).to_tree(RenderContext(), "r")
        self.assertIn("min-h-0", tree["classes"])
        html = Nav("flex-1", home, settings).render(RenderContext(), "r")
        self.assertIn("min-h-0", html.split(">", 1)[0])


if __name__ == "__main__":
    unittest.main()
