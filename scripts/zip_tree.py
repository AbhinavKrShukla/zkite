#!/usr/bin/env python3
"""Write a fresh, deterministic ZIP without depending on a host `zip` command."""
from __future__ import annotations

import argparse
from pathlib import Path
import tempfile
import zipfile


def zip_tree(source: Path, destination: Path, suffix: str | None = None) -> None:
    source, destination = source.resolve(), destination.resolve()
    if not source.is_dir():
        raise ValueError(f"not a source directory: {source}")
    if destination.is_relative_to(source):
        raise ValueError("archive must be outside its input directory")
    destination.parent.mkdir(parents=True, exist_ok=True)
    # Always start fresh: updating an existing zip would retain deleted app files.
    with tempfile.TemporaryDirectory(dir=destination.parent) as scratch:
        temporary = Path(scratch) / "archive.zip"
        with zipfile.ZipFile(temporary, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
            for path in sorted(source.rglob("*")):
                relative = path.relative_to(source)
                if not path.is_file() or path.is_symlink() or not path.resolve().is_relative_to(source):
                    continue
                if "__pycache__" in relative.parts or path.suffix in {".pyc", ".pyo"}:
                    continue
                if suffix is not None and path.suffix != suffix:
                    continue
                entry = zipfile.ZipInfo(relative.as_posix(), date_time=(1980, 1, 1, 0, 0, 0))
                entry.create_system = 3
                entry.external_attr = 0o100644 << 16
                entry.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(entry, path.read_bytes(), compresslevel=9)
        temporary.replace(destination)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--suffix")
    args = parser.parse_args()
    zip_tree(args.source, args.destination, args.suffix)
