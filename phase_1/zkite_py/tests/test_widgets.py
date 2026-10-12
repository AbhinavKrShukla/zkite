import re
import unittest

from zkite.widgets import (
    Button,
    Checkbox,
    Column,
    Grid,
    Icon,
    Image,
    Radio,
    RenderContext,
    Row,
    ScrollView,
    Slider,
    Stack,
    Switch,
    Text,
    TextField,
    Widget,
)


def render(widget: Widget, ctx: RenderContext | None = None) -> str:
    return widget.render(ctx or RenderContext(), "r")


class WidgetRenderTests(unittest.TestCase):
    def test_column_implies_flex_col_and_is_additive(self):
        html = render(Column("gap-6 bg-slate-950"))
        self.assertIn('class="flex flex-col gap-6 bg-slate-950"', html)
        self.assertTrue(html.startswith('<div id="r"'))
        self.assertTrue(html.endswith("</div>"))

    def test_row_implies_flex_row(self):
        html = render(Row("gap-2"))
        self.assertIn('class="flex flex-row gap-2"', html)

    def test_grid_implies_grid(self):
        html = render(Grid("grid-cols-3 gap-4"))
        self.assertIn('class="grid grid-cols-3 gap-4"', html)

    def test_empty_class_string_has_no_class_attr(self):
        html = render(Column())
        self.assertIn('class="flex flex-col"', html)
        html2 = render(Text())
        self.assertNotIn("class=", html2)

    def test_text_renders_escaped_content(self):
        html = render(Text("text-7xl", "<script>1</script>"))
        self.assertIn("&lt;script&gt;1&lt;/script&gt;", html)
        self.assertNotIn("<script>1</script>", html)

    def test_text_content_first_ambiguity_resolved_class_first(self):
        # class string is always arg 0 (see widgets.py module docstring)
        html = render(Text("text-7xl font-bold text-white tabular-nums", "42"))
        self.assertIn('class="text-7xl font-bold text-white tabular-nums"', html)
        self.assertIn(">42<", html)

    def test_button_has_type_button_and_registers_on_tap(self):
        calls = []
        ctx = RenderContext()
        html = render(Button("px-6 py-3", "Increment", on_tap=lambda: calls.append(1)), ctx)
        self.assertIn('type="button"', html)
        self.assertIn('data-zkite-on="tap"', html)
        self.assertTrue(ctx.dispatch("r", "tap"))
        self.assertEqual(calls, [1])

    def test_button_implies_flex_centering(self):
        # Browsers center a plain <button>'s content by default (both axes), via
        # internal UA layout independent of author CSS. The native renderer (phase 3)
        # has no UA stylesheet to inherit that from, so it must be explicit here or
        # the two backends silently diverge on every button — see widgets.py's
        # comment on Button.implied_classes and phase_3/PLAN.md §3.
        html = render(Button("px-6 py-3", "Increment"))
        self.assertIn('class="appearance-none flex items-center justify-center px-6 py-3"', html)

    def test_button_disabled_true_renders_boolean_attr(self):
        html = render(Button("", "x", disabled=True))
        self.assertIn(" disabled", html)

    def test_button_disabled_false_omits_attr(self):
        html = render(Button("", "x", disabled=False))
        self.assertNotIn("disabled", html)

    def test_stack_wraps_children_for_overlay(self):
        html = render(Stack("", Text("", "a"), Text("", "b")))
        self.assertEqual(html.count('class="col-start-1 row-start-1"'), 2)

    def test_scrollview_implies_overflow_auto(self):
        html = render(ScrollView("h-full"))
        self.assertIn('class="flex flex-col overflow-auto min-h-0 h-full"', html)

    def test_scrollview_horizontal_swaps_the_whole_implied_set(self):
        """`horizontal=True` replaces the implied set rather than appending to it.

        Appending would leave `flex-col` and `flex-row` (and `overflow-auto` and
        `overflow-x-auto`) both present, which `twcore` resolves last-wins and real
        Tailwind resolves by stylesheet order — a silent per-backend divergence.
        See `ScrollView.__init__`'s comment.
        """
        html = render(ScrollView("h-full", horizontal=True))
        self.assertIn('class="flex flex-row overflow-x-auto min-w-0 h-full"', html)
        self.assertNotIn("flex-col", html)
        self.assertNotIn("overflow-auto", html)

    def test_scrollview_horizontal_does_not_leak_into_attrs(self):
        """`horizontal` is a constructor flag, not a prop — it must not reach the DOM."""
        html = render(ScrollView("", horizontal=True))
        self.assertNotIn("horizontal", html)
        self.assertEqual(tree(ScrollView("", horizontal=True))["attrs"], {})

    def test_scrollview_horizontal_declares_only_the_x_axis(self):
        """A strip must leave `overflow-y` alone so a vertical drag passes through it.

        `overflow-auto` would declare both axes, and the native gesture layer resolves
        a scroll target per axis — a strip claiming Y would swallow vertical drags
        meant for a list underneath it while having no vertical room to satisfy them.
        """
        classes = tree(ScrollView("", horizontal=True))["classes"]
        self.assertIn("overflow-x-auto", classes)
        self.assertNotIn("overflow-y", classes)

    def test_textfield_is_void_input_with_value_and_placeholder(self):
        html = render(TextField("border", value="hi", placeholder="Search"))
        self.assertTrue(html.startswith("<input"))
        self.assertNotIn("</input>", html)
        self.assertIn('type="text"', html)
        self.assertIn('value="hi"', html)
        self.assertIn('placeholder="Search"', html)

    def test_textfield_secure_becomes_password(self):
        html = render(TextField("", secure=True))
        self.assertIn('type="password"', html)

    def test_image_renders_src_and_alt(self):
        html = render(Image("rounded-full", src="a.png", alt="avatar"))
        self.assertIn('src="a.png"', html)
        self.assertIn('alt="avatar"', html)
        self.assertTrue(html.startswith("<img"))

    def test_an_authored_object_fit_replaces_the_implied_object_cover(self):
        # phase_3/PLAN.md §35.4 — a real backend divergence, not tidiness.
        #
        # `Widget.classes` emits `implied_classes` first, so before this fix an
        # `Image("object-contain")` carried BOTH classes. Native resolves in string
        # order (last wins → contain); the WebView resolves by CSS cascade, and
        # `.object-cover` is emitted later in Tailwind's stylesheet, so it won there
        # regardless of attribute order. Measured in real Chromium against the
        # vendored bundle: `"object-cover object-contain"` and
        # `"object-contain object-cover"` BOTH compute `object-fit: cover`.
        #
        # Asserted on both output paths, because the divergence was between them.
        for mode in ("object-contain", "object-fill", "object-none", "object-scale-down"):
            with self.subTest(mode=mode):
                html = render(Image(f"w-8 h-8 {mode}", src="a.png"))
                self.assertIn(mode, html)
                self.assertNotIn("object-cover", html, "the implied class must not survive")
                t = tree(Image(f"w-8 h-8 {mode}", src="a.png"))
                self.assertIn(mode, t["classes"])
                self.assertNotIn("object-cover", t["classes"])

        # And the default is untouched: an `Image` that says nothing still covers, on
        # both paths. Dropping the implied class unconditionally would have passed
        # every assertion above.
        self.assertIn("object-cover", render(Image("w-8 h-8", src="a.png")))
        self.assertIn("object-cover", tree(Image("w-8 h-8", src="a.png"))["classes"])

    def test_icon_renders_use_element_with_name(self):
        html = render(Icon("w-5 h-5", "check"))
        self.assertIn("<svg", html)
        self.assertIn('href="#icon-check"', html)

    def test_switch_checked_true_renders_boolean_attr(self):
        html = render(Switch("", checked=True))
        self.assertIn('type="checkbox"', html)
        self.assertIn(" checked", html)

    def test_switch_checked_false_omits_attr(self):
        html = render(Switch("", checked=False))
        self.assertNotIn("checked", html)

    def test_slider_renders_range_attrs(self):
        html = render(Slider("", min=0, max=10, step=2, value=4))
        self.assertIn('type="range"', html)
        self.assertIn('min="0"', html)
        self.assertIn('max="10"', html)
        self.assertIn('step="2"', html)
        self.assertIn('value="4"', html)

    def test_slider_renders_zkite_pct_custom_property_for_webruntime_fill_css(self):
        # phase_3/PLAN.md §9.2 item 2: webruntime/index.html's `::-webkit-slider-
        # runnable-track` reads this custom property to paint the filled (indigo)
        # portion up to the current value — without it the WebView backend's Slider
        # has no way to know how far to fill.
        html = render(Slider("", min=0, max=10, value=4))
        self.assertIn('style="--zkite-pct: 40.0000%"', html)

    def test_nested_children_get_stable_path_ids(self):
        html = render(Column("", Row("", Text("", "a")), Text("", "b")))
        self.assertIn('id="r.0"', html)
        self.assertIn('id="r.0.0"', html)
        self.assertIn('id="r.1"', html)

    def test_explicit_key_overrides_path_id(self):
        html = render(Column("", Text("", "a", key="item-1")))
        self.assertIn('id="item-1"', html)
        self.assertNotIn('id="r.0"', html)

    def test_none_and_false_children_are_dropped(self):
        html = render(Column("", None, False, Text("", "a"), [None, Text("", "b")]))
        self.assertEqual(html.count("<span"), 2)

    def test_event_prop_not_leaked_as_html_attribute(self):
        html = render(Button("", "x", on_tap=lambda: None))
        self.assertNotIn("on_tap", html)
        self.assertIn("data-zkite-on", html)

    def test_dispatch_passes_payload_only_if_handler_accepts_it(self):
        received = []
        ctx = RenderContext()
        render(TextField("", on_change=lambda v: received.append(v)), ctx)
        ctx.dispatch("r", "change", "hello")
        self.assertEqual(received, ["hello"])

    def test_dispatch_unknown_returns_false(self):
        ctx = RenderContext()
        render(Text("", "a"), ctx)
        self.assertFalse(ctx.dispatch("r", "tap"))


