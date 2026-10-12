//! AccessKit integration (../PLAN.md §5 item 2) — bridges `zkite_pybridge`'s per-render
//! `accesskit::TreeUpdate` (role/label/value/actions from `RawNode`, bounds from
//! `positioned()` — see `zkite_pybridge::BuiltTree::accessibility_tree_update`) to a
//! real `android.view.accessibility.AccessibilityNodeProvider`, via
//! `accesskit_android::InjectingAdapter`.
//!
//! **Don't hand-roll this — same call phase 0/§7.1 already made for IME.**
//! `InjectingAdapter` installs itself on `ZKiteSurfaceView` via
//! `View.setAccessibilityDelegate`/`setOnHoverListener` — both plain platform APIs —
//! using a Java `Delegate` class loaded from a `.dex` `accesskit_android` embeds
//! (`InMemoryDexClassLoader`, the `embedded-dex` Cargo feature). Unlike
//! `vendor/gametextinput`'s AAR, this needs **zero new Java source of ZKite's own**
//! and **zero androidx dependency** — `Delegate.java`
//! (`~/.cargo/registry/.../accesskit_android-0.7.5/java/dev/accesskit/android/
//! Delegate.java`, read in full before writing this module) imports only
//! `android.os`/`android.view`/`android.view.accessibility`, so this sidesteps the
//! `NoClassDefFoundError: androidx/core/R$id` class of problem §7.3 item 4 hit
//! entirely, not just the `FindClass`-off-thread one §7.2 hit.
//!
//! **Two real findings from reading `accesskit_android`'s own source, not its docs
//! alone (its crates.io page has none beyond the README):**
//!
//! 1. **`InjectingAdapter::new` is safe to call from the render thread, not just the
//!    UI thread — unlike `GameTextInput_init`.** §7.2's bug was `FindClass`-by-name
//!    resolving against the wrong classloader when called from a JVM-attached (not
//!    JVM-created) thread. `accesskit_android::inject::delegate_class`'s
//!    `embedded-dex` path never does that: it `find_class`es only
//!    `dalvik/system/InMemoryDexClassLoader` (a boot-classpath class, resolvable from
//!    any thread) and then calls `loadClass` *on that fresh loader instance* — a
//!    method call on an object already in hand, not a lookup by name against the
//!    calling thread's own classloader. Confirmed by reading `inject.rs` end to end,
//!    not assumed from the "embedded-dex" feature's name alone. So this module's
//!    `Activation`/`Actions` handlers are constructed and `InjectingAdapter::new` is
//!    called directly from `render_thread::State::ensure_a11y_adapter` — no
//!    UI-thread hop, no new JNI entry point, unlike `text_input.rs`'s `TEXT_INPUT`.
//! 2. **`jni-sys = "0.3"` (this crate's own raw JNI type, used for every existing JNI
//!    entry point) and the `jni-sys 0.4` that `accesskit_android`'s `jni = "0.21"`
//!    dependency uses are two distinct Rust types, not interchangeable by simple
//!    reuse, even though both describe the identical stable JNI C ABI.** Checked
//!    directly (`~/.cargo/registry/.../jni-sys-0.3.1/src/lib.rs`): that specific
//!    0.3.1 release is a compatibility shim that re-exports 0.4's `_jobject` (so
//!    `jobject`/`jstring`/etc. values need no cast at all — see `bridge_env`/
//!    `bridge_jobject` below), but defines its own separate `JNIEnv`/
//!    `JNINativeInterface_` struct rather than re-exporting 0.4's — so a `.cast()` on
//!    the `*mut JNIEnv` pointer specifically *is* required, and is sound only because
//!    both crates describe the same underlying `JNINativeInterface` C struct layout —
//!    the same category of pointer-type reinterpretation `lib.rs`'s own
//!    `Native_onSurfaceCreated` already does (`env.cast()` for the `ndk` crate's
//!    `NativeWindow::from_surface`), not a new kind of risk this module introduces.

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, TreeUpdate};
use accesskit_android::InjectingAdapter;
use std::sync::Mutex;

