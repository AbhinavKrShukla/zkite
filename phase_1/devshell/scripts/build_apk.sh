#!/usr/bin/env bash
# Build the Dev Shell APK end to end (../../PLAN.md M4). Same two-halves split as
# phase_0/spike2's build_apk.sh, extended: javac+d8 and the Rust cross-compile are the
# release-pipeline side; the final `zkite-pack` step — reused directly from
# phase_0/spike2_pack_apk, unmodified, since its --app/--stage/--out interface is
# already fully generic — is what a user's machine would run.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEVSHELL="$(dirname "$HERE")"
PHASE1="$(dirname "$DEVSHELL")"
PHASE0="$(dirname "$PHASE1")/phase_0"
SPIKE2="$PHASE0/spike2_pack_apk"
ARTIFACTS="$DEVSHELL/artifacts"
BUILD="$DEVSHELL/build"

: "${ANDROID_HOME:=$HOME/Android/Sdk}"
: "${BUILD_TOOLS_VERSION:=34.0.0}"
: "${ABI:=x86_64}"
: "${PLATFORM:=android-34}"
PY_VERSION=3.14

BUILD_TOOLS="$ANDROID_HOME/build-tools/$BUILD_TOOLS_VERSION"
ANDROID_JAR="$ANDROID_HOME/platforms/$PLATFORM/android.jar"

case "$ABI" in
  x86_64)    ARCH=x86_64 ;;
  arm64-v8a) ARCH=aarch64 ;;
  *) echo "unsupported ABI $ABI"; exit 1 ;;
esac

PY_PREFIX="$PHASE0/vendor/python-$ARCH/prefix"
STAGE="$BUILD/stage-$ABI"
OUT="$ARTIFACTS/zkite-devshell-$ABI.apk"

echo "=== 1/5  compile the dex shim (release-pipeline side: JDK + d8) ==="
rm -rf "$BUILD/classes"
mkdir -p "$BUILD/classes"
javac -source 8 -target 8 -nowarn \
      -bootclasspath "$BUILD_TOOLS/core-lambda-stubs.jar:$ANDROID_JAR" \
      -classpath "$ANDROID_JAR" \
      -d "$BUILD/classes" \
      "$DEVSHELL"/java/dev/zkite/devshell/*.java \
      "$HERE/../../../phase_1/android_shared/java/dev/zkite/runtime/"*.java
"$BUILD_TOOLS/d8" --release --min-api 26 --lib "$ANDROID_JAR" \
      --output "$BUILD" \
      $(find "$BUILD/classes" -name '*.class')
echo "    classes.dex: $(stat -c%s "$BUILD/classes.dex") bytes"

echo
echo "=== 2/5  build the python stdlib + zkite framework assets ==="
python3 "$SPIKE2/scripts/make_stdlib_zip.py" "$PY_PREFIX" "$BUILD/python-stdlib.zip"
python3 "$PHASE1/../scripts/zip_tree.py" "$PHASE1/zkite_py/src" "$BUILD/zkite-framework.zip"
echo "    zkite-framework.zip: $(stat -c%s "$BUILD/zkite-framework.zip") bytes"

echo
echo "=== 3/5  cross-compile the Rust cdylib ==="
# ALWAYS rebuild — do not reuse an existing .so just because the file is there.
#
# This guard used to be `if [ ! -f "$ARTIFACTS/$ABI/libzkitedevshell.so" ]`, which
# reuses a stale binary indefinitely: it tests for *existence*, never for whether the
# source changed. That is not hypothetical — it silently shipped an APK without a
# just-added feature during phase 3's §15.6 work, and it is worse here than in most
# places because `src/runtime.py` is `include_str!`'d into this .so, so editing the
# *Python* glue also requires a Rust rebuild and looks even less like it should.
# Rebuilding costs ~1s when nothing changed (cargo is incremental); the guard was a
# false economy paid for in device-debugging time.
"$HERE/build.sh" "$ARCH"

echo
echo "=== 4/5  stage the APK payload ==="
rm -rf "$STAGE"
mkdir -p "$STAGE/lib/$ABI" "$STAGE/assets"
cp "$BUILD/classes.dex"                     "$STAGE/classes.dex"
cp "$BUILD/python-stdlib.zip"               "$STAGE/assets/"
cp "$BUILD/zkite-framework.zip"              "$STAGE/assets/"
cp -r "$PHASE1/webruntime"                  "$STAGE/assets/webruntime"
rm -rf "$STAGE/assets/webruntime/test" "$STAGE/assets/webruntime/vendor/"*.LICENSE \
       "$STAGE/assets/webruntime/vendor/"*.VERSION
cp "$ARTIFACTS/$ABI/libzkitedevshell.so"     "$STAGE/lib/$ABI/"
cp "$PY_PREFIX/lib/libpython$PY_VERSION.so" "$STAGE/lib/$ABI/"
# Shared libraries some extension modules link against (_ssl, _hashlib, _sqlite3).
# Never staged before, so those modules could not have loaded even once the
# rename below makes them findable — see ../../phase_3/PLAN.md §11.6.
for dep in "$PY_PREFIX"/lib/lib*_python.so; do
  [ -f "$dep" ] || continue
  cp "$dep" "$STAGE/lib/$ABI/"
done

# `<mod>.cpython-314-<triple>.so` -> `libpymod_<mod>.so`, because Android's package
# installer only extracts `lib/<abi>/` entries whose basename matches `lib*.so` and
# silently drops every other name at install time. Measured on a real device: 3 of 70
# staged `.so` files survived installation, which is why no ZKite backend could
# `import math` from phase 1 until this was found. `runtime.py`'s
# `_ZKiteNativeExtensionFinder` maps the renamed files back to module names at import
# time. Full account: ../../phase_3/PLAN.md §11.6.
DYNLOAD="$PY_PREFIX/lib/python$PY_VERSION/lib-dynload"
if [ -d "$DYNLOAD" ]; then
  n=0
  for so in "$DYNLOAD"/*.so; do
    mod="$(basename "$so")"
    mod="${mod%%.*}"
    cp "$so" "$STAGE/lib/$ABI/libpymod_$mod.so"
    n=$((n + 1))
  done
  echo "    staged $n extension modules into lib/$ABI/ as libpymod_*.so"
fi
du -sh "$STAGE" | sed 's/^/    staged: /'

echo
echo "=== 5/5  pack: resources -> ARSC/AXML, zip, align, sign (no JDK, no SDK) ==="
cargo run --quiet --release --manifest-path "$SPIKE2/Cargo.toml" -- \
  --app "$DEVSHELL/app" \
  --stage "$STAGE" \
  --out "$OUT"

echo
echo "APK: $OUT"
