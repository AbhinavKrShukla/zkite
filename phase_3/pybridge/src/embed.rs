//! Embeds CPython + `zkite_py`, calling the tree-mode glue functions in
//! `phase_1/devshell/src/runtime.py` (`_zkite_setup_tree`/`_zkite_dispatch_tree`)
//! instead of the HTML ones `phase_1/devshell`/`phase_2/appshell` call — same
//! embedding (`Py_InitializeFromConfig`, `config.home` not `PYTHONHOME`, `pyo3-ffi`
//! not the higher-level `pyo3` crate — see that crate's own lib.rs for why:
//! PYO3_CONFIG_FILE-driven cross-compilation, no build-script host-interpreter
//! probing), same `runtime.py` (shared via the same relative `include_str!`, so a fix
//! to one glue function's shape is visible to every backend that embeds it), just two
//! different entry points. See ../../PLAN.md §5.
//!
//! Deliberately has no Android/JNI/logging dependency of its own — `setup`/`dispatch`
//! take a `log` callback instead, so this crate stays testable on the host and reusable
//! by any future native shell, not just `phase_3/gpu_harness`.

use std::cell::Cell;
use std::ffi::CString;
use std::path::Path;
use std::sync::{Mutex, Once};

use pyo3_ffi as py;

const RUNTIME_GLUE: &str = include_str!("../../../phase_1/devshell/src/runtime.py");

static INIT: Once = Once::new();
static PYTHON_HOME: Mutex<Option<CString>> = Mutex::new(None);

/// Where a *pushed* frame goes — installed by the shell, called from whatever thread
/// Python happened to be on when a `@store.update()` produced it. `None` (the default)
/// makes the whole push path an inert no-op, which is what keeps every host test and
/// every shell that has not opted in behaving exactly as it did before (../PLAN.md
/// §34.2).
type FrameSink = Box<dyn Fn(&str) + Send + Sync>;
static FRAME_SINK: Mutex<Option<FrameSink>> = Mutex::new(None);

thread_local! {
    /// Whether *this* thread is currently inside one of this module's own calls into
    /// Python — i.e. it asked for a frame and is going to paint whatever comes back.
    ///
    /// **This is the whole of the re-entrancy answer, and it has to be per-thread.**
    /// Every pull (`dispatch`, `back`, `reload`, `scroll`, `setup`) ends in a
    /// `render_tree()`/`render()` that now fires `App`'s push callback, so without
    /// this every tap would notify the sink about a frame its caller is already
    /// holding — a second full parse, layout and paint per touch.
    ///
    /// A process-global flag would be wrong rather than merely coarse: CPython
    /// switches threads every `sys.getswitchinterval()` (5ms by default) *inside* a
    /// call, so a background thread's `update()` can genuinely run while the render
    /// thread sits in `PyObject_CallObject`. A global would read "true" and drop that
    /// frame on the floor — the exact silent-nothing-repaints bug this section exists
    /// to fix, reintroduced one layer down. The GIL serialises execution; it does not
    /// make this call stack the only one.
    static IN_PULL: Cell<bool> = const { Cell::new(false) };
}

/// Install (or clear, with `None`) the sink that receives pushed frames.
///
/// The string is a complete frame in whichever representation this shell consumes —
/// tree JSON for `phase_3/gpu_harness`, an HTML string for the WebView shells —
/// because `runtime.py` registers the push callback that matches the mode the app is
/// already running in. Both are just "the newest frame", so a sink is free to drop an
/// older one it has not consumed yet (`gpu_harness` does exactly that, see
/// `Command::FrameReady`).
///
/// Deliberately a plain callback rather than a channel: this crate has no opinion
/// about how a shell wants to be woken (an `mpsc` send on native, a JNI
/// `AttachCurrentThread` on the WebView), and it is the same "no Android dependency
/// of its own" posture the `log` callbacks already take.
pub fn set_frame_sink(sink: Option<FrameSink>) {
    *FRAME_SINK.lock().unwrap() = sink;
}

