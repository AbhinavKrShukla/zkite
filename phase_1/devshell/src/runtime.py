"""Embedded once at boot (see lib.rs `RUNTIME_GLUE`). Defines the entry points Rust
calls by name into `__main__` — this is where almost all the logic lives, on purpose:
Rust's job is JNI plumbing and process/thread bookkeeping, not framework logic that's
already built, tested, and living in zkite_py (see ../../PLAN.md M4).

sys.stdout/sys.stderr are tee'd the same way phase_0/spike1 does it (Android drops a
bare write(1, ...) on the floor for an app process), so prints and uncaught tracebacks
from *this* glue (not from the user's `view()`, which zkite_py's App already catches
and renders as an overlay) still reach logcat.

`_zkite_setup_tree`/`_zkite_dispatch_tree` are phase 3's native-renderer counterparts of
`_zkite_setup`/`_zkite_dispatch` — same shape (a JNI-callable function taking/returning
plain strings; the Rust side already knows how to call a zero/N-arg glue function and
read back a string-or-None), just JSON instead of HTML. Shared by every backend that
`include_str!`s this file (devshell, appshell, and phase_3/pybridge) exactly like the
HTML functions already are — see phase_3/PLAN.md §5.
"""
import io
import os
import sys

# `json` is deliberately NOT imported here — see `_install_native_extension_finder`.
# Nothing above this line may import a stdlib module that binds a C accelerator at
# import time, because the finder that makes those accelerators importable at all is
# installed further down and cannot run before `os`/`sys` exist.


class _Tee(io.TextIOBase):
    def __init__(self):
        self.buf = io.StringIO()

    def writable(self):
        return True

    def write(self, s):
        self.buf.write(s)
        return len(s)

    def flush(self):
        pass


sys.stdout = _Tee()
sys.stderr = _Tee()


class _ZKiteNativeExtensionFinder:
    """Makes CPython's C-extension stdlib modules importable on Android.

    Android's package installer extracts an APK's ``lib/<abi>/`` entries into the
    app's nativeLibraryDir, but **only** those whose basename matches ``lib*.so``.
    An extension module ships as ``math.cpython-314-aarch64-linux-android.so``,
    which does not match, so it is silently dropped at install time — present in
    the APK, absent on the device, with no diagnostic anywhere in between. That is
    why ``import math`` had never worked on any ZKite backend since phase 1; see
    ../../phase_3/PLAN.md §11.6 for the measurement that found it.

    The build scripts therefore stage each module as ``libpymod_<name>.so``, which
    the installer does extract. That name is invisible to the ordinary import
    machinery (``FileFinder`` looks for ``<name>`` + one of
    ``_imp.extension_suffixes()``, and ``libpymod_math.so`` is not ``math.so``),
    so this finder does the mapping back.

    It is deliberately a plain ``sys.meta_path`` entry rather than a ``.pth`` hook
    or a path-entry finder: it has to answer for module names, not directories,
    and the set of available modules is fixed at build time, so one ``listdir``
    per candidate directory at startup is the whole cost.

    Loading is left entirely to ``ExtensionFileLoader``, which resolves a module's
    init function from the *module name* (``PyInit_math``) and not from the file
    name — so a renamed file loads exactly as the original would. Nothing here
    reimplements any part of the import protocol.
    """

    _PREFIX = "libpymod_"
    _SUFFIX = ".so"

    def __init__(self, modules):
        self._modules = modules  # module name -> absolute .so path

    @classmethod
    def discover(cls, paths):
        """Scan `paths` for `libpymod_*.so`, returning a finder or None.

        `paths` is normally `sys.path`: the shells write a `.pth` naming the
        app's real nativeLibraryDir (only knowable at runtime), which `site`
        appends during interpreter startup. Scanning every entry rather than
        trusting one keeps this independent of how that directory got there.
        """
        modules = {}
        for entry in paths:
            if not entry or not os.path.isdir(entry):
                continue
            try:
                names = os.listdir(entry)
            except OSError:
                continue
            for name in names:
                if name.startswith(cls._PREFIX) and name.endswith(cls._SUFFIX):
                    mod = name[len(cls._PREFIX):-len(cls._SUFFIX)]
                    # First match wins, so an earlier sys.path entry shadows a
                    # later one — the same precedence ordinary imports have.
                    modules.setdefault(mod, os.path.join(entry, name))
        return cls(modules) if modules else None

    def find_spec(self, fullname, path=None, target=None):
        # Extension modules in lib-dynload are all top-level; a dotted name is
        # always something else's submodule and must not be answered for here.
        so_path = self._modules.get(fullname)
        if so_path is None or "." in fullname:
            return None
        from importlib.machinery import ExtensionFileLoader
        from importlib.util import spec_from_file_location
        return spec_from_file_location(
            fullname, so_path, loader=ExtensionFileLoader(fullname, so_path)
        )


