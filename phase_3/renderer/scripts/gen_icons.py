#!/usr/bin/env python3
"""Regenerate ``src/icons.rs`` from the WebView backend's own sprite sheet.

    python3 scripts/gen_icons.py [path-to-webruntime/index.html]

Writes ``icons.json`` (provenance, checked in) and ``src/icons.rs`` (generated).

Why this is generated rather than hand-written
----------------------------------------------
``Icon`` is drawn twice: the WebView backend renders ``<use href="#icon-star">``
against the ``<symbol>`` sheet in ``phase_1/webruntime/index.html``, and the native
backend rasterises the same geometry through vello. Two hand-maintained copies of one
design is a shape this project has already paid for twice — ``phase_3/PLAN.md`` §9.5
(``twcore``'s palette was Tailwind v3 while the WebView ran v4, and every conformance
run reported it clean) and §9.7 (native shaped DejaVu while Chromium resolved
``sans-serif`` to Noto Sans). Both were fixed the same way: point the second consumer
at the file the first one actually runs. This does that for the sprites, so the two
backends cannot drift without someone editing ``index.html``, at which point re-running
this script is the obvious next step.

SVG presentation attributes, resolved here rather than at runtime
-----------------------------------------------------------------
The sprite sheet is real SVG, so ``fill``/``stroke`` follow SVG's own defaults, and
those defaults are *not* what a reader assumes:

* ``fill`` absent means **black**, not "no fill". ``icon-search``'s second path omits
  ``fill="none"`` (an authoring slip in ``index.html``), so per spec it is filled black
  *and* stroked. It is a two-point open line with zero area, so the fill paints nothing
  in either engine — but this script records it faithfully rather than normalising it
  away, because silently "fixing" the source here is exactly how the two backends would
  drift again.
* ``stroke="currentColor"`` resolves to the node's own resolved text colour, which is
  the native side's ``ComputedStyle::text::color``. That is the only reason ``Icon``
  responds to ``text-white`` at all.

Circles become path data here too (two arcs), so the runtime handles exactly one shape
kind and ``kurbo::BezPath::from_svg`` is the whole parser.
"""

from __future__ import annotations

import json
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
DEFAULT_HTML = REPO / "phase_1" / "webruntime" / "index.html"
OUT_RS = Path(__file__).resolve().parents[1] / "src" / "icons.rs"
OUT_JSON = Path(__file__).resolve().parents[1] / "icons.json"

SYMBOL_RE = re.compile(r"<symbol\b[^>]*>.*?</symbol>", re.S)

# SVG's own initial values for the two presentation attributes this sheet uses.
DEFAULT_FILL = "black"
DEFAULT_STROKE = "none"

NAMED_COLOURS = {"black": (0, 0, 0), "white": (255, 255, 255)}


def circle_to_path(cx: float, cy: float, r: float) -> str:
    """A closed circle as two half-arcs — the form `kurbo::BezPath::from_svg` accepts."""
    return f"M{cx - r},{cy}A{r},{r} 0 1,0 {cx + r},{cy}A{r},{r} 0 1,0 {cx - r},{cy}Z"


def parse_paint(value: str, what: str, symbol_id: str) -> str:
    """`none` | `current` | `rgb:R,G,B` — the three cases `IconPaint` can express."""
    value = value.strip()
    if value == "none":
        return "none"
    if value == "currentColor":
        return "current"
    if value in NAMED_COLOURS:
        r, g, b = NAMED_COLOURS[value]
        return f"rgb:{r},{g},{b}"
    if re.fullmatch(r"#[0-9a-fA-F]{6}", value):
        return f"rgb:{int(value[1:3], 16)},{int(value[3:5], 16)},{int(value[5:7], 16)}"
    raise SystemExit(
        f"gen_icons: {symbol_id}: unsupported {what}={value!r}. Add it to NAMED_COLOURS "
        f"or teach IconPaint a new variant — do not silently drop it, that is how the "
        f"two backends diverge."
    )


