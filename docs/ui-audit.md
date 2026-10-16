# UI and development dependency audit

Changes tested on a Moto G54 running Android 15, plus host regression suites.
The native backend was the focus. This is a record of specific fixes and coverage,
not certification of every widget, Android release, keyboard, or GPU.

## Fixed

- Clipped list rows could intercept taps outside their viewport, making unrelated
  controls appear frozen. Hit testing now respects inherited clipping.
- Shrinking a scrolled list could leave an empty viewport. Offsets now clamp after layout.
- Disabled controls now ignore interaction and have a default visual indication.
- Sliders commit the final pointer-up position.
- Skipped presentation is retried; startup waits for an actually presented frame.
- Startup asset extraction runs off the UI thread. The loading view remains until
  the first frame, then fades out; extraction failures show an error.
- Accessibility refreshes when enabled after startup.
- Text selection uses the field's padding, bundled font, baseline and rounded shape.
  The first long-press opens Android's selection toolbar and handles.
- UTF-16 cursor indices and Java modified UTF-8 are converted at the native boundary,
  preserving accented characters and emoji. Duplicate IME change callbacks are suppressed;
  Cut and typing in selection mode update Python immediately.
- Selection overlay position follows keyboard panning. Android 11+ no longer also
  pans the entire window. Removing a focused field clears native focus.
- Navigation validates destinations and arguments before mutation, isolates keys
  by stack entry, restores entry identity on Back, and injects the correct nested host.
  A 160 ms native entrance animation is optional through `transition=False`.
- `zkite dev` now shares the build dependency resolver, caches complete package sets,
  sends transitive packages, metadata and resources, and refreshes declarations live.
  Failed resolution preserves the running app; removed package files are reconciled.
  Source-only edits do not resolve or retransmit unchanged dependencies.

## Verification

- 219 Python widget/navigation tests; supporting Python suites also passed.
- 74 renderer tests and 47 bridge tests passed.
- 101 CLI tests passed; 3 optional tests skipped in the regular run.
- The optional real Requests resolution/import test passed separately, including
  transitive imports, distribution metadata, certificate data and cache reuse.
- `scripts/check_dev_dependencies.py` exercises the actual CLI with a simulated
  reload socket: initial packages, a one-file source edit, and live replacement
  of Requests with IDNA. This is a host transport/import check, not Android execution.
- Native libraries compiled for arm64-v8a and x86_64; the arm64 development shell
  was rebuilt with the runtime changes.
- Before the phone disconnected: tested controls, disabled states, list scrolling
  and shrinking, modal/Select Back ordering, selection toolbar, Cut and typing,
  Unicode input, bottom-field keyboard positioning, navigation push/Back, scroll
  restoration, hot reload, and three background/resume cycles.
- Standalone APK packaging/install succeeded earlier (7.93 seconds packaging in
  that run). A dual-ABI AAB passed packaging/alignment checks in 22.05 seconds.
  These observations do not establish a universal 5–10 second build guarantee.

## Reproduction and limits

Use `phase_3/gpu_harness/demo_app/ui_audit.py` and `navigation_audit.py` as test
project entry points. The visuals fixture expects a local `cyan16.png` image.
The navigation fixture includes a Unicode input near the bottom of the screen.

Declare packages in `requirements.txt`, `app.json` dependencies, or
`pyproject.toml` project dependencies. Host-installed packages are not copied
implicitly. Compatible pure-Python wheels are supported; arbitrary desktop binary
extensions and desktop OS APIs remain unsupported. No package was published to PyPI.

A final minimal Moto G54 check confirmed Requests, IDNA, distribution metadata and
the Certifi certificate resource import correctly through `zkite dev`. Older Android versions, alternative
keyboards, GPU-loss recovery and x86_64 runtime behavior need additional device coverage.

The selection toolbar uses Android TextView's public selection accessibility action
rather than a custom clipboard menu. See the [Android TextView implementation](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android15-release/core/java/android/widget/TextView.java).
