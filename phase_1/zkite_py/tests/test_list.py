"""`List` — the virtualised list widget (plan1.md §8, phase_3/PLAN.md §11).

These tests are all pure window math and tree shape: no renderer, no device. That's
deliberate — the whole point of the spacer design is that virtualisation is invisible
to the layout engine, so its correctness is checkable here rather than only by looking
at a phone. The one property that genuinely needs a device (does it actually stay
smooth at 10k rows) is verified in phase_3/PLAN.md §11.
"""

import unittest

from zkite import Column, List, Row, Text
from zkite._viewport import clear_viewports, get_viewport, set_viewport, window_would_change
from zkite.widgets import RenderContext


def a_list(count=1000, item_height=50.0, overscan=4, class_name="flex-1"):
    return List(
        class_name,
        count=count,
        item_height=item_height,
        overscan=overscan,
        builder=lambda i: Row("items-center px-4", Text("text-white", f"Item {i}")),
    )


class TestWindowMath(unittest.TestCase):
    def setUp(self):
        clear_viewports()

    def tearDown(self):
        clear_viewports()

    def test_unreported_viewport_builds_a_bootstrap_window_not_an_empty_one(self):
        """Before any backend has reported a viewport there is no height to compute
        from. Building nothing would be the literal reading of "no viewport, nothing
        visible" and would render an empty list on the first frame of every app.
        """
        lst = a_list()
        first, last, top, bottom = lst._window("r.0")
        self.assertEqual(first, 0)
        self.assertGreater(last, 0)
        self.assertEqual(top, 0.0)
        # Bootstrap covers a tall screen plus overscan, not the whole list.
        self.assertLess(last, lst.count)
        self.assertEqual(bottom, (lst.count - last) * lst.item_height)

    def test_window_follows_scroll_position(self):
        # 500px viewport / 50px rows = 10 fully visible, +1 partial, +4 overscan
        # either side = a 19-row window wherever it sits.
        lst = a_list(count=1000, item_height=50.0, overscan=4)
        set_viewport("r.0", scroll_y=0.0, viewport_h=500.0)
        self.assertEqual(lst.window_signature("r.0"), (0, 19))

        set_viewport("r.0", scroll_y=5000.0, viewport_h=500.0)
        # 5000/50 = row 100, minus 4 overscan = 96, plus the 19-row window.
        self.assertEqual(lst.window_signature("r.0"), (96, 115))

    def test_window_keeps_its_size_when_clamping_moves_it(self):
        """A window clamped against the top of the list must not also shrink — that
        is what made a negative `scroll_y` render a blank list before this was fixed
        (see `_window`).
        """
        lst = a_list(count=1000, item_height=50.0, overscan=4)
        set_viewport("r.0", scroll_y=5000.0, viewport_h=500.0)
        first, last = lst.window_signature("r.0")
        size = last - first
        for scroll_y in (-5000.0, -1.0, 0.0, 25.0, 1000.0):
            set_viewport("r.0", scroll_y=scroll_y, viewport_h=500.0)
            f, l = lst.window_signature("r.0")
            self.assertEqual(l - f, size, msg=f"window changed size at scroll_y={scroll_y}")

    def test_total_content_height_is_window_invariant(self):
        """The property everything else rests on: whichever window is materialised,
        top_pad + rows + bottom_pad is always exactly count * item_height. That is
        what keeps `max_scroll_y`, the scroll position, and every row's y-coordinate
        identical to a non-virtualised list, so the renderer never has to know a list
        is virtualised at all.
        """
        lst = a_list(count=997, item_height=37.0)
        expected = 997 * 37.0
        for scroll_y in (0.0, 1.0, 37.0, 500.0, 12345.0, 997 * 37.0):
            set_viewport("r.0", scroll_y=scroll_y, viewport_h=811.0)
            first, last, top, bottom = lst._window("r.0")
            total = top + (last - first) * lst.item_height + bottom
            self.assertAlmostEqual(total, expected, msg=f"at scroll_y={scroll_y}")

    def test_window_is_clamped_at_both_ends(self):
        lst = a_list(count=10, item_height=50.0, overscan=4)
        set_viewport("r.0", scroll_y=-999.0, viewport_h=500.0)
        first, last, top, bottom = lst._window("r.0")
        self.assertEqual((first, last), (0, 10))
        self.assertEqual((top, bottom), (0.0, 0.0))

        # Scrolled far past the end — unreachable from the renderer, which clamps
        # to max_scroll_y. The window empties out, but padding still sums to the
        # full content height rather than going negative or overshooting.
        set_viewport("r.0", scroll_y=99999.0, viewport_h=500.0)
        first, last, top, bottom = lst._window("r.0")
        self.assertLessEqual(first, last)
        self.assertGreaterEqual(top, 0.0)
        self.assertGreaterEqual(bottom, 0.0)
        self.assertAlmostEqual(top + (last - first) * lst.item_height + bottom, 10 * 50.0)

    def test_empty_list_produces_an_empty_window_and_no_spacers(self):
        lst = a_list(count=0)
        set_viewport("r.0", scroll_y=0.0, viewport_h=500.0)
        self.assertEqual(lst._window("r.0"), (0, 0, 0.0, 0.0))
        self.assertEqual(lst._tree_children(RenderContext(), "r.0"), [])

    def test_degenerate_reported_height_falls_back_to_bootstrap(self):
        """A list laid out to zero height (collapsed by a flex parent) reports a real
        viewport of 0. Dividing by it would produce a single overscan-sized window at
        whatever the last scroll was — plausible-looking and useless.
        """
        lst = a_list()
        set_viewport("r.0", scroll_y=0.0, viewport_h=0.0)
        _, last, _, _ = lst._window("r.0")
        self.assertGreater(last, 1)

    def test_overscan_zero_builds_exactly_the_visible_rows(self):
        lst = a_list(count=1000, item_height=50.0, overscan=0)
        set_viewport("r.0", scroll_y=1000.0, viewport_h=500.0)
        # row 20 at the top, 10 fully visible + 1 partial
        self.assertEqual(lst.window_signature("r.0"), (20, 31))

    def test_rejects_impossible_construction(self):
        with self.assertRaises(ValueError):
            List("", count=-1, item_height=10, builder=lambda i: Text("", ""))
        with self.assertRaises(ValueError):
            List("", count=1, item_height=0, builder=lambda i: Text("", ""))
        with self.assertRaises(ValueError):
            List("", count=1, item_height=10, overscan=-1, builder=lambda i: Text("", ""))
        with self.assertRaises(TypeError):
            List("", count=1, item_height=10, builder="not callable")

    def test_rejects_gap_because_it_would_desynchronise_the_spacer_math(self):
        """A gap between rows makes the container's real scrollable height differ
        from `count * item_height` — N-1 gaps unvirtualised vs. W+1 windowed — which
        breaks scroll position, `max_scroll_y`, and the window computed from them,
        all subtly and all at once. Refused at construction rather than documented.
        """
        with self.assertRaises(ValueError) as cm:
            List("flex-1 gap-2", count=10, item_height=10, builder=lambda i: Text("", ""))
        self.assertIn("item_height", str(cm.exception))
        # A class that merely starts with the same letters is not a gap utility.
        List("flex-1 gaps-are-not-a-thing", count=10, item_height=10, builder=lambda i: Text("", ""))


