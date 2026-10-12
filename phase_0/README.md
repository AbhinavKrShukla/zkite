# Phase 0 — De-risk

This directory is the implementation of **plan1.md §10, Phase 0**. Its only job is to
answer three pass/fail questions before any framework code gets written:

> 1. Embed official CPython 3.14 Android in a Rust `cdylib`; run `print("hi")` from
>    `android_main`; confirm it works on arm64 + x86_64 emulator.
> 2. Build a signed, installable APK end-to-end using `google/pack` with a prebuilt dex +
>    a native lib. No framework — just prove the toolchain-free path produces something
>    that runs.
> 3. `twcore` prototype: parse 40 common utilities into a `taffy::Style` + paint struct.
>
> **If any spike fails, the architecture changes.**

Nothing here is framework code, and nothing here is meant to survive into phase 1 except
as a reference. Each spike is a standalone Rust crate with its own `Cargo.lock`.

---

## Results

| # | Spike | Verdict | Evidence |
|---|---|---|---|
| 1 | CPython 3.14.6 embedded in a Rust `cdylib` | **PASS on x86_64 and arm64** | `print("hi")` returns `"hi\n"`, `sys.platform == "android"`, from `adb shell` and from inside a real app process, on both ABIs. arm64 was first blocked from execution by the emulator (see [arm64](#arm64-the-emulator-cant-run-it-a-real-phone-can)) and is now confirmed on a real device — see [real-hardware confirmation](#real-hardware-confirmation). |
| 2 | Signed installable APK via `google/pack` | **PASS** | 7.5 MB APK built in **0.27 s**, installs in 0.42 s, launches in 0.86 s on the emulator; installs in 2.86 s and launches in 0.82 s on a real arm64 phone. Independently verified by Google's own `apksigner` (v2 + v3 **Verified**), `zipalign -c` and `aapt2`. |
| 3 | `twcore`: Tailwind → `taffy::Style` + paint | **PASS** | **60** utility families (target: 40), 52 tests, real layouts computed through taffy, **19 ns** per cached resolve. |

All three spikes meet their criteria outright, on both the emulator and — for spikes 1 and
2, where it matters — a real arm64 phone.

**Nothing found here changes the architecture, so per §10 phase 1 is unblocked.** The arm64
caveat below is no longer an open item; it's kept in the doc because the *reason* it was
briefly open (the emulator can't run arm64 on an x86_64 host) is still true and still
shapes how CI has to work.

### Measurements

| | Measured | plan1.md target |
|---|---|---|
| APK build (resources → ARSC/AXML → zip → align → sign v2+v3) | **0.27 s** | §7: 3–8 s |
| `adb install` | 0.42 s | — |
| Cold launch to first frame | 0.86 s | §9: interactive < 500 ms |
| CPython init + `print("hi")` in-app | **~76 ms** | §9: init 100–200 ms |
| Stdlib unpack (first run only, 577 files) | 165–300 ms | — |
| APK size, one ABI, pruned stdlib | **7.5 MB** | §7: 20–26 MB |
| `classes.dex` | 6.4 KB | §3: ~40 KB |
| `twcore` cached resolve | 19 ns | — |

The APK is well under the §7 budget because the size table there assumes a *loose*
stdlib; shipping it as one deflated zip asset takes 10.3 MB of `.py` down to 2.7 MB, and
the extension modules compress hard too. The launch number is a cold start on a
software-rendered emulator and should be read as an upper bound.

### Real-hardware confirmation

Everything above was measured on the x86_64 emulator. The one thing phase 0 originally
could *not* do — run any of this on arm64, because the emulator refuses to run an
arm64 system image on an x86_64 host (see below) — has since been done on a real device:
a Moto G54 5G, arm64-v8a, Android 15 (API 35).

| | Emulator (x86_64) | Real device (arm64-v8a) |
|---|---|---|
| `adb install` | 0.42 s | 2.86 s |
| Cold launch to first frame | 0.86 s | 0.82 s |
| Stdlib unpack (first run, 577 files) | 165–300 ms | 368 ms |

Both on-device checks pass on the phone: the standalone probe (`ZKITE_SPIKE1_RESULT
ok=1`, same `print("hi")`/`sys.platform` result as x86_64) and the packed APK, installed
and launched cold, which unpacks the stdlib and boots CPython from inside the real app
process (`ZKITE_SPIKE2_RESULT ok=1`, then `ZKITE_SPIKE1_RESULT ok=1` again, this time from
`ZKiteActivity` rather than a shell probe). The slower install is expected — it's a debug
`adb install` of a locally-signed testing-key APK, not a Play-installed release, and
comparing it to the emulator's number isn't apples to apples. The launch and unpack
numbers, on real silicon, land in the same range as the emulator's — no arm64-specific
regression showed up.

This was a manual, one-off run against a single physical device, not yet a repeatable
step in `run_all.sh` or CI — see the CI recommendation below, which this result makes
easier to justify rather than less necessary.

---

## Layout

```
phase_0/
├── run_all.sh                  the gate: runs every check, prints a pass/fail table
├── scripts/fetch_python.sh     downloads the official python.org Android prefixes + pack
├── spike1_cpython_embed/       CPython in a Rust cdylib
├── spike2_pack_apk/            APK packaging on top of google/pack
├── spike3_twcore/              Tailwind class strings -> taffy::Style
├── vendor/                     python-{aarch64,x86_64}/, pack/  (downloads, not in git)
└── artifacts/                  built .so, probe binaries, .apk
```

Each spike has its own README with the detail.

## Reproducing

```bash
./scripts/fetch_python.sh     # ~44 MB of official python.org tarballs + a git clone
./run_all.sh                  # host-only: spike 3, cross-compiles, APK build + structure
./run_all.sh --device         # the above plus the on-device checks (needs adb + a device)
```

Prerequisites: a Rust toolchain with the `aarch64-linux-android` and
`x86_64-linux-android` targets, NDK r27+, and — for the dex only — a JDK and
`build-tools`. That last pair is the *release pipeline*, not the user's machine; see
[The toolchain-free claim](#the-toolchain-free-claim-what-phase-0-actually-proves).

Verified on Linux x86_64 with NDK 27.3.13750724, build-tools 34.0.0, an
`android-34 google_apis x86_64` emulator, and Rust 1.97.1.

---

## What each spike found

### Spike 1 — the PyO3 risk was real, and cheap to route around

plan1.md §9 rates "PyO3 embedding on Android is not officially supported" as **High**, and
prescribes: *"Spike this in week 1 — if it resists, drop PyO3 and use raw `pyo3-ffi`
against the C API, which is more code but has no build-script magic to fight."*

That is exactly what happened, and the escape hatch is cheaper than the plan assumes. The
whole cross-compile configuration is a nine-line `PYO3_CONFIG_FILE` written by
`spike1_cpython_embed/scripts/build.sh`; with it, `pyo3-ffi` never tries to run a host
interpreter to introspect the target. Both ABIs build in under a second.

**The finding that matters is not about PyO3 at all.** It is this:

> Setting `PYTHONHOME` does not locate the stdlib for an embedded interpreter, and a
> standalone binary will not tell you.

An isolated `PyConfig` forces `use_environment = 0` inside `PyConfig_Read`, so the
environment variable is discarded no matter what you assign to `config.use_environment`.
The standalone probe passed anyway — because CPython falls back to searching upward from
`argv[0]` for a `lib/python3.14/os.py` landmark, and `/data/local/tmp/zkite-phase0/`
happened to have one. Inside an app process `argv[0]` is the zygote, there is no landmark,
and startup dies with `Failed to import encodings module`.

The fix is to set `config.home` explicitly, which is the supported replacement for the
`Py_SetPythonHome` that 3.13 removed. This is written up at length in
`spike1_cpython_embed/src/lib.rs` because it is precisely the class of bug that passes
every host test and fails on every phone.

It also validates the decision to run spike 1 two ways — standalone *and* in-app. Had the
spike only had the `adb shell` probe, it would have reported PASS and phase 1 would have
inherited a latent failure.

### Spike 2 — `pack` works; the gap is smaller than the plan feared

plan1.md §7 rates `google/pack` maturity as **High** risk and budgets *"3–4 weeks"* for a
fork if resource qualifiers turn out to be unavoidable. Phase 0's answer:

**The qualifier gap is a non-issue, exactly as §7's "sidestep" mitigation predicted.** The
manifest here uses a single density-independent icon and one `strings.xml`, and nothing
is missing. A framework that owns its own rendering does not need `drawable-xhdpi/`.

**The real gap is different and much smaller: `pack` has no concept of a file that is not
a resource.** `Package` is `{ android_manifest, resources }` — there is no way to put
`classes.dex`, `lib/<abi>/*.so`, or `assets/` into the APK, because `pack` was written for
Wear OS watch faces, which contain no code. That is why the compatibility table does not
mention it: it is not a bug, it is the scope.

`spike2_pack_apk/src/apk.rs` closes it in about 60 lines, and every byte of format
knowledge still comes from `pack`. Upstream, this is **one field**:

```rust
pub struct Package {
    pub android_manifest: Vec<u8>,
    pub resources: Vec<FileResource>,
    pub extra_files: Vec<pack_zip::File>,   // <- this
}
```

That is a genuinely small, self-contained PR, and §11's "contribute rather than fork
immediately" is the right call. **No fork was needed.** `vendor/pack` is a pristine clone
at `7b60931`; everything ZKite adds lives in ZKite's own crate.

The output was checked against the tools `pack` is replacing, which is the only
verification that really counts:

```
$ apksigner verify --verbose artifacts/zkite-phase0-x86_64.apk
Verifies
Verified using v2 scheme (APK Signature Scheme v2): true
Verified using v3 scheme (APK Signature Scheme v3): true
Signer #1 key algorithm: RSA, key size: 2048

$ zipalign -c -v 4 artifacts/zkite-phase0-x86_64.apk
Verification succesful

$ aapt2 dump packagename artifacts/zkite-phase0-x86_64.apk
dev.zkite.phase0
```

Two further gaps found, both for phase 2 rather than now:

- **`pack` deflates every entry except `resources.arsc`**, so `extractNativeLibs="false"`
  — uncompressed, page-aligned libs the platform maps straight out of the APK — is not
  reachable today. Play prefers it. This is the second small upstream PR: a
  `CompressionMethod::Stored` path for `lib/**/*.so`.
- **`pack-aab`'s build script requires `protoc`**, so building the *packager* needs a
  protobuf compiler even though running it does not. Irrelevant for a shipped wheel with
  a prebuilt CLI, but worth knowing before phase 2 promises AAB output.

### Spike 2's other finding — Android 10 changed where extension modules can live

CPython's `lib-dynload/*.so` cannot be unpacked into the app's data directory and
imported: since Android 10 an app may only `dlopen` from its own native library
directory. The 67 extension modules therefore ship in `lib/<abi>/` inside the APK, and
the runtime path to them — which is only knowable at runtime — is written into a `.pth`
file that `site` picks up during interpreter startup
(`ZKiteActivity.linkExtensionModules`).

This directly shapes step 4 of the plan's §7 bundler pipeline: "stage `libpython3.14.so`
· `libzkite.so` · `stdlib.zip` · `classes.dex`" needs a fifth item, and wheels containing
native extensions will need the same flattening treatment.

### Spike 3 — Tailwind semantics land on `taffy::Style` with no impedance mismatch

The pass criterion was 40 utilities; `twcore` resolves **60 families**, including the
arbitrary values and the variant set §4 calls out (`hover:` `active:` `focus:`
`disabled:` `dark:` `sm:`…`2xl:` `rtl:` `landscape:`).

Three things are worth reporting beyond the count:

1. **The output is a real `taffy::Style`, not an intermediate form.** `tests/layout.rs`
   builds the counter app from §8 out of class strings, runs taffy over it, and asserts
   pixel positions — `justify-center` really does put the text at y=324 on an 800dp
   screen, `gap-6` really is 24dp, `grid-cols-3` really splits 332dp into 100/100/100
   with 16dp gutters.

2. **Variant resolution is a bitmask, as §4 specifies.** A class string is parsed once
   into `Vec<(VariantSet, Decl)>` sorted by emission rank; resolving for a node state
   tests bits. 10,000 resolves of the same string parse it exactly once
   (`class_string_is_parsed_exactly_once`), and a cached resolve costs **19 ns**.

3. **The palette is not hand-typed.** `src/palette.rs` is generated from
   `tailwindcss@3.4.17`'s own `src/public/colors.js` — 22 hues × 11 shades — and
   `tests/palette.rs` asserts every one of the 242 entries is reachable through a class
   string. Getting `indigo-500` wrong by one hex digit is the kind of thing nobody
   notices for six months.

Emission order was the one subtle piece. Tailwind's precedence comes from CSS source
order, not from the order of classes in the attribute, so `active:bg-indigo-600
bg-indigo-500` and `bg-indigo-500 active:bg-indigo-600` must produce the same result when
pressed. And because media blocks are emitted after the base layer, `md:` beats `hover:`.
Both are asserted.

---

## The toolchain-free claim: what phase 0 actually proves

plan1.md's central promise is *"the user's machine installs nothing but a wheel"*. Phase 0
splits cleanly along that line, and it is worth being precise about which side each tool
falls on:

| Tool | Used by | Runs on |
|---|---|---|
| `javac`, `d8` | building `classes.dex` | **our release pipeline, once per release** |
| Rust + NDK | cross-compiling `libzkitespike1.so` | **our release pipeline, once per release** |
| `zkite-pack` (this spike) | building the APK | **the user's machine** |
| `adb` | install + run | the user's machine (the one allowed external tool) |

The APK build step — resource compilation, AXML/ARSC encoding, zipping, 4-byte alignment,
v2+v3 signing — used **no JDK, no Android SDK, no `aapt2`, no `zipalign`, no `apksigner`,
and no `android.jar`**. That is the claim, and it holds.

The Android tools shown above were invoked *only to check the result*, never to produce it.

---

## arm64: the emulator can't run it, a real phone can

The spike-1 criterion says "arm64 + x86_64 emulator". Only the second half was achievable
*on the emulator*, and the reason is not time or effort:

```
$ emulator -avd zkite_phase0_arm64 -no-window
INFO  | Android emulator version 37.2.2.0 (build_id 15937705)
INFO  | Found systemPath .../system-images/android-34/google_apis/arm64-v8a/
FATAL | Avd's CPU Architecture 'arm64' is not supported by the QEMU2 emulator on
        x86_64 host. System image must match the host architecture.
```

The `arm64-v8a-34` system image downloads and unpacks fine — the emulator simply refuses
to run it. Current Android emulator builds have dropped cross-architecture emulation;
an arm64 AVD needs an arm64 host. (`qemu-user` is not a way around it either: Android
binaries need bionic and its linker, not glibc.)

So the plan's own de-risk step cannot be completed on a commodity x86_64 dev box, which
is worth knowing now rather than in phase 3:

- **ZKite's CI needs arm64 runners**, not just an emulator matrix. §9's mitigation for
  Vello Hybrid already says "ship a device compatibility matrix from real CI-on-device
  runs" — that requirement starts at phase 0, not phase 3.
- **The `arm64-v8a-ps16k-*` images exist** (16 KB page size, API 35+) and would exercise
  §9's 16 KB risk directly. Same host constraint applies.

What's established for arm64 no longer stops at static checks. It cross-compiles against
the official arm64 prefix, links `libpython3.14.so`/`liblog.so`/`libc.so`, exports
`android_main` and the JNI entry, passes the 16 KB page-alignment assertion — and now
**runs**, standalone and inside a real app process, on a real arm64-v8a phone (Moto G54
5G, Android 15). See [real-hardware confirmation](#real-hardware-confirmation) above.

That resolves the correctness question. It does **not** resolve the CI question: an
x86_64 dev box still cannot run an arm64 AVD, so the only way anyone other than someone
with a physical phone on their desk can verify this build is real arm64 CI hardware, not
a bigger emulator matrix. One manual device run is evidence the architecture works; it
is not a substitute for that build being checked on every commit.

## Known gaps

Honest list of what else this phase 0 does not establish.

- **The dex shim is `android.app.Activity`, not `AppCompatActivity`.** §3 is right that
  the shipping shim needs AppCompat for IME and insets. Using it here would have meant
  resolving an AndroidX AAR, which would test the dependency resolver rather than the
  packaging path. **IME is still the §9 High risk it always was — phase 0 says nothing
  about it**, and §11's "do the boring hard things first" still applies.

  Follow-up research after phase 0 confirms this isn't a hypothetical risk. Every other
  framework that renders text itself instead of using `EditText` has paid for it in
  production: Flutter's `InputConnection` bridge has carried multi-year-open,
  keyboard-vendor-specific bugs since 2018 — composing-region duplication, dropped text,
  `endBatchEdit on inactive InputConnection` freezes — clustered almost entirely on
  **Samsung Keyboard**, which the AOSP/emulator keyboard cannot reproduce. Even Jetpack
  Compose's `BasicTextField`, built by the platform team with internal access to the IME
  stack, shipped comparable `InputConnection` state-sync bugs. This is the same gap phase
  0 already flagged for arm64 — no real-device signal — showing up a second time, and it
  won't surface on an emulator running the default keyboard.

  Beyond raw `InputConnection` correctness, a Vello-rendered text field has no `View`
  behind it, so several things that come free with a normal Android widget have to be
  hand-built: IME-aware surface resize (`WindowInsetsAnimation`, since a `SurfaceView`
  doesn't auto-pan like a `View` under `adjustResize`), selection handles and the
  floating copy/paste toolbar (normally OS-drawn from `View` bounds — needs accurate
  `CursorAnchorInfo` reporting or a self-drawn overlay), Autofill (walks a `ViewStructure`
  tree that a single opaque `SurfaceView` is invisible to, without a virtual view
  hierarchy), and spell-check underlines (a `SuggestionSpan` that has to be painted
  through Parley instead of the OS). Autofill and accessibility both reduce to the same
  missing piece — a virtual semantics tree (node → screen rect, role, text) — so that's
  worth building once for both rather than as two bolt-ons.

  **Mitigation for phase 3:** don't hand-roll `android.view.inputmethod.InputConnection`.
  Google's `androidx.games:games-text-input` (GameTextInput) is standalone-usable (not
  GameActivity-only — it attaches to any custom `View` via `onCreateInputConnection`,
  which matches the dex shim's shape) and is the same code base absorbing keyboard-vendor
  bug reports across every shipped Android game that needs text input. It explicitly
  isn't a full-text-editor solution, so selection UI, Autofill, insets, and a11y-sync
  still have to be built regardless — but it removes the single largest source of
  long-tail vendor bugs from ZKite's own surface area. Its native half is packaged for
  Gradle/CMake/Prefab, which ZKite's Gradle-free pipeline doesn't have; the C++ file is
  small enough to fold into the existing `cc`-crate/NDK cross-compile in `build.rs`
  instead, the same way spike 2 avoided Gradle for everything else.

  **Two concrete planning changes fall out of this:** split §9's 4-week IME budget into
  sub-milestones (`InputConnection`/composing → insets/resize → selection UI/toolbar →
  Autofill + a11y-sync) so the highest-risk piece — vendor keyboard correctness — gets
  tested first, in month 2 as §11 argues; and extend the arm64-CI recommendation into a
  real keyboard matrix (Gboard, Samsung Keyboard, SwiftKey, MIUI), since that's where the
  historical bugs concentrate and an emulator can't supply it either.
- **`.py` ships uncompiled.** Step 2 of the §7 pipeline (`.py` → `.pyc`) is not
  implemented; the stdlib asset is source. This costs some startup time and some size.
- **Stdlib pruning is a static deny-list**, not the `modulegraph`-style reachability
  analysis §7 describes. It drops 45.8 MB (mostly CPython's own test suite) and prints
  what it dropped, which is enough to answer "does a pruned stdlib still boot" — yes.
- **No AAB.** Spike 2's criterion is an APK. AAB is phase 2, and see the `protoc` note.
- **Most measurements are still emulator-relative.** Spikes 1 and 2's pass/fail was
  confirmed on a real arm64 phone (see [real-hardware confirmation](#real-hardware-confirmation)),
  but that was a manual one-off run, not a repeatable benchmark — the `twcore` numbers,
  and any timing not called out in that section, are still emulator-only.
- **Taffy, and only taffy.** Parley, Vello and AccessKit are untouched — correctly, they
  are phase 3.

## Recommendation

Proceed to phase 1 as written. Nothing found here changes the architecture.

Two items are worth pulling forward out of what phase 0 learned:

1. **Send the `extra_files` PR to `google/pack` now**, before phase 2 depends on it. It is
   small, it is obviously correct, and §11 is right that a merged PR is worth more as
   marketing than the code is as code. Bundle the `Stored`-for-`.so` change with it.
2. **Get arm64 into CI before phase 1, not before phase 3.** A manual run on one physical
   phone now confirms the architecture works on arm64 (see
   [real-hardware confirmation](#real-hardware-confirmation)) — but it was one developer,
   one phone, one point in time. Almost every number in this document is still from the
   emulator, the emulator still cannot supply arm64 on an x86_64 host, and the whole
   premise is that users install a wheel and it works on their phone, unattended, on
   every release. This is a hardware/CI procurement item, so it has a lead time; start it
   now.