class ChangePayloadTypeTests(unittest.TestCase):
    """The `change` payload's *type*, which both backends got wrong (phase_3/PLAN.md §26).

    Every backend's event bridge is string-in/string-out, so before §26 a `Switch`
    handler received the string `"false"` — and `bool("false")` is `True`, so the
    canonical `Switch(checked=s.on, on_change=lambda v: s.update(on=v))` could be
    turned on and never off. These assert the *type*, not just the value, because a
    truthy string passes any value-only check.

    Payloads here are spelled exactly as the two bridges spell them: JavaScript's
    `String(el.checked)` and Rust's `bool::to_string` both produce `"true"`/`"false"`.
    """

    def _payload(self, widget, payload):
        received = []
        widget.props["on_change"] = lambda v: received.append(v)
        ctx = RenderContext()
        widget.render(ctx, "r")
        self.assertTrue(ctx.dispatch("r", "change", payload))
        return received[0]

    def test_switch_change_payload_is_a_real_bool(self):
        for wire, expected in (("true", True), ("false", False)):
            with self.subTest(wire=wire):
                got = self._payload(Switch("", checked=False), wire)
                self.assertIsInstance(got, bool)
                self.assertIs(got, expected)

    def test_switch_off_is_falsey_which_the_raw_string_was_not(self):
        # The regression itself, stated as the property that actually broke.
        self.assertTrue(bool("false"))
        self.assertFalse(self._payload(Switch("", checked=True), "false"))

    def test_switch_accepts_a_real_bool_unchanged(self):
        self.assertIs(self._payload(Switch("", checked=False), True), True)

    def test_slider_change_payload_is_a_real_float(self):
        got = self._payload(Slider("", min=0, max=100, value=0), "42")
        self.assertIsInstance(got, float)
        self.assertEqual(got, 42.0)

    def test_slider_unparseable_payload_means_no_change_not_zero(self):
        # A broken bridge must not silently move the user's data to 0.
        got = self._payload(Slider("", min=0, max=100, value=37), "not-a-number")
        self.assertEqual(got, 37.0)

    def test_textfield_change_payload_stays_a_string(self):
        got = self._payload(TextField(""), "hello")
        self.assertIsInstance(got, str)
        self.assertEqual(got, "hello")

    def test_both_backends_register_the_same_parser(self):
        # `_register_events` is shared by `render()` and `to_tree()`, which is what
        # makes "typed on one backend" impossible — the property, asserted rather
        # than argued.
        for build in (lambda w, c: w.render(c, "r"), lambda w, c: w.to_tree(c, "r")):
            with self.subTest(build=build):
                received = []
                w = Switch("", checked=True, on_change=lambda v: received.append(v))
                ctx = RenderContext()
                build(w, ctx)
                ctx.dispatch("r", "change", "false")
                self.assertEqual(received, [False])


