#!/usr/bin/env python3
"""zkite build's stdlib pruning step (plan1.md §7's ``--prune-stdlib``).

Two layers, extending rather than replacing phase 0's ``make_stdlib_zip.py``:

1. The same static deny-list of directories that are large and useless on Android
   regardless of what the app imports (the test suite, Tk, 2to3, ...) — unchanged
   floor, copied from ``phase_0/spike2_pack_apk/scripts/make_stdlib_zip.py``.
2. New: ``modulefinder``-based reachability on top of what's left. Every ``.py``
   file under the app dir and the ``zkite`` framework package is scanned (recursively
   — ``modulefinder`` follows imports transitively, including into the stdlib itself,
   so ``import os`` pulls in what ``os.py`` itself imports, and so on); any top-level
   stdlib package modulefinder's bytecode walk never sees an ``IMPORT_NAME`` for gets
   dropped too, unless it's in the small ``ALWAYS_KEEP`` floor below.

Pruning is at *top-level package* granularity (``xml`` kept-or-dropped as a whole,
not ``xml.etree`` separately) — coarser than a submodule-level cut, but immune to a
whole class of "modulefinder saw `os` but not `os.path`, so `os.path` vanished and
`os.py`'s own `import os.path` breaks at boot" bugs a finer cut would need much more
care to avoid. plan1.md §7 asks for a tool that "prints what it removed so the
failure mode is diagnosable rather than mysterious" — this prints every top-level
package it kept and dropped and why, which is the same bar at coarser grain.

``modulefinder``'s bytecode walk registers every ``IMPORT_NAME`` it finds, *including*
ones behind a runtime condition it never evaluates (``if sys.platform == ...``) — so
it already errs toward keeping too much rather than too little for any one app's own
conditionals. It cannot see fully dynamic imports (``importlib.import_module(f"...")``)
or imports made from inside a C extension module; ``ALWAYS_KEEP`` exists because of
that gap, not despite it.
"""

from __future__ import annotations

import modulefinder
import os
import sys
import zipfile

PY_VERSION = "3.14"

# Same floor as phase 0's make_stdlib_zip.py — big, and useless on a phone,
# regardless of reachability.
DROP_DIRS = {
    "test",
    "idlelib",
    "tkinter",
    "turtledemo",
    "lib2to3",
    "pydoc_data",
    "ensurepip",
    "config-%s" % PY_VERSION,
    "lib-dynload",  # staged into lib/<abi>/ by the bundler, not the stdlib zip
    "__pycache__",
    "site-packages",
}
DROP_SUFFIXES = (".pyc", ".pyo", ".whl", ".a", ".o")

