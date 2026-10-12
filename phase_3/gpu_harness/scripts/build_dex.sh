#!/usr/bin/env bash
# Compiles the native shell's classes.dex — release-pipeline only (javac + d8),
# exactly like phase_2/appshell/scripts/build_dex.sh. `zkite build apk --backend
# native` reads the resulting artifacts/classes.dex verbatim; it never invokes
# javac/d8 itself (see ../../../phase_2/PLAN.md §1.1 — the same "compiles once, in
# our CI" bucket as the NDK).
#
# This is the authority for *how* the dex is built; `build_apk.sh` (the harness's own
# APK, which also bakes a ZKITE_APP_MAIN app in) calls this script rather than keeping
# a second copy of the javac/d8 invocation. Two copies of a classpath this specific
# would drift, and the failure mode is a NoClassDefFoundError on a device rather than
# anything a build catches — see the androidx-core note below, which cost exactly that
# once already.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GPU_HARNESS="$(dirname "$HERE")"
ARTIFACTS="$GPU_HARNESS/artifacts"
BUILD="$GPU_HARNESS/build"

: "${ANDROID_HOME:=$HOME/Android/Sdk}"
: "${BUILD_TOOLS_VERSION:=34.0.0}"
: "${PLATFORM:=android-34}"

BUILD_TOOLS="$ANDROID_HOME/build-tools/$BUILD_TOOLS_VERSION"
ANDROID_JAR="$ANDROID_HOME/platforms/$PLATFORM/android.jar"

# ../../PLAN.md §7.1 (IME): ZKiteSurfaceView constructs vendor/gametextinput's
# InputConnection/Settings/State and implements its Listener interface, so javac needs
# both jars on its classpath to type-check at all.
#
# androidx-core turned out *not* to be compile-only, despite fetch_androidx_core.sh's
# original reasoning (androidx.core.graphics.Insets/WindowInsetsCompat only showing up
# in an interface method this code never calls) — found on a real device, not by
# inspection: `gametextinput.InputConnection`'s own *constructor* calls into
# `androidx.core.view.WindowCompat`, which is exercised on every single focus, not just
# the deferred insets path. Skipping it dexed clean and then crashed on-device with
# `NoClassDefFoundError: Landroidx/core/view/WindowCompat` the moment a real focus
# attempt reached `new InputConnection(...)`. So: on javac's classpath *and* dexed in
# below.
GAMETEXTINPUT_JAR="$GPU_HARNESS/vendor/gametextinput/classes.jar"
ANDROIDX_CORE_JAR="$GPU_HARNESS/vendor/androidx-core/classes.jar"
[ -f "$GAMETEXTINPUT_JAR" ] || { echo "missing $GAMETEXTINPUT_JAR (vendored, should be in git)"; exit 1; }
[ -f "$ANDROIDX_CORE_JAR" ] || { echo "missing $ANDROIDX_CORE_JAR — run $HERE/fetch_androidx_core.sh"; exit 1; }

rm -rf "$BUILD/classes"
mkdir -p "$BUILD/classes" "$ARTIFACTS"
javac -source 8 -target 8 -nowarn \
      -bootclasspath "$ANDROID_JAR" \
      -classpath "$ANDROID_JAR:$GAMETEXTINPUT_JAR:$ANDROIDX_CORE_JAR" \
      -d "$BUILD/classes" \
      "$GPU_HARNESS"/java/dev/zkite/gpuharness/*.java \
      "$GPU_HARNESS"/java/androidx/core/*.java \
      "$HERE/../../../phase_1/android_shared/java/dev/zkite/runtime/"*.java
# gametextinput's own classes (InputConnection/GameTextInput/Listener/Settings/State)
# *and* androidx-core (see above) are dexed in alongside ours — both are real runtime
# dependencies. d8 accepts a .jar input directly and dexes every class inside it.
"$BUILD_TOOLS/d8" --release --min-api 26 --lib "$ANDROID_JAR" \
      --output "$BUILD" \
      $(find "$BUILD/classes" -name '*.class') \
      "$GAMETEXTINPUT_JAR" \
      "$ANDROIDX_CORE_JAR"
cp "$BUILD/classes.dex" "$ARTIFACTS/classes.dex"
echo "classes.dex: $(stat -c%s "$ARTIFACTS/classes.dex") bytes -> $ARTIFACTS/classes.dex"

# Which build produced this, recorded beside it — ../../PLAN.md §34.8. §35 caught the
# wrong classes.dex only by byte-comparing it against a freshly built one by hand;
# this is that comparison, kept.
#
# Both vendored jars are inputs, not just our java/: d8 dexes them straight in (see
# above), so a re-fetched androidx-core changes the artifact without touching a line
# of this repository's own source.
python3 "$(dirname "$GPU_HARNESS")/../phase_1/cli/scripts/artifact_provenance.py" record \
  --artifacts "$ARTIFACTS" --shell native --group dex \
  --extra phase_1/android_shared/java \
  --extra phase_3/gpu_harness/java \
  --extra phase_3/gpu_harness/vendor/gametextinput/classes.jar \
  --extra phase_3/gpu_harness/vendor/androidx-core/classes.jar \
  --extra phase_3/gpu_harness/scripts/build_dex.sh \
  --built-by phase_3/gpu_harness/scripts/build_dex.sh \
  --rebuild 'phase_3/gpu_harness/scripts/build_dex.sh' \
  --file classes.dex
