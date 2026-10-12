# Spike 2 — A signed, installable APK from `google/pack`

**Criterion (plan1.md §10):** *Build a signed, installable APK end-to-end using
`google/pack` with a prebuilt dex + a native lib. No framework — just prove the
toolchain-free path produces something that runs.*

**Verdict: PASS.**

```
  16 KB page alignment: 69 native libraries OK
  wrote artifacts/zkite-phase0-x86_64.apk (7.5 MB) in 0.27s — resources compiled, zip aligned, signed v2+v3

  installing zkite-phase0-x86_64.apk … Success   (0.42s)
  launching dev.zkite.phase0/.ZKiteActivity  Status: ok  LaunchState: COLD  TotalTime: 855

  I ZKitePhase0: ZKITE_SPIKE2_RESULT ok=1 abi=x86_64 sdk=34
  I ZKitePhase0: ZKITE_SPIKE1_RESULT ok=1 platform=android stdout="hi\n" version=3.14.6 …
```

## Build and run

```bash
ABI=x86_64 scripts/build_apk.sh     # or ABI=arm64-v8a
scripts/run_on_device.sh            # install, launch, read the verdict out of logcat
cargo test --release                # structural checks on the finished APK
```

## What ran, and on which side of the line

plan1.md §2.1: *"Nothing compiles on the user's machine."* This spike is deliberately
split so that claim is checkable rather than asserted:

```
=== 1/4  compile the dex shim (release-pipeline side: JDK + d8) ===
=== 2/4  build the python stdlib asset ===
=== 3/4  stage the APK payload ===
=== 4/4  pack: resources -> ARSC/AXML, zip, align, sign (no JDK, no SDK) ===
```

Steps 1 and 2 are what *we* do once per framework release. Step 4 is what a user's
`zkite build apk` does, and it ran with **no JDK, no Android SDK, no `aapt2`, no
`zipalign`, no `apksigner`, no `android.jar`** — just `zkite-pack`, a Rust binary.

Google's tools were then used to *check* the result, never to produce it:

```
$ apksigner verify --verbose artifacts/zkite-phase0-x86_64.apk
Verifies
Verified using v2 scheme (APK Signature Scheme v2): true
Verified using v3 scheme (APK Signature Scheme v3): true
Number of signers: 1
Signer #1 certificate DN: CN=pack-sign
Signer #1 key algorithm: RSA, key size (bits): 2048

$ zipalign -c -v 4 artifacts/zkite-phase0-x86_64.apk
Verification succesful

$ aapt2 dump packagename artifacts/zkite-phase0-x86_64.apk
dev.zkite.phase0
```

`aapt2` parsing the manifest is a meaningful check on its own: it means `pack`'s AXML
encoding is byte-compatible with the tool it replaces, not merely acceptable to the
platform.

## The gap in `pack`, and how this sidesteps it

plan1.md §7 anticipates trouble and ranks the mitigations: *sidestep, contribute upstream,
fork (budget 3–4 weeks)*. Phase 0's answer is that **the sidestep is enough and no fork
was needed**, but the gap is not the one §7 expected.

### Not the problem: resource qualifiers

§7 flags "single-language `strings.xml` only, `drawable/` only (no `drawable-xhdpi`, no
`values-es/`)". For a framework that renders its own pixels, this is vestigial. The app
here ships one density-independent icon and one `strings.xml` and wants for nothing.
`read_res_dir` in `src/apk.rs` *rejects* a qualifier directory loudly rather than silently
dropping it, so the day it does matter, it says so.

### The actual problem: `pack` has no notion of a non-resource file

```rust
pub struct Package {
    pub android_manifest: Vec<u8>,
    pub resources: Vec<FileResource>,
}
```

That is the entire input. There is nowhere to put `classes.dex`, `lib/<abi>/*.so`, or
`assets/` — because `pack` targets Wear OS watch faces, which contain no code. This is not
a bug or an omission from the compatibility table; it is the project's scope.

`src/apk.rs` closes it in ~60 lines by reproducing `pack_api::compile_apk` and appending
verbatim zip entries before the zip is written and signed. Every piece of format knowledge
— AXML, ARSC, alignment, v2/v3 signing — still comes from `pack`.

