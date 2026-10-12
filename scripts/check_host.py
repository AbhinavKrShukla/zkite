#!/usr/bin/env python3
"""Repeatable host gates. Android rendering and device lifecycle need device tests."""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

REPO = Path(__file__).resolve().parents[1]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", action="store_true", help="also test Linux font/CPython native crates")
    parser.add_argument("--python-only", action="store_true")
    args = parser.parse_args()
    env = os.environ.copy()
    env["PYTHONPATH"] = str(REPO / "phase_1/zkite_py/src")
    env["PYO3_PYTHON"] = sys.executable
    env.setdefault("ZKITE_PYTHON", sys.executable)
    env.setdefault("CARGO_BUILD_JOBS", "2")
    env.setdefault("CARGO_TARGET_DIR", str(REPO / "target/host"))
    for candidate in ("/usr/share/fonts/truetype/dejavu", "/usr/share/fonts/TTF"):
        if Path(candidate, "DejaVuSans.ttf").is_file():
            env.setdefault("ZKITE_TEST_FONT_DIR", candidate)
            break
    def run(command):
        print("\n+ " + " ".join(map(str, command)), flush=True)
        subprocess.run(command, cwd=REPO, env=env, check=True)
    for suite in ("phase_1/zkite_py/tests", "phase_1/devshell/tests", "phase_2/pybuild/tests", "phase_1/android_shared/tests", "tests"):
        run([sys.executable, "-m", "unittest", "discover", "-s", suite])
    if args.python_only:
        return
    with tempfile.TemporaryDirectory(prefix="zkite-surface-tests-") as temporary:
        test_binary = Path(temporary) / ("surface-tests.exe" if os.name == "nt" else "surface-tests")
        run(["rustc", "--edition=2021", "--test", "phase_3/gpu_harness/src/surface_lifecycle.rs", "-o", str(test_binary)])
        run([str(test_binary)])
    run([sys.executable, "scripts/fetch_pack.py"])
    for crate in ("phase_0/spike3_twcore", "phase_2/bundler", "phase_1/cli",
                  "phase_3/anim", "phase_3/frametiming", "phase_3/imagestore"):
        command = ["cargo", "test", "--locked", "--manifest-path", f"{crate}/Cargo.toml"]
        if crate == "phase_2/bundler":
            command.extend(["--features", "aab"])
        run(command)
    if args.native:
        for crate in ("renderer", "pybridge"):
            run(["cargo", "test", "--locked", "--manifest-path", f"phase_3/{crate}/Cargo.toml"])


if __name__ == "__main__":
    main()
