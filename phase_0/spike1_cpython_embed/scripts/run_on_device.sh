#!/usr/bin/env bash
# Spike 1's pass/fail check, run against whatever device `adb` is talking to.
#
# Pushes the probe binary, libpython and a stdlib to /data/local/tmp and runs it. No APK
# involved, so a failure here is unambiguously about the CPython embedding rather than
# about packaging — that separation is why the spike is worth running on its own.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PHASE0="$(dirname "$(dirname "$HERE")")"
ADB="${ADB:-$HOME/Android/Sdk/platform-tools/adb}"
REMOTE=/data/local/tmp/zkite-phase0
PY_VERSION=3.14

ABI="$($ADB shell getprop ro.product.cpu.abi | tr -d '\r')"
case "$ABI" in
  x86_64)    ARCH=x86_64 ;;
  arm64-v8a) ARCH=aarch64 ;;
  *) echo "unsupported device ABI: $ABI"; exit 1 ;;
esac
echo "device ABI: $ABI"

PREFIX="$PHASE0/vendor/python-$ARCH/prefix"
ART="$PHASE0/artifacts/$ABI"
for f in "$ART/zkite-py-probe" "$ART/libpython$PY_VERSION.so"; do
  [ -f "$f" ] || { echo "missing $f — run spike1_cpython_embed/scripts/build.sh $ARCH"; exit 1; }
done

echo "==> staging to $REMOTE"
$ADB shell "rm -rf $REMOTE && mkdir -p $REMOTE/lib"
$ADB push -q "$ART/zkite-py-probe"            "$REMOTE/" >/dev/null
$ADB push -q "$ART/libpython$PY_VERSION.so"  "$REMOTE/lib/" >/dev/null

# The stdlib. Pushing 60 MB of loose files over adb is slow, so send one tar.
TARBALL="$PHASE0/spike1_cpython_embed/target/stdlib-$ARCH.tar"
if [ ! -f "$TARBALL" ]; then
  mkdir -p "$(dirname "$TARBALL")"
  tar -C "$PREFIX/lib" --exclude=test --exclude=idlelib --exclude=tkinter \
      --exclude=__pycache__ --exclude=turtledemo --exclude=lib2to3 \
      -cf "$TARBALL" "python$PY_VERSION"
fi
$ADB push -q "$TARBALL" "$REMOTE/stdlib.tar" >/dev/null
$ADB shell "cd $REMOTE/lib && tar xf ../stdlib.tar && rm ../stdlib.tar"
$ADB shell "chmod 755 $REMOTE/zkite-py-probe"

echo "==> running probe"
set +e
OUTPUT="$($ADB shell "cd $REMOTE && LD_LIBRARY_PATH=$REMOTE/lib ./zkite-py-probe $REMOTE 2>&1")"
STATUS=$?
set -e
echo "$OUTPUT"

if echo "$OUTPUT" | grep -q 'ZKITE_SPIKE1_RESULT ok=1'; then
  echo
  echo "SPIKE 1: PASS on $ABI"
  exit 0
fi
echo
echo "SPIKE 1: FAIL on $ABI (exit $STATUS)"
exit 1
