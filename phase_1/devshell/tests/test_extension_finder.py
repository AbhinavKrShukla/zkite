"""Host tests for `runtime.py`'s `_ZKiteNativeExtensionFinder` (phase_3/PLAN.md §11.6).

Run: `python3 -m unittest discover -s phase_1/devshell/tests`

The bug this guards against was invisible for three phases because it needs a real
Android *installer* to show up — the APK contained every extension module, the
device did not. What is host-testable is the half that has to be right for the
renamed files to be usable at all: that a `.so` deliberately named something the
ordinary import machinery cannot find is still importable under its real module
name once this finder is installed, and that the finder does not answer for
anything else.

The tests take extension modules from the *host* interpreter's own lib-dynload
and rename copies of them the same way `build_apk.sh` renames the Android ones,
then remove lib-dynload from `sys.path` so the ordinary machinery genuinely
cannot resolve them — which reproduces the device's condition rather than
approximating it.
"""

import ast
import os
import shutil
import subprocess
import sys
import sysconfig
import tempfile
import textwrap
import unittest

RUNTIME_PY = os.path.join(os.path.dirname(__file__), "..", "src", "runtime.py")


def _finder_source():
    """The finder, lifted verbatim out of the real runtime.py.

    Taken from the shipping file rather than duplicated here, so this test cannot
    quietly pass against a stale copy of the code it is supposed to be testing.
    """
    src = open(os.path.abspath(RUNTIME_PY)).read()
    start = src.index("class _ZKiteNativeExtensionFinder")
    end = src.index("_native_extension_count =")
    return src[start:end]


def _host_extension_modules():
    """Every extension module in the host's lib-dynload, as (name, path)."""
    dynload = sysconfig.get_config_var("DESTSHARED")
    if not dynload or not os.path.isdir(dynload):
        return []
    out = []
    for entry in sorted(os.listdir(dynload)):
        if entry.endswith(".so"):
            out.append((entry.split(".", 1)[0], os.path.join(dynload, entry)))
    return out


class ExtensionFinderTest(unittest.TestCase):
    def setUp(self):
        mods = _host_extension_modules()
        if len(mods) < 2:
            self.skipTest("host interpreter has no separate lib-dynload to sample")
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        # Stage *all* of lib-dynload, exactly as build_apk.sh stages all of the
        # Android prefix's: <mod>.cpython-*.so -> libpymod_<mod>.so. Staging the
        # whole set rather than a sample matters — extension modules import each
        # other (`_asyncio` needs `_contextvars`), so a partial stage would fail
        # for reasons that have nothing to do with the finder, and a full one
        # additionally proves those inter-module imports resolve through it too.
        self.staged = {}
        for name, path in mods:
            shutil.copy(path, os.path.join(self.tmp, "libpymod_%s.so" % name))
            self.staged[name] = path
        # Sample something not already in sys.modules: an already-imported module
        # would resolve out of sys.modules and the test would pass without the
        # finder doing anything at all.
        self.sample = next(n for n in self.staged if n not in sys.modules)

    def _run(self, body):
        """Run `body` in a fresh interpreter with lib-dynload made unreachable."""
        script = "import os, sys\n"
        script += _finder_source()
        # Make lib-dynload unreachable *after* the finder source has been defined,
        # so this mirrors the device (extension modules absent from every ordinary
        # path entry) rather than breaking the interpreter's own startup.
        script += '\nsys.path[:] = [p for p in sys.path if "lib-dynload" not in p]\n'
        script += "STAGE = %r\n" % self.tmp
        script += textwrap.dedent(body)
        proc = subprocess.run(
            [sys.executable, "-c", script], capture_output=True, text=True
        )
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        return proc.stdout.strip()

    def test_renamed_module_is_unimportable_without_the_finder(self):
        """The device's actual failure, reproduced: the file exists, the import fails.

        Without this, the next test could pass for the wrong reason — the module
        being resolvable some other way rather than through the finder.
        """
        name = self.sample
        out = self._run(
            """
            sys.path.insert(0, STAGE)
            try:
                __import__({name!r})
                print("IMPORTED")
            except ModuleNotFoundError as e:
                print("ModuleNotFoundError:", e)
            """.format(name=name)
        )
        self.assertIn("ModuleNotFoundError", out)
        self.assertIn(name, out)

    def test_finder_imports_a_renamed_extension_module_and_it_really_runs(self):
        """A `libpymod_*.so` imports under its real name, and its code executes.

        `ExtensionFileLoader` resolves the init function from the module *name*
        (`PyInit_<name>`), never the file name, which is the property that makes
        the rename safe. Asserting `__file__` proves it came from the renamed
        copy and not from the host's own lib-dynload.
        """
        name = self.sample
        out = self._run(
            """
            sys.path.insert(0, STAGE)
            print("count", _install_native_extension_finder())
            m = __import__({name!r})
            print("file", m.__file__)
            print("dir_nonempty", bool([a for a in dir(m) if not a.startswith("__")]))
            """.format(name=name)
        )
        self.assertGreaterEqual(int(out.splitlines()[0].split()[1]), 2)
        self.assertIn("libpymod_%s.so" % name, out)
        self.assertIn("dir_nonempty True", out)

    def test_finder_declines_names_it_has_no_file_for(self):
        """It must not shadow or intercept anything it wasn't staged for."""
        out = self._run(
            """
            sys.path.insert(0, STAGE)
            _install_native_extension_finder()
            try:
                __import__("zkite_no_such_module")
                print("WRONGLY IMPORTED")
            except ModuleNotFoundError as e:
                print("declined:", e)
            """
        )
        self.assertIn("declined:", out)

    def test_no_staged_modules_means_no_meta_path_entry(self):
        """On a backend with nothing staged, installing must be a clean no-op.

        The WebView shells run this same runtime.py; adding a finder that can
        never match anything would put a pointless entry on every import's
        critical path.
        """
        out = self._run(
            """
            before = len(sys.meta_path)
            print("count", _install_native_extension_finder())
            print("delta", len(sys.meta_path) - before)
            """
        )
        self.assertIn("count 0", out)
        self.assertIn("delta 0", out)


