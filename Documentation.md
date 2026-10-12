# ZKite documentation

ZKite builds Android applications from Python. You describe a widget tree, update
Python state, and ZKite lays out and draws the interface using its Rust native
renderer. The default app does not use a WebView. Android supplies the window,
keyboard, accessibility integration, and application lifecycle.

This guide describes the implemented API. The historical design plan in `docs/history/` and the phase plans contain
historical proposals and experiments; they are not installation instructions.

## 1. Install

The complete distribution is `zkite`; it installs matching `zkite-framework` and
`zkite-payload` dependencies. The command and Python import are both `zkite`.
The current source targets `1.1.0`, following the published `1.0.0`. The new wheels
await manual publication. After upload, install with `python -m pip install zkite==1.1.0`.
Before upload, use the matching local wheels in `publish-dist/1.1.0/` as below,
or install the previous stable version with `python -m pip install zkite==1.0.0`.

For the extended learner knowledge base, see
[Documentation-detailed.md](Documentation-detailed.md). It covers public concepts,
API options, Windows, and troubleshooting without framework source listings.

You need:

- **CPython 3.14**, matching the Python runtime embedded in Android.
- **Android platform-tools (`adb`)** for installing, running, or hot reloading.
- An Android 8.0/API 26 or newer arm64 device, or an x86_64 emulator, with USB
  debugging enabled and the computer authorized.

A complete ZKite wheel set includes the CLI, Python API, Android runtime, and native
Dev Shells. App developers do not need Rust, Gradle, JDK, NDK, or Android Studio.
Those tools are used once to produce framework release artifacts.

### Windows PowerShell

Use the Windows **win_amd64** CLI wheel with the matching framework and payload
wheels. All three must have the same version and be in `publish-dist`:

```powershell
py -3.14 -m venv .venv
.\.venv\Scripts\python.exe -m pip install --no-index --find-links publish-dist/1.1.0 zkite==1.1.0
adb devices
.\.venv\Scripts\zkite.exe new hello
cd hello
..\.venv\Scripts\zkite.exe run
```

These commands work without activating PowerShell scripts. If you activate the
environment, use `zkite` directly. An explicit Python override is also supported:

```powershell
$env:ZKITE_PYTHON = 'C:\Path With Spaces\Python314\python.exe'
```

### Linux

Use the CLI wheel built for your OS and CPU:

```bash
python3.14 -m venv .venv
. .venv/bin/activate
python -m pip install --no-index --find-links publish-dist/1.1.0 zkite==1.1.0
adb devices
zkite new hello
cd hello
zkite run
```

The prepared Linux x86_64 wheel supports glibc 2.28 or newer and passed auditwheel
inspection. The local upload set includes Windows x86_64 and Linux x86_64; no macOS
wheel is included. The broader CI workflow contains a macOS Apple-silicon job,
which is separate from the locally verified release set.

## 2. Understand the development commands

| Command | What happens |
|---|---|
| `zkite new hello` | Creates `main.py`, `app.json`, `assets/`, editor configuration, and `.gitignore`. |
| `zkite dev` | Prepares a project preview with your icon and label, installs it, and reloads source changes. |
| `zkite run` | Builds a standalone APK, installs it, and launches it. No dev server is required. |
| `zkite run --logs` | Also follows the launched process's Android logs. |
| `zkite build apk` | Creates signed per-ABI APKs in `dist/`. |
| `zkite build aab` | Creates an Android App Bundle in `dist/`. |
| `zkite doctor` | Checks the payload, Python, dependency tooling, and connected device. |
| `zkite keygen --out zkite-release` | Creates a signing PEM and PKCS#12 keystore. |

New projects include a `.gitignore` for local `.zkite/` state, build output, signing
keys, Python environments and caches, local environment files, and OS/editor
scratch files. Source code, assets, project/editor configuration, lockfiles, and
`.env.example` remain trackable. Existing projects are not modified by `zkite new`.

`zkite dev` and `zkite run` automatically use a single authorized device. With
multiple authorized devices, an interactive terminal shows their model names and
serials: use **Up/Down arrow keys** to move, **Enter** to connect, or **Esc/Ctrl+C**
to cancel. Offline and unauthorized devices are excluded from the picker. The chosen
device stays selected for that command, including dev reconnects.

To skip the picker, choose a device explicitly:

```bash
zkite run --serial emulator-5554
zkite dev --serial DEVICE_SERIAL --port 9000
```

