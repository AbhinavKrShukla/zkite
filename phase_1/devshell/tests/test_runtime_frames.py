"""Exercise shipping Python glue, including store-driven frames and error recovery.

Each case runs in a fresh interpreter: the glue owns stdout/stderr, imports `main`,
and registers process-global stores just as it does inside an Android shell.
"""

import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
RUNTIME = ROOT / "phase_1/devshell/src/runtime.py"
ZKITE_PACKAGE = ROOT / "phase_1/zkite_py/src"

APP = """
from zkite import Button, Column, Text, store
from zkite.widgets import Widget

@store
class State:
    value: float = 1.0
    broken: bool = False

class Data(Widget):
    def _tree_attrs(self):
        return {"nested": {"value": self.props["value"]}}

def view(s: State):
    if s.broken:
        raise RuntimeError("render failed after an event")
    return Column("flex-1",
        Button("", "change", key="change", on_tap=lambda v: s.update(value=float(v))),
        Button("", "break", key="break", on_tap=lambda: s.update(broken=True)),
        Text("", str(s.value)), Data("", value=s.value))
"""


class RuntimeFrameTests(unittest.TestCase):
    def run_glue(self, body, source=APP):
        with tempfile.TemporaryDirectory() as directory:
            Path(directory, "main.py").write_text(textwrap.dedent(source))
            script = f"""
import json, sys, threading
from pathlib import Path
sys.path.insert(0, {str(ZKITE_PACKAGE)!r})
original_stdout, original_stderr = sys.stdout, sys.stderr
ns = {{"__name__": "zkite_test_runtime"}}
try:
    exec(compile(Path({str(RUNTIME)!r}).read_text(), {str(RUNTIME)!r}, "exec"), ns)
finally:
    sys.stdout, sys.stderr = original_stdout, original_stderr
app_dir = {directory!r}
zkite_dir = {str(ZKITE_PACKAGE)!r}
def strict_json(frame):
    def invalid(value):
        raise AssertionError("non-JSON constant escaped: " + value)
    return json.loads(frame, parse_constant=invalid)
def has_overlay(frame):
    return "__zkite_error_overlay" in frame
{textwrap.dedent(body)}
"""
            result = subprocess.run([sys.executable, "-c", script], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_nonfinite_values_in_initial_tree_show_a_valid_error_frame(self):
        for value in ("nan", "inf", "-inf"):
            with self.subTest(value=value):
                self.run_glue("""
frame = ns["_zkite_setup_tree"](zkite_dir, app_dir)
strict_json(frame)
assert has_overlay(frame), frame
assert "JSON" in ns["_zkite_last_error"]()
""", APP.replace("value: float = 1.0", f"value: float = float({value!r})"))

    def test_dispatch_rejects_nonfinite_values_and_recovers(self):
        self.run_glue("""
strict_json(ns["_zkite_setup_tree"](zkite_dir, app_dir))
for value in ("nan", "inf", "-inf"):
    frame = ns["_zkite_dispatch_tree"]("change", "tap", value)
    strict_json(frame)
    assert has_overlay(frame), frame
    assert ns["_zkite_last_error"]() is not None
    frame = ns["_zkite_dispatch_tree"]("change", "tap", "2")
    strict_json(frame)
    assert not has_overlay(frame), frame
    assert ns["_zkite_last_error"]() is None
""")

    def test_a_background_invalid_frame_does_not_raise_from_store_update(self):
        self.run_glue("""
ns["_zkite_setup_tree"](zkite_dir, app_dir)
frames = []
failures = []
ns["_zkite_native_frame_ready"] = frames.append
from zkite._store import get_instance
state = get_instance(ns["_app"].module.State)
def update():
    try:
        state.update(value=float("nan"))
    except Exception as error:
        failures.append(error)
worker = threading.Thread(target=update)
worker.start()
worker.join(timeout=5)
assert not worker.is_alive()
assert not failures, failures
assert len(frames) == 1, frames
strict_json(frames[0])
assert has_overlay(frames[0])
assert ns["_zkite_last_error"]() is not None
state.update(value=3)
assert len(frames) == 2
assert not has_overlay(frames[-1])
assert ns["_zkite_last_error"]() is None
""")

    def test_repeated_invalid_frames_are_logged_once_until_recovery(self):
        self.run_glue("""
import contextlib, io
ns["_zkite_setup_tree"](zkite_dir, app_dir)
logs = io.StringIO()
with contextlib.redirect_stderr(logs):
    for _ in range(5):
        ns["_zkite_dispatch_tree"]("change", "tap", "nan")
    assert logs.getvalue().count("ZKITEERROR view() raised:") == 1, logs.getvalue()
    ns["_zkite_dispatch_tree"]("change", "tap", "4")
assert logs.getvalue().count("ZKITEERROR cleared:") == 1, logs.getvalue()
""")

    def test_event_driven_view_failure_returns_the_overlay_on_both_backends(self):
        for suffix in ("", "_tree"):
            with self.subTest(backend=suffix or "html"):
                self.run_glue(f"""
ns["_zkite_setup{suffix}"](zkite_dir, app_dir)
frame = ns["_zkite_dispatch{suffix}"]("break", "tap", "")
assert has_overlay(frame), frame
assert "render failed after an event" in ns["_zkite_last_error"]()
# Repeated failures retain one good frame beneath one error layer.
for _ in range(3):
    frame = ns["_zkite_dispatch{suffix}"]("break", "tap", "")
    assert frame.count("__zkite_error_overlay") == 1, frame
from zkite._store import get_instance
get_instance(ns["_app"].module.State).update(broken=False)
assert ns["_zkite_last_error"]() is None
assert not has_overlay(str(ns["_app"].last_tree if {bool(suffix)!r} else ns["_app"].last_html))
""")


if __name__ == "__main__":
    unittest.main()
