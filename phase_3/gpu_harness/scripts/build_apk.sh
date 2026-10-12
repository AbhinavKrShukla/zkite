#!/usr/bin/env bash
# Build the GPU harness APK end to end (../../PLAN.md §4/§5). Same shape as
# phase_1/devshell/scripts/build_apk.sh: javac+d8 and the Rust cross-compile are the
# release-pipeline side; the final `zkite-pack` step — phase_0/spike2_pack_apk, reused
# unmodified — is what a user's machine would run.
#
# §5 gave this harness the same CPython embedding devshell/appshell already have
# (`zkite_pybridge`), so step 3 now also builds and stages the same three Python
# assets `phase_2/appshell/scripts/build.sh` does (`python-stdlib.zip`/
# `zkite-framework.zip`/`app.zip`) plus `libpython3.14.so` and the stdlib's compiled
# extension modules — see `ZKiteActivity.java`'s unpacking code for where these land
# on-device.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GPU_HARNESS="$(dirname "$HERE")"
PHASE3="$(dirname "$GPU_HARNESS")"
PHASE0="$(dirname "$PHASE3")/phase_0"
PHASE1="$(dirname "$PHASE3")/phase_1"
SPIKE2="$PHASE0/spike2_pack_apk"
ARTIFACTS="$GPU_HARNESS/artifacts"
BUILD="$GPU_HARNESS/build"

: "${ANDROID_HOME:=$HOME/Android/Sdk}"
: "${BUILD_TOOLS_VERSION:=34.0.0}"
: "${ABI:=x86_64}"
: "${PLATFORM:=android-34}"
: "${ZKITE_APP_MAIN:=$GPU_HARNESS/demo_app/main.py}"
# ZKITE_DEVSHELL=1 builds the *generic Dev Shell* instead of a bundled-app harness
# (../../PLAN.md §15): no app.zip, plus a `zkite-devmode` marker asset that is the only
# thing telling ZKiteActivity to start the reload socket. `zkite dev --backend native`
# installs the result once and thereafter pushes only Python, exactly as plan1.md §6
# describes for the WebView Dev Shell.
: "${ZKITE_DEVSHELL:=}"
PY_VERSION=3.14

BUILD_TOOLS="$ANDROID_HOME/build-tools/$BUILD_TOOLS_VERSION"
ANDROID_JAR="$ANDROID_HOME/platforms/$PLATFORM/android.jar"

if [ -z "$ZKITE_DEVSHELL" ]; then
  [ -f "$ZKITE_APP_MAIN" ] || { echo "app main.py not found at $ZKITE_APP_MAIN — set ZKITE_APP_MAIN"; exit 1; }
fi

case "$ABI" in
  x86_64)    ARCH=x86_64;  NDK_TRIPLE=x86_64-linux-android ;;
  arm64-v8a) ARCH=aarch64; NDK_TRIPLE=aarch64-linux-android ;;
  *) echo "unsupported ABI $ABI"; exit 1 ;;
esac

PY_PREFIX="$PHASE0/vendor/python-$ARCH/prefix"
STAGE="$BUILD/stage-$ABI"
if [ -n "$ZKITE_DEVSHELL" ]; then
  OUT="$ARTIFACTS/zkite-native-devshell-$ABI.apk"
else
  OUT="$ARTIFACTS/zkite-gpu-harness-$ABI.apk"
fi

echo "=== 1/5  compile the dex (release-pipeline side: JDK + d8) ==="
# One source of truth for the javac/d8 classpath, shared with `zkite build apk
# --backend native` (which reads the artifacts/classes.dex this writes) — see
# build_dex.sh's own header for why this isn't inlined here any more.
"$HERE/build_dex.sh"

