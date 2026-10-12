"""Imports C-extension-backed stdlib modules and renders what they compute (§11.6).

This is the regression app for the gap §11.6 found: until this session, *no*
extension module was importable on any ZKite backend, because Android's package
installer only extracts `lib/<abi>/lib*.so` and every extension module ships as
`<mod>.cpython-314-<triple>.so`. `import math` therefore killed the app at boot,
with an error overlay rather than anything a build could catch.

Every import below is deliberately a *different* shape of the same dependency:

  math        the module §11.6 was actually found by (`_viewport.py` used it)
  random      a pure-Python module whose C dependency (`_random`) is indirect —
              proves the finder is reached through a normal import chain, not
              only by a direct `import <extension>`
  datetime    same, via `_datetime`
  zlib        a module with real bytes to check, so success is verifiable from
              the screen rather than inferred from the absence of a crash
  binascii    ditto, and its output is exact rather than approximate

and then the three modules that need more than the rename, because their `.so`
carries a DT_NEEDED on a *second* shared library:

  hashlib     -> _hashlib  -> libcrypto_python.so
  ssl         -> _ssl      -> libssl_python.so + libcrypto_python.so
  sqlite3     -> _sqlite3  -> libsqlite3_python.so

Those three support libraries were never staged into the APK at all, so these
modules would still have failed to load after the rename fix on its own — the
rename makes the module file present, the dynamic linker then fails on its
dependency. They are the reason `build_apk.sh` also copies `lib*_python.so` out
of the prefix, and the reason they are checked here rather than assumed.

The rendered values are computed, not hardcoded, so a screenshot is evidence the
extension code actually ran — an app that merely imported successfully and then
printed a literal would look identical.

Selected at APK build time with `ZKITE_APP_MAIN=.../extension_modules.py
scripts/build_apk.sh`. Remember `adb shell pm clear dev.zkite.gpuharness` before
reinstalling — asset unpacking is stamp-guarded (../../PLAN.md §7.9).
"""

import binascii
import datetime
import hashlib
import math
import random
import sqlite3
import ssl
import zlib

from zkite import Column, Row, Text, store

# Seeded so the value on screen is reproducible across runs and can be checked
# against the same expression on the host, rather than just "some number".
random.seed(20260806)


def _sqlite_answer():
    """Run a real query through a real in-memory database.

    `import sqlite3` alone would only prove the module loaded; opening a
    connection and executing a statement is what actually exercises
    `libsqlite3_python.so` behind it.
    """
    con = sqlite3.connect(":memory:")
    try:
        return con.execute("select 6 * 7").fetchone()[0]
    finally:
        con.close()


CHECKS = [
    ("math", "sqrt(1764) = %g" % math.sqrt(1764)),
    ("random", "randint(0,999) = %d" % random.randint(0, 999)),
    ("datetime", "epoch+1e9s = %s" % datetime.datetime.fromtimestamp(1_000_000_000).date()),
    ("zlib", "crc32(b'zkite') = %d" % zlib.crc32(b"zkite")),
    ("binascii", "hexlify(b'zkite') = %s" % binascii.hexlify(b"zkite").decode()),
    # The three with transitive native dependencies — see the module docstring.
    ("hashlib", "sha256(b'zkite')[:12] = %s" % hashlib.sha256(b"zkite").hexdigest()[:12]),
    ("ssl", ssl.OPENSSL_VERSION),
    ("sqlite3", "select 6*7 -> %s" % _sqlite_answer()),
]


@store
class DemoState:
    n: int = 0


def view(s: DemoState):
    def row(name, value):
        return Row(
            "items-center justify-between px-4 py-3 bg-slate-900 rounded-lg",
            Text("text-white font-bold", name),
            Text("text-slate-300 text-sm", value),
        )

    return Column(
        "flex-1 bg-slate-950 gap-3 p-4 justify-center",
        Text(
            "text-xl font-bold text-white",
            "%d C-extension modules imported" % len(CHECKS),
        ),
        *[row(name, value) for name, value in CHECKS],
    )
