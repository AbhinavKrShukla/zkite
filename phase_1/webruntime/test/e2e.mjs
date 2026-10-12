/**
 * Real-Chromium (via Playwright) end-to-end check of the WebView runtime bundle:
 * loads index.html, applies HTML rendered by the actual Python `App`, and asserts
 * that (a) the vendored Tailwind JIT produces real computed CSS — not just an inert
 * class attribute — and (b) morphdom + zkite-runtime.js's onBeforeElUpdated hook
 * genuinely preserve a focused input's live value and focus across a re-render.
 *
 * (b) is not free from morphdom by default — see the finding in ../../PLAN.md §4 and
 * the comment above `preserveFocusedInputValue` in ../zkite-runtime.js. This test is
 * what caught it.
 *
 * Requires: `npm install playwright` and a Chromium binary. Point at one with
 * ZKITE_CHROMIUM_PATH, or let Playwright resolve its own if you've run
 * `npx playwright install chromium`.
 *
 *   npm install playwright
 *   ZKITE_CHROMIUM_PATH=/path/to/chrome node test/e2e.mjs
 */

import { chromium } from "playwright";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const INDEX_URL = "file://" + path.join(HERE, "..", "index.html");

const HTML_V1 =
  '<div id="r" class="flex flex-col flex-1 items-center justify-center gap-6 bg-slate-950">' +
  '<input id="probe-input" class="appearance-none outline-none border border-white/20 text-white px-3 py-2 rounded" type="text" value="" placeholder="type here">' +
  '<span id="r.1" class="text-7xl font-bold text-white tabular-nums">0</span>' +
  '<button id="r.2" class="appearance-none px-6 py-3 rounded-2xl bg-indigo-500 active:bg-indigo-600 text-white font-medium" type="button" data-zkite-on="tap">Increment</button>' +
  "</div>";

// Same tree with the counter at "1" — what a re-render after a `tap` dispatch looks
// like. `probe-input`'s value="" is intentionally stale (server-rendered), to prove
// morphdom does not clobber what the user actually typed into a live, focused input.
const HTML_V2 = HTML_V1.replace(">0<", ">1<");

const results = { checks: [], failures: [] };
function check(name, cond) {
  results.checks.push(name);
  if (!cond) results.failures.push(name);
  console.log((cond ? "PASS" : "FAIL") + " — " + name);
}

const launchOpts = { args: ["--no-sandbox"] };
if (process.env.ZKITE_CHROMIUM_PATH) {
  launchOpts.executablePath = process.env.ZKITE_CHROMIUM_PATH;
}
const browser = await chromium.launch(launchOpts);
const page = await browser.newPage();
const consoleErrors = [];
// phase_3/PLAN.md §12.1's instrumentation reaches a device by being forwarded from
// the WebView's console to logcat (`ZKiteActivity.forwardConsoleMessage`), so the
// console is the real output channel and testing it here tests the actual mechanism
// rather than a stand-in for it.
const perfLines = [];
page.on("pageerror", (e) => consoleErrors.push(String(e)));
page.on("console", (msg) => {
  if (msg.type() === "error") consoleErrors.push(msg.text());
  if (msg.text().startsWith("ZKITEPERF ")) perfLines.push(msg.text());
});

await page.goto(INDEX_URL);

await page.evaluate((html) => window.ZKiteRuntime.applyHtml(html), HTML_V1);
check(
  "initial render mounted something into #zkite-root",
  await page.$eval("#zkite-root", (el) => el.innerHTML.length > 0)
);
check(
  "counter text starts at 0",
  (await page.$eval("#r\\.1", (el) => el.textContent)) === "0"
);

const EXPECTED_BG = "oklch(0.129 0.042 264.695)"; // Tailwind v4's bg-slate-950, in CSS Color 4 oklch
await page
  .waitForFunction(
    (expected) => getComputedStyle(document.getElementById("r")).backgroundColor === expected,
    EXPECTED_BG,
    { timeout: 3000 }
  )
  .catch(() => {});
check(
  `Tailwind JIT resolved bg-slate-950 to real CSS (${EXPECTED_BG})`,
  (await page.$eval("#r", (el) => getComputedStyle(el).backgroundColor)) === EXPECTED_BG
);
check(
  'Column\'s implied "flex flex-col" is live CSS',
  (await page.$eval("#r", (el) => getComputedStyle(el).display)) === "flex" &&
    (await page.$eval("#r", (el) => getComputedStyle(el).flexDirection)) === "column"
);

