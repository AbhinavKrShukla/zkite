//! Phase 3 spike 2 — see ../PLAN.md §4. JNI surface for the GPU render harness.
//!
//! Same logging/JNI-string idiom as `phase_2/appshell/src/lib.rs` on purpose (raw
//! `jni_sys` calls, no `jni` crate — see that file, and ../PLAN.md §1's finding about
//! why `android-activity` isn't used here either): this crate is plumbing around
//! `render_thread`'s actual logic, the same division of labor the rest of ZKite's
//! native code already uses.

use std::sync::mpsc::Sender;
use std::sync::Mutex;

mod a11y;
mod images;
mod reload;
mod render_thread;
mod surface_lifecycle;
static SURFACE_LIFECYCLE: surface_lifecycle::SurfaceLifecycle = surface_lifecycle::SurfaceLifecycle::new();
mod text_input;
use render_thread::Command;
use text_input::GlobalRef;

#[cfg_attr(not(target_os = "android"), allow(unused_imports))]
use std::ffi::{c_char, c_int, CString};

const TAG: &[u8] = b"ZKiteGpuHarness\0";

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
macro_rules! logi { ($($t:tt)*) => { $crate::log($crate::ANDROID_LOG_INFO, &format!($($t)*)) }; }
macro_rules! loge { ($($t:tt)*) => { $crate::log($crate::ANDROID_LOG_ERROR, &format!($($t)*)) }; }
pub(crate) use loge;
pub(crate) use logi;

static SENDER: Mutex<Option<Sender<Command>>> = Mutex::new(None);

/// The one command channel to the render thread, for modules other than this one
/// (`reload.rs`). Everything that reaches the render thread goes through here, which
/// is why there is exactly one of it and why it is a `Mutex<Option<..>>`: before
/// `boot()` there is no thread to send to, and every JNI entry point in this file
/// already treats "no sender yet" as a no-op rather than an error.
pub(crate) fn sender() -> std::sync::MutexGuard<'static, Option<Sender<Command>>> {
    SENDER.lock().unwrap()
}

/// The `zkite` package fingerprint this shell shipped, published for `reload.rs`'s
/// `hello` frame (../PLAN.md §31.6).
///
/// Written once by the render thread right after `zkite_pybridge::setup`, read by the
/// socket thread. It lives here rather than in `reload.rs` for the same reason
/// `SENDER` does: the socket thread must not call into Python, so the value has to be
/// handed *to* it by the thread that legitimately can.
///
/// **The outer `Option` is "has the render thread published yet", and it is the whole
/// point of this being a `Condvar` rather than a plain `Mutex`.** `ZKiteActivity`
/// starts the socket from `onCreate`, and it dials `zkite dev` in a 500ms retry loop,
/// while CPython boot takes ~650ms on an emulator and ~1.5s on the Moto G54. So the
/// `hello` frame is written *before* `setup` returns, essentially always. With a plain
/// `Mutex<Option<String>>` the socket read `None`, took the deliberate fail-open path,
/// omitted the field, and the staleness check silently never ran — the handshake was
/// dead in exactly the case it exists for, and passed a hand-run test because that
/// test connected to an already-booted app. Found by capturing the frame off the wire,
/// not by reading the code (../PLAN.md §31.6).
///
/// The inner `Option` is the value itself: `None` means "genuinely could not be
/// computed", which `zkite dev` treats as "say nothing" rather than as a mismatch.
///
/// The WebView shell needs none of this — `ZKiteActivity.onCreate` there calls
/// `Native.boot` synchronously *before* `startReloadServer`, so its socket thread
/// cannot outrun the interpreter.
static FRAMEWORK_FINGERPRINT: Mutex<Option<Option<String>>> = Mutex::new(None);
static FRAMEWORK_FINGERPRINT_READY: std::sync::Condvar = std::sync::Condvar::new();

pub(crate) fn set_framework_fingerprint(v: Option<String>) {
    *FRAMEWORK_FINGERPRINT.lock().unwrap() = Some(v);
    FRAMEWORK_FINGERPRINT_READY.notify_all();
}

