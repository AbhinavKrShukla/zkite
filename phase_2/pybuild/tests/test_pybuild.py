"""Regression coverage for the source-analysis/bytecode ordering boundary."""
from __future__ import annotations

import contextlib
import importlib.util
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

SCRIPTS = Path(__file__).resolve().parents[1] / "scripts"


def script_module(name):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class PythonBuildTests(unittest.TestCase):
    def test_mismatched_python_is_rejected_before_changing_source(self):
        for name in ("compile_pyc", "prune_stdlib"):
            module = script_module(name)
            with patch.object(sys, "version_info", (3, 13)), contextlib.redirect_stderr(io.StringIO()) as error:
                self.assertEqual(module.main(), 1)
            self.assertIn("3.14", error.getvalue())

    @unittest.skipUnless(sys.version_info[:2] == (3, 14), "Android bytecode needs CPython 3.14")
    def test_pruning_keeps_app_and_transitive_imports_before_source_removal(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            prefix = root / "prefix"
            stdlib = prefix / "lib/python3.14"
            stdlib.mkdir(parents=True)
            (stdlib / "app_only.py").write_text("value = 1\n", encoding="utf-8")
            (stdlib / "dependency_only.py").write_text("value = 2\n", encoding="utf-8")
            (stdlib / "unused.py").write_text("value = 3\n", encoding="utf-8")
            app = root / "app"
            (app / "dependency").mkdir(parents=True)
            (app / "main.py").write_text("import app_only\nimport dependency\n", encoding="utf-8")
            (app / "dependency/__init__.py").write_text("import dependency_only\n", encoding="utf-8")
            output = root / "stdlib.zip"
            prune = [sys.executable, str(SCRIPTS / "prune_stdlib.py"), str(prefix), str(output), str(app)]
            result = subprocess.run(prune, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            with zipfile.ZipFile(output) as archive:
                self.assertIn("lib/python3.14/app_only.py", archive.namelist())
                self.assertIn("lib/python3.14/dependency_only.py", archive.namelist())
                self.assertNotIn("lib/python3.14/unused.py", archive.namelist())
            compiled = subprocess.run([sys.executable, str(SCRIPTS / "compile_pyc.py"), str(app)], capture_output=True, text=True)
            self.assertEqual(compiled.returncode, 0, compiled.stdout + compiled.stderr)
            self.assertFalse((app / "main.py").exists())
            self.assertTrue((app / "main.pyc").exists())
            output.unlink()
            # The old build order must now fail loudly, never emit a stripped zip.
            result = subprocess.run(prune, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("prune before compiling", result.stderr)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