def _install_native_extension_finder():
    """Append the finder to `sys.meta_path`, if there is anything to find.

    Appended, not prepended: a genuinely importable module (a builtin, or a
    frozen/pure-Python one) must keep winning, so this only ever answers for
    names nothing else could resolve. Returns the module count for logging —
    silence here would recreate exactly the blind spot §11.6 came from.

    **This must run before any stdlib module that binds a C accelerator at import
    time is imported, and that ordering is load-bearing rather than tidy** (see
    ../../phase_3/PLAN.md §43). `json/encoder.py` and `json/scanner.py` do

        try:
            from _json import make_encoder as c_make_encoder
        except ImportError:
            c_make_encoder = None

    at module scope, **once**. `_json` ships as `libpymod__json.so` and is reachable
    only through this finder, so importing `json` first made that `except` branch
    run and pinned the whole process to the pure-Python encoder *and* decoder — for
    good, since the binding is never revisited and `import _json` succeeding later
    changes nothing. Nothing errored and nothing logged; `json` simply ran ~8x
    slower for the life of the app.

    Measured on the Moto G54, where `json.dumps` of a 15KB tree was **3.75ms**, 44%
    of a virtualised list's entire per-rebuild cost, against ~0.3ms predicted for the
    C encoder at this device's measured CPU factor. The probe that settled it read
    `c_make_encoder=False` while `import _json` in the very same call **succeeded**,
    which is the signature of this bug and of nothing else.

    The general hazard is wider than `json` — `pickle`, `datetime`, `decimal`,
    `heapq`, `bisect`, `csv`, `statistics` and `zoneinfo` all use the same
    try/except-at-import-time idiom — which is why this is placed above *every*
    non-essential import in this file rather than just above `json`.
    """
    finder = _ZKiteNativeExtensionFinder.discover(sys.path)
    if finder is None:
        return 0
    sys.meta_path.append(finder)
    return len(finder._modules)


_native_extension_count = _install_native_extension_finder()

# Everything below may now import freely. Kept here, after the finder, rather than at
# the top of the file — a linter will want to move these and must not.
import json  # noqa: E402
import traceback  # noqa: E402

print("zkite: %d native extension modules available" % _native_extension_count)


# Where Android actually keeps its trusted CA certificates, newest first. The apex
# path is the updatable store Android 14+ treats as authoritative; the `/system` one
# is the traditional location and is still present. Measured on a Moto G54 running
# Android 15: **both exist, with 145 certificates each**.
_ANDROID_CA_DIRS = (
    "/apex/com.android.conscrypt/cacerts",
    "/system/etc/security/cacerts",
)

_CA_BUNDLE_NAME = "zkite-cacerts.pem"
_BEGIN_CERT = "-----BEGIN CERTIFICATE-----"
_END_CERT = "-----END CERTIFICATE-----"


