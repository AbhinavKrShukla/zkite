#!/usr/bin/env python3
"""Regenerate ``src/palette.rs`` from Tailwind's own colour definitions.

Hand-typing 286 hex values is a slow-motion bug: `indigo-500` off by one digit is the
kind of thing nobody notices for six months. So the table is generated from upstream and
`tests/palette.rs` asserts every entry round-trips through a class string.

    python3 scripts/gen_palette.py [path-to-tailwind-browser.js]

Writes ``palette.json`` (provenance, checked in) and ``src/palette.rs`` (generated).

Source of truth, and why it changed
-----------------------------------
This originally fetched ``tailwindcss@3.4.17``'s ``src/public/colors.js`` over the
network, which is where the v3 hex table lived. That is now wrong for ZKite, for a
reason phase 3 measured rather than predicted (``phase_3/PLAN.md`` §9.5): phase 1
vendored ``@tailwindcss/browser`` **4.x** for the WebView backend, and v4 re-authored
the entire default palette in OKLCH — genuinely different colours, not a different
encoding of the same ones. ``indigo-500`` is ``#6366f1`` in v3 and renders as
``rgb(97, 95, 255)`` in v4. So the two backends disagreed on every saturated colour,
and every conformance run to date reported it as clean because the delta sat under the
harness's tolerance.

The fix is not "fetch v4 from npm" but "read the palette out of the exact file the
other backend actually runs" — ``phase_1/webruntime/vendor/tailwind-browser.js``. That
makes the two backends agree *by construction*: they cannot drift apart without
someone re-vendoring that file, at which point re-running this script is the obvious
next step. It also removes this script's network dependency entirely.

OKLCH -> sRGB
-------------
v4 states colours as ``oklch(L% C H)``, so this has to do the conversion a browser
would. The transform below is the standard OKLab matrix pair plus the sRGB transfer
function, with out-of-gamut results **clipped** per channel rather than gamut-mapped.
Clipping is what the reference values demand: ``indigo-500``'s chroma puts blue past
1.0, and Chromium renders it as 255. Verified, not assumed — ``spot_values_match_
chromiums_own_rendering`` in ``tests/palette.rs`` pins the two colours phase 3 sampled
straight out of a real Chromium screenshot (``indigo-500`` -> (97, 95, 255),
``slate-950`` -> (2, 6, 24)), and both reproduce exactly.

Two columns, because clipping is right for a fill and wrong for a gradient stop
----------------------------------------------------------------------------------
Each row carries the clipped ``0xRRGGBB`` **and** the same colour *unclipped*, in
encoded sRGB as f32 with the transfer function odd-extended for negative channels
(``-f(-c)``), which is CSS's own representation for an out-of-gamut colour.

The second column exists because of a divergence phase 3 measured (``phase_3/PLAN.md``
§38.3). Chromium interpolates a gradient using the **unclipped** stop colour and clips
only at display time, so ``from-yellow-400`` — whose blue is ``-68.6`` on the 0..255
scale — makes the blue channel stay at 0 for the first fifth of the ramp. Feeding the
clipped hex to the same interpolation starts the ramp 68.6 too high, and 68 is exactly
the per-channel delta the conformance harness reported before this column existed.
A fill is unaffected: what a browser *paints* for a solid ``bg-yellow-400`` is the
clipped value, which is what the first column is for.

Most saturated v4 colours are outside the sRGB gamut in at least one channel, so this
is the common case rather than an exotic one.
"""

from __future__ import annotations

import json
import math
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(CRATE))
DEFAULT_SOURCE = os.path.join(REPO, "phase_1", "webruntime", "vendor", "tailwind-browser.js")

# `--color-indigo-500: oklch(58.5% 0.233 277.117)`. The hue component may be the CSS
# Color 4 keyword `none` for achromatic entries (every `neutral` shade, plus the
# lightest `zinc`/`mauve`/... ones) — chroma is 0 there, so the hue genuinely carries
# no information and 0 is exact rather than an approximation.
ENTRY = re.compile(
    r"--color-([a-z]+)-(\d+):\s*oklch\(\s*([\d.]+)%\s+([\d.]+)\s+(none|[\d.]+)\s*\)"
)


def read_source(path: str) -> str:
    with open(path, encoding="utf-8") as f:
        return f.read()


def source_version(js: str, path: str) -> str:
    """Best-effort version string, from the sibling .VERSION file the vendoring wrote."""
    version_file = os.path.splitext(path)[0] + ".VERSION"
    if os.path.exists(version_file):
        with open(version_file, encoding="utf-8") as f:
            return f.read().strip()
    return os.path.basename(path)


def _encode(c: float) -> float:
    """Linear sRGB -> encoded sRGB, **odd-extended** for negative channels.

    CSS Color 4 defines the transfer function on negatives as `-f(-c)`, which is what
    makes an out-of-gamut colour interpolable rather than merely clippable. Not clipped
    here — the caller decides, because the two columns want different answers.
    """
    sign = -1.0 if c < 0 else 1.0
    c = abs(c)
    e = 12.92 * c if c <= 0.0031308 else 1.055 * (c ** (1 / 2.4)) - 0.055
    return sign * e


