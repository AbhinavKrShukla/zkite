# Phase 1 — DX skeleton, WebView backend

This is the implementation plan for **plan1.md §10, Phase 1**:

> `zkite new` · `zkite dev` · hot reload with state preservation · 12 widgets · Tailwind
> resolver covering the core utility set · on-device error overlay · Dev Shell APK.
> **Exit:** a stranger goes from `pip install` to a hot-reloading counter app on their
> phone in under 3 minutes, unassisted.

Phase 0 passed all three spikes (`../phase_0/README.md`): CPython embeds and runs on-device,
`google/pack` produces a signed installable APK with no JDK/SDK on the user's machine, and
`twcore` proves Tailwind semantics map cleanly onto Rust structs. None of that is wasted here —
§ below explains exactly which parts phase 1 reuses and which parts it deliberately does not.

---

## 1. The one architectural decision that shapes everything else

plan1.md §5: *"build for the custom GPU renderer, but ship the WebView backend first."* Phase 1
is entirely the WebView side of that sentence. Every decision below follows from taking "it's
real CSS" seriously instead of half-seriously:

**We do not use `twcore` in phase 1.** `twcore` (phase 0, spike 3) resolves a class string to a
`taffy::Style` for a renderer that owns its own layout and paint. A WebView already has a
layout engine and a paint engine — Blink's — and it is a better one than we will build before
phase 3. Reimplementing Tailwind-the-CSS-subset in twcore and then reimplementing
Tailwind-the-real-thing again for Blink would be building the same thing twice and shipping the
worse one first. So:

- **The Tailwind engine for phase 1 is real Tailwind**, running as JIT-in-browser via
  `@tailwindcss/browser` (Tailwind v4's zero-build-step distributable — confirmed on npm,
  `4.3.x`, see `webruntime/vendor/`). We vendor the JS file into the wheel at framework-release
  time — same pattern as phase 0 vendoring `classes.dex` — so there is no CDN dependency and no
  network access needed at runtime, only at `zkite` release time.
- This is *more* faithful to Tailwind than twcore is (twcore is deliberately a subset, per its
  own README), and it costs us zero engineering, because it already exists and is MIT-licensed.
- `twcore` stays exactly where phase 0 left it: untouched, reserved for phase 3's native
  renderer, where there is no CSS engine to defer to.
