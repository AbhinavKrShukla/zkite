#!/usr/bin/env python3
"""Record what source produced a prebuilt shell artifact — phase_3/PLAN.md §34.8.

**The problem this exists for.** `zkite build apk` compiles nothing. It copies
`libzkitegpuharness.so`, `libpython3.14.so`, `libc++_shared.so`, `classes.dex` and
`fonts.zip` verbatim out of `<shell>/artifacts/`, and until this script nothing
anywhere recorded which build wrote them. Twice now that has shipped the wrong
binary:

* **§34.8** — a negative-control build with the frame sink disabled was the last
  thing to write `phase_3/gpu_harness/artifacts/`, so the "real app" under test had
  no push path. The symptom looked exactly like a network hang. `strings` could not
  tell the two shells apart, because the control patched a code path that leaves
  every symbol in place; the only reliable tell was the `.so`'s mtime against the
  build log.
* **§35** — the same trap again, avoided only by byte-comparing `classes.dex`
  against a freshly built one by hand.

So the artifact is not the thing to check. **The claim to check is "this artifact
was produced by the source that is on disk right now"**, and that is what this
writes down: a digest over exactly the source inputs that feed the build, stored
beside the artifacts, for `zkite build` to re-derive and compare (`payload.rs`'s
sibling, `provenance.rs`).

**The input set is derived, not listed.** For the Rust half it comes from
`cargo metadata` — every package in the dependency graph whose manifest lives
inside this repository. That is how `phase_0/spike3_twcore` ends up in the native
shell's input set, which a hand-written list would have missed: it reaches
`gpu_harness` two crates deep, through `zkite_renderer`. Crates.io dependencies are
outside the repo and are *not* walked; `Cargo.lock` is in the input set instead,
which is what makes a version bump visible without hashing the registry.

**Adding a path dependency cannot slip past this**, which is the property that
makes a derived-then-frozen list safe: a new path dep means an edited `Cargo.toml`
and `Cargo.lock`, both of which are themselves hashed.

Usage (see `phase_3/gpu_harness/scripts/build.sh` for the real calls):

    artifact_provenance.py record --artifacts <dir> --shell native --group arm64-v8a \\
        --cargo-manifest phase_3/gpu_harness/Cargo.toml \\
        --extra phase_3/gpu_harness/vendor \\
        --rebuild 'phase_3/gpu_harness/scripts/build.sh arm64-v8a' \\
        --file arm64-v8a/libzkitegpuharness.so --file arm64-v8a/libc++_shared.so

    artifact_provenance.py digest --input phase_3/renderer/src   # debugging / tests
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import re
import subprocess
import sys

# ---------------------------------------------------------------------------
# THE CANONICAL STREAM. Implemented twice on purpose — here, and in Rust in
# `phase_1/cli/src/provenance.rs::source_digest`. §31.6 already had to keep two
# implementations of one digest agreeing (`runtime.py` and `dev_cmd.rs`) and settled
# it by writing the stream definition into both docstrings; this does that and then
# pins it with a test that runs *this file* and compares (`provenance::tests::
# the_rust_and_python_digests_agree_over_a_real_tree`), so drift fails a test rather
# than a device.
#
#     collect every regular file under every input path, keyed by its repo-relative
#     path, dropping the excluded components below;
#     sort by that path's raw bytes;
#     for each:  path · NUL · len(bytes) as decimal ASCII · NUL · bytes · NUL
#     sha256 over the whole stream, lowercase hex, full 64 characters.
#
# The length field is not redundant with the NUL terminators: file contents can
# contain NULs (every one of these inputs can — `.jar`, `.ttf`, `.png`), so without
# it the stream is ambiguous and two different trees can collide by construction.
# ---------------------------------------------------------------------------

# Matched against whole path *components*, so `build/` (cargo + javac scratch) is
# excluded while `build.rs` — a real input — is not.
EXCLUDED_COMPONENTS = frozenset(
    {
        "target",  # cargo output
        "build",  # javac/d8 scratch, and gpu_harness's own build/
        "artifacts",  # the outputs this file describes
        "__pycache__",
        ".git",
        "node_modules",
        ".mypy_cache",
        ".pytest_cache",
    }
)
EXCLUDED_SUFFIXES = (".pyc", ".pyo")

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
SCHEMA = 1


def _excluded(rel: str) -> bool:
    parts = rel.split("/")
    return any(p in EXCLUDED_COMPONENTS for p in parts) or rel.endswith(EXCLUDED_SUFFIXES)


def collect(root: str, inputs: list[str]) -> dict[str, str]:
    """Repo-relative path -> absolute path, for every file the digest covers.

    A dict keyed by the relative path, so overlapping inputs (`gpu_harness/src` and
    `gpu_harness` both being listed, say) contribute each file exactly once and the
    digest cannot depend on the order the inputs were passed in.

    Symlinks are skipped rather than followed. Nothing in any current input set is
    one; following them would make the digest depend on what is outside the repo,
    and silently walking out of the tree is the failure `zkite_imagestore`'s lexical
    path check already has filed against it (§35.6).
    """
    found: dict[str, str] = {}
    for rel_input in inputs:
        abs_input = os.path.join(root, rel_input)
        if os.path.islink(abs_input):
            continue
        if os.path.isfile(abs_input):
            if not _excluded(rel_input):
                found[rel_input] = abs_input
            continue
        if not os.path.isdir(abs_input):
            # A declared input that is not there at all. The caller decides what that
            # means — for `record` it is a bug in the build script, for the Rust
            # reader it is the ordinary shape of a packaged payload, which carries
            # `artifacts/` and no sources.
            continue
        for dirpath, dirnames, filenames in os.walk(abs_input):
            dirnames[:] = sorted(d for d in dirnames if d not in EXCLUDED_COMPONENTS)
            for name in sorted(filenames):
                abs_path = os.path.join(dirpath, name)
                if os.path.islink(abs_path) or not os.path.isfile(abs_path):
                    continue
                rel = os.path.relpath(abs_path, root).replace(os.sep, "/")
                if not _excluded(rel):
                    found[rel] = abs_path
    return found


def source_digest(root: str, inputs: list[str]) -> tuple[str, int]:
    """The canonical stream's sha256, and how many files went into it."""
    files = collect(root, inputs)
    h = hashlib.sha256()
    for rel in sorted(files, key=lambda s: s.encode()):
        with open(files[rel], "rb") as fh:
            data = fh.read()
        h.update(rel.encode())
        h.update(b"\0")
        h.update(str(len(data)).encode())
        h.update(b"\0")
        h.update(data)
        h.update(b"\0")
    return h.hexdigest(), len(files)


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def cargo_source_inputs(manifest_rel: str) -> list[str]:
    """Every in-repo package in this crate's dependency graph, as repo-relative dirs.

    `cargo metadata` resolves the whole graph, so a path dependency reached two
    crates deep is included — `phase_0/spike3_twcore` arrives via `zkite_renderer`
    and no hand-written list would have had it. Packages outside the repository
    (crates.io) are deliberately not walked; `Cargo.lock` covers them.
    """
    manifest = os.path.join(REPO, manifest_rel)
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--manifest-path", manifest],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    meta = json.loads(out)
    inputs: set[str] = set()
    repo_prefix = REPO.rstrip("/") + "/"
    for pkg in meta["packages"]:
        pkg_manifest = pkg["manifest_path"]
        if not pkg_manifest.startswith(repo_prefix):
            continue
        pkg_dir = os.path.dirname(pkg_manifest)
        rel_dir = os.path.relpath(pkg_dir, REPO).replace(os.sep, "/")
        for candidate in ("src", "build.rs", "Cargo.toml", "Cargo.lock"):
            if os.path.exists(os.path.join(pkg_dir, candidate)):
                inputs.add(f"{rel_dir}/{candidate}")
    return sorted(inputs)


