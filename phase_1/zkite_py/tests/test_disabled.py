"""Disabled controls must not mutate stores, open popups or bubble taps."""
import unittest

from zkite import Button, Checkbox, Radio, Select, Slider, Switch, Text, TextField
from zkite.widgets import RenderContext


class DisabledTests(unittest.TestCase):
    def test_disabled_controls_consume_taps_without_running_handlers(self):
        for cls in (Button, Checkbox, Radio, Select, Slider, Switch, TextField):
            for mode in ('render', 'to_tree'):
                for labelled in (False, True) if cls in (Checkbox, Radio, Switch) else (False,):
                    with self.subTest(widget=cls.__name__, mode=mode, labelled=labelled):
                        calls = []
                        kids = [Text('', 'Label')] if labelled else []
                        widget = cls('', *kids, disabled=True,
                                     on_tap=lambda: calls.append('tap'),
                                     on_change=lambda value: calls.append(value))
                        ctx = RenderContext()
                        output = getattr(widget, mode)(ctx, 'control')
                        self.assertTrue(ctx.dispatch('control', 'tap'))
                        self.assertFalse(ctx.dispatch('control', 'change', 'true'))
                        self.assertEqual(calls, [])
                        if mode == 'to_tree':
                            self.assertTrue(output['attrs']['disabled'])
                            self.assertIn('opacity-50', output['classes'])
                        else:
                            self.assertIn('aria-disabled="true"', output)

    def test_reenabled_button_dispatches_again(self):
        calls = []
        button = Button('', 'Save', disabled=True, on_tap=lambda: calls.append('saved'))
        button.to_tree(RenderContext(), 'save')
        button.props['disabled'] = False
        ctx = RenderContext()
        button.to_tree(ctx, 'save')
        ctx.dispatch('save', 'tap')
        self.assertEqual(calls, ['saved'])