await page.focus("#probe-input");
await page.keyboard.type("hello");
check(
  "typed text present before re-render",
  (await page.$eval("#probe-input", (el) => el.value)) === "hello"
);

await page.evaluate((html) => window.ZKiteRuntime.applyHtml(html), HTML_V2);

check(
  "counter text updated to 1 after re-render",
  (await page.$eval("#r\\.1", (el) => el.textContent)) === "1"
);
check(
  "typed text SURVIVED the re-render (onBeforeElUpdated preserved the live value)",
  (await page.$eval("#probe-input", (el) => el.value)) === "hello"
);
check(
  "input is STILL focused after re-render",
  (await page.evaluate(() => document.activeElement && document.activeElement.id)) === "probe-input"
);

let threw = false;
try {
  await page.click("#r\\.2");
} catch {
  threw = true;
}
check('clicking a data-zkite-on="tap" element does not throw when ZKiteNative is absent', !threw);

// ---- virtualised list scroll reporting (phase_3/PLAN.md §11) ----------------------
//
// A `List` renders only the rows in view plus two spacers standing in for everything
// above and below, and depends on the runtime reporting where it is scrolled to. Two
// things have to hold for that to work in a real browser, and neither is obvious
// enough to take on faith: the spacers must make the container's scrollable extent
// identical to a non-virtualised list's (otherwise the scrollbar and every scroll
// position are wrong), and the runtime must actually report scroll back to Python.
//
// Installed after the "ZKiteNative is absent" check above, which depends on it being
// absent. The HTML mirrors what `List._render_children` emits; the real, Python-
// generated version is covered by phase_3/conformance.
const ROW_H = 50;
const COUNT = 1000;
const WINDOW = 25;
const listHtml =
  '<div id="lst" data-zkite-virtual="1" class="flex flex-col overflow-auto min-h-0 h-[300px]">' +
  Array.from(
    { length: WINDOW },
    (_, i) =>
      `<div id="lst.i${i}" class="shrink-0 overflow-hidden h-[${ROW_H}px]">` +
      `<span id="lst.i${i}.0" class="text-white">Item ${i}</span></div>`
  ).join("") +
  `<div id="lst.pad-bottom" class="shrink-0 h-[${(COUNT - WINDOW) * ROW_H}px]"></div>` +
  "</div>";

await page.evaluate(() => {
  window.__scrollReports = [];
  window.ZKiteNative = {
    dispatch(id, event, payload) {
      if (event === "scroll") window.__scrollReports.push(`${id}|${payload}`);
    },
  };
});
await page.evaluate((html) => window.ZKiteRuntime.applyHtml(html), listHtml);

// Tailwind's JIT resolves the arbitrary heights asynchronously; wait for the real
// extent rather than racing it.
await page
  .waitForFunction((expected) => document.getElementById("lst").scrollHeight === expected, COUNT * ROW_H, {
    timeout: 3000,
  })
  .catch(() => {});
check(
  `spacers make a ${WINDOW}-row DOM scroll like a ${COUNT}-row one (${COUNT * ROW_H}px)`,
  (await page.$eval("#lst", (el) => el.scrollHeight)) === COUNT * ROW_H
);
check(
  "viewport reported to Python right after render, without waiting for a user scroll",
  (await page.evaluate(() => window.__scrollReports.some((r) => r.startsWith("lst|0,")))) === true
);

await page.evaluate(() => {
  window.__scrollReports = [];
  document.getElementById("lst").scrollTop = 5000;
});
await page.waitForFunction(() => window.__scrollReports.length > 0, { timeout: 3000 }).catch(() => {});
check(
  "scrolling reports the new position back to Python",
  (await page.evaluate(() => window.__scrollReports.some((r) => r.startsWith("lst|5000,")))) === true
);

