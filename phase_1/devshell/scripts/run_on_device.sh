#!/usr/bin/env bash
# Install (fresh — uninstall first, since zkite-pack signs with a new random testing
# key on every build with no --pem, and Android refuses a signature-mismatched
# update) and launch the Dev Shell, same pattern as phase_0/spike2's script.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEVSHELL="$(dirname "$HERE")"
ADB="${ADB:-adb}"
PKG=dev.zkite.devshell
ACTIVITY="$PKG/.ZKiteActivity"

ABI="$($ADB shell getprop ro.product.cpu.abi | tr -d '\r')"
APK="$DEVSHELL/artifacts/zkite-devshell-$ABI.apk"
[ -f "$APK" ] || { echo "missing $APK — run ABI=$ABI scripts/build_apk.sh"; exit 1; }

echo "==> uninstalling any previous build"
$ADB uninstall "$PKG" >/dev/null 2>&1 || true

echo "==> installing $(basename "$APK") ($(stat -c%s "$APK") bytes)"
$ADB install "$APK"

echo "==> launching $ACTIVITY"
$ADB logcat -c
$ADB shell am start -W -n "$ACTIVITY" | sed 's/^/    /'

sleep 2
echo
echo "---- logcat -s ZKiteDevShell ----"
$ADB logcat -d -s ZKiteDevShell
echo "---------------------------------"
