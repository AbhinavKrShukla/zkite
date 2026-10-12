> Historical design notes. See the current user guides and docs/status.md for supported behavior.

# ZKite — a Python framework for Android

> Current installation/API guide: [Documentation.md](../../Documentation.md).
> Release verification and remaining work: [release status](../status.md).

**Zero boilerplate. Sub-second hot reload. Tailwind-native styling. Rust bundler, no Gradle.**

*("ZKite" is a placeholder name — see §12 for naming notes.)*

> **This is the original design document, kept as written.** Phases 0–3 are built; where
> reality has diverged from a claim below, the claim carries an inline status note rather
> than being rewritten (see §5's rendering table, §8's widget list, §10's phase-3 exit).
> **§14 — added after the fact — is the honest inventory of what is still missing**, and is
> the right place to start if you are asking "what does this need to be a real framework?"
> Each phase's own `PLAN.md` (`phase_0/README.md`, `phase_{1,2,3}/PLAN.md`) is more current
> than this file and is the source of truth for what is actually done.

---

## 0. The one-paragraph pitch

```bash
python -m venv .venv && source .venv/bin/activate
pip install zkite
zkite new myapp && cd myapp
zkite dev          # app is on your phone in ~4 seconds, hot reloading
zkite build apk    # signed APK in ~5 seconds
```

No JDK. No Gradle. No Android Studio. No Android SDK. No NDK. No Node. No Flutter.
The only external tool the user needs is `adb`, and `zkite` can vendor that too.

That constraint — *the user's machine installs nothing but a wheel* — is the single design
decision that everything else in this document follows from.

---

## 1. Why this is possible now (and wasn't 18 months ago)

Four things landed recently that make this a build-it project rather than a research project:

| Enabler | Status (Aug 2026) | Why it matters |
|---|---|---|
| **CPython official Android support** (PEP 738) | Tier 3 since 3.13; python.org now ships **official Android binary releases** from 3.14.0rc2 onward (current: 3.14.6) | You no longer maintain a patched CPython fork. You download an official prefix with `libpython3.14.so`, stdlib, and headers per ABI. |
| **PyPI + pip Android wheel tags** | pip supports PEP 738 `android_<api>_<abi>` wheels; PyPI accepts uploads | Dependency resolution for Android is *just pip*, with a platform tag. No custom recipe system (contrast: buildozer's `p4a` recipes). |
| **`google/pack`** | Rust reimplementation of `aapt2` + `zipalign` + `apksigner` + `bundletool`. Explicit goal: *"do not depend at runtime on Java, OpenSSL, the Android SDK, or android.jar."* Apache-2.0. | This is the Gradle killer. APK **and** AAB, ARSC + AXML + ProtoXML encoding, v1/v2/v3 signing — all pure Rust, runs anywhere including on-device. |
| **Rust UI stack matured** | Taffy (flexbox/grid, powers Zed + Bevy), Parley (text layout), Vello CPU / Vello Hybrid (Vello Hybrid reached beta quality in 2026 Q1), `android-activity` | A Tailwind-semantics renderer is now assembly of existing crates, not a from-scratch Skia port. |

Note the shape of the gap in the market. Flet is the closest competitor and it is genuinely
good — but `flet build apk` needs **Flutter SDK + JDK 17 + Android SDK**, auto-downloads them
on first run, and that first build takes 10–30 minutes. Briefcase and Buildozer are in the same
family. Nobody is shipping a *toolchain-free* Python → Android path, and nobody is doing
Tailwind on mobile Python at all.

---

## 2. Non-negotiable design principles

1. **Nothing compiles on the user's machine.** All native artifacts (`libpython`, the Rust
   core, the dex shim, the CLI) are cross-compiled once in *our* CI and shipped as binaries.
   Gradle exists — in our release pipeline, once per release, never in a user's terminal.
2. **The dev loop never builds an APK.** Hot reload beats fast builds. See §6.
3. **The Python API is backend-agnostic.** App code says `Column(...)`, not HTML. This lets us
   ship a boring backend first and swap in the good one later without breaking anyone (§5).
4. **Tailwind is a compiler target, not a CSS pipeline.** We never emit CSS. Class strings
   compile to `ComputedStyle` structs in Rust. See §4.
5. **Boilerplate is a bug.** If a file has a line that is the same in every project, delete
   the line and infer it.

---

## 3. Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  YOUR APP           main.py  ·  pyproject.toml  ·  assets/  │
└─────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  zkite (pure Python wheel)                                   │
│  widgets · reactive stores · hot-reload client · typing     │
└─────────────────────────────────────────────────────────────┘
                     │ CPython C-API (builtin module, no IPC)
┌────────────────────▼────────────────────────────────────────┐
│  libzkite.so  (Rust, prebuilt per ABI)                       │
│                                                             │
│  twcore    Tailwind class → ComputedStyle (our impl)        │
│  taffy     flexbox / grid / block layout                    │
│  parley    text shaping + line breaking (swash/harfbuzz)    │
│  vello_hybrid │ vello_cpu    rasterisation (GL ES3 / CPU)   │
│  accesskit AccessibilityNodeProvider bridge                 │
│  android-activity  lifecycle · input · IME · surfaces       │
└─────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  classes.dex (prebuilt, ~40KB) — ZKiteActivity extends       │
│  AppCompatActivity: IME/InputConnection, back gesture,      │
│  insets, permissions, a11y node provider, file picker       │
└─────────────────────────────────────────────────────────────┘
┌─────────────────────────────────────────────────────────────┐
│  libpython3.14.so + stdlib.zip   (official python.org)      │
└─────────────────────────────────────────────────────────────┘
```

And the host side:

```
zkite CLI (Rust binary in the wheel)
├── zkite dev      device discovery (adb) · file watcher · reload server
├── zkite build    wheel resolution · pyc · pack (ARSC/AXML/sign/align) · AAB
├── zkite doctor   which deps lack Android wheels, adb reachable, etc.
└── zkite run      install + launch + logcat filtering
```

### Why a dex shim at all?

The tempting move is `NativeActivity`, because it ships in the OS and needs **zero** Java —
`cargo-apk` produces APKs with no `classes.dex`. But `android-activity`'s own docs are blunt
about the catch: NativeActivity has no input-method support and can only receive physical key
events, which *"is unlikely to be sufficient for production applications."* Real-world Rust UI
ports confirm it — IME unsupported, window doesn't resize around the keyboard, no soft-keyboard
language switching.

An app framework where text fields don't work is not an app framework. So we ship a real
`AppCompatActivity`. The trick is that **we compile it once**, at framework release time, and
vendor the resulting `classes.dex` into the wheel. The user still never sees a JDK.

---

## 4. Tailwind as a first-class native styling engine

This is the differentiator, so it's worth being precise about what we can and cannot reuse.

**What we can't reuse:** Tailwind v4's Oxide engine is Rust, but Oxide is only the *scanner*
(it walks files and extracts class candidates). The actual compiler — `@theme`, `@utility`,
variants, the utility table — is TypeScript, and it emits CSS. We don't want CSS and we can't
embed a TS runtime in a mobile app.

**What we build instead — `twcore`:** a Rust crate that resolves a class string directly to a
computed style struct. No CSS, no cascade, no selector matching. The utility grammar is public
and finite.

```rust
pub struct ComputedStyle {
    pub layout: taffy::Style,        // flex, grid, gap, p-*, m-*, w-*, h-*, inset-*
    pub paint: Paint,                // bg-*, text-*, border-*, ring-*, shadow-*, opacity-*
    pub text:  TextStyle,            // font-*, text-{size}, leading-*, tracking-*
    pub transform: Transform,        // rotate-*, scale-*, translate-*
}

// resolve("flex-1 px-4 py-2 rounded-xl bg-indigo-500 text-white active:bg-indigo-600")
//   -> ComputedStyle, memoised by FxHash of the class string
```

Scope for v1, in priority order:

- **Core utilities:** layout (flex, grid, gap, position, inset), box model (p/m/w/h/min/max),
  typography, colours, borders, radii, shadows, opacity, overflow.
- **Arbitrary values:** `w-[137px]`, `bg-[#0f172a]`, `text-[13px]` — cheap to support, and
  removes the #1 escape-hatch complaint.
- **Variants:** `hover:` `active:` `focus:` `disabled:` `dark:` `sm:/md:/lg:` `rtl:`
  `landscape:`. Implemented as a small bitset of active states per node, so variant resolution
  is a mask-and-pick, not a re-parse.
- **Theming via `@theme`-equivalent:** a `theme.py` (or `[tool.zkite.theme]` in pyproject) that
  overrides the token tables — colours, spacing scale, font families, radii. Same mental model
  as v4's CSS-first config, expressed in Python.

**Explicitly out of scope:** arbitrary CSS selectors, the cascade, `@apply` against real
stylesheets, container queries (v2), `:has()`. We are Tailwind-*compatible*, not CSS-compatible,
and the docs should say so on page one. That honesty is worth more than a fuzzy claim.

**Pluggable engines** (your "along with others"):

```python
class StyleEngine(Protocol):
    def resolve(self, spec: str | dict, state: NodeState) -> ComputedStyle: ...
```

Ship three: `tailwind` (default), `tokens` (semantic design-token names — `surface.raised`,
`text.muted`), and `inline` (plain dicts, for people who hate class strings). Third parties can
register via entry points. Engines are resolved per-app, not per-widget, so there's no
performance cliff from mixing.

**Editor story:** generate a `zkite.tailwind.json` on `zkite new` so the standard Tailwind
IntelliSense extension autocompletes inside `Column("...")` string literals. Cheap to build,
disproportionately good first impression.

---

## 5. Rendering: pick the ending, ship the middle

Three real options, honestly compared:

| | WebView | Native Android Views | **Custom GPU renderer** |
|---|---|---|---|
| Tailwind fidelity | Perfect (it's CSS) | Poor — Android layout params don't map to flexbox | Excellent — Taffy *is* flexbox/grid |
| Feel | Web-in-a-box; scroll and keyboard jank | Genuinely native | Flutter-like: consistent, controllable |
| Hot reload | Awkward (DOM + Python state desync) | Heavy JNI chatter per update | Trivial — everything is our own data |
| Effort to v1 | ~4 weeks | ~6 months | ~4 months |
| a11y | Free | Free | Must build (AccessKit) |
| Ceiling | Low | Medium (Toga has lived here for a decade) | High |

> **Status (2026-08-07): this table has now been measured, and it is the one part of this
> document that the evidence has moved.** `phase_3/PLAN.md` §12/§13 ran both backends
> against the same apps on the same phones. On "Feel", the charge against the WebView —
> *"scroll and keyboard jank"* — did not survive: the WebView holds a locked 60fps
> scrolling a 10,000-row list, and the native renderer reached ~58fps only after a pacing
> bug was fixed. Native wins interaction latency ~3.6x and loses cold start (~1450ms vs
> ~960ms), memory (~310MB vs ~235–290MB) and 15.5MB of binary. The verdict §13.4 records
> is **"comparable but not better."** The "Ceiling" and "Tailwind fidelity" rows remain
> *untested* rather than contradicted — nothing has measured them.
>
> **Update (`phase_3/PLAN.md` §20): the memory figure above is superseded and the sign
> flips.** The ~310MB was almost entirely wgpu's `MemoryHints::Performance` default —
> which ZKite never set — reserving 128MB device-local and 64MB host-visible
> suballocation blocks up front, independent of what the app draws. Proof it was a
> reservation rather than a cost: it sat flat at ~214MB of `GL mtrack` across a 5-node
> app, a 6-node app and a 152-node list, then moved to 79MB and 36MB purely by shrinking
> the allocator's blocks. Setting the hint explicitly takes the stock counter to
> **~155MB** total resident and a 10,000-row list to **~135MB**, below the WebView's
> 235MB, with scroll frame time unchanged. Cold start and binary size are unaffected.
> The swapchain half (~57–67MB) is still unattributed.
>
> **The backend switch flipped to native anyway** (`phase_3/PLAN.md` §15.4), as a product
> decision made against that ledger rather than because of it: the argument is control
> over rendering — a system WebView's version varies per device and updates out from under
> the app — not speed. Worth knowing before quoting the table above as settled.

**Decision: build for the custom GPU renderer, but ship the WebView backend first.**

Because the Python API is `Column(...)` and not raw HTML, the backend is swappable underneath
without touching app code. That gives you a usable product in week 8 instead of month 6, real
users generating real feedback on the API and the DX, and a de-risked path to the good backend.
Every widget gets a conformance test that must produce pixel-comparable output on both backends
before the switch flips.

The target stack, concretely: **Taffy** for layout, **Parley** for text shaping and line
breaking, **vello_hybrid** for rasterisation with **vello_cpu** as fallback. Hybrid does CPU
preprocessing + GPU rasterisation via GL ES, which matters on Android — the original
compute-shader Vello is more demanding than the low end of the device fleet supports. Vello
Hybrid hit roughly beta quality in 2026 Q1; there were known WebGL/Adreno issues under active
fix, which is precisely why `vello_cpu` stays wired in as a runtime fallback rather than an
afterthought.

---

## 6. The dev loop (the thing people will actually judge you on)

**`zkite dev` never builds an APK.** It installs a generic **ZKite Dev Shell** — a prebuilt APK
containing the runtime and nothing app-specific — once, and thereafter pushes only Python.

```
first run:   adb install zkite-dev-shell.apk   (~3s, one time ever)
             push app source                   (~200ms)
             launch                            (~800ms)
             ─────────────────────────────────────────
             total: ~4s from `zkite dev` to pixels

every save:  hash-diff manifest → push changed files only
             → reload signal over adb reverse tcp
             → re-exec modules → rebuild tree → diff → patch
             ─────────────────────────────────────────
             target: <200ms, keyboard state and scroll position intact
```

**Preserving state across reload** is the hard part, and the answer is to keep state out of
module globals:

```python
@store
class Cart:
    items: list[Item] = field(default_factory=list)
    coupon: str | None = None
```

`@store` registers the instance in a runtime-owned registry keyed by a stable identity. On
reload the module is re-executed, the class object is replaced, and live state is re-attached
by identity. If a store's *shape* changed incompatibly, that one store resets and the console
says so — the rest of the app keeps its state. `zkite dev --restart` is the nuclear option.

**Transport:** `adb reverse tcp:8765 tcp:8765` for USB, wireless debugging (`adb connect`) for
untethered. A hash manifest means a one-character edit sends one file, not a tree.

**Error surface:** a Python traceback renders as an overlay *on the device* with the failing
frame and source line, and simultaneously in the terminal. Nobody should ever have to
`adb logcat | grep`. This is a small feature that buys enormous goodwill.

---

## 7. The bundler

`zkite build apk` / `zkite build aab`, implemented in Rust, driving a fork of `google/pack`.

```
 1  resolve   pip --platform android_21_arm64_v8a --only-binary=:all: → wheels
 2  compile   .py → .pyc  (source-strippable)
 3  bundle    app + deps → assets/app.zip           (zipimport-able)
 4  stage     libpython3.14.so · libzkite.so · stdlib.zip · classes.dex
 5  resources icons, splash, strings → ARSC + binary AXML      [pack]
 6  sign      APK Signature Scheme v2 + v3                     [pack-sign]
 7  align     4-byte zip alignment                             [pack-zip]
 8  emit      app-arm64.apk / app.aab
```

**Why this is fast:** for a pure-Python app, *nothing is compiled*. It's dependency resolution
(cached), bytecode compilation (milliseconds), zipping, and signing. Signing a ~35MB APK is the
dominant cost. A 3–8 second release build is a realistic target, against Gradle's 45s–3min
incremental and Flet's 10–30 minute cold start.

**Size budget, per-ABI split:**

| Component | Approx |
|---|---|
| `libpython3.14.so` | ~10 MB |
| stdlib.zip (full) | ~8 MB → ~2.5 MB pruned |
| `libzkite.so` (vello + taffy + parley) | ~7 MB |
| `classes.dex` + resources | <0.5 MB |
| **Baseline APK** | **~20–26 MB** |

`--prune-stdlib` runs `modulegraph`-style static analysis, drops unreachable stdlib modules, and
prints what it removed so the failure mode is diagnosable rather than mysterious.

### Known gaps in `google/pack` — plan for them explicitly

It's a proof of concept (74 commits, no releases, explicitly "not an officially supported Google
product"), currently aimed at Wear OS watch faces. Its own compatibility table flags:

- **Resource qualifiers not supported** — single-language `strings.xml` only, `drawable/` only
  (no `drawable-xhdpi`, no `values-es/`).
- **v4 signing not implemented.**

Mitigations, in order of preference:
1. **Sidestep.** We *generate* the resource set, so we can emit a single density-independent
   adaptive icon and do localisation in the Python/renderer layer rather than via `values-<lang>/`.
   For a framework that owns its own rendering, resource qualifiers are largely vestigial.
2. **Contribute upstream.** Qualifier support is a well-defined, self-contained addition and
   the project explicitly invites contributions. Best outcome for everyone.
3. **Fork.** Vendor `pack-*` crates. Apache-2.0 permits it. Budget 3–4 weeks if qualifiers
   turn out to be unavoidable.

v4 signing is a non-issue — it's for `adb install --incremental`, not for Play.

**Keystore handling:** `pack` signs from a PEM. Play upload keys are conventionally PKCS#12/JKS.
Wire in `rcgen` + `p12` so `zkite keygen` produces both forms and `zkite build --keystore` accepts
what developers already have. Don't make people install `keytool` at the last mile — that would
undo the entire premise.

> **Status (2026-08-07): half done, and the missing half is what blocks migration.**
> `zkite keygen` produces both forms as promised. `zkite build --keystore` reads a
> **combined PEM only** — it does *not* accept "what developers already have", so
> someone arriving with an `upload-keystore.jks` or a `.p12` (including one `zkite
> keygen` itself wrote) is stuck. There is also **no certificate-fingerprint command
> at all**, so no SHA-1 for Firebase / Google Sign-In / Maps. Researched in
> `phase_3/PLAN.md` §17.3: the `jks` crate (MIT) decodes both formats behind one
> dependency, so the reader is small — and a fingerprint command has to distinguish
> the *upload* key from Play's own *app signing* key, because only the latter matches
> what users install and ZKite cannot compute it.

---

## 8. The API

`zkite new myapp` produces exactly two files. This is the whole app:

```python
# main.py
from zkite import Column, Text, Button, store

@store
class Counter:
    n: int = 0

def view(c: Counter):
    return Column("flex-1 items-center justify-center gap-6 bg-slate-950",
        Text(f"{c.n}", "text-7xl font-bold text-white tabular-nums"),
        Button("Increment", "px-6 py-3 rounded-2xl bg-indigo-500 active:bg-indigo-600 text-white font-medium",
               on_tap=lambda: c.update(n=c.n + 1)),
    )
```

```toml
# pyproject.toml
[project]
name = "myapp"
version = "0.1.0"
```

Design notes:

- **No `App()`, no `run()`, no `if __name__ == "__main__":`.** A module-level `view` function is
  the entry point. The runtime finds it. If you need an explicit app object for advanced cases,
  `zkite.App` exists — but the 20-line demo must not mention it.
- **First positional arg is always the class string; children follow as `*args`.** Reads
  cleanly, no keyword noise, no `__getitem__` cleverness.
- **Stores are injected by type annotation.** `view(c: Counter)` gets the live `Counter`. No
  provider tree, no context, no dependency-injection ceremony.
- **`on_tap`, not `on_click`** — it's a phone.
- Full type hints throughout so autocomplete works everywhere, including on class strings via
  the generated Tailwind config.

Widget set for v1 (deliberately small, all Tailwind-styleable): `Column`, `Row`, `Stack`,
`Grid`, `Text`, `Button`, `TextField`, `Image`, `Icon`, `ScrollView`, `List` (virtualised),
`Switch`, `Slider`, `Sheet`, `Nav`. Plus `Native(...)` as an escape hatch that embeds a real
Android view.

> **Status (2026-08-07): 13 of those 15 exist.** `Sheet` and `Native(...)` do not.
> **`Nav` now does**, along with the thing that was bigger than the widget — real
> navigation and back-stack handling in every shell, built as a substrate a different
> navigation system can be written against rather than as one hardcoded stack. See
> `phase_3/PLAN.md` §16.
>
> This list is also thinner than a production app needs even when complete: it has no
> `Checkbox`, `Radio`, `Select`/picker, `DatePicker`/`TimePicker`, `Dialog`, `Toast`,
> `ProgressIndicator`, or `Carousel`/`PageView`. See §14 for the full inventory and
> `phase_3/PLAN.md` §15.1 for the per-item detail.

---

## 9. Risk register

| Risk | Severity | Mitigation |
|---|---|---|
| **PyO3 embedding on Android is not officially supported** — the maintainers point people at BeeWare's experience and suggest shipping your own Python build | High | Ship our own build (we already do). Link against the python.org Android prefix via `PYO3_CROSS_LIB_DIR`. Spike this in **week 1** — if it resists, drop PyO3 and use raw `pyo3-ffi`/`bindgen` against the C API, which is more code but has no build-script magic to fight. |
| **`google/pack` maturity** | High | §7. Three-tier mitigation. Sidestep is genuinely viable. |
| **IME / text input on a custom renderer** | High | This is why the dex shim exists. Budget 4 weeks for `InputConnection`, composing regions, autocorrect, IME-aware insets. Do it *early* — it's the thing that reveals whether the whole architecture holds. |
| **Accessibility** | High (Play quality bar) | AccessKit → `AccessibilityNodeProvider` in the shim. Not optional, not deferrable to v2 — retrofitting a11y onto a widget tree is far worse than building it in. |
| **Binary dependency availability** | Medium | Android wheels on PyPI are new and thin. `zkite doctor` reports exactly which deps lack `android_*` wheels. Maintain a small supplementary index of prebuilt popular ones (`numpy`, `pydantic-core`, `cryptography`) built with `cibuildwheel`, which CPython's own Android docs recommend for this. |
| **16 KB page size** | Medium | Play requires 16KB-aligned native libs for apps targeting recent Android. Build with NDK r27+ and the right linker flags; add a `zkite build` assertion that fails loudly rather than at Play upload. |
| **Vello Hybrid beta-ness** | Medium | `vello_cpu` fallback selected at runtime by GPU probe. Ship a device compatibility matrix from real CI-on-device runs. |

> **Status (2026-08-07) on the row above, and on a risk this table never named:** the
> `vello_cpu` fallback is real and has been exercised on hardware (`phase_3/PLAN.md`
> §4), and it earned its keep on the first real device for a reason nobody predicted —
> wgpu's *default device limits* exceeded what a current mid-range phone supports.
> **That turned out to be an instance of a general hazard worth stating on its own:
> wgpu's defaults are desktop-shaped, and ZKite has now been bitten by two of them.**
> The second (`phase_3/PLAN.md` §20) was `MemoryHints::Performance`, wgpu's default,
> which reserves 128MB/64MB GPU suballocation blocks regardless of use and accounted
> for nearly all of the ~310MB resident a ZKite app was costing. Both were inherited
> silently via `..Default::default()`. The lesson generalises past these two: on this
> platform, every wgpu default is a desktop assumption until measured.
| **Startup time** | Medium | CPython init is ~100–200ms on mobile ARM. Draw the splash from Rust on frame 1, initialise Python off the UI thread, precompute `.pyc`, use frozen imports for the framework itself. Target: first pixel <100ms, interactive <500ms. |

> **Status (2026-08-07): the "precompute `.pyc`" mitigation was the right instinct and
> was being defeated by one line.** All three shells set `config.write_bytecode = 0`,
> so CPython cached nothing, while the pruned stdlib ships as ~490 *source* files —
> every launch re-parsed all of them. Flipping it, measured on one phone with one
> variable changed: **native 2584ms → ~990ms, WebView 978ms → 420ms**
> (`phase_3/PLAN.md` §17.1). So **the "interactive <500ms" target above is now met by
> the WebView backend**, and §12.6's conclusion that it was missed "by the
> architecture" was wrong — it was missed by a default. The remaining native gap is
> ordinary work, and the rest of this row's mitigation list (splash from Rust on frame
> 1, frozen imports) is still unbuilt.
| **Scope explosion** | Very high | Read §11. |

---

## 10. Roadmap

### Phase 0 — De-risk (2 weeks)
Three spikes, each pass/fail, nothing else:
1. Embed official CPython 3.14 Android in a Rust `cdylib`; run `print("hi")` from
   `android_main`; confirm it works on arm64 + x86_64 emulator.
2. Build a signed, installable APK end-to-end using `google/pack` with a prebuilt dex + a
   native lib. No framework — just prove the toolchain-free path produces something that runs.
3. `twcore` prototype: parse 40 common utilities into a `taffy::Style` + paint struct.

**If any spike fails, the architecture changes.** Don't write framework code until all three pass.

### Phase 1 — DX skeleton, WebView backend (6 weeks)
`zkite new` · `zkite dev` · hot reload with state preservation · 12 widgets · Tailwind resolver
covering the core utility set · on-device error overlay · Dev Shell APK.
**Exit:** a stranger goes from `pip install` to a hot-reloading counter app on their phone in
under 3 minutes, unassisted.

### Phase 2 — The bundler (6 weeks)
`zkite build apk/aab` · wheel resolution for Android tags · pyc + zipimport · signing and
keystores · stdlib pruning · `zkite doctor`.
**Exit:** a signed AAB is accepted by the Play Console. Release build under 10s.

### Phase 3 — Native renderer (12 weeks)
Taffy + Parley + Vello Hybrid · IME · scrolling and virtualised lists · gestures · AccessKit ·
per-widget conformance against the WebView backend.
**Exit:** the backend switch flips by default and no sample app changes a line.

> **Status (2026-08-07): the exit criterion is literally met and phase 3 is not done.**
> `zkite build` and `zkite dev` both default to the native backend, `--backend webview`
> selects the other, and "no sample app changes a line" is checked rather than asserted —
> the compiled `main.pyc` is byte-identical across both backends' APKs
> (`phase_3/PLAN.md` §14.5). Everything §10 names for this phase is built and verified on
> real hardware: IME, virtualised lists, gestures, AccessKit, conformance.
>
> What the criterion does not capture is that the native renderer still cannot draw
> several ordinary things — `Image` and `Icon` paint empty boxes, borders/shadows/rings
> aren't painted, horizontal scrolling doesn't scroll, `dark:` never activates. **The exit
> criterion turned out to be a poor proxy for "ready."** See §14.
>
> (Of that list, only `dark:` is still true: `Icon` landed in `phase_3/PLAN.md` §18,
> horizontal scrolling in §27, borders/shadows/rings in §28, and `Image` in §35. The
> point about the exit criterion stands regardless — it took five further sessions of
> ordinary rendering work after "phase 3 complete" to get here.)

### Phase 4 — Credibility (ongoing)
Animation, navigation with a real back stack, camera/location/notification plugins, iOS
(the Rust core is already portable; swap `android-activity` for a UIKit shell), desktop
(free — it's the same renderer on winit), CI action, a real docs site, three flagship sample
apps on the Play Store.

Roughly **6–7 months to a credible 1.0** with 2–3 experienced engineers. Solo, double it, and
consider dropping Phase 3 to stay on the WebView backend for 1.0 — a shipped WebView framework
with a great dev loop beats an unshipped GPU renderer.

---

## 11. The advice you didn't ask for

The plan above is technically sound. The likeliest failure mode isn't technical.

**Cut Phase 3 from 1.0 if you're not fully staffed.** The custom renderer is the most fun part
and the least load-bearing for adoption. What makes people love a framework is `pip install`
to running app in 3 minutes and saves that appear instantly. Flutter's renderer isn't why
people picked Flutter in 2018; hot reload was. Ship the dev loop, earn users, build the renderer
with their pressure behind it.

**Contribute to `google/pack` rather than forking it immediately.** Your entire "no Gradle"
claim rests on a 74-commit proof of concept with no releases. A fork is a permanent tax; a few
upstreamed PRs make it a shared asset with Google's name on it, which is also the single best
piece of marketing this project could have.

**Do the boring hard things first.** IME and accessibility, done in month 2, shape the
architecture correctly. Done in month 10, they cause a rewrite. Every custom-renderer UI
framework that skipped them regretted it.

**Ruthlessly define what you are not.** Not CSS-compatible — Tailwind-compatible. Not
supporting every PyPI package — supporting packages with Android wheels. Not a game engine.
Every one of these written on the front page saves a hundred disappointed issues.

---

## 12. Naming

`zkite` is a placeholder (a defunct AI-autocomplete company used it, which muddies search
results). Check PyPI, crates.io, and npm availability before committing. Criteria: short,
typeable, no vowel-dropping, imports cleanly (`from X import Column`), and reads well as a verb
in the CLI (`X new`, `X dev`, `X build`).

---

## 13. Reference index

**Foundations**
- PEP 738 — Adding Android as a supported platform · <https://peps.python.org/pep-0738/>
- Using Python on Android (official docs) · <https://docs.python.org/3/using/android.html>
- Python Android binary releases · <https://www.python.org/downloads/android/>

**Bundling**
- `google/pack` — Rust aapt2/zipalign/apksigner/bundletool replacement · <https://github.com/google/pack>
- `rust-mobile/xbuild` — prior art for APK creation from Rust · <https://github.com/rust-mobile/xbuild>
- `rust-mobile/cargo-apk`, `cargo-ndk` — NDK cross-compilation

**Runtime & rendering**
- `rust-mobile/android-activity` — NativeActivity vs GameActivity, and the IME caveat · <https://github.com/rust-mobile/android-activity>
- `DioxusLabs/taffy` — layout · <https://github.com/DioxusLabs/taffy>
- `linebender/vello` — Vello / Vello CPU / Vello Hybrid · <https://github.com/linebender/vello>
- `DioxusLabs/blitz` — closest architectural cousin; note their README explicitly welcomes bindings for other languages including Python · <https://github.com/DioxusLabs/blitz>
- PyO3 on Android discussion · <https://github.com/PyO3/pyo3/discussions/4824>

**Styling**
- Tailwind v4 / Oxide architecture · <https://tailwindcss.com/blog/tailwindcss-v4-alpha>

**Competitive landscape**
- Flet (Flutter + Python; requires Flutter SDK, JDK 17, Android SDK) · <https://flet.dev/docs/publish/android/>
- BeeWare Briefcase + Toga · <https://beeware.org>
- Kivy + Buildozer
---

## 14. What's actually missing — the gap between this document and a production framework

*Added 2026-08-07, after phases 0–3 shipped and the backend switch flipped. This section
did not exist in the original plan, and its absence was itself a gap: §10's phase exits
are milestones, not a definition of "done," and hitting all of them still leaves an app
that cannot navigate between two screens. Per-item detail lives in `phase_3/PLAN.md`
§15.1, which this summarises.*

### 14.1 The one that blocked everything — now built

> **Status (2026-08-07): built and verified live on both backends. See
> `phase_3/PLAN.md` §16.** The paragraph below is kept as written, because it was the
> accurate statement of the gap and it is what motivated the work.

**There is no navigation and no back stack, anywhere, in any shell.** No `onBackPressed`,
no back-gesture handling, no screen stack, no `Nav` widget. Every ZKite app is one screen
and the Back button exits it. §10 files navigation under phase 4 ("navigation with a real
back stack"), and that ordering assumed phase 3 would not be shipping apps yet. It now is.
This is backend-agnostic work and it is the single largest gap in the framework.

**What now exists:** a `Nav` widget, a `Navigator` stack store, and Back handling in all
three shells — deliberately built as three layers (a back-handler chain, a two-method
`NavigationHost` protocol, and the stack as *one* policy over it) so that a tab
navigator, a modal navigator or a URL router can be added later without touching the
substrate. Stack entries hold screen *names*, not function objects, so a pushed screen
picks up a hot-reloaded edit. Verified on a Moto G54 on both backends: three screens,
push twice, Back twice with `@store` state intact, and Back at the root still exits —
plus a no-`Nav` control app confirming one-screen apps are unchanged.

**Still open, and still filed under phase 4:** screen transition animations (there is no
animation driver at all), `Sheet`/`Dialog`, deep links, and state restoration after
process death.

### 14.2 Silent divergences between the two backends

The dangerous class: these work on the WebView backend and quietly do nothing on the
native one, so the app renders, nothing errors, and the behaviour is simply absent.
None of them is visible to the per-widget conformance harness §5 promises, because that
harness renders one state, light mode, unscrolled, at a fixed size.

- ~~**Horizontal scrolling doesn't scroll.**~~ **Built and verified live on both
  backends (`phase_3/PLAN.md` §27).** The renderer's scroll API is axis-generic, the
  render thread resolves a scroll target *per axis* and locks a drag to one axis at the
  touch-slop crossing, and `ScrollView(..., horizontal=True)` is the front door. Two
  findings the entry above could not have predicted: `overflow-auto` parses to *both*
  axes, so every `ScrollView` and `List` in existence already declared itself
  horizontally scrollable — the ancestor walk therefore has to filter on **room**, not
  on the declaration, or every horizontal drag lands on a vertical list and moves
  nothing. And a padded scroll container's **end padding was never scrollable on either
  axis**, a pre-existing `max_scroll_y` bug that only became visible once a fling could
  reach a horizontal bound. `Carousel`/`PageView` are **unblocked, not built**: they
  need snap-to-page, which is a different animation from the fling.
- **`dark:` never activates natively.** The variant parses to a state bit that nothing
  ever sets.
- ~~**`opacity-*` and `translate-*` do nothing natively.**~~ Added and closed
  2026-08-13 (`phase_3/PLAN.md` §45.1). The cleanest specimen of this whole class:
  both compiled to real `ComputedStyle` fields (`paint.opacity`,
  `ComputedStyle::transform`) since phase 0, the WebView backend painted both
  correctly from the real Tailwind browser build, and **no conformance fixture covered
  either property** — so §5's harness was structurally unable to see it and
  `WIDGETS.md` listed `opacity` under "works on both" for three phases. Measured
  against real Chromium at **42.18%** of pixels wrong on an opacity fixture, **78.17%**
  on an overlapping-children one and **15.19%** on a translate one; **0.01% / 0.00% /
  0.10%** after, with all 35 pre-existing fixtures byte-identical across the pair.
  Verified live on a Moto G54, where the composite is arithmetically exact and a
  translated button is *tapped where it is drawn* rather than where it was laid out.
  **`rotate-*` and `scale-*` remain in this state**, deliberately: a translate reuses
  the per-node offset channel scrolling already had, while those two need a real affine
  and an invertible hit test.
- **Gradients do nothing.** `twcore` has no gradient parsing at all — `bg-gradient-*`,
  `from-*`, `via-*`, `to-*` are silently dropped — and the paint path only fills solid
  colours, so it needs a gradient brush too, not just a parser.
- **Emoji don't render.** The bundled faces are DejaVu, which has no emoji coverage, and
  there is no colour-glyph (`COLR`/`CBDT`) support in the paint path — so bundling an
  emoji font would not be enough on its own. This bears on §4's unsettled "which
  typefaces do we bundle" question: reading the device's own `/system/fonts` would solve
  emoji and cross-OEM consistency together, at the cost of the host conformance harness.
- ~~**`change` and a bubbled `tap` fire in opposite orders on the two backends.**~~
  Added 2026-08-08 and **closed 2026-08-09** (`phase_3/PLAN.md` §32.3). It was *measured*
  rather than inferred (§27.6): a tap on a `Switch` inside a row with `on_tap` ran the
  row's handler first on WebView (the runtime listens in the capture phase) and the
  switch's own handler first on native. Both always fired; only the order differed, and
  it only bit when both wrote the same `@store` field — which §30.8 then showed was
  reachable from an ordinary settings screen, netting the opposite result per backend
  from identical code. **The owner's contract decision: the bubbled `tap` runs first and
  native matches the WebView.** Native moved; the WebView is unchanged. Re-verified on a
  Moto G54 against a rebuilt pre-fix control, same fixture and gesture. The shape that
  provoked it is also gone — a labelled control (§32) absorbs the tap, so there is no
  second handler to order.
- ~~**Text painted above every background, and `Stack` children not filling their
  stack.**~~ Added and closed 2026-08-09 (`phase_3/PLAN.md` §31.1, §31.2). The native
  paint path ran three whole-tree passes — every box, then every icon, then every glyph
  — which reproduces DOM stacking for a node against its own ancestors and **inverts it
  across sibling subtrees**, so nothing opaque could cover anything textual. It broke
  `Stack`, whose entire purpose is overlaying children, and it is why an opaque error
  overlay could not hide the screen behind it. Neither defect was visible to the
  conformance harness, because the `stack` fixture overlays two explicitly-sized opaque
  rectangles with **no text in either** — the one shape immune to both. Measured against
  real Chromium: 2.77% → 1.06% and 55.19% → 2.08% on fixtures written to isolate one
  cause each, and re-measured on a Moto G54 (real PowerVR, density 2.5) where the
  width-less `Stack` child now spans **300.00 dp** exactly against a `w-[300px]` stack.
  **What remains open is named rather than latent:** the renderer has document order,
  and no stacking contexts, `z-index` or opacity groups.
- **The system font-size accessibility setting is ignored by *both* backends** — no
  `fontScale` handling anywhere. This is a Play quality-bar item, and it is the same shape
  as the density bug phase 3 fixed: `density` was read from `DisplayMetrics` and
  `fontScale`, sitting beside it, was not.

### 14.3 Widgets and rendering

**Missing from §8's own v1 list:** `Sheet`, `Nav`, `Native(...)`.

**Missing from any list, and needed by an ordinary app:** ~~`Checkbox`, `Radio`,~~
`Select`/picker, `DatePicker`/`TimePicker`, `Dialog`, `Toast`/snackbar,
`ProgressIndicator`, `Carousel`/`PageView` (no longer blocked on horizontal scrolling
as of §27; still needs snap-to-page), pull-to-refresh, sticky/section headers.
Today the form vocabulary is `TextField`, `Switch`, `Slider` and nothing else.

> **Status (2026-08-08): `Checkbox` and `Radio` are built** — see `phase_3/PLAN.md`
> §30. The form vocabulary is **five** controls, on both backends, verified live on a
> Moto G54 with Android's own accessibility layer reporting real
> `android.widget.CheckBox`/`RadioButton` roles. Two findings worth carrying: the three
> checkbox-shaped controls are distinguished by an **ARIA `role` emitted on both
> paths**, not by a ZKite-private marker, which is what let `index.html`'s switch CSS —
> a global `input[type=checkbox]` selector that was correct until a second
> checkbox-shaped widget existed — be scoped rather than duplicated; and a payload bug
> in exactly §14.2's `change`-payload shape was caught before shipping, since a radio's
> DOM default value is `"on"` and `bool`-parsing that gives `False`, so selecting one
> would have reported it *deselected*. **`Select` is deliberately not built:** it needs
> a design decision (self-drawn popup vs. a real Android picker via the unbuilt
> `Native(...)`), written up with a recommendation in §30.6.

> **Status (2026-08-09): all three checkbox-shaped controls now take a label, and can
> drop their own box** — see `phase_3/PLAN.md` §32. `Checkbox("", "Wi-Fi", checked=...,
> on_change=...)` is one tap target with one handler, which is what closes the gap §30
> shipped with: the live area used to be a 20x20dp box and tapping the label did
> nothing. Verified on a Moto G54, where Android's own accessibility layer reports the
> control as `android.widget.CheckBox` **whose name is the label**.
>
> Two findings worth carrying. **HTML's `<label>` was the obvious implementation and was
> rejected on a measurement**: driven against real Chromium, label activation dispatches
> a *second* `isTrusted` click on the contained input, so an ancestor `on_tap` would
> have fired twice on the WebView and once on native — a new silent divergence of
> exactly the §14.2 kind, bought for a feature meant to remove one. Composing the
> control in Python out of pieces both backends already had (a tappable container, a
> presentational box, ordinary content) needed **no runtime change on either backend**.
> And **`indicator=False`** — added after the owner asked how a developer would build a
> multiple-choice question — drops the box entirely while keeping the role, state and
> accessible name, because the alternative developers reach for, a tappable `Row`, is
> announced to a screen reader as generic clickable text.
>
> **`Select`'s decision is now made:** the self-drawn popup, with `Sheet` a separate
> piece of work. Neither is built.

> **Status (2026-08-09): `Select` is built** — see `phase_3/PLAN.md` §33. The form
> vocabulary is **six** controls, verified live on a Moto G54 on **both** backends, with
> Android's own accessibility layer reporting the trigger as a real
> `android.widget.Spinner` named by the current option and the popup as an
> `android.widget.ListView`.
>
> **The finding is bigger than the widget: painting above the whole app needed no
> renderer change at all.** Both backends already agreed on the rule it rests on — a
> node emitted later in document order is painted later and hit-tested first — so a
> popup is an ordinary subtree that the app root emits *last*. `render_thread.rs`,
> `zkite-runtime.js` and `twcore` are untouched, and there is still no `z-index` or
> stacking context anywhere. The layer ships as a public `Overlay`, so `Sheet`,
> `Dialog` and `Toast` are now one composition each over something that exists.
>
> Two things worth carrying. **One word was settled by measurement, not by reading**:
> the panel carries `relative`, because CSS paints positioned elements above
> non-positioned ones regardless of tree order — without it the panel renders correctly
> on native and is buried under its own scrim on the WebView, exactly §14.2's class of
> bug. And **§26.2's payload shape bit a third time** during the first end-to-end run:
> a tap's payload is the empty string on both backends, so the chosen option arrived as
> `""`; the value now never crosses the bridge at all.
>
> The same session paid a debt three sessions old: the **WebView backend was finally run
> on a device**, for `Select` and for §32's labelled controls, matching the native
> results case for case.

> **Status (2026-08-08): that sentence was true as an inventory and false as a
> description of what worked.** On the *default* backend it was `TextField` alone —
> `Switch` and `Slider` rendered and ignored every touch, because the native render
> thread dispatched `"change"` from two sites and both were inside the IME path
> (`phase_3/PLAN.md` §15.1 divergence 6). Both now work (§26). The same work found a
> second, worse thing that this section could not have known about because it was not
> a native gap at all: **the `change` payload was a *string* on both backends**, and
> `bool("false")` is `True`, so the canonical `Switch(on_change=...)` on the WebView
> backend could be turned on and never off. Fixed by typing the payload at the widget,
> which both backends share.

**Not painted by the native renderer:** `Image` content (an empty box of the correct size
— no bitmap decode, no network loading or caching), borders, shadows, rings, text
decoration (underline/line-through), italic faces. `text-align` doesn't cover
content-sized wrapped text, and there is no overscroll or rubber-band at a scroll bound.

> **Status (2026-08-08): borders, shadows and rings now paint** — see
> `phase_3/PLAN.md` §28. All three, including per-side border widths and colours, on
> both native paint paths, with the geometry shared by all four paint consumers.
> Verified live on an **x86_64 emulator**; the Moto G54 run is still outstanding.
> `Image`, `decoration` and italic faces are unchanged and still open.
>
> **Status (2026-08-10): `Image` now paints too** — see `phase_3/PLAN.md` §35. PNG and
> JPEG, from the app directory or a `data:` URI, with all five `object-fit` modes and
> rounded clipping, on both native paint paths; verified live on the Moto G54 on the
> GPU path, the CPU fallback path, and as real `zkite build apk` apps on **both**
> backends. `decoration` and italic faces remain open, and so do network image loading
> and intrinsic sizing (§35.6).
>
> Two corrections it forced. `Image` was **not a native-only gap**: the WebView shell
> loads its page from `file:///android_asset/webruntime/`, so a relative `src` resolved
> to a nonexistent asset and images were broken on that backend too. And `zkite build`
> was dropping the image file *before* it ever reached `app.zip` — `copy_app_source`
> staged `*.py` and nothing else — which is not where `phase_3/PLAN.md` §18.6 had
> located the problem.
>
> **Two findings from it are worth more than the feature.** First, `twcore`'s border,
> shadow and ring *defaults* were all still Tailwind **v3's**: v4 defaults both
> `border-color` and `ring-color` to `currentColor` (v3: gray-200 and blue-500), drops
> the default ring width from 3px to 1px, renames the whole `shadow-*` scale (v4's
> `shadow-xs` is v3's `shadow-sm`) and makes most of it two-layer. §9.6 had already
> found and fixed the *palette* half of this class and its own known-gaps entry
> predicted the rest; nobody checked, because nothing painted any of it. §9.6 even
> touched the ring default and corrected only its hex value while the rule underneath
> it stayed v3's. Second, **the conformance harness cannot see whether a shadow is
> correct** — removing the CSS-blur-to-standard-deviation conversion changed the render
> and moved the reported number by nothing, because Tailwind's entire shadow scale is
> fainter than the harness's per-channel tolerance. That is §14.2's "silent divergence"
> idea applying to the *test tool*, and it was caught only because the negative control
> was run.

> **Status (2026-08-07): `Icon` now paints, and the framing above was hiding something.**
> All twelve bundled sprites render natively — see `phase_3/PLAN.md` §18. Two things
> came out of it worth knowing before quoting the rest of this paragraph. First, **the
> remaining items are one gap, not five**: the native paint path drew exactly two things
> (a solid rounded rect and glyph runs), and *both* vello backends already expose
> `fill_path`/`stroke_path`/`set_stroke`, `fill_blurred_rounded_rect`, and a `set_paint`
> that takes a gradient or image brush — so §14.2's "a gradient brush has to exist too"
> overstates that cost, and borders/shadows are additive against space taffy already
> reserves. Second, `Icon`'s size is now a real `1em` of the *inherited* font size rather
> than a hardcoded 20px box; the conformance fixture that was supposed to cover this used
> `text-[20px]`, so it agreed with the constant by coincidence and could never have
> caught it.

**No animation or transition support at all.** `twcore` parses no `transition-*` or
`animate-*`, and there is no animation driver beyond the scroll fling. §10 files this
under phase 4; a framework without a fade is a hard sell before then.

### 14.4 Platform integration

- **Autofill.** Phase 0 correctly predicted that Autofill and accessibility "reduce to the
  same missing piece" — a virtual semantics tree. Phase 3 built that tree for AccessKit;
  nothing consumes it for Autofill.
- **Accessibility scroll actions** (`ACTION_SCROLL_FORWARD`/`BACKWARD`) and in-field text
  traversal. A screen reader sees only a virtualised list's materialised window, so
  TalkBack's linear navigation stops at the window edge until these are wired.
- ~~**Nothing can repaint on its own.**~~ Added 2026-08-09, **built the same day on both
  backends** (`phase_3/PLAN.md` §34, §15.1). `App.on_render`/`on_render_tree` had
  existed since phase 1 and nothing had ever registered either; `runtime.py` now does,
  against a Rust-injected hook, so a `@store.update()` from a background thread produces
  a frame. Native sends it over the `mpsc` channel §15.2 already had. **The WebView
  shell makes a genuine cross-thread JNI call** rather than the 120ms poll phase 1 §4
  avoided, so a shipped app stays idle between frames — that half was the real work, and
  the owner's decision was that it had to exist rather than leaving an app that loads
  remote data working on one backend and blank on the other.

  **And a ZKite app has now made a network request** — `urllib.request.urlopen` over real
  TLS, from a worker thread, returning `ok 200` on a Moto G54. Two things blocked it and
  only one was predictable. `zkite build` emitted a manifest with **no `<uses-permission>`
  and no way to add one**, so a shipped app could not reach the network at all
  (`permissions = ["INTERNET"]` now exists). And the vendored OpenSSL's compiled-in
  `/usr/local/ssl/cert.pem` does not exist on a phone — but the obvious fix, pointing
  `SSL_CERT_DIR` at Android's own CA directory, **also fails**, because Android's
  filenames use the old MD5 `X509_NAME_hash_old` scheme a modern OpenSSL's `by_dir`
  cannot resolve. Proved on the host with the device's own 145 certificates and identical
  bytes in three arms; ZKite now concatenates them into a cached bundle instead.

  **Both backends are verified end to end on a Moto G54**, as the harness and as real
  `zkite build apk` apps, with a discriminating negative control: with the push path
  disabled the screen does not change at all for 5s, and one tap then reveals 3008 pixels
  of accumulated state — so the app was alive the whole time and the freeze was purely a
  painting gap.

  **The run also found two bugs in `zkite build apk` rather than in the push path itself**
  (§34.8): the stdlib pruner dropped `stringprep`, so no built app could resolve a
  hostname (`LookupError: unknown encoding: idna`) while the harness — which stages the
  stdlib differently — worked fine; and `--backend native` silently staged a prebuilt
  shell left behind by a control experiment, which no `strings` check could distinguish.
  Every device run in this document before now exercised a stdlib and a shell that the
  shipped path does not produce.
- **App lifecycle and state restoration after process death.** Android routinely kills
  backgrounded apps; nothing saves or restores `@store` state, so the app restarts from
  scratch. Affects both backends. Not named anywhere in this document until now.
- **RTL and i18n/l10n**, reduced-motion, deep links, permissions, and the phase-4 plugin
  set (camera, location, notifications, file picker, share).

### 14.5 Toolchain and process

- **No CI for any phase-3 crate**, and no per-fixture pass/fail gate on the conformance
  harness — both dprs are numbers a human reads. §9's own "ship a device compatibility
  matrix from real CI-on-device runs" is still not done, and phase 0 flagged that the
  emulator cannot supply arm64 on an x86_64 host.
- **The keyboard matrix beyond Gboard is untested** (§9's IME risk). Samsung Keyboard and
  MIUI are OEM-exclusive and not installable on the test device.
- ~~**`zkite dev` does not auto-install a Dev Shell**~~ — **§6's "install a generic Dev
  Shell once" now happens automatically** (`phase_3/PLAN.md` §24): `zkite dev` probes the
  device, launches an installed shell, installs a prebuilt one when it finds one, and
  fails with an actionable message otherwise. Two dead ends were closed, not one — the
  *installed but backgrounded* case hung just as silently as the not-installed one,
  because nothing but the `--devshell-apk` path ever ran `am start`. **The distribution
  half is still open:** nothing downloads a Dev Shell, so the auto-install only finds an
  APK inside this checkout.
- **`pip install zkite` does not exist.** Neither `zkite_py` nor the CLI binary is published
  as a wheel, so §0's opening `pip install zkite` is still aspirational — which makes §10's
  phase-1 exit ("a stranger goes from `pip install` to a hot-reloading counter app…
  unassisted") not yet reachable by a stranger.

  > **Researched 2026-08-08 (`phase_3/PLAN.md` §23), and it is smaller than this
  > section and §21.4 both assume.** Two measured corrections. The build payload is
  > **~19 MB compressed**, not the ~250 MB §21.4 estimated — that figure was `du` over
  > a checkout containing CPython's `test/` suite, C headers, `node_modules` and
  > byte-identical duplicate `.so`s, none of which ever reaches an APK. And the payload
  > is **host-independent** (it is all Android-target bytes), while the only
  > host-specific artefact is the `zkite` binary at **5.7 MB stripped** — so the shape is
  > one `py3-none-any` data wheel plus five small platform wheels, not five huge ones.
  > The Windows-specific surface reduces to **one function** (`which()` looks for a bare
  > `python3.14`, and Windows has no versioned executable at all — it needs the `py`
  > launcher); zip separators are already handled at all four sites, file modes are
  > structurally absent from the APK writer, and the symlink and `MAX_PATH` hazards
  > disappear once the payload is pruned. **The binding constraint is not size, code, or
  > the OS — it is that no release pipeline builds the payload**, which is the same item
  > as §14.5's "no CI for any phase-3 crate" seen from another angle.
  >
  > **Update (2026-08-08, `phase_3/PLAN.md` §29): the binding *code* item is done.**
  > `zkite build` no longer runs on only this checkout. All five compile-time path anchors
  > collapse into one `payload::root()` — env override, `zkite-payload/` beside the binary,
  > the compile-time checkout **last** — verified by building a signed, installable APK
  > from an installed binary reading a payload outside this checkout. Proved with a
  > sentinel file that appears inside the artifact only on the payload path, rather than
  > by a build that merely succeeded; the resulting APK boots CPython, imports all 67
  > extension modules and responds to touch. The payload measures **21 MB compressed**,
  > corroborating the ~19 MB above rather than the ~250 MB it replaced. **What remains is
  > exactly what this entry already named: no release pipeline builds the payload, and no
  > wheel exists.** Nothing was run on Windows, macOS or a second physical machine, so
  > §23.5's OS-specific claims are still read rather than observed.
- **Host-native pyc compilers are vendored for Linux x86_64 only**; macOS and Windows
  users must supply a matching Python 3.14 themselves. **The `zkite` CLI itself is also
  a Linux x86_64 binary only** — portable by construction, just never cross-compiled.

  > **Researched 2026-08-07 (`phase_3/PLAN.md` §17.2), and the received framing was
  > wrong about which step causes this.** The `.pyc` step compiles *only the app
  > directory* — one file for the stock app — so it buys almost nothing, and the
  > device now caches bytecode itself anyway (§17.1). But dropping it would **not**
  > make `zkite build` portable, because **stdlib pruning also needs a host Python and,
  > unlike `.pyc`, needs a *matching* one**: run under 3.12 it silently drops
  > `annotationlib`, which is new in 3.14 and which merely importing `zkite` requires —
  > producing an app that cannot boot, with nothing visible at build time. Measured,
  > not assumed. The live options are `requires-python = ">=3.14"` on the wheel (free,
  > and the only one fully faithful to §2's principle 1) or downloading a
  > python-build-standalone interpreter on demand.
- **`version_code` defaults to 1 and never auto-increments.** ~~and is *identical across
  per-ABI APKs*~~ — **the per-ABI collision is fixed (`phase_3/PLAN.md` §29.3).** Each
  APK's code is now `base * 10 + <permanent per-ABI rank>`, the manifest is built inside
  the per-ABI loop, and a build-time assert catches Play's 2,100,000,000 ceiling —
  confirmed by `aapt2 dump badging` reading the real binary AXML and by Android's own
  package manager after install. The scheme is deliberately **not** Google's own
  `rank * 1000 + base` example, which is not monotonic in the base (arm64 at base 2 is
  *lower* than x86_64 at base 1), so "I bumped my version" would mean something
  different per ABI. **The defaults half is untouched:** it is still `1`, still manual,
  and still not derived from `[project] version`. See `phase_3/PLAN.md` §17.4.
