"""Generates zkite.tailwind.json so the standard Tailwind CSS IntelliSense VS Code
extension autocompletes class strings inside widget calls (plan1.md §4, "editor story").

The extension's `tailwindCSS.experimental.classRegex` setting matches a regex against
source text and extracts class-string candidates from the first capture group. Every
zkite widget takes the class string as its first positional argument, so one regex
covers all of them.
"""

from __future__ import annotations

import json
from pathlib import Path

WIDGET_NAMES = [
    "Column", "Row", "Grid", "Stack", "ScrollView",
    "Text", "Button", "Icon", "TextField", "Image", "Switch", "Slider",
]

CONFIG = {
    "$schema": "https://json.schemastore.org/tailwindcss.json",
    "tailwindCSS.experimental.classRegex": [
        [rf"(?:{'|'.join(WIDGET_NAMES)})\(\s*[\"']([^\"']*)[\"']", ""],
    ],
    "tailwindCSS.includeLanguages": {"python": "html"},
}


def generate(project_dir: str | Path) -> Path:
    project_dir = Path(project_dir)
    out_path = project_dir / "zkite.tailwind.json"
    out_path.write_text(json.dumps(CONFIG, indent=2) + "\n")
    return out_path


__all__ = ["generate", "WIDGET_NAMES"]
