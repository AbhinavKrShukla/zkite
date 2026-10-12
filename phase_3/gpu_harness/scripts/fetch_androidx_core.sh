#!/usr/bin/env bash
# Fetches androidx.core:core's classes.jar — needed on javac's compile classpath
# because vendor/gametextinput's `Listener` interface (../vendor/gametextinput/
# classes.jar, ../../PLAN.md §5) declares
# `onImeInsetsChanged(androidx.core.graphics.Insets)`, and `ZKiteSurfaceView`
# implements that interface.
#
# *Also* dexed into the built APK by build_apk.sh (not compile-only, despite this
# script's original assumption) — found on a real device, not by inspection:
# `gametextinput.InputConnection`'s own constructor calls into
# `androidx.core.view.WindowCompat` unconditionally, so it's a real runtime
# dependency of every focus attempt, not just the (still-unused) insets callback.
# See build_apk.sh's own comment on the `NoClassDefFoundError` this produced before
# that was fixed.
#
# Not committed to git regardless (unlike vendor/gametextinput, which is small and
# vendored outright) — 1.2MB/874 classes is closer to phase_2/vendor/protoc's
# "fetch it, don't commit it" bucket; see the root .gitignore's
# `/phase_3/gpu_harness/vendor/androidx-core/` entry.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$(dirname "$HERE")/vendor/androidx-core"
VERSION="1.13.1"
URL="https://dl.google.com/android/maven2/androidx/core/core/$VERSION/core-$VERSION.aar"

mkdir -p "$OUT"
if [ -f "$OUT/classes.jar" ]; then
  echo "already have $OUT/classes.jar"
  exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
echo "fetching $URL"
curl -sL --fail -o "$TMP/core.aar" "$URL"
unzip -p "$TMP/core.aar" classes.jar > "$OUT/classes.jar"
echo "androidx.core:core:$VERSION classes.jar (compile-classpath only)" > "$OUT/VERSION"
echo "wrote $OUT/classes.jar"
