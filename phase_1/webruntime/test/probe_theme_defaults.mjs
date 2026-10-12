/**
 * Audits every remaining `twcore::Theme` default against Tailwind v4, by driving the
 * exact `@tailwindcss/browser` bundle `phase_1/webruntime/vendor/` pins inside real
 * Chromium and reading computed styles. phase_3/PLAN.md §36, and the method §9.6,
 * §28.1, §33.1, §34.1 and §35.4 all used: ask the source of truth, not the note.
 *
 * §28.6's own known-gaps entry is what this closes:
 *
 *   > Nothing re-checks that `twcore`'s remaining v4 defaults are v4's. §9.6's own
 *   > list also names the spacing scale derivation, which nothing here measured.
 *
 * **Lives in this directory on purpose**: Node resolves packages from the *script's*
 * directory and Playwright is installed only here (phase_3/PLAN.md's Node/Playwright
 * trap). Run it from anywhere:
 *
 *   ZKITE_CHROMIUM_PATH=$(find ~/.cache/ms-playwright/chromium-*'/chrome-linux64/chrome' | head -1) \
 *     node phase_1/webruntime/test/probe_theme_defaults.mjs
 *
 * Method notes:
 *
 * - The bundle is **inlined** rather than loaded with a `<script src>`. `setContent`
 *   gives the document an opaque origin, which cannot load a `file://` subresource at
 *   all — the load never completes and the arm times out without naming the cause
 *   (§35's Node/Playwright trap).
 * - Computed styles, not screenshots. Every value here is a length or a number that
 *   Chromium resolves to px, so `getComputedStyle` *is* the browser's own answer;
 *   §35's probe needed pixels only because `object-fit` does not change any computed
 *   property.
 * - `font-size: 16px` is pinned **on `body`, as inherited**, because `em` and the
 *   unitless `line-height`/`letter-spacing` values resolve against the element's own
 *   font size. That 16 is `Theme::root_font_size`, so the comparison is like-for-like.
 *   It must not be an *inline* style on the probe elements: an inline declaration
 *   outranks the utility class, so every `text-*` arm reads back 16px and the probe
 *   reports a divergence in itself. That is how this was first written, and the
 *   13 rows of confident nonsense it produced are why this paragraph is here.
 * - The `twcore` column is transcribed from `phase_0/spike3_twcore/src/theme.rs`.
 *   It is duplicated here deliberately: the point of the probe is to compare two
 *   independently-written tables, and generating one from the other would compare a
 *   table with itself. `tests/utilities.rs` is what pins the Rust side.
 */

import { chromium } from "playwright";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const TAILWIND = readFileSync(join(import.meta.dirname, "../vendor/tailwind-browser.js"), "utf8");
const ROOT_FONT_SIZE = 16; // twcore Theme::root_font_size

// ---------------------------------------------------------------------------
// The twcore side, transcribed from src/theme.rs. `null` means "twcore does not
// parse this token at all" — which is a divergence too, and a quieter one.
// ---------------------------------------------------------------------------

const TWCORE = {
  // Theme::spacing — `n * spacing_unit`, spacing_unit = 4.0, plus the `px` special case.
  spacing: {
    "0": 0, "0.5": 2, "1": 4, "1.5": 6, "2": 8, "2.5": 10, "3": 12, "3.5": 14,
    "4": 16, "5": 20, "6": 24, "7": 28, "8": 32, "9": 36, "10": 40, "11": 44,
    "12": 48, "14": 56, "16": 64, "20": 80, "24": 96, "28": 112, "32": 128,
    "36": 144, "40": 160, "44": 176, "48": 192, "52": 208, "56": 224, "60": 240,
    "64": 256, "72": 288, "80": 320, "96": 384,
    // v4 allows any multiple, including ones v3's fixed scale never had.
    "4.5": 18, "13": 52, "17": 68,
    px: 1,
  },
  // Theme::radius
  radius: {
    none: 0, xs: null, sm: 2, "": 4, md: 6, lg: 8, xl: 12,
    "2xl": 16, "3xl": 24, "4xl": null, full: 9999,
  },
  // Theme::font_size — [size, line-height in px, or {rel} for a unitless one]
  fontSize: {
    xs: [12, 16], sm: [14, 20], base: [16, 24], lg: [18, 28], xl: [20, 28],
    "2xl": [24, 32], "3xl": [30, 36], "4xl": [36, 40],
    "5xl": [48, { rel: 1 }], "6xl": [60, { rel: 1 }], "7xl": [72, { rel: 1 }],
    "8xl": [96, { rel: 1 }], "9xl": [128, { rel: 1 }],
  },
  // Theme::font_weight
  fontWeight: {
    thin: 100, extralight: 200, light: 300, normal: 400, medium: 500,
    semibold: 600, bold: 700, extrabold: 800, black: 900,
  },
  // Theme::leading — relative values, plus the numeric fallthrough onto the spacing scale.
  leading: {
    none: { rel: 1 }, tight: { rel: 1.25 }, snug: { rel: 1.375 },
    normal: { rel: 1.5 }, relaxed: { rel: 1.625 }, loose: { rel: 2 },
    "6": 24, "8": 32, "10": 40, // numeric: spacing scale, leading-6 == 24dp
  },
  // Theme::tracking — returned in em.
  tracking: {
    tighter: -0.05, tight: -0.025, normal: 0, wide: 0.025, wider: 0.05, widest: 0.1,
  },
  // Theme::breakpoint / Screens::default
  breakpoint: { sm: 640, md: 768, lg: 1024, xl: 1280, "2xl": 1536 },
};

