# Phase 2 — The bundler

This is the implementation plan for **plan1.md §10, Phase 2**:

> `zkite build apk/aab` · wheel resolution for Android tags · pyc + zipimport · signing and
> keystores · stdlib pruning · `zkite doctor`.
> **Exit:** a signed AAB is accepted by the Play Console. Release build under 10s.

Phase 1 (`../phase_1/PLAN.md`) shipped the WebView backend and the dev loop: `zkite new`,
`zkite dev`, hot reload, 12 widgets. Everything in phase 1 is a *dev-time* artifact — the Dev
Shell APK is generic, installed once, and never contains a specific app's code; `zkite dev`
pushes source over a socket. Phase 2 is the other half: turning a `zkite` project directory into
a **standalone, signed, installable APK/AAB** that contains the app's own code, ships to a
device with no `zkite dev` running, and (in principle) uploads to Play.

Read this document's §1 before anything else — three findings there change the shape of every
milestone below, the same way phase 0's `PYTHONHOME` finding and phase 1's morphdom finding
each reshaped their own plans.

---

## 1. Findings that shape this plan

### 1.1 `zkite-pack` must become a library, not a subprocess

Phase 0 and phase 1 both invoke packaging via `cargo run --manifest-path
phase_0/spike2_pack_apk/Cargo.toml -- ...` from a shell script. That is correct for those
phases — `build_apk.sh` assembles the *Dev Shell*, which is a release-pipeline artifact **we**
build and vendor, same bucket as `classes.dex`.

`zkite build apk` is different: it runs on **the end user's machine**, as a subcommand of the
one `zkite` binary they got from `pip install zkite`. It cannot shell out to `cargo run` — that
would require a Rust toolchain on the user's machine, which is exactly the thing plan1.md §2
principle 1 rules out ("nothing compiles on the user's machine"). The packaging logic has to be
**compiled into the `zkite` binary itself**.

So phase 2's first job is promotion, not new invention: `spike2_pack_apk/src/apk.rs`'s
`extras`-splicing trick (the thing that lets `pack`, built for Wear OS watch faces with no
code, hold `classes.dex`/`lib/*.so`/`assets/`) becomes `phase_2/bundler`, a real library crate,
and `phase_1/cli` depends on it directly. `spike2_pack_apk` itself is untouched — it stays a
standalone spike binary, phase 0's artifact, not phase 2's.

### 1.2 The release "App Shell" is the Dev Shell minus one thing

Re-reading `devshell/src/runtime.py::_zkite_setup` closely: it already boots straight into the
app if `app_dir/main.py` exists, and only shows `_WAITING_HTML` if it doesn't. Nothing about
booting into a bundled app is dev-loop-specific. **The only thing that makes the Dev Shell a
dev tool is `Native.startReloadServer(...)`** — the background thread that dials
`127.0.0.1:8765` forever, and the 120ms `pollHtml()` UI-thread timer that drains it.

That means the release App Shell (`phase_2/appshell/`) is not a new design, it's
`phase_1/devshell` with:
- the reload-socket thread and `pollHtml` polling loop deleted (a shipped app has no `zkite dev`
  to connect to; leaving the retry loop in would mean an infinite reconnect thread burning
  battery forever, plus an `INTERNET` permission with no purpose at runtime),
- the app's own code baked into `assets/` at build time instead of pushed over a socket,
- `AndroidManifest.xml`, `strings.xml`, and the launcher icon **generated per project**
  (`dev.zkite.devshell` was fine for one generic dev tool; every user app needs its own package
  name, label, and icon — see §3), and
- `android:hasCode="true"` and friends otherwise unchanged from the Dev Shell manifest.

Everything else — the JNI boot/dispatch surface, `ZKiteActivity`'s WebView setup, insets, asset
unpacking, extension-module `.pth` linking, `runtime.py`'s glue — is reused with the package
renamed (`dev.zkite.devshell` → `dev.zkite.appshell`) and nothing else.

### 1.3 There are, empirically, no Android-tagged wheels on PyPI yet

plan1.md §9 rates this "Medium" and says "Android wheels on PyPI are new and thin." Phase 2
checked, live, against real PyPI (2026-08-04):

```
$ pip download --platform android_24_arm64_v8a --python-version 314 \
      --implementation cp --abi cp314 --only-binary=:all: --no-deps pydantic-core
Downloading pydantic_core-0.0.1-py3-none-any.whl (1.2 kB)
```

That is not a native wheel — pip silently fell back to `pydantic-core`'s very first PyPI
upload, an empty `py3-none-any` placeholder from before the project shipped real code, because
it's the only entry in the whole release history compatible with *any* platform tag. Every
other version is `cp3xx-manylinux/win/macos` only. **Zero packages checked had an
`android_*`-tagged wheel.** A pure-Python package (`attrs`) resolves correctly to its real
`py3-none-any` wheel — that part of the pitch is solid — but anything with a native extension
currently has nothing to resolve to, and pip's default behavior (silently taking whatever
tagged wheel is technically platform-compatible) is actively dangerous here: it can hand back
years-old placeholder code with no warning. §6.5 below is the mitigation this finding demands
that plan1.md didn't anticipate: zkite's resolver cannot just call `pip download` and trust the
result, it has to inspect what pip actually resolved and flag non-`android_*` fallbacks loudly.

This is exactly what `zkite doctor` (§10) is for, and it means wheel resolution for v1 is
realistically pure-Python-dependencies-only, with loud, specific diagnostics for anything else
— not a gap to hide, a fact to surface well.

### 1.4 pyc compilation needs a host interpreter matching the target's bytecode version

plan1.md §7 step 2 is "`.py` → `.pyc` (source-strippable)." CPython bytecode is
platform-independent but **version-specific** (the `.pyc` magic number changes every minor
version); compiling with the host's Python 3.12 would produce bytecode the device's embedded
Python 3.14.6 can't load. The dev box here runs 3.12. `uv python install 3.14` cleanly
provisions a host-native (Linux x86_64) CPython 3.14.6 in about 5 seconds, with no system
package manager involvement — good enough to build and prove the pipeline in this session.