**`vendor/pack` is a pristine clone at `7b60931`. Nothing in it is patched.**

### The upstream PR

One field:

```rust
pub struct Package {
    pub android_manifest: Vec<u8>,
    pub resources: Vec<FileResource>,
    pub extra_files: Vec<pack_zip::File>,   // <- deletes src/apk.rs
}
```

Small, self-contained, obviously correct, and useful to anyone building an APK with code
in it. §11 argues a merged PR is worth more as marketing than the code is as code; this
is that PR. Worth bundling with it:

- **A `Stored` path for `lib/**/*.so`.** `pack_zip::zip_apk` deflates everything except
  `resources.arsc`, so `android:extractNativeLibs="false"` — uncompressed, page-aligned
  libraries the platform maps straight out of the APK — is unreachable. Play prefers it.
  This spike therefore ships `extractNativeLibs="true"`, which installs and runs fine but
  is the older shape.
- **`pack-aab` requires `protoc` at build time** (`prost-build`). Building the *packager*
  needs a protobuf compiler even though running it does not. Harmless for a shipped wheel
  with a prebuilt CLI, but phase 2 promises AAB output, so it belongs on the list. This
  crate does not depend on `pack-api` for exactly that reason.

## Where the extension modules live, and why it is not obvious

CPython ships 67 `lib-dynload/*.so` extension modules. They cannot be unpacked into the
app's data directory and imported: **since Android 10, an app may only `dlopen` from its
own native library directory.**

So `build_apk.sh` stages them, name-flattened, into `lib/<abi>/` inside the APK, and
`ZKiteActivity.linkExtensionModules` writes the runtime path to that directory — knowable
only at runtime — into a `.pth` file that `site` picks up during interpreter startup.

This is a real constraint on the phase-2 bundler. §7's step 4 ("stage `libpython3.14.so` ·
`libzkite.so` · `stdlib.zip` · `classes.dex`") needs a fifth item, and any wheel containing
a native extension will need the same treatment.

## The dex shim

`java/dev/zkite/phase0/` is 6.4 KB of dex: an Activity and a one-method JNI class.

It extends `android.app.Activity`, **not `AppCompatActivity`**. plan1.md §3 is right that
the shipping shim needs AppCompat for IME, insets and the back gesture — but pulling in an
AndroidX AAR would have turned this spike into a test of dependency resolution instead of
a test of packaging. What has to be proven here is that *a dex we compiled once, at
release time, runs inside an APK that `pack` produced*. A plain Activity proves that.

**Nothing here says anything about IME.** That remains the §9 High risk it always was, and
§11's "do the boring hard things first" still stands.

## Tests

`cargo test --release` asserts, from the finished artifact rather than from the code that
produced it:

| Test | Checks |
|---|---|
| `contains_everything_an_apk_needs` | manifest, `resources.arsc`, `classes.dex`, the icon, both `.so`s, the stdlib asset |
| `manifest_and_arsc_are_binary_not_text` | ResChunk magic — `pack` *compiled* the XML, it did not copy it |
| `resources_arsc_is_stored_uncompressed` | the platform mmaps it; a deflated one is a runtime failure |
| `has_an_apk_signing_block` | `APK Sig Block 42` is present |
| `stored_entries_are_four_byte_aligned` | what `zipalign` exists to guarantee |
| `native_libraries_are_16kb_page_aligned` | plan1.md §9's 16 KB risk, caught here rather than at Play upload |
| `build_is_deterministic_for_a_fixed_key` | staged entries are sorted, so builds are reproducible |

They skip with a message rather than failing when no APK has been built, so `cargo test`
stays useful on a machine with no SDK.

## Signing keys

This spike uses `pack`'s random throwaway key (`CN=pack-sign`) and prints the warning it
comes with. plan1.md §7's keystore story — `rcgen` + `p12`, `zkite keygen` emitting both
PEM and PKCS#12, `zkite build --keystore` accepting what developers already have — is phase
2. `--pem` is wired up and takes a combined certificate + private key PEM.