/// Run `f` marked as a pull, so any frame it produces is not also pushed.
fn pull<T>(f: impl FnOnce() -> T) -> T {
    IN_PULL.with(|p| p.set(true));
    let out = f();
    IN_PULL.with(|p| p.set(false));
    out
}

/// `_zkite_native_frame_ready(frame)` as Python sees it — `METH_O`, so `arg` is the
/// single string argument directly rather than a tuple.
///
/// Runs with the GIL held (it is being *called* from Python) on whatever thread did
/// the `@store.update()`. Everything it does is therefore deliberately small and
/// non-blocking: copy the string out, hand it to the sink, return `None`. A sink that
/// blocks here blocks the app's Python thread.
unsafe extern "C" fn frame_ready(_self: *mut py::PyObject, arg: *mut py::PyObject) -> *mut py::PyObject {
    if !IN_PULL.with(|p| p.get()) {
        let mut len: py::Py_ssize_t = 0;
        let utf8 = py::PyUnicode_AsUTF8AndSize(arg, &mut len);
        if utf8.is_null() {
            // A non-string argument would have set an exception; clear it rather than
            // letting it surface out of an unrelated `update()` deep in user code.
            py::PyErr_Clear();
        } else {
            let bytes = std::slice::from_raw_parts(utf8 as *const u8, len as usize);
            let frame = String::from_utf8_lossy(bytes);
            if let Some(sink) = FRAME_SINK.lock().unwrap().as_ref() {
                sink(&frame);
            }
        }
    }
    let none = py::Py_None();
    py::Py_IncRef(none);
    none
}

/// Define `_zkite_native_frame_ready` in `__main__`, for `runtime.py`'s
/// `_zkite_push_frame` to find. Called once, with the GIL held, during init.
///
/// The `PyMethodDef` is leaked on purpose: CPython keeps a borrowed pointer to it for
/// the life of the function object, which here is the life of the process.
unsafe fn install_frame_hook() {
    let def = Box::leak(Box::new(py::PyMethodDef {
        ml_name: c"_zkite_native_frame_ready".as_ptr(),
        ml_meth: py::PyMethodDefPointer { PyCFunction: frame_ready },
        ml_flags: py::METH_O,
        ml_doc: c"Hand a freshly rendered frame to the shell (phase_3/PLAN.md §34).".as_ptr(),
    }));
    let func = py::PyCFunction_NewEx(def, std::ptr::null_mut(), std::ptr::null_mut());
    if func.is_null() {
        py::PyErr_Clear();
        return;
    }
    py::PyDict_SetItemString(main_dict(), c"_zkite_native_frame_ready".as_ptr(), func);
    py::Py_DecRef(func);
}

/// See `phase_0/spike1_cpython_embed`'s identically-named function: `config.home`,
/// not `PYTHONHOME`, is what actually locates the stdlib for an embedded, isolated
/// interpreter.
fn set_python_home(prefix: &Path) {
    if let Ok(c) = CString::new(prefix.as_os_str().as_encoded_bytes()) {
        *PYTHON_HOME.lock().unwrap() = Some(c);
    }
    std::env::set_var("PYTHONHOME", prefix);
    std::env::set_var("PYTHONNOUSERSITE", "1");
    std::env::set_var("PYTHONUNBUFFERED", "1");
}

fn with_gil<T>(f: impl FnOnce() -> T) -> T {
    unsafe {
        let gil = py::PyGILState_Ensure();
        let out = f();
        py::PyGILState_Release(gil);
        out
    }
}

