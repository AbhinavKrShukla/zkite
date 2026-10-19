#!/usr/bin/env python3
"""Assemble framework, host CLI and host-independent Android payload wheels.

Installing zkite pulls matching zkite-framework and zkite-payload distributions. Names are
configurable release candidates; this script does not publish to a package index.

Use --verify to install in a fresh venv, import the Python API, and build a real
native APK outside the checkout. --platform labels an already-built host binary;
it cannot make that binary compatible with a different OS or older glibc.

    stage_payload.py /tmp/zkite-payload
    build_wheels.py --payload /tmp/zkite-payload --out /tmp/wheels --verify
"""

from __future__ import annotations

import argparse
import base64
import csv
import hashlib
import io
import os
import re
import subprocess
import sys
import sysconfig
import tempfile
import zipfile

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

# The directory name `payload.rs::BESIDE_BINARY_DIR` probes for. This is the one string
# that genuinely couples the two, and it is not a distribution name.
PAYLOAD_DIR_NAME = "zkite-payload"


def description(args) -> str:
    """A valid project description for package indexes, shared by the wheel set."""
    return (
        "Project-URL: Source, https://github.com/AbhinavKrShukla/kite-v1\n"
        "Description-Content-Type: text/markdown\n\n"
        "# ZKite\n\n"
        "Build native Android applications in Python with reactive widgets, state, "
        "hot reload, standalone APK/AAB packaging, device services, and automatic "
        "Android permission inference.\n\n"
        f"Install the complete framework with `python -m pip install {args.cli_name}`. "
        "Use CPython 3.14 and Android platform-tools (adb). The distribution supplies "
        "prebuilt Android runtimes; application authors do not need Rust or the Android NDK.\n\n"
        "Commands are `zkite new hello`, `zkite dev`, `zkite run`, and `zkite build apk`. "
        "Python applications import `zkite`. The native renderer is the default.\n\n"
        "See Documentation.md, PRODUCTION_READINESS.md, "
        "and PUBLISHING.md in the [source repository](https://github.com/AbhinavKrShukla/kite-v1) "
        "for supported features and release checks.\n"
    )


def wheel_hash(data: bytes) -> str:
    """RECORD's hash column: urlsafe base64 of the sha256, `=` padding stripped."""
    digest = hashlib.sha256(data).digest()
    return "sha256=" + base64.urlsafe_b64encode(digest).decode().rstrip("=")


