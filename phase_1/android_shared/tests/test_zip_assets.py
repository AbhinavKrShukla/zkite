"""Host tests for the Java asset installer used by all three Android shells.

Run: python -m unittest discover -s phase_1/android_shared/tests
Requires java and javac (Java 8 or later); no Android SDK or device is needed.
"""

import io
import shutil
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path


HERE = Path(__file__).resolve().parent
SOURCE = HERE.parent / "java/dev/zkite/runtime/ZipAssets.java"


def zip_bytes(files):
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, body in files.items():
            archive.writestr(name, body)
    return output.getvalue()


@unittest.skipUnless(shutil.which("javac") and shutil.which("java"), "Java JDK is required")
class ZipAssetsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.classes = tempfile.TemporaryDirectory()
        result = subprocess.run(
            ["javac", "-source", "8", "-target", "8", "-d", cls.classes.name,
             str(SOURCE), str(HERE / "ZipAssetsProbe.java")], capture_output=True, text=True)
        if result.returncode:
            cls.classes.cleanup()
            raise AssertionError(result.stderr)

    @classmethod
    def tearDownClass(cls):
        cls.classes.cleanup()

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.apk = self.root / "installed.apk"
        self.dest = self.root / "files/app"
        self.dest.parent.mkdir()

    def package(self, files=None, payload=None):
        if payload is None:
            payload = zip_bytes(files)
        # Reuse exactly the same APK path and shell/version id for every update.
        self.apk.write_bytes(zip_bytes({"assets/app.zip": payload}))

    def install(self, succeeds=True):
        result = subprocess.run(
            ["java", "-cp", self.classes.name, "ZipAssetsProbe", str(self.apk),
             "app.zip", str(self.dest), "app-1"], capture_output=True, text=True)
        if succeeds:
            self.assertEqual(result.returncode, 0, result.stderr)
            return result.stdout.strip() == "true"
        self.assertNotEqual(result.returncode, 0, result.stdout)
        return result.stderr

    def test_same_version_apk_update_replaces_source_and_deletes_removed_modules(self):
        self.package({"main.py": "version one", "old.py": "removed later"})
        self.assertTrue(self.install())
        self.package({"main.py": "version two", "new.py": "added now"})
        self.assertTrue(self.install())
        self.assertEqual((self.dest / "main.py").read_text(), "version two")
        self.assertFalse((self.dest / "old.py").exists())
        self.assertEqual((self.dest / "new.py").read_text(), "added now")

    def test_unchanged_asset_skips_extraction_and_preserves_bytecode_cache(self):
        self.package({"main.py": "hello"})
        self.assertTrue(self.install())
        cache = self.dest / "__pycache__"
        cache.mkdir()
        (cache / "main.pyc").write_bytes(b"cached")
        self.assertFalse(self.install())
        self.assertEqual((cache / "main.pyc").read_bytes(), b"cached")

    def test_old_empty_stamp_migrates_and_sibling_app_data_is_preserved(self):
        self.dest.mkdir()
        (self.dest / "main.py").write_text("stale")
        (self.dest.parent / ".app-1.stamp").touch()
        data = self.dest.parent / "user.sqlite"
        data.write_bytes(b"user data")
        self.package({"main.py": "fresh"})
        self.assertTrue(self.install())
        self.assertEqual((self.dest / "main.py").read_text(), "fresh")
        self.assertEqual(data.read_bytes(), b"user data")

    def test_missing_directory_is_restored_even_if_an_old_stamp_remains(self):
        self.package({"main.py": "hello"})
        self.install()
        shutil.rmtree(self.dest)
        self.assertTrue(self.install())
        self.assertEqual((self.dest / "main.py").read_text(), "hello")

    def test_interrupted_rename_recovers_the_previous_complete_directory(self):
        self.package({"main.py": "version one"})
        self.install()
        self.dest.rename(self.dest.parent / ".app.zkite-old")
        next_dir = self.dest.parent / ".app.zkite-next"
        next_dir.mkdir()
        (next_dir / "main.py").write_text("incomplete")
        self.assertFalse(self.install())
        self.assertEqual((self.dest / "main.py").read_text(), "version one")
        self.assertFalse(next_dir.exists())
        self.package({"main.py": "version two"})
        self.assertTrue(self.install())

    def test_interrupted_cleanup_keeps_the_new_complete_directory(self):
        self.package({"main.py": "version two"})
        self.install()
        old = self.dest.parent / ".app.zkite-old"
        old.mkdir()
        (old / "main.py").write_text("version one")
        self.assertFalse(self.install())
        self.assertEqual((self.dest / "main.py").read_text(), "version two")
        self.assertFalse(old.exists())

    def test_corrupt_nested_zip_does_not_replace_the_previous_app(self):
        self.package({"main.py": "working"})
        self.install()
        broken = zip_bytes({"main.py": "broken"})[:-20]
        self.package(payload=broken)
        self.install(succeeds=False)
        self.assertEqual((self.dest / "main.py").read_text(), "working")
        self.assertFalse((self.dest.parent / ".app.zkite-next").exists())

    def test_zip_traversal_does_not_touch_files_outside_managed_directory(self):
        self.package({"main.py": "working"})
        self.install()
        user_data = self.dest.parent / "user.sqlite"
        user_data.write_text("keep")
        for name in ("../user.sqlite", "dir/../../user.sqlite", "..\\user.sqlite", ".zkite-asset-id"):
            with self.subTest(name=name):
                self.package({name: "overwrite"})
                self.install(succeeds=False)
                self.assertEqual((self.dest / "main.py").read_text(), "working")
                self.assertEqual(user_data.read_text(), "keep")

    def test_a_missing_asset_does_not_delete_the_previous_app(self):
        self.package({"main.py": "working"})
        self.install()
        self.apk.write_bytes(zip_bytes({"assets/something-else": "x"}))
        self.install(succeeds=False)
        self.assertEqual((self.dest / "main.py").read_text(), "working")

    def test_cleanup_never_follows_a_symlink_into_user_data(self):
        user_data = self.dest.parent / "user_data"
        user_data.mkdir()
        (user_data / "keep.txt").write_text("keep")
        self.package({"main.py": "working"})
        self.install()
        old = self.dest.parent / ".app.zkite-old"
        old.mkdir()
        try:
            (old / "link").symlink_to(user_data, target_is_directory=True)
        except (OSError, NotImplementedError):
            self.skipTest("host does not permit symlinks")
        self.assertFalse(self.install())
        self.assertEqual((user_data / "keep.txt").read_text(), "keep")
        self.assertFalse(old.exists())
        shutil.rmtree(self.dest)
        self.dest.symlink_to(user_data, target_is_directory=True)
        self.install(succeeds=False)
        self.assertEqual((user_data / "keep.txt").read_text(), "keep")


if __name__ == "__main__":
    unittest.main()