def tree(widget: Widget, ctx: RenderContext | None = None) -> dict:
    return widget.to_tree(ctx or RenderContext(), "r")


class WidgetTreeTests(unittest.TestCase):
    """`to_tree` (phase_3/PLAN.md §5) mirrors `render`'s node-id/event-registration
    logic exactly, so most of these are the tree-shaped counterpart of a test above —
    same widget, same assertion in spirit, different output format. `_tree_classes`
    genuinely diverges from `classes()` for `Stack`/`Icon` (see widgets.py); those get
    their own tests below rather than a mirrored one.
    """

    def test_column_implies_flex_col_in_tree_too(self):
        t = tree(Column("gap-6 bg-slate-950"))
        self.assertEqual(t["tag"], "div")
        self.assertEqual(t["id"], "r")
        self.assertEqual(t["classes"], "flex flex-col gap-6 bg-slate-950")
        self.assertEqual(t["children"], [])

    def test_bare_string_child_becomes_text_node(self):
        t = tree(Text("text-7xl font-bold text-white tabular-nums", "42"))
        self.assertEqual(t["children"], [{"tag": "#text", "text": "42"}])

    def test_button_tree_attrs_and_events(self):
        calls = []
        ctx = RenderContext()
        t = tree(Button("px-6 py-3", "Increment", on_tap=lambda: calls.append(1)), ctx)
        self.assertEqual(t["tag"], "button")
        self.assertEqual(t["events"], ["tap"])
        self.assertEqual(t["attrs"], {"disabled": False})
        self.assertTrue(ctx.dispatch("r", "tap"))
        self.assertEqual(calls, [1])

    def test_button_implies_flex_centering_in_tree_too(self):
        t = tree(Button("px-6 py-3", "Increment"))
        self.assertEqual(t["classes"], "appearance-none flex items-center justify-center px-6 py-3")

    def test_stack_tree_uses_absolute_overlay_not_grid(self):
        # `col-start-1`/`row-start-1` (the HTML mechanism) isn't a `twcore` utility at
        # all — see widgets.py's `Stack._tree_classes` comment and
        # phase_3/renderer/src/lib.rs's `expected_html_only_classes_are_unknown`-style
        # sweep. The native tree must never emit them.
        t = tree(Stack("", Text("", "a"), Text("", "b")))
        self.assertEqual(t["classes"], "relative")
        self.assertEqual(len(t["children"]), 2)
        for wrapper in t["children"]:
            self.assertEqual(wrapper["tag"], "div")
            # `flex flex-col` is not decoration and must not be "tidied" away: the HTML
            # wrapper is a grid item, i.e. `display:block`, so its child fills the
            # width. taffy has no block mode and defaults to `flex`/`row`, which sized
            # a `Stack` child to its *content* instead — so a child with no explicit
            # `w-` filled the stack on the WebView and did not on native. Same class of
            # bug as `ScrollView`'s own `flex-col`, and the third instance of it.
            # See widgets.py's comment and phase_3/PLAN.md §31.2.
            self.assertEqual(wrapper["classes"], "absolute inset-0 flex flex-col")
            self.assertEqual(len(wrapper["children"]), 1)

    def test_scrollview_implies_overflow_auto_in_tree_too(self):
        t = tree(ScrollView("h-full"))
        self.assertEqual(t["classes"], "flex flex-col overflow-auto min-h-0 h-full")

    def test_textfield_tree_attrs_no_synthetic_children(self):
        t = tree(TextField("border", value="hi", placeholder="Search"))
        self.assertEqual(t["tag"], "input")
        self.assertEqual(t["attrs"], {"input_type": "text", "value": "hi", "placeholder": "Search"})
        self.assertEqual(t["children"], [])

    def test_textfield_secure_becomes_password_in_tree_too(self):
        t = tree(TextField("", secure=True))
        self.assertEqual(t["attrs"]["input_type"], "password")

    def test_image_tree_attrs(self):
        t = tree(Image("rounded-full", src="a.png", alt="avatar"))
        self.assertEqual(t["attrs"], {"src": "a.png", "alt": "avatar"})
        self.assertEqual(t["children"], [])

    def test_icon_tree_emits_no_size_so_the_renderer_can_supply_a_real_em(self):
        # `w-[1em]`/`h-[1em]`/`fill-current` (the HTML implied_classes) aren't `twcore`
        # utilities and are deliberately *not* substituted with anything here.
        #
        # This used to emit a hardcoded `w-[20px] h-[20px]`, which is a correct `1em`
        # only when the surrounding text is 20px. Emitting no size at all is what lets
        # `zkite_renderer::Renderer::icon` default each `auto` axis to the genuinely
        # inherited font size — see widgets.py's `Icon._tree_classes` comment and
        # phase_3/PLAN.md §18. Asserting the *absence* of a size is the point of this
        # test: reintroducing any hardcoded box here would silently break icon sizing at
        # every font size except one, and would look like a harmless default.
        t = tree(Icon("w-5 h-5", "check"))
        self.assertEqual(t["classes"], "inline-block w-5 h-5")
        self.assertNotIn("[20px]", t["classes"])
        self.assertEqual(t["attrs"], {"icon_name": "check"})
        self.assertEqual(t["children"], [])

    def test_icon_with_no_classes_at_all_emits_no_size(self):
        # The default case, and the one a `zkite new` app hits: nothing but the sprite
        # name. Layout comes entirely from the renderer's 1em default.
        t = tree(Icon("", "star"))
        self.assertEqual(t["classes"], "inline-block")

    def test_switch_tree_reflects_checked_state(self):
        t_on = tree(Switch("", checked=True))
        self.assertIn("bg-indigo-500", t_on["classes"])
        self.assertIn("justify-end", t_on["classes"])
        # `role` joined `checked` in §30, when `Checkbox`/`Radio` made "an input
        # carrying `checked`" ambiguous. Still asserted exactly rather than with
        # `assertIn`: this attr set is what `zkite_pybridge::control_of` and
        # `a11y_role` both read, so a silent addition here changes native behaviour.
        self.assertEqual(t_on["attrs"], {"role": "switch", "checked": True})
        self.assertEqual(len(t_on["children"]), 1)
        self.assertEqual(t_on["children"][0]["classes"], "w-[20px] h-[20px] rounded-full bg-white")

        t_off = tree(Switch("", checked=False))
        self.assertIn("bg-slate-700", t_off["classes"])
        self.assertIn("justify-start", t_off["classes"])

    def test_slider_tree_positions_fill_and_thumb_by_percent(self):
        t = tree(Slider("", min=0, max=10, value=4))
        track, thumb = t["children"]
        self.assertEqual(t["attrs"], {"min": 0.0, "max": 10.0, "step": 1.0, "value": 4.0})
        fill = track["children"][0]
        self.assertIn("w-[40.0000%]", fill["classes"])  # (4-0)/(10-0) = 40%
        self.assertIn("left-[40.0000%]", thumb["classes"])

    def test_slider_percent_clamps_when_min_equals_max(self):
        t = tree(Slider("", min=5, max=5, value=5))
        fill = t["children"][0]["children"][0]
        self.assertIn("w-[0.0000%]", fill["classes"])

    def test_nested_children_get_stable_path_ids_in_tree_too(self):
        t = tree(Column("", Row("", Text("", "a")), Text("", "b")))
        self.assertEqual(t["children"][0]["id"], "r.0")
        self.assertEqual(t["children"][0]["children"][0]["id"], "r.0.0")
        self.assertEqual(t["children"][1]["id"], "r.1")

    def test_explicit_key_overrides_path_id_in_tree_too(self):
        t = tree(Column("", Text("", "a", key="item-1")))
        self.assertEqual(t["children"][0]["id"], "item-1")

    def test_none_and_false_children_are_dropped_in_tree_too(self):
        t = tree(Column("", None, False, Text("", "a"), [None, Text("", "b")]))
        self.assertEqual(len(t["children"]), 2)

    def test_render_and_to_tree_assign_identical_node_ids(self):
        # Load-bearing for the native backend's touch dispatch: a tap resolved against
        # the tree built by `to_tree` must land on the *same* node id `render`'s HTML
        # would have used, since both walks register handlers into the same
        # `RenderContext` by node id (`_register_events`, shared by both).
        widget = Column("", Row("", Button("", "go", on_tap=lambda: None)), Text("", "x"))
        html_ids = set(re.findall(r'id="([^"]*)"', render(widget)))
        t = tree(widget)

        def collect_ids(node: dict, out: set[str]) -> None:
            if node["tag"] == "#text":
                return
            out.add(node["id"])
            for c in node["children"]:
                collect_ids(c, out)

        tree_ids = set()
        collect_ids(t, tree_ids)
        self.assertEqual(html_ids, tree_ids)


