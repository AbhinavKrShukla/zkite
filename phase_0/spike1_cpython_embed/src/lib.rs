//! # Phase 0, spike 1 — embed official CPython 3.14 for Android in a Rust `cdylib`
//!
//! Pass criterion (plan1.md §10):
//!
//! > Embed official CPython 3.14 Android in a Rust `cdylib`; run `print("hi")` from
//! > `android_main`; confirm it works on arm64 + x86_64 emulator.
//!
//! What this file does:
//!
//! * initialises the interpreter from a prefix laid down at runtime (`PYTHONHOME`),
//! * redirects `sys.stdout` / `sys.stderr` into logcat so `print()` is *observable*
//!   (Android drops a bare `write(1, ...)` on the floor — this is exactly the trap
//!   `_android_support.py` exists to solve, and the reason the criterion is
//!   "confirm it works" rather than "confirm it compiles"),
//! * runs `print("hi")` and captures the result,
//! * exposes three entry points: `android_main` (the `NativeActivity` shape named in
//!   the plan), a JNI method for the dex-shim shape spike 2 actually ships, and a
//!   plain `zkite_spike1_boot` for the standalone probe binary.
//!
//! No PyO3 high-level crate — see the spike README for why.

use std::ffi::{c_char, c_int, CStr, CString};
use std::os::raw::c_void;
use std::path::Path;
use std::ptr;
use std::sync::{Mutex, Once};

use pyo3_ffi as py;

const TAG: &[u8] = b"ZKitePhase0\0";

// liblog is part of the NDK's stable ABI; declaring the one symbol we need is cheaper
// than a dependency on ndk-sys.
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

macro_rules! logi {
    ($($t:tt)*) => { crate::log(ANDROID_LOG_INFO, &format!($($t)*)) };
}
macro_rules! loge {
    ($($t:tt)*) => { crate::log(ANDROID_LOG_ERROR, &format!($($t)*)) };
}

/// Result of a boot attempt, in a shape that is easy to assert on from a test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootReport {
    pub ok: bool,
    /// `sys.version`.
    pub version: String,
    /// `sys.platform`, expected to be `android`.
    pub platform: String,
    /// Whatever `print("hi")` actually wrote to stdout.
    pub stdout: String,
    pub error: Option<String>,
}

impl BootReport {
    fn failed(msg: impl Into<String>) -> BootReport {
        BootReport {
            ok: false,
            version: String::new(),
            platform: String::new(),
            stdout: String::new(),
            error: Some(msg.into()),
        }
    }

    /// A single line the harness greps for in logcat / adb output.
    pub fn to_line(&self) -> String {
        if self.ok {
            format!(
                "ZKITE_SPIKE1_RESULT ok=1 platform={} stdout={:?} version={}",
                self.platform,
                self.stdout,
                self.version.replace('\n', " ")
            )
        } else {
            format!("ZKITE_SPIKE1_RESULT ok=0 error={:?}", self.error.as_deref().unwrap_or(""))
        }
    }
}

static INIT: Once = Once::new();
static PYTHON_HOME: Mutex<Option<CString>> = Mutex::new(None);

/// Point the interpreter at an unpacked prefix. Must be called before `boot`.
///
/// `prefix` is the directory containing `lib/python3.14/`. On device that is the app's
/// data dir; for the probe binary it is `/data/local/tmp/...`.
///
/// Setting `PYTHONHOME` is *not* sufficient, and getting this wrong is the single
/// sharpest edge in the whole spike. An isolated `PyConfig` forces `use_environment = 0`
/// during `PyConfig_Read` regardless of what you assigned to it, so the environment
/// variable is ignored. A standalone binary gets away with it anyway, because CPython
/// falls back to searching upward from `argv[0]` for a `lib/python3.14/os.py` landmark
/// and happens to find one — but inside an app process `argv[0]` is the zygote, there is
/// no landmark, and startup dies with "Failed to import encodings module".
///
/// So the home is recorded here and written into `config.home` in `boot`, which is the
/// supported replacement for the `Py_SetPythonHome` that 3.13 removed. The environment
/// variables are still set because they cost nothing and help any subprocess.
pub fn set_python_home(prefix: &Path) {
    if let Ok(c) = CString::new(prefix.as_os_str().as_encoded_bytes()) {
        *PYTHON_HOME.lock().unwrap() = Some(c);
    }
    std::env::set_var("PYTHONHOME", prefix);
    // Keep the interpreter from importing anything from a user site dir on device.
    std::env::set_var("PYTHONNOUSERSITE", "1");
    // Unbuffered, so a crash cannot swallow the output we are trying to observe.
    std::env::set_var("PYTHONUNBUFFERED", "1");
}