def _build_ca_bundle(src_dir, bundle_path):
    """Concatenate every certificate in `src_dir` into one PEM file, if it is stale.

    Returns the bundle path, or None if it could not be produced.

    **Why a bundle rather than just pointing `SSL_CERT_DIR` at the directory**, which
    is the obvious thing and is what the first version of this did. Android's CA
    directories *are* OpenSSL `capath` directories, but their filenames are the
    **old** subject hash — `X509_NAME_hash_old`, the MD5-based OpenSSL 0.9.8 scheme —
    and a modern OpenSSL's `by_dir` lookup does not find a certificate under that
    name. Measured, on the host, with the device's own 145 certificates and identical
    bytes in all three arms (phase_3/PLAN.md §34.5):

        capath, device filenames (old MD5 hash)     -> CERTIFICATE_VERIFY_FAILED
        capath, same certs renamed to the SHA-1 hash -> ok 200
        cafile, same certs concatenated              -> ok 200

    Rehashing would mean reimplementing RFC 5280 name canonicalisation in Python to
    reproduce `X509_NAME_hash` exactly; concatenating needs no crypto at all, which is
    why it is the one chosen.

    Cheap after the first launch: rebuilt only when the bundle is missing or the
    source directory's `(mtime, entry count)` has moved, so the steady state is two
    `stat` calls. The certificates still come from the device, so an Android update
    that changes the trust store is picked up rather than pinned — which is the whole
    reason for not shipping a bundle of our own.
    """
    try:
        names = sorted(os.listdir(src_dir))
        stamp = "%d:%d" % (int(os.stat(src_dir).st_mtime), len(names))
        stamp_line = "# zkite-ca-stamp %s\n" % stamp
        try:
            with open(bundle_path, "r") as f:
                if f.readline() == stamp_line:
                    return bundle_path
        except OSError:
            pass
        out = [stamp_line]
        for name in names:
            try:
                with open(os.path.join(src_dir, name), "r") as f:
                    pem = f.read()
            except OSError:
                continue
            # Android's files carry human-readable text after the certificate; take
            # exactly the PEM block and nothing else.
            i = pem.find(_BEGIN_CERT)
            j = pem.find(_END_CERT)
            if i < 0 or j < 0:
                continue
            out.append(pem[i:j + len(_END_CERT)] + "\n")
        if len(out) == 1:
            return None
        tmp = bundle_path + ".tmp"
        with open(tmp, "w") as f:
            f.write("".join(out))
        os.replace(tmp, bundle_path)
        return bundle_path
    except Exception:
        return None


def _configure_ca_certificates(cache_dir):
    """Point OpenSSL at Android's own CA store. Returns a short status for the log.

    **Without this, every HTTPS request from a ZKite app fails**, in the least helpful
    way available: `ssl.create_default_context()` succeeds, loads zero certificates,
    and the failure surfaces at connect time as `CERTIFICATE_VERIFY_FAILED: unable to
    get local issuer certificate` — which reads like a problem with the server. The
    cause is visible in the shipped artifact and has nothing to do with Android: the
    vendored OpenSSL was configured with the default prefix, so
    `X509_get_default_cert_file()` is the string `/usr/local/ssl/cert.pem`, baked into
    `libcrypto_python.so`, naming a path that has never existed on a phone.

    `SSL_CERT_FILE` is the documented override for exactly that constant, read by
    OpenSSL at `SSL_CTX_set_default_verify_paths` time — which is what
    `ssl.SSLContext.load_default_certs()` calls — so setting it in `os.environ` before
    any app code runs is sufficient and needs no change to `ssl.py`.

    Three guards, each doing a different job. An existing `SSL_CERT_FILE` is never
    overwritten, so a developer pinning their own store still wins. A source directory
    that is not there is skipped, which keeps this inert on a host — `zkite_py`'s own
    test suite and the conformance harness run this same glue on Linux, where the
    system OpenSSL is already configured correctly and must not be repointed. And the
    whole thing is wrapped, because a framework that cannot boot because it failed to
    build a certificate bundle is worse than one that cannot do HTTPS.
    """
    try:
        if os.environ.get("SSL_CERT_FILE"):
            return "pinned by SSL_CERT_FILE"
        src = next((p for p in _ANDROID_CA_DIRS if os.path.isdir(p)), None)
        if src is None:
            return None
        if not cache_dir or not os.path.isdir(cache_dir):
            return "no writable cache dir — HTTPS will fail"
        bundle = _build_ca_bundle(src, os.path.join(cache_dir, _CA_BUNDLE_NAME))
        if bundle is None:
            return "could not build a bundle from %s — HTTPS will fail" % src
        os.environ["SSL_CERT_FILE"] = bundle
        return "%s -> %s" % (src, bundle)
    except Exception:
        return None

_app = None
_app_dir = None

