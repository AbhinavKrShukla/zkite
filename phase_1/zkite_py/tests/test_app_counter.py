"""End-to-end (host-side) exercise of the plan1.md §8 counter app: build the tree,
render it, tap the button through the same dispatch path the WebView bridge will use,
verify the store survives a simulated hot reload, and verify a broken `view()` renders
the on-device error overlay instead of crashing the render loop.

This is the "part of the stack most likely to have a design bug, exercised end to end
short of the device hop" mentioned in ../PLAN.md §9.
"""

import contextlib
import io
import tempfile
import textwrap
import unittest
from pathlib import Path

import zkite._store as store_mod
from zkite.app import App
from zkite.reload import load_module, reload_module
from zkite._store import set_render_callback

COUNTER_APP = """
from zkite import Column, Text, Button, store

@store
class Counter:
    n: int = 0

def view(c: Counter):
    return Column("flex-1 items-center justify-center gap-6 bg-slate-950",
        Text("text-7xl font-bold text-white tabular-nums", f"{c.n}"),
        Button("px-6 py-3 rounded-2xl bg-indigo-500 active:bg-indigo-600 text-white font-medium",
               "Increment", on_tap=lambda: c.update(n=c.n + 1)),
    )
"""

BROKEN_APP = """
from zkite import Column, Text, store

@store
class Counter:
    n: int = 0

def view(c: Counter):
    1 / 0
    return Column("", Text("", "unreachable"))
"""


