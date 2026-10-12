import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "phase_1/cli/scripts/artifact_provenance.py"
SPEC = importlib.util.spec_from_file_location("artifact_provenance_for_test", SCRIPT)
provenance = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(provenance)


class ProvenanceInputTests(unittest.TestCase):
    def test_path_modules_and_their_nested_includes_are_tracked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "crate/src").mkdir(parents=True)
            (root / "shared").mkdir()
            (root / "crate/src/lib.rs").write_text('#[path = "../../shared/files.rs"]\nmod files;\n')
            (root / "shared/files.rs").write_text('const DATA: &str = include_str!("data.txt");\n')
            (root / "shared/data.txt").write_text("first")
            with patch.object(provenance, "REPO", directory):
                inputs = provenance.include_macro_inputs(["crate/src"])
            self.assertEqual(inputs, ["shared/data.txt", "shared/files.rs"])
            before = provenance.source_digest(directory, ["crate/src", *inputs])
            (root / "shared/data.txt").write_text("changed")
            self.assertNotEqual(before, provenance.source_digest(directory, ["crate/src", *inputs]))


if __name__ == "__main__":
    unittest.main()