/// Redirect `sys.stdout` / `sys.stderr` into logcat *and* into a buffer we can read back.
///
/// This is the part that would silently not work if you only checked "did Py_Initialize
/// return 0": on Android, fd 1 and 2 go nowhere for an app process.
const REDIRECT_SRC: &str = r#"
import sys, io

class _Tee(io.TextIOBase):
    def __init__(self, level):
        self._level = level
        self.buf = io.StringIO()
    def writable(self):
        return True
    def write(self, s):
        self.buf.write(s)
        return len(s)
    def flush(self):
        pass

sys.stdout = _Tee(4)
sys.stderr = _Tee(6)
"#;

/// Initialise CPython, run `print("hi")`, and report what happened.
///
/// Safe to call more than once; the interpreter is initialised once per process.
pub fn boot() -> BootReport {
    unsafe { boot_inner() }
}

unsafe fn boot_inner() -> BootReport {
    let mut init_err: Option<String> = None;
    INIT.call_once(|| {
        let mut config: py::PyConfig = std::mem::zeroed();
        // Isolated: no signal handlers, no user site dir, no inherited sys.path — the
        // right default for an interpreter embedded in someone else's process.
        py::PyConfig_InitIsolatedConfig(&mut config);
        config.install_signal_handlers = 0;
        // Never write .pyc next to the stdlib: on device that directory may be read-only,
        // and a failed write turns into a confusing import error.
        config.write_bytecode = 0;

        // The one setting that actually locates the stdlib. See `set_python_home`.
        let home = PYTHON_HOME.lock().unwrap().clone();
        if let Some(home) = home {
            let st = py::PyConfig_SetBytesString(&mut config, &mut config.home, home.as_ptr());
            if py::PyStatus_Exception(st) != 0 {
                init_err = Some(format!("PyConfig_SetBytesString(home): {}", cstr_or(st.err_msg, "?")));
            }
        } else {
            init_err = Some("set_python_home() was not called".into());
        }

        if init_err.is_none() {
            let status = py::Py_InitializeFromConfig(&config);
            if py::PyStatus_Exception(status) != 0 {
                init_err = Some(format!(
                    "Py_InitializeFromConfig failed: {}",
                    cstr_or(status.err_msg, "unknown")
                ));
            }
        }
        py::PyConfig_Clear(&mut config);
    });

    if let Some(e) = init_err {
        loge!("{e}");
        return BootReport::failed(e);
    }
    if py::Py_IsInitialized() == 0 {
        return BootReport::failed("interpreter not initialised");
    }

    let gil = py::PyGILState_Ensure();
    let report = run_probe();
    py::PyGILState_Release(gil);
    report
}

unsafe fn run_probe() -> BootReport {
    // 1. Install the stdout/stderr tee.
    if run_string(REDIRECT_SRC).is_err() {
        return BootReport::failed(format!("stdout redirect failed: {}", take_error()));
    }

    // 2. The actual pass criterion.
    if run_string("print(\"hi\")").is_err() {
        return BootReport::failed(format!("print(\"hi\") raised: {}", take_error()));
    }

    // 3. Read back what was printed, plus identifying information.
    let stdout = match eval_to_string("sys.stdout.buf.getvalue()") {
        Ok(s) => s,
        Err(_) => return BootReport::failed(format!("could not read stdout: {}", take_error())),
    };
    let version = eval_to_string("sys.version").unwrap_or_default();
    let platform = eval_to_string("sys.platform").unwrap_or_default();

    let report = BootReport {
        ok: stdout == "hi\n" && platform == "android",
        version,
        platform,
        stdout,
        error: None,
    };
    logi!("{}", report.to_line());
    report
}

