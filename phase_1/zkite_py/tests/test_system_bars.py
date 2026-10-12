import contextlib
import io
import types
import unittest

from zkite import App, Button, Column, Overlay, SystemBars, Text
from zkite.widgets import RenderContext


def app_for(view):
    module = types.ModuleType("system_bar_test")
    module.view = view
    return App(module)


class SystemBarsTests(unittest.TestCase):
    def test_wrapper_preserves_layout_ids_and_event_dispatch(self):
        taps = []
        screen = Column("flex-1", Button("", "Tap", on_tap=lambda: taps.append(True)))
        original = app_for(lambda: screen).render_tree()
        app = app_for(lambda: SystemBars(screen, status_color="#123"))
        styled = app.render_tree()
        style = styled["attrs"].pop("system_bars")
        self.assertEqual(styled, original)
        self.assertEqual(style["status_color"], "#112233")
        self.assertEqual(style["navigation_color"], "#112233")
        self.assertTrue(app.dispatch("r.0", "tap"))
        self.assertEqual(taps, [True])

    def test_auto_icons_and_independent_colors(self):
        style = SystemBars(Text("", "Hello"), status_color="#fff", navigation_color="#000").style
        self.assertTrue(style["status_dark_icons"])
        self.assertFalse(style["navigation_dark_icons"])
        manual = SystemBars(Text("", "Hello"), status_icons="light", navigation_icons="dark").style
        self.assertFalse(manual["status_dark_icons"])
        self.assertTrue(manual["navigation_dark_icons"])

    def test_rejects_invalid_colors_icons_and_children(self):
        for color in ("red", "#1234", "#xyz", "#12345678", None, 123):
            with self.subTest(color=color), self.assertRaises(ValueError):
                SystemBars(Text(), status_color=color)
        with self.assertRaises(ValueError):
            SystemBars(Text(), status_icons="white")
        with self.assertRaises(TypeError):
            SystemBars("not a widget")

    def test_overlay_wrapper_keeps_system_bar_settings_at_frame_root(self):
        screen = SystemBars(Column("", Text("", "Screen"), Overlay("", Text("", "Dialog"))),
                            status_color="#abcdef")
        tree = app_for(lambda: screen).render_tree()
        self.assertEqual(tree["id"], "__zkite_root")
        self.assertEqual(tree["attrs"]["system_bars"]["status_color"], "#abcdef")

    def test_settings_change_and_reset_with_screen_and_survive_errors(self):
        app = app_for(lambda: SystemBars(Text("", "Screen"), status_color="#123456"))
        app.render_tree()
        def broken():
            raise ValueError("broken screen")
        app.view_fn = broken
        with contextlib.redirect_stderr(io.StringIO()):
            tree = app.render_tree()
        self.assertEqual(tree["attrs"]["system_bars"]["status_color"], "#123456")
        app.view_fn = lambda: SystemBars(Text(), status_color="#654321")
        self.assertEqual(app.render_tree()["attrs"]["system_bars"]["status_color"], "#654321")
        app.view_fn = lambda: Text()
        self.assertNotIn("system_bars", app.render_tree()["attrs"])

    def test_webview_child_html_is_unchanged(self):
        child = Text("text-lg", "Hello")
        self.assertEqual(SystemBars(child).render(RenderContext(), "r"), child.render(RenderContext(), "r"))