- Phase 1's "Tailwind resolver covering the core utility set" deliverable (§10's wording) is
  satisfied by: (a) wiring the real engine into the WebView shell, (b) widgets emitting their
  own *implied* structural classes (see §3 below — `Column` implies `flex flex-col` so the
  §8 example works without the user typing it), (c) generating `zkite.tailwind.json` for editor
  autocomplete (§4's "editor story"). If this reading is wrong and a Rust-side resolver is
  wanted even for the WebView path, that's a one-line redirect — twcore's `resolve()` already
  exists — but building it would be solving a problem the WebView doesn't have.

**Consequence for rendering:** Python owns the widget tree and renders it straight to an HTML
string (each widget knows its own tag and classes). The device-side JS runtime takes that string
and calls `morphdom(document.body, html)` — a small (~600 line), dependency-free, MIT-licensed
DOM differ that patches only what changed and **preserves focus, scroll position, and input
value on unchanged nodes for free**. That single library removes what would otherwise be a
bespoke vdom-diff project, and it's the mechanism that makes §6's "keyboard state and scroll
position intact" true without us writing a diff algorithm.

**Consequence for IME:** phase 0's known-gaps section flags IME as a High risk requiring
`GameTextInput` and a hand-built virtual semantics tree — *for the phase 3 custom renderer*,
where text fields are painted pixels with no `View` behind them. None of that applies here. A
WebView `<input>`/`<textarea>` is a real Android `View` under the hood; IME, autofill,
accessibility tree, and the copy/paste toolbar are Blink's problem, already solved. This is the
biggest practical argument for shipping the WebView backend first, beyond speed to market: it
retires the plan's own #1 risk for the entire span of phase 1, and defers paying for it until
phase 3 actually needs to.

---

## 2. Architecture

```
┌───────────────────────────────────────────────────────────────────────┐
│ main.py, pyproject.toml (user's app)                                  │
├───────────────────────────────────────────────────────────────────────┤
│ zkite (pure Python wheel) — phase_1/zkite_py/                           │
│  widgets.py   Widget tree → HTML string, 12 widgets, implied classes  │
│  store.py     @store, singleton registry, .update() → re-render       │
│  app.py       finds `view`, injects stores by annotation, render loop │
│  reload.py    re-exec changed modules, migrate/reset stores by shape  │
├───────────────────────────────────────────────────────────────────────┤
│ CPython C-API (embedded, no IPC) — reuses phase_0/spike1_cpython_embed│
├───────────────────────────────────────────────────────────────────────┤
│ ZKiteActivity : android.app.Activity (dex, prebuilt) — extends        │
│   phase_0/spike2's shim: WebView host · JS bridge · reload-socket    │
│  ┌─────────────────────────────────────────────────────────────────┐ │
│  │ WebView — phase_1/webruntime/                                   │ │
│  │  index.html · @tailwindcss/browser (vendored) · morphdom.js     │ │
│  │  zkite-runtime.js: morphdom(html) on render, delegated events    │ │
│  │  → window.ZKiteNative.dispatch(id, type, payload)                │ │
│  └─────────────────────────────────────────────────────────────────┘ │
└───────────────────────────────────────────────────────────────────────┘
```

Host side:

```
zkite CLI (Rust) — phase_1/cli/
├── zkite new     scaffold main.py + pyproject.toml + zkite.tailwind.json
└── zkite dev     watch → hash-diff manifest → push over adb-reverse socket → reload signal
```

---

## 3. Widget semantics: implied structural classes, and argument order

**Finding — plan1.md §8's own example contradicts its own stated rule.** The design
notes say *"First positional arg is always the class string; children follow as
`*args`"*, but the one code example writes `Text(f"{c.n}", "text-7xl ...")` — content
first, class string second — and `Button("Increment", "px-6 ...", on_tap=...)` the
same way. Every container widget in the same example (`Column("...", child, child)`)
follows the stated rule. Read against the explicit rule and its stated rationale ("no
keyword noise, no special cases to remember"), this looks like the one example wasn't
updated rather than the rule being wrong, so `zkite_py/src/zkite/widgets.py` implements
the *uniform* rule — class string first, on every widget, no exception for leaf/text
widgets:

```python
Text("text-7xl font-bold text-white tabular-nums", f"{c.n}")
Button("px-6 py-3 rounded-2xl bg-indigo-500 ...", "Increment", on_tap=...)
```

`zkite_py/tests/test_app_counter.py` builds the §8 counter app with this corrected
order. If the literal example order was actually intended, it's a small, isolated
change (swap the first two `_tag_attrs`-adjacent params in `Text`/`Button`) — flagging
it here rather than silently picking one, per how phase 0 handled its own judgment
calls.

plan1.md §8's whole demo app never writes `flex flex-col`:

```python
Column("flex-1 items-center justify-center gap-6 bg-slate-950", Text(...), Button(...))
```

For that to render correctly, `Column` itself must imply `display:flex; flex-direction:column`,
and the class string the user writes is *additive*, not the complete style. This is a
consequence, not a separate feature, and it's worth stating explicitly because it constrains
every widget's implementation:

| Widget | Tag | Implied classes |
|---|---|---|
| `Column` | `div` | `flex flex-col` |
| `Row` | `div` | `flex flex-row` |
| `Stack` | `div` | `grid` (children placed `col-start-1 row-start-1`, i.e. overlaid) |
| `Grid` | `div` | `grid` |
| `Text` | `span` | *(none — inline by default, matches CSS text flow)* |
| `Button` | `button` | `appearance-none flex items-center justify-center` (kill native button chrome; the flex/centering pair was added in phase 3 — see below) |
| `TextField` | `input` | `appearance-none outline-none` |
| `Image` | `img` | `object-cover` (sane default; overridable) |
| `Icon` | `svg` | sized from a small bundled icon set, `currentColor` fill |
| `ScrollView` | `div` | `overflow-auto` |
| `Switch` | `input[type=checkbox]` | a small CSS-only track/thumb (styled via `::before`, ships in `index.html`) |
| `Slider` | `input[type=range]` | `appearance-none` (native range styling is unfixable across platforms) |
| `List` | `div` | `flex flex-col overflow-auto min-h-0` (a virtualised `ScrollView`; added in phase 3 — see below) |

**12 widgets, chosen from §8's 15**, deferring `List` (needs virtualization to be honest about
performance — building a non-virtualized stand-in would teach a bad habit), `Sheet` (modal
present/dismiss semantics are cheap to get janky and expensive to get right — wants its own
milestone), and `Nav` (a real back stack matters more than a v1 approximation, and §10 phase 4
explicitly promises "navigation with a real back stack" later — no point building it twice).
This is a scope call, flagged the same way phase 0 flagged its own simplifications — reversible,
not hidden.

**Update (phase 3, §16): `Nav` shipped, and the deferral reasoning above is why it
waited.** It was built once phase 3 was shipping real apps on the native renderer, as
the real back stack this note declined to approximate — a `Navigator` store plus Back
handling in all three shells, over a deliberately separated substrate (a back-handler
chain in `RenderContext`, a two-method `NavigationHost` protocol, and the stack as one
policy over it) so a tab/modal navigator or a URL router is a later addition rather
than the second build this note wanted to avoid. It works on **both** backends and
needed no new bridge surface on either: the WebView side reuses the same
Java→JNI→`runtime.py` shape `dispatch` already had. See `../phase_3/PLAN.md` §16.

**Update (phase 3): `List` shipped, and the deferral reasoning above is exactly why it waited.**
It was built once there was a native renderer whose cost actually scaled with row count — on a
real device, a 10,000-row non-virtualised list spends ~3,850ms per scroll frame, which is the
"bad habit" this note declined to teach. It lives in `widgets.py` alongside the other 12 and
works on **both** backends: rows outside the visible window collapse into two fixed-height
spacers, so the container's scrollable extent is identical to a non-virtualised list's and
neither backend's layout engine learns that a list is windowed. The one thing it needed that
nothing else in this file does is a *backwards* flow of state — the runtime reports each scroll
container's `(scroll_y, viewport_height)` into `zkite/_viewport.py`, on this backend through
`zkite-runtime.js` and the existing `dispatch` bridge under a reserved `"scroll"` event name (no
new `ZKiteNative` method, no Java change in either shipped shell). See `../phase_3/PLAN.md` §11.

**Correction (added during phase 3): `Button`'s implied classes were missing `flex items-center
justify-center`.** Found while building the native renderer, not while working on this backend —
every browser centers a plain `<button>`'s content on both axes by default, via internal UA
layout that predates and is independent of author CSS (cross-engine behavior; see e.g.
[the Mozilla bug on it](https://bugzilla.mozilla.org/show_bug.cgi?id=1397768)). This backend
inherited that centering for free from Blink and so never needed to declare it. The native
renderer has no UA stylesheet to inherit anything from, so the same unstyled button rendered
without an explicit centering declaration would show its label flush top-left instead of
centered — a silent visual divergence between backends on every single button, which is exactly
what plan1.md §5's per-widget conformance testing exists to catch before the backend switch
flips. Fixed at the source (`zkite_py/src/zkite/widgets.py`'s `Button.implied_classes`) rather than
patched per-backend, so both backends stay conformant by construction instead of by discipline.
See `../phase_3/PLAN.md` §3.

---

## 4. Hot reload and state preservation

This is the part worth the most design care, because §6's promise — save a file, see the
change in <200ms, keyboard focus and scroll position untouched — is the entire product thesis
("Flutter's renderer isn't why people picked Flutter in 2018; hot reload was" — §11).

**Store identity survives reload by class-swap, not by copying data.**

```python
@store
class Counter:
    n: int = 0
```

`@store` turns the class into a `dataclasses.dataclass` and registers it in a module-qualified
registry (`"main.Counter"` → class object). The *instance* lives separately, one singleton per
qualified name, created lazily the first time `view()`'s annotations ask for it.

On a file change:

1. Re-exec the changed module into a **fresh** module namespace (`importlib.util.spec_from_file_location` + `exec_module` — not `importlib.reload()`, which mutates the live module
   object in place and fights with wanting to *compare* old vs. new class shape before
   committing to anything).
2. For every `@store` class in the new module whose qualified name already has a live instance:
   compare `dataclasses.fields()` (name + type) old vs. new.
   - **Same shape:** reassign `instance.__class__ = new_class`. The instance object — and every
     closure, callback, or reference already holding it — keeps its identity and its `__dict__`
     data. This is the same trick Python's own interactive reload tooling uses; it's the reason
     "state survives" doesn't require inventing a serialization format.
   - **Different shape:** the old instance is discarded, a fresh one is constructed from
     defaults, and the console gets an explicit message naming the store and why (added/removed/
     retyped field) — §6: *"If a store's shape changed incompatibly, that one store resets and
     the console says so — the rest of the app keeps its state."* Only that store resets; a
     second, unrelated `@store` elsewhere is untouched.
3. Re-render: call the (possibly new) `view` with the (possibly reattached) store instances,
   get an HTML string back, hand it to the device-side `morphdom` call.

**Everything above is pure Python and needs no device.** That's deliberate — it's the highest-
leverage, lowest-risk slice of phase 1, fully unit-testable on the host, and it's where this
session's implementation time goes first (§7, M1).

**Finding — morphdom does not preserve a focused input's value by default.** This
matters enough to call out explicitly, the same way phase 0 flagged the `PYTHONHOME`
surprise: the plan (and this document, in an earlier draft) assumed morphdom's
"preserves element state (focus, inputs, scroll)" tagline meant a re-render could
never clobber what a user is mid-typing. It doesn't, by default — `morphdom`'s
`specialElHandlers.js` unconditionally copies `toEl.value` onto `fromEl` for
`<input>`/`<textarea>`, with no focus check. `webruntime/test/e2e.mjs` (M2, below)
reproduced this concretely: type into a field, apply a re-render with a stale
server-side value, and the keystroke is gone. The fix, in
`webruntime/zkite-runtime.js`'s `preserveFocusedInputValue`, is an `onBeforeElUpdated`
hook that overwrites the *incoming* value with the live one when the element is
`document.activeElement`, before morphdom's own sync runs — so the value-copy becomes
a no-op while every other attribute still morphs normally. Caught and fixed in this
session, verified by the same test.

**Transport (`zkite dev` ↔ device) — built, and verified live on an emulator (§7, M4/M5).**
`adb reverse tcp:8765 tcp:8765`, exactly as §6 specifies. `adb reverse` forwards a connection
*made on the device* to a port on the host — so the **device is the TCP client** (a Rust
background thread in `devshell/src/lib.rs::start_reload_server` dials `127.0.0.1:8765` in a
retry loop) and `zkite dev` (`cli/src/dev_cmd.rs`) is the TCP **server**, pushing to it whenever
the file watcher fires. Frames are newline-delimited JSON, one object per line:

```
{"op":"file","path":"main.py","content_b64":"..."}
{"op":"reload"}
```

On connect, the host pushes every project file (an "old manifest is empty" diff, reusing the
same `Manifest::diff` M3 already had) followed by one `reload`; after that, each file-watcher
batch produces one `file` frame per added/modified path and exactly one trailing `reload` — so
five files saved in one editor "save all" still triggers a single re-render, not five. The
device writes each pushed file into its private storage and, on `reload`, calls back into
`zkite_py`'s `reload.py` (unchanged from M1) and renders.

**Log/error frames — built.** After a `reload`, the device sends `{"op":"log","text":"..."}` for
anything printed during it and `{"op":"error","text":"..."}` if `view()` raised, both read by a
background thread in `serve_connection` and printed to `zkite dev`'s terminal — so a traceback now
shows up in both places at once, per §6. `devshell/src/runtime.py::_zkite_last_error` reads
`App.last_error` (already tracked by M1, unchanged) rather than re-deriving it.

**`adb reverse` is re-established on every reconnect wait, not just once at startup.** Found by
actually leaving `zkite dev` running against a real phone for a while: the mapping is scoped to
the current adb connection session and silently drops on a screen lock, USB re-seat, or wifi adb
hiccup, after which the device's retry loop (already built) keeps retrying a tunnel that no
longer exists. `dev_cmd.rs::run` now re-runs `adb reverse` every time it goes back to waiting for
a connection — idempotent, cheap, and fixes "stops working after a while" without restarting
`zkite dev` itself.

**Known gap, still not built:** no delete-file frame — a file removed on the host stays on the
device until the app is reinstalled. Small, well-scoped, same shape as the frames above.

**Pushing the new HTML back into the WebView, on a file-triggered reload, is polling, not a
callback.** A tap-triggered re-render is trivial — `dispatch()` is already a synchronous
Java-calls-Rust JNI call, so the new HTML is just its return value. A *file-push*-triggered
re-render has no Java call to return through: it happens on the reload socket's own background
thread. The tidy answer would be that thread calling back into Java asynchronously
(`AttachCurrentThread`, a global ref to the Activity, a cached method ID, `runOnUiThread`) — real
JNI machinery with real ways to get it wrong. Instead, the reload thread parks the new HTML in a
`Mutex<Option<String>>` and `ZKiteActivity` polls `Native.pollHtml()` every 120ms on the main
thread. At that interval this is comfortably inside the <200ms budget and removes an entire
class of cross-thread JNI bugs from a one-session build — documented as a deliberate simplicity
trade in `devshell/src/lib.rs`'s module doc, not an oversight.

---

## 5. Error overlay

A Python exception raised inside `view()` (or a store method) is caught by `app.py`'s render
loop, formatted with the failing frame and source line (`traceback` + `linecache`, same
approach as phase 0's spike 1 stdout-tee pattern of "observe, don't assume"), and rendered as a
**second** HTML fragment injected over the app's own DOM — not through `morphdom`, since the app
tree may be broken; the overlay is appended as a sibling and removed on the next successful
render. Built and unit-tested in M1 (`zkite_py/tests/test_app_counter.py`).

Mirrored into `zkite dev`'s terminal too now (§6: "simultaneously in the terminal") — see §4's
transport write-up for the log/error frame that carries it host-side. Verified against the real
`zkite_py` glue (`devshell/src/runtime.py::_zkite_last_error`, forcing a `ZeroDivisionError` in
`view()`) on the host; the full device round-trip needs a rebuilt Dev Shell APK to pick up (the
running Dev Shell on any device installed before this fix is still on the old build).

---

## 6. What phase 0 this reuses, unmodified

- **`spike1_cpython_embed`**: the `PYO3_CONFIG_FILE` cross-compile trick, and — critically —
  `config.home` must be set explicitly (`PYTHONHOME` is silently discarded by an isolated
  config). Phase 1's embedding code starts from this file, not from scratch.
- **`spike2_pack_apk`**: reused directly, unmodified — `devshell/scripts/build_apk.sh` calls
  `cargo run --manifest-path phase_0/spike2_pack_apk/Cargo.toml -- --app ... --stage ... --out
  ...` rather than copying any of it, because spike 2's `--app`/`--stage`/`--out` interface was
  already fully generic (nothing in it assumes phase 0's package name or dex). The
  `.pth`-file trick for `lib-dynload/*.so` extension modules (Android 10+ only allows `dlopen`
  from the app's own native lib dir) carries over unchanged in `ZKiteActivity.linkExtensionModules`.
- **`spike3_twcore`**: deliberately *not* reused here (§1). Untouched, waiting for phase 3.

**Finding — `AppCompatActivity` isn't needed, and phase 0's own README was wrong to imply
otherwise for this backend.** Phase 0's README says the shipping shim "needs AppCompat for IME,
insets and the back gesture." That's true for *phase 3's* custom-rendered text field, which has
no real `View` behind it. It is not true for the WebView backend: `WebView` is a real Android
`View` with its own internal `InputConnection`/`ImeAdapter` handling for every `<input>` in the
page, independent of what the *hosting* Activity extends, and insets/back-gesture are both
plain-platform APIs on modern Android (`View.setOnApplyWindowInsetsListener`, used directly in
`ZKiteActivity`). So `devshell/java/dev/zkite/devshell/ZKiteActivity.java` stays on plain
`android.app.Activity` — which sidesteps the one gap phase 0's spike 2 explicitly punted on
(resolving an AndroidX AAR without Gradle) for zero loss on this backend. Revisit only if phase
3's native renderer needs AppCompat for its own, unrelated reasons.

---

## 7. Milestones and what was actually built and verified

| # | Milestone | Needs a device? | Status |
|---|---|---|---|
| M1 | `zkite` Python core: 12 widgets → HTML, `@store` + class-swap reload, `view()` injection, error formatting | No | **Built + tested** — 38 `unittest` cases, `zkite_py/tests/` |
| M2 | WebView runtime: vendored Tailwind JIT + morphdom, `zkite-runtime.js` glue, delegated events | No (browser-testable) | **Built + tested** — real Chromium via Playwright, `webruntime/test/e2e.mjs`, 10/10 checks including the morphdom finding above |
| M3 | `zkite` CLI: `zkite new` scaffolding, file watcher + hash manifest | Partially (watcher/hash: no; adb parts: yes) | **Built + tested** — 14 `cargo test` cases, `cli/tests` inline |
| M4 | Dev Shell APK: `WebView` + JS bridge + JNI + reload-socket listener, embedding `zkite_py` | Yes | **Built and verified live on a running emulator** — see below |
| M5 | Integration: `zkite new` → `zkite dev` → hot-reloading counter app on a device, tap dispatch, live edits | Yes | **Built and verified live on a running emulator** — see below |

M1 and M2 were built and tested first, on purpose — the reload-identity trick and the
HTML-string-via-morphdom decision are the crux of §6's promise, and getting either wrong would
have been expensive to discover after M4's device work. M4/M5 then reused that already-tested
core almost unmodified: `devshell/src/runtime.py` calls straight into `zkite_py`'s `App`,
`reload_module`, and `errors` with no changes to any of them.

### M4 — the Dev Shell (`devshell/`)

Built exactly as scoped, with two real deviations from this document's original sketch, both
explained where they're implemented and cross-referenced above: plain `Activity` instead of
`AppCompatActivity` (§6), and polling instead of an async JNI callback for file-triggered
reloads (§4). Layout:

- `devshell/src/lib.rs` — extends `phase_0/spike1`'s embedding (same `config.home` fix, same
  `PYO3_CONFIG_FILE` cross-compile trick) with JNI entry points (`boot`, `dispatch`, `pollHtml`,
  `startReloadServer`) that are thin wrappers around four Python functions.
- `devshell/src/runtime.py` — the actual logic, in Python, calling `zkite_py` directly (`_zkite_setup`,
  `_zkite_dispatch`, `_zkite_reload`, plus a stdout/stderr tee to logcat reusing spike 1's pattern).
  This is where "Rust's job is plumbing, not framework logic" (§4) is concretely true: this file
  is ~100 lines and does no rendering itself.
- `devshell/java/dev/zkite/devshell/{ZKiteActivity,Native}.java` — WebView host, JS bridge,
  asset unpacking (stdlib + a new `zkite-framework.zip` containing `zkite_py/src/zkite/*.py`,
  unpacked once with the same stamp-file pattern as phase 0's stdlib unpack).
- `devshell/scripts/{build.sh,build_apk.sh,run_on_device.sh}` — cross-compile, stage, and pack
  (via `spike2_pack_apk`, unmodified), and install+launch+logcat, mirroring phase 0's scripts.

**Two bugs found only by running on a real emulator, not by any host-side test:**

1. **`javac -source 8 -bootclasspath android.jar` cannot compile lambdas or method references** —
   fails with `cannot find symbol: method metafactory`. Phase 0's `ZKiteActivity.java` happened to
   need neither, so this never showed up there. Fixed by writing `ZKiteActivity.java` with
   anonymous inner classes throughout, documented in the file's own class doc so nobody
   reintroduces a lambda and loses an hour to a cryptic javac error.
2. **`#zkite-root` needs `display:flex` for a root widget's `flex-1` to mean anything.**
   `flex-grow` only does something inside a flex container; `webruntime/index.html`'s
   `#zkite-root` was a plain block div, so the counter app's `Column("flex-1 ...")` computed
   correctly (confirmed by M2's Playwright test, which checks *that* `display`/`flex-direction`
   resolve, not that the element *fills the screen*) but visibly occupied only its content's
   height on-device — the rest of the screen stayed black. Fixed by making `#zkite-root` a flex
   column itself. This is the kind of gap a computed-style assertion in a headless browser
   can't catch and a screenshot immediately does — exactly why "a stranger's phone," not just
   Playwright, is the real bar.

Also hit and fixed along the way: `zkite-pack`'s AXML compiler doesn't resolve
`android:configChanges`'s flag-string constants (`"orientation|screenSize|keyboardHidden"` →
`For input string: ...`) — dropped from the manifest; harmless for now (Android just restarts
the Activity on rotation instead of resizing in place), and a candidate for the same upstream
`pack` contribution phase 0 already flagged.

### M5 — `zkite dev`, verified end to end

`cli/src/dev_cmd.rs` implements the host side of §4's transport. Verified live, in this session,
against the emulator already running from phase 0's setup (`emulator-5554`, a booted
`sdk_gphone64_x86_64` AVD) — not simulated, not mocked:

1. `zkite new myapp && cd myapp` — real scaffold, as tested in M3.
2. `zkite dev` (Dev Shell already installed and running, showing "waiting for `zkite dev`…") —
   set up `adb reverse`, bound the socket, and the device's already-retrying background thread
   connected within one 500ms retry interval. The host pushed `main.py`/`pyproject.toml`/
   `zkite.tailwind.json` and a `reload`; **the counter app appeared on the emulator**, replacing
   the waiting screen, with no reinstall.
3. `adb shell input tap` on the Increment button — `ZKiteNative.dispatch` → JNI → Python →
   `store.update()` → re-render → `evaluateJavascript` → the on-screen counter went `0` → `1`,
   synchronously, through the full bridge.
4. Edited `main.py` on the host — new background color, new text, new button label and color —
   while `zkite dev` kept running. The change reached the terminal (`pushed 1 change(s) in 0ms:
   ~main.py`) and the emulator's screen updated to the new colors/text **with the counter still
   showing its previous value (survived the `@store` class-swap reload through the real JNI/socket
   path, not just in a host-side unit test)** — screenshotted and visually confirmed.
5. Tapped the *new* button (new label, new handler bound by the fresh render) — counter went
   `1` → `2`, confirming event handlers rebind correctly after a live reload, not just on first
   boot.

This is the concrete, on-device proof of §6's central claim — "install a generic Dev Shell once,
thereafter push only Python, keep state across reloads" — not a description of an architecture
that should work, but a recorded run of it working.

---

## 8. Directory layout

```
phase_1/
├── PLAN.md                 this file
├── zkite_py/                the `zkite` pure-Python wheel (M1)
│   ├── pyproject.toml
│   ├── src/zkite/
│   │   ├── __init__.py
│   │   ├── widgets.py
│   │   ├── _store.py
│   │   ├── app.py
│   │   ├── reload.py
│   │   ├── errors.py
│   │   └── tailwind_config.py
│   └── tests/
├── webruntime/              WebView HTML/JS shell (M2)
│   ├── vendor/              tailwind-browser.js, morphdom.min.js (fetched, pinned versions)
│   ├── zkite-runtime.js
│   ├── index.html
│   └── test/{e2e.mjs,harness.html}
├── cli/                     zkite CLI Rust crate (M3 + M5's `dev` command)
│   ├── Cargo.toml
│   └── src/{main,new_cmd,manifest,watch,dev_cmd}.rs
├── devshell/                the Dev Shell APK (M4)
│   ├── Cargo.toml
│   ├── src/{lib.rs,runtime.py,bin/probe.rs}
│   ├── java/dev/zkite/devshell/{ZKiteActivity,Native}.java
│   ├── app/{AndroidManifest.xml,res/}
│   └── scripts/{build.sh,build_apk.sh,run_on_device.sh}
└── templates/new_project/   what `zkite new` scaffolds
```

## 9. Exit criterion tracking

§10's Phase 1 exit: *"a stranger goes from `pip install` to a hot-reloading counter app on
their phone in under 3 minutes, unassisted."* The architecture is now proven end to end on a
real emulator (§7, M4/M5) — `zkite new` → `zkite dev` → a live, hot-reloading, tap-responsive
counter app, with state surviving reload, no reinstall. What's still between here and the
literal exit criterion is packaging and polish, not open architectural questions:

- **`pip install zkite`**: doesn't exist yet — `zkite_py` has a `pyproject.toml` but hasn't been
  built or published as a wheel; the CLI binary isn't distributed as one either (that's the
  vendoring/packaging half of what phase 2's bundler work implies for the CLI itself).
- **The Dev Shell doesn't auto-install**: `zkite dev --devshell-apk <path>` can install+launch it,
  but there's no "first run ever, fetch and install the Dev Shell automatically" path — today it
  has to be built (`devshell/scripts/build_apk.sh`) and pointed at explicitly.
- **arm64 confirmed on a real phone, since the above was written.** `ABI=arm64-v8a
  devshell/scripts/build_apk.sh` and `zkite new` → `zkite dev` → hot reload → Tailwind styling were
  all independently verified working on a physical arm64 device, matching the bar phase 0 set for
  itself. What that pass predates: the log/error-frame and `adb reverse` re-establishment fixes
  just above — a device running the pre-fix Dev Shell needs a rebuilt APK to pick them up.
- **Timing wasn't measured.** The live reload in §7 *felt* well under 200ms (the terminal log
  showed `0ms` for the manifest diff itself), but no stopwatch was run against on-screen change,
  the way phase 0 measured install/launch times precisely. Worth doing before claiming the <200ms
  number rather than gesturing at it.
- **Known gaps carried over from §4/§5**: no delete-file frame, no `zkite doctor`, no real
  keystore story, WebView still allows pinch-zoom and browser-style text selection (all
  explicitly phase 2 or small polish items, not architectural).

What's fully built and independently tested at each layer:

- **`zkite_py`**: 38 `unittest` cases — all 12 widgets, `@store` identity-preserving reload
  (same-shape migrate, incompatible-shape reset, unrelated stores untouched), the full §8 counter
  app end to end, and the error-overlay path.
- **`webruntime`**: real-Chromium (Playwright), not a mock DOM — 10/10 checks, including the
  morphdom focus/value-preservation finding (§4) this test is what caught.
- **`cli`**: 14 `cargo test` cases — `zkite new` (validated end to end through `zkite_py`'s `App`,
  not just as a file-scaffolding check), hash-manifest diffing, debounced file watching.
- **`devshell` + `zkite dev` together**: not unit-tested (this layer is inherently an integration
  of JNI, a real WebView, and a real socket — a unit test would mostly test mocks), but run live
  end to end on a real emulator, with the two bugs that run surfaced (§7) fixed and re-verified,
  not just attempted once and left unconfirmed.
