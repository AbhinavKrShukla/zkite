# ZKite 1.1.2 — detailed learner documentation

This is the learner-facing knowledge base for ZKite 1.1.2. It describes the implemented native Android framework, its public API, everyday workflows, and practical debugging. It intentionally contains no framework source listings, private release infrastructure, signing material, or credentials. Commands are terminal instructions; API names and configuration keys are reference material.

This guide covers ZKite **1.1.2**. See [release notes](CHANGELOG.md) and [release status](docs/status.md) for changes and tested scope.

## Contents and suggested learning order

1. [What ZKite does](#what-zkite-does)
2. [Supported environments](#supported-environments)
3. [Installation](#installation)
4. [Connect an Android device](#connect-an-android-device)
5. [Create and run your first project](#create-and-run-your-first-project)
6. [Understand the project files](#understand-the-project-files)
7. [The development cycle](#the-development-cycle)
8. [How an interface is described](#how-an-interface-is-described)
9. [State and events](#state-and-events)
10. [Layout and styling](#layout-and-styling)
11. [Widget reference](#widget-reference)
12. [Forms and text input](#forms-and-text-input)
13. [Images and icons](#images-and-icons)
14. [System status and navigation bars](#system-status-and-navigation-bars)
15. [Swipeable tabs](#swipeable-tabs)
16. [Navigation between screens](#navigation-between-screens)
17. [Overlays and selection](#overlays-and-selection)
18. [Animation](#animation)
19. [Configuration reference](#configuration-reference)
20. [Dependencies and networking](#dependencies-and-networking)
21. [Data and application lifecycle](#data-and-application-lifecycle)
22. [Command reference](#command-reference)
23. [Build, sign, and distribute an Android app](#build-sign-and-distribute-an-android-app)
24. [Windows guide](#windows-guide)
25. [Troubleshooting](#troubleshooting)
26. [Performance and accessibility](#performance-and-accessibility)
27. [Learning exercises](#learning-exercises)
28. [Release checklist](#release-checklist)
29. [Frequently asked questions](#frequently-asked-questions)
30. [Glossary](#glossary)
31. [Guidance for the documentation website](#guidance-for-the-documentation-website)

## What ZKite does

ZKite lets you build Android applications using Python. You describe a screen using widgets, keep changing values in stores, and connect user actions to state updates. ZKite lays out and draws the resulting interface with its native renderer. Android provides the application window, keyboard, system bars, and lifecycle.

The standard experience uses native rendering. It does not require a browser, HTML document, JavaScript application, or web server in the finished app. Its controls are framework-rendered controls integrated with Android services; do not assume each widget is a stock Android View with every stock property. Styling resembles a supported subset of Tailwind utilities, but ZKite is not a browser CSS engine.

A development preview can reload Python changes while you work. A standalone application includes what it needs to run without that development connection. You can start with `zkite run`; running a development preview first is unnecessary.

ZKite suits native Android interfaces built from the documented layout, text, image, input, navigation, and state features. Features such as a camera API, comprehensive platform-service bindings, arbitrary compiled Python packages, and cross-platform iOS deployment are not part of this release's documented base. Historical plans describe ambitions and experiments; they are not promises of available features.

### Names you will encounter

| Name | Meaning |
|---|---|
| ZKite | The framework and product name. |
| `zkite` command | The terminal application used to create, preview, run, and build projects. |
| `zkite` Python import | The public namespace for widgets and state/navigation APIs. |
| `zkite` distribution | The host-specific command-line package installed through pip. |
| `zkite-framework` | The Python API distribution, installed as a matching dependency. |
| `zkite-payload` | The bundled Android runtime distribution, also installed as a matching dependency. |

Install the `zkite` distribution for the complete tool. Installing only the framework API does not install the complete build environment. Keep all three distributions at the same release version. The older names `kite` and `kitecli` are not the installation command for this product.

## Supported environments

| Item | ZKite 1.1 base |
|---|---|
| Host Python | CPython 3.14. Use the same minor version as the supplied Android runtime. |
| Linux host | x86_64, glibc 2.28 or newer, using the manylinux wheel. |
| Windows host | x86_64 Windows, using the `win_amd64` wheel. |
| Android minimum | Android 8.0 / API 26 by default. |
| Android target | API 36 by default; application behavior still needs device testing. |
| Android architectures | `arm64-v8a` and `x86_64`. |
| Device connection | Android platform-tools, including `adb`, with USB debugging authorized. |
| Rendering | Native by default. |
| App dependencies | Compatible pure-Python wheels; availability and transitive dependencies are checked. |

The prepared local release contains Linux and Windows host wheels. Do not advertise macOS installation from this release set: a macOS wheel is not included. Windows ARM, Linux ARM, 32-bit Android, iOS, and web deployment are outside this distribution's support scope. An Android minimum SDK is an installation floor, not evidence that every manufacturer and Android release has been tested.

You do not need Rust, Gradle, a JDK, the Android NDK, or Android Studio to build an ordinary app from the complete pip installation. Android Studio is optional if you want its emulator and device tools. An emulator needs a compatible Android image; the supplied emulator ABI is x86_64.

Windows packaging and execution have been exercised with Windows Python under Wine. That is useful verification, but native Windows testing remains a separate qualification step. Test on the Windows versions and device drivers you intend to support.

## Installation

### Choose an isolated Python environment

A virtual environment keeps ZKite and its Python packages separate from other projects. Use CPython 3.14 deliberately rather than relying on whichever interpreter a generic `python` command happens to find. PyPy is a Python implementation, not the package index; ZKite's supplied runtime expects CPython.

To inspect your interpreter, use `python --version` or the explicit interpreter command for your platform. To inspect the installed release, use `zkite --version`. To examine all three package versions, use `python -m pip show zkite zkite-framework zkite-payload` inside the environment where ZKite is installed.

### Linux installation

Run these commands in order:

1. `python3.14 -m venv .venv`
2. `. .venv/bin/activate`
3. `python -m pip install zkite==1.1.2`
4. `zkite --version`
5. `adb version`

Install Android platform-tools separately if `adb` is unavailable, and add its directory to your PATH. Open a new terminal after changing PATH. The Linux wheel requires glibc; a musl-only environment does not become compatible by renaming the wheel.

### Windows installation

In PowerShell, run:

1. `py -3.14 -m venv .venv`
2. `.\.venv\Scripts\python.exe -m pip install zkite==1.1.2`
3. `.\.venv\Scripts\zkite.exe --version`
4. `adb version`

These commands do not require activating a PowerShell script. If your organization restricts script execution, continue using the explicit executable paths. After moving into a project one directory below the environment, the corresponding CLI path is `..\.venv\Scripts\zkite.exe`.

### Install the current release

Install ZKite with `python -m pip install zkite==1.1.2`. Pip installs the matching
framework and Android runtime packages automatically. An Android APK is not a
pip-installable file.

### Verify installation

`zkite doctor` checks the tools, selected Python, bundled runtime, and device connection. When run inside an app project, it can also check project-related requirements. Read each reported item rather than treating every failure as the same issue. An absent phone can make device checks fail even though offline APK building works.

`python -m pip check` detects inconsistent installed dependencies. A successful pip installation does not prove that USB debugging is authorized or that a particular application dependency works on Android.

## Connect an Android device

Enable Developer options on the phone, enable USB debugging, connect a data-capable USB cable, unlock the screen, and accept the authorization prompt for your computer. Exact settings labels depend on the device manufacturer. Keep the screen unlocked for the first visual test.

Run `adb devices -l` and read the state:

| State | Meaning and next step |
|---|---|
| `device` | Connected and authorized; available for selection. |
| `unauthorized` | Unlock the phone and accept the debugging authorization prompt. |
| `offline` | The transport is not ready; check the cable, reconnect, and inspect the device again. |
| No entry | Check the cable, USB mode, host permissions or Windows driver, and platform-tools installation. |

With one authorized device, `zkite run` and `zkite dev` select it automatically. With multiple authorized devices in an interactive terminal, a menu shows model names and serials. Use **Up/Down** to move, **Enter** to connect, and **Esc** or **Ctrl+C** to cancel. You do not answer with a number or yes/no.

Choose explicitly with `zkite run --serial DEVICE_SERIAL` or `zkite dev --serial DEVICE_SERIAL`. The `ANDROID_SERIAL` environment variable also selects a device; an explicit `--serial` takes precedence. Automated or redirected terminal sessions should pass a serial because they cannot operate an interactive picker. The selection stays fixed for that command, including development reconnects.

ZKite supports selecting connected emulators through the same mechanism. `--adb PATH` selects a particular adb executable if several copies are installed. Use a current Android platform-tools installation and avoid mixing different adb executables while diagnosing connection issues.

## Create and run your first project

Run `zkite new myapp`, then `cd myapp`, then `zkite run`. The generated sample is already an application. The command builds an APK for the selected device architecture, installs it, and starts it. There is no prerequisite development build.

Wait for the command to finish and inspect the phone. The app should display the starter interface and respond to its controls. Return to the launcher and start it again to verify that it works independently of the terminal. Use `zkite run --logs` when you also want to follow the launched app's Android logs.

The generated app uses an orange-and-white Discover/Playground design, swipeable tabs, and a save interaction. Creating or upgrading the framework does not replace existing app source.

For iterative development, run `zkite dev` from the same project directory. Leave that process running while editing supported Python files. The preview reloads changes on the selected device. Stop the command with Ctrl+C when finished.

Project names can contain letters, digits, underscores, and hyphens, but cannot start with a digit. Use a simple name such as `myapp` for learning. `zkite new myapp` refuses to overwrite an existing directory. `zkite new myapp .` uses the current directory and refuses existing scaffold paths before writing files; unrelated files are retained. It does not convert an arbitrary existing Python program into an Android app.

## Understand the project files

| File or directory | Purpose | Normal handling |
|---|---|---|
| `main.py` | Application entry module containing the top-level `view`. | Edit and commit. |
| `app.json` | App identity, version, backend, and Android settings. | Edit and commit. |
| `requirements.txt` / `pyproject.toml` | Optional dependency declarations for pip / uv. | Edit and commit as needed. |
| `zkite.tailwind.json` | Styling/editor configuration created by the scaffold. | Keep with the project. |
| `.gitignore` | Excludes generated and private local files. | Commit; adjust for your application if necessary. |
| `keystore/` | Default release PEM and PKCS#12 signing files. | Back up securely; do not commit. |
| `.zkite/` | Local development state, preview cache, and development signing material. | Keep locally; do not commit. |
| `dist/` | Built application artifacts. | Treat as build output. |
| Top-level helper `.py` files | Supported application modules beside `main.py`. | Edit and commit. |
| `assets/` | Recursively packaged app resources; created by the scaffold. | Commit intended app resources. |
| Top-level PNG/JPG/JPEG files | Supported packaged image assets. | Commit assets intended for the app. |

### Important packaging boundary

Top-level Python modules and PNG/JPG/JPEG files are packaged. The complete `assets/` directory is now included recursively, so `assets/images/logo.png` works as an Image source. Other application package directories are not automatically staged. Third-party wheel packages are resolved separately and preserve their package structure and data.

Put intended JSON, font or other resource files under `assets/`; arbitrary files elsewhere in the project are not automatically included. Symbolic links inside assets are rejected. A launcher icon can use a project-relative path independently of app resource staging. Never put credentials or signing keys in assets.

### Git and private files

The generated ignore file covers local build output, environments, Python caches, signing keys, local environment files, and common operating-system scratch files. Source, configuration, intended assets, lockfiles, and `.env.example` remain trackable. Git ignore rules do not remove files already committed. Review your repository before sharing it.

The `.zkite/debug.pem` development key supports repeat installs with the same signing identity. Deleting it can cause Android to reject an update to an existing debug installation. Back up keys when they matter, but do not publish them. Never put server credentials or private signing material in application assets.

## The development cycle

### Preview versus standalone run

| Workflow | What it is for | Requires a running development command? |
|---|---|---|
| `zkite dev` | Fast source-edit feedback on a device. | Yes, for synchronization and reload. |
| `zkite run` | Build, install, and launch a standalone app. | No. |
| `zkite build apk` | Produce installable APK files. | No. |
| `zkite build aab` | Produce an Android App Bundle for a distribution workflow. | No. |

A normal preview uses the project's icon and a development label under a separate Android package identity. It has separate application data from the standalone app. Seeing two launcher entries is therefore expected. Check which one you opened before diagnosing an apparently missing change.

The default preview is prepared from the bundled native runtime. You do not need to build a generic development shell yourself. `--devshell-apk` is an advanced override for a supplied shell; that shell's embedded identity and compatibility may differ from the normal project preview.

### What reload does and does not mean

Supported Python edits trigger a new view. Compatible store fields can retain their values during reload; newly added fields take their defaults, while incompatible changes may reset state. This is a convenience during development, not a durable database.

Restart the development command after changing project identity, launcher icon, dependencies, permissions, or Android build settings. These affect the installed package and are not ordinary Python view updates. Replacing the bytes of the configured icon at the same path is recognized when the preview is prepared again.

If synchronization stops, inspect the command output and `adb devices -l`. The command can reconnect to its selected device, but cannot make an unauthorized connection authorized. A custom port is available through `zkite dev --port 9000`; use a free port and keep the same running command while editing.

## How an interface is described

The entry module provides a top-level function named `view`. It returns the current widget tree. A tree is a hierarchy: a screen can contain a column, the column can contain text and buttons, and a row inside the column can contain several controls.

The framework discovers this entry function. An ordinary app does not need to call `App.run()`. The exported `App` type is useful for specialized integration or testing, but it is not an extra step in the scaffolded workflow.

Widgets describe the interface for the current state. Keep the view quick and predictable. Do not fetch remote data, repeatedly write files, or perform slow calculations every time it is evaluated. Load or calculate data through an appropriate action, then update state and let the view reflect the result.

### Constructor conventions

Most widgets take a **class string first**, followed by their children or visible content. This includes `Text` and `Button`: their first positional string is styling, not the displayed label. A misplaced string can therefore look like missing text rather than a syntax error. Use the generated sample and public API reference to distinguish classes, content, and keyword properties.

`SystemBars` is a deliberate exception: its first argument is the screen widget it wraps. Other specialized widgets have required keyword properties, such as a virtual list's count, item height, and builder.

### Widget identity

A `key` identifies a widget across updates. Use stable keys when controls move, when list data changes order, or when animation/input state needs to remain associated with the same item. A key should represent the item's identity; an index becomes a poor identity when items are inserted or reordered. Do not create a new random key on every render.

Public exports include `Task`, `Tabs`, the layout and input widgets below, `SystemBars`, `store`, `Nav`, `Navigator`, `NavigationHost`, `Entry`, `Anim`, `AnimEnd`, `Widget`, and `App`. Internal module names are not the supported application API.

## State and events

A store is application state declared with `@store`. Its annotated fields describe the values the interface needs, such as a count, a selected choice, or text input. Give fields appropriate defaults. Annotated store parameters in a view are supplied by the framework.

Use the store's `update` method to change values and request an updated interface. Directly assigning a field or changing a list in place is not the documented notification mechanism. For a collection change, prepare the intended new value and update the relevant field so the framework observes the change.

An event handler connects a user action to application behavior. A button's `on_tap` can increment a count; a text field's `on_change` can update a string; a slider's `on_change` receives a numeric value. Pass a callable handler, not the result of executing the action during view construction.

### Controlled values

A controlled widget displays a value held by the application. For example, the value of a text field comes from a store, and its change callback writes the new text back to that store. If the callback does not update the value, a later render can restore the old value. The same idea applies to switches, checkboxes, radios, sliders, and selections.

Separate transient interface state from durable records. A dialog-open flag is usually transient. A saved note may need persistent storage. Hot reload preserving a value does not mean that Android process termination will preserve it.

### Slow work and errors

Standalone APK/AAB builds show a generic restart message for uncaught Python startup, render and event errors. They do not display the traceback on screen. Developer logs retain diagnostic details, and `zkite dev` retains detailed error overlays. This does not prevent application bugs or Android crash dialogs; handle expected errors in your own UI.

Keep callbacks short enough for responsive input. Move blocking work away from the rendering path, and publish its result through state updates. Use the `Task` API below for blocking I/O. Async event callbacks are also supported, but synchronous work inside them still blocks their event loop.

Represent loading, empty, success, and error states in the UI. Prevent repeated actions while a request is already in progress where appropriate. Validate external data and user input before treating it as application state.

### Background I/O with Task

`view()` and ordinary event handlers run synchronously. Keep them short: doing an
HTTP request inside either blocks input and rendering until it returns. Use `Task`
for blocking I/O. Create tasks at module scope, start them from an event, and read
their reactive state in `view()`.

```python
import json
from urllib.request import urlopen
from zkite import Button, Column, Task, Text

request = Task()

def fetch():
    with urlopen("https://api.example.com/items", timeout=15) as response:
        return json.loads(response.read())

def view():
    return Column("flex-1 p-6 gap-4 bg-white",
        Button("bg-orange-600 text-white p-4 rounded-xl", "Refresh",
               on_tap=lambda: request.start(fetch), disabled=request.loading),
        Text("text-slate-700", "Loading…" if request.loading else str(request.data)),
        Text("text-red-700", str(request.error) if request.error else ""),
    )
```

Replace the example endpoint with your service. Include `"INTERNET"` in
`app.json` → `android.permissions` (new projects already do).

`Task.start(fn, *args, **kwargs)` returns immediately. Four workers run concurrently,
with a bounded admission limit of 32 running/queued jobs. Excess submissions raise
`RuntimeError`; disable or debounce repeated triggers in the UI. Return data from
workers instead of changing widgets or stores in worker code. Completion updates
`loading`, `data`, and `error` under the app's render lock and pushes a fresh frame.
The previous successful `data` remains available during refresh or failure.

Starting the same task again invalidates its previous result. `cancel()` and a
successful hot reload also discard outstanding results. Cancellation cannot stop
an already running Python function or undo a remote operation; network timeouts
are still required. Errors are exposed in `task.error`, and a new start clears it.
Do not call `Future.result()`, sleep, or run HTTP calls in the UI handler. This API
accepts synchronous callables, including `urllib` and synchronous HTTP clients.
For heavy CPU work, use a native implementation that releases the GIL or move the
computation to a service; worker threads are primarily an I/O solution.

## Layout and styling

ZKite uses utility-style class strings to describe appearance and layout. The familiar vocabulary helps keep a screen readable, but the supported native utilities are a subset of Tailwind. Browser CSS, arbitrary web plugins, DOM attributes, and a web stylesheet are not automatically supported.

### Choose the right container

| Container | Use it for | Main consideration |
|---|---|---|
| `Column` | Vertical groups of controls or sections. | Give growing content a bounded available space. |
| `Row` | Horizontal groups, action bars, label/value pairs. | Consider wrapping or available width on small screens. |
| `Grid` | Repeated items in explicit columns. | Define a suitable column count and gap. |
| `Stack` | Overlapping elements. | Later children appear above earlier children. |
| `ScrollView` | A finite collection that can exceed the viewport. | Bound the viewport; otherwise it may simply grow. |
| `List` | Many fixed-height repeated rows. | Declare an accurate row height and stable identity. |

Utilities cover common spacing, width/height, flex layout, colors, text sizes, borders, rounded corners, shadows, opacity, and supported transforms. Use a small set consistently before composing a complicated design. A valid-looking web class is not evidence that the native renderer supports it.

Spacing and layout use the framework's density-aware coordinate system. Do not assume that a layout value is a physical screenshot pixel on every phone. Test long text, narrow devices, landscape where applicable, and the visible area when the keyboard opens.

### Scroll sizing

A scrolling container needs a constrained viewport. In a column screen, a growing content region commonly uses `flex-1`; `min-h-0` can allow that region to shrink within the available height. If a view expands to fit every child, there may be nothing left to scroll inside its own bounds.

Use `horizontal=True` for a horizontal `ScrollView`. Place ordinary finite content inside a scroll view; choose `List` for a large dataset with predictable row height. Avoid several competing scroll regions until the simple layout behaves correctly.

Scroll momentum follows a velocity- and density-aware spline sampled by elapsed
time. A held finger stops stale movement from launching inertia. This does not
remove the need to keep rendering and event callbacks fast.

### Text wrapping, line limits, and overflow

Text wraps within its available width by default, including long URLs and words.
Text retains its natural height in vertical lists; use `flex-1` when a text item
should share constrained space in a row.
Use these explicit controls on both Android and WebView:

```python
Text("flex-1", title, multiline=False, overflow="ellipsis")
Text("w-full", summary, max_lines=3, overflow="ellipsis")
Text("w-full", description, max_lines=2, overflow="clip")
Text("", marquee, wrap=False, overflow="visible")
```

`overflow` accepts `clip` (default), `ellipsis`, or `visible`. `max_lines` must be
a positive integer. `multiline=False` selects one unwrapped line; `wrap=False`
also disables wrapping. Native truncation respects Unicode grapheme boundaries.
Ellipsis needs a constrained width, supplied by the parent or a width class.
Avoid multiple independently styled child widgets inside one truncated Text;
apply line limits to each text paragraph separately.

### Text and theme consistency

Choose a small typography hierarchy and a clear spacing scale. Reuse class strings or project styling conventions rather than making every screen subtly different. Test foreground/background contrast, including disabled and selected controls. Dark app content does not automatically set the Android system-bar colors; use `SystemBars` explicitly.

The generated styling configuration helps keep project/editor conventions together. Editor suggestions are assistance, not a guarantee that every suggested browser utility is available in the native backend.

## Widget reference

### Layout and display

| Widget | Main inputs | Behavior and learner guidance |
|---|---|---|
| `Column` | Classes and child widgets. | Vertical composition. |
| `Row` | Classes and child widgets. | Horizontal composition. |
| `Grid` | Classes and child widgets. | Grid composition using supported grid utilities. |
| `Stack` | Classes and child widgets. | Overlapping composition, later children on top. |
| `ScrollView` | Classes, children, `horizontal`. | Scrolls content within a bounded region. |
| `Text` | Classes, content; `wrap`, `multiline`, `max_lines`, `overflow`. | Wraps and clips by default; supports ellipsis and explicit visible overflow. |
| `Tabs` | Pages, `labels`, `index`, `on_change`, stable `key`. | Controlled selection through header taps or horizontal swipes. |
| `Icon` | Classes, `name`, optional key. | A named built-in icon; not a general SVG loader. |
| `Image` | Classes, `src`, `alt`. | Supported local or data-URI image content with explicit layout sizing. |
| `List` | Classes, `count`, `item_height`, `builder`, `overscan`, key. | Builds the visible part of a fixed-height list. |

### Interactive controls

| Widget | Principal value properties | Principal callback | Value supplied to the callback |
|---|---|---|---|
| `Button` | Visible content, optional `disabled`. | `on_tap` | An action; a no-argument handler is suitable. |
| `TextField` | `value`, `placeholder`, `type`, `secure`. | `on_change`, `on_input` | Current text. |
| `Switch` | `checked`. | `on_change` | Boolean checked state. |
| `Checkbox` | `checked`. | `on_change` | Boolean checked state. |
| `Radio` | `checked`. | `on_change` | Boolean checked state; the app manages the selected choice. |
| `Slider` | `value`, `min`, `max`, `step`. | `on_change` | Numeric value. |
| `Select` | `options`, `value`, placeholder and panel settings. | `on_change` | The chosen option's original value. |

### Virtual lists

`List` is for a count of equal-height rows. Its `builder` receives an item index and returns the row widget. `item_height` must match the row's intended height, including the layout you expect to occupy that slot. `overscan`, defaulting to four, builds some nearby rows outside the visible area to reduce abrupt changes during scrolling.

Do not treat this as automatic variable-height virtualization. If content length changes each row's height unpredictably, choose a finite scroll layout or redesign rows around a consistent size. Incorrect heights can produce clipping or misleading scroll positions. Keep row builders quick; do not perform network requests for every row construction.

## Forms and text input

A form combines controlled fields, validation state, and explicit submission behavior. Keep each field's value in a store, use change callbacks to update it, and show validation messages near the relevant field. A keyboard type is an input hint, not validation.

`TextField` supports input types including text, email, URL, telephone, and number. Use `secure` for password-style masking. Masking changes what is displayed; it does not encrypt stored values, protect network traffic, or make an embedded credential safe.

On the native backend, text-change callbacks follow editing. `on_input` is useful where you intend per-edit updates; do not assume an optional web backend's change/commit timing is identical. Preserve the field's key and position where possible so ordinary updates do not unnecessarily replace its identity.

Test the keyboard and text selection on a physical device. Include deleting all text, inserting in the middle, selecting a range, pasting, entering Unicode, dismissing the keyboard, and returning from another app. Avoid rebuilding the entire field with a new key after each keystroke.

For radios, store one selected choice and derive each radio's checked state from that choice. Radio controls do not create a durable application-level group just because they are visually adjacent. A checked radio remains selected until the application's choice changes.

For a slider, choose a meaningful minimum, maximum, and step. Validate values at the application boundary as well as in the UI, especially when loading previously stored or remote data.

## Images and icons

There are three separate concepts: an app's launcher icon, an `Icon` widget, and an `Image` widget. Changing one does not automatically change the others.

### Launcher icon

Set `android.icon` in `app.json`, for example `"icon": "assets/icon.png"`. Omit it to use the ZKite logo. JSON does not support comments. Use a real square PNG, preferably at least 192 pixels on each side, and a project-relative path for portability. Renaming a JPEG file to `.png` does not convert it.

After changing the configured file or replacing its contents, rerun `zkite run` for the standalone app. For a preview, stop and restart `zkite dev` so it prepares and installs the updated preview. Ordinary source hot reload does not replace launcher resources in an already installed APK.

If an old icon remains, first confirm that you are inspecting the correct standalone or development app, that the configuration is active, and that the install succeeded. Android launchers may cache icons; the app's Android App info page helps distinguish a launcher cache from a packaging problem. Do not uninstall as the first troubleshooting step because that can erase application data.

Builds and previews wrap the PNG as a full-bleed adaptive icon. Supply opaque
artwork without baked-in padding. Android applies its launcher mask; outer edges
may be cropped, and transparent source margins remain transparent.

### Emoji in text

Use Unicode emoji directly in `Text`. Native rendering uses Android's installed
Noto emoji fonts, including color bitmap and flag fallbacks. Sequence coverage
depends on the device font version. Install the updated native runtime to pick up
renderer fixes; Python hot reload alone does not update native code.

### Images inside the screen

The native `Image` supports PNG and JPEG-family images and supported data URIs. For packaged project assets, put the file in `assets/` or beside `main.py` and use its relative path. Set a meaningful size in the layout. A missing or unconstrained size can look like an image-loading failure.

The default fit behavior is cover. Supported fit choices include contain, fill, none, and scale-down. Cover can crop edges; contain can leave unused space. Choose based on whether preserving the whole image or filling a frame matters more. Rounded clipping can shape the displayed frame.

HTTPS PNG/JPEG URLs load asynchronously on native with verified TLS, a 2 MiB per-image limit and a bounded cache; declare the INTERNET permission. Downloads use four workers, a 15-second network timeout and a bounded cache. Failures leave an empty image and a log diagnostic; a later render can retry after 30 seconds. HTTPS redirects cannot downgrade to HTTP. HTTP cleartext, WebP/GIF/AVIF and full browser `object-position` behavior are not supported by the native image loader. Images consume decoded memory as well as disk space; large source dimensions can be expensive even when displayed small.

### Built-in icons

`Icon` chooses a named symbol from the bundled set. Its size and color follow the supported text/icon styling conventions. It does not accept arbitrary SVG source as a substitute for an icon name. Give icon-only actions a clear accessible label where the available widget semantics allow it, and prefer a visible text label when ambiguity would hurt usability.

## System status and navigation bars

The top area containing time, battery, and connection indicators is the Android status bar. The bottom gesture area or navigation-button area is the Android navigation bar. Use the public `SystemBars` wrapper to control their background colors and icon contrast with the screen.

| Setting | Accepted value | Default or meaning |
|---|---|---|
| Child | One screen widget. | Required; the wrapper does not add layout space. |
| `status_color` | `#RGB` or `#RRGGBB`. | White if omitted. |
| `navigation_color` | `#RGB` or `#RRGGBB`. | Uses the status color when omitted. |
| `status_icons` | `auto`, `light`, `dark`. | Automatic contrasting icon color. |
| `navigation_icons` | `auto`, `light`, `dark`. | Automatic contrasting icon color. |

`light` and `dark` describe the icons, not the background. Light icons suit a dark background; dark icons suit a light background. Named colors, alpha colors, and arbitrary CSS expressions are not accepted by these color parameters.

Wrap the root screen in `SystemBars`, set the desired backgrounds, and use automatic contrast unless the design requires a tested explicit choice. Values can follow store state, so a theme switch can update the bars along with the app. The status and navigation colors can differ.

Android still owns the time, battery display, navigation buttons, gesture behavior, and system interactions. This API styles the surrounding bars; it does not replace their contents. Test both gesture navigation and button navigation where your target devices use them. Support here is for the native backend.

## Swipeable tabs

```python
from zkite import ScrollView, Tabs, Text, store

@store
class Screen:
    tab: int = 0

def view(s: Screen):
    return Tabs("flex-1",
        ScrollView("flex-1", Text("p-6", "Chats")),
        ScrollView("flex-1", Text("p-6", "Status")),
        ScrollView("flex-1", Text("p-6", "Calls")),
        labels=["Chats", "Status", "Calls"], index=s.tab,
        on_change=lambda index: s.update(tab=index), key="main-tabs",
    )
```

The selected index is controlled by your store. Native pages track horizontal
finger movement, then animate to a page boundary; vertical scrolling stays with
the page's scroll view. Taps animate to the selected page. Pages remain mounted,
so their vertical scroll offsets survive switching. Use a stable, unique key for
each tab group and keep expensive side effects out of page construction.
The WebView backend uses CSS scroll snapping and smooth programmatic scrolling.

Tabs select peer pages; `Nav` below manages a stack with Android Back behavior.
Use tabs for sections such as Chats, Status, and Calls, and navigation for opening
a detail screen.

## Navigation between screens

`Nav` manages a stack of screens. Screen functions describe individual views, and the first registered screen is the initial root. Screen names identify destinations, so choose stable, distinct names. A screen can receive the framework's `Navigator` through its annotated parameter.

| Navigator action | Intended effect |
|---|---|
| `push` | Open a new screen above the current one. |
| `pop` | Return to the preceding screen when one exists. |
| `replace` | Replace the current destination. |
| `pop_to_root` | Return to the root screen. |
| `reset(screen, **params)` | Replace the whole stack with one destination. |
| `current_entry` | Inspect the active entry and its parameters. |

Pass destination parameters that match what the receiving screen expects. Prefer small values that identify the data the screen needs, rather than treating the navigation stack as a large data store. Only the active screen is built as the current view.

Android Back can dismiss an active overlay before popping navigation. At the root, unhandled Back can leave the activity. Test this explicitly; a button that visually says Back should have behavior consistent with the user's expectations.

After a `Nav` registers its screens, invalid destinations and missing or unexpected parameters raise before changing the stack. Each pushed entry has its own widget identity, including explicit keys, so repeated detail screens do not share input focus or scroll state. Returning to an entry preserves its identity. A nested `Nav(host=...)` injects that host into its screens' `Navigator` parameters.

Native screens use a 160 ms entrance animation. Set `Nav("flex-1", home, detail, transition=False)` to disable it; an explicit animation on the screen's root takes precedence. Only the incoming screen is built.

```python
from zkite import Button, Column, Nav, Navigator, Text

def home(nav: Navigator):
    return Button("p-4", "Open item", on_tap=lambda: nav.push(detail, item_id=42))

def detail(nav: Navigator, item_id: int):
    return Column("gap-4", Text("", f"Item {item_id}"),
                  Button("p-4", "Back", on_tap=nav.pop))

def view():
    return Nav("flex-1", home, detail)
```

Navigation state is not automatically durable across process termination. Deep links and advanced routing policies are not promised by this base. `Entry` represents a route and its parameters; `NavigationHost` is an integration protocol for specialized hosts, not something a beginner must implement to use ordinary navigation.

## Overlays and selection

An `Overlay` places content above the normal screen without taking space in its underlying layout. Typical uses include confirmation panels, simple dialogs, and menus. The placement setting positions the panel; the scrim styles the area behind it.

Use application state to decide whether the overlay is present. Connect `on_dismiss` to the state update that closes it. Outside taps and Back can request dismissal; interacting inside the panel should remain a panel interaction. Showing an overlay conditionally is clearer than leaving a hidden interactive layer over the screen.

`Select` combines a trigger with an option panel. Options can be simple values or value/label pairs. Labels are for display; the callback receives the original chosen value, allowing the application to retain the intended type.

By default, Select manages its own open state. If you supply an explicit `open` value, the application owns that state and must respond to `on_open` and `on_dismiss`. A constant false value prevents opening; a constant true value prevents ordinary dismissal from changing the supplied state.

Panel, option, and selected-option classes let you style the presentation. A custom header can support additional content, but it does not automatically implement searchable selection; filtering remains application logic. Keep long option lists usable within the available screen height.

## Animation

The native animation API supports opacity and translation along the X and Y axes. Use it for restrained entrances, state transitions, or movement that clarifies a change. Rotation, scaling, and arbitrary style-property animation are not part of the documented base.

| Concept | Meaning |
|---|---|
| `Anim.enter` | An animation associated with entering. |
| `Anim.change` | An animation associated with a value change. |
| `Anim.exit_` | An exit animation definition. |
| Duration in milliseconds | A timed transition. |
| Spring stiffness and damping | A spring-based transition; an alternative to timed duration. |
| Easing | Linear, supported ease variants, or a cubic Bézier specification. |
| Delay | A delay in milliseconds before the transition starts. |
| Repeat/direction | Repetition behavior for supported timed tracks. |
| `AnimEnd` | Completion information, including channel and trigger. |

Duration-based and spring-based timing are alternative choices, not settings to combine indiscriminately. Keep animated widget identities stable across updates. For an exit animation, keep the widget present until the corresponding completion event allows the application to remove it; immediately removing the widget leaves nothing to animate.

Handle completion through `on_animend` when the application needs to coordinate the next state. Animation does not automatically add navigation transitions or make a slow view calculation fast. Keep essential information understandable without relying solely on motion.

## Configuration reference

A complete example (omit `icon` to use the supplied ZKite logo):

```json
{
  "name": "myapp",
  "version": "0.1.0",
  "backend": "native",
  "android": {
    "package": "com.yourcompany.myapp",
    "label": "My App",
    "version_code": 1,
    "permissions": ["INTERNET"],
    "icon": "assets/icon.png"
  }
}
```

All app settings come from `app.json` when it exists; legacy TOML app settings are then ignored. Keep the package identifier stable after publishing. JSON is read at build/preview preparation time and cannot change an installed launcher icon or name. A script may generate the file before a build. Stop and restart `zkite dev` after changing launcher settings.


`app.json` contains app settings. Run project commands beside it. It is declarative JSON, evaluated at build time; scripts can generate it. `pyproject.toml` is optional and remains available to package managers. Legacy projects without `app.json` can still use their existing TOML settings. See [configuration examples](Documentation.md#10-configure-and-build-your-application).

### Project metadata

| Section/key | Purpose |
|---|---|
| `name` | Project/distribution-style name used by the app project. |
| `version` | Application version text. It is independent of the framework release. |
| `dependencies` | Legacy only; prefer `requirements.txt` or `[project].dependencies`. |
| `backend` | Rendering backend; native is the default and the supported release path. |

### Android settings

| Key under `android` in app.json | Purpose and guidance |
|---|---|
| `package` | Android application ID. Choose a stable, valid reverse-domain-style identifier. |
| `label` | Human-readable app name shown by Android. |
| `version_code` | Integer base used in generated Android versioning. Increase appropriately for updates. |
| `min_sdk` | Minimum Android API level, default 26. |
| `target_sdk` | Android target API level, default 36. |
| `permissions` | Manifest permission declarations required by application features. |
| `icon` | Path to the launcher icon image. |

The package identifier and signing identity determine whether Android considers an installation an update to the same app. Changing the display label does not change that identity. Choose the final package identifier before distributing an app to users.

Generated per-ABI outputs can derive their Android version codes from the configured base. Inspect the produced artifacts when planning store updates; do not assume every per-ABI APK contains the base integer unchanged. Never reduce a version code for an intended update.

Permission declarations do not automatically implement runtime permission requests. A feature requiring user consent still needs the corresponding application/platform flow. Do not add a long list of permissions speculatively; use only what the app actually needs.

## Dependencies and networking

For pip projects, create `requirements.txt` with one package requirement per line, for example `requests==2.32.5`. Blank lines and comments are supported; pip options, recursive includes and line continuations are not. No `pyproject.toml` is required.

For uv or other package managers, use `[project].dependencies` in `pyproject.toml`. No dependency list is needed in `app.json`. Its legacy `dependencies` array is merged with a deprecation notice; it does not override pyproject dependencies. Requirements from `requirements.txt` are also merged; avoid conflicting constraints. ZKite does not currently consume uv lockfiles, so use pinned requirements when exact versions matter.

Declare application dependencies in these files. Both `zkite dev` and builds resolve the dependency graph, including transitive requirements, for the requested Android architectures. A package installed in your desktop environment is not automatically included in the Android app.

`zkite dev` sends resolved packages, metadata and resource files before loading the app. Editing `requirements.txt`, `[project].dependencies`, or `app.json` dependencies refreshes the package set and reloads the preview; removed packages are removed from the managed files on the phone. Invalid dependency changes report an error and leave the previous app running. Ordinary source edits reuse the installed package set. Resolutions are cached under `.zkite/dev-dependencies`; pin versions for repeatability. To refresh an unchanged, unpinned requirement, stop dev, remove that generated cache directory, and restart dev.

The supported dependency path is compatible pure-Python wheels. A package may look like Python but depend on compiled extensions or desktop operating-system services. Desktop wheels for Windows or Linux cannot simply be copied into Android. An unavailable or unsupported dependency should fail the build rather than be silently omitted.

Start with a small dependency set and build early. Add dependencies one at a time so compatibility failures are easy to locate. Pin versions where reproducibility matters, and verify that each dependency works in the actual Android environment, not only on your computer.

### Android markers and extras

Dependency markers use the Android target environment, not your computer's OS:
ARM64 uses `aarch64`, x86_64 uses `x86_64`, and Python marker versions match the
bundled CPython runtime (3.14.6 in this release). Extras are tracked separately for
each package. A false marker skips that dependency; a true marker resolves and
validates it normally. Pip retains version backtracking. Android release markers
use the minimum SDK baseline; device-specific kernel/build values are unavailable.
Applicable dependencies requiring unsupported native wheels still fail clearly.

### A pure-Python ASGI server

These dependencies can stay in `pyproject.toml`; no copy is needed in `app.json`:

```toml
[project]
name = "my-app"
version = "0.1.0"
dependencies = [
    "starlette==1.8.0",
    "uvicorn==0.54.0",
    "zkite>=1.1.1",
]
```

The `zkite` constraint is a minimum; install 1.1.2 to use the APIs in this guide.
Keep Uvicorn on its pure-Python networking stack:

```python
import uvicorn

config = uvicorn.Config(
    app,  # Your ASGI application.
    host="0.0.0.0",
    port=8767,
    loop="asyncio",
    http="h11",
    lifespan="off",
)
```

Do not start a long-running server inside `view()` or a synchronous UI callback.
Manage its startup and shutdown outside the UI thread. Avoid `uvicorn[standard]`
unless every extra dependency has supported Android wheels; choosing an extra
can legitimately make a previously optional native dependency required.

### Network requests

Declare `"permissions": ["INTERNET"]` under `android` in `app.json` when the app needs network access. New projects include this permission; remove it for offline apps. Requests and its pure-Python dependencies have been tested with verified HTTPS on the Moto G54. A declaration grants the manifest capability; it does not supply an HTTP client, authentication flow, retry strategy, or offline behavior. Use an appropriate compatible Python dependency or standard library feature and keep blocking network work out of view rendering.

Plan for slow connections, no connection, timeouts, invalid responses, and a user leaving the screen before a request finishes. Store credentials appropriately for the application's threat model. A private server secret embedded in an APK is recoverable and should not be treated as private.

### Dynamic imports

The build can prune unused parts of the Python standard library. Static analysis cannot always infer imports assembled dynamically at runtime. If a module works on the host but is missing in a built app, reproduce with `zkite run --no-prune-stdlib` or the equivalent build flag. If that resolves the failure, investigate the dynamic import path before attributing it to an unrelated dependency.

This flag concerns standard-library pruning. It does not add unsupported native packages, undeclared dependencies, or arbitrary application asset directories.

## Data and application lifecycle

The 1.0.0 stabilization commit preserves queued render commands and rejects expired Android surfaces during startup/resume. Text-selection callbacks also guard against an uninitialized overlay owner. These fixes are retained in 1.1.2; they do not make in-memory state persistent.

Android can pause, resume, or terminate an app independently of the development terminal. A well-behaved app must tolerate leaving the foreground and returning. In-memory stores represent the current process state; they are not a guarantee of survival after process termination.

Use deliberate persistence for data users expect to retain. Keep user data outside the managed application-source directory, whose contents can be replaced during app updates. Test save/load behavior after a normal restart and after an update installed with the same package and signing key.

Avoid doing irreversible work merely because a module was imported or a view was evaluated. Development reloads and process restarts can repeat those operations. Make initialization safe to repeat and distinguish loading existing data from creating a new record.

Test startup while the device is busy, background/resume, opening and closing the keyboard, and returning from another app. A successful first launch alone does not cover these lifecycle transitions. If a problem appears only after resume, record that sequence explicitly in the bug report.

## Command reference

Run `zkite --help` for the command list and `zkite COMMAND --help` for exact options. `zkite --version` has been available since 1.0.0 without a project or connected phone.

### `zkite new NAME [.]`

Creates `main.py`, `app.json`, `assets/`, `zkite.tailwind.json`, and `.gitignore`. `zkite new myapp` creates a new directory. `zkite new myapp .` creates these paths in the current directory. Existing scaffold paths are never overwritten; no pyproject file is generated.

### `zkite dev`

Prepares and installs a project preview, connects to the selected device, and watches supported source changes. Normal operation uses the bundled native preview runtime and project branding.

| Option | Use |
|---|---|
| `--serial SERIAL` | Select a device explicitly. |
| `--adb PATH` | Select the adb executable. |
| `--port NUMBER` | Select a host development port; default 8765. |
| `--backend native` | Explicitly select native rendering. |
| `--devshell-apk PATH` | Advanced custom-shell override; usually unnecessary. |

### `zkite run`

Builds a standalone APK for the chosen device, installs it, and launches it. The default signing key is retained per project for repeat development updates.

| Option | Use |
|---|---|
| `--serial SERIAL` | Select a device. |
| `--adb PATH` | Select adb. |
| `--backend native` | Select native rendering. |
| `--keystore PATH` | Use an existing combined signing PEM. |
| `--no-prune-stdlib` | Include the full standard library for diagnosis or dynamic-import needs. |
| `--logs` | Continue following the launched process's Android logs. |

### `zkite build apk` and `zkite build aab`

Build application artifacts without installing them. A connected phone is unnecessary. APK output is per architecture; an AAB can contain the selected architectures together.

| Option | Use |
|---|---|
| `--abi arm64-v8a` | Include the ARM64 Android architecture. |
| `--abi x86_64` | Include the x86_64 Android architecture. Repeat `--abi` to select both. |
| `--keystore PATH` | Override automatic release-key discovery with an existing combined PEM. |
| `--out DIRECTORY` | Choose the output directory; default is project `dist/`. |
| `--no-prune-stdlib` | Disable standard-library pruning. |
| `--backend native` | Select native rendering. |

If no ABI is specified, the command uses the available supported prebuilt architectures. If no signing key is supplied to a build command, it can create a temporary testing identity and warns accordingly. Such output is unsuitable as the basis of a continuing app release identity.

### `zkite doctor`

Checks the local tool and runtime environment. `--adb PATH` selects adb. Read its detailed results, especially the selected Python and runtime, when multiple environments are installed. A successful doctor report does not replace testing the app's own behavior.

### `zkite keygen`

Creates `keystore/release.pem` and `keystore/release.p12` by default, creating the directory and its ignore rule when needed. Existing key files are never overwritten. `--out BASENAME` controls the file basename; `--cn` controls the certificate common name; `--password` controls the PKCS#12 password. The combined PEM contains private key material and is not protected merely because the PKCS#12 has a password.

Use a private directory with appropriate access controls and an intentional password policy. Do not paste signing material into issue reports or commit it to Git. Preserve a secure backup of the actual release identity. A certificate name is descriptive and does not establish store ownership.

## Build, sign, and distribute an Android app

Framework distribution through pip and distribution of your Android app are separate activities. Your users install an APK or obtain the app through an Android distribution channel; they do not need Python, pip, or the ZKite CLI on their phones.

Build with `zkite build apk --abi arm64-v8a` or `zkite build aab`. Both discover
`keystore/release.pem` automatically and reuse it. If no default key exists, the
build creates the PEM/PKCS#12 pair. You can generate it beforehand:

```bash
zkite keygen --cn "My company"
zkite build apk --abi arm64-v8a
zkite build aab
```

Selection order is explicit `--keystore PATH`, `keystore/release.pem`, then legacy
root-level `zkite-release.pem`. If the PKCS#12 exists but its PEM is missing, restore the PEM or explicitly select
another existing key; generation refuses to overwrite an existing pair member. To sign another app
with the same key, copy your backed-up pair into its `keystore/` directory or use
`--keystore` with the original PEM path. Quote paths containing spaces. The folder
is ignored by Git; keep a secure backup outside the project as well.

`zkite run` continues to use the project's `.zkite/debug.pem` development identity.
Its development installs and your release-signed builds use separate signing
workflows.

Before publishing updates, preserve the application package ID and signing identity, increase the application's versioning appropriately, and verify update installation with existing user data. A signing mismatch is not fixed by changing the app label or rebuilding with another random key.

An AAB is an input to a bundle distribution workflow, not a file that `adb install` installs directly. Use the appropriate Android distribution tooling to validate and test bundles. ZKite producing an AAB does not by itself prove that every current store requirement has been met; consult the distribution service's current requirements at submission time.

Keep a record of the framework version, dependency versions, app version, selected architectures, and signing identity used for each shipped build. Do not record private key contents in that release record.

## Windows guide

Use the real Windows wheel and Windows CPython 3.14. You do not need WSL to run the Windows CLI. Keep the environment's Python and ZKite executable together so that package discovery refers to the intended installation.

If `zkite` is not recognized, use `.\.venv\Scripts\zkite.exe` or activate the environment. If `py -3.14` cannot find an interpreter, install the required CPython version and reopen the terminal. If paths contain spaces, quote the complete path; in PowerShell an executable path stored as a quoted string may need the call operator `&`.

Windows phones may require a manufacturer USB driver. If adb shows no device, verify the data cable and authorization first, then inspect the driver setup. A charging connection alone does not prove that debugging transport is available.

The optional `ZKITE_PYTHON` environment variable selects an explicit interpreter when automatic discovery chooses the wrong installation. Prefer the project's intended environment and verify its minor version. `ANDROID_SERIAL` selects a device in Windows just as it does on Linux.

Arrow-key device selection needs a real interactive terminal. For IDE tasks, scripts, redirected output, or CI, pass `--serial`. Do not work around selection issues by relying on whichever of several phones adb happens to choose.

## Troubleshooting

Start by locating the failure stage: installation, project discovery, dependency resolution, build, device connection, Android installation, startup, or interaction. The last useful error message usually identifies the stage. Preserve it before repeatedly changing unrelated settings.

### Installation and command problems

| Symptom | Checks and next action |
|---|---|
| `zkite` is not found | Use the environment's explicit executable path; verify installation with that environment's pip. |
| No matching distribution | Check Python 3.14, supported host architecture, and whether the requested release has been uploaded. |
| Wrong release runs | Compare `zkite --version` with the environment's `pip show`; inspect which executable your shell resolves. |
| Payload missing | Install the complete matching `zkite` set; do not copy only the CLI executable elsewhere. |
| Python version mismatch | Select CPython 3.14, recreate the environment if needed, and inspect `zkite doctor`. |
| PowerShell activation blocked | Use `.venv\Scripts\python.exe` and `.venv\Scripts\zkite.exe` directly. |

### Build and packaging problems

| Symptom | Checks and next action |
|---|---|
| No project or no `main.py` | Run from the generated project's directory and check filenames. |
| Dependency cannot be resolved | Inspect the named dependency and its transitive requirements; verify pure-Python Android compatibility. |
| Module missing only in the app | Check declared dependencies and top-level app modules; try `--no-prune-stdlib` for dynamic standard-library imports. |
| Image missing | Use a supported format under `assets/` or beside `main.py`, its exact relative path, and explicit dimensions. |
| Nested module or asset missing | Keep helper modules at the supported top level; put nested resources under the recursively packaged `assets/` directory. |
| App icon did not change | Activate the configuration line, check the real PNG, restart the command, and inspect the correct installed app. |
| AAB will not install with adb | Build an APK for direct installation or use the appropriate bundle testing workflow. |

### Device and installation problems

| Symptom | Checks and next action |
|---|---|
| Unauthorized device | Unlock it and accept USB debugging authorization. |
| Several devices, no selection | Use an interactive terminal or supply `--serial`. |
| Wrong phone chosen | Inspect explicit `--serial` and `ANDROID_SERIAL`; explicit selection wins. |
| Update rejected by Android | Read the install error; compare package ID, signing key, SDK, architecture, and versioning. |
| Signature mismatch | Find the original signing key; a new key cannot update an app signed with another identity. |
| App appears twice | Distinguish the development preview from the standalone application. |

Do not automatically uninstall an existing app to hide an update failure. Uninstalling can erase user data and removes evidence about the mismatch. Decide whether the installation is disposable before choosing that recovery path.

### Interface problems

| Symptom | Checks and next action |
|---|---|
| Text label is missing | Check the class-string-first constructor convention. |
| State changes but the screen does not | Use store `update` and check that the displayed widget reads the updated field. |
| Text input resets | Update the controlled value and keep a stable field identity. |
| Screen does not scroll | Bound the region, use `flex-1`, and check intermediate parents for `min-h-0`. |
| UI freezes during a request | Move blocking I/O into a module-level `Task` started from an event. |
| Text needs truncation | Constrain its width and use `multiline=False` or `max_lines`, with `overflow="ellipsis"`. |
| List rows overlap or clip | Verify fixed `item_height` against actual row content. |
| Select will not open or close | If `open` is explicit, update it in the corresponding callbacks. |
| Exit animation is invisible | Keep the widget present until its animation completion is handled. |
| Top or bottom bar stays white | Wrap the current root screen in `SystemBars` and provide supported hex colors. |
| Icon contrast is poor | Remember `light`/`dark` refers to icons; try `auto` and inspect on the actual device. |
| Blank screen after resume | Record the exact sequence, framework version, device/Android version, and relevant runtime logs. |

### Development reload problems

Confirm that `zkite dev` is still running, the device is authorized, and the preview is the foreground app you are inspecting. Check whether the changed file is one of the supported top-level source files. Dependency declarations refresh automatically. Android permissions, package identity, and launcher resources require restarting `zkite dev` to prepare a new preview package.

If a port is occupied, stop the old development process you own or choose another port. If the device disconnects, reconnect and inspect the command's reconnect messages. Do not start several competing preview sessions for the same device while diagnosing one failure.

### A useful bug report

Include the ZKite distribution versions, host OS and CPU, Python version, Android version, device model/ABI, exact command, the failure stage, and a short sequence that reproduces it. State whether the issue occurs in preview, standalone, or both, and whether it begins only after background/resume or an update.

Provide the smallest relevant application example through the project's support channel, along with sanitized error text. Remove passwords, tokens, personal data, private signing files, and unnecessary device identifiers. A screenshot can help explain layout; it cannot replace the error message for a build failure.

## Performance and accessibility

Keep view evaluation and event handlers responsive. Avoid rebuilding expensive data or decoding oversized images on every state change. Use `List` for large fixed-height collections, and keep ordinary scroll layouts for smaller content that needs flexible height.

Measure on the target phone. A powerful development computer and an emulator do not predict every mobile device's memory, keyboard, GPU, or background behavior. Test with realistic data sizes and network delays. A tiny display size does not make a very large decoded image cheap.

Use readable text, adequate contrast, clear action labels, and comfortable tap targets. Do not communicate errors only by color. Provide meaningful image alternatives when content matters and treat decorative imagery accordingly. Test with Android accessibility services and font/display settings on intended devices.

The framework has accessibility integration, but that is not a claim of complete TalkBack coverage, support for every system font-scale behavior, or certification against an accessibility standard. Application-specific testing remains necessary.

## Learning exercises

### Exercise 1: Understand the starter

Create a project and run it without a preview session. Identify the root layout, visible text, button callback, and store field in the generated sample. Change the displayed wording, rebuild, and verify the standalone launcher entry. Success means the app launches after the development terminal is closed.

### Exercise 2: Work with state

Add a second action that changes the same value and a separate value used by another control. Use store updates for both. Confirm that changing one field does not unintentionally reset the other. Restart the process to observe the difference between in-memory state and persistence.

### Exercise 3: Build a form

Create a labeled text field, a validation message, and a submission action. Test empty input, long input, Unicode, selection, and keyboard dismissal. Preserve the typed value across ordinary state updates. Explain which values belong to the form state and which should be saved durably.

### Exercise 4: Make a themed screen

Choose consistent spacing and colors, then configure status and navigation bars to match. Toggle the theme through state and verify icon contrast. Test both the app content and the system bars after leaving and returning to the app.

### Exercise 5: Add navigation and confirmation

Create a list screen and a detail screen. Pass an item's identifier to the detail view. Add a confirmation overlay for an action and verify outside-tap dismissal, Android Back, and return navigation. Ensure dismissing the overlay does not unexpectedly leave the screen.

### Exercise 6: Package a realistic app

Set a distinct Android package ID, app label, and launcher icon. Add a supported top-level image and one compatible dependency. Build and install with a retained signing key, then change the app and update it without losing a small piece of saved user data.

## Release checklist

Before sharing an app, verify the following on representative devices:

- A fresh standalone install starts without a computer or development server.
- A same-key update retains intended user data and displays the new application content.
- Text input, selection, keyboard dismissal, navigation, overlays, and Android Back behave correctly.
- Startup, background/resume, and interruption during loading do not leave a blank or frozen screen.
- The launcher icon and app label are correct for the intended package.
- System bars remain readable in each supported theme and navigation mode.
- Layout works with long text, realistic data, narrow screens, and relevant accessibility settings.
- Images and helper modules are actually packaged; dependencies work on Android.
- Networking handles offline, slow, and failed responses without blocking the interface.
- Permissions are appropriate, versioning is correct, and signing material is backed up privately.
- The chosen Android distribution channel accepts and validates the final artifact.

Keep a short record of devices and scenarios tested. Describe remaining limitations accurately rather than inferring coverage from a successful build alone.

## Frequently asked questions

**Do I need to run `zkite dev` before `zkite run`?** No. Run builds and launches a standalone app itself.

**Does the finished app need Python installed on the phone?** No. The supplied runtime is packaged with the application.

**Is the native app a website in a shell?** The default release uses native rendering. Web technologies are not required for the normal app workflow.

**Can I use every Python package?** No. Start with compatible pure-Python dependencies and test their Android behavior. Compiled desktop packages are not automatically supported.

**Can I organize my app into nested folders?** Not for automatic application-source/asset staging in this release. Keep supported app modules and bundled images at the project root.

**Why is there a development app and a normal app?** The preview has a separate package identity and data so development does not replace the standalone application's identity.

**Why did a value disappear after restarting the app?** Stores are in-memory state. Durable data requires deliberate persistence.

**Why does the top bar remain white with a dark screen?** The app background and Android bars are separate settings. Use `SystemBars` with explicit colors.

**Can I change the battery icon or replace Android navigation?** SystemBars controls backgrounds and icon contrast, not the operating system's contents or navigation behavior.

**Why does changing an icon not hot reload immediately?** Launcher icons are package resources. Restart the preview command or rerun the standalone build/install.

**Can I build without connecting a phone?** Yes, with `zkite build apk` or `zkite build aab`. Run and dev need a selected device.

**Can I publish the app to pip?** Pip distributes the development framework. Android users receive your APK or an app distributed through an Android channel.

**Should I upgrade only `zkite-framework`?** Keep the CLI, framework, and payload versions matched; install the complete `zkite` release.

**Does 1.0 mean all planned features are present?** No. This guide defines the base release's scope. Roadmap documents are not the API contract.

## Glossary

| Term | Meaning |
|---|---|
| ABI | The CPU/binary interface an Android native library targets, such as arm64-v8a. |
| adb | Android Debug Bridge, the host tool used to communicate with authorized devices. |
| APK | An Android application package that can be installed directly. |
| AAB | An Android App Bundle used by a bundle distribution workflow. |
| Callback | A function the framework invokes in response to an event. |
| Class string | Utility names describing a widget's layout and appearance. |
| Controlled input | A control whose displayed value comes from application state. |
| Development preview | The app instance used for source synchronization and hot reload. |
| Hot reload | Updating the running development interface after supported source changes. |
| Key | A stable widget identity across updates. |
| Native rendering | Drawing the interface through the native renderer rather than a browser document. |
| Package ID | Android's stable identifier for an application. |
| Payload | The prebuilt Android runtime components distributed with ZKite. |
| PyPI | The package index from which pip downloads Python distributions. |
| Signing identity | The private key and certificate identity used to sign an Android app. |
| Store | Observable in-memory application state. |
| System bars | Android's status bar and navigation/gesture area. |
| Virtual environment | An isolated host Python installation environment. |
| Virtual list | A list that constructs the visible rows and a small nearby buffer. |
| Wheel | A built Python distribution file installed by pip. |
| Widget tree | The hierarchy describing the current interface. |

## Guidance for the documentation website

Use the major sections as stable documentation pages, retaining a visible version selector or version label. Group them into Getting Started, Core Concepts, Widgets, Device Integration, Building Apps, and Troubleshooting. Keep the support matrix and packaging limitations easy to find before a learner chooses a project structure.

Present Windows and Linux installation as separate tabs, with copyable terminal commands and explicit working-directory notes. Keep prerequisites beside the commands. Show the current release number prominently.

Turn widget and configuration tables into reference pages without inventing extra properties. Link conceptual explanations to the corresponding troubleshooting entries. Label unsupported capabilities as current limitations and keep roadmap ideas separate from available API documentation.

This file can be used as public learning content. Do not enrich it with private signing examples, credentials, internal source listings, local maintainer machine paths, or unsupported promises. If the website later adds runnable application examples, verify them against the documented release rather than generating plausible but untested APIs.