`ANDROID_SERIAL` is also supported; `--serial` takes precedence. Scripts and
redirected input must specify one of these when multiple devices are connected.
`--port` changes the host reload port; ZKite maps
the shell's fixed device port correctly. The server listens on loopback.

`zkite run` saves a private development key under `.zkite/debug.pem`. Repeated runs
update the same app without uninstalling it. Keep that directory to retain the
development signing identity. A failed signature match is reported; ZKite does not
silently uninstall your existing app.

## 3. Your first screen

Replace `main.py` with:

```python
from zkite import Button, Column, Text, store


@store
class Counter:
    count: int = 0


def view(counter: Counter):
    return Column(
        "flex-1 items-center justify-center gap-6 bg-slate-950 p-6",
        Text("text-4xl font-bold text-white", f"Count: {counter.count}"),
        Button(
            "rounded-xl bg-indigo-600 px-6 py-3 text-white",
            "Add one",
            on_tap=lambda: counter.update(count=counter.count + 1),
        ),
    )
```

Run `zkite dev`, then edit and save the text or colors. The shell remains installed
while the source changes. Use `zkite run` to test the packaged application.

The conventions are consistent:

- The entry point is a function named `view`; no `App.run()` or boilerplate is needed.
- The **first positional argument is the class string**, including for `Text` and
  `Button`. Content and child widgets follow it.
- Annotated `@store` parameters are injected into `view` and navigation screens.
- Call `state.update(...)` to change state and request a render. Direct attribute
  assignments do not provide the same notification contract.
- Keep `view()` fast and free of I/O. It runs again when state changes.

## 4. Layout and styling

Use `Column` for vertical layout, `Row` for horizontal layout, `Grid` for grids, and
`Stack` for overlapping children. Containers can be nested normally:

```python
from zkite import Column, Row, Text


def view():
    return Column(
        "flex-1 gap-4 bg-white p-6",
        Text("text-2xl font-bold text-slate-900", "Profile"),
        Row("items-center justify-between",
            Text("text-slate-700", "Plan"),
            Text("font-bold text-indigo-600", "Starter")),
    )
```

The native engine implements a documented Tailwind-style subset, not a browser CSS
engine. Common examples include `p-4`, `gap-2`, `flex-1`, `w-full`, `rounded-xl`,
`bg-indigo-600`, `text-lg`, `font-bold`, borders, shadows, gradients, opacity, and
translation. Arbitrary values include `w-[137px]` and `bg-[#0f172a]`.

Lengths describe logical layout units; the renderer applies device density. Give
scroll containers bounded space such as `flex-1`. Unsupported CSS utilities do not
become supported simply because Tailwind's editor extension recognizes them.
See [widget reference](docs/widgets.md) for the detailed widget/style reference.

## 5. Forms and events

Values live in your store. Controls receive the current value and a callback:

```python
from zkite import Checkbox, Column, Select, Slider, Switch, Text, TextField, store


@store
class Settings:
    name: str = ""
    alerts: bool = True
    accepted: bool = False
    volume: float = 50.0
    country: str = "India"


def view(s: Settings):
    return Column(
        "flex-1 gap-4 bg-slate-950 p-6 text-white",
        TextField("rounded-lg bg-slate-800 px-4 py-3",
                  value=s.name, placeholder="Your name",
                  on_change=lambda value: s.update(name=value)),
        Switch("", "Notifications", checked=s.alerts,
               on_change=lambda value: s.update(alerts=value)),
        Checkbox("", "Accept terms", checked=s.accepted,
                 on_change=lambda value: s.update(accepted=value)),
        Slider("w-full", value=s.volume, min=0, max=100,
               on_change=lambda value: s.update(volume=value)),
        Select("rounded-lg bg-slate-800 p-3",
               options=["India", "Kenya", "Brazil"], value=s.country,
               on_change=lambda value: s.update(country=value)),
        Text("text-lg", f"Hello, {s.name or 'friend'}"),
    )
```

Checkbox/switch changes deliver booleans, slider changes deliver numbers, and text
fields deliver text. Handlers can omit an unused payload. Explicit `key="..."`
values help retain widget identity when the surrounding tree changes.

## 6. Scrolling, lists, and images

Wrap ordinary content in `ScrollView("flex-1", ...)`. For large collections, use a
virtualized list with a fixed row height:

```python
from zkite import Column, List, Row, Text


def view():
    return Column("flex-1 bg-white",
        Text("p-4 text-2xl", "Items"),
        List("flex-1", count=10_000, item_height=56,
             builder=lambda i: Row("items-center px-4",
                                   Text("text-slate-900", f"Item {i}"))))
```

