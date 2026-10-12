import sys
import os
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest.mock import patch

import zkite._store as store_mod
from zkite.reload import load_module, reload_module
from zkite._store import get_instance, set_render_callback


COUNTER_V1 = """
from zkite import store

@store
class Counter:
    n: int = 0
"""

COUNTER_V2_SAME_SHAPE = """
from zkite import store

@store
class Counter:
    n: int = 0

# a comment-only change plus a new free function; store shape is untouched
def helper():
    return 42
"""

COUNTER_V3_INCOMPATIBLE_SHAPE = """
from zkite import store

@store
class Counter:
    n: int = 0
    label: str = "hi"
"""


class ReloadTests(unittest.TestCase):
    def setUp(self):
        store_mod.reset_registry()
        set_render_callback(None)
        self._tmpdir = tempfile.TemporaryDirectory()
        self.main_path = Path(self._tmpdir.name) / "main.py"

    def tearDown(self):
        self._tmpdir.cleanup()

    def _write(self, source: str) -> None:
        self.main_path.write_text(textwrap.dedent(source))

    def test_load_module_executes_and_registers_store(self):
        self._write(COUNTER_V1)
        module = load_module(self.main_path, "main")
        Counter = module.Counter
        c = get_instance(Counter)
        self.assertEqual(c.n, 0)

    def test_same_shape_reload_migrates_instance_identity_and_data(self):
        self._write(COUNTER_V1)
        module = load_module(self.main_path, "main")
        c = get_instance(module.Counter)
        c.n = 7

        self._write(COUNTER_V2_SAME_SHAPE)
        result = reload_module(self.main_path, "main")

        self.assertEqual(result.migrated_stores, ["main.Counter"])
        self.assertEqual(result.reset_stores, [])

        new_counter_cls = result.module.Counter
        c_again = get_instance(new_counter_cls)
        self.assertIs(c_again, c)  # same object survived the reload
        self.assertEqual(c_again.n, 7)  # data survived
        self.assertIs(type(c_again), new_counter_cls)  # but class is the new one

    def test_incompatible_shape_reload_resets_that_store_only(self):
        self._write(COUNTER_V1)
        module = load_module(self.main_path, "main")
        c = get_instance(module.Counter)
        c.n = 99

        self._write(COUNTER_V3_INCOMPATIBLE_SHAPE)
        result = reload_module(self.main_path, "main")

        self.assertEqual(result.migrated_stores, [])
        self.assertEqual(result.reset_stores, ["main.Counter"])

        fresh = get_instance(result.module.Counter)
        self.assertEqual(fresh.n, 0)  # back to the default, not 99
        self.assertEqual(fresh.label, "hi")

    def test_reload_of_unrelated_module_leaves_untouched_stores_alone(self):
        source_two_stores = """
        from zkite import store

        @store
        class A:
            x: int = 0

        @store
        class B:
            y: int = 0
        """
        self._write(source_two_stores)
        module = load_module(self.main_path, "main")
        a = get_instance(module.A)
        b = get_instance(module.B)
        a.x = 1
        b.y = 2

        # reload with A's shape broken, B untouched
        broken_a_source = """
        from zkite import store

        @store
        class A:
            x: int = 0
            extra: int = 0

        @store
        class B:
            y: int = 0
        """
        self._write(broken_a_source)
        result = reload_module(self.main_path, "main")

        self.assertEqual(result.reset_stores, ["main.A"])
        self.assertEqual(result.migrated_stores, ["main.B"])
        self.assertEqual(get_instance(result.module.B).y, 2)  # B's state survived