class ImportOrderTest(unittest.TestCase):
    """`runtime.py` must install the finder *before* importing `json`.

    phase_3/PLAN.md §43. `json/encoder.py` and `json/scanner.py` bind their C
    accelerators once, at module import time, inside `try/except ImportError`. On
    Android `_json` is reachable only through `_ZKiteNativeExtensionFinder`, so
    importing `json` before that finder exists pinned the process to the
    pure-Python encoder and decoder permanently — silently, and for the life of the
    app. It cost **3.75ms of every virtualised-list rebuild** on a Moto G54, 44% of
    that rebuild's Python time.

    The device is the evidence; this is the regression guard. It reproduces the
    device's condition rather than approximating it: `_json` is made genuinely
    unimportable until the finder is on `sys.meta_path`, which is exactly the
    device's state, and then the *real, shipping* `runtime.py` is executed.
    """

    def setUp(self):
        # The finder only installs itself when it finds something to serve, so the
        # gate below can only ever open if at least one `libpymod_*.so` is on the
        # path. Stage one, the same way `build_apk.sh` stages the Android set.
        mods = _host_extension_modules()
        if not mods:
            self.skipTest("host interpreter has no separate lib-dynload to sample")
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        name, path = mods[0]
        shutil.copy(path, os.path.join(self.tmp, "libpymod_%s.so" % name))

    # Blocks `_json` until a `_ZKiteNativeExtensionFinder` appears in sys.meta_path —
    # the host's `_json` is a builtin, so it cannot be hidden by path manipulation
    # the way `test_finder_imports_a_renamed_extension_module` hides lib-dynload.
    GATE = textwrap.dedent(
        """
        import sys
        sys.path.insert(0, {stage!r})
        for _m in [m for m in sys.modules if m == "json" or m.startswith("json.")]:
            del sys.modules[_m]
        sys.modules.pop("_json", None)

        class _Gate:
            def find_spec(self, name, path=None, target=None):
                if name != "_json":
                    return None
                if any(type(f).__name__ == "_ZKiteNativeExtensionFinder" for f in sys.meta_path):
                    return None   # finder installed: let the real _json resolve
                raise ImportError("_json unreachable until the finder is installed")

        sys.meta_path.insert(0, _Gate())
        _real_stdout = sys.stdout
        """
    )

    REPORT = textwrap.dedent(
        """
        import json.encoder, json.scanner
        print("c_make_encoder", json.encoder.c_make_encoder is not None, file=_real_stdout)
        print("c_make_scanner", json.scanner.c_make_scanner is not None, file=_real_stdout)
        """
    )

    def _run_runtime(self, source):
        script = self.GATE.format(stage=self.tmp) + "\n" + source + "\n" + self.REPORT
        proc = subprocess.run([sys.executable, "-c", script], capture_output=True, text=True)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        return proc.stdout

    def test_the_shipping_runtime_binds_the_c_json_accelerators(self):
        with open(os.path.abspath(RUNTIME_PY)) as fh:
            src = fh.read()
        out = self._run_runtime(src)
        self.assertIn("c_make_encoder True", out)
        self.assertIn("c_make_scanner True", out)

    def test_negative_control_importing_json_first_loses_the_accelerators(self):
        """The pre-fix order, run through the same harness.

        Without this the test above could pass for the wrong reason — the gate not
        actually gating, or `_json` resolving by some route the device does not
        have. It must fail in exactly the way the device did.
        """
        with open(os.path.abspath(RUNTIME_PY)) as fh:
            src = fh.read()
        # Reproduce the original ordering: json imported in the top block, before
        # the finder is installed.
        self.assertIn("import io\nimport os\nimport sys", src)
        src = src.replace("import io\nimport os\nimport sys", "import io\nimport json\nimport os\nimport sys", 1)
        out = self._run_runtime(src)
        self.assertIn("c_make_encoder False", out)
        self.assertIn("c_make_scanner False", out)