# The last *glue-level* traceback — a failure that happened outside `App`, so `App`
# never saw it and `_app.last_error` cannot report it (phase_3/PLAN.md §31.4).
#
# The motivating case is the common one: an `ImportError` in the user's `main.py`
# raises inside `load_module`, which runs *before* `_app` is assigned, so `_app` is
# still `None`, `_zkite_last_error` returned `None`, no `OutFrame::Error` was ever
# written, and `zkite dev`'s terminal stayed silent while the phone displayed the
# traceback. plan1.md §6 promises the traceback appears "simultaneously in the
# terminal"; for the whole class of errors that stop an app booting at all, it did
# not. Every site that renders a glue overlay now also records it here, and every
# site that renders a *successful* frame clears it, so a fixed error stops being
# reported on the next reload rather than sticking forever.
_glue_error = None

# Whether `zkite dev` has ever driven a reload on this connection. Only the reload
# socket calls `_zkite_reload`/`_zkite_reload_tree`, so this is precisely "a dev server
# has talked to us", which is the question `_WAITING_*` answers (phase_3/PLAN.md
# §31.5). Composing the waiting screen into *every* error was right before a
# connection existed and actively misleading afterwards — it reads as "the dev server
# has disconnected" while it is connected and serving.
_dev_connected = False

_WAITING_HTML = (
    '<div id="r" class="flex flex-1 flex-col items-center justify-center gap-3 bg-slate-950 p-8">'
    '<span class="text-white text-lg font-mono">waiting for `zkite dev`…</span>'
    '<span class="text-slate-500 text-sm font-mono">run it from your project directory</span>'
    "</div>"
)

# Tree-shaped counterpart of `_WAITING_HTML`, node for node — see module doc.
_WAITING_TREE = {
    "tag": "div", "id": "r",
    "classes": "flex flex-1 flex-col items-center justify-center gap-3 bg-slate-950 p-8",
    "events": [], "attrs": {},
    "children": [
        {
            "tag": "span", "id": "__waiting_title", "classes": "text-white text-lg font-mono",
            "events": [], "attrs": {}, "children": [{"tag": "#text", "text": "waiting for `zkite dev`…"}],
        },
        {
            "tag": "span", "id": "__waiting_sub", "classes": "text-slate-500 text-sm font-mono",
            "events": [], "attrs": {},
            "children": [{"tag": "#text", "text": "run it from your project directory"}],
        },
    ],
}


def _zkite_drain_output():
    out = sys.stdout.buf.getvalue() + sys.stderr.buf.getvalue()
    sys.stdout.buf.seek(0)
    sys.stdout.buf.truncate(0)
    sys.stderr.buf.seek(0)
    sys.stderr.buf.truncate(0)
    return out


def _zkite_framework_fingerprint(zkite_pkg_dir):
    """A short content hash of the `zkite` package this shell actually shipped.

    **Why a content hash and not a version string** (phase_3/PLAN.md §31.6). The Dev
    Shell pins the framework: `zkite dev` pushes only the *project's* `.py` files, while
    `zkite/` itself lives in the shell's `assets/zkite-framework.zip`. So a widget added
    after the installed shell was built is simply absent, and the user meets a bare
    `ImportError` on the phone with nothing anywhere naming the cause — which is what
    happened the day after §30 shipped `Checkbox`.

    §15.3's own proposed fix was to put "the shell's `zkite_py` version" in the `hello`
    frame. That would not have detected this: `phase_1/zkite_py/pyproject.toml` reads
    `version = "0.1.0.dev0"` and has not moved across any commit in this repo, so the
    stale shell and the fresh payload would have reported the *same* version and the
    handshake would have stayed silent through the exact failure it was written for.
    What distinguishes a stale shell is its framework *contents*, so that is what is
    hashed. `phase_1/cli/src/dev_cmd.rs::framework_fingerprint` computes the identical
    digest over the payload's own `zkite/*.py` and compares.

    Takes the package directory rather than reading a global so it is a pure function
    of its input, which is what lets `zkite_pybridge`'s own test drive it over a fixture
    directory and assert the digest against the literal `dev_cmd.rs` pins — one spec,
    two implementations, and neither can drift without a test going red.

    Fails **open**, on purpose: any problem here returns `None`, the field is omitted
    from the frame, and `zkite dev` says nothing. A false "your Dev Shell is stale" would
    send someone on a ten-minute rebuild for no reason, which is worse than the silence
    this replaces.
    """
    try:
        import hashlib

        if not zkite_pkg_dir:
            return None
        pkg = os.path.join(zkite_pkg_dir, "zkite")
        if not os.path.isdir(pkg):
            return None
        h = hashlib.sha256()
        # Sorted, name-and-content, NUL-delimited: a rename or a deletion moves the
        # digest just as a content edit does. Only this directory's own `.py` files —
        # `zkite` has no subpackages, and `__pycache__` must not be part of an identity
        # that has to match a *source* tree on the host.
        names = sorted(
            n for n in os.listdir(pkg)
            if n.endswith(".py") and os.path.isfile(os.path.join(pkg, n))
        )
        for name in names:
            with open(os.path.join(pkg, name), "rb") as f:
                body = f.read()
            h.update(name.encode("utf-8"))
            h.update(b"\0")
            h.update(body)
            h.update(b"\0")
        return h.hexdigest()[:12]
    except Exception:
        return None