unsafe fn cstr_or(p: *const c_char, fallback: &str) -> String {
    if p.is_null() {
        fallback.to_string()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

unsafe fn run_string(src: &str) -> Result<(), ()> {
    let c = CString::new(src).map_err(|_| ())?;
    if py::PyRun_SimpleString(c.as_ptr()) == 0 {
        Ok(())
    } else {
        Err(())
    }
}

/// Evaluate an expression in `__main__` and return it as a Rust `String`.
unsafe fn eval_to_string(expr: &str) -> Result<String, ()> {
    let main_mod = py::PyImport_AddModule(c"__main__".as_ptr());
    if main_mod.is_null() {
        return Err(());
    }
    let globals = py::PyModule_GetDict(main_mod);
    let code = CString::new(expr).map_err(|_| ())?;
    let obj = py::PyRun_String(code.as_ptr(), py::Py_eval_input, globals, globals);
    if obj.is_null() {
        return Err(());
    }
    let mut len: py::Py_ssize_t = 0;
    let utf8 = py::PyUnicode_AsUTF8AndSize(obj, &mut len);
    if utf8.is_null() {
        py::Py_DecRef(obj);
        return Err(());
    }
    let bytes = std::slice::from_raw_parts(utf8 as *const u8, len as usize);
    let out = String::from_utf8_lossy(bytes).into_owned();
    py::Py_DecRef(obj);
    Ok(out)
}

/// Drain a pending Python exception into a string, so a failure is diagnosable rather
/// than just "returned -1".
unsafe fn take_error() -> String {
    if py::PyErr_Occurred().is_null() {
        return "no exception set".to_string();
    }
    py::PyErr_Print();
    "see logcat (PyErr_Print)".to_string()
}

// ---------------------------------------------------------------- entry points

/// The `NativeActivity` entry point named in the pass criterion.
///
/// `android-activity` generates a symbol with this name; declaring it directly keeps the
/// spike free of a crate that would drag in a whole event loop we do not need yet.
/// Spike 2 ships the JNI entry below instead, because plan1.md §3 already concluded
/// NativeActivity is not viable for a real app (no IME).
#[no_mangle]
pub extern "C" fn android_main(_app: *mut c_void) {
    let prefix = std::env::var("ZKITE_PYTHON_HOME").unwrap_or_else(|_| "/data/local/tmp/zkite-python".into());
    set_python_home(Path::new(&prefix));
    let report = boot();
    logi!("android_main: {}", report.to_line());
}

/// JNI entry used by the dex shim in spike 2:
/// `dev.zkite.phase0.Native.boot(String prefix) -> String`
///
/// Returns the report line so the Activity can render it on screen, which is what makes
/// the emulator run a real end-to-end check rather than a log-scrape.
#[cfg(target_os = "android")]
#[no_mangle]
pub unsafe extern "C" fn Java_dev_zkite_phase0_Native_boot(
    env: *mut jni_sys::JNIEnv,
    _class: jni_sys::jclass,
    prefix: jni_sys::jstring,
) -> jni_sys::jstring {
    let jni = **env;

    let raw = (jni.GetStringUTFChars.unwrap())(env, prefix, ptr::null_mut());
    let prefix_str = if raw.is_null() {
        String::new()
    } else {
        let s = CStr::from_ptr(raw).to_string_lossy().into_owned();
        (jni.ReleaseStringUTFChars.unwrap())(env, prefix, raw);
        s
    };

    logi!("Native.boot(prefix={prefix_str})");
    set_python_home(Path::new(&prefix_str));
    let line = boot().to_line();
    match CString::new(line) {
        Ok(c) => (jni.NewStringUTF.unwrap())(env, c.as_ptr()),
        Err(_) => ptr::null_mut(),
    }
}

/// Plain C entry for the standalone probe binary and for anything else that wants the
/// report without JNI in the way.
#[no_mangle]
pub unsafe extern "C" fn zkite_spike1_boot(prefix: *const c_char) -> *mut c_char {
    let p = cstr_or(prefix, "");
    set_python_home(Path::new(&p));
    match CString::new(boot().to_line()) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn zkite_spike1_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}