# The Android stdlib that actually ships, and the extension modules that actually ship
# beside it. Read from the vendored prefix rather than from the host interpreter: the
# question is what *this build* degrades, and `build.sh` stages exactly this directory.
ANDROID_PREFIX = os.path.join(
    os.path.dirname(__file__), "..", "..", "..",
    "phase_0", "vendor", "python-aarch64", "prefix", "lib", "python3.14",
)


def _shipped_extension_names() -> set[str]:
    dynload = os.path.join(ANDROID_PREFIX, "lib-dynload")
    if not os.path.isdir(dynload):
        return set()
    return {n.split(".", 1)[0] for n in os.listdir(dynload) if n.endswith(".so")}


def _module_scope_statements(body):
    """Statements reachable at module scope, not descending into a def or class.

    The scope distinction is the whole correctness of this check. The same
    `try: import _x / except ImportError:` idiom *inside a function* is retried on
    every call and picks the accelerator up as soon as it is importable — that is
    what `zipimport` does for `zlib`, and a scan without this distinction reports
    `zipimport` (which really is imported at startup) as a live collision when it is
    nothing of the kind.
    """
    for node in body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            continue
        yield node
        for field in ("body", "orelse", "finalbody"):
            inner = getattr(node, field, None)
            if isinstance(inner, list):
                yield from _module_scope_statements(inner)
        for handler in getattr(node, "handlers", []) or []:
            yield from _module_scope_statements(handler.body)


def _lazily_imported_accelerators(path: str) -> set[str]:
    """Modules this file imports at module scope inside a `try` that swallows
    ImportError — i.e. bindings that silently degrade to a fallback, once, forever."""
    try:
        with open(path, errors="replace") as fh:
            tree = ast.parse(fh.read())
    except (SyntaxError, OSError):
        return set()
    found: set[str] = set()
    for node in _module_scope_statements(tree.body):
        if not isinstance(node, ast.Try):
            continue
        caught: list[str] = []
        for handler in node.handlers:
            exc = handler.type
            if exc is None:
                caught.append("ImportError")
            elif isinstance(exc, ast.Name):
                caught.append(exc.id)
            elif isinstance(exc, ast.Tuple):
                caught += [e.id for e in exc.elts if isinstance(e, ast.Name)]
        if not any(c in ("ImportError", "ModuleNotFoundError", "Exception") for c in caught):
            continue
        # Only what the `try` imports; an import in the handler is the fallback.
        for sub in ast.walk(ast.Module(body=node.body, type_ignores=[])):
            if isinstance(sub, ast.ImportFrom) and sub.module and sub.level == 0:
                found.add(sub.module.split(".")[0])
            elif isinstance(sub, ast.Import):
                found.update(a.name.split(".")[0] for a in sub.names)
    return found


