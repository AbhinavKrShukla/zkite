#!/usr/bin/env python3
"""Build the ``assets/python-stdlib.zip`` payload from a python.org Android prefix.

This is the phase-0 ancestor of ``zkite build``'s ``--prune-stdlib`` (plan1.md §7). It is
deliberately a *static allow/deny list*, not the reachability analysis the real thing
wants — the spike only needs to answer "does a pruned stdlib still boot", and it prints
what it dropped so the failure mode stays diagnosable.

Layout inside the zip mirrors the prefix, so extracting it into ``<prefix>`` yields
``<prefix>/lib/python3.14/...`` and ``PYTHONHOME=<prefix>`` just works.

``lib-dynload`` is excluded on purpose: since Android 10 an app may only ``dlopen`` from
its own native library directory, so the extension modules are staged into
``lib/<abi>/`` by ``build_apk.sh`` instead.
"""

from __future__ import annotations

import os
import sys
import zipfile

PY_VERSION = "3.14"

# Big, and useless on a phone.
DROP_DIRS = {
    "test",           # ~20 MB of CPython's own test suite
    "idlelib",        # a Tk IDE
    "tkinter",        # no Tk on Android
    "turtledemo",
    "lib2to3",
    "pydoc_data",
    "ensurepip",      # a bundled pip wheel; zkite resolves wheels host-side
    "config-%s" % PY_VERSION,
    "lib-dynload",    # staged into lib/<abi>/ — see module docstring
    "__pycache__",
    "site-packages",  # recreated empty at runtime for the .pth hook
}

DROP_SUFFIXES = (".pyc", ".pyo", ".whl", ".a", ".o")


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: make_stdlib_zip.py <prefix> <out.zip>", file=sys.stderr)
        return 2
    prefix, out_path = sys.argv[1], sys.argv[2]
    stdlib = os.path.join(prefix, "lib", "python" + PY_VERSION)
    if not os.path.isdir(stdlib):
        print("no stdlib at %s" % stdlib, file=sys.stderr)
        return 1

    kept_bytes = dropped_bytes = 0
    kept_files = 0
    dropped_top: dict[str, int] = {}

    os.makedirs(os.path.dirname(os.path.abspath(out_path)) or ".", exist_ok=True)
    with zipfile.ZipFile(out_path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for root, dirs, files in os.walk(stdlib):
            rel_root = os.path.relpath(root, stdlib)
            parts = [] if rel_root == "." else rel_root.split(os.sep)

            # Account for what we prune before removing it from the walk.
            for d in list(dirs):
                if d in DROP_DIRS:
                    size = dir_size(os.path.join(root, d))
                    dropped_bytes += size
                    dropped_top[os.path.join(*(parts + [d]))] = size
                    dirs.remove(d)

            for name in files:
                if name.endswith(DROP_SUFFIXES):
                    dropped_bytes += os.path.getsize(os.path.join(root, name))
                    continue
                src = os.path.join(root, name)
                arc = "/".join(["lib", "python" + PY_VERSION] + parts + [name])
                z.write(src, arc)
                kept_bytes += os.path.getsize(src)
                kept_files += 1

    print("    kept    %5d files  %7.1f MB" % (kept_files, kept_bytes / 1e6))
    print("    dropped                %7.1f MB" % (dropped_bytes / 1e6))
    for path, size in sorted(dropped_top.items(), key=lambda kv: -kv[1])[:8]:
        print("      - %-28s %6.1f MB" % (path, size / 1e6))
    print("    %s: %.1f MB compressed" % (out_path, os.path.getsize(out_path) / 1e6))
    return 0


def dir_size(path: str) -> int:
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
