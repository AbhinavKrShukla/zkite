import json
import tempfile
import unittest
from pathlib import Path

from zkite.tailwind_config import generate


class TailwindConfigTests(unittest.TestCase):
    def test_generate_writes_valid_json_with_all_widget_names(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = generate(tmp)
            self.assertTrue(path.exists())
            data = json.loads(path.read_text())
            regex = data["tailwindCSS.experimental.classRegex"][0][0]
            for name in ["Column", "Row", "Text", "Button", "TextField"]:
                self.assertIn(name, regex)


if __name__ == "__main__":
    unittest.main()
