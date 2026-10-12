// Drives real Chromium to settle where `object-fit` actually places a bitmap, so
// `zkite_renderer::object_fit_rect` is checked against the browser rather than against
// the spec text. phase_3/PLAN.md §35.1, and the same method §28.1/§33.1/§34.1 used.
//
// **Lives in this directory on purpose**: Node resolves packages from the *script's*
// directory and Playwright is installed only here (phase_3/PLAN.md's Node/Playwright
// trap). Run it from anywhere; it writes nothing outside its own tmp dir.
//
//   ZKITE_CHROMIUM_PATH=$(find ~/.cache/ms-playwright/chromium-*/chrome-linux64/chrome | head -1) \
//     node phase_1/webruntime/test/probe_object_fit.mjs
//
// Method: a 200x100 box containing a 100x100 solid-red PNG, one `object-fit` per
// arm. Screenshot the box, find the bounding box of red pixels, print it. That
// measures where Chromium *painted*, not what it computed — a `getBoundingClientRect`
// on an `<img>` returns the element box for every mode and would report five
// identical answers.
import { chromium } from "playwright";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync } from "node:child_process";

const MODES = ["fill", "contain", "cover", "none", "scale-down"];
const BOX_W = 200;
const BOX_H = 100;
// Cases chosen to make every mode distinguishable, and to include the one shape that
// separates `scale-down` from `contain` (a bitmap smaller than its box).
const CASES = [
  { name: "square-in-wide-box", img: [100, 100] },
  { name: "small-in-big-box", img: [50, 25] },
  { name: "tall-in-wide-box", img: [40, 200] },
];

const dir = mkdtempSync(join(tmpdir(), "zkite-objectfit-"));

// The bitmaps, written by the same pure-Python encoder the conformance fixtures use,
// so this probe and the fixture cannot disagree about what the image *is*.
//
// Handed to the page as `data:` URIs rather than `file://` ones: `setContent` gives
// the document an opaque origin, which cannot load a `file://` subresource at all —
// the image simply never completes, and the arm times out rather than failing with
// anything that names the cause. Found the hard way.
const py = `
import sys, base64, io, json
sys.path.insert(0, ${JSON.stringify(join(import.meta.dirname, "../../../phase_3/conformance"))})
from pngtool import solid
out = {}
${CASES.map(
  (c) =>
    `solid(${JSON.stringify(join(dir, c.name + ".png"))}, ${c.img[0]}, ${c.img[1]}, (255, 0, 0, 255))\n` +
    `out[${JSON.stringify(c.name)}] = "data:image/png;base64," + base64.b64encode(open(${JSON.stringify(join(dir, c.name + ".png"))}, "rb").read()).decode()`,
).join("\n")}
print(json.dumps(out))
`;
const dataUris = JSON.parse(execFileSync("python3", ["-c", py], { encoding: "utf8" }));

const browser = await chromium.launch({ executablePath: process.env.ZKITE_CHROMIUM_PATH });
const page = await browser.newPage({ deviceScaleFactor: 1 });

const results = [];
for (const c of CASES) {
  for (const mode of MODES) {
    // `display:block` + explicit box, white ground, no borders or margins, so the
    // screenshot's pixel (0,0) is the box's own top-left.
    await page.setContent(`<!doctype html><html><body style="margin:0;background:#fff">
      <img id="t" src="${dataUris[c.name]}"
           style="display:block;width:${BOX_W}px;height:${BOX_H}px;object-fit:${mode}">
    </body></html>`);
    await page.waitForFunction(() => {
      const i = document.getElementById("t");
      return i.complete && i.naturalWidth > 0;
    });
    const shot = join(dir, `${c.name}-${mode}.png`);
    await page.locator("#t").screenshot({ path: shot });
    results.push({ case: c.name, img: c.img, mode, shot });
  }
}
await browser.close();

// Bounding box of the red pixels, in pure Python again — the same reader the device
// measurements use.
const measure = `
import sys, json
sys.path.insert(0, ${JSON.stringify(join(import.meta.dirname, "../../../phase_3/conformance"))})
from pngtool import read_png_rgba
out = []
for shot in json.loads(sys.stdin.read()):
    w, h, px = read_png_rgba(shot)
    xs, ys = [], []
    for y in range(h):
        for x in range(w):
            r, g, b, a = px[y][x]
            # The image is pure red; anti-aliased edges blend toward white, so
            # "clearly more red than anything else" is the discriminating test.
            if a > 128 and r > 128 and g < 128 and b < 128:
                xs.append(x); ys.append(y)
    out.append([min(xs), min(ys), max(xs) - min(xs) + 1, max(ys) - min(ys) + 1] if xs else None)
print(json.dumps(out))
`;
const boxes = JSON.parse(
  execFileSync("python3", ["-c", measure], {
    input: JSON.stringify(results.map((r) => r.shot)),
    encoding: "utf8",
  }),
);

// What `object_fit_rect` says, reimplemented here *independently* so the comparison
// is a real one. Clipped to the box, because a screenshot of the element can only
// ever show the visible part and `cover`/`none` overflow.
function expected(mode, [iw, ih]) {
  let w, h;
  if (mode === "fill") {
    [w, h] = [BOX_W, BOX_H];
  } else {
    const contain = Math.min(BOX_W / iw, BOX_H / ih);
    const scale = { contain, cover: Math.max(BOX_W / iw, BOX_H / ih), none: 1, "scale-down": Math.min(contain, 1) }[mode];
    [w, h] = [iw * scale, ih * scale];
  }
  const x = (BOX_W - w) / 2;
  const y = (BOX_H - h) / 2;
  const cx = Math.max(0, x);
  const cy = Math.max(0, y);
  return [cx, cy, Math.min(BOX_W, x + w) - cx, Math.min(BOX_H, y + h) - cy].map((n) => Math.round(n));
}

console.log(`box ${BOX_W}x${BOX_H}, all numbers x,y,w,h in CSS px\n`);
let bad = 0;
for (let i = 0; i < results.length; i++) {
  const r = results[i];
  const got = boxes[i];
  const exp = expected(r.mode, r.img);
  // 1px of slack: the painted edge is anti-aliased and the threshold above puts a
  // half-covered edge pixel on one side or the other.
  const ok = got && exp.every((v, k) => Math.abs(v - got[k]) <= 1);
  if (!ok) bad++;
  console.log(
    `${ok ? "ok  " : "DIFF"} ${r.case.padEnd(20)} ${r.mode.padEnd(11)} img ${String(r.img).padEnd(8)} ` +
      `chromium ${JSON.stringify(got)}  object_fit_rect ${JSON.stringify(exp)}`,
  );
}
console.log(`\n${results.length - bad}/${results.length} agree`);
process.exit(bad === 0 ? 0 : 1);
