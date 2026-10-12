"""phase_3/PLAN.md §33 — `Overlay` (the layer) and `Select` (one composition over it).

Written against the documented signatures only, per §26.7's rule — which here means
driving a real `App` rather than calling the popup's own builder: an overlay is
*enlisted* where it is written and *rendered* by the app root, so a test that renders
a `Select` on its own and then dispatches to an option would be testing a shape no
backend ever produces. The first draft did exactly that and failed, which is the
useful half of the rule.

Two things are being asserted, and they differ in kind. The `Overlay` tests are about
*structure*: which node owns the dismiss handler, which node is positioned, and where
the layer is emitted — because "paints above everything" is a property of document
order on both backends and of nothing else. The `Select` tests are about *contract*:
the value that reaches `on_change` is the option's own, open state is the widget's
unless the app takes it, and every visual surface is a class string the caller owns.
"""

import json
import types
import unittest

import zkite._store as store_mod
from zkite.app import App
from zkite.widgets import Column, Overlay, RenderContext, Select, Text, Widget


def render(widget: Widget, ctx: RenderContext | None = None) -> str:
    return widget.render(ctx or RenderContext(), "r")


def tree(widget: Widget, ctx: RenderContext | None = None) -> dict:
    return widget.to_tree(ctx or RenderContext(), "r")


def app_from(src: str) -> App:
    store_mod.reset_registry()
    module = types.ModuleType("main")
    exec(src, module.__dict__)  # noqa: S102 - fixture source, not user input
    return App(module)


def select_app(props: str = "", store_body: str = "    country: str = 'in'") -> str:
    return (
        "from zkite import Column, Select, Text, store\n"
        "@store\n"
        "class S:\n"
        f"{store_body}\n"
        "def view(s: S):\n"
        "    return Column('flex-1 bg-slate-950 p-6',\n"
        "        Text('text-white', f'Country: {s.country}'),\n"
        "        Select('px-4 py-3 rounded-xl bg-slate-800 text-white',\n"
        "               options=[('br', 'Brazil'), ('in', 'India'), ('ke', 'Kenya')],\n"
        "               value=s.country,\n"
        "               on_change=lambda v: s.update(country=v)"
        f"{props}))\n"
    )


SELECT_APP = select_app()
TRIGGER = "r.1"


class StoreIsolated(unittest.TestCase):
    """`_SelectState` is a process-wide `@store` singleton, exactly like `Navigator`.

    One app per process on a device, so sharing it is right there; a leak between
    *tests* is not.
    """

    def setUp(self) -> None:
        store_mod.reset_registry()

    def opened(self, src: str = SELECT_APP, mode: str = "html", trigger: str = TRIGGER) -> App:
        app = app_from(src)
        app.render_tree() if mode == "tree" else app.render()
        app.dispatch(trigger, "tap", "")
        return app


