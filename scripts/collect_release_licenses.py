#!/usr/bin/env python3
"""Collect notices from Cargo metadata and supplemental release-component files.

Pass JSON from `cargo metadata --locked --format-version 1` for the CLI and
Android runtime. Build dependencies are included; dev-only dependencies are not.
Crates that omit license files use their recorded upstream Git commit on GitHub.
Supplemental files cover fonts, CPython, pip and native runtimes outside Cargo.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
from urllib.request import urlopen


def license_file(path):
    return path.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT")) or path.name.lower() == "unlicense"


def collect(package):
    root = Path(package["manifest_path"]).parent
    cache = Path("release/license-cache") / (package["name"] + "-" + package["version"] + ".txt")
    if cache.is_file():
        return cache.read_text()
    texts = [(str(p.relative_to(root)), p.read_text(errors="replace"))
             for p in sorted(root.rglob("*")) if p.is_file() and license_file(p)]
    if not texts and package["name"].startswith("pack-"):
        texts = [("pack/LICENSE", (root.parent / "LICENSE").read_text())]
    if not texts and package.get("license") == "CC0-1.0":
        url = "https://raw.githubusercontent.com/spdx/license-list-data/v3.27.0/text/CC0-1.0.txt"
        with urlopen(url, timeout=20) as response:
            texts = [(url, response.read().decode())]
    if not texts:
        vcs = root / ".cargo_vcs_info.json"
        info = json.loads(vcs.read_text()) if vcs.exists() else {}
        revision = info.get("git", {}).get("sha1")
        repo = (package.get("repository") or "").removesuffix(".git").rstrip("/")
        if repo.startswith("https://github.com/"):
            # xml 0.8.20 predates Cargo's embedded VCS information.
            revision = revision or {("xml", "0.8.20"): "0.8.20"}.get((package["name"], package["version"]))
            if revision:
                for name in ("LICENSE", "LICENSE-MIT", "LICENSE-APACHE", "LICENSE-CC0", "UNLICENSE", "LICENSE.txt", "LICENSE.md", "COPYING", "NOTICE", "LICENSE-MIT.md", "LICENSE-APACHE.md", "LICENSES/Apache-2.0.txt", "LICENSES/MIT.txt"):
                    url = repo.replace("https://github.com/", "https://raw.githubusercontent.com/") + f"/{revision}/{name}"
                    try:
                        with urlopen(url, timeout=20) as response:
                            texts.append((url, response.read().decode()))
                    except Exception:
                        continue
    if not texts:
        # Some upstream repositories omit a separate license file too. These
        # packages explicitly offer Apache-2.0; include its full standard text
        # and select that option rather than inventing MIT copyright notices.
        declaration = package.get("license") or ""
        if "Apache-2.0" in declaration and "AND" not in declaration and "WITH" not in declaration:
            url = "https://raw.githubusercontent.com/spdx/license-list-data/v3.27.0/text/Apache-2.0.txt"
            with urlopen(url, timeout=20) as response:
                texts = [("Apache-2.0 option selected; " + url, response.read().decode())]
        else:
            return None
    heading = f"{package['name']} {package['version']}\nLicense declaration: {package.get('license')}\nSource: {package.get('repository') or package.get('source')}\n"
    result = heading + "\n".join(f"\n--- {name} ---\n{text}" for name, text in texts)
    cache.parent.mkdir(parents=True, exist_ok=True)
    cache.write_text(result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("metadata", nargs="+", type=Path)
    parser.add_argument("--supplement", action="append", type=Path, default=[])
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    packages = {}
    for path in args.metadata:
        data = json.loads(path.read_text())
        nodes = {n["id"]: n for n in data["resolve"]["nodes"]}
        pending, used = [data["resolve"]["root"]], set()
        while pending:
            ident = pending.pop()
            if ident in used:
                continue
            used.add(ident)
            pending.extend(d["pkg"] for d in nodes[ident]["deps"] if any(k["kind"] != "dev" for k in d["dep_kinds"]))
        for package in data["packages"]:
            if package["id"] in used and (package.get("source") or package["name"].startswith("pack-")):
                packages[(package["name"], package["version"])] = package
    with ThreadPoolExecutor(max_workers=8) as pool:
        sections = list(pool.map(collect, [packages[key] for key in sorted(packages)]))
    missing = [key for key, section in zip(sorted(packages), sections) if section is None]
    if missing:
        raise RuntimeError(f"Missing upstream license texts: {missing}")
    for path in args.supplement:
        sections.append(f"{path.name}\n\n{path.read_text(errors='replace')}")
    intro = ("ZKite third-party notices\n\nZKite's original code is MIT licensed. Bundled components retain their own\n"
             "licenses below. This inventory includes build-time dependencies and optional\n"
             "platform components; listing does not imply every file is linked on every host.\n\n")
    args.out.write_text(intro + ("\n\n" + "=" * 78 + "\n\n").join(sections))
    print(f"Wrote {len(packages)} Cargo component notices plus {len(args.supplement)} supplements to {args.out}")


if __name__ == "__main__":
    main()
