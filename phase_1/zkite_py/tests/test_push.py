"""The push path, at the layer both backends share (phase_3/PLAN.md §34).

`App` has had `on_render`/`on_render_tree` since phase 1 and **nothing ever registered
either**, so every frame in ZKite was pulled: a shell dispatched an event and painted
what came back. A `@store.update()` from a background thread refreshed `last_tree` and
no frame was ever produced. These tests cover the `zkite_py` half of closing that — the
Rust half (the injected hook, the `IN_PULL` suppression, the JNI hop) is covered by
`zkite_pybridge`'s own suite, which drives a real embedded interpreter.

Three properties, and the concurrency one is the reason the lock exists at all.
"""

import threading
import time
import types
import unittest

import zkite._store as store_mod
from zkite import Column, Text, store
from zkite.app import App


def _module(name, view_fn, **extra):
    """A throwaway module object with a `view`, as `App` expects to be handed one."""
    m = types.ModuleType(name)
    m.view = view_fn
    for k, v in extra.items():
        setattr(m, k, v)
    return m


class PushCallbackTests(unittest.TestCase):
    def setUp(self):
        store_mod.reset_registry()
        self.addCleanup(store_mod.reset_registry)
        self.addCleanup(store_mod.set_render_callback, None)

    def _counter_app(self, mode):
        @store
        class S:
            n: int = 0

        def view(s: S):
            return Column("flex-1", Text("text-white", str(s.n)))

        app = App(_module(f"push_{mode}_{id(self)}", view))
        # Sniff the mode exactly as a shell would: the first render decides.
        (app.render_tree if mode == "tree" else app.render)()
        return app, store_mod.get_instance(S)

    def test_a_store_update_with_no_event_fires_the_tree_callback(self):
        """The gap itself: nobody dispatched anything, and a frame still arrives."""
        app, s = self._counter_app("tree")
        pushed = []
        app.on_render_tree(pushed.append)

        s.update(n=7)

        self.assertEqual(len(pushed), 1)
        self.assertIn("7", str(pushed[0]))
        self.assertEqual(app.last_tree, pushed[0], "the pushed frame is the one stored")

    def test_a_store_update_with_no_event_fires_the_html_callback(self):
        """The WebView backend's half — same mechanism, other representation."""
        app, s = self._counter_app("html")
        pushed = []
        app.on_render(pushed.append)

        s.update(n=3)

        self.assertEqual(len(pushed), 1)
        self.assertIn(">3<", pushed[0])

    def test_only_the_mode_actually_in_use_pushes(self):
        """A tree-mode app must not also produce HTML, and vice versa — `view()` is a
        user function and running it twice per state change for a representation
        nothing consumes is the cost `_mode` exists to avoid."""
        app, s = self._counter_app("tree")
        trees, htmls = [], []
        app.on_render_tree(trees.append)
        app.on_render(htmls.append)

        s.update(n=1)

        self.assertEqual(len(trees), 1)
        self.assertEqual(htmls, [])

    def test_a_view_that_updates_does_not_render_inside_its_own_render(self):
        """Re-entrancy: `view()` calling `update()` (arming a timer, kicking off a
        first fetch) must not start a nested render that replaces `ctx` half way
        through the walk still using it. The nested one is skipped, so exactly one
        tree is produced — and an unconditional `update()` in `view()` terminates
        rather than recursing forever."""
        seen = []

        @store
        class S:
            n: int = 0

        def view(s: S):
            seen.append(s.n)
            if s.n < 3:
                s.update(n=s.n + 1)  # deliberately unconditional-ish
            return Column("flex-1", Text("text-white", str(s.n)))

        app = App(_module(f"reentrant_{id(self)}", view))
        pushed = []
        app.on_render_tree(pushed.append)

        tree = app.render_tree()

        self.assertEqual(len(seen), 1, "view() must run once, not recursively")
        self.assertEqual(len(pushed), 1, "one render produces one frame")
        self.assertEqual(app.last_tree, tree)


class ConcurrentRenderTests(unittest.TestCase):
    """The hazard the push path introduced: two threads rendering the same `App`.

    Before it, a second renderer was impossible by construction. Now a worker thread's
    `update()` renders on that thread while the shell renders on its own.

    **The failure is not a torn frame, and it took a non-discriminating control to say
    what it actually is** (§34.6). `render_tree` reads `self.ctx` once and threads it
    down the walk as a local, so a concurrent render cannot corrupt a walk in
    progress — an earlier version of this test asserted that and passed with the lock
    removed. What interleaves is the *pair*: `self.ctx` ends up belonging to whichever
    render assigned it last and `self.last_tree` to whichever finished last, and those
    are not the same render. A view whose **shape** depends on state therefore leaves
    the shell painting a tree with nodes whose handlers were registered into a
    `RenderContext` nothing points at any more. The app looks completely right and
    some of its buttons silently do nothing, which is the §15.1 failure shape exactly.

    So the invariant under test is coherence, not survival: every node the *stored
    tree* says has a handler must be dispatchable against the *stored ctx*.
    """

    def setUp(self):
        store_mod.reset_registry()
        self.addCleanup(store_mod.reset_registry)
        self.addCleanup(store_mod.set_render_callback, None)

    @staticmethod
    def _tappable_ids(node, out):
        if "tap" in node.get("events", ()):
            out.append(node["id"])
        for c in node.get("children", ()):
            if isinstance(c, dict):
                ConcurrentRenderTests._tappable_ids(c, out)
        return out

    def test_the_stored_tree_and_the_stored_ctx_come_from_one_render(self):
        @store
        class S:
            n: int = 0

        def view(s: S):
            from zkite import Button

            # The child *count* depends on state, so two interleaved renders disagree
            # about which node ids exist — which is what makes the mismatch reachable.
            count = 1 + (s.n % 9)
            return Column(
                "flex-1",
                *[Button("px-2", f"b{i}", on_tap=lambda: None) for i in range(count)],
            )

        app = App(_module(f"conc_{id(self)}", view))
        app.render_tree()
        s = store_mod.get_instance(S)

        stop = threading.Event()

        def churn():
            while not stop.is_set():
                s.update(n=s.n + 1)

        workers = [threading.Thread(target=churn, daemon=True) for _ in range(3)]
        for w in workers:
            w.start()
        try:
            deadline = time.time() + 1.5
            while time.time() < deadline:
                # Sampled under the app's own lock, which is the only way to ask the
                # question at all: "is the stored pair coherent" is meaningless if a
                # render can land between reading the tree and dispatching against the
                # ctx. Holding it is also exactly what a shell does — every pull runs
                # inside it — so this is the real access pattern, not a test-only one.
                # Reaching for a private attribute is deliberate and local: this is
                # `App`'s own unit test, and the property under test *is* the lock.
                with app._lock:
                    tree = app.last_tree
                    for node_id in self._tappable_ids(tree, []):
                        self.assertTrue(
                            app.dispatch(node_id, "tap", ""),
                            f"{node_id} is in the stored tree but has no handler in "
                            f"the stored ctx — they came from different renders",
                        )
        finally:
            stop.set()
            for w in workers:
                w.join(timeout=5)


if __name__ == "__main__":
    unittest.main()
