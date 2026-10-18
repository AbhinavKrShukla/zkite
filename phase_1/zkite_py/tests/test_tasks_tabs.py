import threading
import types
import unittest
from zkite import App, Button, Column, Tabs, Task, Text
from zkite.widgets import RenderContext


class TaskTests(unittest.TestCase):
    def setUp(self):
        self.task = Task()
        self.painted = threading.Event()
        self.module = types.ModuleType("task_test")
        self.module.view = lambda: Column("", Text("", str(self.task.data)), Button("", "tap", key="tap", on_tap=lambda: None))
        self.app = App(self.module)
        self.app.render_tree()
        self.app.on_render_tree(lambda _: self.painted.set() if not self.task.loading else None)

    def test_blocked_io_does_not_block_events_and_completion_pushes(self):
        gate = threading.Event()
        try:
            self.task.start(lambda: (gate.wait(3), "loaded")[1])
            self.assertTrue(self.task.loading)
            self.assertTrue(self.app.dispatch("tap", "tap"))
            gate.set()
            self.assertTrue(self.painted.wait(3))
            self.assertEqual(self.task.data, "loaded")
            self.assertIsNone(self.task.error)
        finally:
            gate.set()

    def test_latest_result_wins(self):
        gate = threading.Event()
        finished = threading.Event()
        started = threading.Event()
        def old():
            started.set()
            gate.wait(3)
            finished.set()
            return "old"
        try:
            self.task.start(old)
            self.assertTrue(started.wait(3))
            self.task.start(lambda: "new")
            self.assertTrue(self.painted.wait(3))
            gate.set()
            self.assertTrue(finished.wait(3))
            self.assertEqual(self.task.data, "new")
        finally:
            gate.set()

    def test_error_is_reactive_and_retry_clears_it(self):
        def fail():
            raise ValueError("offline")
        self.task.start(fail)
        self.assertTrue(self.painted.wait(3))
        self.assertIsInstance(self.task.error, ValueError)
        self.painted.clear()
        self.task.start(lambda: "ok")
        self.assertTrue(self.painted.wait(3))
        self.assertIsNone(self.task.error)

    def test_reload_discards_pending_result(self):
        gate = threading.Event()
        try:
            self.task.start(lambda: gate.wait(3))
            self.app.use_module(self.module)
            self.assertFalse(self.task.loading)
            self.assertIsNone(self.task.data)
        finally:
            gate.set()

    def test_bad_reload_keeps_working_entrypoint(self):
        with self.assertRaisesRegex(AttributeError, "must define a callable"):
            self.app.use_module(types.ModuleType("broken"))
        self.assertIs(self.app.module, self.module)
        self.assertNotIn("plan1.md", self.app.render())


class TabsTests(unittest.TestCase):
    def test_swipe_and_tap_report_the_same_controlled_index(self):
        selected = []
        ctx = RenderContext()
        tabs = Tabs("flex-1", Text("", "one"), Text("", "two"), labels=["One", "Two"],
                    index=0, on_change=selected.append, key="tabs")
        tree = tabs.to_tree(ctx, "r")
        pager = tree["children"][1]
        self.assertEqual(pager["attrs"]["pager_count"], 2)
        self.assertTrue(ctx.dispatch("tabs:pages", "change", "1"))
        self.assertTrue(ctx.dispatch(tree["children"][0]["children"][1]["id"], "tap", ""))
        self.assertEqual(selected, [1, 1])
        ctx.dispatch("tabs:pages", "change", "99")
        self.assertEqual(selected, [1, 1])
        self.assertIn("scroll-snap-align:start", tabs.render(RenderContext(), "r"))

    def test_invalid_selection_rejected(self):
        with self.assertRaises(ValueError):
            Tabs("", Text("", "one"), labels=["One"], index=-1, on_change=lambda _: None)


class TextOptionsTests(unittest.TestCase):
    def test_text_controls_reach_both_backends(self):
        widget = Text("w-40", "Long title", multiline=False, overflow="ellipsis")
        tree = widget.to_tree(RenderContext(), "r")
        attrs = tree["children"][0]["attrs"]
        self.assertEqual(attrs, {"text_wrap": False, "text_max_lines": 1, "text_ellipsis": True})
        self.assertIn("text-overflow:ellipsis", widget.render(RenderContext(), "r"))
        self.assertIn("overflow-visible", Text("", "long", overflow="visible").to_tree(RenderContext(), "r")["classes"])
        with self.assertRaises(ValueError):
            Text("", "x", max_lines=0)