This surfaces a real open question for the actual release pipeline that's worth recording now
rather than rediscovering later: unlike the NDK/JDK (used only to build *our* release
artifacts), a matching-version host Python is needed every time **a user** runs `zkite build`,
on **their** machine. Shipping it is the honest fix — vendor host-native CPython 3.14
"compile helpers" (Linux/macOS/Windows × x86_64/arm64) in the `zkite` wheel, the same way
Android prefixes are vendored, since bytecode compilation only needs a *runnable* interpreter
of the right version, not a target-matching one. Building and testing that for four host
platforms from a single Linux x86_64 dev box is out of reach for this session; §7 below builds
and verifies the Linux x86_64 path only and flags the rest as a known gap, same honest framing
phase 0 used for arm64 CI.

**Update (phase 3, §17.2) — this section is right that a host Python is needed and
wrong about which step needs it.** The `.pyc` step compiles **only the app directory**
(`compiled 1 files to .pyc` for the stock app); the stdlib is excluded by construction,
since `prune_stdlib.py`'s own `DROP_SUFFIXES` drops `.pyc`. So bytecode compilation is
not what makes `zkite build` non-portable — and phase 3 §17.1 has since enabled the
device-side bytecode cache, which compiles the app's files on first launch anyway.

**Pruning is the real dependency, and it needs a *matching* interpreter, which this
section does not say.** Measured: `prune_stdlib.py` under Python 3.12 against the 3.14
stdlib keeps 138 top-level packages where 3.14 keeps 139, and the single missing entry
is `annotationlib` — new in 3.14, and pulled in by merely `import zkite`. Pruning under
a mismatched host therefore ships an app that cannot boot, failing on device with a
`ModuleNotFoundError` and nothing visible at build time: the exact shape of §1.7's
`_android_support` bug. The "just use whatever Python is around" shortcut is refuted,
with a mechanism. See `../phase_3/PLAN.md` §17.2 for the options.

### 1.5 `protoc` is a release-pipeline tool, not a user-facing one

Phase 0's README already flagged `pack-aab`'s build script requiring `protoc` to compile its
`.proto` files into Rust. That compile step happens **once, when `zkite`'s own binary is built**
— exactly the same bucket as `javac`/`d8` for `classes.dex` and the NDK for cross-compiling
`libpython`. A user running `zkite build aab` never touches `protoc`; they run a prebuilt `zkite`
binary that already has `pack-aab`'s generated code linked in. This environment has no system
`protoc` and no `sudo`, so §8 downloads the official `protoc` release binary from GitHub
(network-reachable, no install, `PROTOC` env var) purely to get *this session's* `zkite` binary
built — indistinguishable in kind from phase 0 needing an NDK.

### 1.6 `pack-aab`'s `construct_aab` has the exact same "no code" gap as APK, same fix

`pack_aab::construct_aab` returns `Vec<pack_zip::File>` — `BundleConfig.pb`, `base/resources.pb`,
`base/manifest/AndroidManifest.xml`, `base/res/**` — before v1/v2/v3 signing and zipping. There
is no field for `base/dex/classes.dex`, `base/lib/<abi>/*.so`, or `base/assets/**`, for the
identical reason `pack_api::Package` had none for APK: `pack` was written for code-free Wear OS
watch faces. `phase_2/bundler::aab` closes it exactly the way `spike2_pack_apk::apk` closed the
APK version — splice verbatim `pack_zip::File` entries under the right AAB module-relative
prefix before `add_v1_signature_files` + `zip_apk` + `sign_apk_buffer` — and is a comparably
small, comparably upstreamable addition.

### 1.7 Reachability pruning can drop a module nothing in *application* code ever imports

`_android_support` (CPython's own stdout/stderr-to-logcat redirection shim, added
for official Android support) is imported during interpreter startup itself —
before `runtime.py`'s glue even runs — not from anything in `zkite_py` or app code.
No static bytecode walk seeded from application source can ever see it, by
construction. Pruning it out produced a build that looked completely fine (packed,
signed, `apksigner`/`zipalign`/`aapt2` all happy) and then failed silently on-device:
`Py_InitializeFromConfig` failed with no visible error, because Android drops a bare
C-level `stderr` write from an app process — the exact same swallowing phase 0's
`PYTHONHOME` finding already documented, now hitting a different bug. The standalone
probe binary (§1.2 already established this pattern; used again here) showed the
real error immediately: `ModuleNotFoundError: No module named '_android_support'`.
Fixed by adding it to `prune_stdlib.py`'s `ALWAYS_KEEP` floor — this is exactly the
class of platform-runtime dependency that floor exists for, not a one-off patch.

### 1.8 The dev-loop glue hardcoded `main.py`; a release build ships `main.pyc`