_INCLUDE_RE = re.compile(r"include_(?:str|bytes)!\s*\(\s*\"([^\"]+)\"\s*[,)]")
_PATH_RE = re.compile(r'#\s*\[\s*path\s*=\s*"([^"]+)"\s*\]')


def include_macro_inputs(inputs: list[str]) -> list[str]:
    """Files pulled into a crate by include macros and #[path] modules.

    `cargo metadata` describes the *package* graph, so it finds every crate's `src`
    and nothing else. A file compiled into a binary by `include_str!` from outside
    any crate directory is therefore invisible to it — and `phase_1/devshell/src/
    runtime.py` is exactly that: it is `include_str!`'d by `phase_3/pybridge`, it is
    most of the framework's Python glue, and no crate in the native shell's graph
    contains it.

    The consequence, measured rather than reasoned about (phase_3/PLAN.md §43):
    editing `runtime.py` and rebuilding produced a **different `.so`** — cargo's own
    dependency tracking picks the macro up correctly — under an **identical source
    digest**. So `zkite doctor` reported "verified" for a shell built from Python that
    was no longer in the tree, which is §37.1's incident in the one place §37's check
    could not see. Both directions are broken by it: a stale shell passes, and a
    fresh one is indistinguishable from it.

    Derived by scanning rather than hand-listed, on this file's own argument for why
    the crate set is derived — a hand-written entry is a thing to forget precisely
    when it matters. Paths resolving outside the repo are ignored (nothing includes
    one today; the digest must not depend on anything outside the tree).

    Follow referenced Rust files recursively, because a shared #[path] module can
    have its own include macros. A visited set handles duplicate references/cycles.
    """
    found: set[str] = set()
    pending = list(sorted(collect(REPO, inputs).items()))
    visited: set[str] = set()
    while pending:
        rel, abs_path = pending.pop()
        if abs_path in visited:
            continue
        visited.add(abs_path)
        if not rel.endswith(".rs"):
            continue
        with open(abs_path, "r", errors="replace") as fh:
            src = fh.read()
        for m in (*_INCLUDE_RE.finditer(src), *_PATH_RE.finditer(src)):
            target = os.path.normpath(os.path.join(os.path.dirname(abs_path), m.group(1)))
            if not os.path.isfile(target):
                continue
            rel_target = os.path.relpath(target, REPO).replace(os.sep, "/")
            if rel_target.startswith(".."):
                continue
            found.add(rel_target)
            if target.endswith(".rs"):
                pending.append((rel_target, target))
    return sorted(found)


