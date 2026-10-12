#!/usr/bin/env bash
# Compiles the App Shell's classes.dex — release-pipeline only (javac + d8), exactly
# like phase_0/spike2's and phase_1/devshell's dex builds. `zkite build apk` reads the
# resulting artifacts/classes.dex verbatim; it never invokes javac/d8 itself (see
# ../../PLAN.md §1.1/§1.5 — the same "compiles once, in our CI" bucket as the NDK).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APPSHELL="$(dirname "$HERE")"
ARTIFACTS="$APPSHELL/artifacts"
BUILD="$APPSHELL/build"

: "${ANDROID_HOME:=$HOME/Android/Sdk}"
: "${BUILD_TOOLS_VERSION:=34.0.0}"
: "${PLATFORM:=android-34}"

BUILD_TOOLS="$ANDROID_HOME/build-tools/$BUILD_TOOLS_VERSION"
ANDROID_JAR="$ANDROID_HOME/platforms/$PLATFORM/android.jar"

rm -rf "$BUILD/classes"
mkdir -p "$BUILD/classes" "$ARTIFACTS"
javac -source 8 -target 8 -nowarn \
      -bootclasspath "$ANDROID_JAR" \
      -classpath "$ANDROID_JAR" \
      -d "$BUILD/classes" \
      "$APPSHELL"/java/dev/zkite/appshell/*.java \
      "$HERE/../../../phase_1/android_shared/java/dev/zkite/runtime/"*.java
"$BUILD_TOOLS/d8" --release --min-api 26 --lib "$ANDROID_JAR" \
      --output "$BUILD" \
      $(find "$BUILD/classes" -name '*.class')
cp "$BUILD/classes.dex" "$ARTIFACTS/classes.dex"
echo "classes.dex: $(stat -c%s "$ARTIFACTS/classes.dex") bytes -> $ARTIFACTS/classes.dex"

# Which build produced this, recorded beside it — ../../phase_3/PLAN.md §34.8.
python3 "$(dirname "$APPSHELL")/../phase_1/cli/scripts/artifact_provenance.py" record \
  --artifacts "$ARTIFACTS" --shell webview --group dex \
  --extra phase_1/android_shared/java \
  --extra phase_2/appshell/java \
  --extra phase_2/appshell/scripts/build_dex.sh \
  --built-by phase_2/appshell/scripts/build_dex.sh \
  --rebuild 'phase_2/appshell/scripts/build_dex.sh' \
  --file classes.dex