def oklch_to_srgb(lightness_pct: float, chroma: float, hue_deg: float) -> tuple[float, float, float]:
    """`oklch(L% C H)` -> encoded sRGB, unclipped (channels may be <0 or >1)."""
    lightness = lightness_pct / 100.0
    hue = math.radians(hue_deg)
    a = chroma * math.cos(hue)
    b = chroma * math.sin(hue)

    l_ = lightness + 0.3963377774 * a + 0.2158037573 * b
    m_ = lightness - 0.1055613458 * a - 0.0638541728 * b
    s_ = lightness - 0.0894841775 * a - 1.2914855480 * b
    l, m, s = l_**3, m_**3, s_**3

    linear = (
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    )

    return tuple(_encode(channel) for channel in linear)


def rgb24(encoded: tuple[float, float, float]) -> int:
    """Encoded sRGB -> 0xRRGGBB, clipped per channel — what a browser paints for a fill."""
    rgb = 0
    for c in encoded:
        rgb = (rgb << 8) | round(min(1.0, max(0.0, c)) * 255)
    return rgb


def parse(js: str) -> dict[str, dict[str, str]]:
    """Pull every `--color-<hue>-<shade>` OKLCH declaration out of the bundle."""
    out: dict[str, dict[str, str]] = {}
    for hue, shade, lightness, chroma, hue_deg in ENTRY.findall(js):
        encoded = oklch_to_srgb(
            float(lightness), float(chroma), 0.0 if hue_deg == "none" else float(hue_deg)
        )
        # `palette.json` keeps the hex it always did — it is the provenance record a
        # human reads — plus the unclipped triple the second column is generated from.
        out.setdefault(hue, {})[shade] = {
            "hex": "#%06x" % rgb24(encoded),
            "srgb": [round(c, 6) for c in encoded],
        }
    return out


def emit(palette: dict[str, dict[str, str]], version: str) -> str:
    shades = len(next(iter(palette.values())))
    lines = [
        "// @generated by scripts/gen_palette.py from %s" % version,
        "// Do not edit by hand. %d hues x %d shades, converted from Tailwind v4's own"
        % (len(palette), shades),
        "// OKLCH values — see that script's module docstring for why the source is the",
        "// vendored browser bundle and not npm, and phase_3/PLAN.md §9.5 for the",
        "// backend divergence that prompted it.",
        "",
        "/// (hue, shade, 0xRRGGBB, unclipped encoded sRGB). Sorted by hue then shade for",
        "/// binary search.",
        "///",
        "/// **Two representations of one colour, and they are not interchangeable.** The",
        "/// `u32` is gamut-clipped, which is what a browser paints for a solid fill. The",
        "/// `[f32; 3]` is the same colour *unclipped*, in encoded sRGB with the transfer",
        "/// function odd-extended for negatives — the form CSS interpolates a gradient stop",
        "/// in. Most saturated v4 colours are out of the sRGB gamut in some channel, and",
        "/// using the clipped value as a gradient stop was measured at **68/255** off",
        "/// Chromium (phase_3/PLAN.md §38.3).",
        "pub const PALETTE: &[(&str, u16, u32, [f32; 3])] = &[",
    ]
    for hue in sorted(palette):
        for shade in sorted(palette[hue], key=int):
            row = palette[hue][shade]
            srgb = ", ".join("%.6f" % c for c in row["srgb"])
            lines.append('    ("%s", %s, 0x%s, [%s]),' % (hue, shade, row["hex"][1:], srgb))
    lines += [
        "];",
        "",
        "pub const HUES: &[&str] = &[%s];" % ", ".join('"%s"' % h for h in sorted(palette)),
    ]
    return "\n".join(lines) + "\n"


def main() -> int:
    path = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_SOURCE
    if not os.path.exists(path):
        print("no such source: %s" % path, file=sys.stderr)
        return 1
    js = read_source(path)
    palette = parse(js)
    if not palette:
        print("parsed nothing — upstream layout probably changed", file=sys.stderr)
        return 1

    shade_counts = {len(v) for v in palette.values()}
    if len(shade_counts) != 1:
        print("hues have differing shade counts: %s" % shade_counts, file=sys.stderr)
        return 1

    version = source_version(js, path)
    total = sum(len(v) for v in palette.values())
    with open(os.path.join(CRATE, "palette.json"), "w") as f:
        json.dump(palette, f, indent=0, sort_keys=True)
    with open(os.path.join(CRATE, "src", "palette.rs"), "w") as f:
        f.write(emit(palette, version))

    print("wrote %d hues, %d colours from %s" % (len(palette), total, version))
    print("now run: cargo test --test palette")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