class TestTreeShape(unittest.TestCase):
    def setUp(self):
        clear_viewports()

    def tearDown(self):
        clear_viewports()

    def test_only_windowed_rows_are_built(self):
        """The actual promise: `builder` is not called for off-screen rows. Checked by
        recording which indices it was asked for, not by counting output nodes — an
        implementation that built every row and then discarded most would produce the
        same tree and fail this.
        """
        asked: list[int] = []

        def builder(i):
            asked.append(i)
            return Text("text-white", f"Item {i}")

        lst = List("flex-1", count=10_000, item_height=50.0, overscan=2, builder=builder)
        set_viewport("r.0", scroll_y=5000.0, viewport_h=500.0)
        lst._tree_children(RenderContext(), "r.0")

        # 500/50 = 10 visible, +1 partial, +2 overscan either side = 15 rows,
        # starting at row 100 - 2.
        self.assertEqual(asked, list(range(98, 113)))
        self.assertLess(len(asked), 20, "a 10k-row list must not build 10k rows")

    def test_row_ids_are_keyed_by_item_index_not_window_position(self):
        """A row must keep one identity as the window slides over it — morphdom
        patches rather than replaces it (phase_1/PLAN.md §4), and AccessKit hashes
        this id into a stable a11y node id (phase_3/PLAN.md §8.2). Keying on the
        position within the emitted children would make every row look replaced on
        every scroll.
        """
        lst = a_list(count=1000, item_height=50.0, overscan=0)

        set_viewport("r.0", scroll_y=0.0, viewport_h=500.0)
        early = {c["id"] for c in lst._tree_children(RenderContext(), "r.0") if ".i" in c["id"]}
        set_viewport("r.0", scroll_y=250.0, viewport_h=500.0)
        later = {c["id"] for c in lst._tree_children(RenderContext(), "r.0") if ".i" in c["id"]}

        overlap = early & later
        self.assertTrue(overlap, "windows that overlap must share row ids")
        self.assertIn("r.0.i5", overlap)

    def test_rows_are_wrapped_at_the_declared_item_height(self):
        lst = a_list(count=100, item_height=56.0)
        set_viewport("r.0", scroll_y=0.0, viewport_h=500.0)
        kids = lst._tree_children(RenderContext(), "r.0")
        rows = [c for c in kids if ".i" in c["id"]]
        self.assertTrue(rows)
        for row in rows:
            self.assertIn("h-[56px]", row["classes"])
            self.assertIn("shrink-0", row["classes"])
            self.assertIn("overflow-hidden", row["classes"])

    def test_spacers_are_shrink_0(self):
        """Without `shrink-0` flexbox compresses the spacers to fit the (deliberately
        shorter) container and the list stops scrolling entirely — the same class of
        bug `ScrollView`'s `min-h-0` note documents from the other direction.
        """
        lst = a_list(count=1000, item_height=50.0)
        set_viewport("r.0", scroll_y=5000.0, viewport_h=500.0)
        kids = lst._tree_children(RenderContext(), "r.0")
        spacers = [c for c in kids if c["id"].endswith("pad-top") or c["id"].endswith("pad-bottom")]
        self.assertEqual(len(spacers), 2)
        for s in spacers:
            self.assertIn("shrink-0", s["classes"])

    def test_event_handlers_inside_rows_register_against_the_row_path(self):
        """A tap on a virtualised row has to dispatch like any other tap — the
        handler must land in the RenderContext under the id the tree actually
        carries, or the native side's hit-test -> zkite_id -> dispatch walk finds
        nothing.
        """
        taps: list[int] = []
        lst = List(
            "flex-1",
            count=100,
            item_height=50.0,
            overscan=0,
            builder=lambda i: Column("", Text("", f"Item {i}"), key=f"row-{i}", on_tap=lambda: taps.append(i)),
        )
        ctx = RenderContext()
        set_viewport("r.0", scroll_y=0.0, viewport_h=200.0)
        lst._tree_children(ctx, "r.0")

        self.assertTrue(ctx.dispatch("row-3", "tap"))
        self.assertEqual(taps, [3])

    def test_html_and_native_trees_materialise_the_same_window(self):
        """Both backends must agree on which rows exist — a `List` that virtualised
        differently per backend would break the conformance bar plan1.md §5 sets for
        flipping the default backend.
        """
        lst = a_list(count=500, item_height=40.0, overscan=1)
        set_viewport("r.0", scroll_y=2000.0, viewport_h=400.0)

        html_out = lst._render_children(RenderContext(), "r.0")
        tree_out = lst._tree_children(RenderContext(), "r.0")

        tree_ids = [c["id"] for c in tree_out]
        for node_id in tree_ids:
            self.assertIn(f'id="{node_id}"', html_out)
        # Count the row *wrappers* specifically: a plain `"r.0.i"` substring also
        # matches each row's own inner content (rendered at `r.0.i<n>.0`), which
        # would make this pass for the wrong reason.
        row_ids = [i for i in tree_ids if ".pad-" not in i]
        self.assertTrue(row_ids)
        for row_id in row_ids:
            self.assertEqual(html_out.count(f'<div id="{row_id}" '), 1)

    def test_render_and_to_tree_mark_the_container_for_scroll_reporting(self):
        lst = a_list()
        html_out = lst.render(RenderContext(), "r.0")
        self.assertIn('data-zkite-virtual="1"', html_out)
        self.assertIn("overflow-auto", html_out)
        self.assertIn("min-h-0", html_out)

        tree = lst.to_tree(RenderContext(), "r.0")
        self.assertEqual(tree["attrs"]["virtual_count"], lst.count)
        self.assertEqual(tree["attrs"]["virtual_item_height"], lst.item_height)