/// Wait (briefly) for the render thread to publish, then answer.
///
/// Bounded, because a boot that never completes must not wedge the socket: on timeout
/// this answers `None` and `zkite dev` says nothing, which is the same fail-open
/// behaviour as a fingerprint that could not be computed. `set_framework_fingerprint`
/// is called on *both* the success and failure paths of `setup`, so the only way to
/// reach the timeout is a boot that hangs outright.
pub(crate) fn framework_fingerprint_when_ready(timeout: std::time::Duration) -> Option<String> {
    let guard = FRAMEWORK_FINGERPRINT.lock().unwrap();
    match FRAMEWORK_FINGERPRINT_READY.wait_timeout_while(guard, timeout, |v| v.is_none()) {
        Ok((v, _)) => v.clone().flatten(),
        Err(_) => None,
    }
}

/// The one `GameTextInput` handle for this process (../PLAN.md §5, IME) —
/// deliberately **not** owned by `render_thread::State`, unlike `SENDER`'s
/// `Command`-consuming thread. See `text_input.rs`'s module doc for why: every
/// `GameTextInput_*` call has to happen on a thread the JVM itself called into (a
/// real JNI entry point, which for this app always means the UI thread), and this
/// static — read and written only from JNI entries in this file — is what keeps
/// that true without needing `render_thread.rs` to know anything about it.
static TEXT_INPUT: Mutex<Option<text_input::TextInput>> = Mutex::new(None);

#[cfg(target_os = "android")]
mod jni_bridge {
    use super::*;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;

