#!/usr/bin/env bash
# Run every phase-0 spike and print a pass/fail table.
#
# plan1.md §10: "If any spike fails, the architecture changes. Don't write framework code
# until all three pass." This script is the gate.
#
#   ./run_all.sh            host-only checks (spike 3 + build + APK structure)
#   ./run_all.sh --device   the above, plus the on-device checks (needs adb + a device)
#   ./run_all.sh --strict   (with either of the above) missing build-tools/no device is a
#                           FAIL, not a SKIP — use this in CI, where a silently-skipped
#                           check must not read as a pass
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
: "${ANDROID_HOME:=$HOME/Android/Sdk}"
export ANDROID_HOME
ADB="${ADB:-$ANDROID_HOME/platform-tools/adb}"

WITH_DEVICE=0
STRICT=0
for arg in "$@"; do
  case "$arg" in
    --device) WITH_DEVICE=1 ;;
    --strict) STRICT=1 ;;
  esac
done

declare -a NAMES RESULTS
step() {
  local name="$1"; shift
  echo
  echo "############################################################"
  echo "# $name"
  echo "############################################################"
  if "$@"; then
    NAMES+=("$name"); RESULTS+=(PASS)
  else
    NAMES+=("$name"); RESULTS+=(FAIL)
  fi
}

skip() {
  NAMES+=("$1"); RESULTS+=(SKIP)
}

# ---------------------------------------------------------------- spike 3
step "spike 3 · twcore: Tailwind -> taffy::Style (host tests)" \
  cargo test --manifest-path "$HERE/spike3_twcore/Cargo.toml"

# ---------------------------------------------------------------- spike 1
step "spike 1 · cross-compile CPython embedding (arm64 + x86_64)" \
  "$HERE/spike1_cpython_embed/scripts/build.sh"

# ---------------------------------------------------------------- spike 2
if [ -x "$ANDROID_HOME/build-tools/34.0.0/d8" ]; then
  step "spike 2 · build a signed APK with pack (x86_64)" \
    env ABI=x86_64 "$HERE/spike2_pack_apk/scripts/build_apk.sh"
  step "spike 2 · APK structure tests" \
    cargo test --release --manifest-path "$HERE/spike2_pack_apk/Cargo.toml"
else
  skip "spike 2 · build a signed APK with pack (x86_64)"
  skip "spike 2 · APK structure tests"
  echo "!! no build-tools/34.0.0/d8 — skipping the dex + APK build"
fi

# ---------------------------------------------------------------- on device
if [ "$WITH_DEVICE" = 1 ]; then
  if [ -n "$("$ADB" devices | sed -n '2p')" ]; then
    step "spike 1 · CPython runs on device (standalone probe)" \
      "$HERE/spike1_cpython_embed/scripts/run_on_device.sh"
    step "spike 2 · APK installs, launches, boots CPython in-app" \
      "$HERE/spike2_pack_apk/scripts/run_on_device.sh"
  else
    skip "spike 1 · CPython runs on device (standalone probe)"
    skip "spike 2 · APK installs, launches, boots CPython in-app"
    echo "!! no device attached to adb"
  fi
fi

# ---------------------------------------------------------------- summary
echo
echo "############################################################"
echo "# phase 0 summary"
echo "############################################################"
FAILED=0
SKIPPED=0
for i in "${!NAMES[@]}"; do
  printf "  %-6s %s\n" "${RESULTS[$i]}" "${NAMES[$i]}"
  [ "${RESULTS[$i]}" = FAIL ] && FAILED=1
  [ "${RESULTS[$i]}" = SKIP ] && SKIPPED=$((SKIPPED + 1))
done
echo
if [ "$SKIPPED" -gt 0 ]; then
  if [ "$STRICT" = 1 ]; then
    echo "$SKIPPED check(s) SKIPPED — treating as FAIL because --strict was passed."
    FAILED=1
  else
    echo "$SKIPPED check(s) SKIPPED (missing build-tools or no device attached) — this is"
    echo "NOT a full phase-0 pass. Re-run with the SDK/device present, or --strict in CI."
  fi
fi
if [ "$FAILED" = 0 ]; then
  echo "All executed checks passed."
else
  echo "Something failed — per plan1.md §10, do not start phase 1."
fi
exit $FAILED
