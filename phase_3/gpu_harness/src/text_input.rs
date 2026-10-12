//! Safe(ish) wrapper around vendor/gametextinput's C API (../PLAN.md §5, IME).
//!
//! **Everything in this file runs on the UI thread, not the render thread — found
//! the hard way, not designed that way from the start.** The first version of this
//! integration attached the render thread to the JVM and called `GameTextInput_init`
//! there, reasoning (correctly) that `GameTextInput::env_` is captured once and
//! reused for every later call, so it has to be a `JNIEnv` valid for whatever thread
//! makes those later calls. That's true, but incomplete: `GameTextInput_init`
//! internally calls `env->FindClass("com/google/androidgamesdk/gametextinput/...")`
//! (see vendor/gametextinput/include/game-text-input/gametextinput.cpp's
//! constructor) — and `FindClass`, called by *name* from a thread the JVM didn't
//! create (a raw OS thread merely `AttachCurrentThread`-ed after the fact), resolves
//! against the **boot classloader**, not the app's own — so it reliably fails to
//! find any app-defined class and the JNI runtime aborts the process outright
//! (`FATAL SIGNAL 6`, confirmed live on a real device: the crash was inside
//! `GameTextInput::GameTextInput`, at the first `FindClass`). This is a well-known
//! NDK gotcha for exactly this reason — see Android's own JNI tips on `FindClass`
//! from a native thread — and it applies here because the render thread is a plain
//! Rust-spawned OS thread, not one the JVM itself ever called into.
//!
//! The fix is architectural, not a workaround: `GameTextInput_init` (and everything
//! chained from its cached `env_` — `setInputConnection`/`processEvent`) now only
//! ever runs from a real JNI entry point Android calls directly, all of which happen
//! to already be the UI thread for this app (`Native.boot`, `ZKiteSurfaceView`'s
//! `onCreateInputConnection`/`Listener.stateChanged` callbacks). `lib.rs` owns the
//! one `TextInput` instance in a UI-thread-confined static (`TEXT_INPUT`) instead of
//! `render_thread::State` owning one — see that static's own doc comment. Only
//! *decoded, owned* data (`TextState` — plain `String`/`i32`, no JNI types at all)
//! crosses to the render thread via `render_thread::Command::TextInputEvent`, which
//! sidesteps the cross-thread-`JNIEnv` question entirely for that path.
//!
//! **What the render thread still legitimately does over its own JNI attachment**
//! (see `render_thread.rs`'s `attach_current_thread`/`call_focus_method`/`call_blur_method`):
//! plain instance-method calls (`focusAndShowKeyboard`/`blurAndHideKeyboard`) on a
//! `ZKiteSurfaceView` object it already holds a global ref to, via
//! `GetObjectClass`+`GetMethodID`+`CallVoidMethod`. That's a fundamentally different
//! JNI operation from `FindClass`-by-name: `GetObjectClass` reads an object's actual
//! runtime type directly from the object itself, with no classloader-by-name lookup
//! involved, so it has no such thread restriction. The bug was specifically in
//! resolving *classes by name* from the wrong thread, not in JNI-from-a-background-
//! thread generally.
//!
//! **What this deliberately does NOT do, and why** (see ../PLAN.md §5's own
//! IME write-up for the fuller list of what's left): `GameTextInput_showIme`/
//! `hideIme` are not used here. Showing/hiding the keyboard is done with two small
//! Java helpers on `ZKiteSurfaceView` instead
//! (`focusAndShowKeyboard`/`blurAndHideKeyboard`) — simpler than reasoning about
//! whether `GameTextInput`'s C-side `showIme`/`hideIme` (which calls
//! `InputMethodManager` through the `InputConnection` object) tolerates being
//! invoked before that object even exists yet on first focus. What *is* used is the
//! part with no real substitute: the vendored `InputConnection` Java class's
//! `Editable`/composing-region-aware editing operations
//! (`commitText`/`setComposingText`/`deleteSurroundingText`/...), which is the
//! actual "don't hand-roll `InputConnection`" value plan1.md §9 and phase 0's README
//! both call out.
//!
//! **`GameTextInput_setState` (../PLAN.md §7.7 — tap-to-position-cursor):** used to
//! push a new selection (from a tap, hit-tested against real glyph positions via
//! `zkite_renderer`'s `Cursor`-based helpers) into the live `Editable`. Read
//! `InputConnection.setState`'s own bytecode (via `javap` — no source ships) before
//! changing this: it does `mEditable.clear()` then re-inserts `state.text` from
//! scratch, so every call **must** carry the field's actual current text, not just
//! the new selection — passing a stale or wrong string here silently clobbers
//! whatever the user already typed. **On whether this notifies `Listener`, trust the
//! real device over the bytecode reading:** a first pass at that `javap` reading
//! concluded it doesn't call `stateUpdated` — wrong, or at least incomplete;
//! live logcat during §7.7's verification shows every `setState` call immediately
//! followed by `stateChanged`/`onTextInputEvent` carrying the same text. `setState`
//! finishes by internally calling `finishComposingText`/`setComposingRegion(-1,-1)`
//! to drop any active composing region, and *that* is what fires the notify — easy
//! to miss reading the bytecode in isolation, obvious on a real device. `render_
//! thread.rs` still updates its own idea of the caret position optimistically right
//! after calling this rather than waiting on that event, purely to avoid a frame of
//! visible lag — the two are redundant, not conflicting, since both land on the
//! exact selection this call itself requested.