class CounterAppTests(unittest.TestCase):
    def setUp(self):
        store_mod.reset_registry()
        set_render_callback(None)
        self._tmpdir = tempfile.TemporaryDirectory()
        self.main_path = Path(self._tmpdir.name) / "main.py"

    def tearDown(self):
        self._tmpdir.cleanup()

    def _write(self, source: str) -> None:
        self.main_path.write_text(textwrap.dedent(source))

    def test_initial_render_shows_zero(self):
        self._write(COUNTER_APP)
        module = load_module(self.main_path, "main")
        app = App(module)
        html = app.render()
        self.assertIn("bg-slate-950", html)
        self.assertIn(">0<", html)
        self.assertIsNone(app.last_error)

    def test_tap_increments_and_rerenders_via_store_callback(self):
        self._write(COUNTER_APP)
        module = load_module(self.main_path, "main")
        app = App(module)
        renders = []
        app.on_render(renders.append)

        app.render()
        self.assertEqual(len(renders), 1)

        handled = app.dispatch("r.1", "tap")  # Column -> [0]=Text, [1]=Button
        self.assertTrue(handled)
        self.assertEqual(len(renders), 2)  # update() triggered a re-render
        self.assertIn(">1<", renders[-1])

    def test_state_survives_hot_reload_across_a_style_only_edit(self):
        self._write(COUNTER_APP)
        module = load_module(self.main_path, "main")
        app = App(module)
        app.render()
        app.dispatch("r.1", "tap")
        app.dispatch("r.1", "tap")
        self.assertIn(">2<", app.last_html)

        # simulate a save that only changes styling, not the store shape
        edited = COUNTER_APP.replace("bg-slate-950", "bg-black")
        self._write(edited)
        result = reload_module(self.main_path, "main")
        self.assertEqual(result.reset_stores, [])

        app.use_module(result.module)
        html = app.render()
        self.assertIn("bg-black", html)
        self.assertIn(">2<", html)  # counter value survived the reload

    def test_broken_view_renders_error_overlay_and_keeps_previous_html(self):
        self._write(COUNTER_APP)
        module = load_module(self.main_path, "main")
        app = App(module)
        good_html = app.render()
        self.assertIsNone(app.last_error)

        self._write(BROKEN_APP)
        result = reload_module(self.main_path, "main")
        app.use_module(result.module)
        # Swallowed because this test is about the overlay, not the log line the same
        # failure now also writes — leaving it loose puts a traceback in the middle of
        # a passing suite's output, which reads as a failure.
        with contextlib.redirect_stderr(io.StringIO()):
            html = app.render()

        self.assertIsNotNone(app.last_error)
        self.assertIn("ZeroDivisionError", app.last_error)
        self.assertIn("__zkite_error_overlay", html)
        self.assertIn(good_html, html)  # last good frame stays under the overlay

    # ---- render_tree (phase 3's native path) -------------------------------------
    # Same store/view/dispatch machinery as the HTML tests above, exercised through
    # `render_tree`/`on_render_tree` instead of `render`/`on_render` — see
    # `zkite.app.App` and phase_3/PLAN.md §5.

    def test_tree_initial_render_shows_zero(self):
        self._write(COUNTER_APP)
        module = load_module(self.main_path, "main")
        app = App(module)
        t = app.render_tree()
        self.assertIn("bg-slate-950", t["classes"])
        count_node = t["children"][0]
        self.assertEqual(count_node["children"], [{"tag": "#text", "text": "0"}])
        self.assertIsNone(app.last_error)

    def test_tree_tap_increments_and_rerenders_via_store_callback(self):
        self._write(COUNTER_APP)
        module = load_module(self.main_path, "main")
        app = App(module)
        renders = []
        app.on_render_tree(renders.append)

        app.render_tree()
        self.assertEqual(len(renders), 1)

        handled = app.dispatch("r.1", "tap")
        self.assertTrue(handled)
        self.assertEqual(len(renders), 2)  # update() triggered a re-render
        self.assertEqual(renders[-1]["children"][0]["children"], [{"tag": "#text", "text": "1"}])

    def test_tree_mode_does_not_recompute_html_on_store_update(self):
        # `_render_all` (zkite.app) must recompute only the representation this App
        # instance is actually running as — driving `view()` (a user function) twice
        # per state change for a backend that only ever consumes one of the two
        # outputs would be real, avoidable overhead on every single interaction.
        self._write(COUNTER_APP)
        module = load_module(self.main_path, "main")
        app = App(module)
        app.render_tree()
        stale_html = app.last_html
        app.dispatch("r.1", "tap")
        self.assertEqual(app.last_html, stale_html)  # never touched
        self.assertEqual(app.last_tree["children"][0]["children"], [{"tag": "#text", "text": "1"}])

    def test_tree_dispatch_and_html_dispatch_share_the_same_handlers(self):
        # `render()` and `render_tree()` both go through `_register_events` against
        # the same `RenderContext` shape (same node-id scheme) — a tap resolved by
        # the native bridge's node ids must hit the exact handler the HTML backend
        # would have, not a coincidentally-similar one.
        self._write(COUNTER_APP)
        module = load_module(self.main_path, "main")
        app = App(module)
        app.render_tree()
        self.assertTrue(app.dispatch("r.1", "tap"))
        self.assertEqual(app.last_tree["children"][0]["children"], [{"tag": "#text", "text": "1"}])

    def test_broken_view_renders_error_tree_and_keeps_previous_tree(self):
        self._write(COUNTER_APP)
        module = load_module(self.main_path, "main")
        app = App(module)
        good_tree = app.render_tree()
        self.assertIsNone(app.last_error)

        self._write(BROKEN_APP)
        result = reload_module(self.main_path, "main")
        app.use_module(result.module)
        with contextlib.redirect_stderr(io.StringIO()):  # see the HTML twin above
            t = app.render_tree()

        self.assertIsNotNone(app.last_error)
        self.assertIn("ZeroDivisionError", app.last_error)
        self.assertEqual(t["id"], "__zkite_root")
        self.assertEqual(t["children"][0], good_tree)  # last good frame preserved underneath
        overlay = t["children"][1]
        self.assertEqual(overlay["id"], "__zkite_error_overlay")
        # One `#text` child per traceback line, not one blob: the native renderer
        # shapes a text node as a single run, so a whole traceback in one child was
        # one very long line running off the right edge of the phone — on the one
        # screen a user is looking at *because* something already went wrong. Its HTML
        # twin sets `white-space:pre-wrap` and always read fine, so only the native
        # rendering was broken. See zkite/errors.py and phase_3/PLAN.md §31.3.
        lines = [c["text"] for c in overlay["children"]]
        self.assertGreater(len(lines), 1, "the traceback must be split per line")
        self.assertNotIn("\n", "".join(lines), "no child may still carry a newline")
        self.assertIn("ZeroDivisionError", "\n".join(lines))
        self.assertIn("flex flex-col", overlay["classes"])

    # ---- the traceback also reaches a log (phase_3/PLAN.md §19.3) -----------------
    # Until this existed the overlay was the *only* channel: an app whose `view()`
    # raised wrote nothing to logcat on any tag, so §46.8.1's device sweep passed
    # `error_repro.py` — broken by construction — with `fatal=0`. `zkite.errors.
    # log_error` writes to stderr, which every shell already tees into logcat.

    def _render_broken(self, times: int = 1, mode: str = "tree") -> str:
        """Render a broken app `times` times, returning everything it wrote to stderr."""
        self._write(BROKEN_APP)
        module = load_module(self.main_path, "main")
        app = App(module)
        self._app = app
        captured = io.StringIO()
        with contextlib.redirect_stderr(captured):
            for _ in range(times):
                app.render_tree() if mode == "tree" else app.render()
        return captured.getvalue()

    def test_a_broken_view_writes_its_traceback_to_stderr(self):
        err = self._render_broken()
        self.assertIn("ZKITEERROR", err)
        self.assertIn("ZeroDivisionError", err)
        self.assertIn("division by zero", err)
        # Every line carries the marker, so a logcat grep for one prefix returns the
        # whole traceback rather than its first line and then silence.
        body = [ln for ln in err.strip().split("\n")]
        self.assertTrue(all(ln.startswith("ZKITEERROR") for ln in body), body)

    def test_the_html_backend_logs_it_too(self):
        # Not a native-only gap: launched by anything other than `zkite dev`, the
        # WebView backend was equally silent. Fixed once, in `App`, so both backends
        # stay conformant by construction rather than by discipline (phase_1 §3).
        err = self._render_broken(mode="html")
        self.assertIn("ZKITEERROR", err)
        self.assertIn("ZeroDivisionError", err)

    def test_sixty_broken_frames_log_exactly_one_traceback(self):
        # THE control that matters. A broken `view()` raises on *every* frame, so an
        # unguarded write is sixty tracebacks a second during a scroll. Without
        # `App._note_error`'s `tb != self.last_error` guard this reads 60.
        err = self._render_broken(times=60)
        self.assertEqual(err.count("ZKITEERROR view() raised:"), 1, err[:400])
        self.assertEqual(err.count("ZeroDivisionError"), 1)

    def test_a_different_failure_logs_again_rather_than_being_swallowed(self):
        # The other side of the guard: "log once" must mean once per *distinct*
        # failure, not once per process. A hot reload into a second, different bug
        # would otherwise be invisible — the log would still be describing the first.
        self._write(BROKEN_APP)
        module = load_module(self.main_path, "main")
        app = App(module)
        captured = io.StringIO()
        with contextlib.redirect_stderr(captured):
            app.render_tree()
            app.render_tree()
            self._write(BROKEN_APP.replace("1 / 0", "raise KeyError('nope')"))
            app.use_module(reload_module(self.main_path, "main").module)
            app.render_tree()
        err = captured.getvalue()
        self.assertEqual(err.count("ZKITEERROR view() raised:"), 2, err[:600])
        self.assertIn("ZeroDivisionError", err)
        self.assertIn("KeyError", err)

    def test_recovering_logs_the_cleared_line_and_only_on_the_transition(self):
        # §19.3's third bullet: nothing had ever checked that a reload back to a
        # working `view()` clears the error. Without the `cleared` line a gate reading
        # the log sees an app break and never sees it recover, so the last word in the
        # buffer says "broken" for an app that is fine.
        self._write(BROKEN_APP)
        module = load_module(self.main_path, "main")
        app = App(module)
        captured = io.StringIO()
        with contextlib.redirect_stderr(captured):
            app.render_tree()
            self._write(COUNTER_APP)
            app.use_module(reload_module(self.main_path, "main").module)
            for _ in range(5):
                app.render_tree()
        err = captured.getvalue()
        self.assertIsNone(app.last_error)
        self.assertEqual(err.count("ZKITEERROR cleared"), 1, err[:400])
        # …and the control: five *more* healthy frames add nothing at all.
        captured2 = io.StringIO()
        with contextlib.redirect_stderr(captured2):
            for _ in range(5):
                app.render_tree()
        self.assertEqual(captured2.getvalue(), "")

    def test_a_healthy_app_logs_nothing(self):
        # The negative control for the whole feature: an app that never fails must
        # produce a byte-empty stderr, or every passing device run acquires noise that
        # a gate would then have to filter.
        self._write(COUNTER_APP)
        module = load_module(self.main_path, "main")
        app = App(module)
        captured = io.StringIO()
        with contextlib.redirect_stderr(captured):
            for _ in range(10):
                app.render_tree()
            app.dispatch("r.1", "tap")
        self.assertEqual(captured.getvalue(), "")


if __name__ == "__main__":
    unittest.main()