def _push_frame(frame):
    """Hand a frame the app rendered *on its own* to whatever shell is embedding us.

    This is the whole of the push path in Python (phase_3/PLAN.md §34). Until it
    existed, every frame in ZKite was **pulled**: a shell called `_zkite_dispatch*` and
    painted what came back, so a `@store.update()` from a background thread — a
    finished network fetch, a timer — refreshed `App.last_tree` and nothing ever drew
    it. `App` has had `on_render`/`on_render_tree` since phase 1 and nothing had ever
    registered either.

    `_zkite_native_frame_ready` is defined **in Rust**, injected into this module's
    globals by whichever shell booted the interpreter (`zkite_pybridge::embed`'s
    `install_frame_hook`). Looked up by name at call time, not captured at import:
    this file is `include_str!`'d by three shells and executed before any of them has
    installed anything, and a shell that never installs one (or a host test running
    this glue with no Rust underneath) simply has no hook and pushes nothing. That
    absence is the feature — the push path is inert unless something opted in, which
    is what keeps a pulled frame the only frame anywhere it used to be.

    Never allowed to raise. It runs inside `@store.update()`, arbitrarily deep inside
    user code, and a framework that can turn `s.update(n=1)` into a traceback because
    the transport hiccuped is worse than one that drops a repaint.
    """
    hook = globals().get("_zkite_native_frame_ready")
    if hook is None:
        return
    try:
        hook(frame)
    except Exception:
        pass


def _register_push(app, tree_mode):
    """Point `app`'s render callback at `_push_frame`, in the mode it renders in.

    Serialising here rather than making the shell come back for it is deliberate: the
    callback fires holding exactly the tree that was just built, so what the shell
    receives cannot be a *later* frame that a second `update()` raced in behind it —
    and the shell needs no second GIL acquisition to go and fetch one.
    """
    if tree_mode:
        app.on_render_tree(lambda tree: _push_frame(_tree_frame(tree)))
    else:
        app.on_render(_push_frame)


def _record_glue_error():
    """Capture the exception being handled, so `_zkite_last_error` can report it."""
    global _glue_error
    from zkite.errors import log_error

    tb = traceback.format_exc()
    if tb != _glue_error:
        log_error(tb)
    _glue_error = tb
    return _glue_error


def _ok(frame):
    """Mark this turn as having produced a good frame, and return it.

    Wrapping the successful returns rather than clearing at each entry point's *top*
    is deliberate: a `_zkite_dispatch*` that returns `None` for "nothing handled it"
    has not fixed anything, and must not clear an error the previous frame reported.
    """
    global _glue_error
    if _glue_error is not None:
        from zkite.errors import log_recovered

        log_recovered()
    _glue_error = None
    return frame


def _glue_error_html(exc):
    from zkite.errors import format_error_overlay

    _record_glue_error()
    # The HTML overlay is `position:fixed;inset:0` with an opaque background, so what
    # is underneath it is invisible either way; it is composed conditionally purely so
    # the two backends' glue stays one shape (see `_glue_error_tree`, where it is
    # load-bearing).
    below = "" if _dev_connected else _WAITING_HTML
    return below + format_error_overlay(traceback.format_exc())


