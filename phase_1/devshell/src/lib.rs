//! Phase 1, M4 — the Dev Shell: embeds CPython + `zkite_py`, bridges a WebView.
//!
//! Extends phase_0/spike1_cpython_embed's embedding (the `config.home` finding and
//! the `pyo3-ffi` cross-compile approach carry over unchanged — see that spike's
//! README) with the actual application logic, which lives almost entirely in
//! `runtime.py` (embedded via `include_str!`) and the already-built, already-tested
//! `zkite_py` package (../../zkite_py). Rust's job here is thin: JNI plumbing, GIL
//! bookkeeping, and the dev-loop reload socket. See ../../PLAN.md M4/M5.
//!
//! Design choice worth calling out: rather than having a background thread call back
//! into Java asynchronously (which needs `AttachCurrentThread`/global refs/method-ID
//! lookups — real JNI machinery with real ways to get wrong), a file-triggered reload
//! just parks its result in `PENDING_HTML` and the Java side polls for it on a short
//! timer. At the ~100ms poll interval Java uses, this is comfortably inside plan1.md
//! §6's <200ms reload budget and removes an entire class of cross-thread JNI bugs for
//! a one-session build. Tap-triggered dispatch needs none of this: it's already a
//! synchronous Java->Rust call, so the new HTML is just its return value.

#[cfg_attr(not(target_os = "android"), allow(unused_imports))]
use std::ffi::{c_char, c_int, CStr, CString};
mod managed_files;

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::{Mutex, Once};
use std::time::Duration;

use pyo3_ffi as py;
use serde::{Deserialize, Serialize};

const TAG: &[u8] = b"ZKiteDevShell\0";
const RUNTIME_GLUE: &str = include_str!("runtime.py");

#[cfg(target_os = "android")]
extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}
const ANDROID_LOG_INFO: c_int = 4;
const ANDROID_LOG_WARN: c_int = 5;
const ANDROID_LOG_ERROR: c_int = 6;

