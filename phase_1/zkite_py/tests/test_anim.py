"""The author-facing animation surface — phase_3/PLAN.md §47, Queue F layer 3.

What these pin, in rough order of how much they matter:

1. `anim=` reaches the wire on **every** widget, including the eleven that override
   `_tree_attrs` and build their own dict. That is what the injection point in
   `Widget.to_tree` was chosen for, and it is the thing a per-widget approach would
   have got right for the widgets someone remembered.
2. A declaration that cannot animate is rejected **where it is written**, with a
   reason. `rotate`/`scale` are the specimens: they parse in `twcore`, they work on
   the WebView backend, and the native paint path drops them (§45.7) — so accepting
   one would produce an animation that runs, completes, fires its callback and moves
   no pixels.
3. The completion payload is typed at the widget, which §26.2's history says is the
   difference between a working handler and a string nobody expected.
"""

import contextlib
import io
import unittest

import zkite._store as store_mod
from zkite import Anim, AnimEnd, Button, Column, Slider, Switch, Text
from zkite._anim import _reset_html_warning_for_tests, parse_anim_end
from zkite._store import set_render_callback
from zkite.widgets import RenderContext


class AnimSpecTests(unittest.TestCase):
    def setUp(self):
        store_mod.reset_registry()
        set_render_callback(None)
        _reset_html_warning_for_tests()

    # ---- the wire format --------------------------------------------------------

    def test_one_channel_per_wire_entry_with_defaults_omitted(self):
        wire = Anim.enter(opacity=(0, 1), translate_y=(16, 0), ms=220, ease="out").to_wire()
        self.assertEqual(
            wire,
            [
                {"ch": "opacity", "to": 1.0, "from": 0.0, "ms": 220.0, "ease": "out"},
                {"ch": "translate_y", "to": 0.0, "from": 16.0, "ms": 220.0, "ease": "out"},
            ],
        )
        # `on` is omitted for the default trigger and present otherwise — this rides
        # inside every rebuilt tree, and §43 measured what tree JSON costs on a
        # rebuild frame.
        self.assertNotIn("on", wire[0])
        self.assertEqual(Anim.exit_(opacity=0, ms=150).to_wire()[0]["on"], "exit")
        self.assertEqual(Anim.change(translate_y=8, ms=150).to_wire()[0]["on"], "change")

    def test_a_scalar_is_a_target_and_a_pair_is_from_and_to(self):
        # An unstated `from` must be *absent*, not zero: absent means "wherever this
        # node's class string already puts it", resolved on the renderer side. Sending
        # 0.0 would make every unstated fade start from fully transparent.
        scalar = Anim.enter(opacity=0.5, ms=100).to_wire()[0]
        self.assertNotIn("from", scalar)
        self.assertEqual(scalar["to"], 0.5)
        pair = Anim.enter(opacity=(0.25, 0.75), ms=100).to_wire()[0]
        self.assertEqual((pair["from"], pair["to"]), (0.25, 0.75))

    def test_springs_delays_repeats_and_directions_all_reach_the_wire(self):
        w = Anim.change(translate_y=0, spring=(180, 26)).to_wire()[0]
        self.assertEqual(w["spring"], [180.0, 26.0])
        self.assertNotIn("ms", w)
        w2 = Anim.enter(opacity=1, ms=100, delay=300, repeat="forever", dir="alternate").to_wire()[0]
        self.assertEqual((w2["delay"], w2["repeat"], w2["dir"]), (300.0, "forever", "alternate"))
        # A raw cubic-bezier, so an author is never stuck with the five presets.
        w3 = Anim.enter(opacity=1, ms=100, ease=[0.2, -0.5, 0.8, 1.5]).to_wire()[0]
        self.assertEqual(w3["ease"], [0.2, -0.5, 0.8, 1.5])

    def test_a_list_of_anims_merges_into_one_declaration(self):
        # Both halves of a sheet's transition live on one node.
        w = Column("", anim=[Anim.enter(translate_y=(320, 0), spring=(180, 26)),
                             Anim.exit_(translate_y=(0, 320), ms=180)])._anim_wire()
        self.assertEqual([e["ch"] for e in w], ["translate_y", "translate_y"])
        self.assertEqual([e.get("on") for e in w], [None, "exit"])

    # ---- what must be rejected, at the point it is written -----------------------

    def test_rotate_and_scale_are_refused_with_the_reason(self):
        for ch in ("rotate", "scale", "scale_x", "scale_y"):
            with self.assertRaises(ValueError) as cm:
                Anim.enter(ms=200, **{ch: 90})
            msg = str(cm.exception)
            self.assertIn("painted by nothing", msg, ch)
            self.assertIn("§45.7", msg, ch)

    def test_an_outright_typo_lists_the_channels_that_do_work(self):
        with self.assertRaises(ValueError) as cm:
            Anim.enter(opactiy=1, ms=200)
        self.assertIn("opacity, translate_x, translate_y", str(cm.exception))

    def test_a_declaration_must_pick_exactly_one_curve(self):
        with self.assertRaises(ValueError) as cm:
            Anim.enter(opacity=1)
        self.assertIn("exactly one of ms=", str(cm.exception))
        with self.assertRaises(ValueError):
            Anim.enter(opacity=1, ms=100, spring=(180, 26))
        with self.assertRaises(ValueError) as cm:
            Anim.enter(opacity=1, spring=(180, 26), ease="out")
        self.assertIn("exclusive", str(cm.exception))

    def test_an_empty_or_nonsense_declaration_is_refused(self):
        with self.assertRaises(ValueError) as cm:
            Anim.enter(ms=200)
        self.assertIn("declares no channels", str(cm.exception))
        with self.assertRaises(ValueError):
            Anim.enter(opacity=1, ms=100, ease="bouncy")
        with self.assertRaises(ValueError):
            Anim.enter(opacity=1, ms=100, dir="backwards")
        with self.assertRaises(ValueError):
            Anim.enter(opacity=1, ms=100, repeat="twice")
        with self.assertRaises(ValueError):
            Anim("hover", opacity=1, ms=100)
        with self.assertRaises(ValueError) as cm:
            Anim.enter(opacity=(0, 0.5, 1), ms=100)
        self.assertIn("(from, to)", str(cm.exception))

    def test_a_forever_exit_is_refused_because_it_can_never_complete(self):
        # An exit's completion is what tells the app to drop the node. A forever
        # animation never completes, so the node stays alive and repainting for the
        # life of the process while `view()` waits for a callback that cannot arrive.
        with self.assertRaises(ValueError) as cm:
            Anim.exit_(opacity=0, ms=200, repeat="forever")
        self.assertIn("never completes", str(cm.exception))
        # The control: the identical spec on `enter` is a spinner, and is fine.
        Anim.enter(opacity=0, ms=200, repeat="forever")

    def test_anim_takes_an_anim_or_a_list_and_says_so_otherwise(self):
        with self.assertRaises(TypeError):
            Column("", anim="fade")._anim_wire()
        with self.assertRaises(TypeError):
            Column("", anim=[{"ch": "opacity"}])._anim_wire()
        self.assertEqual(Column("")._anim_wire(), [])

    # ---- reaching the tree ------------------------------------------------------

    def test_a_declaration_reaches_the_tree_on_a_plain_widget(self):
        t = Column("flex-1", Text("", "hi"), anim=Anim.enter(opacity=(0, 1), ms=200)).to_tree(
            RenderContext(), "r"
        )
        self.assertEqual(t["attrs"]["anim"], [{"ch": "opacity", "to": 1.0, "from": 0.0, "ms": 200.0}])
        # …and a widget with no declaration carries no key at all, so every existing
        # app's tree is byte-identical to what it was before this existed.
        self.assertNotIn("anim", Column("flex-1", Text("", "hi")).to_tree(RenderContext(), "r")["attrs"])

    def test_it_reaches_the_tree_on_widgets_that_build_their_own_attrs(self):
        # THE test this injection point exists for. Eleven widgets override
        # `_tree_attrs` and return a dict they construct themselves; merging `anim` in
        # at each of them is what a first draft does, and it works for exactly the
        # ones someone remembered. Merging at the single shared call site in
        # `Widget.to_tree` is what makes these pass without any of them knowing.
        cases = [
            Switch("", checked=True, anim=Anim.enter(opacity=(0, 1), ms=100)),
            Slider("", min=0, max=10, value=5, anim=Anim.enter(opacity=(0, 1), ms=100)),
            Button("", "go", anim=Anim.enter(opacity=(0, 1), ms=100)),
        ]
        for w in cases:
            t = w.to_tree(RenderContext(), "r")
            self.assertIn("anim", t["attrs"], type(w).__name__)
            # The widget's own attributes must survive the merge — this is the half
            # that a naive `attrs = {"anim": ...}` would break.
            self.assertGreater(len(t["attrs"]), 1, type(w).__name__)
        self.assertIs(cases[0].to_tree(RenderContext(), "r")["attrs"]["checked"], True)
        self.assertEqual(cases[1].to_tree(RenderContext(), "r")["attrs"]["value"], 5.0)

    # ---- the completion event ---------------------------------------------------

    def test_on_animend_registers_and_its_payload_is_typed(self):
        ctx = RenderContext()
        seen = []
        w = Column("", anim=Anim.exit_(opacity=0, ms=100), on_animend=seen.append)
        t = w.to_tree(ctx, "r")
        self.assertIn("animend", t["events"])
        # The bridge delivers a string on every backend (`RenderContext.register`).
        ctx.dispatch("r", "animend", '{"channel":"opacity","trigger":"exit"}')
        self.assertEqual(seen, [AnimEnd("opacity", "exit")])
        self.assertEqual(seen[0].trigger, "exit")

    def test_the_typed_payload_survives_widgets_that_override_the_parser(self):
        # `Switch` and `Slider` both override `_payload_parser` for `change`. An
        # override that returned `None` for everything else would silently un-type
        # `animend` on exactly those widgets, handing the handler raw JSON text.
        for w in (Switch("", checked=False, anim=Anim.enter(opacity=1, ms=10), on_animend=lambda v: v),
                  Slider("", value=1, anim=Anim.enter(opacity=1, ms=10), on_animend=lambda v: v)):
            self.assertIs(w._payload_parser("animend"), parse_anim_end, type(w).__name__)
            # …and the widget's own `change` typing is untouched.
            self.assertIsNotNone(w._payload_parser("change"))

    def test_a_malformed_completion_payload_degrades_rather_than_raising(self):
        # This crosses a bridge, and a handler raising inside the render loop is a
        # full-screen traceback. An empty `AnimEnd` is the one reading that cannot be
        # a wrong answer.
        self.assertEqual(parse_anim_end("not json"), AnimEnd("", ""))
        self.assertEqual(parse_anim_end(None), AnimEnd("", ""))
        self.assertEqual(parse_anim_end('{"channel":"opacity"}'), AnimEnd("opacity", ""))

    # ---- the divergence, made loud ----------------------------------------------

    def test_the_html_backend_says_once_that_it_cannot_animate(self):
        # Queue F part 2 is the native driver; the WebView port is layer 4 and does
        # not exist. That is a real §15.1 divergence — and this document's whole
        # record is that the dangerous ones are the *quiet* ones. Once per process,
        # not per render, because `view()` runs sixty times a second.
        w = Column("flex-1", Text("", "hi"), anim=Anim.enter(opacity=(0, 1), ms=200))
        captured = io.StringIO()
        with contextlib.redirect_stderr(captured):
            for _ in range(5):
                w.render(RenderContext(), "r")
        err = captured.getvalue()
        self.assertEqual(err.count("ZKITEANIM"), 1, err)
        self.assertIn("--backend native", err)
        # …and the attribute is not emitted into the DOM, since nothing there reads it.
        self.assertNotIn("anim", w.render(RenderContext(), "r"))

    def test_an_app_with_no_animation_says_nothing_on_either_backend(self):
        # The negative control for the warning: a silent path must stay silent, or
        # every existing app acquires log noise a device gate would have to filter.
        w = Column("flex-1", Text("", "hi"))
        captured = io.StringIO()
        with contextlib.redirect_stderr(captured):
            w.render(RenderContext(), "r")
            w.to_tree(RenderContext(), "r")
        self.assertEqual(captured.getvalue(), "")


if __name__ == "__main__":
    unittest.main()