class TestRebuildFrequency(unittest.TestCase):
    """`window_would_change` — the function that decides whether a scroll frame
    costs a re-render (phase_3/PLAN.md §39.5, §40).

    It had no tests at all before §40, which is how the `overscan`-is-free-slack
    assumption survived: every test here asks about the *rate* of rebuilds, and
    nothing about a single window's contents can see that.
    """

    def setUp(self):
        clear_viewports()

    def tearDown(self):
        clear_viewports()

    def build(self, lst, node_id="r.0", scroll_y=0.0, viewport_h=500.0):
        """Materialise `lst`'s window at this scroll position, exactly as a real
        render does: compute it, then record it.
        """
        set_viewport(node_id, scroll_y, viewport_h)
        first, last = lst.window_signature(node_id)
        lst._note(node_id, first, last)
        return first, last

    def test_a_freshly_built_window_never_asks_to_be_rebuilt(self):
        """The invariant the whole scheme rests on: `compute_window`'s output always
        covers its own visible range. If it did not, every scroll frame would
        re-render forever — the exact opposite of what this is for. Checked across
        the clamping branches at both ends, where `first`/`last` stop tracking
        `scroll_y` and coverage is least obvious.
        """
        for overscan in (0, 1, 4, 16):
            for scroll_y in (0.0, 25.0, 100.0, 1000.0, 49_500.0, 49_999.0):
                with self.subTest(overscan=overscan, scroll_y=scroll_y):
                    clear_viewports()
                    lst = a_list(count=1000, item_height=50.0, overscan=overscan)
                    self.build(lst, scroll_y=scroll_y)
                    self.assertFalse(window_would_change("r.0"))

    def test_scrolling_within_the_overscan_does_not_rebuild(self):
        """The negative control for the fix, and the test that fails against the old
        equality-based implementation: with 4 rows of slack below, scrolling down by
        one row leaves every visible row already materialised, so there is nothing
        to build. The old code rebuilt here — `compute_window`'s `first` had moved.
        """
        lst = a_list(count=1000, item_height=50.0, overscan=4)
        self.build(lst, scroll_y=1000.0)
        for rows_down in (1, 2, 3, 4):
            with self.subTest(rows_down=rows_down):
                set_viewport("r.0", 1000.0 + rows_down * 50.0, 500.0)
                self.assertFalse(window_would_change("r.0"))

    def test_scrolling_past_the_overscan_does_rebuild(self):
        """The other half of the control: coverage is not an excuse to never
        rebuild. One row beyond the slack and the bottom visible row is genuinely
        not materialised, so it must re-render or show a blank strip.
        """
        lst = a_list(count=1000, item_height=50.0, overscan=4)
        self.build(lst, scroll_y=1000.0)
        set_viewport("r.0", 1000.0 + 5 * 50.0, 500.0)
        self.assertTrue(window_would_change("r.0"))

    def test_scrolling_up_past_the_overscan_rebuilds_too(self):
        """Slack is symmetric — a drag has two directions and the old asymmetry
        would only show up on one of them."""
        lst = a_list(count=1000, item_height=50.0, overscan=4)
        self.build(lst, scroll_y=1000.0)
        set_viewport("r.0", 1000.0 - 4 * 50.0, 500.0)
        self.assertFalse(window_would_change("r.0"))
        set_viewport("r.0", 1000.0 - 5 * 50.0, 500.0)
        self.assertTrue(window_would_change("r.0"))

    def rebuilds_over(self, overscan, travel_rows=200, item_height=50.0, viewport_h=500.0):
        """Rebuilds counted over a mid-list drag, the way a real scroll drives it:
        each rebuild re-records the window, so the next one is measured against
        what is genuinely on screen rather than against the first window forever.
        """
        clear_viewports()
        lst = a_list(count=100_000, item_height=item_height, overscan=overscan)
        base = 1_000_000.0
        self.build(lst, scroll_y=base, viewport_h=viewport_h)
        rebuilds, rows_built = 1, lst.window_signature("r.0")[1] - lst.window_signature("r.0")[0]
        for row in range(1, travel_rows + 1):
            set_viewport("r.0", base + row * item_height, viewport_h)
            if window_would_change("r.0"):
                first, last = self.build(lst, scroll_y=base + row * item_height, viewport_h=viewport_h)
                rebuilds += 1
                rows_built += last - first
        return rebuilds, rows_built

    def test_overscan_is_now_a_real_frequency_knob(self):
        """The finding, pinned as a test. Over 200 rows of mid-list scroll the old
        implementation rebuilt 200 times at *every* `overscan` — the offset shifted
        the window without ever stalling it, so the documented §39.5 mitigation
        ("a larger overscan: fewer, costlier rebuilds") bought the same number of
        `sync` spikes, each one bigger.

        Now a rebuild happens once per `overscan + 1` rows, so raising `overscan`
        reduces both the rebuild count *and* the total rows built, at the cost of a
        bigger individual spike. That is the trade §39.5 believed it already had.
        """
        counts = {ov: self.rebuilds_over(ov) for ov in (0, 4, 8, 16)}
        rebuilds = {ov: c[0] for ov, c in counts.items()}
        self.assertEqual(rebuilds[0], 201)  # no slack: still once per row, as before
        self.assertEqual(rebuilds[4], 41)  # once per 5 rows
        self.assertEqual(rebuilds[8], 23)  # once per 9 rows
        self.assertEqual(rebuilds[16], 12)  # once per 17 rows

        # Strictly monotone: more slack is fewer rebuilds *and* less total work,
        # which is what the old equality test could not deliver at any value.
        for lo, hi in ((0, 4), (4, 8), (8, 16)):
            self.assertLess(rebuilds[hi], rebuilds[lo])
            self.assertLess(counts[hi][1], counts[lo][1])

    def test_a_taller_viewport_rebuilds_even_without_scrolling(self):
        """Coverage depends on the viewport height too, not just the offset. A
        rotation or a keyboard dismissal grows the visible range under a window
        that has not moved, and those new rows have to be built.
        """
        lst = a_list(count=1000, item_height=50.0, overscan=4)
        self.build(lst, scroll_y=1000.0, viewport_h=500.0)
        set_viewport("r.0", 1000.0, 700.0)  # +4 rows visible, exactly the slack
        self.assertFalse(window_would_change("r.0"))
        set_viewport("r.0", 1000.0, 900.0)
        self.assertTrue(window_would_change("r.0"))

    def test_a_node_that_never_rendered_a_window_never_asks_for_one(self):
        """A plain `ScrollView` reports its viewport on every scroll frame like
        everything else, and must cost nothing — this is the two-dict-lookup path
        `_zkite_scroll_tree` documents.
        """
        set_viewport("r.0", 5000.0, 500.0)
        self.assertFalse(window_would_change("r.0"))


class TestViewportRegistry(unittest.TestCase):
    def setUp(self):
        clear_viewports()

    def tearDown(self):
        clear_viewports()

    def test_set_viewport_reports_whether_it_changed(self):
        self.assertTrue(set_viewport("r.0", 0.0, 500.0))
        self.assertFalse(set_viewport("r.0", 0.0, 500.0))
        self.assertTrue(set_viewport("r.0", 1.0, 500.0))
        self.assertEqual(get_viewport("r.0"), (1.0, 500.0))

    def test_unknown_node_reads_as_none_not_zero(self):
        """`None` and `(0.0, 0.0)` mean genuinely different things here — "nothing has
        been laid out yet" vs. "laid out, and it's a zero-height box" — and `List`
        treats them differently only in that both bootstrap; conflating them in the
        registry would remove the ability to tell them apart at all.
        """
        self.assertIsNone(get_viewport("nope"))


if __name__ == "__main__":
    unittest.main()
