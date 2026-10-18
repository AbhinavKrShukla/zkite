"""Run pip's backtracking resolver with Android markers, including package extras.

The pip distribution is supplied by the CLI; no packaging install is needed.
"""
import json
import sys

pip_path, target_json, *args = sys.argv[1:]
if pip_path:
    sys.path.insert(0, pip_path)
from pip._vendor.packaging import markers
from pip._internal.cli.main import main

target = json.loads(target_json)
# Replace the entire environment, rather than merging host OS/kernel values.
# pip supplies each candidate's selected extras to Marker.evaluate itself.
markers.default_environment = lambda: dict(target)
raise SystemExit(main(args))
