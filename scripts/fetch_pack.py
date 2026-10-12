#!/usr/bin/env python3
"""Fetch the exact google/pack revision used by ZKite's locked Cargo dependencies."""
from pathlib import Path
import subprocess

REPO = Path(__file__).resolve().parents[1]
REVISION = "7b60931e4058d892344ac57e49fcb582fd349b76"


def main() -> None:
    destination = REPO / "phase_0/vendor/pack"
    if destination.exists():
        current = subprocess.check_output(
            ["git", "-C", str(destination), "rev-parse", "HEAD"], text=True).strip()
        if current != REVISION:
            raise SystemExit(f"{destination} is at {current}; expected {REVISION}. "
                             "Move that checkout aside before fetching. It was not modified.")
    else:
        destination.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git", "init", str(destination)], check=True)
        subprocess.run(["git", "-C", str(destination), "remote", "add", "origin",
                        "https://github.com/google/pack.git"], check=True)
        subprocess.run(["git", "-C", str(destination), "fetch", "--depth", "1", "origin", REVISION], check=True)
        subprocess.run(["git", "-C", str(destination), "checkout", "--detach", "FETCH_HEAD"], check=True)
    print(f"google/pack: {REVISION}")


if __name__ == "__main__":
    main()
