import unittest

import zkite._store as store_mod
from zkite._store import get_instance, set_render_callback, store


class StoreTests(unittest.TestCase):
    def setUp(self):
        store_mod.reset_registry()
        set_render_callback(None)

    def test_store_class_becomes_dataclass_with_defaults(self):
        @store
        class Counter:
            n: int = 0

        c = get_instance(Counter)
        self.assertEqual(c.n, 0)

    def test_get_instance_is_a_singleton(self):
        @store
        class Counter:
            n: int = 0

        a = get_instance(Counter)
        b = get_instance(Counter)
        self.assertIs(a, b)

    def test_update_mutates_and_triggers_render_callback(self):
        @store
        class Counter:
            n: int = 0

        renders = []
        set_render_callback(lambda: renders.append(1))
        c = get_instance(Counter)
        c.update(n=c.n + 1)
        self.assertEqual(c.n, 1)
        self.assertEqual(renders, [1])

    def test_update_with_no_render_callback_does_not_raise(self):
        @store
        class Counter:
            n: int = 0

        c = get_instance(Counter)
        c.update(n=5)  # no callback registered — must be a no-op, not an error
        self.assertEqual(c.n, 5)

    def test_two_distinct_store_classes_do_not_share_state(self):
        @store
        class A:
            x: int = 1

        @store
        class B:
            x: int = 2

        self.assertEqual(get_instance(A).x, 1)
        self.assertEqual(get_instance(B).x, 2)


if __name__ == "__main__":
    unittest.main()
