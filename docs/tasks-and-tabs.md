# Responsive requests and swipeable tabs

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

## Tabs

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

## Icons and emoji

`android.icon` still accepts a PNG path. Builds and project previews wrap it as a
full-bleed adaptive icon, allowing Android's launcher mask to shape it. Supply
square, opaque artwork with no baked-in padding; outer edges can be cropped by
launcher masks. Transparent margins in the source image remain transparent.

The native renderer registers Android's installed Noto color emoji font as its
emoji fallback. Available emoji sequences depend on the device's font version.
Rebuild the native shell to pick up renderer changes; Python hot reload alone does
not replace native code. Existing projects keep their own `main.py`; the new
orange-and-white starter is used by `zkite new`.

## Text overflow

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

## Android dependency markers

Use `pyproject.toml` or `requirements.txt`; no dependency list is needed in
`app.json`. Resolution uses pip's version backtracking and its per-package extras,
with a complete Android marker environment replacing host defaults. ARM64 maps to
`aarch64`, x86 maps to `x86_64`, and Python versions come from the configured bundled
runtime (`phase_1/cli/python/runtime-version.txt`). Android release markers use the
minimum SDK baseline; device-specific kernel/build version is unknown (`""`).
Version-dependent packages must work across the app's supported OS range.
False markers are skipped; applicable native-only dependencies still fail clearly.
For Uvicorn, use `loop="asyncio"`, `http="h11"`, `lifespan="off"` and avoid its
`standard` extra unless all its required wheels are supported on Android.

Scroll momentum now follows a density-independent Android-style spline sampled
against elapsed time, with timestamped input and a recent-sample velocity fit.
A held finger no longer launches momentum from stale movement samples.
