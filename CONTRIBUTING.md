# Contributing to ZKite

Start with the [source map](README.md#source-map), [user guide](Documentation.md),
and [release scope](docs/status.md). The repository includes the framework's source,
tests, templates, and build tooling. Generated binaries and downloaded toolchains
are excluded from Git and recreated by the supplied scripts.

## How the pieces connect

The Python framework builds widget trees and dispatches events. The Python bridge
passes those trees to the Rust layout/renderer, hosted by the Android activity.
The CLI scaffolds projects and prepares previews. For standalone apps it resolves
Python dependencies, packages app/framework/runtime assets, and invokes the APK/AAB
bundler. Runtime binaries are built by maintainers and delivered in the payload
wheel, so ordinary app authors do not compile Rust or install the Android NDK.

## Development setup

Use CPython 3.14, stable Rust, JDK 17, and `protoc` on PATH. Linux native-renderer
tests also need DejaVu fonts. Use a virtual environment for Python tools:

```bash
python3.14 -m venv .venv
. .venv/bin/activate
python -m pip install pytest
python scripts/fetch_pack.py
cargo build --locked --manifest-path phase_1/cli/Cargo.toml
```

On Windows, create the environment with `py -3.14 -m venv .venv` and invoke
`.venv\Scripts\python.exe` explicitly. The host workflow documents platform setup.

## Tests

From the repository root with the virtual environment active:

```bash
python scripts/check_host.py
python -m pytest phase_1/zkite_py/tests -q
```

For the pytest command, set `PYTHONPATH` to `phase_1/zkite_py/src` first (`export
PYTHONPATH=phase_1/zkite_py/src` on Linux; `$env:PYTHONPATH='phase_1/zkite_py/src'`
in PowerShell). On Linux, use `python scripts/check_host.py --native` to include
renderer and embedded-Python bridge checks. Some payload-dependent CLI tests need
the prebuilt artifacts below. Live-index tests are opt-in; missing optional
external tools can skip their dependent checks.

## Build the Android payload

The release build needs Android NDK 27.3.13750724, platform 34, build-tools 34.0.0,
JDK 17, and the two Android Rust targets. Set `ANDROID_HOME` and `JAVA_HOME`, put
JDK 17's `bin` on PATH, and follow the commands in
[framework builds](Documentation.md#12-framework-contributors-and-release-builders).
The fetch scripts obtain the pinned pack source and official Android CPython
binaries. The candidate workflow is the automated reference for a clean build.

The supported compile platform and generated app target SDK differ deliberately:
apps target API 36. Rebuild native artifacts after changing their source; provenance
checks reject stale artifacts. Do not bypass them or relabel binaries for another OS.

## Propose a change

Keep changes focused, update public documentation when behavior changes, and add
regression coverage for meaningful behavior. Explain the problem, the resulting
behavior, and the checks performed in the pull request. Report device-specific
bugs with the ZKite version, Android/device version, backend, reproduction steps,
and relevant logs with private data removed.

Commit source, intended assets, dependency manifests and lockfiles. Do not commit
venvs, toolchains, build output, tokens, signing keys, or personal test apps. Check
`git status --short` and `git diff --cached` before pushing. Ignore rules do not
remove secrets already recorded in Git history.

Use [PUBLISHING.md](PUBLISHING.md) for release preparation. PyPI publication is a
separate maintainer action. Contributions to original framework code use the
repository's MIT license; preserve third-party license notices.