def _purge_app_bytecode_cache(app_dir):
    """Drop any cached bytecode for the *app's own* files before (re)loading them.

    The shells enable CPython's bytecode cache (``config.write_bytecode = 1``),
    which is worth 2.3-2.6x on cold start because the pruned stdlib ships as ~490
    source files — see phase_3/PLAN.md §17.1. That cache is validated by
    **(source mtime in whole seconds, source size)**, and those are exactly the two
    things a hot reload can leave unchanged: ``text-red-500`` -> ``text-red-600``
    is the same length, and two saves can land in the same second. CPython would
    then consider the stale ``.pyc`` valid and the edit would silently not apply —
    the "hot reload is flaky" failure with no error anywhere.

    Found by this cache making ``zkite_pybridge``'s own test suite intermittently
    load the previous test's app (2 failures in 6 runs, 0 in 8 with the cache off);
    those tests rewrite different apps to one path, which is the same shape as a
    reload and is why it reproduced there first.

    Only the app directory is purged. The stdlib and framework caches — which are
    where the entire startup win lives — are untouched, so this costs one small
    recompile per boot/reload and nothing else.
    """
    if not app_dir:
        return
    for dirpath, dirnames, _ in os.walk(app_dir):
        if "__pycache__" in dirnames:
            dirnames.remove("__pycache__")
            cache = os.path.join(dirpath, "__pycache__")
            for name in os.listdir(cache):
                try:
                    os.remove(os.path.join(cache, name))
                except OSError:
                    pass


def _find_main(app_dir):
    """`main.py` for the dev loop (zkite dev always pushes source), `main.pyc` for a
    release build (zkite build apk/aab pyc-compiles and strips source — see
    ../../../phase_2/PLAN.md's pyc milestone). Checked in that order so a project
    directory that happens to have both (stale leftover) prefers the source."""
    for name in ("main.py", "main.pyc"):
        candidate = os.path.join(app_dir, name)
        if os.path.exists(candidate):
            return candidate
    return None


def _boot_app(main_py):
    global _app
    from zkite.app import App
    from zkite.reload import load_module

    _purge_app_bytecode_cache(os.path.dirname(main_py))
    module = load_module(main_py, "main")
    _app = App(module)
    _register_push(_app, tree_mode=False)
    return _app.render()


def _zkite_setup(zkite_pkg_dir, app_dir):
    global _app_dir
    _app_dir = app_dir
    if zkite_pkg_dir not in sys.path:
        sys.path.insert(0, zkite_pkg_dir)
    if app_dir not in sys.path:
        sys.path.insert(0, app_dir)
    # Here rather than at module level, because this is the first point at which a
    # writable directory is known — the bundle is cached beside the unpacked app.
    _ca = _configure_ca_certificates(os.path.dirname(app_dir) or app_dir)
    if _ca:
        print("zkite: TLS trust store %s" % _ca)
    main_py = _find_main(app_dir)
    try:
        if main_py is None:
            return _ok(_WAITING_HTML)
        return _ok(_boot_app(main_py))
    except Exception as exc:  # glue-level failure, not a user `view()` bug
        return _glue_error_html(exc)


def _zkite_dispatch(node_id, event, payload):
    if _app is None:
        return None
    if event == "scroll":
        return _zkite_scroll_html(node_id, payload)
    try:
        handled = _app.dispatch(node_id, event, payload)
    except Exception:
        return _glue_error_html(None)
    if not handled:
        return None
    return _ok(_app.last_html)


