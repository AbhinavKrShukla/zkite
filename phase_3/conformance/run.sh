#!/usr/bin/env bash
# Phase 3 §5 item 5 — runs the full per-widget conformance pipeline end to end:
#
#   fixtures.py (Python, zkite_py)  -> manifest.json  (html + native tree per widget)
#   screenshot_web.mjs (Playwright/Chromium)  -> out/<name>_web.png
#   conformance render-all (Rust: zkite_renderer + vello_cpu)  -> out/<name>_native.png
#   conformance diff-all (Rust)  -> out/<name>_diff.png + out/report.json + a console table
#
# See PLAN.md's own writeup (§5 item 5, §9) for what the numbers mean and which gaps
# are real, already-known ones (Icon/Image have no native paint yet) vs. newly found
# here (see that section before assuming a nonzero % is a regression).
#
# Needs a `node_modules/` here with `playwright` resolvable (gitignored — not
# committed) plus its Chromium browser download. Cheapest way to get one: this
# directory's fixtures reuse phase_1/webruntime's own already-pinned Playwright
# install rather than a second copy —
#   ln -s ../../phase_1/webruntime/test/node_modules node_modules
# — after that directory's own `npm ci` has been run once (see its package-lock.json).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"

export PATH="$HOME/.cargo/bin:$PATH"
# pyo3-ffi (via zkite_pybridge) links against -lpython3.12 at final-binary link time —
# not needed by zkite_pybridge's own `cargo test` (an rlib target never resolves that
# symbol until something links it into an executable), but this crate does produce one.
# See this session's own finding, not documented anywhere before this file existed.
PY_LIBDIR="${ZKITE_PY_LIBDIR:-/usr/lib/python3.12/config-3.12-x86_64-linux-gnu}"
export RUSTFLAGS="-L ${PY_LIBDIR} ${RUSTFLAGS:-}"
export LD_LIBRARY_PATH="${PY_LIBDIR}:${LD_LIBRARY_PATH:-}"

mkdir -p out

# Both backends have to render the *same* typefaces for this comparison to measure
# ZKite's rendering rather than the gap between two fonts (../PLAN.md §9.4). This
# stages the three families into phase_1/webruntime/fonts/, which the WebView shell
# loads via @font-face and `conformance render-all` reads directly.
echo "== 0/5 staging the shared typefaces =="
../../phase_1/webruntime/scripts/stage_fonts.sh

echo "== 1/5 generating fixtures =="
python3 fixtures.py

echo "== 2/5 building conformance binary =="
cargo build --quiet

# Which device pixel ratios to run the whole comparison at (../PLAN.md §13).
#
# 1 is the historical run and every number recorded in PLAN.md §9 is at it. 2 exists
# because at dpr 1 this harness was *structurally incapable* of seeing a density bug —
# both backends render into a fixed pixel canvas where Tailwind's `px-6` is 24 pixels
# either way — and one duly shipped: PLAN.md §12.5.2 found, on a device, that native
# rendered every app at 1/dpr. §9.3 had already named this blind spot; running a second
# dpr is what closes it, and it closes it by testing the exact mechanism the device
# uses (lay out in dp, scale once at paint) rather than by asserting a scalar.
#
# A whole second pass rather than an inner loop because Playwright's deviceScaleFactor
# is a browser-context property and cannot change per screenshot.
DPRS="${ZKITE_CONFORMANCE_DPRS:-1 2}"

if [ -z "${ZKITE_CHROMIUM_PATH:-}" ]; then
  # Same override e2e.mjs already supports (phase_1/webruntime/test/e2e.mjs) — Playwright's
  # own browser-resolution-by-version sometimes points at a revision that isn't the one
  # actually cached (this session hit exactly that), so fall back to whatever Chromium
  # revision is already on disk rather than trying to download one (no guarantee of
  # network access from every environment this ever runs in).
  CACHED_CHROME=$(find "$HOME/.cache/ms-playwright"/chromium-*/chrome-linux64/chrome -maxdepth 0 2>/dev/null | head -1 || true)
  if [ -n "$CACHED_CHROME" ]; then
    export ZKITE_CHROMIUM_PATH="$CACHED_CHROME"
  fi
fi

for dpr in $DPRS; do
  # dpr 1 keeps writing to `out/` so every path named in PLAN.md §9 still resolves;
  # anything else gets its own directory rather than overwriting those PNGs.
  if [ "$dpr" = "1" ]; then outdir=out; else outdir="out-dpr${dpr}"; fi
  mkdir -p "$outdir"

  echo
  echo "===================== device pixel ratio ${dpr}x -> ${outdir}/ ====================="

  echo "== 3/5 native renders (@${dpr}x) =="
  ./target/debug/conformance render-all manifest.json "$outdir" "$dpr"

  echo "== 4/5 web screenshots, real Chromium via Playwright (@${dpr}x) =="
  node screenshot_web.mjs manifest.json "$outdir" "$dpr"

  echo "== 5/5 diff (@${dpr}x) =="
  ./target/debug/conformance diff-all manifest.json "$outdir"
done
