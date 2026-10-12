/**
 * Measures what Tailwind v4 actually emits for the gradient families, by driving the
 * exact `@tailwindcss/browser` bundle `phase_1/webruntime/vendor/` pins inside real
 * Chromium. phase_3/PLAN.md §38, and the method §9.6, §28.1, §33.1, §34.1, §35.4 and
 * §36 all used: ask the source of truth, not the note.
 *
 * `twcore` has **zero** gradient parsing (phase_3/PLAN.md §15.1 item 3), so unlike
 * `probe_theme_defaults.mjs` there is no second table to compare against yet — this
 * probe exists to *write* that table, and the questions it asks are the ones a parser
 * has to answer:
 *
 *   1. What CSS does `bg-linear-to-r from-cyan-500 to-blue-500` compute to?
 *   2. Is v3's `bg-gradient-to-r` spelling still live in v4.3.3?
 *   3. Where does `via-*` put its stops, and what does it do to the other two?
 *   4. Explicit stop positions, `from-10%` / `via-30%` / `to-90%`.
 *   5. Angles: `bg-linear-45`, `bg-linear-[30deg]`.
 *   6. `bg-radial`, `bg-conic`.
 *   7. What colour space does the interpolation happen in, and can it be changed?
 *   8. `from-*` with no `bg-linear-*` at all — gradient, or nothing?
 *   9. `bg-linear-to-r` with only a `from-*` — what is the far stop?
 *  10. Does a gradient replace `bg-<color>`, or paint over it?
 *  11. The `/50` opacity modifier on a stop colour.
 *
 * Also renders one 200x100 swatch per case and reports **real pixels** at fixed
 * fractions along the gradient axis, because the computed-value answer to (7) is a
 * keyword and what a rasteriser has to match is the ramp. `getComputedStyle` is the
 * browser's own answer for a length; it is not the browser's answer for a colour ramp.
 *
 * **Lives in this directory on purpose**: Node resolves packages from the *script's*
 * directory and Playwright is installed only here (phase_3/PLAN.md's Node/Playwright
 * trap). Run it from anywhere:
 *
 *   ZKITE_CHROMIUM_PATH=$(find ~/.cache/ms-playwright/chromium-*'/chrome-linux64/chrome' | head -1) \
 *     node phase_1/webruntime/test/probe_gradients.mjs
 */

import { chromium } from "playwright";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const TAILWIND = readFileSync(join(import.meta.dirname, "../vendor/tailwind-browser.js"), "utf8");

// One case per row: the class string, and a note about which question it answers.
const CASES = [
  // (1) the canonical v4 form
  ["bg-linear-to-r from-cyan-500 to-blue-500", "v4 canonical, two stops"],
  ["bg-linear-to-l from-cyan-500 to-blue-500", "direction: to left"],
  ["bg-linear-to-t from-cyan-500 to-blue-500", "direction: to top"],
  ["bg-linear-to-b from-cyan-500 to-blue-500", "direction: to bottom"],
  ["bg-linear-to-tr from-cyan-500 to-blue-500", "direction: to top right"],
  ["bg-linear-to-tl from-cyan-500 to-blue-500", "direction: to top left"],
  ["bg-linear-to-br from-cyan-500 to-blue-500", "direction: to bottom right"],
  ["bg-linear-to-bl from-cyan-500 to-blue-500", "direction: to bottom left"],
  // (2) is v3's spelling still live?
  ["bg-gradient-to-r from-cyan-500 to-blue-500", "v3 spelling, deprecated in v4?"],
  // (3) via
  ["bg-linear-to-r from-cyan-500 via-fuchsia-500 to-blue-500", "three stops"],
  ["bg-linear-to-r via-fuchsia-500", "via alone"],
  // (4) explicit positions
  ["bg-linear-to-r from-cyan-500 from-10% to-blue-500 to-90%", "from/to positions"],
  ["bg-linear-to-r from-cyan-500 via-fuchsia-500 via-30% to-blue-500", "via position"],
  // (5) angles
  ["bg-linear-45 from-cyan-500 to-blue-500", "bare angle"],
  ["bg-linear-[30deg] from-cyan-500 to-blue-500", "arbitrary angle"],
  // (6) other gradient kinds
  ["bg-radial from-cyan-500 to-blue-500", "radial"],
  ["bg-radial-[at_25%_25%] from-cyan-500 to-blue-500", "radial with position"],
  ["bg-conic from-cyan-500 to-blue-500", "conic"],
  ["bg-conic-180 from-cyan-500 to-blue-500", "conic with angle"],
  // (7) interpolation colour space
  ["bg-linear-to-r/srgb from-cyan-500 to-blue-500", "srgb interpolation"],
  ["bg-linear-to-r/oklch from-cyan-500 to-blue-500", "oklch interpolation"],
  ["bg-linear-to-r/longer from-cyan-500 to-blue-500", "hue direction modifier"],
  // (8) stops with no gradient declared
  ["from-cyan-500 to-blue-500", "stops with no bg-linear-*"],
  // (9) a missing stop
  ["bg-linear-to-r from-cyan-500", "from only"],
  ["bg-linear-to-r to-blue-500", "to only"],
  ["bg-linear-to-r", "direction only"],
  // (10) interaction with a background colour
  ["bg-red-500 bg-linear-to-r from-cyan-500 to-blue-500", "colour + gradient"],
  ["bg-red-500 bg-linear-to-r from-cyan-500", "colour + one stop"],
  // (11) opacity modifier on a stop
  ["bg-linear-to-r from-cyan-500/50 to-blue-500", "stop with /50"],
  // and the arbitrary-value escape hatch, for completeness
  ["bg-linear-to-r from-[#ff0000] to-[#0000ff]", "arbitrary stop colours"],
  // a solid control: does a plain `bg-*` render the same bytes as the same colour used
  // as a gradient endpoint? (twcore's palette is generated from this bundle's OKLCH.)
  ["bg-cyan-500", "solid control"],
  ["bg-blue-500", "solid control"],
  // a position with no colour at all — does it define the stops var?
  ["bg-linear-to-r from-10%", "position only, no colour"],
  // a pair where Oklab and sRGB interpolation disagree loudly, so the choice of
  // interpolation space is measurable rather than a rounding argument
  ["bg-linear-to-r from-yellow-400 to-blue-600", "oklab vs srgb, loud pair"],
  ["bg-linear-to-r/srgb from-yellow-400 to-blue-600", "the same pair in srgb"],
  // v4's own "turn it off" utility
  ["bg-linear-to-r from-cyan-500 to-blue-500 bg-none", "bg-none after a gradient"],
];

