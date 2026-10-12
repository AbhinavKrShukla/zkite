"""§34 — a frame nobody asked for, and the first ZKite app to touch the network.

Two gaps in one screen, because they are one gap in practice (§33.8, §15.1):

1. **Nothing could push a frame from Python.** Every frame was *pulled* — the shell
   dispatched an event and painted the tree it got back — so a `@store.update()` from
   a background thread refreshed `App.last_tree` and **nothing repainted** until the
   user next touched the screen.
2. **No ZKite app had ever made a network request.** `ssl`, `socket`, `urllib` and
   `http.client` are all in the staged stdlib and the `.so`s ship, but that was read
   off the artifact, never observed.

Written against documented signatures only (§26.7's rule) — `Column`, `Row`, `Text`,
`Button` and `@store`, nothing else, and no private module is imported to observe
anything.

**What each row is for, all readable off a screenshot:**

- **`ticks`** is the push path with the network taken out of it. A plain
  `threading.Timer` chain bumps a counter once a second with no touch anywhere. If it
  is climbing on screen, frames are being pushed; if it is frozen at 0 while the log
  says the thread ran, they are not. It is deliberately the *first* thing on screen
  because it isolates the mechanism from everything DNS, TLS and HTTP can go wrong at.
- **`status`/`body`** is the whole point: tap **Fetch**, and the request happens on a
  worker thread. A pulled frame renders `status: fetching…` because the tap itself is
  an event; the **result** arrives with nobody asking, which is the frame that could
  not have existed before this session.
- **`err`** carries the exception *type and message* rather than a bare "failed",
  because the two predicted failure modes are distinguishable from exactly that:
  `URLError`/`PermissionError` from a missing `INTERNET` manifest permission, and
  `CERTIFICATE_VERIFY_FAILED` from OpenSSL's compiled-in `/usr/local/ssl/certs`, which
  does not exist on a phone (§34.5).

`https://example.com` is the target on purpose: it is a real TLS endpoint, run by
IANA, with a tiny stable body and no API key, so a pass is a genuine certificate
validation against Android's own trust store rather than a plaintext fetch that would
prove nothing about `ssl`.
"""

import threading
import urllib.request

from zkite import Button, Column, Row, Text, store

URL = "https://example.com"


@store
class Net:
    ticks: int = 0
    status: str = "idle"
    body: str = ""
    err: str = ""


def _tick(s: Net):
    """Bump a counter every second, forever, from a timer thread.

    Nothing here is touched by a user and nothing dispatches an event, so every repaint
    this causes is a pushed one. Daemon threads so they cannot keep the process alive.
    """
    s.update(ticks=s.ticks + 1)
    t = threading.Timer(1.0, _tick, args=(s,))
    t.daemon = True
    t.start()


_started = False


def _start_ticking(s: Net):
    """Arm the tick chain once, on the first render.

    Scheduled rather than run inline: calling `_tick` here would `update()` from
    inside `view()`, which re-enters the render that is still walking this tree. `App`
    now skips such a nested render (phase_3/PLAN.md §34.2), so it would work — but an
    app should not lean on that, and a demo app least of all.
    """
    global _started
    if _started:
        return
    _started = True
    t = threading.Timer(1.0, _tick, args=(s,))
    t.daemon = True
    t.start()


def _fetch(s: Net):
    """One HTTPS GET, on a worker thread, then `update()` — which is the push."""
    try:
        with urllib.request.urlopen(URL, timeout=15) as r:
            code = r.status
            text = r.read(200).decode("utf-8", "replace")
        first = " ".join(text.split())[:60]
        s.update(status="ok %d" % code, body=first, err="")
    except Exception as exc:  # noqa: BLE001 — the failure *type* is the measurement
        s.update(status="failed", body="", err="%s: %s" % (type(exc).__name__, exc))


def view(s: Net):
    _start_ticking(s)
    return Column(
        "flex-1 bg-slate-950 p-6 gap-4",
        Text("text-white text-2xl font-bold", "§34 push + network"),
        Row(
            "gap-3 items-center",
            Text("text-slate-400 text-lg", "ticks:"),
            Text("text-emerald-400 text-3xl font-bold tabular-nums", str(s.ticks)),
        ),
        Button(
            "px-6 py-3 rounded-2xl bg-indigo-500 text-white font-medium",
            "Fetch",
            on_tap=lambda: _begin(s),
        ),
        Row(
            "gap-3 items-center",
            Text("text-slate-400 text-lg", "status:"),
            Text("text-amber-300 text-xl font-mono", s.status),
        ),
        Text("text-slate-300 text-sm", "body: " + (s.body or "-")),
        Text("text-rose-400 text-sm", "err: " + (s.err or "-")),
    )


def _begin(s: Net):
    """The tap's own work: flip to `fetching…` (a *pulled* frame, because a tap asked
    for it) and hand the slow part to a thread."""
    s.update(status="fetching…", body="", err="")
    t = threading.Thread(target=_fetch, args=(s,), daemon=True)
    t.start()