// The render thread coalesces TouchMove for exactly this reason (PLAN.md §6.5); the
// DOM side has to do the same or a fling reports hundreds of times per second.
await page.evaluate(() => {
  window.__scrollReports = [];
  const el = document.getElementById("lst");
  for (let i = 1; i <= 20; i++) el.scrollTop = 5000 + i * 10;
});
await page.waitForFunction(() => window.__scrollReports.length > 0, { timeout: 3000 }).catch(() => {});
await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
check(
  "20 scroll events in one frame coalesce into a single report",
  (await page.evaluate(() => window.__scrollReports.length)) === 1
);

// --- change vs. bubbled tap: the ordering §26.7 left unverified ---------------
// A `Switch` inside a row that also has `on_tap` fires *both* handlers, on both
// backends — that much was verified on a device for each. What was never checked is
// the *order*, and the native side made a deliberate choice (the control's own
// `change` first, on the argument that a direct target's semantics should not depend
// on what an ancestor did to the tree) against a WebView that listens in the capture
// phase and might therefore differ.
//
// This is the right place to settle it: real Chromium, real event dispatch, no
// device. The markup is not hand-written — it is exactly what `zkite_py` emits for
// `Row(on_tap=...)` wrapping `Switch(on_change=...)`, copied from
// `Row("bg-slate-800", Switch("", checked=False, on_change=...), on_tap=...).render()`,
// so this cannot pass against a shape the framework does not actually produce.
const switchInRowHtml =
  '<div id="r" class="flex flex-row bg-slate-800" data-zkite-on="tap">' +
  '<input id="r.0" class="appearance-none flex items-center w-[44px] h-[24px] rounded-full p-[2px] ' +
  'bg-slate-700 justify-start" type="checkbox" data-zkite-on="change"></div>';

await page.evaluate(() => {
  window.__order = [];
  window.ZKiteNative = {
    dispatch(id, event, payload) {
      window.__order.push(`${id}:${event}=${payload}`);
    },
  };
});
await page.evaluate((html) => window.ZKiteRuntime.applyHtml(html), switchInRowHtml);
await page.evaluate(() => document.getElementById("r.0").click());
await page.evaluate(() => new Promise((r) => requestAnimationFrame(r)));
const order = await page.evaluate(() => window.__order);

check(
  "a tap on a switch inside a tappable row fires both handlers (Chromium)",
  order.length === 2 && order.some((e) => e.includes(":change=")) && order.some((e) => e.includes(":tap="))
);
// **A confirmed divergence, asserted in the direction it actually goes.** Chromium
// dispatches the ancestor's `tap` FIRST and the switch's own `change` second; native
// does the exact opposite (phase_3/PLAN.md §26.3). The cause is not subtle once
// measured: `zkite-runtime.js` listens for `tap` in the **capture** phase, which runs
// root -> target, so the row is reached before the input it contains — while `change`
// is a separate, later browser event. §26.7 recorded this as "an unverified
// difference, not a measured agreement"; it is now measured, and it is a difference.
//
// **This is no longer a divergence — it is the decided contract (§32.3).** It was
// pinned here "in the direction it actually goes" while which order was *correct*
// remained an open owner decision (§15.1 divergence 4b). §30.8 is what forced the
// call: the most natural way to make a checkbox label tappable netted `False` on
// native and `True` here from identical user code. The owner's ruling is that **the
// bubbled `tap` runs first and native matches this backend**, so this side is
// unchanged and `render_thread.rs::dispatch_tap` moved to agree with it.
//
// The assertion is therefore unchanged in content and changed in status: it now pins
// the contract on the backend that already satisfied it, and its native twin is the
// device case in §32.6. Kept loud so that either side flipping fails here.
check(
  "CONTRACT (§32.3): the row's bubbled tap runs BEFORE the control's own change — both backends",
  order[0].startsWith("r:tap=") && order[1] === "r.0:change=true"
);
if (results.failures.length) console.log("dispatch order:", JSON.stringify(order));

// --- Checkbox and Radio payloads in real Chromium (phase_3/PLAN.md §30) ------
// `extractPayload` is the one piece of these two widgets that only this suite can
// cover: the Python side's `_as_bool` is unit-tested and the *rendering* is covered by
// the conformance harness, but what the browser actually hands the bridge for a
// `<input type=radio>` is a fact about Blink.
//
// The regression this pins is precise, and it is §26.2's shape exactly. Before §30
// `extractPayload` special-cased `checkbox` and `range` and fell through to
// `el.value` otherwise — and a radio with no `value` attribute has the DOM default
// `"on"`, so `_as_bool("on")` is `False` and *selecting* a radio would have reported
// it deselected. The markup below is what `zkite_py` emits, not hand-written.
const togglesHtml =
  '<div id="r" class="flex flex-col">' +
  '<input id="r.0" type="checkbox" role="checkbox" data-zkite-on="change">' +
  '<input id="r.1" type="radio" role="radio" data-zkite-on="change"></div>';