class WheelWriter:
    """A PEP 427 wheel, written directly.

    `python -m build` is the usual tool and is not used here on purpose: it wants a
    source tree with a backend, and what is being packaged is a *staged directory* plus
    a cross-compiled binary that no Python build backend produced. Writing the zip is
    about eighty lines and removes a dependency this repo would otherwise need on the
    machine that cuts releases. The output is checked the only way that matters — real
    `pip install` into a real venv (see `--verify`).
    """

    def __init__(self, dist: str, version: str, tag: str, purelib: bool):
        self.dist = dist
        self.version = version
        self.tag = tag
        self.purelib = purelib
        self.entries: list[tuple[str, bytes]] = []

    @property
    def dist_info(self) -> str:
        return f"{re.sub(r'[-_.]+', '_', self.dist)}-{self.version}.dist-info"

    @property
    def data_dir(self) -> str:
        return f"{re.sub(r'[-_.]+', '_', self.dist)}-{self.version}.data"

    def add_bytes(self, arcname: str, data: bytes) -> None:
        self.entries.append((arcname, data))

    def add_file(self, arcname: str, path: str) -> None:
        with open(path, "rb") as fh:
            self.add_bytes(arcname, fh.read())

    def add_tree(self, arc_prefix: str, root: str) -> int:
        n = 0
        for dirpath, dirnames, filenames in os.walk(root):
            dirnames[:] = sorted(d for d in dirnames if d not in {
                "__pycache__", ".git", ".pytest_cache", "node_modules", "target"
            } and not os.path.islink(os.path.join(dirpath, d)))
            for name in sorted(filenames):
                if name.endswith((".pyc", ".pyo")):
                    continue
                abs_path = os.path.join(dirpath, name)
                if os.path.islink(abs_path) or not os.path.isfile(abs_path):
                    continue
                rel = os.path.relpath(abs_path, root).replace(os.sep, "/")
                self.add_file(f"{arc_prefix}/{rel}", abs_path)
                n += 1
        return n

    def write(self, out_dir: str, metadata: str) -> str:
        # The project license covers original ZKite code. Bundled dependencies
        # retain the separate notices collected from the release's actual sources.
        metadata = metadata.replace("Metadata-Version: 2.1\n", "Metadata-Version: 2.4\n", 1)
        license_headers = "License-Expression: MIT\n"
        for name in ("LICENSE", "THIRD_PARTY_NOTICES.txt"):
            self.add_file(f"{self.dist_info}/licenses/{name}", os.path.join(REPO, name))
            license_headers += f"License-File: {name}\n"
        metadata = metadata.replace("Description-Content-Type:", license_headers + "Description-Content-Type:", 1)
        self.add_bytes(f"{self.dist_info}/METADATA", metadata.encode())
        self.add_bytes(
            f"{self.dist_info}/WHEEL",
            (
                "Wheel-Version: 1.0\n"
                "Generator: zkite phase_1/cli/scripts/build_wheels.py\n"
                f"Root-Is-Purelib: {'true' if self.purelib else 'false'}\n"
                f"Tag: {self.tag}\n"
            ).encode(),
        )

        record = io.StringIO()
        writer = csv.writer(record, lineterminator="\n")
        for arcname, data in self.entries:
            writer.writerow([arcname, wheel_hash(data), len(data)])
        record_name = f"{self.dist_info}/RECORD"
        # RECORD lists itself with no hash and no size — PEP 427, and pip rejects a
        # wheel that hashes it.
        writer.writerow([record_name, "", ""])

        name = f"{re.sub(r'[-_.]+', '_', self.dist)}-{self.version}-{self.tag}.whl"
        path = os.path.join(out_dir, name)
        os.makedirs(out_dir, exist_ok=True)
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
            for arcname, data in self.entries:
                info = zipfile.ZipInfo(arcname, date_time=(1980, 1, 1, 0, 0, 0))
                info.create_system = 3  # POSIX modes, including when built on Windows.
                info.compress_type = zipfile.ZIP_DEFLATED
                # The binary under `.data/scripts/` must survive install as executable.
                # `S_IFREG` is not decoration: pip decides with
                # `stat.S_ISREG(mode) and mode & 0o111`, so a bare `0o755` fails the
                # *file-type* half of that test and pip installs a non-executable
                # binary — and reports success. Measured, not read: the first run of
                # this script produced a wheel that pip installed happily and whose
                # `bin/zkite` was mode 664. §15.6.1's list of ways "the build succeeded"
                # has been wrong, in a new place.
                executable = arcname.startswith(f"{self.data_dir}/scripts/")
                info.external_attr = (0o100755 if executable else 0o100644) << 16
                z.writestr(info, data)
            info = zipfile.ZipInfo(record_name, date_time=(1980, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            z.writestr(info, record.getvalue())
        return path


def platform_tag() -> str:
    """This host's wheel platform tag, e.g. `linux_x86_64`.

    **Not a manylinux tag**, and that matters for a real release rather than for this
    script: PyPI rejects a bare `linux_x86_64` upload, so shipping the CLI wheel means
    building it in a manylinux image or auditwheel-repairing it. Nothing here does
    that, and §37's known gaps say so — the binary links only libc/libm/libgcc_s
    (§23.3), so it is a packaging step, not a portability problem.
    """
    return sysconfig.get_platform().replace("-", "_").replace(".", "_")


def build_payload_wheel(args) -> str:
    w = WheelWriter(args.payload_name, args.version, "py3-none-any", purelib=True)
    n = w.add_tree(f"{w.data_dir}/data/{PAYLOAD_DIR_NAME}", args.payload)
    if n == 0:
        sys.exit(f"nothing under {args.payload} — run stage_payload.sh first")
    metadata = (
        "Metadata-Version: 2.1\n"
        f"Name: {args.payload_name}\n"
        f"Version: {args.version}\n"
        "Summary: Android build payload for zkite (shell artifacts, Python prefix, "
        "framework source). Host-independent.\n"
        "Requires-Python: >=3.14\n"
    ) + description(args)
    path = w.write(args.out, metadata)
    print(f"  {os.path.basename(path)}  {os.path.getsize(path) / 1e6:.1f} MB  ({n} files)")
    return path


def build_cli_wheel(args) -> str:
    platform = args.platform or platform_tag()
    executable = "zkite.exe" if platform.startswith("win") else "zkite"
    binary = args.binary or os.path.join(REPO, "phase_1/cli/target/release", executable)
    if not os.path.isfile(binary):
        sys.exit(f"no zkite binary at {binary} — cargo build --release first")
    tag = f"py3-none-{platform}"
    w = WheelWriter(args.cli_name, args.version, tag, purelib=False)
    w.add_file(f"{w.data_dir}/scripts/{executable}", binary)
    metadata = (
        "Metadata-Version: 2.1\n"
        f"Name: {args.cli_name}\n"
        f"Version: {args.version}\n"
        "Summary: zkite CLI — native Android apps from Python.\n"
        "Requires-Python: >=3.14,<3.15\n"
        f"Requires-Dist: {args.payload_name}=={args.version}\n"
        f"Requires-Dist: {args.framework_name}=={args.version}\n"
    ) + description(args)
    path = w.write(args.out, metadata)
    print(f"  {os.path.basename(path)}  {os.path.getsize(path) / 1e6:.1f} MB")
    return path


def build_framework_wheel(args) -> str:
    """Install the same Python API that is embedded into the Android payload."""
    source = os.path.join(args.payload, "phase_1", "zkite_py", "src", "zkite")
    if not os.path.isfile(os.path.join(source, "__init__.py")):
        raise ValueError(f"missing framework source: {source}")
    w = WheelWriter(args.framework_name, args.version, "py3-none-any", purelib=True)
    w.add_tree("zkite", source)
    path = w.write(args.out, (
        "Metadata-Version: 2.1\n"
        f"Name: {args.framework_name}\n"
        f"Version: {args.version}\n"
        "Summary: Python widgets, state and navigation for ZKite Android apps.\n"
        "Requires-Python: >=3.10\n"
    ) + description(args))
    print(f"  {os.path.basename(path)}  {os.path.getsize(path) / 1e6:.1f} MB")
    return path


def verify(args, wheels: list[str]) -> int:
    """Install into a throwaway venv and make the binary find its own payload.

    The check that matters is not "pip accepted the zip" — it is `zkite doctor`
    reporting `beside the zkite binary`. If it reports the checkout instead, the wheel
    installed and the packaging did nothing, which is precisely the silent failure
    §23.2 built `Origin` to expose.
    """
    with tempfile.TemporaryDirectory(prefix="zkite-wheel-check-") as venv:
        return verify_install(args, wheels, venv)


def verify_install(args, wheels: list[str], venv: str) -> int:
    subprocess.run([sys.executable, "-m", "venv", venv], check=True)
    scripts = os.path.join(venv, "Scripts" if os.name == "nt" else "bin")
    python = os.path.join(scripts, "python.exe" if os.name == "nt" else "python")
    subprocess.run([python, "-m", "pip", "install", "--quiet", "--no-index",
                    *map(os.path.abspath, wheels)], check=True)

    zkite = os.path.join(scripts, "zkite.exe" if os.name == "nt" else "zkite")
    if not os.path.isfile(zkite):
        print(f"FAIL: no {zkite} after install", file=sys.stderr)
        return 1
    payload_dir = os.path.join(venv, PAYLOAD_DIR_NAME)
    if not os.path.isdir(payload_dir):
        print(f"FAIL: no {payload_dir} after install", file=sys.stderr)
        return 1

    # `cwd` and a scrubbed environment so nothing about this checkout can be what
    # answers: no ZKITE_PAYLOAD_ROOT, and run from somewhere else entirely.
    env = {k: v for k, v in os.environ.items()
           if k not in {"ZKITE_PAYLOAD_ROOT", "PYTHONPATH", "ZKITE_PYTHON"}}
    env.update(VIRTUAL_ENV=venv, ZKITE_PYTHON=python)
    env["PATH"] = scripts + os.pathsep + env.get("PATH", "")
    subprocess.run([python, "-I", "-c", "from zkite import App, Column, Text, store"],
                   cwd=venv, env=env, check=True)
    result = subprocess.run(
        [zkite, "doctor"], cwd=venv, env=env, capture_output=True, text=True
    )
    out = result.stdout
    for line in out.splitlines():
        if "adb on PATH" in line:
            break
        print(line)
    if "beside the zkite binary" not in out:
        print(
            "FAIL: the installed zkite did not resolve the wheel's own payload — it "
            "found something else, which on this machine means the checkout.",
            file=sys.stderr,
        )
        return 1
    print("\n  ok: the installed binary resolved the wheel's own payload")
    # A doctor failure can be just an absent device. Build a real standalone APK
    # instead of mistaking a resolver message for proof the wheel actually works.
    subprocess.run([zkite, "new", "wheel_smoke"], cwd=venv, env=env, check=True)
    subprocess.run([zkite, "build", "apk", "--abi", args.verify_abi],
                   cwd=os.path.join(venv, "wheel_smoke"), env=env, check=True)
    print("  ok: installed Python API imports and native APK builds outside the checkout")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--payload", required=True, help="a directory from stage_payload.sh")
    ap.add_argument("--out", required=True, help="where to write the .whl files")
    ap.add_argument("--version", default="1.2.0")
    # Local candidate names. Public release requires ownership of all three names.
    ap.add_argument("--payload-name", default="zkite-payload")
    ap.add_argument("--cli-name", default="zkite")
    ap.add_argument("--framework-name", default="zkite-framework")
    ap.add_argument("--platform", help="target wheel platform; label only, never cross-compiles")
    ap.add_argument("--verify-abi", choices=["arm64-v8a", "x86_64"], default="arm64-v8a")
    ap.add_argument("--binary", help="the zkite binary (default: this checkout's release build)")
    ap.add_argument("--verify", action="store_true", help="pip install into a venv and check")
    args = ap.parse_args()
    for name in (args.cli_name, args.payload_name, args.framework_name):
        if not re.fullmatch(r"[A-Za-z0-9]+(?:[._-][A-Za-z0-9]+)*", name):
            ap.error(f"invalid distribution name: {name!r}")
    if not re.fullmatch(r"[0-9]+(?:\.[0-9]+)*(?:(?:a|b|rc)[0-9]+)?(?:\.post[0-9]+)?(?:\.dev[0-9]+)?", args.version):
        ap.error("version must be a numeric PEP 440 release version")
    if args.platform and not re.fullmatch(r"[A-Za-z0-9_]+", args.platform):
        ap.error("platform must be one wheel platform tag")

    print("wheels:")
    wheels = [build_payload_wheel(args), build_framework_wheel(args), build_cli_wheel(args)]
    if args.verify:
        print()
        return verify(args, wheels)
    return 0


if __name__ == "__main__":
    sys.exit(main())
