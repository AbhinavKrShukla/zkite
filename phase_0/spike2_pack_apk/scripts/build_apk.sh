#!/usr/bin/env bash
# Build the phase-0 APK end to end.
#
# Two halves, and the split is the whole point of plan1.md §2.1:
#
#   [release pipeline, once per framework release]  javac + d8  ->  classes.dex
#   [every build, on a user's machine]              zkite-pack   ->  signed APK
#
# The first half needs a JDK and d8 and runs here because we are *building the
# framework*. The second half is what a user would run, and it needs neither.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SPIKE="$(dirname "$HERE")"
PHASE0="$(dirname "$SPIKE")"
VENDOR="$PHASE0/vendor"
ARTIFACTS="$PHASE0/artifacts"
BUILD="$SPIKE/build"

: "${ANDROID_HOME:=$HOME/Android/Sdk}"
: "${BUILD_TOOLS_VERSION:=34.0.0}"
: "${ABI:=x86_64}"                 # x86_64 (emulator) or arm64-v8a (phone)
: "${PLATFORM:=android-34}"
PY_VERSION=3.14

BUILD_TOOLS="$ANDROID_HOME/build-tools/$BUILD_TOOLS_VERSION"
ANDROID_JAR="$ANDROID_HOME/platforms/$PLATFORM/android.jar"

case "$ABI" in
  x86_64)    ARCH=x86_64 ;;
  arm64-v8a) ARCH=aarch64 ;;
  *) echo "unsupported ABI $ABI"; exit 1 ;;
esac

PY_PREFIX="$VENDOR/python-$ARCH/prefix"
STAGE="$BUILD/stage-$ABI"
OUT="$ARTIFACTS/zkite-phase0-$ABI.apk"

echo "=== 1/4  compile the dex shim (release-pipeline side: JDK + d8) ==="
rm -rf "$BUILD/classes"
mkdir -p "$BUILD/classes"
javac -source 8 -target 8 -nowarn \
      -bootclasspath "$ANDROID_JAR" \
      -classpath "$ANDROID_JAR" \
      -d "$BUILD/classes" \
      "$SPIKE"/java/dev/zkite/phase0/*.java
"$BUILD_TOOLS/d8" --release --min-api 24 --lib "$ANDROID_JAR" \
      --output "$BUILD" \
      $(find "$BUILD/classes" -name '*.class')
echo "    classes.dex: $(stat -c%s "$BUILD/classes.dex") bytes"

echo
echo "=== 2/4  build the python stdlib asset ==="
python3 "$HERE/make_stdlib_zip.py" "$PY_PREFIX" "$BUILD/python-stdlib.zip"

echo
echo "=== 3/4  stage the APK payload ==="
rm -rf "$STAGE"
mkdir -p "$STAGE/lib/$ABI" "$STAGE/assets"
cp "$BUILD/classes.dex"                        "$STAGE/classes.dex"
cp "$BUILD/python-stdlib.zip"                  "$STAGE/assets/"
cp "$ARTIFACTS/$ABI/libzkitespike1.so"          "$STAGE/lib/$ABI/"
cp "$PY_PREFIX/lib/libpython$PY_VERSION.so"    "$STAGE/lib/$ABI/"
# CPython's extension modules. They must live in lib/<abi>/ rather than beside the
# stdlib: since Android 10, an app may only dlopen from its own native library
# directory, so anything unpacked into filesDir/ would be unloadable. The names are
# flattened because that directory has no subdirectories.
DYNLOAD="$PY_PREFIX/lib/python$PY_VERSION/lib-dynload"
if [ -d "$DYNLOAD" ]; then
  n=0
  for so in "$DYNLOAD"/*.so; do
    cp "$so" "$STAGE/lib/$ABI/$(basename "$so")"
    n=$((n + 1))
  done
  echo "    staged $n extension modules into lib/$ABI/"
fi
du -sh "$STAGE" | sed 's/^/    staged: /'

echo
echo "=== 4/4  pack: resources -> ARSC/AXML, zip, align, sign (no JDK, no SDK) ==="
cargo run --quiet --release --manifest-path "$SPIKE/Cargo.toml" -- \
  --app "$SPIKE/app" \
  --stage "$STAGE" \
  --out "$OUT"

echo
echo "APK: $OUT"
