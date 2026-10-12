/**
 * Phase 3 §5 item 5 — the WebView half of per-widget conformance testing. Loads the
 * real `phase_1/webruntime/index.html` shell (vendored Tailwind v4 JIT + morphdom,
 * unmodified from phase 1) in real Chromium via Playwright — same tool/pattern
 * `phase_1/webruntime/test/e2e.mjs` already established — applies each fixture's
 * `Widget.render()` HTML fragment via the real `window.ZKiteRuntime.applyHtml`, and
 * screenshots *only* the fixture's own root element (`#r`, cropped to its actual
 * rendered box) so the result is directly comparable to `conformance render-all`'s
 * native PNG, which rasterizes to exactly that fixture's own declared width/height —
 * see fixtures.py's module doc for why neither side depends on its shell's own
 * root-stretch behavior.
 *
 *   node screenshot_web.mjs manifest.json out/ [dpr]
 *
 * `dpr` (default 1) is Playwright's `deviceScaleFactor`, which is the browser's own
 * device pixel ratio — CSS pixels keep their size and the backing store grows, which
 * is exactly what a real phone's WebView does with `initial-scale=1`. Passing it here
 * and the same number to `conformance render-all` is what lets this harness see a
 * density bug at all; at dpr 1 the two backends agreed by construction and ../PLAN.md
 * §9.3 recorded that as a structural blind spot, which ../PLAN.md §12.5.2 then found
 * on a device the hard way. See ../PLAN.md §13.
 */
import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const INDEX_URL = "file://" + path.join(HERE, "..", "..", "phase_1", "webruntime", "index.html");

const [manifestPath, outDir, dprArg] = process.argv.slice(2);
if (!manifestPath || !outDir) {
  console.error("usage: node screenshot_web.mjs <manifest.json> <out_dir> [dpr]");
  process.exit(2);
}
const dpr = dprArg ? Number(dprArg) : 1;
if (!Number.isFinite(dpr) || dpr <= 0) {
  console.error(`bad dpr: ${dprArg}`);
  process.exit(2);
}
const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
fs.mkdirSync(outDir, { recursive: true });

const launchOpts = { args: ["--no-sandbox"] };
if (process.env.ZKITE_CHROMIUM_PATH) launchOpts.executablePath = process.env.ZKITE_CHROMIUM_PATH;

const browser = await chromium.launch(launchOpts);
// `deviceScaleFactor` belongs to the *context*, not the page, and cannot be changed
// per-screenshot — which is why a second dpr is a second whole run rather than an
// extra loop inside this one.
const context = await browser.newContext({ deviceScaleFactor: dpr });
const page = await context.newPage();
const pageErrors = [];
page.on("pageerror", (e) => pageErrors.push(String(e)));

let failures = 0;
for (const [name, fixture] of Object.entries(manifest)) {
  await page.setViewportSize({ width: Math.max(fixture.width, 50), height: Math.max(fixture.height, 50) });
  await page.goto(INDEX_URL);
  // Real Tailwind JIT compiles classes asynchronously off a MutationObserver — wait
  // for the fixture's own background color to actually resolve to real CSS (not just
  // the class attribute existing) before screenshotting, the same discipline
  // e2e.mjs's own `waitForFunction` on `bg-slate-950` already established, generalized
  // to "some real background landed" rather than one hardcoded expected color since
  // fixtures use several different backgrounds.
  await page.evaluate((html) => window.ZKiteRuntime.applyHtml(html), fixture.html);
  await page
    .waitForFunction(
      () => {
        const el = document.getElementById("r");
        return el && getComputedStyle(el).backgroundColor !== "rgba(0, 0, 0, 0)";
      },
      { timeout: 3000 }
    )
    .catch(() => {});

  const outPath = path.join(outDir, `${name}_web.png`);
  const el = await page.$("#r");
  if (!el) {
    console.error(`FAIL ${name}: #r not found in DOM after applyHtml`);
    failures++;
    continue;
  }
  await el.screenshot({ path: outPath });
  // `boundingBox()` is in CSS pixels, so this stays a dp-space check at any dpr —
  // which is the right assertion: the fixture's declared size is a CSS-pixel size and
  // must not change with density. The *file* it just wrote is dpr times that on each
  // axis, which is what `conformance diff-all` compares against the native canvas.
  const box = await el.boundingBox();
  const okSize = box && Math.round(box.width) === fixture.width && Math.round(box.height) === fixture.height;
  console.log(
    `${okSize ? "ok  " : "SIZE"} ${name} -> ${outPath} (rendered ${box ? `${Math.round(box.width)}x${Math.round(box.height)}` : "?"} css px @${dpr}x, expected ${fixture.width}x${fixture.height})`
  );
  if (!okSize) failures++;
}

if (pageErrors.length) {
  console.error("page errors during the run:", pageErrors);
  failures += pageErrors.length;
}

await browser.close();
console.log(`\n${Object.keys(manifest).length - failures}/${Object.keys(manifest).length} fixtures screenshotted cleanly`);
if (failures) process.exit(1);
