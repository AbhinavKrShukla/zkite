/**
 * Throwaway probe: what does real Chromium actually dispatch for a text `<input>`,
 * and in what order, for `input` vs `change`? phase_3/PLAN.md §26.7 records that
 * `on_input` is registered by `_register_events` on both backends and dispatched by
 * neither on native — before wiring it natively, this measures the contract the
 * WebView side already has rather than reasoning about it (§32.1's rule).
 *
 * Lives in this directory because Node resolves packages from the *script's*
 * directory and this is the only place `node_modules` exists.
 *
 *   ZKITE_CHROMIUM_PATH=/path/to/chrome node test/probe_input_change.mjs
 */

import { chromium } from "playwright";

const launchOpts = { args: ["--no-sandbox"] };
if (process.env.ZKITE_CHROMIUM_PATH) launchOpts.executablePath = process.env.ZKITE_CHROMIUM_PATH;
const browser = await chromium.launch(launchOpts);
const page = await browser.newPage();

await page.setContent(
  '<input id="f" type="text" value=""><button id="b">elsewhere</button>'
);

// Exactly zkite-runtime.js's listener shape: document-level, capture phase.
await page.evaluate(() => {
  window.log = [];
  for (const t of ["input", "change"]) {
    document.addEventListener(
      t,
      (e) => window.log.push([t, e.target.id, e.target.value]),
      true
    );
  }
});

await page.focus("#f");
await page.keyboard.type("Ken");
const afterTyping = await page.evaluate(() => window.log.slice());

// Blur by clicking elsewhere — the DOM's own "commit" moment for a text input.
await page.click("#b");
const afterBlur = await page.evaluate(() => window.log.slice());

console.log("--- after typing 'Ken' (3 keystrokes) ---");
for (const row of afterTyping) console.log("  " + JSON.stringify(row));
console.log("--- delta after blur ---");
for (const row of afterBlur.slice(afterTyping.length)) console.log("  " + JSON.stringify(row));

const typedInputs = afterTyping.filter((r) => r[0] === "input").length;
const typedChanges = afterTyping.filter((r) => r[0] === "change").length;
const blurChanges = afterBlur.slice(afterTyping.length).filter((r) => r[0] === "change").length;
console.log(
  `\nSUMMARY typing: input x${typedInputs}, change x${typedChanges} | blur: change x${blurChanges}`
);

await browser.close();
