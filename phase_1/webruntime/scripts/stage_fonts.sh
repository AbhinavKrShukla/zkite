#!/usr/bin/env bash
# Materialise the three typeface families both backends render with, into
# `phase_1/webruntime/fonts/`.
#
# Why one directory shared by both backends, rather than each staging its own:
# phase_3/PLAN.md §9.4 found that the two backends were rendering different
# *typefaces* — native used whichever single font `Renderer::new` was handed, the
# WebView used whatever the host/device resolved `sans-serif` to (Noto Sans on the
# conformance host, Roboto on a device). That is not a bug either backend can fix
# alone; it is fixed by both reading the same files. Keeping them in one directory
# makes that structural instead of a convention two build scripts have to remember.
#
# Not committed (see .gitignore) — these are ~2.8 MB of regenerable binaries, and the
# repo's standing convention is to point at or fetch large vendored blobs rather than
# check them in (phase_2/vendor/protoc, gpu_harness/vendor/androidx-core). Every
# consumer runs this script first; it fails loudly rather than leaving an empty
# directory, because an empty one would make the browser silently fall back to a
# system font and reintroduce the exact divergence this exists to remove.
#
# Override any family with the ZKITE_FONT_* variables, in the same style
# `phase_3/gpu_harness/scripts/build_apk.sh` already uses for ZKITE_FONT/ZKITE_FONT_BOLD.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WEBRUNTIME="$(dirname "$HERE")"
OUT="${1:-$WEBRUNTIME/fonts}"

DEJAVU=/usr/share/fonts/truetype/dejavu
if [ ! -f "$DEJAVU/DejaVuSans.ttf" ] && [ -f /usr/share/fonts/TTF/DejaVuSans.ttf ]; then
  DEJAVU=/usr/share/fonts/TTF
fi
: "${ZKITE_FONT_SANS:=$DEJAVU/DejaVuSans.ttf}"
: "${ZKITE_FONT_SANS_BOLD:=$DEJAVU/DejaVuSans-Bold.ttf}"
: "${ZKITE_FONT_SERIF:=$DEJAVU/DejaVuSerif.ttf}"
: "${ZKITE_FONT_SERIF_BOLD:=$DEJAVU/DejaVuSerif-Bold.ttf}"
: "${ZKITE_FONT_MONO:=$DEJAVU/DejaVuSansMono.ttf}"
: "${ZKITE_FONT_MONO_BOLD:=$DEJAVU/DejaVuSansMono-Bold.ttf}"

mkdir -p "$OUT"

stage() {
  local src="$1" dest="$2" var="$3"
  [ -f "$src" ] || { echo "font not found at $src — set $var" >&2; exit 1; }
  cp "$src" "$OUT/$dest"
}

stage "$ZKITE_FONT_SANS"       sans.ttf        ZKITE_FONT_SANS
stage "$ZKITE_FONT_SANS_BOLD"  sans-bold.ttf   ZKITE_FONT_SANS_BOLD
stage "$ZKITE_FONT_SERIF"      serif.ttf       ZKITE_FONT_SERIF
stage "$ZKITE_FONT_SERIF_BOLD" serif-bold.ttf  ZKITE_FONT_SERIF_BOLD
stage "$ZKITE_FONT_MONO"       mono.ttf        ZKITE_FONT_MONO
stage "$ZKITE_FONT_MONO_BOLD"  mono-bold.ttf   ZKITE_FONT_MONO_BOLD

echo "staged 6 font faces into $OUT"