// The CSS custom properties Tailwind uses to assemble a gradient. Reading these as
// well as `background-image` is what shows *how* the value is built, which is what
// tells a parser which classes are independent and which compose.
const VARS = [
  "--tw-gradient-position",
  "--tw-gradient-from",
  "--tw-gradient-via",
  "--tw-gradient-to",
  "--tw-gradient-stops",
  "--tw-gradient-via-stops",
  "--tw-gradient-from-position",
  "--tw-gradient-via-position",
  "--tw-gradient-to-position",
];

// Where to sample the rendered ramp, as a fraction of the swatch's width. 0 and 1 are
// the endpoints (which say whether the stops landed at all); the interior fractions are
// where an sRGB-interpolated ramp and an Oklab-interpolated one differ.
const SAMPLE_FRACTIONS = [0, 0.25, 0.5, 0.75, 1];
const SWATCH_W = 200;
const SWATCH_H = 100;

const html = `<!doctype html>
<html><head><script type="text/javascript">${TAILWIND}</script>
<style>
  body { margin: 0; font-size: 16px; background: #ffffff; }
  .swatch { width: ${SWATCH_W}px; height: ${SWATCH_H}px; }
</style>
</head><body>
${CASES.map(([cls], i) => `<div id="c${i}" class="swatch ${cls}"></div>`).join("\n")}
</body></html>`;

const browser = await chromium.launch({ executablePath: process.env.ZKITE_CHROMIUM_PATH });
const page = await browser.newPage({ viewport: { width: 320, height: 240 }, deviceScaleFactor: 1 });
// A `data:` URI, not `setContent` — an opaque origin cannot load a `file://`
// subresource and the bundle is inlined anyway (§35's trap).
await page.goto("data:text/html;base64," + Buffer.from(html).toString("base64"));
await page.waitForFunction(() => {
  const el = document.getElementById("c0");
  return el && getComputedStyle(el).backgroundImage !== "none";
}, null, { timeout: 15000 });

const computed = await page.evaluate(
  ({ n, vars }) => {
    const out = [];
    for (let i = 0; i < n; i++) {
      const cs = getComputedStyle(document.getElementById("c" + i));
      const row = { backgroundImage: cs.backgroundImage, backgroundColor: cs.backgroundColor, vars: {} };
      for (const v of vars) {
        const val = cs.getPropertyValue(v).trim();
        if (val) row.vars[v] = val;
      }
      out.push(row);
    }
    return out;
  },
  { n: CASES.length, vars: VARS }
);

// Pixels. One screenshot per swatch so the crop is exact and independent of layout.
const samples = [];
for (let i = 0; i < CASES.length; i++) {
  const el = page.locator("#c" + i);
  const buf = await el.screenshot({ type: "png" });
  const px = await page.evaluate(async (b64) => {
    const img = new Image();
    img.src = "data:image/png;base64," + b64;
    await img.decode();
    const cv = document.createElement("canvas");
    cv.width = img.width;
    cv.height = img.height;
    const ctx = cv.getContext("2d", { willReadFrequently: true });
    ctx.drawImage(img, 0, 0);
    const mid = Math.floor(img.height / 2);
    return { w: img.width, h: img.height, row: Array.from(ctx.getImageData(0, mid, img.width, 1).data) };
  }, buf.toString("base64"));
  const row = [];
  for (const f of SAMPLE_FRACTIONS) {
    const x = Math.min(px.w - 1, Math.max(0, Math.round(f * (px.w - 1))));
    const o = x * 4;
    row.push(`${px.row[o]},${px.row[o + 1]},${px.row[o + 2]},${px.row[o + 3]}`);
  }
  samples.push(row);
}

await browser.close();

console.log(`# Tailwind ${readFileSync(join(import.meta.dirname, "../vendor/tailwind-browser.VERSION"), "utf8").trim()}`);
console.log(`# midline pixels at x fractions ${SAMPLE_FRACTIONS.join(" / ")} of a ${SWATCH_W}x${SWATCH_H} box\n`);
for (let i = 0; i < CASES.length; i++) {
  const [cls, note] = CASES[i];
  console.log(`## ${cls}`);
  console.log(`   (${note})`);
  console.log(`   background-image: ${computed[i].backgroundImage}`);
  console.log(`   background-color: ${computed[i].backgroundColor}`);
  for (const [k, v] of Object.entries(computed[i].vars)) {
    console.log(`   ${k}: ${v}`);
  }
  console.log(`   px: ${samples[i].join("  ")}`);
  console.log("");
}
