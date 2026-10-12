#!/usr/bin/env bash
# Spike 2's pass/fail check: install the pack-built APK and confirm it launches and runs
# our code. Also re-confirms spike 1 from inside a real app process, which is a strictly
# harder environment than the adb-shell probe (SELinux app domain, W^X, no exec-from-data).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PHASE0="$(dirname "$(dirname "$HERE")")"
ADB="${ADB:-$HOME/Android/Sdk/platform-tools/adb}"
PKG=dev.zkite.phase0
ACTIVITY="$PKG/.ZKiteActivity"

ABI="$($ADB shell getprop ro.product.cpu.abi | tr -d '\r')"
APK="$PHASE0/artifacts/zkite-phase0-$ABI.apk"
[ -f "$APK" ] || { echo "missing $APK — run ABI=$ABI scripts/build_apk.sh"; exit 1; }

echo "==> uninstalling any previous build"
$ADB uninstall "$PKG" >/dev/null 2>&1 || true

echo "==> installing $(basename "$APK") ($(stat -c%s "$APK") bytes)"
t0=$(date +%s.%N)
$ADB install "$APK"
t1=$(date +%s.%N)
printf "    install took %.2fs\n" "$(echo "$t1 - $t0" | bc)"

echo "==> launching $ACTIVITY"
$ADB logcat -c
$ADB shell am start -W -n "$ACTIVITY" | sed 's/^/    /'

# The Activity logs its result lines during onCreate; unpacking the stdlib on first run
# takes a moment, so poll rather than sleeping a fixed amount.
echo "==> waiting for results"
DEADLINE=$((SECONDS + 60))
while [ $SECONDS -lt $DEADLINE ]; do
  LOG="$($ADB logcat -d -s ZKitePhase0 2>/dev/null || true)"
  if echo "$LOG" | grep -q 'ZKITE_SPIKE1_RESULT'; then break; fi
  sleep 2
done

echo
echo "---- logcat -s ZKitePhase0 ----"
$ADB logcat -d -s ZKitePhase0
echo "------------------------------"
echo

SPIKE2=fail
SPIKE1=fail
LOG="$($ADB logcat -d -s ZKitePhase0)"
echo "$LOG" | grep -q 'ZKITE_SPIKE2_RESULT ok=1' && SPIKE2=pass
echo "$LOG" | grep -q 'ZKITE_SPIKE1_RESULT ok=1' && SPIKE1=pass

echo "SPIKE 2 (pack-built APK installs, launches, runs our dex + .so): ${SPIKE2^^}"
echo "SPIKE 1 (CPython boots inside the app process):                  ${SPIKE1^^}"
[ "$SPIKE2" = pass ] && [ "$SPIKE1" = pass ]
