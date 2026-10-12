#!/usr/bin/env python3
"""zkite build's pyc-compilation step (plan1.md §7 step 2: ".py -> .pyc
(source-strippable)").

Must run under a host Python whose bytecode magic number matches the target's
embedded interpreter (see ../../PLAN.md §1.4). Refuse a mismatched interpreter
before removing any source files.

Produces "legacy"-location ``.pyc`` files (``pkg/mod.pyc`` next to where
``pkg/mod.py`` was, not ``pkg/__pycache__/mod.cpython-314.pyc``) because that's the
layout ``zipimport`` has always been able to load directly from a zip, with no
``__pycache__`` involved — matching plan1.md §7's "assets/app.zip (zipimport-able)".
"""

from __future__ import annotations

import os
import py_compile
import sys


def main() -> int:
    if sys.version_info[:2] != (3, 14):
        print("bytecode compilation requires CPython 3.14 to match the Android runtime", file=sys.stderr)
        return 1
    if len(sys.argv) != 2:
        print("usage: compile_pyc.py <dir>", file=sys.stderr)
        return 2
    root = sys.argv[1]

    compiled = 0
    errors: list[str] = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d != "__pycache__"]
        for name in filenames:
            if not name.endswith(".py"):
                continue
            src = os.path.join(dirpath, name)
            dst = src[:-3] + ".pyc"
            try:
                py_compile.compile(src, cfile=dst, dfile=name, doraise=True)
            except py_compile.PyCompileError as e:
                errors.append(f"{src}: {e}")
                continue
            os.remove(src)
            compiled += 1

    print(f"    compiled {compiled} files to .pyc" + (f", {len(errors)} error(s)" if errors else ""))
    for e in errors:
        print(f"      ! {e}")
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
