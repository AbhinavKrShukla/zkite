# Changelog

## 1.2.1

- Native deep-link registrations in app.json, cold/warm delivery, URL construction and store-aware listeners.
- Persistent Device Lab audio (removed its two-second stop), transient audio-focus recovery and ducking.
- Native video playback with title-free configurable overlays, fullscreen/Back restoration, fit/bounds, seek, volume, speed and looping.
- Composable Python media controls and cancellable state watchers.
- Correct PyPI source link: https://github.com/AbhinavKrShukla/zkite.


## 1.2.0 — device services

- Async event handlers now execute on an owned event loop, surface errors, and cancel
  on reload. Store updates are atomic with rendering and reject stale async owners.
- Native Python APIs for file/photo picking, export, streaming uploads, system-camera
  photo/video capture, torch, microphone recording, playback, speech, haptics, sensors,
  sharing, clipboard, device state, foreground location, notifications, encrypted
  secrets, and biometric prompts.
- Build/run infer permissions from Python source and preserve them in app.json with
  ownership tracking, atomic updates, and an explanation report. Explicit capabilities
  remain available for dynamic or external code. Only obsolete inferred entries are removed.
- Signed APK/AAB manifests are decoded and checked for exact permission sets, proper
  placement, namespace/resource encoding, and duplicates. Hardware is optional.
- Added Device Lab, service/async/upload/scanner regressions and compiled-manifest tests.
- See [device services](docs/device-services.md) for API contracts and deferred features.

## 1.1.2 — prepared for publication

- Add background I/O tasks and swipeable, controlled tabs.
- Improve adaptive scrolling, text wrapping, line limits and ellipsis.
- Fix native color emoji, adaptive launcher icons and public entry-point errors.
- Introduce an orange-and-white interactive starter.
- Reuse signing keys from ignored `keystore/` by default.
- Resolve dependencies from pyproject/requirements using Android markers and per-package extras.


## 1.1.1

- Fix `zkite dev` dependency resolution, transitive package/resource transfer, caching, and live dependency updates/removal.
- Fix clipped controls intercepting taps, shrinking-list scroll positions, disabled controls, slider release values, and skipped-frame retries.
- Preserve TextField selection geometry and Unicode cursor positions; show the native toolbar on first long-press and suppress duplicate change callbacks.
- Harden navigation validation and screen identity; add optional native entrance transitions.
- Keep startup responsive while extracting assets and dismiss the splash only after a presented frame.
- Refresh accessibility when enabled after startup.

See [audit and coverage](docs/ui-audit.md). All three distributions use version 1.1.1.

## 1.1.0

Feature release after 1.0.0. Existing projects using `[tool.zkite]` in
`pyproject.toml` remain supported.

- New projects use `app.json` for app identity and Android settings. Package-manager
  metadata is optional; dependencies can come from `requirements.txt`, pyproject,
  or an explicit app.json dependencies list.
- `zkite new myapp .` creates a project in the current directory with overwrite checks.
- Templates include `assets/`; builds package its contents recursively.
- The default launcher icon is the ZKite logo.
- Native Image widgets load HTTPS PNG/JPEG URLs asynchronously with verified TLS,
  a 2 MiB download limit per image, and a bounded download cache.
- Standalone APK/AAB Python errors show a generic message instead of a traceback.
  Development overlays and developer logs retain diagnostics.
- Dependency resolution prefers pure-Python wheels, including optional-accelerator
  packages used by requests. Unsupported native-only dependencies fail explicitly.

Names and launcher icons are selected at build time. JSON does not execute code;
a script may generate it before building. Installed launcher resources require
an app update. New templates request INTERNET; remove it for offline-only apps.

## Validation and supported scope

The prepared set contains Linux x86_64 (glibc 2.28+) and Windows x86_64 CLI wheels,
plus matching framework and Android payload wheels. Both Android ABIs, arm64-v8a
and x86_64, are included. No macOS wheel is included in this local release set.

Host regression gates passed, including the native renderer and Python bridge.
Framework pytest: 218 tests and 66 subtests passed. Both host wheel installations
built an APK outside the checkout. Windows validation used Wine, not native
Windows. Linux compatibility passed auditwheel; all four wheels passed strict
Twine validation. On the Moto G54, an app built from the installed wheels fetched HTTPS data with
requests and displayed an internet PNG. APK signatures passed apksigner; the
arm64-v8a/x86_64 AAB passed Google bundletool validation.

Third-party native-extension wheels and HTTP cleartext images are unsupported.
These tests do not establish Google Play acceptance or coverage of every device.

Upload artifacts: `publish-dist/1.1.0/`. Validation logs: `release/validation-1.1.0/`.
No PyPI/TestPyPI upload was performed during preparation.
