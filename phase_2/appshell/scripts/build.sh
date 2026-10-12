#!/usr/bin/env bash
# Cross-compile the App Shell's cdylib for one or more Android ABIs. Identical
# approach to phase_1/devshell/scripts/build.sh (itself phase_0/spike1's
# PYO3_CONFIG_FILE trick, extended) — this *is* that lineage, not a rewrite.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APPSHELL="$(dirname "$HERE")"
PHASE2="$(dirname "$APPSHELL")"
PHASE0="$(dirname "$PHASE2")/phase_0"
VENDOR="$PHASE0/vendor"
OUT="$APPSHELL/artifacts"

: "${ANDROID_HOME:=$HOME/Android/Sdk}"
: "${ANDROID_API:=24}"
: "${NDK_VERSION:=27.3.13750724}"
NDK="$ANDROID_HOME/ndk/$NDK_VERSION"
PY_VERSION=3.14

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

  CONFIG="$APPSHELL/target/pyo3-config-$ARCH.txt"
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
    cargo build --release --manifest-path "$APPSHELL/Cargo.toml" --target "$TRIPLE"

  DEST="$OUT/$ABI_DIR"
  mkdir -p "$DEST"
  cp "$APPSHELL/target/$TRIPLE/release/libzkiteappshell.so" "$DEST/"
  cp "$PREFIX/lib/libpython$PY_VERSION.so"                 "$DEST/"

  # Which build produced this, recorded beside it — ../../phase_3/PLAN.md §34.8, and
  # phase_1/cli/src/provenance.rs for why the check is on the source rather than the
  # bytes. This shell has no path dependencies at all, so `cargo metadata` resolves to
  # this one crate; it is still derived rather than listed, because the day someone
  # adds one is precisely the day a hand-written list would go quietly wrong.
  python3 "$PHASE2/../phase_1/cli/scripts/artifact_provenance.py" record \
    --artifacts "$OUT" --shell webview --group "$ABI_DIR" \
    --cargo-manifest phase_2/appshell/Cargo.toml \
    --built-by phase_2/appshell/scripts/build.sh \
    --rebuild "phase_2/appshell/scripts/build.sh $ARCH" \
    --file "$ABI_DIR/libzkiteappshell.so" \
    --file "$ABI_DIR/libpython$PY_VERSION.so"
  echo "    -> $DEST"
done
