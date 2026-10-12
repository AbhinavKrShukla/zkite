# `zkite` CLI — working commands

> For current installation, Windows instructions, `zkite run`, and release builds,
> start with [Documentation.md](../Documentation.md). Historical command details follow.

`zkite run [--serial SERIAL] [--logs]` builds, installs, and launches a standalone
app with the native backend by default. It does not require `zkite dev`.

Both `zkite run` and `zkite dev` show an arrow-key device picker when multiple
authorized devices are connected: Up/Down moves, Enter selects, Esc/Ctrl+C cancels.
The rows show model names and serials. `--serial SERIAL` or `ANDROID_SERIAL`
bypasses the picker; specify one for scripts without an interactive terminal.

One binary (`phase_1/cli`, built by `cargo build --manifest-path phase_1/cli/Cargo.toml`,
binary at `phase_1/cli/target/{debug,release}/zkite`), evolving across phases — see
`phase_1/PLAN.md` and `phase_2/PLAN.md` for the design narrative behind each command.
This file is the reference for what's actually implemented and verified today, kept
separate from those design docs so it stays a quick lookup, not a read-through.

Build it once before using anything below:

```bash
source ~/.cargo/env   # cargo/rustc on PATH
export PROTOC=$(pwd)/phase_2/vendor/protoc/bin/protoc   # only needed to build zkite itself — see phase_2/PLAN.md §1.5
cd phase_1/cli && cargo build --release
# binary: phase_1/cli/target/release/zkite
```

A matching-version Python 3.14 also needs to be on `PATH` for `zkite build` (not for
`zkite new`/`zkite dev`) — see "Prerequisites" below.

---

## `zkite new <name> [.]`

Scaffolds a project: `main.py`, `app.json`, `assets/`, `zkite.tailwind.json`, and
`.gitignore`. No package-manager file is required. `zkite new myapp .` scaffolds
the current directory and refuses to overwrite existing scaffold paths. The ignore rules cover local runtime state, build output, signing
keys, Python environments and caches, local environment files, and OS/editor
scratch files.

```bash
zkite new myapp
cd myapp
```

**Status: tested.** Scaffold tests (`phase_1/cli/src/new_cmd.rs`) cover the
generated files and assets directory, project name substitution, valid Android identity, starter view,
invalid names, existing directories, and names with hyphens/underscores.

---

## `zkite dev [--devshell-apk <path>] [--port <port>] [--adb <path>]`

Hot-reloading dev loop: repackages the shipped shell with your icon and label
(` (Dev)` suffix), installs a separate project preview, pushes project files over
`adb reverse`, and re-renders on every save. Rerun after changing launcher settings.

```bash
zkite dev --devshell-apk phase_1/devshell/artifacts/zkite-devshell-x86_64.apk
# normally, use the prebuilt shell included with pip:
zkite dev
```

Flags: `--port` (default `8765`), `--adb` (default `adb` on PATH).

**Status: verified live** on a running emulator (phase_1/PLAN.md §7, M5) — `zkite new`
→ `zkite dev` → hot-reloading counter app, tap dispatch, live edits reaching the
device with state surviving reload. 14 `cargo test` cases cover the host-testable
half (hash-manifest diffing, debounced file watching); the adb/device half was run
live, not unit-tested (inherently an integration of JNI + a real WebView + a real
socket).

Pip installations already include the native Dev Shell. Contributors rebuilding
the optional WebView Dev Shell from source can use:

```bash
phase_1/devshell/scripts/build.sh aarch64 x86_64   # cross-compile the .so per ABI
ANDROID_API=24 ABI=x86_64 phase_1/devshell/scripts/build_apk.sh   # pack the APK
```

---

## `zkite build apk [--abi <abi>]... [--keystore <path>] [--out <dir>] [--no-prune-stdlib]`

Produces a signed, standalone APK — one file per requested ABI, no `zkite dev`
dependency at all.

```bash
cd myapp
zkite build apk                                    # both arm64-v8a and x86_64
zkite build apk --abi x86_64                        # one ABI only
zkite build apk --keystore ../zkite-release.pem       # real signing key
zkite build apk --out /tmp/out                       # default: <project>/dist/
```

**Status: verified live**, on both a real phone and an emulator — `phase_2/PLAN.md`
§6 (M6/M7). `zkite new` → `zkite build apk` → `adb install` → `adb shell am start`,
no `zkite dev` process running anywhere, tap-interactive (counter went 0→2 on both
devices via `adb shell input tap`), screenshots confirmed. Output also verified with
real `apksigner verify` (v2 **and** v3), `zipalign -c`, `aapt2 dump packagename`.

Needs the **App Shell** artifacts built first (release-pipeline step, done once —
`zkite build` itself never touches the NDK/JDK):

```bash
phase_2/appshell/scripts/build.sh aarch64 x86_64   # cross-compile the .so per ABI
phase_2/appshell/scripts/build_dex.sh               # classes.dex
```

Without `--keystore`, signs with an ephemeral testing key (printed warning) — fine
for local installs, never for anything you distribute; see `zkite keygen` below.

---

## `zkite build aab [--abi <abi>]... [--keystore <path>] [--out <dir>] [--no-prune-stdlib]`

Same pipeline as `apk`, but bundles every requested ABI into one Android App Bundle
instead of one APK per ABI. Same flags, same App Shell artifact prerequisite.

```bash
cd myapp
zkite build aab --keystore ../zkite-release.pem
```

