#!/usr/bin/env bash
# Cross-compile spike 1 for one or more Android ABIs.
#
# The whole point of plan1.md is that *users* never run this: it is the release-pipeline
# side. Nothing here touches the user's machine.
#
# Usage: scripts/build.sh [aarch64|x86_64 ...]        (default: both)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SPIKE="$(dirname "$HERE")"
PHASE0="$(dirname "$SPIKE")"
VENDOR="$PHASE0/vendor"
OUT="$PHASE0/artifacts"

: "${ANDROID_HOME:=$HOME/Android/Sdk}"
: "${ANDROID_API:=24}"           # matches the python.org prefix (build-details.json)
: "${NDK_VERSION:=27.3.13750724}" # the version CPython 3.14.6 was built with
NDK="$ANDROID_HOME/ndk/$NDK_VERSION"
PY_VERSION=3.14
PY_FULL=3.14.6

[ -d "$NDK" ] || { echo "NDK not found at $NDK"; exit 1; }
TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"

ABIS=("$@")
[ ${#ABIS[@]} -eq 0 ] && ABIS=(aarch64 x86_64)

mkdir -p "$OUT"

for ARCH in "${ABIS[@]}"; do
  case "$ARCH" in
    aarch64) TRIPLE=aarch64-linux-android;  ABI_DIR=arm64-v8a ;;
    x86_64)  TRIPLE=x86_64-linux-android;   ABI_DIR=x86_64 ;;
    *) echo "unknown arch $ARCH"; exit 1 ;;
  esac

  PREFIX="$VENDOR/python-$ARCH/prefix"
  if [ ! -f "$PREFIX/lib/libpython$PY_VERSION.so" ]; then
    echo "missing $PREFIX/lib/libpython$PY_VERSION.so — run $PHASE0/scripts/fetch_python.sh"
    exit 1
  fi

  # pyo3-ffi cross configuration. Handing it a config file is the documented escape from
  # its "run the host interpreter to introspect" build script, which cannot work when the
  # target interpreter is an Android .so.
  CONFIG="$SPIKE/target/pyo3-config-$ARCH.txt"
  mkdir -p "$(dirname "$CONFIG")"
  cat > "$CONFIG" <<EOF
implementation=CPython
version=$PY_VERSION
shared=true
abi3=false
lib_name=python$PY_VERSION
lib_dir=$PREFIX/lib
executable=
pointer_width=64
build_flags=
suppress_build_script_link_lines=false
EOF

  CC="$TOOLCHAIN/${TRIPLE}${ANDROID_API}-clang"
  AR="$TOOLCHAIN/llvm-ar"
  UPPER_TRIPLE=$(echo "$TRIPLE" | tr 'a-z-' 'A-Z_')

  echo "==> building $TRIPLE (API $ANDROID_API)"
  env \
    PYO3_CONFIG_FILE="$CONFIG" \
    CC_${TRIPLE//-/_}="$CC" \
    AR_${TRIPLE//-/_}="$AR" \
    CARGO_TARGET_${UPPER_TRIPLE}_LINKER="$CC" \
    CARGO_TARGET_${UPPER_TRIPLE}_RUSTFLAGS="-L $PREFIX/lib -C link-arg=-Wl,-rpath,\$ORIGIN -C link-arg=-Wl,-z,max-page-size=16384" \
    cargo build --release --manifest-path "$SPIKE/Cargo.toml" --target "$TRIPLE"

  DEST="$OUT/$ABI_DIR"
  mkdir -p "$DEST"
  cp "$SPIKE/target/$TRIPLE/release/libzkitespike1.so" "$DEST/"
  cp "$SPIKE/target/$TRIPLE/release/zkite-py-probe"    "$DEST/"
  cp "$PREFIX/lib/libpython$PY_VERSION.so"            "$DEST/"
  echo "    -> $DEST"
done

echo
echo "artifacts:"
find "$OUT" -type f \( -name '*.so' -o -name 'zkite-py-probe' \) -printf '  %-56p %8s bytes\n' | sort
