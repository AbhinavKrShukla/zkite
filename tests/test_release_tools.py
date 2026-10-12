"""Regression checks for the wheels and payload users actually install."""
from __future__ import annotations

import argparse
import csv
import importlib.util
import io
from pathlib import Path
import stat
import tempfile
import unittest
import zipfile

REPO = Path(__file__).resolve().parents[1]


def load(name, relative):
    spec = importlib.util.spec_from_file_location(name, REPO / relative)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


wheels = load("build_wheels", "phase_1/cli/scripts/build_wheels.py")
staging = load("stage_payload", "phase_1/cli/scripts/stage_payload.py")
zips = load("zip_tree", "scripts/zip_tree.py")


class ReleaseWheelTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.payload = self.root / "payload"
        core = self.payload / "phase_1/zkite_py/src/zkite"
        core.mkdir(parents=True)
        (core / "__init__.py").write_text("# shipped framework\n")
        (core / "__pycache__").mkdir()
        (core / "__pycache__/old.pyc").write_bytes(b"not an input")
        self.binary = self.root / "zkite.exe"
        self.binary.write_bytes(b"MZ-test-binary")
        self.args = argparse.Namespace(
            payload=str(self.payload), out=str(self.root / "wheels"),
            binary=str(self.binary), version="0.1.0", payload_name="zkite-payload",
            cli_name="zkite", framework_name="zkite-framework", platform="win_amd64",
        )

    def test_windows_binary_installs_as_exe_and_depends_on_framework(self):
        path = wheels.build_cli_wheel(self.args)
        self.assertTrue(path.endswith("py3-none-win_amd64.whl"))
        with zipfile.ZipFile(path) as archive:
            executable = archive.getinfo("zkite-0.1.0.data/scripts/zkite.exe")
            self.assertTrue(stat.S_ISREG(executable.external_attr >> 16))
            self.assertTrue(executable.external_attr >> 16 & stat.S_IXUSR)
            metadata = archive.read("zkite-0.1.0.dist-info/METADATA").decode()
            self.assertIn("Requires-Dist: zkite-framework==0.1.0", metadata)
            self.assertIn("Requires-Dist: zkite-payload==0.1.0", metadata)

    def test_unix_binary_remains_executable(self):
        self.args.platform = "manylinux_2_28_x86_64"
        with zipfile.ZipFile(wheels.build_cli_wheel(self.args)) as archive:
            self.assertIn("zkite-0.1.0.data/scripts/zkite", archive.namelist())

    def test_framework_is_importable_layout_and_excludes_caches(self):
        with zipfile.ZipFile(wheels.build_framework_wheel(self.args)) as archive:
            self.assertEqual(archive.read("zkite/__init__.py"), b"# shipped framework\n")
            self.assertFalse(any("__pycache__" in n for n in archive.namelist()))

    def test_payload_keeps_prefix_relative_layout_and_record_hashes(self):
        with zipfile.ZipFile(wheels.build_payload_wheel(self.args)) as archive:
            self.assertIn("zkite_payload-0.1.0.data/data/zkite-payload/phase_1/zkite_py/src/zkite/__init__.py",
                          archive.namelist())
            record = archive.read("zkite_payload-0.1.0.dist-info/RECORD").decode()
            for path, digest, size in csv.reader(io.StringIO(record)):
                if path.endswith("/RECORD"):
                    self.assertEqual((digest, size), ("", ""))
                else:
                    data = archive.read(path)
                    self.assertEqual(digest, wheels.wheel_hash(data))
                    self.assertEqual(int(size), len(data))

    def test_wheel_is_reproducible(self):
        path = wheels.build_framework_wheel(self.args)
        first = Path(path).read_bytes()
        wheels.build_framework_wheel(self.args)
        self.assertEqual(first, Path(path).read_bytes())


class StagePayloadTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()

    def test_rejects_existing_destination_without_removing_it(self):
        destination = self.root / "payload"
        destination.mkdir()
        marker = destination / "keep.txt"
        marker.write_text("existing data")
        with self.assertRaisesRegex(ValueError, "already exists"):
            staging.stage(self.repo, destination, [], [])
        self.assertEqual(marker.read_text(), "existing data")

    def test_failure_does_not_leave_a_partial_payload(self):
        destination = self.root / "payload"
        with self.assertRaisesRegex(ValueError, "missing"):
            staging.stage(self.repo, destination, [("file", Path("missing.so"), "")], [])
        self.assertFalse(destination.exists())

    def test_overlapping_directories_merge_without_nesting(self):
        source = self.repo / "lib/python3.14/lib-dynload"
        source.mkdir(parents=True)
        (source / "sample.so").write_bytes(b"ELF")
        destination = self.root / "payload"
        staging.stage(self.repo, destination, [
            ("dir", Path("lib/python3.14"), ""),
            ("dir", Path("lib/python3.14/lib-dynload"), ""),
        ], ["x86_64"])
        self.assertEqual((destination / "lib/python3.14/lib-dynload/sample.so").read_bytes(), b"ELF")
        self.assertFalse((destination / "lib/python3.14/lib-dynload/lib-dynload").exists())

    def test_no_payload_inside_its_own_input(self):
        (self.repo / "src").mkdir()
        with self.assertRaisesRegex(ValueError, "inside a staged"):
            staging.stage(self.repo, self.repo / "src/payload", [("dir", Path("src"), "")], [])

    def test_manifest_paths_cannot_escape_on_either_platform(self):
        for value in ("../outside", "/absolute", "C:/Windows", "foo\\bar", ""):
            with self.subTest(value=value), self.assertRaises(ValueError):
                staging.relative_path(value)


class AssetZipTests(unittest.TestCase):
    def test_rebuild_drops_removed_files_and_ignores_host_caches(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            (source / "keep.py").write_text("pass")
            (source / "old.py").write_text("removed")
            (source / "__pycache__").mkdir()
            (source / "__pycache__/old.pyc").write_bytes(b"cache")
            archive = root / "asset.zip"
            zips.zip_tree(source, archive)
            (source / "old.py").unlink()
            zips.zip_tree(source, archive)
            with zipfile.ZipFile(archive) as result:
                self.assertEqual(result.namelist(), ["keep.py"])
            first = archive.read_bytes()
            zips.zip_tree(source, archive)
            self.assertEqual(first, archive.read_bytes())


if __name__ == "__main__":
    unittest.main()