await page.evaluate(() => { window.__order = []; });
await page.evaluate((html) => window.ZKiteRuntime.applyHtml(html), togglesHtml);
await page.evaluate(() => document.getElementById("r.0").click());
await page.evaluate(() => document.getElementById("r.1").click());
await page.evaluate(() => new Promise((r) => requestAnimationFrame(r)));
const toggles = await page.evaluate(() => window.__order);

check(
  "a Checkbox sends a real boolean, not a string value",
  toggles.includes("r.0:change=true")
);
check(
  "a Radio sends `true`, NOT the DOM's default value `on` (§26.2's shape)",
  toggles.includes("r.1:change=true") && !toggles.some((e) => e.includes("=on"))
);
if (results.failures.length) console.log("toggle payloads:", JSON.stringify(toggles));

// The CSS scoping half, checked here because it is a fact about real Tailwind + real
// Blink cascade rather than about ZKite: `index.html`'s switch rule was a bare
// `input[type=checkbox]` selector until §30, so a `Checkbox` inherited a 44x24 track
// and a sliding thumb. The conformance harness measures this too (checkbox 0.55% vs
// 19.05% against the unscoped control), but that runs the fixtures rather than the
// shipped stylesheet, so this asserts the rule itself.
const boxWidth = await page.evaluate(() => {
  const el = document.getElementById("r.0");
  return getComputedStyle(el).width;
});
check(
  "the switch track CSS does not leak onto a Checkbox (§30)",
  boxWidth !== "44px"
);

// --- a LABELLED Checkbox (phase_3/PLAN.md §32) --------------------------------
// The feature is "tapping the text toggles the control", and this is the backend
// where that claim is cheapest to check honestly: real Chromium, real event
// dispatch, no device.
//
// **The markup is what `zkite_py` emits**, copied from
// `Checkbox("text-sm text-white", "Wi-Fi", checked=True, on_change=...).render()` —
// same rule as the fixtures above, so this cannot pass against a shape the framework
// does not produce.
//
// What makes these checks worth writing rather than assuming: the obvious
// implementation was an HTML `<label>` wrapper, and a Chromium probe (§32.1) showed
// label activation dispatches a **second, `isTrusted` click** on the contained input.
// Under that design every assertion below that says "exactly one" would have been
// two on this backend and one on native — a new silent divergence. The composition
// that shipped has no `<label>` in it, and these are what hold that to account.
const labelledHtml =
  '<span id="c.0" class="text-sm text-white inline-flex flex-row items-center" ' +
  'role="checkbox" aria-checked="true" data-zkite-on="tap">' +
  '<input id="c.0.box" class="appearance-none flex items-center justify-center w-[20px] h-[20px] ' +
  'rounded-[4px] border-2 text-white bg-indigo-500 border-indigo-500 shrink-0 mr-2" ' +
  'type="checkbox" role="checkbox" checked aria-hidden="true" tabindex="-1">Wi-Fi</span>';

// Clicks the label *text* or the *box*, by measuring each one's own client rect —
// a `text=` selector or the container's centre would not prove which of the two was
// actually hit, and "did the tap land on the text" is the entire claim under test.
// The text node is measured with a `Range`, since a bare `#text` has no element box.
async function clickAndCollect(html, which) {
  await page.evaluate(() => { window.__order = []; });
  await page.evaluate((h) => window.ZKiteRuntime.applyHtml(h), html);
  const rects = await page.evaluate(() => {
    const box = document.getElementById("c.0.box").getBoundingClientRect();
    const span = document.getElementById("c.0");
    const textNode = [...span.childNodes].find((n) => n.nodeType === 3 && n.textContent.trim());
    const range = document.createRange();
    range.selectNodeContents(textNode);
    const text = range.getBoundingClientRect();
    const mid = (r) => ({ x: r.x + r.width / 2, y: r.y + r.height / 2 });
    return { box: mid(box), text: mid(text), disjoint: text.left >= box.right };
  });
  // Guard: if the two ever overlapped, "clicked the text" would be unfalsifiable.
  if (!rects.disjoint) throw new Error("label text and box overlap — the click target is ambiguous");
  const pt = which === "box" ? rects.box : rects.text;
  await page.mouse.click(pt.x, pt.y);
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(r)));
  return page.evaluate(() => window.__order);
}

