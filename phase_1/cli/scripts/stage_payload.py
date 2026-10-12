#!/usr/bin/env python3
"""Stage prebuilt Android artifacts using the CLI's authoritative manifest.

Release tooling only: no native compilation occurs here. Dev Shells are included
by default so the installed distribution supports the first `zkite dev` invocation.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tempfile

REPO = Path(__file__).resolve().parents[3]


def relative_path(value: str) -> Path:
    path = PurePosixPath(value)
    if not value or path.is_absolute() or ".." in path.parts or "\\" in value or ":" in value:
        raise ValueError(f"unsafe payload path: {value!r}")
    return Path(*path.parts)


def manifest(zkite: Path, backend: str, abis: list[str], devshell: bool) -> list[tuple[str, Path, str]]:
    command = [str(zkite), "payload-manifest", "--backend", backend]
    for abi in abis:
        command.extend(["--abi", abi])
    if devshell:
        command.append("--devshell")
    output = subprocess.run(command, check=True, capture_output=True, text=True).stdout
    entries = []
    for line in output.splitlines():
        fields = line.split("\t")
        if len(fields) not in (2, 3) or fields[0] not in {"file", "dir", "glob"}:
            raise ValueError(f"invalid manifest entry: {line!r}")
        kind, value = fields[:2]
        pattern = fields[2] if len(fields) == 3 else ""
        if kind == "glob" and (not pattern or "/" in pattern or "\\" in pattern):
            raise ValueError(f"invalid manifest glob: {line!r}")
        entries.append((kind, relative_path(value), pattern))
    if not entries:
        raise ValueError("zkite returned an empty payload manifest")
    return entries


def stage(repo: Path, dest: Path, entries: list[tuple[str, Path, str]], abis: list[str]) -> None:
    # No rm -rf of a caller-supplied directory. A failed stage leaves the last
    # payload untouched, and an existing destination must be removed deliberately.
    repo, dest = repo.resolve(), dest.resolve()
    if dest == repo or dest in repo.parents:
        raise ValueError("payload destination must not contain the source checkout")
    if dest.exists():
        raise ValueError(f"destination already exists: {dest}; choose a new directory")
    for kind, relative, _ in entries:
        source = repo / relative
        if source.is_symlink() or not source.exists() or not source.resolve().is_relative_to(repo):
            raise ValueError(f"missing or unsafe artifact: {source}; build the release artifacts first")
        if (kind == "file") != source.is_file():
            raise ValueError(f"manifest type mismatch: {source}")
        if source.is_dir() and dest.is_relative_to(source):
            raise ValueError("payload destination must not be inside a staged source directory")
    dest.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".zkite-stage-", dir=dest.parent) as temporary:
        staged = Path(temporary) / "payload"
        staged.mkdir()
        def ignore(directory, names):
            return [name for name in names if name in {
                "__pycache__", "node_modules", ".git", "target", ".pytest_cache"
            } or name.endswith((".pyc", ".pyo")) or (Path(directory) / name).is_symlink()]
        for kind, relative, pattern in entries:
            source, target = repo / relative, staged / relative
            if kind == "dir":
                shutil.copytree(source, target, dirs_exist_ok=True, ignore=ignore)
            elif kind == "file":
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, target)
            else:
                matches = sorted(source.glob(pattern))
                if not matches:
                    raise ValueError(f"no artifacts matched {source / pattern}")
                target.mkdir(parents=True, exist_ok=True)
                for match in matches:
                    if not match.is_file() or match.is_symlink():
                        raise ValueError(f"unsafe glob artifact: {match}")
                    shutil.copy2(match, target / match.name)
        stdlib = staged / "phase_0/vendor/python-x86_64/prefix/lib/python3.14"
        shutil.rmtree(stdlib / "test", ignore_errors=True)
        if "x86_64" not in abis:
            shutil.rmtree(stdlib / "lib-dynload", ignore_errors=True)
        # Browser tests are never runtime inputs, and can include local screenshots.
        shutil.rmtree(staged / "phase_1/webruntime/test", ignore_errors=True)
        staged.rename(dest)
    size = sum(p.stat().st_size for p in dest.rglob("*") if p.is_file())
    print(f"Payload staged: {dest} ({size / 1e6:.1f} MB)")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dest", type=Path)
    parser.add_argument("--repo", type=Path, default=REPO)
    parser.add_argument("--zkite", type=Path)
    parser.add_argument("--abi", choices=["arm64-v8a", "x86_64"], action="append")
    parser.add_argument("--backend", choices=["native", "webview"], action="append")
    parser.add_argument("--without-devshell", action="store_true", help="build-only distribution")
    args = parser.parse_args()
    abis = args.abi or os.environ.get("ABIS", "arm64-v8a").split()
    backends = args.backend or os.environ.get("BACKENDS", "native").split()
    devshell = not args.without_devshell and os.environ.get("INCLUDE_DEVSHELL", "1") != "0"
    zkite = args.zkite or args.repo / "phase_1/cli/target/release" / ("zkite.exe" if os.name == "nt" else "zkite")
    try:
        entries = []
        for backend in backends:
            entries.extend(manifest(zkite.resolve(), backend, abis, devshell))
        stage(args.repo, args.dest, entries, abis)
    except (ValueError, OSError, subprocess.CalledProcessError) as exc:
        parser.exit(1, f"Cannot stage payload: {exc}\n")


if __name__ == "__main__":
    main()