use std::ffi::c_void;
use std::os::raw::c_char;

#[repr(C)]
struct GameTextInputSpanFfi {
    start: i32,
    end: i32,
}

#[repr(C)]
struct GameTextInputStateFfi {
    text_utf8: *const c_char,
    text_length: i32,
    selection: GameTextInputSpanFfi,
    composing_region: GameTextInputSpanFfi,
}

/// Never constructed on the Rust side — only ever a pointer `GameTextInput_init`
/// hands back and every other `GameTextInput_*` call takes as its first argument.
enum GameTextInputOpaque {}

type GetStateCallback = unsafe extern "C" fn(*mut c_void, *const GameTextInputStateFfi);

extern "C" {
    fn GameTextInput_init(env: *mut jni_sys::JNIEnv, max_string_size: u32) -> *mut GameTextInputOpaque;
    fn GameTextInput_setInputConnection(input: *mut GameTextInputOpaque, input_connection: jni_sys::jobject);
    fn GameTextInput_processEvent(input: *mut GameTextInputOpaque, event_state: jni_sys::jobject);
    fn GameTextInput_setEventCallback(input: *mut GameTextInputOpaque, callback: GetStateCallback, context: *mut c_void);
    fn GameTextInput_setState(input: *mut GameTextInputOpaque, state: *const GameTextInputStateFfi);
}

/// A JNI global reference, wrapped so it can cross the `render_thread::Command`
/// channel to the render thread — global refs (unlike local refs) are valid from any
/// thread, so this is sound as long as every value ever stored here really is one.
/// Used only for `ZKiteSurfaceView` itself (`Command::SurfaceViewReady`) — see the
/// module doc for why the `InputConnection`/`State` objects `GameTextInput` deals
/// with never need this at all (they never leave the UI thread).
#[derive(Clone, Copy)]
pub struct GlobalRef(pub jni_sys::jobject);
// SAFETY: a JNI global reference is explicitly documented as valid from any thread —
// see https://docs.oracle.com/en/java/javase/21/docs/specs/jni/design.html#global-and-local-references.
unsafe impl Send for GlobalRef {}

/// The decoded contents of a `GameTextInputState` — owned, so it outlives the C
/// callback that produced it (`text_UTF8` there is only valid for the callback's own
/// duration; see gametextinput.h's doc on `GameTextInputGetStateCallback`) *and* so
/// it can cross to the render thread as plain data (see the module doc).
#[derive(Clone, Debug, Default)]
pub struct TextState {
    pub text: String,
    /// Byte offsets into `text` (UTF-8), matching `GameTextInputState`'s own units.
    /// `start == end` is a plain cursor, no selection (this integration doesn't yet
    /// distinguish a real selection from a cursor anywhere downstream — see
    /// ../PLAN.md §7.7's own known-gaps note). Feeds `render_thread.rs`'s
    /// `focused_selection`, which drives real caret placement via
    /// `zkite_renderer::Renderer::text_caret_geometry`.
    pub selection_start: i32,
    pub selection_end: i32,
}

extern "C" fn state_callback(context: *mut c_void, state: *const GameTextInputStateFfi) {
    // SAFETY: `context` is always a live `*mut Option<TextState>` for the duration of
    // this call — the only caller, `TextInput::process_event`, points it at a local
    // variable that outlives the `GameTextInput_processEvent` call that can invoke
    // this callback (synchronously, on the same thread, before that call returns).
    unsafe {
        let out = &mut *(context as *mut Option<TextState>);
        if state.is_null() {
            return;
        }
        let s = &*state;
        let text = if s.text_utf8.is_null() || s.text_length <= 0 {
            String::new()
        } else {
            let bytes = std::slice::from_raw_parts(s.text_utf8 as *const u8, s.text_length as usize);
            String::from_utf8_lossy(bytes).into_owned()
        };
        *out = Some(TextState { text, selection_start: s.selection.start, selection_end: s.selection.end });
    }
}