class ProjectImportReloadTests(unittest.TestCase):
    def setUp(self):
        store_mod.reset_registry()
        set_render_callback(None)
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.main = self.root / "main.py"
        self.helper = self.root / "zkite_test_helper.py"
        self.path_patch = patch.object(sys, "path", [str(self.root), *sys.path])
        self.path_patch.start()
        self.addCleanup(self.path_patch.stop)
        self.addCleanup(lambda: sys.modules.pop("zkite_test_helper", None))
        self.addCleanup(lambda: sys.modules.pop("zkite_test_main", None))

    def test_edits_to_imported_helpers_reload_even_with_same_size_and_timestamp(self):
        self.helper.write_text("value = 1\n")
        self.main.write_text("import zkite_test_helper\nvalue = zkite_test_helper.value\n")
        initial = load_module(self.main, "zkite_test_main")
        self.assertEqual(initial.value, 1)
        stamp = self.helper.stat()
        self.helper.write_text("value = 2\n")
        os.utime(self.helper, ns=(stamp.st_atime_ns, stamp.st_mtime_ns))
        result = reload_module(self.main, "zkite_test_main")
        self.assertEqual(result.module.value, 2)

    def test_deleted_helpers_fail_import_and_failed_reload_restores_previous_modules(self):
        self.helper.write_text("value = 1\n")
        self.main.write_text("import zkite_test_helper\nvalue = zkite_test_helper.value\n")
        initial = load_module(self.main, "zkite_test_main")
        helper = sys.modules["zkite_test_helper"]
        self.helper.unlink()
        with self.assertRaises(ModuleNotFoundError):
            reload_module(self.main, "zkite_test_main")
        self.assertIs(sys.modules["zkite_test_main"], initial)
        self.assertIs(sys.modules["zkite_test_helper"], helper)
        # Once main no longer imports the removed helper, it cannot linger after
        # a successful reload or a project switch into this app directory.
        self.main.write_text("value = 3\n")
        result = reload_module(self.main, "zkite_test_main")
        self.assertEqual(result.module.value, 3)
        self.assertNotIn("zkite_test_helper", sys.modules)

    def test_stores_declared_in_imported_helpers_preserve_identity_and_state(self):
        self.helper.write_text("from zkite import store\n@store\nclass Counter:\n    n: int = 0\n")
        self.main.write_text("import zkite_test_helper\n")
        initial = load_module(self.main, "zkite_test_main")
        instance = get_instance(initial.zkite_test_helper.Counter)
        instance.n = 42
        self.helper.write_text("from zkite import store\n@store\nclass Counter:\n    n: int = 0\n    def doubled(self): return self.n * 2\n")
        result = reload_module(self.main, "zkite_test_main")
        migrated = get_instance(result.module.zkite_test_helper.Counter)
        self.assertIs(instance, migrated)
        self.assertEqual(migrated.doubled(), 84)
        self.assertIn("zkite_test_helper.Counter", result.migrated_stores)


class SysModulesRegistrationTests(unittest.TestCase):
    """`load_module` must install the module in `sys.modules` *before* executing it.

    Reported from a real app: any `main.py` beginning with
    `from __future__ import annotations` and declaring one `@store` failed to boot,
    with an `AttributeError` raised from inside CPython's `dataclasses` and nothing
    in the traceback naming ZKite. Under PEP 563 every annotation is a string, so
    `dataclasses` resolves names textually to find `KW_ONLY`, via an unguarded
    `sys.modules.get(cls.__module__).__dict__`.
    """

    def setUp(self):
        store_mod.reset_registry()
        set_render_callback(None)
        self._tmpdir = tempfile.TemporaryDirectory()
        self.main_path = Path(self._tmpdir.name) / "main.py"
        sys.modules.pop("main", None)

    def tearDown(self):
        self._tmpdir.cleanup()
        sys.modules.pop("main", None)

    def _write(self, source: str) -> None:
        self.main_path.write_text(textwrap.dedent(source))

    def test_store_under_future_annotations_loads(self):
        self._write(
            """
            from __future__ import annotations

            from zkite import store

            @store
            class Board:
                station_id: int = 1
                query: str = ""
                tags: list = None
            """
        )
        module = load_module(self.main_path, "main")
        self.assertEqual(get_instance(module.Board).station_id, 1)

    def test_module_is_in_sys_modules_while_it_executes(self):
        # The general property, stated directly rather than only through dataclasses:
        # stdlib code is entitled to assume `sys.modules[__name__]` exists.
        self._write(
            """
            import sys

            SELF_VISIBLE = sys.modules.get(__name__) is not None
            """
        )
        module = load_module(self.main_path, "main")
        self.assertTrue(module.SELF_VISIBLE)

    def test_failed_load_restores_the_previous_module(self):
        self._write("VALUE = 1")
        good = load_module(self.main_path, "main")
        self.assertIs(sys.modules["main"], good)

        self._write("raise RuntimeError('boom')")
        with self.assertRaises(RuntimeError):
            load_module(self.main_path, "main")
        # A failed hot reload must not also unregister the module that was working.
        self.assertIs(sys.modules["main"], good)

    def test_failed_first_load_leaves_no_half_executed_module_behind(self):
        self._write("raise RuntimeError('boom')")
        with self.assertRaises(RuntimeError):
            load_module(self.main_path, "main")
        self.assertNotIn("main", sys.modules)


if __name__ == "__main__":
    unittest.main()