**Status: verified live** — `phase_2/PLAN.md` §6 (M8). The output passes
`bundletool validate` (literally Google Play's own bundle validator) and
`bundletool build-apks --mode=universal` successfully re-derives a real, installable
APK from it — which was installed and launched on the emulator and confirmed
tap-interactive, same bar as the `apk` command. This is the closest verification
reachable without an actual Play Console upload.

Known gap: a two-ABI AAB build currently takes ~15s, over the informal "<10s" target
(single-ABI/single-target builds are 6–8s) — root-caused in `phase_2/PLAN.md` §7 to
`pack` deflating already-compressed native `.so` files rather than storing them
uncompressed; not yet fixed.

---

## `zkite keygen [--out <path>] [--cn <name>] [--password <pw>]`

Generates a real signing keystore: a combined PEM (what `--keystore` reads) and a
password-protected PKCS#12 (for `keytool`/`jarsigner`/`bundletool`/Play Console
upload-key workflows).

```bash
zkite keygen --out zkite-release --cn myapp --password 'a-real-password'
# writes zkite-release.pem and zkite-release.p12
```

The PKCS#12 alias defaults to `--cn`'s value — needed if you drive `bundletool`/
`keytool` against the `.p12` directly (`--ks-key-alias <cn-value>`).

**Status: tested**, including real cross-tool interop, not just self-consistency:
`zkite-bundler`'s test suite (`phase_2/bundler/tests/keystore.rs`) checks the PEM
round-trips through the same parser `zkite build` uses, and the `.p12` is readable by
both real `keytool -list` and real `openssl pkcs12` (the latter needs `-legacy` —
documented finding, not a bug: the PKCS#12 uses the traditional RC2/3DES algorithms
every JDK reads natively, which OpenSSL 3 moved behind an opt-in provider). Also
used for real in `phase_2/PLAN.md`'s AAB verification: a `zkite keygen`-produced
keystore successfully signed a bundle that `bundletool build-apks` then re-signed
into an installable APK.

**Keep the output private.** Losing it means you can never publish an update under
the same app identity again.

---

## `zkite doctor [--adb <path>]`

Environment + dependency diagnostic. Run from anywhere for the environment checks;
run from inside a project directory to also check its declared dependencies.

```bash
zkite doctor
```

Checks, in order: **which build payload it resolved** (and how — see below), `adb` on
PATH with a device/emulator connected, a Python 3.14 interpreter
for `.pyc` compilation, pip (vendored `pip.pyz` or system) for dependency
resolution, and prebuilt shell artifacts for **both** backends. If `app.json` or `pyproject.toml` is found in the
current directory, dry-resolves every declared dependency and reports each as
`ok` (real Android wheel, or pure-Python — safe), `WARN` (resolved, but to a
suspiciously old version — pip likely fell back to a stale platform-compatible
release rather than real Android support), or `FAIL` (nothing resolves at all).

**The payload check, and why it is first** (`phase_3/PLAN.md` §29). Everything `zkite
build` reads that is not your project — both shells' prebuilt artifacts, the vendored
Python prefixes, `zkite_py`, the pybuild scripts, `pip.pyz` — lives under one *payload
root*, resolved in this order:

1. `ZKITE_PAYLOAD_ROOT`, if set (a value that is not a payload is a hard error, not a
   silent fallback);
2. a `zkite-payload/` directory beside the `zkite` binary, or one level above it;
3. the checkout `zkite` was compiled in — **last**, and reported as "dev mode".

`zkite doctor` prints which one answered. That matters because the failure this design
introduces is invisible otherwise: a packaged install quietly reading a developer's
checkout works perfectly until the checkout moves.

Only the **default** backend's artifacts (native) are fatal. A missing WebView shell
prints `--` and says `--backend webview` will not work here — a native-only payload is a
working install, not a broken one.

**Status: verified live** — `phase_2/PLAN.md` §6 (M9). Run against a real project
declaring `["zkite", "attrs", "pydantic-core"]`: `attrs` correctly reports `ok`
(pure-Python), `pydantic-core` correctly reports the exact stale-fallback `WARN`
(resolved `0.0.1`, latest on PyPI is `2.47.0` — confirmed live against real PyPI,
2026-08-04: **zero** packages checked have a real `android_*`-tagged wheel yet).

---

## Prerequisites

- **`adb`** — the one external tool `zkite` assumes, for `zkite dev`/`zkite doctor`
  (device checks) and to manually install what `zkite build` produces.
- **A Python 3.14 interpreter on PATH** (as `python3.14`), for `zkite build`'s `.pyc`
  compilation step — bytecode is version-specific, and this dev environment's system
  Python is 3.12. Easiest fix: `uv python install 3.14` (a few seconds, no system
  package manager involved); `zkite build`/`zkite doctor` also fall back to
  `uv python find 3.14` automatically if `uv` is on PATH. See `phase_2/PLAN.md` §1.4
  for why this is needed at all and what the real fix looks like (vendoring
  host-native compile-only Python builds into the `zkite` wheel itself — not done in
  this checkout yet, Linux-x86_64-only environment).
- **Release-pipeline artifacts built once**, before `zkite build`/`zkite dev` can do
  anything (see each command's section above for the exact script). These are never
  rebuilt by `zkite` itself — that's the entire point of `phase_2/PLAN.md` §1.1: `zkite
  build` only ever reads prebuilt `.so`/`classes.dex`, never invokes the NDK/JDK.
- **Not required for anything above**: Android SDK, NDK, Gradle, or `protoc` on the
  *end-user's* machine — those only matter for building `zkite` itself and its
  release-pipeline artifacts (see `phase_2/PLAN.md` §1.1/§1.5), which is what the
  "Build it once" section at the top of this file is doing.
