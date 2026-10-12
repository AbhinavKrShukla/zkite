#!/usr/bin/env bash
# Cross-compile the GPU harness's cdylib for one or more Android ABIs.
# ../../PLAN.md §4/§5.
#
# Originally simpler than devshell/appshell's build.sh (no PyO3/CPython dependency, so
# `cargo ndk` alone resolved the toolchain — see git history). §5's `zkite_pybridge`
# wiring gave this crate the exact same CPython embedding devshell/appshell already
# have, so it now needs the exact same `PYO3_CONFIG_FILE`-driven manual toolchain
# setup they use, not `cargo ndk`'s automatic one — the two don't compose cleanly
# (cargo-ndk sets its own CC/AR/rustflags for the NDK toolchain; overriding those out
# from under it to also inject Python's `-L`/rpath flags is exactly the kind of "two
# things fighting over the same env vars" situation not worth debugging when the
# manual approach is already proven correct by two other crates in this repo).
#
# The one thing this crate alone needs on top of that shared recipe: `ndk`'s
# `nativewindow` feature links against `libnativewindow.so`, which the NDK only ships
# starting at API 26 (see ../../PLAN.md §4's research notes) — hence `ANDROID_API`
# defaults to 26 here, not devshell/appshell's 24. Matches every ZKite manifest's
# `minSdkVersion` (../app/AndroidManifest.xml) either way, so this isn't a new floor.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GPU_HARNESS="$(dirname "$HERE")"
PHASE3="$(dirname "$GPU_HARNESS")"
PHASE0="$(dirname "$PHASE3")/phase_0"
PHASE1="$(dirname "$PHASE3")/phase_1"
VENDOR="$PHASE0/vendor"
OUT="$GPU_HARNESS/artifacts"

: "${ANDROID_HOME:=$HOME/Android/Sdk}"
: "${ANDROID_API:=26}"
: "${NDK_VERSION:=27.3.13750724}"
NDK="$ANDROID_HOME/ndk/$NDK_VERSION"
PY_VERSION=3.14

[ -d "$NDK" ] || { echo "NDK not found at $NDK"; exit 1; }
TOOLCHAIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"