# Needed by interpreter bootstrap or by zkite_py itself in ways a static bytecode
# walk over app code won't reliably see: codecs looked up by string name at
# startup/import time, C-level implementation details, and modules zkite_py's C
# extensions (once any exist) might reach for without a visible IMPORT_NAME.
ALWAYS_KEEP = {
    # CPython's own Android runtime support (redirects C-level stdout/stderr to
    # logcat — there's no other way to observe them from an embedded app). Imported
    # during interpreter startup itself, before any of zkite's own glue code runs,
    # which is exactly why a purely static bytecode walk over main.py/zkite_py can
    # never see it: nothing in *application* code ever writes `import
    # _android_support`. Found the hard way — see ../../PLAN.md's M7 write-up: a
    # first build pruned it, and `Py_InitializeFromConfig` failed with
    # `ModuleNotFoundError: No module named '_android_support'`, silently on-device
    # (Android drops a bare C-level stderr write) and loudly only via the standalone
    # probe binary, the same standalone-vs-in-app split phase 0's PYTHONHOME finding
    # already established as necessary.
    "_android_support",
    "encodings",
    "codecs",
    # `encodings/idna.py` does `import stringprep`, and **every hostname a networked
    # app resolves goes through the `idna` codec** — `socket.getaddrinfo` encodes the
    # host with it. `stringprep` is its own top-level module that nothing in app code
    # or `zkite_py` ever imports by name, and `encodings` is force-kept *as a package*
    # without its own imports being walked, so the reachability pass had no way to see
    # it. Dropping it turns any `urlopen("https://example.com")` into
    # **`LookupError: unknown encoding: idna`** — a codec-registry error that names
    # neither `stringprep` nor the pruner.
    #
    # Found on an emulator in §34.7 by the first app ever built with `zkite build apk`
    # that made a network request, and it is exactly the split §15.6.1 warns about:
    # `build_apk.sh`'s harness stages the stdlib differently and **kept** it, so every
    # device run in this document until now exercised a stdlib the shipped path does
    # not produce. Measured in the artifacts rather than inferred: `stringprep.py` is
    # present in the harness APK and absent from both `zkite build apk` outputs.
    "stringprep",
    "abc",
    "io",
    "_collections_abc",
    "genericpath",
    "posixpath",
    "ntpath",
    "os",
    "stat",
    "collections",
    "_collections",
    "keyword",
    "operator",
    "reprlib",
    "functools",
    "types",
    "enum",
    "sre_compile",
    "sre_parse",
    "sre_constants",
    "re",
    "copyreg",
    "_weakrefset",
    "weakref",
    "linecache",
    "warnings",
    "traceback",
    "importlib",
    "zipimport",
    "_bootlocale",
    "locale",
    "string",
    "heapq",
    "contextlib",
    "typing",
    "dataclasses",
    "inspect",
    "ast",
    "dis",
    "opcode",
    "token",
    "tokenize",
    "textwrap",
    "threading",
    "_threading_local",
    "queue",
    "abc",
    "site",
    "sysconfig",
    "_sitebuiltins",
    "encodings.aliases",
    "encodings.utf_8",
    "encodings.ascii",
    "encodings.latin_1",
}


def find_entry_points(entry_dir: str) -> tuple[list[str], list[str]]:
    """Split an entry dir's direct children into (script paths, package names).

    ``modulefinder.run_script`` always scans its argument as ``__main__`` at import
    level 0 — correct for a real script like ``main.py``, but it raises
    ``ImportError: relative importpath too deep`` on any file that itself does a
    package-relative import (``from . import widgets``, as every file in
    ``zkite_py/src/zkite/`` does), because it isn't actually a top-level script. A
    directory containing ``__init__.py`` is a package, not a script, and has to go
    through ``import_hook`` by name instead, so relative imports resolve against its
    real package identity.
    """
    scripts, packages = [], []
    if not os.path.isdir(entry_dir):
        return scripts, packages
    for name in sorted(os.listdir(entry_dir)):
        full = os.path.join(entry_dir, name)
        if os.path.isdir(full) and os.path.isfile(os.path.join(full, "__init__.py")):
            packages.append(name)
        elif name.endswith(".py"):
            scripts.append(full)
    return scripts, packages


def compute_reachable_top_level(stdlib_dir: str, entry_dirs: list[str]) -> set[str]:
    finder = modulefinder.ModuleFinder(path=[stdlib_dir, *entry_dirs])
    for entry_dir in entry_dirs:
        scripts, packages = find_entry_points(entry_dir)
        for f in scripts:
            try:
                finder.run_script(f)
            except SyntaxError as e:
                print("    ! modulefinder could not parse %s: %s" % (f, e), file=sys.stderr)
        for pkg in packages:
            try:
                finder.import_hook(pkg)
            except ImportError as e:
                print("    ! modulefinder could not import package %s: %s" % (pkg, e), file=sys.stderr)
    reachable = {name.split(".")[0] for name in finder.modules if name != "__main__"}
    return reachable | ALWAYS_KEEP