class CheckboxAndRadioTests(unittest.TestCase):
    """phase_3/PLAN.md §30. Written against the documented signature only —
    `Checkbox(cls, checked=, on_change=)` — per §26.7's rule, which exists because
    `demo_app/main.py` wired its `Switch` with an undocumented `on_tap` and that is how
    divergence 6 survived three phases of device runs."""

    def test_the_three_toggles_are_distinguishable_by_role_on_both_paths(self):
        # The whole design rests on this: `tag` and `checked` are identical across all
        # three, so `role` is the only discriminator either backend has.
        for widget, role, dom_type in (
            (Switch("", checked=True), "switch", "checkbox"),
            (Checkbox("", checked=True), "checkbox", "checkbox"),
            (Radio("", checked=True), "radio", "radio"),
        ):
            with self.subTest(role=role):
                html = render(widget)
                self.assertIn(f'role="{role}"', html)
                self.assertIn(f'type="{dom_type}"', html)
                self.assertEqual(tree(widget)["attrs"]["role"], role)

    def test_checkbox_shows_a_tick_only_when_checked(self):
        checked = tree(Checkbox("", checked=True))
        self.assertEqual([c["attrs"].get("icon_name") for c in checked["children"]], ["check"])
        self.assertEqual(tree(Checkbox("", checked=False))["children"], [])

    def test_radio_shows_a_dot_only_when_checked(self):
        self.assertEqual(len(tree(Radio("", checked=True))["children"]), 1)
        self.assertEqual(tree(Radio("", checked=False))["children"], [])

    def test_the_tick_uses_the_shared_icon_sprite_not_a_second_drawing(self):
        # `Icon`'s sprite table is transcribed from index.html's own <symbol> sheet by
        # renderer/scripts/gen_icons.py (§18.2), so a tick emitted as an `svg` leaf with
        # `icon_name` cannot drift from the WebView's. A hand-rolled path here would be
        # exactly the two-hand-maintained-copies shape §9.5/§9.7/§28.1 each paid for.
        tick = tree(Checkbox("", checked=True))["children"][0]
        self.assertEqual(tick["tag"], "svg")
        self.assertEqual(tick["attrs"]["icon_name"], "check")

    def test_payload_is_a_real_bool_on_both_new_widgets(self):
        # §26.2: the bug was that this arrived as a *string*, and `bool("false")` is
        # `True`, so a control could be turned on and never off.
        for cls in (Checkbox, Radio):
            with self.subTest(widget=cls.__name__):
                got = []
                ctx = RenderContext()
                render(cls("", checked=False, on_change=lambda v: got.append(v)), ctx)
                ctx.dispatch("r", "change", "true")
                ctx.dispatch("r", "change", "false")
                self.assertEqual(got, [True, False])
                self.assertTrue(all(isinstance(v, bool) for v in got))

    def test_checkbox_does_not_inherit_the_switch_track(self):
        # The regression that motivated scoping index.html's CSS to [role=switch]: a
        # Checkbox must not be 44x24 with a sliding thumb.
        self.assertIn("w-[20px] h-[20px]", Checkbox("", checked=True).classes())
        self.assertNotIn("w-[44px]", Checkbox("", checked=True).classes())
        self.assertIn("w-[44px] h-[24px]", Switch("", checked=True).classes())


