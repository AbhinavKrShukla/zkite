//! The release App Shell — `../PLAN.md` §1.2: this is `phase_1/devshell/src/lib.rs`
//! with the dev-loop reload socket (`start_reload_server`, `handle_connection`,
//! `apply_file`, `PENDING_HTML`, the `Frame`/`OutFrame` wire types, and the
//! `pollHtml`/`startReloadServer` JNI entry points) deleted. Everything else —
//! interpreter init, `config.home` (not `PYTHONHOME`), the `_zkite_*` glue calling
//! convention, `boot`/`dispatch` — is unchanged, because none of it was ever
//! dev-loop-specific; only the reload socket was. See `runtime.py`, reused from
//! `phase_1/devshell` byte-for-byte (`../PLAN.md` §1.2 finding: `_zkite_setup` already
//! boots straight into the app if `app_dir/main.py` exists — a release build just
//! guarantees that file is always there before `boot()` is ever called, via
//! `ZKiteActivity`'s one-time `app.zip` unpack instead of a socket push).

#[cfg_attr(not(target_os = "android"), allow(unused_imports))]
use std::ffi::{c_char, c_int, CStr, CString};
use std::path::Path;
use std::sync::{Mutex, Once};

use pyo3_ffi as py;

const TAG: &[u8] = b"ZKiteAppShell\0";
const RUNTIME_GLUE: &str = include_str!("../../../phase_1/devshell/src/runtime.py");

#[cfg(target_os = "android")]
extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}
const ANDROID_LOG_INFO: c_int = 4;
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
macro_rules! loge { ($($t:tt)*) => { crate::log(ANDROID_LOG_ERROR, &format!($($t)*)) }; }

static INIT: Once = Once::new();
static PYTHON_HOME: Mutex<Option<CString>> = Mutex::new(None);

// ------------------------------------------------------------------ the push path
//
// phase_3/PLAN.md §34. Until this existed every frame in ZKite was *pulled*: Java
// called `boot`/`dispatch`/`handleBack` and applied the HTML that came back, so a
// `@store.update()` from a Python worker thread — a finished network fetch, a timer —
// refreshed `App.last_html` and **nothing ever drew it**.
//
// The three pieces below are `phase_3/pybridge/src/embed.rs`'s, deliberately
// duplicated rather than shared: this crate cannot depend on `zkite_pybridge` (that
// would pull `zkite_renderer`, taffy and a whole GPU stack into a WebView shell), and
// it is the same duplication `with_gil`, `call_str_fn` and `ensure_initialized`
// already are between these two files. `runtime.py` — the half that decides *when* a
// frame is pushed — is genuinely shared, via the same `include_str!`, which is where
// the behaviour that must not diverge actually lives.

thread_local! {
    /// Whether this thread is inside one of this file's own calls into Python, and so
    /// is already going to apply whatever comes back. Per-thread rather than global:
    /// CPython switches threads every 5ms *inside* a call, so a worker's `update()`
    /// can genuinely run while the UI thread sits in `PyObject_CallObject`, and a
    /// global flag would drop that frame — the exact bug this exists to fix.
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
            let html = String::from_utf8_lossy(bytes).into_owned();
            push_frame_to_java(&html);
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

#[cfg(not(target_os = "android"))]
fn push_frame_to_java(_html: &str) {}

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
        // See phase_3/pybridge/src/embed.rs for the full rationale and the
        // measurement. Short version: the pruned stdlib ships as ~490 source files,
        // so disabling the bytecode cache re-parsed all of them on every launch.
        // Measured on the Moto G54, same app, same session, this line the only
        // difference: **978ms -> 420ms** to first paint — which puts this backend
        // inside plan1.md §9's "interactive <500ms" target for the first time. See
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
                // After the glue (so `_zkite_push_frame` can find this by name) and
                // before any app boots (so the first `_zkite_setup` can register the
                // push callback).
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
    py::PyErr_Print();
    "see logcat".into()
}