def main() -> int:
    if sys.version_info[:2] != (3, 14):
        print("stdlib pruning requires CPython 3.14; a different version can silently remove required modules", file=sys.stderr)
        return 1
    if len(sys.argv) < 4:
        print(
            "usage: prune_stdlib.py <prefix> <out.zip> <entry_dir>...",
            file=sys.stderr,
        )
        return 2
    prefix, out_path = sys.argv[1], sys.argv[2]
    entry_dirs = sys.argv[3:]
    # Bytecode-only trees cannot be analysed by this source-based pruner. A build
    # used to strip all app .py files before arriving here, leaving nothing to scan.
    # Fail before writing an incomplete stdlib instead of emitting a broken APK.
    for entry_dir in entry_dirs:
        for root, dirs, files in os.walk(entry_dir):
            dirs[:] = [name for name in dirs if name != "__pycache__"]
            for name in files:
                if name.endswith(".pyc") and not os.path.isfile(os.path.join(root, name[:-1])):
                    print("cannot analyse bytecode-only source %s; prune before compiling .py files" % os.path.join(root, name), file=sys.stderr)
                    return 1
    stdlib = os.path.join(prefix, "lib", "python" + PY_VERSION)
    if not os.path.isdir(stdlib):
        print("no stdlib at %s" % stdlib, file=sys.stderr)
        return 1

    reachable = compute_reachable_top_level(stdlib, entry_dirs)

    kept_bytes = dropped_bytes = 0
    kept_files = 0
    dropped_top: dict[str, int] = {}
    kept_top_level: set[str] = set()
    dropped_top_level: set[str] = set()

    os.makedirs(os.path.dirname(os.path.abspath(out_path)) or ".", exist_ok=True)
    with zipfile.ZipFile(out_path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for entry in sorted(os.listdir(stdlib)):
            top_level_name = entry[:-3] if entry.endswith(".py") else entry
            full_path = os.path.join(stdlib, entry)

            if entry in DROP_DIRS:
                size = dir_or_file_size(full_path)
                dropped_bytes += size
                dropped_top[entry] = size
                dropped_top_level.add(entry)
                continue

            if top_level_name not in reachable:
                size = dir_or_file_size(full_path)
                dropped_bytes += size
                dropped_top[top_level_name] = size
                dropped_top_level.add(top_level_name)
                continue

            kept_top_level.add(top_level_name)
            if os.path.isdir(full_path):
                for droot, dirs, files in os.walk(full_path):
                    dirs[:] = [d for d in dirs if d not in DROP_DIRS]
                    for name in files:
                        if name.endswith(DROP_SUFFIXES):
                            dropped_bytes += os.path.getsize(os.path.join(droot, name))
                            continue
                        src = os.path.join(droot, name)
                        rel = os.path.relpath(src, stdlib)
                        arc = "/".join(["lib", "python" + PY_VERSION, rel.replace(os.sep, "/")])
                        z.write(src, arc)
                        kept_bytes += os.path.getsize(src)
                        kept_files += 1
            elif not entry.endswith(DROP_SUFFIXES):
                arc = "/".join(["lib", "python" + PY_VERSION, entry])
                z.write(full_path, arc)
                kept_bytes += os.path.getsize(full_path)
                kept_files += 1

    print("    reachability: %d top-level packages kept, %d dropped" % (
        len(kept_top_level), len(dropped_top_level)))
    print("    kept    %5d files  %7.1f MB" % (kept_files, kept_bytes / 1e6))
    print("    dropped                %7.1f MB" % (dropped_bytes / 1e6))
    for path, size in sorted(dropped_top.items(), key=lambda kv: -kv[1])[:12]:
        print("      - %-28s %6.1f MB" % (path, size / 1e6))
    print("    %s: %.1f MB compressed" % (out_path, os.path.getsize(out_path) / 1e6))
    return 0


def dir_or_file_size(path: str) -> int:
    if os.path.isfile(path):
        return os.path.getsize(path)
    total = 0
    for root, _dirs, files in os.walk(path):
        for f in files:
            try:
                total += os.path.getsize(os.path.join(root, f))
            except OSError:
                pass
    return total


if __name__ == "__main__":
    raise SystemExit(main())