// ---------------------------------------------------------------------------
// Drive Chromium.
// ---------------------------------------------------------------------------

const launchOpts = { args: ["--no-sandbox"] };
if (process.env.ZKITE_CHROMIUM_PATH) launchOpts.executablePath = process.env.ZKITE_CHROMIUM_PATH;
const browser = await chromium.launch(launchOpts);
const page = await browser.newPage({ deviceScaleFactor: 1, viewport: { width: 1600, height: 900 } });

// One element per class, all present before the JIT runs.
const specs = [];
const add = (group, token, cls, prop) => specs.push({ group, token, cls, prop });

for (const t of Object.keys(TWCORE.spacing)) add("spacing", t, `p-${t}`, "paddingTop");
for (const t of Object.keys(TWCORE.radius)) {
  add("radius", t, t === "" ? "rounded" : `rounded-${t}`, "borderTopLeftRadius");
}
for (const t of Object.keys(TWCORE.fontSize)) add("fontSize", t, `text-${t}`, "fontSize+lineHeight");
for (const t of Object.keys(TWCORE.fontWeight)) add("fontWeight", t, `font-${t}`, "fontWeight");
for (const t of Object.keys(TWCORE.leading)) add("leading", t, `leading-${t}`, "lineHeight");
for (const t of Object.keys(TWCORE.tracking)) add("tracking", t, `tracking-${t}`, "letterSpacing");

const html =
  `<!doctype html><html><head><style>` +
  // Inherited, not inline — see the header. A declaration on `body` sets what the
  // probe elements inherit and loses to any `text-*` they declare themselves.
  `html,body{margin:0;font-size:${ROOT_FONT_SIZE}px}</style></head><body>` +
  specs.map((s, i) => `<div id="p${i}" class="${s.cls}">x</div>`).join("") +
  // Breakpoint probes: a variant whose *base* value is unset, so "did it apply" is
  // unambiguous. Read at a series of viewport widths further down.
  Object.keys(TWCORE.breakpoint)
    .map((b) => `<div id="bp-${b}" class="${b}:p-7">x</div>`)
    .join("") +
  `</body></html>`;

await page.setContent(html);
await page.addScriptTag({ content: TAILWIND });

// The JIT is asynchronous — it scans, compiles and injects a <style>. Wait for a
// value it must produce rather than for a timeout.
await page.waitForFunction(
  () => {
    const e = document.querySelector(".p-4");
    return e && getComputedStyle(e).paddingTop === "16px";
  },
  null,
  { timeout: 15000 },
);

const computed = await page.evaluate(
  (n) =>
    Array.from({ length: n }, (_, i) => {
      const cs = getComputedStyle(document.getElementById("p" + i));
      return {
        paddingTop: cs.paddingTop,
        borderTopLeftRadius: cs.borderTopLeftRadius,
        fontSize: cs.fontSize,
        lineHeight: cs.lineHeight,
        fontWeight: cs.fontWeight,
        letterSpacing: cs.letterSpacing,
      };
    }),
  specs.length,
);

// Breakpoints: for each, the width where the variant switches on. Binary-searching
// would be faster and would also hide an off-by-one at a width nobody sampled, so
// this samples the two widths that matter, either side of the claimed value.
const breakpoints = {};
for (const [name, claimed] of Object.entries(TWCORE.breakpoint)) {
  const at = {};
  for (const w of [claimed - 1, claimed]) {
    await page.setViewportSize({ width: w, height: 900 });
    at[w] = await page.evaluate(
      (n) => getComputedStyle(document.getElementById("bp-" + n)).paddingTop,
      name,
    );
  }
  breakpoints[name] = at;
}

await browser.close();

// ---------------------------------------------------------------------------
// Compare.
// ---------------------------------------------------------------------------

const px = (s) => (s === "normal" ? null : parseFloat(s));
const fmt = (v) =>
  v === null || v === undefined
    ? "(unparsed)"
    : typeof v === "object"
      ? `x${v.rel}`
      : String(Math.round(v * 1000) / 1000);

let diffs = 0;
let unparsed = 0;
const rows = [];