class NoDegradedModuleAtFinderInstallTest(unittest.TestCase):
    """Nothing already imported when the finder installs may lose an accelerator.

    phase_3/PLAN.md §43.8 closed §43's fix for `json` and then asserted, without
    measuring it, that modules imported during interpreter startup are unaffected.
    This is that measurement, kept as a test.

    It generalises `ImportOrderTest` from `json` to the whole shipped stdlib: at the
    instant `_install_native_extension_finder` puts the finder on `sys.meta_path`,
    every module already in `sys.modules` is checked against the Android stdlib's own
    source for a module-scope `try: import <shipped extension> / except ImportError`.
    **29 stdlib modules carry that idiom** — `pickle`, `decimal`, `hashlib`, `hmac`,
    `heapq`, `bisect`, `queue`, `ssl`, `zoneinfo`, `statistics` and `json`'s three
    among them — so the set this must stay disjoint from is neither small nor
    hypothetical.
    """

    def setUp(self):
        if not os.path.isdir(ANDROID_PREFIX):
            self.skipTest("vendored Android prefix not present")
        self.ext = _shipped_extension_names()
        if not self.ext:
            self.skipTest("vendored Android lib-dynload not present")
        mods = _host_extension_modules()
        if not mods:
            self.skipTest("host interpreter has no separate lib-dynload to sample")
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        name, path = mods[0]
        shutil.copy(path, os.path.join(self.tmp, "libpymod_%s.so" % name))

    def _modules_at_install(self, source: str) -> list[str]:
        """`sys.modules` at the moment the finder is appended to `sys.meta_path`.

        Captured by wrapping `sys.meta_path.append` rather than by splitting the
        source at the call site — the point of interest is the real runtime's real
        behaviour, and a textual split would stop testing it the moment the file is
        reorganised.
        """
        harness = textwrap.dedent(
            """
            import sys
            sys.path.insert(0, {stage!r})
            _real_stdout = sys.stdout
            _snapshot = []

            # A list *subclass*, because `sys.meta_path` is a plain list whose
            # `append` is read-only, and the import system requires a real list.
            class _Spy(list):
                def append(self, f):
                    if type(f).__name__ == "_ZKiteNativeExtensionFinder":
                        _snapshot.append(sorted(sys.modules))
                    return super().append(f)

            sys.meta_path = _Spy(sys.meta_path)
            """
        ).format(stage=self.tmp)
        report = textwrap.dedent(
            """
            assert _snapshot, "the finder was never installed - test harness is not exercising it"
            print("\\n".join(_snapshot[0]), file=_real_stdout)
            """
        )
        proc = subprocess.run(
            [sys.executable, "-c", harness + "\n" + source + "\n" + report],
            capture_output=True, text=True,
        )
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        return [m for m in proc.stdout.splitlines() if m.strip()]

    def _degraded(self, modules: list[str]) -> dict[str, set[str]]:
        """Of `modules`, those whose Android source lazily binds a shipped accelerator."""
        out: dict[str, set[str]] = {}
        for mod in modules:
            rel = mod.replace(".", os.sep)
            for candidate in (rel + ".py", os.path.join(rel, "__init__.py")):
                path = os.path.join(ANDROID_PREFIX, candidate)
                if not os.path.isfile(path):
                    continue
                hit = _lazily_imported_accelerators(path) & self.ext
                if hit:
                    out[mod] = hit
        return out

    def test_nothing_imported_before_the_finder_loses_an_accelerator(self):
        with open(os.path.abspath(RUNTIME_PY)) as fh:
            src = fh.read()
        modules = self._modules_at_install(src)
        # Guards the guard: if this ever collapsed to a handful of builtins the test
        # would pass by measuring nothing.
        self.assertGreater(len(modules), 20, modules)
        degraded = self._degraded(modules)
        self.assertEqual(degraded, {}, "modules imported before the finder that lose a C accelerator")

    def test_negative_control_importing_json_first_is_caught(self):
        """The pre-fix ordering must be *detected by this test*, not just by the
        `json`-specific one — otherwise this generalisation proves nothing."""
        with open(os.path.abspath(RUNTIME_PY)) as fh:
            src = fh.read()
        self.assertIn("import io\nimport os\nimport sys", src)
        src = src.replace("import io\nimport os\nimport sys", "import io\nimport json\nimport os\nimport sys", 1)
        degraded = self._degraded(self._modules_at_install(src))
        self.assertIn("json.encoder", degraded)
        self.assertEqual(degraded["json.encoder"], {"_json"})


if __name__ == "__main__":
    unittest.main()