`item_height` is a sizing contract: larger row contents are clipped. Variable-height
virtualized rows are not implemented. Horizontal scrolling is available through
`ScrollView(..., horizontal=True)`.

Place local PNG/JPEG files beside `main.py` or inside an asset subdirectory:

```python
Image("h-32 w-48 rounded-xl object-cover", src="assets/photo.jpg", alt="A landscape")
```

Import `Image` from `zkite`. Set dimensions explicitly. Local PNG/JPEG and supported
image data URIs render natively. Network image loading and intrinsic image sizing
are not implemented. Images exceeding 16,777,216 pixels are rejected before decoding
allocates their output buffer; resize large assets before shipping.

## 7. Navigation

Declare named screen functions and register them in a `Nav`:

```python
from zkite import Button, Column, Nav, Navigator, Text


def home(nav: Navigator):
    return Column("flex-1 gap-4 p-6",
        Text("text-2xl", "Home"),
        Button("rounded-xl bg-indigo-600 p-4 text-white", "Open details",
               on_tap=lambda: nav.push(details, item_id=7)))


def details(nav: Navigator, item_id: int):
    return Column("flex-1 gap-4 p-6",
        Text("text-xl", f"Item {item_id}"),
        Button("p-4", "Back", on_tap=lambda: nav.pop()))


def view():
    return Nav("flex-1 bg-white", home, details)
```

Android Back pops the navigation stack; at its root, the platform can exit the app.
`replace(...)` replaces the current screen and `pop_to_root()` returns home.
Navigation/store state survives compatible hot reloads, but automatic persistence
across process death is not implemented.

## 8. Async work and existing animations

Event handlers may be `async def`; ZKite runs awaitables on its background asyncio
loop. Update a store with results. Avoid blocking network/file work in `view()` or
synchronous handlers, and use timeouts for network requests. Android network access
requires `permissions = ["INTERNET"]` in the configuration below.

The existing native animation API supports opacity and X/Y translation:

```python
from zkite import Anim, Text

message = Text("text-xl", "Welcome", key="welcome",
               anim=Anim.enter(ms=200, opacity=(0, 1), translate_y=(12, 0)))
```

`Anim.change` and `Anim.exit_` are also available; see the animation section of
`docs/widgets.md` for completion callbacks and removal behavior. Rotation, scale, and
automatic screen transition policies are deferred. Animations currently target the
native backend; the optional WebView backend does not implement this driver.

## 9. Status bar and navigation bar

Wrap your native screen in `SystemBars` to control the area behind the clock,
battery indicator and navigation controls:

```python
from zkite import Column, SystemBars, Text


def view():
    return SystemBars(
        Column("flex-1 items-center justify-center bg-slate-950",
               Text("text-xl text-white", "My native app")),
        status_color="#020617",
        navigation_color="#020617",
        status_icons="light",
        navigation_icons="light",
    )
```

Colors accept `#RGB` or `#RRGGBB`. `navigation_color` defaults to `status_color`.
Icon choices are `"auto"` (default, chooses contrasting icons), `"light"`, or
`"dark"`. Settings can depend on a store, so changing your app's theme updates the
bars without restarting. The wrapper does not add layout space or change event IDs.
Newly scaffolded screens use dark bars matching their background.