// The whole point of the feature: §30.8 recorded that tapping the *text* did nothing.
const onText = await clickAndCollect(labelledHtml, "text");
check(
  "tapping a labelled Checkbox's TEXT dispatches the control's handler (§30.8's actual complaint)",
  onText.length === 1 && onText[0].startsWith("c.0:tap=")
);

// The box is `aria-hidden` and carries no `data-zkite-on`, so it is presentational on
// this backend exactly as it is on native. One dispatch, and no `change` — a second
// bound node here would be the double-dispatch the feature exists to remove.
const onBox = await clickAndCollect(labelledHtml, "box");
check(
  "tapping the BOX of a labelled Checkbox dispatches exactly once, and not a `change`",
  onBox.length === 1 && onBox[0].startsWith("c.0:tap=") && !onBox.some((e) => e.includes(":change="))
);
if (results.failures.length) console.log("labelled dispatches:", JSON.stringify({ onText, onBox }));

// A labelled control inside a tappable row: `findBoundAncestor` stops at the *nearest*
// bound ancestor, which is the control — and `render_thread.rs`'s tap walk breaks at
// the first `handles(id, "tap")` for the same reason. So the row's handler does NOT
// also fire, on either backend. That is the structural reason §30.8's ordering trap
// cannot be reached from the labelled form: there is never a second handler to order.
const labelledInRowHtml =
  '<div id="row" class="flex flex-row" data-zkite-on="tap">' + labelledHtml + "</div>";
const inRow = await clickAndCollect(labelledInRowHtml, "text");
check(
  "a labelled control absorbs the tap — the ancestor row's handler does not also fire (§32.3)",
  inRow.length === 1 && inRow[0].startsWith("c.0:tap=")
);
if (results.failures.length) console.log("labelled-in-row:", JSON.stringify(inRow));

// --- a flex-1 List, i.e. the shape a real app actually has -------------------
// The fixture above hardcodes `h-[300px]` on the list container, so it is bounded by
// construction and cannot exercise what a real app does: `List("flex-1")` inside a
// `flex-1` root, with no explicit height anywhere. That difference hid a bug that
// only appeared on a device (phase_3/PLAN.md §12) — `#zkite-root { min-height: 100% }`
// lets the root grow past the viewport, so a flex-1 list's own box grows with its
// content, the *larger* box gets reported back to Python as the viewport, Python
// widens the window to fill it, and the list grows again. On a real device this never
// converged: 268 renders, DOM climbing 3,881 -> 10,361 nodes, 223ms per render and
// still going.
//
// The invariant that has to hold is simply that the scroll container is bounded by
// the viewport. §11.3's argument that the feedback loop terminates ("a list's content
// height doesn't depend on its window") is true only if that holds.
const realShapeRows = 40;
const realShapeHtml =
  '<div id="rs" class="flex flex-col flex-1 min-h-0 bg-slate-950">' +
  '<div id="rs.h" class="shrink-0 py-4"><span class="text-white">header</span></div>' +
  '<div id="rs.l" data-zkite-virtual="1" class="flex flex-col overflow-auto min-h-0 flex-1">' +
  Array.from(
    { length: realShapeRows },
    (_, i) =>
      `<div id="rs.l.i${i}" class="shrink-0 overflow-hidden h-[${ROW_H}px]">` +
      `<span class="text-white">Item ${i}</span></div>`
  ).join("") +
  `<div id="rs.l.pad-bottom" class="shrink-0 h-[${(COUNT - realShapeRows) * ROW_H}px]"></div>` +
  "</div></div>";