TARGETS=("$@")
[ ${#TARGETS[@]} -eq 0 ] && TARGETS=(arm64-v8a x86_64)

mkdir -p "$OUT"

# ------------------------------------------------------- ABI-independent artifacts
#
# `zkite build apk --backend native` stages everything below straight out of
# artifacts/, the same way it already does for phase_2/appshell — nothing here may
# require an NDK, a JDK, or a host font directory on the *user's* machine (phase_2/
# PLAN.md §1.1). That is why fonts.zip is baked here rather than zipped by the CLI
# from phase_1/webruntime/fonts/: that directory is gitignored and materialised by
# stage_fonts.sh out of the *host's* /usr/share/fonts, which a user's machine has no
# reason to have.
#
# Which typefaces get baked in is still the open product decision ../../PLAN.md §9.7
# flags (bundle a family vs. read /system/fonts). This makes DejaVu concrete for the
# shipped native backend without settling it — the zip's contents are whatever
# stage_fonts.sh staged, and changing that is a one-line ZKITE_FONT_* override.
"$PHASE1/webruntime/scripts/stage_fonts.sh" "$PHASE1/webruntime/fonts" >/dev/null
python3 "$PHASE1/../scripts/zip_tree.py" "$PHASE1/webruntime/fonts" "$OUT/fonts.zip" --suffix .ttf
echo "fonts.zip: $(stat -c%s "$OUT/fonts.zip") bytes -> $OUT/fonts.zip"

# Which build produced this, recorded beside it — ../../PLAN.md §34.8, and see
# phase_1/cli/src/provenance.rs for why the check is on the *source*, not the bytes.
#
# `--source-incomplete` because the honest answer here is that no digest over this
# repository determines fonts.zip: stage_fonts.sh copies faces out of the *host's*
# /usr/share/fonts (see the comment above), so a matching hash would be asserting
# something it cannot see. `zkite build` prints it as unverified rather than counting
# it, which is the difference between naming a limit and hiding one.
PROVENANCE="$PHASE1/cli/scripts/artifact_provenance.py"
python3 "$PROVENANCE" record \
  --artifacts "$OUT" --shell native --group fonts --source-incomplete \
  --extra phase_1/webruntime/scripts/stage_fonts.sh \
  --extra scripts/zip_tree.py \
  --built-by phase_3/gpu_harness/scripts/build.sh \
  --rebuild 'phase_3/gpu_harness/scripts/build.sh' \
  --file fonts.zip

for T in "${TARGETS[@]}"; do
  case "$T" in
    arm64-v8a) ARCH=aarch64; TRIPLE=aarch64-linux-android; ABI_DIR=arm64-v8a ;;
    x86_64)    ARCH=x86_64;  TRIPLE=x86_64-linux-android;  ABI_DIR=x86_64 ;;
    *) echo "unknown target $T"; exit 1 ;;
  esac

  PREFIX="$VENDOR/python-$ARCH/prefix"
  if [ ! -f "$PREFIX/lib/libpython$PY_VERSION.so" ]; then
    echo "missing $PREFIX/lib/libpython$PY_VERSION.so — run $PHASE0/scripts/fetch_python.sh"
    exit 1
  fi

  CONFIG="$GPU_HARNESS/target/pyo3-config-$ARCH.txt"
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
  CXX="$TOOLCHAIN/${TRIPLE}${ANDROID_API}-clang++"
  AR="$TOOLCHAIN/llvm-ar"
  UPPER_TRIPLE=$(echo "$TRIPLE" | tr 'a-z-' 'A-Z_')

  echo "==> building $TRIPLE (API $ANDROID_API)"
  # --release, not debug: a debug build of the whole wgpu/vello_hybrid/CPython
  # dependency graph is unstripped and hundreds of MB per .so (measured earlier this
  # phase for the pre-Python build alone) — fine for a `cargo check`-style compile
  # pass, not for something you actually want to `adb install`.
  #
  # CXX_<target>, alongside the existing CC_<target>: `build.rs` compiles
  # vendor/gametextinput's C++ source via the `cc` crate (../../PLAN.md §5, IME),
  # which needs the NDK's clang++ specifically, not clang — `cc` looks for this exact
  # env var name for a C++ source file, same underscore convention as CC_<target>.
  env \
    PYO3_CONFIG_FILE="$CONFIG" \
    CC_${TRIPLE//-/_}="$CC" \
    CXX_${TRIPLE//-/_}="$CXX" \
    AR_${TRIPLE//-/_}="$AR" \
    CARGO_TARGET_${UPPER_TRIPLE}_LINKER="$CC" \
    CARGO_TARGET_${UPPER_TRIPLE}_RUSTFLAGS="-L $PREFIX/lib -C link-arg=-Wl,-rpath,\$ORIGIN -C link-arg=-Wl,-z,max-page-size=16384" \
    cargo build --release --locked --manifest-path "$GPU_HARNESS/Cargo.toml" --target "$TRIPLE"

  DEST="$OUT/$ABI_DIR"
  mkdir -p "$DEST"
  cp "$GPU_HARNESS/target/$TRIPLE/release/libzkitegpuharness.so" "$DEST/"

  # vendor/gametextinput's C++ (compiled in by build.rs, ../../PLAN.md §7.1) pulls in
  # the NDK's C++ runtime, which clang++ links as a *shared* library by default — so
  # libzkitegpuharness.so carries a DT_NEEDED on libc++_shared.so that has to be
  # present alongside it in any APK, exactly as libpython3.14.so is. Copied into
  # artifacts/ here rather than located by the CLI: this is the only step in the
  # chain that legitimately has an NDK, and `zkite build` on a user's machine must not
  # need one (phase_2/PLAN.md §1.1).
  LIBCXX="$NDK/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/$TRIPLE/libc++_shared.so"
  [ -f "$LIBCXX" ] || { echo "missing $LIBCXX (NDK layout changed? check NDK_VERSION)"; exit 1; }
  cp "$LIBCXX" "$DEST/"

  # The shipped path stages libpython from artifacts/ too, again so the CLI needs
  # nothing but this directory. (build_apk.sh, the harness's own APK build, still
  # reads the vendored prefix directly — it is a release-pipeline script and already
  # depends on far more of phase_0/vendor than this.)
  cp "$PREFIX/lib/libpython$PY_VERSION.so" "$DEST/"

  # The source set is derived by `cargo metadata`, not listed here: zkite_renderer,
  # zkite_pybridge, zkite_imagestore *and* phase_0/spike3_twcore all feed this .so, and
  # twcore arrives two crates deep — a hand-written list would have missed exactly the
  # crate §36 was editing. vendor/ is added on top because build.rs compiles
  # gametextinput's C++ into the same cdylib (../../PLAN.md §7.1) and no Cargo manifest
  # mentions those sources.
  python3 "$PROVENANCE" record \
    --artifacts "$OUT" --shell native --group "$ABI_DIR" \
    --cargo-manifest phase_3/gpu_harness/Cargo.toml \
    --extra phase_3/gpu_harness/vendor \
    --built-by phase_3/gpu_harness/scripts/build.sh \
    --rebuild "phase_3/gpu_harness/scripts/build.sh $T" \
    --file "$ABI_DIR/libzkitegpuharness.so" \
    --file "$ABI_DIR/libc++_shared.so" \
    --file "$ABI_DIR/libpython$PY_VERSION.so"
  echo "    -> $DEST"
done
