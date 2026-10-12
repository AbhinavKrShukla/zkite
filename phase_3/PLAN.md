# Phase 3 — Native renderer

This is the implementation plan for **plan1.md §10, Phase 3**:

> Taffy + Parley + Vello Hybrid · IME · scrolling and virtualised lists · gestures · AccessKit ·
> per-widget conformance against the WebView backend.
> **Exit:** the backend switch flips by default and no sample app changes a line.

Phase 1 shipped the WebView backend (`phase_1/webruntime` + `phase_1/devshell`); phase 2 shipped
the bundler (`phase_2/bundler` + `phase_2/appshell`). Both reused `phase_1/zkite_py`'s widget tree
and `ZKiteActivity`'s JNI/asset-unpacking scaffolding unmodified across phases — phase 3 continues
that pattern. `twcore` (`phase_0/spike3_twcore`) has sat untouched since phase 0, "reserved for
phase 3's native renderer, where there is no CSS engine to defer to" (phase 1 PLAN.md §1) — this
is the phase where that reservation gets used.

Given the size of this phase (plan1.md §10 budgets **12 weeks**, more than phase 0+1+2 combined),
it's structured the way phase 0 structured its own de-risking: a small number of pass/fail spikes
for the parts nobody has touched yet, before any framework code, because plan1.md §11 is explicit
that IME and accessibility done late "cause a rewrite," and phase 0's own known-gaps section
already found real signal here (Flutter/Compose's multi-year `InputConnection` bugs).

**Status:** the first sessions on this phase covered the first spike (§3, software render
pipeline) and the second (Android GPU surface bring-up, §4), then wiring `zkite_py`'s real widget
tree through `zkite_pybridge` and adding scrolling, gesture disambiguation, and momentum (§6) —
all verified live on the emulator and/or real hardware, not just compiled. A later session
covered the first slice of IME — focus, a real `InputConnection` via `vendor/gametextinput`, live
typing, and blur (§7) — also verified live on real hardware, plus turned up a real, still-
unresolved rendering bug along the way (§7.6). A later session shipped tap-to-position-cursor and
a native-`EditText` selection overlay (§7.7/§7.8). The most recent session closed out the rest of
§5 item 1 — IME insets/resize compensation, per-field input types with real password masking on
both render paths, Unicode-correct word selection, double-tap-to-select, and a live (not just
traced) verification of the cross-field selection-overlay race — and made real, if incomplete,
progress narrowing §7.6's rendering bug down to a 3-node minimal repro (§7.9–§7.13). The most
recent session built §5 item 2, AccessKit — a virtual accessibility tree exposed through a real
`android.view.accessibility.AccessibilityNodeProvider`, verified live against real Android
accessibility tooling (§8). The most recent session built §5 item 5, per-widget conformance
testing — a real, host-only (no device needed) pipeline comparing the WebView backend (real
Chromium via Playwright) against the native backend (`zkite_renderer`+`vello_cpu`) pixel-for-pixel
across all 12 phase-1 widgets, which surfaced several real, previously undocumented rendering
divergences along the way, not just confirmed already-known ones (§9). A follow-up session fixed
three of §9.2's four findings (Slider's missing WebView CSS, Switch's metric/color drift, and
`zkite_renderer` registering a bold font face) and, investigating why the fourth (`button`) didn't
move, surfaced a real, larger, previously-undocumented gap of its own — native has no font-
*family* resolution at all — see §9.4 for the full, honest account of what actually closed vs.
what just moved. The most recent session built §5 item 3, virtualised lists — the `List` widget
plan1.md §8 names and phase 1 deliberately deferred, working on *both* backends, verified live on
the Moto G54 against a like-for-like non-virtualised control (10,000 rows: ~3,850ms per scroll
frame before, ~30ms after) and honest that ~30ms is still short of 60fps and why (§11). That work
also surfaced two things bigger than itself: `zkite_py` could not import any C-extension stdlib
module on the native backend (§11.6, latent since phase 1) and the conformance harness's own
tolerance was hiding a palette-wide colour divergence between the two backends (§9.5). The most
recent session **solved both**. §9.5's palette divergence is closed by regenerating `twcore`'s
palette from the OKLCH values in the very `@tailwindcss/browser` bundle the WebView backend
vendors, so the two backends now agree by construction rather than by coincidence — the strict
conformance column falls from as high as 86.83% to under 1% on every colour-dominated fixture
(§9.6). And §11.8 solved the other — Android's package installer only extracts
`lib/<abi>/` entries named `lib*.so`, so all 67 extension modules had been silently dropped at
install time on every ZKite APK ever built; found by one `adb shell ls` of the installed app's
native library directory, fixed by renaming them into `libpymod_<mod>.so` plus a `sys.meta_path`
finder in the shared `runtime.py`, and verified live on both the native harness and the shipped
`zkite build apk` path, the latter of which additionally needed
`android:extractNativeLibs="true"`. The same session then picked up §7.6's still-unresolved rendering bug with no
physical device available (`adb devices` empty throughout) and, rather than stall, built a new
host-only crate (`phase_3/repro_7_6/`) pushing §7.9's 3-node minimal repro through the *real*
`vello_cpu` paint pipeline instead of just layout metadata — a negative result (33 redraws, "0"
rendered correctly every time) that rules out `zkite_renderer`/`zkite_pybridge`/`vello_cpu`'s own
rasterization for the first time with real evidence, narrowing what's left to Android's
`ANativeWindow` blit path, the GPU backend, or a real keyboard-triggered surface resize — none
reachable without a device; see §7.6's own update. The most recent session built §11.5's named
next lever, the **shaped-text cache** — unchanged text is no longer re-shaped through parley on
every frame, taking a 10,000-row list scroll frame from 38.4ms to 23.8ms (~26 → ~42fps) in a
same-session on-device A/B, with a bounded working set and conformance unmoved on all 14
fixtures. It also corrected §11.5's own account of *why* the frame was slow: re-shaping was 46%
of the device frame, not nearly all of it, and `redraw` now permanently reports all seven frame
phases rather than leaving the next person to attribute a frame by subtraction — which is how
that error was made. See §11.9. The most recent session **built and ran §12, the head-to-head
backend comparison that had never been run** — instrumentation parity on the WebView side, five
apps run byte-identically through both shells, on one device back to back. It found two real
bugs before it produced a single timing: **the native backend ignores device density entirely**,
so every native app renders at `1/dpr` size on every real phone (invisible to §9's harness,
which runs at dpr 1), and **the WebView backend's `List` never converged on a device** — 268
renders, DOM climbing to 10,361 nodes — which is exactly the gap §11.7 had named and is now
fixed. On the timings, the headline is that **the WebView backend holds a locked 60fps
scrolling a 10,000-row list where the native backend manages ~39fps**, the reverse of what
plan1.md §5 asserts, while native wins tap latency ~5x. §12.7 applies §12.3's pre-stated
criteria and lands on "comparable but not better": the switch does not flip on this evidence,
phase 3 is not dropped on it either, and two named fixes (density, vsync pacing) come first.
The most recent session **built both of those fixes and re-ran §12.2** — see §13. Density is
closed: native and the WebView now render the same app at byte-identical size on one phone
(324x119 vs 324x120, where it was 130x48 vs 242x90), verified live along with taps, caret,
tap-to-offset, a11y bounds and the selection overlay, and the conformance harness gained a
second device-pixel-ratio so it can see this class of bug at all — confirmed by reintroducing
the bug and watching all 14 fixtures fail. Pacing turned out not to be a vsync-*alignment*
problem at all: the render thread waited a fixed 16ms **and then** rendered, delivering
`interval + work`; making that a deadline took a 10,000-row scroll from ~33fps to ~58fps in a
same-device A/B. §13.4 promotes §12.7 from provisional to answered — "comparable but not
better" survives, but its sharpest claim does not: **native no longer loses sustained scroll**
(~58 vs 60fps), and the remaining case against flipping the switch is an ordinary ledger
(15.5MB of binary, ~310MB resident dominated by an unexplained 266MB of GPU allocation, ~490ms
more cold start) rather than a performance contradiction. The most recent session built **the
backend switch itself (§14)** — the structural blocker §12.4 named, and the thing plan1.md
§10's exit criterion is written against. `zkite build apk --backend native` now produces a
standalone signed APK running an arbitrary `zkite new` app on the native renderer, from the same
one pipeline and the same byte-identical app bytecode as the WebView build; the default is
unchanged. It turned out to be far smaller than this document had assumed, because
`ZKITE_APP_MAIN` never baked an app into the binary (§14.1) — what was missing was a
release-artifact path, not a shell. Two release-pipeline gaps fell out of that and are closed
(§14.3), and one real gap in the *other* backend surfaced: the shipped WebView path was
shipping bundled fonts or not depending on whether a gitignored directory happened to exist on
the build machine, silently either way (§14.4). **Verified live on the Moto G54**, both backends
back to back from one project: the native build launches, loads its C++ runtime and fonts from
the new artifact path, imports all 67 extension modules and increments its counter `0 → 3` under
real taps; the WebView build of the same project still works and — incidentally, and for the
first time — is confirmed to be rendering the *bundled* typeface rather than falling back to
Roboto (§14.5). The same session also put a control under §13.3's unexplained GPU memory for the
first time (native 272MB of `Graphics` against the WebView's 101MB for the same app on the same
surface, and it is screen-sized rather than content-sized) and refuted the most plausible
explanation for it (§14.7). The most recent session **gave the native backend the dev loop it
lacked and made it the default** (§15). `zkite dev` now hot-reloads on native — verified live with
a real edit that changed the background and the button while the counter kept its value, i.e.
plan1.md §6's promise on this backend — and it turned out *simpler* than the WebView original,
because the render thread already owned "new tree from Python → repaint" and needed no polling.
**The default flip is an explicit owner decision made against §13.4's ledger, not a conclusion
the numbers support** (§15.4): native still loses cold start, memory and 15.5MB of binary, and the
argument that carried it is control over rendering rather than performance — informed by research
that "ship a browser in Rust" (Blitz = Stylo + Taffy + Parley + Vello) and "build the native
renderer" are the same project, that Blitz is alpha, and that Dioxus ships the system WebView for
mobile today. §15.1 is the honest inventory that flip makes urgent: no navigation or back stack
anywhere, `Image`/`Icon` paint empty boxes, no borders/shadows, no animation, and three *silent*
backend divergences found this session — horizontal scrolling doesn't scroll, `dark:` never
activates, and the system font-size setting is ignored by both backends.
The most recent session closed the first item on that inventory: **navigation and a back
stack (§16)**, the gap plan1.md §14.1 called the single largest in the framework. It is
built as three layers rather than one — a back-handler chain, a two-method
`NavigationHost` protocol, and the stack as *one* policy over it — so a tab navigator,
a modal navigator or a URL router is a later addition rather than a rewrite, and none of
that generality costs anything per frame (only the top screen is ever built; the back
chain is untouched unless Back is pressed). Verified live on the Moto G54 on **both**
backends, including the real back gesture driven by hand rather than `adb`, plus a
no-`Nav` control proving one-screen apps still exit exactly as before. Two findings fell
out: `vendor/pack` cannot encode `android:enableOnBackInvokedCallback` (caught by
bundler's own tests, backed out, legacy dispatch covers every API level meanwhile), and
`from __future__ import annotations` had been silently disabling `@store` injection since
phase 1.
The same session then ran **§17, a production-readiness research pass** (signing,
`version_code`, cross-platform `zkite build` — publishing deliberately excluded). The
largest result was not on anyone's list: **all three shells had CPython's bytecode
cache disabled** (`config.write_bytecode = 0`, uncommented, since phase 1) while the
pruned stdlib ships as ~490 *source* files, so every launch re-parsed all of them.
One line, A/B'd on one phone: **native 2584ms → ~990ms, WebView 978ms → 420ms** —
which puts the WebView backend inside plan1.md §9's "interactive <500ms" target that
§12.6 had called architecturally unreachable. Enabling it surfaced a real `zkite dev`
staleness hazard (CPython validates `.pyc` on *whole-second mtime + size*, both of
which a hot reload can leave unchanged) — fixed by purging only the app directory's
cache — and a pre-existing test-isolation bug it was masking. §17.2 also refutes, with
a measurement, the idea that dropping the `.pyc` step would make `zkite build`
portable: **stdlib pruning needs a matching interpreter**, and under 3.12 it silently
drops `annotationlib`, which `import zkite` requires on 3.14.
The most recent session closed §15.1's divergence 6 — **`Switch` and `Slider` now
respond to touch on the native backend (§26)** — after first confirming it on the
device, which turned up two things the code read had not. A tap on a `Switch` did not
do nothing: it dispatched to the nearest `tap` ancestor instead, and *that bubbling is
conformant* (`zkite-runtime.js` does the same), so the divergence was only ever the
missing `change`. And the `change` payload was a **string on both backends** —
`bool("false")` is `True`, so a `Switch` on the *WebView* backend could be turned on
and never off, and WIDGETS.md's documented `bool`/`float` had never been true
anywhere. That is fixed by typing the payload at the widget, which both backends share
by construction. A slider drag is a **capture at touch-down, not a direction test**, so
a vertical drag on a slider moves the slider instead of scrolling the list under it.
The reason this survived three phases is worth more than the fix: `demo_app/main.py`
wired its `Switch` with an undocumented `on_tap` and gave its `Slider` no handler at
all, so every device verification in this document exercised an API no user would
write. A new `slider_range` conformance fixture then found a thumb that sat half its
own width too far right and left the track entirely at maximum — closing a gap
carried as cosmetic, and taking `slider` from 1.28% to **0.02%**. Queue A item 2 landed
alongside it: §25.3's untested PEP 563 class now has a fixture over the whole stack,
confirmed to fail against both known instances.
The most recent session closed §15.1's biggest user-visible rendering gap:
**borders, shadows and rings now paint (§28)**, on both native paint paths, with the
geometry shared by all four paint consumers the way `icons.rs` already shares its own.
It was picked over the rest of the queue for a reason worth repeating — it is the only
entry on that list §9's harness can *see and discriminate*, since a border is a static
thing at offset 0 where §26.6 and §27.5 both had to concede the harness cannot touch or
scroll anything. Two findings came out of it that are larger than the feature.
**`twcore`'s border, shadow and ring defaults were all still Tailwind v3's** — v4
defaults both `border-color` and `ring-color` to `currentColor`, drops the default ring
width from 3px to 1px, renames the whole shadow scale and makes most of it two-layer —
which §9.6's own known-gaps entry predicted and nobody had checked, because nothing
painted any of it; §9.6 had even touched the ring default and corrected only its hex.
And **the conformance harness turns out to be blind to shadow correctness**: deleting
the CSS-blur-to-standard-deviation conversion changed the render and moved the number
by nothing, because Tailwind's entire shadow scale is fainter than `CHANNEL_TOLERANCE`
— §9.5's lesson recurring in a new place, caught only because the negative control was
run. **Verified live on an x86_64 emulator, not on the Moto G54**, which was not
connected this session and is §28's top open item.
The most recent session took the one item on the queue whose evidence does not
depend on a phone, because the Moto G54 was **again** not connected: **`zkite build`
now runs on a machine that is not this checkout (§29)**. All five compile-time path
anchors collapse into one `payload::root()` — env override, `zkite-payload/` beside the
binary, the checkout **last** — with the payload deliberately laid out like the
checkout so the dev fallback is valid by construction rather than by special case.
Proved rather than asserted: a payload staged outside the checkout, an installed binary
that never reads the checkout, and a **sentinel file** that appears inside the built
APK only on the payload path (absent in the control). The APK installs, boots CPython,
imports all 67 extension modules and counts 0 → 3 under real taps — on the emulator,
which is evidence and not proof. Making `zkite doctor` report the resolved root — §23.7
step 1's named "fight" — immediately paid for itself by exposing a bug nothing else
could see: `doctor` **exited 1 on §23.4's own recommended native-only default wheel**,
because its fatal-vs-informational backend split was still hardcoded the pre-§15.4 way
under a comment saying native was not the default. §17.4 item 2's per-ABI `version_code`
collision landed alongside it, confirmed by `aapt2` reading the real binary AXML (13 and
14 where both were 1), on a scheme deliberately *not* Google's own because theirs is not
monotonic in the base. Also found and **deliberately not fixed**: a pre-existing
intermittent failure in `zkite_pybridge` (2 in 15 on a clean tree, 0 in 8 under
`--test-threads=1`), narrowed to a concurrency interaction that is *not* the interpreter
lock §17.1.1 already fixed — §29.5. **And then the phone was connected mid-session, so
§28's owed Moto G54 run finally happened (§29.8)**: all six rows correct on real PowerVR
silicon at density 2.5, `currentColor` borders and rings matching v4's palette at distance
**0.0**, per-side widths and mitres exact, paint order confirmed, idle control
pixel-identical. It found what the emulator could not — **`shadow-2xl` is ~1.75x too dark
at the card edge** against real Chromium driving the vendored bundle, the one entry in the
scale whose blur sigma exceeds its own spread-shrunk rect — and **the conformance harness
scores that 0.00%**, which is the first concrete instance of the blindness §28.4 could
previously only demonstrate with a control. Left unfixed, deliberately, with a number on
it.
With the phone now on the desk, the same session then took the queue that most needs
one: **`Checkbox` and `Radio` (§30)**, taking the native backend's working form
vocabulary from three controls to five — the gap the previous prompt called "the one a
new user hits in hour one". Built on §26's machinery exactly as that prompt predicted;
the only thing that had to be *decided* was how three controls that are all an `input`
carrying `checked` tell themselves apart, and the answer is **a `role`, emitted on both
paths**, because it is a fact about the control rather than about the renderer — it
scopes `index.html`'s switch CSS (which was a global `input[type=checkbox]` selector,
correct until `Checkbox` existed) *and* it is what makes Android announce a real
`CheckBox`/`RadioButton`, confirmed via `uiautomator`. Only `Radio` gets its own
`Control`, for one rule: a tap on a radio sends `true`, never `!checked`. A payload bug
in exactly §26.2's shape was caught before it shipped — a radio's DOM default value is
`"on"`, and `_as_bool("on")` is `False`, so selecting one would have reported it
deselected. Six device cases run in isolation, three of them about the *second* tap,
which no static fixture can ask. `Select` is deliberately **not** built: it is the one
with a real design decision in it and §30.6 writes the decision down with a
recommendation instead of guessing.

---

## 1. Finding that reshapes the renderer's Android integration

**`android-activity` cannot be embedded in ZKite's existing `ZKiteActivity`; it has to own the
Activity, and ZKite's architecture already made a different, incompatible choice.**

plan1.md §3's `libzkite.so` diagram (reproduced below) lists `android-activity` as
"lifecycle · input · IME · surfaces," which reads as if it slots in alongside Taffy/Parley/Vello
as one more library. It doesn't. Checked directly against the crate's own docs: `android-activity`
loads Rust as a `cdylib` through `onCreate` of an Activity that **must** be
`android.app.NativeActivity` or `com.google.androidgamesdk.GameActivity` — "you will need to
either specify the 'native-activity' feature or 'game-activity' feature to identify which
Activity base class your application is based on." There is no documented way to use just its
input types or window handling without ceding the whole Activity. `NativeActivity` was already
ruled out by name in plan1.md §3 ("IME unsupported... not sufficient for production
applications"), and `GameActivity` is a real compiled-Java class from a Google AAR that ZKite's
manifest would have to declare as *the* Activity — which is a different Java entry point than
`dev.zkite.appshell.ZKiteActivity`, the one phase 1/2 already built with its own JNI surface
(`boot`/`dispatch`/`pollHtml`), WebView-host lifecycle, insets handling, and asset unpacking.

Swapping to `GameActivity` would mean rebuilding all of that against a foreign base class instead
of extending it — exactly the "rewrite" §11 warns architecture-shaping decisions cause when made
late. It's also unnecessary: phase 0's own README already flagged the actual fix, for IME
specifically, without naming this consequence — *"GameTextInput is standalone-usable (not
GameActivity-only — it attaches to any custom `View` via `onCreateInputConnection`, which matches
the dex shim's shape)."* The same is true one level down, for the surface itself. Confirmed
against prior art ([`wgpu-in-app`](https://github.com/jinleili/wgpu-in-app), which integrates wgpu
into an existing Android/iOS app without `android-activity`): a plain `android.view.SurfaceView`
(or `TextureView`) hosted inside `ZKiteActivity`, with its `Surface` handed across JNI on
`surfaceCreated`, converts to an `ANativeWindow*` via `ndk_sys::ANativeWindow_fromSurface`, which
`raw-window-handle`'s `AndroidNdkWindowHandle` wraps for `wgpu::Instance::create_surface` — the
same shape `vello_hybrid` needs, since it renders through `wgpu`.

**Decision: don't depend on `android-activity`.** Keep `ZKiteActivity` (plain
`android.app.Activity`, per phase 1 §6's own finding that AppCompat isn't needed) as the one Java
entry point across all three backends. Add a `ZKiteSurfaceView` alongside/replacing the `WebView`,
extend the existing JNI surface (`boot`/`dispatch`/`pollHtml` → plus `onSurfaceCreated` /
`onSurfaceChanged` / `onSurfaceDestroyed` / touch-event forwarding), and reach for the `ndk` and
`ndk-sys` crates directly for the window handle. `android-activity` is not added as a dependency
at all — not even for its input-event types, since accepting them would pull in its Activity
assumption transitively via its lifecycle model. Revisit only if a future finding shows the
existing JNI-driven architecture can't carry input/lifecycle events fast enough, which nothing
so far suggests.

This is worth stating plainly because plan1.md's own architecture diagram is wrong about it in a
way that would have cost real time to discover mid-implementation rather than before writing any
code — the same category of correction phase 1 §3 made to plan1.md §8's widget argument order,
and phase 1 §6 made to phase 0's own AppCompat assumption.

---

## 2. Architecture (revised per §1)

```
┌───────────────────────────────────────────────────────────────────────┐
│ main.py, pyproject.toml (user's app) — UNCHANGED from phase 1/2       │
├───────────────────────────────────────────────────────────────────────┤
│ zkite_py (pure Python) — UNCHANGED: widgets.py still emits a node tree,│
│  just no longer serialised to an HTML string for this backend         │
├───────────────────────────────────────────────────────────────────────┤
│ CPython C-API (embedded) — UNCHANGED from phase 1/2                   │
├───────────────────────────────────────────────────────────────────────┤
│ ZKiteActivity : android.app.Activity (dex) — SAME class across all 3   │
│  backends (WebView / this). Now hosts a ZKiteSurfaceView instead of a  │
│  WebView; JNI surface grows onSurfaceCreated/Changed/Destroyed +      │
│  touch forwarding, boot/dispatch/pollHtml unchanged in shape.         │
│  ┌─────────────────────────────────────────────────────────────────┐ │
│  │ libzkite_render.so (Rust) — phase_3/renderer/                    │ │
│  │  twcore        class string -> ComputedStyle   (phase 0, reused)│ │
│  │  taffy         layout                          (phase 0, reused)│ │
│  │  parley        text shaping + line breaking     (new)           │ │
│  │  vello_cpu     software raster, host-testable   (new, §3)       │ │
│  │  vello_hybrid  GPU raster via wgpu + the ANativeWindow           │ │
│  │                surface from ZKiteSurfaceView      (new, §4)      │ │
│  │  accesskit     a11y tree -> AccessibilityNodeProvider (later)   │ │
│  │  ndk / ndk-sys  ANativeWindow from a JNI Surface  (new, §1)     │ │
│  └─────────────────────────────────────────────────────────────────┘ │
└───────────────────────────────────────────────────────────────────────┘
```

`android-activity` does not appear anywhere in this diagram (§1). Everything above the
`libzkite_render.so` line is byte-for-byte what phase 1/2 already built.

---

## 3. Spike 1 — software render pipeline (host-only, no device)

**Question:** does `twcore`'s `ComputedStyle` actually carry enough information to lay out
(Taffy), shape text for (Parley), and rasterize (`vello_cpu`) the phase 1 counter app correctly,
with no CSS/DOM/Blink underneath it this time? This is the same question phase 0's spike 3 asked
of Taffy alone; spike 1 here extends it through text and pixels, entirely on the host, so it's
fast to iterate on before any Android-specific risk (§4) is in the loop.

Crate versions resolved live against crates.io in this session (`cargo add --dry-run`, not
guessed): `taffy 0.9` (pinned already, matches `twcore`'s existing dependency — no bump needed),
`parley 0.11.0`, `vello_cpu` (workspace-versioned with `vello_hybrid 0.1.0`), `accesskit 0.24.1`,
`accesskit_android 0.7.5` (has an `embedded-dex` feature, worth checking against ZKite's own
prebuilt-dex convention before using it — deferred to the AccessKit milestone, §5).

Layout: `phase_3/spike1_render_pipeline/`, its own crate, same convention as phase 0's spikes.

**Status: passing, text-centering gap closed for real, promoted to a shared crate.** The layout +
measurement half of this pipeline turned out not to be spike-only — every future paint backend
(this spike's `vello_cpu`, §4's `vello_hybrid`, anything later) needs the identical `twcore` →
`taffy` → shaped-text logic, and letting each backend re-derive it was exactly how the vertical-
centering bug below would have silently forked into two different almost-right answers. So it's
promoted out to `phase_3/renderer` (crate `zkite_renderer`) — same move phase 2 §1.1 made for
`pack`'s extras-splicing code ("promotion, not new invention") — and `spike1_render_pipeline` is
now a ~130-line consumer of it, keeping only its own `vello_cpu`-specific paint loop.

**The real fix, not a patch:** the earlier draft of this spike hand-measured text boxes
(`w-[160px] h-[64px]` on the counter, `w-[220px] h-[56px]` on the button) and manually placed
glyphs at the box's top-left — which is why text rendered flush-top instead of centered; a fixed
box has no relationship to where a shaped line actually sits. `zkite_renderer::Renderer` fixes
this at the root instead of patching the symptom: text leaves are `taffy` nodes with a
`NodeContext` (`taffy::TaffyTree<TextContext>` + `compute_layout_with_measure`, taffy's own
documented mechanism for content-sized leaves), and the measure closure actually shapes the text
with `parley` and hands `taffy` the *real* ink size. Once a text leaf's box is its real size,
`items-center`/`justify-center` on an ordinary flex parent centers it correctly — no special-case
baseline math anywhere. This is the same reasoning a browser uses (text nodes are intrinsically
sized; flex/grid alignment does the rest), just implemented explicitly since there's no browser
underneath.

**A second, independent bug surfaced in the same pass, not in this crate:** with real measurement
in place, the "Increment" label centered *inside its own text box* correctly, but the box itself
still sat flush at the top-left of the indigo button — because `Button`'s implied classes
(`phase_1/zkite_py/src/zkite/widgets.py`) never declared `items-center justify-center` in the first
place. The WebView backend never noticed because Chromium centers a plain `<button>`'s content by
default, independent of author CSS; the native renderer has no UA stylesheet to inherit that
from. Fixed at the source, not per-backend — see `phase_1/PLAN.md`'s "Correction" note under §3
and `widgets.py`'s comment on `Button.implied_classes` — so both backends stay conformant by
construction. `zkite_py`'s test suite (39 tests, `PYTHONPATH=src python3 -m unittest discover -s
tests`) still passes after the change, plus one new test locking the fix in.

Run live this session (`cargo run`, `phase_3/spike1_render_pipeline/`), now using
`px-*`/`py-*` padding instead of hardcoded pixel boxes — i.e. plan1.md §8's actual widget tree,
not a stand-in for it:

```
rect (   0.0,    0.0) 400.0x800.0  bg=None  text=None
rect (   0.0,    0.0) 400.0x800.0  bg=Some(Color { r: 2, g: 6, b: 23, a: 255 })  text=None
rect ( 177.0,  325.0)  46.0x 84.0  bg=None  text=Some(1)
rect ( 135.0,  433.0) 130.0x 42.0  bg=Some(Color { r: 99, g: 102, b: 241, a: 255 })  text=None
rect ( 159.0,  445.0)  82.0x 18.0  bg=None  text=Some(1)
wrote counter_app.png (400x800)
```

Box sizes are now *measured*, not asserted (46×84 for a lone "0" glyph at `text-7xl`, 130×42 for
"Increment" plus its button's padding) — and the label's box centers exactly inside the button's
box on both axes (button center (200, 454); label center (200, 454)). Confirmed visually, not
just by the numbers (same "a screenshot catches what a computed check can't" discipline as phase
1 §7 M4): the PNG shows "Increment" sitting dead-center in the rounded indigo pill, and the
button itself now renders with real rounded corners (`RoundedRect`, `radius` wasn't painted at
all in the first draft — added alongside the centering fix since the reference button is
`rounded-2xl` and a square button would have been a visible, avoidable gap in the same image).

Not yet exercised: `text-align` beyond the default (`start`), multi-line wrapping (the measure
function supports it — `break_all_lines(wrap_width)` when a width is constrained — but no test
tree has a long enough string to prove it), `font-weight` beyond regular (parley/fontique font
matching by weight needs more than one registered font face to prove), and borders/shadows/rings
(`Paint` carries them, not painted by any consumer yet — lower priority than radius since neither
backend's reference app uses them).

---

## 4. Spike 2 — Android GPU surface bring-up (device/emulator)

**Question:** does the `ZKiteSurfaceView` → JNI → `ANativeWindow` → `wgpu::Surface` →
`vello_hybrid` chain from §1's finding actually clear a frame to a color on a real device, with
`libpython`/`zkite_py` untouched alongside it (this has to coexist with the existing embedded
interpreter, not replace the app)? This is where §1's architectural decision either holds or
doesn't — it's the highest-uncertainty new integration in this phase, since nothing in phase 0-2
touched GPU surface handling at all.

**Status: build-feasibility checked, integration not started.** Before writing any JNI/Java, the
cheap question worth answering first was whether `vello_hybrid`'s dependency stack — `wgpu`,
`ndk`, `ndk-sys`, `jni`, `raw-window-handle` — cross-compiles for Android at all, since nothing in
phase 0-2 ever touched a GPU crate and a toolchain-level blocker here would reshape §1's plan
before any Java gets written. Checked live this session with a scratch `cdylib` crate (not
committed — pure feasibility check, not a real artifact) via `cargo ndk -t arm64-v8a build`,
`ANDROID_NDK_HOME` pointed at the same `27.3.13750724` phase 0/1/2 already standardised on:
**clean build, ~1 minute, no errors.** `wgpu-hal`, `wgpu-core`, `ndk-sys 0.6`, `ndk 0.9`, `jni
0.21`, `raw-window-handle 0.6` all resolve and compile for `aarch64-linux-android` with no native
build-script failures (contrast §3's `fontconfig` dead end on the host, which needed a feature
flag fix — nothing analogous showed up here). This doesn't prove the runtime behavior (a real GPU
context, a real `Surface`, an actual frame) — only that the crates chosen in §1's revised
architecture are buildable with the existing toolchain, same category of check as phase 0 spike
1's first NDK cross-compile before it ever ran on-device.

**Status: built, and verified live and interactive on the emulator — real-device verification
below.** `phase_3/gpu_harness/` is a full `ZKiteActivity` + `ZKiteSurfaceView` app (`dev.zkite
.gpuharness`), built the same way `phase_1/devshell`/`phase_2/appshell` are (javac+d8 for a small
`classes.dex`, `cargo ndk` for the `cdylib`, `phase_0/spike2_pack_apk` to pack — see
`scripts/build_apk.sh`), booting straight into the plan1.md §8 counter app rendered **for real**
through `wgpu` + `vello_hybrid`, with a live, working tap-to-increment loop. Not a cleared-color
smoke test — the actual counter/button tree, laid out by `zkite_renderer` (shared with §3, so any
future layout fix benefits both backends identically), painted by `vello_hybrid::Scene`, presented
to a real `wgpu::Surface` backed by the `SurfaceView`'s `ANativeWindow`.

Emulator run (`emulator-5554`), this session, end to end: launch → GPU backend initializes
(`SwiftShader Device (Subzero)`, Vulkan backend) → frame renders showing "0" and a correctly
centered "Increment" button → three taps → on-screen count reaches 3, each one a real
touch → hit-test → state update → relayout → repaint → present cycle, confirmed both from logs and
screenshots. Rotation (portrait → landscape → portrait, twice) exercises full surface
teardown/recreation with no crash, no stale-surface reuse, and the tap count survives across all
of it (confirmed at "2" after a tap-tap-rotate-rotate sequence) — see the M4/M5 discipline phase 1
established (verify state survives the thing that's supposed to preserve it, don't just assert the
architecture "should" do so).

**Four real bugs found and fixed this session, each the direct product of actually running on a
device/emulator rather than stopping at "it compiles" — the same pattern phase 0/1/2 each hit at
least once, holding a fourth time:**

1. **A second, mismatched `wgpu::Instance` caused a silent panic with no visible error at all.**
   The first draft's `configure_surface` created a fresh `wgpu::Instance::default()` on every
   call, independent of the one `try_init_gpu` used to obtain the `Adapter`. An `Adapter` and any
   `Surface` it's used with (via `get_capabilities`, or implicitly through `compatible_surface`)
   must come from the *same* `Instance` — wgpu doesn't reject the mismatch up front, it panics
   deep inside `create_surface_unsafe`/`get_capabilities`. And because nothing had installed a
   panic hook yet, the panic unwound silently (default `panic = "unwind"` kills only the
   panicking thread, not the process), leaving a live app showing a permanently black frame with
   *no* logcat line at all — confirmed by checking `/proc/<pid>/task`: the render thread's tid had
   simply vanished. This is the same "Android drops a bare stderr write" class of blind spot
   phase 0's README documented for `Py_InitializeFromConfig`, recurring for `std::panic`'s default
   hook specifically. Fixed two ways: `Gpu` now stores and reuses one `wgpu::Instance` for both
   the adapter and every surface (`render_thread.rs`'s `Gpu` struct), and `run()` installs a
   `std::panic::set_hook` that routes the message through `__android_log_write` — the same
   mechanism `logi!`/`loge!` already use — so a *future* panic on this thread is a loud logcat
   line, not a silent freeze. This second fix is what made the next three bugs findable at all
   instead of three more silent black screens.
2. **`alpha_mode: Opaque` isn't universally supported.** First real `surface.configure()` failed
   validation: `Requested alpha mode Opaque is not in the list of supported alpha modes:
   [Inherit]` (SwiftShader). Fixed by querying `surface.get_capabilities(&adapter).alpha_modes`
   and picking `Opaque` only if it's actually offered, falling back to whatever's first otherwise
   — same "ask, don't assume" principle already applied to format selection.
3. **`Resize` must never create a second surface on the same still-live window.** The first draft
   had one `configure_surface` function that always called `create_surface_unsafe`, so the
   `surfaceChanged` Android guarantees right after every `surfaceCreated` tried to bind a *second*
   `wgpu::Surface` to the same `ANativeWindow` while the first was still bound — Vulkan correctly
   refused it (`ERROR_NATIVE_WINDOW_IN_USE_KHR`), caught immediately by the panic hook from fix #1.
   Split into `ensure_surface` (idempotent — creates once per window, no-ops if one already
   exists) and `reconfigure` (always safe to repeat — reconfigures the existing surface and
   rebuilds the size-bound `vello_hybrid::Renderer`). `NewWindow` calls both; `Resize` calls only
   `reconfigure`.
4. **A tap on a button's own label doesn't hit-test to the button.** `hit_test` correctly finds
   the *deepest* node under a point (the button's text child, not the button container it's
   layered inside), so the first draft's exact-match `hit_test(...) == Some(tree.button)` silently
   failed on every tap that landed on the visible label — which, for a small button, is most taps.
   This is the same target-vs-currentTarget distinction DOM event bubbling makes; `zkite_renderer`
   has no DOM to inherit it from, so it's now explicit: `Renderer::parent(node)` was added, and
   `Touch` handling walks up from the hit node looking for the button among its ancestors before
   giving up. Confirmed by logging both the raw `hit_test` result and `tree.button`'s `NodeId`
   side by side — they differed by exactly one slotmap slot, which is what a parent/child pair
   looks like, not a random miss.

**One more, non-crashing but real:** the first rendered frame was visibly washed out —
`bg-slate-950` (rgb 2,6,23, nearly black) came out a clearly lighter navy. The surface had picked
`Rgba8UnormSrgb` (first in `caps.formats`), which tells the hardware "what you write is linear,
gamma-encode it on the way to the texture" — but `vello_hybrid`'s own bundled example
(`render_to_file.rs`) targets plain `Rgba8Unorm`, meaning its pipeline already outputs sRGB-encoded
bytes matching `twcore::Color`'s own values verbatim. Requesting the `*Srgb` format applied a
second, unwanted gamma correction. Fixed by preferring `Rgba8Unorm`/`Bgra8Unorm` when the surface
offers one. Re-verified visually against §3's software-rendered PNG — colors now match.

**Real-device run (Moto G54 5G, arm64-v8a, Android 15) — done, same physical phone phase 0/1/2
used for their own real-hardware confirmations.** This is the run that matters most: everything
above was the emulator, and phase 0's own README is blunt that an emulator pass is evidence, not
proof — this project's actual promise is "installs a wheel, works on the user's phone." Connected
mid-session (dropped once over USB partway through — matches phase 1's own note that a real-device
adb connection is the fragile part of this whole loop, not the code); reconnected and the full
pass below is against real silicon, not SwiftShader.

**A fifth real bug, found only on real hardware — no emulator, including this session's, would
ever have surfaced it:**

5. **wgpu's default device limits exceed what this actual mobile GPU supports.** First real-device
   attempt: adapter probing found a real GPU (`PowerVR BXM-8-256`, integrated, Vulkan backend —
   this Dimensity-family SoC pairs with Imagination's PowerVR, not Adreno, which is worth noting
   since plan1.md §5/§9 names "Adreno" specifically when flagging this risk class; the actual
   silicon in the phone this project has been using all along is a different vendor, and the
   fallback story has to hold for it too, not just the named one) — but `request_device` itself
   failed: `Limit 'max_inter_stage_shader_variables' value 16 is better than allowed 15`. wgpu's
   default `required_limits` (`Limits::default()`, desktop-shaped) is a *floor* the adapter must
   clear, not a target it gets downgraded to meet, and this real, current, mid-range phone's
   driver supports one less than the default asks for, on exactly one limit. The render harness
   *correctly fell back to `vello_cpu`* — plan1.md §5/§9's fallback design worked exactly as
   intended, on the first real device it ever ran on, for a reason nobody had specifically
   predicted (the plan named a different GPU vendor's issues). That's the fallback earning its
   keep, not a failure. Fixed anyway, because the fallback existing doesn't mean the GPU path
   should stay unreachable on ordinary hardware: `request_device` now asks for `adapter.limits()`
   — what this specific adapter actually reports supporting — instead of the fixed desktop
   baseline. Rebuilt and re-ran on the same device: the GPU path now initializes
   (`GPU backend initialized (wgpu/vello_hybrid)`) where it previously fell back.

**Full pass on real hardware, both rendering paths, each independently verified:**

- **GPU path** (after the limits fix): adapter/device init, frame renders, visually confirmed
  correct (matches the emulator screenshot and §3's software PNG pixel-for-pixel in appearance —
  same dark navy, same indigo button, same centering). Three real taps on the physical
  touchscreen took the on-screen count `0 → 1 → 2 → 3`, each a real touch → hit-test →
  ancestor-walk → state update → relayout → repaint → present cycle on real silicon.
- **CPU fallback path** (`vello_cpu` + `ANativeWindow_lock`, forced on temporarily by hand-editing
  `try_init_gpu` to always fail, rebuilt, tested, then reverted — chosen over the
  `ZKITE_FORCE_CPU_RENDERER` env var because there's no straightforward way to inject a process
  environment variable into an already-installed Android app via `adb`, and a temporary source
  edit is at least as direct a verification): renders correctly (visually indistinguishable from
  the GPU path) and responds to a real tap (`count -> 1`) on the same physical device. This closes
  a real gap in this session's own rigor, not just the code's — the fallback path had been built
  and never actually run anywhere before this. "It compiles" was never the bar the rest of this
  document holds anything else to; the fallback path doesn't get an exception.
- **Rotation / surface lifecycle**: no crash across repeated portrait ⇄ landscape cycles, matching
  the emulator. **Real-hardware-specific finding, not a bug**: on this device, rotating fires a
  full `onDestroy()` (confirmed by the `DestroyWindow` → `zkite-render thread: Stop received` →
  fresh `boot(): spawning render thread` sequence in logcat), whereas the emulator's rotation
  preserved the render thread across the Activity recreation (`boot(): render thread already
  running, ignoring`). Concretely, the tap counter — which is this harness's own ad-hoc stand-in
  state, not real app state — survives rotation on the emulator and resets to 0 on this real
  device. Neither is "wrong": Android's contract only guarantees the Activity is destroyed and
  recreated on an undeclared config change, not whether the hosting *process* survives that, and
  real OEM behavior is where that ambiguity actually gets resolved one way or the other. Worth
  recording precisely because it's exactly the kind of emulator/real-device divergence phase 0
  already found once (arm64 execution) and flagged as a standing category, not a one-off — **this
  harness's own state did not need to survive it, but the real `zkite_pybridge` state bridge built
  since (see §5's intro and `phase_3/pybridge/`) inherits this exact divergence and has not yet
  been tested against it explicitly — still an open gap, not assumed away.**
- **Clean shutdown**: back button → `DestroyWindow` → `Stop received`, no hang, no crash, same as
  the emulator.

---

## 5. What's left, in the order §11's "do the boring hard things first" argues for

§3, §4, and §6 are done — real GPU rendering (with a verified CPU fallback), the real `zkite_py`
widget tree, and scrolling/gesture disambiguation/momentum are all up on both the emulator and/or
real hardware, interactive, with a clean surface lifecycle. §7 (IME) is now *partially* done — see
that section for the full account; summarized in its own item below rather than rewritten here.
Recorded here so the next session picks up the actual remaining roadmap instead of rediscovering it:

1. **IME — done except AccessKit/Autofill and the SwiftKey/Samsung/MIUI slice of the keyboard
   matrix (§7.9–§7.13).** Focus, the vendored `InputConnection`, live typing, blur,
   tap-to-position-cursor, and the selection-overlay toolbar were already done (§7.4/§7.7/§7.8).
   The most recent session closed out insets/resize compensation (a focused field now panned above
   the keyboard, §7.9), real per-field `type`/`secure` → `EditorInfo` mapping with password masking
   on *both* render paths — the primary GPU-painted field and the selection overlay's own real
   `EditText` (§7.10), Unicode-correct (not ASCII-only) word selection plus double-tap-to-select
   (§7.11), and a **live** (not just traced) verification of the cross-field selection-overlay race
   §7.8 could previously only verify by tracing (§7.12). Still open: **AccessKit/a11y-sync and real
   Autofill** (item 2 below — phase 0's own note that these "reduce to the same missing piece," a
   virtual semantics tree, still applies); **the keyboard matrix beyond Gboard** — Samsung
   Keyboard/MIUI are genuinely uninstallable on this Motorola test device (OEM-exclusive), and
   SwiftKey was available but skipped by explicit user choice this session, so it remains untested,
   not ruled out (§7.13); and the **two-long-presses-to-reach-the-toolbar** gap, which this session
   confirmed (live, not assumed) that `EditText.performLongClick()` does *not* close (§7.11) —
   still no known API for it. §7.6's rendering bug is **solved** (§7.14) — it was a
   `SurfaceView` inset/geometry bug, not a paint bug: `setPadding` does nothing to a
   `SurfaceView`, so the surface silently moved up by the status-bar height after the first
   IME show/hide and never moved back. A later session, with no device available, ruled out
   `zkite_renderer`/`zkite_pybridge`/`vello_cpu`'s own rasterization entirely via a new host-only
   crate (`phase_3/repro_7_6/`) that pushes the same 3-node repro through the real paint pipeline
   rather than just layout metadata — negative result, narrowing what's left to the Android-only
   surface/resize/GPU tail; see §7.6's own update for the full account.
2. **AccessKit → `AccessibilityNodeProvider` — done for the core round trip, verified live; see
   §8 for the full account.** `zkite_pybridge::BuiltTree` now builds a real `accesskit::Node` per
   element (role/label/value/actions from the same `RawNode` tag/attrs `build_node` already reads,
   e.g. `Role::Switch`+`Toggled`, `Role::Slider`+numeric range, `Role::PasswordInput`), and
   `gpu_harness`'s render thread stamps in real bounds from `positioned()` and pushes the result to
   Android through `accesskit_android::InjectingAdapter` — no hand-rolled JNI glue, no androidx
   dependency (unlike `vendor/gametextinput`), installed and verified live: `uiautomator dump`
   against the real demo app shows every widget with the *correct* real Android accessibility
   class (`android.widget.Button`/`ToggleButton checked="true"`/`SeekBar`/`EditText
   password="true"`), correct labels, and correct bounds, and real TalkBack successfully attaches
   to `ZKiteSurfaceView`'s virtual hierarchy with no crash. **Not** verified live: an actual
   TalkBack gesture (double-tap-to-activate) driving `ActionHandler::do_action` end to end — see
   §8's own account of why (`adb`-synthesized touches don't engage touch exploration on this test
   device) and what would close that gap. Phase 0's own note that Autofill and a11y "reduce to the
   same missing piece" (a virtual semantics tree) still applies — Autofill itself remains
   unbuilt, but the tree this milestone built is the reusable piece a future session would build
   it from.
3. **Virtualised lists — done, verified live on a 10,000-row list against a non-virtualised
   control on the same device; see §11 for the full account.** The `List` widget plan1.md §8 names
   and phase 1 deferred now exists and works on both backends, windowing via two fixed-height
   spacers that keep total content height invariant, so `zkite_renderer` and the gesture/fling code
   never learn that a list is virtualised at all. Measured: 50,006 → 296 nodes per frame, ~3,850ms
   → ~30ms per scroll frame, 6.34s → 1.48s to first frame. **~30ms is ~34fps, not 60** — what's
   left is per-frame parley reshaping, not row count, and a shaped-run cache is the named next
   lever (§11.5/§11.7). **That cache is now built and verified (§11.9): ~42fps, with the frame
   now dominated by GPU present and the per-scroll-frame Python call rather than by text.**
   Two findings larger than the item itself came out of it: §11.6 (no
   C-extension stdlib module was importable from `zkite_py` on this backend — latent since phase 1;
   **now root-caused and fixed on both backends, §11.8**) and §9.5 (the conformance tolerance was
   hiding a Tailwind v3-vs-v4 palette divergence across every saturated colour).
4. **Overscroll/rubber-band feedback at a scroll bound.** §6's fling stops dead the instant it hits
   `0`/`max_scroll_y` rather than bouncing — a real gap against both platforms' native feel (an
   `EdgeEffect` glow on Android, a rubber-band stretch on iOS), deliberately deferred rather than
   attempted alongside momentum itself; see §6's own note.
5. **Per-widget conformance tests against the WebView backend — done, see §9 for the full
   account.** All 12 phase-1 widgets, real Chromium (Playwright) vs. `zkite_renderer`+`vello_cpu`,
   pixel-diffed with a documented tolerance. 8 of 12 come back effectively clean (<1% mismatch,
   pure anti-aliasing noise); 4 show real, now-quantified divergences — one of them (non-regular
   font weights not rendering natively) was already suspected and listed in this document's own
   known-gaps (§3, §5's "smaller gaps" list below) but never actually measured until this
   milestone; two more (Slider's WebView styling, Switch's track color/size) were previously
   undocumented and are new findings from this work, not confirmations of an old one. **A
   follow-up session fixed Slider and Switch cleanly (both now <1.3%) and registered a bold font
   face in `zkite_renderer`, closing the weight-selection bug for real — but doing so surfaced a
   materially bigger, previously-undocumented gap: native has no font-*family* resolution at all,
   which is what `text`/`button`'s own raw percentages are now dominated by. See §9.4 for the full
   account, including why `button`'s number didn't move at all (correctly, per CSS weight-fallback
   rules) and isn't a sign the fix failed.**

**Smaller, real gaps carried forward rather than silently fixed, same "document, don't hide"
convention as every prior phase:**

- **A touch that arrives between `DestroyWindow` and the next `NewWindow`** (e.g. a tap landing
  right as rotation begins) still updates `count` and calls `redraw()`, which no-ops on the
  missing window rather than being dropped outright — so the increment "counts" with nothing
  on-screen to show it, and only becomes visible once the next `NewWindow` arrives. Not observed
  to cause any actual problem (nothing crashes, final state is consistent), but worth naming
  rather than leaving implicit.
- **`onSurfaceDestroyed`'s 3-second ack timeout is a last-resort escape hatch, not a proof of
  safety.** If the render thread is ever genuinely wedged past 3s, `surfaceDestroyed` returns
  anyway (so as not to hang Android's own callback thread forever) while the render thread may
  still be mid-cleanup of a now-invalidated `ANativeWindow` — undefined behavior if it then
  touches it. Never observed to fire in this session's testing; recorded because "it hasn't
  happened yet" isn't the same claim as "it can't."
- **No per-app icon, resource qualifiers, or anything else phase 0/2 already scoped out** — this
  harness reuses the same single bundled icon convention as every other ZKite shell.
- ~~**The native backend has no device-density scaling at all** (§12.5.2)~~ — **fixed,
  see §13.1.** Layout is now in dp and the whole scene is scaled once at paint; the
  same app renders at the same size on both backends on one phone. (The Moto G54's
  ratio is 2.5, not the 2.625 §12.5.2 assumed — it reports 400dpi, so the pre-fix
  size was 40% rather than 38%.)
- **Text wrapping, `text-align`, borders/shadows/rings** — same list §3 already carried forward,
  still true here since paint code for both backends shares the same `PositionedNode` shape.
  **Bold font weight is fixed** (§9.4), and so are **font-*family*** and **`line-height`**
  (§9.7 — both had the identical bug: parsed, inherited, and then never pushed to parley by any
  shaping site), along with `text-align`, `italic`, `letter-spacing` and `tabular-nums`, audited in
  one pass once the pattern was clear. **Still open:** `decoration` (underline/line-through) needs
  paint support in both backends, not just a pushed property; `italic` has no italic face to select
  yet; and `text-align` doesn't cover content-sized wrapped text — see §9.7's own known gaps. Text
  wrapping and borders/shadows/rings remain as this list originally had them.

---

## 6. Scrolling, gesture disambiguation, and momentum

**Status: done, verified live on real hardware (Moto G54 5G) with a real, 60-row scrollable
list.** Split across the two crates the rest of this phase already splits paint/layout from
input/animation: `zkite_renderer` owns overflow/clip/scroll-offset math (no touch, no time — it
answers "what does this look like at offset X," nothing about how X changes), `gpu_harness`'s
render thread owns the touch state machine and, as of this session, the momentum animation.

### 6.1 Overflow, scroll offset, and clipping (`zkite_renderer`)

An `overflow-y-scroll`/`-auto` container (`taffy::style::Overflow::Scroll`, both map to the same
taffy variant — confirmed against `phase_0/spike3_twcore`'s parser) clips its descendants to its
own box; `Renderer::positioned`'s tree walk (`renderer/src/lib.rs`) takes a `scroll_offsets: &
FxHashMap<NodeId, (f32, f32)>` and, for each scrolled container, shifts every descendant's
absolute rect by that offset while leaving the *container's own* rect exactly where layout put it
— a scroll never moves the viewport, only what's inside it. `Renderer::max_scroll_y` answers "how
far can this go" from each child's real (taffy-computed, unshrunk) layout size, and
`Renderer::is_scrollable_y` lets a caller doing gesture work ask "would dragging here scroll
something" while walking ancestors, without pulling `taffy::style::Overflow` into its own
dependency graph. `renderer/src/lib.rs`'s
`scroll_offset_shifts_children_and_clips_without_moving_the_container` test locks in both halves
of this at once: the container's rect is invariant under scrolling, its children's aren't, and a
child that's scrolled fully or partially out of view gets a `clip` rect matching the container's
box, not its own.

One real bug this depended on, found on a real device before this session and already fixed at
its source: a `flex: 1` scroll region defaults to `min-height: auto` (real flexbox spec behavior,
not a taffy quirk — confirmed by an isolated `zkite_renderer` reproduction), which silently grows
it to fit *all* its content instead of the space actually available, making `max_scroll_y` compute
`0` — there is nothing to scroll because nothing is clipping. `zkite_py`'s `ScrollView`
(`phase_1/zkite_py/src/zkite/widgets.py`) bakes `min-h-0` into its `implied_classes` for exactly this
reason; its own doc comment has the full account, including the one-level-deeper gotcha (every
*intermediate* `flex-1` ancestor between a `ScrollView` and its nearest non-growing container needs
the same override — true in real CSS too, not a ZKite-specific rule).

### 6.2 Gesture disambiguation (`gpu_harness`)

`ZKiteSurfaceView` forwards raw `ACTION_DOWN`/`MOVE`/`UP`/`CANCEL` across JNI unchanged
(`Command::TouchDown`/`TouchMove`/`TouchUp`/`TouchCancel`); `render_thread.rs`'s `Gesture` decides
what they mean. `TouchDown` resolves, once, both what a tap would hit (`Renderer::hit_test`) and
the nearest scrollable ancestor above it (walking up via `Renderer::parent` — the same
target-vs-currentTarget walk `dispatch_tap` uses for a button's own label). `TouchMove` compares
total movement against `touch_slop` — Android's own `ViewConfiguration.getScaledTouchSlop()`, read
once in Java and passed down at `boot()`, not a guessed constant — and only starts moving
`self.scroll_offsets` once that's exceeded, clamped to `[0, max_scroll_y]`. `self.scroll_offsets`
is keyed by `zkite_py` node id rather than taffy `NodeId` because every redraw rebuilds the taffy
tree from scratch (fresh `NodeId`s each time) but scroll position has to survive that rebuild;
`zkite_pybridge::BuiltTree::scroll_offsets_for` converts the persistent map back to the current
build's `NodeId`s each time `positioned`/`hit_test` need it.

### 6.3 Momentum (new this session)

The one piece §4/§5's original writeup flagged as explicitly missing: a released drag used to stop
dead where the finger lifted. Now it decelerates, the way every native scroll view does.

**Velocity estimation.** `Gesture::history` records `(Instant, y)` on every `TouchMove`, trimmed to
the last `VELOCITY_WINDOW` (100ms) — a short recent window, not the whole drag, so a fast flick
followed by a deliberate slow-down-before-release reads as a slow release (correct) rather than
getting dragged up by the fast part earlier in the gesture. At `TouchUp`, `maybe_start_fling`
takes the oldest and newest surviving sample; below `MIN_VELOCITY_SAMPLE_DT` (8ms) the estimate is
too noise-prone to trust and nothing launches. Below `MIN_FLING_START_VELOCITY` (80px/s), a
lift-off reads as "the drag stopped deliberately," not a flick, and again nothing launches.

**Physics.** `Fling` decays velocity exponentially: `v(t) = v0 * exp(-t/FLING_TAU)`, `FLING_TAU =
0.325s`. This is a deliberate choice, not a derivation — unlike `touch_slop`, there's no Android
API to pull a real fling-friction constant from on this thread (it owns no `View`/`Context`, and
piping one across JNI for a single float wasn't worth it), so the constant is hand-tuned to feel
roughly like a native Android/iOS fling: a fast flick (~3000px/s) travels `v0 * FLING_TAU ≈ 975px`
— the exact integral of the exponential, not an approximation — decaying below
`MIN_FLING_STOP_VELOCITY` (20px/s) in under a second. `tick_fling` applies this per-frame using the
*real* elapsed `dt` since the last tick (not a fixed frame-count step), so a late tick (e.g. a
`Resize` briefly holding up the render thread) doesn't cost the animation any distance or duration.

**Driving the animation without a Choreographer.** This thread has never had a per-frame callback
— `run()`'s loop blocks indefinitely on `rx.recv()` between Android events, deliberately (see the
module doc's original reasoning: a dedicated OS thread makes a blocking recv both simpler and more
responsive than polling). A fling needs periodic wakeups nothing will ever send, so `run()` now
switches to `rx.recv_timeout(FRAME_INTERVAL)` (16ms) *only* while `state.fling.is_some()`, and a
timeout drives one `tick_fling()`; the instant the fling ends the loop reverts to the original
indefinitely-blocking `recv()`. An idle app (no fling, waiting on the next touch) never wakes up
for nothing, exactly as before — the polling exists exclusively for the duration of an actual
animation.

**Interruption.** `Command::TouchDown` unconditionally clears `self.fling` before resolving the new
gesture — a finger landing on a still-coasting list grabs it in place immediately, the same
fling-cancel-on-`ACTION_DOWN` contract every native Android scroll view honors. `DestroyWindow`
also clears it, so a fling doesn't keep ticking (and waking the thread every 16ms) after the
window it was animating for is already gone. `TouchCancel` never launches a fling at all — same
reasoning as `TouchUp` not treating a cancelled gesture as a tap, one level up.

**Deliberately not done, carried into §5:** no overscroll/rubber-band bounce at a scroll bound —
`tick_fling` clamps and stops dead the instant `entry.1` would exceed `[0, max_scroll_y]` (§5 item
4). No horizontal fling — matches `max_scroll_y`/`is_scrollable_y`'s own existing y-axis-only scope
(no `zkite_py` widget asks for horizontal scrolling yet). `FRAME_INTERVAL` is a fixed 16ms, not tied
to the display's actual refresh rate — fine on this session's hardware, worth revisiting if a
future device shows tearing/stutter against a high-refresh-rate panel. `FLING_TAU`/the velocity
thresholds are hand-tuned (see above), not calibrated against a device-reported constant the way
`touch_slop` is.

### 6.4 Verification

- **Host:** `zkite_renderer`'s existing test suite (6 tests, including §6.1's scroll/clip test)
  still passes unchanged — this session's changes are entirely in `gpu_harness`, which has no
  host-testable path (`ndk` is an Android-only dependency pulled in unconditionally by
  `render_thread.rs`, so this crate has never had host tests — see §4's own build-feasibility
  note on why the GPU/window code can't split cleanly from touch/animation state without a larger
  refactor than this session's scope).
- **Cross-compile:** `scripts/build.sh arm64-v8a` — clean release build against the real
  `aarch64-linux-android` NDK toolchain, no errors.
- **Real device (Moto G54 5G, the same phone §4 verified on):** installed the demo app
  (`gpu_harness/demo_app/main.py`'s `ScrollView` of 60 rows) fresh (a signature mismatch against
  the previous install — expected, since `build_apk.sh` generates a new ephemeral test key every
  run with no `--pem` given — required an uninstall first, not a real bug). A fast synthetic swipe
  (`adb shell input swipe`) launched a fling (`fling: launching at 10679px/s (max_scroll_y 698)`
  in logcat), animated smoothly, and landed exactly on the list's last row (item 59) with the
  scroll clamped cleanly at the bound — no overscroll glitch, no crash, confirmed both in logcat
  and a screenshot. During the same session, real organic human touches on the device (erratic,
  multi-second, direction-reversing drags — visibly not synthetic input, confirmed by the wandering
  raw coordinates in logcat) also produced coherent, correctly-signed flings with no panics — a
  stronger signal than a scripted swipe, since a real finger's release velocity is far messier than
  `adb`'s linear interpolation. A full-session logcat sweep for `panic`/`fatal`/`crash`/`ANR`
  across the entire test run came back clean, and the process stayed alive and responsive
  throughout.

### 6.5 A real bug §6.4's verification missed, found immediately by an actual user: input backlog under fast scrolling

**Status: found, fixed, re-verified on the same device.** §6.4's synthetic single-swipe testing
looked clean, but real usage — a person actually scrolling the list up and down fast, right after
this was shipped — didn't match it: the list visibly lagged behind the finger, and then, after the
finger lifted, kept scrolling on its own for a few seconds, playing out the whole gesture in
fast-forward. Exactly the kind of gap §4's own discipline exists to catch ("it compiles" was never
the bar; a real device, doing the real thing, is) — this one just needed a real user doing the real
thing rather than a scripted swipe, because the bug only shows up under sustained, fast, repeated
input, not one clean gesture.

**Root cause.** Every `TouchMove` costs a full `redraw()` — rebuild the tree, relayout, reshape
every text run (60 of them, in the demo list), paint. `Native.onTouchMove`'s JNI call sends into an
ordinary unbounded `mpsc::channel` and never blocks the calling (UI) thread, so nothing pushes back
on Android when redraws can't keep up with input. Under fast, sustained scrolling, `Command`s queue
up faster than they drain, and the render thread — having no way to tell a stale queued position
from the current one — processes the entire backlog strictly in FIFO order: visibly behind while
the backlog grows, then grinding through all of it, one stale position at a time, after the finger
has already lifted and stopped adding new events. Latent since §6.2's original gesture code (this
isn't a bug momentum introduced), but momentum is what made it obvious — a fast fling-provoking
scroll is exactly the input pattern that produces the backlog, and watching a list keep scrolling
itself for seconds *after* your finger leaves the screen reads as broken in a way that plain
"the drag feels a bit behind" doesn't.

**Fix.** `run()`'s loop (`render_thread.rs`) now coalesces: whenever the next command to handle is
a `TouchMove`, it drains `rx` via non-blocking `try_recv` — never waiting for more input, only
consuming what's *already* queued — replacing the pending move with each newer one it finds, until
either the channel is empty or a non-`TouchMove` command turns up. That command (most often the
gesture's own `TouchUp`) is saved in a one-slot `pending` buffer and handled next, in its original
order, rather than dropped. Net effect: `redraw()` only ever runs against wherever the finger
actually is *right now*, no matter how large a backlog briefly built up — a stale position is
simply superseded before it's ever acted on, not queued behind.

**Re-verification, same device, harder than §6.4's original pass:** a burst of 10 rapid,
alternating-direction synthetic swipes (60ms each, fired back-to-back with no gap) produced 10
correctly-signed flings (alternating roughly ±14–23K px/s, matching each swipe's own direction),
every one logged within tens of milliseconds of its own `TouchUp` — no growing lag, no post-lift
tail. Confirmed on the process itself, not inferred: `pidof` showed it alive throughout, and a
logcat sweep for `panic`/`fatal`/`crash`/`ANR` across the whole stress run was clean.

**One more real, separate thing this session's testing surfaced, not fixed, worth naming rather
than leaving implicit:** touching the screen very soon (well under a second) after the activity
launches can silently no-op that first gesture — `Command::TouchDown`'s handler returns early if
`self.built` is still `None` (nothing rendered yet to hit-test against), and every subsequent
`TouchMove`/`TouchUp` for that same gesture then also no-ops (`self.gesture` was never set). Not
today's bug (that reproduced in steady-state use, well after launch, same as this session's
10-swipe stress test — which waited for the window to settle first and had no such issue) and not
attempted here — recorded because it's a real, reproducible gap in the same input path, not a
guess.

## 7. IME — focus, a real `InputConnection`, live typing, and blur

First slice of §5 item 1, per §11's "do the boring hard things first" and phase 0's own
known-gaps writeup. Scope, deliberately: get `InputConnection`/composing-region editing genuinely
working end to end (tap a `TextField` → keyboard shows → typed text renders live → tap elsewhere
→ keyboard hides, value persists) before touching insets/resize, selection UI, or Autofill/a11y —
see §5 item 1 for what's still left. Everything below was verified live on the same Moto G54 5G
real device §4/§6 used, not just compiled.

### 7.1 Why `GameTextInput`, and what that costs without Gradle

plan1.md §9 and phase 0's own README already made the call: don't hand-roll
`android.view.inputmethod.InputConnection` — keyboard-vendor-specific bugs are exactly the long
tail Google's `androidx.games:games-text-input` exists to absorb instead of ZKite's own surface
area. Phase 0 also already scoped the mechanism: "the C++ file is small enough to fold into the
existing `cc`-crate/NDK cross-compile in `build.rs`." That held — `vendor/gametextinput/include/
game-text-input/gametextinput.cpp` (~390 lines, Apache-2.0, fetched live from
`dl.google.com/android/maven2/androidx/games/games-text-input/3.0.1/`) compiles cleanly into
`libzkitegpuharness.so` via a new `gpu_harness/build.rs` + the `cc` crate, no different in kind
from how `phase_0/spike1` already links against a prebuilt `libpython3.14.so`.

**What phase 0 didn't anticipate, because it's downstream of not using Gradle specifically:**
`games-text-input`'s AAR ships only compiled `.class` files (`classes.jar`), and its
`InputConnection`/`Listener` classes reference `androidx.core.view.WindowInsetsCompat`/
`graphics.Insets` — a second, separate androidx dependency. Fetched the same way
(`gpu_harness/scripts/fetch_androidx_core.sh`, `androidx.core:core:1.13.1`, not committed —
1.2MB/874 classes is closer to `phase_2/vendor/protoc`'s "fetch, don't commit" bucket than
`vendor/gametextinput`'s "small enough to vendor outright" one). Both jars go on javac's
classpath; only `vendor/gametextinput`'s and (as §7.4 explains) three hand-written `R$id` fields
worth of `androidx-core` actually get dexed into the shipped APK.

### 7.2 Architecture: `GameTextInput` stays on the UI thread, only decoded data crosses to the render thread

The obvious design — attach the render thread to the JVM (same pattern §1 already uses for
`ZKiteSurfaceView` method calls) and run `GameTextInput_init` there too, since its C API caches
one `JNIEnv*` for the object's whole lifetime and every later call has to use that same thread —
**doesn't work**, and the reason isn't obvious from the header. Found by reading
`gametextinput.cpp` itself (the AAR ships no docs beyond the header): the constructor calls
`env->FindClass("com/google/androidgamesdk/gametextinput/State")` by name, and `FindClass`,
called from a thread the JVM didn't itself create (a raw `AttachCurrentThreadAsDaemon`-ed OS
thread, exactly what the render thread is), resolves against the **boot classloader**, not the
app's own — so it reliably fails to find any app-defined class. First real symptom: a bare
`FATAL SIGNAL 6` on launch, no exception message reaching Java at all, `GameTextInput::
GameTextInput` at the top of the native stack.

The fix is architectural: `GameTextInput_init`/`setInputConnection`/`processEvent` now only ever
run from a real JNI entry point Android calls directly — for this app, always the UI thread
(`Native.boot`, `ZKiteSurfaceView`'s `onCreateInputConnection`/`Listener.stateChanged`
callbacks). `lib.rs`'s `TEXT_INPUT` static (`Mutex<Option<text_input::TextInput>>`) owns the one
`GameTextInput` handle; `render_thread::State` never touches it. `Native.onTextInputEvent`
decodes the IME's `State` object into a plain, owned `TextInput::TextState` (`String` + two
`i32`s, no JNI types at all) on the UI thread, *then* sends that — not any `jobject` — to the
render thread via `Command::TextInputEvent`, sidestepping the cross-thread-`JNIEnv` question for
that path entirely. `text_input.rs`'s module doc has the full account, including why this is
different from `resolve_surface_view_methods`/`call_surface_view_method` (still legitimately on
the render thread's own attachment): `GetObjectClass`+`GetMethodID` on an object already held
read the object's actual runtime type directly, no classloader-by-name lookup involved — the bug
was specifically about resolving *classes by name* from the wrong thread, not about JNI-from-a-
background-thread generally.

### 7.3 Four more real bugs, each found only by actually running on the device

Same pattern every prior phase hit at least once (phase 0's `PYTHONHOME`, phase 1's morphdom
focus bug, phase 2's `_android_support`/`main.pyc`, §4's wgpu instance/alpha-mode/limits bugs,
§6.5's input backlog) — "it compiles, host tests pass" was never the bar anything else in this
document gets held to, and IME didn't get an exception:

1. **Missing `libc++_shared.so`.** `gametextinput.cpp` pulls in the NDK's C++ runtime; clang++
   links it as a shared library by default, so `libzkitegpuharness.so` gained a `DT_NEEDED` on
   `libc++_shared.so` that wasn't staged anywhere in the APK. First launch after adding the
   native compile: `UnsatisfiedLinkError: library "libc++_shared.so" not found` at
   `System.loadLibrary`, before a single line of app code ran. Fixed by copying it from the NDK's
   own sysroot (`toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/<triple>/`) into
   `lib/<abi>/` in `build_apk.sh`, the same way `libpython3.14.so` already is.
2. **`CalledFromWrongThreadException` from `requestFocus()`, silently swallowed.** The render
   thread calls `ZKiteSurfaceView.focusAndShowKeyboard()` via a plain JNI instance-method call —
   not the `FindClass` problem (§7.2), a *different* one: `View`/`ViewRootImpl` assert their own
   methods run on the UI thread, a rule with nothing to do with JNI mechanics at all, and it
   applies just as much to a JNI call from a background thread as to ordinary background-thread
   Java code. The exception fired *inside* `requestFocus()`, was caught by ART at the JNI
   boundary, and — because a pending Java exception after a JNI call doesn't crash the native
   caller by itself — was completely invisible until explicit logging in the Java method plus an
   `ExceptionCheck`/`ExceptionDescribe` added to `call_surface_view_method` on the Rust side
   surfaced it. Fixed the standard way: `focusAndShowKeyboard`/`blurAndHideKeyboard` now check
   `Thread.currentThread()` against the view's own `Handler`'s looper thread and `post()`
   themselves back if not already there.
3. **`Listener.stateChanged` never fired, even after the thread-hop fix.** `gti.InputConnection`'s
   own debug logcat lines showed real `commitText`/`setComposingText` calls happening correctly on
   every keystroke — the edits were real — but nothing reached `Native.onTextInputEvent`. Traced
   (via `javap` against the vendored class; the AAR ships no source) to
   `InputConnection.stateUpdated`: it only calls the listener when a private `mSoftKeyboardActive`
   field is `true`, and — despite the name strongly suggesting otherwise — `setSoftKeyboardActive`
   *does not set that field*; it only drives `requestFocus`/`showSoftInput`/
   `hideSoftInputFromWindow`. The field's *only* writer, found by grepping the whole class for
   `putfield` on it, is `InputConnection.onApplyWindowInsets(View, WindowInsetsCompat)`, comparing
   the IME's real visibility (read from the system-delivered insets) against its own cached value.
   Nothing was calling that method at all. Fixed by giving it the real thing: `ZKiteSurfaceView`
   can't install its own `OnApplyWindowInsetsListener` (a view has exactly one, and `ZKiteActivity`
   already needs it for insets-as-padding), so `ZKiteActivity`'s existing listener now also calls a
   new `ZKiteSurfaceView.forwardWindowInsets(WindowInsets)`, which is what actually drives
   `mSoftKeyboardActive` off *real* system insets deliveries — not a synthesized signal.
4. **`NoClassDefFoundError: androidx/core/R$id`, from `forwardWindowInsets` itself.** The two-arg
   `WindowInsetsCompat.toWindowInsetsCompat(insets, view)` overload touches `ViewCompat`, whose
   static initializer reads 32 `R.id.accessibility_custom_action_N` fields (opaque
   `View#setTag(int, Object)` keys, never looked up against a real resource table) — and that
   `R$id` class is normally generated by AAPT2 from the AAR's `res/values/ids.xml` at ordinary
   Gradle build time. This pipeline has no AAPT2/resource-merging step at all (the whole point,
   per phase 0's README). First fix — switch to the one-arg overload, which is the two-arg one
   called with `view=null` and skips `ViewCompat` entirely when the view is null — got past *that*
   call site, but `InputConnection.isSoftwareKeyboardVisible()` (called from inside
   `onApplyWindowInsets` itself) calls `ViewCompat.getRootWindowInsets(view)` unconditionally,
   hitting the same static-initializer failure a different way. Since `ViewCompat` genuinely can't
   be avoided, the actual fix is `gpu_harness/java/androidx/core/R.java` — a small, hand-written
   stand-in providing exactly the 32 fields `javap` against the real `androidx.core:core:1.13.1`
   classes showed `ViewCompat.<clinit>` needs (values are arbitrary-but-distinct 32-bit ints;
   correctness here only requires uniqueness, never compared against a real resource table),
   compiled and dexed in alongside everything else. If a future dependency on more of
   `androidx.core` needs more fields, the class's own doc comment says how: `javap` the real
   class, find the exact missing field, add just that one.

### 7.4 Verification

Live, on the same Moto G54 5G real device §4/§6 used (not the emulator this time — real hardware
throughout, since IME is exactly the kind of feature phase 0's own findings say won't surface its
real bugs on an emulator's AOSP keyboard):

- **Focus → real keyboard.** Tapping the demo app's `TextField` (`gpu_harness/demo_app/main.py`,
  now with `on_change=lambda v: s.update(name=v)` added so the full round trip is exercised, not
  just the native-side caret) focuses it and brings up the real on-device Gboard — confirmed both
  visually (screenshot) and via `dumpsys input_method`: `mInputShown=true`, `mServedView` is
  `ZKiteSurfaceView`, `mServedInputConnection` is the real `gametextinput.InputConnection` object.
- **Live typing.** Real taps on Gboard's own rendered keys (not `adb shell input text` — see the
  note below) for "h" then "i" produced, in order: `gti.InputConnection`'s own
  `Commit`/`setComposingText`/`setSelection`/`finishComposingText` debug lines, then
  `ZKiteSurfaceView.stateChanged: text=h`/`text=hi`, then `Native.onTextInputEvent` decoding the
  same text — and the field visibly showed `hi`, live, with a caret, on screen.
- **Blur.** Tapping elsewhere (the Increment button) called `blurAndHideKeyboard`, logged
  `onApplyWindowInsetsfalse` (confirming the real insets round trip, not just the request), hid
  the keyboard, and left `hi` persisted in the now-unfocused field's own display — reading back
  from `RawNode.attrs["value"]`, i.e. proving `on_change` actually reached the Python store: a
  fresh tree came back from `s.update(name=v)` with `value="hi"` already baked in, not just the
  native-side override remembering what was typed.
- **No crashes across the whole sequence** — confirmed via `pidof` staying constant and a logcat
  sweep for `panic`/`FATAL`/`crash`, same bar every prior phase's own verification held to.

**Real finding, not a bug:** `adb shell input text "..."` does **not** exercise this integration
at all — it synthesizes raw `KeyEvent`s through Android's normal key-dispatch path, which a plain
`SurfaceView` with no key listener wired up has nowhere to send. A real soft keyboard's taps call
`commitText`/`setComposingText` directly on the served `InputConnection` over Binder, a
completely different path that `input text` never exercises. Every verification above used real
synthetic taps on Gboard's own rendered key positions instead, which *does* go through the real
path — worth remembering before trusting an `input text`-based test of any future IME work here.

**Caret rendering was a documented approximation, not a real read of `GameTextInputState.
selection`, as of this section.** `render_thread.rs`'s `caret_rect` always placed the caret right
after the last shaped glyph (or the field's left edge if empty) — correct for a freshly-focused or
append-only field (this session's whole test scope at the time), wrong the instant a user tapped
mid-string. Fixed in §7.7 below: `caret_rect` now reads `focused_selection` (itself now real,
tap-hit-tested state, not a guess) and calls `zkite_renderer`'s parley-`Cursor`-backed
`text_caret_geometry` for the real glyph position at that byte offset.

### 7.5 Known gaps (IME specifically — see §5 item 1 for the prioritized list)

- **Only Gboard tested.** Phase 0's own flagged keyboard matrix (Samsung Keyboard, SwiftKey,
  MIUI) — precisely where historical `InputConnection` bugs concentrate — is still untested.
- **No insets/resize compensation.** `ZKiteSurfaceView.onImeInsetsChanged` (the `Listener` callback
  gametextinput calls once insets are wired — §7.3 item 3 made that wiring work) is currently an
  empty stub. A focused field low on screen can end up behind the keyboard with nothing panning
  it into view.
- **No selection UI, no copy/paste toolbar.** Tap-to-position-cursor itself shipped in §7.7; a
  real range selection (drag handles) and a copy/paste toolbar are still open — the
  native-`EditText`-overlay approach sketched in §7.7's own known-gaps note is the planned path.
- **Every field looks identical to the IME regardless of `zkite_py` props.** `onCreateInputConnection`
  hardcodes `TYPE_CLASS_TEXT` with no flags — a `TextField(secure=True)` (password) or
  `type="email"` renders and edits exactly like plain text natively, unlike the WebView backend,
  which gets real behavior for these for free from the `<input type=...>` attribute.
- **No Autofill, no accessibility sync** — phase 0's own note that these two "reduce to the same
  missing piece" (a virtual semantics tree) still applies; AccessKit (§5 item 2) is the natural
  place to build it once for both.

### 7.6 A real rendering bug found during this work — not an IME bug itself, but found because of it

**Status: SOLVED — see §7.14 for the root cause, the fix, and the live verification.**
The investigation below is kept in full because every negative result in it is still
valid and is what eventually pointed at the answer; read it as the trail, and §7.14 as
the conclusion. In short: nothing here was ever a rendering fault. The surface was
being repositioned by the status-bar height after an IME show/hide cycle, so the
affected node was painted correctly but underneath the status bar.

Once a `TextField` exists anywhere in a tree and can be focused, an entirely unrelated text
node — the demo app's own counter digit, nowhere near the field in the tree or on screen —
silently stops rendering. Confirmed with a plain focus-then-blur, no typing involved at all: the
"0" above the Increment button is present on a fresh launch, vanishes the instant a tap focuses
the (empty) `TextField`, and stays gone. Not cosmetic, not a hitch — persists across repeated
screenshots seconds apart.

**What's been ruled out, with real evidence, not guesses:**

- **Not a layout/data bug.** Diagnostic logging in `redraw()` (temporarily) printed every
  positioned node's rect and text content on every redraw. The counter's rect
  (`(521.0, 32.0, 38.0, 70.0)`), color (`Some(Color { r: 255, g: 255, b: 255, a: 255 })`), clip
  (`None`), and glyph count/position (1 run, 1 glyph, at `(0.0, 55.0)` relative) were **byte-for-
  byte identical** across every redraw checked — fresh boot, focused, and after the viewport
  resize that comes with the keyboard showing. Every value the paint code reads for this node is
  provably correct going in.
- **Not GPU-specific.** Temporarily forcing `try_init_gpu` to always fail (routing every frame
  through `paint_cpu_fallback`, `vello_cpu`'s independent software rasterizer, not
  `vello_hybrid`) reproduced the identical symptom. Two separate rendering pipelines failing
  identically rules out a bug specific to either one's GPU-side internals.
- **Not the caret paint.** Temporarily disabling the caret's own `fill_background` call entirely
  (added by this session, the most obvious new suspect) made no difference.
- **Not an empty-glyph-run `set_paint`-with-nothing-to-draw issue.** The empty (focused, nothing
  typed yet) `TextField`'s own synthesized text child *does* produce `Some(vec![])` — present,
  zero runs — and originally still reached `scene.set_paint`/`ctx.set_paint` before an empty `for
  run in runs` loop. Skipping such nodes entirely (now in both `paint_nodes_into_scene` and
  `paint_cpu_fallback` — a reasonable simplification kept for its own sake) made no difference to
  this bug either.
- **Not reproducible in a host-side `zkite_renderer` test**, including one deliberately shaped to
  match the real sequence as closely as possible — the same long-lived `Renderer` across multiple
  `clear()`+build+`layout()` cycles, unfocused twice then focused once, checking whether
  `font_cx`/`layout_cx` (state `clear()` deliberately leaves warm across renders) accumulate a
  problem from shaping an empty string. `phase_3/renderer/src/lib.rs`'s
  `empty_text_leaf_does_not_corrupt_a_sibling_after_repeated_layouts` passes cleanly. Whatever
  this is, either it needs the real demo app's full tree complexity (60 rows, Switch, Slider, and
  a `TextField`, not two bare text leaves) to trigger, or it's downstream of `zkite_renderer`
  entirely — in the GPU/CPU paint step, or in an Android-level interaction (compositor, surface
  buffer geometry) this session didn't reach.

**Best working hypothesis, not confirmed:** something in the paint step's interaction with the
*surface*/*window* state specifically during or after an IME-triggered resize — `vello_hybrid` is
explicitly beta-quality (plan1.md §5/§9 already flags "known WebGL/Adreno issues under active
fix") and `vello_cpu`'s own `ANativeWindow_lock` blit path has its own untested-until-now code
paths — rather than a logic error in this session's own tree-building code, given every value
that code hands to paint is independently confirmed correct. Left explicitly unresolved rather
than guessed at further; a future session should reach for real GPU-frame-capture tooling or a
minimal from-scratch repro app (stripped of `zkite_py`/`zkite_pybridge` entirely) rather than more
`adb logcat` archaeology, which is where this session's own investigation stopped paying off.

**Update, a later session: the on-device minimal-repro route (rather than GPU-frame-capture
tooling, which wasn't available in this environment) narrowed the trigger from "the full demo app"
down to a 3-node tree — still no root cause, but a much smaller haystack for whoever picks this up
next. See §7.9 for the full bisection and what it does and doesn't rule out.**

**Update, a later session still: no physical device was available this session (`adb devices`
came back empty throughout), so this session couldn't extend §7.9's own on-device bisection
further. Instead it asked a cheaper question first — does §7.9's exact 3-node minimal repro,
pushed through the *real* `vello_cpu` paint pipeline rather than just `zkite_renderer`'s layout
metadata, reproduce purely on the host? A new crate, `phase_3/repro_7_6/`, builds the identical
3-node tree (`Text` + empty `TextField` + sibling `Text`) via `zkite_pybridge::build` (host-testable
on its own — no CPython embedding needed, see §8.1) and paints it through a line-for-line copy of
`gpu_harness::render_thread::paint_cpu_fallback`'s own glyph/clip/background call sequence, minus
only the `ANativeWindow` blit tail that has no host equivalent. Run across 33 total redraws —
far more than the "appeared on the third redraw" the original device run needed, and a real
unfocused→unfocused→…→FOCUSED→FOCUSED→…→blurred-again sequence, not one bare pass — and checked
by direct pixel inspection of the "0" node's own rect, not glyph-count metadata. See that crate's
own module doc for the full account.**

**Result: it did not reproduce, on any of the 33 passes.** This is a real, if negative, finding —
the previous host-only attempt (`zkite_renderer`'s own
`empty_text_leaf_does_not_corrupt_a_sibling_after_repeated_layouts` test) only ever checked layout
metadata across repeated `clear()`+build+`layout()` cycles, since `zkite_renderer` deliberately has
no dependency on either vello flavor at all (its own module doc) — it never actually rasterized
anything. `repro_7_6` is the first host-only attempt that does, using the identical call sequence
the real device's own CPU-fallback path uses, and it still comes back clean on every pass.

**What this newly rules out, with real evidence:** `zkite_renderer`'s layout/shaping,
`zkite_pybridge`'s tree-building/focus-state synthesis, and `vello_cpu`'s own rasterization —
including any warm-state/glyph-cache accumulation across many repeated redraws in one process, and
any per-frame corruption from the specific focus/blur sequence a real session goes through — are
now excluded as the root cause, at least for a short-lived host process. Combined with what §7.6's
original investigation already ruled out (GPU-vs-CPU-specificity, the caret paint, an
empty-glyph-run edge case), the search space has narrowed materially: what's left is either the
`ANativeWindow` buffer-geometry/lock/blit tail (genuinely un-host-testable — there is no
`ANativeWindow` on a Linux host, and `repro_7_6` deliberately doesn't try to fake one), the
`vello_hybrid`/`wgpu` GPU path specifically (no GPU context available in this environment either),
or something tied to a *real* surface resize (`onSurfaceChanged`/`Resize`/`reconfigure` in
`render_thread.rs`) triggered by the keyboard actually showing/hiding — `repro_7_6` never varies
its fixed canvas size or calls anything resize-shaped, unlike a real focus transition on a device.
§7.6's own "best working hypothesis" (the paint step's interaction with surface/window state
during or after an IME-triggered resize) is, if anything, strengthened rather than contradicted by
this result: the more of the shared, portable pipeline gets cleared, the more the remaining
suspects concentrate on the Android-only tail this crate structurally cannot reach.

**Recommendation for whoever picks this up next, updated accordingly:** tree bisection (§7.9's own
route) and host-only pipeline reproduction (this update's route) have both now been tried and both
say the same thing — the bug isn't in the tree shape or the shared rasterization code. The next
productive step needs either a physical device (this session had none) with real GPU-frame-capture
tooling aimed specifically at a focus-triggered resize, or a minimal *Android* repro app (real
`ANativeWindow`, no `zkite_py`/`zkite_pybridge`) that exercises `Resize`/`reconfigure` on a fixed
3-node scene without any real IME involved at all — isolating "does a bare resize event alone,
with no keyboard/IME in the loop, corrupt this specific paint region" as its own, narrower
question. Plain `adb logcat` archaeology already didn't pay off in the original session; more tree
bisection now has direct evidence against it paying off either.

### 7.7 Tap-to-position-cursor

**Motivation.** §7.4 shipped live typing but always appended — no way to tap into the middle of
existing text and have the next keystroke land there. §7.5 flagged this as a gap. The user's own
framing going in: *is there something to wire up here rather than hand-rolling it* — the answer for
this half (a single tap repositioning a cursor) is yes, `GameTextInput_setState` plus parley's own
`editing::Cursor` for hit-testing; real range selection with drag handles and a copy/paste toolbar
is the other half, deliberately deferred to a native-`EditText`-overlay approach (see this
section's own known-gaps note) rather than hand-building Flutter-style selection UI from scratch.

**Design.** Three pieces, wired front-to-back:

- **Hit-testing (`zkite_renderer`, host-testable, no device needed).** `Renderer::shape_isolated`
  shapes a `TextStyle`+`content` pair through parley exactly as the real paint path does, without
  touching `Renderer`'s own persistent tree state. `hit_test_text_offset(style, content, x, y)`
  feeds that layout to `parley::Cursor::from_point(&layout, x, y).index()` — parley's own
  purpose-built text-editor hit-testing, not hand-rolled glyph-x math. `text_caret_geometry(style,
  content, byte_offset, caret_width)` is the inverse: `Cursor::from_byte_index` +
  `.geometry(&layout, caret_width)` for the real on-screen caret rect at a given byte offset. Both
  are covered by a host-side test (`tap_to_offset_and_caret_geometry_agree_with_text_order`) that
  taps near the start and far past the end of `"hello world"` and checks the returned offsets and
  caret x-positions are ordered the way the text is — this class of geometry bug is exactly what a
  fast host test can catch before ever touching a device.
- **Render-thread wiring (`gpu_harness/src/render_thread.rs`).** `dispatch_tap`'s existing
  text-field branch now calls `position_cursor_from_tap(x, y)` right after `focus_text_field`:
  finds the focused text node's positioned rect, converts the tap to node-local coordinates, calls
  `hit_test_text_offset`, and hands the resulting byte offset to `set_cursor_position`, which
  updates `focused_selection` locally, pushes it to Java via a new JNI call, and redraws. Caret
  painting (`caret_rect`) was rewritten to read `focused_selection` and call
  `text_caret_geometry` for a real glyph-accurate position, replacing the always-end-of-text
  approximation §7.4 shipped with (that paragraph is corrected in place above, not left stale).
- **JNI path down to the real `Editable` (`text_input.rs`, `lib.rs`, `Native.java`,
  `ZKiteSurfaceView.java`).** A new `TextInput::set_state` wraps the vendored
  `GameTextInput_setState`, reached from the render thread via `Native.setSelection(text, start,
  end)` — a new JNI entry that, like `focusAndShowKeyboard`/`blurAndHideKeyboard` before it, must
  run on the UI thread, so `ZKiteSurfaceView.setSelection` does the same `post()`-if-needed hop
  those two already established. `setState`'s own contract (read from the vendored class's
  bytecode via `javap`, no source ships) requires the *entire current text* on every call, not
  just the new selection — it does `mEditable.clear()` then reinserts `state.text` from scratch,
  so `render_thread.rs` always passes its own `focused_text`, never just a selection delta.

**Real finding: a `javap`-only read of `setState` gave an incomplete answer on whether it
notifies `Listener`.** Documented earlier (in `text_input.rs`, now corrected in place) as *not*
calling `stateUpdated`. Live logcat during this section's own verification shows otherwise — every
`setState` call is immediately followed by `stateChanged`/`onTextInputEvent` carrying the same
text:

```
D gti.InputConnection: setState: 'zkite', selection=(0,0), composing region=(-1,-1)
D gti.InputConnection: getEditable
D gti.InputConnection: finishComposingText
D gti.InputConnection: setComposingRegion: -1:-1
I ZKiteGpuHarness: stateChanged: text=zkite dismissed=false
I ZKiteGpuHarness: onTextInputEvent: decoded text="zkite"
```

`setState` finishes by internally calling `finishComposingText`/`setComposingRegion(-1,-1)` to drop
any active composing region (reasonable: repositioning the cursor should end whatever the IME was
mid-composing, the same way a real `EditText`'s own touch handling does) — and *that* is what fires
the notify. Easy to miss reading the bytecode for `setState` alone in isolation, obvious on a real
device. Net effect is benign either way — `render_thread.rs`'s own optimistic local update and the
event that (it turns out) does arrive both land on the exact selection `setState` was asked for —
but the lesson generalizes: this vendored library ships no source, and a static bytecode read of
one method in isolation can miss effects that only show up by watching the real object graph run.

**Real finding: a tap-coordinate math script is only valid for the layout it was computed
against, and focusing a field changes the layout.** §7.6 already documents an unrelated node
(the demo app's counter) vanishing when a `TextField` gains focus. That bug has a second-order
consequence specific to *testing* tap-to-cursor: the focused field's own on-screen `y` shifts up
once the sibling above it disappears, so tap coordinates computed from a pre-focus screenshot
land in the wrong place post-focus (in one verification pass, a stale coordinate landed on a list
row instead of the field). The fix is procedural, not code: always re-screenshot *after* focusing,
and read real coordinates off that frame, never off an earlier one — the same discipline §7.4's
own "`adb shell input text` doesn't exercise the real path" finding argues for, applied to
coordinates instead of input method.

**Real finding: Gboard's decoder can silently substitute a different letter than the one
geometrically tapped.** A tap aimed at the key geometrically under `z` was decoded and committed
as `s` (confirmed via `gti.InputConnection: Commit: s, new pos = 1`) — Gboard's key decoder isn't
purely geometric nearest-key matching, it leans on a language-model prior that can override a
well-aimed tap for a statistically rare letter. Harmless for this section's own verification (the
test only cares that *whatever* character was committed landed at the tapped cursor position, not
which character it was) but worth knowing before trusting exact-character assertions in any future
coordinate-tap-based test against a real predictive keyboard.

**Verification.** Live, on the same Moto G54 5G real device every prior IME section used:

- Typed `"zkite"` via real taps on Gboard's own rendered keys → field showed `zkite`, cursor at the
  end (append path from §7.4, unchanged and still correct).
- Tapped near the start of the rendered text → `gti.InputConnection: setState: 'zkite',
  selection=(0,0), ...` (offset correctly hit-tested to the very start) and the caret visibly
  rendered before the `k`. Typed one more character → committed as `s`, field became `szkite`
  (inserted at the tapped position, not appended at the end) — confirmed both via
  `onTextInputEvent: decoded text="szkite"` and a screenshot showing `szkite` with the caret after
  the `s`.
- Tapped a genuine interior position (between `k` and `i` in `szkite`) → `setState: 'szkite',
  selection=(2,2)` — a real interior offset, not an edge case a degenerate hit-test could fake.
  Typed one more character → committed as `o`, field became `skoite` (`sk` + `o` + `ite`,
  confirmed via `onTextInputEvent: decoded text="skoite"` and a screenshot showing the caret
  between `o` and `i`) — proof the general hit-testing math is right, not just its boundary cases.
- No crashes across the whole sequence — `pidof` stayed constant throughout (including across an
  unrelated foreground-app interruption mid-session, unconnected to this code, that briefly
  backgrounded the app; its state was untouched when it returned to the foreground).

**Known gaps.**

- **Cursor only, not a real range selection.** `set_cursor_position` always calls
  `set_state` with `start == end`. Nothing here builds a drag-to-select gesture or renders a
  selection highlight — `focused_selection`'s two fields are already independent, decoded straight
  from `GameTextInputState`, so the plumbing accepts a real range today; only the gesture that
  would produce one (and the highlight paint for it) is missing.
- **No selection handles, no copy/paste toolbar.** The user's own take, taken as this project's
  plan: a native `android.widget.EditText` overlay (positioned over the focused field's screen
  rect, translating its Tailwind-derived style so it visually matches, shown only while focused) is
  the pragmatic route to real selection handles and a real copy/paste toolbar for free from the
  platform, rather than hand-building Flutter-style selection UI — reserve the from-scratch route
  for if visual consistency during editing turns out to matter more than it's expected to.
- **Long-press-to-select and drag-handles were absent as of this section** — shipped in §7.8
  below, via the overlay this note already points at, not by extending the hit-testing
  primitives here (`hit_test_text_offset` already gave that work what it needed). Double-tap-
  to-select-word specifically is still absent — §7.8's own known-gaps note.

### 7.8 Selection handles and a copy/paste toolbar — a native `EditText` overlay

**Motivation.** §7.7 shipped a real cursor but no way to select a *range* — no handles, no
Cut/Copy/Paste. Hand-building selection UI (draggable handles, a floating toolbar, real
system-clipboard access) from scratch is a lot of platform behavior to reproduce correctly; the
user's own framing going in (this session's actual instruction): wire up tap-to-cursor first
since "it's cheap and much of the value," then for selection specifically, reach for "the
native-EditText-overlay... rather than hand-building Flutter-style selection UI," reserving a
from-scratch build for if visual consistency during editing "turns out to matter a lot" — it
didn't have to, once verification below showed the pragmatic version working end to end.

**Design.** A long-press promotes a field from the lightweight, already-proven `GameTextInput`
path (§7.7, unchanged — ordinary tap-to-type never touches any of this) to a real, fully visible
`android.widget.EditText` laid on top of it for the rest of that focus session:

- **Long-press detection (`render_thread.rs`, `Command::TouchUp`).** No new timer/wakeup
  machinery — `Gesture` gained its own `down_time` field (deliberately *not* reusing
  `history[0].0`, which `TouchMove` trims to a 100ms window for velocity estimation and would
  age out the true down time under perfectly ordinary finger jitter; see that field's own doc
  comment), and `TouchUp` checks `down_time.elapsed() >= LONG_PRESS_THRESHOLD` (500ms, Android's
  own documented default) for a gesture that never dragged. Fires at *release*, not mid-press
  with haptic feedback the way a real Android long-press does — this thread has no per-view
  timer callback, and release-detection is simpler and sufficient for what this needed (an
  initial selection range), at the cost of that one bit of native feel.
- **Word-boundary hit-testing (`dispatch_long_press`, `word_range_at`).** Reuses §7.7's
  `hit_test_text_offset` to find the byte offset under the press, then `word_range_at` (ASCII
  alphanumeric/underscore run containing that offset — not full Unicode word segmentation, a
  known gap below) expands it to a word — the same default initial selection every native text
  view gives for free.
- **The overlay itself (`TextSelectionOverlay.java`, new).** A real `EditText`, added once as a
  sibling of `ZKiteSurfaceView` in their shared parent `FrameLayout` (`Activity.setContentView`
  already wraps a single content view in one — no `ZKiteActivity` change needed to get a second
  child into it), normally `GONE`. `show(...)` positions/sizes it via `FrameLayout.LayoutParams`
  margins over the field's rect, styles it (background/text color/size — see the next paragraph
  on why not more than that), seeds text + `[start, end)` selection, then calls `requestFocus()`
  + `showSoftInput` — from there, Android's own `Editor` owns handles, the toolbar, and the real
  `InputConnection` for as long as the overlay stays focused. A `TextWatcher` + overridden
  `onSelectionChanged` forward every live change to `Native.onSelectionOverlayChanged`; losing
  focus (or an explicit native-initiated `hide()`) reports final state via
  `Native.onSelectionOverlayDismissed` and hides. Both funnel through one guarded method
  (`reportChangeAndHide`) keyed on an explicit `visible` flag, not `editText.getVisibility()` —
  setting a focused view `GONE` can itself synchronously re-enter the same callback (Android
  clears focus from a view that becomes invisible), and the flag, set *before* that call, is what
  makes the re-entrant call a no-op instead of a duplicate dismiss report.
- **Fully visible, not a transparent mirror of the GPU-rendered text underneath.** Considered and
  rejected: keeping the overlay invisible (matched font/size, transparent text/background) so it
  would look like the GPU path was still what the user sees, with the overlay only supplying
  invisible hit-targets for handles/toolbar. Rejected because handle *positions* are driven by
  the `EditText`'s own internal `Layout`, a different text-shaping engine (Android's, not
  `parley`) — the two would only ever be approximately aligned, never exactly, so an invisible
  overlay would still show visibly-misplaced handles relative to the real glyphs, for no benefit
  over just admitting the switch. Shown fully opaque instead, sized to the field's exact
  container rect (see the real bug below on *which* rect), so no double-image is possible by
  construction — occlusion is exact, not approximate.
- **Container rect, not the text leaf's rect.** `show_selection_overlay` resolves
  `Renderer::parent(focused_text_node)` and uses *that* node's positioned rect/background for
  the overlay, not `focused_text_node`'s own — see the real bug below.

**Real bug, found by reading `zkite_pybridge` before writing any positioning code, not on a
device:** `BuiltTree::focused_text_node` is the synthesized *text leaf* inside a `TextField`'s
container (`zkite_pybridge::build_node`'s `is_text_field` branch) — tight around the shaped
glyphs, with no padding or background of its own; **all** of the field's visual box (background,
rounding, padding) lives on its parent container, built from the `TextField`'s own Tailwind
classes. `caret_rect`/`position_cursor_from_tap` (§7.7) only ever needed the text leaf's rect,
for glyph-local math — using it for the *overlay's* size/position too (an easy copy-paste
reach) would have shown a small overlay hugging just the current glyphs, floating inside the
real field's dark rounded box instead of covering it, and — since the leaf currently carries no
background of its own — the field's real background would very likely show through around the
edges (or not, depending on paint order, but wrong either way). Caught by re-reading
`build_node` rather than assuming a text node and its "obvious" container are interchangeable;
the fix is one extra `Renderer::parent` call.

**Real finding: showing a non-collapsed selection programmatically renders handles, but does
*not* auto-invoke the floating Cut/Copy/Paste toolbar — a second, genuine long-press (this time
landing on the real `EditText`, not the GPU surface) is what actually shows it.** Discovered on
device, not predicted: the first long-press (`dispatch_long_press` -> `show_selection_overlay` ->
`EditText.setSelection(start, end)`) reliably showed the word highlighted *and* a draggable
cursor handle after a plain tap collapsed the selection — but no toolbar appeared next to it.
Android's `Editor` ties handle visibility to selection state directly (non-collapsed + focused +
cursor-visible is sufficient), but appears to gate the `ActionMode` toolbar on an actual
touch-driven selection gesture, not merely `setSelection` called from Java. A real long-press
performed directly on the now-visible, now-focused overlay (`adb shell input swipe` with
identical start/end coordinates and a 700ms hold — no drag, well past `LONG_PRESS_THRESHOLD`)
reliably brought up the full native toolbar (`Translate | Cut | Copy | Share`, and separately
`Paste | Select all` for a collapsed cursor with clipboard content) every time. Left as-is rather
than chased further (see known gaps) — no public `TextView` API was found for invoking the
`ActionMode` directly without a real touch event standing in for one.

**Verification.** Live, on the same Moto G54 5G real device every prior IME/§7.7 section used,
one continuous sequence, `pidof` unchanged throughout (no crash):

- Typed `zkite` via real Gboard taps (§7.7's already-verified path). Long-pressed inside it
  (`adb shell input swipe` in place, 700ms) — logcat: `Touch (40, 332) -> long-press on text field
  r.3`, then the overlay's own `InputMethodManager: showSoftInput() view=...TextSelectionOverlay$1`
  — and the screenshot showed the *entire word* `zkite` highlighted blue (native selection), sized
  and positioned over the real field, not a mismatched box.
- A plain tap on the selection collapsed it to a cursor and showed a real draggable cursor
  handle (a small blue teardrop) — confirmed visually.
- A second long-press, this time on the overlay itself, brought up the full native
  `Translate | Cut | Copy | Share` toolbar (see the real finding above). Tapped **Copy**: the
  toolbar dismissed, selection collapsed to a cursor, and Android's own clipboard-access
  indicator (the system chip that appears whenever *any* app reads/writes the clipboard, a
  privacy feature independent of anything this session wrote) appeared at the bottom of the
  screen — independent, OS-level proof the real system clipboard was actually written, not just
  that a menu item was tapped.
- Tapped elsewhere (an `Item` row, outside the overlay's rect): the overlay hid, the keyboard
  dismissed, and the field went back to plain GPU-rendered `zkite` with no cursor, no selection
  box, no double-image, no leftover artifact of any kind.
- A fresh long-press (word re-selected, overlay shown again), a tap to collapse to a cursor, a
  second long-press at that same collapsed-cursor point brought up `Paste | Select all` (Android
  only offers *Paste* for a collapsed cursor with clipboard content, correctly distinct from the
  *Cut/Copy* set a real selection gets) — confirmed visually. Tapped **Paste**: the field became
  `zkitezkite` (the earlier copy, inserted exactly at the cursor) — confirmed both via screenshot
  and the keyboard's own suggestion strip updating to show `zkitezkite`. Tapped elsewhere once
  more: clean dismiss, `zkitezkite` persisted correctly through to the plain unfocused GPU-rendered
  display — proof the full copy round trip (`zkite` selected -> Copy -> real clipboard -> Paste ->
  `Command::SelectionOverlayDismissed` -> `on_change` dispatch -> `zkite_py` state -> next render)
  works end to end, not just that a toolbar button visually does something.

**A real correctness bug, found by tracing the event ordering before it ever shipped, not by
hitting it on the device:** tapping a *different* field while one field's overlay is still open
would, without a fix, corrupt the newly-focused field's just-seeded `focused_text`/
`focused_selection`. Sequence: `focus_text_field(B)` runs synchronously first (a plain tap on the
GPU surface, handled immediately) and sets `focused_text`/`focused_selection` to B's own values;
*only then*, asynchronously, does stealing Android focus away from the still-open overlay for A
fire its `OnFocusChangeListener`, eventually delivering `Command::SelectionOverlayDismissed` with
A's final text — which, applied blindly, would stomp B's fresh state with A's stale leftovers.
Fixed with a dedicated `selection_overlay_field: Option<Arc<str>>` (distinct from `focused`,
which has already moved on by the time this arrives) recording which field the overlay was
actually showing for; the dismiss handler only touches `focused_text`/`focused_selection` if
`focused` still equals that remembered id, while *always* still dispatching the field's `change`
handler so A's edit reaches `zkite_py` either way. Not exercised on a real two-field race on
device (the demo app has one `TextField` — see known gaps); the fix is verified by tracing the
sequence against the real code, the same standard every JNI thread-safety fix earlier in §7
already had to meet before a test could even be written for it.

**Known gaps.**

- **Two long-presses, not one, to reach the toolbar** — see the real finding above. The first
  long-press (on the GPU surface) selects a word and shows the overlay; a *second* long-press (now
  on the real `EditText`) is what a user would actually need to reach Cut/Copy/Share. Functional,
  but not the single seamless gesture a native text field gives for free. Worth another pass if a
  clean way to invoke `ActionMode` from code turns up — none was found this session.
- **The cross-field race fix is verified by tracing, not by an on-device two-field test** — see
  above. The demo app has exactly one `TextField`; a future session adding a second is the natural
  place to actually exercise this path live rather than continue to trust the trace.
- **ASCII-only word boundaries** (`word_range_at`) — a long-press inside CJK or other
  non-alphanumeric-script text falls back to a zero-width selection at the tap point instead of a
  real word.
- **No double-tap-to-select-word** — only long-press initiates a selection; `hit_test_text_offset`
  already provides what a double-tap handler would need, same "primitive exists, gesture doesn't"
  shape as §7.7's own note on this.
- **Overlay styling is an approximation, deliberately** (see this section's own design note) —
  background/text color/size are matched; corner radius, exact padding, and font family are not.
  Acceptable per the user's own stated risk tolerance going in; revisit only if it turns out to
  matter more in practice than expected.

### 7.9 Insets/resize compensation — panning the focused field above the keyboard

**Motivation.** §7.5's own known-gaps list: a focused field low on screen could end up entirely
behind the keyboard with nothing panning it into view. `ZKiteSurfaceView.onImeInsetsChanged` (the
`Listener` callback `GameTextInput` already calls once real system insets are wired — §7.3 item 3
made that wiring work) was an empty stub.

**Design.** Reuses `zkite_renderer`'s existing scroll-offset mechanism rather than inventing a
separate transform. `Renderer::positioned`'s tree walk shifts *any* node's descendants by
whatever `(x, y)` a `scroll_offsets: &FxHashMap<NodeId, (f32, f32)>` entry gives it — nothing in
that code requires the node to actually be an `overflow-scroll` container, that's just the only
caller that had used it before. `render_thread.rs`'s `redraw()` now computes a pan amount fresh
every frame (`compute_ime_pan`, from *this frame's* unpanned layout — never accumulated) and, if
non-zero, inserts one more entry into the offsets map keyed on the tree's own root — every
descendant shifts up by exactly that much, with no clipping (root's own `overflow` is `visible`),
and `node_scroll_offsets` (which every touch handler already calls) picks up the same entry, so
hit-testing and painting agree automatically. `compute_ime_pan` reads the *container* node's rect
(the same one `show_selection_overlay`, §7.8, already uses — not the bare text leaf, which carries
no padding/background of its own), and clamps two ways: never more than the IME's own reported
inset (panning further would reveal space below the tree the keyboard isn't actually covering),
and never more than the field's own unpanned distance from the top (a keyboard taller than the
room above the field pans it flush to the top rather than pushing content off-screen chasing an
unreachable target).

**Plumbing.** `ZKiteSurfaceView.onImeInsetsChanged` → `Native.onImeInsetsChanged(int bottomPx)` →
`Command::ImeInsetsChanged { bottom: f32 }` → `State::ime_inset_bottom`, read fresh by
`compute_ime_pan` every `redraw()`.

**Verification, live on the same Moto G54 5G real device every IME section has used.** Focused a
field sitting at the very bottom of the demo app's screen (below the 60-row `ScrollView`, deliberately
placed there for exactly this test): logcat showed `inset_bottom=925`, and the *unpanned* container
rect at that moment was `(16.0, 2353.0, 1048.0, 35.0)` in a `2400`-tall surface — `field_bottom
(2388) - visible_bottom (2400 - 925 = 1475) = 913`, and the very next redraw's rect was `(16.0,
1440.0, 1048.0, 35.0)` — `2353 - 1440 = 913`, exactly matching the formula by hand, not
approximately. Screenshot confirmed the field fully visible immediately above the keyboard, not
clipped or partially covered. Blurring (a tap elsewhere, or the field losing focus) dropped the pan
back to `0` on the very next redraw — confirmed both via logcat (`pan=0`) and a screenshot showing
the list back at its pre-focus scroll position with nothing left panned.

**A real, incidental §7.6 repro, found for free during this verification.** Blurring the field
after this test left the demo app's counter digit permanently gone — the exact §7.6 symptom,
reproduced completely independently of this session's own insets work (the pan mechanism itself
was already back to `0` and behaving correctly when it was noticed). This is what prompted the
minimal-repro bisection recorded in §7.6's own update and expanded on directly below.

**Minimal-repro bisection for §7.6 (this session's actual contribution to that investigation).**
Using `ZKITE_APP_MAIN` to swap in progressively smaller apps (never touching the real
`demo_app/main.py`), each rebuilt and tested live on the same device:

1. `Column(Text("0"), TextField(...))` — two nodes, nothing else. **Did not reproduce.** Focusing
   the field left "0" fully visible.
2. The same two nodes plus a 60-row `ScrollView` (no `Switch`/`Slider`/`Button`). **Reproduced** —
   "0" gone after focus, confirmed by dismissing the keyboard and scrolling back to the top of the
   list to rule out "just scrolled off-screen."
3. The same 60 rows, but in a plain non-scrolling `Column` — no clip, no scroll-offset machinery at
   all. **Still reproduced** — rules out `ScrollView`'s clipping/scroll-offset code specifically.
4. The same shape with only 5 rows instead of 60. **Still reproduced** — rules out "needs real
   scale" as the necessary ingredient.
5. The same shape with exactly **one** extra sibling `Row(Text("Item 0"))` — three text-bearing
   nodes total (the "0", the `TextField`'s own synthesized empty text child, and one more).
   **Still reproduced.**

**Net result: the minimal known trigger is now a 3-node tree** — `Text` + `TextField` + one more
sibling `Text` — down from "the full demo app" (60 rows, `Switch`, `Slider`, `Button`, all
together). `ScrollView` and scale are both ruled out as necessary; the remaining open question is
what specifically differs between "2 text-bearing nodes" (safe) and "3" (bug) — an off-by-one in
some fixed-size structure is one plausible shape for that, a glyph-atlas or paint-command-buffer
interaction is another, neither confirmed. Root cause still not found; this is real, incremental
progress narrowing the search space, not a fix — a future session picking this up should start
from the 3-node repro above (cheap to rebuild via `ZKITE_APP_MAIN`) rather than the full demo app,
and reach for real GPU-frame-capture tooling if bisecting the tree itself stops yielding new
information, per this section's own standing recommendation.

**A real testing-workflow gotcha found while iterating on `ZKITE_APP_MAIN` variants, worth
recording:** `ZKiteActivity`'s asset unpacking is stamp-file-guarded (`STAMP_VERSION`), and `adb
install -r` does **not** clear app data — reinstalling over a previous build with a *different*
`app.zip` (e.g. a different `ZKITE_APP_MAIN`) silently keeps running the **old** Python app, because
the stamp file from the previous install already exists on-device. `adb shell pm clear
<pkg>` before each install is what actually forces a fresh unpack; several early re-tests in this
session were confusingly showing stale UI before this was caught.

### 7.10 Real per-field input types, and password masking on both render paths

**Motivation.** §7.5's own known-gaps list: every `TextField` looked identical to the IME
regardless of `zkite_py`'s own `type`/`secure` props — `onCreateInputConnection` hardcoded
`TYPE_CLASS_TEXT` unconditionally.

**Mapping.** `ZKiteSurfaceView.androidInputTypeFor(String spec)` maps `zkite_py`'s `input_type`
attr (`TextField._tree_attrs` — `"password"` when `secure=True`, else `type=`, defaulting to
`"text"`) to real `InputType` flag combinations: `password` → `TYPE_TEXT_VARIATION_PASSWORD`,
`email` → `TYPE_TEXT_VARIATION_EMAIL_ADDRESS`, `url` → `TYPE_TEXT_VARIATION_URI`, `tel`/`phone` →
`TYPE_CLASS_PHONE`, `number` → `TYPE_CLASS_NUMBER`, anything else (including plain `text`) falls
back to plain `TYPE_CLASS_TEXT` — the same behavior every field already had, not a failure mode.
`render_thread.rs`'s `current_text_field_input_type` (mirrors `current_text_field_value`'s own
recursive-search shape) reads the attr off `self.current` at focus time and passes it down.

**Switching type on an already-focused view.** Android only calls `onCreateInputConnection` again
on its own initiative (first focus, or a view regaining focus after losing it) — merely changing
which field this crate considers "focused" doesn't trigger it, so a password field focused right
after a plain-text one would silently keep the *first* field's `EditorInfo` for the rest of the
session without an explicit nudge. `focusAndShowKeyboard(String inputTypeSpec)` now stores the
spec, and — only when it differs from whatever `onCreateInputConnection` last actually built
(`currentInputTypeSpec`) — calls `InputMethodManager.restartInput(this)`, which forces a fresh
input session (and therefore a fresh `onCreateInputConnection` call) on the already-focused view,
the same technique other custom-rendered text stacks (e.g. Flutter's) use for this. Confirmed live:
switching focus from a `type=text` field to a `type=password` one produced a **new**
`onCreateInputConnection` log line with a different `InputConnection` object identity
(`@c216f1a` → `@280ee3d`), and Gboard's own logs (`KeyboardManager.requestKeyboard(): ...
imeId=password ... className=.latin.keyboard.LatinPasswordKeyboard`) confirmed the real keyboard
app itself switched its rendered layout in response — this isn't just our own `EditorInfo` field
changing, the system keyboard genuinely reacted to it.

**Password masking — a real gap found while verifying, not part of the original plan for this
item.** Neither render path masked anything: the primary GPU-painted field (`zkite_pybridge::
build_node`) painted `RawNode.attrs["value"]`/the live IME text verbatim regardless of
`input_type`, and the selection overlay's real `EditText` (§7.8) had no `inputType` of its own
either — a `secure=True` field would have shown its real password in the clear on *both* paths.
Fixed two different ways, deliberately not the same way, because the two paths need different
things from masking:

- **The primary path** (`zkite_pybridge::mask_password`): replaces the shown text with one `*` per
  **byte** of the real text, not one per character. Deliberately not character-accurate — every
  byte offset this renderer computes against a field's text (`GameTextInputState`'s own selection,
  `hit_test_text_offset`, `text_caret_geometry`) is a byte offset into whatever string was actually
  shaped, and `*` is one UTF-8 byte while the "obvious" mask character (`•`, U+2022) is three;
  masking per character would leave every downstream byte offset wrong the instant a password
  contains a multi-byte character. `render_thread.rs`'s `focused_display_text()` applies the exact
  same transform before every geometry call (`caret_rect`, `position_cursor_from_tap`,
  `dispatch_long_press`'s hit-test) so caret/tap math always shapes what's actually painted, never
  the real characters — a real, would-have-shipped bug if geometry had kept using the real text
  while paint used the masked one. `word_range_at` degrades to a zero-width selection on masked
  content (`*` isn't an alphanumeric "word" byte per Unicode) — accepted, not fixed, since there's
  no meaningful "word" inside a run of mask characters anyway, and the resulting selection is still
  expandable via the overlay's own drag handles once shown.
- **The selection overlay** (`TextSelectionOverlay.show`, a new `isPassword` parameter threaded
  through `Native.showSelectionOverlay`/`ZKiteSurfaceView.showSelectionOverlay`, JNI descriptor
  grew from `(Ljava/lang/String;IIFFFFFII)V` to `...IIZ)V`): gets a real
  `TYPE_CLASS_TEXT | TYPE_TEXT_VARIATION_PASSWORD` on its own `inputType`, reset on *every* call
  (not just when `true`) since the one `EditText` is reused across every field's focus session and
  must not inherit a previous field's type. Deliberately **not** fed pre-masked text — it still
  gets the real value, so Android's own `PasswordTransformationMethod` masks it visually and its
  stock `Editor` correctly hides Cut/Copy for a real selection (both for free), and a real
  "Select All → Copy → Paste elsewhere" round trip moves the *actual* password, not literal
  asterisks.

**Verification, live, same device.** Typed two real characters into a `secure=True` field via
real Gboard taps: `gti.InputConnection: Commit: y/8` (real committed text `"y8"`) but the
on-screen field showed `**`, not `y8` — masking confirmed on the primary path. A long-press then
opened the selection overlay, which showed `..` (Android's own transformation-method rendering,
not `y8` and not `**` either) — masking confirmed independently on the overlay path. A subsequent
"Select All → Copy", tapping elsewhere to dismiss, then pasting into a different field, produced
the real `y8` content (not masked characters) at the paste destination — confirming the overlay's
"real text in, masked on screen" design round-trips correctly end to end, including through the
real system clipboard.

### 7.11 Selection polish: Unicode word boundaries, double-tap-to-select, and one more confirmed negative on the toolbar gap

**Unicode word boundaries.** `word_range_at`'s original ASCII-alphanumeric-run approximation
(§7.8's own known-gaps note) is replaced with real UAX #29 word segmentation via the
`unicode-segmentation` crate's `split_word_bound_indices`, filtered by
`segment.chars().any(char::is_alphanumeric)` — checked directly against that crate's own
`unicode_words()` source to confirm it's the identical predicate, since the crate exposes the
*filter* but not a byte-offset-preserving version of it. Verified by hand against both boundary
directions (tapping right after a word vs. right before the next one) before ever touching a
device, then confirmed live: long-pressing inside a real-Gboard-typed "cat" selected the whole
word, not a partial run.

**Double-tap-to-select-word.** `hit_test_text_offset`/`word_range_at`/`show_selection_overlay`
already existed (§7.7/§7.8) with nothing but a long-press wired to them — this closes that gap.
Real platform constants, piped down at `boot()` the same way `touch_slop` already is:
`ViewConfiguration.getScaledDoubleTapSlop()`/`.getDoubleTapTimeout()`, read once in
`ZKiteActivity` and carried through `Native.boot`/`AppPaths` to `render_thread::State`.
`is_double_tap` pairs a tap with `self.last_tap` (the previous tap's time/field/position) only if
same field, within the time window, and within the slop distance — cleared on any tap that isn't a
`TextField`, on a long-press, and after firing (so a third rapid tap starts a fresh pair rather
than chaining). Verified live: two real Gboard-driven taps close together on a real-typed "cat"
selected the entire word (confirmed via screenshot — full blue highlight), the same outcome as a
long-press, through the double-tap path specifically (`Touch (...) -> double-tap, selecting word`
in logcat).

**A confirmed negative, not a fix, on §7.8's "two long-presses" gap.** Tried
`editText.performLongClick()` right after seeding the overlay's selection, on the theory that it
might synthesize enough of a real long-click to invoke the floating Cut/Copy/Share toolbar without
a second, literal touch event. Checked live, not assumed: it does not — handles and the selection
itself already show from `setSelection` alone (as §7.8 already found), and `performLongClick()`
changed nothing about whether the toolbar appeared. Reverted (dead code doing nothing isn't worth
keeping) but recorded here since "confirmed doesn't work" is still real information for whoever
looks at this gap next — no working API for this was found this session either.

### 7.12 Cross-field selection-overlay race — verified live, not just traced

§7.8 shipped a fix for this race (`selection_overlay_field`, distinct from `focused`, so a
still-in-flight dismiss report for field A can't stomp field B's already-fresher state) but could
only verify it by tracing the code, since the demo app had exactly one `TextField` at the time.

**What it took to actually trigger this live.** The demo app now has two fields (§7.10's own
testing needed one anyway, for the type-switching verification). The first attempt stacked them
vertically, which turned out to make the race *untestable* by touch: panning the focused field
above the keyboard (§7.9) reveals only that field, and a vertically-stacked second field sitting
immediately below it ends up hidden behind the keyboard, unreachable by a real tap without first
dismissing the keyboard — which would defeat the exact "switch fields without going through blur"
sequence the race depends on. Fixed by laying the two fields out side by side in a `Row` instead —
both fields sit at the same panned `y`, so both stay simultaneously reachable once either is
focused. (This is also arguably a more natural demo shape than a vertical stack, not purely a
testing accommodation.)

**The actual sequence, live, real device.** Focused field A (typed "hi" via real Gboard taps),
long-pressed to open the selection overlay (word "hi" shown selected). While the overlay was still
up, tapped field B directly — landing on the GPU surface next to the overlay, not on it — which
`Touch (...) -> focusing text field r.4.1` confirmed reached `dispatch_tap`/`focus_text_field`
normally. Typed one real character into B afterward: `gti.InputConnection: Commit: h, new pos = 1`
— a **fresh, first character**, not appended to anything stale, confirming B started from genuinely
empty state, not corrupted by A's leftover text. A screenshot of the final state showed field A's
"hi" completely undisturbed (plain text, no stale selection highlight) and field B showing `*`
(correctly masked, per §7.10 — B is the `secure=True` field) with a live caret — both fields
holding correct, independent state after the full sequence.

**One loose end, not a bug:** `Native.onSelectionOverlayDismissed`/`onSelectionOverlayChanged`
never produced a log line during this sequence. Checked directly: neither the JNI entry points in
`lib.rs` nor the `Command::SelectionOverlayDismissed`/`SelectionOverlayChanged` handlers in
`render_thread.rs` have ever had explicit logging on this path (unlike, say,
`Command::TextInputEvent`'s), so "no log line" was never good evidence of "didn't fire" for this
specific path — the end-to-end correctness check above is the real evidence, and it passed
cleanly. Worth adding explicit logging here in a future session purely for debuggability, not
because anything is currently wrong.

### 7.13 Keyboard matrix — still only Gboard, and why

Phase 0's own flagged matrix (Gboard, Samsung Keyboard, SwiftKey, MIUI) remains only
one-third tested. **Samsung Keyboard and MIUI are not just untested but genuinely untestable on
this project's test device** (a Motorola Moto G54 5G, see phase_3/PLAN.md §4 and the memory
record of this device) — both are OEM-exclusive keyboards not distributed for other manufacturers'
hardware, so there is no way to install either here regardless of effort spent. **SwiftKey is
installable** (this device has Play Store and working internet, confirmed live this session — `adb
shell ping 8.8.8.8` succeeded, `com.android.vending` is present) but was **not attempted this
session by explicit user choice**, after being asked directly: installing a third-party keyboard
and switching the device's system default IME to it are real, lasting changes to the user's actual
phone (not a disposable CI device), not something to do unprompted. Recorded as untested-by-choice,
not silently skipped or forgotten.

### 7.14 §7.6 solved — the surface was being repositioned, not the text unpainted

**Status: root cause found, fixed, and verified live on the Moto G54. §7.6/§7.9's
"an unrelated text node vanishes once a `TextField` is focused, and never comes
back" is closed.**

**It was never a rendering bug.** Every one of §7.6's own findings was right and
none of them was the cause: the tree data really was byte-for-byte identical, the
glyphs really were being shaped, both paint backends really did behave the same, and
`repro_7_6`/`repro_atlas_lifetime` really did clear the whole shared pipeline. All of
that was true because the renderer was doing its job correctly the entire time. The
text was being drawn — **underneath the status bar**, where it cannot be seen.

**Root cause.** `ZKiteActivity.applyInsetsAsPadding` inset the surface by calling
`view.setPadding(...)` with the system-bar insets. **Padding does nothing to a
`SurfaceView`.** A `SurfaceView`'s surface is sized and positioned from the view's own
layout bounds and ignores padding entirely — so that call never inset anything. It
only *appeared* to work because the content frame Android hands the Activity at launch
already happened to be inset by the system bars, which put the surface in the right
place by luck. Showing and hiding the IME changes that frame to the full window and it
never changes back, so the `SurfaceView` gets re-laid-out at `y=0` instead of
`y=117` — and every pixel ZKite draws moves up by exactly the status-bar height, for
the rest of the session. The topmost widget (the counter digit, in every app this bug
was ever reported against) goes behind the status bar and stays there.

Measured directly, via `uiautomator dump`, which is what finally made it unambiguous:

```
at launch:                  ZKiteSurfaceView bounds=[0,117][1080,2340]   (2223 tall)
after one IME show/hide:    ZKiteSurfaceView bounds=[0,0][1080,2223]     (2223 tall)
```

Same size, moved up 117px — exactly the status-bar height, and exactly the uniform
shift measured off the screenshots (every row's pitch unchanged, so a scale/clip
explanation was ruled out at the same time). The render thread's own log said the
other half: `Resize 1080x2400` arrived when the keyboard appeared and **no resize back
to 2223 ever followed**.

**Why every previous session missed it.** The bug is invisible to every tool the
investigation reached for. It is not in the tree (§7.6 checked), not in layout (§7.9),
not in either rasterizer (`repro_7_6`), not in the GPU atlas (`repro_atlas_lifetime`),
and cannot reproduce on a host at all, because there is no `SurfaceView` and no status
bar. It is also invisible in a screenshot *unless* you measure against a known-good
one — "the counter is gone" reads as a paint failure, and the 117px shift of
everything else is easy to miss when the rest of the screen is a uniform list. The one
diagnostic that would have shown it immediately — logging the surface size on
change — did not exist until this session added it.

**Fix.** Insets are now applied as layout **margins**, not padding
(`ZKiteActivity.applyInsetsAsMargins`), which genuinely move and resize the view, so
the surface follows. Paired with an explicit `setDecorFitsSystemWindows(false)` (with
a `SYSTEM_UI_FLAG_LAYOUT_*` equivalent for API 26–29) so the parent frame is *always*
the full window and those margins are the single source of truth for where the surface
sits, rather than being layered on top of an inset the platform may or may not have
already applied. Three consequences, each handled:

- **The surface no longer resizes when the keyboard appears at all** — the margins
  come from `systemBars()`, deliberately not `systemBars() | ime()`. ZKite already
  handles the IME by panning the focused field itself (§7.9); folding the IME inset
  into the surface would resize the swapchain and rebuild the `vello_hybrid::Renderer`
  twice per focus *and* double-count against that pan. Verified: `Resize` no longer
  fires at all across repeated focus/blur cycles.
- **The IME inset is converted into the view's coordinate space**
  (`ZKiteSurfaceView.onImeInsetsChanged` subtracts the bottom system-bar margin),
  because `render_thread.rs` compares it against `self.height`, which is now the safe
  area rather than the window. Visible in the logs as `ime_pan=853` where it used to
  be `913` — exactly the 60px navigation bar, which is the arithmetic proving the
  conversion is right rather than merely different.
- **The selection overlay is offset by the same margins**
  (`TextSelectionOverlay.setParentOffset`), since it is a sibling of the surface view
  in the now-full-window parent but is positioned in the *surface's* coordinate space
  (§7.8) — without this it would sit one status-bar height above the field it covers.

Everything downstream — touch coordinates, layout, caret geometry, a11y bounds —
keeps its existing meaning of "relative to the safe area" and needed no changes at
all, which is the reason margins were chosen over the alternative (surface stays
full-window, renderer insets its own layout): same correct result, far smaller blast
radius.

**Verification, live on the Moto G54 (arm64-v8a, Android 15), the same physical device
every prior IME section used.** The exact sequence that previously broke the app
permanently:

```
launch:                 frame: 1080x2223 via gpu
focus  ("Your name"):   size=1080x2223  focused=Some("r.4.0")  ime_pan=853
blur:                   size=1080x2223  focused=None           ime_pan=0
```

Then five full focus/blur cycles alternating both fields, plus taps and a scroll: **40
redraws, every one at `1080x2223`, zero `Resize` events, zero panics.** `uiautomator
dump` afterwards reports the counter at `bounds="[519,149][561,219]"` and the surface
at `[0,117][1080,2340]` — **byte-for-byte identical to launch**. Screenshots before and
after the cycle are visually indistinguishable; previously this same sequence left the
counter gone and everything else 117px too high. The counter also still increments on
tap (`0 → 2`) and the list still scrolls, so nothing regressed to get there.

The three behaviours that had to move with the surface were each re-verified live too,
since all of them are coordinate-space changes and none would have failed to compile:
typing through the real Gboard/`GameTextInput` path still commits into the field
(`onTextInputEvent: decoded text="au"`); a long-press still opens the selection overlay
**exactly over the field it covers**, not a status-bar height above it (screenshotted —
this is `setParentOffset`'s whole purpose); and `uiautomator dump` still reports the
full accessibility tree with correct roles, labels and bounds.

**A standing lesson worth recording, since this cost several sessions.** Three
independent host-only reproductions all came back clean and all three were *correct* —
the bug genuinely was not in the code they exercised. Repeated negative results
against the portable pipeline were themselves the signal that the cause lay in the
platform integration, and that signal was available long before it was acted on. When
host repros keep clearing the shared code, stop writing more of them and go measure
the real thing on the device — here, one `uiautomator dump` compared against a
known-good one would have found it outright.

## 8. AccessKit — a virtual accessibility tree for `AccessibilityNodeProvider`

§5 item 2. Scope, deliberately: build the virtual semantics tree (node → screen rect, role,
label/value, actions) and wire it to a real Android `AccessibilityNodeProvider` so TalkBack (or
any assistive technology) can see and query `ZKiteSurfaceView`'s content, and make the core
click/focus actions actually dispatch back into `zkite_py` — not Autofill (phase 0's own note that
it "reduces to the same missing piece" still applies; not attempted this session), not scroll
actions, not in-field text-selection/traversal actions.

### 8.1 Don't hand-roll it — same call as GameTextInput, for the same reason

plan1.md §3's own diagram lists `accesskit` for exactly this. Read live against the actual crate
source (`~/.cargo/registry/.../accesskit_android-0.7.5/src/*.rs`, fetched and read in full this
session, not guessed from the README) before writing any glue code — same discipline §7.1 already
applied to `vendor/gametextinput`. `accesskit_android` ships two layers: a low-level `Adapter`
(more flexibility, more glue) and `InjectingAdapter`, which installs itself on an arbitrary
`android.view.View` via `View.setAccessibilityDelegate`/`setOnHoverListener` — both plain platform
APIs — using a Java `Delegate` class it loads at runtime from an **embedded `.dex`**
(`InMemoryDexClassLoader`, the `embedded-dex` Cargo feature). `InjectingAdapter` is the one used
here, and it turned out to sidestep both of the real problems §7 hit building GameTextInput, not
just one:

- **No androidx dependency at all.** `Delegate.java` (read in full — the crate's `java/` directory
  has exactly one file) imports only `android.os`/`android.view`/`android.view.accessibility` —
  none of `vendor/gametextinput`'s `androidx.core.view.WindowInsetsCompat`/`ViewCompat` chain that
  needed a hand-written `R.java` stub (§7.3 item 4). Nothing new to fetch, nothing new to dex
  beyond the crate's own embedded bytes.
- **Safe to construct from the render thread directly, unlike `GameTextInput_init`.** §7.2's bug
  was `FindClass`-by-name resolving against the wrong classloader when called from a JVM-attached
  (not JVM-created) thread. Read end to end, `accesskit_android::inject::delegate_class`'s
  `embedded-dex` path never does that: it `find_class`es only `dalvik/system/InMemoryDexClassLoader`
  (a boot-classpath class, resolvable from any thread) and then calls `loadClass` on that fresh
  loader instance — a method call on an object already in hand, not a lookup by name against the
  calling thread's own classloader. Confirmed by reading `inject.rs`, not assumed from the feature
  name. So — unlike `text_input::TextInput`, which is deliberately confined to the UI thread (see
  its own module doc) — this session's `a11y::install`/`InjectingAdapter::new` is called directly
  from the render thread's own existing JNI attachment (`State::render_env`, already used for
  `focusAndShowKeyboard`/`blurAndHideKeyboard`), with no new UI-thread hop and no new JNI entry
  point in `lib.rs`.

### 8.2 Design: role/label/value/actions in `zkite_pybridge`, bounds + JNI wiring in `gpu_harness`

Mirrors the existing paint-vs-layout split (§3's module doc: "paint is the one thing that
legitimately differs per backend, layout must not"), applied one layer over: **semantics** (what a
node *is* — role, label, value, checked/toggled, which actions it supports) is derived from the
same `RawNode` tag/attrs `zkite_pybridge::build_node` already reads, so it lives there, stays
host-testable (no JNI, no Android — the base `accesskit` crate is a pure data/schema crate with no
platform code), and is exercised by real host tests against the real `zkite_py` counter app
(`zkite_pybridge`'s own test suite, §8.4). **Bounds** can't be known until `zkite_renderer::Renderer
::layout`/`positioned` have run, and the **actual Android wiring** (`InjectingAdapter`, the JNI
bridge) is inherently platform-specific — both live in `gpu_harness`, the same crate that already
owns `positioned()`'s output and every other JNI surface.

Concretely: `BuiltTree` gained `a11y_nodes: FxHashMap<taffy::NodeId, accesskit::Node>` (built
alongside `node_ids`/`events` in `build_node`'s existing walk) and `a11y_ids: FxHashMap<accesskit
::NodeId, Arc<str>>` (the reverse map — `accesskit::NodeId` is a stable hash of the `zkite_py` node
id, via `rustc_hash::FxHasher`, so the same conceptual element gets the same id across rebuilds,
letting AccessKit's own tree-diffing work correctly; the hash isn't invertible, so the reverse map
is what turns an incoming `ActionRequest.target_node` back into a dispatchable `zkite_id`). A new
`BuiltTree::accessibility_tree_update(positioned, focused)` stamps each node's real bounds (from
`PositionedNode.rect`, the same absolute coordinate space every other consumer of `positioned()`
already uses — no `Affine`/transform is ever set on any node, so accesskit's own transform
composition is just the identity, and "the tree's container" bounds `Node::bounding_box()`
documents are exactly the rect this renderer already computed) and assembles the final
`accesskit::TreeUpdate`. `gpu_harness::render_thread::State::push_a11y_update` calls this after
every `redraw()` and hands the result to `InjectingAdapter::update_if_active`.

**Role mapping** (`zkite_pybridge::a11y_role`) reads exactly the same tag/attrs disambiguation
`build_node`'s own `is_text_field` comment already describes for why `"input"` alone is ambiguous
between `TextField`/`Switch`/`Slider`: `input_type` present → a `TextInput` variant (further
split by its value: `password`/`email`/`url`/`tel`/`number`); `checked` present → `Role::Switch`;
`min`/`max` present → `Role::Slider`; `button`/`span`/`img`/`svg` map directly; everything else
(`div` — `Column`/`Row`/`Stack`/`Grid`/`ScrollView`) is `Role::GenericContainer`. That last choice
is safe even for the app's own root widget specifically because `accesskit_consumer::common_filter`
(read directly, not assumed) treats `GenericContainer` as `FilterResult::ExcludeNode` — invisible
to assistive tech, but its children still walked and exposed — the same "presentation role"
behavior a plain unstyled HTML `<div>` gets in a real browser's own accessibility tree.

**A bare `#text` child never gets its own `accesskit::Node`** — it's concatenated into its
*immediate* tagged parent's own label/value instead (`text_label_from_children`), the same
"innerText" collapsing a real accessibility tree already does for e.g. a `<button>Increment
</button>` (one accessible node named "Increment", not a button node containing a separate text
node). This is why no synthetic id scheme was needed for text leaves at all — only element nodes
(which already have a `zkite_py` id, explicit `key` or auto `path`) ever need one.

**A `TextField`'s label vs. value split matters and is real, not incidental:** when the field is
showing its dimmed placeholder (`build_node`'s own `dimmed` flag — nothing real typed yet), that
text becomes the accessible **label** (the closest thing to a real accessible name this widget has,
since `zkite_py`'s `TextField` has no separate `label=` prop — the same fallback a plain HTML
`<input placeholder=...>` with no `<label>` gets); once there's real content, it becomes the
**value**, with no label. Verified this actually reaches Android correctly, not just internally —
see §8.4.

**Slider gets a numeric range, never a text `value`** — checked directly against
`accesskit_android::node::populate_node_info`'s source: it only emits Android's `RangeInfo` when
`self.0.data().value().is_none()`, so calling `set_value` on a `Slider` node (an easy mistake,
since a `TextField` calls it) would silently suppress the range Android actually renders. A host
test (`slider_gets_numeric_range_not_a_text_value`) locks this in.

### 8.3 A real bridging finding: two different `jni-sys` majors in one binary, and why the bridge is sound

`gpu_harness` already uses raw `jni_sys = "0.3"` for every existing JNI entry point (deliberately —
see `lib.rs`'s own module doc on why no `jni` crate). `accesskit_android`'s `jni = "0.21"`
dependency is built on `jni-sys 0.4` — a **different, non-interchangeable Rust type** for the same
stable JNI C ABI. Checked directly, not assumed: `jni-sys 0.3.1` (the specific version this
workspace resolves) is itself a compatibility shim that re-exports 0.4's opaque `_jobject` type
verbatim (`pub type _jobject = jni_sys_04::_jobject;`), so `jobject`/`jstring`/etc. values need
**no cast at all** between the two crates — but it defines its own separate `JNIEnv`/
`JNINativeInterface_` struct rather than re-exporting 0.4's, so a `.cast()` on the `*mut JNIEnv`
pointer specifically *is* required. `a11y::bridge_env`/`bridge_jobject` do exactly this — the same
category of raw-pointer reinterpretation `lib.rs`'s `Native_onSurfaceCreated` already does
(`env.cast()` for the `ndk` crate's `NativeWindow::from_surface`), sound because both crates
describe the identical underlying `JNINativeInterface` C struct layout, not a new kind of risk.
`JObject`'s own `'static` lifetime (used to wrap the already-promoted `ZKiteSurfaceView` global ref)
is the *documented* correct choice for an already-global reference, confirmed by reading
`jni-0.21.1/src/wrapper/objects/jobject.rs` directly (no `Drop` impl on `JObject` either, so
wrapping it never double-frees the ref this crate already owns and leaks once per process, per
`text_input.rs`'s own precedent).

### 8.4 Verification: live, on the real device, against real Android accessibility tooling

Moto G54 5G, same device every prior section used. `zkite_pybridge`'s host suite (5 tests, all
passing, including two new ones written this session) covers the pure data logic: role
disambiguation for the three `input`-tagged widgets, the Slider numeric-range-not-value rule, and
— against the *real* `zkite_py` counter app via the same `pyo3-ffi` embedding
`builds_the_real_counter_app_and_dispatches_a_tap` already used — that the Button ends up
`Role::Button`, labeled "Increment", `Action::Click`-supported, with real non-zero bounds from a
real `positioned()` call, and that the counter digit is a plain labeled (non-clickable) node.

Cross-compiled clean for `arm64-v8a` (one real compile error along the way — a missing import,
fixed immediately; nothing else). Installed and launched on-device: `ensure_a11y_adapter:
InjectingAdapter installed on ZKiteSurfaceView` logged with no crash, no change to the app's own
rendering (screenshot-confirmed identical to pre-AccessKit). `adb shell uiautomator dump` — a real
Android accessibility client, distinct from the app itself — against the live demo app (Button,
Switch, Slider, 60-row ScrollView, two TextFields, one `secure=True`) shows, verbatim:

```
class="android.widget.TextView"    content-desc="0"
class="android.widget.Button"      content-desc="Increment"
class="android.widget.TextView"    content-desc="Notifications"
class="android.widget.ToggleButton" checkable="true" checked="true"
class="android.widget.TextView"    content-desc="Volume: 40"
class="android.widget.SeekBar"
class="android.widget.TextView"    content-desc="Item 0" ... "Item 59"
class="android.widget.EditText"    content-desc="Your name"  focusable="true" password="false"
class="android.widget.EditText"    content-desc="Password"   focusable="true" password="true"
```

Every one of those Android class names and flags (`ToggleButton`/`checked`, `SeekBar`,
`EditText`/`password`) comes from `accesskit_android::node`'s own `class_name()`/`is_password()`
mapping, driven *entirely* by this session's role assignment — none of it hand-mapped to Android
types directly. `Switch`'s `checked="true"` matches the demo's live `Toggled::True`; the
`secure=True` field's `password="true"` falls out of assigning it `Role::PasswordInput`, with zero
extra code. Off-screen (scrolled-past) list rows report zero-area bounds in this dump — plausibly
correct (the framework's own bounds-intersection-with-screen behavior for genuinely off-screen
virtual nodes), not chased further this session.

Real TalkBack (`com.google.android.marvin.talkback`, enabled live via `adb shell settings put
secure enabled_accessibility_services` — the user was asked first, and it was turned back off
after testing, `settings delete secure enabled_accessibility_services`) successfully bound to the
app: `dumpsys accessibility` showed `touchExplorationEnabled=true`/`TouchExplorer` active for the
display, and a screenshot showed TalkBack's own accessibility-focus rectangle drawn correctly
around the whole `ZKiteSurfaceView` — independent, OS-level confirmation that the
`AccessibilityDelegate`/`AccessibilityNodeProvider` really is attached and recognized, not just
present in a dump. A full logcat sweep for `panic`/`FATAL`/`AndroidRuntime` crash markers across
the entire test session (tree dumps, TalkBack enable/disable, its own tutorial, multiple
taps/swipes) came back clean, and the process (`pidof`) stayed alive throughout.

### 8.5 What wasn't verified, and why — a real, named gap, not a guess

**A live TalkBack gesture (double-tap-to-activate) driving `ActionHandler::do_action` end to end
was attempted and did not succeed on this device, for a specific, confirmed reason: `adb shell
input tap`/`swipe`-synthesized touches never engaged touch exploration for this app at all.**
Concretely checked, not assumed: a fresh `uiautomator dump` after every tap/double-tap/swipe
combination tried still showed `focused="true"` only on the top-level `android.view.View` (the
whole surface, AccessKit's `HOST_VIEW_ID`) — meaning `on_hover_event`/`virtual_view_at_point`
never even fired to move focus onto a specific child, let alone a click reaching `do_action`.
Every one of those same synthetic touches instead reached `ZKiteSurfaceView.onTouchEvent` directly
(confirmed via this session's own added logging — see below), i.e. the *ordinary*, already-proven
touch path, not the accessibility one — for a single tap, a rapid double-tap at the same point,
and a swipe alike. This is the same class of finding §7.4 already made for a different input
mechanism ("`adb shell input text` does **not** exercise this integration at all... synthesizes
raw `KeyEvent`s through Android's normal key-dispatch path") — ordinary `adb`-injected input events
are not always a faithful stand-in for the exact system pipeline a real finger/gesture goes
through, and this is the accessibility-specific instance of that same lesson. Two small, permanent
diagnostic log lines were added to make this checkable in a future session
(`a11y::Actions::do_action` logs the incoming `ActionRequest`; `render_thread::
handle_a11y_action` logs which `zkite_id`/action it resolved, or that it didn't) — neither fired
during any of this session's attempts, which is the concrete evidence behind this section's claim,
not an absence of evidence.

**What this does and doesn't put in doubt:** `handle_a11y_action`'s own logic was not exercised
live, but it does nothing new — its `Action::Click`/`Action::Focus` branches call straight into
`dispatch_tap_to` (factored out of `dispatch_tap` this session specifically so both callers share
one implementation) and `focus_text_field`, both already verified live via real touch in prior
sessions. The gap is specifically "does a real assistive-technology gesture reach
`ActionHandler::do_action` on this device," not "is the code behind it correct" — a future session
picking this up should reach for a real physical finger touch (this environment has none) or a
purpose-built `UiAutomator`-instrumented test (drives via the Accessibility API programmatically
rather than synthetic touch injection, and was judged out of scope to build from scratch this
session) rather than more `adb shell input` variations, which this session already tried
several of (tap, double-tap at one point, swipe, `KEYCODE_TAB`) without one engaging touch
exploration.

### 8.6 Known gaps

- **Autofill not built.** Phase 0's own note that it and a11y "reduce to the same missing piece"
  still applies — this session built that piece (the virtual semantics tree), but didn't wire a
  second consumer to it.
- **No scroll actions** (`ACTION_SCROLL_FORWARD`/`BACKWARD`) and no in-field text-selection/
  traversal actions (`ACTION_SET_SELECTION`, movement-granularity next/previous) — `accesskit_android
  ::node::populate_node_info` already advertises these when a node supports the underlying
  `accesskit::Action`, but nothing in `handle_a11y_action` implements them yet; TalkBack would
  offer the affordance and it would currently no-op.
  A `ScrollView`'s `overflow-y-scroll` maps to `Role::ScrollView` today only implicitly (it's a
  `div`, so it falls to `Role::GenericContainer`) — scroll-specific a11y semantics (is_scrollable,
  scroll position/max reported to AT) aren't wired at all.
- **Icon (`svg`) has no real accessible label prop.** Falls back to its internal sprite name
  (`icon_name`) as a better-than-nothing default (§8.2) — a real `Icon(label=...)`/
  `decorative=True` prop on the widget itself is the correct fix, not attempted this session (no
  such prop exists on `Icon`/`Image` yet at all).
- **The interactive action-dispatch round trip is unverified live** — §8.5.
- **Off-screen list-row bounds in `uiautomator dump`** show as zero-area — plausible, not
  root-caused (§8.4's own note).
- **No per-node `text_selection` reported for a focused `TextField`** — `FocusedField` wasn't
  extended to carry the live selection range into `zkite_pybridge`; TalkBack would see the field's
  value but not exactly where the cursor/selection is inside it. A small, well-scoped follow-up
  (thread `render_thread::State::focused_selection` through the same path `FocusedField::text`
  already takes), not attempted this session to keep scope to what §5 item 2 actually named.

### 8.7 A hard crash: raising an `AccessibilityEvent` while accessibility is off

**Status: found, root-caused, fixed, and verified live on the Moto G54.** Found while
investigating §7.6 — unrelated to it, and considerably more severe than the bug being
hunted, since it kills the process outright.

**Symptom.** Focusing a `TextField` aborts the whole app:

```
E ZKiteGpuHarness: zkite-render thread PANICKED: panicked at accesskit_android-0.7.5/src/event.rs:64:6:
E ZKiteGpuHarness: called `Result::unwrap()` on an `Err` value: JavaException
F libc    : Fatal signal 6 (SIGABRT), code -1 (SI_QUEUE) in tid 29972 (zkite.gpuharness)
```

(The `zkite-render thread` prefix is misleading — `run()` installs a *global*
`std::panic::set_hook`, so every thread's panic is labelled that way. `tid == pid`
here: this panic is on the **UI thread**, which is where `accesskit_android` posts its
event-raising work.)

**Root cause.** `AccessibilityManager.sendAccessibilityEvent` throws
`IllegalStateException("Accessibility off. Did you forget to check that?")` — AOSP's
own wording — when accessibility is disabled and the caller is on the main looper.
`accesskit_android`'s `event.rs::send_completed_event` reaches that throw via
`ViewParent.requestSendAccessibilityEvent` and `.unwrap()`s the result, turning a
documented, expected Java exception into a Rust panic and therefore a process abort.

The trigger is nastier than "the user runs a screen reader," which is why §8.4's own
verification never hit it: **any transient accessibility client activates AccessKit's
adapter and then goes away.** `adb shell uiautomator dump` — the exact tool §8.4 used
to verify the a11y tree — switches accessibility on, which moves the adapter from
`State::Placeholder` to `State::Active`, then disconnects. The adapter has no way to
observe that accessibility was switched off underneath it (nothing in its API reports
it), so it keeps raising events on every subsequent tree update, and the next one
kills the app. In §8.4's original session TalkBack happened to be enabled throughout,
so the throw never fired.

**Confirmed by a clean A/B on real hardware**, not inferred from the stack trace:

| Accessibility state | Identical tap on the same field | Result |
|---|---|---|
| **On** (TalkBack enabled) | focus, `Resize`, `ime_pan=913`, caret | works, process alive |
| **Off** (adapter still `Active`) | same tap | **SIGABRT** |

**Fix.** `render_thread::State::push_a11y_update` now consults
`ZKiteSurfaceView.isAccessibilityEnabled()` (a real
`AccessibilityManager.isEnabled()` call — literally the check the exception message
asks for) before handing anything to `update_if_active`. Queried fresh on every push
rather than cached, since accessibility can be toggled at any instant, and a stale
`true` is precisely the abort this guards. Fails **closed**: if the env, the view, or
the method lookup is unavailable it returns `false`, because skipping one frame's
accessibility update degrades assistive tech while wrongly proceeding crashes the app.

`a11y::set_latest_tree` is deliberately left *outside* the guard — the latest tree is
still recorded while accessibility is off, so `Activation::request_initial_tree` hands
a screen reader started mid-session the real current tree rather than a stale or
placeholder one.

**Verified live**, same device: the exact `uiautomator dump` → tap-the-field sequence
that reproduced the abort 100% of the time now completes normally, and a full sweep
(a11y dump + five focus/blur cycles + taps + scroll) runs with zero panics and the
process alive throughout. Accessibility itself still works — a dump taken after the
fix reports every widget with correct roles, labels and bounds exactly as §8.4
documented.

**Upstream.** This is arguably an `accesskit_android` robustness bug: `.unwrap()` on a
JNI call the platform documents as throwing. Worth reporting — the fix there would be
to check `AccessibilityManager.isEnabled()` internally, or to propagate the error
rather than panic. ZKite's own guard is correct regardless and shouldn't wait on it.

## 9. Per-widget conformance testing against the WebView backend

§5 item 5, plan1.md §5's own exit bar for the whole phase: *"every widget gets a conformance
test that must produce pixel-comparable output on both backends before the switch flips."*
Scope: all 12 phase-1 widgets (`zkite.__all__` minus `App`/`store`/`Widget`), each rendered
through both backends from the *same* `RenderContext`/node-id walk and compared pixel-for-pixel,
entirely host-side — no device or emulator needed, since both backends this pipeline compares
already run on the host (`zkite_renderer`+`vello_cpu` per §3; real Chromium via Playwright per
`phase_1/webruntime/test/e2e.mjs`, reused unmodified here).

### 9.1 Pipeline

`phase_3/conformance/`, four pieces chained by `run.sh`:

```
fixtures.py (Python, imports phase_1/zkite_py directly)
  -> manifest.json: {name: {width, height, html, tree}}   — one entry per widget
       html  = Widget.render(RenderContext(), "r")          (WebView backend's own input)
       tree  = Widget.to_tree(RenderContext(), "r")          (native backend's own input)

conformance render-all manifest.json out/     (Rust)
  -> out/<name>_native.png   via zkite_pybridge::{build, RawNode} + zkite_renderer + vello_cpu —
     the exact same tree-JSON-to-pixels path gpu_harness's render thread uses for a real app,
     not a simplified stand-in (§8.1 already established `build`/`RawNode` need no CPython
     embedding on their own — only `zkite_pybridge::embed` does, and this binary never calls it)

screenshot_web.mjs manifest.json out/     (Node, Playwright)
  -> out/<name>_web.png      loads the real, unmodified phase_1/webruntime/index.html shell
     (vendored Tailwind v4 JIT + morphdom), applies each fixture's html via the real
     window.ZKiteRuntime.applyHtml, screenshots *only* the fixture's own root element (#r)

conformance diff-all manifest.json out/     (Rust)
  -> out/<name>_diff.png + out/report.json + a console table
```

**Why every fixture has an explicit pixel width/height on its own root node, not just "however
big it renders":** neither shell's own root-stretch behavior is part of what's under test.
`phase_1/webruntime/index.html`'s `#zkite-root` CSS (`min-height:100%; display:flex`) and
`zkite_pybridge::build`'s synthetic `flex flex-col w-full h-full` wrapper (see that function's own
doc comment) both exist to make a `flex-1` app root fill an *unconstrained* viewport — a real
concern for a real app, but exactly the kind of shell-dependent stretching that would make a
screenshot's size depend on which shell it came from rather than on the fixture itself. Giving
every fixture's own root an explicit `w-[Npx] h-[Npx]` sidesteps that: both screenshots are
cropped to precisely that element's box on both sides, so a size mismatch (checked by
`screenshot_web.mjs` itself, which fails loudly rather than silently comparing mismatched crops)
would mean a real layout bug, not a shell-stretch artifact.

**Tolerance, not bit-exact comparison.** `conformance`'s `CHANNEL_TOLERANCE` (40, per-channel,
see its own doc comment in `src/main.rs`) exists because two genuinely different rendering
engines — Chromium's own text/shape rasterizer vs. `parley`+`vello_cpu`'s — never produce
byte-identical anti-aliased edges from the same font file at the same size; a tolerance of 0
would fail every fixture with any text or rounded corner in it purely on AA noise, which is not
the signal this tool exists to catch. "Pixel-comparable," per plan1.md §5's own phrase, is
implemented here as: decode both PNGs, count pixels whose max per-channel delta exceeds the
tolerance, report that as a percentage, and write a diff image (mismatched pixels in solid red
over a dimmed grayscale of the web reference) so a human can see *what* differs, not just how
much.

### 9.2 Results — run live, not just built

`./run.sh` end to end, this session, reproducible (re-run twice, identical numbers both times):

```
button         8.29%  max_delta=232 (1327 / 16000 px)
column         0.20%  max_delta=171 (105 / 52800 px)
grid           0.03%  max_delta=82  (12 / 38400 px)
icon           1.22%  max_delta=253 (122 / 10000 px)
image          0.15%  max_delta=51  (16 / 10800 px)
row            0.40%  max_delta=160 (96 / 24000 px)
scrollview     2.77%  max_delta=226 (887 / 32000 px)
slider         9.53%  max_delta=255 (1144 / 12000 px)
stack          0.07%  max_delta=60  (27 / 40000 px)
switch         4.13%  max_delta=218 (347 / 8400 px)
text           6.68%  max_delta=253 (1176 / 17600 px)
textfield      3.36%  max_delta=112 (443 / 13200 px)
```

**6 of 12 (`column`, `grid`, `row`, `stack`, `scrollview`, `textfield`) are genuinely clean** —
all under 3.5%, and visual inspection of every diff image (not just the number) confirms what's
left is AA/rounded-corner edge noise, not a real divergence: `grid`'s 0.03% and `stack`'s 0.07%
in particular are about as close to "the same picture" as two different rendering engines get.
This is real confirmation, not an assumption, of §3/§4's own Button-centering and
text-measurement fixes and §6's `min-h-0`/`flex-col` `ScrollView` fix — the exact things §5 item
5's original bullet named as "early, load-bearing instances of what this milestone is meant to
catch" turn out to have actually closed the gaps they were meant to close.

**The other 6 show a real, explainable divergence each — four distinct root causes, three of them
newly found by this work rather than just confirmed. Two of the six (`icon` 1.22%, `image`
0.15%) are numerically small only because the missing content is a small fraction of their
canvas — see item 4 below, the *cause* is exactly as real as the higher-percentage ones:**

1. **`button` (8.29%) and `text` (6.68%): non-regular font weights don't render on the native
   side at all.** Visual inspection (not just the percentage) shows it plainly: WebView's "Hello
   ZKite" (`font-bold`) is visibly bold; native's is not, rendered at the same (regular) stroke
   weight as unstyled text. Root cause, confirmed by reading the code, not guessed: `Renderer::new`
   (`phase_3/renderer/src/lib.rs`) registers exactly one font file (`DejaVuSans.ttf`, regular/400)
   under the `SansSerif` generic family; `layout()`'s measure closure does set
   `StyleProperty::FontWeight(weight)` on every shaping pass, but with only one face registered,
   fontique's own matching has nothing heavier to select and no synthetic-bold fallback is wired
   up, so every weight silently renders as the one available face. **This was already a suspected
   gap** — §3's own "not yet exercised" list and §5's "smaller gaps" list both already named
   "non-regular font weights" — but this is the first time it's been measured with real pixel
   evidence rather than left as an untested prediction. `button`'s own 8.29% is inflated further
   by its rounded-corner edge AA (visible as a thin red outline in `out/button_diff.png`,
   independent of the text finding) — the text alone accounts for most of it, confirmed by
   `out/text_diff.png` showing the entire glyph area red on a fixture with no rounded corners at
   all. **Not fixed this session** — registering a second (bold) face and picking between them by
   requested weight is a real, contained `zkite_renderer` change, but out of scope for a session
   whose deliverable was the test infrastructure that found this, not a font-matching rewrite;
   flagged here as the natural next fix, same "document, don't hide" convention as every other
   phase's mid-session findings.
2. **`slider` (9.53%): the WebView backend's own `Slider` doesn't match its intended design at
   all — a `webruntime` gap, not a native-renderer one.** `phase_1/webruntime/index.html` has
   real custom CSS for `input[type=checkbox]` (`Switch`'s track/thumb) but **none** for
   `input[type=range]` — `Slider`'s only implied class is `appearance-none w-full`
   (`phase_1/zkite_py/src/zkite/widgets.py`), so Chromium falls back to its own default OS-drawn
   range control (visually, a small floating blue dot with no track at all — see
   `out/slider_web.png`), while the native side paints the custom track/fill/thumb
   `Slider._tree_children` already builds (see `out/slider_native.png`). This was never caught
   before because nothing had screenshotted the WebView backend's `Slider` next to its own
   design intent until this session — a real, previously undocumented gap **in the WebView
   backend**, found by a tool built to check the *native* one. Not fixed this session (a
   `webruntime/index.html` CSS addition, contained but outside this milestone's own scope).
3. **`switch` (4.13%): a real, small, previously unmeasured metric/color mismatch between the two
   backends' independently hand-authored "fake checkbox" designs.** Both backends build a custom
   track+thumb (browsers have no stylable native checkbox chrome either, so this was always going
   to be hand-rolled twice — see `Switch.classes()`'s own comment) — but the two implementations
   were never actually compared pixel-for-pixel until now: `webruntime/index.html`'s CSS uses a
   44×26 track with a 22×22 thumb and an *off*-state color of `#3f3f46` (Tailwind's `zinc-700`);
   `phase_1/zkite_py/src/zkite/widgets.py`'s native `Switch.classes()` uses a 44×24 track (2px
   shorter) with a 20×20 thumb (2px smaller) and an off-state color of `bg-slate-700` (a
   different color *family*, not just a different shade). The *on*-state color (`indigo-500`,
   `#6366f1`) matches
   exactly on both — only the off state and the exact metrics drifted. A small, contained fix
   (align one to the other, most naturally the WebView CSS to `widgets.py`'s own already-Tailwind-
   token values) — not made this session, recorded as a real finding for whoever picks it up.
4. **`icon` (1.22%) and, at a much smaller scale, `image`'s 0.15%: exactly the already-documented,
   already-understood gap, now with a number attached.** `out/icon_diff.png` shows precisely what
   §3/§5 already say: the WebView side paints a real star outline (`out/icon_web.png`); the native
   side paints an empty box of the right size and nothing else (`out/icon_native.png`) — no
   surprise, no new information, just confirmation that the box-sizing approximation
   `Icon._tree_classes` documents is holding (the two boxes occupy the identical screen region;
   only the vector content inside is missing) and a concrete percentage for how much of the frame
   that accounts for. `image`'s 0.15% is smaller because the test fixture's box has a plain white
   `bg-white` fallback already close to the reference 1×1-red-pixel image scaled up — not because
   the underlying gap (no bitmap decode on the native side, same as Icon) is any less real.

### 9.3 Known gaps in the harness itself

- **Text-heavy fixtures' percentages conflate two different signals** — genuine AA noise (harmless,
  present in every fixture) and the real missing-bold-face bug (§9.2 item 1) both show up as "red
  pixels in the glyph area," and this tool doesn't yet distinguish them structurally (e.g. by
  comparing glyph *positions* independently of glyph *rasterization*). Diagnosed by eye this
  session (`out/*_diff.png` + the paired `*_web.png`/`*_native.png`), not by the tool itself —
  a real, if narrower, follow-up would be a text-region-aware comparison mode, not attempted here.
- **No CI wiring.** `run.sh` needs a local Chromium (Playwright's own resolved revision wasn't
  actually cached in this environment — `ZKITE_CHROMIUM_PATH`/a fallback `find` over
  `~/.cache/ms-playwright` papers over it, see the script's own comment) and a matching-version
  `libpython3.12` on the linker's search path for `zkite_pybridge`'s `pyo3-ffi` dependency to link
  into an actual binary (not needed for `zkite_pybridge`'s own `cargo test`, which never produces
  one — `rlib` targets don't resolve that symbol until something does) — both real, documented
  environment requirements (`run.sh`'s own `PY_LIBDIR`/`RUSTFLAGS` handling), neither wired into
  any CI job, since none exists yet for any phase-3 crate.
- ~~**The harness runs at an implicit device pixel ratio of 1, and so cannot see a
  density bug at all.**~~ **Closed — see §13.1.2.** `run.sh` now runs the whole
  pipeline at every dpr in `$ZKITE_CONFORMANCE_DPRS` (default `1 2`): native lays out in
  dp and scales at paint, the WebView gets Playwright's `deviceScaleFactor`, exactly
  mirroring what each does on a real phone. dpr 1 is byte-for-byte the historical run;
  dpr 2 is clean on all 14 fixtures, **and was confirmed to fail on all 14 when the
  density bug was temporarily reintroduced**, so it is not passing for the wrong reason.
  Only integer ratios are exercised, though — see §13.6.
- **No pass/fail gate, only numbers.** `conformance diff-all` reports every fixture's percentage
  and writes a diff image; it doesn't itself decide "under X% is a pass" — §9.2's own read of the
  numbers (8 clean, 4 with named, explained causes) is this session's human judgment, not an
  assertion the tool enforces. A future session wiring this into a real gate should decide
  per-fixture thresholds deliberately (e.g. `icon`/`image`'s known content gap needs a much looser
  bar than `grid`/`stack`'s near-zero one) rather than one global cutoff.
- **12 fixtures, one scene each — not exhaustive.** Each fixture exercises one widget in a
  plausible but singular configuration (one `Slider` value, one `Switch` pair, an 8-row
  `ScrollView`). **Partly addressed for `Slider` (§26.4):** a `slider_range` fixture
  now covers 0/50/100, and it earned its place immediately — the single mid-range
  value had been hiding a thumb that sat half its own width too far right everywhere
  and fell off the track entirely at maximum. — not the full space of props each widget supports (e.g. `TextField`'s `secure`/
  `type` variants, `Button`'s `disabled` state). Widening fixture coverage is straightforward
  (add entries to `fixtures.py`'s dict) but not attempted exhaustively this session.

### 9.4 Follow-up session: three of the four §9.2 findings fixed, the fourth clarified rather than closed

Picked up §9.2's own "natural next fix" list in order. Numbers after all three code changes,
`./run.sh` re-run end to end:

```
button         8.29%  max_delta=232 (1327 / 16000 px)   <- unchanged, see below
column         0.20%  max_delta=171 (105 / 52800 px)
grid           0.03%  max_delta=82  (12 / 38400 px)
icon           1.22%  max_delta=253 (122 / 10000 px)
image          0.15%  max_delta=51  (16 / 10800 px)
row            0.40%  max_delta=160 (96 / 24000 px)
scrollview     2.77%  max_delta=226 (887 / 32000 px)
slider         1.28%  max_delta=253 (154 / 12000 px)    <- was 9.53%
stack          0.07%  max_delta=60  (27 / 40000 px)
switch         0.14%  max_delta=48  (12 / 8400 px)       <- was 4.13%
text          13.41%  max_delta=253 (2360 / 17600 px)   <- was 6.68%, see below
textfield      3.36%  max_delta=112 (443 / 13200 px)
```

**`slider` and `switch` closed cleanly, exactly as scoped.** `slider`: added real
`::-webkit-slider-runnable-track`/`::-webkit-slider-thumb` CSS to
`phase_1/webruntime/index.html`, reskinning the native `<input type="range">` to the same
6px-track/16px-thumb/indigo-fill design `Slider._tree_children` already paints, instead of
falling back to Chromium's bare default range control. The filled portion needed a signal CSS
alone can't derive from a range input, so `Slider._tag_attrs()` (HTML-only — `_tree_attrs`, the
native path, is untouched) now writes a `--zkite-pct` custom property the track gradient reads.
`switch`: aligned `webruntime/index.html`'s checkbox CSS to `widgets.py`'s own values (44×24
track, 20×20 thumb, off-state `#334155`/slate-700) rather than the other direction, per §9.2
item 3's own recommendation — a straight metrics/color copy, no new mechanism. Both verified via
`phase_1/webruntime/test/e2e.mjs` (10/10, unchanged) before and after, plus the conformance
numbers above.

**`zkite_renderer::Renderer::new` now registers a second (bold) font face**, exactly as §9.2 item
1 named as the fix — `new(font_bytes, bold_font_bytes)`, both required, same family (fontique
groups faces into one `FamilyId` by the name parsed out of each file, confirmed directly against
`fontique::CommonData::register_fonts`, not assumed). Once both are registered, `layout()`'s
pre-existing `StyleProperty::FontWeight(weight)` push is sufficient on its own to select the bold
face — no new matching logic needed; verified experimentally before writing any production code
(shaping the same string at weight 400 vs. 700 with both faces registered selects two different,
differently-advancing font blobs) and locked in with a real host test
(`font_bold_selects_a_different_wider_font_face_than_font_normal`, `phase_3/renderer/src/lib.rs`).
Threaded through every caller — `spike1_render_pipeline`, `conformance`, and `gpu_harness`'s full
JNI path (`Native.boot` gained a `fontPathBold` param, `ZKiteActivity` stages a new
`font-bold.ttf` asset, `build_apk.sh` a matching `ZKITE_FONT_BOLD`) — cross-compiled clean for
`arm64-v8a` and the full APK packed successfully with the new asset staged; not re-verified live
on the physical device this session (see known gaps below).

**Why `text` and `button`'s own numbers didn't drop the way `slider`/`switch`'s did — read this
before assuming the bold-face fix didn't work.** Two different reasons, one per fixture, neither
a flaw in the fix itself:

- **`text` (`font-bold`, 700) genuinely improved at the weight-selection level; its raw percentage
  went *up* because fixing the weight mismatch exposed a second, larger, pre-existing gap that
  had been hiding underneath it.** Confirmed directly: temporarily forcing the WebView
  screenshot's page to the same `DejaVu Sans` family the native side bundles (a throwaway
  diagnostic, not a committed change) brought `text` down to 5.00% — *below* the original 6.68%
  baseline, proving the weight fix is real and net-positive once the fonts actually match. Without
  that override, Chromium on this host resolves its default `sans-serif` to **Noto Sans**, not
  DejaVu Sans (`fc-match sans-serif` → `NotoSans-Regular.ttf`) — a completely different typeface
  from whatever `ZKITE_FONT`/`ZKITE_FONT_BOLD` bundles natively. Before this session's fix, native
  rendered `text`'s "Hello ZKite" in *regular*-weight DejaVu Sans against the WebView's *bold*
  Noto Sans — two mismatches stacked (weight *and* family), and weight dominated the signal. After
  the fix, both sides render **bold**, so only the family mismatch remains — and DejaVu Sans Bold
  vs. Noto Sans Bold apparently diverge in glyph metrics *more* than the old (regular DejaVu vs.
  bold Noto) pairing happened to, by coincidence, hence the raw number rising even though the
  actual bug this session targeted is fixed. **This surfaces a real, previously-undocumented gap
  that's materially bigger than font-weight**: `zkite_renderer` has no font-*family* resolution at
  all — `layout()`'s measure closure never reads `ctx.style.text.family`/`Decl::FontFamily`, so
  every app always renders in whichever single family `Renderer::new` was constructed with,
  regardless of `font-sans`/`font-serif`/arbitrary-family classes. Not attempted this session —
  real scope (deciding what native even *has* available to select between, since Android ships no
  fontconfig-equivalent either) well beyond "register a bold face."
- **`button` (`font-medium`, 500) is untouched — byte-for-byte identical native PNG before and
  after this session's fix — and that's *correct*, not a miss.** Verified directly with a host
  test (`font_medium_falls_back_to_the_regular_face_not_bold_per_css_weight_fallback_rules`,
  `phase_3/renderer/src/lib.rs`): with only two faces registered (400, 700), weight 500 resolves
  to the *regular* face, not bold. This isn't a fontique quirk — it's the CSS Fonts module's own
  documented weight-fallback rule (a request for 500 checks 400 before checking anything heavier),
  and empirically the WebView side falls back exactly the same way for the exact same reason:
  `fc-match "sans-serif:weight=medium"` on this host *also* resolves to a Regular face, because
  neither DejaVu nor (here) Noto Sans ships a true 500-weight file. Both backends were already
  spec-correctly rendering `font-medium` as regular before this session, and still are — §9.2 item
  1's original phrasing ("non-regular font weights don't render natively at all") slightly
  overstated `button`'s specific case; the part that was genuinely true and fixed is `font-bold`
  specifically (`text`'s fixture), not every non-400 weight in the abstract. `button`'s own 8.29%
  was, and remains, dominated by the same font-family gap `text` newly exposed, plus the
  rounded-corner AA §9.2 item 1 already named — neither closed by registering more faces DejaVu
  doesn't actually ship (there is no real "DejaVu Sans Medium" file to register).

**Known gaps after this follow-up session:**

- **The font-*family* gap `text`'s own number surfaced is real, undocumented until now, and
  unfixed.** Larger in practice than the weight gap this session closed — every native text node
  renders in one hardcoded family regardless of any `font-*` class. Worth its own milestone before
  claiming full text conformance, not a quick follow-on to this one.
  **Resolved — see §9.7**, which also found that `line-height` had exactly the same bug hiding
  behind it, and closed the "the two backends don't even have the same fonts" half by having both
  read one staged set.
- **The bold-font-asset plumbing through `gpu_harness` (JNI signature, `ZKiteActivity` asset
  unpacking, `build_apk.sh` staging) was cross-compiled and packed successfully but not
  re-verified live on the physical device** — §3/§4/§7's own "compiles ≠ done" standard technically
  still applies here; this session's verification stopped at a clean `arm64-v8a` cross-compile +
  APK pack, not an install+launch+screenshot pass. Low-risk (the change is additive and mirrors
  the existing, already-proven `font.ttf` asset path exactly), but named rather than assumed.
- **`conformance`'s own tolerance/reporting still doesn't distinguish a font-*family* mismatch
  from a font-*weight* one** — §9.3's own "text-heavy fixtures conflate two signals" gap, now with
  a third signal (family) added to the pile it already didn't structurally separate.

### 9.5 The tolerance was hiding a whole class of divergence — including a palette mismatch across every saturated colour

Found while adding §11's `list` fixture, by the oldest method in this document: looking at
the two PNGs instead of the number.

**The `list` fixture scored 3.59% both before and after a fix that visibly changed the
render.** The `List` row wrapper was laying out wrong natively (§11.4) — every row's
background painted at *a third* of its correct width. Fixing it made the native PNG match
the web one. The conformance number did not move by a single pixel: 3.59%, `max_delta=226`,
1150/32000, identical to the digit, before and after. An independent reimplementation of the
diff in Python reproduced that identical number, so the tool was computing exactly what it
claimed to.

**Root cause: `CHANNEL_TOLERANCE` is 40, and dark-theme surface colours are closer together
than that.** `bg-slate-800` (30,41,59) against `bg-slate-950` (2,6,23) is a max per-channel
delta of **36**. So a row background that is entirely absent from two thirds of where it
belongs is, pixel for pixel, "the same" as one that is present. Only the row's white text
ever exceeded 40, and the text sat in the same place either way. §9.1's own justification for
40 — "tight enough that a genuinely wrong flat colour still fails" — is true only for colours
further apart than 40, which the entire slate/zinc/neutral range that every dark UI is built
from is not.

The measurement that actually found it was four lines of Python reading one scanline out of
each PNG and printing where the row background started and stopped: `x=8..68` native versus
`x=8..191` web. That is the §7.14 lesson recurring — when the instrument says clean and the
picture looks wrong, measure the picture.

**Mitigation, not a fix:** `diff-all` now reports a second percentage at
`STRICT_CHANNEL_TOLERANCE` (12) alongside the existing one. One threshold cannot answer both
"is this the same picture, ignoring how two rasterizers draw a glyph edge" and "is anything
structurally missing" — the looseness that buys the first is exactly what costs the second —
so both numbers are printed rather than trading one blindness for the other. A real per-fixture
pass/fail gate (§9.3) still doesn't exist and is still the right place to decide thresholds
deliberately.

**What the strict column immediately surfaced, on its first run: the two backends do not agree
on what Tailwind's colours are.** Sampling the `stack` fixture's flat indigo body:

```
bg-indigo-500   native (99, 102, 241)   web (97, 95, 255)   delta (2, 7, 14)
bg-slate-950    native  (2,   6,  23)   web  (2,  6,  24)   delta (0, 0,  1)
```

`#6366f1` is Tailwind **v3**'s `indigo-500`, and `twcore`'s palette is generated from
`tailwindcss@3.4.17` (phase 0's spike 3 README says so outright). Phase 1 vendored
`@tailwindcss/browser` **4.x** for the WebView backend, and v4 re-authored the entire default
palette in OKLCH — different values, not a different encoding of the same ones. Greys survived
almost intact (1 unit); saturated hues did not.

**Status: fixed — see §9.6 for the regeneration, the before/after strict numbers, and the
two hardcoded-hex divergences it flushed out along the way.**

This is a **palette-wide, backend-wide divergence that every conformance run to date has
reported as clean**, because 14 < 40. It is materially bigger in scope than §9.4's font-family
gap: it affects every widget in every app that uses a colour. The fix is contained and
mechanical — regenerate `phase_0/spike3_twcore/src/palette.rs` from v4's own palette, the same
way phase 0 generated it from v3's — but it edits a phase 0 spike crate and wants its own pass,
so it is recorded here rather than done inside §11's session. Concretely measured, one hue, one
fixture; the mechanism (v3 hex table vs. v4 OKLCH table) predicts the rest and has not been
exhaustively measured.

### 9.6 §9.5 resolved — the palette regenerated from Tailwind v4, and what the strict column then showed

**Status: fixed. `twcore`'s palette is now generated from the v4 OKLCH values, and the
strict conformance column drops from as high as 86.83% to under 1% on every fixture whose
content is mostly flat colour.**

**Source of truth changed, deliberately.** The obvious fix — re-point `gen_palette.py` at
`tailwindcss@4.x` on npm — would have fixed today's divergence and left the same failure
mode in place for tomorrow's, since the WebView backend renders whatever
`phase_1/webruntime/vendor/tailwind-browser.js` says and nothing tied the two together.
The script now reads that vendored bundle directly (`--color-<hue>-<shade>: oklch(...)`,
286 declarations, 26 hues x 11 shades). The two backends now agree *by construction*: they
cannot drift without someone re-vendoring that file, which is exactly the moment to re-run
the generator. It also drops the script's network dependency entirely.

**OKLCH -> sRGB, checked against a browser rather than assumed.** v4 states colours as
`oklch(L% C H)`, so the generator does the conversion: OKLab matrices, the sRGB transfer
function, and — the part with a real decision in it — **clipping** out-of-gamut channels
per channel rather than gamut-mapping them. `indigo-500`'s chroma pushes blue past 1.0, and
clipping is what reproduces Chromium's own answer. Both colours §9.5 sampled out of a real
screenshot come back exact:

```
bg-indigo-500   generated (97, 95, 255)   measured in Chromium (97, 95, 255)
bg-slate-950    generated  (2,  6,  24)   measured in Chromium  (2,  6,  24)
```

`tests/palette.rs` pins those two specifically, and says in the test name that they are
Chromium's rendering rather than the Tailwind docs — they are the only two values in that
file that verify the *conversion* rather than the table.

**The strict column before and after, same fixtures, same run script:**

| fixture | strict, v3 palette | strict, v4 palette |
|---|---|---|
| stack | **86.83%** | **0.21%** |
| column | 48.79% | 0.46% |
| grid | 44.19% | 0.25% |
| button | 37.09% | 9.34% |
| row | 30.64% | 0.90% |
| list | 4.43% | 4.28% |
| scrollview | 3.42% | 3.29% |
| textfield | 14.04% | 14.05% |
| text | 14.08% | 14.08% |
| icon / image / slider / switch | 1.37 / 0.26 / 1.41 / 0.86% | 1.37 / 0.26 / 1.43 / 0.90% |

The **loose** column is unchanged to two decimal places across the board — which is the
whole point of §9.5 and worth restating: a palette-wide colour error on every widget in
every app moved the number this project had been reading by *nothing*. Only the strict
column could see it, and only after §9.5 added it.

**What the strict column says now that colour is no longer drowning it.** The fixtures that
are mostly flat colour are now genuinely clean at tolerance 12, which they never were. The
ones that did not move — `text` (14.08%), `textfield` (14.05%), `button` (9.34%) — did not
move *because they were never colour*: they are the font-family gap §9.4 surfaced, still
open and now clearly the single largest remaining source of conformance divergence. `icon`
and `image` are unchanged for the reason §9.2 item 4 already gives (missing vector/bitmap
content, not colour). That is a better-separated picture than this harness has ever had.

**Ancillary fixes the regeneration forced, each a real divergence in its own right:**

- **`phase_1/webruntime/index.html`'s hand-written Switch/Slider CSS hardcoded v3 hex.**
  §9.4 aligned that CSS to `widgets.py`'s Tailwind tokens by writing the *literal v3 values*
  (`#6366f1` indigo-500, `#334155` slate-700) into the stylesheet. Those became wrong the
  moment native moved to v4 — the same class of "two hand-authored copies of one design"
  problem §9.2 item 3 found in the first place, one layer down. Updated to v4's values;
  `webruntime`'s own e2e suite still passes 14/14.
- **`twcore`'s default ring colour** (`decl.rs`) was v3's `blue-500` (`#3b82f6`) as a bare
  literal. Now v4's (`#2b7fff`), with a comment saying it has to track the palette.

**Known gaps.**

- **v4 adds four hues and removes none** — `mauve`, `mist`, `olive`, `taupe` are new;
  all 22 v3 hue names still exist, asserted by `every_v3_hue_still_exists_so_no_existing_app_loses_a_colour`
  so this regeneration cannot silently cost an app a colour it already uses.
- **Only the default palette was regenerated.** v4 also changed defaults that are not
  colours (spacing scale derivation, default border colour, `shadow-*` values). Nothing here
  checked those, and the conformance harness cannot currently tell a wrong default apart
  from a wrong utility.

  > **This entry was right, and it took until §28 for anyone to check.** The default
  > border colour, the default ring width *and* colour, and the whole `shadow-*` scale
  > were all still v3's — invisible the entire time because nothing painted any of
  > them. Corrected and measured against real Chromium in §28.1. **The spacing scale
  > derivation named above is still unchecked.** Note also that this section's own ring
  > fix (below) corrected v3's blue-500 hex to v4's, while v4 does not default a ring
  > to blue-500 at all — fixing the value and leaving the rule is its own failure mode.
- **The conversion is verified against two colours, not 286.** Both matched exactly, and the
  mechanism is uniform, so the rest are expected to follow — but "expected to follow" is
  what §9.5 said about the v3/v4 delta before it was measured. Sampling more hues out of the
  existing fixture PNGs would be cheap and has not been done.
- **Nothing re-verified on the device.** The palette is compiled into `twcore`, so every
  native colour on hardware changes with it; this session's conformance evidence is entirely
  host-side. Low risk (it is a data table, and the native renderer's paint path is unchanged)
  but by this document's own standard, a different claim.

### 9.7 §9.4 resolved — font-family, plus a second gap of exactly the same shape hiding behind it

**Status: fixed and verified live on the Moto G54. Native resolves `font-sans`/
`font-serif`/`font-mono`, both backends render the same typefaces, and `line-height`
— which turned out to have the identical bug — is applied too.**

**The bug, restated precisely.** §9.4 called this "no font-*family* resolution at
all," which was right but undersold *where* it was missing. `font-serif` parsed in
`twcore`, survived `resolve_inherited`'s inheritance walk, and arrived correctly in
`TextStyle::family`. Then `layout()`'s measure closure pushed `FontSize` and
`FontWeight` and nothing else, so parley fell back to its own default
(`FontFamily::Source("sans-serif")`) on every shaping pass. The whole pipeline worked
except the last line.

`Renderer::new` now takes a `FontSet` (sans required, serif/mono optional) instead of
one family's two faces, registers each under its own `GenericFamily`, and both
shaping sites push a family. `shape_isolated` matters as much as the measure pass and
is easy to overlook: it backs caret geometry and tap-to-offset hit-testing (§7.7), so
shaping a different typeface there than the one painted would put every tap at the
wrong byte offset in a non-default-family field — invisible in a screenshot, only
reproducible under a finger.

**The second gap, found by following the numbers.** With family fixed, `textfield`
still showed 1.89% loose against **13.40% strict** — the widest split of any fixture,
and exactly what §9.5 built the strict column to surface. Sampling the pixels
(§9.5's own method, four lines of Python) found 1200 pixels where web painted
`slate-800` (29,41,61) and native painted `slate-950` (2,6,24) — max delta 37, three
under the loose tolerance of 40. Not a missing background: 1200 px over a ~204 px-wide
field is about six rows, i.e. **native's text box was six pixels shorter than web's**.

Cause: `line_height` had the *same* bug as family — parsed, inherited, tracked in
`TouchedText`, never pushed to parley. Every native text node was sized by the font's
own metrics while the WebView sized it by Tailwind's `text-base`/`leading-*`. One more
`push_default` at both sites, with `Relative` mapping to parley's `FontSizeRelative`
(CSS's unitless `line-height`) and **not** `MetricsRelative`, which is parley's
default and is exactly what the bug looked like.

**Both backends now render the same typefaces, which is what makes any of this
measurable.** §9.4 had already established that native shaped DejaVu Sans while
Chromium resolved `sans-serif` to Noto Sans on this host, and that forcing the page to
DejaVu took `text` from 13.41% to 5.00% — then left it, because it is not a bug either
backend can fix alone. It is fixed by both reading the same files:
`phase_1/webruntime/scripts/stage_fonts.sh` materialises six faces into one directory,
`index.html` loads them via `@font-face` and points Tailwind's own `--font-*` theme
variables at them, and both `conformance` and `gpu_harness` read that same directory.
The two backends can no longer drift without someone re-vendoring the fonts.

**Conformance, across this session's three fixes:**

| fixture | start (loose / strict) | after palette (§9.6) | after family + line-height |
|---|---|---|---|
| button | 8.29 / 37.09 | 8.28 / 9.34 | **1.78 / 3.20** |
| text | 13.41 / 14.08 | 13.41 / 14.08 | **5.00 / 6.58** |
| textfield | 3.36 / 14.04 | 3.39 / 14.05 | **1.89 / 4.07** |
| stack | 0.07 / 86.83 | 0.04 / 0.21 | 0.04 / 0.21 |
| column | 0.20 / 48.79 | 0.22 / 0.46 | 0.22 / 0.46 |
| grid | 0.03 / 44.19 | 0.03 / 0.25 | 0.03 / 0.25 |
| row | 0.40 / 30.64 | 0.40 / 0.91 | 0.40 / 0.91 |
| list | 3.59 / 4.43 | 3.61 / 4.28 | 2.99 / 4.16 |
| scrollview | 2.77 / 3.42 | 2.78 / 3.29 | 2.28 / 3.21 |
| icon / image / slider / switch | unchanged throughout | | 1.22 / 0.15 / 1.28 / 0.17 |

Nothing is above 5% loose or 6.6% strict any more, against a starting spread that
reached 13.41% loose and 86.83% strict.

**A new `typography` fixture** exercises all three families plus `leading-loose`
together. It scores 9.95% — the highest of any fixture — and that number is *not* a
divergence: measuring the vertical position of each rendered text band in both PNGs
gives web `(13, 40, 69, 101)` and native `(13, 40, 69, 101)`, identical to the pixel
including the `leading-loose` line, with only glyph edges differing. Four lines of
text in a 240x140 box is simply the highest text-to-area ratio of any fixture, and
glyph-edge AA between two different rasterizers is the irreducible floor §9.1
describes. Recorded here because the number alone reads like a regression and is not
one — and because checking that took the same "measure the picture" move §9.5 and
§7.14 both had to make.

**Verification, live on the Moto G54 (arm64-v8a, Android 15).**
`demo_app/typography.py` renders the three families in regular and bold plus a
`leading-none`/`leading-loose` pair. On device: three visibly distinct typefaces, each
with a genuinely heavier bold, and clearly different line spacing between the two
leading columns. Zero crash markers in a full logcat sweep, process alive throughout.
`Native.boot` now takes a font *directory* rather than two file paths (7 parameters
instead of 8, where six separate paths would have made it 12), unpacked from a single
`fonts.zip` asset.

**Known gaps.**

- **Which typefaces ZKite should bundle is not settled.** DejaVu is what this repo
  already had on hand, not a decision. The real options are bundling a chosen family
  (what Flutter does — consistent everywhere, ~2.8 MB, non-native-looking on Android)
  or reading the device's own `/system/fonts` (native-looking, free, but divergent
  across OEMs and unavailable to the host conformance harness). This work makes either
  reachable and picks neither; it is a product decision, flagged rather than made.
- **`twcore` cannot express an arbitrary family.** `FontFamily` has exactly three
  variants and nothing parses `font-[...]`, so `font-['Inter']` is silently unknown.
  Bounded and known, not newly introduced here.
- **The rest of the never-pushed properties were audited in one pass, not one at a
  time** — the pattern was unmistakable once line-height made it two. `letter_spacing`
  (converted em -> px, since twcore stores Tailwind's em and parley wants font-size
  units), `tabular_nums` (the real `tnum` OpenType feature — plan1.md §8's own counter
  app asks for it so a ticking number doesn't jitter its own width), `italic`, and
  `text-align` are now all pushed, from one shared function so the measure pass and
  `shape_isolated` cannot drift apart again. Two host tests cover the two with
  observable geometry: letter-spacing asserts the *magnitude* of the added advance,
  not merely "wider", because a px/em mix-up would still be wider and would pass a
  naive comparison; tabular-nums asserts two same-length digit strings measure
  identically.
- **`italic` is pushed but has no italic face to select.** Only regular and bold are
  registered per family, so it currently resolves to upright — exactly how `font-bold`
  behaved before §9.2 item 1 registered a second face. The push is deliberate: adding
  the faces later is then sufficient on its own.
- **`text-align` applies only when the text box's width is externally fixed.** parley
  aligns against the `max_advance` handed to `break_all_lines`, which equals the node's
  own final width only in that case; when the box is content-sized, that value is the
  space *available* rather than the box, and aligning against it would push glyphs
  outside the node's own rect. A content-sized single line has nothing to align within
  anyway — the real uncovered case is content-sized text that wraps to several lines,
  where the shorter lines should align within the longest. Not attempted; it needs a
  second break pass or a width-aware re-align.
- **`decoration` (underline / line-through) is still unpushed, deliberately.** parley
  has the properties, but `GlyphRun` carries only font/size/glyphs, so neither paint
  backend has anything to draw a decoration line *from*. Pushing it would change
  nothing visible while looking like it had been handled — worse than leaving it named
  here. It needs paint support in both backends, not a fifth `push_default`.
- **The WebView side is verified in Chromium, not on a device.** `e2e.mjs` (17/17,
  three new font checks) proves the `@font-face`/`@theme` wiring in real Chromium, but
  no Dev Shell or `zkite build` app with bundled fonts was installed on the phone, and
  neither `devshell` nor `appshell` stages `fonts.zip` yet — only `gpu_harness` does.
  So on a device today the WebView backend still falls back to Roboto.
  **Update (§14.4): confirmed true of the shipped path by inspecting a real built
  APK, and the mechanism is worse than "not implemented" — `stage_webruntime` copies
  `webruntime/` wholesale, so it shipped fonts or didn't depending on whether
  `stage_fonts.sh` had ever run on the build machine, silently either way. `zkite
  build apk` now stages them and says so (or warns that it couldn't); `devshell`'s
  own `build_apk.sh` still has the silent version.**

## 10. Directory layout

```
phase_3/
├── PLAN.md                        this file
├── renderer/                       §3/§4 — shared layout + text-measurement engine (zkite_renderer)
│   ├── Cargo.toml
│   └── src/lib.rs
├── spike1_render_pipeline/        §3 — host-only software (vello_cpu) render pipeline, passing
│   ├── Cargo.toml
│   └── src/main.rs
├── pybridge/                       §5's intro/§8 — bridges zkite_py's to_tree() JSON into
│   ├── Cargo.toml                      zkite_renderer (build/RawNode, host-testable, no Android)
│   └── src/{lib.rs, embed.rs}          and the CPython embedding gpu_harness's render thread uses
├── conformance/                    §9 — per-widget conformance vs. the WebView backend, host-only
│   ├── Cargo.toml                      (real Chromium via Playwright + zkite_renderer/vello_cpu)
│   ├── fixtures.py                 one fixture per phase-1 widget -> manifest.json (html + tree)
│   ├── screenshot_web.mjs          manifest.json -> out/<name>_web.png (Playwright)
│   ├── src/main.rs                 manifest.json -> out/<name>_native.png ("render-all"),
│   │                                   out/<name>_diff.png + report.json ("diff-all")
│   └── run.sh                      the whole pipeline, one command — see §9.1
├── repro_atlas_lifetime/           §7.14 — host-only *headless GPU* harness (real wgpu/vello_
│   ├── Cargo.toml                      hybrid, render-to-texture, no window/device needed). Built
│   └── src/main.rs                     to test one §7.6 hypothesis (that reusing a Resources across
│                                        a rebuilt Renderer blanks cached glyphs); that came back
│                                        negative, so it is now the regression guard for that reuse
│                                        being safe — see its own module doc
├── repro_7_6/                      §7.6 update — host-only reproduction attempt for the still-
│   ├── Cargo.toml                      unresolved "text vanishes on TextField focus" bug, via the
│   └── src/main.rs                     real vello_cpu paint pipeline (not just layout metadata);
│                                        result was negative — see its own module doc for what that
│                                        rules out and what's left, un-host-testable, after it
└── gpu_harness/                    §4/§7 — interactive Android GPU render harness, verified live
    │                                    on the emulator and real hardware (dev.zkite.gpuharness)
    ├── Cargo.toml
    ├── build.rs                    §7.1 — compiles vendor/gametextinput's C++ via the `cc` crate
    ├── vendor/
    │   ├── gametextinput/          §7.1 — vendored outright (small, Apache-2.0): native source +
    │   │                                  classes.jar, fetched from dl.google.com, not built here
    │   └── androidx-core/          §7.1 — fetched, not committed (see fetch_androidx_core.sh);
    │                                      classes.jar on javac's classpath only
    ├── src/{lib.rs, render_thread.rs, text_input.rs, a11y.rs}
    │                                ├ text_input.rs: §7.2 — the one `GameTextInput` handle,
    │                                │ UI-thread-confined; see its module doc
    │                                └ a11y.rs: §8 — accesskit_android::InjectingAdapter,
    │                                  render-thread-confined (the opposite of text_input.rs —
    │                                  see §8.1 for why that's safe here); no new Java, no new
    │                                  androidx dependency, no new JNI entry point in lib.rs
    ├── demo_app/                    the bundled Python apps (ZKITE_APP_MAIN picks one)
    │   ├── main.py                  §4/§7 — the full-feature demo (counter, Switch, Slider,
    │   │                                ScrollView, two TextFields)
    │   ├── list_plain.py            §11 — 10,000 rows through a plain ScrollView: the
    │   │                                non-virtualised control, kept so the comparison is
    │   │                                re-runnable rather than a number in a table
    │   ├── list_virtualised.py      §11 — the same 10,000 rows through `List`
    │   └── nav.py                   §16 — three screens over a real back stack; each has
    │                                    a distinct background colour so which one is on
    │                                    top is a screenshot histogram lookup rather than
    │                                    OCR, and the depth + a pushed param are rendered
    ├── java/dev/zkite/gpuharness/{ZKiteActivity,ZKiteSurfaceView,Native}.java
    ├── java/androidx/core/R.java   §7.3 item 4 — hand-written R$id stub, see its own doc comment
    ├── app/{AndroidManifest.xml, res/}
    ├── artifacts/                  §14 — the release-pipeline output `zkite build apk
    │                                   --backend native` stages verbatim: classes.dex,
    │                                   fonts.zip, and per-ABI libzkitegpuharness.so +
    │                                   libpython3.14.so + libc++_shared.so. Same bucket
    │                                   and same contract as phase_2/appshell/artifacts/.
    └── scripts/{build.sh, build_dex.sh, build_apk.sh, fetch_androidx_core.sh}
                                    build_dex.sh (§14.3) is the one authority for the
                                    javac/d8 classpath; build_apk.sh calls it
```

`gpu_harness` follows the same shape as `phase_1/devshell`/`phase_2/appshell` deliberately (§1) —
a plain `Activity`, JNI surface, `javac`+`d8`+`cargo ndk`+`spike2_pack_apk` build pipeline — so that
`phase_3/pybridge`, which wires this to `zkite_py`'s real widget tree (§5's intro), extends a third
sibling shell instead of learning a new architecture.

## 11. Virtualised lists

§5 item 3, and plan1.md §10's own phase-3 sentence ("scrolling and **virtualised lists**").
Also the last widget missing from plan1.md §8's v1 set: `List` (virtualised) is named there,
and phase 1 shipped 12 of the 15 widgets while deliberately deferring this one — *"needs
virtualization to be honest about performance; building a non-virtualized stand-in would teach
a bad habit"* (`phase_1/PLAN.md` §3). This is that widget, built once there was a native
renderer to need it.

**Status: built, verified live on the Moto G54 5G with a 10,000-row list, and measured against
a like-for-like non-virtualised control on the same device.** Works on both backends. Four real
bugs found along the way, one of them only findable on a device (§11.4), plus a standing
platform gap the device run exposed (§11.6).

### 11.1 Why this is the item that had to be architectural

Everything else on phase 3's remaining list is additive and local. This one is not: a `List`
cannot decide which rows to build without knowing where it is scrolled to, and scroll position
lived entirely in the renderer. So this is the one place where state flows **backwards**, from
the rendered frame into the widget tree, and every layer had to learn about it — `zkite_py`,
`runtime.py`'s glue, `pybridge`, the render thread, and the WebView runtime.

It also attacks a wall this phase already hit once. §6.5's input-backlog bug was per-frame cost
scaling with total row count; coalescing hid the symptom at 60 rows without changing the
scaling. Before this section, **every** layer was O(total rows): Python built every widget, the
JSON carried every node, taffy laid out all of them, `layout()` reshaped every text leaf
through parley on every frame, and `positioned()` cloned every glyph run — with `hit_test`
calling `positioned()` a second time.

### 11.2 Design: the renderer never learns that a list is virtualised

`List("flex-1", count=10_000, item_height=56, builder=lambda i: Row(...))` emits only the rows
in view plus `overscan` either side, and collapses everything above and below into two
fixed-height spacer nodes.

The spacers are the whole trick. They keep the container's content height at exactly
`count * item_height` regardless of which window is materialised, which makes windowing
**invisible to the layout engine**: scroll position, `max_scroll_y`, the gesture code, the
fling physics, and every row's on-screen y-coordinate are identical to a non-virtualised list.
Nothing in `zkite_renderer` or `render_thread.rs` knows what a `List` is. That invariant is
asserted three ways — a host test over the pure window math, a `zkite_pybridge` test that lays
out two completely different windows through the real renderer and asserts `max_scroll_y` is
unchanged, and, on the device, `max_scroll_y 557837` logged identically across every fling
while the window slid by hundreds of rows (that number is exactly `10000 × 56` minus the
viewport).

It is also why the feedback loop terminates: a list's content height doesn't depend on its
window, so reporting a viewport can change the tree but cannot change that viewport.

Two deliberate constraints fall out, both enforced rather than documented-and-hoped:

- **`item_height` is a contract.** Each row is wrapped in a fixed-height `overflow-hidden` box,
  so a row that renders taller is clipped rather than silently desynchronising the spacer math
  from reality. Variable row heights are a real gap (§11.7) — the same first cut Flutter's
  `itemExtent` and react-window's `itemSize` make.
- **`gap-*` is refused at construction.** A non-virtualised list of N rows has N-1 gaps; a
  windowed one has W+1. The container's real scrollable height would stop matching
  `count * item_height`, and scroll position, `max_scroll_y`, and the window computed from them
  would all drift apart subtly and simultaneously. `List` raises a `ValueError` naming the
  offending class and telling the caller to put the spacing inside `item_height`.

### 11.3 The feedback channel, and what it costs

`_viewport.py` holds `node_id -> (scroll_y, viewport_height)`, reported by whichever backend is
running. Deliberately a module-level dict rather than something on `RenderContext`, which is
recreated on every render.

The load-bearing decision is **who decides whether a scroll needs a re-render**. Reporting is
per frame; re-rendering must not be. `runtime.py`'s `_zkite_scroll_tree` records the viewport
and then asks `window_would_change(node_id)` — pure integer arithmetic against the window that
was last actually materialised — returning a fresh tree only if the answer is yes, and `None`
otherwise. So a plain `ScrollView`, or a `List` scrolled less than one row, costs two dict
lookups and no user code at all. `view()` and the row `builder` run only when a window really
moves. Running user callbacks 60 times a second to discover that nothing changed would be a
genuine footgun, not just waste.

Native side: `redraw()` lays out, calls `sync_viewports` (reporting every scrollable container
uniformly — this side can't tell virtualised ones apart without duplicating `List`'s
bookkeeping, and Python answers cheaply), and lays out a second time if that produced a new
tree. Bounded at two passes by `MAX_VIEWPORT_SYNC_PASSES`, which the §11.2 invariant makes
sufficient; exceeding it logs rather than spins. A `reported_viewports` cache means a redraw
that moved nothing — a tap, a focus change, an IME pan — makes no Python call at all.

WebView side: the same two numbers, reported through the **existing** `dispatch` bridge under a
reserved `"scroll"` event name rather than a new `ZKiteNative` method, so no Java or JNI in
either shipped shell had to change. Coalesced to one report per animation frame, the DOM-side
equivalent of §6.5's `TouchMove` coalescing. Verified in real Chromium
(`phase_1/webruntime/test/e2e.mjs`, now 14/14): the spacers make a 25-row DOM report
`scrollHeight === 50000` exactly as a 1000-row one would, the viewport is reported straight
after render without waiting for a user scroll, scrolling reports the new position, and 20
scroll events fired in one frame produce exactly one report.

`redraw()`'s doc comment used to be able to say "never re-invokes Python," and that was the
reason scrolling was cheap. It can't any more, and the comment now says so. What survives is
the part that mattered: Python is *asked* once per scroll frame but only *re-runs `view()`*
when a window actually moved.

### 11.4 Four real bugs, and where each was caught

1. **A negative `scroll_y` collapsed the window to nothing** — caught by a host test, not a
   device. The first draft clamped `last` into range independently of `first`, so a negative
   scroll (which a rubber-band overscroll, §5 item 4, would produce) computed an empty window
   and rendered a blank list at the very top. Fixed by clamping `first` *before* deriving
   `last`, so the window keeps its size when clamping moves it.
2. **Every row rendered at a third of its width natively** — caught by the conformance
   suite's *images*, and emphatically not by its numbers (§9.5). The row wrapper is a plain
   `div`, which is `display:block` in a browser (so its single child fills the width for free)
   and `display:flex` **row** in taffy (so its single child sizes to content). This is the
   third instance in `widgets.py` of the same "no UA stylesheet" divergence that
   `Button.implied_classes` and `ScrollView.implied_classes` already document. Fixed by
   declaring `flex flex-col` on the wrapper.
3. **`import math` killed the app on the device** (§11.6) — every host test passed with it.
4. **The `zkite_pybridge` test suite had a latent race that only a second, different app could
   expose.** There is one CPython per process and one live app inside it (`runtime.py`'s `_app`
   global), both shared by tests `cargo test` runs on parallel threads. Two tests already
   booted an interpreter, but both wrote *byte-identical* counter-app source, so whichever won
   the race produced the answer both were asserting anyway. Adding a test that boots a
   different app surfaced it immediately. Fixed with a test-only mutex. Worth recording because
   "the tests pass" was, for those two, not evidence that they couldn't interfere.

### 11.5 Measured on the real device, against a real control

Two demo apps, `demo_app/list_plain.py` and `demo_app/list_virtualised.py` — same 10,000 rows,
same styling, same explicit 56px row height, differing only in `ScrollView` vs `List`. Both run
on the same Moto G54 5G (arm64-v8a, Android 15), GPU path (`wgpu`/`vello_hybrid`) in both.

| | plain `ScrollView` | virtualised `List` |
|---|---|---|
| Boot: render thread spawn → first frame | **6.34 s** | **1.48 s** |
| Nodes positioned per frame | **50,006** | **296** |
| First redraw | **3,779.5 ms** | **35.5 ms** |
| Redraw during scroll | **3,820 – 3,863 ms** | **avg 29.6 ms, max 38.0 ms** |
| `max_scroll_y` | — | 557,837, constant across every window change |
| Accessibility nodes for list rows | (would be 10,000) | 48 — the window only |

Roughly **130× per frame**, from unusable to interactive. Four swipes on the plain build
produced five consecutive ~3.85-second frames; the same four swipes on the virtualised build
produced sixty frames averaging 29.6 ms and a correctly-signed fling each.

**Said plainly, because the table shouldn't be read as a victory lap: 29.6 ms is about 34 fps,
still well above the 16 ms a 60 fps frame gets.** Virtualisation made a 10,000-row list usable;
it did not make it smooth. The remaining cost is not row count — it is that `Renderer::layout`
reshapes every text leaf through parley on *every* frame, so ~40 visible text nodes are
re-shaped 60 times a second even though neither their content nor their style has changed. A
shaped-run cache keyed by (content, style) is the obvious next perf item and is not built.

> **Update (§11.9): that cache is now built, and this paragraph's diagnosis was half
> right.** Re-shaping unchanged text really was the single biggest phase and caching it
> really did fix it (shaping 17.8 → 3.0 ms, frame 38.4 → 23.8 ms in a same-session A/B).
> But shaping was **46%** of the device frame, not nearly all of it — this paragraph
> generalised from a `zkite_renderer`-only measurement, where it genuinely is ~99% of that
> crate's cost, to the whole frame, which is also a CPython call, a tree build, an
> accessibility push and a paint. `redraw` now reports all seven phases. See §11.9.

Correctness on the device, not just speed: rows render contiguous and correctly numbered deep
into the list (screenshotted at rows 1025–1063), flings launch at ~19–24 K px/s and land
cleanly, `max_scroll_y` is exactly right and invariant, and a full logcat sweep for
`FATAL`/`AndroidRuntime` exceptions across the whole run came back at zero with the process
alive throughout — including after a `uiautomator dump`, so §8.7's SIGABRT guard still holds on
this path.

**A real instrumentation finding, since it cost a wasted measurement pass.** The redraw
counter's first version logged only an every-60-frames average. That reported *nothing at all*
for the plain build — because input coalescing (§6.5) collapses eight full swipes into fewer
than 60 redraws when each one takes four seconds, so the counter never reached its interval. A
ten-minute wait produced no line. An average is also exactly the wrong statistic here: one
400 ms stall inside sixty 8 ms frames reads as 14 ms. Both are fixed by also logging any single
redraw over `SLOW_REDRAW_THRESHOLD` (33 ms) immediately — which is what produced every number
in the table above, and is better permanent diagnostics regardless.

### 11.6 A standing gap the device run exposed: `zkite_py` cannot import extension modules

**Status: root-caused, fixed, and verified live on the Moto G54 — on both the native
harness *and* the shipped `zkite build apk` path. See §11.8 for the account.** The
investigation below is kept as written, because its central negative result (the `.so`
is in the APK, `linkExtensionModules` is character-for-character correct, and it still
doesn't import) is exactly right, and is what makes the actual answer obvious once the
missing measurement is taken.

`_viewport.py`'s first version used `math.ceil`/`math.floor`. Every host test passed. The first
real launch showed the error overlay:

```
File ".../zkite_framework/zkite/_viewport.py", line 28, in <module>
    import math
ModuleNotFoundError: No module named 'math'
```

`math` is an **extension** module, so it ships in `lib/<abi>/` rather than in
`python-stdlib.zip` — phase 0's Android-10 `dlopen` finding — and is reachable only through the
`.pth` file `ZKiteActivity.linkExtensionModules` writes for `site` to pick up at interpreter
startup. `math.cpython-314-aarch64-linux-android.so` **is** present in the built APK (67
extension modules are), and `gpu_harness`'s `linkExtensionModules` is character-for-character
the same as `devshell`'s. It still didn't import.

**Not root-caused.** The obvious suspect is that `site` isn't processing the `.pth` under
`PyConfig_InitIsolatedConfig`, but that is a hypothesis, not a finding — and it could not be
checked on the device, because the harness APK isn't `android:debuggable` so `run-as` refuses
to read the app's private directory. What is established: no extension module has ever been
exercised by any phase-1/2/3 shell (`zkite_py`'s entire stdlib footprint is pure-Python:
`dataclasses`, `html`, `importlib`, `inspect`, `json`, `sys`, `traceback`, `types`, `pathlib`,
`typing`), so this has been latent since phase 1 and nothing had ever tripped it.

Worked around rather than fixed: `_viewport.py` now uses floor division and imports nothing at
all. That is the right call for framework-core code on the render hot path regardless of the
underlying bug — `zkite` is imported before any app code runs, so it is the worst possible place
to depend on something this fragile.

**The gap itself is real, open, and bigger than this section.** As it stands, a user app whose
`view()` does `import math` — or `random`, or `datetime`, or anything else backed by a C
extension — would hit exactly this, and the failure mode is an error overlay at boot rather
than anything that shows up in a build. That is a genuine "the native backend supports a
narrower stdlib than the WebView backends do" divergence, and it deserves its own pass:
reproduce it in `phase_1/devshell` (to establish whether it is harness-specific or shared),
build a debuggable APK so `run-as` can read the interpreter's real `sys.path`, and check
`site_import` against the isolated config.

### 11.7 Known gaps

- **Fixed row heights only.** `item_height` is required and enforced by clipping. Variable or
  measured row heights would need a real measurement pass and a height cache; not attempted.
- **Still not 60 fps** (§11.5). ~34 fps at 10,000 rows. The next lever is a shaped-text cache
  in `zkite_renderer`, not more windowing — row count is no longer what costs.
  **Built (§11.9): ~42 fps now, and still not 60.** The remaining frame is dominated by
  `present` (8.3 ms, GPU submit vs. swapchain blocking not yet separated) and `sync`
  (6.6–8.0 ms, the Python call in the next bullet), not by text.
- **A screen reader sees only the materialised window.** Measured, not assumed: 48 a11y nodes
  for rows 1020–1067 out of 10,000. This is what every virtualised list does (Android's own
  `RecyclerView` included) and it is *why* accessibility scroll actions matter — TalkBack's
  linear navigation will stop at the window edge until `ACTION_SCROLL_FORWARD`/`BACKWARD` are
  wired up, which is exactly §8.6's existing known gap, now with a concrete consequence.
- **Vertical only**, matching `max_scroll_y`/`is_scrollable_y`'s existing y-axis-only scope.
- **`twcore`'s two caches grow with scroll distance.** Both are keyed by class-string hash and
  unbounded (`phase_0/spike3_twcore/src/lib.rs`), and a `List` emits two spacer class strings
  per window (`h-[123456px]`). Bounded by `2 × count` distinct strings, not by pixels — so ~20 K
  entries worst case for a fully-traversed 10,000-row list, not a leak. Not observed to cause a
  problem in this session's testing and not measured for memory; `clear_caches()` already exists
  as the lever if it ever needs one.
- **One Python call per scroll frame**, even when the answer is "nothing changed". Measured only
  in aggregate (it is inside the 29.6 ms), never isolated. If the shaped-text cache above lands
  and frames get near 16 ms, this is the next thing worth measuring on its own.
  **Now isolated (§11.9): 6.6–8.0 ms per scroll frame**, the largest purely-CPU phase left
  and about a third of the frame. Still open, now with a number on it.
- **The `math`/extension-module gap is fixed** (§11.6/§11.8), not merely worked around.
  `_viewport.py`'s own floor-division rewrite is kept regardless — framework-core code on
  the render hot path shouldn't import anything it doesn't need — but that is now a style
  choice rather than a workaround for a broken platform.
- **Not tested against a hot reload.** `_viewport.py` deliberately does *not* clear on reload
  (scroll position surviving a reload is a phase-1 promise), and the self-correcting path is
  argued from the code rather than exercised — no `zkite dev` run against a `List` was done.
- **The WebView half is verified in Chromium, not on a device.** `phase_1/webruntime/test/
  e2e.mjs` drives the real vendored runtime through real Playwright Chromium (14/14, including
  four new `List` checks), and `_zkite_scroll_html` is exercised only by that path's stub bridge
  — no `zkite dev`/Dev Shell run and no `zkite build` app with a `List` in it was installed on the
  phone. Given §11.6 is exactly the class of bug that only appears on a device, and given the
  WebView shells share the same `runtime.py` this session edited, that gap is worth closing
  before the WebView backend is claimed to support `List`.
  **Closed, and it was hiding a real bug — see §12.5.3.** A `zkite build apk` app with a
  `List` was finally installed on a phone and the list never converged: 268 renders, DOM
  climbing 3,881 → 10,361 nodes, defeating virtualisation entirely. Cause was
  `#zkite-root { min-height: 100% }` letting the root grow, so the reported viewport grew
  with the window that had just been rendered — the one assumption §11.3 above says makes
  the feedback loop terminate. Fixed (`height: 100%`, matching what the native wrapper
  always did), verified on device, and pinned by two new e2e checks that use a real app's
  `flex-1` shape instead of the old fixture's hardcoded `h-[300px]`, which was bounded by
  construction and could never have caught it.
- **A tap on a virtualised row is host-tested only.** `test_list.py` asserts a row's handler
  registers under the id the tree actually carries and dispatches correctly, but the device
  demo's rows have no `on_tap`, so the full touch → hit-test → ancestor-walk → dispatch path was
  never run against a windowed row on hardware. Low risk (a windowed row is an ordinary node
  with an ordinary id by construction) but unverified, and this document's own standard is that
  those are different claims.

### 11.8 §11.6 solved — Android's installer silently drops every `lib/<abi>/` entry not named `lib*.so`

**Status: root-caused, fixed, and verified live on the Moto G54 on both backends —
the native `gpu_harness` path and the shipped `zkite build apk` path. §11.6's "a user
app doing `import random` hits this" is closed.**

**The measurement §11.6 never took.** Every fact §11.6 established was correct:
`math.cpython-314-aarch64-linux-android.so` really is in the APK, all 67 extension
modules really are staged, and `linkExtensionModules` really is character-for-character
identical to `devshell`'s. The one thing nobody had checked was whether the file was on
the *device*. It was not:

```
$ adb shell ls /data/app/~~w-eeIqu0syvQ1RmNg7dUcw==/dev.zkite.gpuharness-.../lib/arm64
libc++_shared.so
libzkitegpuharness.so
libpython3.14.so
```

**Three files out of the seventy the APK contains** — and the three that survived are
exactly the three already named `lib*.so`.

**Root cause.** Android's package installer extracts an APK's `lib/<abi>/` entries into
the app's `nativeLibraryDir`, but only those whose basename matches `lib*.so`. Every
other name is skipped: no build-time warning, no install-time warning, no runtime error
beyond an ordinary `ModuleNotFoundError` much later. CPython names extension modules
`<mod>.cpython-<ver>-<triple>.so`, so **all 67 were dropped at install time, on every
ZKite APK ever built, since phase 1.** The `.pth` file, the `site` hypothesis, and the
isolated-config question §11.6 flagged as the "obvious suspect" were all red herrings —
`site` was processing the `.pth` correctly the whole time, pointing at a directory that
genuinely contained no extension modules.

Upstream corroborates the rule rather than contradicting it: CPython's own Android
testbed (`phase_0/vendor/python-*/testbed/app/build.gradle.kts`) stages
`include("lib*_python.so")` into jniLibs, and the python.org prefix ships OpenSSL and
SQLite pre-renamed as `libssl_python.so`/`libcrypto_python.so`/`libsqlite3_python.so`
for exactly this reason.

**Fix, in three parts.**

1. **Rename on the way into the APK** — `<mod>.cpython-*.so` → `libpymod_<mod>.so`, in
   all three staging sites (`phase_3/gpu_harness/scripts/build_apk.sh`,
   `phase_1/devshell/scripts/build_apk.sh`, and `phase_1/cli/src/build_cmd.rs`'s
   `stage_native_libs`, which is the shipped one). Module names are `[A-Za-z0-9_]+`, so
   the mapping round-trips unambiguously.
2. **Map the names back at import time** — `runtime.py`'s `_ZKiteNativeExtensionFinder`,
   a plain `sys.meta_path` entry that scans `sys.path` for `libpymod_*.so` once and
   returns an `ExtensionFileLoader` spec for a match. It is **appended**, never
   prepended, so it can only ever answer for names nothing else could resolve. Living in
   `runtime.py` means all three shells get it from one file, exactly as the tree-mode
   glue functions already do. The rename is safe because `ExtensionFileLoader` resolves
   a module's init function from the *module name* (`PyInit_math`), never the file name.
3. **Stage the transitive dependencies** — `lib*_python.so` was never copied at all, so
   `_ssl`/`_hashlib`/`_sqlite3` would still have failed after (1) and (2), and failed
   with a dynamic-linker error rather than a `ModuleNotFoundError`, which would have read
   like a different bug entirely.

**A fourth thing the shipped path needed, found only because it was actually run.** With
(1)–(3) in place the harness worked and `zkite build apk` still reported `0 native
extension modules available`. Cause: `phase_2/bundler`'s generated manifest sets
`android:extractNativeLibs="false"` — correct and safe on its own terms (pack does store
`lib/**/*.so` uncompressed and 16 KiB-aligned, so the platform mmaps them straight out of
the APK), but it means *nothing is extracted at all*, so there is no filesystem path for
`_imp.create_dynamic` to `dlopen`. `System.loadLibrary` still works because the platform
loader understands APK-internal entries; CPython's importer takes a path, and there
isn't one. This is a genuine conflict — uncompressed-libs-in-APK and loadable C
extensions are mutually exclusive under this mechanism — resolved in favour of working
C extensions, with the tradeoff written into the manifest generator and its test rather
than left implicit. The unexplored third option (keep `"false"`, `dlopen` through
bionic's `<apk>!/lib/<abi>/<name>.so` path form) is recorded there too.

**Verification, live on the Moto G54 5G (arm64-v8a, Android 15).**

`demo_app/extension_modules.py` (new, selected via `ZKITE_APP_MAIN`) imports eight
extension-backed stdlib modules and renders values it *computes*, so a screenshot is
evidence the C code ran rather than that an import didn't raise. Native GPU backend:

```
py: zkite: 67 native extension modules available
math      sqrt(1764) = 42                        binascii  hexlify(b'zkite') = 6b697465
random    randint(0,999) = 374                   hashlib   sha256(b'zkite')[:12] = 8e8c7d42b456
datetime  epoch+1e9s = 2001-09-09                ssl       OpenSSL 3.5.7 9 Jun 2026
zlib      crc32(b'zkite') = 4243486356            sqlite3   select 6*7 -> 42
```

Every value matches the same expression evaluated on the host, digit for digit —
including the seeded `randint`, which is the Mersenne Twister in `libpymod__random.so`
actually executing. `ssl` and `sqlite3` are the two that prove part (3): real OpenSSL
loaded through `libssl_python.so`, and a real query executed through
`libsqlite3_python.so`.

Shipped path, separately: `zkite build apk` against `phase_2/fixtures/testapp` (whose
`main.py` now imports `math`/`random` deliberately, so the bundler's own fixture keeps
this honest) — `72 native libraries` staged, `72` extracted on device, `zkite: 67 native
extension modules available`, `sqrt(1764)=42 rand=374` rendered on screen, and the
counter still increments `0 → 2` on tap with the process alive and no crash markers in
a full logcat sweep.

**Host regression tests** (`phase_1/devshell/tests/test_extension_finder.py`, 4 cases):
that a `libpymod_*.so` is genuinely unimportable without the finder — the device's actual
failure, reproduced on the host so the next test can't pass for the wrong reason — that
it imports and runs with the finder installed, that the finder declines names it has no
file for, and that it adds nothing to `sys.meta_path` on a backend with nothing staged.
The finder source is lifted out of the shipping `runtime.py` at test time rather than
duplicated, so these cannot pass against a stale copy.

**The standing lesson, and it is the same one §7.14 already paid for (§11.8).** §11.6 had ruled
out the tree, the `.pth`, and the glue; it named the isolated-config `site_import`
hypothesis and said, correctly, that it could not be checked because the APK isn't
`android:debuggable` so `run-as` can't read the app's private directory. That blocker was
real but irrelevant — the file that mattered was in `/data/app`, which is world-readable,
and one `adb shell ls` would have answered it at any point in the last three phases. When
the code is provably right and the behaviour is provably wrong, the missing step is
usually a measurement of the artifact, not another hypothesis about the code.

**What this does not cover.** The `.pth`/`site` path is now known to work, but only
because something finally used it — no test asserts it, and a future change to
`config.home` or the isolated config could break it silently again. `zkite build aab` was
not rebuilt or re-validated through `bundletool` after the manifest change (the flag is
in the shared `android_manifest`, so it applies to both, but AAB was not re-run).
`phase_1/devshell`'s own `build_apk.sh` carries the identical staging fix. It has since been
rebuilt and installed on the device — the pack log confirms `staged 67 extension modules into
lib/arm64-v8a/ as libpymod_*.so`, and the bundled typefaces come along for free because
`build_apk.sh` already `cp -r`s the whole `webruntime/` directory (`assets/webruntime/fonts/
sans.ttf` and friends appear in the packed file list). **It was not launched or exercised**:
no `zkite dev` session was run against it, so "the Dev Shell can import `math`" is still an
inference from a shared `runtime.py`, not an observation.

### 11.9 The shaped-text cache — §11.5's named next lever, built, and what it revealed about the rest of the frame

**Status: built, verified live on the Moto G54 against a same-session A/B, and
host-tested. A 10,000-row list scroll frame goes from 38.4ms to 23.8ms (26 → 42fps).
The claim §11.5 made about *why* the frame was slow turns out to have been half right,
and the half it got wrong is the more useful half — see "what this corrects" below.**

**The lever.** §11.5: *"the remaining cost is not row count — it is that
`Renderer::layout` reshapes every text leaf through parley on every frame, so ~40
visible text nodes are re-shaped 60 times a second even though neither their content
nor their style has changed. A shaped-run cache keyed by (content, style) is the
obvious next perf item and is not built."* It is now built, in `zkite_renderer`, keyed
exactly that way.

**Design, and the one decision in it.** `ShapeKey` is `(content, TextStyle, wrap_width,
aligned)` and `ShapedText` is `(width, height, Arc<Vec<GlyphRun>>)`. Three things are
worth stating because each is a place this could have been subtly wrong:

- **The key holds the whole `TextStyle`, not the fields that currently reach parley.**
  `color` and `decoration` are ignored by every shaping call today, so including them
  costs a few redundant entries — and that is the deliberate direction to be wrong in.
  Over-keying wastes cache slots; under-keying returns another node's glyphs. More to
  the point, a `TextStyle` field added later is covered by the derived `PartialEq`
  automatically rather than needing to be remembered, which is precisely the failure
  mode §9.7 found *twice* (`family` and `line_height` were each parsed, inherited, and
  then read by nothing). `Hash` deliberately covers only a cheap subset — a hash map
  requires equal keys to hash equally, not unequal keys to differ, and `Eq` separates
  the collisions.
- **Everything parley sees now comes from one function.** `push_text_style` folds in
  the family/size/weight/line-height pushes that used to be written out at both shaping
  sites. That is what makes the cache's soundness *checkable*: "nothing outside the key
  reaches the shaper" is one function to read, not an audit of every call site. §9.7
  already worried in a comment about the measure pass and `shape_isolated` drifting
  apart; they can no longer drift at all.
- **Two rotated generations, not an LRU.** `layout()` moves current to previous and
  starts a fresh current; a lookup checks current, then previous (promoting a hit),
  then shapes. An entry survives as long as something asks for it and is dropped one
  rotation after it stops — no timestamps, no eviction sweep, and a working set bounded
  by two passes' worth of text rather than by scroll distance. **That bound is the
  point**: `twcore`'s own caches already grow with how far a list has been scrolled
  (§11.7), and a *glyph* cache doing the same would be a real leak on a 10,000-row
  list. Measured on the device: **303–339 entries**, flat, after scrolling through
  hundreds of distinct rows.

Rotating per `layout()` call is safe even though `redraw` can lay out twice per frame
(§11.3's viewport-sync pass): the second pass hits the first pass's entries out of
`previous` and promotes them.

**`shape_isolated` is deliberately not cached.** It backs caret geometry and
tap-to-offset (§7.7) and needs the live `parley::Layout` the summary throws away. It
runs once or twice per frame against one short string, against ~100 leaves for the
measure pass — it was never what §11.5 measured.

**Host measurement, which is what made the device result legible.** A tree shaped like
the real materialised window (48 rows × 2 text leaves + header, 198 nodes positioned):

| | before | after |
|---|---|---|
| full frame (clear+build+layout+positioned) | 3.250 ms | **0.342 ms** |
| ├ build (clear + twcore + taffy tree) | 0.049 ms | 0.049 ms |
| ├ **layout (taffy + parley)** | **3.225 ms** | **0.278 ms** |
| └ positioned (walk + clone) | 0.011 ms | 0.012 ms |

Two findings fell out of taking that measurement rather than assuming it. **Identical
strings cost exactly the same as changing ones before the cache** (3.250 vs 3.238 ms),
which is the direct confirmation that nothing was being reused. And **timing `layout()`
in a loop over one root measures nothing** — taffy caches per-node layout, so every call
after the first is a cache hit and reports ~0 for work a real frame always pays, since
a real frame rebuilds the tree and every node is new. Only build-vs-build+layout over
freshly built trees measures the real thing. An early version of this benchmark got
that wrong and reported layout as 0.061 ms, i.e. 2% of the frame instead of 99%.

**Device A/B, same session, same device, same thermal state, one build differing only
by a bypass in the cache lookup** — the technique §4 used for the CPU-fallback path and
§8.7 used for the accessibility abort, chosen over comparing against §11.5's recorded
number from a previous session:

| phase (avg over 60 scroll frames, 296 nodes) | cache bypassed | cache on |
|---|---|---|
| build | 1.47 ms | 1.51 ms |
| **layout** | **17.81 ms** | **3.01 ms** |
| positioned | 0.22 ms | 0.25 ms |
| sync (the CPython viewport call) | 6.45 ms | 6.77 ms |
| a11y | 0.27 ms | 0.25 ms |
| scene (vello scene building) | 3.54 ms | 3.50 ms |
| present (submit + present) | 8.42 ms | 8.31 ms |
| **whole frame** | **38.43 ms** | **23.80 ms** |
| shape cache hit rate | 0.0% | **98.8%** |

Shaping drops **5.9×**; the frame drops **1.61×**, from ~26fps to ~42fps. Every other
phase is unchanged to within noise, which is the evidence that the cache did what it
claims and nothing else.

**What this corrects in §11.5.** That section said the remaining cost "is not row count
— it is that `Renderer::layout` reshapes every text leaf." The mechanism was right and
the fix worked. The *proportion* was wrong: shaping was **46%** of the device frame
(17.8 of 38.4 ms), not nearly all of it. The reason the error was easy to make is
instructive — measured against `zkite_renderer` alone on the host, shaping really is 99%
of that crate's per-frame cost, and it is true and useless, because a real frame is
also a CPython call, a JSON parse, a tree build, an accessibility push and a full paint.
**Attributing a frame by measuring the one component you happened to have a harness for
is how that goes wrong**, so `redraw` now measures all seven phases permanently rather
than leaving the next person to subtract.

**What the breakdown says to do next, now that it exists.** The two largest phases are
no longer ours-to-shape:

- **`present` 8.3 ms** — `get_current_texture` + submit + present. This one can *block*
  on the swapchain, so it is deliberately reported separately from `scene`: time here
  is not necessarily work done, and reading the two as one number would make vsync
  throttling look like slow painting. Whether it is genuinely GPU-bound or mostly
  waiting is unmeasured and is the first thing to establish.
- **`sync` 6.6–8.0 ms** — the per-scroll-frame CPython call reporting viewport position
  back to `zkite_py`. §11.7 predicted exactly this: *"One Python call per scroll frame,
  even when the answer is 'nothing changed' … If the shaped-text cache above lands and
  frames get near 16 ms, this is the next thing worth measuring on its own."* It has
  now been measured on its own, and it is the largest purely-CPU phase left.

`layout` at 3.0 ms and `scene` at 3.5 ms are now comparable to each other and to
`build` at 1.5 ms; none of them is the bottleneck any more.

**An honest loose end.** §11.5 recorded 29.6 ms for this app; today's *cache-bypassed*
build measures 38.4 ms. Those are different sessions and different builds, so the A/B
above — same session, one bypass — is the number to trust, but the gap is real and
unexplained. The one hypothesis cheap enough to test was that §9.7's font work (six
registered faces across three families, where there had previously been two faces in
one) made each shaping pass walk more candidates. **Tested on the host and refuted:**
0.378 ms per 48-row frame with two faces versus 0.365 ms with six — within noise, and
the six-face case nominally *faster*. Recorded as a negative result rather than left as
a plausible-sounding explanation. Device thermal/governor state and the accumulation of
other changes between sessions remain unexcluded.

**Host tests (`phase_3/renderer`, 14 → 18).** Three of them exist specifically because
a cache can be fast and wrong:

- `shape_cache_is_observationally_transparent` — a **cold** renderer's output compared
  against a **warm** one's, rect for rect and glyph id/x/y for glyph id/x/y, over a tree
  mixing size, weight, family, letter spacing, alignment, line height and wrapping, with
  one string repeated at two styles. Cold-vs-warm, not warm-vs-itself: the failure mode
  worth catching is a key that under-specifies what parley consumed, and that only shows
  up as output differing from a cold shape. A warm renderer agreeing with itself would
  pass with a constant key.
- `same_content_at_a_different_style_never_shares_a_cache_entry` — one pair per field
  that reaches parley, checked separately so a failure names which field fell out.
- `shape_cache_hits_across_a_rebuilt_tree_with_all_new_node_ids` — the cache must
  survive `clear()` and hit against a tree whose every `NodeId` is new, because that is
  the only situation a real frame presents. A cache keyed on `NodeId`, or cleared by
  `clear()`, would miss 100% of the time while passing every other test in the file.
- `shape_cache_working_set_is_bounded_by_the_window_not_by_strings_ever_seen` — 200
  distinct strings through a 20-row window must retain ~2 generations, not 200 entries.

All four fail when the cache is bypassed, which is how the A/B build was confirmed to
genuinely be a bypass rather than a no-op edit.

**A small real finding from a test that failed for the right reason.** The first draft
of the transparency test asserted a fresh `Renderer`'s first `layout()` had zero cache
hits. It had seven, for seven text leaves: **taffy calls the measure closure more than
once per node per pass** (min-content, max-content, then the definite size), so the
cache earns hits *within* a single layout as well as across frames. The assertion was
wrong, not the code; it now asserts what actually matters (the compared pass adds no
misses).

**Verification, live on the Moto G54 5G (arm64-v8a, Android 15).**

- **`list_virtualised.py`** (10,000 rows): rows render contiguous and correctly numbered
  deep into the list (screenshotted at rows 313–351), alternating backgrounds and both
  text columns intact, `max_scroll_y 557841` constant across every fling, cache hit rate
  98.2–98.9% with entries flat at 303–339.
- **`main.py`** (the full-feature demo): counter, Increment, Switch, Slider, 60-row
  ScrollView and both `TextField`s all render correctly; `uiautomator dump` reports the
  full accessibility tree with correct roles, labels, bounds and `password="true"`, and
  the surface still at `[0,117][1080,2340]` — §7.14's geometry and §8.7's abort guard
  both still hold.
- **The IME path specifically**, because `shape_isolated` was refactored and it backs
  caret geometry and tap-to-offset — the one class of bug that is "invisible in a
  screenshot and only reproducible under a finger" (§9.7). Tapping "Your name" focused
  it (`-> focusing text field r.4.0`), rendered a caret at the field's left edge, showed
  the keyboard and panned the field above it; two real taps on Gboard's own rendered
  keys produced `onTextInputEvent: decoded text="hi"`. Real key taps, not `adb shell
  input text`, per §7.4's finding that the latter doesn't exercise this path at all.
- **Zero crash markers** (`panic`/`FATAL`/`AndroidRuntime`/`ANR`) across every session
  above, process alive throughout.

**Conformance is unchanged to two decimal places on all 14 fixtures** — `button` 1.78 /
3.20, `text` 5.00 / 6.58, `textfield` 1.89 / 4.07, `typography` 9.95, and the rest
exactly as §9.7's table records them. That is the strongest available host-side evidence
that the cache is pixel-transparent through the real paint pipeline, not merely
geometrically equal.

**Known gaps.**

- **Still not 60 fps.** 23.8 ms is ~42 fps against the 16.7 ms a 60 fps frame gets.
  Better than the ~34 fps §11.5 measured and much better than the ~26 fps the same build
  gives without the cache, but the honest statement is unchanged in kind: a 10,000-row
  list is now comfortably interactive and still not smooth.
- **`present` is unattributed.** 8.3 ms, the largest single phase, and nothing
  distinguishes GPU work from blocking on the swapchain. Until that is split, "the GPU
  is the bottleneck" would be a guess.
- **The A/B is one app on one device.** `list_virtualised.py` on the Moto G54, GPU path.
  The cache's benefit scales with how much text is on screen and how little of it
  changes, so a text-light app would see less and a static text-heavy one more; neither
  was measured.
- **`shape_isolated` remains uncached** (see above) — deliberate, but it means a future
  feature that calls it per-node rather than per-focused-field would reintroduce
  uncached per-frame shaping through a second door.
- **The 29.6 → 38.4 ms discrepancy is unexplained**, with one hypothesis tested and
  refuted (above).
- **Nothing was re-verified on the WebView backend**, which does not use `zkite_renderer`
  at all and cannot be affected — stated only so the absence isn't read as an oversight.

## 12. The head-to-head backend comparison

**Status: built and run. §12.1–§12.4 below are the original specification, left as
written; §12.5 onward are the results.** §12.3's criteria in particular are *not*
edited — the entire point of stating them before the numbers existed was that the
conclusion could not be fitted to them afterwards, so they are quoted and applied in
§12.7 rather than revised.

**The headline is not a timing.** Running the comparison found two real bugs before it
produced a single frame time, and one of them means the two backends were not
rendering the same UI at all (§12.5). The timings are in §12.6 and the answer in
§12.7; the short version is that **the WebView backend scrolls a 10,000-row list at a
locked 60fps and the native backend manages ~39fps** — the opposite of what plan1.md
§5's qualitative table asserts — while native wins tap latency by ~5x.

Every number in this document is either native-vs-native (§11.5's virtualised list against
a plain one) or pixel conformance (§9). **No app has ever been measured on both backends.**
That is a conspicuous hole given what this phase is for: plan1.md §5 picks the custom
renderer over the WebView on the strength of a qualitative table — "web-in-a-box; scroll
and keyboard jank" vs. "Flutter-like: consistent, controllable", ceiling "Low" vs. "High" —
and phase 3's exit criterion is to make the native backend the default. Flipping that
switch on the basis of an unmeasured table would be the same mistake this document keeps
catching in smaller forms: a claim that everything is consistent with, and nothing has
tested.

The one axis that *has* been compared, incidentally rather than deliberately, already
complicates the story:

| | native (`gpu_harness`) | WebView (`devshell`) |
|---|---|---|
| APK, arm64-v8a | **38.4 MB** | 24.3 MB |
| renderer `.so` | **12.9 MB** (vello + taffy + parley + wgpu + accesskit) | 0.7 MB |

Native is ~14 MB larger and always will be, because a WebView is free — it ships in the
OS — while the native renderer carries its entire stack. That is a real, permanent cost on
the wrong side of the ledger, and it needs wins elsewhere to justify it. Whether those wins
exist is exactly what is unmeasured.

### 12.1 What has to be built before any of this can be measured

**Status: built — see §12.5.1 for what that turned out to be.**

Not a benchmark harness — **instrumentation parity**. The native side already logs per-redraw
timings and flags anything over `SLOW_REDRAW_THRESHOLD` (§11.5); the WebView side has no
equivalent at all, so today the two cannot be compared even in principle. The WebView
backend needs the same three numbers, emitted the same way: time from process start to
first painted frame, per-frame cost during a sustained scroll, and node/DOM count per frame.

One deliberate constraint, learned from §11.5's own wasted measurement pass: **log every
frame over a threshold immediately, never only a rolling average.** An average is the wrong
statistic here — one 400 ms stall inside sixty 8 ms frames reads as 14 ms — and an
every-N-frames average silently reports nothing at all when frames are slow enough that N is
never reached, which is exactly what happened the first time.

Both backends must run the **same `main.py`**, on the **same device**, back to back. The
apps below should live in `gpu_harness/demo_app/` and be bakeable into either shell.

### 12.2 The measurements

1. **Cold start to first pixel.** Process start → first painted frame, ten runs each,
   reported as median and worst case rather than a mean. Native pays CPython init + font
   registration + GPU/adapter probe; WebView pays CPython init + WebView process spawn +
   Blink parse + the Tailwind JIT compiling every class string in the tree at runtime. It is
   genuinely unclear which is worse, and the Tailwind-JIT-at-runtime cost is a WebView-only
   tax nobody has ever priced. plan1.md §9 sets the bar this feeds: "first pixel <100ms,
   interactive <500ms."
2. **Sustained scroll frame time, at three sizes.** 60 rows, 1,000 rows, and 10,000 rows,
   the last two through `List`. Native's 10k number is known (~30 ms, ~34 fps); the WebView's
   is not, and this is the comparison most likely to be decisive either way. Report the frame
   distribution, not just the mean — jank is a tail property, and "scroll jank" is the
   specific charge plan1.md §5 levels at the WebView backend.
3. **Time from tap to visible state change.** Native goes touch → hit-test → ancestor walk →
   Python dispatch → relayout → repaint. WebView goes touch → Blink hit-test → delegated JS
   listener → JNI → Python dispatch → HTML string → `morphdom` diff → Blink relayout. Both
   round-trip through the same CPython, so this isolates the backend's own overhead fairly.
4. **Memory, resident, after a fixed scripted interaction.** `dumpsys meminfo`, and read the
   *total across processes* — a WebView spawns a separate sandboxed renderer process, so a
   single-process reading will flatter it substantially. This is the axis where the native
   backend is most likely to win back some of its 14 MB of binary.
5. **Text-heavy re-render.** A screen of ~200 text nodes whose content changes every frame.
   This targets the specific weakness §11.5 identified — `Renderer::layout` shapes text
   through parley — against Blink's own text caching. Note what changed under this item
   after it was written: §11.9's shaped-text cache **is now built**, and it is precisely the
   case where *nothing* is reused, so this measures the cache's floor rather than its
   value. A scroll reuses rows and hits 98.8%; 200 strings that all change cannot hit at
   all. Both numbers are worth having, which is why item 2 and this item are separate.
6. **Battery/thermal over a sustained scroll**, if it can be made repeatable. Lower
   confidence than the rest; include only if the numbers turn out reproducible across runs,
   and say so plainly if they do not.

### 12.3 What would count as an answer

Deliberately stated before the numbers exist, so the conclusion cannot be fitted to them
afterwards:

- If native is **decisively better on frame time and memory**, the 14 MB is paid for and the
  switch should flip once the remaining gaps close.
- If native is **comparable but not better**, the case rests entirely on the qualitative
  claims (Tailwind fidelity, consistency across devices, the ceiling) — which is a defensible
  position, but a different one from what plan1.md §5 currently asserts, and the document
  should be corrected to say so.
- If native is **worse**, that is the single most valuable finding phase 3 could produce, and
  plan1.md §11's own advice ("consider dropping Phase 3 to stay on the WebView backend for
  1.0") becomes live again rather than hypothetical.

Any of those three is a good outcome. Not measuring is the only bad one.

### 12.4 Known difficulty

The comparison cannot be run at all until the native renderer is reachable with an arbitrary
app *and* the WebView side is instrumented — so this section is blocked behind the same
missing backend switch everything else is. Running it against `gpu_harness` (rebuild per
edit, hardcoded `ZKITE_APP_MAIN`) is possible and is the cheaper path; doing it properly, once
both shells can host either backend, is better and should be preferred if the switch lands
first.

*(That is what was done: the cheaper path. `gpu_harness` with `ZKITE_APP_MAIN` on one side,
`zkite build apk` — i.e. `phase_2/appshell` — on the other, running the same file. `appshell`
rather than `devshell` deliberately: the Dev Shell also runs a reload-socket thread and a
120ms `pollHtml` UI-thread timer, neither of which a shipped app has.)*

> **Update: the switch now exists — see §14**, so the "better" path this section
> defers to is available: both backends are reachable through `zkite build apk`,
> from one project, with no `ZKITE_APP_MAIN` and no per-edit rebuild of a harness.
> §14.1 also corrects this section's own premise — `ZKITE_APP_MAIN` never baked an app
> into the binary; the harness has been able to host an arbitrary app since §5.

### 12.5 What running it found before it produced a single timing

**Two real bugs, one in each backend, and the first of them means the two were never
rendering the same UI.** Both are exactly the class of thing this section existed to
catch — neither is visible to §9's conformance harness, and neither had been possible
to see, because no app had ever been run on both backends on one device.

**Test device, and it is not the usual one.** A Samsung SM-E066B (Galaxy A06), Android
16 / API 36, arm64-v8a, **720x1600 at 300dpi**, MediaTek MT6835 — so a **Mali** GPU,
a third vendor after the Moto G54's PowerVR (§4) and the emulator's SwiftShader. The
Moto G54 every other number in this document was measured on was not available; the
comparison was run here instead, on the reasoning that §12 is internally valid on any
single device as long as both backends are measured on it back to back, and that a
lower-end phone is the harder test rather than the easier one. **This means none of
§12's numbers are comparable to §11.9's**, which is called out again in §12.8. The GPU
path initialized and rendered correctly on Mali with no changes, which is itself a
small positive result for §4's portability.

#### 12.5.1 What "instrumentation parity" turned out to be

§12.1 asked for three numbers on the WebView side. What was built:

- **`phase_1/webruntime/zkite-runtime.js`** — time to first painted frame, a
  frame-interval sampler that runs only during a scroll, per-re-render cost, tap-to-
  paint, and DOM node count. Every one measured **to paint** via a double
  `requestAnimationFrame`, not to the end of our own JS: `morphdom` returning is not a
  frame, and a single rAF callback runs *before* paint, so measuring to it would have
  systematically flattered this backend against a native side that measures through
  `present`.
- **Output channel: the WebView's console, forwarded to logcat** by a
  `WebChromeClient` in both `appshell` and `devshell`. Chosen over a new `ZKiteNative`
  method for the reason §11.3 chose the existing `dispatch` bridge for scroll: no new
  bridge surface, no JNI change. It has an unplanned side benefit — **a JS error in the
  runtime is now visible on a device for the first time**; previously it went nowhere.
- **The cold-start anchor is split, not shared.** `ZKiteActivity` logs
  `boot-call process_rel=` (process start → the shell handing off, from
  `Process.getStartElapsedRealtime()`) and each backend logs its own time from there to
  its first painted frame. Neither side has to reason about the other's clock, and the
  split is the more useful shape anyway: it separates what a backend controls from what
  the platform charges every app regardless.
- **Native gained the two numbers it was missing**: a first-frame line, and **frame
  *interval* as distinct from frame *cost*** — see §12.6, where the gap between them
  turns out to be the most actionable native finding in this section.
- **Five `cmp_*.py` apps** (`phase_3/gpu_harness/demo_app/`), each plain `zkite_py`,
  each run byte-for-byte identically through both shells.
- **`phase_1/webruntime/test/e2e.mjs` 17 → 23 checks**, four covering the
  instrumentation and two covering §12.5.3's bug.

**A real finding about `am start -W`, worth recording because it would mislead anyone
who reached for it first.** It is the obvious tool for §12.2 item 1 and it is wrong for
this job on *both* backends: it reports 215ms for a native cold start whose first real
pixel is at 1267ms. The platform considers the activity "displayed" once its window is
first drawn, which for a `SurfaceView` or a `WebView` happens long before any app
content exists. It is still reported below, because it is identical machinery on both
sides and therefore a fair *relative* number — but it is not cold start, and anyone
using it alone would conclude ZKite starts in 200ms when it takes six times that.

#### 12.5.2 The two backends do not render at the same size — native ignores device density

**This is the largest finding in §12 and it is a correctness bug, not a performance
one.** `twcore` resolves Tailwind's `px-6` to 24, and the native backend hands that
straight to taffy against a surface measured in **device pixels**. The WebView backend
renders the same class into a page with `<meta name="viewport" content="width=device-
width, initial-scale=1">`, where 24 is 24 **CSS px** and Blink multiplies by the device
pixel ratio. So every native app renders at `1/dpr` the size of the same app on the
WebView backend.

Measured, not eyeballed — the same "measure the picture" move §9.5 and §7.14 both had
to make, here by finding the indigo button's bounding box in a real screenshot from
each backend on the same device:

```
native    button bbox x295..424 y842..889   ->  130 x 48 px
webview   button bbox x239..480 y916..1005  ->  242 x 90 px
```

242/130 = 1.862 and 90/48 = **1.875**, against this device's exact ratio of 300/160 =
**1.875**. (The width is a hair off because the rounded corners' anti-aliasing clips
the colour test at the extremes; the height, with flat edges, is exact.)

On the Moto G54 (420dpi, dpr 2.625) the same bug means native renders everything at
**38%** of the correct size. Every native screenshot in this document is affected.

**Why nothing caught it.** §9's conformance harness renders both backends into a fixed
pixel canvas at an implicit dpr of 1, so the two agree exactly and the harness is
right to report them clean — it is structurally incapable of seeing this. Every native
device screenshot was only ever compared against other native screenshots or against a
host PNG, never against the WebView backend on the same phone. That comparison is
precisely what this section is, and it is the first thing it found.

There is no density handling anywhere in `zkite_renderer`, `zkite_pybridge` or
`gpu_harness`'s layout path; the only `getScaled*` calls in the tree are
`ViewConfiguration`'s touch-slop constants, which are input, not layout.

**Consequence for everything else in §12, stated plainly: the frame times below compare
two backends drawing the same app at different sizes**, and it cuts both ways rather
than uniformly favouring one. Native's glyphs are smaller and its filled areas cover
less of the surface (cheaper); but because its viewport is `1/dpr` in *layout* units it
fits roughly twice as many list rows on screen, so it builds, shapes and positions
about twice the content (dearer) — 218 nodes against the WebView's 93 on the same
10,000-row app. Neither correction is a simple scalar, which is why §12.7 does not try
to apply one.

#### 12.5.3 The WebView backend's `List` never converges on a device

§11.7 named this exact gap and it turned out to be a real bug, not a formality:

> **The WebView half is verified in Chromium, not on a device.** ... no `zkite dev`/Dev
> Shell run and no `zkite build` app with a `List` in it was installed on the phone.
> ... that gap is worth closing before the WebView backend is claimed to support `List`.

Installed on the phone, `cmp_list_10k.py` on the WebView backend rendered **268 times
without stopping**, the DOM growing by exactly 36 nodes (12 rows) per render — 3,881 →
10,361 nodes — with each render taking 223ms and still climbing when the run was cut
short. Virtualisation was completely defeated: the list was walking toward
materialising all 10,000 rows.

**Root cause: `#zkite-root { min-height: 100% }`.** A minimum bounds the root from below
and leaves it free to grow above. For every widget that fits on screen — i.e. every app
this project had before `List` — that is indistinguishable from a definite height. For
a `flex-1` scroll container it is not: an unbounded parent means the container resolves
against its own content rather than the viewport, so the `List` reports a viewport as
tall as the rows it just rendered, `window_would_change()` says yes, Python widens the
window, and the list grows again.

This is precisely the assumption §11.3 relied on and named:

> It is also why the feedback loop terminates: a list's content height doesn't depend
> on its window, so reporting a viewport can change the tree but cannot change that
> viewport.

True on native, false on the WebView — because there the *viewport* depended on the
window. The native backend never had the bug because `zkite_pybridge::build`'s
equivalent wrapper is `w-full h-full`, a **definite** height — while its own comment
claims to "mirror" `#zkite-root`'s `min-height` rule. The comment described the looser
rule and the code implemented the stricter one, and only the stricter one is correct.

**Why the existing test suite passed.** `e2e.mjs`'s list fixture hardcodes `h-[300px]`
on the scroll container, so it is bounded by construction and could never exercise what
a real app does (`List("flex-1")` inside a `flex-1` root, no explicit height anywhere).
Same shape as phase 2 §6/M1's `catch_unwind` finding: a test that passes for a reason
unrelated to what it claims to check. Two new checks use the real shape, and they
**fail against the old CSS** — the list container measured 50,000px inside a 720px
viewport — which is how the fix was confirmed to be a fix rather than a coincidence.

**Fix and verification.** `#zkite-root { height: 100% }`, matching what native always
did. On the host, 23/23 e2e checks and the container correctly bounded at 664px. On the
device: **zero renders after the first, DOM stable at 101 nodes**, against 268 renders
and 10,361 nodes before. Conformance re-run end to end afterwards is **unchanged on all
14 fixtures** to two decimal places (`button` 1.78/3.20, `text` 5.00/6.58, `textfield`
1.89/4.07, `list` 2.99/4.16, `typography` 9.95/13.68 — §9.7's table exactly).

### 12.6 The measurements

Samsung SM-E066B, 720x1600 @ 300dpi, Mali/MT6835, Android 16. Same `main.py` on both
backends, back to back, `appshell` vs `gpu_harness`. All WebView numbers are post-fix
(§12.5.3). **Read every timing against §12.5.2 first** — the two backends are not
drawing the same size, and the distortion does not go one way.

#### Cold start to first pixel (§12.2 item 1)

Assets are unpacked by one warm-up launch first, then force-stop between runs — a
user's cold start is not their first-ever launch. Both halves reported, since the split
is where the information is.

| `cmp_counter`, n=8 | native | WebView |
|---|---|---|
| `am start -W` TotalTime (see §12.5.1 — *not* cold start) | 215 ms | 461 ms |
| process start → shell hands off | 43 ms | 246 ms |
| hand-off → first real painted frame | 1226 ms | 945 ms |
| **TOTAL, median** | **1267 ms** | **1203 ms** |
| **TOTAL, worst** | **1330 ms** | 1821 ms |

**Effectively a tie on the median, with native materially more consistent** (1330 vs
1821 worst). Both are far outside plan1.md §9's "first pixel <100ms, interactive
<500ms" — by roughly 2.5x on the looser of the two targets. That target is missed by
the *architecture*, not by either backend: both spend most of the time in the same
CPython init, and neither has a splash-from-Rust-on-frame-1 path (§9's own mitigation)
built.

The 10,000-row app costs both backends more, and about equally: native 1562 ms median,
WebView 1295 ms.

#### Sustained scroll, 10,000 rows through `List` (§12.2 item 2)

The measurement §12.2 predicted would be "most likely to be decisive either way." It
was. Reported as a distribution, per §12.2's own instruction that jank is a tail
property:

| `cmp_list_10k`, 8 swipes | native | WebView |
|---|---|---|
| frame interval, p50 | — (avg 25.1–26.0) | **16.60 ms** |
| frame interval, avg | **25.1–26.0 ms** | **16.6–17.5 ms** |
| frame interval, p95 | — | 16.7–29.6 ms |
| frame interval, max | 39.7–53.8 ms | 33.4 ms |
| effective frame rate | **~39 fps** | **~60 fps** |
| nodes / DOM elements per frame | 218 | 93 |
| our own work per frame | 12.4–13.4 ms | 1.5–6.1 ms (`apply`) |

**The WebView backend holds a locked 60fps on a 10,000-row list and the native backend
does not.** 16.60ms p50 against a 16.67ms vsync period is the panel's refresh rate, not
a coincidence — Blink is scrolling this list on its compositor without re-entering JS
or Python at all, which is the architectural difference: on this backend a scroll that
doesn't move the `List`'s window is *free*, and on the native backend every scroll
frame is a full rebuild-layout-paint.

This directly contradicts plan1.md §5's table, which rates the WebView "web-in-a-box;
scroll and keyboard jank" and the custom renderer "Flutter-like: consistent,
controllable." On the one scroll workload anyone has now actually measured on both, the
ordering is reversed.

**The most actionable native finding in this section comes from measuring interval and
cost separately** — which is why §12.5.1 added the distinction rather than reusing the
existing redraw stat. Native's frame *cost* is 12.4–13.4ms, comfortably inside a 16.7ms
budget; its delivered *interval* is 25ms. **About half of every native frame is the
render thread not working.** That is scheduling, not rendering: §6.3's fling loop wakes
on a fixed `FRAME_INTERVAL` (16ms) `recv_timeout` and is not aligned to vsync or driven
by a Choreographer, so a frame that finishes early waits for a timer instead of the
display. Per-phase, the work itself is now well spread — build 1.22, layout 1.81,
positioned 0.16, sync ~3.5, a11y 0.25, scene 2.85, present 2.95 ms — with no dominant
term left. §11.9's shaped-text cache is doing its job throughout: **99.6% hit rate,
213–219 entries, flat**, confirming §11.9's bounded-working-set claim on a second
device and a different GPU.

#### Tap to visible change (§12.2 item 3)

Same span on both sides: from the backend first acting on the tap to the resulting
frame being presented.

| | native | WebView |
|---|---|---|
| `cmp_counter` (6 taps) | **3.9–6.7 ms** (median ~5.6) | 21.2–32.6 ms (median ~26.8) |
| `cmp_text_churn`, 200 changed texts (6 taps) | **32.7–39.5 ms** (median ~34.5) | 26.8–41.9 ms (median ~38.3) |

**Native wins interaction latency decisively on the small app — roughly 5x.** This is
the axis where owning the whole pipeline pays: a tap is a hit-test, a Python call, a
relayout and a repaint, with no DOM diff, no Blink style recalc, and no JS/Java
round trip in the middle.

**§12.2 item 5's text-churn case is a tie, which is the surprise.** This app was built
to be native's worst case: 200 text nodes whose content *all* changes on every render,
so §11.9's shaped-text cache — 99.6% effective while scrolling, because scrolling
reuses rows — cannot hit at all. Native still comes in at ~34.5 ms against the
WebView's ~38.3 ms. The workload designed to expose the native backend's known weakness
does not, in fact, expose it: parley shaping 200 fresh strings is competitive with
Blink doing the same job, and neither backend is anywhere near 60fps on it.

#### Memory (§12.2 item 4)

§12.2 warned that "a single-process reading will flatter it substantially," and the
first version of this session's own script fell into exactly that trap: `dumpsys
meminfo <pkg>` misses the WebView's sandboxed renderer, which runs as a separate
process under an isolated uid. Reading only the app process understated the WebView
backend by **47 MB**. Summed properly:

| summed PSS across every process the app owns | native | WebView |
|---|---|---|
| `cmp_counter` | 160 MB (1 process) | **143 MB** (98 + 48, 2 processes) |
| `cmp_list_10k` | **160 MB** (1 process) | 189 MB (133 + 61, 2 processes) |
| `cmp_text_churn` | **167 MB** (1 process) | 173 MB (118 + 54, 2 processes) |

**The interesting result is not who wins, it is the shape.** Native is essentially flat
at ~160 MB regardless of app, because it is dominated by a fixed cost — 77 MB of
`Graphics`, i.e. the swapchain, which is sized by the screen and not by the content.
The WebView backend starts 17 MB lower and grows with the DOM, overtaking native on the
10,000-row app. So §12.2 item 4's expectation ("the axis where the native backend is
most likely to win back some of its 14 MB of binary") is **half right**: native wins on
content-heavy apps and loses on small ones, which is close to the opposite of the
intuition that a browser engine is the heavy one.

#### Binary size

Rebuilt this session, same app, same ABI:

| | native | WebView |
|---|---|---|
| APK, arm64-v8a | **38.4 MB** | 23.6 MB |

Unchanged in substance from §12's original table: ~14.8 MB, permanent, on the wrong
side of the ledger.

### 12.7 The answer, against §12.3's pre-stated criteria

§12.3 named three outcomes before any number existed. Taking them in order:

> If native is **decisively better on frame time and memory**, the 14 MB is paid for
> and the switch should flip once the remaining gaps close.

**No.** Native is not decisively better on frame time — it loses the headline scroll
measurement 39fps to 60fps — and it is not better on memory in general, only on
content-heavy apps. The 14.8 MB is not paid for by these numbers.

> If native is **comparable but not better**, the case rests entirely on the
> qualitative claims (Tailwind fidelity, consistency across devices, the ceiling) —
> which is a defensible position, but a different one from what plan1.md §5 currently
> asserts, and the document should be corrected to say so.

**This is the one that fits, and it fits with a sharper edge than it was written
with.** The aggregate is genuinely mixed: native wins tap latency ~5x, ties text-churn
re-render, ties cold start (with a better worst case), wins memory on large apps, loses
memory on small ones, loses 14.8 MB of binary permanently — and loses sustained scroll
decisively. That is "comparable but not better."

The sharper edge is *which* axis it loses. plan1.md §5 does not merely claim native is
faster in the abstract; it specifically charges the WebView backend with "scroll and
keyboard jank" and credits the custom renderer with being "Flutter-like: consistent,
controllable." **On the one scroll workload now measured on both backends on one
device, the WebView holds a locked 60fps and the native renderer does not.** So the
correction §12.3 anticipated is owed, and it is owed on the specific sentence that
motivated building the renderer in the first place.

> If native is **worse**, that is the single most valuable finding phase 3 could
> produce, and plan1.md §11's own advice ("consider dropping Phase 3 to stay on the
> WebView backend for 1.0") becomes live again rather than hypothetical.

**Not claimed, and deliberately not claimed** — because two things in these numbers are
known to be fixable rather than intrinsic, and honesty cuts in this direction too:

1. **Native's frame cost is already inside budget; its interval is not.** 12.4–13.4 ms
   of work delivered on a 25 ms cadence means roughly half of every frame is the render
   thread idle on a fixed 16 ms `recv_timeout` (§6.3) rather than tracking vsync. That
   is a scheduling defect, not a rendering one, and nothing in these measurements says
   the renderer cannot hold 60fps once it is paced by the display.
2. **Native was doing ~2.3x the row work** (218 nodes vs 93) purely because of
   §12.5.2's density bug, which shrinks its layout viewport and so fits more rows on
   screen. Fixing density reduces native's per-frame work; it does not change the
   WebView's.

Neither of those is a reason to discount the result — native *today* is what was
measured, and today it loses. They are reasons the result is not yet final.

**So: the switch does not flip on this evidence, and phase 3 is not dropped on it
either.** The concrete, ordered consequence:

1. **Fix the density bug (§12.5.2).** It is a correctness bug independent of any
   performance argument — every native app currently renders at `1/dpr` size on every
   real phone — and it also removes the largest confound from this comparison. It needs
   a scale factor plumbed from `DisplayMetrics` through `Native.boot` into
   `zkite_renderer`, and it wants its own conformance thinking, since §9's harness runs
   at dpr 1 and will not see whether it is right.
2. **Pace the render thread to vsync** rather than a fixed 16 ms timeout — a
   `Choreographer` callback, or at minimum aligning the fling tick to the surface's
   real refresh rate (§6.3 already flags `FRAME_INTERVAL` as "worth revisiting if a
   future device shows tearing/stutter"; this is that device, and the symptom is worse
   than stutter).
3. **Re-run §12.2** afterwards, on this device *and* the Moto G54, and only then treat
   §12.3 as answered rather than provisionally answered.

Until (1) and (2) land, plan1.md §5's rendering table should be read as **untested at
best and contradicted at worst**, and plan1.md §10's phase-3 exit criterion — "the
backend switch flips by default" — has no evidence behind it.

> **Update: (1), (2) and (3) have all landed — see §13, which supersedes this
> section's forward-looking half.** Both fixes are built and verified live, §12.2 has
> been re-run with both backends on the Moto G54, and §13.4 applies §12.3's criteria
> again to the new numbers. The verdict is unchanged ("comparable but not better") but
> **the "contradicted at worst" reading above is withdrawn**: native's scroll deficit
> was the fixed-timer pacing defect item (1) predicted, and with it gone the gap is
> ~58fps vs 60fps rather than 39 vs 60. plan1.md §5's table remains *untested* on the
> qualitative claims it actually rests on; it is no longer contradicted on scroll.

### 12.8 Known gaps

- **Different device from every other number in this document.** The Samsung
  SM-E066B, not the Moto G54 — so nothing here is comparable to §11.5/§11.9's figures,
  and the native scroll number cannot be cross-checked against §11.9's known 23.8 ms.
  Both backends were measured on the same phone back to back, which is what makes the
  *comparison* valid; it is the continuity with earlier sections that is broken.
- **The density bug confounds every timing** (§12.5.2), in both directions, by
  amounts that are not a single scalar. Stated in §12.6 and again here because it is
  the thing most likely to be forgotten when these tables get quoted.
- **§12.2 items 2 (partially) and 6 not run.** Only the 10,000-row size was measured;
  `cmp_list_60.py` and `cmp_list_1k.py` were built and are ready but not run, so the
  scaling curve between sizes is unmeasured. Battery/thermal (item 6, already flagged
  as lower confidence) was not attempted at all.
  **The scaling curve is now run on both backends — see §13.3**, and it found that
  native's cost tracks materialised node count rather than list length (1,000 and
  10,000 rows cost the same), so the 60-row *non-virtualised* control is native's worst
  case rather than its best. Battery/thermal is still not attempted.
- **Cold-start runs are n=8 for `cmp_counter` and n=2–3 elsewhere.** §12.2 asked for
  ten. The counter numbers are solid; the rest are indicative.
- **The frame-interval sampler is itself a rAF loop**, so it requests frames while it
  runs. It is armed only by real scroll activity and stops 250 ms after the last scroll
  event, but it is not a zero-observer measurement and a p50 pinned exactly at the
  vsync period is partly evidence of that.
- **`am start -W` is reported but is not cold start** (§12.5.1) — kept only as a
  same-machinery relative number.
- **Nothing here tested the qualitative claims** plan1.md §5 actually rests on for
  native (Tailwind fidelity, cross-device consistency, the ceiling). §9's conformance
  work is the closest thing, and it says fidelity is good; none of it is measured
  against the WebView backend *on a device*, which §12.5.2 shows is a different
  question from measuring it on a host.
- **`devshell`'s console forwarding is written but unexercised.** The `WebChromeClient`
  change is in both shells; only `appshell`'s was run.

## 13. Density, vsync pacing, and the re-run of §12

**Status: both of §12.7's named fixes are built and verified live on the Moto G54, and
§12.2 has been re-run on that device with both backends measured back to back. §12.7 is
promoted from provisional to answered below (§13.4). Its verdict — "comparable but not
better" — survives, but the specific charge it was sharpest about does not: native no
longer loses sustained scroll.**

This section is the work §12.7 ordered: (1) fix density, (2) pace the render thread,
(3) re-run §12.2 and only then treat §12.3 as answered.

**Read §12.6's tables as historical.** They were measured on the Samsung SM-E066B and
everything here is on the Moto G54 (1080x2400 @400dpi, dpr **2.5**, PowerVR, Android
15), so the two are not comparable to each other. Every comparison below is
native-vs-WebView on *one* device, back to back, which is the property that makes it
mean anything.

**A correction to this document's own record, found by reading the device rather than
the note:** §12.5.2 and several memory entries describe the Moto G54 as 420dpi
(dpr 2.625). It reports **400dpi, dpr 2.5** (`wm density`, and `dumpsys display`'s own
`density={2.5}`). So the "38%" figure §12.5.2 predicted for this phone was really 40%.
The bug and its shape are unchanged; the number was slightly off because it came from a
remembered spec rather than a measurement.

### 13.1 Density — the fix, and why parley's own `scale` is the wrong lever

**The bug** (§12.5.2): `twcore` resolves `px-6` to 24, and that went straight to taffy
against a surface measured in *device* pixels. The WebView renders the same class into
an `initial-scale=1` page where 24 is 24 CSS px and Blink multiplies by the device
pixel ratio. Every native app therefore rendered at `1/dpr` size on every real phone.

**The shape of the fix is the one §12.7 sketched, with one substantive change.** The
sketch said *"shape text at device resolution so glyphs stay crisp — read parley's own
scale parameter first, it exists for this."* Read first, and it is the wrong lever
**for this architecture**:

`parley`'s `ranged_builder(font_cx, text, scale, quantize)` multiplies the font size,
line height, letter spacing *and everything the resulting layout reports* by `scale`
(`resolve/mod.rs`: `FontSize(*value * scale)`, `line_height.scale(scale)`, …). It
exists for a design where layout itself is in device pixels. Since taffy is staying in
dp, passing `scale` there would hand taffy device-pixel text metrics while every other
box is in dp — precisely the "metrics in device px while layout is in dp, which breaks
measurement" failure the instruction warned about. **`zkite_renderer` therefore keeps
`scale = 1.0` and is completely unaware that density exists.**

Crispness comes from the paint transform instead, and this is not a compromise —
it is the mechanism the library is built around. `vello_hybrid::Scene::glyph_run` and
`vello_cpu::RenderContext::glyph_run` both hand the current scene transform to
`glifo`, whose `prepare_glyph_run` **absorbs a uniform scale into the font size**
(`AbsorbScaleHinted` → `vertical_font_size = run.font_size * t_d`) and builds the
*hinting instance at that device-pixel size*, leaving only translation in the residual
transform. So one `Affine::scale(dpr)` on the scene yields glyph outlines generated and
hinted at true device resolution. `vello_hybrid` additionally sets
`atlas_cache_enabled: false`, so there is no fixed-resolution glyph atlas to blur.
Checked by reading `glifo-0.2.0/src/glyph.rs`, not inferred from the parameter name.

**The coordinate contract**, now stated in one place (`AppPaths::scale`'s doc comment)
rather than implied:

| space | what lives in it |
|---|---|
| **dp** | `twcore` lengths, the taffy tree, every `PositionedNode.rect`, scroll offsets, caret and tap-to-offset geometry, the IME pan, `touch_slop` |
| **device px** | the surface, incoming `MotionEvent` coordinates, the IME inset, accessibility bounds, the selection overlay's Android view rect |

Conversions happen only where those meet, and there are exactly six:

1. **In** — `DisplayMetrics.density` → `Native.boot` → `AppPaths::scale`. Rejects a
   non-finite or non-positive value and falls back to 1.0 *loudly*, because that
   degrades to the old (visibly small, working) behaviour rather than a blank screen.
2. **Layout** — `State::logical_size()` divides the surface by `scale`; taffy lays out
   in dp.
3. **Paint** — one `Affine::scale(scale)` on the scene, in *both* backends. Clip paths
   go through the same transform (`clip_path_transform()` returns the current
   transform), so scrolled content still clips correctly.
4. **Touch** — `State::touch_to_dp()` at the three `Command::Touch*` arms.
   `touch_slop`/`double_tap_slop` are divided once at `boot()`, since
   `ViewConfiguration.getScaled*` returns device pixels.
5. **IME inset** — divided on arrival, because its only consumer compares it against a
   dp layout.
6. **Out** — accessibility bounds and the selection-overlay rect/text-size multiply
   back up, since both are consumed by real Android views.

**The touch half is the one that would have been missed**, exactly as §12.7 predicted
and §9.7 documented: a screenshot cannot see it. Left in device px, taps land at
`density` times their true offset — approximately right near the origin and worse the
further from it, which reads as "hit-testing is flaky" rather than as a unit error.

**One deliberate behavioural change, not a silent one.** Fling velocity is now dp/s.
Fling *distance* is unaffected (travel is `v0 * FLING_TAU`, linear in `v0`, so the same
flick covers the same physical distance). The two threshold constants
(`MIN_FLING_START_VELOCITY`, `MIN_FLING_STOP_VELOCITY`) now mean the same physical
speed on every device instead of getting `density` times stricter as screens get
denser — which is the more correct reading of a "was that a flick" test, so they were
reinterpreted rather than divided to preserve the old numbers exactly. Written into the
constants' own doc comment.

#### 13.1.1 Verification, live on the Moto G54

**The measurement that counts** is §12.5.2's own, re-taken: the same `cmp_counter.py`
through both shells on one phone, finding the indigo button's bounding box by colour.

| | native | WebView | ratio |
|---|---|---|---|
| **before** (§12.5.2, Samsung, dpr 1.875) | 130 x 48 | 242 x 90 | 1.86 / **1.875** |
| **after** (this session, Moto G54, dpr 2.5) | **324 x 119**, x378..701 | **324 x 120**, x378..701 | **1.00** |
| digit "0" | 107 x 136, x487..593 | 108 x 135, x486..593 | **1.00** |

Widths and x-ranges are **identical to the pixel**; the 1px height difference is
rounded-corner AA at the extremes, the same caveat §12.5.2 recorded for its own width.
324/2.5 = 129.6 ≈ the 130 dp box §12.5.2 measured natively pre-fix, which is the
arithmetic confirming this is the same layout scaled rather than a different one.
Re-confirmed independently on a second app (`cmp_text_churn`: 374 x 120 native vs
376 x 120 WebView).

Everything density-sensitive was then exercised **under a finger**, because §9.7's
lesson is that the coordinate paths are invisible in a screenshot:

- **Taps** — `onTouchEvent x=539.0 y=330.0` (device px) → `Touch (215.6, 132)` (dp,
  exactly ÷2.5) → dispatched to the button; counter incremented `0 → 2` on screen.
- **A11y bounds** — `uiautomator dump` reports Increment at `[377,387][702,507]` =
  325 x 120 device px, matching the painted 324 x 119. Pre-fix these would have been
  130 x 48.
- **IME** — focusing "Your name" converts `(285, 2143)` → `(114, 857.2)` dp, shows the
  real keyboard, and pans the field ~835 device px to sit directly above it.
- **Caret and tap-to-offset** — the caret renders 56 device px tall (≈22 dp) directly
  after the text; tapping before the text produced `setState: 'u8', selection=(0,0)`
  and the caret visibly moved to before the first glyph. This is the whole
  dp-hit-test → dp-caret-geometry → device-px-paint chain agreeing.
- **Selection overlay** — a long-press put the real `EditText` at `[40,1375][530,1475]`,
  exactly over the field it covers, preserving §7.8's "occlusion is exact, not
  approximate" premise. In dp it would have sat at a quarter of that, nowhere near it.
- **Scroll/fling** — `fling: launching at 2882px/s (max_scroll_y 2359)`, both now dp.
- Zero `panic`/`FATAL`/`AndroidRuntime`/`ANR` markers across the entire session, process
  alive throughout.

#### 13.1.2 The test story, decided rather than discovered — a second dpr in the conformance harness

§9.3 had already named the blind spot: *"the harness runs at an implicit device pixel
ratio of 1, and so cannot see a density bug at all."* It is now closed rather than
re-noted. `run.sh` runs the **entire pipeline at every dpr in `$ZKITE_CONFORMANCE_DPRS`
(default `1 2`)**; dpr 1 keeps writing to `out/` so every path and number recorded in
§9 still resolves, and 2 gets `out-dpr2/`.

Both sides scale the way the real thing does, which is the point:

- **native** lays out at the fixture's declared size *in dp* — unchanged — and only the
  canvas grows, with the same `Affine::scale(dpr)` scene transform the device uses.
- **WebView** uses Playwright's `deviceScaleFactor`, which likewise keeps CSS pixels
  fixed and multiplies the backing store. (It is a browser-*context* property and
  cannot change per screenshot, which is why a second dpr is a second whole pass.)

A fixture that renders identically at both is evidence the two backends agree about
density; one that passes at 1 and fails at 2 is exactly the bug §12.5.2 found.

**dpr 1 is unchanged to two decimal places on all 14 fixtures** — `button` 1.78/3.20,
`text` 5.00/6.58, `textfield` 1.89/4.07, `list` 2.99/4.16, `typography` 9.95/13.68,
i.e. §9.7's table exactly. **dpr 2 is clean across the board**, and mostly *lower*,
which is expected rather than suspicious: at twice the resolution, glyph-edge AA is a
smaller fraction of the pixels, and AA is the irreducible floor §9.1 describes.

| fixture | dpr 1 (loose / strict) | dpr 2 (loose / strict) |
|---|---|---|
| button | 1.78 / 3.20 | 1.06 / 1.77 |
| text | 5.00 / 6.58 | 2.36 / 2.75 |
| textfield | 1.89 / 4.07 | 1.89 / 2.93 |
| typography | 9.95 / 13.68 | 5.38 / 7.51 |
| list | 2.99 / 4.16 | 2.04 / 2.78 |
| scrollview | 2.28 / 3.21 | 1.56 / 2.12 |
| column / grid / row / stack | 0.22 / 0.03 / 0.40 / 0.04 | 0.18 / 0.05 / 0.35 / 0.02 |
| icon / image / slider / switch | 1.22 / 0.15 / 1.28 / 0.17 | 1.11 / 0.00 / 1.16 / 0.07 |

**And it was confirmed to fail against the bug, not merely to pass.** A passing test
proves nothing until it has been shown to fail — the lesson phase 2 §6/M1 paid for with
its `catch_unwind` test and §12.5.3 paid for again with the `h-[300px]` list fixture.
`render_one` was temporarily reverted to the pre-fix behaviour (lay out against a
*device-pixel* viewport, paint with no transform) and re-run at dpr 2:

```
button 38.90% / 85.72%   column 49.17% / 92.04%   grid 67.89% / 93.19%
stack  85.55% / 85.66%   list   56.18% / 68.03%   textfield 48.26% / 84.44%
```

All 14 fixtures fail, loudly, on both columns. Restored immediately afterwards.

### 13.2 Vsync pacing — the render thread was sleeping a frame *and then* rendering

§12.6's most actionable finding was that native's frame **cost** (12.4–13.4ms) sat
inside a 16.7ms budget while its delivered **interval** was 25ms. The cause turned out
to be simpler and more embarrassing than "not aligned to vsync":

`run()`'s fling path did `rx.recv_timeout(FRAME_INTERVAL)` — a *fixed 16ms wait* — and
then did a frame's work. So the delivered period was `interval + work`, not
`max(interval, work)`. 16 + ~14 ≈ 30ms. The timeout was a delay, not a deadline.

**The swapchain was already the vsync signal all along.** The surface is configured
`PresentMode::Fifo`, so `get_current_texture`/`present` already blocks until the
display is ready — that is what the 8.1ms `present` phase largely *is*. The fixed sleep
was stacked on top of a pipeline that was already display-paced.

**Fix, two parts:**

1. **`next_frame_wait()` is a deadline**, measured from when the last frame *started*
   (`State::last_frame_at`, set at the top of every `redraw()`, not just fling ticks).
   The wait absorbs the work instead of following it. Returning `ZERO` when the period
   has already elapsed is deliberate: it means "render now, we're behind," and the
   Fifo swapchain is what actually decides when the frame is shown. This paces *to*
   vsync rather than replacing it; the timer's only job is to wake a thread that would
   otherwise block forever, and it should never be what gates a frame.
2. **The period comes from the panel**, not a constant — `Display.getRefreshRate()`
   through `Native.boot`, same "ask, don't guess" path as `touch_slop` and density.
   This device advertises 30/60/90/120 and runs at 60; a hardcoded 16ms would
   under-drive a 120Hz panel by half. Implausible values fall back to 60Hz, loudly.
   `FRAME_INTERVAL` is now `FALLBACK_FRAME_INTERVAL` and is only that fallback.

**A/B on one device in one session, one line differing** — the technique §4 used for
the CPU fallback, §8.7 for the accessibility abort and §11.9 for the shape cache,
chosen over comparing against a number recorded on different hardware:

| `cmp_list_10k`, 8 swipes | fixed delay (pre-fix) | deadline (post-fix) |
|---|---|---|
| frame interval, avg | **29.84 – 30.51 ms** | **16.91 – 17.53 ms** |
| effective frame rate | ~33 fps | **~58 fps** |
| frame interval, max | 37.85 – 43.30 ms | 22.80 – 31.23 ms |
| frame **cost**, avg | 15.22 – 15.78 ms | 12.85 – 14.86 ms |

The pre-fix interval is ≈ `16 + work`, which is the mechanism stated as an equation and
then observed. Frame *cost* is essentially unchanged, as it must be — this makes
nothing faster, it stops adding a sleep to something that was already fast enough.

### 13.3 §12.2 re-run — both backends, one device, both fixes in

Moto G54 5G, 1080x2400 @ 400dpi (dpr 2.5), 60Hz, PowerVR, Android 15. Same `main.py`
through `gpu_harness` and through `zkite build apk` (`phase_2/appshell`), back to back.

#### Sustained scroll (§12.2 item 2) — including the scaling curve §12.8 left unrun

| app | native, frame interval | WebView, frame interval |
|---|---|---|
| `cmp_list_60` (60 rows, **plain `ScrollView`**) | 23.14 – 23.73 ms (306 nodes) | 16.57 ms p50 16.60 (184 nodes) |
| `cmp_list_1k` (1,000 rows, `List`) | **16.88 – 16.94 ms** (152 nodes) | 16.61 ms p50 16.60 (98 nodes) |
| `cmp_list_10k` (10,000 rows, `List`) | **16.91 – 17.53 ms** (152 nodes) | 16.59 – 16.91 ms p50 16.60 (98 nodes) |

**Native went from ~39fps to ~58fps on the headline workload, and the 39-vs-60 gap
§12.7 called decisive is now ~58-vs-60.** The WebView still holds a locked 60 (a p50 of
16.60 against a 16.67ms vsync period), and still does it by not re-entering JS or
Python on a scroll that doesn't move the window, which remains a real architectural
advantage. But "the WebView scrolls a 10,000-row list at 60fps and the native backend
manages 39" is no longer a true sentence.

**Two things fell out of finally running the other two sizes:**

- **The curve is flat in list length and tracks materialised node count instead** —
  1,000 and 10,000 rows cost native the same 16.9ms because both materialise 152
  nodes. That is exactly what §11.2's spacer design is *for*, now measured across three
  sizes rather than argued.
- **The 60-row control is native's *worst* case, not its best.** It is a plain
  non-virtualised `ScrollView` (306 nodes, twice the windowed lists') and costs ~40%
  more per frame than a 10,000-row `List`. A real app with a long list is cheaper here
  than a short un-windowed one, which is worth knowing before optimising the wrong end.

#### Tap to visible change (§12.2 item 3)

| | native | WebView |
|---|---|---|
| `cmp_counter`, 6 taps | **6.7 – 14.8 ms** (median ~7.8) | 19.7 – 47.1 ms (median ~28.3) |
| `cmp_text_churn`, 200 changed texts, 6 taps | 31.4 – 34.7 ms (median ~31.9) | 24.0 – 39.1 ms (median ~31.2) |

Native wins interaction latency ~3.6x, and the text-churn workload — built to be
native's worst case, where §11.9's shaped-text cache cannot hit at all — is again a
**tie**. Both findings reproduce §12.6's on different hardware, which is the first time
any §12 result has been confirmed on a second device.

#### Cold start to first pixel (§12.2 item 1)

Warm-up launch first, then force-stop between runs.

| | native | WebView |
|---|---|---|
| `cmp_counter`, n=6, median | 1450 ms (1426 – 1534) | **958 ms** (933 – 1112) |
| `cmp_list_10k`, n=5, median | 1450 ms (1447 – 1468) | **972 ms** (956 – 992) |

**This reverses §12.6's tie: on this device the WebView starts ~490ms faster,
consistently.** Native is remarkably *stable* (a 108ms spread over six runs) but
slower. Both remain far outside plan1.md §9's "interactive <500ms", by ~3x and ~2x —
and, as §12.6 already argued, that target is missed by the architecture (CPython init,
no splash-from-Rust-on-frame-1), not by either backend.

#### Memory (§12.2 item 4)

Summed PSS across every process the app owns. §12.6's trap — `dumpsys meminfo <pkg>`
misses the WebView's sandboxed renderer — has a second one behind it, hit and fixed
here: grepping the whole `dumpsys meminfo` **double-counts**, because the same process
appears under "Total RSS by process", "by OOM adjustment" and "by category" as well as
"Total PSS by process". A first attempt reported 952 MB across "4 processes" for one
app. Only the PSS-by-process table is wanted, and stale renderer processes from earlier
launches must be excluded.

| | native | WebView |
|---|---|---|
| `cmp_counter` | 320 MB (1 proc) | **235 MB** (172 + 63, 2 procs) |
| `cmp_text_churn` | 311 MB (1 proc) | **245 MB** (178 + 67, 2 procs) |
| `cmp_list_10k` | 308 MB (1 proc) | **291 MB** (202 + 89, 2 procs) |

**Native is flat at ~310–320 MB regardless of app and the WebView grows with content**
— the same *shape* §12.6 found. What changed is the crossover: on the Samsung
(720x1600) native was 160 MB and won the large app; here it is ~2x that and loses every
one. The reason is visible in the breakdown: **266 MB of native's 308 MB is
`Graphics`** (GL mtrack 210 MB + EGL mtrack 56 MB) — GPU allocations sized by the
screen, not by the content. This phone has 2.25x the Samsung's pixels; the graphics
cost grew rather more than that, which is **unexplained and worth its own look** (a
swapchain plus `vello_hybrid`'s render targets should not be 266 MB for a 1080x2400
surface). Recorded as an open question rather than explained away.

#### Binary size

Unchanged in substance: 40.3 MB native vs 24.8 MB WebView, arm64-v8a — ~15.5 MB,
permanent, on the wrong side of the ledger.

### 13.4 §12.7, promoted from provisional to answered

§12.3's criteria were fixed before any number existed and are still not edited.

> If native is **decisively better on frame time and memory** …

**Still no.** Frame time is now a near-tie on scroll and a decisive native win on tap
latency, but memory is a clear loss on all three apps on this device, and cold start
is now a loss too. The 15.5 MB is not paid for.

> If native is **comparable but not better** …

**Still this one — and it is now comparable in a way it genuinely was not before.**
The aggregate: native wins tap latency ~3.6x, ties text-churn re-render, **ties
sustained scroll (~58 vs 60fps, where it lost 39-vs-60)**, loses cold start by ~490ms,
loses memory on every app, loses 15.5 MB of binary permanently.

**What has to be withdrawn is §12.7's sharpest sentence.** It said the correction owed
to plan1.md §5 was owed "on the specific sentence that motivated building the renderer"
— that the WebView holds 60fps scrolling where the native renderer does not. That was
true of the code as measured and is no longer true: half of every native frame was a
thread sleeping on a fixed timer, exactly as §12.7 itself suspected, and removing it
recovered the gap. plan1.md §5's table is still **untested** on its own terms
(Tailwind fidelity, cross-device consistency, the ceiling — none of which §12 measures),
but it is no longer **contradicted** on scroll.

> If native is **worse** …

Not claimed, and now less claimable than it was.

**So: the switch still does not flip, and phase 3 is still not dropped.** What changed
is *why*. §12.7's two named blockers are closed; the remaining case against flipping is
no longer "native is slower where it promised to be faster" but a plainer ledger —
15.5 MB of binary, ~310 MB of resident memory dominated by an unexplained GPU
allocation, ~490ms of extra cold start, and the feature gaps §5 still lists (Autofill,
a11y scroll actions, overscroll, decoration, real Icon/Image content). Those are
ordinary engineering, and each has an owner in this document.

### 13.5 Two things found on the way that are not about density

- **The WebView shell renders a 140px system `ActionBar` that the native harness does
  not.** Found while explaining why the two backends' button bounding boxes matched in
  *size* but sat 68px apart vertically: `uiautomator` shows
  `android:id/action_bar_container` at `[0,117][1080,257]` in `appshell`, so its WebView
  content area is `[0,257][1080,2340]` against the native surface's `[0,117][1080,2340]`.
  Each backend centres its content correctly *within its own viewport*; the viewports
  differ. That is a real product-visible divergence between the two backends — a
  shipped ZKite app has a title bar on one backend and not the other — and it also means
  every §12/§13 WebView measurement had ~6% less vertical space than its native
  counterpart. Not fixed here; it is an `appshell` theme question, not a renderer one.
- **`adb exec-out screencap` intermittently returns an all-black frame** for this
  `SurfaceView`, most reliably when the display has dimmed or the app has only just been
  focused. Two separate measurements in this session were nearly taken from a black
  PNG. This is the same family as §12's own `am start -W` and `adb install -r` traps:
  always check the histogram (or `dumpsys display | grep mScreenState`) before trusting
  a screenshot-derived number. `svc power stayon usb` for the duration of a measurement
  run removes it; it was set and reverted here.

### 13.6 Known gaps

- **One device.** Everything here is the Moto G54. §12's tables are the Samsung. Only
  §12.6's tap-latency and text-churn findings have now been reproduced on a second
  phone; scroll, cold start and memory have not been re-measured on the Samsung with
  these fixes, so the cross-device story is still one data point per claim.
- ~~**Native's 266 MB of `Graphics` is unexplained** (§13.3) and is now the single
  largest item on the wrong side of the comparison. It should be attributed —
  swapchain vs. `vello_hybrid` render targets vs. glyph/paint atlases — before anyone
  concludes the native backend is intrinsically memory-hungry.~~ **Attributed and
  reduced — see §20.** It was none of the three candidates listed here: `memory_hints`
  was left at wgpu's `Performance` default, which reserves 128 MB device / 64 MB host
  suballocation blocks up front and is flat across content. `GL mtrack` 214 MB → 36 MB,
  total PSS ~334 MB → ~155 MB (counter) and ~314 MB → ~135 MB (10k list), scroll frame
  time unchanged. The native backend is **not** intrinsically memory-hungry. The
  swapchain half (`EGL mtrack`, 57–67 MB) remains unattributed and is now the largest
  single unexplained GPU item.
- **Cold start regressed relative to the WebView** and was not investigated. Native is
  ~1450ms here against the WebView's ~960ms; §12.6 had them tied at ~1.2s on the
  Samsung. Whether this is the device, the GPU/adapter probe, or something this
  session's changes added is unmeasured — nothing here should have affected it, which
  is a reason to check rather than to assume.
- **`present` is still unattributed** (§11.9's gap, unchanged): 8.1ms, and nothing yet
  separates GPU work from blocking on the swapchain. With deadline pacing in place this
  matters more, not less, because `present` is now the largest phase *and* the thing
  the frame rate is paced by.
- **Battery/thermal (§12.2 item 6) still not attempted.**
- **The pacing fix is fling-only.** A drag is still driven by `TouchMove` arrival with
  §6.5's coalescing, which needs no timer — but it also means the two paths reach a
  frame by different routes, and only the fling path was A/B'd.
- **No real `Choreographer`.** `next_frame_wait` targets the refresh *period*; it does
  not align to vsync *phase*. In `PresentMode::Fifo` the swapchain supplies the phase,
  which is why this is sufficient in practice — but a `Choreographer` callback remains
  the fuller answer and is what §12.7 item 2 literally asked for.
- **The dpr-2 conformance pass doubles the harness's runtime** and is on by default
  (`ZKITE_CONFORMANCE_DPRS=1` opts out). Still no CI, and still no per-fixture pass/fail
  gate (§9.3), so both dprs remain numbers a human reads.
- **Only dpr 1 and 2 are exercised.** Real phones run 1.5, 2.625, 3, 3.5; a
  non-integer dpr is where a rounding disagreement between Blink's backing-store
  rounding and `scaled_size`'s would actually show up, and none is tested.

## 14. The backend switch

§12.4 named this as the thing everything else in the phase was blocked behind:

> The comparison cannot be run at all until the native renderer is reachable with an
> arbitrary app *and* the WebView side is instrumented — so this section is blocked
> behind the same missing backend switch everything else is.

And §5's own framing all along has been that the native renderer is *a third shell*,
not a different architecture. Until this section it was a third shell nobody could
ship an app through: `phase_3/gpu_harness` was reachable only by running its own
`build_apk.sh` from this checkout, and neither `phase_1/cli`, `phase_1/devshell` nor
`phase_2/appshell` referenced `zkite_renderer` or `vello` at all. plan1.md §10's
phase-3 exit criterion — *"the backend switch flips by default and no sample app
changes a line"* — could not even be attempted, because there was no switch to flip.

**Status: `zkite build apk --backend native` exists and produces a standalone, signed
APK that runs an arbitrary `zkite new` app on the native renderer. The default is
unchanged (WebView).** What that took was much less than "build a shipping native
shell", for a reason worth stating because it was not obvious from this document:

### 14.1 The harness was already app-agnostic — `ZKITE_APP_MAIN` never baked an app in

`ZKITE_APP_MAIN` reads as if the harness compiles one app into the binary, and §12.4
described it that way ("rebuild per edit, hardcoded `ZKITE_APP_MAIN`"). It does not.
It selects which `main.py` gets zipped into `assets/app.zip` by `build_apk.sh`, and
`gpu_harness`'s own `ZKiteActivity` unpacks that asset at first launch through
character-for-character the same `unpackZipAssetOnce(APP_ASSET, ...)` call
`phase_2/appshell`'s does. The native shell has been able to host any `zkite_py` app
since §5 gave it `zkite_pybridge`; what was missing was never the shell, it was a
*release-artifact path* — a prebuilt `classes.dex` and `.so` set the CLI could stage
without a JDK, an NDK or a Rust toolchain on the user's machine (phase_2/PLAN.md
§1.1).

So this is promotion, not new invention — the same move phase 2 §1.1 made for
`pack`'s extras-splicing and §3 made for `zkite_renderer`.

### 14.2 What the switch actually is: four inputs, one pipeline

`build_cmd.rs` had four hardcoded references to appshell. They are now the four
fields of one `Shell` struct, selected by a `Backend`:

| | `webview` (default) | `native` |
|---|---|---|
| shell | `phase_2/appshell` | `phase_3/gpu_harness` |
| `lib/<abi>/` | `libzkiteappshell.so`, `libpython3.14.so` | `libzkitegpuharness.so`, `libpython3.14.so`, **`libc++_shared.so`** |
| `classes.dex` | 11 KB | 999 KB (adds `ZKiteSurfaceView`, `TextSelectionOverlay`, gametextinput, the androidx-core subset) |
| backend asset | `assets/webruntime/` | `assets/fonts.zip` |
| launcher activity | `dev.zkite.appshell.ZKiteActivity` | `dev.zkite.gpuharness.ZKiteActivity` |

Everything else in the pipeline — wheel resolution, pyc compilation, stdlib pruning,
`app.zip`, the extension-module rename (§11.8), signing, alignment, the generated
manifest and icon — is one code path shared by both, because **the app is the same
app**. That is plan1.md §5's central bet ("the Python API is backend-agnostic... swap
in the good one later without breaking anyone") expressed as a `match` with four arms
instead of a second build command.

Resolution order is `--backend` > `[tool.zkite] backend` > WebView. The pyproject key
is deliberately `[tool.zkite]`, **not** `[tool.zkite.android]`: which renderer an app
runs on is not an Android-specific fact (plan1.md §10 phase 4 already promises iOS and
desktop off the same renderer), and putting it under the android table would be a name
to regret.

**Deliberately not done: renaming `gpu_harness`.** A shipped app's manifest now names
`dev.zkite.gpuharness.ZKiteActivity`, which is a poor name for a production shell class.
Renaming it means 17 JNI symbols, four Java files, the cdylib name, three build
scripts and ~100 references across this document — mechanical, but a large, noisy diff
that would land on top of the one harness every §12/§13 measurement depends on. The
name is cosmetic, reversible, and invisible to app code; the switch is not. Flagged
rather than bundled in, same convention as every other scope call here.

### 14.3 Two release-pipeline gaps this exposed, both closed

Neither was visible while `build_apk.sh` was the only consumer, because that script
runs on a machine that has an NDK and a host font directory:

- **`libc++_shared.so` was located from the NDK at *staging* time.** `build_apk.sh`
  read it out of `$ANDROID_HOME/ndk/.../sysroot`. That is fine for a release-pipeline
  script and impossible for `zkite build`, which must not need an NDK. It is now
  copied into `artifacts/<abi>/` by `build.sh` — the one step in the chain that
  legitimately has an NDK — alongside the cdylib and `libpython3.14.so`. `resolve_abis`
  correspondingly checks *every* library a shell needs rather than just the cdylib,
  because a tree built before this change has the `.so` and none of the rest, and
  reusing it would produce an APK that dies at `System.loadLibrary` with an
  `UnsatisfiedLinkError` (§7.3 item 1's bug, reachable a second way).
- **`fonts.zip` was zipped from `phase_1/webruntime/fonts/`**, which is gitignored and
  materialised by `stage_fonts.sh` out of the *host's* `/usr/share/fonts`. Also now
  baked into `artifacts/` at release-pipeline time. This makes DejaVu concrete for the
  shipped native backend without settling §9.7's open product decision (bundle a family
  vs. read `/system/fonts`) — the zip's contents are whatever `stage_fonts.sh` staged.

`build_dex.sh` is new and is now the single authority for the native shell's
javac/d8 classpath; `build_apk.sh` calls it instead of keeping a second copy. Two
copies of that classpath would drift, and the failure mode is a
`NoClassDefFoundError` on a device rather than anything a build catches — which is
exactly what the androidx-core note in that script already records having cost once.

### 14.4 A real gap found on the way, in the *other* backend

**The shipped WebView backend was not staging the bundled fonts, and the way it fails
is silent.** §9.7 recorded this as a known gap ("neither `devshell` nor `appshell`
stages `fonts.zip` yet... on a device today the WebView backend still falls back to
Roboto"). Checked against the actual artifact rather than the note: `unzip -l` on the
last `zkite build apk` output contains no font at all.

The mechanism is worse than "not implemented". `stage_webruntime` copies the whole
`phase_1/webruntime` directory, `index.html` loads the six faces by relative path
(`fonts/sans.ttf`), and `webruntime/fonts/` is gitignored — so the build ships fonts
**when they happen to exist on the build machine** and silently omits them when they
don't, with the same command and no output either way. The consequence of omitting
them is invisible: the page falls back to the device's own sans-serif, which renders
perfectly well and diverges from both the native backend and every conformance number.

Not fixed by adding a font pipeline — fixed by making the fact observable.
`stage_webruntime` now prints `staged 6 bundled font faces` or warns loudly that this
app will not match the native backend or the conformance harness. On this checkout
the faces are present (`build.sh` materialises them), so the shipped WebView backend
now genuinely bundles its typefaces, which closes §9.7's last open item for that path.
`phase_1/devshell`'s own `build_apk.sh` has the identical `cp -r` and the identical
silent-omission behaviour, and is **not** fixed here — it is a dev-loop artifact built
only from this checkout, where the directory does exist, but the failure mode is the
same one and is now named.

### 14.5 Verification

**Host, and it is a real check rather than a smoke test.** One stock `zkite new`
project — plan1.md §8's counter, not one line edited — built both ways:

```
$ zkite build apk --backend native --abi arm64-v8a
  wrote switchdemo-arm64-v8a.apk (39.8 MB)
$ zkite build apk --abi arm64-v8a
  wrote switchdemo-arm64-v8a.apk (24.8 MB)
```

- `aapt2 dump badging` reports `launchable-activity: dev.zkite.gpuharness.ZKiteActivity`
  on one and `dev.zkite.appshell.ZKiteActivity` on the other, under the identical
  `package: dev.zkite.app.switchdemo`.
- Both pass `apksigner verify` on v2 **and** v3, the same bar phase 0 set for `pack`'s
  output and phase 2 held every build to.
- Payloads differ in exactly the five rows of §14.2's table and nowhere else.
- **The compiled app bytecode is byte-identical between the two APKs** — `main.pyc`
  past its 16-byte header (which carries the source mtime, so the whole file is not)
  hashes the same from both. That is the literal content of "no sample app changes a
  line", checked rather than asserted.
- 15.0 MB of APK difference, consistent with §13.3's 15.5 MB on the demo app.
- Host suites: `phase_1/cli` 20/20, `phase_2/bundler` 18/18 including the real
  `apksigner`/`zipalign`/`aapt2`/`bundletool`/`keytool` checks.
- One new bundler test asserts the two backends' manifests differ in the activity name
  **and are otherwise character-for-character equal**. The failure it exists to catch
  is an `activity` that silently stays on the appshell default, which would stage the
  native `.so` behind the WebView class and die at
  `System.loadLibrary("zkiteappshell")` — a launch-time failure with nothing host-side
  to see it.

**Device: run, on the Moto G54 (arm64-v8a, Android 15), both backends back to back
from the same project directory.** (The phone dropped off the USB bus mid-session —
absent from `lsusb`, not merely from `adb devices` — and this was done after it came
back.) The five things a device run had to establish, and what it showed:

1. **The app launches** — `dev.zkite.gpuharness.ZKiteActivity` resolved out of a
   `classes.dex` packed by a script that had never packed it before, under
   applicationId `dev.zkite.app.switchdemo`, focused two seconds after `am start`.
   This is the one that could not be checked host-side at all: `android:name` being
   independent of the manifest `package` is documented behaviour, and documented
   behaviour is what §7.3's `setSoftKeyboardActive` and §7.7's `setState` both turned
   out not to be.
2. **`libc++_shared.so` resolved from `artifacts/`** — `boot(): GameTextInput
   initialized` is the vendored C++ running, so `System.loadLibrary` found the library
   staged from the new location rather than the NDK path `build_apk.sh` used.
3. **`fonts.zip` staged from `artifacts/` unpacked and rendered** — `unpacked
   fonts.zip: 6 files in 49 ms`, and the render thread started with `fonts from
   /data/user/0/dev.zkite.app.switchdemo/files/fonts`. Also `67 native extension
   modules available`, so §11.8's rename survives the CLI staging path.
4. **The counter increments on a real tap.** Three `input tap`s at the button's
   measured centre: `Touch (215.6, 492.8) -> dispatching tap to r.1`, `tap-to-paint
   10.0ms / 7.7ms`, and the rendered digit goes from a `0` to a `3` — read out of the
   two screenshots as pixels, not inferred from a log line. The touch coordinates are
   also the whole §13.1 dp chain agreeing end to end: 539 device px → 1232 view-local
   (the 117 px status-bar margin) → 492.8 dp (÷2.5).
5. **`--backend webview` on the same project still works** — same install, same taps,
   `0` → `2`, `tap-to-paint 39.8/24.0ms`.

**Two cross-checks that were not planned and are worth more than the plan.**

- **The button measures 324 x 119 native against 324 x 120 WebView** — identical to
  the pixel in width, the same 1 px rounded-corner AA difference §13.1.1 recorded, and
  the same numbers as §13.1.1's harness-vs-appshell run. §13.1's density fix therefore
  survives the shipped path on both backends, which nothing had checked.
- **The WebView backend is now rendering the bundled typeface**, which §14.4's fix was
  supposed to do and which no device run had ever confirmed. The evidence is
  incidental and hard to argue with: its `0` measures 108 x 135 against native's
  108 x 136, with a pixel-for-pixel matching outline. A Roboto fallback would not
  agree with DejaVu to a pixel.

Zero `panic`/`FATAL`/`AndroidRuntime` markers across the whole session on either
backend; the process stayed alive throughout both.

### 14.7 One thing measured on the way: native's GPU memory now has a control

§13.6 lists the 266 MB of `Graphics` as *"now the single largest item on the wrong
side of the comparison,"* to be attributed *"before anyone concludes the native
backend is intrinsically memory-hungry."* Two facts fell out of having both backends
running the same app on one device in one session, which is exactly the comparison
§13.3 could not make:

| `dumpsys meminfo`, stock counter app | native | WebView |
|---|---|---|
| GL mtrack | 215.0 MB | 50.1 MB |
| EGL mtrack | 57.5 MB | 51.2 MB |
| **Graphics total** | **272.5 MB** | **101.3 MB** |
| process total | 315 MB (1 proc) | 202 MB (152 + 45, 2 procs) |

- **It is not content-dependent.** §13.3 measured 266 MB on a 10,000-row list; this is
  272 MB on a **six-node** counter. The allocation is sized by the screen, not by the
  scene, which was the hypothesis and is now measured rather than assumed.
- **There is now a control for "what is normal on this surface": ~101 MB.** Blink is
  compositing the same 1080x2400 screen for 101 MB. So the open question sharpens from
  "is 266 MB a lot?" to "why is native 2.7x a browser engine on the same surface,"
  which is a much more answerable question.

**One hypothesis tested and refuted, recorded because it is a plausible-sounding
answer that happens to be wrong.** `reconfigure` assigns
`hybrid_renderer = Some(Renderer::new(..))`, and Rust evaluates the right-hand side
first — so the old renderer is alive for the whole of the new one's construction. It
runs twice on every launch (`NewWindow`, then the `Resize` Android always delivers
straight after), and half of 272 is 136, which is the kind of arithmetic that makes a
hypothesis feel confirmed before it is tested. Dropping the old renderer explicitly
first moved GL mtrack **from 215035 KB to 215024 KB** — 11 KB on a 210 MB allocation.
The change was reverted; a line that reads like a memory fix and isn't one is worse
than its absence (same call §7.11 made on `performLongClick`).

What that rules out: transient double-allocation across a reconfigure, and by
extension the "leaks per resize" half of §13.6's own framing, at least for the two
reconfigures a launch performs. What it leaves: the allocation is a single, fixed,
screen-sized cost inside `vello_hybrid::Renderer::new` or the swapchain. A swapchain
at this size is ~9.6 MB an image, so even a triple-buffered one is ~29 MB and cannot
be it. Reading `vello_hybrid 0.1.0`'s sources, several of its textures are sized
`max_texture_dimension_2d` wide (the alpha texture is `max_texture_dimension_2d` x
height at 16 bytes/texel), and this adapter's real limit was never logged — **logging
`adapter.limits()` and instrumenting `Renderer::new`'s allocations is the concrete
next step**, and it is now a much shorter search than §13.6 left it.

> **Answered — see §20, and this paragraph's own next step was aimed one layer too
> low.** The limit is now logged (**16384**, never previously recorded) and it is not
> the cause: `Renderer::new` creates every one of those `max_texture_dimension_2d`-wide
> textures at **height 1**, i.e. 256 KB each, and the image atlas defaults to a 1×1
> placeholder. It was not vello_hybrid at all. `try_init_gpu` inherited
> `memory_hints: MemoryHints::Performance` from `..Default::default()`, which reserves
> 128 MB device-local and 64 MB host-visible suballocation blocks on first use
> regardless of need. The "single, fixed, screen-sized cost" this paragraph correctly
> identified in *shape* was fixed but not screen-sized — it was **block-size-sized**,
> which §20.2 shows by holding it flat at ~214 MB across a 5-node, a 6-node and a
> 152-node app and then moving it to 79 MB and 36 MB purely by shrinking the allocator's
> blocks. Setting the hint explicitly takes `GL mtrack` to 36 MB with no frame-time cost.

### 14.6 Known gaps

- **One device, one app, one ABI.** §14.5 is the Moto G54, arm64-v8a, the stock
  `zkite new` counter. Nothing exercised a `TextField`, a `List`, a `ScrollView` or a
  `Switch` through a `zkite build` native APK — all of those are verified only through
  `gpu_harness`'s own `build_apk.sh` builds, which stage the same artifacts by a
  different route. The counter is the app that proves the *switch*; it is not an app
  that exercises the *renderer*.
- **`zkite dev` is WebView-only.** The switch is `zkite build`'s. `dev_cmd.rs` hardcodes
  `dev.zkite.devshell`/`dev.zkite.devshell/.ZKiteActivity` and the reload-socket protocol
  that `phase_1/devshell` implements and `gpu_harness` does not — the native shell has
  no `startReloadServer`, no `pollHtml`, and no way to be handed a new `main.py` over a
  socket. So a user can *ship* on the native renderer but cannot *develop* on it, which
  is a real asymmetry against plan1.md §6's "the dev loop never builds an APK". Closing
  it is the same shape as §11.3's viewport channel: the render thread would need a
  reload command and a re-`_zkite_setup`, not new architecture. Not attempted.
- **`zkite build aab --backend native` is untested.** It goes through the same
  `stage_common`/`stage_native_libs` and should work — the AAB path differs only in the
  dex destination (phase_2/PLAN.md §6 M8) — but "should work" is what phase 2's own
  `classes.dex`-at-the-module-root bug already looked like, and no `bundletool validate`
  has been run against a native bundle. Also unresolved for either backend: phase 2 §9's
  multi-ABI AAB timing gap, which a 13.5 MB native cdylib per ABI will make worse.
- **The switch has never been exercised at dpr 1** — i.e. no emulator run. Both ABIs
  now have complete native artifacts (arm64-v8a and x86_64), so an x86_64 emulator run
  is reachable, but phase 0's own finding stands: an arm64 AVD needs an arm64 host, so
  the emulator tests a different ABI than any real phone here.
- **Nothing about the switch changes the §13.4 ledger.** It makes the native backend
  *reachable*, not *preferable*: 15.5 MB of binary, ~310 MB resident dominated by
  §13.3's unexplained 266 MB of `Graphics`, and ~490 ms more cold start are all
  unchanged and all still argue against flipping the default. The switch is what makes
  that a decision someone can make; it does not make it.
- **`phase_1/devshell`'s font staging is still the silent version** (§14.4).
- **`gpu_harness` is still called `gpu_harness`** in a shipped app's manifest (§14.2).

## 15. `zkite dev` on native, the default flip, and an honest inventory of what is missing

§14 made the native backend *reachable*. This section makes it the **default**, gives
it the dev loop it did not have, and — because those two together mean every new ZKite
app now lands on it — writes down what it still cannot do. §15.1's list is the most
useful thing in this section.

### 15.1 What is missing — the inventory

Ordered by how hard each blocks a real app, not by how hard it is to build. Everything
marked **(divergence)** works on the WebView backend and silently does not on native,
which makes it worse than a plain gap: the app renders, nothing errors, and the
behaviour is just absent.

#### Silent backend divergences — found by inspection this session, none previously recorded

1. ~~**Horizontal scrolling does not scroll (divergence).**~~ — **built and verified
   live on the Moto G54: see §27.** `zkite_renderer`'s scroll API is axis-generic
   (`max_scroll`/`is_scrollable`/`scrollable_ancestor` over a `ScrollAxis`), the render
   thread resolves a scroll target **per axis** and locks a drag to one axis at the
   slop crossing, fling is axis-aware, and `ScrollView(..., horizontal=True)` is the
   `zkite_py` front door. Two things came out of it that the entry below could not have
   predicted:
   - **`overflow-auto` is `Axis::Both`**, so every `ScrollView` and `List` that has
     ever existed already *declared* itself x-scrollable. Resolving a scroll target off
     the declaration would have handed every horizontal drag to a vertical list and
     then moved nothing. The walk filters on **room**, not declaration.
   - **A padded scroll container's end padding was never scrollable, on *either*
     axis** — a pre-existing `max_scroll_y` bug from §6.1 that only became visible
     when a fling could reach a horizontal bound and the last card sat flush against
     the screen edge. Fixed for both axes (§27.4).

   **`Carousel`/`PageView` are unblocked, not built** — they need snap-to-page, which
   is a different animation from fling decay and does not exist. The original entry
   follows as written.

   `twcore` parses
   `overflow-x-auto`/`-scroll` (`phase_0/spike3_twcore/src/parse.rs`) and taffy honours
   the style, but `zkite_renderer` exposes only `max_scroll_y`/`is_scrollable_y` and
   `render_thread`'s gesture/fling code is y-only by construction (§6.3 says so about
   fling; it is true of dragging too). So a horizontally-scrollable container clips its
   content and never moves. **This is the blocker for `Carousel`/`PageView`**, and it is
   the one gap on this list that is load-bearing for a whole widget class rather than
   for polish.
2. **`dark:` never activates (divergence).** The variant parses to a `StateFlags::DARK`
   bit (`twcore/src/variant.rs`), but every `zkite_renderer` resolve passes
   `NodeState::default()` — nothing reads the system setting, so the bit is never set.
   Blink honours `prefers-color-scheme`, so a `dark:` class works on one backend and is
   inert on the other. Nothing in §9's conformance harness can see this: it renders one
   state, light, at a fixed size.
3. ~~**Gradients do nothing (divergence).**~~ — **built and verified live on the Moto
   G54, on the GPU path and in a real `zkite build apk` app: see §38.** Linear gradients
   in all eight keyword directions (both v4's `bg-linear-*` and v3's still-live
   `bg-gradient-*` spellings), bare and arbitrary angles, `from-*`/`via-*`/`to-*` with
   opacity modifiers and explicit positions, and the four interpolation modifiers. Four
   conformance fixtures against real Chromium read **0.00%** on the loose column at both
   dprs, and the device's own midpoint of a yellow → blue ramp is **(141, 162, 173)**
   against Chromium's **(140, 161, 173)**.

   Two things came out of it the entry below could not have predicted. **The hard part
   was neither layer it names**: it was that a gradient stop cannot use `twcore`'s
   palette hex at all. Tailwind v4 states colours in OKLCH and **95 of the 286 are
   outside the sRGB gamut**; Chromium interpolates the *unclipped* colour and clips only
   at display time, so feeding it the clipped hex put the ramp **68/255** off — the
   palette now carries both forms from one generated source (§38.3). And
   `background-color` and a gradient **coexist** rather than replacing each other, which
   an `enum Background { Solid, Gradient }` cannot express and which is the shape a
   first draft reaches for (§38.1).

   **Still missing: `bg-radial` and `bg-conic`** (unparsed, and loudly so — §38.5), the
   `/srgb` modifier (divergent by 24% for a proven reason in vello, §38.5), and
   `from-current`. The original entry follows as written.

   `twcore` has **zero** gradient parsing —
   no `bg-gradient-*`/`bg-linear-*`, no `from-*`, `via-*` or `to-*`. Unknown classes are
   silently dropped, so `bg-gradient-to-r from-cyan-500 to-blue-500` resolves to a plain
   background natively and renders correctly on the WebView (real Tailwind). It is two
   layers deep, not one: even with parsing, the paint path only fills solid colours, so
   a gradient brush has to exist in `zkite_renderer`'s `Paint` and in both paint backends
   before a parsed gradient could draw. Reported by the user from a real app, not found
   by any test.
4. **Emoji do not render natively (divergence), and it is also two problems.**
   `stage_fonts.sh` bundles six DejaVu faces (sans/serif/mono x regular/bold), none of
   which has emoji coverage — *and* there is no colour-glyph support anywhere in the
   paint path (no `COLR`, `CBDT`, or bitmap-glyph handling in `zkite_renderer` or either
   backend). So bundling Noto Color Emoji would not be sufficient on its own. The
   WebView gets this free from Android's system emoji font. Also reported from a real
   app. This interacts with §9.7's open "which typefaces should ZKite bundle" decision:
   whatever is chosen has to include an emoji strategy, and "read `/system/fonts`" would
   solve emoji and the OEM-divergence problem together.
4b. ~~**`change` and a bubbled `tap` fire in *opposite orders* on the two backends
   (divergence).**~~ — **closed, see §32.3.** The owner decided the contract: **the
   bubbled `tap` runs first, and native matches the WebView.** `zkite-runtime.js` is
   unchanged and `render_thread.rs::dispatch_tap` moved to agree with it; the payload is
   still computed from the pre-tap build, which is what makes it faithful rather than
   merely reordered (the WebView reads `el.checked` from a DOM Blink toggled at gesture
   time and that no re-render has reached yet). Verified on the Moto G54 against a
   rebuilt pre-§32 control: `dispatch order: change tap` before, `tap change` after,
   same fixture and same gesture.

   **And the case that made it urgent no longer needs the answer**: a labelled control
   (§32) absorbs the tap, because both runtimes stop at the nearest `tap`-bound
   ancestor, so there is never a second handler to order. The original entry follows as
   written.

   New, and **measured rather than inferred**: §26.7 recorded the
   ordering as unverified; §27.6 ran it in real Chromium via the webruntime e2e suite
   and it is a real difference. A tap on a `Switch` inside a row with `on_tap` fires
   **the row's `tap` first** on WebView (`zkite-runtime.js` listens in the *capture*
   phase, which runs root → target) and **the switch's own `change` first** on native.
   Both handlers always run — that half is agreement. It only bites when both write the
   same `@store` field, where the last writer wins and is a different one per backend.
   Pinned in `phase_1/webruntime/test/e2e.mjs` in the direction it actually goes, so a
   silent flip fails the suite. **Choosing the contract is an open decision, not an
   oversight** — see §27.6.

   > **This stopped being theoretical (§30.8), and is now resolved (§32.3).** The most
   > natural way to make a checkbox's *label* tappable — `on_change` on the control plus
   > `on_tap` on the row, both writing the same field — made a tap on the box net
   > **`False` on native and `True` on the WebView**, from identical user code. It was
   > reachable from an extremely ordinary settings screen and it failed silently on the
   > default backend. It is what forced the contract decision above, and the label prop
   > removed the shape that provoked it.
4c. ~~**Every glyph in the tree painted above every background in it
   (divergence).**~~ — **fixed, see §31.1.** The native paint path ran three whole-tree
   passes (all boxes, all icons, all glyphs), which reproduces DOM stacking for a node
   against its own *ancestors* and inverts it across sibling subtrees. Nothing opaque
   could cover anything textual. It broke `Stack` — a shipped widget whose only purpose
   is overlaying children — and it is why §15.3's error overlay could not hide the
   waiting screen behind it. Measured at **2.77% against real Chromium** on a two-child
   fixture, `max_channel_delta` 255; **1.06%** after. The conformance harness could not
   see it because the `stack` fixture overlays two opaque rectangles with **no text in
   either**, which is the one shape immune to it (§31.7).

   **Still open, and now named rather than latent:** there are no stacking contexts, no
   `z-index` and no opacity groups. What exists is document order. §31.8.
4d. ~~**A `Stack` child with no explicit width did not fill the stack
   (divergence).**~~ — **fixed, see §31.2.** `Stack`'s HTML wrapper is a grid item
   (`display:block`), so its child fills it; the tree wrapper carried no display class,
   so taffy's own `flex`/`row` default sized the child to its content. **Third instance
   of the divergence `ScrollView`'s implied classes already document** — `twcore` has no
   block layout mode, so every classless container needs `flex flex-col` stated. 55.19%
   → 2.08%.
4e. ~~**One transient swapchain error permanently downgraded a running app to the CPU
   rasteriser.**~~ — **found and fixed, see §39.** Reported by the owner as "scrolling
   isn't smooth". `paint_gpu` returned `Option<()>` whose `None` meant both "this device
   has no GPU path" and "the swapchain hiccuped this frame", so a single
   `get_current_texture: Validation` mid-scroll sent every subsequent frame through
   `paint_cpu_fallback` — ~23–29ms per frame against the GPU path's ~11 — with a comment
   claiming reconfiguration happened "on the next `Resize`", which during a scroll never
   arrives. Now: reconfigure and retry, and the CPU fallback runs only when there is no
   usable GPU path at all. Verified on the Moto G54 and in a shipped `zkite build apk`
   app: **0** fallback frames across 14 swipes, frame-interval avg back to ~17ms.

   **What remains is cost, not correctness, and it is measured in §39.5**: `sync` spikes
   to 10–14ms whenever a virtualised window moves (Python rebuilds the row window), the
   steady state is ~15.5ms of work against a 16.67ms budget, and a finger-down drag is
   not vsync-paced the way a fling is.

   **Two of those three have since been corrected by measurement, in §42.** The steady
   state is **~12.8ms**, not ~15.5ms — §40 improved it and nothing re-measured it until
   now. And "a finger-down drag is not vsync-paced" is **false as a problem statement**:
   Android delivers roughly one `TouchMove` per refresh, already aligned, so the loop is
   paced by the input stream. Pacing it a second time was built and measured and is a
   **regression** (17.37 → 18.08ms), so it is not kept. What survives §39.5's list is the
   `sync` spike's *size* (4g) and overdraw, which is still unmeasured.

4f. **The `sync` spike's *frequency* — halved and then some, see §40; its *size* is
   untouched.** `window_would_change` asked whether `compute_window` would return a
   different range, and `compute_window`'s `first` moves on every row boundary
   regardless of `overscan` — so the window was rebuilt once per row at **every**
   `overscan` value (36 rebuilds per 2000dp at `overscan` 0 *and* 32, with rows built
   per rebuild growing 17 → 81). §39.5's own first-listed mitigation, "a larger
   `overscan`", would therefore have kept the spike count identical and made each spike
   bigger. Asking about **coverage** instead — keep the built window until the viewport
   reaches its edge — makes a rebuild happen once per `overscan + 1` rows. Measured on
   the Moto G54, matched arms: frames re-running `view()` **53.6% → 8.8%**, `sync`
   average **4.55 → 0.88ms**, frame-interval average **18.98 → 17.48ms**. What is left
   is the *size* of a rebuild — still a full `view()` + JSON round trip, still ~10ms —
   which §39.5's other two candidates attack and §40 did not.

   **§43 then found that 44% of a rebuild's cost was a bug, not work, and the
   relationship §42 measured is now reversed.** `runtime.py` imported `json` before it
   installed the finder that makes `_json` importable, and `json/encoder.py` binds its C
   accelerator once at import time — so every ZKite app on **both** backends had been
   running CPython's pure-Python JSON encoder *and decoder* since §11.8. Fixing the
   import order took a rebuild's Python cost **9.09 → 2.09ms** and late frames **13.8% →
   4.4%**, and a rebuild is now essentially never why a frame is late (1 of 13, against
   §42's 29 of 41). What is left of §42's model: the per-row cost is real and linear, but
   it was fitted over a cost that was mostly a defect, and the largest remaining
   component of a rebuild frame is the **second `layout_frame()` pass** (`relayout`,
   1.79ms) that no prior section counted. §43.5.

   **§44 then removed that second pass and got nothing for it.** Reporting viewports
   before the first layout takes `relayout` to **0.00ms** — the mechanism §43.5 named is
   real and is now pinned by a host test — but total frame work moves 12.438 → 12.454ms,
   because a CPython call costs **21% more** at the top of a frame than in the middle of
   one (control: the fixed-work `poll`, **+36%**). Reverted. What survives of Queue H's
   remaining work is the row build, and **overdraw**, which is still unmeasured and is
   still 60% of the frame.

   **§46 measured overdraw and refuted it, and the phase it lived in is mostly not
   ours.** `present` was one `Instant` around three unrelated calls; split, it is
   `render` 0.67 / `finish` 1.66 / `submit` 3.63 / `swap` 0.51ms. Removing **every**
   opaque row background — 1.6 screens of coverage per frame, on an arm with an
   identical node count — moves `render` by **0.000ms** and `present` by −0.202ms,
   while moving `scene` by 0.500ms in the same pair. So overdraw costs ~0.5ms/frame, all
   of it in our own scene-building walk, and **nothing at all** in the 55% phase §39.5
   attributed it to. `finish`+`submit` is **5.29ms = 45% of all frame work and invariant
   to everything the app draws**. Queue H has no large item left.

   **§42 located it precisely and priced the fix.** The dropped frames *are* the rebuild
   frames: 89% of rebuild frames run late against 2.6% of everything else, and 71% of
   all late frames are rebuilds. Rebuild cost is **per-row, not a fixed CPython/JSON
   toll** — `sync ≈ 0.38ms x rows` with an intercept indistinguishable from zero,
   measured at 25/33/49 rows and with the third point predicted before it was taken. So
   **building only the rows that entered** (~5 instead of 25) predicts ~1.2ms against
   ~3.9ms of headroom, and is the one remaining candidate with arithmetic behind it.
   Raising `overscan` is **not** it: §42.4 measured 4/8/16 and the frequency-for-size
   trade prices out flat (17.37 / 17.40 / 17.76ms), so the default stays at 4.
4g. ~~**Fling momentum had exactly one gear.**~~ — **fixed, see §41.** Owner-reported as
   "flat acceleration". Travel after release was `v0 * FLING_TAU` with a *constant*
   `FLING_TAU`, i.e. exactly linear in release speed, so a gentle flick overshot and a
   hard one could not cross a long list. `tau` is now `fling_tau(v0)`, making distance
   go as `v0^1.74` — Android `OverScroller`'s own spline exponent, matched to ~1% over
   600–6000dp/s. Device-verified with matched injected flicks: the same slow flick
   lands on item 15 → **12**, the same hard flick on item 41 → **61**. The 3000dp/s
   calibration point is unchanged by construction.
5. **The system font-size accessibility setting is ignored by *both* backends.** No
   `fontScale`, `scaledDensity` or `setTextZoom` anywhere in any shell. A user who
   enlarges text system-wide sees no change in a ZKite app. This is a Play
   quality/accessibility bar item and is the same class of bug as the density one §13.1
   fixed — `density` was read and `fontScale`, sitting next to it in the same
   `DisplayMetrics`, was not.
6. ~~**`Switch` and `Slider` render but do not respond to touch on native
   (divergence).**~~ — **confirmed on a device, fixed, and re-verified live: see
   §26.** Both now respond, the `change` payload arrives as a real `bool`/`float`
   (it was a *string* on **both** backends, and `bool("false")` is `True`, so a
   WebView switch could be turned on and never off), and a drag on a slider inside a
   `ScrollView` no longer scrolls the list. Two corrections to the entry below, kept
   because the diagnosis was otherwise right and it is what the fix was built from:
   a tap on a `Switch` did not do *nothing*, it dispatched to the nearest `tap`
   ancestor instead — and that bubbling is conformant (`zkite-runtime.js` does the
   same), so the divergence was only ever the missing `change`. The original entry
   follows as written.
   
   Reported by the owner from a real app — "they were there but not
   working" — and confirmed by reading both sides, **not yet by a device run.** They
   *paint* correctly (§9.4 tuned both against the conformance harness) and they
   register `on_change` as a `"change"` event, but `render_thread.rs` dispatches
   `"change"` from **exactly two sites and both are inside the IME/text path**
   (`self.focused`, `focused_text`); a touch anywhere else goes through
   `dispatch_tap`, which only ever sends `"tap"`. A `Switch` handles `"change"` and
   not `"tap"`, so the dispatch is skipped and nothing happens — no error, no repaint.
   `Slider` needs more than a tap-to-toggle mapping: there is **no drag-to-value code
   at all**, and a grep for `switch|slider|thumb|track` across `gpu_harness/src/`
   returns nothing outside comments and a11y. The WebView backend gets both free —
   real `<input type=checkbox>`/`<input type=range>` with
   `document.addEventListener("change", …)` in `zkite-runtime.js`.

   **This leaves the native backend with exactly one working form control
   (`TextField`),** which matters more than its position on this list suggests: §15.1
   already notes there is no `Checkbox`, `Radio`, `Select` or picker, so "the form
   vocabulary is `TextField`, `Switch`, `Slider`" (plan1.md §14.3) is really
   `TextField` alone on the default backend. Nothing in §9's conformance harness can
   see this — it renders one state and never touches anything.

   > **Closed (§26).** The form vocabulary is now the three plan1.md §14.3 always
   > claimed. The harness still cannot see interaction and §26 does not pretend it
   > can; what it gained is a `slider_range` fixture covering value → geometry at the
   > ends, which is the half a rendering harness *can* check — and which immediately
   > found a thumb that left its own track at maximum.
   >
   > **And doubled (§30):** `Checkbox` and `Radio` now exist on both backends, on the
   > machinery this section built, so the working vocabulary is five. The harness's
   > blindness is unchanged and §30 does not pretend otherwise — three of its six
   > device cases are about the *second* tap, which no static fixture can ask.

7. ~~**`rounded-sm` is half the radius it should be, and `rounded-xs`/`rounded-4xl` do
   not render at all (divergence).**~~ — **found and fixed in §36.** `twcore`'s
   `--radius-*` scale was still v3's, and v4 shifted every name in it one step, exactly
   as §28.1 found it had done to the shadows. So `rounded-sm` drew a 2dp corner
   natively against the WebView's 4dp, on every background, border, ring and shadow the
   node carried, and the two names v4 added at either end were dropped as unknown
   utilities — a square corner where the WebView rounds one. Found by auditing the
   *whole* remaining theme table against real Chromium rather than by hitting it in an
   app (`phase_1/webruntime/test/probe_theme_defaults.mjs`), which is also how §36
   settled the question §28.6 was actually worried about: **the spacing scale is
   correct**, and always was.

#### Missing widgets

plan1.md §8 names 15 for v1; 12 exist. Still missing from its own list:

- ~~**`Nav`** — and with it **there is no navigation or back-stack handling anywhere,
  in any shell**~~ — **built, see §16.** `Nav`, a `Navigator` stack store, and Back
  handling in all three shells, verified live on both backends including the real
  back gesture. Deliberately a three-layer substrate (back chain / `NavigationHost`
  protocol / stack policy) so a tab or modal navigator or a URL router can be added
  without touching it. Screen transitions are still unanimated — there is no animation
  driver (below), and animating would cost the "only the top screen is built"
  property the design is built around.
- **`Sheet`** — modal present/dismiss, deferred in phase 1 §3 for its own milestone.
  Now unblocked in the sense that matters: it is a layer-2 `NavigationHost` plus a
  back-chain entry, and §16's chain is already innermost-first.
- **`Native(...)`** — plan1.md §8's escape hatch for embedding a real Android view.

Beyond that list, a production app will immediately want: ~~`Checkbox`, `Radio`,~~
**both built, see §30** — verified live on the Moto G54, with Android's own
accessibility layer reporting real `android.widget.CheckBox`/`RadioButton` roles. The
form vocabulary is now five controls, not three. ~~`Select`/picker~~ — **built, see
§33**, reported by Android as a real `android.widget.Spinner`, so it is **six**. Still
missing: `DatePicker`/`TimePicker`,
`Dialog`, `Toast`/snackbar, `ProgressIndicator`, and `Carousel`/`PageView` (no longer
blocked on divergence 1 — §27 built horizontal scrolling — but still needing
snap-to-page, which is a different animation from the fling and does not exist).
`Dialog`, `Sheet` and `Toast` are each now one composition over §33's `Overlay`.

> **`Select` — built, see §33.** The self-drawn popup, verified live on the Moto G54 on
> **both** backends. The thing worth carrying: it needed **no renderer change at all**.
> "Paints above the whole app" is a property both backends already had — `zkite_renderer`
> paints in document order and hit-tests it in reverse, the DOM does the same for
> positioned elements in tree order — so a layer is an ordinary subtree that `app.App`
> emits *last*. `render_thread.rs`, `zkite-runtime.js` and `twcore` are all untouched.
> The layer is public as **`Overlay`**, so `Sheet`/`Dialog`/`Toast` are now compositions
> over something that exists rather than four separate projects.
>
> This also **narrows §31.8 rather than closing it**: the consumer it was waiting for
> turns out not to need stacking contexts, `z-index` or opacity groups. A `z-index` in a
> class string still parses to nothing.
>
> One word is load-bearing and was settled by driving real Chromium, not by reading:
> the popup's panel carries `relative`, because CSS paints positioned elements above
> non-positioned ones regardless of tree order — without it the panel renders correctly
> on native and is **buried under its own scrim** on the WebView. §33.1.

> **Labels for `Checkbox`/`Radio`/`Switch` — built, see §32.** All three take a label as
> content, and `indicator=False` drops the built-in box for an MCQ or segmented control
> while keeping the role, state and accessible name a tappable `Row` would have thrown
> away. Verified live on the Moto G54, including Android reporting a labelled control as
> `android.widget.CheckBox` whose *name is the label*.

#### Missing rendering features on native

- ~~**`Image` and `Icon` paint an empty box of the correct size** (§9.2 item 4) — no
  bitmap decode, no vector rendering, no network image loading or caching. An app with
  a logo shows a blank rectangle.~~ — **`Icon` built, see §18; `Image` built, see §35.**
  Both paint on both native paint paths. `Image` decodes PNG and JPEG from the app
  directory or a `data:` URI, caches by `src`, and honours all five `object-fit` modes;
  verified live on the Moto G54 on the GPU path, the CPU fallback path, and as real
  `zkite build apk` apps on **both** backends.

  Three things came out of it this entry could not have predicted. **It was never a
  native-only gap** — the WebView shell loads from `file:///android_asset/webruntime/`,
  so a relative `src` resolved to a nonexistent asset and images were broken there too
  (§35.1). **`zkite build` was dropping the file before the zip** — `copy_app_source`
  staged `*.py` and nothing else, which is not what §18.6 said the problem was. And a
  real divergence surfaced underneath: an authored `object-contain` was honoured on
  native and overridden by the implied `object-cover` on the WebView, because the DOM
  resolves by CSS cascade rather than by attribute order (§35.4).

  **Still missing: network image loading and intrinsic sizing** — deliberately deferred
  rather than half-built, see §35.6. `no network image loading or caching` above stands.
- ~~**Borders, shadows, rings are not painted at all.** `twcore` resolves them into
  `Paint`; no backend draws them (§3, §5).~~ — **built, see §28.** All three paint on
  both native paint paths, with geometry shared by all four paint consumers the same
  way `icons.rs` shares its own (§18.3). Verified live on an x86_64 emulator **and now
  on the Moto G54 (§29.8)** — real PowerVR silicon at density 2.5, every claim
  re-measured as arithmetic out of the screenshot. **One real divergence survives, and
  that run is what found it:** `shadow-2xl` is ~1.75x too dark at the card edge against
  real Chromium, and the conformance harness scores it **0.00%**.

  Two things came out of it that this entry could not have predicted. **`twcore`'s
  border, shadow and ring defaults were all still Tailwind v3's** — v4 makes both
  `border-color` and `ring-color` default to `currentColor` (v3: gray-200 and
  blue-500), drops the default ring width from 3px to 1px, renames the entire shadow
  scale (v4's `shadow-xs` is v3's `shadow-sm`) and makes most of it two-layer. §9.6's
  own known-gaps entry predicted exactly this class and nobody had checked, because
  **nothing painted any of it**. And §9.6 had already touched the ring default and
  fixed the wrong half: it updated blue-500's *hex* to v4's value while v4 does not
  default a ring to blue-500 at all. See §28.1.

  Second: **the conformance harness cannot see whether a shadow is correct.** Removing
  the CSS-blur-to-std-dev conversion changed the render and moved the number by
  nothing, because Tailwind's whole shadow scale is fainter than `CHANNEL_TOLERANCE`.
  §9.5's strict column sees an *absent* shadow; nothing sees a wrong one. §28.4.
- **`decoration` (underline / line-through)** is deliberately unpushed — `GlyphRun`
  carries only font/size/glyphs, so neither paint backend has anything to draw the line
  from (§9.7). Needs paint support, not a fifth `push_default`.
- **`italic` is pushed but no italic face is registered**, so it renders upright (§9.7).
- **`text-align` does not cover content-sized wrapped text** (§9.7).
- **No overscroll / rubber-band at a scroll bound** (§5 item 4).
- ~~The `Slider` thumb is left-edge aligned rather than centred~~ — **fixed, §26.4.**
  The `-translate-x-1/2` `twcore` cannot express is emitted as a plain pixel margin
  instead, since the percentage is known at tree-build time; the thumb is now
  pixel-identical to Chromium's at 0/50/100.
- **No animation or transition support anywhere** — `twcore` parses no `transition-*`
  or `animate-*`, and there is no animation driver beyond the fling. Checked by grep
  across `twcore`, `zkite_py` and `zkite_renderer`.

  > **Partly addressed, §45; wired up and usable on the native backend, §47.** The
  > driver's time-domain core is `zkite_anim` (easing, springs, tracks, timeline,
  > interruption policy — 47 tests, host-only), the two channels it drives paint
  > (§45.1), and **all three missing layers now exist**: an `anim` attribute on a node,
  > an `Animator` in `render_thread` sampled once per frame, and `zkite.Anim` as the
  > author surface. An author can write
  > `anim=Anim.enter(opacity=(0,1), translate_y=(16,0), ms=220)` with
  > `on_animend=` for the completion.
  >
  > **Still a divergence, and deliberately a loud one: the driver is native-only.**
  > The WebView port is §45.3's layer 4 and is not built, so a declaration animates on
  > native and renders statically on the WebView with no `on_animend`. `zkite_py` prints
  > one line to stderr per process saying so, which item 1d's fix (§47.1) is what
  > carries to logcat. `transition-*`/`animate-*` still parse to nothing on purpose
  > (§45.7), and `rotate-*`/`scale-*` are now **rejected by name** by the animation
  > surface rather than silently accepted.
  >
  > The architectural result worth carrying: an animation **cannot** re-run `view()`
  > per frame — priced off §44.3's own table at ~16.03ms of a 16.67ms budget (§45.2).
  > **Verified on the x86_64 emulator only (§47.7); the Moto G54 run is outstanding**,
  > which is the first thing to do next.

- **`opacity-*` and `translate-*` were parsed into `ComputedStyle` and painted by
  nothing on native (divergence).** Filed and **closed in the same session, §45.1** —
  recorded here because the *shape* is the point: both compiled to real fields
  (`paint.opacity`, `ComputedStyle::transform`) since phase 0, the WebView backend
  painted both correctly via the real Tailwind browser build, and **no conformance
  fixture covered either**, so §9's harness could not see it and WIDGETS.md documented
  `opacity` as working on both. Measured against real Chromium at 42.18% / 78.17% /
  15.19% of pixels wrong before the fix and 0.01% / 0.00% / 0.10% after, with all 35
  pre-existing fixtures byte-identical across the pair.

- **`rotate-*` and `scale-*` are still parsed and silently dropped on native** (§45.7).
  Same divergence class as the entry above, and the residue of it: a translate maps onto
  the per-node offset channel `positioned` already had for scroll and IME pan, while a
  rotate or a scale needs a real affine over an axis-aligned rect list *and* a
  `hit_test` that can invert it. They work on the WebView backend. They parse, so the
  unknown-class report does not mention them.

#### Missing platform integration

- ~~**Nothing can push a frame from Python — every frame is *pulled*.**~~ — **built on
  both backends, see §34.** `App.on_render`/`on_render_tree` are registered by
  `runtime.py` against a Rust-injected `_zkite_native_frame_ready`; native sends
  `Command::FrameReady` on §15.2's existing channel and the WebView shell makes a real
  cross-thread JNI call (`AttachCurrentThread` → `ZKiteActivity.onFrameFromPython` →
  `runOnUiThread` → `evaluateJavascript`) rather than the poll phase 1 §4 avoided, so a
  shipped app is idle between frames. Verified live on the Moto G54 on native (a
  `threading.Timer` chain moved a counter 3 → 10 → 17 with no touch at any point) and,
  after a mid-session USB disconnection sent the work to an emulator and back, re-run on
  that phone end to end for **both** backends and for real `zkite build apk` apps, with a
  discriminating negative control measured from screenshots (§34.7).
- **`on_change` on a `TextField` fires at different times on the two backends
  (divergence).** Filed 2026-08-09 (§34.1), found while fixing `on_input`, and
  **pre-existing since phase 3 began** rather than introduced. Measured in real
  Chromium: typing `Ken` emits `input` x3 and **no** `change`, and `change` arrives once
  on blur. Native has dispatched `change` **per keystroke** since §7. So `on_change`
  means "live typing" on native and "the user finished" on the WebView, from identical
  user code. `on_input` now works identically on both (§34.1), which is what a typeahead
  actually wants; **retiming `change` is an owner decision** written up in §34.10,
  deliberately not guessed at, because every direction breaks something real.
- **Network access — the artifact was right, the trust store was not.** ~~Never
  exercised~~ — **a real HTTPS request now works, verified live on the Moto G54
  (§34.5, §34.7):** `urllib.request.urlopen("https://example.com")` from a worker thread
  returns `ok 200` and a body. Two things had to be fixed and only one was predictable.
  `zkite build` generated a manifest with **no `<uses-permission>` and no way to add
  one**, so a shipped app could not reach the network at all; `[tool.zkite.android]
  permissions = ["INTERNET"]` now exists (§34.4). And the vendored OpenSSL's compiled-in
  `/usr/local/ssl/cert.pem` does not exist on a phone — but pointing `SSL_CERT_DIR` at
  Android's own store **does not fix it**, because Android's CA filenames use the old
  MD5 `X509_NAME_hash_old` scheme that a modern `by_dir` lookup cannot find. `runtime.py`
  concatenates the device's own certificates into a cached bundle instead (§34.5).
- **A `Text` node wider than the viewport is absent from the accessibility tree.** Filed
  2026-08-09 (§34.7), pre-existing. It paints; a screen reader never hears it.
- ~~**`zkite build apk` produced apps that could not resolve a hostname.**~~ — **fixed,
  §34.8.** `prune_stdlib.py` dropped `stringprep`, which `encodings/idna.py` imports and
  which every `getaddrinfo` goes through, so a built app's first request died with
  `LookupError: unknown encoding: idna`. **The harness kept it**, so no device run in this
  document before §34 had ever exercised the stdlib the shipped path actually produces.
- ~~**`zkite build apk` stages prebuilt shells from `<shell>/artifacts/` with nothing
  checking they match the current source** (§34.8). It silently shipped a shell left
  behind by a negative-control build, and a `strings` check could not tell them apart.~~
  — **fixed, see §37.** The build scripts write `artifacts/provenance.json` recording a
  digest over the source inputs that fed them (derived by `cargo metadata`, so a crate
  reached two deep is included), and `zkite build` re-derives it and **fails** on a
  mismatch. The check is on the *source*, not the bytes, because both prior incidents
  produced a well-formed artifact from the wrong build.

  **A hole in it, found and closed by §43.6:** the source set came from `cargo metadata`,
  which knows the *package* graph, so a file pulled in by `include_str!` from outside every
  crate directory was invisible to it. `phase_1/devshell/src/runtime.py` — most of the
  framework's Python glue — is exactly that, so editing it produced a different `.so` under
  an **identical** source digest and `zkite doctor` said "verified". The recorder now scans
  the collected Rust sources for `include_str!`/`include_bytes!` and adds the in-repo
  targets it resolves (43 → 44 files for the native shell, and the WebView shell picked it
  up too). Derived, not hand-listed, on the same argument the crate set already made.

  **It found a third instance on its first real run, sitting in the tree at `HEAD`**
  (§37.1): the staged native shell was built twelve hours before §36's commit, so
  `zkite build apk --backend native` would have shipped an app whose radius scale was
  still v3's — the exact bug §36 fixed. Proved by recovering the old `.so` from a
  devshell APK and reproducing it byte-identically with only §36's two source files
  reverted. `classes.dex`, `fonts.zip` and the whole WebView shell were current.
  `fonts.zip` is recorded as **not determined by repo source** rather than verified,
  because `stage_fonts.sh` reads the host's `/usr/share/fonts`.
- **Without `INTERNET` a request hangs rather than raising**, past its own `timeout=`, and
  while it hangs the push path stops delivering frames (§34.11). Named, not diagnosed.
- ~~**Network access has never been exercised.**~~ — **exercised and working, see
  §34.5/§34.7.** Kept as a pointer because the entry above now carries the detail: the
  `.so`s shipping was never the binding constraint, the manifest permission and the CA
  trust store were.
- **A stdlib C accelerator is silently lost if its module is imported before
  `_ZKiteNativeExtensionFinder` is installed.** Found and fixed for `json` in §43, where it
  had cost **44% of every virtualised-list rebuild** on both backends since §11.8, with no
  error and no log line. `runtime.py` now installs the finder above every non-essential
  import, and `phase_1/devshell/tests` pins it with a negative control. **Still open:
  anything imported during interpreter startup, before `runtime.py` runs at all**
  (`encodings`, `codecs`, `site`). No such module has a large accelerator shipped as a
  `libpymod_*.so`, so this is believed harmless and was not measured; the robust form is
  to install the finder from Rust before `RUNTIME_GLUE` executes. §43.8.
- **Autofill** — phase 0 flagged it and a11y as "the same missing piece" (a virtual
  semantics tree); §8 built the tree, nothing consumes it for Autofill.
- **AccessKit scroll actions** (`ACTION_SCROLL_FORWARD`/`BACKWARD`) and in-field
  text-traversal actions — §8.6, and §11.7 makes them load-bearing: a screen reader sees
  only the materialised window (48 nodes of 10,000), so TalkBack's linear navigation
  stops at the window edge.
- **The interactive a11y action round trip has never been driven live** (§8.5).
- **App lifecycle and state restoration after process death.** Android routinely kills
  backgrounded apps; nothing saves or restores `@store` state, so the app restarts from
  scratch. Not a native-only gap — it affects both backends and nothing in phases 1–3
  addresses it.
- **RTL and i18n/l10n**, **reduced-motion**, **deep links**, **permissions**, and the
  phase-4 plugin set (camera, location, notifications, file picker, share).

#### Missing in the toolchain

- **`zkite build aab --backend native` is untested** (§14.6).
- **No CI for any phase-3 crate, and no per-fixture conformance gate** (§9.3, §13.6).
- **Only dpr 1 and 2 exercised**; real phones run 1.5, 2.625, 3, 3.5 (§13.6).
- **The keyboard matrix beyond Gboard** (§7.13).
- ~~**`zkite build` runs on exactly one machine — this checkout (§21.4 item 1, §23.2).**~~
  — **fixed, see §29.** All **five** compile-time anchors (§23.2 found four; §24 added
  the fifth on purpose) now go through one `payload::root()` with the search order
  §23.7 step 1 specified: env override, `zkite-payload/` beside the binary, the
  compile-time checkout **last**. Verified by staging a payload outside this checkout
  and building an APK from an installed binary that never read the checkout — proved
  decisively rather than plausibly, via a sentinel file that appears in the APK only on
  the payload path. `zkite doctor` now reports which root it resolved, which §23.7 named
  as the fight; doing that immediately exposed a real bug (§29.2: `doctor` exited 1 on
  §23.4's own recommended native-only default wheel, because the fatal/informational
  backend split was still hardcoded the pre-§15.4 way). **Still open: no wheel is
  built, and nothing was run on Windows, macOS or a second physical machine** — §23.9's
  caveats are narrowed, not closed (§29.7).
- ~~**`version_code` is identical across per-ABI APKs** (§17.4 item 2)~~ — **fixed, see
  §29.3.** `abi_version_code(base, abi)` is `base * 10 + rank` with permanent per-ABI
  ranks, the manifest is built inside the per-ABI loop, and a build-time assert catches
  Play's ceiling. Confirmed by `aapt2 dump badging` reading the real binary AXML (13 and
  14 from base 1) and by Android's own package manager after install. **§17.4 item 1 is
  untouched:** it still defaults to 1, never auto-increments, and is not derived from
  `[project] version`.
- ~~**The Dev Shell pins the framework version, and nothing detects it**~~ — **fixed,
  see §31.6.** The `hello` frame now carries a content hash of the shell's own
  `zkite/*.py`, which `zkite dev` compares against the payload's; a mismatch prints the
  rebuild command instead of leaving the user with a bare `ImportError`. A version
  *string* would not have worked — see §15.3's own correction.
- ~~**Nothing packages the build payload into a distributable** (§21.4 item 2)~~ —
  **two real wheels now exist and install, see §37.4.** `build_wheels.py` produces
  `zkite_payload-…-py3-none-any.whl` (**21.5 MB**, host-independent) and
  `zkite_cli-…-py3-none-linux_x86_64.whl` (**2.9 MB**); `pip install` both into a fresh
  venv, and `zkite new` + `zkite build apk` from a project in `/tmp` produces a signed
  40.5 MB APK with the checkout never read. §23.3's ~19 MB / ~6 MB estimates were close
  and the CLI half was pessimistic. **The distribution *names* remain §23.8's open owner
  decision** — nothing in the resolver keys off one, so it is a flag and a
  `Requires-Dist` line whenever it is made.

  **Still open:** `linux_x86_64` is not a manylinux tag, so nothing here could be
  uploaded to PyPI as-is; only one of §23.7 step 4's five platforms was built; the
  *framework* wheel (`zkite` itself, the pure-Python half `zkite new`'s template depends
  on) is not built by anything; and **§23.7 step 3's release pipeline still does not
  exist** — what exists is the substrate it would call. §37.6.
- ~~**A custom app icon cannot be set at all** (§21.3)~~ — **built, see §22.**
  `[tool.zkite.android] icon` is read, resolved, PNG-validated and verified live on the
  launcher. **Adaptive icons remain blocked** on `pack`'s resource-qualifier gap
  (phase_0 §7), now with a device observation rather than an argument behind that:
  launchers mat a legacy PNG onto a white plate instead of letting it fill the icon mask
  (§22.2).
- ~~**`zkite dev` is a dead end without `--devshell-apk`**~~ — **fixed, see §24.** It
  now probes the device, launches an installed shell, installs a prebuilt one when it
  finds one, and exits with an actionable message when it cannot. **Still open: nothing
  downloads a Dev Shell**, so the auto-install path only works from this checkout —
  §24.3 recommends download-on-demand for this artifact specifically (39 MB, needed once
  per device) against ship-in-wheel for the build payload.

### 15.2 `zkite dev` on the native backend

**Status: built and verified live on the Moto G54, including state preservation across
a real edit.**

§14.6 named this as the asymmetry the switch left behind — you could ship on native but
not develop on it. `dev_cmd.rs` hardcoded `dev.zkite.devshell` and the reload-socket
protocol only `phase_1/devshell` implemented.

**It turned out to be smaller *and* structurally simpler than the WebView original**,
which is worth recording because it is the opposite of the expected direction. The
WebView shell has no way to push a frame from a background thread — a file-triggered
reload has no Java call to return through — so `phase_1/devshell` parks HTML in a
`Mutex<Option<String>>` and `ZKiteActivity` polls `Native.pollHtml()` every 120ms, a
trade phase 1 §4 documents as deliberately avoiding cross-thread JNI. **The native
backend already had the mechanism that was missing**: a render thread that owns "take a
new tree from Python, repaint," reached by an `mpsc` channel four other event sources
already use. So the socket thread never touches Python, never touches JNI, and never
renders — it writes files and sends one `Command::Reload`. No polling, no
`PENDING_HTML`, and no ordering requirement against `boot()` either (devshell must
start its socket only after the interpreter is initialised; here a `Reload` queued
early simply waits in the channel, because `State::new` runs `zkite_pybridge::setup`
before the loop ever reads it).

What it took, end to end:

| Piece | What |
|---|---|
| `runtime.py::_zkite_reload_tree` | The missing tree-mode twin of `_zkite_reload`. Same `reload_module` + `use_module` class-swap, so `@store` preservation is inherited rather than reimplemented. |
| `zkite_pybridge::{reload, last_error}` | Mirrors `dispatch`; `last_error` was already backend-agnostic in `runtime.py`. |
| `gpu_harness/src/reload.rs` | The socket. Wire-compatible with devshell's, so `dev_cmd.rs` needed no protocol changes at all. |
| `Command::Reload { ack }` | Handled on the render thread, `SyncSender` ack so a traceback reaches `zkite dev`'s terminal in the same turn (the `DestroyWindow` idiom). |
| `ZKiteActivity.startReloadServerIfDevShell` | Gated on a `zkite-devmode` marker asset. |
| `ZKITE_DEVSHELL=1 build_apk.sh` | Builds the generic Dev Shell: no `app.zip`, plus the marker. |

**One shell, two artifacts, one marker.** The native Dev Shell and every app
`zkite build --backend native` produces run the identical `classes.dex` and `.so` — they
differ only in what is packed beside them. A shipped app must never run the socket
(phase_2/PLAN.md §1.2: an infinite reconnect thread behind an `INTERNET` permission
with no runtime purpose), so the check is an asset probe rather than a build-time
constant, which is exactly what lets the two artifacts share prebuilt binaries.
Correspondingly: the harness manifest requests `INTERNET`, the generated per-app
manifest does not, and the code that would need it is absent from that build — the
permission and its consumer are absent *together*.

**A real bug this surfaced before the device did:** the Dev Shell has no `app.zip`, and
`unpackZipAssetOnce` throws on a missing asset — caught by the `onCreate` handler that
also skips `Native.boot`. The Dev Shell would have booted to nothing. Fixed with
`unpackOptionalZipAssetOnce`; `runtime.py`'s `_zkite_setup_tree` already rendered a
"waiting for `zkite dev`…" tree for exactly this case, so nothing else had to change.

**Verification, live on the Moto G54.** `zkite new devdemo` → `zkite dev` (no backend
flag — native is the default):

```
no bundled app.zip — dev shell mode, waiting for `zkite dev`
dev shell: starting reload socket on 127.0.0.1:8765
reload socket connected to 127.0.0.1:8765
wrote zkite.tailwind.json / wrote main.py / wrote pyproject.toml
reload: applied, repainted
ZKITEPERF first-frame since_boot_call=1931ms nodes=6 via=gpu
```

Then two real taps (counter → `2`), then a live edit of `main.py` on the host —
`bg-slate-950` → `bg-emerald-950`, `"Increment"` → `"Tap me"` — with `zkite dev` still
running. `pushed 1 change(s) in 0ms: ~main.py`, and on the next frame:

| | before edit | after edit |
|---|---|---|
| background | `(2, 6, 24)` slate-950 | **`(0, 44, 34)` emerald-950** |
| button | 324 x 119 px | **264 x 119 px** (relaid out for the shorter label) |
| counter digit | `2` | **`2`, byte-identical bitmap** |

**That last row is the whole point.** plan1.md §6's central promise — "save a file, see
the change, keyboard state and scroll position intact" — now holds on the native
backend, verified the same way phase 1 §7 M5 verified it on the WebView one: the edit
visibly landed *and* the store survived it.

### 15.3 What is still missing from the dev loop

- ~~**`zkite dev` does not auto-install a Dev Shell.**~~ **Fixed for the local case, see
  §24.** It now probes with `pm path`, launches an installed shell, and installs a
  prebuilt one when it can find it — so `zkite new` → `zkite dev` reaches a running app in
  two commands. **What is still missing is the distribution half**: nothing *downloads*
  a Dev Shell, so the auto-install only finds an APK inside this checkout. §24.3.
- ~~**Four bugs the owner hit in one real `zkite dev` session (2026-08-09), all
  root-caused here and none fixed.**~~ — **all four fixed, see §31.** Reported from the
  product, not from a code read, which is how §25 and §26 both started. The originals
  are kept below as written, with two corrections that matter more than the fixes:

  > **Item 1's proposed fix would not have worked.** It says to add "the shell's
  > `zkite_py` version" to the `hello` frame. `phase_1/zkite_py/pyproject.toml` reads
  > `version = "0.1.0.dev0"` and has never moved; there is no `__version__` in
  > `src/zkite/` at all. The stale shell and the fresh payload would have reported the
  > *same* version and the handshake would have stayed silent through the exact failure
  > it was written for. §31.6 hashes the framework's **contents** instead.
  >
  > **Item 3 had a second cause this entry could not see.** The placeholder being
  > composed unconditionally is real and is fixed (§31.5), but it was *visible* only
  > because the native paint path drew every glyph in the tree above every background in
  > it — so an opaque overlay could not cover it. That is §31.1, it independently broke
  > `Stack`, and it is the larger finding of the two.

  1. **A framework change does not reach a running Dev Shell, and nothing says so.**
     The owner's app failed with `cannot import Checkbox from zkite` immediately after
     §30 shipped `Checkbox`. Correct, and structural: `zkite dev` pushes only the
     *project's* `.py` files, while `zkite/` itself ships inside the Dev Shell's
     `assets/zkite-framework.zip`. So **the Dev Shell pins the framework version**, and
     any widget, fix or API added after the installed shell was built is simply absent —
     as a bare `ImportError` on the phone with no hint about the cause. Confirmed by
     unzipping the checked-in artifact: `zkite-native-devshell-arm64-v8a.apk` was built
     at 04:27 and §30 landed at 23:35, so its `widgets.py` had no `Checkbox`. **This is
     §15.6.1's lesson pointed at the user instead of at us** — "editing Python requires
     rebuilding the shell" is a rule this project already learned and never surfaced in
     the product. The fix is a version handshake: the `hello` frame already exists
     (§15.6) and carries only `backend`; adding the shell's `zkite_py` version and having
     `zkite dev` compare it against the payload's would turn a mystery `ImportError` into
     one line naming the rebuild command.
  2. **A glue-level error never reaches the terminal.** `_zkite_last_error()` returns
     `_app.last_error`, but an import failure in the user's `main.py` raises inside
     `load_module` *before* `_app` is ever assigned — so `_app` is `None`,
     `last_error` is `None`, no `OutFrame::Error` is written, and `zkite dev`'s terminal
     stays silent while the phone shows the traceback. The overlay and the wire report
     are produced at two different levels and only the lower one knows. plan1.md §6
     promises the traceback appears "simultaneously in the terminal"; for the whole
     class of errors that stop an app booting at all, it does not.
  3. **The "waiting for `zkite dev`…" placeholder renders *behind every error*.**
     `_glue_error_tree` composes `[_WAITING_TREE, format_error_tree(...)]`
     unconditionally. That was right when the only way to reach it was before a
     connection existed, and it is actively misleading afterwards — the owner read it as
     "it has disconnected" while it was connected and serving. The shell knows whether a
     socket is attached; the placeholder should be conditional on that.
  4. **The native error overlay does not wrap, so a traceback runs off the right edge.**
     `format_error_tree` puts the raw traceback in a single `#text` child with no
     wrapping; its HTML twin sets `white-space:pre-wrap; overflow:auto` and therefore
     reads fine on the WebView. **Two hand-maintained renderings of one overlay, and
     only one of them wraps** — the same shape as §9.5's palette, §9.7's fonts and
     §28.1's defaults. §5's own list still carries text wrapping as open on native, so
     this is that gap surfacing where it hurts most: the screen you are reading
     *because* something already went wrong. §19.3 records the overlay's other limits
     and did not have this one.
- **No delete-file frame** — a file removed on the host stays on the device (phase 1 §4,
  unchanged, and it affects both backends).
- **Reload timing was not measured.** §6's target is <200ms; the terminal reports `0ms`
  for the manifest diff and the device logs `reload: applied, repainted` immediately,
  but no stopwatch was run against on-screen change. Same honest gap phase 1 §9 records
  for the WebView loop.
- **Only the counter app was reloaded.** A reload with a focused `TextField`, an open
  selection overlay, or a scrolled `List` was not tested — and those are precisely
  where "keyboard state and scroll position intact" is a non-trivial claim rather than
  a free one. The scroll offsets survive by construction (keyed by `zkite_py` node id),
  but that is an argument, not an observation.
- **The 10s ack timeout has never fired** and its behaviour is therefore untested.

### 15.4 The default flipped to native — an owner decision, against the measured ledger

**`zkite dev` and `zkite build` now default to `--backend native`. `--backend webview`
selects the WebView backend, and `[tool.zkite] backend` pins it per project.**

**This is recorded as a product decision, not as a conclusion the numbers support**,
and this document should not be read as claiming the evidence changed. §13.4 applied
§12.3's pre-stated criteria and concluded, in terms: *"the switch still does not flip,
and phase 3 is still not dropped."* Every number behind that stands. On the Moto G54,
native still loses cold start (~1450ms vs ~960ms), still loses memory (§14.7: 272 MB of
`Graphics` against the WebView's 101 MB for the same six-node app), and still costs
15.5 MB of binary permanently. It wins tap latency ~3.6x and ties sustained scroll and
text churn.

> **Status (§20): the memory clause above no longer holds, and the decision does not
> depend on it.** The 272 MB was wgpu's `MemoryHints::Performance` default reserving
> 128 MB/64 MB suballocation blocks, flat regardless of what the app draws; setting the
> hint explicitly takes the same six-node app from ~334 MB to **~155 MB** total PSS,
> i.e. below the WebView's 235 MB, with no frame-time cost. Cold start and binary size
> are unchanged. This is recorded because the ledger should stay accurate, **not** as
> retrospective support for the flip — §15.4 was decided on control over rendering and
> explicitly not on these numbers, and it would have been the same decision either way.

The decision was made by the project owner with those numbers, §15.1's gap list, and
the following research in front of them:

- **"Ship a browser in Rust" and "build the native renderer" are the same project.**
  [Blitz](https://github.com/DioxusLabs/blitz) — which plan1.md §13 already lists as
  the closest architectural cousin — is Stylo + Taffy + Parley + Vello + AccessKit.
  ZKite's native renderer is twcore + Taffy + Parley + Vello + AccessKit. It is the same
  stack with a different style engine, so the credible Rust-browser option is not an
  alternative to phase 3; it is a description of it.
- **Blitz is [alpha](https://blitz.is/about)**, "not ready for production usage," with
  production targeted during 2026. JavaScript, network caching and process isolation
  are explicitly deferred.
- **[Servo](https://servo.org/) is not a WebView replacement yet** — its own ecosystem
  says it "lacks some APIs that make it a perfect replacement for Android WebView
  today."
- **[Dioxus](https://dioxuslabs.com/blog/release-070/), better resourced and further
  along, ships the system WebView for Android in production today**, with its native
  renderer as an experimental preview. They made plan1.md §5's call and have not
  flipped it either.

The argument that carried the decision is therefore **not** performance. It is
*control*: a system WebView's version varies per device and is updated out from under
the app, WKWebView-vs-Blink means "write once" degrades on iOS, and a framework (as
opposed to one app) pays that as a support cost on every device it does not own. A
renderer in the binary is the same everywhere. That argument is real and is not
measured by anything in §12/§13.

**What flipping the default costs, stated plainly:** every `zkite new` app now lands on
a renderer that draws `Image` and `Icon` as empty boxes, paints no borders or shadows,
ignores `dark:`, does not scroll horizontally, and has no Autofill (§15.1). `zkite build`
prints exactly that list on every native build, naming `--backend webview` as the
escape hatch, because a user finding this out from a screenshot is worse than finding
it out from the build.

**plan1.md §10's phase-3 exit criterion is "the backend switch flips by default and no
sample app changes a line."** The second clause is met and checked (§14.5: byte-identical
`main.pyc` across both backends). The first is now literally true. It should not be
read as "phase 3 is complete" — §15.1 is the list of why not.

### 15.5 Known gaps in this section's own work

- **Hot reload verified on one app, one edit, one device.** See §15.3.
- **`reload.rs` has no tests, and cannot.** `gpu_harness`'s `build.rs` compiles
  gametextinput's C++ against NDK headers, so the crate does not build for the host at
  all (§6.4) — tests written there would never run, which is worse than none. The
  path-traversal guard is a verbatim copy of `phase_1/devshell`'s host-testable
  `apply_file`; an equivalent test belongs there and does not exist yet in either.
- ~~**The WebView dev loop was not re-run after this change.**~~ **Closed — but the
  first attempt to close it was itself invalid, see §15.6.** `dev_cmd.rs` changed for
  both backends and `runtime.py` — which is `include_str!`'d into *both* shells —
  gained `_zkite_reload_tree`, so the WebView path had two ways to regress. The first
  re-test *claimed* to have rebuilt the Dev Shell to pick up the shared `runtime.py`
  and had not: `devshell/scripts/build_apk.sh` reused the existing `.so` because the
  file existed, so the APK under test still contained the old `runtime.py`. Re-run
  after fixing that (§15.6), against an artifact verified by `strings` to actually
  contain `_zkite_reload_tree`: `webview Dev Shell connected`, a live edit pushed, and
  the background measured at `(70, 25, 1)` — Tailwind v4's `amber-950`. The original
  run's own observations still stand on their own terms:
  `zkite dev --backend webview` connected, pushed a fresh `zkite new` project, two taps
  took the counter to `2`, and a live edit took the background slate-950 -> rose-950
  and relaid the button out 324 -> 176 px for `"Go"` **with the digit still a `2`**.
  Both dev loops now hot-reload with state preserved, verified the same way.
- **The flipped default was also driven end to end**: `zkite build apk` with no
  `--backend` flag at all, installed and launched, `67 native extension modules
  available`, `first-frame ... via=gpu`, two taps -> `2`, `tap-to-paint` 8.5/7.1 ms.
- ~~**Nothing was done about the §14.7 GPU-memory question**, which flipping the default
  makes more pressing rather than less: ~310 MB resident is now what a default ZKite app
  costs. Logging `adapter.limits()` and instrumenting `vello_hybrid::Renderer::new` is
  still the concrete next step.~~ **Done — see §20.** A default ZKite app now costs
  ~135–155 MB resident rather than ~310–334 MB, from one explicitly-set `memory_hints`
  field, with scroll frame time unchanged.

### 15.6 `--backend` silently did nothing when the other Dev Shell was running

**Status: found by the user on their own phone within an hour of §15 landing, fixed,
and verified live in both directions.** This is a bug §15 introduced.

**The symptom.** `zkite dev` — which defaults to native — brought up the app on the
**WebView** backend. Running it again with an explicit `--devshell-apk` pointing at the
native Dev Shell then worked.

**The cause is a design hole, not a typo.** The reload protocol is deliberately
*identical* on both backends (§15.2 treats that as a feature — it is why `dev_cmd.rs`
needed no per-backend protocol handling), both shells dial `127.0.0.1:8765` in a retry
loop, and **nothing on the wire identifies which backend connected.** `--backend`
selected only which package to *launch* — and with no `--devshell-apk`, `zkite dev`
launches nothing at all. So whichever shell happened to already be running won the
race. On this user's phone both were installed, from this session's own testing.

The failure mode is the one this document keeps cataloguing: the flag reads as
honoured, the app renders, nothing errors, and the only symptom is that the renderer is
the wrong one.

**Fix, in two halves, because either alone is insufficient.**

- **Prevention:** `zkite dev` force-stops the *other* backend's package before binding
  the port.
- **Detection:** each shell now sends `{"op":"hello","backend":"native"|"webview"}` as
  its first frame on connect. A match prints `[device] native Dev Shell connected`; a
  mismatch prints a four-line block naming what was asked for, what actually connected,
  and the two ways to fix it. Deliberately loud rather than fatal — by the time the
  hello arrives the push has happened and the app on screen is real and working, just
  not the requested backend; killing a working session would be worse than saying so.

Prevention alone would not cover a shell launched by hand *after* `zkite dev` starts,
which is exactly how the fix was tested.

**Verified live on the Moto G54, both directions:**

```
# wrong shell launched by hand after `zkite dev` (native) was already listening
!! BACKEND MISMATCH — you asked for `native` and a `webview` Dev Shell connected.
!! What you see on the phone is the webview renderer, not native.

# and the positive case
[device] native Dev Shell connected
```

Plus the prevention half on its own: with the WebView shell running and both installed,
plain `zkite dev` no longer serves it — the shell is stopped before the port is bound.

#### 15.6.1 The bug behind the bug: `build_apk.sh` shipped a stale `.so`

Finding the above surfaced something worse, and it invalidated a claim already written
into §15.5.

**`phase_1/devshell/scripts/build_apk.sh` reused the existing cdylib whenever the file
merely existed** (`if [ ! -f .../libzkitedevshell.so ]`). An existence check is not a
freshness check: the staged `.so` was from `13:22` and the source edit from `01:31`, so
the "rebuilt" APK contained neither the new hello frame nor — much more importantly —
the `runtime.py` change, because **`runtime.py` is `include_str!`'d into that `.so`**.
Editing the *Python* glue requires a *Rust* rebuild, which is exactly the case where a
stale-artifact guard is least likely to be suspected.

The consequence: §15.5's first attempt to re-verify the WebView dev loop tested an APK
that did not contain the change it was verifying. The test passed and proved less than
it claimed. Caught by `strings`-ing the packed `.so` for the new symbol rather than
trusting the build log — the same "measure the artifact, not the code" move §11.8 had
to make with one `adb shell ls`.

**Fixed in both shells by deleting the guard**, not by improving it. `gpu_harness`'s
copy had the identical hole (this session had already widened it to check *every*
artifact, which addressed a different failure and left this one untouched). cargo is
incremental, so an unconditional rebuild costs about a second when nothing changed —
the guard was a false economy paid for in device-debugging time. Both scripts now say
so in a comment naming this incident.

**Standing lesson, and it is a repeat.** "The build succeeded" has now been wrong for a
fourth distinct reason in this project: a missing `extra_files` field (phase 0), an
un-extracted `lib*.so` (§11.8), a manifest naming a class from a dex that was never
packed (§14, avoided), and now a reused binary older than its source. The check that
would have caught every one of them is the same: **verify the artifact contains what
you think you just put in it.**

## 16. Navigation and a back stack

§15.1 opened with this, and plan1.md §14.1 calls it *"the single largest gap in the
framework"*:

> **There is no navigation and no back stack, anywhere, in any shell.** No
> `onBackPressed`, no back-gesture handling, no screen stack, no `Nav` widget. Every
> ZKite app is one screen and the Back button exits it.

Measured again before starting rather than taken from the note: `grep` for
`onBackPressed|onBackInvoked|KEYCODE_BACK|back_stack` across every `.java`, `.rs` and
`.py` in the repo returned **nothing**, and `widgets.py` had no `Nav`.

**Status: built and verified live on the Moto G54 on *both* backends, including the
real back gesture driven by a human finger rather than `adb`.** A three-screen demo
pushes twice and pops back with `@store` state intact, and an app with no `Nav` in it
still exits on Back exactly as before.

### 16.1 Why this was picked, and why it is three layers rather than one

plan1.md §10 files navigation under phase 4. That ordering assumed phase 3 would not
be shipping apps yet, and §15.4 flipped the default to the native renderer, so it now
is. But the argument that actually decided it is not "it is the biggest gap" — it is
that **navigation is the only item on §15.1's list that shapes the public API**, and
therefore the only one that gets more expensive the longer native ships as the
default.

Everything else on that list is additive inside a structure that already exists.
Horizontal scrolling adds an axis to a mechanism that is already there —
`scroll_offsets` is `(f32, f32)` in both `renderer/src/lib.rs` and
`render_thread.rs`, and only `entry.1` is ever written. Gradients add a `Paint`
variant and a brush. `Image`/`Icon` add a decode step. None of them changes what a
`main.py` looks like. Navigation introduces the first concept in ZKite that is not a
widget — a stack of screens with identity that outlives a render — and it has to
interact with `@store` (phase_1/PLAN.md §4's class-swap reload), with `_viewport.py`'s
module-level scroll state, and with the process-death restoration gap §15.1 lists
separately. Get the shape wrong now and every app written against it has to change.

**The instruction this was built to** (the project owner's, verbatim) was to design it
"in such a way that some other navigation system can be made in future easily... but
no compromise with performance." Those pull against each other unless mechanism and
policy are genuinely separated, so they are:

| layer | what it is | where |
|---|---|---|
| 1. the back channel | a chain of handlers, innermost first; a platform Back press walks it | `RenderContext.register_back`/`dispatch_back` (`widgets.py`) |
| 2. `NavigationHost` | the two-method contract a navigator satisfies: `current_entry()` and `handle_back()` | `_nav.py` |
| 3. `Navigator` | the built-in **stack** policy — and the only policy here | `_nav.py` |

A tab navigator, a modal/sheet navigator, a nested per-tab stack, or a URL router are
all further implementations of layer 2 and need no change to layers 1 or 3. A router
in particular falls out nearly free, because layer 2 already speaks in *names and
params* rather than screen functions — see §16.3. A host test
(`test_a_custom_host_needs_only_two_methods`) implements a whole alternative policy —
a latch, no stack at all — against the documented protocol and drives it through the
real `Nav` widget, so "layer 2 is a real extension point" is asserted rather than
claimed.

**And the generality costs nothing per frame, by construction.** Only the top entry is
ever resolved and built, so a screen pushed five deep costs what one at the root does;
`test_deeper_stack_builds_exactly_one_screen` pins that. The back chain is one list
append per `Nav` per render and is never read unless Back is actually pressed; an app
with no `Nav` appends nothing (`test_back_chain_is_empty_when_no_navigator_rendered`).
Nothing was added to `redraw()` or to any per-frame path in either shell — a push is
one `@store` update, which is the same re-render a button tap already causes.

### 16.2 The API

```python
from zkite import Button, Column, Nav, Navigator, Text, store

def home(nav: Navigator, c: Counter):
    return Column("flex-1 ...",
        Text("text-7xl", f"{c.n}"),
        Button("...", "Open settings", on_tap=lambda: nav.push(settings)))

def settings(nav: Navigator):
    return Column("flex-1 ...", Button("...", "Back", on_tap=nav.pop))

def view():
    return Nav("flex-1", home, settings)
```

**A screen is an ordinary view function.** It takes stores by annotation exactly as
`view()` does — same resolution code path — plus whatever params it was pushed with
(`nav.push(detail, item_id=42)`). That is deliberately not "nearly true": both call
sites share `_store.signature_with_resolved_annotations`, so a screen and a view
cannot drift on how injection works. A parameter is either annotated with a `@store`
class or it is not, and that decides which source it comes from, so the two can never
collide.

`Navigator` is a real `@store`, which is not a shortcut — it is the whole reason nav
state behaves correctly across a hot reload. `reload.py` only re-executes and reshapes
stores defined in the *reloaded* module, and this one is defined in `zkite`, so it is
never touched: you stay on the screen you were on while its code updates underneath
you.

The imperative surface is `push`/`pop`/`replace`/`pop_to_root`/`reset`, plus
`depth`/`can_pop`/`current`. `pop()` refuses to empty the stack and returns `False`;
`handle_back()` returns `False` in the same situation, which is what lets the platform
exit the app.

### 16.3 The one decision that would have been a long-lived bug

**`Entry` holds a screen *name*, never a function object.** Hot reload re-executes the
module and replaces every function object in it, so a stack holding function objects
would keep rendering the *old* code for every screen already pushed — the reload would
appear to work on whichever screen happened to be the root and silently not on any
other. That surfaces as "hot reload is flaky", which points nowhere near navigation.

Names are resolved late, by `Nav`, against the screen list the freshly-run `view()`
just handed it. `test_pushed_screen_picks_up_a_reloaded_module` pins it, and **it was
confirmed to fail against the alternative design** — one variable changed (`push`
stores the function it was handed instead of its name, `Nav`'s screen table left
keyed by name so nothing else differs), producing `AssertionError: 'v1' != 'v2'`.
Pushing by *function* is what a real app writes and the only form where the naive
design could capture a stale object; the test's first draft pushed by name, which
would have been safe either way and would have exercised nothing.

This is also what makes a future router cheap: `(name, params)` is already what a
route resolves *to*.

### 16.4 The platform wiring, and why the native side is fire-and-forget

Both shells consume **every** Back press and never call `super.onBackPressed()`.
Whether anything actually wanted it is only knowable inside `zkite_py`, so the decision
is made in exactly one place — `App.dispatch_back()` walking the chain the last render
registered — and the shell acts on the answer.

- **Native.** `ZKiteActivity` → `Native.onBackPressed()` → `Command::Back` on the
  render thread → `zkite_pybridge::back` → repaint, or `ZKiteSurfaceView.finishActivity()`
  back on the UI thread. **Deliberately never blocking**, unlike `DestroyWindow`'s
  acked shutdown: Android wants a synchronous decision from `onBackPressed`, the
  honest answer lives behind the render thread's queue, and that thread may be
  mid-frame or mid-CPython call — blocking the UI thread on it is an ANR waiting for a
  slow frame. The tradeoff, stated rather than discovered later: if the render thread
  is ever genuinely wedged, Back does nothing instead of exiting. That is strictly
  better than an ANR, and unlike `onSurfaceDestroyed`'s 3-second ack it cannot hang
  Android's callback at all, because it never waits.
- **WebView** (`appshell`, `devshell`). `Native.handleBack()` is synchronous and
  returns the new HTML or `null`, but the Activity still calls it **off** the UI
  thread and posts the result. Not because it must — because running an app's `view()`
  on the UI thread would put user Python on the critical path of a gesture, and
  because matching the native side means Back has one observable behaviour across both
  backends rather than two similar ones.

Both shells wire **both** platform mechanisms: an `OnBackInvokedCallback` (API 33+)
and an `onBackPressed()` override below it. Exactly one fires per press, so there is
no double-pop to guard against.

### 16.5 A real finding: `vendor/pack` cannot express `enableOnBackInvokedCallback`

Opting into the modern back dispatcher needs
`android:enableOnBackInvokedCallback="true"` on `<application>`. Setting it in the
generated manifest **fails the entire pack**:

```
pack+sign: UnknownAndroidInternalAttribute("enableOnBackInvokedCallback")
```

Same shape as the `android:configChanges` flag-constant gap phase_1/PLAN.md §7 M4 hit
and dropped for the same reason — `vendor/pack`'s AXML compiler has a fixed attribute
table. **Caught by `phase_2/bundler`'s own test suite, not at install time**, which is
what that suite is for.

Backed out of all three manifests rather than forking pack (phase 0's README argues
against a fork, and this is one table entry — the same small upstream contribution it
already recommends for `extra_files`). Nothing is lost functionally: with the flag
absent the platform uses legacy dispatch and calls `onBackPressed()`, which both
shells override, so **Back works on every API level today**. The registered
`OnBackInvokedCallback` is inert until the flag can be set. The only thing missing is
the predictive-back *animation*, and there are no screen transitions to animate
anyway — §15.1's "no animation driver exists" is unchanged by this section, on purpose
(see §16.7).

### 16.6 Verification, live on the Moto G54 (arm64-v8a, Android 15)

`demo_app/nav.py` — three screens, each a **different background colour**, so which
screen is on top is unambiguous from a screenshot's histogram rather than from OCR or
a log line; the home counter is rendered only on home, so state survival is checkable
by eye; the depth is rendered, so a stack that silently grew or emptied is visible.

**Native backend** (`gpu_harness`, `ZKITE_APP_MAIN=demo_app/nav.py`):

| step | result |
|---|---|
| launch | `home (slate-950)`, depth 1, counter `0` |
| tap Increment ×2 | counter `2` |
| tap Open settings | `settings (emerald-950)`, depth 2 |
| tap Open item 42 | `detail (rose-950)`, depth 3, **`42` rendered from the pushed param** |
| **Back #1** | → settings, `pid` unchanged |
| **Back #2** | → home, **counter still `2`** |
| **Back #3** (root) | app exits to the launcher |

Zero `panic`/`FATAL`/`AndroidRuntime` markers across the whole run.

**The real gesture, by hand, not `adb`.** §8.5 already found that `adb`-synthesised
input is not always a faithful stand-in for the pipeline a real finger goes through,
so the same sequence was driven by a human pressing Back on the device. Same result,
and the logs show it took the identical path:

```
back: onBackPressed() called
back: consumed by zkite_py, repainting        <- while depth > 1
...
back: onBackPressed() called
back: not handled by zkite_py, finishing activity   <- at the root
```

**The control, which is the regression check that matters.** An app with no `Nav`
(`demo_app/cmp_counter.py`) registers no back handlers, so the chain is empty,
`dispatch_back()` returns `False`, and Back exits immediately — `back: not handled by
zkite_py, finishing activity`, straight to the launcher. That is both "nothing
regressed for a one-screen app" and independent evidence that the consumption in the
nav app really comes from the registered handler rather than from Back being
swallowed globally.

**WebView backend**, same app through `zkite build apk --backend webview`, installed as
a real signed APK: identical sequence, identical outcome — push, push, Back → settings,
Back → home with the counter still `2`, Back at root → exits.

**Note the trap this section had to walk into deliberately.** Every prior session's
notes say *"don't send `KEYCODE_BACK` to the native shell — it triggers DestroyWindow
and exits."* That was the pre-fix behaviour and this is the change that removes it, so
sending it is now the correct test. One earlier attempt in this session appeared to
show Back exiting from depth 3 with **no log line at all**; that was an `adb logcat -c`
race losing the evidence, found by re-running with `logcat -T <timestamp>` instead of
clearing. Worth recording because "no log" read as "the handler never ran" and was
wrong.

### 16.7 What this deliberately does not do

- **No screen transition animations.** Animating between screens needs both trees
  alive at once, and there is no animation driver anywhere in the framework
  (§15.1) — building half of it would have meant keeping every screen materialised
  for a feature that cannot currently run. `Nav` builds one screen; that is the
  performance property §16.1 is built around, and it is the thing an animation would
  have to trade away.
- **No `Sheet`, `Dialog`, or modal policy.** All of them sit on top of this and are
  layer-2 implementations; none is built. The back chain's innermost-first ordering
  (`test_nested_nav_gets_back_before_the_outer_one`) is what they will need, and it
  exists.
- **No deep links, and no state restoration after process death.** §15.1 lists both
  separately and both remain open. The stack is `(name, params)` — i.e. already
  serialisable, which is the piece restoration would need — but nothing writes it
  anywhere.
- **No `Nav` conformance fixture.** §9's harness renders one widget in one state, and
  a navigator's interesting states are all transitions; the two backends were compared
  on a device instead, which is where the divergence would actually be. Conformance is
  **unchanged to two decimal places on all 14 fixtures at both dprs** (§13.1.2's table
  exactly), which is the evidence that this changed no rendering.

### 16.8 A pre-existing bug this surfaced and fixed

**`from __future__ import annotations` silently turned off store injection.** Stores
are injected by annotation, and that import makes every annotation a *string*, so
`is_store(ann)` was comparing against `"Counter"` rather than the class — reporting
`view() parameter 'c' is not a @store class` about a parameter that visibly is one.
Latent in `app.py` since phase 1; it only surfaced here because screens are a second
place annotation-based injection happens and the new test module has that import at
the top.

Fixed once, in `_store.signature_with_resolved_annotations` (`eval_str=True`, falling
back to the unevaluated signature so an unresolvable forward reference cannot kill a
render), used by both `view()` and screen resolution.
`test_string_annotations_still_inject_stores` pins it and **was confirmed to fail
against the unfixed code**.

### 16.9 Known gaps

- **One device, one Android version.** Moto G54, API 35, both backends. The legacy
  `onBackPressed()` path is the one actually exercised (see §16.5 — the modern
  callback is inert without the manifest flag), so the `OnBackInvokedCallback` branch
  is **wired and compiled but never fired**. It needs either pack's attribute table
  fixed or an API<33 device, and neither is available here. Named rather than assumed
  working.
- **No hot-reload test of navigation.** §16.3's name-not-function design is what makes
  a pushed screen pick up an edit, and it is verified by a host test that *simulates* a
  reload (same name, new function object) — not by a real `zkite dev` session against a
  pushed screen. That is the natural next check and is exactly §15.3's existing "only
  the counter app was reloaded" gap, now with a second thing worth reloading.
- **Nested navigators are host-tested only.** The innermost-first ordering has a test;
  no app on a device has two `Nav`s.
- **No back-gesture *preview*.** Predictive back's animation needs both the manifest
  flag and an `OnBackAnimationCallback`; neither is wired (§16.5, §16.7).
- **`pop()` on an empty stack is unreachable rather than handled.** `Nav` seeds the
  root on first render and `pop` refuses to remove the last entry, so the empty state
  cannot be reached through the public API — but a custom `NavigationHost` returning
  `None` from `current_entry()` renders an empty box, which is a legitimate state for
  a tab host and is not otherwise exercised.
- **The demo app is not in the conformance fixture set** (§16.7).

## 17. Production readiness — the research pass

Not a feature section. The project owner asked for a deep look at the things that
stand between "phase 3 works" and "someone can actually ship a real app with this",
**excluding publishing** (`pip install zkite` / PyPI), which is deliberately deferred.
Everything below is either measured on the Moto G54 or checked against the actual
code and the actual external rules, not carried over from a summary.

Two of these turned into fixes during the research, because the measurement made the
answer obvious and the fix was one line. The rest are written up with a cost and a
recommendation, not built.

### 17.1 Startup: bytecode caching was off, and it was the single largest cost

**Status: found, fixed, measured on both backends. This is the biggest result in this
section and it was not on anyone's list.**

`config.write_bytecode = 0` sat in all three shells (`pybridge/src/embed.rs`,
`appshell/src/lib.rs`, `devshell/src/lib.rs`) with **no comment saying why**, since
phase 1. Meanwhile `prune_stdlib.py` drops `.pyc` outright (`DROP_SUFFIXES`), so the
stdlib ships as ~490 **source** files. Those two facts together mean every launch
re-parsed the entire reachable stdlib from source, forever, and nothing was ever
cached.

**A/B on the Moto G54, same app, same session, this line the only difference:**

| | `write_bytecode = 0` (shipped) | `= 1` |
|---|---|---|
| native (`gpu_harness`, nav demo) | **2584 ms** | **~990 ms** |
| WebView (`zkite build apk`, same app) | **978 ms** | **420 ms** |

2.6x and 2.3x. The mechanism is confirmed by the launch-by-launch curve after a fresh
install, which is what makes this a cache effect rather than noise:

```
launch 1: 4362 ms   (unpacks 577 files in 351ms, then compiles everything it imports)
launch 2:  933 ms
launch 3: 1040 ms
```

One-time +1.8 s on first launch buys −1.6 s on every launch after it; break-even is
launch two.

**What this changes elsewhere in this document.** §13.6 lists *"Cold start regressed
relative to the WebView and was not investigated"* as an open item — this is the
investigation, and the answer was not native-specific at all: both backends were
paying it. §13.3's ledger entry "loses cold start (~1450ms vs ~960ms)" is unchanged in
*direction* — the WebView improves at least as much — so **§15.4's default-flip
argument is not affected**. And plan1.md §9's *"interactive <500ms"* target, which
§12.6 called missed "by the architecture, not by either backend", is now **met by the
WebView backend (420 ms)** and no longer looks architectural.

**It fails safe.** If the app's private directory is unwritable CPython silently skips
the cache and behaves exactly as before.

#### 17.1.1 The hazard it introduced, and the bug that found it

Turning the cache on made `zkite_pybridge`'s suite intermittently fail (**2 in 6 runs;
0 in 8 with the cache off**), which is what forced a proper look rather than a happy
number and a commit. Two distinct causes came out of it, and only one was mine:

- **A real `zkite dev` hazard, fixed.** CPython validates a `.pyc` against
  **(source mtime in whole seconds, source size)**. Both are things a hot reload can
  leave unchanged — `text-red-500` → `text-red-600` is the same length, and two saves
  can land in the same second — so a stale `.pyc` would be considered valid and the
  edit would silently not apply. That is precisely the "hot reload is flaky" failure
  with no error anywhere. Fixed by `runtime.py::_purge_app_bytecode_cache`, called on
  every boot and every reload: it purges **only the app directory**, so the stdlib and
  framework caches — where the entire win lives — are untouched. Cost is one small
  recompile per launch (measured: median 1087 ms against 990 ms without the purge,
  i.e. inside run-to-run spread, and still 2.4x better than the 2584 ms baseline).
- **A pre-existing test-isolation bug, not caused by this.**
  `accessibility_tree_has_labeled_clickable_button_and_text` asserts the counter digit
  is `"0"`, and `builds_the_real_counter_app_and_dispatches_a_tap` **taps the button**,
  leaving `n = 1` in the process-global `@store` registry under the shared key
  `"main.Counter"` (every test loads its app as module `main`). §11.4's `lock_python()`
  mutex serialises those tests but never *resets* the registry, so the ordering simply
  used to fall the lucky way. Fixed by naming the a11y test's store `A11yCounter` — a
  distinct registry key cannot inherit another test's state. **0 failures in 15 runs**
  after both fixes.

The general lesson is the one §11.4 item 4 already recorded and this repeats from the
other direction: a passing test is not evidence of isolation, and a timing change is a
perfectly good way to discover that.

### 17.2 The `.pyc` step buys almost nothing — but it is not what blocks portability

phase_2/PLAN.md §1.4 calls the host-native Python 3.14 requirement *"the real
cross-platform blocker"*, and plan1.md §14.5 repeats it. That framing is right about
the blocker and wrong about which step causes it.

**What the `.pyc` step actually compiles: the app directory, and nothing else.** The
build log says `compiled 1 files to .pyc` for the stock counter app — literally
`main.py`. The stdlib is excluded by construction (§17.1), and `zkite-framework.zip`
is not compiled either. So the step that forces a matching-version interpreter onto
every developer's machine exists to precompile the user's own handful of files, and
§17.1 has now made the device cache those anyway on first launch.

**But dropping `.pyc` does not make `zkite build` portable, because pruning also needs
Python — and unlike `.pyc`, it needs a *matching* one.** Tested rather than assumed:

```
prune_stdlib.py, same inputs, against the vendored 3.14 stdlib
  under Python 3.14.6 : 139 top-level packages kept, 58 dropped   (487+1 files)
  under Python 3.12.3 : 138 top-level packages kept, 59 dropped
  the one difference  : lib/python3.14/annotationlib.py
```

`annotationlib` is **new in 3.14**, so a 3.12 host's `modulefinder` does not know the
3.14 stdlib reaches it. And it is not incidental — it is load-bearing for ZKite
specifically:

```
$ python3.14 -c "import zkite, sys; print('annotationlib' in sys.modules)"
True
```

Merely importing `zkite` pulls it in (PEP 649/749 annotation machinery, which
`_store.signature_with_resolved_annotations` also goes through — §16.8). Pruning under
a mismatched interpreter would therefore ship an app that **cannot boot**, failing on
device with a `ModuleNotFoundError` and nothing visible at build time — the exact
shape of phase_2/PLAN.md §1.7's `_android_support` bug.

**So the shortcut "any Python will do" is refuted, with a mechanism.** The real options
are:

| option | portability | cost |
|---|---|---|
| **`requires-python = ">=3.14,<3.15"` on the wheel** | total — the interpreter running `zkite` *is* the right one | forces developers onto 3.14; couples the developer's Python to the Android Python |
| **Download python-build-standalone on demand** (uv's model; 3.14.6 builds exist for Linux/macOS/Windows × x86_64/arm64) | total | ~30 MB fetched once into a cache; bends plan1.md §2's "installs nothing but a wheel", though it downloads a *prebuilt binary*, it does not compile |
| Vendor a host CPython per platform in the wheel | total | a huge wheel per platform |
| Precompute a pruned stdlib in *our* pipeline | total | wrong by construction for any app importing something the default set omits |

**Recommendation: `requires-python` first, download-on-demand as the fallback if that
is judged too aggressive.** `requires-python` is the only option fully faithful to
plan1.md §2 principle 1, it is free, and `uv python install 3.14` makes it a one-liner
for anyone not already there. Note the coupling it creates: when ZKite moves to Python
3.15 on Android, developers must move too. That is a real cost and belongs in the
decision, not underneath it.

**Independently worth doing either way:** compile the stdlib and `zkite_py` to `.pyc`
**in the release pipeline**, where the Python version is ours to control — the same
bucket as `classes.dex` and the NDK. That removes the first-launch compile (4362 ms →
much less), shrinks the payload, and is the one change here that needs no decision
about the user's machine at all.

### 17.3 Signing: the migration story, and what it actually costs

`zkite build --keystore` reads a **combined PEM only** (`bundler/keystore.rs::load_pem`
→ `Keys::from_combined_pem_string`). `zkite keygen` *writes* a `.p12`, and nothing
reads one back. A developer arriving from Flutter/Gradle has `upload-keystore.jks` or
a `.p12` and cannot use it, which blocks the "migrate an existing Play app" story
outright.

**This is smaller than plan1.md §7 implies, because the crates exist.** Checked live
on crates.io rather than assumed:

- **`jks` 0.3.3**, MIT, ~30k downloads, *"Java KeyStore (JKS) and PKCS12
  encoder/decoder"* — exposes `KeyStore` / `PrivateKeyEntry` (private key **plus**
  certificate chain) with a `decoder` module for JKS and a `pkcs12` module built on
  `p12-keystore`. One MIT dependency covers **both** formats.

So `--keystore foo.jks --keystore-pass ... --key-alias ...` is adapting a
`PrivateKeyEntry` into the `Keys` that `apk.rs`/`aab.rs` already consume. plan1.md §7
already promises exactly this (*"accepts what developers already have"*); it is simply
unbuilt.

**Fingerprints — and the part that is easy to get wrong.** There is no
fingerprint command at all today (the only SHA-256 in the repo is the dev-loop file
hash), so no SHA-1 for Firebase / Google Sign-In / Maps. Computing it is trivial
(SHA-1 and SHA-256 over the certificate DER, which `keystore.rs` already has in hand).

The subtlety is *which* fingerprint, and it is worth building into the command's own
output rather than leaving to a docs page: under **Play App Signing** — mandatory for
new apps — you upload signed with your **upload key** and Google re-signs with the
**app signing key** it holds. Firebase and Google Sign-In need the fingerprint of the
key that signed what the *user* installed, i.e. Google's. So a developer typically
needs **both** registered: the upload key's (for pre-launch/internal-test builds) and
the app signing key's (for production). ZKite can compute the first and **cannot**
compute the second — it only exists in the Play Console. A `zkite fingerprint` that
prints the local one without saying so would actively mislead, and "my Google Sign-In
works in testing and breaks in production" is the classic symptom.

**Estimated cost:** a `.jks`/`.p12` reader plus `zkite fingerprint` is roughly the
150-line session plan1.md §14 predicted, and the crate discovery makes the reader the
smaller half.

### 17.4 `version_code`: a default that Play rejects, and a per-ABI collision

> **Item 2 is fixed (§29.3); item 1 is not.** The owner promoted item 2 above the
> release pipeline on the grounds that it depends on none of the packaging work, and
> that reading was right — it landed alongside `payload_root()` without touching it.
> One correction to the recommendation below: the conventional `+1000 per ABI` scheme
> named here is Google's own `rank * 1000 + base`, and §29.3 deliberately does **not**
> use it, because it is not monotonic in the base. `base * 10 + rank` is.

Two separate problems; the second is not recorded anywhere and is worse.

1. **It defaults to 1 and never auto-increments** (`build_cmd.rs`, `.unwrap_or(1)`).
   Play rejects a re-upload at a version code it has already seen, so every release
   after the first needs a manual `[tool.zkite.android] version_code` bump. Already
   named in phase_2/PLAN.md §9; still open.
2. **Every per-ABI APK gets the *same* version code — a real Play blocker for the
   multi-APK flow, previously unrecorded.** `build_cmd.rs` builds the manifest **once**
   (`Package { android_manifest: manifest_gen::android_manifest(&config.identity) }`)
   and then loops over ABIs, so `testapp-arm64-v8a.apk` and `testapp-x86_64.apk` carry
   byte-identical `versionCode`. Google Play's multiple-APK support **requires each APK
   to have a distinct version code**; the standard convention is an ABI-derived offset
   (e.g. `+1000` per ABI, leaving room to iterate the base). The ceiling is
   `2100000000`, so an offset scheme has to stay well under it.

   This does not affect `zkite build aab`, which is one artifact — and AAB is the
   format Play requires for new apps anyway, which is presumably why nobody hit it. It
   does affect anyone shipping APKs directly, and it fails at upload with a confusing
   message rather than at build.

**Recommendation:** derive `version_code` from `[project] version` when not set
explicitly (so the two cannot drift), apply a per-ABI offset in the APK path, and
assert the result is under the Play ceiling at build time — the same "fail loudly at
build, not at upload" posture `assert_native_libs_are_sane` already takes for 16 KB
alignment.

### 17.5 The CLI is Linux x86_64 only

`zkite` is one Rust binary. Nothing about it is Linux-specific — `phase_2/bundler`,
`pack`, `rsa`/`rcgen` are all portable — so this is a release-pipeline gap
(cross-compile and ship five targets: linux-x86_64, linux-aarch64, macos-aarch64,
macos-x86_64, windows-x86_64), not a design gap. It is only *reachable* once §17.2's
host-Python question is answered, since a macOS `zkite build` that then cannot find a
Python 3.14 has not actually gained the user anything.

`zkite dev` is the portable half today and needs none of it: it pushes `.py` and never
compiles anything.

### 17.6 What I would do, in order

1. **§17.2's release-pipeline `.pyc`** — compile the stdlib and `zkite_py` in our own
   pipeline. No decision required about anyone's machine, and it removes the
   first-launch cost §17.1 exposed.
2. **§17.3 signing** — the `jks` reader plus `zkite fingerprint` (with the upload-key
   vs. app-signing-key distinction in its own output). Small, contained, and it is what
   unblocks a real developer with a real existing app.
3. **§17.4 version codes** — smaller than either, and it is a correctness bug rather
   than a feature.
4. **§17.2's `requires-python` decision** — needs the owner, not an engineer.
5. **§17.5 CLI cross-compilation** — last, because it is only useful after 4.

### 17.7 Known gaps in this section

- **The startup A/B is one device, one app per backend.** Moto G54 only. The native
  baseline for the nav demo (2584 ms) is much worse than §13.3's recorded 1450 ms for
  `cmp_counter`, and that gap is **unexplained** — different app (13 nodes vs 6) and a
  different session, so the two are not comparable; the A/Bs are same-session and are
  what should be trusted. A `cmp_counter` run under the new setting measured 1051 ms.
- **The purge's cost is inside run-to-run spread**, so "≈100 ms" is an upper bound, not
  a measurement. It was not isolated.
- **Nothing here was verified on the WebView Dev Shell**, only on `appshell` and the
  native shell. `devshell` got the same one-line change for parity, unmeasured.
- **The `jks` crate was read from its metadata and docs, not used.** Whether its
  `PrivateKeyEntry` maps cleanly onto `pack_sign`'s `Keys` is unverified, and its
  PKCS#12 support delegates to `p12-keystore`, which is a second dependency to
  evaluate.
- **No Play upload was attempted** for any of §17.3/§17.4 — the same limit
  phase_2/PLAN.md §8 already records.

## 18. `Icon` paints its sprite — and the paint vocabulary that was actually missing

§15.1's first entry under "missing rendering features on native":

> **`Image` and `Icon` paint an empty box of the correct size** (§9.2 item 4) — no
> bitmap decode, no vector rendering, no network image loading or caching. An app with
> a logo shows a blank rectangle.

**Status: `Icon` is done — all twelve bundled sprites, verified live on the Moto G54 at
three inherited sizes and four inherited colours. `Image` is deliberately not done, and
§18.6 explains why it is a different project rather than the other half of this one.**

### 18.1 The framing was narrower than the inventory reads, and grouped wrong

Checked before starting, not taken from the note. **The native paint path draws exactly
two things**: a solid rounded-rect background and glyph runs (`render_thread.rs`'s
`paint_nodes_into_scene`/`paint_cpu_fallback`, `conformance`'s `render_one`). Every
entry under §15.1's "missing rendering features" heading is downstream of that one
narrow vocabulary rather than of five separate absences.

And the primitives were already there. Read directly out of the vendored sources, not
inferred from crate descriptions — **`vello_hybrid::Scene` and
`vello_cpu::RenderContext` expose an identical set**:

| §15.1 item | primitive it needs | already present on **both** backends |
|---|---|---|
| `Icon` | fill/stroke an arbitrary path | `fill_path` · `stroke_path` · `set_stroke` |
| borders, rings | stroke the rounded rect the background pass already builds | the same three |
| shadows | a blurred rounded rect | `fill_blurred_rounded_rect` — a box-shadow primitive outright |
| gradients | a gradient brush | `set_paint` takes `PaintType = peniko::Brush<Image, Gradient>` |
| `Image` | an image brush | the same `Brush`, plus `ImageSource`; `draw_texture_rects` on hybrid |

So **§15.1 item 3's "the paint path only fills solid colours, so a gradient brush has to
exist too" is true of ZKite's own `Paint` enum and overstates the cost** — the brush
exists in a crate this project already links, and the same is true of the shadow
primitive. `kurbo 0.13`, which `zkite_renderer` already depends on directly, also has
`BezPath::from_svg`, so SVG path data parses for free.

Two more things that were already in place and unread:

- **`twcore` resolves all of it.** `PositionedNode` carries `Arc<ComputedStyle>`
  verbatim to every paint site, so `paint.border_color`, `paint.shadow`, `paint.ring`
  and `text.color` are already *at* the call site.
- **Border width goes to `style.layout.border`** (`decl.rs`), which taffy already
  reserves. Painting a border is therefore purely additive and cannot move a pixel of
  geometry — a rare property, and the reason §18.6 puts borders next rather than
  `Image`.

### 18.2 One sprite sheet, both backends — the §9.6 move again

The sprites are drawn twice: the WebView renders `<use href="#icon-star">` against the
`<symbol>` sheet in `phase_1/webruntime/index.html`, and native rasterises the same
geometry. Two hand-maintained copies of one design is the shape this project has already
paid for twice — §9.5 (`twcore`'s palette was Tailwind v3 while the WebView ran v4, and
every conformance run reported it clean) and §9.7 (native shaped DejaVu while Chromium
resolved `sans-serif` to Noto Sans).

Both were fixed by pointing the second consumer at the file the first one actually runs,
and that is what `phase_3/renderer/scripts/gen_icons.py` does: it reads `index.html`'s
own `<symbol>` blocks and writes `renderer/src/icons.rs`. The two backends cannot drift
without someone editing that sheet, at which point re-running the generator is the
obvious next step.

**The generator resolves SVG's presentation attributes rather than assuming them**, and
those defaults are not what a reader assumes. `fill` absent means **black**, not "no
fill" — `icon-search`'s second path omits `fill="none"` (an authoring slip in
`index.html`), so per spec it is filled black *and* stroked. It is a two-point open line
with zero area, so the fill paints nothing in either engine. It is recorded faithfully
anyway rather than normalised away, because silently "fixing" the source in the
transcriber is exactly how the two would drift again.

`<circle>` becomes two SVG arcs at generation time, so the runtime handles one shape
kind and `BezPath::from_svg` is the entire parser.

### 18.3 Where each piece lives, and why

- **Geometry in `zkite_renderer`, paint in each backend** — the split §3's module doc
  already sets. `ResolvedIcon::transform` (uniform scale + centring, i.e. SVG's own
  `preserveAspectRatio="xMidYMid meet"` default), `IconPaint::resolve` (`currentColor` →
  the node's resolved text colour) and `IconStroke::to_kurbo` are all pure geometry, so
  they live in the shared crate and **all four paint consumers** — `gpu_harness`'s two
  paths, `conformance`, `spike1` — get one implementation. Four re-derivations is how
  §9.2 item 3's two `Switch` designs drifted apart. `kurbo` is a geometry crate with no
  rasteriser, so this does not breach the crate's "no vello dependency" rule.
- **Paths are parsed once per process, never per frame.** `Renderer::icon_geometry` is
  keyed by sprite name and deliberately survives `clear()`, same bucket as `font_cx` and
  the shape cache. It is bounded by the bundled set (12) rather than by anything an app
  does, so unlike `twcore`'s own caches (§11.7) it cannot grow with use. Each backend
  sets one `Affine` and reuses the unit-space path — putting SVG parsing on the render
  hot path is the mistake §11.9 spent a whole section removing from text shaping.
- **The icon transform is composed onto the density transform, then restored.**
  `set_transform` is absolute, so leaving an icon's transform in place would scale every
  node painted after it.
- **Painted between backgrounds and text**, so an icon covers its own background and
  never a sibling's glyphs — the order the WebView gets free from DOM stacking.

### 18.4 The real finding: `1em` was reachable all along, and the fixture hid it

`Icon.implied_classes` is `w-[1em] h-[1em] fill-current`. `twcore` has no `em` unit and
cannot usefully have one — `em` resolves against the *inherited* font size and a
`ComputedStyle` is ancestry-free by design — so `_tree_classes` substituted a hardcoded
`w-[20px] h-[20px]` and both this document (§9.2 item 4) and WIDGETS.md described it as
an approximation native "cannot do".

**That premise was wrong. `zkite_renderer` is precisely the layer that resolves
inheritance**: by the time `Renderer::icon` runs, `resolve_inherited` has walked the
stack and `style.text.size` is the real font size for that node. `icon()` now defaults
each `auto` axis to it — a true `1em`, not an approximation of one — and an explicit
`w-8 h-8` still wins because only `auto` is filled in. `_tree_classes` emits no size at
all.

**Nothing caught this, and the reason is worth recording.** The conformance `icon`
fixture uses `text-[20px]`, which is exactly the hardcoded constant, so native and the
WebView agreed by coincidence and the fixture scored 1.22%. It is the same shape as
§12.5.3's list fixture (`h-[300px]`, bounded by construction, so it could never exercise
what a real app does) and phase 2 §6/M1's `catch_unwind` test: **a test that passes for a
reason unrelated to what it claims to check.**

It was found by adding a fixture at a *different* size for an unrelated reason — to
exercise the one piece of geometry the generator *derives* rather than copies (the
circle → two-arc conversion in `search`/`settings`, which are also the only multi-shape
sprites, so a paint loop that drew only the first shape would pass the `star` fixture).
That fixture scored **4.82%** on its first run, and the diff image showed both sprites
drawn correctly and uniformly too small. Fixing `1em` took it to 0.22%.

### 18.5 Verification

**Host — conformance, and only one fixture moved.** 15 fixtures × 2 dprs:

| fixture | before (dpr1 loose/strict) | after |
|---|---|---|
| icon | 1.22 / 1.37 | **0.05 / 0.29** |
| icon_circles (new) | — | **0.22 / 1.44** (4.82 before the `1em` fix) |

At dpr 2, `icon` is **0.01 / 0.16**. **Every other fixture is unchanged to two decimal
places at both dprs** against §13.1.2's table — `button` 1.78/3.20, `text` 5.00/6.58,
`textfield` 1.89/4.07, `typography` 9.95/13.68, and the rest. The diff images were read,
not just the numbers (§9.5's standing lesson): what remains on both icon fixtures is a
handful of red pixels on tight curves and one miter join, i.e. the AA floor §9.1
describes.

**Host suites:** `zkite_renderer` 18 → **22**, `zkite_py` 102 → **103**, `zkite_pybridge`
6/6, `twcore` unchanged, `webruntime` e2e 23/23.

Three of the four new renderer tests exist because this could be fast and wrong:
`icons_parse_and_stay_inside_their_viewbox` (**confirmed to fail against the bug** — the
generator's circle conversion was temporarily given the diameter where the radius
belongs, and it reported `` `search` shape 0 has bbox … outside its 0..24 viewBox ``; a
first attempt to break it via the arc *sweep flag* correctly did **not** fail the test,
because a sweep flag changes direction of travel and not position, which is a small
negative result worth keeping), `an_unsized_icon_is_one_em_of_the_inherited_font_size`
(checked at 32px and 12px, deliberately never 20, since 20 is the value the old constant
agreed with by coincidence), and
`an_icon_leaf_carries_geometry_and_occupies_the_same_box_as_before`.

**Device — Moto G54 5G (arm64-v8a, Android 15), GPU path.** `demo_app/icons.py` renders
all twelve sprites on three rows at `text-[20px]`/`text-[32px]`/`text-[13px]` in
white/indigo-400/emerald-400, plus a row of explicitly-sized 44px icons inside 13px text,
plus an unknown sprite name. The screenshot shows every sprite drawn correctly; the three
rows are visibly three different sizes tracking the inherited font size; each row takes
its inherited colour; `search` and `settings` both draw both of their shapes; the
explicit `w-[44px]` beats the `1em` default; and the unknown name draws nothing while the
text after it still sits where an empty 32px box puts it.

`uiautomator dump` afterwards reports every icon by name (§8.2's `icon_name` label
fallback still works through the new leaf), so §8.7's accessibility-abort guard held
across the dump too. Zero `FATAL EXCEPTION`/`E AndroidRuntime`/`PANICKED` markers across
the whole session, process alive throughout.

The packed `.so` was checked with `strings` for the sprite table before installing, per
§15.6.1 — verify the artifact, not the build log.

### 18.6 What this deliberately does not do

- **`Image` is untouched, and it is not the other half of this.** `Icon` needed a vector
  path into a paint call. `Image` needs a decoder dependency, a cache keyed by `src`, and
  eventually async network fetch — and it has a staging problem this work surfaced:
  ~~`build_cmd.rs` zips the whole app directory into `assets/app.zip`, so a `logo.png` next
  to `main.py` lands **inside a zip** and never on the filesystem, with no path to hand a
  decoder.~~ That is its own milestone. Note also that `zkite new` scaffolds no image, so
  §15.1's "an app with a logo shows a blank rectangle" is a real gap for a user who adds
  one rather than something the stock app exhibits.

  > **`Image` is built — see §35 — and the struck-through sentence above was wrong in
  > both halves.** It was written from the shape of the build rather than from the code,
  > and §35 corrected it by reading `build_cmd.rs`:
  >
  > - The file **never reached the zip**. `copy_app_source` staged `*.py` and nothing
  >   else, so a `logo.png` was dropped before any zip existed. The zip was never the
  >   obstacle, and the fix is an extension allowlist rather than a decoder that can read
  >   out of an archive.
  > - A file that *does* reach the zip is **not trapped in it**. `ZKiteActivity` unpacks
  >   `app.zip` into `appDir` on first launch on both shells, so a staged asset is an
  >   ordinary file with an ordinary path.
  >
  > The rest of the entry held: a decoder dependency and a cache keyed by `src` are
  > exactly what it needed, and network fetch is still deferred (§35.6). What the entry
  > could not have predicted is that **`Image` was broken on the WebView backend too** —
  > that shell loads its page from `file:///android_asset/webruntime/`, so a relative
  > `src` resolved to a nonexistent asset. §15.1 had it filed as a native-only
  > divergence; it was a plain gap on both backends.
- **Borders, rings and shadows are not painted**, though §18.1 is why they are the
  natural next increment: same two call sites, same already-flowing `ComputedStyle`, and
  purely additive against space taffy already reserves. **One honest correction to that
  estimate**, found while scoping: a *uniform* border is one stroked rounded rect and is
  genuinely cheap, but Tailwind allows per-side widths and colours (`border-t-2
  border-b-4`), and `twcore` already models both (`BorderColors`, `Rect<LengthPercentage>`).
  Per-side borders need four separate edge paths, not one stroke, so the general case is
  meaningfully more than the uniform one. Sizing it as "trivial" would be wrong.
- **Gradients are not parsed.** §15.1 item 3's *other* layer — `twcore` has no
  `bg-gradient-*`/`from-*`/`via-*`/`to-*` at all — is untouched and is the real work
  there now that the brush is known to exist.

### 18.7 Known gaps

- **Twelve sprites is a placeholder set, and `index.html` says so.** Nothing here makes
  it a real icon library; it makes the one that exists actually render.
- **No `Icon(label=...)`/`decorative=True`.** §8.6's gap is unchanged — the accessible
  name is still the sprite name.
- **The generator handles `<path>` and `<circle>` only**, and rejects anything else
  loudly rather than dropping it. A `<rect>`/`<polygon>`/`<g transform>` in a future
  sprite needs a converter added.
- **Non-square and offset viewBoxes are rejected**, not supported — the transform is one
  uniform scale. Every bundled sprite is `0 0 24 24`.
- **`icons.rs` is generated but nothing asserts it is in sync with `index.html`.** A
  hand-edit of the sheet without re-running `gen_icons.py` would diverge silently, which
  is the exact failure mode §9.6 designed against. A regeneration check belongs in the CI
  that does not exist yet (§9.3).
- **The device run is one device, one ABI, the GPU path.** The CPU fallback path carries
  the identical code but was not forced on and re-run this session (§4 did that for its
  own work by hand-editing `try_init_gpu`); `conformance` exercises `vello_cpu`'s
  rasteriser on the host, which is evidence but not the same claim.
- **Icons inside a scrolled container are clipped by the same `clip` path as every other
  node** and that path is exercised by no fixture — `scrollview` has no icon in it.

## 19. The native error overlay — checked, and it works

Queue C item 4, and the one entry on that list whose answer was allowed to be "nothing
to do". Written up from a previous session's verification, which was run and never
committed.

**Status: verified live on the Moto G54. A Python exception in `view()` does *not*
produce a blank screen on the default backend. Nothing needed fixing.** Four residual
gaps came out of it and are named in §19.3; all four are real and none of them is the
thing this check existed to rule out.

### 19.1 What was checked, and what it showed

plan1.md §6 promises a traceback renders "as an overlay *on the device* with the failing
frame and source line, and simultaneously in the terminal". §5 built that for the WebView
backend in phase 1. Nothing had ever confirmed the native backend does it — which
mattered, because the failure mode if it didn't is the worst one available: the default
backend showing a black screen for every app-level bug, with the traceback nowhere.

`demo_app/error_repro.py` (five lines, `view()` does `1 / 0`) built through
`build_apk.sh` and launched. The full traceback renders **red-on-black, full screen**,
with the failing source line and CPython 3.14's `^^^` caret annotations intact and
`ZeroDivisionError: division by zero` on the last line. No crash: `ZKITEPERF first-frame
nodes=5 via=gpu`, no `FATAL EXCEPTION`, no `E AndroidRuntime`, process stable afterwards.

**The plumbing was already complete and correct**, which is why there was nothing to fix.
`App.render_tree` (`phase_1/zkite_py/src/zkite/app.py`) catches any `view()` exception,
formats it with `zkite.errors.format_error_tree`, and merges it as a second child over the
last good tree; `_boot_app_tree` (`phase_1/devshell/src/runtime.py`) never sees the
exception at all. The native backend inherited a working overlay from the shared
`zkite_py`/`runtime.py` layer without anyone checking that it had.

Re-confirmed incidentally during §20's own device work, on a different build and a
different memory configuration — the screenshot there shows the identical overlay.

### 19.2 `error_repro.py` is kept

Five lines, no dependencies, selected like every other demo via `ZKITE_APP_MAIN`. It is
the cheapest possible fixture for the overlay path and for the *smallest tree the
renderer can draw*, and §20 used it for exactly that second purpose within a session of
it being written. Kept rather than deleted.

### 19.3 What the overlay still doesn't do

All four are real. The first was seen on screen; the other three are unverified rather
than known-broken, and are stated that way.

- **Long lines clip instead of wrapping.** The traceback's
  `File "/data/user/0/dev.zkite.gpuharness/files/zkite_framework/zkite/app.py"` lines run
  off the right edge and are cut, with no continuation line. Every native traceback hits
  this, because that path prefix is unavoidable and is already nearly a screen wide on
  its own. Re-confirmed visually in §20's run (`.../zkite/app.` is where it stops).
  **A process failure worth keeping:** a careful read of parley's line breaker predicted
  `BreakReason::Emergency` would force-break an unbreakable token, and the device showed
  it does not. A code read and the hardware disagreed and the hardware was right — the
  same lesson §7.14 paid several sessions for, in miniature. Start from the screenshot.
- **The terminal half of plan1.md §6 is unverified on native.** `_zkite_last_error` exists
  and the reload socket mirrors it, but it is only driven by `zkite dev`; the verification
  launched via `am start`, so that path never ran. Needs one `zkite dev` session against a
  broken app.

  > **Sharpened by §46.8.1, and it is worse than "unverified".** Launched via `am start`,
  > an app whose `view()` raises on **every** frame writes **nothing to logcat on any
  > tag** — not the traceback, not a warning, not a counter. Grepped for
  > `ZeroDivision|Traceback|division by zero` across the *whole* buffer, not just
  > `ZKiteGpuHarness`: zero hits, while the overlay was on screen showing that exact text.
  > The overlay is the *only* channel outside a `zkite dev` session.
  >
  > **Closed in §47.1.** `zkite.errors.log_error`/`log_recovered`, called from
  > `App._note_error`/`_note_rendered`, write the traceback to stderr — which every
  > shell already tees to logcat — guarded on the error *changing*, so a `view()` that
  > raises on every frame logs one burst rather than sixty a second, and a recovery
  > logs one line so a gate can see an app come back. Fixed in `App`, so it lands on
  > **both** backends at once. Six host tests, three negative controls.
  >
  > **The consequence is about tooling, not about the overlay.** §46.8.1's device sweep
  > gates each fixture on `FATAL EXCEPTION`/`E AndroidRuntime`/`PANICKED` being zero, and
  > `error_repro` — an app that is 100% broken by construction — passed that gate with
  > `fatal=0` and a healthy-looking `nodes=14`. **A logcat-based gate cannot distinguish a
  > working app from one displaying a traceback**, which is precisely the shape of check
  > plan1.md §14.5's absent CI would be built from. The cheap fix is for the overlay path
  > to log one line as well as painting; the sweep's own workaround would be to read the
  > screenshot, which is what §35 already says to trust.
- **Only the boot-time error was tested.** Untested: an app that renders fine and then
  hot-reloads into a broken `view()` (does the overlay appear?), and then reloads back to
  a working one (does `last_error = None` clear it?). The code says both work; the device
  has not said so. This is the same shape as §15.3's "only the counter app was reloaded",
  now with a third thing worth reloading.
- **The two backends' overlays probably differ in opacity, and the native one may violate
  §6's wording.** `format_error_overlay` (WebView) uses `rgba(20,0,0,.92)`, so the failing
  frame shows through — which is what §6's "with the failing frame" asks for.
  `format_error_tree` (native) uses `bg-black`, Tailwind's opaque `#000`. At boot there is
  no previous frame so this is invisible, but mid-session the native overlay would fully
  occlude the frame the WebView deliberately shows. **Inferred from the class strings, not
  measured** — it needs a check of `twcore`'s `bg-black` handling and a mid-session error
  before it should be believed.

## 20. The 272 MB of `Graphics`, attributed — it was never the renderer

§13.6 named this *"the single largest item on the wrong side of the comparison,"* to be
attributed *"before anyone concludes the native backend is intrinsically memory-hungry."*
§14.7 put a control under it (the WebView composites the same surface for 101 MB),
refuted the double-allocation theory, and left a concrete next step: *"logging
`adapter.limits()` and instrumenting `Renderer::new`'s allocations."* §15.5 recorded that
flipping the default made it more pressing. It had been the named next step for four
sessions.

**Status: attributed and reduced, measured on the Moto G54 across three configurations.
It is not vello_hybrid, not the swapchain, and not sized by the content. Nearly all of it
is `gpu-allocator`'s suballocation-block reservation, inherited from a wgpu default ZKite
never set.** Total PSS for the stock counter falls **~334 MB → ~155 MB**, and for the
10,000-row list **~314 MB → ~135 MB**, with scroll frame time unchanged.

### 20.1 The answer, and why the search had been pointed one layer too low

`try_init_gpu` built its `wgpu::DeviceDescriptor` with `..Default::default()`, which
supplies `memory_hints: MemoryHints::Performance`. That is wgpu's own default and it is
not neutral. Read out of `wgpu-hal`'s `AllocationSizes::from_memory_hints`:

| `MemoryHints` | min device block | max device block | min host block | max host block |
|---|---|---|---|---|
| **`Performance`** (wgpu's default) | **128 MB** | 256 MB | **64 MB** | 128 MB |
| `MemoryUsage` | 8 MB | 64 MB | 4 MB | 32 MB |
| `Manual { start..end }` | `start` | `end` | `start/2` | `end/2` | 

Those feed `gpu-allocator`, which reserves a whole `VkDeviceMemory` block of at least the
minimum the first time an allocation of each kind is made, independent of how much is
used, and **grows subsequent blocks toward the maximum** (`get_memblock_size`).
`adjust_memblock_size` clamps to 4–256 MB and there is **no scaling against heap size** —
checked in `gpu-allocator 0.28`'s source, not assumed.

**§14.7's own next step was aimed one layer too low.** It reasoned from `vello_hybrid`'s
sources that "several of its textures are sized `max_texture_dimension_2d` wide" and made
logging that limit step one. The limit is now logged — **16384**, which had genuinely
never been recorded — and it is not the cause. `Renderer::new` creates every one of those
`max_texture_dimension_2d`-wide textures at **height 1**
(`INITIAL_ALPHA_TEXTURE_HEIGHT`, `INITIAL_ENCODED_PAINTS_TEXTURE_HEIGHT`,
`INITIAL_GRADIENT_TEXTURE_HEIGHT`, `INITIAL_FILTER_TEXTURE_HEIGHT` are all `1`), i.e.
256 KB each, and `AtlasConfig::default`'s `initial_atlas_count` is `0`, so the image atlas
is a 1×1 placeholder. vello_hybrid's construction-time footprint is single-digit MB.

That is worth recording as the inverse of §11.8's lesson. There, the code was provably
right and the missing step was measuring the artifact. Here the *suspect* was plausible
and the missing step was reading one layer further **up** the stack — ZKite's own call into
wgpu — rather than further down into the renderer it calls.

### 20.2 The measurement, and a false result caught on the way

`ZKITE_MEMORY_HINT` is a named constant so each arm is one line between builds — the
technique §4 used for the CPU fallback, §11.9 for the shape cache and §13.2 for pacing.
Moto G54 5G, arm64-v8a, Android 15, GPU path, warm-up launch discarded (§17.1 makes
launch-1-after-install unrepresentative), force-stop between runs, focus polled rather
than slept for, only the single process's own detail table read (§13.3's double-counting
trap).

**One more piece of discipline had to be added mid-session, because without it this
section reported a result that was ~40 MB too good.** The first `MemoryUsage` runs read
`GL mtrack` 44 MB with **no `EGL mtrack` line at all**; a later run of the identical build
read 79 MB with `EGL mtrack` present. `adb shell svc power stayon usb` does not hold on
this device, and a run caught it in the act: `[mScreenState=OFF focus=NOT-OURS]` → 44 MB,
`[mScreenState=ON focus=dev.zkite.gpuharness]` → 79 MB. **A display-off `SurfaceView` has
its swapchain released, so a `dumpsys meminfo` taken then is measuring a backgrounded app,
not a running one.** Every number below is from a run that asserted screen-on *and*
foreground focus at the moment of the dump. This is a new standing device trap and belongs
with the `screencap`-returns-black one in §13.5 — same cause, same shape, same fix
(assert the display state, don't assume it).

**All three configurations, screen-on and foreground, 2–3 runs each:**

| `cmp_counter` (6 nodes) | `Performance` | `MemoryUsage` | `Manual{4..8 MB}` |
|---|---|---|---|
| `GL mtrack` | 214360 – 214362 KB | 79189 KB | **35592 KB** |
| `EGL mtrack` | 66956 KB | 66956 KB | 66956 KB |
| `Graphics` total | 281316 – 281318 KB | 146145 KB | **102548 KB** |
| **TOTAL PSS** | 333051 – 334407 KB | 186686 – 195308 KB | **155053 KB** |

| `cmp_list_10k` (152 nodes) | `Performance` | `MemoryUsage` | `Manual{4..8 MB}` |
|---|---|---|---|
| `GL mtrack` | 215154 – 215160 KB | 79989 – 80012 KB | **36393 – 36395 KB** |
| **TOTAL PSS** | 313623 – 313983 KB | 178984 – 182554 KB | **135002 – 135049 KB** |

**`GL mtrack` is flat across content at every setting, and tracks the block size
instead.** 214 MB for a 5-node app (`error_repro`, 214358 KB), a 6-node app (214362 KB)
and a 152-node windowed list (215157 KB) — a 30× content range with no effect. Then
128/64 MB blocks → 214 MB, 8/4 MB blocks → 79 MB, 4/4 MB blocks → 36 MB. That is the
attribution: **the number was never measuring what ZKite draws, at any setting.** What the
app genuinely needs is under 36 MB, and even that figure is still block granularity rather
than a measured floor.

**This corrects a claim an earlier draft of this section made.** Working from the
contaminated 44 MB counter reading, it said that under `MemoryUsage` the allocation
"tracks the content." It does not — 79189 KB (counter), 79193 KB (`error_repro`), 79989 KB
(list) is as flat as the `Performance` arm. The contrast was an artifact of comparing a
screen-off counter against screen-on apps. The reservation shrinks; it does not become
content-proportional.

**§14.7's own figure is reproduced**, which is the check that this is the same phenomenon:
it recorded 272.5 MB of `Graphics` for the stock counter, and the `Performance` arm here
reads 281.3 MB.

### 20.3 The performance side, measured rather than assumed

wgpu names this a memory-versus-performance tradeoff, so taking the memory win without
pricing the other side would be exactly the kind of claim this document does not make.
`cmp_list_10k`, 8 swipes per run, the method and workload §13.3 used:

| `cmp_list_10k`, frame interval avg | `Performance` | `MemoryUsage` | `Manual{4..8 MB}` |
|---|---|---|---|
| across all sampled windows | 16.90 – 17.53 ms | 16.84 – 17.36 ms | 16.84 – 17.55 ms |
| effective frame rate | ~58 fps | ~58 fps | ~58 fps |

**Indistinguishable, and all three match §13.3's recorded 16.91–17.53 ms** for this app on
this device — so §13.2's pacing work is undisturbed and the memory win is free on this
workload. Two runs per arm plus a discarded warm-up; zero
`FATAL EXCEPTION`/`E AndroidRuntime`/`PANICKED` markers in any run of any arm.

That is not surprising once the mechanism is clear. Larger blocks buy *fewer*
`VkDeviceMemory` allocations, which matters to workloads that allocate GPU memory
continuously. ZKite allocates almost nothing per frame — §11.9's shape cache and §18.3's
icon-path cache exist precisely so steady-state rendering reuses rather than allocates —
so block size has nearly nothing to bite on.

**Shipped: `Manual { 4 MB .. 8 MB }`**, since it is strictly the lowest-memory arm with no
measured cost on either app. `MemoryUsage` would be the more conservative choice (a named
preset wgpu maintains, rather than two pinned constants) and is 44 MB worse on the list;
the constant's own doc comment records both so the choice can be revisited rather than
rediscovered.

### 20.4 What this does to §13.4's ledger

§15.4 flipped the default to native as an owner decision *against* the measured ledger,
naming ~310 MB resident as one of three things arguing the other way. That line changes:

| `cmp_counter`, total PSS | before | after |
|---|---|---|
| native | ~334 MB | **~155 MB** |
| WebView (§13.3, 2 procs) | 235 MB | 235 MB, unmeasured this session |

**Native now uses materially less memory than the WebView backend on the apps measured
here**, where §13.3 had it losing on all three. `cmp_list_10k` comes in at ~135 MB native
against §13.3's WebView 291 MB, pointing the same way.

Two honest limits on that. The WebView numbers are **from a different session** and are
not a same-session A/B, so the cross-backend comparison is weaker than the native-vs-native
one. And the other two ledger entries — 15.5 MB of binary and ~490 ms of cold start — are
completely untouched by this work.

**This does not reopen §15.4.** That decision was made on control over rendering and
explicitly not on these numbers; it does not need this result and should not be
retrospectively re-argued as if it did. What the result does is remove the specific
concern that motivated §14.7: a default ZKite app costing ~310 MB on a 4 GB phone is the
number that gets it background-killed, and it is now ~135–155 MB.

### 20.5 Known gaps

- **`EGL mtrack` is untouched and unattributed.** 57–67 MB, and it is the swapchain —
  allocated by Android around the `SurfaceView`, not by wgpu's allocator, so no
  `memory_hints` value affects it. It is now the largest single unexplained GPU item. A
  swapchain at 1080×2223×4 B is ~9.6 MB an image, so ~67 MB is roughly seven images'
  worth, which is more than a triple-buffered surface should need. **This is the natural
  next step for whoever picks this up**, and it is a different mechanism from the one this
  section closed.
- **The residual `GL mtrack` is bounded, not itemised.** 36 MB under the shipped setting,
  still flat across content, so it is still mostly block granularity rather than real
  usage — the true floor is somewhere below it and was not measured. Nothing here
  instrumented individual wgpu allocations; the claim is that the number tracks block size
  and not content, which the three-arm table shows, not that every megabyte has a name.
- **One device, one ABI, one GPU vendor.** Moto G54, arm64-v8a, PowerVR, Vulkan. The
  block-size mechanism is in wgpu-hal and is vendor-independent, but whether a driver
  *accounts* a reserved-but-untouched `VkDeviceMemory` block against the process is a
  driver question — and it plausibly explains why §12.6's Samsung/Mali run showed only
  77 MB of `Graphics` where this phone shows 214 MB for the same code. Hypothesis, not
  measurement; the Samsung is not available.
- **`Manual{4..8 MB}` is tuned to an app that draws no images.** When `Image` lands
  (§18.6), a real atlas allocates far more GPU memory than anything ZKite draws today, and
  small blocks mean more `VkDeviceMemory` allocations. Resources larger than a block get
  dedicated allocations, so there is no correctness risk, but the setting should be
  re-measured then rather than inherited.
- **The WebView backend was not re-measured this session**, so §20.4's cross-backend
  comparison mixes this session's native numbers with §13.3's WebView ones.
- **`repro_atlas_lifetime` still requests a device with `..Default::default()`.** A
  host-only diagnostic with no memory claim attached, left alone rather than changed for
  symmetry.
- **No conformance or host-suite impact, and none expected** — this changes an allocator
  parameter, not any geometry or paint code. Not re-run.

## 21. App identity — and the `pip install zkite` audit it turned into

Started as a small question from the owner: *how does someone set their app's logo,
label, and package name, and can they do it after `pip install zkite`?* The first half
had a two-thirds answer and one documented feature that does not exist. The second half
is the useful part: **`zkite build` today works on exactly one machine — the one holding
this checkout — and the reason is one line of Rust, not the cross-platform gap §17.5
already records.**

Two decisions are recorded here, both the owner's: `pyproject.toml` is the interface
for app identity (no CLI subcommand), and `zkite new` now scaffolds it. Icon support is
deferred, with its shape written down in §21.3.

### 21.1 What is settable today, and the one key that was never implemented

Read out of `build_cmd.rs::load_project_config` rather than from any doc:

| key | table | read at | fallback if absent |
|---|---|---|---|
| `package` | `[tool.zkite.android]` | `build_cmd.rs:347` | `dev.zkite.app.<sanitized-name>` + a warning |
| `label` | `[tool.zkite.android]` | `build_cmd.rs:356` | `[project] name` |
| `version_code` | `[tool.zkite.android]` | `build_cmd.rs:360` | `1` (§17.4 — Play rejects the re-upload) |
| `version_name` | `[project] version` | — | required by `[project]` |
| `backend` | `[tool.zkite]` | `build_cmd.rs:341` | `Backend::DEFAULT` (native, §15.4) |
| **`icon`** | — | **nowhere** | — |

**`icon` is not read by any code in this repo.** `AppIdentity` has no icon field, the
generated manifest hardcodes `android:icon="@drawable/icon"`, and the resource comes
from:

```rust
fn default_icon(repo_root: &Path) -> Result<Vec<u8>> {
    Ok(std::fs::read(repo_root.join("phase_2/appshell/app/res/drawable/icon.png"))?)
}
```

No key is consulted, no path is resolved, no override exists. Every app ever built with
`zkite build` ships the same bundled icon.

phase_2/PLAN.md §3 states, in the present tense, that `[tool.zkite.android] icon =
"path/to/icon.png"` **is** the override, and §9's gap list repeats it as though the
manual override were the *shipped* half of a deferred pipeline. It was never built.
This is the same failure shape §16.5 and §7.6 record from the other direction — a
claim whose only evidence is the doc making it — and it is worse here because the doc
reads as a feature list a user would act on. phase_2/PLAN.md §3 and §9 are corrected in
place, pointing at §21.3.

There is also **no CLI surface for any of this**: no `zkite config`, no `--package` /
`--label` / `--icon` on `zkite build`. The subcommands are `new`, `dev`, `build`,
`keygen`, `doctor`.

### 21.2 Decision: `pyproject.toml` is the interface, and `zkite new` now scaffolds it

**Owner decision (2026-08-08): no CLI subcommand for identity. Hand-edited
`pyproject.toml` is the whole interface.** The reasoning, written down because the
opposite is the more obvious-looking choice:

- Identity is **persistent per-project state**, not a per-invocation choice. A flag has
  to be repeated on every build, and the one that would hurt most is `version_code` —
  a release built without the flag silently ships the wrong number.
- A `zkite config` that writes TOML has to round-trip a file the user also hand-edits,
  preserving comments and ordering. That is a real dependency (`toml_edit`) and a real
  class of bug, bought for a field that is set roughly twice in a project's life.
- `[tool.<name>]` is what every Python tool already does. A ZKite user editing
  `pyproject.toml` is not learning a ZKite thing.

**The actual problem was never the mechanism — it was that nothing tells you the keys
exist.** `zkite new` wrote a four-line `pyproject.toml` with no `[tool.zkite.android]`
table at all, so a fresh project gives no hint, and the keys are documented only in
phase_2/PLAN.md and one warning string that fires *after* you have already built.

**Built this session.** `zkite new` now scaffolds:

```toml
[project]
name = "my-cool_App"
version = "0.1.0"
dependencies = ["zkite"]

# How your app appears on the device. `package` is permanent — Play ties your
# listing to it and it can never be changed after your first release — so replace
# `com.example` with a domain you own before you publish. `zkite build` warns while
# it is still set to this.
[tool.zkite.android]
package = "com.example.my_cool_app"
label = "my-cool_App"
version_code = 1
```

Three things in that are deliberate:

1. **`com.example.*`, not a plausible-looking default.** `com.example` is reserved by
   convention and Google Play rejects uploads under it. That is the desired property:
   the scaffolded value **cannot** be shipped by accident. Paired with a build-time
   warning (below), the failure lands at `zkite build` with a message that says what to
   do, instead of at upload with Play's.
2. **The package segment is sanitized, the label is not.** `zkite new` accepts `-` in
   project names; package segments do not. `zkite new my-app` scaffolding
   `com.example.my-app` would have failed `validate_package` on the user's *first
   build* — a bug shipped by the fix. Hence `sanitize_package_segment` in
   `manifest_gen.rs`, shared with `placeholder_package` so the two can't drift, and a
   test that asserts a scaffolded package validates.
3. **The comment, not just the keys.** "Permanent, replace before publishing" is the
   one fact about `package` a first-time Android publisher does not know, and it is
   cheaper to say here than to document.

**`zkite build` now warns on either zkite-owned namespace**, `com.example.*` and
`dev.zkite.app.*`, via `manifest_gen::placeholder_namespace`. Both are legal package
names that install and run fine on a device — they only fail at *upload*, which is the
worst place to learn it. Same posture as `assert_native_libs_are_sane` for 16 KB
alignment, and the same posture §17.4 recommends for version codes: **fail loudly at
build, not at Play.** The old "no `[tool.zkite.android]` package set" warning moved to
this single post-resolution site so it can't print twice.

**On plan1.md §2's "boilerplate is a bug".** Nine lines are added to a file the user
already owns, and none of them is deletable-noise: every line is a value the user will
change or a fact they need. The alternative — a fresh project that says nothing about
identity — costs a user their first Play upload. This is the trade §2 intends, not a
violation of it.

**Verified:** `cargo test` green in both crates (21 CLI, all bundler suites including
`packed_apk_verifies_with_real_android_tools`), plus a real `zkite new my-cool_App` →
the file above verbatim, and a real `zkite build apk` on that project: the warning
printed first, and the scaffolded identity carried all the way through to
`com.example.my_cool_app/dev.zkite.gpuharness.ZKiteActivity` in both signed APKs. The
hyphen case is the one that matters and it is the one that was run. Not a device run —
nothing here changes what is on screen.

### 21.3 Icon support — deferred, with its shape written down

**TODO, not built.** Deliberately deferred by the owner this session. What it is, so
whoever picks it up doesn't re-derive it:

**The small version (~30 lines).** The manifest already references `@drawable/icon`;
the icon is a *resource*, not a manifest field, so `AppIdentity` does not change at
all. Read `[tool.zkite.android] icon`, resolve it relative to the project directory,
read those bytes at `build_cmd.rs:245` instead of calling `default_icon()`. Validate it
is a PNG and error with the path if it isn't — `pack`'s asset compiler does not
transcode, so a `.jpg` named `.png` would be packaged and fail to decode on device.
Falls back to the bundled icon when the key is absent, exactly as today.

**The version that is actually Play-quality is much bigger, and is blocked.** A real
Android icon is an `<adaptive-icon>` XML in `mipmap-anydpi-v26/` with separate
foreground/background (and now monochrome/themed) layers, plus per-density raster
fallbacks. phase_0 §7 recorded that **`pack` has no resource-qualifier support**, which
is why the Dev Shell sidestepped with one density-independent PNG in the first place.
So the adaptive path needs qualifier support in `pack` before any of it is expressible
— it is its own session, and it is a `pack` change, not a ZKite change.

**Recommendation: build the small version, and say in the same commit that it is the
small version.** One custom PNG is the difference between "every ZKite app looks
identical on the home screen" and "my app has my logo", and that is most of the value.
Shipping it while phase_2/PLAN.md §3 keeps promising adaptive layers would repeat the
exact mistake §21.1 found.

### 21.4 `pip install zkite` — what actually has to change in Rust

The question was whether a pip user could do this. They could set `package` and `label`
by hand today; they could not set an icon; and **they could not run `zkite build` at
all.** Five items, ordered by how hard each blocks it. The first is not on any existing
list.

1. **`repo_root()` bakes this machine's checkout path into the binary — the hard
   blocker.**

   ```rust
   fn repo_root() -> PathBuf {
       PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
   }
   ```

   `CARGO_MANIFEST_DIR` is resolved **at compile time**, so the binary carries
   `/home/abhinav/ABHI/CODES/kite-v1` as a literal. Thirteen call sites resolve against
   it — both shells' prebuilt artifact directories, `phase_0/vendor`'s Python prefixes,
   `zkite_py`'s source, `phase_1/webruntime`, the icon — and `pybuild_script()` at
   `build_cmd.rs:596` repeats the same `concat!` independently. On any other machine
   `zkite build` fails at the first missing path.

   **This is not a cross-platform problem.** It breaks a second *Linux x86_64* machine
   just as completely. §17.5's framing ("the CLI is Linux x86_64 only") reads as though
   building for other targets is what is missing; in fact the binary does not work off
   this filesystem at all, on any target. The fix is to resolve payload paths relative
   to the installed binary (or a packaged data directory) with the checkout layout as a
   dev-mode fallback — mechanical, but it touches every staging site.

2. **Nothing packages the payload.** Everything `repo_root` points at is produced by
   `scripts/build.sh` (bash + NDK + cargo) and consumed in place; none of it is in any
   distributable. A wheel has to carry, per backend and per ABI: the shell `.so` set
   (**~21 MB/ABI native** — `libzkitegpuharness.so` 13.7, `libpython3.14.so` 5.8,
   `libc++_shared.so` 1.8; **~6.3 MB/ABI** WebView), `classes.dex`, the vendored
   Android Python prefixes (**84 MB each** for x86_64 and aarch64), `phase_1/webruntime`
   (**22 MB**), `zkite_py`'s source, and the pybuild scripts. That is **order 250 MB
   uncompressed** for the everything-included case, which forces real decisions —
   x86_64 is only there for the emulator, and the two Python prefixes exist to be
   *pruned* into a per-app stdlib. Both are candidates for download-on-demand, which
   is the same lever §17.2 already proposes for the host interpreter.

   There is also an unresolved naming question: the template's
   `dependencies = ["zkite"]` means `zkite` on PyPI is the *Python framework*. Whether
   the CLI ships in that same distribution, or as `zkite-cli`, has never been decided.

3. **The host Python 3.14 requirement (§17.2)** — unchanged and still gating. Not a
   Rust change; it is the `requires-python` decision, which §17.6 correctly parks with
   the owner.

4. **`which()` is Unix-shaped** (`build_cmd.rs:553`): it joins each `PATH` entry with
   the bare name and checks `is_file()`. On Windows the interpreter is `python3.14.exe`
   — and usually reached through the `py` launcher rather than a versioned name at all
   — so this finds nothing and the `uv python find 3.14` fallback becomes the only path
   to a working build. Small, and exactly the kind of thing that makes "Windows is
   unsupported" true without anyone having written it down.

5. **Cross-compiling and releasing the CLI (§17.5)** — and once it is a wheel, that
   means five *wheels* with correct platform tags, each carrying item 2's payload for
   its target, not five loose binaries.

### 21.5 Windows and macOS: the honest answer is narrower than "no"

**No — but the reason is item 1, not the OS.** Nothing in the CLI is Linux-specific by
design: `phase_2/bundler`, `pack`, `rsa`/`rcgen` are all portable, and `zkite build`
never invokes the NDK or cargo (it consumes prebuilt artifacts) — the only subprocess
it spawns is Python. So the OS-shaped gaps are just §21.4 item 4 and the fact that the
binary has never been *built* for those targets, let alone run there.

What actually makes the answer "no" everywhere is `repo_root()`: today `zkite build`
works on this checkout and nowhere else, including on another Linux box. "Cross-platform
support" understates the problem by a lot.

**`zkite new` and `zkite dev` are the portable half and always were.** `new` writes three
files; `dev` shells out to `adb` and pushes `.py` (`dev_cmd.rs` never calls
`repo_root`). Both would work on Windows and macOS the day the binary is built there —
`zkite dev` needs a Dev Shell APK, but it already accepts one via `--devshell-apk` and
otherwise assumes it is installed. So the portable story is real: **edit-and-reload
works anywhere; producing a shippable artifact does not.**

### 21.6 Known gaps in this section

- **The `pip install` half is an audit, not a build.** No wheel was ever produced, so
  "it fails at the first missing path" is read off the code and not observed. The
  ~250 MB payload figure is `du` on an uncompressed tree, before any decision about
  dropping x86_64 or fetching the Python prefixes on demand.
- **Nothing was run on Windows or macOS.** Neither machine was available; the `which()`
  claim comes from reading it, not from a failing run. Item 4 should be confirmed on a
  real Windows box before it is called fixed.
- **The icon work is deferred, so §21.3 is a design, not a measurement.** That `pack`
  cannot express an adaptive icon rests on phase_0 §7's qualifier finding, which was
  not re-tested here.
- **`stdlib_prefix` is hardcoded to `vendor/python-x86_64/prefix`** (`build_cmd.rs:225`)
  regardless of the ABI being built, while the extension-module staging *is*
  per-architecture (`vendor/python-{arch}/...`). The pure-Python stdlib is
  arch-independent so this is probably deliberate, but it is undocumented and reads like
  a bug. Noticed, not investigated.
- **No device run.** Everything here is build-time; the scaffolding change was verified
  on the host with `cargo test` and a real `zkite new` + `zkite build apk`. The APK it
  produced was not installed.
- **The `version_code = 1` in the scaffold is still §17.4's problem**, now written into
  every new project instead of defaulted silently. That is a small improvement in
  visibility and no improvement at all in behaviour: the second release still needs a
  manual bump, and nothing warns.

## 22. A custom app icon — the small version, and the wall behind it

§21.3 deferred this with its shape written down and a recommendation: *"build the small
version, and say in the same commit that it is the small version."* That is what this
is. §21.1's finding is what makes it worth a section rather than a line: the key was
**documented in phase_2/PLAN.md §3 from phase 2 and read by no code for three phases**,
so every app ever built with `zkite build` shipped the same square, and the doc was the
only evidence the feature existed.

### 22.1 What shipped

`build_cmd.rs::load_icon`, ~60 lines including the error paths, plus one field on
`ProjectConfig`. `AppIdentity` is untouched — the manifest already says
`android:icon="@drawable/icon"` and an icon is a *resource*, not a manifest field, which
is why this is small.

| behaviour | why it is there |
|---|---|
| reads `[tool.zkite.android] icon` | the key phase_2/PLAN.md §3 promised |
| resolves against the **project dir**, not the cwd | the path belongs to the project; an absolute path still passes through, since `Path::join` replaces on absolute |
| **validates it is really a PNG** (signature + `IHDR`) | see below — this is the one that is not optional |
| falls back to the bundled default when absent | unchanged behaviour for every existing project |
| prints `icon: <path> (WxH)` | so the build says which icon it used |
| warns under 192x192, and on non-square | a launcher upscales below xxxhdpi; a non-square icon gets cropped |
| errors with **both** the declared string and the resolved path | the two say different things, and the surprising one is usually the second |

**The PNG check is load-bearing, not belt-and-braces.** `pack`'s asset compiler stores
resource bytes verbatim — `FileResource` carries `contents` and nothing decodes them
(its only remark on the subject is a `TODO` about PNG-crushing) — and `pack_zip::File`
is `{ path, data }`. So a `.jpg` renamed `.png` packs, signs, aligns, installs and
*then* fails to decode on the device, with the failure in system logs and a blank icon
on screen. Verified by feeding one in:

```
Error: [tool.zkite.android] icon = "brand.png" is not a PNG (/…/brand.png — looks like a JPEG).
  Android resource packing stores the bytes verbatim — nothing transcodes — so a
  mislabelled file builds and signs cleanly and then fails to decode on the device.
```

**One real defect found by running it rather than reading it.** The first version read
the icon at the resource-assembly site where it is consumed, which is *after* wheel
resolution, pyc compilation, stdlib pruning and key generation. A typo'd path therefore
took a measured **9.21 s** to report. Moved next to `validate_package`, where the other
cheap check already is: **0.00 s**. Same "fail loudly at build" posture §21.2 and §17.4
argue for, with the addition that failing loudly is worth little if it takes ten seconds
to do it.

`zkite new` now scaffolds the key **commented out**. It is the one identity key that
names a file, so an uncommented default would fail every first build — the exact shape
of the bug §21.2 avoided when it sanitized the package segment. Commented, it is
discoverable, which was §21.2's whole argument.

**Verified live on the Moto G54** (arm64-v8a, Android 15), not inferred from a passing
build: a 512x512 PNG lands **byte-identical** in `res/drawable/icon.png` inside the
signed APK (sha256 compared against the source, and against the bundled default to
confirm it is not that), the app installs, and **the launcher draws it** — confirmed in
an app-drawer screenshot, alongside the two ZKite Dev Shells still showing the bundled
square. The app itself still launches and renders.

### 22.2 What the device showed that the design did not predict

Two things, and the second is the more useful.

**The bundled default is a 48x48 flat `indigo-500` square** — decoded from the file
itself (`IHDR 48x48`, colour type 2, one distinct RGB triple). The prediction going in
was that it would look blurry on a 400dpi phone. **It does not**, and the reason is
worth writing down rather than quietly dropping: a single flat colour upscales exactly,
so the one case where a 48x48 icon is *not* visibly soft is the case ZKite happens to
ship. That is luck, not design — any real logo at 48x48 would be soft — which is why the
new `MIN_ICON_PX` warning exists for user icons and deliberately does not fire for the
default.

**A legacy icon gets matted onto a white plate, and this is the adaptive-icon gap made
visible.** On the launcher, the custom PNG is *inset inside a white rounded square*
rather than filling the icon mask the way a system app's does. That is modern Android
normalising a legacy `drawable` icon it cannot treat as adaptive. §21.3 argued the
Play-quality version "is much bigger" from the shape of the API; this is that argument
with a device observation under it.

It also corrects phase_2/PLAN.md §3 a second time. That section says the icon "reuses
the Dev Shell's single density-independent **adaptive** icon approach". It is not
adaptive — it is a plain `drawable`, on every ZKite shell, and always has been. phase_0
§7's own "sidestep" mitigation is accurately described as *"emit a single
density-independent adaptive icon"*, which was the intent; what got built was the
density-independent half only, and nothing since has said so.

**The adaptive half stays deferred, and it is a `pack` change before it is a ZKite
change.** A real adaptive icon is `<adaptive-icon>` XML in `mipmap-anydpi-v26/` with
foreground/background/monochrome layers plus per-density rasters. `pack` has **no
resource-qualifier support at all** (phase_0/README.md §7), and `zip_extras::read_res_dir`
asserts on any hyphenated resource directory precisely so this fails loudly rather than
silently dropping half the resource set. So `mipmap-anydpi-v26/` is not expressible
today, at any effort, without qualifier support landing upstream first. That is its own
session and it is phase_0 §7's mitigation tier 2 ("contribute upstream"), not this one.

### 22.3 Known gaps

- **One device, one launcher.** The white-plate behaviour is Motorola's launcher on
  Android 15. Pixel Launcher and Samsung One UI normalise legacy icons too, but by
  their own rules, and neither was tested.
- **The `>= 192px` threshold is a convention, not a measurement.** It is Android's
  xxxhdpi launcher bucket (48dp at 4x). Nothing here measured the point at which
  upscaling becomes visible, and it will differ per launcher.
- **PNG validation is signature + IHDR, not a decode.** A file with a valid header and
  a corrupt `IDAT` passes this check and fails on the device exactly as a JPEG would.
  Catching that needs a real decoder, i.e. a dependency, for a case nobody has hit.
- **Colour type, bit depth and interlacing are not checked.** `IHDR` is parsed for
  width and height only; a 16-bit or interlaced PNG is passed through untouched. Android
  reads all of those, so this is believed fine and was not tested.
- **The bundled default was left alone.** It is still 48x48. Replacing it is a one-file
  change with no `pack` involvement, and it was not done — it changes what every
  existing project's icon looks like, which is a decision rather than a fix.
- **AAB was not re-checked.** The icon goes through `Package.resources`, which
  `aab.rs` and `apk.rs` share, so this should be format-independent; `zkite build aab`
  with a custom icon was not run.

## 23. `pip install zkite` on Windows and macOS — what it would actually take

§21.4/§21.5 audited this and stopped at "no, and the reason is `repo_root()`". The
owner asked for the rest: a concrete ordered plan for *a stranger on Windows or macOS
goes from `pip install` to an app on their phone*, how the payload gets distributed,
whether it can shrink, what breaks on Windows specifically, and a recommendation with a
decision attached.

**Two of §21.4's own numbers do not survive being measured.** The payload is not
~250 MB, and `repo_root()` is not two compile-time anchors but four. Both changes point
the same way: this is smaller than it has been written up as.

### 23.1 The answer, first

**`zkite new` and `zkite dev` work on Windows and macOS the day the binary is built
there.** `new` writes three files; `dev` shells out to `adb` and pushes `.py`. Neither
resolves a payload path — §24 added the first one to `dev`, deliberately behind an
environment variable so it stays a fallback.

**`zkite build` works on this checkout and nowhere else, on any OS.** A second *Linux*
machine fails identically to a Mac. Calling this a cross-platform gap (§17.5) understates
it by a lot, which §21.5 already said and this section confirms with the site count.

What stands between here and `pip install zkite` is **a path resolver and a packaging
decision**. The resolver is an afternoon. The packaging decision needs the owner. The
release *pipeline* that produces the payload is the part that is actually weeks, and it
is the part no existing list names.

### 23.2 `repo_root()` — four anchors, not two, and the fourth is the worst one

> **Resolved (§29).** All of them — five by the time it was done, since §24 added one on
> purpose — now go through a single `payload::root()`. The section below is kept as
> written because it is the accurate statement of the problem and is what the fix was
> built from. One thing in it turned out to understate itself: the fourth anchor
> (`zkite doctor`) was indeed the worst, and not only for the reason given. Once `doctor`
> could resolve a *real* payload, it also turned out to fail outright on §23.4's
> recommended default wheel for an unrelated, pre-existing reason (§29.2).

§21.4 item 1 says thirteen call sites resolve against `repo_root()` and that
`pybuild_script()` "repeats the same `concat!` independently". Grepping for the macro
rather than the function finds **four** independent compile-time anchors:

| site | what it resolves | consequence off this checkout |
|---|---|---|
| `build_cmd.rs:1005` `repo_root()` | both shells' artifacts, the Python prefixes, `zkite_py`, `webruntime`, the icon | `zkite build` fails at the first missing path |
| `build_cmd.rs:636` `pybuild_script()` | `compile_pyc.py`, `prune_stdlib.py` | same |
| `wheels.rs:76` `vendored_pip_pyz()` | the vendored `pip.pyz` | silently falls back to `python -m pip`, which may not exist |
| `doctor_cmd.rs:131` `check_shell_artifacts()` | the artifact check | **`zkite doctor` reports the artifacts missing and prints a path that does not exist on that machine** |

The last one was not on any list and is the one that hurts most out of proportion to its
size: `doctor` is the command a stuck user runs, and on a fresh machine it currently
answers a real question with a wrong path. A diagnostic that is itself broken is worse
than no diagnostic — this is the same shape as §21.1's doc-only feature, one layer down.

`wheels.rs:76` is the quiet one: it degrades rather than fails, so on a second machine
wheel resolution changes behaviour without saying so.

**The fix is one function, not nineteen edits.** Inside `build_cmd::run` everything goes
through `repo_root`, `phase0`, or `shell.artifacts()`, so the ~19 uses are one value
being passed down. What is needed is a `payload_root()` with an explicit search order —
environment override, then a directory beside the installed binary, then the compile-time
checkout as a dev fallback — and the four anchors collapsing into it. The fight is not
technical: it is that **the checkout entry must stay last and stay a fallback**, and
every future site must go through the resolver. This session added a fifth anchor on
purpose (§24's `find_devshell_apk`) and routed it that way — env var first, checkout
last, with a comment naming the trap — as the pattern to copy rather than an exception
to it.

`zkite doctor` should then print *which* root it resolved, because the failure this
introduces is a wheel finding a stale checkout instead of its own payload.

### 23.3 The payload is ~19 MB compressed, not ~250 MB

§21.4 item 2's "order 250 MB uncompressed" is `du` over the checkout. It counts a lot
that never reaches an APK. Measured, per item:

| what | size | ships? |
|---|---|---|
| Python prefix `test/` | 37 MB x2 | **no** — `prune_stdlib.py` drops it on every build |
| Python prefix `include/` | 5.5 MB x2 | **no** — C headers; `zkite build` compiles nothing |
| `lib{crypto,ssl,sqlite3}.so` | 6.1 MB x2 | **no** — `cmp`-verified byte-identical duplicates of the `_python`-suffixed copies, which are the ones staged |
| prefix `libpython3.14.so` | 5.6 MB x2 | **no** — `cmp`-verified byte-identical to the shell's own copy, already staged from `artifacts/` |
| `webruntime/test/node_modules` | 19 MB of 22 MB | **no** — `stage_webruntime` deletes `test/`; webruntime really ships **3.2 MB** |
| `gpu_harness/artifacts/*.apk` | 92 MB | **no** — prebuilt Dev Shells, not build inputs (§24 is about *those*) |
| pure-Python stdlib | 16 MB | **yes, once** — arch-independent |
| `lib-dynload` + `lib*_python.so` | 13.5 MB | yes, **per ABI** |
| native shell `.so` set | 21.3 MB | yes, per ABI (`libzkitegpuharness` 13.7 + `libpython3.14` 5.8 + `libc++_shared` 1.8) |
| WebView shell `.so` set | 6.3 MB | yes, per ABI (`libzkiteappshell` is **499 KB**; the rest is `libpython`) |
| `classes.dex` + `fonts.zip` | 2.5 MB | yes |
| `zkite_py` + pybuild + `pip.pyz` | 2.1 MB | yes |

Staging exactly that set for the default configuration — native backend, `arm64-v8a`
only — and zipping it the way a wheel is zipped:

```
uncompressed: 53 MB
zip -9:       19 MB          <- a wheel is a zip
```

PyPI's default limits are **100 MB per file and 10 GB per project**. A 19 MB wheel is
unremarkable.

**The split that makes this easy, and which no prior write-up names: the payload is
host-independent.** Every byte of it is Android-target data — `.so`s for `arm64-v8a`,
a Python stdlib, a dex, fonts. It is the same bytes on Linux, macOS and Windows. The
*only* host-specific artefact is the `zkite` binary itself, and that is **7.2 MB
release, 5.7 MB stripped**, linking nothing but `libc`/`libm`/`libgcc_s`.

So the natural shape is not five 250 MB wheels. It is **one `py3-none-any` payload wheel
of ~19 MB plus five platform wheels of ~6 MB each** — which is the ruff/uv/maturin
model for the binary half (maturin's `bin` bindings do exactly this, one wheel per
platform tag) and an ordinary data wheel for the other. §21.4 item 2 said the size
"forces real decisions". Mostly it does not.

### 23.4 What ABI and prefix pruning buys, and what it costs

- **Dropping `x86_64`** saves 35 MB uncompressed / ~13 MB compressed. It costs the
  emulator, which is the only reason it exists — and phase_0 already established that an
  x86_64 host **cannot** run an arm64 AVD, so x86_64 is the only ABI an emulator user on
  an Intel machine can use. Ship it as a separate wheel or an opt-in download rather than
  dropping it.
- **Shipping one Python stdlib instead of two** is already how the code works and is
  worth ~16 MB. `build_cmd.rs:256` hardcodes `stdlib_prefix` to `vendor/python-x86_64/prefix`
  regardless of the ABI being built. §21.6 flagged this as "undocumented and reads like a
  bug". **It is correct**: the pure-Python stdlib is arch-independent, only `lib-dynload`
  and `lib*_python.so` are not, and those *are* staged per-architecture. It should be
  documented rather than fixed — and it is precisely what allows one shared copy in a
  wheel.
- **Shipping only one backend** saves 6.3 MB/ABI plus 3.2 MB of `webruntime`. Not worth
  the loss: `--backend webview` is the documented escape hatch for every gap in §15.1,
  and `zkite build` prints it on every native build.

Net: a native-only, `arm64-v8a`-only default wheel at ~19 MB, with x86_64 and the
WebView backend as separate wheels or extras, covers the common case without any
download-on-demand machinery.

### 23.5 What actually breaks on Windows

§21.4 item 4 found one item by reading. Working through the rest of that class the same
way — **all of this is read, none of it is run; no Windows machine was available**:

1. **`which("python3.14")` is Unix-shaped, and adding `.exe` would not fix it.**
   `build_cmd.rs::which` joins each `PATH` entry with the bare name and checks
   `is_file()`, so `python3.14.exe` is never found. The deeper problem is that
   python.org's Windows installer **does not create a versioned executable at all** — it
   installs `python.exe` in a versioned directory and registers the `py` launcher. So
   the fix is not `.exe`; it is `py -3.14 -c "import sys; print(sys.executable)"`, plus
   `.exe` for completeness. The existing `uv python find 3.14` fallback does work on
   Windows, so this is degraded rather than dead — Windows currently requires `uv`
   without anyone having written that down.
2. **Zip entry separators are already handled — all four sites.** `read_staging_dir`,
   `zip_dir`, `zip_zkite_framework` and `copy_stdlib_unpruned` each do
   `.to_string_lossy().replace('\\', "/")`. Someone already thought about this. Worth
   recording as a *negative* result so nobody re-audits it.
3. **Executable bits are a non-issue by construction.** `pack_zip::File` is
   `{ path: String, data: Vec<u8> }` and the writer sets no external attributes at all,
   so APK entries carry no Unix mode on *any* host and Android's installer assigns
   permissions at extract time. This cannot break on Windows because it does not work by
   file mode anywhere.
4. **`adb`/`uv` invocation reads safe.** `Command::new("adb")` resolves `adb.exe`
   through Windows' own program search, and `check_adb` parses with `str::lines()`
   (which strips a trailing `\r`) and `contains("\tdevice")`. Nothing in the CLI parses
   `adb shell` output, which is where Windows' `\r\r\n` mangling actually bites.
   Unverified.
5. **Symlinks: 6 in the vendored prefixes, 0 in what ships.** `libsqlite3.so.0 ->
   libsqlite3_python.so` and two `pkgconfig/*.pc`. Windows needs Developer Mode or
   elevation to create a symlink, so a wheel carrying the **raw** prefix would fail to
   unpack for ordinary users. A wheel carrying the *pruned* set — §23.3's table — has
   none, so the problem disappears as a side effect of shipping less.
6. **Long paths: safe pruned, marginal raw.** The longest path that would actually ship
   is 78 chars (`lib-dynload/_testimportmultiple.cpython-314-aarch64-linux-android.so`);
   under a typical Windows venv prefix that lands near 180, inside `MAX_PATH` 260. The
   longest path in the raw checkout is 142 (a `test/test_tomllib` fixture), which lands
   near 250 and would break in a deep project directory. Same conclusion as 5: shipping
   the pruned set is what makes this safe.
7. **Case-collisions: none** found in the aarch64 prefix, so a case-insensitive
   filesystem is fine. Only that prefix was checked.
8. **Line endings are a non-issue.** `zkite new`'s templates are `include_str!`'d at
   compile time, so they carry the *build* machine's endings — a Linux CI wheel gives LF
   everywhere, on every host. `zkite dev` pushes file bytes verbatim and Python accepts
   CRLF.

**The honest summary is that the Windows-specific surface is one function and a `py`
launcher story.** Items 2, 3 and 8 are already handled or structurally absent; 5 and 6
resolve themselves once the payload is pruned. That is materially smaller than §21.4
item 4 implied by naming only the one it found.

### 23.6 What comparable projects actually do

Read this session rather than recalled. Every one of them is solving a bigger version of
this problem, and they converge on the same answer — which turns out to be the wrong one
for ZKite, for a reason worth stating.

- **Briefcase / BeeWare.** If it cannot find an Android SDK or JDK, it *"will download
  and install an isolated copy in its data directory"*, cached under
  `~/.cache/briefcase/tools/` (Linux), `~/Library/Caches/org.beeware.briefcase/tools/`
  (macOS), `%LOCALAPPDATA%\BeeWare\briefcase\Cache\tools\` (Windows). Separately, and
  directly relevant to phase_2/PLAN.md §1.3: Briefcase does **not** resolve Android
  wheels from PyPI alone — it uses *"a secondary repository to provide pre-compiled
  Android wheels… maintained by the BeeWare project"*, explicitly not covering every
  package or version. The Android-wheel gap phase 2 found is one others solved by
  running an index, not by waiting.
- **Buildozer / python-for-android.** Same shape, blunter: downloads the SDK, NDK and
  p4a automatically on first build into `~/.buildozer`.
- **Flutter** — the closest analogue, a tool that must ship large per-platform engine
  binaries. `flutter precache` populates `bin/cache`; the download base is
  `https://storage.googleapis.com`, overridable with `FLUTTER_STORAGE_BASE_URL`, and
  **each artifact carries a stamp file compared against a version**, so it re-downloads
  only when the version changes. That is the design to copy *if* download-on-demand is
  ever needed: a versioned artifact, a stamp, an overridable base URL (mirrors, proxies,
  air-gapped networks) and an explicit warm-the-cache command.
- **Chaquopy.** Ships no Python in the plugin at all — the Gradle plugin pulls CPython
  from **Maven Central** at build time, and native third-party packages from Chaquopy's
  own repository.
- **maturin / ruff / uv.** The model for the binary half: one wheel per platform tag
  carrying a compiled binary, which maturin's `bin` bindings produce directly. Worth
  noting alongside phase_2/PLAN.md §1.3: maturin 1.12.4 added **Android platform-tag
  detection**, so the wheel ecosystem is moving on exactly the axis phase 2 found empty.

**The synthesis, and why ZKite should not simply copy it.** Nobody ships the payload in
the distribution; everyone downloads and caches per-user. But everyone in that list also
*requires a toolchain ZKite exists to avoid* — Flutter's engine, a JDK, an Android SDK,
Gradle — and their downloads are hundreds of megabytes to gigabytes. ZKite's entire
payload is 19 MB compressed, which is **smaller than the download those tools perform**.
Shipping it is the only option that keeps plan1.md §2 principle 1 (*"the user's machine
installs nothing but a wheel"*) literally rather than approximately true, and principle
1 is the sentence the whole project follows from.

### 23.7 The ordered plan, with the fight in each one named

> **Step 1 is done (§29); steps 2–6 are unchanged and still open.** Step 1's "fight"
> was correctly identified and is worth reading against what happened: reporting the
> resolved root was not merely good hygiene, it was what exposed a bug in `zkite doctor`
> within minutes of it being possible to see the answer (§29.2).

1. ~~**`payload_root()`, and collapse the four anchors into it.**~~ **Done — §29.**
   Search order: env
   override, beside the installed binary, compile-time checkout last. *The fight:* not
   the code — it is that the checkout entry must stay a fallback forever, and that
   `zkite doctor` must report which root it used, or the first person to debug a wheel
   gets a path that does not exist and no clue why.
2. **Decide the distribution unit.** `dependencies = ["zkite"]` in the `zkite new`
   template means `zkite` on PyPI is the *Python framework*. Whether the CLI ships in that
   same distribution or as `zkite-cli` has never been decided (§21.4 item 2). *The fight:*
   this is a naming decision with a compatibility tail, and it blocks step 3. **Owner,
   not engineer.**
3. **A release pipeline that builds the payload.** Both ABIs x both backends x `.so`s,
   `classes.dex`, `fonts.zip`, plus the pruned Python prefixes, assembled into one
   `py3-none-any` wheel. *The fight, and this is the real one:* **no such pipeline
   exists.** `scripts/build.sh` and `build_dex.sh` are run by hand today; §14.3 closed
   two release-pipeline gaps and §15.6.1 found a third (a stale `.so` shipped because a
   guard checked existence, not freshness). This step is weeks, and it is weeks of CI,
   not of Rust. plan1.md §14.5's "no CI for any phase-3 crate" is the same item seen from
   a different angle.
4. **Cross-compile the CLI to five platform wheels** (linux-x86_64, linux-aarch64,
   macos-aarch64, macos-x86_64, windows-x86_64). *The fight:* mechanical to build, hard
   to *test* — `phase_2/bundler`'s suite shells out to real `apksigner`, `zipalign`,
   `bundletool` and `keytool`, so non-Linux runners can compile but cannot hold the same
   verification bar without those tools installed.
5. **Fix `which()` and add the `py -3.14` path** (§23.5 item 1). *The fight:* none. It
   needs a Windows box to call done, and nobody has one.
6. **The host-Python decision (§17.2), unchanged and still gating.** `requires-python =
   ">=3.14"` remains the recommendation and remains the owner's call.

**What I would not do: download-on-demand for the payload.** At 19 MB it does not pay
for a cache directory, stamp files, a base-URL override, an offline story, and a
first-five-minutes failure mode that depends on a CDN. Revisit if adding x86_64, the
WebView backend and further ABIs takes a single wheel past roughly 60 MB — at which point
§23.6's Flutter design is the one to copy, and the extras/separate-wheel split in §23.4
is the cheaper intermediate step to try first.

### 23.8 Recommendation, and what needs the owner

In §17.6's form — what I would do, in order:

1. ~~**`payload_root()`.**~~ **Done — §29.** It is the only purely-blocking item, it is
   an afternoon, and
   everything after it becomes testable once it exists. (It was an afternoon, and the
   "everything after becomes testable" claim held immediately: the first thing the
   resolver made possible to run also found a bug in `zkite doctor`, §29.2.)
2. **Document `stdlib_prefix`'s hardcoded x86_64 path** (§23.4) rather than "fixing" it.
   Five minutes, and it removes a landmine: it currently reads like a bug, and the
   obvious "fix" would double the payload.
3. **The release pipeline (step 3 above).** This is where the actual time is. Nothing
   downstream is real without it, and it is also what closes plan1.md §14.5's CI gap.
4. **`which()` / `py` launcher.** Free, blocked only on a machine to confirm it.
5. **CLI cross-compilation.** Last, unchanged from §17.5, and still only useful after
   the host-Python decision.

**What needs you rather than an engineer**, stated so it does not get buried:

- **`zkite` vs `zkite-cli`** — the distribution-unit decision. Blocks packaging.
- **`requires-python = ">=3.14"`** — §17.2's open question, still open. It couples every
  ZKite developer's Python to the Android Python, permanently.
- **Whether `x86_64` ships by default** — it is 13 MB compressed and it is the only ABI
  an emulator user on an Intel machine can run.

### 23.9 Known gaps in this section

> **Three of these moved in §29.** The `repo_root()` claim is no longer "read, not
> observed"; the staged set is no longer only derived by reading; the payload size is
> now measured from a script that produces a working build. Each is annotated below
> rather than deleted, since what changed is the *evidence*, not the shape.

- **Nothing was run on Windows or macOS.** Every claim in §23.5 is read from the source,
  not observed — the same limit §21.6 recorded, unchanged, and it is the single largest
  caveat on this section. **Still true after §29.**
- ~~**Nor on a second Linux machine.**~~ **Narrowed, not closed (§29.4).** No second
  machine was used, but the thing a second machine was meant to test — that `zkite build`
  works from an installed binary which never reads this checkout — was tested directly,
  with a sentinel file proving the payload rather than the checkout was read. A genuinely
  separate machine would additionally cover "nothing else on this box is load-bearing",
  which remains unconfirmed.
- **No wheel was built.** The 19 MB is `zip -9` over a hand-staged tree assembled to
  match what `build_cmd.rs` reads, not the output of `python -m build`. Real wheel
  metadata and `RECORD` add a little; nothing here modelled that. **Still true after
  §29**, which staged with a script rather than by hand and measured 21 MB — slightly
  *higher*, because it ships the unpruned pure-Python stdlib, which is correct.
- **The staged set was derived by reading `build_cmd.rs`.** If a staging site was missed,
  the figure is low rather than high. **§29 turns this from a reading into a script**
  (`phase_1/cli/scripts/stage_payload.sh`) whose output demonstrably builds a working,
  installable, launching APK — so a missed site would now fail loudly. Nothing asserts
  the script stays in sync with `build_cmd.rs`, which is the residual gap (§29.7).
- **Only native / `arm64-v8a` was measured end to end.** The WebView and x86_64 numbers
  are arithmetic over separately-measured parts.
- **The prior art is docs and search results, not source, with one exception.** Flutter's
  `cache.dart` was read directly; Briefcase's `gradle.py`, Buildozer's and Chaquopy's
  internals were not. Briefcase's exact cache paths come from its documentation.
- **Step 3's cost is unestimated.** "Weeks of CI" is a judgement from the shape of the
  work, not a plan with tasks in it.
- **`pip.pyz` shipping (phase_2/PLAN.md §9) is still not done** and is assumed into the
  2.1 MB line here rather than solved.

## 24. `zkite dev`'s first run — from a silent hang to a running app

§15.3 and phase_1/PLAN.md §9 both record *"`zkite dev` does not auto-install a Dev
Shell"* as one line of a gap list. Running it as a stranger would shows it is worse than
that line reads, and the fix is smaller.

### 24.1 What was actually wrong — two silent hangs, not one missing feature

With no `--devshell-apk`, `zkite dev` printed

```
no --devshell-apk given — assuming the native Dev Shell is already installed and
running (build one with: ZKITE_DEVSHELL=1 ABI=arm64-v8a phase_3/gpu_harness/scripts/build_apk.sh)
```

and went straight to `listening on 127.0.0.1:8765 — waiting for the Dev Shell to
(re)connect…`. Two separate dead ends came out of that one sentence:

1. **Not installed** — waits forever. Nothing ever connects, nothing ever says why.
2. **Installed but not in the foreground** — *also* waits forever, because
   `install_and_launch` was the only code path that ever ran `am start`, and it only ran
   behind the flag. So a user who installed the shell yesterday runs `zkite dev` today,
   the phone sits on the home screen, and the terminal says it is waiting.

The second is the one that makes this a bug rather than a missing feature: the message
asserted "already installed **and running**" as an assumption, and the second half of
that assumption was never checked and never arranged. It is the same shape §21.1 found
in the docs — a claim whose only evidence is the text making it — here printed at the
user instead.

There was a third, quieter one. `zkite dev` never reads `[tool.zkite.android]`, so the
phone shows *ZKite's* name and icon while the user's `pyproject.toml` says otherwise, and
nothing anywhere said so. That is structural — the Dev Shell is generic by design
(plan1.md §6) — but "structural" is not the same as "obvious", and it was reported as
confusing by the owner during this session, from the product rather than from the docs.

### 24.2 What shipped

`zkite dev` now asks the device instead of assuming, using two probes that cost one `adb`
round trip each:

- `adb shell getprop ro.product.cpu.abi` — which Dev Shell to look for.
- `adb shell pm path <package>` — exits 0 and prints `package:/data/app/…` when
  installed, exits 1 with no output when not. Verified both ways on the real device
  before being relied on.

Then four cases instead of two:

| situation | before | now |
|---|---|---|
| `--devshell-apk` given | uninstall, install, launch | unchanged — someone passing a path means it |
| installed | print an assumption, wait | **launch it**, then serve |
| not installed, a prebuilt APK is findable | print an assumption, wait forever | **install it, launch it**, then serve |
| not installed, nothing findable | print an assumption, wait forever | **exit 1** with the device's ABI, the exact build command with the ABI substituted in, and the flag |

APK lookup order is `ZKITE_DEVSHELL_APK`, then the release-pipeline artifact path in this
checkout. That second entry is `repo_root()` again and is dev-mode-only for exactly the
reason `zkite build` is (§23.2) — which is why it sits *behind* the environment variable
rather than being the mechanism, and why the failure message tells you to build one
rather than implying a download exists. When §23's payload packaging lands, this becomes
one more entry in that search order instead of something to unpick.

> **That last sentence came true (§29.1).** `find_devshell_apk`'s checkout branch now
> resolves through `payload::root()`, so a payload carrying a Dev Shell APK is found
> with no code change (`stage_payload.sh` has an `INCLUDE_DEVSHELL=1` path for it).
> Routing this anchor correctly when it was added is why there were five to collapse
> rather than four to collapse and one to argue about — the note above was worth
> writing at the time. **Not exercised:** only `zkite build` was run from a staged
> payload, never `zkite dev` (§29.7).

And the flag's own help no longer advertises the dead end. It said the APK *"doesn't
ship inside this wheel yet (that's phase 2's job)"* — phase 2 came and went and it is
§23's job now; more to the point, the flag is no longer how you get a Dev Shell.

Finally, two lines after launch, answering the identity question in the product:

```
  note: the Dev Shell is generic — the app name and icon on your phone are ZKite's, not your [tool.zkite.android] ones.
        Those apply to `zkite build` output. Your code and styling are fully live here.
```

### 24.3 Should `zkite dev` build, download, or ship the Dev Shell?

The owner asked which. **Building it is not an option and can be ruled out now rather
than researched**: `build_apk.sh` needs `cargo`, the NDK, `javac` and `d8`, which is
precisely what plan1.md §2 principle 1 forbids on a user's machine. It is a
release-pipeline artifact in the same bucket as `classes.dex`. Building remains what
*this checkout* does, and the new code path finds that output — which is a convenience
for ZKite's own developers, not a distribution answer.

That leaves ship-in-wheel versus download-on-demand, and it is §23's question wearing a
different hat, with one number that changes the answer: **the native Dev Shell APK is
39 MB.** That is twice §23.3's entire build payload, because it is a full standalone app
— shell `.so`s, `libpython`, the stdlib, fonts, dex — packed and signed.

§23.6's prior art is directly on point here and points the other way than it does for the
build payload: Flutter downloads engine artifacts into `bin/cache` with a per-artifact
stamp compared against a version, Briefcase downloads an isolated SDK/JDK into a per-user
cache directory, Buildozer does the same into `~/.buildozer`. Nobody ships an artifact
this size in the distribution.

**Recommendation: download-on-demand for the Dev Shell, ship-in-wheel for the build
payload.** They differ on every axis that matters:

- the build payload is 19 MB and *every* `zkite build` needs all of it; the Dev Shell is
  39 MB and is needed **once per device, ever**, then cached by the device itself;
- a Dev Shell is per-ABI with no useful subset, so it cannot be trimmed the way §23.4
  trims the build payload;
- it is versioned independently and changes rarely — exactly the shape Flutter's
  stamp-file design exists for;
- and the failure mode is benign: no network on first run means "install this APK", not
  a broken build.

Concretely that is a GitHub release asset per `(backend, abi, version)`, a
`~/.cache/zkite/devshell/` (and the platform equivalents Briefcase enumerates), a stamp
file, and a `ZKITE_DEVSHELL_BASE_URL` override for mirrors and air-gapped networks. The
search order this session shipped already has the slot for it: env var, cache, checkout.

**Not built this session, deliberately.** It needs a release pipeline to publish the
asset against (§23.7 step 3), and building a downloader with nothing to download would
be the same mistake §21.1 records — a feature whose only evidence is the code claiming
it.

### 24.4 Verification, live on the Moto G54

All three no-flag cases run on the real device, native backend:

**A — not installed, no APK reachable** (artifact moved aside for the run, restored
after). Exits immediately, `EXIT=1`:

```
Error: no native Dev Shell installed on this device (dev.zkite.gpuharness), and no prebuilt APK to install.
  The Dev Shell is a build artifact that nothing packages yet (phase_3/PLAN.md §23/§24), …
    1. build it:  ZKITE_DEVSHELL=1 ABI=arm64-v8a phase_3/gpu_harness/scripts/build_apk.sh
    2. point at one:  zkite dev --devshell-apk <path>  (or ZKITE_DEVSHELL_APK=<path>)
  Device ABI is arm64-v8a.
```

**B — not installed, prebuilt APK findable.** `zkite new devtest && cd devtest && zkite
dev`, on a phone with the Dev Shell **uninstalled** first:

```
no native Dev Shell on the device — installing one
installing …/zkite-native-devshell-arm64-v8a.apk
Success
launching dev.zkite.gpuharness/.ZKiteActivity
device connected — pushing the full project
  [device] native Dev Shell connected
```

Screenshot confirms the counter app on screen. **That is plan1.md §0's opening sequence
minus the `pip install` — two commands from nothing to a running app**, which had never
been true before this session.

**C — installed but force-stopped**, the case that used to hang with the shell sitting
on the device:

```
native Dev Shell already installed — launching it
launching dev.zkite.gpuharness/.ZKiteActivity
device connected — pushing the full project
  [device] native Dev Shell connected
  pushed 1 change(s) in 0ms: ~main.py
```

No reinstall, and a live edit made during that run (`bg-slate-950` → `bg-emerald-950`,
`"Increment"` → `"Tap me"`) landed on screen — so the hot-reload path is intact through
the new launch flow, not just the launch itself.

### 24.5 Known gaps

- **The download-on-demand path is designed, not built** (§24.3). Nothing fetches
  anything; the failure message deliberately says "build one" rather than implying
  otherwise.
- **Only the native backend was exercised on device.** The WebView branch is the same
  code with different constants and its artifact path was confirmed to exist
  (`phase_1/devshell/artifacts/zkite-devshell-arm64-v8a.apk`), but `zkite dev --backend
  webview` was not re-run through the new flow.
- **No tests.** `dev_cmd.rs` had none before and has none now; `is_installed` and
  `device_abi` both shell out to `adb`, and `find_devshell_apk`'s checkout branch resolves
  a compile-time path — testable only by making both injectable, which is a refactor this
  did not attempt. Verified on hardware instead, which is this repo's standing bar but is
  not a regression guard.
- **Multi-device is unhandled, and was already.** `check_adb` counts devices but nothing
  passes `-s <serial>`, so with two devices attached every `adb` call here hits whichever
  one adb picks. Pre-existing; now more visible because more `adb` calls are made.
- **`am start` on an already-foregrounded shell** is a no-op that still costs a round
  trip and prints a `launching …` line. Harmless, mildly noisy on repeat runs.
- **The identity note prints every run.** It is two lines and it is the answer to a
  real question, but it is not suppressed after first use and there is no way to silence
  it.
- **The ABI fallback is `arm64-v8a`.** If `getprop` fails, `zkite dev` guesses rather
  than failing. That guess is right for essentially every modern phone and wrong for an
  x86_64 emulator, where it would then report no APK found for the wrong ABI.

## 25. Two bugs a real app found in an hour

The owner wrote a ~1000-line app (a metro departures board: 9 screens over `Nav`, a
600-row virtualised `List`, `Switch`/`Slider`/`TextField`, ~640 nodes on the root
screen) and hit both of these immediately. Neither is reachable from the counter app
every prior section verified against, which is the point worth recording: **the demo
apps in this repo are all one screen deep and none of them uses
`from __future__ import annotations`.**

### 25.1 `@store` + `from __future__ import annotations` could not boot at all

**Symptom:** every launch, an on-device error overlay ending in

```
File ".../python3.14/dataclasses.py", ...
    ns = sys.modules.get(cls.__module__).__dict__
AttributeError: 'NoneType' object has no attribute '__dict__'
```

with nothing in the traceback naming ZKite below the `@store` line.

**Cause, and it is one missing line.** `reload.py::load_module` did

```python
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)          # module is NOT in sys.modules yet
```

`reload_module` assigned `sys.modules[module_name]` afterwards, which is too late to
help anything executing *inside* the module. `importlib._bootstrap._load` — and
CPython's own documented "import a source file directly" recipe — register **before**
executing, precisely so `sys.modules[__name__]` exists while module-level code runs.

Under `from __future__ import annotations` (PEP 563) every field annotation is a
*string*, so `dataclasses` has to resolve names textually to spot `KW_ONLY`, and
`_is_type` does the lookup above with no guard. Any `main.py` with that import and a
single `@store` therefore died before rendering a frame. Without the future-import,
annotations are real objects, that branch is never taken, and nothing is wrong — which
is exactly why phases 1–3 never saw it.

**Fix:** register before `exec_module`, and on failure restore the *previous* module
rather than popping, so a failed hot reload does not also unregister the module that
was working a second ago (the courtesy `importlib.reload` extends).

Reproduced host-side in under a minute — the repro is 10 lines — and locked in by four
tests in `test_reload.py::SysModulesRegistrationTests`. **Negative control run**: against
the pre-fix loader those tests give 1 failure and 2 errors; against the fixed one, 4
passes.

**This is a second instance of §16.8's finding, not a repeat of it.** §16.8 found that
`from __future__ import annotations` silently disabled `@store` *injection* and fixed
it in `_store.signature_with_resolved_annotations`. That fix was correct and did not
cover this, because this failure is one layer lower — inside `dataclasses.dataclass()`
itself, before any injection happens. **The lesson is that PEP 563 is a whole class of
divergence, not two bugs**, and nothing in the repo's own fixtures uses it, so the class
is untested by construction. Worth a fixture that does.

### 25.2 `Nav` was missing `min-h-0`, and a tab bar went off the bottom of the display

**Symptom:** the app's bottom tab bar was simply not on screen. No error, correct
scrolling, everything else right.

**Measured rather than guessed.** Dumping the app's real tree and laying it out through
`zkite_pybridge`/`zkite_renderer` at the device's actual viewport (432x960 dp — Moto G54
at density 2.5):

| node | rect |
|---|---|
| synthetic root wrapper | `(0, 0, 432, 960)` |
| **`Nav`** | `(0, 0, 432, **30614**)` |
| screen's root `Column` | `(0, 0, 432, 30614)` |

A flex item's default `min-height: auto` will not shrink below its content, so `Nav`
grew to the `List`'s full 600-row extent instead of the viewport, and everything after
the list sat ~30000dp below the fold.

**Two levels were missing it, and only one is ZKite's.** Isolated by testing each
independently:

| `min-h-0` on | `Nav` rect | screen rect |
|---|---|---|
| neither | 30614 | 30614 |
| screen root only | 30614 | 30614 |
| **`Nav` only** | **960** | 30614 |
| both | 960 | 960 |

`ScrollView` and `List` already imply `min-h-0`; `Nav` is a full-screen host by
construction and did not. Fixed in `_nav.py` with a test asserting it on both render
paths. The screen's own root `Column` is the *user's* container and remains their
responsibility — that is WIDGETS.md's documented "every intermediate `flex-1` ancestor"
rule, which this app is now the worked example for. Both fixes verified live: the tab
bar renders.

**What this says about the conformance harness (§9).** It renders one widget per
fixture at a fixed size, so a bug that only appears when a tall scroll region sits
inside a screen host inside a viewport is invisible to it — the same blind spot §15.1's
"silent divergences" list describes, in a new axis. A fixture with a viewport-height
constraint and a scrollable child would have caught this.

### 25.3 Known gaps

- **Only these two were found because only one app was written.** Nothing here is a
  systematic sweep; a second app of comparable size would likely find more.
- ~~**The PEP 563 class is still untested.**~~ — **closed, see §26.6.**
  `phase_1/zkite_py/tests/test_pep563.py` is a fixture over the whole stack (real
  loader, real `App`, both render paths, a `Nav` screen with a pushed param beside a
  store annotation, dispatch, hot reload) rather than another unit test, and it was
  confirmed to fail against **both** known instances by reverting each fix in turn:
  6/6 failures against the pre-§16.8 resolver, 6/6 errors against the pre-§25.1
  loader. No *demo app* uses the import, which is a smaller remaining gap of the same
  shape — see §26.7 on demo apps as this backend's real test suite.
- **The layout probe was a throwaway.** The `zkite_pybridge` test used to measure §25.2's
  rects was deleted after use rather than kept as a regression fixture. The `_nav.py`
  test asserts the *class* is present, not that a tall screen lays out to the viewport —
  which is the property that actually matters.
- **`mx-auto` is not parsed by `twcore`** — noticed while checking this app's classes
  against the parser, not investigated, and not the cause of anything here. The app uses
  it for centering a line-spine; it silently does nothing on native.
- **The WebView backend was not re-checked for either bug.** §25.1 is backend-agnostic
  (it is `zkite_py`), and §25.2 is real CSS that Blink implements the same way, so both
  are expected to behave identically — expected, not observed.

## 26. `Switch` and `Slider` respond to touch — and the payload they send was wrong on both backends

§15.1's divergence 6, reported by the owner from the same real app §25 came out of:
*"they were there but not working."* That entry was written from a code read and
explicitly **not yet confirmed on a device**, so this session confirmed it first.

**Status: confirmed live, fixed, and re-verified live on the Moto G54.** A `Switch`
toggles, a `Slider` drags (and taps, and clamps), a drag on a slider inside a
`ScrollView` no longer scrolls the list, and the `change` payload arrives as a real
`bool`/`float` instead of a string — the last of which was broken on the **WebView**
backend too, and is the more consequential half.

### 26.1 The confirmation, and the one thing §15.1 read wrong

`demo_app/form_controls.py` (new, kept) is the repro: a bare `Switch`, a `Switch`
inside a row that also has `on_tap`, a `Slider`, and a second `Slider` *inside* a
40-row `ScrollView`, all through the documented `on_change` signature and nothing
else. State is rendered as text and as row background colour so a screenshot answers
each question without a log line.

Pre-fix, on the device, every prediction in §15.1 held:

| action | result |
|---|---|
| tap the bare `Switch` | `Notifications: OFF` — **no dispatch line at all** |
| drag across the `Slider` | `Volume: 40`, unmoved |
| tap the `Slider`'s track | `Volume: 40`, unmoved |
| the `change` payload | `last payload type: -` — nothing ever reached Python |

**The one correction.** §15.1 says a tap on a `Switch` means "no error, no repaint,
nothing." On the device it is worse than nothing *and* the reason is not what it
looks like: `dispatch_tap`'s ancestor walk does not stop at a node it fails to match,
so the tap continues to the nearest `tap` handler above. Logged verbatim:

```
Touch (382, 92) -> dispatching tap to r.1        # r.1 is the *row*, not the switch
```

`rowtaps` incremented 12 → 13 while `Wi-Fi:` stayed `OFF`. So the visible behaviour
was "the wrong thing responded," not silence.

**And that bubbling is not itself the bug — it is conformant.** `zkite-runtime.js`'s
`findBoundAncestor(target, "tap")` walks straight past the `<input>` to the nearest
`data-zkite-on="tap"` ancestor, which is ordinary DOM click bubbling, so the WebView
backend fires the row's handler too. The divergence was only ever the **missing**
`change`. The fix therefore adds the change and *keeps* the bubble; stopping the walk
would have replaced one divergence with another, in the opposite direction. This is
worth stating because "a tap on a switch fires the row" reads like a bug to fix and
is not one.

### 26.2 The second bug, found while fixing the first: the payload is a string

Native could not send a `change` until there was an answer to *what a change payload
is*. Reading that answer out of the WebView backend produced a worse finding than the
one being fixed.

Every backend's event bridge is string-in/string-out — `zkite-runtime.js` sends
`String(payload)` through `ZKiteNative.dispatch`, and the native side calls
`zkite_pybridge::dispatch(id, event, &str)` — and **nothing downstream coerced it.**
Reproduced on the host in one command, before any code changed:

```
payloads received: ['true', '42']   types: ['str', 'str']
bool("false") == True
```

So on the WebView backend the canonical

```python
Switch(checked=s.on, on_change=lambda v: s.update(on=v))
```

stored the *string* `"false"` when switched off, and `"false"` is truthy — **the
switch could be turned on and never off.** WIDGETS.md's "`bool` for `Switch`, `float`
for `Slider`" was false on both backends, and had been since phase 1.

**Fixed by typing the payload at the widget, not at the transport.** Retyping either
bridge would mean touching Java and JNI in three shells to carry information the
widget already has. Instead `RenderContext.register` takes an optional parser and
`Widget._payload_parser(event)` supplies it, so `Switch` → `bool`, `Slider` → `float`,
`TextField` → `str`. Because both backends register events through the *same*
`_register_events`, a widget cannot be typed on one and left broken on the other —
which is the property, not the coercion, that matters here.

Two deliberate choices inside the parsers:

- **Unrecognised means `False`, not truthy.** That single line is the bug.
- **An unparseable slider payload means "no change", not `0.0`** — it falls back to
  the slider's own current value. A malformed payload can only come from a broken
  bridge, and a broken bridge should not also move the user's data.

`"true"`/`"false"` is one wire spelling for both backends by luck worth recording:
JavaScript's `String(el.checked)` and Rust's `bool::to_string` agree.

### 26.3 What shipped, and where each piece lives

| piece | where | why there |
|---|---|---|
| `Control::{Switch, Slider}` + `control_at` | `zkite_pybridge` | Derived from the same `tag`/`attrs` disambiguation `a11y_role` already does, so the two cannot disagree about what a node is. A touch never lands on the control's own node — both widgets synthesize children — so the ancestor walk is the mechanism, not a detail. |
| `slider_value_at` | `zkite_renderer` | **Because `gpu_harness` cannot be tested.** Its `build.rs` compiles gametextinput's C++ against NDK headers, so the crate does not build for the host at all (§15.5). Every piece of arithmetic is on the testable side of that boundary; what is left in the render thread is plumbing. Same call §7.7 made for `hit_test_text_offset`. |
| `dispatch_change_to` | `gpu_harness` | The `"change"` twin of `dispatch_tap_to`. |
| `Gesture::slider` capture | `gpu_harness` | §6.2 disambiguation — see below. |
| `_payload_parser` / `_as_bool` / `_as_float` | `zkite_py` | One implementation, both backends (§26.2). |

**`Switch` and `Slider` are controlled**, and nothing in the render thread keeps a
local value. On-screen state is whatever the last tree said, so a control with no
`on_change` visibly does not move, and one whose handler declines to update the store
snaps back — the same contract `<input type=checkbox>` has under React. A slider drag
holds only the *geometry* it captured, never a pending value.

**Scroll gesture disambiguation (§6.2) is a capture at `TouchDown`, not a direction
test.** When the touch lands inside a `Slider`, `scroll_target` is never populated in
the first place, so there is no race to resolve later. The tempting design — let the
slop test decide, horizontal means the slider and vertical means the scroll ancestor
— is wrong twice: a *vertical* drag starting on a slider must still move the slider
and must certainly not scroll the list under it, and deferring the decision until
slop is exceeded makes the first few millimetres of every slider drag ambiguous,
which reads as the control sticking. A captured gesture also never flings, never
long-presses and never taps on release.

A slider commits its value on `TouchDown` too, which is what makes a plain *tap* on
the track work without a second code path, and is what a real `<input type=range>`
does.

### 26.4 The thumb: a cosmetic gap that stopped being cosmetic

The new `slider_range` conformance fixture (0/50/100 in one canvas) was added to make
the value → geometry mapping visible at the ends, where a single-value fixture cannot
see it. It immediately found something the existing `slider` fixture at value 30
never could. Measured out of the two PNGs rather than eyeballed — §9.5's standing
method — with the track spanning x=16..184:

| value | native thumb | web thumb |
|---|---|---|
| 0 | 16..31 | 16..31 |
| 50 | **100..115** | 92..107 |
| 100 | **185..198** — off the end of its own track | 168..183 |

A browser's range thumb travels over `track_width − thumb_width`, so it stays inside
the track at both ends; `left-[pct%]` puts it at `pct% × W`, which is half a thumb too
far right everywhere and, at maximum, entirely outside the track. This was a known,
documented gap — real Tailwind writes the correction as `-translate-x-1/2`, a
percent-of-own-size translate `twcore` does not implement — and it had been carried
as cosmetic.

**It stopped being cosmetic the moment a drag could reach 100 on native**, which is
this session's own doing. It is also closable with no new `twcore` feature, because
`pct` is known at tree-build time: the percent-of-own-size translate can be emitted as
a plain pixel margin, `-ml-[{pct × 0.16}px]`. After that, the thumb is **pixel-identical
to Chromium's at all three values** (16..31 / 92..107 / 168..183), and `slider`'s own
conformance number falls from 1.28% to **0.02%** — i.e. the offset was essentially the
entire remaining divergence on that fixture.

The *fill* was never divergent. Web's looks shorter only because its thumb occludes
it; both back ends fill to the same x.

### 26.5 Verification

**Host.** `zkite_py` 108 → **121** (7 payload-type tests, 6 PEP 563 fixture tests —
§26.6), `zkite_renderer` 22 → **28** (6 slider-geometry tests), `zkite_pybridge` 6 → **8**,
`cli` 26, `bundler` 18, `twcore` unchanged, `webruntime` e2e 23/23.

Three negative controls, because a passing test proves nothing until it has been shown
to fail (phase 2 §6/M1, §12.5.3, §13.1.2):

- The 7 payload-type tests: **7 failures** against the raw-payload behaviour.
- `slider_value_at`'s tests cover the cases that silently produce a NaN — a zero-width
  rect, `max <= min` — all of which are reachable from ordinary user code
  (`Slider("w-0")`, `Slider(min=1, max=1)`) and would otherwise reach a `@store` and
  render as `NaN` forever with nothing pointing back at a division.
- `a_touch_on_a_synthesized_thumb_resolves_to_the_control_itself` builds through the
  **real** `zkite_py` widgets rather than hand-written JSON, and dispatches a real
  `change` through real CPython, asserting `on=False` and `v=72.0` — the types, not
  just that the tree changed.

**Conformance.** `slider` 1.28 → **0.02 / 0.28** at dpr 1 and 1.16 → **0.03 / 0.14** at
dpr 2; new `slider_range` **0.02 / 0.37** and **0.04 / 0.19**. **Every other fixture is
unchanged to two decimal places at both dprs** against §18.5's table.

**Device — Moto G54 5G (arm64-v8a, Android 15), GPU path**, same repro app, screen-on
and foreground asserted at every capture (§20.2):

```
Touch (382, 36) -> toggling switch r.0.1: false -> true
Touch (382, 36) -> toggling switch r.0.1: true -> false
Touch (382, 92) -> toggling switch r.1.1: false -> true
Touch (382, 92) -> dispatching tap to r.1
```

- **The `false` direction is the one that matters** — under the old payload it would
  have stayed on.
- The last two lines are one tap: the switch *and* the row, which is what the WebView
  does (§26.1).
- **Slider drag:** `Volume: 40 → 88`. Arithmetic, not vibes: the drag ended at 900
  device px = 360 dp, the slider spans 28..404 dp, and (360−28)/376 = 0.883 → **88**.
- **`last payload type: float`** on screen — the type, observed on the device.
- **Extremes:** drags well past both edges clamp to exactly `0` and `100`, with the
  thumb inside its track at both.
- **Disambiguation:** a *horizontal* drag on the slider inside the `ScrollView` set
  `Brightness: 70 → 82` with `Row 0` still at the top; a *purely vertical* drag on the
  same slider set `82 → 46` (x fixed at 200 dp → (200−28)/376 = 0.457 → **46**, exact),
  again without scrolling. Zero `fling` lines for either. The control — a drag starting
  on a list row — still scrolls and still flings.

### 26.6 What this says about the conformance harness, and the PEP 563 fixture

**§9's harness cannot see any of this and this session did not pretend otherwise.** It
renders one state and never touches anything. What it *can* see is the rendered
consequence of a state change, and that is what `slider_range` adds — which was worth
adding on its own merits, since it is what found §26.4. The `switch` fixture already
rendered both states. Interaction coverage lives instead in `zkite_renderer`'s host
tests for the geometry and in `zkite_pybridge`'s for the dispatch; only the touch
plumbing needs a device, which is the smallest that surface can be made without a
device-in-CI story that does not exist (§9.3).

**Queue A item 2, §25.3's "the PEP 563 class is still untested", is closed** by
`phase_1/zkite_py/tests/test_pep563.py`. It is deliberately a *fixture over the stack*
rather than another unit test: one realistic `main.py` with the future import, through
the real loader, the real `App`, both render paths, a `Nav` screen with a pushed param
sitting next to a store annotation, event dispatch, and a hot reload. Both known
instances of the class are covered, each confirmed by reverting its own fix:

| control | result |
|---|---|
| pre-§16.8 resolver (annotations left as strings) | **6 failures / 6** |
| pre-§25.1 loader (registers in `sys.modules` after exec) | **6 errors / 6**, with the original `AttributeError: 'NoneType' object has no attribute '__dict__'` |

### 26.7 Known gaps

- **`demo_app/main.py` was the reason this survived three phases, and that is the
  more general finding.** It wired its `Switch` with **`on_tap`** — an undocumented
  prop that happens to land on `dispatch_tap`'s one working path — and its `Slider`
  had no handler at all. So every device verification in this document exercised a
  switch through an API no user would write, and never had to move a slider. Both are
  now on the documented signature. **The demo apps are the test suite for the native
  backend, and nothing checks that they use the public API.**
- **Only `tap` and `change` are dispatched natively.** `on_input` (WebView's
  per-keystroke event, distinct from `change`) is registered by `_register_events` on
  both backends and dispatched by neither on native. Unchanged by this work, now
  named. **Fixed in §34.1** — native now dispatches `input` as well as `change` from
  every text edit, and the *timing* difference this exposed in `change` itself is filed
  in §15.1.
- **A `TextField` inside a tappable ancestor still does not bubble its tap**, where
  the WebView would. `dispatch_tap` stops at a text field. Pre-existing, out of scope
  here, and a real (small) divergence of the same family as §26.1.
- **The relative ordering of `change` and the bubbled `tap` is not verified against
  Chromium.** Native dispatches the control's own `change` first, on the argument that
  the direct target's semantics should not depend on what an ancestor's handler did to
  the tree. `zkite-runtime.js` listens in the capture phase, so its order may differ.
  Both events fire on both backends, which is the part that was broken; the order is
  an unverified difference, not a measured agreement.
- **The `Slider` conformance fixtures are static.** They pin value → geometry; nothing
  pins that a *drag* produces those values, because the harness cannot touch anything.
  `slider_value_at`'s host tests cover the mapping and the device run covers the
  gesture; nothing covers the join except by inspection.
- **No `step` other than 1, and no non-zero `min`, was exercised on a device.** Both
  are covered by host tests (`slider_step_quantises_from_min_not_from_zero`), and the
  device only ever ran `min=0 max=100 step=1`.
- **The WebView backend's `Switch`/`Slider` were not re-run on a device** after the
  payload fix. The change is in `zkite_py`, which both backends share, and `e2e.mjs`
  covers the JS half in real Chromium — but "expected to behave identically" is what
  §25.3 already had to say about the last shared-`zkite_py` fix, and it is a different
  claim from having watched it.
- **Multi-touch is undefined.** A second finger landing while a slider is captured is
  not modelled anywhere in `render_thread` (it never has been — `Command::Touch*` carry
  no pointer id), so two fingers on two sliders is not a thing that works.
- **`Checkbox`, `Radio`, `Select` and the pickers still do not exist** (§15.1). This
  takes the native backend's working form vocabulary from one control to three, which
  is what plan1.md §14.3 always claimed it was; it does not add a fourth.

## 27. Horizontal scrolling — and the two bugs the axis made visible

§15.1's divergence 1, and the only entry on that list that was load-bearing for a
whole widget class rather than for polish. Picked because §26 had just rebuilt gesture
resolution at `TouchDown` and added a second thing that can own a drag, so the
disambiguation thinking was fresh and `scroll_offsets` was already `(f32, f32)` with
only `.1` ever written.

**Status: built, and verified live on the Moto G54.** A horizontal strip drags, flings
and clamps; a strip inside a vertical list takes horizontal drags while the list takes
vertical ones; a strip with nothing to scroll passes the drag through instead of
swallowing it.

**Scope, stated up front because the gap list overstates it.** §15.1 calls this "the
blocker for `Carousel`/`PageView`", which is true and reads as more than it is:
horizontal scrolling **unblocks** those widgets and does not deliver them. A carousel
needs snap-to-page — a different animation from fling decay, plus a page concept.
Neither exists. What shipped is a horizontally scrollable container.

### 27.1 The thing that would have made this silently break vertical scrolling

Found by reading `parse.rs` before writing any code, and it is the decision the whole
session turns on.

`ScrollView.implied_classes` is `flex flex-col overflow-auto min-h-0`, and `twcore`
maps **`overflow-auto` to `Axis::Both`** (`phase_0/spike3_twcore/src/parse.rs:243`).
So *every `ScrollView` and every `List` that has ever existed in this framework
already declares itself horizontally scrollable*, and always has.

The obvious implementation — mirror `is_scrollable_y` into `is_scrollable_x`, walk
ancestors, take the first match — therefore resolves a horizontal drag on an ordinary
vertical list **to that list**, which then moves nothing, because it has no horizontal
room. Not a crash, not an error: a dead gesture. That is the exact silent-divergence
shape §15.1 exists to track, and it would have shipped looking like "horizontal
scrolling works" on the one fixture built to test it.

**So `scrollable_ancestor` filters on room, not on the declaration**, and a node with
no room is *skipped* rather than terminating the walk — which is also the browser's
scroll-chaining behaviour, resolved once at touch-down.

The negative control is in the suite. Reverting the filter to the declaration-based
walk fails 2 of the 3 `scrollable_ancestor` tests, and the failure message is the trap
itself:

```
assertion `left == right` failed: a horizontal drag must resolve to nothing at all, not to the list
  left: Some((NodeId(4294967302), 0.0))
 right: None
```

A **negative result worth keeping**, in the same spirit as §18.5's arc-sweep-flag one:
`a_horizontal_strip_inside_a_vertical_list_resolves_a_different_target_per_axis` passes
against *both* implementations, because the strip both declares x and has room. It is a
good test of the two-targets design and it does **not** discriminate the room filter.
Only the zero-room fixtures do.

### 27.2 Two targets and a lock, not one target and a direction test

`Gesture` now carries `scroll_x` and `scroll_y` — two independently resolved targets —
plus an `axis` decided later. The nesting case forces this: from a touch on a card
inside a horizontal strip inside a vertical list, the x answer is the strip and the y
answer is the list. One target plus an axis would push the axis decision back to
touch-down, where it cannot be made.

**The axis is locked at the slop crossing, and that is the opposite call from §26.3's
slider capture.** Worth stating together, because §26.3 argues hard for
capture-at-touch-down and this is not a departure from it:

| | `Slider` (§26.3) | scroll axis (here) |
|---|---|---|
| decided at | `TouchDown` | the instant `dragging` becomes true |
| why | a slider **commits a value** at touch-down, so any window where the answer is unknown is a window where the control visibly does not respond | scrolling **moves nothing** until slop is exceeded (`TouchMove` returns early while `!dragging`), so the axis is decided at exactly the first moment anything could move |

There is no ambiguous window in either design; the two differ because the first pixel
of a slider drag is meaningful and the first pixel of a scroll drag is not.

**And it locks rather than panning freely.** A finger is imprecise, so an unlocked 2D
pan turns every intended horizontal swipe into a diagonal drift — which is why every
native touch scroller axis-locks. The cost, named rather than hidden: a genuinely
2D-scrollable container moves on one axis at a time. Ties go to Y, since a perfect 45°
drag is not something a finger produces on purpose and biasing to the axis that has
always worked keeps an ambiguous gesture doing what it did before this session.

A fling inherits its drag's axis and never re-decides — it is that drag's tail.

### 27.3 Where each piece lives

| piece | where | why there |
|---|---|---|
| `ScrollAxis`, `max_scroll`, `is_scrollable`, `scrollable_ancestor` | `zkite_renderer` | **Because `gpu_harness` cannot be host-tested at all** — its `build.rs` compiles gametextinput against NDK headers (§15.5). Every scroll decision that is tree arithmetic sits on the testable side of that line; what is left in the render thread is plumbing. The same split §26.3 made for `slider_value_at` and §7.7 for `hit_test_text_offset`. |
| per-axis target resolution, axis lock, axis-aware fling | `gpu_harness` | touch state, which is what that crate owns |
| `ScrollView(horizontal=True)` | `zkite_py` | one front door, shared by both backends |

`max_scroll_y`/`is_scrollable_y` survive as thin wrappers, because most callers and
every existing test genuinely mean Y.

### 27.4 A pre-existing bug the new axis made visible: end padding was never scrollable

Flinging the bare strip to its bound and *measuring* the result rather than eyeballing
it (§9.5's standing method) found the last card flush against the screen edge:

```
before:  card 19 emerald runs to x=1079 of 1080   -- 0 px of the strip's px-2 padding
after:   20 px of strip background at x=1060..1079 -- 8 dp at density 2.5, exactly px-2
```

`max_scroll` computed `content_end − viewport`, and CSS puts the scrollable overflow
area at the content edges **plus the end padding** (the scrollport being the padding
box). The arithmetic confirms the pixels: content ends at 1600 dp, viewport is 432, so
the old code gave **1168** — exactly what the device logged — where CSS gives
`1608 − 432 = ` **1176**, which is exactly what it logs now.

**This was never x-specific.** `max_scroll_y` had the identical shortfall from §6.1
onwards, so a `p-2` vertical `ScrollView` has always lost its bottom padding at the end
of every scroll. Nothing caught it because the conformance harness renders only offset
0, and 8 dp missing *at a bound* is invisible unless you go to the bound. Fixed for
both axes; the negative control fails at `108` against the expected `116`, i.e. exactly
the 8 px.

This is the second time this session that the new axis surfaced something that was
always true on the old one. Both were found by running it, not by reading it.

### 27.5 Verification

**Host.** `zkite_renderer` 28 → **33**, `zkite_py` 121 → **124**, webruntime e2e 23 →
**25**, `zkite_pybridge` 8, `cli` 26, `bundler` 18, `twcore` 54 — all passing.

Negative controls, because a passing test proves nothing until it has been shown to
fail (phase 2 §6/M1, §12.5.3, §26.5):

- the declaration-vs-room walk: **2 failures of 3**, quoted in §27.1
- the end-padding fix: **1 failure**, `108` vs `116`
- and one deliberate non-result: the nesting test passes either way (§27.1)

**Conformance.** New `scrollview_horizontal` fixture at **0.84 / 0.99** (dpr 1) and
**0.63 / 0.73** (dpr 2). **Every other fixture is unchanged to two decimal places at
both dprs** against §26.5's table, before and after the padding fix.

The harness cannot scroll anything, and this section does not pretend otherwise. What
the fixture pins is the half it *can* see, and that half is not nothing: a horizontal
strip is the first thing in this project whose content deliberately runs past its own
right edge, so "where does the fourth card get cut" is a real question a vertical
fixture cannot ask. Measured out of the two PNGs rather than eyeballed, the cards are
**pixel-identical** on both backends — `8..67`, `76..135`, `144..199`, with the third
cut at the same pixel. The 0.84% residual is glyph AA on the digits.

**Device — Moto G54 5G (arm64-v8a, Android 15), GPU path**, `demo_app/horizontal_scroll.py`,
screen-on and foreground asserted at every capture (§20.2). The artifact was checked
before installing (§15.6.1): the repro as `assets/app.zip`'s `main.py`, the new
`widgets.py` inside `zkite-framework.zip`, and the new fling log line in the `.so`.

| case | result |
|---|---|
| horizontal drag on a bare strip | cards 0..5 → 4..9; strips 3/4 and the list **unmoved** |
| horizontal drag on a strip with **no room** | **zero fling lines**, nothing on screen moved at all |
| **vertical** drag starting **on** a strip | list scrolled row 0 → 2; **every strip still at card 0**, including the one under the finger |
| horizontal drag on a strip inside the list | that strip 0 → 4; sibling strips and the list **unmoved** |
| fast X fling | `5064px/s on X (max_scroll 1176)`, landed on card 19 clamped at the bound, no overscroll glitch |
| vertical fling after the padding fix | `8499px/s on Y (max_scroll 963)`, reached row 11 of 12 and clamped — no regression |
| 8 rapid alternating swipes, back-to-back | **8 flings, all on X, correctly alternating sign** (+4762 −5475 +5095 −5500 +3648 −5503 +5027 −4772), each within ~100–140 ms of its predecessor — no growing lag, no post-lift tail |

That last row is §6.5's stress pattern applied to the new axis, and it was run
deliberately: an earlier *batched* run produced a result that could not be attributed
to the swipe that caused it, during an `adb` reconnect. Rather than explain it away,
every case was re-run in isolation from a clean launch (all clean), and then the
overlapping-input pattern that could have produced it was tested on purpose. Zero
`FATAL`/`PANICKED`/`E AndroidRuntime` markers across the session; process alive
throughout.

### 27.6 Owed from §26, both closed — and one of them is a confirmed divergence

**The WebView `Switch`/`Slider` after the payload fix, on a device.** §26.7 recorded
this as expected-but-unwatched, and §25.3 is the precedent for why that is not the same
claim. Built with `zkite build apk --backend webview`, artifact checked for
`_payload_parser`/`_as_bool` before installing, run on the Moto G54: the switch goes
**ON and back OFF** — the direction that was broken, since `bool("false")` is `True` —
with **`last payload type: bool`** rendered on screen, and both sliders untouched by
the tap.

One thing worth recording about *how* that was verified. A first pass showed both
sliders moving on a switch tap, which looked like a real bug. It did not reproduce: a
clean launch, a 6-second idle control (no drift) and a single isolated tap left both
thumbs at exactly their baseline x. It was stray input during that sequence — this is
the owner's actual phone, which the session prompt warns about specifically. Recorded
because the first observation was wrong and the isolation is what showed it.

**The `change`-vs-bubbled-`tap` ordering — and it is a real divergence.** §26.7 called
it "an unverified difference, not a measured agreement." It is now measured, and it is
a difference. Settled in **real Chromium** via the webruntime e2e suite rather than on
the device, because the WebView shell logs one aggregate `tap-to-paint` line and no
per-dispatch order at all:

```
Chromium: ["r:tap=", "r.0:change=true"]     <- the row first
native:    change first, then the bubbled tap (§26.3)
```

The cause is not subtle once measured: `zkite-runtime.js` listens for `tap` in the
**capture** phase, which runs root → target, so the row is reached before the input
inside it, while `change` is a separate and later browser event.

**Pinned in the direction it actually goes rather than "fixed".** Choosing the order is
a public-API contract decision — it decides the final state whenever a row handler and
a control handler write the same `@store` field — not a bug with an obvious right
answer, and fixing the WebView side means restructuring the listener that the focus,
blur and scroll behaviour is already tuned around. The e2e check asserts the
wrong-looking value on purpose, the same move `zkite_renderer`'s
`expected_html_only_classes_are_unknown` makes, so that a silent flip in either backend
fails loudly. Filed as §15.1 divergence 4b. **The contract itself is an open decision.**

The markup in that test is not hand-written — it is exactly what `zkite_py` emits for
`Row(on_tap=...)` wrapping `Switch(on_change=...)`, so it cannot pass against a shape
the framework does not produce. That is §26.7's own lesson applied one layer down.

### 27.7 Also done: §23.4's five-minute landmine

`build_cmd.rs:256` hardcodes `stdlib_prefix` to `vendor/python-x86_64/prefix`
regardless of the ABI being built. §21.6 flagged it as "undocumented and reads like a
bug"; §23.8 item 2 asked for it to be documented rather than fixed. Done, in place: the
comment now says the pure-Python stdlib is architecture-independent, that the
per-architecture parts (`lib-dynload`, `lib*_python.so`) are staged separately, and
that the obvious "fix" would emit a byte-identical second copy and **double** what a
`pip install zkite` wheel has to carry — one 16 MB stdlib per ABI instead of one shared
copy, which is most of what keeps that wheel at ~19 MB (§23.3).

### 27.8 Known gaps

- **`Carousel`/`PageView` still do not exist.** Unblocked, not built — they need
  snap-to-page, a different animation from fling decay, and there is still no animation
  driver beyond the fling (§15.1).
- **No horizontal virtualisation, and `sync_viewports` is deliberately still y-only.**
  `List` has no horizontal form, so nothing on the Python side could consume an x
  report. The consequence: a horizontal strip's scroll position is never visible to
  Python, and it **builds all of its children every frame**. Fine for the tens of cards
  a strip holds, wrong for thousands.
- **Scroll chaining is resolved once at touch-down, never mid-gesture.** A drag that
  scrolls its target to a bound stops there rather than continuing on the ancestor,
  which is what a browser would do. Deliberate — re-resolving mid-drag reintroduces
  exactly the ambiguity §27.2 removes — but it is a real difference.
- **No overscroll / rubber-band on either axis**, unchanged (§5 item 4). The x axis
  inherits the y axis's stop-dead-at-the-bound behaviour.
- **The axis lock has no escape hatch.** A container that legitimately wants 2D panning
  cannot get it, and nothing warns.
- **The conformance fixture is static and cannot scroll**, same limit §26.6 records for
  the slider fixtures. It pins layout and clipping at offset 0; the *bound* — which is
  where §27.4's bug lived — is covered by host tests and one device measurement, and by
  nothing automated.
- **The end-padding fix is verified against CSS by arithmetic and one device
  measurement, not against Chromium at a scroll bound.** The conformance harness cannot
  scroll, so no fixture compares the two backends' actual maximum scroll offset. If the
  two disagree by a border width somewhere, nothing would catch it.
- **`ScrollView(horizontal=True)` was not run through `zkite dev`,** only through a
  bundled-app harness build and (for the payload work) `zkite build apk`. Hot reload of a
  horizontal strip is untested.
- **One unexplained observation, kept rather than dropped:** a single Y fling line
  appeared alongside an X one during a re-install/relaunch sequence, from a purely
  horizontal swipe. It did not reproduce across three subsequent clean launches (one
  fling each, all X). Most likely a residual touch from the install/launch handoff; not
  chased further, and not claimed to be nothing.
- **Multi-touch is still undefined** (§26.7) — `Command::Touch*` carry no pointer id, so
  two fingers on two strips is not a thing that works.
- **One device, one ABI, the GPU path.** The CPU fallback carries identical code and was
  not forced on; `conformance` exercises `vello_cpu` on the host for the static half.

## 28. Borders, shadows and rings — and the v3 defaults that were still underneath them

§15.1's "missing rendering features on native", second entry, and the one the owner
called the biggest user-visible rendering gap on the default backend:

> **Borders, shadows, rings are not painted at all.** `twcore` resolves them into
> `Paint`; no backend draws them (§3, §5).

**Status: built, and verified live — originally on an x86_64 emulator, and now on the
Moto G54 too (§29.8). §28.6's top open item is closed.**
All three paint on both native paint paths, 21 conformance fixtures are clean at both
dprs with every pre-existing number unchanged, and four negative controls were run —
one of which did not discriminate, which is the most useful result in this section
(§28.4). ~~**The Moto G54 run is outstanding and is the top open item** (§28.6)~~ — done,
on real PowerVR silicon at density 2.5, with every claim re-measured arithmetically:
`currentColor` borders and rings match v4's palette at distance **0.0**, per-side widths
and the mitre are exact, and paint order is confirmed. **It also found something the
emulator run could not**: `shadow-2xl` is ~1.75x too dark at the card edge against real
Chromium — the one entry in the scale whose blur sigma exceeds its own rect — and the
conformance harness reports **0.00%** with that divergence fully present, which is the
first concrete instance of the blindness §28.4 could only demonstrate with a control.

**Why this item and not the rest of the queue.** Not because it is biggest. Because it
is the only entry on §15.1's list that §9's conformance harness can *see and
discriminate*: §26.6 and §27.5 both had to concede that the harness cannot touch or
scroll anything, but a border is a static thing at offset 0, so this work gets real
per-fixture comparison against real Chromium with real negative controls — the
strongest evidence available in this repo without a device, and available here and not
for `Image` or `dark:`.

### 28.1 The finding: `twcore`'s border, shadow and ring defaults were all still v3's

Found before writing any paint code, by doing what §9.6 did for the palette — asking
the source of truth rather than reading the note. §9.6's own known-gaps entry predicted
this exactly and nobody had checked:

> **Only the default palette was regenerated.** v4 also changed defaults that are not
> colours (spacing scale derivation, default border colour, `shadow-*` values).
> Nothing here checked those.

Measured in **real Chromium**, driving the very `@tailwindcss/browser` bundle
`phase_1/webruntime/vendor/` pins, reading computed styles rather than pixels:

| | `twcore` (v3) | Tailwind v4, as Chromium computes it |
|---|---|---|
| default `border-color` | `#e5e7eb` gray-200 | **`currentColor`** — `border-2` computes `rgb(0, 0, 0)`, and `border-2 text-red-500` computes the red |
| default ring width | 3px | **1px** — a bare `ring` computes `0 0 0 1px` |
| default ring colour | blue-500 | **`currentColor`** — `ring-2 text-emerald-500` computes the emerald |
| `shadow-sm` | `0 1px 2px 0 /0.05` | **`0 1px 3px 0 /0.1` + `0 1px 2px -1px /0.1`** |
| `shadow-xs`, `shadow-2xs` | absent | present |
| `shadow-sm`/`md`/`lg`/`xl` | one layer each | **two layers each** |

Two distinct mistakes are stacked in there, and the second is the instructive one.

**v4 renamed the whole shadow scale.** v4's `shadow-xs` is v3's `shadow-sm`, and v4's
`shadow-sm` is v3's bare `shadow`. So `shadow-sm` on the WebView backend and
`shadow-sm` on the native one were not the same shadow drawn two ways — they were
different shadows.

**§9.6 fixed the wrong half of the ring default.** It updated the literal from v3's
blue-500 hex to v4's, with a comment saying it "has to track the palette to stay
conformant". That was true and insufficient: v4 does not default a ring to blue-500 at
*any* hex. The value was corrected while the semantics stayed v3's. That is a sharper
version of the same lesson §9.5 recorded — a divergence can survive being looked at, if
what gets checked is the number rather than the rule.

None of this was observable before this session **because nothing painted any of it**.
That is the whole shape of the thing: a silent divergence inside a silent divergence.

`BoxShadow` is now a fixed-capacity layer list that **refuses** an over-capacity list
rather than truncating (`BoxShadow::new` returns `None`, and the theme table asserts on
it), because a dropped layer renders as a plausible-looking wrong shadow.

### 28.2 Two conversions that are easy to get subtly wrong

Both vello backends expose `fill_blurred_rounded_rect(rect, radius: f32, std_dev: f32,
invert: bool)` and `stroke_path`/`push_clip_path`, exactly as §18.1's table promised.
What §18.1 could not say, because it was reading signatures rather than using them:

- **A CSS blur radius is not a Gaussian standard deviation.** `box-shadow`'s third
  length is defined so the edge transitions over roughly that distance, i.e. `sigma =
  blur / 2`. The primitive wants sigma. Passing CSS's blur straight through renders
  every shadow twice as soft as Chromium's — which looks deliberate, so nothing would
  report it as a bug.
- **The primitive takes one uniform radius**, where `Corners` has four. Exact for every
  `rounded-*` that is not a per-corner utility; the mean otherwise, which is a real
  approximation and is named in §28.6 rather than hidden.

And a third that only showed up under test: **`rounded-full` is 9999dp**, so every
radius has to be clamped to what fits the rect or `kurbo` builds corner arcs larger
than the box. The stroked paths clamped from the start; a host test caught the *shadow*
radius not doing the same, which would have been a garbled shadow under every
pill-shaped button.

### 28.3 §18.6's correction was right, and the harder half is corners

§18.6 said a uniform border is one stroked rect but the general case is four edge
paths, and warned against sizing it as trivial. That held. What it did not name is that
the difficulty is **corners**, not edges: a bordered rounded rect is two concentric
rounded rects, and where adjacent sides differ in width or colour the join is a mitre
across the corner *arc*.

So there are two paths, and the split is a fidelity decision rather than only a cheap
one:

- **Uniform width and colour → one stroked centreline.** *Exact*: a stroke of width `w`
  centred `w/2` inside the border box has its outer edge exactly on that box and its
  inner edge exactly on the padding box, which is what CSS draws. It is also the
  overwhelmingly common case.
- **Anything else → per side, fill a band clipped to a wedge.** The band is the region
  between the outer and inner rounded rects, built as one path with the inner contour
  wound backwards so an ordinary non-zero fill leaves it hollow; the wedge is the
  quadrilateral from that side's outer corners to its inner ones, so adjacent sides meet
  on the corner diagonal. This avoids boolean path ops (`kurbo` has none) and an
  even-odd fill rule (neither backend's `fill_path` takes one). It has anti-aliased
  seams along the mitres, which the uniform path does not — hence the split.

A **ring** is neither: v4 implements it as a spread-only, zero-blur `box-shadow`
*outside* the border box, so it is a stroke on a centreline `w/2` **outward**. It is the
first thing in this renderer that paints outside its own node's rect, and nothing in
taffy reserves room for it — only an ancestor's clip stops it escaping a scroll
container.

**Where each piece lives** — the same split §18.3 made for icons, for the same reason:

| piece | where |
|---|---|
| `PaintColor::{Current, Fixed}`, the v4 defaults, `BoxShadow` layers | `twcore` |
| all geometry: corner maths, blur→std_dev, uniform-vs-per-side, clamping | `zkite_renderer::decoration` |
| `set_paint`/`fill`/`stroke`/`clip` call sequences | each of the four paint consumers |

`PaintColor` deliberately mirrors `IconPaint`, which already existed for exactly this
(SVG's own `currentColor`) and resolves the same way, black fallback included — which
is also what Chromium computes for `border-2` with no `color` set.

**Decoration is painted in the same pass as the background, per node, in tree order**,
rather than as a separate whole-tree pass. A late border pass would paint a parent's
border over a child's background. Within a node the order is shadows → ring →
background → border, and **the shadow list is walked in reverse**, because
`box-shadow`'s first layer is the topmost one. That ordering is why the sequence is a
named function per backend rather than inlined at each call site: getting it backwards
is invisible on a one-layer shadow and wrong on every multi-layer one, which is all of
v4's sm/md/lg/xl.

### 28.4 Verification — including the control that did not discriminate

**Host suites.** `zkite_renderer` 33 → **44**, `twcore` 54 → **57**, `zkite_py` 124,
`zkite_pybridge` 8, `cli` 26, `bundler` 18, webruntime e2e 25/25 — all passing.

**Conformance: four new fixtures, 21 total, both dprs.**

| fixture | dpr 1 (loose / strict) | dpr 2 (loose / strict) |
|---|---|---|
| border | 0.28 / 1.11 | 0.43 / 0.91 |
| border_sides | 0.12 / 0.27 | 0.11 / 0.17 |
| shadow | 0.00 / 0.00 | 0.00 / 0.00 |
| ring | 0.10 / 0.42 | 0.04 / 0.20 |

**Every pre-existing fixture is unchanged to two decimal places at both dprs** against
§27.5's table — `button` 1.78/3.20, `text` 5.00/6.58, `textfield` 1.89/4.07,
`typography` 9.95/13.68, `list` 2.99/4.16, `slider` 0.02/0.28, and the rest.

**Four negative controls, because a passing fixture proves nothing until it has been
shown to fail** (phase 2 §6/M1, §12.5.3, §13.1.2, §26.5, §27.1):

| control | result |
|---|---|
| v3's border/ring default colours restored | `border` 0.28 → **3.68%**, `ring` 0.10 → **1.03%** |
| uniform stroke forced for the per-side case | `border_sides` 0.12 → **6.94%** |
| the `blur / 2` conversion removed | **no change at all: 0.00%, max_delta 5** |
| shadows not painted at all | 0.00% loose, **0.83% strict**, max_delta 32 |

**The third one is the finding.** Doubling every shadow's blur changed the render — the
`shadow-lg` tail moved from y=90 to y=94, measured — and the harness reported the
identical 0.00% / max_delta 5. The fourth control explains why, and it is §9.5's lesson
recurring in a new place: **Tailwind's entire shadow scale is fainter than the
tolerance.** Every shadow in the fixture peaks at 10–32 levels of contrast against a
white background, and `CHANNEL_TOLERANCE` is 40 — so *an entirely absent shadow* also
scores 0.00% on the loose column. Only §9.5's strict column sees absence, and *nothing*
in the harness sees softness.

So `shadow`'s 0.00% is real agreement and a **weak guard**. What actually pins the blur
conversion is the host test `a_css_blur_radius_is_half_a_gaussian_std_dev`, which does
fail against the control (6.0 vs 3.0). Recorded here rather than quietly fixed, per
§27.1's precedent that a control which does not discriminate is worth keeping — and
because the next person to add a shadow feature should know the harness will not catch
them. `shadow-2xl` was added to the fixture on the theory that the darkest entry in the
scale might discriminate; it does not either, which is why the fixture is five cards
and this paragraph exists.

The `border_sides` fixture is unmoved by the *first* control, correctly — every side
there carries an explicit colour, so a default it never consults cannot change it. A
second deliberate non-result, same category.

**Device: x86_64 emulator (API 34, SwiftShader/Vulkan, density 2.75), GPU path.**
`demo_app/decoration.py` — written against the documented styling surface only, no
`@store`, no private modules, per §26.7 — on a deliberately *light* background, because
on `bg-slate-950` a correct shadow and an absent one look identical.

The artifact was checked before installing (§15.6.1): `assets/app.zip`'s `main.py` is
the new demo and the packed `.so` contains the new module. `adb uninstall` then
`pm clear` first, per the two staleness traps.

All six rows render correctly. Two measurements out of the screenshot rather than an
eyeball, since both are exactly what the v4 correction changed and both are arithmetic:

- **A bare `ring` measures 3px at density 2.75 = 1.09dp, coloured `(200, 0, 222)`** —
  v4's 1dp `currentColor` picking up the parent's `text-fuchsia-600`. v3's default would
  have been 3dp of blue-500 (~`(43,127,255)`), i.e. three times as thick and a different
  hue.
- **`border-t-8 border-b-4 border-l-2` measures 22px / 11px = 8.00dp / 4.00dp**, and a
  scanline through the middle of the swatch finds **one** red run (the 2dp left edge)
  and no right-hand band — which is the per-side geometry being right, including the
  side that was never declared.

`rounded-full` renders as a true circle (the radius clamp), the per-side rounded swatch
mitres amber into sky across the top corners, all six shadow sizes are distinguishable
and correctly ordered, and the combined node shows shadow under background under border
with the ring outside it — the paint order, which is the only thing row 6 checks.

Zero `PANICKED`/`FATAL EXCEPTION`/`E AndroidRuntime`/`ANR` markers across the whole
session; `pidof` constant throughout, **including across a `uiautomator dump`**, so
§8.7's accessibility-abort guard still holds, and the dump reports every label.

### 28.5 What this deliberately does not do

- **Inset shadows.** `shadow-inner` exists in v4 (`inset 0 2px 4px 0 /0.05`) and
  `twcore` does not parse it. The primitive's `invert: bool` is the plausible mechanism
  and its semantics were not verified, so no field was added — a parsed-and-never-read
  field is the §9.7 failure mode this document has now paid for twice.
- **Coloured and arbitrary shadows** (`shadow-red-500`, `shadow-[...]`) — unparsed
  before this session and unparsed after.
- **`ring-offset-*`** — not parsed by `twcore`, so a ring always sits flush against the
  border box.
- **Gradients** — §15.1 item 3's other half, untouched. §18.1 established the brush
  exists; the parser still does not.

### 28.6 Known gaps

- ~~**Not verified on the Moto G54.** This is the top open item.~~ — **closed, see
  §29.8.** Run on the real phone (arm64-v8a, Android 15, density 2.5, PowerVR
  BXM-8-256): every row correct, every claim re-measured as arithmetic out of the PNG,
  idle control pixel-identical, zero crash markers. The emulator paragraph below stands
  as written and its caveat was the right one — the phone run found a real divergence
  (`shadow-2xl`, §29.8) that SwiftShader at density 2.75 did not surface. Everything
  above was an **x86_64 emulator on SwiftShader at density 2.75** — a different ABI, a
  different GPU implementation and a different density from every other device number in
  this document, and phase_0/README.md's own standard is that an emulator pass is
  evidence, not proof. Between the two runs there are now two densities and two GPU
  implementations, which is better than either and still not a matrix.
- **`shadow-2xl` is measurably too dark** (§29.8): ~1.75x Chromium's darkness at the card
  edge, 40% heavy by mass, with the extent correct and only the distribution wrong. It is
  the only entry whose blur sigma exceeds its own spread-shrunk rect, i.e. the extreme of
  `fill_blurred_rounded_rect`'s analytic approximation. **The harness scores this 0.00%.**
  Found on the device, left unfixed deliberately.
- **The conformance harness cannot see shadow *correctness*, only shadow absence, and
  only on the strict column** (§28.4). Anyone changing the blur maths is guarded by one
  host test and nothing else.
- **Non-uniform corner radii get a mean radius on shadows.** Exact for every uniform
  `rounded-*`; a `rounded-t-lg` box's shadow is slightly wrong at the corners. Neither
  backend exposes a per-corner blurred rect.
- **A border wider than its own box draws nothing** rather than clamping to what fits.
  Reachable from ordinary user code (`Column("border-[80px]")` in a narrow parent) and
  refused deliberately, since an inverted rect is normalised by the primitives into a
  visible artefact. CSS instead scales the widths down proportionally.
- **The per-side path has AA seams on the mitre diagonals**, where the uniform stroke is
  exact. Visible only under magnification against a high-contrast pair of side colours;
  `border_sides` at 0.12% is the measured cost.
- **A radius smaller than the border width diverges slightly.** CSS clamps the inner
  radius to zero while a centreline stroke would want a negative one; `kurbo` clamps,
  which is close but not identical. Not measured.
- **The CPU fallback path carries identical code and was not forced on.** Same gap
  §18.7 recorded for icons; `conformance` exercises `vello_cpu`'s rasteriser on the
  host, which is evidence for the geometry but not for that path on a device.
- **The decoration call sequence is now duplicated across four paint consumers**, with
  an ordering contract (shadows in reverse) that a copy can get wrong silently. Factored
  into named functions inside `gpu_harness` so its two paths cannot drift; `conformance`
  and `spike1` keep their own copies, matching the icon precedent. A shared trait over
  the two vello types is the obvious cleanup and was not attempted.
- ~~**Nothing re-checks that `twcore`'s remaining v4 defaults are v4's.**~~ — **closed,
  see §36**, which audited all 104 of them against real Chromium. This session corrected
  the three that borders/shadows/rings depend on. §9.6's own list also names the
  **spacing scale derivation**, which nothing here measured and which would show up as
  every padding and gap being subtly wrong — the same shape, still unchecked.

  > **Answered (§36), and the guess in the paragraph above was wrong.** The spacing
  > scale is v4's and always was — 41 values agree, including the fractional and
  > non-v3 multiples (`p-4.5`, `p-13`) that only v4 can express. So do the font sizes,
  > line heights, weights, `leading-*`, `tracking-*` and all five breakpoints. What was
  > still v3's is the one table nobody named: the **radius scale**, carrying the
  > identical rename this section found in the shadows.
- **`Image` still paints an empty box** (§18.6), and `dark:` still never activates
  (§15.1 divergence 2). Both are untouched by this work.

## 29. `payload_root()` — `zkite build` runs on a machine that is not this checkout

§23.7 step 1 and §23.8 item 1, the one thing on that plan called *"the only
purely-blocking item"*:

> **`zkite build` runs on exactly one machine — this checkout.** `repo_root()` is a
> compile-time absolute path, so the binary is not portable to a second Linux box,
> never mind another OS.

**Status: built, and verified by running `zkite build` from an installed binary with
its payload beside it, outside this checkout.** Also in this section: §17.4 item 2's
per-ABI `version_code` collision, promoted above the release pipeline by the owner and
closed; a real bug in `zkite doctor` that only a staged payload could expose; and a
**pre-existing intermittent test failure in `zkite_pybridge`** found on the way, which
is recorded rather than fixed and is the most interesting thing here (§29.5).

**Why this item.** Two reasons, and the second one is the session's, not the plan's.
It is upstream of everything about distribution and it is an afternoon. But it is also
**the only queued item whose verification bar this repo can meet with no phone
attached** — the Moto G54 was not connected for this session either (§29.6), and
§23.9's own list names the confirmation that was missing: *"Nor on a second Linux
machine. That is the cheapest possible confirmation of the `repo_root()` claim and it
was not done."* Picking work whose evidence does not depend on a device I do not have
is deliberate; §28 is the standing example of the alternative.

### 29.1 Five anchors, and the layout decision that makes the fallback free

§23.2 found four independent compile-time anchors; there are now **five**, because §24
added `find_devshell_apk` on purpose and routed it correctly as *"the pattern to copy
rather than an exception to it"*. All five now go through `payload::root()`:

| site | what it resolves | was |
|---|---|---|
| `build_cmd.rs` `repo_root()` | both shells' artifacts, the Python prefixes, `zkite_py`, `webruntime`, the icon | its own `concat!` |
| `build_cmd.rs` `pybuild_script()` | `compile_pyc.py`, `prune_stdlib.py` | a *second*, independent `concat!` |
| `wheels.rs` `vendored_pip_pyz()` | the vendored `pip.pyz` | a third — and the quiet one |
| `doctor_cmd.rs` `check_shell_artifacts()` | the artifact check | a fourth |
| `dev_cmd.rs` `find_devshell_apk()` | the Dev Shell APK | env var, then a fifth |

`repo_root()` is **deleted rather than kept as a wrapper.** A thin alias would leave
two names for one thing, which is how four of these accumulated in the first place;
`run()` binds the resolved root to a local called `repo_root` because that is what the
~19 call sites below it mean by the word.

**The decision the design rests on: a payload root has the checkout's own relative
layout.** `phase_0/vendor/…`, `phase_1/zkite_py/src`, `phase_3/gpu_harness/artifacts`.
That is not laziness. It makes the compile-time checkout **a valid payload root by
construction**, so the dev-mode fallback needs no special case anywhere and cannot
drift from the shipped layout — and every existing `root.join("phase_1/…")` call site
is untouched, which is what keeps the diff small enough to reason about. A host test
asserts this checkout satisfies the same probe a packaged payload does, so moving
`zkite_py`'s source fails here rather than on a stranger's machine.

**Search order** (§23.7 step 1, unchanged from what it asked for): `ZKITE_PAYLOAD_ROOT`
→ `zkite-payload/` beside the binary → the compile-time checkout, **last, forever**.

Three details that are decisions rather than code:

- **A payload is identified by a *probe*, not a stamp file.** Two load-bearing files
  must be present (`zkite_py`'s `__init__.py`, `prune_stdlib.py`). Both are inputs to
  every single build, so a directory that has them is a payload and one that does not
  would have failed later anyway — 80 lines and one wheel resolution later, which is
  the "validate beside the cheap checks" posture §22.1 already argues for. A stamp file
  is the thing someone forgets to write, which is §18.7's `icons.rs` gap exactly.
- **A bad `ZKITE_PAYLOAD_ROOT` is a hard error, not a fall-through.** Someone who sets
  it means it. Falling back to a checkout that happens to exist on this machine is
  precisely the failure mode §23.2 called *"the quiet one — it degrades rather than
  fails"* about `wheels.rs`, and reproducing it inside the resolver written to fix it
  would be absurd.
- **Both beside-the-binary candidates are Python-version-independent** (`<exe_dir>/
  zkite-payload` and `<exe_dir>/../zkite-payload`, the latter being a wheel's
  `.data/data/` scheme). A `site-packages` path is *not* version-independent, and the
  distribution unit is still an open owner decision (§23.8) — so committing to that
  layout now would be guessing at a shape nobody has chosen. See §29.7.

### 29.2 `zkite doctor` reports the root — and that immediately found a bug

§23.7 step 1's fight is not the code: *"`zkite doctor` must report which root it used,
or the first person to debug a wheel gets a path that does not exist and no clue why."*
It now does, as the **first** check, before everything that resolves against it, and
says out loud when the answer is the dev-mode fallback — because a wheel silently
reading a developer's checkout is the one failure mode this whole resolver introduces,
and it would otherwise look completely healthy.

Running that against a staged payload found something no amount of reading would have:

**`zkite doctor` exits 1 on §23.4's own recommended default wheel.** The fatal/
informational split between the two backends was hardcoded — WebView fatal, native
informational — under a comment reading *"the native renderer is not the default"*.
True when written, false since §15.4, and the consequence was never noticed because on
this checkout both backends' artifacts always exist. §23.4 recommends a **native-only,
arm64-only** default wheel; against exactly that payload, `doctor` reported `FAIL` for
the missing WebView shell and exited non-zero on a perfectly working machine. The
diagnostic a stuck user runs would have told them their working install was broken.

Fixed by keying the split off `Backend::DEFAULT` rather than a literal, so a future
flip cannot desynchronise it again, and by reporting both backends either way — a
missing *non-default* backend now prints `--` plus the concrete consequence
(`--backend webview` will not work here).

This is the same shape as §23.2's own finding about this command, one layer up: a
diagnostic that is itself wrong is worse than no diagnostic.

### 29.3 `version_code`: per-ABI, and why not Google's own scheme

§17.4 item 2, which the owner promoted above the release pipeline last session on the
grounds that it depends on none of the packaging work and *"fails at Play upload rather
than at build"*. That reading is right:

`build_cmd.rs` built the manifest **once**, outside the per-ABI loop, so
`app-arm64-v8a.apk` and `app-x86_64.apk` carried a byte-identical `versionCode`. Play's
multiple-APK support requires each APK in a release to have a distinct one.

`manifest_gen::abi_version_code(base, abi)` is `base * 10 + rank`, with the manifest
now built **inside** the loop. Two things in that are decisions:

**The ranks are permanent, and the 32-bit ones are listed although ZKite ships neither.**
Play's rule is that when two APKs can both serve a device, the device gets the higher
code — and ABI support *overlaps*, since a 64-bit ARM device also runs `armeabi-v7a`.
So `arm64-v8a` (3) must outrank `armeabi-v7a` (1). Listing all four now is the
load-bearing part: adding an ABI later must not **renumber** an existing one, because a
version code can never go down for an already-published APK. Getting that wrong is
unfixable rather than inconvenient.

**`base * 10 + rank` is deliberately not the `rank * 1000 + base` that Google's own
docs show.** Theirs satisfies "distinct within a release" and is *not monotonic in the
base*: arm64 at base 2 is 3002, which is **lower** than x86_64 at base 1's 4001, so
bumping the version does not raise every ABI above every code of the previous release.
`base * 10 + rank` does, so "I bumped my version" means the same thing on every ABI.
The cost is a ceiling of 9 ABIs, which is five more than Android has. A build-time
assert catches the Play ceiling (2,100,000,000) — the same fail-loudly-at-build posture
`assert_native_libs_are_sane` takes.

**`zkite build aab` deliberately still uses the base unmodified**, since one artifact has
no collision to fix and Play stamps the splits it generates itself. That leaves a real
edge: an app that shipped per-ABI APKs and then switches to an AAB would upload a
*lower* code and be rejected. `zkite build aab` now says so, with the number, rather than
leaving it to Play's error message. §29.7.

**Still open, and not attempted:** §17.4 item 1 — `version_code` still defaults to 1 and
never auto-increments, and §17.4's recommendation to *derive* it from `[project] version`
so the two cannot drift is unbuilt. That is a separate behaviour change with its own
"what does `0.1.0` become" question, and bundling it into a collision fix would have made
neither reviewable.

### 29.4 Verification

**Host suites.** `cli` 26 → **34**, `bundler` 18 → **24**. `zkite_py` 124,
`zkite_renderer` 44, `twcore` 57 unchanged and green. `zkite_pybridge` — see §29.5.

**Negative controls, because a passing test proves nothing until it has been shown to
fail** (phase 2 §6/M1, §12.5.3, §26.5, §27.1, §28.4):

| control | result |
|---|---|
| `abi_version_code` returns `base` unchanged (the pre-fix behaviour) | **4 of 6 version-code tests fail** |
| the discriminator below, run against the checkout instead of the payload | sentinel **absent** — so it discriminates |
| a `ZKITE_PAYLOAD_ROOT` that is not a payload | exits **1**, naming both markers |
| a half-staged payload (one marker of two) | refused; falls through to the checkout |

And **one deliberate non-result**, in the spirit §27.1 and §28.4 record: the
monotonicity test `bumping_the_base_raises_every_abi_above_every_previous_code`
**passes against the pre-fix control too**, because when the code *is* the base,
base 2 trivially beats base 1. It is a good test of the scheme against Google's
alternative and it does not discriminate the bug. Kept, and named here.

**The off-checkout run, which is the actual claim.** `phase_1/cli/scripts/stage_payload.sh`
(new — and explicitly *not* §23.7 step 3's release pipeline; it is the thing such a
pipeline would call) stages the set `build_cmd.rs` reads into one directory. Then: the
release binary copied to `<tmp>/bin/zkite`, the payload to `<tmp>/bin/zkite-payload`, and
`zkite new` + `zkite build apk` run from `<tmp>`, with nothing pointing at this checkout.

Measured, per §23.3's own table: **55 MB uncompressed, 21 MB `zip -9`** for the default
configuration (native, `arm64-v8a`). §23.3 hand-staged 53 MB / 19 MB; the delta is that
this script ships the *unpruned* pure-Python stdlib, which is correct — pruning is
per-app reachability and happens at build time — so the figure is honestly a little
higher rather than §23.9's feared "low rather than high".

**How the result is made decisive rather than plausible.** A successful build off the
checkout is weak evidence on this machine, because the checkout exists and the fallback
would also have worked. So the staged payload carries a **sentinel file**
(`zkite/_payload_sentinel.py`) that exists nowhere in the checkout, and the APK is
unzipped afterwards — §15.6.1's "verify the artifact contains what you think you just
put in it", which has now been wrong for four distinct reasons:

```
assets/zkite-framework.zip -> zkite/_payload_sentinel.py     PRESENT   (payload build)
                                                           ABSENT    (checkout control)
```

Plus, from the same APK: 73 `lib/arm64-v8a/` entries, and `apksigner verify` reporting
v2 **and** v3 schemes `Verified`. And a second discriminator that fell out for free —
the payload build produced **one** APK (arm64-only payload) where the checkout control
produced **two**, so `resolve_abis` really is reading the payload.

**The per-ABI version codes, read back by Google's own tool** rather than from ZKite's
own log line:

```
$ aapt2 dump badging payloadtest-arm64-v8a.apk | head -1
package: name='com.example.payloadtest' versionCode='13' ...
$ aapt2 dump badging payloadtest-x86_64.apk | head -1
package: name='com.example.payloadtest' versionCode='14' ...
```

13 and 14 from base 1, where both were `1`. Android's own package manager agrees after
install: `versionCode=14 minSdk=26 targetSdk=34`, `primaryCpuAbi=x86_64`.

**Device — and it is an emulator again, deliberately.** A build that satisfies every
host-side tool is not evidence it runs (phase 2 §10's standing lesson). An `x86_64`
payload was staged the same way, an APK built *entirely* through the beside-the-binary
path, and installed on the `zkite_phase0` AVD (API 34, SwiftShader/Vulkan, density 2.75)
after `adb uninstall` + `pm clear`, per the two staleness traps:

```
py: zkite: 67 native extension modules available
GPU backend initialized (wgpu/vello_hybrid)
ZKITEPERF first-frame since_boot_call=895ms nodes=6 via=gpu
```

Three real taps took the counter **0 → 3**, `mCurrentFocus` was the app's own
`ZKiteActivity` before and after, `pidof` was constant, and there were **zero**
`FATAL EXCEPTION`/`E AndroidRuntime`/`PANICKED`/`ANR` markers across the session. So an
APK produced by a `zkite` binary that never read this checkout installs, boots CPython,
imports all 67 extension modules, renders on the GPU and responds to touch.

That is evidence, not proof, and the distinction is phase_0/README.md's own: it is a
different ABI and a different GPU implementation from every device number in this
document. What it *does* cover completely is the part that has nothing to do with the
phone — the resolver, the staging set and the version codes are host-side facts, and
they are checked on the host.

### 29.5 A pre-existing intermittent failure in `zkite_pybridge`, found and not fixed

`zkite_pybridge` is documented at 8 tests. It is 8 tests **most of the time**:
`a_touch_on_a_synthesized_thumb_resolves_to_the_control_itself` (§26.5's own test)
fails intermittently with `a descendant of the Switch must resolve to it; resolved = []`.

This is **not** caused by this session's work, and that was checked rather than argued:
`git stash`ed to a clean tree at `5c7a857` and re-run, **2 failures in 15**. Nothing here
touches `pybridge`, `zkite_renderer` or `twcore`.

Narrowed, because "flaky" is not a diagnosis:

| how it was run | failures |
|---|---|
| the full suite, default (multi-threaded) | **2 / 10**, and 2 / 15 on a clean tree |
| that one test alone, by exact filter | 0 / 12 |
| that test plus the only other one that loads an app through CPython | 0 / 12 |
| the full suite with `--test-threads=1` | **0 / 8** |

So it is a **concurrency** interaction inside the test binary, not test *ordering* and
not the test itself. The four tests that go through `embed::setup`/`dispatch` all take
`lock_python()` and hold it for their whole body; the other four never touch CPython at
all (hand-built `RawNode` trees). That means the shared state being raced on is *not*
the interpreter lock those four already serialise — which is what makes this different
from §17.1.1, whose fix ("0 failures in 15 runs") is real and does not cover this.

**Recorded, not fixed, deliberately.** The remaining candidates are shared globals in
the renderer/font stack reached from `test_renderer()`, and finding it means an
investigation in a crate this session did not otherwise touch. Bundling a change to the
shared setup/render path into a commit about distribution paths would give it none of
the verification it deserves — including a device run, which is not available. Two
consequences worth having written down:

- **`zkite_pybridge`'s documented count of 8 is only reliable under
  `--test-threads=1`.** Anyone reading a `7 passed; 1 failed` should not assume they
  broke it.
- It is a second instance of §17.1.1's general lesson from a new direction: a passing
  test is not evidence of isolation, and this one was found by running the suite
  concurrently with another for unrelated reasons.

### 29.6 Owed — and then paid, mid-session

For most of this session the phone was not attached (`adb devices` reported only
`emulator-5554`, the `zkite_phase0` AVD), and this section was written to say so plainly:
§28's device run had not happened for a second session running, and nothing in §29
substituted for it. **The owner then connected the Moto G54 over wireless debugging and
it was done — see §29.8.** The paragraph is kept rather than deleted because the shape of
the session is the point: the work was *chosen* on the assumption that no device would be
available, which is why it was the item that did not need one.

The run closed §28.6's top item and found a real divergence the emulator had not
(`shadow-2xl`, §29.8) — which is phase_0/README.md's "an emulator pass is evidence, not
proof" landing on a concrete example rather than staying a principle.

### 29.7 Known gaps

- **Nothing was run on Windows, macOS, or a second physical Linux machine.** §23.9's
  first two gaps are *narrowed, not closed*: `zkite build` is now verified to work from
  an installed binary reading a payload outside the checkout, on **this** machine. The
  remaining machine-specific claims in §23.5 — `which("python3.14")`, the `py -3.14`
  launcher, `\r\r\n` in `adb shell` output — are still read from the source and unrun.
- **No wheel was built.** The payload is a staged directory, not `python -m build`
  output; real wheel metadata and `RECORD` add a little. Same gap §23.9 records,
  unchanged, and it is downstream of the `zkite` vs `zkite-cli` decision.
- **The `site-packages` beside-the-binary candidate is not implemented.** The two that
  are, are Python-version-independent on purpose. A data wheel installing into
  `site-packages` needs a version glob, and which layout is right depends on §23.8's
  open distribution-unit decision. This is the slot for it, not an oversight.
- **`stage_payload.sh` is not a release pipeline** (§23.7 step 3), which is still where
  the actual weeks are and still does not exist. The script also derives its staged set
  by mirroring `build_cmd.rs` by hand, so if a staging site is added and not mirrored
  here, an off-checkout build breaks and no test catches it — the same class as §18.7's
  ungated `gen_icons.py`. A check belongs in the CI that does not exist (§9.3).
- **`zkite dev` was not re-run off the checkout.** `find_devshell_apk` now routes through
  the resolver and `stage_payload.sh` has an `INCLUDE_DEVSHELL=1` path, but only
  `zkite build` was exercised from a staged payload. §24.5's "only the native backend
  was exercised on device" is unchanged too.
- **The AAB path was not built at all this session.** `zkite build aab`'s new
  version-code warning and ceiling assert are covered by host tests and by reading;
  no `.aab` was produced, so §14.6's "`zkite build aab --backend native` is untested"
  stands and now has a second reason.
- **The APK→AAB version-code downgrade is warned about, not solved** (§29.3). A user
  who ships per-ABI APKs and then switches to an AAB must bump `version_code` past the
  highest APK code themselves. Making the two schemes agree would mean changing the AAB
  path, which has no bug in it.
- **§17.4 item 1 is untouched:** `version_code` still defaults to 1, never
  auto-increments, and is not derived from `[project] version`.
- **`zkite_pybridge`'s intermittent failure is diagnosed to a shape, not a cause**
  (§29.5), and left in the tree.
- **`zkite doctor`'s new payload check has no test.** `doctor_cmd.rs` had none before and
  has none now; `check_payload` reads a process-global resolver and `check_shell_artifacts`
  hits the filesystem. The resolver underneath it has 8 tests, which is where the logic
  actually is — but the fatal/informational split that §29.2 fixed is verified by having
  run it, not by a guard, so it can regress silently.
- **The 21 MB figure is `zip -9` over a staged tree**, as §23.3's was, and only for
  native/`arm64-v8a`. The other combinations are arithmetic, not measurements.

### 29.8 §28 verified on the Moto G54 — the owed run, and a divergence it found

**Status: done. §28.6's top open item is closed**, two sessions after it was opened. The
phone was connected over wireless debugging partway through this session; every
measurement below is on the real device, and every claim is arithmetic out of the PNG
rather than an eyeball, per §28's own standing lesson.

**Device: Moto G54 5G, arm64-v8a, Android 15, 1080x2400 @ 400dpi (density 2.5), 60Hz,
PowerVR BXM-8-256** — asserted from `getprop`/`wm size`/`wm density` rather than assumed,
and confirmed by the shell's own boot line (`density 2.5, 60Hz -> 16.67ms frame period`).
`wgpu adapter: PowerVR BXM-8-256 (IntegratedGpu, backend Vulkan)`, i.e. a real tiler on
real silicon, where §28 had SwiftShader. **Two devices were attached**, so every `adb`
call passed `-s` explicitly — §24.5 records that nothing in the CLI does this, and it is
exactly the situation that makes it bite.

The APK was **artifact-verified before installing** (§15.6.1): `assets/app.zip`'s
`main.py` is the §28 decoration demo, and the packed `.so` carries the decoration
symbols. Then `adb uninstall` → `install` → `pm clear`, per both staleness traps.
`first-frame since_boot_call=1461ms nodes=44 via=gpu`, 67 extension modules.

**The two measurements §28 called decisive, re-run at a different density:**

| | §28 (emulator, density 2.75) | **Moto G54 (density 2.5)** | v3 would have been |
|---|---|---|---|
| a bare `ring` | 3px = 1.09dp, `(200,0,222)` | **3px = 1.20dp, `(200,0,222)`** | 3dp = 8px of blue-500 `(59,130,246)` |
| `border-t-8 border-b-4 border-l-2` | 8.00 / 4.00dp, no right band | **20px/10px/5px = 8.00 / 4.00 / 2.00dp, no right band** | — |

`(200,0,222)` is v4's `fuchsia-600` to a distance of **0.0** — the ring picking up the
row's inherited `text-fuchsia-600`, which is the whole `currentColor` correction.

**Everything else, measured:**

- **`currentColor` borders track the inherited text colour exactly.** Three swatches with
  a width and no colour class: `(0,153,102)` `(236,0,63)` `(15,23,43)` — emerald-600,
  rose-600, slate-900, each at distance **0.0** from v4's own value. v3's default was a
  flat gray-200 for all three.
- **Per-side colours mitre correctly.** `border-4 border-sky-500 border-t-amber-400`:
  top `(255,185,0)` amber-400, left/right/bottom `(0,166,244)` sky-500, all distance
  **0.0**. Colour does not bleed across the corner diagonal.
- **`border-8` is 20px = 8.00dp**, `rounded-full` is a true circle (dark span 66 → 114 →
  **130** → 112 → 66 px down the swatch, symmetric about the middle), so the radius clamp
  §28.2 added holds on hardware.
- **Paint order, scanning inward from the left of row 6** (`bg-white border-4
  border-slate-900 rounded-xl shadow-lg ring-2 ring-indigo-400`): page → *shadow* darkening
  the page outside the box → `(124,134,255)` indigo-400 for **5px = 2.00dp** (the ring,
  outside the border box) → `(15,23,43)` slate-900 for **10px = 4.00dp** (the border,
  inside) → white fill. Shadow under, ring outside, border over. That is the only thing
  row 6 checks and it is right.
- **Controls.** Zero `FATAL EXCEPTION`/`E AndroidRuntime`/`PANICKED`/`ANR` markers across
  the session; `pidof` constant throughout **including across a `uiautomator dump`**, so
  §8.7's accessibility-abort guard still holds and the dump reports every label;
  `mCurrentFocus` asserted as the app's own `ZKiteActivity` before and after. And an
  **idle control** per §27's rule — a second screenshot after 8s of no input is
  **pixel-identical across the entire app content region** (0 differing scanlines,
  y=96..1619), so nothing here is a stray touch.

#### The finding: `shadow-2xl` is measurably wrong, and the harness reports 0.00%

Row 4 puts all six shadow sizes on cards that are `bg-slate-100` **on a `bg-slate-100`
page** — the card is invisible and the shadow is the only thing that renders it. All six
are present and their vertical reach is ordered.

Their *mass* (summed darkness in a 45px window below the card edge) is **not** monotonic:
`2xl` comes out below `xl`. Rather than call that a bug, the source of truth was asked —
real Chromium driving the very `@tailwindcss/browser` bundle `phase_1/webruntime/vendor/`
pins, at `deviceScaleFactor: 2.5`, measured with the identical metric and an identical
45px window anchored on each card's own bottom edge (alignment confirmed: the first
shadow row is +1 for all six on both).

| | xs | sm | md | lg | xl | 2xl |
|---|---|---|---|---|---|---|
| Chromium mass | 37 | 105 | 252 | 527 | 876 | **486** |
| native mass | 33 | 87 | 227 | 500 | 873 | **682** |
| Chromium peak | 10 | 29 | 32 | 32 | 36 | **12** |
| native peak | 11 | 29 | 33 | 34 | 40 | **21** |

**Chromium is non-monotonic in exactly the same place, and the two agree on the ordering
at every one of the five steps.** So the dip is real CSS, and the reason is visible in
the computed values Chromium reports: `shadow-xl` is **two** stacked `/0.1` layers while
`shadow-2xl` is a **single** `/0.25` layer with a 50px blur and a −12px spread. Five of
six sizes then agree closely (peaks within 1–4 levels, masses within ~12%).

**`2xl` does not.** Native is ~1.75x too dark at the card edge (21 vs 12) and 40% heavy
by mass. Its *extent* is right — the horizontal profile matches Chromium's at both ends
— but the distribution differs: across the card's width Chromium is nearly flat
(7 → 11 → 7) where native is peaked (7 → 19 → 6). `2xl` is the **only** entry in the
scale whose blur sigma (50dp blur → 62.5px sigma at this density) exceeds its own
spread-shrunk rect (70px), which is the extreme of `fill_blurred_rounded_rect`'s
analytic approximation. Not chased further, and not fixed: it is one number in a vello
primitive's input, it wants its own session, and changing blur maths inside a session
about distribution paths would give it none of the verification it needs.

**The part that matters beyond this one class.** The largest per-channel delta here is
**9 levels**, and `CHANNEL_TOLERANCE` is **40** — so `shadow` scores **0.00% loose** with
this divergence fully present. §28.4 predicted precisely this in the abstract ("nothing
in the harness sees softness", found via a control that did not discriminate); §29.8 is
the first *concrete* instance, found on hardware, of a real shadow bug the harness is
constitutionally unable to report. It is also the answer to §28.4's open question of
whether the darkest entry in the scale might discriminate: it is the entry that is
**wrong**, and it still does not.

#### What this run does not close

- **The CPU fallback was not forced on** (§28.6, unchanged). This is the GPU path only.
- **One device, one ABI, one density.** With §28's emulator run there are now two
  densities (2.5 and 2.75) and two GPU implementations, which is materially better than
  either alone, and still not a matrix.
- **`shadow-2xl` is left wrong**, now with a number on it (§29.7).
- Inset/coloured shadows and `ring-offset-*` remain unparsed (§28.5).

## 30. `Checkbox` and `Radio` — the form vocabulary, doubled

§15.1's "missing widgets", and plan1.md §14.3's standing sentence:

> Today the form vocabulary is `TextField`, `Switch`, `Slider` and nothing else.

**Status: `Checkbox` and `Radio` are built, and verified live on the Moto G54.** The
native backend's working form vocabulary goes from three controls to five. `Select` is
**not** built and that is deliberate — it is the one with a real design decision in it
(a native picker overlay versus a self-drawn popup) and it belongs to the owner, not to
an engineer who happens to be mid-session (§30.6).

**Why this, and why now.** The previous session's prompt ranked the form vocabulary
second of everything on the board — "the gap a new user hits in hour one" — and argued
that nothing technical blocks it, since §26 already built every piece of machinery it
needs. That held exactly: `Control`/`control_at` disambiguates a synthesized child back
to its control, `dispatch_change_to` exists, `_payload_parser` types the payload at the
widget so both backends agree by construction. The one thing that had to be *decided*
was how three visually different controls that are all `<input>` carrying `checked`
tell themselves apart (§30.1). It was picked over the rest of the queue this session
for a second reason: **the phone was finally attached**, and this is the queue whose
verification most needs one — §26.6 concedes the conformance harness "renders one state
and never touches anything", and three of the six device cases below are about what
happens on the *second* tap.

### 30.1 The decision: a role, not a marker

`Switch`, `Checkbox` and `Radio` are all an `input` element carrying a `checked`
attribute. `tag` cannot separate them and neither can `checked`, so something has to.
The tempting answer is a ZKite-private marker on the tree path, and §26.3 already argued
against exactly that shape — *"reading these from attrs rather than from a new
Python-side marker is what keeps `zkite_py` free of native-renderer knowledge."*

**So the discriminator is `role`, emitted on *both* paths**, because it is a fact about
the control rather than about the renderer:

- On the **HTML** path it is ARIA's own `role="switch"`, which is the correct role for a
  checkbox presented as a switch, and it is what lets `index.html`'s switch CSS be
  *specific* (§30.2).
- On the **tree** path `zkite_pybridge::a11y_role` reads it to choose between
  `Role::Switch`, `Role::CheckBox` and `Role::RadioButton` — a distinction a screen
  reader user relies on, since "switch" implies on/off, "checkbox" implies an
  independent choice and "radio button" implies one-of-many.

`Checkbox` and `Radio` are **subclasses of `Switch`**, which is the other half of the
same idea: everything except drawing and role genuinely is identical — same tag, same
`checked`, same `bool` payload, same controlled contract, same toggle dispatch — and
subclassing makes "they cannot diverge in the parts that are the same" structural
rather than something to remember. §26.2 is the cost of the alternative.

**Only `Radio` gets its own `Control` variant, and only because its interaction
differs.** A `Checkbox` maps to `Control::Switch`, since adding a variant that behaved
identically would be an invitation to drift. `Control::Radio` exists for one rule:

> **A tap on a radio always sends `true`, never `!checked`.**

A real `<input type=radio>` cannot be unchecked by clicking it, and a radio that turned
itself off would leave its group with nothing selected — a state the group's own
`@store` field usually cannot even represent. There is no `name`-style grouping and
there does not need to be: exclusivity is `checked=(s.choice == "a")` on each radio and
the re-render deselects the others, which is the same controlled contract §26.3
established for every other ZKite control.

### 30.2 A global CSS selector that was correct until it wasn't

`index.html`'s switch styling was a bare `input[type="checkbox"]` selector. That was
right while `Switch` was the only checkbox-shaped widget ZKite had, and it became wrong
the moment `Checkbox` existed: a checkbox would have silently inherited a 44x24 track
and a sliding thumb. It is now scoped to `input[type="checkbox"][role="switch"]`.

This is worth recording rather than fixing quietly, because it is a small instance of
the pattern this document keeps finding: **a rule whose correctness depended on a fact
about the widget set, with nothing tying the two together.** The same shape as §9.7's
`text-align` and §28.1's v3 defaults — right when written, silently wrong later, and
invisible until something new walked into it. The negative control measures it exactly
(§30.4): with the selector unscoped, `checkbox` scores **19.05%** against **0.55%**.

**Where the two backends genuinely do have two copies, stated plainly.** The *box* is
drawn by Tailwind classes on the element itself, so real Tailwind here and `twcore`
there render it from one declaration and there is no second hand-authored design. What
cannot cross over is the **mark inside it**: an `<input>` is a void element and cannot
contain the `Icon("check")` child the tree path nests, so the WebView draws its tick in
CSS. That is one deliberate seam, it is the §9.5/§9.7/§28.1 shape, and the mitigation is
that a *static checked state* is precisely what §9's harness can see — which is the whole
argument §28 made for preferring work a pixel diff can discriminate.

The tick itself is `Icon("check")` on the tree path rather than a hand-rolled path, so
it comes from the sprite table `gen_icons.py` transcribes out of `index.html`'s own
`<symbol>` sheet (§18.2). The native tick cannot drift from the WebView's by
construction; only the CSS approximation of it can, and that is what the fixture pins.

### 30.3 A payload bug avoided, in exactly §26.2's shape

`zkite-runtime.js`'s `extractPayload` special-cased `checkbox` and `range` and fell
through to `el.value` for anything else. A `<input type=radio>` with no `value`
attribute has the DOM default `"on"` — and `_as_bool("on")` is `False`, so **selecting a
radio would have reported it deselected.**

That is §26.2's bug with the sign flipped (`bool("false")` was `True`; here a truthy
selection reads false), in the same function, found by writing the test before trusting
the code. Fixed by reading `.checked` for `radio` as well as `checkbox`, and pinned in
the e2e suite against real Chromium — where the negative control produces literally
`r.1:change=on` (§30.4).

### 30.4 Verification

**Host suites.** `zkite_py` 124 → **130**, `zkite_pybridge` 8 → **10**, webruntime e2e
25 → **28**. `cli` 34, `bundler` 24, `zkite_renderer` 44, `twcore` 57 unchanged.

**Conformance: two new fixtures, 23 total, both dprs.** Both states in one canvas each,
because an *unchecked* box is where a border or fill regression shows — there is no mark
on top of it to hide behind.

| fixture | dpr 1 (loose / strict) | dpr 2 (loose / strict) |
|---|---|---|
| checkbox | 0.55 / 1.14 | 0.51 / 0.73 |
| radio | 0.04 / 0.73 | 0.14 / 0.86 |

**Every pre-existing fixture is unchanged to two decimal places at both dprs** against
§28.4's table — including `switch` (0.17/0.90 at dpr 1), which is the number that
actually proves scoping the CSS did not break the widget the CSS was written for.

**Negative controls** (phase 2 §6/M1, §12.5.3, §26.5, §27.1, §28.4):

| control | result |
|---|---|
| the pre-§30 unscoped `input[type=checkbox]` selector | `checkbox` 0.55 → **19.05%** (dpr 1), 0.51 → **19.04%** (dpr 2) |
| `extractPayload` without the `radio` branch | e2e fails, payload is literally **`r.1:change=on`** |
| `a11y_role` returning `Role::Switch` for anything `checked` | both new role assertions fail |

And **two deliberate non-results**, kept per §27.1/§28.4's precedent: the unscoped-CSS
control moves neither `radio` (which never matched `input[type=checkbox]`) nor `switch`
(which is what the rule was for). Both are correct non-discriminations, and the second
is the more useful one — it is the evidence that the fix was surgical.

**Device — Moto G54 5G (arm64-v8a, Android 15, density 2.5, PowerVR BXM-8-256), GPU
path.** `demo_app/toggles.py`, written against the documented signatures only per
§26.7. The artifact was checked on all three surfaces before installing (§15.6.1) —
`app.zip`'s `main.py`, **`widgets.py` inside `zkite-framework.zip`** (which is where
these two actually ship — only `runtime.py` is `include_str!`'d into the `.so`), and the
`.so` itself — then `adb uninstall` + `pm clear`. Two devices were attached, so every
`adb` call passed `-s`.

**Android's own accessibility layer reports the new roles**, which is the a11y half end
to end through AccessKit: `uiautomator dump` finds **3 `android.widget.CheckBox` and 3
`android.widget.RadioButton`** with correct `checked` values — not `Switch`, which is
what every one of them would have been before this session.

Six cases, each run **in isolation from a clean state** with an idle control first,
per §27's rule about not batching gesture measurements:

| case | result |
|---|---|
| idle control, 6s, no input | every control unchanged |
| tap a `Checkbox` | `false -> true`, **`last payload type: bool`** on screen |
| tap it again | **`true -> false`** — the direction §26.2's payload bug broke |
| tap an unselected `Radio` | selects it **and clears the previously selected one** |
| tap the *selected* `Radio` again | logs `true -> true`; **it does not clear** |
| tap a `Checkbox` inside a tappable `Row` | **both** fire — `toggling switch r.10.0` *and* `dispatching tap to r.10`, row counter 0 → 1 |
| tap the row but not the box | only the row's handler; the box is unchanged |

The fourth and fifth rows are the point of doing this on a device at all: they are about
the *second* tap, and no static fixture can ask that question.

Zero `FATAL EXCEPTION`/`E AndroidRuntime`/`PANICKED`/`ANR` markers; `pidof` constant
throughout including across a `uiautomator dump` (§8.7's guard holds); `mCurrentFocus`
asserted as the app's own `ZKiteActivity` at every capture.

**One trap re-confirmed the hard way.** The first attempt to read state used
`uiautomator dump` and reported *nothing had changed* while the log clearly showed the
dispatch. That is §8.7's documented staleness — `push_a11y_update` skips while
accessibility is disabled, so the dump's `checked` values were the launch state. The
session prompt names this exactly ("screenshots are ground truth for app state; the a11y
tree is ground truth only for roles and bounds"), and every state assertion above is
therefore measured off a **screenshot** — checkbox fill sampled at a known pixel, not
eyeballed and not read from the dump. The role/bounds claims above still come from the
dump, which is what it *is* ground truth for, and were taken right after one.

### 30.5 What this does not do

- **`Select`/picker is not built** (§30.6). It is the remaining form control and the
  one with a genuine design decision in it.
- **No `DatePicker`/`TimePicker`, `Dialog`, `Toast`, `ProgressIndicator`** — §15.1's
  list is shortened, not closed.
- **No indeterminate checkbox state.** `checked` is a `bool`; ARIA's `mixed` is not
  modelled on either backend.
- **No keyboard/focus interaction.** Neither control is reachable or togglable by a
  hardware keyboard or a D-pad on either backend; that is the same gap every ZKite
  control has and it is not made worse here.
- **No `disabled`.** `Button` has it; these do not, and nor does `Switch`.

### 30.6 `Select` — the decision it needs, written down rather than guessed

Named here so the next session has the question rather than rediscovering it. Two
credible designs, and they differ on more than effort:

1. **A self-drawn popup.** A `NavigationHost`-style overlay layer painted by the
   renderer, the same substrate §16.1 built for `Nav` and the one `Sheet` would use. It
   looks identical on every device, is testable in the conformance harness (a static
   open state is a rendered state), and needs no new platform integration — but it is
   the first thing in this framework that has to paint *above* the normal tree and
   dismiss on an outside tap, and it needs the animation driver (Queue F) to not feel
   cheap.
2. **A real Android picker via `Native(...)`.** Correct platform feel and correct
   accessibility for free, but §15.1 lists `Native(...)` itself as unbuilt, and
   embedding a real `View` above a `SurfaceView` is a platform-integration project
   rather than a widget one.

**A recommendation, since one was asked for elsewhere in this document's own style:**
build the self-drawn popup, because it composes with what exists (`Nav`'s layering,
`Sheet`'s eventual needs, the conformance harness) rather than opening a new front, and
because `Native(...)` should be picked deliberately as a platform project rather than
reached through a dropdown. But it is genuinely the owner's call, and it also decides
whether `Sheet` and `Select` are one piece of work or two.

### 30.7 Known gaps

- **The WebView backend was not run on a device for these two.** The rendering is
  covered by the conformance harness — which *is* real Chromium, i.e. the same engine
  the WebView runs — and the payloads by the e2e suite in that same Chromium, but
  "expected to behave identically" is a different claim from having watched it, which
  is what §25.3 and §26.7 both had to say about the last shared-`zkite_py` change. The
  native backend is the default and is where the device time went.
- **The CSS tick is a second drawing of the shared sprite** (§30.2). The fixture bounds
  the drift and does not remove it. A mask-image generated from the same sprite source
  would, and was not attempted.
- **`aria_role` is a class attribute, so a third-party `Widget` subclass can set it to
  anything.** An unknown value falls through to `Role::Switch` and `Control::Switch`,
  which is the behaviour every pre-§30 tree already had; nothing validates it.
- **Radio exclusivity is entirely the app's.** Nothing warns if two radios in a group
  are both `checked=True` — both render filled, which is at least visibly wrong rather
  than silently wrong, but a `RadioGroup` container that made it structural does not
  exist.
- **No hit-target padding.** Both controls are 20x20dp, which is under the ~48dp
  Material/WCAG touch-target guidance. Every device tap above landed, but they were
  synthesized at exact coordinates; a real thumb is less precise. The fix is a padded
  tappable wrapper, and it interacts with the `Row(on_tap=...)` bubbling in §30.4's
  fifth case, so it is a design question rather than a constant.
- **Only Gboard/`adb`-synthesized touches**, one device, one ABI, the GPU path — the
  standing limits of every device run in this document.
- **`zkite_pybridge`'s intermittent failure (§29.5) is unchanged**, and both new tests
  there were run under `--test-threads=1`.

### 30.8 Tapping the label — and the first case where §27.6's divergence costs a user

Reported by the owner within minutes of §30 landing, which is the useful part: it is the
first thing someone writing a real settings screen tries. In `[x] Wi-Fi: ON`, tapping
the **text** does nothing — only the 20x20dp box is live.

**There is a way today, and it is one line:**

```python
# Works on both backends. One handler, wherever you tap.
Row("items-center gap-3", on_tap=lambda: s.update(wifi=not s.wifi),
    Checkbox("", checked=s.wifi),          # <- deliberately NO on_change
    Text("text-sm text-white", "Wi-Fi"))
```

§30.4's device cases 5 and 6 are the measured basis: a tap anywhere in a tappable `Row`
— **including on the box itself** — reaches the row's handler, and the control's own
`change` fires *in addition* only when one is registered. Leaving `on_change` off makes
the `Checkbox` purely presentational, so exactly one handler runs from anywhere in the
row. (`control_of` still classifies it, so the render thread still dispatches a
`"change"`; with no handler registered it is a no-op on both sides.)

**The trap, and it is §27.6 with teeth.** The *obvious* way to write it — `on_change` on
the control **and** an `on_tap` on the row writing the same field — makes a tap on the
box run both handlers, in an order that differs by backend. Driven through the real
`zkite_py` handlers in each backend's own measured order (native's from §30.4's device
log, WebView's from §27.6's Chromium e2e):

| tap target | native (`change` then `tap`) | WebView (`tap` then `change`) |
|---|---|---|
| the label | `True` | `True` |
| **the box** | **`False`** | **`True`** |

Same user code, same gesture, opposite result. On native the box tap looks like it does
nothing; on the WebView it works.

**This is the first case where §15.1 divergence 4b stops being theoretical.** §27.6
recorded the ordering as a real difference that "only bites when both write the same
`@store` field" and left choosing the contract as an open owner decision. This is that
case, it is reachable from the most natural way to write an extremely ordinary UI, and
the failure is silent on the default backend. **It raises the priority of that decision
rather than adding a new gap.**

**The real fix is a feature, not a workaround:** let `Checkbox`/`Radio` take a label as
content — `Checkbox("", "Wi-Fi", checked=..., on_change=...)` — so one node owns box and
text and there is no second handler to order. On the WebView side that is a `<label>`
wrapper, which is what HTML has always used for exactly this. It would also fix the
20x20dp hit target §30.7 flags as under Android's ~48dp guidance. Not built: it changes
these two widgets' signatures, and §30.6's `Select` decision may want the same
label-as-content shape, so the two are better designed together than one after the other.

**Not verified on a device.** The two dispatch orders are each measured (§30.4, §27.6);
the *composition* above was driven through real handlers on the host, because the
phone's wireless-debugging port rotated mid-session and refused to re-pair. A device run
would add nothing to the ordering question — it is `zkite_py` arithmetic once the order
is known — but it has not been watched end to end and that is a different claim.

## 31. The `zkite dev` error path — and the painting bug behind one of its four

§15.3 recorded four bugs the owner hit in one real `zkite dev` session on 2026-08-09,
root-caused and none fixed. **All four are fixed here.** Running the first one to
ground found two things §15.3 could not have known, and both are bigger than the entry
that led to them:

- **The native paint path drew every glyph in the tree above every background in it**
  (§31.1). That is why an opaque error overlay could not hide the "waiting for
  `zkite dev`…" screen behind it — and it independently broke `Stack`, a shipped widget
  whose entire purpose is to overlay children.
- **A `Stack` child with no explicit width did not fill the stack** (§31.2), the third
  instance of a divergence `ScrollView` already documents.

Neither was visible to the conformance harness, and §31.7 is about why: the `stack`
fixture overlays two opaque rectangles with explicit sizes and **no text in either**,
which is precisely the one shape that can see neither defect. It has scored ~0.04%
since phase 3 began.

The session also produced a finding about its own fix (§31.6): the staleness handshake
worked when tested by hand and would have been **dead in every real run**, because the
Dev Shell's socket connects before CPython finishes booting. That was found by
capturing the frame off the wire, not by reading the code.

### 31.1 Text painted above every background, everywhere

`paint_nodes_into_scene` (GPU), its `vello_cpu` twin, and `conformance`'s `paint` all
ran **three whole-tree passes**: every node's box, then every node's icon, then every
node's glyphs. Each carried a comment claiming this reproduced *"the paint order the
WebView backend gets from ordinary DOM stacking."*

It does — for a node against **its own ancestors**. Across sibling subtrees it inverts.
A background belonging to a later sibling is painted in pass 1; an earlier sibling's
text is painted in pass 3; so the text wins, always, regardless of tree order. Nothing
opaque could ever cover anything textual.

**Measured before it was called a bug.** A two-child `Stack`, both children explicitly
sized so no layout question is in play, one with text under an opaque card:

| | loose | strict | max channel delta |
|---|---|---|---|
| three passes (pre-§31) | **2.77%** | 3.39% | **255** |
| per node, in tree order | **1.06%** | 1.65% | 176 |

The residual 1.06% is ordinary glyph antialiasing, the same class as `text`'s 5.00%.
A `max_channel_delta` of 255 is the signature: white glyphs on emerald where Chromium
has flat emerald.

**The fix is one loop instead of three.** Per node: shadows (reversed), ring,
background, border, icon, glyphs — then the next node. The *within-node* sequence is
untouched, so §28.3's ordering contract and §18's icon-between-the-two both still hold;
what changes is only that the next node starts after this one finishes, which is what
DOM paint order is. One clip push/pop per node now covers everything it draws, rather
than one per pass.

**All 23 pre-existing fixtures are unchanged at both dprs — by mismatched *pixel
count*, not merely to two decimals.** That is the evidence the change is behaviour-
preserving for everything previously tested, and it is a stronger statement than the
usual "unchanged to two decimals" because the comparison was made on the raw counts.

This does not make the renderer's stacking model correct in general. There are no
stacking contexts, no `z-index`, no opacity groups; what exists now is document order,
which is what the DOM does in the absence of all three. §31.8.

### 31.2 A `Stack` child that did not fill the stack

Found in the same repro, and initially tangled with §31.1 — isolated by giving both
children explicit sizes, which moved the number from 50.43% to 2.77% and left the
paint-order half behind.

`Stack` overlays children by wrapping each in a positioned div. On the **HTML** path
that wrapper is `col-start-1 row-start-1` — a *grid item*, i.e. `display:block`, whose
child fills its width and takes its content height. On the **tree** path the wrapper was
`absolute inset-0` and carried **no display class at all**, so taffy applied its own
default of `flex`/`row` and sized the child to its *content* width.

So `Stack("...", Column("bg-indigo-600 p-3", Text(...)))` spanned the stack on the
WebView and was a narrow content-width strip on native.

`ScrollView`'s own implied classes already carry `flex flex-col` with a comment
explaining this exact mechanism — a plain `<div>` is `display:block` in a browser,
`twcore`/`zkite_renderer` has no block mode, and taffy's default is `row`. That comment
was written after a 30-row list rendered as 30 narrow vertical strips on a real device.
**This is the third instance of the same divergence**, and the fix is the same one class:
`absolute inset-0 flex flex-col`. Cross-axis stretch gives the full width, main-axis
start gives the content height — which is block layout for one child.

Measured, on the fixture that isolates it: **55.19% → 2.08%** (loose), and on device the
width-less child now spans **825 px = 300.00 dp** against a `w-[300px]` stack, exact.

### 31.3 The overlay now wraps (§15.3 bug 4)

`format_error_tree` put the whole traceback in a single `#text` child. Native text
*does* wrap — parley's `break_all_lines` gets a `max_advance` whenever the box has a
definite width, which `inset-0` gives it — but it wraps a single shaped run at word
boundaries, and a traceback's own newlines are not boundaries the shaper honours. So the
blob shaped as one continuous line and ran off the right edge. Its HTML twin sets
`white-space:pre-wrap` and always read fine: **two hand-maintained renderings of one
overlay and only one of them wrapped**, the same shape as §9.5's palette, §9.7's fonts
and §28.1's defaults.

Now one `#text` child per source line, in a `flex flex-col` box. Measured on device at
density 2.75: the content's x-extent is **43..1018 of 0..1079**, i.e. a 61px right
margin against a 44px (`p-4` = 16dp) padding — nothing reaches the edge.

### 31.4 Glue-level errors reach the terminal (§15.3 bug 2)

`_zkite_last_error()` returned `_app.last_error`, but an `ImportError` in the user's
`main.py` raises inside `load_module`, *before* `_app` is assigned. `_app` was `None`,
so the function returned `None`, `reload.rs` wrote no `OutFrame::Error`, and `zkite dev`'s
terminal stayed silent while the phone displayed the traceback. plan1.md §6 promises the
traceback appears "simultaneously in the terminal"; for the entire class of errors that
stop an app booting, it did not.

`runtime.py` now keeps a module-level `_glue_error`, recorded by every site that renders
a glue overlay and cleared by every site that renders a successful frame. The clearing
is wrapped as `_ok(frame)` around the *successful returns* rather than done at each entry
point's top, deliberately: a `_zkite_dispatch*` that returns `None` for "nothing handled
it" has not fixed anything and must not clear an error the previous frame reported.

Verified on device, which is the only place the whole chain exists — the traceback for
`from zkite import ... NoSuchWidget` now arrives in `zkite dev`'s terminal, through
`_zkite_last_error` → `ReloadReport` → `OutFrame::Error` → the ack thread.

### 31.5 The waiting screen stops claiming a disconnect (§15.3 bug 3)

`_glue_error_tree` composed `[_WAITING_TREE, format_error_tree(...)]` unconditionally.
Right before a connection existed; actively misleading afterwards — the owner read it as
"the dev server has dropped" while it was connected and serving.

`runtime.py` now sets `_dev_connected` when a reload arrives. Only the reload socket
calls `_zkite_reload`/`_zkite_reload_tree`, so that flag is exactly "a dev server has
talked to us", which is the question the placeholder answers.

**Worth being precise about what each half fixes**, because §31.1 alone would have hidden
this rather than fixed it. After §31.1 the opaque overlay covers the placeholder, so the
*screen* is right either way. But the tree is also what AccessKit walks and what
`uiautomator dump` reports, so a screen reader would still have announced "waiting for
`zkite dev`…" behind a live error. Composing it conditionally is what makes the tree say
what it means; §31.1 is what makes the pixels agree with it.

Measured on device: **0 pure-white pixels and 0 slate-950 pixels** in the app content
region of the error screen — the placeholder's own title colour and background, both
entirely absent.

### 31.6 A stale Dev Shell now says so — and the handshake's own bug

§15.3 bug 1. The owner's app failed with `cannot import Checkbox from zkite` the day
after §30 shipped `Checkbox`: `zkite dev` pushes only the *project's* `.py`, while `zkite/`
lives in the shell's `assets/zkite-framework.zip`, so **the installed shell pins the
framework version**.

**§15.3's own proposed fix would not have worked, and that is why this is a content hash.**
It said to put "the shell's `zkite_py` version" in the `hello` frame.
`phase_1/zkite_py/pyproject.toml` reads `version = "0.1.0.dev0"` and has not moved across
any commit in this repo, and there is no `__version__` in `src/zkite/` at all — so the
stale shell and the fresh payload would have reported the *same* version and the
handshake would have stayed silent through the exact failure it was written for. What
distinguishes a stale shell is its framework **contents**.

The digest is specified once and implemented twice:

> for each `*.py` directly in `zkite/`, sorted by name: `name` · `NUL` · file bytes ·
> `NUL`; sha256; first 12 hex characters.

`runtime.py::_zkite_framework_fingerprint` computes it on the device over what the shell
unpacked; `dev_cmd.rs::fingerprint_of` computes it on the host over the payload's own
`zkite/`. **Neither test computes the expected value**: `dev_cmd.rs` pins a literal
`FIXTURE_DIGEST` over a two-file fixture, and `zkite_pybridge` drives the *Python*
function over the identical two files and asserts the *same literal*. One spec, two
implementations, and neither can drift without one of the two tests going red — which
matters because the failure mode of a drifted digest is reporting every Dev Shell as
stale, strictly worse than the silence it replaces.

It **fails open** everywhere: an unreadable directory, a missing `hashlib`, a shell
predating this section (which sends no `framework` field at all — hence
`#[serde(default)]` on the host) all produce silence rather than a warning.

#### The bug in the fix, found on the wire

The first build was tested by hand and passed. It was also **useless in every real run**.

`ZKiteActivity` starts the reload socket from `onCreate`, and it dials in a 500ms retry
loop; the render thread boots CPython separately, taking ~650ms on the emulator and
~1.5s on the Moto G54. So the `hello` frame is written *before* `zkite_pybridge::setup`
returns, essentially always — the socket read an unset fingerprint, took the deliberate
fail-open path, and omitted the field. The hand test passed only because it connected to
an app that had already been running for minutes.

Captured directly rather than reasoned about, by listening on the port through
`adb reverse` and printing the first line:

```
cold launch, pre-fix :  {"op":"hello","backend":"native"}
after a 15s wait     :  {"op":"hello","backend":"native","framework":"e7c0ea4edb80"}
cold launch, post-fix:  {"op":"hello","backend":"native","framework":"e7c0ea4edb80"}
```

The middle line is what made it a race rather than a `None`, and it is the whole
diagnosis. Fixed with a `Condvar` — the fingerprint's slot is now
`Option<Option<String>>`, where the outer layer is "has the render thread published
yet" — and a bounded 20s wait so a boot that hangs outright cannot wedge the socket.
`set_framework_fingerprint` is called on **both** the success and failure paths of
`setup`, so the timeout is unreachable except by a hang.

**The WebView shell needs none of this**, and the asymmetry is real rather than an
oversight: its `ZKiteActivity.onCreate` calls `Native.boot` synchronously *before*
`startReloadServer`, so its socket thread cannot outrun the interpreter. Its fingerprint
is read on the socket thread directly, which is consistent with that shell's own design
— it already enters the interpreter for every `reload()`.

**Standing lesson, and it is a new one for this document's list.** §15.6.1 records four
distinct reasons "the build succeeded" has been wrong. This is a fifth shape and it is
about a *feature* rather than a build: **a fail-open path makes a feature untestable by
observing that it did not fire.** The only evidence that distinguishes "nothing to
report" from "the check never ran" is on the wire.

### 31.7 Verification

**Host suites.** `cli` 34 → **40**, `zkite_pybridge` 10 → **12**. `zkite_py` 130,
`zkite_renderer` 44, `twcore` 57, `bundler` 24, webruntime e2e 28 — all unchanged and
green. Two `zkite_py` tests were *updated* rather than added, both because they asserted
the old behaviour, and both now carry the reason so they are not "fixed" back:
`test_stack_tree_uses_absolute_overlay_not_grid` and
`test_broken_view_renders_error_tree_and_keeps_previous_tree`.

`zkite_pybridge` was run under `--test-threads=1` throughout, per §29.5.

**Conformance: two new fixtures, 25 total, both dprs.**

| fixture | dpr 1 (loose / strict) | dpr 2 (loose / strict) |
|---|---|---|
| stack_paint_order | 1.06 / 1.65 | 0.52 / 0.81 |
| stack_fill | 2.08 / 3.03 | 1.05 / 1.51 |

**Every pre-existing fixture is unchanged at both dprs, to the mismatched-pixel count.**

**Negative controls** (phase 2 §6/M1, §12.5.3, §26.5, §27.1, §28.4, §30.4). The two new
fixtures are deliberately built so each moves for exactly one cause:

| control | `stack_paint_order` | `stack_fill` |
|---|---|---|
| (fixed) | 1.06 / 1.65 | 2.08 / 3.03 |
| the pre-§31 three-pass paint order | **2.77 / 3.39**, max delta **255** | 2.08 *(unmoved)* |
| the `Stack` wrapper without `flex flex-col` | 1.06 *(unmoved)* | **55.19 / 56.14** |

**The two unmoved cells are correct non-discriminations and are the more useful half**,
in the spirit §27.1/§28.4/§30.4 record: `stack_paint_order`'s children carry explicit
widths, so the wrapper class cannot reach it; `stack_fill` has nothing overlapping, so
paint order cannot. Each fixture is a single-cause probe rather than a blunt one.

Three more, on the glue:

| control | result |
|---|---|
| `_zkite_last_error` without the `_glue_error` fallback | boot-failure test fails: *"a glue-level failure must be reportable to `zkite dev`"* |
| `_glue_error_tree` composing `_WAITING_TREE` unconditionally | boot-failure test fails, printing the tree with the placeholder still in it |
| the fingerprint over a directory one `.py` short | digest moves — the `Checkbox` case in miniature |

**Device — the `zkite_phase0` emulator (x86_64, API 34, SwiftShader, density 2.75), GPU
path.** The artifact was checked on all three surfaces before installing (§15.6.1):
`widgets.py` and `errors.py` **inside `zkite-framework.zip`**, and
`_zkite_framework_fingerprint`/`_dev_connected` inside the packed `.so` (only `runtime.py`
is `include_str!`'d). Then `adb uninstall` → `install` → `pm clear`, per both staleness
traps.

| what | result |
|---|---|
| a boot `ImportError` under `zkite dev` | the full traceback arrives in the **terminal** — silent before (§31.4) |
| the same, on screen | wraps; content x-extent **43..1018 of 0..1079**, 61px right margin (§31.3) |
| the placeholder behind it | **0** pure-white and **0** slate-950 pixels — absent (§31.5) |
| overlay glyph colour | (255,100,103), 47,903 px — v4's `red-400` |
| shell built from this tree | connects, **no** staleness warning (the false-positive control) |
| one `zkite/*.py` edited after the shell was built | `STALE DEV SHELL`, both digests, and a pasteable `ABI=x86_64` rebuild command |
| `Stack` paint order | only `ON TOP` renders; white glyphs span x **82..327** |
| ↳ control: the covering card removed | `UNDERNEATH` alone spans x **83..526** — so **199px of glyph run is genuinely occluded** |
| `Stack` fill, child with no `w-` | spans **825px = 300.00dp** against a `w-[300px]` stack, exact |

Controls: **zero** `FATAL EXCEPTION`/`E AndroidRuntime`/`PANICKED`/`ANR` markers across
the session; `pidof` constant; `mCurrentFocus` asserted as the app's own `ZKiteActivity`
at every capture; and an idle control per §27's rule — a second screenshot after 8s of no
input is **pixel-identical across the app content region (0 differing scanlines)**.

**Then the Moto G54, and every claim above re-measured on it — see §31.10.** The
emulator numbers are kept as written, because two densities and two GPU implementations
is materially better evidence than either alone, and because §29.8 is the standing proof
that an emulator pass is evidence rather than proof.

### 31.8 What this deliberately does not do

- **No stacking contexts, no `z-index`, no opacity groups.** §31.1 gives the renderer
  *document order*, which is what the DOM does in the absence of all three. A `z-index`
  in a class string still parses to nothing, and an `absolute` node still paints where
  its tree position says rather than above its siblings. That is a larger project and it
  wants `Select`'s popup and `Sheet` (§30.6) to define what it has to support.
- **`break-all`-style mid-word breaking does not exist**, so a single very long path in
  a traceback still overflows its own line in the overlay. Narrower than the original
  gap and recorded rather than fixed.
- **The WebView Dev Shell was rebuilt but not re-run on a device.** Its half of the
  handshake is the simpler one (no race) and its `runtime.py` is the same file, but that
  is an argument, not an observation — the same limit §30.7 records.
- **No `Select`, no `Sheet`, no `Checkbox`/`Radio` label prop.** Untouched.

### 31.9 Known gaps

- ~~**Nothing in this section was run on the Moto G54.**~~ — **done, see §31.10.**
  Every claim was re-measured on real PowerVR silicon at density 2.5, and nothing
  diverged from the emulator. What that leaves open is one density and one ABI per GPU,
  not a matrix — the standing limit of every device run in this document.
- **The paint-order change was not measured for cost.** Collapsing three passes into one
  changes how many clip push/pops and `set_paint` calls a frame makes, and no frame-time
  comparison was run. It should be cheaper (one clip pair per node instead of up to
  three) but that is reasoning, not a measurement, and §13's harness exists for exactly
  this.
- **`Stack` is still not conformance-covered for the general case.** The two new fixtures
  probe one defect each; a `Stack` with *three* children, partial overlap, or a rounded
  opaque child on top is untested.
- **The `_dev_connected` flag never resets.** A Dev Shell that served a `zkite dev`
  session and then loses it forever will not go back to showing the waiting placeholder
  behind an error. That is the right default — the shell has no way to distinguish "the
  server went away" from "the server is between pushes" — but it means the placeholder is
  now strictly a first-run screen.
- **The 20s fingerprint wait is untested at its bound.** No boot was made to hang, so the
  timeout path is reasoning plus a `Condvar` contract, not an observation. Same class as
  §15.3's "the 10s ack timeout has never fired".
- **`reload.rs` still has no tests**, for §15.5's unchanged reason: `gpu_harness` does not
  build for the host at all. The `Condvar` logic in `lib.rs` is in the same crate and
  inherits that.
- **`zkite dev` still passes no `-s <serial>`** (§24.5). Every `adb` call in this session's
  verification passed it by hand; the CLI's own do not, and this session made more of
  them (the launch probe, plus nothing new — but the gap is unchanged).
- **§29.5's intermittent `zkite_pybridge` failure is unchanged** and both new tests there
  were run under `--test-threads=1`.

### 31.10 The Moto G54 run — every claim re-measured on real silicon

**Status: done, same session.** The owner connected the phone over USB immediately after
§31.9 was written, so the caveat it opens is closed rather than carried. §29.8 is why
this mattered enough to do at once: that run found a real `shadow-2xl` divergence the
emulator had passed cleanly, and §31.1/§31.2 are both *rendering* changes, which is the
same class.

**Device, asserted rather than assumed:** moto g54 5G, `arm64-v8a`, Android 15,
1080x2400 @ 400dpi, and the shell's own boot line agreeing —
`density 2.5, 60Hz -> 16.67ms frame period`, `wgpu adapter: PowerVR BXM-8-256
(IntegratedGpu, backend Vulkan)`, `67 native extension modules available`,
`first-frame since_boot_call=1585ms nodes=6 via=gpu`. Real tiler, real silicon, GPU path.

**Two devices were attached**, so every `adb` call passed `-s ZD222H9YDS` — and the
emulator was then killed outright before running `zkite dev`, because §24.5's gap is that
**the CLI's own `adb` calls pass no `-s`** and would otherwise have picked either one.
That is worth recording as the practical workaround until §24.5 is fixed.

`adb uninstall` → `install` → `pm clear`, per both staleness traps.

| what | emulator (2.75, SwiftShader) | **Moto G54 (2.5, PowerVR)** |
|---|---|---|
| `Stack` paint order — white glyphs on the text row | x 82..327 | **x 77..297** |
| ↳ control, covering card removed | x 83..526 | **x 76..478** |
| ↳ **glyph run genuinely occluded** | 199 px | **181 px** |
| `Stack` fill — child with no `w-`, against `w-[300px]` | 825 px = **300.00 dp** | 750 px = **300.00 dp** |
| ↳ and the emerald card's own width | — | 750 px = **300.00 dp** |
| overlay content x-extent | 43..1018 of 0..1079 | **39..1019 of 0..1079** |
| ↳ right margin vs `p-4` | 61 px vs 44 px | **60 px vs 40 px** (16dp at 2.5) |
| waiting placeholder: pure-white px | 0 | **0** |
| waiting placeholder: slate-950 px | 0 | **0** |
| overlay glyph colour | (255,100,103) | **(255,100,103)**, 44,349 px |

`300.00 dp` at two different densities, from two different pixel counts, is the useful
row: the fill fix is exact rather than approximately right, and it is exact for the
*same reason* on both — cross-axis stretch, not a coincidence of rounding.

**The dev loop, end to end on the phone:**

- **§31.4** — the `ImportError` traceback arrives in `zkite dev`'s terminal, naming
  `/data/user/0/dev.zkite.gpuharness/files/app/main.py` and the real
  `zkite_framework/zkite/__init__.py`. Silent before this section.
- **§31.3/§31.5** — it renders wrapped, with no placeholder behind it (the two zeroes
  above are the measurement).
- **§31.6, positive control** — the shell built from this tree connects with **no**
  staleness warning. This is the one that matters most: a false positive costs a real
  rebuild.
- **§31.6, the real case** — one `zkite/*.py` edited after the shell was built:

```
  !! STALE DEV SHELL — it was built with a different `zkite` framework than this one.
  !!   Dev Shell: e7c0ea4edb80      this checkout/payload: 1ae9e6075c19
  !!   ZKITE_DEVSHELL=1 ABI=arm64-v8a phase_3/gpu_harness/scripts/build_apk.sh
```

  with the device's **real** ABI substituted into a pasteable command.

**Controls.** Zero `FATAL EXCEPTION`/`E AndroidRuntime`/`PANICKED`/`ANR` markers across
the whole session; `pidof` constant (30558) before and after; `mCurrentFocus` asserted as
the app's own `ZKiteActivity` at every capture; and an idle control per §27's rule — a
second screenshot after 8s of no input is **pixel-identical across the app content region
(0 differing scanlines, y=110..2300)**, so nothing here is a stray touch.

**Nothing diverged from the emulator.** That is a real result rather than a null one:
§29.8 established that this pair *can* disagree, so agreeing is information. There are
now two densities (2.5 and 2.75) and two GPU implementations behind §31, which is the
same bar §28/§29.8 ended at.

**Still not covered:** the CPU fallback path was not forced on (its paint loop got the
identical change and is checked only by reading and by the host harness, which uses
`vello_cpu`); the WebView Dev Shell was rebuilt and still not run on a device; and this
remains one phone, one ABI, one density per GPU.

## 32. Labelled controls — and the dispatch order, decided

§30.8's complaint, reported by the owner within minutes of §30 landing: in
`[x] Wi-Fi`, tapping the **text** does nothing, because the live area is a 20x20dp box.
It named the fix as a feature ("let `Checkbox`/`Radio` take a label as content") and
declined to build it.

**Status: built for all three checkbox-shaped controls, and verified live on the
Moto G54.** `Checkbox("text-sm text-white", "Wi-Fi", checked=..., on_change=...)` is one
tap target, one handler, and one accessible node whose *name is the label*. The same
work closes **§15.1 divergence 4b** — the `change`-vs-bubbled-`tap` order — which the
owner decided rather than leaving open (§32.3), and adds `indicator=False` (§32.5) for
the case the label prop alone did not cover.

### 32.1 The design: composed in Python, not borrowed from `<label>`

The obvious implementation was HTML's own answer, and §30.8 explicitly proposed it: "on
the WebView side that is a `<label>` wrapper, which is what HTML has always used for
exactly this." **That was measured and rejected**, and the measurement is the useful
part.

Driven against real Chromium — a `<label>` containing an `<input type=checkbox>` and a
`<span>`, with a document-level capture listener exactly like `zkite-runtime.js`'s:

| click target | click events seen | targets / `detail` |
|---|---|---|
| the label **text** | **2** | `span` `detail=1`, then **`input` `detail=0`** |
| the **box** | 1 | `input` `detail=1` |
| outside the label | 1 | — |

Both clicks are `isTrusted`. Label activation dispatches a *second* click on the
contained control, so an ancestor `Row(on_tap=...)` would have fired **twice on the
WebView and once on native** — a brand-new silent divergence, in exactly the class
§15.1 exists to track — and holding it back would have meant a `detail === 0` guard in
`zkite-runtime.js`, i.e. importing a browser mechanism and then policing its
side effects. The probe also showed `extractPayload` bound to a `<label>` returns
`null`, which `_as_bool` reads as `False`: §26.2's bug shape, a third time.

**What shipped instead is the composition §30.8 had already measured working.** Its own
workaround — a tappable `Row` wrapping a handler-less control and a `Text` — is device-
verified on both backends (§30.4 cases 5 and 6). The widget now renders that shape
itself:

> A labelled control is a **container** that carries the role, the checked state and a
> `tap`; the box is a presentational child; the label is ordinary content.

Every mechanism involved already existed and was already verified. `zkite-runtime.js` is
**unchanged**. `render_thread.rs` needed nothing for this (its tap walk and
`control_at` already do the right thing). The whole feature is `widgets.py` plus three
small generalisations in `zkite_pybridge`.

This is the §9.6/§18.2 move applied to behaviour rather than to data: prefer expressing
a thing once in the layer both backends share over teaching each backend its own copy.

### 32.2 What each node owns

```html
<span id="c.0" class="… inline-flex flex-row items-center"
      role="checkbox" aria-checked="true" data-zkite-on="tap">
  <input id="c.0.box" class="… shrink-0 mr-2" type="checkbox" role="checkbox"
         checked aria-hidden="true" tabindex="-1">Wi-Fi</span>
```

- **The container is the control.** It registers a `tap` — not a `change` — because
  `tap` is the event both runtimes already dispatch to an arbitrary container.
  `Switch._register_events` maps the caller's `on_change` onto it and delivers the new
  value through the payload parser, so the handler still receives a real `bool` and the
  `""` payload both backends send for a tap is discarded rather than parsed.
- **The box is still a real `<input>` on the HTML path**, so `index.html`'s existing
  CSS keeps drawing it. That is deliberate: a labelled control and a bare one render
  the *same pixels* on the WebView, rather than the labelled one becoming a second
  hand-drawn approximation. §30.2's seam is not widened.
- **`aria-hidden` / `aria_hidden` is what moves the semantics outward**, stated on both
  paths rather than inferred from "has a labelled ancestor" — §30.1's role-not-marker
  principle applied to state. `control_of` returns `None` for it (so the box is never a
  second dispatch target) and `a11y_role` returns `GenericContainer` (so a screen reader
  sees one checkbox, not two nested ones).
- **The box is drawn from `_box_classes`, which both forms call.** A labelled box and a
  bare box cannot drift apart in appearance — structural, not remembered, the same
  reason §30.1 made these subclasses rather than copies.
- **The 8dp gap is a margin on the box, not `gap-2` on the container.** `twcore`
  resolves a class string last-wins while real Tailwind resolves by stylesheet order,
  so an implied `gap-2` a caller overrode with `gap-1` would be 4px on native and 8px on
  the WebView — the exact footgun `ScrollView.__init__` documents. A margin has nothing
  to conflict with, so a caller's own `gap-*` is additive and identical on both.

Three generalisations in `zkite_pybridge`, each making an existing rule *more* general
rather than adding a special case: `a11y_role` reads the `role` attribute for **any**
tag (before, only inside the `input` arm — so a labelled control was announced as
`Role::Label`, i.e. a piece of text); `control_of` and `a11y_role` both honour
`aria_hidden`; and `build_a11y_node` gives a role-bearing control an accessible **name**
gathered from its subtree text, which is what makes TalkBack say "Wi-Fi, checkbox,
checked" instead of naming nothing.

### 32.3 The `change`-vs-bubbled-`tap` order — decided, and native moved

§15.1 divergence 4b, open since §26.7 and *measured* in §27.6: a tap on a control inside
a row with `on_tap` fired the row's handler first on the WebView (capture-phase
listener) and the control's own `change` first on native. Both always fired; the order
only mattered when both wrote the same `@store` field. §30.8 found the case where that
costs a real user and raised it from theoretical to urgent.

**The owner's decision: the bubbled `tap` runs first, and native matches the WebView.**
So `zkite-runtime.js` is unchanged and `render_thread.rs::dispatch_tap` moved.

**The payload is still computed from the pre-tap build**, and that is what makes this
faithful rather than merely reordered: on the WebView the `change` payload is
`el.checked` read out of a DOM that Blink toggled at gesture time and that no re-render
has reached yet, because the shell applies new HTML asynchronously. Both backends now
report the value the control had when the finger landed, in the same order, whatever an
ancestor's handler did in between.

**The labelled form makes the whole question unreachable anyway**, and that is
structural rather than a convention: `findBoundAncestor` stops at the *nearest* bound
ancestor and `render_thread`'s walk breaks at the first `handles(id, "tap")` — both of
which are the labelled control itself. So a labelled control inside a tappable row
absorbs the tap and the row's handler does not also fire. There is never a second
handler to order.

### 32.4 Verification

**Host suites.** `zkite_py` 130 → **142**, `zkite_pybridge` 12 → **14**, webruntime e2e
28 → **31**. `cli` 40, `bundler` 24, `zkite_renderer` 44, `twcore` 57 — unchanged.
`zkite_pybridge` was run under `--test-threads=1` throughout, per §29.5.

**The bare form is byte-identical to `HEAD`**, across 12 states (three widgets x checked
x with/without a caller class string) on **both** render paths, compared by diffing the
emitted HTML and tree JSON against the pre-§32 module. That is the regression argument
for everything §30 verified, and it is why no pre-existing conformance fixture could
move.

**Conformance: two new fixtures, 27 total, both dprs.**

| fixture | dpr 1 (loose / strict) | dpr 2 (loose / strict) |
|---|---|---|
| labelled_controls | 2.92 / 4.14 | 1.72 / 2.53 |
| radio_custom | 2.27 / 3.69 | 1.86 / 2.56 |

**Every pre-existing fixture is unchanged at both dprs** against §31.7's and §30.4's
tables — including `checkbox` (0.55/1.14 and 0.51/0.73) and `switch` (0.17/0.90).

Both new numbers were **attributed rather than accepted**, per §28's "don't eyeball a
rendering result" rule, using a ~60-line pure-Python PNG reader (there is still no PIL
on this box):

- `labelled_controls` — the indigo boxes occupy **x 16..35, y 18..101 on both
  backends**, identically; the label glyphs start at **x=45 on both** and share a y
  extent of 22..99. So box geometry, the `mr-2` gap and `items-center` alignment are
  *exact*. Of the mismatch, 945 px sit in the label column and 179 in the box column —
  the latter being §30.2's pre-existing CSS-tick-vs-sprite seam. The rest is glyph
  antialiasing, the same class as `text` (5.00%).
- `radio_custom` — both cards have **identical bounding boxes on both backends**
  (x 16..203, y 21..60 and y 69..108), and 1081 of 1121 differing columns fall in the
  text band.

**Negative controls** (phase 2 §6/M1, §12.5.3, §26.5, §27.1, §28.4, §30.4, §31.7).
Seven on the host, each reverting exactly one thing to its pre-§32 form; **all seven
discriminated**:

| control | result |
|---|---|
| `control_of` without the `aria_hidden` guard | the labelled box becomes a second dispatch target — pybridge test fails |
| `a11y_role` without the role-first hoist | a labelled control is announced as `Role::Label` — **three** tests fail, including §30's own |
| `a11y_role` without `aria_hidden` transparency | two nested checkboxes announced — pybridge test fails |
| `build_a11y_node` without the deep label | the control has no accessible name — pybridge test fails |
| `Radio._next_checked` removed | a labelled radio unticks itself — `zkite_py` test fails |
| the labelled form registering `change` instead of `tap` | four `zkite_py` tests fail |
| `_indicator()` pinned to `True` | two `zkite_py` tests fail |

The second row is worth noting: removing the hoist also breaks §30's own
`role_distinguishes_switch_from_checkbox_from_radio`, which is the evidence that the
hoist genuinely *moved* that logic rather than shadowing it, and that the old test still
guards it.

Two more, on the harnesses rather than on the code:

| control | result |
|---|---|
| the pre-§32 markup (bare control + separate `Text`), click on the text, real Chromium | **0 dispatches** — §30.8's complaint reproduced exactly |
| the tree path's box gap diverged to `mr-4` while HTML keeps `mr-2` | `labelled_controls` 2.92 → **4.44%** (dpr1), 1.72 → **3.64%** (dpr2), while `checkbox`, `radio`, `switch` and `text` are **unmoved to the digit** |

That last row is the §31.7-shaped result: the fixture moves for a labelled-layout
divergence and for nothing else, and the four unmoved cells are the more informative
half.

### 32.5 `indicator=False` — the app's own styling, the platform's semantics

Raised by the owner while this was being built, and it is a real gap the label prop
alone did not close: *"if there are MCQs then instead of radio, developer can show his
own styling along with option and user selects one of them."*

Before this, the box was unconditional, so the only way to build a multiple-choice
question, a segmented control or a colour swatch was a tappable `Row` — **which throws
away the entire reason these widgets exist.** A `Row` is announced to a screen reader as
generic clickable text; there is no role, no checked state, no group semantics. That is
the same trade §30.1 refused when it chose a role over a private marker.

`indicator=False` drops the built-in box/circle/track and nothing else. The container
keeps the role, the state, the accessible name and the tap:

```python
Radio("px-4 py-3 rounded-xl border-2 " + (
        "border-indigo-500 bg-indigo-950 text-white" if picked else "border-slate-700"),
      "B. Mitochondria", checked=picked, indicator=False,
      on_change=lambda v: s.update(answer="b"))
```

It composes with the controlled contract rather than adding to it — `checked=` already
tells the widget its state, and the caller derives their class string from the same
store field — so there is no new mechanism, only a suppressed node. It works on **all
three** controls, not just `Radio` (the flag lives on `Switch` and the other two
inherit it), and content is optional: `indicator=False` with no children is the colour
swatch, where the container *is* the widget.

### 32.6 The Moto G54 run

**Device, asserted rather than assumed:** moto g54 5G, `arm64-v8a`, Android 15,
1080x2400 @ 400dpi, and the shell's own boot line agreeing — `density 2.5, 60Hz ->
16.67ms frame period`, `wgpu adapter: PowerVR BXM-8-256 (IntegratedGpu, backend
Vulkan)`, `67 native extension modules available`, `first-frame ... via=gpu`. One device
attached this time, and every `adb` call still passed `-s ZD222H9YDS`.

The artifact was checked on all three surfaces before installing (§15.6.1, §30's
pattern): `app.zip`'s `main.py`, **`widgets.py` inside `zkite-framework.zip`**, and
`aria_hidden` inside the packed `.so`. Then `adb uninstall` → `install` → `pm clear`,
per both staleness traps.

`demo_app/labels.py`, written against the documented signatures only per §26.7. Each
case run **in isolation from a clean state** per §27's no-batching rule, with
`mCurrentFocus` and `pidof` asserted before *and* after every one.

| case | tap | result |
|---|---|---|
| idle control, 8s, no input | — | **0 differing scanlines** across the app content region |
| **1 — the label** | 400px right of the box | one dispatch, `tap to r.2`; **"Wi-Fi: ON", "toggles: 1"**; box fill 0 → **2276 indigo px** |
| **2 — the box** | on the 20dp box | one dispatch; **`toggles: 1`, not 2** — the `aria_hidden` guard, live |
| **3 — radio, once** | a labelled option | selects it **and clears the previously selected one** |
| **3b — radio, twice** | the *selected* option | still `checked="true"`; **it does not clear** |
| **4 — labelled in a tappable row** | on the control | `tap to r.11.0` only; **`row taps: 0`** — the row did not also fire |
| **4b — control** | on the row, not the control | `tap to r.11`, `row taps: 1`, box unchanged |
| **5 — bare control in a tappable row** | on the box | `dispatching tap to r.13` **then** `toggling switch`; app records `dispatch order: tap change` |
| **6 — `indicator=False` MCQ** | a styled card | `checked="true"` on B, false on A, `answer: b` |

**The a11y half, end to end through AccessKit into Android's own layer.**
`uiautomator dump` reports `android.widget.CheckBox` with **`content-desc="Wi-Fi: OFF"`**
— the label *is* the accessible name — and `RadioButton`s named "Low"/"Medium"/"High".
The MCQ options, which draw **no circle at all**, are still
`android.widget.RadioButton` named "A. Golgi" / "B. Mitochondria" with `checkable="true"`.
That is §32.5's whole argument, observed rather than asserted.

**Hit target, measured:** a labelled control's bounds are `[40,297][1040,387]` =
**400 x 36dp**, against §30.7's bare 20x20dp. Much better, and still short of the ~48dp
Android asks for in *height* — see the known gaps.

**A device negative control for §32.3**, because the reorder is the one claim with no
host test at all (`gpu_harness` does not build for the host, §15.5) — so the device is
its only instrument, and an instrument with no control proves nothing. The pre-§32
order was rebuilt, installed and driven through the identical gesture:

| build | log order | the app's own record |
|---|---|---|
| pre-§32 | `toggling switch` → `dispatching tap` | `dispatch order: change tap` |
| **§32** | `dispatching tap` → `toggling switch` | `dispatch order: tap change` |

Same fixture, same tap coordinate, one variable. The source was then restored, rebuilt,
and cases 4 and 5 re-run against the **final** artifact.

**Controls.** Zero `FATAL EXCEPTION`/`E AndroidRuntime`/`PANICKED`/`ANR` markers across
every case; `pidof` constant within each; `mCurrentFocus` the app's own `ZKiteActivity`
at both ends of every measurement. One measurement was **discarded** rather than
reported: the first idle control ran while the app had been backgrounded and returned
"1383 differing scanlines" — of Instagram. That is §29.8's "the instrument lies" in a
new shape, and it is why the focus assertion moved to *both* ends.

### 32.7 What this deliberately does not do

- **No `<label>` anywhere**, and no `zkite-runtime.js` change. §32.1.
- **No grouping widget.** Exclusivity is still one store field and `checked=` per
  option; nothing warns if two radios in a group are both checked (§30.7, unchanged).
  `indicator=False` makes MCQs easy to build, which makes a `RadioGroup` that owns
  `aria-labelledby`/`role="radiogroup"` more attractive than it was — filed, not built.
- **No hit-target padding is added for you.** The label widens the target a great deal
  and does not make it 48dp tall on its own.
- **No `disabled`, no indeterminate state, no keyboard/focus interaction.** All three
  are §30.5's gaps and all three are unchanged.
- **`Select` is still not built.** The owner's decision this session is the **self-drawn
  popup** (§30.6's recommendation), and separately that `Sheet` is *not* the same piece
  of work. Both are unstarted; the popup drags in §31.8's stacking-context question,
  which is why §30.6 said the two define what it has to support.

### 32.8 Known gaps

- **The WebView backend was not run on a device for any of this.** The rendering is
  covered by the conformance harness (real Chromium, the same engine the WebView runs)
  and the dispatch by the e2e suite in that same Chromium, but "expected to behave
  identically" is a different claim from having watched it — the same limit §30.7 and
  §26.7 both record. The native backend is the default and is where the device time went.
- **A decorative sprite child is still exposed to a screen reader.** The device dump
  shows the checked box's tick as a child `android.widget.ImageView` named `"check"`,
  so TalkBack announces the checkbox and then an image. **This is pre-existing and
  affects the bare form identically** — it is `Icon`'s missing accessible-name/decorative
  prop (§8.6, §18.7) seen from a new angle, not something §32 introduced. Deliberately
  not fixed here: marking the tick presentational would have broken the byte-identical
  property §32.4 rests its bare-form regression argument on, and the right fix is a
  general `Icon(decorative=True)` rather than a special case inside three widgets.
- **`text_label_from_children` is still direct-children-only for the generic branch**,
  so a `Button` holding widget children rather than a bare string *still has no
  accessible name at all*. Found while writing `text_label_deep` and deliberately not
  widened, because it would change the announced name of every such button in every app
  — a real pre-existing a11y bug, now named, with a one-line fix and a blast radius that
  wants its own measurement.
- **`indicator=False` has no device evidence for `Switch` and `Checkbox`**, only for
  `Radio` (the MCQ case). The three share one code path and the host tests cover all
  three, but that is an argument, not an observation.
- **The 8dp gap is not overridable**, only additive. Stated in WIDGETS.md rather than
  fixed, because every mechanism for making it overridable reintroduces the
  class-order divergence §32.2 avoids.
- **No `zkite dev` session was run against a labelled control**, so §15.3's standing
  "only the counter app was reloaded" gap is unchanged and now has one more thing worth
  reloading.
- **One phone, one ABI, one density, the GPU path.** The CPU fallback was not forced on.
  The standing limit of every device run in this document.
- **§29.5's intermittent `zkite_pybridge` failure is unchanged**, and everything here was
  run under `--test-threads=1`.

## 33. `Select` — and the overlay layer it turned out to need almost none of

§30.6 wrote the decision down and §32 recorded the owner's answer: **the self-drawn
popup**, with `Sheet` a separate piece of work. This builds it.

**Status: built on both backends, and verified live on the Moto G54 on *both* — which
also pays the WebView device debt §30.7, §31.8 and §32.8 had each carried forward.**

The headline is not the widget. It is that **"paints above the whole app" needed no
renderer change at all**, because both backends already agree on the rule it rests on:
a node emitted later in document order is painted later and hit-tested first.
`zkite_renderer` paints in document order (§31.1) and `Renderer::hit_test` walks
`positioned` in **reverse**; the DOM does the same for positioned elements in tree
order. So a layer is not a new kind of thing — it is an ordinary subtree that `app.App`
emits last. `render_thread.rs` is untouched, `zkite-runtime.js` is untouched, and
`twcore` gained nothing. §32.1's move, one level up: ask what both backends already do,
and compose that, rather than inventing a mechanism and teaching each side its copy.

### 33.1 The shape, and the one word that is load-bearing

`Overlay` is public, and that is the answer to the owner's question about adaptability —
`Select` is *one composition over it*, not the only thing that can be built on it:

```python
Overlay("rounded-2xl bg-slate-800 p-4",          # the panel: the caller's classes
        Text("text-white", "Anything"),
        placement="items-center justify-center p-6",
        scrim="bg-black/50",
        on_dismiss=lambda: s.update(open=False))
```

Three nodes, each doing one job:

| node | classes | why |
|---|---|---|
| root | `fixed inset-0 flex` + `placement` | full-bleed; `placement` is where the panel sits |
| scrim | `absolute inset-0` + `scrim` | full-bleed, **under** the panel, and owns `on_dismiss` |
| panel | caller's + `relative flex` | the content |

**The dismiss handler is on the scrim, not on the layer root.** That is what lets a tap
on the panel do nothing *without* the panel registering a no-op `tap` to absorb one —
which would have made the listbox announce itself as clickable to a screen reader. The
alternative was considered and rejected for exactly that.

**`relative` on the panel is the load-bearing word, and it is implied rather than
documented as the caller's job.** CSS paints *positioned* elements above non-positioned
ones regardless of tree order, so a non-positioned panel sits correctly on native
(document order) and is **buried under its own scrim** on the WebView. Driven against
real Chromium rather than reasoned about, per §32.1's rule:

| panel `position` | `elementFromPoint(panel centre)` |
|---|---|
| `relative` | **`panel`** |
| `static` (the default) | **`scrim`** |

That is a silent divergence of precisely the class §15.1 exists to track, and a rule a
caller can forget is one waiting to happen — so the widget states it. §33.5 runs it as
a negative control through the whole e2e suite.

**Placement is a default value, not a geometry.** `items-center justify-center` centres
it; `items-end` is a bottom sheet; `items-start` is a top menu. Measured on the device:
the same widget with `placement="items-end"` puts its list at y **1949..2309** of a
2400px screen against the centred default's **989..1469** (§33.6). The one placement out
of reach is *anchored to the trigger*, and deliberately: nothing tells Python where a
node landed on screen, and inventing a measurement channel for a dropdown is the same
mistake as reaching Queue F through it.

### 33.2 `Select`, and what a caller owns

```python
Select("px-4 py-3 rounded-xl bg-slate-800 text-white",
       options=["Brazil", "India", "Kenya"],      # or [(value, label), ...]
       value=s.country,
       on_change=lambda v: s.update(country=v))
```

**Open state is uncontrolled by default.** Requiring a second `@store` field and three
callbacks to show a dropdown is ceremony, not control — so the widget keeps it in a
`@store` defined *in `zkite`*, exactly as `Navigator` is (§16.2), which buys the
re-render for free and makes an open popup survive a hot reload for free. Passing
`open=` takes it over completely and the widget stops touching that state.

Every visual surface is a class string with a default: `placement`, `scrim` (`""` keeps
outside-tap dismissal with no dimming), `panel`, `option_class`,
`option_selected_class`. Two escape hatches beyond that, and both are §32.5's
`indicator=False` argument in another shape — *the framework keeps the semantics, the
app keeps the drawing*:

- **`option=`** replaces a row's contents outright while the row keeps its `role`,
  its selected state, its accessible name and its tap. What developers reach for
  instead — a tappable `Row` — is announced as generic clickable text.
- **passing children** replaces the trigger's default content (the selected label and a
  chevron), the same way a label replaces a `Checkbox`'s (§32).

And **`header=`** puts a widget above the list. That slot is what a search-as-you-type
picker needs, and it is why §33.6 spends a device case on typing into it.

### 33.3 A new sprite, from the file that is already the source of truth

A picker that does not say it opens something needs a chevron, and `twcore` parses no
`rotate-*`, so an existing one could not be turned. `chevron-down` was added to
`phase_1/webruntime/index.html`'s `<symbol>` sheet and
`phase_3/renderer/scripts/gen_icons.py` re-run — 12 sprites became 13 and `icons.rs`
regenerated. That is §18.2's design working as intended: the WebView's own sheet is the
source of truth and the native table is generated from it, so the two cannot drift.

### 33.4 The bug the first end-to-end run found, and the rule it produced

The very first render of the widget picked the placeholder instead of the option that
had just been tapped. `on_change` had received `""`.

An option's handler was a one-parameter lambda closing over its value with a default
(`lambda v=value: ...`). `RenderContext.dispatch` hands a one-parameter handler *the
bridge's payload* — and a tap's payload is the empty string on **both** backends. So the
default never applied and the chosen value was replaced by nothing. That is §26.2's
shape (`bool("false") is True`) for a third time: a value crossing a channel that has no
room for it.

The fix is that the value **never crosses the bridge at all** — `Select._chooser`
returns a genuine zero-argument closure. And the arity rule it collided with is now one
function, `widgets.invoke`, used by both `RenderContext.dispatch` and `Select._choose`,
so "a callback may ignore its payload" is a single implementation rather than two
nearly-identical ones. Real Chromium confirms the premise directly: clicking an option
dispatches `("r.1.opt.2", "tap", "")` — the payload really is empty (§33.5).

### 33.5 Verification on the host

**Suites.** `zkite_py` 142 → **169**, `zkite_pybridge` 14 → **16**, webruntime e2e 31 →
**37**. `cli` 40, `bundler` 24, `zkite_renderer` 44, `twcore` 57 — unchanged. `pybridge`
under `--test-threads=1` throughout, per §29.5.

**Everything that does not open a popup is byte-identical to `HEAD`.** 62 states —
all 27 pre-existing conformance fixtures on *both* render paths, plus `Stack` (edited
here), `Nav`, the counter app and the whole control family through a real `App` —
diffed against the pre-§33 package. Zero differences. That is the regression argument
for §30, §31 and §32 in one measurement, and it holds because a layer wraps the root
**only when one is open** and the view's own root keeps its `"r"` either way (scroll
offsets, `List` windows and IME focus are all keyed by node id).

**Conformance: two new fixtures, 29 total, both dprs.**

| fixture | dpr 1 (loose / strict) | dpr 2 (loose / strict) |
|---|---|---|
| select_closed | 0.73 / 1.18 | 0.45 / 0.68 |
| select_open | 0.91 / 1.27 | 0.51 / 0.69 |

Every pre-existing fixture is unchanged at both dprs against §32.4's table.
`select_open` renders the **real** `Select.popup()` rather than a transcription of its
markup — `build_manifest` renders an `Overlay` fixture as a layer the way `app.App`
does — so it cannot agree with a widget that no longer emits this (§9.5/§18.2's lesson).
`Select.popup()` is public for that reason.

Both numbers **attributed, not accepted** (§28's rule), with a ~60-line pure-Python PNG
reader:

- `select_closed` — the trigger's box occupies **x 16..263, y 18..61 on both backends,
  identically**, and **100%** of the differing pixels lie inside the single text row.
  Zero elsewhere.
- `select_open` — the panel occupies **x 16..263, y 50..209 on both**; **99%** (1004 of
  1012) of the difference is in the three option-text rows and the remaining 8 pixels
  have a max delta of 9, at the rounded corners. The scrim composites to **(1, 3, 12) on
  both**.

**Thirteen negative controls, all discriminating.** Eight on `zkite_py`, four on
`zkite_pybridge`, one on the conformance fixture:

| control | result |
|---|---|
| the panel is not `relative` | 2 tests fail — and real Chromium hit-tests the scrim instead |
| the dismiss handler moves to the layer root | 3 tests fail |
| the chosen value comes from the bridge payload | 3 tests fail (§33.4's own bug, pinned) |
| `invoke` always passes the value | 18 tests fail |
| no `None` skip — an `Overlay` reserves a tree slot | 1 test fails |
| layers emitted first instead of last | 5 tests fail |
| the root is wrapped unconditionally | 5 tests fail |
| `_overlay_layers` iterates instead of draining by index | 1 test fails |
| `a11y_role` without the three new role arms | 2 tests fail |
| `build_a11y_node` without the `selected` arm | 2 tests fail |
| the combobox names itself from direct children only | 1 test fails |
| `expanded` is not set | 1 test fails |
| the tree path's option padding diverges to `px-8` | `select_open` 0.91 → **1.77%**, while `select_closed`, `checkbox`, `labelled_controls`, `stack_fill`, `text` and `typography` are **unmoved to the digit** |

That last row is the §31.7/§32.4 shape again: the fixture moves for a popup-layout
divergence and for nothing else, and the six unmoved cells are the more informative half.

**Six new checks in real Chromium** (`e2e.mjs`), driving HTML rendered by the actual
Python `App` rather than transcribed: the scrim fills the viewport with the panel inside
it; the panel is hit-tested **above** its own absolutely-positioned scrim; a point
outside the panel lands on the scrim; a click on the panel dispatches **nothing**; a
click on an option dispatches its own id with an **empty** payload; a click outside
dispatches the scrim's dismiss.

### 33.6 The Moto G54 run — both backends

**Device, asserted rather than assumed:** moto g54 5G, `arm64-v8a`, Android 15, 1080x2400
@ 400dpi, one device attached, every `adb` call still passing `-s ZD222H9YDS`. The
shell's own boot line agreeing: `density 2.5, 60Hz -> 16.67ms frame period`,
`wgpu adapter: PowerVR BXM-8-256 (IntegratedGpu, backend Vulkan)`, `67 native extension
modules available`, `first-frame ... nodes=44 via=gpu`.

The artifact was checked on all three surfaces before installing (§15.6.1) — `app.zip`'s
`main.py`, `widgets.py` **inside `zkite-framework.zip`**, and the new sprite's path data
inside the packed `.so`. **One check failed and its control is what made it readable:**
`"combobox"`/`"listbox"` are *not* in the `.so` — but neither are `"checkbox"` or
`"radio"`, which §30 and §32 proved working on this device. A short `match &str` arm
compiles to inline byte comparisons with no stored literal; the new *attribute* keys
(`option`, `expanded`, `selected`) are present. An artifact check needs its own control
too.

Each case run **in isolation from a clean launch** per §27's no-batching rule, with
`mCurrentFocus` and `pidof` asserted before *and* after every one. Zero
`FATAL EXCEPTION`/`E AndroidRuntime`/`PANICKED`/`ANR` markers across every case.

**Native (the default backend), 9 cases.**

| case | result |
|---|---|
| idle control | no input, no change |
| **1 — the popup paints above the app** | the three opaque colour blocks *above the trigger in document order* are each **exactly halved** per channel: (255,32,86)→(127,16,43), (0,188,125)→(0,94,62), (255,185,0)→(127,92,0) |
| **2 — choosing** | `chosen: ke`, `changes: 1`, popup gone, and the Spinner's accessible name becomes **"Kenya"** |
| **3 — a tap on the panel** | popup **stays open**, `panel taps: 0` — nothing behind the layer is reachable through it |
| **4 — a tap outside the panel** | popup closes, `changes: 0` |
| **5 — Back** | popup closes, `pidof` and `mCurrentFocus` unchanged — it did **not** exit the app |
| **6 — `placement="items-end"`** | the list sits at y **1949..2309** against the centred default's 989..1469 |
| **7 — app-drawn rows (`option=`)** | `android.view.View` named "A. Golgi"/"B. Mitochondria"/"C. Ribosome" with **`selected="true"`** on the current one |
| **8 — a `TextField` in the popup** | a real `android.widget.EditText`; typing `Ken` gives `query: Ken` and the list **re-filters live to one option** |

**The a11y half, end to end through AccessKit into Android's own layer.** `uiautomator
dump` reports the trigger as **`android.widget.Spinner` whose name is the selected
label** ("India", then "Kenya" after case 2), the panel as **`android.widget.ListView`**,
and each option named by its text with `selected="true"` on the current one.

**WebView (the owed run — three sessions old).** `zkite build apk --backend webview` of
both `demo_app/select.py` and `demo_app/labels.py`, artifact checked on all three
surfaces first (`main.pyc`'s docstring, `widgets.py` inside `zkite-framework.zip`, and
`icon-chevron-down` inside the packed `index.html`). Its accessibility tree is not
exposed to `uiautomator`, so **screenshots are the instrument**, which is the posture
§29.8 already sets.

*`select.py`, against the native table above:*

| case | scrim up | `chosen:` row | trigger row |
|---|---|---|---|
| idle | no | — | — |
| open | **yes** | covered | covered |
| choose | no | **changed** | **relabelled** |
| outside tap | no | **0 px changed** | **0 px changed** |
| Back | no | **0 px changed** | **0 px changed** |

And the same scrim arithmetic, on the other engine: the three blocks halve to
(128,16,43), (0,94,63), (128,93,0) — **within one unit per channel** of native's
(127,16,43), (0,94,62), (127,92,0), the difference being that Chromium rounds half up
where the native path floors.

*`labels.py` — §32's cases, finally watched on this backend:*

| case | result |
|---|---|
| tap the Wi-Fi **label**, 400px from its box | the box goes empty → **checked**, `toggles` moves — §30.8's complaint, fixed here too |
| tap the Bluetooth **box** | checked → empty, `toggles` moves **once**, the other row untouched |
| tap the **row**, not the control | `row taps` text changes; the control is untouched |
| tap the **control inside the tappable row** | the control fires and **`row taps` does not change** — the labelled control absorbs the tap, §32.3's structural property |

### 33.7 What this deliberately does not do

- **No animation.** Decided rather than deferred: §30.6 said the popup "needs the
  animation driver to not feel cheap", and it does, but Queue F is architectural, costs
  `Nav`'s "only the top screen is built" property (§16.7), and should not be built under
  a widget's deadline. The popup appears and disappears in one frame.
- **No anchored-to-the-trigger placement**, and no measurement channel invented for it.
  §33.1.
- **No `Sheet`, no `Dialog`, no `Toast`.** All three are now compositions over a layer
  that exists, which is the point, but none is built.
- **No multi-select and no `keep_open`.** Choosing always closes.
- **No keyboard/focus interaction, no `disabled` option, no type-ahead selection.**
- **No `z-index`, no stacking contexts, no opacity groups.** §31.8 is unchanged. What
  this establishes is that the *specific* consumer §31.8 was waiting for does not need
  them: a root-level layer in document order is what both backends already agree on.
  A `z-index` in a class string still parses to nothing.

### 33.8 Known gaps

- **Nothing pushes a frame from Python.** Found while answering "could a `Select` fetch
  its options from an API as the user types?" — the UI half works (§33.6 case 8), and
  the answer to the rest is **no, not yet**: nothing registers `App.on_render_tree` or
  `App.on_render` on *either* backend. Every frame is **pulled** — the shell calls
  `_zkite_dispatch_tree(...)` and paints the tree it gets back. A background thread that
  finishes a fetch and calls `s.update(...)` refreshes `last_tree` and **nothing
  repaints** until the next touch. This is not a `Select` gap; it is a framework gap
  that nothing had needed until now, and it is filed in §15.1.
- **`on_input` is still dispatched by neither backend on native** (§26.7), so live
  typing works through `on_change` natively and through `on_input` on the WebView. A
  developer writing a typeahead today has to register **both**. §33.6 case 8 used
  `on_change`.
- **Whether a ZKite app can reach the network at all is unproven.** `ssl.py`, `socket.py`,
  `urllib` and `http.client` are in the staged stdlib and `libpymod__ssl.so` /
  `libpymod__socket.so` are in the harness build, and the shell reports 67 extension
  modules — but no ZKite app has ever made an HTTPS request. Read from the artifact, not
  observed.
- **`Role::ListBoxOption` has no Android class mapping.** `accesskit_android` 0.7.5 maps
  `ComboBox` → `android.widget.Spinner` and `ListBox` → `android.widget.ListView`, but
  an option falls through to the generic `android.view.View`. Its *name* and its
  `selected` state do reach Android, which is the half that matters for announcing which
  option is current.
- **`aria-expanded` does not reach Android at all.** That crate consumes `is_selected`
  and never reads `is_expanded`. The state is correct in the accesskit tree and the
  WebView gets the equivalent from Chromium; TalkBack on native will not hear it.
- **No ZKite node is `clickable` in `uiautomator`'s sense**, on any backend, and this is
  pre-existing: `accesskit_android` calls `addAction(ACTION_CLICK)` but never
  `setClickable(true)`, and those are different flags. TalkBack activates from the action
  list, so activation works; a tool reading the flag will disagree.
- **The chevron and the tick are announced as `ImageView`s** named "chevron-down" and
  "check". Second and third instance of §32.8's decorative-sprite gap, unchanged and
  still wanting a general `Icon(decorative=True)`.
- **Only one popup can be open at a time**, by construction (`_SelectState` holds one id).
  Right for a picker; a constraint if `Overlay` is used for something else uncontrolled.
  A controlled `Overlay` has no such limit.
- **An uncontrolled `Select`'s open state is keyed by node id**, so a tree reshuffle
  between renders can close it. Same assumption `List`'s viewport state already makes,
  and `key=` is the escape hatch.
- **The popup is not virtualised.** Every option is built on every render while open;
  a thousand-option picker builds a thousand rows. `List` exists and was not used,
  because it needs a fixed row height and the option row is the caller's to draw.
- **The e2e suite now shells out to `python3.14`** to render its fixture. That is what
  makes the check faithful, and it is a new dependency for that file.
- **One phone, one ABI, one density, the GPU path.** The CPU fallback was not forced on.
- **§29.5's intermittent `zkite_pybridge` failure is unchanged**, and everything here was
  run under `--test-threads=1`.

## 34. The push path, and the first ZKite app to reach the network

§33.8 named the largest gap in the framework, and it was not a widget: **nothing could
push a frame from Python.** Every frame was *pulled* — the shell dispatched an event and
painted the tree it got back — so a `@store.update()` from a background thread refreshed
`App.last_tree` and **nothing repainted** until the user next touched the screen. Paired
with it: **no ZKite app had ever made a network request.** Both had to work before "fetch
and display" was possible, which is why they are one section.

**Status: built on both backends. Verified live on the Moto G54 for the native backend —
including a real HTTPS request returning `ok 200` and a body. The device run is
incomplete: the phone was disconnected from USB partway through, and §34.8 lists exactly
what is owed rather than implying it was done.**

The owner's decision, taken before any code was written: **both backends get a push
path**, because native-only would ship precisely the silent divergence §15.1 exists to
track — an app that loads remote data working on the default backend and showing a blank
screen on the other, with no error anywhere.

### 34.1 `on_input` on native, and the contract that was measured first

§26.7 recorded that `on_input` is registered by `_register_events` on **both** backends
and dispatched by neither on native, so a developer writing a search-as-you-type field
had to register `on_change` for native and `on_input` for the WebView and hope.

Before wiring anything, the contract the WebView actually has was **measured in real
Chromium** rather than reasoned about — §32.1's rule, now the seventh divergence settled
by driving the real thing (`phase_1/webruntime/test/probe_input_change.mjs`, kept):

| action | events dispatched |
|---|---|
| type `Ken` into a `TextField` | `input` x3, **`change` x0** |
| then blur it | **`change` x1** |

`render_thread.rs` now dispatches `input` **and** `change` from one helper
(`dispatch_text_edit`), used by both text-edit sites — `TextInputEvent` (a keystroke) and
`SelectionOverlayDismissed` (a cut/paste from the native toolbar). `input` first, which
is the DOM's own order for a single value transition.

**The gap turned out to be wider than §26.7 named, and the wider half is deliberately not
fixed.** Native has dispatched `change` **per keystroke** since §7; the WebView dispatches
it **on blur**. So `on_change` on a `TextField` means "live typing" on one backend and
"the user finished" on the other — a divergence that has been there since phase 3 began
and is not in §15.1. Retiming native's `change` would match the WebView (which is what
§32.3's precedent did for the `tap`/`change` order) but would **silently break every
native app that uses `on_change` for live typing** — WIDGETS.md documents it that way and
§33.6's own typeahead is built on it. Adding `input` is purely additive: a field that
never registered one dispatches exactly what it dispatched before. **The contract question
is written up for the owner in §34.10 and filed in §15.1; it is not guessed at here.**

Both halves of the measured contract are now pinned in `e2e.mjs`, so a silent flip — in
Blink or in `zkite-runtime.js` — fails the suite.

### 34.2 The push path, and why the re-entrancy answer has to be per-thread

`App.on_render`/`on_render_tree` have existed since phase 1 and **nothing had ever
registered either**. The mechanism was three small pieces:

| piece | what |
|---|---|
| `zkite_pybridge::embed::install_frame_hook` | Defines `_zkite_native_frame_ready(frame)` in `__main__` — a real `PyMethodDef`/`METH_O` C function, injected once at init. |
| `runtime.py::_push_frame` / `_register_push` | Looks that hook up **by name at call time** and registers `App`'s callback in whichever mode the app renders in. |
| `zkite_pybridge::set_frame_sink` | Where the shell says what to do with a frame. `None` by default, which makes the whole path inert. |

Two design points that are load-bearing.

**The frame crosses as a serialized string, not as a signal.** The alternative — notify,
then have the shell come back and pull `last_tree` — loses a race the moment two updates
land close together: the shell would fetch whatever is current *then*, not the frame that
fired. Serializing inside the callback means the sink is handed exactly the tree that was
built. The cost (a `json.dumps` on a worker thread) is paid where the work already is,
and coalescing recovers it: the render thread drops older `FrameReady`s **without parsing
them**, which is where their cost would be.

**The "don't push what you just pulled" guard is a thread-local, and a global would be
wrong rather than merely coarse.** Every pull (`dispatch`, `back`, `reload`, `scroll`,
`setup`) ends in a `render_tree()` that now fires the callback, so without a guard every
tap would notify the sink about a frame its caller is already holding — a second full
parse, layout and paint per touch. But CPython switches threads every
`sys.getswitchinterval()` (5 ms) *inside* a call, so a background thread's `update()` can
genuinely run while the render thread sits in `PyObject_CallObject`. A process-global flag
would read "true" and **drop that frame on the floor** — the exact bug this section exists
to fix, reintroduced one layer down. The GIL serialises execution; it does not make one
call stack the only one.

**Two concurrency guards in `App`, and they are not the same guard.**

- **`_lock` (an `RLock`)** serialises renders against each other. Before the push path a
  second renderer was impossible by construction; now a worker's `update()` renders on
  that thread while the shell renders on its own. The damage is not a torn *frame*, which
  would merely look wrong — `render_tree` reads `self.ctx` once and threads it down the
  walk as a local, so a walk in progress cannot be corrupted. What interleaves is the
  **pair**: `self.ctx` belongs to whichever render assigned it last and `self.last_tree`
  to whichever finished last, and those are not the same render. A view whose *shape*
  depends on state then leaves the shell painting a tree whose handlers were registered
  into a `RenderContext` nothing points at any more — the app looks completely right and
  some of its buttons silently do nothing. §15.1's failure shape exactly.
- **`_rendering` (a bool)** guards the *re-entrant* case, which `_lock` deliberately does
  not: a `view()` that calls `update()` (arming a timer, kicking off a first fetch)
  re-enters `_render_all` from inside `_run_view`. The nested render is skipped, which is
  not a compromise — the state was mutated *before* the callback fired, so the render
  already in progress is the one that will contain it — and it turns an unconditional
  `update()` inside `view()` from unbounded recursion into a no-op. Found by writing this
  section's own demo app, which did exactly that on its first draft.

### 34.3 The two backends, and why one was nearly free and the other was not

**Native.** `Command::FrameReady { tree_json }` on the `mpsc` channel §15.2 already had,
sent by the sink `run()` installs *before* the interpreter exists. Handled exactly as
`Command::Reload` handles a new tree, and for the same reasons: scroll offsets, IME focus
and the shape cache are keyed by node id or by (content, style), so a fetch landing must
not scroll the user's list to the top or close their keyboard. Coalesced like `TouchMove`.
It also calls `zkite_pybridge::drain_log`, which is new and is what makes a `print()`
inside a worker thread reach logcat at all — every other path drains as a side effect of
the call it made, and this path never made one.

**WebView.** The asymmetry §15.2 recorded, from the other end: this backend has *no Java
call to return through*. `phase_1/devshell` already solved it for the reload socket by
parking HTML in a `Mutex` that Java polls on a 120 ms timer, so there the sink is four
lines into `PENDING_HTML`. **`phase_2/appshell` — the shipped shell — deliberately does
not do it that way**: a UI-thread timer running forever is a battery cost a release app
should not pay for a feature it may never use. It makes a genuine cross-thread call —
`GetEnv`/`AttachCurrentThread` → `ZKiteActivity.onFrameFromPython(String)` →
`mainHandler.post` → `evaluateJavascript` — and is completely idle between frames.

Three things that would each have been a device-only bug:

- **The class is resolved at boot, on the UI thread, and cached as a global ref.**
  `FindClass` resolves against the class loader of the *calling* frame, and a thread
  attached later from native code has no Java frames at all, so it falls back to the
  system class loader, which cannot see application classes. Looking up
  `dev/zkite/appshell/ZKiteActivity` from a Python worker would have failed with
  `ClassNotFoundException` on a real device while working in any test that called it from
  Java.
- **Attach and detach per call, and only when this thread was not already attached.**
  `GetEnv` decides which case it is. Leaving a Python worker attached means an ART
  complaint every time such a thread exits; detaching a thread the JVM created would be a
  bug.
- **`liveInstance` is cleared in `onDestroy` guarded on identity.** A configuration change
  destroys the old Activity *after* creating the new one, so an unguarded clear would
  blank the field the new instance just claimed and kill the push path for the rest of the
  process.

The whole path fails **open and loudly**: any failure to cache the target downgrades
pushing to a no-op and leaves an app that repaints on touch exactly as it did before.

### 34.4 An app could not ask for a permission at all

`zkite build` generated a manifest with **no `<uses-permission>` element and no way to add
one**. That was a deliberate phase-2 decision (phase_2/PLAN.md §1.2: a shipped app must
not carry the Dev Shell's `INTERNET`, which existed only for the reload socket) and
`manifest_gen.rs`'s own test asserted it. The consequence had never been noticed: **the
first ZKite app to load remote data would have failed inside `socket`, on a device, with
nothing anywhere naming the manifest as the cause.**

```toml
[tool.zkite.android]
permissions = ["INTERNET"]
```

Bare names are qualified (`INTERNET` → `android.permission.INTERNET`) because both
spellings are what people write and a manifest entry Android silently ignores is the worst
of the three possible behaviours; anything already containing a `.` passes through, so a
vendor or custom permission is not rewritten. Validation is deliberately a character check
rather than an allow-list of known permissions — that set grows every Android release.

**The default is the half that is pinned**: an app that asks for nothing produces a
byte-identical manifest to every build before this existed.

### 34.5 Why HTTPS failed, and it is not what the artifact suggested

The prediction from reading the artifact was right about the symptom and wrong about the
fix. The vendored OpenSSL is configured with the default prefix, so
`X509_get_default_cert_file()` is the literal string `/usr/local/ssl/cert.pem` baked into
`libcrypto_python.so` — a path that has never existed on a phone. So
`ssl.create_default_context()` succeeds, loads **zero** certificates, and the failure
surfaces at connect time as `CERTIFICATE_VERIFY_FAILED: unable to get local issuer
certificate`, which reads like a problem with the server.

The obvious fix is `SSL_CERT_DIR`, pointed at Android's own store. **That was built, run
on the device, and it does not work** — the first fetch failed with exactly the error
above, with the trust store correctly resolved and logged.

The reason, found by pulling the device's 145 certificates and testing them on the host
with **identical bytes in all three arms**:

| trust source | result |
|---|---|
| `capath`, the device's own filenames | **`CERTIFICATE_VERIFY_FAILED`** |
| `capath`, same certs renamed to the modern subject hash | **ok 200** |
| `cafile`, same certs concatenated into one bundle | **ok 200** |

Android's CA directories *are* OpenSSL `capath` directories, but their filenames are the
**old** subject hash — `X509_NAME_hash_old`, the MD5-based OpenSSL 0.9.8 scheme. Measured
directly on one certificate: the device calls it `01419da9.0`, and `openssl x509
-subject_hash` says `8d89cda1` while `-subject_hash_old` says `01419da9`. A modern
OpenSSL's `by_dir` lookup does not find a certificate under that name.

So `runtime.py` **concatenates the device's own certificates into one PEM bundle** and
points `SSL_CERT_FILE` at it. Rehashing would mean reimplementing RFC 5280 name
canonicalisation in Python to reproduce `X509_NAME_hash` exactly; concatenating needs no
crypto at all. The certificates still come from the device, so an Android update to the
trust store is picked up rather than pinned — which is the whole reason for not shipping a
bundle of ZKite's own. It is built beside the unpacked app and rebuilt only when the source
directory's `(mtime, entry count)` moves, so the steady state is two `stat` calls.

**This is host-reproducible**, which matters more than the fix: it needs no phone, only
the device's certificate files, and it was found by asking the real OpenSSL rather than
reading about `capath`.

### 34.6 Verification on the host

**Suites.** `zkite_py` 169 → **174**, `zkite_pybridge` 16 → **18**, `zkite-bundler` 24 →
**27**, webruntime e2e 37 → **39**. `cli` 40, `zkite_renderer` 44, `twcore` 57 — unchanged.
`pybridge` under `--test-threads=1` throughout, per §29.5.

**70 states byte-identical to `HEAD`** — every conformance fixture through
`build_manifest` (which is what a backend actually sees, §33.5's lesson) plus every demo
app through a real `App` in both modes. Two normalisations, both forced and both narrow:
every file's *directory* (HEAD is checked out under a scratch path) and `app.py`'s own
*line numbers* (it is the file this session edited, and a traceback names them). Nothing
else — and the comparator was **shown to discriminate** by changing one class string in
`widgets.py`, which moved 5 states.

**That check earned its keep immediately.** A first draft moved the render bodies into
wrapper methods, which changed the on-device error overlay's top frame from `in
render_tree` to `in _render_tree_body` — `view()`'s traceback is user-facing, and every
helper frame between the entry point and `_run_view` appears in it by name. The lock and
the flag are now held inline around the original body for exactly that reason.

**Conformance: 29 fixtures x 2 dprs, every number identical to §33.5's table**, including
`select_closed` 0.73/1.18 and `select_open` 0.91/1.27 at dpr 1. That is the expected
result rather than a lucky one — the manifest is byte-identical and `zkite_renderer`,
`twcore` and the conformance crate are untouched — but it was run rather than argued.

**Ten negative controls, all discriminating:**

| control | result |
|---|---|
| the tree push callback is never invoked | 4 tests fail |
| the html push callback is never invoked | 2 tests fail |
| no render lock | 1 test fails |
| no re-entrancy guard | 1 test fails |
| `IN_PULL` never set (a pulled frame also pushes) | 1 test fails |
| `runtime.py` never registers the push callback | 1 test fails |
| `_push_frame` ignores the injected hook | 1 test fails |
| the frame hook is never installed in `__main__` | 1 test fails |
| `<uses-permission>` is never emitted | 1 test fails |
| a bare permission name is not qualified | 2 tests fail |

**One of them did not discriminate at first, and that is the more useful half** (§27.1,
§28.4's rule). The render-lock control **passed** against the original concurrency test,
because that test asserted every handler was still dispatchable — and `ctx` is read once
into a local, so a concurrent render cannot corrupt a walk in progress and node ids are
stable anyway. The test was wrong, not the lock. Rewritten to assert **coherence of the
pair** — every node the *stored tree* says has a handler must be dispatchable against the
*stored ctx*, sampled under the app's own lock — it now fails without the lock and passes
with it.

**A deterministic failure this session caused, found and fixed.** Adding a `zkite_pybridge`
test whose app declared `@store class S` broke
`a_touch_on_a_synthesized_thumb_resolves_to_the_control_itself` **5 runs out of 5** under
`--test-threads=1` — deterministic, which is what distinguishes it from §29.5's genuinely
intermittent one. `zkite._store`'s instance registry is process-global and keyed by
qualified name, every test app loads as module `main`, so the second app to declare
`main.S` was handed the first app's instance of a class with entirely different fields;
its `view()` raised, the error tree has no `Switch` in it, and the failure surfaced three
layers away. Renaming one identifier is the whole fix and flipping it back is the control.
**A new test that boots an app through CPython needs a store name no other test uses.**

### 34.7 The Moto G54 run, and an emulator run beside it

**Both were run.** The phone dropped off USB partway through the first attempt, the rest
was done on the emulator, and then the phone came back and everything was re-run on it —
so the results below are real silicon unless labelled otherwise. §29.8 remains the
standing reason an emulator alone would not have been enough.

**Device, asserted rather than assumed:** moto g54 5G, `arm64-v8a`, Android 15, 1080x2400
@ 400 dpi, `-s ZD222H9YDS` on every call (**two devices were attached** for most of this,
so §24.5's warning was live). The shell's own boot lines agreeing: `density 2.5, 60Hz ->
16.67ms frame period`, `PowerVR BXM-8-256 (IntegratedGpu, backend Vulkan)`, `67 native
extension modules available`, and the new `zkite: TLS trust store
/apex/com.android.conscrypt/cacerts -> .../zkite-cacerts.pem`. Artifact checked on all
three surfaces before installing (§15.6.1) **with §33's own control** — a string that
should not be there returned 0. Zero crash markers anywhere; `pidof` and `mCurrentFocus`
asserted before and after every case.

**The instrument is screenshots, and every number is arithmetic out of a ~70-line
pure-Python PNG reader** (§28's rule). The measurement is *self-controlling*: the bands
that must **not** change are read from the **same pair of screenshots** as the band that
must. A stray touch, a dimming display or an unrelated animation shows up as a nonzero
title band, and never did.

| arm (Moto G54) | title band | ticks band |
|---|---|---|
| native harness, 5 s apart, **no input** | **0** | **1045** |
| native harness, **sink not installed** (negative control) | **0** | **0** |
| …that same control, after **one tap** | — | **3008** |
| `zkite build apk --backend native`, 5 s apart, no input | **0** | **697** |
| `zkite build apk --backend webview`, 6 s apart, no input | **0** (app-bar also **0**) | **1178** |

**The control is the pair, and its third row is the informative half.** Without the sink
nothing repaints at all; then a single tap reveals **3008** pixels of accumulated ticks —
so the app was alive and its state had been advancing the whole time, and the freeze is
purely a *painting* gap. That is what a negative control is for, and it is what the claim
rests on.

**The network, on real silicon, both backends, as real `zkite build apk` apps** (not the
harness): `status: ok 200` with a live body from `https://example.com`, ticks still
climbing, on **`--backend native`** and on **`--backend webview`**. The WebView row is the
one that matters most — the JNI hop is the only genuinely new mechanism in this section —
and `push path armed: a background @store.update() will repaint` in its log is the class
and method having resolved on the UI thread as §34.3 requires.

**The permission pair.** `aapt2 dump badging` against the real binary AXML:
`uses-permission: name='android.permission.INTERNET'` with the key, and **no
`uses-permission` line at all** without it. Without the permission the request does not
fail — it **hangs**, which is worse and is filed in §34.11.

**Emulator (x86_64, Android 14, SwiftShader, density 2.75, AVD `zkite_phase0`).** Every
arm above was also run there and agreed — native harness `0`/`3351`, control `0`/`0` then
`4167` on a tap, WebView `0`/`2870`, and `ok 200` on both backends. Two things make that
worth recording rather than discarding: it is a **different OS version, ABI and
rasteriser**, and it is where both bugs in §34.8 were found. It also makes the CA finding
cross-version — Android 14 names its certificates by the same **old MD5 hash**
(`01419da9.0`, modern hash `8d89cda1`), with 134 certificates rather than 145.

**A trap this run walked into, and it is §8.7's seen from a new angle.** The control was
*first* measured with `uiautomator`, which reported the tapped `status` as never
changing — impossible, since the tap dispatched and `ZKITEPERF tap-to-paint=10.5ms` says a
frame was painted. `push_a11y_update` skips `update_if_active` while Android's
accessibility framework is disabled, and `uiautomator` only enables it around its own
dump, so in a build where frames stop being produced the dump can hand back a tree from
before. **Screenshots are ground truth for app state; the a11y tree is ground truth for
roles, names and bounds, and only while frames are still flowing.** Redoing the control
with screenshots is what produced the table above.

**One filed gap the run exposed**: a `Text` node wide enough to overflow the viewport
disappears from the accessibility tree entirely — `body:` renders on screen and is absent
from a well-formed dump, while the same node with short content is present. A screen
reader would not announce it.

### 34.8 Two bugs the device runs found, both in `zkite build apk` rather than in this work

Neither would have been visible from the harness, and that is the point: **every device
run in this document before now exercised a stdlib and a shell that the shipped build path
does not produce.**

**1. `stringprep` was pruned, so no built app could resolve a hostname.** The first app
ever built with `zkite build apk` that made a network request failed with
**`LookupError: unknown encoding: idna`** — a codec-registry error naming neither the
module nor the pruner. `socket.getaddrinfo` encodes every hostname through the `idna`
codec, `encodings/idna.py` does `import stringprep`, and `stringprep` is its own top-level
module that nothing in app code or `zkite_py` imports by name. `prune_stdlib.py` keeps
`encodings` as a package **without walking its imports**, so the reachability pass could
never see it — exactly the gap `ALWAYS_KEEP` exists for. Measured in the artifacts rather
than inferred: present in the harness APK, absent from both `zkite build apk` outputs. One
line in `ALWAYS_KEEP` fixes it, and the fix is verified by the same app then returning
`ok 200` on both backends.

**2. `zkite build apk --backend native` silently staged a shell built by a control
experiment.** That command does not compile anything — it copies a prebuilt `.so` out of
`phase_3/gpu_harness/artifacts/`, and the most recent thing to write that directory was
the negative-control build with the frame sink disabled. So the "real app" under test had
**no push path**, and the symptom (a frozen counter) looked exactly like a network hang.
**The `strings` check could not tell the two shells apart** — `install_frame_hook` is
untouched by the control patch, so `_zkite_native_frame_ready` is present in both. This is
§33's artifact-control lesson and §15.6.1's "the build succeeded has been wrong for four
distinct reasons", now with a fifth: *the artifact was fine and came from the wrong
build*. The only reliable tell was the `.so`'s mtime against the build log.

### 34.9 What is still owed

The device work is done on real silicon — both backends, both as the harness and as real
`zkite build apk` apps, with a discriminating negative control. What is left:

- **`on_input` has no device evidence on either machine.** It is covered by the measured
  Chromium contract (§34.1) and pinned in `e2e.mjs`, but nothing has typed into a
  `TextField` on a device since it landed. The push path was where the device time went.
- **`zkite dev` was not run against any of this**, on either backend — so §15.3's standing
  "only the counter app was ever reloaded" gap now has a push path worth reloading too.
- **The CPU fallback path** is still unexercised. The emulator's SwiftShader adapter is
  reported as `Cpu` by wgpu, but the code path taken is the ordinary GPU one.
- **One phone, one ABI, one density** for the arm64 results, as every device run in this
  document.

### 34.10 What this deliberately does not do

- **It does not retime native's `change`.** §34.1. **This is an owner decision**, of
  exactly §32.3's shape: `on_change` on a `TextField` fires per keystroke on native and on
  blur on the WebView. Making native match the WebView follows §32.3's precedent and
  breaks live-typing apps on the default backend; making the WebView match native means
  `on_change` and `on_input` become the same event on a text field and `zkite-runtime.js`
  changes for the first time since §31. A third reading is that ZKite's own meaning of
  `on_change` — "the value changed", which is what it means on `Switch`, `Checkbox`,
  `Radio`, `Slider` and `Select` — makes the *WebView's* `TextField` the outlier.
- **No timer or scheduling API.** `threading.Timer` is what the demo app uses and it
  works; ZKite offers nothing of its own, and a framework-owned scheduler is what an
  animation driver (Queue F) would want anyway.
- **No `async`/`await`, no event loop.** Threads only. An `asyncio` loop inside the
  embedded interpreter is a separate design question and nothing here forecloses it.
- **No request helper, no JSON fetch, no image loading.** `urllib` is the stdlib and it
  works; `Image` still does not paint (§15.1), which is the other half of "load remote
  data" and is untouched.
- **No back-pressure policy.** A worker thread updating in a tight loop will queue
  `FrameReady`s faster than they can paint; coalescing bounds the *work* but not the
  channel. Nothing throttles the producer.
- **No lifecycle awareness.** A pushed frame arriving while the app is backgrounded is
  handled (native paints into a surface that may not exist; the WebView drops it if the
  Activity is gone) but is not *managed* — nothing pauses a timer on `onPause`.

### 34.11 Known gaps

- **`on_input` has no device evidence**, and `zkite dev` was not run against any of this
  (§34.9).
- **The `on_change` timing divergence on a `TextField`** (§34.1) — now in §15.1, awaiting
  an owner decision.
- **A `Text` node wider than the viewport is absent from the accessibility tree** (§34.7).
  Pre-existing, found here, not fixed.
- **`uiautomator dump` is unsafe as an instrument once frames stop flowing** (§34.7). The
  device-notes section of any handoff should say so.
- **The frame hook is duplicated three times** — `zkite_pybridge`, `phase_2/appshell`,
  `phase_1/devshell` — because phases 1 and 2 cannot depend on a phase-3 crate that pulls
  in `zkite_renderer` and a GPU stack. It is the same duplication `with_gil`,
  `call_str_fn` and `ensure_initialized` already are between those files, and the half
  that must not diverge (*when* a frame is pushed) is genuinely shared in `runtime.py`.
  Nothing tests that the three copies agree.
- **The CA bundle is rebuilt on a `(mtime, count)` heuristic.** An Android update that
  replaced a certificate without changing either would not be picked up until the next
  reinstall. The bundle is also ~218 KB of app storage.
- **Nothing verifies the bundle is well-formed** beyond it being non-empty — a
  garbage-but-present PEM would produce the same silent verification failure this section
  fixed.
- **The JNI hop has no host test at all** — `phase_2/appshell` has no host test harness,
  so its only evidence is the device runs in §34.7.
- **Without `INTERNET`, a request hangs rather than raising**, past its own `timeout=`
  (which bounds connect and read, not DNS), and while it hangs **the push path stops
  delivering frames** even though the app still repaints on touch and its state keeps
  advancing. Native stacks taken with `debuggerd` show the render thread parked on an
  empty command channel and a Python timer thread blocked in `_PyMutex_LockTimed`; the
  full mechanism was **not** run to ground and is filed rather than diagnosed. It is a
  pathological path — the permission is now declarable and the fix is to declare it — but
  "a blocked worker can wedge pushed frames" is the general shape and it deserves its own
  investigation.
- **`zkite build apk` consumes prebuilt shells from `<shell>/artifacts/`, and nothing
  checks they came from the current source** (§34.8 item 2). A `strings` check does not
  discriminate. There is no build-id, no hash, and no warning.
- **`_rendering` skips a nested render rather than deferring it.** If a `view()` mutates
  state and the mutation is *not* reflected in the part of the tree already walked, that
  frame is stale until the next render. Correct for the arming case it was written for;
  not a general answer.
- **One phone, one ABI, one density, the GPU path.** The CPU fallback was not forced on.
- **§29.5's intermittent `zkite_pybridge` failure is unchanged**, and everything here was
  run under `--test-threads=1`.

## 35. `Image` paints — and the atlas that outlived its renderer

Queue A item 1, and the last of §15.1's "missing rendering features on native" that a
user meets on their first screen. `Image` has reserved correct layout space since phase
1 and drawn nothing inside it; it now decodes and paints, **on both backends**, with all
five CSS `object-fit` modes, verified live on the Moto G54 on the GPU path, the CPU
fallback path, and as real `zkite build apk` apps on both backends.

Two things found along the way are worth more than the feature, and one of them is a
bug this session introduced and then caught only because it was run on a phone.

### 35.1 What was actually wrong, and it was not what the plan said

§18.6 recorded `Image`'s blocker as a staging problem: *"`build_cmd.rs` zips the app
directory into `assets/app.zip`, so a `logo.png` next to `main.py` lands inside a zip
with no path to hand a decoder."* **Both halves of that are wrong**, and §35 found out by
reading the code rather than the note:

- `copy_app_source` copied `*.py` **and nothing else**. A `logo.png` never reached the
  zip at all — it was dropped at staging, and the zip was never the obstacle.
- A file that *does* reach the zip is not trapped in it. `ZKiteActivity` unpacks
  `app.zip` into `appDir` on first launch, on **both** shells
  (`unpackOptionalZipAssetOnce`), so a staged asset lands on the filesystem with an
  ordinary path.

So the fix is an extension allowlist in `copy_app_source`, not a zip-reading decoder.
§18.6 is corrected in place.

**The second correction is bigger: this was never a native-only gap.** The WebView shell
loads its page from `file:///android_asset/webruntime/index.html`, so a relative
`<img src="logo.png">` resolves to `file:///android_asset/webruntime/logo.png` — an asset
that does not exist, because the app's own files live in `getFilesDir()/app`. **Images
were broken on both backends**, and §15.1 had it filed as a divergence. It was a plain
gap wearing a divergence's clothes.

### 35.2 The shape: decode is a crate, geometry is shared, paint is per-backend

`zkite_renderer::image()` carries the `src` through to `PositionedNode.image` and stops
there. It never opens a file. That keeps a PNG decoder out of the dependency tree of the
crate whose whole job is "class strings to positioned boxes", and it keeps the answer to
"what pixels" in the layer that can hand them to a GPU atlas.

Its consequence is worth stating because it is a deliberate difference from a browser:
**layout never depends on the bitmap.** A missing file, a corrupt file and a 4000x3000
photo lay out identically, and a decode finishing on a later frame cannot reflow the
page. ZKite has no intrinsic sizing for images; an `Image` is sized by its classes like
any other box.

`zkite_imagestore` is a **new crate**, and it is a crate rather than a `gpu_harness`
module for two independent reasons, either sufficient: `gpu_harness` does not build for
the host (§15.5), so nothing in it can carry a test — and a decoder is exactly where an
untested channel swap survives for months — and the *conformance harness* needs the
identical decode to render an image fixture with `vello_cpu`. Sharing it is what stops
the harness measuring a second decoder against Chromium and reporting the result as
ZKite's.

`zkite_renderer::object_fit_rect` is shared by **four** consumers that must agree — the
GPU paint path, the CPU paint path, the conformance harness, and its own tests — the same
argument `icons.rs` makes for sharing geometry (§18.3). It lives in `zkite_renderer` for
the same reason `slider_value_at` does: that crate builds for the host and can be tested.

`png` and `zune-jpeg` are the decoders. Pure Rust is a hard requirement, not a
preference: this crate cross-compiles to four Android ABIs, and `cargo tree` confirms
`png` resolves `flate2` onto `miniz_oxide` with no C anywhere.

### 35.3 What an author gets

```python
Image("w-32 h-24 rounded-lg", src="logo.png", alt="Our logo")
```

`src` is a path relative to the app directory, or a `data:` URI. `alt` now reaches the
accessibility tree as a real accessible name — it had been in `Image._tree_attrs` since
phase 1 and was read by nothing. **An `alt=""` image stays deliberately unnamed**, which
is HTML's "decorative, skip me" and makes native agree with what the WebView already did
with the identical attribute.

`object-fill`/`-contain`/`-cover`/`-none`/`-scale-down` all parse (new in `twcore`) and
all paint. `rounded-*` clips the bitmap, so `rounded-full` is an avatar.

A `data:` URI is supported because it is the one `src` form needing no staging on either
backend, which is what lets the conformance harness compare identical bytes.

Path confinement: a `src` that tries to leave the app directory is refused on **both**
backends — lexically in `zkite_imagestore::resolve`, and with `getCanonicalPath` in the
WebView's `serveAppFile`. A refused path takes the ordinary failure route.

**Failure is cached.** A missing or corrupt `src` is attempted once and remembered; the
node paints as the empty box it always did. Without the memo a typo'd `src` costs a
failed `open()` per node per frame at 60fps, and the app that most needs to stay smooth
is the one with the bug in it.

### 35.4 A real divergence, found by the fixture that was built to measure something else

The `image_object_fit` conformance fixture reported **14.88%** on its first run. None of
it was about images:

1. **The fixture was over-constrained.** Five 48px boxes plus four 8px gaps plus 16px
   padding overflow a 280px row by 8px, flex-shrink engaged, and the two backends
   resolved the overflow differently — boxes at x=4..51 on web against x=8..53 on
   native. Sized to 288 so nothing shrinks. *A fixture that measures a second bug through
   the hole left by the first reports a number that means nothing.*
2. Underneath it was a **genuine silent divergence, and native was the correct one.**
   `Widget.classes()` emits `implied_classes` first and the author's string second.
   Native resolves in string order, so `Image("object-contain")` was contained. The
   WebView resolves by **CSS cascade**, which has nothing to do with attribute order —
   and driving the real vendored Tailwind bundle in real Chromium shows
   `class="object-cover object-contain"` and `class="object-contain object-cover"`
   **both** compute `object-fit: cover`, because `.object-cover` is emitted later in the
   stylesheet. So `Image("object-contain")` was **letterboxed on native and cropped on
   the WebView, from identical user code**, and `object-contain` was *unexpressible* on
   the WebView for as long as `Image` carried the implied class.

   Fixed in `zkite_py`, where both backends get it: an authored `object-*` **replaces**
   the implied `object-cover` rather than fighting it in the cascade. With one
   `object-*` in the string there is no cascade question to get wrong on either side.
   14.88% → **0.00%**.

The old `image` fixture could not have found this. It drew a **1x1 red pixel** scaled up
by its box — the one image whose diff is blind to a vertical flip, a horizontal mirror, a
transposed axis and every wrong `object-fit` mode, all of which produce a byte-identical
flat rectangle. It is now a 2:1 four-quadrant bitmap. **Measured**: with
`object_fit_rect` forced to always return the `Fill` rect, `image_object_fit` reports
23.94% while `image` stays at 0.07% loose — the single-box fixture is nearly blind to the
bug the five-box one exists to catch.

### 35.5 The bug that only a phone could find: an atlas that outlived its renderer

Every image was **an opaque black rectangle** on the Moto G54 while the identical code
was pixel-perfect against Chromium on the host.

`vello_cpu` samples an `ImageSource::Pixmap` directly. `vello_hybrid`'s wgpu backend
matches only `ImageSource::OpaqueId`, so a bitmap must be uploaded into the renderer's
texture atlas and referred to by the `ImageId` that comes back. `ImageStore` recorded
those ids so it would not re-upload.

**But the atlas texture lives inside `vello_hybrid::Renderer`, and
`render_thread::reconfigure` builds a brand-new `Renderer` on every surface resize —
twice on every launch**, because Android delivers a `Resize` straight after `NewWindow`
(the code says so in its own comment, three lines below where the bug was). So every id
recorded during the first frame pointed into a texture that no longer existed by the
second. The scene still resolved the id, sampled a slot nothing had ever written, and
painted black. Not a crash, not a missing image — a confident black rectangle.

`phase_3/repro_atlas_lifetime` had already established that *glyphs* survive this,
because `vello_hybrid` owns their residency and re-establishes it. Images are the case
where the bookkeeping is **ours**, so the invalidation had to be ours too.

The fix evicts in `reconfigure`, **before** the renderer is replaced, because
`destroy_image` needs the renderer that owns the slots — and the slots are allocated in
the shared `Resources`, which outlives it, so dropping the handles without freeing them
leaks an atlas slot per image per resize (and a resize is what an IME show/hide does).

Two things about how this was found:

- **`invalidate_gpu` already existed, written defensively, with a doc comment asserting
  that nothing could call it** — "`gpu_harness` probes the GPU exactly once per process
  and never rebuilds the `Renderer`". That sentence was false when it was written. The
  method was right and its justification was wrong, which is a worse failure than not
  having written it, because it reads as having considered the case.
- **The host could not have caught it.** The conformance harness renders one frame and
  never resizes; `repro_atlas_lifetime` rebuilds the renderer but tests glyphs. The
  bisection that localised it was cheap and worth reusing: the CPU fallback path shares
  the decode and the geometry and bypasses the atlas entirely, so *"correct on
  `vello_cpu`, black on `vello_hybrid`"* isolates the atlas in one build.

### 35.6 Known gaps

- **No network image loading.** Deliberately out of scope: it needs placeholder and error
  states, a disk cache and a fetch policy, and folding it in would have meant shipping
  the local path unverified. The async half it was waiting for exists (§34), so this is
  now a self-contained piece of work rather than a blocked one.
- **No intrinsic sizing.** An `Image` with no `w-`/`h-` is a zero-size box, where a
  browser would size it from the bitmap. §35.2 records why; it is a real difference an
  author will meet.
- **`object-position` is not implemented** — every mode centres, which is CSS's own
  `50% 50%` default.
- **Asset staging is flat and top-level only**, matching the `*.py` behaviour that
  preceded it. `assets/logo.png` in a subdirectory is not staged, on either the
  `zkite build` path or the harness path.
- **The allowlist is PNG and JPEG.** No WebP, no GIF, no SVG-as-`Image`, no AVIF.
- **The path check is lexical, not resolved** (`zkite_imagestore::resolve`), so a symlink
  inside the app directory pointing outside it would be followed. Accepted rather than
  overlooked: `app_dir`'s contents come from this framework's own zip unpack and `zkite
  dev`'s socket push, neither of which creates symlinks, and canonicalising would cost a
  syscall per node per frame. The WebView side *does* canonicalise, so the two backends
  differ here in the safe direction on one and not the other.
- **Nothing bounds the cache.** Decoded pixmaps live until the process dies; there is no
  eviction, no memory budget and no count limit. An app cycling through many distinct
  `src` values grows without limit. §13's `ZKITEMEM` note asked for a re-measure when
  `Image` landed and **that has not been done** — the fixture holds two small bitmaps.
- **JPEG has no device evidence.** The decoder is wired and unit-tested by magic-byte
  sniffing, but every fixture and every device run used PNG.
- **The emulator was not run this session.** §34 is the standing counter-example for why
  that is a real omission: both of its bugs were found there.
- **`zkite dev` was not run against any of this**, on either backend — the same gap §34
  left open.

### 35.7 Verification

**Host suites.** zkite_py 174 → **175**, cli 40 → **42**, renderer 44 → **50**, pybridge
18 → **20**, and a new `zkite_imagestore` at **8**. bundler 27 and twcore 57 unchanged.
webruntime e2e **39/39**.

**Conformance: 30 fixtures x 2 dprs** (29 before; `image_object_fit` is new). `image`
0.07%/0.00%, `image_object_fit` **0.00%/0.00%**. **Every pre-existing number is identical
to §33.5's table at both dprs** — `button` 1.78/3.20, `text` 5.00/6.58, `textfield`
1.89/4.07, `typography` 9.95/13.68, and the rest.

**27 widget forms byte-identical to `HEAD`**, and the comparator was shown to
discriminate (perturbing `Row.implied_classes` moved exactly `row` and `nested`). *One
instrument trap:* a stale `__pycache__` silently produced a wrong comparison until it was
cleared — worth knowing for any future run of this check.

**Negative controls, all discriminating.** `object_fit_rect` against real Chromium:
15/15 agree; swapping `min`/`max` drops it to 10/15, and collapsing `scale-down` onto
`contain` to 14/15 — the latter confirming that a *small bitmap in a large box* is the
only case separating those two, so a single-case probe would have missed it. On
`zkite_imagestore`, four breaks (premultiplication removed, R/B swapped, failure memo
removed, escape checked on final depth) each failed exactly one test and no others. On
the CLI, the pre-§35 behaviour and the naive "copy everything" fix each fail the staging
test on the assertion written for them. In conformance, painting no image at all reports
`image` **44.41%** / `image_object_fit` **39.93%** — which is what the gap looked like
before this section.

**Device — Moto G54 5G, arm64-v8a, Android 15, PowerVR BXM-8-256, density 2.5.**
`demo_app/images.py`; every number is arithmetic out of a pure-Python PNG reader
(`conformance/pngtool.py`, kept this time rather than thrown away as §34's was). Focus
asserted before and after every measurement.

Row 1's five boxes, 56dp (140 device px) with a 40x20 source:

| mode | expected | GPU harness | CPU fallback | `zkite build apk` native | `--backend webview` |
|---|---|---|---|---|---|
| `object-fill` | 140x140 | 140x140 | 140x140 | 140x140 | 140x140 |
| `object-contain` | 140x70 | 140x70 | 140x70 | 140x70 | 140x70 |
| `object-cover` | 140x140 | 140x140 | 140x140 | 140x140 | 140x140 |
| `object-none` | 100x50 | 100x50 | 100x50 | 100x50 | 100x50 |
| `object-scale-down` | 100x50 | 100x50 | 100x50 | 100x50 | 100x50 |

Orientation (red TL, green TR, blue BL, yellow BR) correct on every arm — a flat bitmap
would have made a flip invisible. The failure row (missing file, escaping path, empty
`src`) paints **0** bitmap pixels on every arm and the app keeps running. The device log
reports `images: 2 decoded, 2 failed, 2 resident` — the two failures being exactly the
missing file and the refused escape.

**This is the first time the native CPU fallback path has been run on a device**, closing
part of a standing gap §18.7 and every handoff since have carried. It was run because it
was the cheapest bisection for §35.5, not for its own sake.

**The WebView control is decisive**: with `serveAppFile` stubbed to return `null`, the
same app drops from **46011** bitmap-coloured pixels to **42**.

**Artifacts were checked before installing**, per §15.6.1 and §34.8: `app.zip` listed out
of the APK (3 entries, the two PNGs and `main.pyc`), `classes.dex` byte-compared against
the freshly built one, and the `.so` mtime against the build log. The negative-control
dex was rebuilt from clean source immediately afterwards and its size re-checked
(14284 → 14972), because that is precisely the §34.8 trap.

## 36. The rest of `twcore`'s v4 defaults, audited — and the one table nobody named

§28.6's last-but-two known gap, standing unchecked for eight sessions and named in every
handoff since:

> **Nothing re-checks that `twcore`'s remaining v4 defaults are v4's.** §9.6's own list
> also names the **spacing scale derivation**, which nothing here measured and which
> would show up as every padding and gap being subtly wrong — the same shape, still
> unchecked.

**Status: audited in full, and the worry in that paragraph was misdirected.** All 104
values `twcore::Theme` can produce were compared against the exact
`@tailwindcss/browser` bundle `phase_1/webruntime/vendor/` pins, driven in real
Chromium. **The spacing scale is correct and always was.** What was still v3's is the
**radius scale**, which nobody had named, carrying the identical rename §28.1 found in
the shadows. Fixed, guarded by a host test with two negative controls, and given the
conformance fixture the scale never had.

**Why this and not the queue's top item.** The release pipeline (Queue B) is the biggest
thing left and it is where the weeks are, but it turns on two decisions filed as the
owner's (`zkite` vs `zkite-cli`, §23.8). This is half a session, needs no device and no
decision, and it sits *underneath* the rest of the queue: every conformance number in
this document, and every device measurement taken against one, is quoted in dp that
`twcore` resolved. Auditing the table that produces them is worth doing before adding
more measurements on top of it — and §28.1, §9.5, §9.6 and §35.4 are four instances of
the same class of bug, which is enough to stop treating it as bad luck.

### 36.1 The finding: the radius scale was v3's, and the spacing scale never was

Measured, not read — `phase_1/webruntime/test/probe_theme_defaults.mjs`, the same
method §9.6, §28.1, §33.1, §34.1 and §35.4 used. It builds one element per class,
inlines the vendored bundle, lets the JIT compile, and reads `getComputedStyle` back:

| group | values | result |
|---|---|---|
| **spacing** (`p-*`) | 41 | **all agree**, incl. `p-0.5`, `p-4.5`, `p-13`, `p-17`, `p-px` |
| **radius** (`rounded-*`) | 11 | **1 wrong, 2 unparsed** — below |
| **font size + line height** (`text-*`) | 26 | all agree |
| **font weight** (`font-*`) | 9 | all agree |
| **leading** (`leading-*`) | 9 | all agree, incl. the numeric fallthrough onto spacing |
| **tracking** (`tracking-*`) | 6 | all agree |
| **breakpoints** (`sm:`…`2xl:`) | 5 | all agree, checked at the boundary and one px below |

The radius rows:

| class | `twcore` (v3) | Tailwind v4, as Chromium computes it |
|---|---|---|
| `rounded-xs` | **unparsed** | 2px |
| `rounded-sm` | 2px | **4px** |
| `rounded-4xl` | **unparsed** | 32px |
| `none`, `rounded`, `md`, `lg`, `xl`, `2xl`, `3xl`, `full` | — | already v4's |

**v4 shifted every name in the scale one step**, exactly as §28.1 found it had done to
the shadows: v4's `rounded-xs` is v3's `rounded-sm`, and v4's `rounded-sm` is v3's bare
`rounded`. So `rounded-sm` on the WebView backend and `rounded-sm` on the native one
were not the same corner drawn two ways — the native one was **half** the radius, on
every background, border, ring and shadow that node carried, since all four read the
same `Corners`.

The two unparsed names fail differently and more quietly. An unknown utility is dropped
silently, so `rounded-xs` and `rounded-4xl` rendered a **square** corner where the
WebView rounds one — a bigger pixel difference than the wrong value, with no class of
error a user could distinguish from "I forgot the class".

`Theme::radius` is the only lookup `parse.rs` performs for the whole `rounded-*` family,
corner variants included, so the table was the entire fix. `rounded-tl-xs` and
`rounded-b-4xl` are pinned by the new test anyway — that the corner path shares the
scale is a property worth asserting rather than assuming.

**`full` is deliberately left at 9999.** v4 states it as `calc(infinity * 1px)`, which
Chromium computes as 3.36e7px. Both mean "a pill"; every consumer clamps the radius to
what the rect can fit (§28.2), and a near-infinite float would leave that clamp as the
only thing between `kurbo` and a non-finite arc.

### 36.2 Two things the probe got wrong before it got them right

**The instrument reported a divergence in itself.** The first run printed 13 confident
`DIFF` rows across the entire `text-*` scale — every font size reading back 16px, every
line height wrong to three decimal places. The probe pinned `font-size: 16px` as an
**inline style** on each element so that `em` and unitless `line-height` would resolve
against a known value. An inline declaration outranks a utility class, so every
`text-lg` was being overridden by the probe's own scaffolding. Moving it to an inherited
`body` rule fixed it, and all 26 rows agree.

This is §35's "a fixture that measures a second bug through the first reports nothing" —
inverted. There the instrument hid a real divergence; here it manufactured a fake one.
The tell was the shape of the result: *every* row in one group failing, and failing at
exactly the probe's own control value, is an instrument fault, not thirteen independent
table errors.

**The grep was not the measurement.** `grep -o -- '--radius-[a-z0-9]*:' tailwind-browser.js`
finds the v4 values in seconds and got the right answer here. It was still worth driving
Chromium: the bundle declares tokens, and what a *utility* does with one is a separate
question. Bare `rounded` maps to 4px and `--radius-none`/`--radius-full` are not declared
at all — a grep-only audit would have had nothing to say about three of the eleven rows,
including the one special case that matters.

### 36.3 Verification

**Host suites.** `twcore` 57 → **58**; `zkite_py` **175**, `cli` **42**, `bundler` **27**,
`zkite_renderer` **50**, `zkite_imagestore` **8**, `zkite_pybridge` **20** (`--test-threads=1`,
§29.5), webruntime e2e **39** — every one unchanged and passing.

**Two negative controls on the host test**, because the two halves of this bug fail
differently and a control that only exercises one of them is a control for half a fix:

| control | result |
|---|---|
| the complete v3 table restored | fails on `rounded-xs`: `unrecognised classes in "rounded-xs"` |
| `rounded-sm` alone put back to 2.0 | fails on the value: `left: 2.0, right: 4.0` |

The first masks the second — an unparsed class panics before any value is compared — so
running only the obvious control would have left the value assertion unproven.

**Conformance: one new fixture, 31 total, both dprs.** `radius_scale` is the whole named
scale, eight 70x70 white boxes on black. Nothing exercised the scale before it: `border`,
`shadow` and `ring` each pick one convenient `rounded-*` to have a corner at all, and
between them they happened to name only tokens v3 and v4 agree on.

| fixture | dpr 1 (loose / strict) | dpr 2 (loose / strict) |
|---|---|---|
| radius_scale | 0.18 / 0.83 | 0.09 / 0.42 |

**Every pre-existing fixture is byte-identical to §35's table at both dprs** — the
post-fix run was diffed against the pre-fix run rather than eyeballed, and the two are
character-for-character the same. Nothing in the existing 30 touches an affected class,
which is the same fact that made the bug survive this long.

**The fixture's own controls, and the honest half of the result:**

| control | dpr 1 | dpr 2 |
|---|---|---|
| the complete v3 table restored | 0.18 → **1.99%** | 0.09 → **1.83%** |
| `rounded-sm` alone put back to 2.0 | 0.18 → **0.18%** | 0.09 → **0.12%** |

**The second row is §28.4's blindness recurring, and it is the more useful number.** A
2dp radius error on one box out of eight moves the loose column by 0.00–0.03 percentage
points — inside the noise of every other fixture in the table. `max_delta` does move
sharply (92 → 126 at dpr1, 111 → **252** at dpr2) and the changed-pixel count with it
(93 → 97, 193 → 254), so the signal exists, but **not in the column anyone reads**. The
harness sees the *unparsed* tokens loudly (a 32dp corner going square is ~880px at dpr1)
and the *wrong value* barely at all. What actually guards the value is the host test.

The control artifacts were reverted and the harness re-run from clean source
**immediately**, per §34.8's trap and §35's precedent, and the regenerated report diffed
against the pre-control run: identical.

**No device run.** No device was attached for this session and none is claimed — the
convention is to say so plainly and pick work whose evidence does not need one (§29's
precedent). Nothing here is device-shaped: the value flows `Theme` → `Paint::radius` →
`zkite_renderer::decoration`, all of it shared code that `conformance` exercises through
the real `vello_cpu` rasteriser on the host, and unlike §35.5 there is no GPU-resident
state for a device to disagree about. A device run would confirm the corner arcs the
conformance images already show. It is worth doing next time a phone is plugged in, and
it is not evidence this section is missing.

### 36.4 Known gaps

- **The conformance harness cannot see a wrong radius *value***, only an absent one, and
  only via `max_delta` — 0.18% against 0.18%. This is the third measured instance of
  §28.4's tolerance blindness (after the shadow blur conversion and `shadow-2xl`), and
  the first where the loose column did not move *at all*. Anyone changing the radius
  maths is guarded by `the_radius_scale_is_v4s_and_not_v3s` and nothing else.
- **The probe is a thing someone runs, not something CI runs.** It needs Chromium and
  Playwright, so `probe_theme_defaults.mjs` guards nothing on its own; the guard is the
  Rust test, and the Rust test pins a table transcribed by hand from what the probe
  printed. Re-vendoring `tailwind-browser.js` is the moment to re-run it, and nothing
  enforces that — the same standing weakness §9.6's generated palette does not have,
  since that one is regenerated from the bundle by `gen_palette.py`. Generating the
  whole theme table the way the palette is generated is the obvious next step and was
  not attempted: the palette is 286 mechanical entries, while these tables carry
  semantics (`LineHeight::Absolute` vs `Relative`, `full`'s deliberate 9999) that a
  generator would have to encode by hand anyway.
- **Only the tokens `twcore` already knows about were audited.** The probe compares
  `twcore`'s table against v4; it does not enumerate v4's utilities and ask what
  `twcore` is missing. `rounded-xs`/`rounded-4xl` were found because the *radius* scale
  was being read off the bundle wholesale — a v4 addition in a family the probe did not
  read that way would not have surfaced. `shadow-*` is the known instance: §28.5 records
  `shadow-inner`, coloured shadows and `ring-offset-*` as unparsed, and this section
  does not change that.
- **`--text-shadow-*` exists in v4 and `twcore` has no concept of it.** Seen in the
  bundle while grepping. Not a divergence today — no widget emits one and neither
  backend would be asked to — but it is a v4 family with nothing on either side.
- **`rounded-s-*`/`rounded-e-*` (logical, RTL-aware) are unparsed.** `parse.rs` handles
  `t/b/l/r` and the four physical corners; v4 also defines the start/end forms, which
  the bundle lists. `twcore` has RTL variant support (`tests/variants_and_cache.rs`), so
  this is a real hole rather than a scoping decision, and it is untouched by §36.
- **No device run** (§36.3).

## 37. Artifact provenance — the shell in this tree was not built from this tree

`zkite build apk` compiles nothing. It copies `libzkitegpuharness.so`, `libpython3.14.so`,
`libc++_shared.so`, `classes.dex` and `fonts.zip` verbatim out of `<shell>/artifacts/`,
and until this session nothing in that path carried a build id, a hash or a warning.
§34.8 filed it after it shipped a negative-control shell and the symptom read as a
network hang; §35 hit the same shape and escaped only by byte-comparing `classes.dex`
against a freshly built one by hand. Two incidents, one filed gap, five handoffs.

This section closes it, and the first thing the check did when it ran was find a third
instance sitting in the tree at `HEAD`.

### 37.1 The finding: §36's fix was not in the artifact anyone would have shipped

The native shell staged in this checkout was **built twelve hours before the commit at
`HEAD`**, and `HEAD` is §36 — the session that fixed `twcore`'s radius scale.

| | |
|---|---|
| `artifacts/arm64-v8a/libzkitegpuharness.so` | written **Aug 10 03:07** |
| `df6a798` (§36), which changed `spike3_twcore/src/theme.rs` **and** `renderer/src/lib.rs` | committed **Aug 10 15:39** |

Both of those files compile into that `.so`. So `zkite build apk --backend native`, run
at any point today, would have produced an app whose `rounded-sm` was still **2dp
against the WebView's 4dp** and whose `rounded-xs`/`rounded-4xl` corners were still
**square** — the exact bug §36 found, fixed, and verified on the host.

**Proved rather than inferred.** The `.so` from that build survives inside
`artifacts/zkite-native-devshell-arm64-v8a.apk`, so it could be recovered and compared:

```
sha256, libzkitegpuharness.so (arm64-v8a)
  6cd44fb332d3…   inside the devshell APK built Aug 10 03:07
  d8cfa8ae16a5…   rebuilt from clean HEAD
  6cd44fb332d3…   rebuilt with only §36's two source files reverted   <- byte-identical
```

The third line is the whole argument. Reverting *just* `theme.rs` and `lib.rs`
reproduces the staged binary exactly, which says the staged binary is the pre-§36 build
and nothing else. It also says this crate builds **reproducibly** — same source, same
bytes, in both directions — which is what makes a source digest a meaningful proxy for
an artifact at all.

**§36 is not wrong about anything it claimed.** It was explicit that no device run
happened and that "a phone run would confirm the corner arcs the conformance images
already show, and is worth doing next time one is plugged in." That run is precisely
what this would have poisoned: the next person to plug in a phone would have measured
2dp corners and concluded §36's fix did not work, from an artifact that never contained
it. **The `x86_64` cdylib was stale for the same reason.** `classes.dex`, `fonts.zip`
and the entire WebView shell were current — rebuilding them produced byte-identical
files, so the tally is two of the six artifact groups, not all of them.

### 37.2 The check is on the source, not on the bytes

Both prior incidents produced a **well-formed** artifact. Nothing about the file was
wrong; what was false was which build it came from. §34.8 recorded that `strings` could
not tell the two shells apart and that "the only reliable tell was the `.so`'s mtime
against the build log". So hashing the artifact answers the wrong question.

The claim worth checking is **"this artifact was produced by the source that is on disk
now"**. The build scripts write `artifacts/provenance.json`
(`phase_1/cli/scripts/artifact_provenance.py`); `zkite build` re-derives the digest and
compares (`phase_1/cli/src/provenance.rs`).

**The input set is derived, not listed.** `cargo metadata` resolves the graph and every
package whose manifest lives inside the repo contributes its `src/`, `build.rs`,
`Cargo.toml` and `Cargo.lock`. That is how `phase_0/spike3_twcore` lands in the native
shell's inputs — it reaches `gpu_harness` two crates deep, via `zkite_renderer`, and it
is exactly the crate §36 edited. A hand-written list would have missed it and this
section would not exist. Crates.io dependencies are outside the repo and are not
walked; `Cargo.lock` is hashed instead, which also means **adding a path dependency
cannot slip past** — a new one edits `Cargo.toml` and `Cargo.lock`, both hashed.

**Three answers, deliberately different**, because collapsing them into silence is the
thing that shipped the wrong app:

| situation | meaning | what `zkite build` does |
|---|---|---|
| digests differ | the claim is **false** | **fails**, naming the group and the rebuild command |
| sources absent | a packaged payload — nothing to compare | reports what the record says, and that it did not re-derive |
| `source_complete: false` | the record does not claim to determine the artifact | reports, never calls it verified |
| no record at all | nothing claims anything | warns, names the artifact and the script |

The third row is `fonts.zip`, and writing it down as data rather than as a comment is
the point: `stage_fonts.sh` copies faces out of the **host's** `/usr/share/fonts`, so no
digest over this repository determines it. A matching hash there would be asserting
something it cannot see.

**The canonical stream is implemented twice and pinned by a test that runs both.** §31.6
already had two implementations of one digest (`runtime.py` and `dev_cmd.rs`) kept in
step by prose in two docstrings. Here the Rust test shells out to the Python writer and
compares over a real fixture tree, with a control that moves one byte and asserts both
digests move *and still agree*. Drift fails a test rather than a device.

### 37.3 §29.7's other half: the staged set no longer exists twice

§29.7 filed it in as many words — "`stage_payload.sh` derives its staged set by
mirroring `build_cmd.rs` by hand, so if a staging site is added and not mirrored here,
an off-checkout build breaks and no test catches it… A check belongs in the CI that does
not exist."

A check is the weaker answer. `zkite payload-manifest` (hidden plumbing) prints the
payload-relative paths `zkite build` reads, and `stage_payload.sh` stages what it is
told. **There is no second list to drift**, and the script builds the binary from *this*
checkout first, because staging against some other `zkite`'s idea of the payload is the
failure the indirection removes.

What a check can still catch is a new *read* site that never becomes a *staged* site,
and that is a lint over `build_cmd.rs`/`dev_cmd.rs`/`wheels.rs`'s own source — it finds
the payload-relative string literals and asserts each is covered by a manifest entry.
It has its own control: an invented `repo_root.join("phase_9/not_staged_anywhere")` must
read as uncovered, and the parser must find the spellings the real source uses. Without
that control the lint passes just as happily when its parser returns nothing, which is
how a lint rots.

Two bugs in the rewrite, both found by running it and worth recording because both are
silent:

- **`cp -a src out` nests when `out` exists**, and the manifest legitimately stages
  overlapping directories — the shared pure-Python stdlib is `python-x86_64/prefix/lib/
  python3.14`, and the per-ABI `lib-dynload` is *inside it* when the ABI is x86_64. The
  first version produced `lib-dynload/lib-dynload`, which only a build on that ABI would
  have found.
- **Pruning the shared `lib-dynload` unconditionally deletes the copy x86_64 needs.**
  The old script pruned before staging per-ABI and got this right by ordering; the new
  one stages first, so the prune has to ask which ABIs are in the payload.

`zkite doctor` gained two checks off the same manifest: **payload completeness** (which
is what notices a missing `pip.pyz` — §23.2's "quiet one", which degrades rather than
failing and which nothing had ever asked about) and **shell provenance**.

### 37.4 The first wheel, and the bug `pip install` reported as success

§23.9 and §29.7 both record the same gap in the same words: **"No wheel was built."**
Every size figure in §23.3 was `zip -9` over a staged directory, and every claim about a
packaged install was reasoning about a shape nobody had produced.

`phase_1/cli/scripts/build_wheels.py` produces two, for §23.3's reason — the payload is
host-independent and the binary is not:

| wheel | size | tag |
|---|---|---|
| `zkite_payload-0.1.0-py3-none-any.whl` | **21.5 MB** (885 files) | one for every platform |
| `zkite_cli-0.1.0-py3-none-linux_x86_64.whl` | **2.9 MB** | one per platform |

§23.3 predicted ~19 MB and ~6 MB. The payload is slightly higher for the reason §29
already found (it ships the unpruned pure-Python stdlib, which is correct); the CLI
wheel is *less than half* the estimate, because 7.2 MB of binary compresses well and
§23.3 estimated the uncompressed size.

**Both distribution names are placeholders and this does not decide §23.8.** What it
does show is that the decision is cheap to defer: nothing in the resolver keys off a
distribution name. `payload.rs` probes for a **directory** called `zkite-payload/` beside
the binary — a name we choose and ship *inside* the wheel — so renaming either
distribution is a flag and a `Requires-Dist` line.

**The bug worth the section.** The first wheel built cleanly, `pip install` succeeded,
and `bin/zkite` was installed **mode 664**. pip decides whether a `.data/scripts/` entry
is executable with `stat.S_ISREG(mode) and mode & 0o111`, and a bare `0o755` in the
zip's external attributes fails the *file-type* half of that test. So the wheel was
correct by every check that does not run the thing, and produced a `PermissionError` on
first use. §15.6.1 keeps a list of ways "the build succeeded" has been wrong; this is a
new one, and it is what `--verify` exists for.

`--verify` does not check that pip accepted the zip. It checks that the installed binary
reports `beside the zkite binary` — because if it reports the checkout instead, the wheel
installed and the packaging did nothing, which is the exact silent failure §23.2 built
`Origin` to expose.

### 37.5 Verification

**No device run, and none is claimed. No device was attached** (`adb devices` empty for
the whole session), which is why this queue was picked over Queue A or F — §29's
posture, and every claim below is host-side by construction.

**The §34.8 trap, run for real** (§37.1's control, in the other direction):

| step | result |
|---|---|
| revert §36's two files, rebuild the shell | `.so` = `6cd44fb3…`, record says source `68e5b126…` |
| restore `HEAD`, **do not** rebuild | tree hashes to `0fb754be…` |
| `zkite build apk` | **exit 1**, names `native/arm64-v8a`, both digests, the rebuild command |
| `ZKITE_SKIP_SHELL_PROVENANCE=1 zkite build apk` | builds, after saying out loud that it did not check |
| rebuild from clean source | `.so` back to `d8cfa8ae…`, `matches source` |

The `dex` group read `matches source` throughout and `fonts` read `not determined by
source` — so the failure is scoped to the group that actually changed, not a blanket
alarm.

**The wheel, end to end, off the checkout.** `pip install` both wheels into a fresh
venv, then `zkite new` + `zkite build apk` from a project in `/tmp` with
`ZKITE_PAYLOAD_ROOT` unset: a **40.5 MB signed APK**, with `payload: …/verify-venv/
zkite-payload (beside the zkite binary)`.

The discriminating control is behavioural rather than a sentinel this time: the staged
payload carries **arm64-v8a only** while the checkout has **both** ABIs, and
`zkite build apk` with no `--abi` built exactly one. `resolve_abis` reads the artifacts
directory, so a build that had reached the checkout would have produced two APKs. The
shipped `libzkitegpuharness.so` is `d8cfa8ae…` — the post-§36 build.

**Tests.** cli **42 → 54**; zkite_py 175, bundler 27, renderer 50, imagestore 8, pybridge
20, twcore 58 — all unchanged and passing. webruntime e2e **39/39**. Conformance
unchanged (nothing in this section touches a rendering path).

Every new invariant has a control, and the controls discriminate:

- the two digest implementations agree **and** both move when one source byte moves;
- `build/` is excluded while `build.rs` is not, checked in both directions;
- a present-but-different source tree reads **stale**, while an absent one reads
  `SourcesAbsent` — the first version of that test asserted `is_stale()` against a tree
  with no sources at all and passed for the wrong reason;
- the manifest lint finds a path that is not staged;
- a half-staged payload is not mistaken for a verified one.

**A fail-open path cannot be tested by observing that it did not fire** (§31.6), so the
packaged-payload path has its own test with the sources really removed, asserting it
both declines to fail *and* still prints what the record says. `zkite doctor` violated
exactly that rule on its first draft — it printed an identical line for a verified shell
and an unverifiable one — which is recorded here rather than quietly fixed, because the
module whose doc comment states the rule is the module that broke it.

### 37.6 Known gaps

- **The digest is over source bytes, so it is comment- and whitespace-sensitive.** A
  doc-comment edit in `twcore` reports the shell as stale when the binary would be
  identical. That is the safe direction and the rebuild is ~6s warm, but it is a real
  cost, and `ZKITE_SKIP_SHELL_PROVENANCE` is the escape. Hashing the *build output*
  instead is what §34.8 already proved does not work.
- **`fonts.zip` is unverifiable and says so, rather than being fixed.** Its faces come
  from the host's `/usr/share/fonts` via `stage_fonts.sh`. Baking the six DejaVu files
  into the repo would close it and collides with §9.7's open "which typefaces should
  ZKite bundle" decision, so it is named, not guessed.
- **The manifest lint cannot see a path assembled from a variable.** It reads string
  literals and truncates a `format!` at its first `{`. A read site built out of a
  variable is invisible to it; the end-to-end off-checkout build is what would catch
  that, and nothing runs that automatically.
- **`provenance.json` has no signature and no tamper resistance.** It is a note the
  build script left for the build tool, on the same filesystem, and defends against
  mistakes rather than against anyone.
- **Nothing runs any of this on a schedule.** §23.7 step 3's release pipeline is still
  the missing piece and is still weeks; what exists now is the substrate it would call —
  `stage_payload.sh`, `build_wheels.py`, and a provenance record that makes "is this
  artifact current" answerable. plan1.md §14.5's "no CI for any phase-3 crate" is
  untouched.
- **`linux_x86_64` is not a manylinux tag.** PyPI rejects a bare `linux_*` upload, so a
  real release means building the CLI wheel in a manylinux image or repairing it with
  auditwheel. The binary links only libc/libm/libgcc_s (§23.3), so this is a packaging
  step rather than a portability problem — but no manylinux build was attempted and
  none is claimed.
- **One platform.** Only `linux_x86_64` was built. §23.7 step 4's other four wheels, and
  §23.5's `which()`/`py -3.14` Windows work, are untouched — still "no Windows or macOS
  machine was available", unchanged since §23.
- **The wheels were installed, not published.** No `twine check`, no TestPyPI upload, no
  `pip install zkite` from an index. `--no-index` local install only.
- **The framework wheel does not exist.** `zkite new`'s template says
  `dependencies = ["zkite"]`, meaning the *Python framework* — `phase_1/zkite_py/src/zkite`
  — should be on PyPI as an ordinary pure-Python wheel. Nothing builds that one, and it
  is the third distribution §23.8's decision has to place.
- **`zkite dev` was not run against any of this**, on either backend — the same gap §34
  and §35 both left open, now for a third session.

## 38. Gradients paint — and the palette column that was right for fills and wrong for stops

§15.1's divergence 3, reported by the owner from a real app rather than found by a test:

> **Gradients do nothing (divergence).** `twcore` has **zero** gradient parsing — no
> `bg-gradient-*`/`bg-linear-*`, no `from-*`, `via-*` or `to-*`. Unknown classes are
> silently dropped, so `bg-gradient-to-r from-cyan-500 to-blue-500` resolves to a plain
> background natively and renders correctly on the WebView (real Tailwind).

It was the largest remaining rendering hole after §35 built `Image`. The entry's own
sizing — "two layers deep, not one" (a parser *and* a gradient brush) — was right about
the shape and wrong about where the difficulty was. Both layers were routine. The work
was in a third place nobody had named: **which representation of a colour a gradient stop
carries**, §38.3.

### 38.1 What v4 actually emits, measured

`phase_1/webruntime/test/probe_gradients.mjs`, driving the exact `@tailwindcss/browser`
4.3.3 bundle `phase_1/webruntime/vendor/` pins inside real Chromium — the method §9.6,
§28.1, §33.1, §34.1, §35.4 and §36 all used. 35 class strings, each read back as a
computed `background-image`, the nine `--tw-gradient-*` custom properties, **and** real
pixels off a rendered swatch, because a computed value cannot answer a question about a
ramp.

Four rules came out of it, all four of which a first draft would have got wrong:

| rule | measured |
|---|---|
| A gradient needs a **line and at least one colour stop** | `from-cyan-500 to-blue-500` alone → `background-image: none`. So does `bg-linear-to-r` alone. So does `bg-linear-to-r from-10%` — a *position* is not a stop. |
| A declared slot with no colour is **transparent** | `bg-linear-to-r from-cyan-500` → `… <cyan> 0%, rgba(0,0,0,0) 100%`. A one-stop gradient fades out. |
| `background-color` and the gradient **coexist** | `bg-red-500 bg-linear-to-r from-cyan-500` keeps `background-color` *and* paints the gradient over it: the swatch's right edge reads (251, 45, 55), i.e. red-500 through the transparent end. |
| The default interpolation space is **oklab**, and every hue modifier implies **oklch** | `--tw-gradient-position: to right in oklab`; `/longer` compiles to `to right in oklch longer hue`. |

**The third rule is the one that shapes the model.** `Paint` carries `background:
Option<Color>` *and* `gradient: GradientSpec` as two independent fields, because they are
two CSS properties. The obvious first design — one `enum Background { Solid, Gradient }` —
cannot express the measured row above, and it is what a draft written from memory reaches
for.

**The fourth is not cosmetic.** `from-yellow-400 to-blue-600` at its midpoint is
(140, 161, 173) in oklab and (136, 145, 92) in sRGB — **81/255 in blue**. `peniko`'s
`Gradient::interpolation_cs` defaults to sRGB, so a paint site that forgot to set it would
render a plausible, wrong ramp. That is why `zkite_renderer::gradient` hands back a finished
`peniko::Gradient` rather than loose geometry: there are **four** paint consumers, and
three fields on that struct (`interpolation_cs`, `interpolation_alpha_space`, `extend`) are
silent-divergence-shaped if any one of them is left at its default.

Also measured, and both kept: **v3's `bg-gradient-to-r` is still live in 4.3.3**,
compiling to a byte-identical `background-image` — refusing it would break class strings
that work on the WebView backend. And the interpolation is in **premultiplied** alpha:
the midpoint of cyan → transparent over white reads (128, 219, 236), where
unpremultiplied would give (127, 173, 182).

### 38.2 The corner keywords are not angles, and Chromium's own pixels say so

CSS defines `to top right` geometrically — the line perpendicular to the gradient line
passes through the *other two* corners — so it depends on the box's aspect ratio and
cannot be pre-converted to an angle in `twcore`, which has no layout. `GradientLine`
therefore keeps keywords as keywords and `zkite_renderer::gradient::line_endpoints`
resolves them against a laid-out rect.

The derivation was checked against Chromium rather than against the spec text. On a
200x100 box, this geometry puts the midline's left edge at offset **0.25** of the
`to-tr` ramp and its centre at **0.50**; Chromium reads (0, 171, 229) and (0, 157, 240)
there, against (0, 171, 230) and (0, 157, 241) at those same two offsets of the same ramp
drawn `to-r`. Pinned as arithmetic in
`a_corner_keyword_on_a_wide_box_matches_chromiums_offsets`, so the claim survives without
Chromium being present.

### 38.3 The finding: a gradient stop cannot use the palette's hex

The parser and the brush both worked on the first run, and the conformance harness
reported **`max_delta=68`** with 16% of pixels outside the strict tolerance. R and G
agreed within 3; only blue diverged, worst at the *yellow* end of a yellow → blue ramp,
where Chromium's blue channel was pinned at **0**.

`yellow-400` is `oklch(85.2% 0.199 91.936)`, and that colour is **outside the sRGB
gamut**: its blue is **-0.0589** in linear terms, **-68.6** on the 0..255 encoded scale.
Chromium interpolates the *unclipped* value and clips only at display time, so blue stays
negative — displayed as 0 — for the first fifth of the ramp. `twcore`'s palette stores
each colour as a gamut-clipped `0xRRGGBB`, which is exactly right for a fill (that is what
a browser paints for a solid `bg-yellow-400`) and lossy for a stop.

Predicted from the clipped-vs-unclipped model and then checked against the seven
Chromium samples the probe had already printed:

```
t=0.126  predicted  -28.3 -> displayed    0   Chromium    0
t=0.251  predicted   11.7 -> displayed   12   Chromium   12
t=0.377  predicted   52.1 -> displayed   52   Chromium   52
t=0.502  predicted   92.1 -> displayed   92   Chromium   92
t=0.628  predicted  132.4 -> displayed  132   Chromium  132
t=0.753  predicted  172.4 -> displayed  172   Chromium  172
t=0.879  predicted  212.8 -> displayed  213   Chromium  212
```

Seven for seven. So `gen_palette.py` now emits **two columns from the one OKLCH source**:
the clipped `u32` it always did, and the same colour unclipped in encoded sRGB as `[f32;
3]`, with the transfer function odd-extended for negatives (`-f(-c)`, CSS's own rule).
`twcore::ExtColor` carries the second one, and only gradient stops read it.

**All 286 clipped values are byte-identical to before** — checked, because this touched the
generator that produces every colour in the framework.

Three things worth carrying:

- **95 of the 286 palette entries are outside the sRGB gamut** — a third of it. This is
  the ordinary case for a gradient between two saturated Tailwind colours, not an edge
  case, and the number is asserted (as a lower bound) by
  `the_palette_really_does_leave_the_srgb_gamut` so a generator that silently clipped both
  columns fails a test instead of quietly restoring the 68.
- **`peniko::Color` can hold it.** It is `AlphaColor<Srgb>` over four `f32`s with no
  clamping, so the negative channel survives into the brush. `from_rgba8` would not.
- **vello carries an out-of-gamut stop through an Oklab interpolation and clips it for an
  sRGB one.** Measured directly: identical ramps for clipped and unclipped stops under
  `ColorSpaceTag::Srgb`, different ones under `Oklab`. `vello_common` builds a u8 LUT for
  the sRGB path, and a u8 cannot be negative. This is why `/srgb` remains divergent
  (§38.5) while the default does not.

### 38.4 Verification

**Host, and it is the fixture table that moved.** Four new conformance fixtures, each
isolating one thing that can be wrong, because one "gradients look right" number would
report a single figure for four independent failures (§31.7's lesson):

| fixture | dpr 1 (loose / strict) | dpr 2 (loose / strict) | max_delta |
|---|---|---|---|
| gradient_linear (8 directions) | **0.00** / 2.51 | **0.00** / 2.32 | 34 |
| gradient_angle (45°, `[30deg]`, corner, all on 2:1 boxes) | **0.00** / 1.12 | **0.00** / 1.13 | 34 |
| gradient_stops (via, positions, fade-to-transparent, over a colour) | **0.00** / 1.02 | **0.00** / 1.02 | 32/34 |
| gradient_space (white→black, oklab vs `/srgb`) | **0.00** / 0.00 | **0.00** / 0.00 | **2** |

Before §38.3's fix the same fixtures read 1.97 / 16.21, 1.15 / 15.90 and 0.59 / 5.01 with
`max_delta=68`. **31 → 35 fixtures**, both dprs; every pre-existing fixture is unchanged.

`gradient_space` is the one that guards the interpolation space, and it is deliberately
in-gamut end to end (white → black) so nothing it reports can be explained by clipping.
Its `max_delta` of **2** is also the tightest number in the whole table — and it is the
answer to §28.4/§36.4's standing complaint that the harness cannot see a *wrong value*.
These fixtures can: the pair was chosen for it.

**Tests.** twcore **58 → 77** (17 gradient, 2 palette-column), `zkite_renderer` **50 →
62**, and cli 54 / bundler 27 / imagestore 8 / pybridge 20 / zkite_py 175 / e2e 39 all
unchanged and passing.

**Device — the Moto G54, arm64-v8a, the GPU path, and this time both an in-tree harness
and a real `zkite build apk` app.** `phase_3/gpu_harness/demo_app/gradients.py` renders
all eight directions, the stop model, the gamut band, a `rounded-full` pill and both angle
forms. Measured out of the screenshot:

| claim | on the device |
|---|---|
| the oklab ramp matches real Chromium | midpoint of the yellow → blue band **(141, 162, 173)** against Chromium's **(140, 161, 173)** |
| `from-25%`/`to-75%` hold their flats | flat (253, 199, 0) to 17%, flat (21, 93, 252) from 83% |
| `via-*` lands at 50% | (225, 43, 251) at the midpoint — fuchsia-500 is `#e12afb` |
| a one-stop gradient fades to the colour beneath | over `bg-red-500`: (253, 195, 2) → **(251, 49, 52)**, i.e. red-500 |
| `bg-linear-45` ≡ `bg-linear-[45deg]` | worst per-channel delta between the two boxes: **0** |
| a gradient is clipped by its own radius | the `rounded-full` pill and the `rounded-2xl` card both clip |

And in a shipped app built by `zkite build apk --backend native` from a project in `/tmp`:
a `bg-linear-to-br from-fuchsia-600 via-violet-500 to-cyan-400` header over a 400-row
virtualised `List`, on the GPU path.

**One instrument lied and its own control caught it.** The first device liveness check
compared two screenshots and found the band that *must not change* had changed in every
pixel — because the second screenshot was all black. The display had dimmed between
captures (§35's trap), `svc power stayon usb` notwithstanding. Re-run with a
`max(channel sum)` assertion on **both** captures, the must-not-change bands read 0.

**A second control failed for a better reason.** The liveness tap did nothing, because the
demo app used `Text(..., on_tap=…)` and WIDGETS.md's table gives `Text` **no events at
all** — an unknown prop, silently accepted, silently ignored. That is §26.7's rule ("write
the repro against the documented signature") failing in exactly the direction the rule
exists for. Fixed to a `Button`; recorded rather than quietly corrected.

### 38.5 Known gaps

- **`bg-radial` and `bg-conic` are still unparsed**, along with `bg-radial-[at_25%_25%]`
  and `bg-conic-180`. They reach the unknown-class report rather than half-working, and a
  test pins that they do — while a family is missing, being *loud* is the whole defence.
  `peniko` has both brushes; CSS's default `radial-gradient` is a `farthest-corner`
  **ellipse** while peniko's radial is circles, so honouring it needs a paint transform,
  which is why this was deferred rather than half-built.
- **`/srgb` is measurably divergent: 24.39% loose, `max_delta=54`.** Cause proven, and it
  is vello's: `vello_common` builds a **u8** LUT for an sRGB-space interpolation, so an
  out-of-gamut stop is clipped before interpolation, while Chromium interpolates the
  unclipped value. The default (oklab) path is unaffected. A fix exists on paper —
  subdivide the ramp ourselves and emit in-gamut stops at each gamut crossing, where the
  clipped ramp's kinks actually are — and it costs an allocation per gradient per frame on
  the paint path, which is why it is filed rather than done.
- **A residual `max_delta` of 32–34 survives at gamut crossings**, invisible in the loose
  column (0.00%) and visible in the strict one. Same mechanism, other direction: vello
  clips its LUT *samples* and interpolates between the clipped values, while Chromium
  interpolates and clips per pixel — so within the out-of-gamut segment of a ramp, vello
  rises linearly from a clipped endpoint where Chromium holds at 0. The crossing-point
  subdivision above would close this too.
- **`zkite_renderer` now depends on `peniko`.** Argued in `gradient.rs`'s module doc (a
  type crate with no rasteriser, the same category as `kurbo`, and the thing both vello
  backends re-export), but it is a new edge in the dependency graph and it must stay
  version-compatible with both backends. A skew is a type error at the call site, not a
  silent divergence, which is the direction that fails safely.
- **`from-current` / a `currentColor` stop is not handled.** `ExtColor` has no
  `PaintColor::Current` equivalent, so a stop that should track the node's text colour
  would fall through to the unknown-class report.
- **No `dark:` variant still** (§15.1 divergence 2), which is the other half of Queue A
  and untouched here.
- **The probe is a thing someone runs**, exactly as §36.4 records for
  `probe_theme_defaults.mjs`: it needs Chromium and Playwright, so the guard is the Rust
  test plus the four conformance fixtures, and re-vendoring `tailwind-browser.js` is the
  moment to re-run it.

## 39. "Scrolling isn't smooth" — one transient error, and a permanently CPU-rendered app

Reported by the owner from their own phone, in those terms: ZKite apps are not very smooth,
scrolling especially, make it smooth like native. Reported from the product, not from a
test — which is how §25, §26 and §31 all started.

**The cause is not that the renderer is slow. It is that the renderer stops being used.**
One transient swapchain error mid-scroll silently and permanently downgraded the app to
the CPU rasteriser, at ~23–29ms per frame against the GPU path's ~11, and nothing ever
undid it.

### 39.1 The measurement, and the two instruments that could not see it

`dumpsys gfxinfo dev.zkite.gpuharness` reports **`Total frames rendered: 1`**. It measures
HWUI, and this renderer owns a `SurfaceView` it draws into from its own thread, so the
industry-standard jank instrument is structurally blind to every frame ZKite paints.
`dumpsys SurfaceFlinger --latency <layer>` returned 128 rows of zeros for the layer named
`SurfaceView[…]#130938` — the container, not the buffer producer. **Neither is useless, but
neither works off the shelf here, and both look like "no problem found" when pointed
wrongly.** The app's own `ZKITEPERF` lines are the instrument that works.

They said this, on a 1,000-row virtualised `List` under a sustained drag:

```
ZKITEPERF frame-interval n=60 avg 19.12ms max 47.06ms (152 nodes)
  phases: build 0.74  layout 1.47  positioned 0.11  sync 1.80  a11y 0.23  scene 4.12  present 9.02  (ms)
```

19.12ms average interval against a 16.67ms budget at 60Hz, with 39–47ms frames throughout.
`present` at 9.02ms is over half the frame — and reading that as "painting is slow" would
have been wrong, which the field's own doc comment already said:

> `get_current_texture` + `vello_hybrid::render` + `submit` + `present`. Deliberately
> separate from `scene`: **this one can block on the swapchain, so time here is not
> necessarily work done**, and reading the two as one number would make vsync throttling
> look like slow painting.

A correct warning, in the code, about a number nobody had ever split. §39 split it — and
**refuted the warning's own hypothesis**: `acquire` is 0.20–2.26ms. The swapchain is not
throttling anything; `present` really is ~7.5ms of work.

### 39.2 Averages cannot see a hitch

The per-phase averages summed to the average frame and said nothing about the 47ms ones. A
hitch is one frame in sixty; a mean of sixty is the wrong statistic for it. So
`FramePhases::take_max` and a second log line:

```
  phases: build 0.68  layout 1.23  positioned 0.10  sync 1.29  a11y 0.18  scene 2.92  acquire 1.92  present 7.44  (ms)
  worst:  build 1.65  layout 3.29  positioned 0.27  sync 11.16  a11y 0.31  scene 5.58  acquire 6.42  present 12.37  (ms)
```

That line found the bug on its second run, in a window that read:

```
  phases: build 0.54  layout 0.97  positioned 0.09  sync 1.58  a11y 0.15  scene 22.78  acquire 0.00  present 0.00  (ms)
```

**`acquire 0.00 present 0.00` with `scene` at 22.78ms.** Those two are only ever zero when
the GPU path did not run at all, and `paint_cpu_fallback`'s time is accumulated into
`scene`. Every frame in that window was software-rendered.

### 39.3 The cause: a fall-back that was a one-way door

```
E get_current_texture: Validation; falling through to CPU for this frame
I frame: 1080x2223 via cpu-fallback
E get_current_texture: Validation; falling through to CPU for this frame
… once per frame, forever
```

`paint_gpu` returned `Option<()>`, and `None` meant two incompatible things: *this device
has no working GPU path, use the CPU rasteriser* — which is what plan1.md §5/§9's fallback
is for — and *the swapchain hiccuped on this one frame*. The recovery for the second was a
comment:

> Still presentable this frame; `configure_surface` gets called again on the next
> `Resize`/`NewWindow`, which is the recommended recovery per wgpu's own doc.

During a scroll neither ever arrives. **A doc comment asserting that recovery happens
somewhere else, and it does not** — §35.5's shape exactly, and the third time in this
document that a comment's claim about the code around it was false.

**Why `Validation`, in wgpu's own words.** This crate installs no `log` bridge, so wgpu's
error text went nowhere and `Validation` was the entire diagnosis. One
`device.on_uncaptured_error` handler later:

```
wgpu uncaptured error: Validation { source: ContextError { fn_ident: "Texture::create_view",
  source: InvalidResource(… ResourceErrorIdent { type: "Texture",
  label: "Placeholder External Texture" }) },
  description: "In Texture::create_view — Texture with 'Placeholder External Texture' label is invalid" }
```

wgpu's Android swapchain images are external (AHardwareBuffer-backed) textures, and one of
them is being invalidated under us mid-scroll. `Surface::configure` is the documented
recovery for exactly that.

**The fix, in three parts:**

| part | what |
|---|---|
| `PaintOutcome` replaces `Option<()>` | `Painted` / `Skip` / `Recreate` / `Fallback` — the four things that can happen, so "no GPU on this device" and "no buffer this millisecond" stop sharing a representation. |
| recovery per wgpu status | `Timeout`/`Occluded` → skip the frame (its own doc says to). `Outdated`/`Validation` → **reconfigure and retry once**, then skip. `Lost` → drop the surface and recreate it from the window, which only `State` holds. |
| the CPU fallback runs **only** for `Fallback` | i.e. only when there is no usable GPU path at all, which is what it was built for. |

A skipped frame leaves the last good frame on screen, which is what a dropped frame *is*.

One ordering change was forced rather than chosen: the acquire now happens **before** the
image uploads and the scene build, because recovery calls `reconfigure`, which replaces the
`vello_hybrid::Renderer` and with it the image atlas. Uploading first and reconfiguring
second would paint every `Image` as an opaque black rectangle — precisely §35.5's bug.

**A hypothesis, named and not proven:** `paint_cpu_fallback` locks the `ANativeWindow` and
writes pixels directly, so for that frame the CPU is the buffer producer for a window whose
swapchain wgpu still owns. That would explain why the old behaviour was *self-sustaining*
rather than merely slow — the first fallback frame breaking the surface for every frame
after it. §39 did not isolate it, and the recovery above fixes the symptom either way.

### 39.4 Verification, on the phone

Same app, same 14-swipe drag, on the Moto G54 over USB, focus asserted before and after:

| | before | after |
|---|---|---|
| `cpu-fallback` frames | every frame after the first error, permanently | **0** |
| `get_current_texture: Validation` | once, then fatal | once, **reconfigured and retried** |
| frame-interval avg | 27.18ms in the degraded window | **17.09 / 17.13 / 17.32ms** |
| `acquire` avg | — | 0.20–1.69ms |

And in a **shipped app** — `zkite new` + `zkite build apk --backend native` from `/tmp`, a
gradient header over a 400-row `List`, 12 swipes: `cpu-fallback` **0**, frame-interval avg
**16.60ms** and **17.29ms**, the same `Validation` raised once and recovered. §34's lesson
holds: the artifact a user gets is the one worth measuring.

### 39.5 What is still not smooth, measured rather than asserted

The one-way door is closed. The app is not yet native-smooth, and these are the numbers
that say why — none of them is fixed here.

- **`sync` spikes to 10.2–14.0ms, every time the virtualised window moves.** It is the
  largest single-phase worst case in every window measured, and on those frames it alone
  eats most of the budget. This is §11's design working as designed: a `List` cannot know
  which rows to build without knowing where it is scrolled to, so scroll position reaches
  `zkite_py` and Python rebuilds the row window. Mitigations worth measuring next: a larger
  `overscan` (fewer, costlier rebuilds), building only the rows that entered the window,
  or moving the window decision out of Python entirely.
- **The steady state has no headroom: ~15.5ms of work per frame at 60Hz.** `present`
  ~7.5ms (`vello_hybrid::render` + submit — CPU-side strip generation for a 1080x2223
  surface, on a PowerVR BXM-8-256) and `scene` ~3.0ms are the two big items, with ~3.7ms of
  build/layout/sync/a11y on top. At that cost, any variation drops a frame, which is what
  `max 31–49ms` is.
- **`vello_hybrid::Scene` cannot be reused.** 0.1 has no `reset()`, so `Scene::new` runs
  per frame and the allocation is not avoidable through the public API. Checked, because
  "allocation on the render hot path" is a thing this document has removed twice (§11.9,
  §18.3).
- **A drag is not vsync-paced.** §13.2's `next_frame_wait` paces a *fling* against a real
  deadline; during a finger-down drag frames are produced as touch events and work allow.
  With work ≈ budget that alternates between one and two vsyncs per presented frame, which
  is felt as irregularity rather than as slowness. Pacing a drag the way a fling is paced
  is the obvious experiment and was not run.
- **Overdraw is unmeasured.** Every row paints an opaque background over the root's, and
  nothing tracks damage or occlusion; the whole surface is re-strip-generated every frame.

  > **Measured in §46, and both halves of this entry are wrong.** Removing every opaque
  > row background, on an arm with an identical node count, moves `vello_hybrid::render`
  > by **0.000ms** and `present` as a whole by −0.202ms; the cost of those rects is
  > 0.500ms and it is in `scene`, our own scene-building walk. And `present` is **not**
  > "the whole surface re-strip-generated": strip generation is 0.67ms of it, while
  > `CommandEncoder::finish` + `Queue::submit` is 5.29ms — **45% of all frame work,
  > invariant to everything the app draws.** The parenthetical in the entry above
  > (*"`present` ~7.5ms (`vello_hybrid::render` + submit — CPU-side strip generation…)"*)
  > is the specific sentence that was quoted forward into §42.8, §44.7 and
  > NEXT_SESSION.md, and it was never checked.
- **No before/after against the WebView backend.** §12/§13's head-to-head said native
  "ties sustained scroll"; that comparison predates this bug, and whether it was measuring
  a degraded native path is now an open question worth re-running.
- **One device, one app shape, synthetic input.** All of this is `adb shell input swipe` on
  a Moto G54 at 60Hz. A real finger samples differently, and a 120Hz panel would halve the
  budget.

## 40. The `sync` spike — and the mitigation that would have made it worse

§39.5 left four measured reasons the app is not yet native-smooth and fixed none of
them. This section takes the first and largest: **`sync` spiking to 10.2–14.0ms every
time a virtualised window moves**, the biggest single-phase worst case in every window
§39 measured.

It is also the first item in §39.5's own list of candidate mitigations, and that
candidate is wrong. Checking it before implementing it is the whole finding.

### 40.1 The finding: `overscan` bought nothing and cost linearly

§39.5 proposed, first in its list: *"a larger `overscan` (fewer, costlier rebuilds)"*.
The parenthetical is false. `compute_window` is four lines:

```python
visible_rows = int(-(-viewport_h // item_height)) + 1
window_rows  = visible_rows + 2 * overscan
first = int(scroll_y // item_height) - overscan
first = max(0, min(first, count))
last  = min(count, first + window_rows)
```

`first` is `scroll_y // item_height` minus a **constant**. Subtracting a constant shifts
the window; it does not stall it. So `first` moves on every row boundary crossed no
matter what `overscan` is, and `window_would_change` — which asked whether
`compute_window` would now return a *different range* — fired on every row boundary at
every value of `overscan`.

Measured on the host over 2000dp of mid-list scroll (889dp viewport, 56dp rows, 100k
rows, far from both clamps):

| `overscan` | window rebuilds | rows per rebuild | rows built |
|---|---|---|---|
| 0 | 36 | 17 | 612 |
| 2 | 36 | 21 | 756 |
| 4 | 36 | 25 | 900 |
| 8 | 36 | 33 | 1188 |
| 16 | 36 | 49 | 1764 |
| 32 | 36 | 81 | 2916 |

**The rebuild count is invariant in `overscan`; the cost of each one grows linearly.**
Following §39.5's own advice would have kept the number of `sync` spikes exactly the
same and made every one of them bigger — the reported symptom, worse.

The first attempt to measure this got it wrong in a way worth recording: run from
`scroll_y = 0`, the numbers *do* fall with `overscan` (36 → 4), which looks exactly like
the predicted win. That is entirely the `first = max(0, ...)` clamp at the top of the
list — at `overscan=32` the clamp covers 1792dp of the 2000dp sampled. A mitigation
measured only from the top of a list would have been "confirmed" and shipped.

### 40.2 Two questions, one function

The cause is that one function was answering two different questions:

- **What should be on screen?** The visible rows. Non-negotiable.
- **What should be built?** Visible plus `overscan` either side — slack, so that the
  answer stays true for a while.

`compute_window` answers the second. `window_would_change` needs the *first*, and was
asking the second and comparing for equality — which is why the slack never functioned
as slack. Naming the visible range as its own function (`visible_range`) and asking
about **coverage** is the whole change:

```python
v0, v1 = visible_range(scroll_y, viewport_h, count, item_height)
return not (first <= v0 and v1 <= last)
```

Keep the window that is already built until the viewport reaches its edge; when it
does, `compute_window` recentres it. A rebuild now happens once per `overscan + 1`
rows, in both directions, and `overscan` finally means what its name says.

**The invariant this rests on**, and it is not optional: a window `compute_window` just
produced must always cover its own visible range, or every scroll frame would re-render
forever — the exact opposite of the intent. It holds under every clamping branch
(`first = v0 - overscan ≤ v0`, and `last ≥ v0 + visible_rows ≥ v1`; when `first` clamps
to 0 it is still `≤ v0`, and when `last` clamps to `count` it is still `≥ v1`).
`test_a_freshly_built_window_never_asks_to_be_rebuilt` pins it across both clamps at
four `overscan` values rather than leaving it as an argument.

After the change, on the same host measurement, `overscan` is monotone in **both**
directions — rebuilds *and* total rows built:

| `overscan` | rebuilds / 200 rows | was |
|---|---|---|
| 0 | 201 | 201 |
| 4 (default) | 41 | 201 |
| 8 | 23 | 201 |
| 16 | 12 | 201 |

That is the trade §39.5 believed it already had: fewer, costlier rebuilds. It did not
exist until this section; it exists now.

### 40.3 One behaviour change, and it is an improvement

The first viewport report of every app's life stops re-rendering.

`BOOTSTRAP_VIEWPORT_PX` is 1400 and its docstring says it is deliberately *"a superset
of the real first window rather than a subset"*. So when the first real layout reports
an 800px viewport, every visible row is already built — and the old equality test still
re-ran `view()`, serialised a whole tree to JSON and re-parsed it, in order to build
**fewer** rows. Under coverage it correctly does nothing.

The cost is that the list holds the bootstrap window's extra rows until the first real
window move, which then recentres it. That is strictly less work than before, and it is
`zkite_pybridge`'s `virtualised_list_windows_without_changing_the_scrollable_range` that
caught the change — the one test in the tree that drove this path through real CPython.
Its premise was updated rather than its assertion loosened, and it gained the §40
contract end to end: four rows of scroll into the overscan returns `None`, five returns
a tree.

### 40.4 A new instrument: the rate, not the cost

§39 added `worst:` because a mean of sixty frames cannot see one hitch. The same problem
appears one level down here: this change alters **how often** Python rebuilds, not what
one rebuild costs, and no existing line could see a rate. So `scroll_stats`, and a line
beside the other two:

```
  scroll: 21 tree rebuilds / 60 frames (35% of frames re-ran view())
```

Reading it beside `sync` separates the two ways `sync` can improve — fewer rebuilds
versus a cheaper rebuild — which the timing alone conflates. It also gives the failure
mode its own signature: `rebuilt 0` with a large `sync` would mean the cheap
"nothing to do" path is not cheap, which is a different bug from the one §39.5 filed.

### 40.5 Verification

Host, all green: `zkite_py` **175 → 182** (7 new, all in a new `TestRebuildFrequency`
class — `window_would_change` had **no tests at all** before this section, which is how
the assumption survived), cli 54, bundler 27, renderer 62, imagestore 8, twcore 77,
`zkite_pybridge` 20 (`--test-threads=1`, §29.5), webruntime e2e 39/39, conformance 35/35
fixtures x 2 dprs with the four gradient fixtures still at 0.00% loose.

**The negative control.** The seven new tests were run against the old equality-based
`window_would_change` with everything else unchanged: **7 failed**. Three of the new
tests pass under *both* implementations and are kept deliberately —
`a_freshly_built_window_never_asks_to_be_rebuilt`,
`scrolling_past_the_overscan_does_rebuild` and
`a_node_that_never_rendered_a_window_never_asks_for_one` — because a fix of this shape
has an obvious degenerate form (answer "no rebuild" more often) and those three are what
distinguish the fix from it.

Both backends share `_viewport.window_would_change` — the native path through
`_zkite_scroll_tree`, the WebView path through `_zkite_scroll_html` — so this is one
change for both, and the e2e suite covers the second.

### 40.6 The device measurement

Moto G54 over USB, `cmp_list_1k.py` (1,000 rows, 56dp, `overscan=4`), 14 sustained
swipes, GPU path. Both arms built through `build_apk.sh`, artifact contents verified
against the working tree before each install, and each run gated on: portrait
`1080x2223` surface **at swipe time and after**, `mCurrentFocus` and pid identical at
both ends, ~285 `onTouchEvent`s, and **0** `cpu-fallback` frames.

| | arm A (equality) | arm B (coverage) | |
|---|---|---|---|
| frames that re-ran `view()` | **53.6%** | **8.8%** | −84% |
| `sync` average | 4.55 ms | **0.88 ms** | −81% |
| `sync` worst | 11.08 ms | 9.74 ms | −12% |
| frame-interval average | 18.98 ms | **17.48 ms** | −8% |
| frame-interval worst | 48.39 ms | 39.41 ms | −19% |

Means over 5 (A) and 6 (B) sixty-frame windows. Per-window rebuild rates were
50/60/63/60/35% against 5/12/12/12/10/2% — the separation is far wider than the
spread, which matters because a drag's speed varies across a swipe.

**`sync` worst barely moved, and that is the honest headline.** This change makes the
spike *rare*, not *small*: a rebuild still re-enters CPython, re-runs `view()`,
serialises a tree and re-parses it, and still costs ~10ms when it happens. What changed
is that it happens on roughly one frame in eleven instead of one in two. The
frame-interval average — the number a user feels — moved 18.98 → 17.48ms against a
16.67ms budget, i.e. ~53fps → ~57fps.

**Three runs were voided before the comparison held, each caught by a control rather
than by inspection.** They are the reason the gates above exist:

- **An incoming WhatsApp call took the foreground.** §39 lost a run to exactly this and
  that is why the rule exists; it is still the single most likely way to lose a
  measurement on a phone that is also somebody's phone.
- **The device had rotated to landscape.** The app window was `2283x903`, the swipe at
  `y=1900` landed off-screen, and the app logged **zero** `onTouchEvent` lines — while
  `mCurrentFocus` was correct throughout. **Focus is not sufficient.** Worse,
  `accelerometer_rotation` re-enables itself on this device within seconds of being set
  to 0, and `cmd window set-user-rotation` does not exist on this build, so the setting
  cannot be relied on at all — only the observed geometry can.
- **Reading the geometry *after* the run reported portrait for a landscape run.** The
  app had rotated in the seconds between the last swipe and the check. The gate now
  reads the surface the app is painting into immediately *before* the swipes and
  asserts it again afterwards. A single post-hoc check would have passed this run.
- **The logcat tag is `ZKiteGpuHarness`, not `zkite`.** A `-s zkite:V` filter returns
  nothing and looks exactly like an app that produced no frames.

### 40.7 Known gaps

- **This reduces the frequency of the spike, not its size.** A rebuild still re-enters
  CPython, re-runs `view()`, serialises a tree to JSON and re-parses it, and still costs
  10–14ms when it happens. §39.5's other two candidates — building only the rows that
  entered the window, and moving the window decision out of Python — attack the size and
  are untouched. At `overscan=4` a spike still lands roughly every fifth row crossed.
- **The default `overscan` was left at 4.** It is now a knob that genuinely trades
  frequency against size, so the right default is an empirical question this section did
  not answer, and changing it would have confounded the A/B above.
- **A rebuild is still synchronous with the frame.** Nothing here makes the window move
  off the render thread; §39.5's framing of that as an architectural option stands.
- **Coverage is not centring.** After a rebuild the window is recentred, but between
  rebuilds it drifts to one edge, so a direction reversal at the far edge rebuilds
  immediately. A hysteresis that recentres on reversal was not explored.
- **Unmeasured on the WebView backend.** The shared function changes both, and the e2e
  suite proves it still behaves, but no scroll-rate measurement was taken there.

## 41. Adaptive fling momentum — the scroll had exactly one gear

Owner-reported, in these terms: *"for the scrolling its flat acceleration but make it
adaptive acceleration."* Reported from the product, like §25, §26, §31 and §39 before it.

**The report is literally true of the code.** Momentum was exponential decay with a
single fixed time constant, `FLING_TAU = 0.325`, so total travel after release was
`v0 * FLING_TAU` — **exactly linear in release speed**. Every flick had the same
gearing: twice as fast went precisely twice as far, at 300dp/s and at 6000dp/s alike.
The old doc comment even stated the property as a feature — *"a harder flick travels
further, proportionally, with no separate fling distance formula needed"* — which is
true, and is the bug.

### 41.1 Why linear is wrong at both ends at once

A constant gear ratio is wrong in two directions simultaneously, which is why it reads
as "flat" rather than as "too fast" or "too slow":

- A **gentle** flick — the one used to nudge a list by a row or two — coasts far too
  far to place anything precisely.
- A **hard** flick — the one used to cross a long list — refuses to cover ground no
  matter how hard it is thrown, because the only way to get more distance is more
  speed, at a fixed exchange rate.

Real scrollers are superlinear. Android's `OverScroller` fits a spline whose fling
distance goes as `v^(d/(d-1))` where `d = ln(0.78)/ln(0.9) ≈ 2.358` — i.e. **`v^1.736`**.
Doubling the flick speed there covers 3.3x the distance, not 2x.

### 41.2 The change: `tau` becomes a function of release speed

```rust
fn fling_tau(v0: f32) -> f32 {
    let ratio = v0.abs() / FLING_REFERENCE_VELOCITY;          // 3000 dp/s
    (FLING_TAU_AT_REFERENCE * ratio.powf(FLING_TAU_EXPONENT)) // 0.325, 0.74
        .clamp(FLING_TAU_MIN, FLING_TAU_MAX)                  // 0.10, 0.55
}
```

Travel is `v0 * tau(v0)`, so distance goes as `v0^1.74` — the `OverScroller` exponent,
reproduced to within ~1% across 600–6000 dp/s. Checked numerically against the spline
exponent *before* the constants were written rather than tuned by feel:

| speed ratio | this | `OverScroller` | old fixed `tau` |
|---|---|---|---|
| 600 → 1200 (2x) | 3.30x | 3.33x | 2.00x |
| 800 → 2400 (3x) | 6.76x | 6.74x | 3.00x |
| 1500 → 6000 (4x) | 11.16x | 11.10x | 4.00x |

Three decisions worth stating:

**`FLING_REFERENCE_VELOCITY` is the fixed point, not a new number to tune.** At
3000dp/s the new `tau` equals the old constant exactly, so the one point on this curve
that had ever been calibrated by hand does not move — §41 changed the *shape* of the
response around a point it deliberately left alone. Device-measured: 3022dp/s → `tau`
0.327, travel 987dp, against the old 982dp.

**`tau` is resolved once, at release, and stored on the `Fling`.** Recomputing it per
tick from the *current* velocity would shrink it as the fling slows, which is no longer
exponential decay — and `tick_fling`'s exact-integral property (correct total travel
regardless of when a tick actually lands, §13.2) depends on `tau` being constant for
the animation's lifetime. This is the kind of thing that looks like a natural
refactor later, so the reason is in the code.

**The clamps deliberately restore linearity at the extremes.** Below ~610dp/s a fling
is a nudge and the floor stops it gliding an invisible distance; above ~6100dp/s the
ceiling bounds duration, because exponential decay has no natural end and a 3-second
fling reads as "the list ran away" rather than as momentum.

### 41.3 Verification on the Moto G54

`cmp_list_1k.py`, one flick per clean launch (force-stop, `pm clear`, relaunch, portrait
`1080x2223` asserted), fixed 1500px swipe with the duration varied to set release speed.
`adb shell input swipe` interpolates linearly, so measured release velocity tracks
`distance/duration` — and reproduced across both arms to within 1%, which is what makes
this a controlled comparison rather than two separate observations:

| swipe | measured v (before / after) | `tau` | travel before | travel after |
|---|---|---|---|---|
| 800ms | 745 / 746 dp/s | 0.116 | 242dp | **87dp** |
| 500ms | 1218 / 1217 dp/s | 0.167 | 396dp | **203dp** |
| 300ms | 1987 / 1988 dp/s | 0.240 | 646dp | **476dp** |
| 200ms | 2987 / 3022 dp/s | 0.327 | 982dp | **987dp** |
| 120ms | 5156 / 5102 dp/s | 0.481 | 1658dp | **2457dp** |

`tau` spans 0.116–0.481 against a constant 0.325 — a value the control cannot produce
at all, which is what makes the log line discriminating rather than merely consistent.

**The on-screen consequence**, from screenshots of the settled list after an identical
flick on each build (the drag itself contributes ~10.6 rows, so the fling's own travel
is the row number minus that):

| flick | before | after | fling travel |
|---|---|---|---|
| slow, ~746dp/s | Item **15** | Item **12** | 4.3 → 1.4 rows (~3x shorter) |
| hard, ~5150dp/s | Item **41** | Item **61** | 30.3 → 50.4 rows (~1.7x further) |

The control's numbers were predicted before it was built — 15.0 and ~40 from
`v * 0.325` — and it landed on 15 and 41, which is the check that the model of the *old*
behaviour was right and not just the new one.

### 41.4 Known gaps

- **No host test.** `gpu_harness` does not build for the host (§15.5), so `fling_tau` is
  verified by the device table above and by a numerical model of the curve, not by a
  unit test. Moving it into a host-testable crate is the §35 move and was not done here;
  it is pure arithmetic with no dependencies, so it is a cheap one.
- **The WebView backend is untouched.** It scrolls in the DOM and gets the platform's
  own momentum, which is *already* superlinear — so the two backends were divergent
  before this section and are now closer, but nothing here measured the WebView curve to
  say how close. §12/§13's head-to-head predates both this and §39's bug.
- **Only the fling was made adaptive.** A finger-down drag remains 1:1 with the finger,
  which is correct for direct manipulation and is not what "flat" referred to.
- **`FLING_TAU_EXPONENT` is fitted to Android, not to this owner.** It reproduces
  `OverScroller`; whether the owner wants `OverScroller` or something livelier is a feel
  judgement that only they can make, and the single constant to turn is named.
- **Clamp boundaries are unmeasured on a real finger.** 610 and 6100dp/s come from the
  constants, and every velocity in the table above came from synthetic `input swipe`.
  A real thumb flick's velocity distribution was never sampled.
- **Overscroll is still absent** (§5 item 4), so a fling that reaches a bound stops dead
  rather than rubber-banding — more visible now that hard flicks travel much further.

## 42. Where the dropped frames actually are — and the second mitigation §39.5 named that would have made things worse

§39.5 left four measured reasons the app is not yet native-smooth. §40 took the first.
This section takes the next one on its list — *"a drag is not vsync-paced… pacing a drag
the way a fling is paced is the obvious experiment and was not run"* — which §39.5 called
the cheapest of the three and the one most likely to be *felt*.

**It was run. It makes things worse.** Not "no better": measurably, reproducibly worse,
on every window of every arm. That is the same shape as §40.1, one item further down the
same list, and it is the second time in three sections that a mitigation this document
proposed for its own reported symptom would have aggravated it.

The reason both went wrong is the same, and it is worth stating once at the top: **each
named a mechanism without measuring whether that mechanism was the one operating.**
§40.1 assumed `overscan` stalled the window (it shifted it). §39.5 assumed a drag's
frames arrive whenever touch and work allow (they arrive one per refresh, already
aligned). Neither assumption was expensive to check. Neither was checked.

### 42.1 The instrument: a distribution, because a mean and a rate cannot see one

§39 added `worst:` because a mean of sixty frames cannot see one hitch. §40 added
`scroll:` because a cost line cannot see a rate. The gap left over is that
`avg 17.48ms max 39.41ms` is *equally consistent* with two different worlds:

- one frame in twenty takes two refreshes and the rest are perfect, or
- most frames run slightly over budget and nothing is dropped at all.

The first is a latency problem, the second a throughput problem, and they have opposite
fixes. No line in the log could tell them apart. So two more, and they are the section:

```
  budget: 55 on-time / 3 late-1 / 0 late-2+ (5% late, 2 of 3 late frames rebuilt)
  slack:  idle 4.55ms/frame = 27% of the 56 unpaced frames' elapsed time; fling 2; moves 87 in / 30 coalesced away
```

`budget:` buckets each frame interval by **how many refresh periods it occupied**, and
cross-tabulates the late ones against §40's rebuild flag. The cross-tab is the point:
knowing that 7% of frames rebuild and 8% are late says nothing whatever about whether
they are the *same* frames, and that is the entire question.

`slack:` is the discriminator for the pacing proposal specifically. Pacing can only help
a thread that is sometimes *waiting*; if this thread never waits, `next_frame_wait()`
returns `ZERO` every time it is consulted and the change is inert by construction. So
the wait itself is timed.

**Two decisions inside it that changed the answer, both found by running it:**

- **Bucket by rounding to the nearest period, not by flooring past a tolerance.** The
  first form put a 30ms interval — which cannot be a single 16.67ms refresh under any
  amount of jitter — in the on-time bucket. An instrument built to find dropped frames
  must not err toward reporting none. `thirty_ms_is_two_refreshes_not_one` is what
  rejected it.
- **Express idle against elapsed time, not against the nominal budget.** The budget form
  was written first and read `idle 3.81ms/frame (23% of budget)` on an emulator run whose
  frames took **235ms** and whose thread was 98% busy — 3.81ms is a large fraction of a
  notional 16.67ms and nothing at all of the 235ms actually spent. Overstating slack is
  precisely the misreading this line exists to prevent, since slack is what decides
  whether pacing can do anything.

Fling frames are excluded from `idle` and counted separately. During a fling the loop
waits on `next_frame_wait` **on purpose** (§13.2); counting that as slack would report
spare time in the window *because* its frames were already paced, which is the answer
this number exists to test for.

### 42.2 The finding: the late frames are the rebuild frames

Moto G54 over USB, `cmp_list_1k.py` (1,000 rows, 56dp, `overscan=4`), 14 sustained
swipes, GPU path, seven sixty-frame windows. Gates per §40.6 — portrait `1080x2223`
asserted before *and* after, `mCurrentFocus` and pid identical at both ends, 457
`onTouchEvent`s, **0** `cpu-fallback` frames.

| | count | share |
|---|---|---|
| frames bucketed | 418 | |
| on time (1 refresh) | 383 | 91.6% |
| late by one refresh | 29 | 6.9% |
| late by two or more | 6 | 1.4% |
| **late, of which rebuilt `view()`** | **25 of 35** | **71%** |
| tree rebuilds | 28 of 420 | 6.7% |

**89% of rebuild frames are late; 2.6% of everything else is.** The dropped frames are
not scattered across the drag — they are, almost exactly, the frames on which the
virtualised window moved. §40 made that event rare (53.6% → 8.8% of frames); it remains
the only thing that reliably misses.

The steady state is also better than §39.5 recorded and that is worth correcting in
place: summing arm A's phases gives **~12.8ms of work**, not the ~15.5ms §39.5 measured
before §40, leaving ~3.9ms of headroom. `idle` (2.15–6.01ms, mean 4.50) and the work sum
to the frame interval, which is the arithmetic check that the two lines describe the
same frames.

### 42.3 Drag pacing, built and measured — a regression

One arm, one behaviour changed: `TouchMove` updates the scroll offset immediately but
no longer paints on arrival; the frame is owed to the next `next_frame_wait()` deadline,
exactly as a fling tick is. Same source otherwise.

| | arm A (paint on touch) | arm B (paced) |
|---|---|---|
| frame-interval average, 7 windows | **17.37 ms** | 18.08 ms |
| late frames | 35 / 418 (8.4%) | **40 / 418 (9.6%)** |
| idle, mean | 4.50 ms | 5.33 ms |

Worse on every window, and `idle` rising is the mechanism visible in the instrument: the
deadline *adds* waiting. A touch that arrives shortly before the deadline is held rather
than painted, and the frame's own work then pushes past the following vsync.

**Why the premise was wrong.** `moves 52–87 in` against 60 frames, with **0 coalesced
away in five of seven windows** — Android delivers roughly one `TouchMove` per refresh,
already vsync-aligned, because input is batched onto the display's own cadence before it
ever reaches this thread. The loop was therefore *already* paced, by the input stream
rather than by a timer, and the 2–6ms of idle is it waiting for the next touch, not
spare capacity to schedule into. Adding a second pacer to a pipeline that already has
one is precisely what §13.2 removed when it deleted the fixed `recv_timeout(FRAME_INTERVAL)`
— the same mistake, arrived at from the other direction.

The change is **not kept**. `next_frame_wait` remains fling-only.

### 42.4 `overscan`: the empirical question §40.7 left open, answered

§40.7 left the default at 4 and said the right value "is an empirical question this
section did not answer". It is answerable now, because `budget:` can see the trade that
`sync`'s average cannot. Same Rust in all three arms — the provenance hash
`e58f9b45b975` is identical across them, so only the app's `overscan` differs.

| `overscan` | rows/rebuild | nodes | rebuild rate | late | late-2+ | `sync` worst | **avg interval** |
|---|---|---|---|---|---|---|---|
| **4** | 25 | 152 | 6.7% | 8.4% | 6 | 8.8 ms | **17.37 ms** |
| 8 | 33 | 200 | 3.6% | 6.0% | 8 | 11.7 ms | 17.40 ms |
| 16 | 49 | 296 | 1.7% | 5.0% | 14 | 17.9 ms | 17.76 ms |

Raising it does exactly what §40 promised — rebuilds fall monotonically, and so does the
count of late frames. **And it buys nothing**, because the frames that are still late are
bigger (`late-2+` rises 6 → 8 → 14) and every ordinary frame now carries more nodes. The
number a user feels is flat to 8 and worse by 16.

**The default stays at 4.** Recorded as an answer rather than a deferral: `overscan` is a
real knob, it trades frequency against size honestly, and on this workload the trade is
priced almost exactly evenly — which is why turning it is not the fix.

### 42.5 What a rebuild costs, and what that predicts

The three arms above are three points on one curve, and they fall on a line:

| rows built per rebuild | 25 | 33 | 49 |
|---|---|---|---|
| `sync` worst (median over windows) | 8.8 ms | 11.7 ms | 17.9 ms |

`sync ≈ 0.38 ms x rows − 0.7`. The intercept is **indistinguishable from zero**, and that
is the useful result: a rebuild's cost is per-row work, *not* a fixed toll for re-entering
CPython and re-parsing a tree. The 49-row point was predicted from the first two (17.2ms)
before it was measured (17.9ms) rather than fitted afterwards, which is what makes it a
test of the model instead of a description of it.

So the untouched candidate from §39.5 and §40.7 — **build only the rows that entered the
window** — is the one with the arithmetic behind it. At `overscan=4` a rebuild rebuilds
25 rows when about 5 have entered; the model puts 5 rows at ~1.2ms, comfortably inside
the ~3.9ms of headroom, where 25 rows at 8.8ms overruns it by more than 2x. That is the
next section's work, and it now has a number attached to it rather than a hope.

> **Corrected by §43, and the correction is large.** Two things above are wrong, both
> because `sync` was treated as one number. **44% of that 8.8ms was `json.dumps` running
> CPython's pure-Python encoder** — an import-order bug in `runtime.py`, not per-row work
> at all, and not something building fewer rows could ever have removed. Fixing it took a
> rebuild's Python cost to **2.09ms** with no row-diffing. And the frame also pays a
> *second* `build`+`layout`+`positioned` on every rebuild, which this arithmetic never
> counted; at **1.79ms** that `relayout` is now a rebuild frame's largest single
> component, larger than the row-building this section ranked first. The per-row model
> itself survives — it is linear, with an intercept indistinguishable from zero down to 5
> rows on the host — but it was fitted over a cost that was mostly a defect. See §43.5.

### 42.6 `zkite_frametiming` — and closing §41.4's first gap on the way

`gpu_harness` does not build for the host (§15.5), so nothing written in it can carry a
test. §41 shipped `fling_tau` — the whole of that section's change — verified only by a
table of device measurements, and said so in its own known-gaps list: *"moving it into a
host-testable crate is the §35 move and was not done here; it is pure arithmetic with no
dependencies, so it is a cheap one."* §42's bucketing had the identical problem, which
made it two functions, which made it worth doing.

`phase_3/frametiming/` — no dependencies, pure functions over `f32` and `Duration`,
**11 tests**. Both negative controls run:

| control | result |
|---|---|
| bucketing reverted to tolerance-and-floor | **2 fail** — 30ms read as one refresh, 45ms as two |
| `fling_tau` reverted to §41's fixed `FLING_TAU` | **4 fail** |

`the_reference_velocity_is_the_fixed_point` passes under **both** fling implementations
and is kept deliberately, on §40.5's argument: it pins §41's *compatibility* claim — that
3000dp/s returns the one hand-calibrated constant unchanged — which is true of both by
construction, and a test that discriminated there would mean §41 had moved something it
promised not to.

`build.sh` picked the new crate up with no edit: the source set is derived by
`cargo metadata`, and the provenance record went from 40 files to 43 by itself.

### 42.7 Verification

Host, all green and at the counts NEXT_SESSION.md records: `zkite_py` 182, cli 54,
bundler 27, renderer 62, imagestore 8, `zkite_pybridge` 20 (`--test-threads=1`, §29.5),
twcore 77, **`zkite_frametiming` 11 new**. Conformance 35/35 fixtures x 2 dprs with
§13.1.2's numbers unmoved to two decimals and §38's four gradient fixtures still 0.00%
loose.

Device: five runs, one voided. The shipping build was re-measured after restoring clean
source and reproduces arm A (mean 17.29ms, late 8.4% against 17.37ms and 8.4%).

**The emulator earned its place again** (§34's lesson, not §29.8's). It cannot time this
renderer — SwiftShader turned in 235ms frames — but it was where the idle-vs-budget
defect in §42.1 surfaced, because a 98%-busy thread is exactly the regime in which that
line lied, and the phone never enters it. It also could not exercise the `fling` counter
at all: at 235ms per frame, coalescing discarded 92% of touch moves and left too little
velocity history to launch one. That path was first exercised on the phone.

**The voided run was caught by the geometry gate, not by inspection.** The device had
rotated to landscape (`2283x903`) between runs while `mCurrentFocus` stayed correct
throughout — §40.6's trap, reproduced exactly, and the reason that gate exists.
`accelerometer_rotation` plus `user_rotation` did hold it portrait on this build once set
together, which §40.6 had found unreliable on its own; the observed geometry is still the
only thing worth gating on.

### 42.8 Known gaps

- **The fix this section argues for is not built.** Building only the rows that entered
  the window is quantified (§42.5) and untouched. So is moving the window decision off
  the render thread.
- **`sync`'s linearity is fitted over 25–49 rows, on one workload.** The extrapolation to
  ~5 rows is outside the measured range, and a per-rebuild fixed cost too small to see at
  25 rows would dominate at 5. The prediction is a prediction.
- **Drag pacing was tested in one form.** Deferring the paint to a deadline measured from
  the last frame's *start* is what §13.2's fling path does; aligning to vsync *phase*
  through a real `Choreographer` callback is a different mechanism and is still what
  §12.7 item 2 literally asked for. This section refutes the cheap form, not the idea
  that phase alignment could matter.
- **`overscan` is priced on one list shape.** 56dp rows, 889dp viewport, 152 nodes. A
  taller row or a heavier `builder` moves the per-row constant and could reprice the
  trade.
- **Still one device, one app shape, synthetic input.** `adb shell input swipe`
  interpolates linearly and a real finger does not; the "one move per refresh" finding in
  §42.3 is a property of Android's input batching and should hold, but it was observed
  through injected events.
- **The WebView backend is unmeasured here**, as it was in §40.7. §12/§13's head-to-head
  now predates §39, §40, §41 *and* this.
- ~~**Overdraw remains unmeasured** — §39.5's fourth item, untouched by §40 and by this.
  `present` at ~7.2ms is still the largest phase and is still CPU-side strip generation
  for the whole surface every frame.~~ — **§46: it is not CPU-side strip generation.**
  That is 0.67ms of it; `finish`+`submit` is 5.29ms and is 45% of the whole frame.
  Overdraw costs 0.000ms of `present`.

## 43. The rebuild was 44% a pure-Python JSON encoder — an import-order bug, six sections old

§42.5 priced the one remaining Queue H candidate off `sync` as a single number
(`~0.38ms x rows`) and predicted that building only the ~5 rows that entered would cost
~1.2ms. This section set out to check the mechanism before implementing the fix, per
§40.1 and §42.3.

**The mechanism was not the one named.** 44% of a rebuild's Python time was `json.dumps`
running CPython's **pure-Python** encoder, because `runtime.py` imported `json` 112 lines
before it installed the finder that makes `_json` importable — and `json/encoder.py`
binds its C accelerator once, at import time, in a `try/except ImportError`. Nothing
errored and nothing logged. Fixing the import order cut a rebuild's Python cost from
**9.09ms to 2.09ms** and late frames from **13.8% to 4.4%**, with no architectural change
and no row-diffing.

This is the third consecutive section in which the document's own named mitigation was
not the operating mechanism. The difference is that §40.1 and §42.3 found the proposed
fix *harmful*; this one found it aimed at the smaller half of a cost whose larger half
was a bug.

### 43.1 The instrument: `sync` was one number covering four different things

`sync` is `sync_viewports`, and §42.5's model treats it as one cost proportional to rows.
It is actually the CPython call (`view()`, the row `builder`s, the `to_tree` walk and
`json.dumps`), plus `RawNode::from_json`, plus — on rebuild frames only — a *second*
`layout_frame()` that `redraw`'s loop runs because the tree changed. Only the first is
work that building fewer rows could remove.

So `FramePhases` gains `sync_py`, `sync_parse` and `relayout`, and a line:

```
  rebuild: 7 x 14.7KB  py 8.95ms  parse 0.13ms  relayout 1.71ms  (610us/KB py, 9us/KB parse)  poll 0.110ms x53
```

Three decisions inside it, each of which changed what it says:

- **Averaged over rebuilds, not frames.** `phases:` divides by frames, which at §42's
  6.7% rebuild rate understates a rebuild's own cost by ~15x. That is why §42.5 had to
  reconstruct the per-rebuild figure from `worst:` across three `overscan` arms; it is
  now read directly.
- **The "nothing to do" call is charged to `poll`, not to `py`.** The same CPython
  entry point answers "no rebuild needed" on ~93% of scroll frames. Folding those into
  `sync_py` and then dividing by the rebuild count would have multiplied their cost by
  ~15 and attributed it to the rebuild. `poll` is also the signature §40.4 named as its
  own distinct bug and nothing could previously see: it measures **0.103ms**, so the
  cheap path is cheap.
- **`/KB`, not `/row`.** The tree that crosses the boundary is the same bytes on the
  host as on the device, so a per-KB figure is directly comparable between them. That
  comparison is what found the bug.

### 43.2 A 10x anomaly that is specific to the interpreter

Host decomposition first, on the real `cmp_list_1k.py` shape at eight window sizes from 5
to 81 rows (x86_64, CPython 3.14):

| term | us/row | notes |
|---|---|---|
| `view()` + widget construction | **~0** | `List` is lazy; the `builder` runs inside `to_tree`, not in `view()` |
| `builder(i)` + the row's `to_tree` walk | 3.47 | |
| `json.dumps` of the whole window | 2.08 | |
| `RawNode::from_json` (Rust, serde_json 1.0.151) | 0.90 | |

Two results worth keeping independently of what follows. **"Re-runs `view()`" is a red
herring** — re-entering CPython and running the view costs 2us; every microsecond of a
rebuild is per-row work. And **the fit is linear down to 5 rows**, intercept 9us against
143us at 25 rows, which answers §42.8's worry that "a per-rebuild fixed cost too small to
see at 25 rows would dominate at 5" in the negative.

Then the device, on the same payload:

| | host | Moto G54 | ratio |
|---|---|---|---|
| CPython half | 11.0 us/KB | 608.8 us/KB | **55.6x** |
| Rust parse | 1.7 us/KB | 9.2 us/KB | **5.4x** |

5.4x is what a Cortex-A55 at 2.0GHz against a desktop core predicts, and the two numbers
are measured **on the same thread microseconds apart**, so core, clock and thermal state
are common-mode and cancel. That leaves a **10.3x slowdown specific to the interpreter**,
which no CPU difference can explain.

**The obvious explanations were checked and are not it.** The vendored interpreter is a
proper release build — `-O3`, `-DNDEBUG`, `Py_DEBUG=0`, `WITH_PYMALLOC=1`,
`WITH_MIMALLOC=1` — read out of its own `_sysconfigdata__android_aarch64-linux-android.py`
rather than assumed. And the host figure was best-of-60 in a hot loop, so cache warmth was
the other candidate: **running an 8MB cache-evicting workload between every rebuild
changed the host number by 1.07x**, and the first-ever call by 2.05x. Neither is 10x.

### 43.3 The finding: `json.dumps` was the pure-Python encoder

Splitting the Python half on the device (an experimental build, reverted — §43.7):

| | ms | share |
|---|---|---|
| `App.render_tree()` — interpreter work, per row | 4.84 | 56% |
| `json.dumps` — C, over the whole window | 3.75 | **44%** |

3.75ms for 15KB is **251 us/KB**. Against the host's C encoder scaled by this device's own
measured 5.4x CPU factor that should be ~20 us/KB. Against the host's **pure-Python**
encoder scaled the same way it should be ~156 us/KB. Measured on the host, the two
encoders differ by **7.8x** on this exact payload.

A probe in the same call settled it rather than leaving it inferred:

```
PROBE c_make_encoder=False c_scanstring='json.encoder' c_make_scanner=False
PROBE import _json OK -> '.../lib/arm64/libpymod__json.so'
```

**`c_make_encoder` is `False` while `import _json` in the very same call succeeds.** That
is the signature of this bug and of nothing else: the module is perfectly importable *now*
and was not importable at the moment `json` was first imported.

The cause is eleven lines apart in one file. `runtime.py` had:

```python
import io
import json          # line 19
...
_native_extension_count = _install_native_extension_finder()   # line 131
```

`_json` ships as `libpymod__json.so` (§11.8: Android's installer drops any `lib/<abi>/`
entry not named `lib*.so`, so every extension module is renamed and reached only through
`_ZKiteNativeExtensionFinder`). `json/encoder.py` and `json/scanner.py` each do
`try: from _json import ... except ImportError: <pure-Python fallback>` at module scope,
**once**. Importing `json` before the finder existed ran the `except` branch and pinned the
process to the pure-Python encoder *and decoder* for the life of the app.

The fix is to install the finder above every non-essential import. It is placed above all
of them rather than just above `json` because the idiom is not `json`'s alone — `pickle`,
`datetime`, `decimal`, `heapq`, `bisect`, `csv`, `statistics` and `zoneinfo` all use it.

**This is not a native-backend bug.** `runtime.py` is `include_str!`'d by `devshell`,
`appshell` and `pybridge` alike, so every ZKite app on both backends has been running a
pure-Python JSON encoder and decoder since §11.8 made extension modules loadable at all.

### 43.4 The measurement

Moto G54 over USB, `cmp_list_1k.py`, 14 sustained swipes, GPU path, five sixty-frame
windows per arm, one variable between them. Gated per §40.6 and §42.7 — portrait
`1080x2223` asserted before *and* after, `mCurrentFocus` and pid identical at both ends,
~283 `onTouchEvent`s, **0** `cpu-fallback` frames. No run was voided.

| | arm A (as shipped) | arm B (import order fixed) | |
|---|---|---|---|
| rebuild `py` | 9.09 ms | **2.09 ms** | **-77%** |
| rebuild `parse` | 0.13 ms | 0.14 ms | — |
| rebuild `relayout` | 1.75 ms | 1.79 ms | — |
| late frames | 41/297 = **13.8%** | 13/297 = **4.4%** | **-68%** |
| late frames that rebuilt | 29/41 (71%) | **1/13 (8%)** | |
| frame-interval average | 17.95 ms | 17.83 ms | -0.7% |
| `idle` | 4.25 ms/frame | 5.16 ms/frame | +21% |

**§42.2's headline is reversed.** It found that 89% of rebuild frames ran late and 71% of
late frames were rebuilds; a rebuild is now essentially never the reason a frame is late
(1 of 13, across five windows).

**And the frame-interval average barely moved, which is the honest headline.** 17.95 →
17.83ms. The reason is visible in the instrument that was built to answer a different
question: `idle` went *up* by 0.9ms. §42.3 established that the loop is paced by the input
stream — Android delivers roughly one `TouchMove` per refresh — so the average interval is
pinned near the input cadence and cheaper work converts into waiting, not into a shorter
frame. The gain is entirely in the distribution, which is precisely the distinction §42.1
built `budget:` to make: `avg 17.9` and `avg 17.8` describe a 3x difference in dropped
frames.

### 43.5 What this does to §42.5's arithmetic

§42.5's prediction — 5 rows at ~1.2ms — was built on a per-row cost that was 44% a bug.
Correcting §42.5 in place rather than leaving it to be quoted:

- A rebuild's Python cost is now **2.09ms**, not 8.8ms, for the same 25 rows.
- Of the remaining cost, `render_tree` is the per-row half that row-caching would attack.
  At the post-fix figure that is roughly 1.2ms of the 2.09ms, so building 5 rows instead
  of 25 predicts saving ~1.0ms.
- **`relayout` is now the largest single component of a rebuild frame at 1.79ms**, and
  §42.5's headroom arithmetic never counted it at all. It is the second
  `build`+`layout`+`positioned` that `redraw` runs because `sync_viewports` changed the
  tree, and it is paid on exactly the frames that were already the late ones.

So the ordering of Queue H's remaining work has changed: **`relayout` is now bigger than
the row-building cost it was ranked behind.** It also looks cheaper to remove — §11.2's
central invariant is that a `List`'s content height does not depend on its window, so the
viewport height a rebuild needs is already known from the previous frame and the sync
could run *before* the first layout rather than between two. That is a hypothesis with a
mechanism named and no measurement behind it yet, and by this document's own rule it is
not to be implemented until the mechanism is checked.

> **Checked, built, measured and reverted — §44.** The mechanism is real: the viewport
> height *is* window-invariant (pinned by a host test through real CPython), the sync
> *can* run before the first layout, and `relayout` goes to **0.00ms**. But the frame does
> not get cheaper — total work 12.438 → 12.454ms — because **the same CPython call costs
> 21% more at the top of a frame than in the middle of one**, with the fixed-work `poll`
> as the control at **+36%**. Net ~0.10ms/frame, below what an input-paced loop can show.
> This is the first of the four consecutive mechanism-checks (§40.1, §42.3, §43, §44) in
> which the named mechanism *was* the one operating and the fix still did not pay.

### 43.6 A hole in §37's provenance check, found by hitting it

Editing `runtime.py` and rebuilding produced a **different `.so`** — cargo's own
dependency tracking follows `include_str!` correctly — under an **identical source
digest**, `dc832638f8a8` before and after. `zkite doctor` said "verified".

§37 derives the source set from `cargo metadata`, which describes the *package* graph and
therefore finds each crate's `src` and nothing else. `phase_1/devshell/src/runtime.py` is
`include_str!`'d by `phase_3/pybridge` from outside every crate directory in the native
shell's graph, so it was never in the set. That is §37.1's own incident — a shell shipped
from source no longer in the tree — in the one place §37's check could not see it, and it
fails in both directions: a stale shell passes, and a fresh one is indistinguishable from
it.

Fixed by **deriving** rather than listing, on the same argument the file already makes for
why the crate set is derived: the recorder scans the collected `.rs` sources for
`include_str!`/`include_bytes!` and adds every in-repo target it resolves. The native
shell's record went 43 → 44 files and picked up `runtime.py` by itself; the WebView
shell's picked it up too, from `appshell/src/lib.rs`'s own `include_str!`.

No Rust change was needed and the §37 cross-language digest pin is untouched:
`source_inputs` is stored *in* the record and `provenance.rs` re-derives from that list,
so the collection logic is unchanged on both sides.

**Control, because a hash that moves is not yet a hash that tracks content:** appending one
byte to `runtime.py` moved the digest `abee6ad57cc7` → `ca3c72b995fd`, and restoring the
file returned it to `abee6ad57cc7` exactly.

### 43.7 Verification

Host, all green and at the counts NEXT_SESSION.md records: `zkite_py` 182, cli 54, bundler
27, renderer 62, imagestore 8, `zkite_frametiming` 11, `zkite_pybridge` 20
(`--test-threads=1`, §29.5), twcore 77. **`phase_1/devshell/tests` 4 → 6.** Conformance
35/35 fixtures x 2 dprs with §38's four gradient fixtures still 0.00% loose. **The
webruntime e2e suite was re-run and is 39/39** — not optional this time, because
`runtime.py` is shared and the WebView backend runs the same import.

**The negative control is the point of the new tests.** `ImportOrderTest` makes `_json`
genuinely unreachable until a `_ZKiteNativeExtensionFinder` appears on `sys.meta_path` —
the device's actual condition, since the host's `_json` is a builtin and cannot be hidden
by path manipulation the way `test_finder_imports_a_renamed_extension_module` hides
lib-dynload — and then executes the **real, shipping** `runtime.py`:

| arm | `c_make_encoder` / `c_make_scanner` |
|---|---|
| shipping `runtime.py` | **True / True** |
| the same file with `import json` moved back to the top block | **False / False** |

The control failed first, for the right reason and not the one intended: the positive case
also read `False`, because on a host nothing stages a `libpymod_*.so` and so no finder is
ever installed and the gate never opens. Staging one, as the sibling test class already
does, is what made the test discriminate rather than merely fail.

Device: four runs, none voided. The shipping build was rebuilt from clean source after the
experimental probe was reverted and re-measured, and reproduces arm B (`py` 2.21ms, late
5.1%, against 2.09ms and 4.4%) rather than arm A's 9.09ms and 13.8% — which is the check
§42.7 established and the reason the probe could be an experimental build at all.

### 43.8 Known gaps

- ~~**Only `json` was checked**, and startup-time imports are believed harmless but
  unmeasured.~~ — **measured, §43.9.** The hazard set is **29 stdlib modules**, and the
  interpreter startup set is **disjoint from it**. The `while it was unmeasured` version
  of this entry named eight modules from memory and got two of them wrong (`datetime` and
  `csv` do not carry the idiom at module scope in 3.14; `hashlib`, `hmac`, `queue`, `ssl`
  and `ctypes` do and were not named). The robust form — installing the finder from Rust
  before `RUNTIME_GLUE` executes — is still not done and is now belt-and-braces rather
  than a known hole.
- **The 10.3x interpreter anomaly is now ~2.5x and still unexplained.** The fix removed the
  `json` half; `render_tree` remains ~4.8ms on device against ~0.1ms on host. A release
  build, cache warmth and CPU clock have all been ruled out as the whole story. An
  in-order Cortex-A55's indirect-branch behaviour under CPython's dispatch loop is the
  leading hypothesis and is **(unverified)** — it would need a microbenchmark separating
  allocation-heavy from branch-heavy Python inside the app.
- **`relayout` is measured but untouched**, and is now a rebuild frame's largest component
  (§43.5). Removing it is a hypothesis with a named mechanism and no measurement.
- **Late frames are down to 4.4% but `late-2+` did not improve** — 6 in arm A, 9 in arm B,
  and 12 of arm B's 13 late frames are *not* rebuilds. Whatever causes those is a
  different mechanism from the one this section fixed, and nothing here identifies it.
- **The WebView backend is unmeasured, again.** It runs the same `runtime.py` and the same
  degraded encoder, so it should gain too — from `_zkite_scroll_html` and every
  `json.dumps`/`json.loads` on that path. Not measured. §12/§13's head-to-head now
  predates §39, §40, §41, §42 *and* this.
- **Startup was not re-measured.** §17.1 found bytecode caching to be the largest startup
  cost; a pure-Python JSON decoder was in force for that measurement too, and nothing here
  re-ran it.
- **Still one device, one app shape, synthetic input**, as §42.8.
- **The include-macro scan is one pass and string-literal only.** An `include_str!` whose
  path is built by a macro, or one inside a file that is itself included, is not followed.
  Neither exists in this tree; the second would be worth failing loudly on.

### 43.9 The blast radius, measured rather than asserted

§43.8's first entry closed the `json` case and then asserted that anything imported
during interpreter startup is unaffected, "believed harmless… not measured". That is
exactly the shape of claim this document keeps finding to be wrong, so it is now
measured, against the vendored Android stdlib that actually ships and the 67 extension
modules `build.sh` actually stages beside it.

**29 stdlib modules carry the idiom at module scope:**

```
_pylong  asyncio.events  asyncio.futures  asyncio.tasks  bisect  concurrent.futures
ctypes  ctypes.macholib.dyld  decimal  getpass  hashlib  heapq  hmac  json.decoder
json.encoder  json.scanner  mailbox  pathlib._os  pickle  queue  quopri  shutil  ssl
statistics  subprocess  xml.etree.ElementTree  xmlrpc.server  zipfile  zoneinfo
```

**The interpreter startup set is 33 modules and is disjoint from it.** So `runtime.py`
installing the finder above every non-essential import does cover everything reachable,
and the entry §43.8 wrote as a hedge is closed rather than outstanding.

Two things the measurement corrected that a list written from memory got wrong. §43.8
named `datetime` and `csv` as at risk and **neither carries the idiom at module scope in
3.14**; it did not name `hashlib`, `hmac`, `queue`, `ssl` or `ctypes`, which do. And
`zipimport` — which really *is* imported at startup, and really does lazily import `zlib`
— is **not** at risk, because its `try` sits inside `_get_decompress_func` rather than at
module scope, so it is retried on every call and picks `zlib` up the moment it becomes
importable.

**That scope distinction is the whole correctness of the check**, and the first version
of the scan did not make it: walking every `Try` node in the file reported 34 modules
including `zipimport`, `random`, `tarfile` and `pty`, i.e. it reported a live
startup-time collision that does not exist. An instrument that over-reports a hazard is
not obviously safer than one that under-reports it — it would have sent the next session
after a bug that was never there.

Kept as two tests in `phase_1/devshell/tests` (**6 → 8**), which snapshot `sys.modules`
at the instant the finder is appended — by substituting a `list` subclass for
`sys.meta_path`, since a plain list's `append` is read-only — and check every module in
it against the Android stdlib's own source. The negative control is that the pre-fix
ordering must be caught *by this generalised test* and not merely by the `json`-specific
one; it is, reporting `json.encoder -> _json`. The positive test additionally asserts it
is examining more than 20 modules, so it cannot pass by measuring nothing.

## 44. `relayout` removed — and the frame did not get cheaper, because the cost moved

§43.5 left one named, unbuilt candidate at the top of Queue H: **`relayout`**, the second
`build`+`layout`+`positioned` that `redraw()` runs because `sync_viewports` changed the
tree, measured at **1.79ms** and, after §43 took the JSON encoder out, the largest single
component of a rebuild frame. §43.5 also named the mechanism to remove it — report
viewports *before* the first layout, using the height the previous frame measured — and
marked it **(unverified)**.

**It was built. `relayout` is now 0.00ms, and nothing a user feels improved.** Total
per-frame work moved 12.438 → 12.454ms. The layout work really does come out; roughly
40% of it goes straight back in because **the same CPython call costs 21% more when it
runs at the top of a frame instead of in the middle of one**, and what survives is
~0.10ms/frame — below what an input-paced loop can express.

The change is **not kept**, on §42.3's precedent. What is kept is the mechanism test, and
the measurement below.

### 44.1 The mechanism, checked first — and this time it held

Three consecutive sections (§40.1, §42.3, §43) found a named mitigation aimed at a
mechanism that was not operating. So the mechanism was checked before anything was built,
and the check is a host test rather than an argument:
`zkite_pybridge::a_lists_viewport_height_does_not_change_when_its_window_does`.

`sync_viewports` needs two numbers per scrollable container. **`scroll_y` is not a layout
output at all** — it is `scroll_offsets`, which the touch handler wrote before `redraw()`
was called, so it is exact before any layout runs. Only `viewport_h` came from layout, and
§43.5's claim is that it cannot depend on which rows are materialised.

That is a claim about *layout*, and the sibling test that already existed
(`virtualised_list_windows_without_changing_the_scrollable_range`) does **not** imply it:
that one pins `max_scroll_y`, the **content** height, which is what §11.2 actually states
and what the spacers exist to preserve. Content height is a property of the list's
children; viewport height is a property of how its *parent* sized it. A `flex-1` list
under a `shrink-0` header is exactly the shape where a header that resized would move it.

So the fixture is `cmp_list_1k.py`'s own shape — same header, same `flex-1`, same 56dp
rows — because that is the app every device measurement since §40 was taken on. Driven
through real CPython, laid out at the device's real logical size (432 x 889.2, i.e.
`1080x2223` at density 2.5), either side of a 5000px scroll that moves the window off row
0 entirely: **the height is identical.**

**Two controls, because an equality assertion has two ways to pass vacuously** — §37's
`is_stale()` control is the precedent, having passed for the wrong reason:

| control | result |
|---|---|
| the window actually moved (`Item 0` present before, absent after) | holds |
| the height is a live layout output, not something inert — same tree, surface 100px shorter | moves by exactly 100.00px |

The second is the one that matters. A helper that returned a constant, or read a field
layout never fills, would report "invariant" for every tree ever laid out and could never
fail.

### 44.2 The change, and why the bad case costs nothing

`presync_viewports` runs before the first `layout_frame()`, driven by the **previous**
frame's live set of scrollables and their heights (`last_scrollables`, cleared and
refilled in place so a steady-state frame does not allocate). It does not replace
`sync_viewports` — it runs in front of it:

- **Guess right** (the steady state): `reported_viewports` already holds
  `(scroll_y, viewport_h)`, so the post-layout call **dedupes to nothing** — not one
  CPython call, not one extra byte across the boundary — and the frame lays out once.
- **Guess wrong** (first frame, resize, rotation, a tree that grew a new scrollable): the
  post-layout pass reports the true height and takes the second layout exactly as before.
  Today's behaviour plus one poll.

Driving it off the previous frame's **live set** rather than off `reported_viewports` is
deliberate: nothing ever removes a key from that map, so a `List` the app has navigated
away from would be re-reported to Python once per frame forever.

The device confirms the guess is always right on this workload: **`relayout` is 0.00ms on
every one of arm B's ten windows**, meaning `sync_viewports` never once returned "changed"
behind the presync.

### 44.3 The measurement

Moto G54 over USB, `cmp_list_1k.py`, 14 sustained swipes, GPU path, **two runs per arm,
ten sixty-frame windows and 594 bucketed frames each, 46 rebuilds each**. Gated per §40.6
and §42.7 — `frame: 1080x2223 via gpu` asserted before *and* after, `mCurrentFocus` and
pid identical at both ends, 284 `onTouchEvent`s, **0** `cpu-fallback`. **No run was
voided.** Arm A is clean `HEAD` (source digest `abee6ad57cc7`), arm B the presync
(`a1df8a3e252f`).

**Arm A reproduces §43's arm B**, which is what makes this continuous with the existing
numbers rather than a fresh baseline: `py` 2.05 against 2.09ms, `relayout` 1.70 against
1.79ms, late 4.9% against 4.4%.

| | A (`HEAD`) | B (presync) | |
|---|---|---|---|
| **rebuild `relayout`** | **1.701 ms** | **0.000 ms** | **−100%** |
| phases build+layout+positioned | 1.859 | 1.696 | −0.163 ms/frame |
| **rebuild `py`** | 2.047 | **2.475** | **+20.9%** |
| **`poll`** | 0.107 | **0.146** | **+35.8%** |
| phase `sync` | 0.292 | 0.352 | +0.060 ms/frame |
| **sum of phases** | **12.438** | **12.454** | **+0.016** |
| frame-interval average | 17.691 | 17.773 | +0.5% |
| frame-interval max | 41.85 | 46.72 | +11.6% |
| late-1 / late-2+ | 16 / 13 | 10 / **16** | |
| late total | 29 (4.9%) | 26 (4.4%) | |

### 44.4 The finding: `poll` is the control, and the cost is positional

`py` rising is ambiguous on its own — a rebuild at the top of a frame might differ from one
in the middle for a dozen reasons. **`poll` is not ambiguous.** It is the identical CPython
entry point doing the identical two dict lookups (`runtime.py`'s `_zkite_scroll_tree`
answering "nothing to do"), on ~548 frames per arm. The presync changes nothing whatever
about what it does — only *when in the frame* it runs. It got **36% slower**.

Work held constant, position varied, cost moved. That is a measurement of position, not an
inference from one.

**The arithmetic closes in both directions**, which is the check that these two lines
describe the same frames:

| | predicted | observed |
|---|---|---|
| layout work removed | `1.701 x 46/594` = 0.132 ms/frame | 0.163 |
| CPython penalty added | `(0.428x46 + 0.038x548)/594` = 0.068 ms/frame | 0.060 |

Net ~0.10ms/frame of real saving on a 12.44ms frame — and invisible in the frame interval
for the reason §42.3 established and §43.4 measured again: the loop is paced by the input
stream, so cheaper work converts into `idle` (5.227 → 5.381ms) rather than into a shorter
frame. `present` moved +0.125ms/frame between arms while being untouched by this change,
which is the run-to-run noise floor and is larger than the entire remaining win.

**Why position costs anything is a hypothesis, not a measurement. (unverified)** In arm A
the CPython call ran after ~1.86ms of Rust had already executed on that core following the
~5.3ms idle wait; in arm B it is the first thing after that wait. §43.2 measured this
family on the host — a first-ever call at 2.05x, an 8MB cache-evicting workload at 1.07x —
and DVFS ramp on an in-order Cortex-A55 coming out of an idle block is the obvious
device-side analogue. Cache warmth and clock ramp were **not** separated here, and doing so
would need a microbenchmark that varies the amount of work preceding the call.

That the Rust phases fell by *more* than the relayout removal alone predicts (0.163 against
0.132) is weak corroboration from the other side — they now run after the CPython call has
warmed the core — but it is within noise and is not offered as evidence.

**Why the change is not kept.** Zero net gain; `late-2+` 13 → 16 and the worst frame
interval +11.6%; it adds state and a second call site; and it inflates `py` by 21%, which
is precisely the cost the next item on Queue H ("build only the rows that entered")
attacks. §42.3's drag pacing set the precedent for building, measuring and discarding.

### 44.5 A `#[cfg(test)]` edit changes the shipped `.so` — and it is metadata

Restoring `render_thread.rs` to `HEAD` and rebuilding did **not** reproduce arm A's
`libzkitegpuharness.so`, which is alarming until it is chased down: the only remaining
difference was the new test in `zkite_pybridge/src/lib.rs`, which `cfg(test)` excludes from
the cdylib entirely.

Three checks, in order, because "the build succeeded" has been wrong six times (§15.6.1):

| check | result |
|---|---|
| is the build reproducible at all? rebuild, same source | **identical** (`a925e533`), so the difference is real |
| revert the test, rebuild | `.so` returns to **`6508a0bf`** — arm A's bytes exactly, digest back to `abee6ad57cc7` |
| strip both and compare | **byte-identical** (`9498a14b`) |

So the executable code is unchanged and the difference is DWARF metadata — the source
file's own checksum and line table, which LLVM embeds per file. **A test-only edit cannot
change shipped behaviour**, and §37's source digest is conservative in the safe direction:
it flags a source change that provably cannot alter the artifact, which is the right way
for that check to err.

Worth recording for one reason beyond the reassurance: §37.1 established provenance by
recovering an old `.so` from an APK and reproducing it *byte-identically*. That technique
is sound, but this shows the comparison has a metadata component, so a future incident
where two `.so`s differ is not yet evidence that their code does — strip first.

Incidentally measured on the way: the shipped `.so` is 14.2MB of which **4.5MB is debug
metadata**, against a 41.0MB APK. Nothing here acts on that.

### 44.6 Verification

Host, all green and at the counts NEXT_SESSION.md records: `zkite_py` 182, `devshell` 8,
cli 54, bundler 27, renderer 62, imagestore 8, `zkite_frametiming` 11, twcore 77.
**`zkite_pybridge` 20 → 21** — the mechanism test of §44.1, which is what this section
leaves behind.

The webruntime e2e suite and the conformance harness were **not** re-run, and that is a
deliberate scoping call rather than an omission: the reverted change was confined to
`gpu_harness/src/render_thread.rs`, which is native-only and which neither harness
exercises, and the surviving change is a `#[cfg(test)]` test in `zkite_pybridge` proved
above to leave the artifact's code byte-identical. §43 re-ran e2e because it changed
`runtime.py`, which both backends share; nothing here touches `zkite_py`, `runtime.py`,
`twcore` or either runtime.

Device: four runs, none voided. The tree was restored to `HEAD` and rebuilt from clean
source, reproducing arm A's `.so` byte-for-byte (§44.5) — a stronger form of §42.7's
restore check, which re-measured rather than compared bytes. `zkite doctor` is green,
which took rebuilding **x86_64** as well: rebuilding one ABI made the other stale and
§37's check said so, exactly as designed.

### 44.7 Known gaps

- **The 1.79ms is real and is still being paid** — this section removes it and hands most
  of it back, so `HEAD` keeps the second layout pass. A way to remove `relayout` *without*
  moving the CPython call to the top of the frame would still be worth ~0.13ms/frame.
  The obvious variant is unexplored **(unverified)**: run the presync after `build`
  (0.58ms of Rust, enough to warm the core?) but before `layout`, saving
  `layout`+`positioned` at the cost of a second `build`. The arithmetic puts it within
  noise of arm B, which is why it was not run — this measurement cannot resolve
  0.03ms/frame against `present`'s ±0.13.
- **The positional penalty is measured; its cause is not.** Cache warmth and DVFS ramp
  are not separated. This is the same open thread as §43.8's "10.3x interpreter anomaly is
  now ~2.5x and still unexplained", approached from a second direction, and a
  microbenchmark that varies the work preceding a fixed CPython call would speak to both.
- **The non-rebuild frames got worse and nothing in the design prevents that.** 92% of
  scroll frames pay a `poll` and have no `relayout` to save, so the presync is pure loss on
  them (+0.038ms). Only Rust knowing the covered window — §39.5's third candidate, "move
  the window decision out of Python", still untouched — removes the poll entirely rather
  than relocating it.
- **`late-2+` got worse again** (13 → 16), as it did across §43's fix (6 → 9). Two
  consecutive changes have moved the one-refresh drops in the right direction and the
  multi-refresh ones in the wrong one, and neither section identified the mechanism.
  §43.8's entry stands and is now twice-observed.
- ~~**Overdraw remains unmeasured** — §39.5's last untouched item. `present` at
  7.43ms/frame is still by far the largest phase, is 60% of the frame's total work, and
  its ±0.13ms run-to-run variation is larger than everything this section moved.~~ —
  **measured and refuted, §46.** `present` is 10% `render` and 45%-of-the-whole-frame
  `finish`+`submit`; overdraw costs 0.000ms of it and ~0.5ms of `scene`.
- **One device, one app shape, synthetic input**, as §42.8 and §43.8.
- **The WebView backend is untouched here** rather than merely unmeasured — the change was
  native-only. §12/§13's head-to-head still predates §39 through §43.

## 45. The animation driver, part 1 — the substrate, and the two channels it had nothing to animate

*Queue F, sliced. The owner asked for the animation driver and for a professional
architecture behind it rather than a feature. This section builds the time-domain core
and closes the gap that would have made it useless: the two properties an animation
driver needs first were parsed by `twcore`, stored in every `ComputedStyle`, and read by
no native paint code at all.*

**Why Queue F is first, argued rather than asserted.** Three reasons, none of which is
"it blocks five features" (which is true, and is the weakest of them):

1. **The substrate already exists and is already measured at 60Hz on this device.**
   `tick_fling` + `next_frame_wait` + `recv_timeout(deadline)` *is* a per-frame animation
   loop (§13.2, §41); the fling is one hard-coded animation running in it. Queue F
   generalises a proven mechanism rather than introducing one, which by this document's
   standards is an unusually low-risk architectural session.
2. **The central design decision was already answered by §44.3's table, with no device
   needed** — see §45.2. That is §40.1/§42.3/§43's discipline applied to an architecture
   choice instead of to a mitigation.
3. **An animation is the first workload this renderer has that is not input-paced.**
   Every measurement since §42.3 has been pinned near the input cadence, which is why
   §43's 7ms win moved the frame-interval average by 0.12ms and §44's whole effect was
   invisible in it. A self-paced driver is a better instrument for §44.7's unexplained
   `late-2+` than anything left on Queue H.

### 45.1 The finding: `opacity` and `transform` were parsed, stored, and painted by nothing

`twcore` compiles `opacity-*` to `Paint::opacity` (`decl.rs`, `cs.paint.opacity = v`) and
`rotate-*`/`scale-*`/`translate-*` to `ComputedStyle::transform` (`style.rs`). Both fields
have existed since phase 0. **Neither was read by `zkite_renderer`, by either `gpu_harness`
paint path, or by the conformance renderer** — checked by grep across all of `phase_3`
before anything was built, and then measured rather than left as a reading of the code.

The WebView backend compiles the identical class strings with the real Tailwind browser
build (`phase_1/webruntime/vendor/tailwind-browser.js`), so all of them have always worked
there. That is a §15.1 divergence of exactly the §14.2 shape — renders, nothing errors,
the behaviour is simply absent — and it had never been filed, for a reason worth naming:
**no conformance fixture covered either property.** §9's harness could not see it, and
**WIDGETS.md listed `opacity` under "Works on both"** (corrected in this session).

Measured on the host against real Chromium by adding the fixtures first and running the
harness before writing any paint code:

| fixture | before | after | |
|---|---|---|---|
| `opacity` (three levels over one background) | **42.18%** | **0.01%** | strict 42.25% → 0.06% |
| `opacity_group` (overlapping children under one `opacity-50`) | **78.17%** | **0.00%** | strict 78.23% → 0.04% |
| `translate` (a shifted node with a child, plus an unshifted control) | **15.19%** | **0.10%** | strict 15.35% → 0.33% |

**All 35 pre-existing fixtures are byte-identical between the two runs** — same
percentages, same pixel counts, same `max_delta`. That is the negative control: the change
moved exactly the three fixtures it targeted. Green at both dprs; the harness is now
**38 fixtures x 2 dprs**.

§26.4 is the corroborating detail from the other direction: the `Slider` thumb's
`-translate-x-1/2` was emitted as a plain pixel margin because "twcore cannot express it",
and the reason that workaround was never revisited is that a transform would not have
painted anyway.

### 45.2 Why the driver cannot be a Python controller — priced off §44.3, not guessed

The obvious API for an animation in a Python framework is a controller the app holds and
reads inside `view()`. **That architecture is ruled out by a measurement that already
existed**, and recording the arithmetic is the point:

- §44.3 arm A: total frame work **12.438ms**; a rebuild costs `py` 2.047 + `parse` 0.14 +
  `relayout` 1.701 = **3.888ms**, paid on 46 of 594 frames (**7.74%**).
- So an ordinary frame is ~**12.14ms**, and a frame that re-runs `view()` is
  ~**16.03ms** against a 16.67ms budget — 96% of it, with nothing left for `present`'s own
  ±0.13ms noise, and before §44.4's finding that the same CPython call costs **21% more**
  at the top of a frame.

The structural half matters more than the number: **`view()`'s cost scales with the whole
screen, not with the node being animated.** Fading one 48dp icon would cost what
rebuilding a 25-row list costs. So the constraint is not a preference:

> **No Python in an animation frame.** The app declares an animation once; whoever owns
> the frame samples it; Python hears back exactly once, when it finishes, down the event
> path it already has.

Two things follow that contradict what this document assumed:

- **§16.7's worry that "animating layout means re-running taffy per frame" is already
  false.** `redraw()` rebuilds the taffy tree and re-lays-out on *every* frame regardless
  — that is §44.3's 1.859ms `build`+`layout`+`positioned` line. A layout-affecting
  animation costs nothing beyond what is already paid.
- **Exit animations do not need the renderer to retain departed subtrees**, which is
  §16.7's other stated blocker. Python keeps the node rendered until the driver dispatches
  one completion event, then drops it: one Python call per animation, not per frame, and
  the same mechanism on both backends.

### 45.3 The architecture

Five principles, each traceable to something measured:

1. **No Python in the animation frame** — §45.2.
2. **One implementation of the semantics, not one per backend.** The timeline and curve
   arithmetic is one host-testable crate; the WebView side will be a port pinned to it by
   a numeric parity test, **not** CSS transitions. §32.1 is the precedent: `<label>` was
   rejected on a measurement because borrowing a browser mechanism buys its side effects,
   and `transition-*` would buy CSS's interruption behaviour, property-change triggers and
   `transitionend` timing — a new silent divergence bought for a feature meant to remove
   one.
3. **Animations are declared in the tree, not commanded.** The tree is the only channel
   both backends already share and is already the frame protocol.
4. **Reuse the mechanisms that exist.** A translate is the offset channel `positioned`
   already has for scroll and IME pan; a completion is an ordinary dispatch.
5. **Don't mimic HTML** — the model is ZKite's own explicit channels and triggers.

```
L0  zkite_anim   the time-domain core — THIS SECTION
L1  wire format an `anim`/`transition` attr on a node                      (F2)
L2  native driver Timeline in render_thread; overrides applied in build;
                 next_frame_wait = min(fling, animator); completions dispatched (F2)
L3  zkite_py     the author surface; Sheet/Dialog/Toast/Nav compose on it   (F2/F3)
L4  webruntime  rAF driver + JS port of L0, pinned by a host parity test   (F3)
L-1 PREREQUISITE actually paint paint.opacity and transform — THIS SECTION
```

**`zkite_anim` (`phase_3/anim/`, 47 tests, no dependencies).** A crate rather than a module
in `gpu_harness` for the reason §35 and §42 already established twice: `gpu_harness` does
not build for the host (§15.5), so anything living there is verifiable only on a device.
Nothing in it calls `Instant::now()` — every entry point that needs the time is handed
one, so a test advances 16.67ms at a step and gets an exactly reproducible answer.

- `Easing` — `Linear | CubicBezier | Steps`, with the four named presets carrying **CSS's
  own control points to the digit**, so the future JS port has a single published answer
  to be checked against rather than a second description to diverge from.
- `Spring` — closed form in all three damping regimes, with velocity. Closed form for the
  same reason `tick_fling` integrates exactly (§41): a per-frame integrator makes the
  result depend on how the frame times fell, and this thread has 39ms frames (§39) and
  235ms ones on the emulator (§42).
- `Track` — one value moving once, with delay, repeat and direction.
- `Timeline` — the running set, `advance(now)`, and **`next_deadline`**, which is what
  lets the loop keep a real deadline instead of "wake me every frame in case".
- `Interrupt::{Restart, Retarget, Ignore}` — the policy an animation driver lives or dies
  by. `Retarget` carries position *and* velocity across for a spring and position only for
  an eased track (documented, pinned, and the reason a grabbable sheet should be a
  spring). `Ignore` is what makes a *declared* animation idempotent, which matters here
  more than in most frameworks: the tree is rebuilt on every frame of a scroll, so the
  same `enter` spec arrives ~60 times a second.

**The paint work.** A translate is applied in `zkite_renderer::walk`, to the node's
resolved position — so it moves the node *and its subtree* (children derive their origin
from the shifted `x`/`y`), leaves layout untouched, and — because `hit_test` resolves
against `positioned()` — is **tapped where it is drawn** for free. Offsetting in the
painter instead would have left touch targets at the untransformed position: invisible in
a screenshot, only reproducible under a finger, which is §9.7/§13's exact failure shape.

CSS `opacity` is not like that. It composites a node *with its subtree* and fades the
result, so it is a layer spanning a range of the flat node list. `PositionedNode` gains
`subtree_len` (exact, because the pre-order walk makes a subtree contiguous by
construction) and `zkite_renderer::OpacityGroups` owns the "which layers are open"
arithmetic. **That extraction is the point, not tidiness:** the paint loop exists three
times in this repo — `paint_nodes_into_scene` (vello_hybrid), `paint_cpu_fallback`
(vello_cpu) and `conformance`'s own — against three scene types with no trait between
them, which is precisely how a property comes to be implemented in one and absent in the
others. Each painter now contributes only the two calls its backend spells differently.

**Group opacity, not inherited alpha, and `opacity_group` is the fixture that says which
shipped.** Multiplying each node's own alpha is the easier implementation and the one a
first draft reaches for; over a white panel on an emerald panel it leaves 25% emerald
mixed into the result and the left half comes out visibly green. It measured 78.17% wrong
before and 0.00% after. This also closes half of §31.8's "no opacity groups".

### 45.4 The device run

Moto G54 over USB, `demo_app/anim_channels.py`, GPU path. Gated per §40.6/§42.7:
`frame: 1080x2223 via gpu` asserted, `mCurrentFocus` and pid identical at both ends
(25367), **0** `cpu-fallback` frames, 0 logged errors. Screenshot bands read with
`pngtool`.

**Opacity composites arithmetically exactly**, on real PowerVR, against slate-950
`(2,6,24)` with emerald-500 painting as `(0,188,125)`:

| declared | predicted | measured |
|---|---|---|
| `opacity-100` | `(0,188,125)` | `(0,188,125)` |
| `opacity-50` | `(1, 97, 74.5)` | `(1, 97, 75)` |
| `opacity-25` | `(1.5, 51.5, 49.25)` | `(1, 52, 49)` |

**Group opacity is confirmed on device, and the two implementations are far apart.** Over
the white panel: group predicts `(128.5, 130.5, 139.5)`, inherited alpha predicts
`(128, 176, 165)`. Measured **`(129, 131, 140)`**.

**Translate is exact and its subtree rides along.** The control bar (no transform) spans
x40-339; `translate-x-8` spans x120-419, shifted by exactly **80px = 32dp = 2rem** at
density 2.5; `translate-x-4` spans x80-379 (+40px), and its rose child spans x80-149 —
starting at its parent's *translated* origin, not at the untransformed x40.

**And the half a screenshot cannot see.** A translated `Button`, three taps, band
comparison:

| tap | expected | result |
|---|---|---|
| where the button is **drawn** (270, 1256) | registers | band **changed** |
| where it would be **untransformed** (80, 1256) | does not | band **identical** |
| the untranslated control button (180, 1397) | registers | band **changed** |
| the control before those taps | untouched | band **identical** |
| the opacity bars, across all three taps | never change | **identical** x3 |

The last row is the §35 discipline — a band that must *not* change, read from the same
pair — and it is what makes the first two rows a measurement rather than two screenshots.

### 45.5 Two ways this session's own tooling was wrong before it was right

**`ZKITE_DEVSHELL=1` and `ZKITE_APP_MAIN` are mutually exclusive, and the loser is silent.**
The devshell branch of `build_apk.sh` stages no `app.zip` at all, so passing both builds
the generic Dev Shell and *ignores the app entirely* — no warning, exit 0. The first
device run duly measured the "waiting for `zkite dev`" screen, and the only thing that
gave it away was `ZKITEPERF first-frame ... nodes=6` against the fixture's 28. **This is a
seventh entry for §15.6.1's list**, and its shape is new: not "the build did not contain
the change" but "the build silently contained a *different app*". The `nodes=` count in
the first-frame line is the cheapest possible check and is already printed on every launch.

**A spring's settle predicate is not monotone in time, and bisecting it is wrong.** The
first `settle_time` searched for the first `t` at which the spring reports `settled`,
which quietly assumes monotonicity. A lightly-damped spring satisfies both the
displacement *and* velocity bounds near every turning point of its ring, long before it
has stopped ringing; `Spring { stiffness: 400, damping: 4 }` reported settled at 5.81s and
was demonstrably still ringing afterwards. Replaced with a closed-form inversion of a
decay envelope that bounds both quantities in all three regimes, which makes the answer an
**upper** bound — a property a test can state densely and check, which "the first moment"
cannot.

### 45.6 Verification

- `phase_3/anim` — **47 tests**, each with a negative control. The controls that earned
  their place: `easing_is_not_the_identity` (rejects a solver that returns `t`);
  `a_zero_slope_control_point_does_not_break_the_solver` (rejects Newton without
  bisection — `ease-out`'s `x` slope at `s=0` is exactly zero, which is the *start of the
  most common entrance curve in the framework*); `under_damped_overshoots_and_the_other_
  two_never_do` (rejects using the critical branch everywhere);
  `a_spring_crossing_its_target_at_speed_is_not_settled` (rejects dropping velocity from
  the settle conjunction — a spring would freeze mid-ring, exactly on target, looking
  correct); `fading_in_from_transparent_keeps_its_hue` (rejects straight-alpha colour
  interpolation, which drags a fade through black); `ignore_leaves_a_running_animation_
  alone_but_not_a_finished_one` and its paired control (rejects restarting a declared
  animation on every rebuilt frame, which never finishes).
- `zkite_renderer` — 62 → **67**. `a_translate_moves_the_node_and_its_subtree_and_
  displaces_nothing` asserts all three claims together because an implementation can
  satisfy any two; `a_translated_node_is_hit_tested_where_it_is_painted` has an
  untransformed control; `opacity_groups_open_and_close_over_exactly_their_own_subtrees`
  replays the documented painter usage and checks the result as a balanced sequence, with
  `a_tree_with_no_opacity_opens_no_layers_at_all` as the control that rejects wrapping
  every node in its own offscreen target.
- **Conformance: 38 fixtures x 2 dprs, green.** Three new, 35 unchanged.
- Everything else re-run and unchanged: `cli` 54, `bundler` 27, `imagestore` 8,
  `frametiming` 11, `pybridge` 21, `zkite_py` 182, `devshell` 8.
- `zkite doctor` clean after rebuilding both ABIs. It **caught a real staleness** on the
  way: the renderer change invalidated the x86_64 shell while arm64-v8a had been rebuilt,
  which is §37 working exactly as designed.

### 45.7 What this deliberately does not do

- **Nothing animates yet.** This section builds the driver's core and the channels it will
  drive; the timeline is not wired into `render_thread` and no wire format exists. That is
  F2, and splitting there is deliberate — the paint prerequisite is a *user-visible bug
  fix* that stands on its own and needed its own before/after measurement, which mixing it
  with a driver would have muddied.
- **`rotate-*` and `scale-*` are still parsed and dropped.** They need a real affine over
  an axis-aligned rect list *and* a `hit_test` that can invert it — a different piece of
  work from a translate, which is the offset channel `positioned` already had. Now filed
  in §15.1 rather than unknown.
- **No `transition-*`/`animate-*` parsing in `twcore`.** Deliberate until the model is
  settled (§45.3 principle 2); the classes still reach the unknown-class report.
- **The WebView backend is untouched.** It already paints all of this correctly, which is
  the whole finding — but `zkite_anim` has no JS counterpart yet, so F3's parity test does
  not exist.

### 45.8 Known gaps

- **A spring's rest thresholds are absolute, not per-channel.** `REST_DISPLACEMENT` is
  1e-3 in the channel's own units — sub-pixel for a dp translate, a tenth of a percent for
  an opacity — so "settled" means something different for `opacity` (range 1) and
  `translate_y` (range 320). It also gives a heavily over-damped spring a genuinely long
  tail: `Spring { stiffness: 180, damping: 60 }` covering 100 units is 99.7% done at 1.8s
  and does not formally settle until **3.7s**, during which the render thread would keep
  painting an invisible change. Recorded with its own test rather than tuned away, because
  it is a property of the spring the author chose; a per-channel threshold is F2's call.
- ~~**An opacity layer is an offscreen target per group.** Nothing measures what
  `push_opacity_layer` costs on this GPU, and a fade over a large subtree is exactly the
  case an animation will produce every frame. `present` is already 60% of the frame
  (§44.7) and this is a new way to add to it. **(unverified — no device measurement of an
  opacity layer's cost exists.)**~~ — **measured, §46.6: 1.88ms/frame**, for one group
  spanning the whole 1080x2223 surface, i.e. +29% on `present` and 16% of the frame's
  total work. Affordable for a 200–300ms transition (~28ms spread over ~15 frames);
  **not** affordable for a persistent full-screen group, which is the argument for
  `translate-*` as the default channel for anything long-lived. Its `swap` worst case
  reaches 9–11ms and is unexplained (§46.10). Measured for one size and one alpha only.
- **A transformed node does not grow its ancestor's scrollable overflow**, as CSS says it
  should. `scroll_extent` reads each child's unshifted layout. Nothing in `zkite_py` can
  produce that shape today, so this is filed rather than fixed.
- **`opacity-0` still paints its whole subtree** into a fully transparent layer. Skipping
  would be faster and would give the painter a shortcut `hit_test` does not share, so it
  was left honest; an animation passes through 0 for at most one frame.
- **No fixture covers an opacity group that also clips or scrolls.** The three new
  fixtures are static, unclipped and unscrolled — §9.3's standing blind spot, unchanged.
- **The fling is still its own mechanism.** `fling_tau` stays in `zkite_frametiming` and
  `tick_fling` is untouched. A fling *is* an animation and the two should merge when the
  driver subsumes it, but moving a shipped, calibrated, device-measured curve at the same
  time as introducing a new one would make any change in scroll feel unattributable.
- **One device, one app shape**, as §42.8, §43.8 and §44.7.

## 46. `present` is half the frame and a tenth of it is painting — the overdraw item, refuted

*Queue F part 2's own named prerequisite, run first. §45.8 left one thing to price
before building the driver — what an opacity layer costs on this GPU — and that price
lives inside `present`, which is the largest phase in the frame and which nothing has
ever decomposed. Splitting it answered §45.8 and **refuted Queue H's last remaining
item** on the way.*

**The headline: of `present`'s 6.47ms, `vello_hybrid::render` — the CPU-side strip
generation this document has called the whole of it since §39.5 — is 0.67ms.**
`CommandEncoder::finish` plus `Queue::submit` is **5.29ms, 45% of all frame work**, and
it does not move when the app draws five times as much or nothing at all. Painting 25
full-width opaque rects over the same pixels costs **0.000ms** of `present` and 0.50ms
of `scene`. Overdraw is real, it is small, and it is not where §39.5 put it.

### 46.1 The claim that was never checked

§39.5 filed overdraw as one of four reasons the app is not native-smooth, and described
the phase it lives in:

> **The steady state has no headroom** … `present` ~7.5ms (`vello_hybrid::render` +
> submit — **CPU-side strip generation** for a 1080x2223 surface…)

§42.8 repeats it (*"`present` at ~7.2ms is still the largest phase and is still CPU-side
strip generation for the whole surface every frame"*), §44.7 repeats it again, and
NEXT_SESSION.md promotes it to the top of the queue: *"the last untouched §39.5 item, and
now the only large one left."*

**That is a claim about composition, and it is the claim the whole item is priced off.**
`present` is one `Instant::now()` spanning three unrelated calls:

```rust
let present_started = Instant::now();
hybrid_renderer.render(&scene, ...)     // ours: strip generation + GPU encode
gpu.queue.submit([encoder.finish()]);   // wgpu's, then the driver's
frame.present();                        // the swapchain's
phases.present += present_started.elapsed();
```

This is exactly the shape §43 found `sync` in — one number covering four kinds of work,
modelled by §42.5 as if it were one, 44% of it a defect. This document's own standing
rule says so: *"a phase in `ZKITEPERF` that covers more than one kind of work will
eventually be modelled as if it were one."* `present` is that phase, it is the biggest
one in the frame, and occlusion culling can only ever remove work from **one** of its
three components.

§39 is the near-miss worth naming. It split `acquire` (`get_current_texture`) out of
`present` and refuted the blocking hypothesis the field's own doc comment stated — but
that is the **dequeue** half of the swapchain. The enqueue half, and everything between
it and `render`, stayed lumped, and §39's conclusion (*"`present` really is ~7.5ms of
work"*) has been read ever since as *"`present` really is ~7.5ms of **strip generation**"*,
which is a different sentence and was never measured.

### 46.2 The instrument, and the mistake it caught in itself

`FramePhases` gains `present_render`, `present_finish`, `present_submit` and
`present_swap`, with `present` **kept as the total** — §43's own call when it split
`sync`, for the same reason: §39, §42, §43 and §44 all quote this number and have to stay
comparable. `phases:` and `worst:` are therefore untouched, and the split is its own line:

```
  present: render 0.67  finish 1.66  submit 3.63  swap 0.51  (of 6.47 total, ms) | worst: …
```

**The first version of this instrument was three fields, not four, and it was wrong in
precisely the way this section is about.** `gpu.queue.submit([encoder.finish()])` is two
calls in one expression, so `present_submit` lumped wgpu's command-buffer resolve with
the driver's submit and reported **`submit 5.28ms`**. Splitting them showed it was
`finish 1.66` + `submit 3.63` — a 31/69 split that the lumped number could not express,
inside a number that was itself created by lumping. Recorded rather than quietly fixed,
because writing an instrument that repeats the defect it was built to find is worth one
paragraph.

The four fields sum to `present` exactly on every window (0.70 + 5.28 + 0.52 = 6.50, and
so on), so nothing is unattributed.

**What the fields do and do not measure.** GPU work is asynchronous, so all four time CPU
occupancy inside a call, not GPU execution; a frame that overruns the GPU can pay for it
at the *next* frame's `acquire`. What they do settle is which call this thread is sitting
in, which is what *"is `present` ours to optimise"* reduces to.

### 46.3 The measurement: four arms, one variable, identical Rust

Moto G54 over USB, `cmp_list_1k.py`'s shape, 14 sustained swipes, GPU path, five
sixty-frame windows per arm. Gated per §40.6 and §42.7 — `frame: 1080x2223 via gpu`
asserted before *and* after, `mCurrentFocus` and pid identical at both ends, 252–253
`onTouchEvent`s, **0** `cpu-fallback` frames, 0 error markers. **No run was voided.**

**All four arms are app-only changes.** The source digest is `1ff6d6ad4026` for every one
of them, so the Rust is byte-identical and the only variable is the app — §42.4's method,
used there to show only `overscan` moved.

| arm | what differs | opaque rects/frame | nodes |
|---|---|---|---|
| **L** | `cmp_list_1k_overdraw0.py` — no row backgrounds at all | 0 | 152 |
| **A** | `cmp_list_1k.py`, unchanged — the baseline every section since §40 uses | ~25 | 152 |
| **H** | `cmp_list_1k_overdraw4.py` — five stacked opaque backgrounds per row | ~125 | 440 |
| **O** | `cmp_list_1k_opacity.py` — arm A under one full-screen `opacity-75` group | ~25 | 153 |

**L and A are the clean pair**: identical node count, identical glyph runs, identical
layout, identical scroll mechanics, differing by exactly 25 full-width opaque rects —
about 1.6 screens of coverage per frame. H is the heavy end and confounds rect count with
node count deliberately, to check the direction over a wide range. O is §45.8's question.

| arm | `scene` | `render` | `finish` | `submit` | `swap` | **`present`** | interval |
|---|---|---|---|---|---|---|---|
| L | 2.176 | 0.672 | 1.684 | 3.786 | 0.524 | **6.672** | 17.454 |
| **A** | 2.676 | 0.672 | 1.660 | 3.626 | 0.512 | **6.470** | 17.730 |
| H | 4.696 | 1.098 | 1.704 | 3.514 | 0.848 | **7.162** | 18.096 |
| O | 2.738 | 0.868 | 2.270 | 3.984 | 1.228 | **8.352** | 17.978 |

Arm A's phases sum to **11.73ms** of work against §44.3's 12.44ms — a different session
and a ~6% offset, so this is continuous with §42–§44 rather than a fresh baseline, and
none of what follows depends on the offset. Run-to-run reproducibility was measured
rather than assumed: arm A was run twice on the earlier build and returned `present`
6.538 and 6.540ms, so the noise floor on these columns is **±0.06ms**.

### 46.4 The finding: overdraw costs nothing in `present`

Deltas against arm A:

| | `scene` | `render` | `finish` | `submit` | `swap` | **`present`** |
|---|---|---|---|---|---|---|
| **L** — remove all 25 opaque rects | **−0.500** | **+0.000** | +0.024 | +0.160 | +0.012 | **+0.202** |
| **H** — 5x the rects, 2.9x the nodes | **+2.020** | +0.426 | +0.044 | −0.112 | +0.336 | **+0.692** |

**On the clean pair, `render` moves by 0.000ms to three decimal places.** Removing 1.6
screens' worth of opaque coverage from every frame changed vello's strip generation by
nothing measurable, and changed `present` *as a whole* by −0.202ms — i.e. removing the
overdraw made the phase nominally **worse**, which is how noise reads.

**The negative control is in the same table, and it is what makes a flat `render` a
measurement rather than a blind instrument.** The identical arms move `scene` by −0.500
and +2.020ms — eight to thirty times the ±0.06 noise floor, cleanly monotone in rect
count. So the apparatus can see content scaling perfectly well. It sees it in our own
scene-building walk, and it sees none of it in `present`.

**What overdraw actually costs, quantified:** ~0.02ms per full-width opaque rect, all of
it in `scene`, none of it in `present`. Occlusion culling on this workload would save
about **0.5ms/frame** — 4% of the frame — and would leave the 60% phase untouched. That
is a real number and a small one, and it is a different number, in a different phase,
from the one §39.5 named.

### 46.5 45% of the frame is `finish` + `submit`, and nothing the app draws moves it

| | L | A | H | range |
|---|---|---|---|---|
| `finish` | 1.684 | 1.660 | 1.704 | **0.044ms** |
| `submit` | 3.786 | 3.626 | 3.514 | **0.272ms**, and it goes *down* as content goes up |

Across 0 → 125 opaque rects and 152 → 440 nodes, `finish` + `submit` is **5.29 / 5.29 /
5.22ms**. It is a fixed per-submitted-frame cost, it is **45% of arm A's 11.73ms of total
frame work**, and it is not work ZKite does — it is wgpu resolving a command buffer and
the PowerVR driver accepting it.

`render`, the only part of `present` that is arguably ours, is **0.67ms — 5.7% of the
frame.**

**Whether the 5.29ms is work or waiting is not established here, and the obvious
hypothesis is the one the data argues against. (unverified)** GPU back-pressure would
predict `submit` *rising* when the GPU is handed 5x the fill work; arm H hands it exactly
that and `submit` falls by 0.112ms. So this does not read as the enqueue-side blocking
§39 never measured. Fixed CPU overhead per submission — in wgpu's resource tracking, in
the driver, or both — is the leading candidate and separating them needs a profiler this
session did not have on the device, not another arm.

### 46.6 §45.8 answered: one full-screen opacity layer costs 1.88ms/frame

Arm O is arm A with the entire scrolling subtree — header, `List`, all 25 materialised
rows, the whole 1080x2223 surface — inside one `opacity-75` group, so the painter opens
one offscreen target spanning the whole node list on **every frame** of a drag. That is
the worst case §45.8 names, and the shape a `Sheet`/`Dialog`/`Toast` fading a full-screen
scrim plus panel would produce.

| | delta vs A | |
|---|---|---|
| `render` | +0.196 | +29% |
| `finish` | +0.610 | +37% |
| `submit` | +0.358 | +10% |
| `swap` | +0.716 | +140% |
| **`present`** | **+1.882** | **+29%** |
| `scene` | +0.062 | unchanged, as it must be — the walk is identical |

**One full-screen opacity group costs ~1.9ms/frame**, 16% of the frame's total work, and
it is the one thing in this section that moves `submit` at all. `swap` more than doubles
and its worst case reaches 9–11ms, which is the enqueue-side blocking showing up exactly
where a real extra render target would put it.

`scene` being flat (+0.062, inside noise) is the control: the tree, the walk and the
`PositionedNode` list are unchanged, so the entire cost is the layer.

**What this means for F2**, stated as a budget rather than a verdict:

- A **transition** — 200–300ms, ~15 frames — costs ~28ms of extra work spread over the
  fade. That is affordable and the driver can be built on it.
- A **persistent** full-screen group costs 1.9ms/frame forever, on a frame with ~5ms of
  slack. That is a real constraint on where `opacity-*` may sit in a `zkite_py` API, and
  it is an argument for `translate-*` as the default channel for anything long-lived —
  a translate reuses the offset channel `positioned` already had and adds no layer.
- The cost scales with the **layer's area**, not with the animated node, which is the
  same structural point §45.2 made about `view()`.

### 46.7 What this does to Queue H, and to §45.2's arithmetic

**Queue H's last large item is not large.** "Overdraw is unmeasured — and after §43 and
§44 it is the only large item left" is measured now, and it is ~0.5ms/frame in `scene`.
The three sessions of Queue H work (§40, §43, §44) plus this one have taken it from four
open items to none worth the machinery, and the phase they were all aimed at turns out to
be 45% fixed driver cost and 5.7% painting.

**§45.2's arithmetic is unchanged in its conclusion and its premise is now sharper.** It
priced the animation API off "an ordinary frame is ~12.14ms against a 16.67ms budget" and
ruled out a Python controller. That still holds — but 5.29ms of that frame is a fixed
cost no design choice can remove, which makes the remaining headroom *less* elastic than
the figure suggests, not more. Nothing here weakens §45.2; it strengthens the constraint.

**The honest consequence is that the frame's largest cost is not ours.** Every phase this
document can shrink — `build`, `layout`, `sync`, `scene`, `render` — sums to ~5.4ms of the
11.73ms. That reframes what "make it smooth" can mean on this device and is the thing to
carry into any future Queue H session.

### 46.8 Verification

Host, all green and at the counts NEXT_SESSION.md records: `zkite_py` **182**, `devshell`
**8**, `zkite_renderer` **67**, `zkite_anim` **47**, `zkite_frametiming` **11**, cli **54**,
bundler **27**, `zkite_pybridge` **21** (`--test-threads=1`, §29.5), twcore **77**.

**No new host test, and that is a real gap rather than a scoping call** — see §46.10.
This section is an instrument and four measurements; `gpu_harness` does not build for the
host (§15.5), which is why §35, §42 and §45 all put testable logic in their own crates,
and there is no arithmetic here to move anywhere.

**The conformance harness and the webruntime e2e suite were not re-run**, on §44.6's
precedent and for the same reason: the change is confined to
`gpu_harness/src/render_thread.rs`, which is native-only and which neither harness
exercises. The one functional edit is `gpu.queue.submit([encoder.finish()])` becoming
`let buffer = encoder.finish(); gpu.queue.submit([buffer])` — the same two calls in the
same order — plus three `Instant::now()`s. Nothing touches `zkite_py`, `runtime.py`,
`twcore` or either runtime, so §43's rule (anything touching shared Python re-runs e2e)
does not fire.

**Artifact verification before every install** (§15.6.1), with a control that
discriminates: the app inside `assets/app.zip` read as the intended arm on all four; the
packed `.so` contained the new `present: render` format string (**1**) and not a
plausible near-miss (`present: rasterize`, **0**); and the packed `.so` hashed identically
to the staged one. The first arm-L build **silently did nothing** because the shell's
working directory had drifted to `phase_1/zkite_py` — caught by the digest line being
absent, and a seventh way "the build succeeded" has been wrong (§15.6.1).

Device: seven runs, none voided. `zkite doctor` green after rebuilding **both** ABIs.

#### 46.8.1 The device regression sweep — every fixture, because `paint_gpu` is shared

The measurement above exercises `cmp_list_1k`'s four variants. **`paint_gpu` is the
function every native ZKite app goes through**, so that is not sufficient evidence that an
edit to it is safe, and plan1.md §14.5's "no CI for any phase-3 crate" means nothing else
was going to check. So all **27** `demo_app` fixtures were built, installed fresh
(`uninstall` → `install` → `pm clear`) and launched on the Moto G54, one at a time.

**27 built, 27 launched, 27 rendered. No failures.** Per fixture: the artifact was checked
to contain *that* app before installing, then pid alive, `mCurrentFocus` on
`ZKiteActivity`, `frame: 1080x2223 via gpu`, a `ZKITEPERF first-frame nodes=N` line, **0**
`cpu-fallback` frames, **0** `FATAL EXCEPTION`/`E AndroidRuntime`/`PANICKED`, and a
distinct-colour count off the screenshot so a blank screen cannot pass (§35's lesson).
Node counts spanned **6** (`cmp_counter`) to **50,006** (`list_plain`, the un-virtualised
control), and colour counts 23 to 538.

**A sweep that passes everything is indistinguishable from a sweep that checks nothing**
(§27.1), so both of its checks were shown to fail:

| control | result |
|---|---|
| the blank-screen detector, on a synthetic all-black 1080x2223 PNG | reads **1** — would FAIL |
| the same detector on a four-quadrant PNG, and on a real fixture | **4** and **538** — PASS |
| the artifact check, harness APK holding `typography` vs `typography.py` | **match** — PASS |
| the same APK vs `gradients.py` | **mismatch** — would FAIL |

**A process failure worth recording, because it is §37's own trap a second time.** The
first artifact control built with `ZKITE_DEVSHELL=1` and `ZKITE_APP_MAIN` together (§45.5's
trap) and reported a mismatch — apparently confirming the check. It was confirming
nothing: the devshell branch writes `zkite-native-devshell-arm64-v8a.apk`, and the control
had unzipped the *stale* `zkite-gpu-harness-arm64-v8a.apk` left by the previous fixture, so
it compared two unrelated artifacts. **§45.5 is exactly right as written** — the devshell
APK contains `assets/zkite-devmode` and no `app.zip` at all, confirmed by `unzip -l` on
both files. A control that passes for the wrong reason is §37.5's `is_stale()` case, and
this is the second time this document has recorded one.

**The shipped path, both backends.** `zkite build apk` stages differently from the harness
and §34.8 is the reason to check it separately. `zkite new` + a `main.py` exercising the
paint vocabulary this section touches — an `opacity-50` group, a `translate-x-4`, a
linear gradient, a `border`+`ring`+`shadow` box, an `Icon`, and a 500-row virtualised
`List` — built and launched on both: native `nodes=217`, `frame: 1080x2223 via gpu`,
0 fatal; WebView launched clean too. **Screenshots agree**: the faded swatch, the
translated swatch overlapping its neighbour rather than displacing it, the gradient, the
ring-outside-border box and the icon all render the same on the two backends.

**One run was voided and it was the focus gate that caught it** — the phone had reached
its lock screen mid-session, which this device reports as `mCurrentFocus=NotificationShade`,
so both arms had measured the lock screen (the WebView arm read **5057** distinct colours,
which is a wallpaper, not an app). Re-run after unlocking. §40.6's gate list earns its
place again, and **`NotificationShade` is worth adding to it as this device's lock-screen
signature** — it does not look like a lock screen in the log.

Also re-run and unchanged: **`phase_0/run_all.sh --device`** — all six checks PASS,
including spike 1 (CPython boots inside the app process on arm64) and spike 2 (a
`pack`-built APK installs, launches and runs our dex + `.so`).

### 46.9 What this deliberately does not do

- **It does not build any of Queue F part 2.** No wire format, no `Timeline` in
  `render_thread`, no `zkite_py` surface. This is F2's stated prerequisite (§45.8) run
  first, and it turned into a finding large enough to own a section; building the driver's
  three layers on top of it in the same session would have given them none of the device
  verification they need.
- **It does not implement occlusion culling or damage tracking.** It prices them: ~0.5ms
  of `scene`, 0.00ms of `present`. Whether 4% of a frame is worth the machinery is a
  ranking call, and it is now a ranking call with a number.
- **It does not chase the 5.29ms.** Naming `finish`+`submit` as 45% of the frame is the
  result; reducing it means either fewer submissions per frame or a different graphics
  path, and both are their own sessions.
- **It does not change any rendering behaviour**, by construction — the section is four
  `Duration` fields, three `Instant`s and one expression split into two statements.

### 46.10 Known gaps

- **No host test, and this instrument is code (§42's own lesson).** §42.1 caught two
  defects in its own instrument by running it — a 30ms interval bucketed as on-time, and
  idle expressed against the wrong denominator — and §42.6 moved that arithmetic into
  `zkite_frametiming` precisely so it could carry tests. This split has no arithmetic to
  extract (it is four `+=`s), but it also has nothing pinning that the four fields
  continue to sum to `present`; today that is checked by reading the numbers off the
  device. A `debug_assert` on the sum would be the cheap version and is not written.
- **Work versus waiting is not separated for `finish`+`submit`** (§46.5). The GPU
  back-pressure hypothesis is argued against by arm H rather than excluded, and the
  alternative — fixed CPU overhead in wgpu or the driver — is **(unverified)**. This is
  the same open thread as §44.7's positional-cost cause and §43.8's interpreter anomaly:
  three findings now whose mechanism is measured and whose cause is not.
- **`swap`'s worst case reaches 9–11ms in arm O** and nothing explains it. It is the only
  column in this section with a heavy tail, it appears only when an offscreen target
  exists, and a tail that size on a 16.67ms budget deserves its own look before F2 ships
  a full-screen fade.
- **One opacity layer, one size, one alpha.** Arm O measures a single full-screen group at
  `opacity-75`. Nothing measures two nested groups, a small group, or whether the cost
  tracks area linearly — all three matter for a `zkite_py` API that lets an author put
  `opacity-*` anywhere, and §45.8's entry is answered for the worst case only.
- **`render` is flat L↔A and rises in H**, where node count nearly tripled alongside rect
  count. So `render`'s insensitivity is established against *overdraw* and not against
  *node count*, and the two are only separated by the L↔A pair.
- **Overdraw is priced on one list shape.** 56dp rows, 152 nodes, 25 materialised rows, a
  1080x2223 surface. §42.8's caveat applies unchanged: a taller row or a heavier painter
  moves the per-rect constant.
- **One device, one ABI, one density, the GPU path**, as §42.8, §43.8, §44.7 and §45.8.
  `finish`+`submit` being 45% of the frame is a statement about a PowerVR BXM-8-256 and
  this wgpu version, and it is exactly the kind of number that could be completely
  different on an Adreno.
- **The §46.8.1 sweep checks that a fixture *runs*, not that it renders *correctly*.** Its
  strongest visual assertion is "more than one distinct colour on screen", so it would
  pass a fixture that renders the wrong colours, the wrong layout, or a traceback (§19.3).
  It is a smoke test, and calling it a conformance gate would be the §29.8 mistake — a
  harness reporting 0.00% on a divergence it is constitutionally unable to see. The real
  per-fixture visual gate is still §9.3's open item.
- **The sweep is a scratch script, not a checked-in gate.** It lives in this session's
  scratch directory and nothing runs it on a schedule, so plan1.md §14.5's "no CI for any
  phase-3 crate" is untouched. Promoting it would want the overlay-detection fix above
  first, or its first CI run would go green on a broken app.
- **The WebView backend is untouched and unmeasured**, as in §44. §12/§13's head-to-head
  now predates §39 through §46.

## 47. The animation driver, part 2 — wired up, and the layer that had to move

*Queue F part 2. §45 built `zkite_anim` and made `opacity-*`/`translate-*` paint;
nothing was connected to either, so no author could write an animation. This section
builds the three layers that were missing — the wire format, the driver in
`render_thread`, and the `zkite_py` surface — and ships the item NEXT_SESSION.md called
1d on the way, because it is a precondition for this section's own evidence.*

**What an author can now write:**

```python
Row("h-[56px] w-[200px] bg-emerald-500",
    Text("text-white", "SLIDE"),
    anim=Anim.enter(opacity=(0, 1), translate_y=(16, 0), ms=220, ease="out"),
    on_animend=lambda e: s.update(shown=True),
)
```

That declaration crosses to the renderer as one `anim` attribute, is started once by a
`Timeline` in the render thread, is sampled **with no Python in the frame**, and
dispatches exactly one completion back down the event path a tap already uses.

### 47.1 Item 1d first: a broken app was invisible to logcat

§46.8.1 found that an app whose `view()` raises writes **nothing** to logcat on any
tag — the traceback goes only to the on-screen overlay — so `error_repro.py`, broken by
construction, passed that sweep's `fatal=0` gate looking healthy. This was done first
rather than last because **this section's own evidence is a device run**, and a gate
that cannot tell a working app from one showing a traceback cannot verify an animation
driver either.

The plumbing already existed and nothing was using it: `runtime.py` tees
`sys.stdout`/`sys.stderr` into a buffer that `zkite_pybridge::embed::drain_output`
flushes to `__android_log_write`, and every path that can produce a tree drains —
`setup`, `dispatch`, `back`, `reload`, `scroll` when a tree comes back, and
`FrameReady`'s explicit `drain_log`. `App` simply never wrote the traceback anywhere but
the overlay. `zkite.errors.log_error`/`log_recovered` now do, from `App._note_error`/
`_note_rendered`, which means the fix lands on **both backends at once** — launched by
anything other than `zkite dev`, the WebView backend was equally silent.

**The guard is the design, not a detail.** A broken `view()` raises on *every* frame, so
an unguarded write is sixty tracebacks a second on an app that is already unusable.
Comparing against `last_error` — which `App` already tracked for `zkite dev`'s socket —
makes it one burst per *distinct* failure, and a recovery logs one line so a gate can
see an app come back rather than only see it break (§19.3's third bullet named that
round trip as untested).

Six host tests, and three negative controls that each fail exactly the right ones:
dropping the change-guard makes 60 frames log 60 tracebacks (2 tests fail), removing the
log entirely fails 4, and removing the recovery line fails 1.

### 47.2 The wire format (layer 1)

One `anim` attribute per node, a list of per-channel declarations, parsed by
`zkite_pybridge::anim` — a crate that is host-testable, which is why the parse lives
there and the driver does not.

```json
[{"ch":"opacity","to":1,"from":0,"ms":220,"ease":"out"},
 {"ch":"translate_y","to":0,"from":16,"spring":[180,26],"on":"change"}]
```

**Three channels, not the five `twcore` parses.** `rotate-*` and `scale-*` compile into
the same `Transform` and are painted by nothing on native (§45.7), so animating one
would run, complete, fire its callback and move no pixels — §15.1's silent divergence
reproduced *inside* the feature built to remove one. They are rejected by name, with the
reason, on both sides.

**What is deliberately strict, and why that is the opposite call from Tailwind classes.**
Unknown keys, unknown channels and non-finite numbers are errors, not silent drops. An
unknown utility class is dropped because Tailwind has thousands of them and a typo is
cheap; an author who writes `rotate=...` has asked for one specific thing, and silence
is the failure mode this whole document is about. The counterweight is that a malformed
declaration must not take the app down with it — so the error is collected per node and
**every other node, including a valid animation in the same tree, still builds**, with a
test that asserts exactly that.

**A finite JSON number can still arrive as a non-finite `f32`, and it is checked.**
Probed rather than assumed: `serde_json` deserialising `1e39` into an `f32` yields `inf`
with no error. A NaN or infinite `translate_y` propagates into `positioned()`'s
arithmetic and from there into `hit_test`, giving a node painted nowhere and tappable
nowhere with nothing logged — so every numeric field is range-checked at the parse.

**An unstated `from` means "wherever the class string puts this node", not zero.** That
value depends on `twcore`, on inheritance, and — for `1em`-relative sizes — on ancestry
Python cannot see (§18.4), so only the renderer can supply it. Defaulting it to `0` here
would make every unstated fade start from fully transparent.

### 47.3 The driver (layer 2), and the ordering §45.3 got wrong

`Animator` in `render_thread.rs`: a `zkite_anim::Timeline` keyed by
`(zkite_py node id, Channel)`, sampled once per frame against one timestamp.

**§45.3's layer diagram says "overrides applied in `zkite_pybridge::build`". They are
not, and cannot be.** Applying inside the build is circular: an animation whose `from`
is unstated starts from the node's *resolved class value*, and that value does not exist
until the node has been built. The driver therefore runs **after `build` and before
`layout`** — which needs no extra tree walk, because `build` collects the declarations
on the pass it is already making, and is safe because neither channel affects layout.
The sketch was wrong in a way that only writing it revealed; it is corrected here rather
than worked around.

**The gate is on the declaration, not on running-ness, and this is the subtle one.**
`Interrupt::Ignore` declines to restart an animation that is still *running*, and
`zkite_anim` documents that a finished entry does not block a new run — right for a spec
that is sent, withdrawn and sent again. It is wrong for a **declaration**, which is what
§45.3 principle 3 makes these: the entrance spec is still on the node the frame after it
completes, so `Ignore` alone would restart it, forever. `should_start` gates on what was
last *started* for that key; `Ignore` stays underneath as a second line of defence.

**Two decisions were moved out of `gpu_harness` to be testable at all**, on §35/§42/§45's
standing precedent that this crate does not build for the host (§15.5):

- `zkite_frametiming::next_wake` — the loop's wait. The obvious implementation is
  `min(frame_due, anim_deadline)` and it **spins the render thread at 100% CPU for the
  length of every animation**, because a running timeline reports `ZERO` and
  `min(anything, 0)` is `0`. `Timeline::next_deadline`'s own doc says why it reports it
  that way: pacing is the render loop's business. So `ZERO` *selects* the frame pacing
  rather than being combined with it, and a purely delayed timeline sleeps the real
  delay — a 300ms toast costs one wakeup, not eighteen. The `min` version fails three
  tests including the busy-wait one, which otherwise only shows up as a warm phone.
- `zkite_pybridge::anim::should_start` — the restart gate above. Gating on running-ness
  instead fails the two tests that exist for it.

**Completions are collected during the frame and dispatched after it**, in the loop
rather than in `redraw`, because a Python handler may `@store.update()` and re-enter the
frame path — which every other dispatch site here already relies on not happening. Only
nodes that registered `on_animend` are dispatched to at all: §45.2's constraint is that
Python hears back *once*, and this is where once can still mean never.

**New instrument, `anim:`** — counts, not time. Its per-frame cost sits inside `build`
and is bounded by the number of *animating* nodes rather than by the tree, so a seventh
time column would report noise; §46's lesson is that a `ZKITEPERF` column covering more
than one kind of work eventually gets modelled as one. What no existing line can see is
whether the driver is *working*: `started` and `done` must balance, and **`driven-frames`
is the number of frames this thread produced because an animation asked for them** —
the first workload this renderer has had that is not input-paced (§45's free instrument).

### 47.4 The author surface (layer 3)

`Anim.enter(...)` / `Anim.change(...)` / `Anim.exit_(...)`, with channels given as a
target (`opacity=1`) or a pair (`opacity=(0, 1)`). The owner's decision (§45.3 principle
2) was explicit channels and triggers rather than Tailwind `transition-*`, so that both
backends share one set of semantics instead of inheriting CSS's interruption behaviour,
property-change triggers and `transitionend` timing.

**`exit` is a declared state, not an event the renderer detects**, and that is forced by
the architecture rather than chosen: the tree is authoritative, so a node that has left
it is gone. §45.2's answer, priced there — `view()` keeps rendering the node with an
`exit` spec on it, the driver runs the animation and dispatches one completion, and
Python drops it then. One Python call per animation, not per frame.

**Merged at one call site, which is the point.** Eleven widgets override `_tree_attrs`
and build their own dict; folding `anim` in at each of them is what a first draft does,
and it works for exactly the ones someone remembered. `Widget.to_tree` merges it once,
and the test that covers `Switch`/`Slider`/`Button` is what that choice is for.

**`animend`'s payload is typed at the widget**, on the base class rather than per widget.
§26.2's untyped string payload has now bitten `Switch`, `Slider` and `Select` in turn;
typing this one at birth is the cheap version of not making it a fourth. That also
required `Switch` and `Slider`'s own `_payload_parser` overrides to *delegate* rather
than return `None` for everything but `change` — otherwise a `Switch` with an `anim=`
would hand its handler raw JSON text while every other widget handed it an `AnimEnd`.

### 47.5 The divergence this ships with, made loud

**The driver is native-only.** The WebView port is layer 4 (§45.3) and is not built, so
a declaration animates on native and renders statically on the WebView, and no
`on_animend` fires there. That is a §15.1 divergence, and this document's whole record is
that the dangerous ones are the *quiet* ones — so `zkite_py` prints one line to stderr,
once per process, naming the section and telling the author to run `--backend native`.
Item 1d's channel is what carries it to logcat.

Nothing shipped depends on the broken half: `Sheet`/`Dialog`/`Toast`, which would need
exit completion to close, are Queue E and are not built. An app that declares no
animation emits a byte-identical tree and a byte-empty stderr, which is the negative
control for both.

### 47.6 Verification

Host, all green:

| suite | before | after |
|---|---|---|
| `zkite_py` | 182 | **205** |
| `zkite_pybridge` | 21 | **44** |
| `zkite_renderer` | 67 | **72** |
| `zkite_frametiming` | 11 | **16** |
| `zkite_anim` | 47 | 47 (untouched — §45 built it complete) |
| `devshell` / cli / bundler / twcore | 8 / 54 / 27 / 77 | unchanged |

`zkite doctor` clean after rebuilding **both** ABIs.

**The test that matters most is the one neither language can write alone.**
`a_real_zkite_py_anim_declaration_parses_into_the_track_it_describes` runs real `zkite_py`
through the real interpreter, takes the JSON it actually emits, and asserts on what the
real Rust parser makes of it — nothing hand-typed. Every test either side of that seam
can pass while the two disagree about a key name or a unit, which is §26.2's shape
exactly. Its controls: renaming `ch` to `channel` on the Python side fails it with the
parser naming the key; sending seconds where milliseconds are read fails it with
`220µs != 220ms`. A third control — stating the default trigger instead of omitting it —
correctly does **not** discriminate, because the two are semantically identical and
omitting is a byte-saving optimisation; recorded on §42's precedent for a control that
passes on purpose.

**§34.6's store-registry trap fired for the second time in this document**, and the file's
own comment is what named it: the new end-to-end test declared `@store class S`, which
collides with `a_touch_on_a_synthesized_thumb_resolves_to_the_control_itself`'s `S`
because the registry is process-global keyed by qualified name and every test app loads
as module `main`. Deterministic, not the §29.5 flake. Fixed by a distinct name, which is
what §34.6 prescribes — but two occurrences suggest the real fix is resetting the
registry between tests rather than relying on everyone reading that comment.

### 47.7 The device run — on the emulator, and **not** on the phone

**Read this first: every measurement below is from the x86_64 emulator (`zkite_phase0`,
Android 14, SwiftShader, density 2.75), because no phone was attached for the whole of
this session.** §29.8 is the standing proof that the emulator is not a substitute for
the Moto G54, and nothing here is offered as one. What it does establish is the regime
this section most needed and could not get from the host: **the driver runs on Android,
in a real app process, on the real render thread, and produces real frames.** §34 and
§42 are the precedent for using it that way — to reach a regime, never to replace the
phone.

Gated per §40.6/§42.7/§46.8.1 at both ends: `mCurrentFocus` on `ZKiteActivity`, the app's
own `frame: 1080x2274 via gpu` line, one pid throughout, **0** `cpu-fallback` frames, no
`FATAL`/`PANICKED`. Artifact verified before install (§15.6.1) with a control that
discriminates — `app.zip` matched `anim_driver.py` and **not** `gradients.py`, and the
packed `.so` contained `ZKITEANIM bad declaration` (1) and not a plausible near-miss (0).

**Both entrance animations are exact at both ends and monotonic in between.** They are
6000ms and `linear` on purpose: a realistic 200–300ms entrance is over before
`screencap` has written a PNG, which makes "it animated" and "it snapped to the end on
frame one" indistinguishable.

| | t2 | t3 | settled |
|---|---|---|---|
| SLIDE, `translate_x` 0 → 100dp, left edge | 264 | 416 | **539** |
| travel, against 100dp = **275px** at density 2.75 | 0 | 152 | **275, exact** |
| FADE, `opacity` 0.1 → 1.0, implied alpha | **0.100** | 0.594 | **1.000** |

Both endpoints land on the declared value exactly. And the two agree with *each other*:
at t3 the slide is 152/275 = 0.553 through its timeline and the fade reads 0.594 —
the same point on the same 6000ms clock, measured through two entirely unrelated
mechanisms, a geometric offset and a colour composite.

**The CONTROL bar is byte-identical in every shot** (1650 bytes, compared as raw RGB
across all pairs), which is what makes the two rows above measurements rather than
"something on screen changed". The band reader was itself shown to fail first, on
synthetic PNGs: it catches a control bar one pixel wider, and one whose colour moved by
4/255 on a single channel.

**The exit round trip, caught mid-flight, is the one that proves the contract.** Two
channels on one node, `opacity` 1 → 0 and `translate_x` 0 → −160dp (−440px), over 4000ms:

| | e0 | e2 | e3 | e5 |
|---|---|---|---|---|
| left edge | 264 | 229 | 10 | — |
| travel | 0 | −35px | −254px | — |
| implied alpha | 1.000 | **0.921** | **0.423** | — |
| alpha predicted from the *other* channel's travel | 1.000 | **0.920** | **0.423** | — |
| row present | yes | yes | yes | **GONE** |

The two channels are sampled from one timeline and agree to three decimals. Then the row
**disappears** — which can only happen if the completion crossed back into Python,
`on_animend` saw `trigger == "exit"`, and the next `view()` omitted the node. That is
§45.2's contract end to end: the renderer cannot animate a node out of a tree it has
left, so Python holds it and drops it on one callback.

**The loop blocks when nothing is moving, and this is the strongest single result here**
— it is the failure `zkite_frametiming::next_wake`'s `min()` control introduces, and the
one that would otherwise only show up as a warm phone:

| | CPU | cumulative CPU time |
|---|---|---|
| during the 6s entrances | 84–93% | 3.52 → 4.89 → 6.45 → 7.72, climbing |
| after they finish | **0.0%** | **9.50 → 9.50 → 9.50**, frozen across 6 seconds |

Not merely low CPU — *no CPU time accumulated at all*. The thread is genuinely back on a
blocking `recv()`, and it self-paced frames before that with no input of any kind, which
is §45's "the first workload this renderer has that is not input-paced".

**A translated node is tapped where it is drawn**, verified with both signs:

| tap x | where the bar was drawn | result |
|---|---|---|
| 500 | 264..813 | hits — springs to 539..1079 |
| 500 | 539..1079 | **misses** |
| 800 | 539..1079 | hits — springs back to 264..813 |

The middle row arrived as an accident (a tap aimed at the old position) and is kept,
because it is the negative control the other two need: the same coordinate hits when the
bar is there and misses when it is not.

**What this run does not establish, and it is a real list.**

- **Nothing was run on the Moto G54.** Every timing here is SwiftShader's — the log is
  full of `redraw SLOW: 281–500ms`, i.e. §42's emulator regime, ~20x the phone's frame
  cost. No claim about smoothness, jank, or per-frame cost survives from this run.
- **The `anim:` ZKITEPERF line was never printed, so the instrument is unverified.** A
  summary needs 60 redraws; a 6000ms animation at ~300ms/frame produces about 20. That is
  exactly the trap NEXT_SESSION.md documents ("the build you most want numbers for is the
  one that prints nothing"), and it means `started`/`done`/`driven-frames` have been
  compiled and never read. The completion *behaviour* is confirmed by the exit row
  vanishing; the counter that would show a completion firing sixty times a second instead
  of once is not.
- **§46.6's 1.88ms/frame opacity-layer cost is untested against a real driver.** The
  fixture deliberately keeps its opacity group to a 200x56dp box; nothing here measured
  what a full-screen fade costs while actually animating.
- **`uiautomator dump` returned empty text** for every node, so the `done=N` counter was
  never read directly. §8.7's guard and NEXT_SESSION.md's own warning both apply: trust
  logcat and the screenshot over it.
- One app shape, one ABI, one density, one backend.

### 47.8 What this deliberately does not do

- **The WebView backend is untouched** — layer 4 of §45.3, and the reason §47.5 exists.
  `zkite_anim` still has no JS counterpart, so F3's numeric parity test does not exist.
- **`Sheet`/`Dialog`/`Toast` are not built.** They are one composition each over §33's
  `Overlay` plus a transition, and they are Queue E. What changed is that the transition
  they were waiting for now exists.
- **`rotate-*`/`scale-*` are still parsed and dropped**, and are now rejected *loudly* by
  the animation surface rather than silently accepted. They need a real affine and an
  invertible `hit_test` (§45.7, Queue A).
- **The fling is still its own mechanism.** §45.8's reasoning is unchanged: a fling is an
  animation, the two should merge, and moving a shipped, calibrated, device-measured
  curve at the same time as introducing a new driver would make any change in scroll feel
  unattributable. `tick_animation_frame` gives them one entry point without merging them.
- **No `transition-*`/`animate-*` parsing in `twcore`**, per §45.3 principle 2.
- **§46.10's `swap` tail was not investigated.** It names itself as worth a look "before
  F2 ships a full-screen fade", and this section declines it deliberately rather than by
  omission: the same run-the-prerequisite-first move already consumed §46 and left F2
  unbuilt, and §46.6's own conclusion supplies the mitigation in the design instead —
  `translate-*` opens no layer, so it is the documented default for anything long-lived
  and `opacity` carries its measured price. The tail remains open.

### 47.9 Known gaps

- **Nothing ran on the Moto G54.** The whole of §47.7 is emulator evidence. Every
  behavioural claim there is device-independent arithmetic (endpoints, monotonicity,
  channel agreement, the tap round trip, CPU idling); every *timing* claim is absent
  because SwiftShader cannot supply one. **This is the first thing the next session
  should do**, and it is cheap: the fixture, the gates and the band reader all exist.
- **The `anim:` instrument has never printed.** See §47.7. It is code, and §42's own
  lesson is that an instrument is code and gets the same treatment — this one has had
  neither a unit test nor a reading.
- **`json.dumps` emits bare `NaN`/`Infinity` and `serde_json` rejects them, killing the
  whole tree.** Found while checking what a non-finite number does to the wire format,
  and **pre-existing, unrelated to animations, and affecting both backends**: probed
  directly, `json.dumps({'x': float('nan')})` produces `{"x": NaN}` and `serde_json`
  answers `expected value at line 1 column 52` for any tree containing it — so one NaN
  anywhere (a `Slider` value, any attr) makes `RawNode::from_json` fail and the app
  renders nothing at all. Not fixed here: the one-line change (`allow_nan=False`, which
  raises a clear Python error the overlay then shows) alters app-wide failure behaviour
  on the shared serialisation path and deserves its own before/after. Filed rather than
  folded in.
- **Two channels, and the third and fourth are not "more of the same".** `rotate`/`scale`
  need an invertible hit test before they can be animated at all.
- **A `change` trigger compares targets by bits.** A target that is recomputed each frame
  by floating-point arithmetic that does not reproduce exactly will retarget every frame
  and never converge. Nothing in `zkite_py` produces that shape today, and a tolerance
  would need a per-channel epsilon (§45.8's rest-threshold gap, in a second place).
- **An animation's cost is inside `build`'s column**, so `phases:` cannot separate it
  from the tree walk. Bounded by animating nodes rather than tree size, and §46's lesson
  argued against a seventh column — but it does mean "what does the driver cost per
  frame" has no direct answer.
- **§34.6's store-registry collision fired for the second time** (§47.6). Two occurrences
  say the real fix is resetting the registry between tests rather than a comment asking
  everyone to pick unique names.
- **No conformance fixture covers an animation**, and cannot: §9's harness renders one
  static state. The channels themselves are covered (§45.1 added `opacity`,
  `opacity_group` and `translate`); the *driver* is not, and a static harness is
  structurally unable to see it — the same blind spot §26.6 records for interaction.