class OverlayLayerTests(StoreIsolated):
    def test_an_overlay_occupies_no_space_where_it_is_written(self):
        # It is handed to the app root instead. A container that composes children
        # generically has to skip it rather than reserve it a slot, on both paths.
        col = Column("flex-1", Text("", "before"), Overlay("bg-slate-800", Text("", "in")))
        html = render(col)
        self.assertIn("before", html)
        self.assertNotIn("bg-slate-800", html)
        kids = tree(col)["children"]
        self.assertEqual(len(kids), 1)
        self.assertEqual(kids[0]["id"], "r.0")

    def test_no_overlay_open_means_no_root_wrapper_at_all(self):
        # The property the whole regression argument rests on: an app that opens
        # nothing emits exactly what it emitted before this feature existed.
        app = app_from(SELECT_APP)
        self.assertEqual(app.render_tree()["id"], "r")
        self.assertNotIn("__zkite_overlay", app.render())

    def test_the_layer_is_emitted_last_so_document_order_puts_it_on_top(self):
        # `zkite_renderer` paints in document order and hit-tests it in reverse; the
        # DOM does the same for positioned elements in tree order. Last *is* the
        # mechanism — there is no z-index anywhere in this.
        root = self.opened(mode="tree").last_tree
        self.assertEqual(root["id"], "__zkite_root")
        self.assertEqual(root["children"][0]["id"], "r")
        self.assertEqual(root["children"][-1]["id"], "__zkite_overlay.0")

        html = self.opened().last_html
        self.assertLess(html.index('id="r"'), html.index('id="__zkite_overlay.0"'))

    def test_the_view_root_keeps_its_id_when_a_layer_appears(self):
        # Scroll offsets, `List` windows and IME focus are all keyed by node id. If
        # opening a popup renumbered the tree, every one of them would reset.
        app = app_from(SELECT_APP)
        closed = json.dumps(app.render_tree())
        app.dispatch(TRIGGER, "tap", "")
        opened_view = app.last_tree["children"][0]
        self.assertEqual(opened_view["id"], "r")
        self.assertEqual(json.dumps(opened_view).count('"id": "r.0"'), closed.count('"id": "r.0"'))

    def test_the_panel_is_positioned_so_its_own_scrim_cannot_cover_it(self):
        # CSS paints positioned elements above non-positioned ones regardless of tree
        # order, so a non-positioned panel would sit correctly on native (document
        # order) and be hidden by its own scrim on the WebView. Implied by the widget
        # rather than left to the caller: a rule a caller can forget is a silent
        # divergence waiting to happen.
        layer = self.opened(mode="tree").last_tree["children"][-1]
        scrim, panel = layer["children"]
        self.assertIn("fixed", layer["classes"])
        self.assertIn("inset-0", layer["classes"])
        self.assertIn("absolute", scrim["classes"])
        self.assertIn("inset-0", scrim["classes"])
        self.assertIn("relative", panel["classes"])

    def test_the_scrim_owns_the_dismiss_and_the_panel_owns_nothing(self):
        # What lets a tap on the panel do nothing without the panel registering a
        # no-op `tap` to absorb one — which would have made a listbox announce itself
        # as clickable.
        app = self.opened()
        self.assertEqual(list(app.ctx.handlers["__zkite_overlay.0.0"]), ["tap"])
        self.assertNotIn("__zkite_overlay.0.1", app.ctx.handlers)
        self.assertNotIn("__zkite_overlay.0", app.ctx.handlers)
        app.dispatch("__zkite_overlay.0.0", "tap", "")
        self.assertNotIn("__zkite_overlay", app.last_html)

    def test_back_reaches_the_overlay_before_anything_outer(self):
        # Innermost-first falls out of *where the overlay was written* — §16.1's
        # layer 1, unchanged. Nothing here tracks depth.
        order = []
        ctx = RenderContext()
        ctx.register_back(lambda: order.append("outer") or False)
        Overlay("", on_dismiss=lambda: order.append("overlay")).render(ctx, "o")
        self.assertTrue(ctx.dispatch_back())
        self.assertEqual(order, ["overlay"])

    def test_back_closes_an_open_select_and_then_stops_consuming(self):
        app = self.opened()
        self.assertTrue(app.dispatch_back())
        self.assertNotIn("__zkite_overlay", app.last_html)
        self.assertFalse(app.dispatch_back())

    def test_an_overlay_with_no_on_dismiss_does_not_consume_back(self):
        ctx = RenderContext()
        Overlay("").render(ctx, "o")
        self.assertFalse(ctx.dispatch_back())

    def test_an_overlay_registered_while_a_layer_renders_is_still_emitted(self):
        # `app.App._overlay_layers` drains by index because rendering one layer can
        # register another. Iterating the list instead would silently drop the second,
        # which is what a `Select` inside a `Sheet` would be.
        app = app_from(
            "from zkite import Column, Overlay, Text\n"
            "from zkite.widgets import Widget\n"
            "class Nested(Widget):\n"
            "    def _render_children(self, ctx, node_id):\n"
            "        Overlay('second-layer', Text('', 'deep')).render(ctx, node_id + '.x')\n"
            "        return ''\n"
            "def view():\n"
            "    return Column('flex-1', Overlay('first-layer', Nested('')))\n"
        )
        html = app.render()
        self.assertIn('id="__zkite_overlay.0"', html)
        self.assertIn('id="__zkite_overlay.1"', html)
        self.assertLess(html.index("first-layer"), html.index("second-layer"))