function row(group, token, cls, ours, theirs, note = "") {
  // `ours === null` is "twcore does not parse it": reported separately from a
  // numeric disagreement, because the failure modes differ — an unparsed class is
  // dropped silently, a wrong number renders confidently.
  let status;
  if (ours === null) {
    status = "MISSING";
    unparsed++;
  } else if (theirs === null || theirs === undefined) {
    status = "n/a";
  } else {
    const a = typeof ours === "object" ? ours.rel : ours;
    const b = typeof theirs === "object" ? theirs.rel : theirs;
    const sameKind = typeof ours === "object" === (typeof theirs === "object");
    const ok = sameKind && Math.abs(a - b) < 0.005;
    status = ok ? "ok" : "DIFF";
    if (!ok) diffs++;
  }
  rows.push({ group, token, cls, ours: fmt(ours), theirs: fmt(theirs), status, note });
}

specs.forEach((s, i) => {
  const c = computed[i];
  if (s.group === "spacing") row("spacing", s.token, s.cls, TWCORE.spacing[s.token], px(c.paddingTop));
  else if (s.group === "radius") {
    // `rounded-full` is `calc(infinity * 1px)` in v4; Chromium computes an enormous
    // finite px. Both sides mean "a pill", so compare the intent, not the number.
    const got = px(c.borderTopLeftRadius);
    const ours = TWCORE.radius[s.token];
    if (s.token === "full") {
      row("radius", s.token, s.cls, ours, ours, `chromium: ${c.borderTopLeftRadius} (both mean "pill")`);
    } else row("radius", s.token, s.cls, ours, got);
  } else if (s.group === "fontSize") {
    const [size, lh] = TWCORE.fontSize[s.token];
    row("fontSize", s.token, s.cls, size, px(c.fontSize));
    // v4 states these as `calc(a / b)`, i.e. unitless, which Chromium resolves to px
    // against the element's own font-size. twcore stores Absolute px for the small
    // end and Relative for 5xl+ — so compare in px on both sides.
    const oursPx = typeof lh === "object" ? lh.rel * size : lh;
    row("fontSize", s.token + " lh", s.cls, oursPx, px(c.lineHeight));
  } else if (s.group === "fontWeight")
    row("fontWeight", s.token, s.cls, TWCORE.fontWeight[s.token], parseFloat(c.fontWeight));
  else if (s.group === "leading") {
    const ours = TWCORE.leading[s.token];
    const oursPx = typeof ours === "object" ? ours.rel * ROOT_FONT_SIZE : ours;
    row("leading", s.token, s.cls, oursPx, px(c.lineHeight));
  } else if (s.group === "tracking")
    row("tracking", s.token, s.cls, TWCORE.tracking[s.token] * ROOT_FONT_SIZE, px(c.letterSpacing) ?? 0);
});

const w = (s, n) => String(s).padEnd(n);
console.log(
  `Tailwind v4 (vendored @tailwindcss/browser, real Chromium) vs twcore::Theme\n` +
    `root font-size ${ROOT_FONT_SIZE}px; lengths in px, tracking in px, leading resolved to px\n`,
);
console.log(`${w("", 4)} ${w("group", 10)} ${w("class", 16)} ${w("twcore", 12)} ${w("v4", 12)} note`);
let lastGroup = null;
for (const r of rows) {
  if (r.group !== lastGroup) {
    console.log("");
    lastGroup = r.group;
  }
  const mark = r.status === "ok" ? "ok  " : r.status === "n/a" ? "--  " : r.status.slice(0, 4);
  console.log(`${w(mark, 4)} ${w(r.group, 10)} ${w(r.cls, 16)} ${w(r.ours, 12)} ${w(r.theirs, 12)} ${r.note}`);
}

console.log("\n     breakpoint (does `<bp>:p-7` apply? 28px = yes, 0px = no)");
let bpDiffs = 0;
for (const [name, claimed] of Object.entries(TWCORE.breakpoint)) {
  const at = breakpoints[name];
  const below = px(at[claimed - 1]);
  const atMin = px(at[claimed]);
  const ok = below === 0 && atMin === 28;
  if (!ok) bpDiffs++;
  console.log(
    `${w(ok ? "ok" : "DIFF", 4)} ${w("breakpoint", 10)} ${w(`${name}:p-7`, 16)} ` +
      `${w(claimed + "px", 12)} ${w(`${claimed - 1}px -> ${at[claimed - 1]}, ${claimed}px -> ${at[claimed]}`, 12)}`,
  );
}

const total = diffs + bpDiffs;
console.log(
  `\n${rows.length + Object.keys(TWCORE.breakpoint).length} values compared: ` +
    `${total} disagree, ${unparsed} unparsed by twcore`,
);
process.exit(total === 0 && unparsed === 0 ? 0 : 1);