def _zkite_scroll_html(node_id, payload):
    """The WebView backend's half of virtualised-list scroll reporting
    (phase_3/PLAN.md §11) — the HTML counterpart of `_zkite_scroll_tree`.

    Routed through the existing `dispatch` bridge with a reserved `"scroll"` event
    name rather than as a new `ZKiteNative` method, deliberately: `dispatch` is
    already wired end to end through both shipped shells' Java, JNI, and Rust
    (`devshell`, `appshell`), and adding a bridge method would mean touching all
    of that for a message the existing one already carries. `"scroll"` never
    reaches `App.dispatch`, so it can't collide with a user's own `on_scroll`
    handler by accident — there is no such prop today, and if one is ever added
    this is the single place that has to route both.

    `payload` is `"<scroll_y>,<viewport_height>"`, the only shape the bridge's
    string-in/string-out contract allows without inventing an encoding.
    """
    try:
        from zkite import _viewport

        scroll_y, _, viewport_h = str(payload).partition(",")
        if not _viewport.set_viewport(node_id, float(scroll_y), float(viewport_h)):
            return None
        if not _viewport.window_would_change(node_id):
            return None
        return _ok(_app.render())
    except Exception:
        return _glue_error_html(None)


def _zkite_back():
    """A platform Back press, HTML backend. See `_zkite_back_tree` for the contract.

    Returns the new HTML if navigation consumed the press, `None` if it did not and
    the shell should do the platform default (exit the app).
    """
    if _app is None:
        return None
    try:
        if not _app.dispatch_back():
            return None
        return _ok(_app.last_html)
    except Exception:
        # A crash inside a back handler must not leave Back permanently dead, but
        # it also must not exit the app — the user would lose their place to a bug
        # in one screen's own code. Show the overlay and treat the press as
        # consumed, which is the same call `_zkite_dispatch` makes for a tap.
        return _glue_error_html(None)


def _zkite_reload():
    global _app, _dev_connected
    _dev_connected = True
    main_py = _find_main(_app_dir)
    try:
        if main_py is None:
            return _ok(_WAITING_HTML)
        if _app is None:
            return _ok(_boot_app(main_py))
        from zkite.reload import reload_module

        _purge_app_bytecode_cache(_app_dir)
        result = reload_module(main_py, "main")
        _app.use_module(result.module)
        return _ok(_app.render())
    except Exception:
        return _glue_error_html(None)


def _zkite_last_error():
    """The last render's traceback (if any), for the reload socket to mirror into
    `zkite dev`'s terminal — the on-device overlay already shows it, but plan1.md §6
    wants both ("simultaneously in the terminal")."""
    if _glue_error is not None:
        return _glue_error
    if _app is None:
        return None
    return _app.last_error


# ---------------------------------------------------------------------------------
# Native (phase 3) tree-mode entry points — see module doc.


def _glue_error_tree(exc):
    """The tree-mode error overlay (phase_3/PLAN.md §31.5).

    `_WAITING_TREE` is composed underneath **only when no `zkite dev` has ever driven a
    reload**. It used to be unconditional, and unlike the HTML twin that is not merely
    cosmetic: the native error overlay is opaque, but the tree is also what AccessKit
    walks and what `uiautomator dump` reports, so a screen reader announced "waiting
    for `zkite dev`…" behind a live error either way.
    """
    from zkite.errors import format_error_tree

    _record_glue_error()
    below = [] if _dev_connected else [_WAITING_TREE]
    return json.dumps({
        "tag": "div", "id": "__zkite_root", "classes": "relative w-full h-full", "events": [], "attrs": {},
        "children": below + [format_error_tree(traceback.format_exc())],
    }, allow_nan=False)


def _tree_frame(tree):
    """Encode a native frame, turning invalid app data into a recoverable overlay.

    Python's default NaN/Infinity literals are not JSON and serde_json rejects the
    whole tree. Keep validation in the existing C encoder, with no extra tree walk.
    This helper is also used by background pushes, where raising would otherwise
    escape from an ordinary store.update() and leave the screen stale.
    """
    try:
        return _ok(json.dumps(tree, allow_nan=False))
    except Exception:
        return _glue_error_tree(None)


def _boot_app_tree(main_py):
    global _app
    from zkite.app import App
    from zkite.reload import load_module

    _purge_app_bytecode_cache(os.path.dirname(main_py))
    module = load_module(main_py, "main")
    _app = App(module)
    _register_push(_app, tree_mode=True)
    return _tree_frame(_app.render_tree())