def parse_symbol(fragment: str) -> dict:
    el = ET.fromstring(fragment)
    symbol_id = el.get("id", "")
    if not symbol_id.startswith("icon-"):
        raise SystemExit(f"gen_icons: <symbol id={symbol_id!r}> does not start with 'icon-'")
    name = symbol_id[len("icon-") :]

    view_box = el.get("viewBox", "")
    parts = [float(p) for p in view_box.replace(",", " ").split()]
    if len(parts) != 4 or parts[0] != 0.0 or parts[1] != 0.0 or parts[2] != parts[3]:
        raise SystemExit(
            f"gen_icons: {symbol_id}: viewBox {view_box!r} is not a square starting at the "
            f"origin. The runtime scales a sprite by one uniform factor into its node's "
            f"box; a non-square or offset viewBox would need a real viewBox transform."
        )
    extent = parts[2]

    shapes = []
    for child in el:
        tag = child.tag.rsplit("}", 1)[-1]
        if tag == "path":
            d = child.get("d")
            if not d:
                raise SystemExit(f"gen_icons: {symbol_id}: <path> with no `d`")
        elif tag == "circle":
            d = circle_to_path(
                float(child.get("cx", "0")), float(child.get("cy", "0")), float(child.get("r", "0"))
            )
        else:
            raise SystemExit(
                f"gen_icons: {symbol_id}: unsupported <{tag}>. Only <path> and <circle> are "
                f"handled; add a converter here rather than dropping the shape."
            )

        fill = parse_paint(child.get("fill", DEFAULT_FILL), "fill", symbol_id)
        stroke_paint = parse_paint(child.get("stroke", DEFAULT_STROKE), "stroke", symbol_id)
        shape = {"d": d, "fill": fill, "stroke": None}
        if stroke_paint != "none":
            shape["stroke"] = {
                "paint": stroke_paint,
                "width": float(child.get("stroke-width", "1")),
                # SVG's initial values, which is what the attribute-less paths in this
                # sheet (e.g. icon-settings') actually get in a browser.
                "cap": child.get("stroke-linecap", "butt"),
                "join": child.get("stroke-linejoin", "miter"),
            }
        shapes.append(shape)

    return {"name": name, "extent": extent, "shapes": shapes}


def rs_paint(spec: str) -> str:
    if spec == "none":
        return "IconPaint::None"
    if spec == "current":
        return "IconPaint::Current"
    r, g, b = spec.removeprefix("rgb:").split(",")
    return f"IconPaint::Rgb({r}, {g}, {b})"


def rs_cap(cap: str) -> str:
    return {"butt": "IconCap::Butt", "round": "IconCap::Round", "square": "IconCap::Square"}[cap]


def rs_join(join: str) -> str:
    return {"miter": "IconJoin::Miter", "round": "IconJoin::Round", "bevel": "IconJoin::Bevel"}[join]