await page.evaluate((html) => window.ZKiteRuntime.applyHtml(html), realShapeHtml);
await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
const bounded = await page.evaluate(() => ({
  inner: window.innerHeight,
  list: document.getElementById("rs.l").clientHeight,
  root: document.getElementById("zkite-root").getBoundingClientRect().height,
}));
check(
  `a flex-1 List's scroll container stays within the viewport (list ${bounded.list} <= inner ${bounded.inner})`,
  bounded.list > 0 && bounded.list <= bounded.inner
);
check(
  `#zkite-root does not grow past the viewport with a tall list inside (root ${Math.round(bounded.root)} <= inner ${bounded.inner})`,
  bounded.root <= bounded.inner + 1
);

// --- §12.1 instrumentation --------------------------------------------------
// The WebView backend had no per-frame timing of any kind, which is why phase 3's
// §12 backend comparison could not be run even in principle. These check the three
// numbers §12.1 names actually get emitted, on the channel that carries them to a
// device.

// The sampler stops SCROLL_IDLE_MS (250) after the last scroll event and reports
// then; the scrolls above are what armed it.
await new Promise((r) => setTimeout(r, 600));

const firstPaintLines = perfLines.filter((l) => l.startsWith("ZKITEPERF first-paint"));
check(
  "cold-start: exactly one first-paint line, no matter how many renders followed",
  firstPaintLines.length === 1
);
check(
  "cold-start line carries a page-relative time and a node count",
  /page_rel=\d+\.\d+ms nodes=\d+/.test(firstPaintLines[0] || "")
);

const scrollFrameLines = perfLines.filter((l) => l.startsWith("ZKITEPERF scroll-frames"));
check(
  "scrolling emits a frame-interval distribution (not a bare mean — jank is a tail property)",
  scrollFrameLines.length >= 1 && /n=\d+ avg [\d.]+ p50 [\d.]+ p95 [\d.]+ max [\d.]+/.test(scrollFrameLines[0])
);

// The re-render summary is the WebView analogue of the native backend's
// `redraw x60` line, and fires on the same 60-frame interval.
const rendersBefore = perfLines.filter((l) => l.startsWith("ZKITEPERF render x")).length;
for (let i = 0; i < 61; i++) {
  await page.evaluate((html) => window.ZKiteRuntime.applyHtml(html), i % 2 ? HTML_V1 : HTML_V2);
}
await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
await new Promise((r) => setTimeout(r, 300));
const renderLines = perfLines.filter((l) => l.startsWith("ZKITEPERF render x"));
check(
  "60 re-renders emit one summary with an average, a max and a node count",
  renderLines.length > rendersBefore &&
    /render x\d+: avg [\d.]+ms max [\d.]+ms \(apply avg [\d.]+ms, \d+ nodes\)/.test(renderLines[renderLines.length - 1])
);

// --- Select's overlay layer, in real Chromium (phase_3/PLAN.md §33) ---------------
//
// The one claim behind the whole feature is that "paints above everything" needs no
// renderer support, because both backends already agree that a node emitted later in
// document order is drawn on top and hit first. On the native side that is
// `Renderer::hit_test`'s reverse walk, which a Rust test can check. On *this* side it
// is CSS painting order, which cannot be checked by reading a class string — CSS
// paints positioned elements above non-positioned ones regardless of tree order, so
// the panel carries `relative` for exactly that reason and nothing but a real engine
// can confirm it worked. §32.1's rule: ask the source of truth, not the note.
//
// The HTML is rendered by the actual Python `App`, not transcribed, so this cannot
// pass against markup the widget no longer emits.
const selectHtml = execFileSync(
  "python3.14",
  [
    "-c",
    [
      "import sys, types",
      "sys.path.insert(0, sys.argv[1])",
      "from zkite.app import App",
      "src = open(sys.argv[2]).read()",
      "m = types.ModuleType('main'); exec(src, m.__dict__)",
      "app = App(m); app.render(); app.dispatch('r.1', 'tap', '')",
      "sys.stdout.write(app.last_html)",
    ].join("\n"),
    path.join(HERE, "..", "..", "zkite_py", "src"),
    path.join(HERE, "select_fixture.py"),
  ],
  { encoding: "utf8" }
);

await page.evaluate((html) => window.ZKiteRuntime.applyHtml(html), selectHtml);
await page.waitForFunction(
  () => getComputedStyle(document.getElementById("__zkite_overlay.0")).position === "fixed",
  { timeout: 5000 }
).catch(() => {});