/// Runs `Py_InitializeFromConfig` exactly once, then defines the `_zkite_*` glue
/// functions in `__main__`. Safe to call from any thread (including one CPython
/// didn't create itself, like `gpu_harness`'s dedicated render thread — that's
/// exactly what `PyGILState_Ensure`/`Release` are for); only the first call does
/// anything, but the calling thread must have already called `set_python_home` via
/// `setup`.
fn ensure_initialized() -> Result<(), String> {
    let mut init_err: Option<String> = None;
    INIT.call_once(|| unsafe {
        let mut config: py::PyConfig = std::mem::zeroed();
        py::PyConfig_InitIsolatedConfig(&mut config);
        config.install_signal_handlers = 0;
        // Let CPython cache bytecode in the app's private directory. This was `0`
        // for three phases with no comment saying why, and it was costing more than
        // anything else in cold start: the pruned stdlib ships as ~490 *source*
        // files (`prune_stdlib.py` drops `.pyc` outright), so with writing disabled
        // every launch re-parsed all of them, forever. Measured on the Moto G54,
        // same app, same session, this line the only difference: **2584ms -> ~990ms
        // to first frame**. The one-time cost is a slower first launch (4362ms,
        // which also unpacks 577 files); break-even is the second launch. See
        // ../../PLAN.md §17.1.
        //
        // Fails safe: if the directory is unwritable CPython silently skips the
        // cache and behaves exactly as it did before. `zkite dev` is unaffected — a
        // pushed file has a new mtime, so it is recompiled, which is the ordinary
        // invalidation rule and not something this has to special-case.
        config.write_bytecode = 1;

        let home = PYTHON_HOME.lock().unwrap().clone();
        match home {
            Some(home) => {
                let st = py::PyConfig_SetBytesString(&mut config, &mut config.home, home.as_ptr());
                if py::PyStatus_Exception(st) != 0 {
                    init_err = Some("PyConfig_SetBytesString(home) failed".to_string());
                }
            }
            None => init_err = Some("set_python_home() was not called".into()),
        }

        if init_err.is_none() {
            let status = py::Py_InitializeFromConfig(&config);
            if py::PyStatus_Exception(status) != 0 {
                init_err = Some("Py_InitializeFromConfig failed".into());
            }
        }
        py::PyConfig_Clear(&mut config);

        if init_err.is_none() {
            // Interpreter init leaves the GIL held by this thread already.
            let src = CString::new(RUNTIME_GLUE).unwrap();
            if py::PyRun_SimpleString(src.as_ptr()) != 0 {
                init_err = Some(format!("runtime.py failed to load: {}", take_error()));
            }
            // After the glue, so `_zkite_push_frame` is already defined and can look
            // this up by name at call time; before any app boots, so the very first
            // `_zkite_setup_tree` can register the push callback.
            if init_err.is_none() {
                install_frame_hook();
            }
            py::PyEval_SaveThread(); // release the GIL we're implicitly holding
        }
    });
    match init_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

unsafe fn take_error() -> String {
    if py::PyErr_Occurred().is_null() {
        return "no exception set".into();
    }
    py::PyErr_Print();
    "see logcat".into()
}

unsafe fn main_dict() -> *mut py::PyObject {
    let name = CString::new("__main__").unwrap();
    let m = py::PyImport_AddModule(name.as_ptr());
    py::PyModule_GetDict(m)
}

/// Call a zero/N-arg function defined in `__main__` by name, with plain-string args,
/// and read back a string-or-None result. Identical in shape to
/// `phase_1/devshell/src/lib.rs`'s helper of the same name — every `_zkite_*` entry
/// point in runtime.py, tree-mode included, has this shape.
unsafe fn call_str_fn(name: &str, args: &[&str]) -> Result<Option<String>, String> {
    let dict = main_dict();
    let cname = CString::new(name).unwrap();
    let func = py::PyDict_GetItemString(dict, cname.as_ptr()); // borrowed
    if func.is_null() {
        return Err(format!("runtime.py glue function {name} is missing"));
    }
    let tuple = py::PyTuple_New(args.len() as py::Py_ssize_t);
    for (i, a) in args.iter().enumerate() {
        let item = match CString::new(*a) {
            Ok(c) => py::PyUnicode_FromString(c.as_ptr()),
            Err(_) => py::PyUnicode_FromString(c"".as_ptr()),
        };
        py::PyTuple_SetItem(tuple, i as py::Py_ssize_t, item); // steals the ref
    }
    let result = py::PyObject_CallObject(func, tuple);
    py::Py_DecRef(tuple);

    if result.is_null() {
        return Err(format!("{name}() raised: {}", take_error()));
    }
    if result == py::Py_None() {
        py::Py_DecRef(result);
        return Ok(None);
    }
    let mut len: py::Py_ssize_t = 0;
    let utf8 = py::PyUnicode_AsUTF8AndSize(result, &mut len);
    let out = if utf8.is_null() {
        None
    } else {
        let bytes = std::slice::from_raw_parts(utf8 as *const u8, len as usize);
        Some(String::from_utf8_lossy(bytes).into_owned())
    };
    py::Py_DecRef(result);
    Ok(out)
}

/// Flush whatever the app has printed since the last drain.
///
/// Public because the push path has no call of its own to drain as a side effect: a
/// background thread's `print()` — the only debugging instrument such a thread has —
/// otherwise accumulates in `runtime.py`'s stdout tee until the user next touches the
/// screen. Every other entry point here drains as part of the call it already made.
pub fn drain_log(log: &dyn Fn(&str)) {
    drain_output(log);
}

fn drain_output(log: &dyn Fn(&str)) {
    let text = with_gil(|| unsafe { call_str_fn("_zkite_drain_output", &[]) })
        .ok()
        .flatten()
        .unwrap_or_default();
    for line in text.lines() {
        if !line.is_empty() {
            log(line);
        }
    }
}

/// Boots the interpreter (once total per process — `runtime.py`'s own `_app` global
/// means a second call just re-runs `main.py` fresh) and returns the initial tree as
/// JSON, matching `RawNode`'s shape (`../lib.rs`). `log` receives each line the glue
/// (or the app's `view()`) printed via stdout/stderr, so this crate can stay Android-
/// agnostic while the caller routes it through `__android_log_write` or similar.
pub fn setup(prefix: &Path, zkite_pkg_dir: &str, app_dir: &str, log: &dyn Fn(&str)) -> Result<String, String> {
    set_python_home(prefix);
    ensure_initialized()?;
    let result = pull(|| with_gil(|| unsafe { call_str_fn("_zkite_setup_tree", &[zkite_pkg_dir, app_dir]) }));
    drain_output(log);
    match result {
        Ok(Some(json)) => Ok(json),
        Ok(None) => Err("_zkite_setup_tree returned None".into()),
        Err(e) => Err(e),
    }
}

/// Dispatches a UI event by `zkite_py` node id. `None` means nothing had a handler
/// registered for `(node_id, event)` — an ordinary, expected outcome (most taps don't
/// land on an interactive node's exact id), not an error; the caller should just skip
/// the redraw it would otherwise do.
pub fn dispatch(node_id: &str, event: &str, payload: &str, log: &dyn Fn(&str)) -> Option<String> {
    let result = pull(|| with_gil(|| unsafe { call_str_fn("_zkite_dispatch_tree", &[node_id, event, payload]) }));
    drain_output(log);
    match result {
        Ok(json) => json,
        Err(e) => {
            log(&format!("dispatch failed: {e}"));
            None
        }
    }
}

/// Re-executes the app's `main.py` from disk and returns the new tree as JSON —
/// `zkite dev`'s hot reload for this backend (`../../PLAN.md` §15). `@store` state
/// survives via the class-swap `zkite.reload` has done since phase 1; nothing here is
/// native-specific except that a tree comes back instead of an HTML string.
///
/// Always returns a tree (never `None`, unlike `dispatch`): a reload always produces
/// a frame, and if `view()` raised, that frame is the error overlay — which is the
/// point, since the alternative is a stale screen that looks like the edit did
/// nothing.
pub fn reload(log: &dyn Fn(&str)) -> Result<String, String> {
    let result = pull(|| with_gil(|| unsafe { call_str_fn("_zkite_reload_tree", &[]) }));
    drain_output(log);
    match result {
        Ok(Some(json)) => Ok(json),
        Ok(None) => Err("_zkite_reload_tree returned None".into()),
        Err(e) => Err(e),
    }
}

/// Offers a platform Back press to the app, returning a fresh tree as JSON if
/// navigation consumed it and `None` if nothing wanted it (../../PLAN.md §16).
///
/// `None` is the whole reason this returns an `Option` rather than a tree: it means
/// the shell must do the platform default and finish the activity. An app with no
/// `Nav` in it registers no back handlers at all, so Back keeps exiting exactly as it
/// did before navigation existed — the one-screen case is unchanged by construction
/// rather than by a special case.
pub fn back(log: &dyn Fn(&str)) -> Option<String> {
    let result = pull(|| with_gil(|| unsafe { call_str_fn("_zkite_back_tree", &[]) }));
    drain_output(log);
    match result {
        Ok(json) => json,
        Err(e) => {
            // A failure to *ask* is not consent to exit: treat it as unconsumed so
            // Back still does the platform thing rather than becoming a dead key.
            log(&format!("back failed: {e}"));
            None
        }
    }
}

/// The last render's traceback, if `view()` raised. `runtime.py`'s `_zkite_last_error`
/// is backend-agnostic (it reads `App.last_error`), so this is the same call the
/// WebView shell makes — it exists so `zkite dev` can mirror a traceback into the host
/// terminal as well as the on-device overlay, which plan1.md §6 asks for explicitly
/// ("simultaneously in the terminal").
pub fn last_error() -> Option<String> {
    with_gil(|| unsafe { call_str_fn("_zkite_last_error", &[]) }).ok().flatten()
}

/// A short content hash of the `zkite` package this shell shipped, or `None` if it
/// could not be computed (../PLAN.md §31.6).
///
/// The Dev Shell pins the framework version — `zkite dev` pushes only the project's
/// own `.py` files, while `zkite/` lives inside the shell's `zkite-framework.zip` — so
/// a shell built before a widget existed meets the user as a bare `ImportError` with
/// nothing naming the cause. `zkite dev` puts this in the `hello` frame and compares it
/// against the payload's own, turning that into one line naming the rebuild command.
///
/// Called once, at setup, rather than from the reload socket: `reload.rs`'s whole
/// design is that the socket thread never touches Python (see its module doc), and a
/// value that cannot change while the process lives has no reason to be recomputed.
pub fn framework_fingerprint(zkite_pkg_dir: &str) -> Option<String> {
    with_gil(|| unsafe { call_str_fn("_zkite_framework_fingerprint", &[zkite_pkg_dir]) }).ok().flatten()
}

/// Reports a scrollable container's current scroll position and visible height to
/// `zkite_py`, returning a fresh tree as JSON **only** if that changed which rows a
/// virtualised `List` has to materialise (../PLAN.md §11). `None` — the overwhelmingly
/// common case, including every scroll of a plain non-virtualised `ScrollView` — means
/// "nothing changed, keep painting the tree you already have."
///
/// Unlike `setup`/`dispatch`, this deliberately does **not** call `drain_output` unless
/// a tree actually came back. This runs on every frame of every drag and fling, and
/// draining is a second full Python call; paying for it 60 times a second to collect
/// output that a no-op scroll cannot have produced would put a measurable cost on the
/// hot path for nothing. Anything the app *does* print while its `view()` re-runs is
/// still drained, because that only happens on the path that returns a tree.
pub fn scroll(node_id: &str, scroll_y: f32, viewport_h: f32, log: &dyn Fn(&str)) -> Option<String> {
    let sy = scroll_y.to_string();
    let vh = viewport_h.to_string();
    let result = pull(|| with_gil(|| unsafe { call_str_fn("_zkite_scroll_tree", &[node_id, &sy, &vh]) }));
    match result {
        Ok(Some(json)) => {
            drain_output(log);
            Some(json)
        }
        Ok(None) => None,
        Err(e) => {
            log(&format!("scroll failed: {e}"));
            None
        }
    }
}