The native shell paints behind transparent system bars, including Android's
[enforced edge-to-edge layout](https://developer.android.com/develop/ui/views/layout/edge-to-edge),
while keeping content inside safe-area margins. Android still controls the actual
clock, battery indicators and navigation buttons. This API currently styles the
native backend; on WebView it renders its child without changing system bars.

## 10. Configure and build your application

App settings live in `app.json`. No package manager or `pyproject.toml` is required:

```json
{
  "name": "hello",
  "version": "0.1.0",
  "backend": "native",
  "android": {
    "package": "com.yourcompany.hello",
    "label": "Hello",
    "version_code": 1,
    "min_sdk": 26,
    "target_sdk": 36,
    "permissions": ["INTERNET"],
    "icon": "assets/icon.png"
  }
}
```

Omit `icon` to use the bundled ZKite logo. Otherwise provide a real square PNG,
preferably at least 192×192, relative to `app.json`. The `assets/` directory is
packaged recursively; use `Image(..., src="assets/photo.png")` for its images.

`zkite new hello .` creates the scaffold in the current directory, refusing to
replace existing scaffold paths. Unrelated files, including `pyproject.toml`, stay
in place. Plain `zkite new hello` creates a new `hello/` directory.

Use a package identifier you control before publication and keep it unchanged
for subsequent releases. Increase `version_code` for each release independently
of `version`; the bundler derives distinct per-ABI APK codes from this base.

Settings are read during build or preview preparation. A script may generate
`app.json` for different builds; changing the file does not change an installed
app's launcher name or icon. JSON does not execute Python. Google Play requires
accurate, non-misleading names, icons and listings; update the listing when branding
changes. See [Google Play metadata policy](https://support.google.com/googleplay/android-developer/answer/9898842)
and [deceptive behavior policy](https://support.google.com/googleplay/android-developer/answer/17006354).

Declare third-party packages in `requirements.txt` (one requirement per line,
including version constraints; blank lines and comments are allowed), or use
`[project].dependencies` in an optional `pyproject.toml` managed by uv or another
tool. Pip options, includes and line continuations in requirements files are not
supported. Alternatively, `app.json` can contain a `dependencies` array; it replaces
the pyproject dependency list. Requirements are added to either list. Packages
merely installed on your computer are not automatically shipped.

Existing projects without `app.json` retain support for `[tool.zkite]` settings in
`pyproject.toml`. Once `app.json` exists, it supplies all app identity settings.

`Image(..., src="https://example.com/photo.png")` loads HTTPS PNG/JPEG images
asynchronously on native, with a 2 MiB download limit per image, four download
workers and a bounded memory cache. Set image dimensions and declare `INTERNET`
(the new template already does). TLS certificates are verified; redirects cannot
downgrade to HTTP. Failures leave an empty image and log a diagnostic; a later
render can retry after 30 seconds. WebView uses its browser image loader.

Standalone APK/AAB builds show a generic restart message for uncaught Python
render/startup/event errors, without exposing tracebacks on screen. Detailed errors
remain in developer logs, and `zkite dev` retains traceback overlays. This does not
prevent app bugs or Android crash dialogs: catch expected errors in your app and
provide an appropriate recovery flow.

`zkite dev` uses the same icon and adds ` (Dev)` to your label. It repackages the
prebuilt native shell without needing Gradle or an Android SDK. The preview has
a separate package and data directory, so it can coexist with the standalone app.
Its APK is cached under `.zkite/preview/`; changes to the icon contents, Android
configuration, signing key or runtime invalidate that cache. Restart `zkite dev`
after changing these settings; Python and UI edits still reload on save. Keep
`.zkite/debug.pem` to update existing installs. Starting another preview on the
same phone stops previous ZKite preview processes to prevent reload conflicts.
An explicit `--devshell-apk` retains the identity built into that custom APK.

Dependency resolution downloads a complete wheel graph for every selected Android
ABI. This release supports pure-Python packages. It fails explicitly if a dependency
is missing, requires native code, uses unsupported host-dependent environment
markers, or resolves to different contents across ABIs. Package metadata and package
data inside wheels are retained. There is no on-device pip installation step.

```bash
zkite keygen --out zkite-release --cn "My company"
zkite build apk --abi arm64-v8a --keystore zkite-release.pem
zkite build aab --keystore zkite-release.pem
```

Keep your release key securely and reuse it. Do not publish APKs signed with the
temporary key generated by `zkite build` when `--keystore` is omitted. The key used
by `zkite run` is a development key, not a release-key management system.

ZKite prunes unused stdlib modules by inspecting Python imports. Fully dynamic
imports cannot always be inferred. Use `zkite build apk --no-prune-stdlib` (or the
same option on `run`) when your app relies on those imports. Bytecode compilation
still uses CPython 3.14.

An APK update replaces managed app source, stdlib, framework, and font directories.
Do not store persistent user data alongside extracted app source. Store it in a
separate application-private directory; in the current Android layout the parent
of `Path(__file__).resolve().parent` for a top-level `main.py` is the app's private
files directory. Keep your database in a separate child such as `user-data/`.

## 11. Troubleshooting

| Symptom | Check |
|---|---|
| `zkite` is not found | Activate the venv or call its `Scripts/zkite.exe` / `bin/zkite` directly. |
| Python discovery fails | Install CPython 3.14; point `ZKITE_PYTHON` at the executable, not a shell command. |
| No authorized device | Run `adb devices`, enable USB debugging, and approve the computer on the phone. |
| Several devices | Select with Up/Down and Enter in a terminal; scripts can pass `--serial` or set `ANDROID_SERIAL`. |
| Missing Dev Shell | Install a complete payload or pass `zkite dev --devshell-apk PATH`. |
| Signature mismatch | Reuse the key that signed the installed app. Uninstall only if losing that app's data is acceptable. |
| Deleted files persist in dev | Replace an old Dev Shell; file reconciliation is capability-negotiated. |
| Missing stdlib import | Retry with `--no-prune-stdlib`, then report the import pattern. |
| Shell provenance mismatch | Framework contributors must rebuild the shell after runtime-source changes. |
| `doctor` fails with no device | Device checks require a connection; building APKs alone does not. |

Development errors appear in an on-screen traceback overlay and logs. Standalone builds show a generic error message while retaining diagnostic logs. Invalid native frame data such
as NaN/Infinity now produces a recoverable error instead of a blank screen. Fix the
source and save; do not remove the development signing key to solve a render error.

## 12. Framework contributors and release builders

App authors install wheels; contributors build the precompiled runtime. On Linux,
the release toolchain uses Rust, Python 3.14, JDK 17, `protoc`, Android NDK
27.3.13750724, platform 34, and build-tools 34.0.0. The Java sources use APIs
available in that compile platform; the generated application target SDK is 36.

```bash
python scripts/fetch_pack.py
phase_0/scripts/fetch_python.sh
phase_3/gpu_harness/scripts/fetch_androidx_core.sh
rustup target add aarch64-linux-android x86_64-linux-android
cargo build --release --locked --manifest-path phase_1/cli/Cargo.toml
ABI=arm64-v8a ZKITE_DEVSHELL=1 phase_3/gpu_harness/scripts/build_apk.sh
ABI=x86_64 ZKITE_DEVSHELL=1 phase_3/gpu_harness/scripts/build_apk.sh
python phase_1/cli/scripts/stage_payload.py release/payload --abi arm64-v8a --abi x86_64
python phase_1/cli/scripts/build_wheels.py --payload release/payload --out release/wheels --version 1.1.0 --verify
```

Put JDK 17's `bin` on `PATH` and set `JAVA_HOME` for those release steps. Staging
requires a new destination and never deletes an existing payload. Dev Shells are
included by default; `--without-devshell` creates a build-only distribution.

`build_wheels.py --verify` installs all three wheels in a fresh environment, imports
the API, verifies that the installed CLI finds its own payload, and builds an APK
outside the checkout. `--platform` labels an already-built binary; it does not
cross-compile it. Windows wheels must contain an actual Windows `zkite.exe`.

Run local gates with `python scripts/check_host.py --native`. The host workflow
runs Windows, Linux, and macOS tests. The manually triggered candidate workflow
builds both Android ABIs plus Windows, Linux, and Apple-silicon wheels. It uploads
artifacts without publishing packages. See [release status](docs/status.md)
for the measured release status and deferred features, and
[PUBLISHING.md](PUBLISHING.md) for the full TestPyPI and PyPI publishing procedure.

## 13. Migrate projects from the earlier Kite candidate

Version `0.1.0.dev2` completes the rename to ZKite. Upgrade all three distributions
together; do not mix the earlier Android payload with the renamed CLI or API.

| Earlier name | Current name |
|---|---|
| `kite new`, `kite dev`, `kite run`, etc. | `zkite new`, `zkite dev`, `zkite run`, etc. |
| `from kite import ...` / `import kite` | `from zkite import ...` / `import zkite` |
| `[tool.kite]`, `[tool.kite.android]` | `[tool.zkite]`, `[tool.zkite.android]` |
| `kite.tailwind.json` | `zkite.tailwind.json` |
| `.kite/` | `.zkite/` |
| `KITE_*` environment variables | `ZKITE_*` environment variables |

Update imports in every app module, rename the TOML tables and Tailwind config,
and update scripts/editor tasks that invoke the CLI. The CLI rejects old TOML
tables so an existing Android package identity cannot silently fall back to defaults.

Move the entire `.kite/` directory to `.zkite/` **before the first `zkite run`**,
preserving `debug.pem`. If both directories exist, preserve both and use
`zkite run --keystore PATH_TO_THE_ORIGINAL_DEBUG_PEM` until you resolve which key
signed the installed app. Update `.gitignore` to ignore `/.zkite/`; retain the
old ignore rule while any old state remains. Keep the existing Android `package`,
version settings, and release signing key to continue updating the same app.

The generic Dev Shell now uses `dev.zkite.gpuharness` (native) or
`dev.zkite.devshell` (WebView). `zkite dev` installs the renamed shell alongside
any older shell. App data in an older Dev Shell does not migrate automatically.