const layer = await page.evaluate(() => {
  const rect = (id) => document.getElementById(id).getBoundingClientRect();
  const panel = rect("__zkite_overlay.0.1");
  const scrim = rect("__zkite_overlay.0.0");
  const at = (x, y) => {
    const el = document.elementFromPoint(x, y);
    return el ? el.id || el.tagName : null;
  };
  const inside = (r) => [r.left + r.width / 2, r.top + r.height / 2];
  return {
    scrimFillsViewport:
      Math.round(scrim.width) === window.innerWidth && Math.round(scrim.height) === window.innerHeight,
    panelInsideScrim: panel.width > 0 && panel.width < scrim.width,
    atPanelCentre: at(...inside(panel)),
    atOptionCentre: at(...inside(rect("r.1.opt.1"))),
    atTopLeftCorner: at(4, 4),
    panelPosition: getComputedStyle(document.getElementById("__zkite_overlay.0.1")).position,
  };
});

check(
  "the layer's scrim fills the viewport and the panel sits inside it",
  layer.scrimFillsViewport && layer.panelInsideScrim
);
check(
  "the panel is hit-tested ABOVE its own absolutely-positioned scrim",
  layer.panelPosition === "relative" &&
    layer.atPanelCentre !== "__zkite_overlay.0.0" &&
    layer.atOptionCentre.startsWith("r.1.opt.1")
);
check("a point outside the panel lands on the scrim", layer.atTopLeftCorner === "__zkite_overlay.0.0");

// What the runtime actually dispatches, through the real delegated listener.
await page.evaluate(() => {
  window.__dispatches = [];
  window.ZKiteNative = {
    dispatch: (id, event, payload) => window.__dispatches.push([id, event, payload]),
  };
});
await page.click("#__zkite_overlay\\.0\\.1"); // the panel itself
await page.click("#r\\.1\\.opt\\.2"); // an option
await page.click("body", { position: { x: 4, y: 4 } }); // outside: the scrim
const dispatches = await page.evaluate(() => window.__dispatches);

check(
  "a click on the panel dispatches nothing — no bound ancestor, and no no-op tap to absorb one",
  dispatches.filter((d) => d[0].startsWith("__zkite_overlay.0.1")).length === 0
);
check(
  "a click on an option dispatches its own id, and the payload is EMPTY",
  dispatches.some((d) => d[0] === "r.1.opt.2" && d[1] === "tap" && d[2] === "")
);
check(
  "a click outside the panel dispatches the scrim's dismiss",
  dispatches.some((d) => d[0] === "__zkite_overlay.0.0" && d[1] === "tap")
);
if (results.failures.length) console.log("layer debug:", JSON.stringify({ layer, dispatches }));
await page.evaluate(() => delete window.ZKiteNative);