def render_rs(sprites: list[dict], source: Path) -> str:
    out = [
        "// @generated by scripts/gen_icons.py — do not edit by hand.",
        f"// Source of truth: {source.relative_to(REPO)} (see that script's doc comment for why).",
        "//! The bundled `Icon` sprite set, transcribed from the sprite sheet the WebView",
        "//! backend renders, so the two backends draw the same geometry by construction",
        "//! rather than by discipline. See `scripts/gen_icons.py`.",
        "",
        "/// How one shape's fill or stroke is coloured.",
        "///",
        "/// `Current` is SVG's `currentColor` and resolves to the node's own inherited",
        "/// text colour (`ComputedStyle::text::color`) at paint time — which is what makes",
        "/// `Icon(\"text-white\", \"star\")` white rather than a fixed colour.",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
        "pub enum IconPaint {",
        "    None,",
        "    Current,",
        "    Rgb(u8, u8, u8),",
        "}",
        "",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
        "pub enum IconCap {",
        "    Butt,",
        "    Round,",
        "    Square,",
        "}",
        "",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
        "pub enum IconJoin {",
        "    Miter,",
        "    Round,",
        "    Bevel,",
        "}",
        "",
        "/// A stroke in the sprite's own viewBox units — `width` is scaled along with the",
        "/// geometry by the paint transform, exactly as SVG scales a stroke inside a",
        "/// `viewBox`, so it must not be pre-multiplied by anything here.",
        "#[derive(Debug, Clone, Copy, PartialEq)]",
        "pub struct IconStroke {",
        "    pub paint: IconPaint,",
        "    pub width: f32,",
        "    pub cap: IconCap,",
        "    pub join: IconJoin,",
        "}",
        "",
        "/// One `<path>`/`<circle>` from a sprite. `d` is SVG path data in viewBox units,",
        "/// parsed once into a `kurbo::BezPath` and cached — see `Renderer::icon`.",
        "#[derive(Debug, Clone, Copy, PartialEq)]",
        "pub struct IconShape {",
        "    pub d: &'static str,",
        "    pub fill: IconPaint,",
        "    pub stroke: Option<IconStroke>,",
        "}",
        "",
        "/// `extent` is the square viewBox's side length; the paint transform scales the",
        "/// sprite by `min(w, h) / extent` so it never distorts in a non-square box.",
        "#[derive(Debug, Clone, Copy, PartialEq)]",
        "pub struct IconSprite {",
        "    pub name: &'static str,",
        "    pub extent: f32,",
        "    pub shapes: &'static [IconShape],",
        "}",
        "",
        f"/// Every bundled sprite, in sprite-sheet order. {len(sprites)} of them.",
        "pub static SPRITES: &[IconSprite] = &[",
    ]
    for s in sprites:
        out.append("    IconSprite {")
        out.append(f'        name: "{s["name"]}",')
        out.append(f"        extent: {s['extent']:.1f},")
        out.append("        shapes: &[")
        for sh in s["shapes"]:
            out.append("            IconShape {")
            out.append(f'                d: "{sh["d"]}",')
            out.append(f"                fill: {rs_paint(sh['fill'])},")
            if sh["stroke"] is None:
                out.append("                stroke: None,")
            else:
                st = sh["stroke"]
                out.append("                stroke: Some(IconStroke {")
                out.append(f"                    paint: {rs_paint(st['paint'])},")
                out.append(f"                    width: {st['width']:.1f},")
                out.append(f"                    cap: {rs_cap(st['cap'])},")
                out.append(f"                    join: {rs_join(st['join'])},")
                out.append("                }),")
            out.append("            },")
        out.append("        ],")
        out.append("    },")
    out += [
        "];",
        "",
        "/// Looks a sprite up by the name `Icon(cls, name)` was given. `None` for an unknown",
        "/// name — a typo'd icon renders as an empty box of the right size, which is what the",
        "/// native backend did for *every* icon before this existed, rather than panicking on",
        "/// a string that came from user code.",
        "pub fn sprite(name: &str) -> Option<&'static IconSprite> {",
        "    SPRITES.iter().find(|s| s.name == name)",
        "}",
        "",
    ]
    return "\n".join(out)


def main() -> None:
    source = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_HTML
    html = source.read_text(encoding="utf-8")
    fragments = SYMBOL_RE.findall(html)
    if not fragments:
        raise SystemExit(f"gen_icons: no <symbol> elements found in {source}")

    sprites = [parse_symbol(f) for f in fragments]
    names = [s["name"] for s in sprites]
    if len(set(names)) != len(names):
        raise SystemExit(f"gen_icons: duplicate sprite names in {source}: {names}")

    OUT_JSON.write_text(
        json.dumps({"source": str(source.relative_to(REPO)), "sprites": sprites}, indent=2) + "\n",
        encoding="utf-8",
    )
    OUT_RS.write_text(render_rs(sprites, source), encoding="utf-8")
    shapes = sum(len(s["shapes"]) for s in sprites)
    print(f"gen_icons: {len(sprites)} sprites, {shapes} shapes -> {OUT_RS.relative_to(REPO)}")


if __name__ == "__main__":
    main()