/// The most recently pushed tree (see `set_latest_tree`) — read by
/// `Activation::request_initial_tree` when Android's accessibility framework
/// (re)attaches, so a TalkBack session that starts *after* the app has already been
/// running for a while sees the real tree immediately, not `accesskit_android`'s own
/// one-node placeholder window. A `Mutex`, not a `render_thread::State` field: unlike
/// `update_if_active` (only ever called from the render thread here — see this
/// module's own doc), `request_initial_tree` is invoked by `accesskit_android` from
/// whatever thread Android calls `Delegate`'s native methods on, which this crate does
/// not control and must not assume is the render thread.
static LATEST_TREE: Mutex<Option<TreeUpdate>> = Mutex::new(None);

/// Called by `render_thread::State` after every `redraw()` — see that method's own
/// `push_a11y_update`.
pub(crate) fn set_latest_tree(update: TreeUpdate) {
    *LATEST_TREE.lock().unwrap() = Some(update);
}

pub(crate) struct Activation;

impl ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        LATEST_TREE.lock().unwrap().clone()
    }
}

/// Forwards every AccessKit action request (a TalkBack double-tap-to-activate, a
/// screen-reader "move to next element" giving a node input focus, ...) to the render
/// thread as an ordinary `Command`, the same channel every touch/IME event already
/// crosses on — `ActionHandler::do_action`'s own doc says the thread it runs on is
/// "platform-dependent," and for this adapter that's whatever thread Android calls
/// `Delegate.performAction` on, never this crate's single-threaded Python/GPU state
/// (`render_thread::State`), which only the render thread may touch. Deliberately a
/// unit struct with no state of its own — `crate::SENDER` (already `pub(crate)`-
/// reachable from any sibling module in this crate, same as every JNI entry point in
/// `lib.rs` already relies on) is the one channel, not a private copy of it.
pub(crate) struct Actions;

impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        crate::logi!("a11y do_action: {:?} on node {}", request.action, request.target_node.0);
        let guard = crate::SENDER.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(crate::render_thread::Command::A11yAction(request));
        }
    }
}

/// Bridges this crate's own raw `jni_sys::JNIEnv` (from `render_thread::State::
/// render_env` — the render thread's own JVM attachment, already used for plain
/// instance-method calls like `focusAndShowKeyboard`, see `render_thread.rs`'s
/// `attach_current_thread`) to the safe `jni` crate's `JNIEnv` that
/// `accesskit_android::InjectingAdapter::new` needs. See this module's own doc,
/// finding 2, for why the cast (not a type-level no-op) is the correct, sound bridge.
pub(crate) unsafe fn bridge_env<'local>(
    raw: *mut jni_sys::JNIEnv,
) -> Result<accesskit_android::jni::JNIEnv<'local>, accesskit_android::jni::errors::Error> {
    unsafe { accesskit_android::jni::JNIEnv::from_raw(raw.cast()) }
}

/// Bridges this crate's own raw `jni_sys::jobject` (from `render_thread::State::
/// surface_view` — a real JNI **global** reference, promoted in `lib.rs`'s
/// `Native_setSurfaceView` before it ever reaches this thread) to `jni::objects::
/// JObject<'static>` — `'static` is the *documented* correct choice for wrapping an
/// already-global reference (`JObject`'s own doc: "if `'local` is `'static`... it is
/// either null or a global reference"), not an unsound shortcut; `JObject` itself has
/// no `Drop` impl (checked directly against `jni-0.21.1/src/wrapper/objects/
/// jobject.rs`), so wrapping it here never double-frees or otherwise disturbs the
/// global ref's own lifetime, which this crate continues to own exactly as before
/// (leaked once per process — see `text_input.rs`'s `GlobalRef` doc for why that's an
/// accepted, pre-existing trade, not something this module changes).
pub(crate) unsafe fn bridge_jobject(raw: jni_sys::jobject) -> accesskit_android::jni::objects::JObject<'static> {
    unsafe { accesskit_android::jni::objects::JObject::from_raw(raw.cast()) }
}

/// Constructs the one `InjectingAdapter` for this process. Not called from this
/// module — `render_thread::State::ensure_a11y_adapter` calls it once both a JNIEnv
/// and the `ZKiteSurfaceView` global ref are available, and stores the result on
/// `State` itself (no static needed: unlike `LATEST_TREE`/`Actions`, which must be
/// reachable from an arbitrary Android-chosen thread, the adapter handle itself is
/// only ever touched from the render thread after construction — see `redraw`'s
/// `push_a11y_update`).
pub(crate) fn install(env: &mut accesskit_android::jni::JNIEnv, host: &accesskit_android::jni::objects::JObject) -> InjectingAdapter {
    InjectingAdapter::new(env, host, Activation, Actions)
}