unsafe fn main_dict() -> *mut py::PyObject {
    let name = CString::new("__main__").unwrap();
    let m = py::PyImport_AddModule(name.as_ptr());
    py::PyModule_GetDict(m)
}

/// Call a zero/N-arg function defined in `__main__` by name, with plain-string args,
/// and read back a string-or-None result. Identical to devshell's helper of the same
/// name — only `boot`/`dispatch` call it here, since `reload`/`last_error` were
/// reload-socket-only.
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

/// Drains the Python stdout/stderr tee and logs each line to logcat. A release build
/// has no `zkite dev` terminal to also forward output to, so — unlike devshell's
/// version of this function — there's nothing to return.
fn drain_output() {
    let text = with_gil(|| unsafe { call_str_fn("_zkite_drain_output", &[]) })
        .ok()
        .flatten()
        .unwrap_or_default();
    for line in text.lines() {
        if !line.is_empty() {
            logi!("py: {line}");
        }
    }
}

// ---------------------------------------------------------------- public API

pub fn setup(prefix: &Path, zkite_pkg_dir: &str, app_dir: &str) -> Result<String, String> {
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

// ---------------------------------------------------------------------- JNI surface

#[cfg(target_os = "android")]
pub use jni_bridge::push_frame_to_java;

#[cfg(target_os = "android")]
mod jni_bridge {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The JVM, `ZKiteActivity`'s class (a **global** ref), and
    /// `onFrameFromPython`'s method id — all resolved once, on the UI thread, during
    /// `boot`. Zero means "not resolved", which makes every push a quiet no-op.
    ///
    /// **Resolving the class at boot rather than at push time is the whole trick.**
    /// `FindClass` resolves against the class loader of the *calling* frame, and a
    /// thread attached later from native code has no Java frames at all — so it falls
    /// back to the system class loader, which cannot see application classes.
    /// Looking `dev/zkite/appshell/ZKiteActivity` up from a Python worker thread would
    /// therefore fail with `ClassNotFoundException` on a real device while working
    /// perfectly in any test that happened to call it from Java. Cached as a global
    /// ref because a local one is only valid for the native frame that created it.
    static JAVA_VM: AtomicUsize = AtomicUsize::new(0);
    static ACTIVITY_CLASS: AtomicUsize = AtomicUsize::new(0);
    static ON_FRAME_METHOD: AtomicUsize = AtomicUsize::new(0);

    /// Cache everything a later cross-thread call needs. Runs on the UI thread inside
    /// `Native.boot`, where `FindClass` can still see application classes.
    ///
    /// Fails **open**, loudly: a failure here downgrades the push path to a no-op and
    /// leaves an app that repaints on touch exactly as it did before, which is the
    /// same posture `gpu_harness` takes toward a failed IME attach.
    pub(super) unsafe fn cache_push_target(env: *mut jni_sys::JNIEnv) {
        let jni = **env;
        let mut vm: *mut jni_sys::JavaVM = std::ptr::null_mut();
        match jni.GetJavaVM {
            Some(get_vm) if get_vm(env, &mut vm) == 0 && !vm.is_null() => {}
            _ => {
                loge!("push path: GetJavaVM failed; a background update will not repaint");
                return;
            }
        }
        let Some(find_class) = jni.FindClass else { return };
        let Some(new_global) = jni.NewGlobalRef else { return };
        let Some(get_static) = jni.GetStaticMethodID else { return };

        let class = find_class(env, c"dev/zkite/appshell/ZKiteActivity".as_ptr());
        if class.is_null() {
            if let Some(clear) = jni.ExceptionClear {
                clear(env);
            }
            loge!("push path: ZKiteActivity not found; a background update will not repaint");
            return;
        }
        let method = get_static(env, class, c"onFrameFromPython".as_ptr(), c"(Ljava/lang/String;)V".as_ptr());
        if method.is_null() {
            if let Some(clear) = jni.ExceptionClear {
                clear(env);
            }
            loge!("push path: onFrameFromPython not found; a background update will not repaint");
            return;
        }
        let global = new_global(env, class);
        if global.is_null() {
            loge!("push path: NewGlobalRef failed; a background update will not repaint");
            return;
        }
        JAVA_VM.store(vm as usize, Ordering::Release);
        ACTIVITY_CLASS.store(global as usize, Ordering::Release);
        ON_FRAME_METHOD.store(method as usize, Ordering::Release);
        logi!("push path armed: a background @store.update() will repaint");
    }

    /// Hand one rendered frame to Java, from whatever Python thread produced it.
    ///
    /// **Attaches and detaches per call, and only when this thread was not already
    /// attached.** Leaving a Python worker attached would mean an ART warning (or
    /// worse) every time such a thread exits, and detaching a thread the JVM created
    /// would be a bug — so `GetEnv` decides which case this is. A pushed frame is a
    /// rare event (a network response, a timer), so the attach is not on any hot path.
    pub fn push_frame_to_java(html: &str) {
        let vm = JAVA_VM.load(Ordering::Acquire) as *mut jni_sys::JavaVM;
        let class = ACTIVITY_CLASS.load(Ordering::Acquire) as jni_sys::jclass;
        let method = ON_FRAME_METHOD.load(Ordering::Acquire) as jni_sys::jmethodID;
        if vm.is_null() || class.is_null() || method.is_null() {
            return;
        }
        unsafe {
            let vtable = **vm;
            let mut env: *mut std::ffi::c_void = std::ptr::null_mut();
            let mut attached_here = false;
            let already = vtable
                .GetEnv
                .map(|get| get(vm, &mut env, jni_sys::JNI_VERSION_1_6))
                .unwrap_or(jni_sys::JNI_ERR);
            if already != jni_sys::JNI_OK || env.is_null() {
                let Some(attach) = vtable.AttachCurrentThread else { return };
                if attach(vm, &mut env, std::ptr::null_mut()) != 0 || env.is_null() {
                    loge!("push path: AttachCurrentThread failed");
                    return;
                }
                attached_here = true;
            }
            let env = env as *mut jni_sys::JNIEnv;
            let jni = **env;
            if let (Some(new_str), Some(call), Some(del)) =
                (jni.NewStringUTF, jni.CallStaticVoidMethod, jni.DeleteLocalRef)
            {
                // NUL-free by construction (it came from Python's UTF-8 buffer as one
                // string), but a `CString` failure must not take the app down.
                if let Ok(c) = CString::new(html) {
                    let jhtml = new_str(env, c.as_ptr());
                    if !jhtml.is_null() {
                        call(env, class, method, jhtml);
                        del(env, jhtml);
                    }
                }
            }
            if attached_here {
                if let Some(detach) = vtable.DetachCurrentThread {
                    detach(vm);
                }
            }
        }
    }

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
    pub unsafe extern "C" fn Java_dev_zkite_appshell_Native_boot(
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
        // Before `setup`, so the class lookup happens on a thread that has Java
        // frames on its stack — see `cache_push_target`. `Native.boot` is called from
        // `zkite-boot`, a thread *Java* created, which is what makes that true here.
        cache_push_target(env);
        match crate::setup(Path::new(&prefix), &zkite_pkg_dir, &app_dir) {
            Ok(html) => string_to_jstr(env, &html),
            Err(e) => {
                loge!("boot failed: {e}");
                string_to_jstr(env, &format!("<div id=\"r\">boot failed: {e}</div>"))
            }
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_appshell_Native_dispatch(
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
    pub unsafe extern "C" fn Java_dev_zkite_appshell_Native_handleBack(
        env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
    ) -> jni_sys::jstring {
        opt_string_to_jstr(env, crate::handle_back())
    }
}