class SelectTests(StoreIsolated):
    def test_the_trigger_shows_the_selected_label_and_falls_back_to_a_placeholder(self):
        self.assertIn("India", render(Select("", options=[("br", "Brazil"), ("in", "India")], value="in")))
        self.assertIn("Pick one", render(Select("", options=[("br", "Brazil")], value=None, placeholder="Pick one")))

    def test_bare_values_are_their_own_labels(self):
        self.assertIn("India", render(Select("", options=["Brazil", "India"], value="India")))

    def test_the_chosen_value_is_the_options_own_not_the_bridge_payload(self):
        # The regression this exists for. A tap's payload is `""` on *both* backends,
        # so a handler that took the payload would report that the user picked
        # nothing — §26.2's bug shape (`bool("false") is True`) a third time. The
        # value travels in a closure precisely because the payload has no room for it.
        app = self.opened()
        app.dispatch("r.1.opt.2", "tap", "")
        self.assertIn("Country: ke", app.last_html)

    def test_values_a_payload_round_trip_would_mangle(self):
        # `0` is falsy, `None` is what `RenderContext.dispatch` reads as "no payload",
        # and neither is a string. All three have to arrive intact.
        got = []
        app = app_from(
            "from zkite import Column, Select, store\n"
            "@store\n"
            "class S:\n"
            "    picked: object = 'unset'\n"
            "def view(s: S):\n"
            "    return Column('flex-1',\n"
            "        Select('', options=[(0, 'Zero'), (None, 'Nothing'), (7, 'Seven')],\n"
            "               value=s.picked, on_change=lambda v: s.update(picked=v)))\n"
        )
        for index, expected in ((0, 0), (1, None), (2, 7)):
            app.render_tree()
            app.dispatch("r.0", "tap", "")
            app.dispatch(f"r.0.opt.{index}", "tap", "")
            got.append(app.module.S.__zkite_store_qualname__ and store_mod.get_instance(app.module.S).picked)
        self.assertEqual(got, [0, None, 7])

    def test_a_handler_that_ignores_the_value_still_works(self):
        # Every callback in this framework may ignore its payload; `widgets.invoke` is
        # the one place that rule lives, so `Select` (whose value never crosses a
        # bridge) and `RenderContext.dispatch` cannot disagree about it.
        app = app_from(
            "from zkite import Column, Select, store\n"
            "@store\n"
            "class S:\n"
            "    n: int = 0\n"
            "def view(s: S):\n"
            "    return Column('flex-1',\n"
            "        Select('', options=['a'], value='a', on_change=lambda: s.update(n=s.n + 1)))\n"
        )
        app.render()
        app.dispatch("r.0", "tap", "")
        app.dispatch("r.0.opt.0", "tap", "")
        self.assertEqual(store_mod.get_instance(app.module.S).n, 1)

    def test_open_state_is_the_widgets_own_until_the_app_asks_for_it(self):
        # One store field for an ordinary picker. Requiring a second field and three
        # callbacks to show a dropdown is ceremony, not control.
        app = app_from(SELECT_APP)
        app.render()
        self.assertNotIn("__zkite_overlay", app.last_html)
        app.dispatch(TRIGGER, "tap", "")
        self.assertIn("__zkite_overlay", app.last_html)
        app.dispatch(TRIGGER, "tap", "")
        self.assertNotIn("__zkite_overlay", app.last_html)

    def test_a_controlled_select_never_touches_the_shared_state(self):
        from zkite.widgets import _SelectState

        opened = []
        ctx = RenderContext()
        s = Select("", options=["a"], value="a", open=False,
                   on_open=lambda: opened.append("open"),
                   on_dismiss=lambda: opened.append("dismiss"))
        html = s.render(ctx, "r")
        self.assertNotIn("__zkite_overlay", html)
        self.assertIn('aria-expanded="false"', html)
        ctx.dispatch("r", "tap", "")
        self.assertEqual(opened, ["open"])
        self.assertEqual(store_mod.get_instance(_SelectState).open_id, "")

    def test_a_controlled_select_that_says_it_is_open_renders_its_popup(self):
        app = app_from(
            "from zkite import Column, Select, store\n"
            "@store\n"
            "class S:\n"
            "    shown: bool = True\n"
            "def view(s: S):\n"
            "    return Column('flex-1',\n"
            "        Select('', options=['a'], value='a', open=s.shown,\n"
            "               on_dismiss=lambda: s.update(shown=False)))\n"
        )
        self.assertIn("__zkite_overlay", app.render())
        app.dispatch("__zkite_overlay.0.0", "tap", "")
        self.assertNotIn("__zkite_overlay", app.last_html)

    def test_choosing_closes_it(self):
        app = self.opened()
        app.dispatch("r.1.opt.0", "tap", "")
        self.assertNotIn("__zkite_overlay", app.last_html)

    def test_two_selects_cannot_be_open_at_once(self):
        app = app_from(
            "from zkite import Column, Select, store\n"
            "@store\n"
            "class S:\n"
            "    a: str = ''\n"
            "def view(s: S):\n"
            "    return Column('flex-1',\n"
            "        Select('', options=['x'], value=s.a),\n"
            "        Select('', options=['y'], value=s.a))\n"
        )
        app.render()
        app.dispatch("r.0", "tap", "")
        self.assertIn('id="r.0.opt.0"', app.last_html)
        app.dispatch("r.1", "tap", "")
        self.assertIn('id="r.1.opt.0"', app.last_html)
        self.assertNotIn('id="r.0.opt.0"', app.last_html)

    def test_every_visual_surface_is_a_class_string_the_caller_owns(self):
        # The centred default is a *default value*, not a geometry baked into the
        # widget: `items-end` is a bottom sheet, and the scrim can be dropped visually
        # while outside-tap dismissal keeps working.
        app = app_from(
            "from zkite import Column, Select\n"
            "def view():\n"
            "    return Column('flex-1',\n"
            "        Select('', options=['a'], value='a',\n"
            "               placement='items-end justify-stretch',\n"
            "               scrim='',\n"
            "               panel='rounded-t-3xl bg-white flex-col',\n"
            "               option_class='px-2 py-1 text-black',\n"
            "               option_selected_class='font-bold'))\n"
        )
        app.render()
        app.dispatch("r.0", "tap", "")
        html = app.last_html
        self.assertIn("items-end justify-stretch", html)
        self.assertIn('class="absolute inset-0"', html)
        self.assertIn("rounded-t-3xl bg-white flex-col relative flex", html)
        self.assertIn("px-2 py-1 text-black font-bold", html)
        # No dimming, and an outside tap still dismisses.
        self.assertNotIn("bg-black/50", html)
        app.dispatch("__zkite_overlay.0.0", "tap", "")
        self.assertNotIn("__zkite_overlay", app.last_html)

    def test_the_option_builder_replaces_the_drawing_and_keeps_the_semantics(self):
        # §32.5's `indicator=False` argument in another shape: the thing developers
        # reach for instead — a tappable `Row` — is announced to a screen reader as
        # generic clickable text, so the framework holds role/selected/tap and hands
        # back the drawing.
        app = app_from(
            "from zkite import Column, Select, Text, store\n"
            "@store\n"
            "class S:\n"
            "    v: str = 'b'\n"
            "def view(s: S):\n"
            "    return Column('flex-1',\n"
            "        Select('', options=[('a', 'Alpha'), ('b', 'Beta')], value=s.v,\n"
            "               option_class='',\n"
            "               option=lambda value, label, selected: Column(\n"
            "                   'border-2 ' + ('border-indigo-500' if selected else 'border-slate-700'),\n"
            "                   Text('text-lg', label)),\n"
            "               on_change=lambda v: s.update(v=v)))\n"
        )
        app.render()
        app.dispatch("r.0", "tap", "")
        html = app.last_html
        self.assertIn("border-indigo-500", html)
        self.assertIn("border-slate-700", html)
        self.assertIn('role="option"', html)
        self.assertIn('aria-selected="true"', html)
        app.dispatch("r.0.opt.0", "tap", "")
        self.assertEqual(store_mod.get_instance(app.module.S).v, "a")

    def test_a_header_sits_above_the_option_list(self):
        # The slot a search-as-you-type picker needs: the header is an ordinary
        # widget, so a `TextField` in it is an ordinary text field.
        app = app_from(
            "from zkite import Column, Select, TextField\n"
            "def view():\n"
            "    return Column('flex-1',\n"
            "        Select('', options=['a'], value='a',\n"
            "               header=TextField('bg-slate-900', placeholder='Search')))\n"
        )
        app.render_tree()
        app.dispatch("r.0", "tap", "")
        panel = app.last_tree["children"][-1]["children"][1]
        self.assertEqual(panel["children"][0]["attrs"].get("input_type"), "text")
        self.assertEqual(panel["children"][1]["attrs"].get("role"), "listbox")

    def test_children_replace_the_default_trigger_content(self):
        html = render(Select("", Text("font-bold", "Custom"), options=["a"], value="a"))
        self.assertIn("Custom", html)
        self.assertNotIn("icon-chevron-down", html)

    def test_the_roles_and_state_are_emitted_on_both_paths(self):
        # `zkite_pybridge::a11y_role`/`build_a11y_node` read exactly these, and
        # Chromium reads the ARIA spellings. §30.1's role-not-marker principle.
        s = Select("", options=[("a", "Alpha"), ("b", "Beta")], value="b")
        html = render(s)
        self.assertIn('role="combobox"', html)
        self.assertIn('aria-haspopup="listbox"', html)
        self.assertIn('aria-expanded="false"', html)
        self.assertEqual(tree(s)["attrs"], {"role": "combobox", "expanded": False})

        app = self.opened(mode="tree")
        trigger = app.last_tree["children"][0]["children"][1]
        self.assertEqual(trigger["attrs"], {"role": "combobox", "expanded": True})
        listbox = app.last_tree["children"][-1]["children"][1]["children"][0]
        self.assertEqual(listbox["attrs"]["role"], "listbox")
        self.assertEqual(
            [(o["attrs"]["role"], o["attrs"]["selected"]) for o in listbox["children"]],
            [("option", False), ("option", True), ("option", False)],
        )

    def test_the_trigger_owns_exactly_one_event(self):
        # `on_change` belongs to the option rows. Registering it here too would give a
        # tap on the trigger a second meaning.
        ctx = RenderContext()
        Select("", options=["a"], value="a", on_change=lambda v: None).render(ctx, "r")
        self.assertEqual(list(ctx.handlers["r"]), ["tap"])

    def test_an_option_id_is_tied_to_its_select_not_to_the_layer(self):
        # So the popup of the *second* `Select` on a screen cannot collide with the
        # first's, and so an id stays stable while a layer index moves.
        app = self.opened()
        self.assertIn('id="r.1.opt.0"', app.last_html)


if __name__ == "__main__":
    unittest.main()