pub fn log(prio: c_int, msg: &str) {
    #[cfg(target_os = "android")]
    {
        if let Ok(c) = CString::new(msg) {
            unsafe { __android_log_write(prio, TAG.as_ptr() as *const c_char, c.as_ptr()) };
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = prio;
        eprintln!("[{}] {}", String::from_utf8_lossy(&TAG[..TAG.len() - 1]), msg);
    }
}
macro_rules! logi { ($($t:tt)*) => { crate::log(ANDROID_LOG_INFO, &format!($($t)*)) }; }
macro_rules! logw { ($($t:tt)*) => { crate::log(ANDROID_LOG_WARN, &format!($($t)*)) }; }
macro_rules! loge { ($($t:tt)*) => { crate::log(ANDROID_LOG_ERROR, &format!($($t)*)) }; }

static INIT: Once = Once::new();
static PYTHON_HOME: Mutex<Option<CString>> = Mutex::new(None);
static PENDING_HTML: Mutex<Option<String>> = Mutex::new(None);

// ------------------------------------------------------------------ the push path
//
// phase_3/PLAN.md §34. The Dev Shell's half is the cheap one, and only because the
// mechanism it needs already exists for another reason: `PENDING_HTML` plus Java's
// `pollHtml` timer was built so the *reload socket* could deliver a frame with no
// Java call to return through (phase_1/PLAN.md §4), and an app pushing its own frame
// is precisely the same problem. So a pushed frame parks in the same slot the socket
// already uses and reaches the WebView by the same route.
//
// `phase_2/appshell` — the *shipped* shell — deliberately does **not** do it this
// way: a UI-thread timer running forever is a battery cost a release app should not
// pay for a feature it may never use, so that one makes a real cross-thread JNI call
// instead. Here the timer is already running and the dev loop already depends on it.

thread_local! {
    /// Whether this thread is inside one of this file's own calls into Python, and so
    /// is already going to apply whatever comes back. Per-thread, not global: CPython
    /// switches threads every 5ms *inside* a call, so a worker's `update()` can
    /// genuinely run while another thread sits in `PyObject_CallObject`.
    static IN_PULL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn pull<T>(f: impl FnOnce() -> T) -> T {
    IN_PULL.with(|p| p.set(true));
    let out = f();
    IN_PULL.with(|p| p.set(false));
    out
}

/// `_zkite_native_frame_ready(html)` as Python sees it (`METH_O`).
unsafe extern "C" fn frame_ready(_self: *mut py::PyObject, arg: *mut py::PyObject) -> *mut py::PyObject {
    if !IN_PULL.with(|p| p.get()) {
        let mut len: py::Py_ssize_t = 0;
        let utf8 = py::PyUnicode_AsUTF8AndSize(arg, &mut len);
        if utf8.is_null() {
            py::PyErr_Clear();
        } else {
            let bytes = std::slice::from_raw_parts(utf8 as *const u8, len as usize);
            // Overwrites rather than queues: `poll_html` takes the newest frame and an
            // older one that never reached the DOM has nothing left to contribute —
            // the same coalescing the native backend does with `Command::FrameReady`.
            *PENDING_HTML.lock().unwrap() = Some(String::from_utf8_lossy(bytes).into_owned());
        }
    }
    let none = py::Py_None();
    py::Py_IncRef(none);
    none
}

/// Define `_zkite_native_frame_ready` in `__main__`. Called once, GIL held, at init.
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

/// See phase_0/spike1's identically-named function: `config.home`, not `PYTHONHOME`,
/// is what actually locates the stdlib for an embedded, isolated interpreter.
pub fn set_python_home(prefix: &Path) {
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
/// functions in `__main__`. Safe to call from any thread; only the first call does
/// anything but the initializing thread must have already called `set_python_home`.
fn ensure_initialized() -> Result<(), String> {
    let mut init_err: Option<String> = None;
    INIT.call_once(|| unsafe {
        let mut config: py::PyConfig = std::mem::zeroed();
        py::PyConfig_InitIsolatedConfig(&mut config);
        config.install_signal_handlers = 0;
        // See phase_3/pybridge/src/embed.rs for the rationale and the measurement —
        // the pruned stdlib ships as source, so disabling the bytecode cache
        // re-parsed ~490 files on every launch (measured: 2.3-2.6x slower cold
        // start on both backends). Applied here for parity with the two shipping
        // shells rather than because the Dev Shell's own start-up was measured; a
        // dev-loop launch has exactly the same imports to pay for. See
        // phase_3/PLAN.md §17.1.
        config.write_bytecode = 1;

        let home = PYTHON_HOME.lock().unwrap().clone();
        match home {
            Some(home) => {
                let st = py::PyConfig_SetBytesString(&mut config, &mut config.home, home.as_ptr());
                if py::PyStatus_Exception(st) != 0 {
                    init_err = Some(format!("PyConfig_SetBytesString(home) failed"));
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
            } else {
                // After the glue, before any app boots — see appshell's twin.
                install_frame_hook();
            }
            py::PyEval_SaveThread(); // release the GIL we're implicitly holding
        }
    });
    match init_err {
        Some(e) => {
            loge!("{e}");
            Err(e)
        }
        None => Ok(()),
    }
}

unsafe fn take_error() -> String {
    if py::PyErr_Occurred().is_null() {
        return "no exception set".into();
    }
    // PyErr_Print also clears it; format via traceback would need an import, so this
    // mirrors spike1: the detail lands in logcat via PyErr_Print's own stderr write,
    // which our Tee (once installed) redirects. Before the Tee exists (glue load
    // itself failing), it goes to the real stderr, which Android drops — acceptable,
    // since the fallback message below still says *something* failed.
    py::PyErr_Print();
    "see logcat".into()
}

unsafe fn main_dict() -> *mut py::PyObject {
    let name = CString::new("__main__").unwrap();
    let m = py::PyImport_AddModule(name.as_ptr());
    py::PyModule_GetDict(m)
}

/// Call a zero/N-arg function defined in `__main__` by name, with plain-string args,
/// and read back a string-or-None result. Every `_zkite_*` entry point in runtime.py
/// has this shape, so one helper covers setup/dispatch/reload/drain.
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

/// Drains the Python stdout/stderr tee, logs each line to logcat, and returns the
/// text so callers with a socket (only `reload`, below) can also forward it to
/// `zkite dev`'s terminal.
fn drain_output() -> String {
    let text = with_gil(|| unsafe { call_str_fn("_zkite_drain_output", &[]) })
        .ok()
        .flatten()
        .unwrap_or_default();
    for line in text.lines() {
        if !line.is_empty() {
            logi!("py: {line}");
        }
    }
    text
}

fn last_error() -> Option<String> {
    with_gil(|| unsafe { call_str_fn("_zkite_last_error", &[]) }).unwrap_or(None)
}

/// A short content hash of the `zkite` package this shell shipped, or `None` if it
/// could not be computed — see `runtime.py::_zkite_framework_fingerprint` and
/// `../../phase_3/PLAN.md` §31.6 for why it is a content hash rather than a version.
///
/// Called from the reload socket's own thread, which is safe here (and *not* in the
/// native shell's twin) because this shell's socket thread already enters the
/// interpreter for every `reload()`; `startReloadServer` runs only after `boot`, so
/// the interpreter is always up by the time a `hello` frame is written.
fn framework_fingerprint() -> Option<String> {
    let dir = ZKITE_PKG_DIR.lock().unwrap().clone()?;
    with_gil(|| unsafe { call_str_fn("_zkite_framework_fingerprint", &[&dir]) }).unwrap_or(None)
}

/// Where `zkite/` was unpacked, recorded by `setup` for `framework_fingerprint`'s use.
/// The glue function is a pure function of this path (see `runtime.py`), so the value
/// has to reach it from the one place that knows it.
static ZKITE_PKG_DIR: Mutex<Option<String>> = Mutex::new(None);

// ---------------------------------------------------------------- public API

pub fn setup(prefix: &Path, zkite_pkg_dir: &str, app_dir: &str) -> Result<String, String> {
    *ZKITE_PKG_DIR.lock().unwrap() = Some(zkite_pkg_dir.to_string());
    set_python_home(prefix);
    ensure_initialized()?;
    let result = pull(|| with_gil(|| unsafe { call_str_fn("_zkite_setup", &[zkite_pkg_dir, app_dir]) }));
    drain_output();
    match result {
        Ok(Some(html)) => Ok(html),
        Ok(None) => Err("_zkite_setup returned None".into()),
        Err(e) => Err(e),
    }
}

pub fn dispatch(node_id: &str, event: &str, payload: &str) -> Option<String> {
    let result = pull(|| with_gil(|| unsafe { call_str_fn("_zkite_dispatch", &[node_id, event, payload]) }));
    drain_output();
    match result {
        Ok(html) => html,
        Err(e) => {
            loge!("dispatch failed: {e}");
            None
        }
    }
}

/// Offers a platform Back press to `zkite_py`'s navigation stack
/// (phase_3/PLAN.md §16). `Some(html)` if navigation consumed it, `None` if nothing
/// wanted it and the caller should finish the Activity.
pub fn handle_back() -> Option<String> {
    let result = pull(|| with_gil(|| unsafe { call_str_fn("_zkite_back", &[]) }));
    drain_output();
    match result {
        Ok(html) => html,
        Err(e) => {
            // Failing to ask is not consent to exit — treat it as unconsumed so Back
            // still does the platform thing rather than becoming a dead key.
            loge!("handle_back failed: {e}");
            None
        }
    }
}

/// What a file-push-triggered reload produced: the new HTML (if render succeeded at
/// all — even an error overlay counts, since `zkite_py`'s `App.render()` always
/// returns *something* to show), anything printed during it, and the traceback if
/// `view()` raised. `handle_connection` forwards `output`/`error` to `zkite dev`'s
/// terminal — see plan1.md §6, "simultaneously in the terminal".
struct ReloadOutcome {
    html: Option<String>,
    output: String,
    error: Option<String>,
}

fn reload() -> ReloadOutcome {
    let result = pull(|| with_gil(|| unsafe { call_str_fn("_zkite_reload", &[]) }));
    let output = drain_output();
    let error = last_error();
    let html = match result {
        Ok(html) => html,
        Err(e) => {
            loge!("reload failed: {e}");
            None
        }
    };
    ReloadOutcome { html, output, error }
}

pub fn poll_html() -> Option<String> {
    PENDING_HTML.lock().unwrap().take()
}

// ------------------------------------------------------------ dev-loop reload socket

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum Frame {
    Sync { paths: Vec<String> },
    File { path: String, content_b64: String },
    Reload,
}

/// Frames sent back to `zkite dev` — the counterpart of `cli/src/dev_cmd.rs`'s
/// `InFrame`. Kept as plain, hand-matched JSON (not a shared crate) for the same
/// reason as `Frame`: the two sides cross-compile for different targets.
#[derive(Serialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum OutFrame<'a> {
    /// Sent once on connect so `zkite dev` can tell which backend reached it — see
    /// `phase_3/gpu_harness/src/reload.rs`'s twin and `phase_3/PLAN.md` §15.6.
    ///
    /// `framework` is this shell's `zkite` package fingerprint, which `zkite dev`
    /// compares against the payload's to catch a Dev Shell that pins an older
    /// framework than the one being developed against (`phase_3/PLAN.md` §31.6).
    /// Skipped when absent so an older `zkite dev` still parses the frame.
    Hello {
        backend: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        framework: Option<String>,
        sync_files: bool,
    },
    Log { text: &'a str },
    Error { text: &'a str },
}

/// Connects out to `host:port` (reached via `adb reverse tcp:<port> tcp:<port>`,
/// established by `zkite dev`), so the *device* is the TCP client — see ../PLAN.md §4
/// for why that's the right direction for `adb reverse`. Retries forever on
/// disconnect: the dev server may not be up yet, or may restart.
pub fn start_reload_server(host: String, port: u16, app_dir: String) {
    std::thread::spawn(move || loop {
        match TcpStream::connect((host.as_str(), port)) {
            Ok(stream) => {
                logi!("reload socket connected to {host}:{port}");
                handle_connection(stream, &app_dir);
                logw!("reload socket disconnected, retrying");
            }
            Err(_) => {}
        }
        std::thread::sleep(Duration::from_millis(500));
    });
}

fn handle_connection(stream: TcpStream, app_dir: &str) {
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let hello = OutFrame::Hello { backend: "webview", framework: framework_fingerprint(), sync_files: true };
    if let Ok(f) = serde_json::to_string(&hello) {
        let _ = writeln!(writer, "{f}");
    }
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = match line {
            Ok(l) if !l.trim().is_empty() => l,
            Ok(_) => continue,
            Err(_) => return,
        };
        let frame: Frame = match serde_json::from_str(&line) {
            Ok(f) => f,
            Err(e) => {
                loge!("bad reload frame: {e}");
                continue;
            }
        };
        match frame {
            Frame::Sync { paths } => {
                if let Err(error) = managed_files::sync(Path::new(app_dir), &paths) {
                    if let Ok(frame) = serde_json::to_string(&OutFrame::Error { text: &error }) {
                        let _ = writeln!(writer, "{frame}");
                    }
                    return;
                }
            }
            Frame::File { path, content_b64 } => match apply_file(app_dir, &path, &content_b64) {
                Ok(()) => logi!("wrote {path}"),
                Err(e) => loge!("could not write {path}: {e}"),
            },
            Frame::Reload => {
                let outcome = reload();
                if let Some(html) = outcome.html {
                    *PENDING_HTML.lock().unwrap() = Some(html);
                }
                if !outcome.output.trim().is_empty() {
                    if let Ok(f) = serde_json::to_string(&OutFrame::Log { text: &outcome.output }) {
                        let _ = writeln!(writer, "{f}");
                    }
                }
                if let Some(err) = &outcome.error {
                    if let Ok(f) = serde_json::to_string(&OutFrame::Error { text: err }) {
                        let _ = writeln!(writer, "{f}");
                    }
                }
                let _ = writeln!(writer, "{{\"ok\":true}}");
            }
        }
    }
}

fn apply_file(app_dir: &str, rel_path: &str, content_b64: &str) -> Result<(), String> {
    let dest = managed_files::destination(Path::new(app_dir), rel_path)?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = base64_decode(content_b64).map_err(|e| e.to_string())?;
    std::fs::write(&dest, bytes).map_err(|e| e.to_string())
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------- JNI surface

#[cfg(target_os = "android")]
mod jni_bridge {
    use super::*;

    unsafe fn jstr_to_string(env: *mut jni_sys::JNIEnv, s: jni_sys::jstring) -> String {
        if s.is_null() {
            return String::new();
        }
        let jni = **env;
        let raw = (jni.GetStringUTFChars.unwrap())(env, s, std::ptr::null_mut());
        if raw.is_null() {
            return String::new();
        }
        let out = CStr::from_ptr(raw).to_string_lossy().into_owned();
        (jni.ReleaseStringUTFChars.unwrap())(env, s, raw);
        out
    }

    unsafe fn string_to_jstr(env: *mut jni_sys::JNIEnv, s: &str) -> jni_sys::jstring {
        let jni = **env;
        match CString::new(s) {
            Ok(c) => (jni.NewStringUTF.unwrap())(env, c.as_ptr()),
            Err(_) => std::ptr::null_mut(),
        }
    }

    unsafe fn opt_string_to_jstr(env: *mut jni_sys::JNIEnv, s: Option<String>) -> jni_sys::jstring {
        match s {
            Some(s) => string_to_jstr(env, &s),
            None => std::ptr::null_mut(),
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_devshell_Native_boot(
        env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        prefix: jni_sys::jstring,
        zkite_pkg_dir: jni_sys::jstring,
        app_dir: jni_sys::jstring,
    ) -> jni_sys::jstring {
        let prefix = jstr_to_string(env, prefix);
        let zkite_pkg_dir = jstr_to_string(env, zkite_pkg_dir);
        let app_dir = jstr_to_string(env, app_dir);
        logi!("boot(prefix={prefix}, zkite_pkg_dir={zkite_pkg_dir}, app_dir={app_dir})");
        match crate::setup(Path::new(&prefix), &zkite_pkg_dir, &app_dir) {
            Ok(html) => string_to_jstr(env, &html),
            Err(e) => {
                loge!("boot failed: {e}");
                string_to_jstr(env, &format!("<div id=\"r\">boot failed: {e}</div>"))
            }
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_devshell_Native_dispatch(
        env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        id: jni_sys::jstring,
        event: jni_sys::jstring,
        payload: jni_sys::jstring,
    ) -> jni_sys::jstring {
        let id = jstr_to_string(env, id);
        let event = jstr_to_string(env, event);
        let payload = jstr_to_string(env, payload);
        opt_string_to_jstr(env, crate::dispatch(&id, &event, &payload))
    }

    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_devshell_Native_handleBack(
        env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
    ) -> jni_sys::jstring {
        opt_string_to_jstr(env, crate::handle_back())
    }

    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_devshell_Native_pollHtml(
        env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
    ) -> jni_sys::jstring {
        opt_string_to_jstr(env, crate::poll_html())
    }

    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_devshell_Native_startReloadServer(
        env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        host: jni_sys::jstring,
        port: jni_sys::jint,
        app_dir: jni_sys::jstring,
    ) {
        let host = jstr_to_string(env, host);
        let app_dir = jstr_to_string(env, app_dir);
        crate::start_reload_server(host, port as u16, app_dir);
    }
}