    unsafe fn jstr_to_string(env: *mut jni_sys::JNIEnv, s: jni_sys::jstring) -> String {
        if s.is_null() {
            return String::new();
        }
        let jni = **env;
        let raw = (jni.GetStringChars.unwrap())(env, s, std::ptr::null_mut());
        if raw.is_null() { return String::new(); }
        let len = (jni.GetStringLength.unwrap())(env, s) as usize;
        let out = String::from_utf16_lossy(std::slice::from_raw_parts(raw, len));
        (jni.ReleaseStringChars.unwrap())(env, s, raw);
        out
    }

    
/// Reads the three families (`sans`/`serif`/`mono`, each regular + bold) `boot` is
/// handed a directory of.
///
/// A directory rather than six JNI string parameters — that would have grown
/// `Native.boot`'s signature to twelve arguments for what is really one concept, and
/// every one of them would have had to be threaded through `ZKiteActivity` by hand.
/// The names match `phase_1/webruntime/scripts/stage_fonts.sh`'s output exactly,
/// which is the point: the WebView backend loads those same six files via
/// `@font-face`, so the two backends render the same typefaces by construction
/// rather than by two build scripts happening to agree (../PLAN.md §9.4).
///
/// All three families are required here, unlike `FontSet::sans_only` — this harness
/// controls its own asset staging, so a missing file is a build bug worth failing
/// loudly on, not a degraded configuration worth silently tolerating.
fn load_font_set(dir: &str) -> Result<zkite_renderer::FontSet, String> {
    let read = |name: String| -> Result<Vec<u8>, String> {
        let path = std::path::Path::new(dir).join(&name);
        std::fs::read(&path).map_err(|e| format!("couldn't read font {}: {e}", path.display()))
    };
    let faces = |stem: &str| -> Result<zkite_renderer::FontFaces, String> {
        Ok(zkite_renderer::FontFaces::new(
            read(format!("{stem}.ttf"))?,
            read(format!("{stem}-bold.ttf"))?,
        ))
    };
    Ok(zkite_renderer::FontSet {
        sans: faces("sans")?,
        serif: Some(faces("serif")?),
        mono: Some(faces("mono")?),
    })
}

#[no_mangle]
    /// Starts the dev-loop reload socket (../PLAN.md §15). Called by
    /// `ZKiteActivity` **only** when the `zkite-devmode` marker asset is present, i.e.
    /// only in a Dev Shell build — a shipped `zkite build --backend native` app must
    /// never run this (`phase_2/PLAN.md` §1.2: an infinite reconnect thread and an
    /// `INTERNET` permission with no runtime purpose).
    ///
    /// Safe to call before or after `boot()`: the socket thread only ever *sends* a
    /// `Command`, and `SENDER` being empty is the same no-op every other pre-boot
    /// JNI entry point here already treats it as.
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_startReloadServer(
        env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        host: jni_sys::jstring,
        port: jni_sys::jint,
        app_dir: jni_sys::jstring,
    ) {
        let host = jstr_to_string(env, host);
        let app_dir = jstr_to_string(env, app_dir);
        logi!("startReloadServer({host}:{port}, app_dir={app_dir})");
        crate::reload::start_reload_server(host, port as u16, app_dir);
    }

#[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_boot(
        env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        python_prefix: jni_sys::jstring,
        zkite_pkg_dir: jni_sys::jstring,
        app_dir: jni_sys::jstring,
        font_dir: jni_sys::jstring,
        touch_slop: jni_sys::jfloat,
        double_tap_slop: jni_sys::jfloat,
        double_tap_timeout_ms: jni_sys::jfloat,
        density: jni_sys::jfloat,
        refresh_rate_hz: jni_sys::jfloat,
    ) {
        // Same three Python paths `phase_2/appshell`'s `Native.boot` takes — see
        // `render_thread::AppPaths`'s doc comment — plus the font path this harness
        // already needed, `ViewConfiguration.getScaledTouchSlop()` for gesture
        // disambiguation (../PLAN.md §5), and `getScaledDoubleTapSlop()`/
        // `getDoubleTapTimeout()` for double-tap-to-select-word (../PLAN.md §5/§7.5).
        // `ZKiteActivity.java` unpacks the first four from bundled assets before
        // calling this.
        let python_prefix = jstr_to_string(env, python_prefix);
        let zkite_pkg_dir = jstr_to_string(env, zkite_pkg_dir);
        let app_dir = jstr_to_string(env, app_dir);
        let font_dir = jstr_to_string(env, font_dir);
        let mut guard = SENDER.lock().unwrap();
        if guard.is_some() {
            logi!("boot(): render thread already running, ignoring");
            return;
        }
        let fonts = match load_font_set(&font_dir) {
            Ok(f) => f,
            Err(e) => {
                loge!("boot(): {e}");
                return;
            }
        };
        // The render thread still needs *a* JNIEnv of its own, attached on itself,
        // for the plain instance-method calls it does make off the UI thread
        // (`focusAndShowKeyboard`/`blurAndHideKeyboard` — see
        // `render_thread.rs`'s `attach_current_thread`) — what it needs from here
        // isn't `env` itself (only valid on this call, this thread) but the
        // process-wide `JavaVM*` that any thread can attach through. Stored as a
        // `usize` purely so `AppPaths` stays `Send` without its own unsafe impl —
        // see that field's own doc comment. `GameTextInput` itself is initialized
        // separately, right below, and stays on *this* thread — see `TEXT_INPUT`'s
        // doc comment and text_input.rs's module doc for why those are no longer
        // the same attachment.
        let mut vm: *mut jni_sys::JavaVM = std::ptr::null_mut();
        let jni = **env;
        if let Some(get_vm) = jni.GetJavaVM {
            let rc = get_vm(env, &mut vm);
            if rc != 0 {
                loge!("boot(): GetJavaVM failed: rc={rc}");
                vm = std::ptr::null_mut();
            }
        }
        {
            let mut ti_guard = TEXT_INPUT.lock().unwrap();
            if ti_guard.is_none() {
                match text_input::TextInput::new(env) {
                    Ok(ti) => {
                        logi!("boot(): GameTextInput initialized");
                        *ti_guard = Some(ti);
                    }
                    Err(e) => {
                        loge!("boot(): TextInput::new failed, TextField focus/IME will be a no-op: {e}");
                    }
                }
            }
        }
        // Fails *safe* rather than trusting the value blindly: a non-finite or
        // non-positive density would divide the surface to nothing (or to NaN) and
        // render a blank app with no other symptom. 1.0 degrades to exactly the
        // pre-§13 behaviour — visibly too small, but working — which is a far better
        // failure than a black screen, and it is logged so it can't pass unnoticed.
        let scale = if density.is_finite() && density > 0.0 {
            density
        } else {
            loge!("boot(): implausible DisplayMetrics.density {density}, falling back to 1.0 (app will render 1/dpr too small)");
            1.0
        };
        // Same fail-safe posture as `scale`: a nonsense refresh rate would either
        // spin the render thread flat out or stall a fling to a crawl, so an
        // implausible value falls back to a plain 60Hz rather than being trusted.
        // The bounds are deliberately wide — 24Hz to 240Hz covers every shipping
        // Android panel — because the point is to reject garbage, not to second-guess
        // the platform.
        let frame_period = if refresh_rate_hz.is_finite() && (24.0..=240.0).contains(&refresh_rate_hz) {
            std::time::Duration::from_secs_f32(1.0 / refresh_rate_hz)
        } else {
            loge!("boot(): implausible Display.getRefreshRate() {refresh_rate_hz}, pacing at 60Hz");
            render_thread::FALLBACK_FRAME_INTERVAL
        };
        logi!(
            "boot(): spawning render thread (fonts from {font_dir}, touch slop {touch_slop}px, density {scale}, {refresh_rate_hz}Hz -> {:.2}ms frame period)",
            frame_period.as_secs_f32() * 1000.0
        );
        let paths = render_thread::AppPaths {
            python_prefix: std::path::PathBuf::from(python_prefix),
            zkite_pkg_dir,
            app_dir,
            // Both slop constants arrive from `ViewConfiguration.getScaled*` in
            // *device* pixels, and every distance they are compared against is now in
            // dp — so they convert here, once, at the same boundary the density
            // itself crosses, rather than at each of their comparison sites.
            touch_slop: touch_slop / scale,
            double_tap_slop: double_tap_slop / scale,
            double_tap_timeout: std::time::Duration::from_millis(double_tap_timeout_ms.max(0.0) as u64),
            scale,
            frame_period,
            java_vm: vm as usize,
        };
        *guard = Some(render_thread::spawn(fonts, paths));
    }

    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_onSurfaceCreated(
        env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        surface: jni_sys::jobject,
        width: jni_sys::jint,
        height: jni_sys::jint,
    ) {
        let Some(window) = ndk::native_window::NativeWindow::from_surface(env.cast(), surface.cast())
        else {
            loge!("onSurfaceCreated: ANativeWindow_fromSurface returned null");
            return;
        };
        let guard = SENDER.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            let generation = SURFACE_LIFECYCLE.created();
            let _ = tx.send(Command::NewWindow { window, width, height, generation });
        } else {
            loge!("onSurfaceCreated: called before boot()");
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_requestRedraw(
        _env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
    ) {
        if let Some(tx) = SENDER.lock().unwrap().as_ref() {
            let _ = tx.send(Command::Redraw);
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_onSurfaceChanged(
        _env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        width: jni_sys::jint,
        height: jni_sys::jint,
    ) {
        let guard = SENDER.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(Command::Resize { width, height });
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_onSurfaceDestroyed(
        _env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
    ) {
        SURFACE_LIFECYCLE.destroyed();
        let guard = SENDER.lock().unwrap();
        let Some(tx) = guard.as_ref() else { return };
        let (ack_tx, ack_rx) = sync_channel(1);
        if tx.send(Command::DestroyWindow { ack: ack_tx }).is_err() {
            return;
        }
        // Bounded, not indefinite: a wedged render thread must not hang Android's own
        // surface-callback thread forever. Android's contract ("nothing may touch the
        // surface once this callback returns") is only actually satisfied when the ack
        // arrives — a timeout here is a last-resort escape hatch, not the expected path.
        if ack_rx.recv_timeout(Duration::from_millis(3000)).is_err() {
            loge!("onSurfaceDestroyed: render thread did not ack within 3s");
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_onTouchDown(
        _env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        x: jni_sys::jfloat,
        y: jni_sys::jfloat,
    ) {
        let guard = SENDER.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(Command::TouchDown { x, y, at: std::time::Instant::now() });
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_onTouchMove(
        _env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        x: jni_sys::jfloat,
        y: jni_sys::jfloat,
    ) {
        let guard = SENDER.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(Command::TouchMove { x, y, at: std::time::Instant::now() });
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_onTouchUp(
        _env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        x: jni_sys::jfloat,
        y: jni_sys::jfloat,
    ) {
        let guard = SENDER.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(Command::TouchUp { x, y, at: std::time::Instant::now() });
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_onTouchCancel(
        _env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
    ) {
        let guard = SENDER.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(Command::TouchCancel);
        }
    }

    /// The user pressed Back (../PLAN.md §16) — the system button, the back gesture,
    /// or Android 13+'s `OnBackInvokedCallback`, all funnelled here by
    /// `ZKiteActivity`.
    ///
    /// Returns immediately and never blocks: Java has already consumed the press, so
    /// there is nothing to answer synchronously, and blocking the UI thread on a
    /// render thread that might be mid-frame is how this becomes an ANR. If nothing
    /// in `zkite_py` wants the press, the render thread calls
    /// `ZKiteSurfaceView.finishActivity()` back on the UI thread — see
    /// `Command::Back`'s own doc comment for the tradeoff that buys.
    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_onBackPressed(
        _env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
    ) {
        let guard = SENDER.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(Command::Back);
        }
    }

    /// Called once from `ZKiteActivity.onCreate()`, right after constructing the
    /// `ZKiteSurfaceView` — hands the render thread its own object (../PLAN.md §5),
    /// so it can later call `focusAndShowKeyboard`/`blurAndHideKeyboard` on it. The
    /// `jobject` Android hands a JNI call is a local ref valid only for this call on
    /// this (UI) thread — promoted to a global ref *here*, before it ever reaches
    /// the render thread, exactly the pattern `text_input.rs`'s module doc
    /// describes for every other cross-thread Java object in this crate.
    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_setSurfaceView(
        env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        view: jni_sys::jobject,
    ) {
        let jni = **env;
        let Some(new_global_ref) = jni.NewGlobalRef else { return };
        let global = new_global_ref(env, view);
        let guard = SENDER.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(Command::SurfaceViewReady(GlobalRef(global)));
        }
    }

    /// Called from `ZKiteSurfaceView.onCreateInputConnection` once it's built a fresh
    /// `gametextinput.InputConnection` — see ../PLAN.md §5. Registers it with
    /// `GameTextInput` directly, on this (UI) thread — **not** forwarded to the
    /// render thread at all, unlike `setSurfaceView`. See `TEXT_INPUT`'s and
    /// `text_input.rs`'s doc comments for why: `input_connection` is an ordinary
    /// local ref, valid for exactly this call, on exactly this thread, which is all
    /// `TextInput::set_input_connection` needs.
    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_onInputConnectionCreated(
        _env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        input_connection: jni_sys::jobject,
    ) {
        let guard = TEXT_INPUT.lock().unwrap();
        if let Some(ti) = guard.as_ref() {
            ti.set_input_connection(input_connection);
        } else {
            loge!("onInputConnectionCreated: no TextInput (boot() TextInput::new must have failed)");
        }
    }

    /// Called from `ZKiteSurfaceView`'s `gametextinput.Listener.stateChanged` (via a
    /// small forwarding method — see that file) every time the IME edits the
    /// focused field's text. Decodes `state` into a plain, owned `TextState` on
    /// *this* thread (`TextInput::process_event`, same reasoning as
    /// `onInputConnectionCreated` — `state` is only a valid local ref here) and
    /// *that* — not any JNI type — is what actually crosses to the render thread.
    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_onTextInputEvent(
        _env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        state: jni_sys::jobject,
    ) {
        let decoded = {
            let guard = TEXT_INPUT.lock().unwrap();
            guard.as_ref().and_then(|ti| ti.process_event(state))
        };
        let Some(new_state) = decoded else {
            loge!("onTextInputEvent: no decoded state (no TextInput, or processEvent produced nothing)");
            return;
        };
        logi!("onTextInputEvent: decoded text={:?}", new_state.text);
        let guard = SENDER.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(Command::TextInputEvent(new_state));
        } else {
            loge!("onTextInputEvent: no SENDER (render thread not running?)");
        }
    }

    /// Called from `ZKiteSurfaceView.setSelection` (../PLAN.md §7.7, tap-to-position-
    /// cursor) — always on the UI thread already (that method does its own
    /// `post()` hop before calling this, mirroring `focusAndShowKeyboard`), so this
    /// can call straight into `TEXT_INPUT` with no further thread juggling. `text`
    /// is a local ref, read out via `jstr_to_string` synchronously within this call
    /// — never stored, so no global-ref promotion needed.
    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_setSelection(
        env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        text: jni_sys::jstring,
        start: jni_sys::jint,
        end: jni_sys::jint,
    ) {
        let text = jstr_to_string(env, text);
        let guard = TEXT_INPUT.lock().unwrap();
        if let Some(ti) = guard.as_ref() {
            ti.set_state(&text, start, end);
        } else {
            loge!("setSelection: no TextInput (boot() TextInput::new must have failed)");
        }
    }

    /// Called from `TextSelectionOverlay`'s own `TextWatcher`/`onSelectionChanged`
    /// (../PLAN.md §7.8) every time the overlay's text or selection changes while it
    /// still has focus. `text` is a local ref, read out synchronously — same
    /// no-global-ref-needed reasoning as `setSelection` just above.
    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_onSelectionOverlayChanged(
        env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        text: jni_sys::jstring,
        start: jni_sys::jint,
        end: jni_sys::jint,
    ) {
        let text = jstr_to_string(env, text);
        let start = zkite_pybridge::utf16_to_utf8_offset(&text, start);
        let end = zkite_pybridge::utf16_to_utf8_offset(&text, end);
        let guard = SENDER.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(Command::SelectionOverlayChanged { text, start, end });
        } else {
            loge!("onSelectionOverlayChanged: no SENDER (render thread not running?)");
        }
    }

    /// Called from `TextSelectionOverlay` once it loses focus (../PLAN.md §7.8) —
    /// either a tap elsewhere took it, or `render_thread.rs`'s `blur_text_field`
    /// explicitly asked it to hide. `text`/`start`/`end` are its state at the
    /// moment of dismissal.
    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_onSelectionOverlayDismissed(
        env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        text: jni_sys::jstring,
        start: jni_sys::jint,
        end: jni_sys::jint,
    ) {
        let text = jstr_to_string(env, text);
        let start = zkite_pybridge::utf16_to_utf8_offset(&text, start);
        let end = zkite_pybridge::utf16_to_utf8_offset(&text, end);
        let guard = SENDER.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(Command::SelectionOverlayDismissed { text, start, end });
        } else {
            loge!("onSelectionOverlayDismissed: no SENDER (render thread not running?)");
        }
    }

    /// Called from `ZKiteSurfaceView`'s `Listener.onSoftwareKeyboardVisibilityChanged`
    /// (when it goes `false`) and `.onEditorAction` — see ../PLAN.md §5. No Java
    /// object involved, so no global-ref dance needed, unlike the three JNI entries
    /// just above.
    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_onKeyboardDismissed(
        _env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
    ) {
        let guard = SENDER.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(Command::KeyboardDismissed);
        }
    }

    /// Called from `ZKiteSurfaceView.onImeInsetsChanged` (../PLAN.md §5/§7.5),
    /// itself driven by the real `WindowInsetsCompat` delivery
    /// `forwardWindowInsets` already routes to `GameTextInput` for
    /// `mSoftKeyboardActive`. No Java object involved, same as
    /// `onKeyboardDismissed`.
    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_onImeInsetsChanged(
        _env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
        bottom_px: jni_sys::jint,
    ) {
        let guard = SENDER.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(Command::ImeInsetsChanged { bottom: bottom_px as f32 });
        }
    }

    /// Called from `ZKiteActivity.onDestroy()` — the user is actually leaving the app
    /// (not just backgrounding it; `onSurfaceDestroyed` already handles rotation/
    /// backgrounding via `DestroyWindow`, which this is not a replacement for). Without
    /// this, the render thread — and the GPU device/adapter/queue it owns — outlives
    /// the Activity for as long as the process happens to stick around, which on
    /// Android can be a while after the user has visibly left. Takes the `Sender` out
    /// of `SENDER` entirely (not just sending `Stop` through it) so a stray event
    /// racing with shutdown hits the `None` branch in every other handler instead of a
    /// channel whose receiver is mid-shutdown.
    #[no_mangle]
    pub unsafe extern "C" fn Java_dev_zkite_gpuharness_Native_onDestroy(
        _env: *mut jni_sys::JNIEnv,
        _class: jni_sys::jclass,
    ) {
        let mut guard = SENDER.lock().unwrap();
        if let Some(tx) = guard.take() {
            let _ = tx.send(Command::Stop);
        }
    }
}
