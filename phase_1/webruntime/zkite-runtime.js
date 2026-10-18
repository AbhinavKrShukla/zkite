/**
 * zkite-runtime.js — the device-side half of the WebView backend.
 *
 * Responsibilities (see ../PLAN.md §2, §4):
 *   1. Take an HTML string rendered by Python (`zkite.app.App.render()`) and patch the
 *      live DOM with `morphdom`, which preserves focus/scroll/input-value on any node
 *      that didn't change — this is the whole mechanism behind "keyboard state and
 *      scroll position intact" (plan1.md §6), not something built by hand here.
 *   2. Delegate DOM events (click/input/change) back to Python via a single bridge
 *      call, keyed by the `id` attribute every widget already renders (widgets.py's
 *      `node_id`) and the `data-zkite-on` attribute listing which logical events that
 *      node has handlers for.
 *
 * `window.ZKiteNative` is the bridge object. In the shipped app it's installed by
 * `ZKiteActivity` via `WebView.addJavascriptInterface` (see M4 in ../PLAN.md, not yet
 * built). In `test/harness.html` it's a stub that logs to the page instead, so this
 * file is testable in any desktop browser with zero Android involved.
 */

(function () {
  "use strict";

  const ROOT_ID = "zkite-root";

  // DOM event -> logical event name (matches Python's `on_<name>` prop convention).
  const DOM_TO_LOGICAL = {
    click: "tap",
    input: "input",
    change: "change",
  };

  function extractPayload(el, domEventType) {
    if (domEventType !== "input" && domEventType !== "change") return null;
    if (el.tagName === "INPUT") {
      // `radio` alongside `checkbox` as of ../PLAN.md §30. Without it a `Radio` fell
      // to `el.value` below, which for a radio with no `value` attribute is the DOM's
      // default string `"on"` — and `_as_bool("on")` is `False`, so selecting a radio
      // would have reported it *deselected*. Exactly the shape of §26.2's original
      // payload bug (`bool("false") is True`), which is why this is a `.checked` read
      // and not a `value` special case.
      if (el.type === "checkbox" || el.type === "radio") return el.checked;
      if (el.type === "range") return Number(el.value);
      return el.value;
    }
    return el.value !== undefined ? el.value : null;
  }

  function findBoundAncestor(target, logicalEvent) {
    let el = target;
    while (el && el !== document.body) {
      const bound = el.getAttribute && el.getAttribute("data-zkite-on");
      if (bound && bound.split(" ").indexOf(logicalEvent) !== -1) {
        return el;
      }
      el = el.parentElement;
    }
    return null;
  }

  function onDomEvent(domEventType) {
    return function (evt) {
      const logical = DOM_TO_LOGICAL[domEventType];
      if (!logical) return;
      const el = findBoundAncestor(evt.target, logical);
      if (!el) return;
      const payload = extractPayload(el, domEventType);
      // phase_3/PLAN.md §12.2 item 3. Started here — this backend first acting on
      // the tap — to match exactly where the native backend starts its own clock
      // (`render_thread.rs`'s `dispatch_tap_to`). Starting at the raw touch on one
      // side and at the delegated handler on the other would charge one backend for
      // platform dispatch the other never sees.
      if (logical === "tap") {
        tapStartedAt = window.performance.now();
        tapPending = true;
      }
      if (window.ZKiteNative && typeof window.ZKiteNative.dispatch === "function") {
        window.ZKiteNative.dispatch(el.id, logical, payload === null ? "" : String(payload));
      }
    };
  }

  function installEventDelegation() {
    document.addEventListener("click", onDomEvent("click"), true);
    document.addEventListener("input", onDomEvent("input"), true);
    document.addEventListener("change", onDomEvent("change"), true);
  }

  // ---- virtualised lists (phase_3/PLAN.md §11) ------------------------------------
  //
  // A `List` only builds the rows in view, so it has to be told where it is
  // scrolled to. This is the one place scroll position flows backwards, from the
  // rendered page into Python. The native backend does the same thing from its
  // render thread (`render_thread.rs`'s `sync_viewports`); both report the same two
  // numbers, so both backends materialise the same window and stay conformant.
  //
  // Reported through the existing `dispatch` bridge under a reserved `"scroll"`
  // event name (see runtime.py's `_zkite_scroll_html`) rather than a new bridge
  // method, so no Java/JNI in either shipped shell has to change.
  //
  // Scroll fires far faster than a frame; coalescing to one report per animation
  // frame is the DOM-side equivalent of the render thread's own `TouchMove`
  // coalescing (PLAN.md §6.5, where the same "input arrives faster than it can be
  // processed" problem cost a real, user-visible bug).
  let scrollReportPending = false;

  function reportScroll(el) {
    if (!window.ZKiteNative || typeof window.ZKiteNative.dispatch !== "function") return;
    window.ZKiteNative.dispatch(el.id, "scroll", `${el.scrollTop},${el.clientHeight}`);
  }

  function onVirtualScroll(evt) {
    const el = evt.target;
    // Frame sampling is armed by *any* scroll, not only a virtualised one: a plain
    // `ScrollView` scrolling entirely inside Blink's compositor — no JS, no Python —
    // is exactly the case §12.2 item 2 is about, and reporting nothing for it would
    // mean measuring only the cases where this backend does work.
    noteScrollActivity();
    if (!el || !el.getAttribute || el.getAttribute("data-zkite-virtual") === null) return;
    if (scrollReportPending) return;
    scrollReportPending = true;
    window.requestAnimationFrame(function () {
      scrollReportPending = false;
      reportScroll(el);
    });
  }

  function installScrollReporting() {
    // Capture phase: `scroll` does not bubble, so a listener on `document` only
    // ever sees it at all by capturing.
    document.addEventListener("scroll", onVirtualScroll, true);
  }

  // A list's first render has no measured height to window against, so Python
  // bootstraps a generously-sized window and corrects it once a real viewport is
  // reported. Reporting right after a render is what makes that correction happen
  // on the next frame instead of on the user's first scroll.
  function reportVirtualViewports() {
    const els = document.querySelectorAll("[data-zkite-virtual]");
    for (let i = 0; i < els.length; i++) {
      if (els[i].id) reportScroll(els[i]);
    }
  }

  // ---- instrumentation (phase_3/PLAN.md §12.1) ------------------------------------
  //
  // The native backend has reported per-redraw timings since §11.5 and flags any
  // frame over a threshold; this backend had *no* per-frame timing of any kind, so
  // the two could not be compared even in principle — which is what §12.1 calls
  // "instrumentation parity" and names as the thing that has to exist before any of
  // §12.2's measurements can be taken. These are the three numbers it asks for:
  // time to the first painted frame, per-frame cost during a sustained scroll, and
  // node count per frame.
  //
  // Emitted through `console.log` with a greppable prefix rather than through a new
  // `ZKiteNative` method, for the same reason §11.3 reported scroll position through
  // the existing `dispatch` bridge: no new bridge surface, and therefore no JNI or
  // protocol change in either shipped shell. The shells forward console messages to
  // logcat (`ZKiteActivity`'s `WebChromeClient`), so these land next to the native
  // backend's own lines and one grep reads both.
  //
  // Three constraints, each learned the expensive way elsewhere in this project:
  //
  //  * **Log a slow frame immediately, never only a rolling average** — §12.1, and
  //    `render_thread.rs`'s own `SLOW_REDRAW_THRESHOLD` doc comment. An average hides
  //    exactly the frames worth knowing about (one 400ms stall inside sixty 8ms
  //    frames reads as 14ms), and an every-N-frames summary reports *nothing at all*
  //    when frames are slow enough that N is never reached — which is precisely the
  //    app you most want numbers for. §11.5 lost a whole measurement pass to this.
  //  * **Measure to paint, not to the end of our own JS.** `morphdom` returning is
  //    not a frame — Blink still has style, layout, paint and composite to do. A
  //    *double* `requestAnimationFrame` is what lands after the frame our mutation
  //    caused actually reached the screen; a single rAF callback runs *before* paint,
  //    so measuring to it would systematically under-report this backend's cost and
  //    flatter it against a native side that measures all the way through `present`.
  //  * **Report a distribution, not a mean** — §12.2 item 2: jank is a tail property,
  //    and "scroll jank" is the specific charge plan1.md §5 levels at this backend.
  //
  // Left permanently on rather than behind a debug flag, matching the native side's
  // own reasoning: a `performance.now()` and an array push per frame is cheap enough
  // to leave on, and leaving it on is the whole point.
  const PERF_TAG = "ZKITEPERF";
  // Both mirror `render_thread.rs` so the two backends' numbers mean the same thing:
  // one missed frame at 30fps, and roughly a second of continuous scrolling.
  const SLOW_FRAME_MS = 33;
  const FRAME_STATS_INTERVAL = 60;
  // How long after the last scroll event the frame sampler stops. Long enough to
  // cover a fling's own tail (which produces frames but no scroll events once the
  // finger is gone), short enough that an idle page isn't kept awake by our own rAF.
  const SCROLL_IDLE_MS = 250;

  function perfLog(line) {
    console.log(PERF_TAG + " " + line);
  }

  // Runs `cb` after the frame containing the current mutation has been painted. The
  // inner rAF is the load-bearing one: the outer callback fires *before* that frame's
  // paint, so a single rAF would time our own JS plus Blink's scheduling and nothing
  // of the actual work.
  function afterPaint(cb) {
    window.requestAnimationFrame(function () {
      window.requestAnimationFrame(cb);
    });
  }

  function domNodeCount() {
    const root = document.getElementById(ROOT_ID);
    return root ? root.getElementsByTagName("*").length : 0;
  }

  // Sorted-sample percentile. Computed over the whole sample rather than tracked
  // incrementally because these arrays are one scroll long (hundreds of entries at
  // most) and being obviously correct matters more here than being clever.
  function percentile(sorted, p) {
    if (!sorted.length) return 0;
    const i = Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length));
    return sorted[i];
  }

  function fmt(ms) {
    return ms.toFixed(2);
  }

  // ---- first paint ----------------------------------------------------------------
  //
  // Reported page-relative (`performance.now()`); the shell adds the process-start
  // anchor, since only Java can see `Process.getStartElapsedRealtime()` and doing the
  // arithmetic there avoids reconciling two clock domains across the bridge.
  let firstPaintReported = false;

  function reportFirstPaintOnce() {
    if (firstPaintReported) return;
    firstPaintReported = true;
    afterPaint(function () {
      perfLog("first-paint page_rel=" + fmt(window.performance.now()) + "ms nodes=" + domNodeCount());
    });
  }

  // ---- re-render cost -------------------------------------------------------------
  //
  // This backend's closest analogue to a native `redraw`: Python has produced new
  // HTML and it has to reach the screen. `apply` is our own morphdom diff; `paint` is
  // that plus everything Blink then does, which is the number actually comparable to
  // the native frame.
  const renderStats = { count: 0, apply: 0, paint: 0, worstPaint: 0 };

  // §12.2 item 3 — set by the delegated `tap` handler, consumed by the first paint
  // that follows it, which is the frame showing the tap's result.
  let tapStartedAt = 0;
  let tapPending = false;

  function noteRender(applyMs) {
    afterPaint(function () {
      const paintMs = window.performance.now() - renderStartedAt;
      if (tapPending) {
        tapPending = false;
        // Same caveat as the native backend's matching line: this is the frame
        // reaching the compositor, not the panel. Both sides exclude the same hop.
        perfLog("tap-to-paint=" + fmt(window.performance.now() - tapStartedAt) + "ms");
      }
      renderStats.count += 1;
      renderStats.apply += applyMs;
      renderStats.paint += paintMs;
      if (paintMs > renderStats.worstPaint) renderStats.worstPaint = paintMs;
      if (paintMs >= SLOW_FRAME_MS) {
        perfLog("render SLOW: " + fmt(paintMs) + "ms (apply " + fmt(applyMs) + "ms, " + domNodeCount() + " nodes)");
      }
      if (renderStats.count >= FRAME_STATS_INTERVAL) {
        perfLog(
          "render x" + renderStats.count +
          ": avg " + fmt(renderStats.paint / renderStats.count) +
          "ms max " + fmt(renderStats.worstPaint) +
          "ms (apply avg " + fmt(renderStats.apply / renderStats.count) +
          "ms, " + domNodeCount() + " nodes)"
        );
        renderStats.count = 0;
        renderStats.apply = 0;
        renderStats.paint = 0;
        renderStats.worstPaint = 0;
      }
    });
  }
  let renderStartedAt = 0;

  // ---- frame interval during a sustained scroll ------------------------------------
  //
  // The one number that is genuinely like-for-like across the two backends, and the
  // reason it has to be measured this way rather than by timing our own work: on this
  // backend a plain scroll runs entirely inside Blink's compositor and re-enters
  // neither JS nor Python, so "time spent in our code" would report ~0 and mean
  // nothing. What a user feels is the interval between painted frames, which is what
  // this samples. On the native side the equivalent is the interval between successive
  // `present` calls (added alongside this — see `render_thread.rs`).
  //
  // Sampling only while scrolling is deliberate: a permanently-running rAF loop would
  // keep the compositor awake and change the thing being measured. The caveat that
  // remains — a rAF loop does itself request frames — is why this starts on real
  // scroll activity rather than running unconditionally.
  let sampler = null;

  function stopSampler() {
    if (!sampler) return;
    const deltas = sampler.deltas;
    window.cancelAnimationFrame(sampler.raf);
    sampler = null;
    if (deltas.length < 2) return;
    const sorted = deltas.slice().sort(function (a, b) { return a - b; });
    const total = deltas.reduce(function (a, b) { return a + b; }, 0);
    perfLog(
      "scroll-frames n=" + deltas.length +
      " avg " + fmt(total / deltas.length) +
      " p50 " + fmt(percentile(sorted, 50)) +
      " p95 " + fmt(percentile(sorted, 95)) +
      " max " + fmt(sorted[sorted.length - 1]) +
      " (ms, " + domNodeCount() + " nodes)"
    );
  }

  function sampleFrame(now) {
    if (!sampler) return;
    if (sampler.last !== 0) {
      const delta = now - sampler.last;
      sampler.deltas.push(delta);
      if (delta >= SLOW_FRAME_MS) {
        perfLog("scroll-frame SLOW: " + fmt(delta) + "ms");
      }
    }
    sampler.last = now;
    if (now >= sampler.idleAt) {
      stopSampler();
      return;
    }
    sampler.raf = window.requestAnimationFrame(sampleFrame);
  }

  function noteScrollActivity() {
    const now = window.performance.now();
    if (sampler) {
      sampler.idleAt = now + SCROLL_IDLE_MS;
      return;
    }
    sampler = { deltas: [], last: 0, raf: 0, idleAt: now + SCROLL_IDLE_MS };
    sampler.raf = window.requestAnimationFrame(sampleFrame);
  }

  // morphdom always copies toEl.value onto fromEl for <input>/<textarea> (see
  // vendor/morphdom.min.js's special-element handling) — it does NOT check focus.
  // Without this hook, a re-render sent while the user is mid-keystroke would
  // clobber whatever they just typed with the server-rendered value, defeating the
  // "keyboard state ... intact" promise (plan1.md §6). The fix: when the live
  // element is focused, overwrite the *incoming* value with the current one before
  // morphdom's own sync runs, so the sync is a no-op but every other attribute
  // (class, disabled, ...) still morphs normally.
  function preserveFocusedInputValue(fromEl, toEl) {
    const isTextLike = fromEl.tagName === "INPUT" || fromEl.tagName === "TEXTAREA";
    if (isTextLike && fromEl === document.activeElement) {
      toEl.value = fromEl.value;
    }
    return true;
  }

  const pagerState = new WeakMap();
  function syncPagers(root) {
    root.querySelectorAll("[data-zkite-pager]").forEach(el => {
      const index = Number(el.getAttribute("data-zkite-pager"));
      let state = pagerState.get(el);
      if (!state) {
        state = { index, timer: null };
        pagerState.set(el, state);
        el.scrollLeft = index * el.clientWidth;
        el.addEventListener("scroll", () => {
          clearTimeout(state.timer);
          state.timer = setTimeout(() => {
            if (!el.isConnected || !el.clientWidth) return;
            const page = Math.round(el.scrollLeft / el.clientWidth);
            if (page !== state.index && window.ZKiteNative) {
              state.index = page;
              window.ZKiteNative.dispatch(el.id, "change", String(page));
            }
          }, 160);
        });
      } else if (state.index !== index) {
        state.index = index;
        el.scrollTo({ left: index * el.clientWidth, behavior: "smooth" });
      }
    });
  }

  function applyHtml(html) {
    const root = document.getElementById(ROOT_ID);
    if (!root) {
      throw new Error(`zkite-runtime: no #${ROOT_ID} element in the page`);
    }
    renderStartedAt = window.performance.now();
    const wrapped = `<div id="${ROOT_ID}">${html}</div>`;
    window.morphdom(root, wrapped, { onBeforeElUpdated: preserveFocusedInputValue });
    syncPagers(root);
    const applyMs = window.performance.now() - renderStartedAt;
    reportVirtualViewports();
    // Order matters: the first-paint report has to be armed before the re-render
    // stats consume this frame's `afterPaint`, so the very first `applyHtml` is
    // counted as the cold-start frame rather than as an ordinary re-render.
    reportFirstPaintOnce();
    noteRender(applyMs);
  }

  function boot() {
    installEventDelegation();
    installScrollReporting();
    if (window.ZKiteNative && typeof window.ZKiteNative.ready === "function") {
      window.ZKiteNative.ready();
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot);
  } else {
    boot();
  }

  window.ZKiteRuntime = { applyHtml, ROOT_ID };
})();