`runtime.py`'s `_zkite_setup`/`_zkite_reload` (§1.2: reused unmodified from the Dev
Shell) check `os.path.exists(os.path.join(app_dir, "main.py"))` literally — correct
for `zkite dev`, which always pushes source, but after M3's pyc-compile step deletes
`main.py` and leaves only `main.pyc`, that check is always false, so every release
build showed the Dev Shell's "waiting for `zkite dev`…" placeholder instead of the
app, indistinguishable from a boot failure by looking at the screen alone. Fixed
with a three-line `_find_main()` helper checking `main.py` then `main.pyc`, in
`phase_1/devshell/src/runtime.py` (shared verbatim by both shells, so the fix
applies to both without touching `appshell`'s own code at all). Backward-compatible
by construction: the dev loop only ever hits the `main.py` branch, so phase 1's own
behavior is unchanged. `zkite_py`'s `load_module` (`importlib.util.
spec_from_file_location`) already handled a `.pyc` path correctly on its own —
the bug was entirely in the file-existence check one layer up, not in module loading.

---

## 2. Architecture

```
                     zkite build apk|aab   (runs on the user's machine)
┌───────────────────────────────────────────────────────────────────────────┐
│ zkite CLI (Rust binary, unchanged identity from phase 1)  — phase_1/cli/   │
│  build_cmd.rs  →  phase_2/bundler (path dep, linked in, no subprocess)   │
│  doctor_cmd.rs, keygen_cmd.rs                                             │
└───────────────────────────────────────────────────────────────────────────┘
                              │
      ┌───────────────────────┼────────────────────────────┐
      ▼                       ▼                             ▼
 wheels.rs               pyc.rs / prune.rs              manifest_gen.rs
 shells to the user's    shells to a matching-version    pyproject.toml → per-app
 own `pip` (already      Python (§1.4) for compileall    AndroidManifest.xml,
 required to `pip        + modulefinder reachability     strings.xml, icon
 install zkite`)          pruning of the Android stdlib.zip
      │                       │                             │
      └───────────────────────┴────────────────┬────────────┘
                                                 ▼
                                    stage/  (classes.dex, lib/<abi>/*.so
                                             incl. libpython + extension
                                             modules, assets/{python-
                                             stdlib.zip, zkite-framework.zip,
                                             app.zip, webruntime/})
                                                 │
                                                 ▼
                              phase_2/bundler::{apk, aab}  (extends
                              pack_asset_compiler / pack_sign / pack_zip,
                              linked in — §1.1)
                                                 │
                                                 ▼
                                app-release.apk / app-release.aab
                                signed with the user's --keystore or an
                                ephemeral testing key (loud warning, §9)
```

Device side, once installed: `phase_2/appshell` — `phase_1/devshell` minus the reload socket
(§1.2). Package `dev.zkite.appshell.*`, `ZKiteActivity`/`Native` unchanged in shape.

---

## 3. Per-app identity: package name, label, icon

Android requires a globally-unique reverse-DNS `applicationId`. `pyproject.toml`'s
`[project] name` is not that (no reverse-DNS component, and two unrelated `myapp` projects on
PyPI would collide). New, optional section:

```toml
[tool.zkite.android]
package = "com.example.myapp"   # applicationId; defaults if absent (see below)
label = "My App"                # defaults to [project].name if absent
```

If `package` is absent, `zkite build` derives `dev.zkite.app.<sanitized-project-name>` and prints
a one-line warning that this is a placeholder namespace, not something to publish under — zkite
cannot guess a developer's real domain, and silently shipping every unconfigured app under one
guessable, unowned namespace would be worse than refusing to guess quietly. `zkite new` does not
write this section by default (plan1.md §2 principle 5: boilerplate is a bug; the field is only
needed at `build` time, and most `zkite dev` sessions never reach it).

Icon: reuses the Dev Shell's single density-independent adaptive icon approach (phase 0 §7's
"sidestep" mitigation for `pack`'s missing resource qualifiers) — one bundled default icon for
v1, `[tool.zkite.android] icon = "path/to/icon.png"` as the override. A per-app custom icon
pipeline (adaptive icon layers, multiple densities) is real scope and explicitly deferred.

> **Correction (phase 3, §21/§22).** Two things above were wrong about the code as
> shipped. Both are now true; the history is kept because *how* the first one survived
> matters more than the fix.
>
> 1. **`[tool.zkite.android] icon` was documented here from phase 2 and not implemented
>    until phase 3 §22.** For three phases no code in this repo read the key —
>    `build_cmd.rs::default_icon` unconditionally read
>    `phase_2/appshell/app/res/drawable/icon.png`, so every app ever built shipped the
>    same square, and the paragraph above was the *only* evidence the feature existed
>    (phase_3/PLAN.md §21.1).
>
>    **What shipped (phase_3/PLAN.md §22.1):** `build_cmd.rs::load_icon` reads the key,
>    resolves it against the project directory (absolute paths pass through), validates
>    the file really is a PNG by signature + IHDR — `pack` stores resource bytes
>    verbatim and never transcodes, so a `.jpg` renamed `.png` would otherwise pack,
>    sign and install cleanly and fail to decode on the device — and falls back to the
>    bundled default when the key is absent. It prints the resolved dimensions, warns
>    below 192x192 and on non-square, and errors with **both** what was written and
>    where that resolved to. Validation runs beside `validate_package` rather than at
>    the resource-assembly site, because everything in between (wheels, pyc, pruning,
>    signing) is expensive: a typo'd path took a measured 9.2s to report, and now takes
>    0.00s. `zkite new` scaffolds the key **commented out** — it is the one identity key
>    that names a file, so an uncommented default would fail every first build.
>    Verified live on a Moto G54: a 512x512 PNG lands byte-identical in the APK and is
>    the icon the launcher draws.
>
>    **Still deferred, and now with a measurement behind it:** the adaptive-icon half.
>    §3's own sentence above — "reuses the Dev Shell's single density-independent
>    *adaptive* icon approach" — is wrong twice over, because what ships is a plain
>    legacy `drawable`, and a legacy icon is not an adaptive one. Observed on the
>    device: the launcher shrinks a legacy PNG and mats it onto a white plate rather
>    than letting it fill the icon mask, so a custom icon is visibly inset where a
>    system app's is not. That is the concrete cost of phase_0/README.md §7's
>    qualifier gap, and closing it is a `pack` change first. See phase_3/PLAN.md §22.2.
> 2. **`zkite new` now *does* write this section**, with `com.example.<sanitized-name>`
>    and a comment (phase_3/PLAN.md §21.2). The reasoning above was sound and the
>    outcome was still bad: the keys were discoverable nowhere, so users reached upload
>    with a placeholder package. `com.example.*` is chosen precisely because Play
>    rejects it, and `zkite build` now warns on both zkite-owned namespaces.
>
> **Neither key is read by `zkite dev`**, and that is structural rather than an
> oversight: the dev loop installs a *generic* Dev Shell and pushes only `.py`, so the
> package name, label and icon on screen during `zkite dev` are the Dev Shell's own.
> Nothing said so until phase_3/PLAN.md §24.

---

## 4. Milestones

| # | Milestone | Needs a device? | Status |
|---|---|---|---|
| M1 | `phase_2/bundler`: APK + AAB packing library with `extras`/code support, promoted from spike 2's `apk.rs` pattern (§1.1, §1.6), plus `zkite keygen` (PEM + PKCS12) | No | **Built + tested** |
| M2 | `phase_2/appshell`: release App Shell — Dev Shell minus reload socket (§1.2), per-app manifest/icon | No (build); yes (verify) | **Built** — cross-compiled for arm64-v8a + x86_64, 16KB page alignment confirmed; device install/launch verification is M7 |
| M3 | `compile_pyc.py`: bytecode compilation via a matching-version host Python (§1.4) | No | **Built + tested** |
| M4 | `prune_stdlib.py`: stdlib pruning via `modulefinder` reachability, seeded from the actual app | No | **Built + tested** |
| M5 | `wheels.rs`: dependency resolution via the user's `pip` + Android platform tags, with fallback detection (§1.3) | No | pending |
| M6 | `zkite build apk` end to end | No (build); yes (verify) | **Built + tested** |
| M7 | Verify: `zkite new` → `zkite build apk` → install + launch on a real device and the emulator | Yes | **Verified live** — Moto G54 5G (arm64-v8a) and an x86_64 emulator, both standalone, both tap-interactive |
| M8 | `zkite build aab`, `protoc` vendored for this build (§1.5), verified with `bundletool` (Play's own validator) | No (build); yes (verify) | **Built + tested** (structural validation only — see §6) |
| M9 | `zkite keygen` + `zkite doctor` | Partially (adb check needs a device to be meaningful) | **Built + tested** |
| M10 | Timing/size measurements against §7's "<10s release build" exit criterion | Yes | **Measured** — see this doc's §7/§8 |

`zkite keygen`'s `--keystore`/PKCS12 wiring lives in M1 rather than as its own milestone
(plan1.md's own roadmap bullet groups them: "signing and keystores"); the CLI flag that
consumes it is part of M6.

Each section below is filled in as its milestone completes, same convention as phase 1 §7 —
"pending" until there's a real result to report, never a description of an architecture that
should work.

---

## 5. Directory layout

```
phase_2/
├── PLAN.md                  this file
├── bundler/                 M1 — packing library (path-dep of phase_1/cli)
│   ├── Cargo.toml
│   ├── src/{lib.rs, apk.rs, aab.rs, keystore.rs, manifest_gen.rs, zip_extras.rs}
│   └── tests/{apk.rs, aab.rs, keystore.rs}      — verified against real apksigner/
│                                                   zipalign/aapt2/bundletool/keytool/openssl
├── appshell/                 M2 — release App Shell (Rust cdylib + Java)
│   ├── Cargo.toml
│   ├── src/{lib.rs, bin/probe.rs}               — runtime.py is phase_1/devshell's, reused (§1.2)
│   ├── java/dev/zkite/appshell/{ZKiteActivity,Native}.java
│   ├── app/res/drawable/icon.png                — default app icon (§3)
│   ├── scripts/{build.sh, build_dex.sh}         — release-pipeline only (§1.1)
│   └── artifacts/                               — prebuilt .so/classes.dex zkite build reads
├── pybuild/                  M3/M4 — python-side build helpers, invoked by the CLI
│   └── scripts/{compile_pyc.py, prune_stdlib.py}
├── vendor/
│   ├── protoc/                release-pipeline-only, builds pack-aab (§1.5)
│   ├── bundletool/            Play's own AAB validator, used only to verify (§1's M8 note)
│   └── pip/pip.pyz            vendored into the real `zkite` wheel too — user-facing (§1.3)
└── fixtures/testapp/         a disposable `zkite new` app used to exercise `zkite build`
```

The CLI itself is **not** duplicated here — `phase_1/cli` gained `build_cmd.rs`,
`doctor_cmd.rs`, `keygen_cmd.rs`, `wheels.rs`, and path-dependencies on
`phase_2/bundler` plus `pack-asset-compiler`/`pack-sign` directly (for resource/key
types `build_cmd.rs` needs). One binary evolving across phases, the same way phase 1
extended (not forked) phase 0's spike 1 embedding code.

**Update (phase 3, §14): that pattern held a third time.** `zkite build apk` now takes
`--backend native` and stages `phase_3/gpu_harness`'s prebuilt artifacts instead of
`appshell`'s. `build_cmd.rs` did not fork or grow a second command — the four places
that named appshell became the four fields of one `Shell` struct, and `AppIdentity`
gained an `activity` so the generated manifest can name the right launcher class.
Everything §6 describes (wheels, pyc, pruning, `app.zip`, signing, alignment) is one
shared code path for both backends, because the app being built is the same app. See
`../phase_3/PLAN.md` §14.2.

---

## 6. Notes as milestones land

### M1 — `bundler`: APK + AAB packing, `zkite keygen`

`phase_2/bundler` builds clean on the first try in both configurations (`cargo build`
for APK-only, `cargo build --features aab` with `protoc` on `PATH`/`$PROTOC` for AAB
— see §1.5). Verified with real tools, same standard phase 0 set for `pack`'s output:

- `packed_apk_verifies_with_real_android_tools` (`tests/apk.rs`): a manifest +
  `res/` + staged `classes.dex`/`assets/app.zip`, packed and signed through
  `apk::compile_and_sign_apk_with_extras`, passes `apksigner verify` (v2 **and** v3
  schemes), `zipalign -c`, and `aapt2 dump packagename` — identical checks to phase
  0's spike 2, now run against the promoted library instead of the spike binary.
- `packed_aab_validates_with_bundletool` (`tests/aab.rs`): the same shape, packed
  through `aab::compile_and_sign_aab_with_extras`, passes `bundletool validate`
  outright — this is literally the tool Play's own backend runs on an upload, so
  "structurally acceptable to Play Console" is genuinely checked here, not just
  asserted. (Actual Play Console acceptance also needs a real developer account and
  upload, which this environment doesn't have — see the exit-criterion note in §9.)
- `extras_land_under_the_base_module` / `extras_land_in_the_apk_byte_for_byte`:
  confirm the splice itself — `dex/`, `lib/<abi>/`, `assets/**` land at the paths
  `appshell`'s build script will actually stage them at, with `base/` correctly
  prefixed for AAB and not for APK.
- `duplicate_extra_paths_panic_instead_of_silently_clobbering`: confirms the zip-slip/
  duplicate-path guard fires *in the actual pack call*, not just in isolation — an
  earlier version of this test had a second bug (see below) that made it pass for the
  wrong reason, worth recording since it's exactly the kind of false-positive that's
  easy to miss.

**Finding, caught by the test suite, not by inspection:** the first draft of
`duplicate_extra_paths_panic_instead_of_silently_clobbering` omitted the `drawable/
icon.png` resource that `android_manifest()` references via `android:icon=
"@drawable/icon"`. `pack` fails *manifest resolution* on the missing reference before
ever reaching the extras loop, so the test "passed" — a panic occurred, just the
wrong one, for a reason that had nothing to do with duplicate-path detection.
Concretely: `catch_unwind` only tells you *a* panic happened, never *which* one;
without a matching-message assertion, a test like this can go green for the wrong
reason indefinitely. Fixed by including the icon resource so the code path actually
reaches the assertion under test; worth remembering next time a `catch_unwind` test
is tempting.

**Finding: `p12`'s generated PKCS#12 needs OpenSSL 3's `-legacy` flag, but not
Java's.** `p12::PFX::new` encrypts with RC2-40-CBC (cert bag) and 3DES (key bag) —
the traditional PKCS#12 algorithms every JDK `keytool`/`jarsigner` still reads
natively. OpenSSL 3.0 moved both behind the `legacy` provider by default, so a plain
`openssl pkcs12 -in ...` fails with "unsupported... Algorithm (RC2-40-CBC)" on a
completely well-formed file. `tests/keystore.rs` checks both: `keytool -list` (the
interop that actually matters — this is the format Play Console's upload-key flow
and Gradle's signing config both expect) passes without any flag; `openssl pkcs12`
needs `-legacy`, documented inline rather than silently worked around, since a real
user hitting this with their own OpenSSL would see the identical error.

Also confirmed empirically (§1.4's host-Python finding didn't block this, since
keystore generation needs no target-version Python at all): `zkite keygen`'s
self-check (`Keys::from_combined_pem_string` round-trip, inside `generate()`) means a
malformed keystore fails at `zkite keygen` time with a specific message, never
silently at the next `zkite build`.

### M2 — `appshell`

`cargo check` (host target) and the real cross-compile (`scripts/build.sh aarch64
x86_64`, same `PYO3_CONFIG_FILE`/NDK-clang approach as `phase_1/devshell`) both
succeed on the first try — expected, since the diff from devshell really is just
deletion (reload socket, `pollHtml`, the `Frame`/`OutFrame` wire types) plus one
addition (`app.zip` unpacking in `ZKiteActivity`), not new design. `readelf -lW`
confirms both `.so`s keep the 16 KB `PT_LOAD` alignment (`0x4000`) the `-Wl,-z,
max-page-size=16384` flag asks for — the same assertion `bundler::apk::
assert_native_libs_are_sane` checks mechanically at pack time. Device install/launch
is M7, once `zkite build apk` (M6) can actually produce a full APK to install.

### M3/M4 — `compile_pyc.py`, `prune_stdlib.py`

Both verified against the real `zkite new` counter app fixture (`fixtures/testapp/`)
plus `zkite_py` itself, using the `uv`-provisioned host-native CPython 3.14.6 (§1.4):

- **pyc**: compiling `fixtures/testapp/main.py` to a legacy-location `main.pyc` and
  deleting the source, the result imports correctly two ways — plain
  `sys.path`-based import, and `zipimport` from inside a real `.zip` (`app.zip`) —
  confirming plan1.md §7's "assets/app.zip (zipimport-able)" holds for the pyc
  output specifically, not just for `.py` source.
- **prune**: run against the real vendored `python-x86_64` prefix, seeded from
  `zkite_py/src` (as a package, via `import_hook`) and `fixtures/testapp` (as a
  script, via `run_script`) — **138 top-level stdlib packages kept, 59 dropped**,
  8.7 MB kept / 47.5 MB dropped, 2.3 MB compressed (vs. phase 0's static-deny-list-only
  2.7 MB — a real, if modest, improvement, because most of the *byte* weight was
  always in `test`/`idlelib`, which both approaches already drop; the reachability
  pass's actual win is dropping whole irrelevant packages like `sqlite3`, `asyncio`,
  `multiprocessing`, `xml`, `http` at *package* granularity for apps that never
  touch them). Boot-tested for real, not just packed: unzipping the pruned prefix
  and launching a fresh `python3.14 -S` against it with `PYTHONHOME` set boots clean
  (`encodings` resolves, `dataclasses`/`inspect`/`traceback`/`typing` — `zkite_py`'s
  own stdlib footprint — all import), and a full `zkite_py` render
  (`App(main).render()`) against that same pruned prefix produces the identical HTML
  the un-pruned dev loop does.

**Finding, caught by the test run rather than by reading the docs:**
`modulefinder.run_script` always scans its argument as `__main__` at import level 0.
That's correct for `main.py` but wrong for anything that is actually a *package
submodule* — every file under `zkite_py/src/zkite/` does a package-relative import
(`from . import widgets`), and `run_script`-ing them individually raised
`ImportError: relative importpath too deep`, because from `run_script`'s point of
view a relative import inside a fake top-level `__main__` has nowhere to resolve
*to*. Fixed by classifying each entry directory's direct children into scripts
(`.py` files — `run_script`) vs. packages (dirs with `__init__.py` —
`finder.import_hook(name)`, which resolves relative imports against the package's
real identity). Same category of bug as phase 0's `PYTHONHOME`-in-isolated-config
finding: passes for something that looks similar, fails for the specific real
shape being fed in — worth checking against the actual framework code, not a toy
script, for exactly this reason.

### M5/M6/M7 — `wheels.rs`, `zkite build apk`, live device verification

`wheels.rs` confirmed §1.3's finding as an automated, regression-pinned test rather
than a one-off manual check: `pure_python_dependency_resolves_cleanly` (a real `pip
download` against real PyPI for `attrs`) and `native_only_dependency_is_flagged_not_silently_trusted`
(same, for `pydantic-core` — resolves `0.0.1` when the real latest is `2.47.0`,
and `version_mismatch` catches it). Both pass live.

`build_cmd.rs` wires everything M1–M5 built into one `zkite build apk` command,
reading only prebuilt release-pipeline artifacts (§1.1) plus per-project state —
no NDK, no JDK, no `cargo`, nothing beyond a matching-version host Python and pip.
First full run, against `zkite new`'s own counter app (`fixtures/testapp/`):

```
building testapp for apk: arm64-v8a, x86_64
  using .../cpython-3.14.../bin/python3.14 for pyc compilation + stdlib reachability
    compiled 1 files to .pyc
    reachability: 139 top-level packages kept, 58 dropped
    kept      488 files      8.7 MB / dropped 47.5 MB / 2.3 MB compressed
  [arm64-v8a] 16 KB page alignment: 69 native libraries OK
  wrote testapp-arm64-v8a.apk (7.0 MB)
  [x86_64] 16 KB page alignment: 69 native libraries OK
  wrote testapp-x86_64.apk (7.2 MB)
done in 7.38s
```

Both APKs verify with real `apksigner` (v2 **and** v3), `zipalign -c`, and
`aapt2 dump packagename` — same tools, same checks, phase 0's original bar.

**M7, live on real hardware, not simulated:** installed and launched on both an
x86_64 emulator and a real Moto G54 5G (arm64-v8a, Android 15) — the same physical
phone phase 0 used for its own real-hardware confirmation. On both: the app boots
straight to the counter UI with **no** "waiting for `zkite dev`…" screen and **no**
`zkite dev` process running anywhere — this is the actual architectural claim phase 2
exists to prove, not an inference from a passing build. `adb shell input tap` on
Increment twice took the on-screen counter `0 → 2` on both devices, confirming
`ZKiteNative.dispatch` → JNI → Python → `store.update()` → re-render →
`evaluateJavascript` all work identically to the Dev Shell's path, just without a
socket anywhere in it. Screenshots taken and visually confirmed on both.

**Two bugs found only by this device run, not by any host-side check** — same
pattern phase 1's M4 called out ("found only by running on a real emulator, not by
any host-side test"), now repeated for phase 2:

1. **§1.7**: pruning dropped `_android_support`, silently (`Py_InitializeFromConfig`
   fails, Android drops the stderr write). Caught by reviving the standalone-probe
   technique — pushed `zkite-appshell-probe` (cross-compiled, same build as the real
   `.so`) plus the *exact* staged `python-stdlib.zip`/`zkite-framework.zip`/`app.zip`
   from the built APK to `/data/local/tmp` and ran it via `adb shell`, where stderr
   is visible. Fixed in `ALWAYS_KEEP` (§1.7).
2. **§1.8**: `runtime.py` checked for `main.py` literally, never finding the
   `main.pyc` M3 actually ships, so every release build silently showed the "waiting
   for `zkite dev`" placeholder — no crash, no error, just the wrong (but
   plausible-looking) screen. Fixed with `_find_main()` (§1.8).

Both required the same diagnostic move: don't trust "the build succeeded and the
tools are happy," actually look at the device. `apksigner verify` passing says
nothing about whether the interpreter can boot; a rendered screen that isn't the
error overlay isn't proof it's the *right* screen.

### M8 — `zkite build aab` (the actual command, not just the library)

`bundler::aab`'s packing logic needed no changes (M1 already covered it). The CLI
command reuses ~90% of `zkite build apk`'s staging — same `stage_common`, same
`stage_native_libs`, same wheel/pyc/prune pipeline — except that it stages **every**
requested ABI into *one* staging tree instead of building one APK per ABI, since an
AAB's whole purpose is bundling all ABIs into a single artifact for Play to split at
install time.

**Finding, from `bundletool validate` on the first real CLI-driven build, not from
reading the AAB spec first:** `Error: Module files can be only in pre-defined
directories, but found 'classes.dex'.` The APK and AAB layouts genuinely differ for
exactly one of the four staged paths — `assets/**` and `lib/<abi>/**` are identical
between the two formats (the `base/` prefix `aab.rs` adds is uniform), but
`classes.dex` belongs at the **APK root** and under **`dex/`** for an AAB's `base`
module. Sharing one `stage_common` between both formats (reasonable, since 3 of 4
paths really are shared) silently produced a structurally invalid bundle that packed
and signed without any error — `pack_zip`/`pack_sign` don't know or care about AAB's
module-layout conventions, only `bundletool` enforces them. Fixed by parameterizing
`stage_common`'s dex destination per format (`"classes.dex"` vs. `"dex/classes.dex"`).

**Full pipeline verified, not just structural validation:**

```
$ zkite build aab --keystore zkite-release.pem
  16 KB page alignment: 138 native libraries OK   # 69 libs × 2 ABIs, one bundle
  wrote testapp.aab (11.9 MB)
done in 6.60s

$ java -jar bundletool.jar validate --bundle=testapp.aab
App Bundle information
------------
Feature modules:
	Feature module: base
		File: assets/app.zip
		...
		File: dex/classes.dex
		File: lib/arm64-v8a/... (69 files)  File: lib/x86_64/... (69 files)
(exit 0)
```

Then further than structural validation: `bundletool build-apks --bundle=testapp.aab
--ks=zkite-release.p12 ... --mode=universal` — using the **exact PKCS#12 `zkite keygen`
produced** — succeeded and emitted `universal.apk`, which installed and launched
clean on the emulator (`adb install` + `am start`, verified via logcat and a
screenshot, same bar as M7). This is the same two-stage process Play's own backend
runs on every upload (validate the bundle, then generate device-specific APKs from
it) — about as close to "accepted by Play Console" as reachable without an actual
developer account and upload, and a materially stronger check than `validate` alone
since it proves the AAB's *content*, not just its shape, is real and installable.

### M9 — `zkite keygen`, `zkite doctor`

`zkite keygen --out zkite-release --cn testapp --password ...` writes
`zkite-release.pem` + `zkite-release.p12` and self-checks the PEM via the same
`Keys::from_combined_pem_string` `zkite build` uses, per `bundler::keystore`'s design
(§6 M1). The alias defaults to the `--cn` value — worth knowing when scripting
`bundletool`/`keytool` against the `.p12` directly, as M8's verification run did
(`--ks-key-alias testapp` after `--cn testapp`).

`zkite doctor` checks adb+device, a matching-version Python, pip (vendored or
system), and prebuilt appshell artifacts, then — if run inside a project directory —
dry-resolves every declared dependency through the same `wheels.rs` path `zkite
build` uses and reports each as `ok` (android-tagged or pure-Python) or `WARN`
(§1.3's stale-fallback symptom) or `FAIL` (nothing resolves at all). Verified against
a scratch project declaring `["zkite", "attrs", "pydantic-core"]`: `attrs` reports
`ok`, `pydantic-core` reports the exact `WARN` §1.3 predicted
(`resolved 0.0.1 but latest is 2.47.0`) — the diagnostic plan1.md §10 asks for,
doing the actual job rather than just existing.

---

## 7. Timing and size measurements

All measured on this session's dev box (Linux x86_64, the same one phase 0's
emulator numbers came from), building `fixtures/testapp` — `zkite new`'s own counter
app, one `.py` file, zero third-party dependencies.

| | Measured | §7's target |
|---|---|---|
| `zkite build apk`, single ABI | **7.95 s** | 3–8 s |
| `zkite build apk`, both ABIs (2 output files) | 8.67–8.86 s | — (not a single-artifact number) |
| `zkite build aab`, single ABI | **6.38 s** | §10: "release build under 10s" |
| `zkite build aab`, both ABIs (1 bundle) | **15.10 s** | §10: "release build under 10s" — **misses** |
| APK size, one ABI, pruned stdlib | 7.0–7.2 MB | §7: 20–26 MB |
| APK size, one ABI, `--no-prune-stdlib` | 20.2 MB | §7: 20–26 MB |
| AAB size, both ABIs | 11.9 MB | — |
| stdlib pruning: kept / dropped | 139 / 58 top-level packages | — |
| stdlib zip, compressed | 2.3 MB | phase 0 (deny-list only): 2.7 MB |
| 16 KB native-lib page alignment | 69 (apk) / 138 (aab) libs, all pass | §9 risk, mechanically checked every build |

**The single-target numbers meet §10's exit criterion; the two-ABI AAB does not** —
15.1s against a 10s target, worth stating plainly rather than rounding away. It
isn't `zkite build`'s own orchestration that's slow (`--no-prune-stdlib` — which
skips the reachability scan entirely — took *longer*, 15.0s, because the resulting
payload is 3× bigger and slower to compress, which is itself evidence the bottleneck
is payload size going through DEFLATE, not the Rust/Python glue around it). The
scaling is roughly payload-proportional: single-ABI AAB (7.2 MB, 69 native libs)
takes 6.4s; both ABIs (11.9 MB, 138 libs, mostly already-compressed `.so` binaries
that DEFLATE can't shrink much but still pays CPU to try) takes 15.1s — worse than
linear, consistent with `pack_zip` deflating every entry including native libraries
it can't meaningfully compress. This is precisely the second gap phase 0's README
already flagged and deferred: *"`pack` deflates every entry except
`resources.arsc`, so `extractNativeLibs="false"` — uncompressed, page-aligned libs
— is not reachable today... a `CompressionMethod::Stored` path for `lib/**/*.so`"*
is still an open upstream contribution, not a new problem phase 2 introduced, and
looks like the direct fix for this specific miss — stored (uncompressed) native
libs would cut both the AAB's size and the CPU time currently spent deflating
incompressible binaries.

---

## 8. Exit criterion tracking

plan1.md §10's Phase 2 exit: *"a signed AAB is accepted by the Play Console. Release
build under 10s."*

- **Signed AAB, structurally real**: `zkite build aab` produces a bundle that passes
  `bundletool validate` (§6 M8) — literally Play's own validator — and
  `bundletool build-apks` successfully re-derives a real, installable, launchable
  APK from it using a `zkite keygen`-produced keystore (§6 M8, verified live on the
  emulator). What this *doesn't* cover: an actual Play Console account and upload,
  which this environment has no way to exercise. Every check reachable without one
  has been run, and passed.
- **Release build under 10s**: true for single-target builds (7.95s apk, 6.38s aab)
  against a small (one-file) app; **false** for a two-ABI AAB (15.1s) — see §7 for
  the honest breakdown and the concrete, already-identified fix (upstream `pack`'s
  pending `Stored`-for-`.so` gap).
- **Everything else §10 names** — wheel resolution, pyc + zipimport, stdlib
  pruning, signing and keystores, `zkite doctor` — built and verified live, not
  just architecturally described (§6, M1–M9).

---

## 9. Known gaps

Honest list of what phase 2 does not establish, same convention as phase 0/1.

- **AAB build time exceeds the 10s target for multi-ABI bundles** (§7/§8) — root
  cause identified (DEFLATE over incompressible native libs), fix not yet built
  (needs the `Stored`-for-`.so` upstream `pack` contribution phase 0 already
  flagged, then wiring `extractNativeLibs="false"` into the generated manifest).
  **Status as of 2026-08-04: half-done and uncommitted.** A working-tree change to
  `manifest_gen.rs` already flips `android:extractNativeLibs` to `false` and adds a
  test asserting it, with a comment claiming it's safe because `lib/**/*.so` is
  "stored uncompressed and page-aligned." That precondition does not exist yet —
  there is no `Stored`/`CompressionMethod` code anywhere in `phase_2/bundler` or the
  vendored `pack` crates, so as this stands the manifest would tell Android the libs
  are uncompressed while `pack_zip` still DEFLATEs them. That's exactly the "passes
  every host-side check, fails on device" shape this document's own §6/M6-M7 kept
  finding (§1.7, §1.8). Left uncommitted and untouched deliberately — not part of
  phase 3, see `../phase_3/PLAN.md` — pick this up by either finishing the
  `Stored`-for-`.so` write path first, or reverting the manifest flag until it does.

  **Update (phase 3, §11.8): the `Stored`-for-`.so` write path did land** — every
  `lib/**/*.so` entry in a `zkite build apk` output is now verifiably
  `ZIP_STORED` (72/72, checked against a real built APK), so the precondition this
  entry said was missing now exists and the flag was genuinely safe on its own
  terms. **It has nonetheless been flipped back to `"true"`, for an unrelated
  reason that outweighs it:** with `extractNativeLibs="false"` nothing is written
  to the app's `nativeLibraryDir`, so CPython has no filesystem path from which to
  `dlopen` an extension module, and *every* C-extension stdlib module (`math`,
  `random`, `zlib`, `ssl`, `sqlite3`) becomes unimportable. Measured on a real
  device: `0 native extension modules available` with `"false"`, `67` with
  `"true"`. The AAB timing gap this entry is about is therefore **still open**, and
  closing it now needs a way to load extension modules without extraction — bionic's
  APK-internal `<apk>!/lib/<abi>/<name>.so` `dlopen` form is the obvious candidate
  and is unexplored. See `../phase_3/PLAN.md` §11.8 and `manifest_gen.rs`'s own
  comment.
- **Wheel resolution is realistically pure-Python-only.** §1.3's finding is
  structural, not a phase-2 shortcoming: as of 2026-08-04, *no* PyPI package ships
  an `android_*`-tagged wheel. `wheels.rs`/`zkite doctor` correctly detect and warn
  about this rather than hide it, but a project depending on anything with a native
  extension will still ship broken until the wheel ecosystem catches up — nothing
  phase 2 can fix unilaterally.
- ~~**No per-app icon at all**~~ — **built in phase 3, §22.1.** This line originally
  claimed "one bundled default icon, `[tool.zkite.android] icon = "path.png"` as a
  manual override (§3)", which was false for three phases: the key was read nowhere
  (phase 3 §21.1). It is now read, resolved against the project directory, validated
  as a real PNG, and verified live on a device. **The adaptive-icon half (foreground/
  background/monochrome layers, `mipmap-anydpi-v26/`, density variants) is still
  deferred and still blocked on `pack`'s qualifier gap** (phase 0 §7) — with a
  device-observed cost attached to it now rather than an argued one: launchers mat a
  legacy PNG onto a white plate instead of letting it fill the icon mask. See §3's
  correction and phase_3/PLAN.md §22.2.
- **`version_code` defaults to 1 and never auto-increments.** Fine for `zkite build`
  iteration; a real release process needs the developer (or a future `zkite`
  feature) to bump `[tool.zkite.android] version_code` themselves.

  **Update (phase 3, §17.4): there is a second, worse half of this that was never
  recorded.** `build_cmd.rs` builds the manifest **once**, outside the per-ABI loop, so
  **every per-ABI APK carries the same `versionCode`**. Google Play's multiple-APK
  support requires each APK in a release to have a *distinct* version code, so
  `zkite build apk` targeting two ABIs currently produces a pair Play will reject — and
  it fails at upload with a confusing message rather than at build. `zkite build aab` is
  unaffected (one artifact), which is presumably why this went unnoticed; AAB is also
  what Play requires for new apps. The fix is a per-ABI offset (the conventional scheme
  is +1000 per ABI) plus a build-time assertion against Play's 2,100,000,000 ceiling —
  the same fail-loudly-at-build posture `assert_native_libs_are_sane` already takes.

  **Update (phase 3, §29.3): the per-ABI collision is fixed; the defaults half is not.**
  `manifest_gen::abi_version_code(base, abi)` returns `base * 10 + <permanent per-ABI
  rank>` and the manifest is built **inside** the per-ABI loop, with a build-time assert
  against Play's 2,100,000,000 ceiling. Confirmed by `aapt2 dump badging` reading the real
  binary AXML (13 and 14 from a base of 1, where both were 1) and by Android's own package
  manager after install. The scheme is deliberately **not** the `+1000 per ABI` convention
  named above — that is Google's `rank * 1000 + base`, which is not monotonic in the base,
  so bumping the version would not raise every ABI above every code of the previous
  release. `zkite build aab` still uses the base unmodified (one artifact, no collision),
  and now warns about the APK→AAB downgrade that difference implies. **Still open:**
  `version_code` defaults to 1, never auto-increments, and is not derived from
  `[project] version`.
- **Host-native pyc-compiler vendoring is Linux x86_64 only** (§1.4). The real fix —
  vendoring host-native CPython 3.14 "compile helpers" for macOS/Windows × arm64/
  x86_64 into the shipped `zkite` wheel — needs those host platforms to build and
  test on, unavailable from this single Linux dev box. Today, `zkite build` on
  another OS needs the user (or their own `uv python install 3.14`) to provide a
  matching interpreter themselves; `zkite doctor` at least reports clearly when
  that's missing rather than failing unhelpfully mid-build.
- **`pip.pyz` is vendored for wheel resolution but not yet wired into `zkite`'s own
  packaged wheel** — this session's `phase_2/vendor/pip/pip.pyz` is real and used
  by every `zkite build`/`zkite doctor` run *in this checkout*, but shipping it
  inside the actual `zkite` PyPI package (so a fresh `pip install zkite` gets it too)
  is packaging work not done this session.
- **No `zkite run`** (plan1.md's CLI sketch names it: install + launch + logcat
  filtering as one command). `zkite build apk` prints the equivalent `adb`
  invocation instead of running it — a small, obvious follow-up, not attempted
  here to keep this session's scope to what §10 actually names for phase 2.
- **AAB module is `base`-only** — no dynamic feature modules, no per-density/locale
  splits beyond what phase 0 already deferred (resource qualifiers). Consistent
  with plan1.md's own scope, not a new gap.
- **Real Play Console upload untested** (§8) — everything short of that is checked.

## 10. Recommendation

Proceed as built. All three of plan1.md §9's phase-2-relevant risks that mattered in
practice — `pack`'s AAB gap, the 16KB page-size requirement, wheel availability —
were found to be exactly as sized as phase 0/1 predicted, and each has either a
working mitigation already in place (extras-splice, mechanical alignment check,
loud doctor warnings) or a clearly identified next step (upstream `Stored`-for-`.so`).
The two bugs that only showed up on-device (`_android_support`, `main.pyc`
detection) are both the same shape as phase 0/1's own device-only findings: cheap to
fix once found, invisible without actually installing and launching the thing. That
pattern held a third time — worth treating as a standing lesson for phase 3, not a
phase-2-specific one: a build that satisfies every host-side tool is not evidence it
runs, only evidence it's well-formed.

Before phase 3: get the `Stored`-for-`.so` fix upstreamed (closes the AAB timing
gap and phase 0's own long-standing note about it in one PR), and vendor host-native
pyc-compilers for macOS/Windows once those build environments are available —
the single largest piece of "phase 2 works everywhere we could test it, not
everywhere it needs to."