def cmd_record(args: argparse.Namespace) -> int:
    inputs: list[str] = list(args.extra)
    if args.cargo_manifest:
        inputs += cargo_source_inputs(args.cargo_manifest)
    # After the crate set, because it is derived *from* the collected Rust sources.
    inputs += include_macro_inputs(inputs)
    inputs = sorted(set(inputs))

    missing = [i for i in inputs if not os.path.exists(os.path.join(REPO, i))]
    if missing:
        # Loud, because a silently-dropped input is a digest that agrees with a
        # source tree it never covered — worse than no digest at all.
        print(f"provenance: declared input(s) not in this checkout: {missing}", file=sys.stderr)
        return 1

    digest, n_files = source_digest(REPO, inputs)

    artifacts = os.path.abspath(args.artifacts)
    files: dict[str, str] = {}
    for rel in args.file:
        path = os.path.join(artifacts, rel)
        if not os.path.isfile(path):
            print(f"provenance: artifact not found: {path}", file=sys.stderr)
            return 1
        files[rel] = sha256_file(path)

    record_path = os.path.join(artifacts, "provenance.json")
    try:
        with open(record_path) as fh:
            record = json.load(fh)
    except (OSError, ValueError):
        record = {}
    if record.get("schema") != SCHEMA or record.get("shell") != args.shell:
        # A schema bump or a directory that somehow described a different shell: start
        # clean rather than merge groups whose meaning has changed underneath them.
        record = {"schema": SCHEMA, "shell": args.shell, "groups": {}}
    record.setdefault("groups", {})

    record["groups"][args.group] = {
        "built_at": datetime.datetime.now(datetime.timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z"),
        "built_by": args.built_by,
        "rebuild": args.rebuild,
        "source_hash": digest,
        "source_file_count": n_files,
        "source_inputs": inputs,
        # False means "the recorded inputs do not fully determine this artifact", and
        # the Rust reader must not claim a match verifies it. `fonts.zip` is the case:
        # stage_fonts.sh copies faces out of the *host's* /usr/share/fonts, which no
        # digest over this repository can see.
        "source_complete": not args.source_incomplete,
        "git_head": _git("rev-parse", "HEAD"),
        "git_dirty": bool(_git("status", "--porcelain")),
        "files": files,
    }
    os.makedirs(artifacts, exist_ok=True)
    with open(record_path, "w") as fh:
        json.dump(record, fh, indent=2, sort_keys=True)
        fh.write("\n")
    print(f"    provenance: {args.shell}/{args.group} source {digest[:12]} ({n_files} files)")
    return 0


def _git(*cmd: str) -> str | None:
    try:
        out = subprocess.run(
            ["git", "-C", REPO, *cmd], check=True, capture_output=True, text=True
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None
    return out or None


def cmd_digest(args: argparse.Namespace) -> int:
    root = os.path.abspath(args.root) if args.root else REPO
    digest, n = source_digest(root, args.input)
    print(f"{digest} {n}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)

    rec = sub.add_parser("record", help="write/update artifacts/provenance.json")
    rec.add_argument("--artifacts", required=True, help="the shell's artifacts/ directory")
    rec.add_argument("--shell", required=True, choices=["native", "webview"])
    rec.add_argument("--group", required=True, help="e.g. arm64-v8a, dex, fonts")
    rec.add_argument("--cargo-manifest", help="repo-relative Cargo.toml to derive inputs from")
    rec.add_argument("--extra", action="append", default=[], help="repo-relative extra input")
    rec.add_argument("--file", action="append", default=[], help="artifact, relative to --artifacts")
    rec.add_argument("--rebuild", required=True, help="the command that regenerates this group")
    rec.add_argument("--built-by", required=True, help="repo-relative script that wrote it")
    rec.add_argument(
        "--source-incomplete",
        action="store_true",
        help="the inputs do not fully determine the artifact (see source_complete)",
    )
    rec.set_defaults(func=cmd_record)

    dig = sub.add_parser("digest", help="print the canonical digest over some inputs")
    dig.add_argument("--root", help="tree to resolve inputs against (default: this repo)")
    dig.add_argument("--input", action="append", default=[], required=True)
    dig.set_defaults(func=cmd_digest)

    args = ap.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