def _zkite_setup_tree(zkite_pkg_dir, app_dir):
    global _app_dir
    _app_dir = app_dir
    if zkite_pkg_dir not in sys.path:
        sys.path.insert(0, zkite_pkg_dir)
    if app_dir not in sys.path:
        sys.path.insert(0, app_dir)
    # Here rather than at module level, because this is the first point at which a
    # writable directory is known — the bundle is cached beside the unpacked app.
    _ca = _configure_ca_certificates(os.path.dirname(app_dir) or app_dir)
    if _ca:
        print("zkite: TLS trust store %s" % _ca)
    main_py = _find_main(app_dir)
    try:
        if main_py is None:
            return _tree_frame(_WAITING_TREE)
        return _boot_app_tree(main_py)
    except Exception as exc:  # glue-level failure, not a user `view()` bug
        return _glue_error_tree(exc)


def _zkite_dispatch_tree(node_id, event, payload):
    if _app is None:
        return None
    try:
        handled = _app.dispatch(node_id, event, payload)
    except Exception:
        return _glue_error_tree(None)
    if not handled:
        return None
    return _tree_frame(_app.last_tree)


def _zkite_back_tree():
    """A platform Back press, tree backend (phase_3/PLAN.md §16).

    The contract both shells are written against, and the reason this decision is
    made in Python rather than in Java: *only* the current render knows whether
    anything wanted the press. `App.dispatch_back` walks the back chain the last
    render registered (`RenderContext.dispatch_back`), innermost handler first.

    Returns the new tree as JSON if the press was consumed — a navigator will
    already have re-rendered by then, since its `update()` fires the render
    callback — or `None` for "nothing wanted it", which the shell must turn into
    the platform default of finishing the activity. That `None` is load-bearing:
    an app with no navigation at all has an empty back chain, so Back keeps
    exiting exactly as it always has, and nothing regresses for a one-screen app.
    """
    if _app is None:
        return None
    try:
        if not _app.dispatch_back():
            return None
        return _tree_frame(_app.last_tree)
    except Exception:
        return _glue_error_tree(None)


def _zkite_reload_tree():
    """Tree-mode twin of `_zkite_reload` — the native backend's hot reload
    (phase_3/PLAN.md §15).

    Identical in every respect that matters: the same `reload_module` +
    `use_module` class-swap that preserves `@store` state across a reload
    (phase_1/PLAN.md §4), against the same `App` instance. Only the last line
    differs, because this backend consumes a tree rather than an HTML string —
    which is the whole reason `zkite dev` works on both backends without
    `zkite_py` learning that there is more than one.

    Returns the new tree as JSON. Unlike `_zkite_dispatch_tree`, never `None`:
    a reload always produces a frame, even if that frame is the error overlay.
    """
    global _app, _dev_connected
    _dev_connected = True
    main_py = _find_main(_app_dir)
    try:
        if main_py is None:
            return _tree_frame(_WAITING_TREE)
        if _app is None:
            return _boot_app_tree(main_py)
        from zkite.reload import reload_module

        _purge_app_bytecode_cache(_app_dir)
        result = reload_module(main_py, "main")
        _app.use_module(result.module)
        return _tree_frame(_app.render_tree())
    except Exception:
        return _glue_error_tree(None)


def _zkite_scroll_tree(node_id, scroll_y, viewport_h):
    """Report where a scrollable container currently is, and re-render only if
    that changes which rows a virtualised `List` has to materialise.

    Called on *every* frame a scroll offset changes (phase_3/PLAN.md §11), so the
    common case has to be cheap and has to run no user code: a plain `ScrollView`,
    or a `List` scrolled less than one row, returns `None` after two dict lookups
    and a little integer arithmetic. `view()` re-runs only when a window actually
    moved — which is also why the row `builder` isn't called 60 times a second to
    discover that nothing changed.

    Returns the new tree as JSON, or `None` for "nothing to do, keep painting the
    tree you already have" — the same contract `_zkite_dispatch_tree` already uses
    for a tap that hit nothing.
    """
    if _app is None:
        return None
    try:
        from zkite import _viewport

        if not _viewport.set_viewport(node_id, float(scroll_y), float(viewport_h)):
            return None
        if not _viewport.window_would_change(node_id):
            return None
        return _tree_frame(_app.render_tree())
    except Exception:
        return _glue_error_tree(None)