echo
echo "=== 2/5  build the python stdlib + zkite framework + app assets ==="
python3 "$SPIKE2/scripts/make_stdlib_zip.py" "$PY_PREFIX" "$BUILD/python-stdlib.zip"
python3 "$PHASE1/../scripts/zip_tree.py" "$PHASE1/zkite_py/src" "$BUILD/zkite-framework.zip"
rm -rf "$BUILD/app" "$BUILD/app.zip" && mkdir -p "$BUILD/app"
if [ -z "$ZKITE_DEVSHELL" ]; then
  cp "$ZKITE_APP_MAIN" "$BUILD/app/main.py"
  # Image assets sitting next to the fixture, staged with the *same* allowlist
  # `zkite build apk` uses (`ASSET_EXTENSIONS` in phase_1/cli/src/build_cmd.rs) — see
  # ../../PLAN.md §35. Without this a harness fixture could never exercise the file
  # form of `src` at all, only `data:` URIs, and the file form is the one a real app
  # uses.
  #
  # Every image in demo_app/ goes into every harness build. That is a few KB and it
  # keeps this script from needing to know which fixture wants which file; the
  # alternative (a per-fixture asset list) is a second thing to keep in sync.
  APP_SRC_DIR="$(dirname "$ZKITE_APP_MAIN")"
  shopt -s nullglob nocaseglob
  for asset in "$APP_SRC_DIR"/*.png "$APP_SRC_DIR"/*.jpg "$APP_SRC_DIR"/*.jpeg; do
    cp "$asset" "$BUILD/app/"
  done
  shopt -u nullglob nocaseglob
  python3 "$PHASE1/../scripts/zip_tree.py" "$BUILD/app" "$BUILD/app.zip"
  echo "    app.zip: $(unzip -l "$BUILD/app.zip" | tail -1)"
fi
echo "    python-stdlib.zip: $(stat -c%s "$BUILD/python-stdlib.zip") bytes"
echo "    zkite-framework.zip: $(stat -c%s "$BUILD/zkite-framework.zip") bytes"
if [ -z "$ZKITE_DEVSHELL" ]; then
  echo "    app.zip: $(stat -c%s "$BUILD/app.zip") bytes ($ZKITE_APP_MAIN)"
else
  echo "    app.zip: (none — dev shell; zkite dev pushes the app over the reload socket)"
fi

echo
echo "=== 3/5  cross-compile the Rust cdylib (if not already built for $ABI) ==="
# ALWAYS rebuild — an existence check is not a freshness check. The earlier version of
# this reused any .so that happened to be present, which ships a stale binary silently;
# `phase_1/devshell`'s identical guard did exactly that during §15.6 and cost a device
# debugging round. cargo is incremental, so this is ~1s when nothing changed. It also
# re-stages libc++_shared.so/libpython/fonts.zip, which a tree built before build.sh
# started staging them would otherwise be missing entirely — an `UnsatisfiedLinkError`
# at first launch.
"$HERE/build.sh" "$ABI"

echo
echo "=== 4/5  stage the APK payload ==="
rm -rf "$STAGE"
mkdir -p "$STAGE/lib/$ABI" "$STAGE/assets"
cp "$ARTIFACTS/classes.dex"                 "$STAGE/classes.dex"
cp "$BUILD/python-stdlib.zip"               "$STAGE/assets/"
cp "$BUILD/zkite-framework.zip"              "$STAGE/assets/"
if [ -z "$ZKITE_DEVSHELL" ]; then
  cp "$BUILD/app.zip"                       "$STAGE/assets/"
else
  # The one thing that makes this a Dev Shell rather than an app — see
  # ZKiteActivity.startReloadServerIfDevShell.
  echo "zkite dev shell" > "$STAGE/assets/zkite-devmode"
fi
# One zip of the six faces, unpacked to a directory on first launch — see
# ZKiteActivity's FONTS_ASSET and Native.boot's `fontDir` parameter. Built by
# build.sh into artifacts/, so this script and `zkite build apk --backend native`
# ship byte-identical fonts.
cp "$ARTIFACTS/fonts.zip"                   "$STAGE/assets/"
# libzkitegpuharness.so, libpython3.14.so and libc++_shared.so — all three now staged
# into artifacts/ by build.sh (see its own comments for why libc++_shared.so is not
# optional and why locating it is the NDK-having step's job, not a consumer's).
cp "$ARTIFACTS/$ABI/libzkitegpuharness.so"   "$STAGE/lib/$ABI/"
cp "$ARTIFACTS/$ABI/libpython$PY_VERSION.so" "$STAGE/lib/$ABI/"
cp "$ARTIFACTS/$ABI/libc++_shared.so"       "$STAGE/lib/$ABI/"
# Third-party shared libraries some extension modules link against (_ssl ->
# libssl_python.so/libcrypto_python.so, _sqlite3 -> libsqlite3_python.so).
# python.org's Android prefix already ships these under a `lib*_python.so` name
# specifically so Android's installer will extract them (see the extension-module
# note directly below); they were never staged at all before, so those three
# modules would have failed to load even once the rename below made them findable.
for dep in "$PY_PREFIX"/lib/lib*_python.so; do
  [ -f "$dep" ] || continue
  cp "$dep" "$STAGE/lib/$ABI/"
done

# Extension modules are renamed `<mod>.cpython-314-<triple>.so` ->
# `libpymod_<mod>.so` on the way into the APK. This is not cosmetic and not
# optional (../../PLAN.md §11.6):
#
#   Android's package installer extracts an APK's `lib/<abi>/` entries into the
#   app's nativeLibraryDir, but it only extracts entries whose basename matches
#   `lib*.so`. Every other name is silently skipped — no warning at build time,
#   no warning at install time, and the file is simply absent at runtime.
#
# Measured directly on the Moto G54: of the 70 `.so` files this script used to
# stage, exactly 3 survived installation — `libpython3.14.so`, `libc++_shared.so`
# and `libzkitegpuharness.so`, i.e. precisely the three already named `lib*.so`.
# All 67 extension modules were dropped, which is why `import math` had never
# worked on any ZKite backend since phase 1.
#
# The module name is recoverable from the staged name by construction (module
# names are `[A-Za-z0-9_]+`, so `libpymod_` + name + `.so` round-trips
# unambiguously); `runtime.py`'s `_ZKiteNativeExtensionFinder` is what turns it
# back into an importable module at runtime.
DYNLOAD="$PY_PREFIX/lib/python$PY_VERSION/lib-dynload"
if [ -d "$DYNLOAD" ]; then
  n=0
  for so in "$DYNLOAD"/*.so; do
    mod="$(basename "$so")"
    mod="${mod%%.*}"                       # math.cpython-314-...so -> math
    cp "$so" "$STAGE/lib/$ABI/libpymod_$mod.so"
    n=$((n + 1))
  done
  echo "    staged $n extension modules into lib/$ABI/ as libpymod_*.so"
fi
du -sh "$STAGE" | sed 's/^/    staged: /'

echo
echo "=== 5/5  pack: resources -> ARSC/AXML, zip, align, sign (no JDK, no SDK) ==="
PEM_ARGS=()
if [ -n "${ZKITE_TEST_PEM:-}" ]; then
  PEM_ARGS=(--pem "$ZKITE_TEST_PEM")
fi
cargo run --quiet --release --locked --manifest-path "$SPIKE2/Cargo.toml" -- \
  --app "$GPU_HARNESS/app" \
  --stage "$STAGE" \
  --out "$OUT" \
  "${PEM_ARGS[@]}"

echo
echo "APK: $OUT"