class LabelledControlTests(unittest.TestCase):
    """phase_3/PLAN.md §32 — `Checkbox("", "Wi-Fi", checked=..., on_change=...)`.

    Written against the documented signature only, per §26.7's rule.

    The feature is a *composition*, so most of what has to hold is about which node
    owns what: one handler, on the container; the box presentational on both paths;
    the box itself drawn from the same declaration as the bare form.
    """

    def test_a_label_makes_the_whole_control_one_tap_target(self):
        # The point of the feature. §30.8: in `[x] Wi-Fi`, tapping the text did
        # nothing — the live area was a 20x20dp box.
        got = []
        ctx = RenderContext()
        c = Checkbox("", "Wi-Fi", checked=False, on_change=lambda v: got.append(v))
        html = c.render(ctx, "c")
        # One handler, and it is a `tap` on the container — not a `change` on a box.
        self.assertIn('data-zkite-on="tap"', html)
        self.assertEqual(list(ctx.handlers["c"]), ["tap"])
        # `""` is what *both* backends send as a tap payload (render_thread.rs's
        # `dispatch_tap_to` and zkite-runtime.js's `extractPayload` for a click).
        ctx.dispatch("c", "tap", "")
        self.assertEqual(got, [True])
        self.assertTrue(all(isinstance(v, bool) for v in got))

    def test_the_labelled_form_registers_no_change_event(self):
        # There is exactly one handler, which is what makes §30.8's ordering trap
        # unreachable: with no second handler there is no order to get wrong.
        ctx = RenderContext()
        Checkbox("", "Wi-Fi", checked=True, on_change=lambda v: None).render(ctx, "c")
        self.assertNotIn("change", ctx.handlers["c"])

    def test_a_labelled_checkbox_toggles_and_a_labelled_radio_always_sends_true(self):
        # The one behavioural difference between the two, restated on the labelled
        # path. Its bare-form twin is `zkite_pybridge::Control::Radio` (native) and the
        # DOM's own behaviour (WebView); §30.1 explains why a radio cannot untick.
        for cls, checked, expected in (
            (Checkbox, False, True),
            (Checkbox, True, False),
            (Radio, False, True),
            (Radio, True, True),
        ):
            with self.subTest(widget=cls.__name__, checked=checked):
                got = []
                ctx = RenderContext()
                cls("", "L", checked=checked, on_change=lambda v: got.append(v)).render(ctx, "c")
                ctx.dispatch("c", "tap", "")
                self.assertEqual(got, [expected])

    def test_the_box_is_presentational_on_both_paths(self):
        # What stops the control being dispatched to twice, and what stops a screen
        # reader announcing two checkboxes. `zkite_pybridge::control_of`/`a11y_role`
        # read exactly these.
        c = Checkbox("", "Wi-Fi", checked=True, on_change=lambda v: None)
        self.assertIn('aria-hidden="true"', render(c))
        box = tree(c)["children"][0]
        self.assertEqual(box["attrs"]["aria_hidden"], True)
        self.assertEqual(box["events"], [])

    def test_the_container_carries_the_role_and_the_state_on_both_paths(self):
        for cls, role in ((Switch, "switch"), (Checkbox, "checkbox"), (Radio, "radio")):
            with self.subTest(widget=cls.__name__):
                w = cls("", "L", checked=True, on_change=lambda v: None)
                html = render(w)
                self.assertIn(f'role="{role}"', html)
                self.assertIn('aria-checked="true"', html)
                node = tree(w)
                self.assertEqual(node["attrs"], {"role": role, "checked": True})
                self.assertEqual(node["tag"], "span")

    def test_the_box_is_drawn_from_the_same_declaration_as_the_bare_form(self):
        # The property that makes a labelled control and a bare one impossible to
        # drift apart visually — the same reason §30.1 made these subclasses rather
        # than copies. The labelled box adds only layout (`shrink-0 mr-2`).
        for cls in (Switch, Checkbox, Radio):
            for checked in (True, False):
                with self.subTest(widget=cls.__name__, checked=checked):
                    bare = cls("", checked=checked).classes()
                    labelled_box = cls("", "L", checked=checked)._box_full_classes()
                    self.assertEqual(labelled_box, f"{bare} shrink-0 mr-2")

    def test_the_label_is_ordinary_content(self):
        # A bare string folds into a `#text`; a widget child renders as itself. Both
        # are just children, which is why nothing special had to be built for either.
        plain = tree(Checkbox("", "Wi-Fi", checked=False))
        self.assertEqual(plain["children"][1], {"tag": "#text", "text": "Wi-Fi"})
        styled = tree(Checkbox("", Text("font-bold", "High"), checked=False))
        self.assertEqual(styled["children"][1]["tag"], "span")
        self.assertEqual(styled["children"][1]["classes"], "font-bold")

    def test_the_bare_form_is_untouched_by_all_of_this(self):
        # The regression guard for the whole section: no label means the pre-§32
        # rendering, on both paths, including the event it registers.
        ctx = RenderContext()
        bare = Checkbox("", checked=False, on_change=lambda v: None)
        html = bare.render(ctx, "c")
        self.assertTrue(html.startswith('<input id="c"'))
        self.assertNotIn("aria-hidden", html)
        self.assertEqual(list(ctx.handlers["c"]), ["change"])
        self.assertEqual(tree(bare)["tag"], "input")

    def test_indicator_false_drops_the_box_but_keeps_everything_that_matters(self):
        # The MCQ / segmented-control / swatch case: the caller shows the selection by
        # styling the option itself. What must survive is exactly what a tappable `Row`
        # would have thrown away — the role, the state and the accessible name.
        #
        # All three controls, not just `Radio`: the flag lives on `Switch` and the
        # other two inherit it, so covering only one would leave "it happens to work"
        # untested on the two that matter most for a segmented control.
        for cls, role in ((Switch, "switch"), (Checkbox, "checkbox"), (Radio, "radio")):
            with self.subTest(widget=cls.__name__):
                got = []
                ctx = RenderContext()
                w = cls("px-4 py-3 rounded-xl border-2 border-indigo-500", "B. Mitochondria",
                        checked=True, indicator=False, on_change=lambda v: got.append(v))
                html = w.render(ctx, "q.1")
                self.assertNotIn("<input", html)
                self.assertIn(f'role="{role}"', html)
                self.assertIn('aria-checked="true"', html)
                self.assertIn("B. Mitochondria", html)

                node = w.to_tree(RenderContext(), "q.1")
                self.assertEqual(node["attrs"], {"role": role, "checked": True})
                self.assertEqual([c["tag"] for c in node["children"]], ["#text"])
                self.assertEqual(node["events"], ["tap"])

                ctx.dispatch("q.1", "tap", "")
                # `Radio` sets, the other two toggle — the one rule that differs.
                self.assertEqual(got, [True] if cls is Radio else [False])

    def test_indicator_false_needs_no_content_at_all(self):
        # A colour swatch: the container *is* the widget. There is nothing to label and
        # nothing to draw beside, so requiring content here would be arbitrary.
        sw = Radio("w-12 h-12 rounded-full bg-red-500", checked=True,
                   indicator=False, on_change=lambda v: None)
        html = render(sw)
        self.assertNotIn("<input", html)
        self.assertTrue(html.startswith("<span"))
        self.assertEqual(sw.to_tree(RenderContext(), "r")["children"], [])

    def test_indicator_defaults_to_on_so_nothing_existing_changes(self):
        for cls in (Switch, Checkbox, Radio):
            with self.subTest(widget=cls.__name__):
                self.assertIn("<input", render(cls("", "L", checked=True)))
                self.assertTrue(render(cls("", checked=True)).startswith("<input"))

    def test_the_control_owns_the_tap_even_if_a_caller_also_passes_on_tap(self):
        # A labelled control's own semantics beat a stray `on_tap`, rather than one
        # silently winning depending on registration order.
        got = []
        ctx = RenderContext()
        Checkbox(
            "", "Wi-Fi", checked=False,
            on_change=lambda v: got.append(("change", v)),
            on_tap=lambda: got.append(("tap", None)),
        ).render(ctx, "c")
        ctx.dispatch("c", "tap", "")
        self.assertEqual(got, [("change", True)])


if __name__ == "__main__":
    unittest.main()