// --- bundled typefaces (phase_3/PLAN.md §9.4) -------------------------------
// The two backends have to render the *same* typefaces, not merely both render
// something. Before this, `font-sans` resolved to whatever the host called
// "sans-serif" (Noto Sans here, Roboto on a device) while the native backend used
// whichever single font it was handed — so the same app looked different per
// backend and per phone. index.html now bundles three families via @font-face and
// points Tailwind's own --font-* theme variables at them.
//
// Checked three ways, because each alone can pass with the bug present: the
// @font-face rules must actually *load* (a missing fonts/ directory leaves them
// "unloaded" and silently falls back), the computed font-family must name them
// (proving the @theme block reached Tailwind rather than being ignored), and the
// three families must measure the same string differently (proving the glyphs
// really came from three different files, not one aliased three ways).
// Inject first, *then* wait: browsers load an @font-face lazily (only once a glyph
// actually needs it), and Tailwind's JIT recompiles asynchronously after a DOM
// mutation — so measuring immediately reports the previous frame's fonts and no
// font-serif/font-mono rules at all. Both were hit while writing this check.
await page.evaluate(() => {
  // A single word can have the same rounded width in two different fonts.
  // Include narrow and wide glyphs so this detects accidental font aliasing.
  const sample = "iiiiiiii WWWWWWWW 0123456789 Hamburgefonstiv";
  document.getElementById("zkite-root").innerHTML =
    `<span id="f-sans" class="font-sans text-2xl">${sample}</span>` +
    `<span id="f-serif" class="font-serif text-2xl">${sample}</span>` +
    `<span id="f-mono" class="font-mono text-2xl">${sample}</span>`;
});
await page.waitForFunction(
  () => getComputedStyle(document.getElementById("f-mono")).fontFamily.includes("Mono"),
  { timeout: 5000 }
).catch(() => {});
const fonts = await page.evaluate(async () => {
  await document.fonts.ready;
  const read = (id) => {
    const el = document.getElementById(id);
    // Range-measure the text, not the element: #zkite-root is a flex column, so
    // every span stretches to the container width and el.getBoundingClientRect()
    // reports an identical 1280px for all three regardless of typeface — a tie
    // that would have made this check pass or fail for reasons unrelated to fonts.
    const range = document.createRange();
    range.selectNodeContents(el);
    return {
      family: getComputedStyle(el).fontFamily,
      width: range.getBoundingClientRect().width,
    };
  };
  return {
    statuses: [...document.fonts].map((f) => f.status),
    faceCount: document.fonts.size,
    sans: read("f-sans"),
    serif: read("f-serif"),
    mono: read("f-mono"),
  };
});
check(
  "all six bundled font faces are declared and the three in use loaded",
  fonts.faceCount === 6 && fonts.statuses.filter((s) => s === "loaded").length >= 3
);
check(
  "font-sans/serif/mono resolve to the bundled families, not the platform's",
  fonts.sans.family.includes("ZKite Sans") &&
    fonts.serif.family.includes("ZKite Serif") &&
    fonts.mono.family.includes("ZKite Mono")
);
check(
  "the three families really are three different typefaces (distinct measured widths)",
  new Set([
    fonts.sans.width.toFixed(2),
    fonts.serif.width.toFixed(2),
    fonts.mono.width.toFixed(2),
  ]).size === 3
);
if (results.failures.length) console.log("font debug:", JSON.stringify(fonts));

// ---- the `input` vs `change` contract on a text field (phase_3/PLAN.md §34.1) ----
//
// §26.7 recorded that `on_input` is registered by `_register_events` on both backends
// and dispatched by neither on native. Before wiring it natively this was *measured*
// here rather than reasoned about (§32.1's rule), and it is pinned so a silent flip
// — in Blink, or in `zkite-runtime.js`'s own listeners — fails the suite:
//
//   typing "Ken" -> input x3, change x0.   blurring -> change x1.
//
// The native backend now dispatches `input` per keystroke to match the first half.
// It deliberately still dispatches `change` per keystroke too, which the second half
// says the WebView does not; that residual divergence is named in §15.1 rather than
// fixed, because retiming `change` would break every native app that uses it for live
// typing (WIDGETS.md documents it that way, and §33.6's typeahead is built on it).
await page.evaluate(() => {
  window.__evts = [];
  for (const t of ["input", "change"]) {
    document.addEventListener(t, (e) => window.__evts.push(t + ":" + e.target.id), true);
  }
});
await page.evaluate(() =>
  window.ZKiteRuntime.applyHtml(
    '<div id="r" class="flex flex-col"><input id="tf" type="text" value="">' +
      '<button id="elsewhere" type="button">x</button></div>'
  )
);
await page.focus("#tf");
await page.keyboard.type("Ken");
const whileTyping = await page.evaluate(() => window.__evts.slice());
await page.click("#elsewhere");
const afterBlur = await page.evaluate(() => window.__evts.slice(window.__typedLen ?? 0));

check(
  "typing 3 characters into a TextField fires `input` 3 times",
  whileTyping.filter((e) => e === "input:tf").length === 3
);
check(
  "typing into a TextField fires no `change` at all — it arrives on blur",
  whileTyping.filter((e) => e.startsWith("change:")).length === 0 &&
    afterBlur.filter((e) => e === "change:tf").length === 1
);

check("no console/page errors during the whole sequence", consoleErrors.length === 0);
if (consoleErrors.length) console.log("console errors:", consoleErrors);

await browser.close();

const failed = results.failures.length;
console.log(`\n${results.checks.length - failed}/${results.checks.length} checks passed`);
if (failed) {
  console.log("FAILED:", results.failures);
  process.exit(1);
}