/// Owns the one `GameTextInput` handle for this process — see the module doc for why
/// this (and every method on it) only ever runs on the UI thread.
pub struct TextInput {
    handle: *mut GameTextInputOpaque,
}

// Stored in `lib.rs`'s `TEXT_INPUT` static, which — being a `static` — needs its
// `Mutex<Option<TextInput>>` to be `Sync`, which needs `TextInput: Send`. Not because
// this is ever actually *used* from a different thread than it was created on (see
// the module doc: every call site is a JNI entry point Android calls on the UI
// thread) — this is the same "defensive, not load-bearing" unsafe impl as
// `render_thread.rs` used to carry on its own now-removed `TextInput` field.
unsafe impl Send for TextInput {}

impl TextInput {
    /// `env` must be a `JNIEnv` for a thread the JVM itself called into (a real JNI
    /// entry point) — see the module doc. `max_string_size` bytes are pre-allocated
    /// by the library for the largest string it'll ever hold; 0 asks for the
    /// library's own default (64KiB), comfortably more than any `zkite_py`
    /// `TextField` needs.
    pub fn new(env: *mut jni_sys::JNIEnv) -> Result<Self, String> {
        let handle = unsafe { GameTextInput_init(env, 0) };
        if handle.is_null() {
            return Err("GameTextInput_init returned null".into());
        }
        Ok(TextInput { handle })
    }

    /// Registers the `InputConnection` Java object `onCreateInputConnection` just
    /// built. `input_connection` is an ordinary local ref, valid because this is
    /// called synchronously, on the UI thread, from within the same JNI entry point
    /// that received it as a parameter (`Native.onInputConnectionCreated`) — no
    /// global-ref promotion needed (contrast `GlobalRef`, used only for
    /// `ZKiteSurfaceView` itself, which *does* have to outlive its own JNI call).
    /// `GameTextInput` makes its own internal global ref (see gametextinput.cpp's
    /// `setInputConnection`) before this call returns.
    pub fn set_input_connection(&self, input_connection: jni_sys::jobject) {
        unsafe { GameTextInput_setInputConnection(self.handle, input_connection) };
    }

    /// Decodes a `gametextinput.State` Java object (from `Listener.stateChanged`,
    /// an ordinary local ref — same reasoning as `set_input_connection`) into an
    /// owned `TextState`, via `GameTextInput`'s own `stateFromJava` (reached through
    /// `processEvent`, the only C entry point that exposes it — see
    /// gametextinput.h). Never registers a persistent event callback
    /// (`GameTextInput_setEventCallback` is called once, here, pointed at a stack
    /// slot that only lives for this call) — the library's callback mechanism is
    /// designed for a long-lived registration, but this crate already gets "new
    /// state arrived" as a distinct JNI entry point per event
    /// (`Native.onTextInputEvent`), so re-deriving the same information via a second
    /// mechanism would be redundant, not additive.
    pub fn process_event(&self, state_obj: jni_sys::jobject) -> Option<TextState> {
        let mut slot: Option<TextState> = None;
        unsafe {
            GameTextInput_setEventCallback(self.handle, state_callback, &mut slot as *mut _ as *mut c_void);
            GameTextInput_processEvent(self.handle, state_obj);
        }
        slot
    }

    /// Pushes `text` (the field's real, current, full text — see this struct's own
    /// module-doc paragraph on why a stale value here would silently clobber a live
    /// edit) and a new `[selection_start, selection_end)` into the live `Editable` —
    /// for tap-to-position-cursor (../PLAN.md §7.7). **Does** trigger
    /// `Listener.stateChanged` in practice (confirmed live on device — see the
    /// module doc; a `javap` read of `setState` alone misses this, since the notify
    /// happens inside the `finishComposingText`/`setComposingRegion` calls `setState`
    /// makes internally to drop the composing region). The caller still updates its
    /// own idea of the caret position immediately after this returns rather than
    /// waiting on that event, purely to avoid a frame of visible lag.
    pub fn set_state(&self, text: &str, selection_start: i32, selection_end: i32) {
        let Ok(c_text) = std::ffi::CString::new(text) else { return };
        let state = GameTextInputStateFfi {
            text_utf8: c_text.as_ptr(),
            text_length: c_text.as_bytes().len() as i32,
            selection: GameTextInputSpanFfi { start: selection_start, end: selection_end },
            // No composing region of our own to preserve — a tap that repositions
            // the cursor implicitly finishes whatever the IME was composing, the
            // same way a real EditText's own touch handling does.
            composing_region: GameTextInputSpanFfi { start: -1, end: -1 },
        };
        unsafe { GameTextInput_setState(self.handle, &state) };
    }
}
